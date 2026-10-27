use std::fmt;
use std::mem;
use std::time::Duration;

/// A responder closure: given the bytes just received, the bytes to answer with, or
/// `None` to stay silent.
pub type Responder = Box<dyn FnMut(&[u8]) -> Option<Vec<u8>> + Send>;

/// A reactor closure: given the bytes just received, what the peer does about them.
pub type Reactor = Box<dyn FnMut(&[u8]) -> Reaction + Send>;

/// What a [`Step::React`] closure decides the peer does about one request.
///
/// A [`Responder`] can only answer or stay silent; a reactor can also end the
/// connection, which is how a fixture injects a lost link on demand. Both
/// [`FakeTransport`] and [`ScriptedPeer`] honour every reaction.
///
/// # Examples
/// ```
/// use stonecrab_testkit::{Reaction, Step};
///
/// // Answer the first request, then drop the link on the second.
/// let mut seen = 0_u8;
/// let step = Step::react(move |request| {
///     seen += 1;
///     if seen == 1 { Reaction::Reply(request.to_vec()) } else { Reaction::Close }
/// });
/// assert_eq!(format!("{step:?}"), "React(<reactor>)");
/// assert_eq!(format!("{:?}", Reaction::Silence), "Silence");
/// ```
///
/// [`FakeTransport`]: crate::FakeTransport
/// [`ScriptedPeer`]: crate::ScriptedPeer
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reaction {
    /// Send these bytes to the client in one write and keep the connection open.
    ///
    /// Armed modifiers ([`Step::Truncate`] and the like) apply to the reply, as they do
    /// to a [`Step::Respond`] answer.
    Reply(Vec<u8>),
    /// Send nothing and keep the connection open; the client runs into its deadline.
    ///
    /// The step stays armed for the next request, and armed modifiers wait for the next
    /// reply.
    Silence,
    /// Close the connection without answering.
    ///
    /// The client sees end of stream (a half-close on the loopback socket, an end of
    /// stream and then `Disconnected` on [`FakeTransport`](crate::FakeTransport)), exactly
    /// as after a trailing [`Step::CloseAt`]`(0)`. Nothing after the close runs and
    /// armed modifiers are dropped.
    Close,
    /// Send these bytes in one write, then close the connection.
    ///
    /// The client reads the bytes and then sees the end of stream, as with
    /// [`Step::CloseAt`] on a reply shorter than its cut. Armed modifiers apply to the
    /// reply, and an armed `CloseAt(n)` still cuts it at `n` bytes.
    ReplyThenClose(Vec<u8>),
}

