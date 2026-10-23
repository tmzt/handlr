//! `qrcodeurl`: a string as a QR code, rendered as terminal text, a 1-bit
//! grayscale PNG, a `data:` URL, base64url, or an inline kitty or iTerm2
//! image (docs/design/20261022_06_QRCODEURL_PLAN.md).
//!
//! The text is encoded in byte mode at the level asked for (M by default),
//! with the 4-module quiet zone ISO/IEC 18004 requires. Everything here is
//! pure: nothing is logged, and no error or `Debug` output contains the
//! text, which may be a secret.

mod png;

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use qrcode::render::unicode::Dense1x2;
use qrcode::types::QrError;
use qrcode::{Color, EcLevel, QrCode, Version};

/// The quiet zone of the terminal rendering, in modules.
const TERMINAL_QUIET_ZONE: u32 = 4;

/// The kitty graphics protocol's largest chunk of base64.
const KITTY_CHUNK: usize = 4096;

/// The error correction level: L 7 %, M 15 %, Q 25 %, H 30 %.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ecc {
    L,
    #[default]
    M,
    Q,
    H,
}

impl From<Ecc> for EcLevel {
    fn from(ecc: Ecc) -> EcLevel {
        match ecc {
            Ecc::L => EcLevel::L,
            Ecc::M => EcLevel::M,
            Ecc::Q => EcLevel::Q,
            Ecc::H => EcLevel::H,
        }
    }
}

/// How a PNG is laid out: pixels per module and the quiet zone in modules.
/// The side is `(modules + 2 × quiet_zone) × scale` pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Png {
    pub scale: u32,
    pub quiet_zone: u32,
}

impl Png {
    /// `scale` in 1..=32 and `quiet_zone` in 0..=16, else [`Error::BadSize`].
    pub fn new(scale: u32, quiet_zone: u32) -> Result<Png, Error> {
        if !(1..=32).contains(&scale) || quiet_zone > 16 {
            return Err(Error::BadSize);
        }
        Ok(Png { scale, quiet_zone })
    }
}

impl Default for Png {
    /// 8 pixels per module, a 4-module quiet zone.
    fn default() -> Self {
        Png {
            scale: 8,
            quiet_zone: 4,
        }
    }
}

/// Which way round the terminal rendering's colours are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Terminal {
    /// Dark modules drawn as spaces on a light-drawn background: for a dark
    /// terminal.
    #[default]
    LightOnDark,
    DarkOnLight,
}

/// Which inline image protocol a terminal speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inline {
    /// The kitty graphics protocol (kitty, Ghostty, WezTerm, and herdr with
    /// `kitty_graphics` on).
    Kitty,
    /// iTerm2's `OSC 1337 File=` (iTerm2, WezTerm).
    Iterm2,
}

/// Why a code could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The text is longer than a version 40 code holds at this level.
    TooLong { len: usize, ecc: Ecc },
    /// [`Png::new`] was given a scale or quiet zone out of range.
    BadSize,
    /// Any other encoding error (not expected for byte input).
    Unencodable,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooLong { len, ecc } => write!(
                f,
                "{len} bytes is too long for a QR code at error correction level {ecc:?}"
            ),
            Error::BadSize => f.write_str("a PNG scale must be 1 to 32 and a quiet zone 0 to 16"),
            Error::Unencodable => f.write_str("the text cannot be encoded as a QR code"),
        }
    }
}

impl std::error::Error for Error {}

/// An encoded QR code.
pub struct Qr {
    code: QrCode,
    version: u8,
}

impl fmt::Debug for Qr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Qr")
            .field("version", &self.version)
            .field("modules", &self.modules())
            .finish()
    }
}

impl Qr {
    /// `text` at error correction level M.
    pub fn new(text: &str) -> Result<Qr, Error> {
        Qr::with_ecc(text, Ecc::M)
    }

    pub fn with_ecc(text: &str, ecc: Ecc) -> Result<Qr, Error> {
        let code = QrCode::with_error_correction_level(text.as_bytes(), ecc.into()).map_err(
            |e| match e {
                QrError::DataTooLong => Error::TooLong {
                    len: text.len(),
                    ecc,
                },
                _ => Error::Unencodable,
            },
        )?;
        let version = match code.version() {
            Version::Normal(v) | Version::Micro(v) => u8::try_from(v).unwrap_or(0),
        };
        Ok(Qr { code, version })
    }

