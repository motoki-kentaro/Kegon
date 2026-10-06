//! The iced application: messages, update logic, and the workbench view.

use iced::keyboard::{self, Modifiers, key};
use iced::widget::{Space, button, column, container, mouse_area, row, space, svg, text, tooltip};
use iced::{
    Border, Center, Color, Element, Event, Fill, Font, Subscription, Theme, event, font, mouse,
};

use crate::icons::{ICON_SIZE, activity_icon};
use crate::workbench::{ActivityItem, TabId, Workbench};

const ACTIVITY_BAR_WIDTH: f32 = 48.0;
const SASH_WIDTH: f32 = 4.0;
const TAB_STRIP_HEIGHT: f32 = 35.0;
const UI_TEXT_SIZE: f32 = 13.0;

/// Colors for the PoC. A proper theme system is out of scope for now.
mod palette {
    use iced::Color;

    pub const ACTIVITY_BAR: Color = Color::from_rgb8(0x2c, 0x2c, 0x2c);
    pub const SIDE_BAR: Color = Color::from_rgb8(0x25, 0x25, 0x26);
    pub const TAB_STRIP: Color = Color::from_rgb8(0x25, 0x25, 0x26);
    pub const TAB_ACTIVE: Color = Color::from_rgb8(0x1e, 0x1e, 0x1e);
    pub const EDITOR: Color = Color::from_rgb8(0x1e, 0x1e, 0x1e);
    pub const BORDER: Color = Color::from_rgb8(0x3c, 0x3c, 0x3c);
    pub const HOVER: Color = Color::from_rgb8(0x37, 0x37, 0x3d);
    pub const ACCENT: Color = Color::from_rgb8(0x00, 0x7a, 0xcc);
    pub const TEXT: Color = Color::from_rgb8(0xcc, 0xcc, 0xcc);
    pub const TEXT_MUTED: Color = Color::from_rgb8(0x85, 0x85, 0x85);
    pub const ICON_ACTIVE: Color = Color::from_rgb8(0xff, 0xff, 0xff);
    pub const ICON_HOVERED: Color = Color::from_rgb8(0xcc, 0xcc, 0xcc);
    pub const ICON_INACTIVE: Color = Color::from_rgb8(0x85, 0x85, 0x85);
}

#[derive(Debug, Clone)]
pub enum Message {
    ActivitySelected(ActivityItem),
    ActivityHovered(ActivityItem),
    ActivityUnhovered(ActivityItem),
    TerminalTabSelected(TabId),
    SashPressed,
    SashDragged(f32),
    SashReleased,
}

#[derive(Debug, Default)]
pub struct Kegon {
    workbench: Workbench,
    /// Whether the Side Bar sash is being dragged. This is transient
    /// interaction state, so it lives here rather than in [`Workbench`].
    resizing_side_bar: bool,
    /// The Activity Bar entry under the pointer, if any. Tracked here
    /// because the hover color applies to the whole entry, not just the
    /// icon's own bounds.
    hovered_activity: Option<ActivityItem>,
}

