//! The first-party interop register map (`fixtures/modbus/interop/register-map.toml`).
//!
//! One file feeds both the S27 pymodbus server generator and the S25 contract fixture, so
//! the values a test expects and the values the server serves cannot drift apart.

use std::path::Path;

use serde::Deserialize;

use crate::golden::GoldenError;

/// Highest quantity a register-map block can be sized to; sizes above it are rejected.
const MAX_BLOCK: usize = 65_536;

/// The interop register map: block sizes and the values at their logical addresses.
///
/// Addresses are the logical tag addresses of the upstream tags; the wire address is one
/// less. Register words are 16-bit values in register order; a multi-register value is
/// stored the way the entry's `byte_order` says (high word first and high byte first
/// unless it says otherwise). Positions that no entry names hold zero or false.
///
/// # Examples
/// ```
/// use std::path::Path;
/// use stonecrab_testkit::InteropMap;
///
/// let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus/interop/register-map.toml");
/// let map = InteropMap::load(&path)?;
/// assert_eq!(map.sizes.holding_registers, 201);
/// assert_eq!(map.holding_extension[0].words, [0xDC0F, 0x4940]);
/// # Ok::<(), stonecrab_testkit::GoldenError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteropMap {
    /// The block sizes of the upstream server.
    pub sizes: BlockSizes,
    /// Holding registers inside the upstream block (logical addresses 1 to 31).
    pub holding_registers: Vec<RegisterEntry>,
    /// Stonecrab-only holding registers outside the upstream block (the little-endian
    /// region, `MB-INTEROP-LE`); a server must size its holding block to cover them.
    #[serde(default)]
    pub holding_extension: Vec<RegisterEntry>,
    /// The coil run (`MB-INTEROP-COIL`).
    pub coils: BitRun,
    /// The discrete-input run (`MB-INTEROP-DI`).
    pub discrete_inputs: BitRun,
    /// Input registers (`MB-INTEROP-IR`).
    pub input_registers: Vec<RegisterEntry>,
}

/// The sizes of the four upstream blocks, in registers or bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockSizes {
    /// Holding registers (201).
    pub holding_registers: usize,
    /// Coils (2000).
    pub coils: usize,
    /// Discrete inputs (2000).
    pub discrete_inputs: usize,
    /// Input registers (21).
    pub input_registers: usize,
}

/// One typed value held in consecutive registers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterEntry {
    /// The `MB-INTEROP-*` row id of docs/parity/modbus.md.
    pub row: String,
    /// The logical address of the first register.
    pub address: u16,
    /// The upstream data type name (`BOOL` through `LREAL`).
    #[serde(rename = "type")]
    pub data_type: String,
    /// The value in words, for readers; the tests check that `words` encode it.
    pub value: String,
    /// The upstream byte order name, absent for the default `BIG_ENDIAN`.
    #[serde(default)]
    pub byte_order: Option<String>,
    /// The register words in register order.
    pub words: Vec<u16>,
}

/// A run of consecutive bits (coils or discrete inputs).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BitRun {
    /// The `MB-INTEROP-*` row id of docs/parity/modbus.md.
    pub row: String,
    /// The logical address of the first bit.
    pub address: u16,
    /// The bits in address order.
    pub values: Vec<bool>,
}

impl InteropMap {
    /// Read and validate the map at `path`.
    ///
    /// # Errors
    /// [`GoldenError::Io`] when the file cannot be read; [`GoldenError::Interop`] (naming
    /// the file) when it is invalid, as for [`from_toml`](Self::from_toml).
    pub fn load(path: &Path) -> Result<Self, GoldenError> {
        Self::from_toml(&GoldenError::read(path)?).map_err(|e| e.in_file(path))
    }

    /// Parse and validate the text of a register map.
    ///
    /// Validation: unknown keys are rejected; every `row` starts with `MB-INTEROP-`;
    /// every `type` is one of the 15 upstream names and `words` has that type's register
    /// count; `byte_order` is absent or one of the four upstream names; addresses are
    /// at least 1; entries of one area do not overlap; entries fit their block (the
    /// holding extension must lie outside the holding block); block sizes are at most
    /// 65536.
    ///
    /// # Errors
    /// [`GoldenError::Interop`] when the text is not valid TOML for the schema or fails
    /// validation. The error carries no file name; [`load`](Self::load) adds it.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::InteropMap;
    ///
    /// assert!(InteropMap::from_toml("surprise = 1").is_err());
    /// ```
    pub fn from_toml(text: &str) -> Result<Self, GoldenError> {
        let map: Self = toml::from_str(text).map_err(|e| GoldenError::Interop {
            path: None,
            message: e.to_string(),
        })?;
        map.validate().map_err(|message| GoldenError::Interop {
            path: None,
            message,
        })?;
        Ok(map)
    }

