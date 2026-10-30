//! The buffer format of the Windows Compression API.
//!
//! Without `COMPRESS_RAW`, `Compress` writes a header and then a sequence of blocks. Each block
//! is a raw LZMS block with a small block header. The rules here are the same as in
//! `cabinet.dll`.
//!
//! The header has 24 bytes (all values are little-endian):
//!
//! | Offset | Size | Field |
//! | --- | --- | --- |
//! | 0 | 4 | Signature `0a 51 e5 c0` |
//! | 4 | 2 | Header size, at least 24 |
//! | 6 | 1 | Check: the low byte of the CRC-32 of the other header bytes |
//! | 7 | 1 | Algorithm (5 is LZMS) |
//! | 8 | 8 | Uncompressed size of all blocks |
//! | 16 | 4 | Uncompressed size of each block, at most 64 MiB |
//! | 20 | 2 | Flags: bit 0 sets variable block sizes |
//! | 22 | 2 | Reserved |
//!
//! With fixed block sizes, each block has a 4-byte compressed size and the compressed data.
//! Each block has the block size from the header, but the last block can be shorter.
//!
//! With variable block sizes, each block has a 4-byte compressed size, a 4-byte uncompressed
//! size and the compressed data. A block header with two zero sizes ends the buffer. The
//! uncompressed size in the header is not used.

use crate::decode::Decoder;
use crate::error::{Error, ErrorKind, Result};
use crate::prelude::*;

/// The first four bytes of a Compression API buffer.
pub const BUFFER_SIGNATURE: [u8; 4] = [0x0a, 0x51, 0xe5, 0xc0];

/// The default limit of [`decompress_buffer`] for the uncompressed size (1 GiB).
pub const DEFAULT_BUFFER_LIMIT: usize = 1 << 30;

/// The Compression API algorithm number of LZMS.
const ALGORITHM_LZMS: u8 = 5;

/// The minimum header size.
const HEADER_SIZE: usize = 24;

/// The largest block size in the header (64 MiB).
const MAX_BLOCK_SIZE: u32 = 1 << 26;

/// Decompress an LZMS buffer that the Compression API made without `COMPRESS_RAW`.
///
/// The uncompressed size must not be more than [`DEFAULT_BUFFER_LIMIT`].
///
/// # Errors
///
/// Return an error if the buffer header or a block is not valid, if the buffer uses an
/// algorithm other than LZMS, or if the output is larger than the limit.
pub fn decompress_buffer(input: &[u8]) -> Result<Vec<u8>> {
    decompress_buffer_with_limit(input, DEFAULT_BUFFER_LIMIT)
}

/// Decompress an LZMS buffer with a limit for the uncompressed size, in bytes.
///
/// # Errors
///
/// Return an error if the buffer header or a block is not valid, if the buffer uses an
/// algorithm other than LZMS, or if the output is larger than `limit`.
pub fn decompress_buffer_with_limit(input: &[u8], limit: usize) -> Result<Vec<u8>> {
    let header = Header::parse(input)?;
    if !header.variable && header.uncompressed > limit as u64 {
        return Err(Error::limit_exceeded(header.uncompressed, limit as u64));
    }
    let blocks = if header.variable {
        variable_blocks(input, header.size)?
    } else {
        fixed_blocks(input, &header)?
    };
    let total = blocks
        .iter()
        .try_fold(0u64, |sum, block| sum.checked_add(block.output_len as u64))
        .ok_or_else(|| Error::invalid_data("the block sizes overflow"))?;
    if total > limit as u64 {
        return Err(Error::limit_exceeded(total, limit as u64));
    }
    let mut output = Vec::new();
    let mut decoder = Decoder::new();
    for block in blocks {
        let start = output.len();
        output.resize(start + block.output_len, 0);
        let data = &input[block.data_start..block.data_start + block.data_len];
        decoder
            .decompress_into(data, &mut output[start..])
            .map_err(|e| e.at_offset(block.data_start as u64))?;
    }
    Ok(output)
}

/// The fields of a buffer header.
struct Header {
    size: usize,
    uncompressed: u64,
    block_size: u32,
    variable: bool,
}

