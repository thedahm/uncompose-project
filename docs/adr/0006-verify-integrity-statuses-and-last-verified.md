# ADR-0006: `verify` — derived integrity statuses and the `last_verified` cache

Status: Accepted — 2026-08-07

## Context

`verify` re-checks every registered asset against the file on disk and tells the
user whether the project's files still match their recorded identity. The schema
(ADR-0001) already fixes the asset shape and reserves an optional
`last_verified` date-time field described as a "cache of the last successful
integrity check, never a status claim." `add` deferred writing that field to the
command that first needs it (ADR-0003); this is that command. Integrity itself is
derived, never stored (uncompose#62): the manifest records identity (`sha256` +
`size`) and, at most, when a file was last known good — not a verified/modified
verdict.

## Decision

- **Integrity is derived per asset, size before hash.** For each asset `verify`
  stats the file: absent → `missing`; a size that differs from the recorded
  `size` → `modified` without hashing (the cheap mismatch, checked first); a
  matching size whose streamed sha256 differs → `modified`; both match →
  `verified`. Status is a value computed at check time
  (`Integrity::{Verified,Modified,Missing}`), never persisted.

- **Passing assets refresh `last_verified`; nothing else is written.** A
  `verified` asset gets a fresh RFC3339 (whole-second, UTC) `last_verified`, and
  the manifest is rewritten canonically and atomically (ADR-0002). A `modified`
  or `missing` asset is never stamped, so the timestamp always means "last known
  good." Passing assets are refreshed even when another asset in the same run
  fails — the cache reflects each file independently. When no asset passes, the
  manifest is left byte-identical (no rewrite).

- **Exit code gates on integrity.** `verify` exits zero only when every asset is
  `verified`; any `modified` or `missing` yields a non-zero exit so scripts and
  CI can gate on project integrity. Each failure prints a warning to stderr
  naming the path and how it failed (contents changed vs. file missing); passes
  print to stdout.

- **The read path is shared and strict.** `verify` reads through the same
  `load_manifest` as `add`, so an unrecognized `schema` URL, an unknown plain
  field, or a non-project directory is refused the same way (ADR-0005), before
  any write. The read errors were factored into a shared `LoadError` that both
  `AddError` and `VerifyError` wrap, so the strict-read policy is spelled once.

## Consequences

- `verify` re-reads the full bytes of every asset whose size still matches;
  cost scales with total project size. The size pre-check skips hashing for files
  that changed length. Acceptable for M1's local, interactive use.
- A file replaced with different content of the same length is caught by the
  hash, not the size check — identity is content, not length.
- `move`/`repair` are out of scope (M1): a relocated file simply reads as
  `missing` at its old path and would be re-added at the new one.
- The milestone DoD lands here as CLI-seam integration tests: create a project,
  add a file, modify it on disk → clear warning + non-zero exit; delete it →
  reported missing + non-zero exit.
