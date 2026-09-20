//! Backend-defined optimization cost.
//!
//! Stage E's principle, verbatim:
//!
//! > The compiler optimizer chooses among transformations; the target
//! > describes what is expensive.
//!
//! Which is why cost lives here rather than in [`crate::pass`]. Two-qubit
//! burden, depth, routing overhead and error rates matter in different
//! proportions on different hardware, so a universal weighted score baked into
//! the optimizer would be a hidden assumption wearing the costume of a
//! measurement (Stage E §2).
//!
//! # Components are never discarded
//!
//! [`Cost`] keeps every raw metric and exposes the scalar as an `Option`
//! alongside them, never instead of them. Stage E §7 is explicit that
//! reporting `OQCI cost = 123.4` alone would hide why the optimizer decided
//! what it decided, which is exactly the thing a benchmark needs to explain.
//!
//! # Weights are configuration, not literals
//!
//! Stage E §6 rejects a scalar like `0.6·depth + 0.3·CX + 0.1·gates` when the
//! coefficients are arbitrary — such a number "has no inherent scientific
//! legitimacy unless the experiment establishes why those coefficients are
//! appropriate". [`WeightedCostModel`] therefore takes its weights as explicit
//! fields and reports them through [`CostModel::configuration`], so any result
//! derived from a scalar carries the weights that produced it.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::Serialize;

use crate::analysis::analyze;
use crate::ir::{Circuit, Instruction, IrError};
use crate::target::profile::BasisProfile;

/// A structured cost breakdown.
///
/// Every field is a raw, independently-meaningful metric except
/// [`Cost::scalar_score`], which is derived and optional.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cost {
    /// All operations, including measurement and reset.
    pub total_gate_count: usize,
    /// Unitary gates on exactly one qubit.
    pub one_qubit_count: usize,
    /// Unitary gates on two or more qubits — the expensive ones on hardware.
    pub two_qubit_count: usize,
    /// ASAP scheduling depth.
    pub depth: usize,
    /// `Swap` operations present in the circuit.
    ///
    /// Routing does not exist yet, so today this only counts swaps the program
    /// itself contained. It is a real field rather than a placeholder because
    /// Stage E §4 requires routing overhead to be a retained component, and a
    /// routed circuit will populate it without this type changing.
    pub swap_count: usize,
    /// Operations that are in the target's basis set.
    pub native_gate_count: usize,
    /// Operations that are **not** in the target's basis set, and would need
    /// decomposition before execution.
    pub non_native_gate_count: usize,
    /// Estimated execution duration, when the backend can supply timing data.
    pub estimated_duration: Option<f64>,
    /// Estimated error contribution, when the backend can supply error data.
    pub estimated_error: Option<f64>,
    /// An optional derived score. Meaningful only alongside the
    /// [`CostModel::configuration`] that produced it.
    pub scalar_score: Option<f64>,
}

/// How a backend defines cost.
///
/// Stage E §9 requires a compiled result to be attributable to the cost model
/// that guided it, which is why identity, version and configuration are part
/// of the contract rather than an implementation detail.
pub trait CostModel: Send + Sync {
    /// Stable identifier, matching a profile's
    /// [`BasisProfile::cost_model_id`].
    fn id(&self) -> &str;

    /// Cost-model version, for experiment provenance.
    fn version(&self) -> &str;

    /// The configuration that produced this model's numbers — weights
    /// included. Recorded with results so a scalar score is never an
    /// unexplained figure (Stage E §6, §9).
    fn configuration(&self) -> BTreeMap<String, String>;

    /// Evaluates a circuit against a target.
    ///
    /// # Errors
    ///
    /// Propagates [`IrError`] from the analysis used to derive the components.
    fn evaluate(&self, circuit: &Circuit, profile: &BasisProfile) -> Result<Cost, IrError>;

    /// Orders two costs, cheapest first.
    fn compare(&self, a: &Cost, b: &Cost) -> Ordering;
}

/// A cost model with explicit, reportable weights.
///
/// The defaults encode nothing but an ordering preference that the components
/// make visible anyway: two-qubit operations are weighted above depth, and
/// depth above raw gate count. They are a starting point for experiments to
/// replace, **not** a claim that these coefficients are correct for any real
/// device — Stage E §5 explicitly declines to fix numeric weights in
/// architecture, and §12.3 of `final-deliverables-spec.md` forbids burying
/// them in optimization code.
#[derive(Debug, Clone, PartialEq)]
pub struct WeightedCostModel {
    id: String,
    version: String,
    /// Weight applied to [`Cost::two_qubit_count`].
    pub two_qubit_weight: f64,
    /// Weight applied to [`Cost::depth`].
    pub depth_weight: f64,
    /// Weight applied to [`Cost::total_gate_count`].
    pub gate_count_weight: f64,
    /// Weight applied to [`Cost::swap_count`].
    pub swap_weight: f64,
    /// Weight applied to [`Cost::non_native_gate_count`], standing in for the
    /// decomposition a non-native operation will eventually cost.
    pub non_native_weight: f64,
}

