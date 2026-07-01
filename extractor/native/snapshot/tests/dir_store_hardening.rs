//! Directory-store hardening: the `file` field from a (possibly tampered/corrupt)
//! `index.json` must NOT be able to read outside the game directory.
//!
//! The container backend is already safe — its `load_file` resolves through the
//! verified `by_path` map, so a `..`/absolute path is simply not a key. The
//! directory backend, however, used to `root.join(game).join(file)` verbatim, so a
//! tampered index `file:"../../secret.json"` (or a Windows absolute path) escaped
//! the game dir and returned arbitrary on-disk JSON as a "snapshot". This pins the
//! read-only-scope guarantee for the dir backend (Codex bug-hunt FIX-1).

use serde_json::json;
use snapshot::Store;
use std::fs;
use std::path::PathBuf;

struct Tmp {
    path: PathBuf,
}
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "hoi4replay_dirhard_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        Tmp { path: p }
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn dir_load_file_rejects_traversal_outside_game_dir() {
    let tmp = Tmp::new("trav");
    // A "secret" file OUTSIDE the game dir (a sibling under the store root) that an
    // attacker-tampered index `file` might try to reach via `..`.
    let secret = tmp.path.join("secret.snapshot.json");
    fs::write(
        &secret,
        serde_json::to_string(&json!({"owner":"LEAKED"})).unwrap(),
    )
    .unwrap();

    // a normal game dir with a legitimate snapshot
    let gdir = tmp.path.join("g");
    fs::create_dir_all(&gdir).unwrap();
    fs::write(
        gdir.join("000000000100.snapshot.json"),
        serde_json::to_string(
            &json!({"game_id":"g","hour_index":100,"states":{"1":{"owner":"FRA"}}}),
        )
        .unwrap(),
    )
    .unwrap();

    let store = Store::open(&tmp.path);

    // the legitimate file still loads fine.
    assert!(store.load_file("g", "000000000100.snapshot.json").is_ok());
    // a segment-prefixed rollback name (legitimate, contains a '/') still loads.
    fs::create_dir_all(gdir.join("seg3")).unwrap();
    fs::write(
        gdir.join("seg3").join("000000000200.snapshot.json"),
        serde_json::to_string(&json!({"game_id":"g","hour_index":200})).unwrap(),
    )
    .unwrap();
    assert!(
        store
            .load_file("g", "seg3/000000000200.snapshot.json")
            .is_ok(),
        "a legitimate segment-prefixed (sub-dir) file must still load"
    );

    // the ATTACK: a `..` traversal must be REFUSED, never reach the sibling file.
    let escaped = store.load_file("g", "../secret.snapshot.json");
    assert!(
        escaped.is_err(),
        "a `..` traversal in the index `file` must be rejected, not read outside the game dir"
    );

    // an absolute path must also be refused.
    let abs = store.load_file("g", "/etc/passwd");
    assert!(abs.is_err(), "an absolute `file` path must be rejected");

    // a Windows drive-letter / backslash path must be refused.
    assert!(store.load_file("g", "C:/Windows/win.ini").is_err());
    assert!(store.load_file("g", "..\\secret.snapshot.json").is_err());
}

#[test]
fn dir_load_by_hour_rejects_negative_synthesized_name() {
    // Defensive: `Store::load(game, hour)` for the dir backend synthesizes a
    // `{:012}.snapshot.json` name. A negative hour would produce a `-` sign that
    // breaks the fixed width; the result must be a clean error (no panic, no
    // traversal), never a successful read of an unrelated file.
    let tmp = Tmp::new("neg");
    let gdir = tmp.path.join("g");
    fs::create_dir_all(&gdir).unwrap();
    let store = Store::open(&tmp.path);
    let r = store.load("g", -1);
    assert!(
        r.is_err(),
        "a missing/negative hour must error, never panic or traverse"
    );
}
