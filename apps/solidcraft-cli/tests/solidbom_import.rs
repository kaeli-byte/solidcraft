//! A programmatic CLI run of the SolidBOM import is not left pending: the headless CLI waits for
//! the background source analysis, so the import reply and the following steps see the review.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{Value, json};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/three_boxes.step")
}

fn output_json(output: Output) -> Value {
    assert!(output.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("the CLI prints JSON")
}

#[test]
fn a_script_import_waits_for_the_analysis_and_the_next_step_sees_the_review() {
    let dir = std::env::temp_dir().join(format!("solidcraft-cli-solidbom-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary folder");
    let script = dir.join("review.json");
    let path = fixture();
    std::fs::write(
        &script,
        json!({"commands": [
            {"command": "solidbom.import", "params": {"path": path.to_string_lossy()}},
            {"command": "solidbom.state"},
        ]})
        .to_string(),
    )
    .expect("the script");
    let results =
        output_json(Command::new(env!("CARGO_BIN_EXE_solidcraft-cli")).arg("run").arg(&script).arg("--results").output().expect("the CLI runs"));
    let steps = results.as_array().expect("one result per step");
    assert_eq!(steps[0]["state"], json!("reviewing"), "the import completed: {results}");
    assert_eq!(steps[0]["definitions"], json!(1), "{results}");
    assert_eq!(steps[0]["occurrences"], json!(3), "{results}");
    assert_eq!(steps[1]["phase"], json!("reviewing"), "the next step sees the review: {results}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_executed_import_waits_for_the_analysis() {
    let path = fixture();
    let reply = output_json(
        Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
            .arg("exec")
            .arg("solidbom.import")
            .arg(json!({"path": path.to_string_lossy()}).to_string())
            .output()
            .expect("the CLI runs"),
    );
    assert_eq!(reply["state"], json!("reviewing"), "the import completed: {reply}");
    assert_eq!(reply["definitions"], json!(1), "{reply}");
    assert_eq!(reply["occurrences"], json!(3), "{reply}");
    assert_eq!(reply["part_version_ids"].as_array().map(Vec::len), Some(1), "{reply}");
}
