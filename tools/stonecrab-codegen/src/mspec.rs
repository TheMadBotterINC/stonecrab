//! The `MSpec` protocol description language: its model and a recursive-descent parser.
//!
//! A hand-written port of upstream's `MSpec.g4` (Apache PLC4X v1.0.0,
//! `code-generation/protocol-base-mspec/src/main/antlr4/.../mspec/MSpec.g4`, lines 21-435);
//! nothing of the grammar file is copied. Every construct of the grammar parses: the
//! `constants`, `globals` and `context` blocks, the four type forms, the twenty field kinds,
//! `batchSet`, the thirteen base data types and attributes. The text between ticks is an
//! expression in the language of `Expression.g4` and goes to [`crate::expression`].
//! Comments are skipped and never kept, so no comment text reaches generated code.

use std::collections::BTreeSet;
use std::fmt;

use crate::expression::{self, Expr};

/// A parse or resolution failure, with the line it was found on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpecError {
    /// The 1-based line in the mspec.
    pub(crate) line: usize,
    /// What went wrong.
    pub(crate) message: String,
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// A whole mspec file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Spec {
    /// The `[constants ...]` block, when present.
    pub(crate) constants: Option<Block>,
    /// The `[globals ...]` block, when present.
    pub(crate) globals: Option<Block>,
    /// The `[context ...]` block, when present.
    pub(crate) context: Option<Block>,
    /// The type definitions in source order.
    pub(crate) types: Vec<TypeDef>,
}

/// A file-level block and the fields in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    /// The line of the block's opening bracket.
    pub(crate) line: usize,
    /// Its fields.
    pub(crate) fields: Vec<Field>,
}

/// The four type forms of `MSpec.g4` line 41-46.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeForm {
    /// `type`
    Type,
    /// `discriminatedType`
    Discriminated,
    /// `enum`
    Enum,
    /// `dataIo`
    DataIo,
}

impl TypeForm {
    /// The keyword as written.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Discriminated => "discriminatedType",
            Self::Enum => "enum",
            Self::DataIo => "dataIo",
        }
    }
}

/// One top-level type definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeDef {
    /// The type form.
    pub(crate) form: TypeForm,
    /// The name as written.
    pub(crate) name: String,
    /// The type parameters.
    pub(crate) params: Vec<Param>,
    /// The attributes after the name and parameters.
    pub(crate) attributes: Vec<Attribute>,
    /// The line of the definition.
    pub(crate) line: usize,
    /// The fields of a `type` or `discriminatedType`, or the one `typeSwitch` of a `dataIo`.
    pub(crate) fields: Vec<Item>,
    /// The base type of an `enum`, when it names one.
    pub(crate) enum_type: Option<DataType>,
    /// The values of an `enum`.
    pub(crate) values: Vec<EnumValue>,
    /// The parameter expressions after the `typeSwitch` of a `dataIo`.
    pub(crate) data_io_params: Vec<MExpr>,
}

/// A type parameter, `type name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Param {
    /// Its type.
    pub(crate) ty: TypeRef,
    /// Its name.
    pub(crate) name: String,
}

/// An attribute, `name='value'`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attribute {
    /// The attribute name.
    pub(crate) name: String,
    /// Its value expression.
    pub(crate) value: MExpr,
}

/// An entry of a field list: a field, or a `batchSet` and the fields in it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "a parse tree built once per file; boxing every field buys nothing"
)]
pub(crate) enum Item {
    /// A field definition.
    Field(Field),
    /// `[batchSet attributes fields]`.
    BatchSet {
        /// The line of the batch set.
        line: usize,
        /// Its attributes.
        attributes: Vec<Attribute>,
        /// Its fields.
        fields: Vec<Field>,
    },
}

/// One field definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    /// The field kind and its parts.
    pub(crate) kind: FieldKind,
    /// The attributes after the field.
    pub(crate) attributes: Vec<Attribute>,
    /// The line of the field.
    pub(crate) line: usize,
}

/// An array loop type (`MSpec.g4` lines 345-349).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoopType {
    /// `count`: the expression is the number of elements.
    Count,
    /// `length`: the expression is the number of bytes.
    Length,
    /// `terminated`: the expression is the end condition.
    Terminated,
}

/// The twenty field kinds of `MSpec.g4` lines 61-162.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FieldKind {
    Abstract {
        ty: TypeRef,
        name: String,
    },
    Array {
        ty: TypeRef,
        name: String,
        loop_type: LoopType,
        expr: MExpr,
    },
    Assert {
        ty: TypeRef,
        name: String,
        condition: MExpr,
    },
    Checksum {
        ty: DataType,
        name: String,
        expr: MExpr,
    },
    Const {
        ty: TypeRef,
        name: String,
        expected: Literal,
    },
    Discriminator {
        ty: TypeRef,
        name: String,
    },
    Enum {
        ty: TypeRef,
        name: String,
        field_name: String,
    },
    Implicit {
        ty: DataType,
        name: String,
        expr: MExpr,
    },
    ManualArray {
        ty: TypeRef,
        name: String,
        loop_type: LoopType,
        loop_expr: MExpr,
        parse: MExpr,
        serialize: MExpr,
        length: MExpr,
    },
    Manual {
        ty: TypeRef,
        name: String,
        parse: MExpr,
        serialize: MExpr,
        length: MExpr,
    },
    Optional {
        ty: TypeRef,
        name: String,
        condition: Option<MExpr>,
    },
    Padding {
        ty: DataType,
        name: String,
        value: MExpr,
        times: MExpr,
    },
    Peek {
        ty: TypeRef,
        name: String,
        offset: Option<MExpr>,
    },
    Reserved {
        ty: DataType,
        expected: MExpr,
    },
    Simple {
        ty: TypeRef,
        name: String,
    },
    State {
        name: String,
    },
    TypeSwitch {
        discriminators: Vec<String>,
        cases: Vec<Case>,
    },
    Unknown {
        ty: DataType,
    },
    Validation {
        condition: MExpr,
        description: Option<String>,
        should_fail: Option<bool>,
    },
    Virtual {
        ty: TypeRef,
        name: String,
        expr: MExpr,
    },
}

