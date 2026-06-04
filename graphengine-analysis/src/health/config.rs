//! Analysis configuration: ecosystem profiles, thresholds, and TOML overrides.
//!
//! Resolution order: `--config` TOML → ecosystem profile → engine defaults.
//! Every hardcoded threshold in the pipeline is sourced from this struct.

use serde::Deserialize;
use std::fmt;

// ---------------------------------------------------------------------------
// Ecosystem enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ecosystem {
    TypeScript,
    JavaScript,
    Rust,
    Java,
    Go,
    Python,
    #[serde(rename = "csharp")]
    CSharp,
    /// Salesforce Apex. Distinct from Java even though the parsing
    /// crate reuses Java grammar shape, because the framework
    /// dispatch model (@AuraEnabled, triggers, Queueable, TDTM) is
    /// nothing like Spring. The dead-code classifier registry must
    /// be able to route Apex to its dedicated module.
    Apex,
    Unknown,
}

impl Ecosystem {
    pub fn from_language_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "typescript" | "tsx" => Self::TypeScript,
            "javascript" | "jsx" => Self::JavaScript,
            "rust" => Self::Rust,
            "java" | "kotlin" => Self::Java,
            "go" | "golang" => Self::Go,
            "python" => Self::Python,
            "csharp" | "c#" | "cs" => Self::CSharp,
            "apex" | "apexcls" | "salesforce" => Self::Apex,
            _ => Self::Unknown,
        }
    }
}

impl fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeScript => write!(f, "typescript"),
            Self::JavaScript => write!(f, "javascript"),
            Self::Rust => write!(f, "rust"),
            Self::Java => write!(f, "java"),
            Self::Go => write!(f, "go"),
            Self::Python => write!(f, "python"),
            Self::CSharp => write!(f, "csharp"),
            Self::Apex => write!(f, "apex"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AnalysisConfig {
    pub ecosystem: Option<Ecosystem>,
    pub modules: ModuleConfig,
    pub thresholds: ThresholdConfig,
    pub dead_code: DeadCodeConfig,
    pub score_weights: ScoreWeights,
    pub max_findings_per_type: usize,
    #[serde(default)]
    pub classification_overrides: Option<ClassificationOverrides>,
    #[serde(default)]
    pub test_detection: TestDetectionConfig,
    /// When true, test files are excluded from all production metrics.
    #[serde(default)]
    pub exclude_tests: bool,
    /// When true, generated/vendor/build-output files are excluded.
    #[serde(default)]
    pub exclude_generated: bool,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self::for_ecosystem(Ecosystem::Unknown)
    }
}

// ---------------------------------------------------------------------------
// Test detection config
// ---------------------------------------------------------------------------

/// Configuration for test file detection overrides.
/// Allows users to extend the built-in framework and path heuristics.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TestDetectionConfig {
    /// Additional module imports that mark a file as test
    /// (e.g., "my_test_lib", "custom_assert")
    pub extra_framework_modules: Vec<String>,
    /// Paths to forcibly classify as production (overrides test detection)
    pub force_production_paths: Vec<String>,
    /// Paths to forcibly classify as test
    pub force_test_paths: Vec<String>,
    /// Additional filename glob patterns for test files
    /// (e.g., "*_spec.py", "check_*.go")
    pub extra_test_file_patterns: Vec<String>,
}

impl AnalysisConfig {
    /// Build a config pre-loaded with the defaults for a given ecosystem.
    pub fn for_ecosystem(eco: Ecosystem) -> Self {
        Self {
            ecosystem: Some(eco),
            modules: ModuleConfig::for_ecosystem(eco),
            thresholds: ThresholdConfig::for_ecosystem(eco),
            dead_code: DeadCodeConfig::for_ecosystem(eco),
            score_weights: ScoreWeights::default(),
            max_findings_per_type: 10,
            classification_overrides: None,
            test_detection: TestDetectionConfig::default(),
            exclude_tests: false,
            exclude_generated: false,
        }
    }

