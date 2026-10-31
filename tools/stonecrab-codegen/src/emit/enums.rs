//! Enums: the variants, the wire value and constant accessors, and `parse` and
//! `serialize` for an enum read from the wire.

use super::{
    Ctx, GenError, Imports, Ty, VISIBILITY_EXPECT, check_fits, check_name, describe, emit_match,
    gen_error, integer_text, line, merge_arms, module, scalar,
};
use crate::mspec::{TypeDef, TypeRef};
use crate::naming::{snake, upper_camel, variant_names_lint_fires};

impl Ctx<'_> {
    pub(super) fn emit_enum(&self, def: &TypeDef) -> Result<String, GenError> {
        let mut imports = Imports::default();
        let name = upper_camel(&def.name);
        let base = match &def.enum_type {
            None => None,
            Some(ty) => match scalar(ty) {
                Some((ty, bits)) if ty != Ty::Bool => Some((ty, bits)),
                _ => {
                    return Err(gen_error(
                        def,
                        def.line,
                        &format!("enum base type `{}` is not emitted", describe(ty)),
                    ));
                }
            },
        };
        let mut variants: Vec<String> = Vec::new();
        let mut wire_values: Vec<(String, u128, String)> = Vec::new();
        for value in &def.values {
            let variant = upper_camel(&value.name);
            check_name(def, value.line, &variant, &value.name, false)?;
            if variants.contains(&variant) {
                return Err(gen_error(
                    def,
                    value.line,
                    &format!("enum value `{}` collides as `{variant}`", value.name),
                ));
            }
            if let Some((ty, _)) = &base {
                let Some(raw) = &value.value else {
                    return Err(gen_error(
                        def,
                        value.line,
                        &format!(
                            "enum value `{}` has no wire value (upstream's counting is not emitted)",
                            value.name
                        ),
                    ));
                };
                let (text, number) = integer_text(raw).ok_or_else(|| {
                    gen_error(def, value.line, "an enum value must be an integer")
                })?;
                check_fits(def, value.line, number, ty, def.enum_type.as_ref())?;
                if wire_values.iter().any(|(_, other, _)| *other == number) {
                    return Err(gen_error(
                        def,
                        value.line,
                        &format!("enum value `{}` repeats wire value {text}", value.name),
                    ));
                }
                wire_values.push((variant.clone(), number, text));
            }
            variants.push(variant);
        }
        let mut out = String::new();
        line(&mut out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]");
        line(&mut out, VISIBILITY_EXPECT);
        if variant_names_lint_fires(&name, &variants) {
            line(
                &mut out,
                "#[expect(clippy::enum_variant_names, reason = \"the variants are the mspec's value names\")]",
            );
        }
        line(&mut out, &format!("pub(crate) enum {name} {{"));
        for variant in &variants {
            line(&mut out, &format!("{variant},"));
        }
        line(&mut out, "}");
        let mut methods = String::new();
        if let Some((ty, bits)) = &base {
            let rust = ty.rust();
            line(
                &mut methods,
                &format!("pub(crate) const fn value(self) -> {rust} {{"),
            );
            let arms: Vec<(String, String)> = wire_values
                .iter()
                .map(|(variant, _, text)| (format!("Self::{variant}"), text.clone()))
                .collect();
            emit_match(&mut methods, "self", &merge_arms(arms));
            line(&mut methods, "}");
            line(&mut methods, "");
            if self.wire_enums.contains(&def.name) {
                self.emit_enum_wire(def, &mut methods, ty, *bits, &wire_values, &mut imports)?;
            }
        }
        emit_enum_constants(def, &mut methods, &variants)?;
        if !methods.is_empty() {
            line(&mut out, "");
            line(&mut out, &format!("impl {name} {{"));
            out.push_str(&methods);
            line(&mut out, "}");
        }
        Ok(module(&imports, self.options, &out))
    }

    fn emit_enum_wire(
        &self,
        def: &TypeDef,
        out: &mut String,
        ty: &Ty,
        bits: u32,
        values: &[(String, u128, String)],
        imports: &mut Imports,
    ) -> Result<(), GenError> {
        let Some((suffix, ordered)) = ty.accessor().filter(|_| bits.is_multiple_of(8)) else {
            return Err(gen_error(
                def,
                def.line,
                "an enum read from the wire must have a byte-aligned base type",
            ));
        };
        let order = if ordered {
            imports.byte_order = true;
            self.order().to_owned()
        } else {
            String::new()
        };
        imports
            .codec
            .extend(["BoundedReader", "BoundedWriter", "ParseError"]);
        line(
            out,
            "pub(crate) fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {",
        );
        line(out, &format!("let value = reader.read_{suffix}({order})?;"));
        line(out, "match value {");
        for (variant, _, text) in values {
            line(out, &format!("{text} => Ok(Self::{variant}),"));
        }
        line(
            out,
            &format!(
                "_ => Err(ParseError::InvalidDiscriminator {{ field: \"{}\", value: u64::from(value) }}),",
                def.name
            ),
        );
        line(out, "}");
        line(out, "}");
        line(out, "");
        line(
            out,
            "pub(crate) fn serialize(self, writer: &mut BoundedWriter) -> Result<(), ParseError> {",
        );
        let separator = if order.is_empty() { "" } else { ", " };
        line(
            out,
            &format!("writer.write_{suffix}(self.value(){separator}{order})"),
        );
        line(out, "}");
        line(out, "");
        Ok(())
    }
}

/// One `const fn` per enum parameter, returning each value's constant.
fn emit_enum_constants(
    def: &TypeDef,
    out: &mut String,
    variants: &[String],
) -> Result<(), GenError> {
    for (index, param) in def.params.iter().enumerate() {
        let TypeRef::Simple(data_type) = &param.ty else {
            return Err(gen_error(
                def,
                def.line,
                &format!("enum constant `{}` must have a base type", param.name),
            ));
        };
        let Some((ty, _)) =
            scalar(data_type).filter(|(ty, _)| ty.widens_to_u64() && *ty != Ty::Bool)
        else {
            return Err(gen_error(
                def,
                def.line,
                &format!(
                    "enum constant `{}` of type `{}` is not emitted",
                    param.name,
                    describe(data_type)
                ),
            ));
        };
        let accessor = snake(&param.name);
        check_name(def, def.line, &accessor, &param.name, false)?;
        let mut arms = Vec::new();
        for (value, variant) in def.values.iter().zip(variants) {
            let Some(raw) = value.constants.get(index) else {
                return Err(gen_error(
                    def,
                    value.line,
                    &format!(
                        "enum value `{}` lacks constant `{}`",
                        value.name, param.name
                    ),
                ));
            };
            let (text, number) = integer_text(raw)
                .ok_or_else(|| gen_error(def, value.line, "an enum constant must be an integer"))?;
            check_fits(def, value.line, number, &ty, Some(data_type))?;
            arms.push((format!("Self::{variant}"), text));
        }
        line(
            out,
            &format!("pub(crate) const fn {accessor}(self) -> {} {{", ty.rust()),
        );
        emit_match(out, "self", &merge_arms(arms));
        line(out, "}");
        line(out, "");
    }
    Ok(())
}
