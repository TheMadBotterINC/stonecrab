//! The Rust emitter (ADR-0019 decisions 3 to 6).
//!
//! It writes plain text, one module per enabled top-level type, which the caller formats
//! with the pinned `rustfmt`. The emitted subset is the feature matrix of `README.md`;
//! anything outside it in an enabled type or case is a [`GenError`] naming the construct,
//! the type and the mspec line. Every read and write goes through `BoundedReader` and
//! `BoundedWriter`; arithmetic is checked and conversions use `try_from`, so the output
//! has no `as`, no indexing and no panic (`CODING_STANDARDS` 17.3).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::expression::{Expr, UnaryOp};
use crate::mspec::{
    Base, Case, DataType, FieldKind, Item, MExpr, Spec, TypeDef, TypeForm, TypeRef, fields_of,
};
use crate::naming::{is_keyword, snake, upper_camel};

mod enums;
mod fields;
mod structs;
mod switch;

#[cfg(test)]
mod array_fixtures;
#[cfg(test)]
mod case_fixtures;
#[cfg(test)]
mod external_fixtures;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod reserved_fixtures;

use fields::{Elem, Part};

/// The byte order of every multi-byte read and write of a protocol: the `byteOrder` an
/// mspec attribute names. Upstream's default (that of its buffers) is `BIG_ENDIAN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Endianness {
    /// `BIG_ENDIAN`, the default.
    Big,
    /// `LITTLE_ENDIAN`, as OPC UA writes (`opc-manual.mspec` line 29 names it on `OpcuaAPU`).
    Little,
}

impl Endianness {
    /// The `byteOrder` value as an mspec attribute and an options file write it.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            Self::Big => "BIG_ENDIAN",
            Self::Little => "LITTLE_ENDIAN",
        }
    }

    /// The `stonecrab::ByteOrder` variant the output passes to every multi-byte access.
    const fn rust(self) -> &'static str {
        match self {
            Self::Big => "ByteOrder::BigEndian",
            Self::Little => "ByteOrder::LittleEndian",
        }
    }

    /// The order named by `keyword`, when it names one.
    pub(crate) fn from_keyword(keyword: &str) -> Option<Self> {
        [Self::Big, Self::Little]
            .into_iter()
            .find(|order| order.keyword() == keyword)
    }
}

/// The options of one protocol (ADR-0019 decision 6): what is enabled, and where the
/// hand-written support module is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    /// The Rust path of the hand-written support module the output imports.
    pub(crate) support_module: String,
    /// The protocol's byte order when the options file names one; `BIG_ENDIAN` otherwise.
    /// A `byteOrder` attribute in the mspec must agree with it.
    pub(crate) byte_order: Option<Endianness>,
    /// The enabled top-level types, by mspec name.
    pub(crate) types: BTreeSet<String>,
    /// The enabled cases of each enabled `discriminatedType` and `dataIo`, by mspec name.
    pub(crate) cases: BTreeMap<String, BTreeSet<String>>,
    /// The Rust path of the recursion budget type, passed through every generated `parse`,
    /// `serialize` and `length_in_bytes` and to every external type (ADR-0034 decision 3).
    /// `None` leaves the signatures as they are; `external` needs it.
    pub(crate) depth: Option<String>,
    /// The hand-written types the output calls instead of generating them, by mspec name.
    pub(crate) external: BTreeMap<String, External>,
}

/// A hand-written type that stands for an mspec type (the `[external]` table of a protocol's
/// options). The Rust type has one fixed shape, whatever the mspec type:
///
/// ```text
/// fn parse(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError>
/// fn serialize(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError>
/// fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError>
/// ```
///
/// `Depth` is the options' `depth` type, a small `Copy` value. The mspec type's parameters
/// are not passed: an external type with parameters pins them to literals here, and a use
/// that passes anything else is a generation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct External {
    /// The Rust path of the type.
    pub(crate) path: String,
    /// The literal argument of each mspec parameter, as written (`true`, `391`), in
    /// parameter order; empty for a type without parameters.
    pub(crate) arguments: Vec<String>,
}

/// A generation failure: the construct, the type and the mspec line (SPEC requirement 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GenError {
    /// The mspec line.
    pub(crate) line: usize,
    /// The type being generated.
    pub(crate) type_name: String,
    /// What is not supported or not consistent.
    pub(crate) message: String,
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, type {}: {}",
            self.line, self.type_name, self.message
        )
    }
}

/// The attribute values the emitter implements; any other value is a generation error.
/// `byteOrder` is checked against the protocol's byte order instead (`check_attributes`).
const ATTRIBUTES: [(&str, &[&str]); 4] = [
    ("unsignedIntegerEncoding", &["unsigned-binary"]),
    ("signedIntegerEncoding", &["twos-complement"]),
    ("floatEncoding", &["IEEE754"]),
    ("stringEncoding", &["UTF8"]),
];

/// Names the emitted code uses for itself, which no field or parameter may take. `depth` is
/// one more when the protocol has a `depth` option ([`BUDGET`]).
const RESERVED: [&str; 8] = [
    "reader",
    "writer",
    "plc_value",
    "size",
    "array_count",
    "array_needed",
    "array_items",
    "array_item",
];

/// The recursion budget parameter of the emitted code, taken only with the `depth` option.
const BUDGET: &str = "depth";

/// The suppression every top-level item carries: the generated module is private, so
/// `pub` would trip `unreachable_pub` and `pub(crate)` trips `redundant_pub_crate`
/// (the precedent of `crate::modbus::session::Wire`).
const VISIBILITY_EXPECT: &str = "#[expect(clippy::redundant_pub_crate, reason = \"unreachable_pub forbids plain pub in a private module\")]";

