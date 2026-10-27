#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod address;
mod clock;
mod executor;
#[cfg(feature = "net")]
mod fake_factory;
#[cfg(feature = "net")]
mod fake_transport;
#[cfg(feature = "net")]
mod peer;
#[cfg(feature = "net")]
mod script;

pub use address::FixtureAddress;
pub use clock::FakeClock;
pub use executor::{CountingWaker, block_on, block_on_with, poll_once, poll_once_with};
#[cfg(feature = "net")]
pub use fake_factory::{FakeTransportFactory, OpenCall};
#[cfg(feature = "net")]
pub use fake_transport::{FakeTransport, FakeTransportProbe};
#[cfg(feature = "net")]
pub use peer::{MAX_PEER_DELAY, ScriptedPeer};
#[cfg(feature = "net")]
pub use script::{Mismatch, Responder, Script, ScriptReport, Step};
