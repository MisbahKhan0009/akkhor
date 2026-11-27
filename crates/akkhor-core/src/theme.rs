/// A straight (non-premultiplied) RGBA color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

/// Reading colors. Night themes avoid pure white on pure black: the
/// contrast is softer on the eyes in a dark room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub background: Rgba,
    pub text: Rgba,
}

impl Theme {
    /// Black on white: PDF pages exactly as designed.
    pub const ORIGINAL: Theme = Theme {
        background: Rgba::rgb(0xFF, 0xFF, 0xFF),
        text: Rgba::rgb(0x00, 0x00, 0x00),
    };
    pub const PAPER: Theme = Theme {
        background: Rgba::rgb(0xF7, 0xF3, 0xEA),
        text: Rgba::rgb(0x2A, 0x26, 0x20),
    };
    pub const SEPIA: Theme = Theme {
        background: Rgba::rgb(0xF1, 0xE4, 0xCC),
        text: Rgba::rgb(0x43, 0x34, 0x22),
    };
    pub const DUSK: Theme = Theme {
        background: Rgba::rgb(0x2A, 0x28, 0x25),
        text: Rgba::rgb(0xDD, 0xD3, 0xC2),
    };
    pub const NIGHT: Theme = Theme {
        background: Rgba::rgb(0x00, 0x00, 0x00),
        text: Rgba::rgb(0xB8, 0xB2, 0xA6),
    };

    pub fn is_dark(&self) -> bool {
        let b = self.background;
        (b.r as u32 * 299 + b.g as u32 * 587 + b.b as u32 * 114) / 1000 < 128
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::PAPER
    }
}
