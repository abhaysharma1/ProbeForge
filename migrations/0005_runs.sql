-- Execution-facing tables: load profiles/scenarios, schedules, runs, results,
-- evidence metadata, metrics, agent sessions, and the append-only audit log.

CREATE TABLE load_profiles (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name text NOT NULL,
    virtual_users integer,
    arrival_rate_per_second numeric(10, 2),
    ramp_up_seconds integer NOT NULL DEFAULT 0,
    duration_seconds integer NOT NULL,
    max_requests bigint NOT NULL,
    max_requests_per_second numeric(10, 2) NOT NULL,
    stop_conditions jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT load_profiles_name_length CHECK (char_length(btrim(name)) BETWEEN 1 AND 160),
    CONSTRAINT load_profiles_sizing_mode CHECK (
        (virtual_users IS NOT NULL) <> (arrival_rate_per_second IS NOT NULL)
    ),
    CONSTRAINT load_profiles_virtual_users_range CHECK (
        virtual_users IS NULL OR virtual_users BETWEEN 1 AND 500
    ),
    CONSTRAINT load_profiles_arrival_rate_range CHECK (
        arrival_rate_per_second IS NULL OR arrival_rate_per_second BETWEEN 0.1 AND 1000
    ),
    CONSTRAINT load_profiles_ramp_range CHECK (ramp_up_seconds BETWEEN 0 AND 3600),
    CONSTRAINT load_profiles_duration_range CHECK (duration_seconds BETWEEN 1 AND 3600),
    CONSTRAINT load_profiles_requests_range CHECK (max_requests BETWEEN 1 AND 1000000),
    CONSTRAINT load_profiles_rate_range CHECK (max_requests_per_second BETWEEN 0.1 AND 1000),
    CONSTRAINT load_profiles_stop_conditions_object CHECK (jsonb_typeof(stop_conditions) = 'object'),
    CONSTRAINT load_profiles_stop_conditions_size CHECK (pg_column_size(stop_conditions) <= 4096)
);

CREATE INDEX load_profiles_project_idx ON load_profiles (project_id, created_at DESC);

CREATE TABLE load_scenarios (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    plan_id uuid NOT NULL REFERENCES test_plans (id) ON DELETE CASCADE,
    profile_id uuid NOT NULL REFERENCES load_profiles (id) ON DELETE RESTRICT,
    name text NOT NULL,
    schema_version text NOT NULL DEFAULT '1',
    scenario jsonb NOT NULL,
    selected boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (plan_id, name),
    CONSTRAINT load_scenarios_name_length CHECK (char_length(btrim(name)) BETWEEN 1 AND 160),
    CONSTRAINT load_scenarios_schema_version CHECK (schema_version = '1'),
    CONSTRAINT load_scenarios_payload_object CHECK (jsonb_typeof(scenario) = 'object'),
    CONSTRAINT load_scenarios_payload_size CHECK (pg_column_size(scenario) <= 262144)
);

CREATE INDEX load_scenarios_plan_idx ON load_scenarios (plan_id);
CREATE INDEX load_scenarios_project_idx ON load_scenarios (project_id, created_at DESC);
CREATE INDEX load_scenarios_selected_idx ON load_scenarios (plan_id, selected) WHERE selected;

CREATE TABLE schedules (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    job_id uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    plan_id uuid NOT NULL REFERENCES test_plans (id) ON DELETE RESTRICT,
    load_scenario_id uuid REFERENCES load_scenarios (id) ON DELETE RESTRICT,
    load_profile_id uuid REFERENCES load_profiles (id) ON DELETE RESTRICT,
    timezone text NOT NULL DEFAULT 'UTC',
    recurrence text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    next_run_at timestamptz,
    last_run_at timestamptz,
    created_by uuid NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT schedules_timezone_length CHECK (char_length(btrim(timezone)) BETWEEN 1 AND 64),
    CONSTRAINT schedules_recurrence_length CHECK (char_length(btrim(recurrence)) BETWEEN 1 AND 200),
    CONSTRAINT schedules_load_pair CHECK (
        (load_scenario_id IS NULL) = (load_profile_id IS NULL)
    ),
    CONSTRAINT schedules_enabled_has_next_run CHECK (NOT enabled OR next_run_at IS NOT NULL)
);

