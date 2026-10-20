//! A crude, dependency-free mutation fuzzer for toolchains libFuzzer cannot
//! use (stable Rust, Windows GNU).
//!
//! It mutates the seed corpus — byte flips, insertions of syntax characters,
//! deletions, duplications, splices — and feeds every mutant to every target,
//! catching panics. It is no substitute for coverage-guided fuzzing (the
//! `fuzz` CI job), but it finds shallow crashes anywhere. Ignored by default:
//!
//! ```bash
//! OQCI_MUTATION_SCALE=10 cargo test --no-default-features --release --test mutate -- --ignored --nocapture
//! ```

use std::fs;
use std::panic;

/// xorshift64*: a fixed seed, so a failure reproduces exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const SYNTAX: &[u8] = b"()[]{};,.:+-*/=<>@#\n\t 0123456789qcxhzrystupmei_\"'\\";

fn mutate(rng: &mut Rng, seeds: &[Vec<u8>], base: &[u8]) -> Vec<u8> {
    let mut out = base.to_vec();
    for _ in 0..1 + rng.below(8) {
        let len = out.len();
        match rng.below(6) {
            0 if len > 0 => {
                let at = rng.below(len);
                out[at] ^= 1 << rng.below(8);
            }
            1 => {
                let at = rng.below(len + 1);
                out.insert(at, SYNTAX[rng.below(SYNTAX.len())]);
            }
            2 if len > 0 => {
                let at = rng.below(len);
                let n = 1 + rng.below(16.min(len - at));
                out.drain(at..at + n);
            }
            3 if len > 0 => {
                let at = rng.below(len);
                let n = 1 + rng.below(32.min(len - at));
                let chunk: Vec<u8> = out[at..at + n].to_vec();
                let to = rng.below(out.len() + 1);
                for (i, b) in chunk.into_iter().enumerate() {
                    out.insert(to + i, b);
                }
            }
            4 => {
                let other = &seeds[rng.below(seeds.len())];
                if !other.is_empty() {
                    let at = rng.below(other.len());
                    let to = rng.below(out.len() + 1);
                    let end = (at + 1 + rng.below(64)).min(other.len());
                    out.splice(to..to, other[at..end].iter().copied());
                }
            }
            _ => {
                let at = rng.below(len + 1);
                let digits = format!("{}", rng.next());
                out.splice(at..at, digits.bytes());
            }
        }
    }
    out
}

fn seeds() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for dir in fs::read_dir("corpus").unwrap() {
        for file in fs::read_dir(dir.unwrap().path()).unwrap() {
            out.push(fs::read(file.unwrap().path()).unwrap());
        }
    }
    out.push(
        include_str!("../../tests/fixtures/heron_style_synthetic.json")
            .as_bytes()
            .to_vec(),
    );
    out.push(
        include_str!("../../examples/bell_cudaq.py")
            .as_bytes()
            .to_vec(),
    );
    out
}

fn run(name: &str, target: fn(&[u8]), iterations: usize, seed: u64) {
    // OQCI_MUTATION_SCALE multiplies every budget (default 1).
    let scale: usize = std::env::var("OQCI_MUTATION_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let iterations = iterations * scale;
    // OQCI_MUTATION_ONLY=<target> runs one target; OQCI_MUTATION_TRACE=1 writes
    // each input to fuzz/last-input before running it, so an input that aborts
    // the process (which no panic hook can catch) is still on disk.
    if std::env::var("OQCI_MUTATION_ONLY").is_ok_and(|only| only != name) {
        return;
    }
    let trace = std::env::var("OQCI_MUTATION_TRACE").is_ok_and(|v| v == "1");
    let seeds = seeds();
    let mut rng = Rng(seed);
    panic::set_hook(Box::new(|_| {}));
    for i in 0..iterations {
        let base = &seeds[rng.below(seeds.len())];
        let input = mutate(&mut rng, &seeds, base);
        if trace {
            fs::write("last-input", &input).unwrap();
        }
        if panic::catch_unwind(|| target(&input)).is_err() {
            let _ = panic::take_hook();
            let path = format!("crash-{name}-{i}");
            fs::write(&path, &input).unwrap();
            panic!("{name} panicked on mutant {i}; input written to fuzz/{path}");
        }
    }
    let _ = panic::take_hook();
    println!("{name}: {iterations} mutants, no panic");
}

#[test]
#[ignore = "slow; run on demand"]
fn mutants_never_panic() {
    run("openqasm", oqci_fuzz::openqasm, 200_000, 0x0C1_5EED_0001);
    run("cudaq", oqci_fuzz::cudaq, 200_000, 0x0C1_5EED_0002);
    run(
        "target_json",
        oqci_fuzz::target_json,
        20_000,
        0x0C1_5EED_0003,
    );
    run(
        "qiskit_handoff",
        oqci_fuzz::qiskit_handoff,
        200_000,
        0x0C1_5EED_0004,
    );
    run(
        "compile_verified",
        oqci_fuzz::compile_verified,
        20_000,
        0x0C1_5EED_0005,
    );
}