/// One instruction of a [`Script`].
///
/// A script is a list of steps executed in order by a scripted peer: [`ScriptedPeer`] on
/// a loopback socket, [`FakeTransport`] in memory. Steps fall in three groups.
///
/// - **Input steps** wait for bytes from the client: [`Expect`](Self::Expect),
///   [`Respond`](Self::Respond) and [`React`](Self::React). A script stalled on an input
///   step sends nothing, so a client that waits for a reply runs into its deadline.
/// - **Output steps** make the peer speak: [`Reply`](Self::Reply) and
///   [`ReplyFragmented`](Self::ReplyFragmented).
/// - **Modifier steps** change the next output step and produce nothing themselves:
///   [`ReplyCoalescedWithNext`](Self::ReplyCoalescedWithNext),
///   [`CloseAt`](Self::CloseAt), [`Truncate`](Self::Truncate),
///   [`Corrupt`](Self::Corrupt), [`WrongTransactionId`](Self::WrongTransactionId) and
///   [`LateReply`](Self::LateReply). Modifiers pile up: each one applies, in script
///   order, to the next reply (the next `Reply`, `ReplyFragmented` or answer of a
///   `Respond`) and is then spent. A modifier with no reply after it is never applied,
///   except that a trailing `CloseAt` closes the connection when the script runs out.
/// - [`Delay`](Self::Delay) pauses the script itself.
///
/// A step is *run* when the script reaches it and finishes it; the indices of the steps
/// that ran are listed in [`ScriptReport::ran`].
///
/// # Examples
/// ```
/// use stonecrab_testkit::Step;
///
/// let steps = [
///     Step::Expect(b"ping".to_vec()),
///     Step::Truncate(2),
///     Step::Reply(b"pong".to_vec()),
/// ];
/// assert_eq!(steps.len(), 3);
/// assert_eq!(format!("{:?}", Step::respond(|_| None)), "Respond(<responder>)");
/// ```
///
/// [`ScriptedPeer`]: crate::ScriptedPeer
/// [`FakeTransport`]: crate::FakeTransport
pub enum Step {
    /// The next `bytes.len()` bytes received from the client must equal `bytes`.
    ///
    /// The step waits until that many bytes have arrived; bytes beyond them stay
    /// available to the next input step. In a strict script a mismatch is recorded in
    /// [`ScriptReport::mismatches`] and fails the script (see [`Script::strict`]); in a
    /// lenient script it is recorded and the script carries on, the bytes consumed
    /// either way.
    Expect(Vec<u8>),
    /// Send these bytes to the client in one write.
    ///
    /// An empty reply sends nothing.
    Reply(Vec<u8>),
    /// Send the pieces to the client as separate writes, in order.
    ///
    /// [`FakeTransport`](crate::FakeTransport) hands the client one piece per `recv`.
    /// [`ScriptedPeer`](crate::ScriptedPeer) flushes each piece with `TCP_NODELAY` set,
    /// but TCP may still merge pieces on the receiving side, so socket tests assert on
    /// the bytes received and not on how many reads delivered them.
    ReplyFragmented(Vec<Vec<u8>>),
    /// Hold the next reply back and send it in the same write as the reply after it.
    ///
    /// Written before the first of two `Reply`/`ReplyFragmented` steps, as in
    /// `[ReplyCoalescedWithNext, Reply(a), Reply(b)]`, which sends `a` followed by `b`
    /// as one write. Repeating the step adds one more reply to the write each time. If
    /// the step after the reply is not itself a reply, the reply is sent alone. Other
    /// modifiers apply to the first reply of the group.
    ReplyCoalescedWithNext,
    /// Pause the script before its next step.
    ///
    /// [`FakeTransport`](crate::FakeTransport) moves its script timeline on by this
    /// long, measured on the [`FakeClock`](crate::FakeClock): replies after the delay
    /// become readable that much later, and a `recv` whose deadline passes first fails
    /// with `Timeout`. [`ScriptedPeer`](crate::ScriptedPeer) sleeps on the Tokio timer
    /// for the duration capped at [`MAX_PEER_DELAY`](crate::MAX_PEER_DELAY), because
    /// loopback tests are about bytes and not timing (decision D21).
    Delay(Duration),
    /// Send only the first `n` bytes of the next reply, then close the connection.
    ///
    /// `CloseAt(0)` closes without sending anything, and a `CloseAt` that no reply follows
    /// closes the connection once the script has run out of steps, so `[Expect(..),
    /// CloseAt(0)]` ends in end of stream. A reply shorter than `n` is sent
    /// whole and then the connection closes. The client sees the bytes, then end of
    /// stream. Steps after the reply do not run.
    CloseAt(usize),
    /// Send only the first `n` bytes of the next reply and keep the connection open.
    ///
    /// The rest of the reply is dropped, so the client holds a partial frame and the
    /// peer carries on with the next step. A reply shorter than `n` is unchanged.
    Truncate(usize),
    /// Overwrite bytes of the next reply, starting at `offset`, with `bytes`.
    ///
    /// The reply keeps its length: bytes that would land past its end are ignored.
    Corrupt {
        /// Index of the first byte to overwrite.
        offset: usize,
        /// The replacement bytes.
        bytes: Vec<u8>,
    },
    /// Flip every bit of the first two bytes of the next reply.
    ///
    /// For a Modbus TCP reply those bytes are the MBAP transaction id, so a correlation
    /// test can inject a mismatched id without knowing the protocol. A reply shorter
    /// than two bytes has the bytes it has flipped.
    WrongTransactionId,
    /// Deliver the next reply this long after the script reached it.
    ///
    /// Unlike [`Delay`](Self::Delay) it does not hold up the script: replies stay in
    /// order on the byte stream, so a later reply cannot overtake a late one. On the
    /// peer the duration is capped like `Delay`'s.
    LateReply(Duration),
    /// Answer received requests with a closure.
    ///
    /// Each time bytes arrive the closure is called with all bytes received since the
    /// previous input step finished (one `send` on [`FakeTransport`](crate::FakeTransport),
    /// one socket read on [`ScriptedPeer`](crate::ScriptedPeer)). `Some(bytes)` is
    /// sent as one reply, `None` stays silent. The step never finishes: it answers every
    /// later request, so steps after it do not run, and one fixture can serve a whole
    /// contract suite. Armed modifiers apply to the next answer. It counts once in
    /// [`ScriptReport::ran`] and every call in [`ScriptReport::responses`]. A responder
    /// cannot end the connection; use [`React`](Self::React) for that.
    Respond(Responder),
    /// Decide what to do about each received request with a closure.
    ///
    /// Like [`Respond`](Self::Respond) the closure is called with all bytes received since
    /// the previous input step finished, the step never finishes and so serves every later
    /// request, armed modifiers apply to the next reply, and it counts once in
    /// [`ScriptReport::ran`] and every call in [`ScriptReport::responses`]. The closure
    /// returns a [`Reaction`]: reply, stay silent, close the connection, or reply and
    /// then close. After a close the script is over, so later requests are not reacted to.
    React(Reactor),
}

impl Step {
    /// A [`Respond`](Self::Respond) step from any closure.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Script, Step};
    ///
    /// // Echo every request back.
    /// let script = Script::new(vec![Step::respond(|request| Some(request.to_vec()))]);
    /// assert_eq!(script.steps.len(), 1);
    /// ```
    #[must_use]
    pub fn respond(responder: impl FnMut(&[u8]) -> Option<Vec<u8>> + Send + 'static) -> Self {
        Self::Respond(Box::new(responder))
    }