    /// The version, 1 to 40.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// The side in modules, without the quiet zone.
    pub fn modules(&self) -> usize {
        self.code.width()
    }

    /// Whether the module at `(x, y)` is dark; outside the code, false.
    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        x < self.modules() && y < self.modules() && self.code[(x, y)] == Color::Dark
    }

    /// The side of the PNG in pixels.
    pub fn png_side(&self, png: Png) -> u32 {
        (self.modules() as u32 + 2 * png.quiet_zone) * png.scale
    }

    /// A 1-bit grayscale PNG.
    pub fn png(&self, png: Png) -> Vec<u8> {
        let offset = png.quiet_zone * png.scale;
        png::encode(self.png_side(png), |x, y| {
            let module = |p: u32| p.checked_sub(offset).map(|p| (p / png.scale) as usize);
            match (module(x), module(y)) {
                (Some(mx), Some(my)) => self.is_dark(mx, my),
                _ => false,
            }
        })
    }

    /// `data:image/png;base64,` and the PNG in standard base64, padded
    /// (RFC 2397).
    pub fn data_url(&self, png: Png) -> String {
        format!("data:image/png;base64,{}", STANDARD.encode(self.png(png)))
    }

    /// The PNG in URL-safe base64 without padding, for carrying inside
    /// another URL.
    pub fn base64url(&self, png: Png) -> String {
        URL_SAFE_NO_PAD.encode(self.png(png))
    }

    /// Unicode half blocks, two modules per character cell, with a 4-module
    /// quiet zone.
    pub fn terminal(&self, style: Terminal) -> String {
        let mut renderer = self.code.render::<Dense1x2>();
        renderer.quiet_zone(TERMINAL_QUIET_ZONE > 0);
        if style == Terminal::LightOnDark {
            renderer
                .dark_color(Dense1x2::Light)
                .light_color(Dense1x2::Dark);
        }
        renderer.build()
    }

    /// The escape sequence that shows the PNG at the cursor, then a newline.
    /// Whether the terminal understands it is the caller's to know.
    pub fn inline(&self, png: Png, protocol: Inline) -> String {
        let bytes = self.png(png);
        let b64 = STANDARD.encode(&bytes);
        let mut out = String::with_capacity(b64.len() + 64);
        match protocol {
            Inline::Kitty => {
                // q=2: no replies into the pane. Base64 is ASCII, so byte
                // chunks are character chunks.
                let chunks: Vec<&[u8]> = b64.as_bytes().chunks(KITTY_CHUNK).collect();
                for (i, chunk) in chunks.iter().enumerate() {
                    let more = u8::from(i + 1 < chunks.len());
                    let chunk = std::str::from_utf8(chunk).expect("base64 is ASCII");
                    if i == 0 {
                        out.push_str(&format!("\x1b_Ga=T,f=100,q=2,m={more};{chunk}\x1b\\"));
                    } else {
                        out.push_str(&format!("\x1b_Gm={more};{chunk}\x1b\\"));
                    }
                }
            }
            Inline::Iterm2 => {
                out.push_str(&format!(
                    "\x1b]1337;File=inline=1;size={};preserveAspectRatio=1:{b64}\x07",
                    bytes.len()
                ));
            }
        }
        out.push('\n');
        out
    }
}

/// [`Qr::png`] of `text` at level M, scale 8, quiet zone 4.
pub fn png(text: &str) -> Result<Vec<u8>, Error> {
    Ok(Qr::new(text)?.png(Png::default()))
}

/// [`Qr::data_url`] of `text` at level M, scale 8, quiet zone 4.
pub fn data_url(text: &str) -> Result<String, Error> {
    Ok(Qr::new(text)?.data_url(Png::default()))
}

/// [`Qr::terminal`] of `text` at level M, light on dark.
pub fn terminal(text: &str) -> Result<String, Error> {
    Ok(Qr::new(text)?.terminal(Terminal::default()))
}
