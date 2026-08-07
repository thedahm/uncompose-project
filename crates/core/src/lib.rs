//! Core library for `uncompose-project`.
//!
//! This crate owns manifest semantics — building, canonically serializing, and
//! writing the `uncompose.project.json` — for the thin CLI layered over it.

pub mod manifest;

pub use manifest::{
    add, init, show, verify, AddError, Asset, AssetStatus, InitError, Integrity, LoadError,
    Manifest, Project, ShowOutput, VerifyError, VerifyReport, DEFAULT_ROLE, SCHEMA_URL,
};

/// The fixed name of the project manifest at the root of an uncompose project.
pub const MANIFEST_FILENAME: &str = "uncompose.project.json";

/// Returns the tool's one-line description, shared by the CLI and future
/// `--help` output.
pub fn tagline() -> &'static str {
    "Record and verify the provenance of derived audio via a portable project manifest."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_filename_is_fixed() {
        assert_eq!(MANIFEST_FILENAME, "uncompose.project.json");
    }

    #[test]
    fn tagline_is_non_empty() {
        assert!(!tagline().is_empty());
    }
}
