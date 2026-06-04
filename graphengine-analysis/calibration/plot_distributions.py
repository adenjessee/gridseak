#!/usr/bin/env python3
"""Generate a clean distribution spread chart for the calibration population."""

import json
import os
from math import sqrt
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.patches as mpatches
import numpy as np

SCRIPT_DIR = Path(__file__).parent
RESULTS_DIR = SCRIPT_DIR / "results"
OUTPUT_DIR = SCRIPT_DIR / "analysis_output"
OUTPUT_DIR.mkdir(exist_ok=True)


def load_reports():
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
        slug = f.stem.replace("_health", "")
        reports.append({
            "slug": slug,
            "health_score": data.get("health_score", 0),
            "cycle_ratio": cycle_nodes / total_nodes if total_nodes > 0 else 0,
            "dead_ratio": dead_funcs / total_funcs if total_funcs > 0 else 0,
            "avg_coupling": summary.get("avg_module_coupling", 0),
            "max_depth": summary.get("max_call_depth", 0),
            "tangle_index": summary.get("tangle_index", 0),
            "total_nodes": total_nodes,
        })
    return reports


def main():
    reports = load_reports()
    n = len(reports)

    metrics = [
        ("health_score",  "Health Score",          True,  (30, 105)),
        ("cycle_ratio",   "Cycle Ratio",           False, (-0.01, 0.45)),
        ("dead_ratio",    "Dead Code Ratio",       False, (-0.02, 0.85)),
        ("avg_coupling",  "Avg Module Coupling",   False, (-0.05, 1.0)),
        ("tangle_index",  "Tangle Index",          False, (-0.01, 0.65)),
        ("max_depth",     "Max Call Depth",         False, (-1, 38)),
    ]

    fig, axes = plt.subplots(2, 3, figsize=(20, 12))
    fig.patch.set_facecolor("#0d1117")
    fig.suptitle(f"Population Distribution Spread — {n} Open Source Projects",
                 fontsize=18, fontweight="bold", color="white", y=0.97)

    for idx, (key, label, higher_better, xlim) in enumerate(metrics):
        ax = axes[idx // 3][idx % 3]
        ax.set_facecolor("#161b22")

        vals = sorted([r[key] for r in reports])
        nn = len(vals)
        mean = sum(vals) / nn
        median = vals[nn // 2] if nn % 2 else (vals[nn // 2 - 1] + vals[nn // 2]) / 2
        p10 = vals[int(nn * 0.10)]
        p25 = vals[int(nn * 0.25)]
        p75 = vals[int(nn * 0.75)]
        p90 = vals[int(nn * 0.90)]

        # Histogram
        bins = 40
        counts, bin_edges, patches = ax.hist(vals, bins=bins, color="#58a6ff",
                                              edgecolor="#0d1117", alpha=0.8, linewidth=0.5)

        # Color the bars by zone
        for patch, left_edge in zip(patches, bin_edges[:-1]):
            right_edge = left_edge + (bin_edges[1] - bin_edges[0])
            mid = (left_edge + right_edge) / 2
            if higher_better:
                if mid >= p75:
                    patch.set_facecolor("#2ea043")
                    patch.set_alpha(0.85)
                elif mid >= p25:
                    patch.set_facecolor("#58a6ff")
                    patch.set_alpha(0.8)
                else:
                    patch.set_facecolor("#f85149")
                    patch.set_alpha(0.8)
            else:
                if mid <= p25:
                    patch.set_facecolor("#2ea043")
                    patch.set_alpha(0.85)
                elif mid <= p75:
                    patch.set_facecolor("#58a6ff")
                    patch.set_alpha(0.8)
                else:
                    patch.set_facecolor("#f85149")
                    patch.set_alpha(0.8)

        # Percentile bands
        ax.axvspan(p25, p75, alpha=0.08, color="white", label="p25–p75 (middle 50%)")
        ax.axvline(median, color="#f0883e", linewidth=2.5, linestyle="-", label=f"median = {median:.3g}", zorder=5)
        ax.axvline(mean, color="#d2a8ff", linewidth=1.5, linestyle=":", label=f"mean = {mean:.3g}", zorder=5)
        ax.axvline(p10, color="#8b949e", linewidth=1, linestyle="--", alpha=0.7, label=f"p10 = {p10:.3g}")
        ax.axvline(p90, color="#8b949e", linewidth=1, linestyle="--", alpha=0.7, label=f"p90 = {p90:.3g}")

        ax.set_xlim(xlim)
        ax.set_title(label, fontsize=14, fontweight="bold", color="white", pad=10)
        ax.set_ylabel("Projects", fontsize=10, color="#8b949e")
        ax.tick_params(colors="#8b949e", labelsize=9)
        for spine in ax.spines.values():
            spine.set_color("#30363d")

        leg = ax.legend(fontsize=8, loc="upper right", facecolor="#161b22",
                        edgecolor="#30363d", labelcolor="#c9d1d9")

    plt.tight_layout(rect=[0, 0.02, 1, 0.94])

    # Bottom annotation
    fig.text(0.5, 0.005,
             "Green = top quartile  |  Blue = middle 50%  |  Red = bottom quartile  |  "
             "Orange line = median  |  Purple dotted = mean  |  Gray dashed = p10/p90",
             ha="center", fontsize=10, color="#8b949e", style="italic")

    path = OUTPUT_DIR / "distribution_spread.png"
    fig.savefig(str(path), dpi=180, facecolor=fig.get_facecolor())
    print(f"Saved: {path}")


if __name__ == "__main__":
    main()
