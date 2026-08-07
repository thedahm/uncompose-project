//! The project manifest: its in-memory shape, canonical serialization, and the
//! `init`/`add` operations that write it atomically.
//!
//! Only what M1's commands need is modeled here. `init` mints an empty manifest
//! and `add` registers assets; derivations and evaluations are part of schema v0
//! (see the in-repo JSON Schema) but no M1 command populates them, so their
//! collections round-trip as opaque JSON until the import work that owns them
//! lands.
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
use serde_json::Value;
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
    pub derivations: Vec<Value>,
    pub evaluations: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<Value>,
}

/// The `project` object: identity minted once at init.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<Value>,
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
    pub ext: Option<Value>,
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
/// that reads before it writes (`add`, `verify`), so the strict-read policy lives
/// in one place. Every variant is raised before any write, leaving the manifest
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
    let mut manifest = load_manifest(root)?;

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
            mint_id(&slugify(stem), &existing_ids)
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
    let mut manifest = load_manifest(root)?;
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

/// Read and strictly parse the manifest at `root`. A missing manifest means the
/// directory is not a project. The read never best-effort-parses a manifest this
/// tool does not own: the `schema` URL must match `SCHEMA_URL` exactly, and any
/// plain field outside the v0 shape (i.e. not `ext`) is rejected. The `schema`
/// check runs first so an unrecognized manifest reports the version mismatch
/// rather than incidental shape errors.
fn load_manifest(root: &Path) -> Result<Manifest, LoadError> {
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
    serde_json::from_value(value).map_err(LoadError::Invalid)
}

/// Resolve `rel` against `root` and return its root-relative, forward-slash path,
/// refusing absolute paths and anything that resolves outside the root (via `../`
/// or a symlink). Canonicalizing both sides collapses `..` and follows symlinks,
/// so an escape shows up as a failed `strip_prefix`.
fn resolve_inside_root(root: &Path, rel: &Path) -> Result<String, AddError> {
    if rel.is_absolute() {
        return Err(AddError::AbsolutePath(rel.to_path_buf()));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|e| AddError::Unreadable(root.to_path_buf(), e))?;
    let canonical = root.join(rel).canonicalize().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => AddError::MissingFile(rel.to_path_buf()),
        _ => AddError::Unreadable(rel.to_path_buf(), e),
    })?;
    let inside = canonical
        .strip_prefix(&canonical_root)
        .map_err(|_| AddError::OutsideRoot(rel.to_path_buf()))?;
    let stored = inside
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok(stored)
}

/// Stream the file at `path` into a SHA-256 hasher, returning its lowercase hex
/// digest — the manifest's sha256 form — and byte length over the exact bytes on
/// disk.
fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let size = io::copy(&mut file, &mut hasher)?;
    let hex = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((hex, size))
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
fn mint_id(base: &str, taken: &HashSet<&str>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken.contains(candidate.as_str()) {
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
