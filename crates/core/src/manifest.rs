//! The project manifest: its in-memory shape, canonical serialization, and the
//! `init`/`add` operations that write it atomically.
//!
//! Only what M1's commands need is modeled here. `init` mints an empty manifest
//! and `add` registers assets; no M1 command creates derivations or evaluations,
//! but both are parsed strictly against schema v0 and round-tripped, so an
//! off-shape record is rejected rather than silently carried and re-emitted.
//!
//! Reads are strict: the manifest's `schema` URL is matched exactly against
//! [`SCHEMA_URL`] and any plain field outside the v0 shape is rejected, so the
//! tool never best-effort-parses a manifest it does not own. The one reserved
//! exception is `ext` — an opaque, namespace-slug-keyed extension subtree legal on
//! every object — which is carried through read-modify-write verbatim (uncompose#64).

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use ulid::Ulid;

use crate::MANIFEST_FILENAME;

/// The absolute schema URL v0 manifests carry, compared by exact string match
/// (uncompose#64). Also the `$id` of the in-repo JSON Schema.
pub const SCHEMA_URL: &str =
    "https://uncompose.org/schemas/project/v0/uncompose.project.schema.json";

/// The role recorded when `add` is not given one. Open vocabulary; `mix` is the
/// common first thing you register in a derived-audio project.
pub const DEFAULT_ROLE: &str = "mix";

/// A project manifest in canonical field order.
///
/// `deny_unknown_fields` makes reads strict: a plain field outside this set (a
/// typo, a from-the-future key) is rejected rather than silently dropped. The one
/// reserved exception is `ext` — a namespace-slug-keyed, opaque extension subtree
/// legal on every object, carried through read-modify-write verbatim (uncompose#64).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub project: Project,
    pub assets: Vec<Asset>,
    pub derivations: Vec<Derivation>,
    /// Reserved: the item shape is owned by uncompose#63 (M2 import). Until then
    /// any object round-trips; a non-object item is rejected per the schema.
    pub evaluations: Vec<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<Map<String, Value>>,
}

/// The `project` object: identity minted once at init.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<Map<String, Value>>,
}

/// A file registered into the project. Its identity is its `sha256` + `size` over
/// exact bytes, captured at registration; `path` is a mutable location hint. Field
/// order matches the schema's canonical order.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub role: String,
    pub added_at: String,
    /// Cache of the last successful integrity check (RFC3339). Never a status
    /// claim — integrity is re-derived from disk on every `verify`. Absent until
    /// the asset first passes; skipped in serialization while absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<Map<String, Value>>,
}

/// A recorded transformation — inputs to outputs by a tool. No M1 command
/// creates one; they are read, shown, and round-tripped, held to the same strict
/// v0 shape as everything else so the tool never rewrites a manifest the
/// published schema rejects. Field order matches the schema's canonical order.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Derivation {
    pub id: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    /// Opaque to the schema; owned by the tool that wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Map<String, Value>>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<Job>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<Map<String, Value>>,
}

/// A hashed reference to an imported job.json — referenced, never absorbed.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub path: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<Map<String, Value>>,
}

impl Manifest {
    /// Build a fresh manifest for `name`, minting a ULID id and an RFC3339 UTC
    /// `created_at` truncated to whole seconds.
    pub fn new(name: impl Into<String>) -> Self {
        Manifest {
            schema: SCHEMA_URL.to_string(),
            project: Project {
                id: Ulid::new().to_string(),
                name: name.into(),
                created_at: now_rfc3339(),
                ext: None,
            },
            assets: Vec::new(),
            derivations: Vec::new(),
            evaluations: Vec::new(),
            ext: None,
        }
    }

    /// Serialize to canonical bytes: fixed field order, 2-space indent, trailing
    /// newline.
    pub fn to_canonical_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("manifest serializes");
        s.push('\n');
        s
    }
}

/// Current UTC time as an RFC3339 string truncated to whole seconds.
fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("0 is a valid nanosecond")
        .format(&Rfc3339)
        .expect("UTC datetime formats as RFC3339")
}

/// Why an `init` could not create a manifest.
#[derive(Debug)]
pub enum InitError {
    /// A manifest already exists at the given path; refuse rather than clobber.
    AlreadyExists(PathBuf),
    /// Writing the manifest failed.
    Io(io::Error),
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitError::AlreadyExists(path) => {
                write!(
                    f,
                    "{} already exists; refusing to reinitialize",
                    path.display()
                )
            }
            InitError::Io(e) => write!(f, "failed to write manifest: {e}"),
        }
    }
}

impl std::error::Error for InitError {}

