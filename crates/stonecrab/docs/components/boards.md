# Stonecrab Boards
Direct board adapters for Stonecrab (`stonecrab::boards`, behind the `boards` feature, off
by default): a sensor chip on a bus of the host, read without a gateway or a protocol
driver. The first adapter is the MAX31855 thermocouple converter on SPI: the thermocouple
temperature, the internal cold-junction temperature and the converter's open and short
fault flags. A reading converts to the measurements of `stonecrab::sensors` (the same
envelope, units and quality model as a sensor profile), so a board and a Modbus device
look alike to the application. Board adapters are Stonecrab extensions, not upstream
PLC4X parity items, and they change no industrial parity status (ADR-0026).

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
| Hardware verification | planned |

`simulator-tested` here means the decoder, the adapter and the backend ran against the
testkit's `FakeSpiBus` (scripted frames, bus errors and silence on a fake clock) and
against in-memory readers; no converter, no SPI controller and no Linux host took part.
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
