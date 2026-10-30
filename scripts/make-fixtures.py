"""Make the LZMS test fixtures with the Windows Compression API (cabinet.dll).

Each case has an input that a fixed-seed generator makes. The tests in tests/fixtures.rs make
the same bytes with the same generator, so the repository keeps only the compressed data and a
manifest. Run this script on Windows with PowerShell 7 (pwsh):

    python scripts/make-fixtures.py

For each case, the script compresses the input with cabinet.dll, decompresses the result with
cabinet.dll, and makes sure that the two are equal.
"""

import hashlib
import json
import pathlib
import struct
import subprocess
import tempfile
import zlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests" / "fixtures"
ORACLE = ROOT / "scripts" / "lzms-oracle.ps1"
MASK = (1 << 64) - 1
MASK32 = (1 << 32) - 1

WORDS = [
    b"the", b"range", b"coder", b"huffman", b"symbol", b"offset", b"length", b"delta",
    b"match", b"literal", b"chunk", b"block", b"window", b"stream", b"filter", b"table",
]

FILLER = [b"\x55", b"\x48\x89\xe5", b"\x90", b"\xc3", b"\x31\xc0", b"\x48\x83\xec\x28"]


class Rng:
    """SplitMix64. tests/fixtures.rs has the same generator."""

    def __init__(self, seed):
        self.state = seed & MASK

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    def below(self, n):
        return self.next() % n


def random_bytes(seed, n):
    rng = Rng(seed)
    return bytes(rng.next() & 0xFF for _ in range(n))


def text(seed, n):
    rng = Rng(seed)
    out = bytearray()
    while len(out) < n:
        out += WORDS[rng.below(len(WORDS))]
        out += b" " if rng.below(8) else b"\n"
    return bytes(out[:n])


