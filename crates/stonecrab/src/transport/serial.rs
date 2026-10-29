use std::io;

use crate::{Deadline, Endpoint, ErrorKind, OpFuture, PlcError, ResourceLimits, Result};
use bytes::{Bytes, BytesMut};

use crate::transport::deadline::{Site, ensure_before, within};
use crate::transport::serial_worker::{POLL, Worker};
use crate::transport::{Transport, TransportFactory};

/// Data bits per character on a serial line.
///
/// Upstream's `data-bits` option; the default is eight.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-SERIAL-CONFIG
/// Values 5 to 8 as upstream (`SerialTransportConfiguration`).
///
/// # Examples
/// ```
/// use stonecrab::transport::{DataBits, SerialFactory};
///
/// assert_eq!(DataBits::default(), DataBits::Eight);
/// assert_eq!(SerialFactory::default().data_bits, DataBits::Eight);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum DataBits {
    /// Five data bits.
    Five,
    /// Six data bits.
    Six,
    /// Seven data bits.
    Seven,
    /// Eight data bits.
    #[default]
    Eight,
}

impl DataBits {
    const fn port(self) -> serial2::CharSize {
        match self {
            Self::Five => serial2::CharSize::Bits5,
            Self::Six => serial2::CharSize::Bits6,
            Self::Seven => serial2::CharSize::Bits7,
            Self::Eight => serial2::CharSize::Bits8,
        }
    }
}

/// Parity of a serial line.
///
/// Upstream's `parity` option; the default is none. Upstream also offers mark and space
/// parity; the `serial2` library has neither, so they are not offered here.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-SERIAL-CONFIG
/// `none`, `odd` and `even` as upstream; `mark` and `space` are a recorded gap.
///
/// # Examples
/// ```
/// use stonecrab::transport::{Parity, SerialFactory};
///
/// assert_eq!(Parity::default(), Parity::None);
/// assert_eq!(SerialFactory::default().parity, Parity::None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Parity {
    /// No parity bit.
    #[default]
    None,
    /// Odd parity.
    Odd,
    /// Even parity.
    Even,
}

impl Parity {
    const fn port(self) -> serial2::Parity {
        match self {
            Self::None => serial2::Parity::None,
            Self::Odd => serial2::Parity::Odd,
            Self::Even => serial2::Parity::Even,
        }
    }
}

/// Stop bits on a serial line.
///
/// Upstream's `stop-bits` option; the default is one.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-SERIAL-CONFIG
/// One and two stop bits; upstream's 1.5 is not offered by the `serial2` library.
///
/// # Examples
/// ```
/// use stonecrab::transport::{SerialFactory, StopBits};
///
/// assert_eq!(StopBits::default(), StopBits::One);
/// assert_eq!(SerialFactory::default().stop_bits, StopBits::One);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum StopBits {
    /// One stop bit.
    #[default]
    One,
    /// Two stop bits.
    Two,
}

impl StopBits {
    const fn port(self) -> serial2::StopBits {
        match self {
            Self::One => serial2::StopBits::One,
            Self::Two => serial2::StopBits::Two,
        }
    }
}

/// Flow control of a serial line.
///
/// Upstream's `flow-control` option; the default is none.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-SERIAL-CONFIG
/// `none`, `rts-cts` and `xon-xoff` as upstream.
///
/// # Examples
/// ```
/// use stonecrab::transport::{FlowControl, SerialFactory};
///
/// assert_eq!(FlowControl::default(), FlowControl::None);
/// assert_eq!(SerialFactory::default().flow_control, FlowControl::None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FlowControl {
    /// No flow control.
    #[default]
    None,
    /// Hardware flow control with the RTS and CTS lines.
    RtsCts,
    /// Software flow control with the XON and XOFF characters.
    XonXoff,
}

