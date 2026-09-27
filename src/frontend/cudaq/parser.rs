//! Kernel extraction and recursive-descent parsing.
//!
//! A CUDA-Q source file is a Python module: the quantum program is the one
//! function decorated `@cudaq.kernel`, and everything else — imports,
//! `cudaq.sample(...)`, `if __name__ == "__main__":` — is host code that runs
//! on a classical machine and is not part of any circuit. [`extract`] cuts the
//! kernel out by line and indentation *before* anything is tokenized, so host
//! code can be arbitrary Python without ever reaching this parser.
//!
//! Inside the kernel, the grammar is deliberately flat: allocations, gate
//! calls and measurements, one per logical line, at one indentation level.
//! Every Python construct outside that — `for`, `if`, `return`, classical
//! assignment, keyword arguments, nested functions — is refused *by name*
//! with [`FrontendError::Unsupported`], never skipped.

use crate::frontend::cudaq::lexer::{Token, TokenKind, tokenize};
use crate::frontend::error::FrontendError;

/// A parsed kernel.
#[derive(Debug, Clone, PartialEq)]
pub struct Kernel {
    /// The Python function's name.
    pub name: String,
    /// Its parameters, with annotations exactly as written.
    pub args: Vec<KernelArg>,
    /// Body statements in program order.
    pub body: Vec<Statement>,
}

/// One kernel parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct KernelArg {
    /// Parameter name.
    pub name: String,
    /// The annotation's source text (`float`, `list[float]`, …), or `None`
    /// when unannotated.
    pub annotation: Option<String>,
    /// 1-based line.
    pub line: u32,
}

/// One body statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// `name = cudaq.qvector(size)` (`size: Some`) or `name = cudaq.qubit()`
    /// (`size: None`).
    Allocate {
        /// The variable bound.
        name: String,
        /// The register width expression, for a `qvector`.
        size: Option<Expr>,
        /// 1-based line.
        line: u32,
    },
    /// A quantum operation call, e.g. `x.ctrl(q[0], q[1])`.
    Call(Call),
}

/// A call statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The dotted callee: `["h"]`, `["x", "ctrl"]`, `["t", "adj"]`.
    pub callee: Vec<String>,
    /// Positional arguments.
    pub args: Vec<Arg>,
    /// 1-based line.
    pub line: u32,
}

/// A positional call argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// An ordinary expression.
    Expr(Expr),
    /// A list literal — the documented spelling of multiple controls,
    /// `x.ctrl([c0, c1], t)`.
    List(Vec<Expr>),
}

/// An expression: an angle, a register index, or a qubit reference.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A numeric literal.
    Number(f64),
    /// A name or dotted name: `q`, `theta`, `math.pi`.
    Path(Vec<String>),
    /// A subscript: `q[0]`.
    Index {
        /// The subscripted name.
        base: Vec<String>,
        /// The index expression.
        index: Box<Expr>,
    },
    /// Unary minus.
    Neg(Box<Expr>),
    /// A binary arithmetic operation.
    Binary {
        /// The operator.
        op: BinOp,
        /// Left operand.
        left: Box<Expr>,
        /// Right operand.
        right: Box<Expr>,
    },
}

/// Arithmetic operators allowed in angle and index expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
}

