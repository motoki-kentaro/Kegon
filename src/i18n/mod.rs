//! Localization of Kegon's own UI text.
//!
//! This module does not depend on the GUI toolkit. The UI asks a
//! [`Localizer`] for the text of a [`MessageKey`]; the localizer looks it up in
//! the Fluent resources under `assets/i18n/`, which are embedded in the
//! binary. See `docs/architecture/i18n.md`.

mod key;
mod locale;

use fluent_bundle::{FluentBundle, FluentResource};

pub use fluent_bundle::FluentArgs;
pub use key::MessageKey;
pub use locale::{Locale, os_preferences, resolve};

type Bundle = FluentBundle<FluentResource>;

/// The Fluent source for a locale.
fn resource(locale: Locale) -> &'static str {
    match locale {
        Locale::EnUs => include_str!("../../assets/i18n/en-US.ftl"),
        Locale::JaJp => include_str!("../../assets/i18n/ja-JP.ftl"),
    }
}

/// Formats UI messages for one locale, falling back to en-US.
///
/// Resources are parsed once, when the localizer is created.
pub struct Localizer {
    locale: Locale,
    /// The requested locale's bundle, or `None` when it is the fallback.
    primary: Option<Bundle>,
    fallback: Bundle,
}

impl Localizer {
    pub fn new(locale: Locale) -> Self {
        let primary = (locale != Locale::FALLBACK).then(|| resource(locale));
        Self::from_sources(locale, primary, resource(Locale::FALLBACK))
    }

    fn from_sources(locale: Locale, primary: Option<&str>, fallback: &str) -> Self {
        Self {
            locale,
            primary: primary.map(|source| bundle(locale, source)),
            fallback: bundle(Locale::FALLBACK, fallback),
        }
    }

    #[cfg(test)]
    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// The text of a message.
    pub fn text(&self, key: MessageKey) -> String {
        self.format(key.id(), None)
    }

    /// The text of a message that takes arguments, such as `{ $count }`.
    #[allow(dead_code)] // No UI message takes arguments yet.
    pub fn text_with(&self, key: MessageKey, args: &FluentArgs) -> String {
        self.format(key.id(), Some(args))
    }

    /// Formats a message from the requested locale, then from en-US.
    ///
    /// If neither has it, the message ID itself is returned, so the UI shows
    /// something identifiable rather than blank text or a crash. The tests
    /// guarantee this does not happen for any [`MessageKey`].
    fn format(&self, id: &str, args: Option<&FluentArgs>) -> String {
        for bundle in self.primary.iter().chain([&self.fallback]) {
            if let Some(text) = format_in(bundle, id, args) {
                return text;
            }

            #[cfg(debug_assertions)]
            eprintln!("i18n: message {id:?} is missing from {}", bundle.locales[0]);
        }

        id.to_owned()
    }
}

impl std::fmt::Debug for Localizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Localizer")
            .field("locale", &self.locale)
            .finish_non_exhaustive()
    }
}

fn bundle(locale: Locale, source: &str) -> Bundle {
    let resource =
        FluentResource::try_new(source.to_owned()).unwrap_or_else(|(partial, errors)| {
            // Keep the messages that did parse; one bad entry should not take
            // the whole locale down. The tests require the resources to parse.
            debug_assert!(
                false,
                "{} resource has syntax errors: {errors:?}",
                locale.tag()
            );
            partial
        });

    let mut bundle = FluentBundle::new(vec![locale.language_identifier()]);

    // Fluent wraps arguments in Unicode bidi isolation marks by default. They
    // only matter for right-to-left text and can render as boxes.
    bundle.set_use_isolating(false);

    if let Err(errors) = bundle.add_resource(resource) {
        debug_assert!(
            false,
            "{} resource has duplicate IDs: {errors:?}",
            locale.tag()
        );
    }

    bundle
}

