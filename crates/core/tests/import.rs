//! Behavior tests for `import` error variants the CLI seam reaches poorly, driven
//! through the public `init`/`import` API against synthesized job folders in temp
//! dirs — never internals — per the coding standards. The happy path and the
//! milestone acceptance (verify/show/dispatch) are covered at the CLI seam.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use uncompose_project_core::{import, init, ImportError};

/// sha256 of `b"hello"`.
const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

fn project() -> TempDir {
    let dir = TempDir::new().unwrap();
    init(dir.path(), "test-project").unwrap();
    dir
}

/// Write an in-tree input and a `run1/` job folder with one `vocals.wav` stem and
/// a `job.json`, and return the job.json path relative to the root.
fn synth_job(
    root: &Path,
    input_bytes: &[u8],
    input_sha256: &str,
    outcome: &str,
) -> std::path::PathBuf {
    fs::write(root.join("mix.wav"), input_bytes).unwrap();
    let job_dir = root.join("run1");
    fs::create_dir_all(&job_dir).unwrap();
    fs::write(job_dir.join("vocals.wav"), b"vocals").unwrap();
    let job = format!(
        "{{\"input_path\":\"mix.wav\",\"input_sha256\":\"{input_sha256}\",\"preset\":\"studio\",\"stems\":[\"vocals\"],\"engine_version\":\"1.2.3\",\"outcome\":\"{outcome}\",\"finished_at_unix\":1577923200}}\n"
    );
    fs::write(job_dir.join("job.json"), job).unwrap();
    Path::new("run1").join("job.json")
}

#[test]
fn import_refuses_a_non_success_outcome_showing_it() {
    let dir = project();
    let job = synth_job(dir.path(), b"hello", HELLO_SHA256, "cancelled");

    let err = import(dir.path(), &job).unwrap_err();
    match err {
        ImportError::NotSuccess(outcome) => assert_eq!(outcome, "cancelled"),
        other => panic!("expected NotSuccess, got {other:?}"),
    }
}

#[test]
fn import_refuses_an_input_whose_bytes_no_longer_match() {
    let dir = project();
    // Input on disk is b"world"; the job records the hash of b"hello".
    let job = synth_job(dir.path(), b"world", HELLO_SHA256, "success");

    let err = import(dir.path(), &job).unwrap_err();
    match err {
        ImportError::InputHashMismatch {
            expected, actual, ..
        } => {
            assert_eq!(expected, HELLO_SHA256);
            assert_ne!(actual, HELLO_SHA256);
        }
        other => panic!("expected InputHashMismatch, got {other:?}"),
    }
}

#[test]
fn import_refuses_a_job_record_missing_a_required_field() {
    let dir = project();
    let job_dir = dir.path().join("run1");
    fs::create_dir_all(&job_dir).unwrap();
    // No `input_sha256` — a field the contract requires.
    fs::write(
        job_dir.join("job.json"),
        "{\"input_path\":\"mix.wav\",\"preset\":\"studio\",\"stems\":[],\"engine_version\":\"1\",\"outcome\":\"success\",\"finished_at_unix\":1}\n",
    )
    .unwrap();

    let err = import(dir.path(), Path::new("run1/job.json")).unwrap_err();
    assert!(matches!(err, ImportError::MalformedJob(..)), "got {err:?}");
}

#[test]
fn import_refuses_a_missing_job_record() {
    let dir = project();
    let err = import(dir.path(), Path::new("nope/job.json")).unwrap_err();
    assert!(matches!(err, ImportError::JobMissing(_)), "got {err:?}");
}
