//! Temporal coupling analysis via git history.
//!
//! Detects files that frequently change together in the same commits.
//! The critical insight: file pairs with high co-change frequency but NO import/call
//! relationship reveal hidden dependencies (shared database state, implicit contracts,
//! config coupling) that are invisible to every other static analysis tool.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use super::config::ThresholdConfig;
use super::graph::AnalysisGraph;
use super::path_classification;
use super::report::{Finding, FindingType, Severity};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TemporalCouplingResult {
    pub pairs: Vec<FilePairCoupling>,
    pub module_pairs: Vec<ModulePairCoupling>,
    pub findings: Vec<Finding>,
    pub high_coupling_pairs: usize,
    pub hidden_coupling_pairs: usize,
}

#[derive(Debug, Clone)]
pub struct FilePairCoupling {
    pub file_a: String,
    pub file_b: String,
    pub co_change_count: usize,
    pub coupling_score: f64,
    pub has_structural_edge: bool,
}

#[derive(Debug, Clone)]
pub struct ModulePairCoupling {
    pub module_a: String,
    pub module_b: String,
    pub co_change_count: usize,
    pub coupling_score: f64,
    pub has_import_coupling: bool,
}

// ---------------------------------------------------------------------------
// Source file detection
// ---------------------------------------------------------------------------

const SOURCE_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "rs", "py", "pyi", "java", "kt", "kts", "go", "cs",
    "c", "cc", "cpp", "cxx", "h", "hpp", "hxx", "rb", "swift", "scala", "php", "dart", "lua", "ex",
    "exs", "hs", "ml", "mli", "vue", "svelte",
];

