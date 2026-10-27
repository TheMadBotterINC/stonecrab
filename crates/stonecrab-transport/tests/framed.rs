//! `FramedTransport` over `FakeTransport` and `FakeClock`.
//!
//! Timing is on the fake clock only (decision D21): nothing here sleeps, and a future
//! that could never finish fails the test instead of hanging it. The framer is the
//! testkit's `LengthPrefixedFramer` (a 2-byte big-endian length, then the payload); its
//! wire form of `abc` is `00 03 61 62 63`.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use stonecrab_core::{
        Clock, Deadline, ErrorKind, FailureCategory, PlcError, ResourceLimits, Result,
    };
    use stonecrab_testkit::{
        FakeClock, FakeTransport, FakeTransportProbe, LengthPrefixedFramer, Script, Step,
        block_on_with,
    };
    use stonecrab_transport::FramedTransport;

    type Framed = FramedTransport<Vec<u8>, LengthPrefixedFramer>;

    /// The wire form of `payload` under the test framer.
    fn wire(payload: &[u8]) -> Vec<u8> {
        let length = u16::try_from(payload.len()).expect("test payloads are short");
        let mut bytes = length.to_be_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    fn limits(frame: usize, buffered: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_frame_bytes(frame)
            .max_buffered_bytes(buffered)
            .build()
            .expect("valid limits")
    }

    struct Rig {
        clock: FakeClock,
        framed: Framed,
        probe: FakeTransportProbe,
    }

    impl Rig {
        /// Roomy limits (frame 256, buffer 1024) and a framer maximum of 64.
        fn new(steps: Vec<Step>) -> Self {
            Self::with(steps, 64, limits(256, 1024))
        }

        fn with(steps: Vec<Step>, framer_max: usize, limits: ResourceLimits) -> Self {
            let clock = FakeClock::default();
            let transport = FakeTransport::new(Script::new(steps), clock.clone(), limits);
            let probe = transport.probe();
            let framed =
                FramedTransport::new(Box::new(transport), LengthPrefixedFramer::new(framer_max));
            Self {
                clock,
                framed,
                probe,
            }
        }

        fn deadline(&self, seconds: u64) -> Deadline {
            Deadline::after(self.clock.now(), Duration::from_secs(seconds)).expect("representable")
        }

        fn next(&mut self, seconds: u64) -> Result<Vec<u8>> {
            let deadline = self.deadline(seconds);
            block_on_with(&self.clock, self.framed.next_frame(deadline))
        }

        fn send(&mut self, payload: &[u8]) -> Result<()> {
            let deadline = self.deadline(5);
            block_on_with(
                &self.clock,
                self.framed.send_frame(&payload.to_vec(), deadline),
            )
        }

        fn resync(&mut self, seconds: u64) -> Result<()> {
            let deadline = self.deadline(seconds);
            block_on_with(&self.clock, self.framed.resync(deadline))
        }
    }

    fn error_of<T>(result: Result<T>) -> PlcError {
        result.err().expect("an error")
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing, peer close at each framing boundary
    #[test]
    fn a_frame_delivered_in_three_pieces_is_assembled() {
        let frame = wire(b"hello");
        let mut rig = Rig::new(vec![Step::ReplyFragmented(vec![
            frame.get(..1).expect("first piece").to_vec(),
            frame.get(1..4).expect("second piece").to_vec(),
            frame.get(4..).expect("third piece").to_vec(),
        ])]);
        assert_eq!(rig.next(5).expect("frame"), b"hello", "payload");
        assert_eq!(rig.framed.buffered_len(), 0, "nothing left over");
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing, peer close at each framing boundary
    #[test]
    fn two_frames_in_one_delivery_come_out_of_two_calls() {
        let mut rig = Rig::new(vec![
            Step::ReplyCoalescedWithNext,
            Step::Reply(wire(b"one")),
            Step::Reply(wire(b"two")),
        ]);
        assert_eq!(rig.next(5).expect("first"), b"one", "first frame");
        assert_eq!(
            rig.framed.buffered_len(),
            wire(b"two").len(),
            "the second frame waits in the buffer"
        );
        // The second frame is already buffered, so an elapsed deadline does not matter.
        let elapsed = Deadline::at(rig.clock.now());
        let second = block_on_with(&rig.clock, rig.framed.next_frame(elapsed)).expect("second");
        assert_eq!(second, b"two", "second frame without any I/O");
        assert_eq!(rig.framed.buffered_len(), 0, "buffer drained");
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing, peer close at each framing boundary
    #[test]
    fn a_delivery_ending_inside_the_next_frame_keeps_the_partial_bytes() {
        let mut bytes = wire(b"one");
        let second = wire(b"two");
        bytes.extend_from_slice(second.get(..3).expect("prefix"));
        let rest = second.get(3..).expect("tail").to_vec();
        let mut rig = Rig::new(vec![Step::ReplyFragmented(vec![bytes, rest])]);
        assert_eq!(rig.next(5).expect("first"), b"one", "first frame");
        assert_eq!(
            rig.framed.buffered_len(),
            3,
            "a partial second frame is kept"
        );
        assert_eq!(
            rig.next(5).expect("second"),
            b"two",
            "completed by the next read"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing, peer close at each framing boundary
    #[test]
    fn a_close_inside_a_frame_is_disconnected_at_every_cut() {
        let frame = wire(b"abc");
        // 0: before the header, 1: inside it, 2: after the header, 3-4: inside the payload.
        for cut in 0..frame.len() {
            let mut rig = Rig::new(vec![Step::CloseAt(cut), Step::Reply(frame.clone())]);
            let error = error_of(rig.next(5));
            assert_eq!(
                error.kind(),
                ErrorKind::Disconnected,
                "cut after {cut} bytes"
            );
            assert_eq!(
                error.diagnostics().category,
                Some(FailureCategory::Transport),
                "category for cut {cut}"
            );
            let again = error_of(rig.next(5));
            assert_eq!(
                again.kind(),
                ErrorKind::Disconnected,
                "still closed after {cut}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing, peer close at each framing boundary
    #[test]
    fn a_close_after_the_payload_delivers_the_frame_then_disconnects() {
        let frame = wire(b"abc");
        let mut rig = Rig::new(vec![Step::CloseAt(frame.len()), Step::Reply(frame)]);
        assert_eq!(
            rig.next(5).expect("complete frame"),
            b"abc",
            "frame before close"
        );
        let error = error_of(rig.next(5));
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "next call sees the close"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing, peer close at each framing boundary
    #[test]
    fn a_close_with_an_incomplete_frame_names_the_buffered_bytes() {
        let mut rig = Rig::new(vec![Step::CloseAt(3), Step::Reply(wire(b"abc"))]);
        let error = error_of(rig.next(5));
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "truncated then closed"
        );
        assert!(
            error.to_string().contains("3 undecoded bytes"),
            "message: {error}"
        );
        assert_eq!(
            rig.framed.buffered_len(),
            3,
            "the partial bytes are not dropped"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (buffer bound)
    #[test]
    fn a_full_buffer_without_a_frame_boundary_is_a_resource_limit() {
        // The framer would accept a 22 byte frame, but the buffer holds only 8.
        let mut rig = Rig::with(
            vec![Step::Reply(vec![0, 20, 1, 2, 3, 4, 5, 6])],
            64,
            limits(8, 8),
        );
        let error = error_of(rig.next(5));
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "buffer full");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Transport),
            "category"
        );
        assert_eq!(rig.framed.buffered_len(), 8, "nothing is discarded");
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads, oversized lengths (framing)
    #[test]
    fn a_header_over_the_frame_maximum_fails_before_the_payload_is_awaited() {
        let mut rig = Rig::with(vec![Step::Reply(vec![0xFF, 0xFF])], 16, limits(256, 1024));
        let started = rig.clock.now();
        let error = error_of(rig.next(5));
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "oversized declared length"
        );
        assert_eq!(
            Error::source(&error)
                .expect("the frame error is kept")
                .to_string(),
            "frame of 65537 bytes exceeds the frame limit",
            "source names the declared size"
        );
        assert_eq!(
            rig.clock.now(),
            started,
            "no time passed: the payload was not awaited"
        );
        assert_eq!(rig.framed.buffered_len(), 2, "the header stays buffered");
        let again = error_of(rig.next(5));
        assert_eq!(
            again.kind(),
            ErrorKind::ResourceLimit,
            "the error repeats until resync"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (deadline expiry)
    #[test]
    fn next_frame_times_out_at_the_deadline_and_the_next_call_continues() {
        let mut rig = Rig::new(vec![
            Step::Delay(Duration::from_secs(10)),
            Step::Reply(wire(b"late")),
        ]);
        let started = rig.clock.now();
        let deadline = rig.deadline(5);
        let error = error_of(block_on_with(&rig.clock, rig.framed.next_frame(deadline)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "deadline passed");
        assert_eq!(
            rig.clock.now() - started,
            Duration::from_secs(5),
            "waited exactly to the deadline"
        );
        assert_eq!(
            rig.next(20).expect("frame after the delay"),
            b"late",
            "usable afterwards"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (deadline expiry)
    #[test]
    fn a_timeout_in_the_middle_of_a_frame_keeps_the_received_prefix() {
        let frame = wire(b"abcd");
        let mut rig = Rig::new(vec![
            Step::ReplyFragmented(vec![frame.get(..3).expect("prefix").to_vec()]),
            Step::Delay(Duration::from_secs(10)),
            Step::Reply(frame.get(3..).expect("tail").to_vec()),
        ]);
        let error = error_of(rig.next(5));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "tail did not arrive in time"
        );
        assert_eq!(rig.framed.buffered_len(), 3, "prefix retained");
        assert_eq!(
            rig.next(20).expect("completed"),
            b"abcd",
            "same frame continues"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (frame bound on send)
    #[test]
    fn a_frame_over_the_framer_maximum_is_refused_before_sending() {
        let mut rig = Rig::with(Vec::new(), 8, limits(256, 1024));
        let error = error_of(rig.send(b"1234567"));
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "9 bytes on the wire > 8"
        );
        assert!(rig.probe.sent().is_empty(), "nothing reached the wire");
        rig.send(b"123456").expect("exactly the maximum");
        assert_eq!(rig.probe.sent().len(), 1, "the frame at the limit is sent");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (frame bound on send)
    #[test]
    fn a_frame_over_the_transport_frame_limit_is_refused_before_sending() {
        // The framer allows 64 bytes, the transport only 8.
        let mut rig = Rig::with(Vec::new(), 64, limits(8, 64));
        let error = error_of(rig.send(&[7; 20]));
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "22 bytes on the wire > 8"
        );
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Transport),
            "category"
        );
        assert!(rig.probe.sent().is_empty(), "sent() stays empty");
        rig.send(&[7; 6]).expect("a frame at the transport limit");
        assert_eq!(rig.probe.sent().len(), 1, "only the good frame is logged");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (frame bound on send)
    #[test]
    fn send_frame_puts_one_encoded_frame_in_the_sent_log_per_call() {
        let mut rig = Rig::new(Vec::new());
        rig.send(b"ping").expect("first");
        rig.send(b"").expect("empty payload");
        let sent: Vec<Vec<u8>> = rig
            .probe
            .sent()
            .iter()
            .map(|frame| frame.to_vec())
            .collect();
        assert_eq!(
            sent,
            vec![wire(b"ping"), wire(b"")],
            "one transport send per frame"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (deadline expiry)
    #[test]
    fn send_frame_with_an_elapsed_deadline_times_out_and_sends_nothing() {
        let mut rig = Rig::new(Vec::new());
        let elapsed = Deadline::at(rig.clock.now());
        let error = error_of(block_on_with(
            &rig.clock,
            rig.framed.send_frame(&b"x".to_vec(), elapsed),
        ));
        assert_eq!(error.kind(), ErrorKind::Timeout, "deadline already passed");
        assert!(rig.probe.sent().is_empty(), "nothing reached the wire");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (request and reply through the framing layer)
    #[test]
    fn a_request_and_its_fragmented_reply_round_trip() {
        let mut rig = Rig::new(vec![
            Step::Expect(wire(b"ping")),
            Step::ReplyFragmented(vec![vec![0, 4, b'p'], b"ong".to_vec()]),
        ]);
        rig.send(b"ping").expect("send");
        assert_eq!(rig.next(5).expect("reply"), b"pong", "reply");
        assert!(
            rig.probe.report().mismatches.is_empty(),
            "the script saw the exact bytes"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization
    #[test]
    fn resync_discards_garbage_until_a_boundary_then_next_frame_succeeds() {
        let mut rig = Rig::with(
            vec![
                Step::Reply(vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
                Step::Reply(wire(b"ok")),
            ],
            16,
            limits(256, 1024),
        );
        let error = error_of(rig.next(5));
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "garbage header is oversized"
        );
        rig.resync(5).expect("a boundary exists after the garbage");
        assert_eq!(rig.framed.buffered_len(), 0, "the garbage is gone");
        assert_eq!(
            rig.next(5).expect("frame after resync"),
            b"ok",
            "aligned again"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization
    #[test]
    fn resync_keeps_receiving_when_the_garbage_runs_out_before_a_boundary() {
        let mut rig = Rig::with(
            vec![
                Step::Reply(vec![0xFF, 0xFF, 0xFF]),
                Step::Delay(Duration::from_secs(2)),
                Step::Reply(wire(b"ok")),
            ],
            16,
            limits(256, 1024),
        );
        let started = rig.clock.now();
        rig.resync(5).expect("boundary after the delay");
        assert_eq!(
            rig.clock.now() - started,
            Duration::from_secs(2),
            "waited for the late bytes"
        );
        assert_eq!(rig.next(5).expect("frame"), b"ok", "frame");
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization
    #[test]
    fn resync_discards_from_a_full_buffer_that_holds_no_frame() {
        // Eight bytes fill the buffer: a header that wants 22 bytes, then noise.
        let mut rig = Rig::with(
            vec![
                Step::Reply(vec![0, 20, 1, 2, 3, 4, 5, 6]),
                Step::Reply(wire(b"ok")),
            ],
            64,
            limits(8, 8),
        );
        let error = error_of(rig.next(5));
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "buffer full");
        rig.resync(5).expect("resync slides past the noise");
        assert_eq!(rig.next(5).expect("frame"), b"ok", "frame after the noise");
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization
    #[test]
    fn resync_completes_a_frame_that_was_half_received_when_a_request_timed_out() {
        let frame = wire(b"late");
        let mut rig = Rig::new(vec![
            Step::ReplyFragmented(vec![frame.get(..3).expect("prefix").to_vec()]),
            Step::Delay(Duration::from_secs(10)),
            Step::Reply(frame.get(3..).expect("tail").to_vec()),
        ]);
        let error = error_of(rig.next(5));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "request timed out mid-frame"
        );
        rig.resync(20)
            .expect("the prefix is a valid header, so it is kept");
        assert_eq!(
            rig.next(1).expect("the stale frame"),
            b"late",
            "returned whole"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization
    #[test]
    fn resync_drops_the_frame_an_earlier_resync_was_holding() {
        let mut bytes = wire(b"one");
        bytes.extend_from_slice(&wire(b"two"));
        let mut rig = Rig::new(vec![Step::Reply(bytes)]);
        rig.resync(5).expect("first boundary");
        rig.resync(5).expect("second boundary");
        assert_eq!(
            rig.next(5).expect("frame"),
            b"two",
            "the first frame was discarded"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization
    #[test]
    fn resync_that_never_sees_a_frame_boundary_fails_disconnected_at_the_deadline() {
        let mut rig = Rig::with(vec![Step::Reply(vec![0xFF; 6])], 16, limits(256, 1024));
        let started = rig.clock.now();
        let error = error_of(rig.resync(5));
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "no boundary before the deadline"
        );
        assert_eq!(
            rig.clock.now() - started,
            Duration::from_secs(5),
            "failed at the deadline"
        );
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Transport),
            "category"
        );
        let source = Error::source(&error).expect("the transport timeout is kept");
        assert!(
            source.to_string().contains("deadline elapsed"),
            "source: {source}"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization
    #[test]
    fn resync_with_an_elapsed_deadline_and_nothing_buffered_is_disconnected() {
        let mut rig = Rig::new(vec![Step::Reply(wire(b"never read"))]);
        let elapsed = Deadline::at(rig.clock.now());
        let error = error_of(block_on_with(&rig.clock, rig.framed.resync(elapsed)));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "no time to receive");
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization, peer close at each framing boundary
    #[test]
    fn resync_when_the_peer_closes_is_disconnected() {
        let mut rig = Rig::new(vec![Step::CloseAt(2), Step::Reply(vec![0xFF; 6])]);
        let error = error_of(rig.resync(5));
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "closed while discarding"
        );
    }

    #[test]
    fn accessors_expose_the_transport_and_give_it_back() {
        let rig = Rig::with(Vec::new(), 64, limits(256, 1024));
        assert_eq!(
            rig.framed.transport().limits().max_buffered_bytes().get(),
            1024,
            "limits"
        );
        let debug = format!("{:?}", rig.framed);
        assert!(
            debug.contains("buffered: 0") && debug.contains("frame_ready: false"),
            "{debug}"
        );
        let mut transport = rig.framed.into_inner();
        let deadline = Deadline::after(rig.clock.now(), Duration::from_secs(1)).expect("deadline");
        block_on_with(&rig.clock, transport.shutdown(deadline)).expect("the transport is usable");
        assert!(rig.probe.is_shut_down(), "the same transport came back");
    }
}
