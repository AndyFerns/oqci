//! The AST for the supported OpenQASM 3 subset.
//!
//! This is deliberately a *narrow* tree: it models exactly the constructs
//! listed in `docs/openqasm_frontend.md` and nothing else. Anything outside
//! the subset is rejected during parsing rather than being represented here
//! and dropped later — an unrepresentable construct should be impossible to
//! hold, not merely ignored.

/// A parsed OpenQASM 3 program.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    /// The version given in the `OPENQASM <version>;` header, if present.
    pub version: Option<String>,
    /// Statements in source order.
    pub statements: Vec<Statement>,
}

/// One supported top-level statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// `include "stdgates.inc";` — accepted and recorded, never loaded.
    Include(String),
    /// `qubit[n] name;` or `qubit name;`
    QubitDecl {
        /// Register name.
        name: String,
        /// Declared width; `None` means a single unindexed qubit.
        size: Option<u32>,
        /// Source position of the declaration.
        pos: Pos,
    },
    /// `bit[n] name;` or `bit name;`
    BitDecl {
        /// Register name.
        name: String,
        /// Declared width; `None` means a single unindexed bit.
        size: Option<u32>,
        /// Source position of the declaration.
        pos: Pos,
    },
    /// `input float[64] name;` — a symbolic circuit parameter.
    InputDecl {
        /// Parameter name.
        name: String,
        /// Source position of the declaration.
        pos: Pos,
    },
    /// A gate application, e.g. `rz(pi/2) q[0];` or `cx a, b;`
    GateCall {
        /// Gate mnemonic as written.
        name: String,
        /// Classical parameter expressions, in source order.
        params: Vec<Expr>,
        /// Qubit operands, in source order.
        operands: Vec<Operand>,
        /// Source position of the gate name.
        pos: Pos,
    },
    /// `measure q[0] -> c[0];` or `c[0] = measure q[0];`
    Measure {
        /// Qubit being measured.
        source: Operand,
        /// Destination classical bit.
        target: Operand,
        /// Source position of the statement.
        pos: Pos,
    },
    /// `reset q[0];`
    Reset {
        /// Qubit being reset.
        target: Operand,
        /// Source position of the statement.
        pos: Pos,
    },
}

/// A reference to a whole register or one of its elements.
#[derive(Debug, Clone, PartialEq)]
pub struct Operand {
    /// Register name.
    pub name: String,
    /// Element index, or `None` to reference the whole register (broadcast).
    pub index: Option<u32>,
    /// Source position of the reference.
    pub pos: Pos,
}

/// A classical parameter expression.
///
/// Arithmetic exists so that idiomatic angles like `pi/2` and `-pi` work; it
/// is constant-folded during translation. Compound expressions over a
/// *symbolic* parameter are rejected — see `docs/openqasm_frontend.md`.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A numeric literal.
    Number(f64),
    /// An identifier: either the constant `pi`, or an `input` parameter.
    Ident(String),
    /// Unary negation.
    Neg(Box<Expr>),
    /// A binary arithmetic operation.
    Binary {
        /// The operator.
        op: BinOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
}

/// Supported binary arithmetic operators.
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

/// A 1-based source position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub column: u32,
}
