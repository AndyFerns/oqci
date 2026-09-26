//! Parsed kernel → validated QC-IR.
//!
//! Every gate name goes through the shared [`map_gate`] table, so this
//! frontend and the OpenQASM one agree on what `rz` means by construction, and
//! every circuit is built through [`CircuitBuilder`], so QC-IR's own
//! invariants — operand ranges, duplicate operands, finite angles — are
//! enforced by the IR rather than re-implemented here.
//!
//! # Operand model
//!
//! Each `cudaq.qvector(N)` / `cudaq.qubit()` is laid out on one flat qubit
//! space in allocation order, as OpenQASM registers are. Every measurement
//! gets an explicit destination: **classical bit `i` receives qubit `i`**, so
//! `mz(q[0]); mz(q[1]); mz(q[2])` is three `Measure` instructions writing
//! three distinct bits. There is no "measured qubits" list for a later call to
//! overwrite.
//!
//! # Verified subset
//!
//! The operations accepted here are the ones NVIDIA's CUDA-Q documentation
//! shows in Python kernel syntax (`api/default_ops.html` and
//! `using/examples/quantum_operations.html`, "latest", retrieved 2026-09-26):
//! see `docs/cudaq_frontend.md`. Anything else is refused by name rather than
//! guessed at — including operations CUDA-Q very likely has but whose Python
//! spelling those pages do not show.

use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;

use crate::frontend::cudaq::parser::{Arg, BinOp, Call, Expr, Kernel, Statement};
use crate::frontend::error::FrontendError;
use crate::frontend::gate_map::map_gate;
use crate::ir::{Circuit, CircuitBuilder, ClbitId, GateKind, Param, QubitId};

/// Single-qubit gates taking no parameters.
const PLAIN: [&str; 6] = ["h", "x", "y", "z", "s", "t"];
/// Single-qubit gates taking one angle, first.
const ROTATIONS: [&str; 4] = ["rx", "ry", "rz", "r1"];

#[derive(Debug, Clone, Copy)]
struct Register {
    offset: u32,
    size: u32,
}

enum Planned {
    Gate(GateKind, Vec<u32>),
    Measure(u32),
}

struct Translator {
    registers: HashMap<String, Register>,
    float_args: HashSet<String>,
    qubits: u32,
    measures: bool,
    planned: Vec<Planned>,
}

/// Translates a parsed kernel into a [`Circuit`] named `name`.
///
/// # Errors
///
/// - [`FrontendError::Unsupported`] for an operation, argument type or
///   expression outside the verified subset.
/// - [`FrontendError::Semantic`] for an undeclared register, an index out of
///   range, or a call with the wrong number of arguments.
/// - [`FrontendError::ParamArity`] from the shared gate table.
/// - [`FrontendError::Ir`] for anything QC-IR itself rejects — including a
///   non-finite angle such as `1 / 0`.
pub fn translate(kernel: &Kernel, name: &str) -> Result<Circuit, FrontendError> {
    let mut t = Translator {
        registers: HashMap::new(),
        float_args: HashSet::new(),
        qubits: 0,
        measures: false,
        planned: Vec::new(),
    };

    for arg in &kernel.args {
        match arg.annotation.as_deref() {
            Some("float") => {
                t.float_args.insert(arg.name.clone());
            }
            Some(other) => {
                return Err(FrontendError::unsupported(format!(
                    "kernel parameter `{}: {other}` (line {}); only `float` parameters are \
                     supported, each becoming a symbolic angle bound before execution",
                    arg.name, arg.line
                )));
            }
            None => {
                return Err(FrontendError::unsupported(format!(
                    "unannotated kernel parameter `{}` (line {}); annotate it `: float`",
                    arg.name, arg.line
                )));
            }
        }
    }

    for statement in &kernel.body {
        match statement {
            Statement::Allocate { name, size, line } => t.allocate(name, size.as_ref(), *line)?,
            Statement::Call(call) => t.call(call)?,
        }
    }

    let mut builder = CircuitBuilder::new(name);
    builder.alloc_qubits(t.qubits);
    builder.alloc_clbits(if t.measures { t.qubits } else { 0 });
    for step in t.planned {
        match step {
            Planned::Gate(kind, qubits) => {
                builder.gate(kind, qubits.into_iter().map(QubitId).collect::<Vec<_>>());
            }
            Planned::Measure(qubit) => {
                builder.measure(QubitId(qubit), ClbitId(qubit));
            }
        }
    }
    Ok(builder.build()?)
}

