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

// --- M1.6: strict read path, schema URL check, unknown-field rejection, ext ---

#[test]
fn add_refuses_a_manifest_whose_schema_url_is_not_the_recognized_v0() {
    let dir = TempDir::new().unwrap();
    // Valid JSON, valid shape, but a schema URL this tool version does not own.
    // Rejected by exact string match — no version-range cleverness (uncompose#64).
    let bogus = "{\n  \"schema\": \"https://uncompose.org/schemas/project/v99/uncompose.project.schema.json\",\n  \"project\": { \"id\": \"01ARZ3\", \"name\": \"x\", \"created_at\": \"2020-01-01T00:00:00Z\" },\n  \"assets\": [],\n  \"derivations\": [],\n  \"evaluations\": []\n}\n";
    fs::write(dir.path().join(MANIFEST_FILENAME), bogus).unwrap();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();

    let output = run(dir.path(), &["add", "song.wav"]);

    assert!(
        !output.status.success(),
        "reading an unrecognized-schema manifest should refuse"
    );
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("schema") && stderr.contains(SCHEMA_URL),
        "error should name the schema mismatch and the expected URL: {stderr}"
    );
    // No partial parsing: the manifest is left byte-identical.
    assert_eq!(
        fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap(),
        bogus
    );
}

#[test]
fn add_refuses_a_manifest_with_an_unknown_field_naming_it() {
    let dir = TempDir::new().unwrap();
    // Correct schema URL, but a stray top-level field (a typo / from-the-future
    // key) that is not `ext`. Must be caught, not silently dropped.
    let with_unknown = format!(
        "{{\n  \"schema\": \"{SCHEMA_URL}\",\n  \"project\": {{ \"id\": \"01ARZ3\", \"name\": \"x\", \"created_at\": \"2020-01-01T00:00:00Z\" }},\n  \"assets\": [],\n  \"derivations\": [],\n  \"evaluations\": [],\n  \"totally_unknown\": true\n}}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILENAME), &with_unknown).unwrap();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();

    let output = run(dir.path(), &["add", "song.wav"]);

    assert!(!output.status.success(), "an unknown field should refuse");
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("totally_unknown"),
        "error should name the offending field: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap(),
        with_unknown
    );
}

// --- M1.4: show, human overview and --json ---

