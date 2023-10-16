use crate::{bit_io::BitReader, deflate::DeflateDecoder, out_with_checksum::OutWithChecksum};
use bitvec::prelude::*;
use std::io;

const GZIP_ID1: u8 = 0x1f;
const GZIP_ID2: u8 = 0x8b;
const GZIP_CM_DEFLATE: u8 = 0x08;

#[derive(Debug)]
enum DecodeStage {
    NewMember,
    DecodeDeflate,
    Complete,
}

#[derive(Debug)]
pub struct GzipDecoder {
    stage: DecodeStage,
}

impl GzipDecoder {
    pub fn new() -> Self {
        Self {
            stage: DecodeStage::NewMember,
        }
    }

    fn advance_stage<R, W>(&mut self, in_: &mut BitReader<R>, out: &mut W) -> io::Result<()>
    where
        R: io::Read,
        W: io::Write,
    {
        match self.stage {
            DecodeStage::NewMember => {
                if in_.is_eof()? {
                    self.stage = DecodeStage::Complete;
                    return Ok(());
                }

                let id1 = in_.read_u8()?;
                if id1 != GZIP_ID1 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("expected ID1={GZIP_ID1:#02x}, got {id1:#02x}"),
                    ));
                }

                let id2 = in_.read_u8()?;
                if id2 != GZIP_ID2 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("expected ID2={GZIP_ID2:#02x}, got {id2:#02x}"),
                    ));
                }

                let cm = in_.read_u8()?;
                if cm != GZIP_CM_DEFLATE {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("expected CM={GZIP_CM_DEFLATE:#02x}, got {cm:#02x}"),
                    ));
                }

                let flg = bits![mut u8, Lsb0; 0; 8];
                in_.read_exact(flg)?;
                flg.reverse();

                let _ftext = flg[0];
                let fhcrc = flg[1];
                let fextra = flg[2];
                let fname = flg[3];
                let fcomment = flg[4];

                let mtime = in_.read_u32()?;
                let xfl = in_.read_u8()?;
                let os = in_.read_u8()?;

                let mut hcrc_hasher = crc32fast::Hasher::new();
                if fhcrc {
                    hcrc_hasher.update(&[id1, id2, cm, flg.load_le::<u8>()]);
                    hcrc_hasher.update(&mtime.to_le_bytes());
                    hcrc_hasher.update(&[xfl, os]);
                }

                if fextra {
                    let xlen = in_.read_u16()?;
                    if fhcrc {
                        hcrc_hasher.update(&xlen.to_le_bytes());
                    }

                    for _ in 0..xlen {
                        let extra_field = in_.read_u8()?;
                        if fhcrc {
                            hcrc_hasher.update(&[extra_field]);
                        }
                    }
                }

                if fname {
                    loop {
                        let byte = in_.read_u8()?;
                        if fhcrc {
                            hcrc_hasher.update(&[byte]);
                        }

                        // Zero-terminated file name
                        if byte == 0 {
                            break;
                        }
                    }
                }

                if fcomment {
                    loop {
                        let byte = in_.read_u8()?;
                        if fhcrc {
                            hcrc_hasher.update(&[byte]);
                        }

                        // Zero-terminated file comment
                        if byte == 0 {
                            break;
                        }
                    }
                }

                if fhcrc {
                    let crc16 = in_.read_u16()?;

                    let actual_crc32 = hcrc_hasher.finalize();
                    let [crc32_0, crc32_1, _, _] = actual_crc32.to_le_bytes();
                    let actual_crc16 = u16::from_le_bytes([crc32_0, crc32_1]);

                    if crc16 != actual_crc16 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("CRC-16 of header does not match; got {actual_crc16}, expected {crc16}")
                        ));
                    }
                }

                self.stage = DecodeStage::DecodeDeflate;
                Ok(())
            }
            DecodeStage::DecodeDeflate => {
                let mut out_with_checksum = OutWithChecksum::new(out);

                let mut deflate_decoder = DeflateDecoder::new();
                deflate_decoder.decode(in_, &mut out_with_checksum)?;

                let actual_crc32 = out_with_checksum.crc32();
                let actual_input_size = out_with_checksum.size();

                let crc32 = in_.read_u32()?;
                let input_size = in_.read_u32()?;

                if crc32 != actual_crc32 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("CRC-32 of gzipped data does not match; got {actual_crc32}, expected {crc32}")
                    ));
                }

                if input_size != actual_input_size {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Input size (mod 2^32) does not match;\ngot {actual_input_size}, expected {input_size}")
                    ));
                }

                self.stage = DecodeStage::NewMember;
                Ok(())
            }
            DecodeStage::Complete => Ok(()),
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
