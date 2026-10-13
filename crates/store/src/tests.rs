use super::*;
use std::{fs::FileTimes, time::Duration};

struct Fixture {
    root: PathBuf,
    store: Store,
    recording: PathBuf,
    content: Vec<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("hoi4-prepared-test-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let recording = root.join("recording");
        let content = vec![root.join("base-content"), root.join("mod-archive")];
        for path in std::iter::once(&recording).chain(&content) {
            fs::write(path, b"synthetic").unwrap();
        }
        Self {
            store: Store {
                directory: root.join("prepared"),
            },
            root,
            recording,
            content,
        }
    }

    fn key(&self) -> SourceKey {
        SourceKey::capture(&self.recording, &self.content).unwrap()
    }

    fn save(&self, bytes: &[u8]) -> SourceKey {
        let key = self.key();
        let mut writer = self.store.replace(key.clone()).unwrap();
        assert_eq!(writer.write("countries", bytes).unwrap(), 0);
        writer.commit().unwrap();
        key
    }

    fn range_path(&self, section: usize, range: u64) -> PathBuf {
        let id = self
            .store
            .catalog()
            .unwrap()
            .unwrap()
            .id(section, range)
            .unwrap()
            .unwrap();
        self.store.directory.join(chunk_name(section, range, id))
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(&self.store.directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert_eq!(self.root.parent(), Some(std::env::temp_dir().as_path()));
        assert!(
            self.root
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("hoi4-prepared-test-")
        );
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn bytes(mut file: File) -> Vec<u8> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn write_replace_and_read_multiple_sections() {
    let fixture = Fixture::new();
    let key = fixture.key();
    assert!(!fixture.store.is_reusable(&key).unwrap());
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("countries", &b"old"[..]).unwrap();
    writer.write("wars", &b"war zero"[..]).unwrap();
    writer.write("wars", &b"war one"[..]).unwrap();
    writer.commit().unwrap();
    assert!(fixture.store.is_reusable(&key).unwrap());
    assert_eq!(
        bytes(fixture.store.read(&key, "wars", 1).unwrap().unwrap()),
        b"war one"
    );
    assert!(fixture.store.read(&key, "wars", 2).unwrap().is_none());
    assert!(fixture.store.read(&key, "absent", 0).unwrap().is_none());
    fixture.save(b"replacement");
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 0).unwrap().unwrap()),
        b"replacement"
    );
    assert!(fixture.store.read(&key, "wars", 0).unwrap().is_none());
    assert!(!fixture.names().iter().any(|name| name.ends_with(".tmp")));
}

#[test]
fn held_reader_keeps_old_file_through_replacement_and_cleanup() {
    let fixture = Fixture::new();
    let key = fixture.save(b"old complete bytes");
    let old_path = fixture.range_path(0, 0);
    let mut held = fixture.store.read(&key, "countries", 0).unwrap().unwrap();
    let mut prefix = [0; 4];
    held.read_exact(&mut prefix).unwrap();
    fixture.save(b"new complete bytes");
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 0).unwrap().unwrap()),
        b"new complete bytes"
    );
    // Next writer open deletes the old name even though a reader still has it.
    drop(fixture.store.replace(key).unwrap());
    assert!(!old_path.exists());
    assert_eq!(
        [prefix.as_slice(), bytes(held).as_slice()].concat(),
        b"old complete bytes"
    );
}

#[test]
fn clear_removes_names_while_held_reader_finishes_and_new_writes_work() {
    let fixture = Fixture::new();
    let key = fixture.save(b"held through clear");
    let held = fixture.store.read(&key, "countries", 0).unwrap().unwrap();
    fixture.store.clear().unwrap();
    assert_eq!(fixture.names(), [LOCK]);
    assert!(!fixture.store.is_reusable(&key).unwrap());
    assert!(fixture.store.read(&key, "countries", 0).unwrap().is_none());
    fixture.save(b"after clear");
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 0).unwrap().unwrap()),
        b"after clear"
    );
    assert_eq!(bytes(held), b"held through clear");
}

