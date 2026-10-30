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

use crate::expression::Expr;
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
mod fixtures;

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

/// Names the emitted code uses for itself, which no field or parameter may take.
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
    let mut ctx = Ctx::new(specs, options)?;
    ctx.prepare()?;
    let mut modules = BTreeMap::new();
    for name in &options.types {
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
    support: BTreeSet<&'static str>,
    siblings: BTreeSet<String>,
}

struct Ctx<'a> {
    types: BTreeMap<&'a str, &'a TypeDef>,
    /// Names defined more than once, with the lines of the first two definitions.
    duplicates: BTreeMap<&'a str, (usize, usize)>,
    constants: BTreeSet<&'a str>,
    options: &'a Options,
    /// Enums read from and written to the wire, which get `parse` and `serialize`.
    wire_enums: BTreeSet<String>,
    /// Types whose `lengthInBytes` an expression reads.
    sized: BTreeSet<String>,
}

impl<'a> Ctx<'a> {
    fn new(specs: &'a [Spec], options: &'a Options) -> Result<Self, GenError> {
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
        Ok(Self {
            types,
            duplicates,
            constants,
            options,
            wire_enums: BTreeSet::new(),
            sized: BTreeSet::new(),
        })
    }

    /// The byte order of every multi-byte read and write: the protocol's, `BIG_ENDIAN`
    /// unless its options say otherwise.
    fn order(&self) -> &'static str {
        self.options.byte_order.unwrap_or(Endianness::Big).rust()
    }

    /// The definition of `name`, which must be enabled and defined once. Two
    /// definitions of a name are ambiguous, and the generator will not pick one, so only a
    /// duplicate the output would use is an error.
    fn def(&self, name: &str, line: usize, from: &str) -> Result<&'a TypeDef, GenError> {
        let Some(def) = self.types.get(name).copied() else {
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
        if !self.options.types.contains(name) {
            return Err(GenError {
                line,
                type_name: from.to_owned(),
                message: format!("refers to type `{name}`, which the options do not enable"),
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
        let mut wire = BTreeSet::new();
        let mut sized = BTreeSet::new();
        let mut nested: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
        for name in &self.options.types {
            let def = self.def(name, 0, name)?;
            check_attributes(def, self.options.byte_order.unwrap_or(Endianness::Big))?;
            // The field lists the output reads: a struct's own, or those of enabled cases.
            let lists: Vec<&[Item]> = match def.form {
                TypeForm::Type => vec![def.fields.as_slice()],
                TypeForm::Discriminated | TypeForm::DataIo => self
                    .enabled_cases(def)?
                    .into_iter()
                    .filter(|(_, on)| *on)
                    .map(|(case, _)| case.fields.as_slice())
                    .collect(),
                TypeForm::Enum => continue,
            };
            for items in lists {
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
                        } else {
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
                    "recursive type: {} -> {inner}; recursion bounds are not emitted yet",
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

fn check_name(def: &TypeDef, line_no: usize, rust: &str, original: &str) -> Result<(), GenError> {
    if rust.is_empty()
        || is_keyword(rust)
        || RESERVED.contains(&rust)
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

/// A decimal or hexadecimal literal as written, and its value.
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
fn module(imports: &Imports, support_module: &str, items: &str) -> String {
    let mut text = imports.render(support_module);
    text.push_str(items);
    text
}

impl Imports {
    fn render(&self, support_module: &str) -> String {
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
                &format!("use {support_module}::{{{}}};", names.join(", ")),
            );
        }
        if !out.is_empty() && !self.siblings.is_empty() {
            line(&mut out, "");
        }
        for sibling in &self.siblings {
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
