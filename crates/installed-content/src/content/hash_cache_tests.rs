use super::*;

struct Fixture {
    root: PathBuf,
    identity: Identity,
    options: ResolveOptions,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hoi4_hash_cache_{name}_{}_{:?}_{}",
            std::process::id(),
            std::thread::current().id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game_dir = root.join("game");
        let user_dir = root.join("user");
        let cache_dir = root.join("cache");
        fs::create_dir_all(game_dir.join("common")).unwrap();
        fs::create_dir_all(game_dir.join("dlc")).unwrap();
        fs::create_dir_all(&user_dir).unwrap();
        let executable = b"synthetic executable";
        fs::write(game_dir.join("hoi4.exe"), executable).unwrap();
        fs::write(game_dir.join("common/a.txt"), b"original").unwrap();
        fs::write(game_dir.join("common/z.txt"), b"last").unwrap();
        fs::write(user_dir.join("settings.txt"), b"language=l_english").unwrap();
        fs::write(
            user_dir.join("dlc_load.json"),
            br#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        )
        .unwrap();
        let gathered = gather(&GatherOptions {
            game_dir: game_dir.clone(),
            user_dir: user_dir.clone(),
            cache_dir: cache_dir.clone(),
            dlc_order: None,
        })
        .unwrap();
        let identity = gathered
            .identity(
                "synthetic build".into(),
                Sha256::digest(executable).into(),
                executable.len() as u64,
                "test".into(),
                [0; 32],
            )
            .unwrap();
        let options = ResolveOptions {
            game_dir,
            user_dir,
            cache_dir,
        };
        resolve(&identity, &options).unwrap();
        Self {
            root,
            identity,
            options,
        }
    }

    fn cache_path(&self) -> PathBuf {
        self.options.cache_dir.join("content-hashes-v1.json")
    }

    fn cache(&self) -> HashCache {
        serde_json::from_slice(&fs::read(self.cache_path()).unwrap()).unwrap()
    }

    fn mark_cache_bytes(&self) -> Vec<u8> {
        let mut bytes = fs::read(self.cache_path()).unwrap();
        bytes.extend_from_slice(b" \r\n\t");
        fs::write(self.cache_path(), &bytes).unwrap();
        bytes
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn unchanged_warm_hash_cache_preserves_bytes_and_verification() {
    let fixture = Fixture::new("unchanged");
    let expected = fixture.mark_cache_bytes();
    let resolved = resolve(&fixture.identity, &fixture.options).unwrap();
    assert_eq!(fs::read(fixture.cache_path()).unwrap(), expected);
    assert_eq!(resolved.stats.files, 3);
    assert_eq!(resolved.stats.files_read, 0);
    assert_eq!(resolved.stats.bytes_read, 0);
    assert_eq!(resolved.stats.cache_hits, resolved.stats.files);
    assert_eq!(resolved.layers.read("common/a.txt").unwrap(), b"original");

    let mut mismatching = fixture.identity.clone();
    mismatching.vanilla_hash[0] ^= 1;
    let error = resolve(&mismatching, &fixture.options).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Game vanilla: content hash mismatch")
    );
    assert_eq!(fs::read(fixture.cache_path()).unwrap(), expected);
}

#[test]
fn missing_or_malformed_hash_cache_is_rebuilt_without_changing_schema() {
    for missing in [true, false] {
        let fixture = Fixture::new(if missing { "missing" } else { "malformed" });
        let expected = fixture.cache();
        if missing {
            fs::remove_file(fixture.cache_path()).unwrap();
        } else {
            fs::write(fixture.cache_path(), b"incomplete hash cache {").unwrap();
        }
        let resolved = resolve(&fixture.identity, &fixture.options).unwrap();
        assert_eq!(resolved.stats.files_read, resolved.stats.files);
        assert_eq!(resolved.stats.files, 3);
        assert_eq!(resolved.stats.cache_hits, 0);
        let bytes = fs::read(fixture.cache_path()).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["entries"]
        );
        let actual: HashCache = serde_json::from_slice(&bytes).unwrap();
        assert!(!actual.dirty);
        assert_eq!(actual.entries.len(), expected.entries.len());
        for (key, entry) in expected.entries {
            assert_eq!(actual.get(&key, &entry.stamp), Some(entry.hash));
        }
        let marked = fixture.mark_cache_bytes();
        let warm = resolve(&fixture.identity, &fixture.options).unwrap();
        assert_eq!(warm.stats.files_read, 0);
        assert_eq!(fs::read(fixture.cache_path()).unwrap(), marked);
    }
}

