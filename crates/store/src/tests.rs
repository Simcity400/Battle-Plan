use super::*;
use std::{fs::FileTimes, time::Duration};

thread_local! {
    pub(super) static VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static MANIFEST_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static INPUT_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static KEY_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[test]
fn bound_pages_do_not_check_content_or_reread_stamps_and_commits_reuse_the_stamp_file() {
    let fixture = Fixture::new();
    let inputs = vec![fixture.content[0].clone(); 64];
    let key = SourceKey::capture(&fixture.recording, &inputs).unwrap();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("pages", &b"page"[..]).unwrap();
    writer.commit().unwrap();
    assert!(fixture.store.is_reusable(&key).unwrap());
    let source = fixture.store.catalog().unwrap().unwrap().header.source;
    let stamps = fixture.store.directory.join(source_name(source));
    let stamp_time = fs::metadata(&stamps).unwrap().modified().unwrap();
    INPUT_CHECKS.set(0);
    KEY_READS.set(0);
    for _ in 0..20 {
        assert_eq!(
            bytes(
                fixture
                    .store
                    .read(&key.clone(), "pages", 0)
                    .unwrap()
                    .unwrap()
            ),
            b"page"
        );
    }
    assert_eq!((INPUT_CHECKS.get(), KEY_READS.get()), (20, 0)); // recording only
    let mut writer = fixture.store.extend(&key).unwrap();
    writer.write_at("pages", 0, &b"updated"[..]).unwrap();
    writer.commit().unwrap();
    assert_eq!(
        fixture.store.catalog().unwrap().unwrap().header.source,
        source
    );
    assert_eq!(
        fs::metadata(stamps).unwrap().modified().unwrap(),
        stamp_time
    );
    assert!(
        fs::metadata(fixture.store.directory.join(MANIFEST))
            .unwrap()
            .len()
            < 256
    );
    // A new content binding refuses the published key; the prior binding and
    // held handles remain valid until a validated replacement is published.
    fs::write(&fixture.content[0], b"changed source").unwrap();
    let changed = SourceKey::capture(&fixture.recording, &inputs).unwrap();
    assert!(!fixture.store.is_reusable(&changed).unwrap());
    KEY_READS.set(0);
    for _ in 0..20 {
        assert!(fixture.store.read(&changed, "pages", 0).unwrap().is_none());
    }
    assert_eq!(KEY_READS.get(), 0);
    assert!(fixture.store.extend(&key).unwrap().commit().is_err());
    assert!(!fixture.store.is_reusable(&key).unwrap());
    assert!(fixture.store.read(&key, "pages", 0).unwrap().is_none());
}

#[test]
#[ignore = "40,000 physical files: content binding/reopen timing"]
fn forty_thousand_inputs_bind_once_then_pages_are_constant_cost() {
    let fixture = Fixture::new();
    let inputs: Vec<_> = (0..40_000)
        .map(|index| fixture.root.join(format!("input-{index}")))
        .collect();
    for path in &inputs {
        fs::write(path, b"synthetic content").unwrap();
    }
    let key = SourceKey::capture(&fixture.recording, &inputs).unwrap();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("pages", &b"page"[..]).unwrap();
    writer.commit().unwrap();
    // Reproduce the former query path's full stamp recapture + embedded JSON.
    let old_header = serde_json::to_vec(&key).unwrap();
    INPUT_CHECKS.set(0);
    let before = std::time::Instant::now();
    for _ in 0..9 {
        for stamp in std::iter::once(&key.recording).chain(key.content.iter()) {
            INPUT_CHECKS.set(INPUT_CHECKS.get() + 1);
            let path = std::path::absolute(&stamp.path).unwrap();
            let metadata = fs::metadata(path).unwrap();
            assert_eq!(
                (metadata.len(), metadata.modified().unwrap()),
                (stamp.size, stamp.modified)
            );
        }
        assert_eq!(
            serde_json::from_reader::<_, SourceKey>(io::BufReader::new(old_header.as_slice()))
                .unwrap(),
            key
        );
    }
    eprintln!(
        "old 9 reads: {:?}, input_checks={}, embedded_stamp_bytes={}",
        before.elapsed(),
        INPUT_CHECKS.get(),
        old_header.len()
    );
    INPUT_CHECKS.set(0);
    KEY_READS.set(0);
    let reopen = std::time::Instant::now();
    let saved = fixture.store.saved_key().unwrap().unwrap();
    let decode_time = reopen.elapsed();
    let reopened = saved.recapture().unwrap();
    assert!(fixture.store.is_reusable(&reopened).unwrap());
    eprintln!(
        "reopen+binding: {:?} (decode {:?}), input_checks={}, stamp_reads={}",
        reopen.elapsed(),
        decode_time,
        INPUT_CHECKS.get(),
        KEY_READS.get()
    );
    INPUT_CHECKS.set(0);
    KEY_READS.set(0);
    let pages = std::time::Instant::now();
    for _ in 0..100 {
        assert!(
            fixture
                .store
                .read(&reopened.clone(), "pages", 0)
                .unwrap()
                .is_some()
        );
    }
    eprintln!(
        "100 pages: {:?}, input_checks={}, stamp_reads={}, manifest_bytes={}",
        pages.elapsed(),
        INPUT_CHECKS.get(),
        KEY_READS.get(),
        fs::metadata(fixture.store.directory.join(MANIFEST))
            .unwrap()
            .len()
    );
    assert_eq!((INPUT_CHECKS.get(), KEY_READS.get()), (100, 0));
}

#[test]
fn reacquired_file_uses_its_current_section_count() {
    let fixture = Fixture::new();
    let key = fixture.save(b"old bytes");
    let mut old = fixture.store.catalog().unwrap().unwrap();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("countries", &b"new bytes"[..]).unwrap();
    writer.write("countries", &b"next range"[..]).unwrap();
    writer.commit().unwrap();
    drop(fixture.store.extend(&key).unwrap());
    assert_eq!(
        fixture
            .store
            .read_catalog(&mut old, "countries", 0)
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    let (count, file) = fixture
        .store
        .read_named_counted(&key, "countries", 0)
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(bytes(file.unwrap()), b"new bytes");
}

#[test]
fn series_metadata_and_first_files_use_the_bound_key() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("catalogue", &b"catalogue"[..]).unwrap();
    for index in 0..5 {
        writer.write("series", &[index][..]).unwrap();
    }
    writer.commit().unwrap();
    VALIDATIONS.set(0);
    MANIFEST_READS.set(0);
    assert_eq!(fixture.store.range_count(&key, "series").unwrap(), Some(5));
    assert!(fixture.store.read(&key, "catalogue", 0).unwrap().is_some());
    assert!(fixture.store.read(&key, "series", 0).unwrap().is_some());
    assert_eq!((VALIDATIONS.get(), MANIFEST_READS.get()), (0, 3));
    VALIDATIONS.set(0);
    MANIFEST_READS.set(0);
    let mut ranges = fixture
        .store
        .read_many_counted(&key, &[("catalogue", 0), ("series", 0), ("missing", 0)])
        .unwrap();
    assert_eq!((VALIDATIONS.get(), MANIFEST_READS.get()), (0, 1));
    assert_eq!(ranges.pop().unwrap().0, 0);
    let (count, file) = ranges.pop().unwrap();
    assert_eq!(count, 5);
    fixture.store.clear().unwrap();
    assert_eq!(bytes(file.unwrap()), [0]);
    assert_eq!(bytes(ranges.pop().unwrap().1.unwrap()), b"catalogue");
}

#[test]
fn requested_ranges_share_one_manifest_without_losing_order_or_held_reads() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let sections: Vec<_> = (0..48)
        .map(|index| format!("synthetic-text-{index}"))
        .collect();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    for section in &sections {
        writer.write(section, section.as_bytes()).unwrap();
    }
    writer.commit().unwrap();
    let mut ranges: Vec<_> = sections
        .iter()
        .map(|section| (section.as_str(), 0))
        .collect();
    VALIDATIONS.set(0);
    MANIFEST_READS.set(0);
    let old: Vec<_> = ranges
        .iter()
        .map(|&(section, range)| bytes(fixture.store.read(&key, section, range).unwrap().unwrap()))
        .collect();
    assert_eq!((VALIDATIONS.get(), MANIFEST_READS.get()), (0, 48));
    ranges.extend([
        ("synthetic-text-0", 0),
        ("absent", 0),
        ("synthetic-text-0", 1),
    ]);
    VALIDATIONS.set(0);
    MANIFEST_READS.set(0);
    let mut files = fixture.store.read_many(&key, &ranges).unwrap();
    assert_eq!((VALIDATIONS.get(), MANIFEST_READS.get()), (0, 1));
    assert_eq!(files.len(), ranges.len());
    assert!(files.pop().unwrap().is_none());
    assert!(files.pop().unwrap().is_none());
    fs::write(&fixture.content[0], b"changed content").unwrap();
    assert!(
        fixture
            .store
            .read_many(&fixture.key(), &ranges)
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
    fixture.store.clear().unwrap();
    let duplicate = files.pop().unwrap().unwrap();
    assert_eq!(bytes(duplicate), old[0]);
    assert_eq!(
        files
            .into_iter()
            .map(|file| bytes(file.unwrap()))
            .collect::<Vec<_>>(),
        old
    );
}

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
fn unchanged_preparation_resumes_without_rewriting_accepted_ranges() {
    let fixture = Fixture::new();
    let key = fixture.save(b"accepted prefix");
    let prefix_path = fixture.range_path(0, 0);
    let held = fixture.store.read(&key, "countries", 0).unwrap().unwrap();
    assert_eq!(fixture.store.saved_key().unwrap(), Some(key.clone()));
    assert_eq!(
        fixture.store.range_count(&key, "countries").unwrap(),
        Some(1)
    );
    assert_eq!(fixture.store.range_count(&key, "absent").unwrap(), None);
    let mut writer = fixture.store.extend(&key).unwrap();
    assert_eq!(writer.bytes_written(), 0);
    assert_eq!(writer.write("countries", &b"accepted tail"[..]).unwrap(), 1);
    assert_eq!(writer.bytes_written(), b"accepted tail".len() as u64);
    assert_eq!(
        fixture.store.range_count(&key, "countries").unwrap(),
        Some(1)
    );
    writer.commit().unwrap();
    assert_eq!(fixture.range_path(0, 0), prefix_path);
    assert_eq!(
        fixture.store.range_count(&key, "countries").unwrap(),
        Some(2)
    );
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 1).unwrap().unwrap()),
        b"accepted tail"
    );
    assert_eq!(bytes(held), b"accepted prefix");
}

