//! Hand-written tokenizer for the supported OpenQASM 3 subset.
//!
//! The subset (see `docs/openqasm_frontend.md`) needs identifiers, numbers,
//! strings, and a small punctuation set — small enough that a parser-generator
//! dependency would cost more than it saves. Every token carries a 1-based
//! line/column so diagnostics can point at the offending source position, as
//! required by `final-deliverables-spec.md` §28.

use crate::frontend::error::FrontendError;

/// A lexical token together with its source position.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// What was scanned.
    pub kind: TokenKind,
    /// 1-based line of the token's first character.
    pub line: u32,
    /// 1-based column of the token's first character.
    pub column: u32,
}

/// The token classes of the supported subset.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// An identifier or keyword (keywords are resolved by the parser).
    Ident(String),
    /// A numeric literal. Integers and floats share one class; the parser
    /// decides which it needs.
    Number(f64),
    /// A double-quoted string literal, without the quotes.
    Str(String),
    /// `;`
    Semicolon,
    /// `,`
    Comma,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `=`
    Equals,
    /// `:`
    ///
    /// Not used by any supported construct. It is scanned so that a range
    /// like `[0:2]` inside an unsupported `for` reaches the parser, which can
    /// then name the *construct* rather than blaming a stray character.
    Colon,
    /// `->`
    Arrow,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// End of input.
    Eof,
}

impl TokenKind {
    /// A short human-readable description, used in parser diagnostics.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Ident(name) => format!("identifier `{name}`"),
            TokenKind::Number(value) => format!("number `{value}`"),
            TokenKind::Str(value) => format!("string \"{value}\""),
            TokenKind::Semicolon => "`;`".into(),
            TokenKind::Comma => "`,`".into(),
            TokenKind::LParen => "`(`".into(),
            TokenKind::RParen => "`)`".into(),
            TokenKind::LBracket => "`[`".into(),
            TokenKind::RBracket => "`]`".into(),
            TokenKind::LBrace => "`{`".into(),
            TokenKind::RBrace => "`}`".into(),
            TokenKind::Equals => "`=`".into(),
            TokenKind::Colon => "`:`".into(),
            TokenKind::Arrow => "`->`".into(),
            TokenKind::Plus => "`+`".into(),
            TokenKind::Minus => "`-`".into(),
            TokenKind::Star => "`*`".into(),
            TokenKind::Slash => "`/`".into(),
            TokenKind::Eof => "end of input".into(),
        }
    }
}

/// Tokenizes an OpenQASM 3 source string, always ending with
/// [`TokenKind::Eof`].
///
/// Line (`//`) and block (`/* … */`) comments are skipped.
///
/// # Errors
///
/// Returns [`FrontendError::Syntax`] for an unterminated string or block
/// comment, a malformed number, or a character outside the subset's alphabet.
pub fn tokenize(source: &str) -> Result<Vec<Token>, FrontendError> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut column = 1u32;

    macro_rules! advance {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
            i += 1;
        }};
    }

    while i < chars.len() {
        let c = chars[i];

        if c.is_whitespace() {
            advance!();
            continue;
        }

        // Comments.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                advance!();
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let (start_line, start_column) = (line, column);
            advance!();
            advance!();
            loop {
                if i >= chars.len() {
                    return Err(FrontendError::syntax(
                        "unterminated block comment",
                        start_line,
                        start_column,
                    ));
                }
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    advance!();
                    advance!();
                    break;
                }
                advance!();
            }
            continue;
        }

        let (start_line, start_column) = (line, column);

        // Identifiers and keywords.
        if c.is_alphabetic() || c == '_' {
            let mut name = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                name.push(chars[i]);
                advance!();
            }
            tokens.push(Token {
                kind: TokenKind::Ident(name),
                line: start_line,
                column: start_column,
            });
            continue;
        }

        // Numbers: digits, an optional fractional part, an optional exponent.
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let mut text = String::new();
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                text.push(chars[i]);
                advance!();
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                text.push(chars[i]);
                advance!();
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    text.push(chars[i]);
                    advance!();
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    text.push(chars[i]);
                    advance!();
                }
            }
            let value = text.parse::<f64>().map_err(|_| {
                FrontendError::syntax(
                    format!("malformed numeric literal `{text}`"),
                    start_line,
                    start_column,
                )
            })?;
            tokens.push(Token {
                kind: TokenKind::Number(value),
                line: start_line,
                column: start_column,
            });
            continue;
        }

        // String literals.
        if c == '"' {
            advance!();
            let mut value = String::new();
            loop {
                if i >= chars.len() || chars[i] == '\n' {
                    return Err(FrontendError::syntax(
                        "unterminated string literal",
                        start_line,
                        start_column,
                    ));
                }
                if chars[i] == '"' {
                    advance!();
                    break;
                }
                value.push(chars[i]);
                advance!();
            }
            tokens.push(Token {
                kind: TokenKind::Str(value),
                line: start_line,
                column: start_column,
            });
            continue;
        }

        // Punctuation.
        let kind = match c {
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '=' => TokenKind::Equals,
            ':' => TokenKind::Colon,
            '+' => TokenKind::Plus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '-' => {
                if chars.get(i + 1) == Some(&'>') {
                    advance!();
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            other => {
                return Err(FrontendError::syntax(
                    format!("unexpected character `{other}`"),
                    start_line,
                    start_column,
                ));
            }
        };
        advance!();
        tokens.push(Token {
            kind,
            line: start_line,
            column: start_column,
        });
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        line,
        column,
    });
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn scans_identifiers_and_punctuation() {
        assert_eq!(
            kinds("qubit[2] q;"),
            vec![
                TokenKind::Ident("qubit".into()),
                TokenKind::LBracket,
                TokenKind::Number(2.0),
                TokenKind::RBracket,
                TokenKind::Ident("q".into()),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn scans_arrow_and_minus_distinctly() {
        assert_eq!(
            kinds("-> -"),
            vec![TokenKind::Arrow, TokenKind::Minus, TokenKind::Eof]
        );
    }

    #[test]
    fn scans_floats_and_exponents() {
        assert_eq!(
            kinds("3.0 1e3 2.5e-2 .5"),
            vec![
                TokenKind::Number(3.0),
                TokenKind::Number(1e3),
                TokenKind::Number(2.5e-2),
                TokenKind::Number(0.5),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn scans_strings() {
        assert_eq!(
            kinds("include \"stdgates.inc\";"),
            vec![
                TokenKind::Ident("include".into()),
                TokenKind::Str("stdgates.inc".into()),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn skips_line_and_block_comments() {
        assert_eq!(
            kinds("// leading\nh /* inline */ q;"),
            vec![
                TokenKind::Ident("h".into()),
                TokenKind::Ident("q".into()),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn tracks_line_and_column() {
        let tokens = tokenize("h q;\n  cx a, b;").unwrap();
        let cx = &tokens[3];
        assert_eq!(cx.kind, TokenKind::Ident("cx".into()));
        assert_eq!((cx.line, cx.column), (2, 3));
    }

    #[test]
    fn unterminated_string_is_rejected() {
        assert!(matches!(
            tokenize("include \"oops;"),
            Err(FrontendError::Syntax { .. })
        ));
    }

    #[test]
    fn unterminated_block_comment_is_rejected() {
        assert!(matches!(
            tokenize("/* forever"),
            Err(FrontendError::Syntax { .. })
        ));
    }

    #[test]
    fn unexpected_character_is_rejected() {
        assert!(matches!(
            tokenize("h $ q;"),
            Err(FrontendError::Syntax { .. })
        ));
    }
}
