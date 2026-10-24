//! Target destination policy, DNS/IP validation, redirect enforcement, and
//! per-run request budgets.
//!
//! All checks are deterministic and server-owned. Neither model output nor
//! repository content can widen a scope or authorize a destination.

use std::{
    collections::BTreeSet,
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use probeforge_contracts::{
    Environment, HttpMethod, LoadScenario, TargetLimits, TargetScope, validate_target_limits,
    validate_target_scope,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, LOCATION};
use thiserror::Error;
use tokio::{
    net::lookup_host,
    sync::{Mutex, OwnedSemaphorePermit, Semaphore, watch},
    time::{Instant, sleep_until},
};
use url::{Host, Url};

pub const DEFAULT_AUTHORIZATION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
pub const MAX_AUTHORIZATION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;
pub const MAX_REDIRECTS: usize = 5;
pub const MAX_ALLOWED_PATHS: usize = 50;
pub const MAX_ALLOWED_HOSTS: usize = 20;
pub const MAX_ALLOWED_PORTS: usize = 10;

/// Ports that commonly expose administration, databases, caches, or cloud
/// metadata relays are never allowed in a target scope, even if explicitly
/// listed. Web ports and ordinary application ports remain available.
const SENSITIVE_PORTS: &[u16] = &[
    22, 23, 25, 53, 110, 143, 465, 587, 993, 995, 2375, 2376, 2379, 2380, 3306, 5432, 6379, 6443,
    9200, 9300, 10250, 11211, 27017,
];

const BLOCKED_V4: &[(u32, u32)] = &[
    (0x0000_0000, 0xff00_0000), // 0.0.0.0/8
    (0x0a00_0000, 0xff00_0000), // 10/8
    (0x6440_0000, 0xffc0_0000), // 100.64/10
    (0x7f00_0000, 0xff00_0000), // 127/8
    (0xa9fe_0000, 0xffff_0000), // 169.254/16
    (0xac10_0000, 0xfff0_0000), // 172.16/12
    (0xc000_0000, 0xffff_ff00), // 192.0.0/24
    (0xc000_0200, 0xffff_ff00), // 192.0.2/24
    (0xc058_6300, 0xffff_ff00), // 192.88.99/24
    (0xc0a8_0000, 0xffff_0000), // 192.168/16
    (0xc612_0000, 0xfffe_0000), // 198.18/15
    (0xc633_6400, 0xffff_ff00), // 198.51.100/24
    (0xcb00_7100, 0xffff_ff00), // 203.0.113/24
    (0xe000_0000, 0xf000_0000), // 224/4 multicast
    (0xf000_0000, 0xf000_0000), // 240/4 reserved
];

const PRIVATE_V4: &[(u32, u32)] = &[
    (0x0a00_0000, 0xff00_0000), // 10/8
    (0x6440_0000, 0xffc0_0000), // 100.64/10
    (0x7f00_0000, 0xff00_0000), // 127/8 loopback
    (0xac10_0000, 0xfff0_0000), // 172.16/12
    (0xc0a8_0000, 0xffff_0000), // 192.168/16
];

const METADATA_V4: &[Ipv4Addr] = &[
    Ipv4Addr::new(100, 100, 100, 200), // Alibaba Cloud
    Ipv4Addr::new(169, 254, 169, 254), // AWS/GCP/Azure IMDS
    Ipv4Addr::new(169, 254, 170, 2),   // ECS task metadata
    Ipv4Addr::new(192, 0, 0, 192),     // Oracle Cloud
    Ipv4Addr::new(168, 63, 129, 16),   // Azure platform virtual IP
];

#[derive(Debug, Clone)]
pub struct TargetPolicy {
    pub allow_private_networks: bool,
    pub request_timeout: Duration,
    pub max_response_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for TargetPolicy {
    fn default() -> Self {
        Self {
            allow_private_networks: false,
            request_timeout: Duration::from_secs(10),
            max_response_bytes: MAX_RESPONSE_BYTES,
            max_total_bytes: MAX_TOTAL_BYTES,
        }
    }
}

impl TargetPolicy {
    pub fn local_test() -> Self {
        Self {
            allow_private_networks: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TargetValidationError {
    #[error("target origin must use http or https")]
    InvalidScheme,
    #[error("target origin must not include credentials")]
    CredentialsNotAllowed,
    #[error("target origin must contain only scheme, host, and optional port")]
    OriginNotBare,
    #[error("target origin is malformed")]
    InvalidOrigin,
    #[error("target host is malformed")]
    InvalidHost,
    #[error("target port is not allowed")]
    PortNotAllowed,
    #[error("target path scope is malformed")]
    InvalidPathScope,
    #[error("target scope is invalid")]
    InvalidScope,
    #[error("target authorization limits are invalid")]
    InvalidLimits,
    #[error("production targets must use HTTPS")]
    InsecureProductionTarget,
    #[error("target must be verified before authorization")]
    TargetNotVerified,
    #[error("target authorization is expired or revoked")]
    AuthorizationInactive,
    #[error("destination resolves to a blocked address class")]
    UnsafeDestination,
    #[error("private-network targets are disabled by server policy")]
    PrivateNetworkDisabled,
    #[error("destination did not resolve to an address")]
    EmptyResolution,
    #[error("destination resolution failed")]
    DnsFailure,
    #[error("request is outside the approved target scope")]
    ScopeViolation,
    #[error("redirect would leave the approved target scope")]
    RedirectViolation,
    #[error("redirect would downgrade HTTPS to HTTP")]
    RedirectDowngrade,
    #[error("redirect limit exceeded")]
    TooManyRedirects,
    #[error("redirect response did not include a usable Location")]
    InvalidRedirect,
}

impl TargetValidationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidScheme
            | Self::CredentialsNotAllowed
            | Self::OriginNotBare
            | Self::InvalidOrigin => "invalid_target_origin",
            Self::InvalidHost => "invalid_target_host",
            Self::PortNotAllowed => "target_port_not_allowed",
            Self::InvalidPathScope => "invalid_path_scope",
            Self::InvalidScope => "invalid_target_scope",
            Self::InvalidLimits => "invalid_target_limits",
            Self::InsecureProductionTarget => "insecure_production_target",
            Self::TargetNotVerified => "target_not_verified",
            Self::AuthorizationInactive => "authorization_inactive",
            Self::UnsafeDestination => "unsafe_destination",
            Self::PrivateNetworkDisabled => "private_network_disabled",
            Self::EmptyResolution | Self::DnsFailure => "dns_resolution_failed",
            Self::ScopeViolation | Self::RedirectViolation => "target_scope_violation",
            Self::RedirectDowngrade => "redirect_downgrade",
            Self::TooManyRedirects | Self::InvalidRedirect => "invalid_redirect",
        }
    }

    pub fn safe_message(&self) -> &'static str {
        match self {
            Self::InvalidScheme => "Target origin must use http or https.",
            Self::CredentialsNotAllowed => "Target origins must not contain credentials.",
            Self::OriginNotBare => {
                "Target origin must not contain a path, query, fragment, or credentials."
            }
            Self::InvalidOrigin => "Target origin is not a valid URL.",
            Self::InvalidHost => "Target host is not a valid hostname or IP address.",
            Self::PortNotAllowed => "The target port is not permitted by policy.",
            Self::InvalidPathScope => "Allowed paths must be safe absolute path prefixes.",
            Self::InvalidScope => "Target scope is invalid.",
            Self::InvalidLimits => "Target authorization limits are outside policy.",
            Self::InsecureProductionTarget => "Production targets must use HTTPS.",
            Self::TargetNotVerified => "Validate the target before creating an authorization.",
            Self::AuthorizationInactive => "The target authorization is expired or revoked.",
            Self::UnsafeDestination => {
                "The destination resolves to a blocked private, link-local, or metadata address."
            }
            Self::PrivateNetworkDisabled => {
                "Private-network targets are disabled. An operator must enable the explicit private-target policy."
            }
            Self::EmptyResolution | Self::DnsFailure => {
                "The target host could not be resolved to an allowed address."
            }
            Self::ScopeViolation => "The request is outside the approved target scope.",
            Self::RedirectViolation => "The redirect destination is outside the approved scope.",
            Self::RedirectDowngrade => "HTTPS redirects may not downgrade to HTTP.",
            Self::TooManyRedirects => "The redirect limit was exceeded.",
            Self::InvalidRedirect => "The redirect response was not usable.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadScenarioScopeIssue {
    pub request_index: usize,
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone)]
pub struct TargetGuard {
    pub environment: Environment,
    pub scope: TargetScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDestination {
    pub host: String,
    pub port: u16,
    pub addresses: Vec<IpAddr>,
}

pub trait TargetResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, TargetValidationError>> + Send + 'a>>;
}

#[derive(Debug, Default)]
pub struct SystemResolver;

impl TargetResolver for SystemResolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, TargetValidationError>> + Send + 'a>> {
        Box::pin(async move {
            let addresses = lookup_host((host, port))
                .await
                .map_err(|_| TargetValidationError::DnsFailure)?;
            let mut unique = BTreeSet::new();
            for address in addresses {
                unique.insert(address.ip());
            }
            if unique.is_empty() {
                return Err(TargetValidationError::EmptyResolution);
            }
            Ok(unique.into_iter().collect())
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressClass {
    Public,
    Private,
    Loopback,
    LinkLocal,
    Metadata,
    Multicast,
    Unspecified,
    Documentation,
    Reserved,
}

impl AddressClass {
    pub fn is_public(self) -> bool {
        matches!(self, Self::Public)
    }

    pub fn is_private_network(self) -> bool {
        matches!(self, Self::Private | Self::Loopback)
    }
}

pub fn classify_ip(address: IpAddr) -> AddressClass {
    match address {
        IpAddr::V4(address) => classify_v4(address),
        IpAddr::V6(address) => classify_v6(address),
    }
}

fn classify_v4(address: Ipv4Addr) -> AddressClass {
    if METADATA_V4.contains(&address) {
        return AddressClass::Metadata;
    }
    let value = u32::from(address);
    if value == 0 {
        return AddressClass::Unspecified;
    }
    if PRIVATE_V4
        .iter()
        .any(|(network, mask)| value & mask == *network)
    {
        return AddressClass::Private;
    }
    if address.is_link_local() {
        return AddressClass::LinkLocal;
    }
    if address.is_multicast() {
        return AddressClass::Multicast;
    }
    if address.is_broadcast() {
        return AddressClass::Reserved;
    }
    if BLOCKED_V4
        .iter()
        .any(|(network, mask)| value & mask == *network)
    {
        return AddressClass::Reserved;
    }
    AddressClass::Public
}

fn classify_v6(address: Ipv6Addr) -> AddressClass {
    let segments = address.segments();
    if address.is_unspecified() {
        return AddressClass::Unspecified;
    }
    if address.is_loopback() {
        return AddressClass::Loopback;
    }
    if let Some(mapped) = address.to_ipv4_mapped() {
        return classify_v4(mapped);
    }
    // AWS IMDS over IPv6 and common link-local metadata aliases.
    if address == Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254)
        || address == Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0xa9fe, 0xa9fe)
    {
        return AddressClass::Metadata;
    }
    if address.is_multicast() || (segments[0] & 0xff00) == 0xff00 {
        return AddressClass::Multicast;
    }
    if (segments[0] & 0xffc0) == 0xfe80 {
        return AddressClass::LinkLocal;
    }
    if (segments[0] & 0xfe00) == 0xfc00 {
        return AddressClass::Private;
    }
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return AddressClass::Documentation;
    }
    if segments[0] == 0x2002
        || (segments[0] == 0x2001 && matches!(segments[1], 0x0000 | 0x0010 | 0x0020))
    {
        return AddressClass::Reserved;
    }
    // Deprecated IPv4-compatible and other low special-purpose ranges.
    if segments[0] == 0 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
        return AddressClass::Reserved;
    }
    AddressClass::Public
}

