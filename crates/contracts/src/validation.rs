//! Deterministic validation for contract payloads.
//!
//! Validation happens before any persistence or execution. Errors intentionally
//! contain paths and rule descriptions, never the offending values, so a target
//! URL or credential reference cannot leak through an error response.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;

use crate::model::{
    CONTRACT_SCHEMA_VERSION, HttpMethod, JobCreation, JobEvent, LoadAssertion,
    LoadAssertionOperator, LoadAssertionSource, LoadExtraction, LoadExtractionSource, LoadProfile,
    LoadScenario, LoadStopConditions, MAX_CONTRACT_BYTES, Plan, PlannedTest, RunRequest,
    TargetAuthorization, TargetLimits, TargetScope, TestOutcome, TestResult,
};

/// A single deterministic validation problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationIssue {
    pub path: String,
    pub message: String,
}

/// Stable error returned by contract parsing and validation.
#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("payload exceeds {actual_bytes} bytes; maximum is {max_bytes}")]
    TooLarge {
        actual_bytes: usize,
        max_bytes: usize,
    },
    #[error("contract validation failed: {message}")]
    Invalid {
        message: String,
        issues: Vec<ValidationIssue>,
    },
}

impl ContractError {
    fn from_issues(issues: Vec<ValidationIssue>) -> Self {
        let message = issues
            .first()
            .map(|issue| format!("{}: {}", issue.path, issue.message))
            .unwrap_or_else(|| "unknown validation failure".to_owned());
        Self::Invalid { message, issues }
    }

    fn from_deserialize(error: serde_json::Error) -> Self {
        // Do not include serde's rendered value in the error: it can echo target
        // URLs, credential references, or other untrusted payload text.
        Self::from_issues(vec![issue(
            "/",
            format!(
                "payload does not match the contract shape (line {}, column {})",
                error.line(),
                error.column()
            ),
        )])
    }

    /// Individual issues, empty for size failures.
    pub fn issues(&self) -> &[ValidationIssue] {
        match self {
            Self::TooLarge { .. } => &[],
            Self::Invalid { issues, .. } => issues,
        }
    }
}

/// Data-only contracts that know how to validate their own invariants.
pub trait Validate {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>>;
}

/// Parse a bounded UTF-8 JSON payload and apply both structural and semantic
/// validation. Callers must reject the payload when this returns an error.
pub fn parse_and_validate_json<T>(raw: &[u8]) -> Result<T, ContractError>
where
    T: DeserializeOwned + Validate,
{
    if raw.len() > MAX_CONTRACT_BYTES {
        return Err(ContractError::TooLarge {
            actual_bytes: raw.len(),
            max_bytes: MAX_CONTRACT_BYTES,
        });
    }

    let parsed: T = serde_json::from_slice(raw).map_err(ContractError::from_deserialize)?;
    parsed.validate().map_err(ContractError::from_issues)?;
    Ok(parsed)
}

pub fn validate_job_creation(value: &Value) -> Result<JobCreation, ContractError> {
    deserialize_and_validate::<JobCreation>(value)
}

pub fn validate_plan(value: &Value) -> Result<Plan, ContractError> {
    deserialize_and_validate::<Plan>(value)
}

pub fn validate_load_scenario(value: &Value) -> Result<LoadScenario, ContractError> {
    deserialize_and_validate::<LoadScenario>(value)
}

pub fn validate_load_profile(value: &Value) -> Result<LoadProfile, ContractError> {
    deserialize_and_validate::<LoadProfile>(value)
}

pub fn validate_target_authorization(value: &Value) -> Result<TargetAuthorization, ContractError> {
    deserialize_and_validate::<TargetAuthorization>(value)
}

/// Validate only the target scope shape and invariants. API target
/// registration uses this before applying stricter destination/port policy.
pub fn validate_target_scope(scope: &TargetScope) -> Result<(), ContractError> {
    let mut issues = Vec::new();
    validate_scope(scope, &mut issues);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(ContractError::from_issues(issues))
    }
}

/// Validate only the authorization limits, independent of a full authorization.
pub fn validate_target_limits(limits: &TargetLimits) -> Result<(), ContractError> {
    let mut issues = Vec::new();
    validate_target_limits_fields(limits, &mut issues);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(ContractError::from_issues(issues))
    }
}

pub fn validate_run_request(value: &Value) -> Result<RunRequest, ContractError> {
    deserialize_and_validate::<RunRequest>(value)
}

pub fn validate_test_result(value: &Value) -> Result<TestResult, ContractError> {
    deserialize_and_validate::<TestResult>(value)
}

pub fn validate_job_event(value: &Value) -> Result<JobEvent, ContractError> {
    deserialize_and_validate::<JobEvent>(value)
}

fn deserialize_and_validate<T>(value: &Value) -> Result<T, ContractError>
where
    T: DeserializeOwned + Validate,
{
    let parsed: T =
        serde_json::from_value(value.clone()).map_err(ContractError::from_deserialize)?;
    parsed.validate().map_err(ContractError::from_issues)?;
    Ok(parsed)
}

