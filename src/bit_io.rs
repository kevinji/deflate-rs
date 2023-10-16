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

    /// When reading, returns `true` if the byte has been fully read.
    /// When writing, returns `true` if the byte-to-write is filled.
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

    fn read<T>(&mut self, slice: &mut BitSlice<T>) -> usize
    where
        T: BitStore,
    {
        let bit_count = self.bits_left().min(slice.len());
        let buffer_bits = &self.byte[self.idx..self.idx + bit_count];

        let bits_to_set = &mut slice[..bit_count];
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
        let byte = self.inner.next().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF when reading next byte",
            )
        })??;
        self.buffer = byte.into();
        Ok(())
    }

    pub fn read_exact<T>(&mut self, mut slice: &mut BitSlice<T>) -> io::Result<()>
    where
        T: BitStore,
    {
        while !slice.is_empty() {
            if self.buffer.needs_flush() {
                self.read_next_byte()?;
            }

            let bit_read_count = self.buffer.read(slice);
            slice = &mut slice[bit_read_count..];
        }

        Ok(())
    }

    pub fn is_eof(&mut self) -> io::Result<bool> {
        if !self.buffer.needs_flush() {
            return Ok(false);
        }

        match self.read_next_byte() {
            Ok(()) => Ok(false),
            Err(e) if matches!(e.kind(), io::ErrorKind::UnexpectedEof) => Ok(true),
            Err(e) => Err(e),
        }
    }

    pub fn read_bool(&mut self) -> io::Result<bool> {
        let arr = bits![mut u8, Lsb0; 0; 1];
        self.read_exact(arr)?;
        Ok(arr[0])
    }

    pub fn read_u8_from_bits(&mut self, bit_count: usize) -> io::Result<u8> {
        assert!(bit_count <= 8);
        let mut bv = <BitVec<u8>>::with_capacity(bit_count);
        bv.resize(bit_count, false);

        self.read_exact(bv.as_mut_bitslice())?;
        Ok(bv.load_le::<u8>())
    }

    pub fn read_u8(&mut self) -> io::Result<u8> {
        self.read_u8_from_bits(8)
    }

    pub fn read_u16_from_bits(&mut self, bit_count: usize) -> io::Result<u16> {
        assert!(bit_count <= 16);
        let mut bv = <BitVec<u16>>::with_capacity(bit_count);
        bv.resize(bit_count, false);

        self.read_exact(bv.as_mut_bitslice())?;
        Ok(bv.load_le::<u16>())
    }

    pub fn read_u16(&mut self) -> io::Result<u16> {
        self.read_u16_from_bits(16)
    }

    pub fn read_u32_from_bits(&mut self, bit_count: usize) -> io::Result<u32> {
        assert!(bit_count <= 32);
        let mut bv = <BitVec<u32>>::with_capacity(bit_count);
        bv.resize(bit_count, false);

        self.read_exact(bv.as_mut_bitslice())?;
        Ok(bv.load_le::<u32>())
    }

    pub fn read_u32(&mut self) -> io::Result<u32> {
        self.read_u32_from_bits(32)
    }

    pub fn skip_to_byte_end(&mut self) {
        self.buffer.idx = <ByteBuffer<R>>::BITS;
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

    fn write<T>(&mut self, slice: &BitSlice<T>) -> usize
    where
        T: BitStore,
    {
        let bit_count = self.bits_left().min(slice.len());
        let buffer_bits = &mut self.byte[self.idx..self.idx + bit_count];

        let bits_to_get = &slice[..bit_count];
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
    pub fn flush_even_if_partial(&mut self) -> io::Result<()> {
        if self.buffer.idx == 0 {
            return Ok(());
        }

        let buffer = mem::replace(&mut self.buffer, ByteBuffer::new_write());
        self.inner.write_all(&[buffer.into()])?;
        Ok(())
    }

    pub fn write_all<T>(&mut self, mut slice: &BitSlice<T>) -> io::Result<()>
    where
        T: BitStore,
    {
        while !slice.is_empty() {
            let bit_write_count = self.buffer.write(slice);
            slice = &slice[bit_write_count..];

            if self.buffer.needs_flush() {
                self.flush_even_if_partial()?;
            }
        }

        Ok(())
    }

    pub fn write_u8(&mut self, byte: u8) -> io::Result<()> {
        self.write_all(byte.view_bits::<Lsb0>())
    }
}
