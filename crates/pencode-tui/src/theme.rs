//! Pencode themes mirrored from the upstream theme definition
//! (`themes/pencode.json`). The active palette is runtime-switchable via
//! the `/themes` command.

use ratatui::style::{Color, Modifier, Style};
use std::sync::RwLock;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color,
    pub ink: Color,
    pub weak: Color,
    pub primary: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
}

/// Upstream `pencode.json` dark palette.
pub const DARK: Palette = Palette {
    bg: Color::Rgb(0x0a, 0x0a, 0x0a),
    ink: Color::Rgb(0xee, 0xee, 0xee),
    weak: Color::Rgb(0x80, 0x80, 0x80),
    primary: Color::Rgb(0xfa, 0xb2, 0x83),
    accent: Color::Rgb(0x9d, 0x7c, 0xd8),
    success: Color::Rgb(0x7f, 0xd8, 0x8f),
    warning: Color::Rgb(0xf5, 0xa7, 0x42),
    error: Color::Rgb(0xe0, 0x6c, 0x75),
    info: Color::Rgb(0x56, 0xb6, 0xc2),
};

/// Upstream `pencode.json` light palette.
pub const LIGHT: Palette = Palette {
    bg: Color::Rgb(0xff, 0xff, 0xff),
    ink: Color::Rgb(0x1a, 0x1a, 0x1a),
    weak: Color::Rgb(0x8a, 0x8a, 0x8a),
    primary: Color::Rgb(0x3b, 0x7d, 0xd8),
    accent: Color::Rgb(0xd6, 0x8c, 0x27),
    success: Color::Rgb(0x3d, 0x9a, 0x57),
    warning: Color::Rgb(0xd6, 0x8c, 0x27),
    error: Color::Rgb(0xd1, 0x38, 0x3d),
    info: Color::Rgb(0x31, 0x87, 0x95),
};

static ACTIVE: RwLock<Palette> = RwLock::new(DARK);

pub fn active() -> Palette {
    *ACTIVE.read().expect("theme lock")
}

pub fn set_dark() {
    *ACTIVE.write().expect("theme lock") = DARK;
}

pub fn set_light() {
    *ACTIVE.write().expect("theme lock") = LIGHT;
}

pub fn is_dark() -> bool {
    active().bg == DARK.bg
}

pub fn fg(color: Color) -> Style {
    Style::default().fg(color)
}

pub fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_match_upstream_definitions() {
        assert_eq!(DARK.primary, Color::Rgb(250, 178, 131));
        assert_eq!(DARK.accent, Color::Rgb(157, 124, 216));
        assert_eq!(LIGHT.primary, Color::Rgb(59, 125, 216));
        assert_eq!(LIGHT.accent, Color::Rgb(214, 140, 39));
        assert_ne!(DARK.bg, LIGHT.bg);
    }

    #[test]
    fn switches_palettes_at_runtime() {
        set_light();
        assert!(is_dark() == false);
        set_dark();
        assert!(is_dark());
    }
}
