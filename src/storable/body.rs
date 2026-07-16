#[derive(Debug, PartialEq)]
pub enum Tag {
    Byte(i8),    // Tagged by SX_BYTE = 0x02 which represents a signed byte.
}

#[derive(Debug, PartialEq)]
pub enum BodyError {
    Truncated,
    UnknownTag(u8),
}

pub fn read_tag(input: &[u8]) -> Result<Tag, BodyError> {
    if input.is_empty() {
        return Err(BodyError::Truncated);
    }
    let b = input[0];
    match b {
        0x02 => {
            if input.len() < 2 {
                return Err(BodyError::Truncated);
            } 
            let value = input[1] as i8;
            Ok(Tag::Byte(value))
        }
        _ => Err(BodyError::UnknownTag(b)),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_tag_positive() {
        assert_eq!(read_tag(&[0x02, 0x2A]), Ok(Tag::Byte(42)));
    }

    #[test]
    fn truncated_input_errors() {
        assert_eq!(read_tag(&[]), Err(BodyError::Truncated));
        assert_eq!(read_tag(&[0x02]), Err(BodyError::Truncated));
    }

    #[test]
    fn unknown_tag_errors() {
        assert_eq!(read_tag(&[0x99]), Err(BodyError::UnknownTag(0x99)));
    }
}