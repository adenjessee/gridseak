//! Resolved LSP runtime policy for a single parse invocation.
//!
//! Replaces scattered `GRIDSEAK_LSP_PROFILE` / `patient_profile_*` reads
//! with one struct built once per `graphengine-parsing parse` run and
//! consulted by `SimpleLspClient`, `DocumentSyncManager`, and (in later
//! phases) per-language readiness wiring.
//!
//! # Precedence
//!
//! 1. Per-field env overrides (`GRAPHENGINE_LSP_REQUEST_TIMEOUT_MS`,
//!    `GRAPHENGINE_LSP_PYTHON_REQUEST_TIMEOUT_MS`, …).
//! 2. Effective policy tier (`GRIDSEAK_LSP_PROFILE` env aliases
//!    `--lsp-policy`; env wins when both are set).
//! 3. Policy-tier defaults (`patient` / `exhaustive` knobs).
//! 4. Language YAML (`lsp_request_timeout_ms`, …).
//! 5. Built-in fast-policy fallbacks.

use std::env;
use std::sync::OnceLock;
use std::time::Duration;

use crate::infrastructure::config::LanguageConfig;
use crate::infrastructure::lsp::session::ReadinessStrategy;

/// Scan-time LSP effort tier. `fast` preserves today's shipping behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LspPolicy {
    #[default]
    Fast,
    Patient,
    Exhaustive,
}

impl LspPolicy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "fast" => Some(Self::Fast),
            "patient" => Some(Self::Patient),
            "exhaustive" => Some(Self::Exhaustive),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Patient => "patient",
            Self::Exhaustive => "exhaustive",
        }
    }

    /// Whether this tier applies the longer-timeout / lower-concurrency
    /// patient defaults. `exhaustive` maps to the same runtime knobs as
    /// `patient` until resumable work queues land in a later phase.
    pub const fn uses_patient_defaults(self) -> bool {
        matches!(self, Self::Patient | Self::Exhaustive)
    }
}

/// Fully resolved knobs for one language parse.
#[derive(Debug, Clone, PartialEq)]
pub struct LspRuntimePolicy {
    pub tier: LspPolicy,
    pub request_timeout_ms: u32,
    pub max_concurrent_requests: u32,
    pub index_wait: Duration,
    pub document_settle: Duration,
    pub chunk_size: usize,
    /// Session readiness strategy. Per-language wiring lives in
    /// [`super::session_options::build_session_options`]; Apex uses
    /// its own builder. PR 2 keeps `Immediate` here as the default
    /// until `build_session_options` applies `ProgressAndProbe`.
    pub readiness: ReadinessStrategy,
}

static INSTALLED_POLICY: OnceLock<LspRuntimePolicy> = OnceLock::new();

/// Install the policy for the current parse subprocess. Called once from
/// the `parse` CLI handler after loading the language config.
pub fn install_runtime_policy(policy: LspRuntimePolicy) -> Result<(), &'static str> {
    INSTALLED_POLICY
        .set(policy)
        .map_err(|_| "LSP runtime policy already installed for this process")
}

/// Active policy for this process. Production parses always call
/// [`install_runtime_policy`] first; unit tests that skip installation
/// get deterministic fast-policy defaults.
pub fn runtime_policy() -> &'static LspRuntimePolicy {
    static FAST_FALLBACK: OnceLock<LspRuntimePolicy> = OnceLock::new();
    INSTALLED_POLICY.get().unwrap_or_else(|| {
        FAST_FALLBACK.get_or_init(|| LspRuntimePolicy {
            tier: LspPolicy::Fast,
            request_timeout_ms: 5_000,
            max_concurrent_requests: 32,
            index_wait: Duration::from_secs(5),
            document_settle: Duration::ZERO,
            chunk_size: 32,
            readiness: ReadinessStrategy::Immediate,
        })
    })
}

impl LspRuntimePolicy {
    pub fn resolve(cli_policy: Option<LspPolicy>, config: &LanguageConfig) -> Self {
        let tier = effective_policy_tier(cli_policy);
        let request_timeout_ms = env_override_u32(&config.language, "REQUEST_TIMEOUT_MS")
            .or_else(|| policy_default_u32(tier, 30_000))
            .or(config.lsp_request_timeout_ms)
            .unwrap_or(5_000);

        let max_concurrent_requests = env_override_u32(&config.language, "MAX_CONCURRENT_REQUESTS")
            .or_else(|| policy_default_u32(tier, 4))
            .or(config.lsp_max_concurrent_requests)
            .unwrap_or(32)
            .max(1);

        let index_wait = env_duration_ms("GRAPHENGINE_LSP_INDEX_WAIT_MS")
            .or_else(|| policy_default_duration(tier, 30_000))
            .unwrap_or_else(|| Duration::from_secs(5));

        let document_settle = env_duration_ms("GRAPHENGINE_LSP_DOCUMENT_SETTLE_MS")
            .or_else(|| policy_default_duration(tier, 5_000))
            .unwrap_or_default();

        let chunk_size = env_usize("GRAPHENGINE_LSP_CHUNK_SIZE")
            .or_else(|| policy_default_usize(tier, 4))
            .unwrap_or(32)
            .max(1);

        Self {
            tier,
            request_timeout_ms,
            max_concurrent_requests,
            index_wait,
            document_settle,
            chunk_size,
            readiness: ReadinessStrategy::Immediate,
        }
    }
}

/// Effective policy tier: `GRIDSEAK_LSP_PROFILE` env beats the CLI flag.
fn effective_policy_tier(cli_policy: Option<LspPolicy>) -> LspPolicy {
    if let Ok(value) = env::var("GRIDSEAK_LSP_PROFILE") {
        if let Some(policy) = LspPolicy::parse(&value) {
            return policy;
        }
    }
    cli_policy.unwrap_or(LspPolicy::Fast)
}

