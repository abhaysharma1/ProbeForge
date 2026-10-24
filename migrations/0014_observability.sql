-- Phase 14 observability, quotas, and retention.
--
-- Server-enforced per-project quotas, generic API rate-limit counters, and a
-- single sanctioned retention path. Immutable rows remain immutable for every
-- ordinary code path; the retention purge is the only exception and must run
-- inside a transaction that opts in with `SET LOCAL probeforge.retention_purge
-- = 'on'`. Without that local setting the trigger rejects every UPDATE and
-- DELETE, so the tamper-resistance guarantee is unchanged.

CREATE TABLE project_quotas (
    project_id uuid PRIMARY KEY REFERENCES projects (id) ON DELETE CASCADE,
    max_active_runs integer NOT NULL DEFAULT 2,
    max_requests_per_window bigint,
    window_seconds integer NOT NULL DEFAULT 86400,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT project_quotas_active_runs_range CHECK (max_active_runs BETWEEN 1 AND 100),
    CONSTRAINT project_quotas_requests_range CHECK (
        max_requests_per_window IS NULL
            OR max_requests_per_window BETWEEN 0 AND 1000000000
    ),
    CONSTRAINT project_quotas_window_range CHECK (window_seconds BETWEEN 60 AND 604800)
);

CREATE TRIGGER project_quotas_set_updated_at
BEFORE UPDATE ON project_quotas
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Generic API rate-limit counters. Keys are SHA-256 digests so the raw
-- identity (user id or IP) is never stored in the clear. This is deliberately
-- separate from the authentication attempt counter, which has its own window.
CREATE TABLE api_rate_limits (
    key_hash bytea PRIMARY KEY,
    window_started_at timestamptz NOT NULL,
    count integer NOT NULL,
    last_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT api_rate_limits_key_length CHECK (octet_length(key_hash) = 32),
    CONSTRAINT api_rate_limits_count_range CHECK (count BETWEEN 0 AND 1000000000)
);

CREATE INDEX api_rate_limits_last_at_idx ON api_rate_limits (last_at);

CREATE INDEX evidence_retention_idx
    ON evidence (retention_until)
    WHERE retention_until IS NOT NULL;

CREATE INDEX job_events_occurred_idx ON job_events (occurred_at);

CREATE OR REPLACE FUNCTION reject_immutable_row()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE'
        AND current_setting('probeforge.retention_purge', true) = 'on'
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'immutable records cannot be modified or deleted';
END;
$$;
