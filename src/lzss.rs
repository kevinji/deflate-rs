use num_traits::int::PrimInt;
use std::io;

#[derive(Debug)]
pub enum Symbol {
    /// A literal byte
    Literal(u8),

    /// End of a block
    EndOfBlock,

    /// A back-reference
    BackReference {
        length_minus_three: u8,
        distance_minus_one: u16,
    },
}

impl Symbol {
    pub fn length_code(&self) -> u16 {
        match self {
            Self::Literal(b) => (*b).into(),
            Self::EndOfBlock => 256,
            Self::BackReference {
                length_minus_three,
                distance_minus_one: _,
            } => Self::back_reference_length_code(*length_minus_three),
        }
    }

    pub fn parse_length_code(
        &self,
        length_code: u16,
        read_next_bit: impl Fn() -> io::Result<bool>,
        read_distance_code: impl FnOnce() -> io::Result<u8>,
    ) -> io::Result<Self> {
        match length_code {
            0..=255 => Ok(Self::Literal(length_code.try_into().unwrap())),
            256 => Ok(Self::EndOfBlock),
            257..=285 => {
                let length_code_minus_257: u8 = (length_code - 257).try_into().unwrap();
                let length_minus_three = match length_code_minus_257 {
                    0..=7 => length_code_minus_257,
                    8..=27 => {
                        let extra_bit_count = length_code_minus_257 / 4 - 1;
                        let extra_bits =
                            Self::read_extra_bits::<u8>(extra_bit_count, &read_next_bit)?;

                        (1 << (length_code_minus_257 / 4 + 1)) + extra_bits
                    }
                    28 => 255,
                    29.. => unreachable!(),
                };

                let distance_code = read_distance_code()?;
                let distance_minus_one = match distance_code {
                    0..=3 => distance_code.into(),
                    4..=29 => {
                        let extra_bit_count = distance_code / 2 - 1;
                        let extra_bits =
                            Self::read_extra_bits::<u16>(extra_bit_count, &read_next_bit)?;

                        (1 << (distance_code / 2)) + extra_bits
                    }
                    30.. => return Err(io::ErrorKind::InvalidData.into()),
                };

                Ok(Self::BackReference {
                    length_minus_three,
                    distance_minus_one,
                })
            }
            286.. => Err(io::ErrorKind::InvalidData.into()),
        }
    }

    fn read_extra_bits<T>(
        extra_bit_count: u8,
        read_next_bit: impl Fn() -> io::Result<bool>,
    ) -> io::Result<T>
    where
        T: PrimInt + From<bool>,
    {
        let mut extra_bits = T::zero();
        for _ in 0..extra_bit_count {
            let bit = read_next_bit()?;
            extra_bits = (extra_bits << 1) + bit.into();
        }
        Ok(extra_bits)
    }

    pub fn back_reference_length_code(length_minus_three: u8) -> u16 {
        257 + u16::from(match length_minus_three {
            0..=7 => length_minus_three,
            8..=254 => {
                let log2: u8 = length_minus_three.ilog2().try_into().unwrap();
                4 * (log2 - 1) + (length_minus_three >> (log2 - 2) & 0b11)
            }
            255 => 28,
        })
    }

    pub fn back_reference_length_extra_bits(length_minus_three: u8) -> u8 {
        match length_minus_three {
            0..=7 => 0,
            8..=254 => u8::try_from(length_minus_three.ilog2()).unwrap() - 2,
            255 => 0,
        }
    }

    pub fn back_reference_distance_code(distance_minus_one: u16) -> u8 {
        match distance_minus_one {
            0..=3 => distance_minus_one.try_into().unwrap(),
            4..=32767 => {
                let log2: u8 = distance_minus_one.ilog2().try_into().unwrap();
                2 * log2 + u8::try_from(distance_minus_one >> (log2 - 1) & 1).unwrap()
            }
            32768.. => panic!("Distance cannot be more than 32768"),
        }
    }

    pub fn back_reference_distance_extra_bits(distance_minus_one: u16) -> u8 {
        match distance_minus_one {
            0..=3 => 0,
            4..=32767 => u8::try_from(distance_minus_one.ilog2()).unwrap() - 1,
            32768.. => panic!("Distance cannot be more than 32768"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn expected_lengths_by_code() -> HashMap<u16, Vec<u16>> {
        vec![
            (257, vec![3]),
            (258, vec![4]),
            (259, vec![5]),
            (260, vec![6]),
            (261, vec![7]),
            (262, vec![8]),
            (263, vec![9]),
            (264, vec![10]),
            (265, vec![11, 12]),
            (266, vec![13, 14]),
            (267, vec![15, 16]),
            (268, vec![17, 18]),
            (269, (19..=22).collect()),
            (270, (23..=26).collect()),
            (271, (27..=30).collect()),
            (272, (31..=34).collect()),
            (273, (35..=42).collect()),
            (274, (43..=50).collect()),
            (275, (51..=58).collect()),
            (276, (59..=66).collect()),
            (277, (67..=82).collect()),
            (278, (83..=98).collect()),
            (279, (99..=114).collect()),
            (280, (115..=130).collect()),
            (281, (131..=162).collect()),
            (282, (163..=194).collect()),
            (283, (195..=226).collect()),
            (284, (227..=257).collect()),
            (285, vec![258]),
        ]
        .into_iter()
        .collect::<HashMap<u16, Vec<u16>>>()
    }

    fn expected_distances_by_code() -> HashMap<u8, Vec<u16>> {
        vec![
            (0, vec![1]),
            (1, vec![2]),
            (2, vec![3]),
            (3, vec![4]),
            (4, vec![5, 6]),
            (5, vec![7, 8]),
            (6, (9..=12).collect()),
            (7, (13..=16).collect()),
            (8, (17..=24).collect()),
            (9, (25..=32).collect()),
            (10, (33..=48).collect()),
            (11, (49..=64).collect()),
            (12, (65..=96).collect()),
            (13, (97..=128).collect()),
            (14, (129..=192).collect()),
            (15, (193..=256).collect()),
            (16, (257..=384).collect()),
            (17, (385..=512).collect()),
            (18, (513..=768).collect()),
            (19, (769..=1024).collect()),
            (20, (1025..=1536).collect()),
            (21, (1537..=2048).collect()),
            (22, (2049..=3072).collect()),
            (23, (3073..=4096).collect()),
            (24, (4097..=6144).collect()),
            (25, (6145..=8192).collect()),
            (26, (8193..=12288).collect()),
            (27, (12289..=16384).collect()),
            (28, (16385..=24576).collect()),
            (29, (24577..=32768).collect()),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn test_back_reference_length_codes() {
        let mut actual_lengths_by_code = <HashMap<u16, Vec<u16>>>::new();
        for length_minus_three in 0..=255 {
            let length_code = Symbol::back_reference_length_code(length_minus_three);
            let length = u16::from(length_minus_three) + 3;
            actual_lengths_by_code
                .entry(length_code)
                .or_default()
                .push(length);
        }

        assert_eq!(expected_lengths_by_code(), actual_lengths_by_code);
    }

    #[test]
    fn test_back_reference_distance_codes() {
        let mut actual_distances_by_code = <HashMap<u8, Vec<u16>>>::new();
        for distance_minus_one in 0..=32767 {
            let distance_code = Symbol::back_reference_distance_code(distance_minus_one);
            let distance = distance_minus_one + 1;
            actual_distances_by_code
                .entry(distance_code)
                .or_default()
                .push(distance);
        }

        assert_eq!(expected_distances_by_code(), actual_distances_by_code);
    }
}