impl WeightedCostModel {
    /// A model with the documented default weights.
    #[must_use]
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        WeightedCostModel {
            id: id.into(),
            version: version.into(),
            two_qubit_weight: 10.0,
            depth_weight: 1.0,
            gate_count_weight: 1.0,
            swap_weight: 30.0,
            non_native_weight: 5.0,
        }
    }

    fn score(&self, cost: &Cost) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        {
            self.two_qubit_weight * cost.two_qubit_count as f64
                + self.depth_weight * cost.depth as f64
                + self.gate_count_weight * cost.total_gate_count as f64
                + self.swap_weight * cost.swap_count as f64
                + self.non_native_weight * cost.non_native_gate_count as f64
        }
    }
}

impl CostModel for WeightedCostModel {
    fn id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn configuration(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("two_qubit_weight".into(), self.two_qubit_weight.to_string()),
            ("depth_weight".into(), self.depth_weight.to_string()),
            (
                "gate_count_weight".into(),
                self.gate_count_weight.to_string(),
            ),
            ("swap_weight".into(), self.swap_weight.to_string()),
            (
                "non_native_weight".into(),
                self.non_native_weight.to_string(),
            ),
        ])
    }

    fn evaluate(&self, circuit: &Circuit, profile: &BasisProfile) -> Result<Cost, IrError> {
        // Components come from `analysis`, the single place any metric is
        // computed — recomputing them here would be a second source of truth.
        let report = analyze(circuit)?;

        let mut native = 0usize;
        let mut non_native = 0usize;
        let mut swaps = 0usize;
        for instruction in circuit.instructions() {
            let mnemonic = match instruction {
                Instruction::Gate { kind, .. } => {
                    if matches!(kind, crate::ir::GateKind::Swap) {
                        swaps += 1;
                    }
                    kind.mnemonic()
                }
                Instruction::Measure { .. } => "measure",
                Instruction::Reset { .. } => "reset",
            };
            if profile.supports_operation(mnemonic) {
                native += 1;
            } else {
                non_native += 1;
            }
        }

        let mut cost = Cost {
            total_gate_count: report.op_count,
            one_qubit_count: report.one_qubit_count,
            two_qubit_count: report.two_qubit_count,
            depth: report.depth,
            swap_count: swaps,
            native_gate_count: native,
            non_native_gate_count: non_native,
            // This model has no timing or error data. Reporting `None` rather
            // than a fabricated number is the point: §33 forbids inventing
            // calibration values the backend never supplied.
            estimated_duration: None,
            estimated_error: None,
            scalar_score: None,
        };
        cost.scalar_score = Some(self.score(&cost));
        Ok(cost)
    }

    fn compare(&self, a: &Cost, b: &Cost) -> Ordering {
        self.score(a)
            .partial_cmp(&self.score(b))
            .unwrap_or(Ordering::Equal)
    }
}

