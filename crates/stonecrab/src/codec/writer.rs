//! The capacity-bounded writer.

use std::fmt;

use crate::ByteOrder;

use crate::codec::bits::{coil_byte_count, pack_bits_lsb_first};
use crate::codec::{ParseError, is_little};

/// Generates one multi-byte `write_*` method with its doctest. The arguments are the
/// method name, the type, the example value and its big-endian and little-endian bytes.
macro_rules! write_scalars {
    ($($name:ident, $ty:ty, $value:expr, $big:expr, $little:expr;)*) => {$(
        #[doc = concat!(
            "Appends one `", stringify!($ty), "` encoded in `order`.\n\n",
            "Nothing is written when the write fails.\n\n",
            "# Errors\n\n[`ParseError::OversizedLength`] when the value does not fit in ",
            "the remaining capacity.\n\n",
            "# Examples\n\n```\n",
            "use stonecrab::codec::{BoundedWriter, ParseError};\n",
            "use stonecrab::ByteOrder;\n\n",
            "let mut big = BoundedWriter::new(16);\n",
            "big.", stringify!($name), "(", stringify!($value), ", ByteOrder::BigEndian)?;\n",
            "assert_eq!(big.as_slice(), ", stringify!($big), ");\n",
            "let mut little = BoundedWriter::new(16);\n",
            "little.", stringify!($name), "(", stringify!($value),
            ", ByteOrder::LittleEndian)?;\n",
            "assert_eq!(little.as_slice(), ", stringify!($little), ");\n",
            "assert!(BoundedWriter::new(0).", stringify!($name),
            "(", stringify!($value), ", ByteOrder::BigEndian).is_err());\n",
            "# Ok::<(), ParseError>(())\n```"
        )]
        pub fn $name(&mut self, value: $ty, order: ByteOrder) -> Result<(), ParseError> {
            self.write_bytes(&if is_little(order) {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            })
        }
    )*};
}

/// A byte buffer that refuses to grow past a fixed capacity.
///
/// Every write is all or nothing: a write that would pass the capacity fails with
/// [`ParseError::OversizedLength`] and leaves the buffer unchanged. The buffer is not
/// pre-allocated, so a large capacity costs nothing until bytes are written.
///
/// # Examples
/// ```
/// use stonecrab::codec::{BoundedWriter, ParseError};
/// use stonecrab::ByteOrder;
///
/// let mut writer = BoundedWriter::new(3);
/// writer.write_u16(0x0102, ByteOrder::LittleEndian)?;
/// assert_eq!(writer.remaining_capacity(), 1);
/// assert!(writer.write_u16(0, ByteOrder::BigEndian).is_err());
/// assert_eq!(writer.into_bytes(), [0x02, 0x01]);
/// # Ok::<(), ParseError>(())
/// ```
#[derive(Clone)]
pub struct BoundedWriter {
    buffer: Vec<u8>,
    capacity: usize,
}

impl fmt::Debug for BoundedWriter {
    /// Shows the written length and the capacity, never the written bytes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundedWriter")
            .field("written", &self.buffer.len())
            .field("capacity", &self.capacity)
            .finish()
    }
}

