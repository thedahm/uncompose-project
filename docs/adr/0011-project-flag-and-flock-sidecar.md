# ADR-0011: `--project` everywhere and the flock sidecar

Status: Accepted — 2026-08-09

## Context

Through M1–M2 every command assumed the project was the current working
directory. M5 integrates `uncompose-project` with the wider toolchain (PRD
thedahm/uncompose-project#26, spec uncompose#89, contracts uncompose#63 and
uncompose#67): another tool invokes it non-interactively, from an arbitrary cwd,
and more than one process may touch the same project at once. Two gaps had to
close before the later M5 slices (evaluation import, `separate --project`,
Compare handover) could build on this repo:

1. **Addressing the project explicitly.** The cross-tool caller needs to name the
   project root on the command line rather than relying on cwd, and the pinned
   argv `uncompose-project import --project <abs-root> <abs-file>` must work from
   any cwd with absolute paths.

2. **Concurrent mutation safety.** Two writers racing a read-modify-write could
   lose one another's write. The atomic temp+rename (ADR-0002) already prevents a
   torn file, but not a lost update.

## Decision

- **`--project <dir>` on every command, defaulting to `.`.** `init`, `add`,
  `import`, `verify`, and `show` all take a global `--project` flag (accepted
  before or after the subcommand). `<dir>` names the project root itself: the
  manifest must be exactly `<dir>/uncompose.project.json`, with **no upward
  walk** — a subdirectory of a project is not a project. The CLI canonicalizes
  `--project` to an absolute root once, up front, so every command names the same
  directory and a missing directory is an error there rather than a confusing
  "not a project" later. `NotAProject` now names the exact manifest path it
  looked for, so the no-upward-walk rule is legible in the error.

- **The pinned argv accepts absolute paths.** Because `import` is the cross-tool
  handoff target, its job argument accepts an absolute path as well as a
  root-relative one; confinement is enforced by resolution (`canonical_inside`),
  so an absolute path that resolves outside the root still refuses as
  `JobOutsideRoot`. This supersedes the M2 rule (ADR-0003/0009) that refused
  absolute job paths outright. The human short forms are unchanged: a relative
  `import <job>` still resolves against the project root, and `add`'s path
  argument keeps its relative-only contract. The record's own `input_path`
  (inside `job.json`) also keeps its absolute-path refusal — that is job content,
  not the cross-tool argv.

- **A flock sidecar serializes mutation.** Every read-modify-write of the manifest
  — `init`, `add`, `import`, and `verify`'s `last_verified` stamp — runs inside an
  exclusive advisory `flock(2)` on `<root>/.uncompose.project.lock` (created on
  first use, never deleted):

  - **Blocking wait, not fail-fast.** Acquisition tries once without blocking; if
    another holder has the lock, the command prints `waiting for project lock…`
    to stderr once and then blocks until the lock frees. A contended command
    queues behind the holder rather than erroring.
  - **Expensive work happens before acquisition.** `add` hashes the target file
    before taking the lock; `import` reads and hashes the job record and every
    stem before taking the lock, leaving only the manifest-dependent decisions
    (idempotency, input resolution, dedupe, id minting) and the single
    conditional input hash under it. The lock is then held only across a fresh
    manifest read and the atomic write, so hold times are milliseconds. The
    manifest is re-read *under* the lock so a concurrent mutator's writes are
    visible — that re-read is what turns "no torn file" into "no lost update".
  - **Crashed holders release automatically.** The advisory lock is tied to the
    open file description; the kernel drops it when the holder's file closes,
    including on a crash. No manual cleanup, no recovery command, no stale-lock
    detection.
  - **Readers take no lock; `verify`'s stamp write is not a read.** `show` and
    `verify` (and any external direct manifest read) hash and render without a
    lock; the canonical atomic write hands every reader a consistent snapshot.
    But `verify` does not only read: when an asset passes it refreshes that
    asset's `last_verified`, and a rewrite is a rewrite. The stamp is therefore
    taken like any other mutation — lock, re-read, write — while the expensive
    part, the hash pass over every asset, stays outside it. Treating the stamp as
    a reader would have been a lost-update bug, not a lost timestamp: the
    snapshot the hash pass runs on is stale by exactly as long as hashing takes,
    so writing it back would erase an `add`/`import` that committed in between.
    Only assets still carrying the sha256 they were verified against are stamped;
    one a mutator re-registered mid-pass was verified against bytes the manifest
    no longer claims.

- **Linux-only, so `flock` is safe.** v0.1 targets Linux (per the roadmap), where
  `flock` is universally available and well-behaved. The lock lives in the core
  crate (`crate::lock`) so every mutating op shares one implementation; the
  contention notice is emitted from there via the shared `LOCK_WAIT_NOTICE`
  constant. That keeps one notice with one wording next to the wait it describes,
  at the cost of the core writing a line to stderr — the one place it does. A
  contended `verify` can now print it too, which is honest: it is waiting for the
  same reason a contended `add` is.

## Consequences

- The cross-tool caller can run `uncompose-project import --project /abs/root
  /abs/job.json` from anywhere, and two concurrent `add`s never corrupt or lose a
  manifest write — the M5 slice-1 definition of done.
- A second mutator waits (with a visible notice) instead of failing, so scripts
  and the integrating tool can fire overlapping commands without retry logic.
- The lock file is a new, never-deleted artifact at the project root; it is
  advisory, so a process that ignores it (a hand-rolled writer) is not protected.
  This is acceptable for a single-tool, single-user v0.1.
- `import` still holds the lock across one small, often-skipped input hash (the
  hash-first input resolution is interleaved with manifest state); moving that
  final hash out of the critical section is a possible future refinement, not a
  correctness gap. Hashing it unconditionally before the lock is *not* that
  refinement: an input already registered under another name need not still exist
  at the path the job recorded, so the eager hash would turn imports that work
  today into `input file not found`.
- `verify` now creates the lock file (on a project where no mutation has happened
  yet) and can block behind a mutator. It still never blocks anyone during its
  hash pass, which is the long part; only the stamp write is serialized.
