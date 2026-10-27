//! The PDU value type.

use crate::codec::{ParseError, coil_byte_count};

use super::{ExceptionCode, FunctionCode};

/// One Modbus PDU: a read request, a read response or an exception response.
///
/// Addresses are the zero-based numbers that go on the wire, not the one-based logical
/// addresses of the text grammar (`ModbusAddress::offset` is the wire number). A response
/// does not repeat the request's quantity, so a bit response holds the packed bytes
/// exactly as received, padding bits included: unpack them with
/// [`unpack_bits_lsb_first`](crate::codec::unpack_bits_lsb_first) and the quantity of
/// the request, after [`check_reply_to`](Self::check_reply_to) has proved the byte count.
/// Write PDUs have no variant (decision D3).
///
/// # Examples
/// ```
/// use stonecrab::modbus::{FunctionCode, ModbusPdu};
///
/// // Read three holding registers from wire address 107 (logical 108).
/// let request = ModbusPdu::ReadHoldingRegisters { start: 107, quantity: 3 };
/// assert_eq!(request.function(), FunctionCode::ReadHoldingRegisters);
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-HR
/// The request variants are the upstream `ModbusPDUReadCoilsRequest`
/// (`MB-TCP-READ-COIL`), `ModbusPDUReadDiscreteInputsRequest` (`MB-TCP-READ-DI`),
/// `ModbusPDUReadHoldingRegistersRequest` (`MB-TCP-READ-HR`) and
/// `ModbusPDUReadInputRegistersRequest` (`MB-TCP-READ-IR`), each with the quantity limits
/// `MB-LIMIT-COILS` and `MB-LIMIT-REGISTERS`; the response variants are their responses
/// and `Exception` is the error PDU (rows `MB-EXC-*`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ModbusPdu {
    /// Function 0x01: read `quantity` coils from `start`.
    ReadCoils {
        /// Wire address of the first coil.
        start: u16,
        /// Number of coils, 1 to 2000.
        quantity: u16,
    },
    /// Function 0x02: read `quantity` discrete inputs from `start`.
    ReadDiscreteInputs {
        /// Wire address of the first input.
        start: u16,
        /// Number of inputs, 1 to 2000.
        quantity: u16,
    },
    /// Function 0x03: read `quantity` holding registers from `start`.
    ReadHoldingRegisters {
        /// Wire address of the first register.
        start: u16,
        /// Number of registers, 1 to 125.
        quantity: u16,
    },
    /// Function 0x04: read `quantity` input registers from `start`.
    ReadInputRegisters {
        /// Wire address of the first register.
        start: u16,
        /// Number of registers, 1 to 125.
        quantity: u16,
    },
    /// The normal response to function 0x01 or 0x02.
    ReadBitsResponse {
        /// `ReadCoils` or `ReadDiscreteInputs`.
        function: FunctionCode,
        /// The bits packed least significant bit first, 1 to 250 bytes, as on the wire.
        bytes: Vec<u8>,
    },
    /// The normal response to function 0x03 or 0x04.
    ReadRegistersResponse {
        /// `ReadHoldingRegisters` or `ReadInputRegisters`.
        function: FunctionCode,
        /// The register values in wire order, 1 to 125 of them.
        registers: Vec<u16>,
    },
    /// An exception response: the function with its high bit set, then a code.
    Exception {
        /// The function the device refused (without the high bit).
        function: FunctionCode,
        /// Why it refused.
        code: ExceptionCode,
    },
}

impl ModbusPdu {
    /// The function this PDU belongs to: the request's function, the response's function,
    /// or for an exception the function that was refused (without the high bit).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{ExceptionCode, FunctionCode, ModbusPdu};
    ///
    /// let refusal = ModbusPdu::Exception {
    ///     function: FunctionCode::ReadCoils,
    ///     code: ExceptionCode::IllegalDataAddress,
    /// };
    /// assert_eq!(refusal.function(), FunctionCode::ReadCoils);
    /// ```
    #[must_use]
    pub const fn function(&self) -> FunctionCode {
        match self {
            Self::ReadCoils { .. } => FunctionCode::ReadCoils,
            Self::ReadDiscreteInputs { .. } => FunctionCode::ReadDiscreteInputs,
            Self::ReadHoldingRegisters { .. } => FunctionCode::ReadHoldingRegisters,
            Self::ReadInputRegisters { .. } => FunctionCode::ReadInputRegisters,
            Self::ReadBitsResponse { function, .. }
            | Self::ReadRegistersResponse { function, .. }
            | Self::Exception { function, .. } => *function,
        }
    }

