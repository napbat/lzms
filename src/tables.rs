//! Slot tables for match offsets and match lengths.
//!
//! A slot gives a base value and a count of extra bits. The value is the base plus the extra
//! bits from the bitstream. The tables are the same as `MatchOffsetDistributionTable` and
//! `MatchLengthDistributionTable` in `cabinet.dll`. Each run below gives a count of slots and
//! the extra bits of each slot in the run. The base of a slot is the base of the slot before
//! it, plus two to the power of the extra bits of the slot before it.

/// The number of offset slots for the largest block.
pub(crate) const OFFSET_SLOTS: usize = 799;

/// The number of length slots.
pub(crate) const LENGTH_SLOTS: usize = 54;

/// The slot runs of the offset table: `(slot count, extra bits)`.
const OFFSET_RUNS: [(usize, u8); 21] = [
    (8, 0),
    (9, 2),
    (7, 3),
    (10, 4),
    (15, 5),
    (15, 6),
    (20, 7),
    (20, 8),
    (30, 9),
    (33, 10),
    (40, 11),
    (42, 12),
    (45, 13),
    (60, 14),
    (73, 15),
    (80, 16),
    (85, 17),
    (95, 18),
    (105, 19),
    (6, 20),
    (1, 30),
];

/// The slot runs of the length table: `(slot count, extra bits)`.
const LENGTH_RUNS: [(usize, u8); 13] = [
    (26, 0),
    (4, 1),
    (6, 2),
    (4, 3),
    (5, 4),
    (2, 5),
    (1, 6),
    (1, 7),
    (1, 8),
    (1, 9),
    (1, 10),
    (1, 16),
    (1, 30),
];

/// A table of slot bases and extra bit counts.
pub(crate) struct SlotTable<const N: usize> {
    /// The base value of each slot.
    pub(crate) base: [u32; N],
    /// The number of extra bits of each slot.
    pub(crate) extra: [u8; N],
}

/// Expand slot runs into a table. The first slot has the base 1.
const fn expand<const N: usize>(runs: &[(usize, u8)]) -> SlotTable<N> {
    let mut table = SlotTable {
        base: [0; N],
        extra: [0; N],
    };
    let mut slot = 0;
    let mut base = 1u32;
    let mut run = 0;
    while run < runs.len() {
        let (count, bits) = runs[run];
        let mut i = 0;
        while i < count {
            table.base[slot] = base;
            table.extra[slot] = bits;
            base += 1 << bits;
            slot += 1;
            i += 1;
        }
        run += 1;
    }
    assert!(slot == N, "the runs must fill the table");
    table
}

/// The offset slots of the LZ offsets and the delta offsets.
pub(crate) static OFFSETS: SlotTable<OFFSET_SLOTS> = expand(&OFFSET_RUNS);

/// The length slots of the LZ matches and the delta matches.
pub(crate) static LENGTHS: SlotTable<LENGTH_SLOTS> = expand(&LENGTH_RUNS);

/// Return the number of offset slots for a block of `size` bytes.
///
/// This is the number of slots with a base that is less than `size`, but at least one.
pub(crate) fn offset_slot_count(size: usize) -> usize {
    let count = OFFSETS
        .base
        .iter()
        .take_while(|&&base| usize::try_from(base).is_ok_and(|base| base < size))
        .count();
    count.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_table_matches_cabinet_dll() {
        assert_eq!(
            &OFFSETS.base[..12],
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 13, 17, 21]
        );
        assert_eq!(OFFSETS.base[17], 45);
        assert_eq!(OFFSETS.base[18], 53);
        assert_eq!(OFFSETS.base[OFFSET_SLOTS - 1], 106_685_605);
        assert_eq!(OFFSETS.extra[OFFSET_SLOTS - 1], 30);
    }

    #[test]
    fn the_length_table_matches_cabinet_dll() {
        assert_eq!(LENGTHS.base[26], 27);
        assert_eq!(LENGTHS.base[27], 29);
        assert_eq!(
            &LENGTHS.base[46..],
            &[203, 235, 299, 427, 683, 1195, 2219, 67755]
        );
        assert_eq!(LENGTHS.extra[52], 16);
        assert_eq!(LENGTHS.extra[53], 30);
    }

    #[test]
    fn the_slot_count_covers_the_largest_offset() {
        assert_eq!(offset_slot_count(1), 1);
        assert_eq!(offset_slot_count(3), 2);
        assert_eq!(offset_slot_count(4), 3);
        assert_eq!(offset_slot_count(10), 9);
        assert_eq!(offset_slot_count(1 << 30), OFFSET_SLOTS);
    }
}
