# Hardware runbook: ADXL345 accelerometer over I2C

> This is a runbook, not a hardware record. Nothing in it has been run on a device. **Operating
> the hardware needs the maintainer's separate approval** (STATUS.md); writing this file does
> not give it. When the run is approved and done, copy the record table below to
> `docs/hardware/adxl345-i2c-<yyyy-mm-dd>.md` (the format of `TEMPLATE.md`) and fill every
> field.

## Support level evidenced

Today: **simulator-tested, not hardware-verified.** The identity check, the configuration
writes, the sample decode, the FIFO drain with its overflow and loss accounting, and the
measurements ran against a scripted bus (`FakeI2cBus`, slice S64, ADR-0029). That shows what
Stonecrab does with the register bytes the tests supply. It shows nothing about an ADXL345,
an I2C controller or a Linux host, and every behavior the datasheet leaves open (the section
"What only hardware can settle") is an assumption of the tests. `hardware-verified` for this
adapter needs the run below, a record in the format of `TEMPLATE.md`, and a note for every open
item that moved. Sensor tests do not establish calibration accuracy, and this run does not
either; it does not show that the adapter is fit for vibration analysis.

## Evidence behind this runbook

| Item | Where |
| --- | --- |
| Vendor document | ADXL345 datasheet, <https://www.analog.com/en/products/adxl345.html>. ADR-0029 was written against Rev. 0 (24 pages, PDF dated 2009-05-29, from the SparkFun mirror, sha256 `2c0d2a0e00ec8cdfbab5fa801859af1f5c4cf8b30c3738e262cf9a0a3bcdec74`); analog.com did not answer on 2026-10-29. Step A0 checks the current revision against it. Vendor documentation, all rights reserved: pinned, never copied |
| Decision record | `docs/adr/0029-adxl345-and-sample-blocks.md` (bus trait, deferred Linux backend, configuration writes, decode, stream rule, sample blocks) |
| Code | `crates/stonecrab/src/boards.rs` (`I2cBus`), `crates/stonecrab/src/boards/adxl345.rs` and `crates/stonecrab/src/testkit/fake_i2c.rs` |
| Tests | `crates/stonecrab/tests/boards_adxl345.rs`, `boards_adxl345_properties.rs` |
| Support table | `docs/COMPATIBILITY.md`, the "Direct bench adapters" rows |

## Setup

1. **Accelerometer.** One ADXL345 on a breakout the maintainer chooses, powered from the supply
   the breakout documents. Wiring is the breakout's own diagram; this runbook copies none of
   it. CS must be tied high (I2C mode). Record the level of SDO/ALT ADDRESS: low is address
   0x53 (the adapter's default), high is 0x1D. Mount the board flat and rigid, z axis up, on a
   surface that can be turned to each axis in turn.
2. **Host.** A Linux host with an I2C controller; the project's target is a BeagleBone with
   Debian 13 Trixie (docs/SPEC.md "Platforms and performance"), and the record names the board,
   the OS, the architecture, the kernel and the toolchain. Enable the controller and its
   `/dev/i2c-N` node through the board's device-tree overlay (the board vendor's documentation
   says how), with the bus at 100 or 400 kHz, and give the user that runs the test permission
   on the node. Record the bus number and its speed. No other device at the same address.
3. **A backend.** Stonecrab ships no Linux I2C backend yet (ADR-0029 decision 3: it needs an
   `ioctl`). Either a later slice has added one, and the run uses it, or the scratch program
   below implements `stonecrab::boards::I2cBus` itself over a Linux I2C crate of the
   maintainer's choosing: `write` as one write transaction, and `write_read` as one combined
   transaction with a repeated start (the `I2C_RDWR` interface; two separate transactions
   with a stop between them are not the same on the wire), each run on a blocking thread
   (`tokio::task::spawn_blocking`) and raced against the deadline. Record the crate and its
   version. The backend is the scratch program's code, not the repository's.
4. **Library.** A Stonecrab build at a recorded commit with the feature `boards`
   (`stonecrab = { ..., features = ["boards"] }`), built for the host (the project's cross
   targets are aarch64 and armv7 Linux).