impl FlowControl {
    const fn port(self) -> serial2::FlowControl {
        match self {
            Self::None => serial2::FlowControl::None,
            Self::RtsCts => serial2::FlowControl::RtsCts,
            Self::XonXoff => serial2::FlowControl::XonXoff,
        }
    }
}

/// A [`Transport`] over one serial port, driven by a blocking worker thread.
///
/// Made by [`SerialFactory::open`], or by [`SerialTransport::from_port`] for a port you opened
/// yourself. Serial ports are read and written with blocking calls, so one dedicated thread
/// owns the port (coding standards section 8.4) and the async caller talks to it over a
/// bounded channel of two commands. Every call is raced against its `Deadline` on the
/// caller's side; the thread reads no clock. The factory gives the port a 20 ms read and
/// write timeout, so every port call returns within one slice and the thread notices a
/// dropped or timed-out call at its next one and abandons its work: a `send` into a stalled
/// or flow-controlled line, which a frame-sized write could hold for as long as the line
/// stays stalled, costs the worker one slice after the caller's deadline, not more. The
/// frame is offered to the port in chunks for the same reason. Bytes the thread read for a
/// call that has gone are kept for the next `recv`, which is why `recv` is cancel-safe.
///
/// The transport has one owner. [`Transport::shutdown`] closes the channel, waits for the
/// thread to drop the port and so releases the device; dropping the transport without it does
/// the same without waiting, and the thread is gone within one slice.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-SERIAL-WORKER
/// Upstream's serial transport (jSerialComm, a 64 KiB receive ring that drops the oldest
/// bytes) has no counterpart for the worker model; the receive bound here is
/// `ResourceLimits::max_buffered_bytes`, as for every transport.
///
/// # Examples
/// See [`SerialFactory`].
#[derive(Debug)]
pub struct SerialTransport {
    worker: Worker,
    peer: Endpoint,
    limits: ResourceLimits,
    /// False once the peer closed, a write was cut short or a read failed.
    usable: bool,
    /// True once `shutdown` has been called, so a repeat is a no-op.
    shut_down: bool,
}

impl SerialTransport {
    /// Wrap a port you opened yourself and start its worker thread.
    ///
    /// `peer` is what [`Transport::peer`] reports. The factory uses this for the ports it
    /// opens; call it directly for a port that needs setup the factory does not offer. The
    /// port must give up a read or a write after a bounded time, by returning
    /// `io::ErrorKind::TimedOut` (a `serial2` port does after the timeout set on it, which is
    /// three seconds unless you change it: set both its read and its write timeout to about
    /// 20 ms, as the factory does, and never to zero, or the worker spins while a call
    /// waits): the worker only notices that a call was dropped or timed out when the port
    /// returns, and `shutdown` waits for it. A read that returns `Ok(0)` or `BrokenPipe` is
    /// taken as the peer having closed; a write that returns `Ok(0)` is taken as a timeout and
    /// tried again until the call is abandoned. Every such retry (`Ok(0)`, `TimedOut`,
    /// `WouldBlock`) that returns at once makes the worker spin until then.
    ///
    /// # Errors
    /// - [`ErrorKind::Disconnected`] if the operating system refuses to start the worker
    ///   thread.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-SERIAL-WORKER
    /// Upstream cannot wrap a caller-supplied port; this is a Stonecrab extension.
    ///
    /// # Examples
    /// The example is not run because it needs a serial device; the crate's tests drive the
    /// same calls over a pseudo-terminal pair.
    /// ```no_run
    /// use std::time::Duration;
    /// use stonecrab::{Endpoint, ResourceLimits};
    /// use stonecrab::transport::SerialTransport;
    ///
    /// # fn demo() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut port = serial2::SerialPort::open("/dev/ttyUSB0", 115_200)?;
    /// port.set_read_timeout(Duration::from_millis(20))?;
    /// port.set_write_timeout(Duration::from_millis(20))?;
    /// let peer = Endpoint::Custom("/dev/ttyUSB0".to_owned());
    /// let transport = SerialTransport::from_port(port, peer, ResourceLimits::builder().build()?)?;
    /// # let _ = transport;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_port<P>(port: P, peer: Endpoint, limits: ResourceLimits) -> Result<Self>
    where
        P: io::Read + io::Write + Send + 'static,
    {
        let worker = Worker::spawn(port).map_err(|error| Site::SERIAL_OPEN.io_error(error))?;
        Ok(Self {
            worker,
            peer,
            limits,
            usable: true,
            shut_down: false,
        })
    }