impl Validate for JobCreation {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        check_schema_version(&self.schema_version, &mut issues);
        check_text(&self.display_name, "/displayName", 120, &mut issues);
        check_repository_url(&self.git.repository_url, &mut issues);
        check_repository_reference(&self.git.reference, &mut issues);
        if let Some(reference) = &self.git.credential_ref {
            check_credential_ref(reference, "/git/credentialRef", &mut issues);
        }
        finish(issues)
    }
}

impl Validate for LoadProfile {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        validate_load_profile_fields(self, "", &mut issues);
        finish(issues)
    }
}

impl Validate for LoadScenario {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        validate_load_scenario_fields(self, "", &mut issues);
        finish(issues)
    }
}

impl Validate for PlannedTest {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        validate_planned_test_fields(self, "", &mut issues);
        finish(issues)
    }
}

impl Validate for Plan {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        check_schema_version(&self.schema_version, &mut issues);
        check_text(&self.summary, "/summary", 8_000, &mut issues);

        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            issues.push(issue(
                "/confidence",
                "confidence must be a finite number between 0 and 1",
            ));
        }

        if self.planned_tests.len() > 100 {
            issues.push(issue(
                "/plannedTests",
                "at most 100 planned tests are allowed",
            ));
        }
        if self.load_scenarios.len() > 50 {
            issues.push(issue(
                "/loadScenarios",
                "at most 50 load scenarios are allowed",
            ));
        }
        check_string_list(
            &self.required_inputs,
            "/requiredInputs",
            20,
            500,
            &mut issues,
        );
        check_string_list(&self.limitations, "/limitations", 20, 1_000, &mut issues);
        check_string_list(
            &self.evidence_references,
            "/evidenceReferences",
            50,
            500,
            &mut issues,
        );

        match self.goal {
            crate::model::JobGoal::VulnerabilityTesting => {
                if self.planned_tests.is_empty() {
                    issues.push(issue(
                        "/plannedTests",
                        "vulnerability plans must propose at least one test",
                    ));
                }
                if !self.load_scenarios.is_empty() {
                    issues.push(issue(
                        "/loadScenarios",
                        "vulnerability plans must not include load scenarios",
                    ));
                }
            }
            crate::model::JobGoal::LoadTesting => {
                if self.load_scenarios.is_empty() {
                    issues.push(issue(
                        "/loadScenarios",
                        "load plans must propose at least one scenario",
                    ));
                }
                if !self.planned_tests.is_empty() {
                    issues.push(issue(
                        "/plannedTests",
                        "load plans must not include vulnerability tests",
                    ));
                }
            }
        }

        for (index, test) in self.planned_tests.iter().enumerate() {
            validate_planned_test_fields(test, &format!("/plannedTests/{index}"), &mut issues);
        }
        for (index, scenario) in self.load_scenarios.iter().enumerate() {
            validate_load_scenario_fields(
                scenario,
                &format!("/loadScenarios/{index}"),
                &mut issues,
            );
        }

        finish(issues)
    }
}

impl Validate for TargetAuthorization {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        check_schema_version(&self.schema_version, &mut issues);
        check_non_nil_uuid(&self.target_id, "/targetId", &mut issues);

        if !self.confirmed {
            issues.push(issue(
                "/confirmed",
                "target authorization requires explicit confirmation",
            ));
        }

        validate_scope(&self.scope, &mut issues);
        validate_target_limits_fields(&self.limits, &mut issues);

        if let (Some(valid_from), Some(expires_at)) = (self.valid_from, self.expires_at)
            && expires_at <= valid_from
        {
            issues.push(issue(
                "/expiresAt",
                "expiresAt must be later than validFrom",
            ));
        }

        finish(issues)
    }
}

impl Validate for RunRequest {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        check_schema_version(&self.schema_version, &mut issues);
        check_non_nil_uuid(&self.job_id, "/jobId", &mut issues);
        check_non_nil_uuid(&self.plan_id, "/planId", &mut issues);
        check_non_nil_uuid(
            &self.target_authorization_id,
            "/targetAuthorizationId",
            &mut issues,
        );
        check_non_nil_uuid(&self.idempotency_key, "/idempotencyKey", &mut issues);

        if self.plan_version < 1 {
            issues.push(issue("/planVersion", "planVersion must be at least 1"));
        }

        let planned_tests = self.planned_test_ids.as_deref().unwrap_or_default();
        let load_scenarios = self.load_scenario_ids.as_deref().unwrap_or_default();

        match self.run_type {
            crate::model::JobGoal::VulnerabilityTesting => {
                if planned_tests.is_empty() {
                    issues.push(issue(
                        "/plannedTestIds",
                        "vulnerability runs require at least one planned test",
                    ));
                }
                if !load_scenarios.is_empty() {
                    issues.push(issue(
                        "/loadScenarioIds",
                        "vulnerability runs must not include load scenarios",
                    ));
                }
            }
            crate::model::JobGoal::LoadTesting => {
                if load_scenarios.is_empty() {
                    issues.push(issue(
                        "/loadScenarioIds",
                        "load runs require at least one load scenario",
                    ));
                }
                if !planned_tests.is_empty() {
                    issues.push(issue(
                        "/plannedTestIds",
                        "load runs must not include vulnerability tests",
                    ));
                }
            }
        }

