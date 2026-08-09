//! Behavior tests for `import` error variants the CLI seam reaches poorly, driven
//! through the public `init`/`import` API against synthesized job folders in temp
//! dirs — never internals — per the coding standards. The happy path and the
//! milestone acceptance (verify/show/dispatch) are covered at the CLI seam.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use uncompose_project_core::{
    add, import, init, AssetOrigin, ImportError, ImportOutcome, COMPARE_SCHEMA_URL,
};

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
    let job = serde_json::json!({
        "input_path": "mix.wav",
        "input_sha256": input_sha256,
        "preset": "studio",
        "stems": ["vocals"],
        "engine_version": "1.2.3",
        "outcome": outcome,
        "finished_at_unix": 1_577_923_200u64,
    });
    fs::write(job_dir.join("job.json"), job.to_string()).unwrap();
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
    let job = serde_json::json!({
        "input_path": "mix.wav",
        "preset": "studio",
        "stems": [],
        "engine_version": "1",
        "outcome": "success",
        "finished_at_unix": 1,
    });
    fs::write(job_dir.join("job.json"), job.to_string()).unwrap();

    let err = import(dir.path(), Path::new("run1/job.json")).unwrap_err();
    assert!(matches!(err, ImportError::MalformedJob(..)), "got {err:?}");
}

#[test]
fn import_refuses_a_missing_job_record() {
    let dir = project();
    let err = import(dir.path(), Path::new("nope/job.json")).unwrap_err();
    assert!(matches!(err, ImportError::JobMissing(_)), "got {err:?}");
}

#[test]
fn import_refuses_an_out_of_tree_input_with_the_add_instruction() {
    let dir = project();
    // A real input file outside the root; the job points at it via `../`.
    let outside = dir
        .path()
        .parent()
        .unwrap()
        .join("uncompose-outside-input.wav");
    fs::write(&outside, b"hello").unwrap();

    let job_dir = dir.path().join("run1");
    fs::create_dir_all(&job_dir).unwrap();
    fs::write(job_dir.join("vocals.wav"), b"vocals").unwrap();
    let job = serde_json::json!({
        "input_path": "../uncompose-outside-input.wav",
        "input_sha256": HELLO_SHA256,
        "preset": "studio",
        "stems": ["vocals"],
        "engine_version": "1",
        "outcome": "success",
        "finished_at_unix": 1,
    });
    fs::write(job_dir.join("job.json"), job.to_string()).unwrap();

    let err = import(dir.path(), Path::new("run1/job.json")).unwrap_err();
    let _ = fs::remove_file(&outside);
    match err {
        ImportError::InputOutsideRoot(_) => {
            // The message tells the user to register it first.
            assert!(
                err.to_string().contains("add"),
                "message should point at `add`: {err}"
            );
        }
        other => panic!("expected InputOutsideRoot, got {other:?}"),
    }
}

#[test]
fn import_tolerates_unknown_extra_fields_in_the_job_record() {
    let dir = project();
    let job_dir = dir.path().join("run1");
    fs::create_dir_all(&job_dir).unwrap();
    fs::write(job_dir.join("vocals.wav"), b"vocals").unwrap();
    let job = serde_json::json!({
        "input_path": "mix.wav",
        "input_sha256": HELLO_SHA256,
        "preset": "studio",
        "stems": ["vocals"],
        "engine_version": "1",
        "outcome": "success",
        "finished_at_unix": 1,
        "models": { "note": "an unknown extra the importer must tolerate" },
        "device": "cpu",
    });
    fs::write(job_dir.join("job.json"), job.to_string()).unwrap();
    fs::write(dir.path().join("mix.wav"), b"hello").unwrap();

    let report = match import(dir.path(), Path::new("run1/job.json")).unwrap() {
        ImportOutcome::Imported(r) => r,
        other => panic!("expected an import, got {other:?}"),
    };
    assert_eq!(report.stems.len(), 1);
}

#[test]
fn re_importing_the_same_job_is_a_stated_noop() {
    let dir = project();
    let job = synth_job(dir.path(), b"hello", HELLO_SHA256, "success");

    let first = match import(dir.path(), &job).unwrap() {
        ImportOutcome::Imported(r) => r,
        other => panic!("expected an import, got {other:?}"),
    };

    match import(dir.path(), &job).unwrap() {
        ImportOutcome::AlreadyImported { derivation_id } => {
            assert_eq!(derivation_id, first.derivation_id);
        }
        other => panic!("expected AlreadyImported, got {other:?}"),
    }
}

#[test]
fn import_refuses_a_stem_path_registered_with_a_conflicting_hash() {
    let dir = project();
    let job = synth_job(dir.path(), b"hello", HELLO_SHA256, "success");

    // Register the stem path, then tamper with the file so the recorded hash and
    // the bytes on disk disagree at import time.
    add(dir.path(), Path::new("run1/vocals.wav"), None, "stem").unwrap();
    fs::write(dir.path().join("run1/vocals.wav"), b"tampered").unwrap();

    let err = import(dir.path(), &job).unwrap_err();
    match err {
        ImportError::StemPathConflict {
            path,
            registered,
            actual,
        } => {
            assert_eq!(path, "run1/vocals.wav");
            assert_ne!(registered, actual);
        }
        other => panic!("expected StemPathConflict, got {other:?}"),
    }
}