#[test]
fn crash_between_data_and_manifest_recovers_previous_complete_state() {
    let fixture = Fixture::new();
    let key = fixture.save(b"committed old");
    let old_path = fixture.range_path(0, 0);
    let manifest_before = fs::read(fixture.store.directory.join(MANIFEST)).unwrap();
    let mut interrupted = fixture.store.replace(key.clone()).unwrap();
    interrupted
        .write("countries", &b"uncommitted new"[..])
        .unwrap();
    interrupted.write("wars", &b"uncommitted wars"[..]).unwrap();
    // Simulate termination with durable data files and a partially written temp.
    fs::write(fixture.store.directory.join("manifest.tmp"), b"partial").unwrap();
    drop(interrupted);
    assert_eq!(
        fs::read(fixture.store.directory.join(MANIFEST)).unwrap(),
        manifest_before
    );
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 0).unwrap().unwrap()),
        b"committed old"
    );
    assert!(fixture.store.read(&key, "wars", 0).unwrap().is_none());
    let recovered = fixture.store.replace(key).unwrap();
    assert_eq!(fixture.names().len(), 3); // manifest, old range, lock
    assert!(old_path.exists());
    drop(recovered);
}

#[test]
fn source_size_or_modified_time_alone_prevents_reuse() {
    for content in [false, true] {
        for change_size in [false, true] {
            let fixture = Fixture::new();
            let old = fixture.save(b"prepared");
            let path = if content {
                &fixture.content[1]
            } else {
                &fixture.recording
            };
            let before = fs::metadata(path).unwrap();
            let modified = before.modified().unwrap();
            if change_size {
                fs::write(path, b"synthetic extended").unwrap();
            }
            let after_time = if change_size {
                modified
            } else {
                modified + Duration::from_secs(2)
            };
            OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(FileTimes::new().set_modified(after_time))
                .unwrap();
            let now = fixture.key();
            assert_ne!(now, old);
            let after = fs::metadata(path).unwrap();
            assert_eq!(after.len() == before.len(), !change_size);
            assert_eq!(after.modified().unwrap() == modified, change_size);
            assert!(!fixture.store.is_reusable(&now).unwrap());
            assert!(!fixture.store.is_reusable(&old).unwrap());
            assert!(fixture.store.read(&now, "countries", 0).unwrap().is_none());
            assert!(fixture.store.read(&old, "countries", 0).unwrap().is_none());
        }
    }
}

#[test]
fn append_adds_ranges_without_rewriting_old_files() {
    let fixture = Fixture::new();
    let old = fixture.save(b"first range");
    let first = fixture.range_path(0, 0);
    let first_bytes = fs::read(&first).unwrap();
    let first_time = fs::metadata(&first).unwrap().modified().unwrap();
    let mut held = fixture.store.read(&old, "countries", 0).unwrap().unwrap();
    fs::write(&fixture.recording, b"synthetic recording appended").unwrap();
    let new = fixture.key();
    assert!(!fixture.store.is_reusable(&new).unwrap());
    let mut writer = fixture.store.append(&old, new.clone()).unwrap();
    assert_eq!(writer.write("countries", &b"second range"[..]).unwrap(), 1);
    assert_eq!(writer.write("new section", &b"only range"[..]).unwrap(), 0);
    writer.commit().unwrap();
    assert!(fixture.store.is_reusable(&new).unwrap());
    assert!(!fixture.store.is_reusable(&old).unwrap());
    assert_eq!(fixture.range_path(0, 0), first);
    assert_eq!(fs::read(&first).unwrap(), first_bytes);
    assert_eq!(
        fs::metadata(&first).unwrap().modified().unwrap(),
        first_time
    );
    assert_eq!(
        bytes(fixture.store.read(&new, "countries", 1).unwrap().unwrap()),
        b"second range"
    );
    assert_eq!(
        bytes(fixture.store.read(&new, "new section", 0).unwrap().unwrap()),
        b"only range"
    );
    held.rewind().unwrap();
    assert_eq!(bytes(held), first_bytes);
}

#[test]
fn writer_is_exclusive_and_clear_cannot_split_the_lock() {
    let fixture = Fixture::new();
    let key = fixture.save(b"prepared");
    let held = fixture.store.replace(key.clone()).unwrap();
    assert!(fixture.store.replace(key.clone()).is_err());
    assert!(fixture.store.clear().is_err());
    assert!(fixture.store.is_reusable(&key).unwrap());
    drop(held);
    fixture.store.clear().unwrap();
    let held = fixture.store.replace(key.clone()).unwrap();
    assert!(fixture.store.replace(key).is_err());
    drop(held);
}