#[test]
fn changed_hash_entries_persist_even_when_identity_verification_fails() {
    let fixture = Fixture::new("changed");
    let previous = fixture.mark_cache_bytes();
    let file = fixture.options.game_dir.join("common/a.txt");
    let replacement = b"changed content with a different size";
    fs::write(&file, replacement).unwrap();
    let error = resolve(&fixture.identity, &fixture.options).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Game vanilla: content hash mismatch")
    );
    let actual = fixture.cache();
    let canonical = fs::canonicalize(file).unwrap();
    assert_eq!(
        actual.get(
            text_path(&canonical).unwrap(),
            &Stamp::read(&canonical).unwrap()
        ),
        Some(Sha256::digest(replacement).into())
    );
    assert_ne!(fs::read(fixture.cache_path()).unwrap(), previous);

    let marked = fixture.mark_cache_bytes();
    let repeated = resolve(&fixture.identity, &fixture.options).unwrap_err();
    assert_eq!(repeated.to_string(), error.to_string());
    assert_eq!(fs::read(fixture.cache_path()).unwrap(), marked);
}

#[test]
fn successful_hash_prefix_persists_before_later_layer_resolution_fails() {
    let fixture = Fixture::new("later_layer");
    let expected = fixture.cache();
    fs::remove_file(fixture.cache_path()).unwrap();
    let mut missing_layer = fixture.identity.clone();
    missing_layer.dlc.push(Dlc {
        id: "missing".into(),
        hash: [0; 32],
        declared_checksum: None,
    });
    let error = resolve(&missing_layer, &fixture.options).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(error.to_string().contains("DLC missing"));
    let actual = fixture.cache();
    for (key, entry) in expected.entries {
        assert_eq!(actual.get(&key, &entry.stamp), Some(entry.hash));
    }
    let marked = fixture.mark_cache_bytes();
    let warm = resolve(&fixture.identity, &fixture.options).unwrap();
    assert_eq!(warm.stats.files_read, 0);
    assert_eq!(warm.stats.cache_hits, warm.stats.files);
    assert_eq!(fs::read(fixture.cache_path()).unwrap(), marked);
}

#[test]
fn missing_or_malformed_hash_cache_is_rebuilt_before_first_hash_failure() {
    for missing in [true, false] {
        let fixture = Fixture::new(if missing {
            "early_missing"
        } else {
            "early_malformed"
        });
        if missing {
            fs::remove_file(fixture.cache_path()).unwrap();
        } else {
            fs::write(fixture.cache_path(), b"incomplete hash cache {").unwrap();
        }
        let mut wrong_size = fixture.identity.clone();
        wrong_size.exe_size += 1;
        let error = resolve(&wrong_size, &fixture.options).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Game hoi4.exe: byte size mismatch")
        );
        let actual = fixture.cache();
        assert!(actual.entries.is_empty());
        assert!(!actual.dirty);
    }
}

#[test]
fn dirty_hash_cache_write_errors_remain_resolution_failures() {
    let fixture = Fixture::new("write_failure");
    fs::remove_file(fixture.cache_path()).unwrap();
    fs::create_dir(fixture.cache_path()).unwrap();
    let expected = fs::write(fixture.cache_path(), b"probe").unwrap_err();
    let error = resolve(&fixture.identity, &fixture.options).unwrap_err();
    assert_eq!(error.kind(), expected.kind());
    assert!(fixture.cache_path().is_dir());
}
