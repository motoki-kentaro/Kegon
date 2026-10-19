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
    Id, Space, button, column, container, mouse_area, pick_list, row, space, stack, svg, text,
    text_input, tooltip,
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
    TerminalMetricOverrides, UiFontResolution, UiFontStatus, resolve_terminal_font,
    resolve_ui_font,
};
use crate::i18n::{self, FluentArgs, Locale, Localizer, MessageKey};
use crate::icons::{
    ICON_SIZE, TAB_ADD_ICON_SIZE, TAB_CLOSE_ICON_SIZE, TAB_SCROLL_ICON_SIZE, activity_icon,
    settings_icon, tab_add_icon, tab_close_icon, tab_scroll_left_icon, tab_scroll_right_icon,
};
use crate::settings::{
    AppearanceSettings, ApplicationSettings, LocalePreference, ShellIntegrationMode, ShellMode,
    ShellPathError, TerminalMetric, ThemePreference, resolve_application_locale, save_settings,
    validate_shell_path,
};
use crate::terminal::dopamine::tab_rainbow;
use crate::terminal::{
    BorderKind, CommandState, CompletionReason, DopamineController, DopamineSoundKind, EventSink,
    InputArbiter, InputRoute, MetadataCorrelation, ParticleOverlayProgram, ShellIntegrationStatus,
    ShellReportedStatus, SystemClipboard, TerminalEvent, TerminalInputEvent, TerminalKeyEncoder,
    TerminalProgram, TerminalRegistry, TerminalSession, TerminalSurface, VerifiedCommandEvent,
    calculate_cursor_rectangle, registry::TerminalEntry,
};
use crate::theme::{KegonTheme, ThemeId, resolve_theme, style};
use crate::workbench::{ActivityItem, SIDE_BAR_DEFAULT_WIDTH, TabId, TabKind, Workbench};

const ACTIVITY_BAR_WIDTH: f32 = 48.0;
const SASH_WIDTH: f32 = 4.0;
pub(crate) const TAB_STRIP_HEIGHT: f32 = 35.0;
/// Width of every tab in the tab strip.
pub(crate) const TAB_WIDTH: f32 = 160.0;
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

/// Text the user is typing into the terminal metric fields of the Settings tab.
///
/// Drafts are never written to the application settings; they only become settings when applied.
/// They live while the Settings tab is open and are rebuilt from the saved settings when it is
/// opened again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MetricDrafts {
    font_size: String,
    cell_height: String,
    cell_width: String,
}

impl MetricDrafts {
    /// Drafts showing the saved values (empty for an unset or ignored value).
    fn from_settings(appearance: &AppearanceSettings) -> Self {
        let text = |metric| {
            appearance
                .metric(metric)
                .map(format_metric)
                .unwrap_or_default()
        };
        Self {
            font_size: text(TerminalMetric::FontSize),
            cell_height: text(TerminalMetric::CellHeight),
            cell_width: text(TerminalMetric::CellWidth),
        }
    }

    fn get(&self, metric: TerminalMetric) -> &str {
        match metric {
            TerminalMetric::FontSize => &self.font_size,
            TerminalMetric::CellHeight => &self.cell_height,
            TerminalMetric::CellWidth => &self.cell_width,
        }
    }

    fn set(&mut self, metric: TerminalMetric, text: String) {
        match metric {
            TerminalMetric::FontSize => self.font_size = text,
            TerminalMetric::CellHeight => self.cell_height = text,
            TerminalMetric::CellWidth => self.cell_width = text,
        }
    }
}

/// Why a draft cannot be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftError {
    /// Nothing typed: not an error to show, just nothing to apply.
    Empty,
    /// Not a finite number inside the metric's range.
    Invalid,
}

/// Parses a draft into a value the settings accept.
fn parse_metric_draft(metric: TerminalMetric, draft: &str) -> Result<f64, DraftError> {
    let draft = draft.trim();
    if draft.is_empty() {
        return Err(DraftError::Empty);
    }
    match draft.parse::<f64>() {
        Ok(value) if value.is_finite() && metric.range().contains(&value) => Ok(value),
        _ => Err(DraftError::Invalid),
    }
}

/// A metric value as shown in an input field (`12`, `1.2`).
fn format_metric(value: f64) -> String {
    value.to_string()
}

/// What the Settings tab shows and enables for one metric, derived from the draft and the saved value.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MetricFieldState {
    /// The draft is valid and differs from the saved value.
    can_apply: bool,
    /// The draft differs from what is saved (shown as "not applied").
    pending: bool,
    /// The draft is not usable and an error is shown.
    invalid: bool,
}

