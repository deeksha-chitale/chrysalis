use crate::storable::value::{PerlValue, SeenTable, ValueRef};

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
pub enum BodyError {
    Truncated,
    UnknownTag(u8),
    InvalidUtf8,
    ObjectIndexOutOfRange(usize),
}

pub fn read_value(
    cursor: &mut Cursor<'_>,
    seen: &mut SeenTable,
) -> Result<ValueRef, BodyError> {
    let b = cursor.read_byte()?;
    match b {
        0x05 => Ok(PerlValue::wrap(PerlValue::Undef)),   // SX_UNDEF
        0x0E => Ok(PerlValue::wrap(PerlValue::Undef)),   // SX_SV_UNDEF (same meaning for our purposes)
        0x0F => Ok(PerlValue::wrap(PerlValue::Yes)),     // SX_SV_YES
        0x10 => Ok(PerlValue::wrap(PerlValue::No)),      // SX_SV_NO

        0x08 => {
            // SX_BYTE — signed byte with +128 bias
            let raw = cursor.read_byte()?;
            let val = PerlValue::wrap(PerlValue::Integer((raw as i16 - 128) as i64));
            seen.register(val.clone());
            Ok(val)
        }
        0x06 => {
            // SX_INTEGER — 4-byte native
            let n = cursor.read_i32_ne()?;
            let val = PerlValue::wrap(PerlValue::Integer(n as i64));
            seen.register(val.clone());
            Ok(val)
        }
        0x09 => {
            // SX_NETINT — 4-byte big-endian
            let bytes = cursor.read_bytes(4)?;
            let n = i32::from_be_bytes(bytes.try_into().unwrap());
            let val = PerlValue::wrap(PerlValue::Integer(n as i64));
            seen.register(val.clone());
            Ok(val)
        }
        0x07 => {
            // SX_DOUBLE — 8-byte native float
            let bytes = cursor.read_bytes(8)?;
            let n = f64::from_ne_bytes(bytes.try_into().unwrap());
            let val = PerlValue::wrap(PerlValue::Double(n));
            seen.register(val.clone());
            Ok(val)
        }
        0x0A => {
            // SX_SCALAR — byte string, length-prefixed
            let len = cursor.read_i32_ne()? as usize;
            let bytes = cursor.read_bytes(len)?.to_vec();
            let val = PerlValue::wrap(PerlValue::Bytes(bytes));
            seen.register(val.clone());
            Ok(val)
        }
        0x17 => {
            // SX_UTF8STR — utf-8 string, length-prefixed
            let len = cursor.read_i32_ne()? as usize;
            let bytes = cursor.read_bytes(len)?;
            let s = std::str::from_utf8(bytes)
                .map_err(|_| BodyError::InvalidUtf8)?
                .to_string();
            let val = PerlValue::wrap(PerlValue::String(s));
            seen.register(val.clone());
            Ok(val)
        }

        0x02 => {
            // SX_ARRAY — register empty array first, then populate
            let val = PerlValue::wrap(PerlValue::Array(Vec::new()));
            seen.register(val.clone());

            let count = cursor.read_i32_ne()? as usize;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(read_value(cursor, seen)?);
            }

            // Now replace the empty array with the populated one
            if let PerlValue::Array(vec) = &mut *val.borrow_mut() {
                *vec = items;
            }
            Ok(val)
        }
        0x03 => {
            // SX_HASH — register empty hash first, then populate
            let val = PerlValue::wrap(PerlValue::Hash(std::collections::HashMap::new()));
            seen.register(val.clone());

            let count = cursor.read_i32_ne()? as usize;
            let mut pairs = std::collections::HashMap::with_capacity(count);
            for _ in 0..count {
                // Value first, then key
                let value = read_value(cursor, seen)?;
                let key_len = cursor.read_i32_ne()? as usize;
                let key = cursor.read_bytes(key_len)?.to_vec();
                pairs.insert(key, value);
            }

            if let PerlValue::Hash(map) = &mut *val.borrow_mut() {
                *map = pairs;
            }
            Ok(val)
        }

        0x04 => {
            // SX_REF — register empty ref first, then populate its inner
            let val = PerlValue::wrap(PerlValue::Ref(PerlValue::wrap(PerlValue::Undef)));
            seen.register(val.clone());

            let inner = read_value(cursor, seen)?;

            if let PerlValue::Ref(r) = &mut *val.borrow_mut() {
                *r = inner;
            }
            Ok(val)
        }

        0x00 => {
            // SX_OBJECT — back-reference to a previously-seen value
            let index = cursor.read_i32_ne()? as usize;
            match seen.get(index) {
                Some(val) => Ok(val),  // returns a clone of the Rc — same underlying value
                None => Err(BodyError::ObjectIndexOutOfRange(index)),
            }
        }
        _ => Err(BodyError::UnknownTag(b)),
    }

    
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(bytes: &[u8]) -> Cursor<'_> {
        Cursor::new(bytes)
    }

    #[test]
    fn unknown_tag_errors() {
        let mut seen = SeenTable::new();
        assert_eq!(
            read_value(&mut cursor(&[0x99]), &mut seen).unwrap_err(),
            BodyError::UnknownTag(0x99)
        );
    }

    #[test]
    fn undef_tag() {
        let mut seen = SeenTable::new();
        let val = read_value(&mut cursor(&[0x05]), &mut seen).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::Undef));
    }

    #[test]
    fn yes_tag() {
        let mut seen = SeenTable::new();
        let val = read_value(&mut cursor(&[0x0F]), &mut seen).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::Yes));
    }

    #[test]
    fn no_tag() {
        let mut seen = SeenTable::new();
        let val = read_value(&mut cursor(&[0x10]), &mut seen).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::No));
    }

    #[test]
    fn immortals_are_not_indexed() {
        let mut seen = SeenTable::new();
        read_value(&mut cursor(&[0x05]), &mut seen).unwrap();
        read_value(&mut cursor(&[0x0F]), &mut seen).unwrap();
        assert_eq!(seen.len(), 0);
    }

    #[test]
    fn byte_becomes_integer_and_is_indexed() {
        let mut seen = SeenTable::new();
        let val = read_value(&mut cursor(&[0x08, 0xAA]), &mut seen).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::Integer(42)));
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn scalar_bytes() {
        let mut seen = SeenTable::new();
        let mut input = vec![0x0A];
        input.extend_from_slice(&3i32.to_ne_bytes());
        input.extend_from_slice(b"abc");
        let val = read_value(&mut cursor(&input), &mut seen).unwrap();
        match &*val.borrow() {
            PerlValue::Bytes(b) => assert_eq!(b, b"abc"),
            _ => panic!("expected Bytes"),
        }
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn utf8_string() {
        let mut seen = SeenTable::new();
        let mut input = vec![0x17];
        input.extend_from_slice(&5i32.to_ne_bytes());
        input.extend_from_slice(b"hello");
        let val = read_value(&mut cursor(&input), &mut seen).unwrap();
        match &*val.borrow() {
            PerlValue::String(s) => assert_eq!(s, "hello"),
            _ => panic!("expected String"),
        }
    }

    #[test]
    fn invalid_utf8_errors() {
        let mut seen = SeenTable::new();
        let mut input = vec![0x17];
        input.extend_from_slice(&2i32.to_ne_bytes());
        input.extend_from_slice(&[0xFF, 0xFE]);
        assert_eq!(
            read_value(&mut cursor(&input), &mut seen).unwrap_err(),
            BodyError::InvalidUtf8
        );
    }

    #[test]
fn empty_array() {
    let mut seen = SeenTable::new();
    let mut input = vec![0x02];
    input.extend_from_slice(&0i32.to_ne_bytes());
    let val = read_value(&mut cursor(&input), &mut seen).unwrap();
    match &*val.borrow() {
        PerlValue::Array(items) => assert!(items.is_empty()),
        _ => panic!("expected Array"),
    }
    assert_eq!(seen.len(), 1); // the array itself is indexed
}

    #[test]
    fn array_of_bytes() {
        let mut seen = SeenTable::new();
        let mut input = vec![0x02];
        input.extend_from_slice(&2i32.to_ne_bytes());
        input.extend_from_slice(&[0x08, 0xAA]);  // 42
        input.extend_from_slice(&[0x08, 0x85]);  // 5
        let val = read_value(&mut cursor(&input), &mut seen).unwrap();
        match &*val.borrow() {
            PerlValue::Array(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(*items[0].borrow(), PerlValue::Integer(42)));
                assert!(matches!(*items[1].borrow(), PerlValue::Integer(5)));
            }
            _ => panic!("expected Array"),
        }
        // Array + 2 bytes = 3 indexed values
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn simple_hash() {
        let mut seen = SeenTable::new();
        let mut input = vec![0x03];
        input.extend_from_slice(&1i32.to_ne_bytes());
        input.extend_from_slice(&[0x08, 0xAA]);  // value: 42
        input.extend_from_slice(&6i32.to_ne_bytes());
        input.extend_from_slice(b"answer");       // key
        let val = read_value(&mut cursor(&input), &mut seen).unwrap();
        match &*val.borrow() {
            PerlValue::Hash(map) => {
                assert_eq!(map.len(), 1);
                let v = map.get(b"answer".as_slice()).unwrap();
                assert!(matches!(*v.borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected Hash"),
        }
    }

    #[test]
    fn ref_to_integer() {
        let mut seen = SeenTable::new();
        let input = vec![0x04, 0x08, 0xAA]; // SX_REF -> SX_BYTE 42
        let val = read_value(&mut cursor(&input), &mut seen).unwrap();
        match &*val.borrow() {
            PerlValue::Ref(inner) => {
                assert!(matches!(*inner.borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected Ref"),
        }
        // Ref + Integer = 2 indexed values
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn back_reference() {
        let mut seen = SeenTable::new();
        // An array with one element, followed by a back-ref to that element
        // But easier: an array [42, ref-to-42] using SX_OBJECT
        // Actually, easiest: [42, SX_OBJECT index=1] — index 0 is the array, index 1 is the byte 42
        let mut input = vec![0x02];
        input.extend_from_slice(&2i32.to_ne_bytes());  // 2 elements
        input.extend_from_slice(&[0x08, 0xAA]);         // element 0: SX_BYTE 42 -> index 1
        input.push(0x00);                                // element 1: SX_OBJECT
        input.extend_from_slice(&1i32.to_ne_bytes());   // pointing at index 1

        let val = read_value(&mut cursor(&input), &mut seen).unwrap();
        match &*val.borrow() {
            PerlValue::Array(items) => {
                assert_eq!(items.len(), 2);
                // Both elements should be the same Rc — shared identity
                assert!(std::rc::Rc::ptr_eq(&items[0], &items[1]));
                assert!(matches!(*items[0].borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected Array"),
        }
    }

    #[test]
    fn back_reference_out_of_range() {
        let mut seen = SeenTable::new();
        let mut input = vec![0x00];
        input.extend_from_slice(&999i32.to_ne_bytes());
        assert_eq!(
            read_value(&mut cursor(&input), &mut seen).unwrap_err(),
            BodyError::ObjectIndexOutOfRange(999)
        );
    }
}