impl FieldKind {
    /// The keyword as written.
    pub(crate) const fn keyword(&self) -> &'static str {
        match self {
            Self::Abstract { .. } => "abstract",
            Self::Array { .. } => "array",
            Self::Assert { .. } => "assert",
            Self::Checksum { .. } => "checksum",
            Self::Const { .. } => "const",
            Self::Discriminator { .. } => "discriminator",
            Self::Enum { .. } => "enum",
            Self::Implicit { .. } => "implicit",
            Self::ManualArray { .. } => "manualArray",
            Self::Manual { .. } => "manual",
            Self::Optional { .. } => "optional",
            Self::Padding { .. } => "padding",
            Self::Peek { .. } => "peek",
            Self::Reserved { .. } => "reserved",
            Self::Simple { .. } => "simple",
            Self::State { .. } => "state",
            Self::TypeSwitch { .. } => "typeSwitch",
            Self::Unknown { .. } => "unknown",
            Self::Validation { .. } => "validation",
            Self::Virtual { .. } => "virtual",
        }
    }
}

/// A case of a `typeSwitch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Case {
    /// The discriminator values, in the order of the switch's discriminators; there may be
    /// fewer values than discriminators.
    pub(crate) values: Vec<MExpr>,
    /// `*Name`: upstream prefixes the parent's name.
    pub(crate) name_wildcard: bool,
    /// The case (sub-type) name.
    pub(crate) name: String,
    /// The case's own parameters.
    pub(crate) params: Vec<Param>,
    /// Its fields.
    pub(crate) fields: Vec<Item>,
    /// The line of the case.
    pub(crate) line: usize,
}

/// An enum value, `['value' NAME ['constant', ...]]`.
///
/// The value and the constants are kept as the text between their ticks, not parsed as
/// expressions: upstream's `MessageFormatListener.enterEnumValueDefinition` takes them as
/// raw strings (lines 744-784), and some are not expressions of `Expression.g4` (the binary
/// literals `0b0000` of canopen.mspec, an empty constant in knx-master-data.mspec).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnumValue {
    /// Its wire value as written, when given (upstream counts up from the previous value
    /// when it is not).
    pub(crate) value: Option<String>,
    /// Its name.
    pub(crate) name: String,
    /// The values of the enum's constant parameters as written, in parameter order.
    pub(crate) constants: Vec<String>,
    /// The line of the value.
    pub(crate) line: usize,
}

/// A reference to a type: a complex type with arguments, or a base data type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeRef {
    /// A named type defined in an mspec, with argument expressions.
    Complex {
        /// The type name.
        name: String,
        /// The arguments.
        args: Vec<MExpr>,
    },
    /// A base data type.
    Simple(DataType),
}

/// The thirteen base data types of `MSpec.g4` lines 177-191.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Base {
    Bit,
    Byte,
    Int,
    Vint,
    Uint,
    Vuint,
    Float,
    Ufloat,
    String,
    Vstring,
    Time,
    Date,
    DateTime,
}

impl Base {
    /// The base types in grammar order with their keywords.
    pub(crate) const ALL: [(Self, &'static str); 13] = [
        (Self::Bit, "bit"),
        (Self::Byte, "byte"),
        (Self::Int, "int"),
        (Self::Vint, "vint"),
        (Self::Uint, "uint"),
        (Self::Vuint, "vuint"),
        (Self::Float, "float"),
        (Self::Ufloat, "ufloat"),
        (Self::String, "string"),
        (Self::Vstring, "vstring"),
        (Self::Time, "time"),
        (Self::Date, "date"),
        (Self::DateTime, "dateTime"),
    ];

    fn from_keyword(word: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, keyword)| *keyword == word)
            .map(|(base, _)| *base)
    }

    /// Whether the keyword takes a size (`int 8`).
    const fn sized(self) -> bool {
        matches!(
            self,
            Self::Int | Self::Uint | Self::Float | Self::Ufloat | Self::String
        )
    }
}

/// A base data type with its size or length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataType {
    /// The base keyword.
    pub(crate) base: Base,
    /// The size in bits of `int`, `uint`, `float`, `ufloat` and `string`.
    pub(crate) size: Option<u32>,
    /// The length expression of a `vstring`.
    pub(crate) length: Option<Box<MExpr>>,
}

/// An `MSpec` expression: the text between ticks, or `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MExpr {
    /// `*`, any value.
    Wildcard,
    /// A parsed expression and its text.
    Expr(Expr, String),
}

/// A bare literal (the expected value of a `const` field, `MSpec.g4` lines 243-249).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Literal {
    /// `true` or `false`.
    Bool(bool),
    /// A hexadecimal literal as written.
    Hex(String),
    /// A decimal integer as written.
    Integer(String),
    /// A decimal fraction as written.
    Float(String),
    /// A string literal without its quotes.
    Str(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Open,
    Close,
    ParenOpen,
    ParenClose,
    Comma,
    Equals,
    Star,
    Dot,
    Word(String),
    Str(String),
    Tick(String),
}

