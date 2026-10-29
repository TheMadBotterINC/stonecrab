# Stonecrab Boards
Direct board adapters for Stonecrab (`stonecrab::boards`, behind the `boards` feature, off
by default): a sensor chip on a bus of the host, read without a gateway or a protocol
driver. Two adapters ship: the MAX31855 thermocouple converter on SPI (the thermocouple
temperature, the internal cold-junction temperature and the converter's open and short
fault flags; ADR-0026) and the ADXL345 three-axis accelerometer on I2C (single samples and
FIFO drains into sample blocks; ADR-0029). A single reading converts to the measurements of
`stonecrab::sensors` (the same envelope, units and quality model as a sensor profile), so a
board and a Modbus device look alike to the application; a block of accelerometer samples
stays a block. Board adapters are Stonecrab extensions, not upstream PLC4X parity items, and
they change no industrial parity status.

The module is `boards`, the feature `boards` needs `sensors` and nothing else (no Tokio,
no new dependency). The Linux backend, `LinuxSpiDev`, is behind `adapters-linux`, which
adds `transport` for its worker thread and timer.

| Capability | Support level |
| --- | --- |
| `SpiBus` trait | build-tested |
| MAX31855 frame decode (`ThermocoupleReading::from_raw`), faults, negative readings, refused frames | simulator-tested |
| `Max31855::read` and `ThermocoupleReading::measurements` over a scripted bus | simulator-tested |
| `LinuxSpiDev` (feature `adapters-linux`) over in-memory readers | simulator-tested |
| `LinuxSpiDev` on a real `/dev/spidev` node | planned |
| MAX31855 on a real converter (`docs/hardware/max31855-spi.md`) | planned |
| `I2cBus` trait | build-tested |
| ADXL345 identity check, configuration writes, sample decode (`AccelSample::from_registers`), `read_sample` and its measurements, `read_fifo` and `SampleBlock` over a scripted bus | simulator-tested |
| A Linux I2C backend (`LinuxI2cDev`) | planned (ADR-0029 decision 3) |
| ADXL345 on a real accelerometer (`docs/hardware/adxl345-i2c.md`) | planned |
| Hardware verification | planned |

`simulator-tested` here means the decoders, the adapters and the backend ran against the
testkit's `FakeSpiBus` and `FakeI2cBus` (scripted replies, bus errors, a device that does not
acknowledge, stalls and silence on a fake clock) and against in-memory readers; no converter, no
accelerometer, no SPI or I2C controller and no Linux host took part.
The temperatures of the datasheet's Tables 4 and 5, further hand-computed counts including
the extremes of both fields, and every combination of the frame's bits are decoded; calibration accuracy is not tested and is not claimed.

## Reading a thermocouple
`Max31855` owns a `Box<dyn SpiBus>` and the `Clock` it stamps readings with.
`read(deadline)` clocks one 32-bit frame out of the converter and decodes it into a
`ThermocoupleReading`: the two temperatures as the integer counts the chip sent
(`thermocouple_quarters`, 0.25 C a count, 14 bits; `cold_junction_sixteenths`, 0.0625 C a
count, 12 bits; both signed), the fault flags and the raw frame. Nothing is rounded, and a
negative temperature is a negative count.

```rust
# #[cfg(feature = "testkit")]
# fn main() -> Result<(), stonecrab::PlcError> {
use std::sync::Arc;
use std::time::Duration;
use stonecrab::{Clock, Deadline};
use stonecrab::boards::Max31855;
use stonecrab::sensors::Quality;
use stonecrab::testkit::{FakeClock, FakeSpiBus, SpiStep, block_on_with};

let clock = FakeClock::default();
// An open thermocouple (fault bit and flag OC) on a chip at 25.0 C.
let frame = vec![0x00, 0x01, 0x19, 0x01];
let bus = FakeSpiBus::new(vec![SpiStep::Reply(frame)], clock.clone());
let mut sensor = Max31855::new(Box::new(bus), Arc::new(clock.clone()));

let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
let reading = block_on_with(&clock, sensor.read(deadline))?;
assert!(reading.fault.is_some_and(|fault| fault.open));
assert_eq!(reading.cold_junction_celsius(), 25.0);

let measurements = reading.measurements()?;
assert_eq!(measurements[0].quality, Quality::Bad); // the thermocouple temperature
assert_eq!(measurements[1].quality, Quality::Good); // the cold junction
# Ok(())
# }
# #[cfg(not(feature = "testkit"))]
# fn main() {}
```

