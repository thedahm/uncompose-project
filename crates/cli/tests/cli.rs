//! Integration tests at the CLI process boundary: run the compiled
//! `uncompose-project` binary in a real temp dir and assert on exit code,
//! stdout/stderr, and the bytes of `uncompose.project.json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;
use uncompose_project_core::{tagline, MANIFEST_FILENAME, SCHEMA_URL};

const BIN: &str = env!("CARGO_BIN_EXE_uncompose-project");

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run the uncompose-project binary")
}

fn schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/project/v0/uncompose.project.schema.json")
}

fn assert_valid_against_schema(manifest: &Value) {
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(schema_path()).expect("read schema file"))
            .expect("schema is valid JSON");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    if let Err(error) = validator.validate(manifest) {
        panic!("emitted manifest does not conform to schema v0: {error}");
    }
}

#[test]
fn prints_the_banner_and_exits_zero() {
    let dir = TempDir::new().unwrap();
    let output = run(dir.path(), &[]);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("uncompose-project — {}\n", tagline())
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn init_creates_a_canonical_manifest_named_after_the_dir() {
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("my-project");
    fs::create_dir(&root).unwrap();

    let output = run(&root, &["init"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let bytes = fs::read_to_string(root.join(MANIFEST_FILENAME)).unwrap();
    let manifest: Value = serde_json::from_str(&bytes).unwrap();

    assert_eq!(manifest["schema"], SCHEMA_URL);
    assert_eq!(manifest["project"]["name"], "my-project");
    assert_eq!(manifest["assets"], serde_json::json!([]));
    assert_eq!(manifest["derivations"], serde_json::json!([]));
    assert_eq!(manifest["evaluations"], serde_json::json!([]));

    // ULID: 26 Crockford base32 chars.
    let id = manifest["project"]["id"].as_str().unwrap();
    assert_eq!(id.len(), 26, "project id should be a ULID: {id}");

    assert_valid_against_schema(&manifest);

    // Canonical bytes: exact field order, 2-space indent, trailing newline.
    let created_at = manifest["project"]["created_at"].as_str().unwrap();
    let expected = format!(
        "{{\n  \"schema\": \"{SCHEMA_URL}\",\n  \"project\": {{\n    \"id\": \"{id}\",\n    \"name\": \"my-project\",\n    \"created_at\": \"{created_at}\"\n  }},\n  \"assets\": [],\n  \"derivations\": [],\n  \"evaluations\": []\n}}\n"
    );
    assert_eq!(bytes, expected);
}

#[test]
fn init_name_flag_overrides_the_dir_name() {
    let dir = TempDir::new().unwrap();
    let output = run(dir.path(), &["init", "--name", "custom-name"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bytes = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    let manifest: Value = serde_json::from_str(&bytes).unwrap();
    assert_eq!(manifest["project"]["name"], "custom-name");
    assert_valid_against_schema(&manifest);
}

#[test]
fn init_refuses_when_a_manifest_already_exists() {
    let dir = TempDir::new().unwrap();
    let manifest = dir.path().join(MANIFEST_FILENAME);
    let sentinel = "{ \"pre-existing\": true }\n";
    fs::write(&manifest, sentinel).unwrap();

    let output = run(dir.path(), &["init"]);

    assert!(
        !output.status.success(),
        "init should refuse and exit non-zero"
    );
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("already exists"), "stderr: {stderr}");

    // The existing manifest is untouched.
    assert_eq!(fs::read_to_string(&manifest).unwrap(), sentinel);
}

/// sha256 of the bytes `b"hello"`; size 5.
const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

/// Initialize a project in a fresh temp dir and return it.
fn init_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    let out = run(dir.path(), &["init"]);
    assert!(out.status.success());
    dir
}

fn read_manifest(root: &Path) -> Value {
    let bytes = fs::read_to_string(root.join(MANIFEST_FILENAME)).unwrap();
    serde_json::from_str(&bytes).unwrap()
}

#[test]
fn add_registers_an_asset_with_hash_size_path_role_and_timestamp() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();

    let output = run(dir.path(), &["add", "song.wav"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let manifest = read_manifest(dir.path());
    let assets = manifest["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 1);
    let asset = &assets[0];
    assert_eq!(asset["id"], "song");
    assert_eq!(asset["path"], "song.wav");
    assert_eq!(asset["sha256"], HELLO_SHA256);
    assert_eq!(asset["size"], 5);
    assert_eq!(asset["role"], "mix");
    assert!(asset["added_at"].as_str().unwrap().contains('T'));

    assert_valid_against_schema(&manifest);
}

#[test]
fn add_role_flag_sets_the_role() {
    let dir = init_project();
    fs::write(dir.path().join("bass.wav"), b"hello").unwrap();

    let output = run(dir.path(), &["add", "bass.wav", "--role", "stem"]);
    assert!(output.status.success());

    let manifest = read_manifest(dir.path());
    assert_eq!(manifest["assets"][0]["role"], "stem");
    assert_valid_against_schema(&manifest);
}

#[test]
fn add_id_flag_overrides_the_minted_slug() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();

    let output = run(dir.path(), &["add", "song.wav", "--id", "lead-vox"]);
    assert!(output.status.success());

    let manifest = read_manifest(dir.path());
    assert_eq!(manifest["assets"][0]["id"], "lead-vox");
    assert_valid_against_schema(&manifest);
}

#[test]
fn add_disambiguates_colliding_slugs_with_a_numeric_suffix() {
    let dir = init_project();
    fs::create_dir(dir.path().join("take1")).unwrap();
    fs::create_dir(dir.path().join("take2")).unwrap();
    fs::write(dir.path().join("take1/vocals.wav"), b"hello").unwrap();
    fs::write(dir.path().join("take2/vocals.wav"), b"world").unwrap();

    assert!(run(dir.path(), &["add", "take1/vocals.wav"])
        .status
        .success());
    assert!(run(dir.path(), &["add", "take2/vocals.wav"])
        .status
        .success());

    let manifest = read_manifest(dir.path());
    let assets = manifest["assets"].as_array().unwrap();
    let ids: Vec<&str> = assets.iter().map(|a| a["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["vocals", "vocals-2"]);
    let paths: Vec<&str> = assets.iter().map(|a| a["path"].as_str().unwrap()).collect();
    assert_eq!(paths, vec!["take1/vocals.wav", "take2/vocals.wav"]);
    assert_valid_against_schema(&manifest);
}

#[test]
fn add_refuses_an_already_registered_path_naming_the_existing_asset() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    assert!(run(dir.path(), &["add", "song.wav", "--id", "first"])
        .status
        .success());

    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    let output = run(dir.path(), &["add", "song.wav"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("already registered"), "stderr: {stderr}");
    assert!(
        stderr.contains("first"),
        "stderr should name the asset: {stderr}"
    );

    // Manifest byte-identical to before the refused add.
    let after = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert_eq!(before, after);
}

#[test]
fn add_refuses_a_missing_file_leaving_the_manifest_untouched() {
    let dir = init_project();
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();

    let output = run(dir.path(), &["add", "nope.wav"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let after = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert_eq!(before, after);
}

#[test]
fn add_refuses_an_invalid_id() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();

    let output = run(dir.path(), &["add", "song.wav", "--id", "Bad Id"]);
    assert!(!output.status.success());
    let after = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert_eq!(before, after);
}

#[test]
fn add_refuses_when_the_directory_is_not_a_project() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();

    let output = run(dir.path(), &["add", "song.wav"]);
    assert!(!output.status.success());
    assert!(!dir.path().join(MANIFEST_FILENAME).exists());
}
