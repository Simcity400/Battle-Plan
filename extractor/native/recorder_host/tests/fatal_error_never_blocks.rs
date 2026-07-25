//! A fatal startup error must reach stderr and end the process, never block.
//!
//! Regression guard for a live wedge: `collector_record` chose between stderr
//! and a modal `MessageBoxW` by asking whether it had a *console window*. A
//! script that launches it with redirected standard handles has no console
//! window, so a rejected argument set popped a dialog nobody could see and the
//! recorder sat at 0% CPU forever with empty logs. Because the recorder holds
//! the target's job-object injection lease while it lives, and a PID that has
//! seen an injection attempt is never retried, one such hang poisoned that game
//! process for every later capture.
//!
//! The unit tests beside `should_show_cli_error` only prove the decision
//! function. This proves the process behaviour, which is the part that broke.

#![cfg(windows)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Exiting is the invariant; 60 s is a generous ceiling for an argument
/// rejection that does no I/O, not a tuning knob.
const EXIT_BUDGET: Duration = Duration::from_secs(60);

#[test]
fn fatal_argument_error_exits_promptly_with_stderr_redirected() {
    // `--profile production` without the required static bundle is rejected
    // during argument validation, so this needs no game, DLL, or capture corpus.
    let mut child = Command::new(env!("CARGO_BIN_EXE_collector_record"))
        .args(["--pid", "4", "--profile", "production"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn collector_record");

    let deadline = Instant::now() + EXIT_BUDGET;
    let status = loop {
        match child.try_wait().expect("poll collector_record") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "collector_record did not exit within {EXIT_BUDGET:?} on a fatal argument \
                     error; it is blocking on a modal dialog again"
                );
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let output = child.wait_with_output().expect("collect collector_record output");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !status.success(),
        "a rejected argument set must fail, got {status:?}"
    );
    assert!(
        stderr.contains("collector_record:"),
        "the failure reason must reach the redirected stderr, got: {stderr:?}"
    );
    assert!(
        stderr.contains("static-bundle-manifest"),
        "the specific rejection must be reported, not a generic message, got: {stderr:?}"
    );
}
