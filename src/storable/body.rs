use crate::storable::value::{ClassTable, PerlValue, SeenTable, ValueRef};

const SHF_TYPE_MASK: u8 = 0x03;
const SHF_LARGE_CLASSLEN: u8 = 0x04;
const SHF_LARGE_STRLEN: u8 = 0x08;
const SHF_LARGE_LISTLEN: u8 = 0x10;
const SHF_IDX_CLASSNAME: u8 = 0x20;
const SHF_NEED_RECURSE: u8 = 0x40;
const SHF_HAS_LIST: u8 = 0x80;

pub struct BodyConfig {
    pub iv_size: u8,      // sizeof(IV) — 4 or 8, determines SX_INTEGER width
    pub nv_size: u8,      // sizeof(NV) — usually 8, determines SX_DOUBLE width
    pub network_order: bool,
}

impl BodyConfig {
    /// Default config for hand-crafted test inputs (64-bit native).
    pub fn native_64() -> Self {
        Self { iv_size: 8, nv_size: 8, network_order: false }
    }
}

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

    pub fn pos(&self) -> usize {
        self.pos
    }
}

#[derive(Debug, PartialEq)]
pub enum BodyError {
    Truncated,
    UnknownTag(u8),
    InvalidUtf8,
    ObjectIndexOutOfRange(usize),
    ClassIndexOutOfRange(usize),   // NEW
    NotYetImplemented(&'static str)
}

pub fn read_value(
    cursor: &mut Cursor<'_>,
    seen: &mut SeenTable,
    classes: &mut ClassTable,
    config: &BodyConfig,
) -> Result<ValueRef, BodyError> {    let b = cursor.read_byte()?;
    match b {
        0x05 => Ok(PerlValue::wrap(PerlValue::Undef)),   // SX_UNDEF
        0x0E => Ok(PerlValue::wrap(PerlValue::Undef)),       // SX_SV_UNDEF (all versions)
        0x0F | 0x22 => Ok(PerlValue::wrap(PerlValue::Yes)),  // SX_SV_YES (old=0x0F, new=0x22)
        0x10 | 0x23 => Ok(PerlValue::wrap(PerlValue::No)),   // SX_SV_NO  (old=0x10, new=0x23)
        0x08 => {
            // SX_BYTE — signed byte with +128 bias
            let raw = cursor.read_byte()?;
            let val = PerlValue::wrap(PerlValue::Integer((raw as i16 - 128) as i64));
            seen.register(val.clone());
            Ok(val)
        }
        0x06 => {
            // SX_INTEGER — sizeof(IV) bytes, native byte order
            let bytes = cursor.read_bytes(config.iv_size as usize)?;
            let n = match config.iv_size {
                4 => i32::from_ne_bytes(bytes.try_into().unwrap()) as i64,
                8 => i64::from_ne_bytes(bytes.try_into().unwrap()),
                _ => return Err(BodyError::UnknownTag(0x06)), // unsupported IV size
            };
            let val = PerlValue::wrap(PerlValue::Integer(n));
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
            // SX_DOUBLE — sizeof(NV) bytes, native byte order
            let bytes = cursor.read_bytes(config.nv_size as usize)?;
            let n = match config.nv_size {
                8 => f64::from_ne_bytes(bytes.try_into().unwrap()),
                _ => return Err(BodyError::UnknownTag(0x07)),
            };
            let val = PerlValue::wrap(PerlValue::Double(n));
            seen.register(val.clone());
            Ok(val)
        }
        0x0A => {
            // SX_SCALAR — byte string, 1-byte length prefix
            let len = cursor.read_byte()? as usize;
            let bytes = cursor.read_bytes(len)?.to_vec();
            let val = PerlValue::wrap(PerlValue::Bytes(bytes));
            seen.register(val.clone());
            Ok(val)
        }
        0x17 => {
            // SX_UTF8STR — utf-8 string, 1-byte length prefix
            let len = cursor.read_byte()? as usize;
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
                items.push(read_value(cursor, seen, classes, config)?);
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
                let value = read_value(cursor, seen, classes, config)?;
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

            let inner = read_value(cursor, seen, classes, config)?;

            if let PerlValue::Ref(r) = &mut *val.borrow_mut() {
                *r = inner;
            }
            Ok(val)
        }

        0x00 => {
            // SX_OBJECT — back-reference to a previously-seen value
            // The index is always stored in network (big-endian) byte order, per Storable.xs
            let bytes = cursor.read_bytes(4)?;
            let index = i32::from_be_bytes(bytes.try_into().unwrap()) as usize;
            match seen.get(index) {
                Some(val) => Ok(val),
                None => Err(BodyError::ObjectIndexOutOfRange(index)),
            }
        }

        0x1B => {
            // SX_WEAKREF — like SX_REF but the inner pointer is weak
            // We need to parse the inner first to get an Rc, then downgrade to Weak
            let val = PerlValue::wrap(PerlValue::Ref(PerlValue::wrap(PerlValue::Undef)));
            seen.register(val.clone());

            let inner = read_value(cursor, seen, classes, config)?;
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

            let inner = read_value(cursor, seen, classes, config)?;  // NEW signature

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

            let inner = read_value(cursor, seen, classes, config)?;

            if let PerlValue::Blessed(r, _) = &mut *val.borrow_mut() {
                *r = inner;
            }
            Ok(val)
        }

        0x01 => {
            // SX_LSCALAR — same as SX_SCALAR, used for strings > i32::MAX bytes
            let len = cursor.read_i32_ne()? as usize;
            let bytes = cursor.read_bytes(len)?.to_vec();
            let val = PerlValue::wrap(PerlValue::Bytes(bytes));
            seen.register(val.clone());
            Ok(val)
        }
        0x18 => {
            // SX_LUTF8STR — same as SX_UTF8STR, used for strings > i32::MAX bytes
            let len = cursor.read_i32_ne()? as usize;
            let bytes = cursor.read_bytes(len)?;
            let s = std::str::from_utf8(bytes)
                .map_err(|_| BodyError::InvalidUtf8)?
                .to_string();
            let val = PerlValue::wrap(PerlValue::String(s));
            seen.register(val.clone());
            Ok(val)
        }

        0x1F => Ok(PerlValue::wrap(PerlValue::Undef)),  // SX_SVUNDEF_ELEM — array element undef
        0x14 => {
            // SX_OVERLOAD — a ref whose referent has overloaded operators
            let val = PerlValue::wrap(PerlValue::Overloaded(PerlValue::wrap(PerlValue::Undef)));
            seen.register(val.clone());
            let inner = read_value(cursor, seen, classes, config)?;
            if let PerlValue::Overloaded(r) = &mut *val.borrow_mut() {
                *r = inner;
            }
            Ok(val)
        }
        0x1C => {
            // SX_WEAKOVERLOAD — like SX_OVERLOAD but the pointer is weak
            let val = PerlValue::wrap(PerlValue::Overloaded(PerlValue::wrap(PerlValue::Undef)));
            seen.register(val.clone());
            let inner = read_value(cursor, seen, classes, config)?;
            let weak = std::rc::Rc::downgrade(&inner);
            *val.borrow_mut() = PerlValue::WeakOverloaded(weak);
            Ok(val)
        }
        
        0x1D => {
            // SX_VSTRING — vstring bytes (1-byte length) followed by actual scalar value
            // retrieve_vstring uses GETMARK (1 byte) for length, not RLEN (4 bytes)
            let len = cursor.read_byte()? as usize;
            let vbytes = cursor.read_bytes(len)?.to_vec();
            // Read the following scalar (which gets registered in seen by its own handler)
            let inner = read_value(cursor, seen, classes, config)?;
            // Return a VString wrapping the bytes; don't register vstring itself
            let val = PerlValue::wrap(PerlValue::VString(vbytes));
            Ok(val)
        }
        0x1E => {
            // SX_LVSTRING — same but length is i32 (4 bytes)
            let len = cursor.read_i32_ne()? as usize;
            let vbytes = cursor.read_bytes(len)?.to_vec();
            let inner = read_value(cursor, seen, classes, config)?;
            let val = PerlValue::wrap(PerlValue::VString(vbytes));
            Ok(val)
        }

        0x19 => {
            // SX_FLAG_HASH — hash with per-key flags
            let val = PerlValue::wrap(PerlValue::FlagHash {
                hash_flags: 0,
                entries: std::collections::HashMap::new(),
            });
            seen.register(val.clone());

            let hash_flags = cursor.read_byte()?;
            let count = cursor.read_i32_ne()? as usize;
            let mut entries = std::collections::HashMap::with_capacity(count);
            for _ in 0..count {
                let value = read_value(cursor, seen, classes, config)?;
                let key_flags = cursor.read_byte()?;
                let key_len = cursor.read_i32_ne()? as usize;
                let key = cursor.read_bytes(key_len)?.to_vec();
                entries.insert(key, (key_flags, value));
            }

            *val.borrow_mut() = PerlValue::FlagHash { hash_flags, entries };
            Ok(val)
        }
       
        0x0D => {
            // SX_TIED_SCALAR
            let val = PerlValue::wrap(PerlValue::TiedScalar(PerlValue::wrap(PerlValue::Undef)));
            seen.register(val.clone());
            let inner = read_value(cursor, seen, classes, config)?;
            if let PerlValue::TiedScalar(r) = &mut *val.borrow_mut() {
                *r = inner;
            }
            Ok(val)
        }
        0x0B => {
            // SX_TIED_ARRAY
            let val = PerlValue::wrap(PerlValue::TiedArray(PerlValue::wrap(PerlValue::Undef)));
            seen.register(val.clone());
            let inner = read_value(cursor, seen, classes, config)?;
            if let PerlValue::TiedArray(r) = &mut *val.borrow_mut() {
                *r = inner;
            }
            Ok(val)
        }
        0x0C => {
            // SX_TIED_HASH
            let val = PerlValue::wrap(PerlValue::TiedHash(PerlValue::wrap(PerlValue::Undef)));
            seen.register(val.clone());
            let inner = read_value(cursor, seen, classes, config)?;
            if let PerlValue::TiedHash(r) = &mut *val.borrow_mut() {
                *r = inner;
            }
            Ok(val)
        }

        0x15 => {
            // SX_TIED_KEY — tied hash key
            let val = PerlValue::wrap(PerlValue::TiedKey(
                PerlValue::wrap(PerlValue::Undef),
                PerlValue::wrap(PerlValue::Undef),
            ));
            seen.register(val.clone());
            let object = read_value(cursor, seen, classes, config)?;
            let key = read_value(cursor, seen, classes, config)?;
            *val.borrow_mut() = PerlValue::TiedKey(object, key);
            Ok(val)
        }
        0x16 => {
            // SX_TIED_IDX — tied array index
            let val = PerlValue::wrap(PerlValue::TiedIdx(
                PerlValue::wrap(PerlValue::Undef),
                0,
            ));
            seen.register(val.clone());
            let object = read_value(cursor, seen, classes, config)?;
            let idx = cursor.read_i32_ne()? as i64;
            *val.borrow_mut() = PerlValue::TiedIdx(object, idx);
            Ok(val)
        }

        0x1A => {
            // SX_CODE — code ref stored as deparsed source
            // Wire: <type:byte> <scalar following that type's format>
            // type is one of SX_SCALAR(0x0A), SX_LSCALAR(0x01), SX_UTF8STR(0x17), SX_LUTF8STR(0x18)
            // Storable also registers a dummy entry in aseen before reading the scalar,
            // so we register a placeholder first.
            let placeholder = PerlValue::wrap(PerlValue::Undef);
            seen.register(placeholder);

            let type_byte = cursor.read_byte()?;
            let source = match type_byte {
                0x0A => {
                    // SX_SCALAR: 1-byte length
                    let len = cursor.read_byte()? as usize;
                    let bytes = cursor.read_bytes(len)?;
                    let inner = PerlValue::wrap(PerlValue::Bytes(bytes.to_vec()));
                    seen.register(inner);
                    String::from_utf8_lossy(bytes).into_owned()
                }
                0x01 => {
                    // SX_LSCALAR: 4-byte length
                    let len = cursor.read_i32_ne()? as usize;
                    let bytes = cursor.read_bytes(len)?;
                    let inner = PerlValue::wrap(PerlValue::Bytes(bytes.to_vec()));
                    seen.register(inner);
                    String::from_utf8_lossy(bytes).into_owned()
                }
                0x17 => {
                    // SX_UTF8STR: 1-byte length
                    let len = cursor.read_byte()? as usize;
                    let bytes = cursor.read_bytes(len)?;
                    let s = std::str::from_utf8(bytes).map_err(|_| BodyError::InvalidUtf8)?.to_string();
                    let inner = PerlValue::wrap(PerlValue::String(s.clone()));
                    seen.register(inner);
                    s
                }
                0x18 => {
                    // SX_LUTF8STR: 4-byte length
                    let len = cursor.read_i32_ne()? as usize;
                    let bytes = cursor.read_bytes(len)?;
                    let s = std::str::from_utf8(bytes).map_err(|_| BodyError::InvalidUtf8)?.to_string();
                    let inner = PerlValue::wrap(PerlValue::String(s.clone()));
                    seen.register(inner);
                    s
                }
                _ => return Err(BodyError::UnknownTag(type_byte)),
            };
            let val = PerlValue::wrap(PerlValue::Code(source));
            seen.register(val.clone());
            Ok(val)
        }

        0x20 => {
            // SX_REGEXP — op_flags byte, then pattern, then flags string
            // Format: <op_flags:byte> <re_len:1or4 bytes> <pattern bytes> <flags_len:byte> <flags bytes>
            let op_flags = cursor.read_byte()?;
            let re_len = if op_flags & 0x01 != 0 {
                cursor.read_i32_ne()? as usize   // SHR_U32_RE_LEN set: 4-byte length
            } else {
                cursor.read_byte()? as usize     // 1-byte length
            };
            let pattern = cursor.read_bytes(re_len)?.to_vec();
            let flags_len = cursor.read_byte()? as usize;
            let flags_bytes = cursor.read_bytes(flags_len)?.to_vec();
            let flags = String::from_utf8(flags_bytes).unwrap_or_default();
            let val = PerlValue::wrap(PerlValue::Regexp {
                pattern: PerlValue::wrap(PerlValue::Bytes(pattern)),
                flags,   // the String, not op_flags
            });
            seen.register(val.clone());
            Ok(val)
        }

        0x13 => {
            // SX_HOOK — user-defined STORABLE_freeze output
            // Register a placeholder immediately (children may back-reference it)
            let val = PerlValue::wrap(PerlValue::Hook {
                class: String::new(),
                obj_type: 0,
                frozen: Vec::new(),
                refs: Vec::new(),
                recurse: Vec::new(),
            });
            seen.register(val.clone());

            let mut flags = cursor.read_byte()?;

            // Handle SHF_NEED_RECURSE: read inner values, re-read flags after each
            let mut recurse = Vec::new();
            while flags & SHF_NEED_RECURSE != 0 {
                recurse.push(read_value(cursor, seen, classes, config)?);
                flags = cursor.read_byte()?;
            }

            let obj_type = flags & SHF_TYPE_MASK;

            // Class name — by index or by string, byte or i32 length
            let class = if flags & SHF_IDX_CLASSNAME != 0 {
                let idx = if flags & SHF_LARGE_CLASSLEN != 0 {
                    cursor.read_i32_ne()? as usize
                } else {
                    cursor.read_byte()? as usize
                };
                classes.get(idx)
                    .ok_or(BodyError::ClassIndexOutOfRange(idx))?
                    .to_string()
            } else {
                let name_len = if flags & SHF_LARGE_CLASSLEN != 0 {
                    cursor.read_i32_ne()? as usize
                } else {
                    cursor.read_byte()? as usize
                };
                let name = String::from_utf8(cursor.read_bytes(name_len)?.to_vec())
                    .map_err(|_| BodyError::InvalidUtf8)?;
                classes.register(name.clone());
                name
            };

            // Frozen payload — byte or u32 length
            let frozen_len = if flags & SHF_LARGE_STRLEN != 0 {
                cursor.read_i32_ne()? as usize
            } else {
                cursor.read_byte()? as usize
            };
            let frozen = cursor.read_bytes(frozen_len)?.to_vec();

            // Optional object-ID list — indices into seen table, network byte order
            let mut refs = Vec::new();
            if flags & SHF_HAS_LIST != 0 {
                let count = if flags & SHF_LARGE_LISTLEN != 0 {
                    cursor.read_i32_ne()? as usize
                } else {
                    cursor.read_byte()? as usize
                };
                for _ in 0..count {
                    let bytes = cursor.read_bytes(4)?;
                    let tag = i32::from_be_bytes(bytes.try_into().unwrap()) as usize;
                    let referenced = seen.get(tag)
                        .ok_or(BodyError::ObjectIndexOutOfRange(tag))?;
                    refs.push(referenced);
                }
            }

            *val.borrow_mut() = PerlValue::Hook {
                class,
                obj_type,
                frozen,
                refs,
                recurse,
            };
            Ok(val)
        }

        0x21 => Err(BodyError::NotYetImplemented("SX_LOBJECT")),

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
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        assert_eq!(
            read_value(&mut cursor(&[0x99]), &mut seen, &mut classes, &config).unwrap_err(),
            BodyError::UnknownTag(0x99)
        );
    }

    #[test]
    fn undef_tag() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let val = read_value(&mut cursor(&[0x05]), &mut seen, &mut classes, &config).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::Undef));
    }

    #[test]
    fn yes_tag() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let val = read_value(&mut cursor(&[0x0F]), &mut seen, &mut classes, &config).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::Yes));
    }

    #[test]
    fn no_tag() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let val = read_value(&mut cursor(&[0x10]), &mut seen, &mut classes, &config).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::No));
    }

    #[test]
    fn immortals_are_not_indexed() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        read_value(&mut cursor(&[0x05]), &mut seen, &mut classes, &config).unwrap();  // SX_UNDEF
        read_value(&mut cursor(&[0x0F]), &mut seen, &mut classes, &config).unwrap();  // SX_SV_YES old
        read_value(&mut cursor(&[0x22]), &mut seen, &mut classes, &config).unwrap();  // SX_SV_YES new
        read_value(&mut cursor(&[0x10]), &mut seen, &mut classes, &config).unwrap();  // SX_SV_NO old
        read_value(&mut cursor(&[0x23]), &mut seen, &mut classes, &config).unwrap();  // SX_SV_NO new
        assert_eq!(seen.len(), 0);
    }

    #[test]
    fn byte_becomes_integer_and_is_indexed() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let val = read_value(&mut cursor(&[0x08, 0xAA]), &mut seen, &mut classes, &config).unwrap();
        assert!(matches!(*val.borrow(), PerlValue::Integer(42)));
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn scalar_bytes() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x0A, 3];
        input.extend_from_slice(b"abc");
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Bytes(b) => assert_eq!(b, b"abc"),
            _ => panic!("expected Bytes"),
        }
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn utf8_string() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x17, 5];
        input.extend_from_slice(b"hello");
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::String(s) => assert_eq!(s, "hello"),
            _ => panic!("expected String"),
        }
    }

    #[test]
    fn invalid_utf8_errors() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x17, 2];
        input.extend_from_slice(&[0xFF, 0xFE]);
        assert_eq!(
            read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap_err(),
            BodyError::InvalidUtf8
        );
    }

    #[test]
    fn empty_array() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x02];
        input.extend_from_slice(&0i32.to_ne_bytes());
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Array(items) => assert!(items.is_empty()),
            _ => panic!("expected Array"),
        }
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn array_of_bytes() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x02];
        input.extend_from_slice(&2i32.to_ne_bytes());
        input.extend_from_slice(&[0x08, 0xAA]);
        input.extend_from_slice(&[0x08, 0x85]);
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Array(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(*items[0].borrow(), PerlValue::Integer(42)));
                assert!(matches!(*items[1].borrow(), PerlValue::Integer(5)));
            }
            _ => panic!("expected Array"),
        }
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn simple_hash() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x03];
        input.extend_from_slice(&1i32.to_ne_bytes());
        input.extend_from_slice(&[0x08, 0xAA]);
        input.extend_from_slice(&6i32.to_ne_bytes());
        input.extend_from_slice(b"answer");
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
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
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let input = vec![0x04, 0x08, 0xAA];
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Ref(inner) => {
                assert!(matches!(*inner.borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected Ref"),
        }
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn back_reference() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x02];
        input.extend_from_slice(&2i32.to_ne_bytes());
        input.extend_from_slice(&[0x08, 0xAA]);
        input.push(0x00);
        input.extend_from_slice(&1i32.to_be_bytes());
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Array(items) => {
                assert_eq!(items.len(), 2);
                assert!(std::rc::Rc::ptr_eq(&items[0], &items[1]));
                assert!(matches!(*items[0].borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected Array"),
        }
    }

    #[test]
    fn back_reference_out_of_range() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x00];
        input.extend_from_slice(&999i32.to_be_bytes());
        assert_eq!(
            read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap_err(),
            BodyError::ObjectIndexOutOfRange(999)
        );
    }

    #[test]
    fn weakref_downgrade() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let input = vec![0x1B, 0x08, 0xAA];
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::WeakRef(w) => {
                let upgraded = w.upgrade().expect("still alive in seen table");
                assert!(matches!(*upgraded.borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected WeakRef"),
        }
    }

    #[test]
    fn cycle_does_not_leak() {
        use std::rc::Rc;
        let mut input = vec![0x02];
        input.extend_from_slice(&1i32.to_ne_bytes());
        input.push(0x1B);
        input.push(0x00);
        input.extend_from_slice(&0i32.to_be_bytes());
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        let strong_count_before = Rc::strong_count(&val);
        drop(seen);
        assert_eq!(Rc::strong_count(&val), 1,
            "cycle should not keep itself alive (was {} before drop, {} after)",
            strong_count_before, Rc::strong_count(&val));
    }

    #[test]
    fn blessed_hashref() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x11];
        input.push(6);
        input.extend_from_slice(b"Animal");
        input.push(0x04);
        input.push(0x03);
        input.extend_from_slice(&1i32.to_ne_bytes());
        input.extend_from_slice(&[0x08, 0x83]);
        input.extend_from_slice(&3i32.to_ne_bytes());
        input.extend_from_slice(b"age");
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
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

    #[test]
    fn lscalar_bytes() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x01];
        input.extend_from_slice(&3i32.to_ne_bytes());
        input.extend_from_slice(b"xyz");
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Bytes(b) => assert_eq!(b, b"xyz"),
            _ => panic!("expected Bytes"),
        }
    }

    #[test]
    fn lutf8str_string() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x18];
        input.extend_from_slice(&5i32.to_ne_bytes());
        input.extend_from_slice(b"world");
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::String(s) => assert_eq!(s, "world"),
            _ => panic!("expected String"),
        }
    }

    #[test]
    fn overloaded_ref() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let input = vec![0x14, 0x08, 0xAA];
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Overloaded(inner) => {
                assert!(matches!(*inner.borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected Overloaded"),
        }
    }

    #[test]
    fn weak_overloaded() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let input = vec![0x1C, 0x08, 0xAA];
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        assert!(matches!(&*val.borrow(), PerlValue::WeakOverloaded(_)));
    }

    #[test]
    fn vstring_short() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x1D];
        input.push(3);                       // 1-byte length
        input.extend_from_slice(&[1, 2, 3]); // vstring bytes
        input.push(0x0A);                    // SX_SCALAR following
        input.push(3);                       // 1-byte scalar length
        input.extend_from_slice(&[1, 2, 3]); // scalar bytes
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::VString(b) => assert_eq!(b, &[1, 2, 3]),
            _ => panic!("expected VString"),
        }
    }

    #[test]
    fn flag_hash_simple() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x19];
        input.push(0x01);
        input.extend_from_slice(&1i32.to_ne_bytes());
        input.extend_from_slice(&[0x08, 0xAA]);
        input.push(0x00);
        input.extend_from_slice(&3i32.to_ne_bytes());
        input.extend_from_slice(b"key");
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::FlagHash { hash_flags, entries } => {
                assert_eq!(*hash_flags, 0x01);
                assert_eq!(entries.len(), 1);
                let (key_flags, v) = entries.get(b"key".as_slice()).unwrap();
                assert_eq!(*key_flags, 0);
                assert!(matches!(*v.borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected FlagHash"),
        }
    }

    #[test]
    fn tied_scalar() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let input = vec![0x0D, 0x08, 0xAA];
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::TiedScalar(inner) => {
                assert!(matches!(*inner.borrow(), PerlValue::Integer(42)));
            }
            _ => panic!("expected TiedScalar"),
        }
    }

    #[test]
    fn tied_idx() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x16, 0x08, 0xAA];
        input.extend_from_slice(&5i32.to_ne_bytes());
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::TiedIdx(object, idx) => {
                assert!(matches!(*object.borrow(), PerlValue::Integer(42)));
                assert_eq!(*idx, 5);
            }
            _ => panic!("expected TiedIdx"),
        }
    }

    #[test]
    fn code_ref() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let src = "sub { $_[0] + 1 }";
        let mut input = vec![0x1A];
        input.extend_from_slice(&(src.len() as i32).to_ne_bytes());
        input.extend_from_slice(src.as_bytes());
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Code(s) => assert_eq!(s, src),
            _ => panic!("expected Code"),
        }
    }

    #[test]
    fn regexp_simple() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        // SX_REGEXP: op_flags=0, re_len=3 (1 byte), "abc", flags_len=1, "i"
        let mut input = vec![0x20];
        input.push(0x00);              // op_flags
        input.push(3);                 // re_len (1 byte)
        input.extend_from_slice(b"abc");
        input.push(1);                 // flags_len
        input.extend_from_slice(b"i"); // flags
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Regexp { pattern, flags } => {
                assert_eq!(flags, "i");
                match &*pattern.borrow() {
                    PerlValue::Bytes(b) => assert_eq!(b, b"abc"),
                    _ => panic!("expected Bytes pattern"),
                }
            }
            _ => panic!("expected Regexp"),
        }
    }

    #[test]
    fn hook_minimal() {
        let mut seen = SeenTable::new();
        let mut classes = ClassTable::new();
        let config = BodyConfig::native_64();
        let mut input = vec![0x13];
        input.push(0x00);
        input.push(5);
        input.extend_from_slice(b"MyPkg");
        input.push(4);
        input.extend_from_slice(b"data");
        let val = read_value(&mut cursor(&input), &mut seen, &mut classes, &config).unwrap();
        match &*val.borrow() {
            PerlValue::Hook { class, obj_type, frozen, refs, recurse } => {
                assert_eq!(class, "MyPkg");
                assert_eq!(*obj_type, 0);
                assert_eq!(frozen, b"data");
                assert!(refs.is_empty());
                assert!(recurse.is_empty());
            }
            _ => panic!("expected Hook"),
        }
    }
}