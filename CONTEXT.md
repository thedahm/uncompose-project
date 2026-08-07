# Uncompose Project

Local-first provenance for derived audio: turn a directory into a project with a single
portable manifest that records where each file came from and lets you verify it has not
silently changed.

## Language

**Manifest**:
The single file `uncompose.project.json` at the project root recording everything the
project knows. Plain, diffable JSON conforming to the published v0 schema; written
canonically (fixed field order, 2-space indent, trailing newline) via atomic temp+rename.
The manifest's directory _is_ the project root.
_Avoid_: metadata file, index, database

**Asset**:
A file registered into the project, recorded as `{id, path, sha256, size, role,
added_at, last_verified?}`. Its identity is its sha256 + size over exact bytes, captured
at registration; the `path` is a mutable location hint, not identity.
_Avoid_: file (when identity is meant), entry, item

**Derivation**:
A recorded relationship between assets — inputs producing outputs via a tool — shape
`{id, inputs[], outputs[], tool, tool_version?, params?, created_at, job?}`. The v0 schema
holds derivations and `show` displays them, but no M1 command creates one (import is M2).
_Avoid_: transform, edge, link

**Evaluation**:
A reserved kind for future quality/comparison records. `evaluations` is an empty array in
v0; its item shape is owned by uncompose#63 and lands post-M1.
_Avoid_: result, score, report

**Integrity state**:
Whether an asset is verified, modified, or missing — derived at check time by re-hashing
disk against the recorded sha256 + size, never stored. The only persisted trace is the
cached `last_verified` timestamp on assets that pass.
_Avoid_: status (as a stored field), state flag

**Role**:
What an asset is _for_ (`mix`, `stem`, `reference`, or a user slug) — an open vocabulary
recorded per asset, distinct from what the file _is_.
_Avoid_: type, kind, category
