# 0026: Board adapters
## Status
Accepted (2026-10-29), as slice S63 of docs/POST_ALPHA_PLAN.md (phase F, milestone M2S:
"MAX31855 over SPI: temperature, cold junction, open and short faults"). It builds on ADR-0018
(one crate, features and per-module tables), ADR-0016 and ADR-0023 (the measurement envelope,
the quality model and the stale tracker), ADR-0014 with its S58 addendum (the blocking-worker
precedent) and ADR-0005 (coverage tiers). It fills in the section of docs/ARCHITECTURE.md
that was drawn and deferred ("6.1 Direct bench adapters"), and says where the build departs
from the drawing. Every change is additive to the published `stonecrab` 0.2.0. No hardware
was operated: the decoding is written from the datasheet and tested against scripted buses,
and the bench record stays a runbook (docs/hardware/max31855-spi.md) that needs the
maintainer's separate approval.

## Context
SPEC.md ("Direct sensor extension") asks for a MAX31855 adapter over SPI: thermocouple
temperature, internal cold-junction temperature and the converter's open and short fault
indications, preserving negative readings and fault state. Board adapters "are extensions to
Stonecrab, not upstream PLC4X parity items. They share the measurement envelope and quality
model but retain device-specific configuration." The M2S exit is "bus mocks and recorded
bench tests pass; faults/loss preserved; no change to industrial parity status", and the
failure cases name "open/short thermocouple flags, I2C/SPI failures, unavailable device".

The MAX31855 has no registers and nothing to configure. Chip select low makes it shift out
one 32-bit frame, most significant bit first:

| Bits | Meaning |
| --- | --- |
| 31 to 18 | thermocouple temperature, 14 bits, two's complement, 0.25 C a count |
| 17 | reserved, reads 0 |
| 16 | fault: set when any of bits 2 to 0 is set |
| 15 to 4 | internal (cold-junction) temperature, 12 bits, two's complement, 0.0625 C a count |
| 3 | reserved, reads 0 |
| 2 | SCV, thermocouple shorted to Vcc |
| 1 | SCG, thermocouple shorted to ground |
| 0 | OC, thermocouple open |

The datasheet could not be retrieved while this slice was written (analog.com did not answer
from the build host, several attempts on 2026-10-29), so this table, the example temperatures
in the tests and the 5 MHz and mode 0 figures of the runbook are from the author's knowledge
of the datasheet, not from a pinned copy. The first
step of the runbook is to retrieve the datasheet, record its hash and check them; a
difference is a defect of this ADR, not of the hardware.

## Decision
1. **Module, features, placement.** `stonecrab::boards`, behind the new feature `boards`
   (`boards = ["sensors"]`: it needs the measurement envelope and nothing else, so no Tokio and
   no dependency). It is off by default: a board adapter is an opt-in extension and
   `cargo add stonecrab` stays what it was. The Linux backend is behind `adapters-linux`
   (`adapters-linux = ["boards", "transport"]`, the name CODING_STANDARDS 11.8 reserved),
   which adds the transport's Tokio for the worker's channel and timer and no crate. The
   testkit gains the SPI double only when `boards` is on. Board adapters depend on `sensors`,
   as the rules under the ARCHITECTURE module graph say.

   The module is one source file, `src/boards.rs`, and carries no module-level lint promotion.
   `scripts/coverage-gate.sh` and `scripts/lint-attrs.sh` hold per-module tables that this
   slice did not edit (another slice owns the coverage gate), and a directory under `src/`
   without a row fails the gate. A single file directly under `src/` is measured with the core
   at 100 percent, which is the tier this ADR wants for it anyway (decision 9). The code uses
   no indexing, slicing, `as` cast or unchecked arithmetic, as the promotions would require.
   When the second adapter (the ADXL345, slice S64) makes the module a directory, it gets a
   `boards` row of 100 in both tables, with `indexing_slicing` promoted on its declaration.

2. **`SpiBus` is a receive, not a transfer.** ARCHITECTURE 6.1 draws `transfer(tx, rx_len,
   deadline)`. The only adapter in this slice sends nothing, and a parameter with one caller
   that always passes an empty value is not earned, so the trait is `read(&mut self, len,
   deadline) -> OpFuture<Bytes>`: clock `len` bytes out of the device under one chip-select
   cycle, return exactly `len` bytes or an error. It is `Send` and object safe, takes the
   `Deadline` the way `Transport` does, and has no default deadline. A method that also
   transmits will be added, with a default that refuses (additive for implementors), together
   with the first device that needs it. The errors: `Timeout` at the deadline or before any
   transfer when it has passed, `Disconnected` when the bus or device cannot be used,
   `InvalidConfig` for a length of zero or one the backend cannot carry.