        if planned_tests.len() > 100 {
            issues.push(issue(
                "/plannedTestIds",
                "at most 100 planned tests are allowed",
            ));
        }
        if load_scenarios.len() > 50 {
            issues.push(issue(
                "/loadScenarioIds",
                "at most 50 load scenarios are allowed",
            ));
        }
        check_unique_uuids(planned_tests, "/plannedTestIds", &mut issues);
        check_unique_uuids(load_scenarios, "/loadScenarioIds", &mut issues);

        finish(issues)
    }
}

impl Validate for TestResult {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        check_schema_version(&self.schema_version, &mut issues);
        check_non_nil_uuid(&self.run_id, "/runId", &mut issues);
        check_catalog_test_id(&self.catalog_test_id, "/catalogTestId", &mut issues);
        check_text(&self.summary, "/summary", 8_000, &mut issues);

        if self.duration_ms > 3_600_000 {
            issues.push(issue("/durationMs", "durationMs must not exceed 3600000"));
        }

        if let Some(confidence) = self.confidence
            && (!confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
        {
            issues.push(issue(
                "/confidence",
                "confidence must be a finite number between 0 and 1",
            ));
        }

        if self.outcome == TestOutcome::Failed && self.severity.is_none() {
            issues.push(issue("/severity", "failed results must include a severity"));
        }

        if !self.details.is_object() {
            issues.push(issue("/details", "details must be a JSON object"));
        }
        check_safe_json(&self.details, "/details", 0, &mut issues);

        if let Some(tool) = &self.tool {
            check_text(&tool.name, "/tool/name", 120, &mut issues);
            check_text(&tool.version, "/tool/version", 80, &mut issues);
        }

        finish(issues)
    }
}

impl Validate for JobEvent {
    fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        check_schema_version(&self.schema_version, &mut issues);
        check_non_nil_uuid(&self.job_id, "/jobId", &mut issues);

        if self.sequence < 1 {
            issues.push(issue("/sequence", "sequence must be at least 1"));
        }

        check_safe_json(&self.payload, "/payload", 0, &mut issues);
        finish(issues)
    }
}

fn validate_planned_test_fields(test: &PlannedTest, path: &str, issues: &mut Vec<ValidationIssue>) {
    let path = |field: &str| format!("{path}/{field}");
    check_catalog_test_id(&test.catalog_test_id, &path("catalogTestId"), issues);
    check_text(&test.name, &path("name"), 160, issues);
    check_text(&test.rationale, &path("rationale"), 2_000, issues);
    check_text(
        &test.expected_evidence,
        &path("expectedEvidence"),
        1_000,
        issues,
    );
    check_string_list(&test.prerequisites, &path("prerequisites"), 20, 300, issues);
    check_string_list(
        &test.target_requirements,
        &path("targetRequirements"),
        20,
        300,
        issues,
    );
    check_string_list(
        &test.rationale_references,
        &path("rationaleReferences"),
        20,
        500,
        issues,
    );
    if !test.configuration.is_object() {
        issues.push(issue(
            path("configuration"),
            "configuration must be a JSON object",
        ));
    }
    check_safe_json(&test.configuration, &path("configuration"), 0, issues);
}

