//! The expression language of `MSpec`, the text between ticks in a field definition.
//!
//! A hand-written port of upstream's `Expression.g4` (Apache PLC4X v1.0.0,
//! `code-generation/protocol-base-mspec/src/main/antlr4/.../mspec/expression/Expression.g4`,
//! lines 34-127); nothing of the grammar file is copied. The precedence is upstream's, not
//! Rust's: from loosest to tightest the ternary, `|`, `||`, `&`, `&&`, `==` and `!=`, the
//! comparisons, the shifts, `+` and `-`, `*`, `/` and `%`, then `^` (right-associative
//! power), with the prefix `-` and `!` tightest of all. `&&` binds tighter than `&`, and
//! `||` tighter than `|` (Expression.g4 lines 38-59).

use std::fmt;

/// A parsed expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Expr {
    /// A decimal number as written: an integer, or digits with a fraction.
    Number(String),
    /// A hexadecimal number as written, `0x` prefix included.
    Hex(String),
    /// `true` or `false`.
    Bool(bool),
    /// `null`.
    Null,
    /// A string literal without its quotes, escapes as written, and any indexes after it.
    Str(String, Vec<Self>),
    /// A variable, a property path or a function call.
    Var(Variable),
    /// A prefix operator.
    Unary(UnaryOp, Box<Self>),
    /// A binary operator.
    Binary(BinaryOp, Box<Self>, Box<Self>),
    /// `condition ? then : otherwise`.
    Ternary(Box<Self>, Box<Self>, Box<Self>),
    /// A parenthesized expression and any indexes after it.
    Group(Box<Self>, Vec<Self>),
}

/// One segment of an identifier path: `name(args)[index].rest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Variable {
    /// The identifier.
    pub(crate) name: String,
    /// The call arguments, when the segment is a call (`COUNT(value)`).
    pub(crate) args: Option<Vec<Expr>>,
    /// Index expressions after the name and arguments.
    pub(crate) indexes: Vec<Expr>,
    /// The next segment after a `.`.
    pub(crate) rest: Option<Box<Self>>,
}

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    /// `-`
    Neg,
    /// `!`
    Not,
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    /// `^`, the power operator of Expression.g4 line 41.
    Pow,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `>>`
    Shr,
    /// `<<`
    Shl,
    /// `>=`
    Ge,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `<`
    Lt,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `&&`
    And,
    /// `&`
    BitAnd,
    /// `||`
    Or,
    /// `|`
    BitOr,
}

impl BinaryOp {
    /// Binding strength: a higher number binds tighter (Expression.g4 lines 41-50).
    const fn precedence(self) -> u8 {
        match self {
            Self::Pow => 11,
            Self::Mul | Self::Div | Self::Rem => 10,
            Self::Add | Self::Sub => 9,
            Self::Shr | Self::Shl => 8,
            Self::Ge | Self::Le | Self::Gt | Self::Lt => 7,
            Self::Eq | Self::Ne => 6,
            Self::And => 5,
            Self::BitAnd => 4,
            Self::Or => 3,
            Self::BitOr => 2,
        }
    }

    /// The operator as written.
    pub(crate) const fn symbol(self) -> &'static str {
        match self {
            Self::Pow => "^",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::Add => "+",
            Self::Sub => "-",
            Self::Shr => ">>",
            Self::Shl => "<<",
            Self::Ge => ">=",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Lt => "<",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::And => "&&",
            Self::BitAnd => "&",
            Self::Or => "||",
            Self::BitOr => "|",
        }
    }
}

/// Precedence of the ternary operator, the loosest (Expression.g4 line 51).
const TERNARY: u8 = 1;

/// A syntax error in an expression: the offset in the expression text and what was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExprError {
    /// Character offset in the expression text.
    pub(crate) offset: usize,
    /// What the parser expected or found.
    pub(crate) message: String,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at offset {}: {}", self.offset, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Number(String),
    Hex(String),
    Bool(bool),
    Null,
    Ident(String),
    Str(String),
    Punct(&'static str),
}

/// Punctuation, longest first so `>>` is not read as two `>`.
const PUNCTUATION: [&str; 26] = [
    ">>", "<<", ">=", "<=", "==", "!=", "&&", "||", "-", "!", "^", "*", "/", "%", "+", ">", "<",
    "&", "|", "?", ":", "(", ")", "[", "]", ",",
];

