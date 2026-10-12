//! Disposable app-owned eligibility hints, keyed by normalized recording path.
use replay::container::{CompactionOutcome, CompactionResult, ReplayRevision};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
pub(super) struct Store {
    entries: BTreeMap<String, Entry>,
    pending: BTreeMap<String, Entry>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Entry {
    revision: ReplayRevision,
    outcome: CompactionOutcome,
}

pub(super) fn key(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let key = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let key = key.strip_prefix("//?/").unwrap_or(&key).to_lowercase();
    key
}
pub(super) fn path() -> PathBuf {
    replay::recorder_settings::directory()
        .parent()
        .unwrap()
        .join("compaction.json")
}
impl Store {
    pub(super) fn load_from(path: &Path) -> Self {
        Self {
            entries: fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default(),
            pending: BTreeMap::new(),
        }
    }

    pub(super) fn eligible(&self, path: &Path, revision: &ReplayRevision) -> bool {
        self.entries.get(&key(path)).is_none_or(|entry| {
            entry.revision != *revision
                || !matches!(
                    entry.outcome,
                    CompactionOutcome::Compacted | CompactionOutcome::AlreadySmall
                )
        })
    }
    pub(super) fn remember(&mut self, path: &Path, result: &CompactionResult) {
        if matches!(
            result.outcome,
            CompactionOutcome::Compacted | CompactionOutcome::AlreadySmall
        ) {
            let entry = Entry {
                revision: result.revision.clone(),
                outcome: result.outcome,
            };
            self.entries.insert(key(path), entry.clone());
            self.pending.insert(key(path), entry);
        }
    }
    pub(super) fn save_to(&mut self, path: &Path) -> io::Result<()> {
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("Missing app data folder"))?;
        fs::create_dir_all(directory)?;
        // An OS file lock serializes the short read/merge/replace across instances.
        // Merge only this store's pending updates, never stale cached entries.
        let lock_path = path.with_extension("json.lock");
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock()?;
        let mut current: BTreeMap<String, Entry> = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            // A transient read/sharing failure must not erase another writer's
            // valid entries. Retain our pending updates for the next attempt.
            Err(error) => return Err(error),
        };
        current.extend(
            self.pending
                .iter()
                .map(|(key, entry)| (key.clone(), entry.clone())),
        );
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        let temporary = path.with_extension(format!(
            "json.{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let _cleanup = Cleanup(temporary.clone());
        serde_json::to_writer(&file, &current)?;
        file.sync_all()?;
        drop(file);
        super::platform::replace_store(&temporary, path)?;
        self.entries = current;
        self.pending.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::container::replay_revision;
    #[test]
    fn two_stale_stores_merge_different_paths_and_never_expose_partial_json() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-compaction-stores-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let store_path = root.join("compaction.json");
        let first_path = root.join("first.battleplan");
        let second_path = root.join("second.battleplan");
        fs::write(&first_path, b"first synthetic revision").unwrap();
        fs::write(&second_path, b"second synthetic revision").unwrap();
        let first_revision = replay_revision(&first_path).unwrap();
        let second_revision = replay_revision(&second_path).unwrap();
        let result = |revision| CompactionResult {
            bytes_before: 100,
            bytes_after: 90,
            blocks_encoded: 1,
            outcome: CompactionOutcome::Compacted,
            revision,
        };
        let mut first = Store::load_from(&store_path);
        let mut second = Store::load_from(&store_path);
        first.remember(&first_path, &result(first_revision.clone()));
        second.remember(&second_path, &result(second_revision.clone()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let spawn = |mut store: Store| {
            let path = store_path.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..20 {
                    store.save_to(&path).unwrap();
                }
            })
        };
        let a = spawn(first);
        let b = spawn(second);
        barrier.wait();
        while !a.is_finished() || !b.is_finished() {
            if let Ok(bytes) = fs::read(&store_path) {
                serde_json::from_slice::<BTreeMap<String, Entry>>(&bytes)
                    .expect("atomic writes always expose complete JSON");
            }
            std::thread::yield_now();
        }
        a.join().unwrap();
        b.join().unwrap();
        let stored = Store::load_from(&store_path);
        assert!(!stored.eligible(&first_path, &first_revision));
        assert!(!stored.eligible(&second_path, &second_revision));
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "tmp")
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn eligibility_tracks_the_left_revision_and_only_completed_outcomes() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-compaction-store-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let recording = root.join("synthetic.battleplan");
        let store_path = root.join("compaction.json");
        fs::write(&recording, b"synthetic revision").unwrap();
        let revision = replay_revision(&recording).unwrap();
        let mut store = Store::load_from(&store_path);
        assert!(store.eligible(&recording, &revision));
        for outcome in [
            CompactionOutcome::Compacted,
            CompactionOutcome::AlreadySmall,
        ] {
            store.remember(
                &recording,
                &CompactionResult {
                    bytes_before: 100,
                    bytes_after: 90,
                    blocks_encoded: 1,
                    outcome,
                    revision: revision.clone(),
                },
            );
            store.save_to(&store_path).unwrap();
            store = Store::load_from(&store_path);
            assert!(!store.eligible(&recording, &revision));
            #[cfg(windows)]
            assert!(!store.eligible(
                &PathBuf::from(recording.to_string_lossy().to_uppercase()),
                &revision
            ));
        }
        // A timestamp-only change also invalidates the hint, even at the same length.
        fs::OpenOptions::new()
            .write(true)
            .open(&recording)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(
                    std::time::SystemTime::now() + std::time::Duration::from_secs(60),
                ),
            )
            .unwrap();
        let retimed = replay_revision(&recording).unwrap();
        assert_ne!(retimed, revision);
        assert!(store.eligible(&recording, &retimed));
        fs::write(&recording, b"appended synthetic revision").unwrap();
        let appended = replay_revision(&recording).unwrap();
        assert!(store.eligible(&recording, &appended));
        store.remember(
            &recording,
            &CompactionResult {
                bytes_before: 100,
                bytes_after: 100,
                blocks_encoded: 1,
                outcome: CompactionOutcome::Changed,
                revision: appended.clone(),
            },
        );
        assert!(store.eligible(&recording, &appended));
        fs::write(&store_path, b"unreadable json").unwrap();
        assert!(Store::load_from(&store_path).eligible(&recording, &revision));
        assert!(
            Store::load_from(&root).eligible(&recording, &revision),
            "an unreadable store is also disposable"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
