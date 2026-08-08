//! Thin CLI for `uncompose-project`.
//!
//! The CLI parses arguments and formats output; the core crate owns manifest
//! semantics. Errors go to stderr; a failed command exits non-zero.

use std::io::Write;
use std::path::PathBuf;
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
}

#[derive(Subcommand)]
enum Command {
    /// Initialize the current directory as an uncompose project.
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
        /// Path to the job's `job.json`, relative to the project root.
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
    match Cli::parse().command {
        Command::Init { name } => run_init(name),
        Command::Add { path, id, role } => run_add(path, id, role),
        Command::Import { job } => run_import(job),
        Command::Verify => run_verify(),
        Command::Show { json } => run_show(json),
    }
}

/// The project root every command operates on: the current directory. Reports
/// on stderr and returns `None` when it cannot be determined.
fn project_root() -> Option<PathBuf> {
    match std::env::current_dir() {
        Ok(dir) => Some(dir),
        Err(e) => {
            eprintln!("error: cannot determine the current directory: {e}");
            None
        }
    }
}

fn run_init(name: Option<String>) -> ExitCode {
    let Some(root) = project_root() else {
        return ExitCode::FAILURE;
    };
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
    match init(&root, &name) {
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

fn run_verify() -> ExitCode {
    let Some(root) = project_root() else {
        return ExitCode::FAILURE;
    };
    let report = match verify(&root) {
        Ok(report) => report,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Passes to stdout; failures to stderr as warnings naming path and cause.
    let mut modified = 0;
    let mut missing = 0;
    for status in &report.statuses {
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
fn run_show(json: bool) -> ExitCode {
    let Some(root) = project_root() else {
        return ExitCode::FAILURE;
    };
    match show(&root) {
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

fn run_import(job: PathBuf) -> ExitCode {
    let Some(root) = project_root() else {
        return ExitCode::FAILURE;
    };
    match import(&root, &job) {
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
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_add(path: PathBuf, id: Option<String>, role: String) -> ExitCode {
    let Some(root) = project_root() else {
        return ExitCode::FAILURE;
    };
    match add(&root, &path, id.as_deref(), &role) {
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
