use crate::bit_io::BitReader;
use std::io;

#[derive(Debug, Eq, PartialEq)]
pub enum CompressionType {
    None,
    FixedHuffman,
    DynamicHuffman,
}

#[derive(Debug)]
pub struct Decompressor<R> {
    reader: BitReader<R>,
}

impl<R> Decompressor<R>
where
    R: io::Read,
{
    pub fn new(reader: R) -> Self {
        Self {
            reader: BitReader::new(reader),
        }
    }
}
