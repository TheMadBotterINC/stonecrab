//! MBAP framing for Modbus TCP: the 7 byte header, the transaction counter and the
//! [`Framer`](crate::transport::Framer) that cuts ADUs out of a byte stream.
//!
//! An application data unit (ADU) is the MBAP header followed by the PDU (decision D5):
//! the transaction id, the protocol id (always 0), a length that counts the unit id and
//! the PDU, the unit id, then the PDU. This module only finds frame boundaries and checks
//! the header; the PDU stays opaque bytes until the session has matched the reply to its
//! request, and `pdu/` decodes it then. Serial (RTU) framing is not part of the alpha.

mod mbap;

pub use mbap::{MbapFramer, MbapHeader, ModbusAdu, TransactionId};
