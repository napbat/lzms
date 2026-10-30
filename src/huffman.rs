//! Adaptive Huffman codes.
//!
//! Each code counts the symbols that it decodes. After a fixed number of symbols (the rebuild
//! period), the decoder makes a new code from the counts, and then halves the counts. The
//! encoder does the same, so no code lengths are in the data. The construction must give the
//! same code lengths as `cabinet.dll`:
//!
//! 1. Sort the symbols by count, with a stable sort.
//! 2. Merge the two smallest nodes. On equal counts, take a leaf before an internal node.
//! 3. If the first leaf is deeper than 15 bits, halve each count (round up) and merge again.
//! 4. Give canonical codewords in order of length, then symbol.

use crate::bits::BitReader;
use crate::error::{Error, Result};
use crate::prelude::*;

/// The maximum length of a codeword, in bits.
const MAX_LENGTH: u32 = 15;

/// The number of bits that the lookup table decodes in one step.
const TABLE_BITS: u32 = 10;

/// Buffers for the tree construction, kept to prevent an allocation at each rebuild.
struct Scratch {
    /// The symbols in order of count.
    order: Vec<usize>,
    /// The count of each leaf, in order of count.
    leaf_count: Vec<u32>,
    /// The parent node of each leaf.
    leaf_parent: Vec<usize>,
    /// The depth of each leaf.
    leaf_depth: Vec<u32>,
    /// The count of each internal node, in order of construction.
    node_count: Vec<u32>,
    /// The parent node of each internal node.
    node_parent: Vec<usize>,
    /// The depth of each internal node.
    node_depth: Vec<u32>,
}

/// An adaptive Huffman code.
pub(crate) struct AdaptiveCode {
    /// The number of symbols between two rebuilds.
    period: u32,
    /// The number of symbols since the last rebuild.
    decoded: u32,
    /// The count of each symbol.
    counts: Vec<u32>,
    /// The codeword length of each symbol.
    lengths: Vec<u32>,
    /// The lookup table: `symbol << 4 | length`, or 0 for a codeword of more than
    /// `TABLE_BITS` bits.
    table: Vec<u16>,
    /// The symbols in order of codeword length, then symbol.
    sorted: Vec<usize>,
    /// The number of codewords of each length.
    per_length: [u32; MAX_LENGTH as usize + 1],
    scratch: Scratch,
}

impl AdaptiveCode {
    /// Make a code for `symbols` symbols, each with the count 1. `symbols` is at least 2.
    pub(crate) fn new(symbols: usize, period: u32) -> Result<Self> {
        if symbols < 2 {
            return Err(Error::invalid_data("a Huffman code needs two symbols"));
        }
        let internal = symbols - 1;
        let mut code = Self {
            period,
            decoded: 0,
            counts: vec![1; symbols],
            lengths: vec![0; symbols],
            table: vec![0; 1 << TABLE_BITS],
            sorted: vec![0; symbols],
            per_length: [0; MAX_LENGTH as usize + 1],
            scratch: Scratch {
                order: vec![0; symbols],
                leaf_count: vec![0; symbols],
                leaf_parent: vec![0; symbols],
                leaf_depth: vec![0; symbols],
                node_count: vec![0; internal],
                node_parent: vec![0; internal],
                node_depth: vec![0; internal],
            },
        };
        code.rebuild()?;
        Ok(code)
    }

    /// Decode one symbol, and count it.
    pub(crate) fn decode(&mut self, bits: &mut BitReader<'_>) -> Result<usize> {
        let window = bits.peek15();
        let entry = self.table[(window >> (MAX_LENGTH - TABLE_BITS)) as usize];
        let short = u32::from(entry & 0xF);
        let (symbol, length) = if short == 0 {
            self.decode_long(window)?
        } else {
            (usize::from(entry >> 4), short)
        };
        bits.consume(length)?;
        self.counts[symbol] += 1;
        self.decoded += 1;
        if self.decoded == self.period {
            self.rebuild()?;
            for count in &mut self.counts {
                *count = (*count >> 1) + 1;
            }
            self.decoded = 0;
        }
        Ok(symbol)
    }

    /// Decode a codeword that is longer than the lookup table, from a 15-bit window.
    fn decode_long(&self, window: u32) -> Result<(usize, u32)> {
        let mut first = 0u32;
        let mut index = 0u32;
        for length in 1..=MAX_LENGTH {
            let code = window >> (MAX_LENGTH - length);
            let count = self.per_length[length as usize];
            if code.wrapping_sub(first) < count {
                let at = usize::try_from(index + code - first)
                    .map_err(|_| Error::invalid_data("Huffman symbol index"))?;
                return Ok((self.sorted[at], length));
            }
            index += count;
            first = (first + count) << 1;
        }
        Err(Error::invalid_data("incomplete Huffman code"))
    }

    /// Make a new code from the symbol counts.
    fn rebuild(&mut self) -> Result<()> {
        self.build_lengths()?;
        self.build_table();
        Ok(())
    }

