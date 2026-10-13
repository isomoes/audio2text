# audio2text

Chinese audio-to-text transcription tool for Wayland using WeType (default) or DashScope.

## Features

- Streaming Chinese speech recognition using WeType or DashScope
- Global hotkey (Super+I) to toggle recording
- Automatic text input into active application
- Supports microphone input with automatic audio format conversion
- Wayland-compatible text input simulation

## Requirements

### System Dependencies

Direct input uses the compositor's `zwp_virtual_keyboard_manager_v1` protocol
(for example, Sway or Hyprland). No `wtype` or `ydotool` is required.
Run audio2text inside your Wayland session so `WAYLAND_DISPLAY` is available.

For the optional manual clipboard fallback:

```bash
# Arch Linux / Manjaro (clipboard only)
sudo pacman -S wl-clipboard
```

### Environment

WeType is the default and needs no account or API key:

```bash
audio2text
```

Audio is sent to the WeType cloud service at `wetype.weixin.qq.com`. This unofficial
protocol is ported from `wetype-x11`; server changes may break compatibility.
WeType appends finalized segments to the active application after about **1 second
of silence**, without sending Backspace or modifying text already typed. Recording
continues while the previous segment is finalized; pending audio is buffered and
segments are processed in order. When you stop (Ctrl+C, SIGTERM, or the silence
timeout), the last segment is also finalized. Partial results are logged only.

This is local **energy-based pause detection**, not a server-side sentence-final
marker or a semantic sentence detector. A 200ms pre-roll preserves quiet word
onsets; uninterrupted speech is split after 30 seconds. Output latency includes
the pause plus cloud finalization. Keep the intended input focused for each
segment's output. With only `wl-copy`, the latest finalized segment is copied for
manual pasting instead of being typed.
DashScope retains its existing sentence-final output behavior.

To use DashScope instead, select it and set your API key:

```bash
export DASHSCOPE_API_KEY="your_api_key_here"
export AUDIO2TEXT_ASR_BACKEND=dashscope
```

Or create a `.env` file in the project directory:

```env
DASHSCOPE_API_KEY=your_api_key_here
AUDIO2TEXT_ASR_BACKEND=dashscope
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

Source builds require ALSA, OpenSSL and libopus development libraries plus
`pkg-config` (Arch: `alsa-lib openssl opus pkgconf`; Debian/Ubuntu:
`libasound2-dev libssl-dev libopus-dev pkg-config`).

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
2. Stream audio to the selected ASR service for Chinese transcription
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
- `src/wetype/mod.rs` - WeType/wx-client protocol, identity registration and Opus streaming
- `src/audio/mod.rs` - Audio capture using cpal
- `src/input/mod.rs` - Text input simulation for Wayland

## Configuration

The application uses the following defaults:

- Sample rate: 16000 Hz (DashScope model: `qwen-audio-3.1-asr-flash-streaming`)
- Audio capture format: PCM 16-bit (WeType uploads 20ms Opus frames in 120ms batches)
- Recording chunk size: 100ms

Optional environment variables (can also go in `.env`):

| Variable | Default | Description |
|---|---|---|
| `AUDIO2TEXT_ASR_BACKEND` | `wetype` | Select `wetype` (alias `wx-client`) or `dashscope`. Only DashScope requires `DASHSCOPE_API_KEY`. |
| `AUDIO2TEXT_WETYPE_IDENTITY` | `~/.cache/audio2text/wetype-identity.json` | WeType device identity cache (respects `XDG_CACHE_HOME`, saved with mode 0600). Can point to the existing `wetype-x11/identity.json`. |
| `AUDIO2TEXT_WETYPE_SEGMENT_SILENCE_MS` | `1000` | Local pause duration that finalizes a segment (`200`-`6000` ms). Increase if short pauses split speech too often. |
| `AUDIO2TEXT_WETYPE_VOICE_THRESHOLD` | `0.01` | Local normalized PCM RMS threshold (`0` < value <= `1`). Lower for quiet speech; raise if background noise prevents detecting pauses. This is separate from DashScope VAD settings. |
| `AUDIO2TEXT_LANGUAGE_HINTS` | `zh,en` | Comma-separated language codes (max 4). `zh,en` handles English terms mixed into Chinese speech. |
| `AUDIO2TEXT_HOTWORDS` | `~/.config/audio2text/hotwords.txt` (if it exists) | Hotword file to improve recognition of names and jargon. |
| `AUDIO2TEXT_VAD_MODEL` | `far_field_meeting_16k` | Server-side voice activity detection. `far_field_meeting_16k` produces longer segments (more context, better accuracy in our tests); `near_meeting_16k` splits more eagerly on short pauses. |
| `AUDIO2TEXT_SPEECH_NOISE_THRESHOLD` | unset (server default) | VAD sensitivity in `[-1.0, 1.0]`. Lower values keep more speech (try `-0.2` if the first word of a sentence gets dropped); higher values filter more noise. |
| `AUDIO2TEXT_MAX_SENTENCE_SILENCE` | unset (server default `1300`) | Silence in ms (`200`-`6000`) that ends a sentence. Higher values give longer segments with more context, but text appears later. |
| `AUDIO2TEXT_SEMANTIC_PUNCTUATION` | unset (server default `false`) | `true` splits sentences by meaning instead of by silence (more accurate segmentation, higher latency). `AUDIO2TEXT_MAX_SENTENCE_SILENCE` then no longer decides sentence ends. |
| `AUDIO2TEXT_DUMP_WAV` | unset | Write the exact audio sent to the ASR service to this WAV file, for debugging accuracy. |

Language hints, hotwords and VAD/punctuation options apply only to DashScope.
`AUDIO2TEXT_DUMP_WAV` works with either backend and saves PCM before encoding.

### Hotwords (DashScope only)

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

### "Native Wayland input unavailable"
- Run from the same Wayland session as the target application.
- The compositor must expose `zwp_virtual_keyboard_manager_v1`.
- Without that protocol, install `wl-clipboard` for manual clipboard-based input.

### Browser drops characters or exits the input with Surfingkeys enabled
- Use the new binary with startup message `Using native Wayland virtual keyboard with printable keycodes`.
- The built-in keyboard maps printable text only to printable evdev keycodes,
  not Esc, Backspace or modifier keys. It sends no `VoidSymbol` events.
- Long segments use multiple keymaps; the virtual keyboard stays connected
  across segments. Keep the intended input focused when text arrives.

### Audio quality issues
- Check your microphone settings in PulseAudio/PipeWire
- Ensure the correct input device is selected
- Run with `AUDIO2TEXT_DUMP_WAV=/tmp/sent.wav audio2text` and listen to the file: if it sounds bad, fix the input (level, device, noise) before tuning the model

### First word of a sentence is dropped
- Check the startup log line `Using config: ...`: the `sample_format` must not be `U8`/`I8` (8-bit capture erases quiet word onsets)
- Record with `AUDIO2TEXT_DUMP_WAV` and check that word starts are audible
- If it still happens, lower the VAD threshold step by step: `AUDIO2TEXT_SPEECH_NOISE_THRESHOLD=-0.1`, then `-0.2`, ...

### Proxy
Both WebSocket clients connect directly and ignore `HTTP(S)_PROXY`.

## License

MIT