    /// The `(start, quantity)` of a read request, `None` for a response.
    pub(crate) const fn request_span(&self) -> Option<(u16, u16)> {
        match self {
            Self::ReadCoils { start, quantity }
            | Self::ReadDiscreteInputs { start, quantity }
            | Self::ReadHoldingRegisters { start, quantity }
            | Self::ReadInputRegisters { start, quantity } => Some((*start, *quantity)),
            Self::ReadBitsResponse { .. }
            | Self::ReadRegistersResponse { .. }
            | Self::Exception { .. } => None,
        }
    }

    /// Checks that `self`, a response, answers `request`, which the response itself cannot
    /// prove: the function must be the request's, and the data must be exactly as long as
    /// the request's quantity (ceil(quantity / 8) bytes for bits, `quantity` registers).
    /// An exception only has to name the request's function.
    ///
    /// # Errors
    /// [`ParseError::InvalidDiscriminator`] when `request` is not a read request, when
    /// `self` is not a response, or when the function differs (the `value` is the
    /// offending function number); [`ParseError::Truncated`] when the data is shorter than
    /// the quantity needs and [`ParseError::TrailingBytes`] when it is longer.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{FunctionCode, ModbusPdu};
    ///
    /// let request = ModbusPdu::ReadCoils { start: 19, quantity: 19 };
    /// let reply = ModbusPdu::ReadBitsResponse {
    ///     function: FunctionCode::ReadCoils,
    ///     bytes: vec![0xCD, 0x6B, 0x05],
    /// };
    /// assert!(reply.check_reply_to(&request).is_ok());
    ///
    /// let short = ModbusPdu::ReadBitsResponse {
    ///     function: FunctionCode::ReadCoils,
    ///     bytes: vec![0xCD, 0x6B],
    /// };
    /// assert!(short.check_reply_to(&request).is_err());
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-READ-MATCH
    /// Upstream compares nothing but the transaction id (`MB-READ-MATCH`); the function
    /// and length checks are the divergence recorded there. The transaction and unit
    /// checks belong to the session (S24b).
    pub fn check_reply_to(&self, request: &Self) -> Result<(), ParseError> {
        let Some((_, quantity)) = request.request_span() else {
            return Err(discriminator("request function code", request.function()));
        };
        if self.function() != request.function() {
            return Err(discriminator("function code", self.function()));
        }
        match self {
            Self::ReadBitsResponse { bytes, .. } if request_is_bits(request) => {
                check_length(coil_byte_count(usize::from(quantity)), bytes.len())
            }
            Self::ReadRegistersResponse { registers, .. } if !request_is_bits(request) => {
                check_length(
                    usize::from(quantity).saturating_mul(2),
                    registers.len().saturating_mul(2),
                )
            }
            Self::Exception { .. } => Ok(()),
            Self::ReadCoils { .. }
            | Self::ReadDiscreteInputs { .. }
            | Self::ReadHoldingRegisters { .. }
            | Self::ReadInputRegisters { .. }
            | Self::ReadBitsResponse { .. }
            | Self::ReadRegistersResponse { .. } => {
                Err(discriminator("response function code", self.function()))
            }
        }
    }
}

/// True for the two requests whose responses carry bits.
const fn request_is_bits(request: &ModbusPdu) -> bool {
    matches!(
        request,
        ModbusPdu::ReadCoils { .. } | ModbusPdu::ReadDiscreteInputs { .. }
    )
}

/// The error for a function code that is not acceptable where it was found.
pub(super) fn discriminator(field: &'static str, function: FunctionCode) -> ParseError {
    ParseError::InvalidDiscriminator {
        field,
        value: u64::from(function.as_u8()),
    }
}

/// Compares the data length a request implies with the one a response has.
const fn check_length(needed: usize, available: usize) -> Result<(), ParseError> {
    if available < needed {
        Err(ParseError::Truncated { needed, available })
    } else if available > needed {
        Err(ParseError::TrailingBytes {
            remaining: available.abs_diff(needed),
        })
    } else {
        Ok(())
    }
}
