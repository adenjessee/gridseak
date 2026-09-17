//! Parse a pending host edit into removed/renamed function names and
//! deleted line ranges.

use std::collections::HashSet;

use super::host_io::HostPayload;
use super::ts_names::{extract_names, heuristic_names, ExtractedName};

#[derive(Debug, Clone, Default)]
pub struct PendingEdit {
    pub file: String,
    pub old_text: String,
    #[allow(dead_code)]
    pub new_text: String,
    pub deleted_ranges: Vec<(u32, u32)>,
    pub removed_names: Vec<String>,
    pub whole_file_delete: bool,
    /// `tree-sitter` | `heuristic` | `scan-range`
    pub extract_source: String,
    /// Host-provided working directory (Cursor `cwd`), if any.
    pub cwd: Option<String>,
}

pub fn from_old_new(file: &str, old: &str, new: &str) -> PendingEdit {
    let (old_names, old_src) = extract_names(file, old);
    let (new_names, _) = extract_names(file, new);
    let new_set: HashSet<&str> = new_names.iter().map(|n| n.name.as_str()).collect();
    let removed: Vec<ExtractedName> = old_names
        .into_iter()
        .filter(|n| !new_set.contains(n.name.as_str()))
        .collect();
    let mut extract_source = old_src.to_string();
    let mut ranges: Vec<(u32, u32)> = removed
        .iter()
        .map(|n| (n.start_line, n.start_line))
        .collect();
    if ranges.is_empty() {
        ranges = deleted_line_ranges(old, new);
        if !ranges.is_empty() && removed.is_empty() {
            extract_source = "scan-range".into();
        }
    }
    PendingEdit {
        file: file.to_string(),
        old_text: old.to_string(),
        new_text: new.to_string(),
        deleted_ranges: ranges,
        removed_names: removed.into_iter().map(|n| n.name).collect(),
        whole_file_delete: new.trim().is_empty() && !old.trim().is_empty(),
        extract_source,
        cwd: None,
    }
}

impl PendingEdit {
    /// Whether this edit can change call-graph symbols. Safe shell
    /// (`pwd`, `cargo test`) is not a scan event.
    pub fn needs_scan(&self) -> bool {
        self.whole_file_delete || !self.removed_names.is_empty() || !self.deleted_ranges.is_empty()
    }
}

pub fn from_host_payload(payload: &HostPayload) -> PendingEdit {
    let cwd = payload.working_dir();
    if payload.is_delete_tool() {
        return PendingEdit {
            file: payload.file_path().unwrap_or_default(),
            old_text: String::new(),
            new_text: String::new(),
            deleted_ranges: Vec::new(),
            removed_names: Vec::new(),
            whole_file_delete: true,
            extract_source: "scan-range".into(),
            cwd,
        };
    }
    if let Some(cmd) = payload.shell_command() {
        if let Some(path) = rm_path(&cmd) {
            return PendingEdit {
                file: path,
                old_text: String::new(),
                new_text: String::new(),
                deleted_ranges: Vec::new(),
                removed_names: Vec::new(),
                whole_file_delete: true,
                extract_source: "scan-range".into(),
                cwd,
            };
        }
    }

    let file = payload.file_path().unwrap_or_default();
    let new = payload.new_text().unwrap_or_default();
    let old = payload
        .old_text()
        .unwrap_or_else(|| existing_file_text(&file, cwd.as_deref()));
    if payload
        .tool_name()
        .as_deref()
        .is_some_and(|n| n.eq_ignore_ascii_case("write"))
        && new.trim().is_empty()
        && !old.trim().is_empty()
    {
        return PendingEdit {
            file,
            old_text: old,
            new_text: new,
            deleted_ranges: Vec::new(),
            removed_names: Vec::new(),
            whole_file_delete: true,
            extract_source: "scan-range".into(),
            cwd,
        };
    }
    if !old.is_empty() || !new.is_empty() {
        let mut edit = from_old_new(&file, &old, &new);
        edit.cwd = cwd;
        return edit;
    }
    PendingEdit {
        file,
        cwd,
        ..PendingEdit::default()
    }
}

