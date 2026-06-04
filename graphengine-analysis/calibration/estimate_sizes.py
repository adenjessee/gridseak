#!/usr/bin/env python3
"""Estimate total disk usage for the calibration pipeline without downloading anything.

Queries GitHub API for actual repo sizes, then estimates clone and parse DB sizes.
"""

import json
import os
import sys
import time
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import HTTPError

SCRIPT_DIR = Path(__file__).parent
sys.path.insert(0, str(SCRIPT_DIR))
from mass_calibrate import CURATED_REPOS, LANGUAGE_TARGETS, fetch_all_repos

TOKEN = os.environ.get("GITHUB_TOKEN") or (sys.argv[1] if len(sys.argv) > 1 else None)


def fetch_repo_size(full_name: str) -> int:
    """Get repo size in KB from GitHub API."""
    url = f"https://api.github.com/repos/{full_name}"
    headers = {"Accept": "application/vnd.github.v3+json"}
    if TOKEN:
        headers["Authorization"] = f"token {TOKEN}"
    try:
        req = Request(url, headers=headers)
        with urlopen(req, timeout=15) as resp:
            data = json.loads(resp.read().decode())
        return data.get("size", 0)
    except HTTPError as e:
        print(f"  [warn] {full_name}: {e}", file=sys.stderr)
        return 0


def main():
    repo_list_file = SCRIPT_DIR / "repo_list.json"

    if repo_list_file.exists():
        print(f"Using cached repo list: {repo_list_file}")
        with open(repo_list_file) as f:
            repos = json.load(f)
    else:
        print("No cached repo list. Querying curated repos only (use --fetch-only to get full list).")
        repos = [{"full_name": r["full_name"], "language": r["language"],
                  "stars": r.get("stars", 0), "size_kb": 0} for r in CURATED_REPOS]

    print(f"\nFetching actual sizes for {len(repos)} repos from GitHub API...\n")

    by_language = {}
    total_size_kb = 0
    repo_sizes = []

    for i, repo in enumerate(repos):
        full_name = repo["full_name"]
        size_kb = repo.get("size_kb", 0)

        if size_kb == 0:
            size_kb = fetch_repo_size(full_name)
            time.sleep(0.3)

        lang = repo.get("language", "unknown")
        if lang not in by_language:
            by_language[lang] = {"count": 0, "size_kb": 0}
        by_language[lang]["count"] += 1
        by_language[lang]["size_kb"] += size_kb
        total_size_kb += size_kb

        repo_sizes.append({"repo": full_name, "language": lang, "size_kb": size_kb, "size_mb": round(size_kb / 1024, 1)})

        if (i + 1) % 20 == 0:
            print(f"  ...{i+1}/{len(repos)} queried")

    repo_sizes.sort(key=lambda r: r["size_kb"], reverse=True)

    total_mb = total_size_kb / 1024
    total_gb = total_mb / 1024

    # GitHub "size" is the repo content size. Shallow clone ≈ 60-80% of that.
    # Parse DB ≈ 20-40% of source size for code-heavy repos.
    clone_gb = total_gb * 0.7
    db_gb = total_gb * 0.3
    health_mb = len(repos) * 0.2

    print(f"\n{'='*70}")
    print(f"DISK USAGE ESTIMATE — {len(repos)} repos")
    print(f"{'='*70}")

    print(f"\nBy language:")
    for lang in sorted(by_language, key=lambda l: by_language[l]["size_kb"], reverse=True):
        info = by_language[lang]
        print(f"  {lang:15s}  {info['count']:3d} repos  {info['size_kb']/1024:8.1f} MB")

    print(f"\n  {'TOTAL':15s}  {len(repos):3d} repos  {total_mb:8.1f} MB  ({total_gb:.1f} GB)")

    print(f"\nEstimated disk usage:")
    print(f"  Shallow clones (deleted after parse):  ~{clone_gb:.1f} GB  (temporary)")
    print(f"  Parse SQLite databases (kept):          ~{db_gb:.1f} GB")
    print(f"  Health JSON reports (kept):              ~{health_mb:.0f} MB")
    print(f"  Per-project reports (kept):              ~{health_mb:.0f} MB")
    print(f"  Population DB (kept):                    ~1 MB")
    print(f"  ─────────────────────────────────────────────────")
    print(f"  Peak disk usage (during run):            ~{clone_gb + db_gb:.1f} GB")
    print(f"  Final disk usage (after run):            ~{db_gb + (health_mb*2)/1024:.1f} GB")

    print(f"\nTop 15 largest repos:")
    for r in repo_sizes[:15]:
        print(f"  {r['repo']:45s}  {r['language']:12s}  {r['size_mb']:8.1f} MB")

    print(f"\nBottom 5 smallest repos:")
    for r in repo_sizes[-5:]:
        print(f"  {r['repo']:45s}  {r['language']:12s}  {r['size_mb']:8.1f} MB")

    with open(SCRIPT_DIR / "size_estimate.json", "w") as f:
        json.dump({
            "total_repos": len(repos),
            "total_github_size_mb": round(total_mb, 1),
            "estimated_clone_gb": round(clone_gb, 1),
            "estimated_db_gb": round(db_gb, 1),
            "by_language": by_language,
            "repos": repo_sizes,
        }, f, indent=2)
    print(f"\nDetailed breakdown saved to: {SCRIPT_DIR / 'size_estimate.json'}")


if __name__ == "__main__":
    main()
