//! Field lists: the plan of a case's fields (sub-byte runs, scalars, constants, implicit
//! fields, byte arrays, enums and nested types), their reads and writes, and the
//! expression subset of the feature matrix.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Binding, Ctx, DiscValue, GenError, Imports, Ty, check_fits, check_name, describe, gen_error,
    line, parse_integer, scalar,
};
use crate::expression::{BinaryOp, Expr, UnaryOp, Variable};
use crate::mspec::{
    Base, DataType, Field, FieldKind, Item, Literal, LoopType, MExpr, TypeDef, TypeForm, TypeRef,
    fields_of,
};
use crate::naming::{snake, upper_camel};

/// A member of a sub-byte run.
#[derive(Debug, Clone)]
pub(super) struct RunMember {
    name: String,
    ty: Ty,
    offset: u32,
    width: u32,
}

/// One planned field.
#[derive(Debug, Clone)]
pub(super) enum Part<'a> {
    /// A run of sub-byte fields read as one byte, most significant bit first.
    Run(Vec<RunMember>),
    /// A byte-aligned scalar (a `simple` or `discriminator` field).
    Scalar { name: String, ty: Ty, bits: u32 },
    /// A `const` field and its expected value.
    Const {
        name: String,
        ty: Ty,
        bits: u32,
        literal: String,
    },
    /// An `implicit` field and its expression.
    Implicit {
        name: String,
        ty: Ty,
        bits: u32,
        expr: &'a Expr,
    },
    /// An array with a `count` expression. `nullable` arrays are `Option<Vec<T>>`: some
    /// `implicit` field of the list writes -1 for `name == null`, or the count is a stored
    /// `simple int 32` field (`stored_count`), which the value must agree with on serialize.
    Array {
        name: String,
        elem: Elem<'a>,
        count: &'a Expr,
        nullable: bool,
        stored_count: bool,
        line: usize,
    },
    /// A `simple` field of a generated enum.
    Enum {
        name: String,
        enum_name: String,
        bits: u32,
    },
    /// A `simple` field of a generated discriminated type, and its arguments.
    Complex {
        name: String,
        type_name: String,
        args: Vec<&'a MExpr>,
    },
}

/// The element of an array.
#[derive(Debug, Clone)]
pub(super) enum Elem<'a> {
    /// A `byte`, read as one slice.
    Byte,
    /// A byte-aligned number.
    Number { ty: Ty, bits: u32 },
    /// A generated enum, with a base type.
    Enum { name: String, bits: u32 },
    /// A generated discriminated type or struct, and its arguments.
    Complex {
        type_name: String,
        args: Vec<&'a MExpr>,
    },
}

impl Elem<'_> {
    /// The Rust type of one element.
    fn ty(&self) -> Ty {
        match self {
            Self::Byte => Ty::U8,
            Self::Number { ty, .. } => ty.clone(),
            Self::Enum { name, .. } => Ty::Enum(name.clone()),
            Self::Complex { type_name, .. } => Ty::Complex(type_name.clone()),
        }
    }

    /// The width of an element that has one, in bytes.
    const fn width(&self) -> Option<u32> {
        match self {
            Self::Byte => Some(1),
            Self::Number { bits, .. } | Self::Enum { bits, .. } => bits.checked_div(8),
            Self::Complex { .. } => None,
        }
    }
}

impl Part<'_> {
    /// The fields this part stores in its variant, in order.
    pub(super) fn stored(&self) -> Vec<(String, Ty)> {
        match self {
            Self::Run(members) => members
                .iter()
                .map(|member| (member.name.clone(), member.ty.clone()))
                .collect(),
            Self::Scalar { name, ty, .. } => vec![(name.clone(), ty.clone())],
            Self::Array {
                name,
                elem,
                nullable,
                ..
            } => {
                vec![(name.clone(), array_ty(elem, *nullable))]
            }
            Self::Enum {
                name, enum_name, ..
            } => vec![(name.clone(), Ty::Enum(enum_name.clone()))],
            Self::Complex {
                name, type_name, ..
            } => vec![(name.clone(), Ty::Complex(type_name.clone()))],
            Self::Const { .. } | Self::Implicit { .. } => Vec::new(),
        }
    }
}

