//! Pencode dark theme, mirrored from the upstream theme definition
//! (`themes/pencode.json`, dark palette) so the TUI matches product branding.

use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Rgb(0x0a, 0x0a, 0x0a);
pub const INK: Color = Color::Rgb(0xee, 0xee, 0xee);
pub const WEAK: Color = Color::Rgb(0x80, 0x80, 0x80);
pub const PRIMARY: Color = Color::Rgb(0xfa, 0xb2, 0x83);
pub const ACCENT: Color = Color::Rgb(0x9d, 0x7c, 0xd8);
pub const SUCCESS: Color = Color::Rgb(0x7f, 0xd8, 0x8f);
pub const WARNING: Color = Color::Rgb(0xf5, 0xa7, 0x42);
pub const ERROR: Color = Color::Rgb(0xe0, 0x6c, 0x75);
pub const INFO: Color = Color::Rgb(0x56, 0xb6, 0xc2);

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
    fn theme_colors_match_upstream_dark_palette() {
        assert_eq!(BG, Color::Rgb(10, 10, 10));
        assert_eq!(PRIMARY, Color::Rgb(250, 178, 131));
        assert_eq!(ACCENT, Color::Rgb(157, 124, 216));
        assert_eq!(WARNING, Color::Rgb(245, 167, 66));
    }
}