fn is_source_file(path: &str) -> bool {
    if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
        SOURCE_EXTENSIONS.contains(&ext.to_lowercase().as_str())
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Git log parsing
// ---------------------------------------------------------------------------

struct Commit {
    files: Vec<String>,
}

/// Parse git log output into a list of commits with their changed files.
/// Uses `--name-only --pretty=format:"%H"` to get commit hashes and file lists.
fn parse_git_log(git_dir: &Path, since_months: usize) -> Result<Vec<Commit>, String> {
    let since_arg = format!("{} months ago", since_months);

    let output = Command::new("git")
        .args([
            "--git-dir",
            &git_dir.to_string_lossy(),
            "log",
            "--name-only",
            "--pretty=format:%H",
            "--since",
            &since_arg,
            "--no-merges",
        ])
        .output()
        .map_err(|e| format!("Failed to run git log: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git log failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_git_log_output(&stdout)
}

fn parse_git_log_output(output: &str) -> Result<Vec<Commit>, String> {
    let mut commits = Vec::new();
    let mut current_files: Vec<String> = Vec::new();
    let mut seen_hash = false;

    for line in output.lines() {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if line.len() == 40 && line.chars().all(|c| c.is_ascii_hexdigit()) {
            if seen_hash && !current_files.is_empty() {
                commits.push(Commit {
                    files: std::mem::take(&mut current_files),
                });
            }
            seen_hash = true;
            continue;
        }

        if seen_hash {
            let normalized = normalize_path(line);
            if is_source_file(&normalized)
                && !path_classification::is_test_path(&normalized)
                && !path_classification::is_auxiliary_path(&normalized)
            {
                current_files.push(normalized);
            }
        }
    }

    if seen_hash && !current_files.is_empty() {
        commits.push(Commit {
            files: current_files,
        });
    }

    Ok(commits)
}

/// Normalize a file path for consistent matching between git and graph paths.
fn normalize_path(path: &str) -> String {
    let p = path.trim().replace('\\', "/");
    p.strip_prefix("./").unwrap_or(&p).to_string()
}

// ---------------------------------------------------------------------------
// Ghost file filtering
// ---------------------------------------------------------------------------

/// Build a set of normalized file paths that exist in the current graph.
/// Used to filter out files from git history that no longer exist.
fn build_known_file_set(graph: &AnalysisGraph) -> HashSet<String> {
    let mut known = HashSet::new();
    for node in graph.nodes.values() {
        if let Some(ref prr) = node.path_repo_rel {
            let norm = normalize_path(prr);
            if !norm.is_empty() {
                known.insert(norm);
            }
        }
    }
    known
}

/// Remove any file from commits that doesn't exist in `known_files`.
/// Returns the count of ghost files removed.
fn filter_ghost_files(commits: &mut Vec<Commit>, known_files: &HashSet<String>) -> usize {
    let mut removed = 0usize;
    for commit in commits.iter_mut() {
        let before = commit.files.len();
        commit.files.retain(|f| known_files.contains(f));
        removed += before - commit.files.len();
    }
    commits.retain(|c| !c.files.is_empty());
    removed
}

// ---------------------------------------------------------------------------
// Co-change computation
// ---------------------------------------------------------------------------

struct CoChangeData {
    co_change_count: usize,
    total_changes_a: usize,
    total_changes_b: usize,
}

fn compute_co_changes(
    commits: &[Commit],
    max_files_per_commit: usize,
) -> HashMap<(String, String), CoChangeData> {
    let mut file_change_counts: HashMap<String, usize> = HashMap::new();
    let mut pair_counts: HashMap<(String, String), usize> = HashMap::new();

    let mut skipped_commits = 0usize;

    for commit in commits {
        let unique_files: HashSet<&str> = commit.files.iter().map(|s| s.as_str()).collect();

        if unique_files.len() > max_files_per_commit {
            skipped_commits += 1;
            continue;
        }

        let files: Vec<&str> = unique_files.into_iter().collect();

        for &file in &files {
            *file_change_counts.entry(file.to_string()).or_default() += 1;
        }

        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                let (a, b) = ordered_pair(files[i], files[j]);
                *pair_counts.entry((a, b)).or_default() += 1;
            }
        }
    }

    if skipped_commits > 0 {
        eprintln!(
            "[ge-analyze] Skipped {} commits with > {} files (bulk refactors/reorgs).",
            skipped_commits, max_files_per_commit,
        );
    }

    pair_counts
        .into_iter()
        .map(|((a, b), count)| {
            let total_a = file_change_counts.get(&a).copied().unwrap_or(0);
            let total_b = file_change_counts.get(&b).copied().unwrap_or(0);
            (
                (a, b),
                CoChangeData {
                    co_change_count: count,
                    total_changes_a: total_a,
                    total_changes_b: total_b,
                },
            )
        })
        .collect()
}

/// Return files in deterministic order (alphabetical) for consistent pair keys.
fn ordered_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

// ---------------------------------------------------------------------------
// Structural edge check
// ---------------------------------------------------------------------------

/// Build a set of file-path pairs that have any structural edge between nodes
/// they contain. Used to determine `has_structural_edge` for temporal coupling.
fn build_file_edge_set(graph: &AnalysisGraph) -> HashSet<(String, String)> {
    let mut edge_set: HashSet<(String, String)> = HashSet::new();

    for &idx in &graph.structural_edge_indices {
        let edge = &graph.edges[idx];

        let from_path = resolve_node_file_path(graph, &edge.from_id);
        let to_path = resolve_node_file_path(graph, &edge.to_id);

        if let (Some(fp_a), Some(fp_b)) = (from_path, to_path) {
            if fp_a != fp_b {
                let (a, b) = ordered_pair(&fp_a, &fp_b);
                edge_set.insert((a, b));
            }
        }
    }

    edge_set
}

/// Resolve a node to its containing file's repo-relative path.
fn resolve_node_file_path(graph: &AnalysisGraph, node_id: &str) -> Option<String> {
    let node = graph.nodes.get(node_id)?;

    if let Some(ref prr) = node.path_repo_rel {
        if !prr.is_empty() {
            return Some(normalize_path(prr));
        }
    }

    let file_node = graph.classification_of(node_id)?;
    file_node.path_repo_rel.as_ref().map(|p| normalize_path(p))
}

// ---------------------------------------------------------------------------
// Module-level aggregation
// ---------------------------------------------------------------------------

/// Build a lookup: normalized file path -> module key (O(N) once, O(1) per lookup).
fn build_file_to_module_map(graph: &AnalysisGraph) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (id, node) in &graph.nodes {
        if let Some(ref prr) = node.path_repo_rel {
            let norm = normalize_path(prr);
            if !norm.is_empty() {
                if let Some(folder) = graph.folder_of(id) {
                    map.insert(norm, folder.clone());
                }
            }
        }
    }
    map
}

