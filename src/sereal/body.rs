use crate::sereal::{Cursor, SerealError};
use crate::sereal::value::SerealSeenTable;
use crate::storable::value::{PerlValue, ValueRef};

fn perl_value_to_key_bytes(val: &ValueRef) -> Vec<u8> {
    match &*val.borrow() {
        PerlValue::Bytes(b) => b.clone(),
        PerlValue::String(s) => s.as_bytes().to_vec(),
        other => panic!("unexpected hash key type: {:?}", other), // TODO: proper error
    }
}

fn perl_value_to_string(val: &ValueRef) -> String {
    match &*val.borrow() {
        PerlValue::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        PerlValue::String(s) => s.clone(),
        other => panic!("expected string-like value, got {:?}", other), // TODO: proper error
    }
}

pub fn read_value(cursor: &mut Cursor, seen: &mut SerealSeenTable) -> Result<ValueRef, SerealError> {
    let start_offset = cursor.pos();
    let tag = cursor.read_byte()?;
    let track = tag & 0x80 != 0;   // high bit = track flag
    let tag = tag & 0x7F;          // mask it off per spec

    let value = match tag {
        // POS_0..POS_15 = 0x00..0x0F — small positive int, value = tag itself
        0x00..=0x0F => PerlValue::wrap(PerlValue::Integer(tag as i64)),

        // NEG_16..NEG_1 = 0x10..0x1F — small negative int, value = tag - 32
        0x10..=0x1F => PerlValue::wrap(PerlValue::Integer(tag as i64 - 32)),

        0x25 => PerlValue::wrap(PerlValue::Undef),          // UNDEF
        0x39 => PerlValue::wrap(PerlValue::Undef),          // CANONICAL_UNDEF
        0x3A => PerlValue::wrap(PerlValue::No),             // FALSE
        0x3B => PerlValue::wrap(PerlValue::Yes),            // TRUE

        0x20 => {
            // VARINT
            let n = cursor.read_varint()?;
            PerlValue::wrap(PerlValue::Integer(n as i64))
        }

        0x21 => {
            // ZIGZAG
            let z = cursor.read_varint()?;
            let n = ((z >> 1) as i64) ^ -((z & 1) as i64);
            PerlValue::wrap(PerlValue::Integer(n))
        }

        0x26 => {
            // BINARY
            let len = cursor.read_varint()? as usize;
            let bytes = cursor.read_bytes(len)?.to_vec();
            PerlValue::wrap(PerlValue::Bytes(bytes))
        }

        0x27 => {
            // STR_UTF8
            let len = cursor.read_varint()? as usize;
            let bytes = cursor.read_bytes(len)?;
            let s = std::str::from_utf8(bytes).map_err(|_| SerealError::InvalidUtf8)?.to_string();
            PerlValue::wrap(PerlValue::String(s))
        }

        0x60..=0x7F => {
            // SHORT_BINARY_0..31 — length in low 5 bits of tag
            let len = (tag & 0x1F) as usize;
            let bytes = cursor.read_bytes(len)?.to_vec();
            PerlValue::wrap(PerlValue::Bytes(bytes))
        }

        0x2B => {
            // ARRAY
            let count = cursor.read_varint()? as usize;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(read_value(cursor, seen)?);
            }
            PerlValue::wrap(PerlValue::Array(items))
        }
        0x40..=0x4F => {
            // ARRAYREF_0..15 — count in low 4 bits
            let count = (tag & 0x0F) as usize;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(read_value(cursor, seen)?);
            }
            let arr = PerlValue::wrap(PerlValue::Array(items));
            PerlValue::wrap(PerlValue::Ref(arr))
        }
        0x2A => {
            // HASH — key first, then value, per pair
            let count = cursor.read_varint()? as usize;
            let mut map = std::collections::HashMap::with_capacity(count);
            for _ in 0..count {
                let key_val = read_value(cursor, seen)?;
                let key_bytes = perl_value_to_key_bytes(&key_val);
                let value = read_value(cursor, seen)?;
                map.insert(key_bytes, value);
            }
            PerlValue::wrap(PerlValue::Hash(map))
        }
        0x50..=0x5F => {
            // HASHREF_0..15 — count in low 4 bits
            let count = (tag & 0x0F) as usize;
            let mut map = std::collections::HashMap::with_capacity(count);
            for _ in 0..count {
                let key_val = read_value(cursor, seen)?;
                let key_bytes = perl_value_to_key_bytes(&key_val);
                let value = read_value(cursor, seen)?;
                map.insert(key_bytes, value);
            }
            let hash = PerlValue::wrap(PerlValue::Hash(map));
            PerlValue::wrap(PerlValue::Ref(hash))
        }

        0x28 => {
            // REFN — ref wraps the next item
            let inner = read_value(cursor, seen)?;
            PerlValue::wrap(PerlValue::Ref(inner))
        }
        0x29 => {
            // REFP — ref to a previously tracked item, by offset
            let offset = cursor.read_varint()? as usize;
            let referenced = seen.get(offset)
                .ok_or(SerealError::OffsetNotFound(offset))?;
            PerlValue::wrap(PerlValue::Ref(referenced))
        }
        0x2E => {
            // ALIAS — reuse a previously tracked item directly (not wrapped in a new Ref)
            let offset = cursor.read_varint()? as usize;
            seen.get(offset).ok_or(SerealError::OffsetNotFound(offset))?
        }
        0x30 => {
            // WEAKEN — the following item is a ref; convert it to a weak ref
            let inner = read_value(cursor, seen)?;
            match &*inner.borrow() {
                PerlValue::Ref(strong) => {
                    let weak = std::rc::Rc::downgrade(strong);
                    PerlValue::wrap(PerlValue::WeakRef(weak))
                }
                other => panic!("WEAKEN expected a Ref, got {:?}", other), // TODO: proper error
            }
        }
        0x2F => {
            // COPY — reread the tag at the given offset as if inserted here
            let offset = cursor.read_varint()? as usize;
            let saved_pos = cursor.pos();
            cursor.seek(offset);
            let result = read_value(cursor, seen)?;
            cursor.seek(saved_pos);
            result
        }

        0x2C => {
            // OBJECT — class name (string tag), then data
            let class_val = read_value(cursor, seen)?;
            let class = perl_value_to_string(&class_val);
            let inner = read_value(cursor, seen)?;
            PerlValue::wrap(PerlValue::Blessed(inner, class))
        }
        0x2D => {
            // OBJECTV — class name by offset, then data
            let offset = cursor.read_varint()? as usize;
            let class_val = seen.get(offset).ok_or(SerealError::OffsetNotFound(offset))?;
            let class = perl_value_to_string(&class_val);
            let inner = read_value(cursor, seen)?;
            PerlValue::wrap(PerlValue::Blessed(inner, class))
        }
        0x32 => {
            // OBJECT_FREEZE — same shape as OBJECT, data went through FREEZE/THAW
            let class_val = read_value(cursor, seen)?;
            let class = perl_value_to_string(&class_val);
            let inner = read_value(cursor, seen)?;
            PerlValue::wrap(PerlValue::Blessed(inner, class))
        }
        0x33 => {
            // OBJECTV_FREEZE — same as OBJECTV, data went through FREEZE/THAW
            let offset = cursor.read_varint()? as usize;
            let class_val = seen.get(offset).ok_or(SerealError::OffsetNotFound(offset))?;
            let class = perl_value_to_string(&class_val);
            let inner = read_value(cursor, seen)?;
            PerlValue::wrap(PerlValue::Blessed(inner, class))
        }
        0x31 => {
            // REGEXP — pattern string, then modifiers string
            let pattern_val = read_value(cursor, seen)?;
            let flags_val = read_value(cursor, seen)?;
            let flags = perl_value_to_string(&flags_val);
            PerlValue::wrap(PerlValue::Regexp { pattern: pattern_val, flags })
        }

        0x22 => {
            // FLOAT — 4-byte IEEE float
            let bytes = cursor.read_bytes(4)?;
            let f = f32::from_le_bytes(bytes.try_into().unwrap());
            PerlValue::wrap(PerlValue::Double(f as f64))
        }
        0x23 => {
            // DOUBLE — 8-byte IEEE double
            let bytes = cursor.read_bytes(8)?;
            let f = f64::from_le_bytes(bytes.try_into().unwrap());
            PerlValue::wrap(PerlValue::Double(f))
        }
        0x24 => {
            // LONG_DOUBLE — platform-dependent extended precision (usually 10 or 16 bytes)
            // Not portably representable in Rust's f64; read and discard the raw bytes,
            // store as Bytes so no data is silently lost.
            // Most encoders don't emit this in practice.
            return Err(SerealError::UnsupportedTag("LONG_DOUBLE"));
        }
        0x3F => {
            // PAD — no-op, skip to next byte and retry
            return read_value(cursor, seen);
        }
        0x3E => {
            return Err(SerealError::UnsupportedTag("EXTEND"));
        }
        0x3C => {
            return Err(SerealError::UnsupportedTag("MANY"));
        }
        0x34..=0x38 => {
            return Err(SerealError::UnsupportedTag("RESERVED"));
        }

        _ => return Err(SerealError::UnknownTag(tag)),
    };

    if track {
        seen.register(start_offset, value.clone());
    }
    Ok(value)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pos_small_int() {
        let mut c = Cursor::new(&[0x05]);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(5)));
    }

    #[test]
    fn neg_small_int() {
        let mut c = Cursor::new(&[0x1F]); // NEG_1 = -1
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(-1)));
    }

    #[test]
    fn undef_tag() {
        let mut c = Cursor::new(&[0x25]);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Undef));
    }

    #[test]
    fn true_false_tags() {
        let mut seen = SerealSeenTable::new();
        let mut c1 = Cursor::new(&[0x3B]);
        assert!(matches!(*read_value(&mut c1, &mut seen).unwrap().borrow(), PerlValue::Yes));
        let mut c2 = Cursor::new(&[0x3A]);
        assert!(matches!(*read_value(&mut c2, &mut seen).unwrap().borrow(), PerlValue::No));
    }

    #[test]
    fn varint_tag() {
        let mut c = Cursor::new(&[0x20, 0x2A]); // VARINT 42
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(42)));
    }

    #[test]
    fn zigzag_negative() {
        let mut c = Cursor::new(&[0x21, 0x01]); // zigzag 1 -> -1
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(-1)));
    }

    #[test]
    fn zigzag_positive() {
        let mut c = Cursor::new(&[0x21, 0x02]); // zigzag 2 -> 1
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(1)));
    }

    #[test]
    fn short_binary_tag() {
        let mut input = vec![0x66]; // SHORT_BINARY_6
        input.extend_from_slice(b"answer");
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Bytes(b) => assert_eq!(b, b"answer"),
            other => panic!("expected Bytes: {:?}", other),
        }
    }

    #[test]
    fn binary_tag() {
        let mut input = vec![0x26, 3];
        input.extend_from_slice(b"xyz");
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Bytes(b) => assert_eq!(b, b"xyz"),
            other => panic!("expected Bytes: {:?}", other),
        }
    }

    #[test]
    fn str_utf8_tag() {
        let mut input = vec![0x27, 5];
        input.extend_from_slice(b"hello");
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::String(s) => assert_eq!(s, "hello"),
            other => panic!("expected String: {:?}", other),
        }
    }

    #[test]
    fn arrayref_compact() {
        // ARRAYREF_2: two SHORT_BINARY-tagged bytes
        let mut input = vec![0x42]; // ARRAYREF_2
        input.push(0x03); input.extend_from_slice(b"abc");
        input.push(0x03); input.extend_from_slice(b"xyz");
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Ref(inner) => match &*inner.borrow() {
                PerlValue::Array(items) => assert_eq!(items.len(), 2),
                other => panic!("expected Array: {:?}", other),
            },
            other => panic!("expected Ref: {:?}", other),
        }
    }

    #[test]
    fn hashref_compact() {
        // HASHREF_1: key "id" (SHORT_BINARY_2), value 42 (POS)
        let mut input = vec![0x51]; // HASHREF_1
        input.push(0x62); input.extend_from_slice(b"id"); // SHORT_BINARY_2 "id"
        input.push(0x2A); // POS_42... wait POS max is 15
        let mut c = Cursor::new(&[0x51, 0x62, b'i', b'd', 0x0A]); // value = POS_10 = 10
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Ref(inner) => match &*inner.borrow() {
                PerlValue::Hash(map) => {
                    assert_eq!(map.len(), 1);
                    let val = map.get(b"id".as_slice()).unwrap();
                    assert!(matches!(*val.borrow(), PerlValue::Integer(10)));
                }
                other => panic!("expected Hash: {:?}", other),
            },
            other => panic!("expected Ref: {:?}", other),
        }
    }

    #[test]
    fn refn_wraps_next_item() {
        let mut c = Cursor::new(&[0x28, 0x0A]); // REFN -> POS_10
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Ref(inner) => assert!(matches!(*inner.borrow(), PerlValue::Integer(10))),
            other => panic!("expected Ref: {:?}", other),
        }
    }

    #[test]
    fn refp_back_reference() {
        // Track a value at offset 0, then REFP back to it at offset 2
        let mut seen = SerealSeenTable::new();
        // Manually populate seen as if offset 0 held an Integer(7)
        let seven = PerlValue::wrap(PerlValue::Integer(7));
        seen.register(0, seven);
        let mut c = Cursor::new(&[0x29, 0x00]); // REFP offset=0
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Ref(inner) => assert!(matches!(*inner.borrow(), PerlValue::Integer(7))),
            other => panic!("expected Ref: {:?}", other),
        }
    }

    #[test]
    fn alias_reuses_directly() {
        let mut seen = SerealSeenTable::new();
        let seven = PerlValue::wrap(PerlValue::Integer(7));
        seen.register(0, seven.clone());
        let mut c = Cursor::new(&[0x2E, 0x00]); // ALIAS offset=0
        let v = read_value(&mut c, &mut seen).unwrap();
        assert!(std::rc::Rc::ptr_eq(&v, &seven));
    }

    #[test]
    fn offset_not_found_errors() {
        let mut seen = SerealSeenTable::new();
        let mut c = Cursor::new(&[0x29, 0x63]); // REFP offset=99, nothing registered
        assert_eq!(read_value(&mut c, &mut seen).unwrap_err(), SerealError::OffsetNotFound(99));
    }

    #[test]
    fn object_tag() {
        // OBJECT: class "Foo" (SHORT_BINARY_3), data = POS_5
        let mut input = vec![0x2C];
        input.push(0x63); input.extend_from_slice(b"Foo"); // SHORT_BINARY_3 "Foo"
        input.push(0x05); // POS_5
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Blessed(inner, class) => {
                assert_eq!(class, "Foo");
                assert!(matches!(*inner.borrow(), PerlValue::Integer(5)));
            }
            other => panic!("expected Blessed: {:?}", other),
        }
    }

    #[test]
    fn regexp_tag() {
        // REGEXP: pattern "abc" (SHORT_BINARY_3), flags "i" (SHORT_BINARY_1)
        let mut input = vec![0x31];
        input.push(0x63); input.extend_from_slice(b"abc");
        input.push(0x61); input.extend_from_slice(b"i");
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Regexp { pattern, flags } => {
                assert_eq!(flags, "i");
                match &*pattern.borrow() {
                    PerlValue::Bytes(b) => assert_eq!(b, b"abc"),
                    other => panic!("expected Bytes pattern: {:?}", other),
                }
            }
            other => panic!("expected Regexp: {:?}", other),
        }
    }

    #[test]
    fn float_tag() {
        let mut input = vec![0x22];
        input.extend_from_slice(&3.14f32.to_le_bytes());
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Double(f) => assert!((*f - 3.14f32 as f64).abs() < 1e-6),
            other => panic!("expected Double: {:?}", other),
        }
    }

    #[test]
    fn double_tag() {
        let mut input = vec![0x23];
        input.extend_from_slice(&2.718281828f64.to_le_bytes());
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        match &*v.borrow() {
            PerlValue::Double(f) => assert_eq!(*f, 2.718281828),
            other => panic!("expected Double: {:?}", other),
        }
    }

    #[test]
    fn pad_is_skipped() {
        let mut c = Cursor::new(&[0x3F, 0x05]); // PAD, then POS_5
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(5)));
    }


}