3. **The Linux backend is shipped, on plain `read(2)`.** `LinuxSpiDev` reads an
   `/dev/spidevB.C` node. The kernel's spidev driver turns a `read(2)` of N bytes into one
   half-duplex transfer with chip select held for its length; that is all a receive-only
   device needs, and it needs no `ioctl`. So the backend uses only `std::fs::File`: no
   dependency, no `unsafe` (the `spidev` crate would add `nix` and `libc` for `ioctl`s the
   adapter does not use, and CODING_STANDARDS 4.4 prefers `std` first), and it compiles on
   every platform, so the Linux CI cells and the macOS developer machines check the same
   code. What it cannot do is configure the bus: clock mode, speed and word size are whatever
   the device tree and the driver gave the node (mode 0 and at most 5 MHz for the MAX31855),
   and the runbook makes that a setup step. A thread owns the file and blocks in the read
   (CODING_STANDARDS 8.4: SPI runs on a dedicated OS thread); a bounded command channel of two
   (8.8) and a `oneshot` per call carry the work, and the async side races the answer against
   the deadline. A call that times out leaves its command to be skipped if the worker has not
   started it, or finished and dropped if it has, so a late frame is never handed to a later
   call. `LinuxSpiDev::from_device` takes any `Read`, as `SerialTransport::from_port` takes a
   port, which is how the tests run it without a device. The deadline is raced with a
   six-line `timeout_at` of its own, because `transport::deadline::within` is private to the
   transport module and this slice did not touch it; making `within` crate-visible and using
   it here is a small follow-up. Whether a real spidev node behaves as described is exactly
   what the bench must show (runbook, open items 3 and 4); until then the backend is
   `simulator-tested` like everything else here.

4. **A frame is decoded with exact integers, and a frame the chip could not send is
   refused.** `ThermocoupleReading::from_raw(raw, sampled_at, received_at)` splits the
   frame into big-endian halves and shifts each arithmetically, which sign-extends the 14-bit
   and 12-bit fields; the readings are kept as the integer counts (`thermocouple_quarters`,
   `cold_junction_sixteenths`) and converted to degrees by multiplying by 0.25 and 0.0625,
   which are exact in binary. Nothing is rounded, a negative temperature is a negative count,
   and the 32-bit frame is kept in `raw`. The three flags are a `ThermocoupleFault { open,
   short_to_ground, short_to_vcc }` rather than the drawn enum: the datasheet does not say the
   flags exclude one another, so all three are kept as reported; `fault` is `Some` exactly
   when the fault bit is set. A frame is refused, with no reading, when
   - it is all zeros or all ones (decision 5): `Disconnected`;
   - a reserved bit (17 or 3) is set: `MalformedData`;
   - the fault bit disagrees with the three flags (the datasheet defines it as their OR):
     `MalformedData`.
   A fault itself (open, short) is not an error: it is a reading.

5. **An unavailable device looks like all zeros or all ones.** Nothing on the bus
   acknowledges a transfer, so absence can only be inferred from the pattern. A data line
   that floats or is pulled up reads as all ones; one that is held low (an unpowered chip, a
   grounded line, an unconnected controller) reads as all zeros. The converter can send
   neither all ones (both reserved bits would be set, and so would the fault bit with all
   three flags) nor, in practice, all zeros (the thermocouple and the die both at exactly 0 C:
   a count of zero in each field and no fault). The first is therefore certain evidence of no
   device; the second is a deliberate false alarm in the one case in which a real chip reads
   exactly 0 C at both junctions, and the price of not reporting "0 C, no fault" from a dead
   line, which is the failure that does damage (a furnace controller reading 0 C). Both
   are `ErrorKind::Disconnected` with the category `Transport`, as a Modbus peer that closed
   is, and the message names the frame in hexadecimal. Other floating patterns (a stuck bit
   among zeros) are caught by the reserved-bit and fault-consistency rules when they touch
   those bits, and are not otherwise distinguishable from a reading; the runbook records what
   the host's line does with nothing attached.