fn aggregate_to_modules(
    pairs: &[FilePairCoupling],
    file_to_module: &HashMap<String, String>,
) -> Vec<ModulePairCoupling> {
    let mut module_pair_data: HashMap<(String, String), (usize, f64, bool)> = HashMap::new();

    for pair in pairs {
        let mod_a = file_to_module.get(&pair.file_a);
        let mod_b = file_to_module.get(&pair.file_b);

        if let (Some(ma), Some(mb)) = (mod_a, mod_b) {
            if ma == mb {
                continue;
            }
            let (m1, m2) = ordered_pair(ma, mb);
            let entry = module_pair_data.entry((m1, m2)).or_insert((0, 0.0, false));
            entry.0 += pair.co_change_count;
            if pair.coupling_score > entry.1 {
                entry.1 = pair.coupling_score;
            }
            entry.2 = entry.2 || pair.has_structural_edge;
        }
    }

    let mut results: Vec<ModulePairCoupling> = module_pair_data
        .into_iter()
        .map(|((ma, mb), (count, score, has_edge))| ModulePairCoupling {
            module_a: ma,
            module_b: mb,
            co_change_count: count,
            coupling_score: score,
            has_import_coupling: has_edge,
        })
        .collect();

    results.sort_by(|a, b| {
        b.coupling_score
            .partial_cmp(&a.coupling_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.co_change_count.cmp(&a.co_change_count))
            .then_with(|| a.module_a.cmp(&b.module_a))
    });

    results
}

// ---------------------------------------------------------------------------
// Finding generation
// ---------------------------------------------------------------------------

