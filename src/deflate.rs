use crate::{
    bit_io::BitReader,
    huffman::{DistanceEncoding, HuffmanTree},
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
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected 2 encoding bits, got {}", slice.len()),
            ));
        }

        match slice.load_le::<u8>() {
            0b00 => Ok(Self::NoCompression),
            0b01 => Ok(Self::FixedHuffman),
            0b10 => Ok(Self::DynamicHuffman),
            0b11 => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "0b11 is not a valid encoding",
            )),
            _ => unreachable!(),
        }
    }
}

impl From<DeflateEncoding> for BitVec<u8> {
    fn from(encoding: DeflateEncoding) -> Self {
        let bits: u8 = match encoding {
            DeflateEncoding::NoCompression => 0b00,
            DeflateEncoding::FixedHuffman => 0b01,
            DeflateEncoding::DynamicHuffman => 0b10,
        };
        Self::from_element(bits)
    }
}

fn parse_symbol<R>(
    length_huffman_tree: &HuffmanTree,
    distance_encoding: &DistanceEncoding,
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
                    let extra_bits = in_.read_u8_from_bits(extra_bit_count.into())?;

                    (1 << (length_code_minus_257 / 4 + 1))
                        + (1 << (length_code_minus_257 / 4 - 1)) * (length_code_minus_257 % 4)
                        + extra_bits
                }
                28 => 255,
                29.. => unreachable!(),
            };

            let distance_code: u8 = distance_encoding.decode(in_)?.try_into().unwrap();
            let distance_minus_one = match distance_code {
                0..=3 => distance_code.into(),
                4..=29 => {
                    let extra_bit_count = distance_code / 2 - 1;
                    let extra_bits = in_.read_u16_from_bits(extra_bit_count.into())?;

                    (1 << (distance_code / 2))
                        + (1 << (distance_code / 2 - 1)) * u16::from(distance_code % 2)
                        + extra_bits
                }
                30.. => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("distance code must be <= 29, got {distance_code}"),
                    ))
                }
            };

            Ok(Symbol::BackReference {
                length_minus_three,
                distance_minus_one,
            })
        }
        286.. => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("length code must be <= 285, got {length_code}"),
        )),
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

impl Default for DecodeStage {
    fn default() -> Self {
        Self::NewBlock
    }
}

#[derive(Debug, Default)]
pub struct DeflateDecoder {
    /// Stores a 32k buffer when blocks are compressed
    out_buffer: OutBuffer,
    stage: DecodeStage,
}

