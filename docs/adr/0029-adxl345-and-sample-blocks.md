# 0029: ADXL345 and sample blocks
## Status
Accepted (2026-10-29), as slice S64 of docs/POST_ALPHA_PLAN.md (phase F, milestone M2S:
"ADXL345 over I2C with a sample-block interface carrying interval, axes, sequence and loss";
exit "FIFO overflow case; bench record"). It builds on ADR-0026 (board adapters: the module,
the `SpiBus` precedent, the measurement mapping and the test double), ADR-0016 and ADR-0023
(the measurement envelope and the quality model) and ADR-0005 (coverage tiers). It fills in
the ADXL345 half of docs/ARCHITECTURE.md section 6.1 and says where the build departs from
the drawing. Every change is additive to the published `stonecrab` 0.2.0. No hardware was
operated: the adapter is written from the datasheet and tested against a scripted bus, and
the bench record stays a runbook (`docs/hardware/adxl345-i2c.md`) that needs the
maintainer's separate approval.

## Context
SPEC.md ("Direct sensor extension") asks for the "ADXL345 over I2C, with SPI as a follow-on:
three-axis acceleration, selected range, output rate and FIFO behavior. Report missing
samples and overflow", and says that "ADXL345 initialization includes explicit configuration
writes even in an acquisition-only application; distinguish these from a public
actuator-write API". The direct-sensor failure cases are "I2C/SPI failures, unavailable
device, invalid identity and FIFO overflow where applicable". ARCHITECTURE 6.1 draws an
`I2cBus` (`write_read`, `write`), a `LinuxI2cDev`, an `Adxl345` (`init`, `verify_identity`,
`read_sample`, `read_fifo`), an `Adxl345Config` (range, rate, FIFO, full resolution), an
`AccelSample` and, in section 6, a `SampleBlock`: "Raw waveform data is a block with
interval, layout and loss accounting, never a stream of unrelated scalars."

