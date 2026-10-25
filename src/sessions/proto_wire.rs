//! Minimal protobuf wire decoder for session steps (plan 20261023_06, step 3).
//!
//! Decodes varints, 32-bit/64-bit fixed numbers, and length-delimited bytes,
//! strings and nested messages from raw wire bytes. Truncated blobs return
//! an error, never panic.

use std::fmt;

pub const WIRE_VARINT: u8 = 0;
pub const WIRE_FIXED64: u8 = 1;
pub const WIRE_LENGTH_DELIMITED: u8 = 2;
pub const WIRE_START_GROUP: u8 = 3;
pub const WIRE_END_GROUP: u8 = 4;
pub const WIRE_FIXED32: u8 = 5;

/// Errors when decoding protobuf wire bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    UnexpectedEof,
    VarintOverflow,
    InvalidWireType(u8),
    InvalidUtf8,
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of protobuf buffer"),
            Self::VarintOverflow => write!(f, "varint exceeds 10 bytes or 64 bits"),
            Self::InvalidWireType(wt) => write!(f, "invalid or unsupported wire type: {wt}"),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 in string field"),
        }
    }
}

impl std::error::Error for ProtoError {}

/// Decode a varint from `buf`, advancing the slice past the varint bytes.
pub fn read_varint(buf: &mut &[u8]) -> Result<u64, ProtoError> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    let mut i = 0;
    while i < buf.len() {
        let b = buf[i];
        i += 1;
        if shift >= 64 {
            return Err(ProtoError::VarintOverflow);
        }
        result |= ((b & 0x7F) as u64) << shift;
        if (b & 0x80) == 0 {
            *buf = &buf[i..];
            return Ok(result);
        }
        shift += 7;
        if shift > 64 || (shift == 64 && i > 10) {
            return Err(ProtoError::VarintOverflow);
        }
    }
    Err(ProtoError::UnexpectedEof)
}

/// A decoded protobuf field value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value<'a> {
    Varint(u64),
    Fixed64(u64),
    LengthDelimited(&'a [u8]),
    Fixed32(u32),
}

/// A decoded protobuf field with its tag (field number) and wire type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field<'a> {
    pub tag: u32,
    pub wire_type: u8,
    pub value: Value<'a>,
}

/// Iterator over fields in a protobuf wire buffer.
pub struct FieldIter<'a> {
    buf: &'a [u8],
}

impl<'a> Iterator for FieldIter<'a> {
    type Item = Result<Field<'a>, ProtoError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        let key = match read_varint(&mut self.buf) {
            Ok(k) => k,
            Err(e) => {
                self.buf = &[];
                return Some(Err(e));
            }
        };
        let tag = (key >> 3) as u32;
        let wire_type = (key & 0x07) as u8;
        let value = match wire_type {
            WIRE_VARINT => match read_varint(&mut self.buf) {
                Ok(v) => Value::Varint(v),
                Err(e) => {
                    self.buf = &[];
                    return Some(Err(e));
                }
            },
            WIRE_FIXED64 => {
                if self.buf.len() < 8 {
                    self.buf = &[];
                    return Some(Err(ProtoError::UnexpectedEof));
                }
                let bytes: [u8; 8] = self.buf[..8].try_into().unwrap();
                self.buf = &self.buf[8..];
                Value::Fixed64(u64::from_le_bytes(bytes))
            }
            WIRE_LENGTH_DELIMITED => {
                let len = match read_varint(&mut self.buf) {
                    Ok(l) => l as usize,
                    Err(e) => {
                        self.buf = &[];
                        return Some(Err(e));
                    }
                };
                if self.buf.len() < len {
                    self.buf = &[];
                    return Some(Err(ProtoError::UnexpectedEof));
                }
                let slice = &self.buf[..len];
                self.buf = &self.buf[len..];
                Value::LengthDelimited(slice)
            }
            WIRE_FIXED32 => {
                if self.buf.len() < 4 {
                    self.buf = &[];
                    return Some(Err(ProtoError::UnexpectedEof));
                }
                let bytes: [u8; 4] = self.buf[..4].try_into().unwrap();
                self.buf = &self.buf[4..];
                Value::Fixed32(u32::from_le_bytes(bytes))
            }
            other => {
                self.buf = &[];
                return Some(Err(ProtoError::InvalidWireType(other)));
            }
        };
        Some(Ok(Field {
            tag,
            wire_type,
            value,
        }))
    }
}

