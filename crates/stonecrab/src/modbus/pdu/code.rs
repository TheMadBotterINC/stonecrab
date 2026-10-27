//! Function codes and exception codes.

/// The function code of a PDU.
///
/// The alpha speaks function codes 01 to 04. Every other value, including the write codes
/// 05, 06, 15 (0x0F), 16 (0x10) and the file record codes 20 and 21 (0x14, 0x15), is
/// [`Unknown`](Self::Unknown): the codec carries the number and refuses to decode a PDU
/// that uses it. Use [`from_u8`](Self::from_u8) to build a value so that a number with a
/// named variant never ends up as `Unknown`.
///
/// # Examples
/// ```
/// use stonecrab::modbus::FunctionCode;
///
/// assert_eq!(FunctionCode::from_u8(0x03), FunctionCode::ReadHoldingRegisters);
/// assert_eq!(FunctionCode::ReadHoldingRegisters.as_u8(), 0x03);
/// assert_eq!(FunctionCode::ReadHoldingRegisters.exception_code(), 0x83);
/// assert_eq!(FunctionCode::from_u8(0x10), FunctionCode::Unknown(0x10));
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-HR
/// The four named variants are the upstream read PDUs: FC01 `MB-TCP-READ-COIL`, FC02
/// `MB-TCP-READ-DI`, FC03 `MB-TCP-READ-HR` and FC04 `MB-TCP-READ-IR`. The write PDUs
/// upstream builds (`MB-TCP-WRITE`) are not transcribed (decision D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FunctionCode {
    /// 0x01, read coils.
    ReadCoils,
    /// 0x02, read discrete inputs.
    ReadDiscreteInputs,
    /// 0x03, read holding registers.
    ReadHoldingRegisters,
    /// 0x04, read input registers.
    ReadInputRegisters,
    /// Any other function code, kept as the number that was on the wire.
    Unknown(u8),
}

impl FunctionCode {
    /// The function code for `value`, `Unknown` when it is not one of 0x01 to 0x04.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::FunctionCode;
    /// assert_eq!(FunctionCode::from_u8(1), FunctionCode::ReadCoils);
    /// assert_eq!(FunctionCode::from_u8(0), FunctionCode::Unknown(0));
    /// ```
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0x01 => Self::ReadCoils,
            0x02 => Self::ReadDiscreteInputs,
            0x03 => Self::ReadHoldingRegisters,
            0x04 => Self::ReadInputRegisters,
            other => Self::Unknown(other),
        }
    }

    /// The number that goes on the wire in a request or a normal response.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::FunctionCode;
    /// assert_eq!(FunctionCode::ReadInputRegisters.as_u8(), 4);
    /// assert_eq!(FunctionCode::Unknown(0x2B).as_u8(), 0x2B);
    /// ```
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ReadCoils => 0x01,
            Self::ReadDiscreteInputs => 0x02,
            Self::ReadHoldingRegisters => 0x03,
            Self::ReadInputRegisters => 0x04,
            Self::Unknown(value) => value,
        }
    }

    /// The first byte of the exception response to this function: the function code with
    /// the high bit set (V1.1b section 7).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::FunctionCode;
    /// assert_eq!(FunctionCode::ReadCoils.exception_code(), 0x81);
    /// ```
    #[must_use]
    pub const fn exception_code(self) -> u8 {
        self.as_u8() | 0x80
    }
}

