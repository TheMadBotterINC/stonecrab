//! `stonecrab-codegen`: generates Stonecrab protocol codecs from Apache PLC4X `MSpec`
//! descriptions (ADR-0019).
//!
//! ```text
//! stonecrab-codegen generate <options.toml> [<output directory>]
//! stonecrab-codegen check <PLC4X checkout>
//! ```
//!
//! `generate` runs from the repository root. It reads the protocol options, finds the
//! protocol's vendored mspec files in `upstream/manifest.toml`, refuses to run when a copy's
//! sha256 differs from the manifest, parses and resolves the files, emits the enabled types,
//! formats each module with the pinned `rustfmt` and replaces `generated.rs`, `generated/`
//! and `generated/MANIFEST.toml` in the output directory (the options' `output` unless one
//! is given). `check` parses every `*.mspec` file of a read-only checkout and prints one
//! line per file; it emits nothing (ADR-0019 decision 10, maintainer-only).

#![forbid(unsafe_code)]

pub mod emit;
pub mod expression;
pub mod mspec;
pub mod naming;
pub mod output;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest as _, Sha256};

use crate::emit::{Endianness, Options};

/// A failure of the tool: what it was doing and why.
#[derive(Debug)]
struct Failure(String);

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn fail<T>(message: impl Into<String>) -> Result<T, Failure> {
    Err(Failure(message.into()))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["generate", options] => generate(Path::new(options), None),
        ["generate", options, out] => generate(Path::new(options), Some(Path::new(out))),
        ["check", checkout] => check(Path::new(checkout), &mut io::stdout().lock()),
        _ => fail(
            "usage: stonecrab-codegen generate <options.toml> [<output directory>]\n       stonecrab-codegen check <PLC4X checkout>",
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        // Exit status 2 when even the report could not be written.
        Err(failure) => match report(&failure) {
            Ok(()) => ExitCode::FAILURE,
            Err(_) => ExitCode::from(2),
        },
    }
}

/// Tells the person running the tool what failed, on standard error.
fn report(failure: &Failure) -> io::Result<()> {
    writeln!(io::stderr().lock(), "stonecrab-codegen: {failure}")
}

fn read(path: &Path) -> Result<String, Failure> {
    std::fs::read_to_string(path)
        .or_else(|error| fail(format!("cannot read {}: {error}", path.display())))
}

fn table(path: &Path) -> Result<toml::Table, Failure> {
    read(path)?
        .parse::<toml::Table>()
        .or_else(|error| fail(format!("{} is not valid TOML: {error}", path.display())))
}

fn string(table: &toml::Table, key: &str, origin: &str) -> Result<String, Failure> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Failure(format!("{origin}: `{key}` must be a string")))
}

fn strings(value: Option<&toml::Value>, origin: &str) -> Result<BTreeSet<String>, Failure> {
    let Some(items) = value.and_then(toml::Value::as_array) else {
        return fail(format!("{origin} must be an array of strings"));
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| Failure(format!("{origin} must be an array of strings")))
        })
        .collect()
}

/// The protocol options file (`tools/stonecrab-codegen/protocols/<protocol>.toml`).
#[derive(Debug)]
struct ProtocolOptions {
    protocol: String,
    output: PathBuf,
    options: Options,
}

fn load_options(path: &Path) -> Result<ProtocolOptions, Failure> {
    let origin = path.display().to_string();
    let table = table(path)?;
    for key in table.keys() {
        if ![
            "protocol",
            "output",
            "support_module",
            "byte_order",
            "types",
            "cases",
        ]
        .contains(&key.as_str())
        {
            return fail(format!("{origin}: unknown key `{key}`"));
        }
    }
    let mut cases = BTreeMap::new();
    if let Some(value) = table.get("cases") {
        let Some(entries) = value.as_table() else {
            return fail(format!("{origin}: `cases` must be a table"));
        };
        for (type_name, names) in entries {
            cases.insert(
                type_name.clone(),
                strings(Some(names), &format!("{origin}: cases.{type_name}"))?,
            );
        }
    }
    let byte_order = if table.contains_key("byte_order") {
        let keyword = string(&table, "byte_order", &origin)?;
        match Endianness::from_keyword(&keyword) {
            Some(order) => Some(order),
            None => {
                return fail(format!(
                    "{origin}: `byte_order` must be \"BIG_ENDIAN\" or \"LITTLE_ENDIAN\", not \"{keyword}\""
                ));
            }
        }
    } else {
        None
    };
    Ok(ProtocolOptions {
        protocol: string(&table, "protocol", &origin)?,
        output: PathBuf::from(string(&table, "output", &origin)?),
        options: Options {
            support_module: string(&table, "support_module", &origin)?,
            byte_order,
            types: strings(table.get("types"), &format!("{origin}: types"))?,
            cases,
        },
    })
}

/// One vendored mspec as `upstream/manifest.toml` lists it.
#[derive(Debug)]
struct MspecFile {
    upstream_path: String,
    vendored_path: String,
    sha256: String,
}

