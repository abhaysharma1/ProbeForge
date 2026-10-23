-- Phase 3 authentication state. Passwords are Argon2id PHC strings; the
-- plaintext password is never stored. Session tokens are stored only as
-- SHA-256 digests, and CSRF tokens use a double-submit cookie rather than a
-- database secret. Existing users may have a null hash until the operator
-- provisions credentials with the create-user binary; null hashes fail closed.

ALTER TABLE users
    ADD COLUMN password_hash text;

ALTER TABLE users
    ADD CONSTRAINT users_password_hash_format CHECK (
        password_hash IS NULL
        OR (
            char_length(password_hash) BETWEEN 50 AND 512
            AND password_hash LIKE '$argon2id$%'
        )
    );

CREATE TABLE auth_sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash bytea NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    CONSTRAINT auth_sessions_token_hash_length CHECK (octet_length(token_hash) = 32),
    CONSTRAINT auth_sessions_expiry_order CHECK (expires_at > created_at),
    CONSTRAINT auth_sessions_revocation_order CHECK (
        revoked_at IS NULL OR revoked_at >= created_at
    )
);

CREATE INDEX auth_sessions_user_created_idx
    ON auth_sessions (user_id, created_at DESC);
CREATE INDEX auth_sessions_expiry_idx
    ON auth_sessions (expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE auth_login_attempts (
    key_hash bytea PRIMARY KEY,
    window_started_at timestamptz NOT NULL,
    attempts integer NOT NULL,
    last_attempt_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT auth_login_attempts_key_length CHECK (octet_length(key_hash) = 32),
    CONSTRAINT auth_login_attempts_attempts_range CHECK (attempts BETWEEN 0 AND 1000000)
);

CREATE INDEX auth_login_attempts_last_attempt_idx
    ON auth_login_attempts (last_attempt_at);
