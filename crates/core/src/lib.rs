//! Core library for `uncompose-project`.
//!
//! This crate owns manifest semantics — building, canonically serializing, and
//! writing the `uncompose.project.json` — for the thin CLI layered over it.

pub mod lock;
pub mod manifest;

pub use lock::{ProjectLock, LOCK_WAIT_NOTICE};
pub use manifest::{
    add, import, init, show, verify, AddError, Asset, AssetOrigin, AssetStatus, Derivation,
    ImportError, ImportOutcome, ImportReport, ImportedAsset, InitError, Integrity, Job, LoadError,
    Manifest, Project, ShowOutput, VerifyError, VerifyReport, DEFAULT_ROLE, SCHEMA_URL,
};

/// The fixed name of the project manifest at the root of an uncompose project.
pub const MANIFEST_FILENAME: &str = "uncompose.project.json";

/// The fixed name of the sidecar lock file mutating commands take (ADR-0011).
/// It sits beside the manifest at the project root, is created on first use, and
/// is never deleted.
pub const LOCK_FILENAME: &str = ".uncompose.project.lock";

/// Returns the tool's one-line description, shared by the CLI and future
/// `--help` output.
pub fn tagline() -> &'static str {
    "Record and verify the provenance of derived audio via a portable project manifest."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagline_is_non_empty() {
        assert!(!tagline().is_empty());
    }
}
