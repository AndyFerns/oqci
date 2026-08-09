//! Thin demonstration binary: builds a Bell circuit in QC-IR, converts it to
//! QCO-IR, lowers it to QIR, and prints the result. This is a smoke demo of the
//! Phase 0 pipeline, not a CLI — frontends and backends are out of scope.

use oqci::ir::{CircuitBuilder, emit_qir, qc_to_qco};

fn main() -> Result<(), oqci::ir::IrError> {
    let mut b = CircuitBuilder::new("bell");
    let q0 = b.alloc_qubit();
    let q1 = b.alloc_qubit();
    let c0 = b.alloc_clbit();
    let c1 = b.alloc_clbit();
    b.h(q0).cx(q0, q1).measure(q0, c0).measure(q1, c1);

    let circuit = b.build()?;
    let dag = qc_to_qco(&circuit)?;
    let qir = emit_qir(&dag)?;

    println!(
        "=== QC-IR: {} ({} qubits) ===",
        circuit.name(),
        circuit.num_qubits()
    );
    for (i, inst) in circuit.instructions().iter().enumerate() {
        println!("  {i}: {inst:?}");
    }
    println!("\n=== QCO-IR: {} op nodes ===", dag.op_count());
    println!("\n=== QIR ===\n{qir}");
    Ok(())
}
