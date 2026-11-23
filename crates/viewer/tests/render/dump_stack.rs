use std::{fs, path::PathBuf, process::Command};

#[test]
#[ignore = "requires a GPU and an authored game, user folder, and recording.battleplan in BATTLEPLAN_VIEWER_TEST_FIXTURES/dump"]
fn a_debug_dump_loads_and_renders_on_the_main_thread_stack() {
    let fixture = PathBuf::from(
        std::env::var_os("BATTLEPLAN_VIEWER_TEST_FIXTURES")
            .expect("choose the authored viewer fixtures folder"),
    )
    .join("dump");
    let game = fixture.join("game");
    let executable = fs::read(game.join("hoi4.exe")).unwrap();
    assert_eq!(
        &engine_executable::build::ExecutableIdentity::of(&executable),
        content_reader_script::test_game::tables().identity(),
        "the dump fixture uses the synthetic game's executable"
    );
    let path = fixture.join("recording.battleplan");
    let replay = replay::campaign::Replay::open(fs::read(&path).unwrap()).unwrap();
    let block = replay.index.first().expect("the dump fixture records an hour");
    let scratch = content_reader_layers::test_support::Stack::new("debug-dump-stack");
    let scratch_dir = scratch.base();
    fs::create_dir_all(&scratch_dir).unwrap();
    let out = scratch_dir.join("dump.png");
    let viewer = std::env::var_os("CARGO_BIN_EXE_viewer")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_viewer")), PathBuf::from);
    let result = Command::new(viewer)
        .env("APPDATA", &scratch_dir)
        .env("LOCALAPPDATA", &scratch_dir)
        .arg("--dump")
        .arg(path)
        .arg("--hour")
        .arg(block.first_hour.to_string())
        .arg("--branch")
        .arg(block.branch.to_string())
        .arg("--out")
        .arg(&out)
        .arg("--game-dir")
        .arg(game)
        .arg("--user-dir")
        .arg(fixture.join("user"))
        .output()
        .expect("the debug viewer runs");
    assert!(
        result.status.success(),
        "the dump failed ({}): {}\n{}",
        result.status,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::read(out).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
}
