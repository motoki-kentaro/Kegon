//! The iced application: messages, update logic, and the workbench view.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use iced::advanced::input_method::Event as ImeEvent;
use iced::futures::SinkExt;
use iced::keyboard;
use iced::widget::canvas::Canvas;
use iced::widget::{
    Space, button, column, container, mouse_area, pick_list, row, space, svg, text, tooltip,
};
use iced::{
    Border, Center, Color, Element, Event, Fill, Font, Subscription, Theme, event, font, mouse,
};

use crate::cli::{SmokeConfirmationDialog, SmokeFontPicker};
use crate::command::{CommandContext, CommandDispatcher, KeybindingResolver, Platform};
use crate::dialog::{
    ActionTone, ConfirmationDialog, ConfirmationResult, DialogKind, FocusTarget, FocusedAction,
    FontPicker, FontPickerFocus, FontPickerMode, FontPickerResult, render_font_picker_overlay,
    render_modal_overlay,
};
use crate::font::{FontCache, SystemFontCatalog, UiFontResolution, UiFontStatus, resolve_ui_font};
use crate::i18n::{self, FluentArgs, Locale, Localizer, MessageKey};
use crate::icons::{ICON_SIZE, activity_icon};
use crate::settings::{
    ApplicationSettings, LocalePreference, ThemePreference, resolve_application_locale,
    save_settings,
};
use crate::terminal::{
    DEFAULT_CELL_HEIGHT, DEFAULT_CELL_WIDTH, InputArbiter, InputRoute, SystemClipboard,
    TerminalEvent, TerminalInputEvent, TerminalKeyEncoder, TerminalProgram, TerminalSession,
    calculate_grid_size,
};
use crate::theme::{KegonTheme, ThemeId, resolve_theme, style};
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
    DialogPrimaryClicked,
    DialogSecondaryClicked,
    DialogBackdropClicked,
    SettingsLocaleChanged(LocalePreference),
    SettingsThemeChanged(ThemeId),
    SettingsChooseUiFontClicked,
    SettingsResetUiFontClicked,
    FontPickerSearchChanged(String),
    FontPickerMonospaceToggled(bool),
    FontPickerCandidateSelected(usize),
    FontPickerResultReceived(FontPickerResult),
    FontPickerBackdropClicked,
}

pub struct Kegon {
    /// Fixed for the lifetime of the application; chosen at startup or updated on settings change.
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
    /// Keybinding resolver and platform configuration.
    keybinding_resolver: KeybindingResolver,
    /// Active modal confirmation dialog, if any. Zero-or-one active modal policy.
    modal: Option<ConfirmationDialog>,
    /// Result of the last closed dialog.
    last_dialog_result: Option<ConfirmationResult>,
    /// Active font picker dialog, if any. Zero-or-one active modal policy.
    font_picker: Option<FontPicker>,
    /// System font catalog (loaded on demand when font picker is opened).
    font_catalog: Option<SystemFontCatalog>,
    /// Font bytes cache for runtime font previews.
    font_cache: FontCache,
    /// Result of the last closed font picker dialog.
    last_font_picker_result: Option<FontPickerResult>,
    /// Persisted application settings model.
    settings: ApplicationSettings,
    /// Process-level CLI locale override, if given (e.g. `--locale ja-JP`).
    cli_locale_override: Option<String>,
    /// Active UI font applied to application chrome and workbench UI components.
    active_ui_font: Font,
    /// UI font resolution status.
    ui_font_status: UiFontStatus,
    /// The theme every view draws with, resolved from
    /// `settings.appearance.theme`. Views never read the setting directly.
    active_theme: &'static KegonTheme,
    /// Where settings are saved. `None` means the OS configuration directory.
    settings_path: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for Kegon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kegon")
            .field("resizing_side_bar", &self.resizing_side_bar)
            .field("hovered_activity", &self.hovered_activity)
            .field("terminal_focused", &self.terminal_focused)
            .field("modal", &self.modal)
            .field("settings", &self.settings)
            .field("ui_font_status", &self.ui_font_status)
            .field("active_theme", &self.active_theme.id)
            .finish()
    }
}

impl Kegon {
    pub fn new(
        locale: Locale,
        settings: ApplicationSettings,
        cli_locale_override: Option<String>,
        smoke_dialog: Option<SmokeConfirmationDialog>,
        smoke_font_picker: Option<SmokeFontPicker>,
    ) -> Self {
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

        let active_theme = resolve_theme(settings.appearance.theme.effective_id());

        let mut app = Self {
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
            keybinding_resolver: KeybindingResolver::default_for_platform(Platform::current()),
            modal: None,
            last_dialog_result: None,
            font_picker: None,
            font_catalog: None,
            font_cache: FontCache::default(),
            last_font_picker_result: None,
            settings,
            cli_locale_override,
            active_ui_font: Font::DEFAULT,
            ui_font_status: UiFontStatus::SystemDefault,
            active_theme,
            settings_path: None,
        };

        app.apply_ui_font_preference();

        if let Some(smoke) = smoke_dialog {
            let (kind, title, message, tone) = match smoke {
                SmokeConfirmationDialog::Question => (
                    DialogKind::Question,
                    MessageKey::DialogSmokeQuestionTitle,
                    MessageKey::DialogSmokeQuestionMessage,
                    ActionTone::Normal,
                ),
                SmokeConfirmationDialog::Warning => (
                    DialogKind::Warning,
                    MessageKey::DialogSmokeWarningTitle,
                    MessageKey::DialogSmokeWarningMessage,
                    ActionTone::Destructive,
                ),
            };

            let dialog = ConfirmationDialog::builder(kind, title, message)
                .primary_action(MessageKey::DialogActionContinue, tone)
                .secondary_action(MessageKey::DialogActionCancel)
                .restore_focus(FocusTarget::Terminal)
                .build();

            app.open_confirmation_dialog(dialog);
        }

        if let Some(smoke_picker) = smoke_font_picker {
            let mode = match smoke_picker {
                SmokeFontPicker::Ui => FontPickerMode::Ui,
                SmokeFontPicker::Terminal => FontPickerMode::Terminal,
            };
            app.open_font_picker(mode);
        }

        app
    }