/// Splits mspec text into tokens with their lines; comments are dropped.
fn lex(text: &str) -> Result<Vec<(usize, Tok)>, SpecError> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut line = 1_usize;
    let mut at = 0_usize;
    while let Some(&c) = chars.get(at) {
        let next = chars.get(at.saturating_add(1)).copied();
        if c == '\n' {
            line = line.saturating_add(1);
            at = at.saturating_add(1);
        } else if c.is_whitespace() {
            at = at.saturating_add(1);
        } else if c == '/' && next == Some('/') {
            while chars.get(at).is_some_and(|c| *c != '\n') {
                at = at.saturating_add(1);
            }
        } else if c == '/' && next == Some('*') {
            let start = line;
            at = at.saturating_add(2);
            loop {
                match chars.get(at) {
                    None => return Err(error(start, "unterminated block comment")),
                    Some('*') if chars.get(at.saturating_add(1)) == Some(&'/') => {
                        at = at.saturating_add(2);
                        break;
                    }
                    Some('\n') => line = line.saturating_add(1),
                    Some(_) => {}
                }
                at = at.saturating_add(1);
            }
        } else if c == '\'' {
            let (body, end) = lex_tick(&chars, at, line)?;
            tokens.push((line, Tok::Tick(body)));
            at = end;
        } else if c == '"' {
            let (body, end) = lex_quoted(&chars, at, line)?;
            tokens.push((line, Tok::Str(body)));
            at = end;
        } else if is_word_char(c) {
            let mut end = at;
            while chars.get(end).copied().is_some_and(is_word_char) {
                end = end.saturating_add(1);
            }
            let word: String = chars.get(at..end).unwrap_or_default().iter().collect();
            tokens.push((line, Tok::Word(word)));
            at = end;
        } else {
            let token = match c {
                '[' => Tok::Open,
                ']' => Tok::Close,
                '(' => Tok::ParenOpen,
                ')' => Tok::ParenClose,
                ',' => Tok::Comma,
                '=' => Tok::Equals,
                '*' => Tok::Star,
                '.' => Tok::Dot,
                other => return Err(error(line, &format!("unexpected character {other:?}"))),
            };
            tokens.push((line, token));
            at = at.saturating_add(1);
        }
    }
    Ok(tokens)
}

/// `IDENTIFIER_LITERAL`, `INTEGER_LITERAL`, `HEX_LITERAL` and the keywords all use these
/// characters (`MSpec.g4` lines 345-405).
const fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// A `"..."` literal; the grammar allows no escape and no line break in it.
fn lex_quoted(chars: &[char], start: usize, line: usize) -> Result<(String, usize), SpecError> {
    let mut at = start.saturating_add(1);
    let mut body = String::new();
    loop {
        match chars.get(at) {
            None | Some('\r' | '\n' | '\\') => {
                return Err(error(line, "unterminated or escaped string literal"));
            }
            Some('"') => return Ok((body, at.saturating_add(1))),
            Some(c) => body.push(*c),
        }
        at = at.saturating_add(1);
    }
}

/// The text between two ticks. A `"..."` string inside is skipped whole, so a tick in it
/// does not end the expression (it is one `STRING_LITERAL` token to the upstream lexer).
fn lex_tick(chars: &[char], start: usize, line: usize) -> Result<(String, usize), SpecError> {
    let mut at = start.saturating_add(1);
    let mut body = String::new();
    loop {
        match chars.get(at) {
            None => return Err(error(line, "unterminated expression")),
            Some('\'') => return Ok((body, at.saturating_add(1))),
            Some('"') => {
                let (text, end) = lex_quoted(chars, at, line)?;
                body.push('"');
                body.push_str(&text);
                body.push('"');
                at = end;
                continue;
            }
            Some(c) => body.push(*c),
        }
        at = at.saturating_add(1);
    }
}

fn error(line: usize, message: &str) -> SpecError {
    SpecError {
        line,
        message: message.to_owned(),
    }
}

/// Parses one mspec file. Type references are not resolved here; see [`resolve`].
///
/// # Errors
/// A [`SpecError`] at the first token that does not fit the grammar.
pub(crate) fn parse(text: &str) -> Result<Spec, SpecError> {
    let tokens = lex(text)?;
    let mut parser = Parser { tokens, at: 0 };
    parser.spec()
}

