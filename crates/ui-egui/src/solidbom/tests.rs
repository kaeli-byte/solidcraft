//! The desktop side of the read-only review policy.

use solidbom_contract::{AnalysisKey, SourceRef};
use solidcraft_engine::solidbom::ReviewPolicy;

fn policy() -> ReviewPolicy {
    ReviewPolicy::new(
        SourceRef { source_id: uuid::Uuid::nil(), sha256: "a".repeat(64), byte_len: 1, original_name: "three_boxes.step".into() },
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: "test".into(), extractor_version: "1".into() },
    )
}

#[test]
fn a_bound_session_does_not_open_a_modelling_tool() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.session.bind_review(policy());
    let revision = harness.app.session.revision;
    harness.app.start("solid.box");
    assert!(harness.app.dialog.is_none(), "no modelling dialog opens");
    assert_eq!(harness.app.session.revision, revision);
    assert!(harness.app.run("solid.box", serde_json::json!({})).is_err());
    assert_eq!(harness.app.session.revision, revision);
}

#[test]
fn an_unbound_session_still_opens_the_tool() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.start("solid.box");
    assert!(harness.app.dialog.is_some());
}
