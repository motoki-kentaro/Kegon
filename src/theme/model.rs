//! Typed theme model: theme identity and semantic color tokens.
//!
//! Every token is a named field, so a theme that forgets one fails to compile.

use iced::Color;

/// Identifies a built-in Kegon theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeId {
    NightDark,
}

impl ThemeId {
    /// All built-in themes, in the order they are offered in Settings.
    pub const ALL: [Self; 1] = [Self::NightDark];

    /// The theme used when none is configured or the configured one is unknown.
    pub const DEFAULT: Self = Self::NightDark;

    /// The stable identifier stored in `settings.toml`. Never localized.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NightDark => "night-dark",
        }
    }

    /// Parses a stored identifier. Returns `None` for unknown themes.
    pub fn from_persisted(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.as_str() == id)
    }

    /// The product name of the theme, shown as-is in every locale.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::NightDark => "Night Dark",
        }
    }
}

/// A complete Kegon theme. See `docs/architecture/themes.md`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KegonTheme {
    pub id: ThemeId,
    pub workbench: WorkbenchColors,
    pub interaction: InteractionColors,
    pub text: TextColors,
    pub icons: IconColors,
    pub semantic: SemanticColors,
    pub terminal: TerminalColors,
}

impl KegonTheme {
    /// The iced theme used for widgets Kegon does not style itself, such as
    /// scrollbars.
    pub fn iced_theme(&self) -> iced::Theme {
        match self.id {
            ThemeId::NightDark => iced::Theme::Dark,
        }
    }
}

/// Workbench areas and surfaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkbenchColors {
    /// Main workbench / content background (`workbench.background`).
    pub background: Color,
    /// `activity_bar.background`
    pub activity_bar_background: Color,
    /// `side_bar.background`
    pub side_bar_background: Color,
    /// `tab_bar.background`
    pub tab_bar_background: Color,
    /// Dialogs and other general surfaces (`surface.background`).
    pub surface_background: Color,
    /// Surfaces raised above `surface_background`: tooltips, secondary
    /// buttons, menus (`surface.elevated`).
    pub surface_elevated: Color,
    /// Scrim drawn over the workbench behind a modal (`surface.backdrop`).
    pub backdrop: Color,
    /// Standard borders and dividers (`border.default`).
    pub border: Color,
}

/// Interaction states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InteractionColors {
    /// `interaction.hover`
    pub hover: Color,
    /// Selected text and other selection highlights (`interaction.selection`).
    pub selection: Color,
    /// Focus, active state, and primary actions (`accent.primary`).
    pub accent: Color,
}

/// Text colors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextColors {
    /// `text.primary`
    pub primary: Color,
    /// `text.muted`
    pub muted: Color,
    /// `text.disabled`
    pub disabled: Color,
    /// Titles, the active tab label, and other text that stands out from
    /// `primary` (`text.emphasis`).
    pub emphasis: Color,
    /// Text and focus rings drawn on accent or destructive fills
    /// (`text.on_accent`).
    pub on_accent: Color,
}

/// Monochrome icon tints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconColors {
    /// `icon.active`
    pub active: Color,
    /// `icon.hovered`
    pub hovered: Color,
    /// `icon.inactive`
    pub inactive: Color,
}

/// Status colors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemanticColors {
    /// `semantic.warning`
    pub warning: Color,
    /// Destructive actions and failures (`semantic.destructive`).
    pub destructive: Color,
    /// `semantic.success`
    pub success: Color,
}

/// Terminal colors used for content without an explicit RGB color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalColors {
    /// `terminal.background`
    pub background: Color,
    /// `terminal.foreground`
    pub foreground: Color,
    /// `terminal.cursor`
    pub cursor: Color,
    /// `terminal.selection`, drawn with the renderer's selection alpha.
    pub selection: Color,
    /// Background behind IME composition text (`terminal.preedit_background`).
    pub preedit_background: Color,
    /// IME composition text and its underline (`terminal.preedit_foreground`).
    pub preedit_foreground: Color,
    pub ansi: AnsiColors,
}

/// The 16 standard ANSI colors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnsiColors {
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub white: Color,
    pub bright_black: Color,
    pub bright_red: Color,
    pub bright_green: Color,
    pub bright_yellow: Color,
    pub bright_blue: Color,
    pub bright_magenta: Color,
    pub bright_cyan: Color,
    pub bright_white: Color,
}

impl AnsiColors {
    /// The color for 256-color indices 0–15. Returns `None` for 16 and up.
    pub fn indexed(&self, index: u8) -> Option<Color> {
        let color = match index {
            0 => self.black,
            1 => self.red,
            2 => self.green,
            3 => self.yellow,
            4 => self.blue,
            5 => self.magenta,
            6 => self.cyan,
            7 => self.white,
            8 => self.bright_black,
            9 => self.bright_red,
            10 => self.bright_green,
            11 => self.bright_yellow,
            12 => self.bright_blue,
            13 => self.bright_magenta,
            14 => self.bright_cyan,
            15 => self.bright_white,
            _ => return None,
        };
        Some(color)
    }
}
