use crate::{
    bit_io::BitReader,
    huffman::HuffmanTree,
    lzss::{OutBuffer, Symbol},
};
use bitvec::prelude::*;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeflateEncoding {
    NoCompression,
    FixedHuffman,
    DynamicHuffman,
}

impl TryFrom<&BitSlice<u8>> for DeflateEncoding {
    type Error = io::Error;

    fn try_from(slice: &BitSlice<u8>) -> io::Result<Self> {
        if slice.len() != 2 {
            return Err(io::ErrorKind::InvalidData.into());
        }

        match (slice[0], slice[1]) {
            (false, false) => Ok(Self::NoCompression),
            (false, true) => Ok(Self::FixedHuffman),
            (true, false) => Ok(Self::DynamicHuffman),
            (true, true) => Err(io::ErrorKind::InvalidData.into()),
        }
    }
}

impl From<DeflateEncoding> for BitVec<u8> {
    fn from(encoding: DeflateEncoding) -> Self {
        match encoding {
            DeflateEncoding::NoCompression => bitvec![u8, Lsb0; 0, 0],
            DeflateEncoding::FixedHuffman => bitvec![u8, Lsb0; 0, 1],
            DeflateEncoding::DynamicHuffman => bitvec![u8, Lsb0; 1, 0],
        }
    }
}

fn parse_symbol<R>(
    length_huffman_tree: &HuffmanTree,
    distance_huffman_tree: &HuffmanTree,
    in_: &mut BitReader<R>,
) -> io::Result<Symbol>
where
    R: io::Read,
{
    let length_code = length_huffman_tree.decode(in_)?;

    match length_code {
        0..=255 => Ok(Symbol::Literal(length_code.try_into().unwrap())),
        256 => Ok(Symbol::EndOfBlock),
        257..=285 => {
            let length_code_minus_257: u8 = (length_code - 257).try_into().unwrap();
            let length_minus_three = match length_code_minus_257 {
                0..=7 => length_code_minus_257,
                8..=27 => {
                    let extra_bit_count = length_code_minus_257 / 4 - 1;
                    let extra_bits = in_.read_u8_from_msb_bits(extra_bit_count.into())?;

                    (1 << (length_code_minus_257 / 4 + 1)) + extra_bits
                }
                28 => 255,
                29.. => unreachable!(),
            };

            // TODO: Perhaps restrict `distance_huffman_tree` to u8
            let distance_code: u8 = distance_huffman_tree.decode(in_)?.try_into().unwrap();
            let distance_minus_one = match distance_code {
                0..=3 => distance_code.into(),
                4..=29 => {
                    let extra_bit_count = distance_code / 2 - 1;
                    let extra_bits = in_.read_u16_from_msb_bits(extra_bit_count.into())?;

                    (1 << (distance_code / 2)) + extra_bits
                }
                30.. => return Err(io::ErrorKind::InvalidData.into()),
            };

            Ok(Symbol::BackReference {
                length_minus_three,
                distance_minus_one,
            })
        }
        286.. => Err(io::ErrorKind::InvalidData.into()),
    }
}

#[derive(Debug)]
enum DecodeStage {
    NewBlock,
    ParsedMode {
        is_final: bool,
        encoding: DeflateEncoding,
    },
    Complete,
}

#[derive(Debug)]
pub struct DeflateDecoder<R, W> {
    in_: BitReader<R>,
    out: W,
    /// Stores a 32k buffer when blocks are compressed
    out_buffer: OutBuffer,
    stage: DecodeStage,
}