impl Ctx<'_> {
    /// Plans a field list: sub-byte runs, scalars, constants, implicit fields, byte arrays,
    /// enums and nested types. Anything else is a generation error.
    pub(super) fn plan<'f>(
        &self,
        def: &TypeDef,
        fields: &[&'f Field],
    ) -> Result<Vec<Part<'f>>, GenError> {
        let mut parts = Vec::new();
        let mut run: Vec<RunMember> = Vec::new();
        let mut run_bits = 0_u32;
        let nullable = nullable_arrays(fields);
        for field in fields {
            if !field.attributes.is_empty() {
                return Err(gen_error(
                    def,
                    field.line,
                    "field attributes are not emitted yet",
                ));
            }
            let sub_byte = if let FieldKind::Discriminator {
                ty: TypeRef::Simple(data_type),
                name,
            }
            | FieldKind::Simple {
                ty: TypeRef::Simple(data_type),
                name,
            } = &field.kind
            {
                scalar(data_type)
                    .filter(|(ty, bits)| *bits < 8 && matches!(ty, Ty::Bool | Ty::U8))
                    .map(|(ty, bits)| (name, ty, bits))
            } else {
                None
            };
            if let Some((name, ty, width)) = sub_byte {
                let rust = snake(name);
                check_name(def, field.line, &rust, name)?;
                run.push(RunMember {
                    name: rust,
                    ty,
                    offset: run_bits,
                    width,
                });
                run_bits = run_bits.saturating_add(width);
                if run_bits == 8 {
                    parts.push(Part::Run(std::mem::take(&mut run)));
                    run_bits = 0;
                } else if run_bits > 8 {
                    return Err(gen_error(
                        def,
                        field.line,
                        "a run of sub-byte fields longer than one byte is not emitted yet",
                    ));
                }
                continue;
            }
            if run_bits != 0 {
                return Err(gen_error(
                    def,
                    field.line,
                    "a run of sub-byte fields does not end on a byte boundary",
                ));
            }
            parts.push(self.plan_field(def, field, &nullable)?);
        }
        if run_bits != 0 {
            return Err(gen_error(
                def,
                def.line,
                "a run of sub-byte fields does not end on a byte boundary",
            ));
        }
        Ok(parts)
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "the emitted subset; every other field kind is one generation error"
    )]
    fn plan_field<'f>(
        &self,
        def: &TypeDef,
        field: &'f Field,
        nullable: &BTreeMap<&str, bool>,
    ) -> Result<Part<'f>, GenError> {
        let unsupported = |what: &str| {
            gen_error(
                def,
                field.line,
                &format!(
                    "a `{}` field{}{what} is not emitted (feature matrix)",
                    field.kind.keyword(),
                    if what.is_empty() { "" } else { " " }
                ),
            )
        };
        let aligned = |data_type: &DataType| {
            scalar(data_type)
                .filter(|(ty, bits)| bits.is_multiple_of(8) && ty.accessor().is_some())
                .ok_or_else(|| unsupported(&format!("of type `{}`", describe(data_type))))
        };
        let named = |name: &str| {
            let rust = snake(name);
            check_name(def, field.line, &rust, name).map(|()| rust)
        };
        Ok(match &field.kind {
            FieldKind::Simple { ty, name } | FieldKind::Discriminator { ty, name } => match ty {
                TypeRef::Simple(data_type) => {
                    let (ty, bits) = aligned(data_type)?;
                    Part::Scalar {
                        name: named(name)?,
                        ty,
                        bits,
                    }
                }
                TypeRef::Complex {
                    name: type_name,
                    args,
                } => {
                    let target = self.def(type_name, field.line, &def.name)?;
                    match target.form {
                        TypeForm::Enum => {
                            if !args.is_empty() {
                                return Err(unsupported("with enum arguments"));
                            }
                            let bits = target
                                .enum_type
                                .as_ref()
                                .and_then(scalar)
                                .map(|(_, bits)| bits)
                                .ok_or_else(|| unsupported("of an enum without a base type"))?;
                            Part::Enum {
                                name: named(name)?,
                                enum_name: type_name.clone(),
                                bits,
                            }
                        }
                        TypeForm::Discriminated | TypeForm::Type => Part::Complex {
                            name: named(name)?,
                            type_name: type_name.clone(),
                            args: args.iter().collect(),
                        },
                        TypeForm::DataIo => {
                            return Err(unsupported(&format!(
                                "of the {} `{type_name}`",
                                target.form.keyword()
                            )));
                        }
                    }
                }
            },
            FieldKind::Const {
                ty: TypeRef::Simple(data_type),
                name,
                expected,
            } => {
                let (ty, bits) = aligned(data_type)?;
                let (Literal::Hex(text) | Literal::Integer(text)) = expected else {
                    return Err(unsupported("with a non-integer value"));
                };
                if !ty.widens_to_u64() || ty == Ty::Bool {
                    return Err(unsupported("of a signed or floating type"));
                }
                let number = parse_integer(text).ok_or_else(|| unsupported("with a bad value"))?;
                check_fits(def, field.line, number, &ty, Some(data_type))?;
                Part::Const {
                    name: named(name)?,
                    ty,
                    bits,
                    literal: text.clone(),
                }
            }
            FieldKind::Implicit { ty, name, expr } => {
                let (ty, bits) = aligned(ty)?;
                if !ty.is_implicit_count() {
                    return Err(unsupported(
                        "other than `uint 8`, `uint 16`, `uint 32` or `int 32`",
                    ));
                }
                let MExpr::Expr(expr, _) = expr else {
                    return Err(unsupported("with a wildcard"));
                };
                Part::Implicit {
                    name: named(name)?,
                    ty,
                    bits,
                    expr,
                }
            }
            FieldKind::Array {
                ty,
                name,
                loop_type: LoopType::Count,
                expr: MExpr::Expr(count, _),
            } => {
                let elem = match ty {
                    TypeRef::Simple(DataType {
                        base: Base::Byte, ..
                    }) => Elem::Byte,
                    TypeRef::Simple(data_type) => {
                        let (ty, bits) = aligned(data_type)?;
                        Elem::Number { ty, bits }
                    }
                    TypeRef::Complex {
                        name: type_name,
                        args,
                    } => {
                        let target = self.def(type_name, field.line, &def.name)?;
                        match target.form {
                            TypeForm::Enum => {
                                if !args.is_empty() {
                                    return Err(unsupported("of an enum with arguments"));
                                }
                                let bits = target
                                    .enum_type
                                    .as_ref()
                                    .and_then(scalar)
                                    .map(|(_, bits)| bits)
                                    .ok_or_else(|| unsupported("of an enum without a base type"))?;
                                Elem::Enum {
                                    name: type_name.clone(),
                                    bits,
                                }
                            }
                            TypeForm::Discriminated | TypeForm::Type => Elem::Complex {
                                type_name: type_name.clone(),
                                args: args.iter().collect(),
                            },
                            TypeForm::DataIo => {
                                return Err(unsupported(&format!(
                                    "of the {} `{type_name}`",
                                    target.form.keyword()
                                )));
                            }
                        }
                    }
                };
                Part::Array {
                    name: named(name)?,
                    elem,
                    count,
                    nullable: nullable.contains_key(name.as_str()),
                    stored_count: nullable.get(name.as_str()).copied().unwrap_or(false),
                    line: field.line,
                }
            }
            FieldKind::Array { .. } => {
                return Err(unsupported("other than a `count` array"));
            }
            _ => return Err(unsupported("")),
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the emission state of one function body"
    )]
    pub(super) fn emit_parse_parts(
        &self,
        def: &TypeDef,
        parts: &[Part<'_>],
        out: &mut String,
        scope: &mut BTreeMap<String, Binding>,
        run_index: &mut u32,
        imports: &mut Imports,
        used_params: &mut BTreeSet<String>,
    ) -> Result<(), GenError> {
        let order = self.order().to_owned();
        for part in parts {
            match part {
                Part::Run(members) => {
                    let bits = if *run_index == 0 {
                        "bits".to_owned()
                    } else {
                        format!("bits_{run_index}")
                    };
                    *run_index = run_index.saturating_add(1);
                    line(out, &format!("let {bits} = reader.read_u8()?;"));
                    imports.support.insert("split_bits");
                    for member in members {
                        let read =
                            format!("split_bits({bits}, {}, {})?", member.offset, member.width);
                        if member.ty == Ty::Bool {
                            line(out, &format!("let {} = {read} != 0;", member.name));
                        } else {
                            line(out, &format!("let {} = {read};", member.name));
                        }
                        scope.insert(
                            member.name.clone(),
                            Binding {
                                ty: member.ty.clone(),
                                by_ref: false,
                            },
                        );
                    }
                }
                Part::Scalar { name, ty, .. }
                | Part::Const { name, ty, .. }
                | Part::Implicit { name, ty, .. } => {
                    let Some((suffix, ordered)) = ty.accessor() else {
                        return Err(gen_error(def, def.line, "scalar without an accessor"));
                    };
                    let argument = if ordered {
                        imports.byte_order = true;
                        order.clone()
                    } else {
                        String::new()
                    };
                    line(
                        out,
                        &format!("let {name} = reader.read_{suffix}({argument})?;"),
                    );
                    if let Part::Const { literal, .. } = part {
                        line(
                            out,
                            &format!(
                                "if {name} != {literal} {{ return Err(ParseError::InvalidDiscriminator {{ field: \"{}\", value: u64::from({name}) }}); }}",
                                original(def, name)
                            ),
                        );
                    }
                    scope.insert(
                        name.clone(),
                        Binding {
                            ty: ty.clone(),
                            by_ref: false,
                        },
                    );
                }
                Part::Array {
                    name,
                    elem,
                    count,
                    nullable,
                    line,
                    ..
                } => {
                    self.emit_array_parse(
                        def,
                        *line,
                        name,
                        elem,
                        count,
                        *nullable,
                        scope,
                        imports,
                        used_params,
                        out,
                    )?;
                    let ty = array_ty(elem, *nullable);
                    scope.insert(name.clone(), Binding { ty, by_ref: false });
                }
                Part::Enum {
                    name, enum_name, ..
                } => {
                    line(
                        out,
                        &format!("let {name} = {}::parse(reader)?;", upper_camel(enum_name)),
                    );
                    imports.siblings.insert(enum_name.clone());
                    scope.insert(
                        name.clone(),
                        Binding {
                            ty: Ty::Enum(enum_name.clone()),
                            by_ref: false,
                        },
                    );
                }
                Part::Complex {
                    name,
                    type_name,
                    args,
                } => {
                    let args = self.args(def, type_name, args, scope, used_params)?;
                    line(
                        out,
                        &format!(
                            "let {name} = {}::parse(reader{args})?;",
                            upper_camel(type_name)
                        ),
                    );
                    imports.siblings.insert(type_name.clone());
                    scope.insert(
                        name.clone(),
                        Binding {
                            ty: Ty::Complex(type_name.clone()),
                            by_ref: false,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    /// The arguments of a nested `parse` call: each a parameter or field of the caller
    /// with the callee parameter's type.
    pub(super) fn args(
        &self,
        def: &TypeDef,
        type_name: &str,
        args: &[&MExpr],
        scope: &BTreeMap<String, Binding>,
        used_params: &mut BTreeSet<String>,
    ) -> Result<String, GenError> {
        let target = self.def(type_name, def.line, &def.name)?;
        let params = self.params(target)?;
        if params.len() != args.len() {
            return Err(gen_error(
                def,
                def.line,
                &format!("`{type_name}` takes {} arguments", params.len()),
            ));
        }
        let mut text = String::new();
        for ((_, want), arg) in params.iter().zip(args) {
            let MExpr::Expr(Expr::Var(var), raw) = arg else {
                return Err(gen_error(
                    def,
                    def.line,
                    "only a name is emitted as a type argument",
                ));
            };
            let rust = snake(&var.name);
            let found = scope.get(&rust).filter(|binding| {
                var.args.is_none()
                    && var.indexes.is_empty()
                    && var.rest.is_none()
                    && binding.ty == *want
                    && !binding.by_ref
            });
            if found.is_none() {
                return Err(gen_error(
                    def,
                    def.line,
                    &format!("argument '{raw}' does not name a value of the parameter's type"),
                ));
            }
            used_params.insert(rust.clone());
            text.push_str(", ");
            text.push_str(&rust);
        }
        Ok(text)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the emission state of one match arm"
    )]
    pub(super) fn emit_serialize_parts(
        &self,
        def: &TypeDef,
        line_no: usize,
        parts: &[Part<'_>],
        disc_values: &BTreeMap<String, DiscValue>,
        out: &mut String,
        scope: &BTreeMap<String, Binding>,
        imports: &mut Imports,
    ) -> Result<(), GenError> {
        let order = self.order().to_owned();
        let mut unused = BTreeSet::new();
        for part in parts {
            match part {
                Part::Run(members) => {
                    let mut constant = 0_u32;
                    let mut dynamic = Vec::new();
                    for member in members {
                        let shift = 8_u32
                            .checked_sub(member.offset)
                            .and_then(|rest| rest.checked_sub(member.width))
                            .ok_or_else(|| gen_error(def, line_no, "bad run layout"))?;
                        match disc_values.get(&member.name) {
                            Some(DiscValue::Literal(literal)) => {
                                let value = match literal.as_str() {
                                    "true" => 1,
                                    "false" => 0,
                                    other => u32::try_from(parse_integer(other).unwrap_or(256))
                                        .unwrap_or(256),
                                };
                                let limit = 1_u32.checked_shl(member.width).unwrap_or(0);
                                if value >= limit {
                                    return Err(gen_error(
                                        def,
                                        line_no,
                                        &format!("value {literal} does not fit `{}`", member.name),
                                    ));
                                }
                                constant |= value.checked_shl(shift).unwrap_or(0);
                            }
                            Some(DiscValue::Stored) | None => {
                                let value = if member.ty == Ty::Bool {
                                    format!("u8::from(*{})", member.name)
                                } else {
                                    format!("*{}", member.name)
                                };
                                dynamic.push((member.offset, member.width, value));
                            }
                        }
                    }
                    let mut value = format!("{constant:#04x}");
                    for (offset, width, member) in dynamic {
                        imports.support.insert("join_bits");
                        value = format!("join_bits({value}, {offset}, {width}, {member})?");
                    }
                    line(out, &format!("writer.write_u8({value})?;"));
                }
                Part::Scalar { name, ty, .. } => {
                    let Some((suffix, ordered)) = ty.accessor() else {
                        return Err(gen_error(def, line_no, "scalar without an accessor"));
                    };
                    let value = match disc_values.get(name) {
                        Some(DiscValue::Literal(literal)) => literal.clone(),
                        Some(DiscValue::Stored) | None => format!("*{name}"),
                    };
                    write_scalar(out, suffix, ordered, &value, &order, imports);
                }
                Part::Const { ty, literal, .. } => {
                    let Some((suffix, ordered)) = ty.accessor() else {
                        return Err(gen_error(def, line_no, "const without an accessor"));
                    };
                    write_scalar(out, suffix, ordered, literal, &order, imports);
                }
                Part::Implicit { name, ty, expr, .. } => {
                    let Some((suffix, ordered)) = ty.accessor() else {
                        return Err(gen_error(def, line_no, "implicit without an accessor"));
                    };
                    let value = self.typed(def, line_no, expr, scope, &mut unused)?;
                    if value.kind == Kind::Bool {
                        return Err(not_a_number(def, line_no));
                    }
                    let value = value.text;
                    line(
                        out,
                        &format!(
                            "let {name} = {}::try_from({value}).map_err(|_overflow| ParseError::Overflow)?;",
                            ty.rust()
                        ),
                    );
                    write_scalar(out, suffix, ordered, name, &order, imports);
                }
                Part::Array {
                    name,
                    elem,
                    count,
                    nullable,
                    stored_count,
                    line,
                } => {
                    if *stored_count {
                        self.emit_stored_count_check(def, *line, name, count, scope, out)?;
                    }
                    emit_array_serialize(out, name, elem, *nullable, &order, imports);
                }
                Part::Enum { name, .. } | Part::Complex { name, .. } => {
                    line(out, &format!("{name}.serialize(writer)?;"));
                }
            }
        }
        Ok(())
    }

    /// Once a case or struct is read, compares each `implicit` field with its expression
    /// over the parsed values (ADR-0019 addendum, decision 5). `items` are the fields the
    /// parts were planned from, which name the field in the error.
    #[expect(
        clippy::too_many_arguments,
        reason = "the emission state of one function body"
    )]
    pub(super) fn emit_implicit_checks(
        &self,
        def: &TypeDef,
        line_no: usize,
        parts: &[Part<'_>],
        items: &[Item],
        scope: &BTreeMap<String, Binding>,
        out: &mut String,
        used_params: &mut BTreeSet<String>,
    ) -> Result<(), GenError> {
        for part in parts {
            if let Part::Implicit {
                name: field,
                ty,
                expr,
                ..
            } = part
            {
                let value = self.typed(def, line_no, expr, scope, used_params)?;
                let error = if *ty == Ty::I32 {
                    format!("u64::from({field}.unsigned_abs())")
                } else {
                    format!("u64::from({field})")
                };
                let (field_value, value) = match (ty.widens_to_usize(), value.kind) {
                    // The lengths of Modbus: the field widens to a `usize`.
                    (true, Kind::Usize) => (format!("usize::from({field})"), value.embed()),
                    (_, Kind::Bool) => return Err(not_a_number(def, line_no)),
                    _ => (
                        format!("i64::from({field})"),
                        Self::to_int(def, line_no, value)?.embed(),
                    ),
                };
                line(
                    out,
                    &format!(
                        "if {field_value} != {value} {{ return Err(ParseError::InvalidDiscriminator {{ field: \"{}\", value: {error} }}); }}",
                        original_name(items, field),
                    ),
                );
                if !ty.is_implicit_count() {
                    return Err(gen_error(def, line_no, "implicit field type not emitted"));
                }
            }
        }
        Ok(())
    }

    /// Emits an expression of the subset in the feature matrix: integers, names, `COUNT` of
    /// an array, `lengthInBytes` of a nested type, `+`, `==` (of numbers, or of a nullable
    /// array and `null`), a prefix `-` and `?:`.
    pub(super) fn typed(
        &self,
        def: &TypeDef,
        line_no: usize,
        expr: &Expr,
        scope: &BTreeMap<String, Binding>,
        used_params: &mut BTreeSet<String>,
    ) -> Result<Typed, GenError> {
        let unsupported = |what: &str| {
            gen_error(
                def,
                line_no,
                &format!("the expression construct {what} is not emitted (feature matrix)"),
            )
        };
        match expr {
            Expr::Number(text) | Expr::Hex(text) => parse_integer(text)
                .filter(|value| *value <= u128::from(i64::MAX.unsigned_abs()))
                .map(|_| Typed {
                    text: text.clone(),
                    kind: Kind::Usize,
                    atomic: true,
                    literal: true,
                })
                .ok_or_else(|| unsupported(&format!("`{text}` (not an integer in range)"))),
            Expr::Group(inner, indexes) if indexes.is_empty() => {
                self.typed(def, line_no, inner, scope, used_params)
            }
            Expr::Binary(BinaryOp::Add, lhs, rhs) => {
                let lhs = self.typed(def, line_no, lhs, scope, used_params)?;
                let rhs = self.typed(def, line_no, rhs, scope, used_params)?;
                let (lhs, rhs) = Self::same_kind(def, line_no, lhs, rhs)?;
                let kind = lhs.kind;
                Ok(Typed::atom(
                    format!(
                        "{}.checked_add({}).ok_or(ParseError::Overflow)?",
                        lhs.embed(),
                        rhs.text
                    ),
                    kind,
                ))
            }
            Expr::Binary(BinaryOp::Eq, lhs, rhs) => {
                if let (Expr::Var(var), Expr::Null) | (Expr::Null, Expr::Var(var)) =
                    (lhs.as_ref(), rhs.as_ref())
                {
                    return Self::null_test(def, line_no, var, scope, used_params);
                }
                let lhs = self.typed(def, line_no, lhs, scope, used_params)?;
                let rhs = self.typed(def, line_no, rhs, scope, used_params)?;
                let (lhs, rhs) = Self::same_kind(def, line_no, lhs, rhs)?;
                Ok(Typed {
                    text: format!("{} == {}", lhs.embed(), rhs.embed()),
                    kind: Kind::Bool,
                    atomic: false,
                    literal: false,
                })
            }
            Expr::Binary(op, _, _) => Err(unsupported(&format!("`{}`", op.symbol()))),
            Expr::Unary(UnaryOp::Neg, inner) => {
                if let Expr::Number(text) | Expr::Hex(text) = inner.as_ref() {
                    parse_integer(text)
                        .filter(|value| *value <= u128::from(i64::MAX.unsigned_abs()))
                        .ok_or_else(|| {
                            unsupported(&format!("`-{text}` (not an integer in range)"))
                        })?;
                    return Ok(Typed {
                        text: format!("-{text}_i64"),
                        kind: Kind::Int,
                        atomic: false,
                        literal: false,
                    });
                }
                let inner = self.typed(def, line_no, inner, scope, used_params)?;
                let inner = Self::to_int(def, line_no, inner)?;
                Ok(Typed::atom(
                    format!(
                        "{}.checked_neg().ok_or(ParseError::Overflow)?",
                        inner.embed()
                    ),
                    Kind::Int,
                ))
            }
            Expr::Unary(UnaryOp::Not, _) => Err(unsupported("`!`")),
            Expr::Ternary(condition, then, otherwise) => {
                let condition = self.typed(def, line_no, condition, scope, used_params)?;
                if condition.kind != Kind::Bool {
                    return Err(unsupported(
                        "`?:` with a condition that is not a comparison",
                    ));
                }
                let then = self.typed(def, line_no, then, scope, used_params)?;
                let otherwise = self.typed(def, line_no, otherwise, scope, used_params)?;
                let (then, otherwise) = Self::same_kind(def, line_no, then, otherwise)?;
                Ok(Typed {
                    text: format!(
                        "if {} {{ {} }} else {{ {} }}",
                        condition.text, then.text, otherwise.text
                    ),
                    kind: then.kind,
                    atomic: false,
                    literal: false,
                })
            }
            Expr::Var(var) => self.variable(def, line_no, var, scope, used_params),
            Expr::Bool(_) => Err(unsupported("a boolean")),
            Expr::Null => Err(unsupported("`null` outside `name == null`")),
            Expr::Str(..) => Err(unsupported("a string")),
            Expr::Group(..) => Err(unsupported("an index")),
        }
    }

    /// `name == null` of a nullable array.
    fn null_test(
        def: &TypeDef,
        line_no: usize,
        var: &Variable,
        scope: &BTreeMap<String, Binding>,
        used_params: &mut BTreeSet<String>,
    ) -> Result<Typed, GenError> {
        let rust = snake(&var.name);
        let nullable = var.args.is_none()
            && var.indexes.is_empty()
            && var.rest.is_none()
            && matches!(
                scope.get(&rust),
                Some(Binding {
                    ty: Ty::Array { nullable: true, .. },
                    ..
                })
            );
        if !nullable {
            return Err(gen_error(
                def,
                line_no,
                &format!(
                    "`{} == null` needs an array that an `implicit` field writes as null (feature matrix)",
                    var.name
                ),
            ));
        }
        used_params.insert(rust.clone());
        Ok(Typed::atom(format!("{rust}.is_none()"), Kind::Bool))
    }

    /// Both operands in one integer domain: `usize` when both are, otherwise `i64`.
    fn same_kind(
        def: &TypeDef,
        line_no: usize,
        lhs: Typed,
        rhs: Typed,
    ) -> Result<(Typed, Typed), GenError> {
        if lhs.kind == Kind::Bool || rhs.kind == Kind::Bool {
            return Err(not_a_number(def, line_no));
        }
        if lhs.kind == Kind::Usize && rhs.kind == Kind::Usize {
            return Ok((lhs, rhs));
        }
        Ok((
            Self::to_int(def, line_no, lhs)?,
            Self::to_int(def, line_no, rhs)?,
        ))
    }

    /// The value as an `i64`: a literal gets the type, any other `usize` is converted.
    fn to_int(def: &TypeDef, line_no: usize, value: Typed) -> Result<Typed, GenError> {
        Ok(match value.kind {
            Kind::Int => value,
            Kind::Usize if value.literal => Typed::atom(format!("{}_i64", value.text), Kind::Int),
            Kind::Usize => Typed::atom(
                format!(
                    "i64::try_from({}).map_err(|_overflow| ParseError::Overflow)?",
                    value.text
                ),
                Kind::Int,
            ),
            Kind::Bool => return Err(not_a_number(def, line_no)),
        })
    }

    fn variable(
        &self,
        def: &TypeDef,
        line_no: usize,
        var: &Variable,
        scope: &BTreeMap<String, Binding>,
        used_params: &mut BTreeSet<String>,
    ) -> Result<Typed, GenError> {
        let fail = |message: &str| gen_error(def, line_no, message);
        if !var.indexes.is_empty() {
            return Err(fail("an indexed name is not emitted (feature matrix)"));
        }
        if let Some(args) = &var.args {
            return match (var.name.as_str(), args.as_slice(), &var.rest) {
                ("COUNT", [Expr::Var(target)], None)
                    if target.args.is_none()
                        && target.rest.is_none()
                        && target.indexes.is_empty() =>
                {
                    let rust = snake(&target.name);
                    let text = match scope.get(&rust).map(|binding| &binding.ty) {
                        Some(
                            Ty::Bytes
                            | Ty::Array {
                                nullable: false, ..
                            },
                        ) => {
                            format!("{rust}.len()")
                        }
                        Some(Ty::Array { nullable: true, .. }) => {
                            format!("{rust}.as_ref().map_or(0, Vec::len)")
                        }
                        _ => {
                            return Err(fail(&format!(
                                "COUNT of `{}`, which is not an array",
                                target.name
                            )));
                        }
                    };
                    Ok(Typed::atom(text, Kind::Usize))
                }
                ("STATIC_CALL", ..) => Err(fail(
                    "STATIC_CALL names a function the protocol's options do not list",
                )),
                (other, ..) => Err(fail(&format!(
                    "the built-in function {other} is not emitted (feature matrix)"
                ))),
            };
        }
        if self.constants.contains(var.name.as_str()) {
            return Err(fail(&format!(
                "refers to constant `{}`; the constants block is parsed but not emitted",
                var.name
            )));
        }
        let rust = snake(&var.name);
        let Some(binding) = scope.get(&rust) else {
            return Err(fail(&format!(
                "`{}` names no field or parameter in scope",
                var.name
            )));
        };
        used_params.insert(rust.clone());
        let value = if binding.by_ref {
            format!("*{rust}")
        } else {
            rust.clone()
        };
        match (&var.rest, &binding.ty) {
            (None, ty) if ty.widens_to_usize() => {
                Ok(Typed::atom(format!("usize::from({value})"), Kind::Usize))
            }
            (None, Ty::U32) => Ok(Typed::atom(
                format!("usize::try_from({value}).map_err(|_overflow| ParseError::Overflow)?"),
                Kind::Usize,
            )),
            (None, Ty::I32) => Ok(Typed::atom(format!("i64::from({value})"), Kind::Int)),
            (Some(property), Ty::Complex(type_name))
                if property.name == "lengthInBytes"
                    && property.args.is_none()
                    && property.indexes.is_empty()
                    && property.rest.is_none() =>
            {
                if !self.sized.contains(type_name) {
                    return Err(fail("lengthInBytes of a type whose length is not emitted"));
                }
                Ok(Typed::atom(
                    format!("{rust}.length_in_bytes()?"),
                    Kind::Usize,
                ))
            }
            (Some(property), _) => Err(fail(&format!(
                "the property `{}` is not emitted (feature matrix)",
                property.name
            ))),
            (None, ty) => Err(fail(&format!(
                "`{}` of type `{}` in an expression is not emitted",
                var.name,
                ty.rust()
            ))),
        }
    }

    /// Before a nullable array whose count is a stored field is written: the stored count
    /// must be -1 for `None` and the length for `Some`, or the bytes would not parse back
    /// to the value.
    fn emit_stored_count_check(
        &self,
        def: &TypeDef,
        line_no: usize,
        name: &str,
        count: &Expr,
        scope: &BTreeMap<String, Binding>,
        out: &mut String,
    ) -> Result<(), GenError> {
        let mut unused = BTreeSet::new();
        let count = self.typed(def, line_no, count, scope, &mut unused)?;
        let field = original(def, name);
        line(
            out,
            &format!(
                "let array_count = match {name} {{ None => -1_i64, Some(array_items) => i64::try_from(array_items.len()).map_err(|_overflow| ParseError::Overflow)? }};"
            ),
        );
        line(
            out,
            &format!(
                "if {} != array_count {{ return Err(ParseError::InvalidDiscriminator {{ field: \"{field}\", value: array_count.unsigned_abs() }}); }}",
                count.embed()
            ),
        );
        Ok(())
    }

    /// Reads an array into `name`: the count, then the elements, bounded by the bytes
    /// present before anything is allocated.
    #[expect(
        clippy::too_many_arguments,
        reason = "the emission state of one function body"
    )]
    fn emit_array_parse(
        &self,
        def: &TypeDef,
        line_no: usize,
        name: &str,
        elem: &Elem<'_>,
        count: &Expr,
        nullable: bool,
        scope: &BTreeMap<String, Binding>,
        imports: &mut Imports,
        used_params: &mut BTreeSet<String>,
        out: &mut String,
    ) -> Result<(), GenError> {
        let count = self.typed(def, line_no, count, scope, used_params)?;
        if count.kind == Kind::Bool {
            return Err(not_a_number(def, line_no));
        }
        if matches!(elem, Elem::Byte) && !nullable && count.kind == Kind::Usize {
            line(
                out,
                &format!(
                    "let {name} = reader.read_bytes({}, usize::MAX)?.to_vec();",
                    count.text
                ),
            );
            return Ok(());
        }
        if nullable && count.kind != Kind::Int {
            return Err(gen_error(
                def,
                line_no,
                &format!(
                    "the array `{}` is null at -1 and its count is not signed (feature matrix)",
                    original(def, name)
                ),
            ));
        }
        let field = original(def, name);
        let (reads, value) =
            self.array_reads(def, line_no, &field, elem, scope, imports, used_params)?;
        let mut body = String::new();
        if count.kind == Kind::Usize && count.literal {
            line(
                &mut body,
                &format!("let array_count = {}_usize;", count.text),
            );
        } else {
            line(&mut body, &format!("let array_count = {};", count.text));
        }
        if count.kind == Kind::Int {
            // -1 is null for a nullable array; any other negative count is no count.
            let convert = format!(
                "let array_count = usize::try_from(array_count).map_err(|_overflow| ParseError::InvalidDiscriminator {{ field: \"{field}\", value: array_count.unsigned_abs() }})?;"
            );
            if nullable {
                line(&mut body, "if array_count == -1 {");
                line(&mut body, "None");
                line(&mut body, "} else {");
                line(&mut body, &convert);
                body.push_str(&reads);
                line(&mut body, &format!("Some({value})"));
                line(&mut body, "}");
            } else {
                line(&mut body, &convert);
                body.push_str(&reads);
                line(&mut body, &value);
            }
        } else {
            body.push_str(&reads);
            line(&mut body, &value);
        }
        line(out, &format!("let {name} = {{"));
        out.push_str(&body);
        line(out, "};");
        Ok(())
    }

    /// The statements that read `array_count` elements, and the expression of the result.
    /// A fixed width is checked as `count * width` against the remaining input; an element
    /// of no fixed width needs at least one byte, so the count is checked against the
    /// remaining input and the vector grows one parsed element at a time.
    #[expect(
        clippy::too_many_arguments,
        reason = "the emission state of one function body"
    )]
    fn array_reads(
        &self,
        def: &TypeDef,
        line_no: usize,
        field: &str,
        elem: &Elem<'_>,
        scope: &BTreeMap<String, Binding>,
        imports: &mut Imports,
        used_params: &mut BTreeSet<String>,
    ) -> Result<(String, String), GenError> {
        let order = self.order().to_owned();
        let mut reads = String::new();
        let check = |reads: &mut String, width: u32| {
            if width == 1 {
                line(
                    reads,
                    &format!(
                        "if array_count > reader.remaining() {{ return Err(ParseError::CountLimit {{ field: \"{field}\", count: array_count, max: reader.remaining() }}); }}"
                    ),
                );
            } else {
                line(
                    reads,
                    &format!(
                        "let array_needed = array_count.checked_mul({width}).ok_or(ParseError::Overflow)?;"
                    ),
                );
                line(
                    reads,
                    &format!(
                        "if array_needed > reader.remaining() {{ return Err(ParseError::CountLimit {{ field: \"{field}\", count: array_count, max: reader.remaining().checked_div({width}).unwrap_or(0) }}); }}"
                    ),
                );
            }
        };
        let read = match elem {
            Elem::Byte => {
                return Ok((
                    reads,
                    "reader.read_bytes(array_count, usize::MAX)?.to_vec()".to_owned(),
                ));
            }
            Elem::Number { ty, .. } => {
                let Some((suffix, ordered)) = ty.accessor() else {
                    return Err(gen_error(def, line_no, "array element without an accessor"));
                };
                if ordered {
                    imports.byte_order = true;
                    format!("reader.read_{suffix}({order})?")
                } else {
                    format!("reader.read_{suffix}()?")
                }
            }
            Elem::Enum { name, .. } => {
                imports.siblings.insert(name.clone());
                format!("{}::parse(reader)?", upper_camel(name))
            }
            Elem::Complex { type_name, args } => {
                let args = self.args(def, type_name, args, scope, used_params)?;
                imports.siblings.insert(type_name.clone());
                format!("{}::parse(reader{args})?", upper_camel(type_name))
            }
        };
        match elem.width() {
            Some(width) if !matches!(elem, Elem::Complex { .. }) => {
                check(&mut reads, width);
                line(
                    &mut reads,
                    "let mut array_items = Vec::with_capacity(array_count);",
                );
            }
            _ => {
                check(&mut reads, 1);
                line(&mut reads, "let mut array_items = Vec::new();");
            }
        }
        line(
            &mut reads,
            &format!("for _ in 0..array_count {{ array_items.push({read}); }}"),
        );
        Ok((reads, "array_items".to_owned()))
    }
}

/// The type an array field is stored as: `Vec<u8>` for bytes, else a vector of the element,
/// optional when the count can be -1.
fn array_ty(elem: &Elem<'_>, nullable: bool) -> Ty {
    if matches!(elem, Elem::Byte) && !nullable {
        Ty::Bytes
    } else {
        Ty::Array {
            elem: Box::new(elem.ty()),
            nullable,
        }
    }
}

/// The names of the arrays that are `Option<Vec<T>>`, each with whether its count is a
/// stored `simple int 32` field: the arrays that an `implicit` field of the list writes as
/// null (`name == null` in its expression), and the arrays counted by a bare `simple int 32`
/// field, where the wire carries -1 for null as well (`ByteStringArray` of OPC UA).
fn nullable_arrays<'f>(fields: &[&'f Field]) -> BTreeMap<&'f str, bool> {
    fn walk<'e>(expr: &'e Expr, found: &mut BTreeSet<&'e str>) {
        match expr {
            Expr::Binary(BinaryOp::Eq, lhs, rhs) => {
                if let (Expr::Var(var), Expr::Null) | (Expr::Null, Expr::Var(var)) =
                    (lhs.as_ref(), rhs.as_ref())
                {
                    found.insert(var.name.as_str());
                }
                walk(lhs, found);
                walk(rhs, found);
            }
            Expr::Binary(_, lhs, rhs) => {
                walk(lhs, found);
                walk(rhs, found);
            }
            Expr::Ternary(a, b, c) => {
                walk(a, found);
                walk(b, found);
                walk(c, found);
            }
            Expr::Unary(_, inner) | Expr::Group(inner, _) => walk(inner, found),
            Expr::Number(_)
            | Expr::Hex(_)
            | Expr::Bool(_)
            | Expr::Null
            | Expr::Str(..)
            | Expr::Var(_) => {}
        }
    }
    let mut found = BTreeSet::new();
    let mut signed = BTreeSet::new();
    for field in fields {
        if let FieldKind::Implicit {
            expr: MExpr::Expr(expr, _),
            ..
        } = &field.kind
        {
            walk(expr, &mut found);
        }
        if let FieldKind::Simple {
            ty: TypeRef::Simple(data_type),
            name,
        } = &field.kind
            && scalar(data_type).is_some_and(|(ty, _)| ty == Ty::I32)
        {
            signed.insert(snake(name));
        }
    }
    let mut nullable: BTreeMap<&str, bool> = found.into_iter().map(|name| (name, false)).collect();
    for field in fields {
        if let FieldKind::Array {
            name,
            loop_type: LoopType::Count,
            expr: MExpr::Expr(Expr::Var(var), _),
            ..
        } = &field.kind
            && var.args.is_none()
            && var.indexes.is_empty()
            && var.rest.is_none()
            && signed.contains(&snake(&var.name))
        {
            nullable.entry(name.as_str()).or_insert(true);
        }
    }
    nullable
}

