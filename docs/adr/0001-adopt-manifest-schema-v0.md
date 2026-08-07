# ADR-0001: Adopt manifest schema v0

Status: Accepted — 2026-08-07

## Context

`uncompose-project` records provenance in a single portable manifest,
`uncompose.project.json`. The manifest's shape was drafted in the `uncompose`
ecosystem under [manifest schema core (uncompose#62)][62] and
[versioning and extension fields (uncompose#64)][64], with the draft JSON Schema
living on the `wayfinder/62-manifest-schema` branch. That branch is the draft's
source, not its home: the schema becomes normative in *this* repo. We need one
in-repo copy that commands emit against and tests validate against, so the format
this tool owns has a single authority.

## Decision

Adopt schema **v0** as a normative JSON Schema file in this repo at
`schemas/project/v0/uncompose.project.schema.json`, mirroring its `$id` URL path.
It carries the draft's shapes, subsuming the uncompose#62 and uncompose#64
decisions this repo now owns:

- **Four top-level kinds**: `project` (ULID `id`, `name`, RFC3339 `created_at`),
  `assets[]`, `derivations[]`, `evaluations[]`. `evaluations` is reserved and
  empty in M1 (its item shape is owned by uncompose#63).
- **Asset**: `{id, path, sha256, size, role, added_at, last_verified?}`. Integrity
  state is derived at check time, never stored; the only persisted trace is the
  cached `last_verified` timestamp.
- **Derivation**: `{id, inputs[], outputs[], tool, tool_version?, params?,
  created_at, job?: {path, sha256}}`. Held and displayed in M1; no M1 command
  creates one (import is M2).
- **Constrained scalars**: slug `^[a-z0-9][a-z0-9._-]*$` for ids and roles;
  sha256 as 64 lowercase hex chars.
- **Versioning (uncompose#64)**: one required `schema` field, the absolute URL
  `https://uncompose.org/schemas/project/v0/uncompose.project.schema.json`,
  matched by exact string (`const` in the schema). An unrecognized URL is a clean
  error, never best-effort parsing. No migration machinery in v0.
- **Closed except `ext` (uncompose#64)**: `additionalProperties: false` on every
  object; a single reserved `ext` key, namespace-slug keyed and opaque, is legal
  on every object and preserved verbatim across read-modify-write. Unknown plain
  fields are invalid so typos and from-the-future files are caught, not silently
  dropped.

The manifest a command emits is validated against this file in tests
(the CLI-process-boundary seam). The URL is only an identifier in v0.1; actually
serving the schema there is M6.

## Consequences

- The rest of the Uncompose family can rely on v0 manifests at M5 integration.
- The schema and the Rust manifest model must stay in step; a conformance test at
  the CLI seam is the guard. In M1.2 only `init` emits a manifest (empty
  collections); `add`/import populate assets and derivations later against the
  same schema.
- Evolving the format means a new versioned schema and URL, not edits to v0.

[62]: https://github.com/thedahm/uncompose/issues/62
[64]: https://github.com/thedahm/uncompose/issues/64
