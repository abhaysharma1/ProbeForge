//! Server-owned pinned repository snapshot materialization.
//!
//! Static security adapters need the exact repository content that the plan was
//! generated from. The execution worker recreates that snapshot at the job's
//! pinned commit into a temporary, server-owned directory using a fixed `git`
//! binary and a typed argument vector. No shell is involved, no plan/model value
//! becomes an argument, and the workspace is removed when the run finishes.
//!
//! Credentialed repositories are rejected with a stable code until a
//! server-owned secret resolver is wired into the execution worker. That is
//! honest: the adapter must not silently scan the wrong content.

use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use sqlx::PgPool;
use thiserror::Error;
use tokio::process::Command;
use uuid::Uuid;

const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const GIT_TOTAL_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("repository snapshot metadata is missing")]
    Missing,
    #[error("job has no pinned commit")]
    MissingCommit,
    #[error("credentialed repositories are not supported by the execution worker yet")]
    CredentialsNotConfigured,
    #[error("repository URL scheme is not supported for snapshot materialization")]
    UnsupportedUrl,
    #[error("git is not available")]
    GitUnavailable,
    #[error("git command timed out")]
    GitTimeout,
    #[error("git command failed")]
    GitFailed,
    #[error("the checked-out commit does not match the pinned commit")]
    CommitMismatch,
    #[error("snapshot workspace could not be created or removed")]
    Workspace,
}

impl SnapshotError {
    /// Stable, non-leaking error code persisted with the test result.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Missing => "repository_snapshot_missing",
            Self::MissingCommit => "repository_commit_missing",
            Self::CredentialsNotConfigured => "repository_credentials_not_configured",
            Self::UnsupportedUrl => "repository_url_unsupported",
            Self::GitUnavailable => "repository_git_unavailable",
            Self::GitTimeout => "repository_git_timeout",
            Self::GitFailed => "repository_git_failed",
            Self::CommitMismatch => "repository_commit_mismatch",
            Self::Workspace => "repository_workspace_unavailable",
        }
    }

    pub fn safe_message(&self) -> &'static str {
        match self {
            Self::Missing => "The job's repository snapshot metadata could not be loaded.",
            Self::MissingCommit => {
                "The job does not have a pinned commit; regenerate the analysis before running static tests."
            }
            Self::CredentialsNotConfigured => {
                "This repository requires a credential that the execution worker cannot resolve yet; configure a server-owned repository credential."
            }
            Self::UnsupportedUrl => {
                "The repository URL scheme cannot be materialized by the worker."
            }
            Self::GitUnavailable => "Git is not available in the execution environment.",
            Self::GitTimeout => "Repository materialization exceeded its time budget.",
            Self::GitFailed => "The pinned commit could not be fetched from the repository.",
            Self::CommitMismatch => "The materialized snapshot does not match the pinned commit.",
            Self::Workspace => "A temporary snapshot workspace could not be created or removed.",
        }
    }
}

/// A temporary pinned snapshot. Dropping the value removes the workspace.
pub struct PinnedSnapshot {
    root: PathBuf,
}

impl PinnedSnapshot {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for PinnedSnapshot {
    fn drop(&mut self) {
        if let Err(error) = remove_tree(&self.root) {
            tracing::warn!(
                event = "snapshot.cleanup_failed",
                error = %error,
                "temporary repository snapshot could not be removed"
            );
        }
    }
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            // Git can leave read-only objects behind on Windows. Clear the
            // read-only bit and retry once.
            make_tree_writable(path);
            std::fs::remove_dir_all(path).map_err(|_| first_error)
        }
    }
}

/// Windows leaves Git object files read-only; clearing the bit is required before
/// removal. The function is only reached after a failed recursive delete.
#[allow(clippy::permissions_set_readonly_false)]
fn make_tree_writable(path: &Path) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        let _ = std::fs::set_permissions(path, permissions);
    }
    if !metadata.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        make_tree_writable(&entry.path());
    }
}

fn is_pinned_commit(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn local_snapshots_allowed() -> bool {
    env::var("PROBE_FORGE_ALLOW_LOCAL_SNAPSHOTS")
        .map(|value| value.trim() == "1")
        .unwrap_or(false)
}

fn validate_repository_url(raw: &str, allow_local: bool) -> Result<(), SnapshotError> {
    let parsed = url::Url::parse(raw).map_err(|_| SnapshotError::UnsupportedUrl)?;
    match parsed.scheme() {
        "https" => {
            if parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(SnapshotError::UnsupportedUrl);
            }
            Ok(())
        }
        "ssh" => {
            if parsed.host_str().is_none()
                || parsed.username() != "git"
                || parsed.password().is_some()
            {
                return Err(SnapshotError::UnsupportedUrl);
            }
            Ok(())
        }
        "file" if allow_local => Ok(()),
        _ => Err(SnapshotError::UnsupportedUrl),
    }
}