#[test]
fn import_refuses_an_input_path_registered_with_a_conflicting_hash() {
    let dir = project();
    // Register the input path with different bytes, then let `synth_job` rewrite
    // the file to match the job record: hash resolution finds no matching asset,
    // and the path is taken by one recording a conflicting hash.
    fs::write(dir.path().join("mix.wav"), b"original").unwrap();
    add(dir.path(), Path::new("mix.wav"), None, "mix").unwrap();
    let job = synth_job(dir.path(), b"hello", HELLO_SHA256, "success");

    let err = import(dir.path(), &job).unwrap_err();
    match err {
        ImportError::InputPathConflict {
            path,
            registered,
            actual,
        } => {
            assert_eq!(path, "mix.wav");
            assert_eq!(actual, HELLO_SHA256);
            assert_ne!(registered, actual);
        }
        other => panic!("expected InputPathConflict, got {other:?}"),
    }
}

/// `import` is the cross-tool handoff target (`import --project <abs-root>
/// <abs-job.json>`), so an absolute job path that resolves inside the root is
/// accepted, not refused (ADR-0011). Confinement is unchanged.
#[test]
fn import_accepts_an_absolute_job_path_inside_the_root() {
    let dir = project();
    let job = synth_job(dir.path(), b"hello", HELLO_SHA256, "success");
    let abs = dir.path().join(&job);

    let outcome = import(dir.path(), &abs).unwrap();
    assert!(
        matches!(outcome, ImportOutcome::Imported(_)),
        "an absolute in-root job path should import: {outcome:?}"
    );
}

/// An absolute job path that resolves outside the root still refuses — the
/// confinement rule is enforced by resolution, not by rejecting absolute paths.
#[test]
fn import_refuses_an_absolute_job_path_outside_the_root() {
    let dir = project();
    synth_job(dir.path(), b"hello", HELLO_SHA256, "success");
    // A real job.json in a sibling directory, referenced by its absolute path.
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("job.json"), b"{}").unwrap();
    let abs = outside.path().join("job.json");

    let err = import(dir.path(), &abs).unwrap_err();
    assert!(
        matches!(err, ImportError::JobOutsideRoot(_)),
        "an out-of-root absolute job path should refuse: {err:?}"
    );
}

/// Same rule for the path the record itself carries: an absolute `input_path` is
/// refused rather than silently resolved, and the message points at `add`.
#[test]
fn import_refuses_an_absolute_input_path_in_the_record() {
    let dir = project();
    fs::write(dir.path().join("mix.wav"), b"hello").unwrap();
    let job_dir = dir.path().join("run1");
    fs::create_dir_all(&job_dir).unwrap();
    fs::write(job_dir.join("vocals.wav"), b"vocals").unwrap();
    let job = serde_json::json!({
        "input_path": dir.path().join("mix.wav").to_str().unwrap(),
        "input_sha256": HELLO_SHA256,
        "preset": "studio",
        "stems": ["vocals"],
        "engine_version": "1",
        "outcome": "success",
        "finished_at_unix": 1,
    });
    fs::write(job_dir.join("job.json"), job.to_string()).unwrap();

    let err = import(dir.path(), Path::new("run1/job.json")).unwrap_err();
    assert!(
        matches!(err, ImportError::InputAbsolutePath(_)),
        "got {err:?}"
    );
    assert!(
        err.to_string().contains("add"),
        "message should point at `add`: {err}"
    );
}

/// A hash-matched input reports as existing while fresh stems report as
/// registered, so the CLI summary can tell them apart (user story 18).
#[test]
fn the_report_marks_a_hash_matched_input_existing_and_new_stems_registered() {
    let dir = project();
    fs::write(dir.path().join("original.wav"), b"hello").unwrap();
    add(dir.path(), Path::new("original.wav"), None, "mix").unwrap();
    let job = synth_job(dir.path(), b"hello", HELLO_SHA256, "success");

    let report = match import(dir.path(), &job).unwrap() {
        ImportOutcome::Imported(r) => r,
        other => panic!("expected an import, got {other:?}"),
    };
    assert_eq!(report.input.asset.id, "original");
    assert_eq!(report.input.origin, AssetOrigin::Existing);
    assert_eq!(report.stems.len(), 1);
    assert_eq!(report.stems[0].origin, AssetOrigin::Registered);
}

/// A stem already registered at its path (matching hash) is reused, and the
/// report says so rather than claiming a fresh registration.
#[test]
fn the_report_marks_an_already_registered_stem_existing() {
    let dir = project();
    let job = synth_job(dir.path(), b"hello", HELLO_SHA256, "success");
    add(dir.path(), Path::new("run1/vocals.wav"), None, "stem").unwrap();

    let report = match import(dir.path(), &job).unwrap() {
        ImportOutcome::Imported(r) => r,
        other => panic!("expected an import, got {other:?}"),
    };
    assert_eq!(report.input.origin, AssetOrigin::Registered);
    assert_eq!(report.stems[0].origin, AssetOrigin::Existing);
}

