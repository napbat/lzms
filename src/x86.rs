//! The x86 filter.
//!
//! Before compression, the encoder changes the relative address in some x86 instructions to
//! an absolute address (the relative address plus the position). This makes repeated calls to
//! the same target equal. After decompression, [`undo`] changes the addresses back.
//!
//! The filter only changes an instruction near a position that looks like x86 code. A position
//! looks like x86 code if an instruction there has the same 16-bit target as an instruction
//! less than 64 KiB before it. The rules are the same as `OpcodeTransformDecoder` in
//! `cabinet.dll`.

/// The number of entries in the table of the last position of each 16-bit target.
pub(crate) const TARGETS: usize = 1 << 16;

/// The filter does not change the last 16 bytes, and does nothing for 17 bytes or fewer.
const TAIL: usize = 16;

/// Undo the x86 filter on a decompressed block. `data` has at most 1 GiB.
///
/// `last_use` is scratch space for the last position of each 16-bit target.
pub(crate) fn undo(data: &mut [u8], last_use: &mut [u32]) {
    if data.len() <= TAIL + 1 {
        return;
    }
    let end = data.len() - TAIL;
    // The start values put each target and the last x86 position far before position 0.
    last_use.fill(0xFFFF_0000);
    let mut last_x86 = 0u32.wrapping_sub(1024);
    let mut i = 1;
    while i < end {
        // The position fits in 32 bits, because a block has at most 1 GiB.
        let position = u32::try_from(i).unwrap_or(u32::MAX);
        let (prefix, window) = match (data[i], data[i + 1], data[i + 2]) {
            // mov reg, [rip + rel32] (48 8B 05 or 48 8B 0D), and
            // lock add dword [rip + rel32], imm8 (F0 83 05).
            (0x48, 0x8B, 0x05 | 0x0D) | (0xF0, 0x83, 0x05) => (3, 1024),
            // lea reg, [rip + rel32] (48 8D or 4C 8D, ModRM r/m = 101).
            (0x48 | 0x4C, 0x8D, modrm) if modrm & 7 == 5 => (3, 1024),
            // call rel32. Translate only in a smaller window after x86 code.
            (0xE8, _, _) => (1, 512),
            // call [rip + rel32].
            (0xFF, 0x15, _) => (2, 1024),
            // jmp rel32: skip the operand.
            (0xE9, _, _) => {
                i += 5;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        };
        let field = i + prefix;
        if position.wrapping_sub(last_x86) < window {
            let value = u32::from_le_bytes([
                data[field],
                data[field + 1],
                data[field + 2],
                data[field + 3],
            ]);
            data[field..field + 4].copy_from_slice(&value.wrapping_sub(position).to_le_bytes());
        }
        let low = position.to_le_bytes();
        let target = u16::from_le_bytes([data[field], data[field + 1]])
            .wrapping_add(u16::from_le_bytes([low[0], low[1]]));
        let next = i + prefix + 3;
        let next_position = u32::try_from(next).unwrap_or(u32::MAX);
        let slot = &mut last_use[usize::from(target)];
        if next_position.wrapping_sub(*slot) < 1 << 16 {
            last_x86 = next_position;
        }
        *slot = next_position;
        i = next + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    #[test]
    fn a_short_block_does_not_change() {
        let mut data = vec![0xE8; 17];
        let mut table = vec![0; TARGETS];
        undo(&mut data, &mut table);
        assert_eq!(data, vec![0xE8; 17]);
    }

    #[test]
    fn a_call_after_x86_code_changes_back() {
        // Two calls to the target 0x1000 (relative address plus position) mark x86 code. The
        // third call is then in the window, so its absolute address changes back.
        let mut data = vec![0x90u8; 64];
        for (at, value) in [(1u32, 0x1000 - 1), (8, 0x1000 - 8), (20, 0x1000)] {
            let at = at as usize;
            data[at] = 0xE8;
            data[at + 1..at + 5].copy_from_slice(&u32::to_le_bytes(value));
        }
        let mut table = vec![0; TARGETS];
        undo(&mut data, &mut table);
        assert_eq!(&data[2..6], &(0x1000u32 - 1).to_le_bytes());
        assert_eq!(&data[9..13], &(0x1000u32 - 8).to_le_bytes());
        assert_eq!(&data[21..25], &(0x1000u32 - 20).to_le_bytes());
    }
}
