//! The golden-vector loader (section 13.6 of the coding standards).
//!
//! A vector directory holds one `PROVENANCE.toml` and, for every vector it lists,
//! `<name>.hex` (one line of hex) and an optional `<name>.expected.json`. The loader
//! enforces the rule "every vector is listed": a `.hex` or `.expected.json` file that the
//! provenance file does not name, or a listed vector without a `.hex`, is an error that
//! names the offending file.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// The provenance file name inside a vector directory.
const PROVENANCE_FILE: &str = "PROVENANCE.toml";

/// Where a golden vector came from.
///
/// The spelling in `PROVENANCE.toml` is the `snake_case` variant name.
///
/// # Examples
/// ```
/// use stonecrab::testkit::{Provenance, VectorSource};
///
/// let provenance = Provenance::parse(
///     "[[vector]]\nname = \"x\"\nsource = \"protocol_spec\"\norigin = \"spec\"\nlicense = \"Apache-2.0\"\n",
/// )?;
/// assert_eq!(provenance.vector[0].source, VectorSource::ProtocolSpec);
/// # Ok::<(), stonecrab::testkit::GoldenError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorSource {
    /// A frame or model taken from an upstream project's test fixtures (`upstream_fixture`).
    UpstreamFixture,
    /// A frame authored from the protocol specification (`protocol_spec`).
    ProtocolSpec,
    /// A frame captured from a real or simulated device (`captured`).
    Captured,
    /// A frame authored to break a decoder: truncation, bad lengths, bad discriminators
    /// (`adversarial`).
    Adversarial,
}

/// One loaded golden vector.
///
/// # Examples
/// ```
/// use stonecrab::testkit::{GoldenVector, VectorSource};
///
/// let vector = GoldenVector {
///     name: "read".to_owned(),
///     source: VectorSource::ProtocolSpec,
///     bytes: vec![0x01, 0x02],
///     expected: None,
///     must_fail: false,
///     note: None,
/// };
/// assert_eq!(vector.bytes.len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GoldenVector {
    /// The vector's name, its path relative to the directory without extension and with `/`
    /// separators, exactly as `PROVENANCE.toml` lists it.
    pub name: String,
    /// Where the vector came from.
    pub source: VectorSource,
    /// The wire bytes from `<name>.hex`.
    pub bytes: Vec<u8>,
    /// The parsed `<name>.expected.json`, or `None` when the file does not exist.
    ///
    /// The file is authored independently of the decoder (13.6); the loader only parses
    /// it.
    pub expected: Option<serde_json::Value>,
    /// True when a decoder must reject the bytes.
    pub must_fail: bool,
    /// Free text from the provenance entry.
    pub note: Option<String>,
}

/// The schema of a `PROVENANCE.toml`, which holds two arrays of tables, both optional.
///
/// ```toml
/// [[vector]]            # one per wire vector
/// name = "tcp/read-request"      # path relative to the file, no extension, "/" separated
/// source = "protocol_spec"       # upstream_fixture | protocol_spec | captured | adversarial
/// origin = "Modbus Application Protocol V1.1b, section 6.3"
/// license = "Apache-2.0"         # SPDX expression
/// must_fail = false              # optional, default false
/// note = "free text"             # optional
///
/// [[file]]              # one per non-vector first-party fixture
/// path = "interop/register-map.toml"   # relative to the file; must exist
/// origin = "docs/parity/modbus.md, Interop register map"
/// license = "Apache-2.0"
/// note = "free text"             # optional
/// ```
///
/// Unknown keys are rejected so a misspelled field cannot silently drop provenance.
///
/// # Examples
/// ```
/// use stonecrab::testkit::Provenance;
///
/// let provenance = Provenance::parse(
///     "[[vector]]\nname = \"read\"\nsource = \"captured\"\norigin = \"bench capture\"\nlicense = \"Apache-2.0\"\n",
/// )?;
/// assert_eq!(provenance.vector.len(), 1);
/// assert!(!provenance.vector[0].must_fail);
/// # Ok::<(), stonecrab::testkit::GoldenError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// The `[[vector]]` entries, in file order.
    #[serde(default)]
    pub vector: Vec<VectorEntry>,
    /// The `[[file]]` entries, in file order.
    #[serde(default)]
    pub file: Vec<FileEntry>,
}

/// A `[[vector]]` entry of a `PROVENANCE.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorEntry {
    /// Path relative to the provenance file, without extension, `/` separated.
    pub name: String,
    /// Where the vector came from.
    pub source: VectorSource,
    /// The upstream file path and revision, or the specification section.
    pub origin: String,
    /// The vector's own license, as an SPDX expression.
    pub license: String,
    /// True when a decoder must reject the bytes. Defaults to false.
    #[serde(default)]
    pub must_fail: bool,
    /// Free text.
    #[serde(default)]
    pub note: Option<String>,
}

