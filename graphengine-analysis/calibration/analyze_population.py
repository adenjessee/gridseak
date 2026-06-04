#!/usr/bin/env python3
"""Analyze the calibration population and produce statistics + visualizations."""

import json
import os
from math import sqrt
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.ticker as mticker

SCRIPT_DIR = Path(__file__).parent
RESULTS_DIR = SCRIPT_DIR / "results"
OUTPUT_DIR = SCRIPT_DIR / "analysis_output"
OUTPUT_DIR.mkdir(exist_ok=True)


def load_all_health_reports():
    reports = []
    for f in sorted(RESULTS_DIR.iterdir()):
        if not f.name.endswith("_health.json"):
            continue
        try:
            data = json.loads(f.read_text())
        except Exception:
            continue

        summary = data.get("summary", {})
        components = data.get("health_score_components", {})
        total_nodes = summary.get("total_nodes", 0)
        total_funcs = summary.get("total_functions", 0)

        if total_nodes < 10:
            continue

        slug = f.stem.replace("_health", "")
        cycle_nodes = summary.get("cycle_total_nodes", 0)
        dead_funcs = summary.get("dead_functions", 0)

        reports.append({
            "slug": slug,
            "health_score": data.get("health_score", 0),
            "total_nodes": total_nodes,
            "total_edges": summary.get("total_edges", 0),
            "total_functions": total_funcs,
            "total_modules": summary.get("total_modules", 0),
            "cycles_found": summary.get("cycles_found", 0),
            "cycle_ratio": cycle_nodes / total_nodes if total_nodes > 0 else 0,
            "dead_ratio": dead_funcs / total_funcs if total_funcs > 0 else 0,
            "avg_coupling": summary.get("avg_module_coupling", 0),
            "max_depth": summary.get("max_call_depth", 0),
            "tangle_index": summary.get("tangle_index", 0),
            "hotspot_count": summary.get("hotspot_count", 0),
            "avg_fan_in": summary.get("avg_fan_in", 0),
            "avg_fan_out": summary.get("avg_fan_out", 0),
            "comp_cycle": components.get("cycle_severity", {}).get("score", 0),
            "comp_coupling": components.get("coupling_health", {}).get("score", 0),
            "comp_hotspot": components.get("hotspot_concentration", {}).get("score", 0),
            "comp_dead": components.get("dead_code_ratio", {}).get("score", 0),
            "comp_depth": components.get("depth_complexity", {}).get("score", 0),
        })
    return reports