/// Generates every enabled type: a map from module name to its unformatted body (no
/// header). The module names are the snake-case type names.
///
/// # Errors
/// A [`GenError`] for the first unsupported construct or inconsistent option.
pub(crate) fn generate(
    specs: &[Spec],
    options: &Options,
) -> Result<BTreeMap<String, String>, GenError> {
    let case_structs = case_structs(specs);
    let mut ctx = Ctx::new(specs, &case_structs, options)?;
    ctx.prepare()?;
    // The enabled types, and the cases that an enabled field refers to as a type.
    let names: BTreeSet<&String> = options.types.iter().chain(&ctx.implicit).collect();
    let mut modules = BTreeMap::new();
    for name in names {
        let def = ctx.def(name, 0, name)?;
        let body = match def.form {
            TypeForm::Enum => ctx.emit_enum(def)?,
            TypeForm::Discriminated => ctx.emit_discriminated(def)?,
            TypeForm::DataIo => ctx.emit_data_io(def)?,
            TypeForm::Type => ctx.emit_struct(def)?,
        };
        let module = snake(name);
        if modules.insert(module.clone(), body).is_some() {
            return Err(gen_error(
                def,
                def.line,
                &format!("module name `{module}` collides"),
            ));
        }
    }
    Ok(modules)
}

/// A case of a `discriminatedType`, as the `type` it becomes when a field refers to it by
/// name (`[simple RequestHeader('391') requestHeader]`): upstream's case class takes the
/// parent's parameters, which the reference supplies.
struct CaseStruct<'a> {
    /// The type the case belongs to.
    parent: &'a TypeDef,
    /// The case.
    case: &'a Case,
    /// The case as a `type`: its fields, with the parent's parameters and attributes.
    def: TypeDef,
}

/// The cases of the top-level `typeSwitch` of every `discriminatedType`, named as
/// `type_names` names them (`*Name` takes the parent's name as a prefix).
fn case_structs(specs: &[Spec]) -> Vec<CaseStruct<'_>> {
    let mut found = Vec::new();
    for spec in specs {
        for parent in &spec.types {
            if parent.form != TypeForm::Discriminated {
                continue;
            }
            let Ok(cases) = switch_cases(parent) else {
                continue;
            };
            for case in cases {
                let name = if case.name_wildcard {
                    format!("{}{}", parent.name, case.name)
                } else {
                    case.name.clone()
                };
                found.push(CaseStruct {
                    parent,
                    case,
                    def: TypeDef {
                        form: TypeForm::Type,
                        name,
                        params: parent.params.clone(),
                        attributes: parent.attributes.clone(),
                        line: case.line,
                        fields: case.fields.clone(),
                        enum_type: None,
                        values: Vec::new(),
                        data_io_params: Vec::new(),
                    },
                });
            }
        }
    }
    found
}

fn gen_error(def: &TypeDef, line: usize, message: &str) -> GenError {
    GenError {
        line,
        type_name: def.name.clone(),
        message: message.to_owned(),
    }
}

/// A Rust type of a field, a parameter or a local.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ty {
    Bool,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    /// A generated enum, by mspec name.
    Enum(String),
    /// A generated discriminated type, by mspec name.
    Complex(String),
    /// A byte array.
    Bytes,
    /// An array of any other element: `Vec<T>`, or `Option<Vec<T>>` when its count can be -1
    /// (null), which is distinct from an empty array.
    Array {
        elem: Box<Self>,
        nullable: bool,
    },
}

impl Ty {
    fn rust(&self) -> String {
        match self {
            Self::Bool => "bool".to_owned(),
            Self::U8 => "u8".to_owned(),
            Self::U16 => "u16".to_owned(),
            Self::U32 => "u32".to_owned(),
            Self::U64 => "u64".to_owned(),
            Self::I8 => "i8".to_owned(),
            Self::I16 => "i16".to_owned(),
            Self::I32 => "i32".to_owned(),
            Self::I64 => "i64".to_owned(),
            Self::F32 => "f32".to_owned(),
            Self::F64 => "f64".to_owned(),
            Self::Enum(name) | Self::Complex(name) => upper_camel(name),
            Self::Bytes => "Vec<u8>".to_owned(),
            Self::Array {
                elem,
                nullable: false,
            } => format!("Vec<{}>", elem.rust()),
            Self::Array {
                elem,
                nullable: true,
            } => format!("Option<Vec<{}>>", elem.rust()),
        }
    }

    /// The generated type this type names, through the elements of an array: what the module
    /// imports from its siblings.
    fn sibling(&self) -> Option<&str> {
        match self {
            Self::Enum(name) | Self::Complex(name) => Some(name),
            Self::Array { elem, .. } => elem.sibling(),
            Self::Bool
            | Self::U8
            | Self::U16
            | Self::U32
            | Self::U64
            | Self::I8
            | Self::I16
            | Self::I32
            | Self::I64
            | Self::F32
            | Self::F64
            | Self::Bytes => None,
        }
    }

    /// Whether a float is stored, directly or as an element: the type is not `Eq`.
    fn has_float(&self) -> bool {
        match self {
            Self::F32 | Self::F64 => true,
            Self::Array { elem, .. } => elem.has_float(),
            Self::Bool
            | Self::U8
            | Self::U16
            | Self::U32
            | Self::U64
            | Self::I8
            | Self::I16
            | Self::I32
            | Self::I64
            | Self::Enum(_)
            | Self::Complex(_)
            | Self::Bytes => false,
        }
    }

    /// The `BoundedReader` and `BoundedWriter` method suffix of a byte-aligned scalar, and
    /// whether it takes a byte order.
    const fn accessor(&self) -> Option<(&'static str, bool)> {
        match self {
            Self::U8 => Some(("u8", false)),
            Self::I8 => Some(("i8", false)),
            Self::U16 => Some(("u16", true)),
            Self::U32 => Some(("u32", true)),
            Self::U64 => Some(("u64", true)),
            Self::I16 => Some(("i16", true)),
            Self::I32 => Some(("i32", true)),
            Self::I64 => Some(("i64", true)),
            Self::F32 => Some(("f32", true)),
            Self::F64 => Some(("f64", true)),
            Self::Bool | Self::Enum(_) | Self::Complex(_) | Self::Bytes | Self::Array { .. } => {
                None
            }
        }
    }

