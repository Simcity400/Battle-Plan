use dashboard_base::test_fixtures::{source, source_with_content};
use dashboard_core::transport::{DataEvent, SourceReply};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

#[test]
fn the_browser_bridge_answers_finished_strips_without_opening_the_disk_store() {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let local = root.join("local");
    let captures = local.join("BattlePlan/captures");
    fs::create_dir_all(&captures).unwrap();
    let (recorded, load) = source_with_content(vec![source(1, 100, Some(100))], &root.join("game"));
    let options = load.content.unwrap().options;
    let recording = captures.join("test.battleplan");
    fs::write(&recording, recorded.replay_ref().bytes()).unwrap();
    drop(recorded);
    let mut bridge = Command::new(env!("CARGO_BIN_EXE_dashboard-bridge"))
        .env("LOCALAPPDATA", &local)
        .env("APPDATA", root.join("roaming"))
        .env("BATTLEPLAN_CACHE", root.join("cache"))
        .arg("--game-dir").arg(options.game_dir)
        .arg("--user-dir").arg(options.user_dir)
        .args(["--artwork-url", "artwork:"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut input = bridge.stdin.take().unwrap();
    let mut output = BufReader::new(bridge.stdout.take().unwrap()).lines();
    let mut ask = |command: Value| {
        writeln!(input, "{command}").unwrap();
        let mut payload = String::new();
        loop {
            let event: DataEvent = serde_json::from_str(&output.next().unwrap().unwrap()).unwrap();
            let DataEvent::Chunk { last, fragment, .. } = event else { panic!("{event:?}") };
            payload.push_str(&fragment);
            if last { return payload; }
        }
    };
    let opened: SourceReply = serde_json::from_str(&ask(json!({
        "type":"open", "id":"open", "path":recording, "revision":"1"
    }))).unwrap();
    let reply: Value = serde_json::from_str(&ask(json!({
        "type":"product", "id":"strip", "request":{
            "sourceId":opened.id, "revision":opened.revision, "product":"strip",
            "params":{"product":"nuclear-arsenal", "asks":[{"branchId":"1", "hour":100}], "whole":true}
        }
    }))).unwrap();
    assert!(reply["data"]["data"].as_str().unwrap().starts_with("QlBTVA"));
    drop(input);
    assert!(bridge.wait().unwrap().success());
    assert!(!local.join("BattlePlan/viewer/strips").exists());
    fs::remove_dir_all(root).unwrap();
}
