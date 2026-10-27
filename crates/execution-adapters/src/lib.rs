//! Small, vetted security adapters with typed configuration and normalized
//! results. No adapter builds commands, invokes a shell, or accepts executable
//! text from a model, plan, or target.
//!
//! Adapters fall into two families:
//!
//! - **Network observations** use the shared guarded HTTP client or a bounded
//!   TLS handshake against the authorized target scope.
//! - **Static observations** read a server-owned, pinned repository snapshot
//!   that the execution worker materializes. A plan can never supply a path.
//!
//! Unsupported catalog IDs fail closed.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    time::{Duration, Instant},
};

use probe_forge_target_policy::{
    GuardedHttpClient, RunBudget, TargetGuard, TargetPolicy, TargetRequestError, TargetResolver,
    is_address_allowed, validate_request_url, validate_target_authority,
};
use probeforge_contracts::Severity;
use reqwest::header::{HeaderMap, HeaderValue, ORIGIN};
use rustls::{
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

const PROBE_ORIGIN: &str = "https://probe.invalid";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_ADAPTER_NAME: &str = "http_probe";
const HTTP_ADAPTER_VERSION: &str = "0.1.0";
const TLS_ADAPTER_NAME: &str = "tls_probe";
const TLS_ADAPTER_VERSION: &str = "0.1.0";
const STATIC_ADAPTER_NAME: &str = "static_secret_scanner";
const STATIC_ADAPTER_VERSION: &str = "0.1.0";
const TLS_TIMEOUT: Duration = Duration::from_secs(30);
const STATIC_TIMEOUT: Duration = Duration::from_secs(60);
const RATE_LIMIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Bounded request budget for the authentication rate-limit observation.
const RATE_LIMIT_ATTEMPTS: u32 = 5;
const RATE_LIMIT_INTERVAL: Duration = Duration::from_millis(250);
/// Cookies from one response are bounded so a hostile target cannot force
/// unbounded result growth.
const MAX_COOKIES_INSPECTED: usize = 32;

const REQUIRED_HTTP_HEADERS: &[&str] = &[
    "content-security-policy",
    "x-content-type-options",
    "referrer-policy",
];
const REQUIRED_HTTPS_HEADERS: &[&str] = &[
    "content-security-policy",
    "x-content-type-options",
    "referrer-policy",
    "strict-transport-security",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterKind {
    HttpHeaders,
    HttpCors,
    HttpRateLimit,
    HttpCookies,
    HttpInfoDisclosure,
    TlsBaseline,
    StaticSecrets,
}

/// Configuration contract a catalog test exposes to the planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationContract {
    /// Exactly one safe absolute `route` or `path`.
    Route,
    /// No configuration values are accepted.
    Empty,
}

#[derive(Debug, Clone, Copy)]
pub struct AdapterSpec {
    pub catalog_test_id: &'static str,
    pub adapter_name: &'static str,
    pub adapter_version: &'static str,
    pub kind: AdapterKind,
    pub timeout: Duration,
    /// True when the adapter only reads state and never changes target state.
    pub passive: bool,
    /// True when execution needs a server-owned pinned repository snapshot.
    pub requires_repository_snapshot: bool,
    pub configuration: ConfigurationContract,
}

pub const ADAPTER_SPECS: &[AdapterSpec] = &[
    AdapterSpec {
        catalog_test_id: "http.headers.missing",
        adapter_name: HTTP_ADAPTER_NAME,
        adapter_version: HTTP_ADAPTER_VERSION,
        kind: AdapterKind::HttpHeaders,
        timeout: HTTP_TIMEOUT,
        passive: true,
        requires_repository_snapshot: false,
        configuration: ConfigurationContract::Route,
    },
    AdapterSpec {
        catalog_test_id: "http.cors.misconfiguration",
        adapter_name: HTTP_ADAPTER_NAME,
        adapter_version: HTTP_ADAPTER_VERSION,
        kind: AdapterKind::HttpCors,
        timeout: HTTP_TIMEOUT,
        passive: true,
        requires_repository_snapshot: false,
        configuration: ConfigurationContract::Route,
    },
    AdapterSpec {
        catalog_test_id: "http.auth.rate-limit",
        adapter_name: HTTP_ADAPTER_NAME,
        adapter_version: HTTP_ADAPTER_VERSION,
        kind: AdapterKind::HttpRateLimit,
        timeout: RATE_LIMIT_TIMEOUT,
        passive: false,
        requires_repository_snapshot: false,
        configuration: ConfigurationContract::Route,
    },
    AdapterSpec {
        catalog_test_id: "http.cookies.insecure",
        adapter_name: HTTP_ADAPTER_NAME,
        adapter_version: HTTP_ADAPTER_VERSION,
        kind: AdapterKind::HttpCookies,
        timeout: HTTP_TIMEOUT,
        passive: true,
        requires_repository_snapshot: false,
        configuration: ConfigurationContract::Route,
    },
    AdapterSpec {
        catalog_test_id: "http.info-disclosure",
        adapter_name: HTTP_ADAPTER_NAME,
        adapter_version: HTTP_ADAPTER_VERSION,
        kind: AdapterKind::HttpInfoDisclosure,
        timeout: HTTP_TIMEOUT,
        passive: true,
        requires_repository_snapshot: false,
        configuration: ConfigurationContract::Route,
    },
    AdapterSpec {
        catalog_test_id: "network.tls.baseline",
        adapter_name: TLS_ADAPTER_NAME,
        adapter_version: TLS_ADAPTER_VERSION,
        kind: AdapterKind::TlsBaseline,
        timeout: TLS_TIMEOUT,
        passive: true,
        requires_repository_snapshot: false,
        configuration: ConfigurationContract::Empty,
    },
    AdapterSpec {
        catalog_test_id: "static.secrets.exposure",
        adapter_name: STATIC_ADAPTER_NAME,
        adapter_version: STATIC_ADAPTER_VERSION,
        kind: AdapterKind::StaticSecrets,
        timeout: STATIC_TIMEOUT,
        passive: true,
        requires_repository_snapshot: true,
        configuration: ConfigurationContract::Empty,
    },
];

/// Stable, serializable registry metadata. The API and UI read this instead of
/// inferring capabilities from tool-requirement strings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterMetadata {
    pub catalog_test_id: &'static str,
    pub adapter_name: &'static str,
    pub adapter_version: &'static str,
    pub category: &'static str,
    pub mode: &'static str,
    pub passive: bool,
    pub requires_repository_snapshot: bool,
    pub requires_target_scope: bool,
    pub requires_test_accounts: bool,
    pub configuration: &'static str,
    pub safety_level: &'static str,
    pub dependencies: Vec<&'static str>,
}

impl AdapterSpec {
    pub fn category(&self) -> &'static str {
        match self.kind {
            AdapterKind::HttpHeaders
            | AdapterKind::HttpCors
            | AdapterKind::HttpRateLimit
            | AdapterKind::HttpCookies
            | AdapterKind::HttpInfoDisclosure => "http",
            AdapterKind::TlsBaseline => "network",
            AdapterKind::StaticSecrets => "static",
        }
    }

    pub fn metadata(&self) -> AdapterMetadata {
        let dependencies = match self.kind {
            AdapterKind::StaticSecrets => vec!["git"],
            AdapterKind::TlsBaseline => vec!["native_tls_roots"],
            _ => Vec::new(),
        };
        AdapterMetadata {
            catalog_test_id: self.catalog_test_id,
            adapter_name: self.adapter_name,
            adapter_version: self.adapter_version,
            category: self.category(),
            mode: "vulnerability_testing",
            passive: self.passive,
            requires_repository_snapshot: self.requires_repository_snapshot,
            requires_target_scope: !self.requires_repository_snapshot,
            requires_test_accounts: false,
            configuration: match self.configuration {
                ConfigurationContract::Route => "route",
                ConfigurationContract::Empty => "none",
            },
            safety_level: if self.passive { "passive" } else { "bounded" },
            dependencies,
        }
    }
}

pub fn registry_metadata() -> Vec<AdapterMetadata> {
    ADAPTER_SPECS.iter().map(AdapterSpec::metadata).collect()
}

pub fn adapters() -> &'static [AdapterSpec] {
    ADAPTER_SPECS
}

pub fn find_adapter(catalog_test_id: &str) -> Option<&'static AdapterSpec> {
    ADAPTER_SPECS
        .iter()
        .find(|spec| spec.catalog_test_id == catalog_test_id)
}

pub fn supported_catalog_ids() -> impl Iterator<Item = &'static str> {
    ADAPTER_SPECS.iter().map(|spec| spec.catalog_test_id)
}

pub fn is_supported(catalog_test_id: &str) -> bool {
    find_adapter(catalog_test_id).is_some()
}

/// Whether execution needs a pinned repository snapshot materialized by the
/// server. The answer is registry-owned, not model-owned.
pub fn requires_repository_snapshot(catalog_test_id: &str) -> bool {
    find_adapter(catalog_test_id).is_some_and(|spec| spec.requires_repository_snapshot)
}

pub fn metadata_for(catalog_test_id: &str) -> Option<AdapterMetadata> {
    find_adapter(catalog_test_id).map(AdapterSpec::metadata)
}

/// Validate a planned test's configuration against the adapter that will run it.
///
/// Run creation calls this before queueing work, so an unusable configuration is
/// rejected while a human can still regenerate the plan instead of surfacing as
/// a completed run whose test errored. Catalog entries without an adapter are
/// skipped at execution time and have no configuration contract to enforce.
pub fn validate_configuration(
    catalog_test_id: &str,
    configuration: &Value,
) -> Result<(), &'static str> {
    let Some(spec) = find_adapter(catalog_test_id) else {
        return Ok(());
    };
    match spec.configuration {
        ConfigurationContract::Route => route_from_configuration(configuration).map(|_| ()),
        ConfigurationContract::Empty => empty_configuration(configuration),
    }
}

fn empty_configuration(configuration: &Value) -> Result<(), &'static str> {
    match configuration {
        Value::Null => Ok(()),
        Value::Object(values) if values.is_empty() => Ok(()),
        _ => Err("This test accepts no configuration values."),
    }
}

