# Importing a job: the imported graph

`import` reads a completed `uncompose` job record and lands the whole separation in
your [manifest](../CONTEXT.md#language) in one step: the source **asset** it derived
from, each output stem as an asset, and one **derivation** tying them together with a
hashed reference to the `job.json` as evidence. After import, `verify` protects the
stems and the source like any other asset, and `show` renders the relationship.

This page is written in the project's glossary vocabulary — **asset**, **derivation**,
**role**, **integrity state**. See [`CONTEXT.md`](../CONTEXT.md#language) for those
terms.

## Running it

```sh
uncompose-project import path/to/run/job.json   # standalone
uncompose project import path/to/run/job.json   # via the root CLI — identical behavior
```

The one argument is the path to a `job.json` written by a completed `uncompose`
separation. It is relative to your current directory like every other command, and it
follows the same path rules as `add`: an absolute path is refused (even one that lands
inside the project), as is anything resolving outside the root. The job folder (the
directory holding the `job.json`) and every file it names must sit inside the project
root; import never copies files into the tree.

## What import records

A `job.json` is **evidence from another tool**: `uncompose` owns its format, so import
parses it tolerantly — it requires only the fields the contract consumes
(`input_path`, `input_sha256`, `preset`, `stems`, `engine_version`, `outcome`,
`finished_at_unix`) and ignores any extra fields, so newer `uncompose` records still
import. This is deliberately unlike the manifest, which rejects unknown fields.

From a successful record, import writes:

- **The source input, resolved by hash first.** Import looks for a registered asset
  whose `sha256` already equals the job's `input_sha256`. A hash match wins regardless
  of the job's `input_path`: the derivation links to that asset and nothing new is
  registered, so re-importing work on a mix you already registered — under any name or
  path — links to it rather than duplicating it. Only when no asset matches does import
  resolve the job's `input_path` and **auto-register the in-tree file with role `mix`**,
  after re-hashing its current bytes and confirming they still match `input_sha256`.
- **Each stem as a `stem` asset, hashed at import.** Stems are `<name>.wav` files in the
  job folder. Each registers with role `stem`, its sha256 + size captured now. A stem
  whose path is already registered with a matching hash is reused, not duplicated.
- **One derivation** recording the relationship: `tool: "uncompose"`,
  `tool_version` = the job's `engine_version`, `created_at` = `finished_at_unix`
  rendered as whole-second RFC3339 UTC, `inputs` = the one resolved input asset,
  `outputs` = the stem assets, and `params` = `{"preset": …}` only. Everything else the
  job knows — models, device, timings — stays in the `job.json` behind a hashed `job`
  reference (`{path, sha256}`, root-relative path plus the sha256 of the exact bytes).
  The derivation id is minted from the job folder name.

### Reference the evidence; don't duplicate the verdict

The manifest does not absorb the job record's contents. It keeps only what it needs to
describe the graph (tool, version, preset, timestamp, the input and output ids) and
points at the `job.json` through a hashed reference for the rest. The `job.json` is
**not** registered as an asset — it is evidence, not an output.

This keeps the manifest small while the full record stays reachable and
**tamper-evident**: because the reference carries the sha256 of the exact bytes, a
manifest reader can detect a `job.json` edited after import. Copying the record's fields
into the manifest would duplicate a verdict that could then silently drift from its
source; a hashed reference records the verdict once, where it was made, and lets anyone
re-check it.

## Reading the summary

A successful import prints the derivation, the resolved input, each stem, and the
derivation id — with a tag on each line saying whether import **registered** that file
or linked one the manifest already held:

```
Imported 'run-2026-08-08' (2 stems: 1 registered, 1 reused)
  input:      mix (audio/mix.wav) [resolved to an existing asset]
  stem:       vocals (audio/run/vocals.wav) [registered]
  stem:       drums (audio/run/drums.wav) [reused]
  derivation: run-2026-08-08
```

The input reads `[registered]` when import auto-registered it and
`[resolved to an existing asset]` when a registered asset already matched the job's
`input_sha256`; a stem reads `[registered]` or `[reused]` on the same distinction by
path. The header repeats the split as a tally.

From this you can trust the result without opening the manifest: which asset the source
resolved to (or was registered as), how many stems landed, where, and which of them were
already under the manifest's protection, and the derivation that ties them together.
`show` renders the same graph later, adding the `preset:` and `job:` lines from the
derivation.

## Idempotency: safe re-runs

Import keys idempotency on the **`job.json` sha256**. Right after reading the record's
bytes, it checks the manifest's derivations for one whose `job.sha256` already matches.
A hit makes the import a **stated no-op**: exit 0, no manifest write, and a message
naming the existing derivation:

```
Already imported as 'run-2026-08-08'; job.json unchanged, nothing to do
```

Because the key is the record's content, not its path:

- **Same path, same bytes → no-op.** A pipeline that runs `import` on every build neither
  errors nor grows the manifest.
- **Same path, different bytes → new derivation.** Re-writing a `job.json` in place with
  different content (a genuinely re-run job) falls through the no-op check and imports as
  a second derivation, so a real re-run is never swallowed.

## What import refuses, and why

Every refusal is raised **before** the single atomic manifest write, so a refused import
leaves the manifest byte-identical and is free to retry once you fix the cause. Import
exits non-zero on every refusal (exit 0 only on a successful import or the stated no-op),
so pipelines can gate on it.

| Refusal | When | Why |
| --- | --- | --- |
| **Failed outcome** | The job's `outcome` is not `"success"` (the outcome is shown) | A run that did not complete successfully is never provenance. |
| **Bad record** | The `job.json` is missing, unreadable, unparsable, or missing a required field (the file and problem are named) | You can tell a bad path from a corrupt record. |
| **Absolute path** | The job argument, or the record's `input_path`, is absolute — even one landing inside the root | Import applies `add`'s path rules unchanged: paths are relative to the project root, so a project stays portable. |
| **Job outside the root** | The `job.json` (and so its job folder) resolves outside the project root | Every recorded path stays root-relative and portable. |
| **Stem outside the root** | A `<stem>.wav` resolves outside the project root | Same reason: the manifest never references files outside the tree. |
| **Out-of-tree input** | No registered asset matches by hash **and** the job's `input_path` resolves outside the root | Import never copies files in; it tells you to `uncompose-project add` the input first. |
| **Input hash mismatch** | An auto-registered in-tree input's current bytes no longer hash to `input_sha256` (both hashes named) | A file changed since the separation is caught, not silently recorded as the source. |
| **Input path conflict** | The input's path is already registered with a different sha256 (both hashes named) | The manifest is never left quietly contradicting the disk. |
| **Stem path conflict** | A stem's path is already registered with a different sha256 (both hashes named) | Same reason: a drifted file on a registered path is caught loudly. |

From these rules you can predict every case: a failed run, an absolute path, a job or
stem outside the root, an input that is neither registered nor in-tree, a file that has
drifted since the separation, or a path already registered with different bytes each
refuse with a clear message and no partial write. Everything else — a success whose input resolves by hash or
auto-registers in place, with stems that are new or already registered at a matching
hash — imports.

## After import

- **`verify`** re-hashes the imported stems and any auto-registered input against their
  recorded sha256 + size, reporting each asset's integrity state (verified, modified, or
  missing) like any other asset.
- **`show`** renders the derivation with its tool, version, inputs, outputs, `created`,
  `preset`, and the hashed `job` reference, so the imported graph is readable without
  opening JSON.

See the [ADRs](adr/) `0008`–`0010` for the decisions behind this behavior.