impl Translator {
    fn allocate(
        &mut self,
        name: &str,
        size: Option<&Expr>,
        line: u32,
    ) -> Result<(), FrontendError> {
        if self.registers.contains_key(name) || self.float_args.contains(name) {
            return Err(FrontendError::semantic(format!(
                "`{name}` is already defined (line {line})"
            )));
        }
        let width = match size {
            None => 1,
            Some(expr) => {
                let value = self.constant(expr, line)?;
                if value.fract() != 0.0 || value < 1.0 || value > f64::from(u32::MAX) {
                    return Err(FrontendError::semantic(format!(
                        "`cudaq.qvector` width must be a positive integer, got {value} (line {line})"
                    )));
                }
                value as u32
            }
        };
        self.registers.insert(
            name.to_string(),
            Register {
                offset: self.qubits,
                size: width,
            },
        );
        self.qubits += width;
        Ok(())
    }

    fn call(&mut self, call: &Call) -> Result<(), FrontendError> {
        let line = call.line;
        let callee: Vec<&str> = call.callee.iter().map(String::as_str).collect();
        let shown = call.callee.join(".");

        match callee.as_slice() {
            [gate] if PLAIN.contains(gate) => {
                let [target] = self.arity::<1>(call, "a qubit or register")?;
                for q in self.qubits_of(target, line)? {
                    self.gate(gate, vec![], vec![q])?;
                }
            }
            [gate] if ROTATIONS.contains(gate) => {
                let [angle, target] = self.arity::<2>(call, "an angle and a qubit")?;
                let angle = self.angle(angle, line)?;
                for q in self.qubits_of(target, line)? {
                    self.gate(gate, vec![angle.clone()], vec![q])?;
                }
            }
            ["u3"] => {
                let [theta, phi, lambda, target] =
                    self.arity::<4>(call, "three angles and a qubit")?;
                let params = vec![
                    self.angle(theta, line)?,
                    self.angle(phi, line)?,
                    self.angle(lambda, line)?,
                ];
                for q in self.qubits_of(target, line)? {
                    self.gate("u3", params.clone(), vec![q])?;
                }
            }
            ["swap"] => {
                let [a, b] = self.arity::<2>(call, "two qubits")?;
                let (a, b) = (self.single(a, line)?, self.single(b, line)?);
                self.gate("swap", vec![], vec![a, b])?;
            }
            ["mz"] => {
                let [target] = self.arity::<1>(call, "a qubit or register")?;
                for q in self.qubits_of(target, line)? {
                    self.planned.push(Planned::Measure(q));
                    self.measures = true;
                }
            }
            [gate, "adj"] => self.adjoint(gate, call)?,
            [gate, "ctrl"] => self.controlled(gate, call)?,
            ["mx" | "my"] => {
                return Err(FrontendError::unsupported(format!(
                    "`{shown}` (line {line}); only computational-basis measurement (`mz`) is \
                     supported"
                )));
            }
            ["sdg" | "tdg"] => {
                return Err(FrontendError::unsupported(format!(
                    "`{shown}` (line {line}) is not a documented CUDA-Q operation; write \
                     `{}.adj(...)`",
                    &shown[..1]
                )));
            }
            ["reset"] => {
                return Err(FrontendError::unsupported(format!(
                    "`reset` (line {line}); its Python spelling is not in the CUDA-Q \
                     documentation this frontend is verified against"
                )));
            }
            ["cudaq", ..] => {
                return Err(FrontendError::unsupported(format!(
                    "`{shown}` inside a kernel (line {line}); only `cudaq.qvector` and \
                     `cudaq.qubit` allocations are supported"
                )));
            }
            _ => {
                return Err(FrontendError::unsupported(format!(
                    "call to `{shown}` (line {line}); kernel calls and custom operations are \
                     outside the supported subset"
                )));
            }
        }
        Ok(())
    }