fn validate_load_scenario_fields(
    scenario: &LoadScenario,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    const MAX_LOAD_REQUESTS: usize = 50;
    const MAX_LOAD_ITEMS: usize = 20;
    const MAX_LOAD_BODY_BYTES: usize = 64 * 1024;

    let path = |field: &str| format!("{path}/{field}");
    check_text(&scenario.name, &path("name"), 160, issues);
    check_text(&scenario.description, &path("description"), 2_000, issues);

    if scenario.requests.is_empty() {
        issues.push(issue(
            path("requests"),
            "a load scenario requires at least one request",
        ));
    }
    if scenario.requests.len() > MAX_LOAD_REQUESTS {
        issues.push(issue(
            path("requests"),
            format!("at most {MAX_LOAD_REQUESTS} requests are allowed"),
        ));
    }

    let mut prior_step_ids = BTreeSet::new();
    let mut seen_step_ids = BTreeSet::new();
    let mut extracted_names = BTreeSet::new();

    for (index, request) in scenario.requests.iter().enumerate() {
        let request_path = path(&format!("requests/{index}"));
        let request_path_field = format!("{request_path}/path");

        if let Some(step_id) = &request.id {
            if !is_load_step_id(step_id) {
                issues.push(issue(
                    format!("{request_path}/id"),
                    "step ids must start with a lowercase letter and contain only lowercase letters, digits, underscores, or hyphens",
                ));
            }
            if !seen_step_ids.insert(step_id.clone()) {
                issues.push(issue(
                    format!("{request_path}/id"),
                    "step ids must be unique within a load scenario",
                ));
            }
        }

        if request.depends_on.len() > MAX_LOAD_ITEMS {
            issues.push(issue(
                format!("{request_path}/dependsOn"),
                format!("at most {MAX_LOAD_ITEMS} dependencies are allowed"),
            ));
        }
        let mut seen_dependencies = BTreeSet::new();
        for dependency in &request.depends_on {
            if !seen_dependencies.insert(dependency) {
                issues.push(issue(
                    format!("{request_path}/dependsOn"),
                    "dependencies must be unique",
                ));
                continue;
            }
            if !prior_step_ids.contains(dependency) {
                issues.push(issue(
                    format!("{request_path}/dependsOn"),
                    "dependencies must refer to an earlier step id",
                ));
            }
        }

        check_request_path(&request.path, &request_path_field, issues);
        check_template_value(&request.path, &extracted_names, &request_path_field, issues);
        check_headers(&request.headers, &format!("{request_path}/headers"), issues);
        check_secret_headers(
            &request.headers,
            &request.secret_headers,
            &format!("{request_path}/secretHeaders"),
            issues,
        );

        if request.extract.len() > MAX_LOAD_ITEMS {
            issues.push(issue(
                format!("{request_path}/extract"),
                format!("at most {MAX_LOAD_ITEMS} extraction rules are allowed"),
            ));
        }
        for (extraction_index, extraction) in request.extract.iter().enumerate() {
            let extraction_path = format!("{request_path}/extract/{extraction_index}");
            check_load_extraction(extraction, &extraction_path, issues);
        }

        if request.assertions.len() > MAX_LOAD_ITEMS {
            issues.push(issue(
                format!("{request_path}/assertions"),
                format!("at most {MAX_LOAD_ITEMS} assertions are allowed"),
            ));
        }
        for (assertion_index, assertion) in request.assertions.iter().enumerate() {
            check_load_assertion(
                assertion,
                &format!("{request_path}/assertions/{assertion_index}"),
                issues,
            );
        }

        if let Some(body) = &request.body_template {
            let body_path = format!("{request_path}/bodyTemplate");
            check_safe_json(body, &body_path, 0, issues);
            check_template_json(body, &extracted_names, &body_path, issues);
            if serde_json::to_vec(body).map_or(usize::MAX, |bytes| bytes.len())
                > MAX_LOAD_BODY_BYTES
            {
                issues.push(issue(
                    body_path,
                    format!("load request bodies must not exceed {MAX_LOAD_BODY_BYTES} bytes"),
                ));
            }
            if matches!(
                request.method,
                HttpMethod::Get | HttpMethod::Head | HttpMethod::Options
            ) {
                issues.push(issue(
                    format!("{request_path}/bodyTemplate"),
                    "GET, HEAD, and OPTIONS load requests must not include a body template",
                ));
            }
        }

        if request.expected_statuses.is_empty() {
            issues.push(issue(
                format!("{request_path}/expectedStatuses"),
                "each load request requires at least one expected status",
            ));
        }
        if request.expected_statuses.len() > MAX_LOAD_ITEMS {
            issues.push(issue(
                format!("{request_path}/expectedStatuses"),
                format!("at most {MAX_LOAD_ITEMS} expected statuses are allowed"),
            ));
        }
        for status in &request.expected_statuses {
            if !(100..=599).contains(status) {
                issues.push(issue(
                    format!("{request_path}/expectedStatuses"),
                    "expected statuses must be between 100 and 599",
                ));
            }
        }

        for (extraction_index, extraction) in request.extract.iter().enumerate() {
            if !extracted_names.insert(extraction.name.clone()) {
                issues.push(issue(
                    format!("{request_path}/extract/{extraction_index}/name"),
                    "extraction variable names must be unique within a load scenario",
                ));
            }
        }

        if let Some(step_id) = &request.id {
            prior_step_ids.insert(step_id.clone());
        }
    }

    validate_load_profile_fields(&scenario.profile, &path("profile"), issues);

    if let Some(reference) = &scenario.credential_ref {
        check_credential_ref(reference, &path("credentialRef"), issues);
    }
}

fn check_load_extraction(
    extraction: &LoadExtraction,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if !is_load_variable_name(&extraction.name) {
        issues.push(issue(
            format!("{path}/name"),
            "extraction names must start with a lowercase letter and contain only lowercase letters, digits, or underscores",
        ));
    }
    match extraction.source {
        LoadExtractionSource::Header => {
            if !is_valid_header_name(&extraction.selector) {
                issues.push(issue(
                    format!("{path}/selector"),
                    "header extraction selectors must be valid HTTP header names",
                ));
            }
        }
        LoadExtractionSource::JsonBody => {
            check_json_pointer(&extraction.selector, &format!("{path}/selector"), issues);
        }
    }
}

fn check_load_assertion(assertion: &LoadAssertion, path: &str, issues: &mut Vec<ValidationIssue>) {
    match assertion.source {
        LoadAssertionSource::Header => {
            if !is_valid_header_name(&assertion.selector) {
                issues.push(issue(
                    format!("{path}/selector"),
                    "header assertion selectors must be valid HTTP header names",
                ));
            }
        }
        LoadAssertionSource::JsonBody => {
            check_json_pointer(&assertion.selector, &format!("{path}/selector"), issues);
        }
    }

    match assertion.operator {
        LoadAssertionOperator::Exists => {
            if assertion.expected.is_some() {
                issues.push(issue(
                    format!("{path}/expected"),
                    "exists assertions must not include an expected value",
                ));
            }
        }
        LoadAssertionOperator::Equals | LoadAssertionOperator::Contains => {
            let Some(expected) = &assertion.expected else {
                issues.push(issue(
                    format!("{path}/expected"),
                    "equals and contains assertions require an expected value",
                ));
                return;
            };
            if matches!(assertion.operator, LoadAssertionOperator::Contains)
                && !expected.is_string()
            {
                issues.push(issue(
                    format!("{path}/expected"),
                    "contains assertions require a string expected value",
                ));
            }
            if matches!(assertion.source, LoadAssertionSource::Header) && !expected.is_string() {
                issues.push(issue(
                    format!("{path}/expected"),
                    "header assertions require a string expected value",
                ));
            }
            if !is_scalar_json_value(expected) {
                issues.push(issue(
                    format!("{path}/expected"),
                    "assertion expected values must be strings, numbers, booleans, or null",
                ));
            }
            if let Value::String(value) = expected
                && (value.chars().count() > 1_024 || value.chars().any(char::is_control))
            {
                issues.push(issue(
                    format!("{path}/expected"),
                    "assertion string values must be bounded and control-free",
                ));
            }
        }
    }
}

