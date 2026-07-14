#![cfg(windows)]

use snapshot::Store;
use std::fs;
use std::os::windows::ffi::OsStrExt;
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
        panic!(
            "file-symlink provisioning gate failed (Win32 {code}); enable Windows Developer Mode, then rerun this mandatory test"
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
        .expect("set BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE to the dehydrated store");
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
