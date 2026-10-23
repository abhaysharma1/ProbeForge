//! Cross-language contract fixtures. The TypeScript test suite runs the same
//! manifest with Ajv, so schema and Rust validator drift fails both suites.

use std::fs;
use std::path::PathBuf;

use probeforge_contracts::{
    ContractError, JobCreation, MAX_CONTRACT_BYTES, parse_and_validate_json, validate_job_creation,
    validate_job_event, validate_load_profile, validate_load_scenario, validate_plan,
    validate_run_request, validate_target_authorization, validate_test_result,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(rename = "schemaVersion")]
    _schema_version: String,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    validator: String,
    path: String,
    valid: bool,
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/contracts/fixtures/v1")
}

fn load_manifest() -> Manifest {
    let raw = fs::read_to_string(fixture_dir().join("manifest.json"))
        .expect("shared fixture manifest must be readable");
    serde_json::from_str(&raw).expect("shared fixture manifest must be valid")
}

fn accepted(validator: &str, value: &Value) -> bool {
    match validator {
        "jobCreation" => validate_job_creation(value).is_ok(),
        "plan" => validate_plan(value).is_ok(),
        "loadScenario" => validate_load_scenario(value).is_ok(),
        "loadProfile" => validate_load_profile(value).is_ok(),
        "targetAuthorization" => validate_target_authorization(value).is_ok(),
        "runRequest" => validate_run_request(value).is_ok(),
        "testResult" => validate_test_result(value).is_ok(),
        "jobEvent" => validate_job_event(value).is_ok(),
        other => panic!("unknown fixture validator: {other}"),
    }
}

#[test]
fn shared_contract_fixtures_match_rust_validators() {
    let manifest = load_manifest();
    assert!(
        !manifest.cases.is_empty(),
        "fixture manifest must not be empty"
    );

    for case in manifest.cases {
        let raw = fs::read_to_string(fixture_dir().join(&case.path))
            .unwrap_or_else(|error| panic!("{}: {error}", case.path));
        let value: Value =
            serde_json::from_str(&raw).unwrap_or_else(|error| panic!("{}: {error}", case.path));
        let accepted = accepted(&case.validator, &value);

        assert_eq!(
            accepted,
            case.valid,
            "{} should have been {}",
            case.name,
            if case.valid { "accepted" } else { "rejected" }
        );
    }
}

#[test]
fn valid_job_fixture_parses_through_bounded_entrypoint() {
    let raw = fs::read_to_string(fixture_dir().join("job.vulnerability.valid.json"))
        .expect("fixture must be readable");

    let job: JobCreation = parse_and_validate_json(raw.as_bytes()).expect("valid job must parse");
    assert_eq!(job.display_name, "Payments API review");
}

#[test]
fn oversized_payloads_fail_before_deserialization() {
    let payload = vec![b' '; MAX_CONTRACT_BYTES + 1];

    let result = parse_and_validate_json::<JobCreation>(&payload);
    assert!(matches!(result, Err(ContractError::TooLarge { .. })));
}
