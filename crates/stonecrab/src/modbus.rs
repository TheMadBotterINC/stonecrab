#![doc = include_str!("../docs/components/modbus.md")]
//!
//! # Parsing a connection string
//!
//! The `modbus-tcp` scheme is registered by the application: build a [`DriverRegistry`](crate::DriverRegistry),
//! register a [`ModbusTcpDriver`] and connect with `modbus-tcp://host[:port]`. Parsing opens
//! no socket. The configuration is parsed first so that its address options can be taken
//! before `connect` consumes it.
//!
//! ```
//! use stonecrab::modbus::{ModbusOptions, ModbusTcpDriver};
//! use stonecrab::{Driver, ProtocolAddress};
//!
//! let driver = ModbusTcpDriver::new();
//! let config = driver
//!     .parse_config("modbus-tcp://192.0.2.10:1502?address-convention=plc4x")?;
//! let options = config.address_options();
//! let address = driver.parse_address("40001:UINT", &options)?;
//! assert_eq!(address.canonical(), "holding-register:1:UINT");
//! assert!(config.driver_options_as::<ModbusOptions>().is_some());
//! # Ok::<(), stonecrab::PlcError>(())
//! ```

mod address;
mod batch;
mod connection;
mod decode;
mod driver;
mod framing;
mod options;
mod pdu;
mod session;

pub use crate::transport::SystemClock;
pub use address::{
    AddressConvention, ModbusAddress, ModbusAddressBuilder, ModbusArea, ModbusDataType, UnitId,
};
pub use batch::{BatchLimits, Batcher, Chunk, Member};
pub use connection::{ConnectionDefaults, ModbusConnection};
pub use decode::TypedDecoder;
pub use driver::ModbusTcpDriver;
pub use framing::{MbapFramer, MbapHeader, ModbusAdu, TransactionId};
pub use options::ModbusOptions;
pub use pdu::{
    ExceptionCode, FileRecord, FileRecordRead, FunctionCode, MAX_PDU_BYTES, MAX_READ_BITS,
    MAX_READ_EXTENDED_REGISTERS, MAX_READ_REGISTERS, ModbusPdu, decode_request, decode_response,
    encode_request, encode_response,
};
pub use session::{ModbusSession, SessionStats, exception_item_error};
