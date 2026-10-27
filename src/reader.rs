//! A bounds-checked cursor over the bytes of a SerializedFile or of a script's serialized fields.

use crate::{Result, invalid};

/// A bounds-checked little- or big-endian reader over a byte slice.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    pub big_endian: bool,
}

macro_rules! read_num {
    ($($f:ident: $t:ty),*) => {$(
        pub fn $f(&mut self) -> Result<$t> {
            let b = self.take(std::mem::size_of::<$t>())?;
            let a = b.try_into().map_err(|_| invalid("a number cut short"))?;
            Ok(if self.big_endian { <$t>::from_be_bytes(a) } else { <$t>::from_le_bytes(a) })
        }
    )*};
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], big_endian: bool) -> Self {
        Reader {
            data,
            pos: 0,
            big_endian,
        }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let short = || {
            invalid(format!(
                "{n} bytes wanted at offset {}, {} left",
                self.pos,
                self.remaining()
            ))
        };
        let end = self.pos.checked_add(n).ok_or_else(short)?;
        let s = self.data.get(self.pos..end).ok_or_else(short)?;
        self.pos = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        let [b] = *self.take(1)? else {
            return Err(invalid(format!("a byte wanted at offset {}", self.pos)));
        };
        Ok(b)
    }

    pub fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(invalid(format!(
                "bool byte {b} at offset {} (0 or 1 expected)",
                self.pos.saturating_sub(1)
            ))),
        }
    }

    read_num!(i8: i8, i16: i16, u16: u16, i32: i32, u32: u32, i64: i64, u64: u64, f32: f32, f64: f64);

    /// Align the position to `n` bytes (from the start of the data).
    pub fn align(&mut self, n: usize) -> Result<()> {
        if n == 0 {
            return Ok(());
        }
        let pad = self.pos.next_multiple_of(n).saturating_sub(self.pos);
        self.take(pad).map(|_| ())
    }

    /// A NUL-terminated UTF-8 string.
    pub fn cstr(&mut self) -> Result<String> {
        let rest = self.data.get(self.pos..).ok_or_else(|| {
            invalid(format!(
                "a string wanted at offset {}, past the {} bytes",
                self.pos,
                self.data.len()
            ))
        })?;
        let n = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| invalid(format!("unterminated string at offset {}", self.pos)))?;
        let s = rest
            .get(..n)
            .and_then(|b| std::str::from_utf8(b).ok())
            .ok_or_else(|| invalid(format!("string at offset {} is not UTF-8", self.pos)))?
            .to_owned();
        // the string and its terminator lie inside `rest`
        self.pos = self.pos.saturating_add(n).saturating_add(1);
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cursor placed past the data (its position is public) reads nothing and says where, instead of slicing out of
    /// range; a wanted length that would wrap the position is a refusal.
    #[test]
    fn a_cursor_past_the_end_refuses() {
        let mut r = Reader::new(b"ab\0", false);
        r.pos = 9;
        assert!(r.cstr().unwrap_err().to_string().contains("offset 9"));
        r.pos = usize::MAX;
        assert!(r.take(2).unwrap_err().to_string().contains("offset"));
        assert_eq!(Reader::new(b"ab\0", false).cstr().unwrap(), "ab");
        let mut r = Reader::new(&[0, 2], false);
        assert!(!r.bool().unwrap());
        assert!(
            r.bool()
                .unwrap_err()
                .to_string()
                .contains("bool byte 2 at offset 1")
        );
        assert!(Reader::new(&[], false).align(0).is_ok());
    }
}
