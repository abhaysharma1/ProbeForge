-- Planning custom OpenAI-compatible provider.
--
-- The planner can target an admin-configured OpenAI-compatible endpoint instead
-- of a pinned Pi SDK catalog provider. When `planning_base_url` is set, the
-- analysis worker registers a custom `openai-completions` provider for it; the
-- provider id becomes an arbitrary lowercase token rather than a catalog id.
--
-- The base URL is constrained to HTTPS here and rejected if it points at an
-- internal host by the API and again by the worker (defense in depth). No secret
-- material is added: the provider API key remains in `planning_api_key_ciphertext`.

ALTER TABLE platform_settings
    ADD COLUMN planning_base_url text,
    ADD COLUMN planning_context_window integer,
    ADD COLUMN planning_max_output_tokens integer;

ALTER TABLE platform_settings
    ADD CONSTRAINT platform_settings_base_url_length CHECK (
        planning_base_url IS NULL OR char_length(planning_base_url) BETWEEN 8 AND 300
    ),
    ADD CONSTRAINT platform_settings_base_url_scheme CHECK (
        planning_base_url IS NULL OR planning_base_url ~* '^https://'
    ),
    ADD CONSTRAINT platform_settings_context_window_range CHECK (
        planning_context_window IS NULL
            OR planning_context_window BETWEEN 1024 AND 2000000
    ),
    ADD CONSTRAINT platform_settings_max_output_tokens_range CHECK (
        planning_max_output_tokens IS NULL
            OR planning_max_output_tokens BETWEEN 128 AND 200000
    ),
    ADD CONSTRAINT platform_settings_output_within_context CHECK (
        planning_max_output_tokens IS NULL
            OR planning_context_window IS NULL
            OR planning_max_output_tokens <= planning_context_window
    );
