//! # lzms
//!
//! A `no_std` decoder for Microsoft LZMS compressed data. It works on each platform, with no
//! Windows API.
//!
//! LZMS is the compression format of WIM files with LZMS chunks, of ESD files, and of
//! `COMPRESS_ALGORITHM_LZMS` in the Windows Compression API (`cabinet.dll`). The output is the
//! same as the output of `Decompress` in `cabinet.dll`.
//!
//! ## Support
//!
//! - Raw LZMS blocks, as WIM and ESD chunks store them (`COMPRESS_RAW`), with [`decompress`]
//!   and [`Decoder`].
//! - Compression API buffers with their header and block headers, with
//!   [`decompress_buffer`].
//!
//! The crate does not compress.
//!
//! ## Example
//!
//! ```no_run
//! # let (chunk, chunk_size): (&[u8], usize) = (&[], 0);
//! let data = lzms::decompress(chunk, chunk_size)?;
//! # Ok::<(), lzms::Error>(())
//! ```

#![no_std]
extern crate alloc;

/// Internal prelude. Each source file uses `use crate::prelude::*;` to get the `alloc` types
/// and macros that the code needs.
pub(crate) mod prelude {
    pub use alloc::format;
    pub use alloc::string::String;
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

mod bits;
mod buffer;
mod decode;
pub mod error;
mod huffman;
mod range;
mod tables;
mod x86;

pub use buffer::{
    BUFFER_SIGNATURE, DEFAULT_BUFFER_LIMIT, decompress_buffer, decompress_buffer_with_limit,
};
pub use decode::{Decoder, MAX_BLOCK_SIZE, decompress};
pub use error::{Error, ErrorContext, ErrorKind, Result};
