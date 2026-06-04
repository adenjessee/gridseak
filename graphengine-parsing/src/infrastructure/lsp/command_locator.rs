//! Resolve LSP server commands based on configuration and environment overrides.

use crate::infrastructure::config::LanguageConfig;
use crate::infrastructure::lsp::errors::LspError;
use std::env;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Placeholder token that `apex.yaml` carries in its `lsp_args`. The Apex
/// branch of [`resolve_lsp_command`] rewrites each occurrence of this token to
/// the resolved absolute path of `apex-jorje-lsp.jar`.
///
/// The placeholder pattern keeps the YAML declarative (no `${...}` expansion
/// logic in the config loader) while centralizing platform-specific JAR path
/// resolution here. Only Apex uses this mechanism today.
pub const APEX_JORJE_JAR_PLACEHOLDER: &str = "APEX_JORJE_JAR_PLACEHOLDER";

/// Indicates where the executable path was sourced from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSource {
    /// Resolved from an environment variable override (e.g. `GRAPHENGINE_LSP_RUST`).
    Environment(String),
    /// Resolved from the language configuration file.
    Config,
}

/// Fully resolved LSP command including executable and arguments.
#[derive(Debug, Clone)]
pub struct ResolvedLspCommand {
    /// Absolute path to the executable that will be launched.
    pub executable: PathBuf,
    /// Complete command (executable + args) suitable for spawning.
    pub command: Vec<String>,
    /// Additional arguments (excluding the executable itself).
    pub args: Vec<String>,
    /// Origin of the executable path.
    pub source: CommandSource,
}

/// Resolve the LSP command for the supplied language configuration.
///
/// Resolution order:
/// 1. If `GRAPHENGINE_LSP_<LANG>` is set, use its value verbatim as the full
///    command (highest-precedence override; unchanged behavior).
/// 2. Otherwise, if `config.language == "apex"`, invoke the Apex-specific
///    launcher assembly in [`resolve_apex_command`]. The launcher wraps the
///    vendored `apex-jorje-lsp.jar` inside a `java -cp <jar> <launcher-class>`
///    invocation, honoring `GRAPHENGINE_APEX_JORJE_JAR`,
///    `GRAPHENGINE_JAVA_HOME`, and the bundled-desktop install layout.
/// 3. Otherwise, fall back to the generic path: use `config.lsp_command` and
///    `config.lsp_args` as declared.
///
/// The Apex branch is isolated so every other language's resolution is
/// byte-identical to the pre-Apex behavior.
pub fn resolve_lsp_command(config: &LanguageConfig) -> Result<ResolvedLspCommand, LspError> {
    let env_var = format!(
        "GRAPHENGINE_LSP_{}",
        config.language.to_ascii_uppercase().replace('-', "_")
    );

    // (1) Full-command env override — highest precedence, bypasses all other
    // logic. Applies to every language uniformly.
    let env_override = env::var(&env_var)
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty());

    if let Some(candidate) = env_override.clone() {
        debug!("Using LSP command from {}", env_var);
        return resolve_generic(
            config,
            candidate,
            CommandSource::Environment(env_var.clone()),
        );
    }

    // (2) Apex-specific launcher assembly.
    if config.language == "apex" {
        return resolve_apex_command(config);
    }

    // (3) Generic config-driven path (unchanged behavior for all other langs).
    let candidate = config.lsp_command.clone().ok_or_else(|| {
        LspError::invalid_config(format!(
            "No LSP command configured for language '{}'. Set `lsp_command` in configs/{}.yaml or {}",
            config.language, config.language, env_var
        ))
    })?;
    resolve_generic(config, candidate, CommandSource::Config)
}

fn resolve_generic(
    config: &LanguageConfig,
    candidate: String,
    source: CommandSource,
) -> Result<ResolvedLspCommand, LspError> {
    let executable = resolve_executable(candidate.as_str()).map_err(|err| {
        LspError::server_not_available(format!(
            "Unable to locate LSP executable '{}': {}",
            candidate, err
        ))
    })?;

    // Don't add default --stdio for rust-analyzer as it doesn't accept it.
    // Apex never reaches this function (its branch builds args explicitly),
    // so no special case is needed here.
    let args = if config.language == "rust" {
        config.lsp_args.clone().unwrap_or_default()
    } else {
        config
            .lsp_args
            .clone()
            .unwrap_or_else(|| vec!["--stdio".to_string()])
    };

    let mut command = Vec::with_capacity(1 + args.len());
    command.push(executable.to_string_lossy().to_string());
    command.extend(args.clone());

    info!(
        "Resolved LSP command for {} via {:?}: {}",
        config.language,
        source,
        executable.display()
    );

    Ok(ResolvedLspCommand {
        executable,
        command,
        args,
        source,
    })
}

