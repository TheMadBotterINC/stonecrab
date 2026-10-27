#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod address;
mod clock;
mod executor;

pub use address::FixtureAddress;
pub use clock::FakeClock;
pub use executor::{CountingWaker, block_on, block_on_with, poll_once, poll_once_with};
