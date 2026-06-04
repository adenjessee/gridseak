#!/usr/bin/env python3
"""
Mass calibration pipeline for GraphEngine structural health scoring.

Scans 200-500 public repositories to:
  1. Build a statistically meaningful population database (population.sqlite)
  2. Stress-test the parser across diverse codebases
  3. Generate population statistics for threshold calibration

Usage:
    python mass_calibrate.py                  # Full pipeline
    python mass_calibrate.py --fetch-only     # Just fetch repo list
    python mass_calibrate.py --skip-fetch     # Use cached repo list
    python mass_calibrate.py --workers 4      # Set parallelism
    python mass_calibrate.py --language rust   # Single language
"""

import argparse
import json
import logging
import os
import shutil
import sqlite3
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from dataclasses import asdict, dataclass, field
from datetime import datetime
from math import sqrt
from pathlib import Path
from typing import Optional
from urllib.request import Request, urlopen
from urllib.error import HTTPError, URLError

SCRIPT_DIR = Path(__file__).parent
WORKSPACE_ROOT = SCRIPT_DIR.parent.parent
CONFIGS_DIR = WORKSPACE_ROOT / "graphengine-parsing" / "configs"
STATE_FILE = SCRIPT_DIR / "calibration_state.json"
REPOS_DIR = SCRIPT_DIR / "repos"
RESULTS_DIR = SCRIPT_DIR / "results"
REPORTS_DIR = SCRIPT_DIR / "reports"
POPULATION_DB = SCRIPT_DIR / "population.sqlite"
FAILURES_FILE = SCRIPT_DIR / "calibration_failures.json"
SUMMARY_FILE = SCRIPT_DIR / "calibration_summary.json"

DEFAULT_CONSECUTIVE_FAIL_LIMIT = 5

# Parser currently supports: rust, python, javascript, typescript, go.
# C# support is in the ecosystem profiles but not yet in the parser.
# When C# parser ships, add "c#": 40 here.
LANGUAGE_TARGETS = {
    "typescript": 90,
    "javascript": 40,
    "python": 80,
    "rust": 50,
    "go": 50,
}

GITHUB_SEARCH_PARAMS = {
    "stars": ">500",
    "pushed": ">2025-08-01",
    "archived": "false",
    "size": "1000..102400",  # 1MB-100MB in KB
}

# Well-known repos to ensure are included (beyond what GitHub search returns).
# These are structurally interesting projects that test parser edge cases.
CURATED_REPOS = [
    # TypeScript — range from tiny (zod) to massive (angular)
    {"full_name": "colinhacks/zod", "language": "typescript", "stars": 35000},
    {"full_name": "honojs/hono", "language": "typescript", "stars": 22000},
    {"full_name": "trpc/trpc", "language": "typescript", "stars": 35000},
    {"full_name": "vercel/next.js", "language": "typescript", "stars": 130000},
    {"full_name": "prisma/prisma", "language": "typescript", "stars": 40000},
    {"full_name": "drizzle-team/drizzle-orm", "language": "typescript", "stars": 25000},
    {"full_name": "tldraw/tldraw", "language": "typescript", "stars": 40000},
    {"full_name": "calcom/cal.com", "language": "typescript", "stars": 33000},
    {"full_name": "Effect-TS/effect", "language": "typescript", "stars": 8000},
    {"full_name": "aidenybai/million", "language": "typescript", "stars": 16000},
    # JavaScript — legacy and modern
    {"full_name": "expressjs/express", "language": "javascript", "stars": 65000},
    {"full_name": "lodash/lodash", "language": "javascript", "stars": 60000},
    {"full_name": "chartjs/Chart.js", "language": "javascript", "stars": 65000},
    {"full_name": "mrdoob/three.js", "language": "javascript", "stars": 103000},
    {"full_name": "webpack/webpack", "language": "javascript", "stars": 64000},
    # Python — web frameworks to ML
    {"full_name": "tiangolo/fastapi", "language": "python", "stars": 80000},
    {"full_name": "pallets/flask", "language": "python", "stars": 68000},
    {"full_name": "django/django", "language": "python", "stars": 82000},
    {"full_name": "pydantic/pydantic", "language": "python", "stars": 22000},
    {"full_name": "encode/httpx", "language": "python", "stars": 13000},
    {"full_name": "psf/requests", "language": "python", "stars": 52000},
    {"full_name": "sqlalchemy/sqlalchemy", "language": "python", "stars": 10000},
    {"full_name": "python-poetry/poetry", "language": "python", "stars": 32000},
    # Rust — systems to web
    {"full_name": "tokio-rs/tokio", "language": "rust", "stars": 28000},
    {"full_name": "serde-rs/serde", "language": "rust", "stars": 9500},
    {"full_name": "BurntSushi/ripgrep", "language": "rust", "stars": 50000},
    {"full_name": "sharkdp/fd", "language": "rust", "stars": 35000},
    {"full_name": "denoland/deno", "language": "rust", "stars": 100000},
    {"full_name": "astral-sh/ruff", "language": "rust", "stars": 35000},
    # Go — infra to web
    {"full_name": "go-chi/chi", "language": "go", "stars": 18000},
    {"full_name": "gin-gonic/gin", "language": "go", "stars": 80000},
    {"full_name": "gofiber/fiber", "language": "go", "stars": 35000},
    {"full_name": "containerd/containerd", "language": "go", "stars": 18000},
    {"full_name": "hashicorp/terraform", "language": "go", "stars": 43000},
    {"full_name": "cli/cli", "language": "go", "stars": 38000},
    # C# — not yet supported by parser. Uncomment when csharp parser ships.
    # {"full_name": "dotnet/aspnetcore", "language": "c#", "stars": 36000},
    # {"full_name": "dotnet/runtime", "language": "c#", "stars": 15000},
    # {"full_name": "jellyfin/jellyfin", "language": "c#", "stars": 37000},
    # {"full_name": "bitwarden/server", "language": "c#", "stars": 16000},
    # {"full_name": "JamesNK/Newtonsoft.Json", "language": "c#", "stars": 11000},
    # {"full_name": "FluentValidation/FluentValidation", "language": "c#", "stars": 9000},
]


# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------

@dataclass
class RepoInfo:
    name: str
    full_name: str
    url: str
    clone_url: str
    language: str
    stars: int
    size_kb: int
    license: Optional[str] = None
    description: Optional[str] = None


@dataclass
class RepoState:
    name: str
    language: str
    status: str = "pending"  # pending, cloned, parsed, analyzed, failed
    error: Optional[str] = None
    parse_time_s: Optional[float] = None
    analyze_time_s: Optional[float] = None
    node_count: Optional[int] = None
    func_count: Optional[int] = None
    health_score: Optional[int] = None


@dataclass
class CalibrationState:
    repos: dict = field(default_factory=dict)  # name -> RepoState dict
    last_fetch: Optional[str] = None
    version: str = "1.0"


# ---------------------------------------------------------------------------
# GitHub API
# ---------------------------------------------------------------------------

def fetch_repos(language: str, count: int, token: Optional[str] = None) -> list[RepoInfo]:
    """Fetch repos from GitHub search API for a given language."""
    repos = []
    per_page = min(count, 100)
    pages_needed = (count + per_page - 1) // per_page

    for page in range(1, pages_needed + 1):
        query_parts = [
            f"language:{language}",
            f"stars:{GITHUB_SEARCH_PARAMS['stars']}",
            f"pushed:{GITHUB_SEARCH_PARAMS['pushed']}",
            f"archived:{GITHUB_SEARCH_PARAMS['archived']}",
            f"size:{GITHUB_SEARCH_PARAMS['size']}",
        ]
        q = "+".join(query_parts)
        url = f"https://api.github.com/search/repositories?q={q}&sort=stars&order=desc&per_page={per_page}&page={page}"

        headers = {"Accept": "application/vnd.github.v3+json"}
        if token:
            headers["Authorization"] = f"token {token}"

        try:
            req = Request(url, headers=headers)
            with urlopen(req, timeout=30) as resp:
                data = json.loads(resp.read().decode())
        except (HTTPError, URLError) as e:
            print(f"  [warn] GitHub API error for {language} page {page}: {e}", file=sys.stderr)
            break

        items = data.get("items", [])
        if not items:
            break

        for item in items:
            lic = item.get("license")
            repos.append(RepoInfo(
                name=item["name"],
                full_name=item["full_name"],
                url=item["html_url"],
                clone_url=item["clone_url"],
                language=language,
                stars=item.get("stargazers_count", 0),
                size_kb=item.get("size", 0),
                license=lic.get("spdx_id") if lic else None,
                description=item.get("description"),
            ))

        if len(repos) >= count:
            break

        time.sleep(1.0)  # rate-limit courtesy

    return repos[:count]


