use crate::storable::body::BodyConfig;

#[derive(Debug, PartialEq)]
pub enum HeaderError {
    UnsupportedMajor(u8),
    Truncated,
    UnknownByteOrder,
}

#[derive(Debug,PartialEq)]
pub struct VersionByte {
    pub major: u8, // Major version number of the Storable format. Stored in the upper bits of the first byte.
    pub minor: u8, // Minor version number of the Storable format. Stored in the second byte.
    pub network_order: bool,
    pub arch: Option<ArchInfo>,
}

impl VersionByte {
    pub fn body_config(&self) -> BodyConfig {
        if let Some(arch) = &self.arch {
            BodyConfig {
                iv_size: arch.word_size,
                nv_size: arch.nv_size,
                network_order: false,
            }
        } else {
            BodyConfig {
                iv_size: 4,       // network order integers are 4 bytes big-endian
                nv_size: 8,
                network_order: true,
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ArchInfo {
    pub byteorder: ByteOrder,
    pub word_size: u8,     // NEW: 4 or 8, derived from byteorder string length
    pub int_size: u8,
    pub long_size: u8,
    pub ptr_size: u8,
    pub nv_size: u8,
}

#[derive(Debug, PartialEq)]
pub enum ByteOrder {
    Little,
    Big,
}

/*
"1234"     → Little endian, 32-bit
"4321"     → Big endian, 32-bit
"12345678" → Little endian, 64-bit
"87654321" → Big endian, 64-bit
*/

pub fn parse(input: &[u8]) -> Result<(VersionByte, &[u8]), HeaderError> {

    // freeze() and nfreeze() write raw streams. store() and nstore() write streams with a "pst0" prefix (called 'magic').
    let input = match input.strip_prefix(b"pst0") {
    Some(rest) => rest,
    None => input,
    };
    
    if input.len() < 2 {
    return Err(HeaderError::Truncated);
    }
    
    let b = input[0];
    let major = b >> 1;
    let minor = input[1];
    let network_order = (b & 0x01) != 0;

    if major != 2 {
        return Err(HeaderError::UnsupportedMajor(major));
    } 

    if network_order {
        return Ok((
            VersionByte { major, minor, network_order, arch: None },
            &input[2..],
        ));
    }

    // The following is for native order; we read the architecture block.

    if input.len() < 3 {
        return Err(HeaderError::Truncated);
    }

    let byte_order_length = input[2] as usize;

    if input.len() < 3 + byte_order_length {
        return Err(HeaderError::Truncated);
    }

    let byte_order_string = &input[3..3 + byte_order_length];

    let (byteorder, word_size) = match byte_order_string {
        b"1234" => (ByteOrder::Little, 4),
        b"4321" => (ByteOrder::Big, 4),
        b"12345678" => (ByteOrder::Little, 8),
        b"87654321" => (ByteOrder::Big, 8),
        _ => return Err(HeaderError::UnknownByteOrder),
    };

    let offset = 3 + byte_order_length;
    if input.len() < offset + 4 {
        return Err(HeaderError::Truncated);
    }

    let arch = ArchInfo {
        byteorder,
        word_size,
        int_size: input[offset],
        long_size: input[offset + 1],
        ptr_size: input[offset + 2],
        nv_size: input[offset + 3],
    };

    Ok((
        VersionByte { major, minor, network_order, arch: Some(arch) },
        &input[offset + 4..],
    ))

}