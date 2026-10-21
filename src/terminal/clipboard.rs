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

    /// Returns the backend, retrying initialization if an earlier attempt failed
    /// (e.g. the display server was not ready at startup).
    fn backend(&mut self) -> Option<&mut Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    /// Reads text from the system clipboard. Returns `None` on failure or if clipboard is empty.
    pub fn get_text(&mut self) -> Option<String> {
        let cb = self.backend()?;
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
        if let Some(cb) = self.backend() {
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

    #[test]
    fn missing_backend_does_not_panic_and_retries() {
        let mut cb = SystemClipboard { clipboard: None };
        cb.set_text("");
        let _ = cb.get_text();
        cb.set_text("retry");
    }
}