    fn ensure_usable(&self, site: Site) -> Result<()> {
        if self.usable {
            Ok(())
        } else {
            Err(site.error(ErrorKind::Disconnected, "transport is no longer usable"))
        }
    }
}

impl Transport for SerialTransport {
    /// Writes `bytes` to the port within `deadline`.
    ///
    /// Checks run in this order, each before any I/O: the deadline has not passed, the
    /// transport is usable, `bytes` fits in `max_frame_bytes`. The deadline is the
    /// `deadline` argument. The call returns when the worker has handed every byte to the
    /// operating system, not when the last bit has left the line.
    ///
    /// # Cancel safety
    /// Not cancel-safe: a prefix of `bytes` may have been written when the future is dropped
    /// or the deadline passes, so the transport marks itself unusable first and only restores
    /// that once the whole frame was written.
    ///
    /// # Wire effects
    /// Transmits `bytes` on the line; on timeout or cancellation the peer may hold any prefix
    /// of it.
    fn send(&mut self, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            ensure_before(deadline, Site::SERIAL_SEND)?;
            self.ensure_usable(Site::SERIAL_SEND)?;
            let limit = self.limits.max_frame_bytes().get();
            if bytes.len() > limit {
                let detail = format!(
                    "frame of {} bytes exceeds the {limit} byte frame limit",
                    bytes.len()
                );
                return Err(Site::SERIAL_SEND.error(ErrorKind::ResourceLimit, &detail));
            }
            self.usable = false;
            within(deadline, Site::SERIAL_SEND, self.worker.write(bytes)).await?;
            self.usable = true;
            Ok(())
        })
    }

    /// Reads whatever has arrived into the free space of `buffer`, within `deadline`.
    ///
    /// Checks run in this order, each before any I/O: the deadline has not passed, the
    /// transport is usable, `buffer` is below `max_buffered_bytes`. At most 1024 bytes are
    /// returned by one call. A port that reports the peer gone (an end of stream or a broken
    /// pipe, as a closed pseudo-terminal does) returns `0`; bytes still unread when the peer hangs
    /// up may be discarded by the operating-system serial layer. The deadline is the `deadline`
    /// argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: the worker keeps bytes it read for a call that was dropped or timed out and
    /// hands them to the next `recv`, so none are lost and the transport stays usable.
    ///
    /// # Wire effects
    /// None.
    fn recv<'a>(&'a mut self, buffer: &'a mut BytesMut, deadline: Deadline) -> OpFuture<'a, usize> {
        Box::pin(async move {
            ensure_before(deadline, Site::SERIAL_RECV)?;
            self.ensure_usable(Site::SERIAL_RECV)?;
            let bound = self.limits.max_buffered_bytes().get();
            let free = bound.saturating_sub(buffer.len());
            if free == 0 {
                let detail = format!("receive buffer already holds the {bound} byte limit");
                return Err(Site::SERIAL_RECV.error(ErrorKind::ResourceLimit, &detail));
            }
            match within(deadline, Site::SERIAL_RECV, self.worker.read(free)).await {
                Ok(bytes) if bytes.is_empty() => {
                    self.usable = false;
                    Ok(0)
                }
                Ok(bytes) => {
                    buffer.extend_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Err(error) => {
                    // A deadline expiry is the only `Timeout` here and loses nothing.
                    self.usable &= error.kind() == ErrorKind::Timeout;
                    Err(error)
                }
            }
        })
    }

    /// Stops the worker and releases the port within `deadline`.
    ///
    /// The transport is closed from the moment this is polled, whether or not the worker has
    /// finished; a repeat call returns `Ok(())` without waiting, even with an elapsed
    /// deadline. The worker drops the port when it ends, so the device is free once this
    /// returns `Ok`. If the deadline passes first (closing a port can wait for output to
    /// drain), the call returns `Timeout` and the worker finishes by itself. The deadline is
    /// the `deadline` argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: nothing is sent, and the transport is closed from the first poll.
    ///
    /// # Wire effects
    /// None beyond what the operating system does when a port is closed.
    fn shutdown(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            if self.shut_down {
                return Ok(());
            }
            ensure_before(deadline, Site::SERIAL_SHUTDOWN)?;
            self.shut_down = true;
            self.usable = false;
            self.worker.close();
            within(deadline, Site::SERIAL_SHUTDOWN, self.worker.stop()).await
        })
    }

    fn peer(&self) -> &Endpoint {
        &self.peer
    }

    fn limits(&self) -> &ResourceLimits {
        &self.limits
    }
}

