//! The iced application: messages, update logic, and the workbench view.

use std::path::PathBuf;
use std::sync::Mutex;

use iced::advanced::input_method::Event as ImeEvent;
use iced::futures::SinkExt;
use iced::futures::StreamExt;
use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use iced::keyboard;
use iced::widget::canvas::Canvas;
use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::scrollable::{self, Direction, Scrollbar};
use iced::widget::{
    Id, Space, button, column, container, mouse_area, pick_list, row, space, svg, text, tooltip,
};
use iced::{
    Border, Center, Color, Element, Event, Fill, Font, Point, Size, Subscription, Task, Theme,
    event, font, mouse,
};

use crate::cli::{SmokeConfirmationDialog, SmokeFontPicker};
use crate::command::{CommandContext, CommandDispatcher, CommandId, KeybindingResolver, Platform};
use crate::dialog::{
    ActionTone, ConfirmationDialog, ConfirmationResult, DialogKind, FocusTarget, FocusedAction,
    FontPicker, FontPickerFocus, FontPickerMode, FontPickerResult, render_font_picker_overlay,
    render_modal_overlay,
};
use crate::font::{
    FontCache, SystemFontCatalog, TerminalCellMetrics, TerminalFontConfig, TerminalFontStatus,
    UiFontResolution, UiFontStatus, resolve_terminal_font, resolve_ui_font,
};
use crate::i18n::{self, FluentArgs, Locale, Localizer, MessageKey};
use crate::icons::{
    ICON_SIZE, TAB_ADD_ICON_SIZE, TAB_CLOSE_ICON_SIZE, TAB_SCROLL_ICON_SIZE, activity_icon,
    settings_icon, tab_add_icon, tab_close_icon, tab_scroll_left_icon, tab_scroll_right_icon,
};
use crate::settings::{
    ApplicationSettings, LocalePreference, ThemePreference, resolve_application_locale,
    save_settings,
};
use crate::terminal::{
    EventSink, InputArbiter, InputRoute, SystemClipboard, TerminalEvent, TerminalInputEvent,
    TerminalKeyEncoder, TerminalProgram, TerminalRegistry, TerminalSession, TerminalSurface,
    calculate_cursor_rectangle, registry::TerminalEntry,
};
use crate::theme::{KegonTheme, ThemeId, resolve_theme, style};
use crate::workbench::{ActivityItem, SIDE_BAR_DEFAULT_WIDTH, TabId, TabKind, Workbench};

const ACTIVITY_BAR_WIDTH: f32 = 48.0;
const SASH_WIDTH: f32 = 4.0;
const TAB_STRIP_HEIGHT: f32 = 35.0;
/// Width of every tab in the tab strip.
const TAB_WIDTH: f32 = 160.0;
const TAB_CLOSE_WIDTH: f32 = 24.0;
const TAB_SCROLL_BUTTON_WIDTH: f32 = 24.0;
const TAB_SCROLLABLE_ID: &str = "workbench_tab_scrollable";
const SCROLL_EPSILON: f32 = 1.0;
const UI_TEXT_SIZE: f32 = 13.0;
/// Widest the Settings page grows, so lines stay readable on wide windows.
const SETTINGS_PAGE_MAX_WIDTH: f32 = 560.0;

/// Everything one Activity Bar button needs.
struct ActivityButton {
    icon: iced::widget::svg::Handle,
    label: String,
    is_active: bool,
    is_hovered: bool,
    press: Message,
    enter: Message,
    exit: Message,
}

/// A terminal event tagged with the tab whose session produced it.
type TabTerminalEvent = (TabId, TerminalEvent);

/// An event sink that tags everything a session emits with its owning tab.
fn tagged_sink(tx: &UnboundedSender<TabTerminalEvent>, tab: TabId) -> EventSink {
    let tx = tx.clone();
    EventSink::from_fn(move |event| {
        // Fails only once the application has dropped the receiver.
        let _ = tx.unbounded_send((tab, event));
    })
}

/// Field-level form of [`Kegon::active_terminal`], for call sites that also borrow other fields.
fn active_entry<'a>(
    workbench: &Workbench,
    terminals: &'a TerminalRegistry,
) -> Option<&'a TerminalEntry> {
    terminals.get(workbench.tabs().active()?.id)
}

static TERMINAL_EVENT_RX: Mutex<Option<UnboundedReceiver<TabTerminalEvent>>> = Mutex::new(None);

fn terminal_events_stream() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(
        100,
        move |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
            let rx = {
                if let Ok(mut rx_guard) = TERMINAL_EVENT_RX.lock() {
                    rx_guard.take()
                } else {
                    None
                }
            };

            let Some(mut rx) = rx else {
                return;
            };

            while let Some((tab, evt)) = rx.next().await {
                if output
                    .send(Message::TerminalEventReceived(tab, evt))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        },
    )
}

/// Calculates clamped column and row line indices for a given screen position.
pub fn calculate_terminal_cell_point(
    position: Point,
    window_size: Size,
    side_bar_width: f32,
    metrics: &TerminalCellMetrics,
) -> (usize, usize) {
    let term_x = ACTIVITY_BAR_WIDTH + SASH_WIDTH + side_bar_width;
    let term_y = TAB_STRIP_HEIGHT;
    let term_w = (window_size.width - term_x).max(1.0);
    let term_h = (window_size.height - term_y).max(1.0);

    let (max_cols, max_rows) = metrics.grid_size(term_w, term_h);

    let local_x = (position.x - term_x).clamp(0.0, (term_w - 0.1).max(0.0));
    let local_y = (position.y - term_y).clamp(0.0, (term_h - 0.1).max(0.0));

    let col = ((local_x / metrics.cell_width).floor() as usize)
        .min((max_cols as usize).saturating_sub(1));
    let line = ((local_y / metrics.cell_height).floor() as usize)
        .min((max_rows as usize).saturating_sub(1));

    (col, line)
}

