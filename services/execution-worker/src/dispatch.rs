//! Run dispatch: claim a queued run and execute it against an authorized target.
//!
//! This is the only place the execution worker sends network traffic. Every
//! request goes through the shared `GuardedHttpClient`, so target scope,
//! redirect, DNS, and budget policy are enforced below the dispatcher. Results
//! and metrics are persisted before the run is completed, and a transient
//! failure is retried with backoff via the queue's fenced transitions.

use std::sync::Arc;

use probe_forge_execution_adapters::{
    AdapterEnvironment, AdapterExtensions, AdapterResult, ExecutionStatus, RepositorySnapshot,
    SecurityOutcome, execute, metadata_for, requires_repository_snapshot,
};
use probe_forge_load_runner::{
    LoadCaps, LoadRunError, NoSecretResolver, RunStatus, budget_for, build_runner_spec,
    persist_metrics, run_load,
};
use probe_forge_target_policy::{RunBudget, SystemResolver, TargetGuard, TargetPolicy};
use probeforge_contracts::{Environment, LoadScenario, Severity, TargetLimits, TargetScope};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use probe_forge_queue::{
    ClaimedRun, QueueError, append_run_and_job_event, claim_due_run_fair, complete_run, fail_run,
};

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Queue(#[from] QueueError),
    #[error("run configuration is invalid ({0})")]
    Invalid(&'static str),
    #[error("load execution failed")]
    Load(#[from] LoadRunError),
}

#[derive(Debug, Clone)]
pub struct DispatchConfig {
    pub worker_id: String,
    pub lease_seconds: i64,
    pub max_active_global: i64,
    pub max_active_per_project: i64,
    pub allow_private_targets: bool,
    pub caps: LoadCaps,
}

impl DispatchConfig {
    pub fn new(worker_id: impl Into<String>) -> Self {
        let caps = LoadCaps::default();
        Self {
            worker_id: worker_id.into(),
            lease_seconds: 300,
            max_active_global: caps.max_active_global,
            max_active_per_project: caps.max_active_per_project,
            allow_private_targets: false,
            caps,
        }
    }

    fn policy(&self) -> TargetPolicy {
        TargetPolicy {
            allow_private_networks: self.allow_private_targets,
            ..TargetPolicy::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct DispatchedRun {
    pub run_id: Uuid,
    pub run_type: String,
    pub status: &'static str,
}

#[derive(Debug, FromRow)]
struct RunRow {
    job_id: Uuid,
    run_type: String,
    project_id: Uuid,
    planned_test_id: Option<Uuid>,
    load_scenario_id: Option<Uuid>,
    target_authorization_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
struct AuthorizationRow {
    environment: String,
    scope: Value,
    limits: Value,
}

/// Claim and execute at most one due run. Returns `Ok(None)` when nothing is
/// eligible. The run claim commits before any network work, and the run is
/// completed or failed with the worker's lease as the fence.
pub async fn dispatch_once(
    pool: &PgPool,
    config: &DispatchConfig,
) -> Result<Option<DispatchedRun>, DispatchError> {
    let Some(claimed) = claim_due_run_fair(
        pool,
        &config.worker_id,
        config.lease_seconds,
        config.max_active_global,
        config.max_active_per_project,
    )
    .await?
    else {
        return Ok(None);
    };

    let result = execute_claimed(pool, config, &claimed).await;
    let status = match &result {
        Ok(status) => *status,
        Err(_) => "failed",
    };
    Ok(Some(DispatchedRun {
        run_id: claimed.id,
        run_type: claimed.run_type.clone(),
        status,
    }))
}

async fn execute_claimed(
    pool: &PgPool,
    config: &DispatchConfig,
    claimed: &ClaimedRun,
) -> Result<&'static str, DispatchError> {
    let run: RunRow = sqlx::query_as(
        "SELECT job_id, run_type, project_id, planned_test_id, load_scenario_id, target_authorization_id
         FROM test_runs WHERE id = $1",
    )
    .bind(claimed.id)
    .fetch_optional(pool)
    .await?
    .ok_or(DispatchError::Invalid("run disappeared after claim"))?;

    let authorization_id = run
        .target_authorization_id
        .ok_or(DispatchError::Invalid("run has no target authorization"))?;
    let (environment, scope, limits) =
        load_authorization(pool, run.project_id, authorization_id).await?;
    let policy = config.policy();

    let result = match run.run_type.as_str() {
        "vulnerability_testing" => {
            let planned_test_id = run.planned_test_id.ok_or(DispatchError::Invalid(
                "vulnerability run has no planned test",
            ))?;
            execute_vulnerability(
                pool,
                claimed.id,
                run.job_id,
                run.project_id,
                planned_test_id,
                environment,
                scope,
                limits,
                policy,
            )
            .await
        }
        "load_testing" => {
            let scenario_id = run
                .load_scenario_id
                .ok_or(DispatchError::Invalid("load run has no scenario"))?;
            execute_load(
                pool,
                config,
                claimed.id,
                run.project_id,
                scenario_id,
                environment,
                scope,
                limits,
                policy,
            )
            .await
        }
        _ => Err(DispatchError::Invalid("run type is not dispatchable")),
    };

    match result {
        Ok(status) => {
            let completed = complete_run(pool, claimed.id, &config.worker_id).await?;
            if !completed {
                tracing::warn!(run_id = %claimed.id, "run completion lost to a newer lease");
                return Err(DispatchError::Invalid("run lease was lost"));
            }
            tracing::info!(run_id = %claimed.id, run_type = %run.run_type, status, "run completed");
            Ok(status)
        }
        Err(error) => {
            let retryable = matches!(error, DispatchError::Load(LoadRunError::Target(_)));
            let code = error_code(&error);
            let outcome = fail_run(pool, claimed.id, &config.worker_id, code, retryable).await?;
            tracing::warn!(
                run_id = %claimed.id,
                error_code = code,
                retryable,
                ?outcome,
                "run failed"
            );
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_vulnerability(
    pool: &PgPool,
    run_id: Uuid,
    job_id: Uuid,
    project_id: Uuid,
    planned_test_id: Uuid,
    environment: Environment,
    scope: TargetScope,
    limits: TargetLimits,
    policy: TargetPolicy,
) -> Result<&'static str, DispatchError> {
    let (catalog_test_id, configuration): (String, Value) =
        sqlx::query_as("SELECT catalog_test_id, configuration FROM planned_tests WHERE id = $1")
            .bind(planned_test_id)
            .fetch_optional(pool)
            .await?
            .ok_or(DispatchError::Invalid("planned test is missing"))?;

    // Announce the single test this run executes so a live Run section can show
    // in-flight work between `run.started` and the eventual result.
    emit_test_started(pool, run_id, &catalog_test_id).await?;

    // Static adapters scan the job's pinned commit. The worker materializes the
    // snapshot itself; a plan or model can never supply a filesystem path.
    let snapshot_guard = if requires_repository_snapshot(&catalog_test_id) {
        match crate::snapshot::materialize_pinned_snapshot(pool, project_id, job_id).await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!(
                    run_id = %run_id,
                    catalog_test_id = %catalog_test_id,
                    error_code = error.code(),
                    "repository snapshot could not be materialized for a static adapter"
                );
                let result = snapshot_failure_result(&catalog_test_id, &error);
                persist_result(pool, project_id, run_id, &result).await?;
                return Ok("completed");
            }
        }
    } else {
        None
    };
    let snapshot = match snapshot_guard.as_ref() {
        Some(snapshot) => match RepositorySnapshot::new(snapshot.root().to_path_buf()) {
            Ok(snapshot) => Some(snapshot),
            Err(_) => {
                return Err(DispatchError::Invalid(
                    "repository_snapshot_workspace_invalid",
                ));
            }
        },
        None => None,
    };

    let adapter_environment = AdapterEnvironment {
        guard: TargetGuard { environment, scope },
        policy: policy.clone(),
        resolver: Arc::new(SystemResolver),
        snapshot,
    };
    let budget = RunBudget::new(limits, &policy);

    let outcome = execute(
        &catalog_test_id,
        adapter_environment,
        &configuration,
        &budget,
    )
    .await;

    persist_result(pool, project_id, run_id, &outcome).await?;
    Ok(match outcome.execution_status {
        ExecutionStatus::Unsupported => "skipped",
        ExecutionStatus::Cancelled => "cancelled",
        _ => "completed",
    })
}

fn snapshot_failure_result(
    catalog_test_id: &str,
    error: &crate::snapshot::SnapshotError,
) -> AdapterResult {
    let metadata = metadata_for(catalog_test_id);
    AdapterResult {
        catalog_test_id: catalog_test_id.to_owned(),
        adapter_name: metadata
            .as_ref()
            .map(|metadata| metadata.adapter_name.to_owned())
            .unwrap_or_else(|| "repository_snapshot".to_owned()),
        adapter_version: metadata
            .as_ref()
            .map(|metadata| metadata.adapter_version.to_owned())
            .unwrap_or_else(|| "0.1.0".to_owned()),
        execution_status: ExecutionStatus::Failed,
        security_outcome: SecurityOutcome::Inconclusive,
        severity: None,
        confidence: 0.0,
        summary: error.safe_message().to_owned(),
        duration_ms: 0,
        evidence: Vec::new(),
        error_code: Some(error.code().to_owned()),
        extensions: AdapterExtensions::default(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_load(
    pool: &PgPool,
    config: &DispatchConfig,
    run_id: Uuid,
    project_id: Uuid,
    scenario_id: Uuid,
    environment: Environment,
    scope: TargetScope,
    limits: TargetLimits,
    policy: TargetPolicy,
) -> Result<&'static str, DispatchError> {
    let scenario_json: Value =
        sqlx::query_scalar("SELECT scenario FROM load_scenarios WHERE id = $1")
            .bind(scenario_id)
            .fetch_optional(pool)
            .await?
            .ok_or(DispatchError::Invalid("load scenario is missing"))?;
    let scenario: LoadScenario = serde_json::from_value(scenario_json)
        .map_err(|_| DispatchError::Invalid("stored load scenario is not a valid contract"))?;

    let spec = build_runner_spec(
        &scenario,
        environment,
        &scope,
        &limits,
        &policy,
        config.caps,
    )?;
    let budget = budget_for(&spec);
    let outcome = run_load(&spec, Arc::new(NoSecretResolver), &budget).await?;
    persist_metrics(pool, project_id, run_id, &outcome.metrics).await?;

    Ok(match outcome.status {
        RunStatus::Completed => "completed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::TimedOut => "timed_out",
        RunStatus::Failed => "failed",
    })
}

async fn load_authorization(
    pool: &PgPool,
    project_id: Uuid,
    authorization_id: Uuid,
) -> Result<(Environment, TargetScope, TargetLimits), DispatchError> {
    let row: AuthorizationRow = sqlx::query_as(
        "SELECT t.environment, a.scope, a.limits
         FROM target_authorizations a
         JOIN targets t ON t.id = a.target_id
         WHERE a.id = $1 AND a.project_id = $2
           AND a.revoked_at IS NULL
           AND a.valid_from <= now()
           AND (a.expires_at IS NULL OR a.expires_at > now())",
    )
    .bind(authorization_id)
    .bind(project_id)
    .fetch_optional(pool)
    .await?
    .ok_or(DispatchError::Invalid(
        "target authorization is missing or inactive",
    ))?;

    let environment = environment_from_db(&row.environment)
        .ok_or(DispatchError::Invalid("target environment is unknown"))?;
    let scope: TargetScope = serde_json::from_value(row.scope)
        .map_err(|_| DispatchError::Invalid("authorization scope is invalid"))?;
    let limits: TargetLimits = serde_json::from_value(row.limits)
        .map_err(|_| DispatchError::Invalid("authorization limits are invalid"))?;
    Ok((environment, scope, limits))
}

async fn persist_result(
    pool: &PgPool,
    project_id: Uuid,
    run_id: Uuid,
    result: &AdapterResult,
) -> Result<(), DispatchError> {
    let job_id: Option<Uuid> = sqlx::query_scalar("SELECT job_id FROM test_runs WHERE id = $1")
        .bind(run_id)
        .fetch_optional(pool)
        .await?;
    let Some(job_id) = job_id else {
        return Err(DispatchError::Invalid("run disappeared before persisting"));
    };

    let (outcome, finding_status, severity) = classify(result);
    let details = redact(serde_json::to_value(result).unwrap_or(Value::Null));
    let observation = redact(json!({
        "summary": result.summary,
        "errorCode": result.error_code,
    }));
    let confidence = (result.confidence.clamp(0.0, 1.0) * 100.0).round() / 100.0;
    let duration_ms = i32::try_from(result.duration_ms.min(3_600_000)).unwrap_or(3_600_000);

    // The result rows and their stream events commit together: a reader either
    // sees the result with its completion event or neither.
    let mut transaction = pool.begin().await?;

    let result_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO test_results (
            project_id, run_id, catalog_test_id, result_key, outcome, finding_status,
            severity, confidence, summary, details, observation, tool_name,
            tool_version, duration_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        ON CONFLICT (run_id, result_key) DO UPDATE SET
            outcome = EXCLUDED.outcome,
            finding_status = EXCLUDED.finding_status,
            severity = EXCLUDED.severity,
            confidence = EXCLUDED.confidence,
            summary = EXCLUDED.summary,
            details = EXCLUDED.details,
            observation = EXCLUDED.observation,
            tool_name = EXCLUDED.tool_name,
            tool_version = EXCLUDED.tool_version,
            duration_ms = EXCLUDED.duration_ms
        RETURNING id
        "#,
    )
    .bind(project_id)
    .bind(run_id)
    .bind(&result.catalog_test_id)
    .bind(&result.catalog_test_id)
    .bind(outcome)
    .bind(finding_status)
    .bind(severity)
    .bind(confidence)
    .bind(truncate(&result.summary, 8000))
    .bind(&details)
    .bind(&observation)
    .bind(&result.adapter_name)
    .bind(&result.adapter_version)
    .bind(duration_ms)
    .fetch_one(&mut *transaction)
    .await?;

    for (index, evidence) in result.evidence.iter().enumerate() {
        let serialized = serde_json::to_vec(evidence).unwrap_or_default();
        let hash = sha256_hex(&serialized);
        sqlx::query(
            "INSERT INTO evidence (
                project_id, result_id, storage_ref, sha256, mime_type, size_bytes, redacted
             ) VALUES ($1, $2, $3, $4, 'application/json', $5, true)
             ON CONFLICT (result_id, storage_ref) DO NOTHING",
        )
        .bind(project_id)
        .bind(result_id)
        .bind(format!("evidence/{run_id}/{index}/{hash}"))
        .bind(&hash)
        .bind(i64::try_from(serialized.len()).unwrap_or(0))
        .execute(&mut *transaction)
        .await?;
    }

    // Confirmations and suspicions are hypotheses until reviewed; the event is
    // navigational evidence, never proof, and its payload carries nothing
    // sensitive (counts and classification only).
    append_run_and_job_event(
        &mut transaction,
        run_id,
        job_id,
        "test.completed",
        json!({
            "runId": run_id,
            "resultId": result_id,
            "catalogTestId": result.catalog_test_id,
            "outcome": outcome,
        }),
    )
    .await?;
    if matches!(finding_status, Some("confirmed" | "suspected")) {
        append_run_and_job_event(
            &mut transaction,
            run_id,
            job_id,
            "finding.recorded",
            json!({
                "runId": run_id,
                "resultId": result_id,
                "findingStatus": finding_status,
                "severity": severity,
            }),
        )
        .await?;
    }

    transaction.commit().await?;
    Ok(())
}

/// Emit `test.started` for the one planned test this run executes. Kept in a
/// standalone transaction because execution itself must never hold the run
/// row's lock; the lease already fences concurrent claims.
async fn emit_test_started(
    pool: &PgPool,
    run_id: Uuid,
    catalog_test_id: &str,
) -> Result<(), DispatchError> {
    let job_id: Option<Uuid> = sqlx::query_scalar("SELECT job_id FROM test_runs WHERE id = $1")
        .bind(run_id)
        .fetch_optional(pool)
        .await?;
    let Some(job_id) = job_id else {
        return Err(DispatchError::Invalid("run disappeared before execution"));
    };

    let mut transaction = pool.begin().await?;
    append_run_and_job_event(
        &mut transaction,
        run_id,
        job_id,
        "test.started",
        json!({
            "runId": run_id,
            "catalogTestId": catalog_test_id,
        }),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

fn classify(result: &AdapterResult) -> (&'static str, Option<&'static str>, Option<&'static str>) {
    match result.execution_status {
        ExecutionStatus::Unsupported => ("skipped", None, None),
        ExecutionStatus::Cancelled => ("cancelled", None, None),
        ExecutionStatus::TimedOut | ExecutionStatus::Failed => ("error", None, None),
        ExecutionStatus::Completed => match result.security_outcome {
            SecurityOutcome::Passed => ("passed", None, None),
            SecurityOutcome::Finding => (
                "failed",
                Some(if result.confidence >= 0.8 {
                    "confirmed"
                } else {
                    "suspected"
                }),
                Some(severity_str(result.severity.unwrap_or(Severity::Low))),
            ),
            SecurityOutcome::Inconclusive => ("inconclusive", None, None),
            SecurityOutcome::NotRun => ("skipped", None, None),
        },
    }
}

fn severity_str(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Low => "low",
        Severity::Medium => "medium",
        Severity::High => "high",
        Severity::Critical => "critical",
    }
}

fn environment_from_db(value: &str) -> Option<Environment> {
    match value {
        "local" => Some(Environment::Local),
        "staging" => Some(Environment::Staging),
        "production" => Some(Environment::Production),
        _ => None,
    }
}

fn error_code(error: &DispatchError) -> &'static str {
    match error {
        DispatchError::Database(_) => "dispatch_database_error",
        DispatchError::Queue(_) => "dispatch_queue_error",
        DispatchError::Invalid(reason) => reason,
        DispatchError::Load(LoadRunError::ScopeViolation(_)) => "target_scope_violation",
        DispatchError::Load(LoadRunError::InvalidScenario) => "invalid_load_scenario",
        DispatchError::Load(LoadRunError::InvalidLimits) => "invalid_load_limits",
        DispatchError::Load(LoadRunError::Target(_)) => "target_request_failed",
        DispatchError::Load(_) => "load_run_failed",
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        value.to_owned()
    } else {
        value.chars().take(max).collect()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn redact(value: Value) -> Value {
    probe_forge_observability::redact_json(value)
}
