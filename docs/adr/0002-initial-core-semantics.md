# 0002: Explicit and bounded core primitives
## Status
Accepted for the initial unreleased foundation.
## Decisions
- Tag identifiers preserve UTF-8 and are 1–256 bytes without control characters or
  edge whitespace. Validation precedes allocation and never echoes rejected text.
  This is a caller identifier policy, not upstream protocol address syntax.
- Deadline construction takes the monotonic reading explicitly. Equality means
  expired; overflow is an invalid configuration error. No clock trait or sleeping
  implementation is needed for these pure operations.
- Integer accessors widen only without loss. Float accessors accept only float
  variants. Raw non-finite floats are retained for diagnosis; future sensor scaling
  will reject non-finite results with explicit quality.
- Calendar, bytes/raw and recursive collection values remain deferred until their
  representation and resource budgets are designed. They are not converted to text.
- Diagnostic fields stay absent when unknown. Caller-supplied error messages and
  source chains must be redacted by the caller; arbitrary errors cannot be sanitized
  automatically.
- Resource budgets are immutable after validation. Every budget is positive and
  a receive buffer must hold a full frame. Builders allocate no device resources.
  Default budgets are development policy, not throughput or hardware guarantees.
- Native and polling subscription capabilities are distinct; unsupported is default.
  Connection lifecycle operations are mandatory, not evidence of reachability.
## Consequences
No protocol compatibility claims are inferred from these contracts. Protocol-specific
parsers, cancellation behavior and lifecycle enforcement require a later testable
connection implementation.
