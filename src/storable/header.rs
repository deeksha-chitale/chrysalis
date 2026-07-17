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

#[derive(Debug, PartialEq)]
pub struct ArchInfo {
    pub byteorder: ByteOrder,
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

    let byteorder = match byte_order_string {
        b"1234" | b"12345678" => ByteOrder::Little,
        b"4321" | b"87654321" => ByteOrder::Big,
        _ => return Err(HeaderError::UnknownByteOrder),
    };

    let offset = 3 + byte_order_length;
    if input.len() < offset + 4 {
        return Err(HeaderError::Truncated);
    }

    let arch = ArchInfo {
        byteorder,
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_magic_is_stripped() {
        let without_magic = &[0x05, 0x0B];
        let with_magic = &[b'p', b's', b't', b'0', 0x05, 0x0B];
        assert_eq!(parse(without_magic), parse(with_magic));
    }

    #[test]
    fn freeze_sets_native_order() {
        assert_eq!(
            parse(&[0x04, 0x0B, 0x08, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', 0x04, 0x04, 0x08, 0x08]),
            Ok((
                VersionByte {
                    major: 2,
                    minor: 11,
                    network_order: false,
                    arch: Some(ArchInfo {
                        byteorder: ByteOrder::Little,
                        int_size: 4,
                        long_size: 4,
                        ptr_size: 8,
                        nv_size: 8,
                    }),
                },
                &[][..],
            ))
        );
    }

    #[test]
    fn nfreeze_sets_network_order() {
        assert_eq!(
            parse(&[0x05, 0x0B]),
            Ok((
                VersionByte {
                    major: 2,
                    minor: 11,
                    network_order: true,
                    arch: None,
                },
                &[][..],
            ))
        );
    }

    #[test]
    fn ancient_major_is_rejected() {
        assert_eq!(
            parse(&[0x02, 0x00]),
            Err(HeaderError::UnsupportedMajor(1))
        );
    }

    #[test]
    fn truncated_input_errors() {
        assert_eq!(parse(&[]), Err(HeaderError::Truncated));
        assert_eq!(parse(&[0x04]), Err(HeaderError::Truncated));
    }

    #[test]
    fn unknown_byteorder_is_rejected() {
        // valid header bytes but garbage byteorder string "9999"
        assert_eq!(
            parse(&[0x04, 0x0B, 0x04, b'9', b'9', b'9', b'9', 0x04, 0x04, 0x08, 0x08]),
            Err(HeaderError::UnknownByteOrder)
        );
    }
}