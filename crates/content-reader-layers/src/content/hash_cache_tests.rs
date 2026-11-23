use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) static PROBED: std::sync::Mutex<Vec<(PathBuf, std::thread::ThreadId)>> =
    std::sync::Mutex::new(Vec::new());

pub(super) static STAMPED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

struct Fixture {
    root: PathBuf,
    identity: Identity,
    options: ResolveOptions,
}

impl Fixture {
    fn new(name: &str) -> Self {
        Self::with_files(name, 0)
    }

    fn with_files(name: &str, extra: usize) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hoi4_hash_cache_{name}_{}_{:?}_{}_{}",
            std::process::id(),
            std::thread::current().id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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
        for index in 0..extra {
            fs::write(
                game_dir.join(format!("common/f{index:04}.txt")),
                format!("file {index}"),
            )
            .unwrap();
        }
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
        self.options.cache_dir.join(HASH_CACHE)
    }

    fn cache(&self) -> HashCache<'_> {
        HashCache::decode(&fs::read(self.cache_path()).unwrap()).unwrap()
    }

    fn mark_cache(&self) -> Marked {
        let time = UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(self.cache_path())
            .unwrap()
            .set_modified(time)
            .unwrap();
        Marked {
            bytes: fs::read(self.cache_path()).unwrap(),
            time,
        }
    }

    fn assert_unwritten(&self, marked: &Marked) {
        assert_eq!(fs::read(self.cache_path()).unwrap(), marked.bytes);
        assert_eq!(
            fs::metadata(self.cache_path()).unwrap().modified().unwrap(),
            marked.time
        );
    }
}

struct Marked {
    bytes: Vec<u8>,
    time: std::time::SystemTime,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_stopped_gathering_hashes_no_more_files() {
    let fixture = Fixture::with_files("stopped", 16);
    let options = GatherOptions {
        game_dir: fixture.options.game_dir.clone(),
        user_dir: fixture.options.user_dir.clone(),
        cache_dir: fixture.root.join("cold cache"),
        dlc_order: None,
    };
    let game_dir = fs::canonicalize(&options.game_dir).unwrap();
    let probed = || {
        PROBED
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, _)| path.starts_with(&game_dir))
            .count()
    };
    let before = probed();
    let stopped = Arc::new(AtomicBool::new(true));
    let error = gather_cancellable(&options, stopped).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(probed(), before, "a stopped gathering probes no file");
    let going = gather_cancellable(&options, Arc::new(AtomicBool::new(false))).unwrap();
    assert_eq!(going.stats.files_read, going.stats.files);
    let warm = gather(&options).unwrap();
    assert_eq!(warm.stats.cache_hits, warm.stats.files);
}

#[test]
fn unchanged_warm_hash_cache_preserves_bytes_and_verification() {
    let fixture = Fixture::new("unchanged");
    let expected = fixture.mark_cache();
    let resolved = resolve(&fixture.identity, &fixture.options).unwrap();
    fixture.assert_unwritten(&expected);
    assert_eq!(resolved.stats.files, 3);
    assert_eq!(resolved.stats.files_read, 0);
    assert_eq!(resolved.stats.bytes_read, 0);
    assert_eq!(resolved.stats.cache_hits, resolved.stats.files);
    assert_eq!(resolved.layers.read("common/a.txt").unwrap(), b"original");

    let mut mismatching = fixture.identity.clone();
    mismatching.vanilla_hash[0] ^= 1;
    let error = resolve(&mismatching, &fixture.options)
        .unwrap()
        .exact()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Game vanilla: content hash mismatch")
    );
    fixture.assert_unwritten(&expected);
}

#[test]
fn cold_content_hashes_are_read_on_several_threads() {
    let fixture = Fixture::with_files("parallel", 256);
    let expected = fixture.cache();
    fs::remove_file(fixture.cache_path()).unwrap();
    let before = PROBED.lock().unwrap().len();
    let resolved = resolve(&fixture.identity, &fixture.options).unwrap();
    assert_eq!(resolved.stats.files, 259);
    assert_eq!(resolved.stats.files_read, resolved.stats.files);
    assert_eq!(fixture.cache().entries.len(), expected.entries.len());
    let folder = fixture
        .root
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let probed: Vec<_> = PROBED.lock().unwrap()[before..]
        .iter()
        .filter(|(path, _)| path.to_string_lossy().contains(&folder))
        .map(|(_, thread)| *thread)
        .collect();
    assert_eq!(probed.len(), 259);
    let threads: std::collections::HashSet<_> = probed.into_iter().collect();
    let cores = std::thread::available_parallelism().map_or(1, |count| count.get());
    assert!(
        threads.len() > 1 || cores == 1,
        "{} probing threads on {cores} cores",
        threads.len()
    );
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
        assert!(bytes.starts_with(HASH_CACHE_MAGIC));
        let actual = HashCache::decode(&bytes).unwrap();
        assert_eq!(actual.encode(), bytes, "written as it decodes");
        assert!(!actual.dirty);
        assert_eq!(actual.entries.len(), expected.entries.len());
        for (key, entry) in expected.entries {
            assert_eq!(actual.get(&key, &entry.stamp), Some(entry.hash));
        }
        let marked = fixture.mark_cache();
        let warm = resolve(&fixture.identity, &fixture.options).unwrap();
        assert_eq!(warm.stats.files_read, 0);
        fixture.assert_unwritten(&marked);
    }
}

