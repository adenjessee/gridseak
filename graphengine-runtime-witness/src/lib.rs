//! Coverage + trace ingest. Observed edges are `p_true = 1.0`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeExecuted {
    pub file: String,
    pub line: u32,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeObserved {
    pub caller: String,
    pub callee: String,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEvidence {
    pub executed: Vec<NodeExecuted>,
    pub observed: Vec<EdgeObserved>,
}

pub fn ingest_istanbul(json: &str) -> Result<RuntimeEvidence, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("istanbul: {e}"))?;
    let mut ev = RuntimeEvidence::default();
    if let Some(map) = v.as_object() {
        for (file, body) in map {
            if let Some(s) = body.get("s").and_then(|x| x.as_object()) {
                for (_id, count) in s {
                    if count.as_u64().unwrap_or(0) > 0 {
                        ev.executed.push(NodeExecuted {
                            file: file.clone(),
                            line: 1,
                            symbol: None,
                        });
                        break;
                    }
                }
            }
        }
    }
    Ok(ev)
}

pub fn ingest_coverage_py(json: &str) -> Result<RuntimeEvidence, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("coverage.py: {e}"))?;
    let mut ev = RuntimeEvidence::default();
    if let Some(files) = v.get("files").and_then(|x| x.as_object()) {
        for (file, body) in files {
            if let Some(lines) = body.get("executed_lines").and_then(|x| x.as_array()) {
                for line in lines {
                    ev.executed.push(NodeExecuted {
                        file: file.clone(),
                        line: line.as_u64().unwrap_or(0) as u32,
                        symbol: None,
                    });
                }
            }
        }
    }
    Ok(ev)
}

pub fn ingest_go_cover(text: &str) -> Result<RuntimeEvidence, String> {
    let mut ev = RuntimeEvidence::default();
    for line in text.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts.last().copied() != Some("0") {
            let loc = parts[0];
            let file = loc.split(':').next().unwrap_or(loc);
            ev.executed.push(NodeExecuted {
                file: file.to_string(),
                line: 1,
                symbol: None,
            });
        }
    }
    Ok(ev)
}

pub fn ingest_jacoco_xml(xml: &str) -> Result<RuntimeEvidence, String> {
    let mut ev = RuntimeEvidence::default();
    for cap in xml.split("sourcefilename=\"").skip(1) {
        let file = cap.split('"').next().unwrap_or("unknown");
        ev.executed.push(NodeExecuted {
            file: file.to_string(),
            line: 1,
            symbol: None,
        });
    }
    Ok(ev)
}

pub fn ingest_coverlet(json: &str) -> Result<RuntimeEvidence, String> {
    ingest_istanbul(json)
}

pub fn ingest_cprofile(json: &str) -> Result<RuntimeEvidence, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("cprofile: {e}"))?;
    let mut ev = RuntimeEvidence::default();
    if let Some(rows) = v.as_array() {
        for row in rows {
            let caller = row.get("caller").and_then(|x| x.as_str()).unwrap_or("");
            let callee = row.get("callee").and_then(|x| x.as_str()).unwrap_or("");
            let count = row.get("count").and_then(|x| x.as_u64()).unwrap_or(1);
            if !caller.is_empty() && !callee.is_empty() {
                ev.observed.push(EdgeObserved {
                    caller: caller.to_string(),
                    callee: callee.to_string(),
                    count,
                });
            }
        }
    }
    Ok(ev)
}

pub fn ingest_pprof_folded(text: &str) -> Result<RuntimeEvidence, String> {
    let mut ev = RuntimeEvidence::default();
    for line in text.lines() {
        let frames: Vec<&str> = line.split_whitespace().collect();
        if frames.len() < 2 {
            continue;
        }
        for w in frames.windows(2).take(frames.len().saturating_sub(1)) {
            if w[1].chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            ev.observed.push(EdgeObserved {
                caller: w[0].to_string(),
                callee: w[1].to_string(),
                count: 1,
            });
        }
    }
    Ok(ev)
}

pub fn ingest_cpu_prof_json(json: &str) -> Result<RuntimeEvidence, String> {
    ingest_cprofile(json)
}

pub fn ingest_jfr_summary(json: &str) -> Result<RuntimeEvidence, String> {
    ingest_cprofile(json)
}

/// Dead-code confidence ceiling: if coverage shows the node executed,
/// static "dead" claims cannot exceed this confidence.
pub fn dead_code_ceiling(executed: bool) -> f64 {
    if executed {
        0.0
    } else {
        0.55
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn istanbul_counts_executed_file() {
        let raw = r#"{"src/a.ts":{"s":{"1":3}}}"#;
        let ev = ingest_istanbul(raw).unwrap();
        assert_eq!(ev.executed.len(), 1);
    }

    #[test]
    fn coverage_py_reads_lines() {
        let raw = r#"{"files":{"a.py":{"executed_lines":[1,2]}}}"#;
        let ev = ingest_coverage_py(raw).unwrap();
        assert_eq!(ev.executed.len(), 2);
    }

    #[test]
    fn go_cover_skips_zeros() {
        let raw = "mode: set\nfoo.go:1.1,2.2 1 1\nbar.go:1.1,2.2 1 0\n";
        let ev = ingest_go_cover(raw).unwrap();
        assert_eq!(ev.executed.len(), 1);
    }

    #[test]
    fn jacoco_reads_sourcefilename() {
        let raw = r#"<report><sourcefilename="Foo.java"/></report>"#;
        let ev = ingest_jacoco_xml(raw).unwrap();
        assert_eq!(ev.executed[0].file, "Foo.java");
    }

    #[test]
    fn cprofile_edges() {
        let raw = r#"[{"caller":"a","callee":"b","count":4}]"#;
        let ev = ingest_cprofile(raw).unwrap();
        assert_eq!(ev.observed[0].count, 4);
    }

    #[test]
    fn dead_ceiling_zero_when_executed() {
        assert_eq!(dead_code_ceiling(true), 0.0);
        assert!(dead_code_ceiling(false) > 0.0);
    }
}
