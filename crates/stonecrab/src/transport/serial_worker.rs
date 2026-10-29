//! The blocking worker thread that owns a serial port (coding standards sections 8.4, 8.7, 8.8).
//!
//! A serial port is read and written with blocking calls, so one dedicated OS thread owns it
//! and the async side talks to that thread over a bounded Tokio channel. Each command carries
//! a `oneshot` for its result; the async side races that result against the caller's deadline
//! (`deadline::within`) and the worker never reads a clock. A dropped or timed-out call closes
//! its `oneshot`, and the worker notices at its next poll slice and abandons the work.
//!
//! The port's own timeouts are [`POLL`]: every read or write call returns within one slice
//! (with `serial2` on Unix the port is non-blocking and waits for readiness for at most the
//! timeout; on Windows the timeout bounds the transfer), so the worker re-checks whether the
//! caller is still waiting at least that often, and between the chunks of a write. Bytes the
//! worker read go into `stash`, shared with the async side; the `oneshot` of a read only says
//! that bytes (or the end of the stream) are ready, and the caller takes them from `stash` when
//! it next runs. Nothing is in flight inside a droppable channel, so a dropped `recv` loses
//! nothing: the bytes wait in `stash` and are the first thing the next read returns.

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

/// How long the port waits in one read or write call, and so how long the worker takes to
/// notice that its caller has gone. The factory sets it as both the read and the write
/// timeout of the port.
pub(super) const POLL: Duration = Duration::from_millis(20);

/// Commands queued ahead of the worker. A transport has one owner and runs one call at a time,
/// so the queue only ever holds the live command plus a command abandoned by a call that timed
/// out while the worker was busy; a full queue makes the next call wait inside its deadline.
const COMMAND_CAPACITY: usize = 2;

/// The most bytes offered to one port write. A write call returns once the line has taken
/// some bytes or the write timeout has passed, so a stalled or flow-controlled line costs
/// the worker one slice per call, never the whole frame; the chunk keeps the work of one call
/// small on a slow line (Windows bounds a write by its timeout, not by the bytes taken) and
/// the worker checks for an abandoned caller between chunks.
const WRITE_CHUNK: usize = 256;

/// The most bytes one read takes from the port; a read returns at most this many.
const READ_CHUNK: usize = 1024;

enum Command {
    /// Make between one and `limit` bytes (`limit` is at least one) ready in the stash; an
    /// answer with the stash still empty is the end of the stream.
    Read {
        limit: usize,
        done: oneshot::Sender<io::Result<()>>,
    },
    /// Write all of `bytes`.
    Write {
        bytes: Bytes,
        done: oneshot::Sender<io::Result<()>>,
    },
}

/// The async side's handle to the worker thread.
///
/// Dropping it closes the command channel; the worker then finishes its current slice and
/// drops the port, but nothing waits for it. [`Worker::stop`] does wait.
#[derive(Debug)]
pub(super) struct Worker {
    commands: Option<mpsc::Sender<Command>>,
    stash: Stash,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Start the thread that owns `port`.
    ///
    /// `port` must give up a read or write after a bounded time, ideally [`POLL`] (a port
    /// with that timeout reports `TimedOut`), or the worker cannot notice an abandoned call
    /// until the port returns.
    pub(super) fn spawn<P>(port: P) -> io::Result<Self>
    where
        P: Read + Write + Send + 'static,
    {
        let (commands, receiver) = mpsc::channel(COMMAND_CAPACITY);
        let stash = Stash::default();
        let worker_stash = Arc::clone(&stash);
        let thread = thread::Builder::new()
            .name("stonecrab-serial".to_owned())
            .spawn(move || run(port, receiver, &worker_stash))?;
        Ok(Self {
            commands: Some(commands),
            stash,
            thread: Some(thread),
        })
    }

    /// Read up to `limit` bytes. Cancel-safe: bytes read for a dropped call are kept for the
    /// next one.
    pub(super) async fn read(&self, limit: usize) -> io::Result<Vec<u8>> {
        let (done, reply) = oneshot::channel();
        self.request(Command::Read { limit, done }, reply).await?;
        Ok(take(&self.stash, limit))
    }