def curated_repo_infos() -> list[RepoInfo]:
    """Build RepoInfo objects from the curated repo list."""
    repos = []
    for entry in CURATED_REPOS:
        full = entry["full_name"]
        name = full.split("/")[-1]
        repos.append(RepoInfo(
            name=name,
            full_name=full,
            url=f"https://github.com/{full}",
            clone_url=f"https://github.com/{full}.git",
            language=entry["language"],
            stars=entry.get("stars", 0),
            size_kb=0,
        ))
    return repos


def fetch_all_repos(token: Optional[str] = None) -> list[RepoInfo]:
    """Fetch repos for all target languages, merged with curated list (deduplicated)."""
    seen = set()
    all_repos = []

    # Curated repos first — these are the ones we specifically want
    for repo in curated_repo_infos():
        if repo.full_name not in seen:
            seen.add(repo.full_name)
            all_repos.append(repo)

    # Fill remaining slots per language from GitHub API
    for lang, target in LANGUAGE_TARGETS.items():
        curated_for_lang = sum(1 for r in all_repos if r.language == lang)
        remaining = max(0, target - curated_for_lang)
        if remaining > 0:
            print(f"  Fetching {remaining} {lang} repos from GitHub API ({curated_for_lang} curated)...")
            api_repos = fetch_repos(lang, remaining + 20, token)  # over-fetch to account for dupes
            added = 0
            for repo in api_repos:
                if repo.full_name not in seen and added < remaining:
                    seen.add(repo.full_name)
                    all_repos.append(repo)
                    added += 1
            print(f"    Added {added} from API (total {curated_for_lang + added} for {lang})")
        else:
            print(f"  {lang}: {curated_for_lang} curated repos fill the target of {target}")

    return all_repos


# ---------------------------------------------------------------------------
# Clone / Parse / Analyze
# ---------------------------------------------------------------------------

def clone_repo(repo: RepoInfo) -> Optional[Path]:
    """Shallow-clone a repo. Returns path or None on failure.
    Uses full_name slug (org__repo) to avoid name collisions."""
    slug = safe_repo_slug(repo)
    dest = REPOS_DIR / slug
    if dest.exists():
        return dest

    try:
        subprocess.run(
            ["git", "clone", "--depth", "1", "--single-branch", repo.clone_url, str(dest)],
            capture_output=True,
            timeout=120,
            check=True,
        )
        return dest
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as e:
        print(f"  [fail] Clone {repo.full_name}: {e}", file=sys.stderr)
        if dest.exists():
            shutil.rmtree(dest, ignore_errors=True)
        return None


def find_binary(name: str) -> Optional[Path]:
    """Find a built binary in the workspace target directory."""
    release = WORKSPACE_ROOT / "target" / "release" / name
    debug = WORKSPACE_ROOT / "target" / "debug" / name
    if release.exists():
        return release
    if debug.exists():
        return debug
    return None


PARSER_LANGUAGE_MAP = {
    "c#": "csharp",
    "c++": "cpp",
}


def parse_repo(repo_path: Path, language: str, db_path: Path, parser_bin: Path) -> tuple[bool, str, float]:
    """Run graphengine-parsing on a repo. Returns (success, stderr, duration_seconds)."""
    parser_lang = PARSER_LANGUAGE_MAP.get(language, language)
    cmd = [
        str(parser_bin), "parse",
        "--root", str(repo_path),
        "--lang", parser_lang,
        "--db", str(db_path),
        "--clear",
    ]
    if CONFIGS_DIR.exists():
        cmd.extend(["--configs-dir", str(CONFIGS_DIR)])

    start = time.time()
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
        elapsed = time.time() - start
        return result.returncode == 0, result.stderr, elapsed
    except subprocess.TimeoutExpired:
        return False, "TIMEOUT after 300s", time.time() - start
    except Exception as e:
        return False, str(e), time.time() - start


def analyze_repo(db_path: Path, output_path: Path, analyzer_bin: Path, norms_path: Optional[Path] = None) -> tuple[bool, str, float]:
    """Run ge-analyze on a parsed database. Returns (success, stderr, duration_seconds)."""
    start = time.time()
    cmd = [str(analyzer_bin), "--db", str(db_path), "--output", str(output_path)]
    if norms_path and norms_path.exists():
        cmd.extend(["--norms", str(norms_path)])

    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        elapsed = time.time() - start
        return result.returncode == 0, result.stderr, elapsed
    except subprocess.TimeoutExpired:
        return False, "TIMEOUT after 120s", time.time() - start
    except Exception as e:
        return False, str(e), time.time() - start


