//! Integration tests at the CLI process boundary: run the compiled
//! `uncompose-project` binary in a real temp dir and assert on exit code,
//! stdout/stderr, and the bytes of `uncompose.project.json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;
use uncompose_project_core::{
    tagline, COMPARE_SCHEMA_URL, LOCK_FILENAME, LOCK_WAIT_NOTICE, MANIFEST_FILENAME, SCHEMA_URL,
};

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
fn bare_invocation_prints_usage_and_exits_nonzero() {
    // No subcommand is not a success: the spec defines the subcommands plus
    // --version/--help, and a script that ran nothing should not see exit 0.
    let dir = TempDir::new().unwrap();
    let output = run(dir.path(), &[]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Usage:"),
        "bare invocation should show usage: {stderr}"
    );
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
    assert!(
        stdout.contains(tagline()),
        "help should carry the tool's one-line description: {stdout}"
    );
    assert!(output.stderr.is_empty());
}

/// Stand up the minimal ADR-0005 dispatcher: an `uncompose` shell shim that execs
/// `uncompose-<sub> <args>` found on PATH. Returns the shim's temp dir (kept alive
/// by the caller), the shim path, and a PATH value resolving both the shim and
/// this crate's compiled binary. v0.1 targets Linux, so a POSIX-shell shim is
/// sufficient.
#[cfg(unix)]
fn install_dispatch_shim() -> (TempDir, PathBuf, String) {
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
    (shim_dir, shim, path)
}