def repeat(pattern, n):
    return (pattern * (n // len(pattern) + 1))[:n]


def x86(seed, n):
    """Make bytes like x86 code: calls and RIP-relative loads to a few targets, and filler."""
    rng = Rng(seed)
    targets = [rng.below(1 << 20) for _ in range(64)]
    out = bytearray()
    while len(out) < n:
        kind = rng.below(10)
        target = targets[rng.below(len(targets))]
        pos = len(out)
        if kind < 3:
            out += b"\xe8" + ((target - pos - 5) & MASK32).to_bytes(4, "little")
        elif kind == 3:
            out += b"\x48\x8b\x05" + ((target - pos - 7) & MASK32).to_bytes(4, "little")
        elif kind == 4:
            out += b"\x48\x8d\x0d" + ((target - pos - 7) & MASK32).to_bytes(4, "little")
        elif kind == 5:
            out += b"\x4c\x8d\x05" + ((target - pos - 7) & MASK32).to_bytes(4, "little")
        elif kind == 6:
            out += b"\xff\x15" + ((target - pos - 6) & MASK32).to_bytes(4, "little")
        elif kind == 7:
            out += b"\xe9" + (rng.next() & MASK32).to_bytes(4, "little")
        elif kind == 8:
            out += b"\xf0\x83\x05" + ((target - pos - 8) & MASK32).to_bytes(4, "little") + b"\x01"
        else:
            out += FILLER[rng.below(len(FILLER))]
    return bytes(out[:n])


def image(seed, n):
    """Make rows of 128 RGB pixels with smooth gradients and a little noise."""
    rng = Rng(seed)
    out = bytearray()
    row = 0
    while len(out) < n:
        for x in range(128):
            for channel in range(3):
                value = x * (channel + 1) + row * 2 + rng.below(3)
                out.append(value & 0xFF)
        row += 1
    return bytes(out[:n])


def samples(seed, n):
    """Make 16-bit little-endian samples of a signal with a slowly changing slope."""
    rng = Rng(seed)
    out = bytearray()
    value = 0
    step = 3
    while len(out) < n:
        if rng.below(64) == 0:
            step = rng.below(17) - 8
        value = (value + step) & 0xFFFF
        out += value.to_bytes(2, "little")
    return bytes(out[:n])


def far(seed, n):
    """Put copies of one random 64 KiB block at growing distances, with zeros between."""
    rng = Rng(seed)
    block = random_bytes(seed, 65536)
    out = bytearray(n)
    pos = 0
    gap = 65536
    while pos + len(block) <= n:
        copy = bytearray(block)
        for _ in range(4):
            copy[rng.below(len(copy))] = rng.next() & 0xFF
        out[pos:pos + len(copy)] = copy
        pos += len(copy) + gap
        gap *= 2
    return bytes(out)


def mixed(seed, n):
    """Join parts of each kind, so that each code rebuilds many times."""
    part = n // 8
    data = (text(seed, 2 * part) + x86(seed + 1, 2 * part) + random_bytes(seed + 2, part)
            + image(seed + 3, part) + samples(seed + 4, part) + repeat(b"0123456789", part))
    return (data + text(seed + 5, n))[:n]


# name -> (kind, input, options). Keep tests/fixtures.rs in sync.
def cases():
    return {
        "stored-1": ("raw", random_bytes(1, 1), {}),
        "text-17": ("raw", text(2, 17), {}),
        "text-18": ("raw", text(3, 18), {}),
        "text-100": ("raw", text(4, 100), {}),
        "text-65536": ("raw", text(5, 65536), {}),
        "text-131071": ("raw", text(6, 131071), {}),
        "text-131072": ("raw", text(7, 131072), {}),
        "text-131073": ("raw", text(8, 131073), {}),
        "random-4096": ("raw", random_bytes(9, 4096), {}),
        "random-text-200000": ("raw", random_bytes(10, 100000) + text(11, 100000), {}),
        "zeros-1048576": ("raw", bytes(1 << 20), {}),
        "repeat-300000": ("raw", repeat(b"abcdefgh", 300000), {}),
        "x86-262144": ("raw", x86(12, 262144), {}),
        "image-262144": ("raw", image(13, 262144), {}),
        "samples-200000": ("raw", samples(14, 200000), {}),
        "mixed-2097152": ("raw", mixed(15, 2 << 20), {}),
        "far-8388608": ("raw", far(16, 8 << 20), {}),
        "far-67108864": ("raw", far(17, 64 << 20), {}),
        "buffer-text-300000": ("buffer", text(18, 300000), {}),
        "buffer-random-100000": ("buffer", random_bytes(19, 100000), {}),
        "buffer-mixed-3145728": ("buffer", mixed(20, 3 << 20), {"block_size": 1 << 20}),
        "buffer-text-2400": ("buffer", text(21, 2400), {}),
        "buffer-variable": ("variable", text(22, 65536) + x86(23, 100000) + b"z", {}),
    }


def oracle(*args):
    result = subprocess.run(["pwsh", "-NoProfile", "-File", str(ORACLE), *map(str, args)],
                            check=True, capture_output=True, text=True)
    return result.stdout


def raw_block(tmp, data):
    """Compress one raw block. A block of more than 64 KiB needs a larger block size."""
    src, out = tmp / "in", tmp / "out"
    src.write_bytes(data)
    args = ["compress", "-In", src, "-Out", out, "-Raw"]
    if len(data) > 65536:
        args += ["-BlockSize", len(data)]
    oracle(*args)
    return out.read_bytes()


def variable_buffer(tmp, parts):
    """Frame raw blocks as a buffer with variable block sizes (flag bit 0)."""
    header = bytearray(struct.pack("<IHBBQIHH", 0xC0E5510A, 24, 0, 5, 0, 0, 1, 0))
    header[6] = zlib.crc32(bytes(header[:6]) + bytes(header[7:])) & 0xFF
    body = bytearray()
    for part in parts:
        block = raw_block(tmp, part)
        body += struct.pack("<II", len(block), len(part)) + block
    return bytes(header) + bytes(body) + struct.pack("<II", 0, 0)


def main():
    FIXTURES.mkdir(parents=True, exist_ok=True)
    manifest = {}
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        for name, (kind, data, options) in cases().items():
            src, out, check = tmp / "case", FIXTURES / f"{name}.lzms", tmp / "check"
            src.write_bytes(data)
            if kind == "raw":
                out.write_bytes(raw_block(tmp, data))
                oracle("decompress", "-In", out, "-Out", check, "-Raw", "-Size", len(data))
            elif kind == "buffer":
                args = ["compress", "-In", src, "-Out", out]
                if "block_size" in options:
                    args += ["-BlockSize", options["block_size"]]
                oracle(*args)
                oracle("decompress", "-In", out, "-Out", check)
            else:
                parts = [data[:65536], data[65536:-1], data[-1:]]
                out.write_bytes(variable_buffer(tmp, parts))
                oracle("decompress", "-In", out, "-Out", check)
            assert check.read_bytes() == data, name
            manifest[name] = {
                "kind": kind,
                "size": len(data),
                "input_sha256": hashlib.sha256(data).hexdigest(),
                "compressed_bytes": out.stat().st_size,
            }
            print(f"{name}: {len(data)} -> {out.stat().st_size} bytes")
        # cabinet.dll must reject a buffer with a wrong header check byte.
        bad = bytearray((FIXTURES / "buffer-text-2400.lzms").read_bytes())
        bad[6] ^= 1
        (tmp / "bad").write_bytes(bytes(bad))
        try:
            oracle("decompress", "-In", tmp / "bad", "-Out", tmp / "check", "-Size", 2400)
        except subprocess.CalledProcessError:
            pass
        else:
            raise AssertionError("cabinet.dll accepted a wrong header check byte")
    (FIXTURES / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
