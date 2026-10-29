//! The serial Modbus drivers: Modbus RTU (ADR-0024) and Modbus ASCII (ADR-0024, addendum
//! S60), their connection-string parameters and the silence on the line before each
//! request. The framing is in `framing/`; the session, the connection and the PDU codec are
//! the ones of Modbus TCP.

mod ascii;
mod driver;
mod line;
mod pace;

pub use ascii::ModbusAsciiDriver;
pub use driver::ModbusRtuDriver;
