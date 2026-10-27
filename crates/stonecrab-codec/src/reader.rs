//! The bounds-checked reader.

use std::fmt;

use stonecrab_core::ByteOrder;

use crate::bits::{coil_byte_count, unpack_bits_lsb_first};
use crate::{ParseError, is_little};

/// Generates one multi-byte `read_*` method with its doctest. The arguments are the
/// method name, the type, its width in bytes, the big-endian and little-endian bytes of
/// the example value, and the example value.
macro_rules! read_scalars {
    ($($name:ident, $ty:ty, $width:literal, $big:expr, $little:expr, $value:expr;)*) => {$(
        #[doc = concat!(
            "Reads one `", stringify!($ty), "` encoded in `order`.\n\n",
            "Nothing is consumed when the read fails.\n\n",
            "# Errors\n\n[`ParseError::Truncated`] when fewer than ", $width,
            " bytes remain.\n\n",
            "# Examples\n\n```\n",
            "use stonecrab_codec::{BoundedReader, ParseError};\n",
            "use stonecrab_core::ByteOrder;\n\n",
            "let mut big = BoundedReader::new(&", stringify!($big), ");\n",
            "assert_eq!(big.", stringify!($name), "(ByteOrder::BigEndian)?, ",
            stringify!($value), ");\n",
            "let mut little = BoundedReader::new(&", stringify!($little), ");\n",
            "assert_eq!(little.", stringify!($name), "(ByteOrder::LittleEndian)?, ",
            stringify!($value), ");\n",
            "assert!(little.", stringify!($name), "(ByteOrder::BigEndian).is_err());\n",
            "# Ok::<(), ParseError>(())\n```"
        )]
        pub fn $name(&mut self, order: ByteOrder) -> Result<$ty, ParseError> {
            let bytes = self.take_array()?;
            Ok(if is_little(order) {
                <$ty>::from_le_bytes(bytes)
            } else {
                <$ty>::from_be_bytes(bytes)
            })
        }
    )*};
}

/// A cursor over a byte slice whose every read is bounds-checked.
///
/// The reader borrows its input and never copies it: [`read_bytes`](Self::read_bytes)
/// returns a sub-slice. A failed read leaves the cursor where it was. Lengths that come
/// from the wire are compared with a caller-supplied maximum and with the bytes that
/// remain before anything is allocated. No method panics on any input.
///
/// # Examples
/// ```
/// use stonecrab_codec::{BoundedReader, ParseError};
/// use stonecrab_core::ByteOrder;
///
/// let mut reader = BoundedReader::new(&[0x00, 0x2A, 0xFF]);
/// assert_eq!(reader.read_u16(ByteOrder::BigEndian)?, 42);
/// assert_eq!(reader.remaining(), 1);
/// assert_eq!(reader.read_i8()?, -1);
/// reader.expect_eof()?;
/// # Ok::<(), ParseError>(())
/// ```
#[derive(Clone)]
pub struct BoundedReader<'a> {
    data: &'a [u8],
}

impl fmt::Debug for BoundedReader<'_> {
    /// Shows the number of unread bytes, never their contents.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundedReader")
            .field("remaining", &self.data.len())
            .finish()
    }
}