/// Initialize `root` as a project: write a canonical manifest named after
/// `name`. Refuses if a manifest already exists, leaving it untouched. Returns
/// the manifest path on success.
pub fn init(root: &Path, name: &str) -> Result<PathBuf, InitError> {
    let manifest_path = root.join(MANIFEST_FILENAME);
    if manifest_path.exists() {
        return Err(InitError::AlreadyExists(manifest_path));
    }
    let bytes = Manifest::new(name).to_canonical_json();
    write_atomic(&manifest_path, bytes.as_bytes()).map_err(InitError::Io)?;
    Ok(manifest_path)
}

/// Why a manifest could not be read and strictly parsed. Shared by every command
/// that reads the manifest before acting (`add`, `verify`, `show`), so the
/// strict-read policy lives in one place; each command maps these into its own
/// error type. Every variant is raised before any write, leaving the manifest
/// byte-identical.
#[derive(Debug)]
pub enum LoadError {
    /// No manifest at the root; run `init` first.
    NotAProject(PathBuf),
    /// The manifest exists but could not be read (permissions, not a regular file).
    Unreadable(PathBuf, io::Error),
    /// The manifest on disk is not valid JSON.
    Parse(serde_json::Error),
    /// The manifest's `schema` is not the exact v0 URL this tool recognizes
    /// (missing, or some other value). Compared by exact string match, no
    /// version-range cleverness (uncompose#64). `found` is the declared value.
    UnrecognizedSchema { found: Option<String> },
    /// The manifest is valid JSON with the right schema URL but does not match the
    /// v0 shape — an unknown plain field (outside `ext`), a missing required field,
    /// or a wrong type. The wrapped error names the offending field.
    Invalid(serde_json::Error),
    /// An `ext` object carries a key that is not a namespace slug (uncompose#64).
    InvalidExtKey { owner: String, key: String },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotAProject(root) => write!(
                f,
                "{} is not an uncompose project; run `uncompose-project init` first",
                root.display()
            ),
            LoadError::Unreadable(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            LoadError::Parse(e) => write!(f, "the manifest is not valid JSON: {e}"),
            LoadError::UnrecognizedSchema { found } => match found {
                Some(url) => write!(
                    f,
                    "the manifest declares schema '{url}', which this tool does not recognize; expected '{SCHEMA_URL}'"
                ),
                None => write!(
                    f,
                    "the manifest has no string 'schema' field; expected '{SCHEMA_URL}'"
                ),
            },
            LoadError::Invalid(e) => {
                write!(f, "the manifest does not conform to schema v0: {e}")
            }
            LoadError::InvalidExtKey { owner, key } => write!(
                f,
                "ext key '{key}' on {owner} is not a namespace slug (lowercase letters, digits, '.', '_', '-'; must start with a letter or digit)"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// Why an `add` could not register an asset. Every variant is raised before any
/// write, so the manifest is left byte-identical on failure.
#[derive(Debug)]
pub enum AddError {
    /// The manifest could not be read/parsed (see [`LoadError`]).
    Load(LoadError),
    /// The path argument was absolute; paths are relative to the project root.
    AbsolutePath(PathBuf),
    /// The path resolves outside the project root (`../` or a symlink escape).
    OutsideRoot(PathBuf),
    /// No file exists at the given path.
    MissingFile(PathBuf),
    /// The file exists but could not be read (permissions, not a regular file).
    Unreadable(PathBuf, io::Error),
    /// The path is already registered by the named asset.
    DuplicatePath { path: String, existing_id: String },
    /// A `--id` (or `--role`) value is not a valid slug.
    InvalidSlug { what: &'static str, value: String },
    /// The requested `--id` is already used by another asset.
    IdInUse(String),
    /// Writing the manifest failed.
    Io(io::Error),
}

impl std::fmt::Display for AddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddError::Load(e) => write!(f, "{e}"),
            AddError::AbsolutePath(p) => write!(
                f,
                "{} is an absolute path; pass a path relative to the project root",
                p.display()
            ),
            AddError::OutsideRoot(p) => write!(
                f,
                "{} resolves outside the project root; only files inside the project can be added",
                p.display()
            ),
            AddError::MissingFile(p) => write!(f, "no such file: {}", p.display()),
            AddError::Unreadable(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            AddError::DuplicatePath { path, existing_id } => write!(
                f,
                "{path} is already registered as asset '{existing_id}'"
            ),
            AddError::InvalidSlug { what, value } => write!(
                f,
                "{what} '{value}' is not a valid slug (lowercase letters, digits, '.', '_', '-'; must start with a letter or digit)"
            ),
            AddError::IdInUse(id) => write!(f, "asset id '{id}' is already in use"),
            AddError::Io(e) => write!(f, "failed to write manifest: {e}"),
        }
    }
}

impl std::error::Error for AddError {}

impl From<LoadError> for AddError {
    fn from(e: LoadError) -> Self {
        AddError::Load(e)
    }
}

/// Register the file at `rel` (relative to `root`) as an asset: hash its exact
/// bytes, record size, root-relative forward-slash path, role, and an `added_at`
/// timestamp. The id is `id` if given (validated against the slug pattern),
/// otherwise a slug minted from the filename stem with a numeric suffix on
/// collision. Writes the updated manifest atomically and returns the new asset.
///
/// Refuses — leaving the manifest untouched — on an absolute or out-of-root path,
/// a missing/unreadable file, an already-registered path, or an invalid/taken id.
pub fn add(root: &Path, rel: &Path, id: Option<&str>, role: &str) -> Result<Asset, AddError> {
    let manifest_path = root.join(MANIFEST_FILENAME);
    let (_, mut manifest) = load_manifest(root)?;

    if !is_valid_slug(role) {
        return Err(AddError::InvalidSlug {
            what: "role",
            value: role.to_string(),
        });
    }

    let stored_path = resolve_inside_root(root, rel)?;

    if let Some(existing) = manifest.assets.iter().find(|a| a.path == stored_path) {
        return Err(AddError::DuplicatePath {
            path: stored_path,
            existing_id: existing.id.clone(),
        });
    }

    let existing_ids: HashSet<&str> = manifest.assets.iter().map(|a| a.id.as_str()).collect();
    let id = match id {
        Some(raw) => {
            if !is_valid_slug(raw) {
                return Err(AddError::InvalidSlug {
                    what: "id",
                    value: raw.to_string(),
                });
            }
            if existing_ids.contains(raw) {
                return Err(AddError::IdInUse(raw.to_string()));
            }
            raw.to_string()
        }
        None => {
            let stem = rel.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            mint_id_with(&slugify(stem), |c| existing_ids.contains(c))
        }
    };

    let (sha256, size) =
        sha256_file(&root.join(rel)).map_err(|e| AddError::Unreadable(rel.to_path_buf(), e))?;

    let asset = Asset {
        id,
        path: stored_path,
        sha256,
        size,
        role: role.to_string(),
        added_at: now_rfc3339(),
        last_verified: None,
        ext: None,
    };
    manifest.assets.push(asset.clone());

    let bytes = manifest.to_canonical_json();
    write_atomic(&manifest_path, bytes.as_bytes()).map_err(AddError::Io)?;
    Ok(asset)
}

/// The tool recorded on derivations created by `import`; the only producer of
/// job records this importer trusts (uncompose#63).
const IMPORT_TOOL: &str = "uncompose";
/// The role auto-registered inputs take.
const INPUT_ROLE: &str = "mix";
/// The role each imported stem takes.
const STEM_ROLE: &str = "stem";

/// A parsed `job.json` — the completion record `uncompose` writes into a job
/// folder. Only the fields the import contract consumes are modeled; unknown
/// fields are tolerated (not `deny_unknown_fields`), because the record format is
/// owned by `uncompose` and will grow, and import treats it as evidence rather
/// than a manifest it owns (uncompose#63).
#[derive(Deserialize)]
struct JobRecord {
    /// The separated input, as `uncompose` recorded it; resolved relative to the
    /// project root.
    input_path: String,
    /// The input's sha256 at separation time; the auto-registered file's current
    /// bytes must still hash to this.
    input_sha256: String,
    /// The separation preset; the sole entry in the derivation's `params`.
    preset: Value,
    /// Stem names; each is `<name>.wav` in the job folder.
    stems: Vec<String>,
    /// The engine version, recorded as the derivation's `tool_version`.
    engine_version: String,
    /// The run's outcome; only `"success"` imports.
    outcome: String,
    /// Completion time, whole Unix seconds; the derivation's `created_at`.
    finished_at_unix: u64,
}

/// The summary an `import` returns for the CLI to print: the resolved input, the
/// registered stems, and the id of the derivation that ties them together.
#[derive(Debug)]
pub struct ImportReport {
    pub input: Asset,
    pub stems: Vec<Asset>,
    pub derivation_id: String,
}

/// Why an `import` could not run. Every variant is raised before any write, so a
/// refused import leaves the manifest byte-identical and is free to retry.
#[derive(Debug)]
pub enum ImportError {
    /// The manifest could not be read/parsed (see [`LoadError`]).
    Load(LoadError),
    /// No `job.json` at the given path.
    JobMissing(PathBuf),
    /// The `job.json` exists but could not be read.
    JobUnreadable(PathBuf, io::Error),
    /// The `job.json` resolves outside the project root.
    JobOutsideRoot(PathBuf),
    /// The `job.json` is not valid JSON, or is missing a field import requires.
    MalformedJob(PathBuf, serde_json::Error),
    /// The job's `outcome` is not `"success"`; the value is shown.
    NotSuccess(String),
    /// `finished_at_unix` is not a representable timestamp.
    InvalidTimestamp(u64),
    /// The job's input file is missing.
    InputMissing(PathBuf),
    /// The job's input file exists but could not be read to hash it.
    InputUnreadable(PathBuf, io::Error),
    /// The job's input resolves outside the project root.
    InputOutsideRoot(PathBuf),
    /// The input's current bytes no longer hash to the job's `input_sha256`; both
    /// hashes are named.
    InputHashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    /// A stem file named by the job is missing from the job folder.
    StemMissing(PathBuf),
    /// A stem file exists but could not be read to hash it.
    StemUnreadable(PathBuf, io::Error),
    /// A stem resolves outside the project root.
    StemOutsideRoot(PathBuf),
    /// Writing the manifest failed.
    Io(io::Error),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Load(e) => write!(f, "{e}"),
            ImportError::JobMissing(p) => write!(f, "no such job record: {}", p.display()),
            ImportError::JobUnreadable(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            ImportError::JobOutsideRoot(p) => write!(
                f,
                "{} resolves outside the project root; job records must live inside the project",
                p.display()
            ),
            ImportError::MalformedJob(p, e) => {
                write!(f, "{} is not a valid job record: {e}", p.display())
            }
            ImportError::NotSuccess(outcome) => write!(
                f,
                "job outcome is '{outcome}', not 'success'; refusing to import a run that did not complete successfully"
            ),
            ImportError::InvalidTimestamp(secs) => write!(
                f,
                "job finished_at_unix {secs} is not a representable timestamp"
            ),
            ImportError::InputMissing(p) => write!(f, "input file not found: {}", p.display()),
            ImportError::InputUnreadable(p, e) => write!(f, "cannot read input {}: {e}", p.display()),
            ImportError::InputOutsideRoot(p) => write!(
                f,
                "input {} resolves outside the project root; `uncompose-project add` it first",
                p.display()
            ),
            ImportError::InputHashMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "input {path} now hashes to {actual} but the job records {expected}; the file has changed since the separation"
            ),
            ImportError::StemMissing(p) => write!(f, "stem file not found: {}", p.display()),
            ImportError::StemUnreadable(p, e) => write!(f, "cannot read stem {}: {e}", p.display()),
            ImportError::StemOutsideRoot(p) => write!(
                f,
                "stem {} resolves outside the project root",
                p.display()
            ),
            ImportError::Io(e) => write!(f, "failed to write manifest: {e}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<LoadError> for ImportError {
    fn from(e: LoadError) -> Self {
        ImportError::Load(e)
    }
}

/// Import a completed `uncompose` job at `job_arg` (relative to `root`): register
/// its input as a `mix` asset, each stem as a `stem` asset, and one derivation
/// recording the separation with a hashed reference to the `job.json`. The
/// `job.json` itself is referenced, never registered as an asset.
///
/// Refuses — leaving the manifest byte-identical — when the job record is
/// missing/unreadable/malformed, its `outcome` is not success, the input's
/// current bytes do not match the recorded `input_sha256`, or any referenced file
/// resolves outside the project root. Writes the updated manifest once, atomically.
pub fn import(root: &Path, job_arg: &Path) -> Result<ImportReport, ImportError> {
    let manifest_path = root.join(MANIFEST_FILENAME);
    let (_, mut manifest) = load_manifest(root)?;

    let canonical_root = root.canonicalize().map_err(ImportError::Io)?;

    // Locate and read the job record.
    let (job_abs, job_rel) =
        canonical_inside(root, &canonical_root, job_arg).map_err(|k| match k {
            ResolveKind::Missing => ImportError::JobMissing(job_arg.to_path_buf()),
            ResolveKind::Unreadable(e) => ImportError::JobUnreadable(job_arg.to_path_buf(), e),
            ResolveKind::Outside => ImportError::JobOutsideRoot(job_arg.to_path_buf()),
        })?;
    let job_bytes = std::fs::read(&job_abs)
        .map_err(|e| ImportError::JobUnreadable(job_arg.to_path_buf(), e))?;
    let job_sha256 = sha256_hex(&job_bytes);
    let job: JobRecord = serde_json::from_slice(&job_bytes)
        .map_err(|e| ImportError::MalformedJob(job_arg.to_path_buf(), e))?;

    // A run that did not complete successfully is never recorded as provenance.
    if job.outcome != "success" {
        return Err(ImportError::NotSuccess(job.outcome));
    }

    let created_at = unix_to_rfc3339(job.finished_at_unix)?;

    // Resolve the input, auto-register it as `mix`, and confirm its current bytes
    // still match what the separation hashed.
    let input_candidate = PathBuf::from(&job.input_path);
    let (input_abs, input_rel) = canonical_inside(root, &canonical_root, &input_candidate)
        .map_err(|k| match k {
            ResolveKind::Missing => ImportError::InputMissing(input_candidate.clone()),
            ResolveKind::Unreadable(e) => ImportError::InputUnreadable(input_candidate.clone(), e),
            ResolveKind::Outside => ImportError::InputOutsideRoot(input_candidate.clone()),
        })?;
    let (input_hash, input_size) = sha256_file(&input_abs)
        .map_err(|e| ImportError::InputUnreadable(input_candidate.clone(), e))?;
    if input_hash != job.input_sha256 {
        return Err(ImportError::InputHashMismatch {
            path: input_rel,
            expected: job.input_sha256,
            actual: input_hash,
        });
    }

    // Resolve and hash each stem; they live as `<name>.wav` in the job folder.
    let job_folder = job_abs
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf());
    let mut stems = Vec::with_capacity(job.stems.len());
    for name in &job.stems {
        let candidate = job_folder.join(format!("{name}.wav"));
        let (stem_abs, stem_rel) =
            canonical_inside(root, &canonical_root, &candidate).map_err(|k| match k {
                ResolveKind::Missing => ImportError::StemMissing(candidate.clone()),
                ResolveKind::Unreadable(e) => ImportError::StemUnreadable(candidate.clone(), e),
                ResolveKind::Outside => ImportError::StemOutsideRoot(candidate.clone()),
            })?;
        let (stem_hash, stem_size) = sha256_file(&stem_abs)
            .map_err(|e| ImportError::StemUnreadable(candidate.clone(), e))?;
        stems.push((name.clone(), stem_rel, stem_hash, stem_size));
    }

    // Mint ids against the manifest's existing asset ids plus the ones this
    // import is about to add, so a run with same-named stems does not collide.
    let now = now_rfc3339();
    let mut taken: HashSet<String> = manifest.assets.iter().map(|a| a.id.clone()).collect();

    let input_stem = Path::new(&input_rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let input_id = mint_id_with(&slugify(input_stem), |c| taken.contains(c));
    taken.insert(input_id.clone());
    let input_asset = Asset {
        id: input_id.clone(),
        path: input_rel,
        sha256: input_hash,
        size: input_size,
        role: INPUT_ROLE.to_string(),
        added_at: now.clone(),
        last_verified: None,
        ext: None,
    };

    let mut stem_assets = Vec::with_capacity(stems.len());
    for (name, path, sha256, size) in stems {
        let id = mint_id_with(&slugify(&name), |c| taken.contains(c));
        taken.insert(id.clone());
        stem_assets.push(Asset {
            id,
            path,
            sha256,
            size,
            role: STEM_ROLE.to_string(),
            added_at: now.clone(),
            last_verified: None,
            ext: None,
        });
    }

    // The derivation ties the input to the stems; `params` carries only the
    // preset, with the full record reachable behind the hashed `job` ref.
    let deriv_taken: HashSet<String> = manifest.derivations.iter().map(|d| d.id.clone()).collect();
    let folder_name = job_folder
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let derivation_id = mint_id_with(&slugify(folder_name), |c| deriv_taken.contains(c));

    let mut params = Map::new();
    params.insert("preset".to_string(), job.preset);

    let derivation = Derivation {
        id: derivation_id.clone(),
        inputs: vec![input_id],
        outputs: stem_assets.iter().map(|a| a.id.clone()).collect(),
        tool: IMPORT_TOOL.to_string(),
        tool_version: Some(job.engine_version),
        params: Some(params),
        created_at,
        job: Some(Job {
            path: job_rel,
            sha256: job_sha256,
            ext: None,
        }),
        ext: None,
    };

    manifest.assets.push(input_asset.clone());
    manifest.assets.extend(stem_assets.iter().cloned());
    manifest.derivations.push(derivation);

    let bytes = manifest.to_canonical_json();
    write_atomic(&manifest_path, bytes.as_bytes()).map_err(ImportError::Io)?;

    Ok(ImportReport {
        input: input_asset,
        stems: stem_assets,
        derivation_id,
    })
}

/// Why [`canonical_inside`] could not resolve a path; each caller maps it into a
/// context-specific error (`add`'s path, or import's input vs stem vs job record).
enum ResolveKind {
    /// No file exists at the resolved location.
    Missing,
    /// The file exists but could not be canonicalized (a read failure).
    Unreadable(io::Error),
    /// The file resolves outside the project root.
    Outside,
}

/// Resolve `candidate` (absolute, or relative to `root`) to its canonical path and
/// its root-relative forward-slash form, confirming it lives inside the root.
/// Canonicalizing both sides collapses `..` and follows symlinks, so an escape
/// surfaces as a failed `strip_prefix` — the one confinement rule every command
/// applies.
fn canonical_inside(
    root: &Path,
    canonical_root: &Path,
    candidate: &Path,
) -> Result<(PathBuf, String), ResolveKind> {
    let canonical = root
        .join(candidate)
        .canonicalize()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => ResolveKind::Missing,
            _ => ResolveKind::Unreadable(e),
        })?;
    let inside = canonical
        .strip_prefix(canonical_root)
        .map_err(|_| ResolveKind::Outside)?;
    let stored = inside
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok((canonical, stored))
}

/// Render whole Unix seconds as a whole-second RFC3339 UTC string, matching the
/// manifest's timestamp style.
fn unix_to_rfc3339(secs: u64) -> Result<String, ImportError> {
    let dt = i64::try_from(secs)
        .ok()
        .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
        .ok_or(ImportError::InvalidTimestamp(secs))?;
    dt.format(&Rfc3339)
        .map_err(|_| ImportError::InvalidTimestamp(secs))
}

/// The integrity of one asset, derived by re-checking disk against its recorded
/// identity. Never stored in the manifest — computed fresh on every `verify`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integrity {
    /// Size and sha256 both match the recorded values.
    Verified,
    /// The file exists but its size or contents no longer match.
    Modified,
    /// No file exists at the asset's path.
    Missing,
}

