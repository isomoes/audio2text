# Architecture

## Overview

This speech-to-text application captures microphone audio, sends it to WeType (default) or Alibaba DashScope, and types the transcription using external input tools.

The application is designed to be triggered by external hotkey software (e.g., AutoKey, sxhkd) rather than having built-in global hotkey functionality.

## Components

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   Main Process  │────▶│  Audio Capture  │────▶│  Microphone     │
│   (main.rs)     │     │   (audio/)      │     │  (cpal)         │
└────────┬────────┘     └─────────────────┘     └─────────────────┘
         │
         ├─────────────────────────────────────────────────────────┐
         │                                                         │
         ▼                                                         ▼
┌─────────────────┐                                     ┌─────────────────┐
│   WebSocket     │◀─────── PCM Audio Data ───────────▶│   ASR Client    │
│   (websocket/)  │                                     │ (DashScope API) │
└────────┬────────┘                                     └─────────────────┘
         │
         │ Chinese Transcription Events
         ▼
┌─────────────────┐
│  Text Input     │────▶ External tools (wtype, ydotool, wl-copy)
│   (input/)      │
└─────────────────┘
```

## Modules

### `main.rs`

The main entry point and application coordinator.

**Responsibilities:**
- Initialize logging and configuration
- Create and manage the `App` state
- Handle Ctrl+C shutdown signal
- Coordinate between audio capture, ASR client, and text input

**Key Functions:**
- `main()`: Entry point, sets up the app and waits for shutdown
- `check_dependencies()`: Verifies required external tools are installed
- `App::start_recording()`: Initiates audio capture and ASR
- `App::stop_recording()`: Gracefully stops all processes

### `audio/mod.rs`

Audio capture using the `cpal` library.

**Responsibilities:**
- Enumerate and select audio input devices
- Capture raw audio data from the microphone
- Convert audio to mono and resample to 16kHz
- Convert to PCM format for the ASR API
- Optionally dump the sent audio to a WAV file (`audio/wav.rs`, enabled by `AUDIO2TEXT_DUMP_WAV`)

**Key Features:**
- Supports all sample formats (I8, I16, I32, I64, U8, U16, U32, U64, F32, F64)
- Automatic stereo-to-mono downmixing
- Band-limited, stateful sample rate conversion to 16kHz using `rubato` (FFT resampler, no aliasing or chunk-boundary artifacts)
- Chunked audio delivery (100ms chunks)

**Target Audio Format:**
- Sample Rate: 16000 Hz
- Channels: 1 (mono)
- Format: 16-bit PCM little-endian

### `websocket/mod.rs`

WebSocket client for Alibaba DashScope ASR API.

Selected with `AUDIO2TEXT_ASR_BACKEND=dashscope` and requires `DASHSCOPE_API_KEY`.

**Responsibilities:**
- Establish WebSocket connection with authentication
- Send run-task command with configuration
- Stream audio data to the API
- Receive and parse Chinese transcription events

**Protocol Flow:**
1. Connect to `wss://dashscope.aliyuncs.com/api-ws/v1/inference/`
2. Send `run-task` command with parameters
3. Wait for `task-started` event
4. Stream binary audio data
5. Receive `result-generated` events with Chinese transcriptions
6. Send `finish-task` command on completion

**Current Configuration:**
```rust
parameters: {
    format: "pcm",
    sample_rate: 16000,
    language_hints: ["zh", "en"],        // AUDIO2TEXT_LANGUAGE_HINTS
    vocabulary: { "hotword": weight },   // instant hotwords from the hotwords file, omitted if empty
    vad_model: "far_field_meeting_16k",  // AUDIO2TEXT_VAD_MODEL
    speech_noise_threshold: f32,         // AUDIO2TEXT_SPEECH_NOISE_THRESHOLD, omitted if unset
    max_sentence_silence: u32,           // AUDIO2TEXT_MAX_SENTENCE_SILENCE, omitted if unset
    semantic_punctuation_enabled: bool   // AUDIO2TEXT_SEMANTIC_PUNCTUATION, omitted if unset
}
```

Runtime configuration is loaded from environment variables in `config.rs` (see README).

**Event Types:**
- `task-started`: ASR session initialized
- `result-generated`: Partial or final Chinese transcription
- `task-finished`: Session completed successfully
- `task-failed`: Error occurred

### `wetype/mod.rs`