struct Parser {
    tokens: Vec<(usize, Tok)>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.peek_at(0)
    }

    fn peek_at(&self, ahead: usize) -> Option<&Tok> {
        self.tokens
            .get(self.at.saturating_add(ahead))
            .map(|(_, token)| token)
    }

    fn line(&self) -> usize {
        self.tokens
            .get(self.at)
            .or_else(|| self.tokens.last())
            .map_or(1, |(line, _)| *line)
    }

    fn bump(&mut self) -> Option<Tok> {
        let token = self.tokens.get(self.at).map(|(_, token)| token.clone());
        self.at = self.at.saturating_add(1);
        token
    }

    fn eat(&mut self, token: &Tok) -> bool {
        if self.peek() == Some(token) {
            self.at = self.at.saturating_add(1);
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Tok, what: &str) -> Result<(), SpecError> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(self.unexpected(what))
        }
    }

    fn unexpected(&self, what: &str) -> SpecError {
        error(
            self.line(),
            &format!("expected {what}, found {:?}", self.peek()),
        )
    }

    fn peek_word(&self) -> Option<&str> {
        match self.peek() {
            Some(Tok::Word(word)) => Some(word),
            _ => None,
        }
    }

    /// `[` followed by the keyword `word`.
    fn at_block(&self, word: &str) -> bool {
        self.peek() == Some(&Tok::Open)
            && matches!(self.peek_at(1), Some(Tok::Word(found)) if found == word)
    }

    fn word(&mut self, what: &str) -> Result<String, SpecError> {
        match self.peek() {
            Some(Tok::Word(word)) => {
                let word = word.clone();
                self.at = self.at.saturating_add(1);
                Ok(word)
            }
            _ => Err(self.unexpected(what)),
        }
    }

    fn keyword(&mut self, keyword: &str) -> Result<(), SpecError> {
        if self.peek_word() == Some(keyword) {
            self.at = self.at.saturating_add(1);
            Ok(())
        } else {
            Err(self.unexpected(&format!("`{keyword}`")))
        }
    }

    fn spec(&mut self) -> Result<Spec, SpecError> {
        let constants = self.block("constants")?;
        let globals = self.block("globals")?;
        let context = self.block("context")?;
        let mut types = Vec::new();
        while self.peek().is_some() {
            types.push(self.type_def()?);
        }
        Ok(Spec {
            constants,
            globals,
            context,
            types,
        })
    }

    /// `[constants [const ...]*]`, `[globals field*]` or `[context field*]`.
    fn block(&mut self, keyword: &str) -> Result<Option<Block>, SpecError> {
        if !self.at_block(keyword) {
            return Ok(None);
        }
        let line = self.line();
        self.bump();
        self.bump();
        let mut fields = Vec::new();
        while !self.eat(&Tok::Close) {
            let field = self.field_definition()?;
            if keyword == "constants" && !matches!(field.kind, FieldKind::Const { .. }) {
                return Err(error(
                    field.line,
                    "a constants block holds const fields only",
                ));
            }
            fields.push(field);
        }
        Ok(Some(Block { line, fields }))
    }

    fn type_def(&mut self) -> Result<TypeDef, SpecError> {
        self.expect(&Tok::Open, "`[` opening a type definition")?;
        let line = self.line();
        let form = match self.word("a type form")?.as_str() {
            "type" => TypeForm::Type,
            "discriminatedType" => TypeForm::Discriminated,
            "enum" => TypeForm::Enum,
            "dataIo" => TypeForm::DataIo,
            other => return Err(error(line, &format!("unknown type form `{other}`"))),
        };
        let enum_type = if form == TypeForm::Enum
            && self.peek_word().and_then(Base::from_keyword).is_some()
            && matches!(self.peek_at(1), Some(Tok::Word(_)))
        {
            Some(self.data_type()?)
        } else {
            None
        };
        let name = self.word("a type name")?;
        let params = self.param_list()?;
        let attributes = self.attributes()?;
        let mut def = TypeDef {
            form,
            name,
            params,
            attributes,
            line,
            fields: Vec::new(),
            enum_type,
            values: Vec::new(),
            data_io_params: Vec::new(),
        };
        match form {
            TypeForm::Type | TypeForm::Discriminated => {
                while !self.eat(&Tok::Close) {
                    def.fields.push(self.item()?);
                }
            }
            TypeForm::Enum => {
                while !self.eat(&Tok::Close) {
                    def.values.push(self.enum_value()?);
                }
            }
            TypeForm::DataIo => {
                // dataIoDefinition: `[typeSwitch ... ([params])? ]` (MSpec.g4 lines 56-59).
                self.expect(&Tok::Open, "`[` opening the dataIo typeSwitch")?;
                let switch_line = self.line();
                self.keyword("typeSwitch")?;
                let (discriminators, cases, params) = self.type_switch_body(true)?;
                def.data_io_params = params;
                def.fields.push(Item::Field(Field {
                    kind: FieldKind::TypeSwitch {
                        discriminators,
                        cases,
                    },
                    attributes: Vec::new(),
                    line: switch_line,
                }));
                self.expect(&Tok::Close, "`]` closing the dataIo typeSwitch")?;
                self.expect(&Tok::Close, "`]` closing the dataIo definition")?;
            }
        }
        Ok(def)
    }

    /// `(type name, ...)`, or nothing.
    fn param_list(&mut self) -> Result<Vec<Param>, SpecError> {
        let mut params = Vec::new();
        if !self.eat(&Tok::ParenOpen) {
            return Ok(params);
        }
        loop {
            let ty = self.type_ref()?;
            let name = self.word("a parameter name")?;
            params.push(Param { ty, name });
            if self.eat(&Tok::ParenClose) {
                return Ok(params);
            }
            self.expect(&Tok::Comma, "`,` or `)` in a parameter list")?;
        }
    }

    /// `name='value'` pairs (`MSpec.g4` lines 193-199).
    fn attributes(&mut self) -> Result<Vec<Attribute>, SpecError> {
        let mut attributes = Vec::new();
        while matches!(self.peek(), Some(Tok::Word(_))) && self.peek_at(1) == Some(&Tok::Equals) {
            let name = self.word("an attribute name")?;
            self.bump();
            let value = self.mexpr()?;
            attributes.push(Attribute { name, value });
        }
        Ok(attributes)
    }

    /// A tick expression or `*`.
    fn mexpr(&mut self) -> Result<MExpr, SpecError> {
        let line = self.line();
        match self.bump() {
            Some(Tok::Star) => Ok(MExpr::Wildcard),
            Some(Tok::Tick(text)) => expression::parse(&text)
                .map(|expr| MExpr::Expr(expr, text.clone()))
                .map_err(|failure| error(line, &format!("in expression '{text}': {failure}"))),
            other => Err(error(
                line,
                &format!("expected a 'quoted' expression, found {other:?}"),
            )),
        }
    }

    fn optional_mexpr(&mut self) -> Result<Option<MExpr>, SpecError> {
        if matches!(self.peek(), Some(Tok::Tick(_))) {
            self.mexpr().map(Some)
        } else {
            Ok(None)
        }
    }

    /// `expression (',' expression)*`
    fn mexprs(&mut self) -> Result<Vec<MExpr>, SpecError> {
        let mut values = vec![self.mexpr()?];
        while self.eat(&Tok::Comma) {
            values.push(self.mexpr()?);
        }
        Ok(values)
    }

    fn data_type(&mut self) -> Result<DataType, SpecError> {
        let word = self.word("a data type")?;
        let Some(base) = Base::from_keyword(&word) else {
            return Err(error(self.line(), &format!("unknown data type `{word}`")));
        };
        let size = if base.sized() {
            let text = self.word("a size in bits")?;
            Some(
                text.parse::<u32>()
                    .map_err(|_not_a_number| error(self.line(), &format!("bad size `{text}`")))?,
            )
        } else {
            None
        };
        let length = if base == Base::Vstring {
            self.optional_mexpr()?.map(Box::new)
        } else {
            None
        };
        Ok(DataType { base, size, length })
    }

    /// `Name ('(' args ')')?` or a base data type (`MSpec.g4` lines 168-171).
    fn type_ref(&mut self) -> Result<TypeRef, SpecError> {
        if self.peek_word().and_then(Base::from_keyword).is_some() {
            return Ok(TypeRef::Simple(self.data_type()?));
        }
        let name = self.word("a type")?;
        let args = if self.eat(&Tok::ParenOpen) {
            let args = self.mexprs()?;
            self.expect(&Tok::ParenClose, "`)` closing type arguments")?;
            args
        } else {
            Vec::new()
        };
        Ok(TypeRef::Complex { name, args })
    }

    fn loop_type(&mut self) -> Result<LoopType, SpecError> {
        match self.word("an array loop type")?.as_str() {
            "count" => Ok(LoopType::Count),
            "length" => Ok(LoopType::Length),
            "terminated" => Ok(LoopType::Terminated),
            other => Err(error(self.line(), &format!("unknown loop type `{other}`"))),
        }
    }

    fn literal(&mut self) -> Result<Literal, SpecError> {
        let line = self.line();
        match self.bump() {
            Some(Tok::Str(text)) => Ok(Literal::Str(text)),
            Some(Tok::Word(word)) if word == "true" || word == "false" => {
                Ok(Literal::Bool(word == "true"))
            }
            Some(Tok::Word(word)) if word.starts_with("0x") || word.starts_with("0X") => {
                Ok(Literal::Hex(word))
            }
            Some(Tok::Word(word)) if word.chars().all(|c| c.is_ascii_digit()) => {
                if self.peek() == Some(&Tok::Dot) {
                    self.bump();
                    let fraction = self.word("the fraction of a number")?;
                    Ok(Literal::Float(format!("{word}.{fraction}")))
                } else {
                    Ok(Literal::Integer(word))
                }
            }
            other => Err(error(line, &format!("expected a literal, found {other:?}"))),
        }
    }

    /// A field or a batch set.
    fn item(&mut self) -> Result<Item, SpecError> {
        if self.at_block("batchSet") {
            let line = self.line();
            self.bump();
            self.bump();
            let attributes = self.attributes()?;
            let mut fields = vec![self.field_definition()?];
            while !self.eat(&Tok::Close) {
                fields.push(self.field_definition()?);
            }
            return Ok(Item::BatchSet {
                line,
                attributes,
                fields,
            });
        }
        self.field_definition().map(Item::Field)
    }

    /// `[ field attributes ]` (`MSpec.g4` line 48-50).
    fn field_definition(&mut self) -> Result<Field, SpecError> {
        self.expect(&Tok::Open, "`[` opening a field")?;
        let line = self.line();
        let kind = self.field_kind()?;
        let attributes = self.attributes()?;
        self.expect(&Tok::Close, "`]` closing a field")?;
        Ok(Field {
            kind,
            attributes,
            line,
        })
    }

    fn field_kind(&mut self) -> Result<FieldKind, SpecError> {
        let keyword = self.word("a field kind")?;
        Ok(match keyword.as_str() {
            "abstract" => FieldKind::Abstract {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
            },
            "array" => FieldKind::Array {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
                loop_type: self.loop_type()?,
                expr: self.mexpr()?,
            },
            "assert" => FieldKind::Assert {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
                condition: self.mexpr()?,
            },
            "checksum" => FieldKind::Checksum {
                ty: self.data_type()?,
                name: self.word("a field name")?,
                expr: self.mexpr()?,
            },
            "const" => FieldKind::Const {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
                expected: self.literal()?,
            },
            "discriminator" => FieldKind::Discriminator {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
            },
            "enum" => FieldKind::Enum {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
                field_name: self.word("the enum's field name")?,
            },
            "implicit" => FieldKind::Implicit {
                ty: self.data_type()?,
                name: self.word("a field name")?,
                expr: self.mexpr()?,
            },
            "manualArray" => FieldKind::ManualArray {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
                loop_type: self.loop_type()?,
                loop_expr: self.mexpr()?,
                parse: self.mexpr()?,
                serialize: self.mexpr()?,
                length: self.mexpr()?,
            },
            "manual" => FieldKind::Manual {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
                parse: self.mexpr()?,
                serialize: self.mexpr()?,
                length: self.mexpr()?,
            },
            "optional" => FieldKind::Optional {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
                condition: self.optional_mexpr()?,
            },
            "padding" => FieldKind::Padding {
                ty: self.data_type()?,
                name: self.word("a field name")?,
                value: self.mexpr()?,
                times: self.mexpr()?,
            },
            "peek" => FieldKind::Peek {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
                offset: self.optional_mexpr()?,
            },
            "reserved" => FieldKind::Reserved {
                ty: self.data_type()?,
                expected: self.mexpr()?,
            },
            "simple" => FieldKind::Simple {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
            },
            "state" => FieldKind::State {
                name: self.word("a field name")?,
            },
            "typeSwitch" => {
                let (discriminators, cases, _) = self.type_switch_body(false)?;
                FieldKind::TypeSwitch {
                    discriminators,
                    cases,
                }
            }
            "unknown" => FieldKind::Unknown {
                ty: self.data_type()?,
            },
            "validation" => self.validation()?,
            "virtual" => FieldKind::Virtual {
                ty: self.type_ref()?,
                name: self.word("a field name")?,
                expr: self.mexpr()?,
            },
            other => {
                return Err(error(self.line(), &format!("unknown field kind `{other}`")));
            }
        })
    }

    /// `validation 'expr' ("description")? (shouldFail=true|false)?` (`MSpec.g4` line 157).
    fn validation(&mut self) -> Result<FieldKind, SpecError> {
        let condition = self.mexpr()?;
        let description = match self.peek() {
            Some(Tok::Str(text)) => {
                let text = text.clone();
                self.bump();
                Some(text)
            }
            _ => None,
        };
        let should_fail =
            if self.peek_word() == Some("shouldFail") && self.peek_at(1) == Some(&Tok::Equals) {
                self.bump();
                self.bump();
                match self.word("true or false")?.as_str() {
                    "true" => Some(true),
                    "false" => Some(false),
                    other => return Err(error(self.line(), &format!("bad shouldFail `{other}`"))),
                }
            } else {
                None
            };
        Ok(FieldKind::Validation {
            condition,
            description,
            should_fail,
        })
    }

    /// `name ('.' name | '[' INTEGER ']')*`, one variable literal of a switch header
    /// (`MSpec.g4` lines 223-227), kept as its text.
    fn variable_literal(&mut self) -> Result<String, SpecError> {
        let mut text = self.word("a discriminator name")?;
        loop {
            if self.peek() == Some(&Tok::Dot) {
                self.bump();
                text.push('.');
                text.push_str(&self.word("a name after `.`")?);
            } else if self.peek() == Some(&Tok::Open)
                && matches!(self.peek_at(1), Some(Tok::Word(index)) if index.chars().all(|c| c.is_ascii_digit()))
                && self.peek_at(2) == Some(&Tok::Close)
            {
                self.bump();
                let index = self.word("an index")?;
                self.bump();
                text.push('[');
                text.push_str(&index);
                text.push(']');
            } else {
                return Ok(text);
            }
        }
    }

    /// The discriminators and cases of a `typeSwitch`; in a `dataIo`, also the optional
    /// trailing `['param', ...]` (`MSpec.g4` lines 56-59, 148-150, 173-175).
    #[expect(
        clippy::type_complexity,
        reason = "the three parts of one grammar rule, used by two callers"
    )]
    fn type_switch_body(
        &mut self,
        data_io: bool,
    ) -> Result<(Vec<String>, Vec<Case>, Vec<MExpr>), SpecError> {
        let mut discriminators = vec![self.variable_literal()?];
        while self.eat(&Tok::Comma) {
            discriminators.push(self.variable_literal()?);
        }
        let mut cases = Vec::new();
        let mut params = Vec::new();
        while self.peek() == Some(&Tok::Open) {
            let line = self.line();
            self.bump();
            // ANTLR's optional rules are greedy: a `*` that opens a case is a wildcard
            // value (`[* Name`, `[* *Name`); a `*` after the values marks the name.
            let mut values = Vec::new();
            if matches!(self.peek(), Some(Tok::Tick(_) | Tok::Star)) {
                values = self.mexprs()?;
            }
            if data_io && self.peek() == Some(&Tok::Close) && !values.is_empty() {
                self.bump();
                params = values;
                break;
            }
            let name_wildcard = self.eat(&Tok::Star);
            let name = self.word("a case name")?;
            let case_params = self.param_list()?;
            let mut fields = Vec::new();
            while !self.eat(&Tok::Close) {
                fields.push(self.item()?);
            }
            cases.push(Case {
                values,
                name_wildcard,
                name,
                params: case_params,
                fields,
                line,
            });
        }
        Ok((discriminators, cases, params))
    }

    /// The text of a tick expression, unparsed, when one comes next.
    fn raw(&mut self) -> Option<String> {
        if let Some(Tok::Tick(text)) = self.peek() {
            let text = text.trim().to_owned();
            self.bump();
            Some(text)
        } else {
            None
        }
    }

    /// `['value']? NAME (['constant', ...])? ]` (`MSpec.g4` lines 164-166).
    fn enum_value(&mut self) -> Result<EnumValue, SpecError> {
        self.expect(&Tok::Open, "`[` opening an enum value")?;
        let line = self.line();
        let value = self.raw();
        let name = self.word("an enum value name")?;
        let mut constants = Vec::new();
        if self.eat(&Tok::Open) {
            loop {
                constants.push(
                    self.raw()
                        .ok_or_else(|| self.unexpected("a 'quoted' enum constant"))?,
                );
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(&Tok::Close, "`]` closing enum constants")?;
        }
        self.expect(&Tok::Close, "`]` closing an enum value")?;
        Ok(EnumValue {
            value,
            name,
            constants,
            line,
        })
    }
}