/// Writes an array: the elements only, as the count is its `implicit` field's.
fn emit_array_serialize(
    out: &mut String,
    name: &str,
    elem: &Elem<'_>,
    nullable: bool,
    order: &str,
    imports: &mut Imports,
) {
    if matches!(elem, Elem::Byte) {
        if nullable {
            line(
                out,
                &format!(
                    "if let Some(array_items) = {name} {{ writer.write_bytes(array_items)?; }}"
                ),
            );
        } else {
            line(out, &format!("writer.write_bytes({name})?;"));
        }
        return;
    }
    let mut write = String::new();
    match elem {
        Elem::Number { ty, .. } => {
            if let Some((suffix, ordered)) = ty.accessor() {
                write_scalar(&mut write, suffix, ordered, "*array_item", order, imports);
            }
        }
        Elem::Enum { .. } | Elem::Complex { .. } => {
            line(&mut write, "array_item.serialize(writer)?;");
        }
        Elem::Byte => {}
    }
    let write = write.trim_end();
    if nullable {
        line(
            out,
            &format!(
                "if let Some(array_items) = {name} {{ for array_item in array_items {{ {write} }} }}"
            ),
        );
    } else {
        line(out, &format!("for array_item in {name} {{ {write} }}"));
    }
}

/// The integer domain of an emitted expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// A `usize`: lengths, counts and the unsigned fields that widen to one.
    Usize,
    /// An `i64`: a signed field, a negative literal, or any mix with one.
    Int,
    /// A `bool`: a comparison.
    Bool,
}