/// One asset's integrity outcome for a `verify` run: its id, path, and status.
#[derive(Debug, Clone)]
pub struct AssetStatus {
    pub id: String,
    pub path: String,
    pub integrity: Integrity,
}

/// The result of a `verify`: a per-asset integrity status in manifest order.
#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub statuses: Vec<AssetStatus>,
}

impl VerifyReport {
    /// Whether every asset verified. `verify` callers exit non-zero when this is
    /// false so scripts and CI can gate on project integrity.
    pub fn all_verified(&self) -> bool {
        self.statuses
            .iter()
            .all(|s| s.integrity == Integrity::Verified)
    }
}

/// Why a `verify` could not run to completion.
#[derive(Debug)]
pub enum VerifyError {
    /// The manifest could not be read/parsed (see [`LoadError`]).
    Load(LoadError),
    /// An asset's file exists but could not be read to hash it (permissions, a
    /// directory). A missing file is an [`Integrity::Missing`] status, not this.
    Unreadable(PathBuf, io::Error),
    /// Rewriting the manifest with refreshed `last_verified` timestamps failed.
    Io(io::Error),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::Load(e) => write!(f, "{e}"),
            VerifyError::Unreadable(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            VerifyError::Io(e) => write!(f, "failed to write manifest: {e}"),
        }
    }
}

