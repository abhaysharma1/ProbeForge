use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use chrono::Utc;
use probe_forge_target_policy::{RunBudget, TargetRequestError};
use probeforge_contracts::{
    LoadAssertion, LoadAssertionOperator, LoadAssertionSource, LoadExtraction, LoadExtractionSource,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use tokio::{
    sync::{Mutex, Semaphore},
    task::JoinSet,
    time::sleep,
};

use crate::metrics::{RequestSample, aggregate_metrics};
use crate::{LoadRunError, LoadRunOutcome, RunStatus, RunnerSpec, RunnerStep, SecretResolver};

struct IterationResult {
    requests: u64,
    dropped: bool,
}

pub async fn run_load(
    spec: &RunnerSpec,
    resolver: Arc<dyn SecretResolver>,
    budget: &RunBudget,
) -> Result<LoadRunOutcome, LoadRunError> {
    let client = spec.client();
    let samples = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let stop_condition = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicU64::new(0));
    let started_at = Utc::now();

    tracing::debug!(
        event = "load.run_started",
        scenario = %spec.scenario_name,
        steps = spec.steps.len(),
        virtual_users = ?spec.limits.virtual_users,
        arrival_rate_per_second = ?spec.limits.arrival_rate_per_second,
        max_requests = spec.limits.max_requests,
        duration_seconds = spec.limits.duration_seconds,
        "load run started"
    );

    for step in &spec.steps {
        for reference in step.secret_headers.values() {
            resolver.resolve(reference)?;
        }
    }

    if let Some(vus) = spec.limits.virtual_users {
        run_virtual_users(
            spec,
            resolver.clone(),
            budget,
            &client,
            &samples,
            &stop,
            &stop_condition,
            &dropped,
            vus,
        )
        .await?;
    } else if let Some(arrival_rate) = spec.limits.arrival_rate_per_second {
        run_arrival_rate(
            spec,
            resolver.clone(),
            budget,
            &client,
            &samples,
            &stop,
            &stop_condition,
            &dropped,
            arrival_rate,
        )
        .await?;
    } else {
        return Err(LoadRunError::InvalidLimits);
    }

    let finished_at = Utc::now();
    let collected = samples.lock().await.clone();
    let metrics = aggregate_metrics(
        &collected,
        started_at,
        finished_at,
        spec.limits.bucket_seconds,
        dropped.load(Ordering::SeqCst),
    );
    let status = if budget.is_cancelled() {
        RunStatus::Cancelled
    } else if stop_condition.load(Ordering::SeqCst) {
        RunStatus::Failed
    } else {
        RunStatus::Completed
    };
    tracing::info!(
        event = "load.run_finished",
        scenario = %spec.scenario_name,
        status = ?status,
        requests = metrics.summary.request_count,
        errors = metrics.summary.error_count,
        timeouts = metrics.summary.timeout_count,
        dropped_iterations = metrics.summary.dropped_iterations,
        "load run finished"
    );
    Ok(LoadRunOutcome { status, metrics })
}

