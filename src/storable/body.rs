use crate::storable::value::{ClassTable, PerlValue, SeenTable, ValueRef};
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
    ClassIndexOutOfRange(usize),   // NEW
}

pub fn read_value(
    cursor: &mut Cursor<'_>,
    seen: &mut SeenTable,
    classes: &mut ClassTable,
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
                items.push(read_value(cursor, seen, classes)?);
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
                let value = read_value(cursor, seen, classes)?;
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

            let inner = read_value(cursor, seen, classes)?;

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

        0x1B => {
            // SX_WEAKREF — like SX_REF but the inner pointer is weak
            // We need to parse the inner first to get an Rc, then downgrade to Weak
            let val = PerlValue::wrap(PerlValue::Ref(PerlValue::wrap(PerlValue::Undef)));
            seen.register(val.clone());

            let inner = read_value(cursor, seen, classes)?;
            let weak = std::rc::Rc::downgrade(&inner);

            *val.borrow_mut() = PerlValue::WeakRef(weak);
            Ok(val)
        }

        0x11 => {
            let name_len = cursor.read_byte()? as usize;
            let name_bytes = cursor.read_bytes(name_len)?.to_vec();
            let class = String::from_utf8(name_bytes)
                .map_err(|_| BodyError::InvalidUtf8)?;
            classes.register(class.clone());  // NEW

            let val = PerlValue::wrap(PerlValue::Blessed(
                PerlValue::wrap(PerlValue::Undef),
                class.clone(),
            ));
            seen.register(val.clone());

            let inner = read_value(cursor, seen, classes)?;  // NEW signature

            if let PerlValue::Blessed(r, _) = &mut *val.borrow_mut() {
                *r = inner;
            }
            Ok(val)
        }

        0x12 => {
            // SX_IX_BLESS — class name given by index into class table
            let idx = cursor.read_byte()? as usize;
            let class = classes.get(idx)
                .ok_or(BodyError::ClassIndexOutOfRange(idx))?
                .to_string();

            let val = PerlValue::wrap(PerlValue::Blessed(
                PerlValue::wrap(PerlValue::Undef),
                class,
            ));
            seen.register(val.clone());

            let inner = read_value(cursor, seen, classes)?;

            if let PerlValue::Blessed(r, _) = &mut *val.borrow_mut() {
                *r = inner;
            }
            Ok(val)
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
    let mut classes: ClassTable = ClassTable::new();
    assert_eq!(
            read_value(&mut cursor(&[0x99]), &mut seen, &mut classes).unwrap_err(),
            BodyError::UnknownTag(0x99)
        );
    }

    #[test]
    fn undef_tag() {
        let mut seen = SeenTable::new();
        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&[0x05]), &mut seen, &mut classes).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::Undef));
    }

    #[test]
    fn yes_tag() {
        let mut seen = SeenTable::new();
        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&[0x0F]), &mut seen, &mut classes).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::Yes));
    }

    #[test]
    fn no_tag() {
        let mut seen = SeenTable::new();
        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&[0x10]), &mut seen, &mut classes).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::No));
    }

    #[test]
    fn immortals_are_not_indexed() {
        let mut seen = SeenTable::new();
        let mut classes: ClassTable = ClassTable::new();
        read_value(&mut cursor(&[0x05]), &mut seen, &mut classes).unwrap();
        read_value(&mut cursor(&[0x0F]), &mut seen, &mut classes).unwrap();
        assert_eq!(seen.len(), 0);
    }

    #[test]
    fn byte_becomes_integer_and_is_indexed() {
        let mut seen = SeenTable::new();
        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&[0x08, 0xAA]), &mut seen, &mut classes).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::Integer(42)));
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn scalar_bytes() {
        let mut seen = SeenTable::new();
        let mut classes: ClassTable = ClassTable::new();
        let mut input = vec![0x0A];
        input.extend_from_slice(&3i32.to_ne_bytes());
        input.extend_from_slice(b"abc");
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();
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
        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();
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
        let mut classes: ClassTable = ClassTable::new();
        assert_eq!(
            read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap_err(),
            BodyError::InvalidUtf8
        );
    }

    #[test]
    fn empty_array() {
        let mut seen = SeenTable::new();
        let mut input = vec![0x02];
        input.extend_from_slice(&0i32.to_ne_bytes());
        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();
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
        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();
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
        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();
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
        let mut classes: ClassTable = ClassTable::new();
        let input = vec![0x04, 0x08, 0xAA]; // SX_REF -> SX_BYTE 42
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();
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
        let mut classes: ClassTable = ClassTable::new();
        // An array with one element, followed by a back-ref to that element
        // But easier: an array [42, ref-to-42] using SX_OBJECT
        // Actually, easiest: [42, SX_OBJECT index=1] — index 0 is the array, index 1 is the byte 42
        let mut input = vec![0x02];
        input.extend_from_slice(&2i32.to_ne_bytes());  // 2 elements
        input.extend_from_slice(&[0x08, 0xAA]);         // element 0: SX_BYTE 42 -> index 1
        input.push(0x00);                                // element 1: SX_OBJECT
        input.extend_from_slice(&1i32.to_ne_bytes());   // pointing at index 1

        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();
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
        let mut classes: ClassTable = ClassTable::new();
        assert_eq!(
            read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap_err(),
            BodyError::ObjectIndexOutOfRange(999)
        );
    }

    #[test]
    fn weakref_downgrade() {
        let mut seen = SeenTable::new();
        let mut classes: ClassTable = ClassTable::new();
        let input = vec![0x1B, 0x08, 0xAA]; // SX_WEAKREF -> SX_BYTE 42
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();
        match &*val.borrow() {
            PerlValue::WeakRef(w) => {
                // Weak can be upgraded to Rc while the strong ref still exists in seen
                let upgraded = w.upgrade().expect("still alive in seen table");
                assert!(matches!(*upgraded.borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected WeakRef"),
        }
    }

    #[test]
    fn cycle_does_not_leak() {
        use std::rc::Rc;

        // Construct: an array whose first element weakly refers back to the array itself
        // Wire: SX_ARRAY count=1, SX_WEAKREF, SX_OBJECT index=0
        // seen[0] = array, seen[1] = weakref wrapper
        let mut input = vec![0x02];
        input.extend_from_slice(&1i32.to_ne_bytes());
        input.push(0x1B);                               // SX_WEAKREF
        input.push(0x00);                               // SX_OBJECT
        input.extend_from_slice(&0i32.to_ne_bytes());  // pointing at index 0 (the array)

        let mut seen = SeenTable::new();
        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();

        // Verify the cycle exists
        let strong_count_before = Rc::strong_count(&val);

        // Drop the seen table — this is the moment of truth
        drop(seen);

        // If the cycle were made of strong Rcs, dropping seen would leave val's
        // strong count > 1 (because the cycle keeps itself alive).
        // Because we used WeakRef, dropping seen brings val's strong count back to 1.
        assert_eq!(Rc::strong_count(&val), 1,
            "cycle should not keep itself alive (was {} before drop, {} after)",
            strong_count_before, Rc::strong_count(&val));
    }

    #[test]
    fn blessed_hashref() {
        let mut seen = SeenTable::new();
        let mut classes: ClassTable = ClassTable::new();
        // SX_BLESS "Animal" -> SX_REF -> SX_HASH count=1 -> SX_BYTE 3, key "age"
        let mut input = vec![0x11];
        input.push(6);                                    // class name length
        input.extend_from_slice(b"Animal");               // class name
        input.push(0x04);                                 // SX_REF
        input.push(0x03);                                 // SX_HASH
        input.extend_from_slice(&1i32.to_ne_bytes());    // 1 pair
        input.extend_from_slice(&[0x08, 0x83]);          // SX_BYTE 3
        input.extend_from_slice(&3i32.to_ne_bytes());    // key length
        input.extend_from_slice(b"age");                  // key

        let mut classes: ClassTable = ClassTable::new();
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes).unwrap();
        match &*val.borrow() {
            PerlValue::Blessed(inner, class) => {
                assert_eq!(class, "Animal");
                match &*inner.borrow() {
                    PerlValue::Ref(hash_ref) => match &*hash_ref.borrow() {
                        PerlValue::Hash(map) => {
                            assert_eq!(map.len(), 1);
                            let age = map.get(b"age".as_slice()).unwrap();
                            assert!(matches!(*age.borrow(), PerlValue::Integer(3)));
                        }
                        _ => panic!("expected Hash inside Ref"),
                    },
                    _ => panic!("expected Ref"),
                }
            }
            _ => panic!("expected Blessed"),
        }
    }
}