/// An emitted expression, and what it is.
#[derive(Debug, Clone)]
pub(super) struct Typed {
    /// The Rust text.
    pub(super) text: String,
    /// Its domain.
    pub(super) kind: Kind,
    /// Whether the text is one operand (a name, a call, a method chain) that needs no
    /// parentheses when it is embedded in another expression.
    atomic: bool,
    /// Whether it is an integer literal, which takes the type of its context.
    literal: bool,
}

impl Typed {
    const fn atom(text: String, kind: Kind) -> Self {
        Self {
            text,
            kind,
            atomic: true,
            literal: false,
        }
    }

    /// The text as an operand of another expression.
    fn embed(&self) -> String {
        if self.atomic {
            self.text.clone()
        } else {
            format!("({})", self.text)
        }
    }
}

fn not_a_number(def: &TypeDef, line_no: usize) -> GenError {
    gen_error(
        def,
        line_no,
        "a comparison where a number is needed (feature matrix)",
    )
}

/// Writes one scalar with its byte order when it has one.
fn write_scalar(
    out: &mut String,
    suffix: &str,
    ordered: bool,
    value: &str,
    order: &str,
    imports: &mut Imports,
) {
    if ordered {
        imports.byte_order = true;
        line(out, &format!("writer.write_{suffix}({value}, {order})?;"));
    } else {
        line(out, &format!("writer.write_{suffix}({value})?;"));
    }
}

