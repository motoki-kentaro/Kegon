//! Night Dark: the canonical definition. Do not copy these values elsewhere.

use iced::Color;

use super::model::{
    AnsiColors, IconColors, InteractionColors, KegonTheme, SemanticColors, TerminalColors,
    TextColors, ThemeId, WorkbenchColors,
};

const fn hex(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub static NIGHT_DARK: KegonTheme = KegonTheme {
    id: ThemeId::NightDark,
    workbench: WorkbenchColors {
        background: hex(0x1E1E1E),
        activity_bar_background: hex(0x2C2C2C),
        side_bar_background: hex(0x252526),
        tab_bar_background: hex(0x252526),
        surface_background: hex(0x252526),
        surface_elevated: hex(0x2D2D30),
        backdrop: Color::from_rgba8(0x00, 0x00, 0x00, 0.65),
        border: hex(0x3C3C3C),
    },
    interaction: InteractionColors {
        hover: hex(0x37373D),
        selection: hex(0x264F78),
        accent: hex(0x007ACC),
    },
    text: TextColors {
        primary: hex(0xCCCCCC),
        muted: hex(0x858585),
        disabled: hex(0x666666),
        emphasis: hex(0xFFFFFF),
        on_accent: hex(0xFFFFFF),
    },
    icons: IconColors {
        active: hex(0xFFFFFF),
        hovered: hex(0xCCCCCC),
        inactive: hex(0x858585),
    },
    semantic: SemanticColors {
        warning: hex(0xD7BA7D),
        destructive: hex(0xD73A49),
        success: hex(0x4EC9B0),
    },
    terminal: TerminalColors {
        background: hex(0x1E1E1E),
        foreground: hex(0xCCCCCC),
        cursor: hex(0xAEAFAD),
        selection: hex(0x264F78),
        preedit_background: hex(0x3A3D41),
        preedit_foreground: hex(0xFFFFFF),
        ansi: AnsiColors {
            black: hex(0x1E1E1E),
            red: hex(0xCD3131),
            green: hex(0x0DBC79),
            yellow: hex(0xE5E510),
            blue: hex(0x2472C8),
            magenta: hex(0xBC3FBC),
            cyan: hex(0x11A8CD),
            white: hex(0xE5E5E5),
            bright_black: hex(0x666666),
            bright_red: hex(0xF14C4C),
            bright_green: hex(0x23D18B),
            bright_yellow: hex(0xF5F543),
            bright_blue: hex(0x3B8EE8),
            bright_magenta: hex(0xD670D6),
            bright_cyan: hex(0x29B8DB),
            bright_white: hex(0xFFFFFF),
        },
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(color: Color) -> u32 {
        let [r, g, b, a] = color.into_rgba8();
        assert_eq!(a, 255, "token is expected to be opaque");
        (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
    }

    #[test]
    fn identity() {
        assert_eq!(NIGHT_DARK.id, ThemeId::NightDark);
    }

    #[test]
    fn workbench_tokens_match_schema() {
        let w = NIGHT_DARK.workbench;
        assert_eq!(rgb(w.background), 0x1E1E1E);
        assert_eq!(rgb(w.activity_bar_background), 0x2C2C2C);
        assert_eq!(rgb(w.side_bar_background), 0x252526);
        assert_eq!(rgb(w.tab_bar_background), 0x252526);
        assert_eq!(rgb(w.surface_background), 0x252526);
        assert_eq!(rgb(w.surface_elevated), 0x2D2D30);
        assert_eq!(rgb(w.border), 0x3C3C3C);
        assert_eq!(w.backdrop, Color::from_rgba8(0, 0, 0, 0.65));
    }

    #[test]
    fn interaction_text_icon_and_semantic_tokens_match_schema() {
        let i = NIGHT_DARK.interaction;
        assert_eq!(rgb(i.hover), 0x37373D);
        assert_eq!(rgb(i.selection), 0x264F78);
        assert_eq!(rgb(i.accent), 0x007ACC);

        let t = NIGHT_DARK.text;
        assert_eq!(rgb(t.primary), 0xCCCCCC);
        assert_eq!(rgb(t.muted), 0x858585);
        assert_eq!(rgb(t.disabled), 0x666666);
        assert_eq!(rgb(t.emphasis), 0xFFFFFF);
        assert_eq!(rgb(t.on_accent), 0xFFFFFF);

        let icons = NIGHT_DARK.icons;
        assert_eq!(rgb(icons.active), 0xFFFFFF);
        assert_eq!(rgb(icons.hovered), 0xCCCCCC);
        assert_eq!(rgb(icons.inactive), 0x858585);

        let s = NIGHT_DARK.semantic;
        assert_eq!(rgb(s.warning), 0xD7BA7D);
        assert_eq!(rgb(s.destructive), 0xD73A49);
        assert_eq!(rgb(s.success), 0x4EC9B0);
    }

    #[test]
    fn terminal_base_tokens_match_schema() {
        let t = NIGHT_DARK.terminal;
        assert_eq!(rgb(t.background), 0x1E1E1E);
        assert_eq!(rgb(t.foreground), 0xCCCCCC);
        assert_eq!(rgb(t.cursor), 0xAEAFAD);
        assert_eq!(rgb(t.selection), 0x264F78);
        assert_eq!(rgb(t.preedit_background), 0x3A3D41);
        assert_eq!(rgb(t.preedit_foreground), 0xFFFFFF);
    }

    #[test]
    fn terminal_ansi_palette_matches_schema() {
        let expected = [
            0x1E1E1E, 0xCD3131, 0x0DBC79, 0xE5E510, 0x2472C8, 0xBC3FBC, 0x11A8CD, 0xE5E5E5,
            0x666666, 0xF14C4C, 0x23D18B, 0xF5F543, 0x3B8EE8, 0xD670D6, 0x29B8DB, 0xFFFFFF,
        ];
        let ansi = NIGHT_DARK.terminal.ansi;
        for (index, &want) in expected.iter().enumerate() {
            let got = ansi.indexed(index as u8).expect("0-15 are defined");
            assert_eq!(rgb(got), want, "ANSI index {index}");
        }
        assert_eq!(ansi.indexed(16), None);
    }
}
