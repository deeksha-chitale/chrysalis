#[derive(Debug,PartialEq)]
pub struct VersionByte {
    pub major: u8,
    pub network_order: bool,
}

pub fn parse_version_byte(b: u8) -> VersionByte {
    VersionByte { 
        major: b >> 1, 
        network_order: (b & 0x01) != 0 
    }
}

#[cfg(test)] // the following module is compiled only when running tests.
mod tests {
    use super::*; // imports everything from the parent module.
    
    #[test] // first test
    fn freeze_sets_native_order(){
        assert_eq!(
            parse_version_byte(0x04),
            VersionByte{
                major: 2,
                network_order: false
            }
        );
    }
    
    #[test] // second test
    fn nfreeze_sets_network_order(){
        assert_eq!(
            parse_version_byte(0x05),
            VersionByte{
                major: 2,
                network_order: true
            }
        );
    }

}