-- Phase 11 load-scenario safety: platform maxima for generated profiles and a
-- closed stop-condition key set. The constraints are added NOT VALID so an
-- existing deployment with historical rows is not rewritten during migration;
-- PostgreSQL still enforces them for every new or updated row.

ALTER TABLE load_profiles
    ADD CONSTRAINT load_profiles_platform_virtual_users CHECK (
        virtual_users IS NULL OR virtual_users BETWEEN 1 AND 100
    ) NOT VALID,
    ADD CONSTRAINT load_profiles_platform_arrival_rate CHECK (
        arrival_rate_per_second IS NULL OR arrival_rate_per_second BETWEEN 0.1 AND 100
    ) NOT VALID,
    ADD CONSTRAINT load_profiles_platform_ramp CHECK (
        ramp_up_seconds BETWEEN 0 AND 300
    ) NOT VALID,
    ADD CONSTRAINT load_profiles_platform_duration CHECK (
        duration_seconds BETWEEN 1 AND 900
    ) NOT VALID,
    ADD CONSTRAINT load_profiles_platform_requests CHECK (
        max_requests BETWEEN 1 AND 100000
    ) NOT VALID,
    ADD CONSTRAINT load_profiles_platform_rate CHECK (
        max_requests_per_second BETWEEN 0.1 AND 100
    ) NOT VALID,
    ADD CONSTRAINT load_profiles_stop_condition_keys CHECK (
        stop_conditions = '{}'::jsonb
        OR (
            jsonb_typeof(stop_conditions) = 'object'
            AND (stop_conditions - ARRAY['errorRate', 'p95LatencyMs', 'maxFailedRequests']) = '{}'::jsonb
        )
    ) NOT VALID;
