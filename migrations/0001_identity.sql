-- Identity and project tenancy. Phase 3 implements authentication and
-- membership management; the tables and tenant boundaries exist from Phase 2.

CREATE FUNCTION set_updated_at()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$;

CREATE FUNCTION reject_immutable_row()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'immutable records cannot be modified or deleted';
END;
$$;

CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email text NOT NULL UNIQUE,
    display_name text NOT NULL,
    status text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT users_email_format CHECK (
        email = lower(email)
        AND char_length(email) BETWEEN 3 AND 320
        AND position('@' IN email) > 1
    ),
    CONSTRAINT users_display_name_length CHECK (
        char_length(btrim(display_name)) BETWEEN 1 AND 120
    ),
    CONSTRAINT users_status_values CHECK (status IN ('active', 'disabled'))
);

CREATE TRIGGER users_set_updated_at
BEFORE UPDATE ON users
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE projects (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    slug text NOT NULL UNIQUE,
    owner_id uuid NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT projects_name_length CHECK (char_length(btrim(name)) BETWEEN 1 AND 120),
    CONSTRAINT projects_slug_format CHECK (slug ~ '^[a-z0-9][a-z0-9-]{1,62}$')
);

CREATE TRIGGER projects_set_updated_at
BEFORE UPDATE ON projects
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE project_members (
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id),
    CONSTRAINT project_members_role_values CHECK (role IN ('owner', 'admin', 'collaborator', 'viewer'))
);

CREATE INDEX project_members_user_idx ON project_members (user_id);
CREATE INDEX projects_owner_idx ON projects (owner_id);