/// Opens [`SerialTransport`]s.
///
/// `open` takes an `Endpoint::Custom` whose text is the port name (`/dev/ttyUSB0`, `COM3`),
/// passed to the operating system as is, like upstream. Any other endpoint, an empty name or a
/// zero baud rate is `InvalidConfig`, and so is a baud rate the platform has no setting for.
/// The port is opened on Tokio's blocking pool under the connect deadline. A port that is
/// missing, not permitted or refuses the settings is `Disconnected`; a deadline that passes is
/// `Timeout`. The port is not opened exclusively: a second open of the same device succeeds,
/// and two transports on one port would interleave their bytes, so keep one transport per
/// port. The fields are upstream's options of the same names, with upstream's defaults: 9600
/// baud, 8 data bits, no parity, 1 stop bit, no flow control. The struct is
/// `#[non_exhaustive]`: build one with `SerialFactory::default()` and assign the fields.
///
/// Upstream's `read-timeout-ms` and `write-timeout-ms` have no counterpart: every call takes
/// its own `Deadline`. Its `dtr`, `rts`, `reuse-port` and `interframe-delay` are not offered
/// yet; inter-frame timing belongs to the Modbus RTU framing.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-SERIAL-CONFIG
/// Option names and defaults follow upstream's `SerialTransportConfiguration`; the unoffered
/// options are recorded there.
///
/// # Examples
/// Open a port at 19200 baud, 8 data bits, even parity and one stop bit, and exchange a frame.
/// The example is not run because it needs a serial device; the crate's tests drive the same
/// calls over a pseudo-terminal pair.
/// ```no_run
/// use std::time::{Duration, Instant};
/// use bytes::{Bytes, BytesMut};
/// use stonecrab::{Deadline, Endpoint, ResourceLimits};
/// use stonecrab::transport::{Parity, SerialFactory, TransportFactory};
///
/// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
/// let mut factory = SerialFactory::default();
/// factory.baud_rate = 19_200;
/// factory.parity = Parity::Even;
///
/// let endpoint = Endpoint::Custom("/dev/ttyUSB0".to_owned());
/// let deadline = Deadline::after(Instant::now(), Duration::from_secs(5))?;
/// let mut transport = factory.open(&endpoint, ResourceLimits::builder().build()?, deadline).await?;
///
/// transport.send(Bytes::from_static(b"\x01\x03\x00\x00\x00\x01\x84\x0a"), deadline).await?;
/// let mut reply = BytesMut::new();
/// transport.recv(&mut reply, deadline).await?;
/// transport.shutdown(deadline).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SerialFactory {
    /// Bits per second. Default 9600; zero is refused.
    pub baud_rate: u32,
    /// Data bits per character. Default eight.
    pub data_bits: DataBits,
    /// Parity. Default none.
    pub parity: Parity,
    /// Stop bits. Default one.
    pub stop_bits: StopBits,
    /// Flow control. Default none.
    pub flow_control: FlowControl,
}