pub fn is_address_allowed(address: IpAddr, policy: &TargetPolicy) -> bool {
    match classify_ip(address) {
        AddressClass::Public => true,
        AddressClass::Private | AddressClass::Loopback => policy.allow_private_networks,
        AddressClass::Metadata
        | AddressClass::LinkLocal
        | AddressClass::Multicast
        | AddressClass::Unspecified
        | AddressClass::Documentation
        | AddressClass::Reserved => false,
    }
}

fn assert_address_allowed(
    address: IpAddr,
    policy: &TargetPolicy,
) -> Result<(), TargetValidationError> {
    if is_address_allowed(address, policy) {
        Ok(())
    } else if classify_ip(address).is_private_network() && !policy.allow_private_networks {
        Err(TargetValidationError::PrivateNetworkDisabled)
    } else {
        Err(TargetValidationError::UnsafeDestination)
    }
}

fn normalize_host(value: &str) -> Result<String, TargetValidationError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 253 {
        return Err(TargetValidationError::InvalidHost);
    }
    if value.starts_with('*') || value.contains('*') {
        return Err(TargetValidationError::InvalidHost);
    }
    let unbracketed = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    if let Ok(address) = unbracketed.parse::<IpAddr>() {
        return Ok(address.to_string());
    }
    let hostname = value.trim_end_matches('.').to_ascii_lowercase();
    if hostname.is_empty() || hostname.len() > 253 || hostname.contains("..") {
        return Err(TargetValidationError::InvalidHost);
    }
    for label in hostname.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err(TargetValidationError::InvalidHost);
        }
    }
    Ok(hostname)
}

