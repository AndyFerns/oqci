//! Recursive-descent parser for the supported OpenQASM 3 subset.
//!
//! Out-of-subset constructs are rejected here, by name, with
//! [`FrontendError::Unsupported`] — the parser never skips a statement it does
//! not understand. That is the Stage F boundary made mechanical: a program
//! containing `if` is refused rather than silently compiled as though the
//! branch were unconditional.

use crate::frontend::error::FrontendError;
use crate::frontend::openqasm::ast::{BinOp, Expr, Operand, Pos, Program, Statement};
use crate::frontend::openqasm::lexer::{Token, TokenKind, tokenize};

/// Keywords that name a construct outside the supported subset. Each is
/// reported with its own message so the diagnostic says what was refused.
const UNSUPPORTED_KEYWORDS: &[(&str, &str)] = &[
    ("if", "classical control flow (`if`)"),
    ("else", "classical control flow (`else`)"),
    ("for", "loops (`for`)"),
    ("while", "loops (`while`)"),
    ("def", "subroutine definitions (`def`)"),
    ("defcal", "calibration definitions (`defcal`)"),
    ("gate", "user-defined gate definitions (`gate`)"),
    ("extern", "external declarations (`extern`)"),
    ("output", "`output` declarations"),
    ("barrier", "`barrier`"),
    ("delay", "`delay`"),
    ("box", "`box`"),
    ("array", "array declarations"),
    ("return", "`return`"),
    ("end", "`end`"),
];

