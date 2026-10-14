//! Схемы цветов: фон, текст, курсор и 16 цветов ANSI. «Как в системе»
//! строится из палитры `[appearance]`: фон и текст — оформления synshell,
//! курсор — акцент, цвета ANSI — тёмный или светлый набор под тему.

use synshell_common::config::{Palette, Rgba};
use syngui::prelude::Color;

#[derive(Clone, Copy, PartialEq)]
pub struct Scheme {
    pub bg: Rgba,
    pub fg: Rgba,
    pub cursor: Rgba,
    pub ansi: [Rgba; 16],
}

impl Scheme {
    pub fn is_dark(&self) -> bool {
        luma(self.bg) < 0.5
    }

    pub fn ansi_colors(&self) -> [Color; 16] {
        self.ansi.map(color)
    }
}

pub fn color(c: Rgba) -> Color {
    Color::from_srgb(c.r, c.g, c.b, c.a as f32 / 255.0)
}

fn luma(c: Rgba) -> f32 {
    (0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32) / 255.0
}

const fn hex(v: u32) -> Rgba {
    Rgba::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const fn set(v: [u32; 16]) -> [Rgba; 16] {
    let mut out = [Rgba::rgb(0, 0, 0); 16];
    let mut i = 0;
    while i < 16 {
        out[i] = hex(v[i]);
        i += 1;
    }
    out
}

/// Встроенная схема: id (в настройках), название, фон, текст, курсор, ANSI.
pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    pub bg: u32,
    pub fg: u32,
    pub cursor: u32,
    pub ansi: [u32; 16],
}

/// Набор ANSI «как в системе» для тёмной темы (близок к One Dark).
const SYSTEM_DARK: [u32; 16] = [
    0x3b4048, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xc8ccd4, //
    0x5c6370, 0xff7b86, 0xb5e890, 0xffd68a, 0x82c4ff, 0xde9bf0, 0x7fd4de, 0xffffff,
];
/// Набор ANSI «как в системе» для светлой темы.
const SYSTEM_LIGHT: [u32; 16] = [
    0x383a42, 0xd1383d, 0x3f8f2f, 0xa66f00, 0x2f6fd8, 0x9a3fb5, 0x0f8794, 0x9aa0a8, //
    0x6b7079, 0xe5484d, 0x4fa83c, 0xc18401, 0x4a86ef, 0xb257cf, 0x1aa3b2, 0x2b2d33,
];

pub const BUILTIN: &[Builtin] = &[
    Builtin {
        id: "tango",
        name: "Tango",
        bg: 0x1e1f22,
        fg: 0xd3d7cf,
        cursor: 0xd3d7cf,
        ansi: [
            0x2e3436, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf, //
            0x555753, 0xef2929, 0x8ae234, 0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
        ],
    },
    Builtin {
        id: "one-dark",
        name: "One Dark",
        bg: 0x282c34,
        fg: 0xabb2bf,
        cursor: 0x528bff,
        ansi: [
            0x3f4451, 0xe06c75, 0x98c379, 0xd19a66, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf, //
            0x5c6370, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
        ],
    },
    Builtin {
        id: "dracula",
        name: "Dracula",
        bg: 0x282a36,
        fg: 0xf8f8f2,
        cursor: 0xf8f8f2,
        ansi: [
            0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2, //
            0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
        ],
    },
    Builtin {
        id: "gruvbox-dark",
        name: "Gruvbox",
        bg: 0x282828,
        fg: 0xebdbb2,
        cursor: 0xebdbb2,
        ansi: [
            0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984, //
            0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
        ],
    },
    Builtin {
        id: "nord",
        name: "Nord",
        bg: 0x2e3440,
        fg: 0xd8dee9,
        cursor: 0x88c0d0,
        ansi: [
            0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0, //
            0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
        ],
    },
    Builtin {
        id: "catppuccin-mocha",
        name: "Catppuccin",
        bg: 0x1e1e2e,
        fg: 0xcdd6f4,
        cursor: 0xf5e0dc,
        ansi: [
            0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, //
            0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
        ],
    },
    Builtin {
        id: "solarized-dark",
        name: "Solarized",
        bg: 0x002b36,
        fg: 0x93a1a1,
        cursor: 0x93a1a1,
        ansi: [
            0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, //
            0x586e75, 0xcb4b16, 0x859900, 0xb58900, 0x268bd2, 0x6c71c4, 0x2aa198, 0xfdf6e3,
        ],
    },
    Builtin {
        id: "solarized-light",
        name: "Solarized Light",
        bg: 0xfdf6e3,
        fg: 0x586e75,
        cursor: 0x586e75,
        ansi: [
            0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, //
            0x586e75, 0xcb4b16, 0x859900, 0xb58900, 0x268bd2, 0x6c71c4, 0x2aa198, 0x002b36,
        ],
    },
    Builtin {
        id: "github-light",
        name: "GitHub Light",
        bg: 0xffffff,
        fg: 0x24292f,
        cursor: 0x0969da,
        ansi: [
            0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781, //
            0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
        ],
    },
];

/// «Как в системе»: цвета оформления synshell.
pub fn system(p: &Palette, dark: bool) -> Scheme {
    // Фон чуть глубже окна, как у содержимого в «Проводнике».
    let bg = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.18) } else { p.bg.mix(Rgba::rgb(255, 255, 255), 0.5) };
    Scheme {
        bg: Rgba { a: 255, ..bg },
        fg: Rgba { a: 255, ..p.fg },
        cursor: Rgba { a: 255, ..p.accent },
        ansi: set(if dark { SYSTEM_DARK } else { SYSTEM_LIGHT }),
    }
}

pub fn builtin(b: &Builtin) -> Scheme {
    Scheme { bg: hex(b.bg), fg: hex(b.fg), cursor: hex(b.cursor), ansi: set(b.ansi) }
}

/// Схема по id из настроек; неизвестная — «как в системе».
pub fn resolve(id: &str, p: &Palette, dark: bool) -> Scheme {
    BUILTIN.iter().find(|b| b.id == id).map(builtin).unwrap_or_else(|| system(p, dark))
}
