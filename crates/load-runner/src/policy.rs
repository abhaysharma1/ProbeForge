use probeforge_contracts::{LoadProfile, TargetLimits};

use crate::LoadRunError;

/// Platform-owned ceilings. A scenario, model, or target authorization can only
/// reduce these; effective limits are always the minimum of all three.
#[derive(Debug, Clone, Copy)]
pub struct LoadCaps {
    pub max_virtual_users: u32,
    pub max_arrival_rate: f64,
    pub max_ramp_seconds: u32,
    pub max_duration_seconds: u32,
    pub max_requests: u64,
    pub max_requests_per_second: f64,
    pub max_concurrency: u32,
    pub max_active_global: i64,
    pub max_active_per_project: i64,
    pub bucket_seconds: u32,
}

impl Default for LoadCaps {
    fn default() -> Self {
        Self {
            max_virtual_users: 100,
            max_arrival_rate: 100.0,
            max_ramp_seconds: 300,
            max_duration_seconds: 900,
            max_requests: 100_000,
            max_requests_per_second: 100.0,
            max_concurrency: 100,
            max_active_global: 4,
            max_active_per_project: 1,
            bucket_seconds: 10,
        }
    }
}

impl LoadCaps {
    pub fn validation() -> Self {
        Self {
            bucket_seconds: 1,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EffectiveLimits {
    pub virtual_users: Option<u32>,
    pub arrival_rate_per_second: Option<f64>,
    pub ramp_up_seconds: u32,
    pub duration_seconds: u32,
    pub max_requests: u64,
    pub max_requests_per_second: f64,
    pub max_concurrency: u32,
    pub max_response_bytes: u64,
    pub max_total_bytes: u64,
    pub max_failed_requests: Option<u64>,
    pub max_error_rate: Option<f64>,
    pub max_p95_latency_ms: Option<u32>,
    pub bucket_seconds: u32,
}

pub fn resolve_limits(
    profile: &LoadProfile,
    target_limits: &TargetLimits,
    caps: LoadCaps,
) -> Result<EffectiveLimits, LoadRunError> {
    let virtual_users = profile.virtual_users;
    let arrival_rate = profile.arrival_rate_per_second;
    if virtual_users.is_none() == arrival_rate.is_none() {
        return Err(LoadRunError::InvalidLimits);
    }
    if !target_limits.max_requests_per_second.is_finite()
        || target_limits.max_requests_per_second <= 0.0
        || target_limits.max_concurrency == 0
        || target_limits.max_duration_seconds == 0
        || target_limits.max_requests == 0
    {
        return Err(LoadRunError::InvalidLimits);
    }

    let max_concurrency = target_limits
        .max_concurrency
        .min(caps.max_concurrency)
        .max(1);
    let virtual_users = virtual_users.map(|value| {
        value
            .min(caps.max_virtual_users)
            .min(max_concurrency)
            .max(1)
    });
    let arrival_rate = arrival_rate.map(|value| {
        value
            .min(caps.max_arrival_rate)
            .min(target_limits.max_requests_per_second)
            .min(caps.max_requests_per_second)
            .max(0.1)
    });
    let duration_seconds = profile
        .duration_seconds
        .min(target_limits.max_duration_seconds)
        .min(caps.max_duration_seconds)
        .max(1);
    let ramp_up_seconds = profile
        .ramp_up_seconds
        .min(caps.max_ramp_seconds)
        .min(duration_seconds);
    let max_requests = profile
        .max_requests
        .min(target_limits.max_requests)
        .min(caps.max_requests)
        .max(1);
    let max_requests_per_second = profile
        .max_requests_per_second
        .min(target_limits.max_requests_per_second)
        .min(caps.max_requests_per_second)
        .max(0.1);

    if let Some(vus) = virtual_users
        && vus > max_concurrency
    {
        return Err(LoadRunError::InvalidLimits);
    }

    Ok(EffectiveLimits {
        virtual_users,
        arrival_rate_per_second: arrival_rate,
        ramp_up_seconds,
        duration_seconds,
        max_requests,
        max_requests_per_second,
        max_concurrency,
        max_response_bytes: crate::MAX_RESPONSE_BYTES,
        max_total_bytes: crate::MAX_TOTAL_BYTES,
        max_failed_requests: profile.stop_conditions.max_failed_requests,
        max_error_rate: profile.stop_conditions.error_rate,
        max_p95_latency_ms: profile.stop_conditions.p95_latency_ms,
        bucket_seconds: caps.bucket_seconds.clamp(1, 3_600),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(vus: u32) -> LoadProfile {
        LoadProfile {
            virtual_users: Some(vus),
            arrival_rate_per_second: None,
            ramp_up_seconds: 10,
            duration_seconds: 60,
            max_requests: 1_000,
            max_requests_per_second: 5.0,
            stop_conditions: Default::default(),
        }
    }

    fn target() -> TargetLimits {
        TargetLimits {
            max_requests_per_second: 3.0,
            max_concurrency: 4,
            max_duration_seconds: 30,
            max_requests: 100,
        }
    }

    #[test]
    fn effective_limits_are_the_minimum_of_profile_target_and_platform_caps() {
        let limits = resolve_limits(&profile(50), &target(), LoadCaps::default()).expect("limits");
        assert_eq!(limits.virtual_users, Some(4));
        assert_eq!(limits.max_requests_per_second, 3.0);
        assert_eq!(limits.duration_seconds, 30);
        assert_eq!(limits.max_requests, 100);
        assert_eq!(limits.max_concurrency, 4);
        assert_eq!(limits.ramp_up_seconds, 10);
    }

    #[test]
    fn platform_caps_clamp_virtual_users_even_with_a_generous_target() {
        let limits = resolve_limits(
            &profile(500),
            &TargetLimits {
                max_requests_per_second: 1_000.0,
                max_concurrency: 100,
                max_duration_seconds: 3_600,
                max_requests: 1_000_000,
            },
            LoadCaps::default(),
        )
        .expect("limits");
        assert_eq!(limits.virtual_users, Some(100));
    }

    #[test]
    fn invalid_sizing_mode_fails_closed() {
        let mut value = profile(1);
        value.virtual_users = None;
        value.arrival_rate_per_second = None;
        assert!(matches!(
            resolve_limits(&value, &target(), LoadCaps::default()),
            Err(LoadRunError::InvalidLimits)
        ));
    }
}