/// The exception code that follows the function byte of an exception response.
///
/// The ten named variants are the codes V1.1b section 7 defines (there is no code 9).
/// [`Unknown`](Self::Unknown) carries any other number; [`from_u8`](Self::from_u8) makes
/// it for every value outside the ten. The decoder does not accept an unknown code in a
/// response (see [`decode_response`](crate::modbus::decode_response)); the variant exists so that
/// the number can still be named, logged and reported by the layers above.
///
/// # Examples
/// ```
/// use stonecrab::modbus::ExceptionCode;
///
/// let code = ExceptionCode::from_u8(0x0A);
/// assert_eq!(code, ExceptionCode::GatewayPathUnavailable);
/// assert_eq!(code.as_u8(), 10);
/// assert_eq!(code.name(), "GATEWAY PATH UNAVAILABLE");
/// assert_eq!(ExceptionCode::from_u8(9), ExceptionCode::Unknown(9));
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-EXC-01
/// The variants are the upstream `ModbusErrorCode` values, rows `MB-EXC-01` to
/// `MB-EXC-08`, `MB-EXC-0A` and `MB-EXC-0B`; `Unknown` is row `MB-EXC-UNKNOWN`. How each
/// code maps to an `ErrorKind` (decision D18, rows `MB-EXC-03` and `MB-EXC-05` deviate
/// from upstream) is the session's job, not the codec's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExceptionCode {
    /// 0x01: the function code is not supported (`MB-EXC-01`).
    IllegalFunction,
    /// 0x02: the data address is not allowed for the device (`MB-EXC-02`).
    IllegalDataAddress,
    /// 0x03: a value in the request is not allowed (`MB-EXC-03`).
    IllegalDataValue,
    /// 0x04: the device failed while performing the request (`MB-EXC-04`).
    ServerDeviceFailure,
    /// 0x05: the request was accepted and will take a long time (`MB-EXC-05`). It is not
    /// a result.
    Acknowledge,
    /// 0x06: the device is busy with a long-running command (`MB-EXC-06`).
    ServerDeviceBusy,
    /// 0x07: the device cannot perform the program function (`MB-EXC-07`).
    NegativeAcknowledge,
    /// 0x08: a parity error was found in the device memory (`MB-EXC-08`).
    MemoryParityError,
    /// 0x0A: the gateway could not allocate a path to the target (`MB-EXC-0A`).
    GatewayPathUnavailable,
    /// 0x0B: the target device behind a gateway did not answer (`MB-EXC-0B`).
    GatewayTargetDeviceFailedToRespond,
    /// Any other code, kept as the number that was seen (`MB-EXC-UNKNOWN`).
    Unknown(u8),
}

impl ExceptionCode {
    /// The exception code for `value`, `Unknown` for every number outside the ten defined
    /// codes (including 0 and 9).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::ExceptionCode;
    /// assert_eq!(ExceptionCode::from_u8(2), ExceptionCode::IllegalDataAddress);
    /// assert_eq!(ExceptionCode::from_u8(0x0C), ExceptionCode::Unknown(0x0C));
    /// ```
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0x01 => Self::IllegalFunction,
            0x02 => Self::IllegalDataAddress,
            0x03 => Self::IllegalDataValue,
            0x04 => Self::ServerDeviceFailure,
            0x05 => Self::Acknowledge,
            0x06 => Self::ServerDeviceBusy,
            0x07 => Self::NegativeAcknowledge,
            0x08 => Self::MemoryParityError,
            0x0A => Self::GatewayPathUnavailable,
            0x0B => Self::GatewayTargetDeviceFailedToRespond,
            other => Self::Unknown(other),
        }
    }

    /// The number that goes on the wire.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::ExceptionCode;
    /// assert_eq!(ExceptionCode::GatewayTargetDeviceFailedToRespond.as_u8(), 11);
    /// assert_eq!(ExceptionCode::Unknown(0x63).as_u8(), 0x63);
    /// ```
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::IllegalFunction => 0x01,
            Self::IllegalDataAddress => 0x02,
            Self::IllegalDataValue => 0x03,
            Self::ServerDeviceFailure => 0x04,
            Self::Acknowledge => 0x05,
            Self::ServerDeviceBusy => 0x06,
            Self::NegativeAcknowledge => 0x07,
            Self::MemoryParityError => 0x08,
            Self::GatewayPathUnavailable => 0x0A,
            Self::GatewayTargetDeviceFailedToRespond => 0x0B,
            Self::Unknown(value) => value,
        }
    }

    /// The name V1.1b section 7 gives the code, in capitals, or `"UNKNOWN"` for
    /// [`Unknown`](Self::Unknown). The text is for logs and `ProtocolStatus` names; the
    /// number from [`as_u8`](Self::as_u8) is the identity.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::ExceptionCode;
    /// assert_eq!(ExceptionCode::IllegalFunction.name(), "ILLEGAL FUNCTION");
    /// assert_eq!(ExceptionCode::Unknown(9).name(), "UNKNOWN");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::IllegalFunction => "ILLEGAL FUNCTION",
            Self::IllegalDataAddress => "ILLEGAL DATA ADDRESS",
            Self::IllegalDataValue => "ILLEGAL DATA VALUE",
            Self::ServerDeviceFailure => "SERVER DEVICE FAILURE",
            Self::Acknowledge => "ACKNOWLEDGE",
            Self::ServerDeviceBusy => "SERVER DEVICE BUSY",
            Self::NegativeAcknowledge => "NEGATIVE ACKNOWLEDGE",
            Self::MemoryParityError => "MEMORY PARITY ERROR",
            Self::GatewayPathUnavailable => "GATEWAY PATH UNAVAILABLE",
            Self::GatewayTargetDeviceFailedToRespond => "GATEWAY TARGET DEVICE FAILED TO RESPOND",
            Self::Unknown(_) => "UNKNOWN",
        }
    }
}
