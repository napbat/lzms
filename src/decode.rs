//! The decoder for one raw LZMS block.
//!
//! A block has two streams in one buffer. The range decoder reads the decisions from the
//! start. The backward bitstream reads the Huffman symbols and the extra bits from the end.
//! Each item is one of these:
//!
//! - A literal byte.
//! - An LZ match: copy `length` bytes from `offset` bytes before.
//! - A delta match: a byte is the sum of the byte `span` before it and the change between
//!   the two bytes at `offset` and at `offset + span` before it.
//!
//! A match can use one of three recent offsets. The queue of recent offsets adds the offset of
//! a match only after the next item. After the last item, the decoder undoes the x86 filter.

use crate::bits::BitReader;
use crate::error::{Error, Result};
use crate::huffman::AdaptiveCode;
use crate::prelude::*;
use crate::range::{Decision, RangeDecoder};
use crate::tables::{self, LENGTH_SLOTS, LENGTHS, OFFSETS, SlotTable};
use crate::x86;

/// The largest block that the decoder accepts, in bytes (1 GiB).
pub const MAX_BLOCK_SIZE: usize = 1 << 30;

/// A queue of recent match sources, with the update after one item.
struct Recent<T: Copy> {
    items: [T; 4],
    /// The source of the last item, which goes to the front after the next item.
    pending: Option<T>,
}

impl<T: Copy> Recent<T> {
    /// Remove the item at `index` (0 to 2) from the queue and return it.
    fn take(&mut self, index: usize) -> T {
        let item = self.items[index];
        self.items.copy_within(index + 1.., index);
        item
    }

    /// End an item: put the pending source at the front, and keep `current` for later.
    fn finish(&mut self, current: Option<T>) {
        if let Some(item) = self.pending {
            self.items.copy_within(..3, 1);
            self.items[0] = item;
        }
        self.pending = current;
    }
}

/// A delta match source: the power of two of the span, and the offset in spans.
#[derive(Clone, Copy)]
struct DeltaSource {
    power: u32,
    offset: u32,
}

/// A reusable LZMS decoder.
///
/// The decoder keeps a 256 KiB table for the x86 filter between blocks. Use one decoder for
/// many blocks to prevent an allocation for each block.
pub struct Decoder {
    last_use: Vec<u32>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// Make a decoder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_use: vec![0; x86::TARGETS],
        }
    }

    /// Decompress one raw LZMS block into `output`.
    ///
    /// The length of `output` is the uncompressed size of the block. A block with the same
    /// size as `output` is stored data, which the decoder copies. This is the same as
    /// `Decompress` of the Compression API with `COMPRESS_ALGORITHM_LZMS | COMPRESS_RAW`.
    ///
    /// # Errors
    ///
    /// Return an error if the block is not valid LZMS data for this size, or if `output` has
    /// more than [`MAX_BLOCK_SIZE`] bytes.
    pub fn decompress_into(&mut self, input: &[u8], output: &mut [u8]) -> Result<()> {
        let size = output.len();
        if size > MAX_BLOCK_SIZE {
            return Err(Error::limit_exceeded(size as u64, MAX_BLOCK_SIZE as u64));
        }
        if input.is_empty() {
            return Err(Error::invalid_data("the compressed block is empty"));
        }
        if input.len() > size {
            return Err(Error::invalid_data(format!(
                "the compressed block has {} bytes, more than the output size {size}",
                input.len()
            )));
        }
        if input.len() == size {
            output.copy_from_slice(input);
            return Ok(());
        }
        if input.len() < 4 {
            return Err(Error::unexpected_end().in_structure("block"));
        }
        Block::new(input, size)?.run(output)?;
        x86::undo(output, &mut self.last_use);
        Ok(())
    }
}

/// Decompress one raw LZMS block, as a WIM or ESD chunk stores it.
///
/// `output_len` is the uncompressed size of the block. Use [`Decoder`] to decompress many
/// blocks.
///
/// # Errors
///
/// Return an error if the block is not valid LZMS data for this size, or if `output_len` is
/// more than [`MAX_BLOCK_SIZE`].
///
/// # Example
///
/// ```
/// // A block that is as long as its output is stored data.
/// assert_eq!(lzms::decompress(b"abc", 3)?, b"abc");
/// # Ok::<(), lzms::Error>(())
/// ```
pub fn decompress(input: &[u8], output_len: usize) -> Result<Vec<u8>> {
    if output_len > MAX_BLOCK_SIZE {
        return Err(Error::limit_exceeded(
            output_len as u64,
            MAX_BLOCK_SIZE as u64,
        ));
    }
    let mut output = vec![0; output_len];
    Decoder::new().decompress_into(input, &mut output)?;
    Ok(output)
}

/// The state of the decoder for one block.
struct Block<'a> {
    range: RangeDecoder<'a>,
    bits: BitReader<'a>,
    literal: AdaptiveCode,
    lz_offset: AdaptiveCode,
    length: AdaptiveCode,
    delta_offset: AdaptiveCode,
    delta_power: AdaptiveCode,
    main: Decision<16>,
    is_delta: Decision<32>,
    lz_repeat: Decision<64>,
    lz_index: [Decision<64>; 2],
    delta_repeat: Decision<64>,
    delta_index: [Decision<64>; 2],
    lz_recent: Recent<u32>,
    delta_recent: Recent<DeltaSource>,
}

