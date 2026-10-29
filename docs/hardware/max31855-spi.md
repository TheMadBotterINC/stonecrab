# Hardware runbook: MAX31855 thermocouple converter over SPI

> This is a runbook, not a hardware record. Nothing in it has been run on a device. **Operating
> the hardware needs the maintainer's separate approval** (STATUS.md); writing this file does
> not give it. When the run is approved and done, copy the record table below to
> `docs/hardware/max31855-spi-<yyyy-mm-dd>.md` (the format of `TEMPLATE.md`) and fill every
> field.

## Support level evidenced

Today: **simulator-tested, not hardware-verified.** The frame decode, the `Max31855` adapter,
the three measurements and the Linux backend (`LinuxSpiDev`) ran against scripted buses
(`FakeSpiBus`), in-memory readers and temporary files (slice S63, ADR-0026). That shows what
Stonecrab does with the frames the tests supply. It shows nothing about a converter, an SPI
controller or a Linux `spidev` node, and every behavior the documents leave open (the section
"What only hardware can settle") is an assumption of the tests. `hardware-verified` for this
adapter needs the run below, a record in the format of `TEMPLATE.md`, and a note for every open
item that moved. Sensor tests do not establish calibration accuracy, and this run does not
either.

## Evidence behind this runbook

| Item | Where |
| --- | --- |
| Vendor document | MAX31855 datasheet, <https://www.analog.com/en/products/max31855.html>. The frame layout of ADR-0026 was checked on 2026-10-29 against Rev 2 (2/12, document 19-5793, sha256 `45904b81e79c46af49fd5c880df7ec04678c8599be9df73cea8422d241ceb7a2`; see the addendum of the ADR); step B0 checks it again against the datasheet of the part actually used and records its revision and hash. Vendor documentation, all rights reserved: pinned, never copied |
| Decision record | `docs/adr/0026-board-adapters.md` (module, backend, decode, refused frames, measurements) |
| Code | `crates/stonecrab/src/boards.rs` (`SpiBus`), `crates/stonecrab/src/boards/max31855.rs` (`Max31855`, `ThermocoupleReading`), `crates/stonecrab/src/boards/linux.rs` (`LinuxSpiDev`; the module became a directory in S64) and `crates/stonecrab/src/testkit/fake_spi.rs` |
| Tests | `crates/stonecrab/tests/boards_max31855.rs`, `boards_properties.rs`, `boards_spidev.rs` |
| Support table | `docs/COMPATIBILITY.md`, the "Direct bench adapters" rows |

## Setup

1. **Converter.** One MAX31855 on a breakout or board the maintainer chooses, powered from the
   supply the breakout documents, with a thermocouple of the type the chip variant expects
   (the variant, K, J, N, T, E, S or R, is printed on the chip or listed by the breakout; the
   chip does the linearization for its own type). Wiring is the breakout's own diagram; this
   runbook copies none of it. Use a thermocouple that is safe to short and to unplug.
2. **Host.** A Linux host with an SPI controller; the project's target is a BeagleBone with
   Debian 13 Trixie (docs/SPEC.md "Platforms and performance"), and the record names the board,
   the OS, the architecture, the kernel and the toolchain. SPI wiring: chip select, clock and
   the converter's data output to the controller's MISO. The converter has no data input, so
   MOSI stays unconnected.
3. **The `spidev` node.** Enable the controller and a `spidev` node for the chip select
   through the board's device-tree overlay (the board vendor's documentation says how), and
   give the user that runs the test read permission on `/dev/spidevB.C`. `LinuxSpiDev` only
   reads the node, so it cannot set the bus: **the device tree must give the node SPI mode 0
   (clock idle low, data sampled on the leading edge) and a maximum speed not above the
   converter's limit** (5 MHz is the figure ADR-0026 assumes; step B0 checks it). Record the
   mode, the speed and the word size the node ends up with.
4. **Library.** A Stonecrab build at a recorded commit with the feature `adapters-linux`
   (`stonecrab = { ..., features = ["adapters-linux"] }`), built for the host (the project's
   cross targets are aarch64 and armv7 Linux).
5. **A scratch program outside the repository.** It opens the node, reads in a loop and prints
   what it gets. It is the whole of the runtime code; nothing else of the repository runs.

   ```rust
   use std::sync::Arc;
   use std::time::Duration;

   use stonecrab::Clock;
   use stonecrab::Deadline;
   use stonecrab::boards::{LinuxSpiDev, Max31855};
   use stonecrab::transport::SystemClock;

   #[tokio::main(flavor = "current_thread")]
   async fn main() -> Result<(), stonecrab::PlcError> {
       let clock = Arc::new(SystemClock::default());
       let bus = LinuxSpiDev::open("/dev/spidev1.0")?; // the node of the record
       let mut sensor = Max31855::new(Box::new(bus), clock.clone());
       loop {
           let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
           match sensor.read(deadline).await {
               Ok(reading) => {
                   for m in reading.measurements()? {
                       println!("{} = {:?} {} ({}) raw {:?}", m.name, m.value, m.unit, m.quality, m.raw);
                   }
                   println!("frame {:#010x} fault {:?}", reading.raw, reading.fault);
               }
               Err(error) => println!("error {:?}: {error}", error.kind()),
           }
           tokio::time::sleep(Duration::from_millis(250)).await;
       }
   }
   ```

   The sample needs `tokio` with `macros`, `rt` and `time`. The loop interval is the program's
   business; the converter finishes a conversion about every 100 ms, so a faster loop repeats
   a value.
6. **Approval.** Obtain the maintainer's approval for the run in writing before step B1, and
   note its date in the record.

## Acceptance steps

Each step says what to do and what passes. Every value the run observes goes in the record,
passing or not; a difference from ADR-0026 is the finding, not a failure to hide.

