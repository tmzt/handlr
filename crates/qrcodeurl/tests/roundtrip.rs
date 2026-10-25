//! Plan 20261022_06, step 1: the PNG read back by `png`, the code read back
//! by `rqrr`, and the other renderings.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use qrcodeurl::{Ecc, Error, Inline, Png, Qr, Terminal};

const BASE58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// `flankr1` and base58 from a fixed seed, `len` characters in all.
fn pairing_like(len: usize) -> String {
    let mut seed: u32 = 0x2545_F491;
    let mut s = String::from("flankr1");
    while s.len() < len {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        s.push(BASE58[seed as usize % BASE58.len()] as char);
    }
    s
}

fn pairing_url(len: usize) -> String {
    format!(
        "flankr://m4.tail1a2b3c.ts.net:7347/pair#{}",
        pairing_like(len)
    )
}

/// The PNG as 8-bit luma: (side, pixels).
fn decode_png(bytes: &[u8]) -> (usize, Vec<u8>) {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    {
        let info = reader.info();
        assert_eq!(info.color_type, png::ColorType::Grayscale);
        assert_eq!(info.bit_depth, png::BitDepth::One);
        assert_eq!(info.width, info.height);
    }
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut buf).unwrap();
    assert_eq!(frame.bit_depth, png::BitDepth::Eight);
    buf.truncate(frame.buffer_size());
    (frame.width as usize, buf)
}

/// What `rqrr` reads from the PNG.
fn read_back(bytes: &[u8]) -> String {
    let (side, luma) = decode_png(bytes);
    let mut img =
        rqrr::PreparedImage::prepare_from_greyscale(side, side, |x, y| luma[y * side + x]);
    let grids = img.detect_grids();
    assert_eq!(grids.len(), 1, "one code in the image");
    match grids[0].decode() {
        Ok((_, text)) => text,
        Err(e) => format!("<{e:?}>"),
    }
}

#[test]
fn every_level_and_scale_reads_back() {
    let texts = [
        pairing_like(136),
        pairing_url(184),
        pairing_url(241),
        "héllo wörld".to_string(),
    ];
    for text in &texts {
        for ecc in [Ecc::L, Ecc::M, Ecc::Q, Ecc::H] {
            let qr = Qr::with_ecc(text, ecc).unwrap();
            // rqrr cannot sample the densest of these (version 18 at H) at
            // one pixel per module; scale 1 is checked below on its own.
            for scale in [2, 8] {
                let png = qr.png(Png::new(scale, 4).unwrap());
                assert_eq!(&read_back(&png), text, "{ecc:?}, scale {scale}");
            }
        }
    }
}

#[test]
fn one_pixel_per_module_reads_back() {
    let text = pairing_url(184);
    let png = Qr::new(&text).unwrap().png(Png::new(1, 4).unwrap());
    assert_eq!(read_back(&png), text);
}

#[test]
fn chunks_crcs_and_header() {
    let qr = Qr::new(&pairing_url(184)).unwrap();
    let png = qr.png(Png::default());
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let mut kinds = Vec::new();
    let mut at = 8;
    while at < png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let body = &png[at + 4..at + 8 + len];
        let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
        assert_eq!(crc, crc32(body), "{}", String::from_utf8_lossy(&body[..4]));
        kinds.push(String::from_utf8_lossy(&body[..4]).into_owned());
        if &body[..4] == b"IHDR" {
            let side = qr.png_side(Png::default());
            assert_eq!(&body[4..8], &side.to_be_bytes());
            assert_eq!(&body[8..12], &side.to_be_bytes());
            assert_eq!(&body[12..17], &[1, 0, 0, 0, 0]);
        }
        at += 12 + len;
    }
    assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
    assert_eq!(&png[png.len() - 4..], &[0xAE, 0x42, 0x60, 0x82]);
    println!("a 184-character pairing URL: {} bytes of PNG", png.len());
    assert!(png.len() < 4096);
}

/// An independent CRC-32, bit by bit.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn quiet_zone_finder_and_odd_sides() {
    let qr = Qr::new(&pairing_like(136)).unwrap();
    let (side, luma) = decode_png(&qr.png(Png::default()));
    assert_eq!(side, (qr.modules() + 8) * 8);
    let at = |x: usize, y: usize| luma[y * side + x];
    for i in 0..side {
        for q in 0..32 {
            assert_eq!(at(i, q), 255);
            assert_eq!(at(q, i), 255);
            assert_eq!(at(i, side - 1 - q), 255);
            assert_eq!(at(side - 1 - q, i), 255);
        }
    }
    // The top-left finder pattern's corner.
    assert_eq!(at(32, 32), 0);

    // A side that is not a multiple of 8 pads its rows.
    let png = qr.png(Png::new(3, 4).unwrap());
    let (side, _) = decode_png(&png);
    assert_eq!(side, (qr.modules() + 8) * 3);
    assert_ne!(side % 8, 0);
    assert_eq!(read_back(&png), pairing_like(136));
}

