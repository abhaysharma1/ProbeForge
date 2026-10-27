//! Bounded static scanning for pinned repository snapshots.
//!
//! The scanner walks a server-owned workspace and reports **redacted** credential
//! indicators. It never returns a matched secret value, never follows symlinks,
//! and never executes repository content. All matching is implemented with
//! literal prefix/shape checks and Shannon entropy, so there is no regular
//! expression engine to exploit (no ReDoS surface).

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// Default hard limits. Callers may lower these but should not raise them
/// without a documented policy change.
#[derive(Debug, Clone, Copy)]
pub struct ScanLimits {
    pub max_files: u32,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_findings: u32,
    pub max_depth: u32,
    pub max_findings_per_file: u32,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_file_bytes: 1_048_576,
            max_total_bytes: 64 * 1024 * 1024,
            max_findings: 500,
            max_depth: 32,
            max_findings_per_file: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecretFinding {
    pub rule_id: &'static str,
    pub pattern_name: &'static str,
    /// Repository-relative path with forward slashes.
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub confidence: f32,
    pub entropy: f32,
    /// Line context where the matched span is replaced by `***`.
    pub redacted_preview: String,
    pub value_length: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanReport {
    pub files_scanned: u32,
    pub bytes_scanned: u64,
    pub skipped_files: u32,
    pub truncated: bool,
    pub cancelled: bool,
    pub findings: Vec<SecretFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    InvalidRoot,
    RootNotDirectory,
    UnreadableRoot,
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRoot => write!(formatter, "scan root must be an absolute directory"),
            Self::RootNotDirectory => write!(formatter, "scan root is not a directory"),
            Self::UnreadableRoot => write!(formatter, "scan root could not be read"),
        }
    }
}

impl std::error::Error for ScanError {}

const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "vendor",
    "dist",
    "build",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".nuxt",
    "coverage",
];

const SKIPPED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "ico", "webp", "bmp", "pdf", "zip", "gz", "tgz", "bz2", "7z",
    "rar", "jar", "class", "so", "dylib", "dll", "exe", "bin", "woff", "woff2", "ttf", "otf",
    "eot", "mp3", "mp4", "mov", "avi", "wav", "webm", "wasm", "sqlite", "db",
];

const GENERIC_SECRET_NAMES: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "access-key",
    "private_key",
    "private-key",
    "client_secret",
    "client-secret",
    "auth_token",
    "auth-token",
];

const PLACEHOLDER_MARKERS: &[&str] = &[
    "example",
    "changeme",
    "change_me",
    "change-me",
    "your_",
    "your-",
    "placeholder",
    "redacted",
    "dummy",
    "fake",
    "sample",
    "todo",
    "replace_me",
    "replace-me",
    "notasecret",
    "***",
    "<",
    ">",
];

struct Rule {
    id: &'static str,
    name: &'static str,
    confidence: f32,
}

const RULE_PRIVATE_KEY: Rule = Rule {
    id: "PF-SEC-001",
    name: "private_key_block",
    confidence: 0.95,
};
const RULE_AWS_ACCESS_KEY: Rule = Rule {
    id: "PF-SEC-002",
    name: "aws_access_key_id",
    confidence: 0.95,
};
const RULE_GITHUB_TOKEN: Rule = Rule {
    id: "PF-SEC-003",
    name: "github_token",
    confidence: 0.92,
};
const RULE_SLACK_TOKEN: Rule = Rule {
    id: "PF-SEC-004",
    name: "slack_token",
    confidence: 0.9,
};
const RULE_STRIPE_KEY: Rule = Rule {
    id: "PF-SEC-005",
    name: "stripe_secret_key",
    confidence: 0.9,
};
const RULE_GOOGLE_API_KEY: Rule = Rule {
    id: "PF-SEC-006",
    name: "google_api_key",
    confidence: 0.9,
};
const RULE_GENERIC_ASSIGNMENT: Rule = Rule {
    id: "PF-SEC-100",
    name: "generic_credential_assignment",
    confidence: 0.55,
};

fn spans_overlap(spans: &[(usize, usize)], start: usize, end: usize) -> bool {
    spans
        .iter()
        .any(|(existing_start, existing_end)| start < *existing_end && end > *existing_start)
}