#[derive(Debug, Clone)]
pub enum Message {
    ActivitySelected(ActivityItem),
    ActivityHovered(ActivityItem),
    ActivityUnhovered(ActivityItem),
    /// The Settings gear at the bottom of the Activity Bar: opens (or re-selects) the Settings
    /// tab. It does not touch the Side Bar.
    OpenSettingsTab,
    SettingsGearHovered(bool),
    TabSelected(TabId),
    /// The tab strip's `+` button.
    NewTerminalTab,
    /// The close button of one tab.
    TabClosed(TabId),
    /// Viewport event emitted when the tab strip scrollable scrolls.
    TabStripScrolled(scrollable::Viewport),
    /// Scroll tabs to the left by one tab width.
    ScrollTabsLeft,
    /// Scroll tabs to the right by one tab width.
    ScrollTabsRight,
    SashPressed,
    SashDragged(f32),
    SashReleased,
    IcedEventReceived(Event),
    TerminalEventReceived(TabId, TerminalEvent),
    DialogPrimaryClicked,
    DialogSecondaryClicked,
    DialogBackdropClicked,
    SettingsLocaleChanged(LocalePreference),
    SettingsThemeChanged(ThemeId),
    SettingsChooseUiFontClicked,
    SettingsResetUiFontClicked,
    SettingsChooseTerminalFontClicked,
    SettingsResetTerminalFontClicked,
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
    /// Whether the pointer is over the Settings gear.
    settings_gear_hovered: bool,
    /// Terminal sessions (and their titles) by tab. Only the active tab's session receives
    /// input and is drawn; the others keep running in the background.
    terminals: TerminalRegistry,
    /// Sender cloned into every session, tagging its events with the owning tab.
    terminal_event_tx: UnboundedSender<TabTerminalEvent>,
    /// Set when the last tab was closed; `run_update` turns it into an application exit.
    exit_requested: bool,
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
    /// Active Terminal font configuration and derived cell metrics.
    terminal_font_config: TerminalFontConfig,
    /// Current window bounds size.
    window_size: Size,
    /// The theme every view draws with, resolved from
    /// `settings.appearance.theme`. Views never read the setting directly.
    active_theme: &'static KegonTheme,
    /// Where settings are saved. `None` means the OS configuration directory.
    settings_path: Option<std::path::PathBuf>,
    /// Horizontal scroll offset of the tab strip in pixels.
    tab_scroll_offset_x: f32,
    /// Viewport width of the tab strip scrollable in pixels.
    tab_viewport_width: f32,
    /// Pending scroll command to execute in run_update.
    pending_tab_scroll_task: Option<TabScrollTask>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TabScrollTask {
    SnapToEnd,
    ScrollTo(f32),
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
            .field("terminal_font_status", &self.terminal_font_config.status)
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
        let font_catalog = SystemFontCatalog::load_system();
        let mut font_cache = FontCache::default();
        let terminal_font_config = resolve_terminal_font(
            settings.appearance.terminal_font_family.as_deref(),
            &font_catalog,
            &mut font_cache,
        );

        let initial_window_size = Size::new(1200.0, 760.0);
        let side_bar_w = SIDE_BAR_DEFAULT_WIDTH;
        let main_w =
            (initial_window_size.width - ACTIVITY_BAR_WIDTH - SASH_WIDTH - side_bar_w).max(10.0);
        let main_h = (initial_window_size.height - TAB_STRIP_HEIGHT).max(10.0);
        let (cols, rows) = terminal_font_config.metrics.grid_size(main_w, main_h);

        let (tx, rx) = unbounded();
        if let Ok(mut rx_guard) = TERMINAL_EVENT_RX.lock() {
            *rx_guard = Some(rx);
        }

        let workbench = Workbench::default();
        let mut terminals = TerminalRegistry::new();
        if let Some(tab) = workbench.tabs().active().map(|tab| tab.id)
            && let Ok(session) = TerminalSession::spawn(
                cols,
                rows,
                terminal_font_config.metrics.cell_width as u16,
                terminal_font_config.metrics.cell_height as u16,
                tagged_sink(&tx, tab),
            )
        {
            let _ = terminals.insert(tab, session);
        }

        let active_theme = resolve_theme(settings.appearance.theme.effective_id());

        let mut app = Self {
            localizer: Localizer::new(locale),
            workbench,
            resizing_side_bar: false,
            hovered_activity: None,
            settings_gear_hovered: false,
            terminals,
            terminal_event_tx: tx,
            exit_requested: false,
            system_clipboard: SystemClipboard::new(),
            preedit_text: None,
            terminal_focused: true,
            cursor_position: iced::Point::ORIGIN,
            mouse_dragging_selection: false,
            keybinding_resolver: KeybindingResolver::default_for_platform(Platform::current()),
            modal: None,
            last_dialog_result: None,
            font_picker: None,
            font_catalog: Some(font_catalog),
            font_cache,
            last_font_picker_result: None,
            settings,
            cli_locale_override,
            active_ui_font: Font::DEFAULT,
            ui_font_status: UiFontStatus::SystemDefault,
            terminal_font_config,
            window_size: initial_window_size,
            active_theme,
            settings_path: None,
            tab_scroll_offset_x: 0.0,
            tab_viewport_width: 0.0,
            pending_tab_scroll_task: None,
        };

        app.apply_ui_font_preference();
        app.apply_terminal_font_preference();

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

    /// The window title: `<terminal title> - Kegon`, or the untranslated product name alone.
    pub fn title(&self) -> String {
        crate::terminal::title::window_title(self.active_terminal().and_then(TerminalEntry::title))
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

    /// Re-resolves and updates the active Terminal font based on current application settings.
    pub fn apply_terminal_font_preference(&mut self) {
        if self.font_catalog.is_none() {
            self.font_catalog = Some(SystemFontCatalog::load_system());
        }
        let catalog = self.font_catalog.as_ref().unwrap();
        self.terminal_font_config = resolve_terminal_font(
            self.settings.appearance.terminal_font_family.as_deref(),
            catalog,
            &mut self.font_cache,
        );
        self.resize_terminal_view();
    }

    /// The entry of the active tab, if the active tab hosts a terminal.
    fn active_terminal(&self) -> Option<&TerminalEntry> {
        active_entry(&self.workbench, &self.terminals)
    }

    /// The session of the active tab: the single target of input, selection, and scrolling.
    fn active_session(&self) -> Option<&TerminalSession> {
        self.active_terminal().map(TerminalEntry::session)
    }

    /// Terminal grid size (columns, rows) for the current viewport and cell metrics.
    fn terminal_grid_size(&self) -> (u16, u16) {
        let side_bar_w = self.workbench.side_bar_width();
        let main_w =
            (self.window_size.width - ACTIVITY_BAR_WIDTH - SASH_WIDTH - side_bar_w).max(10.0);
        let main_h = (self.window_size.height - TAB_STRIP_HEIGHT).max(10.0);
        self.terminal_font_config.metrics.grid_size(main_w, main_h)
    }

    /// Drops state that belongs to the terminal that was just left or closed, and puts input
    /// focus on the terminal now shown. A tab without a terminal (Settings) takes no terminal
    /// input, so nothing is focused then.
    fn focus_active_terminal(&mut self) {
        self.preedit_text = None;
        self.mouse_dragging_selection = false;
        self.terminal_focused = self.active_terminal().is_some();
    }

    /// Scrolls the strip to a tab that was just appended and activated.
    fn follow_new_last_tab(&mut self) {
        self.pending_tab_scroll_task = Some(TabScrollTask::SnapToEnd);
        let total_tabs = self.workbench.tabs().len();
        let base_width = self.tab_strip_base_width();
        let geometry =
            calculate_tab_strip_geometry(total_tabs, base_width, self.tab_scroll_offset_x);
        self.tab_scroll_offset_x = geometry.max_scroll;
    }

    /// Opens a tab that hosts no terminal (no PTY or session is created) and activates it. A
    /// singleton kind that is already open is only brought to the front.
    fn open_special_tab(&mut self, kind: TabKind) -> TabId {
        let (tab, is_new) = self.workbench.open_special_tab(kind);
        self.focus_active_terminal();
        if is_new {
            self.follow_new_last_tab();
        } else {
            let index = self.workbench.tabs().iter().position(|t| t.id == tab);
            if let Some(index) = index {
                self.ensure_tab_visible(index);
            }
        }
        tab
    }

    /// Opens a terminal in a new last tab and activates it. On failure nothing is left behind:
    /// the tab is removed again and the active tab is unchanged.
    fn open_terminal_tab(&mut self) -> Option<TabId> {
        let (cols, rows) = self.terminal_grid_size();
        let metrics = self.terminal_font_config.metrics;
        let tx = self.terminal_event_tx.clone();
        self.open_terminal_tab_with(|tab, cwd| {
            let spawn = |cwd| {
                TerminalSession::spawn_in(
                    cols,
                    rows,
                    metrics.cell_width as u16,
                    metrics.cell_height as u16,
                    tagged_sink(&tx, tab),
                    cwd,
                )
            };
            // A directory that cannot be entered must not stop the tab from opening.
            match cwd {
                Some(cwd) => spawn(Some(cwd)).or_else(|_| spawn(None)),
                None => spawn(None),
            }
        })
    }

    /// The directory a new terminal should start in: the one the active terminal's shell last
    /// reported, if it still exists. `None` means "use the default directory"; a directory the
    /// shell has not reported is never guessed.
    fn inheritable_cwd(&self) -> Option<PathBuf> {
        let cwd = self.active_terminal()?.reported_cwd()?;
        cwd.is_dir().then(|| cwd.to_path_buf())
    }

    fn open_terminal_tab_with(
        &mut self,
        spawn: impl FnOnce(TabId, Option<PathBuf>) -> Result<TerminalSession, String>,
    ) -> Option<TabId> {
        let cwd = self.inheritable_cwd();
        let tab = self.workbench.add_tab(TabKind::Terminal);
        let registered = match spawn(tab, cwd) {
            Ok(session) => self.terminals.insert(tab, session).is_ok(),
            Err(error) => {
                eprintln!("kegon: {error}");
                false
            }
        };
        if !registered {
            self.workbench.remove_tab(tab);
            return None;
        }
        self.workbench.select_tab(tab);
        self.focus_active_terminal();
        self.follow_new_last_tab();
        Some(tab)
    }

    /// Closes a tab together with its terminal session. Returns whether the application should
    /// exit because no tab is left.
    fn close_tab(&mut self, tab: TabId) -> bool {
        let was_active = self.workbench.tabs().is_active(tab);
        // Dropping the entry hands the session to its shutdown; later events for `tab` are
        // ignored by the registry.
        self.terminals.remove(tab);
        if !self.workbench.remove_tab(tab) {
            return false;
        }
        if was_active {
            self.focus_active_terminal();
        }
        self.clamp_tab_scroll_and_follow_active();
        self.workbench.tabs().is_empty()
    }

    /// Resizes every terminal session to fit the current viewport with active cell metrics.
    ///
    /// All sessions share one viewport, so background terminals follow it too: switching tabs then
    /// needs no resize, and the PTY and grid never lag behind what is shown.
    fn resize_terminal_view(&mut self) {
        {
            let (cols, rows) = self.terminal_grid_size();
            for (_, session) in self.terminals.sessions() {
                session.resize(
                    cols,
                    rows,
                    self.terminal_font_config.metrics.cell_width as u16,
                    self.terminal_font_config.metrics.cell_height as u16,
                );
            }
        }
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
            } else if mode == FontPickerMode::Terminal
                && let Some(family) = &self.settings.appearance.terminal_font_family
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

    /// Available width for tab buttons without any scroll navigation triangles or new tab button.
    fn tab_strip_base_width(&self) -> f32 {
        (self.window_size.width
            - ACTIVITY_BAR_WIDTH
            - SASH_WIDTH
            - self.workbench.side_bar_width()
            - TAB_STRIP_HEIGHT)
            .max(0.0)
    }

    /// Ensures that the tab at `index` is within the visible tab strip viewport, scrolling if needed.
    fn ensure_tab_visible(&mut self, index: usize) {
        let total_tabs = self.workbench.tabs().len();
        let base_width = self.tab_strip_base_width();
        if let Some(target_x) =
            tab_scroll_target_for_index(index, total_tabs, base_width, self.tab_scroll_offset_x)
        {
            self.tab_scroll_offset_x = target_x;
            self.pending_tab_scroll_task = Some(TabScrollTask::ScrollTo(target_x));
        }
    }

    /// Clamps current scroll offset to valid bounds and keeps the active tab visible on resize.
    fn clamp_tab_scroll_and_follow_active(&mut self) {
        let total_tabs = self.workbench.tabs().len();
        if total_tabs == 0 {
            return;
        }
        let base_width = self.tab_strip_base_width();
        let active_index = self
            .workbench
            .tabs()
            .active()
            .and_then(|active| self.workbench.tabs().iter().position(|t| t.id == active.id));

        if let Some(index) = active_index {
            if let Some(target_x) =
                tab_scroll_target_for_index(index, total_tabs, base_width, self.tab_scroll_offset_x)
            {
                self.tab_scroll_offset_x = target_x;
                self.pending_tab_scroll_task = Some(TabScrollTask::ScrollTo(target_x));
            }
        } else {
            let geometry =
                calculate_tab_strip_geometry(total_tabs, base_width, self.tab_scroll_offset_x);
            let clamped_x = self.tab_scroll_offset_x.min(geometry.max_scroll);
            if (clamped_x - self.tab_scroll_offset_x).abs() > SCROLL_EPSILON {
                self.tab_scroll_offset_x = clamped_x;
                self.pending_tab_scroll_task = Some(TabScrollTask::ScrollTo(clamped_x));
            }
        }
    }

    /// Takes the pending tab scroll task for execution in [`run_update`].
    fn take_tab_scroll_task(&mut self) -> Option<Task<Message>> {
        let action = self.pending_tab_scroll_task.take()?;
        let id = Id::new(TAB_SCROLLABLE_ID);
        match action {
            TabScrollTask::SnapToEnd => Some(operation::snap_to_end(id)),
            TabScrollTask::ScrollTo(x) => Some(operation::scroll_to(
                id,
                AbsoluteOffset {
                    x: Some(x),
                    y: None,
                },
            )),
        }
    }

    /// The update function handed to iced: [`Self::update`] plus the exit request it may leave.
    pub fn run_update(&mut self, message: Message) -> Task<Message> {
        self.update(message);
        if self.exit_requested {
            iced::exit()
        } else if let Some(task) = self.take_tab_scroll_task() {
            task
        } else {
            Task::none()
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ActivitySelected(item) => self.workbench.select_activity(item),
            Message::OpenSettingsTab => {
                self.open_special_tab(TabKind::Settings);
            }
            Message::SettingsGearHovered(hovered) => self.settings_gear_hovered = hovered,
            Message::ActivityHovered(item) => self.hovered_activity = Some(item),
            Message::ActivityUnhovered(item) => {
                if self.hovered_activity == Some(item) {
                    self.hovered_activity = None;
                }
            }
            Message::TabSelected(id) => {
                let previous = self.workbench.tabs().active().map(|tab| tab.id);
                self.workbench.select_tab(id);
                if self.workbench.tabs().active().map(|tab| tab.id) != previous {
                    self.focus_active_terminal();
                }
                let tab_index = self.workbench.tabs().iter().position(|t| t.id == id);
                if let Some(index) = tab_index {
                    self.ensure_tab_visible(index);
                }
            }
            Message::NewTerminalTab => {
                self.open_terminal_tab();
            }
            Message::TabClosed(id) => {
                if self.close_tab(id) {
                    self.exit_requested = true;
                }
            }
            Message::TabStripScrolled(viewport) => {
                self.tab_scroll_offset_x = viewport.absolute_offset().x;
                self.tab_viewport_width = viewport.bounds().width;
            }
            Message::ScrollTabsLeft => {
                let target_x = (self.tab_scroll_offset_x - TAB_WIDTH).max(0.0);
                if (target_x - self.tab_scroll_offset_x).abs() > SCROLL_EPSILON {
                    self.tab_scroll_offset_x = target_x;
                    self.pending_tab_scroll_task = Some(TabScrollTask::ScrollTo(target_x));
                }
            }
            Message::ScrollTabsRight => {
                let total_tabs = self.workbench.tabs().len();
                let base_width = self.tab_strip_base_width();
                let geometry =
                    calculate_tab_strip_geometry(total_tabs, base_width, self.tab_scroll_offset_x);
                let target_x = (self.tab_scroll_offset_x + TAB_WIDTH).min(geometry.max_scroll);
                if (target_x - self.tab_scroll_offset_x).abs() > SCROLL_EPSILON {
                    self.tab_scroll_offset_x = target_x;
                    self.pending_tab_scroll_task = Some(TabScrollTask::ScrollTo(target_x));
                }
            }
            Message::SashPressed => self.resizing_side_bar = true,
            Message::SashDragged(cursor_x) => {
                if self.resizing_side_bar {
                    self.workbench
                        .resize_side_bar(cursor_x - ACTIVITY_BAR_WIDTH - SASH_WIDTH / 2.0);
                    self.clamp_tab_scroll_and_follow_active();
                }
            }
            Message::SashReleased => {
                self.resizing_side_bar = false;
                self.clamp_tab_scroll_and_follow_active();
            }
            // Events from removed sessions are dropped by the registry.
            Message::TerminalEventReceived(tab, event) => {
                self.terminals.handle_event(tab, event);
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
            Message::SettingsChooseTerminalFontClicked => {
                self.open_font_picker(FontPickerMode::Terminal);
            }
            Message::SettingsResetTerminalFontClicked => {
                let mut updated_settings = self.settings.clone();
                updated_settings.appearance.terminal_font_family = None;
                self.commit_settings(updated_settings);
                self.apply_terminal_font_preference();
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
                {
                    match picker.mode {
                        FontPickerMode::Ui => {
                            let mut updated_settings = self.settings.clone();
                            updated_settings.appearance.ui_font_family =
                                Some(candidate.family_name.clone());
                            self.commit_settings(updated_settings);
                            self.apply_ui_font_preference();
                        }
                        FontPickerMode::Terminal => {
                            let mut updated_settings = self.settings.clone();
                            updated_settings.appearance.terminal_font_family =
                                Some(candidate.family_name.clone());
                            self.commit_settings(updated_settings);
                            self.apply_terminal_font_preference();
                        }
                    }
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
                self.window_size = size;
                self.resize_terminal_view();
                self.clamp_tab_scroll_and_follow_active();
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
                    // Opening a tab also moves focus and the tab strip, which only the
                    // application can do; the gear takes the same path.
                    InputRoute::Command(CommandId::WorkbenchSettingsOpen) => {
                        self.open_special_tab(TabKind::Settings);
                    }
                    InputRoute::Command(command_id) => {
                        let active_tab = self.workbench.tabs().active().map(|tab| tab.id);
                        CommandDispatcher::dispatch(
                            command_id,
                            &mut self.workbench,
                            active_tab
                                .and_then(|id| self.terminals.get(id))
                                .map(TerminalEntry::session),
                            &mut self.system_clipboard,
                        );
                    }
                    InputRoute::Terminal => {
                        if self.terminal_focused
                            && let Some(session) = self.active_session()
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
                    && let Some(session) = self.active_session()
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
                    && let Some(session) =
                        active_entry(&self.workbench, &self.terminals).map(TerminalEntry::session)
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
                        if let Some(session) = self.active_session() {
                            let (col, line) = calculate_terminal_cell_point(
                                self.cursor_position,
                                self.window_size,
                                self.workbench.side_bar_width(),
                                &self.terminal_font_config.metrics,
                            );
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
                    && let Some(session) = self.active_session()
                {
                    let (col, line) = calculate_terminal_cell_point(
                        position,
                        self.window_size,
                        self.workbench.side_bar_width(),
                        &self.terminal_font_config.metrics,
                    );
                    session.update_selection(col, line);
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if !self.is_modal_open() {
                    self.mouse_dragging_selection = false;
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let term_x = ACTIVITY_BAR_WIDTH + SASH_WIDTH + self.workbench.side_bar_width();
                let is_over_tab_strip = self.cursor_position.y >= 0.0
                    && self.cursor_position.y < TAB_STRIP_HEIGHT
                    && self.cursor_position.x >= term_x
                    && self.cursor_position.x <= self.window_size.width;

                if is_over_tab_strip {
                    // Only convert pure vertical wheel (x == 0) to horizontal scroll for mouse users.
                    // Touchpads and horizontal wheels emit delta.x directly, which iced's horizontal
                    // scrollable natively handles; handling x here would cause double-scrolling.
                    let (has_horizontal, vertical_y) = match delta {
                        mouse::ScrollDelta::Lines { x, y } => (x.abs() > 0.001, y),
                        mouse::ScrollDelta::Pixels { x, y } => (x.abs() > 0.001, y / 60.0),
                    };

                    if !has_horizontal && vertical_y.abs() > 0.001 {
                        let delta_x = -vertical_y * 60.0;
                        let total_tabs = self.workbench.tabs().len();
                        let base_width = self.tab_strip_base_width();
                        let geometry = calculate_tab_strip_geometry(
                            total_tabs,
                            base_width,
                            self.tab_scroll_offset_x,
                        );
                        let target_x =
                            (self.tab_scroll_offset_x + delta_x).clamp(0.0, geometry.max_scroll);
                        if (target_x - self.tab_scroll_offset_x).abs() > SCROLL_EPSILON {
                            self.tab_scroll_offset_x = target_x;
                            self.pending_tab_scroll_task = Some(TabScrollTask::ScrollTo(target_x));
                        }
                    }
                } else {
                    let is_over_terminal = self.cursor_position.x >= term_x
                        && self.cursor_position.y >= TAB_STRIP_HEIGHT
                        && self.cursor_position.x <= self.window_size.width
                        && self.cursor_position.y <= self.window_size.height;

                    if is_over_terminal
                        && !self.is_modal_open()
                        && let Some(session) = self.active_session()
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

        // The gear is an action, not an activity: it opens the Settings tab and leaves the
        // Side Bar as it is.
        let settings_active =
            self.workbench.tabs().active().map(|tab| tab.kind) == Some(TabKind::Settings);
        let settings_item = self.activity_button(ActivityButton {
            icon: settings_icon(),
            label: self.localizer.text(MessageKey::ActivitySettings),
            is_active: settings_active,
            is_hovered: self.settings_gear_hovered,
            press: Message::OpenSettingsTab,
            enter: Message::SettingsGearHovered(true),
            exit: Message::SettingsGearHovered(false),
        });

        let content = column![column(top_items), Space::new().height(Fill), settings_item,];

        let theme = self.active_theme;
        container(content)
            .width(ACTIVITY_BAR_WIDTH)
            .height(Fill)
            .style(move |_| activity_bar_style(theme))
            .into()
    }

    fn activity_bar_item(&self, item: ActivityItem, active: ActivityItem) -> Element<'_, Message> {
        self.activity_button(ActivityButton {
            icon: activity_icon(item),
            label: self.localizer.text(activity_label(item)),
            is_active: item == active,
            is_hovered: self.hovered_activity == Some(item),
            press: Message::ActivitySelected(item),
            enter: Message::ActivityHovered(item),
            exit: Message::ActivityUnhovered(item),
        })
    }

    fn activity_button(&self, spec: ActivityButton) -> Element<'_, Message> {
        let ActivityButton {
            icon,
            label,
            is_active,
            is_hovered,
            press,
            enter,
            exit,
        } = spec;
        let theme = self.active_theme;
        let color = activity_icon_color(theme, is_active, is_hovered);

        let icon = svg(icon)
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
            .on_press(press)
            .style(|_, _| button::Style::default());

        let entry = mouse_area(row![indicator, entry])
            .on_enter(enter)
            .on_exit(exit);

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

        let content: Element<'_, Message> = column![
            header,
            container(
                text(self.localizer.text(placeholder_key))
                    .size(UI_TEXT_SIZE)
                    .font(self.active_ui_font)
                    .color(theme.text.muted)
            )
            .padding([8, 20]),
        ]
        .into();

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

        let terminal_view: Element<'_, Message> = if let Some(session) = self.active_session() {
            let canvas_program = TerminalProgram {
                session,
                preedit_text: self.preedit_text.as_deref(),
                is_focused: self.terminal_focused,
                colors: &theme.terminal,
                font_config: &self.terminal_font_config,
            };

            let canvas_widget = Canvas::new(canvas_program).width(Fill).height(Fill);

            let (cursor_col, cursor_line) = session.cursor_position();
            let cursor_rect = calculate_cursor_rectangle(
                cursor_col,
                cursor_line,
                &self.terminal_font_config.metrics,
            );

            let is_focused = self.terminal_focused && !self.is_modal_open();

            let main_content: Element<'_, Message> =
                TerminalSurface::new(canvas_widget, is_focused, Some(cursor_rect)).into();

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
        } else if self.workbench.tabs().active().map(|tab| tab.kind) == Some(TabKind::Settings) {
            self.settings_tab_view()
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

    /// Content of the Settings tab: a scrollable page of categories. Every control edits the
    /// application settings through the existing `Settings*` messages, which save immediately.
    fn settings_tab_view(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let label_text = self.settings_label(MessageKey::SettingsLanguageLabel);

        let options = vec![
            LocaleOption {
                pref: LocalePreference::System,
                label: self.localizer.text(MessageKey::SettingsLanguageSystem),
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
            let note = text(self.localizer.text(MessageKey::SettingsCliOverrideNote))
                .size(11.0)
                .font(self.active_ui_font)
                .color(theme.text.muted);
            language_col = language_col.push(note);
        }

        let theme_label = self.settings_label(MessageKey::SettingsThemeLabel);
        let theme_options: Vec<ThemeOption> = ThemeId::ALL.into_iter().map(ThemeOption).collect();
        let theme_picker =
            self.settings_pick_list(theme_options, Some(ThemeOption(theme.id)), |opt| {
                Message::SettingsThemeChanged(opt.0)
            });

        let ui_font_label = self.settings_label(MessageKey::SettingsUiFontLabel);

        let font_status_text = match &self.ui_font_status {
            UiFontStatus::SystemDefault => self.localizer.text(MessageKey::SettingsUiFontSystem),
            UiFontStatus::Active(family) => family.clone(),
            UiFontStatus::NotInstalled(family) | UiFontStatus::LoadFailed(family) => {
                let mut args = FluentArgs::new();
                args.set("family", family.clone());
                self.localizer
                    .text_with(MessageKey::SettingsUiFontNotInstalled, &args)
            }
        };

        let font_status_val = text(font_status_text)
            .size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .color(theme.text.muted);

        let choose_btn = button(
            text(self.localizer.text(MessageKey::SettingsUiFontChoose))
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font),
        )
        .style(move |_, status| style::secondary_button(theme, status, false))
        .on_press(Message::SettingsChooseUiFontClicked);

        let mut reset_btn = button(
            text(self.localizer.text(MessageKey::SettingsUiFontReset))
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

        let terminal_font_label = self.settings_label(MessageKey::SettingsTerminalFontLabel);

        let terminal_font_status_text = match &self.terminal_font_config.status {
            TerminalFontStatus::Default => {
                self.localizer.text(MessageKey::SettingsTerminalFontDefault)
            }
            TerminalFontStatus::Active(family) => family.clone(),
            TerminalFontStatus::NotInstalled(family) | TerminalFontStatus::LoadFailed(family) => {
                let mut args = FluentArgs::new();
                args.set("family", family.clone());
                self.localizer
                    .text_with(MessageKey::SettingsTerminalFontNotInstalled, &args)
            }
        };

        let terminal_font_status_val = text(terminal_font_status_text)
            .size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .color(theme.text.muted);

        let choose_terminal_font_btn = button(
            text(self.localizer.text(MessageKey::SettingsTerminalFontChoose))
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font),
        )
        .style(move |_, status| style::secondary_button(theme, status, false))
        .on_press(Message::SettingsChooseTerminalFontClicked);

        let mut reset_terminal_font_btn = button(
            text(self.localizer.text(MessageKey::SettingsTerminalFontReset))
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font),
        )
        .style(move |_, status| style::secondary_button(theme, status, false));
        if self.settings.appearance.terminal_font_family.is_some() {
            reset_terminal_font_btn =
                reset_terminal_font_btn.on_press(Message::SettingsResetTerminalFontClicked);
        }

        let terminal_font_btn_row =
            row![choose_terminal_font_btn, reset_terminal_font_btn].spacing(8);

        let terminal_font_col = column![
            terminal_font_label,
            terminal_font_status_val,
            terminal_font_btn_row
        ]
        .spacing(8);

        let heading = text(self.localizer.text(MessageKey::SettingsTabTitle))
            .size(UI_TEXT_SIZE + 6.0)
            .font(Font {
                weight: font::Weight::Bold,
                ..self.active_ui_font
            })
            .color(theme.text.primary);
        let description = text(self.localizer.text(MessageKey::SettingsTabDescription))
            .size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .color(theme.text.muted);

        let general = self.settings_category(
            MessageKey::SettingsCategoryGeneralTitle,
            MessageKey::SettingsCategoryGeneralDescription,
            language_col.into(),
        );
        let appearance = self.settings_category(
            MessageKey::SettingsCategoryAppearanceTitle,
            MessageKey::SettingsCategoryAppearanceDescription,
            column![theme_col, ui_font_col].spacing(16).into(),
        );
        let terminal = self.settings_category(
            MessageKey::SettingsCategoryTerminalTitle,
            MessageKey::SettingsCategoryTerminalDescription,
            terminal_font_col.into(),
        );

        let page = column![
            column![heading, description].spacing(6),
            general,
            appearance,
            terminal
        ]
        .spacing(28)
        .max_width(SETTINGS_PAGE_MAX_WIDTH);

        container(
            iced::widget::scrollable(container(page).padding(24).width(Fill))
                .width(Fill)
                .height(Fill),
        )
        .width(Fill)
        .height(Fill)
        .style(move |_| workbench_content_style(theme))
        .into()
    }

    /// One settings category: a title, a short description, then its controls.
    fn settings_category<'a>(
        &'a self,
        title: MessageKey,
        description: MessageKey,
        body: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let theme = self.active_theme;
        let title = text(self.localizer.text(title))
            .size(UI_TEXT_SIZE + 2.0)
            .font(Font {
                weight: font::Weight::Bold,
                ..self.active_ui_font
            })
            .color(theme.text.primary);
        let description = text(self.localizer.text(description))
            .size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .color(theme.text.muted);
        let divider = container(Space::new().width(Fill).height(1))
            .style(move |_| container::Style::default().background(theme.workbench.border));

        column![
            column![title, description].spacing(4),
            divider,
            container(body).padding([4, 0]),
        ]
        .spacing(10)
        .into()
    }

    /// Label of the active terminal tab: the child's title, or the translated default.
    #[cfg(test)]
    fn active_tab_title(&self) -> String {
        self.workbench
            .tabs()
            .active()
            .map(|tab| self.tab_title(tab.id))
            .unwrap_or_else(|| self.localizer.text(MessageKey::TerminalTabDefaultTitle))
    }

    /// Label of a tab: its terminal's title, or the translated default.
    fn tab_title(&self, tab: TabId) -> String {
        match self.workbench.tabs().get(tab).map(|t| t.kind) {
            Some(TabKind::Settings) => self.localizer.text(MessageKey::SettingsTabTitle),
            Some(TabKind::Terminal) | None => self
                .terminals
                .get(tab)
                .and_then(TerminalEntry::title)
                .map(str::to_string)
                .unwrap_or_else(|| self.localizer.text(MessageKey::TerminalTabDefaultTitle)),
        }
    }

    /// Tooltip of a tab's close button.
    fn tab_close_tooltip(&self, kind: TabKind) -> String {
        self.localizer.text(match kind {
            TabKind::Terminal => MessageKey::TerminalCloseTabTooltip,
            TabKind::Settings => MessageKey::SettingsTabCloseTooltip,
        })
    }

    fn tab_strip(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let tabs = self.workbench.tabs();
        let total_tabs = tabs.len();
        let base_width = self.tab_strip_base_width();
        let geometry =
            calculate_tab_strip_geometry(total_tabs, base_width, self.tab_scroll_offset_x);

        let tab_buttons = tabs.iter().map(|tab| {
            let is_active = tabs.is_active(tab.id);
            let title = self.tab_title(tab.id);

            // Two sibling buttons rather than a nested one, so a click on the close button can
            // never also select the tab.
            let select = button(
                container(
                    text(title)
                        .size(UI_TEXT_SIZE)
                        .font(self.active_ui_font)
                        .wrapping(text::Wrapping::None),
                )
                .width(Fill)
                .height(Fill)
                .clip(true)
                .align_y(Center),
            )
            .width(Fill)
            .height(TAB_STRIP_HEIGHT)
            .padding(iced::Padding::ZERO.left(12.0).right(4.0))
            .on_press(Message::TabSelected(tab.id))
            .style(move |_, status| tab_style(theme, is_active, status));

            let close = tooltip(
                button(
                    container(
                        svg(tab_close_icon())
                            .width(TAB_CLOSE_ICON_SIZE)
                            .height(TAB_CLOSE_ICON_SIZE)
                            .style(move |_, status| {
                                let color = if matches!(status, svg::Status::Hovered) || is_active {
                                    theme.icons.active
                                } else {
                                    theme.icons.inactive
                                };
                                svg::Style { color: Some(color) }
                            }),
                    )
                    .center(Fill),
                )
                .width(TAB_CLOSE_WIDTH)
                .height(TAB_STRIP_HEIGHT)
                .padding(0)
                .on_press(Message::TabClosed(tab.id))
                .style(move |_, status| tab_style(theme, is_active, status)),
                tooltip_label(self.tab_close_tooltip(tab.kind), self.active_ui_font, theme),
                tooltip::Position::Bottom,
            );

            container(row![select, close].height(TAB_STRIP_HEIGHT).align_y(Center))
                .width(TAB_WIDTH)
                .height(TAB_STRIP_HEIGHT)
                .into()
        });

        let scrollable_tabs = iced::widget::scrollable(row(tab_buttons).height(TAB_STRIP_HEIGHT))
            .direction(Direction::Horizontal(Scrollbar::hidden()))
            .id(Id::new(TAB_SCROLLABLE_ID))
            .on_scroll(Message::TabStripScrolled)
            .width(Fill)
            .height(TAB_STRIP_HEIGHT);

        let new_tab = tooltip(
            button(
                container(
                    svg(tab_add_icon())
                        .width(TAB_ADD_ICON_SIZE)
                        .height(TAB_ADD_ICON_SIZE)
                        .style(move |_, status| {
                            let color = if matches!(status, svg::Status::Hovered) {
                                theme.icons.active
                            } else {
                                theme.icons.inactive
                            };
                            svg::Style { color: Some(color) }
                        }),
                )
                .center(Fill),
            )
            .width(TAB_STRIP_HEIGHT)
            .height(TAB_STRIP_HEIGHT)
            .padding(0)
            .on_press(Message::NewTerminalTab)
            .style(move |_, status| tab_icon_button_style(theme, status)),
            tooltip_label(
                self.localizer.text(MessageKey::TerminalNewTabTooltip),
                self.active_ui_font,
                theme,
            ),
            tooltip::Position::Bottom,
        );

        let mut strip = row![].height(TAB_STRIP_HEIGHT).align_y(Center);

        if geometry.show_left_triangle {
            let left_btn = button(
                container(
                    svg(tab_scroll_left_icon())
                        .width(TAB_SCROLL_ICON_SIZE)
                        .height(TAB_SCROLL_ICON_SIZE)
                        .style(move |_, status| {
                            let color = if matches!(status, svg::Status::Hovered) {
                                theme.icons.active
                            } else {
                                theme.icons.inactive
                            };
                            svg::Style { color: Some(color) }
                        }),
                )
                .center(Fill),
            )
            .width(TAB_SCROLL_BUTTON_WIDTH)
            .height(TAB_STRIP_HEIGHT)
            .padding(0)
            .on_press(Message::ScrollTabsLeft)
            .style(move |_, status| tab_icon_button_style(theme, status));

            strip = strip.push(left_btn);
        }

        strip = strip.push(scrollable_tabs);

        if geometry.show_right_triangle {
            let right_btn = button(
                container(
                    svg(tab_scroll_right_icon())
                        .width(TAB_SCROLL_ICON_SIZE)
                        .height(TAB_SCROLL_ICON_SIZE)
                        .style(move |_, status| {
                            let color = if matches!(status, svg::Status::Hovered) {
                                theme.icons.active
                            } else {
                                theme.icons.inactive
                            };
                            svg::Style { color: Some(color) }
                        }),
                )
                .center(Fill),
            )
            .width(TAB_SCROLL_BUTTON_WIDTH)
            .height(TAB_STRIP_HEIGHT)
            .padding(0)
            .on_press(Message::ScrollTabsRight)
            .style(move |_, status| tab_icon_button_style(theme, status));

            strip = strip.push(right_btn);
        }

        strip = strip.push(new_tab);

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

fn tab_icon_button_style(theme: &KegonTheme, status: button::Status) -> button::Style {
    let background = if matches!(status, button::Status::Hovered) {
        theme.interaction.hover
    } else {
        theme.workbench.tab_bar_background
    };

    button::Style {
        background: Some(background.into()),
        text_color: theme.text.primary,
        border: Border {
            color: theme.workbench.border,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..button::Style::default()
    }
}

/// Geometry and visibility of the tab strip scrollable container and navigation triangles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabStripGeometry {
    pub show_left_triangle: bool,
    pub show_right_triangle: bool,
    pub effective_viewport_width: f32,
    pub max_scroll: f32,
}

impl TabStripGeometry {
    pub fn is_overflowing(&self) -> bool {
        self.show_left_triangle || self.show_right_triangle || self.max_scroll > 0.0
    }
}

/// Computes visibility of left/right triangle navigation buttons and effective viewport bounds.
///
/// Rules (Kegon Issue #53, Section 2):
/// - Fits without overflow: both triangles hidden.
/// - Overflow at left edge: left hidden, right shown.
/// - Overflow in middle: both shown.
/// - Overflow at right edge: left shown, right hidden.
pub fn calculate_tab_strip_geometry(
    total_tabs: usize,
    base_width: f32,
    scroll_offset_x: f32,
) -> TabStripGeometry {
    let content_width = total_tabs as f32 * TAB_WIDTH;
    let is_overflowing = content_width > base_width + SCROLL_EPSILON;

    if !is_overflowing {
        return TabStripGeometry {
            show_left_triangle: false,
            show_right_triangle: false,
            effective_viewport_width: base_width.max(0.0),
            max_scroll: 0.0,
        };
    }

    let available_without_right = (base_width - TAB_SCROLL_BUTTON_WIDTH).max(0.0);
    let max_scroll = (content_width - available_without_right).max(0.0);

    let can_scroll_left = scroll_offset_x > SCROLL_EPSILON;
    let can_scroll_right = scroll_offset_x < max_scroll - SCROLL_EPSILON;

    let effective_viewport_width = match (can_scroll_left, can_scroll_right) {
        (true, true) => (base_width - 2.0 * TAB_SCROLL_BUTTON_WIDTH).max(0.0),
        (true, false) | (false, true) => (base_width - TAB_SCROLL_BUTTON_WIDTH).max(0.0),
        (false, false) => base_width.max(0.0),
    };

    TabStripGeometry {
        show_left_triangle: can_scroll_left,
        show_right_triangle: can_scroll_right,
        effective_viewport_width,
        max_scroll,
    }
}

/// Computes the scroll offset target to make the tab at `index` visible.
/// Returns `Some(target_x)` if scrolling is necessary, or `None` if already visible.
pub fn tab_scroll_target_for_index(
    index: usize,
    total_tabs: usize,
    base_width: f32,
    current_scroll_x: f32,
) -> Option<f32> {
    if total_tabs == 0 || index >= total_tabs {
        return None;
    }
    let geometry = calculate_tab_strip_geometry(total_tabs, base_width, current_scroll_x);
    if !geometry.is_overflowing() {
        if current_scroll_x.abs() > SCROLL_EPSILON {
            return Some(0.0);
        }
        return None;
    }

    let tab_start = index as f32 * TAB_WIDTH;
    let tab_end = tab_start + TAB_WIDTH;
    let view_start = current_scroll_x;
    let view_end = current_scroll_x + geometry.effective_viewport_width;

    if tab_start < view_start {
        let target_x = tab_start.clamp(0.0, geometry.max_scroll);
        if (target_x - current_scroll_x).abs() > SCROLL_EPSILON {
            Some(target_x)
        } else {
            None
        }
    } else if tab_end > view_end {
        let target_viewport_width = if index == total_tabs - 1 {
            (base_width - TAB_SCROLL_BUTTON_WIDTH).max(0.0)
        } else {
            (base_width - 2.0 * TAB_SCROLL_BUTTON_WIDTH).max(0.0)
        };
        let target_x = if index == total_tabs - 1 {
            geometry.max_scroll
        } else if TAB_WIDTH > target_viewport_width {
            tab_start.clamp(0.0, geometry.max_scroll)
        } else {
            (tab_end - target_viewport_width).clamp(0.0, geometry.max_scroll)
        };
        if (target_x - current_scroll_x).abs() > SCROLL_EPSILON {
            Some(target_x)
        } else {
            None
        }
    } else if current_scroll_x > geometry.max_scroll + SCROLL_EPSILON {
        Some(geometry.max_scroll)
    } else {
        None
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
        let keys: std::collections::HashSet<_> = ActivityItem::TOP_ITEMS
            .into_iter()
            .flat_map(|item| {
                let (title, placeholder) = side_bar_text(item);
                [activity_label(item), title, placeholder]
            })
            .collect();

        assert_eq!(keys.len(), ActivityItem::TOP_ITEMS.len() * 3);
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
                terminal_font_family: None,
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
    }

    #[test]
    fn settings_gear_opens_the_tab_and_leaves_the_side_bar_alone() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::ActivitySelected(ActivityItem::Git));
        app.update(Message::SashDragged(ACTIVITY_BAR_WIDTH + 300.0));
        let width = app.workbench.side_bar_width();

        app.update(Message::OpenSettingsTab);

        assert_eq!(app.workbench.active_activity(), ActivityItem::Git);
        assert_eq!(app.workbench.side_bar_width(), width);
        assert!(settings_tab_of(&app).is_some());
    }

    #[test]
    fn settings_gear_hover_is_tracked_separately_from_activities() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::ActivityHovered(ActivityItem::Git));
        app.update(Message::SettingsGearHovered(true));
        assert!(app.settings_gear_hovered);
        assert_eq!(app.hovered_activity, Some(ActivityItem::Git));
        app.update(Message::SettingsGearHovered(false));
        assert!(!app.settings_gear_hovered);
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

    fn active_tab(app: &Kegon) -> TabId {
        app.workbench.tabs().active().unwrap().id
    }

    /// Adds a second terminal tab with its own session, tagging events the way production does.
    fn add_terminal_tab(app: &mut Kegon) -> TabId {
        let previous = active_tab(app);
        let tab = app.open_terminal_tab().expect("terminal tab");
        // `open_terminal_tab` activates the new tab; callers want it in the background.
        app.workbench.select_tab(previous);
        tab
    }

    fn tab_order(app: &Kegon) -> Vec<TabId> {
        app.workbench.tabs().iter().map(|tab| tab.id).collect()
    }

    /// Tabs and sessions must always correspond one to one.
    fn assert_tabs_and_terminals_agree(app: &Kegon) {
        let tabs = tab_order(app);
        assert_eq!(app.terminals.len(), tabs.len());
        assert!(tabs.iter().all(|id| app.terminals.contains(*id)));
        if let Some(active) = app.workbench.tabs().active() {
            assert!(tabs.contains(&active.id));
        } else {
            assert!(tabs.is_empty());
        }
    }

    fn report_cwd(app: &mut Kegon, tab: TabId, path: &std::path::Path) {
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::WorkingDirectory(path.to_path_buf()),
        ));
    }

    /// A real, uniquely named directory with a space and Japanese text in its name.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kegon test 日本語 {tag} {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unreported_cwd_means_default_directory() {
        let mut app = new_test_app(Locale::EnUs);
        assert!(app.inheritable_cwd().is_none());

        let mut seen = None;
        app.open_terminal_tab_with(|tab, cwd| {
            seen = Some(cwd);
            TerminalSession::spawn(80, 24, 10, 20, tagged_sink(&app_tx_for_test(), tab))
        });
        assert_eq!(seen, Some(None));
    }

    fn app_tx_for_test() -> UnboundedSender<TabTerminalEvent> {
        unbounded().0
    }

    #[test]
    fn reported_cwd_is_passed_to_the_new_session() {
        let dir = temp_dir("inherit");
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        report_cwd(&mut app, first, &dir);

        let mut seen = None;
        let tx = app.terminal_event_tx.clone();
        let opened = app.open_terminal_tab_with(|tab, cwd| {
            seen = Some(cwd.clone());
            TerminalSession::spawn_in(80, 24, 10, 20, tagged_sink(&tx, tab), cwd)
        });

        assert!(opened.is_some());
        assert_eq!(seen, Some(Some(dir.clone())));
        // The source terminal's own record is untouched.
        assert_eq!(
            app.terminals.get(first).unwrap().reported_cwd(),
            Some(dir.as_path())
        );
        // The new tab has not reported anything yet, so it is not assumed to be in `dir`.
        assert!(app.inheritable_cwd().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deleted_directories_are_not_inherited() {
        let dir = temp_dir("deleted");
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        report_cwd(&mut app, first, &dir);
        assert_eq!(app.inheritable_cwd(), Some(dir.clone()));

        std::fs::remove_dir_all(&dir).unwrap();
        assert!(app.inheritable_cwd().is_none());

        // A file is not a directory either.
        let file = std::env::temp_dir().join(format!("kegon test file {}", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        report_cwd(&mut app, first, &file);
        assert!(app.inheritable_cwd().is_none());
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn each_tab_keeps_its_own_cwd_and_background_reports_are_kept() {
        let (dir_a, dir_b) = (temp_dir("a"), temp_dir("b"));
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);

        report_cwd(&mut app, second, &dir_b); // background tab
        assert!(app.inheritable_cwd().is_none()); // the active tab has reported nothing
        report_cwd(&mut app, first, &dir_a);
        assert_eq!(app.inheritable_cwd(), Some(dir_a.clone()));

        app.update(Message::TabSelected(second));
        assert_eq!(app.inheritable_cwd(), Some(dir_b.clone()));
        assert_eq!(
            app.terminals.get(first).unwrap().reported_cwd(),
            Some(dir_a.as_path())
        );
        let _ = std::fs::remove_dir_all(dir_a);
        let _ = std::fs::remove_dir_all(dir_b);
    }

    #[test]
    fn cwd_reports_from_closed_tabs_are_ignored() {
        let dir = temp_dir("late");
        let mut app = new_test_app(Locale::EnUs);
        let second = add_terminal_tab(&mut app);
        app.update(Message::TabClosed(second));

        report_cwd(&mut app, second, &dir);

        assert_eq!(app.terminals.len(), 1);
        assert!(app.inheritable_cwd().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_spawn_failure_after_cwd_inheritance_leaves_no_tab() {
        let dir = temp_dir("fail");
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        report_cwd(&mut app, first, &dir);

        assert!(
            app.open_terminal_tab_with(|_, _| Err(String::from("nope")))
                .is_none()
        );
        assert_eq!(tab_order(&app), vec![first]);
        assert_tabs_and_terminals_agree(&app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn new_terminal_tab_is_appended_activated_and_focused() {
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        app.terminal_focused = false;
        app.preedit_text = Some(String::from("x"));

        app.update(Message::NewTerminalTab);

        let order = tab_order(&app);
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], first);
        assert_eq!(active_tab(&app), order[1]);
        assert!(app.terminal_focused);
        assert!(app.preedit_text.is_none());
        assert_tabs_and_terminals_agree(&app);
    }

    #[test]
    fn failed_spawn_leaves_no_tab_and_keeps_the_active_one() {
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);

        let opened = app.open_terminal_tab_with(|_, _| Err(String::from("spawn failed")));

        assert!(opened.is_none());
        assert_eq!(tab_order(&app), vec![first]);
        assert_eq!(active_tab(&app), first);
        assert_tabs_and_terminals_agree(&app);
    }

    #[test]
    fn closing_an_inactive_tab_keeps_the_active_tab() {
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);

        app.update(Message::TabClosed(second));

        assert_eq!(tab_order(&app), vec![first]);
        assert_eq!(active_tab(&app), first);
        assert!(!app.exit_requested);
        assert_tabs_and_terminals_agree(&app);
    }

    #[test]
    fn closing_the_active_tab_selects_right_then_left_neighbour() {
        let mut app = new_test_app(Locale::EnUs);
        let a = active_tab(&app);
        let b = add_terminal_tab(&mut app);
        let c = add_terminal_tab(&mut app);

        app.update(Message::TabClosed(a));
        assert_eq!(active_tab(&app), b);

        app.update(Message::TabSelected(c));
        app.update(Message::TabClosed(c));
        assert_eq!(active_tab(&app), b);
        assert!(app.terminal_focused);
        assert_tabs_and_terminals_agree(&app);
    }

    fn settings_tab_of(app: &Kegon) -> Option<TabId> {
        app.workbench.tabs().find_by_kind(TabKind::Settings)
    }

    #[test]
    fn settings_opens_as_a_tab_without_a_terminal_and_only_once() {
        let mut app = new_test_app(Locale::EnUs);
        let terminal = active_tab(&app);

        app.update(Message::OpenSettingsTab);
        let settings = settings_tab_of(&app).expect("settings tab");

        assert_eq!(tab_order(&app), [terminal, settings]);
        assert_eq!(active_tab(&app), settings);
        assert_eq!(
            app.terminals.len(),
            1,
            "settings must not create a PTY session"
        );
        assert!(!app.terminals.contains(settings));
        assert!(app.active_session().is_none());
        assert!(!app.terminal_focused);
        assert_eq!(app.tab_title(settings), "Settings");

        // Second open re-selects the existing tab, from wherever the user is.
        app.update(Message::TabSelected(terminal));
        assert!(app.terminal_focused);
        app.update(Message::OpenSettingsTab);
        assert_eq!(tab_order(&app), [terminal, settings]);
        assert_eq!(active_tab(&app), settings);
        assert_eq!(app.terminals.len(), 1);
    }

    #[test]
    fn settings_title_is_localized() {
        let mut app = new_test_app(Locale::JaJp);
        app.update(Message::OpenSettingsTab);
        let settings = settings_tab_of(&app).unwrap();
        assert_eq!(app.tab_title(settings), "設定");
    }

    #[test]
    fn switching_between_terminal_and_settings_follows_focus_and_closing_settings_keeps_terminals()
    {
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);
        app.update(Message::OpenSettingsTab);
        let settings = settings_tab_of(&app).unwrap();

        app.update(Message::TabSelected(second));
        assert!(app.terminal_focused);
        assert!(app.active_session().is_some());
        app.update(Message::TabSelected(settings));
        assert!(!app.terminal_focused);
        app.update(Message::TabSelected(first));
        assert!(app.terminal_focused);

        // Closing the active Settings tab selects its left neighbour; both terminals survive.
        app.update(Message::TabSelected(settings));
        app.update(Message::TabClosed(settings));
        assert!(!app.exit_requested);
        assert_eq!(tab_order(&app), [first, second]);
        assert_eq!(active_tab(&app), second);
        assert!(app.terminal_focused);
        assert_tabs_and_terminals_agree(&app);
    }

    #[test]
    fn settings_can_remain_after_the_last_terminal_and_closing_it_exits() {
        let mut app = new_test_app(Locale::EnUs);
        let terminal = active_tab(&app);
        app.update(Message::OpenSettingsTab);
        let settings = settings_tab_of(&app).unwrap();

        app.update(Message::TabClosed(terminal));
        assert!(!app.exit_requested, "Settings alone keeps the app running");
        assert_eq!(app.terminals.len(), 0);
        assert_eq!(tab_order(&app), [settings]);
        assert_eq!(active_tab(&app), settings);
        assert!(!app.terminal_focused);

        // "+" still adds a terminal while only Settings is open.
        app.update(Message::NewTerminalTab);
        assert_eq!(app.terminals.len(), 1);
        let new_terminal = active_tab(&app);
        assert_eq!(tab_order(&app), [settings, new_terminal]);
        assert!(app.terminal_focused);

        app.update(Message::TabClosed(new_terminal));
        assert!(!app.exit_requested);
        app.update(Message::TabClosed(settings));
        assert!(
            app.exit_requested,
            "the last tab of any kind closes the app"
        );
        assert!(app.workbench.tabs().is_empty());
    }

    #[test]
    fn closing_the_last_tab_requests_exit() {
        let mut app = new_test_app(Locale::EnUs);
        let only = active_tab(&app);

        app.update(Message::TabClosed(only));

        assert!(app.exit_requested);
        assert!(app.workbench.tabs().is_empty());
        assert_eq!(app.terminals.len(), 0);
        // The state stays usable while the exit is pending.
        assert!(app.active_session().is_none());
        assert_eq!(app.title(), "Kegon");
    }

    #[test]
    fn closing_an_unknown_tab_changes_nothing() {
        let mut app = new_test_app(Locale::EnUs);
        let second = add_terminal_tab(&mut app);
        app.update(Message::TabClosed(second));
        let before = tab_order(&app);

        app.update(Message::TabClosed(second));

        assert_eq!(tab_order(&app), before);
        assert!(!app.exit_requested);
        assert_tabs_and_terminals_agree(&app);
    }

    #[test]
    fn events_from_a_closed_tab_do_not_reach_other_tabs() {
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);
        app.update(Message::TabClosed(second));

        app.update(Message::TerminalEventReceived(
            second,
            TerminalEvent::Title(String::from("late")),
        ));

        assert_eq!(
            app.tab_title(first),
            app.localizer.text(MessageKey::TerminalTabDefaultTitle)
        );
        assert_eq!(app.title(), "Kegon");
    }

    #[test]
    fn tab_ids_are_not_reused_after_closing() {
        let mut app = new_test_app(Locale::EnUs);
        let second = add_terminal_tab(&mut app);
        app.update(Message::TabClosed(second));
        let third = add_terminal_tab(&mut app);
        assert_ne!(second, third);
    }

    #[test]
    fn repeated_open_select_close_keeps_tabs_and_sessions_in_step() {
        let mut app = new_test_app(Locale::EnUs);
        for round in 0..6usize {
            app.update(Message::NewTerminalTab);
            assert_tabs_and_terminals_agree(&app);
            let order = tab_order(&app);
            app.update(Message::TabSelected(order[round % order.len()]));
            if round % 2 == 1 {
                app.update(Message::TabClosed(order[(round / 2) % order.len()]));
            }
            assert_tabs_and_terminals_agree(&app);
        }
        while let Some(id) = tab_order(&app).first().copied() {
            app.update(Message::TabClosed(id));
            assert_tabs_and_terminals_agree(&app);
        }
        assert!(app.exit_requested);
    }

    /// Drives the tab/session state with a long pseudo-random mix of every operation (open, failed
    /// open, select, close, unknown ids, events for live, closed and never-issued tabs) and checks
    /// the invariants after each step.
    #[test]
    fn random_operation_sequences_keep_tabs_and_sessions_consistent() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        let mut app = new_test_app(Locale::EnUs);
        let mut ever_issued = tab_order(&app);
        let mut closed: Vec<TabId> = Vec::new();

        for step in 0..120 {
            let live = tab_order(&app);
            match next(8) {
                0 | 1 => {
                    if live.len() < 6 {
                        app.update(Message::NewTerminalTab);
                        let tab = active_tab(&app);
                        assert!(!ever_issued.contains(&tab), "tab id reused at step {step}");
                        ever_issued.push(tab);
                    }
                }
                2 => {
                    let before = tab_order(&app);
                    let opened = app.open_terminal_tab_with(|_, _| Err(String::from("no")));
                    assert!(opened.is_none());
                    assert_eq!(tab_order(&app), before);
                }
                3 | 4 => {
                    if let Some(&tab) = live.get(next(live.len().max(1))) {
                        app.update(Message::TabSelected(tab));
                        assert_eq!(active_tab(&app), tab);
                    }
                }
                5 => {
                    // Keep at least one tab: closing the last one is the exit path.
                    if live.len() > 1 {
                        let tab = live[next(live.len())];
                        let was_active = app.workbench.tabs().is_active(tab);
                        let others: Vec<TabId> =
                            live.iter().copied().filter(|t| *t != tab).collect();
                        let active_before = active_tab(&app);
                        app.update(Message::TabClosed(tab));
                        closed.push(tab);
                        if !was_active {
                            assert_eq!(active_tab(&app), active_before);
                        }
                        assert!(others.contains(&active_tab(&app)));
                    }
                }
                6 => {
                    // Events for closed tabs must change nothing visible.
                    if let Some(&tab) = closed.get(next(closed.len().max(1))) {
                        let title_before = app.title();
                        app.update(Message::TerminalEventReceived(
                            tab,
                            TerminalEvent::Title(String::from("ghost")),
                        ));
                        app.update(Message::TerminalEventReceived(
                            tab,
                            TerminalEvent::WorkingDirectory(std::env::temp_dir()),
                        ));
                        app.update(Message::TabClosed(tab));
                        app.update(Message::TabSelected(tab));
                        assert_eq!(app.title(), title_before);
                        assert!(app.inheritable_cwd().is_none() || app.active_session().is_some());
                    }
                }
                _ => {
                    if let Some(&tab) = live.get(next(live.len().max(1))) {
                        app.update(Message::TerminalEventReceived(
                            tab,
                            TerminalEvent::Title(format!("t{step}")),
                        ));
                        assert_eq!(app.tab_title(tab), format!("t{step}"));
                    }
                }
            }
            assert_tabs_and_terminals_agree(&app);
            assert!(!app.exit_requested);
            assert!(
                app.active_session().is_some(),
                "active tab lost its session at step {step}"
            );
            for gone in &closed {
                assert!(
                    !app.terminals.contains(*gone) && app.workbench.tabs().get(*gone).is_none()
                );
            }
        }
    }

    /// (handle count of this process, number of its direct child processes), read from the OS.
    #[cfg(windows)]
    fn process_stats() -> (usize, usize) {
        let pid = std::process::id();
        let script = format!(
            "$p = Get-Process -Id {pid}; $c = (Get-CimInstance Win32_Process -Filter 'ParentProcessId={pid}' | Measure-Object).Count; \"$($p.HandleCount) $c\""
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let mut numbers = text.split_whitespace().map(|n| n.parse::<usize>().unwrap());
        (numbers.next().unwrap(), numbers.next().unwrap())
    }

    /// Opens and closes terminal tabs against real shells and ConPTYs, comparing the process's
    /// handle count and child processes before and after. Prints every measurement.
    ///
    /// Run with `cargo test repeated_tab_open_close -- --ignored --nocapture`. Child processes
    /// must return to the baseline exactly. Handles must too, up to a small fixed slack for OS
    /// noise (the measuring PowerShell, lazily created runtime handles); the patched
    /// `alacritty_terminal` closes every handle it opens per pseudoconsole, so any growth that
    /// scales with the number of tabs fails: 40 more cycles must not add more than the slack.
    #[cfg(windows)]
    #[test]
    #[ignore = "spawns many real shells; measures OS process and handle counts"]
    fn repeated_tab_open_close_does_not_accumulate_resources() {
        // Fixed allowance for OS noise; deliberately not multiplied by the number of cycles.
        const HANDLE_SLACK: usize = 8;
        let settle = || std::thread::sleep(std::time::Duration::from_secs(3));
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);

        // Warm up so one-time allocations are not counted as growth.
        for _ in 0..3 {
            app.update(Message::NewTerminalTab);
            let tab = active_tab(&app);
            app.update(Message::TabClosed(tab));
        }
        settle();
        // The measuring PowerShell is itself a child, and shells from earlier tests may still be
        // winding down, so take the lowest of a few readings as the baseline.
        let readings: Vec<_> = (0..3)
            .map(|_| {
                std::thread::sleep(std::time::Duration::from_secs(2));
                process_stats()
            })
            .collect();
        let base_handles = readings.iter().map(|r| r.0).min().unwrap();
        let base_children = readings.iter().map(|r| r.1).min().unwrap();
        // Later readings wait (bounded) for the child count to come back down before judging it.
        let settled_stats = || {
            let mut stats = process_stats();
            for _ in 0..10 {
                if stats.1 <= base_children {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
                stats = process_stats();
            }
            stats
        };
        println!("baseline          handles={base_handles} children={base_children}");

        let mut trend = Vec::new();
        let mut slowest_close = std::time::Duration::ZERO;
        for (label, cycles) in [("10 cycles", 10), ("50 cycles", 50)] {
            for _ in 0..cycles {
                app.update(Message::NewTerminalTab);
                std::thread::sleep(std::time::Duration::from_millis(150));
                let tab = active_tab(&app);
                let started = std::time::Instant::now();
                app.update(Message::TabClosed(tab));
                slowest_close = slowest_close.max(started.elapsed());
                assert_eq!(tab_order(&app), vec![first]);
            }
            settle();
            let (handles, children) = settled_stats();
            println!("after {label:<9} handles={handles} children={children}");
            trend.push((handles, children));
        }

        // Several tabs held at once, closed in a different order than opened.
        let mut held = Vec::new();
        for _ in 0..6 {
            app.update(Message::NewTerminalTab);
            held.push(active_tab(&app));
        }
        settle();
        let (peak_handles, peak_children) = process_stats();
        println!("6 extra tabs open handles={peak_handles} children={peak_children}");
        for index in [3, 0, 5, 1, 4, 2] {
            app.update(Message::TabClosed(held[index]));
        }
        settle();
        let (end_handles, end_children) = settled_stats();
        println!("after closing all handles={end_handles} children={end_children}");
        assert_tabs_and_terminals_agree(&app);

        assert!(
            peak_children > base_children,
            "extra tabs should have started shells"
        );
        assert_eq!(
            end_children, base_children,
            "child processes were left behind"
        );
        println!("slowest tab close on the UI thread: {slowest_close:?}");
        assert!(
            slowest_close < std::time::Duration::from_millis(250),
            "closing a tab blocked the UI thread for {slowest_close:?}"
        );
        for (handles, children) in &trend {
            assert_eq!(*children, base_children, "child processes were left behind");
            assert!(
                *handles <= base_handles + HANDLE_SLACK,
                "handles grew: {base_handles} -> {handles}"
            );
        }
        // The slope check: 40 further open/close cycles between the two readings.
        assert!(
            trend[1].0 <= trend[0].0 + HANDLE_SLACK,
            "handles grow with the number of tabs: {} after 10 cycles, {} after 50",
            trend[0].0,
            trend[1].0
        );
        assert!(
            end_handles <= base_handles + HANDLE_SLACK,
            "handles grew: {base_handles} -> {end_handles}"
        );
    }

    #[test]
    fn an_exited_terminal_can_be_switched_away_from_and_closed() {
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);
        app.terminals
            .get(second)
            .unwrap()
            .session()
            .write_input(b"exit\r\n".to_vec());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !app.terminals.get(second).unwrap().session().is_exited() {
            assert!(
                std::time::Instant::now() < deadline,
                "shell did not exit on `exit`"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        // The exited tab stays until closed and can be selected.
        assert_tabs_and_terminals_agree(&app);
        app.update(Message::TabSelected(second));
        assert!(app.active_session().unwrap().is_exited());
        app.update(Message::TabSelected(first));

        app.update(Message::TabClosed(second));
        assert_eq!(tab_order(&app), vec![first]);
        assert_tabs_and_terminals_agree(&app);
    }

    #[test]
    fn startup_registers_a_session_for_the_active_terminal_tab() {
        let app = new_test_app(Locale::EnUs);
        let tab = active_tab(&app);
        assert_eq!(app.terminals.len(), 1);
        assert!(app.terminals.contains(tab));
        assert!(app.active_session().is_some());
    }

    #[test]
    fn background_terminal_events_update_only_their_own_tab() {
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);

        app.update(Message::TerminalEventReceived(
            second,
            TerminalEvent::Title(String::from("background")),
        ));
        assert_eq!(app.title(), "Kegon");
        assert_eq!(app.tab_title(second), "background");
        assert_eq!(
            app.tab_title(first),
            app.localizer.text(MessageKey::TerminalTabDefaultTitle)
        );

        app.update(Message::TabSelected(second));
        assert_eq!(app.title(), "background - Kegon");
    }

    #[test]
    fn selecting_a_tab_switches_the_input_target() {
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);
        app.active_session().unwrap().start_selection(0, 0);
        app.active_session().unwrap().update_selection(3, 0);
        assert!(app.terminals.get(first).unwrap().session().has_selection());

        app.preedit_text = Some(String::from("あ"));
        app.update(Message::TabSelected(second));
        assert_eq!(active_tab(&app), second);
        assert!(app.preedit_text.is_none());
        assert!(!app.active_session().unwrap().has_selection());
        assert!(app.terminals.get(first).unwrap().session().has_selection());
    }

    #[test]
    fn late_events_from_removed_terminals_are_ignored() {
        let mut app = new_test_app(Locale::EnUs);
        let second = add_terminal_tab(&mut app);
        app.terminals.remove(second);

        app.update(Message::TerminalEventReceived(
            second,
            TerminalEvent::Title(String::from("late")),
        ));
        assert_eq!(app.title(), "Kegon");
    }

    #[test]
    fn viewport_resize_reaches_every_session() {
        let mut app = new_test_app(Locale::EnUs);
        let second = add_terminal_tab(&mut app);
        app.window_size = Size::new(900.0, 500.0);
        app.resize_terminal_view();

        use alacritty_terminal::grid::Dimensions;
        let size = |app: &Kegon, tab| {
            let term = app.terminals.get(tab).unwrap().session().term().lock();
            (term.columns(), term.screen_lines())
        };
        assert_eq!(size(&app, active_tab(&app)), size(&app, second));
        assert_ne!(size(&app, second), (80, 24));
    }

    #[test]
    fn settings_changes_reach_every_terminal_and_keep_all_tabs() {
        use alacritty_terminal::grid::Dimensions;

        let mut settings = ApplicationSettings::default();
        settings.appearance.terminal_font_family = Some(String::from("Consolas"));
        let mut app = with_test_settings_path(Kegon::new(Locale::EnUs, settings, None, None, None));
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);
        app.update(Message::OpenSettingsTab);
        let settings_tab = settings_tab_of(&app).unwrap();
        app.window_size = Size::new(900.0, 500.0);

        app.update(Message::SettingsResetTerminalFontClicked);
        app.update(Message::SettingsThemeChanged(ThemeId::NightDark));
        app.update(Message::SettingsLocaleChanged(LocalePreference::JaJp));

        // No session was created, closed, or restarted by changing settings.
        assert_eq!(tab_order(&app), [first, second, settings_tab]);
        assert_eq!(app.terminals.len(), 2);
        assert!(app.terminals.contains(first) && app.terminals.contains(second));
        assert_eq!(active_tab(&app), settings_tab);
        let size = |app: &Kegon, tab| {
            let term = app.terminals.get(tab).unwrap().session().term().lock();
            (term.columns(), term.screen_lines())
        };
        assert_eq!(size(&app, first), size(&app, second));
        assert_eq!(app.tab_title(settings_tab), "設定");

        // The page renders for both locales at a narrow window.
        app.window_size = Size::new(360.0, 300.0);
        let _ = app.view();
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
    fn terminal_title_events_drive_tab_and_window_titles() {
        let mut app = new_test_app(Locale::EnUs);
        let default_label = app.localizer.text(MessageKey::TerminalTabDefaultTitle);
        assert_eq!(app.title(), "Kegon");
        assert_eq!(app.active_tab_title(), default_label);

        app.update(Message::TerminalEventReceived(
            active_tab(&app),
            TerminalEvent::Title(String::from("Kegon title test")),
        ));
        assert_eq!(app.title(), "Kegon title test - Kegon");
        assert_eq!(app.active_tab_title(), "Kegon title test");

        app.update(Message::TerminalEventReceived(
            active_tab(&app),
            TerminalEvent::ResetTitle,
        ));
        assert_eq!(app.title(), "Kegon");
        assert_eq!(app.active_tab_title(), default_label);
    }

    #[test]
    fn last_terminal_title_event_wins() {
        let mut app = new_test_app(Locale::EnUs);
        for event in [
            TerminalEvent::Title(String::from("A")),
            TerminalEvent::Title(String::from("B")),
            TerminalEvent::ResetTitle,
            TerminalEvent::Title(String::from("C")),
        ] {
            app.update(Message::TerminalEventReceived(active_tab(&app), event));
        }
        assert_eq!(app.title(), "C - Kegon");
    }

    #[test]
    fn blank_terminal_title_falls_back_and_exit_keeps_title() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::TerminalEventReceived(
            active_tab(&app),
            TerminalEvent::Title(String::from("pwsh")),
        ));
        app.update(Message::TerminalEventReceived(
            active_tab(&app),
            TerminalEvent::ChildExit(Some(0)),
        ));
        assert_eq!(app.title(), "pwsh - Kegon");
        assert_eq!(app.active_tab_title(), "pwsh");

        app.update(Message::TerminalEventReceived(
            active_tab(&app),
            TerminalEvent::Title(String::from(" \x07 ")),
        ));
        assert_eq!(app.title(), "Kegon");
        assert_eq!(
            app.active_tab_title(),
            app.localizer.text(MessageKey::TerminalTabDefaultTitle)
        );
        assert_eq!(app.active_terminal().and_then(TerminalEntry::title), None);
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

    #[test]
    fn terminal_font_settings_and_resolution() {
        let mut settings = ApplicationSettings::default();
        settings.appearance.terminal_font_family = Some(String::from("Consolas"));

        let app = Kegon::new(Locale::EnUs, settings, None, None, None);
        assert!(matches!(
            app.terminal_font_config.status,
            TerminalFontStatus::Active(_) | TerminalFontStatus::NotInstalled(_)
        ));
    }

    #[test]
    fn terminal_font_choose_open_picker_and_reset() {
        let mut settings = ApplicationSettings::default();
        settings.appearance.terminal_font_family = Some(String::from("Consolas"));

        let mut app = with_test_settings_path(Kegon::new(Locale::EnUs, settings, None, None, None));
        assert_eq!(
            app.settings.appearance.terminal_font_family,
            Some("Consolas".into())
        );

        // Choose clicked opens font picker in Terminal mode
        app.update(Message::SettingsChooseTerminalFontClicked);
        assert!(app.is_modal_open());
        assert_eq!(
            app.font_picker.as_ref().unwrap().mode,
            FontPickerMode::Terminal
        );

        app.close_font_picker(FontPickerResult::Cancel);

        // Reset clicked sets terminal_font_family to None
        app.update(Message::SettingsResetTerminalFontClicked);
        assert_eq!(app.settings.appearance.terminal_font_family, None);
        assert_eq!(app.terminal_font_config.status, TerminalFontStatus::Default);
    }

    #[test]
    fn terminal_cell_point_calculation_and_clamping() {
        let metrics = TerminalCellMetrics::new(10.0, 20.0, 13.0);
        let window_size = Size::new(1000.0, 600.0);
        let side_bar_w = 200.0;

        let pt = Point::new(272.0, 75.0);
        let (col, line) = calculate_terminal_cell_point(pt, window_size, side_bar_w, &metrics);
        assert_eq!((col, line), (2, 2));

        let pt_neg = Point::new(-100.0, -50.0);
        let (col_neg, line_neg) =
            calculate_terminal_cell_point(pt_neg, window_size, side_bar_w, &metrics);
        assert_eq!((col_neg, line_neg), (0, 0));

        let pt_far = Point::new(5000.0, 5000.0);
        let (col_far, line_far) =
            calculate_terminal_cell_point(pt_far, window_size, side_bar_w, &metrics);
        let (max_cols, max_rows) = metrics.grid_size(1000.0 - 252.0, 600.0 - 35.0);
        assert_eq!(
            (col_far, line_far),
            ((max_cols - 1) as usize, (max_rows - 1) as usize)
        );
    }

    #[test]
    fn ime_preedit_and_commit_flow() {
        use iced::advanced::input_method::Event as ImeEvent;

        let mut app = new_test_app(Locale::EnUs);
        app.terminal_focused = true;

        assert_eq!(app.preedit_text, None);

        // Preedit event updates preedit_text
        app.update(Message::IcedEventReceived(Event::InputMethod(
            ImeEvent::Preedit(String::from("にほんご"), None),
        )));
        assert_eq!(app.preedit_text.as_deref(), Some("にほんご"));

        // Commit event clears preedit_text
        app.update(Message::IcedEventReceived(Event::InputMethod(
            ImeEvent::Commit(String::from("日本語")),
        )));
        assert_eq!(app.preedit_text, None);

        // Closed event clears preedit_text
        app.update(Message::IcedEventReceived(Event::InputMethod(
            ImeEvent::Preedit(String::from("かな"), None),
        )));
        assert_eq!(app.preedit_text.as_deref(), Some("かな"));

        app.update(Message::IcedEventReceived(Event::InputMethod(
            ImeEvent::Closed,
        )));
        assert_eq!(app.preedit_text, None);
    }

    #[test]
    fn terminal_focus_toggles_with_modal_and_clicks() {
        let mut app = new_test_app(Locale::EnUs);
        app.terminal_focused = true;

        // Terminal click keeps focus when no modal
        let term_x = ACTIVITY_BAR_WIDTH + SASH_WIDTH + app.workbench.side_bar_width();
        let term_y = TAB_STRIP_HEIGHT;
        app.cursor_position = Point::new(term_x + 5.0, term_y + 5.0);
        app.update(Message::IcedEventReceived(Event::Mouse(
            mouse::Event::ButtonPressed(mouse::Button::Left),
        )));
        assert!(app.terminal_focused);

        // Open modal
        let dialog = ConfirmationDialog::builder(
            DialogKind::Question,
            MessageKey::DialogSmokeQuestionTitle,
            MessageKey::DialogSmokeQuestionMessage,
        )
        .restore_focus(FocusTarget::Terminal)
        .build();

        app.open_confirmation_dialog(dialog);
        assert!(app.is_modal_open());

        // Close modal restores terminal focus
        app.close_modal(ConfirmationResult::Primary);
        assert!(!app.is_modal_open());
        assert!(app.terminal_focused);
    }

    #[test]
    fn mouse_drag_selection_lifecycle_and_persistence() {
        let mut app = new_test_app(Locale::EnUs);

        {
            let session = app.active_session().unwrap();
            let mut term = session.term().lock();
            use alacritty_terminal::vte::ansi::Processor;
            let mut parser: Processor = Processor::new();
            parser.advance(&mut *term, b"Hello World\r\n");
        }

        assert!(!app.active_session().unwrap().has_selection());

        let term_x = ACTIVITY_BAR_WIDTH + SASH_WIDTH + app.workbench.side_bar_width();
        let term_y = TAB_STRIP_HEIGHT;

        // Press mouse down at (0,0) cell position
        app.cursor_position = Point::new(term_x + 5.0, term_y + 5.0);
        app.update(Message::IcedEventReceived(Event::Mouse(
            mouse::Event::ButtonPressed(mouse::Button::Left),
        )));
        assert!(app.mouse_dragging_selection);
        assert!(!app.active_session().unwrap().has_selection()); // Single click is not copyable

        // Move mouse to select (0,0)->(4,0) ("Hello")
        let cell_w = app.terminal_font_config.metrics.cell_width;
        app.update(Message::IcedEventReceived(Event::Mouse(
            mouse::Event::CursorMoved {
                position: Point::new(term_x + cell_w * 4.5, term_y + 5.0),
            },
        )));
        assert!(app.active_session().unwrap().has_selection());
        assert_eq!(
            app.active_session().unwrap().copy_selection().as_deref(),
            Some("Hello")
        );

        // Release mouse button: selection persists!
        app.update(Message::IcedEventReceived(Event::Mouse(
            mouse::Event::ButtonReleased(mouse::Button::Left),
        )));
        assert!(!app.mouse_dragging_selection);
        assert!(app.active_session().unwrap().has_selection());
        assert_eq!(
            app.active_session().unwrap().copy_selection().as_deref(),
            Some("Hello")
        );
    }

    #[test]
    fn tab_geometry_fits_without_overflow_both_triangles_hidden() {
        // 2 tabs * 160px = 320px in base_width 500px -> no overflow
        let geom = calculate_tab_strip_geometry(2, 500.0, 0.0);
        assert!(!geom.show_left_triangle);
        assert!(!geom.show_right_triangle);
        assert!(!geom.is_overflowing());
        assert_eq!(geom.effective_viewport_width, 500.0);
        assert_eq!(geom.max_scroll, 0.0);
    }

    #[test]
    fn tab_geometry_overflow_left_edge_shows_right_triangle_only() {
        // 5 tabs * 160px = 800px in base_width 400px, scroll_offset_x = 0.0
        let geom = calculate_tab_strip_geometry(5, 400.0, 0.0);
        assert!(!geom.show_left_triangle);
        assert!(geom.show_right_triangle);
        assert!(geom.is_overflowing());
        // base_width - TAB_SCROLL_BUTTON_WIDTH = 400 - 24 = 376
        assert_eq!(geom.effective_viewport_width, 376.0);
        // content_width (800) - 376 = 424
        assert_eq!(geom.max_scroll, 424.0);
    }

    #[test]
    fn tab_geometry_overflow_middle_shows_both_triangles() {
        // 5 tabs in 400px base_width, scroll_offset_x = 160.0
        let geom = calculate_tab_strip_geometry(5, 400.0, 160.0);
        assert!(geom.show_left_triangle);
        assert!(geom.show_right_triangle);
        assert!(geom.is_overflowing());
        // base_width - 2 * TAB_SCROLL_BUTTON_WIDTH = 400 - 48 = 352
        assert_eq!(geom.effective_viewport_width, 352.0);
        assert_eq!(geom.max_scroll, 424.0);
    }

    #[test]
    fn tab_geometry_overflow_right_edge_shows_left_triangle_only() {
        // 5 tabs in 400px base_width, scroll_offset_x = 424.0 (max_scroll)
        let geom = calculate_tab_strip_geometry(5, 400.0, 424.0);
        assert!(geom.show_left_triangle);
        assert!(!geom.show_right_triangle);
        assert!(geom.is_overflowing());
        // base_width - TAB_SCROLL_BUTTON_WIDTH = 400 - 24 = 376
        assert_eq!(geom.effective_viewport_width, 376.0);
        assert_eq!(geom.max_scroll, 424.0);
    }

    #[test]
    fn tab_geometry_no_oscillation_at_boundaries() {
        // Test near left edge (0.0, 0.5, 1.0, 1.1)
        let g0 = calculate_tab_strip_geometry(5, 400.0, 0.0);
        assert!(!g0.show_left_triangle);
        assert!(g0.show_right_triangle);

        let g_sub = calculate_tab_strip_geometry(5, 400.0, 0.5);
        assert!(!g_sub.show_left_triangle);
        assert!(g_sub.show_right_triangle);

        let g_eps = calculate_tab_strip_geometry(5, 400.0, 1.0);
        assert!(!g_eps.show_left_triangle);
        assert!(g_eps.show_right_triangle);

        let g_just_past = calculate_tab_strip_geometry(5, 400.0, 1.1);
        assert!(g_just_past.show_left_triangle);
        assert!(g_just_past.show_right_triangle);

        // Test near right edge (max_scroll = 424.0)
        let g_before_max = calculate_tab_strip_geometry(5, 400.0, 422.9);
        assert!(g_before_max.show_left_triangle);
        assert!(g_before_max.show_right_triangle);

        let g_at_max_eps = calculate_tab_strip_geometry(5, 400.0, 423.0);
        assert!(g_at_max_eps.show_left_triangle);
        assert!(!g_at_max_eps.show_right_triangle);

        let g_at_max = calculate_tab_strip_geometry(5, 400.0, 424.0);
        assert!(g_at_max.show_left_triangle);
        assert!(!g_at_max.show_right_triangle);
    }

    #[test]
    fn tab_geometry_extreme_narrow_window() {
        // base_width = 10.0 or 0.0 should not panic or produce NaN/negatives
        let g_narrow = calculate_tab_strip_geometry(4, 10.0, 0.0);
        assert!(g_narrow.is_overflowing());
        assert!(g_narrow.effective_viewport_width >= 0.0);
        assert!(g_narrow.max_scroll >= 0.0);

        let g_zero = calculate_tab_strip_geometry(4, 0.0, 0.0);
        assert!(g_zero.is_overflowing());
        assert_eq!(g_zero.effective_viewport_width, 0.0);
        assert_eq!(g_zero.max_scroll, 640.0);
    }

    #[test]
    fn tab_scroll_target_follows_active_tab_on_add_select_and_resize() {
        let mut app = new_test_app(Locale::EnUs);
        app.window_size = Size::new(600.0, 600.0);

        // Initial state: 1 tab, fits easily
        assert_eq!(app.workbench.tabs().len(), 1);
        assert_eq!(app.tab_scroll_offset_x, 0.0);

        // Add 5 more tabs to total 6 tabs (6 * 160 = 960px > base_width ~ 290px)
        for _ in 0..5 {
            app.open_terminal_tab();
        }
        assert_eq!(app.workbench.tabs().len(), 6);
        // Newly added tab should be active and snapped to end
        let geom =
            calculate_tab_strip_geometry(6, app.tab_strip_base_width(), app.tab_scroll_offset_x);
        assert_eq!(app.tab_scroll_offset_x, geom.max_scroll);
        assert!(matches!(
            app.pending_tab_scroll_task,
            Some(TabScrollTask::SnapToEnd)
        ));

        // Select tab 0 (which is hidden far to the left)
        let tab0_id = app.workbench.tabs().iter().next().unwrap().id;
        app.update(Message::TabSelected(tab0_id));
        assert_eq!(app.tab_scroll_offset_x, 0.0);
        assert!(matches!(
            app.pending_tab_scroll_task,
            Some(TabScrollTask::ScrollTo(0.0))
        ));

        // Select tab 5 (which is hidden to the right)
        let tab5_id = app.workbench.tabs().iter().last().unwrap().id;
        app.update(Message::TabSelected(tab5_id));
        let geom =
            calculate_tab_strip_geometry(6, app.tab_strip_base_width(), app.tab_scroll_offset_x);
        assert_eq!(app.tab_scroll_offset_x, geom.max_scroll);

        // Window resize wider: scroll is clamped safely
        app.update(Message::IcedEventReceived(Event::Window(
            iced::window::Event::Resized(Size::new(1200.0, 600.0)),
        )));
        let new_base = app.tab_strip_base_width();
        let new_geom = calculate_tab_strip_geometry(6, new_base, app.tab_scroll_offset_x);
        assert!(app.tab_scroll_offset_x <= new_geom.max_scroll);

        // Window resize narrower: active tab (tab 5) remains followed
        app.update(Message::IcedEventReceived(Event::Window(
            iced::window::Event::Resized(Size::new(500.0, 500.0)),
        )));
        let narrow_geom =
            calculate_tab_strip_geometry(6, app.tab_strip_base_width(), app.tab_scroll_offset_x);
        assert_eq!(app.tab_scroll_offset_x, narrow_geom.max_scroll);
    }

    #[test]
    fn tab_strip_close_and_add_buttons_preserved_with_long_titles() {
        let mut app = new_test_app(Locale::EnUs);
        let active_id = app.workbench.tabs().active().unwrap().id;

        // Simulate a very long terminal title (200 characters)
        let long_title = "a".repeat(200);
        app.terminals
            .handle_event(active_id, TerminalEvent::Title(long_title.clone()));
        let rendered_title = app.tab_title(active_id);
        assert!(rendered_title.ends_with('…'));
        assert!(rendered_title.len() < long_title.len());

        // Close button width and tab width constants are strictly preserved
        assert_eq!(TAB_WIDTH, 160.0);
        assert_eq!(TAB_CLOSE_WIDTH, 24.0);
        assert_eq!(TAB_SCROLL_BUTTON_WIDTH, 24.0);

        // View renders without panicking
        let _ = app.view();
    }

    #[test]
    fn tab_navigation_messages_scroll_by_tab_width() {
        let mut app = new_test_app(Locale::EnUs);
        app.window_size = Size::new(600.0, 600.0);
        for _ in 0..5 {
            app.open_terminal_tab();
        }
        // Move to start
        app.tab_scroll_offset_x = 0.0;

        // Scroll right
        app.update(Message::ScrollTabsRight);
        assert_eq!(app.tab_scroll_offset_x, TAB_WIDTH);
        assert!(matches!(
            app.pending_tab_scroll_task,
            Some(TabScrollTask::ScrollTo(x)) if (x - TAB_WIDTH).abs() < 0.01
        ));

        // Scroll left
        app.update(Message::ScrollTabsLeft);
        assert_eq!(app.tab_scroll_offset_x, 0.0);
        assert!(matches!(
            app.pending_tab_scroll_task,
            Some(TabScrollTask::ScrollTo(0.0))
        ));
    }

    #[test]
    fn tab_wheel_scroll_audit_scope_and_touchpad_isolation() {
        let mut app = new_test_app(Locale::EnUs);
        app.window_size = Size::new(800.0, 600.0);
        for _ in 0..5 {
            app.open_terminal_tab();
        }
        app.tab_scroll_offset_x = 0.0;
        app.pending_tab_scroll_task = None;

        let term_x = ACTIVITY_BAR_WIDTH + SASH_WIDTH + app.workbench.side_bar_width();

        // 1. Wheel over Side Bar (x < term_x):
        // Does NOT scroll tab strip and does NOT scroll terminal
        app.cursor_position = Point::new(term_x - 10.0, 10.0);
        app.handle_iced_event(Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
        }));
        assert_eq!(app.tab_scroll_offset_x, 0.0);
        assert!(app.pending_tab_scroll_task.is_none());

        // 2. Pure vertical wheel over Tab Strip (x >= term_x, y < TAB_STRIP_HEIGHT):
        // Converts vertical wheel to horizontal tab scroll
        app.cursor_position = Point::new(term_x + 50.0, 10.0);
        app.handle_iced_event(Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
        }));
        assert_eq!(app.tab_scroll_offset_x, 60.0);
        assert!(matches!(
            app.pending_tab_scroll_task,
            Some(TabScrollTask::ScrollTo(x)) if (x - 60.0).abs() < 0.01
        ));
        app.pending_tab_scroll_task = None;

