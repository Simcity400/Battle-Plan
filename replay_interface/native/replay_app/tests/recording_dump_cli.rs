use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/positive/fx_owner_controller_flip.battleplan")
}

fn bounded_output(mut command: Command) -> Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn replay_app preflight");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait().expect("poll replay_app preflight").is_some() {
            return child
                .wait_with_output()
                .expect("collect replay_app preflight output");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .expect("collect timed-out replay_app output");
            panic!(
                "replay_app invalid-hour preflight did not exit within 15s; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn invalid_canonical_dump_hour_exits_nonzero_with_requested_hour_and_valid_range() {
    let recording = fixture_path();
    let source = replay_core::open_recording(&recording).expect("open fixture index");
    let first = source.hours()[0];
    let last = *source.hours().last().expect("last fixture hour");

    for (requested, relation) in [(first - 1, "before"), (last + 1, "after")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_replay_app"));
        command
            .arg("--recording")
            .arg(&recording)
            .arg("--dump-frame")
            .arg("must_not_render.bmp")
            .arg("--dump-hour")
            .arg(requested.to_string());
        let output = bounded_output(command);

        assert_eq!(output.status.code(), Some(2), "invalid hour must exit 2");
        assert!(output.stdout.is_empty(), "invalid preflight wrote stdout");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!(
                "requested canonical hour {requested} is {relation}"
            )),
            "stderr omitted requested hour/relation: {stderr}"
        );
        assert!(
            stderr.contains(&format!(
                "recording's valid canonical range {first}..={last}"
            )),
            "stderr omitted canonical range: {stderr}"
        );
        assert!(
            !stderr.contains("resource"),
            "selection must fail before resource discovery: {stderr}"
        );
    }
}
