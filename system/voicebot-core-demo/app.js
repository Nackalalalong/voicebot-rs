// Voicebot Core Demo — Browser client
// Connects to the voicebot WS server, sends mic PCM, displays chat, plays TTS audio.

const SAMPLE_RATE = 16000;
const FRAME_SIZE = 320; // 20ms at 16kHz
const FRAME_BYTES = FRAME_SIZE * 2; // i16 = 2 bytes per sample

// DOM refs
const wsUrlInput = document.getElementById('ws-url');
const languageSelect = document.getElementById('language');
const statusEl = document.getElementById('status');
const messagesEl = document.getElementById('messages');
const btnConnect = document.getElementById('btn-connect');
const btnMic = document.getElementById('btn-mic');
const micLabel = document.getElementById('mic-label');
const vuBar = document.getElementById('vu-bar');

// State
let ws = null;
let audioCtx = null;
let playbackCtx = null;
let playbackNode = null;
let micStream = null;
let processor = null;
let micActive = false;
let currentBotMsg = null; // accumulates streaming agent text
let currentUserMsg = null; // accumulates streaming transcript

// TTS audio accumulation for the current bot response
let currentTtsChunks = []; // Array<ArrayBuffer> of raw PCM i16 LE chunks

// Streaming playback queue for TTS PCM
let playbackQueue = []; // Array<Float32Array>
let playbackQueueOffset = 0;
let playbackQueuedSamples = 0;
let playbackPrimed = false;
let playbackStreamOpen = false;
const PLAYBACK_BUFFER_SIZE = 2048;
const PREBUFFER_MS = 400; // accumulate 400ms before starting to avoid underruns

// --- Status ---
function setStatus(state, label) {
    statusEl.className = 'status ' + state;
    statusEl.textContent = label || state.charAt(0).toUpperCase() + state.slice(1);
}

// --- Chat messages ---
function addMessage(role, text, isPartial) {
    const div = document.createElement('div');
    div.className = 'message ' + role;

    const roleLabel = document.createElement('div');
    roleLabel.className = 'role';
    roleLabel.textContent = role === 'user' ? 'You' : role === 'bot' ? 'Assistant' : '';

    const content = document.createElement('div');
    if (isPartial) content.className = 'partial';
    content.textContent = text;

    if (role !== 'system') div.appendChild(roleLabel);
    div.appendChild(content);
    messagesEl.appendChild(div);
    messagesEl.parentElement.scrollTop = messagesEl.parentElement.scrollHeight;
    return div;
}

function addSystemMessage(text) {
    return addMessage('system', text, false);
}

function updateMessageText(msgEl, text, isPartial) {
    const content = msgEl.querySelector('div:last-child');
    content.textContent = text;
    content.className = isPartial ? 'partial' : '';
}

function ensurePlaybackEngine() {
    if (!playbackCtx) {
        playbackCtx = new AudioContext({sampleRate: SAMPLE_RATE});
    }

    if (!playbackNode) {
        playbackNode = playbackCtx.createScriptProcessor(PLAYBACK_BUFFER_SIZE, 0, 1);
        playbackNode.onaudioprocess = (event) => {
            const output = event.outputBuffer.getChannelData(0);
            output.fill(0);

            if (!playbackPrimed) return;

            let writeOffset = 0;
            while (writeOffset < output.length && playbackQueue.length > 0) {
                const chunk = playbackQueue[0];
                const remaining = chunk.length - playbackQueueOffset;
                const copyCount = Math.min(output.length - writeOffset, remaining);

                output.set(
                    chunk.subarray(playbackQueueOffset, playbackQueueOffset + copyCount),
                    writeOffset,
                );

                writeOffset += copyCount;
                playbackQueueOffset += copyCount;
                playbackQueuedSamples -= copyCount;

                if (playbackQueueOffset >= chunk.length) {
                    playbackQueue.shift();
                    playbackQueueOffset = 0;
                }
            }

            if (playbackQueuedSamples === 0 && !playbackStreamOpen) {
                playbackPrimed = false;
            }
        };
        playbackNode.connect(playbackCtx.destination);
    }

    if (playbackCtx.state === 'suspended') {
        void playbackCtx.resume().catch(() => {});
    }
}

function pcm16ToFloat32(arrayBuffer) {
    const i16 = new Int16Array(arrayBuffer);
    const floats = new Float32Array(i16.length);
    for (let i = 0; i < i16.length; i++) {
        floats[i] = i16[i] / 32768;
    }
    return floats;
}

function resetPlaybackQueue() {
    playbackQueue = [];
    playbackQueueOffset = 0;
    playbackQueuedSamples = 0;
    playbackPrimed = false;
    playbackStreamOpen = false;
}

