//! No-LLM gate fixtures. Synthetic graph — CI does not need G1 sqlite.

use std::path::PathBuf;

use rusqlite::Connection;

#[cfg(test)]
use super::decision::{decide, Permission, Request};
#[cfg(test)]
use super::parse_edit::from_old_new;

#[cfg(test)]
pub fn write_fixture_db(path: &std::path::Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE nodes (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            fqn TEXT NOT NULL,
            location TEXT NOT NULL DEFAULT '{}',
            provenance TEXT NOT NULL DEFAULT '{}',
            properties TEXT NOT NULL DEFAULT '{}',
            trait_metadata TEXT
        );
        CREATE TABLE edges (
            from_id TEXT NOT NULL,
            to_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            provenance TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (from_id, to_id, kind)
        );
        CREATE TABLE metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        INSERT INTO metadata (key, value) VALUES ('language', 'go');

        INSERT INTO nodes (id, kind, fqn, location) VALUES
            ('nr', 'Function', 'chi::NewRouter',
                '{"file":"chi.go","start_line":120}'),
            ('use', 'Method', 'chi::Mux::Use',
                '{"file":"chi.go","start_line":80}'),
            ('test_strip', 'Function', 'middleware::strip_test::TestStripSlashes',
                '{"file":"middleware/strip_test.go","start_line":10}'),
            ('use_caller', 'Function', 'app::wire',
                '{"file":"app.go","start_line":4}'),
            ('parse_src', 'Function', 'zod::parse',
                '{"file":"src/types.ts","start_line":40}'),
            ('parse_deno', 'Function', 'deno::lib::parse',
                '{"file":"deno/lib/types.ts","start_line":40}');

        INSERT INTO edges (from_id, to_id, kind, provenance) VALUES
            ('test_strip', 'nr', '{"kind":"Call"}',
                '{"source":"Compiler","confidence":"High"}'),
            ('use_caller', 'use', '{"kind":"Call"}',
                '{"source":"Compiler","confidence":"High"}');
        "#,
    )
    .unwrap();
}

#[cfg(test)]
fn chi_old() -> &'static str {
    "package chi\n\nfunc NewRouter() *Mux {\n\treturn &Mux{}\n}\n"
}

#[cfg(test)]
fn chi_deleted() -> &'static str {
    "package chi\n"
}

#[cfg(test)]
fn chi_comment() -> &'static str {
    "package chi\n\n// harmless comment\nfunc NewRouter() *Mux {\n\treturn &Mux{}\n}\n"
}

/// Pinned chi G1 sqlite (`73abec07…`). Override with `GRIDSEAK_G1_CHI_ARTIFACT`.
#[cfg(test)]
pub fn g1_chi_artifact() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GRIDSEAK_G1_CHI_ARTIFACT") {
        let path = PathBuf::from(p);
        return path.is_file().then_some(path);
    }
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home)
        .join("Library/Application Support/com.gridseak.desktop/project-graphs")
        .join("73abec07-12fd-47e9-a2e8-fb3c69fdf971.sqlite");
    path.is_file().then_some(path)
}