/// Parses an OpenQASM 3 source string into a [`Program`].
///
/// # Errors
///
/// [`FrontendError::Syntax`] for malformed source, or
/// [`FrontendError::Unsupported`] for a construct outside the documented
/// subset.
pub fn parse(source: &str) -> Result<Program, FrontendError> {
    Parser {
        tokens: tokenize(source)?,
        at: 0,
    }
    .program()
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> &TokenKind {
        &self.tokens[self.at].kind
    }

    fn peek_at(&self, offset: usize) -> &TokenKind {
        let index = (self.at + offset).min(self.tokens.len() - 1);
        &self.tokens[index].kind
    }

    fn pos(&self) -> Pos {
        Pos {
            line: self.tokens[self.at].line,
            column: self.tokens[self.at].column,
        }
    }

    fn bump(&mut self) -> Token {
        let token = self.tokens[self.at].clone();
        if self.at + 1 < self.tokens.len() {
            self.at += 1;
        }
        token
    }

    fn error(&self, message: impl Into<String>) -> FrontendError {
        let pos = self.pos();
        FrontendError::syntax(message, pos.line, pos.column)
    }

    fn expect(&mut self, expected: &TokenKind) -> Result<Token, FrontendError> {
        if self.peek() == expected {
            Ok(self.bump())
        } else {
            Err(self.error(format!(
                "expected {}, found {}",
                expected.describe(),
                self.peek().describe()
            )))
        }
    }

    fn expect_ident(&mut self) -> Result<String, FrontendError> {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.error(format!(
                "expected an identifier, found {}",
                other.describe()
            ))),
        }
    }

    /// Parses `[n]`, returning `n`. The caller has already seen the `[`.
    fn expect_index(&mut self) -> Result<u32, FrontendError> {
        self.expect(&TokenKind::LBracket)?;
        let value = match self.peek().clone() {
            TokenKind::Number(value) => {
                self.bump();
                value
            }
            other => {
                return Err(self.error(format!(
                    "expected an integer index, found {}",
                    other.describe()
                )));
            }
        };
        if value < 0.0 || value.fract() != 0.0 || value > f64::from(u32::MAX) {
            return Err(self.error(format!(
                "index must be a non-negative integer, found {value}"
            )));
        }
        self.expect(&TokenKind::RBracket)?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(value as u32)
    }

    fn program(&mut self) -> Result<Program, FrontendError> {
        let mut version = None;
        let mut statements = Vec::new();

        // Optional `OPENQASM <version>;` header.
        if matches!(self.peek(), TokenKind::Ident(name) if name == "OPENQASM") {
            self.bump();
            version = Some(match self.peek().clone() {
                TokenKind::Number(value) => {
                    self.bump();
                    format!("{value}")
                }
                other => {
                    return Err(self.error(format!(
                        "expected a version number after `OPENQASM`, found {}",
                        other.describe()
                    )));
                }
            });
            self.expect(&TokenKind::Semicolon)?;
        }

        while *self.peek() != TokenKind::Eof {
            statements.push(self.statement()?);
        }

        Ok(Program {
            version,
            statements,
        })
    }

    fn statement(&mut self) -> Result<Statement, FrontendError> {
        let pos = self.pos();
        let TokenKind::Ident(head) = self.peek().clone() else {
            return Err(self.error(format!(
                "expected a statement, found {}",
                self.peek().describe()
            )));
        };

        if let Some((_, description)) = UNSUPPORTED_KEYWORDS.iter().find(|(kw, _)| *kw == head) {
            return Err(FrontendError::unsupported(format!(
                "{description} at line {}, column {}",
                pos.line, pos.column
            )));
        }

        match head.as_str() {
            "include" => {
                self.bump();
                let path = match self.peek().clone() {
                    TokenKind::Str(path) => {
                        self.bump();
                        path
                    }
                    other => {
                        return Err(self.error(format!(
                            "expected a quoted path after `include`, found {}",
                            other.describe()
                        )));
                    }
                };
                self.expect(&TokenKind::Semicolon)?;
                Ok(Statement::Include(path))
            }
            "qubit" | "bit" | "qreg" | "creg" => self.declaration(&head, pos),
            "input" => self.input_declaration(pos),
            "measure" => {
                self.bump();
                let source = self.operand()?;
                self.expect(&TokenKind::Arrow)?;
                let target = self.operand()?;
                self.expect(&TokenKind::Semicolon)?;
                Ok(Statement::Measure {
                    source,
                    target,
                    pos,
                })
            }
            "reset" => {
                self.bump();
                let target = self.operand()?;
                self.expect(&TokenKind::Semicolon)?;
                Ok(Statement::Reset { target, pos })
            }
            // Either an assignment-form measurement (`c[0] = measure q[0];`)
            // or a gate call (`h q[0];`, `rz(pi) q[0];`).
            _ => {
                let assignment = matches!(self.peek_at(1), TokenKind::Equals)
                    || (matches!(self.peek_at(1), TokenKind::LBracket)
                        && matches!(self.peek_at(4), TokenKind::Equals));
                if assignment {
                    self.measure_assignment(pos)
                } else {
                    self.gate_call(pos)
                }
            }
        }
    }

    fn declaration(&mut self, head: &str, pos: Pos) -> Result<Statement, FrontendError> {
        if head == "qreg" || head == "creg" {
            return Err(FrontendError::unsupported(format!(
                "OpenQASM 2 register syntax (`{head}`) at line {}, column {}; use `qubit[n]` / `bit[n]`",
                pos.line, pos.column
            )));
        }
        self.bump();
        let size = if matches!(self.peek(), TokenKind::LBracket) {
            Some(self.expect_index()?)
        } else {
            None
        };
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(if head == "qubit" {
            Statement::QubitDecl { name, size, pos }
        } else {
            Statement::BitDecl { name, size, pos }
        })
    }

    fn input_declaration(&mut self, pos: Pos) -> Result<Statement, FrontendError> {
        self.bump();
        let ty = self.expect_ident()?;
        if ty != "float" && ty != "angle" {
            return Err(FrontendError::unsupported(format!(
                "`input` of type `{ty}` at line {}, column {}; only `float`/`angle` parameters are supported",
                pos.line, pos.column
            )));
        }
        if matches!(self.peek(), TokenKind::LBracket) {
            self.expect_index()?;
        }
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Statement::InputDecl { name, pos })
    }

    fn measure_assignment(&mut self, pos: Pos) -> Result<Statement, FrontendError> {
        let target = self.operand()?;
        self.expect(&TokenKind::Equals)?;
        match self.peek().clone() {
            TokenKind::Ident(name) if name == "measure" => self.bump(),
            other => {
                return Err(self.error(format!(
                    "expected `measure` after `=`, found {}",
                    other.describe()
                )));
            }
        };
        let source = self.operand()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Statement::Measure {
            source,
            target,
            pos,
        })
    }

    fn gate_call(&mut self, pos: Pos) -> Result<Statement, FrontendError> {
        let name = self.expect_ident()?;

        let mut params = Vec::new();
        if matches!(self.peek(), TokenKind::LParen) {
            self.bump();
            if !matches!(self.peek(), TokenKind::RParen) {
                loop {
                    params.push(self.expression()?);
                    if matches!(self.peek(), TokenKind::Comma) {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen)?;
        }

        let mut operands = vec![self.operand()?];
        while matches!(self.peek(), TokenKind::Comma) {
            self.bump();
            operands.push(self.operand()?);
        }
        self.expect(&TokenKind::Semicolon)?;

        Ok(Statement::GateCall {
            name,
            params,
            operands,
            pos,
        })
    }

    fn operand(&mut self) -> Result<Operand, FrontendError> {
        let pos = self.pos();
        let name = self.expect_ident()?;
        let index = if matches!(self.peek(), TokenKind::LBracket) {
            Some(self.expect_index()?)
        } else {
            None
        };
        Ok(Operand { name, index, pos })
    }

    // --- Expressions: `+`/`-` over `*`//`, then unary `-`, then atoms -------

    fn expression(&mut self) -> Result<Expr, FrontendError> {
        let mut lhs = self.term()?;
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.term()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn term(&mut self) -> Result<Expr, FrontendError> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                _ => break,
            };
            self.bump();
            let rhs = self.unary()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, FrontendError> {
        if matches!(self.peek(), TokenKind::Minus) {
            self.bump();
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Expr, FrontendError> {
        match self.peek().clone() {
            TokenKind::Number(value) => {
                self.bump();
                Ok(Expr::Number(value))
            }
            TokenKind::Ident(name) => {
                self.bump();
                Ok(Expr::Ident(name))
            }
            TokenKind::LParen => {
                self.bump();
                let inner = self.expression()?;
                self.expect(&TokenKind::RParen)?;
                Ok(inner)
            }
            other => Err(self.error(format!(
                "expected a parameter expression, found {}",
                other.describe()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statements(source: &str) -> Vec<Statement> {
        parse(source).unwrap().statements
    }

    #[test]
    fn parses_header_and_include() {
        let program = parse("OPENQASM 3.0;\ninclude \"stdgates.inc\";").unwrap();
        assert_eq!(program.version.as_deref(), Some("3"));
        assert_eq!(
            program.statements,
            vec![Statement::Include("stdgates.inc".into())]
        );
    }

    #[test]
    fn parses_declarations() {
        let stmts = statements("qubit[2] q; bit[2] c; qubit single; bit flag;");
        assert!(matches!(
            &stmts[0],
            Statement::QubitDecl { name, size: Some(2), .. } if name == "q"
        ));
        assert!(matches!(
            &stmts[1],
            Statement::BitDecl { name, size: Some(2), .. } if name == "c"
        ));
        assert!(matches!(&stmts[2], Statement::QubitDecl { size: None, .. }));
        assert!(matches!(&stmts[3], Statement::BitDecl { size: None, .. }));
    }

    #[test]
    fn parses_input_declaration() {
        let stmts = statements("input float[64] theta;");
        assert!(matches!(&stmts[0], Statement::InputDecl { name, .. } if name == "theta"));
    }

    #[test]
    fn parses_gate_calls_with_and_without_params() {
        let stmts = statements("h q[0]; cx q[0], q[1]; rz(pi/2) q[0];");
        assert!(matches!(
            &stmts[0],
            Statement::GateCall { name, params, operands, .. }
                if name == "h" && params.is_empty() && operands.len() == 1
        ));
        assert!(matches!(
            &stmts[1],
            Statement::GateCall { name, operands, .. } if name == "cx" && operands.len() == 2
        ));
        assert!(matches!(
            &stmts[2],
            Statement::GateCall { name, params, .. } if name == "rz" && params.len() == 1
        ));
    }

    #[test]
    fn parses_both_measure_forms() {
        let arrow = statements("measure q[0] -> c[0];");
        let assign = statements("c[0] = measure q[0];");
        let (
            Statement::Measure {
                source: s1,
                target: t1,
                ..
            },
            Statement::Measure {
                source: s2,
                target: t2,
                ..
            },
        ) = (&arrow[0], &assign[0])
        else {
            panic!("expected measurements");
        };
        assert_eq!((s1.name.as_str(), s1.index), ("q", Some(0)));
        assert_eq!((t1.name.as_str(), t1.index), ("c", Some(0)));
        assert_eq!((s2.name.as_str(), s2.index), ("q", Some(0)));
        assert_eq!((t2.name.as_str(), t2.index), ("c", Some(0)));
    }

    #[test]
    fn parses_whole_register_measure_assignment() {
        let stmts = statements("c = measure q;");
        assert!(matches!(
            &stmts[0],
            Statement::Measure { source, target, .. }
                if source.index.is_none() && target.index.is_none()
        ));
    }

    #[test]
    fn parses_reset() {
        assert!(matches!(
            &statements("reset q[0];")[0],
            Statement::Reset { .. }
        ));
    }

    #[test]
    fn expression_precedence_is_standard() {
        let stmts = statements("rz(1 + 2 * 3) q[0];");
        let Statement::GateCall { params, .. } = &stmts[0] else {
            panic!("expected a gate call");
        };
        assert_eq!(
            params[0],
            Expr::Binary {
                op: BinOp::Add,
                lhs: Box::new(Expr::Number(1.0)),
                rhs: Box::new(Expr::Binary {
                    op: BinOp::Mul,
                    lhs: Box::new(Expr::Number(2.0)),
                    rhs: Box::new(Expr::Number(3.0)),
                }),
            }
        );
    }

    #[test]
    fn unsupported_keywords_are_named() {
        for (source, needle) in [
            ("if (c == 1) { x q[0]; }", "if"),
            ("for i in [0:2] { x q[0]; }", "for"),
            ("while (true) { x q[0]; }", "while"),
            ("gate mygate a { x a; }", "gate"),
            ("def f() { }", "def"),
            ("output bit c;", "output"),
            ("barrier q;", "barrier"),
        ] {
            let error = parse(source).unwrap_err();
            assert!(
                matches!(&error, FrontendError::Unsupported(msg) if msg.contains(needle)),
                "{source} produced {error:?}"
            );
        }
    }

    #[test]
    fn openqasm2_register_syntax_is_refused_with_guidance() {
        let error = parse("qreg q[2];").unwrap_err();
        assert!(matches!(&error, FrontendError::Unsupported(msg) if msg.contains("qubit[n]")));
    }

    #[test]
    fn missing_semicolon_is_a_syntax_error() {
        assert!(matches!(parse("h q[0]"), Err(FrontendError::Syntax { .. })));
    }

    #[test]
    fn syntax_errors_carry_position() {
        let error = parse("qubit[2] q;\nh ;").unwrap_err();
        let FrontendError::Syntax { line, column, .. } = error else {
            panic!("expected a syntax error, got {error:?}");
        };
        assert_eq!(line, 2);
        assert_eq!(column, 3);
    }

    #[test]
    fn non_integer_index_is_rejected() {
        assert!(matches!(
            parse("qubit[2.5] q;"),
            Err(FrontendError::Syntax { .. })
        ));
    }
}