fn metric_field_state(
    metric: TerminalMetric,
    draft: &str,
    appearance: &AppearanceSettings,
) -> MetricFieldState {
    let saved = appearance.metric(metric);
    match parse_metric_draft(metric, draft) {
        Ok(value) => {
            let differs = saved != Some(value);
            MetricFieldState {
                can_apply: differs,
                pending: differs,
                invalid: false,
            }
        }
        Err(DraftError::Empty) => MetricFieldState {
            can_apply: false,
            pending: saved.is_some(),
            invalid: false,
        },
        Err(DraftError::Invalid) => MetricFieldState {
            can_apply: false,
            pending: true,
            invalid: true,
        },
    }
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
    /// The window scale factor became known or changed.
    ScaleFactorChanged(f32),
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
    /// A terminal metric field was edited (draft only; nothing is saved).
    SettingsMetricDraftChanged(TerminalMetric, String),
    /// Enter or the Apply button of a terminal metric field.
    SettingsMetricApply(TerminalMetric),
    /// The Reset button of a terminal metric field.
    SettingsMetricReset(TerminalMetric),
    /// The Built-in Glyphs checkbox (saved and applied at once).
    SettingsBuiltinGlyphsToggled(bool),
    /// The Reset button of the Built-in Glyphs setting: removes the key (default: on).
    SettingsBuiltinGlyphsReset,
    /// The Dopamine Mode checkbox (saved and applied at once).
    SettingsDopamineToggled(bool),
    /// The Reset button of the Dopamine Mode setting: removes the key (default: off).
    SettingsDopamineReset,
    /// Sound profile choice for Dopamine Mode.
    SettingsDopamineSoundChanged(DopamineSoundKind),
    /// Preview active sound profile in Settings tab.
    SettingsDopamineSoundPreview,
    /// Redraw/physics tick for Dopamine Mode animations.
    DopamineTicked(std::time::Instant),
    /// The Shell Mode pick list selection changed ("auto" or "custom").
    SettingsShellModeChanged(ShellMode),
    /// The Shell Path input text changed.
    SettingsShellPathChanged(String),
    /// The Shell Integration pick list selection changed ("auto" or "disabled").
    SettingsShellIntegrationChanged(ShellIntegrationMode),
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
    /// Drafts of the terminal metric fields while the Settings tab is open.
    metric_drafts: Option<MetricDrafts>,
    /// Window scale factor (physical pixels per logical pixel); 1.0 until the window reports it.
    scale_factor: f32,
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
    /// Dopamine Mode controller managing particles and audio.
    dopamine: DopamineController,
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
            &TerminalMetricOverrides::from_settings(&settings.appearance),
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
            && let Ok(session) = TerminalSession::spawn_with_settings(
                &settings.terminal,
                cols,
                rows,
                terminal_font_config.metrics.cell_width as u16,
                terminal_font_config.metrics.cell_height as u16,
                tagged_sink(&tx, tab),
                None,
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
            metric_drafts: None,
            scale_factor: 1.0,
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
            dopamine: {
                let mut ctrl = DopamineController::new(settings.dopamine.enabled());
                ctrl.set_sound_kind(settings.dopamine.sound());
                ctrl
            },
            settings,
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
            &TerminalMetricOverrides::from_settings(&self.settings.appearance),
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

    /// Validates the draft of `metric` and, if it is usable and new, saves and applies it.
    fn apply_metric_draft(&mut self, metric: TerminalMetric) {
        let drafts = self
            .metric_drafts
            .get_or_insert_with(|| MetricDrafts::from_settings(&self.settings.appearance));
        let Ok(value) = parse_metric_draft(metric, drafts.get(metric)) else {
            return;
        };
        if self.settings.appearance.metric(metric) == Some(value) {
            return;
        }
        drafts.set(metric, format_metric(value));
        let mut updated = self.settings.clone();
        updated.appearance.set_metric(metric, Some(value));
        self.commit_settings(updated);
        self.apply_terminal_font_preference();
    }

    /// Clears `metric` (also replacing an ignored value), saves, applies, and empties its draft.
    fn reset_metric(&mut self, metric: TerminalMetric) {
        self.metric_drafts
            .get_or_insert_with(|| MetricDrafts::from_settings(&self.settings.appearance))
            .set(metric, String::new());
        if !self.settings.appearance.has_metric_key(metric) {
            return;
        }
        let mut updated = self.settings.clone();
        updated.appearance.set_metric(metric, None);
        self.commit_settings(updated);
        self.apply_terminal_font_preference();
    }

    /// Saves the Built-in Glyphs choice and applies it at once. Nothing else changes: the terminal
    /// sessions, the unapplied metric drafts and the grid size are left alone, and the next frame is
    /// drawn the other way.
    fn set_builtin_glyphs(&mut self, enabled: bool) {
        if self.settings.appearance.terminal_builtin_glyphs
            == Some(crate::settings::BoolSetting::Value(enabled))
        {
            return;
        }
        let mut updated = self.settings.clone();
        updated.appearance.set_builtin_glyphs(enabled);
        self.commit_settings(updated);
        self.builtin_glyphs_changed();
    }

    /// Removes the Built-in Glyphs key, which means the default (on), also replacing an ignored value.
    fn reset_builtin_glyphs(&mut self) {
        if !self.settings.appearance.has_builtin_glyphs_key() {
            return;
        }
        let mut updated = self.settings.clone();
        updated.appearance.clear_builtin_glyphs();
        self.commit_settings(updated);
        self.builtin_glyphs_changed();
    }

    /// Frees the shade tiles when the built-in glyphs are off (nothing draws them then); they are built
    /// again on demand when the setting is turned back on.
    fn builtin_glyphs_changed(&mut self) {
        if !self.settings.appearance.builtin_glyphs() {
            crate::terminal::renderer::clear_shade_tiles();
        }
    }

    /// Saves the Dopamine Mode choice and applies it at once. When disabled, all active particles
    /// and audio are cleared immediately.
    fn set_dopamine_enabled(&mut self, enabled: bool) {
        if self.settings.dopamine.enabled == Some(crate::settings::BoolSetting::Value(enabled)) {
            return;
        }
        let mut updated = self.settings.clone();
        updated.dopamine.set_enabled(enabled);
        self.commit_settings(updated);
        self.dopamine.set_enabled(enabled);
    }

    /// Sets the active Dopamine Mode sound profile, saves it, and previews it immediately.
    fn set_dopamine_sound(&mut self, kind: DopamineSoundKind) {
        if self.settings.dopamine.sound == Some(kind.as_str().to_string()) {
            return;
        }
        let mut updated = self.settings.clone();
        updated.dopamine.set_sound(kind);
        self.commit_settings(updated);
        self.dopamine.set_sound_kind(kind);
        self.dopamine.audio_mut().play_erase();
    }

    /// Removes the Dopamine Mode keys (enabled and sound), returning to defaults.
    fn reset_dopamine(&mut self) {
        if !self.settings.dopamine.has_enabled_key() && !self.settings.dopamine.has_sound_key() {
            return;
        }
        let mut updated = self.settings.clone();
        updated.dopamine.clear_enabled();
        updated.dopamine.clear_sound();
        self.commit_settings(updated);
        self.dopamine.set_enabled(self.settings.dopamine.enabled());
        self.dopamine.set_sound_kind(self.settings.dopamine.sound());
    }

    /// Records the window scale factor; a value that is not a positive finite number is ignored.
    fn set_scale_factor(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.scale_factor = factor;
        }
    }

    /// Work to run once at startup: ask the window for its scale factor.
    pub fn initial_task() -> Task<Message> {
        iced::window::oldest().then(|id| match id {
            Some(id) => iced::window::scale_factor(id).map(Message::ScaleFactorChanged),
            None => Task::none(),
        })
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
        if is_new && kind == TabKind::Settings {
            // A reopened Settings tab starts from the saved values, never from an old draft.
            self.metric_drafts = Some(MetricDrafts::from_settings(&self.settings.appearance));
        }
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
        let terminal_settings = self.settings.terminal.clone();
        self.open_terminal_tab_with(|tab, cwd| {
            let spawn = |cwd| {
                TerminalSession::spawn_with_settings(
                    &terminal_settings,
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
        if self.workbench.tabs().get(tab).map(|t| t.kind) == Some(TabKind::Settings) {
            self.metric_drafts = None;
        }
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
            Message::ScaleFactorChanged(factor) => self.set_scale_factor(factor),
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
                self.dopamine.clear_tab(id);
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
                let is_shell_exit = matches!(event, TerminalEvent::ChildExit(_));
                let (_handled, _cmd_event) =
                    self.terminals.handle_event_with_command_event(tab, event);

                if is_shell_exit {
                    self.dopamine.on_shell_exited(tab);
                } else {
                    match _cmd_event {
                        Some(VerifiedCommandEvent::CommandStarted { command_id }) => {
                            self.dopamine.on_command_started(
                                tab,
                                command_id,
                                std::time::Instant::now(),
                            );
                        }
                        Some(VerifiedCommandEvent::CommandFinished {
                            command_id,
                            execution_result,
                        }) => {
                            if execution_result.completion == CompletionReason::ShellExited {
                                self.dopamine.on_shell_exited(tab);
                            } else {
                                let is_integration_active = self
                                    .terminals
                                    .get(tab)
                                    .map(|e| e.shell_integration_status().is_active())
                                    .unwrap_or(false);

                                let valid_correlation = execution_result.completion
                                    == CompletionReason::DObserved
                                    && execution_result.correlation == MetadataCorrelation::Matched;

                                if self.dopamine.is_enabled()
                                    && is_integration_active
                                    && valid_correlation
                                {
                                    match execution_result.shell_status {
                                        ShellReportedStatus::Success => {
                                            self.dopamine.trigger_command_border(
                                                tab,
                                                command_id,
                                                BorderKind::Rainbow,
                                                std::time::Instant::now(),
                                            );
                                            self.dopamine.on_command_finished(
                                                tab,
                                                command_id,
                                                true,
                                                true,
                                                std::time::Instant::now(),
                                            );
                                        }
                                        ShellReportedStatus::Failure => {
                                            self.dopamine.trigger_command_border(
                                                tab,
                                                command_id,
                                                BorderKind::Red,
                                                std::time::Instant::now(),
                                            );
                                            self.dopamine.on_command_finished(
                                                tab,
                                                command_id,
                                                false,
                                                true,
                                                std::time::Instant::now(),
                                            );
                                        }
                                        ShellReportedStatus::Unknown => {
                                            self.dopamine.on_command_finished(
                                                tab,
                                                command_id,
                                                false,
                                                true,
                                                std::time::Instant::now(),
                                            );
                                        }
                                    }
                                } else if self.dopamine.is_enabled() {
                                    let is_missing_d =
                                        execution_result.completion == CompletionReason::MissingD;
                                    let now = std::time::Instant::now();
                                    self.dopamine
                                        .on_command_finished(tab, command_id, false, false, now);
                                    if is_missing_d {
                                        self.dopamine.on_prompt_started(tab, now);
                                    }
                                }
                            }
                        }
                        Some(VerifiedCommandEvent::PromptStarted) => {
                            self.dopamine
                                .on_prompt_started(tab, std::time::Instant::now());
                        }
                        _ => {}
                    }
                }
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
            Message::SettingsMetricDraftChanged(metric, text) => {
                self.metric_drafts
                    .get_or_insert_with(|| MetricDrafts::from_settings(&self.settings.appearance))
                    .set(metric, text);
            }
            Message::SettingsMetricApply(metric) => self.apply_metric_draft(metric),
            Message::SettingsMetricReset(metric) => self.reset_metric(metric),
            Message::SettingsBuiltinGlyphsToggled(enabled) => self.set_builtin_glyphs(enabled),
            Message::SettingsBuiltinGlyphsReset => self.reset_builtin_glyphs(),
            Message::SettingsDopamineToggled(enabled) => self.set_dopamine_enabled(enabled),
            Message::SettingsDopamineReset => self.reset_dopamine(),
            Message::SettingsDopamineSoundChanged(kind) => self.set_dopamine_sound(kind),
            Message::SettingsDopamineSoundPreview => {
                self.dopamine.audio_mut().play_erase();
            }
            Message::DopamineTicked(now) => {
                let h = (self.window_size.height - TAB_STRIP_HEIGHT).max(100.0);
                self.dopamine.tick(now, h);
            }
            Message::SettingsShellModeChanged(mode) => {
                let mut updated_settings = self.settings.clone();
                updated_settings.terminal.shell_mode = mode;
                self.commit_settings(updated_settings);
            }
            Message::SettingsShellPathChanged(path) => {
                let mut updated_settings = self.settings.clone();
                updated_settings.terminal.shell_path = path;
                self.commit_settings(updated_settings);
            }
            Message::SettingsShellIntegrationChanged(mode) => {
                let mut updated_settings = self.settings.clone();
                updated_settings.terminal.shell_integration = mode;
                self.commit_settings(updated_settings);
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
            Event::Window(iced::window::Event::Rescaled(factor)) => self.set_scale_factor(factor),
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
                        let is_paste = command_id == CommandId::TerminalPaste;
                        let pasted_text = if is_paste && self.dopamine.is_enabled() {
                            self.system_clipboard.get_text()
                        } else {
                            None
                        };
                        CommandDispatcher::dispatch(
                            command_id,
                            &mut self.workbench,
                            active_tab
                                .and_then(|id| self.terminals.get(id))
                                .map(TerminalEntry::session),
                            &mut self.system_clipboard,
                        );
                        if let Some(tab_id) = active_tab
                            && let Some(text) = pasted_text
                        {
                            self.dopamine
                                .on_text_input(tab_id, &text, std::time::Instant::now());
                        }
                    }
                    InputRoute::Terminal => {
                        if self.terminal_focused
                            && let Some(session) = self.active_session()
                        {
                            let is_backspace = logical_key
                                == keyboard::Key::Named(keyboard::key::Named::Backspace);
                            let input_evt = TerminalInputEvent::press(
                                logical_key.clone(),
                                physical_key,
                                modifiers,
                                text.as_deref().map(|s| s.to_string()),
                            );
                            let mode = session.keyboard_mode();
                            if let Some(bytes) = TerminalKeyEncoder::encode(&input_evt, mode) {
                                session.write_input(bytes);

                                if let Some(tab_id) = self.workbench.tabs().active().map(|t| t.id)
                                    && self.dopamine.is_enabled()
                                {
                                    let now = std::time::Instant::now();
                                    if is_backspace {
                                        let (col, line) = session.cursor_position();
                                        let metrics = &self.terminal_font_config.metrics;
                                        let origin_x = if col > 0 {
                                            (col as f32 - 0.5) * metrics.cell_width
                                        } else {
                                            0.5 * metrics.cell_width
                                        };
                                        let origin_y = (line as f32 + 0.5) * metrics.cell_height;

                                        let base = session.foreground_left_of_cursor().map(|fg| {
                                            crate::terminal::renderer::convert_color(
                                                fg,
                                                true,
                                                &self.active_theme.terminal,
                                            )
                                        });

                                        if !modifiers.control() && !modifiers.alt() {
                                            self.dopamine.on_backspace_input(tab_id, now);
                                        }
                                        self.dopamine.on_backspace_colored(
                                            tab_id,
                                            Point::new(origin_x, origin_y),
                                            base,
                                            now,
                                        );
                                    } else if !modifiers.control() && !modifiers.alt() {
                                        if logical_key
                                            == keyboard::Key::Named(keyboard::key::Named::Enter)
                                        {
                                            self.dopamine.on_enter(tab_id, now);
                                        } else if let Some(ref s) = text {
                                            for c in s.chars() {
                                                self.dopamine.on_char_input(tab_id, c, now);
                                            }
                                        } else if logical_key
                                            == keyboard::Key::Named(keyboard::key::Named::Space)
                                        {
                                            self.dopamine.on_char_input(tab_id, ' ', now);
                                        }
                                    }
                                }
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
                    let active_tab = self.workbench.tabs().active().map(|t| t.id);
                    match ime_event {
                        ImeEvent::Preedit(text, _) => {
                            self.preedit_text = if text.is_empty() { None } else { Some(text) };
                        }
                        ImeEvent::Commit(text) => {
                            self.preedit_text = None;
                            session.write_input(text.clone().into_bytes());
                            if let Some(tab_id) = active_tab
                                && self.dopamine.is_enabled()
                            {
                                self.dopamine.on_text_input(
                                    tab_id,
                                    &text,
                                    std::time::Instant::now(),
                                );
                            }
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

        let dopamine_tick = if self.dopamine.is_enabled() {
            iced::window::frames().map(Message::DopamineTicked)
        } else {
            Subscription::none()
        };

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

            Subscription::batch([events, terminal_events, dopamine_tick, drag])
        } else {
            Subscription::batch([events, terminal_events, dopamine_tick])
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

    /// One terminal metric row of the Settings tab: label, description, current value, a draft input
    /// with Apply and Reset, and the pending / error / ignored-value notes.
    fn settings_metric_field(&self, metric: TerminalMetric) -> Element<'_, Message> {
        let theme = self.active_theme;
        let (label_key, description_key, default_key) = match metric {
            TerminalMetric::FontSize => (
                MessageKey::SettingsFontSizeLabel,
                MessageKey::SettingsFontSizeDescription,
                MessageKey::SettingsFontSizeDefault,
            ),
            TerminalMetric::CellHeight => (
                MessageKey::SettingsCellHeightLabel,
                MessageKey::SettingsCellHeightDescription,
                MessageKey::SettingsCellHeightDefault,
            ),
            TerminalMetric::CellWidth => (
                MessageKey::SettingsCellWidthLabel,
                MessageKey::SettingsCellWidthDescription,
                MessageKey::SettingsCellWidthDefault,
            ),
        };
        let appearance = &self.settings.appearance;
        let fallback;
        let draft = match &self.metric_drafts {
            Some(drafts) => drafts.get(metric),
            None => {
                fallback = MetricDrafts::from_settings(appearance);
                fallback.get(metric)
            }
        };
        let state = metric_field_state(metric, draft, appearance);

        let small = |content: String, color: Color| {
            text(content)
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font)
                .color(color)
        };

        let default_text = self.localizer.text(default_key);
        let current = match appearance.metric(metric) {
            Some(value) => {
                let shown = match metric {
                    TerminalMetric::FontSize => format!("{} pt", format_metric(value)),
                    _ => format!("× {}", format_metric(value)),
                };
                let mut args = FluentArgs::new();
                args.set("value", shown);
                self.localizer
                    .text_with(MessageKey::SettingsMetricCurrent, &args)
            }
            None => {
                let mut args = FluentArgs::new();
                args.set("value", default_text.clone());
                self.localizer
                    .text_with(MessageKey::SettingsMetricDefaultValue, &args)
            }
        };

        let input = iced::widget::text_input(&default_text, draft)
            .on_input(move |text| Message::SettingsMetricDraftChanged(metric, text))
            .on_submit(Message::SettingsMetricApply(metric))
            .size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .width(160);

        let mut apply = button(
            text(self.localizer.text(MessageKey::SettingsMetricApply))
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font),
        )
        .style(move |_, status| style::secondary_button(theme, status, false));
        if state.can_apply {
            apply = apply.on_press(Message::SettingsMetricApply(metric));
        }

        let mut reset = button(
            text(self.localizer.text(MessageKey::SettingsMetricReset))
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font),
        )
        .style(move |_, status| style::secondary_button(theme, status, false));
        if appearance.has_metric_key(metric) {
            reset = reset.on_press(Message::SettingsMetricReset(metric));
        }

        let mut controls = row![input, apply, reset].spacing(8).align_y(Center);
        if state.pending && !state.invalid {
            controls = controls.push(small(
                self.localizer.text(MessageKey::SettingsMetricPending),
                theme.semantic.warning,
            ));
        }

        let mut field = column![
            self.settings_label(label_key),
            small(self.localizer.text(description_key), theme.text.muted),
            small(current, theme.text.muted),
            controls,
        ]
        .spacing(6);

        if state.invalid {
            let range = metric.range();
            let mut args = FluentArgs::new();
            args.set("min", format_metric(*range.start()));
            args.set("max", format_metric(*range.end()));
            field = field.push(small(
                self.localizer
                    .text_with(MessageKey::SettingsMetricRangeError, &args),
                theme.semantic.destructive,
            ));
        }
        if let Some(raw) = appearance.invalid_metric_text(metric) {
            let mut args = FluentArgs::new();
            args.set("value", raw);
            field = field.push(small(
                self.localizer
                    .text_with(MessageKey::SettingsMetricStoredInvalid, &args),
                theme.semantic.destructive,
            ));
        }
        field.into()
    }

    /// The Built-in Glyphs row of the Settings tab: a checkbox that saves and applies at once, its
    /// description, a Reset button, and a note when the file holds a value that is not a boolean.
    fn settings_builtin_glyphs_field(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let appearance = &self.settings.appearance;
        let small = |content: String, color: Color| {
            text(content)
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font)
                .color(color)
        };
        let inset = theme.workbench.background;
        let checkbox = iced::widget::checkbox(appearance.builtin_glyphs())
            .label(self.localizer.text(MessageKey::SettingsBuiltinGlyphsLabel))
            .on_toggle(Message::SettingsBuiltinGlyphsToggled)
            .size(16)
            .text_size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .style(move |_, _| iced::widget::checkbox::Style {
                background: inset.into(),
                icon_color: theme.text.emphasis,
                border: iced::Border {
                    color: theme.workbench.border,
                    width: 1.0,
                    radius: 3.0.into(),
                },
                text_color: Some(theme.text.primary),
            });
        let mut reset = button(
            text(self.localizer.text(MessageKey::SettingsMetricReset))
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font),
        )
        .style(move |_, status| style::secondary_button(theme, status, false));
        if appearance.has_builtin_glyphs_key() {
            reset = reset.on_press(Message::SettingsBuiltinGlyphsReset);
        }
        let mut field = column![
            row![checkbox, reset].spacing(16).align_y(Center),
            small(
                self.localizer
                    .text(MessageKey::SettingsBuiltinGlyphsDescription),
                theme.text.muted,
            ),
        ]
        .spacing(6);
        if let Some(raw) = appearance.invalid_builtin_glyphs_text() {
            let mut args = FluentArgs::new();
            args.set("value", raw);
            field = field.push(small(
                self.localizer
                    .text_with(MessageKey::SettingsBuiltinGlyphsStoredInvalid, &args),
                theme.semantic.destructive,
            ));
        }
        field.into()
    }

    /// The Dopamine Mode row of the Settings tab: a checkbox that saves and applies at once, its
    /// description, a Reset button, and a note when the file holds an invalid value.
    fn settings_dopamine_field(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let dopamine = &self.settings.dopamine;
        let small = |content: String, color: Color| {
            text(content)
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font)
                .color(color)
        };
        let inset = theme.workbench.background;
        let checkbox = iced::widget::checkbox(dopamine.enabled())
            .label(
                self.localizer
                    .text(MessageKey::SettingsDopamineEnabledLabel),
            )
            .on_toggle(Message::SettingsDopamineToggled)
            .size(16)
            .text_size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .style(move |_, _| iced::widget::checkbox::Style {
                background: inset.into(),
                icon_color: theme.text.emphasis,
                border: iced::Border {
                    color: theme.workbench.border,
                    width: 1.0,
                    radius: 3.0.into(),
                },
                text_color: Some(theme.text.primary),
            });
        let current_sound = dopamine.sound();
        let sound_label = small(
            self.localizer.text(MessageKey::SettingsDopamineSoundLabel),
            theme.text.primary,
        );

        let sound_btn = |kind: DopamineSoundKind, label_key: MessageKey| {
            let is_selected = current_sound == kind;
            button(
                text(self.localizer.text(label_key))
                    .size(UI_TEXT_SIZE)
                    .font(self.active_ui_font),
            )
            .style(move |_, status| {
                if is_selected {
                    style::filled_button(theme, theme.interaction.accent, status, false)
                } else {
                    style::secondary_button(theme, status, false)
                }
            })
            .on_press(Message::SettingsDopamineSoundChanged(kind))
        };

        let preview_btn = button(
            text(
                self.localizer
                    .text(MessageKey::SettingsDopamineSoundPreview),
            )
            .size(UI_TEXT_SIZE)
            .font(self.active_ui_font),
        )
        .style(move |_, status| style::secondary_button(theme, status, false))
        .on_press(Message::SettingsDopamineSoundPreview);

        let sound_row = row![
            sound_btn(DopamineSoundKind::Zap, MessageKey::SettingsDopamineSoundZap),
            sound_btn(
                DopamineSoundKind::Crack,
                MessageKey::SettingsDopamineSoundCrack
            ),
            sound_btn(
                DopamineSoundKind::Burst,
                MessageKey::SettingsDopamineSoundBurst
            ),
            preview_btn,
        ]
        .spacing(8)
        .align_y(Center);

        let mut reset = button(
            text(self.localizer.text(MessageKey::SettingsMetricReset))
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font),
        )
        .style(move |_, status| style::secondary_button(theme, status, false));
        if dopamine.has_enabled_key() || dopamine.has_sound_key() {
            reset = reset.on_press(Message::SettingsDopamineReset);
        }
        let mut field = column![
            row![checkbox, reset].spacing(16).align_y(Center),
            small(
                self.localizer
                    .text(MessageKey::SettingsDopamineEnabledDescription),
                theme.text.muted,
            ),
            sound_label,
            sound_row,
        ]
        .spacing(8);
        if let Some(raw) = dopamine.invalid_enabled_text() {
            let mut args = FluentArgs::new();
            args.set("value", raw);
            field = field.push(small(
                self.localizer
                    .text_with(MessageKey::SettingsDopamineEnabledStoredInvalid, &args),
                theme.semantic.destructive,
            ));
        }
        field.into()
    }

    /// Shell selection and executable path setting for terminal sessions.
    fn settings_shell_field(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let is_custom = matches!(self.settings.terminal.shell_mode, ShellMode::Custom);

        let mode_label = self.settings_label(MessageKey::SettingsShellModeLabel);
        let mode_description = text(
            self.localizer
                .text(MessageKey::SettingsShellModeDescription),
        )
        .size(UI_TEXT_SIZE)
        .font(self.active_ui_font)
        .color(theme.text.muted);

        let mut mode_options = vec![
            ShellModeOption {
                mode: ShellMode::Auto,
                label: self.localizer.text(MessageKey::SettingsShellModeAuto),
            },
            ShellModeOption {
                mode: ShellMode::Custom,
                label: self.localizer.text(MessageKey::SettingsShellModeCustom),
            },
        ];
        if let ShellMode::Unknown(ref raw) = self.settings.terminal.shell_mode {
            mode_options.push(ShellModeOption {
                mode: ShellMode::Unknown(raw.clone()),
                label: format!("Unknown ({raw})"),
            });
        }

        let selected_mode = mode_options
            .iter()
            .find(|opt| opt.mode == self.settings.terminal.shell_mode)
            .cloned();

        let mode_picker = self.settings_pick_list(mode_options, selected_mode, |opt| {
            Message::SettingsShellModeChanged(opt.mode)
        });

        let path_error = if is_custom {
            validate_shell_path(&self.settings.terminal.shell_path).err()
        } else {
            None
        };
        let is_path_invalid = path_error.is_some();

        let path_label = text(self.localizer.text(MessageKey::SettingsShellPathLabel))
            .size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .color(if is_custom {
                theme.text.primary
            } else {
                theme.text.disabled
            });

        let placeholder = self
            .localizer
            .text(MessageKey::SettingsShellPathPlaceholder);
        let mut path_input = text_input(&placeholder, &self.settings.terminal.shell_path)
            .size(UI_TEXT_SIZE)
            .font(self.active_ui_font)
            .padding(8)
            .style(move |_, status| shell_text_input_style(theme, status, is_path_invalid));

        if is_custom {
            path_input = path_input.on_input(Message::SettingsShellPathChanged);
        }

        let note = text(self.localizer.text(MessageKey::SettingsShellNewTabNote))
            .size(11.0)
            .font(self.active_ui_font)
            .color(theme.text.muted);

        let mut col = column![
            mode_label,
            mode_description,
            mode_picker,
            path_label,
            path_input,
        ]
        .spacing(8);

        if let Some(err) = path_error {
            let error_key = match err {
                ShellPathError::Empty => MessageKey::SettingsShellPathEmpty,
                ShellPathError::NotAbsolute => MessageKey::SettingsShellPathNotAbsolute,
                ShellPathError::NotFound => MessageKey::SettingsShellPathNotFound,
                ShellPathError::IsDirectory => MessageKey::SettingsShellPathIsDirectory,
                ShellPathError::NotExecutable => MessageKey::SettingsShellPathNotExecutable,
                ShellPathError::HasArguments => MessageKey::SettingsShellPathHasArguments,
            };
            let err_text = text(self.localizer.text(error_key))
                .size(UI_TEXT_SIZE)
                .font(self.active_ui_font)
                .color(theme.semantic.destructive);
            col = col.push(err_text);
        }

        col = col.push(note);
        col.into()
    }

    /// Shell Integration toggle setting (Auto / Disabled).
    fn settings_shell_integration_field(&self) -> Element<'_, Message> {
        let theme = self.active_theme;
        let label = self.settings_label(MessageKey::SettingsShellIntegrationLabel);
        let description = text(
            self.localizer
                .text(MessageKey::SettingsShellIntegrationDescription),
        )
        .size(UI_TEXT_SIZE)
        .font(self.active_ui_font)
        .color(theme.text.muted);

        let mut options = vec![
            ShellIntegrationOption {
                mode: ShellIntegrationMode::Auto,
                label: self
                    .localizer
                    .text(MessageKey::SettingsShellIntegrationAuto),
            },
            ShellIntegrationOption {
                mode: ShellIntegrationMode::Disabled,
                label: self
                    .localizer
                    .text(MessageKey::SettingsShellIntegrationDisabled),
            },
        ];
        if let ShellIntegrationMode::Unknown(ref raw) = self.settings.terminal.shell_integration {
            options.push(ShellIntegrationOption {
                mode: ShellIntegrationMode::Unknown(raw.clone()),
                label: format!("Unknown ({raw})"),
            });
        }

        let selected = options
            .iter()
            .find(|opt| opt.mode == self.settings.terminal.shell_integration)
            .cloned();

        let picker = self.settings_pick_list(options, selected, |opt| {
            Message::SettingsShellIntegrationChanged(opt.mode)
        });

        let note = text(self.localizer.text(MessageKey::SettingsShellNewTabNote))
            .size(11.0)
            .font(self.active_ui_font)
            .color(theme.text.muted);

        column![label, description, picker, note].spacing(8).into()
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
                scale_factor: self.scale_factor,
                builtin_glyphs: self.settings.appearance.builtin_glyphs(),
            };

            let canvas_widget = Canvas::new(canvas_program).width(Fill).height(Fill);

            let (cursor_col, cursor_line) = session.cursor_position();
            let cursor_rect = calculate_cursor_rectangle(
                cursor_col,
                cursor_line,
                &self.terminal_font_config.metrics,
            );

            let is_focused = self.terminal_focused && !self.is_modal_open();

            let surface: Element<'_, Message> =
                TerminalSurface::new(canvas_widget, is_focused, Some(cursor_rect)).into();

            let active_tab_id = self.workbench.tabs().active().map(|tab| tab.id);
            let particles = active_tab_id
                .map(|id| self.dopamine.particles_for_tab(id))
                .unwrap_or(&[]);
            let border = active_tab_id
                .and_then(|id| self.dopamine.border_for_tab(id, std::time::Instant::now()));
            let overlay_program = ParticleOverlayProgram::new(particles, border, self.scale_factor);
            let overlay_widget = Canvas::new(overlay_program).width(Fill).height(Fill);

            let main_content: Element<'_, Message> = stack![surface, overlay_widget]
                .width(Fill)
                .height(Fill)
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
            column![
                terminal_font_col,
                self.settings_metric_field(TerminalMetric::FontSize),
                self.settings_metric_field(TerminalMetric::CellHeight),
                self.settings_metric_field(TerminalMetric::CellWidth),
                self.settings_builtin_glyphs_field(),
                self.settings_shell_field(),
                self.settings_shell_integration_field(),
            ]
            .spacing(16)
            .into(),
        );

        let dopamine = self.settings_category(
            MessageKey::SettingsCategoryDopamineTitle,
            MessageKey::SettingsCategoryDopamineDescription,
            column![self.settings_dopamine_field()].spacing(16).into(),
        );

        let page = column![
            column![heading, description].spacing(6),
            general,
            appearance,
            terminal,
            dopamine,
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

    /// Text lines of the Shell Integration hover tooltip for a terminal tab.
    fn terminal_tab_tooltip_lines(&self, tab: TabId) -> Option<Vec<String>> {
        let entry = self.terminals.get(tab)?;

        let raw_program = entry.shell_program();
        let display_program = std::path::Path::new(raw_program)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(raw_program);

        let status = entry.shell_integration_status();
        let status_key = match status {
            ShellIntegrationStatus::Active => MessageKey::TerminalTooltipStatusActive,
            ShellIntegrationStatus::ActivePromptOnly => {
                MessageKey::TerminalTooltipStatusActivePromptOnly
            }
            ShellIntegrationStatus::Waiting => MessageKey::TerminalTooltipStatusWaiting,
            ShellIntegrationStatus::Disabled => MessageKey::TerminalTooltipStatusDisabled,
            ShellIntegrationStatus::Unsupported => MessageKey::TerminalTooltipStatusUnsupported,
            ShellIntegrationStatus::Unavailable => MessageKey::TerminalTooltipStatusUnavailable,
        };

        let integration_label = self
            .localizer
            .text(MessageKey::TerminalTooltipShellIntegration);
        let status_text = self.localizer.text(status_key);

        let mut lines = vec![
            display_program.to_string(),
            format!("{integration_label}: {status_text}"),
        ];

        if status.is_active() {
            let state_label = self.localizer.text(MessageKey::TerminalTooltipCommandState);
            let state_key = match entry.command_state() {
                CommandState::Unknown => MessageKey::TerminalTooltipCommandStateUnknown,
                CommandState::Prompt => MessageKey::TerminalTooltipCommandStatePrompt,
                CommandState::Editing => MessageKey::TerminalTooltipCommandStateEditing,
                CommandState::Executing { .. } => MessageKey::TerminalTooltipCommandStateExecuting,
                CommandState::Finished { .. } => MessageKey::TerminalTooltipCommandStateFinished,
            };
            let state_text = self.localizer.text(state_key);
            lines.push(format!("{state_label}: {state_text}"));

            let shell_status_label = self
                .localizer
                .text(MessageKey::TerminalTooltipShellReportedStatus);
            let last_exec = entry.command_state_machine().last_execution_result();
            let shell_status_text = match last_exec {
                Some(res) if res.correlation == MetadataCorrelation::Matched => {
                    match res.shell_status {
                        ShellReportedStatus::Success => self
                            .localizer
                            .text(MessageKey::TerminalTooltipStatusSuccess),
                        ShellReportedStatus::Failure => self
                            .localizer
                            .text(MessageKey::TerminalTooltipStatusFailure),
                        ShellReportedStatus::Unknown => self
                            .localizer
                            .text(MessageKey::TerminalTooltipStatusUnknown),
                    }
                }
                Some(_) => self
                    .localizer
                    .text(MessageKey::TerminalTooltipStatusUnknown),
                None => self.localizer.text(MessageKey::TerminalTooltipExitCodeNone),
            };
            lines.push(format!("{shell_status_label}: {shell_status_text}"));

            let osc_exit_code_label = self.localizer.text(MessageKey::TerminalTooltipOscExitCode);
            let osc_exit_code_text = match last_exec.and_then(|r| r.osc133_exit) {
                Some(code) => code.to_string(),
                None => self.localizer.text(MessageKey::TerminalTooltipExitCodeNone),
            };
            lines.push(format!("{osc_exit_code_label}: {osc_exit_code_text}"));
        }

        Some(lines)
    }

    /// Shell Integration hover tooltip for a terminal tab.
    fn terminal_tab_tooltip(&self, tab: TabId) -> Option<Element<'_, Message>> {
        let text_lines = self.terminal_tab_tooltip_lines(tab)?;
        let theme = self.active_theme;

        let mut lines = Vec::with_capacity(text_lines.len());
        for (i, line) in text_lines.into_iter().enumerate() {
            let color = match i {
                0 => theme.text.emphasis,
                1 => theme.text.primary,
                _ => theme.text.muted,
            };
            lines.push(
                text(line)
                    .size(UI_TEXT_SIZE)
                    .font(self.active_ui_font)
                    .color(color)
                    .into(),
            );
        }

        let content =
            container(column(lines).spacing(3))
                .padding([6, 10])
                .style(move |_: &Theme| {
                    container::Style::default()
                        .background(theme.workbench.surface_elevated)
                        .border(Border {
                            color: theme.workbench.border,
                            width: 1.0,
                            radius: 3.0.into(),
                        })
                });

        Some(content.into())
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

        let is_dopamine = self.dopamine.is_enabled();
        let now = std::time::Instant::now();
        let elapsed_secs = self.dopamine.elapsed_animation_seconds(now);

        let tab_buttons = tabs.iter().enumerate().map(|(index, tab)| {
            let is_active = tabs.is_active(tab.id);
            let title = self.tab_title(tab.id);
            let combo = if is_dopamine {
                self.dopamine.combo_for_tab(tab.id, now)
            } else {
                0
            };

            let title_widget = container(
                text(title)
                    .size(UI_TEXT_SIZE)
                    .font(self.active_ui_font)
                    .wrapping(text::Wrapping::None),
            )
            .width(Fill)
            .height(Fill)
            .clip(true)
            .align_y(Center);

            let select_content: Element<'_, Message> = if is_dopamine {
                let badge = tab_rainbow::hit_badge_element(combo, self.active_ui_font);
                row![title_widget, badge]
                    .spacing(4.0)
                    .align_y(Center)
                    .width(Fill)
                    .height(Fill)
                    .into()
            } else {
                title_widget.into()
            };

            // Two sibling buttons rather than a nested one, so a click on the close button can
            // never also select the tab.
            let select_button = button(select_content)
                .width(Fill)
                .height(TAB_STRIP_HEIGHT)
                .padding(iced::Padding::ZERO.left(12.0).right(4.0))
                .on_press(Message::TabSelected(tab.id))
                .style(move |_, status| {
                    tab_rainbow::tab_select_button_style(is_active, status, is_dopamine, theme)
                });

            let select: Element<'_, Message> = if tab.kind == TabKind::Terminal {
                if let Some(tooltip_content) = self.terminal_tab_tooltip(tab.id) {
                    tooltip(select_button, tooltip_content, tooltip::Position::Bottom).into()
                } else {
                    select_button.into()
                }
            } else {
                select_button.into()
            };

            let close = tooltip(
                button(
                    container(
                        svg(tab_close_icon())
                            .width(TAB_CLOSE_ICON_SIZE)
                            .height(TAB_CLOSE_ICON_SIZE)
                            .style(move |_, status| {
                                let is_hovered = matches!(status, svg::Status::Hovered);
                                let color = tab_rainbow::tab_close_icon_color(
                                    is_active,
                                    is_hovered,
                                    is_dopamine,
                                    theme,
                                );
                                svg::Style { color: Some(color) }
                            }),
                    )
                    .center(Fill),
                )
                .width(TAB_CLOSE_WIDTH)
                .height(TAB_STRIP_HEIGHT)
                .padding(0)
                .on_press(Message::TabClosed(tab.id))
                .style(move |_, status| {
                    tab_rainbow::tab_close_button_style(is_active, status, is_dopamine, theme)
                }),
                tooltip_label(self.tab_close_tooltip(tab.kind), self.active_ui_font, theme),
                tooltip::Position::Bottom,
            );

            container(row![select, close].height(TAB_STRIP_HEIGHT).align_y(Center))
                .width(TAB_WIDTH)
                .height(TAB_STRIP_HEIGHT)
                .style(move |_| {
                    tab_rainbow::tab_container_style(
                        index,
                        is_active,
                        is_dopamine,
                        elapsed_secs,
                        TAB_WIDTH,
                        TAB_STRIP_HEIGHT,
                    )
                })
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

pub(crate) fn tab_style(
    theme: &KegonTheme,
    is_active: bool,
    status: button::Status,
) -> button::Style {
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

fn shell_text_input_style(
    theme: &KegonTheme,
    status: text_input::Status,
    is_invalid: bool,
) -> text_input::Style {
    let is_disabled = matches!(status, text_input::Status::Disabled);
    let border_color = if is_disabled {
        theme.workbench.border
    } else if is_invalid {
        theme.semantic.destructive
    } else {
        match status {
            text_input::Status::Focused { .. } | text_input::Status::Hovered => {
                theme.interaction.accent
            }
            _ => theme.workbench.border,
        }
    };
    let value_color = if is_disabled {
        theme.text.disabled
    } else if is_invalid {
        theme.semantic.destructive
    } else {
        theme.text.primary
    };
    text_input::Style {
        background: theme.workbench.background.into(),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: 4.0.into(),
        },
        icon: if is_disabled {
            theme.text.disabled
        } else {
            theme.text.muted
        },
        placeholder: theme.text.muted,
        value: value_color,
        selection: theme.interaction.selection,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellModeOption {
    mode: ShellMode,
    label: String,
}

impl std::fmt::Display for ShellModeOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellIntegrationOption {
    mode: ShellIntegrationMode,
    label: String,
}

impl std::fmt::Display for ShellIntegrationOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandId, KeyChord, Modifiers};
    use crate::terminal::CommandResult;

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
                ..Default::default()
            },
            ..Default::default()
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

    // --- terminal metric settings in the Settings tab ---

    fn app_with_settings(settings: ApplicationSettings) -> Kegon {
        with_test_settings_path(Kegon::new(Locale::EnUs, settings, None, None, None))
    }

    fn saved_text(app: &Kegon) -> Option<String> {
        std::fs::read_to_string(app.settings_path.as_ref().unwrap()).ok()
    }

    fn type_draft(app: &mut Kegon, metric: TerminalMetric, text: &str) {
        app.update(Message::SettingsMetricDraftChanged(
            metric,
            text.to_string(),
        ));
    }

    #[test]
    fn metric_drafts_do_not_touch_settings_or_the_file_until_applied() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::OpenSettingsTab);
        let before = app.settings.clone();
        let metrics_before = app.terminal_font_config.metrics;

        type_draft(&mut app, TerminalMetric::FontSize, "12");
        type_draft(&mut app, TerminalMetric::CellWidth, "garbage");

        assert_eq!(app.settings, before);
        assert_eq!(app.terminal_font_config.metrics, metrics_before);
        assert!(
            saved_text(&app).is_none(),
            "nothing may be saved while typing"
        );
    }

    #[test]
    fn applying_a_valid_draft_saves_and_updates_the_metrics_for_every_terminal() {
        let mut app = new_test_app(Locale::EnUs);
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);
        app.update(Message::OpenSettingsTab);
        let settings_tab = settings_tab_of(&app).unwrap();
        app.window_size = Size::new(1200.0, 760.0);
        app.resize_terminal_view();

        type_draft(&mut app, TerminalMetric::FontSize, " 12 ");
        type_draft(&mut app, TerminalMetric::CellHeight, "1.2");
        type_draft(&mut app, TerminalMetric::CellWidth, "0.6");
        for metric in TerminalMetric::ALL {
            app.update(Message::SettingsMetricApply(metric));
        }

        let m = app.terminal_font_config.metrics;
        assert!((m.font_size - 16.0).abs() < 1e-4);
        assert!((m.cell_width - 9.6).abs() < 1e-4);
        assert!((m.cell_height - 19.2).abs() < 1e-4);
        let text = saved_text(&app).expect("saved");
        assert!(text.contains("terminal_font_size = 12.0"), "{text}");
        assert!(text.contains("terminal_cell_height = 1.2"), "{text}");
        assert!(text.contains("terminal_cell_width = 0.6"), "{text}");

        // Every session follows with one shared grid, PTY size and cell size; nothing is recreated.
        let (cols, rows) = app.terminal_grid_size();
        for tab in [first, second] {
            let session = app.terminals.get(tab).unwrap().session();
            assert_eq!(session.pty_size(), (cols, rows, 9, 19), "tab {tab:?}");
            use alacritty_terminal::grid::Dimensions;
            let term = session.term().lock();
            assert_eq!(
                (term.columns() as u16, term.screen_lines() as u16),
                (cols, rows)
            );
        }
        assert_eq!(tab_order(&app), [first, second, settings_tab]);
        assert_eq!(app.terminals.len(), 2);
        assert_eq!(
            active_tab(&app),
            settings_tab,
            "applying must not change the selected tab"
        );
    }

    #[test]
    fn invalid_empty_and_unchanged_drafts_are_not_applied() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::OpenSettingsTab);
        let before = app.settings.clone();
        for (metric, text) in [
            (TerminalMetric::FontSize, "0.9"),
            (TerminalMetric::FontSize, "100.1"),
            (TerminalMetric::FontSize, "abc"),
            (TerminalMetric::FontSize, "nan"),
            (TerminalMetric::FontSize, "inf"),
            (TerminalMetric::FontSize, "-12"),
            (TerminalMetric::FontSize, ""),
            (TerminalMetric::CellHeight, "0.49"),
            (TerminalMetric::CellHeight, "3.01"),
            (TerminalMetric::CellWidth, "0.19"),
            (TerminalMetric::CellWidth, "2.01"),
        ] {
            type_draft(&mut app, metric, text);
            app.update(Message::SettingsMetricApply(metric));
            assert_eq!(app.settings, before, "{metric:?} {text:?}");
        }
        assert!(saved_text(&app).is_none());

        // Applying the value that is already saved does nothing either.
        type_draft(&mut app, TerminalMetric::FontSize, "12");
        app.update(Message::SettingsMetricApply(TerminalMetric::FontSize));
        let saved = app.settings.clone();
        app.update(Message::SettingsMetricApply(TerminalMetric::FontSize));
        assert_eq!(app.settings, saved);
    }

    #[test]
    fn field_state_enables_apply_only_for_a_valid_new_value() {
        let mut appearance = AppearanceSettings::default();
        let m = TerminalMetric::FontSize;
        let flags = |s: MetricFieldState| (s.can_apply, s.pending, s.invalid);

        assert_eq!(
            flags(metric_field_state(m, "", &appearance)),
            (false, false, false)
        );
        assert_eq!(
            flags(metric_field_state(m, "12", &appearance)),
            (true, true, false)
        );
        assert_eq!(
            flags(metric_field_state(m, "x", &appearance)),
            (false, true, true)
        );

        appearance.set_metric(m, Some(12.0));
        assert_eq!(
            flags(metric_field_state(m, "12.0", &appearance)),
            (false, false, false)
        );
        assert_eq!(
            flags(metric_field_state(m, "", &appearance)),
            (false, true, false)
        );
    }

    #[test]
    fn other_setting_changes_keep_unapplied_drafts() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::OpenSettingsTab);
        type_draft(&mut app, TerminalMetric::CellHeight, "1.5");

        app.update(Message::SettingsLocaleChanged(LocalePreference::JaJp));
        app.update(Message::SettingsThemeChanged(ThemeId::NightDark));
        app.update(Message::SettingsResetTerminalFontClicked);
        type_draft(&mut app, TerminalMetric::CellWidth, "0.7");
        app.update(Message::SettingsMetricApply(TerminalMetric::CellWidth));

        let drafts = app.metric_drafts.as_ref().unwrap();
        assert_eq!(drafts.get(TerminalMetric::CellHeight), "1.5");
        assert_eq!(
            app.settings.appearance.metric(TerminalMetric::CellHeight),
            None
        );
        assert_eq!(
            app.settings.appearance.metric(TerminalMetric::CellWidth),
            Some(0.7)
        );
    }

    #[test]
    fn closing_settings_discards_drafts_and_reopening_starts_from_saved_values() {
        let mut settings = ApplicationSettings::default();
        settings
            .appearance
            .set_metric(TerminalMetric::FontSize, Some(11.0));
        let mut app = app_with_settings(settings);
        app.update(Message::OpenSettingsTab);
        assert_eq!(
            app.metric_drafts
                .as_ref()
                .unwrap()
                .get(TerminalMetric::FontSize),
            "11"
        );

        type_draft(&mut app, TerminalMetric::FontSize, "20");
        // Switching tabs keeps the draft; closing the Settings tab drops it.
        let terminal = app.workbench.tabs().iter().next().unwrap().id;
        app.update(Message::TabSelected(terminal));
        assert_eq!(
            app.metric_drafts
                .as_ref()
                .unwrap()
                .get(TerminalMetric::FontSize),
            "20"
        );
        let settings_tab = settings_tab_of(&app).unwrap();
        app.update(Message::TabClosed(settings_tab));
        assert!(app.metric_drafts.is_none());
        assert_eq!(
            app.settings.appearance.metric(TerminalMetric::FontSize),
            Some(11.0)
        );

        app.update(Message::OpenSettingsTab);
        assert_eq!(
            app.metric_drafts
                .as_ref()
                .unwrap()
                .get(TerminalMetric::FontSize),
            "11"
        );
    }

    #[test]
    fn reset_applies_immediately_and_restores_the_legacy_metrics() {
        let mut app = new_test_app(Locale::EnUs);
        let legacy = app.terminal_font_config.metrics;
        app.update(Message::OpenSettingsTab);
        type_draft(&mut app, TerminalMetric::FontSize, "20");
        app.update(Message::SettingsMetricApply(TerminalMetric::FontSize));
        type_draft(&mut app, TerminalMetric::CellWidth, "1.0");
        app.update(Message::SettingsMetricApply(TerminalMetric::CellWidth));
        assert_ne!(app.terminal_font_config.metrics, legacy);

        app.update(Message::SettingsMetricReset(TerminalMetric::FontSize));
        app.update(Message::SettingsMetricReset(TerminalMetric::CellWidth));

        assert_eq!(app.terminal_font_config.metrics, legacy);
        assert_eq!(
            app.metric_drafts
                .as_ref()
                .unwrap()
                .get(TerminalMetric::FontSize),
            ""
        );
        let text = saved_text(&app).unwrap();
        assert!(
            !text.contains("terminal_font_size") && !text.contains("terminal_cell_width"),
            "{text}"
        );
    }

    #[test]
    fn ignored_stored_values_are_kept_until_the_user_commits_or_resets() {
        let dir = std::env::temp_dir().join(format!("kegon_metric_keep_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.toml");
        std::fs::write(
            &path,
            "[appearance]\nterminal_font_size = \"big\"\nfuture_key = 1\n",
        )
        .unwrap();
        let (settings, warning) = crate::settings::load_settings(Some(&path));
        assert!(warning.is_none());

        let mut app = app_with_settings(settings);
        app.settings_path = Some(path.clone());
        let legacy = new_test_app(Locale::EnUs).terminal_font_config.metrics;
        assert_eq!(
            app.terminal_font_config.metrics, legacy,
            "an ignored value leaves the legacy look"
        );

        // An unrelated change keeps the original TOML value and unknown keys.
        app.update(Message::SettingsLocaleChanged(LocalePreference::JaJp));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("terminal_font_size = \"big\""), "{text}");
        assert!(text.contains("future_key = 1"), "{text}");

        // A valid commit replaces it.
        app.update(Message::OpenSettingsTab);
        type_draft(&mut app, TerminalMetric::FontSize, "12");
        app.update(Message::SettingsMetricApply(TerminalMetric::FontSize));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("terminal_font_size = 12.0") && !text.contains("big"),
            "{text}"
        );
        assert!(text.contains("future_key = 1"), "{text}");

        // Reset also replaces an ignored value (clears the key).
        let (settings, _) = crate::settings::load_settings(Some(&path));
        let mut app2 = app_with_settings(settings);
        app2.settings_path = Some(path.clone());
        app2.settings.appearance.terminal_font_size.replace(
            crate::settings::NumericSetting::Other(toml::Value::String("bad".into())),
        );
        app2.update(Message::OpenSettingsTab);
        app2.update(Message::SettingsMetricReset(TerminalMetric::FontSize));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("terminal_font_size"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Built-in Glyphs setting ---

    #[test]
    fn built_in_glyphs_are_on_by_default_and_the_toggle_saves_and_applies_at_once() {
        let mut app = new_test_app(Locale::EnUs);
        assert!(app.settings.appearance.builtin_glyphs(), "unset means on");
        let first = active_tab(&app);
        let second = add_terminal_tab(&mut app);
        app.update(Message::OpenSettingsTab);
        app.window_size = Size::new(1200.0, 760.0);
        app.resize_terminal_view();
        let (cols, rows) = app.terminal_grid_size();
        let pty_sizes = |app: &Kegon| {
            [first, second].map(|tab| app.terminals.get(tab).unwrap().session().pty_size())
        };
        let before = pty_sizes(&app);
        // An unapplied metric draft must survive the toggle.
        type_draft(&mut app, TerminalMetric::FontSize, "12");

        app.update(Message::SettingsBuiltinGlyphsToggled(false));
        assert!(!app.settings.appearance.builtin_glyphs());
        let text = saved_text(&app).expect("saved at once");
        assert!(text.contains("terminal_builtin_glyphs = false"), "{text}");
        assert_eq!(app.terminal_grid_size(), (cols, rows), "no metric changed");
        assert_eq!(pty_sizes(&app), before, "no PTY was resized or recreated");
        assert_eq!(app.terminals.len(), 2);
        assert_eq!(
            app.metric_drafts
                .as_ref()
                .unwrap()
                .get(TerminalMetric::FontSize),
            "12",
            "the unapplied draft is kept"
        );
        assert_eq!(
            app.settings.appearance.metric(TerminalMetric::FontSize),
            None
        );
        let _ = app.view(); // draws in both states

        app.update(Message::SettingsBuiltinGlyphsToggled(true));
        assert!(app.settings.appearance.builtin_glyphs());
        let text = saved_text(&app).unwrap();
        assert!(text.contains("terminal_builtin_glyphs = true"), "{text}");
        let _ = app.view();

        // Reset removes the key, which is the default (on).
        app.update(Message::SettingsBuiltinGlyphsToggled(false));
        app.update(Message::SettingsBuiltinGlyphsReset);
        assert!(app.settings.appearance.builtin_glyphs());
        let text = saved_text(&app).unwrap();
        assert!(!text.contains("terminal_builtin_glyphs"), "{text}");
        // Resetting an absent key changes nothing and writes nothing new.
        let before = saved_text(&app);
        app.update(Message::SettingsBuiltinGlyphsReset);
        assert_eq!(saved_text(&app), before);
    }

    #[test]
    fn the_choice_survives_a_restart_and_old_files_mean_on() {
        let dir = std::env::temp_dir().join(format!("kegon_glyphs_restart_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.toml");
        // A file written before the setting existed.
        std::fs::write(
            &path,
            "locale = \"en-US\"\n\n[appearance]\ntheme = \"night-dark\"\nterminal_font_size = 12.0\n",
        )
        .unwrap();
        let (settings, warning) = crate::settings::load_settings(Some(&path));
        assert!(warning.is_none());
        let mut app = app_with_settings(settings);
        app.settings_path = Some(path.clone());
        assert!(app.settings.appearance.builtin_glyphs());
        // Saving something else does not invent the key.
        app.update(Message::SettingsLocaleChanged(LocalePreference::JaJp));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("terminal_builtin_glyphs"), "{text}");
        assert!(text.contains("terminal_font_size = 12.0"), "{text}");

        app.update(Message::SettingsBuiltinGlyphsToggled(false));
        // "Restart": a new application from what is on disk.
        let (settings, warning) = crate::settings::load_settings(Some(&path));
        assert!(warning.is_none());
        let restarted = app_with_settings(settings);
        assert!(!restarted.settings.appearance.builtin_glyphs());
        assert_eq!(
            restarted
                .settings
                .appearance
                .metric(TerminalMetric::FontSize),
            Some(12.0),
            "other settings are untouched"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_value_of_the_wrong_type_counts_as_on_and_is_kept_until_the_user_decides() {
        let dir = std::env::temp_dir().join(format!("kegon_glyphs_invalid_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.toml");
        for bad in ["\"no\"", "0", "[true]", "1.5"] {
            std::fs::write(
                &path,
                format!("[appearance]\nterminal_builtin_glyphs = {bad}\nfuture_key = 1\n"),
            )
            .unwrap();
            let (settings, warning) = crate::settings::load_settings(Some(&path));
            assert!(warning.is_none(), "{bad}");
            let mut app = app_with_settings(settings);
            app.settings_path = Some(path.clone());
            assert!(
                app.settings.appearance.builtin_glyphs(),
                "{bad}: counts as on"
            );
            assert!(
                app.settings
                    .appearance
                    .invalid_builtin_glyphs_text()
                    .is_some()
            );
            assert!(
                app.settings
                    .appearance
                    .metric_diagnostics()
                    .iter()
                    .any(|d| d.contains("terminal_builtin_glyphs")),
                "{bad}"
            );
            for locale in [Locale::EnUs, Locale::JaJp] {
                app.localizer = Localizer::new(locale);
                app.update(Message::OpenSettingsTab);
                let _ = app.view();
            }

            // Saving something else keeps the original value and the unknown key.
            app.update(Message::SettingsLocaleChanged(LocalePreference::EnUs));
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("terminal_builtin_glyphs"), "{bad}: {text}");
            assert!(text.contains("future_key = 1"), "{text}");
            assert!(
                !text.contains("terminal_builtin_glyphs = true"),
                "{bad}: {text}"
            );

            // The user's explicit choice replaces it with a real boolean.
            app.update(Message::SettingsBuiltinGlyphsToggled(false));
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                text.contains("terminal_builtin_glyphs = false"),
                "{bad}: {text}"
            );
            assert!(text.contains("future_key = 1"), "{text}");
            assert!(
                app.settings
                    .appearance
                    .invalid_builtin_glyphs_text()
                    .is_none()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toggling_to_the_value_already_set_writes_nothing() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::SettingsBuiltinGlyphsToggled(false));
        let text = saved_text(&app).unwrap();
        std::fs::remove_file(app.settings_path.as_ref().unwrap()).unwrap();
        app.update(Message::SettingsBuiltinGlyphsToggled(false));
        assert!(
            saved_text(&app).is_none(),
            "no second write for the same value"
        );
        assert!(text.contains("terminal_builtin_glyphs = false"));
    }

    #[test]
    fn a_failed_save_does_not_break_the_running_application() {
        // The settings file's directory cannot be created because a regular file sits there.
        let blocker =
            std::env::temp_dir().join(format!("kegon_glyphs_blocker_{}", std::process::id()));
        std::fs::write(&blocker, "not a directory").unwrap();
        let mut app = new_test_app(Locale::EnUs);
        app.settings_path = Some(blocker.join("settings.toml"));
        app.update(Message::SettingsBuiltinGlyphsToggled(false));
        // The failure is reported on stderr by `commit_settings` like for every setting; the choice still
        // applies for this run and nothing is written.
        assert!(!app.settings.appearance.builtin_glyphs());
        assert!(!blocker.join("settings.toml").exists());
        let _ = app.view();
        let _ = std::fs::remove_file(&blocker);
    }

    #[test]
    fn metric_fields_render_in_both_languages_and_a_narrow_window() {
        for locale in [Locale::EnUs, Locale::JaJp] {
            let mut settings = ApplicationSettings::default();
            settings.appearance.terminal_font_size = Some(crate::settings::NumericSetting::Other(
                toml::Value::String("big".into()),
            ));
            let mut app = app_with_settings(settings);
            app.localizer = Localizer::new(locale);
            app.update(Message::OpenSettingsTab);
            type_draft(&mut app, TerminalMetric::CellWidth, "abc");
            type_draft(&mut app, TerminalMetric::CellHeight, "1.2");
            app.window_size = Size::new(360.0, 300.0);
            let _ = app.view();
        }
    }

    #[test]
    fn scale_factor_is_taken_from_the_window_and_bad_values_are_ignored() {
        let mut app = new_test_app(Locale::EnUs);
        assert_eq!(app.scale_factor, 1.0);

        app.update(Message::ScaleFactorChanged(1.25));
        assert_eq!(app.scale_factor, 1.25);
        app.handle_iced_event(Event::Window(iced::window::Event::Rescaled(1.5)));
        assert_eq!(app.scale_factor, 1.5);

        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            app.update(Message::ScaleFactorChanged(bad));
            assert_eq!(app.scale_factor, 1.5, "{bad}");
        }
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

    // --- Dopamine Mode tests ---

    #[test]
    fn dopamine_mode_default_off_and_settings_toggle_and_reset() {
        let mut app = new_test_app(Locale::EnUs);
        assert!(!app.settings.dopamine.enabled());
        assert!(!app.dopamine.is_enabled());

        // Toggle on
        app.update(Message::SettingsDopamineToggled(true));
        assert!(app.settings.dopamine.enabled());
        assert!(app.dopamine.is_enabled());
        let text = saved_text(&app).expect("settings saved on toggle");
        assert!(text.contains("enabled = true"), "{text}");

        // Toggle off
        app.update(Message::SettingsDopamineToggled(false));
        assert!(!app.settings.dopamine.enabled());
        assert!(!app.dopamine.is_enabled());
        let text = saved_text(&app).unwrap();
        assert!(text.contains("enabled = false"), "{text}");

        // Switch sound profile
        app.update(Message::SettingsDopamineSoundChanged(
            DopamineSoundKind::Crack,
        ));
        assert_eq!(app.settings.dopamine.sound(), DopamineSoundKind::Crack);
        assert_eq!(app.dopamine.audio().sound_kind(), DopamineSoundKind::Crack);
        let text = saved_text(&app).unwrap();
        assert!(text.contains("sound = \"crack\""), "{text}");

        // Preview sound
        app.update(Message::SettingsDopamineSoundPreview);

        // Reset clears keys back to defaults
        app.update(Message::SettingsDopamineReset);
        assert!(!app.settings.dopamine.enabled());
        assert!(!app.dopamine.is_enabled());
        assert_eq!(app.settings.dopamine.sound(), DopamineSoundKind::Zap);
        let text = saved_text(&app).unwrap();
        assert!(!text.contains("enabled = true"), "{text}");
        assert!(!text.contains("sound ="), "{text}");
    }

    #[test]
    fn dopamine_backspace_triggers_particles_when_enabled() {
        let mut app = new_test_app(Locale::EnUs);
        app.terminal_focused = true;
        app.update(Message::SettingsDopamineToggled(true));
        let tab = active_tab(&app);

        assert!(!app.dopamine.has_active_particles());

        // Send Backspace key press
        app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Backspace),
            modified_key: keyboard::Key::Named(keyboard::key::Named::Backspace),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Backspace),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            repeat: false,
            text: None,
        }));

        assert!(app.dopamine.has_active_particles());
        let particles = app.dopamine.particles_for_tab(tab);
        assert_eq!(
            particles.len(),
            crate::terminal::dopamine::BACKSPACE_BURST_COUNT
        );

        let kinds: Vec<_> = particles.iter().map(|p| p.kind).collect();
        assert!(kinds.contains(&crate::terminal::dopamine::ParticleKind::Flash));
        assert!(kinds.contains(&crate::terminal::dopamine::ParticleKind::Spark));
        assert!(kinds.contains(&crate::terminal::dopamine::ParticleKind::Mote));

        // Advance simulation: particles must accelerate down past the viewport
        app.update(Message::DopamineTicked(
            std::time::Instant::now() + std::time::Duration::from_millis(500),
        ));
        assert!(
            app.dopamine.has_active_particles(),
            "Motes and long sparks must remain alive and falling across the screen"
        );

        // Advance simulation past safe viewport exit limit
        app.update(Message::DopamineTicked(
            std::time::Instant::now() + std::time::Duration::from_millis(3000),
        ));
        assert!(
            !app.dopamine.has_active_particles(),
            "All particles must be culled once exiting the viewport"
        );
        assert_eq!(app.dopamine.particles_for_tab(tab).len(), 0);
    }

    #[test]
    fn dopamine_backspace_ignored_when_disabled_or_modal_open() {
        let mut app = new_test_app(Locale::EnUs);
        app.terminal_focused = true;
        let tab = active_tab(&app);

        // 1. Dopamine mode is OFF by default -> No particles
        app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Backspace),
            modified_key: keyboard::Key::Named(keyboard::key::Named::Backspace),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Backspace),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            repeat: false,
            text: None,
        }));
        assert!(!app.dopamine.has_active_particles());
        assert_eq!(app.dopamine.particles_for_tab(tab).len(), 0);

        // 2. Enable dopamine mode, but open a modal dialog -> No particles
        app.update(Message::SettingsDopamineToggled(true));
        let dialog = ConfirmationDialog::builder(
            DialogKind::Question,
            MessageKey::DialogSmokeQuestionTitle,
            MessageKey::DialogSmokeQuestionMessage,
        )
        .build();
        app.open_confirmation_dialog(dialog);
        assert!(app.is_modal_open());

        app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Backspace),
            modified_key: keyboard::Key::Named(keyboard::key::Named::Backspace),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Backspace),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            repeat: false,
            text: None,
        }));
        assert!(!app.dopamine.has_active_particles());
    }

    #[test]
    fn dopamine_tab_close_clears_particles() {
        let mut app = new_test_app(Locale::EnUs);
        app.terminal_focused = true;
        app.update(Message::SettingsDopamineToggled(true));
        let tab = active_tab(&app);

        app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Backspace),
            modified_key: keyboard::Key::Named(keyboard::key::Named::Backspace),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Backspace),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            repeat: false,
            text: None,
        }));
        assert!(app.dopamine.has_active_particles());

        // Close the tab
        app.update(Message::TabClosed(tab));
        assert_eq!(app.dopamine.particles_for_tab(tab).len(), 0);
    }

    #[test]
    fn terminal_shell_settings_messages_update_and_persist() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::SettingsShellModeChanged(ShellMode::Custom));
        app.update(Message::SettingsShellPathChanged(
            r"C:\Windows\System32\cmd.exe".to_string(),
        ));
        app.update(Message::SettingsShellIntegrationChanged(
            ShellIntegrationMode::Disabled,
        ));

        assert_eq!(app.settings.terminal.shell_mode, ShellMode::Custom);
        assert_eq!(
            app.settings.terminal.shell_path,
            r"C:\Windows\System32\cmd.exe"
        );
        assert_eq!(
            app.settings.terminal.shell_integration,
            ShellIntegrationMode::Disabled
        );

        let path = app.settings_path.as_ref().unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("shell_mode = \"custom\""), "{text}");
        assert!(text.contains(r"C:\Windows\System32\cmd.exe"), "{text}");
        assert!(text.contains("shell_integration = \"disabled\""), "{text}");
    }

    #[test]
    fn terminal_tab_spawning_with_custom_shell() {
        let mut app = new_test_app(Locale::EnUs);
        app.update(Message::SettingsShellModeChanged(ShellMode::Custom));
        app.update(Message::SettingsShellPathChanged(
            r"C:\Windows\System32\cmd.exe".to_string(),
        ));

        let initial_tabs = app.workbench.tabs().len();
        let new_tab = app.open_terminal_tab();
        assert!(
            new_tab.is_some(),
            "Spawning valid custom shell should succeed"
        );
        assert_eq!(app.workbench.tabs().len(), initial_tabs + 1);
        assert!(app.terminals.get(new_tab.unwrap()).is_some());
    }

    #[test]
    fn terminal_tab_spawning_fails_safely_with_invalid_custom_shell() {
        let mut app = new_test_app(Locale::EnUs);
        let initial_tabs = app.workbench.tabs().len();
        let initial_active = app.workbench.tabs().active().map(|t| t.id);

        app.update(Message::SettingsShellModeChanged(ShellMode::Custom));
        app.update(Message::SettingsShellPathChanged(
            r"C:\Windows\System32\definitely_not_a_real_shell_xyz.exe".to_string(),
        ));

        let failed_tab = app.open_terminal_tab();
        assert!(
            failed_tab.is_none(),
            "Invalid shell path must fail cleanly without falling back to auto"
        );
        assert_eq!(
            app.workbench.tabs().len(),
            initial_tabs,
            "Failed tab must be removed"
        );
        assert_eq!(
            app.workbench.tabs().active().map(|t| t.id),
            initial_active,
            "Active tab must remain unchanged"
        );
    }

    #[test]
    fn existing_tabs_remain_unaffected_when_shell_settings_change() {
        let mut app = new_test_app(Locale::EnUs);
        let initial_tab = app.workbench.tabs().active().map(|t| t.id).unwrap();
        assert!(app.terminals.get(initial_tab).is_some());

        // Change settings to invalid custom shell
        app.update(Message::SettingsShellModeChanged(ShellMode::Custom));
        app.update(Message::SettingsShellPathChanged(
            r"C:\invalid\path.exe".to_string(),
        ));

        // Existing tab session is completely unaffected
        assert!(
            app.terminals.get(initial_tab).is_some(),
            "Existing running tab must not be touched by settings change"
        );
    }

    #[test]
    fn terminal_tab_tooltip_presence_and_settings_tab_exclusion() {
        let mut app = new_test_app(Locale::EnUs);
        let terminal_tab = app.workbench.tabs().active().unwrap().id;

        // Terminal tab must return a tooltip element
        assert!(app.terminal_tab_tooltip(terminal_tab).is_some());

        // Open settings tab
        app.update(Message::OpenSettingsTab);
        let settings_tab = app.workbench.tabs().active().unwrap().id;
        assert_eq!(
            app.workbench.tabs().get(settings_tab).unwrap().kind,
            TabKind::Settings
        );

        // Settings tab has no shell integration tooltip
        assert!(app.terminal_tab_tooltip(settings_tab).is_none());
    }

    #[test]
    fn terminal_tab_tooltip_lifecycle_and_tab_isolation() {
        use crate::terminal::integration::Osc133Marker;

        let mut app = new_test_app(Locale::EnUs);
        let tab0 = app.workbench.tabs().active().unwrap().id;

        // Open second tab
        let tab1 = app.open_terminal_tab().unwrap();

        // Initial tooltip exists for both tabs
        assert!(app.terminal_tab_tooltip(tab0).is_some());
        assert!(app.terminal_tab_tooltip(tab1).is_some());

        // Tab 0 receives OSC 133 markers
        app.update(Message::TerminalEventReceived(
            tab0,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab0,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));

        // Tab 0 status is ActivePromptOnly, tab 1 is still in initial state
        let entry0 = app.terminals.get(tab0).unwrap();
        let entry1 = app.terminals.get(tab1).unwrap();
        assert_eq!(
            entry0.shell_integration_status(),
            ShellIntegrationStatus::ActivePromptOnly
        );
        assert_eq!(
            entry1.shell_integration_status(),
            ShellIntegrationStatus::Waiting
        );

        // Tab 0 executes a command and finishes with code 0
        app.update(Message::TerminalEventReceived(
            tab0,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab0,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
        ));

        let entry0 = app.terminals.get(tab0).unwrap();
        assert_eq!(
            entry0.shell_integration_status(),
            ShellIntegrationStatus::Active
        );
        assert_eq!(
            entry0.command_state_machine().last_result(),
            Some(CommandResult::Success)
        );

        // Tab 1 remains completely isolated
        let entry1 = app.terminals.get(tab1).unwrap();
        assert_eq!(
            entry1.shell_integration_status(),
            ShellIntegrationStatus::Waiting
        );
        assert_eq!(entry1.command_state_machine().last_result(), None);

        // Closing tab 0 cleans up registry and tooltip returns None
        app.update(Message::TabClosed(tab0));
        assert!(app.terminals.get(tab0).is_none());
        assert!(app.terminal_tab_tooltip(tab0).is_none());
    }

    #[test]
    fn terminal_tab_tooltip_japanese_locale_rendering() {
        use crate::terminal::integration::Osc133Marker;

        let mut app = new_test_app(Locale::JaJp);
        let tab = app.workbench.tabs().active().unwrap().id;

        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(42))),
        ));

        let lines = app.terminal_tab_tooltip_lines(tab).unwrap();
        assert_eq!(lines[1], "Shell Integration: 有効");
        assert_eq!(lines[2], "コマンド状態: 完了");
        assert_eq!(lines[3], "シェル報告成否: 不明");
        assert_eq!(lines[4], "OSC報告終了コード: 42");

        let entry = app.terminals.get(tab).unwrap();
        assert_eq!(
            entry.shell_integration_status(),
            ShellIntegrationStatus::Active
        );
        assert_eq!(
            entry.command_state_machine().last_result(),
            Some(CommandResult::Failure(42))
        );
    }

    #[test]
    fn terminal_tab_tooltip_japanese_matched_success_and_failure() {
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};

        let mut app = new_test_app(Locale::JaJp);
        let tab = app.workbench.tabs().active().unwrap().id;

        // Fresh tab: status waiting, no command executed
        let init_lines = app.terminal_tab_tooltip_lines(tab).unwrap();
        assert_eq!(init_lines[1], "Shell Integration: 待機中");

        // Set expected SID on tab's state machine
        app.terminals
            .command_state_machine_mut(tab)
            .unwrap()
            .set_expected_session_id(Some("TEST_SID".into()));

        // Command 1: Matched success with codeless D (PowerShell standard)
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "TEST_SID".into(),
                    seq: 1,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));

        // While executing: Command State is Executing
        let executing_lines = app.terminal_tab_tooltip_lines(tab).unwrap();
        assert_eq!(executing_lines[2], "コマンド状態: 実行中");
        assert_eq!(executing_lines[3], "シェル報告成否: —");
        assert_eq!(executing_lines[4], "OSC報告終了コード: —");

        // Independent metadata (success) + codeless D
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "TEST_SID".into(),
                seq: 1,
                shell_status: ShellReportedStatus::Success,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));

        let success_lines = app.terminal_tab_tooltip_lines(tab).unwrap();
        assert_eq!(success_lines[2], "コマンド状態: 完了");
        assert_eq!(success_lines[3], "シェル報告成否: 成功");
        assert_eq!(success_lines[4], "OSC報告終了コード: —");

        // Command 2: Matched failure with codeless D
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "TEST_SID".into(),
                    seq: 2,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "TEST_SID".into(),
                seq: 2,
                shell_status: ShellReportedStatus::Failure,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));

        let failure_lines = app.terminal_tab_tooltip_lines(tab).unwrap();
        assert_eq!(failure_lines[2], "コマンド状態: 完了");
        assert_eq!(failure_lines[3], "シェル報告成否: 失敗");
        assert_eq!(failure_lines[4], "OSC報告終了コード: —");

        // Command 3: Mismatched seq -> Unknown
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "TEST_SID".into(),
                    seq: 3,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "TEST_SID".into(),
                seq: 999,
                shell_status: ShellReportedStatus::Success,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));

        let mismatch_lines = app.terminal_tab_tooltip_lines(tab).unwrap();
        assert_eq!(mismatch_lines[2], "コマンド状態: 完了");
        assert_eq!(mismatch_lines[3], "シェル報告成否: 不明");
        assert_eq!(mismatch_lines[4], "OSC報告終了コード: —");
    }

    #[test]
    fn terminal_tab_tooltip_english_matched_rendering() {
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};

        let mut app = new_test_app(Locale::EnUs);
        let tab = app.workbench.tabs().active().unwrap().id;

        app.terminals
            .command_state_machine_mut(tab)
            .unwrap()
            .set_expected_session_id(Some("EN_SID".into()));

        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "EN_SID".into(),
                    seq: 1,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "EN_SID".into(),
                seq: 1,
                shell_status: ShellReportedStatus::Success,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
        ));

        let lines = app.terminal_tab_tooltip_lines(tab).unwrap();
        assert_eq!(lines[1], "Shell Integration: Active");
        assert_eq!(lines[2], "Command State: Finished");
        assert_eq!(lines[3], "Shell-reported status: Success");
        assert_eq!(lines[4], "OSC-reported exit code: 0");

        // Verify typed result query on registry
        let result = app.terminals.last_execution_result(tab).unwrap();
        assert_eq!(result.shell_status, ShellReportedStatus::Success);
        assert_eq!(result.osc133_exit, Some(0));
        assert!(result.correlation.is_matched());
    }

    #[test]
    fn dopamine_command_completion_border_success_rainbow_and_failure_red() {
        use crate::terminal::BorderKind;
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};

        let mut app = new_test_app(Locale::JaJp);
        app.update(Message::SettingsDopamineToggled(true));
        let tab = active_tab(&app);

        // Ensure session has expected session ID
        app.terminals
            .command_state_machine_mut(tab)
            .unwrap()
            .set_expected_session_id(Some("TEST_SID".into()));

        // --- Command 1: Success -> Rainbow ---
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "TEST_SID".into(),
                    seq: 1,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "TEST_SID".into(),
                seq: 1,
                shell_status: ShellReportedStatus::Success,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));

        // Border effect must be Rainbow with 100% initial opacity
        let border = app
            .dopamine
            .border_for_tab(tab, std::time::Instant::now())
            .unwrap();
        assert_eq!(border.kind, BorderKind::Rainbow);
        assert!((border.alpha - 1.0).abs() < 1e-4);
        assert!(app.dopamine.has_active_animations());

        // --- Command 2: Failure -> Red ---
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "TEST_SID".into(),
                    seq: 2,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "TEST_SID".into(),
                seq: 2,
                shell_status: ShellReportedStatus::Failure,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));

        // Border effect must be replaced with Red at 100% opacity
        let border2 = app
            .dopamine
            .border_for_tab(tab, std::time::Instant::now())
            .unwrap();
        assert_eq!(border2.kind, BorderKind::Red);
        assert!((border2.alpha - 1.0).abs() < 1e-4);
    }

    #[test]
    fn dopamine_command_completion_border_negative_conditions_no_effect() {
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};

        let mut app = new_test_app(Locale::JaJp);
        app.update(Message::SettingsDopamineToggled(true));
        let tab = active_tab(&app);
        app.terminals
            .command_state_machine_mut(tab)
            .unwrap()
            .set_expected_session_id(Some("TEST_SID".into()));

        // Case 1: Legacy OSC 133 D with exit code 0 (Unprovided metadata) -> No effect
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
        ));
        assert!(
            app.dopamine
                .border_for_tab(tab, std::time::Instant::now())
                .is_none()
        );

        // Case 2: Mismatched correlation (seq jumped to 999) -> No effect
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "TEST_SID".into(),
                    seq: 1,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "TEST_SID".into(),
                seq: 999,
                shell_status: ShellReportedStatus::Success,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));
        assert!(
            app.dopamine
                .border_for_tab(tab, std::time::Instant::now())
                .is_none()
        );

        // Case 3: MissingD (interrupted by prompt A/B) -> No effect
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "TEST_SID".into(),
                    seq: 2,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        assert!(
            app.dopamine
                .border_for_tab(tab, std::time::Instant::now())
                .is_none()
        );

        // Case 4: Dopamine Mode OFF -> No effect
        app.update(Message::SettingsDopamineToggled(false));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "TEST_SID".into(),
                    seq: 3,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "TEST_SID".into(),
                seq: 3,
                shell_status: ShellReportedStatus::Success,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));
        assert!(
            app.dopamine
                .border_for_tab(tab, std::time::Instant::now())
                .is_none()
        );
    }

    #[test]
    fn dopamine_command_completion_border_tab_isolation_fade_and_close() {
        use crate::terminal::BorderKind;
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};
        use std::time::{Duration, Instant};

        let mut app = new_test_app(Locale::JaJp);
        app.update(Message::SettingsDopamineToggled(true));
        let tab1 = active_tab(&app);

        // Open a second terminal tab
        app.update(Message::NewTerminalTab);
        let tab2 = active_tab(&app);
        assert_ne!(tab1, tab2);

        app.terminals
            .command_state_machine_mut(tab1)
            .unwrap()
            .set_expected_session_id(Some("SID_1".into()));

        // Trigger Success on tab1
        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "SID_1".into(),
                    seq: 1,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "SID_1".into(),
                seq: 1,
                shell_status: ShellReportedStatus::Success,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));

        let t0 = Instant::now();
        // Tab isolation: tab1 has Rainbow border, tab2 has None
        assert_eq!(
            app.dopamine.border_for_tab(tab1, t0).map(|b| b.kind),
            Some(BorderKind::Rainbow)
        );
        assert!(app.dopamine.border_for_tab(tab2, t0).is_none());

        // Fade out check: after 450ms, border is gone
        let t_fade = t0 + Duration::from_millis(450);
        app.update(Message::DopamineTicked(t_fade));
        assert!(app.dopamine.border_for_tab(tab1, t_fade).is_none());
        assert!(!app.dopamine.has_active_animations());

        // Close tab clears border state
        app.update(Message::TabClosed(tab1));
        assert!(app.dopamine.border_for_tab(tab1, t0).is_none());
    }

    #[test]
    fn dopamine_combo_and_gauge_input_flow_and_command_success_multiplier() {
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};
        use std::time::{Duration, Instant};

        let mut app = new_test_app(Locale::JaJp);
        app.terminal_focused = true;
        app.update(Message::SettingsDopamineToggled(true));
        let tab = active_tab(&app);

        let type_char = |app: &mut Kegon, s: &str| {
            app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(s.into()),
                modified_key: keyboard::Key::Character(s.into()),
                physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyA),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                repeat: false,
                text: Some(s.into()),
            }));
        };

        let type_space = |app: &mut Kegon| {
            app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Space),
                modified_key: keyboard::Key::Named(keyboard::key::Named::Space),
                physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Space),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                repeat: false,
                text: Some(" ".into()),
            }));
        };

        let type_enter = |app: &mut Kegon| {
            app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Enter),
                modified_key: keyboard::Key::Named(keyboard::key::Named::Enter),
                physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Enter),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                repeat: false,
                text: None,
            }));
        };

        let t0 = Instant::now();

        // 1. Initial state
        assert_eq!(app.dopamine.combo_for_tab(tab, t0), 0);
        assert_eq!(app.dopamine.gauge_for_tab(tab), 0);

        // 2. Type "cargo" -> token 1 (combo = 1)
        type_char(&mut app, "c");
        type_char(&mut app, "a");
        type_char(&mut app, "r");
        type_char(&mut app, "g");
        type_char(&mut app, "o");
        assert_eq!(app.dopamine.combo_for_tab(tab, t0), 1);

        // 3. Consecutive spaces do not increment combo
        type_space(&mut app);
        type_space(&mut app);
        type_space(&mut app);
        assert_eq!(app.dopamine.combo_for_tab(tab, t0), 1);

        // 4. Type "test" -> token 2 (combo = 2)
        type_char(&mut app, "t");
        type_char(&mut app, "e");
        type_char(&mut app, "s");
        type_char(&mut app, "t");
        assert_eq!(app.dopamine.combo_for_tab(tab, t0), 2);

        // 5. Enter maintains combo count
        type_enter(&mut app);
        assert_eq!(app.dopamine.combo_for_tab(tab, t0), 2);

        // 6. Verified command completion Success doubles combo (2 -> 4)
        app.terminals
            .command_state_machine_mut(tab)
            .unwrap()
            .set_expected_session_id(Some("SID_TEST".into()));

        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "SID_TEST".into(),
                    seq: 1,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "SID_TEST".into(),
                seq: 1,
                shell_status: ShellReportedStatus::Success,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));

        let t_finished = Instant::now();
        assert_eq!(app.dopamine.combo_for_tab(tab, t_finished), 4);
        assert_eq!(app.dopamine.gauge_for_tab(tab), 0);

        // 7. Timeout at 1,000ms transfers 4 points to gauge
        let t_timeout = t_finished + Duration::from_millis(1000);
        app.update(Message::DopamineTicked(t_timeout));
        assert_eq!(app.dopamine.combo_for_tab(tab, t_timeout), 0);
        assert_eq!(app.dopamine.gauge_for_tab(tab), 4);
    }

    #[test]
    fn dopamine_combo_failure_and_tab_isolation_and_toggle() {
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};
        use std::time::{Duration, Instant};

        let mut app = new_test_app(Locale::JaJp);
        app.terminal_focused = true;
        app.update(Message::SettingsDopamineToggled(true));
        let tab1 = active_tab(&app);

        // Type on tab1: 2 tokens
        app.dopamine
            .on_text_input(tab1, "cargo test", Instant::now());
        assert_eq!(app.dopamine.combo_for_tab(tab1, Instant::now()), 2);

        // Command fails -> combo is NOT doubled
        app.terminals
            .command_state_machine_mut(tab1)
            .unwrap()
            .set_expected_session_id(Some("SID_1".into()));

        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "SID_1".into(),
                    seq: 1,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::IndependentMetadata(Osc133Metadata {
                version: 1,
                session_id: "SID_1".into(),
                seq: 1,
                shell_status: ShellReportedStatus::Failure,
            })),
        ));
        app.update(Message::TerminalEventReceived(
            tab1,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(None)),
        ));

        // Combo remains 2 (not doubled)
        assert_eq!(app.dopamine.combo_for_tab(tab1, Instant::now()), 2);

        // Open a second tab: tab2 has 0 combo and 0 gauge
        app.update(Message::NewTerminalTab);
        let tab2 = active_tab(&app);
        assert_ne!(tab1, tab2);
        assert_eq!(app.dopamine.combo_for_tab(tab2, Instant::now()), 0);
        assert_eq!(app.dopamine.gauge_for_tab(tab2), 0);

        // Type on tab2: 1 token
        app.dopamine.on_text_input(tab2, "git", Instant::now());
        assert_eq!(app.dopamine.combo_for_tab(tab2, Instant::now()), 1);
        assert_eq!(app.dopamine.combo_for_tab(tab1, Instant::now()), 2);

        // Tick past timeout: tab1 gauge becomes 2, tab2 gauge becomes 1
        let t_timeout = Instant::now() + Duration::from_millis(1500);
        app.update(Message::DopamineTicked(t_timeout));
        assert_eq!(app.dopamine.gauge_for_tab(tab1), 2);
        assert_eq!(app.dopamine.gauge_for_tab(tab2), 1);

        // Close tab1: clears tab1's state, tab2 is untouched
        app.update(Message::TabClosed(tab1));
        assert_eq!(app.dopamine.gauge_for_tab(tab1), 0);
        assert_eq!(app.dopamine.gauge_for_tab(tab2), 1);

        // Toggle dopamine OFF: clears all dopamine combo & gauge states
        app.update(Message::SettingsDopamineToggled(false));
        assert_eq!(app.dopamine.gauge_for_tab(tab2), 0);
        assert_eq!(app.dopamine.combo_for_tab(tab2, Instant::now()), 0);
    }

    #[test]
    fn dopamine_paste_and_ime_commit_tokens() {
        use iced::advanced::input_method::Event as ImeEvent;
        use std::time::{Duration, Instant};

        let mut app = new_test_app(Locale::EnUs);
        app.terminal_focused = true;
        app.update(Message::SettingsDopamineToggled(true));
        let tab = active_tab(&app);
        let t0 = Instant::now();

        // 1. IME commit with multiple tokens
        app.update(Message::IcedEventReceived(Event::InputMethod(
            ImeEvent::Commit(String::from("echo 日本語 テスト")),
        )));
        assert_eq!(app.dopamine.combo_for_tab(tab, t0), 3);

        // 2. IME preedit does not add combo and does NOT refresh clock (F-3)
        app.update(Message::IcedEventReceived(Event::InputMethod(
            ImeEvent::Preedit(String::from("へんかん"), None),
        )));
        assert_eq!(app.dopamine.combo_for_tab(tab, t0), 3);
        let t_expired = t0 + Duration::from_millis(1001);
        assert_eq!(app.dopamine.combo_for_tab(tab, t_expired), 0);
    }

    #[test]
    fn dopamine_shortcuts_and_anomaly_recovery() {
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};
        use std::time::Instant;

        let mut app = new_test_app(Locale::JaJp);
        app.terminal_focused = true;
        app.update(Message::SettingsDopamineToggled(true));
        let tab = active_tab(&app);

        // 1. Non-character keys (e.g. ArrowUp) do not create combo or clock (F-2)
        app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::ArrowUp),
            modified_key: keyboard::Key::Named(keyboard::key::Named::ArrowUp),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::ArrowUp),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            repeat: false,
            text: None,
        }));
        assert_eq!(app.dopamine.combo_for_tab(tab, Instant::now()), 0);

        // 2. Ctrl/Alt shortcuts (e.g. Ctrl+C) do not create combo or clock (F-5)
        app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character("c".into()),
            modified_key: keyboard::Key::Character("c".into()),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyC),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::CTRL,
            repeat: false,
            text: Some("\x03".into()),
        }));
        assert_eq!(app.dopamine.combo_for_tab(tab, Instant::now()), 0);

        // 3. Normal typing starts combo
        app.dopamine.on_char_input(tab, 'a', Instant::now());
        assert_eq!(app.dopamine.combo_for_tab(tab, Instant::now()), 1);

        // 4. CommandStarted (C) pauses clock during execution
        app.terminals
            .command_state_machine_mut(tab)
            .unwrap()
            .set_expected_session_id(Some("SID_ANOMALY".into()));

        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "SID_ANOMALY".into(),
                    seq: 1,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));

        // Animation subscriptions are quiet while command runs (F-7)
        assert!(!app.dopamine.has_active_combos(Instant::now()));

        // 5. Anomaly: PromptStarted (A) arrives without D (MissingD)
        // Discards combo to 0 without transferring to gauge
        app.update(Message::TerminalEventReceived(
            tab,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        assert_eq!(app.dopamine.combo_for_tab(tab, Instant::now()), 0);
        assert_eq!(app.dopamine.gauge_for_tab(tab), 0);
    }

    #[test]
    fn dopamine_n1_subscription_and_n2_shell_exited_integration() {
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};
        use std::time::{Duration, Instant};

        let mut app = new_test_app(Locale::JaJp);
        app.terminal_focused = true;
        app.update(Message::SettingsDopamineToggled(true));
        let tab1 = active_tab(&app);
        let t0 = Instant::now();

        // 1. Tab 1 Running with combo = 2
        app.dopamine.on_text_input(tab1, "cargo test", t0);
        assert_eq!(app.dopamine.combo_for_tab(tab1, t0), 2);

        // Open Tab 2
        app.update(Message::NewTerminalTab);
        let tab2 = active_tab(&app);
        assert_ne!(tab1, tab2);

        // 2. Tab 2 starts a command -> Paused
        app.dopamine.on_text_input(tab2, "git", t0);
        app.terminals
            .command_state_machine_mut(tab2)
            .unwrap()
            .set_expected_session_id(Some("SID_T2".into()));
        app.update(Message::TerminalEventReceived(
            tab2,
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        ));
        app.update(Message::TerminalEventReceived(
            tab2,
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                Osc133Metadata {
                    version: 1,
                    session_id: "SID_T2".into(),
                    seq: 1,
                    shell_status: ShellReportedStatus::Unknown,
                },
            )),
        ));
        assert_eq!(app.dopamine.combo_for_tab(tab2, t0), 1);

        // N-1 Integration Check:
        // Advance time to 1,500ms (past 1,000ms deadline) WITHOUT calling DopamineTicked yet.
        let t_past = t0 + Duration::from_millis(1500);
        // Subscription must STILL be active because Tab 1 hasn't been ticked to Idle!
        assert!(app.dopamine.has_active_combos(t_past));
        assert!(app.dopamine.has_active_animations());
        assert_eq!(app.dopamine.combo_for_tab(tab1, t_past), 0); // read query shows expired
        assert_eq!(app.dopamine.effective_gauge_for_tab(tab1, t_past), 2); // points pending
        assert_eq!(app.dopamine.gauge_for_tab(tab1), 0); // not yet transferred

        // Frame ticks -> transfers Tab 1 points to gauge
        app.update(Message::DopamineTicked(t_past));
        assert_eq!(app.dopamine.gauge_for_tab(tab1), 2);
        assert_eq!(app.dopamine.combo_for_tab(tab1, t_past), 0);

        // Tab 1 is now Idle, Tab 2 is Paused -> NO frames required!
        assert!(!app.dopamine.has_active_combos(t_past));

        // N-2 Integration Check:
        // Tab 2 shell exits while command is Paused
        app.update(Message::TerminalEventReceived(
            tab2,
            TerminalEvent::ChildExit(Some(0)),
        ));

        // Tab 2 combo and gauge are completely discarded without transfer
        assert_eq!(app.dopamine.combo_for_tab(tab2, t_past), 0);
        assert_eq!(app.dopamine.gauge_for_tab(tab2), 0);
        assert!(app.dopamine.drain_combo_events(tab2).is_empty());

        // Tab 1 state is completely preserved
        assert_eq!(app.dopamine.gauge_for_tab(tab1), 2);

        // TabClosed on Tab 2 closes cleanly
        app.update(Message::TabClosed(tab2));
        assert_eq!(app.dopamine.gauge_for_tab(tab1), 2);
    }

    #[test]
    fn dopamine_running_arrow_and_ime_preedit_do_not_extend_clock() {
        use iced::advanced::input_method::Event as ImeEvent;
        use std::time::{Duration, Instant};

        let mut app = new_test_app(Locale::JaJp);
        app.terminal_focused = true;
        app.update(Message::SettingsDopamineToggled(true));
        let tab = active_tab(&app);
        // Start combo at a fixed past timestamp (800ms ago)
        let t0 = Instant::now() - Duration::from_millis(800);
        app.dopamine.on_char_input(tab, 'c', t0);
        assert_eq!(app.dopamine.combo_for_tab(tab, t0), 1);

        // 2. Press ArrowLeft/ArrowRight at t0 + 500ms: must NOT refresh clock
        app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::ArrowLeft),
            modified_key: keyboard::Key::Named(keyboard::key::Named::ArrowLeft),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::ArrowLeft),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            repeat: false,
            text: None,
        }));
        app.handle_iced_event(Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::ArrowRight),
            modified_key: keyboard::Key::Named(keyboard::key::Named::ArrowRight),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::ArrowRight),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            repeat: false,
            text: None,
        }));

        // 3. IME Preedit at t0 + 700ms: must NOT refresh clock
        app.update(Message::IcedEventReceived(Event::InputMethod(
            ImeEvent::Preedit(String::from("へんかん"), None),
        )));

        // 4. At t0 + 1001ms (from original 'c' input): combo has expired!
        let t_expired = t0 + Duration::from_millis(1001);
        assert_eq!(app.dopamine.combo_for_tab(tab, t_expired), 0);
        assert_eq!(app.dopamine.effective_gauge_for_tab(tab, t_expired), 1);

        // Tick transfers 1 point
        app.update(Message::DopamineTicked(t_expired));
        assert_eq!(app.dopamine.gauge_for_tab(tab), 1);
        assert_eq!(app.dopamine.combo_for_tab(tab, t_expired), 0);
    }

    #[test]
    fn dopamine_slice2_rainbow_tabs_active_inactive_styling_and_toggle() {
        let mut app = new_test_app(Locale::JaJp);
        let tab1 = active_tab(&app);
        app.update(Message::NewTerminalTab);
        let tab2 = active_tab(&app);
        assert_ne!(tab1, tab2);

        // Select tab1
        app.update(Message::TabSelected(tab1));
        assert_eq!(active_tab(&app), tab1);

        // 1. Initial state: Dopamine OFF -> standard theme styling
        assert!(!app.dopamine.is_enabled());
        let theme = app.active_theme;
        assert_eq!(
            tab_rainbow::tab_title_color(true, false, theme),
            theme.text.emphasis
        );
        assert_eq!(
            tab_rainbow::tab_title_color(false, false, theme),
            theme.text.muted
        );
        assert_eq!(
            tab_rainbow::tab_close_icon_color(true, false, false, theme),
            theme.icons.active
        );
        assert_eq!(
            tab_rainbow::tab_close_icon_color(false, false, false, theme),
            theme.icons.inactive
        );

        // 2. Toggle Dopamine Mode ON
        app.update(Message::SettingsDopamineToggled(true));
        assert!(app.dopamine.is_enabled());

        // tab1 is active: black title and black close icon
        assert_eq!(active_tab(&app), tab1);
        assert_eq!(
            tab_rainbow::tab_title_color(true, true, theme),
            Color::BLACK
        );
        assert_eq!(
            tab_rainbow::tab_close_icon_color(true, false, true, theme),
            Color::BLACK
        );

        // tab2 is inactive: primary text and inactive icon
        assert_eq!(
            tab_rainbow::tab_title_color(false, true, theme),
            theme.text.primary
        );
        assert_eq!(
            tab_rainbow::tab_close_icon_color(false, false, true, theme),
            theme.icons.inactive
        );
        assert_eq!(
            tab_rainbow::tab_close_icon_color(false, true, true, theme),
            theme.icons.active
        );

        // 3. Switch active tab to tab2
        app.update(Message::TabSelected(tab2));
        assert_eq!(active_tab(&app), tab2);

        // 4. Toggle Dopamine Mode OFF: reverts to exact standard theme styling
        app.update(Message::SettingsDopamineToggled(false));
        assert!(!app.dopamine.is_enabled());
        assert_eq!(
            tab_rainbow::tab_title_color(true, false, theme),
            theme.text.emphasis
        );
        assert_eq!(
            tab_rainbow::tab_title_color(false, false, theme),
            theme.text.muted
        );
    }

    #[test]
    fn dopamine_slice2_hit_badge_fixed_width_and_combo_update() {
        use std::time::{Duration, Instant};

        let mut app = new_test_app(Locale::JaJp);
        app.update(Message::SettingsDopamineToggled(true));
        let tab = active_tab(&app);
        let now = Instant::now();

        // 1. Initial combo is 0: HIT text is None (hidden), reserved width is maintained
        assert_eq!(app.dopamine.combo_for_tab(tab, now), 0);
        assert_eq!(tab_rainbow::format_hit_text(0), None);
        assert_eq!(tab_rainbow::HIT_BADGE_WIDTH, 42.0);

        // 2. Single token input -> combo = 1
        app.dopamine.on_char_input(tab, 'g', now);
        assert_eq!(app.dopamine.combo_for_tab(tab, now), 1);
        assert_eq!(tab_rainbow::format_hit_text(1), Some("1!".to_string()));

        // 3. Multi-word input -> combo grows
        app.dopamine.on_text_input(tab, "it status", now);
        assert_eq!(app.dopamine.combo_for_tab(tab, now), 2);
        assert_eq!(tab_rainbow::format_hit_text(2), Some("2!".to_string()));

        // 4. Three-digit combo
        for i in 3..=128 {
            app.dopamine.on_text_input(tab, " a", now);
            assert_eq!(app.dopamine.combo_for_tab(tab, now), i);
        }
        assert_eq!(app.dopamine.combo_for_tab(tab, now), 128);
        assert_eq!(tab_rainbow::format_hit_text(128), Some("128!".to_string()));

        // 5. Capped at 999
        assert_eq!(tab_rainbow::format_hit_text(999), Some("999!".to_string()));
        assert_eq!(tab_rainbow::format_hit_text(1000), Some("999!".to_string()));

        // 6. Timeout -> combo resets to 0, HIT text returns to None
        let t_expired = now + Duration::from_millis(1001);
        assert_eq!(app.dopamine.combo_for_tab(tab, t_expired), 0);
        assert_eq!(tab_rainbow::format_hit_text(0), None);
    }

    #[test]
    fn dopamine_slice2_tab_strip_view_does_not_mutate_titles_or_tooltips() {
        use std::time::Instant;

        let mut app = new_test_app(Locale::JaJp);
        let tab = active_tab(&app);
        let initial_title = app.tab_title(tab);

        // Dopamine ON and accumulating combo
        app.update(Message::SettingsDopamineToggled(true));
        app.dopamine
            .on_text_input(tab, "echo hello", Instant::now());
        assert!(app.dopamine.combo_for_tab(tab, Instant::now()) > 0);

        // Title string remains pristine (combo is NOT appended to title)
        assert_eq!(app.tab_title(tab), initial_title);

        // Tab strip builds without panic
        let _view = app.tab_strip();
        drop(_view);

        // Dopamine OFF
        app.update(Message::SettingsDopamineToggled(false));
        assert_eq!(app.tab_title(tab), initial_title);
        let _view_off = app.tab_strip();
        drop(_view_off);
    }

    #[test]
    fn dopamine_slice2_tab_close_and_right_neighbor_fallback() {
        use std::time::Instant;

        let mut app = new_test_app(Locale::JaJp);
        app.update(Message::SettingsDopamineToggled(true));
        let tab1 = active_tab(&app);
        app.update(Message::NewTerminalTab);
        let tab2 = active_tab(&app);
        app.update(Message::NewTerminalTab);
        let tab3 = active_tab(&app);
        assert_ne!(tab1, tab2);
        assert_ne!(tab2, tab3);

        // Select tab2
        app.update(Message::TabSelected(tab2));
        assert_eq!(active_tab(&app), tab2);

        // Add combo to tab2
        app.dopamine.on_char_input(tab2, 'x', Instant::now());
        assert_eq!(app.dopamine.combo_for_tab(tab2, Instant::now()), 1);

        // Close tab2
        app.update(Message::TabClosed(tab2));

        // tab2 is removed and its combo state cleared
        assert_eq!(app.dopamine.combo_for_tab(tab2, Instant::now()), 0);

        // Right-neighbor fallback selects tab3
        assert_eq!(active_tab(&app), tab3);

        // Remaining tab count is 2 (tab1 and tab3)
        assert_eq!(app.workbench.tabs().len(), 2);

        // Tab strip renders cleanly with 2 tabs
        let _view = app.tab_strip();
    }

    #[test]
    fn dopamine_slice2_rainbow_wave_tab_count_invariance_and_time_monotonicity() {
        use std::time::{Duration, Instant};

        let mut app = new_test_app(Locale::JaJp);
        let t0 = Instant::now();

        // 1. Off: elapsed seconds is 0.0
        assert_eq!(app.dopamine.elapsed_animation_seconds(t0), 0.0);

        // 2. Toggled ON: animation_start is set to now
        app.update(Message::SettingsDopamineToggled(true));
        assert!(app.dopamine.is_enabled());

        // Use deterministic time offset
        let t1 = t0 + Duration::from_millis(500);
        let t2 = t0 + Duration::from_secs(5);
        app.dopamine.set_animation_start(t0);

        let e1 = app.dopamine.elapsed_animation_seconds(t1);
        let e2 = app.dopamine.elapsed_animation_seconds(t2);
        assert!((e1 - 0.5).abs() < 1e-4);
        assert!((e2 - 5.0).abs() < 1e-4);
        assert!(e2 > e1);

        // 3. Tab count invariance:
        // Tab 0 hue at any given time t is strictly identical whether there is 1 tab, 2 tabs, or 10 tabs
        let tab0_hue_1tab = tab_rainbow::rainbow_wave_hue(0.0, e1);
        app.update(Message::NewTerminalTab);
        let tab0_hue_2tabs = tab_rainbow::rainbow_wave_hue(0.0, e1);
        app.update(Message::NewTerminalTab);
        let tab0_hue_3tabs = tab_rainbow::rainbow_wave_hue(0.0, e1);

        assert_eq!(tab0_hue_1tab, tab0_hue_2tabs);
        assert_eq!(tab0_hue_2tabs, tab0_hue_3tabs);

        // 4. Tab strip dimensions and style rendering with layout constants
        assert_eq!(TAB_WIDTH, 160.0);
        assert_eq!(TAB_STRIP_HEIGHT, 35.0);
        let _style =
            tab_rainbow::tab_container_style(0, true, true, e1, TAB_WIDTH, TAB_STRIP_HEIGHT);
        let _view = app.tab_strip();
        drop(_view);

        // 5. Toggling OFF resets animation timer to 0.0
        app.update(Message::SettingsDopamineToggled(false));
        assert_eq!(app.dopamine.elapsed_animation_seconds(t2), 0.0);
    }
}
