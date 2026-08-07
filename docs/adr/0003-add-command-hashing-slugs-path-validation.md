# ADR-0003: `add` — hashing, slug ids, and path validation

Status: Accepted — 2026-08-07

## Context

`add <path>` registers a file as an asset. Three decisions shape it: how an
asset's identity is captured, how its id is chosen, and how the path argument is
constrained so a project only ever references files inside its own root.

The schema (ADR-0001) already fixes the asset shape — `{id, path, sha256, size,
role, added_at}` with `id`/`role` constrained to the slug pattern
`^[a-z0-9][a-z0-9._-]*$` and `path` "relative to the project root, forward
slashes, resolves inside the root." This ADR records how `add` satisfies that.

## Decision

- **Identity is content, captured now.** `add` streams the file's exact bytes
  through SHA-256 and records the lowercase hex digest plus the byte length. This
  is the asset's identity (CONTEXT.md); `path` is a mutable location hint. An
  `added_at` RFC3339 UTC timestamp (whole seconds, matching `created_at`) is
  recorded.

- **Ids are slugs minted from the filename stem.** The stem is lowercased,
  disallowed characters become `-`, leading non-alphanumerics and trailing `-`
  are trimmed (empty → `asset`). On collision with an existing id the mint appends
  `-2`, `-3`, … so `take2/vocals.wav` after `take1/vocals.wav` yields `vocals`
  then `vocals-2`. `--id` overrides the mint and is validated against the slug
  pattern; a taken id is refused rather than silently suffixed, since an explicit
  id is a deliberate choice. `--role` defaults to `mix` and is likewise validated
  as a slug so every emitted manifest conforms to the schema.

- **Paths are confined to the root by resolution, not string matching.** `add`
  refuses an absolute path outright, then canonicalizes both the root and the
  joined target — which collapses `..` and follows symlinks — and requires the
  result to sit under the canonical root (`strip_prefix`). A `../` escape or a
  symlink pointing outside the root therefore fails the same check; the stored
  path is the canonical root-relative path with forward slashes. A missing file
  surfaces as its own error; an unreadable file (permissions, a directory) surfaces
  as a read failure.

- **Every refusal precedes the write.** A duplicate path (its error names the
  existing asset), an out-of-root path, a missing/unreadable file, and an
  invalid/taken id all return before the atomic write (ADR-0002), so a refused
  `add` leaves the manifest byte-identical.

## Consequences

- Registering a file re-reads its full bytes; hashing cost scales with file size.
  Acceptable for M1's local, interactive use.
- Minted ids are stable and human-readable but not guaranteed meaningful; `--id`
  exists for when the stem is a poor name.
- Canonicalization requires the file to exist on disk at `add` time, which is
  already required to hash it — the checks share one resolution.
- `add` round-trips only the v0 core fields it models. `ext` subtrees and future
  optional asset fields (e.g. `last_verified`) are preserved by the command that
  first writes them (verify, import), consistent with modeling only what each
  command needs (ADR-0001, ADR-0002).
