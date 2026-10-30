# lzms

lzms decompresses Microsoft LZMS data. It is portable Rust: it does not use `cabinet.dll` or
other Windows APIs, and it works with `no_std + alloc`.

WIM files with LZMS chunks, ESD files and the Windows Compression API
(`COMPRESS_ALGORITHM_LZMS`) use LZMS. lzms gives the same output as `Decompress` in
`cabinet.dll` on each platform.

## Status

| Feature | Status |
| --- | --- |
| Raw LZMS blocks (`COMPRESS_RAW`, WIM and ESD chunks), up to 1 GiB | Supported |
| Compression API buffers, fixed and variable block sizes | Supported |
| x86 filter | Supported |
| Compression | Not supported |

## Usage

```rust,no_run
# let (chunk, chunk_size, buffer): (&[u8], usize, &[u8]) = (&[], 0, &[]);
// A raw block, for example a WIM chunk. The caller knows the uncompressed size.
let data = lzms::decompress(chunk, chunk_size)?;

// A Compression API buffer, with its header.
let data = lzms::decompress_buffer(buffer)?;

// Many blocks with one decoder, without a new x86 filter table for each block.
let mut decoder = lzms::Decoder::new();
let mut output = vec![0; chunk_size];
decoder.decompress_into(chunk, &mut output)?;
# Ok::<(), lzms::Error>(())
```

`decompress_buffer` limits the output to 1 GiB. `decompress_buffer_with_limit` sets a different
limit.

## Format notes

A raw block has two streams in one buffer. A range decoder reads 16-bit little-endian words
from the start. It decodes the binary decisions with adaptive probabilities. A bitstream reads
16-bit little-endian words from the end, most significant bit first. It gives the Huffman
symbols and the extra bits of offsets and lengths.

Each item is a literal, an LZ match or a delta match. A match has an explicit source or one of
three recent sources. The queue of recent sources adds the source of a match after the next
item. Five adaptive Huffman codes (literals, LZ offsets, lengths, delta offsets and delta
powers) rebuild from their symbol counts after each 512 or 1024 symbols. After the last item,
the decoder undoes an x86 filter that changed relative call and load addresses to absolute
addresses.

A block that is as long as its output is stored data.

A Compression API buffer starts with `0a 51 e5 c0` and a 24-byte header. The header has a
CRC-32 check byte, the algorithm (5), the uncompressed size and the block size. A compressed
size comes before each block. With the variable flag, an uncompressed size also comes before
each block, and two zero sizes end the buffer.

## Tests

`scripts/make-fixtures.py` makes the fixtures with `cabinet.dll` on Windows (PowerShell 7 runs
`scripts/lzms-oracle.ps1`). A fixed-seed generator makes the inputs, so the repository keeps
only the compressed data. The tests compare the output of lzms with the input. The script
makes sure that `cabinet.dll` gives the same input back.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
