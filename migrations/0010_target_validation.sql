-- Phase 9 target validation evidence. The normalized origin and scope remain
-- protected by the existing target constraints; these columns record the last
-- server-side DNS/IP validation without storing request bodies or credentials.

ALTER TABLE targets
    ADD COLUMN verification_checked_at timestamptz;

ALTER TABLE targets
    ADD COLUMN verified_addresses jsonb NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE targets
    ADD COLUMN allowed_ports jsonb NOT NULL DEFAULT '[443]'::jsonb;

ALTER TABLE targets
    ADD CONSTRAINT targets_verified_addresses_array CHECK (
        jsonb_typeof(verified_addresses) = 'array'
    );

ALTER TABLE targets
    ADD CONSTRAINT targets_verified_addresses_size CHECK (
        pg_column_size(verified_addresses) <= 8192
    );

ALTER TABLE targets
    ADD CONSTRAINT targets_allowed_ports_array CHECK (
        jsonb_typeof(allowed_ports) = 'array'
        AND jsonb_array_length(allowed_ports) > 0
    );

ALTER TABLE targets
    ADD CONSTRAINT targets_allowed_ports_size CHECK (
        pg_column_size(allowed_ports) <= 8192
    );
