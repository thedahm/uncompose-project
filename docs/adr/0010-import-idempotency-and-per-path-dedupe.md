# ADR-0010: `import` — idempotent re-import and per-path asset dedupe

Status: Accepted — 2026-08-08

## Context

ADR-0008 and ADR-0009 landed the happy path and the refusals guarding the job
record and the files it names, and both explicitly deferred two behaviors to a
later slice: making a repeated import a stated no-op, and per-path stem
dedupe/conflict. This ADR records that third slice (PRD
thedahm/uncompose-project#11, contract uncompose#63) — safe re-runs.

## Decision

- **Idempotency keys on the `job.json` sha256.** Right after reading the job
  record's bytes and hashing them, import checks the manifest's derivations for
  one whose `job.sha256` already equals this hash. A match makes the import a
  stated no-op: exit 0, no manifest write, and a message naming the existing
  derivation. The check precedes parsing and every refusal, so a byte-identical
  re-run is a fast no-op regardless of what else changed on disk. This is modeled
  as a new `ImportOutcome` enum returned by the core op — `Imported(..)` for a
  fresh import that wrote the manifest, `AlreadyImported { derivation_id }` for
  the no-op — so the CLI can print the two cases distinctly while both exit 0.

- **A different sha256 at the same path is a new derivation.** Because the key is
  the record's content hash, not its path, re-writing a `job.json` in place with
  different content (a genuinely re-run job) falls through the no-op check and
  imports as a second derivation. Same path, same bytes → no-op; same path,
  different bytes → new derivation.

- **Per-path asset dedupe.** A stem (or auto-registered input) whose resolved
  root-relative path is already registered with a matching hash reuses that
  asset — no duplicate row, and the derivation links the existing id. This
  matches on path, not hash, so identical bytes at two different paths stay two
  distinct assets. Stem paths are checked against both the manifest's existing
  assets and any minted earlier in the same run.

- **A registered path whose recorded hash conflicts with the file refuses.** When
  a path is already registered but its recorded `sha256` disagrees with the bytes
  import would record, import refuses naming the path and both hashes
  (`StemPathConflict` / `InputPathConflict`) rather than shadowing the manifest
  with a contradicting row. For the input this can only arise in the
  auto-register branch: hash resolution (ADR-0009) already found no asset matching
  the input's hash, so a path match there necessarily carries a different hash.

- **Refusals precede the one atomic write (ADR-0002).** As with every prior
  slice, the conflict refusals are raised before the single canonical atomic
  write, so a refused import leaves the manifest byte-identical and is free to
  retry once the conflict is resolved.

## Consequences

- Scripts can re-run `import` safely: an unchanged job is a no-op, so a pipeline
  that imports on every build neither errors nor grows the manifest.
- Overlapping imports (a stem already `add`ed, or shared between runs staged in
  the same folder) reuse the existing asset instead of duplicating it, while a
  drifted file on a registered path is caught loudly instead of silently
  recorded.
- The M2 import slices are now feature-complete for the milestone's definition of
  done; remaining PRD items (docs page, richer coverage) are documentation and
  test surface, not new import behavior.