#[cfg(test)]
fn g1_chi_source() -> Option<String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../.cache/g1/chi/chi.go");
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
fn delete_newrouter_hunk(src: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in src.lines() {
        if line.starts_with("func NewRouter") {
            skipping = true;
            continue;
        }
        if skipping {
            if line == "}" {
                skipping = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn artifact() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gate.sqlite");
        write_fixture_db(&path);
        (dir, path)
    }

    #[test]
    fn newrouter_delete_denies_with_compiler_witness() {
        let (_dir, path) = artifact();
        let edit = from_old_new("chi.go", chi_old(), chi_deleted());
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Deny, "{}", d.reason);
        assert_eq!(d.tier, "compiler");
        assert!(
            d.witnesses.iter().any(|w| w.contains("TestStripSlashes")),
            "witnesses={:?}",
            d.witnesses
        );
    }

    #[test]
    fn deno_twin_edit_denies_with_canonical_file() {
        let (_dir, path) = artifact();
        let edit = from_old_new(
            "deno/lib/types.ts",
            "export function parse(x: unknown) { return x }\n",
            "export function parse(x: unknown) { return x as any }\n",
        );
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Deny, "{}", d.reason);
        assert!(
            d.reason.contains("src/types.ts")
                || d.witnesses.iter().any(|w| w.contains("src/types.ts")),
            "reason={} witnesses={:?}",
            d.reason,
            d.witnesses
        );
    }

    #[test]
    fn harmless_comment_allows() {
        let (_dir, path) = artifact();
        let edit = from_old_new("chi.go", chi_old(), chi_comment());
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Allow, "{}", d.reason);
    }

    fn mux_use_old() -> &'static str {
        "package chi\n\nfunc (m *Mux) Use(mw any) {\n\tm.mws = append(m.mws, mw)\n}\n"
    }

    #[test]
    fn go_method_use_delete_denies_compiler() {
        let (_dir, path) = artifact();
        let edit = from_old_new("chi.go", mux_use_old(), "package chi\n");
        assert!(
            edit.removed_names.iter().any(|n| n == "Use"),
            "tree-sitter must see method Use, names={:?}",
            edit.removed_names
        );
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Deny, "{}", d.reason);
        assert_eq!(d.tier, "compiler");
        assert!(
            d.witnesses
                .iter()
                .any(|w| w.contains("wire") || w.contains("Use")),
            "witnesses={:?}",
            d.witnesses
        );
    }

    #[test]
    fn rename_newrouter_to_newrouter2_denies() {
        let (_dir, path) = artifact();
        let edit = from_old_new(
            "chi.go",
            chi_old(),
            "package chi\n\nfunc NewRouter2() *Mux {\n\treturn &Mux{}\n}\n",
        );
        assert!(
            edit.removed_names.iter().any(|n| n == "NewRouter"),
            "rename is a remove of NewRouter, names={:?}",
            edit.removed_names
        );
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Deny, "{}", d.reason);
        assert_eq!(d.tier, "compiler");
    }

    #[test]
    fn bash_rm_chi_go_denies_newrouter() {
        let (_dir, path) = artifact();
        let edit = crate::gate::parse_edit::from_host_payload(&crate::gate::host_io::HostPayload {
            command: Some("rm chi.go".into()),
            ..Default::default()
        });
        assert!(edit.whole_file_delete);
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Deny, "{}", d.reason);
        assert!(
            d.symbols.iter().any(|s| s.fqn.contains("NewRouter")),
            "whole-file delete must resolve NewRouter via scan-range, symbols={:?}",
            d.symbols
        );
    }

    #[test]
    fn git_rm_chi_go_denies_newrouter() {
        let (_dir, path) = artifact();
        let edit = crate::gate::parse_edit::from_host_payload(&crate::gate::host_io::HostPayload {
            command: Some("git rm chi.go".into()),
            ..Default::default()
        });
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Deny, "{}", d.reason);
    }

    #[test]
    fn ts_const_arrow_or_scan_range() {
        let (_dir, path) = artifact();
        // Pad so scan-range can intersect the fixture node at line 40
        // if the TSX grammar does not treat `export const parse =` as
        // a named function.
        let mut old = String::new();
        for i in 1..40 {
            old.push_str(&format!("// line {i}\n"));
        }
        old.push_str("export const parse = (x: unknown) => x;\n");
        let new = old.replace(
            "export const parse = (x: unknown) => x;",
            "export const other = (x: unknown) => x;",
        );
        let edit = from_old_new("src/types.ts", &old, &new);
        if !edit.removed_names.iter().any(|n| n == "parse") {
            eprintln!(
                "TS miss: export const parse = is not a named function; extract_source={} names={:?} ranges={:?}",
                edit.extract_source, edit.removed_names, edit.deleted_ranges
            );
            assert!(
                !edit.deleted_ranges.is_empty(),
                "scan-range fallback requires deleted lines"
            );
        }
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert!(
            d.symbols.iter().any(|s| s.fqn.contains("parse")),
            "name or scan-range must resolve zod::parse, symbols={:?} source-note",
            d.symbols
        );
    }

    #[test]
    fn stale_dirty_asks_rescan() {
        let (_dir, path) = artifact();
        let edit = from_old_new("chi.go", chi_old(), chi_deleted());
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: true,
            dirty_paths: vec!["chi.go".into()],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Ask, "{}", d.reason);
        assert!(d.reason.contains("stale"), "{}", d.reason);
        assert_eq!(d.tier, "stale");
    }

    /// Real pinned chi G1 artifact. CI skips unless the sqlite is present.
    /// Run locally: `cargo test -p gridseak-cli g1_chi_newrouter -- --ignored --nocapture`
    #[test]
    #[ignore = "requires pinned chi G1 sqlite (73abec07)"]
    fn g1_chi_newrouter_delete_denies_compiler() {
        let artifact = g1_chi_artifact().expect(
            "chi G1 sqlite missing — set GRIDSEAK_G1_CHI_ARTIFACT or install 73abec07-….sqlite",
        );
        let old = g1_chi_source().expect("missing .cache/g1/chi/chi.go");
        assert!(
            old.contains("func NewRouter"),
            "chi.go pin does not define NewRouter"
        );
        let new = delete_newrouter_hunk(&old);
        assert!(!new.contains("func NewRouter"));
        let d = decide(&Request {
            edit: from_old_new("chi.go", &old, &new),
            artifact,
            scan_id: "73abec07-12fd-47e9-a2e8-fb3c69fdf971".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Deny, "{}", d.reason);
        assert_eq!(d.tier, "compiler", "tier={} reason={}", d.tier, d.reason);
        assert!(
            d.witnesses.iter().any(|w| w.contains("TestStripSlashes")),
            "expected Compiler caller TestStripSlashes, witnesses={:?}",
            d.witnesses
        );
    }

    fn write_preferred_db(path: &std::path::Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE nodes (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, fqn TEXT NOT NULL,
                location TEXT NOT NULL DEFAULT '{}', provenance TEXT NOT NULL DEFAULT '{}',
                properties TEXT NOT NULL DEFAULT '{}', trait_metadata TEXT
            );
            CREATE TABLE edges (
                from_id TEXT NOT NULL, to_id TEXT NOT NULL, kind TEXT NOT NULL,
                provenance TEXT NOT NULL DEFAULT '{}',
                PRIMARY KEY (from_id, to_id, kind)
            );
            CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO metadata (key, value) VALUES ('language', 'go');
            INSERT INTO nodes (id, kind, fqn, location) VALUES
                ('iface', 'Method', 'chi::Router::Use',
                    '{"file":"chi.go","start_line":72}'),
                ('mux', 'Method', 'chi::Mux::Use',
                    '{"file":"mux.go","start_line":80}'),
                ('wire', 'Function', 'app::wire',
                    '{"file":"app.go","start_line":4}');
            INSERT INTO edges (from_id, to_id, kind, provenance) VALUES
                ('wire', 'mux', '{"kind":"Call"}',
                    '{"source":"Compiler","confidence":"High"}');
            "#,
        )
        .unwrap();
    }

    #[test]
    fn go_interface_use_preferred_denies_twin() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("graph.sqlite");
        write_preferred_db(&path);
        let old = "package chi\n\ntype Router interface {\n\tUse(middlewares ...func()) \n}\n";
        let new =
            "package chi\n\ntype Router interface {\n\tUseRenamed(middlewares ...func()) \n}\n";
        let edit = from_old_new("chi.go", old, new);
        assert!(
            edit.removed_names.iter().any(|n| n == "Use"),
            "names={:?}",
            edit.removed_names
        );
        let d = decide(&Request {
            edit,
            artifact: path,
            scan_id: "fixture".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        })
        .unwrap();
        assert_eq!(d.permission, Permission::Deny, "{}", d.reason);
        assert_eq!(d.tier, "twin", "{}", d.reason);
        assert!(
            d.reason.contains("mux.go"),
            "canonical file missing: {}",
            d.reason
        );
    }

    /// Warm `decide()` on the pinned chi NewRouter delete. Fails if this
    /// machine's warm path is slower than 50ms — print the number, do
    /// not hardcode a fake pass.
    #[test]
    #[ignore = "requires pinned chi G1 sqlite (73abec07)"]
    fn g1_chi_newrouter_decide_warm_under_50ms() {
        let artifact = g1_chi_artifact().expect("chi G1 sqlite missing");
        let old = g1_chi_source().expect("missing .cache/g1/chi/chi.go");
        let new = delete_newrouter_hunk(&old);
        let req = Request {
            edit: from_old_new("chi.go", &old, &new),
            artifact,
            scan_id: "73abec07-12fd-47e9-a2e8-fb3c69fdf971".into(),
            stale: false,
            dirty_paths: vec![],
            extra_symbols: vec![],
        };
        let _ = decide(&req).unwrap();
        let started = std::time::Instant::now();
        let d = decide(&req).unwrap();
        let ms = started.elapsed().as_millis();
        eprintln!(
            "warm decide() NewRouter delete: {ms}ms permission={:?}",
            d.permission
        );
        assert_eq!(d.permission, Permission::Deny, "{}", d.reason);
        assert!(ms <= 50, "warm decide() {ms}ms > 50ms on this machine");
    }
}