/// Every name a type reference may use: the top-level types and the case types of every
/// `typeSwitch`, across all `specs` (one protocol may span several files).
pub(crate) fn type_names(specs: &[&Spec]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for spec in specs {
        for def in &spec.types {
            names.insert(def.name.clone());
            collect_case_names(&def.fields, &def.name, &mut names);
        }
    }
    names
}

fn collect_case_names(items: &[Item], parent: &str, names: &mut BTreeSet<String>) {
    for field in fields_of(items) {
        if let FieldKind::TypeSwitch { cases, .. } = &field.kind {
            for case in cases {
                let name = if case.name_wildcard {
                    format!("{parent}{}", case.name)
                } else {
                    case.name.clone()
                };
                collect_case_names(&case.fields, &name, names);
                names.insert(name);
            }
        }
    }
}

/// The fields of a list, batch sets flattened, in source order.
pub(crate) fn fields_of(items: &[Item]) -> Vec<&Field> {
    let mut fields = Vec::new();
    for item in items {
        match item {
            Item::Field(field) => fields.push(field),
            Item::BatchSet { fields: inner, .. } => fields.extend(inner),
        }
    }
    fields
}

/// Fails on the first type reference of `spec` that names no type of `known`, as
/// upstream's `ValidatableTypeContext.validate()` fails with "Unresolved types left".
///
/// # Errors
/// A [`SpecError`] naming the type and the line that refers to it.
pub(crate) fn resolve(spec: &Spec, known: &BTreeSet<String>) -> Result<(), SpecError> {
    let check = |ty: &TypeRef, line: usize| match ty {
        TypeRef::Complex { name, .. } if !known.contains(name) => {
            Err(error(line, &format!("unresolved type reference `{name}`")))
        }
        TypeRef::Complex { .. } | TypeRef::Simple(_) => Ok(()),
    };
    for def in &spec.types {
        for param in &def.params {
            check(&param.ty, def.line)?;
        }
        resolve_items(&def.fields, &check)?;
    }
    for block in [&spec.constants, &spec.globals, &spec.context]
        .into_iter()
        .flatten()
    {
        for field in &block.fields {
            resolve_field(field, &check)?;
        }
    }
    Ok(())
}