/// Server-owned repository snapshot root. Constructing one validates that the
/// path is an absolute directory; adapters only read beneath it.
#[derive(Debug, Clone)]
pub struct RepositorySnapshot {
    root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotError {
    InvalidRoot,
    NotADirectory,
}

impl RepositorySnapshot {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, SnapshotError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(SnapshotError::InvalidRoot);
        }
        let metadata = std::fs::metadata(&root).map_err(|_| SnapshotError::NotADirectory)?;
        if !metadata.is_dir() {
            return Err(SnapshotError::NotADirectory);
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Completed,
    Failed,
    TimedOut,
    Cancelled,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityOutcome {
    Passed,
    Finding,
    Inconclusive,
    NotRun,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterEvidence {
    pub kind: String,
    pub summary: String,
    pub attributes: Value,
}

/// Optional normalized fields that the existing storage layer keeps inside the
/// result `details` payload. They are never required for a finding.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterExtensions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cwe_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cve_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_assets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterResult {
    pub catalog_test_id: String,
    pub adapter_name: String,
    pub adapter_version: String,
    pub execution_status: ExecutionStatus,
    pub security_outcome: SecurityOutcome,
    pub severity: Option<Severity>,
    pub confidence: f64,
    pub summary: String,
    pub duration_ms: u64,
    pub evidence: Vec<AdapterEvidence>,
    pub error_code: Option<String>,
    #[serde(flatten)]
    pub extensions: AdapterExtensions,
}

#[derive(Clone)]
pub struct AdapterEnvironment {
    pub guard: TargetGuard,
    pub policy: TargetPolicy,
    pub resolver: Arc<dyn TargetResolver>,
    /// Present only when the worker materialized a server-owned pinned snapshot
    /// for a static adapter.
    pub snapshot: Option<RepositorySnapshot>,
}

pub async fn execute(
    catalog_test_id: &str,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
) -> AdapterResult {
    let Some(spec) = find_adapter(catalog_test_id) else {
        return unsupported_result(catalog_test_id);
    };
    execute_spec(spec, environment, configuration, budget, spec.timeout).await
}

pub async fn execute_with_timeout(
    catalog_test_id: &str,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
    timeout: Duration,
) -> AdapterResult {
    let Some(spec) = find_adapter(catalog_test_id) else {
        return unsupported_result(catalog_test_id);
    };
    execute_spec(spec, environment, configuration, budget, timeout).await
}

async fn execute_spec(
    spec: &'static AdapterSpec,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
    timeout: Duration,
) -> AdapterResult {
    let started = Instant::now();
    tracing::debug!(
        event = "adapter.started",
        catalog_test_id = spec.catalog_test_id,
        adapter = spec.adapter_name,
        adapter_version = spec.adapter_version,
        "adapter execution started"
    );
    let mut result = match tokio::time::timeout(
        timeout,
        run_adapter(spec, environment, configuration, budget),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => failure_result(
            spec,
            ExecutionStatus::TimedOut,
            "adapter_timeout",
            "The adapter exceeded its execution timeout.",
        ),
    };
    result.duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    tracing::info!(
        event = "adapter.executed",
        catalog_test_id = %result.catalog_test_id,
        adapter = %result.adapter_name,
        adapter_version = %result.adapter_version,
        execution_status = ?result.execution_status,
        security_outcome = ?result.security_outcome,
        duration_ms = result.duration_ms,
        error_code = ?result.error_code,
        "adapter execution finished"
    );
    result
}

async fn run_adapter(
    spec: &'static AdapterSpec,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
) -> AdapterResult {
    match spec.kind {
        AdapterKind::TlsBaseline => {
            run_tls_baseline(spec, environment, configuration, budget).await
        }
        AdapterKind::StaticSecrets => {
            run_static_secrets(spec, environment, configuration, budget).await
        }
        AdapterKind::HttpHeaders
        | AdapterKind::HttpCors
        | AdapterKind::HttpRateLimit
        | AdapterKind::HttpCookies
        | AdapterKind::HttpInfoDisclosure => {
            run_http_observation(spec, environment, configuration, budget).await
        }
    }
}

async fn run_http_observation(
    spec: &'static AdapterSpec,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
) -> AdapterResult {
    let path = match route_from_configuration(configuration) {
        Ok(path) => path,
        Err(summary) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "invalid_adapter_configuration",
                summary,
            );
        }
    };
    let origin = match Url::parse(&environment.guard.scope.origin) {
        Ok(origin) => origin,
        Err(_) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "invalid_target_scope",
                "The authorized target origin is invalid.",
            );
        }
    };
    let url = match origin.join(&path) {
        Ok(url) => url,
        Err(_) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "invalid_adapter_configuration",
                "The test route is not a valid target path.",
            );
        }
    };
    if let Err(error) = validate_request_url(&environment.guard, &url, &environment.policy) {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            error.code(),
            "The test route is outside the authorized target scope.",
        );
    }

    let client = GuardedHttpClient::new(environment.policy, environment.resolver);
    if spec.kind == AdapterKind::HttpRateLimit {
        return run_rate_limit(spec, &client, &environment.guard, &url, budget).await;
    }
    let response = if spec.kind == AdapterKind::HttpCors {
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static(PROBE_ORIGIN));
        client
            .get_with_headers(&environment.guard, &url, &headers, budget)
            .await
    } else {
        client.get(&environment.guard, &url, budget).await
    };
    let response = match response {
        Ok(response) => response,
        Err(error) => return request_failure(spec, error),
    };

    if response.status >= 500 && spec.kind != AdapterKind::HttpInfoDisclosure {
        return AdapterResult {
            catalog_test_id: spec.catalog_test_id.to_owned(),
            adapter_name: spec.adapter_name.to_owned(),
            adapter_version: spec.adapter_version.to_owned(),
            execution_status: ExecutionStatus::Completed,
            security_outcome: SecurityOutcome::Inconclusive,
            severity: None,
            confidence: 0.3,
            summary:
                "The target returned a server error; the security observation is inconclusive."
                    .to_owned(),
            duration_ms: 0,
            evidence: vec![status_evidence(&response)],
            error_code: None,
            extensions: AdapterExtensions::default(),
        };
    }

    match spec.kind {
        AdapterKind::HttpHeaders => normalize_headers(spec, &response),
        AdapterKind::HttpCors => normalize_cors(spec, &response),
        AdapterKind::HttpCookies => normalize_cookies(spec, &response),
        AdapterKind::HttpInfoDisclosure => normalize_info_disclosure(spec, &response),
        AdapterKind::HttpRateLimit | AdapterKind::TlsBaseline | AdapterKind::StaticSecrets => {
            failure_result(
                spec,
                ExecutionStatus::Failed,
                "unsupported_adapter_dispatch",
                "The adapter could not be dispatched.",
            )
        }
    }
}

fn normalize_headers(
    spec: &'static AdapterSpec,
    response: &probe_forge_target_policy::GuardedResponse,
) -> AdapterResult {
    let scheme = response.final_url.scheme();
    let required = if scheme == "https" {
        REQUIRED_HTTPS_HEADERS
    } else {
        REQUIRED_HTTP_HEADERS
    };
    let mut missing = Vec::new();
    let mut present = Vec::new();
    for header in required {
        if response.headers.contains_key(*header) {
            present.push((*header).to_owned());
        } else {
            missing.push((*header).to_owned());
        }
    }
    let evidence = AdapterEvidence {
        kind: "http_response_headers".to_owned(),
        summary: "Observed security headers for the authorized route.".to_owned(),
        attributes: json!({
            "status": response.status,
            "finalPath": response.final_url.path(),
            "scheme": scheme,
            "missingHeaders": missing.clone(),
            "presentHeaders": present.clone(),
        }),
    };
    if missing.is_empty() {
        AdapterResult {
            catalog_test_id: spec.catalog_test_id.to_owned(),
            adapter_name: spec.adapter_name.to_owned(),
            adapter_version: spec.adapter_version.to_owned(),
            execution_status: ExecutionStatus::Completed,
            security_outcome: SecurityOutcome::Passed,
            severity: None,
            confidence: 0.8,
            summary: "Required baseline security headers were present.".to_owned(),
            duration_ms: 0,
            evidence: vec![evidence],
            error_code: None,
            extensions: AdapterExtensions::default(),
        }
    } else {
        AdapterResult {
            catalog_test_id: spec.catalog_test_id.to_owned(),
            adapter_name: spec.adapter_name.to_owned(),
            adapter_version: spec.adapter_version.to_owned(),
            execution_status: ExecutionStatus::Completed,
            security_outcome: SecurityOutcome::Finding,
            severity: Some(Severity::Low),
            confidence: 0.9,
            summary: format!("Missing baseline security headers: {}.", missing.join(", ")),
            duration_ms: 0,
            evidence: vec![evidence],
            error_code: None,
            extensions: AdapterExtensions {
                remediation: Some(
                    "Set the missing response headers on the affected route, including HSTS on HTTPS origins."
                        .to_owned(),
                ),
                cwe_ids: vec!["CWE-693".to_owned()],
                affected_assets: vec![response.final_url.path().to_owned()],
                rule_id: Some("PF-HTTP-001".to_owned()),
                references: vec![
                    "https://owasp.org/www-project-secure-headers/".to_owned(),
                ],
                ..AdapterExtensions::default()
            },
        }
    }
}

fn normalize_cors(
    spec: &'static AdapterSpec,
    response: &probe_forge_target_policy::GuardedResponse,
) -> AdapterResult {
    let allow_origin = response
        .headers
        .get("access-control-allow-origin")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or("");
    let allow_credentials = response
        .headers
        .get("access-control-allow-credentials")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let reflected = allow_origin.eq_ignore_ascii_case(PROBE_ORIGIN);
    let wildcard = allow_origin == "*";
    let (outcome, severity, confidence, summary) = if wildcard && allow_credentials {
        (
            SecurityOutcome::Finding,
            Some(Severity::High),
            0.95,
            "CORS allows any origin together with credentials.",
        )
    } else if reflected {
        (
            SecurityOutcome::Finding,
            Some(if allow_credentials {
                Severity::High
            } else {
                Severity::Medium
            }),
            0.9,
            if allow_credentials {
                "CORS reflects an arbitrary origin and allows credentials."
            } else {
                "CORS reflects an arbitrary origin."
            },
        )
    } else {
        (
            SecurityOutcome::Passed,
            None,
            0.8,
            "No overly broad CORS origin policy was observed.",
        )
    };
    AdapterResult {
        catalog_test_id: spec.catalog_test_id.to_owned(),
        adapter_name: spec.adapter_name.to_owned(),
        adapter_version: spec.adapter_version.to_owned(),
        execution_status: ExecutionStatus::Completed,
        security_outcome: outcome,
        severity,
        confidence,
        summary: summary.to_owned(),
        duration_ms: 0,
        evidence: vec![AdapterEvidence {
            kind: "cors_response".to_owned(),
            summary: "Observed CORS response headers for a harmless probe origin.".to_owned(),
            attributes: json!({
                "status": response.status,
                "finalPath": response.final_url.path(),
                "requestOrigin": PROBE_ORIGIN,
                "allowOrigin": allow_origin,
                "allowCredentials": allow_credentials,
            }),
        }],
        error_code: None,
        extensions: if outcome == SecurityOutcome::Finding {
            AdapterExtensions {
                remediation: Some(
                    "Restrict Access-Control-Allow-Origin to explicit trusted origins and never combine a wildcard with credentials."
                        .to_owned(),
                ),
                cwe_ids: vec!["CWE-942".to_owned()],
                affected_assets: vec![response.final_url.path().to_owned()],
                rule_id: Some("PF-HTTP-002".to_owned()),
                references: vec![
                    "https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS".to_owned(),
                ],
                ..AdapterExtensions::default()
            }
        } else {
            AdapterExtensions::default()
        },
    }
}

fn status_evidence(response: &probe_forge_target_policy::GuardedResponse) -> AdapterEvidence {
    AdapterEvidence {
        kind: "http_response_status".to_owned(),
        summary: "Observed response status without reading evidence from the body.".to_owned(),
        attributes: json!({
            "status": response.status,
            "finalPath": response.final_url.path(),
        }),
    }
}

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Info => 0,
        Severity::Low => 1,
        Severity::Medium => 2,
        Severity::High => 3,
        Severity::Critical => 4,
    }
}

fn max_severity(current: Option<Severity>, candidate: Severity) -> Option<Severity> {
    match current {
        Some(existing) if severity_rank(existing) >= severity_rank(candidate) => Some(existing),
        _ => Some(candidate),
    }
}

// ---------------------------------------------------------------------------
// Authentication rate limiting (bounded, read-only observation)
// ---------------------------------------------------------------------------