fn format_in(bundle: &Bundle, id: &str, args: Option<&FluentArgs>) -> Option<String> {
    let pattern = bundle.get_message(id)?.value()?;
    let mut errors = Vec::new();
    let text = bundle.format_pattern(pattern, args, &mut errors);

    #[cfg(debug_assertions)]
    if !errors.is_empty() {
        eprintln!("i18n: errors formatting {id:?}: {errors:?}");
    }

    Some(text.into_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use fluent_bundle::FluentResource;
    use fluent_syntax::ast::Entry;

    use super::*;

    /// Message IDs defined in a resource.
    fn message_ids(source: &str) -> BTreeSet<String> {
        let resource = FluentResource::try_new(source.to_owned())
            .unwrap_or_else(|(_, errors)| panic!("syntax errors: {errors:?}"));

        resource
            .entries()
            .filter_map(|entry| match entry {
                Entry::Message(message) => Some(message.id.name.to_owned()),
                _ => None,
            })
            .collect()
    }

    fn key_ids() -> BTreeSet<String> {
        MessageKey::ALL
            .iter()
            .map(|key| key.id().to_owned())
            .collect()
    }

    #[test]
    fn every_locale_defines_exactly_the_message_keys() {
        for locale in Locale::ALL {
            let ids = message_ids(resource(locale));
            let keys = key_ids();

            let missing: Vec<_> = keys.difference(&ids).collect();
            let unused: Vec<_> = ids.difference(&keys).collect();
            assert!(
                missing.is_empty(),
                "{} is missing {missing:?}",
                locale.tag()
            );
            assert!(unused.is_empty(), "{} has unused {unused:?}", locale.tag());
        }
    }

    #[test]
    fn every_message_has_text_in_every_locale() {
        for locale in Locale::ALL {
            let localizer = Localizer::new(locale);
            for &key in MessageKey::ALL {
                let text = localizer.text(key);
                assert!(!text.trim().is_empty(), "{key:?} is blank");
                assert_ne!(text, key.id(), "{key:?} fell through to its ID");
            }
        }
    }

    #[test]
    fn english_lookup() {
        let localizer = Localizer::new(Locale::EnUs);
        assert_eq!(localizer.locale(), Locale::EnUs);
        assert_eq!(localizer.text(MessageKey::SideBarExplorerTitle), "EXPLORER");
        assert_eq!(
            localizer.text(MessageKey::TerminalTabDefaultTitle),
            "Terminal"
        );
    }

    #[test]
    fn japanese_lookup() {
        let localizer = Localizer::new(Locale::JaJp);
        assert_eq!(localizer.locale(), Locale::JaJp);
        assert_eq!(localizer.text(MessageKey::SideBarGitTitle), "ソース管理");
        assert_eq!(
            localizer.text(MessageKey::TerminalTabDefaultTitle),
            "ターミナル"
        );
    }

    #[test]
    fn unsupported_locale_resolves_to_english_text() {
        let localizer = Localizer::new(resolve(Some("fr-FR"), ["ja-JP"]));
        assert_eq!(localizer.locale(), Locale::EnUs);
        assert_eq!(localizer.text(MessageKey::SideBarSearchTitle), "SEARCH");
    }

    // The tests below use small fixture resources, not Kegon's UI strings.

    #[test]
    fn message_missing_from_locale_falls_back_to_english() {
        let localizer = Localizer::from_sources(
            Locale::JaJp,
            Some("translated = 翻訳済み\n"),
            "translated = Translated\nuntranslated = Untranslated\n",
        );

        assert_eq!(localizer.format("translated", None), "翻訳済み");
        assert_eq!(localizer.format("untranslated", None), "Untranslated");
    }

    #[test]
    fn message_missing_everywhere_shows_its_id() {
        let localizer = Localizer::from_sources(Locale::JaJp, Some(""), "");
        assert_eq!(localizer.format("no-such-message", None), "no-such-message");
    }

    #[test]
    fn arguments_are_substituted() {
        let localizer = Localizer::from_sources(
            Locale::EnUs,
            None,
            "not-found = \"{ $name }\" was not found\n",
        );

        let mut args = FluentArgs::new();
        args.set("name", "Cargo.toml");
        assert_eq!(
            localizer.format("not-found", Some(&args)),
            "\"Cargo.toml\" was not found"
        );
    }

    #[test]
    fn plurals_follow_the_locale_rules() {
        let english = Localizer::from_sources(
            Locale::EnUs,
            None,
            "files = { $count ->\n    [one] { $count } file\n   *[other] { $count } files\n}\n",
        );
        let japanese = Localizer::from_sources(
            Locale::JaJp,
            Some("files = { $count ->\n    [one] 単数\n   *[other] { $count } 個のファイル\n}\n"),
            "",
        );

        let count = |n: i64| {
            let mut args = FluentArgs::new();
            args.set("count", n);
            args
        };

        assert_eq!(english.format("files", Some(&count(1))), "1 file");
        assert_eq!(english.format("files", Some(&count(2))), "2 files");
        assert_eq!(english.format("files", Some(&count(0))), "0 files");
        // Japanese has no grammatical plural: CLDR maps every count to "other".
        assert_eq!(japanese.format("files", Some(&count(1))), "1 個のファイル");
        assert_eq!(japanese.format("files", Some(&count(2))), "2 個のファイル");
    }
}