/// The cost model a profile's [`BasisProfile::cost_model_id`] refers to.
///
/// Stage E §9 requires a compiled result to be attributable to the cost model
/// that guided it. That only means something if the id actually selects a
/// model: naming one and then using another's weights would make the recorded
/// provenance false.
///
/// Returns `None` for an unknown id rather than silently substituting a
/// default, since a profile referring to a model OQCI does not have is a
/// configuration error, not something to paper over.
#[must_use]
pub fn resolve(cost_model_id: &str) -> Option<WeightedCostModel> {
    let mut model = WeightedCostModel::new(cost_model_id, "1");
    match cost_model_id {
        // Every component counts the same: nothing is "expensive" on a target
        // with no constraints, so this weights by sheer size only.
        "uniform" => {
            model.two_qubit_weight = 1.0;
            model.depth_weight = 1.0;
            model.gate_count_weight = 1.0;
            model.swap_weight = 1.0;
            model.non_native_weight = 1.0;
            Some(model)
        }
        // The documented NISQ-shaped defaults: two-qubit operations above
        // depth, depth above raw count, routing above both.
        "nisq-weighted" => Some(model),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CircuitBuilder, GateKind, QubitId};
    use crate::target::builtin;

    fn model() -> WeightedCostModel {
        WeightedCostModel::new("test-cost", "1")
    }

    fn circuit(build: impl FnOnce(&mut CircuitBuilder)) -> Circuit {
        let mut b = CircuitBuilder::new("c");
        b.alloc_qubits(3);
        build(&mut b);
        b.build().unwrap()
    }

    #[test]
    fn components_mirror_the_analysis_module() {
        let c = circuit(|b| {
            b.h(QubitId(0)).cx(QubitId(0), QubitId(1));
        });
        let cost = model().evaluate(&c, &builtin::ideal_simulator()).unwrap();

        assert_eq!(cost.total_gate_count, 2);
        assert_eq!(cost.one_qubit_count, 1);
        assert_eq!(cost.two_qubit_count, 1);
        assert_eq!(cost.depth, 2);
    }

    #[test]
    fn native_and_non_native_operations_are_split_by_the_profile() {
        // `t` is outside the linear-NISQ basis but inside the simulator's.
        let c = circuit(|b| {
            b.gate(GateKind::T, [QubitId(0)]);
        });

        let on_simulator = model().evaluate(&c, &builtin::ideal_simulator()).unwrap();
        assert_eq!(on_simulator.native_gate_count, 1);
        assert_eq!(on_simulator.non_native_gate_count, 0);

        let on_nisq = model().evaluate(&c, &builtin::linear_nisq(3)).unwrap();
        assert_eq!(on_nisq.native_gate_count, 0);
        assert_eq!(on_nisq.non_native_gate_count, 1);
    }

    #[test]
    fn swaps_are_counted_as_routing_overhead() {
        let c = circuit(|b| {
            b.swap(QubitId(0), QubitId(1));
        });
        let cost = model().evaluate(&c, &builtin::ideal_simulator()).unwrap();
        assert_eq!(cost.swap_count, 1);
    }

    #[test]
    fn unavailable_estimates_are_none_not_zero() {
        // A fabricated zero would read as "no error", which is a different and
        // false claim from "this backend supplied no error data".
        let c = circuit(|b| {
            b.h(QubitId(0));
        });
        let cost = model().evaluate(&c, &builtin::ideal_simulator()).unwrap();
        assert_eq!(cost.estimated_duration, None);
        assert_eq!(cost.estimated_error, None);
    }

    #[test]
    fn the_scalar_never_replaces_the_components() {
        let c = circuit(|b| {
            b.h(QubitId(0)).cx(QubitId(0), QubitId(1));
        });
        let cost = model().evaluate(&c, &builtin::ideal_simulator()).unwrap();
        assert!(cost.scalar_score.is_some());
        assert_eq!(cost.two_qubit_count, 1, "raw metrics survive alongside it");
    }

    #[test]
    fn the_scalar_follows_the_configured_weights() {
        let c = circuit(|b| {
            b.cx(QubitId(0), QubitId(1));
        });
        let profile = builtin::ideal_simulator();

        let mut cheap = model();
        cheap.two_qubit_weight = 1.0;
        let mut dear = model();
        dear.two_qubit_weight = 100.0;

        let cheap_score = cheap.evaluate(&c, &profile).unwrap().scalar_score.unwrap();
        let dear_score = dear.evaluate(&c, &profile).unwrap().scalar_score.unwrap();
        assert!(dear_score > cheap_score);
    }

    #[test]
    fn configuration_reports_every_weight() {
        let configuration = model().configuration();
        for key in [
            "two_qubit_weight",
            "depth_weight",
            "gate_count_weight",
            "swap_weight",
            "non_native_weight",
        ] {
            assert!(configuration.contains_key(key), "{key} must be reportable");
        }
    }

    #[test]
    fn comparison_orders_cheapest_first() {
        let profile = builtin::ideal_simulator();
        let model = model();

        let cheap = model
            .evaluate(
                &circuit(|b| {
                    b.h(QubitId(0));
                }),
                &profile,
            )
            .unwrap();
        let expensive = model
            .evaluate(
                &circuit(|b| {
                    b.cx(QubitId(0), QubitId(1)).cx(QubitId(1), QubitId(2));
                }),
                &profile,
            )
            .unwrap();

        assert_eq!(model.compare(&cheap, &expensive), Ordering::Less);
        assert_eq!(model.compare(&cheap, &cheap), Ordering::Equal);
    }

    #[test]
    fn every_builtin_profiles_cost_model_resolves() {
        // A profile naming a model nothing can resolve would make its
        // recorded provenance a lie.
        for profile in builtin::all() {
            assert!(
                resolve(profile.cost_model_id()).is_some(),
                "{} references unresolvable cost model `{}`",
                profile.id(),
                profile.cost_model_id()
            );
        }
    }

    #[test]
    fn resolution_returns_the_named_models_own_weights() {
        let uniform = resolve("uniform").unwrap();
        let nisq = resolve("nisq-weighted").unwrap();
        assert_eq!(uniform.id(), "uniform");
        assert_eq!(uniform.two_qubit_weight, 1.0);
        assert!(
            nisq.two_qubit_weight > uniform.two_qubit_weight,
            "the two models must actually differ, not just be named differently"
        );
    }

    #[test]
    fn an_unknown_cost_model_does_not_silently_default() {
        assert!(resolve("no-such-model").is_none());
    }

    #[test]
    fn an_empty_circuit_costs_nothing() {
        let cost = model()
            .evaluate(&circuit(|_| {}), &builtin::ideal_simulator())
            .unwrap();
        assert_eq!(cost.total_gate_count, 0);
        assert_eq!(cost.depth, 0);
        assert_eq!(cost.scalar_score, Some(0.0));
    }
}