    fn adjoint(&mut self, gate: &str, call: &Call) -> Result<(), FrontendError> {
        let line = call.line;
        match gate {
            "s" | "t" => {
                let [target] = self.arity::<1>(call, "a qubit or register")?;
                let dagger = if gate == "s" { "sdg" } else { "tdg" };
                for q in self.qubits_of(target, line)? {
                    self.gate(dagger, vec![], vec![q])?;
                }
            }
            // Hermitian: each is its own adjoint.
            "h" | "x" | "y" | "z" => {
                let [target] = self.arity::<1>(call, "a qubit or register")?;
                for q in self.qubits_of(target, line)? {
                    self.gate(gate, vec![], vec![q])?;
                }
            }
            g if ROTATIONS.contains(&g) => {
                let [angle, target] = self.arity::<2>(call, "an angle and a qubit")?;
                // The adjoint of a rotation by θ is the rotation by −θ. That
                // needs arithmetic on the angle, which a symbol does not have.
                let negated = match self.angle(angle, line)? {
                    Param::Concrete(value) => Param::concrete(-value.radians()),
                    Param::Symbol(symbol) => {
                        return Err(FrontendError::unsupported(format!(
                            "`{g}.adj` of symbolic angle `{symbol}` (line {line}); negating a \
                             symbol is a compound expression QC-IR cannot represent"
                        )));
                    }
                };
                for q in self.qubits_of(target, line)? {
                    self.gate(g, vec![negated.clone()], vec![q])?;
                }
            }
            other => {
                return Err(FrontendError::unsupported(format!(
                    "`{other}.adj` (line {line}); adjoints are supported for h, x, y, z, s, t \
                     and the rotations"
                )));
            }
        }
        Ok(())
    }

    fn controlled(&mut self, gate: &str, call: &Call) -> Result<(), FrontendError> {
        let line = call.line;
        let [controls, target] = self.arity::<2>(call, "controls and a target qubit")?;
        let controls = match controls {
            Arg::List(items) => items
                .iter()
                .map(|item| self.single(&Arg::Expr(item.clone()), line))
                .collect::<Result<Vec<_>, _>>()?,
            single => vec![self.single(single, line)?],
        };
        let target = self.single(target, line)?;

        let mnemonic = match (gate, controls.len()) {
            ("x", 1) => "cx",
            ("x", 2) => "ccx",
            ("y", 1) => "cy",
            ("z", 1) => "cz",
            _ => {
                return Err(FrontendError::unsupported(format!(
                    "`{gate}.ctrl` with {} control(s) (line {line}); QC-IR has controlled \
                     forms only for x (1 or 2 controls), y and z (1 control)",
                    controls.len()
                )));
            }
        };
        let mut operands = controls;
        operands.push(target);
        self.gate(mnemonic, vec![], operands)
    }

    fn gate(
        &mut self,
        name: &str,
        params: Vec<Param>,
        qubits: Vec<u32>,
    ) -> Result<(), FrontendError> {
        let kind = map_gate(name, params)?;
        self.planned.push(Planned::Gate(kind, qubits));
        Ok(())
    }

