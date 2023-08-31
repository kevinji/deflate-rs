use bitvec::prelude::*;
use core::{marker::PhantomData, mem};
use std::io;

/// Phantom type representing a buffer to read from.
#[derive(Debug)]
struct ReadBuffer;

/// Phantom type representing a buffer to write to.
#[derive(Debug)]
struct WriteBuffer;

/// `ByteBuffer` is a buffer consisting of one in-progress byte that is either
/// being read or written.
///
/// The behavior is slightly different depending on whether a read or write is
/// happening:
/// - If reading, `idx` will refer to the next bit to read from.
/// - If writing, `idx` will refer to the next bit to write into.
///
/// As a result, during reading, `byte` will always contain a full byte of data,
/// while during writing, `byte` will start out incomplete and be filled over
/// time.
#[derive(Debug)]
struct ByteBuffer<T> {
    _kind: PhantomData<T>,
    byte: BitArray<[u8; 1]>,
    idx: usize,
}

impl<T> ByteBuffer<T> {
    const BITS: usize = 8;

    fn bits_left(&self) -> usize {
        Self::BITS - self.idx
    }

    fn needs_flush(&self) -> bool {
        self.idx == Self::BITS
    }
}

impl ByteBuffer<ReadBuffer> {
    fn new_read() -> Self {
        Self {
            _kind: PhantomData,
            byte: BitArray::ZERO,
            idx: Self::BITS,
        }
    }

    fn read<T>(&mut self, bits: &mut BitSlice<T>) -> usize
    where
        T: BitStore,
    {
        let bit_count = self.bits_left().min(bits.len());
        let buffer_bits = self.byte.get(self.idx..self.idx + bit_count).unwrap();

        let bits_to_set = bits.get_mut(..bit_count).unwrap();
        bits_to_set.clone_from_bitslice(buffer_bits);

        self.idx += bit_count;
        bit_count
    }
}

impl From<u8> for ByteBuffer<ReadBuffer> {
    fn from(byte: u8) -> Self {
        Self {
            _kind: PhantomData,
            byte: [byte].into(),
            idx: 0,
        }
    }
}

#[derive(Debug)]
pub struct BitReader<R> {
    buffer: ByteBuffer<ReadBuffer>,
    inner: io::Bytes<R>,
}

impl<R> BitReader<R>
where
    R: io::Read,
{
    pub fn new(inner: R) -> Self {
        Self {
            buffer: ByteBuffer::new_read(),
            inner: inner.bytes(),
        }
    }

    /// Precondition: `self.buffer.needs_flush()`
    fn read_next_byte(&mut self) -> io::Result<()> {
        let byte = self
            .inner
            .next()
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))??;
        self.buffer = byte.into();
        Ok(())
    }

    pub fn read_exact<T>(&mut self, mut bits: &mut BitSlice<T>) -> io::Result<()>
    where
        T: BitStore,
    {
        while bits.len() > 0 {
            if self.buffer.needs_flush() {
                self.read_next_byte()?;
            }

            let bit_read_count = self.buffer.read(bits);
            bits = bits.get_mut(bit_read_count..).unwrap();
        }

        Ok(())
    }
}

impl ByteBuffer<WriteBuffer> {
    fn new_write() -> Self {
        Self {
            _kind: PhantomData,
            byte: BitArray::ZERO,
            idx: 0,
        }
    }

    fn write<T>(&mut self, bits: &BitSlice<T>) -> usize
    where
        T: BitStore,
    {
        let bit_count = self.bits_left().min(bits.len());
        let buffer_bits = self.byte.get_mut(self.idx..self.idx + bit_count).unwrap();

        let bits_to_get = bits.get(..bit_count).unwrap();
        buffer_bits.clone_from_bitslice(bits_to_get);

        self.idx += bit_count;
        bit_count
    }
}

impl From<ByteBuffer<WriteBuffer>> for u8 {
    fn from(ByteBuffer { byte, .. }: ByteBuffer<WriteBuffer>) -> Self {
        let [byte] = byte.into_inner();
        byte
    }
}

#[derive(Debug)]
pub struct BitWriter<W> {
    buffer: ByteBuffer<WriteBuffer>,
    inner: W,
}

impl<W> BitWriter<W>
where
    W: io::Write,
{
    pub fn new(inner: W) -> Self {
        Self {
            buffer: ByteBuffer::new_write(),
            inner,
        }
    }

    /// Flushes the current byte. If the byte has not been fully written to, it
    /// will be padded with zeros.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.buffer.idx == 0 {
            return Ok(());
        }

        let buffer = mem::replace(&mut self.buffer, ByteBuffer::new_write());
        self.inner.write_all(&[buffer.into()])?;
        Ok(())
    }

    pub fn write_all<T>(&mut self, mut bits: &mut BitSlice<T>) -> io::Result<()>
    where
        T: BitStore,
    {
        while bits.len() > 0 {
            if self.buffer.needs_flush() {
                self.flush()?;
            }

            let bit_write_count = self.buffer.write(bits);
            bits = bits.get_mut(bit_write_count..).unwrap();
        }

        Ok(())
    }
}