    /// Close the command channel now, so the worker ends after its current slice.
    pub(super) fn close(&mut self) {
        self.commands = None;
    }

    /// Write all of `bytes`. Not cancel-safe: a dropped call may have written a prefix.
    pub(super) async fn write(&self, bytes: Bytes) -> io::Result<()> {
        let (done, reply) = oneshot::channel();
        self.request(Command::Write { bytes, done }, reply).await
    }

    async fn request<T>(
        &self,
        command: Command,
        reply: oneshot::Receiver<io::Result<T>>,
    ) -> io::Result<T> {
        let sent = match &self.commands {
            Some(commands) => commands.send(command).await.is_ok(),
            None => false,
        };
        if !sent {
            return Err(stopped());
        }
        reply.await.unwrap_or_else(|_closed| Err(stopped()))
    }

    /// Close the command channel and wait for the thread to finish, which drops the port.
    ///
    /// The wait runs on Tokio's blocking pool so the runtime thread never blocks. If the
    /// caller gives up first, the join finishes there by itself.
    pub(super) async fn stop(&mut self) -> io::Result<()> {
        self.close();
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        let joined = tokio::task::spawn_blocking(move || thread.join())
            .await
            .map_err(io::Error::other)?;
        joined.map_err(|_panic| io::Error::other("the serial worker panicked"))
    }
}

/// Bytes the worker has read and the async side has not yet taken.
type Stash = Arc<Mutex<Vec<u8>>>;

fn lock(stash: &Stash) -> MutexGuard<'_, Vec<u8>> {
    stash.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Take up to `limit` bytes from the front of `stash`.
fn take(stash: &Stash, limit: usize) -> Vec<u8> {
    let mut bytes = lock(stash);
    let count = limit.min(bytes.len());
    bytes.drain(..count).collect()
}

fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "the serial worker has stopped")
}

/// The worker loop: serve commands until the channel closes.
fn run<P: Read + Write>(mut port: P, mut commands: mpsc::Receiver<Command>, stash: &Stash) {
    let mut scratch = vec![0_u8; READ_CHUNK];
    while let Some(command) = commands.blocking_recv() {
        match command {
            Command::Read { limit, done } => {
                serve_read(&mut port, limit, done, &mut scratch, stash);
            }
            Command::Write { bytes, done } => {
                if let Some(result) = write_all(&mut port, &bytes, &done) {
                    finish(done, result);
                }
            }
        }
    }
}

/// Whether a port call that fails this way should simply be tried again: a timeout (an idle
/// line, or a full one), a signal, or a non-blocking call that found the line not ready after
/// all.
fn is_retry(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}

/// Write all of `bytes`, or `None` if the caller goes away first.
///
/// A write that takes nothing (`Ok(0)`, which some platforms return on a timeout) is tried
/// again like a `TimedOut` one and is never an error: the caller's deadline ends a stalled
/// line. Each call is bounded by the port's write timeout and the caller is looked for before
/// every call, so a caller whose deadline has passed leaves the worker free within one slice
/// even if the line never takes a byte.
fn write_all<P: Write>(
    port: &mut P,
    bytes: &[u8],
    done: &oneshot::Sender<io::Result<()>>,
) -> Option<io::Result<()>> {
    let mut rest = bytes;
    while !rest.is_empty() {
        if done.is_closed() {
            return None;
        }
        let (chunk, _) = rest.split_at(rest.len().min(WRITE_CHUNK));
        match port.write(chunk) {
            // A write that timed out may be reported as `Ok(0)`; the caller's deadline decides.
            Ok(0) => thread::yield_now(),
            Ok(written) => rest = rest.get(written..).unwrap_or_default(),
            Err(error) if is_retry(&error) => thread::yield_now(),
            Err(error) => return Some(Err(error)),
        }
    }
    Some(Ok(()))
}

