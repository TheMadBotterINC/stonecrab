#![doc = include_str!("../docs/components/transport.md")]

mod clock;
mod deadline;
mod error;
mod framed;
mod framer;
mod guard;
mod jitter;
mod tcp;
mod traits;

pub use clock::SystemClock;
pub use error::FrameError;
pub use framed::FramedTransport;
pub use framer::Framer;
pub use guard::{ConnectionGuard, GuardConfig, GuardStats};
pub use jitter::Jitter;
pub use tcp::{TcpFactory, TcpTransport};
pub use traits::{Transport, TransportFactory};
