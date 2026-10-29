//! The preview's image body (docs/design/20261028_05_BINARY_FILES_PLAN.md,
//! decision 10.3, as the 21:29 EDT amendment's item 6 and the 23:48 EDT
//! amendment leave it): on the desktop a PNG is decoded with the `png`
//! crate (an image over 8192 px on a side or 40 megapixels is refused
//! before anything is allocated for its pixels), downscaled by an integer
//! factor to at most 4096 px on its long side, and drawn as one texture
//! ([`super::gpu`]); fit to the width, pinch zoom 1×-8×, pan, and a double
//! tap toggles fit and 1:1. JPEG, WebP and GIF need Android's decoder,
//! which libmorph does not surface yet, so on Android every image, and on
//! the desktop every one but a PNG, shows a line instead.

use libcontrols::Rect;

/// The longest side decoded, and the most pixels.
pub const SIDE_MAX: u32 = 8192;
pub const PIXELS_MAX: u64 = 40_000_000;
/// The longest side of the texture.
pub const TEXTURE_MAX: u32 = 4096;
/// The zoom, times the fit.
pub const ZOOM_MAX: f32 = 8.0;

/// Decoded pixels, RGBA8, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Why an image is not shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoImage {
    /// Over [`SIDE_MAX`] or [`PIXELS_MAX`].
    TooLarge,
    /// Not decodable here: another type than PNG, or Android.
    Unsupported,
    /// The bytes are not a good PNG.
    Damaged,
}

/// Whether a `width` × `height` image may be decoded.
pub fn fits(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= SIDE_MAX
        && height <= SIDE_MAX
        && u64::from(width) * u64::from(height) <= PIXELS_MAX
}

/// `bytes` of type `mime` decoded for the preview, downscaled to at most
/// [`TEXTURE_MAX`] on its long side.
pub fn decode(bytes: &[u8], mime: &str) -> Result<Rgba, NoImage> {
    if cfg!(target_os = "android") || mime != "image/png" {
        return Err(NoImage::Unsupported);
    }
    let rgba = decode_png(bytes)?;
    Ok(downscale(rgba, TEXTURE_MAX))
}

/// A PNG as RGBA8; its size is checked from the header first.
pub fn decode_png(bytes: &[u8]) -> Result<Rgba, NoImage> {
    // The size from IHDR, before the decoder reads on.
    if bytes.len() >= 24 && &bytes[12..16] == b"IHDR" {
        let be = |at: usize| {
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        if !fits(be(16), be(20)) {
            return Err(NoImage::TooLarge);
        }
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|_| NoImage::Damaged)?;
    let (w, h) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if !fits(w, h) {
        return Err(NoImage::TooLarge);
    }
    let size = reader.output_buffer_size().ok_or(NoImage::TooLarge)?;
    let mut buf = vec![0; size];
    let frame = reader.next_frame(&mut buf).map_err(|_| NoImage::Damaged)?;
    let (width, height) = (frame.width, frame.height);
    let samples = frame.color_type.samples();
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in buf[..frame.buffer_size()]
        .chunks(frame.line_size)
        .take(height as usize)
    {
        for px in row.chunks(samples).take(width as usize) {
            let rgba = match *px {
                [g] => [g, g, g, 255],
                [g, a] => [g, g, g, a],
                [r, g, b] => [r, g, b, 255],
                [r, g, b, a] => [r, g, b, a],
                _ => [0, 0, 0, 0],
            };
            pixels.extend_from_slice(&rgba);
        }
    }
    Ok(Rgba {
        width,
        height,
        pixels,
    })
}

/// `img` box-filtered down by the smallest integer factor that brings its
/// long side to at most `max`.
pub fn downscale(img: Rgba, max: u32) -> Rgba {
    let long = img.width.max(img.height);
    let k = long.div_ceil(max.max(1)).max(1);
    if k == 1 {
        return img;
    }
    let (w, h) = (img.width / k, img.height / k);
    let (w, h) = (w.max(1), h.max(1));
    let mut out = vec![0u8; w as usize * h as usize * 4];
    let stride = img.width as usize * 4;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut sum = [0u32; 4];
            for dy in 0..k as usize {
                let row = (y * k as usize + dy) * stride;
                for dx in 0..k as usize {
                    let at = row + (x * k as usize + dx) * 4;
                    for (c, v) in sum.iter_mut().enumerate() {
                        *v += u32::from(img.pixels[at + c]);
                    }
                }
            }
            let n = k * k;
            let at = (y * w as usize + x) * 4;
            for c in 0..4 {
                out[at + c] = (sum[c] / n) as u8;
            }
        }
    }
    Rgba {
        width: w,
        height: h,
        pixels: out,
    }
}

/// Where the image is in its window: the zoom over the fit, and the pan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// The image's size in pixels (the texture's).
    pub size: [f32; 2],
    /// 1 is fit to the width; at most [`ZOOM_MAX`].
    pub zoom: f32,
    /// The image's top-left corner, from the window's, in pixels.
    pub pan: [f32; 2],
}

impl View {
    pub fn new(width: u32, height: u32) -> View {
        View {
            size: [width as f32, height as f32],
            zoom: 1.0,
            pan: [0.0, 0.0],
        }
    }

    /// The fit's scale in `window`: the image across its width.
    pub fn fit(&self, window: Rect) -> f32 {
        if self.size[0] <= 0.0 {
            1.0
        } else {
            window.w / self.size[0]
        }
    }

