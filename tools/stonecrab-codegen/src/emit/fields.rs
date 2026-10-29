//! Field lists: the plan of a case's fields (sub-byte runs, scalars, constants, implicit
//! fields, byte arrays, enums and nested types), their reads and writes, and the
//! expression subset of the feature matrix.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    BYTE_ORDER, Binding, Ctx, DiscValue, GenError, Imports, Ty, check_fits, check_name, describe,
    gen_error, line, parse_integer, scalar,
};
use crate::expression::{BinaryOp, Expr, Variable};
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
    /// A `byte` array with a `count` expression.
    Bytes { name: String, count: &'a Expr },
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

impl Part<'_> {
    /// The fields this part stores in its variant, in order.
    pub(super) fn stored(&self) -> Vec<(String, Ty)> {
        match self {
            Self::Run(members) => members
                .iter()
                .map(|member| (member.name.clone(), member.ty.clone()))
                .collect(),
            Self::Scalar { name, ty, .. } => vec![(name.clone(), ty.clone())],
            Self::Bytes { name, .. } => vec![(name.clone(), Ty::Bytes)],
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
            parts.push(self.plan_field(def, field)?);
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
    fn plan_field<'f>(&self, def: &TypeDef, field: &'f Field) -> Result<Part<'f>, GenError> {
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
                        TypeForm::Discriminated => Part::Complex {
                            name: named(name)?,
                            type_name: type_name.clone(),
                            args: args.iter().collect(),
                        },
                        TypeForm::Type | TypeForm::DataIo => {
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
                if !ty.widens_to_usize() {
                    return Err(unsupported("wider than 16 bits"));
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
                ty:
                    TypeRef::Simple(DataType {
                        base: Base::Byte, ..
                    }),
                name,
                loop_type: LoopType::Count,
                expr: MExpr::Expr(count, _),
            } => Part::Bytes {
                name: named(name)?,
                count,
            },
            FieldKind::Array { .. } => {
                return Err(unsupported("other than a `count` array of `byte`"));
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
        let order = BYTE_ORDER.to_owned();
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
                Part::Bytes { name, count } => {
                    let count = self.expr(def, def.line, count, scope, used_params)?;
                    line(
                        out,
                        &format!("let {name} = reader.read_bytes({count}, usize::MAX)?.to_vec();"),
                    );
                    scope.insert(
                        name.clone(),
                        Binding {
                            ty: Ty::Bytes,
                            by_ref: false,
                        },
                    );
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
        let order = BYTE_ORDER.to_owned();
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
                    let value = self.expr(def, line_no, expr, scope, &mut unused)?;
                    line(
                        out,
                        &format!(
                            "let {name} = {}::try_from({value}).map_err(|_overflow| ParseError::Overflow)?;",
                            ty.rust()
                        ),
                    );
                    write_scalar(out, suffix, ordered, name, &order, imports);
                }
                Part::Bytes { name, .. } => line(out, &format!("writer.write_bytes({name})?;")),
                Part::Enum { name, .. } | Part::Complex { name, .. } => {
                    line(out, &format!("{name}.serialize(writer)?;"));
                }
            }
        }
        Ok(())
    }

    /// Emits an integer expression of the subset in the feature matrix, as a `usize`.
    pub(super) fn expr(
        &self,
        def: &TypeDef,
        line_no: usize,
        expr: &Expr,
        scope: &BTreeMap<String, Binding>,
        used_params: &mut BTreeSet<String>,
    ) -> Result<String, GenError> {
        let unsupported = |what: &str| {
            gen_error(
                def,
                line_no,
                &format!("the expression construct {what} is not emitted (feature matrix)"),
            )
        };
        match expr {
            Expr::Number(text) | Expr::Hex(text) => parse_integer(text)
                .map(|_| text.clone())
                .ok_or_else(|| unsupported(&format!("`{text}` (not an integer)"))),
            Expr::Group(inner, indexes) if indexes.is_empty() => {
                self.expr(def, line_no, inner, scope, used_params)
            }
            Expr::Binary(BinaryOp::Add, lhs, rhs) => {
                let lhs = self.expr(def, line_no, lhs, scope, used_params)?;
                let rhs = self.expr(def, line_no, rhs, scope, used_params)?;
                Ok(format!(
                    "{lhs}.checked_add({rhs}).ok_or(ParseError::Overflow)?"
                ))
            }
            Expr::Binary(op, _, _) => Err(unsupported(&format!("`{}`", op.symbol()))),
            Expr::Var(var) => self.variable(def, line_no, var, scope, used_params),
            Expr::Bool(_) => Err(unsupported("a boolean")),
            Expr::Null => Err(unsupported("`null`")),
            Expr::Str(..) => Err(unsupported("a string")),
            Expr::Unary(..) => Err(unsupported("a prefix operator")),
            Expr::Ternary(..) => Err(unsupported("`?:`")),
            Expr::Group(..) => Err(unsupported("an index")),
        }
    }

    fn variable(
        &self,
        def: &TypeDef,
        line_no: usize,
        var: &Variable,
        scope: &BTreeMap<String, Binding>,
        used_params: &mut BTreeSet<String>,
    ) -> Result<String, GenError> {
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
                    match scope.get(&rust) {
                        Some(Binding { ty: Ty::Bytes, .. }) => Ok(format!("{rust}.len()")),
                        _ => Err(fail(&format!(
                            "COUNT of `{}`, which is not a byte array",
                            target.name
                        ))),
                    }
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
        match (&var.rest, &binding.ty) {
            (None, ty) if ty.widens_to_usize() => {
                let value = if binding.by_ref {
                    format!("*{rust}")
                } else {
                    rust
                };
                Ok(format!("usize::from({value})"))
            }
            (Some(property), Ty::Complex(type_name))
                if property.name == "lengthInBytes"
                    && property.args.is_none()
                    && property.indexes.is_empty()
                    && property.rest.is_none() =>
            {
                if !self.sized.contains(type_name) {
                    return Err(fail("lengthInBytes of a type whose length is not emitted"));
                }
                Ok(format!("{rust}.length_in_bytes()?"))
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