CREATE TRIGGER schedules_set_updated_at
BEFORE UPDATE ON schedules
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX schedules_due_idx ON schedules (next_run_at) WHERE enabled;
CREATE INDEX schedules_project_idx ON schedules (project_id, created_at DESC);

CREATE TABLE test_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    job_id uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    plan_id uuid NOT NULL,
    plan_version integer NOT NULL,
    plan_approval_id uuid NOT NULL REFERENCES plan_approvals (id) ON DELETE RESTRICT,
    run_type text NOT NULL,
    status text NOT NULL DEFAULT 'draft',
    planned_test_id uuid REFERENCES planned_tests (id) ON DELETE RESTRICT,
    load_scenario_id uuid REFERENCES load_scenarios (id) ON DELETE RESTRICT,
    load_profile_id uuid REFERENCES load_profiles (id) ON DELETE RESTRICT,
    target_authorization_id uuid NOT NULL REFERENCES target_authorizations (id) ON DELETE RESTRICT,
    schedule_id uuid REFERENCES schedules (id) ON DELETE SET NULL,
    limits jsonb NOT NULL DEFAULT '{}'::jsonb,
    attempt integer NOT NULL DEFAULT 0,
    max_attempts integer NOT NULL DEFAULT 3,
    worker_id text,
    lease_expires_at timestamptz,
    heartbeat_at timestamptz,
    idempotency_key text NOT NULL,
    scheduled_for timestamptz,
    started_at timestamptz,
    finished_at timestamptz,
    last_error_code text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, idempotency_key),
    CONSTRAINT test_runs_plan_fkey
        FOREIGN KEY (plan_id, plan_version)
        REFERENCES test_plans (id, version)
        ON DELETE RESTRICT,
    CONSTRAINT test_runs_run_type_values CHECK (run_type IN ('vulnerability_testing', 'load_testing')),
    CONSTRAINT test_runs_status_values CHECK (
        status IN ('draft', 'awaiting_target', 'scheduled', 'queued', 'running', 'analyzing_results', 'completed', 'failed', 'cancelled', 'expired', 'completed_with_report_error')
    ),
    CONSTRAINT test_runs_target_shape CHECK (
        (run_type = 'vulnerability_testing'
            AND planned_test_id IS NOT NULL
            AND load_scenario_id IS NULL
            AND load_profile_id IS NULL)
        OR
        (run_type = 'load_testing'
            AND planned_test_id IS NULL
            AND load_scenario_id IS NOT NULL
            AND load_profile_id IS NOT NULL)
    ),
    CONSTRAINT test_runs_plan_version_positive CHECK (plan_version > 0),
    CONSTRAINT test_runs_attempt_range CHECK (attempt BETWEEN 0 AND max_attempts),
    CONSTRAINT test_runs_max_attempts_range CHECK (max_attempts BETWEEN 1 AND 20),
    CONSTRAINT test_runs_idempotency_key_length CHECK (char_length(idempotency_key) BETWEEN 1 AND 200),
    CONSTRAINT test_runs_limits_object CHECK (jsonb_typeof(limits) = 'object'),
    CONSTRAINT test_runs_limits_size CHECK (pg_column_size(limits) <= 8192),
    CONSTRAINT test_runs_lease_pair CHECK ((worker_id IS NULL) = (lease_expires_at IS NULL)),
    CONSTRAINT test_runs_heartbeat_requires_lease CHECK (
        heartbeat_at IS NULL OR lease_expires_at IS NOT NULL
    ),
    CONSTRAINT test_runs_heartbeat_before_lease CHECK (
        heartbeat_at IS NULL OR heartbeat_at <= lease_expires_at
    ),
    CONSTRAINT test_runs_lease_status CHECK (
        lease_expires_at IS NULL OR status IN ('running', 'analyzing_results')
    ),
    CONSTRAINT test_runs_time_order CHECK (
        finished_at IS NULL OR started_at IS NULL OR finished_at >= started_at
    )
);