impl DeflateDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn advance_stage<R, W>(&mut self, in_: &mut BitReader<R>, out: &mut W) -> io::Result<()>
    where
        R: io::Read,
        W: io::Write,
    {
        match self.stage {
            DecodeStage::NewBlock => {
                let is_final = in_.read_bool()?;

                let encoding_bits = bits![mut u8, Lsb0; 0; 2];
                in_.read_exact(encoding_bits)?;
                let encoding = (&*encoding_bits).try_into()?;

                self.stage = DecodeStage::ParsedMode { is_final, encoding };

                Ok(())
            }
            DecodeStage::ParsedMode { is_final, encoding } => {
                match encoding {
                    DeflateEncoding::NoCompression => {
                        in_.skip_to_byte_end();

                        let len = in_.read_u16()?;
                        let nlen = in_.read_u16()?;

                        if !len != nlen {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("len {len} does not match nlen {nlen}"),
                            ));
                        }

                        for _ in 0..len {
                            out.write_all(&[in_.read_u8()?])?;
                        }
                    }
                    DeflateEncoding::FixedHuffman => {
                        let literal_huffman_tree = HuffmanTree::fixed_literal();

                        self.decode_huffman_block(
                            in_,
                            out,
                            &literal_huffman_tree,
                            &DistanceEncoding::Fixed,
                        )?;
                    }
                    DeflateEncoding::DynamicHuffman => {
                        let literal_code_length_count = in_.read_u16_from_bits(5)? + 257;
                        let distance_code_length_count = in_.read_u8_from_bits(5)? + 1;
                        let code_length_symbol_count = in_.read_u8_from_bits(4)? + 4;

                        let mut code_lengths_in_symbol_order =
                            Vec::with_capacity(code_length_symbol_count.into());
                        for _ in 0..code_length_symbol_count {
                            let code_length = in_.read_u8_from_bits(3)?;
                            code_lengths_in_symbol_order.push(code_length);
                        }

                        let code_lengths_huffman_tree =
                            HuffmanTree::dynamic_code_lengths(&code_lengths_in_symbol_order);

                        let literal_huffman_tree = code_lengths_huffman_tree
                            .decode_code_lengths(literal_code_length_count.into(), in_)?;

                        let distance_huffman_tree = code_lengths_huffman_tree
                            .decode_code_lengths(distance_code_length_count.into(), in_)?;

                        self.decode_huffman_block(
                            in_,
                            out,
                            &literal_huffman_tree,
                            &DistanceEncoding::Dynamic(distance_huffman_tree),
                        )?;
                    }
                }

                if is_final {
                    in_.skip_to_byte_end();
                    out.flush()?;
                    self.stage = DecodeStage::Complete;
                } else {
                    self.stage = DecodeStage::NewBlock;
                }

                Ok(())
            }
            DecodeStage::Complete => Ok(()),
        }
    }

    fn decode_huffman_block<R, W>(
        &mut self,
        in_: &mut BitReader<R>,
        out: &mut W,
        literal_huffman_tree: &HuffmanTree,
        distance_encoding: &DistanceEncoding,
    ) -> io::Result<()>
    where
        R: io::Read,
        W: io::Write,
    {
        loop {
            let length_symbol = parse_symbol(literal_huffman_tree, distance_encoding, in_)?;

            match length_symbol {
                Symbol::Literal(literal) => {
                    out.write_all(&[literal])?;
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
                        let byte =
                            self.out_buffer
                                .get(distance_minus_one.into())
                                .ok_or_else(|| {
                                    io::Error::new(
                                        io::ErrorKind::InvalidData,
                                        format!(
                                            "invalid backreference with distance {}",
                                            distance_minus_one + 1,
                                        ),
                                    )
                                })?;

                        out.write_all(&[byte])?;
                        self.out_buffer.push(byte);
                    }
                }
            }
        }
    }

    pub fn decode<R, W>(&mut self, in_: &mut BitReader<R>, out: &mut W) -> io::Result<()>
    where
        R: io::Read,
        W: io::Write,
    {
        while !matches!(self.stage, DecodeStage::Complete) {
            self.advance_stage(in_, out)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
enum EncodeStage {
    NewBlock,
    Complete,
}

impl Default for EncodeStage {
    fn default() -> Self {
        Self::NewBlock
    }
}

#[derive(Debug, Default)]
pub struct DeflateEncoder {
    stage: EncodeStage,
}

impl DeflateEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn advance_stage<R, W>(&mut self, in_: &mut R, out: &mut W) -> io::Result<()>
    where
        R: io::Read,
        W: io::Write,
    {
        match self.stage {
            EncodeStage::NewBlock => {
                const MAX_BYTES_PER_BLOCK: usize = u16::MAX as usize;
                let mut buf = [0u8; MAX_BYTES_PER_BLOCK];
                let mut len = 0;
                let mut is_eof = false;

                loop {
                    match in_.read(&mut buf[len..]) {
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
                        Err(e) if matches!(e.kind(), io::ErrorKind::Interrupted) => continue,
                        Err(e) => return Err(e),
                    }
                }

                let mut header_bits = bitvec![u8, Lsb0; 0; 0];
                header_bits.push(is_eof);

                let encoding_bits = BitVec::from(DeflateEncoding::NoCompression);
                header_bits.extend_from_bitslice(encoding_bits.as_bitslice());

                // Pad bits to a full byte
                header_bits.resize(8, false);

                io::copy(&mut header_bits, out)?;

                // `.unwrap()` is safe because `len <= u16::MAX`
                let len_header: u16 = len.try_into().unwrap();
                let nlen_header = !len_header;

                out.write_all(&len_header.to_le_bytes())?;
                out.write_all(&nlen_header.to_le_bytes())?;
                out.write_all(&buf[..len])?;

                if is_eof {
                    out.flush()?;
                    self.stage = EncodeStage::Complete;
                }

                Ok(())
            }
            EncodeStage::Complete => Ok(()),
        }
    }

    pub fn encode<R, W>(&mut self, in_: &mut R, out: &mut W) -> io::Result<()>
    where
        R: io::Read,
        W: io::Write,
    {
        while !matches!(self.stage, EncodeStage::Complete) {
            self.advance_stage(in_, out)?;
        }

        Ok(())
    }
}
