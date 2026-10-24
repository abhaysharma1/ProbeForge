-- Phase 12 load execution: schedules carry the exact target authorization they
-- may use, an optional expiry, and a cancellation timestamp. Recurrence is a
-- closed server-owned vocabulary; next_run_at remains the authoritative UTC
-- execution timestamp. Existing rows are not rewritten (NOT VALID), but new and
-- updated schedules must use the vocabulary.

ALTER TABLE schedules
    ADD COLUMN target_authorization_id uuid REFERENCES target_authorizations (id) ON DELETE RESTRICT,
    ADD COLUMN expires_at timestamptz,
    ADD COLUMN cancelled_at timestamptz;

ALTER TABLE schedules
    ADD CONSTRAINT schedules_recurrence_values CHECK (
        recurrence IN ('once', 'hourly', 'daily', 'weekly')
    ) NOT VALID,
    ADD CONSTRAINT schedules_expiry_order CHECK (
        expires_at IS NULL OR next_run_at IS NULL OR expires_at >= next_run_at
    ) NOT VALID;

CREATE INDEX schedules_target_authorization_idx
    ON schedules (target_authorization_id)
    WHERE target_authorization_id IS NOT NULL;

CREATE INDEX schedules_enabled_due_idx
    ON schedules (next_run_at)
    WHERE enabled AND target_authorization_id IS NOT NULL;