    /// A [`React`](Self::React) step from any closure.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Reaction, Script, Step};
    ///
    /// // Drop the link on any request.
    /// let script = Script::new(vec![Step::react(|_| Reaction::Close)]);
    /// assert_eq!(script.steps.len(), 1);
    /// ```
    #[must_use]
    pub fn react(reactor: impl FnMut(&[u8]) -> Reaction + Send + 'static) -> Self {
        Self::React(Box::new(reactor))
    }
}

impl fmt::Debug for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expect(bytes) => f.debug_tuple("Expect").field(bytes).finish(),
            Self::Reply(bytes) => f.debug_tuple("Reply").field(bytes).finish(),
            Self::ReplyFragmented(pieces) => {
                f.debug_tuple("ReplyFragmented").field(pieces).finish()
            }
            Self::ReplyCoalescedWithNext => f.write_str("ReplyCoalescedWithNext"),
            Self::Delay(by) => f.debug_tuple("Delay").field(by).finish(),
            Self::CloseAt(n) => f.debug_tuple("CloseAt").field(n).finish(),
            Self::Truncate(n) => f.debug_tuple("Truncate").field(n).finish(),
            Self::Corrupt { offset, bytes } => f
                .debug_struct("Corrupt")
                .field("offset", offset)
                .field("bytes", bytes)
                .finish(),
            Self::WrongTransactionId => f.write_str("WrongTransactionId"),
            Self::LateReply(by) => f.debug_tuple("LateReply").field(by).finish(),
            Self::Respond(_) => f.write_str("Respond(<responder>)"),
            Self::React(_) => f.write_str("React(<reactor>)"),
        }
    }
}

/// What a scripted peer does: [`Step`]s in order, and whether a mismatch is fatal.
///
/// The same script drives both doubles. Build one with [`Script::new`] (lenient) or
/// [`Script::strict`], or with a struct literal.
///
/// # Strict mode
/// When `strict` is set, the first [`Step::Expect`] whose bytes differ is recorded in
/// [`ScriptReport::mismatches`], [`ScriptReport::failed`] is set, and the peer closes the
/// connection: replies already queued are still delivered, nothing after the failed step
/// runs. A lenient script records the same mismatch and carries on.
///
/// # Examples
/// ```
/// use stonecrab_testkit::{Script, Step};
///
/// let script = Script::strict(vec![
///     Step::Expect(b"hello".to_vec()),
///     Step::Reply(b"world".to_vec()),
/// ]);
/// assert!(script.strict);
/// assert_eq!(script.steps.len(), 2);
/// ```
#[derive(Debug, Default)]
pub struct Script {
    /// The steps, run front to back.
    pub steps: Vec<Step>,
    /// Whether a mismatched [`Step::Expect`] fails the script.
    pub strict: bool,
}

impl Script {
    /// A lenient script: mismatches are recorded but do not stop it.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Script, Step};
    ///
    /// let script = Script::new(vec![Step::Reply(b"banner".to_vec())]);
    /// assert!(!script.strict);
    /// ```
    #[must_use]
    pub const fn new(steps: Vec<Step>) -> Self {
        Self {
            steps,
            strict: false,
        }
    }

    /// A strict script: the first mismatched [`Step::Expect`] fails it.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::Script;
    ///
    /// assert!(Script::strict(Vec::new()).strict);
    /// ```
    #[must_use]
    pub const fn strict(steps: Vec<Step>) -> Self {
        Self {
            steps,
            strict: true,
        }
    }
}

/// A [`Step::Expect`] that did not match.
///
/// # Examples
/// ```
/// use stonecrab_testkit::Mismatch;
///
/// let mismatch = Mismatch { step: 0, expected: vec![1], actual: vec![2] };
/// assert_eq!(mismatch.step, 0);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// Index of the `Expect` step in the script.
    pub step: usize,
    /// The bytes the step required.
    pub expected: Vec<u8>,
    /// The bytes that arrived in their place (the same length as `expected`).
    pub actual: Vec<u8>,
}

/// What a script saw and did, for tests to assert on.
///
/// Returned by [`ScriptedPeer::close`](crate::ScriptedPeer::close) and
/// [`FakeTransportProbe::report`](crate::FakeTransportProbe::report).
///
/// # Examples
/// ```
/// use stonecrab_testkit::ScriptReport;
///
/// let report = ScriptReport::default();
/// assert!(report.received.is_empty() && report.mismatches.is_empty() && !report.failed);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptReport {
    /// Every byte the peer received from the client, in order, including bytes the
    /// script did not consume.
    pub received: Vec<u8>,
    /// Every [`Step::Expect`] that did not match, in order.
    pub mismatches: Vec<Mismatch>,
    /// Indices of the steps that ran, in the order they ran. A [`Step::Respond`] or
    /// [`Step::React`] appears once, when it first ran.
    pub ran: Vec<usize>,
    /// How many times a [`Step::Respond`] or [`Step::React`] closure was called.
    pub responses: usize,
    /// True when a strict script stopped on a mismatch.
    pub failed: bool,
    /// True when the script closed the connection ([`Step::CloseAt`], a closing
    /// [`Reaction`] or a strict failure).
    pub closed: bool,
}

