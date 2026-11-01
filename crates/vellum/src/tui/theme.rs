//! Themes: the Midnight Bloom palette (docs/brand.md), an ANSI palette that
//! inherits the terminal's colors, and a NO_COLOR one that uses only bold,
//! reverse and underline.
//!
//! The view never names a color. It asks the [`Theme`] for a role (text,
//! focus, selection, ...), so a new widget only has to pick the right role.

use ratatui::style::{Color, Modifier, Style};

use crate::config::ThemeName;

/// Midnight Bloom tokens (docs/brand.md).
mod token {
    use ratatui::style::Color;
    pub const INK: Color = Color::Rgb(0x0B, 0x0D, 0x14);
    pub const CANVAS_HI: Color = Color::Rgb(0x1A, 0x1B, 0x2E);
    pub const MINT: Color = Color::Rgb(0x8D, 0xE3, 0xDC);
    pub const BLUE: Color = Color::Rgb(0x7A, 0xA2, 0xF7);
    pub const TEXT: Color = Color::Rgb(0xC0, 0xCA, 0xF5);
    pub const SUBTEXT: Color = Color::Rgb(0xA9, 0xB1, 0xD6);
    pub const MUTED: Color = Color::Rgb(0x56, 0x5F, 0x89);
    pub const ROSE: Color = Color::Rgb(0xF7, 0x76, 0x8E);
    pub const ORANGE: Color = Color::Rgb(0xFF, 0x9E, 0x64);
    pub const GREEN: Color = Color::Rgb(0x9E, 0xCE, 0x6A);
}

/// Which palette is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    MidnightBloom,
    Ansi,
    NoColor,
}

/// A style for every role the view uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub palette: Palette,
    /// The whole screen: background and default text.
    pub base: Style,
    /// Raised surfaces: toasts and overlays.
    pub panel: Style,
    /// Primary text.
    pub text: Style,
    /// Metadata and footer labels.
    pub secondary: Style,
    /// Borders and disabled items.
    pub muted: Style,
    /// The one focus highlight: focused border, cursor, key glyphs.
    pub focus: Style,
    /// The highlighted list row. Never color alone.
    pub selection: Style,
    pub error: Style,
    pub warning: Style,
    pub success: Style,
}

/// What the terminal environment says about color.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColorEnv {
    /// `NO_COLOR` is set to something non-empty.
    pub no_color: bool,
    /// `COLORTERM` says truecolor or 24bit.
    pub truecolor: bool,
}

impl ColorEnv {
    pub fn from_process() -> ColorEnv {
        let var = |name| std::env::var(name).unwrap_or_default();
        ColorEnv {
            no_color: !var("NO_COLOR").is_empty(),
            truecolor: matches!(var("COLORTERM").as_str(), "truecolor" | "24bit"),
        }
    }
}

impl Theme {
    /// The theme for Config `name` in `env`. NO_COLOR always wins, and
    /// Midnight Bloom needs truecolor, else it falls back to ANSI.
    pub fn resolve(name: ThemeName, env: ColorEnv) -> Theme {
        if env.no_color {
            Theme::no_color()
        } else if name == ThemeName::Midnight && env.truecolor {
            Theme::midnight_bloom()
        } else {
            Theme::ansi()
        }
    }

    pub fn midnight_bloom() -> Theme {
        use token::*;
        let fg = |c| Style::new().fg(c);
        Theme {
            palette: Palette::MidnightBloom,
            // The terminal's own background, so its opacity and blur show
            // through. An RGB fill here would paint over them.
            base: Style::new().fg(TEXT).bg(Color::Reset),
            panel: Style::new().fg(TEXT).bg(CANVAS_HI),
            text: fg(TEXT),
            secondary: fg(SUBTEXT),
            muted: fg(MUTED),
            focus: fg(MINT),
            selection: Style::new().fg(INK).bg(BLUE).add_modifier(Modifier::BOLD),
            error: fg(ROSE),
            warning: fg(ORANGE),
            success: fg(GREEN),
        }
    }

    /// The terminal's own palette and background.
    pub fn ansi() -> Theme {
        let fg = |c| Style::new().fg(c);
        Theme {
            palette: Palette::Ansi,
            base: Style::new().fg(Color::Reset).bg(Color::Reset),
            panel: Style::new().add_modifier(Modifier::REVERSED),
            text: fg(Color::White),
            secondary: fg(Color::Gray),
            muted: fg(Color::DarkGray),
            focus: fg(Color::Cyan),
            selection: Style::new()
                .fg(Color::Blue)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
            error: fg(Color::Red),
            warning: fg(Color::Yellow),
            success: fg(Color::Green),
        }
    }

    /// Bold, reverse and underline only.
    pub fn no_color() -> Theme {
        let plain = Style::new();
        let bold = Style::new().add_modifier(Modifier::BOLD);
        Theme {
            palette: Palette::NoColor,
            base: plain,
            panel: Style::new().add_modifier(Modifier::REVERSED),
            text: plain,
            secondary: plain,
            muted: plain,
            focus: bold,
            selection: Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            error: bold,
            warning: bold,
            success: bold,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRUECOLOR: ColorEnv = ColorEnv {
        no_color: false,
        truecolor: true,
    };

    #[test]
    fn midnight_bloom_is_the_default_on_truecolor_terminals() {
        let theme = Theme::resolve(ThemeName::Midnight, TRUECOLOR);
        assert_eq!(theme.palette, Palette::MidnightBloom);
        assert_eq!(theme.focus.fg, Some(Color::Rgb(0x8D, 0xE3, 0xDC)));
        assert_eq!(theme.selection.bg, Some(Color::Rgb(0x7A, 0xA2, 0xF7)));
        assert_eq!(theme.text.fg, Some(Color::Rgb(0xC0, 0xCA, 0xF5)));
        assert_eq!(theme.secondary.fg, Some(Color::Rgb(0xA9, 0xB1, 0xD6)));
    }

    #[test]
    fn midnight_bloom_keeps_the_terminal_background() {
        let theme = Theme::resolve(ThemeName::Midnight, TRUECOLOR);
        assert_eq!(theme.base.bg, Some(Color::Reset));
    }

    #[test]
    fn other_terminals_fall_back_to_ansi() {
        let env = ColorEnv::default();
        assert_eq!(
            Theme::resolve(ThemeName::Midnight, env).palette,
            Palette::Ansi
        );
    }

    #[test]
    fn the_terminal_theme_uses_ansi_even_with_truecolor() {
        let theme = Theme::resolve(ThemeName::Terminal, TRUECOLOR);
        assert_eq!(theme.palette, Palette::Ansi);
        assert_eq!(theme.base.bg, Some(Color::Reset));
    }

    #[test]
    fn no_color_wins_and_marks_selection_by_style() {
        let env = ColorEnv {
            no_color: true,
            truecolor: true,
        };
        let theme = Theme::resolve(ThemeName::Midnight, env);
        assert_eq!(theme.palette, Palette::NoColor);
        for style in [
            &theme.base,
            &theme.text,
            &theme.focus,
            &theme.selection,
            &theme.error,
        ] {
            assert_eq!((style.fg, style.bg), (None, None), "{style:?}");
        }
        assert!(theme.selection.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn selection_is_never_color_alone() {
        for theme in [Theme::midnight_bloom(), Theme::ansi(), Theme::no_color()] {
            assert!(
                theme.selection.add_modifier.contains(Modifier::BOLD),
                "{:?}",
                theme.palette
            );
        }
    }
}