    /// The `PlcValue` variant a `dataIo` field of this type becomes.
    const fn plc_value(&self) -> Option<&'static str> {
        match self {
            Self::U8 => Some("U8"),
            Self::U16 => Some("U16"),
            Self::U32 => Some("U32"),
            Self::U64 => Some("U64"),
            Self::I8 => Some("I8"),
            Self::I16 => Some("I16"),
            Self::I32 => Some("I32"),
            Self::I64 => Some("I64"),
            Self::F32 => Some("F32"),
            Self::F64 => Some("F64"),
            Self::Bool | Self::Enum(_) | Self::Complex(_) | Self::Bytes | Self::Array { .. } => {
                None
            }
        }
    }

    /// Whether the value converts to `u64` with `From` (for error values and checks).
    const fn widens_to_u64(&self) -> bool {
        matches!(
            self,
            Self::Bool | Self::U8 | Self::U16 | Self::U32 | Self::U64
        )
    }

    /// Whether the value converts to `usize` with `From` (for expressions).
    const fn widens_to_usize(&self) -> bool {
        matches!(self, Self::U8 | Self::U16)
    }

    /// Whether the integer literal `value` fits this type; `false` for a type that is not an
    /// integer.
    fn holds(&self, value: i128) -> bool {
        let (smallest, largest) = match self {
            Self::U8 => (0, i128::from(u8::MAX)),
            Self::U16 => (0, i128::from(u16::MAX)),
            Self::U32 => (0, i128::from(u32::MAX)),
            Self::U64 => (0, i128::from(u64::MAX)),
            Self::I8 => (i128::from(i8::MIN), i128::from(i8::MAX)),
            Self::I16 => (i128::from(i16::MIN), i128::from(i16::MAX)),
            Self::I32 => (i128::from(i32::MIN), i128::from(i32::MAX)),
            Self::I64 => (i128::from(i64::MIN), i128::from(i64::MAX)),
            Self::Bool
            | Self::F32
            | Self::F64
            | Self::Enum(_)
            | Self::Complex(_)
            | Self::Bytes
            | Self::Array { .. } => return false,
        };
        (smallest..=largest).contains(&value)
    }

    /// Whether an `implicit` field may have this type: an unsigned number up to 32 bits, or
    /// `int 32` (an OPC UA array count, where -1 is null).
    const fn is_implicit_count(&self) -> bool {
        matches!(self, Self::U8 | Self::U16 | Self::U32 | Self::I32)
    }
}

/// A base data type as a Rust type and its width in bits.
fn scalar(ty: &DataType) -> Option<(Ty, u32)> {
    let size = ty.size.unwrap_or(0);
    Some(match (ty.base, size) {
        (Base::Bit, _) => (Ty::Bool, 1),
        (Base::Byte, _) => (Ty::U8, 8),
        (Base::Uint, 1..=8) => (Ty::U8, size),
        (Base::Uint, 16) => (Ty::U16, 16),
        (Base::Uint, 32) => (Ty::U32, 32),
        (Base::Uint, 64) => (Ty::U64, 64),
        (Base::Int, 8) => (Ty::I8, 8),
        (Base::Int, 16) => (Ty::I16, 16),
        (Base::Int, 32) => (Ty::I32, 32),
        (Base::Int, 64) => (Ty::I64, 64),
        (Base::Float, 32) => (Ty::F32, 32),
        (Base::Float, 64) => (Ty::F64, 64),
        _ => return None,
    })
}

fn describe(ty: &DataType) -> String {
    let keyword = Base::ALL
        .iter()
        .find(|(base, _)| *base == ty.base)
        .map_or("?", |(_, keyword)| *keyword);
    ty.size
        .map_or_else(|| keyword.to_owned(), |size| format!("{keyword} {size}"))
}

/// The value of a discriminator in one case: a literal, or the stored field.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DiscValue {
    /// The case's value, as a Rust pattern or literal.
    Literal(String),
    /// Not matched by the case: stored in the variant.
    Stored,
}

/// A switch discriminator: a parameter or a discriminator field of the parent.
#[derive(Debug, Clone)]
struct Discriminator {
    name: String,
    ty: Ty,
    is_field: bool,
}

/// What a name in an expression refers to.
#[derive(Debug, Clone)]
struct Binding {
    ty: Ty,
    /// In `serialize`, fields are references and scalars need `*`.
    by_ref: bool,
}

/// Imports a module needs, collected while its body is written.
#[derive(Debug, Default)]
struct Imports {
    codec: BTreeSet<&'static str>,
    byte_order: bool,
    plc_value: bool,
    /// Whether a function of the module takes the recursion budget.
    depth: bool,
    support: BTreeSet<&'static str>,
    /// The types of the protocol the module names: generated ones are imported from their
    /// sibling module, external ones from their path.
    siblings: BTreeSet<String>,
}

struct Ctx<'a> {
    types: BTreeMap<&'a str, &'a TypeDef>,
    /// Names defined more than once, with the lines of the first two definitions.
    duplicates: BTreeMap<&'a str, (usize, usize)>,
    /// The cases that no type shares a name with, as the `type` a reference to them makes,
    /// each with the type it belongs to.
    case_types: BTreeMap<&'a str, &'a CaseStruct<'a>>,
    /// Case names that more than one `discriminatedType` defines.
    ambiguous_cases: BTreeSet<&'a str>,
    constants: BTreeSet<&'a str>,
    options: &'a Options,
    /// Enums read from and written to the wire, which get `parse` and `serialize`.
    wire_enums: BTreeSet<String>,
    /// Types whose `lengthInBytes` an expression reads.
    sized: BTreeSet<String>,
    /// The cases that an enabled field refers to by name: generated as structs of their own
    /// whether or not the options enable them as variants of their parent.
    implicit: BTreeSet<String>,
}