        // 3. Touchpad horizontal scroll over Tab Strip (x != 0):
        // Handled directly by iced's scrollable widget; handle_iced_event does NOT double-apply it
        app.cursor_position = Point::new(term_x + 50.0, 10.0);
        app.handle_iced_event(Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 1.0, y: 0.0 },
        }));
        // Scroll offset in app remains 60.0 (not double incremented)
        assert_eq!(app.tab_scroll_offset_x, 60.0);
        assert!(app.pending_tab_scroll_task.is_none());

        // 4. Wheel over Terminal Body (y >= TAB_STRIP_HEIGHT):
        // Does NOT scroll tab strip
        app.cursor_position = Point::new(term_x + 50.0, TAB_STRIP_HEIGHT + 50.0);
        app.handle_iced_event(Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
        }));
        assert_eq!(app.tab_scroll_offset_x, 60.0);
        assert!(app.pending_tab_scroll_task.is_none());
    }

    #[test]
    fn tab_strip_boundary_and_triangle_oscillation_audit() {
        let mut app = new_test_app(Locale::EnUs);
        app.window_size = Size::new(600.0, 600.0);
        for _ in 0..5 {
            app.open_terminal_tab();
        }
        let total_tabs = app.workbench.tabs().len(); // 6 tabs = 960px
        let content_width = total_tabs as f32 * TAB_WIDTH;
        let base_width = app.tab_strip_base_width();

        // 1. Left edge: offset = 0.0
        let g_left = calculate_tab_strip_geometry(total_tabs, base_width, 0.0);
        assert!(!g_left.show_left_triangle);
        assert!(g_left.show_right_triangle);
        assert_eq!(
            g_left.effective_viewport_width,
            base_width - TAB_SCROLL_BUTTON_WIDTH
        );

        // 2. Middle: offset = TAB_WIDTH
        let g_mid = calculate_tab_strip_geometry(total_tabs, base_width, TAB_WIDTH);
        assert!(g_mid.show_left_triangle);
        assert!(g_mid.show_right_triangle);
        assert_eq!(
            g_mid.effective_viewport_width,
            base_width - 2.0 * TAB_SCROLL_BUTTON_WIDTH
        );

        // 3. Right edge: offset = max_scroll
        let g_right = calculate_tab_strip_geometry(total_tabs, base_width, g_left.max_scroll);
        assert!(g_right.show_left_triangle);
        assert!(!g_right.show_right_triangle);
        assert_eq!(
            g_right.effective_viewport_width,
            base_width - TAB_SCROLL_BUTTON_WIDTH
        );
        // Verify perfect coordinate alignment at right edge:
        assert_eq!(
            g_left.max_scroll + g_right.effective_viewport_width,
            content_width
        );

        // 4. Middle again: offset = TAB_WIDTH
        let g_mid2 = calculate_tab_strip_geometry(total_tabs, base_width, TAB_WIDTH);
        assert_eq!(g_mid, g_mid2);

        // 5. Left edge again: offset = 0.0
        let g_left2 = calculate_tab_strip_geometry(total_tabs, base_width, 0.0);
        assert_eq!(g_left, g_left2);

        // 6. Sash drag resize audit:
        app.tab_scroll_offset_x = g_left.max_scroll;
        // Drag sash to make sidebar wider (shrinking tab strip base width)
        app.update(Message::SashPressed);
        app.update(Message::SashDragged(ACTIVITY_BAR_WIDTH + 350.0));
        app.update(Message::SashReleased);
        let new_base = app.tab_strip_base_width();
        let g_sash_wide =
            calculate_tab_strip_geometry(total_tabs, new_base, app.tab_scroll_offset_x);
        assert!(app.tab_scroll_offset_x <= g_sash_wide.max_scroll);

        // Drag sash back to default narrow sidebar (expanding tab strip base width)
        app.update(Message::SashPressed);
        app.update(Message::SashDragged(ACTIVITY_BAR_WIDTH + 170.0));
        app.update(Message::SashReleased);
        let new_base2 = app.tab_strip_base_width();
        let g_sash_narrow =
            calculate_tab_strip_geometry(total_tabs, new_base2, app.tab_scroll_offset_x);
        assert!(app.tab_scroll_offset_x <= g_sash_narrow.max_scroll);
    }
}