fn git_binary() -> std::ffi::OsString {
    env::var_os("PROBE_FORGE_GIT_BINARY").unwrap_or_else(|| "git".into())
}

/// Health signal for the execution environment: static adapters need a working
/// Git binary to materialize a pinned snapshot.
pub async fn git_available() -> bool {
    let mut command = Command::new(git_binary());
    command
        .arg("--version")
        .env_clear()
        .env("PATH", env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(Duration::from_secs(2), command.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

async fn run_git(
    cwd: &Path,
    args: &[std::ffi::OsString],
    capture_stdout: bool,
    home: &Path,
) -> Result<Vec<u8>, SnapshotError> {
    let mut command = Command::new(git_binary());
    command
        .current_dir(cwd)
        .env_clear()
        .env("PATH", env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("gitconfig-empty"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=yes",
        )
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(if capture_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command.args(args);

    let child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SnapshotError::GitUnavailable
        } else {
            SnapshotError::GitFailed
        }
    })?;
    let output = match tokio::time::timeout(GIT_COMMAND_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(_)) => return Err(SnapshotError::GitFailed),
        Err(_) => return Err(SnapshotError::GitTimeout),
    };
    if !output.status.success() {
        return Err(SnapshotError::GitFailed);
    }
    Ok(output.stdout)
}

/// Recreates the job's pinned repository snapshot into a fresh temporary
/// workspace. The temporary root is removed when the returned value is dropped.
pub async fn materialize_pinned_snapshot(
    pool: &PgPool,
    project_id: Uuid,
    job_id: Uuid,
) -> Result<PinnedSnapshot, SnapshotError> {
    let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT g.normalized_url, g.credential_ref, j.current_commit_sha
         FROM jobs j
         JOIN git_sources g ON g.id = j.git_source_id
         WHERE j.id = $1 AND j.project_id = $2",
    )
    .bind(job_id)
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| SnapshotError::Missing)?;
    let Some((repository_url, credential_ref, commit_sha)) = row else {
        return Err(SnapshotError::Missing);
    };
    if credential_ref.is_some() {
        return Err(SnapshotError::CredentialsNotConfigured);
    }
    let Some(commit_sha) = commit_sha else {
        return Err(SnapshotError::MissingCommit);
    };
    if !is_pinned_commit(&commit_sha) {
        return Err(SnapshotError::MissingCommit);
    }
    materialize_from_repository(&repository_url, &commit_sha, local_snapshots_allowed()).await
}

