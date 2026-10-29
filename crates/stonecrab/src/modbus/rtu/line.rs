//! The parameters of a `modbus-rtu` or `modbus-ascii` connection string and the silence the
//! line needs before a request.

use std::time::Duration;

use crate::transport::{DataBits, FlowControl, Parity, SerialFactory, StopBits};
use crate::{AccessPolicy, DriverOptions, Result};

use crate::modbus::options::{KEY_ECHO_LIMIT, invalid, number};
use crate::modbus::session::Wire;
use crate::modbus::{AddressConvention, ModbusOptions};

/// The prefix of the serial line parameters (upstream reads transport options under the
/// transport code).
const PREFIX: &str = "serial.";
const BAUD_RATE: &str = "baud-rate";
const DATA_BITS: &str = "data-bits";
const STOP_BITS: &str = "stop-bits";
const PARITY: &str = "parity";
const FLOW_CONTROL: &str = "flow-control";
const INTERFRAME_DELAY: &str = "interframe-delay";
const REUSE_PORT: &str = "reuse-port";

/// The unit address a Modbus serial master broadcasts to.
const BROADCAST: u8 = 0;

/// Bits of one character besides the data bits, parity and stop bits: the start bit.
const START_BIT: u64 = 1;

/// The fixed t3.5 above 19200 baud (Modbus over Serial Line V1.02, section 2.5.1.1).
const FAST_LINE_SILENCE: Duration = Duration::from_micros(1750);

/// The baud rate above which the fixed silence applies.
const FAST_LINE_BAUD: u32 = 19_200;

/// Nanoseconds per second, for character times.
const NANOS: u64 = 1_000_000_000;

/// What a `modbus-rtu` or `modbus-ascii` connection string says: the Modbus parameters shared
/// with `modbus-tcp`, the serial line settings and the inter-frame delay. It travels in the
/// configuration's driver options slot.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct SerialOptions {
    /// The six upstream parameters, as for Modbus TCP.
    pub(super) modbus: ModbusOptions,
    /// Baud rate, data bits, parity, stop bits and flow control of the serial carrier; the data
    /// bits are eight for RTU and seven or eight for ASCII.
    pub(super) line: SerialFactory,
    /// The least silence before a request (`serial.interframe-delay`).
    pub(super) interframe: Duration,
}

impl DriverOptions for SerialOptions {}

/// What the parameters of a connection string decide, besides the options themselves.
pub(super) type Parsed = (SerialOptions, Option<AddressConvention>, AccessPolicy);

impl SerialOptions {
    /// Read `parameters` of a connection string of the driver speaking `wire`: the `serial.`
    /// keys here (only when the carrier is a serial line), every other key through
    /// [`ModbusOptions`], which refuses unknown ones.
    pub(super) fn from_parameters(
        parameters: &[(String, String)],
        serial: bool,
        wire: Wire,
    ) -> Result<Parsed> {
        let (line_keys, shared): (Vec<_>, Vec<_>) = parameters
            .iter()
            .cloned()
            .partition(|(key, _)| key.starts_with(PREFIX));
        let (modbus, convention, access) =
            ModbusOptions::from_parameters(ModbusOptions::default(), &shared)?;
        if modbus.unit_id.get() == BROADCAST {
            return Err(invalid(
                "default-unit-identifier must be 1 to 255: unit 0 is the broadcast address of a \
                 serial line, which no server answers"
                    .to_owned(),
            ));
        }
        let mut options = Self {
            modbus,
            ..Self::default()
        };
        for (key, value) in &line_keys {
            if !serial {
                return Err(invalid(format!(
                    "{key} is a serial line parameter; the tcp carrier takes none"
                )));
            }
            options.apply(key.strip_prefix(PREFIX).unwrap_or(key), value, wire)?;
        }
        Ok((options, convention, access))
    }