// --- WebSocket ---
function connect() {
    if (ws) {
        disconnect();
        return;
    }

    const url = wsUrlInput.value.trim();
    if (!url) return;

    setStatus('connecting', 'Connecting...');
    if (!audioCtx) audioCtx = new AudioContext();
    ensurePlaybackEngine();
    ws = new WebSocket(url);
    ws.binaryType = 'arraybuffer';

    ws.onopen = () => {
        setStatus('connected', 'Connected');
        btnConnect.textContent = 'Disconnect';
        btnConnect.classList.add('active');
        btnMic.disabled = false;
        addSystemMessage('Connected to server');

        // Send session_start
        ws.send(
            JSON.stringify({
                type: 'session_start',
                language: languageSelect.value,
                asr: 'speaches',
                tts: 'speaches',
                sample_rate: Math.round(audioCtx?.sampleRate || SAMPLE_RATE),
            }),
        );
    };

    ws.onmessage = (event) => {
        if (typeof event.data === 'string') {
            handleServerText(JSON.parse(event.data));
        } else {
            handleServerAudio(event.data);
        }
    };

    ws.onclose = () => {
        setStatus('disconnected', 'Disconnected');
        btnConnect.textContent = 'Connect';
        btnConnect.classList.remove('active');
        btnMic.disabled = true;
        stopMic();
        resetPlaybackQueue();
        ws = null;
    };

    ws.onerror = () => {
        addSystemMessage('Connection error');
    };
}

function disconnect() {
    if (ws) {
        ws.send(JSON.stringify({type: 'session_end'}));
        ws.close();
    }
    resetPlaybackQueue();
}

function handleServerText(msg) {
    switch (msg.type) {
        case 'transcript_partial':
            if (!currentUserMsg) {
                currentUserMsg = addMessage('user', msg.text, true);
            } else {
                updateMessageText(currentUserMsg, msg.text, true);
            }
            setStatus('listening', 'Listening...');
            break;

        case 'transcript_final':
            if (currentUserMsg) {
                updateMessageText(currentUserMsg, msg.text, false);
            } else {
                currentUserMsg = addMessage('user', msg.text, false);
            }
            currentUserMsg = null;
            setStatus('thinking', 'Thinking...');
            break;

        case 'agent_partial':
            if (!currentBotMsg) {
                currentBotMsg = addMessage('bot', msg.text, true);
                currentBotMsg._fullText = msg.text;
                currentTtsChunks = [];
                playbackStreamOpen = true;
            } else {
                currentBotMsg._fullText += msg.text;
                updateMessageText(currentBotMsg, currentBotMsg._fullText, true);
            }
            break;

        case 'agent_final':
            if (currentBotMsg) {
                updateMessageText(currentBotMsg, msg.text, false);
            } else {
                currentBotMsg = addMessage('bot', msg.text, false);
                currentTtsChunks = [];
            }
            setStatus('speaking', 'Speaking...');
            break;

        case 'tts_complete':
            playbackStreamOpen = false;
            flushPendingAudio();
            finalizeTtsAudio(currentBotMsg, currentTtsChunks);
            currentBotMsg = null;
            currentTtsChunks = [];
            if (ws && ws.readyState === WebSocket.OPEN) {
                setStatus('connected', 'Connected');
            }
            break;

        case 'error':
            addSystemMessage(`Error: ${msg.code}`);
            break;
    }
}

// --- Audio playback ---
function handleServerAudio(arrayBuffer) {
    currentTtsChunks.push(arrayBuffer);
    ensurePlaybackEngine();

    const floats = pcm16ToFloat32(arrayBuffer);
    playbackQueue.push(floats);
    playbackQueuedSamples += floats.length;
    playbackStreamOpen = true;

    const pendingMs = (playbackQueuedSamples / SAMPLE_RATE) * 1000;
    if (!playbackPrimed && pendingMs >= PREBUFFER_MS) {
        playbackPrimed = true;
    }
}

// Flush any chunks that didn't reach the pre-buffer threshold (last sentence).
function flushPendingAudio() {
    if (playbackQueuedSamples > 0) {
        playbackPrimed = true;
    }
}

// Build a WAV file from raw PCM i16 LE chunks and attach an <audio> player
// to the given message element.
function finalizeTtsAudio(msgEl, chunks) {
    if (!msgEl || chunks.length === 0) return;

    // Concatenate all PCM chunks
    const totalBytes = chunks.reduce((n, b) => n + b.byteLength, 0);
    const pcm = new Uint8Array(totalBytes);
    let offset = 0;
    for (const chunk of chunks) {
        pcm.set(new Uint8Array(chunk), offset);
        offset += chunk.byteLength;
    }

    // Wrap in a minimal WAV container (16kHz mono 16-bit PCM)
    const wavBytes = buildWav(pcm, SAMPLE_RATE, 1, 16);
    const blob = new Blob([wavBytes], {type: 'audio/wav'});
    const url = URL.createObjectURL(blob);

    const audio = document.createElement('audio');
    audio.controls = true;
    audio.src = url;
    audio.className = 'tts-player';
    // Revoke object URL when no longer needed
    audio.onended = () => {};

    msgEl.appendChild(audio);
    messagesEl.parentElement.scrollTop = messagesEl.parentElement.scrollHeight;
}

