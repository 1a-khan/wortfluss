//! Supervision in the style of Erlang/OTP: a small supervisor process starts the real
//! work as a child process, watches it, and restarts it when it crashes.
//!
//!   vtt run            supervisor  (permanent child: always restarted after a crash)
//!    └─ vtt worker     hotkey, recording, transcription, typing, tray
//!   vtt session run    supervisor  (transient child: restarted only after a crash)
//!    └─ vtt session worker ...     recording and transcribing a session
//!
//! Like OTP's restart intensity: more than `max_restarts` crashes within `within` means
//! something is really broken, so the supervisor gives up and says so instead of
//! looping. Between restarts it waits 1, 2, 4 ... up to 30 s. A child can ask for a clean
//! restart (exit code `RESTART`, e.g. after the settings changed); that is not a crash.
//! Stopping the supervisor (Ctrl+C, SIGTERM from `vtt stop`) stops the child first and
//! gives it time to finish (e.g. a session writing its notes).
//!
//! Inside the worker, threads follow the same idea at a smaller scale (see main.rs and
//! hotkey.rs): keyboard readers are re-attached by a manager, and typing/transcription
//! handle each item in isolation, so one bad item can't take the service down.

use crate::platform;
use anyhow::{bail, Result};
use std::collections::VecDeque;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Exit code with which a child asks to be restarted right away (not counted as a crash).
pub const RESTART: i32 = 75;
/// Environment variable telling a child that it runs under a supervisor.
pub const SUPERVISED: &str = "VTT_SUPERVISED";

pub struct Policy {
    /// Name used in messages ("Wortfluss", "The session").
    pub name: &'static str,
    /// Give up after more than this many crashes within `within`. (Permanent and
    /// transient children both end the supervision with a normal exit, code 0; only a
    /// crash leads to a restart.)
    pub max_restarts: usize,
    pub within: Duration,
    /// How long the child may take to finish after SIGTERM before it is killed.
    pub grace: Duration,
}

/// Is this process running under a supervisor?
pub fn is_supervised() -> bool {
    std::env::var_os(SUPERVISED).is_some()
}

fn describe(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(c), _) => format!("exit code {c}"),
        (None, Some(9)) => "killed (SIGKILL)".into(),
        (None, Some(11)) => "crashed (segmentation fault)".into(),
        (None, Some(6)) => "aborted (SIGABRT)".into(),
        (None, Some(s)) => format!("signal {s}"),
        _ => "unknown".into(),
    }
}

/// Run `child_args(restart_number)` under supervision until it exits normally (0), the
/// supervisor is asked to stop, or the restart intensity is exceeded.
pub fn supervise(policy: &Policy, mut child_args: impl FnMut(u32) -> Vec<String>) -> Result<()> {
    platform::catch_stop_signals();
    let exe = platform::exe_path()?;
    let mut crashes: VecDeque<Instant> = VecDeque::new();
    let mut restarts = 0u32;
    loop {
        let mut child: Child = Command::new(&exe)
            .args(child_args(restarts))
            .env(SUPERVISED, "1")
            .spawn()?;
        let status = wait(&mut child, policy.grace)?;
        if platform::stop_requested() {
            return Ok(()); // we were asked to stop; the child has finished
        }
        match status.code() {
            Some(0) => return Ok(()),
            Some(RESTART) => {
                restarts += 1;
                continue;
            }
            _ => {}
        }
        // A crash.
        let now = Instant::now();
        crashes.push_back(now);
        while crashes.front().is_some_and(|t| now.duration_since(*t) > policy.within) {
            crashes.pop_front();
        }
        eprintln!("supervisor: {} {}", policy.name, describe(status));
        if crashes.len() > policy.max_restarts {
            let msg = format!(
                "{} stopped after {} crashes within {} s. See {}",
                policy.name,
                crashes.len(),
                policy.within.as_secs(),
                platform::log_file().map(|p| p.display().to_string()).unwrap_or_default()
            );
            let _ = crate::notification().summary("Wortfluss").body(&msg).show();
            bail!(msg);
        }
        let backoff = Duration::from_secs((1u64 << (crashes.len() - 1).min(5)).min(30));
        eprintln!("supervisor: restarting {} in {} s", policy.name, backoff.as_secs());
        if crashes.len() == 1 {
            let _ = crate::notification()
                .summary("Wortfluss")
                .body(&format!("{} stopped unexpectedly and is being restarted.", policy.name))
                .timeout(4000)
                .show();
        }
        let until = Instant::now() + backoff;
        while Instant::now() < until {
            if platform::stop_requested() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        restarts += 1;
    }
}

/// Wait for the child; on a stop request pass SIGTERM on and allow `grace` to finish.
fn wait(child: &mut Child, grace: Duration) -> Result<std::process::ExitStatus> {
    let mut deadline: Option<Instant> = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if platform::stop_requested() {
            match deadline {
                None => {
                    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
                    deadline = Some(Instant::now() + grace);
                }
                Some(d) if Instant::now() > d => {
                    let _ = child.kill();
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_exits() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(describe(std::process::ExitStatus::from_raw(9)), "killed (SIGKILL)");
        assert_eq!(describe(std::process::ExitStatus::from_raw(3 << 8)), "exit code 3");
    }
}