#[test]
fn resume_refuses_changed_inputs_and_retains_prior_key_for_prefix_decisions() {
    let fixture = Fixture::new();
    let key = fixture.save(b"accepted prefix");
    fs::write(&fixture.recording, b"synthetic appended bytes").unwrap();
    assert!(fixture.store.extend(&key).is_err());
    assert_eq!(fixture.store.range_count(&key, "countries").unwrap(), None);
    assert_eq!(fixture.store.saved_key().unwrap(), Some(key));
}

#[test]
fn a_verified_appended_prefix_can_restore_its_accepted_accumulator() {
    let fixture = Fixture::new();
    let previous = fixture.save(b"accepted accumulator");
    fs::write(&fixture.recording, b"synthetic appended tail").unwrap();
    let current = fixture.key();
    assert_eq!(previous.recording_size(), b"synthetic".len() as u64);
    assert!(
        fixture
            .store
            .read(&previous, "countries", 0)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture
            .store
            .prefix_range_count(&previous, &current, "countries")
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        bytes(
            fixture
                .store
                .read_prefix(&previous, &current, "countries", 0)
                .unwrap()
                .unwrap()
        ),
        b"accepted accumulator"
    );
    fs::write(&fixture.content[0], b"changed base content").unwrap();
    let changed = fixture.key();
    assert!(
        fixture
            .store
            .read_prefix(&previous, &changed, "countries", 0)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture
            .store
            .prefix_range_count(&previous, &changed, "countries")
            .unwrap(),
        None
    );
}

