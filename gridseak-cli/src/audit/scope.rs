//! What a `gridseak audit` answer key can vouch for, and how the question
//! states it: the files the compiler index covers, the files the parser could
//! only partly read, and TypeScript call shapes the generic question leaves open.

use std::collections::HashSet;

use rusqlite::Connection;

/// Where the compiler-sourced call graph is complete. Files outside the
/// compiler project have no call edges at all, and files the parser could
/// only partly read may hide callers, so "every edge is from the compiler"
/// certifies nothing about them.
#[derive(Default)]
pub(super) struct Coverage {
    /// Files the compiler index skipped (repo-relative).
    pub(super) unindexed: HashSet<String>,
    /// Indexed files with syntax errors: their functions may be missing.
    pub(super) partial: HashSet<String>,
    /// Shortest paths (dirs end in `/`) covering exactly the unindexed files.
    excluded: Vec<String>,
}

impl Coverage {
    pub(super) fn load(conn: &Connection, root_s: &str, files: &HashSet<String>) -> Self {
        let rel = |f: &str| f.strip_prefix(root_s).unwrap_or(f).to_string();
        let unindexed: HashSet<String> = conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'resolution_compiler_unindexed_files'",
                [],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok())
            .unwrap_or_default()
            .iter()
            .map(|f| rel(f))
            .collect();
        let mut partial = HashSet::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT file_path FROM file_extraction_coverage
             WHERE json_extract(payload_json, '$.confidence') = 'Low'",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                partial.extend(rows.flatten().map(|f| rel(&f)));
            }
        }
        let indexed: Vec<&String> = files.iter().filter(|f| !unindexed.contains(*f)).collect();
        let mut excluded: Vec<String> = unindexed
            .iter()
            .map(|f| {
                // Widest directory holding no indexed file, else the file itself.
                let parts: Vec<&str> = f.split('/').collect();
                (1..parts.len())
                    .map(|n| format!("{}/", parts[..n].join("/")))
                    .find(|d| !indexed.iter().any(|i| i.starts_with(d.as_str())))
                    .unwrap_or_else(|| f.clone())
            })
            .collect();
        excluded.sort();
        excluded.dedup();
        Self {
            unindexed,
            partial,
            excluded,
        }
    }

    /// Sentence added to each question so the asked scope matches the key.
    pub(super) fn question_note(&self) -> String {
        if self.excluded.is_empty() {
            return String::new();
        }
        format!(
            " Only consider code the compiler project covers: ignore {}.",
            self.excluded
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    pub(super) fn key_note(&self) -> String {
        let mut s = String::new();
        if !self.excluded.is_empty() {
            s += &format!(
                "; the {} files outside the compiler project are excluded from questions and keys",
                self.unindexed.len()
            );
        }
        if !self.partial.is_empty() {
            s += &format!(
                "; targets whose callers could reach the {} partly parsed file(s) are not asked",
                self.partial.len()
            );
        }
        s
    }
}

/// TypeScript/JavaScript shapes the generic question leaves open, stated the
/// way the key (and the TypeScript checker) treats them.
pub(super) fn ts_rules(file: &str) -> &'static str {
    let ts = [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"];
    if !ts.iter().any(|e| file.ends_with(e)) {
        return "";
    }
    " In TypeScript, an arrow function or function expression assigned to a variable or class field is \
     a named function (named by the variable or field); one written anywhere else (an argument, an \
     object-literal property value, a JSX attribute) is anonymous. `new C(...)` calls C's constructor, \
     `super(...)` calls the base class's constructor, and a tagged template f`...` calls f. Reading a \
     getter, rendering a JSX element (`<C />`) and `f.call`/`f.apply`/`f.bind` are not direct calls."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_names_the_widest_unindexed_paths() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
               CREATE TABLE file_extraction_coverage (file_path TEXT PRIMARY KEY, language TEXT, payload_json TEXT);
               INSERT INTO metadata VALUES ('resolution_compiler_unindexed_files',
                 '["adapters/bun/a.ts","adapters/deno/b.ts","src/gen/x.ts","vite.config.ts"]');
               INSERT INTO file_extraction_coverage VALUES ('/r/src/ctx.ts', 'typescript', '{"confidence":"Low"}');
               INSERT INTO file_extraction_coverage VALUES ('/r/src/ok.ts', 'typescript', '{"confidence":"High"}');"#,
        )
        .unwrap();
        let files: HashSet<String> = [
            "adapters/bun/a.ts",
            "adapters/deno/b.ts",
            "src/gen/x.ts",
            "src/gen/y.ts",
            "src/ctx.ts",
            "src/ok.ts",
            "vite.config.ts",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let cov = Coverage::load(&conn, "/r/", &files);
        assert_eq!(
            cov.excluded,
            ["adapters/", "src/gen/x.ts", "vite.config.ts"]
        );
        assert_eq!(cov.unindexed.len(), 4);
        assert!(cov.partial.contains("src/ctx.ts") && cov.partial.len() == 1);
        assert!(cov
            .question_note()
            .contains("ignore `adapters/`, `src/gen/x.ts`, `vite.config.ts`."));
        assert!(ts_rules("src/a.ts").contains("super(...)"));
        assert_eq!(ts_rules("main.go"), "");
    }

    #[test]
    fn no_metadata_means_no_scope_note() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        let cov = Coverage::load(&conn, "/r/", &HashSet::new());
        assert!(cov.question_note().is_empty() && cov.key_note().is_empty());
    }
}
