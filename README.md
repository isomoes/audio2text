# audio2text

Real-time Chinese audio-to-text transcription tool for Wayland using the DashScope API.

## Features

- Real-time Chinese speech recognition using DashScope WebSocket API
- Global hotkey (Super+I) to toggle recording
- Automatic text input into active application
- Supports microphone input with automatic audio format conversion
- Wayland-compatible text input simulation

## Requirements

### System Dependencies

Install the required tools for Wayland text input:

```bash
# Arch Linux / Manjaro
sudo pacman -S wtype

# Or alternative (ydotool)
sudo pacman -S ydotool

# Or fallback (clipboard only)
sudo pacman -S wl-clipboard
```

### Environment

Set your DashScope API key:

```bash
export DASHSCOPE_API_KEY="your_api_key_here"
```

Or create a `.env` file in the project directory:

```env
DASHSCOPE_API_KEY=your_api_key_here
```

## Installation

### Install from source

```bash
# Install globally (adds to ~/.cargo/bin)
cargo install --path .

# Verify installation
which audio2text
```

The binary will be installed to `~/.cargo/bin/audio2text`. Make sure `~/.cargo/bin` is in your PATH.

### Sway Configuration

To use audio2text with a toggle hotkey in Sway, add the following to your `~/.config/sway/config`:

```bash
# Toggle audio-to-text recording with Super+I
bindsym $mod+Shift+i exec --no-startup-id \
    if pidof -q audio2text; then \
        pkill audio2text && notify-send "Audio2Text Stopped"; \
    else \
        audio2text && notify-send "Audio2Text Started"; \
    fi
```

Reload your Sway configuration:

```bash
swaymsg reload
```

Now pressing `Super+Shift+I` will start/stop the audio-to-text service.

## Usage

### Direct execution

```bash
audio2text
```

The application will:
1. Start recording immediately from your microphone
2. Stream audio to DashScope for real-time Chinese transcription
3. Type the recognized Chinese text into your active application
4. Continue until you press `Ctrl+C` to exit

### Via Sway hotkey (Recommended)

After configuring Sway (see above):
1. Press `Super+Shift+I` to start recording
2. Speak into your microphone
3. Press `Super+Shift+I` again to stop recording

This is the recommended way to use audio2text in a Sway environment.

## Project Structure

- `src/main.rs` - Main application logic and state management
- `src/config.rs` - Environment-based configuration (language hints, hotwords, WAV dump)
- `src/websocket/mod.rs` - DashScope WebSocket client implementation
- `src/audio/mod.rs` - Audio capture using cpal
- `src/input/mod.rs` - Text input simulation for Wayland

## Configuration

The application uses the following defaults:

- Sample rate: 16000 Hz (`qwen-audio-3.1-asr-flash-streaming`)
- Audio format: PCM 16-bit
- Recording chunk size: 100ms

Optional environment variables (can also go in `.env`):

| Variable | Default | Description |
|---|---|---|
| `AUDIO2TEXT_LANGUAGE_HINTS` | `zh,en` | Comma-separated language codes (max 4). `zh,en` handles English terms mixed into Chinese speech. |
| `AUDIO2TEXT_HOTWORDS` | `~/.config/audio2text/hotwords.txt` (if it exists) | Hotword file to improve recognition of names and jargon. |
| `AUDIO2TEXT_VAD_MODEL` | `far_field_meeting_16k` | Server-side voice activity detection. `far_field_meeting_16k` produces longer segments (more context, better accuracy in our tests); `near_meeting_16k` splits more eagerly on short pauses. |
| `AUDIO2TEXT_SPEECH_NOISE_THRESHOLD` | unset (server default) | VAD sensitivity in `[-1.0, 1.0]`. Lower values keep more speech (try `-0.2` if the first word of a sentence gets dropped); higher values filter more noise. |
| `AUDIO2TEXT_MAX_SENTENCE_SILENCE` | unset (server default `1300`) | Silence in ms (`200`-`6000`) that ends a sentence. Higher values give longer segments with more context, but text appears later. |
| `AUDIO2TEXT_SEMANTIC_PUNCTUATION` | unset (server default `false`) | `true` splits sentences by meaning instead of by silence (more accurate segmentation, higher latency). `AUDIO2TEXT_MAX_SENTENCE_SILENCE` then no longer decides sentence ends. |
| `AUDIO2TEXT_DUMP_WAV` | unset | Write the exact audio sent to the ASR service to this WAV file, for debugging accuracy. |

### Hotwords

One hotword per line, optionally followed by a weight: `1`-`5` (higher = stronger bias, default `4`) or `50` for a "super" hotword (max 50 of these). Up to 2000 hotwords. Lines starting with `#` are comments.

```text
# ~/.config/audio2text/hotwords.txt
通义千问 5
WebSocket
Claude Code 5
Sway
```

Hotkey configuration is handled by your window manager (see Sway Configuration above).

## Troubleshooting

### "No input device available"
- Ensure your microphone is connected and recognized by the system
- Check with `pavucontrol` or `wpctl` (PipeWire)

### "No suitable text input method found"
- Install `wtype` (recommended) or `ydotool` for Wayland
- Alternatively, install `wl-clipboard` for clipboard-based input

### Audio quality issues
- Check your microphone settings in PulseAudio/PipeWire
- Ensure the correct input device is selected
- Run with `AUDIO2TEXT_DUMP_WAV=/tmp/sent.wav audio2text` and listen to the file: if it sounds bad, fix the input (level, device, noise) before tuning the model

### First word of a sentence is dropped
- Check the startup log line `Using config: ...`: the `sample_format` must not be `U8`/`I8` (8-bit capture erases quiet word onsets)
- Record with `AUDIO2TEXT_DUMP_WAV` and check that word starts are audible
- If it still happens, lower the VAD threshold step by step: `AUDIO2TEXT_SPEECH_NOISE_THRESHOLD=-0.1`, then `-0.2`, ...

### Proxy
The WebSocket client connects to DashScope directly and ignores `HTTP(S)_PROXY`. This is intended: DashScope is a China endpoint, and in tests a local HTTP proxy added ~3 s to connection setup and ~2 s to result latency with no accuracy benefit.

## License

MIT