fn lex(text: &str) -> Result<Vec<(usize, Token)>, ExprError> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while let Some(&c) = chars.get(at) {
        let start = at;
        if c.is_whitespace() {
            at = start.saturating_add(1);
            continue;
        }
        if c == '"' || c == '\'' {
            let (body, end) = lex_string(&chars, start, c)?;
            tokens.push((start, Token::Str(body)));
            at = end;
            continue;
        }
        if c == '.' {
            tokens.push((start, Token::Punct(".")));
            at = start.saturating_add(1);
            continue;
        }
        if c.is_ascii_digit() {
            let (token, end) = lex_number(&chars, start);
            tokens.push((start, token));
            at = end;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let end = scan(&chars, start, |c| c.is_ascii_alphanumeric() || c == '_');
            let word: String = chars.get(start..end).unwrap_or_default().iter().collect();
            tokens.push((
                start,
                match word.as_str() {
                    "true" => Token::Bool(true),
                    "false" => Token::Bool(false),
                    "null" => Token::Null,
                    _ => Token::Ident(word),
                },
            ));
            at = end;
            continue;
        }
        let rest: String = chars.get(start..).unwrap_or_default().iter().collect();
        let Some(symbol) = PUNCTUATION.iter().find(|symbol| rest.starts_with(**symbol)) else {
            return Err(ExprError {
                offset: start,
                message: format!("unexpected character {c:?}"),
            });
        };
        tokens.push((start, Token::Punct(symbol)));
        at = start.saturating_add(symbol.len());
    }
    Ok(tokens)
}

/// The end of the run of characters from `start` that satisfy `accept`.
fn scan(chars: &[char], start: usize, accept: impl Fn(char) -> bool) -> usize {
    let mut end = start;
    while chars.get(end).copied().is_some_and(&accept) {
        end = end.saturating_add(1);
    }
    end
}

/// A number: `0x` and hex digits, or `Int ('.' Digit*)?` (Expression.g4 lines 92-94, 109-127).
fn lex_number(chars: &[char], start: usize) -> (Token, usize) {
    let second = chars.get(start.saturating_add(1)).copied();
    let hex_start = start.saturating_add(2);
    if chars.get(start) == Some(&'0')
        && matches!(second, Some('x' | 'X'))
        && chars.get(hex_start).is_some_and(char::is_ascii_hexdigit)
    {
        let end = scan(chars, hex_start, |c| c.is_ascii_hexdigit());
        let text: String = chars.get(start..end).unwrap_or_default().iter().collect();
        return (Token::Hex(text), end);
    }
    // `Int` is `0` alone or a digit run not starting with 0.
    let mut end = if chars.get(start) == Some(&'0') {
        start.saturating_add(1)
    } else {
        scan(chars, start, |c| c.is_ascii_digit())
    };
    if chars.get(end) == Some(&'.')
        && !chars
            .get(end.saturating_add(1))
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_')
    {
        end = scan(chars, end.saturating_add(1), |c| c.is_ascii_digit());
    }
    let text: String = chars.get(start..end).unwrap_or_default().iter().collect();
    (Token::Number(text), end)
}

/// A string in `quote`s with backslash escapes; returns the body as written and the end.
fn lex_string(chars: &[char], start: usize, quote: char) -> Result<(String, usize), ExprError> {
    let mut at = start.saturating_add(1);
    let mut body = String::new();
    loop {
        match chars.get(at).copied() {
            None | Some('\r' | '\n') => {
                return Err(ExprError {
                    offset: start,
                    message: "unterminated string".to_owned(),
                });
            }
            Some('\\') => {
                let Some(escaped) = chars.get(at.saturating_add(1)).copied() else {
                    return Err(ExprError {
                        offset: at,
                        message: "unterminated escape".to_owned(),
                    });
                };
                body.push('\\');
                body.push(escaped);
                at = at.saturating_add(2);
            }
            Some(c) if c == quote => return Ok((body, at.saturating_add(1))),
            Some(c) => {
                body.push(c);
                at = at.saturating_add(1);
            }
        }
    }
}

/// Parses the text of one expression (without its ticks).
///
/// # Errors
/// An [`ExprError`] at the first character or token that does not fit the grammar,
/// including text left over after a complete expression.
pub(crate) fn parse(text: &str) -> Result<Expr, ExprError> {
    let tokens = lex(text)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        len: text.chars().count(),
    };
    let expr = parser.expression(0)?;
    if let Some((offset, token)) = parser.tokens.get(parser.at) {
        return Err(ExprError {
            offset: *offset,
            message: format!("unexpected {token:?} after the expression"),
        });
    }
    Ok(expr)
}

