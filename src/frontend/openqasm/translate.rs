//! AST → QC-IR translation.
//!
//! The translator owns exactly one job: resolving OpenQASM's *named, indexed*
//! register model onto QC-IR's *flat, dense* qubit and classical spaces, then
//! replaying the program through [`CircuitBuilder`]. It performs no validation
//! that QC-IR already performs — arity, operand duplication, and range checks
//! all surface from [`CircuitBuilder::build`] as
//! [`FrontendError::Ir`], per Stage A §3.2.

use std::collections::{HashMap, HashSet};

use crate::frontend::error::FrontendError;
use crate::frontend::gate_map::map_gate;
use crate::frontend::openqasm::ast::{BinOp, Expr, Operand, Program, Statement};
use crate::ir::{Circuit, CircuitBuilder, ClbitId, Param, QubitId};

/// A named register's slice of the flat QC-IR index space.
#[derive(Debug, Clone, Copy)]
struct Register {
    start: u32,
    size: u32,
}

impl Register {
    fn resolve(&self, index: u32) -> Option<u32> {
        (index < self.size).then_some(self.start + index)
    }
}

#[derive(Default)]
struct Scope {
    qubits: HashMap<String, Register>,
    clbits: HashMap<String, Register>,
    inputs: HashSet<String>,
}

/// Translates a parsed [`Program`] into a validated [`Circuit`].
///
/// # Errors
///
/// [`FrontendError::Semantic`] for undeclared or out-of-range register
/// references and mismatched broadcast widths, [`FrontendError::Unsupported`]
/// for compound symbolic expressions, and [`FrontendError::Ir`] for anything
/// QC-IR's own validation rejects.
pub fn translate(program: &Program, name: &str) -> Result<Circuit, FrontendError> {
    let mut builder = CircuitBuilder::new(name);
    let mut scope = Scope::default();

    for statement in &program.statements {
        match statement {
            Statement::Include(_) => {}

            Statement::QubitDecl { name, size, pos } => {
                if scope.qubits.contains_key(name) || scope.clbits.contains_key(name) {
                    return Err(FrontendError::semantic(format!(
                        "register `{name}` is already declared (line {}, column {})",
                        pos.line, pos.column
                    )));
                }
                let size = size.unwrap_or(1);
                let start = builder.alloc_qubits(size).first().map_or(0, |q| q.index());
                scope.qubits.insert(name.clone(), Register { start, size });
            }

            Statement::BitDecl { name, size, pos } => {
                if scope.qubits.contains_key(name) || scope.clbits.contains_key(name) {
                    return Err(FrontendError::semantic(format!(
                        "register `{name}` is already declared (line {}, column {})",
                        pos.line, pos.column
                    )));
                }
                let size = size.unwrap_or(1);
                let start = builder.alloc_clbits(size).first().map_or(0, |c| c.index());
                scope.clbits.insert(name.clone(), Register { start, size });
            }

            Statement::InputDecl { name, pos } => {
                if !scope.inputs.insert(name.clone()) {
                    return Err(FrontendError::semantic(format!(
                        "parameter `{name}` is already declared (line {}, column {})",
                        pos.line, pos.column
                    )));
                }
            }

            Statement::GateCall {
                name,
                params,
                operands,
                pos,
            } => {
                let params: Vec<Param> = params
                    .iter()
                    .map(|expr| eval_param(expr, &scope.inputs))
                    .collect::<Result<_, _>>()?;

                let operand_indices = broadcast(operands, &scope.qubits, "qubit", *pos)?;
                for targets in operand_indices {
                    let kind = map_gate(name, params.clone())?;
                    let qubits: Vec<QubitId> = targets.into_iter().map(QubitId).collect();
                    builder.gate(kind, qubits);
                }
            }

            Statement::Measure {
                source,
                target,
                pos,
            } => {
                let qubits = expand(source, &scope.qubits, "qubit")?;
                let clbits = expand(target, &scope.clbits, "classical bit")?;
                if qubits.len() != clbits.len() {
                    return Err(FrontendError::semantic(format!(
                        "measurement broadcasts {} qubit(s) onto {} classical bit(s) (line {}, column {})",
                        qubits.len(),
                        clbits.len(),
                        pos.line,
                        pos.column
                    )));
                }
                for (q, c) in qubits.into_iter().zip(clbits) {
                    builder.measure(QubitId(q), ClbitId(c));
                }
            }

            Statement::Reset { target, .. } => {
                for q in expand(target, &scope.qubits, "qubit")? {
                    builder.reset(QubitId(q));
                }
            }
        }
    }

    Ok(builder.build()?)
}