    fn validate(&self) -> Result<(), String> {
        let sizes = self.sizes;
        for (area, size) in [
            ("holding_registers", sizes.holding_registers),
            ("coils", sizes.coils),
            ("discrete_inputs", sizes.discrete_inputs),
            ("input_registers", sizes.input_registers),
        ] {
            if size == 0 || size > MAX_BLOCK {
                return Err(format!("sizes.{area} = {size} is outside 1..={MAX_BLOCK}"));
            }
        }
        let mut holding = Vec::new();
        check_registers(
            &self.holding_registers,
            sizes.holding_registers,
            &mut holding,
        )?;
        for entry in &self.holding_extension {
            if usize::from(entry.address) < sizes.holding_registers {
                return Err(format!(
                    "{}: holding extension at {} lies inside the {}-register block",
                    entry.row, entry.address, sizes.holding_registers
                ));
            }
        }
        check_registers(&self.holding_extension, MAX_BLOCK, &mut holding)?;
        check_registers(
            &self.input_registers,
            sizes.input_registers,
            &mut Vec::new(),
        )?;
        check_bits(&self.coils, sizes.coils)?;
        check_bits(&self.discrete_inputs, sizes.discrete_inputs)
    }
}

/// The register count of an upstream data type name.
fn register_width(data_type: &str) -> Option<usize> {
    match data_type {
        "BOOL" | "BYTE" | "WORD" | "SINT" | "USINT" | "INT" | "UINT" => Some(1),
        "DWORD" | "DINT" | "UDINT" | "REAL" => Some(2),
        "LWORD" | "LINT" | "ULINT" | "LREAL" => Some(4),
        _ => None,
    }
}

fn check_row(row: &str) -> Result<(), String> {
    if row.starts_with("MB-INTEROP-") {
        Ok(())
    } else {
        Err(format!("row `{row}` is not an MB-INTEROP-* row id"))
    }
}

/// Check `entries` and add their `(first, last)` spans to `spans`, which must stay free of
/// overlap. Spans are logical addresses; `size` means logical addresses `1..=size` are
/// served (the server keeps `size + 1` slots, ADR-0015 decision 16).
fn check_registers(
    entries: &[RegisterEntry],
    size: usize,
    spans: &mut Vec<(usize, usize, String)>,
) -> Result<(), String> {
    for entry in entries {
        check_row(&entry.row)?;
        let width = register_width(&entry.data_type)
            .ok_or_else(|| format!("{}: unknown type `{}`", entry.row, entry.data_type))?;
        if entry.words.len() != width {
            return Err(format!(
                "{}: {} takes {width} register(s) but has {} word(s)",
                entry.row,
                entry.data_type,
                entry.words.len()
            ));
        }
        if let Some(order) = &entry.byte_order
            && !matches!(
                order.as_str(),
                "BIG_ENDIAN" | "LITTLE_ENDIAN" | "BIG_ENDIAN_BYTE_SWAP" | "LITTLE_ENDIAN_BYTE_SWAP"
            )
        {
            return Err(format!("{}: unknown byte order `{order}`", entry.row));
        }
        let first = usize::from(entry.address);
        let last = first.saturating_add(width).saturating_sub(1);
        if first == 0 || last >= size {
            return Err(format!(
                "{}: registers {first}..={last} are outside 1..{size}",
                entry.row
            ));
        }
        if let Some((_, _, other)) = spans.iter().find(|(f, l, _)| first <= *l && *f <= last) {
            return Err(format!("{}: overlaps {other}", entry.row));
        }
        spans.push((first, last, entry.row.clone()));
    }
    Ok(())
}