/// A protobuf message slice supporting field and path queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message<'a> {
    bytes: &'a [u8],
}

impl<'a> Message<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn iter(&self) -> FieldIter<'a> {
        FieldIter { buf: self.bytes }
    }

    /// Read the first varint field with field number `tag`.
    pub fn varint(&self, tag: u32) -> Result<Option<u64>, ProtoError> {
        for f in self.iter() {
            let f = f?;
            if f.tag == tag
                && let Value::Varint(v) = f.value
            {
                return Ok(Some(v));
            }
        }
        Ok(None)
    }

    /// Read the first length-delimited byte slice with field number `tag`.
    pub fn bytes(&self, tag: u32) -> Result<Option<&'a [u8]>, ProtoError> {
        for f in self.iter() {
            let f = f?;
            if f.tag == tag
                && let Value::LengthDelimited(slice) = f.value
            {
                return Ok(Some(slice));
            }
        }
        Ok(None)
    }

    /// Read the first string field with field number `tag`.
    pub fn string(&self, tag: u32) -> Result<Option<&'a str>, ProtoError> {
        match self.bytes(tag)? {
            Some(slice) => std::str::from_utf8(slice)
                .map(Some)
                .map_err(|_| ProtoError::InvalidUtf8),
            None => Ok(None),
        }
    }

    /// Read the first submessage with field number `tag`.
    pub fn message(&self, tag: u32) -> Result<Option<Message<'a>>, ProtoError> {
        match self.bytes(tag)? {
            Some(slice) => Ok(Some(Message::new(slice))),
            None => Ok(None),
        }
    }

    /// Read all submessages with field number `tag` (for repeated fields).
    pub fn repeated_messages(&self, tag: u32) -> Result<Vec<Message<'a>>, ProtoError> {
        let mut out = Vec::new();
        for f in self.iter() {
            let f = f?;
            if f.tag == tag
                && let Value::LengthDelimited(slice) = f.value
            {
                out.push(Message::new(slice));
            }
        }
        Ok(out)
    }

    /// Walk a path of nested field numbers to a submessage.
    pub fn path_message(&self, path: &[u32]) -> Result<Option<Message<'a>>, ProtoError> {
        let mut current = *self;
        for &tag in path {
            match current.message(tag)? {
                Some(next) => current = next,
                None => return Ok(None),
            }
        }
        Ok(Some(current))
    }

    /// Walk a path to a string field: `path[..len-1]` as messages, last as string.
    pub fn path_string(&self, path: &[u32]) -> Result<Option<&'a str>, ProtoError> {
        let Some((&last, prefix)) = path.split_last() else {
            return Ok(None);
        };
        match self.path_message(prefix)? {
            Some(m) => m.string(last),
            None => Ok(None),
        }
    }

    /// Walk a path to a byte slice: `path[..len-1]` as messages, last as bytes.
    pub fn path_bytes(&self, path: &[u32]) -> Result<Option<&'a [u8]>, ProtoError> {
        let Some((&last, prefix)) = path.split_last() else {
            return Ok(None);
        };
        match self.path_message(prefix)? {
            Some(m) => m.bytes(last),
            None => Ok(None),
        }
    }

    /// Walk a path to a varint field: `path[..len-1]` as messages, last as varint.
    pub fn path_varint(&self, path: &[u32]) -> Result<Option<u64>, ProtoError> {
        let Some((&last, prefix)) = path.split_last() else {
            return Ok(None);
        };
        match self.path_message(prefix)? {
            Some(m) => m.varint(last),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_varint(mut n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let b = (n & 0x7F) as u8;
            n >>= 7;
            if n != 0 {
                out.push(b | 0x80);
            } else {
                out.push(b);
                break;
            }
        }
        out
    }

    fn encode_field(tag: u32, wire_type: u8, mut payload: Vec<u8>) -> Vec<u8> {
        let key = ((tag as u64) << 3) | (wire_type as u64);
        let mut out = encode_varint(key);
        out.append(&mut payload);
        out
    }

    fn encode_varint_field(tag: u32, val: u64) -> Vec<u8> {
        encode_field(tag, WIRE_VARINT, encode_varint(val))
    }

    fn encode_delimited_field(tag: u32, bytes: &[u8]) -> Vec<u8> {
        let mut payload = encode_varint(bytes.len() as u64);
        payload.extend_from_slice(bytes);
        encode_field(tag, WIRE_LENGTH_DELIMITED, payload)
    }

    #[test]
    fn decode_varints_valid() {
        let cases = [0u64, 1, 127, 128, 300, 16384, u32::MAX as u64, u64::MAX];
        for &n in &cases {
            let encoded = encode_varint(n);
            let mut slice = encoded.as_slice();
            let got = read_varint(&mut slice).expect("valid varint");
            assert_eq!(got, n);
            assert!(slice.is_empty());
        }
    }

    #[test]
    fn truncated_varint_returns_error_not_panic() {
        // High bit set indicating another byte follows, but buffer ends
        let truncated = [0x80u8];
        let mut slice = truncated.as_slice();
        let err = read_varint(&mut slice).unwrap_err();
        assert_eq!(err, ProtoError::UnexpectedEof);
    }

    #[test]
    fn overlong_varint_returns_error_not_panic() {
        // 11 bytes all with high bit set
        let overlong = [0x80; 11];
        let mut slice = overlong.as_slice();
        let err = read_varint(&mut slice).unwrap_err();
        assert_eq!(err, ProtoError::VarintOverflow);
    }

    #[test]
    fn decode_nested_messages_and_strings() {
        // Construct:
        // Message {
        //   1: 14 (varint),
        //   4: 3 (varint),
        //   5: Message {
        //     1: Message {
        //       1: 1729700000 (varint timestamp)
        //     }
        //   },
        //   19: Message {
        //     2: "Start the example build." (string)
        //   }
        // }
        let ts_inner = encode_varint_field(1, 1729700000);
        let ts_msg = encode_delimited_field(1, &ts_inner);
        let header_msg = encode_delimited_field(5, &ts_msg);

        let user_inner = encode_delimited_field(2, b"Start the example build.");
        let user_msg = encode_delimited_field(19, &user_inner);

        let mut data = Vec::new();
        data.extend_from_slice(&encode_varint_field(1, 14));
        data.extend_from_slice(&encode_varint_field(4, 3));
        data.extend_from_slice(&header_msg);
        data.extend_from_slice(&user_msg);

        let msg = Message::new(&data);
        assert_eq!(msg.varint(1).unwrap(), Some(14));
        assert_eq!(msg.varint(4).unwrap(), Some(3));
        assert_eq!(msg.path_varint(&[5, 1, 1]).unwrap(), Some(1729700000));
        assert_eq!(
            msg.path_string(&[19, 2]).unwrap(),
            Some("Start the example build.")
        );
    }

    #[test]
    fn truncated_length_delimited_field_returns_error() {
        let mut data = encode_varint_field(1, 10);
        // Field 2 declared as 20 bytes long, but only 3 bytes provided
        let key = (2u64 << 3) | (WIRE_LENGTH_DELIMITED as u64);
        data.extend_from_slice(&encode_varint(key));
        data.extend_from_slice(&encode_varint(20)); // length = 20
        data.extend_from_slice(b"abc"); // only 3 bytes

        let msg = Message::new(&data);
        let err = msg.string(2).unwrap_err();
        assert_eq!(err, ProtoError::UnexpectedEof);
    }

    #[test]
    fn invalid_wire_type_returns_error() {
        // Key with wire type 6 (reserved/invalid)
        let key = (1u64 << 3) | 6;
        let data = encode_varint(key);
        let msg = Message::new(&data);
        let mut iter = msg.iter();
        let err = iter.next().unwrap().unwrap_err();
        assert_eq!(err, ProtoError::InvalidWireType(6));
    }

    #[test]
    fn repeated_messages_collected() {
        // Two tool call messages at field 7
        let call1 = encode_delimited_field(1, b"c1");
        let call2 = encode_delimited_field(1, b"c2");

        let mut data = Vec::new();
        data.extend_from_slice(&encode_delimited_field(7, &call1));
        data.extend_from_slice(&encode_delimited_field(7, &call2));

        let msg = Message::new(&data);
        let calls = msg.repeated_messages(7).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].string(1).unwrap(), Some("c1"));
        assert_eq!(calls[1].string(1).unwrap(), Some("c2"));
    }
}