## What a reading becomes
`ThermocoupleReading::measurements` returns three measurements in a fixed order:

| Name | Unit | Value | Quality |
| --- | --- | --- | --- |
| `thermocouple_temperature` | `Cel` | `F64`; `None` while a fault is flagged | `Good`; `Bad` with a fault |
| `cold_junction_temperature` | `Cel` | `F64` | `Good` |
| `thermocouple_fault` | `1` | `U8`: bit 0 open, bit 1 short to ground, bit 2 short to Vcc; 0 for none | `Good` |

The fault stays visible: the temperature is `Bad` with no value, the internal temperature
stays usable (a thermocouple fault does not touch the internal sensor), and the fault
measurement says which flags are set. Each carries its integer count in `raw`. Their
`origin` is `Documented`: the decoding is written from the datasheet and no hardware
record exists.

## Frames that are refused
A frame the converter could not have sent gives an error and no reading:

- all zeros or all ones, `Disconnected`: nothing answered (a floating data line reads as
  all ones, a line held low as all zeros). A real reading of exactly 0 C at both junctions
  is refused too, because the two cases cannot be told apart; the chip would have to sit at
  exactly 0.0000 C.
- a reserved bit set, `MalformedData`.
- a fault bit that disagrees with the three flags, `MalformedData`.

A bus failure keeps its kind (`Timeout` at the deadline, `Disconnected`), and a bus that
returns other than four bytes is `MalformedData`. A converter fault (open, short) is not an
error: it is a reading.

## The Linux backend
`LinuxSpiDev` reads an `/dev/spidevB.C` node with plain `read(2)` on a dedicated thread,
so it needs no `ioctl`, no dependency and no `unsafe`. The kernel turns each read into one
chip-select cycle. The price is that nothing here sets the clock mode or speed: the device
tree and the driver do (mode 0, at most 5 MHz for the MAX31855). The runbook
`docs/hardware/max31855-spi.md` lists the wiring, the device-tree settings and the checks
only a bench can do.

## Reading an accelerometer
`Adxl345` owns a `Box<dyn I2cBus>`, the `Clock` it stamps samples with and an
`Adxl345Config` (address, range, output rate, FIFO mode, full resolution). `init(deadline)`
reads DEVID (anything but 0xE5 is `Trust`, and nothing is written), then writes the
configuration in a fixed order: standby, FIFO bypass (which empties it), one discarded read
of the data registers (which clears the flags an earlier stream left), zero offsets, the
data format, the rate, the FIFO mode, measure. These are the only writes the adapter makes;
there is no public register write. With the FIFO bypassed, `read_sample` reads a sample the
device flags as new (`DATA_READY`) and returns `None` when there is none: while the device
turns on after `init` (about one output period plus 1.1 ms), when its registers still hold
reset zeros, and when the host reads faster than the output rate. With the FIFO in FIFO or
stream mode, `read_fifo` drains it into a `SampleBlock`.

