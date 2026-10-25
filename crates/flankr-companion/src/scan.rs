//! What [`Input::Pair`](crate::Input::Pair) is given (plan 20261022_06,
//! step 5, from the 23:07 EDT amendment): the pairing URL
//! `flankr://<host>:<port>/pair#flankr1…`, the bare `flankr1…` string, or
//! a `data:image/png;base64,` URL of the QR code `flankr pair` prints. The
//! QR code must hold one of the first two forms.
//!
//! The PNG is read with `png` (expanded to 8-bit samples, then to luma)
//! and the code with `rqrr`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use libflankr_proto::link::PairingPayload;

/// The only data URL accepted.
pub const DATA_URL_PREFIX: &str = "data:image/png;base64,";

/// The longest data URL read: far more than a pairing QR code needs.
const MAX_DATA_URL: usize = 4 << 20;

/// The server a pairing input names, and what to call it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pairing {
    pub payload: PairingPayload,
    /// The first DNS label of the URL's host, or of the first address's
    /// host for a bare string: `m4` for `m4.tail1a2b3c.ts.net`. An IP
    /// address is kept whole.
    pub server: String,
}

/// Reads a pairing input in any of its three forms.
pub fn read(input: &str) -> Result<Pairing, String> {
    let input = input.trim();
    let text = if input.starts_with("data:") {
        let text = read_data_url(input)?;
        if text.trim().starts_with("data:") {
            return Err("the QR code holds another data URL".to_string());
        }
        text
    } else {
        input.to_string()
    };
    let text = text.trim();
    let payload = PairingPayload::decode(text).map_err(|e| e.to_string())?;
    if payload.addresses.is_empty() {
        return Err("the pairing string has no addresses".to_string());
    }
    // `decode` has checked that a URL's host is one of the addresses.
    let address = text
        .strip_prefix("flankr://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(&payload.addresses[0]);
    Ok(Pairing {
        server: server_name(address),
        payload,
    })
}

/// `m4` for `m4.tail1a2b3c.ts.net:7347`; `127.0.0.1` for `127.0.0.1:7347`.
fn server_name(address: &str) -> String {
    let host = match address.rsplit_once(':') {
        Some((host, port)) if port.parse::<u16>().is_ok() => host,
        _ => address,
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_string();
    }
    host.split('.').next().unwrap_or(host).to_string()
}

/// The text of the QR code in a `data:image/png;base64,` URL.
pub fn read_data_url(url: &str) -> Result<String, String> {
    let b64 = url
        .strip_prefix(DATA_URL_PREFIX)
        .ok_or("only a data:image/png;base64, URL can be read")?;
    if b64.len() > MAX_DATA_URL {
        return Err("the data URL is too large".to_string());
    }
    let b64: String = b64.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let png = STANDARD
        .decode(b64)
        .map_err(|e| format!("the data URL is not base64: {e}"))?;
    read_qr(&png)
}

/// The text of the QR code in a PNG.
pub fn read_qr(png_bytes: &[u8]) -> Result<String, String> {
    let (width, height, luma) = luma(png_bytes).map_err(|e| format!("not a PNG: {e}"))?;
    let mut img =
        rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| luma[y * width + x]);
    img.detect_grids()
        .iter()
        .find_map(|grid| grid.decode().ok().map(|(_, text)| text))
        .ok_or_else(|| "no QR code could be read in the image".to_string())
}

