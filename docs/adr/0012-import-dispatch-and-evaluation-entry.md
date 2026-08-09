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

- **The evaluation entry.** A compare record is **evidence** (like a job record):
  parsed tolerantly (unknown fields ignored — observations, loops, and playback
  stay in the file), never absorbed. On success `import` appends one
  `evaluations[]` entry:
  - `id` — a slug minted from the candidate asset ids as `<a>-vs-<b>`,
    disambiguated against existing evaluation ids by the shared numeric-suffix rule
    (`…-2`, `…-3`).
  - `candidates` — the referenced asset ids, in the record's candidate order.
  - `preference` — the record's preferred candidate label, mapped **through the
    record's own `candidates[]`** (label → the candidate's `asset`) to an asset id;
    a null/absent preference stays `null` (always serialized).
  - `confidence` — copied verbatim when the record carries one (its type is owned
    by the compare schema), omitted otherwise.
  - `created_at` — the record's `completed_at`.
  - `record` — `{path, sha256}`: the root-relative path and the sha256 of the
    record's exact bytes. **Reference the evidence, don't duplicate the verdict** —
    the same hashed-ref pattern the job ref uses, so a reader can detect a record
    edited after import and `verify` can police it.

- **Refusals, all before the single atomic write** (so a refused import leaves the
  manifest byte-identical):
  - a candidate with no `asset` ref (`CandidateMissingAsset`) — v0.1 registers
    project-launched records only, where every candidate is an asset in this
    project;
  - a candidate asset id absent from the manifest (`UnknownAsset`, named);
  - a `preference` label matching no candidate (`UnknownPreference`, named) — a
    preference that cannot be resolved would be a silently wrong verdict;
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
  carried.

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
- The exact compare record shape (candidate `{label, asset}` objects, top-level
  `preference` label, `confidence`, `completed_at`) is taken from uncompose#65 as
  landed in `uncompose-compare` M4; the label→asset mapping and the refusal set are
  what this repo commits to. Cross-repo conformance is exercised in a later slice.
- `evaluations` is no longer reserved/empty: a manifest carrying a v0-shaped
  evaluation now round-trips, and one carrying an off-shape evaluation is refused
  on read.