/// Expands one operand to the flat indices it names: a single element, or
/// every element of a whole register.
fn expand(
    operand: &Operand,
    registers: &HashMap<String, Register>,
    what: &str,
) -> Result<Vec<u32>, FrontendError> {
    let register = registers.get(&operand.name).ok_or_else(|| {
        FrontendError::semantic(format!(
            "undeclared {what} register `{}` (line {}, column {})",
            operand.name, operand.pos.line, operand.pos.column
        ))
    })?;

    match operand.index {
        Some(index) => {
            let flat = register.resolve(index).ok_or_else(|| {
                FrontendError::semantic(format!(
                    "index {index} is out of range for `{}`, which has width {} (line {}, column {})",
                    operand.name, register.size, operand.pos.line, operand.pos.column
                ))
            })?;
            Ok(vec![flat])
        }
        None => Ok((0..register.size).map(|i| register.start + i).collect()),
    }
}

/// Resolves a gate's operand list, applying OpenQASM's broadcast rule.
///
/// Supported shapes (see `docs/openqasm_frontend.md`):
/// - every operand indexed → a single application;
/// - every operand a whole register of the same width `n` → `n` applications;
/// - a single-qubit gate over one whole register of width `n` → `n`
///   applications.
///
/// Mixing indexed and whole-register operands is refused rather than guessed.
fn broadcast(
    operands: &[Operand],
    registers: &HashMap<String, Register>,
    what: &str,
    pos: crate::frontend::openqasm::ast::Pos,
) -> Result<Vec<Vec<u32>>, FrontendError> {
    let expanded: Vec<Vec<u32>> = operands
        .iter()
        .map(|operand| expand(operand, registers, what))
        .collect::<Result<_, _>>()?;

    let all_indexed = operands.iter().all(|o| o.index.is_some());
    if all_indexed {
        return Ok(vec![expanded.into_iter().flatten().collect()]);
    }

    let any_indexed = operands.iter().any(|o| o.index.is_some());
    if any_indexed {
        return Err(FrontendError::unsupported(format!(
            "mixing indexed and whole-register operands in one gate call (line {}, column {})",
            pos.line, pos.column
        )));
    }

    let width = expanded[0].len();
    if expanded.iter().any(|slot| slot.len() != width) {
        return Err(FrontendError::semantic(format!(
            "cannot broadcast registers of differing widths in one gate call (line {}, column {})",
            pos.line, pos.column
        )));
    }

    Ok((0..width)
        .map(|i| expanded.iter().map(|slot| slot[i]).collect())
        .collect())
}

/// Evaluates a parameter expression to a [`Param`].
///
/// A bare `input`-declared identifier becomes [`Param::Symbol`]. Everything
/// else must fold to a concrete value: combining a symbol with arithmetic is
/// refused, because QC-IR represents a symbol, not a symbolic *expression*.
fn eval_param(expr: &Expr, inputs: &HashSet<String>) -> Result<Param, FrontendError> {
    if let Expr::Ident(name) = expr
        && inputs.contains(name)
    {
        return Ok(Param::symbol(name));
    }
    eval_numeric(expr, inputs).map(Param::Concrete)
}

fn eval_numeric(expr: &Expr, inputs: &HashSet<String>) -> Result<crate::ir::Angle, FrontendError> {
    let value = eval_f64(expr, inputs)?;
    Ok(crate::ir::Angle::new(value))
}

