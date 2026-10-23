-- A run may only cite the approval that belongs to its exact plan version.
-- The original schema had independent foreign keys for the plan and approval;
-- this composite constraint closes that integrity gap.

ALTER TABLE plan_approvals
    ADD CONSTRAINT plan_approvals_id_plan_version_key UNIQUE (id, plan_id, plan_version);

ALTER TABLE test_runs
    ADD CONSTRAINT test_runs_approval_match_fkey
        FOREIGN KEY (plan_approval_id, plan_id, plan_version)
        REFERENCES plan_approvals (id, plan_id, plan_version)
        ON DELETE RESTRICT;
