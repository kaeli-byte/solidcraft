//! SolidBOM's published command schemas and discovery: the ten `solidbom.*` ids, their input and
//! output schemas, and the offline behavior of a session with no service.

use serde_json::{Value, json};

use super::super::tests::{assert_keys, conforms};
use super::{BatchInputsParams, EmptyParams, ImportParams, ImportReply, InputsParams, JobReply, PartsReply, ReopenParams, StateReply};
use crate::Session;
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

/// The version 1 command ids, sorted. Adding one is a contract change reviewed with downstream
/// consumers; renaming or removing one is a breaking change.
const PUBLISHED: [&str; 10] = [
    "solidbom.calculate",
    "solidbom.import",
    "solidbom.inputs",
    "solidbom.inputs.batch",
    "solidbom.parts",
    "solidbom.reopen",
    "solidbom.retry",
    "solidbom.review",
    "solidbom.save",
    "solidbom.state",
];

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<ImportParams>(), &["path"]);
    assert_keys(
        input_schema_for::<InputsParams>(),
        &[
            "allowance_rate",
            "cavities",
            "classification",
            "classification_confirmed",
            "cycle_seconds",
            "manual_reason",
            "material_overhead_rate",
            "material_version_id",
            "part_version_id",
            "production_rate_version_id",
            "setup_hours",
            "setup_rate_version_id",
            "sourcing",
            "volume_confirmed",
            "volume_mm3",
        ],
    );
    assert_keys(input_schema_for::<BatchInputsParams>(), &["material_overhead_rate", "parts"]);
    assert_keys(input_schema_for::<EmptyParams>(), &[]);
    assert_keys(input_schema_for::<ReopenParams>(), &["project_id"]);
}

#[test]
fn the_solidbom_command_ids_are_exactly_the_published_set() {
    let mut found: Vec<&str> = crate::command_specs().iter().filter(|spec| spec.id.starts_with("solidbom.")).map(|spec| spec.id).collect();
    found.sort_unstable();
    assert_eq!(found, PUBLISHED);
    for id in PUBLISHED {
        let spec = crate::find_command(id).unwrap_or_else(|| panic!("{id} is registered"));
        assert!(spec.contract.is_some(), "{id} publishes no input contract");
        assert!(spec.output.is_some(), "{id} publishes no output schema");
        assert!(!spec.undoable, "{id} must not enter CAD undo");
        assert!(!spec.label.is_empty(), "{id} has no label");
        assert!(!spec.params.is_empty(), "{id} has no parameter description");
    }
}

#[test]
fn a_review_state_reply_matches_its_published_output_schema() {
    let schema = output_schema_for::<StateReply>().as_value().clone();
    let reply = json!({
        "phase": "idle",
        "save": "clean",
        "result": "no_result",
        "project_id": null,
        "version": 0,
        "definitions": 0,
        "occurrences": 0,
        "issues": [],
        "calculations": [],
        "current_calculation": null,
        "assembly_subtotal": null,
        "per_part_subtotal": null,
        "setup_total": null,
        "volume_mm3": null,
        "volume_source": null,
        "volume_review": null,
        "part_costs": [],
        "error": null,
        "notice": null,
        "needs_service": false,
        "retryable": false,
    });
    assert!(conforms(&schema, &schema, &reply), "{reply} does not match {schema}");
    for module in [output_schema_for::<ImportReply>(), output_schema_for::<PartsReply>(), output_schema_for::<JobReply>()] {
        let schema = module.as_value().clone();
        assert!(schema.get("properties").is_some(), "an output schema needs properties: {schema}");
    }
}

#[test]
fn the_input_schema_takes_decimal_strings_and_refuses_unknown_keys() {
    let schema = input_schema_for::<InputsParams>().as_value().clone();
    assert!(conforms(&schema, &schema, &json!({"part_version_id": "id", "volume_mm3": "2400", "allowance_rate": "0.02", "cavities": 1})));
    assert!(!conforms(&schema, &schema, &json!({"part_version_id": "id", "volume_mm3": 2400})), "a decimal is a string");
    assert_eq!(schema.get("additionalProperties"), Some(&Value::Bool(false)), "the schema refuses unknown keys");
}

#[test]
fn strict_refuses_an_unknown_key_before_a_solidbom_handler() {
    for (id, params) in [
        ("solidbom.import", json!({"not_a_solidbom_key": 1})),
        ("solidbom.inputs", json!({"not_a_solidbom_key": 1})),
        ("solidbom.inputs.batch", json!({"not_a_solidbom_key": 1})),
        ("solidbom.parts", json!({"not_a_solidbom_key": 1})),
        ("solidbom.state", json!({"not_a_solidbom_key": 1})),
        ("solidbom.review", json!({"not_a_solidbom_key": 1})),
    ] {
        let mut session = Session { param_check: ParamCheck::Strict, ..Session::default() };
        let error = session.execute(id, &params).expect_err("an unknown key is refused in strict");
        assert!(error.to_string().contains("unknown key"), "{id}: {error}");
    }
}

#[test]
fn a_review_session_without_a_service_reports_the_missing_capability() {
    let error = Session::default()
        .execute("solidbom.reopen", &json!({"project_id": "00000000-0000-4000-8000-000000000001"}))
        .expect_err("there is no service");
    assert!(error.to_string().contains(crate::solidbom::state::SERVICE_MISSING), "{error}");
    let info = crate::find_command("solidbom.reopen").expect("reopen").info(&Session::default());
    assert!(!info.enabled);
    assert_eq!(info.disabled_reason.as_deref(), Some(crate::solidbom::state::SERVICE_MISSING));
}
