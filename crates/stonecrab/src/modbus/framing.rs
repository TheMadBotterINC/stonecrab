//! Framing: MBAP for Modbus TCP (the 7 byte header, the transaction counter and the
//! [`Framer`](crate::transport::Framer) that cuts ADUs out of a byte stream), RTU for
//! Modbus RTU (the address, the PDU and the CRC-16 on a serial line) and ASCII for Modbus
//! ASCII (a colon, the hex of the address, the PDU and the LRC, then CR LF).
//!
//! An application data unit (ADU) is the MBAP header followed by the PDU (decision D5):
//! the transaction id, the protocol id (always 0), a length that counts the unit id and
//! the PDU, the unit id, then the PDU. An RTU ADU is the address byte, the PDU and the
//! CRC-16, low byte first (ADR-0024). An ASCII ADU is the address, the PDU and the LRC, each
//! byte as two hex characters between a colon and CR LF (ADR-0024, addendum S60). This module
//! only finds frame boundaries and checks the header, the CRC or the LRC; the PDU stays
//! opaque bytes until the session has matched the reply to its request, and `pdu/` decodes
//! it then, the same code for every framing.

mod ascii;
mod mbap;
mod rtu;

pub use ascii::{AsciiAdu, AsciiFramer};
pub use mbap::{MbapFramer, MbapHeader, ModbusAdu, TransactionId};
pub use rtu::{RtuAdu, RtuFramer};
