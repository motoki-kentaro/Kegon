// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_icon;
mod icons;
mod workbench;

use app::Kegon;

fn main() -> iced::Result {
    iced::application(Kegon::default, Kegon::update, Kegon::view)
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
