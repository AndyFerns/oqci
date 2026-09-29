//! QCO-IR → QIR emission.
//!
//! Emits **textual, LLVM-compatible QIR 1.0** (Quantum Intermediate
//! Representation), structured as the Base Profile specifies:
//! `qir-alliance/qir-spec`, tag `1.0`, commit
//! `e872897d107516d889718b5b6f5fee909dd64238`,
//! `specification/profiles/Base_Profile.md`.
//!
//! # What every module contains
//!
//! - an entry point that takes no arguments and returns `i64` (`ret i64 0`);
//! - an `entry` block calling `__quantum__rt__initialize`, and an `output`
//!   block recording each measured result under a non-null, unique label
//!   (a global string constant);
//! - `__quantum__qis__mz__body` declared with the `irreversible` attribute,
//!   and its result parameter marked `writeonly`;
//! - the entry-point attributes `entry_point`, `qir_profiles`,
//!   `output_labeling_schema` (with a value), `required_num_qubits` and
//!   `required_num_results`;
//! - the four module flags `qir_major_version` = 1, `qir_minor_version` = 0,
//!   `dynamic_qubit_management` = false, `dynamic_result_management` = false.
//!
//! Qubits and results are identified by constant integers cast to pointers
//! (static management). Pointers are opaque (`ptr`) by default, or typed
//! (`%Qubit*`, `%Result*`, as in the specification's LLVM 13 examples) on
//! request; see [`PointerStyle`].
//!
//! # The profile label
//!
//! [`emit_qir`] and [`emit_qir_with`] always label a module
//! `"qir_profiles"="custom"`: this module cannot know whether every gate in the
//! circuit belongs to the instruction set a Base Profile consumer expects, so
//! it never claims it. The `base_profile` label is written only by the
//! compiler orchestrator, after checking the circuit against the `qir-base`
//! target profile (`docs/qir_lowering.md`). A Base-labeled module additionally
//! splits the program into the Base Profile's `body` (unitary operations) and
//! `measurements` (irreversible operations) blocks; a custom module keeps every
//! operation, including measurement and reset, in one `body` block in
//! deterministic topological order.
//!
//! # Other caveats
//!
//! See `docs/qir_lowering.md` §3: gates outside a consumer's instruction set
//! are emitted as declared `__quantum__qis__*` externs and never decomposed
//! here; an `Opaque` name is kept verbatim, quoted when LLVM's unquoted
//! identifier syntax cannot express it.
//!
//! # Float encoding
//!
//! Angle parameters are emitted as LLVM hexadecimal `double` literals
//! (`0x` + the 16 IEEE-754 bits), which LLVM always accepts exactly — avoiding
//! any lossy decimal-formatting pitfalls.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::ir::error::IrError;
use crate::ir::qc::Instruction;
use crate::ir::qco::QcoCircuit;
use crate::ir::types::{ClbitId, GateKind, QubitId};

/// The value of the `"output_labeling_schema"` entry-point attribute: result
/// labels are the classical bit's name, `c<index>`.
pub const OUTPUT_LABELING_SCHEMA: &str = "oqci_clbit";

/// How qubit, result and label pointers are spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerStyle {
    /// LLVM opaque pointers, `ptr`: what LLVM 17 and later print, and what
    /// PyQIR 0.12.5 emits.
    #[default]
    Opaque,
    /// Typed pointers, `%Qubit*` / `%Result*` / `i8*`, as in the QIR 1.0
    /// specification's LLVM 13 examples. Older LLVM releases (LLVM 14, for
    /// example) default to this spelling.
    Typed,
}

impl PointerStyle {
    /// A stable identifier, used on the command line and by the Python SDK.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            PointerStyle::Opaque => "opaque",
            PointerStyle::Typed => "typed",
        }
    }

    /// Resolves an [`id`](PointerStyle::id).
    #[must_use]
    pub fn from_id(id: &str) -> Option<PointerStyle> {
        [PointerStyle::Opaque, PointerStyle::Typed]
            .into_iter()
            .find(|style| style.id() == id)
    }
}

