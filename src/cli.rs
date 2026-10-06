//! Command-line options.
//!
//! Kegon has very few options, so they are parsed by hand. Problems are
//! reported as warnings and never stop the application from starting.

/// Options given on the command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The raw value of `--locale`, if given. An empty string means the
    /// option was given without a value.
    pub locale: Option<String>,
}

/// Parses arguments, excluding the program name.
///
/// Accepts `--locale <tag>` and `--locale=<tag>`. If the option is repeated,
/// the last one wins. Unknown arguments are ignored. Returns the options and
/// any warnings to show.
pub fn parse<I>(args: I) -> (Options, Vec<String>)
where
    I: IntoIterator<Item = String>,
{
    let mut options = Options::default();
    let mut warnings = Vec::new();
    let mut args = args.into_iter().peekable();

    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--locale=") {
            options.locale = Some(value.to_owned());
        } else if arg == "--locale" {
            let value = args.next_if(|next| !next.starts_with('-'));
            if value.is_none() {
                warnings.push(String::from(
                    "--locale requires a value, such as --locale ja-JP",
                ));
            }
            options.locale = Some(value.unwrap_or_default());
        } else {
            warnings.push(format!("ignoring unknown argument {arg:?}"));
        }
    }

    (options, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> (Options, Vec<String>) {
        parse(args.iter().map(|arg| arg.to_string()))
    }

    fn locale(args: &[&str]) -> Option<String> {
        parse_strs(args).0.locale
    }

    #[test]
    fn no_arguments() {
        assert_eq!(parse_strs(&[]), (Options::default(), vec![]));
    }

    #[test]
    fn locale_as_separate_or_joined_value() {
        assert_eq!(locale(&["--locale", "ja-JP"]).as_deref(), Some("ja-JP"));
        assert_eq!(locale(&["--locale=ja-JP"]).as_deref(), Some("ja-JP"));
    }

    #[test]
    fn last_locale_wins() {
        assert_eq!(
            locale(&["--locale", "ja-JP", "--locale=en-US"]).as_deref(),
            Some("en-US")
        );
    }

    #[test]
    fn locale_values_are_passed_through_unvalidated() {
        // Validation and fallback happen in locale resolution.
        assert_eq!(locale(&["--locale", "fr-FR"]).as_deref(), Some("fr-FR"));
        assert_eq!(locale(&["--locale=???"]).as_deref(), Some("???"));
    }

    #[test]
    fn missing_locale_value_warns_and_requests_an_empty_locale() {
        for args in [&["--locale"][..], &["--locale", "--other"], &["--locale="]] {
            let (options, _) = parse_strs(args);
            assert_eq!(options.locale.as_deref(), Some(""), "{args:?}");
        }
        assert_eq!(parse_strs(&["--locale"]).1.len(), 1);
    }

    #[test]
    fn unknown_arguments_warn_and_are_ignored() {
        let (options, warnings) = parse_strs(&["--verbose", "--locale", "ja-JP", "file.txt"]);
        assert_eq!(options.locale.as_deref(), Some("ja-JP"));
        assert_eq!(warnings.len(), 2);
    }
}
