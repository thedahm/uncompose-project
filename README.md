# Uncompose Project

Local-first provenance for derived audio.

`uncompose-project` turns a directory into a project with a single portable manifest,
`uncompose.project.json`. When you derive audio from other audio — stem separations,
renders, edits — the results otherwise accumulate as loose files with no trustworthy
record of where they came from or whether they have silently changed. This tool records
each file's identity (sha256 + size) the moment you register it and lets you verify, at
any time, that the files on disk still match. The manifest is plain, diffable JSON
conforming to a published schema, so it outlives the tool and is readable by the rest of
the Uncompose family.

## Status

Pre-v0.1: the first release is being built in the open on the
[issue tracker](https://github.com/thedahm/uncompose-project/issues), with decisions
recorded in [`docs/adr/`](docs/adr/). The install line below goes live with the `v0.1.0`
tag.

## Install

```sh
pip install uncompose-project        # placeholder — lands with v0.1.0
```

## Commands

`init` creates a project, `add` registers a file as an asset, `verify` checks the
registered assets against disk, and `show` renders the manifest. `import` reads a
completed `uncompose` job record and lands its source, output stems, and their
derivation in one step — see [Importing a job](docs/import.md) for what it records, what
it refuses and why, and how re-import stays idempotent.

Every command takes `--project <dir>` (default `.`) to name the project root explicitly,
so you can operate on a project from any directory; it names the root itself, with no
search of parent directories. Each mutating command (`init`, `add`, `import`) serializes
its write behind an advisory lock at `<root>/.uncompose.project.lock`, so concurrent
writers wait rather than corrupt or lose a write.

## Responsible use

`uncompose-project` records and verifies audio you provide, entirely on your own machine —
nothing is uploaded anywhere, no accounts, no telemetry, no network I/O. You are
responsible for making sure you have the rights to the audio you register, and the rights
to what you do with it follow from the rights you hold in the input. Recording a file's
provenance does not grant you any rights to it.

## Family

`uncompose-project` is part of the [Uncompose](https://github.com/thedahm/uncompose)
family of local-first audio tools. It runs standalone as `uncompose-project`, and — per
the family dispatch contract (uncompose ADR-0005) — as `uncompose project <args>` from the
root CLI, including `--version` and `--help`.

## License

[MIT](LICENSE) © 2026 Dominic Hanzely