    /// The window title: the product name, which is not translated.
    pub fn title(&self) -> String {
        String::from("Kegon")
    }

    /// The iced theme for widgets Kegon does not style itself.
    pub fn iced_theme(&self) -> Theme {
        self.active_theme.iced_theme()
    }

    /// Resolves the configured theme into the active theme.
    fn apply_theme_preference(&mut self) {
        self.active_theme = resolve_theme(self.settings.appearance.theme.effective_id());
    }

    /// Saves `updated` through the settings store and makes it current.
    fn commit_settings(&mut self, updated: ApplicationSettings) {
        if let Err(err) = save_settings(&updated, self.settings_path.as_deref()) {
            eprintln!("kegon: failed to save settings: {err}");
        }
        self.settings = updated;
    }

    /// Re-resolves and updates the localizer based on current settings and CLI override.
    fn apply_locale_preference(&mut self) {
        let new_locale = resolve_application_locale(
            self.cli_locale_override.as_deref(),
            self.settings.locale,
            i18n::os_preferences(),
        );
        self.localizer = Localizer::new(new_locale);
    }

    /// Re-resolves and updates the active UI font based on current application settings.
    fn apply_ui_font_preference(&mut self) {
        if self.settings.appearance.ui_font_family.is_some() && self.font_catalog.is_none() {
            self.font_catalog = Some(SystemFontCatalog::load_system());
        }
        let catalog_ref = self.font_catalog.as_ref();
        let resolution = match (
            self.settings.appearance.ui_font_family.as_deref(),
            catalog_ref,
        ) {
            (Some(family), Some(catalog)) => {
                resolve_ui_font(Some(family), catalog, &mut self.font_cache)
            }
            _ => UiFontResolution {
                status: UiFontStatus::SystemDefault,
                font: Font::DEFAULT,
            },
        };
        self.active_ui_font = resolution.font;
        self.ui_font_status = resolution.status;
    }

    /// Opens a confirmation dialog. Returns true if opened, or false if a modal is already active.
    pub fn open_confirmation_dialog(&mut self, dialog: ConfirmationDialog) -> bool {
        if self.is_modal_open() {
            eprintln!("kegon: cannot open modal dialog; a modal is already active");
            false
        } else {
            self.modal = Some(dialog);
            true
        }
    }

    /// Opens a font picker dialog. Returns true if opened, or false if a modal is already active.
    pub fn open_font_picker(&mut self, mode: FontPickerMode) -> bool {
        if self.is_modal_open() {
            eprintln!("kegon: cannot open font picker; a modal is already active");
            false
        } else {
            if self.font_catalog.is_none() {
                self.font_catalog = Some(SystemFontCatalog::load_system());
            }
            let catalog = self.font_catalog.as_ref().unwrap().clone();
            let mut picker = FontPicker::new(mode, catalog);
            if mode == FontPickerMode::Ui
                && let Some(family) = &self.settings.appearance.ui_font_family
            {
                picker.select_family(family);
            }
            if let Some(candidate) = picker.highlighted_candidate() {
                self.font_cache.get_or_load(candidate);
            }
            self.font_picker = Some(picker);
            true
        }
    }

    /// Closes the active confirmation dialog and restores focus according to dialog settings.
    pub fn close_modal(&mut self, result: ConfirmationResult) {
        if let Some(dialog) = self.modal.take() {
            self.last_dialog_result = Some(result);
            match dialog.restore_focus {
                FocusTarget::Terminal => self.terminal_focused = true,
                FocusTarget::Workbench => self.terminal_focused = false,
            }
        }
    }

    /// Closes the active font picker dialog.
    pub fn close_font_picker(&mut self, result: FontPickerResult) {
        if let Some(_picker) = self.font_picker.take() {
            self.last_font_picker_result = Some(result);
            self.terminal_focused = true;
        }
    }

    /// Returns the result of the last closed confirmation dialog, if any.
    #[allow(dead_code)]
    pub fn last_dialog_result(&self) -> Option<ConfirmationResult> {
        self.last_dialog_result
    }

    /// Returns the result of the last closed font picker dialog, if any.
    #[allow(dead_code)]
    pub fn last_font_picker_result(&self) -> Option<FontPickerResult> {
        self.last_font_picker_result.clone()
    }