impl<R, W> DeflateDecoder<R, W>
where
    R: io::Read,
    W: io::Write,
{
    pub fn new(in_: R, out: W) -> Self {
        Self {
            in_: BitReader::new(in_),
            out,
            out_buffer: OutBuffer::new(),
            stage: DecodeStage::NewBlock,
        }
    }

    fn advance_stage(&mut self) -> io::Result<()> {
        match self.stage {
            DecodeStage::NewBlock => {
                let is_final = self.in_.read_bool()?;

                let encoding_bits = bits![mut u8, Lsb0; 0; 2];
                self.in_.read_exact(encoding_bits)?;
                let encoding = (&*encoding_bits).try_into()?;

                self.stage = DecodeStage::ParsedMode { is_final, encoding };

                Ok(())
            }
            DecodeStage::ParsedMode { is_final, encoding } => {
                match encoding {
                    DeflateEncoding::NoCompression => {
                        self.in_.skip_to_byte_end();

                        let len = self.in_.read_u16()?;
                        let nlen = self.in_.read_u16()?;

                        if !len != nlen {
                            return Err(io::ErrorKind::InvalidData.into());
                        }

                        for _ in 0..len {
                            self.out.write_all(&[self.in_.read_u8()?])?;
                        }
                    }
                    DeflateEncoding::FixedHuffman => {
                        let literal_huffman_tree = HuffmanTree::fixed_literal();
                        let distance_huffman_tree = HuffmanTree::fixed_distance();

                        self.decode_huffman_block(&literal_huffman_tree, &distance_huffman_tree)?;
                    }
                    DeflateEncoding::DynamicHuffman => {
                        let literal_code_length_count =
                            u16::from(self.in_.read_u8_from_msb_bits(5)?) + 257;
                        let distance_code_length_count = self.in_.read_u8_from_msb_bits(5)? + 1;
                        let code_length_symbol_count = self.in_.read_u8_from_msb_bits(4)? + 4;

                        let mut code_lengths_in_symbol_order =
                            Vec::with_capacity(code_length_symbol_count.into());
                        for _ in 0..code_length_symbol_count {
                            let code_length = self.in_.read_u8_from_msb_bits(3)?;
                            code_lengths_in_symbol_order.push(code_length);
                        }

                        let code_lengths_huffman_tree =
                            HuffmanTree::dynamic_code_lengths(&code_lengths_in_symbol_order);

                        let literal_huffman_tree = code_lengths_huffman_tree
                            .decode_code_lengths(literal_code_length_count, &mut self.in_)?;

                        let distance_huffman_tree = code_lengths_huffman_tree.decode_code_lengths(
                            distance_code_length_count.into(),
                            &mut self.in_,
                        )?;

                        self.decode_huffman_block(&literal_huffman_tree, &distance_huffman_tree)?;
                    }
                }

                if is_final {
                    self.out.flush()?;
                    self.stage = DecodeStage::Complete;
                } else {
                    self.stage = DecodeStage::NewBlock;
                }

                Ok(())
            }
            DecodeStage::Complete => Ok(()),
        }
    }

    fn decode_huffman_block(
        &mut self,
        literal_huffman_tree: &HuffmanTree,
        distance_huffman_tree: &HuffmanTree,
    ) -> io::Result<()> {
        loop {
            let length_symbol =
                parse_symbol(literal_huffman_tree, distance_huffman_tree, &mut self.in_)?;

            match length_symbol {
                Symbol::Literal(literal) => {
                    self.out.write_all(&[literal])?;
                    self.out_buffer.push(literal);
                }
                Symbol::EndOfBlock => {
                    return Ok(());
                }
                Symbol::BackReference {
                    length_minus_three,
                    distance_minus_one,
                } => {
                    let length = u16::from(length_minus_three) + 3;
                    for _ in 0..length {
                        let byte = self
                            .out_buffer
                            .get(distance_minus_one.into())
                            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;

                        self.out.write_all(&[byte])?;
                        self.out_buffer.push(byte);
                    }
                }
            }
        }
    }

    pub fn decode(&mut self) -> io::Result<()> {
        while !matches!(self.stage, DecodeStage::Complete) {
            self.advance_stage()?;
        }

        Ok(())
    }
}

#[derive(Debug)]
enum EncodeStage {
    NewBlock,
    Complete,
}

#[derive(Debug)]
pub struct DeflateEncoder<R, W> {
    in_: R,
    out: W,
    stage: EncodeStage,
}

impl<R, W> DeflateEncoder<R, W>
where
    R: io::Read,
    W: io::Write,
{
    pub fn new(in_: R, out: W) -> Self {
        Self {
            in_,
            out,
            stage: EncodeStage::NewBlock,
        }
    }

    fn advance_stage(&mut self) -> io::Result<()> {
        match self.stage {
            EncodeStage::NewBlock => {
                const MAX_BYTES_PER_BLOCK: usize = u16::MAX as usize;
                let mut buf = [0u8; MAX_BYTES_PER_BLOCK];
                let mut len = 0;
                let mut is_eof = false;

                loop {
                    match self.in_.read(&mut buf[len..]) {
                        Ok(0) => {
                            is_eof = true;
                            break;
                        }
                        Ok(n) => {
                            len += n;
                            if len == MAX_BYTES_PER_BLOCK {
                                break;
                            }
                        }
                        Err(err) => match err.kind() {
                            io::ErrorKind::Interrupted => continue,
                            _ => return Err(err),
                        },
                    }
                }

                let mut header_bits = bitvec![u8, Lsb0; 0; 0];
                header_bits.push(is_eof);

                let encoding_bits = BitVec::from(DeflateEncoding::NoCompression);
                header_bits.extend_from_bitslice(encoding_bits.as_bitslice());

                // Pad bits to a full byte
                header_bits.resize(8, false);

                io::copy(&mut header_bits, &mut self.out)?;

                // `.unwrap()` is safe because `len <= u16::MAX`
                let len_header: u16 = len.try_into().unwrap();
                let nlen_header = !len_header;

                self.out.write_all(&len_header.to_le_bytes())?;
                self.out.write_all(&nlen_header.to_le_bytes())?;
                self.out.write_all(&buf[..len])?;

                if is_eof {
                    self.out.flush()?;
                    self.stage = EncodeStage::Complete;
                }

                Ok(())
            }
            EncodeStage::Complete => Ok(()),
        }
    }

    pub fn encode(&mut self) -> io::Result<()> {
        while !matches!(self.stage, EncodeStage::Complete) {
            self.advance_stage()?;
        }

        Ok(())
    }
}