    fn arity<'c, const N: usize>(
        &self,
        call: &'c Call,
        expected: &str,
    ) -> Result<[&'c Arg; N], FrontendError> {
        let args: Vec<&Arg> = call.args.iter().collect();
        args.try_into().map_err(|args: Vec<&Arg>| {
            FrontendError::semantic(format!(
                "`{}` takes {expected} ({N} argument(s)), got {} (line {})",
                call.callee.join("."),
                args.len(),
                call.line
            ))
        })
    }

    /// Resolves a qubit reference to flat qubit indices: one for `q[i]` or a
    /// `cudaq.qubit()`, every qubit of a register for a bare `qvector` name
    /// (CUDA-Q's documented broadcast of single-qubit operations).
    fn qubits_of(&self, arg: &Arg, line: u32) -> Result<Vec<u32>, FrontendError> {
        let expr = match arg {
            Arg::Expr(expr) => expr,
            Arg::List(_) => {
                return Err(FrontendError::semantic(format!(
                    "expected a qubit, found a list (line {line}); lists are only meaningful \
                     as the controls of `.ctrl`"
                )));
            }
        };
        match expr {
            Expr::Path(path) if path.len() == 1 => {
                let register = self.register(&path[0], line)?;
                Ok((register.offset..register.offset + register.size).collect())
            }
            Expr::Index { base, index } if base.len() == 1 => {
                let register = self.register(&base[0], line)?;
                let value = match self.constant(index, line) {
                    Ok(value) => value,
                    Err(_) => {
                        return Err(FrontendError::unsupported(format!(
                            "index into `{}` is not a compile-time constant (line {line}); \
                             indexing by a loop variable or parameter is outside the \
                             supported subset",
                            base[0]
                        )));
                    }
                };
                if value.fract() != 0.0 || value < 0.0 {
                    return Err(FrontendError::unsupported(format!(
                        "index {value} into `{}` (line {line}); only non-negative integer \
                         indices are supported",
                        base[0]
                    )));
                }
                let index = value as u64;
                if index >= u64::from(register.size) {
                    return Err(FrontendError::semantic(format!(
                        "index {index} is out of range for `{}` of width {} (line {line})",
                        base[0], register.size
                    )));
                }
                Ok(vec![register.offset + index as u32])
            }
            _ => Err(FrontendError::semantic(format!(
                "expected a qubit such as `q[0]` or a register name (line {line})"
            ))),
        }
    }

    fn single(&self, arg: &Arg, line: u32) -> Result<u32, FrontendError> {
        match self.qubits_of(arg, line)?.as_slice() {
            [only] => Ok(*only),
            many => Err(FrontendError::unsupported(format!(
                "a register of {} qubits where a single qubit is required (line {line}); \
                 index it, e.g. `q[0]`",
                many.len()
            ))),
        }
    }

    fn register(&self, name: &str, line: u32) -> Result<Register, FrontendError> {
        self.registers.get(name).copied().ok_or_else(|| {
            FrontendError::semantic(format!(
                "`{name}` is not an allocated qubit or register (line {line})"
            ))
        })
    }

    /// A gate angle: a bare `float` kernel parameter becomes a symbol;
    /// anything else must fold to a constant.
    fn angle(&self, arg: &Arg, line: u32) -> Result<Param, FrontendError> {
        let Arg::Expr(expr) = arg else {
            return Err(FrontendError::semantic(format!(
                "expected an angle, found a list (line {line})"
            )));
        };
        if let Expr::Path(path) = expr
            && let [name] = path.as_slice()
            && self.float_args.contains(name)
        {
            return Ok(Param::symbol(name.clone()));
        }
        Ok(Param::concrete(self.constant(expr, line)?))
    }

    /// Folds an expression to a number. Kernel parameters are refused here:
    /// arithmetic on a symbol has no QC-IR representation (the same rule the
    /// OpenQASM frontend applies to compound `input` expressions).
    fn constant(&self, expr: &Expr, line: u32) -> Result<f64, FrontendError> {
        Ok(match expr {
            Expr::Number(value) => *value,
            Expr::Path(path) => match path
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["math" | "numpy" | "np", "pi"] => PI,
                [name] if self.float_args.contains(*name) => {
                    return Err(FrontendError::unsupported(format!(
                        "arithmetic on kernel parameter `{name}` (line {line}); only a bare \
                         parameter is representable as a symbolic angle"
                    )));
                }
                _ => {
                    return Err(FrontendError::semantic(format!(
                        "`{}` is not a constant or a `float` kernel parameter (line {line})",
                        path.join(".")
                    )));
                }
            },
            Expr::Neg(inner) => -self.constant(inner, line)?,
            Expr::Binary { op, left, right } => {
                let (l, r) = (self.constant(left, line)?, self.constant(right, line)?);
                match op {
                    BinOp::Add => l + r,
                    BinOp::Sub => l - r,
                    BinOp::Mul => l * r,
                    BinOp::Div => l / r,
                }
            }
            Expr::Index { base, .. } => {
                return Err(FrontendError::unsupported(format!(
                    "`{}[...]` used as a number (line {line})",
                    base.join(".")
                )));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::frontend::cudaq::parse_cudaq;
    use crate::frontend::error::FrontendError;
    use crate::ir::{ClbitId, GateKind, Instruction, IrError, Param, QubitId};

    fn kernel(body: &str) -> String {
        let indented: Vec<String> = body.lines().map(|l| format!("    {l}")).collect();
        format!(
            "import cudaq\n\n@cudaq.kernel\ndef k():\n{}\n",
            indented.join("\n")
        )
    }

    fn gate(kind: GateKind, qubits: &[u32]) -> Instruction {
        Instruction::Gate {
            kind,
            qubits: qubits.iter().copied().map(QubitId).collect(),
        }
    }

    fn measure(q: u32) -> Instruction {
        Instruction::Measure {
            qubit: QubitId(q),
            target: ClbitId(q),
        }
    }

    #[test]
    fn every_mz_call_gets_its_own_measurement() {
        // The regression this frontend exists to fix: three per-qubit `mz`
        // calls must yield three measurements, not the last one.
        let c = parse_cudaq(&kernel(
            "q = cudaq.qvector(3)\nh(q[0])\nx.ctrl(q[0], q[1])\nx.ctrl(q[1], q[2])\nmz(q[0])\nmz(q[1])\nmz(q[2])",
        ))
        .unwrap();
        assert_eq!(c.num_clbits(), 3);
        assert_eq!(&c.instructions()[3..], [measure(0), measure(1), measure(2)]);
    }

    #[test]
    fn mz_on_a_register_measures_every_qubit() {
        let c = parse_cudaq(&kernel("q = cudaq.qvector(2)\nmz(q)")).unwrap();
        assert_eq!(c.instructions(), [measure(0), measure(1)]);
    }

    #[test]
    fn a_single_qubit_gate_broadcasts_over_a_register() {
        let c = parse_cudaq(&kernel("q = cudaq.qvector(3)\nh(q)")).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c.num_clbits(), 0);
    }

    #[test]
    fn registers_share_one_flat_qubit_space() {
        let c = parse_cudaq(&kernel(
            "a = cudaq.qvector(2)\nb = cudaq.qubit()\nx.ctrl(a[1], b)",
        ))
        .unwrap();
        assert_eq!(c.num_qubits(), 3);
        assert_eq!(c.instructions(), [gate(GateKind::Cx, &[1, 2])]);
    }

    #[test]
    fn controlled_forms_map_to_registered_gates() {
        let c = parse_cudaq(&kernel(
            "q = cudaq.qvector(3)\nx.ctrl([q[0], q[1]], q[2])\ny.ctrl(q[0], q[1])\nz.ctrl(q[1], q[2])",
        ))
        .unwrap();
        assert_eq!(
            c.instructions(),
            [
                gate(GateKind::Ccx, &[0, 1, 2]),
                gate(GateKind::Cy, &[0, 1]),
                gate(GateKind::Cz, &[1, 2]),
            ]
        );
    }

    #[test]
    fn adjoints_map_to_daggers_and_negated_angles() {
        let c = parse_cudaq(&kernel(
            "q = cudaq.qubit()\ns.adj(q)\nt.adj(q)\nh.adj(q)\nrz.adj(0.25, q)",
        ))
        .unwrap();
        assert_eq!(
            c.instructions(),
            [
                gate(GateKind::Sdg, &[0]),
                gate(GateKind::Tdg, &[0]),
                gate(GateKind::H, &[0]),
                gate(GateKind::Rz(Param::concrete(-0.25)), &[0]),
            ]
        );
    }

    #[test]
    fn r1_and_u3_resolve_through_the_shared_gate_table() {
        let c = parse_cudaq(&kernel(
            "q = cudaq.qubit()\nr1(math.pi / 2, q)\nu3(0.1, 0.2, 0.3, q)",
        ))
        .unwrap();
        assert_eq!(
            c.instructions()[0],
            gate(
                GateKind::P(Param::concrete(std::f64::consts::FRAC_PI_2)),
                &[0]
            )
        );
        assert!(matches!(
            c.instructions()[1],
            Instruction::Gate {
                kind: GateKind::U { .. },
                ..
            }
        ));
    }

    #[test]
    fn a_float_parameter_becomes_a_symbolic_angle() {
        let c = parse_cudaq(
            "@cudaq.kernel\ndef ansatz(theta: float):\n    q = cudaq.qubit()\n    ry(theta, q)\n",
        )
        .unwrap();
        assert_eq!(c.parameters(), vec!["theta".to_string()]);
    }

    #[test]
    fn arithmetic_on_a_parameter_is_refused() {
        let error = parse_cudaq(
            "@cudaq.kernel\ndef k(theta: float):\n    q = cudaq.qubit()\n    ry(theta / 2, q)\n",
        )
        .unwrap_err();
        assert!(matches!(error, FrontendError::Unsupported(m) if m.contains("theta")));
    }

    #[test]
    fn non_float_parameters_are_refused() {
        let error = parse_cudaq("@cudaq.kernel\ndef k(n: int):\n    pass\n").unwrap_err();
        assert!(matches!(error, FrontendError::Unsupported(m) if m.contains("n: int")));
    }

    #[test]
    fn indexing_by_a_parameter_is_refused() {
        let error =
            parse_cudaq("@cudaq.kernel\ndef k(i: float):\n    q = cudaq.qvector(2)\n    h(q[i])\n")
                .unwrap_err();
        assert!(
            matches!(error, FrontendError::Unsupported(m) if m.contains("compile-time constant"))
        );
    }

    #[test]
    fn out_of_range_and_negative_indices_are_refused() {
        assert!(matches!(
            parse_cudaq(&kernel("q = cudaq.qvector(2)\nh(q[2])")),
            Err(FrontendError::Semantic(m)) if m.contains("out of range")
        ));
        assert!(matches!(
            parse_cudaq(&kernel("q = cudaq.qvector(2)\nh(q[-1])")),
            Err(FrontendError::Unsupported(_))
        ));
    }

    #[test]
    fn a_non_finite_angle_is_rejected_by_the_ir() {
        let error = parse_cudaq(&kernel("q = cudaq.qubit()\nrz(1 / 0, q)")).unwrap_err();
        assert!(
            matches!(error, FrontendError::Ir(IrError::NonFiniteAngle { .. })),
            "{error:?}"
        );
    }

    #[test]
    fn a_duplicate_operand_is_rejected_by_the_ir() {
        let error = parse_cudaq(&kernel("q = cudaq.qvector(2)\nx.ctrl(q[0], q[0])")).unwrap_err();
        assert!(matches!(error, FrontendError::Ir(_)), "{error:?}");
    }

    #[test]
    fn undocumented_and_unsupported_operations_are_refused_by_name() {
        for (line, needle) in [
            ("mx(q)", "mx"),
            ("sdg(q)", "s.adj"),
            ("reset(q)", "reset"),
            ("other_kernel(q)", "other_kernel"),
            ("h.ctrl(q, q)", "h.ctrl"),
            ("cudaq.mz(q)", "cudaq.mz"),
        ] {
            let error = parse_cudaq(&kernel(&format!("q = cudaq.qubit()\n{line}"))).unwrap_err();
            assert!(
                matches!(&error, FrontendError::Unsupported(m) if m.contains(needle)),
                "{line}: {error:?}"
            );
        }
    }

    #[test]
    fn an_unallocated_register_is_a_semantic_error() {
        assert!(matches!(
            parse_cudaq(&kernel("h(q[0])")),
            Err(FrontendError::Semantic(m)) if m.contains("not an allocated")
        ));
    }

    #[test]
    fn a_register_as_a_control_is_refused() {
        let error = parse_cudaq(&kernel(
            "q = cudaq.qvector(2)\nt = cudaq.qubit()\nx.ctrl(q, t)",
        ))
        .unwrap_err();
        assert!(matches!(error, FrontendError::Unsupported(m) if m.contains("single qubit")));
    }
}