    /// Returns whether a modal dialog is currently active.
    #[allow(dead_code)]
    pub fn is_modal_open(&self) -> bool {
        self.modal.is_some() || self.font_picker.is_some()
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
                if !self.is_modal_open() {
                    self.terminal_focused = true;
                }
            }
            Message::TerminalEventReceived(_event) => {
                // Terminal event arrived; iced will re-render automatically.
            }
            Message::IcedEventReceived(event) => {
                self.handle_iced_event(event);
            }
            Message::DialogPrimaryClicked => {
                self.close_modal(ConfirmationResult::Primary);
            }
            Message::DialogSecondaryClicked => {
                self.close_modal(ConfirmationResult::Secondary);
            }
            Message::DialogBackdropClicked => {
                // Backdrop clicks are intentionally ignored to avoid accidental dismissal.
            }
            Message::SettingsLocaleChanged(new_preference) => {
                if self.cli_locale_override.is_none() {
                    let mut updated_settings = self.settings.clone();
                    updated_settings.locale = new_preference;
                    self.commit_settings(updated_settings);
                    self.apply_locale_preference();
                }
            }
            Message::SettingsThemeChanged(theme_id) => {
                let mut updated_settings = self.settings.clone();
                updated_settings.appearance.theme = ThemePreference::Builtin(theme_id);
                self.commit_settings(updated_settings);
                self.apply_theme_preference();
            }
            Message::SettingsChooseUiFontClicked => {
                self.open_font_picker(FontPickerMode::Ui);
            }
            Message::SettingsResetUiFontClicked => {
                let mut updated_settings = self.settings.clone();
                updated_settings.appearance.ui_font_family = None;
                self.commit_settings(updated_settings);
                self.apply_ui_font_preference();
            }
            Message::FontPickerSearchChanged(query) => {
                if let Some(picker) = self.font_picker.as_mut() {
                    picker.set_search_query(query);
                    if let Some(candidate) = picker.highlighted_candidate() {
                        self.font_cache.get_or_load(candidate);
                    }
                }
            }
            Message::FontPickerMonospaceToggled(_enabled) => {
                if let Some(picker) = self.font_picker.as_mut() {
                    picker.toggle_monospace_only();
                    if let Some(candidate) = picker.highlighted_candidate() {
                        self.font_cache.get_or_load(candidate);
                    }
                }
            }
            Message::FontPickerCandidateSelected(idx) => {
                if let Some(picker) = self.font_picker.as_mut() {
                    picker.select_index(idx);
                    if let Some(candidate) = picker.highlighted_candidate() {
                        self.font_cache.get_or_load(candidate);
                    }
                }
            }
            Message::FontPickerResultReceived(result) => {
                if let FontPickerResult::Select(candidate) = &result
                    && let Some(picker) = &self.font_picker
                    && picker.mode == FontPickerMode::Ui
                {
                    let mut updated_settings = self.settings.clone();
                    updated_settings.appearance.ui_font_family =
                        Some(candidate.family_name.clone());
                    self.commit_settings(updated_settings);
                    self.apply_ui_font_preference();
                }
                self.close_font_picker(result);
            }
            Message::FontPickerBackdropClicked => {
                // Backdrop clicks are intentionally ignored to avoid accidental dismissal.
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
                let context = if self.is_modal_open() {
                    CommandContext::Modal
                } else if self.terminal_focused {
                    CommandContext::TerminalFocused
                } else {
                    CommandContext::Workbench
                };

                let is_ime_composing = self.preedit_text.is_some();
                let is_modal_open = self.is_modal_open();
                let route = InputArbiter::arbitrate_key_event(
                    &self.keybinding_resolver,
                    context,
                    &logical_key,
                    modifiers,
                    is_ime_composing,
                    is_modal_open,
                );

                match route {
                    InputRoute::Modal => {
                        if let Some(picker) = self.font_picker.as_mut() {
                            match logical_key {
                                keyboard::key::Key::Named(keyboard::key::Named::Enter) => {
                                    let result = match picker.focus {
                                        FontPickerFocus::CancelButton => {
                                            Some(FontPickerResult::Cancel)
                                        }
                                        FontPickerFocus::MonospaceToggle => {
                                            picker.toggle_monospace_only();
                                            None
                                        }
                                        FontPickerFocus::SelectButton
                                        | FontPickerFocus::CandidateList
                                        | FontPickerFocus::SearchInput => {
                                            picker.confirm_selection()
                                        }
                                    };
                                    if let Some(res) = result {
                                        self.close_font_picker(res);
                                    }
                                }
                                keyboard::key::Key::Named(keyboard::key::Named::Escape) => {
                                    self.close_font_picker(FontPickerResult::Cancel);
                                }
                                keyboard::key::Key::Named(keyboard::key::Named::Tab) => {
                                    if modifiers.shift() {
                                        picker.focus_previous();
                                    } else {
                                        picker.focus_next();
                                    }
                                }
                                keyboard::key::Key::Named(keyboard::key::Named::ArrowUp) => {
                                    picker.move_highlight_up();
                                    if let Some(candidate) = picker.highlighted_candidate() {
                                        self.font_cache.get_or_load(candidate);
                                    }
                                }
                                keyboard::key::Key::Named(keyboard::key::Named::ArrowDown) => {
                                    picker.move_highlight_down();
                                    if let Some(candidate) = picker.highlighted_candidate() {
                                        self.font_cache.get_or_load(candidate);
                                    }
                                }
                                _ => {}
                            }
                        } else if let Some(dialog) = self.modal.as_mut() {
                            match logical_key {
                                keyboard::key::Key::Named(keyboard::key::Named::Enter) => {
                                    let result = match dialog.focused_action {
                                        FocusedAction::Primary => ConfirmationResult::Primary,
                                        FocusedAction::Secondary => ConfirmationResult::Secondary,
                                    };
                                    self.close_modal(result);
                                }
                                keyboard::key::Key::Named(keyboard::key::Named::Escape) => {
                                    self.close_modal(ConfirmationResult::Secondary);
                                }
                                keyboard::key::Key::Named(keyboard::key::Named::Tab) => {
                                    dialog.toggle_focus();
                                }
                                _ => {}
                            }
                        }
                    }
                    InputRoute::Ime => {}
                    InputRoute::Command(command_id) => {
                        CommandDispatcher::dispatch(
                            command_id,
                            &mut self.workbench,
                            self.terminal_session.as_ref(),
                            &mut self.system_clipboard,
                        );
                    }
                    InputRoute::Terminal => {
                        if self.terminal_focused
                            && let Some(session) = &self.terminal_session
                        {
                            let input_evt = TerminalInputEvent::press(
                                logical_key,
                                physical_key,
                                modifiers,
                                text.map(|s| s.to_string()),
                            );
                            let mode = session.keyboard_mode();
                            if let Some(bytes) = TerminalKeyEncoder::encode(&input_evt, mode) {
                                session.write_input(bytes);
                            }
                        }
                    }
                }
            }
            Event::Keyboard(keyboard::Event::KeyReleased {
                physical_key,
                key: logical_key,
                modifiers,
                ..
            }) => {
                if !self.is_modal_open()
                    && self.terminal_focused
                    && let Some(session) = &self.terminal_session
                {
                    let mode = session.keyboard_mode();
                    if mode.report_event_types {
                        let input_evt =
                            TerminalInputEvent::release(logical_key, physical_key, modifiers);
                        if let Some(bytes) = TerminalKeyEncoder::encode(&input_evt, mode) {
                            session.write_input(bytes);
                        }
                    }
                }
            }
            Event::InputMethod(ime_event) => {
                if !self.is_modal_open()
                    && self.terminal_focused
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
                if !self.is_modal_open() {
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
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                self.cursor_position = position;
                if !self.is_modal_open()
                    && self.mouse_dragging_selection
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
                if !self.is_modal_open() {
                    self.mouse_dragging_selection = false;
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if !self.is_modal_open()
                    && let Some(session) = &self.terminal_session
                {
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

        let content: Element<'_, Message> = if self.resizing_side_bar {
            mouse_area(body)
                .interaction(mouse::Interaction::ResizingHorizontally)
                .into()
        } else {
            body.into()
        };

        if let Some(font_picker) = &self.font_picker {
            render_font_picker_overlay(
                content,
                font_picker,
                &self.localizer,
                self.active_ui_font,
                self.active_theme,
                Message::FontPickerSearchChanged,
                Message::FontPickerMonospaceToggled,
                Message::FontPickerCandidateSelected,
                Message::FontPickerResultReceived,
                Message::FontPickerBackdropClicked,
            )
        } else if let Some(dialog) = &self.modal {
            render_modal_overlay(
                content,
                dialog,
                &self.localizer,
                self.active_ui_font,
                self.active_theme,
                |result| match result {
                    ConfirmationResult::Primary => Message::DialogPrimaryClicked,
                    ConfirmationResult::Secondary => Message::DialogSecondaryClicked,
                },
                Message::DialogBackdropClicked,
            )
        } else {
            content
        }
    }

    fn activity_bar(&self) -> Element<'_, Message> {
        let active = self.workbench.active_activity();

        let top_items = ActivityItem::TOP_ITEMS
            .into_iter()
            .map(|item| self.activity_bar_item(item, active));

        let settings_item = self.activity_bar_item(ActivityItem::Settings, active);

        let content = column![column(top_items), Space::new().height(Fill), settings_item,];

        let theme = self.active_theme;
        container(content)
            .width(ACTIVITY_BAR_WIDTH)
            .height(Fill)
            .style(move |_| activity_bar_style(theme))
            .into()
    }

    fn activity_bar_item(&self, item: ActivityItem, active: ActivityItem) -> Element<'_, Message> {
        let theme = self.active_theme;
        let is_active = item == active;
        let color = activity_icon_color(theme, is_active, self.hovered_activity == Some(item));

        let icon = svg(activity_icon(item))
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .style(move |_, _| svg::Style { color: Some(color) });

        let indicator = container(space())
            .width(2)
            .height(ACTIVITY_BAR_WIDTH)
            .style(move |_| {
                container::Style::default().background(if is_active {
                    theme.icons.active
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
        tooltip(
            entry,
            tooltip_label(label, self.active_ui_font, theme),
            tooltip::Position::Right,
        )
        .into()
    }

    fn side_bar(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let active_item = self.workbench.active_activity();
        let (title_key, placeholder_key) = side_bar_text(active_item);

        let header = container(
            text(self.localizer.text(title_key))
                .size(11)
                .font(Font {
                    weight: font::Weight::Bold,
                    ..self.active_ui_font
                })
                .color(theme.text.primary),
        )
        .height(TAB_STRIP_HEIGHT)
        .padding([0, 20])
        .align_y(Center);

        let content: Element<'_, Message> = if active_item == ActivityItem::Settings {
            let label_text = self.settings_label(MessageKey::SideBarSettingsLanguageLabel);

            let options = vec![
                LocaleOption {
                    pref: LocalePreference::System,
                    label: self
                        .localizer
                        .text(MessageKey::SideBarSettingsLanguageSystem),
                },
                LocaleOption {
                    pref: LocalePreference::EnUs,
                    label: String::from("English (United States)"),
                },
                LocaleOption {
                    pref: LocalePreference::JaJp,
                    label: String::from("日本語"),
                },
            ];

            let selected = options
                .iter()
                .find(|opt| opt.pref == self.settings.locale)
                .cloned();

            let picker = self.settings_pick_list(options, selected, |opt| {
                Message::SettingsLocaleChanged(opt.pref)
            });

            let mut language_col = column![label_text, picker].spacing(8);

            if self.cli_locale_override.is_some() {
                let note = text(
                    self.localizer
                        .text(MessageKey::SideBarSettingsCliOverrideNote),
                )
                .size(11.0)
                .font(self.active_ui_font)
                .color(theme.text.muted);
                language_col = language_col.push(note);
            }

            let appearance_header = text(
                self.localizer
                    .text(MessageKey::SideBarSettingsAppearanceTitle),
            )
            .size(11)
            .font(Font {
                weight: font::Weight::Bold,
                ..self.active_ui_font
            })
            .color(theme.text.primary);

            let theme_label = self.settings_label(MessageKey::SideBarSettingsThemeLabel);
            let theme_options: Vec<ThemeOption> =
                ThemeId::ALL.into_iter().map(ThemeOption).collect();
            let theme_picker =
                self.settings_pick_list(theme_options, Some(ThemeOption(theme.id)), |opt| {
                    Message::SettingsThemeChanged(opt.0)
                });

            let ui_font_label = self.settings_label(MessageKey::SideBarSettingsUiFontLabel);

            let font_status_text = match &self.ui_font_status {
                UiFontStatus::SystemDefault => {
                    self.localizer.text(MessageKey::SideBarSettingsUiFontSystem)
                }
                UiFontStatus::Active(family) => family.clone(),
                UiFontStatus::NotInstalled(family) | UiFontStatus::LoadFailed(family) => {
                    let mut args = FluentArgs::new();
                    args.set("family", family.clone());
                    self.localizer
                        .text_with(MessageKey::SideBarSettingsUiFontNotInstalled, &args)
                }
            };

            let font_status_val = text(font_status_text)
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font)
                .color(theme.text.muted);

            let choose_btn = button(
                text(self.localizer.text(MessageKey::SideBarSettingsUiFontChoose))
                    .size(UI_TEXT_SIZE)
                    .font(self.active_ui_font),
            )
            .style(move |_, status| style::secondary_button(theme, status, false))
            .on_press(Message::SettingsChooseUiFontClicked);

            let mut reset_btn = button(
                text(self.localizer.text(MessageKey::SideBarSettingsUiFontReset))
                    .size(UI_TEXT_SIZE)
                    .font(self.active_ui_font),
            )
            .style(move |_, status| style::secondary_button(theme, status, false));
            if self.settings.appearance.ui_font_family.is_some() {
                reset_btn = reset_btn.on_press(Message::SettingsResetUiFontClicked);
            }

            let btn_row = row![choose_btn, reset_btn].spacing(8);

            let theme_col = column![theme_label, theme_picker].spacing(8);
            let ui_font_col = column![ui_font_label, font_status_val, btn_row].spacing(8);
            let appearance_col = column![appearance_header, theme_col, ui_font_col].spacing(12);

            let settings_col = column![language_col, Space::new().height(16), appearance_col];

            column![header, container(settings_col).padding([8, 20])].into()
        } else {
            column![
                header,
                container(
                    text(self.localizer.text(placeholder_key))
                        .size(UI_TEXT_SIZE)
                        .font(self.active_ui_font)
                        .color(theme.text.muted)
                )
                .padding([8, 20]),
            ]
            .into()
        };

        container(content)
            .width(self.workbench.side_bar_width())
            .height(Fill)
            .style(move |_| side_bar_style(theme))
            .into()
    }

    /// A label above a settings control.
    fn settings_label(&self, key: MessageKey) -> Element<'_, Message> {
        text(self.localizer.text(key))
            .size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .color(self.active_theme.text.primary)
            .into()
    }

    /// A themed settings drop-down.
    ///
    /// iced 0.14 renders pick list text with basic shaping by default, which
    /// ignores a named system UI font; `Shaping::Auto` makes the UI font apply.
    fn settings_pick_list<T>(
        &self,
        options: Vec<T>,
        selected: Option<T>,
        on_select: impl Fn(T) -> Message + 'static,
    ) -> Element<'_, Message>
    where
        T: ToString + PartialEq + Clone + 'static,
    {
        let theme = self.active_theme;
        pick_list(options, selected, on_select)
            .font(self.active_ui_font)
            .text_size(UI_TEXT_SIZE)
            .text_shaping(iced::widget::text::Shaping::Auto)
            .width(Fill)
            .style(move |_, status| pick_list_style(theme, status))
            .menu_style(move |_| pick_list_menu_style(theme))
            .into()
    }

    fn sash(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let line = container(Space::new().width(1).height(Fill))
            .style(move |_| container::Style::default().background(theme.workbench.border));

        let highlight = self.resizing_side_bar;
        let handle = container(line)
            .width(SASH_WIDTH)
            .height(Fill)
            .align_x(Center)
            .style(move |_| {
                container::Style::default().background(if highlight {
                    theme.interaction.accent
                } else {
                    theme.workbench.side_bar_background
                })
            });

        mouse_area(handle)
            .on_press(Message::SashPressed)
            .interaction(mouse::Interaction::ResizingHorizontally)
            .into()
    }

    fn main_area(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let tab_strip = self.tab_strip();

        let terminal_view: Element<'_, Message> = if let Some(session) = &self.terminal_session {
            let canvas_program = TerminalProgram {
                session,
                preedit_text: self.preedit_text.as_deref(),
                is_focused: self.terminal_focused,
                colors: &theme.terminal,
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
                    .font(self.active_ui_font)
                    .color(theme.semantic.destructive),
                )
                .padding([6, 12])
                .style(move |_| exit_banner_style(theme));

                column![main_content, exit_banner].height(Fill).into()
            } else {
                main_content
            }
        } else {
            let placeholder = column![
                text(self.localizer.text(MessageKey::MainPlaceholderTitle))
                    .size(UI_TEXT_SIZE + 2.0)
                    .font(self.active_ui_font)
                    .color(theme.text.primary),
                text(self.localizer.text(MessageKey::MainPlaceholderBody))
                    .size(UI_TEXT_SIZE)
                    .font(self.active_ui_font)
                    .color(theme.text.muted),
            ]
            .spacing(6)
            .align_x(Center);

            container(placeholder)
                .center(Fill)
                .style(move |_| workbench_content_style(theme))
                .into()
        };

