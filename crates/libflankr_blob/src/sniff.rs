//! Images by their magic bytes (decision 4.9).

/// The four image types a transfer may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageType {
    Png,
    Jpeg,
    WebP,
    Gif,
}

/// How many leading bytes [`ImageType::sniff`] needs.
pub const SNIFF_LEN: usize = 12;

impl ImageType {
    /// The type of the image whose first bytes are `head`, from at least
    /// the first 12 bytes: PNG `89 50 4E 47 0D 0A 1A 0A`, JPEG `FF D8 FF`,
    /// GIF `GIF87a` or `GIF89a`, WebP `RIFF` at 0 and `WEBP` at 8. Fewer
    /// than 12 bytes is `None`.
    pub fn sniff(head: &[u8]) -> Option<ImageType> {
        if head.len() < SNIFF_LEN {
            return None;
        }
        if head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(ImageType::Png)
        } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(ImageType::Jpeg)
        } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
            Some(ImageType::Gif)
        } else if head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
            Some(ImageType::WebP)
        } else {
            None
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            ImageType::Png => "image/png",
            ImageType::Jpeg => "image/jpeg",
            ImageType::WebP => "image/webp",
            ImageType::Gif => "image/gif",
        }
    }

    /// The file extension, without the dot.
    pub fn ext(self) -> &'static str {
        match self {
            ImageType::Png => "png",
            ImageType::Jpeg => "jpg",
            ImageType::WebP => "webp",
            ImageType::Gif => "gif",
        }
    }

    /// The type a MIME string names exactly, if one of the four.
    pub fn from_mime(mime: &str) -> Option<ImageType> {
        Some(match mime {
            "image/png" => ImageType::Png,
            "image/jpeg" => ImageType::Jpeg,
            "image/webp" => ImageType::WebP,
            "image/gif" => ImageType::Gif,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn padded(magic: &[u8]) -> Vec<u8> {
        let mut v = magic.to_vec();
        v.resize(16, 0);
        v
    }

    #[test]
    fn each_type_sniffs_by_its_magic() {
        let png = padded(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(ImageType::sniff(&png), Some(ImageType::Png));
        assert_eq!(
            ImageType::sniff(&padded(&[0xFF, 0xD8, 0xFF, 0xE0])),
            Some(ImageType::Jpeg)
        );
        assert_eq!(ImageType::sniff(&padded(b"GIF87a")), Some(ImageType::Gif));
        assert_eq!(ImageType::sniff(&padded(b"GIF89a")), Some(ImageType::Gif));
        assert_eq!(
            ImageType::sniff(b"RIFF\0\0\0\0WEBPVP8 "),
            Some(ImageType::WebP)
        );
        assert_eq!(ImageType::sniff(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(ImageType::sniff(&padded(b"GIF88a")), None);
        assert_eq!(ImageType::sniff(&padded(b"%PDF-1.7")), None);
        // 11 bytes are not enough, even of a PNG.
        assert_eq!(ImageType::sniff(&png[..11]), None);
        assert_eq!(ImageType::sniff(&png[..12]), Some(ImageType::Png));
        // A PNG named JPEG: the bytes say PNG, which is not the MIME's.
        let named = ImageType::from_mime("image/jpeg").unwrap();
        assert_ne!(ImageType::sniff(&png), Some(named));
    }

    #[test]
    fn mime_and_ext_round_trip() {
        for t in [
            ImageType::Png,
            ImageType::Jpeg,
            ImageType::WebP,
            ImageType::Gif,
        ] {
            assert_eq!(ImageType::from_mime(t.mime()), Some(t));
        }
        assert_eq!(ImageType::Jpeg.ext(), "jpg");
        assert_eq!(ImageType::from_mime("image/heic"), None);
        assert_eq!(ImageType::from_mime("IMAGE/PNG"), None);
    }
}
