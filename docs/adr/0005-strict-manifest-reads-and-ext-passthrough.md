# ADR-0005: Strict manifest reads and `ext` pass-through

Status: Accepted — 2026-08-07

## Context

Every command that reads a manifest (in M1, only `add`) must never
best-effort-parse a file this tool does not own, and third-party extensions must
be safe to carry across a rewrite. The schema v0 (ADR-0001, uncompose#64) already
fixes three properties: `schema` is an absolute URL compared by exact string
match, every object sets `additionalProperties: false`, and each object may carry
one reserved `ext` key — a namespace-slug-keyed, opaque extension subtree. This
ADR records how the read path enforces the first two and preserves the third.

## Decision

- **`schema` is matched exactly, first.** On read, the manifest is parsed as JSON,
  then its `schema` field is compared byte-for-byte against `SCHEMA_URL`. A missing
  or differing value is refused (`UnrecognizedSchema`) with an error naming both
  what was found and the expected URL — no version-range cleverness. The check runs
  before shape validation so an unrecognized (e.g. from-the-future) manifest reports
  the version mismatch, not an incidental field error.

- **Unknown plain fields are rejected.** `Manifest`, `Project`, and `Asset` derive
  `#[serde(deny_unknown_fields)]`, so a plain field outside the modeled v0 set fails
  deserialization (`Invalid`) with serde's message naming the offending field. Typos
  and from-the-future keys surface instead of being silently dropped. `derivations`
  and `evaluations` remain opaque `Vec<Value>` (no M1 command populates them), so
  strictness there arrives with the import work that models them.

- **`ext` passes through verbatim.** Each modeled object gains an
  `ext: Option<Value>` field (serialized last, per the schema's canonical order,
  and skipped when absent so empty manifests are byte-identical to before). Reading,
  modifying, and rewriting a manifest carries every `ext` subtree — at project,
  asset, and derivation level — through untouched. To keep "verbatim" literal rather
  than merely "intact," `serde_json`'s `preserve_order` feature is enabled so an
  opaque blob's key order is not normalized on rewrite.

- **Refusals precede the write.** All three refusals are raised in `load_manifest`,
  before any resolution or atomic write (ADR-0002), so a rejected read leaves the
  manifest byte-identical.

## Consequences

- This supersedes ADR-0003's note that `ext` is "preserved by the command that
  first writes them": `ext` pass-through is now a property of the read path itself,
  so every reading and rewriting command gets it for free. Optional *typed* fields
  the schema defines but no command yet models (e.g. `last_verified`) are still
  deferred to the command that owns them (verify, #7); until then a manifest
  carrying one is refused as an unknown field, which is correct for M1 since no M1
  command writes it.
- `preserve_order` pulls in `indexmap` and makes `serde_json::Value` order-
  preserving crate-wide. The canonical manifest is unaffected — its typed fields
  already serialize in declaration order — but opaque `Value` subtrees now keep
  their on-disk key order.
- Strict reads mean a hand-edited or foreign manifest with a stray field is refused
  rather than partially applied; the error names the field so the fix is obvious.