fn is_delimiter(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '"' | '\''
                | '`'
                | '='
                | ':'
                | ','
                | ';'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '|'
                | '&'
                | '/'
                | '\\'
        )
}

fn shannon_entropy(value: &str) -> f32 {
    if value.is_empty() {
        return 0.0;
    }
    let mut counts = [0_u32; 256];
    for byte in value.bytes() {
        counts[byte as usize] += 1;
    }
    let length = value.len() as f32;
    let mut entropy = 0.0;
    for count in counts {
        if count == 0 {
            continue;
        }
        let probability = count as f32 / length;
        entropy -= probability * probability.log2();
    }
    entropy
}

fn has_placeholder_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    PLACEHOLDER_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

fn looks_like_aws_access_key(token: &str) -> bool {
    (token.starts_with("AKIA") || token.starts_with("ASIA"))
        && token.len() == 20
        && token
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

fn looks_like_github_token(token: &str) -> bool {
    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| {
            token
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.len() >= 30 && is_alphanumeric(rest))
        })
}

fn looks_like_slack_token(token: &str) -> bool {
    ["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"]
        .iter()
        .any(|prefix| {
            token.strip_prefix(prefix).is_some_and(|rest| {
                rest.len() >= 10
                    && rest
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '-')
            })
        })
}

fn looks_like_stripe_key(token: &str) -> bool {
    ["sk_live_", "rk_live_"].iter().any(|prefix| {
        token
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.len() >= 20 && is_alphanumeric(rest))
    })
}

fn looks_like_google_api_key(token: &str) -> bool {
    token
        .strip_prefix("AIza")
        .is_some_and(|rest| rest.len() == 35 && is_alphanumeric_underscore(rest))
}

fn is_alphanumeric(value: &str) -> bool {
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric())
}

fn is_alphanumeric_underscore(value: &str) -> bool {
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
}

fn redact_line(line: &str, start: usize, end: usize) -> String {
    redact_line_with_spans(line, &[(start, end)])
}

/// Masks every matched span on a line. Findings on a line are reported together
/// so a preview never exposes a second credential that also matched.
fn redact_line_with_spans(line: &str, spans: &[(usize, usize)]) -> String {
    let mut spans: Vec<(usize, usize)> = spans
        .iter()
        .copied()
        .filter(|(start, end)| start <= end && *end <= line.len())
        .collect();
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some((_, previous_end)) if start <= *previous_end => {
                *previous_end = (*previous_end).max(end);
            }
            _ => merged.push((start, end)),
        }
    }
    let mut preview = String::with_capacity(line.len().min(200) + 8);
    let mut cursor = 0;
    for (start, end) in merged {
        if start > cursor {
            preview.push_str(&line[cursor..start]);
        }
        preview.push_str("***");
        cursor = end;
    }
    if cursor < line.len() {
        preview.push_str(&line[cursor..]);
    }
    let trimmed = preview.trim();
    let bounded: String = trimmed.chars().take(160).collect();
    if bounded.is_empty() {
        "(redacted)".to_owned()
    } else {
        bounded
    }
}

fn push_finding(
    report: &mut ScanReport,
    limits: &ScanLimits,
    per_file: &mut u32,
    finding: SecretFinding,
) -> bool {
    if report.findings.len() >= limits.max_findings as usize {
        report.truncated = true;
        return false;
    }
    if *per_file >= limits.max_findings_per_file {
        report.truncated = true;
        return true;
    }
    *per_file += 1;
    report.findings.push(finding);
    true
}

