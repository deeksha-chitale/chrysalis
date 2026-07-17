pub struct Cursor<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    pub fn read_byte(&mut self) -> Result<u8, BodyError> {
        if self.pos >= self.input.len() {
            return Err(BodyError::Truncated);
        }
        let b = self.input[self.pos];
        self.pos += 1;
        Ok(b)
    }

    pub fn read_bytes(&mut self, n: usize) -> Result<&[u8], BodyError> {
        if self.pos + n > self.input.len() {
            return Err(BodyError::Truncated);
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    pub fn read_i32_ne(&mut self) -> Result<i32, BodyError> {
        let bytes = self.read_bytes(4)?;
        Ok(i32::from_ne_bytes(bytes.try_into().unwrap()))
    }
}

#[derive(Debug, PartialEq)]
pub enum Tag {
    Byte(i8),        // Tagged by SX_BYTE = 0x08 which represents a signed byte.
    Undefined,       // Tagged by SX_UNDEF = 0x05 which represents an undefined value.
    Integer(i32),    // Tagged by SX_INTEGER = 0x06 which represents a signed integer in native byte order (4 bytes).
    Double(f64),     // Tagged by SX_DOUBLE = 0x07 (8 bytes) in native byte order.
    NetInteger(i32), // Tagged by SX_NETINT = 0x09 which represents a signed integer (4 bytes) in network byte order.
    Scalar(Vec<u8>), // Tagged by SX_SCALAR = 0x0A, raw bytes
    Utf8Str(String), // Tagged by SX_UTF8STR = 0x17, a utf-8 string
    Array(Vec<Tag>),  // SX_ARRAY = 0x02. First, element count as native i32, then that many values
    Hash(Vec<(Vec<u8>, Tag)>),  // SX_HASH = 0x03, first we have the pair count as native i32, then value/key pairs. The key is not a tagged value.
}

#[derive(Debug, PartialEq)]
pub enum BodyError {
    Truncated,
    UnknownTag(u8),
    InvalidUtf8,
}

pub fn read_tag(cursor: &mut Cursor<'_>) -> Result<Tag, BodyError> {
    let b = cursor.read_byte()?;
    match b {
        0x08 => {
            let value = cursor.read_byte()? as i8;
            Ok(Tag::Byte(value))
        }
        0x05 => Ok(Tag::Undefined),
        0x06 => {
            let n = cursor.read_i32_ne()?;
            Ok(Tag::Integer(n))
        }
        0x07 => {
            let bytes = cursor.read_bytes(8)?;
            let n = f64::from_ne_bytes(bytes.try_into().unwrap());
            Ok(Tag::Double(n))
        }
        0x09 => {
            let bytes = cursor.read_bytes(4)?;
            let n = i32::from_be_bytes(bytes.try_into().unwrap());
            Ok(Tag::NetInteger(n))
        }
        0x0A => {
            let len = cursor.read_i32_ne()? as usize;
            let bytes = cursor.read_bytes(len)?;
            Ok(Tag::Scalar(bytes.to_vec()))
        }
        0x17 => {
            let len = cursor.read_i32_ne()? as usize;
            let bytes = cursor.read_bytes(len)?;
            match std::str::from_utf8(bytes) {
                Ok(s) => Ok(Tag::Utf8Str(s.to_string())),
                Err(_) => Err(BodyError::InvalidUtf8),
            }
        }

        0x02 => {
            let count = cursor.read_i32_ne()? as usize;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(read_tag(cursor)?);
            }
            Ok(Tag::Array(items))
        }

        0x03 => {
            let count = cursor.read_i32_ne()? as usize;
            let mut pairs = Vec::with_capacity(count);
            for _ in 0..count {
                // Value first, then key (this is how Storable writes hash pairs)
                let value = read_tag(cursor)?;
                let key_len = cursor.read_i32_ne()? as usize;
                let key_bytes = cursor.read_bytes(key_len)?.to_vec();
                pairs.push((key_bytes, value));
            }
            Ok(Tag::Hash(pairs))
        }
        _ => Err(BodyError::UnknownTag(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor<'a>(bytes: &'a [u8]) -> Cursor<'a> {
        Cursor::new(bytes)
    }

    #[test]
    fn byte_tag_positive() {
        assert_eq!(read_tag(&mut cursor(&[0x08, 0x2A])), Ok(Tag::Byte(42)));
    }

    #[test]
    fn truncated_input_errors() {
        assert_eq!(read_tag(&mut cursor(&[])), Err(BodyError::Truncated));
        assert_eq!(read_tag(&mut cursor(&[0x08])), Err(BodyError::Truncated));
    }

    #[test]
    fn unknown_tag_errors() {
        assert_eq!(read_tag(&mut cursor(&[0x99])), Err(BodyError::UnknownTag(0x99)));
    }

    #[test]
    fn undef_tag() {
        assert_eq!(read_tag(&mut cursor(&[0x05])), Ok(Tag::Undefined));
    }

    #[test]
    fn integer_tag() {
        let mut input = vec![0x06];
        input.extend_from_slice(&42i32.to_ne_bytes());
        assert_eq!(read_tag(&mut cursor(&input)), Ok(Tag::Integer(42)));
    }

    #[test]
    fn netint_tag() {
        let mut input = vec![0x09];
        input.extend_from_slice(&42i32.to_be_bytes());
        assert_eq!(read_tag(&mut cursor(&input)), Ok(Tag::NetInteger(42)));
    }

    #[test]
    fn double_tag() {
        let mut input = vec![0x07];
        input.extend_from_slice(&3.14f64.to_ne_bytes());
        assert_eq!(read_tag(&mut cursor(&input)), Ok(Tag::Double(3.14)));
    }

    #[test]
    fn scalar_tag() {
        let mut input = vec![0x0A];
        input.extend_from_slice(&3i32.to_ne_bytes());
        input.extend_from_slice(b"abc");
        assert_eq!(read_tag(&mut cursor(&input)), Ok(Tag::Scalar(b"abc".to_vec())));
    }

    #[test]
    fn utf8str_tag() {
        let mut input = vec![0x17];
        input.extend_from_slice(&5i32.to_ne_bytes());
        input.extend_from_slice(b"hello");
        assert_eq!(read_tag(&mut cursor(&input)), Ok(Tag::Utf8Str("hello".to_string())));
    }

    #[test]
    fn invalid_utf8_errors() {
        let mut input = vec![0x17];
        input.extend_from_slice(&2i32.to_ne_bytes());
        input.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8
        assert_eq!(read_tag(&mut cursor(&input)), Err(BodyError::InvalidUtf8));
    }

    #[test]
    fn empty_array() {
        let mut input = vec![0x02];
        input.extend_from_slice(&0i32.to_ne_bytes());
        assert_eq!(read_tag(&mut cursor(&input)), Ok(Tag::Array(vec![])));
    }

    #[test]
    fn array_of_bytes() {
        let mut input = vec![0x02];
        input.extend_from_slice(&2i32.to_ne_bytes());
        input.extend_from_slice(&[0x08, 0x2A]);  // SX_BYTE 42
        input.extend_from_slice(&[0x08, 0x05]);  // SX_BYTE 5
        assert_eq!(
            read_tag(&mut cursor(&input)),
            Ok(Tag::Array(vec![Tag::Byte(42), Tag::Byte(5)]))
        );
    }

    #[test]
    fn simple_hash() {
        // hash with 1 pair: { "answer" => 42 }
        // Value first (SX_BYTE 42), then key length + "answer"
        let mut input = vec![0x03];
        input.extend_from_slice(&1i32.to_ne_bytes());
        input.extend_from_slice(&[0x08, 0x2A]);  // SX_BYTE 42
        input.extend_from_slice(&6i32.to_ne_bytes());
        input.extend_from_slice(b"answer");
        assert_eq!(
            read_tag(&mut cursor(&input)),
            Ok(Tag::Hash(vec![(b"answer".to_vec(), Tag::Byte(42))]))
        );
    }
}