/// The mspec name of a field from its Rust name, for error messages in the output.
fn original(def: &TypeDef, rust: &str) -> String {
    original_name(&def.fields, rust)
}

pub(super) fn original_name(items: &[Item], rust: &str) -> String {
    let mut found = None;
    visit_names(items, &mut |name| {
        if found.is_none() && snake(name) == rust {
            found = Some(name.to_owned());
        }
    });
    found.unwrap_or_else(|| rust.to_owned())
}

fn visit_names(items: &[Item], visit: &mut impl FnMut(&str)) {
    for field in fields_of(items) {
        match &field.kind {
            FieldKind::TypeSwitch { cases, .. } => {
                for case in cases {
                    visit_names(&case.fields, visit);
                }
            }
            FieldKind::Abstract { name, .. }
            | FieldKind::Array { name, .. }
            | FieldKind::Assert { name, .. }
            | FieldKind::Checksum { name, .. }
            | FieldKind::Const { name, .. }
            | FieldKind::Discriminator { name, .. }
            | FieldKind::Enum { name, .. }
            | FieldKind::Implicit { name, .. }
            | FieldKind::ManualArray { name, .. }
            | FieldKind::Manual { name, .. }
            | FieldKind::Optional { name, .. }
            | FieldKind::Padding { name, .. }
            | FieldKind::Peek { name, .. }
            | FieldKind::Simple { name, .. }
            | FieldKind::State { name }
            | FieldKind::Virtual { name, .. } => visit(name),
            FieldKind::Reserved { .. }
            | FieldKind::Unknown { .. }
            | FieldKind::Validation { .. } => {}
        }
    }
}
