-- Approvals cover both job goals. A vulnerability approval selects vetted tests;
-- a load approval selects scenarios. At least one selection must be present.
-- The arrays are evidence of the approved selection, not foreign keys: approval
-- snapshots remain immutable even if the underlying plan is later superseded.

ALTER TABLE plan_approvals
    ADD COLUMN approved_load_scenario_ids uuid[] NOT NULL DEFAULT '{}'::uuid[];

ALTER TABLE plan_approvals
    DROP CONSTRAINT plan_approvals_tests_present;

ALTER TABLE plan_approvals
    ADD CONSTRAINT plan_approvals_selection_present CHECK (
        cardinality(approved_test_ids) > 0
        OR cardinality(approved_load_scenario_ids) > 0
    );