impl Kegon {
    pub fn title(&self) -> String {
        String::from("Kegon")
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ActivitySelected(item) => self.workbench.select_activity(item),
            Message::ActivityHovered(item) => self.hovered_activity = Some(item),
            Message::ActivityUnhovered(item) => {
                // Ignore a late exit from an entry the pointer already left.
                if self.hovered_activity == Some(item) {
                    self.hovered_activity = None;
                }
            }
            Message::TerminalTabSelected(id) => self.workbench.select_terminal_tab(id),
            Message::SashPressed => self.resizing_side_bar = true,
            Message::SashDragged(cursor_x) => {
                if self.resizing_side_bar {
                    self.workbench
                        .resize_side_bar(cursor_x - ACTIVITY_BAR_WIDTH - SASH_WIDTH / 2.0);
                }
            }
            Message::SashReleased => self.resizing_side_bar = false,
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let shortcuts = keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed {
                physical_key,
                modifiers,
                ..
            } => activity_shortcut(physical_key, modifiers).map(Message::ActivitySelected),
            _ => None,
        });

        if self.resizing_side_bar {
            // Track the cursor across the whole window while dragging, so the
            // drag continues even when the pointer leaves the thin sash.
            let drag = event::listen_with(|event, _status, _window| match event {
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(Message::SashDragged(position.x))
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::SashReleased)
                }
                _ => None,
            });

            Subscription::batch([shortcuts, drag])
        } else {
            shortcuts
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let body = row![
            self.activity_bar(),
            self.side_bar(),
            self.sash(),
            self.main_area(),
        ]
        .height(Fill);

        if self.resizing_side_bar {
            // Keep the resize cursor while the pointer is outside the sash.
            mouse_area(body)
                .interaction(mouse::Interaction::ResizingHorizontally)
                .into()
        } else {
            body.into()
        }
    }

    fn activity_bar(&self) -> Element<'_, Message> {
        let active = self.workbench.active_activity();

        let items = ActivityItem::ALL.into_iter().map(|item| {
            let is_active = item == active;
            let color = activity_icon_color(is_active, self.hovered_activity == Some(item));

            let icon = svg(activity_icon(item))
                .width(ICON_SIZE)
                .height(ICON_SIZE)
                .style(move |_, _| svg::Style { color: Some(color) });

            // A thin bar on the left edge marks the selected item.
            let indicator = container(space())
                .width(2)
                .height(ACTIVITY_BAR_WIDTH)
                .style(move |_| {
                    container::Style::default().background(if is_active {
                        palette::ICON_ACTIVE
                    } else {
                        Color::TRANSPARENT
                    })
                });

            let entry = button(container(icon).center(Fill))
                .width(ACTIVITY_BAR_WIDTH - 2.0)
                .height(ACTIVITY_BAR_WIDTH)
                .padding(0)
                .on_press(Message::ActivitySelected(item))
                .style(|_, _| button::Style::default());

            let entry = mouse_area(row![indicator, entry])
                .on_enter(Message::ActivityHovered(item))
                .on_exit(Message::ActivityUnhovered(item));

            tooltip(entry, tooltip_label(item.label()), tooltip::Position::Right).into()
        });

        container(column(items))
            .width(ACTIVITY_BAR_WIDTH)
            .height(Fill)
            .style(|_| container::Style::default().background(palette::ACTIVITY_BAR))
            .into()
    }

    fn side_bar(&self) -> Element<'_, Message> {
        let item = self.workbench.active_activity();

        let placeholder = match item {
            ActivityItem::Explorer => "File Explorer is not implemented yet.",
            ActivityItem::Search => "Project-wide search is not implemented yet.",
            ActivityItem::Git => "Git integration is not implemented yet.",
        };

        let header = container(
            text(item.side_bar_title())
                .size(11)
                .font(Font {
                    weight: font::Weight::Bold,
                    ..Font::DEFAULT
                })
                .color(palette::TEXT),
        )
        .height(TAB_STRIP_HEIGHT)
        .padding([0, 20])
        .align_y(Center);

        let content = column![
            header,
            container(
                text(placeholder)
                    .size(UI_TEXT_SIZE)
                    .color(palette::TEXT_MUTED)
            )
            .padding([8, 20]),
        ];

        container(content)
            .width(self.workbench.side_bar_width())
            .height(Fill)
            .style(|_| container::Style::default().background(palette::SIDE_BAR))
            .into()
    }

    /// The draggable divider between the Side Bar and the main area.
    fn sash(&self) -> Element<'_, Message> {
        let line = container(Space::new().width(1).height(Fill))
            .style(|_| container::Style::default().background(palette::BORDER));

        let highlight = self.resizing_side_bar;
        let handle = container(line)
            .width(SASH_WIDTH)
            .height(Fill)
            .align_x(Center)
            .style(move |_| {
                container::Style::default().background(if highlight {
                    palette::ACCENT
                } else {
                    palette::SIDE_BAR
                })
            });

        mouse_area(handle)
            .on_press(Message::SashPressed)
            .interaction(mouse::Interaction::ResizingHorizontally)
            .into()
    }

    fn main_area(&self) -> Element<'_, Message> {
        let tab_strip = self.tab_strip();

        let active = self.workbench.terminal_tabs().active();
        let placeholder = column![
            text(format!("{} (placeholder)", active.title))
                .size(UI_TEXT_SIZE + 2.0)
                .color(palette::TEXT),
            text("Terminal sessions are not implemented yet.")
                .size(UI_TEXT_SIZE)
                .color(palette::TEXT_MUTED),
            text("ターミナルはまだ実装されていません。")
                .size(UI_TEXT_SIZE)
                .color(palette::TEXT_MUTED),
        ]
        .spacing(6)
        .align_x(Center);

        let content = container(placeholder)
            .center(Fill)
            .style(|_| container::Style::default().background(palette::EDITOR));

        column![tab_strip, content].width(Fill).into()
    }

    fn tab_strip(&self) -> Element<'_, Message> {
        let tabs = self.workbench.terminal_tabs();

        let tab_buttons = tabs.iter().map(|tab| {
            let is_active = tabs.is_active(tab.id);

            button(
                container(text(&tab.title).size(UI_TEXT_SIZE))
                    .height(Fill)
                    .align_y(Center),
            )
            .height(TAB_STRIP_HEIGHT)
            .padding([0, 16])
            .on_press(Message::TerminalTabSelected(tab.id))
            .style(move |_, status| tab_style(is_active, status))
            .into()
        });

        // Creating sessions is out of scope for now: the button has no
        // `on_press`, which also renders it as disabled.
        let new_tab = tooltip(
            button(container(text("+").size(16)).center(Fill))
                .width(TAB_STRIP_HEIGHT)
                .height(TAB_STRIP_HEIGHT)
                .padding(0)
                .style(|_, _| button::Style {
                    text_color: palette::TEXT_MUTED,
                    ..button::Style::default()
                }),
            tooltip_label("New Terminal (not implemented yet)"),
            tooltip::Position::Bottom,
        );

        let strip = row(tab_buttons).push(new_tab).push(space::horizontal());

        container(strip)
            .width(Fill)
            .height(TAB_STRIP_HEIGHT)
            .style(|_| container::Style::default().background(palette::TAB_STRIP))
            .into()
    }
}

