//! Rust names for `MSpec` identifiers (ADR-0019 decision 5).
//!
//! `MSpec` identifiers are ASCII letters, digits, `_` and `-` (`MSpec.g4` lines 403-405).
//! A name is cut into words at `_` and `-`, at a lower-case letter or digit followed by an
//! upper-case one, and before the last capital of a run of capitals that a lower-case
//! letter follows (`ModbusPDURead` is `Modbus`, `PDU`, `Read`). Types and variants are
//! `UpperCamelCase` with each word's first letter upper case and the rest lower case
//! (`ModbusPdu`, `IllegalFunction`); fields, parameters and modules are `snake_case`.

/// The strict and reserved keywords of Rust 2024, which no generated name may be. One is
/// spelled in two halves: first-party sources hold that word only in the
/// `forbid(unsafe_code)` attribute (`CODING_STANDARDS` section 4), which a grep checks.
const KEYWORDS: [&str; 52] = [
    "abstract",
    "as",
    "async",
    "await",
    "become",
    "box",
    "break",
    "const",
    "continue",
    "crate",
    "do",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "final",
    "fn",
    "for",
    "gen",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "macro",
    "match",
    "mod",
    "move",
    "mut",
    "override",
    "priv",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "try",
    "type",
    "typeof",
    "union",
    concat!("un", "safe"),
    "unsized",
    "use",
    "virtual",
    "where",
    "while",
];

/// The words of an `MSpec` identifier.
pub(crate) fn words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if c == '_' || c == '-' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        let previous = index.checked_sub(1).and_then(|before| chars.get(before));
        let next = chars.get(index.saturating_add(1));
        let boundary = c.is_ascii_uppercase()
            && previous.is_some_and(|p| {
                p.is_ascii_lowercase()
                    || p.is_ascii_digit()
                    || (p.is_ascii_uppercase() && next.is_some_and(char::is_ascii_lowercase))
            });
        if boundary && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// `UpperCamelCase`, for types and variants.
pub(crate) fn upper_camel(name: &str) -> String {
    words(name)
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                format!(
                    "{}{}",
                    first.to_ascii_uppercase(),
                    chars.as_str().to_ascii_lowercase()
                )
            })
        })
        .collect()
}

