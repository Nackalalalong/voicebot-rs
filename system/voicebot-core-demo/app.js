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
let micStream = null;
let processor = null;
let micActive = false;
let currentBotMsg = null; // accumulates streaming agent text
let currentUserMsg = null; // accumulates streaming transcript

// Audio playback queue
let playbackQueue = [];
let isPlaying = false;

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

// --- WebSocket ---
function connect() {
    if (ws) { disconnect(); return; }

    const url = wsUrlInput.value.trim();
    if (!url) return;

    setStatus('connecting', 'Connecting...');
    ws = new WebSocket(url);
    ws.binaryType = 'arraybuffer';

    ws.onopen = () => {
        setStatus('connected', 'Connected');
        btnConnect.textContent = 'Disconnect';
        btnConnect.classList.add('active');
        btnMic.disabled = false;
        addSystemMessage('Connected to server');

        // Send session_start
        ws.send(JSON.stringify({
            type: 'session_start',
            language: languageSelect.value,
            asr: 'speaches',
            tts: 'speaches',
        }));
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
        ws = null;
    };

    ws.onerror = () => {
        addSystemMessage('Connection error');
    };
}

function disconnect() {
    if (ws) {
        ws.send(JSON.stringify({ type: 'session_end' }));
        ws.close();
    }
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
            }
            currentBotMsg = null;
            setStatus('speaking', 'Speaking...');
            break;

        case 'error':
            addSystemMessage(`Error: ${msg.code}`);
            break;
    }
}

// --- Audio playback ---
function handleServerAudio(arrayBuffer) {
    // Binary frames are raw PCM i16 LE at 16kHz mono
    playbackQueue.push(arrayBuffer);
    if (!isPlaying) drainPlaybackQueue();
}

async function drainPlaybackQueue() {
    if (!audioCtx) {
        audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });
    }
    isPlaying = true;

    while (playbackQueue.length > 0) {
        const buf = playbackQueue.shift();
        const i16 = new Int16Array(buf);
        const floats = new Float32Array(i16.length);
        for (let i = 0; i < i16.length; i++) {
            floats[i] = i16[i] / 32768;
        }

        const audioBuf = audioCtx.createBuffer(1, floats.length, SAMPLE_RATE);
        audioBuf.getChannelData(0).set(floats);

        const src = audioCtx.createBufferSource();
        src.buffer = audioBuf;
        src.connect(audioCtx.destination);

        await new Promise((resolve) => {
            src.onended = resolve;
            src.start();
        });
    }

    isPlaying = false;
    if (ws && ws.readyState === WebSocket.OPEN) {
        setStatus('connected', 'Connected');
    }
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
            }
        });
    } catch (e) {
        addSystemMessage('Microphone access denied');
        return;
    }

    if (!audioCtx) {
        audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });
    }

    // Resample if browser context rate differs from 16kHz
    const source = audioCtx.createMediaStreamSource(micStream);

    // Use ScriptProcessor for PCM extraction (AudioWorklet alternative is more complex)
    const bufferSize = 4096;
    processor = audioCtx.createScriptProcessor(bufferSize, 1, 1);
    let residual = new Float32Array(0);

    processor.onaudioprocess = (e) => {
        if (!ws || ws.readyState !== WebSocket.OPEN) return;

        const input = e.inputBuffer.getChannelData(0);

        // If AudioContext sampleRate != 16kHz, resample
        let samples;
        if (audioCtx.sampleRate !== SAMPLE_RATE) {
            const ratio = audioCtx.sampleRate / SAMPLE_RATE;
            const outLen = Math.floor(input.length / ratio);
            samples = new Float32Array(outLen);
            for (let i = 0; i < outLen; i++) {
                samples[i] = input[Math.floor(i * ratio)];
            }
        } else {
            samples = input;
        }

        // Merge with residual from previous callback
        const merged = new Float32Array(residual.length + samples.length);
        merged.set(residual);
        merged.set(samples, residual.length);

        // Send complete 20ms frames (320 samples)
        let offset = 0;
        while (offset + FRAME_SIZE <= merged.length) {
            const frame = merged.subarray(offset, offset + FRAME_SIZE);
            const i16 = new Int16Array(FRAME_SIZE);
            for (let i = 0; i < FRAME_SIZE; i++) {
                const s = Math.max(-1, Math.min(1, frame[i]));
                i16[i] = s < 0 ? s * 32768 : s * 32767;
            }
            ws.send(i16.buffer);
            offset += FRAME_SIZE;
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
    processor.connect(audioCtx.destination); // required for ScriptProcessor to fire

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
        micStream.getTracks().forEach(t => t.stop());
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
    if (e.code === 'Space' && !e.repeat && document.activeElement.tagName !== 'INPUT' && document.activeElement.tagName !== 'SELECT') {
        e.preventDefault();
        if (ws && ws.readyState === WebSocket.OPEN) toggleMic();
    }
});