/// Answer one read: say that bytes are stashed, reading the next ones from the port if none are.
///
/// A port that reports the peer gone (a zero-length read or a broken pipe) is answered with
/// nothing stashed, the end of the stream; any other port error is passed on. The bytes stay
/// in `stash` whether or not the caller is still there to take them.
fn serve_read<P: Read>(
    port: &mut P,
    limit: usize,
    done: oneshot::Sender<io::Result<()>>,
    scratch: &mut [u8],
    stash: &Stash,
) {
    loop {
        if !lock(stash).is_empty() {
            return finish(done, Ok(()));
        }
        if done.is_closed() {
            return;
        }
        let (window, _) = scratch.split_at_mut(limit.min(scratch.len()));
        match port.read(window) {
            Ok(0) => return finish(done, Ok(())),
            Ok(read) => lock(stash).extend(scratch.iter().take(read)),
            Err(error) if is_retry(&error) => thread::yield_now(),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                return finish(done, Ok(()));
            }
            Err(error) => return finish(done, Err(error)),
        }
    }
}

/// Send a result to a caller that may already have gone.
fn finish<T>(done: oneshot::Sender<T>, result: T) {
    drop(done.send(result));
}

/// A port that plays back a script, for the unit tests of the worker and the transport.
///
/// It stands in for a `std::io::Read + Write` port, not for a Stonecrab trait. A read with
/// nothing scripted behaves like an idle line: it yields and reports `TimedOut`.
#[cfg(test)]
pub(super) mod scripted {
    use std::collections::VecDeque;
    use std::io::{self, Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::thread;

    /// What one `read` does.
    #[derive(Debug)]
    pub(in crate::transport) enum ReadStep {
        /// Return these bytes, as many as fit; the rest is returned by the next read.
        Data(Vec<u8>),
        /// Return `Ok(0)`.
        Eof,
        /// Fail with this kind of error.
        Fail(io::ErrorKind),
        /// Panic, to end the worker thread.
        Panic,
    }

    /// What one `write` does.
    #[derive(Debug)]
    pub(in crate::transport) enum WriteStep {
        /// Take at most this many bytes.
        Accept(u8),
        /// Return `Ok(0)`.
        Zero,
        /// Fail with this kind of error.
        Fail(io::ErrorKind),
        /// Report `TimedOut` for ever, as a stalled flow-controlled line does.
        Stall,
    }

    /// Everything the port was asked to write.
    pub(in crate::transport) type Written = Arc<Mutex<Vec<u8>>>;

    #[derive(Debug)]
    pub(in crate::transport) struct ScriptedPort {
        reads: VecDeque<ReadStep>,
        writes: VecDeque<WriteStep>,
        written: Written,
        write_calls: Arc<AtomicUsize>,
    }

    impl ScriptedPort {
        /// A port that plays `reads` and `writes`, and the record of what it was given.
        pub(in crate::transport) fn new(
            reads: Vec<ReadStep>,
            writes: Vec<WriteStep>,
        ) -> (Self, Written) {
            let written = Written::default();
            let port = Self {
                reads: reads.into(),
                writes: writes.into(),
                written: Arc::clone(&written),
                write_calls: Arc::default(),
            };
            (port, written)
        }

        /// How many times the port has been asked to write, counting every attempt.
        pub(in crate::transport) fn write_calls(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.write_calls)
        }
    }

