use crate::sereal::{Cursor, SerealError};

pub const MAGIC_V1_V2: [u8; 4] = [0x3D, 0x73, 0x72, 0x6C]; // "=srl"
pub const MAGIC_V3_PLUS: [u8; 4] = [0x3D, 0xF3, 0x72, 0x6C]; // "=\xF3rl"

#[derive(Debug, PartialEq)]
pub enum Encoding {
    Raw,
    SnappyLegacy,
    SnappyIncremental,
    Zlib,
    Zstd,
}

#[derive(Debug, PartialEq)]
pub struct Header {
    pub version: u8,
    pub encoding: Encoding,
}

pub fn parse<'a>(input: &'a [u8]) -> Result<(Header, &'a [u8]), SerealError> {
    if input.len() < 5 {
        return Err(SerealError::Truncated);
    }

    let magic = &input[0..4];
    if magic != MAGIC_V1_V2 && magic != MAGIC_V3_PLUS {
        return Err(SerealError::BadMagic);
    }

    let version_type = input[4];
    let version = version_type & 0x0F;
    let encoding_bits = version_type >> 4;

    let encoding = match encoding_bits {
        0 => Encoding::Raw,
        1 => Encoding::SnappyLegacy,
        2 => Encoding::SnappyIncremental,
        3 => Encoding::Zlib,
        4 => Encoding::Zstd,
        other => return Err(SerealError::UnknownEncoding(other)),
    };

    // Enforce magic/version pairing: v1/v2 use old magic, v3+ use new magic
    if version <= 2 && magic != MAGIC_V1_V2 {
        return Err(SerealError::MagicVersionMismatch);
    }
    if version >= 3 && magic != MAGIC_V3_PLUS {
        return Err(SerealError::MagicVersionMismatch);
    }

    let mut cursor = Cursor::new(&input[5..]);
    let suffix_len = cursor.read_varint()? as usize;
    let after_varint = cursor.remaining();

    if after_varint.len() < suffix_len {
        return Err(SerealError::Truncated);
    }

    let body_start = &after_varint[suffix_len..];

    Ok((Header { version, encoding }, body_start))
}