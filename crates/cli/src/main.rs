//! Thin CLI for `uncompose-project`.
//!
//! The CLI parses arguments and formats output; the core crate owns manifest
//! semantics. Errors go to stderr; a failed command exits non-zero.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use uncompose_project_core::{add, init, tagline, DEFAULT_ROLE};

#[derive(Parser)]
#[command(name = "uncompose-project", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
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
}

fn main() -> ExitCode {
    match Cli::parse().command {
        None => {
            println!("uncompose-project — {}", tagline());
            ExitCode::SUCCESS
        }
        Some(Command::Init { name }) => run_init(name),
        Some(Command::Add { path, id, role }) => run_add(path, id, role),
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