fn check_secret_headers(
    literal_headers: &BTreeMap<String, String>,
    secret_headers: &BTreeMap<String, String>,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if secret_headers.len() > 20 {
        issues.push(issue(
            path,
            "at most 20 secret header references are allowed",
        ));
    }
    let mut seen = BTreeSet::new();
    for (name, reference) in secret_headers {
        if !is_valid_header_name(name) {
            issues.push(issue(
                format!("{path}/{name}"),
                "header names must be valid HTTP token characters",
            ));
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            issues.push(issue(
                format!("{path}/{name}"),
                "secret header names must be unique ignoring case",
            ));
        }
        if is_forbidden_load_header_name(name) {
            issues.push(issue(
                format!("{path}/{name}"),
                "hop-by-hop and length-managed headers are not allowed",
            ));
        }
        if literal_headers
            .keys()
            .any(|literal| literal.eq_ignore_ascii_case(name))
        {
            issues.push(issue(
                format!("{path}/{name}"),
                "a header cannot be supplied as both literal and secret-backed",
            ));
        }
        check_credential_ref(reference, &format!("{path}/{name}"), issues);
    }
}

fn check_template_value(
    value: &str,
    extracted_names: &BTreeSet<String>,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut remainder = value;
    while let Some(start) = remainder.find("${") {
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find('}') else {
            issues.push(issue(
                path,
                "template placeholders must use the form ${name}",
            ));
            return;
        };
        let name = &after_start[..end];
        if !is_load_variable_name(name) {
            issues.push(issue(
                path,
                "template placeholders must use a bounded lowercase variable name",
            ));
            return;
        }
        if !extracted_names.contains(name) {
            issues.push(issue(
                path,
                "template variables must be extracted by an earlier step",
            ));
        }
        remainder = &after_start[end + 1..];
    }
}

fn check_template_json(
    value: &Value,
    extracted_names: &BTreeSet<String>,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    match value {
        Value::String(text) => check_template_value(text, extracted_names, path, issues),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                check_template_json(item, extracted_names, &format!("{path}/{index}"), issues);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                check_template_json(item, extracted_names, &format!("{path}/{key}"), issues);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn check_json_pointer(value: &str, path: &str, issues: &mut Vec<ValidationIssue>) {
    let valid = value.starts_with('/')
        && value.chars().count() <= 500
        && !value.chars().any(char::is_control)
        && !value.contains("${");
    if !valid {
        issues.push(issue(
            path,
            "JSON selectors must be bounded JSON pointers beginning with /",
        ));
    }
}

fn is_scalar_json_value(value: &Value) -> bool {
    matches!(
        value,
        Value::String(_) | Value::Bool(_) | Value::Null | Value::Number(_)
    )
}

fn is_load_variable_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    value.len() <= 64
        && first.is_ascii_lowercase()
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn is_load_step_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    value.len() <= 64
        && first.is_ascii_lowercase()
        && chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '_'
                || character == '-'
        })
}

fn validate_load_profile_fields(
    profile: &LoadProfile,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let path = |field: &str| format!("{path}/{field}");
    let has_virtual_users = profile.virtual_users.is_some();
    let has_arrival_rate = profile.arrival_rate_per_second.is_some();
    if has_virtual_users == has_arrival_rate {
        issues.push(issue(
            path("virtualUsers"),
            "provide exactly one of virtualUsers or arrivalRatePerSecond",
        ));
    }

    if let Some(virtual_users) = profile.virtual_users
        && !(1..=100).contains(&virtual_users)
    {
        issues.push(issue(
            path("virtualUsers"),
            "virtualUsers must be between 1 and 100",
        ));
    }
    if let Some(arrival_rate) = profile.arrival_rate_per_second
        && (!arrival_rate.is_finite() || !(0.1..=100.0).contains(&arrival_rate))
    {
        issues.push(issue(
            path("arrivalRatePerSecond"),
            "arrivalRatePerSecond must be between 0.1 and 100",
        ));
    }

    if profile.ramp_up_seconds > 300 {
        issues.push(issue(
            path("rampUpSeconds"),
            "rampUpSeconds must not exceed 300",
        ));
    }
    if !(1..=900).contains(&profile.duration_seconds) {
        issues.push(issue(
            path("durationSeconds"),
            "durationSeconds must be between 1 and 900",
        ));
    }
    if !(1..=100_000).contains(&profile.max_requests) {
        issues.push(issue(
            path("maxRequests"),
            "maxRequests must be between 1 and 100000",
        ));
    }
    if !profile.max_requests_per_second.is_finite()
        || !(0.1..=100.0).contains(&profile.max_requests_per_second)
    {
        issues.push(issue(
            path("maxRequestsPerSecond"),
            "maxRequestsPerSecond must be between 0.1 and 100",
        ));
    }

    validate_load_stop_conditions(&profile.stop_conditions, &path("stopConditions"), issues);
}