    impl Read for ScriptedPort {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.reads.pop_front() {
                Some(ReadStep::Data(mut bytes)) => {
                    let taken = buf.len().min(bytes.len());
                    let rest = bytes.split_off(taken);
                    buf.iter_mut()
                        .zip(&bytes)
                        .for_each(|(slot, byte)| *slot = *byte);
                    if !rest.is_empty() {
                        self.reads.push_front(ReadStep::Data(rest));
                    }
                    Ok(taken)
                }
                Some(ReadStep::Eof) => Ok(0),
                Some(ReadStep::Fail(kind)) => Err(kind.into()),
                Some(ReadStep::Panic) => panic!("scripted panic in the port"),
                None => {
                    thread::yield_now();
                    Err(io::ErrorKind::TimedOut.into())
                }
            }
        }
    }

    impl Write for ScriptedPort {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.write_calls.fetch_add(1, Ordering::SeqCst);
            let accepted = match self.writes.pop_front() {
                Some(WriteStep::Accept(most)) => buf.len().min(usize::from(most)),
                Some(WriteStep::Zero) => 0,
                Some(WriteStep::Fail(kind)) => return Err(kind.into()),
                Some(WriteStep::Stall) => {
                    self.writes.push_front(WriteStep::Stall);
                    thread::yield_now();
                    return Err(io::ErrorKind::TimedOut.into());
                }
                None => buf.len(),
            };
            self.written
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(buf.iter().take(accepted));
            Ok(accepted)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll, Wake, Waker};

    use super::scripted::{ReadStep, ScriptedPort, WriteStep};
    use super::*;

    fn read_with(
        port: &mut ScriptedPort,
        limit: usize,
        stash: &Stash,
    ) -> Option<io::Result<Vec<u8>>> {
        let (done, mut reply) = oneshot::channel();
        serve_read(port, limit, done, &mut [0_u8; 8], stash);
        let answer = reply.try_recv().ok()?;
        Some(answer.map(|()| take(stash, limit)))
    }

    #[test]
    fn a_read_returns_at_most_the_limit_and_keeps_the_rest_in_order() {
        let (mut port, _) = ScriptedPort::new(vec![ReadStep::Data(b"abcdef".to_vec())], vec![]);
        let stash = Stash::default();
        let first = read_with(&mut port, 4, &stash)
            .expect("answered")
            .expect("bytes");
        assert_eq!(first, b"abcd", "limited to the window");
        let second = read_with(&mut port, 4, &stash)
            .expect("answered")
            .expect("bytes");
        assert_eq!(second, b"ef", "the rest follows");
        assert!(lock(&stash).is_empty(), "everything was delivered");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (peer close)
    #[test]
    fn a_hang_up_is_an_empty_reply_and_other_port_errors_are_passed_on() {
        for step in [ReadStep::Eof, ReadStep::Fail(io::ErrorKind::BrokenPipe)] {
            let (mut port, _) = ScriptedPort::new(vec![step], vec![]);
            let reply = read_with(&mut port, 4, &Stash::default()).expect("answered");
            assert!(reply.expect("not an error").is_empty(), "end of stream");
        }
        let (mut port, _) = ScriptedPort::new(vec![ReadStep::Fail(io::ErrorKind::Other)], vec![]);
        let reply = read_with(&mut port, 4, &Stash::default()).expect("answered");
        assert_eq!(
            reply.expect_err("an error").kind(),
            io::ErrorKind::Other,
            "passed on"
        );
    }

    #[test]
    fn a_timed_out_interrupted_or_not_ready_read_is_tried_again() {
        let steps = vec![
            ReadStep::Fail(io::ErrorKind::TimedOut),
            ReadStep::Fail(io::ErrorKind::Interrupted),
            ReadStep::Fail(io::ErrorKind::WouldBlock),
            ReadStep::Data(b"ok".to_vec()),
        ];
        let (mut port, _) = ScriptedPort::new(steps, vec![]);
        let reply = read_with(&mut port, 4, &Stash::default()).expect("answered");
        assert_eq!(reply.expect("bytes"), b"ok", "the retries are invisible");
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (bytes read for a gone caller)
    #[test]
    fn bytes_for_a_caller_that_has_gone_stay_stashed_in_order() {
        let (mut port, _) = ScriptedPort::new(vec![], vec![]);
        let stash = Stash::default();
        lock(&stash).extend(b"abc");
        let (done, reply) = oneshot::channel();
        drop(reply);
        serve_read(&mut port, 2, done, &mut [0_u8; 8], &stash);
        assert_eq!(*lock(&stash), b"abc", "undelivered bytes stay in front");

        let (mut port, _) = ScriptedPort::new(vec![ReadStep::Data(b"x".to_vec())], vec![]);
        let (done, reply) = oneshot::channel();
        drop(reply);
        serve_read(&mut port, 2, done, &mut [0_u8; 8], &Stash::default());
        let reply = read_with(&mut port, 2, &Stash::default()).expect("answered");
        assert_eq!(
            reply.expect("bytes"),
            b"x",
            "a gone caller did not touch the port"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (answer already sent)
    #[test]
    fn bytes_answered_before_the_caller_is_dropped_are_not_lost() {
        let (mut port, _) = ScriptedPort::new(vec![ReadStep::Data(b"abcd".to_vec())], vec![]);
        let stash = Stash::default();
        let (done, reply) = oneshot::channel();
        serve_read(&mut port, 8, done, &mut [0_u8; 8], &stash);
        drop(reply);
        let (done, mut reply) = oneshot::channel();
        serve_read(&mut port, 8, done, &mut [0_u8; 8], &stash);
        assert!(reply.try_recv().expect("answered").is_ok(), "ready");
        assert_eq!(take(&stash, 8), b"abcd", "the next read still has them");
    }

    #[test]
    fn a_write_is_finished_in_pieces_and_retried_after_a_timeout() {
        let writes = vec![
            WriteStep::Accept(2),
            WriteStep::Fail(io::ErrorKind::TimedOut),
            WriteStep::Accept(2),
        ];
        let (mut port, written) = ScriptedPort::new(vec![], writes);
        let (done, _reply) = oneshot::channel();
        let outcome = write_all(&mut port, b"abcde", &done).expect("not abandoned");
        outcome.expect("written");
        port.flush().expect("the scripted port flushes trivially");
        assert_eq!(
            *written.lock().expect("record"),
            b"abcde",
            "every byte once, in order"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (a write that returns Ok(0))
    #[test]
    fn a_failed_write_is_an_error_a_zero_or_not_ready_write_is_retried_and_a_gone_caller_stops_it()
    {
        let (done, _reply) = oneshot::channel();
        let steps = vec![
            WriteStep::Zero,
            WriteStep::Zero,
            WriteStep::Fail(io::ErrorKind::WouldBlock),
        ];
        let (mut port, written) = ScriptedPort::new(vec![], steps);
        write_all(&mut port, b"ab", &done)
            .expect("answered")
            .expect("a port that took nothing is tried again, and a zero write is no error");
        assert_eq!(*written.lock().expect("record"), b"ab", "then written");
        let steps = vec![WriteStep::Fail(io::ErrorKind::BrokenPipe)];
        let (mut port, _) = ScriptedPort::new(vec![], steps);
        let error = write_all(&mut port, b"ab", &done)
            .expect("answered")
            .expect_err("broken");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe, "passed on");

        let (done, reply) = oneshot::channel();
        drop(reply);
        let (mut port, written) = ScriptedPort::new(vec![], vec![]);
        assert!(write_all(&mut port, b"ab", &done).is_none(), "abandoned");
        assert!(
            written.lock().expect("record").is_empty(),
            "nothing was written"
        );
    }

    #[test]
    fn a_frame_is_offered_to_the_port_in_small_chunks() {
        struct Largest(usize, Vec<u8>);
        impl Write for Largest {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0 = self.0.max(buf.len());
                self.1.extend(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let frame: Vec<u8> = (0..=u8::MAX).cycle().take(WRITE_CHUNK * 3 + 5).collect();
        let (done, _reply) = oneshot::channel();
        let mut port = Largest(0, Vec::new());
        write_all(&mut port, &frame, &done)
            .expect("answered")
            .expect("written");
        assert_eq!(port.0, WRITE_CHUNK, "no write is larger than a chunk");
        assert_eq!(port.1, frame, "every byte once, in order");
    }

    #[tokio::test]
    async fn the_worker_serves_writes_and_reads_and_stops_on_request() {
        let (port, written) = ScriptedPort::new(vec![ReadStep::Data(b"reply".to_vec())], vec![]);
        let mut worker = Worker::spawn(port).expect("spawn");
        worker
            .write(Bytes::from_static(b"request"))
            .await
            .expect("write");
        assert_eq!(*written.lock().expect("record"), b"request", "written");
        assert_eq!(worker.read(16).await.expect("read"), b"reply", "read");

        worker.stop().await.expect("stop joins the thread");
        worker
            .stop()
            .await
            .expect("a second stop has nothing to join");
        let error = worker.read(1).await.expect_err("stopped");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe, "read after stop");
        let error = worker.write(Bytes::new()).await.expect_err("stopped");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe, "write after stop");
    }

    /// Poll `future` once with a waker that does nothing, so a command is sent and the reply
    /// is awaited, and keep the future to drop later.
    fn poll_once<F: Future>(future: &mut Pin<Box<F>>) -> Poll<F::Output> {
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
    }

    /// A waker that records that it was woken, which a `oneshot` does when it is completed.
    struct Flag(AtomicBool);

    impl Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// Let the worker thread run until `ready` holds, giving up after a bounded number of
    /// yields so that a regression fails the test instead of hanging it.
    async fn until(ready: impl Fn() -> bool) {
        for _ in 0..50_000_000_u32 {
            if ready() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the worker did not get there");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (a write that never completes)
    #[tokio::test]
    async fn a_write_on_a_stalled_line_ends_within_a_slice_of_its_caller_leaving() {
        let (port, written) = ScriptedPort::new(vec![], vec![WriteStep::Stall]);
        let calls = port.write_calls();
        let mut worker = Worker::spawn(port).expect("spawn");
        let mut write = Box::pin(worker.write(Bytes::from_static(b"frame")));
        assert!(poll_once(&mut write).is_pending(), "the line takes nothing");
        until(|| calls.load(Ordering::SeqCst) >= 3).await;

        // The caller's deadline passes: its future is dropped while the worker is inside the
        // write loop, retrying a port that only ever times out.
        drop(write);
        worker
            .stop()
            .await
            .expect("the worker left the write and the thread was joined");
        assert!(
            written.lock().expect("record").is_empty(),
            "nothing went out"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (dropped after the worker answered)
    #[tokio::test]
    async fn a_read_dropped_after_the_worker_answered_loses_nothing() {
        let (port, _) = ScriptedPort::new(vec![ReadStep::Data(b"abcdef".to_vec())], vec![]);
        let mut worker = Worker::spawn(port).expect("spawn");
        let mut read = Box::pin(worker.read(4));
        let woken = Arc::new(Flag(AtomicBool::new(false)));
        {
            // The worker needs the stash lock to answer, so the first poll cannot see a reply.
            let _held = lock(&worker.stash);
            let waker = Waker::from(Arc::clone(&woken));
            let polled = read.as_mut().poll(&mut Context::from_waker(&waker));
            assert!(polled.is_pending(), "sent, not answered");
        }
        // The oneshot wakes the waiter only when the worker completes it: the answer is in.
        until(|| woken.0.load(Ordering::SeqCst)).await;

        // The reply is ready, the caller has not run again, and the future goes away.
        drop(read);
        assert_eq!(worker.read(2).await.expect("read"), b"ab", "in order");
        assert_eq!(
            worker.read(8).await.expect("read"),
            b"cd",
            "the rest of the stash"
        );
        assert_eq!(worker.read(8).await.expect("read"), b"ef", "then the port");
        worker.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn a_worker_whose_port_panics_reports_it_to_the_caller_and_to_stop() {
        let (port, _) = ScriptedPort::new(vec![ReadStep::Panic], vec![]);
        let mut worker = Worker::spawn(port).expect("spawn");
        let error = worker.read(1).await.expect_err("the thread died");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe, "no reply came");
        let error = worker.stop().await.expect_err("the join reports the panic");
        assert_eq!(
            error.to_string(),
            "the serial worker panicked",
            "stop names the panic"
        );
    }
}
