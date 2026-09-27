//! Tokenizer for a CUDA-Q kernel's source region.
//!
//! Only the kernel itself is ever tokenized — [`super::parser::extract`]
//! cuts it out of the file first — so host-side Python (argument parsing,
//! `cudaq.sample`, f-strings, anything) never reaches this lexer and cannot
//! trip it.
//!
//! Python's line structure is the one thing this lexer has to get right that
//! the OpenQASM lexer does not: a newline ends a statement *unless* it sits
//! inside brackets, and a trailing backslash joins two lines. Indentation is
//! not tokenized; the parser checks it from each statement's first column.

use crate::frontend::error::FrontendError;

/// A lexical token together with its 1-based source position.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// What was scanned.
    pub kind: TokenKind,
    /// 1-based line in the original file.
    pub line: u32,
    /// 1-based column in the original file.
    pub column: u32,
}

/// The token classes a supported kernel can contain.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// An identifier or keyword; keywords are resolved by the parser.
    Ident(String),
    /// A numeric literal.
    Number(f64),
    /// A string literal (only ever meaningful as a docstring).
    Str,
    /// The end of a logical line.
    Newline,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `,`
    Comma,
    /// `.`
    Dot,
    /// `:`
    Colon,
    /// `=`
    Equals,
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
    /// Any other Python operator (`**`, `==`, `+=`, `%`, …). Scanned rather
    /// than rejected so the parser can name the construct it belongs to.
    Operator(String),
    /// End of the kernel region.
    Eof,
}

impl TokenKind {
    /// A short human-readable description, used in parser diagnostics.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Ident(name) => format!("`{name}`"),
            TokenKind::Number(value) => format!("number `{value}`"),
            TokenKind::Str => "a string literal".into(),
            TokenKind::Newline => "end of line".into(),
            TokenKind::LParen => "`(`".into(),
            TokenKind::RParen => "`)`".into(),
            TokenKind::LBracket => "`[`".into(),
            TokenKind::RBracket => "`]`".into(),
            TokenKind::Comma => "`,`".into(),
            TokenKind::Dot => "`.`".into(),
            TokenKind::Colon => "`:`".into(),
            TokenKind::Equals => "`=`".into(),
            TokenKind::Arrow => "`->`".into(),
            TokenKind::Plus => "`+`".into(),
            TokenKind::Minus => "`-`".into(),
            TokenKind::Star => "`*`".into(),
            TokenKind::Slash => "`/`".into(),
            TokenKind::Operator(op) => format!("operator `{op}`"),
            TokenKind::Eof => "end of kernel".into(),
        }
    }
}

/// Multi-character operators, longest first so `**=` beats `**`.
const OPERATORS: [&str; 22] = [
    "**=", "//=", "**", "//", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "%=", "<<", ">>",
    "%", "<", ">", "&", "|", "^", "~",
];