```rust
# #[cfg(feature = "testkit")]
# fn main() -> Result<(), stonecrab::PlcError> {
use std::sync::Arc;
use std::time::Duration;
use stonecrab::{Clock, Deadline};
use stonecrab::boards::{Adxl345, Adxl345Config, OutputRate};
use stonecrab::testkit::{FakeClock, FakeI2cBus, I2cStep, block_on_with};

let clock = FakeClock::default();
let mut config = Adxl345Config::default(); // 0x53, 2 g, full resolution, stream mode
config.rate = OutputRate::Hz100;
let mut script = vec![I2cStep::Reply(vec![0xE5]), I2cStep::Ack, I2cStep::Ack]; // DEVID, standby, bypass
script.push(I2cStep::Reply(vec![0; 6])); // the data registers, read once to clear the flags
script.extend(std::iter::repeat_n(I2cStep::Ack, 5)); // the rest of the configuration
script.push(I2cStep::Reply(vec![0x01])); // INT_SOURCE: the overrun flag
script.push(I2cStep::Reply(vec![0x20])); // FIFO_STATUS: 32 entries
for _ in 0..32 {
    script.push(I2cStep::Reply(vec![0, 0, 0, 0, 0x00, 0x01])); // z = +1 g
}
let bus = FakeI2cBus::new(script, clock.clone());
let mut sensor = Adxl345::new(Box::new(bus), Arc::new(clock.clone()), config);

let deadline = Deadline::after(clock.now(), Duration::from_secs(5))?;
block_on_with(&clock, sensor.init(deadline))?;
clock.advance(Duration::from_secs(1)); // the host was late: 100 samples were produced
let block = block_on_with(&clock, sensor.read_fifo(deadline))?;
assert_eq!(block.sequence, 0);
assert_eq!(block.interval, Duration::from_millis(10));
assert_eq!(block.samples.len(), 32);
assert!(block.overflowed);
assert_eq!(block.lost, 68);
# Ok(())
# }
# #[cfg(not(feature = "testkit"))]
# fn main() {}
```

Samples are counts (`AccelSample { x, y, z }` as `i16`), right-justified and sign-extended,
with `counts_per_g` beside them: 256 at full resolution, and 256, 128, 64 or 32 in 10-bit
mode at 2, 4, 8 or 16 g. A count outside the configured resolution is `MalformedData`. The
device clips to its range at 2, 4 and 8 g; at 16 g the datasheet does not promise it, so an
acceleration beyond the range can roll over to a count that fits and is not detected.

## Sample blocks and loss
A `SampleBlock` is raw waveform data: equally spaced samples (`interval`, exact), oldest
first, with three axes each, unless `overflowed` says a gap may fall among them. It is not turned into measurements. It says what it does not
hold:

- `sequence` counts blocks from 0 after each `init`, with no gap, so a consumer that drops a
  block can tell.
- `overflowed` is the device's overrun flag, or a level of 32 entries or more (the FIFO may
  have filled between the two reads), or a FIFO that filled during the drain by the host's
  clock. Every pop clears the flag, so a host stall between two pops leaves no trace on the
  device: after each pop the adapter checks whether the entries not popped before it and the
  samples produced since the level was read exceed the 33 the device keeps. In stream mode
  such a stall overwrites entries not popped yet, so the gap falls inside the block.
- `lost` is the host's estimate of the samples lost: with the flag or a full level, those
  produced since the previous drain that the block does not hold (the time between drains
  divided by the interval, less the entries), plus the excess found during the drain; 0
  unless `overflowed` is set. Samples counted during a drain are not counted again by the
  next block. The device counts nothing it drops, so it is an estimate, off by the samples
  that fit in the bus latency.

A single reading (`read_sample`) is a scalar observation, so `AccelReading::measurements`
returns `acceleration_x`, `acceleration_y` and `acceleration_z` in `[g]` (`F64`, exact,
quality `Good`, the count in `raw`).

A read that fails or is dropped ends the stream: the next read is refused with
`Disconnected` until `init` runs again, because a failed drain may have popped samples and a
device that lost power is back at its defaults. `init` is the recovery; the next block starts
at sequence 0.

## No Linux I2C backend yet
A Linux `i2c-dev` node needs an `ioctl` to address a device, which needs `unsafe` code or a
new dependency, and neither is in this slice (ADR-0029). An application supplies its own
`I2cBus`; the runbook `docs/hardware/adxl345-i2c.md` describes one for the bench.
