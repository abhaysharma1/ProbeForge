-- Platform administration: global admins and all platform settings.
--
-- A platform admin is a settings-only operator: the role grants access to
-- /api/admin/* and nothing cross-tenant. Platform settings are a singleton row.
-- Secret values (provider API keys) are stored only as AES-256-GCM ciphertext.

CREATE TABLE platform_admins (
    user_id uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid REFERENCES users (id) ON DELETE SET NULL
);

CREATE TABLE platform_settings (
    id boolean PRIMARY KEY DEFAULT true,
    CONSTRAINT platform_settings_singleton CHECK (id),

    planning_provider text,
    planning_model text,
    planning_limits jsonb NOT NULL DEFAULT '{}'::jsonb,
    planning_api_key_ciphertext text,
    planning_api_key_hint text,
    planning_api_key_updated_at timestamptz,

    api_requests_per_minute integer NOT NULL DEFAULT 240,
    expensive_requests_per_minute integer NOT NULL DEFAULT 20,

    retention_event_days integer NOT NULL DEFAULT 30,
    retention_report_days integer NOT NULL DEFAULT 180,
    retention_counter_days integer NOT NULL DEFAULT 7,

    default_max_active_runs integer NOT NULL DEFAULT 2,
    default_max_requests_per_window bigint,
    default_window_seconds integer NOT NULL DEFAULT 86400,

    updated_by uuid REFERENCES users (id) ON DELETE SET NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT platform_settings_provider_length CHECK (
        planning_provider IS NULL OR char_length(planning_provider) BETWEEN 1 AND 80
    ),
    CONSTRAINT platform_settings_model_length CHECK (
        planning_model IS NULL OR char_length(planning_model) BETWEEN 1 AND 160
    ),
    CONSTRAINT platform_settings_limits_object CHECK (jsonb_typeof(planning_limits) = 'object'),
    CONSTRAINT platform_settings_limits_size CHECK (pg_column_size(planning_limits) <= 8192),
    CONSTRAINT platform_settings_key_format CHECK (
        planning_api_key_ciphertext IS NULL
            OR planning_api_key_ciphertext ~ '^v1\.[A-Za-z0-9+/=]+\.[A-Za-z0-9+/=]+$'
    ),
    CONSTRAINT platform_settings_hint_length CHECK (
        planning_api_key_hint IS NULL OR char_length(planning_api_key_hint) <= 16
    ),
    CONSTRAINT platform_settings_api_rate_range CHECK (
        api_requests_per_minute BETWEEN 10 AND 100000
            AND expensive_requests_per_minute BETWEEN 1 AND 10000
    ),
    CONSTRAINT platform_settings_retention_range CHECK (
        retention_event_days BETWEEN 1 AND 3650
            AND retention_report_days BETWEEN 1 AND 3650
            AND retention_counter_days BETWEEN 1 AND 3650
    ),
    CONSTRAINT platform_settings_defaults_range CHECK (
        default_max_active_runs BETWEEN 1 AND 100
            AND default_window_seconds BETWEEN 60 AND 604800
    ),
    CONSTRAINT platform_settings_default_budget_range CHECK (
        default_max_requests_per_window IS NULL
            OR default_max_requests_per_window BETWEEN 0 AND 1000000000
    )
);

CREATE TRIGGER platform_settings_set_updated_at
BEFORE UPDATE ON platform_settings
FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The singleton row always exists so reads never need a fallback.
INSERT INTO platform_settings (id) VALUES (true);