fn policy_default_u32(tier: LspPolicy, patient_value: u32) -> Option<u32> {
    tier.uses_patient_defaults().then_some(patient_value)
}

fn policy_default_duration(tier: LspPolicy, patient_ms: u64) -> Option<Duration> {
    tier.uses_patient_defaults()
        .then(|| Duration::from_millis(patient_ms))
}

fn policy_default_usize(tier: LspPolicy, patient_value: usize) -> Option<usize> {
    tier.uses_patient_defaults().then_some(patient_value)
}

fn env_override_u32(language: &str, suffix: &str) -> Option<u32> {
    let language_key = language.to_ascii_uppercase().replace('-', "_");
    let language_var = format!("GRAPHENGINE_LSP_{language_key}_{suffix}");
    env_u32(&language_var).or_else(|| env_u32(&format!("GRAPHENGINE_LSP_{suffix}")))
}

fn env_u32(name: &str) -> Option<u32> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
}

fn env_duration_ms(name: &str) -> Option<Duration> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
}

fn env_usize(name: &str) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    fn env_mutex() -> &'static Mutex<()> {
        static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_MUTEX.get_or_init(|| Mutex::new(()))
    }

    fn sample_config() -> LanguageConfig {
        let mut queries = HashMap::new();
        queries.insert("functions".into(), "dummy".into());
        queries.insert("structs".into(), "dummy".into());
        queries.insert("modules".into(), "dummy".into());
        queries.insert("call_sites".into(), "dummy".into());
        LanguageConfig {
            language: "python".into(),
            file_extensions: vec![".py".into()],
            queries,
            kind_mappings: HashMap::new(),
            grammar_path: None,
            lsp_command: Some("pyright-langserver".into()),
            lsp_args: Some(vec!["--stdio".into()]),
            version: "1.0".into(),
            receiver_type_detection: None,
            lsp_request_timeout_ms: Some(5_000),
            lsp_max_concurrent_requests: Some(32),
            lsp_initialization_options: None,
        }
    }

    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn new(vars: &[(&'static str, Option<&str>)]) -> Self {
            let saved: Vec<_> = vars.iter().map(|(k, _)| (*k, env::var(k).ok())).collect();
            for (k, v) in vars {
                match v {
                    Some(val) => env::set_var(k, val),
                    None => env::remove_var(k),
                }
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in self.saved.iter().rev() {
                match v {
                    Some(val) => env::set_var(k, val),
                    None => env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn fast_policy_uses_yaml_defaults() {
        let _guard = env_mutex().lock().unwrap();
        let _env = EnvGuard::new(&[
            ("GRIDSEAK_LSP_PROFILE", None),
            ("GRAPHENGINE_LSP_REQUEST_TIMEOUT_MS", None),
        ]);
        let config = sample_config();
        let policy = LspRuntimePolicy::resolve(Some(LspPolicy::Fast), &config);
        assert_eq!(policy.tier, LspPolicy::Fast);
        assert_eq!(policy.request_timeout_ms, 5_000);
        assert_eq!(policy.max_concurrent_requests, 32);
        assert_eq!(policy.index_wait, Duration::from_secs(5));
        assert!(policy.document_settle.is_zero());
        assert_eq!(policy.chunk_size, 32);
    }

    #[test]
    fn patient_policy_applies_longer_defaults() {
        let _guard = env_mutex().lock().unwrap();
        let _env = EnvGuard::new(&[
            ("GRIDSEAK_LSP_PROFILE", None),
            ("GRAPHENGINE_LSP_REQUEST_TIMEOUT_MS", None),
        ]);
        let config = sample_config();
        let policy = LspRuntimePolicy::resolve(Some(LspPolicy::Patient), &config);
        assert_eq!(policy.tier, LspPolicy::Patient);
        assert_eq!(policy.request_timeout_ms, 30_000);
        assert_eq!(policy.max_concurrent_requests, 4);
        assert_eq!(policy.index_wait, Duration::from_secs(30));
        assert_eq!(policy.document_settle, Duration::from_secs(5));
        assert_eq!(policy.chunk_size, 4);
    }

    #[test]
    fn env_profile_beats_cli_flag() {
        let _guard = env_mutex().lock().unwrap();
        let _env = EnvGuard::new(&[("GRIDSEAK_LSP_PROFILE", Some("patient"))]);
        let config = sample_config();
        let policy = LspRuntimePolicy::resolve(Some(LspPolicy::Fast), &config);
        assert_eq!(policy.tier, LspPolicy::Patient);
        assert_eq!(policy.request_timeout_ms, 30_000);
    }

    #[test]
    fn per_field_env_override_beats_policy_tier() {
        let _guard = env_mutex().lock().unwrap();
        let _env = EnvGuard::new(&[
            ("GRIDSEAK_LSP_PROFILE", None),
            ("GRAPHENGINE_LSP_REQUEST_TIMEOUT_MS", Some("12000")),
        ]);
        let config = sample_config();
        let policy = LspRuntimePolicy::resolve(Some(LspPolicy::Patient), &config);
        assert_eq!(policy.request_timeout_ms, 12_000);
    }

    #[test]
    fn exhaustive_maps_to_patient_runtime_knobs() {
        let _guard = env_mutex().lock().unwrap();
        let _env = EnvGuard::new(&[("GRIDSEAK_LSP_PROFILE", None)]);
        let config = sample_config();
        let policy = LspRuntimePolicy::resolve(Some(LspPolicy::Exhaustive), &config);
        assert_eq!(policy.tier, LspPolicy::Exhaustive);
        assert_eq!(policy.chunk_size, 4);
        assert_eq!(policy.request_timeout_ms, 30_000);
    }
}