async fn run_rate_limit(
    spec: &'static AdapterSpec,
    client: &GuardedHttpClient,
    guard: &TargetGuard,
    url: &Url,
    budget: &RunBudget,
) -> AdapterResult {
    let started = Instant::now();
    let mut statuses: Vec<u16> = Vec::new();
    let mut retry_after_present = false;
    let mut throttled = false;
    let mut observed_server_error = false;
    let mut budget_exhausted = false;

    for attempt in 0..RATE_LIMIT_ATTEMPTS {
        if attempt > 0 {
            tokio::select! {
                _ = budget.cancelled() => {
                    return failure_result(
                        spec,
                        ExecutionStatus::Cancelled,
                        "cancelled",
                        "The adapter was cancelled.",
                    );
                }
                _ = tokio::time::sleep(RATE_LIMIT_INTERVAL) => {}
            }
        }
        match client.get(guard, url, budget).await {
            Ok(response) => {
                statuses.push(response.status);
                let has_retry_after = response.headers.contains_key(reqwest::header::RETRY_AFTER);
                if has_retry_after {
                    retry_after_present = true;
                }
                if response.status == 429 || (response.status == 503 && has_retry_after) {
                    throttled = true;
                } else if response.status >= 500 {
                    observed_server_error = true;
                }
                if throttled {
                    break;
                }
            }
            Err(TargetRequestError::MaxRequests) => {
                budget_exhausted = true;
                break;
            }
            Err(error) => return request_failure(spec, error),
        }
    }

    let evidence = AdapterEvidence {
        kind: "rate_limit_observation".to_owned(),
        summary:
            "Bounded sequence of unauthenticated GET probes used only to observe throttling responses."
                .to_owned(),
        attributes: json!({
            "method": "GET",
            "attempts": statuses.len(),
            "maxAttempts": RATE_LIMIT_ATTEMPTS,
            "minimumIntervalMs": RATE_LIMIT_INTERVAL.as_millis(),
            "statuses": statuses,
            "throttled": throttled,
            "retryAfterPresent": retry_after_present,
            "limitation":
                "The adapter cannot exercise credential submission or method-specific limits; absence of a 429 on read-only probes is a suspected, not confirmed, finding.",
            "finalPath": url.path(),
        }),
    };

    if budget_exhausted {
        return AdapterResult {
            catalog_test_id: spec.catalog_test_id.to_owned(),
            adapter_name: spec.adapter_name.to_owned(),
            adapter_version: spec.adapter_version.to_owned(),
            execution_status: ExecutionStatus::Failed,
            security_outcome: SecurityOutcome::Inconclusive,
            severity: None,
            confidence: 0.0,
            summary:
                "The run request budget was exhausted before the bounded rate-limit observation completed."
                    .to_owned(),
            duration_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            evidence: vec![evidence],
            error_code: Some("request_budget_exhausted".to_owned()),
            extensions: AdapterExtensions::default(),
        };
    }

    let (outcome, severity, confidence, summary) = if throttled {
        (
            SecurityOutcome::Passed,
            None,
            0.9,
            "A throttling response was observed during the bounded probe sequence.".to_owned(),
        )
    } else if observed_server_error {
        (
            SecurityOutcome::Inconclusive,
            None,
            0.3,
            "The target returned server errors during the bounded probe sequence.".to_owned(),
        )
    } else {
        (
            SecurityOutcome::Finding,
            Some(Severity::Medium),
            0.6,
            format!(
                "No rate limiting was observed within {RATE_LIMIT_ATTEMPTS} bounded read-only probes; authentication throttling is unconfirmed."
            ),
        )
    };

    AdapterResult {
        catalog_test_id: spec.catalog_test_id.to_owned(),
        adapter_name: spec.adapter_name.to_owned(),
        adapter_version: spec.adapter_version.to_owned(),
        execution_status: ExecutionStatus::Completed,
        security_outcome: outcome,
        severity,
        confidence,
        summary,
        duration_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        evidence: vec![evidence],
        error_code: None,
        extensions: if outcome == SecurityOutcome::Finding {
            AdapterExtensions {
                remediation: Some(
                    "Enforce per-account and per-IP throttling on authentication endpoints and return HTTP 429 with Retry-After."
                        .to_owned(),
                ),
                cwe_ids: vec!["CWE-307".to_owned(), "CWE-799".to_owned()],
                affected_assets: vec![url.path().to_owned()],
                rule_id: Some("PF-HTTP-003".to_owned()),
                references: vec![
                    "https://owasp.org/www-community/controls/Blocking_Brute_Force_Attacks".to_owned(),
                ],
                ..AdapterExtensions::default()
            }
        } else {
            AdapterExtensions::default()
        },
    }
}

// ---------------------------------------------------------------------------
// Cookie attribute observation
// ---------------------------------------------------------------------------

struct CookieObservation {
    name: String,
    secure: bool,
    http_only: bool,
    same_site: Option<String>,
    session_like: bool,
    issues: Vec<&'static str>,
    severity: Option<Severity>,
}

fn is_session_like_cookie(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["sess", "sid", "token", "auth", "jwt", "login", "remember"]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn normalize_cookies(
    spec: &'static AdapterSpec,
    response: &probe_forge_target_policy::GuardedResponse,
) -> AdapterResult {
    let is_https = response.final_url.scheme() == "https";
    let mut observations: Vec<CookieObservation> = Vec::new();
    let mut finding_severity: Option<Severity> = None;
    let mut inspected = 0_usize;

    for value in response
        .headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .take(MAX_COOKIES_INSPECTED)
    {
        inspected += 1;
        let Ok(text) = value.to_str() else {
            continue;
        };
        let Ok(cookie) = cookie::Cookie::parse(text) else {
            continue;
        };
        let name: String = cookie.name().chars().take(64).collect();
        if name.is_empty() {
            continue;
        }
        let secure = cookie.secure().unwrap_or(false);
        let http_only = cookie.http_only().unwrap_or(false);
        let same_site = cookie.same_site().map(|value| match value {
            cookie::SameSite::Strict => "strict".to_owned(),
            cookie::SameSite::Lax => "lax".to_owned(),
            cookie::SameSite::None => "none".to_owned(),
        });
        let session_like = is_session_like_cookie(&name);
        let mut issues = Vec::new();
        let mut severity: Option<Severity> = None;

        if is_https && !secure {
            issues.push("missing_secure");
            severity = max_severity(severity, Severity::Medium);
        }
        if !http_only {
            issues.push("missing_http_only");
            severity = max_severity(
                severity,
                if session_like {
                    Severity::High
                } else {
                    Severity::Medium
                },
            );
        }
        match same_site.as_deref() {
            Some("none") if !secure => {
                issues.push("same_site_none_without_secure");
                severity = max_severity(severity, Severity::Medium);
            }
            Some("none") => {
                issues.push("same_site_none");
                severity = max_severity(severity, Severity::Low);
            }
            None => {
                issues.push("missing_same_site");
                severity = max_severity(severity, Severity::Low);
            }
            _ => {}
        }

        if let Some(current) = severity {
            finding_severity = max_severity(finding_severity, current);
        }
        observations.push(CookieObservation {
            name,
            secure,
            http_only,
            same_site,
            session_like,
            issues,
            severity,
        });
    }

    let flagged = observations
        .iter()
        .filter(|observation| observation.severity.is_some())
        .count();
    let evidence = AdapterEvidence {
        kind: "cookie_security".to_owned(),
        summary:
            "Cookie attribute flags observed in Set-Cookie headers. Cookie values are never read or returned."
                .to_owned(),
        attributes: json!({
            "status": response.status,
            "finalPath": response.final_url.path(),
            "scheme": response.final_url.scheme(),
            "cookiesInspected": inspected,
            "cookiesFlagged": flagged,
            "cookies": observations.iter().map(|observation| json!({
                "name": observation.name,
                "secure": observation.secure,
                "httpOnly": observation.http_only,
                "sameSite": observation.same_site,
                "sessionLike": observation.session_like,
                "issues": observation.issues,
            })).collect::<Vec<_>>(),
        }),
    };

    let (outcome, severity, confidence, summary) = if inspected == 0 {
        (
            SecurityOutcome::Passed,
            None,
            0.7,
            "The tested route did not set any cookies.".to_owned(),
        )
    } else if flagged == 0 {
        (
            SecurityOutcome::Passed,
            None,
            0.9,
            "All observed cookies carried the baseline protective attributes.".to_owned(),
        )
    } else {
        (
            SecurityOutcome::Finding,
            finding_severity,
            0.9,
            format!(
                "{flagged} of {inspected} observed cookies are missing one or more protective attributes."
            ),
        )
    };

    AdapterResult {
        catalog_test_id: spec.catalog_test_id.to_owned(),
        adapter_name: spec.adapter_name.to_owned(),
        adapter_version: spec.adapter_version.to_owned(),
        execution_status: ExecutionStatus::Completed,
        security_outcome: outcome,
        severity,
        confidence,
        summary,
        duration_ms: 0,
        evidence: vec![evidence],
        error_code: None,
        extensions: if outcome == SecurityOutcome::Finding {
            AdapterExtensions {
                remediation: Some(
                    "Set Secure and HttpOnly on sensitive cookies and use SameSite=Lax or Strict unless cross-site delivery is required."
                        .to_owned(),
                ),
                cwe_ids: vec![
                    "CWE-614".to_owned(),
                    "CWE-1004".to_owned(),
                    "CWE-1275".to_owned(),
                ],
                affected_assets: vec![response.final_url.path().to_owned()],
                rule_id: Some("PF-HTTP-004".to_owned()),
                references: vec![
                    "https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Set-Cookie".to_owned(),
                ],
                ..AdapterExtensions::default()
            }
        } else {
            AdapterExtensions::default()
        },
    }
}

// ---------------------------------------------------------------------------
// Passive information disclosure observation
// ---------------------------------------------------------------------------

struct BodyMarker {
    id: &'static str,
    needle: &'static str,
    severity: Severity,
}

const BODY_MARKERS: &[BodyMarker] = &[
    BodyMarker {
        id: "python_traceback",
        needle: "Traceback (most recent call last)",
        severity: Severity::High,
    },
    BodyMarker {
        id: "sql_error",
        needle: "SQLSTATE[",
        severity: Severity::High,
    },
    BodyMarker {
        id: "passwd_file_contents",
        needle: "root:x:0:0",
        severity: Severity::High,
    },
    BodyMarker {
        id: "java_stack_trace",
        needle: "java.lang.",
        severity: Severity::Medium,
    },
    BodyMarker {
        id: "dotnet_stack_trace",
        needle: "System.NullReferenceException",
        severity: Severity::Medium,
    },
    BodyMarker {
        id: "generic_stack_trace",
        needle: "Stack trace:",
        severity: Severity::Medium,
    },
    BodyMarker {
        id: "php_error_path",
        needle: " on line ",
        severity: Severity::Low,
    },
    BodyMarker {
        id: "directory_listing",
        needle: "<title>Index of /",
        severity: Severity::Low,
    },
    BodyMarker {
        id: "debug_comment",
        needle: "<!-- DEBUG",
        severity: Severity::Low,
    },
];

const DEBUG_RESPONSE_HEADERS: &[&str] = &[
    "x-debug-token",
    "x-debug-token-link",
    "x-aspnet-version",
    "x-aspnetmvc-version",
];

fn safe_header_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 80 {
        return None;
    }
    if value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(
                character,
                ' ' | '.' | '-' | '_' | '/' | '(' | ')' | ':' | '+' | ';' | '=' | ','
            )
    }) {
        Some(value.to_owned())
    } else {
        None
    }
}

fn header_string(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(safe_header_value)
}