/// A `[[file]]` entry of a `PROVENANCE.toml`: a first-party fixture that is not a wire
/// vector, such as the interop register map.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    /// Path relative to the provenance file. The loader checks that it exists.
    pub path: String,
    /// Where the content was authored from.
    pub origin: String,
    /// The file's license, as an SPDX expression.
    pub license: String,
    /// Free text.
    #[serde(default)]
    pub note: Option<String>,
}

impl Provenance {
    /// Parse and validate the text of a `PROVENANCE.toml`.
    ///
    /// Validation: vector names are non-empty `/`-separated components of ASCII letters,
    /// digits, `-`, `_` and `.` that do not start with a dot, and are unique; file paths
    /// are relative with no `..`, root or prefix component and are unique; `origin` and
    /// `license` are not blank.
    ///
    /// # Errors
    /// [`GoldenError::Provenance`] when the text is not valid TOML for the schema or fails
    /// validation. The error carries no file name; [`load_dir`] adds it.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::Provenance;
    ///
    /// assert!(Provenance::parse("").is_ok(), "both arrays are optional");
    /// assert!(Provenance::parse("[[vector]]\nname = \"../x\"").is_err());
    /// ```
    pub fn parse(text: &str) -> Result<Self, GoldenError> {
        let provenance: Self = toml::from_str(text).map_err(|e| GoldenError::Provenance {
            path: None,
            message: e.to_string(),
        })?;
        provenance
            .validate()
            .map_err(|message| GoldenError::Provenance {
                path: None,
                message,
            })?;
        Ok(provenance)
    }

    fn validate(&self) -> Result<(), String> {
        let mut names = BTreeSet::new();
        for entry in &self.vector {
            if !valid_name(&entry.name) {
                return Err(format!(
                    "vector name `{}` is not a safe relative name",
                    entry.name
                ));
            }
            if !names.insert(entry.name.as_str()) {
                return Err(format!("vector `{}` is listed twice", entry.name));
            }
            check_text(&entry.name, &entry.origin, &entry.license)?;
        }
        let mut paths = BTreeSet::new();
        for entry in &self.file {
            if !valid_relative_path(&entry.path) {
                return Err(format!(
                    "file path `{}` is not a safe relative path",
                    entry.path
                ));
            }
            if !paths.insert(entry.path.as_str()) {
                return Err(format!("file `{}` is listed twice", entry.path));
            }
            check_text(&entry.path, &entry.origin, &entry.license)?;
        }
        Ok(())
    }
}

fn check_text(what: &str, origin: &str, license: &str) -> Result<(), String> {
    if origin.trim().is_empty() {
        return Err(format!("`{what}` has a blank origin"));
    }
    if license.trim().is_empty() {
        return Err(format!("`{what}` has a blank license"));
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains("//")
        && !path.ends_with('/')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// What is wrong with a hex vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HexProblem {
    /// The text has no data line (only comments and blank lines, or nothing).
    NoData,
    /// A second data line follows the first; a vector is exactly one line.
    ExtraLine {
        /// The 1-based line number of the second data line.
        line: usize,
    },
    /// A character that is neither a hex digit nor whitespace.
    InvalidDigit {
        /// The 1-based line number.
        line: usize,
        /// The offending character.
        digit: char,
    },
    /// The data line has an odd number of hex digits.
    OddLength {
        /// The 1-based line number.
        line: usize,
        /// The digit count.
        digits: usize,
    },
}

impl fmt::Display for HexProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoData => f.write_str("no hex data line"),
            Self::ExtraLine { line } => write!(f, "line {line}: a second hex data line"),
            Self::InvalidDigit { line, digit } => {
                write!(f, "line {line}: `{digit}` is not a hex digit")
            }
            Self::OddLength { line, digits } => {
                write!(f, "line {line}: odd number of hex digits ({digits})")
            }
        }
    }
}

