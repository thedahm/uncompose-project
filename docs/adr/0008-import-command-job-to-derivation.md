# ADR-0008: `import` — job.json to input, stems, and a derivation

Status: Accepted — 2026-08-08

## Context

`import <job.json>` reads a completed `uncompose` job record and lands the whole
separation in the manifest in one step: the source input, the output stems, and a
derivation that ties them together with a hashed reference to the `job.json` as
evidence. It implements the M2 import contract (PRD thedahm/uncompose-project#11,
uncompose#63). This ADR records the decisions of the first slice — the happy path
(one in-tree input, N stems, one derivation) plus the two refusals that guard it
(non-success outcome, input hash mismatch). Later slices add hash-match input
reuse, idempotency on the `job.sha256`, out-of-tree refusals, stem dedupe, and the
richer `show` rendering.

## Decision

- **The job record is evidence, parsed tolerantly.** `job.json` is owned by
  `uncompose`, not this tool, so its deserializer requires only the fields the
  contract consumes (`input_path`, `input_sha256`, `preset`, `stems`,
  `engine_version`, `outcome`, `finished_at_unix`) and tolerates unknown extras —
  deliberately unlike the manifest's `deny_unknown_fields` policy (ADR-0005), so
  `uncompose` can grow its record without breaking older importers.

- **A non-success run is never provenance.** An `outcome` other than `"success"`
  refuses with the outcome shown, before any resolution or write.

- **The in-tree input auto-registers as `mix`, re-hashed against the record.** The
  input resolves relative to the project root under the same canonicalize-and-
  confine rule `add` uses (ADR-0003); its current bytes are re-hashed and must
  match the recorded `input_sha256`, or import refuses naming both hashes, so a
  file changed since the separation is caught rather than silently recorded.

- **Each stem registers as a `stem` asset, hashed at import.** Stems live as
  `<name>.wav` in the job folder (the directory holding the `job.json`); ids are
  minted from the stem name with the shared slugify/mint rules, disambiguated
  against both existing ids and ids minted earlier in the same import.

- **One derivation records the relationship; the job.json is referenced, not
  absorbed.** `tool: "uncompose"`, `tool_version` = `engine_version`, `created_at`
  = `finished_at_unix` rendered whole-second RFC3339 UTC, `inputs` = the input
  asset, `outputs` = the stem assets, `params` = `{"preset": …}` only. Everything
  else (models, device, timings) stays in the `job.json` behind a hashed `job`
  ref (`{path, sha256}`, root-relative path, sha256 of exact bytes). The
  `job.json` is not a registered asset. The derivation id is minted from the job
  folder name.

- **Refusals precede the one atomic write.** As with every command, a missing/
  unreadable/malformed record, a non-success outcome, an input hash mismatch, and
  any file resolving outside the root all return before the single canonical
  atomic write (ADR-0002), so a refused import leaves the manifest byte-identical.

## Consequences

- Import re-hashes the input and every stem, so cost scales with total bytes;
  acceptable for local, interactive use.
- The hashed `job` ref keeps the manifest small while the full record stays
  reachable and tamper-evident — a manifest reader can detect a `job.json`
  edited after import.
- `verify` (ADR-0006) and `show` (ADR-0007) cover imported assets and derivations
  with no extra work: they are ordinary assets and an ordinary derivation. The
  richer `show` rendering (preset, job ref) and the remaining contract behaviors
  are deferred to later M2 slices.
