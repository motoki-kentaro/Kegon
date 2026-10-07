//! The iced application: messages, update logic, and the workbench view.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use iced::advanced::input_method::Event as ImeEvent;
use iced::futures::SinkExt;
use iced::keyboard::{self, Modifiers, key};
use iced::widget::canvas::Canvas;
use iced::widget::{Space, button, column, container, mouse_area, row, space, svg, text, tooltip};
use iced::{
    Border, Center, Color, Element, Event, Fill, Font, Subscription, Theme, event, font, mouse,
};

use crate::i18n::{FluentArgs, Locale, Localizer, MessageKey};
use crate::icons::{ICON_SIZE, activity_icon};
use crate::terminal::{
    DEFAULT_CELL_HEIGHT, DEFAULT_CELL_WIDTH, InputAction, SystemClipboard, TerminalEvent,
    TerminalProgram, TerminalSession, calculate_grid_size, format_paste, process_key_event,
};
use crate::workbench::{ActivityItem, TabId, Workbench};

const ACTIVITY_BAR_WIDTH: f32 = 48.0;
const SASH_WIDTH: f32 = 4.0;
const TAB_STRIP_HEIGHT: f32 = 35.0;
const UI_TEXT_SIZE: f32 = 13.0;

static TERMINAL_EVENT_RX: Mutex<Option<Receiver<TerminalEvent>>> = Mutex::new(None);

fn terminal_events_stream() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(
        100,
        move |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
            loop {
                let event = {
                    if let Ok(rx_guard) = TERMINAL_EVENT_RX.lock() {
                        if let Some(rx) = rx_guard.as_ref() {
                            rx.try_recv().ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(evt) = event {
                    let _ = output.send(Message::TerminalEventReceived(evt)).await;
                } else {
                    iced::futures::pending!();
                }
            }
        },
    )
}

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
    IcedEventReceived(Event),
    TerminalEventReceived(TerminalEvent),
    TerminalClicked,
}

pub struct Kegon {
    /// Fixed for the lifetime of the application; chosen at startup.
    localizer: Localizer,
    workbench: Workbench,
    /// Whether the Side Bar sash is being dragged.
    resizing_side_bar: bool,
    /// The Activity Bar entry under the pointer, if any.
    hovered_activity: Option<ActivityItem>,
    /// Active terminal session.
    terminal_session: Option<TerminalSession>,
    /// System clipboard interface.
    system_clipboard: SystemClipboard,
    /// Transient Japanese IME preedit composition string.
    preedit_text: Option<String>,
    /// Whether keyboard focus is in the terminal.
    terminal_focused: bool,
    /// Last known mouse cursor position.
    cursor_position: iced::Point,
    /// Whether the mouse left button is currently dragging a text selection.
    mouse_dragging_selection: bool,
}

impl std::fmt::Debug for Kegon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kegon")
            .field("resizing_side_bar", &self.resizing_side_bar)
            .field("hovered_activity", &self.hovered_activity)
            .field("terminal_focused", &self.terminal_focused)
            .finish()
    }
}

impl Kegon {
    pub fn new(locale: Locale) -> Self {
        let (tx, rx) = channel();
        if let Ok(mut rx_guard) = TERMINAL_EVENT_RX.lock() {
            *rx_guard = Some(rx);
        }

        let terminal_session = TerminalSession::spawn(
            80,
            24,
            DEFAULT_CELL_WIDTH as u16,
            DEFAULT_CELL_HEIGHT as u16,
            tx,
        )
        .ok();

        Self {
            localizer: Localizer::new(locale),
            workbench: Workbench::default(),
            resizing_side_bar: false,
            hovered_activity: None,
            terminal_session,
            system_clipboard: SystemClipboard::new(),
            preedit_text: None,
            terminal_focused: true,
            cursor_position: iced::Point::ORIGIN,
            mouse_dragging_selection: false,
        }
    }

    /// The window title: the product name, which is not translated.
    pub fn title(&self) -> String {
        String::from("Kegon")
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ActivitySelected(item) => self.workbench.select_activity(item),
            Message::ActivityHovered(item) => self.hovered_activity = Some(item),
            Message::ActivityUnhovered(item) => {
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
            Message::TerminalClicked => {
                self.terminal_focused = true;
            }
            Message::TerminalEventReceived(_event) => {
                // Terminal event arrived; iced will re-render automatically.
            }
            Message::IcedEventReceived(event) => {
                self.handle_iced_event(event);
            }
        }
    }