#[test]
fn show_prints_a_human_overview_of_an_empty_project() {
    let dir = TempDir::new().unwrap();
    assert!(run(dir.path(), &["init", "--name", "demo"])
        .status
        .success());

    let output = run(dir.path(), &["show"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("demo"), "should name the project: {stdout}");
    // Zero assets and derivations are shown explicitly, not silently omitted.
    assert!(stdout.contains("Assets (0)"), "{stdout}");
    assert!(stdout.contains("Derivations (0)"), "{stdout}");
}

#[test]
fn show_lists_assets_with_id_path_role_and_hash() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    assert!(run(dir.path(), &["add", "song.wav", "--role", "stem"])
        .status
        .success());

    let output = run(dir.path(), &["show"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Assets (1)"), "{stdout}");
    assert!(
        stdout.contains("song"),
        "should show the asset id: {stdout}"
    );
    assert!(
        stdout.contains("song.wav"),
        "should show the path: {stdout}"
    );
    assert!(stdout.contains("stem"), "should show the role: {stdout}");
    assert!(
        stdout.contains(HELLO_SHA256),
        "should show the sha256: {stdout}"
    );
}

/// `derivations[]` has no M1 command that creates one, but a hand-authored
/// manifest (valid per schema v0) carrying one must render in `show`.
#[test]
fn show_renders_a_hand_authored_derivation() {
    let dir = TempDir::new().unwrap();
    let seed = format!(
        "{{\n  \"schema\": \"{SCHEMA_URL}\",\n  \"project\": {{ \"id\": \"01ARZ3NDEKTSV4RRFFQ69G5FAV\", \"name\": \"demo\", \"created_at\": \"2020-01-01T00:00:00Z\" }},\n  \"assets\": [],\n  \"derivations\": [\n    {{\n      \"id\": \"split-1\",\n      \"inputs\": [\"source-mix\"],\n      \"outputs\": [\"lead-vox\", \"drums\"],\n      \"tool\": \"demucs\",\n      \"tool_version\": \"4.0\",\n      \"created_at\": \"2020-01-02T00:00:00Z\"\n    }}\n  ],\n  \"evaluations\": []\n}}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILENAME), &seed).unwrap();

    let output = run(dir.path(), &["show"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Derivations (1)"), "{stdout}");
    assert!(stdout.contains("split-1"), "derivation id: {stdout}");
    assert!(stdout.contains("demucs"), "tool: {stdout}");
    assert!(stdout.contains("4.0"), "tool_version: {stdout}");
    assert!(stdout.contains("source-mix"), "input slug: {stdout}");
    assert!(stdout.contains("lead-vox"), "output slug: {stdout}");
    assert!(stdout.contains("drums"), "output slug: {stdout}");
}

#[test]
fn show_json_is_byte_identical_to_the_manifest_file() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    assert!(run(dir.path(), &["add", "song.wav"]).status.success());

    let file_bytes = fs::read(dir.path().join(MANIFEST_FILENAME)).unwrap();
    let output = run(dir.path(), &["show", "--json"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout, file_bytes,
        "show --json must be byte-identical to the manifest file"
    );
}

/// `--json` emits the file verbatim, not a re-serialization: a deliberately
/// non-canonical (compact) manifest comes back byte-for-byte unchanged.
#[test]
fn show_json_emits_a_hand_authored_manifest_verbatim() {
    let dir = TempDir::new().unwrap();
    let seed = format!(
        "{{\"schema\":\"{SCHEMA_URL}\",\"project\":{{\"id\":\"01ARZ3\",\"name\":\"x\",\"created_at\":\"2020-01-01T00:00:00Z\"}},\"assets\":[],\"derivations\":[],\"evaluations\":[]}}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILENAME), &seed).unwrap();

    let output = run(dir.path(), &["show", "--json"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), seed);
}

#[test]
fn show_refuses_when_the_directory_is_not_a_project() {
    let dir = TempDir::new().unwrap();
    let output = run(dir.path(), &["show"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("not an uncompose project"),
        "stderr: {stderr}"
    );
}

/// `show` uses the same strict read as `add`: an unrecognized `schema` URL is
/// refused, never best-effort-rendered (uncompose#64).
#[test]
fn show_refuses_a_manifest_whose_schema_url_is_not_the_recognized_v0() {
    let dir = TempDir::new().unwrap();
    let bogus = "{\n  \"schema\": \"https://uncompose.org/schemas/project/v99/uncompose.project.schema.json\",\n  \"project\": { \"id\": \"01ARZ3\", \"name\": \"x\", \"created_at\": \"2020-01-01T00:00:00Z\" },\n  \"assets\": [],\n  \"derivations\": [],\n  \"evaluations\": []\n}\n";
    fs::write(dir.path().join(MANIFEST_FILENAME), bogus).unwrap();

    let output = run(dir.path(), &["show"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("schema") && stderr.contains(SCHEMA_URL),
        "stderr: {stderr}"
    );
}

/// Named conformance test (uncompose#64): `ext` blobs at project, asset, and
/// derivation level survive a rewriting command (`add`) verbatim.
#[test]
fn ext_subtrees_survive_read_modify_write_at_every_level() {
    let dir = TempDir::new().unwrap();
    // A schema-valid manifest carrying `ext` at three levels. Keys are namespace
    // slugs, deliberately in non-alphabetical order so a re-sort would show.
    let seed = format!(
        "{{\n  \"schema\": \"{SCHEMA_URL}\",\n  \"project\": {{\n    \"id\": \"01ARZ3NDEKTSV4RRFFQ69G5FAV\",\n    \"name\": \"demo\",\n    \"created_at\": \"2020-01-01T00:00:00Z\",\n    \"ext\": {{\n      \"zeta.notes\": {{ \"beta\": 2, \"alpha\": 1 }},\n      \"acme.tags\": [\"keep\", \"me\"]\n    }}\n  }},\n  \"assets\": [\n    {{\n      \"id\": \"existing\",\n      \"path\": \"existing.wav\",\n      \"sha256\": \"{HELLO_SHA256}\",\n      \"size\": 5,\n      \"role\": \"mix\",\n      \"added_at\": \"2020-01-01T00:00:00Z\",\n      \"ext\": {{ \"vendor.meta\": {{ \"take\": 3 }} }}\n    }}\n  ],\n  \"derivations\": [\n    {{\n      \"id\": \"mixdown\",\n      \"inputs\": [\"existing\"],\n      \"outputs\": [\"existing\"],\n      \"tool\": \"manual\",\n      \"created_at\": \"2020-01-01T00:00:00Z\",\n      \"ext\": {{ \"zzz.last\": {{ \"seen\": true }}, \"aaa.first\": {{ \"seen\": false }} }}\n    }}\n  ],\n  \"evaluations\": []\n}}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILENAME), &seed).unwrap();
    fs::write(dir.path().join("new.wav"), b"hello").unwrap();

    let output = run(dir.path(), &["add", "new.wav"]);
    assert!(
        output.status.success(),
        "add over an ext-bearing manifest should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    let manifest: Value = serde_json::from_str(&text).unwrap();

    // The rewrite took effect: the new asset is present alongside the seeded one.
    let assets = manifest["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 2);
    assert_eq!(assets[1]["id"], "new");

    // Every ext subtree survives with its data intact.
    assert_eq!(
        manifest["project"]["ext"],
        serde_json::json!({ "zeta.notes": { "beta": 2, "alpha": 1 }, "acme.tags": ["keep", "me"] })
    );
    assert_eq!(
        manifest["assets"][0]["ext"],
        serde_json::json!({ "vendor.meta": { "take": 3 } })
    );
    assert_eq!(
        manifest["derivations"][0]["ext"],
        serde_json::json!({ "zzz.last": { "seen": true }, "aaa.first": { "seen": false } })
    );

    // Verbatim, not merely intact: the original key order is preserved, so a
    // rewrite never normalizes a third party's opaque blob.
    let zeta = text.find("zeta.notes").unwrap();
    let acme = text.find("acme.tags").unwrap();
    assert!(
        zeta < acme,
        "project ext key order should be preserved: {text}"
    );
    let zzz = text.find("zzz.last").unwrap();
    let aaa = text.find("aaa.first").unwrap();
    assert!(
        zzz < aaa,
        "derivation ext key order should be preserved: {text}"
    );

    assert_valid_against_schema(&manifest);
}

// --- M1.5: verify with integrity statuses and the milestone DoD ---

#[test]
fn verify_reports_all_verified_updates_last_verified_and_exits_zero() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    assert!(run(dir.path(), &["add", "song.wav"]).status.success());

    // Nothing on disk changed: verify should pass, exit zero, and stamp
    // last_verified on the asset that passed.
    let output = run(dir.path(), &["verify"]);
    assert!(
        output.status.success(),
        "verify should exit zero when all assets pass: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("verified") && stdout.contains("song.wav"),
        "verify should report the asset as verified: {stdout}"
    );

    let manifest = read_manifest(dir.path());
    let asset = &manifest["assets"][0];
    let last_verified = asset["last_verified"].as_str();
    assert!(
        last_verified.is_some_and(|s| s.contains('T')),
        "a passing asset should get an RFC3339 last_verified: {asset}"
    );
    // Integrity is derived, never a stored status claim.
    assert!(
        asset.get("status").is_none(),
        "no status field is persisted"
    );
    assert_valid_against_schema(&manifest);
}

/// Milestone DoD (modified): change a registered file on disk, then `verify`
/// must warn — naming the path and that the contents changed — and exit non-zero.
#[test]
fn verify_flags_a_modified_file_with_a_warning_and_nonzero_exit() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    assert!(run(dir.path(), &["add", "song.wav"]).status.success());

    // Same size, different bytes — the hash catches what the size check misses.
    fs::write(dir.path().join("song.wav"), b"world").unwrap();

    let output = run(dir.path(), &["verify"]);
    assert!(
        !output.status.success(),
        "verify should exit non-zero when an asset is modified"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("song.wav") && combined.contains("modified"),
        "verify should name the path and how it failed: {combined}"
    );

    // A modified asset does not get a fresh last_verified.
    let manifest = read_manifest(dir.path());
    assert!(
        manifest["assets"][0]["last_verified"].is_null(),
        "a failing asset must not be stamped last_verified"
    );
}

/// Milestone DoD (missing): delete a registered file, then `verify` must report
/// it missing and exit non-zero.
#[test]
fn verify_flags_a_missing_file_with_a_warning_and_nonzero_exit() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    assert!(run(dir.path(), &["add", "song.wav"]).status.success());

    fs::remove_file(dir.path().join("song.wav")).unwrap();

    let output = run(dir.path(), &["verify"]);
    assert!(
        !output.status.success(),
        "verify should exit non-zero when an asset is missing"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("song.wav") && combined.contains("missing"),
        "verify should name the path and report it missing: {combined}"
    );
}

#[test]
fn verify_checks_size_before_hash_flagging_a_truncated_file_modified() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    assert!(run(dir.path(), &["add", "song.wav"]).status.success());

    // A different size is a cheap mismatch caught before hashing.
    fs::write(dir.path().join("song.wav"), b"hi").unwrap();

    let output = run(dir.path(), &["verify"]);
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("song.wav") && combined.contains("modified"),
        "a size mismatch should read as modified: {combined}"
    );
}

#[test]
fn verify_updates_passing_assets_even_when_another_asset_fails() {
    let dir = init_project();
    fs::write(dir.path().join("good.wav"), b"hello").unwrap();
    fs::write(dir.path().join("bad.wav"), b"world").unwrap();
    assert!(run(dir.path(), &["add", "good.wav"]).status.success());
    assert!(run(dir.path(), &["add", "bad.wav"]).status.success());

    fs::remove_file(dir.path().join("bad.wav")).unwrap();

    let output = run(dir.path(), &["verify"]);
    assert!(!output.status.success(), "a missing asset fails the run");

    let manifest = read_manifest(dir.path());
    let assets = manifest["assets"].as_array().unwrap();
    let good = assets.iter().find(|a| a["id"] == "good").unwrap();
    let bad = assets.iter().find(|a| a["id"] == "bad").unwrap();
    assert!(
        good["last_verified"]
            .as_str()
            .is_some_and(|s| s.contains('T')),
        "the passing asset should be stamped even though another failed: {good}"
    );
    assert!(
        bad["last_verified"].is_null(),
        "the missing asset must not be stamped: {bad}"
    );
    assert_valid_against_schema(&manifest);
}

#[test]
fn verify_refuses_when_the_directory_is_not_a_project() {
    let dir = TempDir::new().unwrap();
    let output = run(dir.path(), &["verify"]);
    assert!(!output.status.success());
    assert!(!dir.path().join(MANIFEST_FILENAME).exists());
}