    /// Compute the codeword lengths from the symbol counts.
    fn build_lengths(&mut self) -> Result<()> {
        let counts = &self.counts;
        let s = &mut self.scratch;
        for (index, symbol) in s.order.iter_mut().enumerate() {
            *symbol = index;
        }
        s.order.sort_by_key(|&symbol| counts[symbol]);
        for (leaf, &symbol) in s.order.iter().enumerate() {
            s.leaf_count[leaf] = counts[symbol];
        }
        loop {
            s.build_tree();
            if s.leaf_depth[0] <= MAX_LENGTH {
                break;
            }
            for count in &mut s.leaf_count {
                *count = (*count + 1) >> 1;
            }
        }
        for (leaf, &symbol) in s.order.iter().enumerate() {
            let depth = s.leaf_depth[leaf];
            if depth > MAX_LENGTH {
                return Err(Error::invalid_data("Huffman codeword longer than 15 bits"));
            }
            self.lengths[symbol] = depth;
        }
        Ok(())
    }

    /// Make the canonical codewords and the lookup table from the codeword lengths.
    fn build_table(&mut self) {
        self.per_length = [0; MAX_LENGTH as usize + 1];
        for &length in &self.lengths {
            self.per_length[length as usize] += 1;
        }
        let mut start = [0usize; MAX_LENGTH as usize + 2];
        for length in 1..=MAX_LENGTH as usize {
            start[length + 1] = start[length] + self.per_length[length] as usize;
        }
        let mut next = start;
        for (symbol, &length) in self.lengths.iter().enumerate() {
            self.sorted[next[length as usize]] = symbol;
            next[length as usize] += 1;
        }
        self.table.fill(0);
        let mut code = 0usize;
        for length in 1..=TABLE_BITS {
            let span = 1usize << (TABLE_BITS - length);
            for &symbol in &self.sorted[start[length as usize]..start[length as usize + 1]] {
                // The symbol is less than 799 and the length is at most 10, so the entry fits.
                let entry = u16::try_from(symbol << 4 | length as usize).unwrap_or_default();
                self.table[code * span..(code + 1) * span].fill(entry);
                code += 1;
            }
            code <<= 1;
        }
    }
}

impl Scratch {
    /// Build the Huffman tree from `leaf_count`, and set `leaf_depth`.
    ///
    /// The leaves are in order of count. The internal nodes are in order of construction, so
    /// their counts also increase. The first node merges the first two leaves.
    fn build_tree(&mut self) {
        let leaves = self.leaf_count.len();
        self.node_count[0] = self.leaf_count[0] + self.leaf_count[1];
        self.leaf_parent[0] = 0;
        self.leaf_parent[1] = 0;
        let mut next_leaf = 2;
        let mut next_node = 0;
        let mut created = 1;
        while next_leaf < leaves || next_node + 1 < created {
            let new = created;
            created += 1;
            let mut sum = 0;
            for _ in 0..2 {
                let take_node = next_leaf == leaves
                    || (next_node < new && self.node_count[next_node] < self.leaf_count[next_leaf]);
                if take_node {
                    sum += self.node_count[next_node];
                    self.node_parent[next_node] = new;
                    next_node += 1;
                } else {
                    sum += self.leaf_count[next_leaf];
                    self.leaf_parent[next_leaf] = new;
                    next_leaf += 1;
                }
            }
            self.node_count[new] = sum;
        }
        let root = created - 1;
        self.node_depth[root] = 0;
        for node in (0..root).rev() {
            self.node_depth[node] = self.node_depth[self.node_parent[node]] + 1;
        }
        for leaf in 0..leaves {
            self.leaf_depth[leaf] = self.node_depth[self.leaf_parent[leaf]] + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lengths(counts: &[u32]) -> Vec<u32> {
        let mut code = AdaptiveCode::new(counts.len(), 1024).unwrap();
        code.counts.copy_from_slice(counts);
        code.rebuild().unwrap();
        code.lengths.clone()
    }

    #[test]
    fn equal_counts_give_a_balanced_code() {
        assert_eq!(lengths(&[1; 256]), vec![8; 256]);
        assert_eq!(lengths(&[1; 8]), vec![3; 8]);
    }

    #[test]
    fn a_tie_takes_the_leaf_first() {
        // After the merge of the two 1-counts, a leaf and the node have the count 2.
        assert_eq!(lengths(&[1, 1, 2, 2]), vec![2, 2, 2, 2]);
    }

    #[test]
    fn a_skewed_code_is_limited_to_15_bits() {
        let mut counts = vec![1u32; 20];
        let mut fib = (1u32, 1u32);
        for count in &mut counts {
            *count = fib.0;
            fib = (fib.1, fib.0 + fib.1);
        }
        let lengths = lengths(&counts);
        assert!(lengths.iter().all(|&length| (1..=15).contains(&length)));
        let kraft: f64 = lengths
            .iter()
            .map(|&l| 0.5f64.powi(i32::try_from(l).unwrap()))
            .sum();
        assert!((kraft - 1.0).abs() < 1e-9);
    }
}
