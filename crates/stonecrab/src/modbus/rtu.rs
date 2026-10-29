//! Modbus RTU (ADR-0024): the `modbus-rtu` driver, its connection-string parameters and the
//! silence on the line before each request. The framing is in `framing/`; the session, the
//! connection and the PDU codec are the ones of Modbus TCP.

mod driver;
mod line;
mod pace;

pub use driver::ModbusRtuDriver;