/// The upstream baseline and the protocol's vendored files.
#[derive(Debug)]
struct Baseline {
    project: String,
    tag: String,
    commit: String,
    files: Vec<MspecFile>,
}

const MANIFEST: &str = "upstream/manifest.toml";

fn load_baseline(protocol: &str) -> Result<Baseline, Failure> {
    let manifest = table(Path::new(MANIFEST))?;
    let Some(upstream) = manifest.get("upstream").and_then(toml::Value::as_table) else {
        return fail(format!("{MANIFEST}: no [upstream] table"));
    };
    let mut files = Vec::new();
    if let Some(entries) = manifest.get("mspec").and_then(toml::Value::as_array) {
        for entry in entries {
            let Some(entry) = entry.as_table() else {
                return fail(format!("{MANIFEST}: [[mspec]] entries must be tables"));
            };
            if string(entry, "protocol", MANIFEST)? == protocol {
                files.push(MspecFile {
                    upstream_path: string(entry, "upstream_path", MANIFEST)?,
                    vendored_path: string(entry, "vendored_path", MANIFEST)?,
                    sha256: string(entry, "sha256", MANIFEST)?,
                });
            }
        }
    }
    if files.is_empty() {
        return fail(format!(
            "{MANIFEST}: no [[mspec]] entry for protocol `{protocol}`"
        ));
    }
    Ok(Baseline {
        project: string(upstream, "project", MANIFEST)?,
        tag: string(upstream, "tag", MANIFEST)?,
        commit: string(upstream, "commit", MANIFEST)?,
        files,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .concat()
}

fn generate(options_path: &Path, out: Option<&Path>) -> Result<(), Failure> {
    if !Path::new(MANIFEST).is_file() {
        return fail(format!(
            "run from the repository root: {MANIFEST} not found"
        ));
    }
    if options_path.is_absolute() {
        return fail("give the options file relative to the repository root");
    }
    let options = load_options(options_path)?;
    let baseline = load_baseline(&options.protocol)?;
    let mut specs = Vec::new();
    for file in &baseline.files {
        let bytes = std::fs::read(&file.vendored_path)
            .or_else(|error| fail(format!("cannot read {}: {error}", file.vendored_path)))?;
        let digest = sha256_hex(&bytes);
        if digest != file.sha256 {
            return fail(format!(
                "{} has sha256 {digest}, but {MANIFEST} records {}; refusing to generate",
                file.vendored_path, file.sha256
            ));
        }
        let text = String::from_utf8(bytes)
            .or_else(|error| fail(format!("{} is not UTF-8: {error}", file.vendored_path)))?;
        let spec = mspec::parse(&text)
            .or_else(|error| fail(format!("{}: {error}", file.vendored_path)))?;
        specs.push(spec);
    }
    let refs: Vec<&mspec::Spec> = specs.iter().collect();
    let known = mspec::type_names(&refs);
    for (spec, file) in specs.iter().zip(&baseline.files) {
        mspec::resolve(spec, &known)
            .or_else(|error| fail(format!("{}: {error}", file.vendored_path)))?;
    }
    let modules = emit::generate(&specs, &options.options)
        .or_else(|error| fail(format!("generation error: {error}")))?;
    let header = output::header(&baseline);
    let mut files = BTreeMap::new();
    for (module, body) in &modules {
        files.insert(
            format!("generated/{module}.rs"),
            format!("{header}\n{}", output::rustfmt(body)?),
        );
    }
    files.insert(
        "generated.rs".to_owned(),
        format!(
            "{header}\n{}",
            output::rustfmt(&output::module_list(modules.keys()))?
        ),
    );
    files.insert(
        "generated/MANIFEST.toml".to_owned(),
        output::manifest(&baseline, &options.options, options_path),
    );
    output::replace(out.unwrap_or(&options.output), &files)
}

fn check(checkout: &Path, out: &mut impl io::Write) -> Result<(), Failure> {
    let mut files = Vec::new();
    collect_mspecs(checkout, &mut files)?;
    files.sort();
    let mut by_context: BTreeMap<PathBuf, Vec<(PathBuf, mspec::Spec)>> = BTreeMap::new();
    let mut failures = 0_usize;
    let mut say = |text: String| {
        writeln!(out, "{text}").or_else(|error| fail(format!("cannot write the report: {error}")))
    };
    for path in files {
        let shown = path
            .strip_prefix(checkout)
            .unwrap_or(&path)
            .display()
            .to_string();
        match read(&path)
            .and_then(|text| mspec::parse(&text).or_else(|error| fail(error.to_string())))
        {
            Ok(spec) => {
                let context = context_of(checkout, &path);
                by_context.entry(context).or_default().push((path, spec));
            }
            Err(failure) => {
                failures = failures.saturating_add(1);
                say(format!("FAIL {shown}: {failure}"))?;
            }
        }
    }
    for specs in by_context.values() {
        let refs: Vec<&mspec::Spec> = specs.iter().map(|(_, spec)| spec).collect();
        let known = mspec::type_names(&refs);
        for (path, spec) in specs {
            let shown = path
                .strip_prefix(checkout)
                .unwrap_or(path)
                .display()
                .to_string();
            match mspec::resolve(spec, &known) {
                Ok(()) => say(format!("ok   {shown} ({} types)", spec.types.len()))?,
                Err(error) => {
                    failures = failures.saturating_add(1);
                    say(format!("FAIL {shown}: {error}"))?;
                }
            }
        }
    }
    if failures == 0 {
        Ok(())
    } else {
        fail(format!("{failures} mspec files failed"))
    }
}

/// The type context a file belongs to: every file of one `protocols/<name>` module, as a
/// protocol's `Protocol` class loads its resources and generated mspecs together (for
/// example the `TagClass` that the generated `bacnetip` files use from `src/main/resources`);
/// elsewhere the file's directory.
fn context_of(checkout: &Path, path: &Path) -> PathBuf {
    let relative = path.strip_prefix(checkout).unwrap_or(path);
    let mut parts = relative.components();
    match (parts.next(), parts.next()) {
        (Some(first), Some(second)) if first.as_os_str() == "protocols" => {
            Path::new(first.as_os_str()).join(second.as_os_str())
        }
        _ => path.parent().map(Path::to_path_buf).unwrap_or_default(),
    }
}

fn collect_mspecs(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), Failure> {
    let entries = std::fs::read_dir(directory)
        .or_else(|error| fail(format!("cannot list {}: {error}", directory.display())))?;
    for entry in entries {
        let path = entry
            .or_else(|error| fail(format!("cannot list {}: {error}", directory.display())))?
            .path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            collect_mspecs(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "mspec")
        {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Baseline, check, load_options, output, sha256_hex};
    use crate::emit::Endianness;

    #[test]
    fn sha256_matches_a_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "FIPS 180-2 example"
        );
    }

    #[test]
    fn check_reports_each_file_and_fails_on_a_bad_one() {
        let root =
            std::env::temp_dir().join(format!("stonecrab-codegen-check-{}", std::process::id()));
        let protocol = root.join("protocols/demo");
        std::fs::create_dir_all(&protocol).expect("scratch directory");
        std::fs::write(protocol.join("good.mspec"), "[type A [simple B b]]").expect("write");
        std::fs::write(protocol.join("other.mspec"), "[enum uint 8 B ['1' ONE]]").expect("write");
        let mut report = Vec::new();
        check(&root, &mut report).expect("both files parse and resolve across the directory");
        let text = String::from_utf8(report).expect("UTF-8");
        assert_eq!(text.lines().count(), 2, "{text}");
        std::fs::write(protocol.join("bad.mspec"), "[type C [simple Missing m]]").expect("write");
        let mut report = Vec::new();
        assert!(
            check(&root, &mut report).is_err(),
            "an unresolved type fails"
        );
        let text = String::from_utf8(report).expect("UTF-8");
        assert!(text.contains("FAIL protocols/demo/bad.mspec"), "{text}");
        std::fs::remove_dir_all(&root).expect("clean up");
    }

    fn options_file(name: &str, byte_order: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "stonecrab-codegen-options-{name}-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            format!(
                "protocol = \"demo\"\noutput = \"out\"\nsupport_module = \"crate::support\"\n{byte_order}types = []\n"
            ),
        )
        .expect("write");
        path
    }

    #[test]
    fn the_byte_order_option_is_big_or_little_endian_and_recorded_when_set() {
        let baseline = Baseline {
            project: "Demo".to_owned(),
            tag: "v1".to_owned(),
            commit: "0".repeat(40),
            files: Vec::new(),
        };
        for (name, line, expected) in [
            ("absent", "", None),
            (
                "big",
                "byte_order = \"BIG_ENDIAN\"\n",
                Some(Endianness::Big),
            ),
            (
                "little",
                "byte_order = \"LITTLE_ENDIAN\"\n",
                Some(Endianness::Little),
            ),
        ] {
            let path = options_file(name, line);
            let loaded = load_options(&path).expect("loads");
            std::fs::remove_file(&path).expect("clean up");
            assert_eq!(loaded.options.byte_order, expected, "{name}");
            let manifest = output::manifest(&baseline, &loaded.options, &path);
            assert_eq!(
                manifest.contains("byte_order"),
                expected.is_some(),
                "{name}: recorded when the options set it\n{manifest}"
            );
            if let Some(order) = expected {
                assert!(
                    manifest.contains(&format!("byte_order = \"{}\"", order.keyword())),
                    "{name}\n{manifest}"
                );
            }
        }
        for (name, line) in [
            ("middle", "byte_order = \"MIDDLE_ENDIAN\"\n"),
            ("lowercase", "byte_order = \"little_endian\"\n"),
            ("number", "byte_order = 1\n"),
        ] {
            let path = options_file(name, line);
            let error = load_options(&path).expect_err(name);
            std::fs::remove_file(&path).expect("clean up");
            assert!(error.0.contains("byte_order"), "{name}: {}", error.0);
        }
    }
}