        column![tab_strip, terminal_view].width(Fill).into()
    }

    fn tab_strip(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let tabs = self.workbench.terminal_tabs();

        let tab_buttons = tabs.iter().map(|tab| {
            let is_active = tabs.is_active(tab.id);
            let title = self.localizer.text(MessageKey::TerminalTabDefaultTitle);

            button(
                container(text(title).size(UI_TEXT_SIZE).font(self.active_ui_font))
                    .height(Fill)
                    .align_y(Center),
            )
            .height(TAB_STRIP_HEIGHT)
            .padding([0, 16])
            .on_press(Message::TerminalTabSelected(tab.id))
            .style(move |_, status| tab_style(theme, is_active, status))
            .into()
        });

        let new_tab = tooltip(
            button(container(text("+").size(16).font(self.active_ui_font)).center(Fill))
                .width(TAB_STRIP_HEIGHT)
                .height(TAB_STRIP_HEIGHT)
                .padding(0)
                .style(move |_, _| button::Style {
                    text_color: theme.text.muted,
                    ..button::Style::default()
                }),
            tooltip_label(
                self.localizer.text(MessageKey::TerminalNewTabTooltip),
                self.active_ui_font,
                theme,
            ),
            tooltip::Position::Bottom,
        );

        let strip = row(tab_buttons).push(new_tab).push(space::horizontal());

        container(strip)
            .width(Fill)
            .height(TAB_STRIP_HEIGHT)
            .style(move |_| tab_bar_style(theme))
            .into()
    }
}