/// Materializes a repository into a temporary workspace without consulting the
/// database. Exposed for deterministic tests and future snapshot-cache reuse.
pub async fn materialize_from_repository(
    repository_url: &str,
    commit_sha: &str,
    allow_local: bool,
) -> Result<PinnedSnapshot, SnapshotError> {
    if !is_pinned_commit(commit_sha) {
        return Err(SnapshotError::MissingCommit);
    }
    validate_repository_url(repository_url, allow_local)?;
    let repository_url = repository_url.to_owned();
    let commit_sha = commit_sha.to_ascii_lowercase();

    let root = env::temp_dir().join(format!("probeforge-snapshot-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).map_err(|_| SnapshotError::Workspace)?;
    let snapshot = PinnedSnapshot { root };
    let repo = snapshot.root.join("repo");
    let home = snapshot.root.join("home");
    let hooks = snapshot.root.join("hooks");
    std::fs::create_dir_all(&repo).map_err(|_| SnapshotError::Workspace)?;
    std::fs::create_dir_all(&home).map_err(|_| SnapshotError::Workspace)?;
    std::fs::create_dir_all(&hooks).map_err(|_| SnapshotError::Workspace)?;
    std::fs::write(home.join("gitconfig-empty"), b"").map_err(|_| SnapshotError::Workspace)?;

    let hooks_arg = format!("core.hooksPath={}", hooks.display());
    let symlinks_arg = "core.symlinks=false".to_owned();
    let autocrlf_arg = "core.autocrlf=false".to_owned();

    let total = tokio::time::timeout(GIT_TOTAL_TIMEOUT, async {
        run_git(
            &repo,
            &[
                "-c".into(),
                hooks_arg.clone().into(),
                "-c".into(),
                symlinks_arg.clone().into(),
                "-c".into(),
                autocrlf_arg.clone().into(),
                "init".into(),
                "--quiet".into(),
            ],
            false,
            &home,
        )
        .await?;
        run_git(
            &repo,
            &[
                "-c".into(),
                hooks_arg.clone().into(),
                "remote".into(),
                "add".into(),
                "origin".into(),
                repository_url.clone().into(),
            ],
            false,
            &home,
        )
        .await?;
        run_git(
            &repo,
            &[
                "-c".into(),
                hooks_arg.clone().into(),
                "fetch".into(),
                "--quiet".into(),
                "--depth=1".into(),
                "--no-tags".into(),
                "origin".into(),
                commit_sha.clone().into(),
            ],
            false,
            &home,
        )
        .await?;
        run_git(
            &repo,
            &[
                "-c".into(),
                hooks_arg.clone().into(),
                "-c".into(),
                symlinks_arg.into(),
                "-c".into(),
                autocrlf_arg.into(),
                "checkout".into(),
                "--detach".into(),
                "--quiet".into(),
                "FETCH_HEAD".into(),
            ],
            false,
            &home,
        )
        .await?;
        let head = run_git(&repo, &["rev-parse".into(), "HEAD".into()], true, &home).await?;
        Ok::<Vec<u8>, SnapshotError>(head)
    })
    .await
    .map_err(|_| SnapshotError::GitTimeout)??;

    let head = String::from_utf8_lossy(&total).trim().to_ascii_lowercase();
    if head != commit_sha.to_ascii_lowercase() {
        return Err(SnapshotError::CommitMismatch);
    }

    tracing::info!(
        event = "snapshot.materialized",
        commit_sha = %commit_sha,
        "pinned repository snapshot materialized for static adapters"
    );
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_repository_urls_and_pinned_commits() {
        assert!(validate_repository_url("https://github.com/acme/app.git", false).is_ok());
        assert!(validate_repository_url("ssh://git@gitlab.com/acme/app.git", false).is_ok());
        assert_eq!(
            validate_repository_url("http://github.com/acme/app.git", false)
                .unwrap_err()
                .code(),
            "repository_url_unsupported"
        );
        assert_eq!(
            validate_repository_url("https://user:pass@github.com/acme/app.git", false)
                .unwrap_err()
                .code(),
            "repository_url_unsupported"
        );
        assert_eq!(
            validate_repository_url("ssh://root@gitlab.com/acme/app.git", false)
                .unwrap_err()
                .code(),
            "repository_url_unsupported"
        );
        assert!(
            validate_repository_url("file:///tmp/repo", false).is_err(),
            "local repositories require the explicit opt-in"
        );
        assert!(validate_repository_url("file:///tmp/repo", true).is_ok());
        assert!(is_pinned_commit("a".repeat(40).as_str()));
        assert!(!is_pinned_commit("main"));
        assert!(!is_pinned_commit("g".repeat(40).as_str()));
    }

    #[tokio::test]
    async fn git_health_reports_availability() {
        assert!(
            git_available().await,
            "git must be available for static adapters"
        );
    }

    #[tokio::test]
    async fn materializes_a_local_pinned_snapshot_and_rejects_mismatches() {
        let source = env::temp_dir().join(format!("probeforge-snapshot-source-{}", Uuid::new_v4()));
        std::fs::create_dir_all(source.join("src")).expect("source dir");
        std::fs::write(source.join("src/main.rs"), b"fn main() {}\n").expect("source file");
        let run = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(&source)
                .args([
                    "-c",
                    "user.name=ProbeForge Test",
                    "-c",
                    "user.email=probeforge-test@example.test",
                    "-c",
                    "core.autocrlf=false",
                ])
                .args(args)
                .output()
                .expect("git runs");
            assert!(output.status.success(), "git {args:?} failed");
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        run(&["init", "--quiet", "--initial-branch=main"]);
        run(&["add", "-A"]);
        run(&["commit", "--quiet", "-m", "baseline"]);
        let sha = run(&["rev-parse", "HEAD"]);
        let url = url::Url::from_file_path(&source)
            .expect("file url")
            .to_string();

        let snapshot = materialize_from_repository(&url, &sha, true)
            .await
            .expect("materialize");
        assert_eq!(
            std::fs::read_to_string(snapshot.root().join("repo/src/main.rs"))
                .expect("materialized file"),
            "fn main() {}\n"
        );
        let head = std::process::Command::new("git")
            .current_dir(snapshot.root().join("repo"))
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("rev-parse");
        assert_eq!(
            String::from_utf8_lossy(&head.stdout)
                .trim()
                .to_ascii_lowercase(),
            sha
        );
        let snapshot_root = snapshot.root().to_path_buf();
        drop(snapshot);
        assert!(!snapshot_root.exists(), "snapshot workspace is removed");

        let mismatch = materialize_from_repository(&url, &"0".repeat(40), true).await;
        assert!(matches!(mismatch, Err(SnapshotError::GitFailed)));

        let _ = std::fs::remove_dir_all(&source);
    }

    #[test]
    fn removes_a_temporary_tree() {
        let root = env::temp_dir().join(format!("probeforge-snapshot-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("nested")).expect("create");
        std::fs::write(root.join("nested/file.txt"), b"x").expect("write");
        remove_tree(&root).expect("remove");
        assert!(!root.exists());
    }
}
