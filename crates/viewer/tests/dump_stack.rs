
use std::{path::PathBuf, process::Command};

#[test]
#[ignore = "requires BATTLE_PLAN_DUMP_REPLAY, BATTLE_PLAN_DUMP_HOUR, BATTLE_PLAN_DUMP_BRANCH, BATTLE_PLAN_GAME_DIR, BATTLE_PLAN_USER_DIR and a GPU"]
fn a_debug_dump_loads_and_renders_on_the_main_thread_stack() {
    let var = |name| std::env::var_os(name).unwrap_or_else(|| panic!("{name} is unset"));
    let out = std::env::temp_dir().join(format!("viewer-dump-stack-{}.png", std::process::id()));
    let viewer = std::env::var_os("CARGO_BIN_EXE_viewer")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_viewer")), PathBuf::from);
    let status = Command::new(viewer)
        .arg("--dump")
        .arg(PathBuf::from(var("BATTLE_PLAN_DUMP_REPLAY").or_else(|_| var("HOI4_DUMP_REPLAY"))))
        .arg("--hour")
        .arg(var("BATTLE_PLAN_DUMP_HOUR").or_else(|_| var("HOI4_DUMP_HOUR")))
        .arg("--branch")
        .arg(var("BATTLE_PLAN_DUMP_BRANCH").or_else(|_| var("HOI4_DUMP_BRANCH")))
        .arg("--out")
        .arg(&out)
        .arg("--game-dir")
        .arg(var("BATTLE_PLAN_GAME_DIR").or_else(|_| var("HOI4_GAME_DIR")))
        .arg("--user-dir")
        .arg(var("BATTLE_PLAN_USER_DIR").or_else(|_| var("HOI4_USER_DIR")))
        .status()
        .expect("the viewer runs");
    let _ = std::fs::remove_file(&out);
    assert!(status.success(), "the dump failed: {status}");
}