fn activity_bar_style(theme: &KegonTheme) -> container::Style {
    container::Style::default().background(theme.workbench.activity_bar_background)
}

fn side_bar_style(theme: &KegonTheme) -> container::Style {
    container::Style::default().background(theme.workbench.side_bar_background)
}

fn tab_bar_style(theme: &KegonTheme) -> container::Style {
    container::Style::default().background(theme.workbench.tab_bar_background)
}

fn workbench_content_style(theme: &KegonTheme) -> container::Style {
    container::Style::default().background(theme.workbench.background)
}

/// The banner shown under a terminal whose process has exited: a faint
/// destructive tint over the workbench background.
fn exit_banner_style(theme: &KegonTheme) -> container::Style {
    const TINT: f32 = 0.1;
    container::Style::default().background(style::blend(
        theme.workbench.background,
        theme.semantic.destructive,
        TINT,
    ))
}

fn activity_icon_color(theme: &KegonTheme, is_active: bool, is_hovered: bool) -> Color {
    if is_active {
        theme.icons.active
    } else if is_hovered {
        theme.icons.hovered
    } else {
        theme.icons.inactive
    }
}

fn tab_style(theme: &KegonTheme, is_active: bool, status: button::Status) -> button::Style {
    // The active tab merges into the workbench content below it.
    let background = if is_active {
        theme.workbench.background
    } else if matches!(status, button::Status::Hovered) {
        theme.interaction.hover
    } else {
        theme.workbench.tab_bar_background
    };

    button::Style {
        background: Some(background.into()),
        text_color: if is_active {
            theme.text.emphasis
        } else {
            theme.text.muted
        },
        border: Border {
            color: theme.workbench.border,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..button::Style::default()
    }
}

fn pick_list_style(theme: &KegonTheme, status: pick_list::Status) -> pick_list::Style {
    let border_color = match status {
        pick_list::Status::Active => theme.workbench.border,
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => theme.interaction.accent,
    };

    pick_list::Style {
        text_color: theme.text.primary,
        placeholder_color: theme.text.muted,
        handle_color: theme.text.muted,
        background: theme.workbench.background.into(),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: 4.0.into(),
        },
    }
}