impl BoundedWriter {
    /// Creates an empty writer that accepts at most `capacity` bytes.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::codec::BoundedWriter;
    /// let writer = BoundedWriter::new(253);
    /// assert_eq!(writer.capacity(), 253);
    /// assert!(writer.as_slice().is_empty());
    /// ```
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            buffer: Vec::new(),
            capacity,
        }
    }

    /// The most bytes this writer accepts in total.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many more bytes fit before the capacity is reached.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::codec::BoundedWriter;
    /// let mut writer = BoundedWriter::new(4);
    /// writer.write_u8(1).unwrap();
    /// assert_eq!(writer.remaining_capacity(), 3);
    /// ```
    #[must_use]
    pub const fn remaining_capacity(&self) -> usize {
        self.capacity.saturating_sub(self.buffer.len())
    }

    /// The bytes written so far.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.buffer
    }

    /// Consumes the writer and returns the bytes written.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }

    /// Appends one unsigned byte.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] when the capacity is already reached.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::codec::BoundedWriter;
    /// let mut writer = BoundedWriter::new(1);
    /// writer.write_u8(0xFE).unwrap();
    /// assert!(writer.write_u8(0).is_err());
    /// assert_eq!(writer.as_slice(), [0xFE]);
    /// ```
    pub fn write_u8(&mut self, value: u8) -> Result<(), ParseError> {
        self.write_bytes(&[value])
    }

    /// Appends one signed byte.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] when the capacity is already reached.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::codec::BoundedWriter;
    /// let mut writer = BoundedWriter::new(1);
    /// writer.write_i8(-2).unwrap();
    /// assert_eq!(writer.as_slice(), [0xFE]);
    /// ```
    pub fn write_i8(&mut self, value: i8) -> Result<(), ParseError> {
        self.write_bytes(&value.to_be_bytes())
    }

    write_scalars! {
        write_u16, u16, 0x1234, [0x12, 0x34], [0x34, 0x12];
        write_i16, i16, -2, [0xFF, 0xFE], [0xFE, 0xFF];
        write_u32, u32, 0x1234_5678, [0x12, 0x34, 0x56, 0x78], [0x78, 0x56, 0x34, 0x12];
        write_i32, i32, -2, [0xFF, 0xFF, 0xFF, 0xFE], [0xFE, 0xFF, 0xFF, 0xFF];
        write_u64, u64, 0x0102_0304_0506_0708,
            [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01];
        write_i64, i64, -2,
            [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE],
            [0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        write_f32, f32, 1.5, [0x3F, 0xC0, 0x00, 0x00], [0x00, 0x00, 0xC0, 0x3F];
        write_f64, f64, 1.5,
            [0x3F, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0x3F];
    }

    /// Appends `bytes`.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] when the total length would pass the capacity
    /// (`length` is that total); [`ParseError::Overflow`] when adding the lengths
    /// overflows `usize`. The buffer is unchanged on failure.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::codec::{BoundedWriter, ParseError};
    /// let mut writer = BoundedWriter::new(4);
    /// writer.write_bytes(&[1, 2, 3])?;
    /// assert_eq!(
    ///     writer.write_bytes(&[4, 5]),
    ///     Err(ParseError::OversizedLength { length: 5, max: 4 })
    /// );
    /// assert_eq!(writer.as_slice(), [1, 2, 3]);
    /// # Ok::<(), ParseError>(())
    /// ```
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ParseError> {
        self.check_room(bytes.len())?;
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    /// Appends `bits` packed least significant bit first (Modbus coil packing).
    ///
    /// # Errors
    /// The same as [`write_bytes`](Self::write_bytes), for the packed byte count.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::codec::{BoundedWriter, ParseError};
    /// let mut writer = BoundedWriter::new(2);
    /// writer.write_bits_lsb_first(&[true, true, false, true])?;
    /// assert_eq!(writer.as_slice(), [0b0000_1011]);
    /// # Ok::<(), ParseError>(())
    /// ```
    pub fn write_bits_lsb_first(&mut self, bits: &[bool]) -> Result<(), ParseError> {
        self.check_room(coil_byte_count(bits.len()))?;
        self.write_bytes(&pack_bits_lsb_first(bits))
    }

    /// Fails unless `extra` more bytes fit, before anything is allocated or copied.
    fn check_room(&self, extra: usize) -> Result<(), ParseError> {
        let total = self
            .buffer
            .len()
            .checked_add(extra)
            .ok_or(ParseError::Overflow)?;
        if total > self.capacity {
            return Err(ParseError::OversizedLength {
                length: total,
                max: self.capacity,
            });
        }
        Ok(())
    }
}
