//! The `canon` binary.
//!
//! Two kinds of subcommand, with different contracts.
//!
//! Hook commands read a JSON payload on stdin and answer on stdout in the
//! host's protocol. They exit 0 whatever happens, never touch stderr, and log
//! to a file. A hook has no channel to a human, so there is nothing useful to
//! say on failure and several harmful ways to say it.
//!
//! Commands for people print to stdout, because someone is reading.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod args;
mod child;
mod commands;
mod config;
mod git;
mod logging;
mod paths;
mod ticket;
mod tokens;

use args::Command;

fn main() -> std::process::ExitCode {
    let command = match args::parse(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            // Argument errors reach a human, so stderr is safe here: no hook
            // has run, so nothing is listening on the protocol channel.
            report(&format!("canon: {e}"));
            return std::process::ExitCode::from(2);
        }
    };

    match command {
        Command::SessionStart => hook(commands::session_start),
        Command::SubagentStart => hook(commands::subagent_start),
        Command::Inject => hook(commands::inject),
        Command::Verify => hook(commands::verify),
        Command::Reconcile => hook(commands::reconcile),
        Command::Check => human(commands::check),
        Command::Index { rebuild } => human(move |root| commands::index(root, rebuild)),
        Command::Explain { path, id } => {
            human(move |root| commands::explain(root, path.as_deref(), id.as_deref()))
        }
        Command::Help => {
            emit(args::USAGE);
            std::process::ExitCode::SUCCESS
        }
        Command::Version => {
            emit(&format!("canon {}\n", env!("CARGO_PKG_VERSION")));
            std::process::ExitCode::SUCCESS
        }
    }
}

/// Run a hook command under the fail-open harness.
fn hook<F>(handler: F) -> std::process::ExitCode
where
    F: Fn(&canon_hook::HookInput) -> canon_hook::HookOutput + std::panic::RefUnwindSafe,
{
    canon_hook::run(|input| {
        start_logging(&input.root());
        handler(&input)
    });
    // Always zero. A refusal travels as `permissionDecision: deny` in the
    // JSON, never as an exit code: exit 2 would block the tool call with
    // whatever is on stderr, and stderr is a channel this binary must not
    // touch. Exiting non-zero would also turn every internal failure into a
    // blocked write, which is the opposite of the contract.
    std::process::ExitCode::SUCCESS
}

/// Run a command a person is waiting on.
fn human<F: FnOnce(&std::path::Path) -> String>(handler: F) -> std::process::ExitCode {
    let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    start_logging(&root);
    emit(&handler(&root));
    std::process::ExitCode::SUCCESS
}

fn start_logging(root: &std::path::Path) {
    // Resolved independently of whether the config is usable. A config that
    // will not load falls back to settings with logging off, and taking the
    // log level from that fallback silenced the one line that would have said
    // why — "config unusable, running on defaults" cannot be written by a
    // logger the same failure just turned off. `CANON_LOG` is the channel that
    // has to keep working when the file does not.
    let level = config::log_level_for(root);
    logging::init(&level, paths::log_path());
}

/// The one place in the workspace that writes human-facing text to stdout.
///
/// `print_stdout` is denied everywhere else so a stray `println!` on a hook
/// path, where stdout is the protocol channel, cannot compile.
#[allow(clippy::print_stdout)]
fn emit(text: &str) {
    use std::io::Write as _;
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = write!(lock, "{text}");
    let _ = lock.flush();
}

/// Argument errors only. Never reachable once a hook payload is in flight.
#[allow(clippy::print_stderr)]
fn report(text: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{text}");
}
