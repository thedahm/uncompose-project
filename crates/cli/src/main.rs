//! Thin CLI for `uncompose-project`.
//!
//! The CLI parses arguments and formats output; the core crate owns manifest
//! semantics. For the M1.1 foundation this is a hello-world skeleton: it prints
//! the tool's tagline so the binary compiles, runs, and is exercised by tests.

use uncompose_project_core::tagline;

fn main() {
    println!("{}", banner());
}

/// The startup banner, kept as a pure function so it is testable without
/// spawning the process.
fn banner() -> String {
    format!("uncompose-project — {}", tagline())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_mentions_the_tool() {
        assert!(banner().starts_with("uncompose-project"));
    }
}
