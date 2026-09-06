#![cfg(windows)]

use std::fs::File;
use std::io::Read as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::process::CommandExt as _;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use snapshot::{pack, PackEntry, PackIdentity};

const WORKER_EXE_NAME: &str = "Battle Plan Recorder.exe";

const INTERNAL_VERB: &str = "--internal-prefold-worker-v1";
const RECEIPT_SCHEMA: &str = "hoi4_prefold_worker_receipt_v2";
const DECODER_ID: &str = "extractor.replay_decoder.production_world_fold.worker.v1";
const DATE_EPOCH_CONST: i32 = 43_799_999;
const HOUR: i64 = 16_959_360;
const DOMAIN_FRAME: u16 = 1000;
const CHILD_LIMIT: Duration = Duration::from_secs(60);

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn bounded_output(command: &mut Command, label: &str) -> Output {
    command.creation_flags(0x0800_0000).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command
        .spawn()
        .unwrap_or_else(|error| panic!("spawn {label}: {error}"));
    let mut guard = ChildGuard(Some(child));
    let deadline = Instant::now() + CHILD_LIMIT;
    let status = loop {
        match guard.0.as_mut().expect("live child").try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => panic!("{label} exceeded the 60 second test limit"),
            Err(error) => panic!("poll {label}: {error}"),
        }
    };

    let child = guard.0.as_mut().expect("completed child");
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .expect("piped child stdout")
        .read_to_end(&mut stdout)
        .unwrap_or_else(|error| panic!("read {label} stdout: {error}"));
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .expect("piped child stderr")
        .read_to_end(&mut stderr)
        .unwrap_or_else(|error| panic!("read {label} stderr: {error}"));
    let _ = guard.0.take();
    Output {
        status,
        stdout,
        stderr,
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "replay_app_recorder_prefold_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir(&root).expect("create fixture root");
        Self(root)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn push_record(out: &mut Vec<u8>, sequence: u64, domain: u16, kind: u16, payload: &[u8]) {
    out.extend_from_slice(b"H4CR");
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
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
    country.extend_from_slice(b"CIDN\x03\x00\x08\x00");
    country.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
    country.extend_from_slice(&47u32.to_le_bytes());
    country.extend_from_slice(&3u16.to_le_bytes());
    country.extend_from_slice(b"GER");
    country.push(1);
    country.extend_from_slice(&[0, 1, 0, 1]);

    let mut bytes = Vec::new();
    push_record(&mut bytes, 1, 18, 3, &country);
    push_record(&mut bytes, 2, DOMAIN_FRAME, 2, &[0u8; 32]);
    bytes
}

fn executable_sha256(path: &Path) -> String {
    let mut file = File::open(path).expect("open executable for hashing");
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).expect("hash executable");
        if count == 0 { break; }
        hash.update(&buffer[..count]);
    }
    format!("{:x}", hash.finalize())
}

fn stage_binaries(root: &Path, include_worker: bool) -> (PathBuf, PathBuf) {
    let app = root.join("replay_app.exe");
    let built_app = Path::new(env!("CARGO_BIN_EXE_replay_app"));
    std::fs::copy(built_app, &app).expect("stage built replay_app");
    assert_eq!(executable_sha256(&app), executable_sha256(built_app));
    let worker = root.join(WORKER_EXE_NAME);
    if include_worker {
        let built_worker = PathBuf::from(std::env::var_os("BATTLE_PLAN_TEST_RECORDER_WORKER").or_else(|| std::env::var_os("HOI4_TEST_RECORDER_WORKER"))
            .expect("set BATTLE_PLAN_TEST_RECORDER_WORKER to the paired built recorder binary"));
        let expected = std::env::var("BATTLE_PLAN_TEST_RECORDER_WORKER_SHA256").or_else(|_| std::env::var("HOI4_TEST_RECORDER_WORKER_SHA256"))
            .expect("set BATTLE_PLAN_TEST_RECORDER_WORKER_SHA256 from the paired build receipt");
        assert_eq!(expected.len(), 64);
        assert_eq!(executable_sha256(&built_worker), expected, "paired recorder source image changed");
        std::fs::copy(&built_worker, &worker).expect("stage exact recorder sibling");
        assert_eq!(executable_sha256(&worker), expected);
    }
    println!("PREFOLD_DEPLOYMENT_V1 {}", serde_json::json!({
        "app_sha256": executable_sha256(&app),
        "worker_sha256": include_worker.then(|| executable_sha256(&worker)),
        "worker_file_name": WORKER_EXE_NAME,
    }));
    (app, worker)
}