/// Why a golden directory, vector or interop map did not load.
///
/// Every variant names the file it concerns, except where the text came from a string
/// ([`load_hex`], [`Provenance::parse`], [`crate::testkit::InteropMap::from_toml`]), where the
/// `path` is `None`.
#[derive(Debug)]
#[non_exhaustive]
pub enum GoldenError {
    /// A file or directory could not be read.
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The operating-system error.
        source: io::Error,
    },
    /// `PROVENANCE.toml` is not valid for the schema or fails validation.
    Provenance {
        /// The provenance file, when known.
        path: Option<PathBuf>,
        /// What is wrong.
        message: String,
    },
    /// An interop register map is not valid for the schema or fails validation.
    Interop {
        /// The map file, when known.
        path: Option<PathBuf>,
        /// What is wrong.
        message: String,
    },
    /// A `<name>.expected.json` is not valid JSON.
    Json {
        /// The JSON file.
        path: PathBuf,
        /// The parser's message.
        message: String,
    },
    /// A hex vector is malformed.
    Hex {
        /// The hex file, when the text came from one.
        path: Option<PathBuf>,
        /// What is wrong.
        problem: HexProblem,
    },
    /// A listed vector has no `.hex` file.
    MissingVector {
        /// The listed name.
        name: String,
        /// The `.hex` path that does not exist.
        path: PathBuf,
    },
    /// A `[[file]]` entry names a file that does not exist.
    MissingFile {
        /// The path that does not exist.
        path: PathBuf,
    },
    /// A `.hex` or `.expected.json` file exists that `PROVENANCE.toml` does not list.
    Unlisted {
        /// The unlisted file.
        path: PathBuf,
    },
}

impl GoldenError {
    /// Read a text file, mapping every failure to [`GoldenError::Io`].
    pub(crate) fn read(path: &Path) -> Result<String, Self> {
        fs::read_to_string(path).map_err(|source| Self::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Attach the file the text came from, unless the error already names one.
    pub(crate) fn in_file(self, file: &Path) -> Self {
        let file = Some(file.to_path_buf());
        match self {
            Self::Provenance {
                path: None,
                message,
            } => Self::Provenance {
                path: file,
                message,
            },
            Self::Interop {
                path: None,
                message,
            } => Self::Interop {
                path: file,
                message,
            },
            Self::Hex {
                path: None,
                problem,
            } => Self::Hex {
                path: file,
                problem,
            },
            named @ (Self::Provenance { .. }
            | Self::Interop { .. }
            | Self::Hex { .. }
            | Self::Io { .. }
            | Self::Json { .. }
            | Self::MissingVector { .. }
            | Self::MissingFile { .. }
            | Self::Unlisted { .. }) => named,
        }
    }
}

fn shown(path: Option<&PathBuf>) -> String {
    path.map_or_else(|| "<text>".to_owned(), |p| p.display().to_string())
}

impl fmt::Display for GoldenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Provenance { path, message } => {
                write!(f, "{}: invalid provenance: {message}", shown(path.as_ref()))
            }
            Self::Interop { path, message } => {
                write!(
                    f,
                    "{}: invalid interop map: {message}",
                    shown(path.as_ref())
                )
            }
            Self::Json { path, message } => {
                write!(f, "{}: invalid JSON: {message}", path.display())
            }
            Self::Hex { path, problem } => {
                write!(f, "{}: invalid hex: {problem}", shown(path.as_ref()))
            }
            Self::MissingVector { name, path } => write!(
                f,
                "{}: vector `{name}` is listed in PROVENANCE.toml but has no .hex file",
                path.display()
            ),
            Self::MissingFile { path } => write!(
                f,
                "{}: listed in PROVENANCE.toml but does not exist",
                path.display()
            ),
            Self::Unlisted { path } => write!(
                f,
                "{}: exists but is not listed in PROVENANCE.toml",
                path.display()
            ),
        }
    }
}

impl Error for GoldenError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Provenance { .. }
            | Self::Interop { .. }
            | Self::Json { .. }
            | Self::Hex { .. }
            | Self::MissingVector { .. }
            | Self::MissingFile { .. }
            | Self::Unlisted { .. } => None,
        }
    }
}

