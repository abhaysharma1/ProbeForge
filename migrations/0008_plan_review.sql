-- Phase 8 plan review: optimistic revisioning, immutable approval revision
-- evidence, and rationale references on planned tests.

ALTER TABLE test_plans
    ADD COLUMN revision integer NOT NULL DEFAULT 0;

ALTER TABLE test_plans
    ADD CONSTRAINT test_plans_revision_nonnegative CHECK (revision >= 0);

ALTER TABLE plan_approvals
    ADD COLUMN plan_revision integer NOT NULL DEFAULT 0;

ALTER TABLE plan_approvals
    ADD CONSTRAINT plan_approvals_revision_nonnegative CHECK (plan_revision >= 0);

ALTER TABLE planned_tests
    ADD COLUMN rationale_references jsonb NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE planned_tests
    ADD CONSTRAINT planned_tests_rationale_references_array CHECK (
        jsonb_typeof(rationale_references) = 'array'
    );

ALTER TABLE planned_tests
    ADD CONSTRAINT planned_tests_rationale_references_size CHECK (
        pg_column_size(rationale_references) <= 16384
    );