/// Python keywords that open a construct outside the subset, with the reason
/// given when one is found in a kernel body.
const REFUSED_KEYWORDS: [(&str, &str); 22] = [
    (
        "for",
        "loops (`for`) are outside the supported subset — unroll them; OQCI compiles static circuits only",
    ),
    (
        "while",
        "loops (`while`) are outside the supported subset; OQCI compiles static circuits only",
    ),
    (
        "if",
        "conditionals (`if`) are outside the supported subset; measurement-conditioned control flow is deferred (Stage F)",
    ),
    (
        "elif",
        "conditionals (`elif`) are outside the supported subset",
    ),
    (
        "else",
        "conditionals (`else`) are outside the supported subset",
    ),
    (
        "match",
        "`match` statements are outside the supported subset",
    ),
    (
        "return",
        "a kernel that returns a value is outside the supported subset",
    ),
    ("def", "nested functions are outside the supported subset"),
    (
        "class",
        "class definitions are outside the supported subset",
    ),
    ("lambda", "lambdas are outside the supported subset"),
    ("with", "`with` blocks are outside the supported subset"),
    ("try", "`try` blocks are outside the supported subset"),
    ("except", "`except` blocks are outside the supported subset"),
    (
        "finally",
        "`finally` blocks are outside the supported subset",
    ),
    (
        "import",
        "imports inside a kernel are outside the supported subset",
    ),
    (
        "from",
        "imports inside a kernel are outside the supported subset",
    ),
    ("yield", "generators are outside the supported subset"),
    ("assert", "`assert` is outside the supported subset"),
    ("raise", "`raise` is outside the supported subset"),
    ("del", "`del` is outside the supported subset"),
    ("global", "`global` is outside the supported subset"),
    ("nonlocal", "`nonlocal` is outside the supported subset"),
];

/// Extracts and parses the file's single `@cudaq.kernel` function.
///
/// # Errors
///
/// - [`FrontendError::Semantic`] if the file declares no kernel.
/// - [`FrontendError::Unsupported`] for more than one kernel, a decorator
///   with arguments, or any construct outside the subset.
/// - [`FrontendError::Syntax`] for malformed kernel source.
pub fn parse(source: &str) -> Result<Kernel, FrontendError> {
    let (region, first_line) = extract(source)?;
    let tokens = tokenize(&region, first_line)?;
    Parser { tokens, pos: 0 }.kernel()
}

/// Returns the kernel's `def` line and body as one string, together with the
/// 1-based line number the region starts at.
///
/// The body is everything after the `def` line that is indented deeper than
/// it, blank, a comment, or a continuation of a bracketed expression; the
/// first line that is none of those ends it.
///
/// # Errors
///
/// As [`parse`].
pub fn extract(source: &str) -> Result<(String, u32), FrontendError> {
    let lines: Vec<&str> = source.lines().collect();

    let decorators: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| strip_comment(line).trim().starts_with("@cudaq.kernel"))
        .map(|(i, _)| i)
        .collect();

    let decorator = match decorators.as_slice() {
        [] => {
            return Err(FrontendError::semantic(
                "no `@cudaq.kernel` function found in this file",
            ));
        }
        [only] => *only,
        many => {
            let at: Vec<String> = many.iter().map(|i| (i + 1).to_string()).collect();
            return Err(FrontendError::unsupported(format!(
                "{} `@cudaq.kernel` functions (lines {}); a file must declare exactly one \
                 kernel — calling one kernel from another is outside the supported subset",
                many.len(),
                at.join(", ")
            )));
        }
    };

    let decorator_text: String = strip_comment(lines[decorator])
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if decorator_text != "@cudaq.kernel" && decorator_text != "@cudaq.kernel()" {
        return Err(FrontendError::unsupported(format!(
            "decorator `{}` (line {}); only `@cudaq.kernel` and `@cudaq.kernel()` are supported",
            lines[decorator].trim(),
            decorator + 1
        )));
    }

    let def_index = (decorator + 1..lines.len())
        .find(|&i| !is_blank(lines[i]))
        .ok_or_else(|| {
            FrontendError::syntax(
                "expected a `def` after `@cudaq.kernel`",
                decorator as u32 + 1,
                1,
            )
        })?;
    let def_trimmed = lines[def_index].trim_start();
    if def_trimmed.starts_with('@') {
        return Err(FrontendError::unsupported(format!(
            "stacked decorator `{}` (line {}); a kernel must carry `@cudaq.kernel` alone",
            def_trimmed.trim(),
            def_index + 1
        )));
    }
    if !def_trimmed.starts_with("def ") {
        return Err(FrontendError::syntax(
            "expected a `def` after `@cudaq.kernel`",
            def_index as u32 + 1,
            1,
        ));
    }
    let def_indent = indent_of(lines[def_index]);

    let mut end = def_index + 1;
    let mut depth = bracket_delta(lines[def_index]).max(0);
    while end < lines.len() {
        let line = lines[end];
        let belongs = depth > 0 || is_blank(line) || indent_of(line) > def_indent;
        if !belongs {
            break;
        }
        depth = (depth + bracket_delta(line)).max(0);
        end += 1;
    }

    Ok((lines[def_index..end].join("\n"), def_index as u32 + 1))
}

fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("")
}

fn is_blank(line: &str) -> bool {
    strip_comment(line).trim().is_empty()
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// Net bracket depth change across one line, ignoring strings and comments.
fn bracket_delta(line: &str) -> i64 {
    let mut delta = 0i64;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '#' => break,
                '"' | '\'' => quote = Some(c),
                '(' | '[' | '{' => delta += 1,
                ')' | ']' | '}' => delta -= 1,
                _ => {}
            },
        }
    }
    delta
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn peek_at(&self, offset: usize) -> &TokenKind {
        &self.tokens[(self.pos + offset).min(self.tokens.len() - 1)].kind
    }

    fn next(&mut self) -> Token {
        let token = self.peek().clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        token
    }

    fn error(&self, message: impl Into<String>) -> FrontendError {
        let token = self.peek();
        FrontendError::syntax(message, token.line, token.column)
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> Result<Token, FrontendError> {
        if &self.peek().kind == kind {
            Ok(self.next())
        } else {
            Err(self.error(format!(
                "expected {what}, found {}",
                self.peek().kind.describe()
            )))
        }
    }

    fn ident(&mut self, what: &str) -> Result<String, FrontendError> {
        match self.peek().kind.clone() {
            TokenKind::Ident(name) => {
                self.next();
                Ok(name)
            }
            other => Err(self.error(format!("expected {what}, found {}", other.describe()))),
        }
    }

    fn kernel(mut self) -> Result<Kernel, FrontendError> {
        let def = self.next();
        if def.kind != TokenKind::Ident("def".into()) {
            return Err(FrontendError::syntax(
                "expected `def`",
                def.line,
                def.column,
            ));
        }
        let def_column = def.column;
        let name = self.ident("a kernel name")?;
        self.expect(&TokenKind::LParen, "`(`")?;
        let args = self.kernel_args()?;
        if self.peek().kind == TokenKind::Arrow {
            self.next();
            self.annotation(&[TokenKind::Colon])?;
        }
        self.expect(&TokenKind::Colon, "`:` ending the `def` line")?;
        self.expect(&TokenKind::Newline, "a new line after the `def` line")?;

        let mut body = Vec::new();
        let mut body_column: Option<u32> = None;
        while self.peek().kind != TokenKind::Eof {
            let start = self.peek().clone();
            match body_column {
                None if start.column <= def_column => {
                    return Err(FrontendError::syntax(
                        "expected an indented kernel body",
                        start.line,
                        start.column,
                    ));
                }
                None => body_column = Some(start.column),
                Some(col) if col != start.column => {
                    return Err(FrontendError::syntax(
                        "inconsistent indentation in the kernel body",
                        start.line,
                        start.column,
                    ));
                }
                Some(_) => {}
            }
            if let Some(statement) = self.statement()? {
                body.push(statement);
            }
        }

        Ok(Kernel { name, args, body })
    }

    fn kernel_args(&mut self) -> Result<Vec<KernelArg>, FrontendError> {
        let mut args = Vec::new();
        while self.peek().kind != TokenKind::RParen {
            let line = self.peek().line;
            let name = self.ident("a parameter name")?;
            let annotation = if self.peek().kind == TokenKind::Colon {
                self.next();
                Some(self.annotation(&[TokenKind::Comma, TokenKind::RParen])?)
            } else {
                None
            };
            if self.peek().kind == TokenKind::Equals {
                return Err(FrontendError::unsupported(format!(
                    "default value for kernel parameter `{name}` (line {line})"
                )));
            }
            args.push(KernelArg {
                name,
                annotation,
                line,
            });
            if self.peek().kind == TokenKind::Comma {
                self.next();
            } else if self.peek().kind != TokenKind::RParen {
                return Err(self.error(format!(
                    "expected `,` or `)`, found {}",
                    self.peek().kind.describe()
                )));
            }
        }
        self.next();
        Ok(args)
    }

    /// Collects an annotation's source text up to (not including) one of the
    /// `stops`, at bracket depth zero.
    fn annotation(&mut self, stops: &[TokenKind]) -> Result<String, FrontendError> {
        let mut text = String::new();
        let mut depth = 0usize;
        loop {
            let kind = self.peek().kind.clone();
            if depth == 0 && stops.contains(&kind) {
                break;
            }
            match &kind {
                TokenKind::Eof | TokenKind::Newline => {
                    return Err(self.error("unterminated parameter annotation"));
                }
                TokenKind::LBracket | TokenKind::LParen => depth += 1,
                TokenKind::RBracket | TokenKind::RParen => depth = depth.saturating_sub(1),
                _ => {}
            }
            text.push_str(&match kind {
                TokenKind::Ident(name) => name,
                TokenKind::LBracket => "[".into(),
                TokenKind::RBracket => "]".into(),
                TokenKind::LParen => "(".into(),
                TokenKind::RParen => ")".into(),
                TokenKind::Comma => ", ".into(),
                TokenKind::Dot => ".".into(),
                other => other.describe(),
            });
            self.next();
        }
        if text.is_empty() {
            return Err(self.error("expected a type annotation"));
        }
        Ok(text)
    }

    /// Parses one statement; `None` for a no-op (`pass`, a docstring).
    fn statement(&mut self) -> Result<Option<Statement>, FrontendError> {
        let start = self.peek().clone();
        let line = start.line;

        match &start.kind {
            TokenKind::Str => {
                self.next();
                self.end_of_statement()?;
                return Ok(None);
            }
            TokenKind::Ident(word) if word == "pass" => {
                self.next();
                self.end_of_statement()?;
                return Ok(None);
            }
            TokenKind::Ident(word) => {
                if let Some((_, reason)) = REFUSED_KEYWORDS.iter().find(|(kw, _)| kw == word) {
                    return Err(FrontendError::unsupported(format!(
                        "{reason} (line {line})"
                    )));
                }
                if word == "break" || word == "continue" {
                    return Err(FrontendError::unsupported(format!(
                        "`{word}` is outside the supported subset (line {line})"
                    )));
                }
            }
            other => {
                return Err(FrontendError::syntax(
                    format!("expected a statement, found {}", other.describe()),
                    start.line,
                    start.column,
                ));
            }
        }

        // `name = ...` and augmented assignment.
        if matches!(self.peek_at(1), TokenKind::Equals) {
            return self.assignment().map(Some);
        }
        if let TokenKind::Operator(op) = self.peek_at(1)
            && op.ends_with('=')
            && op != "=="
            && op != "!="
        {
            return Err(FrontendError::unsupported(format!(
                "augmented assignment `{op}` (line {line}); classical variables are \
                 outside the supported subset"
            )));
        }

        let callee = self.dotted()?;
        if self.peek().kind != TokenKind::LParen {
            return Err(self.error(format!(
                "expected a call such as `h(q[0])`, found {} after `{}`",
                self.peek().kind.describe(),
                callee.join(".")
            )));
        }
        let args = self.call_args()?;
        self.end_of_statement()?;
        Ok(Some(Statement::Call(Call { callee, args, line })))
    }

    fn assignment(&mut self) -> Result<Statement, FrontendError> {
        let line = self.peek().line;
        let name = self.ident("a variable name")?;
        self.next(); // `=`

        let rhs_start = self.pos;
        let callee = match self.peek().kind {
            TokenKind::Ident(_) => self.dotted()?,
            _ => Vec::new(),
        };
        let is_call = self.peek().kind == TokenKind::LParen;

        match (
            callee
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice(),
            is_call,
        ) {
            (["cudaq", "qvector"], true) => {
                let args = self.call_args()?;
                self.end_of_statement()?;
                match args.as_slice() {
                    [Arg::Expr(size)] => Ok(Statement::Allocate {
                        name,
                        size: Some(size.clone()),
                        line,
                    }),
                    _ => Err(FrontendError::unsupported(format!(
                        "`cudaq.qvector` with {} argument(s) (line {line}); only \
                         `cudaq.qvector(N)` with a constant width is supported",
                        args.len()
                    ))),
                }
            }
            (["cudaq", "qubit"], true) => {
                let args = self.call_args()?;
                self.end_of_statement()?;
                if args.is_empty() {
                    Ok(Statement::Allocate {
                        name,
                        size: None,
                        line,
                    })
                } else {
                    Err(FrontendError::unsupported(format!(
                        "`cudaq.qubit` takes no arguments (line {line})"
                    )))
                }
            }
            ([gate], true) if ["mz", "mx", "my"].contains(gate) => {
                Err(FrontendError::unsupported(format!(
                    "assigning a measurement result to `{name}` (line {line}); measurement \
                     results feed classical logic, which is outside the supported subset — \
                     call `{gate}(...)` as a statement instead"
                )))
            }
            _ => {
                self.pos = rhs_start;
                Err(FrontendError::unsupported(format!(
                    "classical variable `{name}` (line {line}); only `cudaq.qvector(N)` and \
                     `cudaq.qubit()` may be assigned inside a kernel — write angles inline"
                )))
            }
        }
    }

    fn end_of_statement(&mut self) -> Result<(), FrontendError> {
        match self.peek().kind {
            TokenKind::Newline => {
                self.next();
                Ok(())
            }
            TokenKind::Eof => Ok(()),
            _ => Err(self.error(format!(
                "expected end of line, found {}",
                self.peek().kind.describe()
            ))),
        }
    }

    fn dotted(&mut self) -> Result<Vec<String>, FrontendError> {
        let mut path = vec![self.ident("a name")?];
        while self.peek().kind == TokenKind::Dot {
            self.next();
            path.push(self.ident("a name after `.`")?);
        }
        Ok(path)
    }

    fn call_args(&mut self) -> Result<Vec<Arg>, FrontendError> {
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut args = Vec::new();
        while self.peek().kind != TokenKind::RParen {
            if matches!(self.peek().kind, TokenKind::Ident(_))
                && matches!(self.peek_at(1), TokenKind::Equals)
            {
                let line = self.peek().line;
                let name = self.ident("an argument")?;
                return Err(FrontendError::unsupported(format!(
                    "keyword argument `{name}=` (line {line}); only positional arguments are supported"
                )));
            }
            if self.peek().kind == TokenKind::LBracket {
                self.next();
                let mut items = Vec::new();
                while self.peek().kind != TokenKind::RBracket {
                    items.push(self.expr()?);
                    if self.peek().kind == TokenKind::Comma {
                        self.next();
                    } else if self.peek().kind != TokenKind::RBracket {
                        return Err(self.error(format!(
                            "expected `,` or `]`, found {}",
                            self.peek().kind.describe()
                        )));
                    }
                }
                self.next();
                args.push(Arg::List(items));
            } else {
                args.push(Arg::Expr(self.expr()?));
            }
            if self.peek().kind == TokenKind::Comma {
                self.next();
            } else if self.peek().kind != TokenKind::RParen {
                return Err(self.error(format!(
                    "expected `,` or `)`, found {}",
                    self.peek().kind.describe()
                )));
            }
        }
        self.next();
        Ok(args)
    }

    fn expr(&mut self) -> Result<Expr, FrontendError> {
        let mut left = self.term()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => return Ok(left),
            };
            self.next();
            let right = self.term()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn term(&mut self) -> Result<Expr, FrontendError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                _ => return Ok(left),
            };
            self.next();
            let right = self.unary()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, FrontendError> {
        match self.peek().kind {
            TokenKind::Minus => {
                self.next();
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            TokenKind::Plus => {
                self.next();
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Result<Expr, FrontendError> {
        let token = self.peek().clone();
        match token.kind {
            TokenKind::Number(value) => {
                self.next();
                Ok(Expr::Number(value))
            }
            TokenKind::LParen => {
                self.next();
                let inner = self.expr()?;
                self.expect(&TokenKind::RParen, "`)`")?;
                Ok(inner)
            }
            TokenKind::Ident(_) => {
                let path = self.dotted()?;
                match self.peek().kind {
                    TokenKind::LBracket => {
                        self.next();
                        let index = self.expr()?;
                        if self.peek().kind == TokenKind::Colon {
                            return Err(FrontendError::unsupported(format!(
                                "slice of `{}` (line {}); index single qubits instead",
                                path.join("."),
                                token.line
                            )));
                        }
                        self.expect(&TokenKind::RBracket, "`]`")?;
                        Ok(Expr::Index {
                            base: path,
                            index: Box::new(index),
                        })
                    }
                    TokenKind::LParen => Err(FrontendError::unsupported(format!(
                        "call to `{}` inside an expression (line {}); angles must be constants, \
                         `math.pi` arithmetic, or a bare `float` kernel parameter",
                        path.join("."),
                        token.line
                    ))),
                    _ => Ok(Expr::Path(path)),
                }
            }
            TokenKind::Operator(op) => Err(FrontendError::unsupported(format!(
                "operator `{op}` (line {}); expressions support `+ - * /` only",
                token.line
            ))),
            other => Err(FrontendError::syntax(
                format!("expected an expression, found {}", other.describe()),
                token.line,
                token.column,
            )),
        }
        .and_then(|expr| {
            if let TokenKind::Operator(op) = &self.peek().kind {
                return Err(FrontendError::unsupported(format!(
                    "operator `{op}` (line {}); expressions support `+ - * /` only",
                    self.peek().line
                )));
            }
            Ok(expr)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BELL: &str = r#"
import cudaq

@cudaq.kernel
def bell():
    q = cudaq.qvector(2)
    h(q[0])
    x.ctrl(q[0], q[1])
    mz(q)

if __name__ == "__main__":
    print(cudaq.sample(bell, shots_count=1000))
"#;

    #[test]
    fn extraction_stops_at_host_code() {
        let (region, first) = extract(BELL).unwrap();
        assert_eq!(first, 5);
        assert!(region.starts_with("def bell():"));
        assert!(!region.contains("__main__"), "{region}");
    }

    #[test]
    fn a_bell_kernel_parses() {
        let kernel = parse(BELL).unwrap();
        assert_eq!(kernel.name, "bell");
        assert!(kernel.args.is_empty());
        assert_eq!(kernel.body.len(), 4);
        assert!(matches!(
            &kernel.body[2],
            Statement::Call(Call { callee, .. }) if callee == &["x".to_string(), "ctrl".to_string()]
        ));
    }

    #[test]
    fn parenthesised_decorator_is_accepted() {
        let kernel = parse("@cudaq.kernel()\ndef k():\n    q = cudaq.qubit()\n    h(q)\n").unwrap();
        assert_eq!(kernel.body.len(), 2);
    }

    #[test]
    fn annotated_arguments_are_recorded_verbatim() {
        let kernel =
            parse("@cudaq.kernel\ndef k(theta: float, xs: list[float]) -> None:\n    pass\n")
                .unwrap();
        assert_eq!(kernel.args[0].annotation.as_deref(), Some("float"));
        assert_eq!(kernel.args[1].annotation.as_deref(), Some("list[float]"));
    }

    #[test]
    fn docstrings_and_pass_are_no_ops() {
        let kernel = parse("@cudaq.kernel\ndef k():\n    \"\"\"doc\"\"\"\n    pass\n").unwrap();
        assert!(kernel.body.is_empty());
    }

    #[test]
    fn a_list_of_controls_parses() {
        let kernel = parse(
            "@cudaq.kernel\ndef k():\n    q = cudaq.qvector(3)\n    x.ctrl([q[0], q[1]], q[2])\n",
        )
        .unwrap();
        let Statement::Call(call) = &kernel.body[1] else {
            panic!("expected a call");
        };
        assert!(matches!(&call.args[0], Arg::List(items) if items.len() == 2));
    }

    #[test]
    fn no_kernel_is_a_semantic_error() {
        assert!(matches!(
            parse("import cudaq\nprint('hi')\n"),
            Err(FrontendError::Semantic(_))
        ));
    }

    #[test]
    fn two_kernels_are_refused() {
        let source = "@cudaq.kernel\ndef a():\n    pass\n@cudaq.kernel\ndef b():\n    pass\n";
        assert!(
            matches!(parse(source), Err(FrontendError::Unsupported(m)) if m.contains("exactly one"))
        );
    }

    #[test]
    fn decorator_arguments_are_refused() {
        assert!(matches!(
            parse("@cudaq.kernel(verbose=True)\ndef k():\n    pass\n"),
            Err(FrontendError::Unsupported(_))
        ));
    }

    #[test]
    fn control_flow_is_refused_by_name() {
        for (snippet, word) in [
            ("for i in range(2):\n        h(q[i])", "for"),
            ("if True:\n        h(q[0])", "if"),
            ("while True:\n        pass", "while"),
            ("return", "return"),
        ] {
            let source =
                format!("@cudaq.kernel\ndef k():\n    q = cudaq.qvector(2)\n    {snippet}\n");
            let error = parse(&source).unwrap_err();
            assert!(
                matches!(&error, FrontendError::Unsupported(m) if m.contains(word)),
                "{word}: {error:?}"
            );
        }
    }

    #[test]
    fn classical_assignment_is_refused() {
        let error = parse("@cudaq.kernel\ndef k():\n    angle = 0.5\n").unwrap_err();
        assert!(matches!(error, FrontendError::Unsupported(m) if m.contains("classical variable")));
    }

    #[test]
    fn assigning_a_measurement_is_refused() {
        let error =
            parse("@cudaq.kernel\ndef k():\n    q = cudaq.qubit()\n    b = mz(q)\n").unwrap_err();
        assert!(matches!(error, FrontendError::Unsupported(m) if m.contains("measurement result")));
    }

    #[test]
    fn keyword_arguments_are_refused() {
        let error =
            parse("@cudaq.kernel\ndef k():\n    q = cudaq.qubit()\n    rx(angle=0.5, target=q)\n")
                .unwrap_err();
        assert!(matches!(error, FrontendError::Unsupported(m) if m.contains("keyword")));
    }

    #[test]
    fn power_operator_is_refused() {
        let error = parse("@cudaq.kernel\ndef k():\n    q = cudaq.qubit()\n    rx(2 ** 3, q)\n")
            .unwrap_err();
        assert!(matches!(error, FrontendError::Unsupported(m) if m.contains("**")));
    }

    #[test]
    fn inconsistent_indentation_is_a_syntax_error() {
        let error =
            parse("@cudaq.kernel\ndef k():\n    q = cudaq.qubit()\n      h(q)\n").unwrap_err();
        assert!(
            matches!(error, FrontendError::Syntax { line: 4, .. }),
            "{error:?}"
        );
    }
}