fn normalize_info_disclosure(
    spec: &'static AdapterSpec,
    response: &probe_forge_target_policy::GuardedResponse,
) -> AdapterResult {
    let mut severity: Option<Severity> = None;
    let mut findings: Vec<&'static str> = Vec::new();

    let server_header = header_string(&response.headers, "server");
    let server_banner = server_header.is_some();
    let server_versioned = server_header
        .as_deref()
        .is_some_and(|value| value.chars().any(|character| character.is_ascii_digit()));
    if server_banner {
        findings.push(if server_versioned {
            "versioned_server_banner"
        } else {
            "server_banner"
        });
        severity = max_severity(
            severity,
            if server_versioned {
                Severity::Low
            } else {
                Severity::Info
            },
        );
    }

    let powered_by = header_string(&response.headers, "x-powered-by");
    let framework_banner = powered_by.is_some();
    if framework_banner {
        findings.push("framework_banner");
        severity = max_severity(severity, Severity::Low);
    }

    let mut debug_headers: Vec<&'static str> = Vec::new();
    for name in DEBUG_RESPONSE_HEADERS {
        if response.headers.contains_key(*name) {
            debug_headers.push(name);
            findings.push("debug_header");
            severity = max_severity(severity, Severity::Medium);
        }
    }

    let content_type = header_string(&response.headers, "content-type")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let textual = content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("javascript");
    let mut matched_markers: Vec<&'static str> = Vec::new();
    let mut body_truncated = false;
    if textual && !response.body.is_empty() {
        const MAX_SCAN_BYTES: usize = 262_144;
        let slice = &response.body[..response.body.len().min(MAX_SCAN_BYTES)];
        body_truncated = response.body.len() > MAX_SCAN_BYTES;
        let text = String::from_utf8_lossy(slice);
        for marker in BODY_MARKERS {
            if text.contains(marker.needle) {
                matched_markers.push(marker.id);
                findings.push(marker.id);
                severity = max_severity(severity, marker.severity);
            }
        }
    }

    let evidence = AdapterEvidence {
        kind: "information_disclosure".to_owned(),
        summary:
            "Passive inspection of response banners, debug headers, and bounded body markers. Response bodies are not retained."
                .to_owned(),
        attributes: json!({
            "status": response.status,
            "finalPath": response.final_url.path(),
            "serverBanner": server_banner,
            "serverBannerVersioned": server_versioned,
            "frameworkBanner": framework_banner,
            "debugHeaders": debug_headers,
            "bodyMarkers": matched_markers,
            "bodyScanTruncated": body_truncated,
        }),
    };

    findings.sort_unstable();
    findings.dedup();
    let flagged = !findings.is_empty();
    let server_error = response.status >= 500;
    AdapterResult {
        catalog_test_id: spec.catalog_test_id.to_owned(),
        adapter_name: spec.adapter_name.to_owned(),
        adapter_version: spec.adapter_version.to_owned(),
        execution_status: ExecutionStatus::Completed,
        security_outcome: if flagged {
            SecurityOutcome::Finding
        } else if server_error {
            SecurityOutcome::Inconclusive
        } else {
            SecurityOutcome::Passed
        },
        severity: if flagged { severity } else { None },
        confidence: if flagged {
            0.8
        } else if server_error {
            0.3
        } else {
            0.7
        },
        summary: if flagged {
            format!(
                "The response discloses implementation or debug information: {}.",
                findings.join(", ")
            )
        } else if server_error {
            "The target returned a server error without observable disclosure markers; the check is inconclusive."
                .to_owned()
        } else {
            "No version banners, debug headers, or stack-trace markers were observed.".to_owned()
        },
        duration_ms: 0,
        evidence: vec![evidence],
        error_code: None,
        extensions: if flagged {
            AdapterExtensions {
                remediation: Some(
                    "Remove version banners and debug headers in deployed environments and return generic error pages without stack traces or internal paths."
                        .to_owned(),
                ),
                cwe_ids: vec!["CWE-200".to_owned(), "CWE-209".to_owned()],
                affected_assets: vec![response.final_url.path().to_owned()],
                rule_id: Some("PF-HTTP-005".to_owned()),
                references: vec![
                    "https://owasp.org/www-project-top-ten/2017/A3_2017-Sensitive_Data_Exposure".to_owned(),
                ],
                ..AdapterExtensions::default()
            }
        } else {
            AdapterExtensions::default()
        },
    }
}

// ---------------------------------------------------------------------------
// TLS baseline (bounded handshake, certificate metadata only)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CapturingVerifier {
    supported: WebPkiSupportedAlgorithms,
    chain: Mutex<Vec<CertificateDer<'static>>>,
}

impl ServerCertVerifier for CapturingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let mut chain = self
            .chain
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        chain.clear();
        chain.push(end_entity.clone().into_owned());
        for certificate in intermediates {
            chain.push(certificate.clone().into_owned());
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

struct TlsObservation {
    protocol: Option<String>,
    cipher_suite: Option<String>,
    chain: Vec<CertificateDer<'static>>,
}

#[derive(Debug)]
enum TlsFailure {
    Connect,
    Handshake(rustls::Error),
    Io,
    Cancelled,
    Timeout,
}

#[derive(Debug)]
struct CertificateMetadata {
    not_after_unix: i64,
    not_before_unix: i64,
    subject: String,
    issuer: String,
    self_signed: bool,
    san_count: usize,
}

fn bounded_name(value: String) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(200)
        .collect()
}

fn certificate_metadata(der: &[u8]) -> Option<CertificateMetadata> {
    let (_, certificate) = x509_parser::parse_x509_certificate(der).ok()?;
    let validity = certificate.validity();
    let not_after_unix = validity.not_after.timestamp();
    let not_before_unix = validity.not_before.timestamp();
    let subject = bounded_name(certificate.subject().to_string());
    let issuer = bounded_name(certificate.issuer().to_string());
    let self_signed = subject == issuer;
    let san_count = certificate
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|extension| extension.value.general_names.len())
        .unwrap_or(0);
    Some(CertificateMetadata {
        not_after_unix,
        not_before_unix,
        subject,
        issuer,
        self_signed,
        san_count,
    })
}

fn certificate_fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn root_store() -> Option<RootCertStore> {
    let result = rustls_native_certs::load_native_certs();
    if !result.errors.is_empty() {
        tracing::warn!(
            event = "tls.native_roots_partial",
            errors = result.errors.len(),
            "some native root certificates could not be loaded"
        );
    }
    let mut roots = RootCertStore::empty();
    for certificate in result.certs {
        let _ = roots.add(certificate);
    }
    if roots.is_empty() { None } else { Some(roots) }
}

fn verified_config() -> Option<Arc<ClientConfig>> {
    let roots = root_store()?;
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Some(Arc::new(config))
}

/// Health signal for the execution environment: the TLS adapter cannot verify
/// certificates without a native root store.
pub fn tls_root_store_available() -> bool {
    root_store().is_some()
}

fn capturing_config() -> Option<(Arc<ClientConfig>, Arc<CapturingVerifier>)> {
    let supported = rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms;
    let verifier = Arc::new(CapturingVerifier {
        supported,
        chain: Mutex::new(Vec::new()),
    });
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    Some((Arc::new(config), verifier))
}

