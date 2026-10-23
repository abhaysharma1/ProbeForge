-- Repository sources, jobs, and ordered job events.

CREATE TABLE git_sources (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    provider text NOT NULL,
    normalized_url text NOT NULL,
    default_reference text NOT NULL,
    credential_ref text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, normalized_url),
    CONSTRAINT git_sources_provider_values CHECK (
        provider IN ('github', 'gitlab', 'bitbucket', 'generic')
    ),
    CONSTRAINT git_sources_url_length CHECK (char_length(normalized_url) BETWEEN 8 AND 2048),
    CONSTRAINT git_sources_reference_length CHECK (
        char_length(btrim(default_reference)) BETWEEN 1 AND 200
    ),
    CONSTRAINT git_sources_credential_ref_format CHECK (
        credential_ref IS NULL
        OR credential_ref ~ '^secret://[a-z0-9][a-z0-9/_.-]{0,190}$'
    )
);

CREATE TRIGGER git_sources_set_updated_at
BEFORE UPDATE ON git_sources
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX git_sources_project_idx ON git_sources (project_id, created_at DESC);

CREATE TABLE repository_snapshots (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    git_source_id uuid NOT NULL REFERENCES git_sources (id) ON DELETE CASCADE,
    commit_sha text NOT NULL,
    detected_languages text[] NOT NULL DEFAULT '{}',
    frameworks text[] NOT NULL DEFAULT '{}',
    manifest_inventory jsonb NOT NULL DEFAULT '{}'::jsonb,
    analysis_metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (git_source_id, commit_sha),
    CONSTRAINT repository_snapshots_commit_sha_format CHECK (
        commit_sha ~ '^[0-9a-f]{40}([0-9a-f]{24})?$'
    ),
    CONSTRAINT repository_snapshots_manifest_size CHECK (pg_column_size(manifest_inventory) <= 65536),
    CONSTRAINT repository_snapshots_metadata_size CHECK (pg_column_size(analysis_metadata) <= 65536)
);

CREATE INDEX repository_snapshots_project_idx
    ON repository_snapshots (project_id, created_at DESC);

CREATE TABLE jobs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    created_by uuid NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    goal text NOT NULL,
    intensity text NOT NULL,
    status text NOT NULL DEFAULT 'draft',
    priority smallint NOT NULL DEFAULT 0,
    display_name text NOT NULL,
    git_source_id uuid NOT NULL REFERENCES git_sources (id) ON DELETE RESTRICT,
    source_reference text NOT NULL,
    current_commit_sha text,
    current_plan_id uuid,
    idempotency_key text,
    not_before timestamptz NOT NULL DEFAULT now(),
    attempt integer NOT NULL DEFAULT 0,
    max_attempts integer NOT NULL DEFAULT 3,
    worker_id text,
    lease_expires_at timestamptz,
    heartbeat_at timestamptz,
    last_error_code text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT jobs_goal_values CHECK (goal IN ('vulnerability_testing', 'load_testing')),
    CONSTRAINT jobs_intensity_values CHECK (intensity IN ('low', 'standard', 'high')),
    CONSTRAINT jobs_status_values CHECK (
        status IN ('draft', 'queued', 'analyzing', 'planning', 'awaiting_approval', 'failed', 'cancelled', 'expired')
    ),
    CONSTRAINT jobs_priority_range CHECK (priority BETWEEN -100 AND 100),
    CONSTRAINT jobs_display_name_length CHECK (char_length(btrim(display_name)) BETWEEN 1 AND 120),
    CONSTRAINT jobs_source_reference_safe CHECK (
        source_reference ~ '^[A-Za-z0-9._/-]{1,200}$'
        AND source_reference !~ '^-'
        AND position('..' IN source_reference) = 0
    ),
    CONSTRAINT jobs_commit_sha_format CHECK (
        current_commit_sha IS NULL
        OR current_commit_sha ~ '^[0-9a-f]{40}([0-9a-f]{24})?$'
    ),
    CONSTRAINT jobs_idempotency_key_length CHECK (
        idempotency_key IS NULL OR char_length(idempotency_key) BETWEEN 1 AND 200
    ),
    CONSTRAINT jobs_attempt_range CHECK (attempt BETWEEN 0 AND max_attempts),
    CONSTRAINT jobs_max_attempts_range CHECK (max_attempts BETWEEN 1 AND 20),
    CONSTRAINT jobs_lease_pair CHECK ((worker_id IS NULL) = (lease_expires_at IS NULL)),
    CONSTRAINT jobs_heartbeat_requires_lease CHECK (
        heartbeat_at IS NULL OR lease_expires_at IS NOT NULL
    ),
    CONSTRAINT jobs_heartbeat_before_lease CHECK (
        heartbeat_at IS NULL OR heartbeat_at <= lease_expires_at
    ),
    CONSTRAINT jobs_lease_status CHECK (
        lease_expires_at IS NULL OR status IN ('analyzing', 'planning')
    )
);

CREATE TRIGGER jobs_set_updated_at
BEFORE UPDATE ON jobs
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE UNIQUE INDEX jobs_idempotency_key_key
    ON jobs (project_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX jobs_queue_idx
    ON jobs (status, priority DESC, not_before, created_at)
    WHERE status = 'queued';

CREATE INDEX jobs_lease_idx
    ON jobs (lease_expires_at)
    WHERE lease_expires_at IS NOT NULL;

CREATE INDEX jobs_project_created_idx ON jobs (project_id, created_at DESC);
CREATE INDEX jobs_project_status_idx ON jobs (project_id, status, created_at DESC);

CREATE TABLE job_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    sequence bigint NOT NULL,
    event_type text NOT NULL,
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (job_id, sequence),
    CONSTRAINT job_events_sequence_positive CHECK (sequence > 0),
    CONSTRAINT job_events_type_format CHECK (
        event_type ~ '^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$'
    ),
    CONSTRAINT job_events_payload_size CHECK (pg_column_size(payload) <= 65536)
);

CREATE INDEX job_events_job_sequence_idx ON job_events (job_id, sequence);

CREATE TRIGGER job_events_immutable
BEFORE UPDATE OR DELETE ON job_events
FOR EACH ROW EXECUTE FUNCTION reject_immutable_row();