impl<'a> Block<'a> {
    fn new(input: &'a [u8], size: usize) -> Result<Self> {
        let slots = tables::offset_slot_count(size);
        Ok(Self {
            range: RangeDecoder::new(input)?,
            bits: BitReader::new(input)?,
            literal: AdaptiveCode::new(256, 1024)?,
            lz_offset: AdaptiveCode::new(slots, 1024)?,
            length: AdaptiveCode::new(LENGTH_SLOTS, 512)?,
            delta_offset: AdaptiveCode::new(slots, 1024)?,
            delta_power: AdaptiveCode::new(8, 512)?,
            main: Decision::new(),
            is_delta: Decision::new(),
            lz_repeat: Decision::new(),
            lz_index: [Decision::new(), Decision::new()],
            delta_repeat: Decision::new(),
            delta_index: [Decision::new(), Decision::new()],
            lz_recent: Recent {
                items: [1, 2, 3, 4],
                pending: None,
            },
            delta_recent: Recent {
                items: [1, 2, 3, 4].map(|offset| DeltaSource { power: 0, offset }),
                pending: None,
            },
        })
    }

    /// Decode all items into `output`.
    fn run(mut self, output: &mut [u8]) -> Result<()> {
        let mut position = 0;
        while position < output.len() {
            if !self.main.decode(&mut self.range)? {
                // The symbol is less than 256.
                output[position] = u8::try_from(self.literal.decode(&mut self.bits)?)
                    .map_err(|_| Error::invalid_data("literal"))?;
                position += 1;
                self.lz_recent.finish(None);
                self.delta_recent.finish(None);
            } else if !self.is_delta.decode(&mut self.range)? {
                position = self.lz_match(output, position)?;
            } else {
                position = self.delta_match(output, position)?;
            }
        }
        // `cabinet.dll` rejects a block where the two streams read the same bytes.
        if self.range.consumed() >= self.bits.unread_end() + 2 {
            return Err(Error::invalid_data(
                "the range coder and the bitstream overlap",
            ));
        }
        Ok(())
    }

    /// Decode a repeat index from 0 to 2.
    fn repeat_index(range: &mut RangeDecoder<'_>, index: &mut [Decision<64>; 2]) -> Result<usize> {
        if !index[0].decode(range)? {
            Ok(0)
        } else if !index[1].decode(range)? {
            Ok(1)
        } else {
            Ok(2)
        }
    }

    /// Decode a value from a slot code and its extra bits.
    fn value<const N: usize>(
        code: &mut AdaptiveCode,
        bits: &mut BitReader<'_>,
        table: &SlotTable<N>,
    ) -> Result<u32> {
        let slot = code.decode(bits)?;
        let extra = u32::from(table.extra[slot]);
        let base = table.base[slot];
        if extra == 0 {
            return Ok(base);
        }
        Ok(base + bits.read(extra)?)
    }

    /// Decode the length of a match, and check it against the rest of the output.
    fn length(&mut self, position: usize, output_len: usize) -> Result<usize> {
        let length = Self::value(&mut self.length, &mut self.bits, &LENGTHS)?;
        let length = usize::try_from(length).unwrap_or(usize::MAX);
        if length > output_len - position {
            return Err(
                Error::invalid_data("a match goes past the end of the block")
                    .at_offset(position as u64),
            );
        }
        Ok(length)
    }

    /// Decode and copy an LZ match at `position`. Return the position after it.
    fn lz_match(&mut self, output: &mut [u8], position: usize) -> Result<usize> {
        let offset = if self.lz_repeat.decode(&mut self.range)? {
            let index = Self::repeat_index(&mut self.range, &mut self.lz_index)?;
            self.lz_recent.take(index)
        } else {
            Self::value(&mut self.lz_offset, &mut self.bits, &OFFSETS)?
        };
        let length = self.length(position, output.len())?;
        let distance = usize::try_from(offset).unwrap_or(usize::MAX);
        if distance > position {
            return Err(Error::invalid_data("an LZ match starts before the block")
                .at_offset(position as u64));
        }
        let source = position - distance;
        if distance >= length {
            output.copy_within(source..source + length, position);
        } else {
            for i in 0..length {
                output[position + i] = output[source + i];
            }
        }
        self.lz_recent.finish(Some(offset));
        self.delta_recent.finish(None);
        Ok(position + length)
    }

    /// Decode and apply a delta match at `position`. Return the position after it.
    fn delta_match(&mut self, output: &mut [u8], position: usize) -> Result<usize> {
        let source = if self.delta_repeat.decode(&mut self.range)? {
            let index = Self::repeat_index(&mut self.range, &mut self.delta_index)?;
            self.delta_recent.take(index)
        } else {
            let power = self.delta_power.decode(&mut self.bits)?;
            let offset = Self::value(&mut self.delta_offset, &mut self.bits, &OFFSETS)?;
            DeltaSource {
                // The delta power code has 8 symbols.
                power: u32::try_from(power).unwrap_or(u32::MAX),
                offset,
            }
        };
        let length = self.length(position, output.len())?;
        let span = 1u64 << source.power;
        let distance = u64::from(source.offset) << source.power;
        if distance + span > position as u64 {
            return Err(Error::invalid_data("a delta match starts before the block")
                .at_offset(position as u64));
        }
        // Both values are at most `position`, so they fit.
        let span = usize::try_from(span).unwrap_or(usize::MAX);
        let distance = usize::try_from(distance).unwrap_or(usize::MAX);
        for at in position..position + length {
            let base = output[at - span];
            let change = output[at - distance].wrapping_sub(output[at - distance - span]);
            output[at] = base.wrapping_add(change);
        }
        self.lz_recent.finish(None);
        self.delta_recent.finish(Some(source));
        Ok(position + length)
    }
}