/// Register `mix-a.wav`/`mix-b.wav` as assets `mix-a`/`mix-b` and write a compare
/// record (uncompose#65) comparing them, returning the record path relative to the
/// root. `result` is the record's verdict object verbatim, so a test can hand it a
/// well-formed or a deliberately off-shape one; everything else is a conforming v0
/// record as `uncompose-compare` writes it.
fn synth_compare(dir: &TempDir, result: serde_json::Value) -> std::path::PathBuf {
    for (file, id) in [("mix-a.wav", "mix-a"), ("mix-b.wav", "mix-b")] {
        fs::write(dir.path().join(file), id.as_bytes()).unwrap();
        add(dir.path(), Path::new(file), None, "mix").unwrap();
    }
    let eval_dir = dir.path().join("evaluations");
    fs::create_dir_all(&eval_dir).unwrap();
    let record = serde_json::json!({
        "schema": COMPARE_SCHEMA_URL,
        "id": "01J4QF8ZK3M2X7W9C5V1B6N4TQ",
        "created_at": "2020-01-03T00:00:00Z",
        "completed_at": "2020-01-03T00:10:00Z",
        "candidates": [
            { "label": "A", "path": "mix-a.wav", "sha256": A_SHA256, "size": 5, "asset": "mix-a" },
            { "label": "B", "path": "mix-b.wav", "sha256": B_SHA256, "size": 5, "asset": "mix-b" },
        ],
        "mode": "ab-blind",
        "playback": { "loudness_match": { "enabled": false } },
        "observations": [],
        "result": result,
    });
    fs::write(eval_dir.join("cmp.json"), record.to_string()).unwrap();
    Path::new("evaluations").join("cmp.json")
}

/// sha256 of `b"mix-a"` / `b"mix-b"` — the candidate identities a real record
/// carries alongside the asset refs.
const A_SHA256: &str = "4cf51cd1f675a1e1168e59a9830efe719fdb1b9cfb230068261dbe41ae5376f7";
const B_SHA256: &str = "b74dcd1fa8e5a54389f7b48782b2477a1174714dc8fb143382a415040ab8da3f";

/// The verdict lives in `result` (uncompose#65): a preference there maps through
/// the record's candidates to an asset id, and the confidence beside it is copied.
#[test]
fn import_reads_the_verdict_from_the_records_result_object() {
    let dir = project();
    let record = synth_compare(
        &dir,
        serde_json::json!({ "preference": "B", "confidence": 4 }),
    );

    match import(dir.path(), &record).unwrap() {
        ImportOutcome::EvaluationImported(report) => {
            assert_eq!(report.preference.as_deref(), Some("mix-b"));
            assert_eq!(report.confidence, Some(serde_json::Value::from(4)));
        }
        other => panic!("expected EvaluationImported, got {other:?}"),
    }
}

#[test]
fn import_refuses_a_preference_label_that_names_no_candidate() {
    let dir = project();
    // The candidates are labelled "A" and "B"; the record prefers "Z".
    let record = synth_compare(
        &dir,
        serde_json::json!({ "preference": "Z", "confidence": 3 }),
    );

    let err = import(dir.path(), &record).unwrap_err();
    match err {
        ImportError::UnknownPreference(label) => assert_eq!(label, "Z"),
        other => panic!("expected UnknownPreference, got {other:?}"),
    }
}

/// A record that does not conform to compare v0 refuses, naming the violation —
/// here a confidence outside the schema's 1–5 range. The record is evidence from
/// another tool, so its own published schema is the contract it is held to.
#[test]
fn import_refuses_a_record_that_violates_the_compare_schema() {
    let dir = project();
    let record = synth_compare(
        &dir,
        serde_json::json!({ "preference": "A", "confidence": 9 }),
    );

    match import(dir.path(), &record).unwrap_err() {
        ImportError::RecordNotConforming(path, detail) => {
            assert!(path.ends_with("cmp.json"), "names the record: {path:?}");
            assert!(detail.contains('9'), "names the violation: {detail}");
        }
        other => panic!("expected RecordNotConforming, got {other:?}"),
    }
}

/// The verdict's shape is the schema's, not a shape this tool invented: a
/// top-level `preference` (never written by `uncompose-compare`) is an unknown
/// property, and the missing `result` is a missing required one. Either way the
/// record refuses rather than importing as a silent "no preference".
#[test]
fn import_refuses_a_record_with_the_preference_outside_result() {
    let dir = project();
    let record = synth_compare(&dir, serde_json::json!({ "preference": null }));
    let path = dir.path().join(&record);
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    value["preference"] = serde_json::Value::from("A");
    value.as_object_mut().unwrap().remove("result");
    fs::write(&path, value.to_string()).unwrap();

    match import(dir.path(), &record).unwrap_err() {
        ImportError::RecordNotConforming(..) => {}
        other => panic!("expected RecordNotConforming, got {other:?}"),
    }
}
