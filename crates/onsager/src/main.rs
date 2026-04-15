//! onsager — dispatcher for the AI factory CLI stack.
//!
//! This binary is a thin git-style dispatcher. It does NOT depend on any
//! subsystem crate. Instead, it looks up subcommands on PATH:
//!
//!   $ onsager stiglab serve    ->  exec `stiglab serve` (or `onsager-stiglab serve`)
//!   $ onsager synodic rules    ->  exec `synodic rules` (or `onsager-synodic rules`)
//!
//! This preserves the architectural loose coupling between subsystems —
//! they are independent binaries that coordinate via the Onsager event spine,
//! never statically linked into a shared process.

use std::env;
use std::process::{exit, Command};

const HELP: &str = "\
onsager — AI factory dispatcher

USAGE:
    onsager <subcommand> [args...]
    onsager --help
    onsager --version

Subcommands are discovered on PATH. Any executable named `onsager-<name>`
or `<name>` (for known subsystems) is a valid subcommand.

KNOWN SUBCOMMANDS:
    stiglab     Distributed AI agent session orchestration
    synodic     AI agent governance
";

const KNOWN: &[&str] = &["stiglab", "synodic"];

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        println!("{HELP}");
        exit(0);
    }

    match args[1].as_str() {
        "-h" | "--help" | "help" => {
            println!("{HELP}");
            exit(0);
        }
        "-V" | "--version" => {
            println!("onsager {}", env!("CARGO_PKG_VERSION"));
            exit(0);
        }
        sub => dispatch(sub, &args[2..]),
    }
}

fn dispatch(sub: &str, rest: &[String]) {
    // Try `onsager-<sub>` first, then `<sub>` if it's a known subsystem.
    // This supports both git-style prefixed binaries and direct binary names.
    let candidates: Vec<String> = if KNOWN.contains(&sub) {
        vec![format!("onsager-{sub}"), sub.to_string()]
    } else {
        vec![format!("onsager-{sub}")]
    };

    for candidate in &candidates {
        match Command::new(candidate).args(rest).status() {
            Ok(status) => exit(status.code().unwrap_or(1)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                eprintln!("onsager: failed to exec `{candidate}`: {e}");
                exit(127);
            }
        }
    }

    eprintln!(
        "onsager: '{sub}' is not an onsager subcommand.\n\
         Tried: {}\n\
         Make sure one of them is in your PATH.",
        candidates.join(", ")
    );
    exit(127);
}
