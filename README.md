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
- `src/websocket/mod.rs` - DashScope WebSocket client implementation
- `src/audio/mod.rs` - Audio capture using cpal
- `src/input/mod.rs` - Text input simulation for Wayland

## Configuration

The application uses the following defaults:

- Sample rate: 16000 Hz (`qwen-audio-3.0-asr-flash-streaming`)
- Audio format: PCM 16-bit
- Recording chunk size: 100ms

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

## License

MIT