/// The value of a module's `"qir_profiles"` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QirProfile {
    /// `"base_profile"`: the module follows the QIR 1.0 Base Profile.
    Base,
    /// `"custom"`: no profile is claimed.
    Custom,
}

impl QirProfile {
    /// The attribute value written into the module.
    #[must_use]
    pub fn attribute_value(self) -> &'static str {
        match self {
            QirProfile::Base => "base_profile",
            QirProfile::Custom => "custom",
        }
    }
}

/// Options for [`emit_qir_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QirOptions {
    /// How pointers are spelled.
    pub pointers: PointerStyle,
}

/// Emits a QCO-IR circuit as a QIR module labeled `custom`, with opaque
/// pointers.
///
/// # Errors
///
/// As [`emit_qir_with`].
pub fn emit_qir(dag: &QcoCircuit) -> Result<String, IrError> {
    emit_qir_with(dag, &QirOptions::default())
}

/// Emits a QCO-IR circuit as a QIR module labeled `custom`.
///
/// # Errors
///
/// - Propagates [`IrError::CyclicGraph`] from the topological traversal (which
///   cannot occur for a graph built from valid QC-IR).
/// - Returns [`IrError::UnboundParameter`] if any gate still carries a
///   symbolic parameter. QIR requires concrete `double` arguments, so the
///   circuit must pass through [`crate::ir::bind_parameters`] first (Stage F
///   makes binding an explicit step; lowering never invents a value).
pub fn emit_qir_with(dag: &QcoCircuit, options: &QirOptions) -> Result<String, IrError> {
    emit(dag, options, QirProfile::Custom)
}

/// Emits a QCO-IR circuit as a QIR module labeled `base_profile`, with the
/// Base Profile's separate `body` and `measurements` blocks.
///
/// Crate-private on purpose: the label is a conformance claim, and only the
/// orchestrator can make it, after checking the circuit's gates against the
/// `qir-base` target profile. This function checks what the IR layer can see
/// on its own — no reset, and no operation on a qubit after it has been
/// measured — so that even a wrong caller cannot produce a Base-labeled module
/// that breaks those rules.
///
/// # Errors
///
/// [`IrError::NotBaseProfile`] for a reset or an operation after a
/// measurement on the same qubit, and otherwise as [`emit_qir_with`].
pub(crate) fn emit_qir_base(dag: &QcoCircuit, options: &QirOptions) -> Result<String, IrError> {
    emit(dag, options, QirProfile::Base)
}

/// The pieces of a module that differ between pointer styles.
struct Spelling {
    qubit_ty: &'static str,
    result_ty: &'static str,
    label_ty: &'static str,
}

impl Spelling {
    fn of(style: PointerStyle) -> Spelling {
        match style {
            PointerStyle::Opaque => Spelling {
                qubit_ty: "ptr",
                result_ty: "ptr",
                label_ty: "ptr",
            },
            PointerStyle::Typed => Spelling {
                qubit_ty: "%Qubit*",
                result_ty: "%Result*",
                label_ty: "i8*",
            },
        }
    }

    fn qubit(&self, q: QubitId) -> String {
        format!("{0} inttoptr (i64 {1} to {0})", self.qubit_ty, q.index())
    }

    fn result(&self, c: ClbitId) -> String {
        format!("{0} inttoptr (i64 {1} to {0})", self.result_ty, c.index())
    }

    fn written_result(&self, c: ClbitId) -> String {
        format!(
            "{0} writeonly inttoptr (i64 {1} to {0})",
            self.result_ty,
            c.index()
        )
    }
}

