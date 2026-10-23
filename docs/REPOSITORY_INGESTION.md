# ProbeForge read-only Git ingestion

Phase 5 adds a read-only repository ingestion library and an operator CLI to the
TypeScript analysis worker. It does not add an HTTP ingestion endpoint, does not
persist snapshots to PostgreSQL yet, and never executes repository content.

## Scope and non-goals

Phase 5 can:

- validate supported Git URLs and reject unsafe schemes, hosts, paths, ports,
  credentials in URLs, and unsafe source references;
- resolve a branch or tag to an immutable commit SHA, or accept a full SHA;
- fetch and check out only that commit into a unique temporary workspace;
- apply bounded process, output, file-count, byte, depth, and workspace limits;
- record symlinks and submodules without following or initializing them;
- inventory source files, package manifests, and lockfiles as inert text;
- detect common languages and frameworks from file types and dependency names;
- write structured snapshot metadata and append-only audit events;
- remove the workspace on success, failure, and cancellation.

Not included in Phase 5:

- an unauthenticated repository-ingestion HTTP route;
- database persistence of snapshots or audit events;
- package installation, builds, tests, application startup, migrations, or
  arbitrary project scripts;
- submodule contents, Git LFS object downloads, Git hooks, repository Git
  configuration, SSH agent forwarding, or passphrase-protected SSH keys;
- source parsing, route discovery, scanning, planning, approvals, target contact,
  or load generation.

## Supported Git transports

| Transport | Accepted | Credential reference |
|---|---|---|
| HTTPS | Yes, default port 443 only | `http_basic` |
| SSH | Yes, default port 22 only | unencrypted `ssh_private_key` |
| `file:` | Only with explicit local-fixture mode | none |
| HTTP, `git:`, FTP, custom helpers | No | none |

Remote hosts must be in `PROBE_FORGE_ANALYSIS_ALLOWED_GIT_HOSTS`. The default
allowlist is `github.com,gitlab.com,bitbucket.org`; a deployment must explicitly
add any other host. URLs with embedded usernames/passwords are rejected. The
only accepted SSH username is `git`.

Before Git connects, the worker resolves the hostname and rejects the request if
the hostname is internal or if **any** resolved address is loopback, private,
link-local, carrier-grade NAT, documentation-only, multicast, unspecified,
IPv4-mapped private, or a cloud metadata address. HTTP redirects are disabled,
TLS verification is enabled, and Git's protocol allowlist is restricted to
HTTPS, SSH, and (for fixtures only) `file`.

DNS rebinding is reduced by the allowlist, disabled redirects, and the preflight
resolution check, but the Git CLI performs its own final resolution. A future
network-egress policy or a transport that pins the validated address is still
required for a hardened deployment.

## Source references

`--ref` accepts:

- a branch name such as `main` or `release/1.0`;
- a tag name such as `v1.2.3`;
- an exact `refs/heads/...` or `refs/tags/...` reference;
- a full lowercase 40- or 64-character commit SHA.

References are passed as argument-array values, never through a shell. Revision
expressions (`main~1`, `main^{}`), option-like values, path traversal, control
characters, and non-branch/tag `refs/` namespaces are rejected. A name that is
both a branch and a tag with different commits is ambiguous and fails closed.

## Secret references

Credentials are never written into the remote URL or command arguments.
`credentialRef` must use `secret://...`. The environment resolver maps:

```text
secret://git/github  ->  PROBE_FORGE_SECRET_GIT_GITHUB
```

The runtime value is JSON:

```json
{"kind":"http_basic","username":"x-access-token","password":"runtime-only"}
```

For SSH, the runtime value is:

```json
{"kind":"ssh_private_key","privateKey":"<unencrypted private key material>"}
```

The key is written to the unique workspace with owner-only permissions and is
removed with the workspace. The HTTP credential is supplied to Git through
`GIT_CONFIG_VALUE_0` as an `http.extraHeader`, not through the URL. The worker
uses an isolated `HOME`, empty global/system Git config, an empty hooks
directory, no credential helper, no interactive prompts, and no inherited
`GIT_*` or SSH-agent variables. Git errors are redacted before they become
messages, audit fields, or CLI output.

Do not commit real credential values. `.env.example` contains placeholders only;
production values belong in the deployment's secret injection mechanism.

## Generic Git arguments

Every Git invocation uses a fixed executable (`git`), an argument array, and
`shell: false`. The following controls are applied:

- protocol allowlist: HTTPS, SSH, and fixture-only `file`;
- HTTP redirects disabled and TLS verification enabled;
- object verification (`fetch.fsckObjects`, `transfer.fsckObjects`);
- repository hooks disabled through an isolated empty hooks directory;
- submodule recursion disabled;
- symlink checkout disabled where Git honors `core.symlinks`;
- LFS smudge downloads disabled;
- no interactive credentials and no credential helper;
- optional locks, automatic GC, and maintenance disabled;
- isolated temporary, home, global-config, and system-config paths;
- a hard per-command timeout and a combined stdout/stderr byte cap;
- child-process-tree termination on timeout, output overflow, or cancellation.

The worker never runs `git submodule update`, `git lfs pull`, `npm install`,
`bun install`, `cargo build`, `make`, `pytest`, `go test`, `dotnet build`,
`docker`, application entry points, or repository-provided scripts.

## Bounds

Defaults are conservative and configurable through `PROBE_FORGE_ANALYSIS_*`
variables:

| Setting | Default |
|---|---|
| Tracked file entries | 5,000 |
| Total tracked blob bytes | 64 MiB |
| Bytes read from one file | 2 MiB |
| Repository path depth | 32 segments |
| On-disk workspace bytes | 128 MiB |
| Git command output | 8 MiB |
| Git command timeout | 60 seconds |
| Files included in snapshot metadata | 500 |
| Dependencies per manifest | 200 |

Tree metadata is checked before checkout. Workspace size is checked after
checkout. Inspection reads files from the checked-out tree with `lstat`,
`realpath`, path containment, and byte caps. Symlinks, submodules, generated
directories, oversized files, and binary files are not read as source. Absolute
paths, `..`, backslashes, colon stream separators, `.git` segments, and
Windows-reserved filenames are rejected.

Generated directories such as `node_modules`, `vendor`, `dist`, `build`,
`target`, `.venv`, `__pycache__`, `.next`, and `coverage` are excluded from
inspection, but still count toward tree and workspace limits.

## Inspection output

The snapshot records:

- commit SHA and the requested branch/tag/SHA;
- detected languages with file/byte statistics (ordering by bytes);
- frameworks inferred from dependency names and a few file markers;
- package manifests and lockfiles, parsed as text/JSON with bounded dependency
  lists, script names only (never script bodies), and explicit parse errors;
- up to 500 file inventory entries with path, size, kind (`text`, `binary`,
  `oversized`), and language;
- symlink and submodule paths, without their content;
- truncation flags and `executedRepositoryCode: false`;
- `untrustedContent: true` on the snapshot and metadata.

The snapshot is metadata, not source content. The callback receives a
`RepositorySession` with a bounded `readFile` method; that method is available
only while the workspace exists. Do not persist repository file content or treat
it as instructions.

## Operator CLI

The CLI is deliberately local and separate from the worker's health server:

```bash
bun run --cwd services/analysis-worker ingest -- \
  --fixture /absolute/path/to/inert-fixture \
  --ref main \
  --audit-log /absolute/path/to/ingestion-audit.jsonl \
  --output /absolute/path/to/snapshot.json
```

For a remote repository:

```bash
bun run --cwd services/analysis-worker ingest -- \
  --url https://github.com/example/repository.git \
  --ref v1.2.3 \
  --credential-ref secret://git/github \
  --audit-log /absolute/path/to/ingestion-audit.jsonl \
  --output /absolute/path/to/snapshot.json
```

`--fixture` is the only way to enable `file:` URLs, and the ingestion library
requires an explicit `allowLocalFixtures: true` option for the same reason. The
fixture path must be inside its parent directory (both lexically and after
`realpath`). Remote URLs remain subject to the host allowlist and DNS policy.

The command writes JSON errors to stderr with a stable code and a safe message.
It never prints credentials, remote URLs from Git errors, absolute workspace
paths, or stack traces. Use `--max-files`, `--max-total-bytes`,
`--max-file-bytes`, `--max-depth`, `--max-workspace-bytes`,
`--max-inventory-files`, `--max-dependencies-per-manifest`, and
`--git-timeout-ms` to tighten limits further. Add `--discovery-output <file>`
to run the Phase 6 static discovery layer against the same pinned workspace and
write a bounded application inventory; see
[DISCOVERY.md](DISCOVERY.md). Discovery is optional and is never performed by
the ingestion library automatically.

## Audit events

The JSONL sink writes one JSON object per line. Events intentionally omit remote
URLs, credential references, absolute paths, repository content, and secrets:

- `repository.clone.started`
- `repository.reference.resolved`
- `repository.snapshot.ready`
- `repository.ingest.failed`
- `repository.workspace.cleaned`
- `repository.workspace.cleanup_failed`

Every event carries a timestamp and a random workspace ID. Snapshot creation and
workspace cleanup are always represented when the sink is reachable. A failed
audit write is surfaced as `audit_failed`; a cleanup failure is recorded as
`repository.workspace.cleanup_failed` and becomes the returned error when the
operation otherwise succeeded.

## Tests and fixtures

`services/analysis-worker/fixtures/inert-repo/` contains small TypeScript,
Python, Rust, Go, Java, Ruby, PHP, and C# files plus Node, Rust, Python, Go,
Java, Ruby, PHP, and .NET manifests and lockfiles. It also contains install,
build, test, and start scripts, a Makefile, and a shell script. `scripts/pwn.js`
would write `fixture-executed.txt` if executed; tests assert that marker never
appears. Dynamic tests add symlink entries, gitlinks, malformed manifests,
oversized files, and binary files without contacting a network.

All tests use local inert fixtures and local processes. No real repository,
scanner, load generator, or external target is contacted.

Phase 6 builds on this boundary with deterministic static discovery. It reads
the bounded `RepositorySession`, never starts a second Git process, and never
executes repository code. See [DISCOVERY.md](DISCOVERY.md) for the inventory
model, parsers, limits, and Pi toolset. The optional CLI `--discovery-output`
writes that inventory beside the Phase 5 snapshot.

## Deferred follow-ups

- Persist snapshots, inventories, and audit events to PostgreSQL and connect the
  analysis worker to the durable job queue.
- Add an authenticated, authorized control-plane endpoint that requests
  ingestion; do not expose the operator CLI as a network service.
- Add a network-egress policy or address-pinned Git transport for stronger
  DNS-rebinding resistance.
- Decide how to manage known SSH host keys and encrypted SSH keys.
- Wire the Phase 6 discovery toolset to Pi in Phase 7 with token and output
  budgets; do not expose it as an unrestricted file or shell tool.
- Add source parsing and route discovery in Phase 6.
