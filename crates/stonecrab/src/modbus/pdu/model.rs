//! The PDU value type.

use crate::codec::{ParseError, coil_byte_count};
use crate::modbus::ModbusArea;

use super::codec::check_request;
use super::{ExceptionCode, FunctionCode, MAX_WRITE_COILS, MAX_WRITE_REGISTERS};

/// One Modbus PDU: a read or write request, its normal response or an exception response.
///
/// Addresses are the zero-based numbers that go on the wire, not the one-based logical
/// addresses of the text grammar (`ModbusAddress::offset` is the wire number). A response
/// does not repeat the request's quantity, so a bit response holds the packed bytes
/// exactly as received, padding bits included: unpack them with
/// [`unpack_bits_lsb_first`](crate::codec::unpack_bits_lsb_first) and the quantity of
/// the request, after [`check_reply_to`](Self::check_reply_to) has proved the byte count.
/// A write request holds its data, so its quantity and byte count are facts of the value,
/// not separate fields: a register write has as many registers as its quantity, and a coil
/// write keeps the quantity (the bits do not say how many of them count) with exactly
/// `ceil(quantity / 8)` bytes. Function code 05 has no variant (`MB-QUIRK-C`).
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
/// and `Exception` is the error PDU (rows `MB-EXC-*`). The write variants are the PDUs
/// upstream builds for `MB-TCP-WRITE`: `ModbusPDUWriteMultipleCoilsRequest`,
/// `ModbusPDUWriteSingleRegisterRequest` and
/// `ModbusPDUWriteMultipleHoldingRegistersRequest` with their responses.
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
    /// Function 0x0F: write `quantity` coils from `start`, the first of them the least
    /// significant bit of the first byte.
    WriteMultipleCoils {
        /// Wire address of the first coil.
        start: u16,
        /// Number of coils, 1 to 1968.
        quantity: u16,
        /// The bits packed least significant bit first, `ceil(quantity / 8)` bytes, as on
        /// the wire.
        bytes: Vec<u8>,
    },
    /// Function 0x06: write one register.
    WriteSingleRegister {
        /// Wire address of the register.
        address: u16,
        /// The value to write.
        value: u16,
    },
    /// Function 0x10: write the `registers` from `start`.
    WriteMultipleRegisters {
        /// Wire address of the first register.
        start: u16,
        /// The values in wire order, 1 to 123 of them.
        registers: Vec<u16>,
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
    /// The normal response to function 0x06: the request's address and value, echoed.
    WriteSingleRegisterResponse {
        /// The address of the request.
        address: u16,
        /// The value of the request.
        value: u16,
    },
    /// The normal response to function 0x0F or 0x10: the request's start and quantity,
    /// echoed.
    WriteMultipleResponse {
        /// `WriteMultipleCoils` or `WriteMultipleRegisters`.
        function: FunctionCode,
        /// The start address of the request.
        start: u16,
        /// The quantity of the request.
        quantity: u16,
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
            Self::WriteMultipleCoils { .. } => FunctionCode::WriteMultipleCoils,
            Self::WriteSingleRegister { .. } | Self::WriteSingleRegisterResponse { .. } => {
                FunctionCode::WriteSingleRegister
            }
            Self::WriteMultipleRegisters { .. } => FunctionCode::WriteMultipleRegisters,
            Self::ReadBitsResponse { function, .. }
            | Self::ReadRegistersResponse { function, .. }
            | Self::WriteMultipleResponse { function, .. }
            | Self::Exception { function, .. } => *function,
        }
    }

    /// Whether this value is a request (a read request or a write request).
    const fn is_request(&self) -> bool {
        matches!(
            self,
            Self::ReadCoils { .. }
                | Self::ReadDiscreteInputs { .. }
                | Self::ReadHoldingRegisters { .. }
                | Self::ReadInputRegisters { .. }
                | Self::WriteMultipleCoils { .. }
                | Self::WriteSingleRegister { .. }
                | Self::WriteMultipleRegisters { .. }
        )
    }

    /// Picks the write request for `payload`, the way upstream does: coils always go in
    /// Write Multiple Coils (function 15, even a single coil), and a holding-register
    /// payload of exactly 2 bytes goes in Write Single Register (function 06) while any
    /// other goes in Write Multiple Registers (function 16).
    ///
    /// `quantity` is the number of coils or registers, `start` the zero-based wire address
    /// and `payload` the data already encoded: coil bits packed least significant bit
    /// first in `ceil(quantity / 8)` bytes (padding bits are sent as given), or
    /// `2 * quantity` register bytes in wire order. The result is always one that
    /// [`encode_request`](crate::modbus::encode_request) accepts. The function chooses a
    /// PDU and sends nothing.
    ///
    /// # Errors
    /// [`ParseError::InvalidDiscriminator`] when the area is read-only (field
    /// `write area`, the value its prefix digit) or the quantity is 0;
    /// [`ParseError::OversizedLength`] when the quantity is over 1968 coils or 123
    /// registers or `start + quantity` passes the address space; [`ParseError::Truncated`]
    /// or [`ParseError::TrailingBytes`] when the payload is not exactly the length the
    /// quantity implies.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{ModbusArea, ModbusPdu};
    ///
    /// // A 2-byte payload is one register: function 06.
    /// let single = ModbusPdu::write_request(ModbusArea::HoldingRegister, 1, 1, &[0x00, 0x03])?;
    /// assert_eq!(single, ModbusPdu::WriteSingleRegister { address: 1, value: 3 });
    ///
    /// // Two registers are function 16.
    /// let several = ModbusPdu::write_request(
    ///     ModbusArea::HoldingRegister,
    ///     1,
    ///     2,
    ///     &[0x00, 0x0A, 0x01, 0x02],
    /// )?;
    /// assert_eq!(
    ///     several,
    ///     ModbusPdu::WriteMultipleRegisters { start: 1, registers: vec![0x000A, 0x0102] }
    /// );
    ///
    /// // A single coil is still function 15.
    /// let coil = ModbusPdu::write_request(ModbusArea::Coil, 172, 1, &[0x01])?;
    /// assert_eq!(
    ///     coil,
    ///     ModbusPdu::WriteMultipleCoils { start: 172, quantity: 1, bytes: vec![0x01] }
    /// );
    ///
    /// // An input register cannot be written.
    /// assert!(ModbusPdu::write_request(ModbusArea::InputRegister, 0, 1, &[0, 1]).is_err());
    /// # Ok::<(), stonecrab::codec::ParseError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-WRITE
    /// The rule is upstream's `getWriteRequestPdu` (Conn:438-465). Upstream picks function
    /// 06 whenever the payload is 2 bytes whatever the tag's register count and checks the
    /// lengths afterwards; this function requires the payload to fit the quantity first,
    /// so a 2-byte payload for 2 registers is a length error, not a single write. Function
    /// 05 is never chosen (`MB-QUIRK-C`).
    pub fn write_request(
        area: ModbusArea,
        start: u16,
        quantity: u16,
        payload: &[u8],
    ) -> Result<Self, ParseError> {
        match area {
            ModbusArea::Coil => {
                check_request(start, quantity, MAX_WRITE_COILS)?;
                check_length(coil_byte_count(usize::from(quantity)), payload.len())?;
                Ok(Self::WriteMultipleCoils {
                    start,
                    quantity,
                    bytes: payload.to_vec(),
                })
            }
            ModbusArea::HoldingRegister => {
                check_request(start, quantity, MAX_WRITE_REGISTERS)?;
                check_length(usize::from(quantity).saturating_mul(2), payload.len())?;
                let registers: Vec<u16> = payload
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_be_bytes(*pair))
                    .collect();
                Ok(match registers.as_slice() {
                    [value] => Self::WriteSingleRegister {
                        address: start,
                        value: *value,
                    },
                    _ => Self::WriteMultipleRegisters { start, registers },
                })
            }
            ModbusArea::DiscreteInput | ModbusArea::InputRegister => {
                Err(ParseError::InvalidDiscriminator {
                    field: "write area",
                    value: u64::from(area.prefix_digit()),
                })
            }
        }
    }

    /// Checks that `self`, a response, answers `request`, which the response itself cannot
    /// prove: the function must be the request's, and the data must be exactly as long as
    /// the request's quantity (ceil(quantity / 8) bytes for bits, `quantity` registers)
    /// for a read, or echo the request's address and quantity (address and value for
    /// function 06) for a write. An exception only has to name the request's function.
    ///
    /// # Errors
    /// [`ParseError::InvalidDiscriminator`] when `request` is not a request, when `self`
    /// is not a response, when the function differs (the `value` is the offending function
    /// number) or when a write response echoes the wrong value (the field is `echoed start
    /// address`, `echoed quantity` or `echoed value` and the `value` is what the response
    /// holds; the address is judged first); [`ParseError::Truncated`] when read data is
    /// shorter than the quantity needs and [`ParseError::TrailingBytes`] when it is longer.
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
    ///
    /// let write = ModbusPdu::WriteMultipleRegisters { start: 1, registers: vec![10, 258] };
    /// let echo = ModbusPdu::WriteMultipleResponse {
    ///     function: FunctionCode::WriteMultipleRegisters,
    ///     start: 1,
    ///     quantity: 2,
    /// };
    /// assert!(echo.check_reply_to(&write).is_ok());
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-READ-MATCH
    /// Upstream compares nothing but the transaction id (`MB-READ-MATCH`); the function
    /// and length checks are the divergence recorded there. Upstream's only echo check
    /// is the function 05 branch that never runs (`MB-QUIRK-C`); the write echo checks
    /// here are Stonecrab's own (`MB-TCP-WRITE`). The transaction and unit checks belong
    /// to the session (S24b).
    pub fn check_reply_to(&self, request: &Self) -> Result<(), ParseError> {
        if !request.is_request() {
            return Err(discriminator("request function code", request.function()));
        }
        if self.function() != request.function() {
            return Err(discriminator("function code", self.function()));
        }
        match (request, self) {
            (_, Self::Exception { .. }) => Ok(()),
            (
                Self::ReadCoils { quantity, .. } | Self::ReadDiscreteInputs { quantity, .. },
                Self::ReadBitsResponse { bytes, .. },
            ) => check_length(coil_byte_count(usize::from(*quantity)), bytes.len()),
            (
                Self::ReadHoldingRegisters { quantity, .. }
                | Self::ReadInputRegisters { quantity, .. },
                Self::ReadRegistersResponse { registers, .. },
            ) => check_length(
                usize::from(*quantity).saturating_mul(2),
                registers.len().saturating_mul(2),
            ),
            (
                Self::WriteSingleRegister { address, value },
                Self::WriteSingleRegisterResponse {
                    address: echoed_address,
                    value: echoed_value,
                },
            ) => check_echo(
                "echoed start address",
                usize::from(*address),
                *echoed_address,
            )
            .and_then(|()| check_echo("echoed value", usize::from(*value), *echoed_value)),
            (
                Self::WriteMultipleCoils {
                    start, quantity, ..
                },
                Self::WriteMultipleResponse {
                    start: echoed_start,
                    quantity: echoed_quantity,
                    ..
                },
            ) => check_echo("echoed start address", usize::from(*start), *echoed_start).and_then(
                |()| check_echo("echoed quantity", usize::from(*quantity), *echoed_quantity),
            ),
            (
                Self::WriteMultipleRegisters { start, registers },
                Self::WriteMultipleResponse {
                    start: echoed_start,
                    quantity: echoed_quantity,
                    ..
                },
            ) => check_echo("echoed start address", usize::from(*start), *echoed_start)
                .and_then(|()| check_echo("echoed quantity", registers.len(), *echoed_quantity)),
            _ => Err(discriminator("response function code", self.function())),
        }
    }
}

/// The error for a function code that is not acceptable where it was found.
pub(super) fn discriminator(field: &'static str, function: FunctionCode) -> ParseError {
    ParseError::InvalidDiscriminator {
        field,
        value: u64::from(function.as_u8()),
    }
}

/// Compares the data length a request implies with the one a response has.
pub(super) const fn check_length(needed: usize, available: usize) -> Result<(), ParseError> {
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

/// Compares a field of a write response with the request's.
fn check_echo(field: &'static str, sent: usize, echoed: u16) -> Result<(), ParseError> {
    if sent == usize::from(echoed) {
        Ok(())
    } else {
        Err(ParseError::InvalidDiscriminator {
            field,
            value: u64::from(echoed),
        })
    }
}