# ---------------------------------------------------------------------------
# Metrics extraction
# ---------------------------------------------------------------------------

def extract_metrics(health_json_path: Path) -> Optional[dict]:
    """Extract raw metric values from a health report JSON."""
    try:
        with open(health_json_path) as f:
            report = json.load(f)
    except Exception:
        return None

    metrics = report.get("metrics", {})
    summary = report.get("summary", {})

    return {
        "cycle_ratio": metrics.get("cycles", {}).get("ratio", 0.0),
        "avg_coupling": metrics.get("coupling", {}).get("avg_coupling"),
        "dead_ratio": metrics.get("dead_code", {}).get("ratio", 0.0),
        "hotspot_concentration": metrics.get("hotspot_concentration", {}).get("ratio", 0.0),
        "max_depth": metrics.get("depth", {}).get("max_call_depth", 0),
        "tangle_index": metrics.get("tangle_index", {}).get("ratio", 0.0),
        "node_count": summary.get("total_nodes", 0),
        "func_count": summary.get("total_functions", 0),
        "health_score": report.get("health_score"),
    }


# ---------------------------------------------------------------------------
# Per-project report
# ---------------------------------------------------------------------------

def write_project_report(repo: RepoInfo, result: dict, report_dir: Path):
    """Write a self-contained report JSON for a single project into reports/<slug>.json.

    This is the human-readable record: who is this project, what did we measure,
    did it succeed, and how long did it take. The full ge-analyze health JSON
    is at results/<slug>_health.json for anyone who needs the raw data.
    """
    slug = safe_repo_slug(repo)
    report_dir.mkdir(parents=True, exist_ok=True)

    report = {
        "repo": repo.full_name,
        "url": repo.url,
        "language": repo.language,
        "stars": repo.stars,
        "analyzed_at": datetime.utcnow().isoformat() + "Z",
        "status": result["status"],
        "parse_time_s": result.get("parse_time_s"),
        "analyze_time_s": result.get("analyze_time_s"),
        "error": result.get("error"),
    }

    if result.get("metrics"):
        m = result["metrics"]
        report["metrics"] = {
            "node_count": m.get("node_count", 0),
            "func_count": m.get("func_count", 0),
            "health_score": m.get("health_score"),
            "cycle_ratio": m.get("cycle_ratio", 0.0),
            "avg_coupling": m.get("avg_coupling"),
            "dead_ratio": m.get("dead_ratio", 0.0),
            "hotspot_concentration": m.get("hotspot_concentration", 0.0),
            "max_depth": m.get("max_depth", 0),
            "tangle_index": m.get("tangle_index", 0.0),
        }
        report["health_json"] = str(RESULTS_DIR / f"{slug}_health.json")

    report_path = report_dir / f"{slug}.json"
    with open(report_path, "w") as f:
        json.dump(report, f, indent=2)


# ---------------------------------------------------------------------------
# Population database
# ---------------------------------------------------------------------------

def init_population_db(db_path: Path):
    """Create the population SQLite database."""
    conn = sqlite3.connect(str(db_path))
    conn.execute("""
        CREATE TABLE IF NOT EXISTS population (
            id                    TEXT PRIMARY KEY,
            analyzed_at           TEXT NOT NULL,
            language              TEXT NOT NULL,
            node_count            INTEGER NOT NULL,
            func_count            INTEGER NOT NULL,
            cycle_ratio           REAL NOT NULL,
            avg_coupling          REAL,
            dead_ratio            REAL NOT NULL,
            hotspot_concentration REAL NOT NULL,
            max_depth             INTEGER NOT NULL,
            tangle_index          REAL NOT NULL,
            source                TEXT NOT NULL,
            repo_url              TEXT,
            stars                 INTEGER
        )
    """)
    conn.commit()
    return conn