impl Header {
    fn parse(input: &[u8]) -> Result<Self> {
        // `cabinet.dll` needs at least the header and one block size.
        if input.len() < HEADER_SIZE + 4 {
            return Err(Error::unexpected_end().in_structure("buffer header"));
        }
        let signature = [input[0], input[1], input[2], input[3]];
        if signature != BUFFER_SIGNATURE {
            return Err(ErrorKind::InvalidSignature(signature).into());
        }
        let size = usize::from(u16::from_le_bytes([input[4], input[5]]));
        if size < HEADER_SIZE {
            return Err(
                Error::invalid_data(format!("the header size {size} is too small"))
                    .in_structure("buffer header"),
            );
        }
        if input.len() < size + 4 {
            return Err(Error::unexpected_end().in_structure("buffer header"));
        }
        let flags = u16::from_le_bytes([input[20], input[21]]);
        if flags & !1 != 0 {
            return Err(Error::unsupported(format!("buffer flags {flags:#x}")));
        }
        if input[7] != ALGORITHM_LZMS {
            return Err(Error::unsupported(format!(
                "Compression API algorithm {}",
                input[7]
            )));
        }
        let block_size = u32::from_le_bytes([input[16], input[17], input[18], input[19]]);
        if block_size > MAX_BLOCK_SIZE {
            return Err(
                Error::invalid_data(format!("the block size {block_size} is too large"))
                    .in_structure("buffer header"),
            );
        }
        let check = crc32(crc32(0, &input[..6]), &input[7..size]).to_le_bytes()[0];
        if check != input[6] {
            return Err(Error::invalid_data("the header check byte is not correct")
                .in_structure("buffer header"));
        }
        let mut uncompressed = [0; 8];
        uncompressed.copy_from_slice(&input[8..16]);
        Ok(Self {
            size,
            uncompressed: u64::from_le_bytes(uncompressed),
            block_size,
            variable: flags & 1 != 0,
        })
    }
}

/// The location of one block.
struct Block {
    data_start: usize,
    data_len: usize,
    output_len: usize,
}

/// Read a little-endian `u32` at `offset`, if the input has it.
fn u32_at(input: &[u8], offset: usize) -> Option<u32> {
    let bytes = input.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Find the data of a block with a header of `header_len` bytes at `offset`.
fn block_data(input: &[u8], offset: usize, header_len: usize) -> Result<(usize, usize)> {
    let end_of_input = || Error::unexpected_end().at_offset(offset as u64);
    let compressed = u32_at(input, offset).ok_or_else(end_of_input)?;
    if compressed == 0 {
        return Err(Error::invalid_data("a block has no data").at_offset(offset as u64));
    }
    let start = offset + header_len;
    let len = usize::try_from(compressed).map_err(|_| end_of_input())?;
    match start.checked_add(len) {
        Some(end) if end <= input.len() => Ok((start, len)),
        _ => Err(end_of_input()),
    }
}

/// List the blocks of a buffer with fixed block sizes.
fn fixed_blocks(input: &[u8], header: &Header) -> Result<Vec<Block>> {
    let total = header.uncompressed;
    if total == 0 {
        return Err(
            Error::invalid_data("the uncompressed size is zero").in_structure("buffer header")
        );
    }
    // A block size of less than 32 KiB is valid only for one block of that size.
    if header.block_size < 0x8000 && total != u64::from(header.block_size) {
        return Err(
            Error::invalid_data("the block size is too small").in_structure("buffer header")
        );
    }
    let block_size = u64::from(header.block_size);
    let mut blocks = Vec::new();
    let mut offset = header.size;
    let mut done = 0u64;
    while done < total {
        let (data_start, data_len) = block_data(input, offset, 4)?;
        let output_len = usize::try_from(block_size.min(total - done))
            .map_err(|_| Error::limit_exceeded(total, usize::MAX as u64))?;
        blocks.push(Block {
            data_start,
            data_len,
            output_len,
        });
        done += output_len as u64;
        offset = data_start + data_len;
    }
    Ok(blocks)
}

/// List the blocks of a buffer with variable block sizes.
fn variable_blocks(input: &[u8], mut offset: usize) -> Result<Vec<Block>> {
    let mut blocks = Vec::new();
    loop {
        let end_of_input = || Error::unexpected_end().at_offset(offset as u64);
        let compressed = u32_at(input, offset).ok_or_else(end_of_input)?;
        let uncompressed = u32_at(input, offset + 4).ok_or_else(end_of_input)?;
        if compressed == 0 && uncompressed == 0 {
            return Ok(blocks);
        }
        if uncompressed == 0 {
            return Err(Error::invalid_data("a block has no output").at_offset(offset as u64));
        }
        let (data_start, data_len) = block_data(input, offset, 8)?;
        blocks.push(Block {
            data_start,
            data_len,
            output_len: usize::try_from(uncompressed).map_err(|_| end_of_input())?,
        });
        offset = data_start + data_len;
    }
}

/// Continue a CRC-32 (IEEE, the same as `RtlComputeCrc32`) with `data`.
fn crc32(crc: u32, data: &[u8]) -> u32 {
    let mut crc = !crc;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(crc32(0, b"1234"), b"56789"), 0xCBF4_3926);
    }
}
