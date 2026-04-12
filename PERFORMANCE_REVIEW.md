# Performance Review — voicebot-rs

> Reviewed: April 2026 · Milestones 1–9 complete Methodology: Full static analysis of all hot-path crate source files. This is a pre-optimization audit. No profiling data yet — severity is estimated from call frequency and allocation size.

---

## Summary

The pipeline is functionally correct and the architecture is sound. The dominant cost in a real call is network I/O to Speaches (ASR + TTS round trips). That said, there are several areas where avoidable allocations and redundant work accumulate in sub-second hot paths, particularly during streaming LLM responses and audio frame ingress/egress.

| # | Issue | File | Line(s) | Severity | Category |
| --- | --- | --- | --- | --- | --- |
| 1 | SSE line buffer: full string reallocated per newline | `agent/src/openai.rs` | 151–152 | **High** | Alloc |
| 2 | Sentence boundary iterates string twice per partial response | `core/src/orchestrator.rs` | 343–354 | **High** | CPU + Alloc |
| 3 | `drain().collect()` to build sentence needlessly clones buffer | `core/src/orchestrator.rs` | 354 | **High** | Alloc |
| 4 | `to_pcm_bytes()` allocates `Vec<u8>` on every call (4 call sites) | `common/src/audio.rs` | 57 | **High** | Alloc |
| 5 | ASR collects audio with `to_pcm_bytes()` per frame (double copy) | `asr/src/speaches.rs` | 132–134 | **High** | Alloc |
| 6 | TTS frame chunking drains into intermediate `Vec<u8>` | `tts/src/speaches.rs` | 117 | **Medium** | Alloc |
| 7 | `ConversationMemory::messages()` clones entire history per LLM call | `agent/src/memory.rs` | 38–39 | **Medium** | Alloc |
| 8 | Tool definitions rebuilt (`collect()`) on every LLM tool loop iteration | `agent/src/core.rs` | 60–61 | **Medium** | Alloc |
| 9 | LLM internal channel created per tool-loop iteration | `agent/src/core.rs` | 57 | **Medium** | Overhead |
| 10 | `FrameChunker::push()` returns `Vec<Vec<i16>>` — two heap allocs per call | `vad/src/energy.rs` | 31–36 | **Medium** | Alloc |
| 11 | `rms_energy` uses `.powi(2)` instead of multiply in 16 kHz sample loop | `vad/src/energy.rs` | 6 | **Medium** | CPU |
| 12 | `AudioFrame` Arc cloned per 20 ms audio frame in session fanout | `core/src/session.rs` | 236–241 | **Low** | CPU |
| 13 | `flush_remaining` clones trimmed string before moving it | `core/src/orchestrator.rs` | 372 | **Low** | Alloc |
| 14 | `substitute_env_vars` calls `String::replace` once per variable | `common/src/config.rs` | 155 | **Low** | Alloc (startup only) |

---

## Detailed Findings

---

### 1 · SSE Line Buffer Full Reallocation Per Newline

**File:** `voicebot/crates/agent/src/openai.rs` lines 150–152  
**Severity:** High

```rust
while let Some(newline_pos) = line_buffer.find('\n') {
    let line = line_buffer[..newline_pos].trim().to_string();  // alloc 1
    line_buffer = line_buffer[newline_pos + 1..].to_string();  // alloc 2
```

With streaming LLM responses (50–200 events/sec), each SSE line found causes two String allocations: one for the extracted line and one to rebuild the remaining buffer. This is O(remaining_buffer_len) memory copied per newline.

**Fix:** Use an index into an existing buffer rather than rebuilding it.

```rust
// Replace inner while loop with index-based approach:
let mut consumed = 0;
while let Some(rel) = line_buffer[consumed..].find('\n') {
    let abs = consumed + rel;
    let line = line_buffer[consumed..abs].trim();
    // process line using &str — no allocation
    process_sse_line(line, &mut ...);
    consumed = abs + 1;
}
// Drain once at the end
line_buffer.drain(..consumed);  // O(remaining) copy, only once per chunk
```

---

### 2 · Sentence Boundary Detects by Iterating String Twice

**File:** `voicebot/crates/core/src/orchestrator.rs` lines 343–350  
**Severity:** High

```rust
let boundary = self
    .sentence_buffer
    .char_indices()                          // iterator 1
    .zip(self.sentence_buffer.chars().skip(1))  // iterator 2 — full scan from start
    .find(|((_, c), next)| { ... })
    .map(|((i, c), _)| i + c.len_utf8());
```

`chars().skip(1)` creates a second full iterator over the same string, consuming it in lockstep with `char_indices()`. This iterates every character of `sentence_buffer` twice per partial response event.

**Fix:** Single pass with a lookahead from `char_indices`:

