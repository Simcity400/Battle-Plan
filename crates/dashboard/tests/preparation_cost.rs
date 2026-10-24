use std::{path::PathBuf, process::Command};

fn benchmark(mode: &str) {
    let exe = std::env::var_os("BATTLE_PLAN_COST_EXE").or_else(|| std::env::var_os("HOI4_COST_EXE")).map(PathBuf::from).expect(
        "Prerequisite failed: build cargo build --release -p dashboard --bin preparation-cost, copy the executable into captures/w1b-cost, then set BATTLE_PLAN_COST_EXE to the copy before compiling this benchmark; set BATTLE_PLAN_GAME_DIR and optionally BATTLE_PLAN_USER_DIR and BATTLE_PLAN_COST_RECORDING",
    );
    assert!(
        exe.is_file(),
        "Prerequisite failed: BATTLE_PLAN_COST_EXE does not exist"
    );
    let output = PathBuf::from(r"captures\w1b-cost")
        .join(format!("{mode}-pair-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&output).expect("Create benchmark evidence directory");
    let mut reports = Vec::new();
    for (name, enabled) in [("scopes", true), ("scopes-off", false)] {
        let directory = output.join(name);
        let mut command = Command::new(&exe);
        command.args(["--mode", mode, "--output"]).arg(&directory);
        if !enabled {
            command.arg("--scopes-off");
        }
        let status = command.status().expect("Start preparation-cost");
        assert!(
            status.success(),
            "Preparation cost benchmark failed its prerequisites, completion, or CPU reconciliation; inspect {}",
            directory.display()
        );
        let report: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(directory.join("report.json")).unwrap())
                .unwrap();
        reports.push(report);
    }
    assert_eq!(
        reports[0]["source"], reports[1]["source"],
        "The input binding changed between instrumentation runs"
    );
    assert_eq!(reports[0]["recorded_hours"], reports[1]["recorded_hours"]);
    if mode != "families" {
        assert!(reports[0]["decoded_output_checksum"].is_string());
        assert_eq!(
            reports[0]["decoded_output_checksum"], reports[1]["decoded_output_checksum"],
            "Decoded output checksum changed between instrumentation runs"
        );
    }
    let enabled = reports[0]["process_cpu_ns"].as_f64().unwrap();
    let disabled = reports[1]["process_cpu_ns"].as_f64().unwrap();
    serde_json::to_writer_pretty(std::fs::File::create(output.join("overhead.json")).unwrap(), &serde_json::json!({
        "scopes_process_cpu_ns":enabled,"scopes_off_process_cpu_ns":disabled,
        "instrumentation_cpu_difference_ns":enabled-disabled,"instrumentation_cpu_ratio":enabled/disabled,
        "order":["scopes","scopes-off"],"file_cache_state":"uncontrolled; paired result is diagnostic, not a cache-matched performance claim"
    })).unwrap();
    println!("Paired benchmark evidence: {}", output.display());
}

#[test]
#[ignore = "benchmark: real recording and game content; lead-run only"]
fn preparation_decode_apply_floor() {
    benchmark("decode");
}

#[test]
#[ignore = "benchmark: ordinary production driver; lead-run only"]
fn preparation_block_cost_longrec117() {
    benchmark("families");
}
