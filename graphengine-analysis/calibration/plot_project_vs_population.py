#!/usr/bin/env python3
"""Plot a single project's metrics against the calibration population."""

import json
import sys
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.patches as mpatches

SCRIPT_DIR = Path(__file__).parent
RESULTS_DIR = SCRIPT_DIR / "results"
OUTPUT_DIR = SCRIPT_DIR / "analysis_output"


def load_population():
    reports = []
    for f in sorted(RESULTS_DIR.iterdir()):
        if not f.name.endswith("_health.json"):
            continue
        try:
            data = json.loads(f.read_text())
        except Exception:
            continue
        summary = data.get("summary", {})
        total_nodes = summary.get("total_nodes", 0)
        total_funcs = summary.get("total_functions", 0)
        if total_nodes < 10:
            continue
        cycle_nodes = summary.get("cycle_total_nodes", 0)
        dead_funcs = summary.get("dead_functions", 0)
        reports.append({
            "health_score": data.get("health_score", 0),
            "cycle_ratio": cycle_nodes / total_nodes if total_nodes > 0 else 0,
            "dead_ratio": dead_funcs / total_funcs if total_funcs > 0 else 0,
            "avg_coupling": summary.get("avg_module_coupling", 0),
            "max_depth": summary.get("max_call_depth", 0),
            "tangle_index": summary.get("tangle_index", 0),
        })
    return reports


def percentile_of(value, sorted_vals):
    n = len(sorted_vals)
    if n == 0:
        return 50
    below = sum(1 for v in sorted_vals if v < value)
    equal = sum(1 for v in sorted_vals if v == value)
    return round(100 * (below + equal / 2) / n)


