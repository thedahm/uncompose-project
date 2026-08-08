# ADR-0009: `import` — input resolution by hash, out-of-tree and bad-record refusals

Status: Accepted — 2026-08-08

## Context

ADR-0008 landed the first slice of `import` (PRD thedahm/uncompose-project#11,
uncompose#63): the happy path plus the non-success-outcome and input-hash-mismatch
refusals, and it explicitly deferred hash-match input reuse, the out-of-tree
refusals, and the bad-record refusals. This ADR records the second slice, which
completes input resolution and the refusals guarding the job record and the files
it names.

## Decision

- **Input resolves by hash first, path second (uncompose#63).** Before touching
  the job's `input_path`, import looks for a registered asset whose `sha256`
  already equals the job's `input_sha256`. A match wins regardless of the job's
  `input_path`: the derivation links to that asset and no new asset is registered,
  so re-importing work on a mix registered under any name (or any path) links to
  it rather than duplicating it. Only when no asset matches does import fall back
  to resolving the job's `input_path` and auto-registering it as `mix` after
  re-hashing against the record (the ADR-0008 path).

- **An out-of-tree input refuses with a `project add` instruction.** When no
  registered asset matches by hash and the job's `input_path` resolves outside the
  project root (under the same canonicalize-and-confine rule `add` uses,
  ADR-0003), import refuses and tells the user to `add` it first. Import never
  copies files into the tree, so the manifest never references a file outside the
  root.

- **The job folder and every stem must sit inside the root.** A `job.json` (and
  therefore its job folder) or any `<stem>.wav` resolving outside the project root
  refuses, so every recorded path stays root-relative and portable.

- **Absolute paths refuse, exactly as in `add` (ADR-0003).** "The same rule `add`
  uses" includes its absolute-path refusal, not just the canonicalize-and-strip
  confinement: both the `job.json` argument and the record's own `input_path` are
  root-relative paths, so an absolute one refuses even when it would land inside
  the root. Accepting it would make `import` quietly laxer than `add` about the
  one path contract every command shares, and would let a job record's
  machine-specific absolute path resolve on the machine that wrote it.

- **Bad records fail naming the file and the cause.** A missing, unreadable, or
  unparsable `job.json`, or one missing a field the contract consumes, produces a
  distinct error naming the file and the problem — a bad path is told apart from a
  corrupt record. Unknown extra fields remain tolerated (ADR-0008): `uncompose`
  owns the record format and may grow it.

- **Refusals precede the one atomic write (ADR-0002).** Every new refusal is
  raised before the single canonical atomic write, so a refused import leaves the
  manifest byte-identical and is free to retry once the cause is fixed.

## Consequences

- Hash resolution makes import order-independent: whether the mix was `add`ed
  first or resolves from the job's own `input_path`, the same asset is linked and
  never duplicated.
- Because a hash match wins regardless of `input_path`, an in-tree file whose
  bytes match a registered asset is never re-registered even at a different path;
  identical bytes at a distinct path do not create a second input asset for the
  same import.
- Per-path stem dedupe/conflict and idempotency on the `job.sha256` remain for a
  later slice; this slice does not yet make a repeated import a stated no-op.
