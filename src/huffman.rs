use crate::bit_io::BitReader;
use bitvec::prelude::*;
use std::{collections::BTreeMap, io};

const FIXED_CODE_LENGTHS: [&[u8]; 4] = [&[8; 144], &[9; 112], &[7; 24], &[8; 8]];

fn compute_heap_index(code: u16, code_len: usize) -> usize {
    let code_bits = &code.view_bits::<Lsb0>()[..code_len];
    let mut index = 1;
    for bit in code_bits.iter().by_vals().rev() {
        index = 2 * index + bit as usize;
    }

    index
}

#[derive(Debug)]
pub struct HuffmanTree {
    /// The Huffman tree, encoded as an array-based heap.
    /// The root node is at index 1, and children are at 2n and 2n+1.
    tree: Vec<Option<u16>>,
}

impl HuffmanTree {
    pub fn new(code_lengths: &[u8]) -> Self {
        let code_length_counts =
            code_lengths
                .iter()
                .fold(<BTreeMap<_, u16>>::new(), |mut map, &length| {
                    *map.entry(length).or_default() += 1;
                    map
                });

        let largest_code_length = code_length_counts
            .last_key_value()
            .map_or(0, |(&code_len, _)| code_len);

        let mut next_code = vec![0];
        let mut code = 0;

        for length in 1..=largest_code_length {
            let count = code_length_counts
                .get(&(length - 1))
                .copied()
                .unwrap_or_default();
            code = (code + count) << 1;
            next_code.push(code);
        }

        let mut tree = vec![None; 1 << (largest_code_length + 1)];
        for (symbol, &code_len) in code_lengths.iter().enumerate() {
            let code_len = usize::from(code_len);
            let code = next_code[code_len];

            let heap_index = compute_heap_index(code, code_len);
            let heap_symbol = u16::try_from(symbol).unwrap();
            tree[heap_index] = Some(heap_symbol);

            next_code[code_len] += 1;
        }

        Self { tree }
    }

    /// Fixed tree:
    /// 0-143: 0011 -> 10 (8 bits) -> 144 (18*8)
    /// 144-255: 11001 -> 1101, 111 (9 bits) -> 112 (14*8)
    /// 256-279: 000 -> 0010 (7 bits) -> 24 (3*8)
    /// 280-287: 11000 (8 bits) -> 8 (1*8)
    pub fn fixed() -> Self {
        Self::new(&FIXED_CODE_LENGTHS.concat())
    }

    pub fn decode<R>(&self, in_: &mut BitReader<R>) -> io::Result<u16>
    where
        R: io::Read,
    {
        loop {
            let bit = in_.read_bool()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_huffman() {
        assert_eq!(FIXED_CODE_LENGTHS.concat().len(), 288);
        let _tree = HuffmanTree::fixed();
    }
}