fn pick_list_menu_style(theme: &KegonTheme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: theme.workbench.surface_elevated.into(),
        border: Border {
            color: theme.workbench.border,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: theme.text.primary,
        selected_text_color: theme.text.emphasis,
        selected_background: theme.interaction.selection.into(),
        shadow: iced::Shadow::default(),
    }
}

fn activity_label(item: ActivityItem) -> MessageKey {
    match item {
        ActivityItem::Explorer => MessageKey::ActivityExplorer,
        ActivityItem::Search => MessageKey::ActivitySearch,
        ActivityItem::Git => MessageKey::ActivityGit,
        ActivityItem::Settings => MessageKey::ActivitySettings,
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
        ActivityItem::Settings => (
            MessageKey::SideBarSettingsTitle,
            MessageKey::SideBarSettingsTitle,
        ),
    }
}

fn tooltip_label<'a>(label: String, ui_font: Font, theme: &'a KegonTheme) -> Element<'a, Message> {
    container(
        text(label)
            .size(UI_TEXT_SIZE)
            .font(ui_font)
            .color(theme.text.primary),
    )
    .padding([4, 8])
    .style(move |_: &Theme| {
        container::Style::default()
            .background(theme.workbench.surface_elevated)
            .border(Border {
                color: theme.workbench.border,
                width: 1.0,
                radius: 3.0.into(),
            })
    })
    .into()
}

/// A theme in the Settings drop-down, shown by its product name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThemeOption(ThemeId);

impl std::fmt::Display for ThemeOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.display_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocaleOption {
    pref: LocalePreference,
    label: String,
}