CREATE TRIGGER test_runs_set_updated_at
BEFORE UPDATE ON test_runs
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX test_runs_queue_idx
    ON test_runs (status, scheduled_for, created_at)
    WHERE status IN ('scheduled', 'queued');

CREATE INDEX test_runs_lease_idx
    ON test_runs (lease_expires_at)
    WHERE lease_expires_at IS NOT NULL;

CREATE INDEX test_runs_project_created_idx ON test_runs (project_id, created_at DESC);
CREATE INDEX test_runs_job_idx ON test_runs (job_id, created_at DESC);
CREATE INDEX test_runs_schedule_idx ON test_runs (schedule_id) WHERE schedule_id IS NOT NULL;

CREATE TABLE test_results (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES test_runs (id) ON DELETE CASCADE,
    catalog_test_id text NOT NULL,
    result_key text NOT NULL,
    outcome text NOT NULL,
    severity text,
    confidence numeric(3, 2),
    summary text NOT NULL,
    details jsonb NOT NULL DEFAULT '{}'::jsonb,
    tool_name text,
    tool_version text,
    duration_ms integer NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (run_id, result_key),
    CONSTRAINT test_results_catalog_id_format CHECK (
        catalog_test_id ~ '^[a-z0-9][a-z0-9.-]{1,118}$'
    ),
    CONSTRAINT test_results_result_key_length CHECK (char_length(result_key) BETWEEN 1 AND 200),
    CONSTRAINT test_results_outcome_values CHECK (
        outcome IN ('passed', 'failed', 'skipped', 'inconclusive', 'error', 'cancelled')
    ),
    CONSTRAINT test_results_severity_values CHECK (
        severity IS NULL OR severity IN ('info', 'low', 'medium', 'high', 'critical')
    ),
    CONSTRAINT test_results_failed_severity CHECK (outcome <> 'failed' OR severity IS NOT NULL),
    CONSTRAINT test_results_confidence_range CHECK (
        confidence IS NULL OR confidence BETWEEN 0 AND 1
    ),
    CONSTRAINT test_results_summary_length CHECK (char_length(btrim(summary)) BETWEEN 1 AND 8000),
    CONSTRAINT test_results_details_object CHECK (jsonb_typeof(details) = 'object'),
    CONSTRAINT test_results_details_size CHECK (pg_column_size(details) <= 131072),
    CONSTRAINT test_results_duration_range CHECK (duration_ms BETWEEN 0 AND 3600000)
);

CREATE INDEX test_results_run_idx ON test_results (run_id, created_at);
CREATE INDEX test_results_project_idx ON test_results (project_id, created_at DESC);

CREATE TABLE evidence (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    result_id uuid NOT NULL REFERENCES test_results (id) ON DELETE CASCADE,
    storage_ref text NOT NULL,
    sha256 text NOT NULL,
    mime_type text NOT NULL,
    size_bytes bigint NOT NULL,
    redacted boolean NOT NULL DEFAULT true,
    retention_until timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (result_id, storage_ref),
    CONSTRAINT evidence_storage_ref_length CHECK (char_length(storage_ref) BETWEEN 1 AND 1024),
    CONSTRAINT evidence_sha256_format CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT evidence_mime_type_length CHECK (char_length(mime_type) BETWEEN 1 AND 120),
    CONSTRAINT evidence_size_range CHECK (size_bytes BETWEEN 0 AND 10485760)
);

CREATE INDEX evidence_result_idx ON evidence (result_id);
CREATE INDEX evidence_project_idx ON evidence (project_id, created_at DESC);

CREATE TRIGGER evidence_immutable
BEFORE UPDATE OR DELETE ON evidence
FOR EACH ROW EXECUTE FUNCTION reject_immutable_row();

