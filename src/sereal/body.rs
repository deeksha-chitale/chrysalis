use crate::sereal::{Cursor, SerealError};
use crate::sereal::value::SerealSeenTable;
use crate::storable::value::{PerlValue, ValueRef};


const MAX_DEPTH: usize = 100;

fn perl_value_to_string(val: &ValueRef) -> Result<String, SerealError> {
    match &*val.borrow() {
        PerlValue::Bytes(b) => Ok(String::from_utf8_lossy(b).into_owned()),
        PerlValue::String(s) => Ok(s.clone()),
        _ => Err(SerealError::UnexpectedValueType {
            expected: "string-like",
            context: "class name",
        }),
    }
}

fn perl_value_to_key_bytes(val: &ValueRef) -> Result<Vec<u8>, SerealError> {
    match &*val.borrow() {
        PerlValue::Bytes(b) => Ok(b.clone()),
        PerlValue::String(s) => Ok(s.as_bytes().to_vec()),
        _ => Err(SerealError::UnexpectedValueType {
            expected: "string-like",
            context: "hash key",
        }),
    }
}

pub fn read_value(cursor: &mut Cursor, seen: &mut SerealSeenTable, depth: usize) -> Result<ValueRef, SerealError> {
    if depth > MAX_DEPTH {
        return Err(SerealError::MaxDepthExceeded);
    }
    let tag = cursor.read_byte()?;          // consume the tag byte first
    let content_offset = cursor.pos();      // NOW capture position — this is "tag position + 1"
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
            let count = cursor.read_varint()? as usize;
            if count > cursor.remaining_len() {
                return Err(SerealError::CountExceedsInput { count, remaining: cursor.remaining_len() });
            }
            let placeholder = PerlValue::wrap(PerlValue::Array(Vec::new()));
            if track {
                seen.register(content_offset, placeholder.clone());
            }
            let mut items = Vec::with_capacity(count.min(cursor.remaining_len()));
            for _ in 0..count {
                items.push(read_value(cursor, seen, depth + 1)?);
            }
            if let PerlValue::Array(v) = &mut *placeholder.borrow_mut() {
                *v = items;
            }
            return Ok(placeholder);
        }

        0x40..=0x4F => {
            let count = (tag & 0x0F) as usize;
            let placeholder_arr = PerlValue::wrap(PerlValue::Array(Vec::new()));
            if track {
                seen.register(content_offset, placeholder_arr.clone());   // register the INNER array
            }
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(read_value(cursor, seen, depth + 1)?);
            }
            if let PerlValue::Array(v) = &mut *placeholder_arr.borrow_mut() {
                *v = items;
            }
            let outer_ref = PerlValue::wrap(PerlValue::Ref(placeholder_arr));
            return Ok(outer_ref);
        }

        0x2A => {
            let count = cursor.read_varint()? as usize;
            if count.checked_mul(2).map_or(true, |min| min > cursor.remaining_len()) {
                return Err(SerealError::CountExceedsInput { count, remaining: cursor.remaining_len() });
            }
            let placeholder = PerlValue::wrap(PerlValue::Hash(std::collections::HashMap::new()));
            if track {
                seen.register(content_offset, placeholder.clone());
            }
            let mut map = std::collections::HashMap::with_capacity(count.min(cursor.remaining_len() / 2));
            for _ in 0..count {
                let key_val = read_value(cursor, seen, depth + 1)?;
                let key_bytes = perl_value_to_key_bytes(&key_val)?;
                let value = read_value(cursor, seen, depth + 1)?;
                map.insert(key_bytes, value);
            }
            if let PerlValue::Hash(m) = &mut *placeholder.borrow_mut() {
                *m = map;
            }
            return Ok(placeholder);
        }

        0x50..=0x5F => {
            let count = (tag & 0x0F) as usize;
            let placeholder_hash = PerlValue::wrap(PerlValue::Hash(std::collections::HashMap::new()));
            if track {
                seen.register(content_offset, placeholder_hash.clone());   // register the INNER hash, not a Ref wrapper
            }
            let mut map = std::collections::HashMap::with_capacity(count);
            for _ in 0..count {
                let key_val = read_value(cursor, seen, depth + 1)?;
                let key_bytes = perl_value_to_key_bytes(&key_val)?;
                let value = read_value(cursor, seen, depth + 1)?;
                map.insert(key_bytes, value);
            }
            if let PerlValue::Hash(m) = &mut *placeholder_hash.borrow_mut() {
                *m = map;
            }
            let outer_ref = PerlValue::wrap(PerlValue::Ref(placeholder_hash));
            return Ok(outer_ref);
        }

        0x28 => {
            // REFN — ref wraps the next item
            let inner = read_value(cursor, seen, depth + 1)?;
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
            let inner = read_value(cursor, seen, depth + 1)?;
            let borrowed = inner.borrow();
            match &*borrowed {
                PerlValue::Ref(strong) => {
                    let weak = std::rc::Rc::downgrade(strong);
                    drop(borrowed);
                    PerlValue::wrap(PerlValue::WeakRef(weak))
                }
                _ => return Err(SerealError::UnexpectedValueType {
                    expected: "Ref",
                    context: "WEAKEN target",
                }),
            }
        }
        0x2F => {
            let offset = cursor.read_varint()? as usize;
            let seek_pos = offset.checked_sub(1)
                .ok_or(SerealError::InvalidCopyOffset(offset))?;
            let saved_pos = cursor.pos();
            cursor.seek(seek_pos);
            let result = read_value(cursor, seen, depth + 1)?;
            cursor.seek(saved_pos);
            result
        }

        0x2C => {
            let class_tag_pos = cursor.pos();      // position of the class-name tag byte
            let class_val = read_value(cursor, seen, depth + 1)?;
            let class = perl_value_to_string(&class_val)?;
            seen.register_class(class_tag_pos + 1, class.clone());   // +1, matching the universal rule
            let inner = read_value(cursor, seen, depth + 1)?;
            PerlValue::wrap(PerlValue::Blessed(inner, class))
        }

        0x2D => {
            let offset = cursor.read_varint()? as usize;
            let class = seen.get_class(offset)
                .ok_or(SerealError::ClassOffsetNotFound(offset))?;
            let inner = read_value(cursor, seen, depth + 1)?;
            PerlValue::wrap(PerlValue::Blessed(inner, class))
        }

        0x32 => {
            // OBJECT_FREEZE — same shape as OBJECT, data went through FREEZE/THAW
            let class_val = read_value(cursor, seen, depth + 1)?;
            let class = perl_value_to_string(&class_val)?;
            let inner = read_value(cursor, seen, depth + 1)?;
            PerlValue::wrap(PerlValue::Blessed(inner, class))
        }
        0x33 => {
            // OBJECTV_FREEZE — same as OBJECTV, data went through FREEZE/THAW
            let offset = cursor.read_varint()? as usize;
            let class_val = seen.get(offset).ok_or(SerealError::OffsetNotFound(offset))?;
            let class = perl_value_to_string(&class_val)?;
            let inner = read_value(cursor, seen, depth + 1)?;
            PerlValue::wrap(PerlValue::Blessed(inner, class))
        }
        0x31 => {
            // REGEXP — pattern string, then modifiers string
            let pattern_val = read_value(cursor, seen, depth + 1)?;
            let flags_val = read_value(cursor, seen, depth + 1)?;
            let flags = perl_value_to_string(&flags_val)?;
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
            return read_value(cursor, seen, depth+1);
        }
        0x3E => {
            return Err(SerealError::UnsupportedTag("EXTEND"));
        }
        0x3C => {
            return Err(SerealError::UnsupportedTag("MANY"));
        }
        0x34 => PerlValue::wrap(PerlValue::No),   // NO (SvIsBOOL PL_No, Perl 5.36+)
        0x35 => PerlValue::wrap(PerlValue::Yes),  // YES (SvIsBOOL PL_Yes, Perl 5.36+)
        0x36..=0x37 => return Err(SerealError::UnsupportedTag("RESERVED")),
        0x38 => return Err(SerealError::UnsupportedTag("FLOAT_128")),

        0x3D => return Err(SerealError::UnexpectedPacketStart),

        0x20 => {
            let n = cursor.read_varint()?;
            if n > i64::MAX as u64 {
                PerlValue::wrap(PerlValue::UnsignedInteger(n))
            } else {
                PerlValue::wrap(PerlValue::Integer(n as i64))
            }
        }

        _ => return Err(SerealError::UnknownTag(tag)),
    };

    if track {
        seen.register(content_offset, value.clone());
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(5)));
    }

    #[test]
    fn neg_small_int() {
        let mut c = Cursor::new(&[0x1F]); // NEG_1 = -1
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen, 0).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(-1)));
    }

    #[test]
    fn undef_tag() {
        let mut c = Cursor::new(&[0x25]);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen, 0).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Undef));
    }

    #[test]
    fn true_false_tags() {
        let mut seen = SerealSeenTable::new();
        let mut c1 = Cursor::new(&[0x3B]);
        assert!(matches!(*read_value(&mut c1, &mut seen, 0).unwrap().borrow(), PerlValue::Yes));
        let mut c2 = Cursor::new(&[0x3A]);
        assert!(matches!(*read_value(&mut c2, &mut seen, 0).unwrap().borrow(), PerlValue::No));
    }

    #[test]
    fn varint_tag() {
        let mut c = Cursor::new(&[0x20, 0x2A]); // VARINT 42
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen, 0).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(42)));
    }

    #[test]
    fn zigzag_negative() {
        let mut c = Cursor::new(&[0x21, 0x01]); // zigzag 1 -> -1
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen, 0).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(-1)));
    }

    #[test]
    fn zigzag_positive() {
        let mut c = Cursor::new(&[0x21, 0x02]); // zigzag 2 -> 1
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen, 0).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(1)));
    }

    #[test]
    fn short_binary_tag() {
        let mut input = vec![0x66]; // SHORT_BINARY_6
        input.extend_from_slice(b"answer");
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
        assert!(std::rc::Rc::ptr_eq(&v, &seven));
    }

    #[test]
    fn offset_not_found_errors() {
        let mut seen = SerealSeenTable::new();
        let mut c = Cursor::new(&[0x29, 0x63]); // REFP offset=99, nothing registered
        assert_eq!(read_value(&mut c, &mut seen, 0).unwrap_err(), SerealError::OffsetNotFound(99));
    }

    #[test]
    fn object_tag() {
        // OBJECT: class "Foo" (SHORT_BINARY_3), data = POS_5
        let mut input = vec![0x2C];
        input.push(0x63); input.extend_from_slice(b"Foo"); // SHORT_BINARY_3 "Foo"
        input.push(0x05); // POS_5
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
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
        let v = read_value(&mut c, &mut seen, 0).unwrap();
        match &*v.borrow() {
            PerlValue::Double(f) => assert_eq!(*f, 2.718281828),
            other => panic!("expected Double: {:?}", other),
        }
    }

    #[test]
    fn pad_is_skipped() {
        let mut c = Cursor::new(&[0x3F, 0x05]); // PAD, then POS_5
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen, 0).unwrap();
        assert!(matches!(*v.borrow(), PerlValue::Integer(5)));
    }

    #[test]
    fn huge_array_count_does_not_allocate_unboundedly() {
        // ARRAY tag (0x2B) followed by a varint encoding a huge count (2^40),
        // but only a few actual bytes in the buffer.
        let mut input = vec![0x2B];
        // varint encoding of 2^40 = 1_099_511_627_776
        let mut n: u64 = 1u64 << 40;
        loop {
            let mut byte = (n & 0x7F) as u8;
            n >>= 7;
            if n != 0 { byte |= 0x80; }
            input.push(byte);
            if n == 0 { break; }
        }
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let result = read_value(&mut c, &mut seen, 0);
        assert!(result.is_err(), "should reject a count that exceeds remaining input, not attempt to allocate");
    }

    #[test]
    fn self_referential_copy_does_not_overflow_stack() {
        // COPY tag (0x2F) whose offset points at itself (offset 0)
        let input = vec![0x2F, 0x00];
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let result = read_value(&mut c, &mut seen, 0);
        assert_eq!(result.unwrap_err(), SerealError::MaxDepthExceeded);
    }

    #[test]
    fn deeply_nested_refn_hits_depth_limit_not_stack_overflow() {
        let mut input = vec![0x28; 200]; // comfortably past MAX_DEPTH=100
        input.push(0x00);
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let result = read_value(&mut c, &mut seen, 0);
        assert_eq!(result.unwrap_err(), SerealError::MaxDepthExceeded);
    }

    #[test]
    fn object_with_non_string_class_name_errors_not_panics() {
        // OBJECT tag (0x2C), class name given as POS_5 (an integer, not a string) — invalid
        let input = vec![0x2C, 0x05, 0x00];
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let result = read_value(&mut c, &mut seen, 0);
        assert!(result.is_err(), "should error, not panic, on non-string class name");
    }

    #[test]
    fn weaken_on_non_ref_errors_not_panics() {
        // WEAKEN tag (0x30) applied to POS_5 (an integer, not a Ref) — invalid
        let input = vec![0x30, 0x05];
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let result = read_value(&mut c, &mut seen, 0);
        assert!(result.is_err(), "should error, not panic, when WEAKEN target isn't a Ref");
    }

    #[test]
    fn hash_key_non_string_errors_not_panics() {
        // HASHREF_1 (0x51) whose key is POS_5 (an integer), not a string — invalid per spec
        let input = vec![0x51, 0x05, 0x0A];
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let result = read_value(&mut c, &mut seen, 0);
        assert!(result.is_err(), "should error, not panic, on non-string hash key");
    }


    #[test]
    fn packet_start_mid_stream_errors_clearly() {
        // 0x3D (PACKET_START) should never legally appear as a value tag mid-body
        let input = vec![0x3D];
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let result = read_value(&mut c, &mut seen, 0);
        assert_eq!(result.unwrap_err(), SerealError::UnexpectedPacketStart);
    }

    #[test]
    fn varint_at_i64_max_boundary_does_not_corrupt() {
            // u64::MAX = 18446744073709551615, which as i64 wraps to -1.
            // A correct parser must NOT silently return Integer(-1) here.
            let mut input = vec![0x20]; // VARINT tag
            let mut n: u64 = u64::MAX;
            loop {
                let mut byte = (n & 0x7F) as u8;
                n >>= 7;
                if n != 0 { byte |= 0x80; }
                input.push(byte);
                if n == 0 { break; }
            }
            let mut c = Cursor::new(&input);
            let mut seen = SerealSeenTable::new();
            let v = read_value(&mut c, &mut seen, 0).unwrap();
            match &*v.borrow() {
                PerlValue::UnsignedInteger(n) => assert_eq!(*n, u64::MAX),
                PerlValue::Integer(n) => panic!("u64::MAX silently corrupted to Integer({})", n),
                other => panic!("unexpected variant: {:?}", other),
            }
    }

    #[test]
    fn self_referential_hashref_resolves_correctly() {
        let mut input = vec![0x51 | 0x80]; // HASHREF_1, tracked — tag at position 0
        input.push(0x64); input.extend_from_slice(b"self");
        input.push(0x29); input.push(0x01); // REFP offset=1 (was 0x00) — tag_pos(0) + 1
       let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v = read_value(&mut c, &mut seen, 0).unwrap();

        let outer_hash_rc = match &*v.borrow() {
            PerlValue::Ref(inner) => inner.clone(),
            other => panic!("expected Ref: {:?}", other),
        };

        let inner_hash_rc = {
            let borrowed = outer_hash_rc.borrow();
            let map = match &*borrowed {
                PerlValue::Hash(m) => m,
                other => panic!("expected Hash: {:?}", other),
            };
            let self_ref = map.get(b"self".as_slice()).unwrap();
            match &*self_ref.borrow() {
                PerlValue::Ref(inner2) => inner2.clone(),
                other => panic!("expected self_ref to be a Ref: {:?}", other),
            }
        };

        // The two REFERENCE wrappers are legitimately different Rcs (Perl copies
        // the scalar), but they must point at the SAME underlying hash data.
        assert!(std::rc::Rc::ptr_eq(&outer_hash_rc, &inner_hash_rc),
            "self-reference should share the same underlying hash, even though the outer Ref wrappers differ");
    }

    #[test]
    fn huge_hash_count_does_not_allocate_unboundedly() {
        let mut input = vec![0x2A];
        let mut n: u64 = 1u64 << 40;
        loop {
            let mut byte = (n & 0x7F) as u8;
            n >>= 7;
            if n != 0 { byte |= 0x80; }
            input.push(byte);
            if n == 0 { break; }
        }
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let result = read_value(&mut c, &mut seen, 0);
        assert!(result.is_err(), "HASH count exceeding input should error, not allocate");
    }

    #[test]
    fn tracked_array_with_multibyte_count_uses_tag_plus_one_offset() {
        // ARRAY tag (0x2B), tracked, count=200 (needs a 2-byte varint)
        // followed by 200 POS_0 elements, then a REFP back to offset 1 (tag_pos=0, +1=1)
        let mut input = vec![0x2B | 0x80]; // ARRAY, tracked
        // varint encoding of 200: 200 = 0b1100_1000 -> low7=1001000|0x80=0xC8, high=1
        input.push(0xC8);
        input.push(0x01);
        for _ in 0..200 {
            input.push(0x00); // POS_0
        }
        input.push(0x29); // REFP
        input.push(0x01); // offset = 1 (tag_pos 0 + 1)

        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let array_val = read_value(&mut c, &mut seen, 0).unwrap();
        let refp_val = read_value(&mut c, &mut seen, 0).unwrap();

        match &*refp_val.borrow() {
            PerlValue::Ref(inner) => {
                assert!(std::rc::Rc::ptr_eq(inner, &array_val),
                    "REFP at offset (tag_pos+1) should resolve to the tracked array");
            }
            other => panic!("expected Ref: {:?}", other),
        }
    }

    #[test]
    fn objectv_resolves_class_name_by_offset_independent_of_track_bit() {
        // OBJECT "Pet" (untracked class-name tag) -> some data
        // followed by OBJECTV pointing at that class-name tag's offset -> more data
        let class_name_offset = 2; // was 1 — tag_pos(1) + 1
        let mut input = vec![0x2C]; // OBJECT, tag at position 0
        input.push(0x63); input.extend_from_slice(b"Pet"); // class-name tag at position 1
        input.push(0x05);
        input.push(0x2D); // OBJECTV
        input.push(class_name_offset as u8); // now 2
        input.push(0x06);
        let mut c = Cursor::new(&input);
        let mut seen = SerealSeenTable::new();
        let v1 = read_value(&mut c, &mut seen, 0).unwrap();
        let v2 = read_value(&mut c, &mut seen, 0).unwrap();

        match &*v1.borrow() {
            PerlValue::Blessed(_, class) => assert_eq!(class, "Pet"),
            other => panic!("expected Blessed: {:?}", other),
        }
        match &*v2.borrow() {
            PerlValue::Blessed(_, class) => assert_eq!(class, "Pet"),
            other => panic!("expected Blessed: {:?}", other),
        }
    }

    #[test]
    fn no_yes_tags_5_36_plus() {
        let mut seen = SerealSeenTable::new();
        let mut c1 = Cursor::new(&[0x34]);
        assert!(matches!(*read_value(&mut c1, &mut seen, 0).unwrap().borrow(), PerlValue::No));
        let mut c2 = Cursor::new(&[0x35]);
        assert!(matches!(*read_value(&mut c2, &mut seen, 0).unwrap().borrow(), PerlValue::Yes));
    }
}

