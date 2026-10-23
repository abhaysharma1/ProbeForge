-- Plans, model-proposed tests, and immutable approval snapshots.

CREATE TABLE test_plans (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    job_id uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    version integer NOT NULL,
    status text NOT NULL DEFAULT 'draft',
    schema_version text NOT NULL DEFAULT '1',
    summary text NOT NULL,
    confidence numeric(3, 2) NOT NULL,
    plan_payload jsonb NOT NULL,
    agent_provider text,
    agent_model text,
    created_by_agent boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (job_id, version),
    UNIQUE (id, version),
    CONSTRAINT test_plans_version_positive CHECK (version > 0),
    CONSTRAINT test_plans_status_values CHECK (
        status IN ('draft', 'proposed', 'approved', 'superseded', 'rejected')
    ),
    CONSTRAINT test_plans_schema_version CHECK (schema_version = '1'),
    CONSTRAINT test_plans_summary_length CHECK (char_length(btrim(summary)) BETWEEN 1 AND 8000),
    CONSTRAINT test_plans_confidence_range CHECK (confidence >= 0 AND confidence <= 1),
    CONSTRAINT test_plans_payload_object CHECK (jsonb_typeof(plan_payload) = 'object'),
    CONSTRAINT test_plans_payload_size CHECK (pg_column_size(plan_payload) <= 262144)
);

CREATE TRIGGER test_plans_set_updated_at
BEFORE UPDATE ON test_plans
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX test_plans_job_version_idx ON test_plans (job_id, version DESC);
CREATE INDEX test_plans_project_idx ON test_plans (project_id, created_at DESC);

ALTER TABLE jobs
    ADD CONSTRAINT jobs_current_plan_id_fkey
    FOREIGN KEY (current_plan_id) REFERENCES test_plans (id) ON DELETE SET NULL;

CREATE TABLE planned_tests (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    plan_id uuid NOT NULL REFERENCES test_plans (id) ON DELETE CASCADE,
    catalog_test_id text NOT NULL,
    name text NOT NULL,
    rationale text NOT NULL,
    risk text NOT NULL,
    prerequisites jsonb NOT NULL DEFAULT '[]'::jsonb,
    expected_evidence text NOT NULL,
    target_requirements jsonb NOT NULL DEFAULT '[]'::jsonb,
    requires_explicit_confirmation boolean NOT NULL DEFAULT false,
    configuration jsonb NOT NULL DEFAULT '{}'::jsonb,
    selected boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (plan_id, catalog_test_id),
    CONSTRAINT planned_tests_catalog_id_format CHECK (
        catalog_test_id ~ '^[a-z0-9][a-z0-9.-]{1,118}$'
    ),
    CONSTRAINT planned_tests_name_length CHECK (char_length(btrim(name)) BETWEEN 1 AND 160),
    CONSTRAINT planned_tests_rationale_length CHECK (char_length(btrim(rationale)) BETWEEN 1 AND 2000),
    CONSTRAINT planned_tests_risk_values CHECK (risk IN ('low', 'medium', 'high', 'critical')),
    CONSTRAINT planned_tests_expected_evidence_length CHECK (
        char_length(btrim(expected_evidence)) BETWEEN 1 AND 1000
    ),
    CONSTRAINT planned_tests_prerequisites_array CHECK (jsonb_typeof(prerequisites) = 'array'),
    CONSTRAINT planned_tests_target_requirements_array CHECK (jsonb_typeof(target_requirements) = 'array'),
    CONSTRAINT planned_tests_configuration_size CHECK (pg_column_size(configuration) <= 16384)
);

CREATE INDEX planned_tests_plan_idx ON planned_tests (plan_id);
CREATE INDEX planned_tests_selected_idx ON planned_tests (plan_id, selected) WHERE selected;

CREATE TABLE plan_approvals (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    plan_id uuid NOT NULL,
    plan_version integer NOT NULL,
    approved_by uuid NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    approved_test_ids uuid[] NOT NULL,
    approval_snapshot jsonb NOT NULL,
    approved_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (plan_id, plan_version),
    CONSTRAINT plan_approvals_plan_version_positive CHECK (plan_version > 0),
    CONSTRAINT plan_approvals_tests_present CHECK (cardinality(approved_test_ids) > 0),
    CONSTRAINT plan_approvals_snapshot_object CHECK (jsonb_typeof(approval_snapshot) = 'object'),
    CONSTRAINT plan_approvals_snapshot_size CHECK (pg_column_size(approval_snapshot) <= 1048576),
    CONSTRAINT plan_approvals_plan_fkey
        FOREIGN KEY (plan_id, plan_version)
        REFERENCES test_plans (id, version)
        ON DELETE RESTRICT
);

CREATE INDEX plan_approvals_project_idx
    ON plan_approvals (project_id, approved_at DESC);

CREATE TRIGGER plan_approvals_immutable
BEFORE UPDATE OR DELETE ON plan_approvals
FOR EACH ROW EXECUTE FUNCTION reject_immutable_row();