CREATE TABLE run_metrics (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES test_runs (id) ON DELETE CASCADE,
    bucket_start timestamptz NOT NULL,
    bucket_seconds integer NOT NULL,
    endpoint text NOT NULL DEFAULT '',
    status_code integer NOT NULL DEFAULT 0,
    request_count bigint NOT NULL DEFAULT 0,
    error_count bigint NOT NULL DEFAULT 0,
    latency_p50_ms numeric(10, 2),
    latency_p95_ms numeric(10, 2),
    latency_p99_ms numeric(10, 2),
    throughput_rps numeric(10, 2),
    response_bytes bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (run_id, bucket_start, endpoint, status_code),
    CONSTRAINT run_metrics_bucket_range CHECK (bucket_seconds BETWEEN 1 AND 3600),
    CONSTRAINT run_metrics_status_code CHECK (status_code = 0 OR status_code BETWEEN 100 AND 599),
    CONSTRAINT run_metrics_counts CHECK (request_count >= 0 AND error_count >= 0 AND error_count <= request_count),
    CONSTRAINT run_metrics_latency_nonnegative CHECK (
        (latency_p50_ms IS NULL OR latency_p50_ms >= 0)
        AND (latency_p95_ms IS NULL OR latency_p95_ms >= 0)
        AND (latency_p99_ms IS NULL OR latency_p99_ms >= 0)
    ),
    CONSTRAINT run_metrics_throughput_nonnegative CHECK (throughput_rps IS NULL OR throughput_rps >= 0),
    CONSTRAINT run_metrics_response_bytes_nonnegative CHECK (response_bytes >= 0)
);

CREATE INDEX run_metrics_run_time_idx ON run_metrics (run_id, bucket_start);
CREATE INDEX run_metrics_project_time_idx ON run_metrics (project_id, bucket_start DESC);

CREATE TABLE agent_sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    job_id uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    provider text NOT NULL,
    model text NOT NULL,
    status text NOT NULL,
    trace_metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    started_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT agent_sessions_provider_length CHECK (char_length(btrim(provider)) BETWEEN 1 AND 120),
    CONSTRAINT agent_sessions_model_length CHECK (char_length(btrim(model)) BETWEEN 1 AND 160),
    CONSTRAINT agent_sessions_status_values CHECK (
        status IN ('running', 'completed', 'failed', 'cancelled')
    ),
    CONSTRAINT agent_sessions_trace_object CHECK (jsonb_typeof(trace_metadata) = 'object'),
    CONSTRAINT agent_sessions_trace_size CHECK (pg_column_size(trace_metadata) <= 32768),
    CONSTRAINT agent_sessions_time_order CHECK (finished_at IS NULL OR finished_at >= started_at)
);

CREATE INDEX agent_sessions_job_idx ON agent_sessions (job_id, started_at DESC);
CREATE INDEX agent_sessions_project_idx ON agent_sessions (project_id, started_at DESC);

CREATE TABLE audit_log (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid REFERENCES projects (id) ON DELETE SET NULL,
    actor_id uuid REFERENCES users (id) ON DELETE SET NULL,
    action text NOT NULL,
    resource_type text NOT NULL,
    resource_id uuid,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT audit_log_action_length CHECK (char_length(btrim(action)) BETWEEN 1 AND 120),
    CONSTRAINT audit_log_resource_type_length CHECK (char_length(btrim(resource_type)) BETWEEN 1 AND 80),
    CONSTRAINT audit_log_metadata_object CHECK (jsonb_typeof(metadata) = 'object'),
    CONSTRAINT audit_log_metadata_size CHECK (pg_column_size(metadata) <= 32768)
);

CREATE INDEX audit_log_project_created_idx ON audit_log (project_id, created_at DESC);
CREATE INDEX audit_log_actor_created_idx ON audit_log (actor_id, created_at DESC);

CREATE TRIGGER audit_log_immutable
BEFORE UPDATE OR DELETE ON audit_log
FOR EACH ROW EXECUTE FUNCTION reject_immutable_row();