fn normalize_path(value: &str) -> Result<String, TargetValidationError> {
    if !value.starts_with('/')
        || value.len() > 2_000
        || value.contains('\\')
        || value.contains("..")
        || value.contains("//")
        || value.chars().any(char::is_control)
        || value.to_ascii_lowercase().contains("%2e")
        || value.to_ascii_lowercase().contains("%5c")
    {
        return Err(TargetValidationError::InvalidPathScope);
    }
    if value == "/" {
        return Ok("/".to_owned());
    }
    Ok(value.trim_end_matches('/').to_owned())
}

fn normalize_origin(
    value: &str,
    environment: Environment,
) -> Result<(String, String, u16), TargetValidationError> {
    let parsed = Url::parse(value).map_err(|_| TargetValidationError::InvalidOrigin)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(TargetValidationError::InvalidScheme);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(TargetValidationError::CredentialsNotAllowed);
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(TargetValidationError::OriginNotBare);
    }
    let host = match parsed.host() {
        Some(Host::Ipv4(address)) => address.to_string(),
        Some(Host::Ipv6(address)) => address.to_string(),
        Some(Host::Domain(domain)) => normalize_host(domain)?,
        None => return Err(TargetValidationError::InvalidOrigin),
    };
    let port = parsed
        .port_or_known_default()
        .ok_or(TargetValidationError::InvalidOrigin)?;
    if port == 0 || SENSITIVE_PORTS.contains(&port) {
        return Err(TargetValidationError::PortNotAllowed);
    }
    if environment == Environment::Production && parsed.scheme() != "https" {
        return Err(TargetValidationError::InsecureProductionTarget);
    }
    let default_port =
        (parsed.scheme() == "https" && port == 443) || (parsed.scheme() == "http" && port == 80);
    let authority = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };
    let normalized = if default_port {
        format!("{}://{authority}", parsed.scheme())
    } else {
        format!("{}://{authority}:{port}", parsed.scheme())
    };
    Ok((normalized, host, port))
}

pub fn normalize_target_scope(
    origin: &str,
    environment: Environment,
    allowed_hosts: &[String],
    allowed_paths: &[String],
    allowed_ports: &[u16],
    policy: &TargetPolicy,
) -> Result<TargetScope, TargetValidationError> {
    let (normalized_origin, origin_host, origin_port) = normalize_origin(origin, environment)?;

    if let Ok(address) = origin_host.parse::<IpAddr>() {
        assert_address_allowed(address, policy)?;
    } else if origin_host == "localhost" && !policy.allow_private_networks {
        return Err(TargetValidationError::PrivateNetworkDisabled);
    }

    let mut hosts = BTreeSet::new();
    for host in allowed_hosts {
        hosts.insert(normalize_host(host)?);
    }
    if hosts.is_empty() {
        hosts.insert(origin_host.clone());
    }
    if hosts.len() > MAX_ALLOWED_HOSTS || !hosts.contains(&origin_host) {
        return Err(TargetValidationError::InvalidScope);
    }

    let mut paths = BTreeSet::new();
    for path in allowed_paths {
        paths.insert(normalize_path(path)?);
    }
    if paths.is_empty() || paths.len() > MAX_ALLOWED_PATHS {
        return Err(TargetValidationError::InvalidPathScope);
    }

    let mut ports = BTreeSet::new();
    for port in allowed_ports {
        if *port == 0 || SENSITIVE_PORTS.contains(port) {
            return Err(TargetValidationError::PortNotAllowed);
        }
        ports.insert(*port);
    }
    if ports.is_empty() {
        ports.insert(origin_port);
    }
    if ports.len() > MAX_ALLOWED_PORTS || !ports.contains(&origin_port) {
        return Err(TargetValidationError::InvalidScope);
    }

    let scope = TargetScope {
        origin: normalized_origin,
        allowed_hosts: hosts.into_iter().collect(),
        allowed_paths: paths.into_iter().collect(),
        allowed_ports: ports.into_iter().collect(),
    };
    validate_target_scope(&scope).map_err(|_| TargetValidationError::InvalidScope)?;
    Ok(scope)
}

fn path_in_scope(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return path.starts_with('/');
    }
    let prefix = prefix.trim_end_matches('/');
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn validate_authority(
    guard: &TargetGuard,
    value: &Url,
    policy: &TargetPolicy,
) -> Result<(String, u16), TargetValidationError> {
    if !matches!(value.scheme(), "http" | "https") {
        return Err(TargetValidationError::InvalidScheme);
    }
    if !value.username().is_empty() || value.password().is_some() {
        return Err(TargetValidationError::CredentialsNotAllowed);
    }
    if value.fragment().is_some() {
        return Err(TargetValidationError::ScopeViolation);
    }
    if guard.environment == Environment::Production && value.scheme() != "https" {
        return Err(TargetValidationError::InsecureProductionTarget);
    }
    let host = match value.host() {
        Some(Host::Ipv4(address)) => address.to_string(),
        Some(Host::Ipv6(address)) => address.to_string(),
        Some(Host::Domain(domain)) => normalize_host(domain)?,
        None => return Err(TargetValidationError::InvalidHost),
    };
    if !guard
        .scope
        .allowed_hosts
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&host))
    {
        return Err(TargetValidationError::ScopeViolation);
    }
    let port = value
        .port_or_known_default()
        .ok_or(TargetValidationError::PortNotAllowed)?;
    if port == 0 || SENSITIVE_PORTS.contains(&port) || !guard.scope.allowed_ports.contains(&port) {
        return Err(TargetValidationError::PortNotAllowed);
    }
    if let Ok(address) = host.parse::<IpAddr>() {
        assert_address_allowed(address, policy)?;
    }
    Ok((host, port))
}

pub fn validate_request_url(
    guard: &TargetGuard,
    value: &Url,
    policy: &TargetPolicy,
) -> Result<(), TargetValidationError> {
    validate_authority(guard, value, policy)?;
    let path = value.path();
    if path.contains('\\')
        || path.contains("..")
        || path.contains("//")
        || path.chars().any(char::is_control)
        || path.to_ascii_lowercase().contains("%2e")
        || path.to_ascii_lowercase().contains("%5c")
        || path.to_ascii_lowercase().contains("%2f")
        || path.to_ascii_lowercase().contains("%25")
        || path.to_ascii_lowercase().contains("%00")
        || path.to_ascii_lowercase().contains("%0a")
        || path.to_ascii_lowercase().contains("%0d")
        || !guard
            .scope
            .allowed_paths
            .iter()
            .any(|prefix| path_in_scope(path, prefix))
    {
        return Err(TargetValidationError::ScopeViolation);
    }
    Ok(())
}