#[test]
fn partial_tail_replacement_preserves_logical_ranges_and_held_readers() {
    let fixture = Fixture::new();
    let key = fixture.save(b"old tail");
    let held = fixture.store.read(&key, "countries", 0).unwrap().unwrap();
    let mut writer = fixture.store.extend(&key).unwrap();
    writer
        .write_at("countries", 0, &b"extended tail"[..])
        .unwrap();
    writer.write_at("countries", 1, &b"next"[..]).unwrap();
    writer
        .write_at("countries", 1, &b"next completed"[..])
        .unwrap();
    assert!(writer.write_at("countries", 3, &b"hole"[..]).is_err());
    assert_eq!(
        bytes(writer.read_at("countries", 0).unwrap().unwrap()),
        b"extended tail"
    );
    assert_eq!(
        bytes(writer.read_at("countries", 1).unwrap().unwrap()),
        b"next completed"
    );
    assert!(writer.read_at("countries", 2).unwrap().is_none());
    assert!(writer.read_at("unknown", 0).unwrap().is_none());
    assert_eq!(
        writer.payload_bytes_change(),
        (b"extended tail".len() + b"next completed".len() - b"old tail".len()) as i128
    );
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 0).unwrap().unwrap()),
        b"old tail"
    );
    writer.commit().unwrap();
    assert_eq!(
        fixture.store.range_count(&key, "countries").unwrap(),
        Some(2)
    );
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 0).unwrap().unwrap()),
        b"extended tail"
    );
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 1).unwrap().unwrap()),
        b"next completed"
    );
    assert_eq!(bytes(held), b"old tail");
}