| Step | What runs | Pass criterion, and what to record |
| --- | --- | --- |
| B0 datasheet | Retrieve the datasheet of the part used, record its revision and sha256, and compare it with the table of ADR-0026: bit positions, the fields' widths and LSB sizes, the reserved bits, the fault bit as the OR of the three flags, the maximum SCK frequency and the SPI mode | Each item of the ADR agrees. A disagreement stops the run: fix ADR-0026, `boards/max31855.rs` and the tests first. Record the revision and hash |
| B1 bus setup | Read the node's mode, speed and word size from the kernel (the tool the board's documentation names), and read it with `LinuxSpiDev` once | Mode 0, a speed not above the chip's limit, 8 bits. The first read returns an accepted frame (open item 3). Record the three settings |
| B2 room temperature | Read for 60 s with the thermocouple at room temperature, next to an independent thermometer on the same junction | `thermocouple_temperature` and `cold_junction_temperature` are `Good`, within a few degrees (not tenths) of the thermometer, steady to a degree or two. `thermocouple_fault` is 0. This does not establish calibration |
| B3 ice point | Put the thermocouple junction in an ice-and-water bath and read for 60 s | The reading is near 0 C, `Good`. If it reads exactly 0 C with the cold junction also at exactly 0.0000 C, record it (open item 7) |
| B4 negative reading | With the maintainer's agreement, cool the junction below 0 C (a salt-and-ice bath, or a freezer if the thermocouple and its insulation allow it) and read | A negative `thermocouple_temperature` with the right sign and size against the thermometer, and its `raw` count negative. Record the counts. The cold junction need not move |
| B5 open circuit | With the program running, unplug the thermocouple from the converter, wait three reads, plug it back | During the unplug: `fault.open` is set, `thermocouple_fault` is 1, `thermocouple_temperature` is `Bad` with no value, `cold_junction_temperature` stays `Good`. After: `Good` again within a few reads. Record the frame (open items 1 and 2) |
| B6 short to ground and to Vcc | **Only with the maintainer's agreement, and only if the breakout's documentation allows it.** Short the thermocouple input to ground for three reads, then to the supply, each time restoring it | `short_to_ground` (`thermocouple_fault` 2) and `short_to_vcc` (4) set in turn, the temperature `Bad`. Record whether other flags set with them (open item 2). Skip, and say so, if the documentation does not allow it |
| B7 no device | Disconnect the converter's data output (or run the program on a free chip select with nothing attached), and read | `Disconnected` from every read. Record the frame the host sees on the line (all ones, all zeros or something else) and with the chip powered and unpowered (open item 5) |
| B8 recovery | Reconnect the converter after B7 without restarting the program | The next read is a reading again, with no restart |
| B9 repeat rate | Read 40 times in 1 s | No error, and the values repeat in steps (conversions are about 100 ms apart). Record the interval in which the frame changes |
| B10 soak | Read once a second for 10 minutes at a steady temperature | Count the errors by kind. None is expected. Every `MalformedData` is a finding: record the frame (open item 6) |
| B11 permissions and path | Run as an unprivileged user in the group the setup names | The node opens and reads without root. Record the node, the group and the kernel |

## What only hardware can settle

ADR-0026 and the tests assume the value in the right column; the run settles each, and the
ADR, the decoder or the tests change to match.

| No | Open item | Assumption | Settled by |
| --- | --- | --- | --- |
| 1 | What the thermocouple field (bits 31 to 18) holds while a fault is flagged | It is not a temperature; the adapter offers none and keeps the count in `raw` | B5, B6 |
| 2 | Whether the three fault flags can be set together, and whether the fault bit is always their OR | Flags are independent, the fault bit is their OR, and a frame that breaks the OR is `MalformedData` | B5, B6, B10 |
| 3 | That `read(2)` of 4 bytes on a `spidev` node is one transfer of 4 bytes with chip select held low for all 32 clocks and released after (no extra clocks, no second cycle) | It is, as the kernel's `spidev` documentation describes half-duplex reads | B1 (an accepted frame), B9 (frames change in steps), a logic analyzer if one is at hand |
| 4 | That the node's defaults (mode, speed) are what the device tree set, and that the node never changes them | They are, and mode 0 at no more than 5 MHz suits the chip | B0, B1 |
| 5 | What a host line with nothing answering reads as, with the chip unpowered, unconnected and powered with a floating output | All ones or all zeros, both `Disconnected` | B7 |
| 6 | The rate of corrupt frames on a short bench wire | None in a 10 minute run | B10 |
| 7 | Whether an all-zeros frame can come from a working converter (a true 0 C at both junctions) | It cannot in practice, and a false `Disconnected` there is accepted (ADR-0026 decision 5) | B3 |
| 8 | The conversion interval and the first frame after power-up (the chip may answer before its first conversion finishes) | About 100 ms; the first frame is a reading | B9, and a power cycle |
| 9 | The thermocouple type and chip variant on the bench | The variant matches the thermocouple; the adapter does not know the type | B2 |

## Record (to be filled by the approved run)

| Field | Value |
| --- | --- |
| Device model | MAX31855 (variant: ) |
| Firmware | none (the chip has no firmware); datasheet revision and sha256: |
| Adapter | the host's SPI controller and the `/dev/spidev` node, read by `LinuxSpiDev` (feature `adapters-linux`) |
| Wiring / interface configuration | |
| Host OS / architecture | |
| Library commit | |
| Operation set | read of one 32-bit frame (`Max31855::read`), per the steps above |
| Load | |
| Result | |

## Notes

Observations, deviations and links to logs. Never include credentials, network plans or
private device data. When a step moved an open item, say which ADR decision and which test the
change touches.
