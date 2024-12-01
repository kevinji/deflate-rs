use std::io;

#[derive(Debug)]
pub struct OutWithChecksum<'a, O> {
    out: &'a mut O,
    size: u32,
    crc_hasher: crc32fast::Hasher,
}

impl<'a, O> OutWithChecksum<'a, O> {
    pub fn new(out: &'a mut O) -> Self {
        Self {
            out,
            size: 0,
            crc_hasher: crc32fast::Hasher::new(),
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn crc32(&self) -> u32 {
        self.crc_hasher.clone().finalize()
    }
}

impl<O> io::Write for OutWithChecksum<'_, O>
where
    O: io::Write,
{
    #[allow(clippy::cast_possible_truncation)]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let bytes = self.out.write(buf)?;
        self.crc_hasher.update(&buf[..bytes]);
        self.size = self.size.wrapping_add(bytes as u32);
        Ok(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