/// Validates every load step against an already-approved target scope without
/// resolving DNS or sending traffic. Path template variables are expanded to a
/// harmless literal for the scope check; the Phase 12 runner must percent-encode
/// runtime values before substitution.
pub fn validate_load_scenario_scope(
    scenario: &LoadScenario,
    environment: Environment,
    scope: &TargetScope,
    policy: &TargetPolicy,
) -> Result<(), Vec<LoadScenarioScopeIssue>> {
    if validate_target_scope(scope).is_err() {
        return Err(vec![LoadScenarioScopeIssue {
            request_index: 0,
            code: "invalid_target_scope",
            message: "The target scope is invalid.",
        }]);
    }
    let Ok(origin) = Url::parse(&scope.origin) else {
        return Err(vec![LoadScenarioScopeIssue {
            request_index: 0,
            code: "invalid_target_origin",
            message: "The target origin is invalid.",
        }]);
    };
    let guard = TargetGuard {
        environment,
        scope: scope.clone(),
    };
    let mut issues = Vec::new();

    for (index, request) in scenario.requests.iter().enumerate() {
        let path = match expand_load_path(&request.path) {
            Ok(path) => path,
            Err(()) => {
                issues.push(LoadScenarioScopeIssue {
                    request_index: index,
                    code: "invalid_load_path",
                    message: "The load step path contains an invalid template.",
                });
                continue;
            }
        };
        let Ok(url) = origin.join(&path) else {
            issues.push(LoadScenarioScopeIssue {
                request_index: index,
                code: "invalid_load_path",
                message: "The load step path is not a valid URL path.",
            });
            continue;
        };
        if let Err(error) = validate_request_url(&guard, &url, policy) {
            issues.push(LoadScenarioScopeIssue {
                request_index: index,
                code: error.code(),
                message: error.safe_message(),
            });
        }
        if request.body_template.is_some()
            && matches!(
                request.method,
                HttpMethod::Get | HttpMethod::Head | HttpMethod::Options
            )
        {
            issues.push(LoadScenarioScopeIssue {
                request_index: index,
                code: "invalid_load_body",
                message: "GET, HEAD, and OPTIONS load steps must not include a body.",
            });
        }
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

fn expand_load_path(path: &str) -> Result<String, ()> {
    let mut expanded = String::with_capacity(path.len());
    let mut remainder = path;
    while let Some(start) = remainder.find("${") {
        expanded.push_str(&remainder[..start]);
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find('}') else {
            return Err(());
        };
        let name = &after_start[..end];
        let mut chars = name.chars();
        let Some(first) = chars.next() else {
            return Err(());
        };
        if name.len() > 64
            || !first.is_ascii_lowercase()
            || !chars.all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
            })
        {
            return Err(());
        }
        expanded.push_str("probe");
        remainder = &after_start[end + 1..];
    }
    expanded.push_str(remainder);
    Ok(expanded)
}

pub fn validate_redirect(
    guard: &TargetGuard,
    current: &Url,
    location: &str,
    policy: &TargetPolicy,
) -> Result<Url, TargetValidationError> {
    let next = current
        .join(location)
        .map_err(|_| TargetValidationError::InvalidRedirect)?;
    if current.scheme() == "https" && next.scheme() != "https" {
        return Err(TargetValidationError::RedirectDowngrade);
    }
    validate_request_url(guard, &next, policy).map_err(|error| match error {
        TargetValidationError::ScopeViolation
        | TargetValidationError::PortNotAllowed
        | TargetValidationError::InvalidScheme
        | TargetValidationError::InvalidHost => TargetValidationError::RedirectViolation,
        other => other,
    })?;
    Ok(next)
}

pub async fn resolve_destination(
    guard: &TargetGuard,
    resolver: &dyn TargetResolver,
    policy: &TargetPolicy,
) -> Result<ResolvedDestination, TargetValidationError> {
    let origin =
        Url::parse(&guard.scope.origin).map_err(|_| TargetValidationError::InvalidOrigin)?;
    let (host, port) = validate_authority(guard, &origin, policy)?;
    resolve_host(&host, port, resolver, policy).await
}

pub async fn resolve_url_destination(
    guard: &TargetGuard,
    value: &Url,
    resolver: &dyn TargetResolver,
    policy: &TargetPolicy,
) -> Result<ResolvedDestination, TargetValidationError> {
    validate_request_url(guard, value, policy)?;
    let (host, port) = validate_authority(guard, value, policy)?;
    resolve_host(&host, port, resolver, policy).await
}

async fn resolve_host(
    host: &str,
    port: u16,
    resolver: &dyn TargetResolver,
    policy: &TargetPolicy,
) -> Result<ResolvedDestination, TargetValidationError> {
    if let Ok(address) = host.parse::<IpAddr>() {
        assert_address_allowed(address, policy)?;
        return Ok(ResolvedDestination {
            host: host.to_owned(),
            port,
            addresses: vec![address],
        });
    }
    let mut addresses = resolver.resolve(host, port).await?;
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(TargetValidationError::EmptyResolution);
    }
    for address in &addresses {
        assert_address_allowed(*address, policy)?;
    }
    Ok(ResolvedDestination {
        host: host.to_owned(),
        port,
        addresses,
    })
}

pub fn validate_authorization_limits(limits: &TargetLimits) -> Result<(), TargetValidationError> {
    validate_target_limits(limits).map_err(|_| TargetValidationError::InvalidLimits)
}

pub fn authorization_expiry(
    requested: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<chrono::DateTime<chrono::Utc>, TargetValidationError> {
    let expiry = requested.unwrap_or_else(|| {
        now + chrono::Duration::from_std(DEFAULT_AUTHORIZATION_TTL)
            .expect("default authorization TTL is representable")
    });
    if expiry <= now {
        return Err(TargetValidationError::AuthorizationInactive);
    }
    let maximum = now
        + chrono::Duration::from_std(MAX_AUTHORIZATION_TTL)
            .expect("maximum authorization TTL is representable");
    if expiry > maximum {
        return Err(TargetValidationError::AuthorizationInactive);
    }
    Ok(expiry)
}

pub fn authorization_is_active(
    valid_from: chrono::DateTime<chrono::Utc>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    revoked_at.is_none()
        && valid_from <= now
        && expires_at.is_some_and(|expires_at| expires_at > now)
}

#[derive(Debug, Clone, Error)]
pub enum TargetRequestError {
    #[error("request budget is exhausted")]
    MaxRequests,
    #[error("request duration budget is exhausted")]
    DurationExceeded,
    #[error("response size budget is exhausted")]
    ResponseTooLarge,
    #[error("total data budget is exhausted")]
    TotalBytesExceeded,
    #[error("request was cancelled")]
    Cancelled,
    #[error("request failed at the network boundary")]
    Network,
    #[error("request timed out")]
    RequestTimeout,
    #[error("request destination is not allowed")]
    Validation(TargetValidationError),
    #[error("request headers are not allowed")]
    InvalidHeader,
    #[error("request body exceeds the platform cap")]
    RequestBodyTooLarge,
}

impl TargetRequestError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MaxRequests => "request_budget_exhausted",
            Self::DurationExceeded => "duration_budget_exhausted",
            Self::ResponseTooLarge => "response_size_exceeded",
            Self::TotalBytesExceeded => "total_data_budget_exhausted",
            Self::Cancelled => "cancelled",
            Self::Network => "network_error",
            Self::RequestTimeout => "request_timeout",
            Self::Validation(error) => error.code(),
            Self::InvalidHeader => "invalid_adapter_header",
            Self::RequestBodyTooLarge => "request_body_too_large",
        }
    }
}