/// Maps VS Code-style shortcuts to Activity Bar entries:
/// Ctrl+Shift+E / F / G (Cmd+Shift on macOS).
///
/// Physical keys are used so that the shortcuts do not depend on the
/// keyboard layout.
fn activity_shortcut(physical_key: key::Physical, modifiers: Modifiers) -> Option<ActivityItem> {
    if !(modifiers.command() && modifiers.shift()) || modifiers.alt() {
        return None;
    }

    match physical_key {
        key::Physical::Code(key::Code::KeyE) => Some(ActivityItem::Explorer),
        key::Physical::Code(key::Code::KeyF) => Some(ActivityItem::Search),
        key::Physical::Code(key::Code::KeyG) => Some(ActivityItem::Git),
        _ => None,
    }
}

/// Icon color for an Activity Bar entry. Selection takes precedence over
/// hover.
fn activity_icon_color(is_active: bool, is_hovered: bool) -> Color {
    if is_active {
        palette::ICON_ACTIVE
    } else if is_hovered {
        palette::ICON_HOVERED
    } else {
        palette::ICON_INACTIVE
    }
}

fn tab_style(is_active: bool, status: button::Status) -> button::Style {
    let background = if is_active {
        palette::TAB_ACTIVE
    } else if matches!(status, button::Status::Hovered) {
        palette::HOVER
    } else {
        palette::TAB_STRIP
    };

    button::Style {
        background: Some(background.into()),
        text_color: if is_active {
            palette::ICON_ACTIVE
        } else {
            palette::TEXT_MUTED
        },
        border: Border {
            color: palette::BORDER,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..button::Style::default()
    }
}

fn tooltip_label(label: &str) -> Element<'_, Message> {
    container(text(label).size(UI_TEXT_SIZE).color(palette::TEXT))
        .padding([4, 8])
        .style(|_: &Theme| {
            container::Style::default()
                .background(palette::ACTIVITY_BAR)
                .border(Border {
                    color: palette::BORDER,
                    width: 1.0,
                    radius: 3.0.into(),
                })
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl_shift() -> Modifiers {
        Modifiers::COMMAND | Modifiers::SHIFT
    }

    fn code(code: key::Code) -> key::Physical {
        key::Physical::Code(code)
    }

    #[test]
    fn shortcuts_select_activity_items() {
        assert_eq!(
            activity_shortcut(code(key::Code::KeyE), ctrl_shift()),
            Some(ActivityItem::Explorer)
        );
        assert_eq!(
            activity_shortcut(code(key::Code::KeyF), ctrl_shift()),
            Some(ActivityItem::Search)
        );
        assert_eq!(
            activity_shortcut(code(key::Code::KeyG), ctrl_shift()),
            Some(ActivityItem::Git)
        );
    }

    #[test]
    fn shortcuts_require_command_and_shift() {
        assert_eq!(
            activity_shortcut(code(key::Code::KeyE), Modifiers::COMMAND),
            None
        );
        assert_eq!(
            activity_shortcut(code(key::Code::KeyE), Modifiers::SHIFT),
            None
        );
        assert_eq!(
            activity_shortcut(code(key::Code::KeyE), ctrl_shift() | Modifiers::ALT),
            None
        );
        assert_eq!(activity_shortcut(code(key::Code::KeyX), ctrl_shift()), None);
    }

    #[test]
    fn activity_icon_color_reflects_state() {
        assert_eq!(activity_icon_color(false, false), palette::ICON_INACTIVE);
        assert_eq!(activity_icon_color(false, true), palette::ICON_HOVERED);
        assert_eq!(activity_icon_color(true, false), palette::ICON_ACTIVE);
        assert_eq!(activity_icon_color(true, true), palette::ICON_ACTIVE);
    }

    #[test]
    fn hover_tracks_the_entry_under_the_pointer() {
        let mut app = Kegon::default();

        app.update(Message::ActivityHovered(ActivityItem::Search));
        assert_eq!(app.hovered_activity, Some(ActivityItem::Search));

        // Moving straight to a neighbor may deliver the new enter before the
        // old exit; the stale exit must not clear the new hover.
        app.update(Message::ActivityHovered(ActivityItem::Git));
        app.update(Message::ActivityUnhovered(ActivityItem::Search));
        assert_eq!(app.hovered_activity, Some(ActivityItem::Git));

        app.update(Message::ActivityUnhovered(ActivityItem::Git));
        assert_eq!(app.hovered_activity, None);
    }

    #[test]
    fn activity_messages_update_the_workbench() {
        let mut app = Kegon::default();

        app.update(Message::ActivitySelected(ActivityItem::Git));
        assert_eq!(app.workbench.active_activity(), ActivityItem::Git);
    }

    #[test]
    fn sash_drag_resizes_the_side_bar_only_while_pressed() {
        let mut app = Kegon::default();
        let initial = app.workbench.side_bar_width();

        app.update(Message::SashDragged(350.0));
        assert_eq!(app.workbench.side_bar_width(), initial);

        app.update(Message::SashPressed);
        app.update(Message::SashDragged(350.0));
        assert_eq!(
            app.workbench.side_bar_width(),
            350.0 - ACTIVITY_BAR_WIDTH - SASH_WIDTH / 2.0
        );

        app.update(Message::SashReleased);
        app.update(Message::SashDragged(200.0));
        assert_eq!(
            app.workbench.side_bar_width(),
            350.0 - ACTIVITY_BAR_WIDTH - SASH_WIDTH / 2.0
        );
    }
}