impl std::fmt::Display for LocaleOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandId, KeyChord, Modifiers};

    #[test]
    fn shortcuts_select_activity_items() {
        let resolver = KeybindingResolver::default_for_platform(Platform::Windows);

        assert_eq!(
            resolver.resolve(CommandContext::Workbench, &KeyChord::ctrl_shift_char('e')),
            Some(CommandId::WorkbenchExplorerFocus)
        );
        assert_eq!(
            resolver.resolve(CommandContext::Workbench, &KeyChord::ctrl_shift_char('f')),
            Some(CommandId::WorkbenchSearchFocus)
        );
        assert_eq!(
            resolver.resolve(CommandContext::Workbench, &KeyChord::ctrl_shift_char('g')),
            Some(CommandId::WorkbenchSourceControlFocus)
        );
    }

    #[test]
    fn shortcuts_require_command_and_shift() {
        let resolver = KeybindingResolver::default_for_platform(Platform::Windows);

        assert_eq!(
            resolver.resolve(CommandContext::Workbench, &KeyChord::ctrl_char('e')),
            None
        );
        let ctrl_alt_shift_e = KeyChord::new(
            crate::command::Key::Character("e".into()),
            Modifiers {
                ctrl: true,
                shift: true,
                alt: true,
                super_key: false,
            },
        );
        assert_eq!(
            resolver.resolve(CommandContext::Workbench, &ctrl_alt_shift_e),
            None
        );
    }

    fn new_test_app(locale: Locale) -> Kegon {
        with_test_settings_path(Kegon::new(
            locale,
            ApplicationSettings::default(),
            None,
            None,
            None,
        ))
    }

    /// Points saves at a per-test temporary file, so tests never touch the
    /// user's real settings.
    fn with_test_settings_path(mut app: Kegon) -> Kegon {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let dir = std::env::temp_dir().join(format!(
            "kegon_app_test_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        app.settings_path = Some(dir.join("settings.toml"));
        app
    }

    fn background(style: &container::Style) -> Option<Color> {
        match style.background {
            Some(iced::Background::Color(color)) => Some(color),
            _ => None,
        }
    }

    /// A theme whose every token differs from Night Dark, to prove that
    /// styles read the theme they are given rather than fixed colors.
    fn synthetic_theme() -> &'static KegonTheme {
        let mut theme = *resolve_theme(ThemeId::NightDark);
        theme.workbench.background = Color::from_rgb8(1, 0, 0);
        theme.workbench.activity_bar_background = Color::from_rgb8(2, 0, 0);
        theme.workbench.side_bar_background = Color::from_rgb8(3, 0, 0);
        theme.workbench.tab_bar_background = Color::from_rgb8(4, 0, 0);
        theme.interaction.hover = Color::from_rgb8(5, 0, 0);
        theme.interaction.accent = Color::from_rgb8(6, 0, 0);
        theme.text.muted = Color::from_rgb8(7, 0, 0);
        theme.text.emphasis = Color::from_rgb8(8, 0, 0);
        theme.icons.active = Color::from_rgb8(9, 0, 0);
        theme.icons.hovered = Color::from_rgb8(10, 0, 0);
        theme.icons.inactive = Color::from_rgb8(11, 0, 0);
        Box::leak(Box::new(theme))
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

        assert_eq!(keys.len(), ActivityItem::ALL.len() * 3 - 1);
    }

    #[test]
    fn activity_icon_color_reflects_state() {
        let theme = synthetic_theme();
        assert_eq!(
            activity_icon_color(theme, false, false),
            theme.icons.inactive
        );
        assert_eq!(activity_icon_color(theme, false, true), theme.icons.hovered);
        assert_eq!(activity_icon_color(theme, true, false), theme.icons.active);
        assert_eq!(activity_icon_color(theme, true, true), theme.icons.active);
    }

    #[test]
    fn workbench_areas_take_their_backgrounds_from_the_theme() {
        let theme = synthetic_theme();
        assert_eq!(
            background(&activity_bar_style(theme)),
            Some(theme.workbench.activity_bar_background)
        );
        assert_eq!(
            background(&side_bar_style(theme)),
            Some(theme.workbench.side_bar_background)
        );
        assert_eq!(
            background(&tab_bar_style(theme)),
            Some(theme.workbench.tab_bar_background)
        );
        assert_eq!(
            background(&workbench_content_style(theme)),
            Some(theme.workbench.background)
        );
    }

    #[test]
    fn tabs_use_theme_tokens() {
        let theme = synthetic_theme();
        let tab_background = |style: &button::Style| match style.background {
            Some(iced::Background::Color(color)) => color,
            _ => panic!("expected a solid background"),
        };

        let active = tab_style(theme, true, button::Status::Active);
        assert_eq!(tab_background(&active), theme.workbench.background);
        assert_eq!(active.text_color, theme.text.emphasis);

        let inactive = tab_style(theme, false, button::Status::Active);
        assert_eq!(
            tab_background(&inactive),
            theme.workbench.tab_bar_background
        );
        assert_eq!(inactive.text_color, theme.text.muted);

        let hovered = tab_style(theme, false, button::Status::Hovered);
        assert_eq!(tab_background(&hovered), theme.interaction.hover);
    }

    #[test]
    fn settings_pick_lists_use_theme_tokens() {
        let theme = synthetic_theme();
        let idle = pick_list_style(theme, pick_list::Status::Active);
        assert_eq!(idle.text_color, theme.text.primary);
        assert_eq!(idle.handle_color, theme.text.muted);
        let hovered = pick_list_style(theme, pick_list::Status::Hovered);
        assert_eq!(hovered.border.color, theme.interaction.accent);
    }

    #[test]
    fn default_settings_activate_night_dark() {
        let app = new_test_app(Locale::EnUs);
        assert_eq!(app.active_theme.id, ThemeId::NightDark);
        assert_eq!(app.iced_theme(), Theme::Dark);
    }

    #[test]
    fn unknown_theme_setting_activates_night_dark_and_is_kept() {
        let mut settings = ApplicationSettings::default();
        settings.appearance.theme = ThemePreference::Unknown("future-theme".into());

        let app = Kegon::new(Locale::EnUs, settings, None, None, None);
        assert_eq!(app.active_theme.id, ThemeId::NightDark);
        assert_eq!(
            app.settings.appearance.theme,
            ThemePreference::Unknown("future-theme".into())
        );
    }

    #[test]
    fn theme_change_updates_the_active_theme_and_keeps_other_settings() {
        let settings = ApplicationSettings {
            locale: LocalePreference::JaJp,
            appearance: crate::settings::AppearanceSettings {
                theme: ThemePreference::Unknown("future-theme".into()),
                ui_font_family: None,
            },
        };
        let mut app = with_test_settings_path(Kegon::new(Locale::JaJp, settings, None, None, None));
        let font_before = app.active_ui_font;
        let font_status_before = app.ui_font_status.clone();

        app.update(Message::SettingsThemeChanged(ThemeId::NightDark));

        assert_eq!(
            app.settings.appearance.theme,
            ThemePreference::Builtin(ThemeId::NightDark)
        );
        assert_eq!(app.active_theme.id, ThemeId::NightDark);
        assert_eq!(app.settings.locale, LocalePreference::JaJp);
        assert_eq!(app.settings.appearance.ui_font_family, None);
        assert_eq!(app.active_ui_font, font_before);
        assert_eq!(app.ui_font_status, font_status_before);

        let saved = std::fs::read_to_string(app.settings_path.as_ref().unwrap()).unwrap();
        assert!(saved.contains("theme = \"night-dark\""), "{saved}");
        assert!(saved.contains("locale = \"ja-JP\""), "{saved}");
    }

    #[test]
    fn locale_change_does_not_touch_the_theme() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::SettingsLocaleChanged(LocalePreference::JaJp));
        assert_eq!(app.settings.appearance.theme, ThemePreference::default());
        assert_eq!(app.active_theme.id, ThemeId::NightDark);
    }

    #[test]
    fn hover_tracks_the_entry_under_the_pointer() {
        let mut app = new_test_app(Locale::EnUs);

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
        let mut app = new_test_app(Locale::EnUs);

        app.update(Message::ActivitySelected(ActivityItem::Git));
        assert_eq!(app.workbench.active_activity(), ActivityItem::Git);

        app.update(Message::ActivitySelected(ActivityItem::Settings));
        assert_eq!(app.workbench.active_activity(), ActivityItem::Settings);
    }

    #[test]
    fn sash_drag_resizes_the_side_bar_only_while_pressed() {
        let mut app = new_test_app(Locale::EnUs);
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

    #[test]
    fn single_active_modal_policy() {
        let mut app = new_test_app(Locale::EnUs);

        let d1 = ConfirmationDialog::builder(
            DialogKind::Question,
            MessageKey::DialogSmokeQuestionTitle,
            MessageKey::DialogSmokeQuestionMessage,
        )
        .build();

        let d2 = ConfirmationDialog::builder(
            DialogKind::Warning,
            MessageKey::DialogSmokeWarningTitle,
            MessageKey::DialogSmokeWarningMessage,
        )
        .build();

        assert!(app.open_confirmation_dialog(d1));
        assert!(app.is_modal_open());

        // Second modal is deterministically rejected
        assert!(!app.open_confirmation_dialog(d2));

        app.close_modal(ConfirmationResult::Secondary);
        assert!(!app.is_modal_open());
        assert_eq!(
            app.last_dialog_result(),
            Some(ConfirmationResult::Secondary)
        );
    }

    #[test]
    fn modal_focus_restoration() {
        let mut app = new_test_app(Locale::EnUs);

        let d = ConfirmationDialog::builder(
            DialogKind::Question,
            MessageKey::DialogSmokeQuestionTitle,
            MessageKey::DialogSmokeQuestionMessage,
        )
        .restore_focus(FocusTarget::Workbench)
        .build();

        app.terminal_focused = true;
        app.open_confirmation_dialog(d);

        app.close_modal(ConfirmationResult::Primary);
        assert!(!app.terminal_focused);
        assert_eq!(app.last_dialog_result(), Some(ConfirmationResult::Primary));
    }

    #[test]
    fn smoke_confirmation_dialog_initialization() {
        let app_q = Kegon::new(
            Locale::EnUs,
            ApplicationSettings::default(),
            None,
            Some(SmokeConfirmationDialog::Question),
            None,
        );
        assert!(app_q.is_modal_open());

        let app_w = Kegon::new(
            Locale::EnUs,
            ApplicationSettings::default(),
            None,
            Some(SmokeConfirmationDialog::Warning),
            None,
        );
        assert!(app_w.is_modal_open());
    }

    #[test]
    fn smoke_font_picker_initialization() {
        let app_ui = Kegon::new(
            Locale::EnUs,
            ApplicationSettings::default(),
            None,
            None,
            Some(SmokeFontPicker::Ui),
        );
        assert!(app_ui.is_modal_open());
        assert_eq!(
            app_ui.font_picker.as_ref().unwrap().mode,
            FontPickerMode::Ui
        );

        let app_term = Kegon::new(
            Locale::EnUs,
            ApplicationSettings::default(),
            None,
            None,
            Some(SmokeFontPicker::Terminal),
        );
        assert!(app_term.is_modal_open());
        assert_eq!(
            app_term.font_picker.as_ref().unwrap().mode,
            FontPickerMode::Terminal
        );
    }

    #[test]
    fn runtime_locale_switch_preserves_terminal_session() {
        let mut app = new_test_app(Locale::EnUs);
        let initial_text = app.localizer.text(MessageKey::ActivitySettings);
        assert_eq!(initial_text, "Settings");

        app.update(Message::SettingsLocaleChanged(LocalePreference::JaJp));
        let updated_text = app.localizer.text(MessageKey::ActivitySettings);
        assert_eq!(updated_text, "設定");
    }

    #[test]
    fn ui_font_settings_and_resolution() {
        let mut settings = ApplicationSettings::default();
        settings.appearance.ui_font_family = Some(String::from("Consolas"));

        let app = Kegon::new(Locale::EnUs, settings, None, None, None);
        assert!(matches!(
            app.ui_font_status,
            UiFontStatus::Active(_) | UiFontStatus::NotInstalled(_)
        ));
    }

    #[test]
    fn ui_font_choose_open_picker_and_reset() {
        let mut settings = ApplicationSettings::default();
        settings.appearance.ui_font_family = Some(String::from("Arial"));

        let mut app = with_test_settings_path(Kegon::new(Locale::EnUs, settings, None, None, None));
        assert_eq!(app.settings.appearance.ui_font_family, Some("Arial".into()));

        // Choose clicked opens font picker
        app.update(Message::SettingsChooseUiFontClicked);
        assert!(app.is_modal_open());
        assert_eq!(app.font_picker.as_ref().unwrap().mode, FontPickerMode::Ui);

        app.close_font_picker(FontPickerResult::Cancel);

        // Reset clicked sets ui_font_family to None
        app.update(Message::SettingsResetUiFontClicked);
        assert_eq!(app.settings.appearance.ui_font_family, None);
        assert_eq!(app.ui_font_status, UiFontStatus::SystemDefault);
    }
}