/// Scans a single line of text, appending redacted findings.
fn scan_line(
    relative_path: &str,
    line_number: u32,
    line: &str,
    report: &mut ScanReport,
    limits: &ScanLimits,
    per_file: &mut u32,
) {
    let mut reported_spans: Vec<(usize, usize)> = Vec::new();
    let findings_before = report.findings.len();

    // Private key blocks are a whole-line signal; the block body is never read.
    if line.contains("-----BEGIN ") && line.contains("PRIVATE KEY-----") {
        let start = line.find("-----BEGIN ").unwrap_or(0);
        let end = line
            .rfind("PRIVATE KEY-----")
            .map_or(line.len(), |index| index + 15);
        if end <= line.len() && !spans_overlap(&reported_spans, start, end) {
            reported_spans.push((start, end));
            let _ = push_finding(
                report,
                limits,
                per_file,
                SecretFinding {
                    rule_id: RULE_PRIVATE_KEY.id,
                    pattern_name: RULE_PRIVATE_KEY.name,
                    file_path: relative_path.to_owned(),
                    line: line_number,
                    column: (start + 1) as u32,
                    confidence: RULE_PRIVATE_KEY.confidence,
                    entropy: 0.0,
                    redacted_preview: redact_line(line, start, end),
                    value_length: 32,
                },
            );
        }
    }

    // Extract token spans delimited by common punctuation and whitespace.
    let mut token_start: Option<usize> = None;
    let mut index = 0;
    while index <= line.len() {
        let at_end = index == line.len();
        let character = if at_end {
            '\0'
        } else {
            line[index..].chars().next().unwrap_or('\0')
        };
        let width = if at_end { 1 } else { character.len_utf8() };
        let delimiter = at_end || is_delimiter(character);
        if delimiter {
            if let Some(start) = token_start.take()
                && start < index
            {
                inspect_token(
                    relative_path,
                    line_number,
                    line,
                    start,
                    index,
                    report,
                    limits,
                    per_file,
                    &mut reported_spans,
                );
            }
        } else if token_start.is_none() {
            token_start = Some(index);
        }
        if at_end {
            break;
        }
        index += width;
    }

    // Generic `name = value` assignments that are not covered by a token rule.
    if let Some((key, value, value_start, value_end)) = parse_assignment(line) {
        let key_lower = key.to_ascii_lowercase();
        let key_matches = GENERIC_SECRET_NAMES
            .iter()
            .any(|name| key_lower.contains(name));
        if key_matches && value.len() >= 8 && !has_placeholder_marker(value) {
            let entropy = shannon_entropy(value);
            if entropy >= 3.0 && !spans_overlap(&reported_spans, value_start, value_end) {
                let confidence = if entropy >= 4.0 { 0.75 } else { 0.65 };
                reported_spans.push((value_start, value_end));
                let _ = push_finding(
                    report,
                    limits,
                    per_file,
                    SecretFinding {
                        rule_id: RULE_GENERIC_ASSIGNMENT.id,
                        pattern_name: RULE_GENERIC_ASSIGNMENT.name,
                        file_path: relative_path.to_owned(),
                        line: line_number,
                        column: (value_start + 1) as u32,
                        confidence,
                        entropy,
                        redacted_preview: redact_line(line, value_start, value_end),
                        value_length: value.len() as u32,
                    },
                );
            }
        }
    }

    if report.findings.len() > findings_before {
        let preview = redact_line_with_spans(line, &reported_spans);
        for finding in &mut report.findings[findings_before..] {
            finding.redacted_preview.clone_from(&preview);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn inspect_token(
    relative_path: &str,
    line_number: u32,
    line: &str,
    start: usize,
    end: usize,
    report: &mut ScanReport,
    limits: &ScanLimits,
    per_file: &mut u32,
    reported_spans: &mut Vec<(usize, usize)>,
) {
    let token = &line[start..end];
    if token.len() < 8 || has_placeholder_marker(token) {
        return;
    }
    let rule = if looks_like_aws_access_key(token) {
        RULE_AWS_ACCESS_KEY
    } else if looks_like_github_token(token) {
        RULE_GITHUB_TOKEN
    } else if looks_like_slack_token(token) {
        RULE_SLACK_TOKEN
    } else if looks_like_stripe_key(token) {
        RULE_STRIPE_KEY
    } else if looks_like_google_api_key(token) {
        RULE_GOOGLE_API_KEY
    } else {
        return;
    };
    if reported_spans
        .iter()
        .any(|(existing_start, existing_end)| start < *existing_end && end > *existing_start)
    {
        return;
    }
    reported_spans.push((start, end));
    let _ = push_finding(
        report,
        limits,
        per_file,
        SecretFinding {
            rule_id: rule.id,
            pattern_name: rule.name,
            file_path: relative_path.to_owned(),
            line: line_number,
            column: (start + 1) as u32,
            confidence: rule.confidence,
            entropy: shannon_entropy(token),
            redacted_preview: redact_line(line, start, end),
            value_length: token.len() as u32,
        },
    );
}

fn parse_assignment(line: &str) -> Option<(&str, &str, usize, usize)> {
    let separator = line.find('=').or_else(|| line.find(':'))?;
    let key = line[..separator].trim();
    // Support common declaration prefixes: `const password = ...`,
    // `export const api_key = ...`, `let token: ...`.
    let key = key.split_whitespace().last().unwrap_or("");
    if key.is_empty() || key.len() > 80 {
        return None;
    }
    let mut value_start = separator + 1;
    let remainder = &line[value_start..];
    let trimmed = remainder.trim_start();
    value_start += remainder.len() - trimmed.len();
    let value_slice = &line[value_start..];
    let value_end = value_slice
        .find(&[',', ';', '#', ' ', '\t', '\r', '\n'][..])
        .map_or(line.len(), |offset| value_start + offset);
    let raw_value = line[value_start..value_end].trim();
    let value = raw_value.trim_matches(|character: char| {
        matches!(
            character,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
        )
    });
    if value.is_empty() {
        return None;
    }
    // Recompute the span for the trimmed value inside the original line.
    let leading_quote = raw_value.len()
        - raw_value
            .trim_start_matches(&['"', '\'', '`', '(', '[', '{', '<'][..])
            .len();
    let value_start = value_start + leading_quote;
    let value_end = value_start + value.len();
    if value_end > line.len() {
        return None;
    }
    Some((key, value, value_start, value_end))
}

fn is_skipped_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| SKIPPED_EXTENSIONS.contains(&extension.as_str()))
}

fn looks_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    sample.contains(&0)
}

