use anyhow::{Context, Result};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

mod virtual_keyboard;
use virtual_keyboard::VirtualKeyboard;

/// Text input handler for Wayland
/// Uses a native virtual keyboard with printable evdev keycodes.
#[derive(Clone)]
pub struct TextInputHandler {
    method: InputMethod,
}

#[derive(Clone)]
enum InputMethod {
    Wayland(Arc<Mutex<VirtualKeyboard>>),
    WlCopy, // Fallback: copy to clipboard
}

impl TextInputHandler {
    pub fn new() -> Self {
        let method = match VirtualKeyboard::connect() {
            Ok(keyboard) => {
                info!("Using native Wayland virtual keyboard with printable keycodes");
                InputMethod::Wayland(Arc::new(Mutex::new(keyboard)))
            }
            Err(error) => {
                warn!("Native Wayland input unavailable: {error:#}; falling back to manual clipboard paste");
                if !is_command_available("wl-copy") {
                    warn!("Install wl-clipboard to enable the clipboard fallback");
                }
                InputMethod::WlCopy
            }
        };

        Self { method }
    }

    /// Type the given text as keyboard input
    pub fn type_text(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        debug!("Typing text: {}", text);

        match &self.method {
            InputMethod::Wayland(keyboard) => keyboard
                .lock()
                .map_err(|_| anyhow::anyhow!("Virtual keyboard lock poisoned"))?
                .type_text(text),
            InputMethod::WlCopy => self.copy_to_clipboard(text),
        }
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        // Fallback: copy to clipboard and let user paste
        let mut child = Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("Failed to execute wl-copy")?;

        use std::io::Write;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(text.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("wl-copy failed: {}", stderr);
        }

        info!("Text copied to clipboard. Press Ctrl+V to paste.");
        Ok(())
    }
}

fn is_command_available(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

impl Default for TextInputHandler {
    fn default() -> Self {
        Self::new()
    }
}