struct Parser {
    tokens: Vec<(usize, Token)>,
    at: usize,
    len: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at).map(|(_, token)| token)
    }

    fn offset(&self) -> usize {
        self.tokens
            .get(self.at)
            .map_or(self.len, |(offset, _)| *offset)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).map(|(_, token)| token.clone());
        self.at = self.at.saturating_add(1);
        token
    }

    fn eat(&mut self, symbol: &str) -> bool {
        if matches!(self.peek(), Some(Token::Punct(found)) if *found == symbol) {
            self.at = self.at.saturating_add(1);
            true
        } else {
            false
        }
    }

    fn expect(&mut self, symbol: &str) -> Result<(), ExprError> {
        if self.eat(symbol) {
            Ok(())
        } else {
            Err(self.error(&format!("expected `{symbol}`")))
        }
    }

    fn error(&self, message: &str) -> ExprError {
        ExprError {
            offset: self.offset(),
            message: format!("{message}, found {:?}", self.peek()),
        }
    }

    fn binary_op(&self) -> Option<BinaryOp> {
        let Some(Token::Punct(symbol)) = self.peek() else {
            return None;
        };
        Some(match *symbol {
            "^" => BinaryOp::Pow,
            "*" => BinaryOp::Mul,
            "/" => BinaryOp::Div,
            "%" => BinaryOp::Rem,
            "+" => BinaryOp::Add,
            "-" => BinaryOp::Sub,
            ">>" => BinaryOp::Shr,
            "<<" => BinaryOp::Shl,
            ">=" => BinaryOp::Ge,
            "<=" => BinaryOp::Le,
            ">" => BinaryOp::Gt,
            "<" => BinaryOp::Lt,
            "==" => BinaryOp::Eq,
            "!=" => BinaryOp::Ne,
            "&&" => BinaryOp::And,
            "&" => BinaryOp::BitAnd,
            "||" => BinaryOp::Or,
            "|" => BinaryOp::BitOr,
            _ => return None,
        })
    }

    /// Precedence climbing: parses an expression whose operators bind at least as tightly
    /// as `min`.
    fn expression(&mut self, min: u8) -> Result<Expr, ExprError> {
        let mut lhs = self.unary()?;
        loop {
            if min <= TERNARY && self.eat("?") {
                let then = self.expression(0)?;
                self.expect(":")?;
                // ANTLR's default left associativity: `a ? b : c ? d : e` groups the first
                // ternary as the condition of the second.
                let otherwise = self.expression(TERNARY.saturating_add(1))?;
                lhs = Expr::Ternary(Box::new(lhs), Box::new(then), Box::new(otherwise));
                continue;
            }
            let Some(op) = self.binary_op() else {
                return Ok(lhs);
            };
            let precedence = op.precedence();
            if precedence < min {
                return Ok(lhs);
            }
            self.at = self.at.saturating_add(1);
            let next = if op == BinaryOp::Pow {
                precedence
            } else {
                precedence.saturating_add(1)
            };
            let rhs = self.expression(next)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn unary(&mut self) -> Result<Expr, ExprError> {
        if self.eat("-") {
            return Ok(Expr::Unary(UnaryOp::Neg, Box::new(self.unary()?)));
        }
        if self.eat("!") {
            return Ok(Expr::Unary(UnaryOp::Not, Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, ExprError> {
        let offset = self.offset();
        match self.bump() {
            Some(Token::Number(text)) => Ok(Expr::Number(text)),
            Some(Token::Hex(text)) => Ok(Expr::Hex(text)),
            Some(Token::Bool(value)) => Ok(Expr::Bool(value)),
            Some(Token::Null) => Ok(Expr::Null),
            Some(Token::Str(text)) => Ok(Expr::Str(text, self.indexes()?)),
            Some(Token::Ident(name)) => Ok(Expr::Var(self.segment(name)?)),
            Some(Token::Punct("(")) => {
                let inner = self.expression(0)?;
                self.expect(")")?;
                Ok(Expr::Group(Box::new(inner), self.indexes()?))
            }
            other => Err(ExprError {
                offset,
                message: format!("expected an operand, found {other:?}"),
            }),
        }
    }

    /// `( '[' expression ']' )*`
    fn indexes(&mut self) -> Result<Vec<Expr>, ExprError> {
        let mut indexes = Vec::new();
        while self.eat("[") {
            indexes.push(self.expression(0)?);
            self.expect("]")?;
        }
        Ok(indexes)
    }

    /// The rest of an identifier segment after its name (Expression.g4 lines 61-75).
    fn segment(&mut self, name: String) -> Result<Variable, ExprError> {
        let args = if self.eat("(") {
            let mut args = Vec::new();
            if !self.eat(")") {
                loop {
                    args.push(self.expression(0)?);
                    if self.eat(")") {
                        break;
                    }
                    self.expect(",")?;
                }
            }
            Some(args)
        } else {
            None
        };
        let indexes = self.indexes()?;
        let rest = if self.eat(".") {
            let offset = self.offset();
            match self.bump() {
                Some(Token::Ident(next)) => Some(Box::new(self.segment(next)?)),
                other => {
                    return Err(ExprError {
                        offset,
                        message: format!("expected an identifier after `.`, found {other:?}"),
                    });
                }
            }
        } else {
            None
        };
        Ok(Variable {
            name,
            args,
            indexes,
            rest,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{BinaryOp, Expr, UnaryOp, Variable, parse};

    fn var(name: &str) -> Expr {
        Expr::Var(Variable {
            name: name.to_owned(),
            args: None,
            indexes: Vec::new(),
            rest: None,
        })
    }

    fn bin(op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
        Expr::Binary(op, Box::new(lhs), Box::new(rhs))
    }

    #[test]
    fn logical_and_binds_tighter_than_bitwise_and_as_upstream_has_it() {
        assert_eq!(
            parse("a & b && c").expect("parses"),
            bin(
                BinaryOp::BitAnd,
                var("a"),
                bin(BinaryOp::And, var("b"), var("c"))
            ),
            "Expression.g4 lists && (line 47) before & (line 48)"
        );
        assert_eq!(
            parse("a | b || c").expect("parses"),
            bin(
                BinaryOp::BitOr,
                var("a"),
                bin(BinaryOp::Or, var("b"), var("c"))
            ),
            "Expression.g4 lists || (line 49) before | (line 50)"
        );
    }

    #[test]
    fn arithmetic_is_left_associative_and_power_is_right_associative() {
        assert_eq!(
            parse("a - b - c").expect("parses"),
            bin(
                BinaryOp::Sub,
                bin(BinaryOp::Sub, var("a"), var("b")),
                var("c")
            ),
            "subtraction groups to the left"
        );
        assert_eq!(
            parse("a ^ b ^ c").expect("parses"),
            bin(
                BinaryOp::Pow,
                var("a"),
                bin(BinaryOp::Pow, var("b"), var("c"))
            ),
            "power groups to the right"
        );
        assert_eq!(
            parse("(COUNT(fifoValue) * 2) + 2").expect("parses"),
            bin(
                BinaryOp::Add,
                Expr::Group(
                    Box::new(bin(
                        BinaryOp::Mul,
                        Expr::Var(Variable {
                            name: "COUNT".to_owned(),
                            args: Some(vec![var("fifoValue")]),
                            indexes: Vec::new(),
                            rest: None,
                        }),
                        Expr::Number("2".to_owned()),
                    )),
                    Vec::new(),
                ),
                Expr::Number("2".to_owned()),
            ),
            "modbus.mspec line 192"
        );
    }

    #[test]
    fn prefix_operators_bind_tightest_and_ternaries_group_left() {
        assert_eq!(
            parse("-a * b").expect("parses"),
            bin(
                BinaryOp::Mul,
                Expr::Unary(UnaryOp::Neg, Box::new(var("a"))),
                var("b")
            ),
            "unary minus is the first alternative"
        );
        let Expr::Ternary(condition, _, _) = parse("a ? b : c ? d : e").expect("parses") else {
            panic!("a ternary");
        };
        assert!(
            matches!(*condition, Expr::Ternary(..)),
            "the first ternary is the condition of the second"
        );
        assert_eq!(
            parse("a == b && c < d").expect("parses"),
            bin(
                BinaryOp::And,
                bin(BinaryOp::Eq, var("a"), var("b")),
                bin(BinaryOp::Lt, var("c"), var("d"))
            ),
            "comparisons bind tighter than &&"
        );
    }

    #[test]
    fn paths_calls_literals_and_strings_parse() {
        let Ok(Expr::Binary(BinaryOp::Add, lhs, _)) = parse("pdu.lengthInBytes + 1") else {
            panic!("an addition");
        };
        let Expr::Var(path) = *lhs else {
            panic!("a variable");
        };
        assert_eq!(path.name, "pdu", "first segment");
        assert_eq!(
            path.rest.map(|rest| rest.name),
            Some("lengthInBytes".to_owned()),
            "second segment"
        );
        let Ok(Expr::Var(call)) = parse("STATIC_CALL(\"rtuCrcCheck\", address, pdu)") else {
            panic!("a call");
        };
        assert_eq!(call.args.map(|args| args.len()), Some(3), "three arguments");
        assert_eq!(
            parse("0x0F").expect("parses"),
            Expr::Hex("0x0F".to_owned()),
            "hex as written"
        );
        assert_eq!(
            parse("\"BIG_ENDIAN\"").expect("parses"),
            Expr::Str("BIG_ENDIAN".to_owned(), Vec::new()),
            "string"
        );
        assert_eq!(
            parse("1.5").expect("parses"),
            Expr::Number("1.5".to_owned()),
            "decimal"
        );
        assert_eq!(
            parse("a[1].b").map(|expr| matches!(expr, Expr::Var(_))),
            Ok(true),
            "indexed path"
        );
    }

    #[test]
    fn malformed_expressions_are_errors() {
        for text in ["", "a +", "(a", "a b", "COUNT(a,", "\"open", "a.1", "#"] {
            assert!(parse(text).is_err(), "{text:?} must not parse");
        }
    }
}
