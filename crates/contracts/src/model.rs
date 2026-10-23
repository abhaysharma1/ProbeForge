//! Wire-level contract types. Field names are camelCase to match the shared
//! JSON Schema bundle in `packages/contracts/schemas/probeforge.v1.schema.json`.
//!
//! These types are deliberately data-only. They must never grow fields that let
//! a model supply shell commands, executable paths, or arbitrary test scripts.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Version of the shared API/agent contract bundle.
pub const CONTRACT_SCHEMA_VERSION: &str = "1";

/// Hard ceiling for a single serialized contract payload.
pub const MAX_CONTRACT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobGoal {
    VulnerabilityTesting,
    LoadTesting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intensity {
    Low,
    Standard,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Local,
    Staging,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "HEAD")]
    Head,
    #[serde(rename = "POST")]
    Post,
    #[serde(rename = "PUT")]
    Put,
    #[serde(rename = "PATCH")]
    Patch,
    #[serde(rename = "DELETE")]
    Delete,
    #[serde(rename = "OPTIONS")]
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationMethod {
    ExplicitConfirmation,
    OwnershipVerification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    Passed,
    Failed,
    Skipped,
    Inconclusive,
    Error,
    Cancelled,
}

/// Event names are operator-visible and form part of the SSE replay contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    #[serde(rename = "job.queued")]
    JobQueued,
    #[serde(rename = "job.claimed")]
    JobClaimed,
    #[serde(rename = "job.retry_scheduled")]
    JobRetryScheduled,
    #[serde(rename = "job.failed")]
    JobFailed,
    #[serde(rename = "job.cancelled")]
    JobCancelled,
    #[serde(rename = "repository.clone.started")]
    RepositoryCloneStarted,
    #[serde(rename = "repository.snapshot.ready")]
    RepositorySnapshotReady,
    #[serde(rename = "repository.clone.failed")]
    RepositoryCloneFailed,
    #[serde(rename = "agent.discovery.progress")]
    AgentDiscoveryProgress,
    #[serde(rename = "plan.proposed")]
    PlanProposed,
    #[serde(rename = "plan.approved")]
    PlanApproved,
    #[serde(rename = "target.validated")]
    TargetValidated,
    #[serde(rename = "run.scheduled")]
    RunScheduled,
    #[serde(rename = "run.started")]
    RunStarted,
    #[serde(rename = "run.cancelled")]
    RunCancelled,
    #[serde(rename = "run.completed")]
    RunCompleted,
    #[serde(rename = "run.failed")]
    RunFailed,
    #[serde(rename = "test.started")]
    TestStarted,
    #[serde(rename = "test.progress")]
    TestProgress,
    #[serde(rename = "test.completed")]
    TestCompleted,
    #[serde(rename = "test.failed")]
    TestFailed,
    #[serde(rename = "finding.recorded")]
    FindingRecorded,
    #[serde(rename = "report.completed")]
    ReportCompleted,
    #[serde(rename = "report.failed")]
    ReportFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GitSource {
    pub repository_url: String,
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct JobCreation {
    pub schema_version: String,
    pub goal: JobGoal,
    pub intensity: Intensity,
    pub display_name: String,
    pub git: GitSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LoadStopConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_latency_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_failed_requests: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LoadProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_users: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_rate_per_second: Option<f64>,
    pub ramp_up_seconds: u32,
    pub duration_seconds: u32,
    pub max_requests: u64,
    pub max_requests_per_second: f64,
    #[serde(default, deserialize_with = "deserialize_load_stop_conditions")]
    pub stop_conditions: LoadStopConditions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadExtractionSource {
    Header,
    JsonBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LoadExtraction {
    pub name: String,
    pub source: LoadExtractionSource,
    pub selector: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadAssertionSource {
    Header,
    JsonBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadAssertionOperator {
    Equals,
    Contains,
    Exists,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LoadAssertion {
    pub source: LoadAssertionSource,
    pub selector: String,
    pub operator: LoadAssertionOperator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LoadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub method: HttpMethod,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub extract: Vec<LoadExtraction>,
    #[serde(default)]
    pub assertions: Vec<LoadAssertion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_template: Option<Value>,
    pub expected_statuses: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LoadScenario {
    pub name: String,
    pub description: String,
    pub requests: Vec<LoadRequest>,
    pub profile: LoadProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PlannedTest {
    pub catalog_test_id: String,
    pub name: String,
    pub rationale: String,
    pub risk: Risk,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    pub expected_evidence: String,
    #[serde(default)]
    pub target_requirements: Vec<String>,
    pub requires_explicit_confirmation: bool,
    #[serde(default = "empty_object")]
    pub configuration: Value,
    #[serde(default)]
    pub rationale_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Plan {
    pub schema_version: String,
    pub goal: JobGoal,
    pub summary: String,
    pub confidence: f64,
    pub planned_tests: Vec<PlannedTest>,
    pub load_scenarios: Vec<LoadScenario>,
    pub required_inputs: Vec<String>,
    pub limitations: Vec<String>,
    #[serde(default)]
    pub evidence_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TargetScope {
    pub origin: String,
    pub allowed_hosts: Vec<String>,
    pub allowed_paths: Vec<String>,
    pub allowed_ports: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TargetLimits {
    pub max_requests_per_second: f64,
    pub max_concurrency: u32,
    pub max_duration_seconds: u32,
    pub max_requests: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TargetAuthorization {
    pub schema_version: String,
    pub target_id: Uuid,
    pub authorization_method: AuthorizationMethod,
    pub confirmed: bool,
    pub scope: TargetScope,
    pub limits: TargetLimits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RunRequest {
    pub schema_version: String,
    pub run_type: JobGoal,
    pub job_id: Uuid,
    pub plan_id: Uuid,
    pub plan_version: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_test_ids: Option<Vec<Uuid>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_scenario_ids: Option<Vec<Uuid>>,
    pub target_authorization_id: Uuid,
    pub idempotency_key: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TestResult {
    pub schema_version: String,
    pub run_id: Uuid,
    pub catalog_test_id: String,
    pub outcome: TestOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub summary: String,
    #[serde(default = "empty_object")]
    pub details: Value,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct JobEvent {
    pub schema_version: String,
    pub job_id: Uuid,
    pub sequence: i64,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub occurred_at: DateTime<Utc>,
    #[serde(default = "empty_object")]
    pub payload: Value,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn deserialize_load_stop_conditions<'de, D>(deserializer: D) -> Result<LoadStopConditions, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if !value.is_object() {
        return Err(de::Error::custom("stopConditions must be a JSON object"));
    }
    serde_json::from_value(value).map_err(de::Error::custom)
}

fn default_true() -> bool {
    true
}
