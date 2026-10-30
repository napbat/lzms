//! Decompress the data that `cabinet.dll` compressed, and compare it with the input.
//!
//! `scripts/make-fixtures.py` made the files in `tests/fixtures/` with the Compression API.
//! The generator in this file makes the same inputs from the same seeds.

use lzms::{Decoder, ErrorKind};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

/// The `SplitMix64` generator, the same as in `scripts/make-fixtures.py`.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn index(&mut self, n: usize) -> usize {
        usize::try_from(self.below(n as u64)).unwrap()
    }

    fn byte(&mut self) -> u8 {
        self.next().to_le_bytes()[0]
    }
}

const WORDS: [&[u8]; 16] = [
    b"the", b"range", b"coder", b"huffman", b"symbol", b"offset", b"length", b"delta", b"match",
    b"literal", b"chunk", b"block", b"window", b"stream", b"filter", b"table",
];

const FILLER: [&[u8]; 6] = [
    b"\x55",
    b"\x48\x89\xe5",
    b"\x90",
    b"\xc3",
    b"\x31\xc0",
    b"\x48\x83\xec\x28",
];

fn random_bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut rng = Rng(seed);
    (0..n).map(|_| rng.byte()).collect()
}

fn text(seed: u64, n: usize) -> Vec<u8> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(n + 16);
    while out.len() < n {
        out.extend_from_slice(WORDS[rng.index(WORDS.len())]);
        out.push(if rng.below(8) == 0 { b'\n' } else { b' ' });
    }
    out.truncate(n);
    out
}

fn repeat(pattern: &[u8], n: usize) -> Vec<u8> {
    pattern.iter().copied().cycle().take(n).collect()
}

/// Return `target - pos - size` as four little-endian bytes.
fn relative(target: u64, pos: usize, size: u64) -> [u8; 4] {
    let value = target.wrapping_sub(pos as u64).wrapping_sub(size);
    let bytes = value.to_le_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

fn x86(seed: u64, n: usize) -> Vec<u8> {
    let mut rng = Rng(seed);
    let targets: Vec<u64> = (0..64).map(|_| rng.below(1 << 20)).collect();
    let mut out = Vec::with_capacity(n + 16);
    while out.len() < n {
        let kind = rng.below(10);
        let target = targets[rng.index(targets.len())];
        let pos = out.len();
        match kind {
            0..=2 => {
                out.push(0xe8);
                out.extend_from_slice(&relative(target, pos, 5));
            }
            3 => {
                out.extend_from_slice(b"\x48\x8b\x05");
                out.extend_from_slice(&relative(target, pos, 7));
            }
            4 => {
                out.extend_from_slice(b"\x48\x8d\x0d");
                out.extend_from_slice(&relative(target, pos, 7));
            }
            5 => {
                out.extend_from_slice(b"\x4c\x8d\x05");
                out.extend_from_slice(&relative(target, pos, 7));
            }
            6 => {
                out.extend_from_slice(b"\xff\x15");
                out.extend_from_slice(&relative(target, pos, 6));
            }
            7 => {
                out.push(0xe9);
                out.extend_from_slice(&rng.next().to_le_bytes()[..4]);
            }
            8 => {
                out.extend_from_slice(b"\xf0\x83\x05");
                out.extend_from_slice(&relative(target, pos, 8));
                out.push(1);
            }
            _ => out.extend_from_slice(FILLER[rng.index(FILLER.len())]),
        }
    }
    out.truncate(n);
    out
}

fn image(seed: u64, n: usize) -> Vec<u8> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(n + 384);
    let mut row = 0u64;
    while out.len() < n {
        for x in 0..128u64 {
            for channel in 0..3u64 {
                let value = x * (channel + 1) + row * 2 + rng.below(3);
                out.push(value.to_le_bytes()[0]);
            }
        }
        row += 1;
    }
    out.truncate(n);
    out
}

fn samples(seed: u64, n: usize) -> Vec<u8> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(n + 2);
    let mut value = 0u16;
    let mut step = 3i64;
    while out.len() < n {
        if rng.below(64) == 0 {
            step = i64::try_from(rng.below(17)).unwrap() - 8;
        }
        value = value.wrapping_add_signed(i16::try_from(step).unwrap());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.truncate(n);
    out
}

fn far(seed: u64, n: usize) -> Vec<u8> {
    let mut rng = Rng(seed);
    let block = random_bytes(seed, 65536);
    let mut out = vec![0; n];
    let mut pos = 0;
    let mut gap = 65536;
    while pos + block.len() <= n {
        let mut copy = block.clone();
        for _ in 0..4 {
            // Python evaluates the value before the index in `copy[index] = value`.
            let value = rng.byte();
            let at = rng.index(copy.len());
            copy[at] = value;
        }
        out[pos..pos + copy.len()].copy_from_slice(&copy);
        pos += copy.len() + gap;
        gap *= 2;
    }
    out
}

fn mixed(seed: u64, n: usize) -> Vec<u8> {
    let part = n / 8;
    let mut data = text(seed, 2 * part);
    data.extend(x86(seed + 1, 2 * part));
    data.extend(random_bytes(seed + 2, part));
    data.extend(image(seed + 3, part));
    data.extend(samples(seed + 4, part));
    data.extend(repeat(b"0123456789", part));
    data.extend(text(seed + 5, n));
    data.truncate(n);
    data
}

