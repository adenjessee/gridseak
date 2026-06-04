//! Sprint E.4 — Trigger event surfacing end-to-end.
//!
//! The `trigger_events` YAML query in `configs/apex.yaml` was dead
//! code until this sprint: the query compiled, but no extractor
//! bound to it, so `Node.properties.trigger_events` never appeared
//! on trigger struct nodes. Analysis downstream (workload classifier,
//! before/after fanout metrics) had nothing to read.
//!
//! This test pins three behaviours:
//! 1. events are populated on the trigger Struct;
//! 2. their order matches source order (some analysis cares about
//!    insertion-first vs. update-first timing);
//! 3. the events property is absent, not empty, when no events are
//!    declared (keeps output tight and avoids ambiguity between
//!    "no events" and "events not surfaced").

use graphengine_parsing::application::ports::SyntaxExtractor;
use graphengine_parsing::domain::NodeKind;
use graphengine_parsing::infrastructure::config::load_config;
use graphengine_parsing::syntax::treesitter::TreeSitterExtractor;
use std::path::PathBuf;
use tempfile::TempDir;

fn write_trigger(dir: &std::path::Path, name: &str, src: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, src).expect("write");
    p
}

async fn parse(files: Vec<PathBuf>) -> graphengine_parsing::application::ports::SyntaxResults {
    let config = load_config("apex").expect("load apex config");
    let extractor = TreeSitterExtractor::new(config).expect("extractor");
    extractor.extract(&files).await.expect("parse")
}

#[tokio::test]
async fn trigger_events_populated_in_source_order() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_trigger(
        tmp.path(),
        "AccountTrigger.trigger",
        "\
trigger AccountTrigger on Account (before insert, after update, before delete) {
    // body intentionally empty
}
",
    );
    let hints = parse(vec![path]).await;

    let trigger_struct = hints
        .symbols
        .iter()
        .find(|n| {
            n.kind == NodeKind::Struct
                && n.fqn.ends_with("::AccountTrigger")
                && n.properties.get("subtype").and_then(|v| v.as_str()) == Some("trigger")
        })
        .expect("trigger Struct node missing");

    let events = trigger_struct
        .properties
        .get("trigger_events")
        .and_then(|v| v.as_array())
        .expect("trigger_events property missing");
    let event_strs: Vec<&str> = events.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        event_strs,
        vec!["before insert", "after update", "before delete"],
        "events must preserve source order; got {event_strs:?}"
    );
}

#[tokio::test]
async fn trigger_events_property_absent_when_no_events_declared() {
    // A trigger with no declared events must NOT gain an empty array
    // property. Absent > empty keeps "events not surfaced" and "no
    // events declared" impossible to confuse downstream.
    let tmp = TempDir::new().expect("tempdir");
    let path = write_trigger(
        tmp.path(),
        "EmptyTrigger.trigger",
        "\
trigger EmptyTrigger on Contact () {
    // no events, no body
}
",
    );
    let hints = parse(vec![path]).await;
    let trigger_struct = hints
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with("::EmptyTrigger"))
        .expect("trigger Struct node missing");

    assert!(
        !trigger_struct.properties.contains_key("trigger_events"),
        "events property should be absent when none declared; got {:?}",
        trigger_struct.properties.get("trigger_events")
    );
}

#[tokio::test]
async fn non_trigger_class_has_no_trigger_events_property() {
    // Defensive: ordinary classes must never receive `trigger_events`
    // (guards against the `extract_trigger_metadata` hook leaking
    // into the non-trigger struct metadata path).
    let tmp = TempDir::new().expect("tempdir");
    let path = write_trigger(
        tmp.path(),
        "Foo.cls",
        "public class Foo { public void go() {} }",
    );
    let hints = parse(vec![path]).await;

    let class_struct = hints
        .symbols
        .iter()
        .find(|n| n.kind == NodeKind::Struct && n.fqn.ends_with("::Foo"))
        .expect("class Struct missing");

    assert!(
        !class_struct.properties.contains_key("trigger_events"),
        "non-trigger class must not carry trigger_events; got {:?}",
        class_struct.properties
    );
    assert_ne!(
        class_struct
            .properties
            .get("subtype")
            .and_then(|v| v.as_str()),
        Some("trigger"),
    );
}