def populate_norms_db(conn: sqlite3.Connection, repo: RepoInfo, metrics: dict):
    """Insert metrics for a single repo into the population database."""
    now = datetime.utcnow().isoformat() + "Z"
    conn.execute(
        """INSERT OR REPLACE INTO population
           (id, analyzed_at, language, node_count, func_count, cycle_ratio, avg_coupling,
            dead_ratio, hotspot_concentration, max_depth, tangle_index, source, repo_url, stars)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (
            repo.full_name,
            now,
            repo.language,
            metrics.get("node_count", 0),
            metrics.get("func_count", 0),
            metrics.get("cycle_ratio", 0.0),
            metrics.get("avg_coupling"),
            metrics.get("dead_ratio", 0.0),
            metrics.get("hotspot_concentration", 0.0),
            metrics.get("max_depth", 0),
            metrics.get("tangle_index", 0.0),
            "mass_scan",
            repo.url,
            repo.stars,
        ),
    )
    conn.commit()


# ---------------------------------------------------------------------------
# Summary statistics
# ---------------------------------------------------------------------------

def compute_summary_stats(conn: sqlite3.Connection) -> dict:
    """Compute population statistics for all metrics."""
    cur = conn.execute("SELECT COUNT(*) FROM population")
    total = cur.fetchone()[0]

    by_language = {}
    for row in conn.execute("SELECT language, COUNT(*) FROM population GROUP BY language"):
        by_language[row[0]] = row[1]

    metric_cols = ["cycle_ratio", "avg_coupling", "dead_ratio", "hotspot_concentration", "max_depth", "tangle_index"]
    metrics_stats = {}

    for col in metric_cols:
        values = []
        for row in conn.execute(f"SELECT {col} FROM population WHERE {col} IS NOT NULL"):
            values.append(row[0])

        if not values:
            continue

        values.sort()
        n = len(values)
        mean = sum(values) / n
        variance = sum((v - mean) ** 2 for v in values) / n if n > 1 else 0
        stddev = sqrt(variance)
        median = values[n // 2] if n % 2 == 1 else (values[n // 2 - 1] + values[n // 2]) / 2
        p25 = values[int(n * 0.25)]
        p75 = values[int(n * 0.75)]
        p95 = values[int(n * 0.95)]

        metrics_stats[col] = {
            "count": n,
            "mean": round(mean, 6),
            "median": round(median, 6),
            "p25": round(p25, 6),
            "p75": round(p75, 6),
            "p95": round(p95, 6),
            "stddev": round(stddev, 6),
            "min": round(values[0], 6),
            "max": round(values[-1], 6),
        }

    return {
        "population_size": total,
        "by_language": by_language,
        "metrics": metrics_stats,
    }


# ---------------------------------------------------------------------------
# State management
# ---------------------------------------------------------------------------

def load_state() -> CalibrationState:
    if STATE_FILE.exists():
        with open(STATE_FILE) as f:
            data = json.load(f)
        state = CalibrationState()
        state.repos = data.get("repos", {})
        state.last_fetch = data.get("last_fetch")
        state.version = data.get("version", "1.0")
        return state
    return CalibrationState()


def save_state(state: CalibrationState):
    with open(STATE_FILE, "w") as f:
        json.dump({"repos": state.repos, "last_fetch": state.last_fetch, "version": state.version}, f, indent=2)


# ---------------------------------------------------------------------------
# Process a single repo (used by parallel executor)
# ---------------------------------------------------------------------------

def safe_repo_slug(repo: RepoInfo) -> str:
    """Generate a filesystem-safe slug from full_name (org/repo -> org__repo)."""
    return repo.full_name.replace("/", "__")


def process_single_repo(repo_dict: dict, parser_bin_str: str, analyzer_bin_str: str, keep_clones: bool = False) -> dict:
    """Process a single repo: clone -> parse -> analyze -> extract metrics -> cleanup.
    Returns a dict with status and results. Runs in a separate process.

    Disk strategy: clones are deleted after parsing (15-30GB saved). Parse databases
    (~3-10GB total) are kept so ge-analyze can be re-run with different params
    without re-parsing. Health JSONs (~60MB total) are always kept.
    """
    repo = RepoInfo(**repo_dict)
    parser_bin = Path(parser_bin_str)
    analyzer_bin = Path(analyzer_bin_str)
    slug = safe_repo_slug(repo)
    log = logging.getLogger(f"repo.{slug}")
    result = {"name": repo.full_name, "status": "failed", "error": None, "metrics": None}

    log.info(f"[clone] {repo.full_name} ({repo.language})")
    repo_path = clone_repo(repo)
    if repo_path is None:
        result["error"] = "clone_failed"
        log.error(f"[clone] FAILED {repo.full_name}")
        return result

    db_path = RESULTS_DIR / f"{slug}.sqlite"
    health_path = RESULTS_DIR / f"{slug}_health.json"

    try:
        log.info(f"[parse] {repo.full_name} -> {db_path.name}")
        ok, stderr, parse_time = parse_repo(repo_path, repo.language, db_path, parser_bin)
        result["parse_time_s"] = parse_time
        if not ok:
            result["error"] = f"parse_failed: {stderr[:500]}"
            log.error(f"[parse] FAILED {repo.full_name} ({parse_time:.1f}s): {stderr[:200]}")
            return result
        log.info(f"[parse] OK {repo.full_name} in {parse_time:.1f}s")

        log.info(f"[analyze] {repo.full_name}")
        ok, stderr, analyze_time = analyze_repo(db_path, health_path, analyzer_bin)
        result["analyze_time_s"] = analyze_time
        if not ok:
            result["error"] = f"analyze_failed: {stderr[:500]}"
            log.error(f"[analyze] FAILED {repo.full_name} ({analyze_time:.1f}s): {stderr[:200]}")
            return result
        log.info(f"[analyze] OK {repo.full_name} in {analyze_time:.1f}s")

        metrics = extract_metrics(health_path)
        if metrics is None:
            result["error"] = "metrics_extraction_failed"
            log.error(f"[extract] FAILED {repo.full_name}: could not read {health_path}")
            return result

        result["status"] = "analyzed"
        result["metrics"] = metrics
        log.info(f"[done] {repo.full_name}: nodes={metrics.get('node_count')}, "
                 f"score={metrics.get('health_score')}")
        return result

    finally:
        if not keep_clones and repo_path and repo_path.exists():
            clone_size_mb = sum(f.stat().st_size for f in repo_path.rglob("*") if f.is_file()) / (1024 * 1024)
            shutil.rmtree(repo_path, ignore_errors=True)
            log.info(f"[cleanup] Deleted clone for {repo.full_name} ({clone_size_mb:.0f} MB freed)")


# ---------------------------------------------------------------------------
# Main pipeline
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="Mass calibration pipeline for GraphEngine")
    parser.add_argument("--fetch-only", action="store_true", help="Only fetch repo list, don't process")
    parser.add_argument("--skip-fetch", action="store_true", help="Use cached repo list")
    parser.add_argument("--workers", type=int, default=2, help="Number of parallel workers")
    parser.add_argument("--language", type=str, help="Process only this language")
    parser.add_argument("--limit", type=int, help="Max repos to process")
    parser.add_argument("--token", type=str, help="GitHub API token (or set GITHUB_TOKEN env var)")
    parser.add_argument("--seed-existing", action="store_true", help="Seed population DB from existing calibration results")
    parser.add_argument("--keep-clones", action="store_true", help="Keep cloned source repos after processing (adds ~15-30GB)")
    parser.add_argument("--stop-after-failures", type=int, default=DEFAULT_CONSECUTIVE_FAIL_LIMIT,
                        help=f"Stop pipeline after N consecutive failures (default: {DEFAULT_CONSECUTIVE_FAIL_LIMIT}). Set 0 to never stop.")
    args = parser.parse_args()

    REPOS_DIR.mkdir(parents=True, exist_ok=True)
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    REPORTS_DIR.mkdir(parents=True, exist_ok=True)

    log_file = SCRIPT_DIR / "calibration.log"
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(levelname)s] %(message)s",
        handlers=[
            logging.FileHandler(str(log_file), mode="a"),
            logging.StreamHandler(sys.stdout),
        ],
    )
    log = logging.getLogger("calibrate")
    log.info("=" * 60)
    log.info("Mass calibration pipeline started")
    log.info(f"  Workspace root: {WORKSPACE_ROOT}")
    log.info(f"  Configs dir:    {CONFIGS_DIR} (exists={CONFIGS_DIR.exists()})")
    log.info(f"  Results dir:    {RESULTS_DIR}")
    log.info(f"  Reports dir:    {REPORTS_DIR}")
    log.info(f"  Log file:       {log_file}")

    token = args.token or os.environ.get("GITHUB_TOKEN")

    state = load_state()

    # --- Seed from existing calibration results (no GitHub needed) ---
    if args.seed_existing:
        print("Seeding population database from existing calibration results...")
        pop_conn = init_population_db(POPULATION_DB)
        seeded = 0
        for health_file in RESULTS_DIR.glob("*_health.json"):
            repo_name = health_file.stem.replace("_health", "")
            metrics = extract_metrics(health_file)
            if metrics:
                repo = RepoInfo(
                    name=repo_name,
                    full_name=repo_name,
                    url="",
                    clone_url="",
                    language="unknown",
                    stars=0,
                    size_kb=0,
                )
                populate_norms_db(pop_conn, repo, metrics)
                seeded += 1
                print(f"  Seeded: {repo_name}")
        pop_conn.close()
        print(f"Seeded {seeded} repos into {POPULATION_DB}")

        summary = compute_summary_stats(sqlite3.connect(str(POPULATION_DB)))
        with open(SUMMARY_FILE, "w") as f:
            json.dump(summary, f, indent=2)
        print(f"Summary stats written to {SUMMARY_FILE}")
        return

    # --- Fetch repos ---
    repo_list_file = SCRIPT_DIR / "repo_list.json"

    if not args.skip_fetch:
        if not token:
            print("[warn] No GitHub token provided. Rate limits will apply.", file=sys.stderr)
            print("       Set GITHUB_TOKEN env var or use --token", file=sys.stderr)

        print("Fetching repos from GitHub API...")
        if args.language:
            target = LANGUAGE_TARGETS.get(args.language, 50)
            all_repos = fetch_repos(args.language, target, token)
        else:
            all_repos = fetch_all_repos(token)

        with open(repo_list_file, "w") as f:
            json.dump([asdict(r) for r in all_repos], f, indent=2)

        state.last_fetch = datetime.utcnow().isoformat()
        save_state(state)
        print(f"Fetched {len(all_repos)} repos, saved to {repo_list_file}")

        if args.fetch_only:
            return
    else:
        if not repo_list_file.exists():
            print("Error: No cached repo list. Run without --skip-fetch first.", file=sys.stderr)
            sys.exit(1)
        with open(repo_list_file) as f:
            all_repos = [RepoInfo(**r) for r in json.load(f)]
        print(f"Loaded {len(all_repos)} repos from cache")

    # --- Find binaries ---
    parser_bin = find_binary("graphengine-parsing")
    analyzer_bin = find_binary("ge-analyze")

    if not parser_bin:
        print("Error: graphengine-parsing binary not found. Run `cargo build --release` first.", file=sys.stderr)
        sys.exit(1)
    if not analyzer_bin:
        print("Error: ge-analyze binary not found. Run `cargo build --release` first.", file=sys.stderr)
        sys.exit(1)

    log.info(f"Parser:   {parser_bin}")
    log.info(f"Analyzer: {analyzer_bin}")

    # Preflight: verify binaries actually execute
    for label, bin_path in [("Parser", parser_bin), ("Analyzer", analyzer_bin)]:
        try:
            probe = subprocess.run([str(bin_path), "--help"], capture_output=True, timeout=10)
            if probe.returncode != 0:
                log.warning(f"{label} --help returned exit code {probe.returncode}")
        except Exception as e:
            log.error(f"{label} binary failed to execute: {e}")
            sys.exit(1)

    # --- Filter repos ---
    repos_to_process = []
    for repo in all_repos:
        repo_state = state.repos.get(repo.full_name, {})
        if repo_state.get("status") == "analyzed":
            continue
        repos_to_process.append(repo)

    if args.limit:
        repos_to_process = repos_to_process[:args.limit]

    print(f"\nProcessing {len(repos_to_process)} repos ({len(all_repos) - len(repos_to_process)} already done)...")

    # --- Process repos ---
    pop_conn = init_population_db(POPULATION_DB)
    failures = []
    success_count = 0
    processed_count = 0
    total_count = len(repos_to_process)
    consecutive_failures = 0
    fail_limit = args.stop_after_failures
    stopped_early = False

    keep = args.keep_clones

    if args.workers <= 1:
        for i, repo in enumerate(repos_to_process):
            print(f"\n[{i+1}/{total_count}] {repo.full_name} ({repo.language}, {repo.stars}★)")
            result = process_single_repo(asdict(repo), str(parser_bin), str(analyzer_bin), keep_clones=keep)
            write_project_report(repo, result, REPORTS_DIR)
            _handle_result(result, repo, state, pop_conn, failures)
            processed_count += 1

            if result["status"] == "analyzed":
                success_count += 1
                consecutive_failures = 0
            else:
                consecutive_failures += 1

            save_state(state)

            if fail_limit > 0 and consecutive_failures >= fail_limit:
                print(f"\n{'='*60}")
                print(f"STOPPED: {consecutive_failures} consecutive failures.")
                print(f"Last error: {result.get('error', 'unknown')[:200]}")
                print(f"\nThis usually means a systemic issue (broken binary, disk full, etc).")
                print(f"Fix the issue, then re-run with --skip-fetch to resume where you left off.")
                print(f"Already-analyzed repos will be skipped automatically.")
                stopped_early = True
                break
    else:
        with ProcessPoolExecutor(max_workers=args.workers) as executor:
            futures = {}
            for repo in repos_to_process:
                future = executor.submit(
                    process_single_repo, asdict(repo), str(parser_bin), str(analyzer_bin), keep_clones=keep
                )
                futures[future] = repo

            for i, future in enumerate(as_completed(futures)):
                repo = futures[future]
                print(f"\n[{i+1}/{total_count}] {repo.full_name}")
                try:
                    result = future.result()
                except Exception as e:
                    result = {"name": repo.full_name, "status": "failed", "error": str(e), "metrics": None}

                write_project_report(repo, result, REPORTS_DIR)
                _handle_result(result, repo, state, pop_conn, failures)
                processed_count += 1

                if result["status"] == "analyzed":
                    success_count += 1
                    consecutive_failures = 0
                else:
                    consecutive_failures += 1

                save_state(state)

                if fail_limit > 0 and consecutive_failures >= fail_limit:
                    print(f"\n{'='*60}")
                    print(f"STOPPED: {consecutive_failures} consecutive failures.")
                    print(f"Last error: {result.get('error', 'unknown')[:200]}")
                    print(f"\nFix the issue, then re-run with --skip-fetch to resume.")
                    stopped_early = True
                    executor.shutdown(wait=False, cancel_futures=True)
                    break

    pop_conn.close()

    # --- Write failures ---
    with open(FAILURES_FILE, "w") as f:
        json.dump(failures, f, indent=2)

    # --- Compute and write summary ---
    summary_conn = sqlite3.connect(str(POPULATION_DB))
    summary = compute_summary_stats(summary_conn)
    summary["parse_success_rate"] = success_count / processed_count if processed_count > 0 else 0
    summary["parse_failures_by_language"] = {}
    for fail in failures:
        lang = fail.get("language", "unknown")
        summary["parse_failures_by_language"][lang] = summary["parse_failures_by_language"].get(lang, 0) + 1
    summary["stopped_early"] = stopped_early
    summary["processed"] = processed_count
    summary["remaining"] = total_count - processed_count
    summary_conn.close()

    with open(SUMMARY_FILE, "w") as f:
        json.dump(summary, f, indent=2)

    print(f"\n{'='*60}")
    if stopped_early:
        print(f"Pipeline stopped early due to consecutive failures.")
    else:
        print(f"Calibration complete!")
    print(f"  Processed:    {processed_count}/{total_count}")
    print(f"  Succeeded:    {success_count}")
    print(f"  Failed:       {len(failures)}")
    if processed_count > 0:
        print(f"  Success rate: {success_count/processed_count:.1%}")
    if stopped_early:
        print(f"  Remaining:    {total_count - processed_count}")
    print(f"\nOutputs:")
    print(f"  Reports:        {REPORTS_DIR}/  (one JSON per project)")
    print(f"  Parse DBs:      {RESULTS_DIR}/*.sqlite  (re-analyzable)")
    print(f"  Health JSONs:   {RESULTS_DIR}/*_health.json")
    print(f"  Population DB:  {POPULATION_DB}")
    print(f"  Failures:       {FAILURES_FILE}")
    print(f"  Summary stats:  {SUMMARY_FILE}")
    print(f"  State:          {STATE_FILE}")
    if stopped_early:
        print(f"\nTo resume: python3 mass_calibrate.py --skip-fetch")


def _handle_result(result: dict, repo: RepoInfo, state: CalibrationState, pop_conn: sqlite3.Connection, failures: list):
    """Process a single repo result: update state, populate DB, track failures."""
    if result["status"] == "analyzed" and result.get("metrics"):
        print(f"  ✓ {repo.full_name}: nodes={result['metrics'].get('node_count', '?')}, "
              f"score={result['metrics'].get('health_score', '?')}")
        populate_norms_db(pop_conn, repo, result["metrics"])
        state.repos[repo.full_name] = {
            "status": "analyzed",
            "language": repo.language,
            "node_count": result["metrics"].get("node_count"),
            "func_count": result["metrics"].get("func_count"),
            "health_score": result["metrics"].get("health_score"),
            "parse_time_s": result.get("parse_time_s"),
            "analyze_time_s": result.get("analyze_time_s"),
        }
    else:
        error_msg = result.get("error", "unknown")
        print(f"  ✗ {repo.full_name}: {error_msg[:100]}")
        failures.append({
            "repo": repo.full_name,
            "language": repo.language,
            "error": error_msg,
            "stars": repo.stars,
        })
        state.repos[repo.full_name] = {
            "status": "failed",
            "language": repo.language,
            "error": error_msg[:500],
        }


if __name__ == "__main__":
    main()
