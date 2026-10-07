//! Command-line options.
//!
//! Kegon has very few options, so they are parsed by hand. Problems are
//! reported as warnings and never stop the application from starting.

/// Smoke test dialog kind requested on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmokeConfirmationDialog {
    Question,
    Warning,
}

/// Smoke test font picker mode requested on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmokeFontPicker {
    Ui,
    Terminal,
}

/// Options given on the command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The raw value of `--locale`, if given. An empty string means the
    /// option was given without a value.
    pub locale: Option<String>,
    /// Smoke test confirmation dialog option, if requested.
    pub smoke_confirmation_dialog: Option<SmokeConfirmationDialog>,
    /// Smoke test font picker option, if requested.
    pub smoke_font_picker: Option<SmokeFontPicker>,
}

/// Parses arguments, excluding the program name.
///
/// Accepts `--locale <tag>` and `--locale=<tag>`, as well as
/// `--smoke-confirmation-dialog <question|warning>`,
/// `--smoke-confirmation-dialog=<question|warning>`,
/// `--smoke-font-picker <ui|terminal>`, and
/// `--smoke-font-picker=<ui|terminal>`. If an option is repeated,
/// the last valid one wins. Unknown arguments are ignored. Returns the options and
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
        } else if let Some(value) = arg.strip_prefix("--smoke-confirmation-dialog=") {
            match value {
                "question" => {
                    options.smoke_confirmation_dialog = Some(SmokeConfirmationDialog::Question)
                }
                "warning" => {
                    options.smoke_confirmation_dialog = Some(SmokeConfirmationDialog::Warning)
                }
                other => {
                    warnings.push(format!(
                        "--smoke-confirmation-dialog expected 'question' or 'warning', got {other:?}"
                    ));
                    options.smoke_confirmation_dialog = None;
                }
            }
        } else if arg == "--smoke-confirmation-dialog" {
            let value = args.next_if(|next| !next.starts_with('-'));
            match value.as_deref() {
                Some("question") => {
                    options.smoke_confirmation_dialog = Some(SmokeConfirmationDialog::Question)
                }
                Some("warning") => {
                    options.smoke_confirmation_dialog = Some(SmokeConfirmationDialog::Warning)
                }
                Some(other) => {
                    warnings.push(format!(
                        "--smoke-confirmation-dialog expected 'question' or 'warning', got {other:?}"
                    ));
                    options.smoke_confirmation_dialog = None;
                }
                None => {
                    warnings.push(String::from(
                        "--smoke-confirmation-dialog requires a value ('question' or 'warning')",
                    ));
                    options.smoke_confirmation_dialog = None;
                }
            }
        } else if let Some(value) = arg.strip_prefix("--smoke-font-picker=") {
            match value {
                "ui" => options.smoke_font_picker = Some(SmokeFontPicker::Ui),
                "terminal" => options.smoke_font_picker = Some(SmokeFontPicker::Terminal),
                other => {
                    warnings.push(format!(
                        "--smoke-font-picker expected 'ui' or 'terminal', got {other:?}"
                    ));
                    options.smoke_font_picker = None;
                }
            }
        } else if arg == "--smoke-font-picker" {
            let value = args.next_if(|next| !next.starts_with('-'));
            match value.as_deref() {
                Some("ui") => options.smoke_font_picker = Some(SmokeFontPicker::Ui),
                Some("terminal") => options.smoke_font_picker = Some(SmokeFontPicker::Terminal),
                Some(other) => {
                    warnings.push(format!(
                        "--smoke-font-picker expected 'ui' or 'terminal', got {other:?}"
                    ));
                    options.smoke_font_picker = None;
                }
                None => {
                    warnings.push(String::from(
                        "--smoke-font-picker requires a value ('ui' or 'terminal')",
                    ));
                    options.smoke_font_picker = None;
                }
            }
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

    #[test]
    fn smoke_confirmation_dialog_parsing() {
        assert_eq!(
            parse_strs(&["--smoke-confirmation-dialog", "question"])
                .0
                .smoke_confirmation_dialog,
            Some(SmokeConfirmationDialog::Question)
        );
        assert_eq!(
            parse_strs(&["--smoke-confirmation-dialog=warning"])
                .0
                .smoke_confirmation_dialog,
            Some(SmokeConfirmationDialog::Warning)
        );

        let (opts, warns) = parse_strs(&["--smoke-confirmation-dialog", "invalid"]);
        assert_eq!(opts.smoke_confirmation_dialog, None);
        assert_eq!(warns.len(), 1);

        let (opts2, warns2) = parse_strs(&["--smoke-confirmation-dialog"]);
        assert_eq!(opts2.smoke_confirmation_dialog, None);
        assert_eq!(warns2.len(), 1);
    }

    #[test]
    fn smoke_font_picker_parsing() {
        assert_eq!(
            parse_strs(&["--smoke-font-picker", "ui"])
                .0
                .smoke_font_picker,
            Some(SmokeFontPicker::Ui)
        );
        assert_eq!(
            parse_strs(&["--smoke-font-picker=terminal"])
                .0
                .smoke_font_picker,
            Some(SmokeFontPicker::Terminal)
        );

        let (opts, warns) = parse_strs(&["--smoke-font-picker", "invalid"]);
        assert_eq!(opts.smoke_font_picker, None);
        assert_eq!(warns.len(), 1);

        let (opts2, warns2) = parse_strs(&["--smoke-font-picker"]);
        assert_eq!(opts2.smoke_font_picker, None);
        assert_eq!(warns2.len(), 1);
    }
}
