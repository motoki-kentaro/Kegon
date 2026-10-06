//! The application icon shown on the window (title bar, taskbar, Alt+Tab).
//!
//! On Windows the executable also carries the icon as a resource (see
//! `build.rs`), but that only affects Explorer: windows get no icon unless one
//! is set explicitly. Both come from the same generated `.ico`.

use iced::window;

const ICO: &[u8] = include_bytes!("../assets/icons/kegon/kegon.ico");

/// Size of the `.ico` entry used for the window. The platform scales it for
/// the title bar and taskbar.
const WINDOW_ICON_SIZE: u32 = 48;

/// Returns the window icon, or `None` if it cannot be decoded. A missing icon
/// is cosmetic, so it never prevents startup.
pub fn window_icon() -> Option<window::Icon> {
    let (rgba, size) = decode(WINDOW_ICON_SIZE)?;
    window::icon::from_rgba(rgba, size, size).ok()
}

/// Decodes the square entry of the given size into straight RGBA.
fn decode(size: u32) -> Option<(Vec<u8>, u32)> {
    let icon_dir = ico::IconDir::read(std::io::Cursor::new(ICO)).ok()?;
    let entry = icon_dir
        .entries()
        .iter()
        .find(|entry| entry.width() == size && entry.height() == size)?;
    let image = entry.decode().ok()?;

    Some((image.rgba_data().to_vec(), size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ico_contains_every_required_size() {
        for size in [16, 24, 32, 48, 64, 128, 256] {
            let (rgba, decoded) = decode(size).unwrap_or_else(|| panic!("missing {size}px"));
            assert_eq!(decoded, size);
            assert_eq!(rgba.len(), (size * size * 4) as usize);
        }
    }

    #[test]
    fn window_icon_decodes() {
        assert!(window_icon().is_some());
    }
}