impl std::error::Error for VerifyError {}

impl From<LoadError> for VerifyError {
    fn from(e: LoadError) -> Self {
        VerifyError::Load(e)
    }
}

/// Re-check every asset against the files on disk and report each as verified,
/// modified, or missing. Size is compared first (a cheap mismatch), then the
/// sha256. Assets that pass get their cached `last_verified` refreshed and the
/// manifest is rewritten canonically and atomically; integrity itself is never
/// stored. Refuses — leaving the manifest untouched — when the directory is not a
/// project or the manifest does not conform.
pub fn verify(root: &Path) -> Result<VerifyReport, VerifyError> {
    let manifest_path = root.join(MANIFEST_FILENAME);
    let (_, mut manifest) = load_manifest(root)?;
    let now = now_rfc3339();

    let mut statuses = Vec::with_capacity(manifest.assets.len());
    let mut changed = false;
    for asset in &mut manifest.assets {
        let integrity = check_integrity(root, asset)?;
        if integrity == Integrity::Verified {
            asset.last_verified = Some(now.clone());
            changed = true;
        }
        statuses.push(AssetStatus {
            id: asset.id.clone(),
            path: asset.path.clone(),
            integrity,
        });
    }

    // Only rewrite when a passing asset refreshed its timestamp; an all-failing
    // run leaves the manifest byte-identical.
    if changed {
        let bytes = manifest.to_canonical_json();
        write_atomic(&manifest_path, bytes.as_bytes()).map_err(VerifyError::Io)?;
    }

    Ok(VerifyReport { statuses })
}