#[allow(clippy::too_many_arguments)]
async fn run_virtual_users(
    spec: &RunnerSpec,
    resolver: Arc<dyn SecretResolver>,
    budget: &RunBudget,
    client: &probe_forge_target_policy::GuardedHttpClient,
    samples: &Arc<Mutex<Vec<RequestSample>>>,
    stop: &Arc<AtomicBool>,
    stop_condition: &Arc<AtomicBool>,
    dropped: &Arc<AtomicU64>,
    virtual_users: u32,
) -> Result<(), LoadRunError> {
    let mut tasks = JoinSet::new();
    for index in 0..virtual_users {
        let delay_seconds = if virtual_users > 0 {
            u64::from(spec.limits.ramp_up_seconds) * u64::from(index) / u64::from(virtual_users)
        } else {
            0
        };
        let spec = spec.clone();
        let resolver = resolver.clone();
        let budget = budget.clone();
        let client = client.clone();
        let samples = samples.clone();
        let stop = stop.clone();
        let stop_condition = stop_condition.clone();
        let dropped = dropped.clone();
        tasks.spawn(async move {
            if delay_seconds > 0 {
                sleep(std::time::Duration::from_secs(delay_seconds)).await;
            }
            loop {
                if stop.load(Ordering::SeqCst)
                    || budget.is_cancelled()
                    || budget.remaining().is_zero()
                {
                    break;
                }
                let result =
                    execute_iteration(&spec, resolver.as_ref(), &budget, &client, &samples).await;
                match result {
                    Ok(iteration) => {
                        if iteration.dropped {
                            dropped.fetch_add(1, Ordering::SeqCst);
                        }
                        if iteration.requests > 0 && should_stop(&spec, &samples).await {
                            stop_condition.store(true, Ordering::SeqCst);
                            stop.store(true, Ordering::SeqCst);
                            break;
                        }
                        if iteration.requests == 0 {
                            break;
                        }
                    }
                    Err(LoadRunError::Target(TargetRequestError::Cancelled))
                    | Err(LoadRunError::Target(TargetRequestError::DurationExceeded))
                    | Err(LoadRunError::Target(TargetRequestError::MaxRequests)) => break,
                    Err(error) => return Err(error),
                }
            }
            Ok::<(), LoadRunError>(())
        });
    }

    while let Some(result) = tasks.join_next().await {
        result.map_err(|_| LoadRunError::InvalidConfiguration)??;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_arrival_rate(
    spec: &RunnerSpec,
    resolver: Arc<dyn SecretResolver>,
    budget: &RunBudget,
    client: &probe_forge_target_policy::GuardedHttpClient,
    samples: &Arc<Mutex<Vec<RequestSample>>>,
    stop: &Arc<AtomicBool>,
    stop_condition: &Arc<AtomicBool>,
    dropped: &Arc<AtomicU64>,
    arrival_rate: f64,
) -> Result<(), LoadRunError> {
    let interval = std::time::Duration::from_secs_f64(1.0 / arrival_rate.max(0.1));
    let semaphore = Arc::new(Semaphore::new(spec.limits.max_concurrency as usize));
    let mut tasks = JoinSet::new();

    loop {
        if stop.load(Ordering::SeqCst) || budget.is_cancelled() || budget.remaining().is_zero() {
            break;
        }
        let Ok(permit) = semaphore.clone().acquire_owned().await else {
            break;
        };
        let spec = spec.clone();
        let resolver = resolver.clone();
        let budget = budget.clone();
        let client = client.clone();
        let samples = samples.clone();
        let stop = stop.clone();
        let stop_condition = stop_condition.clone();
        let dropped = dropped.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let result =
                execute_iteration(&spec, resolver.as_ref(), &budget, &client, &samples).await;
            match result {
                Ok(iteration) => {
                    if iteration.dropped {
                        dropped.fetch_add(1, Ordering::SeqCst);
                    }
                    if iteration.requests > 0 && should_stop(&spec, &samples).await {
                        stop_condition.store(true, Ordering::SeqCst);
                        stop.store(true, Ordering::SeqCst);
                    }
                }
                Err(LoadRunError::Target(TargetRequestError::Cancelled))
                | Err(LoadRunError::Target(TargetRequestError::DurationExceeded))
                | Err(LoadRunError::Target(TargetRequestError::MaxRequests)) => {}
                Err(error) => return Err(error),
            }
            Ok::<(), LoadRunError>(())
        });
        while let Some(result) = tasks.try_join_next() {
            result.map_err(|_| LoadRunError::InvalidConfiguration)??;
        }
        sleep(interval).await;
    }

    while let Some(result) = tasks.join_next().await {
        result.map_err(|_| LoadRunError::InvalidConfiguration)??;
    }
    Ok(())
}