/// What the machine asks its runner to do next. Timing belongs to the runner.
#[derive(Debug, PartialEq, Eq)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) enum Action {
    /// Pause the script timeline.
    Delay(Duration),
    /// Deliver `chunks` as separate writes, `late` after the script reached them, then
    /// close the connection when `close` is set.
    Write {
        chunks: Vec<Vec<u8>>,
        late: Duration,
        close: bool,
    },
    /// Close the connection.
    Close,
}

/// A byte edit pending for the next reply.
#[derive(Debug)]
enum Edit {
    Truncate(usize),
    Corrupt { offset: usize, bytes: Vec<u8> },
    WrongTransactionId,
}

/// Modifiers armed for the next reply.
#[derive(Debug, Default)]
struct Armed {
    edits: Vec<Edit>,
    late: Duration,
    close_at: Option<usize>,
    coalesce: usize,
}

/// The pure state machine behind both doubles: it consumes received bytes and yields
/// [`Action`]s, with no I/O and no clock.
#[derive(Debug)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) struct Machine {
    steps: Vec<Step>,
    strict: bool,
    pos: usize,
    inbox: Vec<u8>,
    armed: Armed,
    report: ScriptReport,
    halted: bool,
}

/// The pieces of a `Reply` or `ReplyFragmented` step, cloned out of the script.
fn reply_pieces(step: Option<&Step>) -> Option<Vec<Vec<u8>>> {
    match step? {
        Step::Reply(bytes) => Some(vec![bytes.clone()]),
        Step::ReplyFragmented(pieces) => Some(pieces.clone()),
        Step::Expect(_)
        | Step::ReplyCoalescedWithNext
        | Step::Delay(_)
        | Step::CloseAt(_)
        | Step::Truncate(_)
        | Step::Corrupt { .. }
        | Step::WrongTransactionId
        | Step::LateReply(_)
        | Step::Respond(_)
        | Step::React(_) => None,
    }
}

/// Apply the armed modifiers to a reply and turn it into a write.
fn build_write(armed: &mut Armed, pieces: Vec<Vec<u8>>, merge: bool) -> Action {
    let lens: Vec<usize> = if merge {
        vec![pieces.iter().map(Vec::len).sum()]
    } else {
        pieces.iter().map(Vec::len).collect()
    };
    let mut bytes: Vec<u8> = pieces.into_iter().flatten().collect();
    for edit in armed.edits.drain(..) {
        match edit {
            Edit::Truncate(n) => bytes.truncate(n),
            Edit::Corrupt {
                offset,
                bytes: patch,
            } => {
                for (index, byte) in patch.into_iter().enumerate() {
                    if let Some(slot) = bytes.get_mut(offset.saturating_add(index)) {
                        *slot = byte;
                    }
                }
            }
            Edit::WrongTransactionId => {
                for slot in bytes.iter_mut().take(2) {
                    *slot = !*slot;
                }
            }
        }
    }
    let close_at = armed.close_at.take();
    if let Some(n) = close_at {
        bytes.truncate(n);
    }
    let mut chunks = Vec::new();
    let mut rest = bytes.as_slice();
    for len in lens {
        let (head, tail) = rest.split_at_checked(len).unwrap_or((rest, &[]));
        if !head.is_empty() {
            chunks.push(head.to_vec());
        }
        rest = tail;
    }
    Action::Write {
        chunks,
        late: mem::take(&mut armed.late),
        close: close_at.is_some(),
    }
}

impl Machine {
    pub(super) fn new(script: Script) -> Self {
        Self {
            steps: script.steps,
            strict: script.strict,
            pos: 0,
            inbox: Vec::new(),
            armed: Armed::default(),
            report: ScriptReport::default(),
            halted: false,
        }
    }

    /// The report so far.
    pub(super) const fn report(&self) -> &ScriptReport {
        &self.report
    }

    /// Consume the machine for its final report.
    pub(super) fn into_report(self) -> ScriptReport {
        self.report
    }

    /// Record bytes received from the client.
    pub(super) fn feed(&mut self, bytes: &[u8]) {
        self.report.received.extend_from_slice(bytes);
        if !self.halted {
            self.inbox.extend_from_slice(bytes);
        }
    }