impl<'a> BoundedReader<'a> {
    /// Starts reading at the first byte of `data`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_codec::BoundedReader;
    /// assert_eq!(BoundedReader::new(&[1, 2, 3]).remaining(), 3);
    /// ```
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Number of bytes not yet read.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_codec::BoundedReader;
    /// let mut reader = BoundedReader::new(&[1, 2]);
    /// assert_eq!(reader.remaining(), 2);
    /// reader.read_u8().unwrap();
    /// assert_eq!(reader.remaining(), 1);
    /// ```
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.data.len()
    }

    /// Takes the next `len` bytes, or fails without moving the cursor.
    const fn take(&mut self, len: usize) -> Result<&'a [u8], ParseError> {
        if let Some((head, tail)) = self.data.split_at_checked(len) {
            self.data = tail;
            Ok(head)
        } else {
            Err(ParseError::Truncated {
                needed: len,
                available: self.data.len(),
            })
        }
    }

    /// Takes the next `N` bytes as an array, or fails without moving the cursor.
    const fn take_array<const N: usize>(&mut self) -> Result<[u8; N], ParseError> {
        if let Some((head, tail)) = self.data.split_first_chunk::<N>() {
            self.data = tail;
            Ok(*head)
        } else {
            Err(ParseError::Truncated {
                needed: N,
                available: self.data.len(),
            })
        }
    }

    /// Reads one unsigned byte.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when no byte remains.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_codec::{BoundedReader, ParseError};
    /// assert_eq!(BoundedReader::new(&[0xFE]).read_u8()?, 0xFE);
    /// assert!(BoundedReader::new(&[]).read_u8().is_err());
    /// # Ok::<(), ParseError>(())
    /// ```
    pub fn read_u8(&mut self) -> Result<u8, ParseError> {
        let [byte] = self.take_array()?;
        Ok(byte)
    }

    /// Reads one signed byte.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when no byte remains.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_codec::{BoundedReader, ParseError};
    /// assert_eq!(BoundedReader::new(&[0xFE]).read_i8()?, -2);
    /// assert!(BoundedReader::new(&[]).read_i8().is_err());
    /// # Ok::<(), ParseError>(())
    /// ```
    pub fn read_i8(&mut self) -> Result<i8, ParseError> {
        Ok(i8::from_be_bytes(self.take_array()?))
    }

    read_scalars! {
        read_u16, u16, 2, [0x12, 0x34], [0x34, 0x12], 0x1234;
        read_i16, i16, 2, [0xFF, 0xFE], [0xFE, 0xFF], -2;
        read_u32, u32, 4, [0x12, 0x34, 0x56, 0x78], [0x78, 0x56, 0x34, 0x12], 0x1234_5678;
        read_i32, i32, 4, [0xFF, 0xFF, 0xFF, 0xFE], [0xFE, 0xFF, 0xFF, 0xFF], -2;
        read_u64, u64, 8,
            [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01],
            0x0102_0304_0506_0708;
        read_i64, i64, 8,
            [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE],
            [0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            -2;
        read_f32, f32, 4, [0x3F, 0xC0, 0x00, 0x00], [0x00, 0x00, 0xC0, 0x3F], 1.5;
        read_f64, f64, 8,
            [0x3F, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0x3F],
            1.5;
    }

    /// Reads `len` bytes and returns them as a sub-slice of the input.
    ///
    /// `len` is compared with `max` and with the remaining input before any byte is
    /// touched, so a hostile length field costs nothing. No allocation is made.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] when `len` exceeds `max`;
    /// [`ParseError::Truncated`] when fewer than `len` bytes remain.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_codec::{BoundedReader, ParseError};
    /// let mut reader = BoundedReader::new(&[1, 2, 3, 4]);
    /// assert_eq!(reader.read_bytes(3, 8)?, [1, 2, 3]);
    /// assert_eq!(
    ///     reader.read_bytes(2, 1),
    ///     Err(ParseError::OversizedLength { length: 2, max: 1 })
    /// );
    /// # Ok::<(), ParseError>(())
    /// ```
    pub const fn read_bytes(&mut self, len: usize, max: usize) -> Result<&'a [u8], ParseError> {
        if len > max {
            return Err(ParseError::OversizedLength { length: len, max });
        }
        self.take(len)
    }

    /// Reads `count` bits packed least significant bit first (Modbus coil packing).
    ///
    /// Bit 0 of the first byte is the first element. This consumes
    /// [`coil_byte_count`]`(count)` bytes; padding bits in the last byte are ignored.
    /// `count` is checked against `max` and the byte count against the remaining input
    /// before the result is allocated.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] when `count` exceeds `max`;
    /// [`ParseError::Truncated`] when the packed bytes are not all present.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_codec::{BoundedReader, ParseError};
    /// let mut reader = BoundedReader::new(&[0b0000_0101]);
    /// assert_eq!(reader.read_bits_lsb_first(3, 2000)?, [true, false, true]);
    /// reader.expect_eof()?;
    /// # Ok::<(), ParseError>(())
    /// ```
    pub fn read_bits_lsb_first(
        &mut self,
        count: usize,
        max: usize,
    ) -> Result<Vec<bool>, ParseError> {
        if count > max {
            return Err(ParseError::OversizedLength { length: count, max });
        }
        let packed = self.take(coil_byte_count(count))?;
        unpack_bits_lsb_first(packed, count)
    }

    /// Succeeds only when every byte has been read.
    ///
    /// # Errors
    /// [`ParseError::TrailingBytes`] with the number of unread bytes.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_codec::{BoundedReader, ParseError};
    /// let mut reader = BoundedReader::new(&[1, 2]);
    /// reader.read_u8()?;
    /// assert_eq!(reader.expect_eof(), Err(ParseError::TrailingBytes { remaining: 1 }));
    /// reader.read_u8()?;
    /// assert_eq!(reader.expect_eof(), Ok(()));
    /// # Ok::<(), ParseError>(())
    /// ```
    pub const fn expect_eof(&self) -> Result<(), ParseError> {
        if self.data.is_empty() {
            Ok(())
        } else {
            Err(ParseError::TrailingBytes {
                remaining: self.data.len(),
            })
        }
    }
}
