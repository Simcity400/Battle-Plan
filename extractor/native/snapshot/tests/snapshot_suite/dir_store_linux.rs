#![cfg(target_os = "linux")]

use snapshot::Store;
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::PathBuf;
use std::process::Command;

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "snapshot_linux_{tag}_{}_{}",
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

#[test]
fn posix_symlink_component_is_rejected_and_linked_root_is_allowed() {
    let temp = Temp::new("links");
    let real = temp.0.join("real");
    let game = real.join("g");
    fs::create_dir_all(&game).unwrap();
    fs::write(game.join("timeline.json"), b"{}").unwrap();
    let linked_root = temp.0.join("linked");
    symlink(&real, &linked_root).unwrap();
    let store = Store::open_directory(&linked_root).unwrap();
    assert!(store
        .read_game_file_bytes("g", "timeline.json")
        .unwrap()
        .is_some());

    let target = game.join("target.json");
    fs::write(&target, b"{}").unwrap();
    symlink(&target, game.join("linked.json")).unwrap();
    let error = store
        .read_game_file_bytes("g", "linked.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("name-surrogate symlink"), "{error}");
}

#[test]
fn intermediate_ascii_lookalikes_and_multi_variants_are_rejected() {
    let temp = Temp::new("lookalikes");
    let root = temp.0.join("store");
    let game = root.join("g");
    fs::create_dir_all(game.join("Recorder")).unwrap();
    fs::write(game.join("Recorder").join("manifest.json"), b"{}").unwrap();
    let store = Store::open_directory(&root).unwrap();
    let error = store
        .read_game_file_bytes("g", "recorder/manifest.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("ASCII-lookalike policy"), "{error}");
    assert!(error.contains("Recorder"), "{error}");

    fs::create_dir_all(game.join("recorder")).unwrap();
    fs::write(game.join("recorder").join("manifest.json"), b"{}").unwrap();
    let error = store
        .read_game_file_bytes("g", "recorder/manifest.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("ASCII-lookalike policy"), "{error}");
    assert!(error.contains("Recorder"), "{error}");
}

#[test]
fn real_bind_mount_crossing_rejects_when_environment_permits_mount() {
    let temp = Temp::new("bind_mount");
    let root = temp.0.join("store");
    let game = root.join("g");
    let mountpoint = game.join("recorder");
    let outside = temp.0.join("outside");
    fs::create_dir_all(&mountpoint).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("manifest.json"), b"{}").unwrap();
    let status = Command::new("mount")
        .args(["--bind"])
        .arg(&outside)
        .arg(&mountpoint)
        .status()
        .expect("execute mount --bind");
    if !status.success() {
        eprintln!("bind-mount integration unavailable on this host; deterministic mount-id double remains mandatory");
        return;
    }

    let same_device =
        fs::metadata(&game).unwrap().dev() == fs::metadata(&mountpoint).unwrap().dev();
    assert!(
        same_device,
        "bind-mount fixture must exercise same-st_dev crossing"
    );
    let store = Store::open_directory(&root).unwrap();
    let error = store
        .read_game_file_bytes("g", "recorder/manifest.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("mount-instance containment"), "{error}");
    let unmounted = Command::new("umount")
        .arg(&mountpoint)
        .status()
        .expect("execute umount");
    assert!(unmounted.success(), "failed to unmount bind fixture");
}