#[test]
fn changed_hash_entries_persist_even_when_identity_verification_fails() {
    let fixture = Fixture::new("changed");
    let previous = fixture.mark_cache();
    let file = fixture.options.game_dir.join("common/a.txt");
    let replacement = b"changed content with a different size";
    fs::write(&file, replacement).unwrap();
    let error = resolve(&fixture.identity, &fixture.options)
        .unwrap()
        .exact()
        .unwrap_err();
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
    assert_ne!(fs::read(fixture.cache_path()).unwrap(), previous.bytes);

    let marked = fixture.mark_cache();
    let repeated = resolve(&fixture.identity, &fixture.options)
        .unwrap()
        .exact()
        .unwrap_err();
    assert_eq!(repeated.to_string(), error.to_string());
    fixture.assert_unwritten(&marked);
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
    let marked = fixture.mark_cache();
    let warm = resolve(&fixture.identity, &fixture.options).unwrap();
    assert_eq!(warm.stats.files_read, 0);
    assert_eq!(warm.stats.cache_hits, warm.stats.files);
    fixture.assert_unwritten(&marked);
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
        fs::remove_file(fixture.options.game_dir.join("hoi4.exe")).unwrap();
        let error = resolve(&fixture.identity, &fixture.options).unwrap_err();
        assert!(error.to_string().starts_with("Game hoi4.exe: "), "{error}");
        let actual = fixture.cache();
        assert!(actual.entries.is_empty());
        assert!(!actual.dirty);
    }
}

#[test]
fn dirty_hash_cache_write_errors_leave_resolution_intact() {
    let fixture = Fixture::new("write_failure");
    fs::remove_file(fixture.cache_path()).unwrap();
    fs::create_dir(fixture.cache_path()).unwrap();
    let resolved = resolve(&fixture.identity, &fixture.options).unwrap();
    assert!(resolved.stats.cache_unsaved.is_some());
    assert!(fixture.cache_path().is_dir());
    fs::remove_file(fixture.options.game_dir.join("hoi4.exe")).unwrap();
    let error = resolve(&fixture.identity, &fixture.options).unwrap_err();
    assert!(error.to_string().starts_with("Game hoi4.exe: "), "{error}");
}

#[test]
fn warm_resolution_stamps_listed_files_from_the_walk() {
    let fixture = Fixture::with_files("walk_stamps", 20);
    let listed = |path: &PathBuf| {
        let text = path.to_string_lossy();
        text.contains("hoi4_hash_cache_walk_stamps") && text.contains("common")
    };
    STAMPED.lock().expect("stamped paths").retain(|path| !listed(path));
    let resolved = resolve(&fixture.identity, &fixture.options).unwrap();
    assert_eq!(resolved.stats.cache_hits, resolved.stats.files);
    let stamped: Vec<_> = STAMPED
        .lock()
        .expect("stamped paths")
        .iter()
        .filter(|path| listed(path))
        .cloned()
        .collect();
    assert!(stamped.is_empty(), "stamped again: {stamped:?}");
    let mut members = 0;
    for layer in &resolved.layers.layers {
        let LayerSource::Folder(root) = &layer.source else {
            continue;
        };
        for (relative, member) in layer.files.iter() {
            let path = root.join(std::str::from_utf8(relative).unwrap());
            let metadata = fs::metadata(&path).unwrap();
            assert_eq!(
                member.stamp.as_ref(),
                Some(&Stamp::of(&metadata).unwrap()),
                "{}",
                path.display()
            );
            members += 1;
        }
    }
    assert_eq!(members, 22);
}

#[test]
fn hash_cache_files_decode_only_whole() {
    let fixture = Fixture::with_files("whole", 3);
    let bytes = fs::read(fixture.cache_path()).unwrap();
    let cache = HashCache::decode(&bytes).unwrap();
    assert_eq!(cache.entries.len(), 6);
    assert_eq!(cache.encode(), bytes);
    assert!(HashCache::decode(&bytes[..bytes.len() - 1]).is_none());
    let mut extended = bytes.clone();
    extended.push(0);
    assert!(HashCache::decode(&extended).is_none());
    assert!(HashCache::decode(b"{\"entries\":{}}").is_none());
}
