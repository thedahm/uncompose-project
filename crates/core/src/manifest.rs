//! The project manifest: its in-memory shape, canonical serialization, and the
//! `init` operation that writes a fresh one atomically.
//!
//! Only what `init` needs is modeled here. Assets and derivations are part of
//! schema v0 (see the in-repo JSON Schema) but no M1 command populates them yet,
//! so their collections serialize as empty arrays until the `add`/import work
//! that owns them lands.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use ulid::Ulid;

use crate::MANIFEST_FILENAME;

/// The absolute schema URL v0 manifests carry, compared by exact string match
/// (uncompose#64). Also the `$id` of the in-repo JSON Schema.
pub const SCHEMA_URL: &str =
    "https://uncompose.org/schemas/project/v0/uncompose.project.schema.json";

/// A project manifest in canonical field order.
#[derive(Serialize)]
pub struct Manifest {
    pub schema: &'static str,
    pub project: Project,
    pub assets: Vec<Value>,
    pub derivations: Vec<Value>,
    pub evaluations: Vec<Value>,
}

/// The `project` object: identity minted once at init.
#[derive(Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

impl Manifest {
    /// Build a fresh manifest for `name`, minting a ULID id and an RFC3339 UTC
    /// `created_at` truncated to whole seconds.
    pub fn new(name: impl Into<String>) -> Self {
        let created_at = OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .expect("0 is a valid nanosecond")
            .format(&Rfc3339)
            .expect("UTC datetime formats as RFC3339");
        Manifest {
            schema: SCHEMA_URL,
            project: Project {
                id: Ulid::new().to_string(),
                name: name.into(),
                created_at,
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