fn emit(dag: &QcoCircuit, options: &QirOptions, profile: QirProfile) -> Result<String, IrError> {
    let ops = dag.topological_ops()?;
    let spell = Spelling::of(options.pointers);

    if profile == QirProfile::Base {
        check_base_structure(ops.iter().map(|(_, inst)| *inst))?;
    }

    let mut declarations: BTreeMap<String, String> = BTreeMap::new();
    let mut body = String::new();
    let mut measurements = String::new();
    let mut measured: BTreeSet<u32> = BTreeSet::new();
    let mut max_qubit: Option<u32> = None;

    for (_, inst) in &ops {
        for q in inst.qubits() {
            max_qubit = Some(max_qubit.map_or(q.index(), |m| m.max(q.index())));
        }
        let into_measurements =
            profile == QirProfile::Base && matches!(inst, Instruction::Measure { .. });
        let block = if into_measurements {
            &mut measurements
        } else {
            &mut body
        };
        emit_instruction(inst, &spell, block, &mut declarations, &mut measured)?;
    }

    register_decl(
        &mut declarations,
        "__quantum__rt__initialize",
        &[spell.label_ty.to_string()],
        None,
    );
    if !measured.is_empty() {
        register_decl(
            &mut declarations,
            "__quantum__rt__result_record_output",
            &[spell.result_ty.to_string(), spell.label_ty.to_string()],
            None,
        );
    }

    let required_qubits = max_qubit.map_or(0, |m| u64::from(m) + 1);
    let required_results = measured.last().map_or(0, |&m| u64::from(m) + 1);

    let entry = sanitize_symbol(dag.name());
    let mut out = String::new();
    out.push_str(&format!("; QIR module for circuit `{}`\n", dag.name()));
    out.push_str(&format!(
        "; Generated by OQCI: QIR 1.0 (qir-spec tag 1.0, commit e872897), \
         profile {}, {} pointers.\n",
        profile.attribute_value(),
        options.pointers.id()
    ));
    out.push_str(&format!(
        "source_filename = \"{}\"\n\n",
        escape_string(dag.name())
    ));
    if options.pointers == PointerStyle::Typed {
        out.push_str("%Qubit = type opaque\n");
        out.push_str("%Result = type opaque\n\n");
    }

    // Output labels: one unique, null-terminated global per recorded result,
    // numbered in recording order (LLVM requires unnamed globals in order).
    for (label, clbit) in measured.iter().enumerate() {
        let text = format!("c{clbit}");
        out.push_str(&format!(
            "@{label} = internal constant [{} x i8] c\"{text}\\00\"\n",
            text.len() + 1
        ));
    }
    if !measured.is_empty() {
        out.push('\n');
    }

    out.push_str(&format!("define i64 @{entry}() #0 {{\n"));
    out.push_str("entry:\n");
    out.push_str(&format!(
        "  call void @__quantum__rt__initialize({} null)\n",
        spell.label_ty
    ));
    out.push_str("  br label %body\n\n");
    out.push_str("body:\n");
    out.push_str(&body);
    if profile == QirProfile::Base {
        out.push_str("  br label %measurements\n\n");
        out.push_str("measurements:\n");
        out.push_str(&measurements);
    }
    out.push_str("  br label %output\n\n");
    out.push_str("output:\n");
    for (label, clbit) in measured.iter().enumerate() {
        let label_ptr = match options.pointers {
            PointerStyle::Opaque => format!("ptr @{label}"),
            PointerStyle::Typed => {
                let len = format!("c{clbit}").len() + 1;
                format!(
                    "i8* getelementptr inbounds ([{len} x i8], [{len} x i8]* @{label}, i32 0, i32 0)"
                )
            }
        };
        out.push_str(&format!(
            "  call void @__quantum__rt__result_record_output({}, {label_ptr})\n",
            spell.result(ClbitId(*clbit))
        ));
    }
    out.push_str("  ret i64 0\n");
    out.push_str("}\n\n");

    for decl in declarations.values() {
        out.push_str(decl);
        out.push('\n');
    }
    out.push('\n');

    out.push_str(&format!(
        "attributes #0 = {{ \"entry_point\" \"output_labeling_schema\"=\"{OUTPUT_LABELING_SCHEMA}\" \
         \"qir_profiles\"=\"{}\" \"required_num_qubits\"=\"{required_qubits}\" \
         \"required_num_results\"=\"{required_results}\" }}\n",
        profile.attribute_value()
    ));
    out.push_str("attributes #1 = { \"irreversible\" }\n\n");

    out.push_str("!llvm.module.flags = !{!0, !1, !2, !3}\n\n");
    out.push_str("!0 = !{i32 1, !\"qir_major_version\", i32 1}\n");
    out.push_str("!1 = !{i32 7, !\"qir_minor_version\", i32 0}\n");
    out.push_str("!2 = !{i32 1, !\"dynamic_qubit_management\", i1 false}\n");
    out.push_str("!3 = !{i32 1, !\"dynamic_result_management\", i1 false}\n");

    Ok(out)
}

