# Known limitations

Honest gaps in `uncompose-project` v0.1, so you know what you're getting before
you install rather than after. Everything here is a deliberate v0.1 scope call,
not an oversight; each is tracked as a backlog issue for anyone who wants to pick
it up.

## Linux only

Wheels are built and published for Linux only (`ci/release-version.sh` builds
against `manylinux`). macOS and Windows are untested and unsupported for now.

## No `move` or `repair` — moved-path survival is passive

There's no command that relocates or re-links an asset. If a registered file
moves, `verify` reports it `missing` at its old path; nothing re-discovers it at
the new one automatically. What v0.1 *does* give you: identity is sha256 + size,
captured at registration and never derived from the path, so the information a
future `repair` command would need to relink a moved file by content is already
in the manifest. Writing that command is deferred past v0.1.

## No schema migration machinery

The manifest schema sits at `v0` for the whole pre-1.0 period, and breaking
changes may ship between releases without a URL bump (see the
[v0.1.0 release notes](releases/v0.1.0.md) and
[uncompose#64](https://github.com/thedahm/uncompose/issues/64)). There's no
`migrate` command in v0.1: `import` and every other command refuse a `schema`
URL they don't recognize outright rather than attempting to read it. If your
manifest predates a breaking change, the remedy is to regenerate it or hand-edit
it to match the current schema.
