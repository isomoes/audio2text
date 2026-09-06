---
title: "Audio2Text: Real-Time Speech-to-Text for AI Workflows"
description: A comprehensive guide to Audio2Text, a real-time Chinese speech recognition tool that enhances AI-driven development workflows
date: 2026-01-14
author: isomo
tags: [audio2text, ai-workflow]
video: "https://www.bilibili.com/video/BV1uErWBXESE/"
---

# Introducing Audio2Text: A Real-Time Speech-to-Text Workflow Enhancement Tool

## Overview

In our modern AI-powered development workflow, seamless input methods are crucial for maximizing productivity. We've developed **Audio2Text**, a real-time audio-to-text transcription tool that transforms spoken language into typed text automatically, enabling hands-free interaction with your development environment.

What makes Audio2Text useful is its ability to capture and transcribe Chinese speech in real-time. This makes it an ideal tool for:

- **Chinese speakers** who want to work more efficiently with English-based AI systems
- **Language learners** who want to practice speaking while getting instant written output
- **Anyone** who prefers voice input over typing, especially when Chinese input methods are slow

This tool is particularly useful when working with AI assistants like OpenCoder, or multi-agent systems, allowing you to naturally communicate while maintaining your workflow momentum.

---

## How Audio2Text Integrates with Your AI Workflow

### Use Case Scenarios

Audio2Text is designed to work seamlessly with your existing AI development ecosystem. Here's how it fits into various workflows:

#### 1. OpenCoder Integration

When you're working with OpenCoder for code generation and review, Audio2Text allows you to:

- Verbally describe coding requirements without leaving your editor
- Dictate complex architectural explanations that get transcribed directly into OpenCoder's input field
- Provide natural language feedback on generated code while keeping your hands on the keyboard for navigation

#### 2. AI Agent Orchestration

When working with multiple AI agents:

- Give natural language instructions to specific agents
- Provide context and feedback through spoken communication
- Coordinate between agents while keeping your hands free for other tasks

