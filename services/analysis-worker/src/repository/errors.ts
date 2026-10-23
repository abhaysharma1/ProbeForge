export type RepositoryIngestErrorCode =
  | "ambiguous_reference"
  | "audit_failed"
  | "binary_file"
  | "cancelled"
  | "commit_mismatch"
  | "commit_unavailable"
  | "credential_unavailable"
  | "credentials_in_url"
  | "dns_resolution_failed"
  | "file_too_large"
  | "git_failed"
  | "git_output_limit"
  | "git_timeout"
  | "git_unavailable"
  | "internal_error"
  | "invalid_credential_ref"
  | "invalid_manifest"
  | "invalid_url"
  | "not_a_file"
  | "reference_not_found"
  | "repository_too_deep"
  | "repository_too_large"
  | "snapshot_metadata_too_large"
  | "too_many_files"
  | "unsafe_path"
  | "unsafe_reference"
  | "unsupported_credential_kind"
  | "unsupported_credential_scheme"
  | "unsupported_git_host"
  | "unsupported_scheme"
  | "unsupported_tree_entry"
  | "workspace_cleanup_failed"
  | "workspace_too_large";

/**
 * A stable, safe-to-surface ingestion failure. Messages must never contain
 * credentials, full remote URLs, absolute workspace paths, or repository text.
 */
export class RepositoryIngestError extends Error {
  readonly code: RepositoryIngestErrorCode;
  readonly retryable: boolean;

  constructor(
    code: RepositoryIngestErrorCode,
    message: string,
    options: { readonly cause?: unknown; readonly retryable?: boolean } = {},
  ) {
    super(message, options.cause === undefined ? undefined : { cause: options.cause });
    this.name = "RepositoryIngestError";
    this.code = code;
    this.retryable = options.retryable ?? false;
  }
}

export function isRepositoryIngestError(error: unknown): error is RepositoryIngestError {
  return error instanceof RepositoryIngestError;
}

export function toRepositoryIngestError(
  error: unknown,
  fallbackCode: RepositoryIngestErrorCode = "internal_error",
  fallbackMessage = "Repository ingestion failed",
): RepositoryIngestError {
  if (isRepositoryIngestError(error)) return error;
  if (error instanceof Error && error.name === "AbortError") {
    return new RepositoryIngestError("cancelled", "Repository ingestion was cancelled", {
      cause: error,
    });
  }
  return new RepositoryIngestError(fallbackCode, fallbackMessage, { cause: error });
}
