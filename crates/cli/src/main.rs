//! Thin CLI for `uncompose-project`.
//!
//! The CLI parses arguments and formats output; the core crate owns manifest
//! semantics. Errors go to stderr; a failed command exits non-zero.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use uncompose_project_core::{init, tagline};

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
}

fn main() -> ExitCode {
    match Cli::parse().command {
        None => {
            println!("uncompose-project — {}", tagline());
            ExitCode::SUCCESS
        }
        Some(Command::Init { name }) => run_init(name),
    }
}

fn run_init(name: Option<String>) -> ExitCode {
    let root = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("error: cannot determine the current directory: {e}");
            return ExitCode::FAILURE;
        }
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
