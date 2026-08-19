use replay_core::{Leaf, WorldTimeline};
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const STATE_COUNT: u32 = 1_081;
const HOUR_COUNT: usize = 72;
const START_HOUR: i64 = 16_959_372;

fn unique_temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "replay_core_demo_join_{}_{}",
        std::process::id(),
        nonce
    ))
}

#[test]
fn generated_demo_binds_all_owners_and_exposes_controller_flips() {
    let root = unique_temp_dir();
    let bundle = root.join("bundle");
    let recording = root.join("synthetic_demo.battleplan");
    std::fs::create_dir_all(&bundle).expect("create bundle fixture");

    let mut owners = Map::new();
    let mut province_to_state = Map::new();
    for state_id in 1..=STATE_COUNT {
        let owner = if state_id <= 216 { "FRA" } else { "GER" };
        owners.insert(state_id.to_string(), Value::String(owner.to_string()));
        province_to_state.insert(
            (10_000 + state_id).to_string(),
            Value::Number(state_id.into()),
        );
    }
    std::fs::write(
        bundle.join("state_owner_1936.json"),
        serde_json::to_vec(&owners).expect("serialize owners"),
    )
    .expect("write owners");
    std::fs::write(
        bundle.join("province_to_state.json"),
        serde_json::to_vec(&province_to_state).expect("serialize province mapping"),
    )
    .expect("write province mapping");

    let output = Command::new(env!("CARGO_BIN_EXE_make_demo_recording"))
        .args(["--bundle"])
        .arg(&bundle)
        .args(["--out"])
        .arg(&recording)
        .output()
        .expect("run make_demo_recording");
    assert!(
        output.status.success(),
        "generator failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("72 hours, 1081 owned states"),
        "unexpected generator summary: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let source = replay_core::open_recording(&recording).expect("open generated recording");
    let timeline = WorldTimeline::open(source).expect("open generated timeline");
    assert_eq!(timeline.hours().len(), HOUR_COUNT);
    assert_eq!(timeline.first_hour(), START_HOUR);
    assert_eq!(timeline.last_hour(), START_HOUR + HOUR_COUNT as i64 - 1);

    for (step, &hour) in timeline.hours().iter().enumerate() {
        let frame = timeline.frame(hour).expect("exact generated frame");
        assert_eq!(
            frame.owner_by_state().len(),
            STATE_COUNT as usize,
            "owner rows at step {step}"
        );
        assert_eq!(
            frame.control_by_state().len(),
            STATE_COUNT as usize,
            "controller rows at step {step}"
        );
        assert!(
            frame.owner_by_state().values().all(Leaf::is_measured),
            "all generated owners must bind at step {step}"
        );
        assert!(
            frame.control_by_state().values().all(Leaf::is_measured),
            "all generated controllers must bind at step {step}"
        );
        assert_eq!(
            frame.owner_by_state().get(&1),
            Some(&Leaf::Measured("FRA".to_string())),
            "occupation must preserve state 1's legal owner at step {step}"
        );
    }

    let before = timeline.frame(START_HOUR).expect("pre-flip frame");
    let after = timeline.frame(START_HOUR + 1).expect("post-flip frame");
    assert_eq!(
        before.control_by_state().get(&1),
        Some(&Leaf::Measured("FRA".to_string()))
    );
    assert_eq!(
        after.control_by_state().get(&1),
        Some(&Leaf::Measured("GER".to_string()))
    );
    assert_eq!(
        before.owner_by_state().get(&1),
        after.owner_by_state().get(&1),
        "controller flip must not mutate legal ownership"
    );

    drop(timeline);
    portable_test_fs_retry::remove_dir_all(&root).expect("remove demo gate fixture");
}
