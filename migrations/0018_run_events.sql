-- Run SSE events: the run section keeps its own append-only, per-run sequenced
-- stream mirroring job_events. Lifecycle (run.*), execution (test.*), and
-- report (report.*) producers write here in the same transaction as the stream
-- change they describe, so `GET /runs/{run_id}/events` can replay missed events
-- with `Last-Event-ID` while PostgreSQL remains the source of truth.

CREATE TABLE run_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES test_runs (id) ON DELETE CASCADE,
    sequence bigint NOT NULL,
    event_type text NOT NULL,
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (run_id, sequence),
    CONSTRAINT run_events_sequence_positive CHECK (sequence > 0),
    CONSTRAINT run_events_type_format CHECK (
        event_type ~ '^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$'
    ),
    CONSTRAINT run_events_payload_size CHECK (pg_column_size(payload) <= 65536)
);

CREATE INDEX run_events_run_sequence_idx ON run_events (run_id, sequence);
CREATE INDEX run_events_project_created_idx ON run_events (project_id, created_at DESC);

CREATE TRIGGER run_events_immutable
BEFORE UPDATE OR DELETE ON run_events
FOR EACH ROW EXECUTE FUNCTION reject_immutable_row();