    /// Where it is drawn in `window`.
    pub fn rect(&self, window: Rect) -> Rect {
        let scale = self.fit(window) * self.zoom;
        let (w, h) = (self.size[0] * scale, self.size[1] * scale);
        Rect::new(window.x + self.pan[0], window.y + self.pan[1], w, h)
    }

    /// The pan kept so the image covers the window where it can, and is
    /// centred where it is smaller.
    pub fn clamp(&mut self, window: Rect) {
        let r = self.rect(window);
        for (axis, (len, room)) in [(r.w, window.w), (r.h, window.h)].into_iter().enumerate() {
            self.pan[axis] = if len <= room {
                (room - len) * 0.5
            } else {
                self.pan[axis].clamp(room - len, 0.0)
            };
        }
    }

    /// Zooms to `zoom` (kept to 1-[`ZOOM_MAX`]) about `at`, a point in the
    /// window.
    pub fn zoom_about(&mut self, zoom: f32, at: [f32; 2], window: Rect) {
        let zoom = zoom.clamp(1.0, ZOOM_MAX);
        let ratio = zoom / self.zoom;
        for (axis, origin) in [window.x, window.y].into_iter().enumerate() {
            let p = at[axis] - origin;
            self.pan[axis] = p - (p - self.pan[axis]) * ratio;
        }
        self.zoom = zoom;
        self.clamp(window);
    }

    /// A double tap: 1:1 (one image pixel a screen pixel) from the fit, the
    /// fit from anything else.
    pub fn toggle(&mut self, at: [f32; 2], window: Rect) {
        if (self.zoom - 1.0).abs() > 1e-3 {
            self.zoom_about(1.0, at, window);
        } else {
            let one = 1.0 / self.fit(window);
            self.zoom_about(one.max(1.0), at, window);
        }
    }

    /// A pan by `d` pixels.
    pub fn pan_by(&mut self, d: [f32; 2], window: Rect) {
        self.pan[0] += d[0];
        self.pan[1] += d[1];
        self.clamp(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let data: Vec<u8> = (0..w * h * 3).map(|i| (i % 251) as u8).collect();
            enc.write_header().unwrap().write_image_data(&data).unwrap();
        }
        out
    }

    /// A PNG header claiming `w` × `h`, with no pixels.
    fn header_only(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        b.extend_from_slice(&13u32.to_be_bytes());
        let mut chunk = b"IHDR".to_vec();
        chunk.extend_from_slice(&ihdr);
        let crc = crc32(&chunk);
        b.extend_from_slice(&chunk);
        b.extend_from_slice(&crc.to_be_bytes());
        b
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &byte in data {
            c ^= u32::from(byte);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }

    #[test]
    #[cfg(not(target_os = "android"))]
    fn a_png_decodes_and_others_are_not_decoded_here() {
        let img = decode(&png(3, 2), "image/png").unwrap();
        assert_eq!((img.width, img.height, img.pixels.len()), (3, 2, 24));
        assert_eq!(&img.pixels[..4], &[0, 1, 2, 255]);
        assert_eq!(decode(&png(3, 2), "image/jpeg"), Err(NoImage::Unsupported));
        assert_eq!(decode(b"not a png", "image/png"), Err(NoImage::Damaged));
    }

    #[test]
    fn over_8192_px_or_40_mp_is_refused_before_decoding() {
        assert_eq!(decode_png(&header_only(8193, 10)), Err(NoImage::TooLarge));
        assert_eq!(decode_png(&header_only(10, 8193)), Err(NoImage::TooLarge));
        assert_eq!(decode_png(&header_only(8000, 6000)), Err(NoImage::TooLarge));
        assert!(fits(8192, 4882));
        assert!(!fits(8192, 4883));
    }

    #[test]
    fn a_large_image_is_downscaled_by_an_integer_factor() {
        let img = Rgba {
            width: 9000,
            height: 4,
            pixels: vec![200; 9000 * 4 * 4],
        };
        let d = downscale(img, TEXTURE_MAX);
        assert_eq!((d.width, d.height), (3000, 1));
        assert!(d.pixels.iter().all(|&p| p == 200));
        let small = Rgba {
            width: 10,
            height: 10,
            pixels: vec![1; 400],
        };
        assert_eq!(downscale(small.clone(), TEXTURE_MAX), small);
    }

    #[test]
    fn the_view_fits_zooms_pans_and_toggles() {
        let window = Rect::new(0.0, 100.0, 400.0, 800.0);
        let mut v = View::new(800, 400);
        v.clamp(window);
        // Fit to the width, centred down.
        assert_eq!(v.rect(window), Rect::new(0.0, 400.0, 400.0, 200.0));
        v.zoom_about(20.0, [200.0, 500.0], window);
        assert_eq!(v.zoom, ZOOM_MAX);
        v.zoom_about(0.5, [200.0, 500.0], window);
        assert_eq!(v.zoom, 1.0);
        // A double tap: 1:1, the image's pixels the screen's.
        v.toggle([200.0, 500.0], window);
        assert_eq!(v.rect(window).w, 800.0);
        v.pan_by([-10_000.0, 0.0], window);
        assert_eq!(v.rect(window).x, -400.0);
        v.toggle([200.0, 500.0], window);
        assert_eq!(v.zoom, 1.0);
    }
}