/// Assemble the Apex LSP launch command.
///
/// The Apex LSP (`apex-jorje-lsp.jar`) is a Java application — it cannot be
/// launched directly like a native binary. This function resolves two moving
/// pieces independently:
///
/// 1. **Java executable** — `GRAPHENGINE_JAVA_HOME/bin/java` if set, else the
///    bundled JRE at `<install>/runtime/jre/bin/java` if present, else `java`
///    on `PATH` (standard system Java).
/// 2. **`apex-jorje-lsp.jar`** — `GRAPHENGINE_APEX_JORJE_JAR` if set, else the
///    bundled JAR at `<install>/lsp/apex-jorje-lsp.jar`.
///
/// The YAML-declared `lsp_args` is then used as a template: every occurrence
/// of [`APEX_JORJE_JAR_PLACEHOLDER`] is rewritten to the resolved JAR path.
/// This keeps the canonical launcher invocation (`-cp <jar>
/// apex.jorje.lsp.ApexLanguageServerLauncher` plus debug flags) visible in
/// the config while path resolution stays in Rust.
fn resolve_apex_command(config: &LanguageConfig) -> Result<ResolvedLspCommand, LspError> {
    let java_path = resolve_java_executable()?;
    let jar_path = resolve_apex_jorje_jar()?;

    let template = config.lsp_args.clone().ok_or_else(|| {
        LspError::invalid_config(
            "apex config is missing `lsp_args` — expected the java-cp launcher template",
        )
    })?;

    // Rewrite every JAR placeholder to the resolved absolute path.
    let mut placeholder_seen = false;
    let args: Vec<String> = template
        .into_iter()
        .map(|arg| {
            if arg == APEX_JORJE_JAR_PLACEHOLDER {
                placeholder_seen = true;
                jar_path.to_string_lossy().to_string()
            } else {
                arg
            }
        })
        .collect();

    if !placeholder_seen {
        warn!(
            "apex.yaml lsp_args did not contain `{}`; command may not \
             reach the apex-jorje JAR. args={:?}",
            APEX_JORJE_JAR_PLACEHOLDER, args
        );
    }

    let mut command = Vec::with_capacity(1 + args.len());
    command.push(java_path.to_string_lossy().to_string());
    command.extend(args.clone());

    info!(
        "Resolved Apex LSP command: java={} jar={}",
        java_path.display(),
        jar_path.display()
    );

    Ok(ResolvedLspCommand {
        executable: java_path,
        command,
        args,
        source: CommandSource::Config,
    })
}

/// Resolve the Java executable to launch the Apex LSP with.
///
/// Precedence:
/// 1. `GRAPHENGINE_JAVA_HOME` — explicit override.
/// 2. `<install>/runtime/jre/bin/java` — bundled Temurin JRE shipped with the
///    desktop installer.
/// 3. `java` on `PATH` — system Java fallback (SFDX CLI users have this).
fn resolve_java_executable() -> Result<PathBuf, LspError> {
    if let Ok(home) = env::var("GRAPHENGINE_JAVA_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            let candidate = Path::new(trimmed).join("bin").join(java_binary_name());
            if candidate.exists() {
                debug!(
                    "Apex LSP using Java from GRAPHENGINE_JAVA_HOME: {}",
                    candidate.display()
                );
                return Ok(candidate);
            }
            return Err(LspError::server_not_available(format!(
                "GRAPHENGINE_JAVA_HOME='{}' but {} does not exist",
                home,
                candidate.display()
            )));
        }
    }

    if let Some(bundled) = find_bundled_jre() {
        debug!("Apex LSP using bundled JRE at {}", bundled.display());
        return Ok(bundled);
    }

    match which::which("java") {
        Ok(path) => {
            debug!("Apex LSP using system `java` from PATH: {}", path.display());
            Ok(path)
        }
        Err(e) => Err(LspError::server_not_available(format!(
            "Apex LSP requires Java 11+. None of these were available:\n  \
             - GRAPHENGINE_JAVA_HOME (unset)\n  \
             - bundled JRE at <install>/runtime/jre/bin/{}\n  \
             - `java` on PATH ({})\n\
             Install Java, set GRAPHENGINE_JAVA_HOME, or use the desktop \
             installer which bundles Eclipse Temurin 17.",
            java_binary_name(),
            e
        ))),
    }
}