    /// Apply one serial line parameter, named without its prefix.
    fn apply(&mut self, name: &str, value: &str, wire: Wire) -> Result<()> {
        let normalized = value.to_ascii_lowercase().replace('_', "-");
        match (name, normalized.as_str()) {
            (BAUD_RATE, _) => {
                self.line.baud_rate = number(BAUD_RATE, value)
                    .ok()
                    .filter(|baud| *baud > 0)
                    .ok_or_else(|| {
                        invalid(format!(
                            "{PREFIX}{BAUD_RATE} must be a whole number above 0"
                        ))
                    })?;
            }
            (DATA_BITS, "8") => self.line.data_bits = DataBits::Eight,
            (DATA_BITS, "7") if wire == Wire::Ascii => self.line.data_bits = DataBits::Seven,
            (REUSE_PORT, "false") => {}
            (DATA_BITS, _) => {
                return Err(invalid(if wire == Wire::Ascii {
                    format!(
                        "{PREFIX}{DATA_BITS} must be 7 or 8: an ASCII character is seven or \
                         eight data bits"
                    )
                } else {
                    format!("{PREFIX}{DATA_BITS} must be 8: every RTU byte is eight data bits")
                }));
            }
            (STOP_BITS, "1") => self.line.stop_bits = StopBits::One,
            (STOP_BITS, "2") => self.line.stop_bits = StopBits::Two,
            (PARITY, "none") => self.line.parity = Parity::None,
            (PARITY, "odd") => self.line.parity = Parity::Odd,
            (PARITY, "even") => self.line.parity = Parity::Even,
            (FLOW_CONTROL, "none") => self.line.flow_control = FlowControl::None,
            (FLOW_CONTROL, "rts-cts" | "rtscts") => self.line.flow_control = FlowControl::RtsCts,
            (FLOW_CONTROL, "xon-xoff" | "xonxoff") => {
                self.line.flow_control = FlowControl::XonXoff;
            }
            (INTERFRAME_DELAY, _) => {
                self.interframe = Duration::from_millis(number(INTERFRAME_DELAY, value)?);
            }
            (REUSE_PORT, "true") => {
                return Err(invalid(format!(
                    "{PREFIX}{REUSE_PORT}=true is not supported: a serial port is not shared \
                     between connections"
                )));
            }
            (STOP_BITS | PARITY | FLOW_CONTROL | REUSE_PORT, _) => {
                return Err(invalid(format!(
                    "{PREFIX}{name} does not take this value; {PREFIX}{STOP_BITS} takes 1 or \
                     2, {PREFIX}{PARITY} none, odd or even, {PREFIX}{FLOW_CONTROL} none, \
                     rts-cts or xon-xoff, {PREFIX}{REUSE_PORT} false"
                )));
            }
            _ => {
                let shown: String = name.chars().take(KEY_ECHO_LIMIT).collect();
                return Err(invalid(format!(
                    "unknown serial line parameter '{PREFIX}{shown}'; the serial line \
                     parameters are {PREFIX}{BAUD_RATE}, {PREFIX}{DATA_BITS}, {PREFIX}{STOP_BITS}, \
                     {PREFIX}{PARITY}, {PREFIX}{FLOW_CONTROL}, {PREFIX}{INTERFRAME_DELAY} \
                     and {PREFIX}{REUSE_PORT}"
                )));
            }
        }
        Ok(())
    }

    /// The silence a request needs before it goes out on the line of the driver speaking
    /// `wire`: for RTU the line's t3.5 and the inter-frame delay, for ASCII the inter-frame
    /// delay alone (an ASCII frame is bounded by its colon and CR LF, not by silence). The
    /// drain after an unanswered request adds the time of the largest frame of the framing.
    /// A gateway's line is not known (the `tcp` carrier takes no `serial.` key, so the line
    /// holds upstream's 9600 8N1): its gap is the inter-frame delay alone, and its drain is
    /// that of 9600 8N1, so a late reply still unread in the socket is read and dropped
    /// before the next request.
    pub(super) fn silence(&self, serial: bool, wire: Wire) -> Silence {
        // The parameters allow seven or eight data bits and nothing else.
        let data_bits = if self.line.data_bits == DataBits::Seven {
            7
        } else {
            8
        };
        let bits = START_BIT
            .saturating_add(data_bits)
            .saturating_add(u64::from(self.line.parity != Parity::None))
            .saturating_add(match self.line.stop_bits {
                StopBits::One => 1,
                StopBits::Two => 2,
            });
        let character = bits
            .saturating_mul(NANOS)
            .checked_div(u64::from(self.line.baud_rate))
            .unwrap_or(NANOS);
        let t35 = if wire == Wire::Ascii {
            Duration::ZERO
        } else if self.line.baud_rate > FAST_LINE_BAUD {
            FAST_LINE_SILENCE
        } else {
            Duration::from_nanos(character.saturating_mul(35).checked_div(10).unwrap_or(0))
        };
        let gap = t35.max(self.interframe);
        let largest_frame = u64::try_from(wire.max_frame_bytes()).unwrap_or(u64::MAX);
        let drain = gap.saturating_add(Duration::from_nanos(
            character.saturating_mul(largest_frame),
        ));
        Silence {
            gap: if serial { gap } else { self.interframe },
            drain,
        }
    }
}