impl From<TargetValidationError> for TargetRequestError {
    fn from(value: TargetValidationError) -> Self {
        Self::Validation(value)
    }
}

struct BudgetInner {
    semaphore: Arc<Semaphore>,
    last_request: Mutex<Option<Instant>>,
    requests: AtomicU64,
    total_bytes: AtomicU64,
    cancelled: AtomicBool,
    cancel_signal: watch::Sender<bool>,
    _cancel_state: watch::Receiver<bool>,
    deadline: Instant,
    limits: TargetLimits,
    max_response_bytes: u64,
    max_total_bytes: u64,
}

#[derive(Clone)]
pub struct RunBudget {
    inner: Arc<BudgetInner>,
}

pub struct BudgetPermit {
    _permit: OwnedSemaphorePermit,
}

impl RunBudget {
    pub fn new(limits: TargetLimits, policy: &TargetPolicy) -> Self {
        let deadline = Instant::now() + Duration::from_secs(u64::from(limits.max_duration_seconds));
        let (cancel_signal, cancel_state) = watch::channel(false);
        Self {
            inner: Arc::new(BudgetInner {
                semaphore: Arc::new(Semaphore::new(limits.max_concurrency as usize)),
                last_request: Mutex::new(None),
                requests: AtomicU64::new(0),
                total_bytes: AtomicU64::new(0),
                cancelled: AtomicBool::new(false),
                cancel_signal,
                _cancel_state: cancel_state,
                deadline,
                limits,
                max_response_bytes: policy.max_response_bytes,
                max_total_bytes: policy.max_total_bytes,
            }),
        }
    }

    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
        let _ = self.inner.cancel_signal.send(true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let mut receiver = self.inner.cancel_signal.subscribe();
        if *receiver.borrow() {
            return;
        }
        let _ = receiver.changed().await;
    }

    pub fn remaining(&self) -> Duration {
        self.inner
            .deadline
            .saturating_duration_since(Instant::now())
    }

    pub fn limits(&self) -> &TargetLimits {
        &self.inner.limits
    }

    pub async fn acquire(&self) -> Result<BudgetPermit, TargetRequestError> {
        if self.is_cancelled() {
            return Err(TargetRequestError::Cancelled);
        }
        let remaining = self.remaining();
        if remaining.is_zero() {
            return Err(TargetRequestError::DurationExceeded);
        }

        let interval = if self.inner.limits.max_requests_per_second > 0.0 {
            Duration::from_secs_f64(1.0 / self.inner.limits.max_requests_per_second)
        } else {
            Duration::ZERO
        };
        let scheduled = {
            let mut last = self.inner.last_request.lock().await;
            let now = Instant::now();
            let scheduled = match *last {
                Some(previous) => std::cmp::max(now, previous + interval),
                None => now,
            };
            *last = Some(scheduled);
            scheduled
        };
        tokio::select! {
            _ = self.cancelled() => return Err(TargetRequestError::Cancelled),
            result = tokio::time::timeout(self.remaining(), sleep_until(scheduled)) => {
                if result.is_err() {
                    return Err(TargetRequestError::DurationExceeded);
                }
            }
        }
        if self.remaining().is_zero() {
            return Err(TargetRequestError::DurationExceeded);
        }

        let permit = tokio::select! {
            _ = self.cancelled() => return Err(TargetRequestError::Cancelled),
            result = tokio::time::timeout(
                self.remaining(),
                self.inner.semaphore.clone().acquire_owned(),
            ) => result
                .map_err(|_| TargetRequestError::DurationExceeded)?
                .map_err(|_| TargetRequestError::Network)?,
        };
        if self.is_cancelled() {
            return Err(TargetRequestError::Cancelled);
        }
        let previous = self.inner.requests.fetch_add(1, Ordering::SeqCst);
        if previous >= self.inner.limits.max_requests {
            self.inner.requests.fetch_sub(1, Ordering::SeqCst);
            return Err(TargetRequestError::MaxRequests);
        }
        Ok(BudgetPermit { _permit: permit })
    }

    pub fn record_total_bytes(&self, bytes: u64) -> Result<(), TargetRequestError> {
        let total = self.inner.total_bytes.fetch_add(bytes, Ordering::SeqCst) + bytes;
        if total > self.inner.max_total_bytes {
            Err(TargetRequestError::TotalBytesExceeded)
        } else {
            Ok(())
        }
    }