5. **A scratch program outside the repository.** It initializes the device and reads in a
   loop. Apart from its backend, it is this:

   ```rust
   use std::sync::Arc;
   use std::time::Duration;

   use stonecrab::boards::{Adxl345, Adxl345Config, FifoMode, I2cBus};
   use stonecrab::transport::SystemClock;
   use stonecrab::{Clock, Deadline};

   async fn run(bus: Box<dyn I2cBus>, fifo: FifoMode) -> Result<(), stonecrab::PlcError> {
       let clock = Arc::new(SystemClock::default());
       let mut config = Adxl345Config::default(); // 0x53, 2 g, full resolution, 100 Hz
       config.fifo = fifo;
       let mut sensor = Adxl345::new(bus, clock.clone(), config);
       sensor.init(Deadline::after(clock.now(), Duration::from_secs(1))?).await?;
       loop {
           let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
           if fifo == FifoMode::Bypass {
               match sensor.read_sample(deadline).await {
                   Ok(Some(reading)) => {
                       for m in reading.measurements()? {
                           println!("{} = {:?} {} raw {:?}", m.name, m.value, m.unit, m.raw);
                       }
                   }
                   Ok(None) => println!("no new sample"),
                   Err(error) => println!("error {:?}: {error}", error.kind()),
               }
           } else {
               match sensor.read_fifo(deadline).await {
                   Ok(block) => println!(
                       "block {} samples {} overflowed {} lost {}",
                       block.sequence, block.samples.len(), block.overflowed, block.lost
                   ),
                   Err(error) => println!("error {:?}: {error}", error.kind()),
               }
           }
           tokio::time::sleep(Duration::from_millis(100)).await;
       }
   }
   ```

   After an error the adapter refuses reads until `init` runs again (ADR-0029 decision 7); the
   recovery steps add the call. The sample needs `tokio` with `macros`, `rt` and `time`, and
   the `transport` feature for `SystemClock`.
6. **Approval.** Obtain the maintainer's approval for the run in writing before step A1, and
   note its date in the record.

## Acceptance steps

Each step says what to do and what passes. Every value the run observes goes in the record,
passing or not; a difference from ADR-0029 is the finding, not a failure to hide.

| Step | What runs | Pass criterion, and what to record |
| --- | --- | --- |
| A0 datasheet | Retrieve the current datasheet, record its revision and sha256, and compare it with the register table of ADR-0029: DEVID, the addresses and bit positions of BW_RATE, POWER_CTL, INT_SOURCE, DATA_FORMAT, the data registers, FIFO_CTL and FIFO_STATUS, the rate codes, Table 1's sensitivities and resolutions and the FULL_RES scale factor, the I2C addresses, the FIFO depth and the overrun behavior | Each item of the ADR agrees. A disagreement stops the run: fix ADR-0029, `adxl345.rs` and the tests first. Record the revision and hash, and which rates the revision adds |
| A1 identity | With the bus up and nothing else running, read DEVID once with the host's own tool (the one the board's documentation names), then run `verify_identity` | 0xE5 both ways. Record the tool and the address |
| A2 init | Run `init`, then read back BW_RATE, POWER_CTL, DATA_FORMAT and FIFO_CTL with the host's tool | The values ADR-0029 decision 5 lists for the configuration used (0x0A, 0x08, 0x08, 0x80 for the default). Record them |
| A3 stationary axes | Bypass mode, default configuration. First call `read_sample` straight after `init` with no delay, and twice in a row with no delay after a sample was returned; then read 100 samples with the board flat (z up), then on each of its six faces | The calls without delay return `None` (open item 8); a sample of reset zeros is a finding. Then the axis pointing up reads about +1 g and the one pointing down about -1 g; the others about 0 g. Expect the datasheet's 0 g offsets (up to about 0.15 g on x and y, 0.25 g on z) and sensitivity spread (232 to 286 LSB/g at 2 g): agreement within those, not better. Record the mean count per axis and face. This does not establish calibration |
| A4 ranges and resolutions | Repeat A3 flat only, for 10-bit mode at 2, 4, 8 and 16 g and full resolution at 16 g | +1 g on z reads about 256, 128, 64, 32 and 256 counts in turn; `counts_per_g` matches. No `MalformedData` (open item 2). Record the counts |
| A5 stream drain | Stream mode at 100 Hz, `read_fifo` every 100 ms for 60 s | About 10 samples per block, `overflowed` false and `lost` 0 in every block, sequence numbers 0, 1, 2 without a gap. Record the entries per block and their spread (open item 1) |
| A6 overflow | Stream mode at 100 Hz: drain, wait 1 s, drain; then the same in FIFO mode. Then once more: wait 1 s, run `init` without draining, and drain at once | The second block of each has `overflowed` true, about 32 or 33 samples, and `lost` near 100 less the samples delivered. The block right after the new `init` has `overflowed` false. Record the entries, `lost` and the wait (open items 1, 3 and 9) |
| A7 high rate | Stream mode at 3200 Hz, `read_fifo` as fast as the loop goes (no sleep), for 10 s, at 100 kHz and then 400 kHz | Record the blocks with `overflowed` set and the sum of `lost`. Overflow at this rate is expected on a slow bus and is the finding: the adapter promises reporting, not bandwidth |
| A8 no device | Disconnect the breakout's power, or point the configuration at an address with nothing on it, and run `init` | `Disconnected` from the backend (no acknowledge); nothing else. Record what the backend reported |
| A9 wrong device | If another I2C device is at hand, point the configuration at it and run `init` | `Trust`, naming its DEVID byte; the host's tool shows its registers unchanged. Skip, and say so, if none is at hand |
| A10 recovery | During A5, unplug the breakout's power for two seconds and plug it back, with the program calling `init` after each error | The reads fail (`Disconnected` or `Timeout`), are refused until `init`, and after a successful `init` the blocks start again at sequence 0. Record whether any read succeeded with zeros before an error showed the reset (open item 4) |
| A11 soak | Stream mode at 100 Hz, `read_fifo` every 100 ms for 10 minutes | Count the errors by kind, and the blocks with `overflowed`. None is expected. Every `MalformedData` is a finding: record the bytes |

