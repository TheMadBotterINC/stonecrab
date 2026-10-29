//! A virtual serial pair for tests: a pseudo-terminal whose slave end is a port the serial
//! transport can use, and whose master end is "the device" the test drives by hand.
//!
//! The pair is made by `serial2::SerialPort::pair`, so it exists on Unix only; the including
//! test file is compiled out elsewhere. Nothing leaves the machine and no hardware is involved.
//! The device end waits for bytes for one short port timeout per attempt, so every helper
//! retries a bounded number of times instead of sleeping.

#![expect(
    unreachable_pub,
    reason = "plain pub in a privately included module: pub(crate) would trip redundant_pub_crate"
)]

use std::io;
use std::time::Duration;

use serial2::SerialPort;

/// The port timeout of the device end, times the attempts below: how long a helper waits in total.
const SLICE: Duration = Duration::from_millis(20);
const ATTEMPTS: usize = 50;

/// One pseudo-terminal: the device end and the transport end.
///
/// The transport end is either opened by name through the factory (`port_name`, Linux only) or
/// handed over as a port (`take_slave`). Until it is taken, the pair keeps it open so the
/// pseudo-terminal outlives every transport that opens it by name.
pub struct Pair {
    device: SerialPort,
    slave: Option<SerialPort>,
    name: String,
}

impl Pair {
    /// A new pseudo-terminal.
    pub fn new() -> io::Result<Self> {
        let (mut device, slave) = SerialPort::pair()?;
        device.set_read_timeout(SLICE)?;
        device.set_write_timeout(SLICE)?;
        let name = slave_name(&slave);
        Ok(Self {
            device,
            slave: Some(slave),
            name,
        })
    }

    /// The transport end as a port, with the read and write timeout the worker polls by
    /// (what `SerialFactory` does for a port it opens); `None` once taken.
    pub fn take_slave(&mut self) -> Option<SerialPort> {
        let mut slave = self.slave.take()?;
        slave.set_read_timeout(SLICE).ok()?;
        slave.set_write_timeout(SLICE).ok()?;
        Some(slave)
    }

    /// The port name to put in `Endpoint::Custom`: the path of the slave where the platform
    /// shows one (Linux), a label otherwise.
    pub fn port_name(&self) -> &str {
        &self.name
    }

    /// Write `bytes` as the device.
    pub fn send(&self, bytes: &[u8]) -> io::Result<()> {
        self.device.write_all(bytes)
    }

    /// Read `count` bytes as the device, or fewer if they do not arrive in time.
    pub fn receive(&self, count: usize) -> io::Result<Vec<u8>> {
        let mut received = Vec::new();
        let mut chunk = [0_u8; 64];
        for _ in 0..ATTEMPTS {
            if received.len() >= count {
                break;
            }
            match self.device.read(&mut chunk) {
                Ok(read) => received.extend(chunk.iter().take(read)),
                Err(error) if error.kind() == io::ErrorKind::TimedOut => {}
                Err(error) => return Err(error),
            }
        }
        received.truncate(count);
        Ok(received)
    }

    /// Close the device end, which the transport sees as the peer going away.
    pub fn close(self) {
        drop(self.device);
    }

    /// Whether the transport end has been closed, which the device end sees as a hang-up (an
    /// end of stream, or an error, depending on the platform) instead of an empty line.
    pub fn transport_end_closed(&self) -> bool {
        let mut chunk = [0_u8; 1];
        (0..ATTEMPTS).any(|_| match self.device.read(&mut chunk) {
            Ok(read) => read == 0,
            Err(error) => error.kind() != io::ErrorKind::TimedOut,
        })
    }
}

/// The path of the slave end: Linux lists the open files of a process under `/proc/self/fd`.
#[cfg(target_os = "linux")]
fn slave_name(slave: &SerialPort) -> String {
    use std::os::fd::AsRawFd;

    std::fs::read_link(format!("/proc/self/fd/{}", slave.as_raw_fd())).map_or_else(
        |_| "pty-slave".to_owned(),
        |path| path.display().to_string(),
    )
}

/// Other platforms have no safe way to ask for the path; the name is only a label there.
#[cfg(not(target_os = "linux"))]
fn slave_name(_slave: &SerialPort) -> String {
    "pty-slave".to_owned()
}