    pub fn check_response_size(&self, bytes: u64) -> Result<(), TargetRequestError> {
        if bytes > self.inner.max_response_bytes {
            Err(TargetRequestError::ResponseTooLarge)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
}

impl SafeMethod {
    fn as_reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Head => reqwest::Method::HEAD,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
            Self::Options => reqwest::Method::OPTIONS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GuardedResponse {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
    pub final_url: Url,
    pub redirects: usize,
}

const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "proxy-authorization",
    "set-cookie",
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
];

fn map_reqwest_error(error: reqwest::Error) -> TargetRequestError {
    if error.is_timeout() {
        TargetRequestError::RequestTimeout
    } else {
        TargetRequestError::Network
    }
}

fn sanitize_headers(headers: &HeaderMap) -> Result<HeaderMap, TargetRequestError> {
    sanitize_headers_with_credentials(headers, false)
}

fn sanitize_load_headers(headers: &HeaderMap) -> Result<HeaderMap, TargetRequestError> {
    sanitize_headers_with_credentials(headers, true)
}

fn sanitize_headers_with_credentials(
    headers: &HeaderMap,
    allow_credentials: bool,
) -> Result<HeaderMap, TargetRequestError> {
    if headers.len() > 20 {
        return Err(TargetRequestError::InvalidHeader);
    }
    let mut sanitized = HeaderMap::new();
    for (name, value) in headers {
        let name_lower = name.as_str().to_ascii_lowercase();
        let forbidden = if allow_credentials {
            matches!(
                name_lower.as_str(),
                "proxy-authorization"
                    | "set-cookie"
                    | "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "keep-alive"
            )
        } else {
            SENSITIVE_HEADERS.contains(&name_lower.as_str())
        };
        if forbidden {
            return Err(TargetRequestError::InvalidHeader);
        }
        if value.as_bytes().len() > 1_024
            || value.as_bytes().iter().any(|byte| byte.is_ascii_control())
        {
            return Err(TargetRequestError::InvalidHeader);
        }
        let name = HeaderName::from_bytes(name.as_str().as_bytes())
            .map_err(|_| TargetRequestError::InvalidHeader)?;
        let value = HeaderValue::from_bytes(value.as_bytes())
            .map_err(|_| TargetRequestError::InvalidHeader)?;
        sanitized.append(name, value);
    }
    Ok(sanitized)
}

#[derive(Clone)]
pub struct GuardedHttpClient {
    policy: TargetPolicy,
    resolver: Arc<dyn TargetResolver>,
}

impl GuardedHttpClient {
    pub fn new(policy: TargetPolicy, resolver: Arc<dyn TargetResolver>) -> Self {
        Self { policy, resolver }
    }

    pub async fn get(
        &self,
        guard: &TargetGuard,
        url: &Url,
        budget: &RunBudget,
    ) -> Result<GuardedResponse, TargetRequestError> {
        self.get_with_headers(guard, url, &HeaderMap::new(), budget)
            .await
    }

    pub async fn get_with_headers(
        &self,
        guard: &TargetGuard,
        url: &Url,
        headers: &HeaderMap,
        budget: &RunBudget,
    ) -> Result<GuardedResponse, TargetRequestError> {
        let headers = sanitize_headers(headers)?;
        self.execute(guard, SafeMethod::Get, url, &headers, None, budget)
            .await
    }

    pub async fn head(
        &self,
        guard: &TargetGuard,
        url: &Url,
        budget: &RunBudget,
    ) -> Result<GuardedResponse, TargetRequestError> {
        let headers = sanitize_headers(&HeaderMap::new())?;
        self.execute(guard, SafeMethod::Head, url, &headers, None, budget)
            .await
    }

    /// Execute a validated load-runner request. This is the only client path
    /// that permits resolved credential headers; callers must resolve them from
    /// a server-owned secret provider and must never log them.
    pub async fn load_request(
        &self,
        guard: &TargetGuard,
        method: SafeMethod,
        url: &Url,
        headers: &HeaderMap,
        request_body: Option<&[u8]>,
        budget: &RunBudget,
    ) -> Result<GuardedResponse, TargetRequestError> {
        if request_body.is_some_and(|body| body.len() > MAX_REQUEST_BODY_BYTES) {
            return Err(TargetRequestError::RequestBodyTooLarge);
        }
        let headers = sanitize_load_headers(headers)?;
        self.execute(guard, method, url, &headers, request_body, budget)
            .await
    }

    async fn execute(
        &self,
        guard: &TargetGuard,
        method: SafeMethod,
        url: &Url,
        headers: &HeaderMap,
        request_body: Option<&[u8]>,
        budget: &RunBudget,
    ) -> Result<GuardedResponse, TargetRequestError> {
        let remaining = budget.remaining();
        if remaining.is_zero() {
            return Err(TargetRequestError::DurationExceeded);
        }
        tokio::time::timeout(
            remaining,
            self.execute_redirects(guard, method, url, headers, request_body, budget),
        )
        .await
        .map_err(|_| TargetRequestError::DurationExceeded)?
    }

    async fn execute_redirects(
        &self,
        guard: &TargetGuard,
        method: SafeMethod,
        url: &Url,
        headers: &HeaderMap,
        request_body: Option<&[u8]>,
        budget: &RunBudget,
    ) -> Result<GuardedResponse, TargetRequestError> {
        let mut current = url.clone();
        let mut redirects = 0;
        loop {
            let resolved =
                resolve_url_destination(guard, &current, self.resolver.as_ref(), &self.policy)
                    .await?;
            let _permit = budget.acquire().await?;

            let mut builder = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .pool_max_idle_per_host(0)
                .connect_timeout(self.policy.request_timeout)
                .timeout(self.policy.request_timeout)
                .user_agent("ProbeForge/0.1");
            if !resolved.host.parse::<IpAddr>().is_ok() {
                let addresses: Vec<SocketAddr> = resolved
                    .addresses
                    .iter()
                    .map(|address| SocketAddr::new(*address, resolved.port))
                    .collect();
                builder = builder.resolve_to_addrs(&resolved.host, &addresses);
            }
            let client = builder.build().map_err(|_| TargetRequestError::Network)?;
            let mut request = client
                .request(method.as_reqwest(), current.clone())
                .headers(headers.clone());
            if let Some(request_body) = request_body {
                request = request.body(request_body.to_vec());
            }
            let response = tokio::select! {
                _ = budget.cancelled() => return Err(TargetRequestError::Cancelled),
                result = request.send() => result.map_err(map_reqwest_error)?,
            };

            if response.status().is_redirection() {
                if redirects >= MAX_REDIRECTS {
                    return Err(TargetRequestError::Validation(
                        TargetValidationError::TooManyRedirects,
                    ));
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(TargetRequestError::Validation(
                        TargetValidationError::InvalidRedirect,
                    ))?;
                current = validate_redirect(guard, &current, location, &self.policy)?;
                redirects += 1;
                continue;
            }

            let status = response.status().as_u16();
            let headers = response.headers().clone();
            if let Some(length) = response.content_length()
                && length > self.policy.max_response_bytes
            {
                return Err(TargetRequestError::ResponseTooLarge);
            }
            let mut response = response;
            let mut body = Vec::new();
            loop {
                let chunk = tokio::select! {
                    _ = budget.cancelled() => return Err(TargetRequestError::Cancelled),
                    result = response.chunk() => result.map_err(map_reqwest_error)?,
                };
                let Some(chunk) = chunk else {
                    break;
                };
                if body.len() as u64 + chunk.len() as u64 > self.policy.max_response_bytes {
                    return Err(TargetRequestError::ResponseTooLarge);
                }
                budget.record_total_bytes(chunk.len() as u64)?;
                body.extend_from_slice(&chunk);
            }
            budget.check_response_size(body.len() as u64)?;
            return Ok(GuardedResponse {
                status,
                headers,
                body,
                final_url: current,
                redirects,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use probeforge_contracts::{LoadProfile, LoadRequest};
    use std::{collections::BTreeMap, sync::atomic::AtomicUsize};

    struct StaticResolver(Vec<IpAddr>);

    impl TargetResolver for StaticResolver {
        fn resolve<'a>(
            &'a self,
            _host: &'a str,
            _port: u16,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, TargetValidationError>> + Send + 'a>>
        {
            let addresses = self.0.clone();
            Box::pin(async move { Ok(addresses) })
        }
    }

    fn scope(origin: &str, paths: &[&str], ports: &[u16]) -> TargetScope {
        TargetScope {
            origin: origin.to_owned(),
            allowed_hosts: vec![
                Url::parse(origin)
                    .expect("origin")
                    .host_str()
                    .expect("host")
                    .to_owned(),
            ],
            allowed_paths: paths.iter().map(|path| (*path).to_owned()).collect(),
            allowed_ports: ports.to_vec(),
        }
    }

    fn load_request(method: HttpMethod, path: &str) -> LoadRequest {
        LoadRequest {
            id: Some("step".to_owned()),
            method,
            path: path.to_owned(),
            headers: BTreeMap::new(),
            secret_headers: BTreeMap::new(),
            depends_on: Vec::new(),
            extract: Vec::new(),
            assertions: Vec::new(),
            body_template: None,
            expected_statuses: vec![200],
        }
    }

    fn load_scenario(requests: Vec<LoadRequest>) -> LoadScenario {
        LoadScenario {
            name: "Scope fixture".to_owned(),
            description: "Bounded fixture; no traffic is sent.".to_owned(),
            requests,
            profile: LoadProfile {
                virtual_users: Some(2),
                arrival_rate_per_second: None,
                ramp_up_seconds: 5,
                duration_seconds: 30,
                max_requests: 200,
                max_requests_per_second: 2.0,
                stop_conditions: Default::default(),
            },
            credential_ref: None,
        }
    }

    #[test]
    fn load_scenario_scope_checks_every_step_without_network_access() {
        let target_scope = scope("https://api.example.test", &["/api"], &[443]);
        let scenario = load_scenario(vec![
            load_request(HttpMethod::Get, "/api/items"),
            load_request(HttpMethod::Get, "/api/items/${item_id}"),
        ]);
        assert!(
            validate_load_scenario_scope(
                &scenario,
                Environment::Staging,
                &target_scope,
                &TargetPolicy::default(),
            )
            .is_ok()
        );

        let outside = load_scenario(vec![load_request(HttpMethod::Get, "/admin")]);
        let issues = validate_load_scenario_scope(
            &outside,
            Environment::Staging,
            &target_scope,
            &TargetPolicy::default(),
        )
        .expect_err("out-of-scope path must fail");
        assert_eq!(issues[0].code, "target_scope_violation");

        let expression = load_scenario(vec![load_request(
            HttpMethod::Get,
            "/api/${process.env.SECRET}",
        )]);
        let issues = validate_load_scenario_scope(
            &expression,
            Environment::Staging,
            &target_scope,
            &TargetPolicy::default(),
        )
        .expect_err("invalid template must fail");
        assert_eq!(issues[0].code, "invalid_load_path");

        let mut body_request = load_request(HttpMethod::Get, "/api/items");
        body_request.body_template = Some(serde_json::json!({"query": "fixture"}));
        let issues = validate_load_scenario_scope(
            &load_scenario(vec![body_request]),
            Environment::Staging,
            &target_scope,
            &TargetPolicy::default(),
        )
        .expect_err("GET body must fail");
        assert_eq!(issues[0].code, "invalid_load_body");

        let insecure = scope("http://api.example.test", &["/api"], &[80]);
        let issues = validate_load_scenario_scope(
            &load_scenario(vec![load_request(HttpMethod::Get, "/api/items")]),
            Environment::Production,
            &insecure,
            &TargetPolicy::default(),
        )
        .expect_err("production HTTP must fail");
        assert_eq!(issues[0].code, "insecure_production_target");
    }

    #[test]
    fn classifies_private_link_local_metadata_and_reserved_ranges() {
        for address in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "169.254.1.1",
            "169.254.169.254",
            "100.100.100.200",
            "192.0.0.192",
            "168.63.129.16",
            "224.0.0.1",
            "240.0.0.1",
            "0.0.0.0",
        ] {
            assert!(
                !is_address_allowed(address.parse().expect("ip"), &TargetPolicy::default()),
                "{address} should be blocked"
            );
        }
        for address in ["::1", "fc00::1", "fe80::1", "fd00:ec2::254", "2001:db8::1"] {
            assert!(
                !is_address_allowed(address.parse().expect("ip"), &TargetPolicy::default()),
                "{address} should be blocked"
            );
        }
        assert!(is_address_allowed(
            "93.184.216.34".parse().expect("ip"),
            &TargetPolicy::default()
        ));
        assert!(is_address_allowed(
            "2606:4700:4700::1111".parse().expect("ip"),
            &TargetPolicy::default()
        ));
    }

    #[test]
    fn private_mode_allows_loopback_and_rfc1918_but_not_metadata_or_link_local() {
        let policy = TargetPolicy::local_test();
        assert!(is_address_allowed(
            "127.0.0.1".parse().expect("ip"),
            &policy
        ));
        assert!(is_address_allowed("10.0.0.5".parse().expect("ip"), &policy));
        assert!(!is_address_allowed(
            "169.254.169.254".parse().expect("ip"),
            &policy
        ));
        assert!(!is_address_allowed("fe80::1".parse().expect("ip"), &policy));
        assert!(!is_address_allowed(
            "fd00:ec2::254".parse().expect("ip"),
            &policy
        ));
    }

    #[test]
    fn normalizes_origin_and_rejects_unsafe_scope_inputs() {
        let policy = TargetPolicy::default();
        let scope = normalize_target_scope(
            "https://API.Example.test:443/",
            Environment::Staging,
            &["api.example.test".to_owned()],
            &["/api/".to_owned()],
            &[443],
            &policy,
        )
        .expect("valid scope");
        assert_eq!(scope.origin, "https://api.example.test");
        assert_eq!(scope.allowed_paths, vec!["/api"]);

        let ipv6 = normalize_target_scope(
            "https://[::1]:8443/",
            Environment::Staging,
            &["::1".to_owned()],
            &["/".to_owned()],
            &[8443],
            &TargetPolicy::local_test(),
        )
        .expect("IPv6 loopback scope under explicit private policy");
        assert_eq!(ipv6.origin, "https://[::1]:8443");
        assert_eq!(ipv6.allowed_hosts, vec!["::1"]);

        for (origin, hosts, paths, ports, expected) in [
            (
                "ftp://example.test",
                vec!["example.test".to_owned()],
                vec!["/".to_owned()],
                vec![21],
                TargetValidationError::InvalidScheme,
            ),
            (
                "https://user:pass@example.test",
                vec!["example.test".to_owned()],
                vec!["/".to_owned()],
                vec![443],
                TargetValidationError::CredentialsNotAllowed,
            ),
            (
                "https://example.test/api",
                vec!["example.test".to_owned()],
                vec!["/".to_owned()],
                vec![443],
                TargetValidationError::OriginNotBare,
            ),
            (
                "https://example.test",
                vec!["example.test".to_owned()],
                vec!["/../admin".to_owned()],
                vec![443],
                TargetValidationError::InvalidPathScope,
            ),
            (
                "https://example.test",
                vec!["example.test".to_owned()],
                vec!["/".to_owned()],
                vec![22],
                TargetValidationError::PortNotAllowed,
            ),
        ] {
            let result = normalize_target_scope(
                origin,
                Environment::Staging,
                &hosts,
                &paths,
                &ports,
                &policy,
            );
            assert_eq!(result, Err(expected));
        }
    }

    #[test]
    fn rejects_scope_escapes_and_redirect_escapes() {
        let guard = TargetGuard {
            environment: Environment::Staging,
            scope: scope("https://api.example.test", &["/api"], &[443]),
        };
        let policy = TargetPolicy::default();
        assert!(
            validate_request_url(
                &guard,
                &Url::parse("https://api.example.test/api/items").expect("url"),
                &policy
            )
            .is_ok()
        );
        for value in [
            "https://api.example.test/admin",
            "https://other.example.test/api/items",
            "https://api.example.test:8443/api/items",
            "http://api.example.test/api/items",
            "https://api.example.test/api/%2e%2e/admin",
            "https://api.example.test/api/%252e%252e/admin",
            "https://api.example.test/api/%00/admin",
        ] {
            assert!(
                validate_request_url(&guard, &Url::parse(value).expect("url"), &policy).is_err(),
                "{value} should be rejected"
            );
        }
        let current = Url::parse("https://api.example.test/api/start").expect("url");
        assert!(validate_redirect(&guard, &current, "/api/final", &policy).is_ok());
        assert_eq!(
            validate_redirect(&guard, &current, "https://api.example.test/admin", &policy),
            Err(TargetValidationError::RedirectViolation)
        );
        assert_eq!(
            validate_redirect(
                &TargetGuard {
                    environment: Environment::Staging,
                    scope: TargetScope {
                        origin: "https://api.example.test".to_owned(),
                        allowed_hosts: vec!["api.example.test".to_owned()],
                        allowed_paths: vec!["/".to_owned()],
                        allowed_ports: vec![80, 443],
                    },
                },
                &current,
                "http://api.example.test/api/final",
                &policy
            ),
            Err(TargetValidationError::RedirectDowngrade)
        );
    }

    #[test]
    fn rejects_a_dns_answer_that_changes_to_a_blocked_address() {
        let guard = TargetGuard {
            environment: Environment::Staging,
            scope: scope("https://api.example.test", &["/"], &[443]),
        };
        let public = StaticResolver(vec!["93.184.216.34".parse().expect("ip")]);
        let rebound = StaticResolver(vec!["127.0.0.1".parse().expect("ip")]);
        let policy = TargetPolicy::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            assert!(resolve_destination(&guard, &public, &policy).await.is_ok());
            assert_eq!(
                resolve_destination(&guard, &rebound, &policy).await,
                Err(TargetValidationError::PrivateNetworkDisabled)
            );
        });
    }

    #[tokio::test]
    async fn run_budget_enforces_request_count_bytes_duration_and_cancellation() {
        let limits = TargetLimits {
            max_requests_per_second: 100.0,
            max_concurrency: 1,
            max_duration_seconds: 1,
            max_requests: 2,
        };
        let budget = RunBudget::new(limits, &TargetPolicy::default());
        let first = budget.acquire().await.expect("first request");
        drop(first);
        let second = budget.acquire().await.expect("second request");
        drop(second);
        assert!(matches!(
            budget.acquire().await,
            Err(TargetRequestError::MaxRequests)
        ));
        assert!(matches!(
            budget.check_response_size(MAX_RESPONSE_BYTES + 1),
            Err(TargetRequestError::ResponseTooLarge)
        ));
        assert!(matches!(
            budget.record_total_bytes(MAX_TOTAL_BYTES + 1),
            Err(TargetRequestError::TotalBytesExceeded)
        ));
        budget.cancel();
        assert!(budget.is_cancelled());
        assert!(matches!(
            budget.acquire().await,
            Err(TargetRequestError::Cancelled)
        ));

        let concurrency_limits = TargetLimits {
            max_requests_per_second: 100.0,
            max_concurrency: 1,
            max_duration_seconds: 5,
            max_requests: 10,
        };
        let concurrency = RunBudget::new(concurrency_limits, &TargetPolicy::default());
        let held = concurrency
            .acquire()
            .await
            .expect("first concurrent request");
        let waiting = tokio::time::timeout(Duration::from_millis(50), concurrency.acquire()).await;
        assert!(
            waiting.is_err(),
            "concurrency limit should queue the second request"
        );
        drop(held);
        let after = concurrency.acquire().await.expect("permit after release");
        drop(after);

        let short_limits = TargetLimits {
            max_requests_per_second: 100.0,
            max_concurrency: 1,
            max_duration_seconds: 1,
            max_requests: 10,
        };
        let short = RunBudget::new(short_limits, &TargetPolicy::default());
        let _held = short.acquire().await.expect("initial request");
        tokio::time::sleep(Duration::from_millis(1_050)).await;
        assert!(matches!(
            short.acquire().await,
            Err(TargetRequestError::DurationExceeded)
        ));
    }

    #[test]
    fn rejects_credential_and_hop_by_hop_request_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );
        assert!(matches!(
            sanitize_headers(&headers),
            Err(TargetRequestError::InvalidHeader)
        ));

        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::COOKIE,
            HeaderValue::from_static("session=secret"),
        );
        assert!(matches!(
            sanitize_headers(&headers),
            Err(TargetRequestError::InvalidHeader)
        ));

        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::ORIGIN,
            HeaderValue::from_static("https://probe.invalid"),
        );
        assert!(sanitize_headers(&headers).is_ok());
    }

