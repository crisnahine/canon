//! Every child process canon spawns, under one bound.
//!
//! `Command::output` blocks until the child exits with no bound of its own,
//! so this is what stands between a slow subprocess and a session that never
//! returns. Stdout is drained on its own thread rather than read after the
//! child exits: a child that fills the pipe buffer before this process gets
//! around to reading it is not slow, it is deadlocked against a parent that
//! is waiting for it to finish before reading anything it has written.
//!
//! Stdin is written on its own thread for the same reason, and it exists for
//! one caller. `curl --config -` reads its options, including a credential,
//! from stdin, because an argument vector is readable by every process on the
//! machine through `ps`.

use std::io::{Read as _, Write as _};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// What a bounded child produced.
pub(crate) struct ChildOutput {
    /// Everything the child wrote to stdout, up to the caller's cap.
    pub(crate) stdout: Vec<u8>,
    /// Whether it exited zero.
    ///
    /// Reported rather than folded into the return value, because the two
    /// callers disagree about what a non-zero exit means. A failed `git` has
    /// said nothing worth reading. A `curl` that exits non-zero on a partial
    /// transfer has already written a complete response body, and discarding
    /// it turns a readable answer into silence.
    pub(crate) success: bool,
}

/// Run `cmd` and collect its stdout, killing it if it is still running once
/// `timeout` has passed.
///
/// `stdin` is written to the child and the handle closed; `None` nulls it.
/// `max_bytes` bounds what is read, because a network response has no size
/// this process chose.
pub(crate) fn bounded(
    cmd: &mut Command,
    timeout: Duration,
    stdin: Option<Vec<u8>>,
    max_bytes: usize,
) -> Option<ChildOutput> {
    let stdin_mode = if stdin.is_some() { Stdio::piped() } else { Stdio::null() };
    let mut child =
        cmd.stdin(stdin_mode).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    if let Some(bytes) = stdin
        && let Some(mut handle) = child.stdin.take()
    {
        // Off this thread, and the handle dropped when it finishes: dropping
        // it is what the child reads as end of input, and a parent blocked on
        // a write while the child is blocked on a full stdout pipe is the
        // deadlock this whole module exists to avoid.
        thread::spawn(move || {
            let _ = handle.write_all(&bytes);
        });
    }
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(drain(stdout, max_bytes));
    });

    let collected = rx.recv_timeout(timeout).ok().flatten();
    if collected.is_none() {
        // Either the timeout fired or the reading thread never sent, and
        // either way the child may still be running. Killing an already-dead
        // process just errors, which is discarded: there is nothing useful to
        // do about it here.
        let _ = child.kill();
    }
    let status = child.wait().ok()?;
    collected.map(|stdout| ChildOutput { stdout, success: status.success() })
}

