// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_icon;
mod cli;
mod command;
mod dialog;
mod font;
mod i18n;
mod icons;
mod settings;
mod terminal;
mod theme;
mod workbench;

use app::Kegon;

fn main() -> iced::Result {
    let (options, warnings) = cli::parse(std::env::args().skip(1));
    for warning in warnings {
        eprintln!("kegon: {warning}");
    }

    let (settings, settings_warning) = settings::load_settings(None);
    if let Some(warning) = settings_warning {
        eprintln!("kegon: {warning}");
    }
    if let Some(warning) = settings.appearance.theme.diagnostic() {
        eprintln!("kegon: {warning}");
    }
    for warning in settings.appearance.metric_diagnostics() {
        eprintln!("kegon: {warning}");
    }
    if let Some(warning) = settings.dopamine.diagnostic() {
        eprintln!("kegon: {warning}");
    }

    let locale = settings::resolve_application_locale(
        options.locale.as_deref(),
        settings.locale,
        i18n::os_preferences(),
    );

    let cli_locale_override = options.locale.clone();

    let result = iced::application(
        move || {
            (
                Kegon::new(
                    locale,
                    settings.clone(),
                    cli_locale_override.clone(),
                    options.smoke_confirmation_dialog,
                    options.smoke_font_picker,
                ),
                Kegon::initial_task(),
            )
        },
        Kegon::run_update,
        Kegon::view,
    )
    .title(Kegon::title)
    .subscription(Kegon::subscription)
    .theme(Kegon::iced_theme)
    .window(iced::window::Settings {
        size: iced::Size::new(1200.0, 760.0),
        min_size: Some(iced::Size::new(640.0, 400.0)),
        icon: app_icon::window_icon(),
        ..iced::window::Settings::default()
    })
    .run();

    // Closing a tab only asks its PTY thread to stop. Without this the process can exit before
    // the thread closed the pseudoconsole and before a surviving shell was terminated.
    let cleanup = terminal::wait_for_shell_cleanup(terminal::SHUTDOWN_WAIT);
    if let Some(warning) = cleanup.exit_warning(terminal::SHUTDOWN_WAIT) {
        eprintln!("kegon: {warning}");
    }
    result
}
