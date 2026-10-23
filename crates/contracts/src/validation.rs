//! Deterministic validation for contract payloads.
//!
//! Validation happens before any persistence or execution. Errors intentionally
//! contain paths and rule descriptions, never the offending values, so a target
//! URL or credential reference cannot leak through an error response.

use std::collections::BTreeSet;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;

use crate::model::{
    CONTRACT_SCHEMA_VERSION, JobCreation, JobEvent, LoadProfile, LoadScenario, MAX_CONTRACT_BYTES,
    Plan, PlannedTest, RunRequest, TargetAuthorization, TargetLimits, TargetScope, TestOutcome,
    TestResult,
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
    let path = |field: &str| format!("{path}/{field}");
    check_text(&scenario.name, &path("name"), 160, issues);
    check_text(&scenario.description, &path("description"), 2_000, issues);

    if scenario.requests.is_empty() {
        issues.push(issue(
            path("requests"),
            "a load scenario requires at least one request",
        ));
    }
    if scenario.requests.len() > 50 {
        issues.push(issue(path("requests"), "at most 50 requests are allowed"));
    }

    for (index, request) in scenario.requests.iter().enumerate() {
        let request_path = path(&format!("requests/{index}"));
        check_request_path(&request.path, &format!("{request_path}/path"), issues);
        check_headers(&request.headers, &format!("{request_path}/headers"), issues);
        if let Some(body) = &request.body_template {
            check_safe_json(body, &format!("{request_path}/bodyTemplate"), 0, issues);
        }
        if request.expected_statuses.len() > 20 {
            issues.push(issue(
                format!("{request_path}/expectedStatuses"),
                "at most 20 expected statuses are allowed",
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
    }

    validate_load_profile_fields(&scenario.profile, &path("profile"), issues);

    if let Some(reference) = &scenario.credential_ref {
        check_credential_ref(reference, &path("credentialRef"), issues);
    }
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
        && !(1..=500).contains(&virtual_users)
    {
        issues.push(issue(
            path("virtualUsers"),
            "virtualUsers must be between 1 and 500",
        ));
    }
    if let Some(arrival_rate) = profile.arrival_rate_per_second
        && (!arrival_rate.is_finite() || !(0.1..=1_000.0).contains(&arrival_rate))
    {
        issues.push(issue(
            path("arrivalRatePerSecond"),
            "arrivalRatePerSecond must be between 0.1 and 1000",
        ));
    }

    if profile.ramp_up_seconds > 3_600 {
        issues.push(issue(
            path("rampUpSeconds"),
            "rampUpSeconds must not exceed 3600",
        ));
    }
    if !(1..=3_600).contains(&profile.duration_seconds) {
        issues.push(issue(
            path("durationSeconds"),
            "durationSeconds must be between 1 and 3600",
        ));
    }
    if !(1..=1_000_000).contains(&profile.max_requests) {
        issues.push(issue(
            path("maxRequests"),
            "maxRequests must be between 1 and 1000000",
        ));
    }
    if !profile.max_requests_per_second.is_finite()
        || !(0.1..=1_000.0).contains(&profile.max_requests_per_second)
    {
        issues.push(issue(
            path("maxRequestsPerSecond"),
            "maxRequestsPerSecond must be between 0.1 and 1000",
        ));
    }

    if !profile.stop_conditions.is_object() {
        issues.push(issue(
            path("stopConditions"),
            "stopConditions must be a JSON object",
        ));
    }
    check_safe_json(&profile.stop_conditions, &path("stopConditions"), 0, issues);
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
        && !value.chars().any(char::is_control);
    if !valid {
        issues.push(issue(
            path,
            "must be an absolute path without traversal segments",
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
