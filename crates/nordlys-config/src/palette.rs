//! The Nordlys palette. These values match the design canvas.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex_u32(v: u32) -> Self {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Parse `#RRGGBB` (the `#` is optional).
    pub fn parse(s: &str) -> Option<Self> {
        let h = s.strip_prefix('#').unwrap_or(s);
        if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(h, 16).ok().map(Self::hex_u32)
    }

    /// `#RRGGBB`, upper case.
    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }

    /// `RRGGBB` without the `#`, for foot and friends.
    pub fn bare(self) -> String {
        self.hex()[1..].to_owned()
    }

    /// Perceived brightness, 0.0–1.0.
    pub fn luma(self) -> f32 {
        (0.299 * self.0 as f32 + 0.587 * self.1 as f32 + 0.114 * self.2 as f32) / 255.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub ground: Rgb,
    pub surface: Rgb,
    pub raised: Rgb,
    pub border: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub dim: Rgb,
    pub accent: Rgb,
    pub on_accent: Rgb,
    /// Corner radius base in pixels (see `corner`). From the config.
    pub roundness: u8,
    /// Terminal colours 0–15.
    pub ansi: [Rgb; 16],
}

const fn c(v: u32) -> Rgb {
    Rgb::hex_u32(v)
}

impl Palette {
    /// A corner radius: `base` px at the default roundness, scaled to the
    /// configured one. Zero roundness gives square corners.
    pub fn corner(&self, base: f32) -> f32 {
        base * f32::from(self.roundness) / f32::from(crate::DEFAULT_ROUNDNESS)
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            ground: c(0x0A0D13),
            surface: c(0x12161E),
            raised: c(0x1B2029),
            border: c(0x2A303B),
            text: c(0xE6EAF0),
            muted: c(0xA3ADBB),
            dim: c(0x8590A0),
            accent: c(0x8AB4FF),
            on_accent: c(0x0A0D13),
            roundness: crate::DEFAULT_ROUNDNESS,
            ansi: [
                c(0x1B2029),
                c(0xF28B82),
                c(0xA8D8A0),
                c(0xF6C177),
                c(0x8AB4FF),
                c(0xC9A7FF),
                c(0x8EE0D4),
                c(0xC8D0DC),
                c(0x4A5262),
                c(0xF6A59E),
                c(0xC0E6B9),
                c(0xF9D39B),
                c(0xB5CEFF),
                c(0xDCC6FF),
                c(0xB0EBE2),
                c(0xE6EAF0),
            ],
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            ground: c(0xEEF1F6),
            surface: c(0xFFFFFF),
            raised: c(0xF6F8FB),
            border: c(0xD5DAE2),
            text: c(0x141821),
            muted: c(0x4F5867),
            dim: c(0x5D6675),
            accent: c(0x2F5BD8),
            on_accent: c(0xFFFFFF),
            roundness: crate::DEFAULT_ROUNDNESS,
            ansi: [
                c(0x141821),
                c(0xC4362B),
                c(0x2F7D32),
                c(0x9A6700),
                c(0x2F5BD8),
                c(0x7C3AED),
                c(0x0E7C76),
                c(0xD5DAE2),
                c(0x4F5867),
                c(0xD9534A),
                c(0x3C9A40),
                c(0xB98000),
                c(0x4F78E6),
                c(0x9466F0),
                c(0x16968F),
                c(0xEEF1F6),
            ],
        }
    }

    pub fn set_accent(&mut self, a: Rgb) {
        self.accent = a;
        self.on_accent = if a.luma() > 0.6 {
            c(0x0A0D13)
        } else {
            c(0xFFFFFF)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_print() {
        assert_eq!(Rgb::parse("#8ab4ff"), Some(c(0x8AB4FF)));
        assert_eq!(Rgb::parse("8AB4FF").unwrap().hex(), "#8AB4FF");
        assert_eq!(Rgb::parse("#8AB4F"), None);
        assert_eq!(Rgb::parse("#GGGGGG"), None);
    }

    #[test]
    fn text_on_accent_stays_readable() {
        let mut p = Palette::dark();
        p.set_accent(c(0xA8D8A0));
        assert_eq!(p.on_accent, c(0x0A0D13));
        p.set_accent(c(0x2F5BD8));
        assert_eq!(p.on_accent, c(0xFFFFFF));
    }
}