#[test]
fn append_refuses_unchanged_recording_changed_content_or_wrong_prefix() {
    let fixture = Fixture::new();
    let old = fixture.save(b"prepared");
    assert!(fixture.store.append(&old, old.clone()).is_err());
    fs::write(&fixture.recording, b"synthetic recording appended").unwrap();
    let grown = fixture.key();
    let mut wrong_prefix = old.clone();
    wrong_prefix.recording.size -= 1;
    assert!(fixture.store.append(&wrong_prefix, grown).is_err());
    fs::write(&fixture.content[1], b"changed mod archive").unwrap();
    assert!(fixture.store.append(&old, fixture.key()).is_err());
    assert_eq!(fixture.names().len(), 3);
}

#[test]
fn commit_refuses_sources_changed_during_preparation() {
    let fixture = Fixture::new();
    let key = fixture.save(b"committed");
    let manifest_before = fs::read(fixture.store.directory.join(MANIFEST)).unwrap();
    let mut writer = fixture.store.replace(key).unwrap();
    writer.write("countries", &b"new"[..]).unwrap();
    fs::write(&fixture.content[0], b"changed content").unwrap();
    assert!(writer.commit().is_err());
    assert_eq!(
        fs::read(fixture.store.directory.join(MANIFEST)).unwrap(),
        manifest_before
    );
}

#[test]
fn hundred_year_file_count_opens_only_the_manifest_header_and_one_entry() {
    let fixture = Fixture::new();
    let key = fixture.save(b"placeholder");
    // One hourly range per section over 100 years. References are synthetic;
    // all but the final range are deliberately absent: opening cannot touch them.
    let count = 100 * 366 * 24;
    let id = Uuid::new_v4();
    let header = Header {
        version: VERSION,
        key: key.clone(),
        sections: vec![
            Section {
                name: "countries".into(),
                count,
            },
            Section {
                name: "wars".into(),
                count,
            },
        ],
    };
    publish(&fixture.store.directory.join(MANIFEST), |file| {
        let header = serde_json::to_vec(&header)?;
        file.write_all(&(header.len() as u64).to_le_bytes())?;
        file.write_all(&header)?;
        // Seeking over the flat list supplies the logical count without millions of writes.
        let offset = 8 + header.len() as u64 + (2 * count - 1) * ID_BYTES;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(id.as_bytes())
    })
    .unwrap();
    let path = fixture.store.directory.join(chunk_name(1, count - 1, id));
    publish(&path, |file| file.write_all(b"last century range")).unwrap();
    assert!(fixture.store.is_reusable(&key).unwrap());
    let mut opened = fixture.store.catalog().unwrap().unwrap();
    assert_eq!(opened.offsets.len(), 2);
    assert_eq!(opened.file.stream_position().unwrap(), opened.offsets[0]);
    assert_eq!(opened.id(1, count - 1).unwrap(), Some(id));
    assert_eq!(
        opened.file.stream_position().unwrap(),
        opened.file.metadata().unwrap().len()
    );
    assert_eq!(
        bytes(
            fixture
                .store
                .read(&key, "wars", count - 1)
                .unwrap()
                .unwrap()
        ),
        b"last century range"
    );
    assert_eq!(
        fixture
            .names()
            .iter()
            .filter(|name| name.ends_with(".bin"))
            .count(),
        2
    );
}

#[test]
fn malformed_manifest_is_refused_without_cleanup() {
    let fixture = Fixture::new();
    let key = fixture.save(b"prepared");
    let path = fixture.range_path(0, 0);
    fs::write(
        fixture.store.directory.join(MANIFEST),
        u64::MAX.to_le_bytes(),
    )
    .unwrap();
    assert!(fixture.store.is_reusable(&key).is_err());
    assert!(fixture.store.replace(key).is_err());
    assert!(path.exists());
    // Clear can discard an invalid store without parsing its manifest.
    fixture.store.clear().unwrap();
    assert_eq!(fixture.names(), [LOCK]);
}
