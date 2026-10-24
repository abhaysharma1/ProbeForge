-- Phase 13 results, recommendations, and versioned reports.
--
-- Raw structured observations stay on test_results independently of any AI or
-- generated summary. Recommendations are traceable to a result or evidence row
-- unless they are explicitly a hypothesis. Reports keep immutable, versioned
-- snapshots separate from results so a generation failure cannot destroy work.

ALTER TABLE test_results
    ADD COLUMN finding_status text,
    ADD COLUMN observation jsonb NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE test_results
    ADD CONSTRAINT test_results_finding_status_values CHECK (
        finding_status IS NULL
            OR finding_status IN ('confirmed', 'suspected', 'not_applicable')
    ) NOT VALID,
    ADD CONSTRAINT test_results_failed_finding_status CHECK (
        outcome <> 'failed'
            OR finding_status IN ('confirmed', 'suspected')
    ) NOT VALID,
    ADD CONSTRAINT test_results_observation_object CHECK (
        jsonb_typeof(observation) = 'object'
    ) NOT VALID,
    ADD CONSTRAINT test_results_observation_size CHECK (
        pg_column_size(observation) <= 32768
    ) NOT VALID;

CREATE INDEX test_results_outcome_idx
    ON test_results (run_id, outcome);

CREATE TABLE recommendations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES test_runs (id) ON DELETE CASCADE,
    result_id uuid REFERENCES test_results (id) ON DELETE SET NULL,
    evidence_id uuid REFERENCES evidence (id) ON DELETE SET NULL,
    run_metric_id uuid REFERENCES run_metrics (id) ON DELETE SET NULL,
    kind text NOT NULL,
    severity text NOT NULL,
    title text NOT NULL,
    body text NOT NULL,
    rationale text,
    confidence numeric(3, 2),
    generated_by text NOT NULL,
    model_provider text,
    model_name text,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT recommendations_kind_values CHECK (
        kind IN ('observed', 'hypothesis', 'recommendation')
    ),
    CONSTRAINT recommendations_severity_values CHECK (
        severity IN ('info', 'low', 'medium', 'high', 'critical')
    ),
    CONSTRAINT recommendations_title_length CHECK (
        char_length(btrim(title)) BETWEEN 1 AND 300
    ),
    CONSTRAINT recommendations_body_length CHECK (
        char_length(btrim(body)) BETWEEN 1 AND 8000
    ),
    CONSTRAINT recommendations_rationale_length CHECK (
        rationale IS NULL OR char_length(rationale) <= 8000
    ),
    CONSTRAINT recommendations_confidence_range CHECK (
        confidence IS NULL OR confidence BETWEEN 0 AND 1
    ),
    CONSTRAINT recommendations_generated_by_values CHECK (
        generated_by IN ('system', 'model')
    ),
    -- Every claim must cite a stored result, evidence row, or metric bucket,
    -- unless it is explicitly labeled a hypothesis.
    CONSTRAINT recommendations_traceability CHECK (
        kind = 'hypothesis'
            OR result_id IS NOT NULL
            OR evidence_id IS NOT NULL
            OR run_metric_id IS NOT NULL
    )
);

CREATE INDEX recommendations_run_idx ON recommendations (run_id, created_at);
CREATE INDEX recommendations_project_idx ON recommendations (project_id, created_at DESC);

CREATE TABLE reports (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES test_runs (id) ON DELETE CASCADE,
    report_type text NOT NULL,
    status text NOT NULL DEFAULT 'draft',
    current_version integer NOT NULL DEFAULT 0,
    last_error_code text,
    created_by uuid NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (run_id, report_type),
    CONSTRAINT reports_type_values CHECK (report_type IN ('security', 'load')),
    CONSTRAINT reports_status_values CHECK (
        status IN ('draft', 'final', 'failed')
    ),
    CONSTRAINT reports_current_version_nonnegative CHECK (current_version >= 0),
    CONSTRAINT reports_error_code_length CHECK (
        last_error_code IS NULL OR char_length(last_error_code) BETWEEN 1 AND 120
    )
);

CREATE TRIGGER reports_set_updated_at
BEFORE UPDATE ON reports
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX reports_run_idx ON reports (run_id);
CREATE INDEX reports_project_idx ON reports (project_id, created_at DESC);

CREATE TABLE report_versions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    report_id uuid NOT NULL REFERENCES reports (id) ON DELETE CASCADE,
    run_id uuid NOT NULL REFERENCES test_runs (id) ON DELETE CASCADE,
    version integer NOT NULL,
    status text NOT NULL,
    content jsonb NOT NULL,
    content_hash text NOT NULL,
    summary text NOT NULL,
    generated_by text NOT NULL,
    model_provider text,
    model_name text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (report_id, version),
    CONSTRAINT report_versions_version_positive CHECK (version > 0),
    CONSTRAINT report_versions_status_values CHECK (
        status IN ('final', 'failed')
    ),
    CONSTRAINT report_versions_content_object CHECK (jsonb_typeof(content) = 'object'),
    CONSTRAINT report_versions_content_size CHECK (pg_column_size(content) <= 262144),
    CONSTRAINT report_versions_hash_format CHECK (content_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT report_versions_summary_length CHECK (
        char_length(btrim(summary)) BETWEEN 1 AND 8000
    ),
    CONSTRAINT report_versions_generated_by_values CHECK (
        generated_by IN ('system', 'model')
    )
);

CREATE INDEX report_versions_report_idx ON report_versions (report_id, version DESC);
CREATE INDEX report_versions_run_idx ON report_versions (run_id, version DESC);

CREATE TRIGGER report_versions_immutable
BEFORE UPDATE OR DELETE ON report_versions
FOR EACH ROW EXECUTE FUNCTION reject_immutable_row();