impl Default for SerialFactory {
    fn default() -> Self {
        Self {
            baud_rate: 9600,
            data_bits: DataBits::default(),
            parity: Parity::default(),
            stop_bits: StopBits::default(),
            flow_control: FlowControl::default(),
        }
    }
}

/// An open failure: bad input is a configuration error, anything else a lost port.
fn open_error(error: io::Error) -> PlcError {
    if error.kind() == io::ErrorKind::InvalidInput {
        let detail = error.to_string();
        Site::SERIAL_OPEN
            .error(ErrorKind::InvalidConfig, &detail)
            .with_source(error)
    } else {
        Site::SERIAL_OPEN.io_error(error)
    }
}

impl SerialFactory {
    /// The line settings applied on top of the port's raw mode.
    fn settings(self, mut settings: serial2::Settings) -> io::Result<serial2::Settings> {
        settings.set_raw();
        settings.set_baud_rate(self.baud_rate)?;
        settings.set_char_size(self.data_bits.port());
        settings.set_stop_bits(self.stop_bits.port());
        settings.set_parity(self.parity.port());
        settings.set_flow_control(self.flow_control.port());
        Ok(settings)
    }

    /// Open and configure the port, with the read and write timeouts the worker polls by.
    fn open_port(self, name: &str) -> io::Result<serial2::SerialPort> {
        let mut port = serial2::SerialPort::open(name, |settings| self.settings(settings))?;
        port.set_read_timeout(POLL)?;
        port.set_write_timeout(POLL)?;
        Ok(port)
    }