/// Return the input of a case.
fn case(name: &str) -> Vec<u8> {
    match name {
        "stored-1" => random_bytes(1, 1),
        "repeat-17" => repeat(b"ab", 17),
        "repeat-18" => repeat(b"ab", 18),
        "calls-40" => repeat(b"\x90\xe8\x10\x00\x00\x00", 40),
        "text-100" => text(4, 100),
        "text-65536" => text(5, 65536),
        "text-131071" => text(6, 131_071),
        "text-131072" => text(7, 131_072),
        "text-131073" => text(8, 131_073),
        "random-4096" => random_bytes(9, 4096),
        "random-text-200000" => {
            let mut data = random_bytes(10, 100_000);
            data.extend(text(11, 100_000));
            data
        }
        "zeros-1048576" => vec![0; 1 << 20],
        "repeat-300000" => repeat(b"abcdefgh", 300_000),
        "x86-262144" => x86(12, 262_144),
        "image-262144" => image(13, 262_144),
        "samples-200000" => samples(14, 200_000),
        "mixed-2097152" => mixed(15, 2 << 20),
        "far-8388608" => far(16, 8 << 20),
        "far-67108864" => far(17, 64 << 20),
        "buffer-text-300000" => text(18, 300_000),
        "buffer-random-100000" => random_bytes(19, 100_000),
        "buffer-mixed-3145728" => mixed(20, 3 << 20),
        "buffer-text-2400" => text(21, 2400),
        "buffer-variable" => {
            let mut data = text(22, 65536);
            data.extend(x86(23, 100_000));
            data.push(b'z');
            data
        }
        _ => panic!("unknown case {name}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        write!(s, "{b:02x}").unwrap();
        s
    })
}

fn manifest() -> serde_json::Map<String, Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifest.json");
    match serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap() {
        Value::Object(map) => map,
        _ => panic!("the manifest is not an object"),
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.lzms"));
    std::fs::read(path).unwrap()
}

/// Decompress a fixture with the API for its kind.
fn decompress(kind: &str, data: &[u8], size: usize) -> lzms::Result<Vec<u8>> {
    if kind == "raw" {
        lzms::decompress(data, size)
    } else {
        lzms::decompress_buffer(data)
    }
}

#[test]
fn the_generator_makes_the_fixture_inputs() {
    for (name, entry) in manifest() {
        let input = case(&name);
        assert_eq!(
            input.len() as u64,
            entry["size"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            hex(&Sha256::digest(&input)),
            entry["input_sha256"].as_str().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn each_fixture_gives_the_output_of_cabinet_dll() {
    let mut decoder = Decoder::new();
    for (name, entry) in manifest() {
        let input = case(&name);
        let data = fixture(&name);
        assert_eq!(
            data.len() as u64,
            entry["compressed_bytes"].as_u64().unwrap()
        );
        let kind = entry["kind"].as_str().unwrap();
        let output = decompress(kind, &data, input.len()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(output == input, "{name}: the output is not the input");
        if kind == "raw" {
            let mut again = vec![0; input.len()];
            decoder.decompress_into(&data, &mut again).unwrap();
            assert!(
                again == input,
                "{name}: the reused decoder gives other output"
            );
        }
    }
}

#[test]
fn every_truncated_block_is_an_error() {
    for name in ["text-100", "x86-262144", "image-262144"] {
        let data = fixture(name);
        let size = case(name).len();
        let step = (data.len() / 200).max(1);
        for len in (0..data.len()).step_by(step) {
            assert!(
                lzms::decompress(&data[..len], size).is_err(),
                "{name}: {len} bytes"
            );
        }
    }
}

#[test]
fn a_wrong_output_size_does_not_panic() {
    let data = fixture("text-65536");
    for size in [5, 1000, 65535, 65537, 200_000] {
        let _ = lzms::decompress(&data, size);
    }
}

#[test]
fn a_wrong_header_check_byte_is_an_error() {
    // `scripts/make-fixtures.py` makes sure that `cabinet.dll` also rejects this buffer.
    let mut data = fixture("buffer-text-2400");
    data[6] ^= 1;
    let error = lzms::decompress_buffer(&data).unwrap_err();
    assert!(matches!(error.kind, ErrorKind::InvalidData(_)), "{error}");
}

#[test]
fn the_buffer_limit_rejects_a_large_output() {
    let data = fixture("buffer-mixed-3145728");
    let error = lzms::decompress_buffer_with_limit(&data, 1 << 20).unwrap_err();
    assert_eq!(
        error.kind,
        ErrorKind::LimitExceeded {
            needed: 3 << 20,
            limit: 1 << 20
        }
    );
}

#[test]
fn a_raw_block_is_not_a_buffer() {
    let error = lzms::decompress_buffer(&fixture("text-65536")).unwrap_err();
    assert!(
        matches!(error.kind, ErrorKind::InvalidSignature(_)),
        "{error}"
    );
}
