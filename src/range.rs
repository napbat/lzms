//! The range decoder and the adaptive binary decisions.
//!
//! The range decoder reads 16-bit little-endian words from the start of the block. Each
//! decision has a state: the last bits that it decoded. The state selects one probability
//! from a table. Each probability counts the zero bits in the last 64 bits that it decoded.

use crate::error::{Error, Result};

/// The number of bits in a probability. A probability is a count out of 64.
const PROBABILITY_BITS: u32 = 6;

/// The adaptive probability of one binary decision state.
#[derive(Clone, Copy)]
struct Probability {
    /// The number of zero bits in `recent`.
    zeros: u32,
    /// The last 64 bits, the newest bit in bit 0.
    recent: u64,
}

impl Probability {
    /// The start value: 48 zero bits in 64.
    const INITIAL: Self = Self {
        zeros: 48,
        recent: 0x5555_5555,
    };

    /// Record a decoded bit.
    fn update(&mut self, bit: u32) {
        let oldest = self.recent >> 63;
        if bit == 0 {
            if oldest == 1 {
                self.zeros += 1;
            }
            self.recent <<= 1;
        } else {
            if oldest == 0 {
                self.zeros -= 1;
            }
            self.recent = (self.recent << 1) | 1;
        }
    }
}

/// The range decoder.
pub(crate) struct RangeDecoder<'a> {
    input: &'a [u8],
    /// The offset of the next word to read.
    next: usize,
    range: u32,
    code: u32,
}

impl<'a> RangeDecoder<'a> {
    /// Start to decode `input`. The caller makes sure that `input` has at least 4 bytes.
    pub(crate) fn new(input: &'a [u8]) -> Result<Self> {
        let (Some(high), Some(low)) = (word_at(input, 0), word_at(input, 2)) else {
            return Err(Error::unexpected_end().in_structure("range decoder"));
        };
        Ok(Self {
            input,
            next: 4,
            range: u32::MAX,
            code: (u32::from(high) << 16) | u32::from(low),
        })
    }

    /// Return the number of bytes that the decoder read.
    pub(crate) fn consumed(&self) -> usize {
        self.next
    }

    /// Decode one bit with a probability.
    fn bit(&mut self, probability: &mut Probability) -> Result<u32> {
        if self.range <= 0xFFFF {
            let Some(word) = word_at(self.input, self.next) else {
                return Err(Error::unexpected_end()
                    .at_offset(self.next as u64)
                    .in_structure("range decoder"));
            };
            self.next += 2;
            self.range <<= 16;
            self.code = (self.code << 16) | u32::from(word);
        }
        let weight = probability.zeros.clamp(1, (1 << PROBABILITY_BITS) - 1);
        let bound = (self.range >> PROBABILITY_BITS) * weight;
        let bit = if self.code < bound {
            self.range = bound;
            0
        } else {
            self.range -= bound;
            self.code -= bound;
            1
        };
        probability.update(bit);
        Ok(bit)
    }
}

/// A binary decision with `N` states. `N` is a power of two.
pub(crate) struct Decision<const N: usize> {
    state: usize,
    probabilities: [Probability; N],
}

impl<const N: usize> Decision<N> {
    /// Make a decision in the start state.
    pub(crate) fn new() -> Self {
        Self {
            state: 0,
            probabilities: [Probability::INITIAL; N],
        }
    }

    /// Decode one bit, and add it to the state.
    pub(crate) fn decode(&mut self, decoder: &mut RangeDecoder<'_>) -> Result<bool> {
        let bit = decoder.bit(&mut self.probabilities[self.state])?;
        self.state = ((self.state << 1) | usize::from(bit == 1)) % N;
        Ok(bit == 1)
    }
}

/// Read the little-endian word at `offset`, if the input has two bytes there.
pub(crate) fn word_at(input: &[u8], offset: usize) -> Option<u16> {
    let bytes = input.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_initial_probability_has_48_zero_bits() {
        let p = Probability::INITIAL;
        assert_eq!(64 - p.recent.count_ones(), p.zeros);
    }

    #[test]
    fn the_zero_count_follows_the_last_64_bits() {
        let mut p = Probability::INITIAL;
        for i in 0..500u32 {
            p.update(u32::from(i % 3 == 0));
            assert_eq!(64 - p.recent.count_ones(), p.zeros);
        }
    }
}