fn eval_f64(expr: &Expr, inputs: &HashSet<String>) -> Result<f64, FrontendError> {
    match expr {
        Expr::Number(value) => Ok(*value),
        Expr::Ident(name) => match name.as_str() {
            "pi" | "PI" | "π" => Ok(std::f64::consts::PI),
            "tau" | "TAU" | "τ" => Ok(std::f64::consts::TAU),
            "euler" | "ℇ" => Ok(std::f64::consts::E),
            _ if inputs.contains(name) => Err(FrontendError::unsupported(format!(
                "compound expression over symbolic parameter `{name}`; only a bare parameter may be used as a gate argument"
            ))),
            _ => Err(FrontendError::semantic(format!(
                "unknown parameter or constant `{name}`"
            ))),
        },
        Expr::Neg(inner) => Ok(-eval_f64(inner, inputs)?),
        Expr::Binary { op, lhs, rhs } => {
            let (lhs, rhs) = (eval_f64(lhs, inputs)?, eval_f64(rhs, inputs)?);
            Ok(match op {
                BinOp::Add => lhs + rhs,
                BinOp::Sub => lhs - rhs,
                BinOp::Mul => lhs * rhs,
                BinOp::Div => lhs / rhs,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::openqasm::parser::parse;
    use crate::ir::{GateKind, Instruction};

    fn circuit(source: &str) -> Circuit {
        translate(&parse(source).unwrap(), "test").unwrap()
    }

    fn error(source: &str) -> FrontendError {
        translate(&parse(source).unwrap(), "test").unwrap_err()
    }

    #[test]
    fn allocates_registers_in_declaration_order() {
        let c = circuit("qubit[2] a; qubit[3] b; bit[2] c;");
        assert_eq!(c.num_qubits(), 5);
        assert_eq!(c.num_clbits(), 2);
    }

    #[test]
    fn indexes_are_relative_to_their_register() {
        let c = circuit("qubit[2] a; qubit[2] b; x b[1];");
        assert_eq!(
            c.instructions(),
            [Instruction::Gate {
                kind: GateKind::X,
                qubits: vec![QubitId(3)]
            }]
        );
    }

    #[test]
    fn single_qubit_gate_broadcasts_over_a_register() {
        let c = circuit("qubit[3] q; h q;");
        assert_eq!(c.len(), 3);
        assert_eq!(c.instructions()[2].qubits(), vec![QubitId(2)]);
    }

    #[test]
    fn two_qubit_gate_broadcasts_pairwise() {
        let c = circuit("qubit[2] a; qubit[2] b; cx a, b;");
        assert_eq!(
            c.instructions(),
            [
                Instruction::Gate {
                    kind: GateKind::Cx,
                    qubits: vec![QubitId(0), QubitId(2)]
                },
                Instruction::Gate {
                    kind: GateKind::Cx,
                    qubits: vec![QubitId(1), QubitId(3)]
                },
            ]
        );
    }

    #[test]
    fn measurement_broadcasts_pairwise() {
        let c = circuit("qubit[2] q; bit[2] r; r = measure q;");
        assert_eq!(
            c.instructions(),
            [
                Instruction::Measure {
                    qubit: QubitId(0),
                    target: ClbitId(0)
                },
                Instruction::Measure {
                    qubit: QubitId(1),
                    target: ClbitId(1)
                },
            ]
        );
    }

    #[test]
    fn angle_arithmetic_is_constant_folded() {
        let c = circuit("qubit[1] q; rz(pi/2) q[0];");
        assert_eq!(
            c.instructions()[0],
            Instruction::Gate {
                kind: GateKind::Rz(Param::concrete(std::f64::consts::FRAC_PI_2)),
                qubits: vec![QubitId(0)]
            }
        );
    }

    #[test]
    fn negative_angles_fold() {
        let c = circuit("qubit[1] q; rx(-pi) q[0];");
        assert_eq!(
            c.instructions()[0],
            Instruction::Gate {
                kind: GateKind::Rx(Param::concrete(-std::f64::consts::PI)),
                qubits: vec![QubitId(0)]
            }
        );
    }

    #[test]
    fn input_parameters_become_symbols() {
        let c = circuit("input float[64] theta; qubit[1] q; rz(theta) q[0];");
        assert_eq!(c.parameters(), vec!["theta".to_string()]);
        assert!(!c.is_concrete());
    }

    #[test]
    fn unknown_gate_becomes_opaque() {
        let c = circuit("qubit[2] q; iswap q[0], q[1];");
        assert!(matches!(
            &c.instructions()[0],
            Instruction::Gate {
                kind: GateKind::Opaque { name, .. },
                ..
            } if name == "iswap"
        ));
    }

    #[test]
    fn undeclared_register_is_rejected() {
        assert!(matches!(
            error("h q[0];"),
            FrontendError::Semantic(msg) if msg.contains("undeclared")
        ));
    }

    #[test]
    fn out_of_range_index_is_rejected() {
        assert!(matches!(
            error("qubit[2] q; x q[5];"),
            FrontendError::Semantic(msg) if msg.contains("out of range")
        ));
    }

    #[test]
    fn duplicate_declaration_is_rejected() {
        assert!(matches!(
            error("qubit[2] q; qubit[2] q;"),
            FrontendError::Semantic(msg) if msg.contains("already declared")
        ));
    }

    #[test]
    fn mismatched_broadcast_widths_are_rejected() {
        assert!(matches!(
            error("qubit[2] a; qubit[3] b; cx a, b;"),
            FrontendError::Semantic(msg) if msg.contains("differing widths")
        ));
    }

    #[test]
    fn mixed_indexed_and_register_operands_are_rejected() {
        assert!(matches!(
            error("qubit[2] a; qubit[2] b; cx a[0], b;"),
            FrontendError::Unsupported(msg) if msg.contains("mixing")
        ));
    }

    #[test]
    fn compound_symbolic_expression_is_rejected() {
        assert!(matches!(
            error("input float[64] theta; qubit[1] q; rz(2*theta) q[0];"),
            FrontendError::Unsupported(msg) if msg.contains("compound expression")
        ));
    }

    #[test]
    fn unknown_constant_is_rejected() {
        assert!(matches!(
            error("qubit[1] q; rz(gamma) q[0];"),
            FrontendError::Semantic(msg) if msg.contains("unknown parameter")
        ));
    }

    #[test]
    fn ir_validation_errors_propagate() {
        // `cx` on the same qubit twice is an IR-level invariant, not a parse
        // error — the frontend must surface it rather than pre-empt it.
        let error = error("qubit[2] q; cx q[0], q[0];");
        assert!(matches!(
            error,
            FrontendError::Ir(crate::ir::IrError::DuplicateQubit { .. })
        ));
    }

    #[test]
    fn gate_parameter_arity_is_checked() {
        assert!(matches!(
            error("qubit[1] q; rz() q[0];"),
            FrontendError::Syntax { .. } | FrontendError::ParamArity { .. }
        ));
    }
}