async fn execute_iteration(
    spec: &RunnerSpec,
    resolver: &dyn SecretResolver,
    budget: &RunBudget,
    client: &probe_forge_target_policy::GuardedHttpClient,
    samples: &Arc<Mutex<Vec<RequestSample>>>,
) -> Result<IterationResult, LoadRunError> {
    let mut extracted = BTreeMap::new();
    let mut requests = 0;

    for step in &spec.steps {
        if budget.is_cancelled() || budget.remaining().is_zero() {
            return Ok(IterationResult {
                requests,
                dropped: true,
            });
        }
        let path = render_path(&step.path_template, &extracted)?;
        let url = spec
            .origin
            .join(&path)
            .map_err(|_| LoadRunError::InvalidTemplate)?;
        let headers = build_headers(step, resolver)?;
        let body = render_body(step.body_template.as_ref(), &extracted)?;
        let started = Instant::now();
        let result = client
            .load_request(
                &spec.guard,
                step.method,
                &url,
                &headers,
                body.as_deref(),
                budget,
            )
            .await;
        let latency_ms = started.elapsed().as_secs_f64() * 1_000.0;
        requests += 1;

        match result {
            Ok(response) => {
                let expected = step.expected_statuses.contains(&response.status);
                let assertion_ok = step
                    .assertions
                    .iter()
                    .all(|assertion| assertion_matches(assertion, &response));
                let extraction_ok = extract_values(step, &response, &mut extracted);
                let error = !expected || !assertion_ok || !extraction_ok;
                samples.lock().await.push(RequestSample {
                    at: Utc::now(),
                    endpoint: step.id.clone(),
                    status_code: Some(response.status),
                    latency_ms,
                    response_bytes: response.body.len() as u64,
                    error,
                    timeout: false,
                });
                if !assertion_ok || !extraction_ok {
                    break;
                }
            }
            Err(TargetRequestError::Cancelled) => return Err(TargetRequestError::Cancelled.into()),
            Err(TargetRequestError::DurationExceeded | TargetRequestError::MaxRequests) => {
                return Ok(IterationResult {
                    requests,
                    dropped: true,
                });
            }
            Err(error) => {
                let timeout = matches!(error, TargetRequestError::RequestTimeout);
                samples.lock().await.push(RequestSample {
                    at: Utc::now(),
                    endpoint: step.id.clone(),
                    status_code: None,
                    latency_ms,
                    response_bytes: 0,
                    error: true,
                    timeout,
                });
                if matches!(
                    error,
                    TargetRequestError::Validation(_) | TargetRequestError::InvalidHeader
                ) {
                    return Err(error.into());
                }
                if timeout {
                    break;
                }
            }
        }
    }

    Ok(IterationResult {
        requests,
        dropped: false,
    })
}

async fn should_stop(spec: &RunnerSpec, samples: &Arc<Mutex<Vec<RequestSample>>>) -> bool {
    let samples = samples.lock().await;
    let requests = samples.len() as u64;
    if requests < 10 {
        return false;
    }
    let errors = samples.iter().filter(|sample| sample.error).count() as u64;
    if spec
        .limits
        .max_failed_requests
        .is_some_and(|maximum| errors >= maximum)
    {
        return true;
    }
    if spec
        .limits
        .max_error_rate
        .is_some_and(|maximum| errors as f64 / requests as f64 > maximum)
    {
        return true;
    }
    if let Some(maximum) = spec.limits.max_p95_latency_ms {
        let mut latencies: Vec<f64> = samples
            .iter()
            .filter(|sample| !sample.timeout)
            .map(|sample| sample.latency_ms)
            .collect();
        latencies.sort_by(|left, right| left.total_cmp(right));
        if !latencies.is_empty() {
            let rank = ((0.95 * latencies.len() as f64).ceil() as usize)
                .saturating_sub(1)
                .min(latencies.len() - 1);
            if latencies[rank] > f64::from(maximum) {
                return true;
            }
        }
    }
    false
}

fn build_headers(
    step: &RunnerStep,
    resolver: &dyn SecretResolver,
) -> Result<HeaderMap, LoadRunError> {
    let mut headers = HeaderMap::new();
    for (name, value) in &step.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| LoadRunError::InvalidConfiguration)?;
        let value = HeaderValue::from_str(value).map_err(|_| LoadRunError::InvalidConfiguration)?;
        headers.insert(name, value);
    }
    for (name, reference) in &step.secret_headers {
        let resolved = resolver.resolve(reference)?;
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| LoadRunError::InvalidConfiguration)?;
        let value =
            HeaderValue::from_str(&resolved).map_err(|_| LoadRunError::InvalidConfiguration)?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn render_path(template: &str, values: &BTreeMap<String, String>) -> Result<String, LoadRunError> {
    render_template(template, values, true)
}