fn existing_file_text(file: &str, cwd: Option<&str>) -> String {
    if file.is_empty() {
        return String::new();
    }
    let path = std::path::Path::new(file);
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(cwd) = cwd {
        std::path::Path::new(cwd).join(path)
    } else {
        path.to_path_buf()
    };
    std::fs::read_to_string(full).unwrap_or_default()
}

pub fn from_unified_diff(diff: &str) -> Vec<PendingEdit> {
    let mut out = Vec::new();
    let mut file = String::new();
    let mut old_lines: Vec<String> = Vec::new();
    let mut new_lines: Vec<String> = Vec::new();
    let mut deleted_ranges = Vec::new();
    let mut old_line: u32 = 1;
    let mut range_start: Option<u32> = None;

    let flush = |file: &str,
                 old_lines: &[String],
                 new_lines: &[String],
                 deleted_ranges: &[(u32, u32)]|
     -> Option<PendingEdit> {
        if file.is_empty() && old_lines.is_empty() && new_lines.is_empty() {
            return None;
        }
        let old = old_lines.join("\n");
        let new = new_lines.join("\n");
        let mut edit = from_old_new(file, &old, &new);
        if edit.deleted_ranges.is_empty() {
            edit.deleted_ranges = deleted_ranges.to_vec();
            if edit.extract_source.is_empty() {
                edit.extract_source = "scan-range".into();
            }
        }
        Some(edit)
    };

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            file = rest.trim().to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("+++ ") {
            file = rest.trim().trim_start_matches("b/").to_string();
            continue;
        }
        if line.starts_with("diff --git") || line.starts_with("--- ") {
            continue;
        }
        if let Some(hunk) = line.strip_prefix("@@") {
            if let Some(old_start) = parse_hunk_old_start(hunk) {
                old_line = old_start;
            }
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            old_lines.push(line[1..].to_string());
            if range_start.is_none() {
                range_start = Some(old_line);
            }
            old_line += 1;
            continue;
        }
        if line.starts_with('+') && !line.starts_with("+++") {
            if let Some(start) = range_start.take() {
                deleted_ranges.push((start, old_line.saturating_sub(1)));
            }
            new_lines.push(line[1..].to_string());
            continue;
        }
        if let Some(start) = range_start.take() {
            deleted_ranges.push((start, old_line.saturating_sub(1)));
        }
        if let Some(rest) = line.strip_prefix(' ') {
            old_lines.push(rest.to_string());
            new_lines.push(rest.to_string());
            old_line += 1;
        }
    }
    if let Some(start) = range_start.take() {
        deleted_ranges.push((start, old_line.saturating_sub(1)));
    }
    if let Some(edit) = flush(&file, &old_lines, &new_lines, &deleted_ranges) {
        out.push(edit);
    }
    out
}