impl<'a> Ctx<'a> {
    fn new(
        specs: &'a [Spec],
        case_structs: &'a [CaseStruct<'a>],
        options: &'a Options,
    ) -> Result<Self, GenError> {
        let mut types: BTreeMap<&str, &TypeDef> = BTreeMap::new();
        let mut duplicates = BTreeMap::new();
        let mut constants = BTreeSet::new();
        for spec in specs {
            if let Some(block) = spec.globals.as_ref().or(spec.context.as_ref()) {
                return Err(GenError {
                    line: block.line,
                    type_name: "(file)".to_owned(),
                    message: "a globals or context block is parsed but not emitted yet".to_owned(),
                });
            }
            for field in spec.constants.iter().flat_map(|block| &block.fields) {
                if let FieldKind::Const { name, .. } = &field.kind {
                    constants.insert(name.as_str());
                }
            }
            for def in &spec.types {
                if let Some(first) = types.insert(def.name.as_str(), def) {
                    duplicates
                        .entry(def.name.as_str())
                        .or_insert((first.line, def.line));
                }
            }
        }
        let mut case_types: BTreeMap<&str, &CaseStruct<'_>> = BTreeMap::new();
        let mut ambiguous_cases = BTreeSet::new();
        for found in case_structs {
            let name = found.def.name.as_str();
            if types.contains_key(name) {
                continue;
            }
            if case_types.insert(name, found).is_some() {
                ambiguous_cases.insert(name);
            }
        }
        for name in &ambiguous_cases {
            case_types.remove(name);
        }
        Ok(Self {
            types,
            duplicates,
            case_types,
            ambiguous_cases,
            constants,
            options,
            wire_enums: BTreeSet::new(),
            sized: BTreeSet::new(),
            implicit: BTreeSet::new(),
        })
    }

    /// The budget as the extra argument of a nested `parse` or `serialize` call, which is
    /// `, depth` when the protocol has a budget type and nothing otherwise.
    const fn depth_arg(&self) -> &'static str {
        if self.options.depth.is_some() {
            ", depth"
        } else {
            ""
        }
    }

    /// The budget as a parameter of a generated function: `, depth: Depth`, or the
    /// underscored name when the body never passes it on; nothing without a budget type.
    fn depth_parameter(&self, used: bool) -> String {
        match (self.options.depth.is_some(), used) {
            (false, _) => String::new(),
            (true, true) => ", depth: Depth".to_owned(),
            (true, false) => ", _depth: Depth".to_owned(),
        }
    }

    /// Whether a nested value of this type can say how long it is: a generated type whose
    /// length an expression reads, or an external type, which always can.
    fn has_length(&self, type_name: &str) -> bool {
        self.sized.contains(type_name) || self.options.external.contains_key(type_name)
    }

    /// The budget as the whole argument list of a `length_in_bytes` call.
    const fn depth_name(&self) -> &'static str {
        if self.options.depth.is_some() {
            "depth"
        } else {
            ""
        }
    }

    /// The byte order of every multi-byte read and write: the protocol's, `BIG_ENDIAN`
    /// unless its options say otherwise.
    fn order(&self) -> &'static str {
        self.options.byte_order.unwrap_or(Endianness::Big).rust()
    }

    /// The definition of `name`, which must be enabled (or external, or a case an enabled
    /// field refers to) and defined once. Two definitions of a name are ambiguous, and the
    /// generator will not pick one, so only a duplicate the output would use is an error.
    fn def(&self, name: &str, line: usize, from: &str) -> Result<&'a TypeDef, GenError> {
        let Some(def) = self.types.get(name).copied().or_else(|| {
            self.case_types
                .get(name)
                .map(|found: &&'a CaseStruct<'a>| &found.def)
        }) else {
            if self.ambiguous_cases.contains(name) {
                return Err(GenError {
                    line,
                    type_name: from.to_owned(),
                    message: format!(
                        "`{name}` is a case of more than one type; the generator does not pick one"
                    ),
                });
            }
            return Err(GenError {
                line,
                type_name: from.to_owned(),
                message: format!("type `{name}` is not defined"),
            });
        };
        if let Some((first, second)) = self.duplicates.get(name) {
            return Err(GenError {
                line: *second,
                type_name: name.to_owned(),
                message: format!(
                    "type `{name}` is defined twice (lines {first} and {second}); the generator does not pick one"
                ),
            });
        }
        if !(self.options.types.contains(name)
            || self.options.external.contains_key(name)
            || self.implicit.contains(name))
        {
            return Err(GenError {
                line,
                type_name: from.to_owned(),
                message: format!(
                    "refers to type `{name}`, which the options do not enable and do not list under [external]"
                ),
            });
        }
        Ok(def)
    }

    fn enabled_cases(&self, def: &'a TypeDef) -> Result<Vec<(&'a Case, bool)>, GenError> {
        let cases = switch_cases(def)?;
        let enabled = self.options.cases.get(&def.name);
        for name in enabled.into_iter().flatten() {
            if !cases.iter().any(|case| &case.name == name) {
                return Err(gen_error(
                    def,
                    def.line,
                    &format!("the options enable case `{name}`, which the type does not have"),
                ));
            }
        }
        Ok(cases
            .into_iter()
            .map(|case| {
                let on = enabled.is_some_and(|names| names.contains(&case.name));
                (case, on)
            })
            .collect())
    }

    /// The field lists the output reads for `def`: a struct's own, or those of the enabled
    /// cases of a `discriminatedType` or `dataIo`.
    fn field_lists(&self, def: &'a TypeDef) -> Result<Vec<&'a [Item]>, GenError> {
        Ok(match def.form {
            TypeForm::Type => vec![def.fields.as_slice()],
            TypeForm::Discriminated | TypeForm::DataIo => self
                .enabled_cases(def)?
                .into_iter()
                .filter(|(_, on)| *on)
                .map(|(case, _)| case.fields.as_slice())
                .collect(),
            TypeForm::Enum => Vec::new(),
        })
    }

    /// Checks the `[external]` table and the `depth` type against the mspec files.
    fn check_external(&self) -> Result<(), GenError> {
        let options_error = |type_name: &str, message: String| GenError {
            line: 0,
            type_name: type_name.to_owned(),
            message,
        };
        if let Some(path) = &self.options.depth
            && !is_rust_path(path)
        {
            return Err(options_error(
                "(options)",
                format!("`depth` = \"{path}\" is not a Rust path"),
            ));
        }
        if !self.options.external.is_empty() && self.options.depth.is_none() {
            return Err(options_error(
                "(options)",
                "an [external] table needs `depth`, the budget type of the external signature"
                    .to_owned(),
            ));
        }
        for (name, external) in &self.options.external {
            let missing =
                |what: &str| options_error(name, format!("external type `{name}` {what}"));
            if self.options.types.contains(name) {
                return Err(missing(
                    "is also enabled in `types`; it is either generated or external",
                ));
            }
            if !is_rust_path(&external.path) {
                return Err(missing(&format!(
                    "maps to `{}`, which is not a Rust path",
                    external.path
                )));
            }
            let Some(def) = self.types.get(name.as_str()).copied() else {
                return Err(missing("is not defined by the mspec files"));
            };
            if let Some((first, second)) = self.duplicates.get(name.as_str()) {
                return Err(missing(&format!(
                    "is defined twice (lines {first} and {second}); the generator does not pick one"
                )));
            }
            if !matches!(def.form, TypeForm::Type | TypeForm::Discriminated) {
                return Err(missing(&format!(
                    "is {} `{name}`, which a hand-written type cannot stand for",
                    def.form.keyword()
                )));
            }
            if external.arguments.len() != def.params.len() {
                return Err(missing(&format!(
                    "takes {} parameters in the mspec (line {}) and the table pins {}",
                    def.params.len(),
                    def.line,
                    external.arguments.len()
                )));
            }
            for (param, argument) in def.params.iter().zip(&external.arguments) {
                let ty = match &param.ty {
                    TypeRef::Simple(data_type) => scalar(data_type).map(|(ty, _)| ty),
                    TypeRef::Complex { .. } => None,
                };
                let fits = match (ty.as_ref(), argument.as_str()) {
                    (Some(Ty::Bool), "true" | "false") => true,
                    (Some(Ty::Bool) | None, _) => false,
                    (Some(ty), text) => match text_literal(text) {
                        Some(Lit::Int(value)) => ty.holds(value),
                        Some(Lit::Bool(_)) | None => false,
                    },
                };
                if !fits {
                    return Err(missing(&format!(
                        "pins `{argument}` for the parameter `{}`, which is not a literal of its type",
                        param.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// Finds the cases that an enabled field names as a type, and the cases they name in
    /// turn: each is generated as a struct of its own.
    fn find_implicit_cases(&mut self) -> Result<(), GenError> {
        let mut implicit = BTreeSet::new();
        let mut pending: Vec<&'a TypeDef> = self
            .options
            .types
            .iter()
            .filter_map(|name| self.types.get(name.as_str()).copied())
            .collect();
        while let Some(def) = pending.pop() {
            for items in self.field_lists(def)? {
                for field in fields_of(items) {
                    let (FieldKind::Simple {
                        ty: TypeRef::Complex { name, .. },
                        ..
                    }
                    | FieldKind::Array {
                        ty: TypeRef::Complex { name, .. },
                        ..
                    }) = &field.kind
                    else {
                        continue;
                    };
                    let Some(found) = self.case_types.get(name.as_str()).copied() else {
                        continue;
                    };
                    if implicit.insert(name.clone()) {
                        check_case_reference(found, field.line, &def.name)?;
                        pending.push(&found.def);
                    }
                }
            }
        }
        self.implicit = implicit;
        Ok(())
    }

    /// Checks the options and finds the enums read from the wire and the types whose
    /// length an expression reads.
    fn prepare(&mut self) -> Result<(), GenError> {
        for type_name in self.options.cases.keys() {
            if !self.options.types.contains(type_name) {
                return Err(GenError {
                    line: 0,
                    type_name: type_name.clone(),
                    message: "the options list cases of a type they do not enable".to_owned(),
                });
            }
        }
        self.check_external()?;
        self.find_implicit_cases()?;
        let mut wire = BTreeSet::new();
        let mut sized = BTreeSet::new();
        let mut nested: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
        let names: Vec<String> = self
            .options
            .types
            .iter()
            .chain(&self.implicit)
            .cloned()
            .collect();
        for name in &names {
            let def = self.def(name, 0, name)?;
            check_attributes(def, self.options.byte_order.unwrap_or(Endianness::Big))?;
            for items in self.field_lists(def)? {
                for field in fields_of(items) {
                    if let FieldKind::Simple {
                        ty: TypeRef::Complex { name, .. },
                        ..
                    }
                    | FieldKind::Array {
                        ty: TypeRef::Complex { name, .. },
                        ..
                    } = &field.kind
                    {
                        if self.def(name, field.line, &def.name)?.form == TypeForm::Enum {
                            wire.insert(name.clone());
                        } else if !self.options.external.contains_key(name) {
                            // An external type is a leaf here: a cycle through one is the
                            // external's to bound, with the budget it is handed.
                            nested
                                .entry(def.name.clone())
                                .or_default()
                                .push((name.clone(), field.line));
                        }
                    }
                    for expr in field_exprs(&field.kind) {
                        collect_sized(expr, items, &mut sized);
                    }
                }
            }
        }
        for name in nested.keys() {
            check_recursion(name, &nested, &mut Vec::new())?;
        }
        self.wire_enums = wire;
        self.sized = sized;
        Ok(())
    }
}

/// Fails unless a case that a field names as a type can stand alone: the reference supplies
/// the parent's parameters and nothing else, so the parent reads nothing before its
/// `typeSwitch`, and the case has no parameters of its own.
fn check_case_reference(found: &CaseStruct<'_>, line: usize, from: &str) -> Result<(), GenError> {
    let refuse = |message: String| GenError {
        line,
        type_name: from.to_owned(),
        message,
    };
    let name = &found.def.name;
    if fields_of(&found.parent.fields).len() != 1 {
        return Err(refuse(format!(
            "`{name}` is a case of `{}`, which reads fields before its typeSwitch; a case is a type of its own only when its parent reads none (feature matrix)",
            found.parent.name
        )));
    }
    if !found.case.params.is_empty() {
        return Err(refuse(format!(
            "`{name}` has case parameters and is named as a type; the reference passes only the parent's parameters (feature matrix)"
        )));
    }
    Ok(())
}

/// Whether `text` is a path of identifiers (`crate::opcua::codec::Variant`) that can be
/// written in a `use`: at least two segments, none empty, none a keyword except a leading
/// `crate`, `self` or `super`.
fn is_rust_path(text: &str) -> bool {
    let segments: Vec<&str> = text.split("::").collect();
    segments.len() >= 2
        && segments.iter().enumerate().all(|(index, segment)| {
            let mut chars = segment.chars();
            chars
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
                && (!is_keyword(segment)
                    || (index == 0 && ["crate", "self", "super"].contains(segment)))
        })
}

/// Fails when a type reaches itself through the nested fields of its enabled cases
/// (ADR-0019 decision 5): a recursive type needs a depth argument and
/// `ParseError::RecursionLimit`, which the first protocol that has one adds.
fn check_recursion(
    name: &str,
    nested: &BTreeMap<String, Vec<(String, usize)>>,
    path: &mut Vec<String>,
) -> Result<(), GenError> {
    path.push(name.to_owned());
    for (inner, line_no) in nested.get(name).into_iter().flatten() {
        if path.contains(inner) {
            return Err(GenError {
                line: *line_no,
                type_name: name.to_owned(),
                message: format!(
                    "recursive type: {} -> {inner}; a cycle must pass through an [external] type",
                    path.join(" -> ")
                ),
            });
        }
        check_recursion(inner, nested, path)?;
    }
    path.pop();
    Ok(())
}

/// Merges arms whose bodies are the same text, at the first one's place, so clippy's
/// `match_same_arms` has nothing to report. The callers' patterns never overlap (enum
/// variants, or one enum value per arm), so merging keeps the meaning.
fn merge_arms(arms: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut merged: Vec<(Vec<String>, String)> = Vec::new();
    for (pattern, body) in arms {
        if let Some((patterns, _)) = merged.iter_mut().find(|(_, existing)| *existing == body) {
            patterns.push(pattern);
        } else {
            merged.push((vec![pattern], body));
        }
    }
    merged
        .into_iter()
        .map(|(patterns, body)| (patterns.join(" | "), body))
        .collect()
}

/// `match scrutinee { arms }`; an empty scrutinee means `self` was matched by the caller's
/// already opened `match self {`.
fn emit_match(out: &mut String, scrutinee: &str, arms: &[(String, String)]) {
    if !scrutinee.is_empty() {
        line(out, &format!("match {scrutinee} {{"));
    }
    for (pattern, body) in arms {
        line(out, &format!("{pattern} => {body},"));
    }
    line(out, "}");
}

/// The case list of a `discriminatedType` or `dataIo`.
fn switch_cases(def: &TypeDef) -> Result<Vec<&Case>, GenError> {
    for field in fields_of(&def.fields) {
        if let FieldKind::TypeSwitch { cases, .. } = &field.kind {
            return Ok(cases.iter().collect());
        }
    }
    Err(gen_error(def, def.line, "no typeSwitch"))
}

/// The expressions of a field kind the prepare pass looks into.
fn field_exprs(kind: &FieldKind) -> Vec<&Expr> {
    if let FieldKind::Implicit { expr, .. } | FieldKind::Array { expr, .. } = kind
        && let MExpr::Expr(expr, _) = expr
    {
        vec![expr]
    } else {
        Vec::new()
    }
}

/// Finds `field.lengthInBytes` in `expr` and records the field's type.
fn collect_sized(expr: &Expr, fields: &[Item], sized: &mut BTreeSet<String>) {
    match expr {
        Expr::Var(var) => {
            if var
                .rest
                .as_ref()
                .is_some_and(|rest| rest.name == "lengthInBytes")
            {
                for field in fields_of(fields) {
                    if let FieldKind::Simple {
                        ty: TypeRef::Complex { name, .. },
                        name: field_name,
                    } = &field.kind
                        && *field_name == var.name
                    {
                        sized.insert(name.clone());
                    }
                }
            }
            for arg in var.args.iter().flatten() {
                collect_sized(arg, fields, sized);
            }
        }
        Expr::Unary(_, inner) | Expr::Group(inner, _) => collect_sized(inner, fields, sized),
        Expr::Binary(_, lhs, rhs) => {
            collect_sized(lhs, fields, sized);
            collect_sized(rhs, fields, sized);
        }
        Expr::Ternary(a, b, c) => {
            collect_sized(a, fields, sized);
            collect_sized(b, fields, sized);
            collect_sized(c, fields, sized);
        }
        Expr::Number(_) | Expr::Hex(_) | Expr::Bool(_) | Expr::Null | Expr::Str(..) => {}
    }
}

fn check_attributes(def: &TypeDef, order: Endianness) -> Result<(), GenError> {
    check_batch_sets(def, &def.fields)?;
    for attribute in &def.attributes {
        let value = if let MExpr::Expr(Expr::Str(text, indexes), _) = &attribute.value
            && indexes.is_empty()
        {
            text.as_str()
        } else {
            ""
        };
        if attribute.name == "byteOrder" {
            // A per-type byte order waits for a protocol that needs one: the protocol's
            // order is the only one the output uses.
            if Endianness::from_keyword(value).is_none() {
                return Err(gen_error(
                    def,
                    def.line,
                    &format!("attribute byteOrder='{value}' is not a byte order"),
                ));
            }
            if value != order.keyword() {
                return Err(gen_error(
                    def,
                    def.line,
                    &format!(
                        "attribute byteOrder='{value}' differs from the protocol's byte order {}; a per-type byte order is not emitted (feature matrix)",
                        order.keyword()
                    ),
                ));
            }
            continue;
        }
        let known = ATTRIBUTES
            .iter()
            .find(|(name, _)| *name == attribute.name)
            .is_some_and(|(_, values)| values.contains(&value));
        if !known {
            return Err(gen_error(
                def,
                def.line,
                &format!(
                    "attribute {}='{value}' is not emitted (feature matrix)",
                    attribute.name
                ),
            ));
        }
    }
    Ok(())
}

/// Fails on a `batchSet` that carries attributes, in the type's fields or in any case:
/// the emitter flattens batch sets, so a `byteOrder` there would be dropped silently.
fn check_batch_sets(def: &TypeDef, items: &[Item]) -> Result<(), GenError> {
    for item in items {
        if let Item::BatchSet {
            line, attributes, ..
        } = item
            && !attributes.is_empty()
        {
            return Err(gen_error(
                def,
                *line,
                "batchSet attributes are not emitted (feature matrix)",
            ));
        }
    }
    for field in fields_of(items) {
        if let FieldKind::TypeSwitch { cases, .. } = &field.kind {
            for case in cases {
                check_batch_sets(def, &case.fields)?;
            }
        }
    }
    Ok(())
}

/// Fails on a name the emitted code needs: `budgeted` is whether the protocol has the `depth`
/// option, which makes `depth` the recursion budget parameter.
fn check_name(
    def: &TypeDef,
    line_no: usize,
    rust: &str,
    original: &str,
    budgeted: bool,
) -> Result<(), GenError> {
    if rust.is_empty()
        || is_keyword(rust)
        || RESERVED.contains(&rust)
        || (budgeted && rust == BUDGET)
        || rust == "bits"
        || rust.starts_with("bits_")
    {
        return Err(gen_error(
            def,
            line_no,
            &format!("name `{original}` becomes `{rust}`, which is a keyword or reserved"),
        ));
    }
    Ok(())
}

/// A literal argument or case value: a boolean or an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    variant_size_differences,
    reason = "an i128 holds every u64 and i64 literal; a bool is the other kind"
)]
enum Lit {
    Bool(bool),
    Int(i128),
}

/// The literal an expression is: `true`, `false`, an integer, or a negated one.
fn literal_value(expr: &Expr) -> Option<Lit> {
    match expr {
        Expr::Bool(value) => Some(Lit::Bool(*value)),
        Expr::Number(text) | Expr::Hex(text) => parse_integer(text)
            .and_then(|value| i128::try_from(value).ok())
            .map(Lit::Int),
        Expr::Unary(UnaryOp::Neg, inner) => match literal_value(inner)? {
            Lit::Int(value) => value.checked_neg().map(Lit::Int),
            Lit::Bool(_) => None,
        },
        Expr::Group(inner, indexes) if indexes.is_empty() => literal_value(inner),
        Expr::Null
        | Expr::Str(..)
        | Expr::Var(_)
        | Expr::Unary(..)
        | Expr::Binary(..)
        | Expr::Ternary(..)
        | Expr::Group(..) => None,
    }
}

/// The literal a pinned argument of the options is.
fn text_literal(text: &str) -> Option<Lit> {
    match text {
        "true" => Some(Lit::Bool(true)),
        "false" => Some(Lit::Bool(false)),
        _ => parse_integer(text)
            .and_then(|value| i128::try_from(value).ok())
            .map(Lit::Int),
    }
}

/// An integer written as an enum value or constant, and its value.
fn integer_text(raw: &str) -> Option<(String, u128)> {
    parse_integer(raw).map(|value| (raw.to_owned(), value))
}

fn integer_literal(expr: &Expr) -> Option<(String, u128)> {
    if let Expr::Number(text) | Expr::Hex(text) = expr {
        parse_integer(text).map(|value| (text.clone(), value))
    } else {
        None
    }
}

/// A decimal integer as Rust source with `_` every three digits from six digits up, the form
/// `clippy::unreadable_literal` accepts (`16_909_060`).
fn decimal(value: impl fmt::Display) -> String {
    let text = value.to_string();
    let (sign, digits) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |digits| ("-", digits));
    if digits.len() < 6 {
        return text;
    }
    // Groups of three counted from the right: chunk the reversed digits, then reverse back.
    let reversed: Vec<char> = digits.chars().rev().collect();
    let groups: Vec<String> = reversed
        .chunks(3)
        .map(|chunk| chunk.iter().rev().collect())
        .collect();
    let mut grouped = String::from(sign);
    grouped.push_str(&groups.into_iter().rev().collect::<Vec<_>>().join("_"));
    grouped
}

fn parse_integer(text: &str) -> Option<u128> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u128::from_str_radix(hex, 16).ok()
    } else if let Some(binary) = text.strip_prefix("0b") {
        u128::from_str_radix(binary, 2).ok()
    } else {
        text.parse::<u128>().ok()
    }
}

/// Fails unless `value` fits the field width (`uint 7` holds at most 127).
fn check_fits(
    def: &TypeDef,
    line_no: usize,
    value: u128,
    ty: &Ty,
    data_type: Option<&DataType>,
) -> Result<(), GenError> {
    let bits = data_type
        .and_then(scalar)
        .map_or(0, |(_, bits)| bits)
        .min(match ty {
            Ty::U8 => 8,
            Ty::U16 => 16,
            Ty::U32 => 32,
            Ty::U64 => 64,
            Ty::Bool
            | Ty::I8
            | Ty::I16
            | Ty::I32
            | Ty::I64
            | Ty::F32
            | Ty::F64
            | Ty::Enum(_)
            | Ty::Complex(_)
            | Ty::Bytes
            | Ty::Array { .. } => 0,
        });
    let limit = 1_u128.checked_shl(bits).unwrap_or(0);
    if value >= limit {
        return Err(gen_error(
            def,
            line_no,
            &format!("value {value} does not fit {bits} bits"),
        ));
    }
    Ok(())
}

fn line(out: &mut String, text: &str) {
    out.push_str(text);
    out.push('\n');
}

/// A module body: its imports, then its items.
fn module(imports: &Imports, options: &Options, items: &str) -> String {
    let mut text = imports.render(options);
    text.push_str(items);
    text
}

impl Imports {
    fn render(&self, options: &Options) -> String {
        let mut out = String::new();
        if self.byte_order {
            line(&mut out, "use crate::ByteOrder;");
        }
        if self.plc_value {
            line(&mut out, "use crate::PlcValue;");
        }
        if !self.codec.is_empty() {
            let names: Vec<&str> = self.codec.iter().copied().collect();
            line(
                &mut out,
                &format!("use crate::codec::{{{}}};", names.join(", ")),
            );
        }
        if !self.support.is_empty() {
            let names: Vec<&str> = self.support.iter().copied().collect();
            line(
                &mut out,
                &format!("use {}::{{{}}};", options.support_module, names.join(", ")),
            );
        }
        if let (true, Some(path)) = (self.depth, &options.depth) {
            line(&mut out, &import_line(path, "Depth"));
        }
        let mut generated = Vec::new();
        for sibling in &self.siblings {
            match options.external.get(sibling) {
                Some(external) => {
                    line(
                        &mut out,
                        &import_line(&external.path, &upper_camel(sibling)),
                    );
                }
                None => generated.push(sibling),
            }
        }
        if !out.is_empty() && !generated.is_empty() {
            line(&mut out, "");
        }
        for sibling in generated {
            line(
                &mut out,
                &format!("use super::{}::{};", snake(sibling), upper_camel(sibling)),
            );
        }
        if !out.is_empty() {
            line(&mut out, "");
        }
        out
    }
}

/// `use path;`, or `use path as name;` when the path does not end in the name the output
/// uses for the type.
fn import_line(path: &str, name: &str) -> String {
    if path.rsplit("::").next() == Some(name) {
        format!("use {path};")
    } else {
        format!("use {path} as {name};")
    }
}

/// Whether `text` names the recursion budget (`depth`, which no field may take) anywhere.
fn mentions_depth(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| word == "depth")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{Options, generate};
    use crate::mspec::{parse, resolve, type_names};

    fn options(types: &[&str], cases: &[(&str, &[&str])]) -> Options {
        Options {
            support_module: "crate::support".to_owned(),
            byte_order: None,
            types: types.iter().map(|name| (*name).to_owned()).collect(),
            cases: cases
                .iter()
                .map(|(name, list)| {
                    (
                        (*name).to_owned(),
                        list.iter()
                            .map(|case| (*case).to_owned())
                            .collect::<BTreeSet<_>>(),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
            depth: None,
            external: BTreeMap::new(),
        }
    }

    /// Generates `text` with `options`, or returns the generation error as text.
    fn run(text: &str, options: &Options) -> Result<BTreeMap<String, String>, String> {
        let spec = parse(text).map_err(|error| format!("parse: {error}"))?;
        resolve(&spec, &type_names(&[&spec])).map_err(|error| format!("resolve: {error}"))?;
        generate(&[spec], options).map_err(|error| error.to_string())
    }

    const PDU: &str = "
[discriminatedType Pdu(bit response)
    [discriminator bit flag]
    [discriminator uint 7 code]
    [typeSwitch flag,code,response
        ['true' Failure [simple Code reason]]
        ['false','0x01','false' Ask [simple uint 16 start]]
        ['false','0x01','true' Answer
            [implicit uint 8 count 'COUNT(data)']
            [array byte data count 'count']
        ]
        ['false','0x02','false' Other [virtual uint 8 v 'code + 1']]
    ]
]
[enum uint 8 Code ['1' ONE] ['2' TWO]]
";

    #[test]
    fn the_output_is_the_same_for_the_same_input() {
        let options = options(&["Code", "Pdu"], &[("Pdu", &["Failure", "Ask", "Answer"])]);
        let first = run(PDU, &options).expect("generates");
        assert_eq!(run(PDU, &options), Ok(first.clone()), "deterministic");
        assert_eq!(
            first.keys().map(String::as_str).collect::<Vec<_>>(),
            ["code", "pdu"],
            "one module per type, sorted"
        );
        let pdu = first.get("pdu").expect("the pdu module");
        assert!(
            pdu.contains("field: \"Pdu.Other (disabled)\""),
            "a disabled case is refused by name"
        );
        assert!(
            pdu.contains("checked_add") || !pdu.contains(" + "),
            "no unchecked addition"
        );
        assert!(!pdu.contains(" as "), "no `as` cast");
    }

    #[test]
    fn an_unsupported_construct_in_an_enabled_case_names_the_construct_the_type_and_the_line() {
        let error = run(
            PDU,
            &options(
                &["Code", "Pdu"],
                &[("Pdu", &["Failure", "Ask", "Answer", "Other"])],
            ),
        )
        .expect_err("virtual is not emitted");
        assert!(error.contains("line 12"), "{error}");
        assert!(error.contains("type Pdu"), "{error}");
        assert!(error.contains("`virtual`"), "{error}");
    }

    #[test]
    fn inconsistent_options_are_errors() {
        for (options, expected) in [
            (
                options(&["Pdu"], &[("Pdu", &["Failure"])]),
                "refers to type `Code`",
            ),
            (
                options(&["Code", "Pdu"], &[("Pdu", &["Missing"])]),
                "does not have",
            ),
            (options(&["Code"], &[("Pdu", &["Ask"])]), "do not enable"),
            (options(&["Absent"], &[]), "is not defined"),
        ] {
            let error = run(PDU, &options).expect_err(expected);
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn constructs_outside_the_subset_are_refused() {
        let cases: [(&str, &str); 12] = [
            (
                "[discriminatedType T byteOrder='\"LITTLE_ENDIAN\"' [discriminator uint 8 d] [typeSwitch d ['1' A]]]",
                "LITTLE_ENDIAN",
            ),
            (
                "[discriminatedType T [discriminator uint 8 d] [discriminator uint 8 e] [typeSwitch d,e ['1',* A]]]",
                "`*` case value",
            ),
            (
                "[discriminatedType T [discriminator uint 8 d] [typeSwitch d [A]]]",
                "default case",
            ),
            (
                "[discriminatedType T [discriminator uint 8 d] [typeSwitch d ['1' *A]]]",
                "`*Name`",
            ),
            (
                "[constants [const uint 8 k 1]] [discriminatedType T [discriminator uint 8 d] [typeSwitch d ['1' A [implicit uint 8 n 'k']]]]",
                "constant `k`",
            ),
            (
                "[discriminatedType T [discriminator uint 8 d] [typeSwitch d ['1' A [implicit uint 8 n 'STATIC_CALL(\"f\")']]]]",
                "STATIC_CALL",
            ),
            (
                "[discriminatedType T [discriminator uint 4 d] [typeSwitch d ['1' A [simple uint 16 x]]]]",
                "byte boundary",
            ),
            (
                "[discriminatedType T byteOrder='\"MIDDLE_ENDIAN\"' [discriminator uint 8 d] [typeSwitch d ['1' A]]]",
                "byteOrder",
            ),
            (
                "[discriminatedType T [discriminator uint 8 d] [typeSwitch d ['1' A [simple uint 8 type]]]]",
                "keyword",
            ),
            (
                "[discriminatedType T [discriminator uint 8 d] [typeSwitch d ['1' A [simple T inner]]]]",
                "recursive",
            ),
            (
                "[discriminatedType T [discriminator uint 8 d] [typeSwitch d ['1' A [implicit uint 8 n 'd * 2']]]]",
                "`*`",
            ),
            (
                "[globals [simple uint 8 g]] [discriminatedType T [discriminator uint 8 d] [typeSwitch d ['1' A]]]",
                "globals",
            ),
        ];
        for (text, expected) in cases {
            let error = run(text, &options(&["T"], &[("T", &["A"])])).expect_err(text);
            assert!(error.contains(expected), "{text}: {error}");
        }
    }
}
