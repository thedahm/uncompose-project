# ADR-0006: `show` — human overview and `--json`

Status: Accepted — 2026-08-07

## Context

`show` is the M1 read-only inspection command (PRD #1, issue #6): a user runs
`uncompose-project show` to see what the project knows at a glance, and
`show --json` to feed project state to other tools and shell pipelines without
parsing human-oriented text. It is the second command to read a manifest, after
`add`. Two questions this ADR settles: how strictly `show` reads, and what
`--json` actually emits.

## Decision

- **`show` reuses the same strict read as `add`.** The strict-read logic behind
  `add` (ADR-0005: exact `schema` URL match, unknown-field rejection) is extracted
  into a shared `ReadError` and a `load_manifest` that returns both the manifest's
  exact file bytes and the parsed `Manifest`. `add` maps `ReadError` into its own
  `AddError::Read`; `show` surfaces it directly. So `show` never
  best-effort-renders a manifest this tool does not own — an unrecognized `schema`
  URL or a stray field is refused with the same actionable error, whether you are
  adding or just looking.

- **`--json` emits the file verbatim, not a re-serialization.** `show --json`
  writes the manifest's exact on-disk bytes to stdout, byte-for-byte identical to
  the file. It does *not* re-serialize the parsed model, which would silently
  normalize a hand-authored or foreign manifest's whitespace and could drop
  anything the model does not round-trip. The manifest is still parsed first (the
  strict read above), so `--json` refuses an unrecognized manifest rather than
  echoing it — but what it emits on success is the raw bytes. This keeps `--json`
  a faithful pass-through: `show --json > copy` reproduces the manifest exactly.

- **The human overview is rendered in core, printed by the CLI.** Core builds the
  overview string (project header, then assets and derivations, each with an
  explicit count so an empty collection reads as `(0): none` rather than being
  omitted); the thin CLI chooses between printing it and writing the raw JSON
  bytes. Output shape is asserted at the CLI seam, the one test boundary (PRD
  testing decisions).

- **Derivations are rendered from their opaque JSON.** No M1 command creates a
  derivation, so the model keeps `derivations` as an opaque `Vec<Value>`
  (ADR-0005). `show` renders each by reading the schema v0 fields it displays
  (`id`, `tool`, optional `tool_version`, `inputs`, `outputs`, `created_at`)
  directly from the value, degrading a missing/off-type field to a placeholder
  rather than panicking. A hand-authored manifest carrying a schema-valid
  derivation displays correctly without `show` having to model — or validate — the
  derivation shape, which the import work (M2) owns.

## Consequences

- The read path is now shared by two commands, so a future reader (verify, #7)
  inherits the strict `schema`/unknown-field checks by calling `load_manifest`.
- `AddError` no longer owns the read-path variants; they live on `ReadError` and
  reach `AddError` through `AddError::Read`. The CLI's error text is unchanged
  (the `Display` messages moved with the variants), so existing stderr assertions
  hold.
- Because `--json` is a byte-exact pass-through, it preserves `ext` subtrees,
  key order, and any not-yet-modeled fields a future manifest might carry — the
  same "verbatim, not merely intact" property ADR-0005 established for rewrites,
  now for reads.
- `show` renders derivations without validating them; a malformed derivation in a
  hand-edited manifest displays with placeholders instead of being refused. The
  strict validation of the derivation shape is deferred to the command that models
  it (M2 import), consistent with ADR-0005 leaving `derivations` opaque in M1.