/// Resolve the path to `apex-jorje-lsp.jar`.
///
/// Precedence:
/// 1. `GRAPHENGINE_APEX_JORJE_JAR` — explicit path override.
/// 2. `<install>/lsp/apex-jorje-lsp.jar` — bundled jar shipped with the
///    desktop installer.
fn resolve_apex_jorje_jar() -> Result<PathBuf, LspError> {
    if let Ok(path) = env::var("GRAPHENGINE_APEX_JORJE_JAR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            let pb = PathBuf::from(trimmed);
            if pb.exists() {
                return Ok(pb);
            }
            return Err(LspError::server_not_available(format!(
                "GRAPHENGINE_APEX_JORJE_JAR='{}' but path does not exist",
                path
            )));
        }
    }

    if let Some(bundled) = find_bundled_apex_jar() {
        return Ok(bundled);
    }

    Err(LspError::server_not_available(
        "apex-jorje-lsp.jar not found. Set GRAPHENGINE_APEX_JORJE_JAR to the \
         jar's absolute path, or use the GridSeak desktop installer which \
         bundles the jar at <install>/lsp/apex-jorje-lsp.jar.",
    ))
}

#[cfg(target_os = "windows")]
fn java_binary_name() -> &'static str {
    "java.exe"
}

#[cfg(not(target_os = "windows"))]
fn java_binary_name() -> &'static str {
    "java"
}