```rust
let mut boundary = None;
let chars: Vec<(usize, char)> = self.sentence_buffer.char_indices().collect();
for i in 0..chars.len().saturating_sub(1) {
    let (byte_idx, c) = chars[i];
    let (_, next) = chars[i + 1];
    if (c == '.' || c == '!' || c == '?' || c == '\n') && next.is_whitespace() {
        boundary = Some(byte_idx + c.len_utf8());
        break;
    }
}
```

Or even simpler — match on byte slices since all sentence-ending chars are ASCII (single byte), avoiding the `char_indices` overhead entirely.

---

### 3 · `drain().collect()` Builds Intermediate String for Sentence

**File:** `voicebot/crates/core/src/orchestrator.rs` line 354  
**Severity:** High

```rust
let sentence: String = self.sentence_buffer.drain(..pos).collect();
let trimmed = sentence.trim();
if !trimmed.is_empty() {
    if tx.send(trimmed.to_string()).await.is_err() {  // 3rd copy
```

Three copies happen here:

1. `drain().collect()` — moves chars into new `String`
2. `.trim()` — borrows the String (no copy here)
3. `trimmed.to_string()` — clones for the channel send

`drain()` already mutates `sentence_buffer` in-place (efficient), but `collect()` into a `String` followed by an immediate `trim().to_string()` is two allocations where one suffices.

**Fix:**

```rust
// Get the slice before draining, trim it, only then send (one allocation)
let raw = &self.sentence_buffer[..pos];
let trimmed = raw.trim().to_owned();
self.sentence_buffer.drain(..pos);
if !trimmed.is_empty() {
    if tx.send(trimmed).await.is_err() { ... }
}
```

---

### 4 · `to_pcm_bytes()` Allocates `Vec<u8>` on Every Call

**File:** `voicebot/crates/common/src/audio.rs` lines 56–60  
**Severity:** High

```rust
pub fn to_pcm_bytes(&self) -> Vec<u8> {
    self.data.iter().flat_map(|s| s.to_le_bytes()).collect()
}
```

Call sites (all hot paths):

- `transport/websocket/src/handler.rs:261` — every TTS audio chunk to client (~50 frames/sec during TTS)
- `asr/src/speaches.rs:133` — every audio frame during ASR collection (~50 frames/sec per utterance)
- `transport/asterisk/src/audiosocket.rs:74` — every TTS chunk to Asterisk

For a 20ms frame at 16 kHz: 320 samples × 2 bytes = 640 bytes allocated and immediately discarded after each call.

**Fix:** Add a write variant that avoids allocation in hot paths, keep the existing method for the rare cases that need ownership.

```rust
/// Write PCM bytes directly into an existing buffer (no allocation).
pub fn append_pcm_bytes_to(&self, buf: &mut Vec<u8>) {
    buf.reserve(self.data.len() * 2);
    for &s in &*self.data {
        buf.extend_from_slice(&s.to_le_bytes());
    }
}
```

---

### 5 · ASR Audio Collection: Double Copy Per Frame

**File:** `voicebot/crates/asr/src/speaches.rs` lines 131–134  
**Severity:** High

```rust
let mut pcm_bytes: Vec<u8> = Vec::new();
while let Some(frame) = audio.recv().await {
    pcm_bytes.extend_from_slice(&frame.to_pcm_bytes());
    //                                  ↑ allocates Vec<u8>  ↑ then copies it again
}
```

`to_pcm_bytes()` allocates a transient `Vec<u8>`, which is immediately consumed by `extend_from_slice` into `pcm_bytes`. For a 5-second utterance at 50 frames/sec this is 250 wasted allocations (each 640 bytes), totalling ~160 KB of garbage per utterance.

**Fix:** Use the write variant from fix #4:

```rust
let mut pcm_bytes: Vec<u8> = Vec::new();
while let Some(frame) = audio.recv().await {
    frame.append_pcm_bytes_to(&mut pcm_bytes);  // zero intermediate allocation
}
```

Or inline directly:

```rust
while let Some(frame) = audio.recv().await {
    pcm_bytes.reserve(frame.data.len() * 2);
    for &s in &*frame.data {
        pcm_bytes.extend_from_slice(&s.to_le_bytes());
    }
}
```

---

### 6 · TTS Frame Chunking: Intermediate `Vec<u8>` Per 20 ms Frame

**File:** `voicebot/crates/tts/src/speaches.rs` line 117  
**Severity:** Medium

```rust
while pcm_buf.len() >= frame_bytes {
    let frame_data: Vec<u8> = pcm_buf.drain(..frame_bytes).collect(); // unnecessary
    let frame = AudioFrame::from_pcm_bytes(&frame_data, 0);           // takes &[u8]
```

