#![cfg(windows)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const EXIT_BUDGET: Duration = Duration::from_secs(60);

#[test]
fn fatal_argument_error_exits_promptly_with_stderr_redirected() {
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

    let output = child
        .wait_with_output()
        .expect("collect collector_record output");
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
