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

/// arboard 3.6.1 corrupts the heap on Windows (STATUS_HEAP_CORRUPTION) when two threads use the
/// OS clipboard at once, so every test that touches it takes this lock first.
#[cfg(test)]
static OS_CLIPBOARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Hold the guard in the first local of the test: it is dropped last, after the clipboard.
///
/// A poisoned lock only means an earlier clipboard test already failed, and that failure is
/// reported on its own, so the next test still runs instead of failing with a `PoisonError`.
#[cfg(test)]
pub(crate) fn lock_os_clipboard() -> std::sync::MutexGuard<'static, ()> {
    OS_CLIPBOARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_instantiates_without_panic() {
        let _os_clipboard = lock_os_clipboard();
        let mut cb = SystemClipboard::new();
        // Accessing clipboard should not panic regardless of environment.
        let _ = cb.get_text();
    }

    #[test]
    fn missing_backend_does_not_panic_and_retries() {
        let _os_clipboard = lock_os_clipboard();
        let mut cb = SystemClipboard { clipboard: None };
        cb.set_text("");
        let _ = cb.get_text();
        cb.set_text("retry");
    }
}