/// The Base Profile rules visible from the instruction stream alone: no
/// reset, and no operation on a qubit once it has been measured
/// (`Base_Profile.md`, "Qubit and Result Usage").
fn check_base_structure<'a>(ops: impl Iterator<Item = &'a Instruction>) -> Result<(), IrError> {
    let mut measured: BTreeSet<u32> = BTreeSet::new();
    for inst in ops {
        if let Instruction::Reset { qubit } = inst {
            return Err(IrError::NotBaseProfile {
                reason: format!(
                    "reset on qubit {} (the Base Profile has no reset)",
                    qubit.index()
                ),
            });
        }
        for q in inst.qubits() {
            if measured.contains(&q.index()) {
                return Err(IrError::NotBaseProfile {
                    reason: format!(
                        "qubit {} is used after it was measured (measurements must be last)",
                        q.index()
                    ),
                });
            }
        }
        if let Instruction::Measure { qubit, .. } = inst {
            measured.insert(qubit.index());
        }
    }
    Ok(())
}

fn emit_instruction(
    inst: &Instruction,
    spell: &Spelling,
    block: &mut String,
    declarations: &mut BTreeMap<String, String>,
    measured: &mut BTreeSet<u32>,
) -> Result<(), IrError> {
    match inst {
        Instruction::Gate { kind, qubits } => {
            let fname = intrinsic_name(kind);
            // Argument list: parameters (doubles) first, then qubit operands.
            let mut arg_tys: Vec<String> = Vec::new();
            let mut arg_vals: Vec<String> = Vec::new();
            for p in kind.params() {
                let angle = p.as_concrete().ok_or_else(|| IrError::UnboundParameter {
                    gate: kind.mnemonic().to_string(),
                    symbol: p.as_symbol().unwrap_or_default().to_string(),
                })?;
                arg_tys.push("double".to_string());
                arg_vals.push(format!("double {}", llvm_double(angle.radians())));
            }
            for q in qubits {
                arg_tys.push(spell.qubit_ty.to_string());
                arg_vals.push(spell.qubit(*q));
            }
            register_decl(declarations, &fname, &arg_tys, None);
            block.push_str(&format!(
                "  call void {}({})\n",
                llvm_global(&fname),
                arg_vals.join(", ")
            ));
        }
        Instruction::Measure { qubit, target } => {
            let fname = "__quantum__qis__mz__body";
            register_decl(
                declarations,
                fname,
                &[
                    spell.qubit_ty.to_string(),
                    format!("{} writeonly", spell.result_ty),
                ],
                Some("#1"),
            );
            block.push_str(&format!(
                "  call void @{fname}({}, {})\n",
                spell.qubit(*qubit),
                spell.written_result(*target)
            ));
            measured.insert(target.index());
        }
        Instruction::Reset { qubit } => {
            let fname = "__quantum__qis__reset__body";
            register_decl(declarations, fname, &[spell.qubit_ty.to_string()], None);
            block.push_str(&format!("  call void @{fname}({})\n", spell.qubit(*qubit)));
        }
    }
    Ok(())
}

