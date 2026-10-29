# Hardware runbook HW2: AutomationDirect SOLO Modular SLM1-C over RS-485

> This is a runbook, not a hardware record. Nothing in it has been run on a device, and no
> device is owned yet (docs/POST_ALPHA_PLAN.md, decision E4 is deferred). The run also needs
> Modbus RTU (slice S59, shipped) and, for the controller's factory settings, Modbus ASCII
> (slice S60, shipped). **Operating the hardware needs the maintainer's separate approval**
> (STATUS.md); writing this file does not give it. When the run is approved and done, copy the
> record table below to `docs/hardware/slm1-c-<yyyy-mm-dd>.md` (the format of `TEMPLATE.md`)
> and fill every field.

## Support level evidenced

Today: **simulator-tested against a documented map, not hardware-verified.** The register
map is a hand transcription of the vendor's user manual (slice S52d), replayed through
`ModbusRtuDriver` and the first-party responder with RTU framing and CRC-16 over the fake
transport, on the line the unit has once it speaks RTU: 9600 baud, 8 data bits, even parity,
one stop bit, network address 1 (slice S52e; the registers each setting comes from are in the
fixture `fixtures/modbus/devices/slm1-c.toml`). That shows what Stonecrab does with the
manual's tables and limits and that its RTU framing reaches them. It shows nothing about the
device or the electrical line, and every behavior the manual leaves open (the section "What
the documents leave open") is an assumption of the replay. `hardware-verified` for RTU needs
both the real electrical behavior and the virtual framing (docs/SPEC.md, "Hardware
evidence"); the virtual half is S58 to S60. This file is the electrical half. Sensor tests do
not establish calibration accuracy.

## Evidence behind this runbook

| Item | Where |
| --- | --- |
| Vendor document | SOLO Modular Temperature Controller User Manual, 1st Edition 07/2023, <https://cdn.automationdirect.com/static/manuals/soloslm/soloslmman.pdf>, sha256 `830b99162dfbab23ed87c1778428747e65ce658dbce48e76899d4b948295d7cc`, retrieved 2026-10-28. Vendor documentation, all rights reserved: pinned, never copied |
| Documented map | `fixtures/modbus/devices/slm1-c.toml`; provenance in `fixtures/modbus/devices/PROVENANCE.toml` |
| Device profile | `profiles/documented/modbus-rtu/solo_slm1_c.toml` (`validation = "documented"`, scheme `modbus-rtu`, schema version 2 for the value rules of the error codes, the serial line and `single-coil-write=fc05` in its `[connection.options]`) |
| Replay suite | `crates/stonecrab/tests/modbus_device_replays.rs`, module `slm1_c`; parity rows `MBS-DEVICE-SLM1C-*` in `docs/parity/modbus-serial.md` |
| Plan | `docs/POST_ALPHA_PLAN.md`, section "Test hardware" (the SLM1-C and the Waveshare adapter) and the row `MBS-HW-RS485` |

The firmware is not pinned by this repository. Register 102FH reports it (the manual's
example: V1.00 is 0x100); the record gives the raw word and, if the unit shows a version by
another means, that text.

## Setup

1. **Device and supply.** One SLM1-C, the main unit (the manual's model code "1" carries the
   power and communication terminals), on a 24 VDC supply inside the manual's range, with an
   ungrounded type K thermocouple on its input (the manual allows only ungrounded
   thermocouples). Leave the DC current output unconnected. Wiring is the manual's chapter 2;
   this runbook copies none of it.
2. **Adapter.** The plan names a Waveshare USB TO RS485 (B). Record the adapter, the cable, the
   termination and the polarity used. The adapter's device node is the serial path in the
   connection string.
3. **Factory settings.** The unit ships set to Modbus ASCII, 7 data bits, even parity, 1 stop
   bit, 9600 baud, network address 1 (registers 1071H to 1076H: 1, 0, 2, 1, 1, 1). A thermocouple
   needs the input type (1004H, default 12 for Pt100) written to 0 for type K. To reach RTU,
   write 1072H to 1 (protocol) and 1074H to 0 (8 data bits). The first of those writes has to
   travel in ASCII with S60, or the maintainer sets the unit with the manufacturer's own
   configuration software, which is not part of Stonecrab. Whether and when the new settings
   take effect is open item 7. The profile reads PV as 0.1 degree, which holds for
   temperature inputs only (a linear input reads in 1 EU), so use a thermocouple.
4. **Host.** Linux x86_64 or aarch64, a Stonecrab build at a recorded commit that includes S59
   (and S60 for the ASCII steps), nothing else on the serial port. The record names the OS, the
   architecture, the library commit and the toolchain.
5. **Connection.** The string the replay and the profile use, with the adapter's device node
   as the path:
   `modbus-rtu:///dev/ttyUSB0?serial.baud-rate=9600&serial.data-bits=8&serial.parity=even&serial.stop-bits=1&address-convention=zero-based&default-unit-identifier=1&max-registers-per-request=8`
   (RTU 8E1 after step B2; before it the unit speaks ASCII and the string is `modbus-ascii`
   with `serial.data-bits=7`). The profile's addresses are `zero-based`: the manual's
   hexadecimal numbers are wire addresses, and its appendix prints the one-based Modbus
   decimal beside them (1040H is 44161). Add `access=read-write` only for steps marked
   "write", and `single-coil-write=fc05` to those that write a bit register: the manual
   lists only function 05 for a bit, and without the parameter Stonecrab writes a coil with
   function 15, which the replay assumes the unit refuses (exception 01).
6. **Approval.** Obtain the maintainer's approval for the run in writing before step B1, and
   note its date in the record.

## Acceptance steps

Each step names the replay test it repeats. The replay tests pass today against the
transcription; a step passes on hardware when the device gives the same answer, and the
difference is the finding when it does not. Every value the run observes goes in the record,
passing or not.

| Step | What runs | Pass criterion, and what to record |
| --- | --- | --- |
| B1 factory read | In ASCII (S60) or after the setup above: read 102FH, 1004H, 1071H to 1076H | The raw words next to the manual's defaults (open items 5 and 13). No pass or fail |
| B2 switch to RTU | Write 1072H to 1 and 1074H to 0, then reconnect in RTU 8E1 | Both writes confirmed or, if the device changes framing before it answers, `Unknown` and a read in the new framing shows the state (open item 7). Record which |
| B3 every register | `every_documented_register_and_coil_reads_back`, read-only: every documented register and the bit registers, eight registers a request | Each answers; the defaults match the manual's where it states one; record any exception and its code (open item 1) and what the registers the manual does not list return (open item 8) |
| B4 input type | `the_documented_write_targets_take_a_write_and_read_back`, write: 1004H to 0, read back, then PV | The read-back is 0; PV settles to a plausible temperature within a few scans (the manual states 400 ms a scan for a thermocouple). Leave type K for the next steps |
| B5 negative numbers | Read 1003H (default -200) and 1001H; write 1001H to -500 and read back; restore 0 | The raw word of -200 (0xFF38 if signed, open item 4) and of -500 (0xFE0C). The profile's PV is signed 16 bit by this assumption |
| B6 PV against a reference | Read 1000H with the probe at an ice point and at room temperature, with an independent thermometer on the same junction | 0.1 degree steps; agreement within a few degrees (the manual states +-0.3 percent of full scale); no calibration claim |
| B7 error codes | `a_process_variable_error_code_is_never_a_good_reading`: unplug the probe (8003H expected); read within the first scan after power-on (8002H expected); set a mismatched input type with the probe fitted (8004H expected); never force 8006H or 8007H | The raw word each time, signed as the register is, and the profile's quality and reason (`Bad` with the manual's meaning for 8002H, 8003H, 8004H, 8006H and 8007H, as schema version 2 value rules on the signed 16 bit codes; never `Good`). Record whether 8005H ever appears (open item 4); the profile has no rule for it and would show it `Uncertain` by the range |
| B8 units | `a_fahrenheit_display_makes_the_process_value_bad`, write: status bits 102AH read bit 2 for degrees C; set the display to degrees F (bit register 0811H, or the controller's own menu) and read | PV and the status bits agree; the profile marks PV `Bad` while the display is not degrees C. Restore degrees C |
| B9 read limit | `the_documented_request_limit_cuts_reads_and_refuses_larger_requests`, read half: eight registers in one request, then nine | Eight served; nine refused, ignored, truncated or answered. Record which (open item 2) and the code (open item 1) |
| B10 write limit | The write half: function 16 with eight then nine registers. **Run only with the maintainer's agreement on the target.** The ramp/soak SV registers 2000H to 2008H hold data only and default to 0; confirm the control method is not ramp/soak first, and restore them | Eight accepted, nine refused or not. If the nine land anywhere, say where |
| B11 unlisted functions and bit writes | `functions_04_and_15_are_refused_because_the_manual_does_not_list_them`, `single_coil_write_fc05_writes_the_bit_registers_the_manual_lists`, `every_documented_bit_register_takes_a_function_05_write_and_reads_back`: function 04 on 1000H; function 15 on a bit register (a coil write without `single-coil-write`); function 05 with `single-coil-write=fc05` | The manual lists 01, 02, 03, 05, 06 and 16 only. Record the reply to 04 and 15 (open item 1) and whether 05 works: the write is confirmed when the unit echoes the address and the state, and the replay assumes it does. For 05, write the value the bit already holds (a bit that reads 1 gets `true`): none of the bits in 0811H to 0816H is harmless to toggle. Writing and restoring every bit is for the maintainer to approve per bit |
| B12 bit registers | Read 0811H to 0816H with function 01 and with 02 | Both answer the same; the five addresses are bit registers at all (open item 6) |
| B13 serial line | RTU timing on the real line: a request to a wrong network address, a request cut short, a second request before the first answered, the line after a timeout, the adapter's turnaround | No answer for a wrong address; a typed timeout inside the deadline; no late reply taken for a later request. Record the baud rate, the inter-frame gap and the reply time |
| B14 ASCII | The same PV read in ASCII at 7E1 (S60) | The same value in both framings |
| B15 expansion | With an SLM2 extension unit fitted, read the main unit, then try to reach the extension unit | How, if at all, an extension unit is addressed (open item 3). Skip when no SLM2 is owned |

## What the documents leave open

The manual states none of these. The replay assumes the value in the right column; the run
settles each, and the fixture, the profile and the parity rows change to match.

| No | Open item | Replay's assumption | Settled by |
| --- | --- | --- | --- |
| 1 | The exception codes for an oversize read or write, an unlisted function (04, 15), an unmapped address and a write to a read-only register | 03 for an oversize request, 01 for an unlisted function; the rest not modeled | B3, B9, B10, B11 |
| 2 | What a request over eight registers does (refusal, truncation, silence) | A refusal with exception 03 | B9, B10 |
| 3 | How an SLM2 extension unit is reached, and whether its registers share the main unit's map | Not modeled: one map | B15 |
| 4 | Whether a negative PV and a negative set point are signed 16 bit integers, and whether 8005H exists | Signed (the profile's value rules are the signed 16 bit codes -32766 to -32764, -32762 and -32761), and 8005H absent (the manual lists 8002H to 8004H, 8006H and 8007H) | B5, B7 |
| 5 | The exact bytes of the firmware word and of the defaults after a factory reset | The manual's example 0x100 | B1 |
| 6 | Whether 0811H to 0816H are bit registers, and what function 05 answers | Coils; function 05 is served and echoes the request (FF00H for on, 0000H for off); another state is refused with exception 03 | B11, B12 |
| 7 | When a change of 1072H to 1076H takes effect (at once, after the answer, after a power cycle), and in which framing the answer travels | After the answer | B2 |
| 8 | What a read of an address the manual does not list returns (for example 1017H and 1018H) | Zero | B3 |
| 9 | Whether the hexadecimal numbers are wire addresses | Yes; the appendix's decimals are 40001 plus the hex | B3 |
| 10 | The reply to broadcast address 0 and to a network address outside 1 to 247 | Not modeled | B13 |
| 11 | The minimum gap between frames and the reply time | Not modeled (the replay waits the driver's t3.5 of an 8E1 line and the device answers at once on the fake clock) | B13 |
| 12 | Whether function 04 is served although unlisted | Refused | B11 |
| 13 | The power-on value of PV, and the state of the status registers 102AH and 102EH at rest | PV 25.0 and degrees C | B1, B7 |

## Record (to be filled by the approved run)

| Field | Value |
| --- | --- |
| Device model | SOLO Modular SLM1-C |
| Firmware | |
| Adapter | |
| Wiring / interface configuration | |
| Host OS / architecture | |
| Library commit | |
| Operation set | read (function 01 to 03), write (05 with `single-coil-write=fc05`, 06, 16), RTU and ASCII, per the steps above |
| Load | |
| Result | |

## Notes

Observations, deviations and links to logs. Never include credentials or private device
data. When a step moved an open item, say which fixture row and which test the change
touches. The electrical layer and the virtual framing are separate claims: say which of the
two a result supports.