    /// The body of `open`, returning the concrete transport so tests can inspect it.
    async fn open_serial(
        &self,
        endpoint: &Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> Result<SerialTransport> {
        let Endpoint::Custom(name) = endpoint else {
            return Err(Site::SERIAL_OPEN.error(
                ErrorKind::InvalidConfig,
                "the serial transport needs an Endpoint::Custom holding the port name",
            ));
        };
        if name.is_empty() || self.baud_rate == 0 {
            return Err(Site::SERIAL_OPEN.error(
                ErrorKind::InvalidConfig,
                "the serial port name must not be empty and the baud rate must be positive",
            ));
        }
        let (factory, name) = (*self, name.clone());
        let opened = within(deadline, Site::SERIAL_OPEN, async {
            tokio::task::spawn_blocking(move || factory.open_port(&name))
                .await
                .map_err(io::Error::other)
        })
        .await?;
        let port = opened.map_err(open_error)?;
        SerialTransport::from_port(port, endpoint.clone(), limits)
    }
}

impl TransportFactory for SerialFactory {
    /// Opens the port named by `endpoint` and starts its worker, within `deadline`.
    ///
    /// Checks run in this order: the endpoint is an `Endpoint::Custom` with a name and the
    /// baud rate is positive (else `InvalidConfig`), the deadline has not passed, then the
    /// port opens under the deadline, which is the `deadline` argument. The port is opened on
    /// Tokio's blocking pool; if the deadline passes during the open the call returns
    /// `Timeout` at once and the open finishes in the background, dropping the port.
    ///
    /// # Cancel safety
    /// Cancel-safe: a dropped `open` leaves no transport behind, and a port opened after the
    /// drop is closed without being used.
    ///
    /// # Wire effects
    /// Opens the device and applies the line settings; nothing is sent. Opening can reset an
    /// attached device that wires DTR to its reset line.
    fn open<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> OpFuture<'a, Box<dyn Transport>> {
        Box::pin(async move {
            let transport: Box<dyn Transport> =
                Box::new(self.open_serial(endpoint, limits, deadline).await?);
            Ok(transport)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::transport::deadline::now;
    use crate::transport::serial_worker::scripted::{ReadStep, ScriptedPort, WriteStep};

    fn soon() -> Deadline {
        Deadline::after(now(), Duration::from_secs(5)).expect("representable")
    }

    fn limits(frame: usize, buffered: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_frame_bytes(frame)
            .max_buffered_bytes(buffered)
            .build()
            .expect("valid limits")
    }

    fn scripted(reads: Vec<ReadStep>, writes: Vec<WriteStep>) -> SerialTransport {
        let (port, _) = ScriptedPort::new(reads, writes);
        let peer = Endpoint::Custom("scripted".to_owned());
        SerialTransport::from_port(port, peer, limits(6, 8)).expect("spawn the worker")
    }

    fn data(bytes: &[u8]) -> ReadStep {
        ReadStep::Data(bytes.to_vec())
    }

    #[test]
    fn line_settings_map_to_the_port_library_and_default_to_upstream() {
        let factory = SerialFactory::default();
        assert_eq!(
            (
                factory.baud_rate,
                factory.data_bits,
                factory.parity,
                factory.stop_bits,
                factory.flow_control
            ),
            (
                9600,
                DataBits::Eight,
                Parity::None,
                StopBits::One,
                FlowControl::None
            ),
            "upstream defaults"
        );
        let data_bits = [
            (DataBits::Five, serial2::CharSize::Bits5),
            (DataBits::Six, serial2::CharSize::Bits6),
            (DataBits::Seven, serial2::CharSize::Bits7),
            (DataBits::Eight, serial2::CharSize::Bits8),
        ];
        for (ours, theirs) in data_bits {
            assert_eq!(ours.port(), theirs, "{ours:?}");
        }
        let parity = [
            (Parity::None, serial2::Parity::None),
            (Parity::Odd, serial2::Parity::Odd),
            (Parity::Even, serial2::Parity::Even),
        ];
        for (ours, theirs) in parity {
            assert_eq!(ours.port(), theirs, "{ours:?}");
        }
        assert_eq!(StopBits::One.port(), serial2::StopBits::One, "one");
        assert_eq!(StopBits::Two.port(), serial2::StopBits::Two, "two");
        let flow = [
            (FlowControl::None, serial2::FlowControl::None),
            (FlowControl::RtsCts, serial2::FlowControl::RtsCts),
            (FlowControl::XonXoff, serial2::FlowControl::XonXoff),
        ];
        for (ours, theirs) in flow {
            assert_eq!(ours.port(), theirs, "{ours:?}");
        }
    }

    /// The settings of a pseudo-terminal, which a unit test may change in memory without a
    /// device. `Settings` cannot be made without a port, `serial2::SerialPort::pair` is Unix
    /// only, and on macOS it fails (its settings call needs the baud-rate ioctl, which a
    /// pseudo-terminal refuses), so these tests run on Unix other than macOS.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn blank_settings() -> serial2::Settings {
        let (_device, port) = serial2::SerialPort::pair().expect("a pseudo-terminal pair");
        port.get_configuration().expect("the settings of a pty")
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn the_factory_settings_are_what_the_library_is_asked_to_apply() {
        let factory = SerialFactory {
            baud_rate: 19_200,
            data_bits: DataBits::Seven,
            parity: Parity::Odd,
            stop_bits: StopBits::Two,
            flow_control: FlowControl::RtsCts,
        };
        let settings = factory.settings(blank_settings()).expect("settings");
        assert_eq!(settings.get_baud_rate().expect("baud"), 19_200, "baud rate");
        assert_eq!(
            settings.get_char_size().expect("size"),
            serial2::CharSize::Bits7,
            "data bits"
        );
        assert_eq!(
            settings.get_parity().expect("parity"),
            serial2::Parity::Odd,
            "parity"
        );
        assert_eq!(
            settings.get_stop_bits().expect("stop"),
            serial2::StopBits::Two,
            "stop bits"
        );
        assert_eq!(
            settings.get_flow_control().expect("flow"),
            serial2::FlowControl::RtsCts,
            "flow control"
        );

        let defaults = SerialFactory::default()
            .settings(blank_settings())
            .expect("settings");
        assert_eq!(
            defaults.get_baud_rate().expect("baud"),
            9600,
            "default baud"
        );
        assert_eq!(
            defaults.get_char_size().expect("size"),
            serial2::CharSize::Bits8,
            "default data bits"
        );
        assert_eq!(
            defaults.get_parity().expect("parity"),
            serial2::Parity::None,
            "default parity"
        );
        assert_eq!(
            defaults.get_stop_bits().expect("stop"),
            serial2::StopBits::One,
            "default stop bits"
        );
        assert_eq!(
            defaults.get_flow_control().expect("flow"),
            serial2::FlowControl::None,
            "default flow control"
        );
    }

    #[test]
    fn an_open_failure_is_invalid_config_for_bad_input_and_disconnected_otherwise() {
        let bad = io::Error::new(io::ErrorKind::InvalidInput, "no such baud");
        let error = open_error(bad);
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "bad input");
        assert_eq!(error.to_string(), "serial open: no such baud", "message");
        assert!(
            std::error::Error::source(&error).is_some(),
            "the cause is kept"
        );
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Other,
        ] {
            let error = open_error(io::Error::from(kind));
            assert_eq!(error.kind(), ErrorKind::Disconnected, "{kind:?}");
            assert_eq!(
                error.diagnostics().operation,
                Some(crate::Operation::Connect),
                "{kind:?}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (a write into a stalled line)
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_library_write_to_a_line_that_takes_nothing_gives_up_after_its_timeout() {
        let (_device, mut port) = serial2::SerialPort::pair().expect("a pseudo-terminal pair");
        port.set_write_timeout(POLL).expect("write timeout");
        let frame = [0_u8; 256];
        // The device end never reads, so the pseudo-terminal fills and the write that finds no
        // room reports a timeout; the cap keeps a regression from filling memory.
        let outcome = (0..100_000).find_map(|_| port.write(&frame).err());
        let error = outcome.expect("the line filled up and a write gave up");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut, "a bounded write");
    }

    #[test]
    fn the_peer_and_the_limits_are_what_the_transport_was_given() {
        let transport = scripted(vec![], vec![]);
        assert_eq!(
            transport.peer(),
            &Endpoint::Custom("scripted".to_owned()),
            "peer"
        );
        assert_eq!(transport.limits(), &limits(6, 8), "limits");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (frame bound, buffer bound)
    #[tokio::test]
    async fn frame_and_buffer_bounds_are_enforced_before_any_io() {
        let (port, written) = ScriptedPort::new(vec![data(b"abcdefgh")], vec![]);
        let peer = Endpoint::Custom("scripted".to_owned());
        let mut transport = SerialTransport::from_port(port, peer, limits(4, 6)).expect("spawn");

        let error = transport
            .send(Bytes::from_static(b"12345"), soon())
            .await
            .expect_err("over the frame limit");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "frame limit");
        assert!(
            written.lock().expect("record").is_empty(),
            "nothing was sent"
        );
        transport
            .send(Bytes::from_static(b"1234"), soon())
            .await
            .expect("a frame at the limit still goes, so the refusal did not poison");

        let mut buffer = BytesMut::new();
        assert_eq!(
            transport.recv(&mut buffer, soon()).await.expect("recv"),
            6,
            "free space"
        );
        let error = transport
            .recv(&mut buffer, soon())
            .await
            .expect_err("buffer is full");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "buffer limit");
        buffer.clear();
        assert_eq!(
            transport.recv(&mut buffer, soon()).await.expect("recv"),
            2,
            "the rest"
        );
        assert_eq!(&buffer[..], b"gh", "kept in order");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (peer close)
    #[tokio::test]
    async fn an_end_of_stream_returns_zero_and_poisons_the_transport() {
        let mut transport = scripted(vec![ReadStep::Eof], vec![]);
        let mut buffer = BytesMut::new();
        assert_eq!(
            transport.recv(&mut buffer, soon()).await.expect("recv"),
            0,
            "closed"
        );
        let error = transport
            .recv(&mut buffer, soon())
            .await
            .expect_err("poisoned");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "recv after close");
        let error = transport
            .send(Bytes::from_static(b"x"), soon())
            .await
            .expect_err("poisoned");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "send after close");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (port error)
    #[tokio::test]
    async fn port_errors_are_disconnected_with_the_cause_and_poison_the_transport() {
        let mut transport = scripted(vec![ReadStep::Fail(io::ErrorKind::Other)], vec![]);
        let mut buffer = BytesMut::new();
        let error = transport
            .recv(&mut buffer, soon())
            .await
            .expect_err("read fails");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "read error");
        assert!(
            std::error::Error::source(&error).is_some(),
            "the io error is kept"
        );
        let error = transport
            .recv(&mut buffer, soon())
            .await
            .expect_err("poisoned");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "poisoned");