The datasheet used is Analog Devices, "ADXL345: 3-Axis, +/-2 g/+/-4 g/+/-8 g/+/-16 g Digital
Accelerometer", **Rev. 0** (24 pages, PDF dated 2009-05-29), retrieved on 2026-10-29 from the
SparkFun mirror (<https://www.sparkfun.com/datasheets/Sensors/Accelerometer/ADXL345.pdf>,
the same file at <https://cdn.sparkfun.com/assets/9/1/8/9/9/ADXL345.pdf>), sha256
`2c0d2a0e00ec8cdfbab5fa801859af1f5c4cf8b30c3738e262cf9a0a3bcdec74`. It is vendor
documentation, pinned by hash and not copied. analog.com did not answer from the build host
(an HTTP/2 stream error, then a 90 s timeout). Later revisions exist; the facts below are
the ones of Rev. 0, and the runbook's step A0 compares the revision of the part actually
used. The facts this ADR relies on:

| Register | Address | Use here |
| --- | --- | --- |
| DEVID | 0x00 | read only, always 0xE5 |
| OFSX, OFSY, OFSZ | 0x1E to 0x20 | offsets the device adds to every sample, reset 0 |
| BW_RATE | 0x2C | bit 4 low power, bits 3 to 0 the rate code (Table 6: 1111 3200 Hz halving to 0110 6.25 Hz; reset 1010, 100 Hz) |
| POWER_CTL | 0x2D | bit 5 link, 4 auto-sleep, 3 measure, 2 sleep, 1 and 0 wakeup; powers up in standby; "configure the device in standby mode and then ... enable measurement mode" |
| INT_SOURCE | 0x30 | bit 7 DATA_READY ("set when new data is available"), bit 0 overrun (in bypass mode, new data replaced unread data; in the FIFO modes, "set when FIFO is filled"); both "set ... regardless of the INT_ENABLE register settings" and cleared by reading the data registers |
| DATA_FORMAT | 0x31 | bit 7 self-test, 6 SPI, 5 interrupt polarity, 3 FULL_RES, 2 justify (1 left, 0 right "with sign extension"), 1 and 0 range (00 2 g to 11 16 g); "All data, except that for the +/-16 g range, must be clipped to avoid rollover" |
| DATAX0 to DATAZ1 | 0x32 to 0x37 | two's complement, low byte first; "a multiple-byte read of all registers" is recommended |
| FIFO_CTL | 0x38 | bits 7 and 6 mode (00 bypass, 01 FIFO, 10 stream, 11 trigger), 5 trigger pin, 4 to 0 samples (the watermark) |
| FIFO_STATUS | 0x39 | bit 7 trigger event, bit 6 reads 0, bits 5 to 0 the entries; at most 32 in the FIFO, "a maximum of 33 entries available at any given time" with the one at the output |

Table 1 gives the resolutions (10 bits in 10-bit mode; 10, 11, 12 and 13 bits at full
resolution for plus or minus 2, 4, 8 and 16 g) and the 10-bit sensitivities: 256, 128, 64 and
32 LSB/g (typical; 232 to 286 at 2 g, roughly -9 to +12 percent). The full-resolution
sensitivity, 256 LSB/g at every range, is the DATA_FORMAT FULL_RES description ("maintain a 4
mg/LSB scale factor"); Table 1 states it for the 2 g row only. The I2C address is 0x1D with
SDO/ALT ADDRESS high and 0x53 with it low; the bus runs at 100 or 400 kHz. A FIFO entry is
popped by any read of the data registers, so "all axes ... should be read in a burst"; the 5
us pause the FIFO needs between two pops is "not a concern when using I2C mode". In stream
mode a full FIFO overwrites its oldest entry; in FIFO mode it stops collecting; the overrun
bit is set "when FIFO is filled" in both, and "automatically cleared when the contents of FIFO
are read". The device counts nothing it drops. The turn-on time (footnote to the
specifications table) is about tau + 1.1 ms, tau being one output period (11.1 ms at 100 Hz);
before the first sample the data registers hold their reset value, 0.

## Decision
1. **Module, features, placement.** The adapter joins `stonecrab::boards` behind the existing
   `boards` feature: no new feature and no dependency. The module becomes a directory, as
   ADR-0026 decision 1 planned: `src/boards.rs` keeps the bus traits and two helpers shared by
   both adapters (the failure constructor and the measurement builder), `src/boards/max31855.rs`
   and `src/boards/linux.rs` hold the MAX31855 code moved unchanged, and
   `src/boards/adxl345.rs` holds this slice. Every public path is unchanged
   (`stonecrab::boards::Max31855` and so on, re-exported from the root of the module). The
   module gets its own row in `scripts/coverage-gate.sh` at 100 percent and in
   `scripts/lint-attrs.sh`, and its declaration in `lib.rs` promotes
   `clippy::indexing_slicing`, `clippy::arithmetic_side_effects` and `clippy::as_conversions`
   to deny, as codec, modbus and sim do. ADR-0026 named only the first; the code already met all
   three (the workspace warns on them and the gates deny warnings), so promoting all three
   costs nothing and keeps the module at the strictest tier.

2. **`I2cBus` has the two transactions a register device needs.** `write(address, bytes,
   deadline) -> OpFuture<()>` (start, address and write bit, the bytes, stop) and
   `write_read(address, bytes, len, deadline) -> OpFuture<Bytes>` (the bytes, a repeated
   start, `len` bytes read, stop). The address is a parameter of each call, as drawn, because
   an I2C bus carries several devices; the adapter keeps its own. The bytes are `Bytes`, as
   `Transport::send` takes them. There is no read without a write: the ADXL345 never needs one
   (every read names its register first). The trait is `Send` and object safe, takes the
   `Deadline` the way `SpiBus` does and has no default deadline. Its errors: `Timeout` at the
   deadline or before any transaction when it has passed; `Disconnected` when the device does
   not acknowledge its address or a byte (absent, unpowered, at another address: the NACK
   case) or the bus cannot be used; `InvalidConfig` for an address above 0x7F, an empty write
   or a length the backend cannot carry. Unlike `SpiBus::read`, neither method is cancel-safe:
   a write may have landed, and reading the data registers pops the FIFO.

3. **No Linux I2C backend in this slice.** `LinuxI2cDev` is deferred, with this reason: the
   Linux `i2c-dev` interface needs an `ioctl` to address a device (`I2C_SLAVE` for plain reads
   and writes, `I2C_RDWR` for the repeated start of a register read), the standard library has
   no `ioctl`, first-party `unsafe` is forbidden (`#![forbid(unsafe_code)]`, CODING_STANDARDS),
   and no dependency of the crate offers a safe one: `libc` is in the graph only through Tokio
   and `serial2`, calling its `ioctl` would be first-party `unsafe`, and `serial2` exposes no
   generic `ioctl`. A crate that wraps it (`i2cdev`, `linux-embedded-hal`, `nix`, `rustix`) is a
   new dependency with `unsafe` inside, which needs a DEPENDENCY_AUDIT entry and the
   maintainer's decision, so it is a later slice. Until then the bench runbook has the scratch
   program, outside the repository, implement `I2cBus` over a crate of the maintainer's
   choosing. The kernel's own ADXL345 driver (IIO, `/sys/bus/iio`) is a different design (the
   kernel owns the configuration and the FIFO) and is not this adapter's backend.

