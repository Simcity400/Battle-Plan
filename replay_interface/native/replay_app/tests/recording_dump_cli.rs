use std::path::PathBuf;
use std::process::Command;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/positive/fx_owner_controller_flip.battleplan")
}

#[test]
fn invalid_canonical_dump_hour_exits_nonzero_with_requested_hour_and_valid_range() {
    let recording = fixture_path();
    let source = replay_core::open_recording(&recording).expect("open fixture index");
    let first = source.hours()[0];
    let last = *source.hours().last().expect("last fixture hour");

    for (requested, relation) in [(first - 1, "before"), (last + 1, "after")] {
        let output = Command::new(env!("CARGO_BIN_EXE_replay_app"))
            .arg("--recording")
            .arg(&recording)
            .arg("--dump-frame")
            .arg("must_not_render.bmp")
            .arg("--dump-hour")
            .arg(requested.to_string())
            .output()
            .expect("run replay_app invalid-hour preflight");

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
