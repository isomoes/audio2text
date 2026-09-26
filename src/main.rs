mod audio;
mod config;
mod input;
mod websocket;

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tracing::{error, info, warn};

use audio::AudioCapture;
use config::Config;
use input::TextInputHandler;
use websocket::{AsrClient, AsrEvent};

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppState {
    Idle,
    Recording,
}

/// Maximum silence duration before auto-stop (60 seconds of no speech detected by ASR)
const MAX_SILENCE_SECONDS: u64 = 60;

struct App {
    state: AppState,
    audio_capture: AudioCapture,
    text_input: TextInputHandler,
    api_key: String,
    config: Config,
    current_text: String,
    audio_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    last_asr_result: Arc<AtomicBool>,
}

impl App {
    fn new(api_key: String, config: Config) -> Self {
        Self {
            state: AppState::Idle,
            audio_capture: AudioCapture::new(),
            text_input: TextInputHandler::new(),
            api_key,
            config,
            current_text: String::new(),
            audio_tx: None,
            last_asr_result: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn start_recording(&mut self) -> Result<()> {
        info!("Starting recording...");

        // Reset ASR result flag
        self.last_asr_result.store(false, Ordering::SeqCst);

        // Create channels
        let (audio_tx, audio_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);
        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<AsrEvent>(100);

        // Store audio sender for later use
        self.audio_tx = Some(audio_tx.clone());

        // Share the ASR result flag with the event handler
        let last_asr_result = self.last_asr_result.clone();

        // Start audio capture
        self.audio_capture.start(audio_tx)?;

        let audio_rx = match &self.config.dump_wav {
            Some(path) => audio::wav::tee_to_wav(audio_rx, path)?,
            None => audio_rx,
        };

        // Start ASR client
        let api_key = self.api_key.clone();
        let config = self.config.clone();
        tokio::spawn(async move {
            let mut client = AsrClient::new(api_key, config);
            if let Err(e) = client.start_recognition(audio_rx, event_tx).await {
                error!("ASR client error: {}", e);
            }
        });

        // Handle ASR events in a separate task
        let text_input = self.text_input.clone();
        tokio::spawn(async move {
            let mut event_rx = event_rx;
            while let Some(event) = event_rx.recv().await {
                match event {
                    AsrEvent::TaskStarted => {
                        info!("ASR task started");
                    }
                    AsrEvent::ResultGenerated { text, is_final } => {
                        // Update flag when we receive any ASR result (speech detected)
                        last_asr_result.store(true, Ordering::SeqCst);

                        if is_final {
                            // Type the final text
                            if let Err(e) = text_input.type_text(&text) {
                                error!("Failed to type text: {}", e);
                            }
                            info!("Final: {}", text);
                        } else {
                            // Partial result
                            info!("Partial: {}", text);
                        }
                    }
                    AsrEvent::TaskFinished => {
                        info!("ASR task finished");
                        break;
                    }
                    AsrEvent::TaskFailed { error } => {
                        error!("ASR task failed: {}", error);
                        break;
                    }
                }
            }
        });

        self.state = AppState::Recording;
        info!("Recording started. Will auto-stop after {} seconds of silence.", MAX_SILENCE_SECONDS);
        info!("Press Ctrl+C to stop manually.");

        Ok(())
    }

    async fn stop_recording(&mut self) -> Result<()> {
        info!("Stopping recording...");

        // Stop audio capture (this will close the audio sender)
        self.audio_capture.stop();
        self.audio_tx = None;

        // Reset state
        self.current_text.clear();
        self.state = AppState::Idle;

        info!("Recording stopped.");
        Ok(())
    }

    /// Check if ASR has detected any speech since the last check
    fn check_and_reset_asr_result(&self) -> bool {
        let result = self.last_asr_result.load(Ordering::SeqCst);
        if result {
            self.last_asr_result.store(false, Ordering::SeqCst);
        }
        result
    }
}

impl Clone for TextInputHandler {
    fn clone(&self) -> Self {
        TextInputHandler::new()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    // Load .env file if present
    let _ = dotenvy::dotenv();

    // Get API key
    let api_key = std::env::var("DASHSCOPE_API_KEY")
        .context("DASHSCOPE_API_KEY environment variable not set")?;

    info!("Audio2Text v{} - Real-time speech recognition", env!("CARGO_PKG_VERSION"));
    info!("Will auto-stop after {} seconds of silence (no speech detected).", MAX_SILENCE_SECONDS);
    info!("Press Ctrl+C to stop manually.");

    // Check for required tools
    check_dependencies();

    let config = Config::from_env()?;
    info!(
        "Language hints: {:?}, VAD model: {}, speech/noise threshold: {:?}, \
         max sentence silence: {:?}, semantic punctuation: {:?}",
        config.language_hints,
        config.vad_model,
        config.speech_noise_threshold,
        config.max_sentence_silence,
        config.semantic_punctuation
    );

    // Create app
    let app = Arc::new(tokio::sync::Mutex::new(App::new(api_key, config)));

    // Handle shutdown signal
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    // Spawn a separate task for Ctrl+C handling
    tokio::spawn(async move {
        signal::ctrl_c().await.ok();
        info!("\nShutting down...");
        running_clone.store(false, Ordering::SeqCst);
    });

    // Start recording immediately
    {
        let mut app = app.lock().await;
        app.start_recording().await?;
    }

    // Main event loop - monitors for silence timeout
    let mut last_speech_time = Instant::now();
    let mut check_interval = tokio::time::interval(Duration::from_millis(100));

    while running.load(Ordering::SeqCst) {
        check_interval.tick().await;

        // Check if ASR has detected any speech
        let has_speech = {
            let app = app.lock().await;
            app.check_and_reset_asr_result()
        };

        if has_speech {
            last_speech_time = Instant::now();
        }

        // Check if we've exceeded the silence timeout
        let silence_duration = last_speech_time.elapsed().as_secs();
        if silence_duration >= MAX_SILENCE_SECONDS {
            info!("No speech detected for {} seconds. Auto-stopping...", MAX_SILENCE_SECONDS);
            running.store(false, Ordering::SeqCst);
            break;
        }

        // Optional: Log silence progress every 10 seconds
        if silence_duration > 0 && silence_duration % 10 == 0 && silence_duration < MAX_SILENCE_SECONDS {
            let prev_check = last_speech_time.elapsed().as_secs();
            if prev_check == silence_duration {
                info!("Silence duration: {} seconds / {} maximum", silence_duration, MAX_SILENCE_SECONDS);
            }
        }
    }

    // Stop recording if active
    let mut app = app.lock().await;
    if app.state == AppState::Recording {
        let _ = app.stop_recording().await;
    }

    info!("Goodbye!");
    Ok(())
}

fn check_dependencies() {
    let tools = [
        ("wtype", "For typing text in Wayland"),
        ("ydotool", "Alternative for typing text"),
        ("wl-copy", "Fallback: copy to clipboard"),
    ];

    let mut found_input_tool = false;

    for (tool, description) in tools {
        if std::process::Command::new("which")
            .arg(tool)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            info!("✓ {} found - {}", tool, description);
            if tool == "wtype" || tool == "ydotool" {
                found_input_tool = true;
            }
        } else {
            warn!("✗ {} not found - {}", tool, description);
        }
    }

    if !found_input_tool {
        warn!("No text input tool found! Install wtype or ydotool for best experience.");
        warn!("Install with: sudo pacman -S wtype  (Arch Linux)");
    }
}