6. **A reading becomes three measurements.** `ThermocoupleReading::measurements()` returns
   `thermocouple_temperature` (`Cel`, `F64`), `cold_junction_temperature` (`Cel`, `F64`) and
   `thermocouple_fault` (`1`, `U8`), in that order, as `stonecrab::sensors::Measurement` with
   the envelope of ADR-0016.
   - A fault makes the thermocouple temperature `Bad` **with no value**: the datasheet gives no
     temperature meaning to the bits under a fault, and a consumer that ignores quality must
     not find a number there. The envelope allows a `Bad` measurement with a value (a status
     field keeps the scaled value); this adapter chooses the stricter form on purpose. The
     counts stay in `raw` (`I16`), so nothing is lost for diagnosis.
   - The cold-junction temperature stays `Good` under a thermocouple fault: the internal sensor
     is a separate part of the chip, and the frame was accepted.
   - `thermocouple_fault` is the three flags as a `U8` (bit 0 open, bit 1 short to ground, bit
     2 short to Vcc), 0 for none, always present, so "no fault" is stated and not inferred
     from absence. Its quality is `Good`: it reports the state faithfully, and a non-zero
     value beside a `Bad` temperature is how the fault stays visible.
   - `origin` is `Documented` (written from the datasheet, no retained fixture from a device),
     `received_mono` and `received_at` are the reading's stamps from the adapter's `Clock`,
     taken after the transfer completes, `source_time` is `None` (the chip stamps nothing) and
     `protocol_status` is `None`. `raw` holds the integer count.
   - A read that failed gives no reading and so no measurements. That is what the stale
     tracker of ADR-0023 expects: a value that is not replaced ages and goes `Stale`, and a
     `Bad` measurement without a value is not retained, so a faulted thermocouple lets the
     last good temperature age out instead of replacing it.
   `sensors` gains nothing: the adapter builds `Measurement` with its struct literal from
   inside the crate. `measurements()` returns `Result` only because `TagId::new` does; the
   names are constants and it does not fail.

7. **`Max31855` is the adapter.** `Max31855::new(Box<dyn SpiBus>, Arc<dyn Clock>)` and
   `read(deadline) -> OpFuture<ThermocoupleReading>`: one four-byte read, stamped from the
   clock, decoded. The drawn `as_measurements` is a method of the reading instead, since it
   needs no bus. A bus that returns other than four bytes is `MalformedData`; the bus's own
   errors pass through unchanged. There is no retry, no reconnect and no background task: a
   caller schedules reads (the converter finishes a conversion about every 100 ms, so a faster
   rate repeats a value) and decides what an error means.

8. **The test double lives in the testkit.** `FakeSpiBus` plays a script of `SpiStep`s
   (`Reply(bytes)`, `Fail(kind)`, `Silence`) on a `FakeClock`; a spent script behaves as
   `Silence`, which sleeps on the fake clock until the deadline and times out. Clones share
   the script and the log of requested lengths, so a test keeps a handle after boxing the bus
   (CODING_STANDARDS 13.5).

9. **Tier, tests and records.** The module is measured at 100 percent of lines with the core
   (decision 1), the testkit double with the testkit at 90. The tests are: the datasheet's
   Tables 4 and 5 (positive, negative, both fields) and hand-computed extremes of each field,
   each fault flag and combination, an open-circuit frame as the datasheet describes it,
   reserved bits, an inconsistent fault bit, all zeros and all ones, a bus error, a
   timeout at the deadline and after an elapsed deadline, a short and a long transfer, the
   measurements produced, the Linux backend over blocking readers and files, and property
   tests and exhaustive sweeps of the decode (every low half of the frame against
   representative high halves and the reverse, and generated valid frames) against a reference
   decoder that uses different arithmetic. Board adapters are not upstream items, so no
   `# Compatibility` section and no parity row is added; the support level is
   `docs/COMPATIBILITY.md` (`simulator-tested`, which is the matrix's word for a scripted
   peer). SPEC.md's rule stands: hardware verification of the device needs a record in the
   format of `docs/hardware/TEMPLATE.md`.

## Alternatives considered
- **The `spidev` crate (or `linux-embedded-hal`).** Gives mode and speed configuration and
  full-duplex transfers. Rejected for this slice: it adds `nix` and `libc` and their
  `unsafe` to the audited surface for features a receive-only adapter does not use, and the
  MAX31855 works with the node's defaults. The decision is revisited by the first device that
  needs a configured bus or a transmit, with a DEPENDENCY_AUDIT entry then.
- **`embedded-hal`'s `SpiDevice`.** Rejected: it is blocking and deadline-free, and the
  library's rule is that every I/O call has a deadline.
- **An existing MAX31855 crate.** Rejected: the decode is a few lines, and the point of this
  adapter is that it keeps the fault flags, the negative counts and the refused frames, which
  those crates reduce to a float or `NaN`.
- **Treat all zeros as a reading.** Rejected (decision 5).
- **Give a faulted thermocouple a value with quality `Bad`.** Rejected (decision 6); the
  counts remain in `raw`.
- **`transfer(tx, rx_len, deadline)` as drawn.** Rejected for now (decision 2).
- **A directory module with its own table rows.** Deferred only for the reason in decision 1.

