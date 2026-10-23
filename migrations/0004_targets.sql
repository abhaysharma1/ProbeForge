-- Deployed targets and immutable, limit-bearing authorization records.

CREATE TABLE targets (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    normalized_origin text NOT NULL,
    environment text NOT NULL,
    allowed_hosts jsonb NOT NULL,
    allowed_paths jsonb NOT NULL,
    verification_status text NOT NULL DEFAULT 'pending',
    created_by uuid NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, normalized_origin),
    CONSTRAINT targets_environment_values CHECK (environment IN ('local', 'staging', 'production')),
    CONSTRAINT targets_verification_values CHECK (
        verification_status IN ('pending', 'verified', 'failed')
    ),
    CONSTRAINT targets_origin_length CHECK (char_length(normalized_origin) BETWEEN 8 AND 2048),
    CONSTRAINT targets_allowed_hosts_array CHECK (
        jsonb_typeof(allowed_hosts) = 'array' AND jsonb_array_length(allowed_hosts) > 0
    ),
    CONSTRAINT targets_allowed_paths_array CHECK (
        jsonb_typeof(allowed_paths) = 'array' AND jsonb_array_length(allowed_paths) > 0
    ),
    CONSTRAINT targets_scope_size CHECK (pg_column_size(allowed_hosts) <= 8192 AND pg_column_size(allowed_paths) <= 32768)
);

CREATE TRIGGER targets_set_updated_at
BEFORE UPDATE ON targets
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX targets_project_idx ON targets (project_id, created_at DESC);

CREATE TABLE target_authorizations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    target_id uuid NOT NULL REFERENCES targets (id) ON DELETE RESTRICT,
    authorized_by uuid NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    authorization_method text NOT NULL,
    scope jsonb NOT NULL,
    limits jsonb NOT NULL,
    confirmed_at timestamptz NOT NULL DEFAULT now(),
    valid_from timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT target_authorizations_method_values CHECK (
        authorization_method IN ('explicit_confirmation', 'ownership_verification')
    ),
    CONSTRAINT target_authorizations_scope_object CHECK (jsonb_typeof(scope) = 'object'),
    CONSTRAINT target_authorizations_limits_object CHECK (jsonb_typeof(limits) = 'object'),
    CONSTRAINT target_authorizations_scope_size CHECK (pg_column_size(scope) <= 32768),
    CONSTRAINT target_authorizations_limits_size CHECK (pg_column_size(limits) <= 8192),
    CONSTRAINT target_authorizations_validity CHECK (
        expires_at IS NULL OR expires_at > valid_from
    ),
    CONSTRAINT target_authorizations_revocation CHECK (
        revoked_at IS NULL OR revoked_at >= confirmed_at
    )
);

CREATE INDEX target_authorizations_target_idx
    ON target_authorizations (target_id, confirmed_at DESC);
CREATE INDEX target_authorizations_project_idx
    ON target_authorizations (project_id, confirmed_at DESC);
CREATE INDEX target_authorizations_active_idx
    ON target_authorizations (target_id, expires_at)
    WHERE revoked_at IS NULL;

CREATE FUNCTION protect_target_authorization()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'target authorization records cannot be deleted';
    END IF;

    IF NEW.project_id IS DISTINCT FROM OLD.project_id
        OR NEW.target_id IS DISTINCT FROM OLD.target_id
        OR NEW.authorized_by IS DISTINCT FROM OLD.authorized_by
        OR NEW.authorization_method IS DISTINCT FROM OLD.authorization_method
        OR NEW.scope IS DISTINCT FROM OLD.scope
        OR NEW.limits IS DISTINCT FROM OLD.limits
        OR NEW.confirmed_at IS DISTINCT FROM OLD.confirmed_at
        OR NEW.valid_from IS DISTINCT FROM OLD.valid_from
        OR NEW.expires_at IS DISTINCT FROM OLD.expires_at
    THEN
        RAISE EXCEPTION 'target authorization scope and limits are immutable';
    END IF;

    IF OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at THEN
        RAISE EXCEPTION 'target authorization is already revoked';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER target_authorizations_immutable
BEFORE UPDATE OR DELETE ON target_authorizations
FOR EACH ROW EXECUTE FUNCTION protect_target_authorization();
