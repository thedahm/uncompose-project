# Architecture Decision Records

This directory holds `uncompose-project`'s own numbered ADRs, starting at `0001`.

Each ADR records a decision this repo owns — what was chosen, the alternatives, and the
reasoning — in a file named `NNNN-short-slug.md`. Ecosystem-wide decisions live in the
[`thedahm/uncompose`](https://github.com/thedahm/uncompose) ADR series and are cited by
number where they apply here (e.g. manifest schema core uncompose#62, versioning
uncompose#64, dispatch uncompose ADR-0005).

## Records

- [0001 — Adopt manifest schema v0](0001-adopt-manifest-schema-v0.md)
- [0002 — Canonical, atomic manifest writes](0002-canonical-atomic-manifest-writes.md)
- [0003 — `add`: hashing, slug ids, and path validation](0003-add-command-hashing-slugs-path-validation.md)
- [0004 — maturin wheel packaging and root dispatch conformance](0004-maturin-wheel-packaging-and-dispatch-conformance.md)
- [0005 — Strict manifest reads and `ext` pass-through](0005-strict-manifest-reads-and-ext-passthrough.md)
- [0006 — `show`: human overview and `--json`](0006-show-command-human-overview-and-json.md)