## What only hardware can settle

ADR-0029 and the tests assume the value in the right column; the run settles each, and the
ADR, the adapter or the tests change to match.

| No | Open item | Assumption | Settled by |
| --- | --- | --- | --- |
| 1 | Whether FIFO_STATUS's entries count includes the sample waiting at the output (32 or 33 at most) | The adapter reads exactly the entries reported and accepts up to 33 | A5, A6 |
| 2 | That right-justified data at each range and resolution is sign-extended to 16 bits and never exceeds the resolution ("All data, except that for the +/-16 g range, must be clipped") | It is, and a count outside is `MalformedData` | A4, A11 |
| 3 | When the overrun bit sets: when the FIFO reaches 32, or when a sample is first dropped, and whether reading INT_SOURCE leaves it set | When the FIFO fills, and it stays set until the data registers are read; the adapter also counts a level of 32 or more as an overflow. Every pop clears the bit, so a FIFO that fills during a drain (a host stall between two pops) leaves no flag; the adapter detects it only on its own clock, after each pop (ADR-0029 decision 8) | A6, A7 (compare the blocks flagged with the stalls the host saw) |
| 4 | Whether a device that lost power fails a transaction before it answers again, or answers in standby with zeros (which the adapter would not detect) | A power loss fails at least one transaction | A10 |
| 5 | That the one-byte reads of INT_SOURCE and FIFO_STATUS before a drain do not pop the FIFO | They do not | A5 (no gap in the samples of a steady signal) |
| 6 | The rate the bus sustains with this adapter at 100 and 400 kHz | Not promised; reported as loss | A7 |
| 7 | That writing FIFO_CTL to bypass empties the FIFO at `init` | It does, so the first block holds only new samples | A6, A10 |
| 8 | That DATA_READY is clear after `init` until the first sample (the turn-on time, about one output period plus 1.1 ms) and again after each six-byte read until the next sample | It is, so `read_sample` returns `None` in both cases and never the reset zeros or a repeat | A3 |
| 9 | That the six-byte read `init` makes in standby, with the FIFO bypassed, clears an overrun flag an earlier stream left | It does, so the first block of a new stream on a device that kept power is not flagged | A6 (run `init` again right after the overflowed drain, then drain at once: `overflowed` false) |
| 10 | Whether, at 16 g, an acceleration beyond the range rolls over to a count that fits (the datasheet clips only the other ranges) | It can, and it is reported as `Good`; nothing detects it | not settled by this run: a shock beyond 16 g is not a bench step; record any count near the end of the range in A4 |

## Record (to be filled by the approved run)

| Field | Value |
| --- | --- |
| Device model | ADXL345 (breakout: ) |
| Firmware | none (the chip has no firmware); datasheet revision and sha256: |
| Adapter | the host's I2C controller and `/dev/i2c-N`, through the backend named in setup step 3 |
| Wiring / interface configuration | address, bus number, bus speed, CS and SDO/ALT ADDRESS levels |
| Host OS / architecture | |
| Library commit | |
| Operation set | `init`, `verify_identity`, `read_sample`, `read_fifo`, per the steps above |
| Load | |
| Result | |

## Notes

Observations, deviations and links to logs. Never include credentials, network plans or
private device data. When a step moved an open item, say which ADR decision and which test the
change touches.
