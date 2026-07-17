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
}