`drain(..).collect()` builds a new `Vec<u8>` only to pass it as `&[u8]` to `from_pcm_bytes`. The slice of `pcm_buf` is already contiguous in memory.

**Fix:**

```rust
while pcm_buf.len() >= frame_bytes {
    let frame = AudioFrame::from_pcm_bytes(&pcm_buf[..frame_bytes], sequence_ts);
    pcm_buf.drain(..frame_bytes);  // mutate in-place, no intermediate Vec
```

---

### 7 · `ConversationMemory::messages()` Clones Entire History Per LLM Call

**File:** `voicebot/crates/agent/src/memory.rs` lines 38–39  
**Severity:** Medium

```rust
pub fn messages(&self) -> Vec<Message> {
    self.messages.iter().cloned().collect()  // clones every Message (contains Strings)
}
```

Called on every LLM completion attempt (up to 5 per turn, for tool loop iterations). `Message` contains `String` (role + content), so this clones the full conversation history — up to 40 messages × String content per LLM call.

**Fix:** Change the LLM provider trait to accept a slice reference:

```rust
// In traits.rs:
async fn stream_completion(
    &self,
    messages: &[Message],   // borrow, not own
    ...
```

Then `messages()` can return `&[Message]` or callers use `self.memory.messages_ref()` returning `Vec<&Message>`. This avoids all clones of conversation history.

---

### 8 · Tool Definitions Rebuilt Every Tool-Loop Iteration

**File:** `voicebot/crates/agent/src/core.rs` lines 60–61  
**Severity:** Medium

```rust
loop {  // iterates up to 5 times per turn
    ...
    let tool_defs: Vec<ToolDefinition> =
        self.tools.iter().map(|t| t.definition()).collect();
```

`t.definition()` returns (likely) owned `ToolDefinition` structs which are collected into a Vec on every iteration. Since tools never change during a turn, this is pure waste.

**Fix:** Compute once before the loop:

```rust
let tool_defs: Vec<ToolDefinition> =
    self.tools.iter().map(|t| t.definition()).collect();

loop {
    ...
    result = llm.stream_completion(&messages, &tool_defs, ...) => result,
```

Or, if `ToolDefinition` is cheap to reference, cache it on `AgentCore` at construction time.

---

### 9 · LLM Response Channel Allocated Each Tool-Loop Iteration

**File:** `voicebot/crates/agent/src/core.rs` lines 57–66  
**Severity:** Medium

```rust
loop {
    ...
    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<PipelineEvent>(20);
    ...
    let llm_handle = tokio::spawn(async move { ... });
```

Each iteration (up to 5) creates a new bounded mpsc channel (heap allocation) and spawns a new task for the LLM call. Spawning is cheap in Tokio but not free.

The channel could be reused across iterations by resetting it, or the LLM call could be driven directly without an intermediate channel since the handle is awaited immediately.

**Fix (minor):** Reuse the pattern by `await`ing the LLM future directly without a channel spawn if no concurrent work is needed during that phase.

---

### 10 · `FrameChunker::push()` Allocates `Vec<Vec<i16>>` Per Call

**File:** `voicebot/crates/vad/src/energy.rs` lines 31–36  
**Severity:** Medium

```rust
pub fn push(&mut self, samples: &[i16]) -> Vec<Vec<i16>> {
    self.buffer.extend_from_slice(samples);
    let mut chunks = Vec::new();          // heap alloc
    while self.buffer.len() >= self.chunk_size {
        chunks.push(self.buffer.drain(..self.chunk_size).collect());  // inner heap alloc
    }
    chunks
}
```

Every call to `push()` allocates an outer `Vec` (for the returned chunks) and an inner `Vec<i16>` per complete chunk. At 16 kHz with 20 ms frames arriving at 50 Hz, this is 50+ outer Vec allocations and potentially 50+ inner Vec allocations per second.

**Fix:** Use a callback or iterator API to avoid the outer allocation:

```rust
pub fn push_with<F: FnMut(&[i16])>(&mut self, samples: &[i16], mut callback: F) {
    self.buffer.extend_from_slice(samples);
    while self.buffer.len() >= self.chunk_size {
        callback(&self.buffer[..self.chunk_size]);
        self.buffer.drain(..self.chunk_size);
    }
}
```

Since the callers in `vad/component.rs` process each chunk immediately, the callback form avoids all allocations for the common case.

---

### 11 · `rms_energy` Uses `.powi(2)` Instead of Multiply

**File:** `voicebot/crates/vad/src/energy.rs` line 6  
**Severity:** Medium

```rust
let sum_sq: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
```