Default ASR backend (`AUDIO2TEXT_ASR_BACKEND=wetype`, alias `wx-client`), ported
from `wetype-x11/main.go`. Uses the existing 16kHz mono PCM capture, encodes
20ms Opus frames, and uploads six frames per request over a `wxws_pb` WebSocket.
The protocol uses protobuf-wire messages, secp128r1 ECDH, AES-256-ECB with PKCS#7,
signed headers and raw Snappy compression. Device registration is automatic;
the identity is cached at `$XDG_CACHE_HOME/audio2text/wetype-identity.json`
(or `~/.cache/audio2text/wetype-identity.json`) with mode 0600. Override with
`AUDIO2TEXT_WETYPE_IDENTITY` to reuse a wetype-x11 identity.

`wetype/segment.rs` independently detects local pauses using the RMS energy of
20ms PCM frames (default threshold 0.01). It retains 200ms pre-roll, closes a
segment after 1000ms silence, and splits continuous speech at 30 seconds.
`AUDIO2TEXT_WETYPE_SEGMENT_SILENCE_MS` and `AUDIO2TEXT_WETYPE_VOICE_THRESHOLD`
configure pause detection. This is not a server-side sentence-final marker.

A bounded 30-second frame queue separates capture/segmentation from sequential
network requests. Each segment has a fresh voice ID and Opus encoder; the
authenticated connection is reused. Finalization polls for polished text,
falling back to the latest partial. Exactly one final result per segment is
appended by the input handler, without Backspace. Partials are logged only.
When capture stops, queued audio is drained and the last active segment is
finalized (padding an incomplete frame). DashScope still types only finalized
sentences. Ctrl+C and SIGTERM both
allow finalization before exit. No API key is required. This is an unofficial
protocol and sends audio to `wetype.weixin.qq.com`.

### `input/mod.rs`

Text input handler that simulates keyboard typing.

**Responsibilities:**
- Type text into the active window
- Fallback to clipboard if direct typing fails

**Supported Tools (in order of preference):**
1. `wtype` - Wayland native input tool
2. `ydotool` - Universal input tool (requires daemon)
3. `wl-copy` - Clipboard fallback (Wayland)

**Implementation:**
- Uses `std::process::Command` to invoke external tools
- Only finalized segments/sentences are typed, once each. No Backspace is sent.
- Clipboard-only input copies the latest finalized segment for manual pasting.
- Errors are logged, without retrying possibly partially typed text. Keep the
  intended input focused when finalized text arrives.

## Data Flow

```
Microphone → cpal → Audio Samples → Format Conversion → Mono/Resample → PCM → WebSocket → DashScope → Chinese Transcription → Text Input → Active Window
```

## Threading Model

The application uses Tokio's async runtime for concurrency:

1. **Main Task**: Runs the event loop and waits for shutdown
2. **Audio Thread**: Created by cpal, captures audio via callback
3. **ASR Client Task**: Handles WebSocket communication and events
4. **Event Handler Task**: Processes ASR results and triggers text input
5. **Signal Handler Task**: Listens for Ctrl+C and initiates shutdown

All tasks communicate via Tokio channels (`mpsc`).

## Building and Running

```bash
# Development build
cargo build

# Release build (recommended)
cargo build --release

# Run directly
cargo run --release

# Run the release binary
./target/release/audio2text
```

## Environment Variables

- `AUDIO2TEXT_ASR_BACKEND`: `wetype` (default) or `dashscope`
- `DASHSCOPE_API_KEY`: Required only for the DashScope backend
- `RUST_LOG`: (Optional) Log level (e.g., `debug`, `info`, `warn`)

## External Hotkey Setup

Since this application starts recording immediately and exits on Ctrl+C, it's meant to be triggered by external hotkey software:

**Example with sxhkd (Wayland):**
```
# ~/.config/sxhkd/sxhkdrc
super + i
    /path/to/audio2text
```

**Example with AutoKey:**
Create a hotkey that runs the command `/path/to/audio2text`

**Example with systemd user service:**
Create a service that can be started/stopped via hotkey.

## Dependencies

### Runtime
- `cpal` - Audio capture
- `tokio` - Async runtime
- `tokio-tungstenite` - WebSocket client
- `libopus` - WeType audio encoding
- `wtype` or `ydotool` - Text input (external)

### Development
- `anyhow` - Error handling
- `serde` - Serialization
- `tracing` - Logging
- `uuid` - Unique task IDs
