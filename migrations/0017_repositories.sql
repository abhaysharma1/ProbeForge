-- Repository registry: friendly names and archive-based removal.
--
-- `git_sources` was previously only an implicit side effect of job creation, so a
-- project could not list or remove a connected repository. This migration makes it
-- a first-class, project-scoped registry by adding an optional display name and an
-- archive marker.
--
-- Removal archives a repository (`archived_at`) instead of deleting the row. Job
-- history (plans, approvals, runs, evidence) and `job_events` are append-only and
-- protected by delete-rejecting triggers; archiving keeps that history intact while
-- hiding the repository from the active list and blocking new analysis. Reconnecting
-- the same URL (a new job or an explicit registration) clears `archived_at`.
--
-- The existing `UNIQUE (project_id, normalized_url)` is intentionally unchanged: one
-- row per project+URL, archived or not.

ALTER TABLE git_sources
    ADD COLUMN display_name text,
    ADD COLUMN archived_at timestamptz;

ALTER TABLE git_sources
    ADD CONSTRAINT git_sources_display_name_length CHECK (
        display_name IS NULL OR char_length(btrim(display_name)) BETWEEN 1 AND 120
    );

-- List queries filter on active repositories only.
CREATE INDEX git_sources_project_active_idx
    ON git_sources (project_id, created_at DESC)
    WHERE archived_at IS NULL;