        let mut transport = scripted(vec![], vec![WriteStep::Fail(io::ErrorKind::BrokenPipe)]);
        let error = transport
            .send(Bytes::from_static(b"x"), soon())
            .await
            .expect_err("write fails");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "write error");
        let error = transport
            .send(Bytes::from_static(b"x"), soon())
            .await
            .expect_err("poisoned");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "poisoned");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (deadline expiry)
    #[tokio::test(start_paused = true)]
    async fn a_recv_deadline_keeps_the_transport_and_a_send_deadline_poisons_it() {
        let mut transport = scripted(vec![], vec![WriteStep::Stall]);
        let mut buffer = BytesMut::new();
        let started = tokio::time::Instant::now();
        let three_seconds = Deadline::after(now(), Duration::from_secs(3)).expect("deadline");
        let error = transport
            .recv(&mut buffer, three_seconds)
            .await
            .expect_err("an idle line");
        assert_eq!(error.kind(), ErrorKind::Timeout, "recv deadline");
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(3),
            "paused time stopped at the deadline"
        );
        assert!(buffer.is_empty(), "nothing appended");

        // The port never takes a byte, as a stalled flow-controlled line would not.
        let error = transport
            .send(Bytes::from_static(b"x"), soon())
            .await
            .expect_err("a stalled write");
        assert_eq!(error.kind(), ErrorKind::Timeout, "send deadline");
        let error = transport
            .send(Bytes::from_static(b"x"), soon())
            .await
            .expect_err("poisoned");
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "a send cut short may have written a prefix"
        );
    }

    #[tokio::test]
    async fn an_elapsed_deadline_fails_before_any_io_and_leaves_shutdown_undone() {
        let mut transport = scripted(vec![data(b"ab")], vec![]);
        let elapsed = Deadline::at(now());
        let mut buffer = BytesMut::new();
        for (call, error) in [
            (
                "send",
                transport
                    .send(Bytes::from_static(b"x"), elapsed)
                    .await
                    .expect_err("elapsed"),
            ),
            (
                "recv",
                transport
                    .recv(&mut buffer, elapsed)
                    .await
                    .expect_err("elapsed"),
            ),
            (
                "shutdown",
                transport.shutdown(elapsed).await.expect_err("elapsed"),
            ),
        ] {
            assert_eq!(error.kind(), ErrorKind::Timeout, "{call}");
        }
        assert_eq!(
            transport.recv(&mut buffer, soon()).await.expect("recv"),
            2,
            "still open, and the bytes were not consumed"
        );

        transport.shutdown(soon()).await.expect("shutdown");
        transport
            .shutdown(elapsed)
            .await
            .expect("a repeat shutdown does nothing, whatever the deadline");
        let error = transport
            .recv(&mut buffer, soon())
            .await
            .expect_err("closed");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "recv after shutdown");
    }
}
