"""Tables and figures, generated only from a run's serialized data (§21, §30).

Nothing here is edited by hand, and nothing is computed from anything but
the files in the run directory: ``records.jsonl`` and, when present,
``determinism.json``, ``interop.json``, ``mutants.json``, ``coverage.json``
and ``hardware/``. Every table is written as CSV (machine-readable), Markdown
and LaTeX; every figure as PNG.

The report states what the data shows and nothing more (§30.1). Ratios are
reported with their confidence intervals and test statistics; whether a
difference matters is for a reader to judge.
"""

from __future__ import annotations

import csv
import json
import math
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable

from . import hardware as hardware_module
from . import stats

MAIN_METRICS = ("two_qubit_gates", "depth", "total_gates")


def _records(run: Path) -> list[dict]:
    with (run / "records.jsonl").open(encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def _write_table(directory: Path, name: str, header: list[str], rows: list[list[Any]],
                 caption: str) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    with (directory / f"{name}.csv").open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(header)
        writer.writerows(rows)

    def fmt(value: Any) -> str:
        if isinstance(value, float):
            if math.isnan(value):
                return "—"
            return f"{value:.4g}"
        return "—" if value is None else str(value)

    md = [f"<!-- generated from serialized data; do not edit -->", f"**{caption}**", "",
          "| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    md += ["| " + " | ".join(fmt(v) for v in row) + " |" for row in rows]
    (directory / f"{name}.md").write_text("\n".join(md) + "\n", encoding="utf-8")

    def tex(value: Any) -> str:
        return fmt(value).replace("_", r"\_").replace("%", r"\%").replace("—", "--")

    lines = [r"% generated from serialized data; do not edit", r"\begin{table}[ht]", r"\centering",
             rf"\caption{{{tex(caption)}}}", r"\begin{tabular}{" + "l" * len(header) + "}",
             r"\hline", " & ".join(tex(h) for h in header) + r" \\", r"\hline"]
    lines += [" & ".join(tex(v) for v in row) + r" \\" for row in rows]
    lines += [r"\hline", r"\end{tabular}", r"\end{table}"]
    (directory / f"{name}.tex").write_text("\n".join(lines) + "\n", encoding="utf-8")


def _median(values: Iterable[float]) -> float:
    values = list(values)
    return statistics.median(values) if values else float("nan")


def _main_configs(config: dict) -> list[str]:
    return [config["statistics"]["primary"], *config["statistics"]["baselines"]]


def compile_tables(records: list[dict], config: dict, tables: Path) -> dict[str, Any]:
    # Per-record flat table, with routing overhead against the all-to-all
    # result of the same circuit and configuration.
    two_qubit_all_to_all = {
        (r["circuit_id"], r["config_id"]): r["metrics"]["two_qubit_gates"]
        for r in records
        if r["target_kind"] == "all-to-all"
    }
    flat = []
    for r in records:
        baseline = two_qubit_all_to_all.get((r["circuit_id"], r["config_id"]))
        overhead = (
            r["metrics"]["two_qubit_gates"] - baseline
            if baseline is not None and r["target_kind"] != "all-to-all"
            else None
        )
        r["_routing_overhead"] = overhead
        flat.append([
            r["circuit_id"], r["circuit_family"], r["circuit_size"], r["circuit_width"],
            r["target_kind"], r["config_id"], r["metrics"]["total_gates"],
            r["metrics"]["one_qubit_gates"], r["metrics"]["two_qubit_gates"], r["metrics"]["depth"],
            r["metrics"]["native_gates"], r["swaps_inserted"], overhead,
            r["compilation_time"]["median_ms"], r["compilation_time"]["iqr_ms"],
            r["correctness"]["status"], r["correctness"]["method"],
        ])
    _write_table(
        tables, "compile_records",
        ["circuit", "family", "n", "width", "target", "config", "gates", "1q", "2q", "depth",
         "native", "swaps", "routing_overhead_2q", "time_median_ms", "time_iqr_ms",
         "correctness", "method"],
        flat, "Every compiled output: metrics, timing and correctness",
    )

    # Family x size x target summary for the main configurations. Random
    # circuits are aggregated as the median over instances, with min/max.
    main = _main_configs(config)
    groups: dict[tuple, list[dict]] = defaultdict(list)
    for r in records:
        if r["config_id"] in main:
            groups[(r["circuit_family"], r["circuit_size"], r["circuit_reps"], r["target_kind"],
                    r["config_id"])].append(r)
    summary = []
    for key in sorted(groups, key=lambda k: (k[0], k[1], k[2], k[3], main.index(k[4]))):
        rs = groups[key]
        two = [r["metrics"]["two_qubit_gates"] for r in rs]
        depth = [r["metrics"]["depth"] for r in rs]
        time_ms = [r["compilation_time"]["median_ms"] for r in rs]
        summary.append([key[0], key[1], key[2], key[3], key[4], len(rs), _median(two), min(two),
                        max(two), _median(depth), _median(time_ms)])
    _write_table(
        tables, "summary_by_family",
        ["family", "n", "reps", "target", "config", "instances", "2q_median", "2q_min", "2q_max",
         "depth_median", "time_median_ms"],
        summary, "Main configurations by family, size and target (medians over instances)",
    )

    # Paired ratios: primary / each baseline, per target and metric.
    primary = config["statistics"]["primary"]
    seed = int(config["seeds"]["master"])
    by_key = {(r["circuit_id"], r["target_kind"], r["config_id"]): r for r in records}
    ratio_rows = []
    ratios = {}
    for baseline in config["statistics"]["baselines"]:
        for kind in config["targets"]["kinds"]:
            pairs = [
                (by_key[(cid, kind, primary)], by_key[(cid, kind, baseline)])
                for (cid, k, c) in by_key
                if k == kind and c == primary and (cid, kind, baseline) in by_key
            ]
            if not pairs:
                continue
            for metric in ("two_qubit_gates", "depth", "compile_time_ms"):
                if metric == "compile_time_ms":
                    a = [p["compilation_time"]["median_ms"] for p, _ in pairs]
                    b = [q["compilation_time"]["median_ms"] for _, q in pairs]
                else:
                    a = [p["metrics"][metric] for p, _ in pairs]
                    b = [q["metrics"][metric] for _, q in pairs]
                result = stats.paired_ratio(
                    a, b, resamples=int(config["statistics"]["bootstrap_resamples"]),
                    confidence=float(config["statistics"]["confidence"]), seed=seed,
                )
                ratios[(baseline, kind, metric)] = result
                ratio_rows.append([
                    f"{primary} / {baseline}", kind, metric, result.get("n"),
                    result.get("excluded"), result.get("geometric_mean_ratio"),
                    result.get("ci_low"), result.get("ci_high"), result.get("wilcoxon_p"),
                ])
    _write_table(
        tables, "paired_ratios",
        ["comparison", "target", "metric", "pairs", "excluded", "geo_mean_ratio", "ci_low",
         "ci_high", "wilcoxon_p"],
        ratio_rows,
        "Paired ratios (geometric mean, 95% bootstrap CI, two-sided Wilcoxon signed-rank). "
        "A ratio below 1 means the primary configuration's value is smaller.",
    )

    # Ablation: each OQCI configuration against oqci-default, on the same items.
    ablation_rows = []
    ablations = sorted({r["config_id"] for r in records if r["compiler"] == "oqci"} - {primary})
    for config_id in ablations:
        for kind in config["targets"]["kinds"]:
            pairs = [
                (by_key[(cid, kind, primary)], by_key[(cid, kind, config_id)])
                for (cid, k, c) in by_key
                if k == kind and c == config_id and (cid, kind, primary) in by_key
            ]
            if not pairs:
                continue
            d2 = [q["metrics"]["two_qubit_gates"] - p["metrics"]["two_qubit_gates"] for p, q in pairs]
            dt = [q["metrics"]["total_gates"] - p["metrics"]["total_gates"] for p, q in pairs]
            dd = [q["metrics"]["depth"] - p["metrics"]["depth"] for p, q in pairs]
            ablation_rows.append([config_id, kind, len(pairs), statistics.mean(d2),
                                  statistics.mean(dt), statistics.mean(dd),
                                  sum(1 for x in dt if x > 0)])
    _write_table(
        tables, "ablation",
        ["config", "target", "items", "mean_delta_2q", "mean_delta_gates", "mean_delta_depth",
         "items_worse_in_gates"],
        ablation_rows, f"Ablation: each configuration minus {primary}, paired by item",
    )

    # Correctness methods per configuration.
    counts: dict[tuple, int] = defaultdict(int)
    for r in records:
        counts[(r["config_id"], r["correctness"]["status"], r["correctness"]["method"])] += 1
    _write_table(
        tables, "correctness",
        ["config", "status", "method", "records"],
        [[k[0], k[1], k[2], v] for k, v in sorted(counts.items())],
        "Correctness verdicts: every compiled output checked against its source",
    )

    # Pass-by-pass effect for the primary configuration.
    effect: dict[str, list[int]] = defaultdict(list)
    for r in records:
        if r["config_id"] == primary and r.get("oqci_passes"):
            for position, p in enumerate(r["oqci_passes"]):
                effect[f"{position}:{p['id']}"].append(p["before_ops"] - p["after_ops"])
    _write_table(
        tables, "pass_effect",
        ["position:pass", "records", "mean_ops_removed", "records_changed"],
        [[k, len(v), statistics.mean(v), sum(1 for x in v if x)] for k, v in sorted(effect.items())],
        f"Operations each pass removed, {primary}, all targets",
    )

    # Timing breakdown for OQCI.
    stage_rows = []
    for kind in config["targets"]["kinds"]:
        rs = [r for r in records if r["config_id"] == primary and r["target_kind"] == kind]
        if not rs:
            continue
        keys = [k for k in rs[0]["oqci_timings"] if k.endswith("_us")]
        stage_rows += [[kind, k[:-3], _median((r["oqci_timings"][k] or 0) for r in rs)] for k in keys]
    _write_table(tables, "oqci_stage_timings", ["target", "stage", "median_us"], stage_rows,
                 f"{primary}: median per-stage time, from the compiler's own Timings")
    return {"ratios": ratios, "summary_rows": len(summary)}


def figures(records: list[dict], config: dict, out: Path) -> list[str]:
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    out.mkdir(parents=True, exist_ok=True)
    main = _main_configs(config)
    made = []
    for metric, label in (("two_qubit_gates", "two-qubit gates"), ("depth", "depth")):
        for kind in config["targets"]["kinds"]:
            families = sorted({r["circuit_family"] for r in records})
            fig, axes = plt.subplots(1, len(families), figsize=(4 * len(families), 3.2), squeeze=False)
            for ax, family in zip(axes[0], families):
                for config_id in main:
                    rs = [r for r in records if r["circuit_family"] == family
                          and r["target_kind"] == kind and r["config_id"] == config_id
                          and r["circuit_reps"] in (0, 1)]
                    by_n: dict[int, list[float]] = defaultdict(list)
                    for r in rs:
                        by_n[r["circuit_size"]].append(r["metrics"][metric])
                    xs = sorted(by_n)
                    if xs:
                        ax.plot(xs, [_median(by_n[x]) for x in xs], marker="o", label=config_id)
                ax.set_title(f"{family} ({kind})")
                ax.set_xlabel("n")
                ax.set_ylabel(label)
            axes[0][0].legend(fontsize=7)
            fig.tight_layout()
            name = f"{metric}_by_size_{kind}.png"
            fig.savefig(out / name, dpi=150)
            plt.close(fig)
            made.append(name)

    fig, ax = plt.subplots(figsize=(7, 3.5))
    data, labels = [], []
    for config_id in main:
        times = [r["compilation_time"]["median_ms"] for r in records if r["config_id"] == config_id]
        if times:
            data.append(times)
            labels.append(config_id)
    ax.boxplot(data, tick_labels=labels)
    ax.set_yscale("log")
    ax.set_ylabel("compile time per circuit (ms, median of repetitions)")
    fig.tight_layout()
    fig.savefig(out / "compile_time.png", dpi=150)
    plt.close(fig)
    made.append("compile_time.png")

    primary = config["statistics"]["primary"]
    by_key = {(r["circuit_id"], r["target_kind"], r["config_id"]): r for r in records}
    fig, ax = plt.subplots(figsize=(7, 3.5))
    data, labels = [], []
    for baseline in config["statistics"]["baselines"]:
        ratios = [
            by_key[(c, k, primary)]["metrics"]["two_qubit_gates"]
            / by_key[(c, k, baseline)]["metrics"]["two_qubit_gates"]
            for (c, k, cfg) in by_key
            if cfg == primary and (c, k, baseline) in by_key
            and by_key[(c, k, baseline)]["metrics"]["two_qubit_gates"] > 0
            and by_key[(c, k, primary)]["metrics"]["two_qubit_gates"] > 0
        ]
        if ratios:
            data.append(ratios)
            labels.append(f"{primary}\n/ {baseline}")
    if data:
        ax.boxplot(data, tick_labels=labels)
        ax.axhline(1.0, color="grey", linewidth=0.8)
        ax.set_yscale("log")
        ax.set_ylabel("two-qubit gate ratio (per circuit and target)")
        fig.tight_layout()
        fig.savefig(out / "two_qubit_ratios.png", dpi=150)
        made.append("two_qubit_ratios.png")
    plt.close(fig)

    effect: dict[str, list[int]] = defaultdict(list)
    order: list[str] = []
    for r in records:
        if r["config_id"] == primary and r.get("oqci_passes"):
            for position, p in enumerate(r["oqci_passes"]):
                key = f"{position}:{p['id']}"
                if key not in order:
                    order.append(key)
                effect[key].append(p["before_ops"] - p["after_ops"])
    if order:
        fig, ax = plt.subplots(figsize=(7, 3.2))
        ax.bar(range(len(order)), [statistics.mean(effect[k]) for k in order])
        ax.set_xticks(range(len(order)), order, rotation=30, ha="right", fontsize=8)
        ax.set_ylabel("mean operations removed")
        ax.set_title(f"pass-by-pass effect ({primary})")
        fig.tight_layout()
        fig.savefig(out / "pass_by_pass.png", dpi=150)
        plt.close(fig)
        made.append("pass_by_pass.png")
    return made


def _optional(run: Path, name: str) -> Any:
    path = run / name
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else None


def secondary_tables(run: Path, tables: Path) -> dict[str, Any]:
    found = {}
    determinism = _optional(run, "determinism.json")
    if determinism:
        found["determinism"] = determinism
        _write_table(tables, "determinism",
                     ["platform", "compiler", "targets", "repetitions", "fresh_process", "items", "passed"],
                     [[determinism["platform"], determinism["compiler"], ",".join(determinism["targets"]),
                       determinism["repetitions"], determinism["fresh_process"], determinism["items"],
                       determinism["passed"]]],
                     "Determinism: byte-identical semantic outputs across fresh processes")
    interop = _optional(run, "interop.json")
    if interop:
        found["interop"] = interop
        rows = [[c, v["accepted"], v["rejected"], v["not_run"], v["not_applicable"]]
                for c, v in interop["acceptance"]["summary"].items()]
        _write_table(tables, "interop_acceptance",
                     ["consumer", "accepted", "rejected", "not_run", "not_applicable"], rows,
                     "QIR acceptance by external consumers (a consumer not run is not a pass)")
        rows = []
        for c in interop["agreement"]["circuits"]:
            for consumer, result in c["results"].items():
                rows.append([c["circuit_id"], consumer, result["status"], result.get("p_value"),
                             result.get("tvd"), result.get("bins")])
        _write_table(tables, "interop_agreement",
                     ["circuit", "consumer", "status", "p_value", "tvd", "bins"], rows,
                     f"Execution agreement with the exact distribution (chi-square, alpha={interop['agreement']['alpha']})")
    mutants = _optional(run, "mutants.json")
    if mutants:
        found["mutants"] = mutants
        checkers = [c["id"] for c in mutants["summary"]["checkers"]]
        rows = []
        for m in mutants["results"]:
            if not m.get("builds"):
                rows.append([m["id"], "does not build"] + [""] * len(checkers))
                continue
            rows.append([m["id"], ",".join(m["caught_by"]) or "SURVIVED"]
                        + ["caught" if m["checkers"][c]["caught"] else "–" for c in checkers])
        _write_table(tables, "mutation_evidence", ["mutant", "caught_by", *checkers], rows,
                     "Mutation evidence: which checker caught each injected defect")
    coverage = _optional(run, "coverage.json")
    if coverage:
        found["coverage"] = coverage
        rows = [[r["transformation"], r["stage"]] + [len(r["checkers"][k]) for k in coverage["kinds"]]
                + [r["strongest"]] for r in coverage["rows"]]
        _write_table(tables, "checker_coverage", ["transformation", "stage", *coverage["kinds"], "strongest"],
                     rows, "Checker coverage: number of checkers of each kind per transformation")
    hw_root = run / "hardware"
    if hw_root.exists():
        rows = []
        for directory in sorted(p for p in hw_root.iterdir() if (p / "results.json").exists()):
            for row in hardware_module.analyze(directory):
                rows.append([directory.name, row["backend"], row["fake"] or "", row["circuit_id"],
                             row["config_id"], row["shots"], row["hellinger_fidelity"], row["tvd"]])
        if rows:
            found["hardware"] = rows
            _write_table(tables, "hardware",
                         ["run", "backend", "fake", "circuit", "config", "shots", "hellinger_fidelity", "tvd"],
                         rows, "Hardware subset: Hellinger fidelity of observed counts to the exact distribution")
    return found


def build(run: Path, config: dict) -> Path:
    records = _records(run)
    report = run / "report"
    tables = report / "tables"
    compile_summary = compile_tables(records, config, tables)
    made = figures(records, config, report / "figures")
    found = secondary_tables(run, tables)
    summary = json.loads((run / "summary.json").read_text(encoding="utf-8")) if (run / "summary.json").exists() else {}
    env = json.loads((run / "environment.json").read_text(encoding="utf-8"))

    lines = [
        f"# Benchmark report — `{run.name}`",
        "",
        "<!-- generated by `python -m oqci_bench report`; do not edit -->",
        "",
        f"- Protocol: `{config['protocol']['name']}` {config['protocol']['version']}",
        f"- Repository commit: `{env['repository_commit']}`"
        + (" (working tree had local changes)" if env.get("repository_dirty") else ""),
        f"- OQCI {env['oqci'].get('version')} built from `{env['oqci'].get('compiled_from_commit')}`",
        f"- Platform: {env['platform']}, Python {env['python'].split()[0]}",
        f"- Compilers: " + ", ".join(f"{k} {v}" for k, v in env["packages"].items() if v and k in ("qiskit", "pytket")),
        f"- Records: {len(records)}; correctness {summary.get('correctness', {})}",
        "",
        "This report states what was measured. It makes no claim that one compiler is better,",
        "faster or more accurate than another beyond the ratios, intervals and tests shown,",
        "for the circuits, targets and versions listed (`final-deliverables-spec.md` §30.1).",
        "",
        "## Tables",
        "",
    ]
    for table in sorted(tables.glob("*.md")):
        lines.append(f"### {table.stem}")
        lines.append("")
        lines.append(table.read_text(encoding="utf-8"))
    lines += ["## Figures", ""]
    lines += [f"![{name}](figures/{name})" for name in made]
    if "interop" not in found:
        lines += ["", "_Interoperability was not run for this report._"]
    if "mutants" not in found:
        lines += ["", "_Mutation evidence was not run for this report._"]
    (report / "README.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return report
