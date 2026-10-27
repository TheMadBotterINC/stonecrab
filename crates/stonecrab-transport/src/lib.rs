#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(clippy::indexing_slicing)]

mod clock;
#[cfg(feature = "tcp")]
mod deadline;
mod error;
mod jitter;
#[cfg(feature = "tcp")]
mod tcp;
mod transport;

pub use clock::SystemClock;
pub use error::FrameError;
pub use jitter::Jitter;
#[cfg(feature = "tcp")]
#[cfg_attr(docsrs, doc(cfg(feature = "tcp")))]
pub use tcp::{TcpFactory, TcpTransport};
pub use transport::{Transport, TransportFactory};
