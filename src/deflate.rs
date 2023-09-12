use crate::bit_io::{BitReader, BitWriter};
use bitvec::prelude::*;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompressionType {
    None,
    FixedHuffman,
    DynamicHuffman,
}

impl TryFrom<&BitSlice<u8>> for CompressionType {
    type Error = io::Error;

    fn try_from(slice: &BitSlice<u8>) -> io::Result<Self> {
        if slice.len() != 2 {
            return Err(io::ErrorKind::InvalidData.into());
        }

        match (slice[0], slice[1]) {
            (false, false) => Ok(Self::None),
            (false, true) => Ok(Self::FixedHuffman),
            (true, false) => Ok(Self::DynamicHuffman),
            (true, true) => Err(io::ErrorKind::InvalidData.into()),
        }
    }
}

impl From<CompressionType> for BitArray<[u8; 1]> {
    fn from(compression_type: CompressionType) -> Self {
        match compression_type {
            CompressionType::None => bitarr![u8, Lsb0; 0, 0],
            CompressionType::FixedHuffman => bitarr![u8, Lsb0; 0, 1],
            CompressionType::DynamicHuffman => bitarr![u8, Lsb0; 1, 0],
        }
    }
}

#[derive(Debug)]
enum DecompressionStage {
    NewBlock,
    ParsedMode {
        is_final: bool,
        compression_type: CompressionType,
    },
    Complete,
}

#[derive(Debug)]
pub struct Decompressor<R, W> {
    in_: BitReader<R>,
    out: BitWriter<W>,
    stage: DecompressionStage,
}

impl<R, W> Decompressor<R, W>
where
    R: io::Read,
    W: io::Write,
{
    pub fn new(in_: R, out: W) -> Self {
        Self {
            in_: BitReader::new(in_),
            out: BitWriter::new(out),
            stage: DecompressionStage::NewBlock,
        }
    }

    fn advance_stage(&mut self) -> io::Result<()> {
        match self.stage {
            DecompressionStage::NewBlock => {
                let is_final = self.in_.read_bool()?;

                let mut compression_type_bits = bitarr![u8, Lsb0; 0; 2];
                self.in_.read_exact(&mut compression_type_bits)?;
                let compression_type = compression_type_bits.as_bitslice().try_into()?;

                self.stage = DecompressionStage::ParsedMode {
                    is_final,
                    compression_type,
                };

                Ok(())
            }
            DecompressionStage::ParsedMode {
                is_final,
                compression_type,
            } => {
                match compression_type {
                    CompressionType::None => {
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
                    CompressionType::FixedHuffman => todo!(),
                    CompressionType::DynamicHuffman => todo!(),
                }

                if is_final {
                    self.out.flush_even_if_partial()?;
                    self.stage = DecompressionStage::Complete;
                } else {
                    self.stage = DecompressionStage::NewBlock;
                }

                Ok(())
            }
            DecompressionStage::Complete => Ok(()),
        }
    }

    pub fn decompress(&mut self) -> io::Result<()> {
        while !matches!(self.stage, DecompressionStage::Complete) {
            self.advance_stage()?;
        }

        Ok(())
    }
}

#[derive(Debug)]
enum CompressionStage {
    Begin,
    NewBlock,
    Complete,
}

#[derive(Debug)]
pub struct Compressor<R, W> {
    in_: R,
    out: W,
    stage: CompressionStage,
}

impl<R, W> Compressor<R, W>
where
    R: io::Read,
    W: io::Write,
{
    pub fn new(in_: R, out: W) -> Self {
        Self {
            in_,
            out,
            stage: CompressionStage::Begin,
        }
    }

    fn advance_stage(&mut self) -> io::Result<()> {
        match self.stage {
            CompressionStage::Begin => {
                io::copy(
                    &mut <BitArray<_>>::from(CompressionType::None).as_bitslice(),
                    &mut self.out,
                )?;
                self.stage = CompressionStage::NewBlock;
                Ok(())
            }
            CompressionStage::NewBlock => {
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

                // `.unwrap()` is safe because `len <= u16::MAX`
                let len_header: u16 = len.try_into().unwrap();
                let nlen_header = !len_header;

                self.out.write_all(&len_header.to_le_bytes())?;
                self.out.write_all(&nlen_header.to_le_bytes())?;
                self.out.write_all(&buf[..len])?;

                if is_eof {
                    self.out.flush()?;
                    self.stage = CompressionStage::Complete;
                }

                Ok(())
            }
            CompressionStage::Complete => Ok(()),
        }
    }

    pub fn compress(&mut self) -> io::Result<()> {
        while !matches!(self.stage, CompressionStage::Complete) {
            self.advance_stage()?;
        }

        Ok(())
    }
}