/// The PNG as 8-bit luma, row by row, with any alpha over white.
fn luma(png_bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), png::DecodingError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let size = reader
        .output_buffer_size()
        .ok_or(png::DecodingError::LimitsExceeded)?;
    let mut buf = vec![0; size];
    let frame = reader.next_frame(&mut buf)?;
    let (width, height) = (frame.width as usize, frame.height as usize);
    let samples = frame.color_type.samples();
    let mut out = Vec::with_capacity(width * height);
    for row in buf[..frame.buffer_size()]
        .chunks(frame.line_size)
        .take(height)
    {
        for px in row.chunks(samples).take(width) {
            let (value, alpha) = match px {
                [g] => (u32::from(*g), 255),
                [g, a] => (u32::from(*g), u32::from(*a)),
                [r, g, b] => (rgb_luma(*r, *g, *b), 255),
                [r, g, b, a, ..] => (rgb_luma(*r, *g, *b), u32::from(*a)),
                [] => (255, 255),
            };
            out.push(((value * alpha + 255 * (255 - alpha)) / 255) as u8);
        }
    }
    Ok((width, height, out))
}

fn rgb_luma(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_proto::link::generate_keypair;

    fn payload(addresses: &[&str]) -> PairingPayload {
        PairingPayload {
            server_key: generate_keypair().unwrap().public,
            addresses: addresses.iter().map(|a| a.to_string()).collect(),
        }
    }

    fn data_url(text: &str) -> String {
        format!(
            "{DATA_URL_PREFIX}{}",
            STANDARD.encode(qrcodeurl::png(text).unwrap())
        )
    }

    #[test]
    fn the_three_forms_name_the_same_server() {
        let p = payload(&["m4.tail1a2b3c.ts.net:7347", "127.0.0.1:7347"]);
        let url = p.url().unwrap();
        let string = p.encode().unwrap();
        for input in [
            url.clone(),
            string.clone(),
            data_url(&url),
            data_url(&string),
        ] {
            let read = read(&format!("  {input}\n")).unwrap();
            assert_eq!(read.payload, p);
            assert_eq!(read.server, "m4");
        }
    }

    #[test]
    fn the_name_follows_the_url_host_or_the_first_address() {
        let p = payload(&["127.0.0.1:7347", "box.example.com:7347"]);
        // The URL prefers the remote address; the bare string has only the
        // first one.
        assert_eq!(read(&p.url().unwrap()).unwrap().server, "box");
        assert_eq!(read(&p.encode().unwrap()).unwrap().server, "127.0.0.1");
        assert_eq!(server_name("[::1]:7347"), "::1");
        assert_eq!(server_name("mini"), "mini");
    }

    #[test]
    fn bad_inputs_are_refused() {
        let p = payload(&["m4.ts.net:7347"]);
        // A QR code of something else, a nested data URL, a JPEG data URL,
        // bad base64, a PNG with no code, and a broken checksum.
        assert!(read(&data_url("https://example.com/")).is_err());
        assert!(read(&data_url(&data_url("x"))).is_err());
        assert!(read("data:image/jpeg;base64,AAAA").is_err());
        assert!(read("data:image/png;base64,!!!").is_err());
        let mut blank = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut blank, 40, 40);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[255; 1600]).unwrap();
        }
        let blank = format!("{DATA_URL_PREFIX}{}", STANDARD.encode(blank));
        assert_eq!(
            read(&blank).unwrap_err(),
            "no QR code could be read in the image"
        );
        let mut string = p.encode().unwrap();
        let last = string.pop().unwrap();
        string.push(if last == '2' { '3' } else { '2' });
        assert!(
            read(&string)
                .unwrap_err()
                .contains("invalid pairing string")
        );
        // No addresses.
        assert!(read(&payload(&[]).encode().unwrap()).is_err());
    }

    #[test]
    fn rgba_pngs_are_read_too() {
        // The QR code's PNG, redrawn as RGBA: what a screenshot gives.
        let text = payload(&["m4.ts.net:7347"]).url().unwrap();
        let png_bytes = qrcodeurl::png(&text).unwrap();
        let (w, h, gray) = luma(&png_bytes).unwrap();
        let rgba: Vec<u8> = gray.iter().flat_map(|&v| [v, v, v, 255]).collect();
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&rgba).unwrap();
        }
        assert_eq!(read_qr(&out).unwrap(), text);
    }
}