def main():
    health_path = Path(sys.argv[1]) if len(sys.argv) > 1 else OUTPUT_DIR / "realbenefit_health.json"
    project_name = sys.argv[2] if len(sys.argv) > 2 else "RealBennefitSolutions"

    data = json.loads(health_path.read_text())
    summary = data.get("summary", {})
    tn = summary.get("total_nodes", 1)
    tf = summary.get("total_functions", 1)

    project = {
        "health_score": data.get("health_score", 0),
        "cycle_ratio": summary.get("cycle_total_nodes", 0) / tn if tn > 0 else 0,
        "dead_ratio": summary.get("dead_functions", 0) / tf if tf > 0 else 0,
        "avg_coupling": summary.get("avg_module_coupling", 0),
        "max_depth": summary.get("max_call_depth", 0),
        "tangle_index": summary.get("tangle_index", 0),
    }

    population = load_population()
    n = len(population)

    metrics = [
        ("health_score",  "Health Score",        True,  (30, 105)),
        ("cycle_ratio",   "Cycle Ratio",         False, (-0.01, 0.45)),
        ("dead_ratio",    "Dead Code Ratio",     False, (-0.02, 0.85)),
        ("avg_coupling",  "Avg Module Coupling", False, (-0.05, 1.0)),
        ("tangle_index",  "Tangle Index",        False, (-0.01, 0.65)),
        ("max_depth",     "Max Call Depth",       False, (-1, 38)),
    ]

    fig, axes = plt.subplots(2, 3, figsize=(20, 12))
    fig.patch.set_facecolor("#0d1117")
    fig.suptitle(f"{project_name}  vs  {n} Open Source Projects",
                 fontsize=18, fontweight="bold", color="white", y=0.97)

    percentiles = {}

    for idx, (key, label, higher_better, xlim) in enumerate(metrics):
        ax = axes[idx // 3][idx % 3]
        ax.set_facecolor("#161b22")

        pop_vals = sorted([r[key] for r in population])
        proj_val = project[key]

        if higher_better:
            pct = percentile_of(proj_val, pop_vals)
        else:
            pct = 100 - percentile_of(proj_val, pop_vals)

        percentiles[key] = pct

        nn = len(pop_vals)
        median = pop_vals[nn // 2] if nn % 2 else (pop_vals[nn // 2 - 1] + pop_vals[nn // 2]) / 2
        p25 = pop_vals[int(nn * 0.25)]
        p75 = pop_vals[int(nn * 0.75)]

        counts, bin_edges, patches = ax.hist(pop_vals, bins=40, color="#58a6ff",
                                              edgecolor="#0d1117", alpha=0.5, linewidth=0.5)

        for patch, left_edge in zip(patches, bin_edges[:-1]):
            right_edge = left_edge + (bin_edges[1] - bin_edges[0])
            mid = (left_edge + right_edge) / 2
            if higher_better:
                if mid >= p75:
                    patch.set_facecolor("#2ea043"); patch.set_alpha(0.4)
                elif mid >= p25:
                    patch.set_facecolor("#58a6ff"); patch.set_alpha(0.4)
                else:
                    patch.set_facecolor("#f85149"); patch.set_alpha(0.4)
            else:
                if mid <= p25:
                    patch.set_facecolor("#2ea043"); patch.set_alpha(0.4)
                elif mid <= p75:
                    patch.set_facecolor("#58a6ff"); patch.set_alpha(0.4)
                else:
                    patch.set_facecolor("#f85149"); patch.set_alpha(0.4)

        ax.axvline(median, color="#8b949e", linewidth=1, linestyle="--", alpha=0.6, label=f"pop median={median:.3g}")

        # Project marker — tall bright line with glow + large arrow + label
        ymax = max(counts) if len(counts) > 0 else 10
        if pct >= 60:
            color = "#3eff6e"
            glow = "#2ea04380"
        elif pct >= 40:
            color = "#ffb347"
            glow = "#f0883e80"
        else:
            color = "#ff4444"
            glow = "#f8514980"

        # Glow effect (wide translucent line behind the main line)
        ax.axvline(proj_val, color=glow, linewidth=14, linestyle="-", zorder=8)
        # Main line
        ax.axvline(proj_val, color=color, linewidth=4, linestyle="-", zorder=10)
        # Large downward arrow
        ax.plot(proj_val, ymax * 0.95, marker="v", markersize=22, color=color,
                zorder=11, markeredgecolor="white", markeredgewidth=1.5)
        # Label with background box
        ax.annotate(f" YOU: {proj_val:.3g}\n percentile {pct}",
                    xy=(proj_val, ymax * 0.85), fontsize=11, fontweight="bold",
                    color="white", ha="left", va="top", zorder=12,
                    bbox=dict(boxstyle="round,pad=0.4", facecolor=color, edgecolor="white",
                              alpha=0.92, linewidth=1.5))

        ax.set_xlim(xlim)
        ax.set_title(f"{label}  —  percentile {pct}", fontsize=13, fontweight="bold",
                     color=color, pad=10)
        ax.set_ylabel("Projects", fontsize=10, color="#8b949e")
        ax.tick_params(colors="#8b949e", labelsize=9)
        for spine in ax.spines.values():
            spine.set_color("#30363d")
        ax.legend(fontsize=8, facecolor="#161b22", edgecolor="#30363d", labelcolor="#8b949e")

    plt.tight_layout(rect=[0, 0.06, 1, 0.93])

    composite = round(sum(percentiles.values()) / len(percentiles))
    breakdown = "  |  ".join(f"{k}: p{v}" for k, v in percentiles.items())
    fig.text(0.5, 0.015,
             f"Composite Percentile: {composite}    ({breakdown})",
             ha="center", fontsize=11, color="#c9d1d9", fontweight="bold",
             bbox=dict(boxstyle="round,pad=0.5", facecolor="#161b22", edgecolor="#30363d"))

    path = OUTPUT_DIR / f"{project_name.lower().replace(' ', '_')}_vs_population.png"
    fig.savefig(str(path), dpi=180, facecolor=fig.get_facecolor())
    print(f"Saved: {path}")

    print(f"\n{'='*60}")
    print(f"  {project_name} — Percentile Report")
    print(f"{'='*60}")
    for key, label, higher_better, _ in metrics:
        pct = percentiles[key]
        val = project[key]
        direction = "better" if higher_better else "cleaner"
        print(f"  {label:25s}  {val:>8.4f}  →  percentile {pct:>3d}  ({direction} than {pct}% of population)")
    print(f"\n  Composite percentile: {composite}")
    print(f"  Health score: {project['health_score']}")


if __name__ == "__main__":
    main()