fn generate_findings(pairs: &[FilePairCoupling], thresholds: &ThresholdConfig) -> Vec<Finding> {
    let mut findings = Vec::new();

    for (idx, pair) in pairs.iter().enumerate() {
        let severity = if pair.coupling_score > thresholds.temporal_hidden_high_score
            && !pair.has_structural_edge
            && pair.co_change_count >= thresholds.temporal_hidden_high_min_co_changes
        {
            Severity::High
        } else if !pair.has_structural_edge {
            Severity::Warning
        } else {
            Severity::Info
        };

        let edge_desc = if pair.has_structural_edge {
            "with an existing dependency"
        } else {
            "with no import relationship"
        };

        findings.push(Finding {
            id: format!("temporal-{}", idx + 1),
            finding_type: FindingType::TemporalCoupling,
            severity,
            description: format!(
                "{} and {} change together in {:.0}% of commits ({} co-changes) {}",
                pair.file_a,
                pair.file_b,
                pair.coupling_score * 100.0,
                pair.co_change_count,
                edge_desc,
            ),
            detail: Some(
                "Temporal coupling is inferred from version control: files that change together in commits may share hidden dependencies. \
                 When no import relationship exists, this suggests implicit coupling that could be made explicit or eliminated."
                    .into(),
            ),
            node_ids: vec![],
            edge_ids: None,
            primary_node_id: None,
            metric_name: Some("temporal_coupling_score".into()),
            metric_value: Some(pair.coupling_score),
            impact: None,
            blast_radius: None,
            recommendation: if pair.has_structural_edge {
                Some("These files change together and have an explicit dependency. This is expected coupling — monitor for growth.".into())
            } else {
                Some("These files change together but have no explicit dependency. Investigate shared state, configuration, or implicit contracts. Consider making the dependency explicit or extracting a shared module.".into())
            },
            cycle_length: None,
            fan_in: None,
            coupling_score: None,
            internal_edges: None,
            external_edges: None,
            count: None,
            hub_score: None,
            file_a: Some(pair.file_a.clone()),
            file_b: Some(pair.file_b.clone()),
            co_change_count: Some(pair.co_change_count),
            temporal_coupling_score: Some(pair.coupling_score),
            has_import_edge: Some(pair.has_structural_edge),
            confidence: None,
        });
    }

    findings
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run temporal coupling analysis using git history.
///
/// `git_dir` should point to the `.git` directory of the repository.
/// The graph is used for `has_structural_edge` checking and module-level aggregation.
const ADAPTIVE_MIN_COMMITS: usize = 30;
const ADAPTIVE_WINDOWS: &[usize] = &[12, 24];

pub fn analyze_temporal_coupling(
    git_dir: &Path,
    graph: &AnalysisGraph,
    thresholds: &ThresholdConfig,
) -> Result<TemporalCouplingResult, String> {
    let mut since_months = thresholds.temporal_since_months;
    eprintln!(
        "[ge-analyze] Parsing git log (last {} months)...",
        since_months
    );
    let mut commits = parse_git_log(git_dir, since_months)?;
    eprintln!("[ge-analyze] {} commits parsed.", commits.len());

    if commits.len() < ADAPTIVE_MIN_COMMITS {
        for &window in ADAPTIVE_WINDOWS {
            if window <= since_months {
                continue;
            }
            eprintln!(
                "[ge-analyze] Only {} commits in {}-month window (need {}). Expanding to {} months...",
                commits.len(), since_months, ADAPTIVE_MIN_COMMITS, window,
            );
            since_months = window;
            commits = parse_git_log(git_dir, since_months)?;
            eprintln!(
                "[ge-analyze] {} commits parsed ({}-month window).",
                commits.len(),
                since_months
            );
            if commits.len() >= ADAPTIVE_MIN_COMMITS {
                break;
            }
        }
    }

    // Filter out ghost files: paths from git history that no longer exist
    let known_files = build_known_file_set(graph);
    let ghost_count = filter_ghost_files(&mut commits, &known_files);
    if ghost_count > 0 {
        eprintln!(
            "[ge-analyze] Filtered {} ghost file references (files no longer in current tree).",
            ghost_count,
        );
    }

    if commits.is_empty() {
        eprintln!(
            "[ge-analyze] No temporal coupling pairs found (0 commits in {}-month window). \
             Consider cloning with more depth or extending temporal_since_months.",
            since_months,
        );
        return Ok(TemporalCouplingResult {
            pairs: vec![],
            module_pairs: vec![],
            findings: vec![],
            high_coupling_pairs: 0,
            hidden_coupling_pairs: 0,
        });
    }

    let co_changes = compute_co_changes(&commits, thresholds.temporal_max_files_per_commit);
    let file_edge_set = build_file_edge_set(graph);

    // Warn if no file paths could be resolved (old DB schema or empty graph)
    if file_edge_set.is_empty() && graph.structural_edge_indices.len() > 100 {
        eprintln!(
            "[ge-analyze] Warning: could not resolve file paths for structural edge check. \
             All temporal coupling pairs will be marked as hidden. \
             Re-parse the repository with the current parser to populate path_repo_rel."
        );
    }

    let mut pairs: Vec<FilePairCoupling> = co_changes
        .into_iter()
        .filter_map(|((a, b), data)| {
            if data.co_change_count < thresholds.temporal_min_co_changes {
                return None;
            }

            let max_changes = data.total_changes_a.max(data.total_changes_b);
            if max_changes == 0 {
                return None;
            }

            let coupling_score = data.co_change_count as f64 / max_changes as f64;
            if coupling_score < thresholds.temporal_min_coupling_score {
                return None;
            }

            let (na, nb) = ordered_pair(&a, &b);
            let has_structural_edge = file_edge_set.contains(&(na.clone(), nb.clone()));

            Some(FilePairCoupling {
                file_a: na,
                file_b: nb,
                co_change_count: data.co_change_count,
                coupling_score,
                has_structural_edge,
            })
        })
        .collect();

    // Sort: hidden coupling first (no edge), then by score desc, then by count desc
    pairs.sort_by(|a, b| {
        a.has_structural_edge
            .cmp(&b.has_structural_edge)
            .then_with(|| {
                b.coupling_score
                    .partial_cmp(&a.coupling_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| b.co_change_count.cmp(&a.co_change_count))
            .then_with(|| a.file_a.cmp(&b.file_a))
    });

    let high_coupling_pairs = pairs
        .iter()
        .filter(|p| p.coupling_score >= thresholds.temporal_hidden_high_score)
        .count();

    let hidden_coupling_pairs = pairs.iter().filter(|p| !p.has_structural_edge).count();

    let file_to_module = build_file_to_module_map(graph);
    let module_pairs = aggregate_to_modules(&pairs, &file_to_module);
    let findings = generate_findings(&pairs, thresholds);

    if pairs.is_empty() {
        eprintln!(
            "[ge-analyze] No temporal coupling pairs found ({} commits in {}-month window, \
             min_co_changes={}, min_score={:.0}%). Consider cloning with more depth.",
            commits.len(),
            since_months,
            thresholds.temporal_min_co_changes,
            thresholds.temporal_min_coupling_score * 100.0,
        );
    } else {
        eprintln!(
            "[ge-analyze] {} temporal coupling pairs ({} hidden, {} high-score).",
            pairs.len(),
            hidden_coupling_pairs,
            high_coupling_pairs,
        );
    }

    Ok(TemporalCouplingResult {
        pairs,
        module_pairs,
        findings,
        high_coupling_pairs,
        hidden_coupling_pairs,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_log_output_basic() {
        let output = "\
abc123abc123abc123abc123abc123abc123abcd

src/auth/login.ts
src/database/users.ts

def456def456def456def456def456def456defa

src/auth/login.ts
src/config/settings.ts
src/database/users.ts
";
        let commits = parse_git_log_output(output).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].files.len(), 2);
        assert_eq!(commits[1].files.len(), 3);
    }

    #[test]
    fn parse_git_log_output_empty() {
        let commits = parse_git_log_output("").unwrap();
        assert!(commits.is_empty());
    }

    #[test]
    fn co_change_computation() {
        let commits = vec![
            Commit {
                files: vec!["a.ts".into(), "b.ts".into()],
            },
            Commit {
                files: vec!["a.ts".into(), "b.ts".into()],
            },
            Commit {
                files: vec!["a.ts".into(), "c.ts".into()],
            },
            Commit {
                files: vec!["b.ts".into(), "c.ts".into()],
            },
        ];

        let co_changes = compute_co_changes(&commits, 100);

        let (a, b) = ordered_pair("a.ts", "b.ts");
        let ab = co_changes.get(&(a, b)).unwrap();
        assert_eq!(ab.co_change_count, 2);
        assert_eq!(ab.total_changes_a, 3); // a.ts changed 3 times
        assert_eq!(ab.total_changes_b, 3); // b.ts changed 3 times

        let (a, c) = ordered_pair("a.ts", "c.ts");
        let ac = co_changes.get(&(a, c)).unwrap();
        assert_eq!(ac.co_change_count, 1);
    }

    #[test]
    fn coupling_score_calculation() {
        // a.ts changes 4 times, b.ts changes 4 times, they co-change 3 times
        // coupling = 3 / max(4, 4) = 0.75
        let commits = vec![
            Commit {
                files: vec!["a.ts".into(), "b.ts".into()],
            },
            Commit {
                files: vec!["a.ts".into(), "b.ts".into()],
            },
            Commit {
                files: vec!["a.ts".into(), "b.ts".into()],
            },
            Commit {
                files: vec!["a.ts".into()],
            },
            Commit {
                files: vec!["b.ts".into()],
            },
        ];

        let co_changes = compute_co_changes(&commits, 100);
        let (a, b) = ordered_pair("a.ts", "b.ts");
        let data = co_changes.get(&(a, b)).unwrap();
        let score =
            data.co_change_count as f64 / data.total_changes_a.max(data.total_changes_b) as f64;
        assert!((score - 0.75).abs() < 0.01);
    }

    #[test]
    fn ordered_pair_is_deterministic() {
        let (a1, b1) = ordered_pair("b.ts", "a.ts");
        let (a2, b2) = ordered_pair("a.ts", "b.ts");
        assert_eq!(a1, a2);
        assert_eq!(b1, b2);
        assert_eq!(a1, "a.ts");
        assert_eq!(b1, "b.ts");
    }

    #[test]
    fn normalize_path_strips_prefix() {
        assert_eq!(normalize_path("./src/foo.ts"), "src/foo.ts");
        assert_eq!(normalize_path("src/foo.ts"), "src/foo.ts");
        assert_eq!(normalize_path("src\\foo.ts"), "src/foo.ts");
    }

    #[test]
    fn large_commit_pair_count() {
        // A commit with 5 files produces 10 pairs (5 choose 2)
        let commits = vec![Commit {
            files: vec![
                "a.ts".into(),
                "b.ts".into(),
                "c.ts".into(),
                "d.ts".into(),
                "e.ts".into(),
            ],
        }];
        let co_changes = compute_co_changes(&commits, 100);
        assert_eq!(co_changes.len(), 10);
    }

    #[test]
    fn large_commits_are_skipped() {
        let commits = vec![
            Commit {
                files: vec!["a.ts".into(), "b.ts".into()],
            },
            Commit {
                files: (0..60).map(|i| format!("file{i}.ts")).collect(),
            },
        ];
        // max_files_per_commit = 50 should skip the 60-file commit
        let co_changes = compute_co_changes(&commits, 50);
        assert_eq!(co_changes.len(), 1); // only a.ts <-> b.ts
    }

    #[test]
    fn non_source_files_are_filtered() {
        let output = "\
abc123abc123abc123abc123abc123abc123abcd

src/main.rs
Cargo.lock
README.md
package.json
src/lib.rs
";
        let commits = parse_git_log_output(output).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].files.len(), 2); // only .rs files
        assert!(commits[0].files.contains(&"src/main.rs".to_string()));
        assert!(commits[0].files.contains(&"src/lib.rs".to_string()));
    }

    #[test]
    fn test_files_excluded_from_git_log_parsing() {
        let output = "\
abc123abc123abc123abc123abc123abc123abcd

src/utils/url.ts
src/utils/url.test.ts
src/adapter/handler.ts
src/adapter/handler.spec.ts
tests/integration.ts
";
        let commits = parse_git_log_output(output).unwrap();
        assert_eq!(commits.len(), 1);
        // Only production files should survive: url.ts and handler.ts
        assert_eq!(commits[0].files.len(), 2);
        assert!(commits[0].files.contains(&"src/utils/url.ts".to_string()));
        assert!(commits[0]
            .files
            .contains(&"src/adapter/handler.ts".to_string()));
    }

    #[test]
    fn go_test_files_excluded() {
        let output = "\
abc123abc123abc123abc123abc123abc123abcd

pkg/router.go
pkg/router_test.go
internal/handler.go
internal/handler_test.go
";
        let commits = parse_git_log_output(output).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].files.len(), 2);
        assert!(commits[0].files.contains(&"pkg/router.go".to_string()));
        assert!(commits[0]
            .files
            .contains(&"internal/handler.go".to_string()));
    }

    #[test]
    fn python_test_files_excluded() {
        let output = "\
abc123abc123abc123abc123abc123abc123abcd

src/auth/login.py
tests/test_auth.py
src/utils/helpers.py
src/utils/helpers_test.py
";
        let commits = parse_git_log_output(output).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].files.len(), 2);
        assert!(commits[0].files.contains(&"src/auth/login.py".to_string()));
        assert!(commits[0]
            .files
            .contains(&"src/utils/helpers.py".to_string()));
    }

    #[test]
    fn is_source_file_detection() {
        assert!(is_source_file("src/auth/login.ts"));
        assert!(is_source_file("lib/utils.py"));
        assert!(is_source_file("Main.java"));
        assert!(is_source_file("handler.go"));
        assert!(is_source_file("component.vue"));
        assert!(!is_source_file("Cargo.lock"));
        assert!(!is_source_file("README.md"));
        assert!(!is_source_file("config.yaml"));
        assert!(!is_source_file("data.json"));
        assert!(!is_source_file("schema.toml"));
        assert!(!is_source_file("image.png"));
        assert!(!is_source_file("app.db"));
    }

    #[test]
    fn ghost_files_are_filtered() {
        let mut commits = vec![
            Commit {
                files: vec![
                    "src/auth/login.ts".into(),
                    "old-project/src/deleted.ts".into(),
                ],
            },
            Commit {
                files: vec![
                    "old-project/src/deleted.ts".into(),
                    "old-project/src/also_deleted.ts".into(),
                ],
            },
            Commit {
                files: vec!["src/auth/login.ts".into(), "src/utils/helper.ts".into()],
            },
        ];

        let mut known = HashSet::new();
        known.insert("src/auth/login.ts".to_string());
        known.insert("src/utils/helper.ts".to_string());

        let removed = filter_ghost_files(&mut commits, &known);
        assert_eq!(removed, 3);
        // Second commit had only ghost files — should be removed entirely
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].files.len(), 1);
        assert_eq!(commits[0].files[0], "src/auth/login.ts");
        assert_eq!(commits[1].files.len(), 2);
    }
}