## Consequences
- `stonecrab` gains the features `boards` and `adapters-linux`, one module, one test double and
  no dependency. `Cargo.lock` is unchanged. The default feature set and the semver surface
  of 0.2.0 are unchanged: every item is new and no existing enum gained a variant.
- A host with a `/dev/spidev` node can read a MAX31855 with no root-only configuration beyond
  the device tree and permission on the node, and with no `unsafe` in the dependency graph
  for it. A host whose node is configured for another mode or speed reads garbage that the
  reserved-bit rule will often, not always, refuse; the runbook is the guard.
- The Linux backend ends its thread only when the `LinuxSpiDev` is dropped and the transfer it
  is in returns; a node that never returns would hold the thread (the kernel bounds a
  transfer, so this is a defect of the device, not of the design).
- Every temperature is in degrees Celsius as `F64`. The type of thermocouple (K, J, N, T,
  E, S, R) is a property of the chip variant, which does its linearization; the adapter
  neither knows nor needs it, and the runbook records the variant.
- The module is exempt from the module-level lint promotions and from a coverage row until it
  becomes a directory (decision 1); the gate measures it as core. The shortcoming is
  recorded here and in the notes of slice S63, not hidden.

## Compliance
- `Cargo.toml` declares `boards` and `adapters-linux` and the test stanzas of
  `boards_max31855` (`boards`, `testkit`), `boards_properties` (`boards`) and `boards_spidev`
  (`adapters-linux`), matching each file's `#![cfg]`.
- `cargo tree -p stonecrab -e normal` with every feature shows no crate that `transport` and
  `sensors` did not already bring in; `mise run dependencies` is unchanged.
- `src/boards.rs` and `src/testkit/fake_spi.rs` and `docs/components/boards.md` are in
  `packaging/stonecrab.txt`.
- The names of the three measurements, their units and qualities are pinned by
  `a_good_reading_becomes_three_good_measurements_with_units_and_counts` and
  `a_fault_makes_the_thermocouple_bad_and_keeps_the_fault_and_the_cold_junction`.
- `tests/boards_properties.rs` decodes every value of each half of the frame against twelve
  representative values of the other half, and random and generated frames, and checks each
  outcome against a reference decoder that uses different arithmetic. The 2^32 frames are
  not enumerated.

## Addendum (2026-10-29): the datasheet was checked
The datasheet could be retrieved after this ADR was written (the Adafruit mirror of the
Maxim document 19-5793, Rev 2, 2/12; sha256
`45904b81e79c46af49fd5c880df7ec04678c8599be9df73cea8422d241ceb7a2`; vendor documentation,
pinned by hash and not copied). The statement under "Context" that the table is from the
author's knowledge is superseded: every claim of the table agrees with it. Checked: the bit
positions and widths of both temperature fields (D[31:18] at 0.25 C, D[15:4] at 0.0625 C,
two's complement), D17 and D3 reserved and always 0, D16 set when any of D2 (SCV), D1 (SCG)
or D0 (OC) is set, fSCL at most 5 MHz, tCONV 70 to 100 ms. The datasheet describes the data
shifted out on the falling edge of SCK with CS low; it does not name an SPI mode, and mode 0
is the reading of that description that the runbook keeps. The example tests now use the rows
of the datasheet's Tables 4 and 5, and the counts that are not rows are named as hand-computed.

One fact decision 6 did not record: with T+ and T- unconnected the datasheet says D31 is 0
and D[30:18] are all 1, so an open-circuit frame carries the thermocouple field 0x1FFF
(8191 quarters, +2047.75 C as a count) with the fault bit and OC set. The decode keeps those
bits in `raw` and offers no temperature, as decision 6 says; the datasheet gives no
temperature meaning to them under any other fault. A test pins the frame `0x7FFD_1901`.
The runbook's step B0 still compares the revision of the part actually bought.

## Addendum (2026-10-29): the module is a directory (S64)
The second adapter arrived (the ADXL345, slice S64, ADR-0029), and decision 1's deferred part
is done. `stonecrab::boards` is `src/boards.rs` (the `SpiBus` and `I2cBus` traits and two
helpers both adapters use: the failure constructor and the builder of a `Measurement` of
origin `Documented`) with `src/boards/max31855.rs` and `src/boards/linux.rs` (this ADR's code,
moved without a change of behavior or of public path) and `src/boards/adxl345.rs`. The module
has its own rows: `boards|100` in `scripts/coverage-gate.sh`, so it is no longer measured with
the core, and a `boards` row in `scripts/lint-attrs.sh`. Its declaration promotes
`clippy::indexing_slicing`, `clippy::arithmetic_side_effects` and `clippy::as_conversions`,
not only the first as decision 1 said: the code already met all three, as decision 1 noted.
Decisions 2 to 9 are unchanged.