/// ADR-0005 root dispatch: the binary answers through the dispatcher with
/// identical output and preserved exit codes.
#[cfg(unix)]
#[test]
fn root_dispatch_delegates_preserving_args_and_exit_codes() {
    let (_shim_dir, shim, path) = install_dispatch_shim();
    let dispatch = |dir: &Path, args: &[&str]| -> Output {
        // Invoke the shim through `sh <shim>` rather than exec'ing it directly.
        // A direct exec of a file just written by this test races other tests'
        // fork()s, which transiently inherit the still-open writable fd and make
        // the kernel refuse the exec with ETXTBSY ("Text file busy"). Running it
        // via the shell (which only opens the shim for reading, matching its
        // `#!/bin/sh` shebang) is behaviorally identical and race-free.
        Command::new("/bin/sh")
            .arg(&shim)
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

/// Strict reads reach inside `derivations[]` too: an unknown field on a
/// derivation is rejected, never carried and re-emitted (uncompose#64,
/// `additionalProperties: false` outside `ext`).
#[test]
fn show_refuses_a_derivation_with_an_unknown_field() {
    let dir = TempDir::new().unwrap();
    let bogus = format!(
        "{{\n  \"schema\": \"{SCHEMA_URL}\",\n  \"project\": {{ \"id\": \"01ARZ3\", \"name\": \"x\", \"created_at\": \"2020-01-01T00:00:00Z\" }},\n  \"assets\": [],\n  \"derivations\": [\n    {{\n      \"id\": \"split-1\",\n      \"inputs\": [\"a\"],\n      \"outputs\": [\"b\"],\n      \"tool\": \"demucs\",\n      \"created_at\": \"2020-01-02T00:00:00Z\",\n      \"totally_unknown\": true\n    }}\n  ],\n  \"evaluations\": []\n}}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILENAME), &bogus).unwrap();

    let output = run(dir.path(), &["show"]);

    assert!(
        !output.status.success(),
        "an off-shape derivation should refuse"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("totally_unknown"),
        "error should name the offending field: {stderr}"
    );
}

/// A derivation missing a required field (here `tool`) is off-shape, not
/// best-effort-rendered.
#[test]
fn show_refuses_a_derivation_missing_a_required_field() {
    let dir = TempDir::new().unwrap();
    let bogus = format!(
        "{{\n  \"schema\": \"{SCHEMA_URL}\",\n  \"project\": {{ \"id\": \"01ARZ3\", \"name\": \"x\", \"created_at\": \"2020-01-01T00:00:00Z\" }},\n  \"assets\": [],\n  \"derivations\": [\n    {{\n      \"id\": \"split-1\",\n      \"inputs\": [\"a\"],\n      \"outputs\": [\"b\"],\n      \"created_at\": \"2020-01-02T00:00:00Z\"\n    }}\n  ],\n  \"evaluations\": []\n}}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILENAME), &bogus).unwrap();

    let output = run(dir.path(), &["show"]);

    assert!(
        !output.status.success(),
        "a derivation without `tool` should refuse"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("tool"),
        "error should name the missing field: {stderr}"
    );
}

/// `ext` keys are namespace slugs (uncompose#64). A key outside the slug
/// pattern is rejected so the tool never re-emits a manifest the published
/// schema fails.
#[test]
fn show_refuses_an_ext_key_that_is_not_a_namespace_slug() {
    let dir = TempDir::new().unwrap();
    let bogus = format!(
        "{{\n  \"schema\": \"{SCHEMA_URL}\",\n  \"project\": {{ \"id\": \"01ARZ3\", \"name\": \"x\", \"created_at\": \"2020-01-01T00:00:00Z\", \"ext\": {{ \"Bad Key\": 1 }} }},\n  \"assets\": [],\n  \"derivations\": [],\n  \"evaluations\": []\n}}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILENAME), &bogus).unwrap();

    let output = run(dir.path(), &["show"]);

    assert!(!output.status.success(), "a non-slug ext key should refuse");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Bad Key"),
        "error should name the offending key: {stderr}"
    );
}

/// `ext` must be an object; a scalar there is off-shape, not opaque data.
#[test]
fn show_refuses_an_ext_that_is_not_an_object() {
    let dir = TempDir::new().unwrap();
    let bogus = format!(
        "{{\n  \"schema\": \"{SCHEMA_URL}\",\n  \"project\": {{ \"id\": \"01ARZ3\", \"name\": \"x\", \"created_at\": \"2020-01-01T00:00:00Z\" }},\n  \"assets\": [],\n  \"derivations\": [],\n  \"evaluations\": [],\n  \"ext\": 5\n}}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILENAME), &bogus).unwrap();

    let output = run(dir.path(), &["show"]);

    assert!(!output.status.success(), "a scalar ext should refuse");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("does not conform to schema v0"),
        "error should say the manifest is off-shape: {stderr}"
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

// --- M2 slice 1: happy-path import — job.json to input, stems, derivation ---

/// A fixed completion time and its whole-second RFC3339 UTC rendering, so the
/// happy-path test can assert `created_at` exactly.
const FINISHED_AT_UNIX: u64 = 1_577_923_200;
const FINISHED_AT_RFC3339: &str = "2020-01-02T00:00:00Z";

/// Synthesize a completed uncompose job under `root`: an in-tree input at
/// `input_rel` and a job folder `<folder>/` holding `<stem>.wav` for each stem
/// plus a `job.json`. Tiny generated bytes stand in for audio — the import
/// contract never inspects audio content. Returns the job.json path relative to
/// the root. `input_sha256` in the record is the true sha256 of the input bytes
/// unless overridden, so callers can force a mismatch. The record carries a
/// `models` field the importer must tolerate as an unknown extra.
fn synth_job(
    root: &Path,
    input_rel: &str,
    input_bytes: &[u8],
    input_sha256: &str,
    folder: &str,
    stems: &[&str],
    outcome: &str,
) -> String {
    fs::write(root.join(input_rel), input_bytes).unwrap();
    let job_dir = root.join(folder);
    fs::create_dir_all(&job_dir).unwrap();
    for stem in stems {
        fs::write(job_dir.join(format!("{stem}.wav")), format!("{stem}-audio")).unwrap();
    }
    let job = serde_json::json!({
        "input_path": input_rel,
        "input_sha256": input_sha256,
        "preset": "studio",
        "stems": stems,
        "engine_version": "1.2.3",
        "outcome": outcome,
        "finished_at_unix": FINISHED_AT_UNIX,
        "models": { "note": "tolerated unknown field" },
    });
    fs::write(job_dir.join("job.json"), job.to_string()).unwrap();
    format!("{folder}/job.json")
}

#[test]
fn import_lands_input_stems_and_a_derivation_in_one_step() {
    let dir = init_project();
    // finished_at_unix 1577923200 == 2020-01-02T00:00:00Z.
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals", "drums"],
        "success",
    );

    let output = run(dir.path(), &["import", &job]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The summary names the input, each stem, and the derivation id.
    assert!(
        stdout.contains("mix"),
        "summary should name the input: {stdout}"
    );
    assert!(
        stdout.contains("vocals") && stdout.contains("drums"),
        "{stdout}"
    );
    // …and says what happened to each: nothing was registered before this run.
    assert!(
        stdout.contains("2 stems: 2 registered"),
        "summary should tally the stems: {stdout}"
    );
    assert_eq!(
        stdout.matches("[registered]").count(),
        3,
        "input and both stems are newly registered: {stdout}"
    );

    let manifest = read_manifest(dir.path());

    // One `mix` input plus two `stem` assets.
    let assets = manifest["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 3, "input + 2 stems: {assets:?}");
    let input = &assets[0];
    assert_eq!(input["id"], "mix");
    assert_eq!(input["path"], "mix.wav");
    assert_eq!(input["sha256"], HELLO_SHA256);
    assert_eq!(input["role"], "mix");
    let roles: Vec<&str> = assets[1..]
        .iter()
        .map(|a| a["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["stem", "stem"]);
    let stem_paths: Vec<&str> = assets[1..]
        .iter()
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    assert_eq!(stem_paths, vec!["run1/vocals.wav", "run1/drums.wav"]);

    // One derivation ties the input to the stems.
    let derivations = manifest["derivations"].as_array().unwrap();
    assert_eq!(derivations.len(), 1);
    let d = &derivations[0];
    assert_eq!(d["id"], "run1");
    assert_eq!(d["tool"], "uncompose");
    assert_eq!(d["tool_version"], "1.2.3");
    assert_eq!(d["created_at"], FINISHED_AT_RFC3339);
    assert_eq!(d["inputs"], serde_json::json!(["mix"]));
    assert_eq!(d["outputs"], serde_json::json!(["vocals", "drums"]));
    // `params` carries only the preset; everything else stays behind the job ref.
    assert_eq!(d["params"], serde_json::json!({ "preset": "studio" }));
    assert_eq!(d["job"]["path"], "run1/job.json");
    let job_sha = d["job"]["sha256"].as_str().unwrap();
    assert_eq!(
        job_sha.len(),
        64,
        "job ref sha256 is a hex digest: {job_sha}"
    );
    assert!(job_sha.chars().all(|c| c.is_ascii_hexdigit()));

    // job.json is referenced, never registered as an asset.
    assert!(
        assets
            .iter()
            .all(|a| a["path"].as_str().unwrap() != "run1/job.json"),
        "job.json must not be a registered asset"
    );

    assert_valid_against_schema(&manifest);
}

#[test]
fn import_leaves_the_project_verifiable_and_shown() {
    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    assert!(run(dir.path(), &["import", &job]).status.success());

    // verify is green: every imported asset matches disk.
    let verify = run(dir.path(), &["verify"]);
    assert!(
        verify.status.success(),
        "verify should be green after import: {}",
        String::from_utf8_lossy(&verify.stderr)
    );

    // show lists the new assets and the derivation.
    let show = run(dir.path(), &["show"]);
    assert!(show.status.success());
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("Assets (2)"), "{stdout}");
    assert!(stdout.contains("mix.wav"), "{stdout}");
    assert!(stdout.contains("run1/vocals.wav"), "{stdout}");
    assert!(stdout.contains("Derivations (1)"), "{stdout}");
    assert!(stdout.contains("uncompose"), "{stdout}");
}

// --- M2 slice 4: show renders the imported graph ---

/// `show`'s overview of an imported derivation grows the preset and the hashed
/// job reference (path + sha256), alongside the existing tool/version, inputs,
/// outputs, and created line.
#[test]
fn show_renders_the_preset_and_job_ref_of_an_imported_derivation() {
    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    assert!(run(dir.path(), &["import", &job]).status.success());

    // The job ref's sha256 is whatever the importer hashed job.json to.
    let manifest = read_manifest(dir.path());
    let job_sha = manifest["derivations"][0]["job"]["sha256"]
        .as_str()
        .unwrap()
        .to_string();

    let show = run(dir.path(), &["show"]);
    assert!(show.status.success());
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(
        stdout.contains("preset") && stdout.contains("studio"),
        "overview should show the preset: {stdout}"
    );
    assert!(
        stdout.contains("run1/job.json"),
        "overview should show the job ref path: {stdout}"
    );
    assert!(
        stdout.contains(&job_sha),
        "overview should show the job ref sha256: {stdout}"
    );
}

#[test]
fn import_refuses_a_non_success_outcome_leaving_the_manifest_untouched() {
    let dir = init_project();
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "failed",
    );

    let output = run(dir.path(), &["import", &job]);
    assert!(!output.status.success(), "a failed job should refuse");
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("failed") && stderr.contains("outcome"),
        "error should show the outcome: {stderr}"
    );

    let after = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert_eq!(
        before, after,
        "a refused import leaves the manifest untouched"
    );
}

#[test]
fn import_refuses_an_input_hash_mismatch_naming_both_hashes() {
    let dir = init_project();
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    // The input on disk is b"world" but the job records the hash of b"hello".
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"world",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );

    let output = run(dir.path(), &["import", &job]);
    assert!(!output.status.success(), "a hash mismatch should refuse");
    let stderr = String::from_utf8(output.stderr).unwrap();
    // Both the recorded and the current hash are named.
    let world_sha = "486ea46224d1bb4fb680f34f7c9ad96a8f24ec88be73ea8e5a6c65260e9cb8a7";
    assert!(
        stderr.contains(HELLO_SHA256) && stderr.contains(world_sha),
        "error should name both hashes: {stderr}"
    );

    let after = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert_eq!(before, after);
}

/// Acceptance: import behaves identically through the ADR-0005 root dispatch
/// (`uncompose project import <job>`), the same shim the init dispatch test uses.
#[cfg(unix)]
#[test]
fn import_works_through_root_dispatch() {
    let (_shim_dir, shim, path) = install_dispatch_shim();

    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );

    let output = Command::new(&shim)
        .args(["project", "import", &job])
        .env("PATH", &path)
        .current_dir(dir.path())
        .output()
        .expect("failed to run the dispatch shim");
    assert!(
        output.status.success(),
        "import via dispatch should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = read_manifest(dir.path());
    assert_eq!(manifest["derivations"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["assets"].as_array().unwrap().len(), 2);
}

// --- M2 slice 2: input resolution by hash, out-of-tree and bad-record refusals ---

/// Acceptance: a job input whose `input_sha256` matches an already-registered
/// asset reuses that asset (hash wins regardless of the job's `input_path`) — no
/// duplicate registration, and the derivation links to the existing id.
#[test]
fn import_reuses_a_registered_asset_matching_the_input_hash() {
    let dir = init_project();
    // Register `original.wav` (b"hello") up front; its id mints from the stem.
    fs::write(dir.path().join("original.wav"), b"hello").unwrap();
    assert!(run(dir.path(), &["add", "original.wav"]).status.success());

    // A job whose input lives at a different path (`mix.wav`) but carries the
    // same bytes, so its `input_sha256` matches the registered asset.
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    let output = run(dir.path(), &["import", &job]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The summary distinguishes a resolved input from a registered one.
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(
        stdout.contains("input:      original (original.wav) [resolved to an existing asset]"),
        "summary should say the input resolved, not registered: {stdout}"
    );

    let manifest = read_manifest(dir.path());
    let assets = manifest["assets"].as_array().unwrap();
    // Only the pre-registered input and the one stem — `mix.wav` is not added.
    let paths: Vec<&str> = assets.iter().map(|a| a["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        vec!["original.wav", "run1/vocals.wav"],
        "no dup input"
    );

    // The derivation links to the existing asset's id, not a fresh one.
    let d = &manifest["derivations"].as_array().unwrap()[0];
    assert_eq!(d["inputs"], serde_json::json!(["original"]));
    assert_valid_against_schema(&manifest);
}

/// Acceptance: `import` is the cross-tool handoff target, so an absolute job path
/// that lands inside the root is accepted (the pinned argv passes absolute paths).
#[test]
fn import_accepts_an_absolute_job_path_inside_the_root() {
    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    let abs = dir.path().join(&job);

    let output = run(dir.path(), &["import", abs.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "an absolute in-root job path should import: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = read_manifest(dir.path());
    assert_eq!(manifest["derivations"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["assets"].as_array().unwrap().len(), 2);
}

/// Acceptance: a job folder outside the project root refuses; the manifest is
/// left byte-identical.
#[test]
fn import_refuses_a_job_folder_outside_the_project_root() {
    let dir = init_project();
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();

    // A job folder in the parent of the project root, reached via `../`.
    let outside = dir.path().parent().unwrap().join("uncompose-outside-run");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("job.json"), b"{}").unwrap();

    let output = run(dir.path(), &["import", "../uncompose-outside-run/job.json"]);
    let _ = fs::remove_dir_all(&outside);
    assert!(!output.status.success(), "an out-of-root job should refuse");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("outside the project root"),
        "error should name the confinement: {stderr}"
    );

    let after = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert_eq!(
        before, after,
        "a refused import leaves the manifest untouched"
    );
}

// --- M2 slice 3: idempotent re-import and per-path asset dedupe ---

/// Acceptance: re-importing the same `job.json` (matching sha256) is a stated
/// no-op — exit 0, a message that names the existing derivation, and the manifest
/// left byte-identical.
#[test]
fn a_second_import_of_the_same_job_is_a_stated_noop() {
    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    assert!(run(dir.path(), &["import", &job]).status.success());
    let after_first = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();

    let output = run(dir.path(), &["import", &job]);
    assert!(
        output.status.success(),
        "a re-import must exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("run1") && stdout.to_lowercase().contains("already"),
        "the no-op should name the existing derivation: {stdout}"
    );

    let after_second = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert_eq!(
        after_first, after_second,
        "a no-op import leaves the manifest byte-identical"
    );
}

/// Acceptance: the same job path with different content (a distinct sha256) is a
/// genuinely different run and imports as a second derivation.
#[test]
fn same_job_path_with_different_content_imports_a_new_derivation() {
    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    assert!(run(dir.path(), &["import", &job]).status.success());

    // Rewrite the job.json in place with a different preset — same path, new bytes.
    let changed = serde_json::json!({
        "input_path": "mix.wav",
        "input_sha256": HELLO_SHA256,
        "preset": "live",
        "stems": ["vocals"],
        "engine_version": "1.2.3",
        "outcome": "success",
        "finished_at_unix": FINISHED_AT_UNIX,
    });
    fs::write(dir.path().join(&job), changed.to_string()).unwrap();

    let output = run(dir.path(), &["import", &job]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = read_manifest(dir.path());
    let derivations = manifest["derivations"].as_array().unwrap();
    assert_eq!(
        derivations.len(),
        2,
        "a differently-hashed job at the same path is a new derivation"
    );
    let presets: Vec<&Value> = derivations.iter().map(|d| &d["params"]["preset"]).collect();
    assert!(
        presets.contains(&&Value::from("studio")) && presets.contains(&&Value::from("live")),
        "both runs recorded: {presets:?}"
    );
    assert_valid_against_schema(&manifest);
}

/// Acceptance: a stem path already registered (here via `add`) with a matching
/// hash is reused, not duplicated; the derivation links the existing id.
#[test]
fn import_reuses_a_same_path_same_hash_stem_asset() {
    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    // Register the stem file up front, so import overlaps an existing asset.
    assert!(run(dir.path(), &["add", "run1/vocals.wav"])
        .status
        .success());
    let before_assets = read_manifest(dir.path())["assets"]
        .as_array()
        .unwrap()
        .len();

    let output = run(dir.path(), &["import", &job]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The summary marks the overlapping stem reused rather than registered.
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(
        stdout.contains("1 stem: 1 reused") && stdout.contains("[reused]"),
        "summary should say the stem was reused: {stdout}"
    );

    let manifest = read_manifest(dir.path());
    let assets = manifest["assets"].as_array().unwrap();
    let vocals: Vec<&Value> = assets
        .iter()
        .filter(|a| a["path"] == "run1/vocals.wav")
        .collect();
    assert_eq!(
        vocals.len(),
        1,
        "the stem path is not duplicated: {assets:?}"
    );
    // Import added only the input `mix`, not a second stem row.
    assert_eq!(assets.len(), before_assets + 1);

    let d = &manifest["derivations"].as_array().unwrap()[0];
    let reused_id = vocals[0]["id"].as_str().unwrap();
    assert_eq!(
        d["outputs"],
        serde_json::json!([reused_id]),
        "the derivation links the reused asset id"
    );
    assert_valid_against_schema(&manifest);
}

/// Acceptance: a registered stem path whose bytes no longer match refuses the
/// import naming the conflict, and leaves the manifest byte-identical.
#[test]
fn import_refuses_a_stem_path_whose_registered_hash_conflicts() {
    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    // Register the stem, then change the file on disk so the recorded hash and the
    // bytes the import would hash now disagree.
    assert!(run(dir.path(), &["add", "run1/vocals.wav"])
        .status
        .success());
    fs::write(dir.path().join("run1/vocals.wav"), b"tampered").unwrap();
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();

    let output = run(dir.path(), &["import", &job]);
    assert!(!output.status.success(), "a path/hash conflict must refuse");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("run1/vocals.wav"),
        "the error should name the conflicting path: {stderr}"
    );

    let after = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert_eq!(
        before, after,
        "a refused import leaves the manifest untouched"
    );
}

/// Acceptance: identical bytes at two different paths stay two distinct assets —
/// per-path dedupe never collapses by hash alone.
#[test]
fn identical_bytes_at_two_paths_remain_two_assets() {
    let dir = init_project();
    // Both stems carry identical bytes but live at distinct paths.
    fs::write(dir.path().join("mix.wav"), b"hello").unwrap();
    let job_dir = dir.path().join("run1");
    fs::create_dir_all(&job_dir).unwrap();
    fs::write(job_dir.join("left.wav"), b"same-bytes").unwrap();
    fs::write(job_dir.join("right.wav"), b"same-bytes").unwrap();
    let job = serde_json::json!({
        "input_path": "mix.wav",
        "input_sha256": HELLO_SHA256,
        "preset": "studio",
        "stems": ["left", "right"],
        "engine_version": "1.2.3",
        "outcome": "success",
        "finished_at_unix": FINISHED_AT_UNIX,
    });
    fs::write(job_dir.join("job.json"), job.to_string()).unwrap();

    let output = run(dir.path(), &["import", "run1/job.json"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = read_manifest(dir.path());
    let stem_paths: Vec<&str> = manifest["assets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["role"] == "stem")
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        stem_paths,
        vec!["run1/left.wav", "run1/right.wav"],
        "identical bytes at two paths are two assets"
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

// --- M5 slice 1: the `--project` flag everywhere and the flock sidecar ---

/// DoD: the pinned cross-tool argv `import --project <abs-root> <abs-job.json>`
/// registers a job from any cwd, with both paths absolute.
#[test]
fn import_project_flag_works_with_absolute_paths_from_an_unrelated_cwd() {
    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    let job_abs = dir.path().join(&job);
    // Run from a directory that is not the project and not its parent.
    let elsewhere = TempDir::new().unwrap();

    let output = Command::new(BIN)
        .args([
            "import",
            "--project",
            dir.path().to_str().unwrap(),
            job_abs.to_str().unwrap(),
        ])
        .current_dir(elsewhere.path())
        .output()
        .expect("failed to run the binary");
    assert!(
        output.status.success(),
        "pinned argv should import from any cwd: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = read_manifest(dir.path());
    assert_eq!(manifest["derivations"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["assets"].as_array().unwrap().len(), 2);
}

/// `--project` names the root itself: every command reads exactly
/// `<dir>/uncompose.project.json` and works from an unrelated cwd.
#[test]
fn every_command_honors_project_flag_from_an_unrelated_cwd() {
    let root = TempDir::new().unwrap();
    let elsewhere = TempDir::new().unwrap();
    let at = |args: &[&str]| -> Output {
        let mut full = vec!["--project", root.path().to_str().unwrap()];
        full.extend_from_slice(args);
        Command::new(BIN)
            .args(&full)
            .current_dir(elsewhere.path())
            .output()
            .expect("failed to run the binary")
    };

    assert!(at(&["init", "--name", "remote"]).status.success());
    assert!(root.path().join(MANIFEST_FILENAME).exists());
    fs::write(root.path().join("song.wav"), b"hello").unwrap();
    assert!(at(&["add", "song.wav"]).status.success());
    assert!(at(&["verify"]).status.success());
    let show = at(&["show"]);
    assert!(show.status.success());
    assert!(String::from_utf8(show.stdout).unwrap().contains("remote"));
}

/// No upward walk: pointing `--project` at a subdirectory of a project refuses,
/// naming the manifest path it looked for (never the parent's manifest).
#[test]
fn project_flag_does_not_walk_up_to_a_parent_manifest() {
    let dir = init_project();
    let sub = dir.path().join("nested");
    fs::create_dir(&sub).unwrap();

    let output = run(&sub, &["show"]);
    assert!(
        !output.status.success(),
        "a subdirectory of a project is not itself a project"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("not an uncompose project"),
        "should refuse as not-a-project: {stderr}"
    );
    assert!(
        stderr.contains(&sub.join(MANIFEST_FILENAME).display().to_string()),
        "should name the manifest path it expected, not the parent's: {stderr}"
    );
}

/// Spawn a background process that acquires the exclusive project lock, signals
/// readiness by creating `marker`, then holds the lock until it exits. Uses
/// `flock --no-fork ... exec sleep`, so the single held process can be SIGKILLed
/// to release the lock (no forked child inherits the locked fd).
#[cfg(unix)]
fn spawn_lock_holder(root: &Path, marker: &Path, hold_secs: u32) -> std::process::Child {
    let lock = root.join(LOCK_FILENAME);
    Command::new("flock")
        .arg("--no-fork")
        .arg(&lock)
        .arg("sh")
        .arg("-c")
        .arg(format!(
            "touch {}; exec sleep {hold_secs}",
            marker.display()
        ))
        .spawn()
        .expect("failed to spawn flock lock holder")
}

/// Wait until `marker` appears — i.e. the holder has the lock — or time out.
#[cfg(unix)]
fn wait_for(marker: &Path) {
    let start = Instant::now();
    while !marker.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "lock holder never signaled readiness"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A contended mutating command waits for the lock (printing the notice) and then
/// succeeds once the holder releases it — a blocking wait, never a fail-fast.
#[cfg(unix)]
#[test]
fn a_held_lock_makes_a_mutating_command_wait_then_succeed() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    let marker = dir.path().join("holder-ready");

    let mut holder = spawn_lock_holder(dir.path(), &marker, 2);
    wait_for(&marker);

    // `add` must block on the held lock, print the notice, then succeed.
    let start = Instant::now();
    let output = run(dir.path(), &["add", "song.wav"]);
    let waited = start.elapsed();
    holder.wait().unwrap();

    assert!(
        output.status.success(),
        "add should wait for the lock then succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains(LOCK_WAIT_NOTICE),
        "a contended command should print the waiting notice: {stderr}"
    );
    assert!(
        waited >= Duration::from_millis(500),
        "add should have blocked until the holder released, waited {waited:?}"
    );
    assert_eq!(
        read_manifest(dir.path())["assets"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// Two real processes mutating one project concurrently both succeed, and the
/// manifest ends canonical and complete — neither write is lost (the DoD).
#[cfg(unix)]
#[test]
fn two_concurrent_adds_both_land_without_losing_a_write() {
    let dir = init_project();
    fs::write(dir.path().join("a.wav"), b"aaaa").unwrap();
    fs::write(dir.path().join("b.wav"), b"bbbbb").unwrap();

    let spawn_add = |file: &str| {
        Command::new(BIN)
            .args(["add", file])
            .current_dir(dir.path())
            .spawn()
            .expect("failed to spawn add")
    };
    // Spawn both before waiting on either, so their read-modify-writes overlap.
    let mut first = spawn_add("a.wav");
    let mut second = spawn_add("b.wav");
    assert!(first.wait().unwrap().success());
    assert!(second.wait().unwrap().success());

    let manifest = read_manifest(dir.path());
    let assets = manifest["assets"].as_array().unwrap();
    let mut ids: Vec<&str> = assets.iter().map(|a| a["id"].as_str().unwrap()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["a", "b"], "both adds must survive; no lost write");
    // The manifest is canonical (reparses, ends in a trailing newline) and valid.
    let bytes = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert!(
        bytes.ends_with("}\n"),
        "manifest should end canonical: {bytes}"
    );
    assert_valid_against_schema(&manifest);
}

/// Crash recovery: a killed lock holder releases its advisory lock (kernel
/// cleanup), so the next mutating command acquires immediately rather than
/// wedging. If the lock leaked, this `add` would block forever.
#[cfg(unix)]
#[test]
fn a_killed_lock_holder_does_not_wedge_the_next_command() {
    let dir = init_project();
    fs::write(dir.path().join("song.wav"), b"hello").unwrap();
    let marker = dir.path().join("holder-ready");

    // Holder grabs the lock and would hold it for an hour…
    let mut holder = spawn_lock_holder(dir.path(), &marker, 3600);
    wait_for(&marker);
    // …but is killed mid-hold. The kernel releases the advisory lock.
    holder.kill().unwrap();
    holder.wait().unwrap();

    let output = run(dir.path(), &["add", "song.wav"]);
    assert!(
        output.status.success(),
        "a killed holder must not wedge the next command: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        read_manifest(dir.path())["assets"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

// --- M5 slice 2: import dispatch on schema URL, evaluation import ---

/// Register two mixes to compare, then write a compare record (uncompose#65) into
/// `<root>/evaluations/<name>.json` referencing them by asset id. `preference` is
/// the preferred candidate's label (or `Value::Null` for no preference), mapped
/// through `candidates` to an asset id. Returns the record path relative to root.
fn synth_compare(
    root: &Path,
    name: &str,
    a_asset: &str,
    b_asset: &str,
    preference: Value,
    confidence: Option<Value>,
    completed_at: &str,
) -> String {
    let eval_dir = root.join("evaluations");
    fs::create_dir_all(&eval_dir).unwrap();
    let mut record = serde_json::json!({
        "schema": COMPARE_SCHEMA_URL,
        "candidates": [
            { "label": "A", "asset": a_asset },
            { "label": "B", "asset": b_asset },
        ],
        "preference": preference,
        "completed_at": completed_at,
        // An unknown extra the importer must tolerate: the record is evidence.
        "observations": [{ "loop": 1, "note": "stays in the record file" }],
    });
    if let Some(c) = confidence {
        record["confidence"] = c;
    }
    let rel = format!("evaluations/{name}.json");
    fs::write(root.join(&rel), record.to_string()).unwrap();
    rel
}

/// Register `mix-a.wav` and `mix-b.wav` so their auto-minted ids are `mix-a` and
/// `mix-b`, and return the project.
fn project_with_two_mixes() -> TempDir {
    let dir = init_project();
    fs::write(dir.path().join("mix-a.wav"), b"aaaa").unwrap();
    fs::write(dir.path().join("mix-b.wav"), b"bbbb").unwrap();
    assert!(run(dir.path(), &["add", "mix-a.wav"]).status.success());
    assert!(run(dir.path(), &["add", "mix-b.wav"]).status.success());
    dir
}

/// Happy path: a compare record imports as one evaluation whose full content is
/// correct — candidates in record order, preference mapped label→asset, confidence
/// copied, created_at from the record, and a hashed record ref.
#[test]
fn import_lands_a_compare_record_as_one_evaluation() {
    let dir = project_with_two_mixes();
    let record = synth_compare(
        dir.path(),
        "cmp",
        "mix-a",
        "mix-b",
        Value::from("A"),
        Some(Value::from(0.9)),
        "2020-01-03T00:00:00Z",
    );

    let output = run(dir.path(), &["import", &record]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("evaluation") && stdout.contains("mix-a") && stdout.contains("mix-b"),
        "summary should name the evaluation and candidates: {stdout}"
    );

    let manifest = read_manifest(dir.path());
    // No new assets/derivations — an evaluation only references existing assets.
    assert_eq!(manifest["assets"].as_array().unwrap().len(), 2);
    assert_eq!(manifest["derivations"].as_array().unwrap().len(), 0);

    let evals = manifest["evaluations"].as_array().unwrap();
    assert_eq!(evals.len(), 1);
    let e = &evals[0];
    assert_eq!(e["id"], "mix-a-vs-mix-b");
    assert_eq!(e["candidates"], serde_json::json!(["mix-a", "mix-b"]));
    // Preference resolved the label "A" through candidates to the asset id.
    assert_eq!(e["preference"], "mix-a");
    assert_eq!(e["confidence"], 0.9);
    assert_eq!(e["created_at"], "2020-01-03T00:00:00Z");
    assert_eq!(e["record"]["path"], "evaluations/cmp.json");
    let sha = e["record"]["sha256"].as_str().unwrap();
    assert_eq!(sha.len(), 64);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    // The record's observations were not absorbed into the manifest.
    assert!(
        !manifest.to_string().contains("stays in the record file"),
        "the verdict is referenced, not duplicated: {manifest}"
    );

    assert_valid_against_schema(&manifest);
}

/// A record with no preference keeps `preference` null in the evaluation, and a
/// missing confidence is omitted rather than written as null.
#[test]
fn import_maps_a_null_preference_to_null_and_omits_missing_confidence() {
    let dir = project_with_two_mixes();
    let record = synth_compare(
        dir.path(),
        "tie",
        "mix-a",
        "mix-b",
        Value::Null,
        None,
        "2020-01-03T00:00:00Z",
    );

    assert!(run(dir.path(), &["import", &record]).status.success());

    let manifest = read_manifest(dir.path());
    let e = &manifest["evaluations"].as_array().unwrap()[0];
    assert!(e["preference"].is_null(), "null preference stays null: {e}");
    assert!(
        e.get("confidence").is_none(),
        "a missing confidence is omitted, not null: {e}"
    );
    assert_valid_against_schema(&manifest);
}

/// Regression: a job.json (no `schema` field) still imports as a derivation — the
/// dispatch leaves the original path byte-for-byte unchanged.
#[test]
fn import_still_dispatches_a_schemaless_job_record_to_the_job_path() {
    let dir = init_project();
    let job = synth_job(
        dir.path(),
        "mix.wav",
        b"hello",
        HELLO_SHA256,
        "run1",
        &["vocals"],
        "success",
    );
    assert!(run(dir.path(), &["import", &job]).status.success());

    let manifest = read_manifest(dir.path());
    assert_eq!(manifest["derivations"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["evaluations"].as_array().unwrap().len(), 0);
}

/// A file whose `schema` is neither absent nor the compare URL refuses, naming the
/// URL found, and leaves the manifest untouched.
#[test]
fn import_refuses_an_unrecognized_schema_url() {
    let dir = init_project();
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    let bogus = "https://uncompose.org/schemas/compare/v99/uncompose.compare.schema.json";
    fs::write(
        dir.path().join("other.json"),
        serde_json::json!({ "schema": bogus }).to_string(),
    )
    .unwrap();

    let output = run(dir.path(), &["import", "other.json"]);
    assert!(!output.status.success(), "an unknown schema should refuse");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains(bogus),
        "the error should name the schema URL found: {stderr}"
    );

    let after = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();
    assert_eq!(before, after);
}

/// A candidate with no `asset` ref refuses: v0.1 registers project-launched
/// records only.
#[test]
fn import_refuses_a_candidate_without_an_asset_ref() {
    let dir = project_with_two_mixes();
    let record = serde_json::json!({
        "schema": COMPARE_SCHEMA_URL,
        "candidates": [
            { "label": "A", "asset": "mix-a" },
            { "label": "B" },
        ],
        "preference": "A",
        "completed_at": "2020-01-03T00:00:00Z",
    });
    fs::create_dir_all(dir.path().join("evaluations")).unwrap();
    fs::write(dir.path().join("evaluations/cmp.json"), record.to_string()).unwrap();
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();

    let output = run(dir.path(), &["import", "evaluations/cmp.json"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("asset"),
        "the error should mention the missing asset ref: {stderr}"
    );
    assert_eq!(
        before,
        fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap()
    );
}

/// A candidate referencing an asset id absent from the manifest refuses, naming it.
#[test]
fn import_refuses_a_candidate_asset_absent_from_the_manifest() {
    let dir = project_with_two_mixes();
    let record = synth_compare(
        dir.path(),
        "cmp",
        "mix-a",
        "ghost",
        Value::from("A"),
        None,
        "2020-01-03T00:00:00Z",
    );
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();

    let output = run(dir.path(), &["import", &record]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("ghost"),
        "the error should name the unknown asset: {stderr}"
    );
    assert_eq!(
        before,
        fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap()
    );
}

/// A compare record outside the project root refuses; the manifest is untouched.
#[test]
fn import_refuses_a_compare_record_outside_the_project_root() {
    let dir = project_with_two_mixes();
    let before = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();

    let outside = dir.path().parent().unwrap().join("uncompose-outside-cmp");
    fs::create_dir_all(&outside).unwrap();
    fs::write(
        outside.join("cmp.json"),
        serde_json::json!({ "schema": COMPARE_SCHEMA_URL }).to_string(),
    )
    .unwrap();

    let output = run(dir.path(), &["import", "../uncompose-outside-cmp/cmp.json"]);
    let _ = fs::remove_dir_all(&outside);
    assert!(
        !output.status.success(),
        "an out-of-root record should refuse"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("outside the project root"),
        "the error should name the confinement: {stderr}"
    );
    assert_eq!(
        before,
        fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap()
    );
}

/// Idempotency: re-importing the same compare record (matching sha256) exits 0 as
/// a stated no-op and leaves the manifest byte-identical.
#[test]
fn a_second_import_of_the_same_compare_record_is_a_stated_noop() {
    let dir = project_with_two_mixes();
    let record = synth_compare(
        dir.path(),
        "cmp",
        "mix-a",
        "mix-b",
        Value::from("A"),
        Some(Value::from(0.9)),
        "2020-01-03T00:00:00Z",
    );
    assert!(run(dir.path(), &["import", &record]).status.success());
    let after_first = fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap();

    let output = run(dir.path(), &["import", &record]);
    assert!(
        output.status.success(),
        "a re-import must exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.to_lowercase().contains("already") && stdout.contains("mix-a-vs-mix-b"),
        "the no-op should name the existing evaluation: {stdout}"
    );
    assert_eq!(
        after_first,
        fs::read_to_string(dir.path().join(MANIFEST_FILENAME)).unwrap()
    );
}

/// The same record path with different bytes imports as a second evaluation.
#[test]
fn a_modified_compare_record_at_the_same_path_imports_a_second_evaluation() {
    let dir = project_with_two_mixes();
    let record = synth_compare(
        dir.path(),
        "cmp",
        "mix-a",
        "mix-b",
        Value::from("A"),
        None,
        "2020-01-03T00:00:00Z",
    );
    assert!(run(dir.path(), &["import", &record]).status.success());

    // Rewrite the record in place with a different verdict — same path, new bytes.
    synth_compare(
        dir.path(),
        "cmp",
        "mix-a",
        "mix-b",
        Value::from("B"),
        None,
        "2020-01-04T00:00:00Z",
    );
    assert!(run(dir.path(), &["import", &record]).status.success());

    let manifest = read_manifest(dir.path());
    let evals = manifest["evaluations"].as_array().unwrap();
    assert_eq!(evals.len(), 2, "a differently-hashed record is a new entry");
    // The disambiguated id keeps the second entry distinct.
    assert_eq!(evals[0]["id"], "mix-a-vs-mix-b");
    assert_eq!(evals[1]["id"], "mix-a-vs-mix-b-2");
    let prefs: Vec<&Value> = evals.iter().map(|e| &e["preference"]).collect();
    assert!(
        prefs.contains(&&Value::from("mix-a")) && prefs.contains(&&Value::from("mix-b")),
        "both verdicts recorded: {prefs:?}"
    );
    assert_valid_against_schema(&manifest);
}

/// `show` renders an imported evaluation: id, candidates, preference, confidence,
/// and the hashed record ref.
#[test]
fn show_renders_an_imported_evaluation() {
    let dir = project_with_two_mixes();
    let record = synth_compare(
        dir.path(),
        "cmp",
        "mix-a",
        "mix-b",
        Value::from("A"),
        Some(Value::from(0.9)),
        "2020-01-03T00:00:00Z",
    );
    assert!(run(dir.path(), &["import", &record]).status.success());

    let show = run(dir.path(), &["show"]);
    assert!(show.status.success());
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("Evaluations (1)"), "{stdout}");
    assert!(stdout.contains("mix-a-vs-mix-b"), "{stdout}");
    assert!(
        stdout.contains("preference") && stdout.contains("mix-a"),
        "overview should show the preference: {stdout}"
    );
    assert!(
        stdout.contains("confidence") && stdout.contains("0.9"),
        "overview should show the confidence: {stdout}"
    );
    assert!(
        stdout.contains("evaluations/cmp.json"),
        "overview should show the record ref path: {stdout}"
    );
}

/// `verify` polices an evaluation's record file: a missing record fails the run,
/// and its path is reported missing.
#[test]
fn verify_flags_a_missing_evaluation_record() {
    let dir = project_with_two_mixes();
    let record = synth_compare(
        dir.path(),
        "cmp",
        "mix-a",
        "mix-b",
        Value::from("A"),
        None,
        "2020-01-03T00:00:00Z",
    );
    assert!(run(dir.path(), &["import", &record]).status.success());
    // A clean project verifies (assets and the record file all present).
    assert!(run(dir.path(), &["verify"]).status.success());

    fs::remove_file(dir.path().join(&record)).unwrap();

    let output = run(dir.path(), &["verify"]);
    assert!(
        !output.status.success(),
        "a missing record file should fail verify"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("evaluations/cmp.json") && combined.contains("missing"),
        "verify should report the record path missing: {combined}"
    );
}

/// `verify` flags an evaluation record whose bytes changed after import as
/// modified.
#[test]
fn verify_flags_a_modified_evaluation_record() {
    let dir = project_with_two_mixes();
    let record = synth_compare(
        dir.path(),
        "cmp",
        "mix-a",
        "mix-b",
        Value::from("A"),
        None,
        "2020-01-03T00:00:00Z",
    );
    assert!(run(dir.path(), &["import", &record]).status.success());

    // Tamper with the record file after import.
    fs::write(dir.path().join(&record), b"{}").unwrap();

    let output = run(dir.path(), &["verify"]);
    assert!(
        !output.status.success(),
        "a modified record file should fail verify"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("evaluations/cmp.json") && combined.contains("modified"),
        "verify should report the record path modified: {combined}"
    );
}
