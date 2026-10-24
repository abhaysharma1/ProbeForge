//! Constrained load runner. It accepts only a validated, data-only scenario and
//! an already-approved target scope. Every request goes through
//! `GuardedHttpClient`; no model text becomes executable code.

mod executor;
mod metrics;
mod policy;

use std::collections::BTreeMap;

use probe_forge_target_policy::{
    GuardedHttpClient, SafeMethod, TargetGuard, TargetPolicy, TargetRequestError,
    validate_load_scenario_scope,
};
use probeforge_contracts::{
    Environment, HttpMethod, LoadScenario, TargetLimits, TargetScope, validate_target_limits,
};
use serde_json::Value;
use url::Url;

pub use metrics::{
    MetricBucket, RequestSample, RunMetrics, RunSummary, aggregate_metrics, persist_metrics,
};
pub use policy::{EffectiveLimits, LoadCaps, resolve_limits};
pub use probe_forge_target_policy::{MAX_REQUEST_BODY_BYTES, MAX_RESPONSE_BYTES, MAX_TOTAL_BYTES};

#[derive(Debug, thiserror::Error)]
pub enum LoadRunError {
    #[error("load scenario is invalid")]
    InvalidScenario,
    #[error("load limits are invalid")]
    InvalidLimits,
    #[error("load scenario is outside the approved target scope ({0})")]
    ScopeViolation(&'static str),
    #[error("secret reference is unavailable")]
    SecretUnavailable,
    #[error("secret reference could not be resolved")]
    SecretResolution,
    #[error("request template contains an unknown variable or unsupported expression")]
    InvalidTemplate,
    #[error("request body exceeds the platform cap")]
    RequestBodyTooLarge,
    #[error("load run configuration is invalid")]
    InvalidConfiguration,
    #[error(transparent)]
    Target(#[from] TargetRequestError),
    #[error("metrics are invalid")]
    MetricsInvalid,
    #[error("metrics exceed the persistence cap")]
    MetricsTooLarge,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Completed,
    Cancelled,
    TimedOut,
    Failed,
}

#[derive(Debug, Clone)]
pub struct LoadRunOutcome {
    pub status: RunStatus,
    pub metrics: RunMetrics,
}

#[derive(Debug, Clone)]
pub struct RunnerStep {
    pub id: String,
    pub method: SafeMethod,
    pub path_template: String,
    pub headers: BTreeMap<String, String>,
    pub secret_headers: BTreeMap<String, String>,
    pub depends_on: Vec<String>,
    pub extract: Vec<probeforge_contracts::LoadExtraction>,
    pub assertions: Vec<probeforge_contracts::LoadAssertion>,
    pub body_template: Option<Value>,
    pub expected_statuses: Vec<u16>,
}

#[derive(Debug, Clone)]
pub struct RunnerSpec {
    pub scenario_name: String,
    pub origin: Url,
    pub guard: TargetGuard,
    pub policy: TargetPolicy,
    pub steps: Vec<RunnerStep>,
    pub limits: EffectiveLimits,
}

pub trait SecretResolver: Send + Sync {
    fn resolve(&self, reference: &str) -> Result<String, LoadRunError>;
}

#[derive(Debug, Default)]
pub struct NoSecretResolver;

impl SecretResolver for NoSecretResolver {
    fn resolve(&self, _reference: &str) -> Result<String, LoadRunError> {
        Err(LoadRunError::SecretUnavailable)
    }
}

#[derive(Debug, Default)]
pub struct StaticSecretResolver {
    values: BTreeMap<String, String>,
}

impl StaticSecretResolver {
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            values: pairs.into_iter().collect(),
        }
    }
}

impl SecretResolver for StaticSecretResolver {
    fn resolve(&self, reference: &str) -> Result<String, LoadRunError> {
        self.values
            .get(reference)
            .cloned()
            .ok_or(LoadRunError::SecretResolution)
    }
}

pub fn build_runner_spec(
    scenario: &LoadScenario,
    environment: Environment,
    scope: &TargetScope,
    target_limits: &TargetLimits,
    policy: &TargetPolicy,
    caps: LoadCaps,
) -> Result<RunnerSpec, LoadRunError> {
    validate_target_limits(target_limits).map_err(|_| LoadRunError::InvalidLimits)?;
    let limits = resolve_limits(&scenario.profile, target_limits, caps)?;
    let origin = Url::parse(&scope.origin).map_err(|_| LoadRunError::InvalidScenario)?;

    validate_load_scenario_scope(scenario, environment, scope, policy).map_err(|issues| {
        let code = issues
            .first()
            .map_or("target_scope_violation", |issue| issue.code);
        tracing::warn!(
            event = "load.scope_violation",
            scenario = %scenario.name,
            code,
            "load scenario is outside the approved target scope"
        );
        LoadRunError::ScopeViolation(code)
    })?;

    let steps = scenario
        .requests
        .iter()
        .enumerate()
        .map(|(index, request)| {
            Ok(RunnerStep {
                id: request
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("step-{index}")),
                method: safe_method(request.method)?,
                path_template: request.path.clone(),
                headers: request.headers.clone(),
                secret_headers: request.secret_headers.clone(),
                depends_on: request.depends_on.clone(),
                extract: request.extract.clone(),
                assertions: request.assertions.clone(),
                body_template: request.body_template.clone(),
                expected_statuses: request.expected_statuses.clone(),
            })
        })
        .collect::<Result<Vec<_>, LoadRunError>>()?;
    if steps.is_empty() {
        return Err(LoadRunError::InvalidScenario);
    }

    tracing::debug!(
        event = "load.spec_built",
        scenario = %scenario.name,
        steps = steps.len(),
        max_requests = limits.max_requests,
        "built a validated load runner spec"
    );
    Ok(RunnerSpec {
        scenario_name: scenario.name.clone(),
        origin,
        guard: TargetGuard {
            environment,
            scope: scope.clone(),
        },
        policy: policy.clone(),
        steps,
        limits,
    })
}

fn safe_method(method: HttpMethod) -> Result<SafeMethod, LoadRunError> {
    match method {
        HttpMethod::Get => Ok(SafeMethod::Get),
        HttpMethod::Head => Ok(SafeMethod::Head),
        HttpMethod::Post => Ok(SafeMethod::Post),
        HttpMethod::Put => Ok(SafeMethod::Put),
        HttpMethod::Patch => Ok(SafeMethod::Patch),
        HttpMethod::Delete => Ok(SafeMethod::Delete),
        HttpMethod::Options => Ok(SafeMethod::Options),
    }
}

impl RunnerSpec {
    pub fn client(&self) -> GuardedHttpClient {
        GuardedHttpClient::new(
            self.policy.clone(),
            std::sync::Arc::new(probe_forge_target_policy::SystemResolver),
        )
    }
}

pub use executor::{budget_for, run_load};