fn validate_load_stop_conditions(
    conditions: &LoadStopConditions,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if let Some(error_rate) = conditions.error_rate
        && (!error_rate.is_finite() || !(0.0..=1.0).contains(&error_rate))
    {
        issues.push(issue(
            format!("{path}/errorRate"),
            "errorRate must be between 0 and 1",
        ));
    }
    if let Some(latency) = conditions.p95_latency_ms
        && !(1..=60_000).contains(&latency)
    {
        issues.push(issue(
            format!("{path}/p95LatencyMs"),
            "p95LatencyMs must be between 1 and 60000",
        ));
    }
    if let Some(failed) = conditions.max_failed_requests
        && !(1..=100_000).contains(&failed)
    {
        issues.push(issue(
            format!("{path}/maxFailedRequests"),
            "maxFailedRequests must be between 1 and 100000",
        ));
    }
}

fn validate_scope(scope: &crate::model::TargetScope, issues: &mut Vec<ValidationIssue>) {
    let origin = check_origin(&scope.origin, "/scope/origin", issues);

    if scope.allowed_hosts.is_empty() || scope.allowed_hosts.len() > 20 {
        issues.push(issue(
            "/scope/allowedHosts",
            "allowedHosts must contain between 1 and 20 hosts",
        ));
    }
    for (index, host) in scope.allowed_hosts.iter().enumerate() {
        check_host(host, &format!("/scope/allowedHosts/{index}"), issues);
    }

    if scope.allowed_paths.is_empty() || scope.allowed_paths.len() > 50 {
        issues.push(issue(
            "/scope/allowedPaths",
            "allowedPaths must contain between 1 and 50 paths",
        ));
    }
    for (index, path) in scope.allowed_paths.iter().enumerate() {
        check_request_path(path, &format!("/scope/allowedPaths/{index}"), issues);
    }

    if scope.allowed_ports.is_empty() || scope.allowed_ports.len() > 10 {
        issues.push(issue(
            "/scope/allowedPorts",
            "allowedPorts must contain between 1 and 10 ports",
        ));
    }
    let mut seen_ports = BTreeSet::new();
    for port in &scope.allowed_ports {
        if !seen_ports.insert(*port) {
            issues.push(issue("/scope/allowedPorts", "ports must be unique"));
        }
    }

    if let Some(origin) = origin {
        if let Some(host) = origin.host_str() {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            if !scope
                .allowed_hosts
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(host))
            {
                issues.push(issue(
                    "/scope/allowedHosts",
                    "allowedHosts must include the origin host",
                ));
            }
        }
        if let Some(port) = origin.port_or_known_default()
            && !scope.allowed_ports.contains(&port)
        {
            issues.push(issue(
                "/scope/allowedPorts",
                "allowedPorts must include the origin port",
            ));
        }
    }
}

fn validate_target_limits_fields(
    limits: &crate::model::TargetLimits,
    issues: &mut Vec<ValidationIssue>,
) {
    if !limits.max_requests_per_second.is_finite()
        || !(0.1..=1_000.0).contains(&limits.max_requests_per_second)
    {
        issues.push(issue(
            "/limits/maxRequestsPerSecond",
            "maxRequestsPerSecond must be between 0.1 and 1000",
        ));
    }
    if !(1..=100).contains(&limits.max_concurrency) {
        issues.push(issue(
            "/limits/maxConcurrency",
            "maxConcurrency must be between 1 and 100",
        ));
    }
    if !(1..=3_600).contains(&limits.max_duration_seconds) {
        issues.push(issue(
            "/limits/maxDurationSeconds",
            "maxDurationSeconds must be between 1 and 3600",
        ));
    }
    if !(1..=1_000_000).contains(&limits.max_requests) {
        issues.push(issue(
            "/limits/maxRequests",
            "maxRequests must be between 1 and 1000000",
        ));
    }
}

fn check_schema_version(version: &str, issues: &mut Vec<ValidationIssue>) {
    if version != CONTRACT_SCHEMA_VERSION {
        issues.push(issue(
            "/schemaVersion",
            format!("schemaVersion must be \"{CONTRACT_SCHEMA_VERSION}\""),
        ));
    }
}

fn check_text(value: &str, path: &str, max_chars: usize, issues: &mut Vec<ValidationIssue>) {
    if value.trim().is_empty() {
        issues.push(issue(path, "must not be blank"));
    }
    if value.chars().count() > max_chars {
        issues.push(issue(
            path,
            format!("must be at most {max_chars} characters"),
        ));
    }
    if value.chars().any(char::is_control) {
        issues.push(issue(path, "must not contain control characters"));
    }
}

