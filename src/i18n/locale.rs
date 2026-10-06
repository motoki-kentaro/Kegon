//! Supported locales and how one is chosen at startup.

use unic_langid::LanguageIdentifier;

/// A locale Kegon ships UI strings for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    EnUs,
    JaJp,
}

impl Locale {
    #[cfg(test)]
    pub const ALL: [Self; 2] = [Self::EnUs, Self::JaJp];

    /// The canonical locale. Every message exists in it, and every other
    /// locale falls back to it.
    pub const FALLBACK: Self = Self::EnUs;

    /// The BCP 47 tag, as used for resource file names and `--locale`.
    pub fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::JaJp => "ja-JP",
        }
    }

    /// Maps a locale tag to a supported locale by its language.
    ///
    /// Accepts BCP 47 tags (`ja-JP`) as well as the forms operating systems
    /// report (`ja_JP.UTF-8`, `ja`), case-insensitively. Regional variants of
    /// a supported language resolve to it (`en-GB` → `en-US`). Returns `None`
    /// for unsupported languages and malformed tags.
    pub fn from_tag(tag: &str) -> Option<Self> {
        match parse_tag(tag)?.language.as_str() {
            "en" => Some(Self::EnUs),
            "ja" => Some(Self::JaJp),
            _ => None,
        }
    }

    pub(super) fn language_identifier(self) -> LanguageIdentifier {
        parse_tag(self.tag()).expect("supported locale tags are valid")
    }
}

/// Normalizes and parses a locale tag.
fn parse_tag(tag: &str) -> Option<LanguageIdentifier> {
    // POSIX locales carry an encoding and modifier: `ja_JP.UTF-8@modifier`.
    let tag = tag.trim();
    let tag = tag.split(['.', '@']).next().unwrap_or_default();
    let tag = tag.replace('_', "-");

    if tag.is_empty() {
        return None;
    }

    tag.parse().ok()
}

/// Chooses the UI locale.
///
/// 1. An explicit request (such as `--locale`) wins. If it is unsupported
///    or malformed, the result is [`Locale::FALLBACK`], regardless of the OS.
/// 2. Otherwise the first supported locale among the OS's preferred UI
///    languages, in order.
/// 3. Otherwise [`Locale::FALLBACK`].
pub fn resolve<I>(explicit: Option<&str>, os_preferences: I) -> Locale
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    if let Some(tag) = explicit {
        return Locale::from_tag(tag).unwrap_or(Locale::FALLBACK);
    }

    os_preferences
        .into_iter()
        .find_map(|tag| Locale::from_tag(tag.as_ref()))
        .unwrap_or(Locale::FALLBACK)
}

/// The OS's preferred UI languages, most preferred first. Empty if they
/// cannot be determined.
pub fn os_preferences() -> impl Iterator<Item = String> {
    sys_locale::get_locales()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_OS: [&str; 0] = [];

    #[test]
    fn tags_round_trip() {
        for locale in Locale::ALL {
            assert_eq!(Locale::from_tag(locale.tag()), Some(locale));
        }
    }

    #[test]
    fn tags_are_normalized() {
        for tag in [
            "ja-JP",
            "ja-jp",
            "JA-JP",
            "ja_JP",
            "ja_JP.UTF-8",
            "ja",
            " ja-JP ",
        ] {
            assert_eq!(Locale::from_tag(tag), Some(Locale::JaJp), "{tag:?}");
        }
        for tag in ["en-US", "en_US.UTF-8", "en", "en-GB", "en-Latn-US"] {
            assert_eq!(Locale::from_tag(tag), Some(Locale::EnUs), "{tag:?}");
        }
    }

    #[test]
    fn unsupported_and_malformed_tags_are_rejected() {
        for tag in [
            "fr-FR",
            "zh-Hans-CN",
            "",
            "   ",
            "C",
            "not a locale",
            "-",
            "ja--JP",
        ] {
            assert_eq!(Locale::from_tag(tag), None, "{tag:?}");
        }
    }

    #[test]
    fn explicit_locale_wins_over_os() {
        assert_eq!(resolve(Some("ja-JP"), ["en-US"]), Locale::JaJp);
        assert_eq!(resolve(Some("en-US"), ["ja-JP"]), Locale::EnUs);
    }

    #[test]
    fn unsupported_explicit_locale_falls_back_to_english_not_os() {
        assert_eq!(resolve(Some("fr-FR"), ["ja-JP"]), Locale::EnUs);
        assert_eq!(resolve(Some(""), ["ja-JP"]), Locale::EnUs);
        assert_eq!(resolve(Some("???"), ["ja-JP"]), Locale::EnUs);
    }

    #[test]
    fn os_locale_is_used_without_explicit_request() {
        assert_eq!(resolve(None, ["ja-JP"]), Locale::JaJp);
        assert_eq!(resolve(None, ["en-US"]), Locale::EnUs);
    }

    #[test]
    fn first_supported_os_preference_is_used() {
        assert_eq!(resolve(None, ["fr-FR", "ja-JP", "en-US"]), Locale::JaJp);
    }

    #[test]
    fn unsupported_or_unknown_os_locale_falls_back_to_english() {
        assert_eq!(resolve(None, ["fr-FR", "de-DE"]), Locale::EnUs);
        assert_eq!(resolve(None, NO_OS), Locale::EnUs);
    }
}