    /// Merge a (possibly partial) TOML-parsed config on top of `self`.
    /// Only fields explicitly present in `overlay` overwrite.
    /// Since serde `#[serde(default)]` fills missing fields with defaults,
    /// we use Option wrappers in the TOML layer and selectively apply.
    pub fn apply_toml_overlay(&mut self, overlay: TomlOverlay) {
        if let Some(eco) = overlay.ecosystem {
            self.ecosystem = Some(eco);
        }
        if let Some(m) = overlay.modules {
            if let Some(v) = m.analysis_depth {
                self.modules.analysis_depth = v;
            }
            if let Some(v) = m.min_module_size {
                self.modules.min_module_size = v;
            }
            if let Some(v) = m.exclude_test_modules_from_coupling {
                self.modules.exclude_test_modules_from_coupling = v;
            }
            if let Some(v) = m.strip_build_convention_dirs {
                self.modules.strip_build_convention_dirs = v;
            }
        }
        if let Some(t) = overlay.thresholds {
            t.apply_to(&mut self.thresholds);
        }
        if let Some(dc) = overlay.dead_code {
            dc.apply_to(&mut self.dead_code);
        }
        if let Some(sw) = overlay.score_weights {
            if let Some(v) = sw.cycle_severity {
                self.score_weights.cycle_severity = v;
            }
            if let Some(v) = sw.coupling_health {
                self.score_weights.coupling_health = v;
            }
            if let Some(v) = sw.hotspot_concentration {
                self.score_weights.hotspot_concentration = v;
            }
            if let Some(v) = sw.dead_code_ratio {
                self.score_weights.dead_code_ratio = v;
            }
            if let Some(v) = sw.depth_complexity {
                self.score_weights.depth_complexity = v;
            }
            if let Some(v) = sw.complexity {
                self.score_weights.complexity = v;
            }
            if let Some(v) = sw.cohesion {
                self.score_weights.cohesion = v;
            }
            if let Some(v) = sw.distance {
                self.score_weights.distance = v;
            }
            if let Some(v) = sw.temporal_coupling {
                self.score_weights.temporal_coupling = v;
            }
        }
        if let Some(v) = overlay.max_findings_per_type {
            self.max_findings_per_type = v;
        }
        if let Some(co) = overlay.classification {
            self.classification_overrides = Some(co);
        }
        if let Some(td) = overlay.test_detection {
            if let Some(v) = td.extra_framework_modules {
                self.test_detection.extra_framework_modules = v;
            }
            if let Some(v) = td.force_production_paths {
                self.test_detection.force_production_paths = v;
            }
            if let Some(v) = td.force_test_paths {
                self.test_detection.force_test_paths = v;
            }
            if let Some(v) = td.extra_test_file_patterns {
                self.test_detection.extra_test_file_patterns = v;
            }
        }
    }

    /// Validate the config, returning a list of errors (empty = valid).
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        let sum = self.score_weights.cycle_severity
            + self.score_weights.coupling_health
            + self.score_weights.hotspot_concentration
            + self.score_weights.dead_code_ratio
            + self.score_weights.depth_complexity
            + self.score_weights.complexity
            + self.score_weights.cohesion
            + self.score_weights.distance
            + self.score_weights.temporal_coupling;
        if (sum - 1.0).abs() > 0.001 {
            errors.push(format!("score_weights must sum to 1.0, got {sum:.4}"));
        }

        let pct_fields = [
            ("ifc_percentile", self.thresholds.ifc_percentile),
            (
                "ifc_severity_critical_percentile",
                self.thresholds.ifc_severity_critical_percentile,
            ),
            (
                "ifc_severity_high_percentile",
                self.thresholds.ifc_severity_high_percentile,
            ),
            ("hub_percentile", self.thresholds.hub_percentile),
            (
                "hub_severity_high_percentile",
                self.thresholds.hub_severity_high_percentile,
            ),
            (
                "hub_severity_warning_percentile",
                self.thresholds.hub_severity_warning_percentile,
            ),
            ("hotspot_percentile", self.thresholds.hotspot_percentile),
        ];
        for (name, val) in pct_fields {
            if !(1..=99).contains(&val) {
                errors.push(format!("{name} must be in [1, 99], got {val}"));
            }
        }

        if self.modules.analysis_depth == 0 {
            errors.push("modules.analysis_depth must be >= 1".to_string());
        }

        errors
    }

    /// Resolved ecosystem: explicit setting or Unknown.
    pub fn resolved_ecosystem(&self) -> Ecosystem {
        self.ecosystem.unwrap_or(Ecosystem::Unknown)
    }
}

// ---------------------------------------------------------------------------
// Module boundary config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ModuleConfig {
    pub analysis_depth: usize,
    pub min_module_size: usize,
    pub exclude_test_modules_from_coupling: bool,
    pub test_module_ratio: f64,
    /// Strip build-system convention directories (e.g. `src/main/java/`) from
    /// paths before computing analysis module boundaries. This prevents Maven/Gradle
    /// scaffolding from inflating module depth and distorting coupling metrics.
    pub strip_build_convention_dirs: bool,
}

impl Default for ModuleConfig {
    fn default() -> Self {
        Self {
            analysis_depth: 2,
            min_module_size: 3,
            exclude_test_modules_from_coupling: true,
            test_module_ratio: 0.5,
            strip_build_convention_dirs: false,
        }
    }
}