async fn tls_handshake(
    addresses: &[std::net::IpAddr],
    port: u16,
    server_name: ServerName<'static>,
    config: Arc<ClientConfig>,
    budget: &RunBudget,
) -> Result<TlsObservation, TlsFailure> {
    let mut stream = None;
    for address in addresses {
        let connect_budget = budget
            .remaining()
            .min(Duration::from_secs(10))
            .max(Duration::from_millis(1));
        let attempt = tokio::select! {
            _ = budget.cancelled() => return Err(TlsFailure::Cancelled),
            result = tokio::time::timeout(
                connect_budget,
                tokio::net::TcpStream::connect(std::net::SocketAddr::new(*address, port)),
            ) => result,
        };
        match attempt {
            Ok(Ok(connected)) => {
                stream = Some(connected);
                break;
            }
            Ok(Err(_)) => continue,
            Err(_) => return Err(TlsFailure::Timeout),
        }
    }
    let Some(stream) = stream else {
        return Err(TlsFailure::Connect);
    };

    let connector = tokio_rustls::TlsConnector::from(config);
    let handshake = tokio::select! {
        _ = budget.cancelled() => return Err(TlsFailure::Cancelled),
        result = tokio::time::timeout(budget.remaining(), connector.connect(server_name, stream)) => result,
    };
    let stream = match handshake {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            let rustls_error = error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
                .cloned();
            return Err(match rustls_error {
                Some(error) => TlsFailure::Handshake(error),
                None => TlsFailure::Io,
            });
        }
        Err(_) => return Err(TlsFailure::Timeout),
    };

    let (_, session) = stream.get_ref();
    let protocol = session
        .protocol_version()
        .map(|version| format!("{version:?}"));
    let cipher_suite = session
        .negotiated_cipher_suite()
        .map(|suite| format!("{:?}", suite.suite()));
    let chain = session
        .peer_certificates()
        .map(<[CertificateDer<'static>]>::to_vec)
        .unwrap_or_default();
    Ok(TlsObservation {
        protocol,
        cipher_suite,
        chain,
    })
}

fn tls_certificate_error(
    error: &rustls::CertificateError,
) -> (&'static str, Severity, &'static str) {
    match error {
        rustls::CertificateError::Expired => (
            "tls_certificate_expired",
            Severity::High,
            "The server certificate has expired.",
        ),
        rustls::CertificateError::NotValidYet => (
            "tls_certificate_not_yet_valid",
            Severity::High,
            "The server certificate is not yet valid.",
        ),
        rustls::CertificateError::NotValidForName => (
            "tls_certificate_hostname_mismatch",
            Severity::High,
            "The server certificate is not valid for the requested hostname.",
        ),
        rustls::CertificateError::Revoked => (
            "tls_certificate_revoked",
            Severity::High,
            "The server certificate has been revoked.",
        ),
        rustls::CertificateError::UnknownIssuer => (
            "tls_certificate_untrusted_issuer",
            Severity::Medium,
            "The server certificate was issued by an untrusted authority or is self-signed.",
        ),
        _ => (
            "tls_certificate_invalid",
            Severity::Medium,
            "The server certificate failed verification.",
        ),
    }
}

fn tls_evidence(
    host: &str,
    port: u16,
    observation: &TlsObservation,
    verification: &str,
    verification_code: Option<&str>,
) -> AdapterEvidence {
    let leaf = observation.chain.first();
    let metadata = leaf.and_then(|certificate| certificate_metadata(certificate.as_ref()));
    let fingerprint = leaf.map(|certificate| certificate_fingerprint(certificate.as_ref()));
    let days_remaining = metadata
        .as_ref()
        .map(|metadata| (metadata.not_after_unix - now_unix()) / 86_400);
    AdapterEvidence {
        kind: "tls_observation".to_owned(),
        summary: "Bounded TLS handshake and certificate metadata for the authorized origin."
            .to_owned(),
        attributes: json!({
            "hostname": host,
            "port": port,
            "protocol": observation.protocol,
            "cipherSuite": observation.cipher_suite,
            "chainLength": observation.chain.len(),
            "leafCertificateSha256": fingerprint,
            "verification": verification,
            "verificationCode": verification_code,
            "subject": metadata.as_ref().map(|metadata| metadata.subject.as_str()),
            "issuer": metadata.as_ref().map(|metadata| metadata.issuer.as_str()),
            "selfSigned": metadata.as_ref().map(|metadata| metadata.self_signed),
            "subjectAlternativeNames": metadata.as_ref().map(|metadata| metadata.san_count),
            "notBeforeUnix": metadata.as_ref().map(|metadata| metadata.not_before_unix),
            "notAfterUnix": metadata.as_ref().map(|metadata| metadata.not_after_unix),
            "daysUntilExpiry": days_remaining,
        }),
    }
}

fn tls_failure_result(spec: &'static AdapterSpec, failure: TlsFailure) -> AdapterResult {
    match failure {
        TlsFailure::Cancelled => failure_result(
            spec,
            ExecutionStatus::Cancelled,
            "cancelled",
            "The adapter was cancelled.",
        ),
        TlsFailure::Timeout => failure_result(
            spec,
            ExecutionStatus::TimedOut,
            "tls_timeout",
            "The TLS handshake did not complete within the configured limit.",
        ),
        TlsFailure::Connect => failure_result(
            spec,
            ExecutionStatus::Failed,
            "tls_connect_failed",
            "A TCP connection to the authorized TLS port could not be established.",
        ),
        TlsFailure::Io => failure_result(
            spec,
            ExecutionStatus::Failed,
            "tls_handshake_failed",
            "The TLS handshake failed at the transport boundary.",
        ),
        TlsFailure::Handshake(error) => match error {
            rustls::Error::InvalidCertificate(error) => {
                let (code, _, message) = tls_certificate_error(&error);
                failure_result(spec, ExecutionStatus::Failed, code, message)
            }
            _ => failure_result(
                spec,
                ExecutionStatus::Failed,
                "tls_handshake_failed",
                "The TLS handshake failed at the transport boundary.",
            ),
        },
    }
}

async fn run_tls_baseline(
    spec: &'static AdapterSpec,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
) -> AdapterResult {
    if empty_configuration(configuration).is_err() {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            "invalid_adapter_configuration",
            "This test accepts no configuration values.",
        );
    }
    let origin = match Url::parse(&environment.guard.scope.origin) {
        Ok(origin) => origin,
        Err(_) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "invalid_target_scope",
                "The authorized target origin is invalid.",
            );
        }
    };
    if origin.scheme() != "https" {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            "tls_requires_https",
            "The TLS baseline test requires an authorized HTTPS origin.",
        );
    }
    if let Err(error) = validate_target_authority(&environment.guard, &origin, &environment.policy)
    {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            error.code(),
            error.safe_message(),
        );
    }
    let host = match origin.host() {
        Some(url::Host::Domain(domain)) => domain.to_ascii_lowercase(),
        Some(url::Host::Ipv4(address)) => address.to_string(),
        Some(url::Host::Ipv6(address)) => address.to_string(),
        None => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "invalid_target_host",
                "The authorized target host is invalid.",
            );
        }
    };
    let Some(port) = origin.port_or_known_default() else {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            "target_port_not_allowed",
            "The authorized target port is missing.",
        );
    };
    let server_name = match ServerName::try_from(host.clone()) {
        Ok(server_name) => server_name,
        Err(_) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "invalid_target_host",
                "The authorized target host is not a valid TLS server name.",
            );
        }
    };
    let resolved = match environment.resolver.resolve(&host, port).await {
        Ok(addresses) => addresses,
        Err(error) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                error.code(),
                error.safe_message(),
            );
        }
    };
    let addresses: Vec<std::net::IpAddr> = resolved
        .into_iter()
        .filter(|address| is_address_allowed(*address, &environment.policy))
        .collect();
    if addresses.is_empty() {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            "unsafe_destination",
            "The target host resolved only to addresses blocked by server policy.",
        );
    }

    let permit = match budget.acquire().await {
        Ok(permit) => permit,
        Err(error) => return request_failure(spec, error),
    };
    let Some(config) = verified_config() else {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            "tls_crypto_unavailable",
            "No trusted certificate roots are available to the execution environment.",
        );
    };

    let deadline = budget.remaining().min(spec.timeout);
    let verified = tokio::time::timeout(
        deadline,
        tls_handshake(&addresses, port, server_name.clone(), config, budget),
    )
    .await
    .unwrap_or(Err(TlsFailure::Timeout));

    let (observation, verification, verification_code, finding) = match verified {
        Ok(observation) => (observation, "verified", None, None),
        Err(TlsFailure::Handshake(rustls::Error::InvalidCertificate(error))) => {
            let (code, severity, message) = tls_certificate_error(&error);
            let Some((capture_config, _verifier)) = capturing_config() else {
                return failure_result(
                    spec,
                    ExecutionStatus::Failed,
                    "tls_crypto_unavailable",
                    "TLS certificate observation is unavailable in this environment.",
                );
            };
            match tls_handshake(&addresses, port, server_name, capture_config, budget).await {
                Ok(observation) => (
                    observation,
                    "failed",
                    Some(code),
                    Some((severity, code, message)),
                ),
                Err(failure) => return tls_failure_result(spec, failure),
            }
        }
        Err(failure) => {
            drop(permit);
            return tls_failure_result(spec, failure);
        }
    };
    drop(permit);

    let evidence = tls_evidence(&host, port, &observation, verification, verification_code);
    let days_remaining = observation
        .chain
        .first()
        .and_then(|certificate| certificate_metadata(certificate.as_ref()))
        .map(|metadata| (metadata.not_after_unix - now_unix()) / 86_400);

    let (outcome, severity, confidence, summary, rule_id) =
        if let Some((severity, code, message)) = finding {
            (
                SecurityOutcome::Finding,
                Some(severity),
                0.85,
                format!("{message} ({code})."),
                "PF-TLS-001",
            )
        } else if days_remaining.is_some_and(|days| days <= 30) {
            (
                SecurityOutcome::Finding,
                Some(Severity::Low),
                0.9,
                "The server certificate is valid but expires within 30 days.".to_owned(),
                "PF-TLS-003",
            )
        } else {
            (
                SecurityOutcome::Passed,
                None,
                0.9,
                format!(
                    "The TLS handshake completed with {} and a verified certificate.",
                    observation
                        .protocol
                        .as_deref()
                        .unwrap_or("a supported protocol")
                ),
                "PF-TLS-000",
            )
        };

    AdapterResult {
        catalog_test_id: spec.catalog_test_id.to_owned(),
        adapter_name: spec.adapter_name.to_owned(),
        adapter_version: spec.adapter_version.to_owned(),
        execution_status: ExecutionStatus::Completed,
        security_outcome: outcome,
        severity,
        confidence,
        summary,
        duration_ms: 0,
        evidence: vec![evidence],
        error_code: None,
        extensions: if outcome == SecurityOutcome::Finding {
            AdapterExtensions {
                remediation: Some(
                    "Renew and correctly chain the certificate for this hostname, then prefer TLS 1.3 with modern cipher suites."
                        .to_owned(),
                ),
                cwe_ids: vec!["CWE-295".to_owned()],
                affected_assets: vec![format!("{host}:{port}")],
                rule_id: Some(rule_id.to_owned()),
                references: vec![
                    "https://owasp.org/www-project-top-ten/2017/A6_2017-Security_Misconfiguration".to_owned(),
                ],
                ..AdapterExtensions::default()
            }
        } else {
            AdapterExtensions::default()
        },
    }
}

// ---------------------------------------------------------------------------
// Static secret exposure (server-owned pinned snapshot)
// ---------------------------------------------------------------------------

fn static_finding_evidence(finding: &probe_forge_static_scanner::SecretFinding) -> AdapterEvidence {
    AdapterEvidence {
        kind: "secret_pattern_match".to_owned(),
        summary:
            "A credential-shaped value matched a fixed rule. The value is replaced with *** and is never stored."
                .to_owned(),
        attributes: json!({
            "ruleId": finding.rule_id,
            "pattern": finding.pattern_name,
            "filePath": finding.file_path,
            "line": finding.line,
            "column": finding.column,
            "confidence": finding.confidence,
            "entropy": finding.entropy,
            "valueLength": finding.value_length,
            "preview": finding.redacted_preview,
        }),
    }
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, AtomicOrdering::SeqCst);
    }
}

async fn run_static_secrets(
    spec: &'static AdapterSpec,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
) -> AdapterResult {
    if empty_configuration(configuration).is_err() {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            "invalid_adapter_configuration",
            "This test accepts no configuration values.",
        );
    }
    let Some(snapshot) = environment.snapshot else {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            "repository_snapshot_unavailable",
            "No pinned repository snapshot was available to the execution worker.",
        );
    };

    let limits = probe_forge_static_scanner::ScanLimits::default();
    let scan_root = snapshot.root().to_path_buf();
    let cancel = Arc::new(AtomicBool::new(false));
    let scan_cancel = Arc::clone(&cancel);
    let mut task = tokio::task::spawn_blocking(move || {
        probe_forge_static_scanner::scan_snapshot_cancellable(&scan_root, &limits, &scan_cancel)
    });
    // If this future is dropped by the adapter timeout, the guard stops the
    // blocking scan at its next file/line boundary instead of letting it run on.
    let cancel_guard = CancelOnDrop(Arc::clone(&cancel));
    let scanned = tokio::select! {
        _ = budget.cancelled() => {
            cancel.store(true, AtomicOrdering::SeqCst);
            return failure_result(
                spec,
                ExecutionStatus::Cancelled,
                "cancelled",
                "The adapter was cancelled.",
            );
        }
        result = &mut task => result,
    };
    drop(cancel_guard);
    let report = match scanned {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "repository_snapshot_unreadable",
                match error {
                    probe_forge_static_scanner::ScanError::InvalidRoot => {
                        "The snapshot root was not an absolute directory."
                    }
                    probe_forge_static_scanner::ScanError::RootNotDirectory => {
                        "The snapshot root was not a directory."
                    }
                    probe_forge_static_scanner::ScanError::UnreadableRoot => {
                        "The snapshot root could not be read."
                    }
                },
            );
        }
        Err(_) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "static_scanner_failed",
                "The static scanner did not complete.",
            );
        }
    };
    if report.cancelled {
        return failure_result(
            spec,
            ExecutionStatus::Cancelled,
            "cancelled",
            "The adapter was cancelled before the snapshot scan completed.",
        );
    }

    let mut evidence: Vec<AdapterEvidence> = report
        .findings
        .iter()
        .take(50)
        .map(static_finding_evidence)
        .collect();
    evidence.push(AdapterEvidence {
        kind: "static_scan_summary".to_owned(),
        summary: "Bounded snapshot scan summary.".to_owned(),
        attributes: json!({
            "filesScanned": report.files_scanned,
            "bytesScanned": report.bytes_scanned,
            "skippedFiles": report.skipped_files,
            "findings": report.findings.len(),
            "truncated": report.truncated,
        }),
    });

    let mut affected_assets: Vec<String> = Vec::new();
    for finding in &report.findings {
        if affected_assets.len() >= 20 {
            break;
        }
        if !affected_assets.contains(&finding.file_path) {
            affected_assets.push(finding.file_path.clone());
        }
    }
    let max_confidence = report
        .findings
        .iter()
        .map(|finding| finding.confidence)
        .fold(0.0_f32, f32::max);
    let severity = if max_confidence >= 0.85 {
        Severity::High
    } else {
        Severity::Medium
    };
    let distinct_rules: std::collections::BTreeSet<&str> = report
        .findings
        .iter()
        .map(|finding| finding.rule_id)
        .collect();
    let rule_id = if distinct_rules.len() == 1 {
        distinct_rules.iter().next().map(|rule| (*rule).to_owned())
    } else {
        Some("PF-SEC".to_owned())
    };

    if report.findings.is_empty() {
        return AdapterResult {
            catalog_test_id: spec.catalog_test_id.to_owned(),
            adapter_name: spec.adapter_name.to_owned(),
            adapter_version: spec.adapter_version.to_owned(),
            execution_status: ExecutionStatus::Completed,
            security_outcome: if report.truncated {
                SecurityOutcome::Inconclusive
            } else {
                SecurityOutcome::Passed
            },
            severity: None,
            confidence: if report.truncated { 0.4 } else { 0.85 },
            summary: if report.truncated {
                "No credential indicators were found, but the bounded scan was truncated before completion."
                    .to_owned()
            } else {
                "No high-confidence credential indicators were found in the pinned snapshot."
                    .to_owned()
            },
            duration_ms: 0,
            evidence,
            error_code: None,
            extensions: AdapterExtensions::default(),
        };
    }

    AdapterResult {
        catalog_test_id: spec.catalog_test_id.to_owned(),
        adapter_name: spec.adapter_name.to_owned(),
        adapter_version: spec.adapter_version.to_owned(),
        execution_status: ExecutionStatus::Completed,
        security_outcome: SecurityOutcome::Finding,
        severity: Some(severity),
        confidence: f64::from(max_confidence),
        summary: format!(
            "{} potential credential exposure(s) were detected in the pinned snapshot. Values are redacted.",
            report.findings.len()
        ),
        duration_ms: 0,
        evidence,
        error_code: None,
        extensions: AdapterExtensions {
            remediation: Some(
                "Remove committed credentials, rotate every exposed value, and load secrets from a managed secret store at runtime."
                    .to_owned(),
            ),
            cwe_ids: vec!["CWE-798".to_owned(), "CWE-522".to_owned()],
            affected_assets,
            rule_id,
            references: vec![
                "https://owasp.org/www-community/vulnerabilities/Use_of_hard-coded_password".to_owned(),
            ],
            ..AdapterExtensions::default()
        },
    }
}