    #[tokio::test]
    async fn guarded_client_follows_safe_redirects_and_blocks_scope_escapes() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server bind");
        let port = listener.local_addr().expect("mock address").port();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_server = hits.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let hits = hits_for_server.clone();
                tokio::spawn(async move {
                    let mut request = [0_u8; 2048];
                    let Ok(read) = socket.read(&mut request).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&request[..read]);
                    hits.fetch_add(1, Ordering::SeqCst);
                    let response = if request.starts_with("GET /start ") {
                        "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                    } else if request.starts_with("GET /final ") {
                        "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello"
                            .to_owned()
                    } else {
                        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                            .to_owned()
                    };
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });

        let origin = format!("http://127.0.0.1:{port}");
        let guard = TargetGuard {
            environment: Environment::Local,
            scope: TargetScope {
                origin: origin.clone(),
                allowed_hosts: vec!["127.0.0.1".to_owned()],
                allowed_paths: vec!["/start".to_owned(), "/final".to_owned()],
                allowed_ports: vec![port],
            },
        };
        let policy = TargetPolicy::local_test();
        let client = GuardedHttpClient::new(policy.clone(), Arc::new(SystemResolver));
        let limits = TargetLimits {
            max_requests_per_second: 100.0,
            max_concurrency: 1,
            max_duration_seconds: 5,
            max_requests: 10,
        };
        let budget = RunBudget::new(limits.clone(), &policy);
        let response = client
            .get(
                &guard,
                &Url::parse(&format!("{origin}/start")).expect("url"),
                &budget,
            )
            .await
            .expect("safe response");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        assert_eq!(response.redirects, 1);
        assert_eq!(hits.load(Ordering::SeqCst), 2);

        let budget = RunBudget::new(limits, &policy);
        let error = client
            .get(
                &guard,
                &Url::parse(&format!("{origin}/final/../admin")).expect("url"),
                &budget,
            )
            .await
            .expect_err("scope escape must fail");
        assert!(matches!(
            error,
            TargetRequestError::Validation(TargetValidationError::ScopeViolation)
        ));
        server.abort();
    }
}