/// Read at most `max_bytes`, then stop and let the child find out.
///
/// Returning early drops the read handle, so the child's next write fails and
/// it exits, rather than filling this process's memory with a response nobody
/// asked for the size of.
fn drain(mut stdout: std::process::ChildStdout, max_bytes: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let room = max_bytes.saturating_sub(buf.len());
        if room == 0 {
            return Some(buf);
        }
        let read = stdout.read(&mut chunk).ok()?;
        if read == 0 {
            return Some(buf);
        }
        buf.extend_from_slice(chunk.get(..read.min(room))?);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Whether this machine has a POSIX shell, asked of `std` and never of
    /// `bounded`.
    ///
    /// Three tests below hand `bounded` a child that cannot finish if
    /// `bounded` is wrong, so a skip decided by `bounded` is satisfied by the
    /// exact regression those tests exist to catch and reports it as a pass.
    /// That holds however the skip is spelled: `let Some(out) = bounded(..)
    /// else { return }` and a `has_posix_shell` that calls `bounded` are the
    /// same hole one call apart. `Command::status` is the way out, because a
    /// `bounded` that answers `None` to everything cannot make it say there is
    /// no shell here.
    /// Nothing is redirected because `exit 0` writes nothing, which keeps this
    /// helper inside the two imports Step 1 is allowed to have.
    fn has_posix_shell() -> bool {
        Command::new("sh").args(["-c", "exit 0"]).status().is_ok_and(|s| s.success())
    }

    #[test]
    fn bounded_output_gives_up_rather_than_waiting_out_a_slow_child() {
        let start = std::time::Instant::now();
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let result = bounded(&mut cmd, Duration::from_millis(200), None, usize::MAX);
        assert!(result.is_none(), "a killed child must not report success output");
        assert!(start.elapsed() < Duration::from_secs(3), "took {:?}", start.elapsed());
    }

    #[test]
    fn bounded_output_returns_a_fast_childs_stdout() {
        let mut cmd = Command::new("printf");
        cmd.arg("hello");
        let result = bounded(&mut cmd, Duration::from_secs(5), None, usize::MAX);
        assert_eq!(result.map(|o| o.stdout).as_deref(), Some(b"hello".as_slice()));
    }

    #[test]
    fn a_child_that_exits_non_zero_still_hands_back_what_it_wrote() {
        // The property the fetch path needs and git does not. curl exits
        // non-zero on a partial transfer after writing a complete response
        // body, and discarding that body turns a readable answer into
        // silence. Skipped where there is no POSIX shell, the same way the
        // git tests skip where there is no git.
        if !has_posix_shell() {
            assert!(
                Command::new("sh").arg("-c").arg("exit 0").status().is_err(),
                "sh runs on this machine, so a test that cannot use it is a test bug"
            );
            return;
        }
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "printf hello; exit 3"]);
        let out = bounded(&mut cmd, Duration::from_secs(5), None, usize::MAX)
            .expect("a child that exited non-zero still hands back what it wrote");
        assert_eq!(out.stdout, b"hello");
        assert!(!out.success, "exit 3 is not success");
    }

    #[test]
    fn what_is_written_to_stdin_reaches_the_child() {
        // The whole reason this exists: the credential travels here rather
        // than in argv, where `ps` shows it to every process on the machine.
        // `cat` only ends when the write handle is dropped, so a `bounded`
        // that forgets to drop it makes this child time out; requiring `Some`
        // is what keeps that from reading as "no shell here, skip".
        if !has_posix_shell() {
            assert!(
                Command::new("sh").arg("-c").arg("exit 0").status().is_err(),
                "sh runs on this machine, so a test that cannot use it is a test bug"
            );
            return;
        }
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "cat"]);
        let payload = b"user = \"someone@example.com:secret\"".to_vec();
        let out = bounded(&mut cmd, Duration::from_secs(5), Some(payload.clone()), usize::MAX)
            .expect("closing stdin is what lets the child finish");
        assert_eq!(out.stdout, payload);
        assert!(out.success);
    }

    #[test]
    fn a_response_larger_than_the_cap_stops_at_the_cap() {
        // `drain` drops the read handle at the cap, so this child dies of
        // SIGPIPE and exits non-zero every time. Requiring `Some` is what
        // stops that from being read as "no shell here, skip".
        if !has_posix_shell() {
            assert!(
                Command::new("sh").arg("-c").arg("exit 0").status().is_err(),
                "sh runs on this machine, so a test that cannot use it is a test bug"
            );
            return;
        }
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "cat"]);
        let out = bounded(&mut cmd, Duration::from_secs(5), Some(vec![b'x'; 100_000]), 64)
            .expect("a child killed by the cap still hands back what it wrote");
        assert_eq!(out.stdout.len(), 64, "the cap is the cap");
    }

    #[test]
    fn a_program_that_is_not_installed_is_none_rather_than_a_panic() {
        let mut cmd = Command::new("canon-no-such-program-exists");
        assert!(bounded(&mut cmd, Duration::from_secs(1), None, usize::MAX).is_none());
    }
}