fn request_failure(spec: &'static AdapterSpec, error: TargetRequestError) -> AdapterResult {
    match error {
        TargetRequestError::Cancelled => failure_result(
            spec,
            ExecutionStatus::Cancelled,
            "cancelled",
            "The adapter was cancelled.",
        ),
        TargetRequestError::DurationExceeded => failure_result(
            spec,
            ExecutionStatus::TimedOut,
            "duration_budget_exhausted",
            "The run duration budget was exhausted.",
        ),
        TargetRequestError::MaxRequests => failure_result(
            spec,
            ExecutionStatus::Failed,
            "request_budget_exhausted",
            "The request budget was exhausted.",
        ),
        TargetRequestError::ResponseTooLarge => failure_result(
            spec,
            ExecutionStatus::Failed,
            "response_size_exceeded",
            "The response exceeded the configured size cap.",
        ),
        TargetRequestError::TotalBytesExceeded => failure_result(
            spec,
            ExecutionStatus::Failed,
            "total_data_budget_exhausted",
            "The run total-data budget was exhausted.",
        ),
        TargetRequestError::InvalidHeader => failure_result(
            spec,
            ExecutionStatus::Failed,
            "invalid_adapter_header",
            "The adapter attempted to send a disallowed request header.",
        ),
        TargetRequestError::RequestBodyTooLarge => failure_result(
            spec,
            ExecutionStatus::Failed,
            "request_body_too_large",
            "The adapter request exceeded the platform body cap.",
        ),
        TargetRequestError::RequestTimeout => failure_result(
            spec,
            ExecutionStatus::TimedOut,
            "request_timeout",
            "The target request timed out.",
        ),
        TargetRequestError::Network => failure_result(
            spec,
            ExecutionStatus::Failed,
            "network_error",
            "The target request failed at the network boundary.",
        ),
        TargetRequestError::Validation(error) => failure_result(
            spec,
            ExecutionStatus::Failed,
            error.code(),
            error.safe_message(),
        ),
    }
}

fn failure_result(
    spec: &'static AdapterSpec,
    execution_status: ExecutionStatus,
    error_code: &str,
    summary: &str,
) -> AdapterResult {
    AdapterResult {
        catalog_test_id: spec.catalog_test_id.to_owned(),
        adapter_name: spec.adapter_name.to_owned(),
        adapter_version: spec.adapter_version.to_owned(),
        execution_status,
        security_outcome: SecurityOutcome::Inconclusive,
        severity: None,
        confidence: 0.0,
        summary: summary.to_owned(),
        duration_ms: 0,
        evidence: Vec::new(),
        error_code: Some(error_code.to_owned()),
        extensions: AdapterExtensions::default(),
    }
}