    /// The next thing the peer does, or `None` when it needs input or has finished.
    pub(super) fn next(&mut self) -> Option<Action> {
        loop {
            if self.halted {
                return None;
            }
            let index = self.pos;
            let Some(step) = self.steps.get(index) else {
                if self.armed.close_at.take().is_some() {
                    self.halted = true;
                    self.report.closed = true;
                    return Some(Action::Close);
                }
                return None;
            };
            match step {
                Step::Expect(expected) => {
                    if self.inbox.len() < expected.len() {
                        return None;
                    }
                    let actual: Vec<u8> = self.inbox.drain(..expected.len()).collect();
                    let matched = actual == *expected;
                    if !matched {
                        self.report.mismatches.push(Mismatch {
                            step: index,
                            expected: expected.clone(),
                            actual,
                        });
                    }
                    self.report.ran.push(index);
                    self.pos = index.saturating_add(1);
                    if !matched && self.strict {
                        self.halted = true;
                        self.report.failed = true;
                        self.report.closed = true;
                        return Some(Action::Close);
                    }
                }
                Step::Reply(_) | Step::ReplyFragmented(_) => {
                    let mut pieces = reply_pieces(self.steps.get(index)).unwrap_or_default();
                    self.report.ran.push(index);
                    self.pos = index.saturating_add(1);
                    let merge = self.armed.coalesce > 0;
                    for _ in 0..mem::take(&mut self.armed.coalesce) {
                        let Some(more) = reply_pieces(self.steps.get(self.pos)) else {
                            break;
                        };
                        pieces.extend(more);
                        self.report.ran.push(self.pos);
                        self.pos = self.pos.saturating_add(1);
                    }
                    return Some(self.write(pieces, merge));
                }
                Step::ReplyCoalescedWithNext => {
                    self.armed.coalesce = self.armed.coalesce.saturating_add(1);
                    self.advance(index);
                }
                Step::Delay(by) => {
                    let by = *by;
                    self.advance(index);
                    return Some(Action::Delay(by));
                }
                Step::CloseAt(n) => {
                    self.armed.close_at = Some(*n);
                    self.advance(index);
                }
                Step::Truncate(n) => {
                    self.armed.edits.push(Edit::Truncate(*n));
                    self.advance(index);
                }
                Step::Corrupt { offset, bytes } => {
                    self.armed.edits.push(Edit::Corrupt {
                        offset: *offset,
                        bytes: bytes.clone(),
                    });
                    self.advance(index);
                }
                Step::WrongTransactionId => {
                    self.armed.edits.push(Edit::WrongTransactionId);
                    self.advance(index);
                }
                Step::LateReply(by) => {
                    self.armed.late = self.armed.late.saturating_add(*by);
                    self.advance(index);
                }
                Step::Respond(_) | Step::React(_) => {
                    let request = self.take_request(index)?;
                    let reaction = match self.steps.get_mut(index) {
                        Some(Step::Respond(responder)) => {
                            responder(&request).map_or(Reaction::Silence, Reaction::Reply)
                        }
                        Some(Step::React(reactor)) => reactor(&request),
                        _ => return None,
                    };
                    if let Some(action) = self.react(reaction) {
                        return Some(action);
                    }
                }
            }
        }
    }

    /// Take the pending request for the responding step at `index` and count the call;
    /// `None` while no request has arrived.
    fn take_request(&mut self, index: usize) -> Option<Vec<u8>> {
        if self.inbox.is_empty() {
            return None;
        }
        let request = mem::take(&mut self.inbox);
        if self.report.ran.last() != Some(&index) {
            self.report.ran.push(index);
        }
        self.report.responses = self.report.responses.saturating_add(1);
        Some(request)
    }

    /// Turn one reaction into the action it asks for; `None` for silence.
    fn react(&mut self, reaction: Reaction) -> Option<Action> {
        match reaction {
            Reaction::Silence => None,
            Reaction::Reply(answer) => {
                self.armed.coalesce = 0;
                Some(self.write(vec![answer], false))
            }
            Reaction::ReplyThenClose(answer) => {
                self.armed.coalesce = 0;
                self.armed.close_at = self.armed.close_at.or(Some(usize::MAX));
                Some(self.write(vec![answer], false))
            }
            Reaction::Close => {
                self.armed = Armed::default();
                self.halted = true;
                self.report.closed = true;
                Some(Action::Close)
            }
        }
    }

    fn advance(&mut self, index: usize) {
        self.report.ran.push(index);
        self.pos = index.saturating_add(1);
    }

