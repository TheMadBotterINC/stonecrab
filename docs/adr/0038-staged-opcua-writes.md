# 0038: Staged OPC UA writes

## Status

Accepted and implemented S100 (2026-11-03), under ADR-0034 and ADR-0010.
[S100 evidence](../evidence/opcua-s100.md) records the completed checks, archive and
software-peer validation. No new public value/operation type, feature,
dependency or version. Normal signed-off commits and merge preserve review history.

## Design

Validate access and request boundaries, resolve missing variable metadata on the existing
lane, then encode eligible values and send one WriteRequest for the batch. Keep local
item rejections and device statuses correlated; replace Unsupported write with explicit
confirmed/rejected/unknown outcomes, and never apply read retries to a write.

- Reuse ConnectionConfig.access. Add the existing access=read-only/read-write convention
  to OPC UA options/configuration, default read-only, including programmatically built
  configurations. Access denial precedes every other check and all metadata I/O.
- Keep state, item limits, address downcast/count, deadline and cancellation validation
  at the connection boundary. Writes target Value only; other attributes are typed
  per-item rejections. IndexRange requires a List even for one selected element.
- Refactor read::nodes into a reusable per-tag node constructor without changing Read
  semantics. Reuse read status classification/protocol status helpers for writes, with
  BadNotWritable explicitly AccessDenied. Unknown numeric statuses retain their code.
- Enable WriteValue (670), WriteRequest (673), WriteResponse (676) in the existing
  generator options; generated changes are a separate gen commit. Extend independent
  service vectors, fuzz seeds and pinned upstream-write vectors' exact expectations.
- Private opcua/attributes.rs owns the variable metadata shape and bounded per-session
  resolution/cache on Session. DataType, ValueRank, ArrayDimensions and AccessLevel are
  read using existing ReadValueId/ReadRequest codecs. Cache keys are exact NodeIds,
  independent of tag suffix/range. Capacity is max_items_per_request, FIFO replacement;
  keys/attributes are owned by the session and disappear on close/reconnect. No lock is
  held across I/O. Concurrent misses may read twice; insertion still respects the bound.
  Metadata requests are chunked by the same item limit and share the write's absolute
  deadline/cancel token. Only successful metadata is cached. Missing/unsupported
  attributes permit explicit inference, while bad node/access/device errors remain
  errors. Custom datatype resolution stays S119/S120, rather than guessing a wire type.
- A suffix fixes the wire type and avoids metadata lookup. Otherwise use declared
  built-in type, then explicit PlcValue inference when no concrete type is available.
  Abstract UInteger selects the smallest unsigned built-in that holds every element;
  Integer/Number remain constrained to their numeric families. Preserve published
  suffix vocabulary (WORD/DWORD/LWORD/WSTRING are not accepted suffixes upstream or
  here); UInt16/32/64 and String cover their corresponding wire representations.
- Private opcua/write.rs assembles/results; opcua/write/value.rs handles pure bounded
  type selection, rectangular flattening and encoding. Reuse existing Scalar/Array/
  Variant/DataValue and temporal accessors. Integer conversions are checked; floating
  narrowing must be exact, with no silent truncation. TIME is exact Int32 milliseconds;
  fractions and overflow reject. DateTime uses checked 1601 ticks with exact 100 ns
  alignment, so representable instants round-trip, including dates before 1970.
  Nested Lists flatten row-major with dimensions for matrices; ragged/mixed/deep or
  oversized trees are rejected under the existing value-node/depth and byte bounds.
  Empty arrays need an explicit/server element type. Server rank/dimensions constrain
  whole values, and selected ranges constrain shape/cardinality independently. Null is
  explicit; custom Struct writes stay S120 and produce a per-item Unsupported error.
  Only value is set in DataValue; no status/timestamps are invented.
- Lane keeps its existing exchange callers. Add a write entry point using the same
  private exchange implementation and a per-request AtomicBool borrowed only during
  that exchange. Mark possible transmission when send I/O is first polled, after queue,
  size, cancellation and deadline checks; other concurrent requests cannot change it.
  A partial/failed send remains channel-fatal as today, with conservative Unknown for
  that write. Before-send failures remain errors. Once possibly transmitted, loss,
  cancellation, timeout or an unusable exchange yields Unknown without replay. An
  exact WriteResponse supplies per-item statuses; a wrong result count is the roadmap's
  explicit MalformedData error and must never imply safe replay. A ServiceFault cannot
  prove individual item outcomes, so is conservatively unknown after transmission.
- Share connection error stamping and the existing close-stop token between Read/Write;
  preserve read retry behavior. Close cancels current operations; no new spawned task.
- Tests cover scalar wire types/conversions, time/date boundaries, shape/ranges, suffix
  precedence, declared/inferred/abstract types, per-session cache reuse/eviction and
  failure behavior, exact status/count mapping, read-only/no-I/O, cancellation/deadline,
  concurrent exchanges and pre-/post-send errors. Extend the independent byte responder
  and ContractSuite with write and unknown-after-transmit injection. Native open62541
  writes/readbacks cover every supported writable type and arrays/matrices; no hardware
  or secure-channel claim. Keep the peer isolated on its existing disposable namespace.
- Coverage floors stay unchanged (handwritten OPC UA/core 100%). Run full check, isolated
  feature matrix, fuzz smoke, configured focused mutation campaign, regeneration and
  self-tests, semver/default/all, dependencies, parity/attributes/DCO and fresh package/
  unpacked builds before merge. Update parity's ten S100 rows, architecture/support
  docs, plan, status and evidence; keep custom structures/native subscriptions deferred.

## Sources and boundaries

The pinned mspec defines the generated service fields. The official
[Write service](https://reference.opcfoundation.org/specs/OPC-10000-4/5.11.4)
requires array-valued range writes and correlates results in request order.
[ValueRank/ArrayDimensions](https://reference.opcfoundation.org/specs/OPC-10000-3/8.6)
provide the shape constraints. Existing codec bounds and immutable value APIs are reused;
metadata failure never authorizes an anonymous retry, write retry, or schema guess.

Native range validation exposed the old scalar hint on ranged OPC UA addresses. A tag
with any IndexRange now hints List, including a single index; the suffix still describes
its element type. Whole-array writes without a range use server metadata/inference. The
core builder's exact hint contract stays unchanged. Writes race the existing connection
stop token inside the lane's tracked entry point, preserving per-request delivery state
when close interrupts them. The redundant second connection-state check is removed.

Mutation review moves node accounting to enqueue: reserve the root and each list's children
exactly once with checked subtraction, before growing the traversal stack. This enforces
both traversal work and pending-allocation limits without a redundant prospective check.
The owned WriteRequest also needs iterative disposal when a caller supplies a deeply
nested raw List and encoding rejects it, or drops the operation before polling. Reuse
S98's discard_tree in a private WriteRequest destructor; preserve its public fields,
builder, clone and accessors. Tests cover 100000 levels on a 128 KiB thread, both rejected
encoding and abandoned futures. Builder/explicit Clone behavior is unchanged.

Final disposition review makes an unexpected correlated service type and an Uncertain
Write status Unknown, rather than a rejection implying a definite refusal. The latter
keeps its exact device status; core's existing fallback reason is TimeoutAfterTransmit
because no unusable-reply reason exists. No new public variant is introduced. A verified
WriteResponse with a wrong result count remains the explicit roadmap MalformedData error.
