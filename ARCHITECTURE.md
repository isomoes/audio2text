# Architecture

## Overview

This is a real-time speech-to-text application that captures audio from the microphone, sends it to Alibaba DashScope's ASR (Automatic Speech Recognition) API, and types out the translated text using external input tools.

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
- Convert audio to mono and resample to 8kHz
- Convert to PCM format for the ASR API

**Key Features:**
- Supports all sample formats (I8, I16, I32, I64, U8, U16, U32, U64, F32, F64)
- Automatic stereo-to-mono downmixing
- Sample rate conversion to 16kHz (recommended by the selected ASR model)
- Chunked audio delivery (100ms chunks)

**Target Audio Format:**
- Sample Rate: 16000 Hz
- Channels: 1 (mono)
- Format: 16-bit PCM little-endian

### `websocket/mod.rs`

WebSocket client for Alibaba DashScope ASR API.

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
    language_hints: ["zh"]
}
```

**Event Types:**
- `task-started`: ASR session initialized
- `result-generated`: Partial or final Chinese transcription
- `task-finished`: Session completed successfully
- `task-failed`: Error occurred

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
- Each word is typed individually for better real-time feedback
- Errors are logged but don't stop the application

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

- `DASHSCOPE_API_KEY`: (Required) Alibaba DashScope API key
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
- `wtype` or `ydotool` - Text input (external)

### Development
- `anyhow` - Error handling
- `serde` - Serialization
- `tracing` - Logging
- `uuid` - Unique task IDs
