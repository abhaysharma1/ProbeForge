# Custom OpenAI-compatible planning provider (post-Phase 15 enhancement)

This report records an enhancement built on top of the completed phase plan. It
does not introduce a new numbered phase.

- **Date:** 2026-10-24
- **Decision:** [D-044](../DECISION_LOG.md#d-044---planners-can-target-an-admin-configured-openai-compatible-provider)

## Summary

An admin can now point the plan generator at any OpenAI-compatible endpoint from
the dashboard, instead of only providers present in the pinned Pi SDK catalog.
This unblocks Nebius AI Token Factory and similar endpoints without a code change
per vendor.

## What changed

- **Migration `0016_planning_custom_provider.sql`** adds `planning_base_url`,
  `planning_context_window`, and `planning_max_output_tokens` to
  `platform_settings`, with HTTPS-scheme, length, range, and output-within-context
  constraints mirroring the style of `0015_platform_admin.sql`.
- **API** (`services/api/src/admin.rs`, `platform_settings.rs`): the settings DTO
  and update request carry the new fields; `normalize_base_url` and
  `validate_custom_provider` reject non-HTTPS and internal/metadata hosts and
  inconsistent metadata; the update/clear queries and audit metadata include the
  new columns.
- **Worker**: `platform-settings.ts` reads the new columns; `job-loop.ts` builds a
  `CustomOpenAiCompatConfig`; `agent/model.ts` registers a custom
  `openai-completions` provider (`createProvider` + `setProvider`) when a base URL
  is set and re-validates the URL; `repository/net.ts` gained a synchronous
  `isPublicHost` reusing the existing host/address classification.
- **Frontend** (`apps/web/src/api.ts`, `AdminView.tsx`): the "Planning (Pi)"
  section gains Base URL, Context window, and Max output tokens fields with
  supporting copy; custom configuration takes precedence over a built-in id.
- **Tests**: `services/analysis-worker/src/agent/model.test.ts` (custom provider
  build, precedence, URL rejection, metadata bounds); new unit tests in
  `admin.rs`; a DB-backed integration test in `services/api/tests/admin_settings.rs`.
- **Docs**: `docs/PLANNING.md` and `docs/DECISION_LOG.md` (D-044).

## Checks run and observed results

| Check | Command | Result |
|---|---|---|
| Worker typecheck | `bun run --cwd services/analysis-worker typecheck` | Pass |
| Web typecheck | `bun run --cwd apps/web typecheck` | Pass |
| Worker tests | `bun run --cwd services/analysis-worker test` | 131 pass, 10 skip, 0 fail |
| Web tests | `bun run --cwd apps/web test` | 14 pass |
| Packages tests | `bun run test:ts` | test-catalog 6, contracts 55, worker 131, web 14 — all pass |
| Rust lint/format | `cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --all -- --check` | Pass (exit 0) |
| Rust API suite (incl. DB) | `cargo test -p probe-forge-api -j 1` | 82 pass, 0 fail (44 unit + 38 integration, including `migrations` and `admin_settings`) |
| JS/TS lint | `bun run lint` (Biome) | Pass |
| Docs contract | `bun scripts/check-docs.mjs` | Pass (113/113 files) |

## Known limitations / unverified

- `cargo test --workspace` (all crates at once) could not complete in this
  environment: parallel codegen of many test binaries exhausted memory
  ("paging file is too small"). The affected crate's full suite was run serially
  with `-j 1` and passed, and `cargo clippy --workspace --all-targets` confirms
  every crate still compiles. Crates outside `probe-forge-api` are unaffected by
  this change.
- No live model call was made. The custom-provider tests assert construction and
  configuration, and the DB integration test round-trips the settings, but an
  end-to-end plan against a live OpenAI-compatible endpoint was not exercised.
- DNS-rebinding is mitigated best-effort only: the worker validates the literal
  host before registering the provider, but the Pi SDK performs the actual fetch.

## Next steps (suggested)

- Run `bun run check` and `bun run check:db` in CI.
- Optionally exercise one real OpenAI-compatible endpoint in a secured staging
  environment with tool calling enabled.
