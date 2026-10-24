use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::LoadRunError;

#[derive(Debug, Clone)]
pub struct RequestSample {
    pub at: DateTime<Utc>,
    pub endpoint: String,
    pub status_code: Option<u16>,
    pub latency_ms: f64,
    pub response_bytes: u64,
    pub error: bool,
    pub timeout: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricBucket {
    pub bucket_start: DateTime<Utc>,
    pub bucket_seconds: u32,
    pub endpoint: String,
    pub status_code: u16,
    pub request_count: u64,
    pub error_count: u64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub throughput_rps: f64,
    pub response_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunSummary {
    pub request_count: u64,
    pub error_count: u64,
    pub timeout_count: u64,
    pub dropped_iterations: u64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub throughput_rps: f64,
    pub response_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunMetrics {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub bucket_seconds: u32,
    pub buckets: Vec<MetricBucket>,
    pub summary: RunSummary,
}

pub fn aggregate_metrics(
    samples: &[RequestSample],
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    bucket_seconds: u32,
    dropped_iterations: u64,
) -> RunMetrics {
    let bucket_seconds = bucket_seconds.clamp(1, 3_600);
    let mut grouped: BTreeMap<(i64, String, u16), Vec<&RequestSample>> = BTreeMap::new();
    for sample in samples {
        let offset_ms = (sample.at - started_at).num_milliseconds().max(0);
        let bucket_index = offset_ms / (i64::from(bucket_seconds) * 1_000);
        let bucket_start = started_at + Duration::seconds(bucket_index * i64::from(bucket_seconds));
        grouped
            .entry((
                bucket_start.timestamp(),
                sample.endpoint.clone(),
                sample.status_code.unwrap_or(0),
            ))
            .or_default()
            .push(sample);
    }

    let mut buckets = Vec::with_capacity(grouped.len());
    for ((bucket_timestamp, endpoint, status_code), entries) in grouped {
        let mut latencies: Vec<f64> = entries.iter().map(|sample| sample.latency_ms).collect();
        latencies.sort_by(|left, right| left.total_cmp(right));
        let request_count = entries.len() as u64;
        buckets.push(MetricBucket {
            bucket_start: DateTime::from_timestamp(bucket_timestamp, 0).unwrap_or(started_at),
            bucket_seconds,
            endpoint,
            status_code,
            request_count,
            error_count: entries.iter().filter(|sample| sample.error).count() as u64,
            latency_p50_ms: percentile(&latencies, 0.50),
            latency_p95_ms: percentile(&latencies, 0.95),
            latency_p99_ms: percentile(&latencies, 0.99),
            throughput_rps: request_count as f64 / f64::from(bucket_seconds),
            response_bytes: entries.iter().map(|sample| sample.response_bytes).sum(),
        });
    }

    let mut all_latencies: Vec<f64> = samples.iter().map(|sample| sample.latency_ms).collect();
    all_latencies.sort_by(|left, right| left.total_cmp(right));
    let elapsed_seconds = (finished_at - started_at).num_milliseconds().max(1) as f64 / 1_000.0;
    let summary = RunSummary {
        request_count: samples.len() as u64,
        error_count: samples.iter().filter(|sample| sample.error).count() as u64,
        timeout_count: samples.iter().filter(|sample| sample.timeout).count() as u64,
        dropped_iterations,
        latency_p50_ms: percentile(&all_latencies, 0.50),
        latency_p95_ms: percentile(&all_latencies, 0.95),
        latency_p99_ms: percentile(&all_latencies, 0.99),
        throughput_rps: samples.len() as f64 / elapsed_seconds,
        response_bytes: samples.iter().map(|sample| sample.response_bytes).sum(),
    };

    RunMetrics {
        started_at,
        finished_at,
        bucket_seconds,
        buckets,
        summary,
    }
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((quantile * sorted.len() as f64).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[rank]
}

const MAX_PERSISTED_BUCKETS: usize = 10_000;

pub async fn persist_metrics(
    pool: &PgPool,
    project_id: Uuid,
    run_id: Uuid,
    metrics: &RunMetrics,
) -> Result<(), LoadRunError> {
    if metrics.buckets.len() > MAX_PERSISTED_BUCKETS {
        return Err(LoadRunError::MetricsTooLarge);
    }
    if !(1..=3_600).contains(&metrics.bucket_seconds) {
        return Err(LoadRunError::MetricsInvalid);
    }

    let mut transaction = pool.begin().await?;
    for bucket in &metrics.buckets {
        if bucket.endpoint.is_empty()
            || bucket.endpoint.len() > 500
            || bucket.endpoint.chars().any(char::is_control)
            || bucket.status_code > 599
            || !bucket.latency_p50_ms.is_finite()
            || !bucket.latency_p95_ms.is_finite()
            || !bucket.latency_p99_ms.is_finite()
            || !bucket.throughput_rps.is_finite()
        {
            return Err(LoadRunError::MetricsInvalid);
        }
        sqlx::query(
            r#"
            INSERT INTO run_metrics (
                project_id, run_id, bucket_start, bucket_seconds, endpoint,
                status_code, request_count, error_count, latency_p50_ms,
                latency_p95_ms, latency_p99_ms, throughput_rps, response_bytes
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (run_id, bucket_start, endpoint, status_code)
            DO UPDATE SET
                request_count = EXCLUDED.request_count,
                error_count = EXCLUDED.error_count,
                latency_p50_ms = EXCLUDED.latency_p50_ms,
                latency_p95_ms = EXCLUDED.latency_p95_ms,
                latency_p99_ms = EXCLUDED.latency_p99_ms,
                throughput_rps = EXCLUDED.throughput_rps,
                response_bytes = EXCLUDED.response_bytes
            "#,
        )
        .bind(project_id)
        .bind(run_id)
        .bind(bucket.bucket_start)
        .bind(i32::try_from(bucket.bucket_seconds).map_err(|_| LoadRunError::MetricsInvalid)?)
        .bind(&bucket.endpoint)
        .bind(i32::from(bucket.status_code))
        .bind(i64::try_from(bucket.request_count).map_err(|_| LoadRunError::MetricsInvalid)?)
        .bind(i64::try_from(bucket.error_count).map_err(|_| LoadRunError::MetricsInvalid)?)
        .bind(bucket.latency_p50_ms)
        .bind(bucket.latency_p95_ms)
        .bind(bucket.latency_p99_ms)
        .bind(bucket.throughput_rps)
        .bind(i64::try_from(bucket.response_bytes).map_err(|_| LoadRunError::MetricsInvalid)?)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at_ms: i64, endpoint: &str, status: u16, latency: f64, error: bool) -> RequestSample {
        RequestSample {
            at: DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp")
                + Duration::milliseconds(at_ms),
            endpoint: endpoint.to_owned(),
            status_code: Some(status),
            latency_ms: latency,
            response_bytes: 10,
            error,
            timeout: false,
        }
    }

    #[test]
    fn metrics_percentiles_and_buckets_match_known_fixture() {
        let started_at = DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp");
        let finished_at = started_at + Duration::seconds(10);
        let samples = vec![
            sample(0, "/api/items", 200, 10.0, false),
            sample(1_000, "/api/items", 200, 20.0, false),
            sample(2_000, "/api/items", 500, 30.0, true),
            sample(3_000, "/api/items", 200, 40.0, false),
            sample(11_000, "/api/items", 200, 100.0, false),
        ];
        let metrics = aggregate_metrics(&samples, started_at, finished_at, 10, 2);
        assert_eq!(metrics.summary.request_count, 5);
        assert_eq!(metrics.summary.error_count, 1);
        assert_eq!(metrics.summary.dropped_iterations, 2);
        assert_eq!(metrics.summary.latency_p50_ms, 30.0);
        assert_eq!(metrics.summary.latency_p95_ms, 100.0);
        assert_eq!(metrics.summary.latency_p99_ms, 100.0);
        assert_eq!(metrics.buckets.len(), 3);
        assert_eq!(metrics.buckets[0].request_count, 3);
        assert_eq!(metrics.buckets[0].error_count, 0);
        assert_eq!(metrics.buckets[0].throughput_rps, 0.3);
        assert_eq!(metrics.buckets[1].request_count, 1);
        assert_eq!(metrics.buckets[1].error_count, 1);
        assert_eq!(metrics.buckets[2].request_count, 1);
    }

    #[test]
    fn empty_metrics_are_zero_not_missing() {
        let started_at = Utc::now();
        let metrics = aggregate_metrics(&[], started_at, started_at, 1, 0);
        assert_eq!(metrics.summary.request_count, 0);
        assert_eq!(metrics.summary.latency_p50_ms, 0.0);
        assert!(metrics.buckets.is_empty());
    }
}