fn resolve_items(
    items: &[Item],
    check: &impl Fn(&TypeRef, usize) -> Result<(), SpecError>,
) -> Result<(), SpecError> {
    for field in fields_of(items) {
        resolve_field(field, check)?;
    }
    Ok(())
}

fn resolve_field(
    field: &Field,
    check: &impl Fn(&TypeRef, usize) -> Result<(), SpecError>,
) -> Result<(), SpecError> {
    match &field.kind {
        FieldKind::Abstract { ty, .. }
        | FieldKind::Array { ty, .. }
        | FieldKind::Assert { ty, .. }
        | FieldKind::Const { ty, .. }
        | FieldKind::Discriminator { ty, .. }
        | FieldKind::Enum { ty, .. }
        | FieldKind::ManualArray { ty, .. }
        | FieldKind::Manual { ty, .. }
        | FieldKind::Optional { ty, .. }
        | FieldKind::Peek { ty, .. }
        | FieldKind::Simple { ty, .. }
        | FieldKind::Virtual { ty, .. } => check(ty, field.line),
        FieldKind::TypeSwitch { cases, .. } => {
            for case in cases {
                for param in &case.params {
                    check(&param.ty, case.line)?;
                }
                resolve_items(&case.fields, check)?;
            }
            Ok(())
        }
        FieldKind::Checksum { .. }
        | FieldKind::Implicit { .. }
        | FieldKind::Padding { .. }
        | FieldKind::Reserved { .. }
        | FieldKind::State { .. }
        | FieldKind::Unknown { .. }
        | FieldKind::Validation { .. } => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Base, FieldKind, Item, Literal, LoopType, MExpr, TypeForm, TypeRef, fields_of, parse,
        resolve, type_names,
    };

    const SAMPLE: &str = r#"
/* block comment with a ' tick */
// line comment
[constants
    [const uint 16 port 502]
]
[globals [simple uint 8 g]]
[context [simple uint 8 c]]
[enum uint 8 Color(uint 8 weight) ['0x01' RED ['2']] [BLUE]]
[enum Kind ['1' A]]
[discriminatedType Root(bit response) byteOrder='"BIG_ENDIAN"'
    [discriminator bit flag]
    [discriminator uint 7 code]
    [typeSwitch flag,code,response
        ['true' Failure [simple Color color]]
        ['false','0x01',* *Read(uint 8 extra)
            [implicit uint 8 count 'COUNT(data)']
            [array byte data count 'count']
            [const uint 16 marker 0x0000]
            [batchSet byteOrder='"LITTLE_ENDIAN"'
                [reserved uint 8 '0x00']
                [padding uint 8 pad '0x00' '2']
            ]
        ]
        [Default]
    ]
    [abstract uint 8 a]
    [assert uint 8 b 'b == 1']
    [checksum uint 16 crc 'STATIC_CALL("crc", flag)']
    [enum Color color2 weight]
    [manualArray byte m terminated 'true' 'x' 'y' 'z']
    [manual uint 8 n 'p' 's' 'l']
    [optional uint 8 o 'flag']
    [optional uint 8 p]
    [peek uint 8 q '1']
    [state st]
    [unknown uint 8]
    [validation 'flag' "must hold" shouldFail=false]
    [virtual uint 8 v 'code + 1']
    [simple vstring 'count * 8' s stringEncoding='"UTF8"']
    [simple time t]
]
[dataIo Item(Color color) unsignedIntegerEncoding='"unsigned-binary"'
    [typeSwitch color
        ['RED' RED [simple uint 8 value]]
        ['color']
    ]
]
"#;

    #[test]
    fn every_construct_of_the_grammar_parses() {
        let spec = parse(SAMPLE).expect("the sample parses");
        assert_eq!(
            spec.constants.as_ref().map(|block| block.fields.len()),
            Some(1),
            "constants"
        );
        assert!(spec.globals.is_some(), "globals");
        assert!(spec.context.is_some(), "context");
        let forms: Vec<TypeForm> = spec.types.iter().map(|def| def.form).collect();
        assert_eq!(
            forms,
            [
                TypeForm::Enum,
                TypeForm::Enum,
                TypeForm::Discriminated,
                TypeForm::DataIo
            ],
            "the type forms"
        );
        let color = spec.types.first().expect("an enum");
        assert_eq!(
            color.enum_type.as_ref().map(|ty| (ty.base, ty.size)),
            Some((Base::Uint, Some(8))),
            "enum base type"
        );
        assert_eq!(color.values.len(), 2, "two values");
        assert_eq!(
            color.values.get(1).map(|value| value.value.is_none()),
            Some(true),
            "a value without a wire value"
        );
        let root = spec.types.get(2).expect("the discriminated type");
        assert_eq!(root.attributes.len(), 1, "one attribute");
        let kinds: Vec<&str> = fields_of(&root.fields)
            .iter()
            .map(|field| field.kind.keyword())
            .collect();
        assert_eq!(
            kinds,
            [
                "discriminator",
                "discriminator",
                "typeSwitch",
                "abstract",
                "assert",
                "checksum",
                "enum",
                "manualArray",
                "manual",
                "optional",
                "optional",
                "peek",
                "state",
                "unknown",
                "validation",
                "virtual",
                "simple",
                "simple"
            ],
            "every field kind outside the switch"
        );
        let Some(FieldKind::TypeSwitch {
            discriminators,
            cases,
        }) = fields_of(&root.fields).get(2).map(|field| &field.kind)
        else {
            panic!("a type switch");
        };
        assert_eq!(discriminators.len(), 3, "three discriminators");
        let read = cases.get(1).expect("the second case");
        assert!(read.name_wildcard, "a *Name case");
        assert_eq!(
            read.values.get(2),
            Some(&MExpr::Wildcard),
            "a wildcard value"
        );
        assert_eq!(read.params.len(), 1, "case parameter");
        assert!(
            matches!(read.fields.get(3), Some(Item::BatchSet { fields, .. }) if fields.len() == 2),
            "a batch set of two fields"
        );
        assert!(
            matches!(
                fields_of(&read.fields).get(1).map(|field| &field.kind),
                Some(FieldKind::Array {
                    loop_type: LoopType::Count,
                    ..
                })
            ),
            "a count array"
        );
        assert!(
            matches!(
                fields_of(&read.fields).get(2).map(|field| &field.kind),
                Some(FieldKind::Const {
                    expected: Literal::Hex(_),
                    ..
                })
            ),
            "a const with a hex value"
        );
        assert!(
            cases.get(2).is_some_and(|case| case.values.is_empty()),
            "a default case"
        );
        let item = spec.types.get(3).expect("the dataIo");
        assert_eq!(item.data_io_params.len(), 1, "dataIo parameters");
        let names = type_names(&[&spec]);
        assert!(
            names.contains("RootRead"),
            "the wildcard case name is prefixed"
        );
        assert_eq!(resolve(&spec, &names), Ok(()), "every reference resolves");
    }

    #[test]
    fn a_vstring_length_and_type_arguments_parse() {
        let spec = parse(
            "[type T(bit r) [simple vstring 'n' s] [simple Other('r', '1') o]] [type Other(bit r, uint 8 n)]",
        )
        .expect("parses");
        let fields = fields_of(&spec.types.first().expect("T").fields);
        assert!(
            matches!(fields.first().map(|field| &field.kind), Some(FieldKind::Simple { ty: TypeRef::Simple(ty), .. }) if ty.length.is_some()),
            "vstring length"
        );
        assert!(
            matches!(fields.get(1).map(|field| &field.kind), Some(FieldKind::Simple { ty: TypeRef::Complex { args, .. }, .. }) if args.len() == 2),
            "two type arguments"
        );
    }

    #[test]
    fn an_unresolved_type_is_an_error_naming_it_and_its_line() {
        let spec = parse("[type A\n    [simple Missing m]\n]").expect("parses");
        let error = resolve(&spec, &type_names(&[&spec])).expect_err("unresolved");
        assert_eq!(error.line, 2, "the field's line");
        assert!(error.message.contains("Missing"), "{error}");
    }

    #[test]
    fn malformed_input_is_an_error_with_a_line() {
        for (text, line) in [
            ("[type A [simple uint x y]]", 1),
            ("[type A\n[frobnicate uint 8 x]]", 2),
            ("[type A [simple uint 8 x 'a +']]", 1),
            ("[type A /* open", 1),
            ("[constants [simple uint 8 x]]", 1),
            ("[type A [simple uint 8 x] ", 1),
            ("[mystery A]", 1),
            ("[type A [simple uint 8 \"x]]", 1),
            ("[type A [array byte x sometimes '1']]", 1),
            ("#", 1),
        ] {
            let error = parse(text).expect_err(text);
            assert_eq!(error.line, line, "{text}: {error}");
        }
    }
}
