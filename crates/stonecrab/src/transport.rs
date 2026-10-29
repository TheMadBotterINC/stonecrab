#![doc = include_str!("../docs/components/transport.md")]

mod clock;
mod deadline;
mod error;
mod framed;
mod framer;
mod guard;
mod jitter;
#[cfg(feature = "serial")]
mod serial;
#[cfg(feature = "serial")]
mod serial_worker;
mod tcp;
#[cfg(feature = "tls")]
mod tls;
mod traits;
mod udp;

pub use clock::SystemClock;
pub use error::FrameError;
pub use framed::FramedTransport;
pub use framer::Framer;
pub use guard::{ConnectionGuard, GuardConfig, GuardStats};
pub use jitter::Jitter;
#[cfg(feature = "serial")]
#[cfg_attr(docsrs, doc(cfg(feature = "serial")))]
pub use serial::{DataBits, FlowControl, Parity, SerialFactory, SerialTransport, StopBits};
pub use tcp::{TcpFactory, TcpTransport};
#[cfg(feature = "tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub use tls::{TlsFactory, TlsSettings};
pub use traits::{Transport, TransportFactory};
pub use udp::{UdpFactory, UdpTransport};