impl ModuleConfig {
    /// Ecosystem-aware defaults. Each language's typical project layout
    /// determines how many directory levels produce meaningful module boundaries.
    ///
    /// - Rust: depth 4 for `crate/src/module/submodule` workspace layouts
    /// - Go: depth 3 for `cmd/app/handler` and `internal/pkg` patterns
    /// - Python: depth 3 for `src/package/module` layouts (e.g. `src/click/core.py`)
    /// - TS/JS/Java/C#: depth 2 (typical `src/component` flat layouts)
    pub fn for_ecosystem(eco: Ecosystem) -> Self {
        let (depth, strip) = match eco {
            Ecosystem::Rust => (4, false),
            Ecosystem::Go | Ecosystem::Python => (3, false),
            // Java: strip `src/main/java/` etc., then depth=5 maps to package-level
            // (e.g. `gson/com/google/gson/internal` from multi-module Gradle projects)
            Ecosystem::Java => (5, true),
            // C#: `src/ProjectName/Namespace` at depth=3 gives meaningful boundaries
            Ecosystem::CSharp => (3, false),
            // Apex: SFDX layout `force-app/main/default/classes` — a single
            // flat classes directory. Depth 2 mirrors TS (path prefix of
            // the class file, which is effectively the module).
            Ecosystem::Apex => (2, false),
            _ => (2, false),
        };
        Self {
            analysis_depth: depth,
            strip_build_convention_dirs: strip,
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Finding threshold config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ThresholdConfig {
    // Coupling
    /// Expected average coupling for this ecosystem. Coupling at or below this
    /// value scores 100/100. Penalties apply only above it.
    pub coupling_baseline: f64,
    pub coupling_finding: f64,
    pub coupling_info: f64,
    pub coupling_warning: f64,
    pub coupling_high: f64,
    pub coupling_critical: f64,
    pub high_coupling_threshold: f64,

    // IFC
    pub ifc_percentile: usize,
    pub ifc_severity_critical_percentile: usize,
    pub ifc_severity_high_percentile: usize,
    pub ifc_floor: usize,
    pub ifc_floor_critical: usize,
    pub ifc_floor_high: usize,

    // Hub
    pub hub_percentile: usize,
    pub hub_severity_high_percentile: usize,
    pub hub_severity_warning_percentile: usize,
    pub hub_floor: usize,
    pub hub_floor_high: usize,
    pub hub_floor_warning: usize,

    // Depth
    pub depth_warning: usize,
    pub depth_high: usize,
    pub depth_critical: usize,

    // API surface
    pub api_surface_warning: f64,
    pub api_surface_high: f64,
    pub api_surface_critical: f64,

    // Hotspot
    pub hotspot_percentile: usize,
    pub hotspot_small_graph_threshold: usize,
    pub hotspot_small_graph_fixed: usize,

    // Cycle severity
    pub cycle_critical_length: usize,
    pub cycle_high_length: usize,

    // Complexity
    pub cyclomatic_critical: u32,
    pub cyclomatic_high: u32,
    pub cyclomatic_warning: u32,
    pub cognitive_critical: u32,
    pub cognitive_high: u32,
    pub cognitive_warning: u32,

    // Temporal coupling
    pub temporal_min_co_changes: usize,
    pub temporal_min_coupling_score: f64,
    pub temporal_hidden_high_score: f64,
    pub temporal_hidden_high_min_co_changes: usize,
    pub temporal_since_months: usize,
    pub temporal_max_files_per_commit: usize,

    // Cohesion (LCOM4): thresholds are *lower bounds* — below these is a finding
    pub cohesion_finding: f64,
    pub cohesion_critical: f64,
    pub cohesion_high: f64,

    // Layer violations: minimum layer gap to report as a finding
    pub layer_violation_min_gap: usize,
    pub layer_violation_critical_gap: usize,
    pub layer_violation_high_gap: usize,
    /// Percentile threshold for high-fan-in exclusion (1-99). Callees in the
    /// top (100 - N)% of fan-in are excluded from layer violation detection
    /// because they are universal utility functions (e.g. `.len()`, `.to_string()`).
    pub layer_violation_fan_in_exclude_percentile: usize,

    // God function detection (all three conditions must be met)
    pub god_function_cyclomatic_min: u32,
    pub god_function_fan_out_min: usize,
    pub god_function_loc_min: usize,
    pub god_function_cyclomatic_critical: u32,
    pub god_function_fan_out_critical: usize,
    pub god_function_loc_critical: usize,

    // ---- Metric-status contract ----
    //
    // Used to populate `MetricStatus` on each per-metric detail block in the
    // HealthReport. See graphengine-analysis/src/health/metric_status.rs.
    /// Minimum number of production structural edges required for
    /// cycle / tangle / depth metrics to be considered meaningful.
    /// Below this, the metrics are stamped `InsufficientEdges` and the
    /// raw numeric value is not safe to render as-is in product UIs.
    pub min_edges_for_cycle_metric: usize,

    /// Minimum number of production call edges (Call-kind only) required
    /// for max-call-depth to be meaningful. Depth is more sensitive to
    /// call-edge density than cycles are because a missing function-call
    /// edge truncates a chain entirely.
    pub min_call_edges_for_depth_metric: usize,

    /// Fraction of resolution that came from heuristic fallback (0.0-1.0)
    /// above which a metric is considered `FrameworkInvisible` when the
    /// ecosystem uses declarative wiring. Default 0.30 — if more than 30%
    /// of edges are name-matched in a Django/Rails/Apex/Spring codebase,
    /// cycle-family metrics are not trustworthy because the routing layer
    /// is almost certainly not being parsed.
    pub framework_invisible_fallback_rate: f64,
}

impl ThresholdConfig {
    /// Ecosystem-aware coupling thresholds. Different module depths produce
    /// structurally different coupling ratios — Rust at depth 4 naturally has
    /// higher external/total edge ratios than flat TS packages at depth 2.
    /// Baselines prevent penalizing expected architectural patterns.
    pub fn for_ecosystem(eco: Ecosystem) -> Self {
        let mut base = Self::default();
        match eco {
            Ecosystem::Rust => {
                base.coupling_baseline = 0.75;
                base.coupling_finding = 0.60;
                base.coupling_info = 0.60;
                base.coupling_warning = 0.75;
                base.coupling_high = 0.88;
                base.coupling_critical = 0.95;
                base.high_coupling_threshold = 0.88;
            }
            Ecosystem::Go | Ecosystem::Python => {
                base.coupling_baseline = 0.55;
                base.coupling_finding = 0.45;
                base.coupling_info = 0.45;
                base.coupling_warning = 0.60;
                base.coupling_high = 0.80;
                base.coupling_critical = 0.92;
                base.high_coupling_threshold = 0.80;
            }
            Ecosystem::TypeScript | Ecosystem::JavaScript | Ecosystem::CSharp => {
                base.coupling_baseline = 0.40;
                base.coupling_finding = 0.35;
                base.coupling_info = 0.35;
                base.coupling_warning = 0.55;
                base.coupling_high = 0.75;
                base.coupling_critical = 0.90;
                base.high_coupling_threshold = 0.75;
            }
            // Java: higher baseline because type-reference edges are pervasive
            // in Java's nominally-typed system. Method signatures, generics,
            // and annotations all produce cross-module type edges.
            Ecosystem::Java => {
                base.coupling_baseline = 0.65;
                base.coupling_finding = 0.55;
                base.coupling_info = 0.55;
                base.coupling_warning = 0.70;
                base.coupling_high = 0.85;
                base.coupling_critical = 0.95;
                base.high_coupling_threshold = 0.85;
            }
            // Apex: Java-family coupling characteristics — strong
            // static typing, inheritance-heavy, annotation-driven
            // dispatch — so inherit Java's baseline.
            Ecosystem::Apex => {
                base.coupling_baseline = 0.65;
                base.coupling_finding = 0.55;
                base.coupling_info = 0.55;
                base.coupling_warning = 0.70;
                base.coupling_high = 0.85;
                base.coupling_critical = 0.95;
                base.high_coupling_threshold = 0.85;
            }
            Ecosystem::Unknown => {} // keep defaults
        }
        base
    }
}

impl Default for ThresholdConfig {
    fn default() -> Self {
        Self {
            coupling_baseline: 0.35,
            coupling_finding: 0.3,
            coupling_info: 0.3,
            coupling_warning: 0.5,
            coupling_high: 0.7,
            coupling_critical: 0.85,
            high_coupling_threshold: 0.7,

            ifc_percentile: 95,
            ifc_severity_critical_percentile: 99,
            ifc_severity_high_percentile: 97,
            ifc_floor: 100,
            ifc_floor_critical: 400,
            ifc_floor_high: 200,

            hub_percentile: 95,
            hub_severity_high_percentile: 99,
            hub_severity_warning_percentile: 97,
            hub_floor: 4,
            hub_floor_high: 6,
            hub_floor_warning: 5,

            depth_warning: 10,
            depth_high: 15,
            depth_critical: 20,

            api_surface_warning: 0.6,
            api_surface_high: 0.75,
            api_surface_critical: 0.9,

            hotspot_percentile: 95,
            hotspot_small_graph_threshold: 20,
            hotspot_small_graph_fixed: 8,

            cycle_critical_length: 4,
            cycle_high_length: 3,

            cyclomatic_critical: 25,
            cyclomatic_high: 15,
            cyclomatic_warning: 10,
            cognitive_critical: 30,
            cognitive_high: 20,
            cognitive_warning: 12,

            temporal_min_co_changes: 3,
            temporal_min_coupling_score: 0.5,
            temporal_hidden_high_score: 0.8,
            temporal_hidden_high_min_co_changes: 5,
            temporal_since_months: 6,
            temporal_max_files_per_commit: 50,

            cohesion_finding: 0.50,
            cohesion_critical: 0.20,
            cohesion_high: 0.34,

            layer_violation_min_gap: 2,
            layer_violation_critical_gap: 4,
            layer_violation_high_gap: 3,
            layer_violation_fan_in_exclude_percentile: 95,

            god_function_cyclomatic_min: 10,
            god_function_fan_out_min: 8,
            god_function_loc_min: 40,
            god_function_cyclomatic_critical: 20,
            god_function_fan_out_critical: 15,
            god_function_loc_critical: 80,

            // Metric-status contract defaults — conservative, tuned to
            // suppress false "0 cycles" claims on sparse call graphs.
            min_edges_for_cycle_metric: 50,
            min_call_edges_for_depth_metric: 20,
            framework_invisible_fallback_rate: 0.30,
        }
    }
}

// ---------------------------------------------------------------------------
// Dead code heuristic config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DeadCodeConfig {
    // Universal (always on)
    pub barrel_files: bool,
    pub entrypoint_files: bool,
    pub framework_handlers: bool,
    pub lifecycle_methods: bool,
    // Ecosystem-gated
    pub jsx_components: bool,
    pub jsx_runtime: bool,
    pub jsx_intrinsic: bool,
    pub property_accessors: bool,
    pub mock_functions: bool,
    pub unity_lifecycle: bool,
    pub spring_annotations: bool,
    pub trait_impls: bool,
    pub exported_symbols: bool,
    pub extra_entry_point_patterns: Vec<String>,
}

impl Default for DeadCodeConfig {
    fn default() -> Self {
        Self::for_ecosystem(Ecosystem::Unknown)
    }
}

impl DeadCodeConfig {
    pub fn for_ecosystem(eco: Ecosystem) -> Self {
        let universal = Self {
            barrel_files: true,
            entrypoint_files: true,
            framework_handlers: true,
            lifecycle_methods: true,
            jsx_components: false,
            jsx_runtime: false,
            jsx_intrinsic: false,
            property_accessors: false,
            mock_functions: false,
            unity_lifecycle: false,
            spring_annotations: false,
            trait_impls: false,
            exported_symbols: true,
            extra_entry_point_patterns: Vec::new(),
        };

        match eco {
            Ecosystem::TypeScript | Ecosystem::JavaScript => Self {
                jsx_components: true,
                jsx_runtime: true,
                jsx_intrinsic: true,
                property_accessors: true,
                mock_functions: true,
                ..universal
            },
            Ecosystem::Rust => Self {
                trait_impls: true,
                ..universal
            },
            Ecosystem::Java => Self {
                spring_annotations: true,
                ..universal
            },
            Ecosystem::Python => Self {
                property_accessors: true,
                mock_functions: true,
                ..universal
            },
            Ecosystem::CSharp => Self {
                unity_lifecycle: true,
                ..universal
            },
            // Apex: dead-code entry-point rules are driven almost
            // entirely by the Apex-specific classifier (annotations,
            // trigger handlers, global, webservice, implements
            // Schedulable/Batchable/Queueable). Here we keep only the
            // universal rules; the classifier module layers on the
            // Apex-specific signals from the node's properties.
            Ecosystem::Apex => universal,
            Ecosystem::Go | Ecosystem::Unknown => Self {
                // Unknown uses all heuristics (most conservative)
                jsx_components: eco == Ecosystem::Unknown,
                jsx_runtime: eco == Ecosystem::Unknown,
                jsx_intrinsic: eco == Ecosystem::Unknown,
                property_accessors: eco == Ecosystem::Unknown,
                mock_functions: eco == Ecosystem::Unknown,
                ..universal
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Score weights
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScoreWeights {
    pub cycle_severity: f64,
    pub coupling_health: f64,
    pub hotspot_concentration: f64,
    pub dead_code_ratio: f64,
    pub depth_complexity: f64,
    pub complexity: f64,
    pub cohesion: f64,
    pub distance: f64,
    pub temporal_coupling: f64,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            cycle_severity: 0.20,
            coupling_health: 0.18,
            hotspot_concentration: 0.15,
            dead_code_ratio: 0.10,
            depth_complexity: 0.10,
            complexity: 0.10,
            cohesion: 0.07,
            distance: 0.05,
            temporal_coupling: 0.05,
        }
    }
}

impl ScoreWeights {
    /// When `--git-dir` is absent, temporal coupling cannot be computed.
    /// Redistribute its weight proportionally among the remaining components.
    pub fn without_temporal(&self) -> ScoreWeights {
        let non_temporal = self.cycle_severity
            + self.coupling_health
            + self.hotspot_concentration
            + self.dead_code_ratio
            + self.depth_complexity
            + self.complexity
            + self.cohesion
            + self.distance;
        if non_temporal <= 0.0 {
            return self.clone();
        }
        let factor = 1.0 / non_temporal;
        ScoreWeights {
            cycle_severity: self.cycle_severity * factor,
            coupling_health: self.coupling_health * factor,
            hotspot_concentration: self.hotspot_concentration * factor,
            dead_code_ratio: self.dead_code_ratio * factor,
            depth_complexity: self.depth_complexity * factor,
            complexity: self.complexity * factor,
            cohesion: self.cohesion * factor,
            distance: self.distance * factor,
            temporal_coupling: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Classification overrides
// ---------------------------------------------------------------------------

/// User-provided path overrides for production vs. non-production classification.
/// Paths are prefix-matched against module keys. When present, these override
/// any heuristic or structural detection.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ClassificationOverrides {
    pub production_paths: Vec<String>,
    pub non_production_paths: Vec<String>,
}

// ---------------------------------------------------------------------------
// TOML overlay (partial deserialization for --config)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TomlOverlay {
    pub ecosystem: Option<Ecosystem>,
    pub modules: Option<TomlModuleOverlay>,
    pub thresholds: Option<TomlThresholdOverlay>,
    pub dead_code: Option<TomlDeadCodeOverlay>,
    pub score_weights: Option<TomlScoreWeightsOverlay>,
    pub max_findings_per_type: Option<usize>,
    pub classification: Option<ClassificationOverrides>,
    pub test_detection: Option<TomlTestDetectionOverlay>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TomlTestDetectionOverlay {
    pub extra_framework_modules: Option<Vec<String>>,
    pub force_production_paths: Option<Vec<String>>,
    pub force_test_paths: Option<Vec<String>>,
    pub extra_test_file_patterns: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TomlModuleOverlay {
    pub analysis_depth: Option<usize>,
    pub min_module_size: Option<usize>,
    pub exclude_test_modules_from_coupling: Option<bool>,
    pub strip_build_convention_dirs: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TomlThresholdOverlay {
    pub coupling_baseline: Option<f64>,
    pub coupling_finding: Option<f64>,
    pub coupling_info: Option<f64>,
    pub coupling_warning: Option<f64>,
    pub coupling_high: Option<f64>,
    pub coupling_critical: Option<f64>,
    pub ifc_percentile: Option<usize>,
    pub ifc_severity_critical_percentile: Option<usize>,
    pub ifc_severity_high_percentile: Option<usize>,
    pub ifc_floor: Option<usize>,
    pub hub_percentile: Option<usize>,
    pub hub_severity_high_percentile: Option<usize>,
    pub hub_severity_warning_percentile: Option<usize>,
    pub hub_floor: Option<usize>,
    pub depth_warning: Option<usize>,
    pub depth_high: Option<usize>,
    pub depth_critical: Option<usize>,
    pub api_surface_warning: Option<f64>,
    pub api_surface_high: Option<f64>,
    pub api_surface_critical: Option<f64>,
    pub hotspot_percentile: Option<usize>,
    pub hotspot_small_graph_threshold: Option<usize>,
    pub hotspot_small_graph_fixed: Option<usize>,
    pub cyclomatic_critical: Option<u32>,
    pub cyclomatic_high: Option<u32>,
    pub cyclomatic_warning: Option<u32>,
    pub cognitive_critical: Option<u32>,
    pub cognitive_high: Option<u32>,
    pub cognitive_warning: Option<u32>,
    pub temporal_min_co_changes: Option<usize>,
    pub temporal_min_coupling_score: Option<f64>,
    pub temporal_hidden_high_score: Option<f64>,
    pub temporal_hidden_high_min_co_changes: Option<usize>,
    pub temporal_since_months: Option<usize>,
    pub temporal_max_files_per_commit: Option<usize>,
}

impl TomlThresholdOverlay {
    fn apply_to(self, target: &mut ThresholdConfig) {
        macro_rules! apply {
            ($field:ident) => {
                if let Some(v) = self.$field {
                    target.$field = v;
                }
            };
        }
        apply!(coupling_baseline);
        apply!(coupling_finding);
        apply!(coupling_info);
        apply!(coupling_warning);
        apply!(coupling_high);
        apply!(coupling_critical);
        apply!(ifc_percentile);
        apply!(ifc_severity_critical_percentile);
        apply!(ifc_severity_high_percentile);
        apply!(ifc_floor);
        apply!(hub_percentile);
        apply!(hub_severity_high_percentile);
        apply!(hub_severity_warning_percentile);
        apply!(hub_floor);
        apply!(depth_warning);
        apply!(depth_high);
        apply!(depth_critical);
        apply!(api_surface_warning);
        apply!(api_surface_high);
        apply!(api_surface_critical);
        apply!(hotspot_percentile);
        apply!(hotspot_small_graph_threshold);
        apply!(hotspot_small_graph_fixed);
        apply!(cyclomatic_critical);
        apply!(cyclomatic_high);
        apply!(cyclomatic_warning);
        apply!(cognitive_critical);
        apply!(cognitive_high);
        apply!(cognitive_warning);
        apply!(temporal_min_co_changes);
        apply!(temporal_min_coupling_score);
        apply!(temporal_hidden_high_score);
        apply!(temporal_hidden_high_min_co_changes);
        apply!(temporal_since_months);
        apply!(temporal_max_files_per_commit);
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TomlDeadCodeOverlay {
    pub barrel_files: Option<bool>,
    pub entrypoint_files: Option<bool>,
    pub framework_handlers: Option<bool>,
    pub lifecycle_methods: Option<bool>,
    pub jsx_components: Option<bool>,
    pub jsx_runtime: Option<bool>,
    pub jsx_intrinsic: Option<bool>,
    pub property_accessors: Option<bool>,
    pub mock_functions: Option<bool>,
    pub unity_lifecycle: Option<bool>,
    pub spring_annotations: Option<bool>,
    pub trait_impls: Option<bool>,
    pub extra_entry_point_patterns: Option<Vec<String>>,
}

impl TomlDeadCodeOverlay {
    fn apply_to(self, target: &mut DeadCodeConfig) {
        macro_rules! apply {
            ($field:ident) => {
                if let Some(v) = self.$field {
                    target.$field = v;
                }
            };
        }
        apply!(barrel_files);
        apply!(entrypoint_files);
        apply!(framework_handlers);
        apply!(lifecycle_methods);
        apply!(jsx_components);
        apply!(jsx_runtime);
        apply!(jsx_intrinsic);
        apply!(property_accessors);
        apply!(mock_functions);
        apply!(unity_lifecycle);
        apply!(spring_annotations);
        apply!(trait_impls);
        if let Some(v) = self.extra_entry_point_patterns {
            target.extra_entry_point_patterns = v;
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TomlScoreWeightsOverlay {
    pub cycle_severity: Option<f64>,
    pub coupling_health: Option<f64>,
    pub hotspot_concentration: Option<f64>,
    pub dead_code_ratio: Option<f64>,
    pub depth_complexity: Option<f64>,
    pub complexity: Option<f64>,
    pub cohesion: Option<f64>,
    pub distance: Option<f64>,
    pub temporal_coupling: Option<f64>,
}

// ---------------------------------------------------------------------------
// TOML loading
// ---------------------------------------------------------------------------

/// Parse a TOML config file and merge it onto an ecosystem profile.
pub fn load_config_from_toml(
    toml_path: &str,
    base_ecosystem: Ecosystem,
) -> Result<AnalysisConfig, String> {
    let content = std::fs::read_to_string(toml_path)
        .map_err(|e| format!("Failed to read config file '{}': {}", toml_path, e))?;

    let overlay: TomlOverlay =
        toml::from_str(&content).map_err(|e| format!("Failed to parse config TOML: {}", e))?;

    let effective_eco = overlay.ecosystem.unwrap_or(base_ecosystem);
    let mut config = AnalysisConfig::for_ecosystem(effective_eco);
    config.apply_toml_overlay(overlay);

    let errors = config.validate();
    if !errors.is_empty() {
        return Err(format!(
            "Config validation failed:\n  {}",
            errors.join("\n  ")
        ));
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let cfg = AnalysisConfig::default();
        assert!(cfg.validate().is_empty());
    }

    #[test]
    fn all_ecosystem_profiles_are_valid() {
        for eco in [
            Ecosystem::TypeScript,
            Ecosystem::JavaScript,
            Ecosystem::Rust,
            Ecosystem::Java,
            Ecosystem::Go,
            Ecosystem::Python,
            Ecosystem::CSharp,
            Ecosystem::Unknown,
        ] {
            let cfg = AnalysisConfig::for_ecosystem(eco);
            let errors = cfg.validate();
            assert!(errors.is_empty(), "Profile {:?} invalid: {:?}", eco, errors);
        }
    }

    #[test]
    fn typescript_profile_enables_jsx_heuristics() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::TypeScript);
        assert!(cfg.dead_code.jsx_components);
        assert!(cfg.dead_code.jsx_runtime);
        assert!(cfg.dead_code.jsx_intrinsic);
        assert!(cfg.dead_code.property_accessors);
    }

    #[test]
    fn rust_profile_disables_jsx_heuristics() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::Rust);
        assert!(!cfg.dead_code.jsx_components);
        assert!(!cfg.dead_code.jsx_runtime);
        assert!(!cfg.dead_code.jsx_intrinsic);
        assert!(cfg.dead_code.trait_impls);
    }

    #[test]
    fn rust_profile_uses_depth_4() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::Rust);
        assert_eq!(cfg.modules.analysis_depth, 4);
    }

    #[test]
    fn go_profile_uses_depth_3() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::Go);
        assert_eq!(cfg.modules.analysis_depth, 3);
    }

    #[test]
    fn python_profile_uses_depth_3() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::Python);
        assert_eq!(cfg.modules.analysis_depth, 3);
    }

    #[test]
    fn typescript_profile_uses_depth_2() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::TypeScript);
        assert_eq!(cfg.modules.analysis_depth, 2);
    }

    #[test]
    fn java_profile_uses_depth_5_with_strip() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::Java);
        assert_eq!(cfg.modules.analysis_depth, 5);
        assert!(cfg.modules.strip_build_convention_dirs);
    }

    #[test]
    fn csharp_profile_uses_depth_3() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::CSharp);
        assert_eq!(cfg.modules.analysis_depth, 3);
        assert!(!cfg.modules.strip_build_convention_dirs);
    }

    #[test]
    fn go_profile_disables_all_ecosystem_specific() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::Go);
        assert!(!cfg.dead_code.jsx_components);
        assert!(!cfg.dead_code.unity_lifecycle);
        assert!(!cfg.dead_code.spring_annotations);
        assert!(!cfg.dead_code.trait_impls);
    }

    #[test]
    fn rust_profile_has_relaxed_coupling_thresholds() {
        let cfg = AnalysisConfig::for_ecosystem(Ecosystem::Rust);
        assert!(
            cfg.thresholds.coupling_baseline > 0.5,
            "Rust baseline should be >0.50, got {}",
            cfg.thresholds.coupling_baseline
        );
        assert!(
            cfg.thresholds.high_coupling_threshold > 0.7,
            "Rust high_coupling_threshold should be >0.70, got {}",
            cfg.thresholds.high_coupling_threshold
        );
        assert!(
            cfg.thresholds.coupling_finding > 0.3,
            "Rust coupling_finding should be >0.30, got {}",
            cfg.thresholds.coupling_finding
        );
    }

    #[test]
    fn typescript_profile_has_stricter_coupling_thresholds() {
        let ts_cfg = AnalysisConfig::for_ecosystem(Ecosystem::TypeScript);
        let rust_cfg = AnalysisConfig::for_ecosystem(Ecosystem::Rust);
        assert!(
            ts_cfg.thresholds.coupling_baseline < rust_cfg.thresholds.coupling_baseline,
            "TS baseline ({}) should be lower than Rust ({})",
            ts_cfg.thresholds.coupling_baseline,
            rust_cfg.thresholds.coupling_baseline
        );
    }

    #[test]
    fn invalid_weights_detected() {
        let mut cfg = AnalysisConfig::default();
        cfg.score_weights.cycle_severity = 0.5;
        let errors = cfg.validate();
        assert!(!errors.is_empty());
        assert!(errors[0].contains("sum to 1.0"));
    }

    #[test]
    fn invalid_percentile_detected() {
        let mut cfg = AnalysisConfig::default();
        cfg.thresholds.ifc_percentile = 100;
        let errors = cfg.validate();
        assert!(!errors.is_empty());
    }

    #[test]
    fn toml_partial_overlay() {
        let toml_str = r#"
ecosystem = "rust"

[thresholds]
depth_warning = 15
depth_high = 25
"#;
        let overlay: TomlOverlay = toml::from_str(toml_str).unwrap();
        let mut cfg = AnalysisConfig::for_ecosystem(Ecosystem::TypeScript);
        cfg.apply_toml_overlay(overlay);

        assert_eq!(cfg.ecosystem, Some(Ecosystem::Rust));
        assert_eq!(cfg.thresholds.depth_warning, 15);
        assert_eq!(cfg.thresholds.depth_high, 25);
        // Unset fields retain the original value
        assert_eq!(cfg.thresholds.depth_critical, 20);
    }

    #[test]
    fn ecosystem_from_language_str() {
        assert_eq!(
            Ecosystem::from_language_str("typescript"),
            Ecosystem::TypeScript
        );
        assert_eq!(Ecosystem::from_language_str("Rust"), Ecosystem::Rust);
        assert_eq!(Ecosystem::from_language_str("golang"), Ecosystem::Go);
        assert_eq!(Ecosystem::from_language_str("C#"), Ecosystem::CSharp);
        assert_eq!(Ecosystem::from_language_str("obscure"), Ecosystem::Unknown);
    }
}
