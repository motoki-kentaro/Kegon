//! Safe wrapper for system clipboard interaction.

use arboard::Clipboard;

pub struct SystemClipboard {
    clipboard: Option<Clipboard>,
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemClipboard {
    pub fn new() -> Self {
        let clipboard = Clipboard::new().ok();
        Self { clipboard }
    }

    /// Reads text from the system clipboard. Returns `None` on failure or if clipboard is empty.
    pub fn get_text(&mut self) -> Option<String> {
        let cb = self.clipboard.as_mut()?;
        match cb.get_text() {
            Ok(text) if !text.is_empty() => Some(text),
            _ => None,
        }
    }

    /// Sets text into the system clipboard. Fails gracefully if clipboard access fails.
    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        if let Some(cb) = self.clipboard.as_mut() {
            let _ = cb.set_text(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_instantiates_without_panic() {
        let mut cb = SystemClipboard::new();
        // Accessing clipboard should not panic regardless of environment.
        let _ = cb.get_text();
    }
}
