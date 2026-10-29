# Hardware runbook HW1: AutomationDirect STRIDE SIO-MB04THMS

> This is a runbook, not a hardware record. Nothing in it has been run on a device, and no
> device is owned yet (docs/POST_ALPHA_PLAN.md, decision E4 is deferred). **Operating the
> hardware needs the maintainer's separate approval** (STATUS.md); writing this file does
> not give it. When the run is approved and done, copy the record table below to
> `docs/hardware/sio-mb04thms-<yyyy-mm-dd>.md` (the format of `TEMPLATE.md`) and fill every
> field.

## Support level evidenced

Today: **simulator-tested against a documented map, not hardware-verified.** The register
map is a hand transcription of the vendor's user guide, replayed through the first-party
responder (slice S52d). That shows what Stonecrab does with the guide's tables. It shows
nothing about the device, and every behavior the guide leaves open (the section "What the
documents leave open") is an assumption of the replay. `hardware-verified` for this device
needs the run below, a record in the format of `TEMPLATE.md`, and a note for every open item
that moved. Sensor tests do not establish calibration accuracy.

## Evidence behind this runbook

| Item | Where |
| --- | --- |
| Vendor document | SIO-MB04THMS User Guide, 1st Edition, Rev. C, <https://cdn.automationdirect.com/static/manuals/stridefio/siomb04thmsug.pdf>, sha256 `0800b63b890638626cc359e6b539d3fe14e315c1749a47f28063fd5ecbc53854`, retrieved 2026-10-28. Vendor documentation, all rights reserved: pinned, never copied |
| Documented map | `fixtures/modbus/devices/sio-mb04thms.toml`; provenance in `fixtures/modbus/devices/PROVENANCE.toml` |
| Device profile | `profiles/documented/modbus-tcp/stride_sio_mb04thms.toml` (`validation = "documented"`) |
| Replay suite | `crates/stonecrab/tests/modbus_device_replays.rs`, module `sio_mb04thms`; parity rows `MB-DEVICE-SIO-*` in `docs/parity/modbus.md` |
| Plan | `docs/POST_ALPHA_PLAN.md`, section "Test hardware" (device, accessories, the open items) |

The firmware is not pinned by this repository: the guide does not carry it. The firmware
version the unit reports on the Software Update page of its web interface goes in the record.
The plan cites Advisory PA-COM-006: firmware before 8.5.4.0 sends the web password back on any
login attempt, and AutomationDirect lists 8.5.4.0 as the current shipping version. Check the
unit's version before it joins any network, and treat a unit that cannot take the update as
the plan describes.

## Setup

1. **Device and supply.** One SIO-MB04THMS on a 24 VDC Class 2 supply, an ungrounded type K
   thermocouple on channel 0 (the plan names the PSL-24-030 and the THMK-T18L06-01), and the
   other channels shorted or fitted with probes the maintainer chooses. Wiring is the
   diagram printed on the module's side; this runbook copies none of it.
2. **Network.** An isolated bench LAN with only the module and the host. The web interface is
   plain HTTP on port 80 and ships with a default login that is public in the guide: change it
   at once, and never write the login, the IP address plan or any other credential into the
   record, the repository or a log that leaves the bench. The factory network values (for a
   unit after the guide's hardware reset) and the INIT-mode address are in the guide, page
   11; they are not repeated here.
3. **Unit id.** The node id register (40007) defaults to 1; use unit 1.
4. **Host.** Linux x86_64 or aarch64, a Stonecrab build at a recorded commit, and nothing else
   on the module's network. The record names the OS, the architecture, the library commit and
   the toolchain.
5. **Connection string.** `modbus-tcp://<module address>?address-convention=zero-based&max-registers-per-request=64&max-coils-per-request=128`,
   with `&access=read-write` added only for the steps marked "write". The port is not in the
   guide (open item 1): omit it to use 502, and record what the module accepts. The
   convention is the replay's assumption (open item 2); if the first step shows the device
   counts from one, change `zero-based` to `one-based-40001` here and in the profile and
   nothing else.
6. **Approval.** Obtain the maintainer's approval for the run in writing before step 1, and
   note its date in the record.

## Acceptance steps

Each step names the replay test it repeats. The replay tests pass today against the
transcription; a step passes on hardware when the device gives the same answer, and the
difference is the finding when it does not. Every value the run observes goes in the record,
passing or not.

| Step | What runs | Pass criterion, and what to record |
| --- | --- | --- |
| A1 identity | Read 40002 and 40003; read the firmware version off the web interface | The two raw words next to the version text: the encoding is open item 5. No pass or fail |
| A2 address base | Read 40041 to 40044 at wire 41 to 44 and again at wire 40 to 43; compare with the web interface's Analog Input page; read coil 161 at wire 161 and at wire 160 | Exactly one reading matches the web interface: that is the convention (open item 2). The guide's hex column (coil 161 is 0x00A1) points to wire 161 |
| A3 every register and coil | `every_documented_register_and_coil_reads_back_through_both_mirrors`: every documented register through function 03 and 04, every coil through 01 and 02 | Each register and coil answers, and the 03 and 04 answers (01 and 02) are equal. Record any exception and its code. The temperatures plausibly match the probe |
| A4 read limits | `the_documented_request_limits_cut_reads_and_refuse_larger_requests`, read half: 64 registers in one request, then 65; 128 coils in one request, then 129 | 64 and 128 are served; 65 and 129 are refused. Record the exception code (open item 3) and what a read of unmapped registers inside the 64 returns (open item 7) |
| A5 write limits | The write half of the same test: function 16 with 64 then 65 registers, function 15 with 64 then 65 coils. **Run only with the maintainer's agreement on the target addresses.** Choose a range the guide lists no register in, never one that holds a documented writable register, and record the web interface's settings before and after | 64 accepted and 65 refused with a code. If the device accepts 65, or changes a setting, restore it from the web interface (the hardware reset to default returns the network settings to their factory values, so use it last) and record it |
| A6 break flag | `a_break_bit_makes_that_channel_bad_and_no_other`: with a profile read in a loop, open channel N's thermocouple loop (unplug the probe; a relay in series, as the plan suggests, if the maintainer has one), then close it again, for each channel in turn | Break Status (40050) sets bit 2N for channel N and no other, and clears when the loop closes; the measurement is `Bad` while set and `Good` after. Record the value the analog input holds during the break (open item 8) and over-temperature behavior if it can be reached safely |
| A7 degree units | `a_degree_unit_other_than_celsius_makes_that_channel_bad`, write: set 41218 to 1 then 2, read channel 0, restore 0 | The reading changes unit with one implied decimal (24.6 C is about 76.3 F and 297.8 K), and the profile marks the channel `Bad` while the unit is not Celsius |
| A8 write targets | `the_documented_write_targets_take_a_write_and_read_back`, write: input type (40031), units (41218), offset (41242), coils 161 to 163, each restored to its documented default | Each write is confirmed and reads back; the offset shifts the reading by its tenths; record the function code the wire used (06 for a single register, 15 for a coil) |
| A9 system flags | Write coil 161, then 162, then 163; read 40011 after each; then write 40011 and read the coils | Which bit of 40011 each coil moves (open item 6). The guide's table says bits 8, 9 and 10 |
| A10 watchdog | Set 40013 to a short time, enable the watchdog (coil 161), stay silent past the time, then read coil 162 and the status LED; reset the event; restore 40013 to 10 | The event coil sets after the silence and clears when written, and any command resets the timer. This is the plan's device-side failure path |
| A11 socket timeout | Set the socket timeout on the web interface, keep the connection idle past it, then read | The device closes the connection and Stonecrab reports `Disconnected`, then the next read reconnects (the replay's `device_side_faults_end_in_typed_outcomes_and_a_lost_confirmation_is_unknown`) |
| A12 unit id | Read with `default-unit-identifier` 2, 0 and 255 | Whether the device answers, stays silent or refuses (open item 4) |
| A13 refusals | A read of an unmapped address past the last register, a write to a read-only register (40041, with the maintainer's agreement), function 05 by a raw frame, and an unlisted function (for example 0x2B) | The exception code of each (open item 3), or silence |
| A14 cross-check | Read channel 0 and a MAX31855 (or any independent thermometer) on the same junction at an ice point and at room temperature; read 40031 to 40034 next to the readings | Agreement within a few degrees, not tenths (the plan's plausibility bound). This does not establish calibration. Every channel read has an input type of a thermocouple (not 0 or 1), because the profile assumes one (open item 13) |

## What the documents leave open

The guide states none of these. The replay assumes the value in the right column; the run
settles each, and the fixture, the profile and the parity rows change to match.

| No | Open item | Replay's assumption | Settled by |
| --- | --- | --- | --- |
| 1 | The Modbus TCP port | 502 | Connect without a port; scan nothing else |
| 2 | Whether the printed number 40041 is wire address 41 or 40 (and coil 00161 wire 161 or 160) | The wire address is the printed number without the leading digit (`zero-based`), from the one hex datum | A2 |
| 3 | The exception codes for an oversize read or write, an unmapped address, a read-only register, an unlisted function and function 05 | 03 (oversize), 02 (unmapped address), 01 (an unlisted function and function 05) | A4, A5, A13 |
| 4 | The reply to a wrong or unknown unit id | Not modeled (the replay answers every unit id) | A12 |
| 5 | The encoding of the firmware registers 40002 and 40003 | Two arbitrary words | A1 |
| 6 | Whether coils 161 to 163 mirror System Flags (40011) bits 8, 9 and 10, in that order | Not linked: the replay serves the register and the coils separately | A9 |
| 7 | What a read of a register the guide does not map returns (40001, 40006, 40008 to 40010 and the rest; 40004 and 40005 are listed as Reserved, read-only, and are deliberately not mapped) | Zero | A4 |
| 8 | The value an analog input holds while its break bit is set, or while the channel is disabled | Not modeled: the quality comes from the flag | A6 |
| 9 | The power-on state of the watchdog enable bit and of the watchdog event bit (coil 162), and the default offsets | Clear, and zero | A1 and A8 |
| 10 | Whether a read-only register refuses a write | Not modeled (the replay stores it) | A13 |
| 11 | The extent of the register and coil spaces | The replay serves up to the last listed address | A13 |
| 12 | The measuring range of each thermocouple type | None: the profile has no range | The record, from the web interface |
| 13 | What a channel set to 250 mV (input type 1) or Disabled (0) reads, since the profile assumes a thermocouple type: mV stores two implied decimals and the profile would show 10 times the millivolts as Cel with quality Good | The profile cannot detect it (schema version 1); channels are left at the default TC K | A14 |

## Record (to be filled by the approved run)

| Field | Value |
| --- | --- |
| Device model | STRIDE SIO-MB04THMS |
| Firmware | |
| Adapter | none: Ethernet |
| Wiring / interface configuration | |
| Host OS / architecture | |
| Library commit | |
| Operation set | read (function 01 to 04), write (06, 15, 16), per the steps above |
| Load | |
| Result | |

## Notes

Observations, deviations and links to logs. Never include credentials, network plans or
private device data. When a step moved an open item, say which fixture row and which test the
change touches.
