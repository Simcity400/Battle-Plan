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
    let secret = tmp.path.join("secret.snapshot.json");
    fs::write(
        &secret,
        serde_json::to_string(&json!({"owner":"LEAKED"})).unwrap(),
    )
    .unwrap();

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

    let canonical = store.load_file("g", "000000000100.snapshot.json");
    assert!(
        canonical.is_ok(),
        "canonical handle-relative load failed: {canonical:?}"
    );
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

    let escaped = store.load_file("g", "../secret.snapshot.json");
    assert!(
        escaped.is_err(),
        "a `..` traversal in the index `file` must be rejected, not read outside the game dir"
    );

    let abs = store.load_file("g", "/etc/passwd");
    assert!(abs.is_err(), "an absolute `file` path must be rejected");

    assert!(store.load_file("g", "C:/Windows/win.ini").is_err());
    assert!(store.load_file("g", "..\\secret.snapshot.json").is_err());
}

#[test]
fn dir_load_by_hour_rejects_negative_synthesized_name() {
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

#[test]
fn directory_snapshot_duplicate_members_are_rejected_recursively() {
    for (case, snapshot_bytes, duplicate_key) in [
        (
            "duplicate_session",
            br#"{"game_id":"g","hour_index":1,"session":{"active_dlcs":["Wrong DLC"]},"session":{}}"#.as_slice(),
            "session",
        ),
        (
            "duplicate_nested_active_dlcs",
            br#"{"game_id":"g","hour_index":1,"session":{"active_dlcs":["Wrong DLC"],"active_dlcs":[]}}"#.as_slice(),
            "active_dlcs",
        ),
    ] {
        let tmp = Tmp::new(case);
        let game = tmp.path.join("g");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("index.json"),
            br#"{"game_id":"g","snapshots":[{"hour_index":1,"file":"000000000001.snapshot.json"}]}"#,
        )
        .unwrap();
        fs::write(game.join("000000000001.snapshot.json"), snapshot_bytes).unwrap();
        let store = Store::open_directory(&tmp.path).unwrap();

        let error = store
            .load_file("g", "000000000001.snapshot.json")
            .expect_err("duplicate snapshot members must be rejected")
            .to_string();
        assert!(error.contains("duplicate JSON member"), "{case}: {error}");
        assert!(error.contains(duplicate_key), "{case}: {error}");
    }
}
