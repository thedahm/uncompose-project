# ADR-0013: Tag-driven trusted release automation

Status: Accepted — 2026-08-10

## Context

ADR-0004 decided how a wheel is built and left publishing to M6. That is now here
(uncompose#92 stories 21–24, slice uncompose#97): `uncompose-project` v0.1.0 ships to
PyPI, and the way it ships has to be one auditable action a maintainer can trust six
months later, holding no credential worth stealing. The family constraint is
uncompose ADR-0008, which fixes the shape for every family package; this ADR records
what this repo does with it and why the alternatives were declined.

## Decision

- **A `vX.Y.Z` tag is the only thing that publishes.** `release.yml` triggers on the
  tag pattern and runs the release end to end — version check, gates, build, publish —
  as a single run, so the audit trail is one URL rather than a correlation exercise
  across four. Publishing from a manual run against the real index was rejected: a
  release should be a fact in the repository's history, not an act somebody performed.

- **Cargo.toml is the version, and a tag is a claim about it.** `ci/release-version.sh`
  reads `[workspace.package] version` — the same number `pyproject.toml` takes via
  `dynamic = ["version"]` (ADR-0004) — and refuses a tag that disagrees, in the first
  job, before anything is built. `ci/check-wheel.sh` then checks the *artifact*: the
  built wheel's distribution, version, and manylinux tag, so however the build got its
  version, a wheel that does not carry the released one never reaches PyPI. The two
  guards are shell, and their test (`ci/release-checks.test.sh`) drives them as
  processes and asserts exit codes and messages — the seam this repo tests everything
  at. Nothing is scaffolded around Actions internals.

- **Prerelease tags are refused, not translated.** Cargo's semver prereleases
  (`0.1.0-rc.1`) and PEP 440's (`0.1.0rc1`) do not spell the same version the same way,
  so a check that accepted them would have to invent a mapping and could no longer say
  honestly that the tag and the package version agree. Rehearsals go to TestPyPI from a
  manual run instead (`docs/releasing.md`), which is the same pipeline with a different
  index rather than a second, differently-shaped path to production.

- **The gate is `ci.yml` itself,** called with `workflow_call`. Restating the suite in
  the release workflow would let "the release ran the full suite" drift from what the
  suite is; calling it makes that impossible.

- **Trusted Publishing (OIDC), with a GitHub environment.** `pypa/gh-action-pypi-publish`
  exchanges a short-lived OIDC token for upload rights. No API token exists in this
  repository or on the release path, so there is none to leak, rotate, or scope wrong.
  The publish job runs in the `pypi` environment (`testpypi` for rehearsals) and holds
  `id-token: write`; every other job holds `contents: read`. The environment is part of
  the identity PyPI trusts, so it also becomes the place to add required reviewers if a
  release should ever need a second pair of eyes.

- **Attestations on.** PEP 740 attestations are signed with the same OIDC identity, so
  the published wheel names the repository, workflow, and commit that produced it.
  Story 23 asks for traceability that is verifiable rather than asserted; this is the
  form of it that a stranger can check from the index alone.

- **One Linux wheel, built in the manylinux container.** The v0.1 platform scope is
  Linux x86_64 (ADR-0004), so there is no matrix. The release build goes through
  `PyO3/maturin-action` with `manylinux: auto`, as uncompose's own release workflow
  does, rather than the plain `maturin build` the CI wheel lane uses: on the runner the
  binary is tagged against the runner's glibc, which quietly excludes every machine
  older than it. CI's lane keeps the simpler build — it is checking that the packaging
  still works, not producing the artifact anyone installs.

- **No sdist.** Here this repo departs from uncompose's release, which publishes one:
  an extension sdist would compile a Rust toolchain on a user's machine, including on
  platforms this release does not support, where the absence of a wheel is the clearer
  answer — and `uncompose-compare` cannot ship a buildable one at all, since its sdist
  would lack the frontend bundle its build embeds. The two extensions release together
  and are installed together; they publish the same shape.

- **The wheel that was proven is the wheel that is published.** The build job runs
  `ci/smoke-wheel.sh` against the artifact and uploads it; `publish` downloads exactly
  that and never rebuilds.

## Consequences

- Cutting a release is: bump the version, land it, push the tag. Everything else is
  the run, and everything it refuses, it refuses before publishing rather than after.
- PyPI (and TestPyPI) must be told which workflow they trust — a one-time human step
  per index, recorded in `docs/releasing.md`. Until that exists, `publish` fails with
  no token; the wheel is still built and proven.
- Renaming `release.yml`, or the environments, breaks publishing until the trusted
  publisher is updated to match. That is the mechanism working: the workflow's identity
  is part of the credential.
- A repeated TestPyPI rehearsal of the same version is refused by the index. The
  remedy is a version bump on the rehearsal branch, not a workaround in the pipeline.
