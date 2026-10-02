use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb as AlacRgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn hex(value: u32) -> Self {
        Self::new((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    pub fn to_u32(self) -> u32 {
        (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }
}

/// Terminal colors: default foreground/background and the 16 ANSI colors.
/// Indexed colors 16–255 follow the standard xterm cube and gray ramp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub ansi: [Rgb; 16],
}

const fn ansi(values: [u32; 16]) -> [Rgb; 16] {
    let mut out = [Rgb::new(0, 0, 0); 16];
    let mut i = 0;
    while i < 16 {
        out[i] = Rgb::hex(values[i]);
        i += 1;
    }
    out
}

impl Palette {
    pub const fn new(foreground: u32, background: u32, colors: [u32; 16]) -> Self {
        Self {
            foreground: Rgb::hex(foreground),
            background: Rgb::hex(background),
            ansi: ansi(colors),
        }
    }

    pub fn dark() -> Self {
        Self::new(
            0xd4d4d8,
            0x0a0a0a,
            [
                0x1e1e24, 0xf05d5d, 0x5fd08a, 0xe8c15a, 0x5f9bf5, 0xc084fc, 0x4fc8d8, 0xd4d4d8,
                0x5a5a66, 0xff7b7b, 0x7ee7a6, 0xf5d57a, 0x86b6ff, 0xd6a8ff, 0x7ddfec, 0xfafafa,
            ],
        )
    }

    pub fn light() -> Self {
        Self::new(
            0x27272a,
            0xffffff,
            [
                0x27272a, 0xc42b2b, 0x1f8a4c, 0x9a6b00, 0x1d5fd1, 0x8a3fd1, 0x0f7f8f, 0xa1a1aa,
                0x71717a, 0xdc3c3c, 0x23a05a, 0xb58300, 0x2f73e8, 0xa055e8, 0x1395a8, 0x3f3f46,
            ],
        )
    }

    fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => self.ansi[index as usize],
            16..=231 => {
                let i = index - 16;
                let scale = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Rgb::new(scale(i / 36), scale((i / 6) % 6), scale(i % 6))
            }
            _ => {
                let v = 8 + (index - 232) * 10;
                Rgb::new(v, v, v)
            }
        }
    }

    /// Resolve a cell color, honoring colors the application redefined via
    /// OSC 4/10/11.
    pub(crate) fn resolve(&self, color: Color, overrides: &Colors, is_fg: bool) -> Rgb {
        let from = |AlacRgb { r, g, b }: AlacRgb| Rgb::new(r, g, b);
        match color {
            Color::Spec(rgb) => from(rgb),
            Color::Indexed(index) => overrides[index as usize]
                .map(from)
                .unwrap_or_else(|| self.indexed(index)),
            Color::Named(named) => {
                if let Some(rgb) = overrides[named] {
                    return from(rgb);
                }
                let index = named as usize;
                match named {
                    NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => {
                        self.foreground
                    }
                    NamedColor::Background => self.background,
                    NamedColor::DimForeground => self.foreground,
                    _ if index < 16 => self.ansi[index],
                    _ if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize)
                        .contains(&index) =>
                    {
                        self.ansi[index - NamedColor::DimBlack as usize]
                    }
                    _ if is_fg => self.foreground,
                    _ => self.background,
                }
            }
        }
    }
}