fn check_bits(run: &BitRun, size: usize) -> Result<(), String> {
    check_row(&run.row)?;
    let first = usize::from(run.address);
    let last = first.saturating_add(run.values.len()).saturating_sub(1);
    if run.values.is_empty() || first == 0 || last >= size {
        return Err(format!(
            "{}: bits {first}..={last} are empty or outside 1..{size}",
            run.row
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn map_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml")
    }

    fn map_text() -> String {
        std::fs::read_to_string(map_path()).expect("register map exists")
    }

    /// The words an entry's `value` encodes to, computed here and not by any decoder.
    fn encode(entry: &RegisterEntry) -> Vec<u16> {
        let big: i128 = match entry.data_type.as_str() {
            "BOOL" => i128::from(entry.value == "true"),
            "REAL" => i128::from(entry.value.parse::<f32>().expect("f32").to_bits()),
            "LREAL" => i128::from(entry.value.parse::<f64>().expect("f64").to_bits()),
            _ => entry.value.parse().expect("integer value"),
        };
        let width = register_width(&entry.data_type).expect("known type");
        let all = big.to_be_bytes();
        let skip = all.len().saturating_sub(width.saturating_mul(2));
        let mut bytes: Vec<u8> = all.iter().skip(skip).copied().collect();
        if entry.byte_order.as_deref() == Some("LITTLE_ENDIAN") {
            bytes.reverse();
        }
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[hi, lo]| u16::from_be_bytes([*hi, *lo]))
            .collect()
    }

    fn parse_error(text: &str) -> String {
        InteropMap::from_toml(text)
            .expect_err("must be rejected")
            .to_string()
    }

    #[test]
    fn register_map_parses_with_the_documented_sizes() {
        let map = InteropMap::load(&map_path()).expect("register map loads");
        assert_eq!(
            map.sizes,
            BlockSizes {
                holding_registers: 201,
                coils: 2000,
                discrete_inputs: 2000,
                input_registers: 21
            },
            "block sizes from the parity prose"
        );
    }

    #[test]
    fn holding_registers_cover_1_to_31_exactly_once() {
        let map = InteropMap::load(&map_path()).expect("loads");
        let mut covered = Vec::new();
        for entry in &map.holding_registers {
            let first = usize::from(entry.address);
            covered.extend(first..first + entry.words.len());
        }
        assert_eq!(
            covered,
            (1..=31).collect::<Vec<_>>(),
            "registers 1 to 31, in order"
        );
    }

    #[test]
    fn every_entry_words_encode_its_stated_value() {
        let map = InteropMap::load(&map_path()).expect("loads");
        let entries = map
            .holding_registers
            .iter()
            .chain(&map.holding_extension)
            .chain(&map.input_registers);
        for entry in entries {
            assert_eq!(
                entry.words,
                encode(entry),
                "{} {} at {}: {} does not encode to the listed words",
                entry.row,
                entry.data_type,
                entry.address,
                entry.value
            );
        }
    }

    #[test]
    fn little_endian_region_is_registers_300_and_301() {
        let map = InteropMap::load(&map_path()).expect("loads");
        let entry = map.holding_extension.first().expect("one extension entry");
        assert_eq!(map.holding_extension.len(), 1, "only the D8 region");
        assert_eq!(
            (entry.row.as_str(), entry.address, entry.data_type.as_str()),
            ("MB-INTEROP-LE", 300, "REAL"),
            "row, address and type"
        );
        assert_eq!(
            entry.words,
            [0xDC0F, 0x4940],
            "REAL 3.141593 under LITTLE_ENDIAN"
        );
        assert_eq!(
            entry.byte_order.as_deref(),
            Some("LITTLE_ENDIAN"),
            "byte order"
        );
    }

    #[test]
    fn bit_runs_and_input_registers_match_the_parity_rows() {
        let map = InteropMap::load(&map_path()).expect("loads");
        let render = |run: &BitRun| {
            run.values
                .iter()
                .map(|b| if *b { 'T' } else { 'F' })
                .collect::<String>()
        };
        assert_eq!(
            (map.coils.row.as_str(), map.coils.address),
            ("MB-INTEROP-COIL", 1),
            "coil row"
        );
        assert_eq!(render(&map.coils), "TFTTFTFFTF", "coils 1 to 10");
        assert_eq!(
            (
                map.discrete_inputs.row.as_str(),
                map.discrete_inputs.address
            ),
            ("MB-INTEROP-DI", 1),
            "discrete-input row"
        );
        assert_eq!(
            render(&map.discrete_inputs),
            "FTFTTFTTFT",
            "discrete inputs 1 to 10"
        );
        let words: Vec<u16> = map
            .input_registers
            .iter()
            .flat_map(|e| e.words.clone())
            .collect();
        assert_eq!(
            words,
            [0x0001, 0xF688, 0xA5B8, 0xF18C, 0x9F48, 0x4049, 0x0FDC],
            "input registers 1 to 7 (REAL starts at 6 after DINT at 4-5)"
        );
        assert!(
            map.input_registers.iter().all(|e| e.row == "MB-INTEROP-IR"),
            "labels"
        );
    }

    #[test]
    fn every_value_is_labeled_with_an_interop_row_id() {
        let map = InteropMap::load(&map_path()).expect("loads");
        let rows = map
            .holding_registers
            .iter()
            .chain(&map.holding_extension)
            .chain(&map.input_registers)
            .map(|e| e.row.as_str());
        assert!(
            rows.chain([map.coils.row.as_str(), map.discrete_inputs.row.as_str()])
                .all(|r| r.starts_with("MB-INTEROP-"))
        );
    }

    #[test]
    fn invalid_maps_are_rejected_with_a_reason() {
        // covers: CODING_STANDARDS 13.7, invalid register maps are rejected
        let text = map_text();
        let cases = [
            (format!("surprise = 1\n{text}"), "unknown field"),
            ("not toml [".to_owned(), "TOML"),
            (
                text.replace("holding_registers = 201", "holding_registers = 0"),
                "sizes.holding_registers",
            ),
            (
                text.replace("input_registers = 21", "input_registers = 70000"),
                "sizes.input_registers",
            ),
            (
                text.replacen("MB-INTEROP-HR-01", "ROW-1", 1),
                "not an MB-INTEROP-* row id",
            ),
            (
                text.replacen("type = \"BOOL\"", "type = \"STRING\"", 1),
                "unknown type",
            ),
            (
                text.replacen("words = [0xFCDE, 0x88B8]", "words = [0xFCDE]", 1),
                "takes 2 register(s)",
            ),
            (
                text.replace("\"LITTLE_ENDIAN\"", "\"MIDDLE_ENDIAN\""),
                "unknown byte order",
            ),
            (
                text.replacen("address = 1\ntype", "address = 0\ntype", 1),
                "outside 1..",
            ),
            (
                text.replacen("address = 3\ntype", "address = 2\ntype", 1),
                "overlaps",
            ),
            (
                text.replacen("address = 300", "address = 200", 1),
                "inside the 201-register block",
            ),
            (
                text.replacen("address = 28\ntype", "address = 200\ntype", 1),
                "outside 1..",
            ),
            (
                text.replacen("address = 300", "address = 65535", 1),
                "outside 1..",
            ),
        ];
        for (mutated, needle) in cases {
            assert!(parse_error(&mutated).contains(needle), "wanted `{needle}`");
        }
    }

    #[test]
    fn invalid_bit_runs_are_rejected() {
        // covers: CODING_STANDARDS 13.7, invalid bit runs are rejected
        let text = map_text();
        let coils = "values = [true, false, true, true, false, true, false, false, true, false]";
        assert!(text.contains(coils), "the test's anchor exists");
        let empty = text.replace(coils, "values = []");
        assert!(
            parse_error(&empty).contains("empty or outside"),
            "empty run"
        );
        let high = text.replacen("address = 1\nvalues", "address = 1995\nvalues", 1);
        assert!(
            parse_error(&high).contains("empty or outside"),
            "past the block"
        );
        let bad_row = text.replace("MB-INTEROP-COIL", "COIL");
        assert!(
            parse_error(&bad_row).contains("not an MB-INTEROP-* row id"),
            "row id"
        );
    }

    #[test]
    fn load_names_the_file_in_errors() {
        let missing =
            InteropMap::load(Path::new("no/such/register-map.toml")).expect_err("missing");
        assert!(
            missing.to_string().contains("register-map.toml"),
            "{missing}"
        );
        let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/clean/notes.toml");
        let invalid = InteropMap::load(&tree).expect_err("a comment-only file lacks the sections");
        assert!(
            matches!(&invalid, GoldenError::Interop { path: Some(p), .. } if p.ends_with("notes.toml")),
            "{invalid}"
        );
    }
}