// Build a WAV ArrayBuffer from raw PCM bytes.
function buildWav(pcmBytes, sampleRate, channels, bitsPerSample) {
    const dataLen = pcmBytes.byteLength;
    const buf = new ArrayBuffer(44 + dataLen);
    const view = new DataView(buf);
    const byteRate = sampleRate * channels * (bitsPerSample / 8);
    const blockAlign = channels * (bitsPerSample / 8);

    const writeStr = (off, s) => {
        for (let i = 0; i < s.length; i++) view.setUint8(off + i, s.charCodeAt(i));
    };

    writeStr(0, 'RIFF');
    view.setUint32(4, 36 + dataLen, true);
    writeStr(8, 'WAVE');
    writeStr(12, 'fmt ');
    view.setUint32(16, 16, true); // PCM chunk size
    view.setUint16(20, 1, true); // PCM format
    view.setUint16(22, channels, true);
    view.setUint32(24, sampleRate, true);
    view.setUint32(28, byteRate, true);
    view.setUint16(32, blockAlign, true);
    view.setUint16(34, bitsPerSample, true);
    writeStr(36, 'data');
    view.setUint32(40, dataLen, true);
    new Uint8Array(buf, 44).set(pcmBytes);
    return buf;
}

// --- Microphone ---
async function startMic() {
    if (micActive) return;

    try {
        micStream = await navigator.mediaDevices.getUserMedia({
            audio: {
                sampleRate: SAMPLE_RATE,
                channelCount: 1,
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true,
            },
        });
    } catch (e) {
        addSystemMessage('Microphone access denied');
        return;
    }

    if (!audioCtx) {
        audioCtx = new AudioContext();
    }

    const source = audioCtx.createMediaStreamSource(micStream);

    // Use ScriptProcessor for PCM extraction (AudioWorklet alternative is more complex)
    const bufferSize = 4096;
    processor = audioCtx.createScriptProcessor(bufferSize, 1, 1);
    let residual = new Float32Array(0);
    const inputFrameSize = Math.max(1, Math.round(audioCtx.sampleRate / 50));

    processor.onaudioprocess = (e) => {
        if (!ws || ws.readyState !== WebSocket.OPEN) return;

        const samples = e.inputBuffer.getChannelData(0);

        // Merge with residual from previous callback
        const merged = new Float32Array(residual.length + samples.length);
        merged.set(residual);
        merged.set(samples, residual.length);

        // Send complete 20ms frames at the native input sample rate.
        let offset = 0;
        while (offset + inputFrameSize <= merged.length) {
            const frame = merged.subarray(offset, offset + inputFrameSize);
            const i16 = new Int16Array(inputFrameSize);
            for (let i = 0; i < inputFrameSize; i++) {
                const s = Math.max(-1, Math.min(1, frame[i]));
                i16[i] = s < 0 ? s * 32768 : s * 32767;
            }
            ws.send(i16.buffer);
            offset += inputFrameSize;
        }

        // Save remaining samples
        residual = merged.subarray(offset);

        // VU meter
        let sum = 0;
        for (let i = 0; i < samples.length; i++) sum += samples[i] * samples[i];
        const rms = Math.sqrt(sum / samples.length);
        vuBar.style.width = Math.min(100, rms * 500) + '%';
    };

    source.connect(processor);
    // ScriptProcessor requires a graph connection to fire onaudioprocess,
    // but we must NOT route mic audio to the speakers (feedback loop).
    // A zero-gain node silences the output while keeping the node active.
    const silentGain = audioCtx.createGain();
    silentGain.gain.value = 0;
    processor.connect(silentGain);
    silentGain.connect(audioCtx.destination);

    micActive = true;
    btnMic.classList.add('active');
    micLabel.textContent = 'Mic On';
    setStatus('listening', 'Listening...');
}

function stopMic() {
    if (processor) {
        processor.disconnect();
        processor = null;
    }
    if (micStream) {
        micStream.getTracks().forEach((t) => t.stop());
        micStream = null;
    }
    micActive = false;
    btnMic.classList.remove('active');
    micLabel.textContent = 'Mic Off';
    vuBar.style.width = '0%';
    if (ws && ws.readyState === WebSocket.OPEN) {
        setStatus('connected', 'Connected');
    }
}

function toggleMic() {
    if (micActive) stopMic();
    else startMic();
}

// --- Events ---
btnConnect.addEventListener('click', connect);
btnMic.addEventListener('click', toggleMic);

// Keyboard shortcut: Space to toggle mic
document.addEventListener('keydown', (e) => {
    if (
        e.code === 'Space' &&
        !e.repeat &&
        document.activeElement.tagName !== 'INPUT' &&
        document.activeElement.tagName !== 'SELECT'
    ) {
        e.preventDefault();
        if (ws && ws.readyState === WebSocket.OPEN) toggleMic();
    }
});
