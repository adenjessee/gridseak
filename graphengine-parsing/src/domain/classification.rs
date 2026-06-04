//! Deterministic project classification + recommended view roots.
//!
//! This module provides stable, explainable labels for filesystem-derived nodes
//! (File/Folder/Project) and ranked candidate "view roots" for clients.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalRole {
    Source,
    Test,
    Docs,
    Tooling,
    BuildOutput,
    Generated,
    Vendor,
    Unknown,
}

impl UniversalRole {
    pub fn as_str(self) -> &'static str {
        match self {
            UniversalRole::Source => "source",
            UniversalRole::Test => "test",
            UniversalRole::Docs => "docs",
            UniversalRole::Tooling => "tooling",
            UniversalRole::BuildOutput => "build_output",
            UniversalRole::Generated => "generated",
            UniversalRole::Vendor => "vendor",
            UniversalRole::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleConfidence {
    High,
    Medium,
    Low,
}

impl RoleConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            RoleConfidence::High => "high",
            RoleConfidence::Medium => "medium",
            RoleConfidence::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationProvenanceSource {
    Detector,
    Manifest,
    Heuristic,
    Ai,
}

impl ClassificationProvenanceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ClassificationProvenanceSource::Detector => "detector",
            ClassificationProvenanceSource::Manifest => "manifest",
            ClassificationProvenanceSource::Heuristic => "heuristic",
            ClassificationProvenanceSource::Ai => "ai",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    pub role: UniversalRole,
    pub confidence: RoleConfidence,
    pub reason: String,
    pub provenance_source: ClassificationProvenanceSource,
    pub provenance_confidence: RoleConfidence,
    pub is_generated: bool,
    pub is_vendor: bool,
    pub is_build_output: bool,
    pub is_test: bool,
}

impl Classification {
    pub fn to_properties(
        &self,
        path_abs: &str,
        path_repo_rel: Option<&str>,
        language: Option<&str>,
    ) -> HashMap<String, serde_json::Value> {
        let mut props = HashMap::new();
        props.insert(
            "path_abs".to_string(),
            serde_json::Value::String(path_abs.to_string()),
        );
        if let Some(rel) = path_repo_rel {
            props.insert(
                "path_repo_rel".to_string(),
                serde_json::Value::String(rel.to_string()),
            );
        }
        props.insert(
            "role".to_string(),
            serde_json::Value::String(self.role.as_str().to_string()),
        );
        props.insert(
            "role_confidence".to_string(),
            serde_json::Value::String(self.confidence.as_str().to_string()),
        );
        props.insert(
            "role_reason".to_string(),
            serde_json::Value::String(self.reason.clone()),
        );
        props.insert(
            "is_generated".to_string(),
            serde_json::Value::Bool(self.is_generated),
        );
        props.insert(
            "is_vendor".to_string(),
            serde_json::Value::Bool(self.is_vendor),
        );
        props.insert(
            "is_build_output".to_string(),
            serde_json::Value::Bool(self.is_build_output),
        );
        props.insert("is_test".to_string(), serde_json::Value::Bool(self.is_test));
        if let Some(lang) = language {
            props.insert(
                "language".to_string(),
                serde_json::Value::String(lang.to_string()),
            );
        }
        props.insert(
            "role_provenance_source".to_string(),
            serde_json::Value::String(self.provenance_source.as_str().to_string()),
        );
        props.insert(
            "role_provenance_confidence".to_string(),
            serde_json::Value::String(self.provenance_confidence.as_str().to_string()),
        );
        props
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewRootCandidate {
    pub root_path_repo_rel: String,
    pub confidence: RoleConfidence,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

pub fn repo_relative_path(workspace_root: &Path, abs_path: &Path) -> Option<String> {
    let rel = abs_path.strip_prefix(workspace_root).ok()?;
    let rel = rel.to_string_lossy().to_string();
    // Normalize to forward slashes for cross-client consistency.
    Some(rel.replace('\\', "/").trim_start_matches('/').to_string())
}

/// Classify a file path. When `first_bytes` is provided, content-based generated
/// code detection supplements the path-based heuristics.
pub fn classify_path(
    abs_path: &Path,
    workspace_root: Option<&Path>,
    language: Option<&str>,
) -> Classification {
    classify_path_with_content(abs_path, workspace_root, language, None)
}

pub fn classify_path_with_content(
    abs_path: &Path,
    workspace_root: Option<&Path>,
    language: Option<&str>,
    first_bytes: Option<&str>,
) -> Classification {
    let repo_rel = workspace_root.and_then(|root| repo_relative_path(root, abs_path));
    let rel = repo_rel.as_deref().unwrap_or("");
    let file_name = abs_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let lower_rel = rel.to_ascii_lowercase();

    let segments: Vec<&str> = lower_rel.split('/').filter(|s| !s.is_empty()).collect();

    let has_segment = |name: &str| segments.contains(&name);
    let has_any_segment = |names: &[&str]| names.iter().any(|n| has_segment(n));

    // Vendor
    if has_any_segment(&["node_modules", "vendor"]) {
        return Classification {
            role: UniversalRole::Vendor,
            confidence: RoleConfidence::High,
            reason: "under vendor directory (node_modules/vendor)".to_string(),
            provenance_source: ClassificationProvenanceSource::Detector,
            provenance_confidence: RoleConfidence::High,
            is_generated: false,
            is_vendor: true,
            is_build_output: false,
            is_test: false,
        };
    }

    // Salesforce static resources — minified JS/CSS bundles, images,
    // archives, fonts. Any file under a "static resource" directory
    // is a third-party or deploy-artefact payload, never first-party
    // Apex source. Treating them as Vendor at the parse boundary
    // prevents the parser from emitting Function nodes for minified
    // tokens (see R19: 924+ phantom function nodes in NPSP).
    //
    // The convention is *not* uniform across Salesforce projects:
    //
    //   - `staticresources/`        — canonical SFDX MDAPI directory.
    //   - `StaticResourceSources/`  — NPSP, EDA, and many large DX
    //     projects keep unminified source here and bundle into
    //     `staticresources/` at build time.
    //
    // The matcher is case-insensitive and accepts any segment whose
    // name starts with `staticresource`. That covers both variants
    // and any future `StaticResource_Something/` flavour without
    // another one-off fix here. `lower_rel` has already been
    // lowercased, so the segment comparison is case-insensitive by
    // construction.
    if segments.iter().any(|s| s.starts_with("staticresource")) {
        return Classification {
            role: UniversalRole::Vendor,
            confidence: RoleConfidence::High,
            reason: "under Salesforce static-resource directory (minified/bundled payloads)"
                .to_string(),
            provenance_source: ClassificationProvenanceSource::Detector,
            provenance_confidence: RoleConfidence::High,
            is_generated: false,
            is_vendor: true,
            is_build_output: false,
            is_test: false,
        };
    }

    // Build outputs
    if has_any_segment(&[
        "dist",
        "build",
        "out",
        "target",
        "bin",
        "obj",
        "__pycache__",
    ]) {
        return Classification {
            role: UniversalRole::BuildOutput,
            confidence: RoleConfidence::High,
            reason: "under build output directory (dist/build/out/target/bin/obj/__pycache__)"
                .to_string(),
            provenance_source: ClassificationProvenanceSource::Detector,
            provenance_confidence: RoleConfidence::High,
            is_generated: false,
            is_vendor: false,
            is_build_output: true,
            is_test: false,
        };
    }

    // Docs
    if has_any_segment(&["docs", "doc"]) || file_name.eq_ignore_ascii_case("readme.md") {
        return Classification {
            role: UniversalRole::Docs,
            confidence: RoleConfidence::Medium,
            reason: "matched docs folder or README".to_string(),
            provenance_source: ClassificationProvenanceSource::Heuristic,
            provenance_confidence: RoleConfidence::Medium,
            is_generated: false,
            is_vendor: false,
            is_build_output: false,
            is_test: false,
        };
    }

    // Tests — Apex `*_TEST.cls` / `*_TESTS.cls` filename convention.
    // Salesforce DX and every major Apex project (NPSP, EDA, HEDA) mark
    // test classes with this suffix. The convention predates `@IsTest`
    // and coexists with it — many classes use both. Treating the
    // filename as authoritative prevents thousands of methods from
    // leaking into the production dead-code bucket when the parser's
    // AST-level `@IsTest` detection misses (see R18).
    //
    // Matching rules (all case-insensitive, stem = filename without
    // extension):
    //
    //   - stem ends with `_test`  or `_tests`
    //   - stem ends with `_test<N>` or `_tests<N>` where <N> is one or
    //     more digits — NPSP ships several companion fixtures named
    //     `FOO_TEST2.cls`, `FOO_TEST3.cls`, etc. Without the digit
    //     suffix allowance the hand-audit caught test methods leaking
    //     into `dynamic_dispatch_target` (2/10 wrong).
    //
    // This runs BEFORE the generic test block because it is high-
    // confidence language-specific evidence.
    if language == Some("apex") {
        let stem = file_name
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(file_name);
        let stem_lc = stem.to_ascii_lowercase();
        let stripped_digits: &str = stem_lc.trim_end_matches(|c: char| c.is_ascii_digit());
        if stripped_digits.ends_with("_test") || stripped_digits.ends_with("_tests") {
            return Classification {
                role: UniversalRole::Test,
                confidence: RoleConfidence::High,
                reason:
                    "Apex test class (_TEST / _TESTS filename convention, optional trailing digits)"
                        .to_string(),
                provenance_source: ClassificationProvenanceSource::Detector,
                provenance_confidence: RoleConfidence::High,
                is_generated: false,
                is_vendor: false,
                is_build_output: false,
                is_test: true,
            };
        }
    }

    // Tests — generic conventions (JS/TS, Rust, Python, Go).
    if has_any_segment(&["test", "tests", "__tests__", "__test__"])
        || lower_rel.ends_with(".test.ts")
        || lower_rel.ends_with(".spec.ts")
        || lower_rel.ends_with(".test.tsx")
        || lower_rel.ends_with(".spec.tsx")
        || lower_rel.ends_with("_test.rs")
    {
        return Classification {
            role: UniversalRole::Test,
            confidence: RoleConfidence::High,
            reason: "matched test directory or test filename pattern".to_string(),
            provenance_source: ClassificationProvenanceSource::Detector,
            provenance_confidence: RoleConfidence::High,
            is_generated: false,
            is_vendor: false,
            is_build_output: false,
            is_test: true,
        };
    }

    // Generated: path-based patterns
    let is_d_ts = file_name.ends_with(".d.ts");
    let is_protobuf = file_name.ends_with(".pb.ts")
        || file_name.ends_with(".pb.go")
        || file_name.ends_with(".pb.rs")
        || file_name.ends_with(".pb.js");
    let is_graphql_codegen = file_name.ends_with(".graphql.ts")
        || file_name.ends_with(".gql.ts")
        || file_name.ends_with(".graphql.js")
        || file_name.ends_with(".gql.js");
    let is_swagger = file_name.contains(".swagger.") || file_name.contains(".openapi.");
    let in_generated_dir = has_segment("__generated__");
    let looks_generated = file_name.contains(".generated.")
        || file_name.contains(".gen.")
        || file_name.ends_with("_generated.rs")
        || file_name.ends_with("_generated.go")
        || is_d_ts
        || is_protobuf
        || is_graphql_codegen
        || is_swagger
        || in_generated_dir;
    if looks_generated {
        let (confidence, reason) = if is_d_ts {
            (
                RoleConfidence::Low,
                "matched .d.ts (often generated; treated conservatively)",
            )
        } else if is_protobuf {
            (
                RoleConfidence::High,
                "matched protobuf generated file (*.pb.*)",
            )
        } else if is_graphql_codegen {
            (
                RoleConfidence::High,
                "matched GraphQL codegen output (*.graphql.ts / *.gql.ts)",
            )
        } else if is_swagger {
            (
                RoleConfidence::High,
                "matched API spec generated file (*.swagger.* / *.openapi.*)",
            )
        } else if in_generated_dir {
            (RoleConfidence::High, "under __generated__/ directory")
        } else {
            (RoleConfidence::High, "matched generated filename pattern")
        };
        return Classification {
            role: UniversalRole::Generated,
            confidence,
            reason: reason.to_string(),
            provenance_source: ClassificationProvenanceSource::Heuristic,
            provenance_confidence: confidence,
            is_generated: true,
            is_vendor: false,
            is_build_output: false,
            is_test: false,
        };
    }

    // Vendor: extended patterns
    if has_any_segment(&["third_party", "third-party", "external", "extern"]) {
        return Classification {
            role: UniversalRole::Vendor,
            confidence: RoleConfidence::High,
            reason: "under third-party/external directory".to_string(),
            provenance_source: ClassificationProvenanceSource::Detector,
            provenance_confidence: RoleConfidence::High,
            is_generated: false,
            is_vendor: true,
            is_build_output: false,
            is_test: false,
        };
    }

    // Tooling (conservative; only when unambiguous)
    if has_any_segment(&[".github", ".git", ".vscode", "scripts", "tools"]) {
        return Classification {
            role: UniversalRole::Tooling,
            confidence: RoleConfidence::Medium,
            reason: "matched tooling/config directory".to_string(),
            provenance_source: ClassificationProvenanceSource::Heuristic,
            provenance_confidence: RoleConfidence::Medium,
            is_generated: false,
            is_vendor: false,
            is_build_output: false,
            is_test: false,
        };
    }

    // Content-based generated code detection (when file content is available)
    if let Some(content) = first_bytes {
        if is_generated_content(content) {
            return Classification {
                role: UniversalRole::Generated,
                confidence: RoleConfidence::High,
                reason: "matched generated code marker in file header".to_string(),
                provenance_source: ClassificationProvenanceSource::Detector,
                provenance_confidence: RoleConfidence::High,
                is_generated: true,
                is_vendor: false,
                is_build_output: false,
                is_test: false,
            };
        }
    }

    // Default: source (deterministic, but conservative confidence if no manifest corroboration)
    let _ = language; // reserved for future language-specific refinements
    Classification {
        role: UniversalRole::Source,
        confidence: RoleConfidence::Medium,
        reason: "defaulted to source (no vendor/test/build/generated match)".to_string(),
        provenance_source: ClassificationProvenanceSource::Heuristic,
        provenance_confidence: RoleConfidence::Medium,
        is_generated: false,
        is_vendor: false,
        is_build_output: false,
        is_test: false,
    }
}

/// Content-based generated code detection. Checks the first N lines of source
/// content for codegen markers. Returns `true` if the content appears generated.
///
/// This supplements the path-based detection in `classify_path` for cases where
/// the filename alone does not indicate generated code (e.g., a `models.go` that
/// starts with `// Code generated by ... DO NOT EDIT`).
pub fn is_generated_content(content: &str) -> bool {
    let first_lines: Vec<&str> = content.lines().take(5).collect();
    for line in &first_lines {
        let trimmed = line.trim();
        // Go-style code generation marker
        if trimmed.contains("Code generated") && trimmed.contains("DO NOT EDIT") {
            return true;
        }
        // @generated annotation (various code generators)
        if trimmed.contains("@generated") || trimmed.contains("@auto-generated") {
            return true;
        }
        // Protocol Buffers generated header
        if trimmed.contains("Generated by the protocol buffer compiler") {
            return true;
        }
        // gRPC generated
        if trimmed.contains("Generated by the gRPC") {
            return true;
        }
        // OpenAPI/Swagger generator
        if trimmed.contains("This file was automatically generated") {
            return true;
        }
    }
    false
}

pub fn compute_recommended_view_roots(
    workspace_root: &Path,
    language: Option<&str>,
    source_files_abs: &[String],
) -> Vec<ViewRootCandidate> {
    let mut candidates: Vec<ViewRootCandidate> = Vec::new();

    // Language-aware manifest candidates.
    if language
        .map(|l| l.eq_ignore_ascii_case("typescript") || l.eq_ignore_ascii_case("javascript"))
        .unwrap_or(false)
    {
        if let Some(tsconfig) = find_manifest(workspace_root, "tsconfig.json", 4) {
            if let Some(root_dir) = tsconfig_root_dir(&tsconfig, workspace_root) {
                candidates.push(ViewRootCandidate {
                    root_path_repo_rel: root_dir,
                    confidence: RoleConfidence::High,
                    reason: "tsconfig.json compilerOptions.rootDir".to_string(),
                    tags: vec!["source".to_string()],
                });
            } else if let Some(include_root) = tsconfig_include_root(&tsconfig, workspace_root) {
                candidates.push(ViewRootCandidate {
                    root_path_repo_rel: include_root,
                    confidence: RoleConfidence::Medium,
                    reason: "tsconfig.json include (best common directory)".to_string(),
                    tags: vec!["source".to_string()],
                });
            }
        }
        if workspace_root.join("src").is_dir() {
            candidates.push(ViewRootCandidate {
                root_path_repo_rel: "src".to_string(),
                confidence: RoleConfidence::Medium,
                reason: "src/ directory exists".to_string(),
                tags: vec!["source".to_string()],
            });
        }
    }

    if language
        .map(|l| l.eq_ignore_ascii_case("rust"))
        .unwrap_or(false)
    {
        if let Some(cargo_toml) = workspace_root
            .join("Cargo.toml")
            .is_file()
            .then(|| workspace_root.join("Cargo.toml"))
        {
            if let Some(member_roots) = cargo_workspace_members(&cargo_toml, workspace_root) {
                for member in member_roots {
                    candidates.push(ViewRootCandidate {
                        root_path_repo_rel: member,
                        confidence: RoleConfidence::High,
                        reason: "Cargo.toml [workspace] members".to_string(),
                        tags: vec!["source".to_string()],
                    });
                }
            }
        }
        if workspace_root.join("src").is_dir() {
            candidates.push(ViewRootCandidate {
                root_path_repo_rel: "src".to_string(),
                confidence: RoleConfidence::High,
                reason: "Rust crate src/ directory exists".to_string(),
                tags: vec!["source".to_string()],
            });
        }
    }

    // Common monorepo heuristic.
    if workspace_root.join("packages").is_dir() {
        candidates.push(ViewRootCandidate {
            root_path_repo_rel: "packages".to_string(),
            confidence: RoleConfidence::Medium,
            reason: "packages/ directory exists (monorepo heuristic)".to_string(),
            tags: vec!["source".to_string()],
        });
    }

    // Density fallback: pick directory with the most discovered source files.
    if let Some(dense) = densest_directory_candidate(workspace_root, source_files_abs) {
        candidates.push(dense);
    }

    // Deduplicate, rank, and cap.
    dedup_and_rank(&mut candidates);
    candidates.truncate(6);
    candidates
}

fn dedup_and_rank(candidates: &mut Vec<ViewRootCandidate>) {
    // Dedup by path, keep highest confidence, then longer (deeper) paths.
    candidates.sort_by(|a, b| {
        let score = |c: RoleConfidence| match c {
            RoleConfidence::High => 3,
            RoleConfidence::Medium => 2,
            RoleConfidence::Low => 1,
        };
        score(b.confidence)
            .cmp(&score(a.confidence))
            .then_with(|| b.root_path_repo_rel.len().cmp(&a.root_path_repo_rel.len()))
            .then_with(|| a.root_path_repo_rel.cmp(&b.root_path_repo_rel))
    });

    let mut seen = std::collections::HashSet::new();
    candidates.retain(|c| seen.insert(c.root_path_repo_rel.clone()));
}

fn densest_directory_candidate(
    workspace_root: &Path,
    source_files_abs: &[String],
) -> Option<ViewRootCandidate> {
    // Instead of choosing the deepest directory containing the most files (which often picks
    // overly-specific paths like `src/typescript/lib`), we count file membership across ancestor
    // directories and pick a stable "best root" with a mild preference for depth≈2-3.
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut total_files: usize = 0;

    for f in source_files_abs {
        let abs = Path::new(f);
        let rel = repo_relative_path(workspace_root, abs)?;

        // Ignore obvious non-source subtrees (defensive; callers may pass broad file lists).
        let lower = rel.to_ascii_lowercase();
        if lower.contains("node_modules/")
            || lower.starts_with("node_modules/")
            || lower.contains("/dist/")
            || lower.starts_with("dist/")
            || lower.contains("/build/")
            || lower.starts_with("build/")
            || lower.contains("/out/")
            || lower.starts_with("out/")
            || lower.contains("/target/")
            || lower.starts_with("target/")
        {
            continue;
        }

        total_files += 1;

        let rel_path = Path::new(&rel);
        let dir = rel_path
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .trim_matches('/');
        if dir.is_empty() {
            continue;
        }

        let segments: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
        let capped_len = segments.len().min(4);
        for i in 1..=capped_len {
            let candidate = segments[..i].join("/");
            *counts.entry(candidate).or_insert(0) += 1;
        }
    }

    if total_files < 2 {
        return None;
    }

    // Prefer depth 2-3 when counts tie; it's usually a better "project root" than depth 1
    // (`src`) or depth 4 (`src/foo/bar/baz`).
    //
    // Since we're selecting a maximum, higher scores are better.
    let depth_preference_score = |depth: usize| match depth {
        2 => 4,
        3 => 3,
        1 => 2,
        4 => 1,
        _ => 0,
    };

    let (best_dir, best_count) =
        counts
            .into_iter()
            .filter(|(_, c)| *c >= 2)
            .max_by_key(|(dir, count)| {
                let depth = dir.split('/').filter(|s| !s.is_empty()).count();
                (*count, depth_preference_score(depth), dir.len())
            })?;

    let confidence = if best_count >= 3 && (best_count * 10) >= (total_files * 6) {
        RoleConfidence::Medium
    } else {
        RoleConfidence::Low
    };

    Some(ViewRootCandidate {
        root_path_repo_rel: best_dir,
        confidence,
        reason: "fallback: directory with highest discovered source file subtree density"
            .to_string(),
        tags: vec!["source".to_string()],
    })
}

fn find_manifest(workspace_root: &Path, filename: &str, max_depth: usize) -> Option<PathBuf> {
    for entry in walkdir::WalkDir::new(workspace_root)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.file_name().to_string_lossy() != filename {
            continue;
        }
        // Avoid node_modules noise for tsconfig searches.
        if entry.path().components().any(|c| {
            c.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("node_modules")
        }) {
            continue;
        }
        return Some(entry.into_path());
    }
    None
}

fn tsconfig_root_dir(tsconfig_path: &Path, workspace_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(tsconfig_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let root_dir = v
        .get("compilerOptions")
        .and_then(|c| c.get("rootDir"))
        .and_then(|r| r.as_str())?;

    let abs = if Path::new(root_dir).is_absolute() {
        PathBuf::from(root_dir)
    } else {
        workspace_root.join(root_dir)
    };
    let abs = abs.canonicalize().unwrap_or(abs);
    repo_relative_path(workspace_root, &abs)
}

fn tsconfig_include_root(tsconfig_path: &Path, workspace_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(tsconfig_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let include = v.get("include")?.as_array()?;
    let mut roots: Vec<String> = Vec::new();
    for item in include {
        let s = item.as_str()?;
        let s = s
            .split('*')
            .next()
            .unwrap_or("")
            .trim_matches('/')
            .to_string();
        if s.is_empty() {
            continue;
        }
        roots.push(s);
    }
    if roots.is_empty() {
        return None;
    }
    // Prefer deepest common path among include entries.
    roots.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let candidate = roots[0].clone();
    let abs = workspace_root.join(&candidate);
    if abs.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn cargo_workspace_members(cargo_toml_path: &Path, workspace_root: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(cargo_toml_path).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let members = v.get("workspace")?.get("members")?.as_array()?;

    let mut out = Vec::new();
    for m in members {
        let s = m.as_str()?;
        // Ignore globs for now; keep the literal prefix directory as a candidate if it exists.
        let prefix = s
            .split('*')
            .next()
            .unwrap_or("")
            .trim_matches('/')
            .to_string();
        if prefix.is_empty() {
            continue;
        }
        let abs = workspace_root.join(&prefix);
        if abs.exists() {
            out.push(prefix);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_vendor_node_modules() {
        let root = Path::new("/repo");
        let abs = Path::new("/repo/node_modules/pkg/index.ts");
        let c = classify_path(abs, Some(root), Some("typescript"));
        assert_eq!(c.role, UniversalRole::Vendor);
        assert!(c.is_vendor);
    }

    #[test]
    fn test_classify_build_output_dist() {
        let root = Path::new("/repo");
        let abs = Path::new("/repo/dist/app.js");
        let c = classify_path(abs, Some(root), Some("javascript"));
        assert_eq!(c.role, UniversalRole::BuildOutput);
        assert!(c.is_build_output);
    }

    #[test]
    fn test_repo_relative_path_normalizes() {
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/main.ts");
        assert_eq!(
            repo_relative_path(root, abs).as_deref(),
            Some("src/main.ts")
        );
    }

    #[test]
    fn test_classify_protobuf_generated() {
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/proto/messages.pb.ts");
        let c = classify_path(abs, Some(root), Some("typescript"));
        assert_eq!(c.role, UniversalRole::Generated);
        assert!(c.is_generated);
    }

    #[test]
    fn test_classify_graphql_codegen() {
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/schema.graphql.ts");
        let c = classify_path(abs, Some(root), Some("typescript"));
        assert_eq!(c.role, UniversalRole::Generated);
    }

    #[test]
    fn test_classify_generated_dir() {
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/__generated__/types.ts");
        let c = classify_path(abs, Some(root), Some("typescript"));
        assert_eq!(c.role, UniversalRole::Generated);
    }

    #[test]
    fn test_content_based_generated_detection() {
        assert!(is_generated_content(
            "// Code generated by protoc-gen-go. DO NOT EDIT.\npackage foo"
        ));
        assert!(is_generated_content("# @generated\nimport foo"));
        assert!(is_generated_content(
            "// @auto-generated by graphql-codegen\n"
        ));
        assert!(!is_generated_content(
            "// Regular source file\nfn main() {}"
        ));
    }

    #[test]
    fn test_classify_with_content_overrides() {
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/models.go");
        let c = classify_path_with_content(
            abs,
            Some(root),
            Some("go"),
            Some("// Code generated by sqlc. DO NOT EDIT.\npackage models"),
        );
        assert_eq!(c.role, UniversalRole::Generated);
        assert!(c.is_generated);
    }

    #[test]
    fn test_classify_swagger_generated() {
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/api.swagger.json");
        let c = classify_path(abs, Some(root), None);
        assert_eq!(c.role, UniversalRole::Generated);
    }

    #[test]
    fn staticresources_classifies_as_vendor() {
        // Raw file directly under staticresources/.
        let root = Path::new("/repo");
        let abs = Path::new("/repo/force-app/main/default/staticresources/jQuery.js");
        let c = classify_path(abs, Some(root), Some("javascript"));
        assert_eq!(c.role, UniversalRole::Vendor);
        assert!(c.is_vendor);
        assert!(c.reason.to_lowercase().contains("static-resource"));
    }

    #[test]
    fn staticresources_nested_classifies_as_vendor() {
        // NPSP stores many staticresources inside a bundle directory.
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/staticresources/CumulusStatic/js/bootstrap.min.js");
        let c = classify_path(abs, Some(root), Some("javascript"));
        assert_eq!(c.role, UniversalRole::Vendor);
        assert!(c.is_vendor);
    }

    #[test]
    fn staticresources_match_is_case_insensitive() {
        // `segments` is built from `lower_rel` (the lowercased path),
        // so any casing of the directory name classifies as Vendor.
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/StaticResources/foo.js");
        let c = classify_path(abs, Some(root), Some("javascript"));
        assert_eq!(c.role, UniversalRole::Vendor);
        assert!(c.is_vendor);
    }

    #[test]
    fn npsp_static_resource_sources_classifies_as_vendor() {
        // NPSP (and several other DX projects) keep unminified source
        // here and bundle it into `staticresources/` at build time.
        // The prefix match on `staticresource` catches this variant
        // without a one-off exception. Layer-5 hand-audit for Wave 1
        // found 10/10 `visibility_private_unused` samples from here.
        let root = Path::new("/repo");
        let abs = Path::new("/repo/StaticResourceSources/Bootstrap/js/bootstrap.min.js");
        let c = classify_path(abs, Some(root), Some("javascript"));
        assert_eq!(c.role, UniversalRole::Vendor);
        assert!(c.is_vendor);
    }

    #[test]
    fn static_resource_variants_all_classify_as_vendor() {
        let root = Path::new("/repo");
        for rel in [
            "staticresources/a.js",
            "StaticResources/a.js",
            "STATICRESOURCES/a.js",
            "StaticResourceSources/x/y.js",
            "staticresource_bundle/a.js",
        ] {
            let abs_str = format!("/repo/{}", rel);
            let abs = Path::new(&abs_str);
            let c = classify_path(abs, Some(root), Some("javascript"));
            assert_eq!(c.role, UniversalRole::Vendor, "expected Vendor for {}", rel);
        }
    }

    #[test]
    fn unrelated_resource_directories_are_not_classified_as_vendor_by_static_rule() {
        // Guard against prefix overmatch: a directory named `resources/`
        // or `StaticAnalysis/` must not hit the static-resource Vendor
        // arm (it may still be classified elsewhere).
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/resources/foo.js");
        let c = classify_path(abs, Some(root), Some("javascript"));
        assert!(
            !c.reason.to_lowercase().contains("static-resource"),
            "resources/ must not trigger static-resource Vendor rule, got reason: {}",
            c.reason
        );
    }

    #[test]
    fn apex_test_filename_suffix_classifies_as_test() {
        // `_TEST.cls` — uppercase convention used by NPSP/EDA.
        let root = Path::new("/repo");
        let abs = Path::new("/repo/force-app/main/default/classes/CON_ContactMergeTDTM_TEST.cls");
        let c = classify_path(abs, Some(root), Some("apex"));
        assert_eq!(c.role, UniversalRole::Test);
        assert!(c.is_test);
    }

    #[test]
    fn apex_tests_filename_suffix_classifies_as_test() {
        // `_TESTS.cls` — plural variant also seen in NPSP.
        let root = Path::new("/repo");
        let abs = Path::new("/repo/force-app/main/default/classes/ADDR_Validator_TESTS.cls");
        let c = classify_path(abs, Some(root), Some("apex"));
        assert_eq!(c.role, UniversalRole::Test);
        assert!(c.is_test);
    }

    #[test]
    fn apex_test_filename_suffix_is_case_insensitive() {
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/Foo_Test.cls");
        let c = classify_path(abs, Some(root), Some("apex"));
        assert_eq!(c.role, UniversalRole::Test);
        assert!(c.is_test);
    }

    #[test]
    fn apex_test_convention_only_fires_for_apex_language() {
        // A JS file named `_TEST.js` must NOT be hijacked by the Apex arm.
        // It can still hit the generic Test block if naming conventions apply,
        // but here the filename pattern alone should not match.
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/Foo_TEST.js");
        let c = classify_path(abs, Some(root), Some("javascript"));
        // Not under /test/, not .test.js — must not be classified as test.
        assert_ne!(c.role, UniversalRole::Test);
        assert!(!c.is_test);
    }

    #[test]
    fn apex_non_test_class_is_not_test() {
        // Regression: baseline `.cls` with no `_TEST` suffix must not be test.
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/CON_ContactMergeTDTM.cls");
        let c = classify_path(abs, Some(root), Some("apex"));
        assert!(!c.is_test);
    }

    #[test]
    fn apex_test_filename_with_trailing_digit_classifies_as_test() {
        // NPSP ships several companion fixtures named `FOO_TEST2.cls`,
        // `FOO_TEST3.cls`, etc. The Layer-5 hand-audit (Wave 1) caught
        // `CON_ContactMergeTDTM_TEST2.cls` methods leaking into
        // `dynamic_dispatch_target`; the parser rule must accept
        // optional trailing digits on the suffix.
        let root = Path::new("/repo");
        for file in [
            "CON_ContactMergeTDTM_TEST2.cls",
            "Foo_TEST3.cls",
            "Bar_Tests2.cls",
            "Baz_TEST42.cls",
        ] {
            let abs_str = format!("/repo/src/{}", file);
            let abs = Path::new(&abs_str);
            let c = classify_path(abs, Some(root), Some("apex"));
            assert!(c.is_test, "expected is_test=true for {}", file);
            assert_eq!(c.role, UniversalRole::Test, "file {}", file);
        }
    }

    #[test]
    fn apex_non_test_class_with_trailing_digit_is_not_test() {
        // Regression: the digit-suffix rule must not overmatch.
        // A class named `FooBar2.cls` (no `_TEST` infix) must stay
        // non-test.
        let root = Path::new("/repo");
        let abs = Path::new("/repo/src/FooBar2.cls");
        let c = classify_path(abs, Some(root), Some("apex"));
        assert!(!c.is_test);
    }

    #[test]
    fn typescript_view_root_density_prefers_src_typescript_over_deeper_leaf() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Mimic a TS project where most files live under src/typescript/lib.
        let files = [
            root.join("src/typescript/lib/a.ts"),
            root.join("src/typescript/lib/b.ts"),
            root.join("src/typescript/lib/c.ts"),
            root.join("src/typescript/lib/d.ts"),
            root.join("src/typescript/lib/e.ts"),
            root.join("src/typescript/lib/f.ts"),
        ];
        for p in &files {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, "export const x = 1;").unwrap();
        }

        // Ensure src/ exists so the generic heuristic would also add it; we want the density
        // heuristic to still surface src/typescript as a stronger default.
        std::fs::create_dir_all(root.join("src")).unwrap();

        let file_list = files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let roots = compute_recommended_view_roots(root, Some("typescript"), &file_list);
        assert!(
            roots
                .iter()
                .any(|c| c.root_path_repo_rel == "src/typescript"),
            "Expected src/typescript candidate; got: {:?}",
            roots
                .iter()
                .map(|c| &c.root_path_repo_rel)
                .collect::<Vec<_>>()
        );
    }
}