fn unsupported_result(catalog_test_id: &str) -> AdapterResult {
    tracing::warn!(
        event = "adapter.unsupported",
        catalog_test_id,
        "no vetted adapter exists for this catalog test; failing closed"
    );
    AdapterResult {
        catalog_test_id: catalog_test_id.to_owned(),
        adapter_name: "none".to_owned(),
        adapter_version: "none".to_owned(),
        execution_status: ExecutionStatus::Unsupported,
        security_outcome: SecurityOutcome::NotRun,
        severity: None,
        confidence: 0.0,
        summary: "No vetted adapter exists for this catalog test; execution is disabled."
            .to_owned(),
        duration_ms: 0,
        evidence: Vec::new(),
        error_code: Some("unsupported_catalog_test".to_owned()),
        extensions: AdapterExtensions::default(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouteConfiguration {
    route: Option<String>,
    path: Option<String>,
}

fn route_from_configuration(configuration: &Value) -> Result<String, &'static str> {
    let parsed: RouteConfiguration = serde_json::from_value(configuration.clone())
        .map_err(|_| "Test configuration must contain only a route or path.")?;
    let route = match (parsed.route, parsed.path) {
        (Some(route), None) => route,
        (None, Some(path)) => path,
        (Some(route), Some(path)) if route.trim() == path.trim() => route,
        _ => {
            return Err("Test configuration must contain exactly one route or path value.");
        }
    };
    let route = route.trim();
    if !route.starts_with('/')
        || route.len() > 2_000
        || route.contains('?')
        || route.contains('#')
        || route.contains('\\')
        || route.contains("..")
        || route.contains("//")
        || route.chars().any(char::is_control)
    {
        return Err("Test route must be a safe absolute path.");
    }
    Ok(route.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    struct MockServer {
        port: u16,
        requests: Arc<AtomicUsize>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn write_response(
        stream: &mut TcpStream,
        status: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(body);
        let _ = stream.write_all(response.as_bytes()).await;
    }

    async fn start_mock_server() -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("mock bind");
        let port = listener.local_addr().expect("mock addr").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_server = requests.clone();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let requests = requests_for_server.clone();
                tokio::spawn(async move {
                    let mut buffer = [0_u8; 2048];
                    let Ok(read) = stream.read(&mut buffer).await else {
                        return;
                    };
                    requests.fetch_add(1, Ordering::SeqCst);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");
                    match path {
                        "/headers-missing" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[
                                    ("Content-Security-Policy", "default-src 'none'"),
                                    ("X-Content-Type-Options", "nosniff"),
                                    ("Set-Cookie", "session=server-secret"),
                                ],
                                "SECRET_RESPONSE_BODY",
                            )
                            .await;
                        }
                        "/headers-present" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[
                                    ("Content-Security-Policy", "default-src 'none'"),
                                    ("X-Content-Type-Options", "nosniff"),
                                    ("Referrer-Policy", "no-referrer"),
                                    ("Strict-Transport-Security", "max-age=31536000"),
                                ],
                                "ok",
                            )
                            .await;
                        }
                        "/cors-reflect" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[
                                    ("Access-Control-Allow-Origin", PROBE_ORIGIN),
                                    ("Access-Control-Allow-Credentials", "true"),
                                    ("Set-Cookie", "session=cors-server-secret"),
                                ],
                                "SECRET_CORS_BODY",
                            )
                            .await;
                        }
                        "/cors-safe" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[("Access-Control-Allow-Origin", "*")],
                                "ok",
                            )
                            .await;
                        }
                        "/cors-wildcard-credentials" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[
                                    ("Access-Control-Allow-Origin", "*"),
                                    ("Access-Control-Allow-Credentials", "true"),
                                ],
                                "ok",
                            )
                            .await;
                        }
                        "/rate-limited" => {
                            write_response(
                                &mut stream,
                                "429 Too Many Requests",
                                &[("Retry-After", "30")],
                                "slow down",
                            )
                            .await;
                        }
                        "/rate-open" => {
                            write_response(&mut stream, "200 OK", &[], "ok").await;
                        }
                        "/rate-error" => {
                            write_response(&mut stream, "500 Internal Server Error", &[], "error")
                                .await;
                        }
                        "/rate-unavailable" => {
                            write_response(
                                &mut stream,
                                "503 Service Unavailable",
                                &[],
                                "maintenance",
                            )
                            .await;
                        }
                        "/rate-503-limited" => {
                            write_response(
                                &mut stream,
                                "503 Service Unavailable",
                                &[("Retry-After", "10")],
                                "slow down",
                            )
                            .await;
                        }
                        "/cookies-insecure" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[
                                    ("Set-Cookie", "sessionid=abc123; Path=/"),
                                    ("Set-Cookie", "theme=dark; Path=/"),
                                ],
                                "ok",
                            )
                            .await;
                        }
                        "/cookies-safe" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[(
                                    "Set-Cookie",
                                    "sessionid=abc123; Secure; HttpOnly; SameSite=Lax; Path=/",
                                )],
                                "ok",
                            )
                            .await;
                        }
                        "/cookies-none" => {
                            write_response(&mut stream, "200 OK", &[], "ok").await;
                        }
                        "/info-disclosure" => {
                            write_response(
                                &mut stream,
                                "500 Internal Server Error",
                                &[
                                    ("Server", "Apache/2.4.41 (Ubuntu)"),
                                    ("X-Powered-By", "Express"),
                                    ("Content-Type", "text/html; charset=utf-8"),
                                ],
                                "<html>Traceback (most recent call last):\n  File \"/app/main.py\", line 42</html>",
                            )
                            .await;
                        }
                        "/info-clean" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[("Content-Type", "text/html; charset=utf-8")],
                                "<html>ok</html>",
                            )
                            .await;
                        }
                        "/server-error" => {
                            write_response(&mut stream, "500 Internal Server Error", &[], "error")
                                .await;
                        }
                        "/large" => {
                            let response = "HTTP/1.1 200 OK\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n";
                            let _ = stream.write_all(response.as_bytes()).await;
                        }
                        "/slow" => {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            write_response(&mut stream, "200 OK", &[], "slow").await;
                        }
                        _ => {
                            write_response(&mut stream, "404 Not Found", &[], "missing").await;
                        }
                    }
                });
            }
        });
        MockServer {
            port,
            requests,
            handle,
        }
    }

    fn environment(port: u16) -> AdapterEnvironment {
        AdapterEnvironment {
            guard: TargetGuard {
                environment: probeforge_contracts::Environment::Local,
                scope: probeforge_contracts::TargetScope {
                    origin: format!("http://127.0.0.1:{port}"),
                    allowed_hosts: vec!["127.0.0.1".to_owned()],
                    allowed_paths: vec!["/".to_owned()],
                    allowed_ports: vec![port],
                },
            },
            policy: TargetPolicy::local_test(),
            resolver: Arc::new(probe_forge_target_policy::SystemResolver),
            snapshot: None,
        }
    }

    fn https_environment(port: u16) -> AdapterEnvironment {
        let mut environment = environment(port);
        environment.guard.scope.origin = format!("https://127.0.0.1:{port}");
        environment
    }

    fn budget() -> RunBudget {
        RunBudget::new(
            probeforge_contracts::TargetLimits {
                max_requests_per_second: 100.0,
                max_concurrency: 1,
                max_duration_seconds: 5,
                max_requests: 10,
            },
            &TargetPolicy::local_test(),
        )
    }

    #[test]
    fn registry_is_versioned_unique_and_reports_capabilities() {
        assert_eq!(
            supported_catalog_ids().collect::<Vec<_>>(),
            vec![
                "http.headers.missing",
                "http.cors.misconfiguration",
                "http.auth.rate-limit",
                "http.cookies.insecure",
                "http.info-disclosure",
                "network.tls.baseline",
                "static.secrets.exposure",
            ]
        );
        let mut seen = std::collections::HashSet::new();
        for spec in adapters() {
            assert!(seen.insert(spec.catalog_test_id));
            assert!(!spec.adapter_name.is_empty());
            assert!(!spec.adapter_version.is_empty());
            let metadata = spec.metadata();
            assert_eq!(metadata.catalog_test_id, spec.catalog_test_id);
            assert_eq!(metadata.mode, "vulnerability_testing");
            assert!(!metadata.requires_test_accounts);
            assert!(matches!(metadata.category, "http" | "network" | "static"));
        }
        assert!(requires_repository_snapshot("static.secrets.exposure"));
        assert!(!requires_repository_snapshot("http.headers.missing"));
        assert_eq!(
            metadata_for("network.tls.baseline")
                .expect("tls metadata")
                .configuration,
            "none"
        );
        assert_eq!(
            metadata_for("network.tls.baseline")
                .expect("tls metadata")
                .dependencies,
            vec!["native_tls_roots"]
        );
        assert_eq!(
            metadata_for("static.secrets.exposure")
                .expect("static metadata")
                .dependencies,
            vec!["git"]
        );
        assert!(metadata_for("http.invented").is_none());
        assert_eq!(registry_metadata().len(), adapters().len());
    }

    #[test]
    fn route_configuration_accepts_one_route_and_rejects_ambiguous_or_unsafe_values() {
        assert_eq!(
            route_from_configuration(&json!({"route": "/api/items"})),
            Ok("/api/items".to_owned())
        );
        assert_eq!(
            route_from_configuration(&json!({"path": "/api/items"})),
            Ok("/api/items".to_owned())
        );
        assert_eq!(
            route_from_configuration(&json!({"route": " /api/items ", "path": "/api/items"})),
            Ok("/api/items".to_owned())
        );

        for invalid in [
            json!({}),
            json!({"route": "/a", "path": "/b"}),
            json!({"route": "/a", "method": "GET"}),
            json!({"route": 7}),
            json!(null),
        ] {
            assert!(
                route_from_configuration(&invalid).is_err(),
                "configuration {invalid} must be rejected"
            );
        }

        for unsafe_path in [
            "api/items",
            "//evil.example/path",
            "/../admin",
            "/api/items?x=1",
            "/api/items#fragment",
            "/api\\items",
            "/api//items",
            "/api/\u{0007}items",
        ] {
            assert!(
                route_from_configuration(&json!({"route": unsafe_path})).is_err(),
                "route {unsafe_path:?} must be rejected"
            );
        }
    }

    #[test]
    fn configuration_validation_is_contract_specific() {
        for route_kind in [
            "http.headers.missing",
            "http.cors.misconfiguration",
            "http.auth.rate-limit",
            "http.cookies.insecure",
            "http.info-disclosure",
        ] {
            assert!(validate_configuration(route_kind, &json!({"route": "/"})).is_ok());
            assert_eq!(
                validate_configuration(route_kind, &json!({})),
                Err("Test configuration must contain exactly one route or path value.")
            );
        }
        for empty_kind in ["network.tls.baseline", "static.secrets.exposure"] {
            assert!(validate_configuration(empty_kind, &json!({})).is_ok());
            assert!(validate_configuration(empty_kind, &json!(null)).is_ok());
            assert_eq!(
                validate_configuration(empty_kind, &json!({"route": "/"})),
                Err("This test accepts no configuration values.")
            );
            assert_eq!(
                validate_configuration(empty_kind, &json!({"command": "whoami"})),
                Err("This test accepts no configuration values.")
            );
        }
        assert_eq!(
            validate_configuration("http.headers.missing", &json!({"route": "relative"})),
            Err("Test route must be a safe absolute path.")
        );
        // Unsupported entries are skipped, so they have no configuration contract.
        assert!(validate_configuration("http.invented", &json!({})).is_ok());
    }

    #[tokio::test]
    async fn normalizes_missing_and_present_security_headers_without_body_leakage() {
        let server = start_mock_server().await;
        let environment = environment(server.port);
        let missing = execute(
            "http.headers.missing",
            environment.clone(),
            &json!({"route": "/headers-missing"}),
            &budget(),
        )
        .await;
        assert_eq!(missing.execution_status, ExecutionStatus::Completed);
        assert_eq!(missing.security_outcome, SecurityOutcome::Finding);
        assert_eq!(missing.severity, Some(Severity::Low));
        assert!(missing.summary.contains("referrer-policy"));
        assert_eq!(missing.extensions.rule_id.as_deref(), Some("PF-HTTP-001"));
        assert!(missing.extensions.cwe_ids.contains(&"CWE-693".to_owned()));
        let serialized = serde_json::to_string(&missing).expect("serialize result");
        assert!(!serialized.contains("SECRET_RESPONSE_BODY"));
        assert!(!serialized.contains("server-secret"));
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);

        let present = execute(
            "http.headers.missing",
            environment,
            &json!({"path": "/headers-present"}),
            &budget(),
        )
        .await;
        assert_eq!(present.execution_status, ExecutionStatus::Completed);
        assert_eq!(present.security_outcome, SecurityOutcome::Passed);
        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn detects_reflected_cors_with_credentials_and_accepts_wildcard_without_credentials() {
        let server = start_mock_server().await;
        let environment = environment(server.port);
        let reflected = execute(
            "http.cors.misconfiguration",
            environment.clone(),
            &json!({"route": "/cors-reflect"}),
            &budget(),
        )
        .await;
        assert_eq!(reflected.execution_status, ExecutionStatus::Completed);
        assert_eq!(reflected.security_outcome, SecurityOutcome::Finding);
        assert_eq!(reflected.severity, Some(Severity::High));
        assert!(reflected.summary.contains("reflects"));
        assert!(reflected.extensions.cwe_ids.contains(&"CWE-942".to_owned()));
        let serialized = serde_json::to_string(&reflected).expect("serialize CORS result");
        assert!(!serialized.contains("SECRET_CORS_BODY"));
        assert!(!serialized.contains("cors-server-secret"));

        let safe = execute(
            "http.cors.misconfiguration",
            environment.clone(),
            &json!({"route": "/cors-safe"}),
            &budget(),
        )
        .await;
        assert_eq!(safe.security_outcome, SecurityOutcome::Passed);

        let wildcard_credentials = execute(
            "http.cors.misconfiguration",
            environment,
            &json!({"route": "/cors-wildcard-credentials"}),
            &budget(),
        )
        .await;
        assert_eq!(
            wildcard_credentials.security_outcome,
            SecurityOutcome::Finding
        );
        assert_eq!(wildcard_credentials.severity, Some(Severity::High));
        assert!(wildcard_credentials.summary.contains("any origin"));
    }

    #[tokio::test]
    async fn observes_authentication_rate_limiting_with_a_bounded_request_budget() {
        let server = start_mock_server().await;
        let limited = execute(
            "http.auth.rate-limit",
            environment(server.port),
            &json!({"route": "/rate-limited"}),
            &budget(),
        )
        .await;
        assert_eq!(limited.execution_status, ExecutionStatus::Completed);
        assert_eq!(limited.security_outcome, SecurityOutcome::Passed);
        assert!(limited.summary.contains("throttling"));
        let attributes = &limited.evidence[0].attributes;
        assert_eq!(attributes["throttled"], true);
        assert_eq!(attributes["retryAfterPresent"], true);
        assert_eq!(attributes["maxAttempts"], RATE_LIMIT_ATTEMPTS);
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);

        let open = execute(
            "http.auth.rate-limit",
            environment(server.port),
            &json!({"route": "/rate-open"}),
            &budget(),
        )
        .await;
        assert_eq!(open.execution_status, ExecutionStatus::Completed);
        assert_eq!(open.security_outcome, SecurityOutcome::Finding);
        assert_eq!(open.severity, Some(Severity::Medium));
        assert!(open.summary.contains("No rate limiting"));
        assert_eq!(server.requests.load(Ordering::SeqCst), 6);

        let errored = execute(
            "http.auth.rate-limit",
            environment(server.port),
            &json!({"route": "/rate-error"}),
            &budget(),
        )
        .await;
        assert_eq!(errored.execution_status, ExecutionStatus::Completed);
        assert_eq!(errored.security_outcome, SecurityOutcome::Inconclusive);
        assert!(errored.severity.is_none());

        let unavailable = execute(
            "http.auth.rate-limit",
            environment(server.port),
            &json!({"route": "/rate-unavailable"}),
            &budget(),
        )
        .await;
        assert_eq!(unavailable.security_outcome, SecurityOutcome::Inconclusive);

        let service_limited = execute(
            "http.auth.rate-limit",
            environment(server.port),
            &json!({"route": "/rate-503-limited"}),
            &budget(),
        )
        .await;
        assert_eq!(service_limited.security_outcome, SecurityOutcome::Passed);
        assert_eq!(service_limited.evidence[0].attributes["throttled"], true);
    }

    #[tokio::test]
    async fn inspects_cookie_attributes_without_returning_values() {
        let server = start_mock_server().await;
        let insecure = execute(
            "http.cookies.insecure",
            environment(server.port),
            &json!({"route": "/cookies-insecure"}),
            &budget(),
        )
        .await;
        assert_eq!(insecure.execution_status, ExecutionStatus::Completed);
        assert_eq!(insecure.security_outcome, SecurityOutcome::Finding);
        assert_eq!(insecure.severity, Some(Severity::High));
        let attributes = &insecure.evidence[0].attributes;
        assert_eq!(attributes["cookiesInspected"], 2);
        assert_eq!(attributes["cookiesFlagged"], 2);
        let serialized = serde_json::to_string(&insecure).expect("serialize cookies");
        assert!(!serialized.contains("abc123"));
        assert!(!serialized.contains("dark"));
        assert!(serialized.contains("sessionid"));

        let safe = execute(
            "http.cookies.insecure",
            environment(server.port),
            &json!({"route": "/cookies-safe"}),
            &budget(),
        )
        .await;
        assert_eq!(safe.security_outcome, SecurityOutcome::Passed);

        let none = execute(
            "http.cookies.insecure",
            environment(server.port),
            &json!({"route": "/cookies-none"}),
            &budget(),
        )
        .await;
        assert_eq!(none.security_outcome, SecurityOutcome::Passed);
        assert!(none.summary.contains("did not set any cookies"));
    }

    #[tokio::test]
    async fn detects_passive_information_disclosure_without_body_retention() {
        let server = start_mock_server().await;
        let disclosed = execute(
            "http.info-disclosure",
            environment(server.port),
            &json!({"route": "/info-disclosure"}),
            &budget(),
        )
        .await;
        assert_eq!(disclosed.execution_status, ExecutionStatus::Completed);
        assert_eq!(disclosed.security_outcome, SecurityOutcome::Finding);
        assert_eq!(disclosed.severity, Some(Severity::High));
        assert!(disclosed.extensions.cwe_ids.contains(&"CWE-209".to_owned()));
        let serialized = serde_json::to_string(&disclosed).expect("serialize disclosure");
        assert!(!serialized.contains("/app/main.py"));
        assert!(!serialized.contains("Traceback"));
        assert!(serialized.contains("python_traceback"));

        let clean = execute(
            "http.info-disclosure",
            environment(server.port),
            &json!({"route": "/info-clean"}),
            &budget(),
        )
        .await;
        assert_eq!(clean.security_outcome, SecurityOutcome::Passed);
    }

    #[tokio::test]
    async fn unsupported_ids_and_invalid_configurations_fail_closed_without_requests() {
        let server = start_mock_server().await;
        let unsupported = execute(
            "static.dependencies.known-vulnerable",
            environment(server.port),
            &json!({}),
            &budget(),
        )
        .await;
        assert_eq!(unsupported.execution_status, ExecutionStatus::Unsupported);
        assert_eq!(unsupported.security_outcome, SecurityOutcome::NotRun);
        assert_eq!(
            unsupported.error_code.as_deref(),
            Some("unsupported_catalog_test")
        );
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);

        for catalog_test_id in [
            "http.headers.missing",
            "http.cors.misconfiguration",
            "http.auth.rate-limit",
            "http.cookies.insecure",
            "http.info-disclosure",
        ] {
            let missing = execute(
                catalog_test_id,
                environment(server.port),
                &json!({}),
                &budget(),
            )
            .await;
            assert_eq!(missing.execution_status, ExecutionStatus::Failed);
            assert_eq!(missing.security_outcome, SecurityOutcome::Inconclusive);
            assert_eq!(
                missing.error_code.as_deref(),
                Some("invalid_adapter_configuration")
            );
        }
        for catalog_test_id in ["network.tls.baseline", "static.secrets.exposure"] {
            let configured = execute(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/headers-present"}),
                &budget(),
            )
            .await;
            assert_eq!(configured.execution_status, ExecutionStatus::Failed);
            assert_eq!(
                configured.error_code.as_deref(),
                Some("invalid_adapter_configuration")
            );
        }
        for catalog_test_id in supported_catalog_ids() {
            let unknown_field = execute(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/headers-present", "command": "whoami"}),
                &budget(),
            )
            .await;
            assert_eq!(unknown_field.execution_status, ExecutionStatus::Failed);
            assert_eq!(
                unknown_field.error_code.as_deref(),
                Some("invalid_adapter_configuration")
            );
        }
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);

        let invalid = execute(
            "http.headers.missing",
            environment(server.port),
            &json!({"route": "/../admin"}),
            &budget(),
        )
        .await;
        assert_eq!(invalid.execution_status, ExecutionStatus::Failed);
        assert_eq!(invalid.security_outcome, SecurityOutcome::Inconclusive);
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn timeout_cancellation_and_server_errors_are_separate_from_security_outcome() {
        let server = start_mock_server().await;
        for catalog_test_id in [
            "http.headers.missing",
            "http.cors.misconfiguration",
            "http.auth.rate-limit",
            "http.cookies.insecure",
        ] {
            let slow = execute_with_timeout(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/slow"}),
                &budget(),
                Duration::from_millis(50),
            )
            .await;
            assert_eq!(slow.execution_status, ExecutionStatus::TimedOut);
            assert_eq!(slow.security_outcome, SecurityOutcome::Inconclusive);

            let cancelled_budget = budget();
            cancelled_budget.cancel();
            let cancelled = execute(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/headers-present"}),
                &cancelled_budget,
            )
            .await;
            assert_eq!(cancelled.execution_status, ExecutionStatus::Cancelled);
            assert_eq!(cancelled.security_outcome, SecurityOutcome::Inconclusive);

            let server_error = execute(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/server-error"}),
                &budget(),
            )
            .await;
            assert_eq!(server_error.execution_status, ExecutionStatus::Completed);
            assert_eq!(server_error.security_outcome, SecurityOutcome::Inconclusive);
            assert!(server_error.severity.is_none());
        }
    }

    #[tokio::test]
    async fn every_http_adapter_enforces_the_response_size_cap_before_reading_a_body() {
        let server = start_mock_server().await;
        for catalog_test_id in [
            "http.headers.missing",
            "http.cors.misconfiguration",
            "http.auth.rate-limit",
            "http.cookies.insecure",
            "http.info-disclosure",
        ] {
            let oversized = execute(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/large"}),
                &budget(),
            )
            .await;
            assert_eq!(oversized.execution_status, ExecutionStatus::Failed);
            assert_eq!(oversized.security_outcome, SecurityOutcome::Inconclusive);
            assert_eq!(
                oversized.error_code.as_deref(),
                Some("response_size_exceeded")
            );
        }
    }

    #[tokio::test]
    async fn tls_baseline_requires_https_and_enforces_host_scope_before_connecting() {
        let server = start_mock_server().await;

        let plain_http = execute(
            "network.tls.baseline",
            environment(server.port),
            &json!({}),
            &budget(),
        )
        .await;
        assert_eq!(plain_http.execution_status, ExecutionStatus::Failed);
        assert_eq!(plain_http.error_code.as_deref(), Some("tls_requires_https"));
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);

        let mut out_of_scope = https_environment(server.port);
        out_of_scope.guard.scope.allowed_ports = vec![server.port.saturating_add(1)];
        let boundary = execute("network.tls.baseline", out_of_scope, &json!({}), &budget()).await;
        assert_eq!(boundary.execution_status, ExecutionStatus::Failed);
        assert_eq!(
            boundary.error_code.as_deref(),
            Some("target_port_not_allowed")
        );

        let mut unresolved = https_environment(server.port);
        unresolved.guard.scope.origin = "https://unresolvable.invalid:443".to_owned();
        unresolved.guard.scope.allowed_hosts = vec!["unresolvable.invalid".to_owned()];
        unresolved.guard.scope.allowed_ports = vec![443];
        let dns = execute("network.tls.baseline", unresolved, &json!({}), &budget()).await;
        assert_eq!(dns.execution_status, ExecutionStatus::Failed);
        assert_eq!(dns.error_code.as_deref(), Some("dns_resolution_failed"));

        let cancelled_budget = budget();
        cancelled_budget.cancel();
        let cancelled = execute(
            "network.tls.baseline",
            https_environment(server.port),
            &json!({}),
            &cancelled_budget,
        )
        .await;
        assert_eq!(cancelled.execution_status, ExecutionStatus::Cancelled);
    }

    #[tokio::test]
    async fn tls_handshake_observes_certificate_metadata_for_a_self_signed_origin() {
        use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

        let certified =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("certificate");
        let cert_der = certified.cert.der().clone();
        let key_der = PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], PrivateKeyDer::Pkcs8(key_der))
            .expect("server config");
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("tls bind");
        let port = listener.local_addr().expect("tls addr").port();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let _ = acceptor.accept(stream).await;
                });
            }
        });

        let result = execute(
            "network.tls.baseline",
            https_environment(port),
            &json!({}),
            &budget(),
        )
        .await;
        handle.abort();

        assert_eq!(result.execution_status, ExecutionStatus::Completed);
        assert_eq!(result.security_outcome, SecurityOutcome::Finding);
        assert_eq!(result.severity, Some(Severity::Medium));
        let attributes = &result.evidence[0].attributes;
        assert!(attributes["protocol"].as_str().is_some());
        assert!(attributes["cipherSuite"].as_str().is_some());
        assert!(attributes["leafCertificateSha256"].as_str().is_some());
        assert_eq!(attributes["verification"], "failed");
        assert_eq!(
            attributes["verificationCode"],
            "tls_certificate_untrusted_issuer"
        );
        assert_eq!(attributes["selfSigned"], true);
    }

    #[test]
    fn tls_certificate_error_mapping_is_stable() {
        assert_eq!(
            tls_certificate_error(&rustls::CertificateError::Expired).0,
            "tls_certificate_expired"
        );
        assert_eq!(
            tls_certificate_error(&rustls::CertificateError::NotValidForName).0,
            "tls_certificate_hostname_mismatch"
        );
        assert_eq!(
            tls_certificate_error(&rustls::CertificateError::UnknownIssuer).1,
            Severity::Medium
        );
        assert!(certificate_metadata(b"not a certificate").is_none());
    }

    struct SnapshotFixture {
        root: std::path::PathBuf,
    }

    impl SnapshotFixture {
        fn empty() -> Self {
            let root = std::env::temp_dir().join(format!(
                "probeforge-adapter-snapshot-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_nanos())
                    .unwrap_or_default()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("snapshot root");
            Self { root }
        }

        fn with_file(self, path: &str, contents: &str) -> Self {
            let target = self.root.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).expect("snapshot parent");
            }
            std::fs::write(target, contents).expect("snapshot write");
            self
        }

        fn snapshot(&self) -> RepositorySnapshot {
            RepositorySnapshot::new(self.root.clone()).expect("snapshot handle")
        }
    }

    impl Drop for SnapshotFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn static_fixture_aws_key() -> String {
        ["AKIA", "IOSFODNN7AB12CD3"].concat()
    }

    #[tokio::test]
    async fn static_secret_scanning_uses_a_server_owned_snapshot_and_redacts_values() {
        let aws_key = static_fixture_aws_key();
        let fixture = SnapshotFixture::empty().with_file(
            "config/production.env",
            &format!("AWS_ACCESS_KEY_ID={aws_key}\n"),
        );
        let mut static_environment = environment(1);
        static_environment.snapshot = Some(fixture.snapshot());
        let result = execute(
            "static.secrets.exposure",
            static_environment,
            &json!({}),
            &budget(),
        )
        .await;
        assert_eq!(result.execution_status, ExecutionStatus::Completed);
        assert_eq!(result.security_outcome, SecurityOutcome::Finding);
        assert_eq!(result.severity, Some(Severity::High));
        assert_eq!(result.extensions.rule_id.as_deref(), Some("PF-SEC-002"));
        assert!(
            result
                .extensions
                .affected_assets
                .contains(&"config/production.env".to_owned())
        );
        let serialized = serde_json::to_string(&result).expect("serialize static result");
        assert!(!serialized.contains(&aws_key));
        assert!(serialized.contains("***"));

        let clean_fixture = SnapshotFixture::empty().with_file("src/main.rs", "fn main() {}\n");
        let mut clean_environment = environment(1);
        clean_environment.snapshot = Some(clean_fixture.snapshot());
        let clean = execute(
            "static.secrets.exposure",
            clean_environment,
            &json!({}),
            &budget(),
        )
        .await;
        assert_eq!(clean.security_outcome, SecurityOutcome::Passed);

        let missing = execute(
            "static.secrets.exposure",
            environment(1),
            &json!({}),
            &budget(),
        )
        .await;
        assert_eq!(missing.execution_status, ExecutionStatus::Failed);
        assert_eq!(
            missing.error_code.as_deref(),
            Some("repository_snapshot_unavailable")
        );

        let cancel_fixture = SnapshotFixture::empty().with_file(
            "config/production.env",
            &format!("AWS_ACCESS_KEY_ID={}\n", static_fixture_aws_key()),
        );
        let mut cancel_environment = environment(1);
        cancel_environment.snapshot = Some(cancel_fixture.snapshot());
        let cancelled_budget = budget();
        cancelled_budget.cancel();
        let cancelled = execute(
            "static.secrets.exposure",
            cancel_environment,
            &json!({}),
            &cancelled_budget,
        )
        .await;
        assert_eq!(cancelled.execution_status, ExecutionStatus::Cancelled);
        assert_eq!(cancelled.security_outcome, SecurityOutcome::Inconclusive);
    }
}
