# Contributing to Uncompose Project

Thanks for your interest in `uncompose-project`. The project is pre-v0.1 and this
guide is intentionally minimal. It grows into a full contributor guide once v0.1
exists.

## Working on the code

The repo is a Cargo workspace: a `core` library crate that owns manifest semantics
and a thin `uncompose-project` CLI binary layered over it. Setup:

- A stable Rust toolchain; `cargo test` at the repo root runs everything.
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
  are the same gates CI enforces — run them before opening a pull request.

Development is test-first at the CLI process boundary: integration tests invoke the
compiled `uncompose-project` binary in a real temp project directory and assert on
exit codes, stdout/stderr, and the bytes of `uncompose.project.json`. No mocks, no
filesystem abstractions — real files in temp dirs. See
[`.sandcastle/CODING_STANDARDS.md`](.sandcastle/CODING_STANDARDS.md) for the full
testing and architecture rules.

## Governance

Uncompose is created and maintained by Dominic Hanzely ([@thedahm](https://github.com/thedahm)),
who acts as the project's maintainer and final decision-maker. Significant decisions are
recorded as numbered architecture decision records in [`docs/adr/`](docs/adr/), so the
reasoning behind the project's choices is public and reviewable. Ecosystem-wide decisions
live in the [`thedahm/uncompose`](https://github.com/thedahm/uncompose) ADR series and are
cited explicitly where they apply. Issues and pull requests are answered on a best-effort
basis.

## Documentation carries rationale, not narration

Code is the source of truth for what the project does; committed documentation exists to
carry what code cannot: the reasoning, the constraints, and the roads not taken. ADRs in
[`docs/adr/`](docs/adr/) are the home for "we did X instead of Y because". Comments state
constraints the code can't show. Structural docs (layout, vocabulary, standards) are
welcome. What we avoid is documentation that restates what code already says: it competes
with the source of truth and loses the moment either changes.

## Before opening a large pull request

Open an issue first. Discussing the change before you build it keeps you from investing
effort in something that conflicts with a recorded decision or the current milestone.
Small fixes (typos, broken links, obvious corrections) are welcome directly.

## Conduct

Participation in the project is covered by the [Code of Conduct](CODE_OF_CONDUCT.md).
