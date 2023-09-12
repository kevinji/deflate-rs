use crate::bit_io::{BitReader, BitWriter};
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
    out: BitWriter<W>,
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
            out: BitWriter::new(out),
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
                            self.out.write_u8(self.in_.read_u8()?)?;
                        }
                    }
                    DeflateEncoding::FixedHuffman => todo!(),
                    DeflateEncoding::DynamicHuffman => todo!(),
                }

                if is_final {
                    self.out.flush_even_if_partial()?;
                    self.stage = DecodeStage::Complete;
                } else {
                    self.stage = DecodeStage::NewBlock;
                }

                Ok(())
            }
            DecodeStage::Complete => Ok(()),
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
                header_bits.push(is_eof.into());

                let encoding_bits = <BitVec<_>>::from(DeflateEncoding::NoCompression);
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
