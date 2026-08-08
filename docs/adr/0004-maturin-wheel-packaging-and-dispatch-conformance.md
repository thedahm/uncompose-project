# ADR-0004: maturin wheel packaging and root dispatch conformance

Status: Accepted — 2026-08-07

## Context

`uncompose-project` is a Rust CLI, but the Uncompose family distributes its
tools as `pip install`-able wheels so a user gets a working binary with one
command (PRD #1, story 23) and the root `uncompose` CLI can dispatch to them
(story 24, uncompose ADR-0005). We need a build that turns the CLI crate's
binary into a wheel, and we need to confirm the binary is a conformant
delegatee. The family reference is uncompose PR #50; this repo owns only the
wheel-build portion of that pattern — release/publish automation is M6
(uncompose#76), explicitly out of scope here.

## Decision

- **maturin, `bin` bindings.** `pyproject.toml` at the repo root uses the
  maturin build backend with `bindings = "bin"` and
  `manifest-path = "crates/cli/Cargo.toml"`. maturin compiles the CLI crate and
  packages its `uncompose-project` binary as a console entry point, so
  installing the wheel puts `uncompose-project` on PATH. There is no Python
  extension module and no Python source.
- **Single version source.** The distribution version is `dynamic = ["version"]`,
  read from `Cargo.toml`. The Rust workspace is the only place a version is
  declared, so there is nothing to keep in sync — no separate version-lockstep
  check is needed.
- **Linux x86_64 manylinux only for v0.1.** The binary links glibc; maturin
  auto-tags the highest manylinux policy it satisfies. No other targets are
  built.
- **Dispatch conformance is exec-transparency.** ADR-0005 root dispatch execs
  `uncompose-<sub> <args>` found on PATH, so `uncompose project <args>` is a bare
  exec of this binary with the remaining arguments. The binary needs only to
  answer `--version` and `--help` (both provided by clap) and to preserve
  arguments and exit codes — which an exec'd process does by construction. No
  dispatch-specific code lives in this repo.

## Consequences

- `uvx maturin build` produces a manylinux wheel; installing it into a clean
  venv yields a working `uncompose-project` binary.
- CI gains a `wheel` job that builds the wheel per-PR and runs
  `ci/smoke-wheel.sh`, which installs the wheel into a fresh venv and exercises
  `--version`, `--help`, `init`, and — through a minimal ADR-0005 dispatcher
  shim — delegated invocation with output and exit codes preserved. The same
  contract is asserted at the Rust CLI process boundary
  (`root_dispatch_delegates_preserving_args_and_exit_codes`).
- The real root `uncompose` is a separate, not-yet-published package, so the
  delegation check uses a shim that implements the ADR-0005 exec contract rather
  than the real dispatcher; if that contract changes, both the shim and this ADR
  must follow.
- Release automation (trusted publishing, PyPI pending publishers, version
  bumps) is deliberately absent; M6 owns it.
