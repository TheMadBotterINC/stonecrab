//! Discriminated types and `dataIo`: the enum of enabled cases, the `typeSwitch` as one
//! `match`, disabled cases refused by name, `serialize` and `length_in_bytes`.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Binding, Ctx, DiscValue, Discriminator, Elem, GenError, Imports, Part, Ty, VISIBILITY_EXPECT,
    check_name, describe, emit_match, fields_of, gen_error, integer_literal, integer_text, line,
    merge_arms, module, scalar,
};
use crate::expression::Expr;
use crate::mspec::{Case, Field, FieldKind, MExpr, TypeDef, TypeForm, TypeRef};
use crate::naming::{snake, upper_camel, variant_names_lint_fires};

impl Ctx<'_> {
    pub(super) fn emit_discriminated(&self, def: &TypeDef) -> Result<String, GenError> {
        let mut imports = Imports::default();
        imports
            .codec
            .extend(["BoundedReader", "BoundedWriter", "ParseError"]);
        let name = upper_camel(&def.name);
        let parent_fields = fields_of(&def.fields);
        let (switch_field, own) = parent_fields
            .split_last()
            .ok_or_else(|| gen_error(def, def.line, "a discriminatedType needs a typeSwitch"))?;
        let FieldKind::TypeSwitch { discriminators, .. } = &switch_field.kind else {
            return Err(gen_error(
                def,
                switch_field.line,
                "fields after the typeSwitch are not emitted yet",
            ));
        };
        for field in own {
            if !matches!(field.kind, FieldKind::Discriminator { .. }) {
                return Err(gen_error(
                    def,
                    field.line,
                    &format!(
                        "a `{}` field of the parent (a struct around the variants) is not emitted yet",
                        field.kind.keyword()
                    ),
                ));
            }
        }
        let parent_parts = self.plan(def, own)?;
        let discs = self.discriminators(def, discriminators, own, switch_field.line)?;
        let params = self.params(def)?;
        for (_, ty) in &params {
            if let Ty::Enum(enum_name) = ty {
                imports.siblings.insert(enum_name.clone());
            }
        }
        let cases = self.enabled_cases(def)?;
        let mut used_params = BTreeSet::new();
        for disc in &discs {
            if !disc.is_field {
                used_params.insert(disc.name.clone());
            }
        }

        // The variants.
        let mut variants = Vec::new();
        let mut planned = Vec::new();
        for (case, on) in &cases {
            if !on {
                continue;
            }
            let variant = upper_camel(&case.name);
            check_name(def, case.line, &variant, &case.name)?;
            check_case_form(def, case)?;
            if case.values.len() > discs.len() {
                return Err(gen_error(
                    def,
                    case.line,
                    "more case values than discriminators",
                ));
            }
            let values = self.case_values(def, case, &discs)?;
            let case_fields = fields_of(&case.fields);
            let parts = self.plan(def, &case_fields)?;
            let mut stored: Vec<(String, Ty)> = Vec::new();
            for (disc, value) in discs.iter().zip(&values) {
                if disc.is_field && *value == DiscValue::Stored {
                    stored.push((disc.name.clone(), disc.ty.clone()));
                }
            }
            for part in &parts {
                stored.extend(part.stored());
            }
            let mut seen = BTreeSet::new();
            for (field, _) in &stored {
                if !seen.insert(field.clone()) || params.iter().any(|(param, _)| param == field) {
                    return Err(gen_error(
                        def,
                        case.line,
                        &format!("field name `{field}` collides"),
                    ));
                }
            }
            variants.push(variant.clone());
            planned.push((*case, variant, values, parts, stored));
        }

        let mut out = String::new();
        let eq = planned
            .iter()
            .all(|(_, _, _, _, stored)| stored.iter().all(|(_, ty)| !ty.has_float()));
        line(
            &mut out,
            if eq {
                "#[derive(Debug, Clone, PartialEq, Eq)]"
            } else {
                "#[derive(Debug, Clone, PartialEq)]"
            },
        );
        line(&mut out, VISIBILITY_EXPECT);
        if variant_names_lint_fires(&name, &variants) {
            line(
                &mut out,
                "#[expect(clippy::enum_variant_names, reason = \"the variants are the mspec's case names\")]",
            );
        }
        line(&mut out, &format!("pub(crate) enum {name} {{"));
        for (_, variant, _, _, stored) in &planned {
            if stored.is_empty() {
                line(&mut out, &format!("{variant},"));
            } else {
                let fields: Vec<String> = stored
                    .iter()
                    .map(|(field, ty)| format!("{field}: {}", ty.rust()))
                    .collect();
                line(&mut out, &format!("{variant} {{ {} }},", fields.join(", ")));
            }
            for (_, ty) in stored {
                if let Some(other) = ty.sibling() {
                    imports.siblings.insert(other.to_owned());
                }
            }
        }
        line(&mut out, "}");
        line(&mut out, "");
        line(&mut out, &format!("impl {name} {{"));

        // parse
        let mut body = String::new();
        let mut parse_scope: BTreeMap<String, Binding> = params
            .iter()
            .map(|(param, ty)| {
                (
                    param.clone(),
                    Binding {
                        ty: ty.clone(),
                        by_ref: false,
                    },
                )
            })
            .collect();
        let mut run_index = 0_u32;
        self.emit_parse_parts(
            def,
            &parent_parts,
            &mut body,
            &mut parse_scope,
            &mut run_index,
            &mut imports,
            &mut used_params,
        )?;
        let scrutinee = match discs.as_slice() {
            [one] => one.name.clone(),
            many => format!(
                "({})",
                many.iter()
                    .map(|disc| disc.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        line(&mut body, &format!("match {scrutinee} {{"));
        let mut covered_enum_values = BTreeSet::new();
        for (case, on) in &cases {
            let values = self.case_values(def, case, &discs)?;
            let pattern = pattern(&discs, &values);
            if discs.len() == 1 {
                covered_enum_values.insert(pattern.clone());
            }
            if !on {
                let value = self.error_value(def, case.line, &discs, Some(&values))?;
                line(
                    &mut body,
                    &format!(
                        "{pattern} => Err(ParseError::InvalidDiscriminator {{ field: \"{}.{} (disabled)\", value: {value} }}),",
                        def.name, case.name
                    ),
                );
                continue;
            }
            let Some((_, variant, _, parts, stored)) = planned
                .iter()
                .find(|(planned_case, ..)| std::ptr::eq(*planned_case, *case))
            else {
                continue;
            };
            line(&mut body, &format!("{pattern} => {{"));
            let mut scope = parse_scope.clone();
            self.emit_parse_parts(
                def,
                parts,
                &mut body,
                &mut scope,
                &mut run_index,
                &mut imports,
                &mut used_params,
            )?;
            self.emit_implicit_checks(
                def,
                case.line,
                parts,
                &case.fields,
                &scope,
                &mut body,
                &mut used_params,
            )?;
            line(&mut body, &format!("Ok({})", construct(variant, stored)));
            line(&mut body, "}");
        }
        let needs_rest = match discs.as_slice() {
            [one] => match &one.ty {
                Ty::Enum(enum_name) => {
                    let all = self.enum_patterns(def, enum_name)?;
                    let rest: Vec<String> = all
                        .into_iter()
                        .filter(|value| !covered_enum_values.contains(value))
                        .collect();
                    if rest.is_empty() {
                        None
                    } else {
                        Some(rest.join(" | "))
                    }
                }
                Ty::Bool => {
                    if covered_enum_values.contains("true") && covered_enum_values.contains("false")
                    {
                        None
                    } else {
                        Some("_".to_owned())
                    }
                }
                Ty::U8
                | Ty::U16
                | Ty::U32
                | Ty::U64
                | Ty::I8
                | Ty::I16
                | Ty::I32
                | Ty::I64
                | Ty::F32
                | Ty::F64
                | Ty::Complex(_)
                | Ty::Bytes
                | Ty::Array { .. } => Some("_".to_owned()),
            },
            many => {
                if !many.iter().any(|disc| {
                    matches!(
                        disc.ty,
                        Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64
                    )
                }) {
                    return Err(gen_error(
                        def,
                        switch_field.line,
                        "a switch over several discriminators needs an integer one",
                    ));
                }
                Some("_".to_owned())
            }
        };
        if let Some(rest) = needs_rest {
            let error_value = self.error_value(def, switch_field.line, &discs, None)?;
            line(
                &mut body,
                &format!(
                    "{rest} => Err(ParseError::InvalidDiscriminator {{ field: \"{}\", value: {error_value} }}),",
                    def.name
                ),
            );
        }
        line(&mut body, "}");
        let signature: Vec<String> = params
            .iter()
            .map(|(param, ty)| {
                let shown = if used_params.contains(param) {
                    param.clone()
                } else {
                    format!("_{param}")
                };
                format!("{shown}: {}", ty.rust())
            })
            .collect();
        let extra = if signature.is_empty() {
            String::new()
        } else {
            format!(", {}", signature.join(", "))
        };
        line(
            &mut out,
            &format!(
                "pub(crate) fn parse(reader: &mut BoundedReader<'_>{extra}) -> Result<Self, ParseError> {{"
            ),
        );
        out.push_str(&body);
        line(&mut out, "}");
        line(&mut out, "");

        // serialize
        line(
            &mut out,
            "pub(crate) fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {",
        );
        line(&mut out, "match self {");
        let mut arms = Vec::new();
        for (case, variant, values, parts, stored) in &planned {
            let mut arm = String::new();
            let scope: BTreeMap<String, Binding> = stored
                .iter()
                .map(|(field, ty)| {
                    (
                        field.clone(),
                        Binding {
                            ty: ty.clone(),
                            by_ref: true,
                        },
                    )
                })
                .collect();
            let disc_values: BTreeMap<String, DiscValue> = discs
                .iter()
                .zip(values)
                .filter(|(disc, _)| disc.is_field)
                .map(|(disc, value)| (disc.name.clone(), value.clone()))
                .collect();
            self.emit_serialize_parts(
                def,
                case.line,
                &parent_parts,
                &disc_values,
                &mut arm,
                &scope,
                &mut imports,
            )?;
            self.emit_serialize_parts(
                def,
                case.line,
                parts,
                &BTreeMap::new(),
                &mut arm,
                &scope,
                &mut imports,
            )?;
            arms.push((bind(variant, stored, None), format!("{{\n{}}}", tail(&arm))));
        }
        emit_match(&mut out, "", &merge_arms(arms));
        line(&mut out, "}");
        line(&mut out, "");

        if self.sized.contains(&def.name) {
            self.emit_length(def, &mut out, &parent_parts, &planned)?;
        }
        line(&mut out, "}");
        Ok(module(&imports, &self.options.support_module, &out))
    }

    #[expect(
        clippy::type_complexity,
        reason = "the planned cases of emit_discriminated, passed through"
    )]
    fn emit_length(
        &self,
        def: &TypeDef,
        out: &mut String,
        parent_parts: &[Part<'_>],
        planned: &[(
            &Case,
            String,
            Vec<DiscValue>,
            Vec<Part<'_>>,
            Vec<(String, Ty)>,
        )],
    ) -> Result<(), GenError> {
        line(
            out,
            "pub(crate) fn length_in_bytes(&self) -> Result<usize, ParseError> {",
        );
        line(out, "match self {");
        let mut arms = Vec::new();
        for (case, variant, _, parts, stored) in planned {
            let all: Vec<&Part<'_>> = parent_parts.iter().chain(parts).collect();
            let (body, used) = self.length_body(def, case.line, &all)?;
            arms.push((bind(variant, stored, Some(&used)), body));
        }
        emit_match(out, "", &merge_arms(arms));
        line(out, "}");
        line(out, "");
        Ok(())
    }

    /// The length of a case or struct made of `parts`, as the body of `length_in_bytes`,
    /// and the stored fields that body reads.
    pub(super) fn length_body(
        &self,
        def: &TypeDef,
        line_no: usize,
        parts: &[&Part<'_>],
    ) -> Result<(String, BTreeSet<String>), GenError> {
        let mut bits = 0_u32;
        let mut dynamic: Vec<String> = Vec::new();
        let mut used: BTreeSet<String> = BTreeSet::new();
        for part in parts {
            let (fixed, term) = self.part_length(def, line_no, part)?;
            bits = bits
                .checked_add(fixed)
                .ok_or_else(|| gen_error(def, line_no, "length overflows"))?;
            if let Some((field, term)) = term {
                used.insert(field);
                dynamic.push(term);
            }
        }
        if !bits.is_multiple_of(8) {
            return Err(gen_error(
                def,
                line_no,
                "the case is not a whole number of bytes",
            ));
        }
        let bytes = bits.checked_div(8).unwrap_or(0);
        let body = match dynamic.as_slice() {
            [] => format!("Ok({bytes})"),
            [one] => format!("{bytes}_usize.checked_add({one}).ok_or(ParseError::Overflow)"),
            many => {
                let mut text = format!("{{ let mut size = {bytes}_usize;");
                for term in many {
                    text.push_str("size = size.checked_add(");
                    text.push_str(term);
                    text.push_str(").ok_or(ParseError::Overflow)?;");
                }
                text.push_str("Ok(size) }");
                text
            }
        };
        Ok((body, used))
    }

    /// The fixed bits of a part, and its variable length as a stored field and a term.
    fn part_length(
        &self,
        def: &TypeDef,
        line_no: usize,
        part: &Part<'_>,
    ) -> Result<(u32, Option<(String, String)>), GenError> {
        Ok(match part {
            Part::Run(_) => (8, None),
            Part::Scalar { bits, .. }
            | Part::Const { bits, .. }
            | Part::Implicit { bits, .. }
            | Part::Enum { bits, .. } => (*bits, None),
            Part::Array {
                name,
                elem,
                nullable,
                ..
            } => {
                let count = if *nullable {
                    format!("{name}.as_ref().map_or(0, Vec::len)")
                } else {
                    format!("{name}.len()")
                };
                let term = match elem {
                    Elem::Byte => count,
                    Elem::Number { bits, .. } | Elem::Enum { bits, .. } => {
                        match bits.checked_div(8).unwrap_or(0) {
                            1 => count,
                            width => {
                                format!("{count}.checked_mul({width}).ok_or(ParseError::Overflow)?")
                            }
                        }
                    }
                    Elem::Complex { type_name, .. } => {
                        if !self.sized.contains(type_name) {
                            return Err(gen_error(
                                def,
                                line_no,
                                &format!("the length of `{type_name}` is not emitted"),
                            ));
                        }
                        let items = if *nullable {
                            format!("{name}.iter().flatten()")
                        } else {
                            format!("{name}.iter()")
                        };
                        format!(
                            "{items}.try_fold(0_usize, |size, array_item| array_item.length_in_bytes().and_then(|length| size.checked_add(length).ok_or(ParseError::Overflow)))?"
                        )
                    }
                };
                (0, Some((name.clone(), term)))
            }
            Part::Complex {
                name, type_name, ..
            } => {
                if !self.sized.contains(type_name) {
                    return Err(gen_error(
                        def,
                        line_no,
                        &format!("the length of `{type_name}` is not emitted"),
                    ));
                }
                (
                    0,
                    Some((name.clone(), format!("{name}.length_in_bytes()?"))),
                )
            }
        })
    }

    pub(super) fn emit_data_io(&self, def: &TypeDef) -> Result<String, GenError> {
        let mut imports = Imports::default();
        imports
            .codec
            .extend(["BoundedReader", "BoundedWriter", "ParseError"]);
        imports.plc_value = true;
        if !def.data_io_params.is_empty() {
            return Err(gen_error(
                def,
                def.line,
                "dataIo parameters after the typeSwitch are not emitted yet",
            ));
        }
        let fields = fields_of(&def.fields);
        let Some(FieldKind::TypeSwitch { discriminators, .. }) =
            fields.first().map(|field| &field.kind)
        else {
            return Err(gen_error(def, def.line, "a dataIo needs a typeSwitch"));
        };
        let params = self.params(def)?;
        let discs = self.discriminators(def, discriminators, &[], def.line)?;
        let [disc] = discs.as_slice() else {
            return Err(gen_error(
                def,
                def.line,
                "a dataIo switch over several discriminators is not emitted yet",
            ));
        };
        let Ty::Enum(enum_name) = &disc.ty else {
            return Err(gen_error(
                def,
                def.line,
                "a dataIo switch must be over an enum parameter",
            ));
        };
        let enum_def = self.def(enum_name, def.line, &def.name)?;
        if enum_def.enum_type.is_none() {
            return Err(gen_error(
                def,
                def.line,
                "the dataIo switch enum needs a wire value",
            ));
        }
        let error_value = format!("u64::from({}.value())", disc.name);
        // (value pattern, parse body, PlcValue pattern, serialize body) per case.
        let mut entries: Vec<(String, String, String, String)> = Vec::new();
        let mut covered = BTreeSet::new();
        let order = self.order().to_owned();
        for (case, on) in self.enabled_cases(def)? {
            let values = self.case_values(def, case, &discs)?;
            let pattern = pattern(&discs, &values);
            covered.insert(pattern.clone());
            if !on {
                let body = format!(
                    "Err(ParseError::InvalidDiscriminator {{ field: \"{}.{} (disabled)\", value: {error_value} }})",
                    def.name, case.name
                );
                entries.push((pattern, body.clone(), "_".to_owned(), body));
                continue;
            }
            check_case_form(def, case)?;
            let case_fields = fields_of(&case.fields);
            let [field] = case_fields.as_slice() else {
                return Err(gen_error(
                    def,
                    case.line,
                    "a dataIo case must hold exactly one field",
                ));
            };
            let FieldKind::Simple {
                ty: TypeRef::Simple(data_type),
                name: field_name,
            } = &field.kind
            else {
                return Err(gen_error(
                    def,
                    field.line,
                    &format!(
                        "a `{}` field in a dataIo case is not emitted",
                        field.kind.keyword()
                    ),
                ));
            };
            if !field.attributes.is_empty() {
                return Err(gen_error(
                    def,
                    field.line,
                    "field attributes are not emitted yet",
                ));
            }
            let Some((ty, bits)) = scalar(data_type).filter(|(_, bits)| bits.is_multiple_of(8))
            else {
                return Err(gen_error(
                    def,
                    field.line,
                    &format!(
                        "a `{}` dataIo value is not emitted (not a byte-aligned number)",
                        describe(data_type)
                    ),
                ));
            };
            let (Some(variant), Some((suffix, ordered))) = (ty.plc_value(), ty.accessor()) else {
                return Err(gen_error(
                    def,
                    field.line,
                    "no PlcValue variant for this type",
                ));
            };
            if bits == 0 {
                return Err(gen_error(def, field.line, "zero-width value"));
            }
            let binding = snake(field_name);
            check_name(def, field.line, &binding, field_name)?;
            let read_order = if ordered {
                imports.byte_order = true;
                order.clone()
            } else {
                String::new()
            };
            let write_order = if ordered {
                format!(", {order}")
            } else {
                String::new()
            };
            entries.push((
                pattern,
                format!("Ok(PlcValue::{variant}(reader.read_{suffix}({read_order})?))"),
                format!("PlcValue::{variant}({binding})"),
                format!("writer.write_{suffix}(*{binding}{write_order})"),
            ));
        }
        let rest: Vec<String> = self
            .enum_patterns(def, enum_name)?
            .into_iter()
            .filter(|value| !covered.contains(value))
            .collect();
        let no_case = format!(
            "Err(ParseError::InvalidDiscriminator {{ field: \"{}\", value: {error_value} }})",
            def.name
        );
        // Cases with the same body share an arm (`BYTE | USINT`), nested in the serialize
        // tuple as clippy's `unnested_or_patterns` wants.
        let mut groups: Vec<(Vec<String>, String, String, String)> = Vec::new();
        for (pattern, parse, value, write) in entries {
            if let Some(group) = groups.iter_mut().find(|group| group.1 == parse) {
                group.0.push(pattern);
            } else {
                groups.push((vec![pattern], parse, value, write));
            }
        }
        let mut parse_arms: Vec<(String, String)> = groups
            .iter()
            .map(|(patterns, parse, _, _)| (patterns.join(" | "), parse.clone()))
            .collect();
        let mut write_arms: Vec<(String, String)> = groups
            .iter()
            .map(|(patterns, _, value, write)| {
                (
                    format!("({}, {value})", patterns.join(" | ")),
                    write.clone(),
                )
            })
            .collect();
        if !rest.is_empty() {
            parse_arms.push((rest.join(" | "), no_case.clone()));
        }
        write_arms.push(("_".to_owned(), no_case));
        let signature: Vec<String> = params
            .iter()
            .map(|(param, ty)| {
                let shown = if *param == disc.name {
                    param.clone()
                } else {
                    format!("_{param}")
                };
                format!("{shown}: {}", ty.rust())
            })
            .collect();
        let mut out = String::new();
        line(&mut out, VISIBILITY_EXPECT);
        line(
            &mut out,
            &format!(
                "pub(crate) fn parse(reader: &mut BoundedReader<'_>, {}) -> Result<PlcValue, ParseError> {{",
                signature.join(", ")
            ),
        );
        emit_match(&mut out, &disc.name, &parse_arms);
        line(&mut out, "}");
        line(&mut out, "");
        line(&mut out, VISIBILITY_EXPECT);
        line(
            &mut out,
            &format!(
                "pub(crate) fn serialize(plc_value: &PlcValue, {}, writer: &mut BoundedWriter) -> Result<(), ParseError> {{",
                signature.join(", ")
            ),
        );
        emit_match(
            &mut out,
            &format!("({}, plc_value)", disc.name),
            &write_arms,
        );
        line(&mut out, "}");
        imports.siblings.insert(enum_name.clone());
        Ok(module(&imports, &self.options.support_module, &out))
    }

    /// The `value` of an `InvalidDiscriminator` raised by a switch: the last discriminator
    /// that is not a `bit` (the last one when all are), as a `u64`. An enum without a wire
    /// type has no number at run time, so a disabled case gives the mspec value of its own
    /// literal and the no-match arm is a generation error.
    fn error_value(
        &self,
        def: &TypeDef,
        line_no: usize,
        discs: &[Discriminator],
        values: Option<&[DiscValue]>,
    ) -> Result<String, GenError> {
        let index = discs
            .iter()
            .rposition(|disc| disc.ty != Ty::Bool)
            .or_else(|| discs.len().checked_sub(1))
            .ok_or_else(|| gen_error(def, line_no, "a switch without discriminators"))?;
        let chosen = discs
            .get(index)
            .ok_or_else(|| gen_error(def, line_no, "a switch without discriminators"))?;
        match &chosen.ty {
            ty if ty.widens_to_u64() => Ok(format!("u64::from({})", chosen.name)),
            Ty::Enum(enum_name) => {
                let enum_def = self.def(enum_name, line_no, &def.name)?;
                if enum_def.enum_type.is_some() {
                    return Ok(format!("u64::from({}.value())", chosen.name));
                }
                let rust = upper_camel(enum_name);
                let literal = values
                    .and_then(|values| values.get(index))
                    .and_then(|value| match value {
                        DiscValue::Literal(pattern) => enum_def.values.iter().find(|value| {
                            *pattern == format!("{rust}::{}", upper_camel(&value.name))
                        }),
                        DiscValue::Stored => None,
                    })
                    .and_then(|value| value.value.as_deref().and_then(integer_text));
                literal.map(|(text, _)| text).ok_or_else(|| {
                    gen_error(
                        def,
                        line_no,
                        &format!("`{enum_name}` has no wire type to report in an error"),
                    )
                })
            }
            Ty::Bool
            | Ty::U8
            | Ty::U16
            | Ty::U32
            | Ty::U64
            | Ty::I8
            | Ty::I16
            | Ty::I32
            | Ty::I64
            | Ty::F32
            | Ty::F64
            | Ty::Complex(_)
            | Ty::Bytes
            | Ty::Array { .. } => Err(gen_error(
                def,
                line_no,
                "the switch's error value is not emitted",
            )),
        }
    }

    /// Every value of an enum as a pattern, in source order.
    pub(super) fn enum_patterns(
        &self,
        def: &TypeDef,
        enum_name: &str,
    ) -> Result<Vec<String>, GenError> {
        let enum_def = self.def(enum_name, def.line, &def.name)?;
        let rust = upper_camel(enum_name);
        Ok(enum_def
            .values
            .iter()
            .map(|value| format!("{rust}::{}", upper_camel(&value.name)))
            .collect())
    }

    /// The type parameters as Rust names and types.
    pub(super) fn params(&self, def: &TypeDef) -> Result<Vec<(String, Ty)>, GenError> {
        let mut params = Vec::new();
        for param in &def.params {
            let name = snake(&param.name);
            check_name(def, def.line, &name, &param.name)?;
            let ty = match &param.ty {
                TypeRef::Simple(data_type) => match scalar(data_type) {
                    Some((ty, bits)) if bits == 1 || bits.is_multiple_of(8) => ty,
                    _ => {
                        return Err(gen_error(
                            def,
                            def.line,
                            &format!("parameter type `{}` is not emitted", describe(data_type)),
                        ));
                    }
                },
                TypeRef::Complex {
                    name: type_name, ..
                } => {
                    let param_def = self.def(type_name, def.line, &def.name)?;
                    if param_def.form != TypeForm::Enum {
                        return Err(gen_error(
                            def,
                            def.line,
                            "only enum and base-type parameters are emitted",
                        ));
                    }
                    Ty::Enum(type_name.clone())
                }
            };
            params.push((name, ty));
        }
        Ok(params)
    }

    /// The switch discriminators: parameters or discriminator fields of the parent.
    fn discriminators(
        &self,
        def: &TypeDef,
        names: &[String],
        own: &[&Field],
        line_no: usize,
    ) -> Result<Vec<Discriminator>, GenError> {
        let params = self.params(def)?;
        let mut discs = Vec::new();
        for name in names {
            let rust = snake(name);
            if let Some((_, ty)) = params.iter().find(|(param, _)| *param == rust) {
                discs.push(Discriminator {
                    name: rust,
                    ty: ty.clone(),
                    is_field: false,
                });
                continue;
            }
            let field = own.iter().find(|field| {
                matches!(&field.kind, FieldKind::Discriminator { name: field_name, .. } if field_name == name)
            });
            let Some(FieldKind::Discriminator { ty, .. }) = field.map(|field| &field.kind) else {
                return Err(gen_error(
                    def,
                    line_no,
                    &format!(
                        "switch discriminator `{name}` is not a parameter or discriminator field"
                    ),
                ));
            };
            let ty = match ty {
                TypeRef::Simple(data_type) => scalar(data_type)
                    .map(|(ty, _)| ty)
                    .ok_or_else(|| gen_error(def, line_no, "discriminator type not emitted"))?,
                TypeRef::Complex {
                    name: enum_name, ..
                } => Ty::Enum(enum_name.clone()),
            };
            discs.push(Discriminator {
                name: rust,
                ty,
                is_field: true,
            });
        }
        for field in own {
            if let FieldKind::Discriminator { name, .. } = &field.kind
                && !names.contains(name)
            {
                return Err(gen_error(
                    def,
                    field.line,
                    &format!("discriminator `{name}` is not switched on"),
                ));
            }
        }
        Ok(discs)
    }

    /// The value each discriminator takes in `case`.
    fn case_values(
        &self,
        def: &TypeDef,
        case: &Case,
        discs: &[Discriminator],
    ) -> Result<Vec<DiscValue>, GenError> {
        let mut values = Vec::new();
        for (index, disc) in discs.iter().enumerate() {
            let value = match case.values.get(index) {
                None | Some(MExpr::Wildcard) => DiscValue::Stored,
                Some(MExpr::Expr(expr, text)) => {
                    DiscValue::Literal(self.disc_literal(def, case.line, &disc.ty, expr, text)?)
                }
            };
            values.push(value);
        }
        Ok(values)
    }

    fn disc_literal(
        &self,
        def: &TypeDef,
        line_no: usize,
        ty: &Ty,
        expr: &Expr,
        text: &str,
    ) -> Result<String, GenError> {
        match (ty, expr) {
            (Ty::Bool, Expr::Bool(value)) => Ok(value.to_string()),
            (Ty::Enum(enum_name), Expr::Var(var))
                if var.args.is_none() && var.indexes.is_empty() && var.rest.is_none() =>
            {
                let enum_def = self.def(enum_name, line_no, &def.name)?;
                if !enum_def.values.iter().any(|value| value.name == var.name) {
                    return Err(gen_error(
                        def,
                        line_no,
                        &format!("`{}` is not a value of `{enum_name}`", var.name),
                    ));
                }
                Ok(format!(
                    "{}::{}",
                    upper_camel(enum_name),
                    upper_camel(&var.name)
                ))
            }
            (Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64, _) => {
                integer_literal(expr).map(|(text, _)| text).ok_or_else(|| {
                    gen_error(
                        def,
                        line_no,
                        &format!("case value '{text}' is not an integer"),
                    )
                })
            }
            _ => Err(gen_error(
                def,
                line_no,
                &format!("case value '{text}' does not fit its discriminator"),
            )),
        }
    }
}

/// Refuses the case forms the Modbus vectors do not validate: case parameters, a `*Name`
/// case, a default case without values and a `*` value.
fn check_case_form(def: &TypeDef, case: &Case) -> Result<(), GenError> {
    let refused = if !case.params.is_empty() {
        "case parameters are"
    } else if case.name_wildcard {
        "a `*Name` case is"
    } else if case.values.is_empty() {
        "a default case (no values) is"
    } else if case.values.contains(&MExpr::Wildcard) {
        "a `*` case value is"
    } else {
        return Ok(());
    };
    Err(gen_error(
        def,
        case.line,
        &format!("{refused} not emitted yet (feature matrix)"),
    ))
}

/// The match pattern of a case.
fn pattern(discs: &[Discriminator], values: &[DiscValue]) -> String {
    let parts: Vec<String> = values
        .iter()
        .map(|value| match value {
            DiscValue::Literal(literal) => literal.clone(),
            DiscValue::Stored => "_".to_owned(),
        })
        .collect();
    if discs.len() == 1 {
        parts.into_iter().next().unwrap_or_else(|| "_".to_owned())
    } else {
        format!("({})", parts.join(", "))
    }
}

/// Statements that end in a fallible write, with the last write as the block's value
/// (an arm of `Ok(())` after every write would trip clippy's `branches_sharing_code`).
pub(super) fn tail(statements: &str) -> String {
    let trimmed = statements.trim_end();
    match trimmed.strip_suffix("?;") {
        Some(head)
            if !trimmed
                .lines()
                .last()
                .unwrap_or_default()
                .starts_with("let ") =>
        {
            format!("{head}\n")
        }
        _ => format!("{statements}Ok(())\n"),
    }
}

/// `Self::Variant { fields }`, or `Self::Variant` when it stores nothing.
fn construct(variant: &str, stored: &[(String, Ty)]) -> String {
    if stored.is_empty() {
        format!("Self::{variant}")
    } else {
        let names: Vec<&str> = stored.iter().map(|(name, _)| name.as_str()).collect();
        format!("Self::{variant} {{ {} }}", names.join(", "))
    }
}

/// The pattern binding `used` fields of a variant (all of them when `used` is `None`).
fn bind(variant: &str, stored: &[(String, Ty)], used: Option<&BTreeSet<String>>) -> String {
    if stored.is_empty() {
        return format!("Self::{variant}");
    }
    let names: Vec<&str> = stored
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| used.is_none_or(|used| used.contains(*name)))
        .collect();
    if names.len() == stored.len() {
        format!("Self::{variant} {{ {} }}", names.join(", "))
    } else if names.is_empty() {
        format!("Self::{variant} {{ .. }}")
    } else {
        format!("Self::{variant} {{ {}, .. }}", names.join(", "))
    }
}

/// `let Self { fields }` binding `used` fields of a struct (all of them when `used` is
/// `None`); the caller adds ` = self;`.
pub(super) fn bind_struct(stored: &[(String, Ty)], used: Option<&BTreeSet<String>>) -> String {
    let names: Vec<&str> = stored
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| used.is_none_or(|used| used.contains(*name)))
        .collect();
    if names.len() == stored.len() {
        format!("let Self {{ {} }}", names.join(", "))
    } else {
        format!("let Self {{ {}, .. }}", names.join(", "))
    }
}
