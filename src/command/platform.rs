//! Operating system platform abstraction.

/// Operating system platform targeting default keymap configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Windows,
    Linux,
    MacOS,
}

impl Platform {
    /// Detect the target host platform at runtime.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOS
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_current_returns_a_variant() {
        let platform = Platform::current();
        assert!(matches!(
            platform,
            Platform::Windows | Platform::Linux | Platform::MacOS
        ));
    }
}
