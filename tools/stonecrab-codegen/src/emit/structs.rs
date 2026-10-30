//! Structs: a `type` becomes a struct of its fields with `parse`, `serialize` and, when an
//! expression reads its `lengthInBytes`, `length_in_bytes`.

use std::collections::{BTreeMap, BTreeSet};

use super::switch::{bind_struct, tail};
use super::{
    Binding, Ctx, GenError, Imports, Part, Ty, VISIBILITY_EXPECT, fields_of, gen_error, line,
    module,
};
use crate::mspec::{FieldKind, TypeDef};
use crate::naming::{struct_field_names_lint_fires, upper_camel};

impl Ctx<'_> {
    pub(super) fn emit_struct(&self, def: &TypeDef) -> Result<String, GenError> {
        let mut imports = Imports::default();
        imports
            .codec
            .extend(["BoundedReader", "BoundedWriter", "ParseError"]);
        let name = upper_camel(&def.name);
        let items = fields_of(&def.fields);
        if let Some(field) = items
            .iter()
            .find(|field| matches!(field.kind, FieldKind::Discriminator { .. }))
        {
            return Err(gen_error(
                def,
                field.line,
                "a `discriminator` field in a `type` is not emitted (only a discriminatedType switches)",
            ));
        }
        let params = self.params(def)?;
        for (_, ty) in &params {
            if let Ty::Enum(enum_name) = ty {
                imports.siblings.insert(enum_name.clone());
            }
        }
        let parts = self.plan(def, &items)?;
        let stored: Vec<(String, Ty)> = parts.iter().flat_map(Part::stored).collect();
        if stored.is_empty() {
            return Err(gen_error(
                def,
                def.line,
                "a `type` that stores no field is not emitted",
            ));
        }
        let mut seen = BTreeSet::new();
        for (field, ty) in &stored {
            if !seen.insert(field.clone()) || params.iter().any(|(param, _)| param == field) {
                return Err(gen_error(
                    def,
                    def.line,
                    &format!("field name `{field}` collides"),
                ));
            }
            if let Some(other) = ty.sibling() {
                imports.siblings.insert(other.to_owned());
            }
        }

        let mut out = String::new();
        let eq = stored.iter().all(|(_, ty)| !ty.has_float());
        line(
            &mut out,
            if eq {
                "#[derive(Debug, Clone, PartialEq, Eq)]"
            } else {
                "#[derive(Debug, Clone, PartialEq)]"
            },
        );
        line(&mut out, VISIBILITY_EXPECT);
        let field_names: Vec<String> = stored.iter().map(|(field, _)| field.clone()).collect();
        if struct_field_names_lint_fires(&name, &field_names) {
            line(
                &mut out,
                "#[expect(clippy::struct_field_names, reason = \"the fields are the mspec's field names\")]",
            );
        }
        line(&mut out, &format!("pub(crate) struct {name} {{"));
        for (field, ty) in &stored {
            line(&mut out, &format!("pub(crate) {field}: {},", ty.rust()));
        }
        line(&mut out, "}");
        line(&mut out, "");
        line(&mut out, &format!("impl {name} {{"));

        // parse
        let mut used_params = BTreeSet::new();
        let mut scope: BTreeMap<String, Binding> = params
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
        let mut body = String::new();
        let mut run_index = 0_u32;
        self.emit_parse_parts(
            def,
            &parts,
            &mut body,
            &mut scope,
            &mut run_index,
            &mut imports,
            &mut used_params,
        )?;
        self.emit_implicit_checks(
            def,
            def.line,
            &parts,
            &def.fields,
            &scope,
            &mut body,
            &mut used_params,
        )?;
        line(
            &mut body,
            &format!("Ok(Self {{ {} }})", field_names.join(", ")),
        );
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
        let mut statements = String::new();
        self.emit_serialize_parts(
            def,
            def.line,
            &parts,
            &BTreeMap::new(),
            &mut statements,
            &scope,
            &mut imports,
        )?;
        line(
            &mut out,
            "pub(crate) fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {",
        );
        line(&mut out, &format!("{} = self;", bind_struct(&stored, None)));
        out.push_str(&tail(&statements));
        line(&mut out, "}");

        if self.sized.contains(&def.name) {
            let all: Vec<&Part<'_>> = parts.iter().collect();
            let (length, used) = self.length_body(def, def.line, &all)?;
            line(&mut out, "");
            // A fixed length is a `const fn` that still takes `&self` and returns a `Result`,
            // so a caller asks the value and adds it to a fallible sum like any other length.
            let keyword = if used.is_empty() {
                line(
                    &mut out,
                    "#[expect(clippy::unused_self, clippy::unnecessary_wraps, reason = \"every length is asked of the value and may fail\")]",
                );
                "const fn"
            } else {
                "fn"
            };
            line(
                &mut out,
                &format!(
                    "pub(crate) {keyword} length_in_bytes(&self) -> Result<usize, ParseError> {{"
                ),
            );
            if !used.is_empty() {
                line(
                    &mut out,
                    &format!("{} = self;", bind_struct(&stored, Some(&used))),
                );
            }
            line(&mut out, &length);
            line(&mut out, "}");
        }
        line(&mut out, "}");
        Ok(module(&imports, &self.options.support_module, &out))
    }
}