fn open_substitutable_read_handle(path: &Path) -> File {
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
        .expect("open retained compact source with explicit share-delete")
}

fn write_compact_container(path: &Path, game_id: &str, compact_stream: &[u8]) {
    let entries = vec![
        PackEntry {
            path: format!("payload/{game_id}/index.json"),
            bytes: serde_json::to_vec(&serde_json::json!({
                "game_id": game_id,
                "schema_version": "1",
                "snapshots": []
            }))
            .expect("serialize compact index"),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/collector_raw/collector_stream.bin"),
            bytes: compact_stream.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some("1".to_string()),
        ..Default::default()
    };
    std::fs::write(
        path,
        pack(game_id, &entries, &identity, true).expect("pack compact recording"),
    )
    .expect("write compact recording");
}

#[test]
fn shipped_cli_opens_compact_recording_through_the_production_parent_worker() {
    let root = TempDir::new();
    let (app, _) = stage_binaries(&root.0, true);
    let recording = root.0.join("parent-boundary.battleplan");
    write_compact_container(&recording, "production-parent-worker", &stream());

    let _scanner_guard = portable_test_fs_retry::open(&recording)
        .expect("hold the recording open across the scanner's window");
    let output = bounded_output(
        Command::new(&app)
            .arg("--validate-recording")
            .arg(&recording),
        "shipped replay_app product validation path",
    );

    assert!(
        output.status.success(),
        "product parent path failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("validation output is UTF-8");
    assert!(
        stdout.contains("opened \"production-parent-worker\"")
            && stdout.contains(DECODER_ID),
        "unexpected validation summary: {stdout}"
    );
}

#[test]
fn shipped_recorder_worker_folds_the_inherited_source_handle_not_its_path() {
    let root = TempDir::new();
    let source_path = root.0.join("source.h4cr");
    let retained_path = root.0.join("retained-source.h4cr");
    let snapshot_dir = root.0.join("snapshots");
    std::fs::create_dir(&snapshot_dir).expect("create snapshot spool");
    let bytes = stream();
    std::fs::write(&source_path, &bytes).expect("write compact source");
    let source_sha256 = snapshot::sha256::Sha256::hex(&bytes);

    let retained = open_substitutable_read_handle(&source_path);
    std::fs::rename(&source_path, &retained_path).expect("move retained source pathname");
    std::fs::write(&source_path, vec![0xa5; bytes.len()]).expect("substitute source path");

    let (_, executable) = stage_binaries(&root.0, true);
    let executable_sha256 = executable_sha256(&executable);
    let output = bounded_output(
        Command::new(&executable)
            .arg(INTERNAL_VERB)
            .arg("--game-id")
            .arg("production-recorder-worker")
            .arg("--source")
            .arg(&source_path)
            .arg("--snapshot-dir")
            .arg(&snapshot_dir)
            .arg("--source-len")
            .arg(bytes.len().to_string())
            .arg("--source-sha256")
            .arg(&source_sha256)
            .arg("--identity-mode")
            .arg("legacy")
            .arg("--worker-executable-sha256")
            .arg(&executable_sha256)
            .stdin(Stdio::from(retained)),
        "shipped recorder internal worker",
    );

    assert!(
        output.status.success(),
        "recorder worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "successful worker wrote stderr");
    let receipt: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse worker receipt");
    assert_eq!(receipt["schema"], RECEIPT_SCHEMA);
    assert_eq!(receipt["decoder_id"], DECODER_ID);
    assert_eq!(receipt["worker_executable_sha256"], executable_sha256);
    assert_eq!(receipt["game_id"], "production-recorder-worker");
    assert_eq!(receipt["identity_mode"], "legacy");
    let entries = receipt["entries"].as_array().expect("receipt entries");
    assert_eq!(entries.len(), 1, "one frame-end must produce one snapshot");
    let entry = entries[0].as_object().expect("entry object");
    let file_name = entry["file_name"].as_str().expect("snapshot file name");
    let snapshot_path = snapshot_dir.join(file_name);
    let snapshot_bytes = portable_test_fs_retry::read(snapshot_path).expect("read emitted snapshot");
    let snapshot_sha256 = snapshot::sha256::Sha256::hex(&snapshot_bytes);
    assert_eq!(entry["byte_len"].as_u64(), Some(snapshot_bytes.len() as u64));
    assert_eq!(entry["sha256"].as_str(), Some(snapshot_sha256.as_str()));
    let frame: serde_json::Value = serde_json::from_slice(&snapshot_bytes).expect("worker snapshot JSON");
    assert_eq!(frame["hour_index"], HOUR);
    assert_eq!(frame["measured_countries"]["known"], true);
    assert_eq!(frame["measured_countries"]["tag_db_ids"], serde_json::json!([47]));
    assert_eq!(frame["measured_countries"]["tags"], serde_json::json!(["GER"]));
    assert_eq!(frame["tag_db_id_index"]["country_link_indices"],
        serde_json::json!({"status":"unavailable","reason":"unreadable"}));
}

#[test]
fn shipped_recorder_worker_refuses_image_and_source_identity_mismatches() {
    for wrong_image in [true, false] {
        let root = TempDir::new();
        let (_, executable) = stage_binaries(&root.0, true);
        let source_path = root.0.join("source.h4cr");
        let snapshot_dir = root.0.join("snapshots");
        std::fs::create_dir(&snapshot_dir).expect("create snapshot spool");
        let bytes = stream();
        std::fs::write(&source_path, &bytes).expect("write compact source");
        let source = portable_test_fs_retry::open(&source_path).expect("open compact source");
        let source_hash = if wrong_image { snapshot::sha256::Sha256::hex(&bytes) } else { "00".repeat(32) };
        let image_hash = if wrong_image { "00".repeat(32) } else { executable_sha256(&executable) };
        let output = bounded_output(
            Command::new(&executable)
                .arg(INTERNAL_VERB).arg("--game-id").arg("identity-refusal")
                .arg("--source").arg(&source_path)
                .arg("--snapshot-dir").arg(&snapshot_dir)
                .arg("--source-len").arg(bytes.len().to_string())
                .arg("--source-sha256").arg(source_hash)
                .arg("--identity-mode").arg("legacy")
                .arg("--worker-executable-sha256").arg(image_hash)
                .stdin(Stdio::from(source)),
            "recorder worker identity refusal",
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty(), "no successful spool receipt may escape refusal");
        let expected = if wrong_image { "internal prefold worker executable identity mismatch" }
            else { "folded H4CR source identity mismatch" };
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected), "{}", String::from_utf8_lossy(&output.stderr));
        if wrong_image {
            assert_eq!(std::fs::read_dir(&snapshot_dir).unwrap().count(), 0,
                "image rejection must precede folding");
        }
    }
}

#[test]
fn shipped_cli_refuses_a_missing_recorder_sibling_without_falling_back_to_itself() {
    let root = TempDir::new();
    let (app, worker) = stage_binaries(&root.0, false);
    assert!(!worker.exists());
    let recording = root.0.join("missing-worker.battleplan");
    write_compact_container(&recording, "missing-recorder", &stream());
    let before = executable_sha256(&recording);
    let output = bounded_output(Command::new(app).arg("--validate-recording").arg(&recording),
        "product open without recorder sibling");
    assert!(!output.status.success());
    let diagnostic = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(diagnostic.contains("recorder worker executable"), "{diagnostic}");
    assert!(diagnostic.contains(WORKER_EXE_NAME), "{diagnostic}");
    assert_eq!(executable_sha256(&recording), before);
}