fn render_text(template: &str, values: &BTreeMap<String, String>) -> Result<String, LoadRunError> {
    render_template(template, values, false)
}

fn render_template(
    template: &str,
    values: &BTreeMap<String, String>,
    encode_path: bool,
) -> Result<String, LoadRunError> {
    let mut output = String::with_capacity(template.len());
    let mut remainder = template;
    while let Some(start) = remainder.find("${") {
        output.push_str(&remainder[..start]);
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find('}') else {
            return Err(LoadRunError::InvalidTemplate);
        };
        let name = &after_start[..end];
        let value = values.get(name).ok_or(LoadRunError::InvalidTemplate)?;
        if encode_path {
            output.push_str(&encode_path_value(value));
        } else {
            if value.chars().any(char::is_control) {
                return Err(LoadRunError::InvalidTemplate);
            }
            output.push_str(value);
        }
        remainder = &after_start[end + 1..];
    }
    output.push_str(remainder);
    Ok(output)
}

fn encode_path_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn render_body(
    template: Option<&Value>,
    values: &BTreeMap<String, String>,
) -> Result<Option<Vec<u8>>, LoadRunError> {
    let Some(template) = template else {
        return Ok(None);
    };
    let rendered = render_json(template, values)?;
    let body = serde_json::to_vec(&rendered).map_err(|_| LoadRunError::InvalidTemplate)?;
    if body.len() > crate::MAX_REQUEST_BODY_BYTES {
        return Err(LoadRunError::RequestBodyTooLarge);
    }
    Ok(Some(body))
}

fn render_json(value: &Value, values: &BTreeMap<String, String>) -> Result<Value, LoadRunError> {
    match value {
        Value::String(text) => Ok(Value::String(render_text(text, values)?)),
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|item| render_json(item, values))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Object(map) => {
            let mut rendered = serde_json::Map::new();
            for (key, item) in map {
                rendered.insert(key.clone(), render_json(item, values)?);
            }
            Ok(Value::Object(rendered))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
    }
}

fn assertion_matches(
    assertion: &LoadAssertion,
    response: &probe_forge_target_policy::GuardedResponse,
) -> bool {
    let actual = match assertion.source {
        LoadAssertionSource::Header => response
            .headers
            .get(&assertion.selector)
            .and_then(|value| value.to_str().ok())
            .map(|value| Value::String(value.to_owned())),
        LoadAssertionSource::JsonBody => serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|body| pointer_value(&body, &assertion.selector).cloned()),
    };
    match assertion.operator {
        LoadAssertionOperator::Exists => actual.is_some(),
        LoadAssertionOperator::Equals => actual.as_ref() == assertion.expected.as_ref(),
        LoadAssertionOperator::Contains => match (actual, assertion.expected.as_ref()) {
            (Some(Value::String(actual)), Some(Value::String(expected))) => {
                actual.contains(expected)
            }
            (Some(Value::Array(actual)), Some(expected)) => actual.contains(expected),
            _ => false,
        },
    }
}

fn extract_values(
    step: &RunnerStep,
    response: &probe_forge_target_policy::GuardedResponse,
    extracted: &mut BTreeMap<String, String>,
) -> bool {
    for extraction in &step.extract {
        let value = extraction_value(extraction, response);
        match value {
            Some(value) => {
                extracted.insert(extraction.name.clone(), value);
            }
            None if extraction.required => return false,
            None => {}
        }
    }
    true
}

fn extraction_value(
    extraction: &LoadExtraction,
    response: &probe_forge_target_policy::GuardedResponse,
) -> Option<String> {
    match extraction.source {
        LoadExtractionSource::Header => response
            .headers
            .get(&extraction.selector)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        LoadExtractionSource::JsonBody => {
            let body = serde_json::from_slice::<Value>(&response.body).ok()?;
            let value = pointer_value(&body, &extraction.selector)?;
            match value {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                Value::Bool(value) => Some(value.to_string()),
                _ => None,
            }
        }
    }
}

