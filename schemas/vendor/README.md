# Vendored schemas

Schemas this repo **does not own** but must validate against, pinned as exact
bytes so the contract cannot drift under us silently. This repo's own published
schema lives one level up in [`schemas/project/`](../project/); nothing here is
published by `uncompose-project`.

| File | Owner | Pinned from |
|---|---|---|
| `compare/v0/uncompose.compare.schema.json` | [`thedahm/uncompose-compare`](https://github.com/thedahm/uncompose-compare) (`schemas/compare/v0/`) | `main` @ `5405a7b` |

`import` embeds the compare schema with `include_str!` and validates every
comparison record against it before mapping the verdict into an evaluation
(ADR-0012) — the same bytes `uncompose-compare` validates against when it writes
the record, so a record that one tool accepts the other accepts too.

Updating a vendored copy is a contract change: re-copy the file verbatim, update
the pin above, and re-run the suite — a shape change will surface as failing
import tests rather than as silently wrong manifest data.