`.powi(2)` dispatches through a general-purpose integer-power routine. `x * x` compiles to a single `fmul` instruction. At 16,000 samples/sec (one `powi` call per sample) this is measurable on CPU. Compilers may or may not optimize this away — better to be explicit.

**Fix:**

```rust
let sum_sq: f64 = samples.iter().map(|&s| {
    let v = s as f64;
    v * v
}).sum();
```

Also consider computing in `f32` throughout (halving FP width) and using the `i32` intermediate trick to avoid widening each sample to `f64`:

```rust
let sum_sq: i64 = samples.iter().map(|&s| (s as i64) * (s as i64)).sum();
let rms = ((sum_sq as f64 / samples.len() as f64).sqrt() / i16::MAX as f64) as f32;
```

---

### 12 · `AudioFrame` Cloned Per 20 ms Frame in Session Fanout

**File:** `voicebot/crates/core/src/session.rs` lines 236–241  
**Severity:** Low

```rust
if let Some(ref tx) = current_asr_tx {
    let _ = tx.try_send(f.clone());   // Arc reference count increment
}
let _ = vad_audio_tx.send(f).await;  // moves original
```

`AudioFrame::clone()` increments the `Arc<[i16]>` reference count (atomic operation). At 50 frames/sec this is 50 atomic inc/dec pairs per second. This is genuinely cheap in isolation but worth noting for future multi-consumer expansion.

The current design is already correct (Arc shared ownership, not data copy). No change needed unless profiling reveals contention.

---

### 13 · `flush_remaining` Clones Trimmed String Before Moving It

**File:** `voicebot/crates/core/src/orchestrator.rs` line 372  
**Severity:** Low

```rust
let trimmed = self.sentence_buffer.trim().to_string();  // clone
if !trimmed.is_empty() {
    if let Some(tx) = &self.tts_text_tx {
        if tx.send(trimmed).await.is_err() {  // moved
```

`trim()` returns `&str`, then `.to_string()` clones it before checking if it is empty. The non-empty check should happen on the `&str` before cloning.

**Fix:**

```rust
let trimmed = self.sentence_buffer.trim();
if !trimmed.is_empty() {
    let owned = trimmed.to_string();  // clone only when needed
    if let Some(tx) = &self.tts_text_tx {
        if tx.send(owned).await.is_err() { ... }
    }
}
self.sentence_buffer.clear();
```

---

### 14 · `substitute_env_vars` Reallocates String Per Variable

**File:** `voicebot/crates/common/src/config.rs` lines 150–157  
**Severity:** Low (startup only)

```rust
for cap in re.captures_iter(input) {
    match std::env::var(var_name) {
        Ok(value) => {
            result = result.replace(&cap[0], &value);  // new String each iteration
```

`String::replace` creates a new owned `String` for every environment variable substitution. With N vars in config this is N string copies of the entire config file. Since this runs once at startup it has zero runtime impact, but it is a straightforward fix.

**Fix:** Build the result string in a single pass using `Regex::replace_all` or by iterating captures and pushing slices/values into a pre-allocated buffer.

---

## What Is Already Good

- `AudioFrame.data` is `Arc<[i16]>` — cross-task sharing is zero-copy.
- `try_send` (non-blocking) is used for audio ingress; backpressure drops frames rather than blocking the transport task.
- `biased` is correctly applied to the session fanout `select!` so cancellation is checked before blocking on audio.
- `FrameChunker` keeps a persistent `Vec<i16>` buffer between calls — correct.
- `reqwest::Client` is constructed once per provider instance and reused — connection pooling is in effect.
- `CancellationToken` child tokens are used throughout; cleanup is not conditional on explicit message passing.
- Channel capacities are documented and bounded everywhere.

---

## Recommended Fix Order

**Phase 1 — high frequency, easy wins (~1 day)**

1. Issue #4: Add `AudioFrame::append_pcm_bytes_to(&mut Vec<u8>)`.
2. Issue #5: Use it in `asr/speaches.rs` audio collection loop.
3. Issue #6: Pass `&pcm_buf[..frame_bytes]` directly to `from_pcm_bytes` in TTS.
4. Issue #11: Replace `.powi(2)` with `v * v` in `rms_energy`.

**Phase 2 — streaming LLM path (~half day)**

5. Issue #1: Index-based SSE line buffer (avoid per-newline String rebuild).
6. Issue #8: Move `tool_defs` collection outside the tool loop.
7. Issue #7: Change `messages()` to return `&[Message]` and update trait.

**Phase 3 — sentence boundary / orchestrator (~half day)**

8. Issues #2 + #3: Single-pass boundary scan + drain-before-clone.
9. Issue #13: Trim before cloning.

**Phase 4 — low priority**

10. Issue #10: `FrameChunker` callback API.
11. Issues #12, #14: Cosmetic / startup-only.
