"""The statistics Stage G locks: goodness of fit, distances, paired ratios."""

from __future__ import annotations

import math
from typing import Mapping, Sequence

import numpy as np


def normalize(counts: Mapping[str, int]) -> dict[str, float]:
    total = sum(counts.values())
    return {k: v / total for k, v in counts.items()} if total else {}


def total_variation(p: Mapping[str, float], q: Mapping[str, float]) -> float:
    keys = set(p) | set(q)
    return 0.5 * sum(abs(p.get(k, 0.0) - q.get(k, 0.0)) for k in keys)


def hellinger_fidelity(p: Mapping[str, float], q: Mapping[str, float]) -> float:
    """(Σ √(p q))² — Qiskit's ``hellinger_fidelity`` definition."""
    keys = set(p) | set(q)
    bc = sum(math.sqrt(p.get(k, 0.0) * q.get(k, 0.0)) for k in keys)
    return bc * bc


def chi_square_gof(
    counts: Mapping[str, int], expected: Mapping[str, float], *, min_expected: float
) -> dict[str, float]:
    """Pearson goodness of fit of observed ``counts`` against probabilities.

    Bins whose expected count is below ``min_expected`` are pooled into one
    bin (and, if that pool is still below it, merged into the smallest kept
    bin), so the chi-square approximation holds. Outcomes observed but with
    zero expected probability are pooled too — and make the fit fail, as they
    should: they are outcomes the source cannot produce.
    """
    from scipy import stats

    shots = sum(counts.values())
    impossible = [k for k, v in counts.items() if v > 0 and expected.get(k, 0.0) <= 0.0]
    if impossible:
        # Outcomes the source cannot produce: no pooling may hide them.
        return {"statistic": math.inf, "p_value": 0.0, "dof": -1, "bins": 0,
                "impossible_outcomes": len(impossible)}
    keys = set(counts) | set(expected)
    expected_counts = {k: expected.get(k, 0.0) * shots for k in keys}
    kept = sorted(k for k in keys if expected_counts[k] >= min_expected)
    pooled = [k for k in keys if expected_counts[k] < min_expected]
    observed_bins = [counts.get(k, 0) for k in kept]
    expected_bins = [expected_counts[k] for k in kept]
    if pooled:
        pooled_obs = sum(counts.get(k, 0) for k in pooled)
        pooled_exp = sum(expected_counts[k] for k in pooled)
        if pooled_exp >= min_expected or not kept:
            observed_bins.append(pooled_obs)
            expected_bins.append(pooled_exp)
        else:
            smallest = int(np.argmin(expected_bins))
            observed_bins[smallest] += pooled_obs
            expected_bins[smallest] += pooled_exp
    if len(observed_bins) < 2:
        # One outcome: the fit is exact iff every shot landed on it.
        return {"statistic": 0.0, "p_value": 1.0, "dof": 0, "bins": len(observed_bins)}
    if any(e == 0 for e in expected_bins):
        return {"statistic": math.inf, "p_value": 0.0, "dof": len(observed_bins) - 1,
                "bins": len(observed_bins)}
    # Renormalize to the observed total, as chisquare requires equal sums.
    scale = sum(observed_bins) / sum(expected_bins)
    result = stats.chisquare(observed_bins, [e * scale for e in expected_bins])
    return {
        "statistic": float(result.statistic),
        "p_value": float(result.pvalue),
        "dof": len(observed_bins) - 1,
        "bins": len(observed_bins),
    }


def paired_ratio(
    primary: Sequence[float], baseline: Sequence[float], *, resamples: int, confidence: float,
    seed: int,
) -> dict[str, float]:
    """Geometric-mean ratio primary/baseline over paired items, with a
    percentile bootstrap CI, and a two-sided Wilcoxon signed-rank test on the
    paired differences. Pairs where either value is zero are excluded from the
    ratio (and counted), never imputed."""
    from scipy import stats

    pairs = [(a, b) for a, b in zip(primary, baseline) if a > 0 and b > 0]
    excluded = len(primary) - len(pairs)
    if not pairs:
        return {"n": 0, "excluded": excluded}
    logs = np.log([a / b for a, b in pairs])
    rng = np.random.default_rng(seed)
    boots = [float(np.mean(rng.choice(logs, size=len(logs), replace=True))) for _ in range(resamples)]
    alpha = (1 - confidence) / 2
    lo, hi = np.quantile(boots, [alpha, 1 - alpha])
    differences = [a - b for a, b in zip(primary, baseline)]
    if any(d != 0 for d in differences):
        w = stats.wilcoxon(differences, alternative="two-sided", zero_method="wilcox")
        w_stat, w_p = float(w.statistic), float(w.pvalue)
    else:
        w_stat, w_p = 0.0, 1.0
    return {
        "n": len(pairs),
        "excluded": excluded,
        "geometric_mean_ratio": float(math.exp(np.mean(logs))),
        "ci_low": float(math.exp(lo)),
        "ci_high": float(math.exp(hi)),
        "confidence": confidence,
        "wilcoxon_statistic": w_stat,
        "wilcoxon_p": w_p,
    }


def exact_distribution(circuit) -> dict[str, float]:
    """The exact outcome distribution of a circuit that measures every qubit
    into the classical bit of the same index, keyed clbit 0 rightmost."""
    from qiskit.quantum_info import Statevector

    unitary = circuit.remove_final_measurements(inplace=False)
    probabilities = Statevector(unitary).probabilities_dict()
    return {k: float(v) for k, v in probabilities.items() if v > 1e-15}
