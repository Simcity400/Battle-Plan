use compact_contract::calendar::DATE_EPOCH_CONST;
use compact_contract::{
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION,
};
use std::process::Command;

const GAME: &str = "worker_process_fixture";
const HOUR: i64 = 16_959_360;
const DOMAIN_FRAME: u16 = 1000;

fn push_record(out: &mut Vec<u8>, sequence: u64, domain: u16, kind: u16, payload: &[u8]) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&(DATE_EPOCH_CONST + HOUR as i32).to_le_bytes());
    out.extend_from_slice(&domain.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
}

fn stream() -> Vec<u8> {
    let mut country = Vec::new();
    country.extend_from_slice(&0u64.to_le_bytes());
    country.extend_from_slice(&1u32.to_le_bytes());
    country.extend_from_slice(&1u32.to_le_bytes());
    country.extend_from_slice(&0u32.to_le_bytes());
    country.extend_from_slice(&47u32.to_le_bytes());
    country.extend_from_slice(&3u16.to_le_bytes());
    country.extend_from_slice(b"GER");
    country.push(1);

    let mut bytes = Vec::new();
    push_record(
        &mut bytes,
        1,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country,
    );
    push_record(
        &mut bytes,
        2,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &[0u8; 32],
    );
    bytes
}

#[test]
fn worker_process_folds_authenticated_source_and_emits_portable_receipt() {
    let root = std::env::temp_dir().join(format!(
        "replay_prefold_worker_process_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let snapshots = root.join("snapshots");
    std::fs::create_dir_all(&snapshots).unwrap();
    let source = root.join("source.h4cr");
    let bytes = stream();
    std::fs::write(&source, &bytes).unwrap();
    let sha256 = snapshot::sha256::Sha256::hex(&bytes);

    let output = Command::new(env!("CARGO_BIN_EXE_replay_prefold_worker"))
        .arg("--game-id")
        .arg(GAME)
        .arg("--source")
        .arg(&source)
        .arg("--snapshot-dir")
        .arg(&snapshots)
        .arg("--source-len")
        .arg(bytes.len().to_string())
        .arg("--source-sha256")
        .arg(&sha256)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["schema"], "hoi4_prefold_worker_receipt_v1");
    assert_eq!(receipt["decoder_id"], "extractor.replay_decoder.production_world_fold.worker.v1");
    assert_eq!(receipt["game_id"], GAME);
    assert_eq!(receipt["schema_version"], "1");
    let entries = receipt["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["hour_index"], HOUR);
    let file_name = entries[0]["file_name"].as_str().unwrap();
    let snapshot_bytes = std::fs::read(snapshots.join(file_name)).unwrap();
    assert_eq!(
        snapshot::sha256::Sha256::hex(&snapshot_bytes),
        entries[0]["sha256"].as_str().unwrap()
    );
    std::fs::remove_dir_all(root).unwrap();
}
