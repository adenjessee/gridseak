//! Machine-readable JSON envelope around [`ScanReportView`].
//!
//! Why a wrapper instead of just printing the raw `HealthReport`:
//! every other renderer in this module renders the *view*, not the
//! report. Keeping the JSON renderer consistent means a JSON consumer
//! gets the same hero number, the same priority rows, and the same
//! status labels the terminal user sees. The full unmodified
//! `HealthReport` remains available through `gridseak scan report
//! [PATH] --full` — that path is for tooling that needs every field.

use serde::Serialize;
use serde_json::Value;
use std::io::{self, Write};

use crate::render::view::ScanReportView;

/// Stable JSON envelope. The `schema` field lets consumers gate on
/// breaking changes; bump it whenever a field meaning changes.
#[derive(Debug, Serialize)]
pub struct ScanReportEnvelope<'a> {
    schema: &'static str,
    view: &'a ScanReportView,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_report: Option<&'a Value>,
}

impl<'a> ScanReportEnvelope<'a> {
    pub fn new(view: &'a ScanReportView) -> Self {
        Self {
            schema: "gridseak.scan_report.v1",
            view,
            raw_report: None,
        }
    }

    /// Attach the raw `HealthReport` value. Used by `--full` mode
    /// (wired in Stage 5 when `gridseak scan report --full` lands).
    #[allow(dead_code)]
    pub fn with_raw_report(mut self, raw: &'a Value) -> Self {
        self.raw_report = Some(raw);
        self
    }
}

/// Write the JSON envelope to `out` as pretty-printed JSON.
pub fn render_hero(view: &ScanReportView, out: &mut dyn Write) -> io::Result<()> {
    let envelope = ScanReportEnvelope::new(view);
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

#[allow(dead_code)] // exercised by unit tests; reserved for snapshot harness.
pub fn render_hero_to_string(view: &ScanReportView) -> String {
    let mut buf = Vec::with_capacity(2048);
    render_hero(view, &mut buf).expect("Vec<u8> writes never fail");
    String::from_utf8(buf).expect("JSON renderer emits valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::view::fixture_spec_example;

    #[test]
    fn json_envelope_round_trips() {
        let s = render_hero_to_string(&fixture_spec_example());
        let parsed: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["schema"], "gridseak.scan_report.v1");
        assert_eq!(parsed["view"]["repo_name"], "my-service");
        assert_eq!(parsed["view"]["score"], 62);
        assert_eq!(parsed["view"]["priorities"][0]["finding_id"], "finding_001");
    }

    /// Stage 3 snapshot lock for the JSON envelope. The shape is
    /// part of the contract with downstream tooling; this test
    /// captures the literal bytes so any drift is reviewed.
    #[test]
    fn hero_json_snapshot() {
        let rendered = render_hero_to_string(&fixture_spec_example());
        insta::assert_snapshot!("hero_json", rendered);
    }
}