/// Decode the text of a `.hex` vector.
///
/// The text has exactly one data line: pairs of hex digits (either case) with any
/// whitespace between them. Everything from a `#` to the end of a line is a comment, and
/// blank or comment-only lines are skipped.
///
/// # Errors
/// [`GoldenError::Hex`] with a [`HexProblem`] when there is no data line, when there is
/// more than one, when a character is not a hex digit, or when the digit count is odd.
///
/// # Examples
/// ```
/// use stonecrab::testkit::load_hex;
///
/// let bytes = load_hex("# read request\n00 01 0A ff # trailing comment\n")?;
/// assert_eq!(bytes, [0x00, 0x01, 0x0a, 0xff]);
/// assert!(load_hex("0g").is_err());
/// # Ok::<(), stonecrab::testkit::GoldenError>(())
/// ```
pub fn load_hex(text: &str) -> Result<Vec<u8>, GoldenError> {
    let fail = |problem| GoldenError::Hex {
        path: None,
        problem,
    };
    let mut bytes = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index.saturating_add(1);
        let data = raw.split('#').next().unwrap_or_default();
        if data.chars().all(char::is_whitespace) {
            continue;
        }
        if bytes.is_some() {
            return Err(fail(HexProblem::ExtraLine { line }));
        }
        let mut nibbles = Vec::new();
        for digit in data.chars().filter(|c| !c.is_whitespace()) {
            let value = digit
                .to_digit(16)
                .and_then(|v| u8::try_from(v).ok())
                .ok_or_else(|| fail(HexProblem::InvalidDigit { line, digit }))?;
            nibbles.push(value);
        }
        if !nibbles.len().is_multiple_of(2) {
            return Err(fail(HexProblem::OddLength {
                line,
                digits: nibbles.len(),
            }));
        }
        bytes = Some(
            nibbles
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[hi, lo]| hi.rotate_left(4) | lo)
                .collect::<Vec<u8>>(),
        );
    }
    bytes.ok_or_else(|| fail(HexProblem::NoData))
}

/// Read the `PROVENANCE.toml` of a vector directory.
///
/// # Errors
/// [`GoldenError::Io`] when the file cannot be read, [`GoldenError::Provenance`] when it
/// is invalid.
///
/// # Examples
/// ```
/// use std::path::Path;
/// use stonecrab::testkit::load_provenance;
///
/// let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/clean");
/// let provenance = load_provenance(&dir)?;
/// assert_eq!(provenance.vector.len(), 2);
/// # Ok::<(), stonecrab::testkit::GoldenError>(())
/// ```
pub fn load_provenance(dir: &Path) -> Result<Provenance, GoldenError> {
    let path = dir.join(PROVENANCE_FILE);
    Provenance::parse(&GoldenError::read(&path)?).map_err(|e| e.in_file(&path))
}

/// Load every vector of a golden directory.
///
/// Reads `PROVENANCE.toml`, checks that every `[[file]]` entry exists, then enforces the
/// rule that every vector is listed: any `.hex` or `.expected.json` file under `dir`
/// (subdirectories included, symbolic links not followed) whose name is not a `[[vector]]`
/// entry is an error. For each listed vector it reads `<name>.hex` with [`load_hex`] and
/// the optional `<name>.expected.json`. The result is in provenance order.
///
/// The loader never decodes anything: `expected` is whatever the independently authored
/// JSON says (13.6).
///
/// # Errors
/// [`GoldenError::Io`] for an unreadable directory or file, [`GoldenError::Provenance`]
/// for an invalid provenance file, [`GoldenError::Unlisted`] for a vector file the
/// provenance does not list, [`GoldenError::MissingVector`] for a listed vector without a
/// `.hex`, [`GoldenError::MissingFile`] for a missing `[[file]]`, [`GoldenError::Hex`]
/// for a malformed `.hex` and [`GoldenError::Json`] for a malformed `.expected.json`.
/// Each names the file.
///
/// # Examples
/// ```
/// use std::path::Path;
/// use stonecrab::testkit::{VectorSource, load_dir};
///
/// let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/clean");
/// let vectors = load_dir(&dir)?;
/// assert_eq!(vectors[0].name, "read-request");
/// assert_eq!(vectors[0].source, VectorSource::ProtocolSpec);
/// assert_eq!(vectors[0].bytes.len(), 12);
/// assert!(vectors[0].expected.is_some());
/// # Ok::<(), stonecrab::testkit::GoldenError>(())
/// ```
pub fn load_dir(dir: &Path) -> Result<Vec<GoldenVector>, GoldenError> {
    let provenance = load_provenance(dir)?;
    for entry in &provenance.file {
        let path = dir.join(&entry.path);
        if !path.is_file() {
            return Err(GoldenError::MissingFile { path });
        }
    }
    let listed: BTreeSet<&str> = provenance.vector.iter().map(|v| v.name.as_str()).collect();
    let mut found = BTreeMap::new();
    discover(dir, "", &mut found)?;
    if let Some((_, path)) = found
        .iter()
        .find(|(name, _)| !listed.contains(name.as_str()))
    {
        return Err(GoldenError::Unlisted { path: path.clone() });
    }
    provenance
        .vector
        .iter()
        .map(|entry| load_vector(dir, entry))
        .collect()
}