    fn handle_iced_event(&mut self, event: Event) {
        match event {
            Event::Window(iced::window::Event::Resized(size)) => {
                if let Some(session) = &self.terminal_session {
                    let side_bar_w = self.workbench.side_bar_width();
                    let main_w =
                        (size.width - ACTIVITY_BAR_WIDTH - SASH_WIDTH - side_bar_w).max(10.0);
                    let main_h = (size.height - TAB_STRIP_HEIGHT).max(10.0);
                    let (cols, rows) = calculate_grid_size(main_w, main_h);
                    session.resize(
                        cols,
                        rows,
                        DEFAULT_CELL_WIDTH as u16,
                        DEFAULT_CELL_HEIGHT as u16,
                    );
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                physical_key,
                key: logical_key,
                modifiers,
                text,
                ..
            }) => {
                if self.terminal_focused
                    && let Some(session) = &self.terminal_session
                {
                    let has_selection = session.has_selection();
                    let app_cursor = session.is_app_cursor_keys();
                    let action = process_key_event(
                        physical_key,
                        &logical_key,
                        modifiers,
                        text.as_deref(),
                        has_selection,
                        app_cursor,
                    );

                    match action {
                        InputAction::WorkbenchShortcut(item) => {
                            self.workbench.select_activity(item);
                        }
                        InputAction::CopySelection => {
                            if let Some(selected_text) = session.copy_selection() {
                                self.system_clipboard.set_text(selected_text);
                                session.clear_selection();
                            }
                        }
                        InputAction::PasteFromClipboard => {
                            if let Some(pasted_text) = self.system_clipboard.get_text() {
                                let formatted =
                                    format_paste(&pasted_text, session.is_bracketed_paste());
                                session.write_input(formatted);
                            }
                        }
                        InputAction::SendToPty(bytes) => {
                            session.write_input(bytes);
                        }
                        InputAction::Ignore => {}
                    }
                }
            }
            Event::InputMethod(ime_event) => {
                if self.terminal_focused
                    && let Some(session) = &self.terminal_session
                {
                    match ime_event {
                        ImeEvent::Preedit(text, _) => {
                            self.preedit_text = if text.is_empty() { None } else { Some(text) };
                        }
                        ImeEvent::Commit(text) => {
                            self.preedit_text = None;
                            session.write_input(text.into_bytes());
                        }
                        ImeEvent::Closed => {
                            self.preedit_text = None;
                        }
                        _ => {}
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let term_x = ACTIVITY_BAR_WIDTH + SASH_WIDTH + self.workbench.side_bar_width();
                let term_y = TAB_STRIP_HEIGHT;

                if self.cursor_position.x >= term_x && self.cursor_position.y >= term_y {
                    self.terminal_focused = true;
                    if let Some(session) = &self.terminal_session {
                        let local_x = (self.cursor_position.x - term_x).max(0.0);
                        let local_y = (self.cursor_position.y - term_y).max(0.0);
                        let col = (local_x / DEFAULT_CELL_WIDTH).floor() as usize;
                        let line = (local_y / DEFAULT_CELL_HEIGHT).floor() as usize;
                        session.start_selection(col, line);
                        self.mouse_dragging_selection = true;
                    }
                } else {
                    self.terminal_focused = false;
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                self.cursor_position = position;
                if self.mouse_dragging_selection
                    && let Some(session) = &self.terminal_session
                {
                    let term_x = ACTIVITY_BAR_WIDTH + SASH_WIDTH + self.workbench.side_bar_width();
                    let term_y = TAB_STRIP_HEIGHT;
                    let local_x = (position.x - term_x).max(0.0);
                    let local_y = (position.y - term_y).max(0.0);
                    let col = (local_x / DEFAULT_CELL_WIDTH).floor() as usize;
                    let line = (local_y / DEFAULT_CELL_HEIGHT).floor() as usize;
                    session.update_selection(col, line);
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                self.mouse_dragging_selection = false;
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if let Some(session) = &self.terminal_session {
                    let lines = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => (y * 3.0) as i32,
                        mouse::ScrollDelta::Pixels { y, .. } => (y / 6.0) as i32,
                    };
                    if lines != 0 {
                        session.scroll_display(lines);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let events = event::listen().map(Message::IcedEventReceived);
        let terminal_events = Subscription::run(terminal_events_stream);

        if self.resizing_side_bar {
            let drag = event::listen_with(|event, _status, _window| match event {
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(Message::SashDragged(position.x))
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::SashReleased)
                }
                _ => None,
            });

            Subscription::batch([events, terminal_events, drag])
        } else {
            Subscription::batch([events, terminal_events])
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

            let label = self.localizer.text(activity_label(item));
            tooltip(entry, tooltip_label(label), tooltip::Position::Right).into()
        });

        container(column(items))
            .width(ACTIVITY_BAR_WIDTH)
            .height(Fill)
            .style(|_| container::Style::default().background(palette::ACTIVITY_BAR))
            .into()
    }

    fn side_bar(&self) -> Element<'_, Message> {
        let (title, placeholder) = side_bar_text(self.workbench.active_activity());

        let header = container(
            text(self.localizer.text(title))
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
                text(self.localizer.text(placeholder))
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

        let terminal_view: Element<'_, Message> = if let Some(session) = &self.terminal_session {
            let canvas_program = TerminalProgram {
                session,
                preedit_text: self.preedit_text.as_deref(),
                is_focused: self.terminal_focused,
            };

            let canvas_widget = Canvas::new(canvas_program).width(Fill).height(Fill);

            let main_content: Element<'_, Message> = mouse_area(canvas_widget)
                .on_press(Message::TerminalClicked)
                .into();

            if session.is_exited() {
                let code_str = session
                    .exit_code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| String::from("N/A"));

                let mut args = FluentArgs::new();
                args.set("code", code_str);

                let exit_banner = container(
                    text(
                        self.localizer
                            .text_with(MessageKey::TerminalProcessExited, &args),
                    )
                    .size(UI_TEXT_SIZE)
                    .color(Color::from_rgb8(0xf1, 0x4c, 0x4c)),
                )
                .padding([6, 12])
                .style(|_| {
                    container::Style::default().background(Color::from_rgb8(0x2d, 0x20, 0x20))
                });

                column![main_content, exit_banner].height(Fill).into()
            } else {
                main_content
            }
        } else {
            let placeholder = column![
                text(self.localizer.text(MessageKey::MainPlaceholderTitle))
                    .size(UI_TEXT_SIZE + 2.0)
                    .color(palette::TEXT),
                text(self.localizer.text(MessageKey::MainPlaceholderBody))
                    .size(UI_TEXT_SIZE)
                    .color(palette::TEXT_MUTED),
            ]
            .spacing(6)
            .align_x(Center);

            container(placeholder)
                .center(Fill)
                .style(|_| container::Style::default().background(palette::EDITOR))
                .into()
        };

        column![tab_strip, terminal_view].width(Fill).into()
    }

    fn tab_strip(&self) -> Element<'_, Message> {
        let tabs = self.workbench.terminal_tabs();

        let tab_buttons = tabs.iter().map(|tab| {
            let is_active = tabs.is_active(tab.id);
            let title = self.localizer.text(MessageKey::TerminalTabDefaultTitle);

            button(
                container(text(title).size(UI_TEXT_SIZE))
                    .height(Fill)
                    .align_y(Center),
            )
            .height(TAB_STRIP_HEIGHT)
            .padding([0, 16])
            .on_press(Message::TerminalTabSelected(tab.id))
            .style(move |_, status| tab_style(is_active, status))
            .into()
        });

        let new_tab = tooltip(
            button(container(text("+").size(16)).center(Fill))
                .width(TAB_STRIP_HEIGHT)
                .height(TAB_STRIP_HEIGHT)
                .padding(0)
                .style(|_, _| button::Style {
                    text_color: palette::TEXT_MUTED,
                    ..button::Style::default()
                }),
            tooltip_label(self.localizer.text(MessageKey::TerminalNewTabTooltip)),
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

#[allow(dead_code)]
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

fn activity_label(item: ActivityItem) -> MessageKey {
    match item {
        ActivityItem::Explorer => MessageKey::ActivityExplorer,
        ActivityItem::Search => MessageKey::ActivitySearch,
        ActivityItem::Git => MessageKey::ActivityGit,
    }
}

fn side_bar_text(item: ActivityItem) -> (MessageKey, MessageKey) {
    match item {
        ActivityItem::Explorer => (
            MessageKey::SideBarExplorerTitle,
            MessageKey::SideBarExplorerPlaceholder,
        ),
        ActivityItem::Search => (
            MessageKey::SideBarSearchTitle,
            MessageKey::SideBarSearchPlaceholder,
        ),
        ActivityItem::Git => (
            MessageKey::SideBarGitTitle,
            MessageKey::SideBarGitPlaceholder,
        ),
    }
}

fn tooltip_label<'a>(label: String) -> Element<'a, Message> {
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
    fn each_activity_has_its_own_messages() {
        let keys: std::collections::HashSet<_> = ActivityItem::ALL
            .into_iter()
            .flat_map(|item| {
                let (title, placeholder) = side_bar_text(item);
                [activity_label(item), title, placeholder]
            })
            .collect();

        assert_eq!(keys.len(), ActivityItem::ALL.len() * 3);
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
        let mut app = Kegon::new(Locale::EnUs);

        app.update(Message::ActivityHovered(ActivityItem::Search));
        assert_eq!(app.hovered_activity, Some(ActivityItem::Search));

        app.update(Message::ActivityHovered(ActivityItem::Git));
        app.update(Message::ActivityUnhovered(ActivityItem::Search));
        assert_eq!(app.hovered_activity, Some(ActivityItem::Git));

        app.update(Message::ActivityUnhovered(ActivityItem::Git));
        assert_eq!(app.hovered_activity, None);
    }

    #[test]
    fn activity_messages_update_the_workbench() {
        let mut app = Kegon::new(Locale::EnUs);

        app.update(Message::ActivitySelected(ActivityItem::Git));
        assert_eq!(app.workbench.active_activity(), ActivityItem::Git);
    }

    #[test]
    fn sash_drag_resizes_the_side_bar_only_while_pressed() {
        let mut app = Kegon::new(Locale::EnUs);
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
