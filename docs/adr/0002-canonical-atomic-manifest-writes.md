# ADR-0002: Canonical, atomic manifest writes

Status: Accepted — 2026-08-07

## Context

`uncompose.project.json` is meant to be committed to git and read by humans and
by the rest of the Uncompose family. Two failure modes threaten that:

- **Noisy diffs.** If field order or formatting varies between writes, every
  rewrite churns the whole file and buries the real change under reordering.
- **Truncated files.** If the tool is interrupted mid-write (crash, `SIGKILL`,
  full disk), a naive in-place write can leave a half-written, unparseable
  manifest — losing the very record the tool exists to protect.

## Decision

Every manifest write is **canonical** and **atomic**.

- **Canonical form**: fixed field order (the schema's declaration order — for the
  top level `schema`, `project`, `assets`, `derivations`, `evaluations`), 2-space
  indentation, and a single trailing newline. In Rust this falls out of
  serializing structs whose fields are declared in canonical order with
  `serde_json::to_string_pretty` (2-space indent) plus one appended `\n`. Diffs
  stay minimal and reviewable.
- **Atomic write**: serialize fully into a temp file in the *same directory* as
  the target, flush it to disk (`sync_all`), then `rename` it over the target.
  Rename within a directory is atomic on the platforms we target, so a reader
  ever sees either the old complete manifest or the new complete one — never a
  truncation. The temp file is cleaned up if the rename fails.

Both properties are asserted at the CLI process boundary: tests compare the exact
emitted bytes (order, indent, trailing newline) rather than a re-parsed value.

## Consequences

- Manifests diff cleanly in git; review focuses on the semantic change.
- An interrupted write never corrupts an existing manifest.
- The temp file lives in the project directory momentarily; its name is
  dot-prefixed and ULID-suffixed to avoid collisions and stray visibility.
- Every future write path (`add`, `verify`'s `last_verified` update, import) must
  route through the same canonical + atomic writer, never writing the manifest in
  place. `ext` subtrees, preserved verbatim on read-modify-write per ADR-0001,
  ride through this writer unchanged.