/// Derive one asset's integrity from disk: size first (cheap), then sha256. A
/// file that is not there is [`Integrity::Missing`]; a size or content mismatch is
/// [`Integrity::Modified`]; a genuine read failure is a [`VerifyError`].
fn check_integrity(root: &Path, asset: &Asset) -> Result<Integrity, VerifyError> {
    // Stored paths are forward-slash and root-relative; rebuild per-OS components.
    let rel: PathBuf = asset.path.split('/').collect();
    let path = root.join(rel);

    let metadata = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Integrity::Missing),
        Err(e) => return Err(VerifyError::Unreadable(path, e)),
    };
    if metadata.len() != asset.size {
        return Ok(Integrity::Modified);
    }

    let (sha256, _) = sha256_file(&path).map_err(|e| VerifyError::Unreadable(path, e))?;
    if sha256 == asset.sha256 {
        Ok(Integrity::Verified)
    } else {
        Ok(Integrity::Modified)
    }
}

/// The result of `show`: the manifest's exact file bytes (for `--json`) and a
/// human-readable overview of the project, its assets, and its derivations.
pub struct ShowOutput {
    /// The manifest file's bytes, verbatim — emitted unchanged by `show --json`.
    pub raw: Vec<u8>,
    /// A human-readable overview, ending in a newline.
    pub overview: String,
}