fn pointer_value<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() {
        return Some(value);
    }
    if !pointer.starts_with('/') {
        return None;
    }
    let mut current = value;
    for token in pointer[1..].split('/') {
        let token = token.replace("~1", "/").replace("~0", "~");
        current = match current {
            Value::Object(map) => map.get(&token)?,
            Value::Array(items) => items.get(token.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

pub fn budget_for(spec: &RunnerSpec) -> RunBudget {
    RunBudget::new(
        probeforge_contracts::TargetLimits {
            max_requests_per_second: spec.limits.max_requests_per_second,
            max_concurrency: spec.limits.max_concurrency,
            max_duration_seconds: spec.limits.duration_seconds,
            max_requests: spec.limits.max_requests,
        },
        &spec.policy,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LoadCaps, build_runner_spec};
    use probe_forge_target_policy::TargetPolicy;
    use probeforge_contracts::{Environment, LoadProfile, LoadRequest, LoadScenario, TargetScope};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    struct MockServer {
        port: u16,
        requests: Arc<AtomicUsize>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn write_response(
        stream: &mut TcpStream,
        status: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(body);
        let _ = stream.write_all(response.as_bytes()).await;
    }

    async fn start_mock() -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut buffer = [0_u8; 4096];
                    let Ok(read) = stream.read(&mut buffer).await else {
                        return;
                    };
                    counter.fetch_add(1, Ordering::SeqCst);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    if request.starts_with("POST /api/session ") {
                        write_response(
                            &mut stream,
                            "200 OK",
                            "{\"status\":\"ok\"}",
                            &[("x-session-token", "fixture-token")],
                        )
                        .await;
                    } else if request.starts_with("GET /api/profile/fixture-token ") {
                        write_response(&mut stream, "200 OK", "{\"status\":\"ok\"}", &[]).await;
                    } else if request.starts_with("GET /api/slow ") {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        write_response(&mut stream, "200 OK", "{}", &[]).await;
                    } else {
                        write_response(&mut stream, "404 Not Found", "missing", &[]).await;
                    }
                });
            }
        });
        MockServer {
            port,
            requests,
            handle,
        }
    }

    fn scenario(_port: u16) -> LoadScenario {
        LoadScenario {
            name: "Local mock flow".to_owned(),
            description: "Local fixture; no real target.".to_owned(),
            requests: vec![
                LoadRequest {
                    id: Some("session".to_owned()),
                    method: probeforge_contracts::HttpMethod::Post,
                    path: "/api/session".to_owned(),
                    headers: BTreeMap::from([(
                        "Content-Type".to_owned(),
                        "application/json".to_owned(),
                    )]),
                    secret_headers: BTreeMap::new(),
                    depends_on: vec![],
                    extract: vec![LoadExtraction {
                        name: "token".to_owned(),
                        source: LoadExtractionSource::Header,
                        selector: "x-session-token".to_owned(),
                        required: true,
                    }],
                    assertions: vec![],
                    body_template: Some(serde_json::json!({"user": "fixture"})),
                    expected_statuses: vec![200],
                },
                LoadRequest {
                    id: Some("profile".to_owned()),
                    method: probeforge_contracts::HttpMethod::Get,
                    path: "/api/profile/${token}".to_owned(),
                    headers: BTreeMap::from([("X-Session".to_owned(), "${token}".to_owned())]),
                    secret_headers: BTreeMap::new(),
                    depends_on: vec!["session".to_owned()],
                    extract: vec![],
                    assertions: vec![LoadAssertion {
                        source: LoadAssertionSource::JsonBody,
                        selector: "/status".to_owned(),
                        operator: LoadAssertionOperator::Equals,
                        expected: Some(Value::String("ok".to_owned())),
                    }],
                    body_template: None,
                    expected_statuses: vec![200],
                },
            ],
            profile: LoadProfile {
                virtual_users: Some(2),
                arrival_rate_per_second: None,
                ramp_up_seconds: 0,
                duration_seconds: 1,
                max_requests: 20,
                max_requests_per_second: 20.0,
                stop_conditions: Default::default(),
            },
            credential_ref: None,
        }
    }

    #[tokio::test]
    async fn executes_correlated_steps_against_a_local_mock_only() {
        let server = start_mock().await;
        let scope = TargetScope {
            origin: format!("http://127.0.0.1:{}", server.port),
            allowed_hosts: vec!["127.0.0.1".to_owned()],
            allowed_paths: vec!["/".to_owned()],
            allowed_ports: vec![server.port],
        };
        let spec = build_runner_spec(
            &scenario(server.port),
            Environment::Local,
            &scope,
            &probeforge_contracts::TargetLimits {
                max_requests_per_second: 20.0,
                max_concurrency: 2,
                max_duration_seconds: 1,
                max_requests: 20,
            },
            &TargetPolicy::local_test(),
            LoadCaps::validation(),
        )
        .expect("spec");
        let budget = budget_for(&spec);
        let outcome = run_load(&spec, Arc::new(crate::NoSecretResolver), &budget)
            .await
            .expect("run");
        assert_eq!(outcome.status, RunStatus::Completed);
        assert_eq!(outcome.metrics.summary.error_count, 0);
        assert!(outcome.metrics.summary.request_count >= 2);
        assert!(server.requests.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn cancellation_stops_the_local_runner() {
        let server = start_mock().await;
        let scope = TargetScope {
            origin: format!("http://127.0.0.1:{}", server.port),
            allowed_hosts: vec!["127.0.0.1".to_owned()],
            allowed_paths: vec!["/".to_owned()],
            allowed_ports: vec![server.port],
        };
        let spec = build_runner_spec(
            &scenario(server.port),
            Environment::Local,
            &scope,
            &probeforge_contracts::TargetLimits {
                max_requests_per_second: 20.0,
                max_concurrency: 2,
                max_duration_seconds: 1,
                max_requests: 20,
            },
            &TargetPolicy::local_test(),
            LoadCaps::validation(),
        )
        .expect("spec");
        let budget = budget_for(&spec);
        budget.cancel();
        let outcome = run_load(&spec, Arc::new(crate::NoSecretResolver), &budget)
            .await
            .expect("run");
        assert_eq!(outcome.status, RunStatus::Cancelled);
    }

    #[tokio::test]
    async fn request_timeouts_are_captured_and_do_not_abort_the_run() {
        let server = start_mock().await;
        let scope = TargetScope {
            origin: format!("http://127.0.0.1:{}", server.port),
            allowed_hosts: vec!["127.0.0.1".to_owned()],
            allowed_paths: vec!["/".to_owned()],
            allowed_ports: vec![server.port],
        };
        let scenario = LoadScenario {
            name: "Timeout fixture".to_owned(),
            description: "Local mock that responds slower than the request timeout.".to_owned(),
            requests: vec![LoadRequest {
                id: Some("slow".to_owned()),
                method: probeforge_contracts::HttpMethod::Get,
                path: "/api/slow".to_owned(),
                headers: BTreeMap::new(),
                secret_headers: BTreeMap::new(),
                depends_on: vec![],
                extract: vec![],
                assertions: vec![],
                body_template: None,
                expected_statuses: vec![200],
            }],
            profile: LoadProfile {
                virtual_users: Some(1),
                arrival_rate_per_second: None,
                ramp_up_seconds: 0,
                duration_seconds: 1,
                max_requests: 1,
                max_requests_per_second: 1.0,
                stop_conditions: Default::default(),
            },
            credential_ref: None,
        };
        let policy = TargetPolicy {
            request_timeout: std::time::Duration::from_millis(50),
            ..TargetPolicy::local_test()
        };
        let spec = build_runner_spec(
            &scenario,
            Environment::Local,
            &scope,
            &probeforge_contracts::TargetLimits {
                max_requests_per_second: 1.0,
                max_concurrency: 1,
                max_duration_seconds: 1,
                max_requests: 1,
            },
            &policy,
            LoadCaps::validation(),
        )
        .expect("spec");
        let budget = budget_for(&spec);
        let outcome = run_load(&spec, Arc::new(crate::NoSecretResolver), &budget)
            .await
            .expect("run");
        assert_eq!(outcome.status, RunStatus::Completed);
        assert!(outcome.metrics.summary.timeout_count >= 1);
    }

    #[test]
    fn path_encoding_prevents_extracted_value_segment_escape() {
        let values = BTreeMap::from([("token".to_owned(), "a/b c".to_owned())]);
        assert_eq!(
            render_path("/api/${token}", &values).expect("path"),
            "/api/a%2Fb%20c"
        );
    }
}