/// Returns the fully-qualified QIR intrinsic function name for a gate.
fn intrinsic_name(kind: &GateKind) -> String {
    match kind {
        // Irregular names, as PyQIR 0.12.5 spells them.
        GateKind::Cx => "__quantum__qis__cnot__body".to_string(),
        GateKind::Sdg => "__quantum__qis__s__adj".to_string(),
        GateKind::Tdg => "__quantum__qis__t__adj".to_string(),
        // Everything else uses `__quantum__qis__<mnemonic>__body`. For gates
        // outside PyQIR's instruction set (id, sx, sxdg, p, u, cy, opaque) a
        // consumer must supply the intrinsic; see docs/qir_lowering.md.
        other => format!("__quantum__qis__{}__body", other.mnemonic()),
    }
}

fn register_decl(
    declarations: &mut BTreeMap<String, String>,
    fname: &str,
    arg_tys: &[String],
    attributes: Option<&str>,
) {
    declarations.entry(fname.to_string()).or_insert_with(|| {
        let suffix = attributes.map(|a| format!(" {a}")).unwrap_or_default();
        format!(
            "declare void {}({}){suffix}",
            llvm_global(fname),
            arg_tys.join(", ")
        )
    });
}

/// Spells a global symbol the way LLVM will parse it back unchanged.
///
/// An unquoted global identifier may only use `[-a-zA-Z$._0-9]` and must not
/// start with a digit. Every intrinsic name OQCI generates for a registered
/// gate satisfies that, but a [`GateKind::Opaque`] name is whatever the
/// frontend was given — a Qiskit custom gate may be called `my gate` — and I5
/// only requires it to be non-empty. Such a name is emitted as a quoted
/// identifier, `@"..."`, with `"`, `\` and non-printable bytes written as
/// LLVM `\XX` hex escapes. The name is kept exactly as written: nothing is
/// mangled, so two distinct gates can never collide on one symbol.
fn llvm_global(name: &str) -> String {
    let plain = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '$' | '.' | '_'));
    if plain {
        return format!("@{name}");
    }
    format!("@\"{}\"", escape_string(name))
}

/// Escapes a string for use inside LLVM double quotes: `"`, `\` and
/// non-printable bytes become `\XX` hex escapes.
fn escape_string(text: &str) -> String {
    let mut escaped = String::new();
    for byte in text.bytes() {
        if byte == b'"' || byte == b'\\' || !(0x20..0x7f).contains(&byte) {
            escaped.push_str(&format!("\\{byte:02X}"));
        } else {
            escaped.push(char::from(byte));
        }
    }
    escaped
}

/// Formats an `f64` as an LLVM hexadecimal `double` literal (exact).
fn llvm_double(x: f64) -> String {
    format!("0x{:016X}", x.to_bits())
}

