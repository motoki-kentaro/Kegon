// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_icon;
mod cli;
mod command;
mod dialog;
mod i18n;
mod icons;
mod settings;
mod terminal;
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

    let locale = settings::resolve_application_locale(
        options.locale.as_deref(),
        settings.locale,
        i18n::os_preferences(),
    );

    let cli_locale_override = options.locale.clone();

    iced::application(
        move || {
            Kegon::new(
                locale,
                settings.clone(),
                cli_locale_override.clone(),
                options.smoke_confirmation_dialog,
            )
        },
        Kegon::update,
        Kegon::view,
    )
    .title(Kegon::title)
    .subscription(Kegon::subscription)
    .theme(|_: &Kegon| iced::Theme::Dark)
    .window(iced::window::Settings {
        size: iced::Size::new(1200.0, 760.0),
        min_size: Some(iced::Size::new(640.0, 400.0)),
        icon: app_icon::window_icon(),
        ..iced::window::Settings::default()
    })
    .run()
}