/// How long the line must have been quiet, watched, before a request goes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Silence {
    /// After a request that was answered: for RTU t3.5 (3.5 character times, 1.75 ms above
    /// 19200 baud) or the inter-frame delay, whichever is longer; for ASCII, and for a
    /// gateway, the inter-frame delay alone.
    pub(super) gap: Duration,
    /// After a request that nothing answered: the gap plus the time the largest frame takes
    /// (256 bytes for RTU, 513 for ASCII), so a reply that was late but already on its way
    /// is heard out and dropped.
    pub(super) drain: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(text: &[(&str, &str)]) -> Vec<(String, String)> {
        text.iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn line(text: &[(&str, &str)]) -> SerialOptions {
        SerialOptions::from_parameters(&pairs(text), true, Wire::Rtu)
            .expect("valid parameters")
            .0
    }

    // covers: MBS-CFG-SERIAL-BAUD / serial_baud_rate_default_is_9600
    // covers: MBS-CFG-SERIAL-DATA-BITS / serial_data_bits_default_is_8
    // covers: MBS-CFG-SERIAL-STOP-BITS / serial_stop_bits_default_is_1
    // covers: MBS-CFG-SERIAL-PARITY / serial_parity_default_is_none
    // covers: MBS-CFG-SERIAL-FLOW / serial_flow_control_default_is_none
    // covers: MBS-CFG-SERIAL-INTERFRAME / serial_interframe_delay_default_is_0
    // covers: MBS-CFG-REQUEST-TIMEOUT / serial_request_timeout_default_is_5000
    // covers: MBS-CFG-MAX-COILS / serial_max_coils_default_is_2000
    // covers: MBS-CFG-MAX-REGISTERS / serial_max_registers_default_is_125
    #[test]
    fn the_defaults_are_upstreams() {
        let (options, convention, access) =
            SerialOptions::from_parameters(&[], true, Wire::Rtu).expect("no parameters");
        assert_eq!(options.line, SerialFactory::default());
        assert_eq!(options.line.baud_rate, 9600);
        assert_eq!(
            (options.line.data_bits, options.line.stop_bits),
            (DataBits::Eight, StopBits::One)
        );
        assert_eq!(
            (options.line.parity, options.line.flow_control),
            (Parity::None, FlowControl::None)
        );
        assert_eq!(options.interframe, Duration::ZERO);
        assert_eq!(options.modbus, ModbusOptions::default());
        assert_eq!(options.modbus.request_timeout, Duration::from_millis(5000));
        assert_eq!(
            options.modbus.unit_id.get(),
            1,
            "serial_default_unit_id_is_1"
        );
        assert_eq!((convention, access), (None, AccessPolicy::ReadOnly));
    }

    // covers: MBS-CFG-SERIAL-PARITY / serial_parity_names_are_case_insensitive
    // covers: MBS-CFG-SERIAL-FLOW / serial_flow_control_aliases_and_case_are_accepted
    #[test]
    fn each_serial_parameter_reaches_the_line() {
        let options = line(&[
            ("serial.baud-rate", "115200"),
            ("serial.stop-bits", "2"),
            ("serial.parity", "Even"),
            ("serial.flow-control", "RTS_CTS"),
            ("serial.interframe-delay", "7"),
        ]);
        assert_eq!(options.line.baud_rate, 115_200);
        assert_eq!(options.line.stop_bits, StopBits::Two);
        assert_eq!(options.line.parity, Parity::Even);
        assert_eq!(options.line.flow_control, FlowControl::RtsCts);
        assert_eq!(options.interframe, Duration::from_millis(7));
        for (value, flow) in [
            ("xonxoff", FlowControl::XonXoff),
            ("XON_XOFF", FlowControl::XonXoff),
            ("rtscts", FlowControl::RtsCts),
            ("none", FlowControl::None),
        ] {
            assert_eq!(
                line(&[("serial.flow-control", value)]).line.flow_control,
                flow,
                "{value}"
            );
        }
        for (value, parity) in [("ODD", Parity::Odd), ("none", Parity::None)] {
            assert_eq!(
                line(&[("serial.parity", value)]).line.parity,
                parity,
                "{value}"
            );
        }
        assert_eq!(
            line(&[("serial.stop-bits", "1")]).line.stop_bits,
            StopBits::One
        );
    }

    // covers: MBS-CFG-SERIAL-INTERFRAME / t3.5 from the line settings
    #[test]
    fn the_gap_is_three_and_a_half_characters_up_to_19200_baud() {
        // 9600 8N1 (no parity, one stop bit, ten bits): 3.5 * 10 / 9600 s.
        let plain = line(&[]).silence(true, Wire::Rtu);
        assert_eq!(plain.gap, Duration::from_nanos(3_645_831));
        assert_eq!(
            plain.drain,
            Duration::from_nanos(270_312_327),
            "the drain adds the 256 characters of the largest frame"
        );
        // 9600 8E1: eleven bits.
        let even = line(&[("serial.parity", "even")]).silence(true, Wire::Rtu);
        assert_eq!(even.gap, Duration::from_nanos(4_010_415));
        // 19200 8N2 is still timed: eleven bits at 19200.
        let two = line(&[("serial.baud-rate", "19200"), ("serial.stop-bits", "2")])
            .silence(true, Wire::Rtu);
        assert_eq!(two.gap, Duration::from_nanos(2_005_206));
        assert_eq!(two.drain, Duration::from_nanos(148_671_702));
    }

    // covers: MBS-CFG-SERIAL-INTERFRAME / 1.75 ms above 19200 baud, interframe-delay as a floor
    #[test]
    fn the_gap_is_fixed_above_19200_baud_and_never_below_the_interframe_delay() {
        let fast = line(&[("serial.baud-rate", "115200")]).silence(true, Wire::Rtu);
        assert_eq!(fast.gap, Duration::from_micros(1750));
        let delayed = line(&[
            ("serial.baud-rate", "115200"),
            ("serial.interframe-delay", "20"),
        ])
        .silence(true, Wire::Rtu);
        assert_eq!(delayed.gap, Duration::from_millis(20));
        let gateway = line(&[]).silence(false, Wire::Rtu);
        assert_eq!(
            (gateway.gap, gateway.drain),
            (Duration::ZERO, Duration::from_nanos(270_312_327)),
            "a gateway waits no gap, and drains as a 9600 8N1 line after an unanswered request"
        );
    }

    fn ascii_line(text: &[(&str, &str)]) -> SerialOptions {
        SerialOptions::from_parameters(&pairs(text), true, Wire::Ascii)
            .expect("valid parameters")
            .0
    }

    // covers: MBS-CFG-SERIAL-DATA-BITS / ascii_accepts_7_data_bits_with_even_parity
    #[test]
    fn an_ascii_line_takes_7_or_8_data_bits_and_an_rtu_line_only_8() {
        assert_eq!(ascii_line(&[]).line.data_bits, DataBits::Eight);
        assert_eq!(
            ascii_line(&[("serial.data-bits", "8")]).line.data_bits,
            DataBits::Eight
        );
        let seven = ascii_line(&[("serial.data-bits", "7"), ("serial.parity", "even")]);
        assert_eq!(seven.line.data_bits, DataBits::Seven);
        assert_eq!(seven.line.parity, Parity::Even);
        for bits in ["5", "6", "9", "x"] {
            let message = SerialOptions::from_parameters(
                &pairs(&[("serial.data-bits", bits)]),
                true,
                Wire::Ascii,
            )
            .expect_err(bits)
            .to_string();
            assert!(message.contains("7 or 8"), "{bits}: {message}");
        }
        let message =
            SerialOptions::from_parameters(&pairs(&[("serial.data-bits", "7")]), true, Wire::Rtu)
                .expect_err("RTU has eight data bits")
                .to_string();
        assert!(message.contains("must be 8"), "{message}");
    }

    // covers: MBS-ASCII-TIMING / ascii_has_no_silence_between_frames
    #[test]
    fn an_ascii_line_has_no_gap_of_its_own_and_drains_the_time_of_513_characters() {
        // 9600 8N1: ten bits, 1 041 666 ns a character; no t3.5 for ASCII.
        let plain = ascii_line(&[]).silence(true, Wire::Ascii);
        assert_eq!(plain.gap, Duration::ZERO);
        assert_eq!(plain.drain, Duration::from_nanos(534_374_658));
        // 7 data bits, even parity, one stop bit is ten bits as well; 7N1 is nine.
        let seven_even = ascii_line(&[("serial.data-bits", "7"), ("serial.parity", "even")]);
        assert_eq!(seven_even.silence(true, Wire::Ascii), plain);
        let seven_none = ascii_line(&[("serial.data-bits", "7")]);
        assert_eq!(
            seven_none.silence(true, Wire::Ascii).drain,
            Duration::from_nanos(513 * 937_500)
        );
        // serial.interframe-delay is the whole gap, however fast the line.
        let delayed = ascii_line(&[
            ("serial.baud-rate", "115200"),
            ("serial.interframe-delay", "20"),
        ])
        .silence(true, Wire::Ascii);
        assert_eq!(delayed.gap, Duration::from_millis(20));
        let fast = ascii_line(&[("serial.baud-rate", "115200")]).silence(true, Wire::Ascii);
        assert_eq!(fast.gap, Duration::ZERO, "no 1.75 ms above 19200 baud");
        // A gateway is a 9600 8N1 line of unknown speed.
        let gateway = ascii_line(&[]).silence(false, Wire::Ascii);
        assert_eq!((gateway.gap, gateway.drain), (Duration::ZERO, plain.drain));
    }
}
