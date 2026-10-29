#![doc = include_str!("../docs/components/testkit.md")]

mod address;
mod clock;
mod contract;
mod convention;
mod counting_factory;
mod executor;
mod fake_factory;
#[cfg(feature = "boards")]
mod fake_spi;
mod fake_transport;
mod framer;
mod golden;
mod interop;
mod peer;
mod peer_factory;
mod script;

pub use address::FixtureAddress;
pub use clock::FakeClock;
pub use contract::{
    CaseOutcome, ContractCase, ContractFixture, ContractReport, ContractSuite, Fault, Requires,
};
pub use convention::ConventionDriver;
pub use counting_factory::CountingFactory;
pub use executor::{CountingWaker, block_on, block_on_with, poll_once, poll_once_with};
pub use fake_factory::{FakeTransportFactory, OpenCall};
#[cfg(feature = "boards")]
#[cfg_attr(docsrs, doc(cfg(feature = "boards")))]
pub use fake_spi::{FakeSpiBus, SpiStep};
pub use fake_transport::{FakeTransport, FakeTransportProbe};
pub use framer::LengthPrefixedFramer;
pub use golden::{
    FileEntry, GoldenError, GoldenVector, HexProblem, Provenance, VectorEntry, VectorSource,
    load_dir, load_hex, load_provenance,
};
pub use interop::{BitRun, BlockSizes, InteropMap, RegisterEntry};
pub use peer::{MAX_PEER_DELAY, ScriptedPeer};
pub use peer_factory::ScriptedPeerFactory;
pub use script::{Mismatch, Reaction, Reactor, Responder, Script, ScriptReport, Step};