/// Read the project at `root` (strictly, like `add`) and render it for `show`.
/// Returns the manifest's exact bytes alongside a human overview; the caller
/// picks which to print. Refuses a missing, unparsable, wrong-schema, or
/// off-shape manifest — `show` never best-effort-renders a manifest it does not
/// own.
pub fn show(root: &Path) -> Result<ShowOutput, LoadError> {
    let (raw, manifest) = load_manifest(root)?;
    Ok(ShowOutput {
        raw,
        overview: render_overview(&manifest),
    })
}

/// Render a manifest as a human-readable overview: the project header, then the
/// assets and derivations, each with a count so an empty collection is shown
/// explicitly rather than silently omitted.
fn render_overview(m: &Manifest) -> String {
    use std::fmt::Write;
    let mut o = String::new();

    let _ = writeln!(o, "Project: {}", m.project.name);
    let _ = writeln!(o, "  id:      {}", m.project.id);
    let _ = writeln!(o, "  created: {}", m.project.created_at);
    o.push('\n');

    if m.assets.is_empty() {
        let _ = writeln!(o, "Assets (0): none");
    } else {
        let _ = writeln!(o, "Assets ({}):", m.assets.len());
        for a in &m.assets {
            let _ = writeln!(o, "  {}  {}  ({}, {} bytes)", a.id, a.path, a.role, a.size);
            let _ = writeln!(o, "    sha256: {}", a.sha256);
            let _ = writeln!(o, "    added:  {}", a.added_at);
        }
    }
    o.push('\n');

    if m.derivations.is_empty() {
        let _ = writeln!(o, "Derivations (0): none");
    } else {
        let _ = writeln!(o, "Derivations ({}):", m.derivations.len());
        for d in &m.derivations {
            render_derivation(&mut o, d);
        }
    }

    o
}

