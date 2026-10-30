//! The backward bitstream for Huffman symbols and extra bits.
//!
//! The bitstream reads 16-bit little-endian words from the end of the block toward the start.
//! It gives the bits of each word from the most significant bit. The reader keeps at least 16
//! bits in its buffer after each read, the same as `cabinet.dll`. Thus it reads a word before
//! the bits of the word are necessary, and the overlap check at the end of the block gives the
//! same result as `cabinet.dll`.

use crate::error::{Error, Result};
use crate::range::word_at;

/// The minimum number of bits in the buffer after each read.
const MIN_BITS: u32 = 16;

/// The backward bit reader.
pub(crate) struct BitReader<'a> {
    input: &'a [u8],
    /// The offset after the last unread byte. The next word is at `end - 2`.
    end: usize,
    /// The buffered bits, the next bit in bit 63.
    buffer: u64,
    /// The number of valid bits in `buffer`.
    valid: u32,
}

impl<'a> BitReader<'a> {
    /// Start to read `input` from its end. The caller makes sure that `input` has at least 4
    /// bytes.
    pub(crate) fn new(input: &'a [u8]) -> Result<Self> {
        let mut reader = Self {
            input,
            end: input.len(),
            buffer: 0,
            valid: 0,
        };
        reader.refill(2 * MIN_BITS)?;
        Ok(reader)
    }

    /// Return the offset after the last unread byte.
    pub(crate) fn unread_end(&self) -> usize {
        self.end
    }

    /// Read words until the buffer has at least `bits` bits.
    fn refill(&mut self, bits: u32) -> Result<()> {
        while self.valid < bits {
            let Some(start) = self.end.checked_sub(2) else {
                return Err(Error::unexpected_end().in_structure("bitstream"));
            };
            let Some(word) = word_at(self.input, start) else {
                return Err(Error::unexpected_end().in_structure("bitstream"));
            };
            self.end = start;
            self.buffer |= u64::from(word) << (48 - self.valid);
            self.valid += 16;
        }
        Ok(())
    }

    /// Return the next 15 bits without a read. The buffer always has them.
    pub(crate) fn peek15(&self) -> u32 {
        let bits = self.buffer >> (64 - 15);
        // The shift leaves 15 bits, so the value fits.
        u32::try_from(bits).unwrap_or_default()
    }

    /// Remove `count` bits from the buffer, then refill it. `count` is at most 16.
    pub(crate) fn consume(&mut self, count: u32) -> Result<()> {
        self.buffer <<= count;
        self.valid -= count;
        self.refill(MIN_BITS)
    }

    /// Read `count` bits as a number. `count` is from 1 to 32.
    pub(crate) fn read(&mut self, count: u32) -> Result<u32> {
        self.refill(count)?;
        let value = u32::try_from(self.buffer >> (64 - count)).unwrap_or_default();
        self.buffer <<= count;
        self.valid -= count;
        self.refill(MIN_BITS)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reader_starts_at_the_last_word() {
        // Words from the end: 0x8001, then 0x00ff, then 0x1234.
        let input = [0x34, 0x12, 0xff, 0x00, 0x01, 0x80];
        let mut reader = BitReader::new(&input).unwrap();
        assert_eq!(reader.unread_end(), 2);
        assert_eq!(reader.read(1).unwrap(), 1);
        assert_eq!(reader.read(15).unwrap(), 1);
        assert_eq!(reader.unread_end(), 2);
        assert_eq!(reader.read(16).unwrap(), 0x00ff);
        assert_eq!(reader.unread_end(), 0);
        // The reader cannot keep 16 bits after this read, so the read fails.
        assert!(reader.read(16).is_err());
    }

    #[test]
    fn a_long_read_crosses_words() {
        let input = [0xcd, 0xab, 0x78, 0x56, 0x34, 0x12];
        let mut reader = BitReader::new(&input).unwrap();
        assert_eq!(reader.read(4).unwrap(), 0x1);
        assert_eq!(reader.read(24).unwrap(), 0x23_4567);
    }
}
