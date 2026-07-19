#![cfg(windows)]

use snapshot::Store;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Storage::FileSystem::{
    CreateSymbolicLinkW, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
};

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "snapshot_namespace_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_junction(link: &Path, target: &Path) {
    let status = Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .status()
        .expect("launch mklink /J");
    assert!(status.success(), "mklink /J failed with {status}");
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

const FILE_ATTRIBUTE_OFFLINE_RAW: u32 = 0x0000_1000;
const FILE_ATTRIBUTE_REPARSE_POINT_RAW: u32 = 0x0000_0400;
const FILE_ATTRIBUTE_RECALL_ON_OPEN_RAW: u32 = 0x0004_0000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_RAW: u32 = 0x0040_0000;

fn carries_dehydrated_payload_metadata(attributes: u32) -> bool {
    attributes
        & (FILE_ATTRIBUTE_OFFLINE_RAW
            | FILE_ATTRIBUTE_RECALL_ON_OPEN_RAW
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_RAW)
        != 0
}

/// Metadata-only preflight. This never opens a file for data access, so it
/// cannot accidentally hydrate the fixture merely while deciding whether the
/// ignored integration gate is valid to run.
fn dehydrated_payload_metadata_count(root: &Path) -> Result<(usize, usize), String> {
    let mut pending = vec![root.to_path_buf()];
    let mut scanned_files = 0usize;
    let mut dehydrated_files = 0usize;
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            format!("enumerate metadata under {}: {error}", directory.display())
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("read metadata entry under {}: {error}", directory.display())
            })?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                format!(
                    "query link-safe metadata for {}: {error}",
                    entry.path().display()
                )
            })?;
            let attributes = metadata.file_attributes();
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                // The preflight does not resolve descendant reparse directories;
                // the real anchored loader owns tag classification and refusal.
                if attributes & FILE_ATTRIBUTE_REPARSE_POINT_RAW == 0 {
                    pending.push(entry.path());
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            scanned_files += 1;
            if carries_dehydrated_payload_metadata(attributes) {
                dehydrated_files += 1;
            }
        }
    }
    Ok((scanned_files, dehydrated_files))
}

#[test]
fn intermediate_junction_is_name_surrogate_rejected() {
    let temp = Temp::new("intermediate_junction");
    let root = temp.0.join("store");
    let game = root.join("g");
    let outside = temp.0.join("outside_recorder");
    fs::create_dir_all(&game).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("manifest.json"), b"{}").unwrap();
    make_junction(&game.join("recorder"), &outside);

    let store = Store::open_directory(&root).unwrap();
    let error = store
        .read_game_file_bytes("g", "recorder/manifest.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("name-surrogate"), "{error}");
    assert!(error.contains("recorder"), "{error}");
}

#[test]
fn game_component_junction_is_name_surrogate_rejected() {
    let temp = Temp::new("game_junction");
    let root = temp.0.join("store");
    let outside = temp.0.join("outside_game");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("timeline.json"), b"{}").unwrap();
    make_junction(&root.join("g"), &outside);

    let store = Store::open_directory(&root).unwrap();
    let error = store
        .read_game_file_bytes("g", "timeline.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("name-surrogate"), "{error}");
    assert!(
        error.contains("component") && error.contains('g'),
        "{error}"
    );
}

#[test]
fn file_symlink_capability_and_rejection_are_mandatory() {
    let temp = Temp::new("file_symlink");
    let root = temp.0.join("store");
    let game = root.join("g");
    fs::create_dir_all(&game).unwrap();
    let target = game.join("target.json");
    let link = game.join("timeline.json");
    fs::write(&target, b"{}").unwrap();
    let link_wide = wide(&link);
    let target_wide = wide(&target);
    let created = unsafe {
        CreateSymbolicLinkW(
            link_wide.as_ptr(),
            target_wide.as_ptr(),
            SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
        )
    };
    if !created {
        let code = unsafe { GetLastError() };
        if code == 1314 {
            panic!(
                "F3_ENV_GATE=SYMLINK_PRIVILEGE_NOT_HELD: CreateSymbolicLinkW failed with Win32 1314; enable Windows Developer Mode (or supply SeCreateSymbolicLinkPrivilege), then rerun this exact gate"
            );
        }
        panic!(
            "F3_ENV_GATE=SYMLINK_PROVISION_UNEXPECTED_ERROR: CreateSymbolicLinkW failed with Win32 {code}; Developer Mode alone may not resolve this error"
        );
    }

    let store = Store::open_directory(&root).unwrap();
    let error = store
        .read_game_file_bytes("g", "timeline.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("name-surrogate"), "{error}");
    assert!(error.contains("timeline.json"), "{error}");
}

#[test]
fn linked_root_is_permitted_and_descendants_remain_anchored() {
    let temp = Temp::new("linked_root");
    let real = temp.0.join("real_store");
    let game = real.join("g");
    fs::create_dir_all(&game).unwrap();
    fs::write(game.join("timeline.json"), br#"{"legacy":true}"#).unwrap();
    let linked = temp.0.join("linked_store");
    make_junction(&linked, &real);

    let store = Store::open_directory(&linked).unwrap();
    let bytes = store
        .read_game_file_bytes("g", "timeline.json")
        .unwrap()
        .unwrap();
    assert_eq!(bytes, br#"{"legacy":true}"#);
}

/// Must run on the primary OneDrive dev rig before ship. Point the environment
/// variable at a genuinely dehydrated directory replay store; this proves that
/// the non-surrogate reopen hydrates and returns logical bytes.
#[test]
#[ignore = "requires a real dehydrated OneDrive Files-On-Demand store on the dev rig"]
fn windows_onedrive_dehydrated_placeholder_store_must_pass_before_ship() {
    let path = std::env::var_os("BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE").or_else(|| std::env::var_os("HOI4_ONEDRIVE_DEHYDRATED_REPLAY_STORE"))
        .map(PathBuf::from)
        .expect("F3_ENV_GATE=ONEDRIVE_STORE_ENV_NOT_SET: set BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE to a genuinely dehydrated directory store");
    let (scanned_files, dehydrated_files) = dehydrated_payload_metadata_count(&path)
        .unwrap_or_else(|error| panic!("F3_ENV_GATE=ONEDRIVE_METADATA_PROBE_FAILED: {error}"));
    assert!(
        dehydrated_files > 0,
        "F3_ENV_GATE=ONEDRIVE_STORE_NOT_DEHYDRATED: metadata-only scan found {dehydrated_files} dehydrated payloads among {scanned_files} files; refusing a vacuous hydrated-store pass"
    );
    let store = Store::open_directory(&path).unwrap();
    let games = store.games().unwrap();
    let game = games
        .first()
        .expect("dehydrated integration store must contain one game");
    let index = store.index(game).unwrap();
    let first = index
        .snapshots
        .first()
        .expect("dehydrated integration store must have a snapshot");
    let snapshot = store.load_file(game, &first.file).unwrap();
    assert_eq!(snapshot.game_id.as_deref(), Some(game.as_str()));
}

#[test]
fn dehydrated_placeholder_attribute_probe_is_exact_and_non_vacuous() {
    assert!(!carries_dehydrated_payload_metadata(0));
    assert!(!carries_dehydrated_payload_metadata(
        FILE_ATTRIBUTE_REPARSE_POINT_RAW
    ));
    for attribute in [
        FILE_ATTRIBUTE_OFFLINE_RAW,
        FILE_ATTRIBUTE_RECALL_ON_OPEN_RAW,
        FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_RAW,
    ] {
        assert!(carries_dehydrated_payload_metadata(attribute));
    }
}