def stats(values):
    if not values:
        return {}
    values = sorted(values)
    n = len(values)
    mean = sum(values) / n
    var = sum((v - mean) ** 2 for v in values) / n
    std = sqrt(var)
    median = values[n // 2] if n % 2 else (values[n // 2 - 1] + values[n // 2]) / 2
    return {
        "n": n, "mean": round(mean, 4), "std": round(std, 4),
        "median": round(median, 4), "min": round(values[0], 4), "max": round(values[-1], 4),
        "p5": round(values[int(n * 0.05)], 4), "p10": round(values[int(n * 0.10)], 4),
        "p25": round(values[int(n * 0.25)], 4), "p75": round(values[int(n * 0.75)], 4),
        "p90": round(values[int(n * 0.90)], 4), "p95": round(values[int(n * 0.95)], 4),
    }


def print_stats_table(name, s):
    print(f"\n  {name}:")
    print(f"    n={s['n']}  mean={s['mean']:.4f}  std={s['std']:.4f}  median={s['median']:.4f}")
    print(f"    min={s['min']:.4f}  p5={s['p5']:.4f}  p10={s['p10']:.4f}  p25={s['p25']:.4f}")
    print(f"    p75={s['p75']:.4f}  p90={s['p90']:.4f}  p95={s['p95']:.4f}  max={s['max']:.4f}")


def percentile_of(value, sorted_population):
    n = len(sorted_population)
    if n == 0:
        return 50
    below = sum(1 for v in sorted_population if v < value)
    equal = sum(1 for v in sorted_population if v == value)
    return int(100 * (below + equal / 2) / n)


def main():
    reports = load_all_health_reports()
    n = len(reports)
    print(f"Loaded {n} health reports (filtered: total_nodes >= 10)\n")

    # ── Metric definitions ──
    metric_defs = {
        "health_score": {"label": "Health Score (0-100)", "values": [r["health_score"] for r in reports], "higher_is_better": True},
        "cycle_ratio": {"label": "Cycle Ratio (nodes in cycles / total nodes)", "values": [r["cycle_ratio"] for r in reports], "higher_is_better": False},
        "dead_ratio": {"label": "Dead Code Ratio (dead funcs / total funcs)", "values": [r["dead_ratio"] for r in reports], "higher_is_better": False},
        "avg_coupling": {"label": "Avg Module Coupling (0-1)", "values": [r["avg_coupling"] for r in reports], "higher_is_better": False},
        "max_depth": {"label": "Max Call Depth", "values": [r["max_depth"] for r in reports], "higher_is_better": False},
        "tangle_index": {"label": "Tangle Index (0-1)", "values": [r["tangle_index"] for r in reports], "higher_is_better": False},
        "total_nodes": {"label": "Total Nodes (codebase size)", "values": [r["total_nodes"] for r in reports], "higher_is_better": None},
        "avg_fan_in": {"label": "Avg Fan-In", "values": [r["avg_fan_in"] for r in reports], "higher_is_better": None},
    }

    # ── Print stats ──
    all_stats = {}
    print("=" * 70)
    print("POPULATION STATISTICS — 242 Open Source Projects")
    print("=" * 70)

    for key, mdef in metric_defs.items():
        s = stats(mdef["values"])
        all_stats[key] = s
        print_stats_table(mdef["label"], s)

    # ── Component scores ──
    print("\n" + "=" * 70)
    print("COMPONENT SCORES (from old scoring system, 0-100 each)")
    print("=" * 70)
    for comp in ["comp_cycle", "comp_coupling", "comp_hotspot", "comp_dead", "comp_depth"]:
        s = stats([r[comp] for r in reports])
        print_stats_table(comp, s)

    # ── Notable projects ──
    print("\n" + "=" * 70)
    print("NOTABLE PROJECTS")
    print("=" * 70)

    sorted_by_score = sorted(reports, key=lambda r: r["health_score"], reverse=True)
    print("\n  Top 10 healthiest:")
    for r in sorted_by_score[:10]:
        print(f"    {r['health_score']:3d}  {r['slug']:50s}  nodes={r['total_nodes']}")

    print("\n  Bottom 10:")
    for r in sorted_by_score[-10:]:
        print(f"    {r['health_score']:3d}  {r['slug']:50s}  nodes={r['total_nodes']}")

    print("\n  Highest cycle ratios:")
    for r in sorted(reports, key=lambda r: r["cycle_ratio"], reverse=True)[:5]:
        print(f"    {r['cycle_ratio']:.4f}  {r['slug']:50s}  cycles={r['cycles_found']}")

    print("\n  Highest coupling:")
    for r in sorted(reports, key=lambda r: r["avg_coupling"], reverse=True)[:5]:
        print(f"    {r['avg_coupling']:.4f}  {r['slug']:50s}")

    print("\n  Largest codebases:")
    for r in sorted(reports, key=lambda r: r["total_nodes"], reverse=True)[:5]:
        print(f"    {r['total_nodes']:6d} nodes  {r['slug']}")

    # ── CHARTS ──
    fig, axes = plt.subplots(3, 3, figsize=(18, 14))
    fig.suptitle(f"GraphEngine Calibration Population — {n} Projects", fontsize=16, fontweight="bold")

    chart_metrics = [
        ("health_score", "Health Score", True),
        ("cycle_ratio", "Cycle Ratio", False),
        ("dead_ratio", "Dead Code Ratio", False),
        ("avg_coupling", "Avg Module Coupling", False),
        ("max_depth", "Max Call Depth", False),
        ("tangle_index", "Tangle Index", False),
        ("total_nodes", "Codebase Size (nodes)", None),
        ("avg_fan_in", "Avg Fan-In", None),
    ]

    for idx, (key, label, higher_better) in enumerate(chart_metrics):
        ax = axes[idx // 3][idx % 3]
        vals = metric_defs[key]["values"]
        s = all_stats[key]

        ax.hist(vals, bins=30, color="#4a90d9", edgecolor="white", alpha=0.85)
        ax.axvline(s["median"], color="#e74c3c", linewidth=2, linestyle="--", label=f"median={s['median']:.3f}")
        ax.axvline(s["mean"], color="#2ecc71", linewidth=2, linestyle=":", label=f"mean={s['mean']:.3f}")
        ax.set_title(label, fontsize=11, fontweight="bold")
        ax.set_ylabel("Count")
        ax.legend(fontsize=8)

    # Last cell: component score boxplot
    ax = axes[2][2]
    comp_data = [
        [r["comp_cycle"] for r in reports],
        [r["comp_coupling"] for r in reports],
        [r["comp_hotspot"] for r in reports],
        [r["comp_dead"] for r in reports],
        [r["comp_depth"] for r in reports],
    ]
    bp = ax.boxplot(comp_data, labels=["Cycle", "Coupling", "Hotspot", "Dead Code", "Depth"],
                    patch_artist=True, showmeans=True)
    colors = ["#e74c3c", "#3498db", "#e67e22", "#2ecc71", "#9b59b6"]
    for patch, color in zip(bp["boxes"], colors):
        patch.set_facecolor(color)
        patch.set_alpha(0.6)
    ax.set_title("Component Scores (0-100)", fontsize=11, fontweight="bold")
    ax.set_ylabel("Score")

    plt.tight_layout(rect=[0, 0, 1, 0.95])
    chart_path = OUTPUT_DIR / "population_distributions.png"
    fig.savefig(str(chart_path), dpi=150)
    print(f"\n\nChart saved: {chart_path}")

    # ── Percentile lookup table ──
    print("\n" + "=" * 70)
    print("PERCENTILE LOOKUP (where does a value sit in the population?)")
    print("=" * 70)

    for key in ["health_score", "cycle_ratio", "dead_ratio", "avg_coupling", "tangle_index", "max_depth"]:
        vals = sorted(metric_defs[key]["values"])
        s = all_stats[key]
        print(f"\n  {key}:")
        print(f"    p10={s['p10']:.4f}  p25={s['p25']:.4f}  p50={s['median']:.4f}  p75={s['p75']:.4f}  p90={s['p90']:.4f}")
        if key == "health_score":
            for test_val in [40, 50, 60, 70, 80, 90]:
                pct = percentile_of(test_val, vals)
                print(f"    score={test_val} → percentile {pct} (better than {pct}% of population)")
        elif key in ("cycle_ratio", "dead_ratio", "tangle_index"):
            for test_val in [0.0, 0.01, 0.05, 0.10, 0.20]:
                pct = 100 - percentile_of(test_val, vals)
                print(f"    ratio={test_val:.2f} → percentile {pct} (cleaner than {pct}% of population)")

    # Save full stats as JSON
    stats_path = OUTPUT_DIR / "population_stats.json"
    with open(stats_path, "w") as f:
        json.dump({"population_size": n, "metrics": all_stats}, f, indent=2)
    print(f"\nFull stats JSON: {stats_path}")


if __name__ == "__main__":
    main()