/// Reduces a circuit name to a valid LLVM global identifier, defaulting to
/// `main` if nothing usable remains.
fn sanitize_symbol(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("main_{s}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::qc::CircuitBuilder;
    use crate::ir::qc_to_qco;

    fn dag_of(build: impl FnOnce(&mut CircuitBuilder)) -> QcoCircuit {
        let mut b = CircuitBuilder::new("test");
        build(&mut b);
        qc_to_qco(&b.build().unwrap()).unwrap()
    }

    fn qir_of(build: impl FnOnce(&mut CircuitBuilder)) -> String {
        emit_qir(&dag_of(build)).unwrap()
    }

    fn bell(b: &mut CircuitBuilder) {
        let q = b.alloc_qubits(2);
        let c = b.alloc_clbits(2);
        b.h(q[0])
            .cx(q[0], q[1])
            .measure(q[0], c[0])
            .measure(q[1], c[1]);
    }

    #[test]
    fn every_module_has_the_qir_1_0_skeleton() {
        let qir = qir_of(|_| {});
        assert!(qir.contains("define i64 @test() #0 {"));
        assert!(qir.contains("call void @__quantum__rt__initialize(ptr null)"));
        assert!(qir.contains("ret i64 0"));
        assert!(qir.contains("\"entry_point\""));
        assert!(qir.contains("\"output_labeling_schema\"=\"oqci_clbit\""));
        assert!(qir.contains("\"qir_profiles\"=\"custom\""));
        for flag in [
            "!0 = !{i32 1, !\"qir_major_version\", i32 1}",
            "!1 = !{i32 7, !\"qir_minor_version\", i32 0}",
            "!2 = !{i32 1, !\"dynamic_qubit_management\", i1 false}",
            "!3 = !{i32 1, !\"dynamic_result_management\", i1 false}",
        ] {
            assert!(qir.contains(flag), "missing module flag: {flag}");
        }
    }

    #[test]
    fn the_public_emitters_never_claim_the_base_profile() {
        let qir = qir_of(bell);
        assert!(!qir.contains("base_profile"));
        assert!(!qir.contains("measurements:"));
    }

    #[test]
    fn measurement_is_irreversible_and_writes_a_writeonly_result() {
        let qir = qir_of(bell);
        assert!(qir.contains("declare void @__quantum__qis__mz__body(ptr, ptr writeonly) #1"));
        assert!(qir.contains("attributes #1 = { \"irreversible\" }"));
        assert!(qir.contains(
            "call void @__quantum__qis__mz__body(ptr inttoptr (i64 1 to ptr), \
             ptr writeonly inttoptr (i64 1 to ptr))"
        ));
    }

    #[test]
    fn results_are_recorded_under_unique_non_null_labels() {
        let qir = qir_of(bell);
        assert!(qir.contains("@0 = internal constant [3 x i8] c\"c0\\00\""));
        assert!(qir.contains("@1 = internal constant [3 x i8] c\"c1\\00\""));
        assert!(qir.contains(
            "call void @__quantum__rt__result_record_output(ptr inttoptr (i64 0 to ptr), ptr @0)"
        ));
        assert!(!qir.contains("null)\n  ret"));
    }

    #[test]
    fn typed_pointers_spell_every_pointer_the_llvm_13_way() {
        let qir = emit_qir_with(
            &dag_of(bell),
            &QirOptions {
                pointers: PointerStyle::Typed,
            },
        )
        .unwrap();
        assert!(qir.contains("%Qubit = type opaque"));
        assert!(qir.contains("call void @__quantum__rt__initialize(i8* null)"));
        assert!(
            qir.contains("declare void @__quantum__qis__mz__body(%Qubit*, %Result* writeonly) #1")
        );
        assert!(qir.contains("i8* getelementptr inbounds ([3 x i8], [3 x i8]* @0, i32 0, i32 0)"));
        // No opaque `ptr` type anywhere (`inttoptr` is an instruction, not a type).
        assert!(!qir.contains(" ptr") && !qir.contains("(ptr"));
    }

    #[test]
    fn the_base_form_separates_unitaries_from_measurements() {
        // Measure q0 before touching q1: legal (q0 is never used again), and
        // the Base form must still put every unitary before every measurement.
        let dag = dag_of(|b| {
            let q = b.alloc_qubits(2);
            let c = b.alloc_clbits(1);
            b.h(q[0]).measure(q[0], c[0]).x(q[1]);
        });
        let qir = emit_qir_base(&dag, &QirOptions::default()).unwrap();
        assert!(qir.contains("\"qir_profiles\"=\"base_profile\""));
        let body = qir.find("body:").unwrap();
        let measurements = qir.find("measurements:").unwrap();
        let x = qir.find("@__quantum__qis__x__body(ptr").unwrap();
        let mz = qir.find("call void @__quantum__qis__mz__body").unwrap();
        assert!(body < x && x < measurements && measurements < mz);
    }

    #[test]
    fn the_base_form_refuses_what_the_base_profile_forbids() {
        let reset = dag_of(|b| {
            let q0 = b.alloc_qubit();
            b.reset(q0);
        });
        assert!(matches!(
            emit_qir_base(&reset, &QirOptions::default()),
            Err(IrError::NotBaseProfile { .. })
        ));
        let reuse = dag_of(|b| {
            let q0 = b.alloc_qubit();
            let c0 = b.alloc_clbit();
            b.measure(q0, c0).x(q0);
        });
        assert!(matches!(
            emit_qir_base(&reuse, &QirOptions::default()),
            Err(IrError::NotBaseProfile { .. })
        ));
    }

    #[test]
    fn required_counts_are_what_the_entry_point_uses() {
        // Four qubits declared, highest used is 2; three clbits declared,
        // highest measured is 1.
        let qir = qir_of(|b| {
            let q = b.alloc_qubits(4);
            let c = b.alloc_clbits(3);
            b.h(q[2]).measure(q[0], c[1]);
        });
        assert!(qir.contains("\"required_num_qubits\"=\"3\""));
        assert!(qir.contains("\"required_num_results\"=\"2\""));
    }

    #[test]
    fn rotation_emits_hex_double() {
        let qir = qir_of(|b| {
            let q0 = b.alloc_qubit();
            b.rz(std::f64::consts::PI, q0);
        });
        // PI encoded as its exact IEEE-754 hex.
        assert!(qir.contains(&format!("double 0x{:016X}", std::f64::consts::PI.to_bits())));
        assert!(qir.contains("@__quantum__qis__rz__body"));
    }

    #[test]
    fn declarations_are_deduplicated() {
        let qir = qir_of(|b| {
            let q0 = b.alloc_qubit();
            b.x(q0).x(q0).x(q0);
        });
        assert_eq!(
            qir.matches("declare void @__quantum__qis__x__body").count(),
            1
        );
    }

    #[test]
    fn unbound_parameter_is_rejected() {
        use crate::ir::param::Param;

        let mut b = CircuitBuilder::new("ansatz");
        let q0 = b.alloc_qubit();
        b.rz(Param::symbol("theta"), q0);
        let dag = qc_to_qco(&b.build().unwrap()).unwrap();

        assert_eq!(
            emit_qir(&dag),
            Err(IrError::UnboundParameter {
                gate: "rz".into(),
                symbol: "theta".into()
            })
        );
    }

    #[test]
    fn binding_then_emitting_matches_a_concrete_circuit() {
        use std::collections::HashMap;

        use crate::ir::bind::bind_parameters;
        use crate::ir::param::Param;

        let mut b = CircuitBuilder::new("test");
        let q0 = b.alloc_qubit();
        b.rz(Param::symbol("theta"), q0);
        let bound = bind_parameters(
            &b.build().unwrap(),
            &HashMap::from([("theta".into(), 0.75)]),
        )
        .unwrap();
        let symbolic_qir = emit_qir(&qc_to_qco(&bound).unwrap()).unwrap();

        assert_eq!(
            symbolic_qir,
            qir_of(|b| {
                let q0 = b.alloc_qubit();
                b.rz(0.75, q0);
            })
        );
    }

    #[test]
    fn reset_emits_reset_intrinsic_in_a_custom_module() {
        let qir = qir_of(|b| {
            let q0 = b.alloc_qubit();
            b.reset(q0);
        });
        assert!(qir.contains("@__quantum__qis__reset__body"));
    }

    #[test]
    fn pointer_style_ids_round_trip() {
        for style in [PointerStyle::Opaque, PointerStyle::Typed] {
            assert_eq!(PointerStyle::from_id(style.id()), Some(style));
        }
        assert_eq!(PointerStyle::from_id("fancy"), None);
    }
}