/// Walk a few levels of the executable's parent directory looking for a
/// bundled Temurin JRE at `runtime/jre/bin/java`. Mirrors the layout probed
/// by `resolved_configs_dir` in `infrastructure::config`.
fn find_bundled_jre() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let java_leaf = Path::new("runtime")
        .join("jre")
        .join("bin")
        .join(java_binary_name());
    for rel in ["", "..", "../..", "../../.."] {
        let candidate = exe_dir.join(rel).join(&java_leaf);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Walk a few levels of the executable's parent directory looking for a
/// bundled `apex-jorje-lsp.jar` at `lsp/apex-jorje-lsp.jar`.
fn find_bundled_apex_jar() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let jar_leaf = Path::new("lsp").join("apex-jorje-lsp.jar");
    for rel in ["", "..", "../..", "../../.."] {
        let candidate = exe_dir.join(rel).join(&jar_leaf);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn resolve_executable(candidate: &str) -> Result<PathBuf, String> {
    let path = Path::new(candidate);

    if path.is_absolute() || path.components().count() > 1 {
        if path.exists() {
            Ok(path.to_path_buf())
        } else {
            Err("path does not exist".to_string())
        }
    } else {
        match which::which(candidate) {
            Ok(resolved) => Ok(resolved),
            Err(e) => Err(format!("{}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::NodeKind;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    fn env_mutex() -> &'static Mutex<()> {
        static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_MUTEX.get_or_init(|| Mutex::new(()))
    }

    fn base_config() -> LanguageConfig {
        let mut queries = HashMap::new();
        queries.insert("functions".into(), "dummy".into());
        queries.insert("structs".into(), "dummy".into());
        queries.insert("modules".into(), "dummy".into());
        queries.insert("call_sites".into(), "dummy".into());
        let mut kind_mappings = HashMap::new();
        kind_mappings.insert("function_item".into(), NodeKind::Function);
        kind_mappings.insert("struct_item".into(), NodeKind::Struct);
        kind_mappings.insert("mod_item".into(), NodeKind::Module);

        let exe = std::env::current_exe().unwrap();
        LanguageConfig {
            language: "rust".into(),
            file_extensions: vec![".rs".into()],
            queries,
            kind_mappings,
            grammar_path: None,
            lsp_command: Some(exe.to_string_lossy().to_string()),
            lsp_args: Some(vec![]),
            version: "1.0".into(),
            receiver_type_detection: None,
            lsp_request_timeout_ms: None,
            lsp_max_concurrent_requests: None,
            lsp_initialization_options: None,
        }
    }

    #[test]
    fn resolve_prefers_env_override() {
        let _guard = env_mutex().lock().unwrap();
        let config = base_config();
        let var = "GRAPHENGINE_LSP_RUST";
        let previous = env::var(var).ok();
        let exe = std::env::current_exe().unwrap();
        env::set_var(var, exe.to_string_lossy().to_string());

        let resolved = resolve_lsp_command(&config).unwrap();
        assert!(matches!(resolved.source, CommandSource::Environment(_)));
        assert_eq!(resolved.executable, exe);

        if let Some(value) = previous {
            env::set_var(var, value);
        } else {
            env::remove_var(var);
        }
    }

    #[test]
    fn resolve_uses_config_when_no_env() {
        let _guard = env_mutex().lock().unwrap();
        env::remove_var("GRAPHENGINE_LSP_RUST");
        let config = base_config();
        let resolved = resolve_lsp_command(&config)
            .expect("should resolve config command when which finds it");
        assert_eq!(resolved.command[0], resolved.executable.to_string_lossy());
    }

    /// Construct an apex config skeleton that matches `configs/apex.yaml`'s
    /// shape — exactly one `APEX_JORJE_JAR_PLACEHOLDER` between `-cp` and the
    /// launcher class, plus the two apex-jorje debug flags. Used by the Apex
    /// branch tests below.
    fn apex_config() -> LanguageConfig {
        let mut queries = HashMap::new();
        queries.insert("functions".into(), "dummy".into());
        queries.insert("structs".into(), "dummy".into());
        queries.insert("modules".into(), "dummy".into());
        queries.insert("call_sites".into(), "dummy".into());
        let mut kind_mappings = HashMap::new();
        kind_mappings.insert("class_declaration".into(), NodeKind::Struct);

        LanguageConfig {
            language: "apex".into(),
            file_extensions: vec![".cls".into(), ".trigger".into(), ".apxc".into()],
            queries,
            kind_mappings,
            grammar_path: None,
            lsp_command: Some("java".into()),
            lsp_args: Some(vec![
                "-cp".into(),
                APEX_JORJE_JAR_PLACEHOLDER.into(),
                "-Ddebug.internal.errors=true".into(),
                "-Ddebug.semantic.errors=true".into(),
                "apex.jorje.lsp.ApexLanguageServerLauncher".into(),
            ]),
            version: "1.0".into(),
            receiver_type_detection: None,
            lsp_request_timeout_ms: Some(15_000),
            lsp_max_concurrent_requests: Some(8),
            lsp_initialization_options: Some(serde_json::json!({
                "enableSemanticErrors": true
            })),
        }
    }

    struct ApexEnvGuard {
        prev_java_home: Option<String>,
        prev_apex_jar: Option<String>,
        prev_apex_override: Option<String>,
    }

    impl ApexEnvGuard {
        fn setup(java_home: Option<&Path>, jar: Option<&Path>) -> Self {
            let guard = ApexEnvGuard {
                prev_java_home: env::var("GRAPHENGINE_JAVA_HOME").ok(),
                prev_apex_jar: env::var("GRAPHENGINE_APEX_JORJE_JAR").ok(),
                prev_apex_override: env::var("GRAPHENGINE_LSP_APEX").ok(),
            };
            env::remove_var("GRAPHENGINE_LSP_APEX");
            match java_home {
                Some(p) => env::set_var("GRAPHENGINE_JAVA_HOME", p),
                None => env::remove_var("GRAPHENGINE_JAVA_HOME"),
            }
            match jar {
                Some(p) => env::set_var("GRAPHENGINE_APEX_JORJE_JAR", p),
                None => env::remove_var("GRAPHENGINE_APEX_JORJE_JAR"),
            }
            guard
        }
    }

    impl Drop for ApexEnvGuard {
        fn drop(&mut self) {
            match &self.prev_java_home {
                Some(v) => env::set_var("GRAPHENGINE_JAVA_HOME", v),
                None => env::remove_var("GRAPHENGINE_JAVA_HOME"),
            }
            match &self.prev_apex_jar {
                Some(v) => env::set_var("GRAPHENGINE_APEX_JORJE_JAR", v),
                None => env::remove_var("GRAPHENGINE_APEX_JORJE_JAR"),
            }
            match &self.prev_apex_override {
                Some(v) => env::set_var("GRAPHENGINE_LSP_APEX", v),
                None => env::remove_var("GRAPHENGINE_LSP_APEX"),
            }
        }
    }

    #[test]
    fn apex_branch_substitutes_jar_placeholder_in_args() {
        let _guard = env_mutex().lock().unwrap();

        // Stage a fake JRE layout (<dir>/bin/java -> any existing executable)
        // and a fake jar on disk. current_exe is guaranteed to exist.
        let temp = tempfile::tempdir().unwrap();
        let java_home = temp.path();
        let bin_dir = java_home.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let fake_java = bin_dir.join(java_binary_name());
        std::fs::copy(std::env::current_exe().unwrap(), &fake_java).unwrap();

        let fake_jar = temp.path().join("apex-jorje-lsp.jar");
        std::fs::write(&fake_jar, b"fake jar bytes").unwrap();

        let _env = ApexEnvGuard::setup(Some(java_home), Some(&fake_jar));
        let config = apex_config();

        let resolved =
            resolve_lsp_command(&config).expect("apex branch should resolve with env vars set");

        // Executable is the Java binary, NOT the JAR.
        assert_eq!(resolved.executable, fake_java);
        assert_eq!(resolved.command[0], fake_java.to_string_lossy());

        // Placeholder replaced exactly once by the resolved jar path.
        let jar_str = fake_jar.to_string_lossy().to_string();
        let jar_occurrences = resolved.args.iter().filter(|a| **a == jar_str).count();
        assert_eq!(
            jar_occurrences, 1,
            "expected exactly one resolved jar path in args, got args: {:?}",
            resolved.args
        );

        // No placeholder leakage remains.
        assert!(
            !resolved
                .args
                .iter()
                .any(|a| a == APEX_JORJE_JAR_PLACEHOLDER),
            "jar placeholder must not survive resolution: {:?}",
            resolved.args
        );

        // Launcher class remains present — we only substituted the jar token.
        assert!(resolved
            .args
            .iter()
            .any(|a| a == "apex.jorje.lsp.ApexLanguageServerLauncher"));
    }

    #[test]
    fn apex_branch_full_command_env_override_beats_apex_logic() {
        let _guard = env_mutex().lock().unwrap();
        let config = apex_config();
        let exe = std::env::current_exe().unwrap();

        let prev = env::var("GRAPHENGINE_LSP_APEX").ok();
        env::set_var("GRAPHENGINE_LSP_APEX", exe.to_string_lossy().to_string());

        let resolved = resolve_lsp_command(&config).expect("full-command override should resolve");
        assert!(matches!(resolved.source, CommandSource::Environment(_)));
        assert_eq!(resolved.executable, exe);
        // In full-override mode, the Apex launcher assembly is NOT applied —
        // args come from the config untouched, which for apex means the
        // placeholder is still present. That's expected: the user asked for
        // a verbatim command.
        assert!(
            resolved
                .args
                .iter()
                .any(|a| a == APEX_JORJE_JAR_PLACEHOLDER),
            "full override must not invoke apex placeholder substitution"
        );

        if let Some(v) = prev {
            env::set_var("GRAPHENGINE_LSP_APEX", v);
        } else {
            env::remove_var("GRAPHENGINE_LSP_APEX");
        }
    }

    #[test]
    fn apex_branch_reports_missing_jar_clearly() {
        let _guard = env_mutex().lock().unwrap();
        // Point java home at something real, jar at a nonexistent path.
        let temp = tempfile::tempdir().unwrap();
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let fake_java = bin_dir.join(java_binary_name());
        std::fs::copy(std::env::current_exe().unwrap(), &fake_java).unwrap();

        let missing_jar = temp.path().join("does-not-exist.jar");
        let _env = ApexEnvGuard::setup(Some(temp.path()), Some(&missing_jar));

        let err = resolve_lsp_command(&apex_config()).expect_err("missing jar must error out");
        let msg = format!("{}", err);
        assert!(
            msg.contains("does not exist"),
            "error should mention missing jar, got: {msg}"
        );
    }
}