#[test]
fn abandoned_tail_replacement_keeps_the_published_tail() {
    let fixture = Fixture::new();
    let key = fixture.save(b"published tail");
    let mut writer = fixture.store.extend(&key).unwrap();
    writer
        .write_at("countries", 0, &b"unaccepted partial"[..])
        .unwrap();
    drop(writer);
    let writer = fixture.store.extend(&key).unwrap();
    drop(writer);
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 0).unwrap().unwrap()),
        b"published tail"
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
        bytes(
            fixture
                .store
                .read(&fixture.key(), "countries", 0)
                .unwrap()
                .unwrap()
        ),
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
    assert_eq!(fixture.names().len(), 4); // manifest, source stamps, old range, lock
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
            assert_eq!(fixture.store.is_reusable(&old).unwrap(), content);
            assert!(fixture.store.read(&now, "countries", 0).unwrap().is_none());
            assert_eq!(
                fixture.store.read(&old, "countries", 0).unwrap().is_some(),
                content
            );
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
    wrong_prefix.binding = Arc::default();
    assert!(fixture.store.append(&wrong_prefix, grown).is_err());
    fs::write(&fixture.content[1], b"changed mod archive").unwrap();
    assert!(fixture.store.append(&old, fixture.key()).is_err());
    assert_eq!(fixture.names().len(), 4);
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
        source: fixture.store.catalog().unwrap().unwrap().header.source,
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

#[test]
fn recapture_checks_same_ordered_paths_and_current_stamps() {
    let fixture = Fixture::new();
    let initial = fixture.key();
    assert_eq!(initial.recapture().unwrap(), initial);
    fs::write(&fixture.recording, b"synthetic appended recording").unwrap();
    let grown = initial.recapture().unwrap();
    assert_eq!(grown, fixture.key());
    assert!(grown.recording_size() > initial.recording_size());
    fs::write(&fixture.content[1], b"changed mod archive length").unwrap();
    let changed = initial.recapture().unwrap();
    assert_eq!(changed, fixture.key());
    assert_ne!(changed, grown);
    assert_eq!(changed.content[0], initial.content[0]);
    assert_eq!(changed.content[1].path, initial.content[1].path);
}