    fn write(&mut self, pieces: Vec<Vec<u8>>, merge: bool) -> Action {
        let action = build_write(&mut self.armed, pieces, merge);
        if matches!(action, Action::Write { close: true, .. }) {
            self.halted = true;
            self.report.closed = true;
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(steps: Vec<Step>) -> Machine {
        Machine::new(Script::new(steps))
    }

    fn write(chunks: &[&[u8]]) -> Action {
        Action::Write {
            chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
            late: Duration::ZERO,
            close: false,
        }
    }

    #[test]
    fn debug_prints_every_step_and_hides_the_closure() {
        let steps = vec![
            Step::Expect(vec![1]),
            Step::Reply(vec![2]),
            Step::ReplyFragmented(vec![vec![3]]),
            Step::ReplyCoalescedWithNext,
            Step::Delay(Duration::from_secs(1)),
            Step::CloseAt(4),
            Step::Truncate(5),
            Step::Corrupt {
                offset: 6,
                bytes: vec![7],
            },
            Step::WrongTransactionId,
            Step::LateReply(Duration::from_secs(2)),
            Step::respond(|_| None),
            Step::react(|_| Reaction::Silence),
        ];
        let text = format!("{:?}", Script::strict(steps));
        for part in [
            "Expect([1])",
            "Reply([2])",
            "ReplyFragmented([[3]])",
            "ReplyCoalescedWithNext",
            "Delay(1s)",
            "CloseAt(4)",
            "Truncate(5)",
            "Corrupt { offset: 6, bytes: [7] }",
            "WrongTransactionId",
            "LateReply(2s)",
            "Respond(<responder>)",
            "React(<reactor>)",
            "strict: true",
        ] {
            assert!(text.contains(part), "debug output lacks {part}: {text}");
        }
    }

    #[test]
    fn constructors_set_strictness_and_default_is_an_empty_lenient_script() {
        assert!(!Script::new(Vec::new()).strict, "new is lenient");
        assert!(Script::strict(Vec::new()).strict, "strict is strict");
        let default = Script::default();
        assert!(default.steps.is_empty() && !default.strict, "default");
    }

    #[test]
    fn expect_waits_for_enough_bytes_and_keeps_the_surplus() {
        let mut machine = machine(vec![
            Step::Expect(b"abc".to_vec()),
            Step::Expect(b"de".to_vec()),
            Step::Reply(b"ok".to_vec()),
        ]);
        assert_eq!(machine.next(), None, "nothing received yet");
        machine.feed(b"ab");
        assert_eq!(machine.next(), None, "two of three bytes");
        machine.feed(b"cd");
        assert_eq!(
            machine.next(),
            None,
            "the surplus d is not yet enough for de"
        );
        machine.feed(b"e");
        assert_eq!(
            machine.next(),
            Some(write(&[b"ok"])),
            "both expectations met"
        );
        let report = machine.into_report();
        assert_eq!(report.received, b"abcde", "everything received is kept");
        assert_eq!(report.ran, vec![0, 1, 2], "all steps ran");
        assert!(report.mismatches.is_empty() && !report.failed, "clean run");
    }

    #[test]
    fn a_lenient_mismatch_is_recorded_and_the_script_carries_on() {
        let mut machine = machine(vec![Step::Expect(b"abc".to_vec()), Step::Reply(vec![1])]);
        machine.feed(b"abd");
        assert_eq!(
            machine.next(),
            Some(write(&[&[1]])),
            "the reply still goes out"
        );
        let report = machine.report();
        assert_eq!(
            report.mismatches,
            vec![Mismatch {
                step: 0,
                expected: b"abc".to_vec(),
                actual: b"abd".to_vec()
            }],
            "mismatch recorded"
        );
        assert!(!report.failed && !report.closed, "lenient does not fail");
    }

    #[test]
    fn a_strict_mismatch_fails_closes_and_ignores_later_input() {
        let mut machine = Machine::new(Script::strict(vec![
            Step::Expect(b"abc".to_vec()),
            Step::Reply(vec![1]),
        ]));
        machine.feed(b"xyz");
        assert_eq!(
            machine.next(),
            Some(Action::Close),
            "strict mismatch closes"
        );
        assert_eq!(machine.next(), None, "nothing runs after the failure");
        machine.feed(b"more");
        assert_eq!(machine.next(), None, "still halted");
        let report = machine.into_report();
        assert!(report.failed && report.closed, "failed and closed");
        assert_eq!(report.ran, vec![0], "only the failing step ran");
        assert_eq!(report.received, b"xyzmore", "late bytes are still recorded");
    }

    #[test]
    fn modifiers_edit_the_next_reply_in_script_order() {
        let mut machine = machine(vec![
            Step::WrongTransactionId,
            Step::Corrupt {
                offset: 3,
                bytes: vec![0xAA, 0xBB, 0xCC],
            },
            Step::Truncate(5),
            Step::LateReply(Duration::from_secs(2)),
            Step::Reply(vec![0, 1, 2, 3, 4, 5, 6]),
            Step::Reply(vec![9]),
        ]);
        assert_eq!(
            machine.next(),
            Some(Action::Write {
                chunks: vec![vec![0xFF, 0xFE, 2, 0xAA, 0xBB]],
                late: Duration::from_secs(2),
                close: false
            }),
            "flip, overwrite (clipped at the end), cut to five bytes"
        );
        assert_eq!(machine.next(), Some(write(&[&[9]])), "modifiers are spent");
    }

    #[test]
    fn close_at_cuts_the_reply_and_halts_the_script() {
        let mut machine = machine(vec![
            Step::CloseAt(3),
            Step::ReplyFragmented(vec![vec![1, 2], vec![3, 4], vec![5]]),
            Step::Reply(vec![9]),
        ]);
        assert_eq!(
            machine.next(),
            Some(Action::Write {
                chunks: vec![vec![1, 2], vec![3]],
                late: Duration::ZERO,
                close: true
            }),
            "fragments cut at three bytes"
        );
        assert_eq!(machine.next(), None, "nothing after the close");
        assert!(
            machine.report().closed && !machine.report().failed,
            "closed"
        );
    }

    #[test]
    fn a_trailing_close_at_closes_when_the_script_runs_out() {
        let mut machine = machine(vec![Step::CloseAt(0)]);
        assert_eq!(machine.next(), Some(Action::Close), "close");
        assert!(
            machine.report().closed && !machine.report().failed,
            "closed"
        );
        assert_eq!(machine.next(), None, "halted");
    }

    #[test]
    fn close_at_zero_sends_nothing_and_a_short_reply_is_sent_whole() {
        let mut machine = machine(vec![Step::CloseAt(0), Step::Reply(vec![1, 2])]);
        assert_eq!(
            machine.next(),
            Some(Action::Write {
                chunks: Vec::new(),
                late: Duration::ZERO,
                close: true
            }),
            "empty write then close"
        );
        let mut machine = self::machine(vec![Step::CloseAt(10), Step::Reply(vec![1, 2])]);
        assert_eq!(
            machine.next(),
            Some(Action::Write {
                chunks: vec![vec![1, 2]],
                late: Duration::ZERO,
                close: true
            }),
            "short reply whole, then close"
        );
    }

    #[test]
    fn coalescing_merges_the_next_replies_into_one_write() {
        let mut machine = machine(vec![
            Step::ReplyCoalescedWithNext,
            Step::Reply(vec![1]),
            Step::ReplyFragmented(vec![vec![2], vec![3]]),
            Step::Reply(vec![4]),
            Step::ReplyCoalescedWithNext,
            Step::ReplyCoalescedWithNext,
            Step::Reply(vec![5]),
            Step::Reply(vec![6]),
            Step::Expect(vec![0]),
            Step::ReplyCoalescedWithNext,
            Step::Reply(vec![7]),
            Step::Expect(vec![0]),
        ]);
        assert_eq!(
            machine.next(),
            Some(write(&[&[1, 2, 3]])),
            "two replies, one write"
        );
        assert_eq!(
            machine.next(),
            Some(write(&[&[4]])),
            "the next reply is alone"
        );
        assert_eq!(
            machine.next(),
            Some(write(&[&[5, 6]])),
            "two markers allow three replies; only two are available before the Expect"
        );
        machine.feed(&[0]);
        assert_eq!(
            machine.next(),
            Some(write(&[&[7]])),
            "nothing to merge with"
        );
        assert_eq!(machine.report().ran.len(), 11, "every step reached ran");
    }

    #[test]
    fn delay_is_reported_as_an_action() {
        let mut machine = machine(vec![Step::Delay(Duration::from_secs(4))]);
        assert_eq!(
            machine.next(),
            Some(Action::Delay(Duration::from_secs(4))),
            "delay"
        );
        assert_eq!(machine.next(), None, "then done");
    }

    #[test]
    fn respond_answers_every_request_and_may_stay_silent() {
        let mut calls = 0_u32;
        let mut machine = machine(vec![
            Step::WrongTransactionId,
            Step::respond(move |request| {
                calls += 1;
                (calls != 2).then(|| request.to_vec())
            }),
            Step::Reply(vec![0xEE]),
        ]);
        assert_eq!(machine.next(), None, "no request yet");
        machine.feed(&[1, 2]);
        assert_eq!(
            machine.next(),
            Some(write(&[&[0xFE, 0xFD]])),
            "first answer is edited"
        );
        machine.feed(&[3]);
        assert_eq!(machine.next(), None, "second request gets silence");
        machine.feed(&[4]);
        assert_eq!(
            machine.next(),
            Some(write(&[&[4]])),
            "third answer is plain"
        );
        let report = machine.into_report();
        assert_eq!(report.responses, 3, "three calls");
        assert_eq!(
            report.ran,
            vec![0, 1],
            "respond counted once; later steps never run"
        );
    }

    #[test]
    fn a_respond_answer_clears_a_pending_coalesce() {
        let mut machine = machine(vec![
            Step::ReplyCoalescedWithNext,
            Step::respond(|_| Some(vec![1])),
        ]);
        machine.feed(&[0]);
        assert_eq!(machine.next(), Some(write(&[&[1]])), "answer sent");
        assert_eq!(machine.armed.coalesce, 0, "the marker was spent");
    }

    /// A machine over one `React` step that plays `reactions` in order, one per request.
    fn reacting(reactions: Vec<Reaction>) -> Machine {
        let mut queue = reactions.into_iter();
        machine(vec![Step::react(move |_| {
            queue.next().unwrap_or(Reaction::Silence)
        })])
    }

    #[test]
    fn every_reaction_maps_to_its_action() {
        let mut machine = reacting(vec![
            Reaction::Reply(vec![1, 2]),
            Reaction::Silence,
            Reaction::ReplyThenClose(vec![3]),
        ]);
        machine.feed(&[0]);
        assert_eq!(machine.next(), Some(write(&[&[1, 2]])), "reply");
        machine.feed(&[0]);
        assert_eq!(machine.next(), None, "silence does nothing");
        assert!(!machine.report().closed, "silence keeps the link");
        machine.feed(&[0]);
        assert_eq!(
            machine.next(),
            Some(Action::Write {
                chunks: vec![vec![3]],
                late: Duration::ZERO,
                close: true
            }),
            "reply then close"
        );
        assert!(machine.report().closed, "the link is closed");
        machine.feed(&[0]);
        assert_eq!(machine.next(), None, "halted after the close");
        let report = machine.into_report();
        assert_eq!(
            report.responses, 3,
            "the fourth request was never reacted to"
        );
        assert_eq!(report.ran, vec![0], "counted once");
        assert!(!report.failed, "a reaction close is not a failure");
    }

    #[test]
    fn a_close_reaction_closes_without_a_reply_and_drops_armed_modifiers() {
        let mut machine = machine(vec![
            Step::LateReply(Duration::from_secs(3)),
            Step::Truncate(1),
            Step::react(|_| Reaction::Close),
            Step::Reply(vec![9]),
        ]);
        machine.feed(&[7]);
        assert_eq!(machine.next(), Some(Action::Close), "close");
        assert!(machine.report().closed, "closed");
        assert_eq!(machine.next(), None, "nothing after the close");
        machine.feed(&[8]);
        assert_eq!(machine.next(), None, "still halted");
        assert_eq!(
            machine.report().received,
            vec![7, 8],
            "bytes still recorded"
        );
        assert_eq!(machine.report().responses, 1, "one call");
    }

    #[test]
    fn a_react_step_stays_armed_and_the_closure_sees_each_request() {
        let mut machine = machine(vec![
            Step::react(|request| Reaction::Reply(request.iter().rev().copied().collect())),
            Step::Reply(vec![0xEE]),
        ]);
        assert_eq!(machine.next(), None, "no request yet");
        machine.feed(&[1, 2]);
        assert_eq!(machine.next(), Some(write(&[&[2, 1]])), "first");
        machine.feed(&[3]);
        assert_eq!(machine.next(), Some(write(&[&[3]])), "second");
        machine.feed(&[4, 5, 6]);
        assert_eq!(machine.next(), Some(write(&[&[6, 5, 4]])), "third");
        let report = machine.into_report();
        assert_eq!(report.responses, 3, "three calls");
        assert_eq!(report.ran, vec![0], "counted once; later steps never run");
    }

    #[test]
    fn reactions_that_reply_take_the_armed_modifiers_and_clear_a_pending_coalesce() {
        let mut machine = machine(vec![
            Step::ReplyCoalescedWithNext,
            Step::WrongTransactionId,
            Step::react(|_| Reaction::Reply(vec![1, 2, 3])),
        ]);
        machine.feed(&[0]);
        assert_eq!(machine.next(), Some(write(&[&[0xFE, 0xFD, 3]])), "edited");
        assert_eq!(machine.armed.coalesce, 0, "the marker was spent");

        let mut machine = self::machine(vec![
            Step::Truncate(2),
            Step::CloseAt(1),
            Step::react(|_| Reaction::ReplyThenClose(vec![5, 6, 7])),
        ]);
        machine.feed(&[0]);
        assert_eq!(
            machine.next(),
            Some(Action::Write {
                chunks: vec![vec![5]],
                late: Duration::ZERO,
                close: true
            }),
            "the earlier cut wins; the link closes"
        );
        let mut machine = self::machine(vec![
            Step::WrongTransactionId,
            Step::react(|_| Reaction::ReplyThenClose(vec![1, 2, 3])),
        ]);
        machine.feed(&[0]);
        assert_eq!(
            machine.next(),
            Some(Action::Write {
                chunks: vec![vec![0xFE, 0xFD, 3]],
                late: Duration::ZERO,
                close: true
            }),
            "edited, whole, then closed"
        );
    }

    #[test]
    fn a_silent_reaction_keeps_modifiers_armed_for_the_next_reply() {
        let mut turns = 0_u8;
        let mut machine = machine(vec![
            Step::WrongTransactionId,
            Step::react(move |_| {
                turns += 1;
                if turns == 1 {
                    Reaction::Silence
                } else {
                    Reaction::Reply(vec![1, 2])
                }
            }),
        ]);
        machine.feed(&[0]);
        assert_eq!(machine.next(), None, "silent");
        machine.feed(&[0]);
        assert_eq!(
            machine.next(),
            Some(write(&[&[0xFE, 0xFD]])),
            "edited later"
        );
    }

    #[test]
    fn respond_none_is_silence_and_an_empty_answer_is_an_empty_write() {
        let mut machine = machine(vec![Step::respond(|_| None)]);
        machine.feed(&[1]);
        assert_eq!(machine.next(), None, "respond stayed silent");
        let mut machine = self::machine(vec![Step::respond(|_| Some(vec![]))]);
        machine.feed(&[1]);
        assert_eq!(
            machine.next(),
            Some(write(&[])),
            "an empty answer is an empty write"
        );
        assert_eq!(machine.report().responses, 1, "counted");
    }
}
