#[derive(Debug, PartialEq)]
pub enum HeaderError {
    UnsupportedMajor(u8),
    Truncated
}

#[derive(Debug,PartialEq)]
pub struct VersionByte {
    pub major: u8, // Major version number of the Storable format. Stored in the upper bits of the first byte.
    pub minor: u8, // Minor version number of the Storable format. Stored in the second byte.
    pub network_order: bool,
}

pub fn parse(input: &[u8]) -> Result<VersionByte, HeaderError> {
    if input.is_empty() {
        return Err(HeaderError::Truncated);
    }

    if input.len() < 2 {
    return Err(HeaderError::Truncated);
}
    let b = input[0];
    let major = b >> 1;
    let minor = input[1];
    let network_order = (b & 0x01) != 0;

    if major != 2 {
        Err(HeaderError::UnsupportedMajor(major))
    } else {
        // TODO : Add minor version check.
        Ok(VersionByte { major, minor, network_order })
    }
}





#[cfg(test)] // the following module is compiled only when running tests.
mod tests {
    use super::*; // imports everything from the parent module.
    
    #[test] // first test
    fn freeze_sets_native_order(){ //  test for native order
        assert_eq!(
            parse(&[0x04, 0x0B]),
            Ok (VersionByte{
                major: 2,
                minor: 11,
                network_order: false
            })
        );
    }
    
    #[test] // second test
    fn nfreeze_sets_network_order(){ // test for network order
        assert_eq!(
        parse(&[0x05, 0x0B]),
            Ok (VersionByte{
                major: 2,
                minor: 11,
                network_order: true
            })
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

}