### Workflow Integration Diagram

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   Microphone    │────▶│   Audio2Text    │────▶│  Active App     │
│   (Chinese      │     │   (Real-time    │     │  (OpenCoder,   │
│    Voice)       │     │   ASR +         │     │   WebBoard,    │
│                 │     │   Chinese ASR  │     │   etc.)         │
└─────────────────┘     │   directly     │     └─────────────────┘
                        └────────┬────────┘              │
                                │                        │
                                ▼                        ▼
                         ┌─────────────────┐     ┌─────────────────┐
                         │  DashScope API │────▶│  AI Engine     │
                          │  (Chinese      │     │  (AI Engine     │
                          │   Transcription)│    │   Input)        │
                         └─────────────────┘     └─────────────────┘
```

---

## Design Motivation and Architecture

### Personal Motivations

Before diving into the technical design, let us share our personal reasons for building Audio2Text. These motivations shaped every aspect of the project:

#### 1. **Bridging Language Barriers**

While we are native Chinese speakers, we recognize that English is the preferred language for interacting with Large Language Models (LLMs) and most AI tools. The quality of responses from models like GPT, Claude, and others tends to be better when using English prompts. However, our English proficiency isn't perfect, and typing in English can be time-consuming.

Audio2Text solves this by allowing us to speak in Chinese and automatically transcribe it to Chinese text in real-time. This lets us express our ideas naturally in our native language while keeping the original wording available in the active application.

#### 2. **Efficiency: Voice vs. Typing Chinese**

Chinese input methods are inherently slower than English typing. Each character requires pinyin input followed by selecting the correct character from a menu, which disrupts flow state and slows down communication. Voice input in Chinese, on the other hand, is much faster and more natural.

By using voice as the primary input method, we can convey ideas at conversational speed rather than typing speed, making the interaction with AI tools significantly more efficient.

#### 3. **Building Confidence in Expression**

We've found that when we speak, we're often more expressive and confident in conveying complex ideas. Writing can sometimes feel constrained, especially in a second language. Voice allows for more natural intonation, emphasis, and emotional context that gets lost in typed text.

Audio2Text gives us the opportunity to practice speaking more regularly and improve our Chinese verbal expression.

#### 4. **Structured Thinking Through Speech**

There's something about the act of speaking that encourages more structured thinking. When you have to explain something out loud, you naturally organize your thoughts more coherently. By making voice input a regular part of our workflow, we're training ourselves to think and express ideas more clearly and systematically.

### Technical Problems We Solved

Beyond our personal motivations, we also addressed several technical pain points in our day-to-day development work:

1. **Context Switching Costs**: Moving between speaking and typing breaks the flow state
2. **Repetitive Input**: Entering similar prompts or feedback multiple times across different tools
3. **Language Barrier Optimization**: Native Chinese users can work more efficiently with English-based AI tools
4. **Speed Constraints**: Spoken language (especially in Chinese) conveys complex ideas faster than typing Chinese characters

### Design Philosophy

Our design principles guided every architectural decision:

#### 1. **Non-Intrusive Integration**

The tool runs as a background service that can be triggered on-demand. It doesn't interfere with your existing workflow but enhances it when needed.

#### 2. **Real-Time Performance**

- Uses WebSocket streaming for immediate transcription feedback
- Provides partial results as you speak, with final confirmation when sentences complete
- Auto-detects speech activity and stops after 60 seconds of silence

#### 3. **Technology Stack Selection**

**Why Rust?**

- Performance and safety for real-time audio processing
- Cross-platform compatibility (currently Wayland-optimized)
- Robust async runtime (Tokio) for handling concurrent operations

**Why DashScope API?**

- Real-time streaming support via WebSocket
- Built-in real-time transcription capabilities
- Reliable and cost-effective for production use
- Low latency for natural conversational flow

**Why Wayland Integration?**

- Modern Linux display protocol with better security
- Direct text input using native tools (wtype, ydotool)
- Works seamlessly with Sway and other Wayland compositors

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        Audio2Text Application                  │
├─────────────────┬─────────────────┬─────────────────────────────┤
│  Audio Capture  │  WebSocket ASR  │  Text Input                 │
│  (cpal)         │  (DashScope)    │  (wtype/ydotool)           │
└────────┬────────┴────────┬────────┴──────────────────────┬────┘
         │                 │                                 │
         ▼                 ▼                                 ▼
   Microphone Input    Real-time        Active Application
    (PCM 16kHz)         Chinese ASR      (Auto-typing)
```

### Key Features

#### Audio Processing

- Automatic sample rate conversion to 16kHz
- Stereo to mono downmixing
- Support for multiple audio formats (I8, I16, I32, F32, etc.)
- 100ms chunk-based streaming for minimal latency

#### Speech Recognition

- Model: `qwen-audio-3.0-asr-flash-streaming` from DashScope
- Direct Chinese transcription without translation
- Real-time streaming via WebSocket protocol
- Speech activity detection for auto-stop functionality

#### Text Output

- Multiple fallback options: wtype → ydotool → wl-copy
- Direct typing into active window
- Word-by-word output for immediate feedback
- Error handling with graceful degradation

---

## Integration into Your Workflow

### Quick Start Setup

#### 1. Installation

```bash
# Clone and build
cargo install --path .

# Set up API key
export DASHSCOPE_API_KEY="your_api_key_here"
```

#### 2. Configure Window Manager

For Sway users, add this to your `~/.config/sway/config`:

```bash
# Toggle audio-to-text with Super+Shift+I
bindsym $mod+Shift+i exec --no-startup-id \
    if pidof -q audio2text; then \
        pkill audio2text && notify-send "Audio2Text Stopped"; \
    else \
        audio2text && notify-send "Audio2Text Started"; \
    fi
```

#### 3. Usage

1. Press `Super+Shift+I` to start recording
2. Speak into your microphone
3. Watch text appear automatically in your active application
4. Press `Super+Shift+I` again to stop (or wait for 60-second auto-stop)

### Backend Model and Cost

#### API Model Selection

We chose **Alibaba DashScope's `qwen-audio-3.0-asr-flash-streaming`** model for several reasons:

- **Real-time streaming**: Low-latency transcription suitable for live typing
- **Chinese recognition**: Direct transcription with the source language set to Chinese
- **Cost-effective**: Competitive pricing for development and production use
- **Reliability**: Enterprise-grade uptime and support

#### Cost Estimation

Based on our testing and typical usage patterns:

- **Estimated cost**: ~1 Chinese Yuan (CNY) per 2 hours of active recording
- **Calculation**:
  - DashScope ASR pricing: check the current DashScope pricing for the selected region

**Note**: Costs may vary based on actual usage, audio length, and region-specific pricing. Always check the latest DashScope pricing for accurate estimates.

### Advanced Configuration

#### Environment Variables

```bash
# Required
DASHSCOPE_API_KEY=your_api_key_here

# Optional - Adjust log level
RUST_LOG=info  # Options: debug, info, warn, error
```

#### System Requirements

**Wayland Environment** (Required):

- Sway, GNOME Wayland, or other Wayland compositor

**Input Tools** (at least one):

- `wtype` (recommended) - Native Wayland input
- `ydotool` - Universal input tool (requires daemon)
- `wl-copy` - Clipboard fallback

**Audio System**:

- PulseAudio or PipeWire
- Working microphone device

---

## Conclusion

Audio2Text represents more than just a productivity tool—it's a personal journey of improvement and innovation. By bridging voice input and text-based AI workflows, we've created a tool that not only enhances developer productivity but also serves as a platform for language learning and self-expression.

For us, this project is about:

- **Preserving meaning** through direct Chinese transcription
- **Improving efficiency** by leveraging voice over slow Chinese input methods
- **Building confidence** in Chinese verbal expression
- **Developing structured thinking** through the natural flow of speech

Whether you're a native English speaker looking to optimize your workflow, or someone who speaks Chinese and wants to enter Chinese text more efficiently, Audio2Text offers a path to more natural and efficient communication.

Most importantly, this is a tool that grows with you. As you use it to speak and interact with AI systems, you'll naturally improve your language skills, become more comfortable with expression, and discover new ways to structure your thoughts.

**Ready to get started?** Check out our [GitHub repository](https://github.com/jiahaoxiang2000/audio2text) for the latest code and documentation.
