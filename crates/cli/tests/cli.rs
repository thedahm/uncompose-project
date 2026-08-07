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
fn version_flag_prints_name_and_version() {
    // ADR-0005 dispatch contract: the delegated binary must answer `--version`.
    let dir = TempDir::new().unwrap();
    let output = run(dir.path(), &["--version"]);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("uncompose-project {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn help_flag_prints_usage_and_commands() {
    // ADR-0005 dispatch contract: the delegated binary must answer `--help`.
    let dir = TempDir::new().unwrap();
    let output = run(dir.path(), &["--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Usage:"),
        "help should show usage: {stdout}"
    );
    assert!(
        stdout.contains("init"),
        "help should list the init command: {stdout}"
    );
    assert!(output.stderr.is_empty());
}

/// ADR-0005 root dispatch: `uncompose <sub> <args>` execs `uncompose-<sub> <args>`
/// found on PATH. We stand up a minimal dispatcher matching that contract and
/// confirm the binary answers through it with identical output and preserved
/// exit codes. v0.1 targets Linux, so a POSIX-shell shim is sufficient.
#[cfg(unix)]
#[test]
fn root_dispatch_delegates_preserving_args_and_exit_codes() {
    use std::os::unix::fs::PermissionsExt;

    let bin_dir = Path::new(BIN).parent().unwrap();
    let shim_dir = TempDir::new().unwrap();
    let shim = shim_dir.path().join("uncompose");
    fs::write(
        &shim,
        r#"#!/bin/sh
sub="$1"; shift
exec "uncompose-$sub" "$@"
"#,
    )
    .unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();

    let path = format!(
        "{}:{}:{}",
        shim_dir.path().display(),
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let dispatch = |dir: &Path, args: &[&str]| -> Output {
        Command::new(&shim)
            .args(args)
            .env("PATH", &path)
            .current_dir(dir)
            .output()
            .expect("failed to run the dispatch shim")
    };

    let project = TempDir::new().unwrap();

    // `--version` through dispatch is byte-identical to a direct invocation.
    let delegated = dispatch(project.path(), &["project", "--version"]);
    assert!(delegated.status.success());
    assert_eq!(delegated.stdout, run(project.path(), &["--version"]).stdout);

    // A delegated command succeeds and takes effect (manifest written).
    let init = dispatch(project.path(), &["project", "init"]);
    assert!(
        init.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(project.path().join(MANIFEST_FILENAME).exists());

    // A refusal's non-zero exit code is preserved through dispatch.
    let reinit = dispatch(project.path(), &["project", "init"]);
    assert!(
        !reinit.status.success(),
        "re-init should refuse and exit non-zero through dispatch"
    );
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