/// Render one derivation: id and tool header, then inputs, outputs, and when it
/// was created.
fn render_derivation(o: &mut String, d: &Derivation) {
    use std::fmt::Write;
    let tool_label = match &d.tool_version {
        Some(version) => format!("{} v{version}", d.tool),
        None => d.tool.clone(),
    };
    let _ = writeln!(o, "  {}  ({tool_label})", d.id);
    let _ = writeln!(o, "    inputs:  {}", d.inputs.join(", "));
    let _ = writeln!(o, "    outputs: {}", d.outputs.join(", "));
    let _ = writeln!(o, "    created: {}", d.created_at);
}

/// Read and strictly parse the manifest at `root`, returning both the exact file
/// bytes (so a command can re-emit them verbatim) and the parsed manifest. A
/// missing manifest means the directory is not a project. The read never
/// best-effort-parses a manifest this tool does not own: the `schema` URL must
/// match `SCHEMA_URL` exactly, and any plain field outside the v0 shape (i.e. not
/// `ext`) is rejected. The `schema` check runs first so an unrecognized manifest
/// reports the version mismatch rather than incidental shape errors.
fn load_manifest(root: &Path) -> Result<(Vec<u8>, Manifest), LoadError> {
    let manifest_path = root.join(MANIFEST_FILENAME);
    let bytes = match std::fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(LoadError::NotAProject(root.to_path_buf()))
        }
        Err(e) => return Err(LoadError::Unreadable(manifest_path, e)),
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(LoadError::Parse)?;
    let found = value.get("schema").and_then(Value::as_str);
    if found != Some(SCHEMA_URL) {
        return Err(LoadError::UnrecognizedSchema {
            found: found.map(str::to_string),
        });
    }
    let manifest: Manifest = serde_json::from_value(value).map_err(LoadError::Invalid)?;
    validate_ext_keys(&manifest)?;
    Ok((bytes, manifest))
}

