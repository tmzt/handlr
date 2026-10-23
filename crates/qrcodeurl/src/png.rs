//! A 1-bit grayscale PNG, written by hand (ISO/IEC 15948): the signature,
//! `IHDR`, one `IDAT` deflated with `miniz_oxide`, and `IEND`.

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// zlib level for the `IDAT`: the image is tiny, so the best is cheap.
const LEVEL: u8 = 9;

/// A `side × side` PNG where `dark(x, y)` says which pixels are dark.
/// Light is bit 1 and dark bit 0, as PNG grayscale defines them; each row is
/// filter byte 0 (None), then pixels MSB first, padded with light bits.
pub(crate) fn encode(side: u32, dark: impl Fn(u32, u32) -> bool) -> Vec<u8> {
    let row_bytes = side.div_ceil(8) as usize;
    let mut raw = Vec::with_capacity((row_bytes + 1) * side as usize);
    for y in 0..side {
        raw.push(0);
        let start = raw.len();
        raw.resize(start + row_bytes, 0xFF);
        for x in (0..side).filter(|&x| dark(x, y)) {
            raw[start + (x / 8) as usize] &= !(0x80 >> (x % 8));
        }
    }

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&side.to_be_bytes());
    ihdr.extend_from_slice(&side.to_be_bytes());
    // Bit depth 1, colour type 0 (grayscale), compression, filter and
    // interlace methods 0.
    ihdr.extend_from_slice(&[1, 0, 0, 0, 0]);
    let idat = miniz_oxide::deflate::compress_to_vec_zlib(&raw, LEVEL);

    let mut out = Vec::with_capacity(SIGNATURE.len() + 3 * 12 + ihdr.len() + idat.len());
    out.extend_from_slice(&SIGNATURE);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Length, type, data, and the CRC of the type and data.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len()).expect("a chunk under 4 GiB");
    out.extend_from_slice(&len.to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// CRC-32 (ISO-HDLC), as PNG uses it.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0u32, |crc, &b| {
        CRC_TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8)
    })
}

const CRC_TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn a_row_is_padded_with_light_bits() {
        // 3 × 3, dark on the diagonal: rows 0b0111_1111, 0b1011_1111, 0b1101_1111.
        let png = encode(3, |x, y| x == y);
        assert_eq!(&png[..8], &SIGNATURE);
        let idat_len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&png[41..41 + idat_len]).unwrap();
        assert_eq!(raw, [0, 0x7F, 0, 0xBF, 0, 0xDF]);
    }
}