fn relative_display_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut display = String::new();
    for component in relative.components() {
        let segment = component.as_os_str().to_str()?;
        if segment == ".." || segment.contains('\\') && std::path::MAIN_SEPARATOR == '\\' {
            return None;
        }
        if !display.is_empty() {
            display.push('/');
        }
        display.push_str(segment);
    }
    if display.is_empty() || display.starts_with('/') {
        return None;
    }
    Some(display)
}

struct WalkState<'a> {
    root: &'a Path,
    limits: &'a ScanLimits,
    cancel: &'a AtomicBool,
    report: ScanReport,
    seen_files: HashSet<PathBuf>,
}

fn walk_directory(state: &mut WalkState<'_>, directory: &Path, depth: u32) {
    if state.cancel.load(Ordering::Relaxed) {
        state.report.cancelled = true;
        return;
    }
    if depth > state.limits.max_depth {
        state.report.truncated = true;
        return;
    }
    if state.report.files_scanned >= state.limits.max_files {
        state.report.truncated = true;
        return;
    }
    if state.report.bytes_scanned >= state.limits.max_total_bytes {
        state.report.truncated = true;
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        state.report.skipped_files += 1;
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if state.cancel.load(Ordering::Relaxed) {
            state.report.cancelled = true;
            return;
        }
        if state.report.files_scanned >= state.limits.max_files
            || state.report.bytes_scanned >= state.limits.max_total_bytes
        {
            state.report.truncated = true;
            return;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            state.report.skipped_files += 1;
            continue;
        };
        if metadata.file_type().is_symlink() {
            state.report.skipped_files += 1;
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if metadata.is_dir() {
            if SKIPPED_DIRECTORIES
                .iter()
                .any(|skipped| skipped.eq_ignore_ascii_case(&name))
            {
                continue;
            }
            walk_directory(state, &path, depth + 1);
            continue;
        }
        if !metadata.is_file() {
            state.report.skipped_files += 1;
            continue;
        }
        if is_skipped_extension(&path) {
            state.report.skipped_files += 1;
            continue;
        }
        if metadata.len() > state.limits.max_file_bytes {
            state.report.skipped_files += 1;
            state.report.truncated = true;
            continue;
        }
        if state.report.bytes_scanned + metadata.len() > state.limits.max_total_bytes {
            state.report.truncated = true;
            return;
        }
        let Some(relative_path) = relative_display_path(state.root, &path) else {
            state.report.skipped_files += 1;
            continue;
        };
        if !state.seen_files.insert(path.clone()) {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            state.report.skipped_files += 1;
            continue;
        };
        state.report.bytes_scanned = state
            .report
            .bytes_scanned
            .saturating_add(bytes.len() as u64);
        if looks_binary(&bytes) {
            state.report.skipped_files += 1;
            continue;
        }
        state.report.files_scanned += 1;
        let Ok(text) = std::str::from_utf8(&bytes) else {
            state.report.skipped_files += 1;
            continue;
        };
        let mut per_file = 0_u32;
        for (index, line) in text.lines().enumerate() {
            if state.cancel.load(Ordering::Relaxed) {
                state.report.cancelled = true;
                return;
            }
            if per_file >= state.limits.max_findings_per_file {
                state.report.truncated = true;
                break;
            }
            let line_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            scan_line(
                &relative_path,
                line_number,
                line,
                &mut state.report,
                state.limits,
                &mut per_file,
            );
            if state.report.findings.len() >= state.limits.max_findings as usize {
                state.report.truncated = true;
                break;
            }
        }
    }
}

/// Scans a server-owned snapshot root. The root must be an absolute directory.
pub fn scan_snapshot(root: &Path, limits: &ScanLimits) -> Result<ScanReport, ScanError> {
    let never_cancelled = AtomicBool::new(false);
    scan_snapshot_cancellable(root, limits, &never_cancelled)
}

/// Scans with a cooperative cancellation flag. The flag is checked before each
/// directory entry, file, and line so a cancelled run stops promptly instead of
/// finishing a bounded but unnecessary scan.
pub fn scan_snapshot_cancellable(
    root: &Path,
    limits: &ScanLimits,
    cancel: &AtomicBool,
) -> Result<ScanReport, ScanError> {
    if !root.is_absolute() {
        return Err(ScanError::InvalidRoot);
    }
    let metadata = fs::metadata(root).map_err(|_| ScanError::UnreadableRoot)?;
    if !metadata.is_dir() {
        return Err(ScanError::RootNotDirectory);
    }
    let mut state = WalkState {
        root,
        limits,
        cancel,
        report: ScanReport::default(),
        seen_files: HashSet::new(),
    };
    walk_directory(&mut state, root, 0);
    let report = state.report;
    tracing::debug!(
        event = "static_scan.completed",
        files_scanned = report.files_scanned,
        bytes_scanned = report.bytes_scanned,
        skipped_files = report.skipped_files,
        findings = report.findings.len(),
        truncated = report.truncated,
        cancelled = report.cancelled,
        "static snapshot scan completed"
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::SeqCst);
            let root = std::env::temp_dir().join(format!(
                "probeforge-static-scan-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).expect("fixture root");
            Self { root }
        }

        fn write(&self, path: &str, contents: &[u8]) {
            let target = self.root.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).expect("fixture parent");
            }
            fs::write(target, contents).expect("fixture write");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn scan(fixture: &Fixture) -> ScanReport {
        scan_snapshot(&fixture.root, &ScanLimits::default()).expect("scan")
    }

    fn aws_access_key() -> String {
        ["AKIA", "IOSFODNN7AB12CD3"].concat()
    }

    fn github_token() -> String {
        ["ghp_", "abcdefghijklmnopqrstuvwxyz0123456789"].concat()
    }

    fn private_key_header() -> String {
        ["-----BEGIN ", "OPENSSH ", "PRIVATE KEY-----"].concat()
    }

    #[test]
    fn detects_prefixed_credentials_and_never_returns_the_value() {
        let fixture = Fixture::new();
        let aws = aws_access_key();
        let github = github_token();
        fixture.write(
            ".env",
            format!("AWS_ACCESS_KEY_ID={aws}\nGITHUB_TOKEN={github}\n").as_bytes(),
        );
        let report = scan(&fixture);
        assert_eq!(report.findings.len(), 2);
        let aws_finding = report
            .findings
            .iter()
            .find(|finding| finding.pattern_name == "aws_access_key_id")
            .expect("aws finding");
        assert_eq!(aws_finding.file_path, ".env");
        assert_eq!(aws_finding.line, 1);
        assert!(aws_finding.redacted_preview.contains("***"));
        assert!(!aws_finding.redacted_preview.contains(&aws));
        assert!(!format!("{report:?}").contains(&aws));
        assert!(!format!("{report:?}").contains(&github));
    }

    #[test]
    fn detects_private_key_headers_without_reading_the_block() {
        let fixture = Fixture::new();
        let header = private_key_header();
        fixture.write("keys/id_rsa", format!("{header}\nbody\n").as_bytes());
        let report = scan(&fixture);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].pattern_name, "private_key_block");
        assert!(!format!("{report:?}").contains("body"));
    }

    #[test]
    fn generic_assignments_require_entropy_and_skip_placeholders() {
        let fixture = Fixture::new();
        fixture.write(
            "config.js",
            b"const password = 's3cr3t-V4lu3-with-entropy';\nconst api_key = 'example-value';\nconst token = 'changeme';\n",
        );
        let report = scan(&fixture);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(
            report.findings[0].pattern_name,
            "generic_credential_assignment"
        );
        assert!(
            !report.findings[0]
                .redacted_preview
                .contains("s3cr3t-V4lu3-with-entropy")
        );
    }

    #[test]
    fn skips_symlinks_binary_files_and_dependency_directories() {
        let fixture = Fixture::new();
        fixture.write(
            "node_modules/package/index.js",
            format!("token = '{}'\n", github_token()).as_bytes(),
        );
        fixture.write("image.bin", &[0_u8, 1, 2, 3]);
        fixture.write("src/real.txt", b"nothing to see\n");

        #[cfg(unix)]
        std::os::unix::fs::symlink(
            fixture.root.join("src/real.txt"),
            fixture.root.join("link.txt"),
        )
        .expect("symlink");

        let report = scan(&fixture);
        assert!(report.findings.is_empty());
        assert_eq!(report.files_scanned, 1);
    }

    #[test]
    fn enforces_file_and_finding_limits() {
        let fixture = Fixture::new();
        let mut contents = String::new();
        let key_prefix = ["AKIA", "IOSFODNN7EXAMPL"].concat();
        for index in 0..10 {
            contents.push_str(&format!("AWS_ACCESS_KEY_ID_{index}={key_prefix}{index}\n"));
        }
        fixture.write("many.env", contents.as_bytes());
        let limits = ScanLimits {
            max_findings: 3,
            max_findings_per_file: 2,
            ..ScanLimits::default()
        };
        let report = scan_snapshot(&fixture.root, &limits).expect("scan");
        assert_eq!(report.findings.len(), 2);
        assert!(report.truncated);
    }

    #[test]
    fn rejects_invalid_roots() {
        assert_eq!(
            scan_snapshot(Path::new("relative/path"), &ScanLimits::default()),
            Err(ScanError::InvalidRoot)
        );
        let fixture = Fixture::new();
        fixture.write("file.txt", b"text");
        assert_eq!(
            scan_snapshot(&fixture.root.join("file.txt"), &ScanLimits::default()),
            Err(ScanError::RootNotDirectory)
        );
    }

    #[test]
    fn handles_empty_and_non_utf8_content() {
        let fixture = Fixture::new();
        fixture.write("empty.txt", b"");
        fixture.write("latin1.txt", &[0x66, 0x6f, 0x6f, 0xff, 0x0a]);
        let report = scan(&fixture);
        assert!(report.findings.is_empty());
    }

    #[test]
    fn stops_promptly_when_cancelled() {
        let fixture = Fixture::new();
        fixture.write(
            "config.env",
            format!("AWS_ACCESS_KEY_ID={}\n", aws_access_key()).as_bytes(),
        );
        let cancel = AtomicBool::new(true);
        let report = scan_snapshot_cancellable(&fixture.root, &ScanLimits::default(), &cancel)
            .expect("scan");
        assert!(report.cancelled);
        assert!(report.findings.is_empty());
    }
}