/// Enforce the one rule `ext` subtrees have: keys are namespace slugs
/// (uncompose#64). Values stay opaque. Serde cannot express a key pattern, so
/// this runs as a post-parse walk over every object that may carry `ext`.
fn validate_ext_keys(manifest: &Manifest) -> Result<(), LoadError> {
    check_ext("the manifest", manifest.ext.as_ref())?;
    check_ext("project", manifest.project.ext.as_ref())?;
    for a in &manifest.assets {
        check_ext(&format!("asset '{}'", a.id), a.ext.as_ref())?;
    }
    for d in &manifest.derivations {
        check_ext(&format!("derivation '{}'", d.id), d.ext.as_ref())?;
        if let Some(job) = &d.job {
            check_ext(&format!("derivation '{}' job", d.id), job.ext.as_ref())?;
        }
    }
    Ok(())
}

/// Check one `ext` object's keys against the slug pattern.
fn check_ext(owner: &str, ext: Option<&Map<String, Value>>) -> Result<(), LoadError> {
    if let Some(map) = ext {
        if let Some(key) = map.keys().find(|k| !is_valid_slug(k)) {
            return Err(LoadError::InvalidExtKey {
                owner: owner.to_string(),
                key: key.clone(),
            });
        }
    }
    Ok(())
}

/// Resolve `rel` against `root` and return its root-relative, forward-slash path
/// (via [`canonical_inside`]), refusing absolute paths and anything that resolves
/// outside the root (via `../` or a symlink).
fn resolve_inside_root(root: &Path, rel: &Path) -> Result<String, AddError> {
    if rel.is_absolute() {
        return Err(AddError::AbsolutePath(rel.to_path_buf()));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|e| AddError::Unreadable(root.to_path_buf(), e))?;
    let (_, stored) = canonical_inside(root, &canonical_root, rel).map_err(|k| match k {
        ResolveKind::Missing => AddError::MissingFile(rel.to_path_buf()),
        ResolveKind::Unreadable(e) => AddError::Unreadable(rel.to_path_buf(), e),
        ResolveKind::Outside => AddError::OutsideRoot(rel.to_path_buf()),
    })?;
    Ok(stored)
}

/// Stream the file at `path` into a SHA-256 hasher, returning its lowercase hex
/// digest — the manifest's sha256 form — and byte length over the exact bytes on
/// disk.
fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let size = io::copy(&mut file, &mut hasher)?;
    Ok((hex_digest(hasher), size))
}

/// SHA-256 hex digest of exact in-memory bytes — the manifest's sha256 form. Used
/// for the `job.json` reference, whose bytes are already in memory from parsing.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher)
}

/// Finish a SHA-256 and render its digest as lowercase hex.
fn hex_digest(hasher: Sha256) -> String {
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A character the schema slug pattern allows at the start: `[a-z0-9]`.
fn is_slug_start(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

/// A character the schema slug pattern allows after the start: `[a-z0-9._-]`.
fn is_slug_char(c: char) -> bool {
    is_slug_start(c) || matches!(c, '.' | '_' | '-')
}

/// Whether `s` matches the schema slug pattern `^[a-z0-9][a-z0-9._-]*$`.
fn is_valid_slug(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if is_slug_start(c)) && chars.all(is_slug_char)
}

/// Lowercase and sanitize `stem` into a slug: disallowed characters become `-`,
/// leading non-alphanumerics and trailing `-` are trimmed. Falls back to `asset`
/// when nothing usable remains.
fn slugify(stem: &str) -> String {
    let mut out = String::with_capacity(stem.len());
    for c in stem.chars() {
        let lc = c.to_ascii_lowercase();
        out.push(if is_slug_char(lc) { lc } else { '-' });
    }
    let trimmed = out
        .trim_start_matches(|c: char| !is_slug_start(c))
        .trim_end_matches('-');
    if trimmed.is_empty() {
        "asset".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Return `base` if free, otherwise `base-2`, `base-3`, … until one is unused.
/// `taken` reports whether a candidate id is already used; a closure so callers
/// with a growing set (import mints several ids in one pass) can answer against
/// ids minted earlier in the same run, not just the manifest's existing ones.
fn mint_id_with(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Write `bytes` to `target` atomically: a same-directory temp file, flushed to
/// disk, then renamed over `target`. A crash never leaves a truncated manifest.
fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let tmp = dir.join(format!(".{MANIFEST_FILENAME}.{}.tmp", Ulid::new()));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