#[test]
fn versions_at_level_m() {
    for (len, version) in [(136, 8), (184, 10), (211, 10), (241, 11)] {
        let qr = Qr::new(&pairing_like(len)).unwrap();
        assert_eq!(qr.version(), version, "{len} characters");
        assert_eq!(qr.modules(), 17 + 4 * version as usize);
    }
}

#[test]
fn both_base64_forms_carry_the_png() {
    let qr = Qr::new(&pairing_url(184)).unwrap();
    let png = qr.png(Png::default());
    let url = qr.data_url(Png::default());
    let b64 = url.strip_prefix("data:image/png;base64,").unwrap();
    assert_eq!(STANDARD.decode(b64).unwrap(), png);
    let b64url = qr.base64url(Png::default());
    assert!(!b64url.contains(['+', '/', '=']));
    assert_eq!(URL_SAFE_NO_PAD.decode(b64url).unwrap(), png);
    assert_eq!(qrcodeurl::data_url(&pairing_url(184)).unwrap(), url);
}

#[test]
fn terminal_size_and_colours() {
    let qr = Qr::new(&pairing_url(184)).unwrap();
    let side = qr.modules() + 8;
    let light = qr.terminal(Terminal::LightOnDark);
    let dark = qr.terminal(Terminal::DarkOnLight);
    let lines: Vec<&str> = light.lines().collect();
    assert_eq!(lines.len(), side.div_ceil(2));
    assert!(lines.iter().all(|l| l.chars().count() == side));
    assert!(light.contains(['█', '▀', '▄']));
    assert_ne!(light, dark);
}

#[test]
fn inline_images() {
    let qr = Qr::new(&pairing_url(184)).unwrap();
    let png = qr.png(Png::default());

    let kitty = qr.inline(Png::default(), Inline::Kitty);
    assert!(kitty.starts_with("\x1b_Ga=T,f=100,q=2,m="));
    assert!(kitty.ends_with("\x1b\\\n"));
    let mut b64 = String::new();
    let frames: Vec<&str> = kitty
        .trim_end()
        .split("\x1b\\")
        .filter(|f| !f.is_empty())
        .collect();
    for (i, frame) in frames.iter().enumerate() {
        let (head, data) = frame.split_once(';').unwrap();
        let last = i + 1 == frames.len();
        assert!(head.ends_with(if last { "m=0" } else { "m=1" }), "{head}");
        assert!(data.len() <= 4096);
        b64.push_str(data);
    }
    assert_eq!(STANDARD.decode(b64).unwrap(), png);

    // Big enough for several chunks.
    let big = Qr::new(&pairing_url(241)).unwrap();
    let chunks = big
        .inline(Png::new(32, 4).unwrap(), Inline::Kitty)
        .matches("\x1b_G")
        .count();
    assert!(chunks > 1, "{chunks}");

    let iterm = qr.inline(Png::default(), Inline::Iterm2);
    let head = format!(
        "\x1b]1337;File=inline=1;size={};preserveAspectRatio=1:",
        png.len()
    );
    let data = iterm
        .strip_prefix(&head)
        .unwrap()
        .strip_suffix("\x07\n")
        .unwrap();
    assert_eq!(STANDARD.decode(data).unwrap(), png);
}

#[test]
fn errors_never_show_the_text() {
    let secret = "s3cret".repeat(500);
    let err = Qr::new(&secret).unwrap_err();
    assert_eq!(
        err,
        Error::TooLong {
            len: 3000,
            ecc: Ecc::M
        }
    );
    assert!(!err.to_string().contains("s3cret"));
    assert!(!format!("{err:?}").contains("s3cret"));
    let qr = Qr::new("s3cret").unwrap();
    assert!(!format!("{qr:?}").contains("s3cret"));
    assert_eq!(Png::new(0, 4), Err(Error::BadSize));
    assert_eq!(Png::new(33, 4), Err(Error::BadSize));
    assert_eq!(Png::new(8, 17), Err(Error::BadSize));
}