4. **The identity is checked before anything is written.** `verify_identity(deadline)` reads
   DEVID and fails unless it is 0xE5. A wrong value is `ErrorKind::Trust` (category
   `Protocol`), the kind the crate defines for "peer identity ... validation failed", with the
   byte and the address in the message; it is not `MalformedData`, because the byte was read
   correctly and names another device. `init` calls it first, so a device that is not an
   ADXL345 receives no write. `verify_identity` alone changes nothing on the device and does
   not start a stream.

5. **The configuration is device configuration, written by `init` alone.** `Adxl345Config`
   has `address` (default 0x53), `range` (`AccelRange::{G2, G4, G8, G16}`, default `G2`),
   `rate` (`OutputRate::{Hz3200 ... Hz6_25}`, default `Hz100`), `fifo`
   (`FifoMode::{Bypass, Fifo, Stream}`, default `Stream`) and `full_resolution` (default
   `true`); it is `Copy`, `#[non_exhaustive]` and built with `Default` and field assignment.
   The adapter takes it at `new` and offers no setter and no register write: `init` is the
   only code that writes, and it writes, in this order, after the identity check:
   1. POWER_CTL 0x00 (standby while configuring, as the datasheet asks, and sleep, link and
      auto-sleep off);
   2. FIFO_CTL 0x00 (bypass, which empties a FIFO left by an earlier program);
   3. no write, but one six-byte read of DATAX0 to DATAZ1, discarded and not decoded (it may
      be in another program's format): it clears DATA_READY and an overrun flag an earlier
      stream left on a device that kept power, so the new stream reports neither; in standby
      no sample replaces it;
   4. OFSX, OFSY and OFSZ 0 in one write (an offset left by another program would shift every
      sample silently);
   5. DATA_FORMAT: FULL_RES and the range bits; right-justified, no self-test, interrupts
      active high;
   6. BW_RATE: the rate code, normal power;
   7. FIFO_CTL: the mode bits, the trigger bit 0 and the samples (watermark) bits 0;
   8. POWER_CTL 0x08 (measure).
   The watermark and the interrupt registers are not configured and not options: the adapter
   polls, the overrun flag it needs is set whatever INT_ENABLE says, and an option nothing
   reads is not earned. An interrupt-driven reader would add both. INT_ENABLE and INT_MAP are
   left as found, not cleared: a samples value of 0 "immediately sets the watermark status
   bit", so on a device that kept power with the watermark interrupt enabled by another
   program, that INT pin stays asserted. The adapter reads only bits 7 and 0 of INT_SOURCE
   and is not affected. The rates are the codes of
   the pinned revision (0110 to 1111); the lower rates of later revisions are not offered.
   The low-power bit and the trigger mode (which needs an interrupt line) are not offered.

6. **A sample is decoded with exact integers.** `AccelSample { x, y, z }` holds the three
   counts as `i16`. `AccelSample::from_registers(data, config)` reads each little-endian pair
   as a two's complement `i16`, which is the right-justified, sign-extended layout `init`
   configures; the left-justified layout is never configured, so it is never decoded (and the
   pinned revision does not give its bit positions per resolution). A count outside the
   configured resolution (-512 to 511 in 10-bit mode; up to -4096 to 4095 at full resolution
   and 16 g) is refused as `MalformedData` naming the axis: at 2, 4 and 8 g the device clips
   to its range, so such a count is a corrupt transfer or another configuration. At 16 g the
   datasheet does not promise clipping: an acceleration beyond the range can roll over inside
   the 10 or 13 bit field to a count that fits, which no check can tell from a true one, so
   quality `Good` (decision 9) does not rule it out there. The scale is kept as
   `counts_per_g` (256 at full resolution; 256, 128, 64 or 32 in 10-bit mode), the datasheet's
   typical sensitivity; a power of two, so a count divided by it is exact in `f64`. Nothing
   claims calibration.

7. **A failed or dropped read ends the stream.** The adapter keeps a `ready` flag: `init`
   clears it first and sets it last; `read_sample` and `read_fifo` clear it before their first
   transaction and set it again only on success. So after an error, a timeout or a dropped
   future, the next read is refused with `Disconnected` (category `Validation`, no I/O) until
   `init` runs again: a FIFO read that failed may have popped entries, and a device that lost
   power is back in standby at its defaults, where `read_sample` would find no new sample and
   `read_fifo` an empty FIFO, without an error. `init` is the recovery: it checks the
   identity, rewrites the configuration and starts a new stream. A device that resets between
   two successful reads, with no failed transaction to show it, is not detected; the runbook
   checks it. `read_sample` is for
   `FifoMode::Bypass` and `read_fifo` for the two FIFO modes; the other combination is refused
   with `InvalidConfig` before any I/O and leaves the stream as it was.

8. **`read_fifo` drains into a `SampleBlock`.** It reads INT_SOURCE (the overrun flag, bit 0)
   before anything pops the FIFO, then FIFO_STATUS, and stamps the block from the adapter's
   clock. The two reads are not atomic: the FIFO can fill between them, so the flag reads
   clear and the level 32 or 33. A level of 32 or more therefore counts as an overflow
   whatever the flag said (a full FIFO is what sets it). A status with bit 6 set or more than
   33 entries is `MalformedData`; the trigger flag (bit 7) is ignored, since trigger mode is
   never configured. Then it reads each entry the status reported, oldest first, each in one
   six-byte read of 0x32. Samples that arrive during the drain stay for the next block. Every
   pop clears the overrun flag, so the device keeps no trace of a FIFO that fills during the
   drain (a host stall between two pops, at any point of the drain): the flag read at the
   start cannot see it, and the pops after it clear it before the next drain. The adapter
   therefore checks on its clock after each pop: the entries not popped before that pop plus
   the samples produced since the level was read (rounded down), beyond the 33 the device
   keeps, were lost. The largest such excess sets `overflowed` and is added to `lost`, and the
   next block's estimate starts that many intervals after this block's level read, so the
   same samples are not counted twice. The check assumes the stall came just before the pop,
   so it can count a sample or two of bus latency too many. The block is:
   - `sequence: u64`: 0 for the first block after `init`, then one more per block, with no
     gap. A gap is a block the consumer dropped or missed; a new `init` starts again at 0, and
     samples between two streams belong to neither.
   - `interval: Duration`: exact, from the rate (312 500 ns at 3200 Hz, doubling per step).
   - `counts_per_g: u16` and `samples: Vec<AccelSample>`: the axes in the fixed layout x, y,
     z per sample; an empty `samples` is an empty FIFO.
   - `overflowed: bool`: the device's overrun flag at the start of the drain, a level of 32
     entries or more, or a FIFO that filled during the drain by the host's clock.
   - `lost: u64`: when the flag or the level says so, the samples the device produced since
     the previous drain (the time between the two FIFO_STATUS reads, or since `init`, divided
     by the interval, rounded down) less the entries this block holds, at least 0; plus the
     samples lost during the drain, as above; 0 when `overflowed` is clear. The device counts
     nothing it drops, so this is the host's estimate and can be off by the samples that fit
     in the bus latency; the flag is the device's fact. In stream mode the lost samples precede
     the block's first, or, when the FIFO filled during the drain, fall between two of its
     samples, which are then not equally spaced; in FIFO mode they follow its last.
   - `sampled_at` and `received_at`: when the FIFO's level was read. Per-sample times are not
     invented; a consumer that needs them counts back from `sampled_at` by `interval`, knowing
     the jitter.
   The drawn `AxisLayout` is the `AccelSample` struct (the device always delivers three axes);
   the drawn `tag` is left out (a block belongs to the adapter that produced it, and no
   profile names one); the drawn `first_sample_at` is replaced by `sampled_at`, the one time
   the host knows.

9. **Blocks stay blocks; a single reading becomes measurements.** A block is not turned into
   `Measurement`s: one measurement per sample would be the "stream of unrelated scalars" the
   design forbids, and would drop the interval, the sequence and the loss that make the
   waveform usable. A block shares the envelope's facts as fields (the two stamps, the scale)
   and states its loss explicitly instead of through a quality. A single reading from
   `read_sample` (`AccelReading { sample, counts_per_g, sampled_at, received_at }`) is a
   scalar observation. `read_sample` reads INT_SOURCE first and the data registers only when
   DATA_READY is set; otherwise it returns `Ok(None)` and the stream goes on. Without that
   check, a read during the turn-on time after `init` would return the reset zeros, and a read
   faster than the output rate the previous sample, each as a new `Good` reading, which
   SPEC.md forbids ("it must not silently publish the previous value as a new good reading").
   Reading the data clears DATA_READY. The bypass-mode overrun flag (a sample replaced unread)
   is ignored: a single reading does not claim to keep every sample; `read_fifo` does.
   `AccelReading::measurements()` returns `acceleration_x`,
   `acceleration_y` and `acceleration_z` in `[g]` (the UCUM unit of the standard acceleration
   of free fall) as `F64`, the count in `raw` (`I16`), quality `Good` (read and within the
   resolution; not a claim of calibration), origin `Documented`, the reading's stamps, no
   source time and no protocol status, through the same builder as the MAX31855's.

10. **The test double lives in the testkit.** `FakeI2cBus` plays a script of `I2cStep`s
    (`Ack`, `Reply(bytes)`, `Fail(kind)`, `Silence`, `Delay(duration)`) on a `FakeClock` and
    records every transaction as an `I2cTransfer { address, write, read }`; clones share the
    script and the record (CODING_STANDARDS 13.5). A device that does not acknowledge is
    `Fail(ErrorKind::Disconnected)`, which is what the trait specifies for a NACK, so no
    separate step is needed. A spent script is silent: the call sleeps on the fake clock until
    its deadline and times out. `Delay` holds the transaction on the fake clock before the
    next step, as a host or bus stall would, which is how a test stalls a drain between two
    pops; a stall past the deadline times out at the deadline.

11. **Tier, tests and records.** The module is measured at 100 percent of lines in its own
    row, the double with the testkit at 90. The tests (`tests/boards_adxl345.rs`,
    `tests/boards_adxl345_properties.rs`) cover the write sequence for every range,
    resolution, rate and FIFO mode (at address 0x1D as well as the default 0x53), a wrong
    DEVID (nothing written), a device that does not acknowledge, a bus error during the
    configuration and recovery by `init`, a silent device and an elapsed deadline, short
    replies (including the discarded read of `init`), a read with DATA_READY clear (after
    `init` and after a sample was read), the exact interval of every rate and the loss it
    scales, the decode at each range and resolution against the widths of Table 1 and the
    sensitivities of Table 1 and the FULL_RES description, a count one past each resolution,
    the measurements, a drain of N entries, an empty FIFO, the overflow in stream and FIFO
    mode with its loss count, a full level with the flag clear, loss counted from the previous
    drain and only with the flag or a full level, a FIFO that fills during a stalled drain
    (counted once, not again by the next block), sequence numbers across blocks and after
    `init`, invalid FIFO levels, a failure, a timeout, a stall past the deadline and a dropped
    future in the middle of a drain, and property tests and
    an exhaustive sweep of every pair on every axis at every resolution against a reference
    decoder with different arithmetic. Board adapters are not upstream items: no
    `# Compatibility` section and no parity row. The support level is `simulator-tested` in
    `docs/COMPATIBILITY.md`; hardware verification needs a record in the format of
    `docs/hardware/TEMPLATE.md`.

## Alternatives considered
- **A Linux backend on `i2cdev`, `linux-embedded-hal`, `nix` or `rustix`.** Deferred
  (decision 3): each is a new dependency with `unsafe` for the `ioctl`, and this slice adds
  none.
- **`embedded-hal`'s `I2c` trait.** Rejected, as for SPI in ADR-0026: it is blocking and
  deadline-free.
- **An existing ADXL345 crate.** Rejected: they return floats per sample and drop the overrun
  flag, the FIFO level and the refused counts this adapter keeps.
- **`init` as the constructor, so an adapter exists only configured.** Rejected: recovery
  after a device reset needs `init` again on the same adapter and bus, and a constructor that
  consumed the bus on failure would lose it.
- **A watermark option and interrupt configuration.** Rejected (decision 5): nothing in this
  slice reads them.
- **Left-justified data and the trigger mode.** Rejected: never configured, so never decoded
  or drained.
- **`MalformedData` for a wrong DEVID.** Rejected (decision 4).
- **Only the overrun flag, no loss count.** Rejected: SPEC.md asks to report missing samples,
  and the flag alone says that some were lost, not how many; the estimate is labeled as one.
- **One measurement per FIFO sample, or a `Quality` on the block.** Rejected (decision 9).
- **An error, or a reading marked stale, when DATA_READY is clear.** Rejected (decision 9):
  no new sample is the normal answer to a poll faster than the output rate, not a failure, and
  the reset zeros of a device turning on are not a previous value that could be stale.
  `Option` says it without a new error kind.
- **Waiting out the turn-on time in `init`.** Rejected: DATA_READY answers the question on
  the device's own word, and `init` would otherwise sleep on the clock for up to about 161 ms
  (at 6.25 Hz).
- **Only the overrun flag for `overflowed`, or a re-read of INT_SOURCE after FIFO_STATUS.**
  Rejected (decision 8): a full level is the device's own evidence of the fill, and a second
  read narrows the race without closing it.
- **A generic `SampleBlock` in `sensors`, with a tag and an axis-layout type.** Deferred: one
  device produces blocks today; a second (another accelerometer, an SPI ADXL345) decides what
  is common.

## Consequences
- `stonecrab::boards` gains `I2cBus`, `Adxl345`, `Adxl345Config`, `AccelRange`, `OutputRate`,
  `FifoMode`, `AccelSample`, `AccelReading` and `SampleBlock`; `stonecrab::testkit` gains
  `FakeI2cBus`, `I2cStep` and `I2cTransfer` (feature `boards`). No feature, no dependency, no
  change to `Cargo.lock`; no existing enum gains a variant. The MAX31855 items keep their paths.
- No host can drive an ADXL345 with the crate alone yet; an application supplies an
  `I2cBus`. The bench runbook says how.
- After any failed or dropped read the caller must call `init`, which restarts the stream
  (sequence 0) and empties the FIFO: what was in it is lost, and the new stream says nothing
  about it.
- `lost` is an estimate on the host's clock; `overflowed` is the device's word (its flag or a
  full level) or, for a FIFO that filled during the drain, the host's clock, since the pops
  clear the flag. A consumer that needs a guarantee treats any `overflowed` block as broken.
- `read_sample` returns `Option<AccelReading>`: a caller that polls faster than the output
  rate, or right after `init`, gets `None` and polls again. ARCHITECTURE 6.1 drew it returning
  an `AccelSample`.
- The SPI interface of the ADXL345 (SPEC: "SPI as a follow-on") needs `SpiBus` to gain a
  transmitting method (ADR-0026 decision 2) and is a later slice.

## Compliance
- `crates/stonecrab/Cargo.toml` has the `[[test]]` stanzas of `boards_adxl345` (`boards`,
  `testkit`) and `boards_adxl345_properties` (`boards`), matching each file's `#![cfg]`.
- `packaging/stonecrab.txt` lists `src/boards/adxl345.rs`, `src/boards/linux.rs`,
  `src/boards/max31855.rs` and `src/testkit/fake_i2c.rs`.
- `scripts/coverage-gate.sh` has `boards|100` and `scripts/lint-attrs.sh` the `boards`
  promotions; `lib.rs` declares them on `pub mod boards`.
- The write sequence is pinned by
  `init_checks_the_identity_then_writes_the_default_configuration_in_order` and
  `init_writes_every_range_resolution_rate_and_fifo_mode_as_configured`; the overflow by
  `a_stream_overflow_sets_the_overrun_flag_and_counts_the_samples_lost` and
  `a_fifo_mode_overflow_counts_the_samples_dropped_after_the_full_fifo` and
  `a_full_fifo_is_an_overflow_even_when_the_flag_read_before_it_was_clear` and
  `a_fifo_that_fills_during_the_drain_is_an_overflow_counted_once`; the identity by
  `a_wrong_devid_is_an_invalid_identity_and_nothing_is_written`; the DATA_READY check by
  `a_read_without_a_new_sample_returns_none_without_reading_the_data_and_the_stream_goes_on`;
  the intervals by `every_output_rate_has_its_exact_interval_and_scales_the_loss`.
- Failure-case tests carry `// covers: SPEC mandatory-failure-cases / F10 ...` tags.
