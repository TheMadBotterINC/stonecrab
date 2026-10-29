//! `LinuxSpiDev` over in-memory readers and a temporary file, with the MAX31855 on top.
//!
//! The node of a real SPI device is read by `read(2)`, so a reader stands in for it: a
//! channel-fed reader that blocks like a device does, and plain files. No device is touched.
//! Deadline expiry runs on Tokio's paused clock, which advances whenever every task is idle,
//! whether or not the worker thread is about to answer. A test therefore pauses the clock
//! only while a call that cannot be answered is pending, after a barrier has shown that the
//! worker is inside the device, and resumes the real clock before any call that must succeed.
//! Those calls carry a generous deadline, so a regression fails instead of hanging.
#![cfg(feature = "adapters-linux")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read};
    use std::path::PathBuf;
    use std::pin::pin;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Barrier};
    use std::task::{Context, Waker};
    use std::time::Duration;

    use stonecrab::boards::{LinuxSpiDev, Max31855, SpiBus};
    use stonecrab::transport::SystemClock;
    use stonecrab::{Deadline, ErrorKind, FailureCategory, Operation, PlcError};
    use tokio::sync::mpsc;

    /// A deadline on Tokio's clock, which is the plain clock unless a test has paused it.
    fn deadline_in(duration: Duration) -> Deadline {
        Deadline::after(tokio::time::Instant::now().into_std(), duration).expect("representable")
    }

    fn deadline() -> Deadline {
        deadline_in(Duration::from_secs(30))
    }

    /// A reader that answers each `read` with the next chunk sent to it, and blocks the thread
    /// that called it until one comes, as a device node does while a transfer runs. Dropping the
    /// sender ends the stream. When `entered` is set, the first `read` also waits there, so a
    /// test knows the worker is inside the device.
    struct Blocking {
        chunks: mpsc::Receiver<io::Result<Vec<u8>>>,
        entered: Option<Arc<Barrier>>,
    }

    impl Read for Blocking {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                entered.wait();
            }
            let Some(next) = self.chunks.blocking_recv() else {
                return Ok(0);
            };
            let bytes = next?;
            let count = bytes.len().min(buf.len());
            buf.iter_mut()
                .zip(&bytes)
                .for_each(|(slot, byte)| *slot = *byte);
            Ok(count)
        }
    }

    type Chunks = mpsc::Sender<io::Result<Vec<u8>>>;

    fn start(entered: Option<Arc<Barrier>>) -> (Chunks, LinuxSpiDev) {
        let (sender, chunks) = mpsc::channel(8);
        let bus = LinuxSpiDev::from_device(Blocking { chunks, entered }).expect("worker starts");
        (sender, bus)
    }

    fn blocking() -> (Chunks, LinuxSpiDev) {
        start(None)
    }

    /// A bus whose first transfer announces itself at the returned barrier.
    fn announcing() -> (Chunks, LinuxSpiDev, Arc<Barrier>) {
        let entered = Arc::new(Barrier::new(2));
        let (sender, bus) = start(Some(Arc::clone(&entered)));
        (sender, bus, entered)
    }

    /// Start a read the device will not answer, wait until the worker is inside the device, then
    /// let the deadline pass on the paused clock. Returns the error and the fake time it took.
    async fn parked_read(
        bus: &mut LinuxSpiDev,
        entered: &Barrier,
        wait: Duration,
    ) -> (PlcError, Duration) {
        let started = tokio::time::Instant::now();
        let call = bus.read(4, deadline_in(wait));
        let mut call = pin!(call);
        {
            let mut cx = Context::from_waker(Waker::noop());
            assert!(
                call.as_mut().poll(&mut cx).is_pending(),
                "the command is queued and unanswered"
            );
        }
        entered.wait();
        let error = call.await.expect_err("the device never answers");
        (error, started.elapsed())
    }

    #[tokio::test]
    async fn a_read_returns_the_bytes_of_one_transfer() {
        let mut bus = LinuxSpiDev::from_device(Cursor::new(vec![0x3E, 0x8C, 0x19, 0x00, 7, 8]))
            .expect("worker starts");
        let first = bus.read(4, deadline()).await.expect("a transfer");
        assert_eq!(
            &first[..],
            &[0x3E, 0x8C, 0x19, 0x00],
            "the first four bytes"
        );
        let second = bus.read(2, deadline()).await.expect("the rest");
        assert_eq!(&second[..], &[7, 8], "the next two");
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (SPI failure)
    #[tokio::test]
    async fn a_failing_device_is_disconnected_and_keeps_the_os_error() {
        let (sender, mut bus) = blocking();
        sender
            .send(Err(io::Error::from(io::ErrorKind::PermissionDenied)))
            .await
            .expect("queued");
        let error = bus.read(4, deadline()).await.expect_err("the read failed");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Transport),
            "category"
        );
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Read),
            "operation"
        );
        let source = std::error::Error::source(&error).expect("the os error is kept");
        assert_eq!(
            source.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(io::ErrorKind::PermissionDenied),
            "source"
        );
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (SPI failure, short frame)
    #[tokio::test]
    async fn a_device_that_runs_out_before_the_transfer_ends_is_disconnected() {
        let mut bus =
            LinuxSpiDev::from_device(Cursor::new(vec![1, 2, 3, 4, 5, 6])).expect("worker starts");
        let frame = bus.read(4, deadline()).await.expect("a whole transfer");
        assert_eq!(&frame[..], &[1, 2, 3, 4], "the first transfer");
        let error = bus
            .read(4, deadline())
            .await
            .expect_err("two bytes of four");
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "a cut-short transfer"
        );
    }

    #[tokio::test]
    async fn a_length_the_node_cannot_carry_is_refused_before_anything_is_read() {
        let (sender, mut bus) = blocking();
        for len in [0, 4097, usize::MAX] {
            let error = bus.read(len, deadline()).await.expect_err("bad length");
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "length {len}");
            assert_eq!(
                error.diagnostics().category,
                Some(FailureCategory::Validation),
                "category"
            );
        }
        sender.send(Ok(vec![5; 4096])).await.expect("queued");
        let frame = bus
            .read(4096, deadline())
            .await
            .expect("the largest transfer");
        assert_eq!(frame.len(), 4096, "4096 bytes are allowed");
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (SPI timeout)
    #[tokio::test(start_paused = true)]
    async fn a_blocked_transfer_times_out_at_the_deadline_and_its_late_bytes_are_dropped() {
        let (sender, mut bus, entered) = announcing();
        let (error, elapsed) = parked_read(&mut bus, &entered, Duration::from_secs(3)).await;
        assert_eq!(error.kind(), ErrorKind::Timeout, "timeout");
        assert_eq!(
            elapsed,
            Duration::from_secs(3),
            "paused time ran to the deadline"
        );
        tokio::time::resume();
        // The transfer finishes late. Its bytes belong to the call that gave up, and the next
        // call gets its own transfer, not those.
        sender.send(Ok(vec![1, 1, 1, 1])).await.expect("late bytes");
        sender
            .send(Ok(vec![2, 2, 2, 2]))
            .await
            .expect("fresh bytes");
        let frame = bus.read(4, deadline()).await.expect("the next call");
        assert_eq!(
            &frame[..],
            &[2, 2, 2, 2],
            "the late transfer was not handed over"
        );
    }

    #[tokio::test]
    async fn an_elapsed_deadline_times_out_without_a_transfer() {
        let (sender, mut bus) = blocking();
        let error = bus
            .read(4, Deadline::at(tokio::time::Instant::now().into_std()))
            .await
            .expect_err("elapsed");
        assert_eq!(error.kind(), ErrorKind::Timeout, "timeout");
        sender.send(Ok(vec![3, 3, 3, 3])).await.expect("queued");
        let frame = bus.read(4, deadline()).await.expect("nothing was consumed");
        assert_eq!(
            &frame[..],
            &[3, 3, 3, 3],
            "the first transfer is the next call's"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_command_abandoned_in_the_queue_is_skipped_by_the_worker() {
        let (sender, mut bus, entered) = announcing();
        // The first call parks the worker inside the device and times out. The second queues a
        // command behind it and times out too. Both callers are gone when the worker is freed.
        let (first, _) = parked_read(&mut bus, &entered, Duration::from_secs(1)).await;
        assert_eq!(first.kind(), ErrorKind::Timeout, "the parked call");
        let second = bus
            .read(4, deadline_in(Duration::from_secs(1)))
            .await
            .expect_err("queued behind the parked transfer");
        assert_eq!(second.kind(), ErrorKind::Timeout, "the queued call");
        tokio::time::resume();
        sender
            .send(Ok(vec![1, 1, 1, 1]))
            .await
            .expect("frees the worker");
        sender
            .send(Ok(vec![2, 2, 2, 2]))
            .await
            .expect("answers the live call");
        let frame = bus.read(4, deadline()).await.expect("the live call");
        assert_eq!(
            &frame[..],
            &[2, 2, 2, 2],
            "the abandoned command read nothing"
        );
    }

    #[tokio::test]
    async fn a_dropped_sender_ends_the_stream_as_a_disconnect() {
        let (sender, mut bus) = blocking();
        drop(sender);
        let error = bus.read(4, deadline()).await.expect_err("end of stream");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "end of stream");
    }

    /// A reader whose worker panics on the first read.
    struct Panicking;

    impl Read for Panicking {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            panic!("the device driver panicked");
        }
    }

    #[tokio::test]
    async fn a_worker_that_has_died_is_a_disconnect_not_a_hang() {
        let mut bus = LinuxSpiDev::from_device(Panicking).expect("worker starts");
        let first = bus
            .read(4, deadline())
            .await
            .expect_err("the worker panicked");
        assert_eq!(
            first.kind(),
            ErrorKind::Disconnected,
            "the reply channel closed"
        );
        let second = bus
            .read(4, deadline())
            .await
            .expect_err("the worker is gone");
        assert_eq!(
            second.kind(),
            ErrorKind::Disconnected,
            "the command channel closed"
        );
    }

    /// A file of `bytes` in the temporary directory, removed when dropped.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let name = format!(
                "stonecrab-spidev-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(name);
            std::fs::write(&path, bytes).expect("a temporary file");
            Self(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            drop(std::fs::remove_file(&self.0));
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (unavailable device)
    #[tokio::test]
    async fn opening_a_node_reads_it_and_a_missing_node_is_disconnected() {
        let file = TempFile::new(&[0x3E, 0x8C, 0x19, 0x00]);
        let mut bus = LinuxSpiDev::open(&file.0).expect("opens");
        let frame = bus.read(4, deadline()).await.expect("a transfer");
        assert_eq!(&frame[..], &[0x3E, 0x8C, 0x19, 0x00], "the node's bytes");

        let missing = std::env::temp_dir().join("stonecrab-spidev-no-such-node");
        let error = LinuxSpiDev::open(&missing).expect_err("no such node");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Read),
            "operation"
        );
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (unavailable device)
    #[tokio::test]
    async fn the_converter_over_a_node_reads_a_temperature_and_refuses_an_empty_line() {
        let good = TempFile::new(&[0x3E, 0x8C, 0x19, 0x00]);
        let bus = LinuxSpiDev::open(&good.0).expect("opens");
        let mut sensor = Max31855::new(Box::new(bus), Arc::new(SystemClock::default()));
        let reading = sensor.read(deadline()).await.expect("a reading");
        assert_eq!(reading.thermocouple_celsius(), Some(1000.75), "temperature");
        assert_eq!(reading.cold_junction_celsius(), 25.0, "cold junction");

        // A line that reads as zeros: nothing answers.
        let empty = TempFile::new(&[0, 0, 0, 0]);
        let bus = LinuxSpiDev::open(&empty.0).expect("opens");
        let mut sensor = Max31855::new(Box::new(bus), Arc::new(SystemClock::default()));
        let error = sensor.read(deadline()).await.expect_err("no device");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "no device answered");
    }
}
