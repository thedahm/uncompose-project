//! Thin CLI for `uncompose-project`.
//!
//! The CLI parses arguments and formats output; the core crate owns manifest
//! semantics. Errors go to stderr; a failed command exits non-zero.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use uncompose_project_core::{
    add, import, init, show, tagline, verify, AssetOrigin, ImportOutcome, ImportedAsset, Integrity,
    DEFAULT_ROLE,
};

#[derive(Parser)]
#[command(name = "uncompose-project", version, about = tagline(), arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Project root directory (defaults to the current directory). Names the root
    /// itself: the manifest must be exactly `<dir>/uncompose.project.json`; no
    /// parent directory is searched. `global` so it is accepted before or after
    /// the subcommand.
    #[arg(long, global = true, default_value = ".")]
    project: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize the project directory as an uncompose project.
    Init {
        /// Project name (defaults to the directory name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Register a file as an asset, recording its sha256, size, and path.
    Add {
        /// File to register, relative to the project root.
        path: PathBuf,
        /// Asset id (auto-minted from the filename stem by default).
        #[arg(long)]
        id: Option<String>,
        /// What the asset is for; open vocabulary (e.g. mix, stem, reference).
        #[arg(long, default_value = DEFAULT_ROLE)]
        role: String,
    },
    /// Import a completed uncompose job: register its input, stems, and the
    /// derivation that ties them together.
    Import {
        /// Path to the job's `job.json`: relative to the project root, or an
        /// absolute path that resolves inside it.
        job: PathBuf,
    },
    /// Check that each registered file still matches its recorded identity.
    Verify,
    /// Print a readable overview of the project, its assets, and derivations.
    Show {
        /// Emit the manifest verbatim as JSON instead of the human overview.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Resolve `--project` (default `.`) to an absolute root once, up front, so every
    // command names the same directory and the pinned cross-tool argv works from any
    // cwd with absolute paths. Canonicalizing also anchors the manifest at exactly
    // `<dir>/uncompose.project.json` with no upward walk — a missing directory is an
    // error here rather than a confusing "not a project" later.
    let root = match cli.project.canonicalize() {
        Ok(root) => root,
        Err(e) => {
            eprintln!(
                "error: cannot access project directory {}: {e}",
                cli.project.display()
            );
            return ExitCode::FAILURE;
        }
    };
    match cli.command {
        Command::Init { name } => run_init(&root, name),
        Command::Add { path, id, role } => run_add(&root, path, id, role),
        Command::Import { job } => run_import(&root, job),
        Command::Verify => run_verify(&root),
        Command::Show { json } => run_show(&root, json),
    }
}

fn run_init(root: &Path, name: Option<String>) -> ExitCode {
    let name = match name {
        Some(name) => name,
        None => match root.file_name().and_then(|s| s.to_str()) {
            Some(name) => name.to_string(),
            None => {
                eprintln!(
                    "error: cannot derive a project name from {}; pass --name",
                    root.display()
                );
                return ExitCode::FAILURE;
            }
        },
    };
    match init(root, &name) {
        Ok(path) => {
            println!(
                "Initialized uncompose project '{name}' ({})",
                path.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_verify(root: &Path) -> ExitCode {
    let report = match verify(root) {
        Ok(report) => report,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Passes to stdout; failures to stderr as warnings naming path and cause.
    // Evaluation record files (report.records) are policed the same way as assets.
    let mut modified = 0;
    let mut missing = 0;
    for status in report.statuses.iter().chain(&report.records) {
        match status.integrity {
            Integrity::Verified => println!("verified  {}", status.path),
            Integrity::Modified => {
                modified += 1;
                eprintln!("warning: {} modified (contents changed)", status.path);
            }
            Integrity::Missing => {
                missing += 1;
                eprintln!("warning: {} missing (file not found)", status.path);
            }
        }
    }

    if report.all_verified() {
        ExitCode::SUCCESS
    } else {
        eprintln!("error: verification failed: {modified} modified, {missing} missing");
        ExitCode::FAILURE
    }
}

/// Print the project overview, or with `--json` the manifest bytes verbatim
/// (byte-identical to the file, for scripts and pipelines).
fn run_show(root: &Path, json: bool) -> ExitCode {
    match show(root) {
        Ok(out) => {
            if json {
                // A short write here (closed pipe, full disk) must not exit 0:
                // scripts trust `show --json > copy` to be the whole manifest.
                let mut stdout = std::io::stdout();
                if let Err(e) = stdout.write_all(&out.raw).and_then(|()| stdout.flush()) {
                    eprintln!("error: failed to write the manifest to stdout: {e}");
                    return ExitCode::FAILURE;
                }
            } else {
                print!("{}", out.overview);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Render an import's stem count with its registered/reused split — `2 stems: 1
/// registered, 1 reused`. Zero-count halves are left out, so the common all-new
/// case reads plainly and a stemless job still says `0 stems`.
fn stem_tally(stems: &[ImportedAsset]) -> String {
    let reused = stems
        .iter()
        .filter(|s| s.origin == AssetOrigin::Existing)
        .count();
    let registered = stems.len() - reused;
    let noun = if stems.len() == 1 { "stem" } else { "stems" };
    let parts: Vec<String> = [(registered, "registered"), (reused, "reused")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
    if parts.is_empty() {
        format!("{} {noun}", stems.len())
    } else {
        format!("{} {noun}: {}", stems.len(), parts.join(", "))
    }
}

fn run_import(root: &Path, job: PathBuf) -> ExitCode {
    match import(root, &job) {
        Ok(ImportOutcome::Imported(report)) => {
            println!(
                "Imported '{}' ({})",
                report.derivation_id,
                stem_tally(&report.stems)
            );
            // Whether each file was captured now or was already under the
            // manifest's protection is the point of the summary, so every line
            // says which: the input resolved to an existing asset or registered,
            // each stem registered or reused.
            println!(
                "  input:      {} ({}) [{}]",
                report.input.asset.id,
                report.input.asset.path,
                match report.input.origin {
                    AssetOrigin::Registered => "registered",
                    AssetOrigin::Existing => "resolved to an existing asset",
                }
            );
            for stem in &report.stems {
                println!(
                    "  stem:       {} ({}) [{}]",
                    stem.asset.id,
                    stem.asset.path,
                    match stem.origin {
                        AssetOrigin::Registered => "registered",
                        AssetOrigin::Existing => "reused",
                    }
                );
            }
            println!("  derivation: {}", report.derivation_id);
            ExitCode::SUCCESS
        }
        Ok(ImportOutcome::AlreadyImported { derivation_id }) => {
            println!("Already imported as '{derivation_id}'; job.json unchanged, nothing to do");
            ExitCode::SUCCESS
        }
        Ok(ImportOutcome::EvaluationImported(report)) => {
            println!("Imported evaluation '{}'", report.evaluation_id);
            println!("  candidates: {}", report.candidates.join(", "));
            println!(
                "  preference: {}",
                report.preference.as_deref().unwrap_or("none")
            );
            if let Some(confidence) = &report.confidence {
                // A string confidence prints bare; anything else as compact JSON.
                let rendered = confidence
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| confidence.to_string());
                println!("  confidence: {rendered}");
            }
            println!("  evaluation: {}", report.evaluation_id);
            ExitCode::SUCCESS
        }
        Ok(ImportOutcome::EvaluationAlreadyImported { evaluation_id }) => {
            println!(
                "Already imported as evaluation '{evaluation_id}'; record unchanged, nothing to do"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_add(root: &Path, path: PathBuf, id: Option<String>, role: String) -> ExitCode {
    match add(root, &path, id.as_deref(), &role) {
        Ok(asset) => {
            println!(
                "Added asset '{}' ({}, {} bytes, role {})",
                asset.id, asset.path, asset.size, asset.role
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