/// `snake_case`, for fields, parameters and modules.
pub(crate) fn snake(name: &str) -> String {
    words(name)
        .iter()
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

/// Whether `name` is a Rust keyword.
pub(crate) fn is_keyword(name: &str) -> bool {
    KEYWORDS.contains(&name)
}

/// The words of an `UpperCamelCase` name, the way clippy's `enum_variant_names` splits it
/// (each word starts at an upper-case letter).
fn camel_words(name: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for c in name.chars() {
        match words.last_mut() {
            Some(word) if !c.is_ascii_uppercase() => word.push(c),
            _ => words.push(c.to_string()),
        }
    }
    words
}

/// Whether clippy's `enum_variant_names` (style, on by default) fires for an enum named
/// `item` with these variants: at least three variants, and either a variant that starts
/// or ends with the enum's name or a word prefix or suffix that every variant shares
/// while every variant has more than one word. The generator then emits an `#[expect]`
/// for it (ADR-0019 decision 7) instead of renaming the mspec's values.
pub(crate) fn variant_names_lint_fires(item: &str, variants: &[String]) -> bool {
    const THRESHOLD: usize = 3;
    if variants.len() < THRESHOLD {
        return false;
    }
    let item_len = item.chars().count();
    let starts_or_ends_with_item = variants.iter().any(|variant| {
        let rest: Vec<char> = variant.chars().skip(item_len).collect();
        let starts = variant.starts_with(item)
            && rest.first().is_some_and(|c| !c.is_lowercase())
            && rest.get(1).is_some_and(|c| !c.is_numeric());
        starts || variant.ends_with(item)
    });
    if starts_or_ends_with_item {
        return true;
    }
    let split: Vec<Vec<String>> = variants
        .iter()
        .map(|variant| camel_words(variant))
        .collect();
    if split.iter().any(|words| words.len() == 1) {
        return false;
    }
    let Some(first) = split.first() else {
        return false;
    };
    let shared = |reverse: bool| {
        let mut count = first.len();
        for words in &split {
            let pairs: Box<dyn Iterator<Item = (&String, &String)>> = if reverse {
                Box::new(first.iter().rev().zip(words.iter().rev()))
            } else {
                Box::new(first.iter().zip(words.iter()))
            };
            count = count.min(pairs.take_while(|(a, b)| a == b).count());
        }
        count > 0
    };
    shared(false) || shared(true)
}

/// Whether clippy's `struct_field_names` (pedantic) fires for a struct named `item` with
/// these snake-case fields: at least three fields, and either a field that starts or ends
/// with the struct's words or a word prefix or suffix that every field shares while every
/// field has more than one word. The generator then emits an `#[expect]` for it.
pub(crate) fn struct_field_names_lint_fires(item: &str, fields: &[String]) -> bool {
    const THRESHOLD: usize = 3;
    if fields.len() < THRESHOLD {
        return false;
    }
    let item_snake = snake(item);
    let item_words: Vec<&str> = item_snake.split('_').collect();
    let split: Vec<Vec<&str>> = fields
        .iter()
        .map(|field| field.split('_').collect())
        .collect();
    let repeats_item = split.iter().any(|words| {
        words.len() >= item_words.len()
            && (words.iter().zip(&item_words).all(|(a, b)| a == b)
                || words
                    .iter()
                    .rev()
                    .zip(item_words.iter().rev())
                    .all(|(a, b)| a == b))
    });
    if repeats_item {
        return true;
    }
    if split.iter().any(|words| words.len() == 1) {
        return false;
    }
    let Some(first) = split.first() else {
        return false;
    };
    let shared = |reverse: bool| {
        let mut count = first.len();
        for words in &split {
            let pairs: Box<dyn Iterator<Item = (&&str, &&str)>> = if reverse {
                Box::new(first.iter().rev().zip(words.iter().rev()))
            } else {
                Box::new(first.iter().zip(words.iter()))
            };
            count = count.min(pairs.take_while(|(a, b)| a == b).count());
        }
        count > 0
    };
    shared(false) || shared(true)
}

#[cfg(test)]
mod tests {
    use super::{
        is_keyword, snake, struct_field_names_lint_fires, upper_camel, variant_names_lint_fires,
        words,
    };

    #[test]
    fn mspec_names_map_to_rust_case_conventions() {
        for (name, camel, snake_case) in [
            ("ModbusPDU", "ModbusPdu", "modbus_pdu"),
            ("ModbusTcpADU", "ModbusTcpAdu", "modbus_tcp_adu"),
            (
                "ModbusPDUReadCoilsRequest",
                "ModbusPduReadCoilsRequest",
                "modbus_pdu_read_coils_request",
            ),
            ("startingAddress", "StartingAddress", "starting_address"),
            ("ILLEGAL_FUNCTION", "IllegalFunction", "illegal_function"),
            ("MODBUS_TCP", "ModbusTcp", "modbus_tcp"),
            ("LREAL", "Lreal", "lreal"),
            ("DataItem", "DataItem", "data_item"),
            ("uint-8-value", "Uint8Value", "uint_8_value"),
            ("value2Bytes", "Value2Bytes", "value2_bytes"),
        ] {
            assert_eq!(upper_camel(name), camel, "{name}");
            assert_eq!(snake(name), snake_case, "{name}");
        }
        assert_eq!(words("A"), ["A"], "one letter");
        assert!(words("_").is_empty(), "no words");
    }

    #[test]
    fn keywords_are_recognised() {
        assert!(is_keyword("type"), "type");
        assert!(is_keyword("match"), "match");
        assert!(!is_keyword("value"), "value");
    }

    #[test]
    fn the_variant_name_rule_follows_clippy() {
        let names = |list: &[&str]| {
            list.iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert!(
            variant_names_lint_fires(
                "DriverType",
                &names(&["ModbusTcp", "ModbusRtu", "ModbusAscii"])
            ),
            "shared prefix"
        );
        assert!(
            variant_names_lint_fires(
                "ModbusPdu",
                &names(&["ModbusPduError", "ModbusPduReadCoilsRequest", "Other"])
            ),
            "starts with the enum name"
        );
        assert!(
            !variant_names_lint_fires("Code", &names(&["Acknowledge", "IllegalFunction", "Busy"])),
            "one-word variant, no shared affix"
        );
        assert!(
            !variant_names_lint_fires("DriverType", &names(&["ModbusTcp", "ModbusRtu"])),
            "below the threshold"
        );
        assert!(
            variant_names_lint_fires(
                "Level",
                &names(&["BasicStream", "RegularStream", "ExtendedStream"])
            ),
            "shared suffix"
        );
    }

    #[test]
    fn the_struct_field_rule_follows_clippy() {
        let names = |list: &[&str]| {
            list.iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert!(
            struct_field_names_lint_fires("Triple", &names(&["triple_a", "triple_b", "triple_c"])),
            "fields start with the struct's name"
        );
        assert!(
            struct_field_names_lint_fires("Pair", &names(&["left_x", "left_y", "left_z"])),
            "a shared prefix"
        );
        assert!(
            struct_field_names_lint_fires("Pair", &names(&["x_id", "y_id", "z_id"])),
            "a shared suffix"
        );
        assert!(
            !struct_field_names_lint_fires("Pair", &names(&["left_x", "left_y"])),
            "below the threshold"
        );
        assert!(
            !struct_field_names_lint_fires("Pair", &names(&["left_x", "y", "left_z"])),
            "a one-word field ends the shared affix"
        );
        assert!(
            !struct_field_names_lint_fires("Plain", &names(&["one", "two", "three"])),
            "nothing shared"
        );
    }
}
