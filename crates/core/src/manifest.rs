//! The project manifest: its in-memory shape, canonical serialization, and the
//! `init`/`add` operations that write it atomically.
//!
//! Only what M1's commands need is modeled here. `init` mints an empty manifest
//! and `add` registers assets; derivations and evaluations are part of schema v0
//! (see the in-repo JSON Schema) but no M1 command populates them, so their
//! collections round-trip as opaque JSON until the import work that owns them
//! lands. Likewise, `ext` passthrough on read-modify-write arrives with the first
//! command that writes an `ext` subtree; nothing in M1 does.

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
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub project: Project,
    pub assets: Vec<Asset>,
    pub derivations: Vec<Value>,
    pub evaluations: Vec<Value>,
}

/// The `project` object: identity minted once at init.
#[derive(Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// A file registered into the project. Its identity is its `sha256` + `size` over
/// exact bytes, captured at registration; `path` is a mutable location hint. Field
/// order matches the schema's canonical order.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Asset {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub role: String,
    pub added_at: String,
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
            },
            assets: Vec::new(),
            derivations: Vec::new(),
            evaluations: Vec::new(),
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

/// Why an `add` could not register an asset. Every variant is raised before any
/// write, so the manifest is left byte-identical on failure.
#[derive(Debug)]
pub enum AddError {
    /// No manifest at the root; run `init` first.
    NotAProject(PathBuf),
    /// The manifest on disk could not be parsed.
    Parse(serde_json::Error),
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
            AddError::NotAProject(root) => write!(
                f,
                "{} is not an uncompose project; run `uncompose-project init` first",
                root.display()
            ),
            AddError::Parse(e) => write!(f, "the manifest is not valid JSON: {e}"),
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

    let (sha256, size) = hash_file(root, rel)?;

    let asset = Asset {
        id,
        path: stored_path,
        sha256,
        size,
        role: role.to_string(),
        added_at: now_rfc3339(),
    };
    manifest.assets.push(asset.clone());

    let bytes = manifest.to_canonical_json();
    write_atomic(&manifest_path, bytes.as_bytes()).map_err(AddError::Io)?;
    Ok(asset)
}

/// Read and parse the manifest at `root`. A missing manifest means the
/// directory is not a project.
fn load_manifest(root: &Path) -> Result<Manifest, AddError> {
    let manifest_path = root.join(MANIFEST_FILENAME);
    let bytes = match std::fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(AddError::NotAProject(root.to_path_buf()))
        }
        Err(e) => return Err(AddError::Unreadable(manifest_path, e)),
    };
    serde_json::from_slice(&bytes).map_err(AddError::Parse)
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

/// Stream the file at `root/rel` into a SHA-256 hasher, returning its lowercase
/// hex digest and byte length over the exact bytes on disk.
fn hash_file(root: &Path, rel: &Path) -> Result<(String, u64), AddError> {
    let path = root.join(rel);
    let mut file = File::open(&path).map_err(|e| AddError::Unreadable(rel.to_path_buf(), e))?;
    let mut hasher = Sha256::new();
    let size =
        io::copy(&mut file, &mut hasher).map_err(|e| AddError::Unreadable(rel.to_path_buf(), e))?;
    let digest = hasher.finalize();
    let hex = digest.iter().map(|b| format!("{b:02x}")).collect();
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