fn load_vector(dir: &Path, entry: &VectorEntry) -> Result<GoldenVector, GoldenError> {
    let hex_path = dir.join(format!("{}.hex", entry.name));
    let text = fs::read_to_string(&hex_path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            GoldenError::MissingVector {
                name: entry.name.clone(),
                path: hex_path.clone(),
            }
        } else {
            GoldenError::Io {
                path: hex_path.clone(),
                source,
            }
        }
    })?;
    let bytes = load_hex(&text).map_err(|e| e.in_file(&hex_path))?;
    let json_path = dir.join(format!("{}.expected.json", entry.name));
    let expected = match fs::read_to_string(&json_path) {
        Ok(text) => Some(serde_json::from_str(&text).map_err(|e| GoldenError::Json {
            path: json_path.clone(),
            message: e.to_string(),
        })?),
        Err(source) if source.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(GoldenError::Io {
                path: json_path,
                source,
            });
        }
    };
    Ok(GoldenVector {
        name: entry.name.clone(),
        source: entry.source,
        bytes,
        expected,
        must_fail: entry.must_fail,
        note: entry.note.clone(),
    })
}

/// Collect the vector names implied by every `.hex` and `.expected.json` file under `dir`,
/// each with the first path that implied it, visiting entries in name order.
fn discover(
    dir: &Path,
    prefix: &str,
    found: &mut BTreeMap<String, PathBuf>,
) -> Result<(), GoldenError> {
    let io_error = |source| GoldenError::Io {
        path: dir.to_path_buf(),
        source,
    };
    let mut entries = fs::read_dir(dir)
        .map_err(io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_error)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let file_type = entry.file_type().map_err(io_error)?;
        if file_type.is_dir() {
            discover(&entry.path(), &format!("{prefix}{file_name}/"), found)?;
        } else if file_type.is_file() {
            let stem = file_name
                .strip_suffix(".hex")
                .or_else(|| file_name.strip_suffix(".expected.json"));
            if let Some(stem) = stem {
                found
                    .entry(format!("{prefix}{stem}"))
                    .or_insert_with(|| entry.path());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/golden")
            .join(name)
    }

    fn file_name(path: &Path) -> String {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    #[test]
    fn clean_tree_loads_both_vectors_in_provenance_order() {
        let vectors = load_dir(&tree("clean")).expect("clean tree loads");
        let names: Vec<_> = vectors.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(
            names,
            ["read-request", "tcp/read-response"],
            "provenance order"
        );
        let first = vectors.first().expect("first vector");
        assert_eq!(first.source, VectorSource::ProtocolSpec, "source");
        assert_eq!(
            first.bytes,
            [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02
            ],
            "comments and whitespace skipped"
        );
        assert_eq!(
            first.expected.as_ref().and_then(|v| v.get("quantity")),
            Some(&serde_json::json!(2)),
            "expected.json parsed, not computed"
        );
        assert!(!first.must_fail, "must_fail defaults to false");
        assert_eq!(
            first.note.as_deref(),
            Some("FC03 read of two holding registers, unit 1")
        );
        let second = vectors.get(1).expect("second vector");
        assert_eq!(
            second.source,
            VectorSource::Captured,
            "nested vector source"
        );
        assert_eq!(second.bytes.len(), 13, "nested vector bytes");
        assert!(second.note.is_none(), "no note");
    }

    #[test]
    fn must_fail_vector_without_expected_json_loads() {
        let vectors = load_dir(&tree("must_fail")).expect("loads");
        let vector = vectors.first().expect("one vector");
        assert!(vector.must_fail, "flag carried");
        assert!(vector.expected.is_none(), "no expected.json is allowed");
        assert_eq!(vector.source, VectorSource::Adversarial, "source");
        assert_eq!(vector.bytes, [0, 1, 0, 0], "bytes");
    }

    #[test]
    fn unlisted_hex_file_is_an_error_naming_the_file() {
        // covers: CODING_STANDARDS 13.6, an unlisted vector file is an error
        let error = load_dir(&tree("unlisted")).expect_err("stray.hex is unlisted");
        assert!(
            matches!(&error, GoldenError::Unlisted { path } if file_name(path) == "stray.hex"),
            "unexpected {error}"
        );
        assert!(
            error.to_string().contains("stray.hex"),
            "Display names the file"
        );
        assert!(error.to_string().contains("not listed"), "Display says why");
    }

    #[test]
    fn unlisted_expected_json_is_an_error_naming_the_file() {
        // covers: CODING_STANDARDS 13.6, an unlisted expected file is an error
        let error = load_dir(&tree("unlisted_expected")).expect_err("orphan is unlisted");
        assert!(
            matches!(&error, GoldenError::Unlisted { path } if file_name(path) == "orphan.expected.json"),
            "unexpected {error}"
        );
    }

    #[test]
    fn listed_vector_without_hex_is_an_error_naming_the_file() {
        // covers: CODING_STANDARDS 13.6, a listed vector without a hex file is an error
        let error = load_dir(&tree("missing_hex")).expect_err("absent.hex is missing");
        assert!(
            matches!(&error, GoldenError::MissingVector { name, path }
                if name == "absent" && file_name(path) == "absent.hex"),
            "unexpected {error}"
        );
        assert!(
            error.to_string().contains("absent.hex"),
            "Display names the file"
        );
    }

    #[test]
    fn listed_file_that_does_not_exist_is_an_error() {
        // covers: CODING_STANDARDS 13.6, a listed file that is absent is an error
        let error = load_dir(&tree("missing_file")).expect_err("absent.toml is missing");
        assert!(
            matches!(&error, GoldenError::MissingFile { path } if file_name(path) == "absent.toml"),
            "unexpected {error}"
        );
        assert!(
            error.to_string().contains("absent.toml"),
            "Display names the file"
        );
    }

    #[test]
    fn odd_hex_line_is_an_error_naming_the_file_and_line() {
        // covers: CODING_STANDARDS 13.7, malformed hex is rejected with its file and line
        let error = load_dir(&tree("bad_hex")).expect_err("odd digit count");
        assert!(
            matches!(&error, GoldenError::Hex { path: Some(p), problem: HexProblem::OddLength { line: 2, digits: 5 } }
                if file_name(p) == "odd.hex"),
            "unexpected {error}"
        );
        let text = error.to_string();
        assert!(
            text.contains("odd.hex") && text.contains("line 2"),
            "Display: {text}"
        );
    }

    #[test]
    fn invalid_expected_json_is_an_error_naming_the_file() {
        // covers: CODING_STANDARDS 13.7, malformed expected JSON is rejected
        let error = load_dir(&tree("bad_json")).expect_err("truncated JSON");
        assert!(
            matches!(&error, GoldenError::Json { path, .. } if file_name(path) == "v.expected.json"),
            "unexpected {error}"
        );
        assert!(
            error.to_string().contains("v.expected.json"),
            "Display names the file"
        );
    }

    #[test]
    fn invalid_provenance_is_an_error_naming_the_file() {
        // covers: CODING_STANDARDS 13.7, malformed provenance is rejected
        let error = load_dir(&tree("bad_provenance")).expect_err("hostile name");
        assert!(
            matches!(&error, GoldenError::Provenance { path: Some(p), .. } if file_name(p) == "PROVENANCE.toml"),
            "unexpected {error}"
        );
        assert!(
            error.to_string().contains("../escape"),
            "message names the entry"
        );
    }

    #[test]
    fn missing_provenance_and_non_directories_are_io_errors() {
        let no_provenance =
            load_dir(&tree("")).expect_err("testdata/golden has no PROVENANCE.toml");
        assert!(
            matches!(&no_provenance, GoldenError::Io { path, .. } if file_name(path) == "PROVENANCE.toml"),
            "unexpected {no_provenance}"
        );
        assert!(
            no_provenance.source().is_some(),
            "the OS error is the source"
        );
        assert!(
            no_provenance.to_string().contains("PROVENANCE.toml"),
            "Display names the file"
        );
    }

    #[test]
    fn unreadable_vector_files_are_io_errors_not_missing_vectors() {
        // A directory standing where a file belongs fails on every platform with an
        // error that is not NotFound.
        for (tree_name, expected_file) in
            [("io_hex_dir", "v.hex"), ("io_json_dir", "v.expected.json")]
        {
            let error = load_dir(&tree(tree_name)).expect_err("a directory is not a file");
            assert!(
                matches!(&error, GoldenError::Io { path, .. } if file_name(path) == expected_file),
                "unexpected {error}"
            );
        }
    }

    #[test]
    fn golden_dir_lists_every_vector_in_provenance() {
        // covers: CODING_STANDARDS 13.6, every vector is listed in PROVENANCE.toml
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        let vectors = load_dir(&dir).expect("fixtures/modbus loads: every file is listed");
        assert!(!vectors.is_empty(), "the Modbus vectors");
        let upstream: Vec<_> = vectors
            .iter()
            .filter(|v| v.source == VectorSource::UpstreamFixture)
            .collect();
        // The driver-suite adapter (S04b, tests/modbus_driver_suite_adapter.rs) owns the
        // vectors named after DriverTestsuite.xml and the two all-datatypes files.
        let parser_suite = upstream
            .iter()
            .filter(|v| {
                !v.name.starts_with("tcp/adu_upstream-driver-suite-")
                    && !v.name.starts_with("tcp/adu_upstream-all-datatypes-")
            })
            .count();
        assert_eq!(
            parser_suite, 6,
            "the six <raw> elements of the parser suite"
        );
        assert_eq!(
            upstream.len(),
            70,
            "the six plus the 64 request and response vectors of the 32 driver-suite read exchanges"
        );
        assert!(
            vectors.iter().all(|v| v.source != VectorSource::Captured),
            "nothing is captured from a device yet"
        );
        let provenance = load_provenance(&dir).expect("provenance parses");
        let files: Vec<_> = provenance.file.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            files,
            [
                "interop/register-map.toml",
                "upstream/tcp/ParserSerializerTestsuite.xml",
                "upstream/tcp/DriverTestsuite.xml",
                "upstream/tcp/Modbus-all-datatypes.xml",
                "upstream/tcp/Modbus-all-datatypes-little-endian.xml",
            ],
            "the register map and the four retained upstream files"
        );
    }

    // Every upstream vector is derived from the retained file, never hand-typed.
    #[test]
    fn adapter_rederives_each_parser_suite_hex_from_raw_text() {
        // The order of the <raw> elements in upstream/tcp/ParserSerializerTestsuite.xml.
        let names = [
            "tcp/adu_upstream-read-input-registers-request",
            "tcp/adu_upstream-read-input-registers-response",
            "tcp/adu_upstream-read-extended-registers-request-split-file-record",
            "tcp/adu_upstream-read-extended-registers-response-split-file-record",
            "tcp/adu_upstream-write-extended-registers-request-file-record",
            "tcp/adu_upstream-write-extended-registers-request-split-file-record",
        ];
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        let xml = fs::read_to_string(dir.join("upstream/tcp/ParserSerializerTestsuite.xml"))
            .expect("the retained parser suite is readable");
        // A string scan, not an XML parser: the text between each `<raw>` and its `</raw>`.
        let mut raws = Vec::new();
        let mut rest = xml.as_str();
        while let Some((_, after_open)) = rest.split_once("<raw>") {
            let (inside, after_close) = after_open
                .split_once("</raw>")
                .expect("every <raw> is closed");
            raws.push(inside.split_whitespace().collect::<String>().to_lowercase());
            rest = after_close;
        }
        assert_eq!(raws.len(), names.len(), "six <raw> elements");
        let vectors = load_dir(&dir).expect("fixtures/modbus loads");
        for (name, raw) in names.iter().zip(&raws) {
            let vector = vectors
                .iter()
                .find(|v| v.name == *name)
                .unwrap_or_else(|| panic!("{name} is listed"));
            assert_eq!(vector.source, VectorSource::UpstreamFixture, "{name}");
            let mut derived = String::new();
            for byte in &vector.bytes {
                for nibble in [byte >> 4, byte & 0x0F] {
                    derived.push(char::from_digit(u32::from(nibble), 16).expect("a nibble"));
                }
            }
            assert_eq!(&derived, raw, "{name}: the .hex against the <raw> text");
        }
    }

    #[test]
    fn hex_accepts_case_whitespace_and_comments() {
        assert_eq!(load_hex("aB cd\n").expect("mixed case"), [0xab, 0xcd]);
        assert_eq!(
            load_hex("# c\n\n  0102 # t\n\t\n").expect("comments"),
            [1, 2]
        );
        assert_eq!(load_hex("00\r\n").expect("crlf"), [0]);
    }

    #[test]
    fn hex_rejects_malformed_text() {
        // covers: CODING_STANDARDS 13.7, every malformed hex shape is rejected
        let problem = |text: &str| match load_hex(text) {
            Err(GoldenError::Hex {
                path: None,
                problem,
            }) => problem,
            other => panic!("expected a hex error, got {other:?}"),
        };
        assert_eq!(problem(""), HexProblem::NoData, "empty");
        assert_eq!(
            problem("# only a comment\n \n"),
            HexProblem::NoData,
            "comments only"
        );
        assert_eq!(
            problem("0\n"),
            HexProblem::OddLength { line: 1, digits: 1 },
            "odd"
        );
        assert_eq!(
            problem("# c\n0x01\n"),
            HexProblem::InvalidDigit {
                line: 2,
                digit: 'x'
            },
            "prefix"
        );
        assert_eq!(
            problem("00 \u{e9}\n"),
            HexProblem::InvalidDigit {
                line: 1,
                digit: '\u{e9}'
            },
            "non-ascii"
        );
        assert_eq!(
            problem("01\n02\n"),
            HexProblem::ExtraLine { line: 2 },
            "two lines"
        );
    }

    #[test]
    fn hex_problems_render_a_reason() {
        for (problem, needle) in [
            (HexProblem::NoData, "no hex data"),
            (HexProblem::ExtraLine { line: 3 }, "line 3"),
            (
                HexProblem::InvalidDigit {
                    line: 1,
                    digit: 'z',
                },
                "`z`",
            ),
            (HexProblem::OddLength { line: 1, digits: 7 }, "(7)"),
        ] {
            assert!(
                problem.to_string().contains(needle),
                "{problem} lacks {needle}"
            );
        }
        let text = GoldenError::Hex {
            path: None,
            problem: HexProblem::NoData,
        }
        .to_string();
        assert!(
            text.starts_with("<text>:"),
            "string input has no file: {text}"
        );
    }

    #[test]
    fn provenance_defaults_and_rejects_unknown_keys() {
        let empty = Provenance::parse("").expect("empty file is valid");
        assert!(
            empty.vector.is_empty() && empty.file.is_empty(),
            "both arrays optional"
        );
        for bad in [
            "surprise = 1",
            "[[vector]]\nname = \"a\"\nsource = \"captured\"\norigin = \"o\"\nlicense = \"l\"\nextra = 1\n",
            "[[vector]]\nname = \"a\"\nsource = \"somewhere\"\norigin = \"o\"\nlicense = \"l\"\n",
            "[[vector]]\nname = \"a\"\nsource = \"captured\"\nlicense = \"l\"\n",
            "not toml [",
        ] {
            let error = Provenance::parse(bad).expect_err(bad);
            assert!(
                matches!(error, GoldenError::Provenance { path: None, .. }),
                "{bad}"
            );
            assert!(
                error.to_string().starts_with("<text>:"),
                "no file for string input"
            );
            assert!(error.source().is_none(), "no source");
        }
    }

    #[test]
    fn provenance_validation_rejects_unsafe_duplicate_and_blank_entries() {
        // covers: CODING_STANDARDS 13.7, hostile provenance entries are rejected
        let vector = |name: &str, origin: &str, license: &str| {
            format!(
                "[[vector]]\nname = \"{name}\"\nsource = \"captured\"\norigin = \"{origin}\"\nlicense = \"{license}\"\n"
            )
        };
        let file = |path: &str, origin: &str, license: &str| {
            format!("[[file]]\npath = \"{path}\"\norigin = \"{origin}\"\nlicense = \"{license}\"\n")
        };
        for (text, needle) in [
            (vector("", "o", "l"), "safe relative name"),
            (vector("/abs", "o", "l"), "safe relative name"),
            (vector("a//b", "o", "l"), "safe relative name"),
            (vector(".hidden", "o", "l"), "safe relative name"),
            (vector("a/../b", "o", "l"), "safe relative name"),
            (vector("a b", "o", "l"), "safe relative name"),
            (vector("a", " ", "l"), "blank origin"),
            (vector("a", "o", ""), "blank license"),
            (
                format!("{}{}", vector("a", "o", "l"), vector("a", "o", "l")),
                "listed twice",
            ),
            (file("", "o", "l"), "safe relative path"),
            (file("/etc/passwd", "o", "l"), "safe relative path"),
            (file("../x", "o", "l"), "safe relative path"),
            (file("sub//x", "o", "l"), "safe relative path"),
            (file("sub/", "o", "l"), "safe relative path"),
            (file("a", "", "l"), "blank origin"),
            (file("a", "o", " "), "blank license"),
            (
                format!("{}{}", file("a", "o", "l"), file("a", "o", "l")),
                "listed twice",
            ),
        ] {
            let error = Provenance::parse(&text).expect_err(&text);
            assert!(error.to_string().contains(needle), "{text}: {error}");
        }
        let fine = format!(
            "{}{}",
            vector("tcp/a-b_c.d", "o", "l"),
            file("x/y.toml", "o", "l")
        );
        assert!(
            Provenance::parse(&fine).is_ok(),
            "valid names and paths pass"
        );
    }

    #[test]
    fn io_error_exposes_its_source() {
        let error = GoldenError::Io {
            path: PathBuf::from("x"),
            source: io::Error::other("boom"),
        };
        assert_eq!(error.to_string(), "x: boom", "Display");
        assert!(error.source().is_some(), "source");
    }

    #[test]
    fn in_file_keeps_an_existing_name() {
        let error = GoldenError::Provenance {
            path: Some(PathBuf::from("first")),
            message: "m".to_owned(),
        }
        .in_file(Path::new("second"));
        assert!(error.to_string().starts_with("first:"), "{error}");
        let interop = GoldenError::Interop {
            path: None,
            message: "m".to_owned(),
        }
        .in_file(Path::new("map.toml"));
        assert!(
            interop
                .to_string()
                .starts_with("map.toml: invalid interop map"),
            "{interop}"
        );
    }
}
