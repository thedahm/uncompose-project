# ADR-0012: `import` dispatch on schema URL and the evaluation entry

Status: Accepted — 2026-08-09

## Context

M2 gave `import` one job: read a completed `uncompose` job record and land the
separation as a derivation. M5 (PRD thedahm/uncompose-project#26, spec
uncompose#89, contract uncompose#65) adds a second kind of evidence — a
**comparison record** written by `uncompose-compare` (schema
`https://uncompose.org/schemas/compare/v0/uncompose.compare.schema.json`) — that a
project should be able to record as an **evaluation**: a verdict over assets it
already tracks.

The integrating tools call one verb. Rather than a second subcommand, `import`
should stay "import an evidence file" and decide what the file is from its
contents, so the cross-tool argv and the human short form are identical for both
kinds and old job folders (which have no `schema` field) stay importable forever.

## Decision

- **Dispatch on the file's top-level `schema` string.** After `import` locates,
  reads, and hashes the file (ADR-0011's pre-lock work), it parses it to a generic
  JSON value and inspects `schema`:
  - **absent** → a job record; the M2 path, byte-for-byte unchanged.
  - **the compare v0 URL** → an evaluation import.
  - **any other value** → refused as `UnrecognizedImportSchema`, naming the URL
    found.

  Parsing to a value first means invalid JSON still surfaces as the existing
  "malformed job argument" error, and the dispatch needs neither record type
  modeled to make its choice.

- **Validate the record against the compare v0 schema, then read it.** A compare
  record is evidence this repo does not own, so the contract it is held to is
  Compare's own published one, not a subset invented here. The schema is vendored
  verbatim (`schemas/vendor/compare/v0/`, pinned in that directory's README),
  embedded with `include_str!`, and every record is validated whole before a single
  field is read out of it — the same crate and the same bytes `uncompose-compare`
  validates with when it writes the record, so what one tool accepts the other
  accepts. A violation refuses as `RecordNotConforming`, naming it.

  This replaces the earlier "parse tolerantly, ignore unknown fields" stance. That
  reading came from the job-record path, where `uncompose` publishes no schema — but
  compare v0 is `additionalProperties: false` throughout, with `ext` as its growth
  channel (uncompose#64), so tolerating unknown keys would not be forward
  compatibility, only a way to accept files Compare itself would reject. Referenced,
  not absorbed, still holds: validating a record reads nothing extra into the
  manifest, and observations, loops, and playback stay in the file behind the ref.

- **The evaluation entry.** On success `import` appends one `evaluations[]` entry:
  - `id` — a slug minted from the candidate asset ids as `<a>-vs-<b>`,
    disambiguated against existing evaluation ids by the shared numeric-suffix rule
    (`…-2`, `…-3`).
  - `candidates` — the referenced asset ids, in the record's candidate order.
  - `preference` — the record's `result.preference`, a candidate *label*, mapped
    **through the record's own `candidates[]`** (label → the candidate's `asset`) to
    an asset id; a null preference stays `null` (always serialized).
  - `confidence` — the record's `result.confidence`, copied verbatim when it
    carries one (compare v0 pins it to an integer 1–5, required exactly when the
    preference is non-null; the manifest schema leaves the type to Compare),
    omitted otherwise.
  - `created_at` — the record's `completed_at`.
  - `record` — `{path, sha256}`: the root-relative path and the sha256 of the
    record's exact bytes. **Reference the evidence, don't duplicate the verdict** —
    the same hashed-ref pattern the job ref uses, so a reader can detect a record
    edited after import and `verify` can police it.

- **Refusals, all before the single atomic write** (so a refused import leaves the
  manifest byte-identical):
  - a record that does not validate against compare v0 (`RecordNotConforming`,
    naming the first violation);
  - a candidate with no `asset` ref (`CandidateMissingAsset`) — `asset` is optional
    in compare v0, because a record can be written standalone, but v0.1 registers
    project-launched records only, where every candidate is an asset in this
    project;
  - a candidate asset id absent from the manifest (`UnknownAsset`, named);
  - a `result.preference` label matching no candidate (`UnknownPreference`, named)
    — the schema cannot express that cross-reference, and a preference that cannot
    be resolved would be a silently wrong verdict;
  - a record file outside the project root (`JobOutsideRoot`, via the shared
    `canonical_inside` confinement).

- **Idempotency on the record sha256.** An existing evaluation whose
  `record.sha256` matches is a stated no-op (`EvaluationAlreadyImported`, exit 0,
  no write). The same path with different bytes is a genuinely different verdict
  and appends a second evaluation — mirroring the job path's idempotency on
  `job.sha256` (ADR-0010).

- **The schema types the entry.** `evaluations[].items` is no longer the reserved
  open `{"type": "object"}`; it is a closed shape (`additionalProperties: false`,
  with the reserved namespace-slug-keyed `ext` on the entry and its `record`). The
  Rust `Evaluation`/`EvalRecord` structs are `deny_unknown_fields`, so an off-shape
  evaluation is rejected on read like every other object (ADR-0005), not silently
  carried. Required-and-nullable is read as required: `preference` must be present,
  `null` being the meaningful "no preference" verdict rather than an absence, so a
  manifest missing the key refuses instead of parsing and acquiring a `null` on the
  next rewrite.

- **`show` and `verify` cover evaluations.** `show`'s human overview grows an
  `Evaluations (N)` section (id, candidates, preference, confidence, record ref);
  `--json` stays verbatim manifest bytes. `verify` checks each evaluation's record
  file with the same missing/modified statuses as assets — but never stamps it (the
  manifest holds no size or `last_verified` for a record, only the hashed ref), so
  a deleted or edited comparison record fails the run just like a drifted asset.

## Consequences

- One verb imports both kinds of evidence; a job folder with no `schema` imports
  exactly as before, so nothing about the M2 path changes.
- A comparison record written into `<root>/evaluations/` becomes one manifest
  evaluation whose verdict summary is correct and whose hashed ref `verify` polices
  — the slice-2 definition of done.
- The compare record shape is no longer restated here or guessed at: it is whatever
  the vendored schema says (candidates carrying `{label, path, sha256, size}` plus
  an optional `asset`, the verdict under `result`, `completed_at` at the top level).
  What this repo commits to is the label→asset mapping and the refusal set. Fixtures
  in the suite are conforming records, so a shape drift shows up as failing tests
  rather than as an evaluation that quietly records no preference.
- Updating the vendored schema is a deliberate act (see `schemas/vendor/README.md`).
  A compare v1 would arrive as a different `$id`, and therefore as a different
  dispatch arm, not as a silent change under this one.
- `evaluations` is no longer reserved/empty: a manifest carrying a v0-shaped
  evaluation now round-trips, and one carrying an off-shape evaluation is refused
  on read.
