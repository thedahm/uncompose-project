//! Behavior tests for `add` edge cases the CLI seam reaches poorly: path
//! validation (`../`, absolute, symlink escape) and slug disambiguation. These
//! drive the public `init`/`add` API against real files in temp dirs — never
//! internals — per the coding standards.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use uncompose_project_core::{add, init, AddError};

fn project() -> TempDir {
    let dir = TempDir::new().unwrap();
    init(dir.path(), "test-project").unwrap();
    dir
}

#[test]
fn add_refuses_a_parent_directory_escape() {
    let dir = project();
    // A real file outside the root, reached via `../`.
    let outside = dir.path().parent().unwrap().join("outside.wav");
    fs::write(&outside, b"x").unwrap();

    let err = add(dir.path(), Path::new("../outside.wav"), None, "mix").unwrap_err();
    assert!(matches!(err, AddError::OutsideRoot(_)), "got {err:?}");
    let _ = fs::remove_file(&outside);
}

#[test]
fn add_refuses_an_absolute_path() {
    let dir = project();
    fs::write(dir.path().join("song.wav"), b"x").unwrap();
    let abs = dir.path().join("song.wav");

    let err = add(dir.path(), &abs, None, "mix").unwrap_err();
    assert!(matches!(err, AddError::AbsolutePath(_)), "got {err:?}");
}

#[cfg(unix)]
#[test]
fn add_refuses_a_symlink_that_escapes_the_root() {
    let dir = project();
    // Target lives outside the root; a symlink inside the root points at it.
    let target = dir.path().parent().unwrap().join("secret.wav");
    fs::write(&target, b"x").unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join("link.wav")).unwrap();

    let err = add(dir.path(), Path::new("link.wav"), None, "mix").unwrap_err();
    assert!(matches!(err, AddError::OutsideRoot(_)), "got {err:?}");
    let _ = fs::remove_file(&target);
}

#[test]
fn add_mints_and_disambiguates_slugs_from_the_filename_stem() {
    let dir = project();
    for sub in ["a", "b", "c"] {
        fs::create_dir(dir.path().join(sub)).unwrap();
        fs::write(dir.path().join(sub).join("Lead Vocal.wav"), b"x").unwrap();
    }

    let first = add(dir.path(), Path::new("a/Lead Vocal.wav"), None, "stem").unwrap();
    let second = add(dir.path(), Path::new("b/Lead Vocal.wav"), None, "stem").unwrap();
    let third = add(dir.path(), Path::new("c/Lead Vocal.wav"), None, "stem").unwrap();

    // "Lead Vocal" lowercases and sanitizes to "lead-vocal"; collisions suffix.
    assert_eq!(first.id, "lead-vocal");
    assert_eq!(second.id, "lead-vocal-2");
    assert_eq!(third.id, "lead-vocal-3");
}

#[test]
fn add_refuses_an_explicit_id_already_in_use() {
    let dir = project();
    fs::write(dir.path().join("a.wav"), b"x").unwrap();
    fs::write(dir.path().join("b.wav"), b"y").unwrap();
    add(dir.path(), Path::new("a.wav"), Some("shared"), "mix").unwrap();

    let err = add(dir.path(), Path::new("b.wav"), Some("shared"), "mix").unwrap_err();
    assert!(matches!(err, AddError::IdInUse(_)), "got {err:?}");
}

#[test]
fn add_refuses_a_role_that_is_not_a_slug() {
    let dir = project();
    fs::write(dir.path().join("a.wav"), b"x").unwrap();

    let err = add(dir.path(), Path::new("a.wav"), None, "Not A Slug").unwrap_err();
    assert!(
        matches!(err, AddError::InvalidSlug { what: "role", .. }),
        "got {err:?}"
    );
}