fn check_string_list(
    values: &[String],
    path: &str,
    max_items: usize,
    max_chars: usize,
    issues: &mut Vec<ValidationIssue>,
) {
    if values.len() > max_items {
        issues.push(issue(
            path,
            format!("must contain at most {max_items} items"),
        ));
    }
    for (index, value) in values.iter().enumerate() {
        check_text(value, &format!("{path}/{index}"), max_chars, issues);
    }
}

fn check_repository_url(value: &str, issues: &mut Vec<ValidationIssue>) {
    if value.chars().count() > 2_048 {
        issues.push(issue(
            "/git/repositoryUrl",
            "repositoryUrl must be at most 2048 characters",
        ));
        return;
    }

    let Ok(url) = Url::parse(value) else {
        issues.push(issue("/git/repositoryUrl", "must be a valid URL"));
        return;
    };

    if !matches!(url.scheme(), "https" | "ssh") {
        issues.push(issue("/git/repositoryUrl", "must use https or ssh"));
    }
    if url.password().is_some() {
        issues.push(issue(
            "/git/repositoryUrl",
            "must not embed a password; use credentialRef",
        ));
    }
    if url.scheme() == "https" && !url.username().is_empty() {
        issues.push(issue(
            "/git/repositoryUrl",
            "https repository URLs must not include a username",
        ));
    }
    if url.scheme() == "ssh" && !url.username().is_empty() && url.username() != "git" {
        issues.push(issue(
            "/git/repositoryUrl",
            "ssh repository URLs may only use the git username",
        ));
    }
    if url.host_str().is_none() {
        issues.push(issue("/git/repositoryUrl", "must include a host"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        issues.push(issue(
            "/git/repositoryUrl",
            "must not include a query or fragment",
        ));
    }
}

fn check_repository_reference(value: &str, issues: &mut Vec<ValidationIssue>) {
    check_text(value, "/git/reference", 200, issues);
    let invalid = value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("..")
        || value.contains("//")
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    if invalid {
        issues.push(issue(
            "/git/reference",
            "must be a safe branch, tag, or commit reference",
        ));
    }
}

fn check_credential_ref(value: &str, path: &str, issues: &mut Vec<ValidationIssue>) {
    let Some(reference) = value.strip_prefix("secret://") else {
        issues.push(issue(path, "must be a secret:// reference"));
        return;
    };

    let valid = !reference.is_empty()
        && reference.len() <= 190
        && reference
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && reference.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '/' | '_' | '.' | '-')
        });
    if !valid {
        issues.push(issue(path, "has an invalid secret reference format"));
    }
}

fn check_catalog_test_id(value: &str, path: &str, issues: &mut Vec<ValidationIssue>) {
    let length = value.chars().count();
    let valid = (2..=119).contains(&length)
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-'));
    if !valid {
        issues.push(issue(path, "has an invalid catalog test id"));
    }
}

fn check_origin(value: &str, path: &str, issues: &mut Vec<ValidationIssue>) -> Option<Url> {
    let Ok(url) = Url::parse(value) else {
        issues.push(issue(path, "must be a valid URL origin"));
        return None;
    };

    if !matches!(url.scheme(), "http" | "https") {
        issues.push(issue(path, "must use http or https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        issues.push(issue(path, "must not include credentials"));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        issues.push(issue(
            path,
            "must be an origin without a path, query, or fragment",
        ));
    }
    if url.host_str().is_none() {
        issues.push(issue(path, "must include a host"));
        return None;
    }
    Some(url)
}

fn check_host(value: &str, path: &str, issues: &mut Vec<ValidationIssue>) {
    let valid = !value.is_empty()
        && value.len() <= 253
        && !value.starts_with('.')
        && !value.ends_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if !valid {
        issues.push(issue(path, "has an invalid host format"));
    }
}

fn check_request_path(value: &str, path: &str, issues: &mut Vec<ValidationIssue>) {
    let valid = value.starts_with('/')
        && value.chars().count() <= 2_000
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains('\\')
        && !value.chars().any(char::is_control);
    if !valid {
        issues.push(issue(
            path,
            "must be an absolute path without traversal or separator escapes",
        ));
    }
}

fn check_headers(
    headers: &std::collections::BTreeMap<String, String>,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if headers.len() > 20 {
        issues.push(issue(path, "at most 20 headers are allowed"));
    }
    for (name, value) in headers {
        if !is_valid_header_name(name) {
            issues.push(issue(
                format!("{path}/{name}"),
                "header names must be valid HTTP token characters",
            ));
        }
        if name.eq_ignore_ascii_case("authorization")
            || name.eq_ignore_ascii_case("cookie")
            || name.eq_ignore_ascii_case("proxy-authorization")
            || name.eq_ignore_ascii_case("set-cookie")
        {
            issues.push(issue(
                format!("{path}/{name}"),
                "credential-bearing headers must use credentialRef instead",
            ));
        }
        if is_forbidden_load_header_name(name) {
            issues.push(issue(
                format!("{path}/{name}"),
                "hop-by-hop and length-managed headers are not allowed",
            ));
        }
        if value.chars().count() > 1_024 {
            issues.push(issue(
                format!("{path}/{name}"),
                "header values must be at most 1024 characters",
            ));
        }
        if value.chars().any(char::is_control) {
            issues.push(issue(
                format!("{path}/{name}"),
                "header values must not contain control characters",
            ));
        }
    }
}

fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn is_forbidden_load_header_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "transfer-encoding" | "connection" | "keep-alive"
    )
}

fn check_safe_json(value: &Value, path: &str, depth: usize, issues: &mut Vec<ValidationIssue>) {
    const MAX_DEPTH: usize = 20;
    if depth > MAX_DEPTH {
        issues.push(issue(path, "JSON nesting is too deep"));
        return;
    }

    match value {
        Value::String(text) => {
            if text.chars().count() > 16_384 {
                issues.push(issue(path, "strings must be at most 16384 characters"));
            }
        }
        Value::Array(items) => {
            if items.len() > 100 {
                issues.push(issue(path, "arrays may contain at most 100 items"));
            }
            for (index, item) in items.iter().enumerate() {
                check_safe_json(item, &format!("{path}/{index}"), depth + 1, issues);
            }
        }
        Value::Object(map) => {
            if map.len() > 50 {
                issues.push(issue(path, "objects may contain at most 50 properties"));
            }
            for (key, item) in map {
                if is_forbidden_key(key) {
                    issues.push(issue(
                        path,
                        "executable command or script fields are not allowed",
                    ));
                }
                check_safe_json(item, &format!("{path}/{key}"), depth + 1, issues);
            }
        }
        Value::Number(number) => {
            if number.as_f64().is_none() {
                issues.push(issue(path, "numbers must be finite and representable"));
            }
        }
        Value::Null | Value::Bool(_) => {}
    }
}

fn is_forbidden_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "command" | "cmd" | "script" | "scripts" | "executable" | "shell" | "entrypoint"
    )
}

fn check_non_nil_uuid(value: &uuid::Uuid, path: &str, issues: &mut Vec<ValidationIssue>) {
    if value.is_nil() {
        issues.push(issue(path, "must not be a nil UUID"));
    }
}

fn check_unique_uuids(values: &[uuid::Uuid], path: &str, issues: &mut Vec<ValidationIssue>) {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.is_nil() {
            issues.push(issue(path, "must not contain a nil UUID"));
        }
        if !seen.insert(*value) {
            issues.push(issue(path, "must not contain duplicate ids"));
        }
    }
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> ValidationIssue {
    ValidationIssue {
        path: path.into(),
        message: message.into(),
    }
}

fn finish(issues: Vec<ValidationIssue>) -> Result<(), Vec<ValidationIssue>> {
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn load_stop_conditions_must_be_an_object() {
        let value = json!({
            "virtualUsers": 10,
            "rampUpSeconds": 30,
            "durationSeconds": 120,
            "maxRequests": 5000,
            "maxRequestsPerSecond": 50,
            "stopConditions": []
        });
        assert!(validate_load_profile(&value).is_err());
    }

    fn valid_load_scenario() -> Value {
        json!({
            "name": "Bounded fixture flow",
            "description": "Bounded local fixture; no traffic is sent.",
            "requests": [{
                "id": "first",
                "method": "GET",
                "path": "/api/items",
                "expectedStatuses": [200]
            }],
            "profile": {
                "virtualUsers": 2,
                "rampUpSeconds": 5,
                "durationSeconds": 30,
                "maxRequests": 200,
                "maxRequestsPerSecond": 2
            }
        })
    }

    #[test]
    fn load_scenario_enforces_platform_maxima_and_rejects_loops() {
        assert!(validate_load_scenario(&valid_load_scenario()).is_ok());

        let mut excessive = valid_load_scenario();
        excessive["profile"]["virtualUsers"] = json!(101);
        assert!(validate_load_scenario(&excessive).is_err());

        let mut loop_attempt = valid_load_scenario();
        loop_attempt["requests"][0]["loop"] = json!({"while": true});
        assert!(validate_load_scenario(&loop_attempt).is_err());
    }

    #[test]
    fn load_scenario_rejects_oversized_bodies_and_script_templates() {
        let mut oversized = valid_load_scenario();
        oversized["requests"][0]["method"] = json!("POST");
        oversized["requests"][0]["bodyTemplate"] = json!({"payload": "x".repeat(70 * 1024)});
        assert!(validate_load_scenario(&oversized).is_err());

        let mut expression = valid_load_scenario();
        expression["requests"][0]["path"] = json!("/api/${process.env.SECRET}");
        assert!(validate_load_scenario(&expression).is_err());
    }

    #[test]
    fn load_scenario_rejects_same_step_correlation_references() {
        let mut scenario = valid_load_scenario();
        scenario["requests"][0]["method"] = json!("POST");
        scenario["requests"][0]["extract"] = json!([
            {"name": "token", "source": "header", "selector": "x-session-token"}
        ]);
        scenario["requests"][0]["bodyTemplate"] = json!({"token": "${token}"});
        assert!(validate_load_scenario(&scenario).is_err());
    }

    #[test]
    fn load_scenario_requires_earlier_dependencies_and_secret_references() {
        let mut dependency = valid_load_scenario();
        dependency["requests"][0]["dependsOn"] = json!(["later"]);
        assert!(validate_load_scenario(&dependency).is_err());

        let mut raw_secret = valid_load_scenario();
        raw_secret["requests"][0]["secretHeaders"] =
            json!({"Authorization": "Bearer raw-token-value"});
        assert!(validate_load_scenario(&raw_secret).is_err());
    }
}
