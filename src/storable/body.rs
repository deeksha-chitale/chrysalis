#[derive(Debug, PartialEq)]
pub enum Tag {
    Byte(i8),    // Tagged by SX_BYTE = 0x02 which represents a signed byte.
    Undefined,   // Tagged by SX_UNDEF = 0x05 which represents an undefined value.
    Integer(i32), // Tagged by SX_INTEGER = 0x06 which represents a signed integer in native byte order (4 bytes).
    Double(f64), // Tagged by SX_DOUBLE = 0x07 (8 bytes) in native byte order.
    NetInteger(i32), // Tagged by SX_NETINT = 0x09 which represents a signed integer (4 bytes) in network byte order.
    Scalar(Vec<u8>),  // Tagged by SX_SCALAR = 0x0A — raw bytes
    Utf8Str(String),  // Tagged by SX_UTF8STR = 0x17 — utf-8 string
}

#[derive(Debug, PartialEq)]
pub enum BodyError {
    Truncated,
    UnknownTag(u8),
    InvalidUtf8,
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

        0x05 => Ok(Tag::Undefined),

        0x06 => {
        if input.len() < 5 { return Err(BodyError::Truncated); }
        let n = i32::from_ne_bytes(input[1..5].try_into().unwrap());
        Ok(Tag::Integer(n))
        }

        0x07 => {
            if input.len() < 9 { return Err(BodyError::Truncated); }
            let n = f64::from_ne_bytes(input[1..9].try_into().unwrap());
            Ok(Tag::Double(n))
        }

        0x09 => {
            if input.len() < 5 { return Err(BodyError::Truncated); }
            let n = i32::from_be_bytes(input[1..5].try_into().unwrap());
            Ok(Tag::NetInteger(n))
        }

        0x0A => {
            if input.len() < 5 { return Err(BodyError::Truncated); }
            let len = i32::from_ne_bytes(input[1..5].try_into().unwrap()) as usize;
            if input.len() < 5 + len { return Err(BodyError::Truncated); }
            Ok(Tag::Scalar(input[5..5 + len].to_vec()))
        }
        0x17 => {
            if input.len() < 5 { return Err(BodyError::Truncated); }
            let len = i32::from_ne_bytes(input[1..5].try_into().unwrap()) as usize;
            if input.len() < 5 + len { return Err(BodyError::Truncated); }
            let bytes = &input[5..5 + len];
            match std::str::from_utf8(bytes) {
                Ok(s) => Ok(Tag::Utf8Str(s.to_string())),
                Err(_) => Err(BodyError::InvalidUtf8),
            }
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

    #[test]
    fn undef_tag() {
        assert_eq!(read_tag(&[0x05]), Ok(Tag::Undefined));
    }

    #[test]
    fn integer_tag() {
        let mut input = vec![0x06];
        input.extend_from_slice(&42i32.to_ne_bytes());
        assert_eq!(read_tag(&input), Ok(Tag::Integer(42)));
    }

    #[test]
    fn netint_tag() {
        let mut input = vec![0x09];
        input.extend_from_slice(&42i32.to_be_bytes());
        assert_eq!(read_tag(&input), Ok(Tag::NetInteger(42)));
    }

    #[test]
    fn double_tag() {
        let mut input = vec![0x07];
        input.extend_from_slice(&3.14f64.to_ne_bytes());
        assert_eq!(read_tag(&input), Ok(Tag::Double(3.14)));
    }

    #[test]
fn scalar_tag() {
    let mut input = vec![0x0A];
    input.extend_from_slice(&3i32.to_ne_bytes());
    input.extend_from_slice(b"abc");
    assert_eq!(read_tag(&input), Ok(Tag::Scalar(b"abc".to_vec())));
}

    #[test]
    fn utf8str_tag() {
        let mut input = vec![0x17];
        input.extend_from_slice(&5i32.to_ne_bytes());
        input.extend_from_slice(b"hello");
        assert_eq!(read_tag(&input), Ok(Tag::Utf8Str("hello".to_string())));
    }

    #[test]
    fn invalid_utf8_errors() {
        let mut input = vec![0x17];
        input.extend_from_slice(&2i32.to_ne_bytes());
        input.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8
        assert_eq!(read_tag(&input), Err(BodyError::InvalidUtf8));
    }
}