fn parse_hunk_old_start(hunk: &str) -> Option<u32> {
    let minus = hunk.find('-')?;
    let rest = &hunk[minus + 1..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

pub fn function_names_in(src: &str) -> Vec<String> {
    let mut names: Vec<String> = heuristic_names(src).into_iter().map(|n| n.name).collect();
    names.sort();
    names.dedup();
    names
}

fn deleted_line_ranges(old: &str, new: &str) -> Vec<(u32, u32)> {
    let new_set: HashSet<&str> = new.lines().collect();
    let mut ranges = Vec::new();
    for (i, line) in old.lines().enumerate() {
        if !new_set.contains(line) {
            let n = (i as u32) + 1;
            ranges.push((n, n));
        }
    }
    ranges
}

fn rm_path(cmd: &str) -> Option<String> {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let is_rm = tokens[0] == "rm"
        || tokens[0].ends_with("/rm")
        || (tokens[0] == "git" && tokens.get(1) == Some(&"rm"));
    if !is_rm {
        return None;
    }
    tokens
        .iter()
        .rev()
        .find(|t| !t.starts_with('-') && **t != "rm" && **t != "git")
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_go_newrouter() {
        let old = "func NewRouter() *Mux {\n  return &Mux{}\n}\n";
        let new = "// gone\n";
        let edit = from_old_new("chi.go", old, new);
        assert!(edit.removed_names.iter().any(|n| n == "NewRouter"));
        assert_eq!(edit.extract_source, "tree-sitter");
    }

    #[test]
    fn go_interface_use_rename_counts_as_remove() {
        let old = "package chi\n\ntype Router interface {\n\tUse(middlewares ...func()) \n}\n";
        let new =
            "package chi\n\ntype Router interface {\n\tUseRenamed(middlewares ...func()) \n}\n";
        let edit = from_old_new("chi.go", old, new);
        assert!(
            edit.removed_names.iter().any(|n| n == "Use"),
            "interface rename must count as remove of Use, names={:?}",
            edit.removed_names
        );
    }

    #[test]
    fn go_method_use_removed() {
        let old = "package chi\n\nfunc (m *Mux) Use(mw any) { }\n";
        let new = "package chi\n";
        let edit = from_old_new("chi.go", old, new);
        assert!(
            edit.removed_names.iter().any(|n| n == "Use"),
            "removed={:?}",
            edit.removed_names
        );
    }

    #[test]
    fn rename_newrouter_counts_as_remove() {
        let old = "package chi\n\nfunc NewRouter() *Mux { return nil }\n";
        let new = "package chi\n\nfunc NewRouter2() *Mux { return nil }\n";
        let edit = from_old_new("chi.go", old, new);
        assert!(
            edit.removed_names.iter().any(|n| n == "NewRouter"),
            "rename must count as remove of NewRouter, got {:?}",
            edit.removed_names
        );
        assert!(!edit.removed_names.iter().any(|n| n == "NewRouter2"));
    }

    #[test]
    fn comment_only_removes_nothing() {
        let old = "func NewRouter() *Mux { return nil }\n";
        let new = "// note\nfunc NewRouter() *Mux { return nil }\n";
        let edit = from_old_new("chi.go", old, new);
        assert!(edit.removed_names.is_empty());
    }

    #[test]
    fn rm_chi_go_is_whole_file_delete() {
        let payload = HostPayload {
            command: Some("rm chi.go".into()),
            ..HostPayload::default()
        };
        let edit = from_host_payload(&payload);
        assert!(edit.whole_file_delete);
        assert_eq!(edit.file, "chi.go");
        assert_eq!(edit.extract_source, "scan-range");
    }

    #[test]
    fn git_rm_chi_go_is_whole_file_delete() {
        let payload = HostPayload {
            command: Some("git rm chi.go".into()),
            ..HostPayload::default()
        };
        let edit = from_host_payload(&payload);
        assert!(edit.whole_file_delete);
        assert_eq!(edit.file, "chi.go");
    }

    #[test]
    fn cursor_delete_tool_is_whole_file_delete() {
        let payload: HostPayload = serde_json::from_str(
            r#"{"tool_name":"Delete","tool_input":{"file_path":"chi.go"},"workspace_roots":["/tmp"]}"#,
        )
        .unwrap();
        let edit = from_host_payload(&payload);
        assert!(edit.whole_file_delete);
        assert_eq!(edit.file, "chi.go");
        assert_eq!(edit.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn cursor_write_reads_disk_old_to_see_removed_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("chi.go");
        std::fs::write(
            &path,
            "package chi\n\nfunc NewRouter() *Mux { return nil }\n",
        )
        .unwrap();
        let payload: HostPayload = serde_json::from_str(&format!(
            r#"{{"tool_name":"Write","tool_input":{{"file_path":"{}","content":"package chi\n"}}}}"#,
            path.display()
        ))
        .unwrap();
        let edit = from_host_payload(&payload);
        assert!(
            edit.removed_names.iter().any(|n| n == "NewRouter"),
            "Write must compare disk old vs content, names={:?}",
            edit.removed_names
        );
    }

    #[test]
    fn unified_diff_hunk() {
        let diff = r#"--- a/chi.go
+++ b/chi.go
@@ -10,4 +10,1 @@
-func NewRouter() *Mux {
-  return &Mux{}
-}
 package chi
"#;
        let edits = from_unified_diff(diff);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].file, "chi.go");
        assert!(edits[0].removed_names.iter().any(|n| n == "NewRouter"));
    }
}