/// Tokenizes a kernel region whose first line is line `first_line` of the
/// original file. Always ends with [`TokenKind::Eof`].
///
/// # Errors
///
/// [`FrontendError::Syntax`] for an unterminated string, a malformed number,
/// or a character no supported construct uses.
pub fn tokenize(region: &str, first_line: u32) -> Result<Vec<Token>, FrontendError> {
    let chars: Vec<char> = region.chars().collect();
    let mut tokens: Vec<Token> = Vec::new();
    let mut i = 0usize;
    let mut line = first_line;
    let mut column = 1u32;
    let mut depth = 0usize;

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

    macro_rules! push {
        ($kind:expr, $line:expr, $column:expr) => {
            tokens.push(Token {
                kind: $kind,
                line: $line,
                column: $column,
            })
        };
    }

    while i < chars.len() {
        let c = chars[i];
        let (start_line, start_column) = (line, column);

        if c == '\n' {
            if depth == 0 && tokens.last().is_some_and(|t| t.kind != TokenKind::Newline) {
                push!(TokenKind::Newline, start_line, start_column);
            }
            advance!();
            continue;
        }
        if c.is_whitespace() {
            advance!();
            continue;
        }
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                advance!();
            }
            continue;
        }
        // Explicit line joining: a backslash immediately before the newline.
        if c == '\\' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '\n' && chars[j].is_whitespace() {
                j += 1;
            }
            if j >= chars.len() || chars[j] == '\n' {
                while i <= j && i < chars.len() {
                    advance!();
                }
                continue;
            }
            return Err(FrontendError::syntax(
                "unexpected `\\` outside a line continuation",
                start_line,
                start_column,
            ));
        }

        if c.is_alphabetic() || c == '_' {
            let mut name = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                name.push(chars[i]);
                advance!();
            }
            push!(TokenKind::Ident(name), start_line, start_column);
            continue;
        }

        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let mut text = String::new();
            while i < chars.len()
                && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_')
            {
                if chars[i] != '_' {
                    text.push(chars[i]);
                }
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
            push!(TokenKind::Number(value), start_line, start_column);
            continue;
        }

        if c == '"' || c == '\'' {
            let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
            let quotes = if triple { 3 } else { 1 };
            for _ in 0..quotes {
                advance!();
            }
            loop {
                if i >= chars.len() || (!triple && chars[i] == '\n') {
                    return Err(FrontendError::syntax(
                        "unterminated string literal",
                        start_line,
                        start_column,
                    ));
                }
                if chars[i] == '\\' && i + 1 < chars.len() {
                    advance!();
                    advance!();
                    continue;
                }
                if chars[i] == c
                    && (!triple || (chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c)))
                {
                    for _ in 0..quotes {
                        advance!();
                    }
                    break;
                }
                advance!();
            }
            push!(TokenKind::Str, start_line, start_column);
            continue;
        }

        if let Some(op) = OPERATORS.iter().find(|op| {
            op.chars()
                .enumerate()
                .all(|(k, ch)| chars.get(i + k) == Some(&ch))
        }) {
            for _ in 0..op.len() {
                advance!();
            }
            push!(
                TokenKind::Operator((*op).to_string()),
                start_line,
                start_column
            );
            continue;
        }

        let kind = match c {
            '(' => {
                depth += 1;
                TokenKind::LParen
            }
            '[' => {
                depth += 1;
                TokenKind::LBracket
            }
            ')' => {
                depth = depth.saturating_sub(1);
                TokenKind::RParen
            }
            ']' => {
                depth = depth.saturating_sub(1);
                TokenKind::RBracket
            }
            ',' => TokenKind::Comma,
            '.' => TokenKind::Dot,
            ':' => TokenKind::Colon,
            ';' => TokenKind::Newline,
            '=' => TokenKind::Equals,
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
            '{' | '}' => {
                return Err(FrontendError::syntax(
                    "dicts and sets are not part of the supported kernel subset",
                    start_line,
                    start_column,
                ));
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
        push!(kind, start_line, start_column);
    }

    if tokens.last().is_some_and(|t| t.kind != TokenKind::Newline) {
        push!(TokenKind::Newline, line, column);
    }
    push!(TokenKind::Eof, line, column);
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source, 1)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    fn ident(name: &str) -> TokenKind {
        TokenKind::Ident(name.into())
    }

    #[test]
    fn a_gate_call_scans_with_a_trailing_newline() {
        assert_eq!(
            kinds("h(q[0])"),
            vec![
                ident("h"),
                TokenKind::LParen,
                ident("q"),
                TokenKind::LBracket,
                TokenKind::Number(0.0),
                TokenKind::RBracket,
                TokenKind::RParen,
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn newlines_inside_brackets_do_not_end_the_statement() {
        let k = kinds("x.ctrl(q[0],\n       q[1])\nh(q[0])");
        let newlines = k.iter().filter(|t| **t == TokenKind::Newline).count();
        assert_eq!(newlines, 2, "{k:?}");
    }

    #[test]
    fn backslash_joins_lines() {
        let k = kinds("rz(0.5, \\\n q[0])");
        assert_eq!(k.iter().filter(|t| **t == TokenKind::Newline).count(), 1);
    }

    #[test]
    fn comments_are_skipped_and_blank_lines_collapse() {
        assert_eq!(
            kinds("# leading\n\n h(q)  # trailing\n\n"),
            vec![
                ident("h"),
                TokenKind::LParen,
                ident("q"),
                TokenKind::RParen,
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn a_semicolon_separates_statements() {
        let k = kinds("h(q); x(q)");
        assert_eq!(k.iter().filter(|t| **t == TokenKind::Newline).count(), 2);
    }

    #[test]
    fn docstrings_scan_as_one_token_even_across_lines() {
        assert_eq!(
            kinds("\"\"\"Bell\nstate\"\"\"\nh(q)")[..2],
            [TokenKind::Str, TokenKind::Newline]
        );
    }

    #[test]
    fn arrows_and_multi_character_operators_are_distinct() {
        assert_eq!(
            kinds("-> ** == -"),
            vec![
                TokenKind::Arrow,
                TokenKind::Operator("**".into()),
                TokenKind::Operator("==".into()),
                TokenKind::Minus,
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn positions_are_reported_in_the_original_file() {
        let tokens = tokenize("def k():\n    h(q)", 7).unwrap();
        let h = tokens.iter().find(|t| t.kind == ident("h")).unwrap();
        assert_eq!((h.line, h.column), (8, 5));
    }

    #[test]
    fn unterminated_string_is_rejected() {
        assert!(matches!(
            tokenize("'oops\nh(q)", 1),
            Err(FrontendError::Syntax { .. })
        ));
    }

    #[test]
    fn unexpected_character_is_rejected() {
        assert!(matches!(
            tokenize("h(q) $", 1),
            Err(FrontendError::Syntax { .. })
        ));
    }
}
