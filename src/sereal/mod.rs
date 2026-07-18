pub mod header;
pub mod value; 
pub mod body;

pub struct Cursor<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn read_byte(&mut self) -> Result<u8, SerealError> {
        if self.pos >= self.input.len() {
            return Err(SerealError::Truncated);
        }
        let b = self.input[self.pos];
        self.pos += 1;
        Ok(b)
    }

    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], SerealError> {
        if self.pos + n > self.input.len() {
            return Err(SerealError::Truncated);
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }


    /// Read a varint: 7 bits per byte, high bit means "more bytes follow".
    pub fn read_varint(&mut self) -> Result<u64, SerealError> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.read_byte()?;
            if shift >= 64 {
                return Err(SerealError::VarintTooLong);
            }
            result |= ((byte & 0x7F) as u64) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Ok(result)
    }

    pub fn remaining(&self) -> &'a [u8] {
    &self.input[self.pos..]
}
}

#[derive(Debug, PartialEq)]
pub enum SerealError {
    Truncated,
    VarintTooLong,
    BadMagic,
    UnknownEncoding(u8),
    UnsupportedTag(&'static str),
    UnknownTag(u8),
    MagicVersionMismatch,
    OffsetNotFound(usize),
    InvalidUtf8 
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_byte_varint() {
        // 5 fits in one byte, high bit clear
        let mut c = Cursor::new(&[0x05]);
        assert_eq!(c.read_varint(), Ok(5));
    }

    #[test]
    fn two_byte_varint() {
        // 300 = 0b1_00101100
        // low 7 bits: 0101100 = 0x2C, with high bit set (more follows) = 0xAC
        // remaining bits: 10 = 0x02
        let mut c = Cursor::new(&[0xAC, 0x02]);
        assert_eq!(c.read_varint(), Ok(300));
    }

    #[test]
    fn zero_varint() {
        let mut c = Cursor::new(&[0x00]);
        assert_eq!(c.read_varint(), Ok(0));
    }

    #[test]
    fn truncated_varint() {
        let mut c = Cursor::new(&[0x80]); // high bit set, no more bytes
        assert_eq!(c.read_varint(), Err(SerealError::Truncated));
    }
}