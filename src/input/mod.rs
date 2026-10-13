use anyhow::{Context, Result};
use std::process::Command;
use tracing::{debug, error, info, warn};

/// Text input handler for Wayland
/// Uses wtype (preferred) or ydotool for text input simulation
#[derive(Clone)]
pub struct TextInputHandler {
    method: InputMethod,
}

#[derive(Debug, Clone, Copy)]
enum InputMethod {
    Wtype,
    Ydotool,
    WlCopy, // Fallback: copy to clipboard
}

impl TextInputHandler {
    pub fn new() -> Self {
        // Detect available input method
        let method = if is_command_available("wtype") {
            info!("Using wtype for text input");
            InputMethod::Wtype
        } else if is_command_available("ydotool") {
            info!("Using ydotool for text input");
            InputMethod::Ydotool
        } else if is_command_available("wl-copy") {
            warn!("wtype and ydotool not found, falling back to wl-copy (clipboard)");
            InputMethod::WlCopy
        } else {
            warn!("No suitable text input method found! Install wtype, ydotool, or wl-copy");
            InputMethod::WlCopy
        };

        Self { method }
    }

    /// Type the given text as keyboard input
    pub fn type_text(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        debug!("Typing text: {}", text);

        match self.method {
            InputMethod::Wtype => self.type_with_wtype(text),
            InputMethod::Ydotool => self.type_with_ydotool(text),
            InputMethod::WlCopy => self.copy_to_clipboard(text),
        }
    }

    fn type_with_wtype(&self, text: &str) -> Result<()> {
        // wtype directly types the text
        let output = Command::new("wtype")
            .arg("--")
            .arg(text)
            .output()
            .context("Failed to execute wtype")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("wtype failed: {}", stderr);
            return Err(anyhow::anyhow!("wtype failed: {}", stderr));
        }

        Ok(())
    }

    fn type_with_ydotool(&self, text: &str) -> Result<()> {
        // ydotool needs ydotoold daemon running
        let output = Command::new("ydotool")
            .arg("type")
            .arg("--")
            .arg(text)
            .output()
            .context("Failed to execute ydotool")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("ydotool failed: {}", stderr);
            return Err(anyhow::anyhow!("ydotool failed: {}", stderr));
        }

        Ok(())
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
