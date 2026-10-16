use super::*;
use std::{
    fs::FileTimes,
    io::{Seek, SeekFrom},
    time::Duration,
};

thread_local! {
    pub(super) static VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static MANIFEST_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static INPUT_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static KEY_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Files the store created: packs and publication temporaries.
    pub(super) static CREATED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Flushes the store issued (or skipped under `test-support`).
    pub(super) static SYNCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Flush even under `test-support`, for timing tests.
    pub(super) static FORCE_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The commit content recheck interval; zero rechecks at every commit.
    pub(super) static CONTENT_RECHECK: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

#[test]
fn a_commit_creates_and_flushes_one_pack_per_writer_whatever_its_range_count() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut counts = Vec::new();
    for ranges in [10u64, 1_000] {
        CREATED.set(0);
        SYNCS.set(0);
        let mut writer = fixture.store.replace(key.clone()).unwrap();
        let mut forks = [writer.fork().unwrap(), writer.fork().unwrap()];
        for range in 0..ranges {
            assert_eq!(
                writer.write("parent", &range.to_le_bytes()[..]).unwrap(),
                range
            );
            for (index, fork) in forks.iter_mut().enumerate() {
                fork.write(&format!("fork-{index}"), &range.to_be_bytes()[..])
                    .unwrap();
            }
        }
        assert_eq!(SYNCS.get(), 0, "writes flush nothing before commit");
        for fork in forks {
            for staged in fork.into_staged().unwrap() {
                writer.adopt(staged).unwrap();
            }
        }
        writer.commit().unwrap();
        counts.push((CREATED.get(), SYNCS.get()));
        let last = ranges - 1;
        assert_eq!(
            bytes(fixture.store.read(&key, "parent", last).unwrap().unwrap()),
            last.to_le_bytes()
        );
        assert_eq!(
            bytes(fixture.store.read(&key, "fork-1", last).unwrap().unwrap()),
            last.to_be_bytes()
        );
    }
    // Three packs and the manifest's temporary, each manifest flushed before
    // and after its rename; the first commit also publishes the source stamps.
    assert_eq!(counts, [(3 + 2, 3 + 4), (3 + 1, 3 + 2)]);
    drop(fixture.store.extend(&key).unwrap());
    let names = fixture.names();
    assert_eq!(names.len(), 6, "{names:?}"); // lock, manifest, stamps, three packs
    assert_eq!(
        names
            .iter()
            .filter(|name| parse_pack(name).is_some())
            .count(),
        3
    );
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
    let old = fixture.store.catalog().unwrap().unwrap();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("countries", &b"new bytes"[..]).unwrap();
    writer.write("countries", &b"next range"[..]).unwrap();
    writer.commit().unwrap();
    drop(fixture.store.extend(&key).unwrap());
    assert_eq!(
        fixture
            .store
            .read_catalog(&old, "countries", 0, &mut Opened::new())
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

    /// The pack that holds a published range.
    fn range_path(&self, section: usize, range: u64) -> PathBuf {
        let id = self
            .store
            .catalog()
            .unwrap()
            .unwrap()
            .entry(section, range)
            .unwrap()
            .unwrap();
        self.store.directory.join(pack_name(id.pack))
    }

    fn packs(&self) -> Vec<PackRecord> {
        self.store.catalog().unwrap().unwrap().packs().unwrap()
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

fn bytes(mut range: Range) -> Vec<u8> {
    let mut bytes = Vec::new();
    range.read_to_end(&mut bytes).unwrap();
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
fn commits_recheck_content_once_per_interval_and_reopen_refuses_an_edit_within_it() {
    let fixture = Fixture::new();
    let inputs = vec![fixture.content[0].clone(); 64];
    CONTENT_RECHECK.set(Duration::from_secs(3600));
    let key = SourceKey::capture(&fixture.recording, &inputs).unwrap();
    INPUT_CHECKS.set(0);
    for page in 0..5_u8 {
        let mut writer = fixture.store.replace(key.clone()).unwrap();
        writer.write("pages", &[page][..]).unwrap();
        writer.commit().unwrap();
    }
    // The recording at each commit; the content was read at capture.
    assert_eq!(INPUT_CHECKS.get(), 5);
    // An edit within the interval is published under the old stamps, which
    // the next binding of the edited content refuses.
    fs::write(&fixture.content[0], b"edited within the interval").unwrap();
    let mut writer = fixture.store.extend(&key).unwrap();
    writer.write("pages", &b"late"[..]).unwrap();
    writer.commit().unwrap();
    let edited = SourceKey::capture(&fixture.recording, &inputs).unwrap();
    assert!(!fixture.store.is_reusable(&edited).unwrap());
    // Once the interval has passed, a commit reads the content again and
    // refuses the edit.
    CONTENT_RECHECK.set(Duration::ZERO);
    INPUT_CHECKS.set(0);
    assert!(fixture.store.extend(&key).unwrap().commit().is_err());
    // The recording when extending, then the recording and every input.
    assert_eq!(INPUT_CHECKS.get(), 2 + inputs.len());
    // A recording change is refused at every commit.
    CONTENT_RECHECK.set(Duration::from_secs(3600));
    let fresh = SourceKey::capture(&fixture.recording, &[fixture.content[1].clone()]).unwrap();
    let mut writer = fixture.store.replace(fresh).unwrap();
    writer.write("pages", &b"page"[..]).unwrap();
    fs::write(&fixture.recording, b"a grown recording").unwrap();
    assert!(writer.commit().is_err());
    CONTENT_RECHECK.set(Duration::ZERO);
}

#[test]
fn hundred_year_range_count_opens_only_the_manifest_header_and_one_entry() {
    let fixture = Fixture::new();
    let key = fixture.save(b"placeholder");
    // One hourly range per section over 100 years. References are synthetic;
    // all but the final range are deliberately absent: opening cannot touch them.
    let count = 100 * 366 * 24;
    let pack = 0x0123_4567_89ab_cdef;
    let id = RangeId {
        pack,
        offset: 7,
        len: 18,
    };
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
        let offset = 8 + header.len() as u64 + (2 * count - 1) * ENTRY_BYTES;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&id.encode())?;
        file.write_all(
            &PackRecord {
                id: pack,
                live: 18,
                size: 25,
            }
            .encode(),
        )
    })
    .unwrap();
    let path = fixture.store.directory.join(pack_name(pack));
    publish(&path, |file| file.write_all(b"unused last century range")).unwrap();
    assert!(fixture.store.is_reusable(&key).unwrap());
    let mut opened = fixture.store.catalog().unwrap().unwrap();
    assert_eq!(opened.offsets.len(), 2);
    assert_eq!(opened.file.stream_position().unwrap(), opened.offsets[0]);
    assert_eq!(opened.entry(1, count - 1).unwrap(), Some(id));
    assert_eq!(opened.file.stream_position().unwrap(), opened.table);
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
            .filter(|name| parse_pack(name).is_some())
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

/// The published sections in manifest order and each range's bytes.
type Published = (Vec<(String, u64)>, Vec<Vec<Vec<u8>>>);

fn published(fixture: &Fixture, key: &SourceKey) -> Published {
    drop(fixture.store.extend(key).unwrap());
    let catalog = fixture.store.catalog().unwrap().unwrap();
    let sections: Vec<_> = catalog
        .header
        .sections
        .iter()
        .map(|section| (section.name.clone(), section.count))
        .collect();
    let payloads = sections
        .iter()
        .enumerate()
        .map(|(index, (_, count))| {
            (0..*count)
                .map(|range| {
                    let id = catalog.entry(index, range).unwrap().unwrap();
                    bytes(id.open(&fixture.store.directory).unwrap())
                })
                .collect()
        })
        .collect();
    (sections, payloads)
}

/// Publishes the prefix both runs extend: two sections, three ranges.
fn forked_prefix(fixture: &Fixture) -> SourceKey {
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("old", &b"old 0"[..]).unwrap();
    writer.write("old", &b"old 1"[..]).unwrap();
    writer.write("other", &b"other 0"[..]).unwrap();
    writer.commit().unwrap();
    key
}

/// The serial order of four producer steps; producers 0 and 1 alternate.
const FORKED_STEPS: [fn(&mut Writer); 4] = [
    |writer| {
        writer.write_at("old", 0, &b"old 0 v2"[..]).unwrap();
        assert_eq!(writer.write("old", &b"old 2"[..]).unwrap(), 2);
        writer.write_at("a-new", 0, &b"a 0"[..]).unwrap();
    },
    |writer| {
        assert_eq!(
            bytes(writer.read_at("parent", 0).unwrap().unwrap()),
            b"parent 0"
        );
        assert_eq!(writer.write("b-new", &b"b 0"[..]).unwrap(), 0);
        writer.write_at("other", 0, &b"other 0 v2"[..]).unwrap();
    },
    |writer| {
        // Rewrites and reads back ranges this producer wrote in its first step.
        writer.write_at("a-new", 0, &b"a 0 v2"[..]).unwrap();
        assert_eq!(
            bytes(writer.read_at("a-new", 0).unwrap().unwrap()),
            b"a 0 v2"
        );
        writer.write_at("old", 2, &b"old 2 v2"[..]).unwrap();
        assert_eq!(
            bytes(writer.read_at("old", 0).unwrap().unwrap()),
            b"old 0 v2"
        );
        assert_eq!(bytes(writer.read_at("old", 1).unwrap().unwrap()), b"old 1");
        assert!(writer.read_at("old", 3).unwrap().is_none());
        assert!(writer.read_at("absent", 0).unwrap().is_none());
        writer.write("a-later", &b"a later"[..]).unwrap();
    },
    |writer| {
        assert_eq!(writer.write("b-new", &b"b 1"[..]).unwrap(), 1);
    },
];

#[test]
fn forked_batches_adopted_in_order_publish_exactly_what_direct_writes_publish() {
    let direct = Fixture::new();
    let key = forked_prefix(&direct);
    let mut writer = direct.store.extend(&key).unwrap();
    writer.write("parent", &b"parent 0"[..]).unwrap();
    for step in FORKED_STEPS {
        step(&mut writer);
    }
    let direct_bytes = (writer.bytes_written(), writer.payload_bytes_change());
    writer.commit().unwrap();
    let expected = published(&direct, &key);
    assert_eq!(
        expected.0,
        [
            ("old".to_owned(), 3),
            ("other".to_owned(), 1),
            ("parent".to_owned(), 1),
            ("a-new".to_owned(), 1),
            ("b-new".to_owned(), 2),
            ("a-later".to_owned(), 1),
        ]
    );

    let forked = Fixture::new();
    let key = forked_prefix(&forked);
    let mut writer = forked.store.extend(&key).unwrap();
    writer.write("parent", &b"parent 0"[..]).unwrap();
    // Each producer runs its steps on its own thread, the second producer
    // first, so only adoption order can reproduce the serial layout.
    let mut producers = [writer.fork().unwrap(), writer.fork().unwrap()];
    let [first, second] = &mut producers;
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                for step in [1, 3] {
                    FORKED_STEPS[step](second);
                    second.end_batch();
                }
            })
            .join()
            .unwrap();
        scope.spawn(|| {
            for step in [0, 2] {
                FORKED_STEPS[step](first);
                first.end_batch();
            }
        });
    });
    assert_eq!(writer.bytes_written(), b"parent 0".len() as u64);
    let [first, second] = producers.map(|fork| fork.into_staged().unwrap());
    assert_eq!((first.len(), second.len()), (2, 2));
    let (mut first, mut second) = (first.into_iter(), second.into_iter());
    for staged in [
        first.next().unwrap(),
        second.next().unwrap(),
        first.next().unwrap(),
        second.next().unwrap(),
    ] {
        writer.adopt(staged).unwrap();
    }
    assert_eq!(
        (writer.bytes_written(), writer.payload_bytes_change()),
        direct_bytes
    );
    writer.commit().unwrap();
    assert_eq!(published(&forked, &key), expected);
}

#[test]
fn adoption_refuses_overlapping_forks_and_other_transactions() {
    let fixture = Fixture::new();
    let key = fixture.save(b"zero");
    let mut writer = fixture.store.extend(&key).unwrap();
    let mut first = writer.fork().unwrap();
    let mut second = writer.fork().unwrap();
    let mut reader = writer.fork().unwrap();
    let mut created = writer.fork().unwrap();
    let mut recreated = writer.fork().unwrap();
    first.write("countries", &b"one"[..]).unwrap();
    second.write("countries", &b"also one"[..]).unwrap();
    assert!(reader.read_at("countries", 1).unwrap().is_none());
    created.write("fresh", &b"fresh"[..]).unwrap();
    recreated.write("fresh", &b"fresh again"[..]).unwrap();
    assert_eq!(
        first.fork().err().map(|error| error.kind()),
        Some(io::ErrorKind::Unsupported)
    );
    let staged = |fork: Writer| {
        let mut staged = fork.into_staged().unwrap();
        assert_eq!(staged.len(), 1);
        staged.pop().unwrap()
    };
    writer.adopt(staged(first)).unwrap();
    writer.adopt(staged(created)).unwrap();
    // Overlaps are errors, never a family's refusal of its data (InvalidData).
    for fork in [second, reader, recreated] {
        let error = writer.adopt(staged(fork)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other, "{error}");
    }
    assert_eq!(
        writer.fork().unwrap().commit().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    // A fork's duplicate lock handle keeps other writers out until it closes.
    let mut late = writer.fork().unwrap();
    late.write("late", &b"late"[..]).unwrap();
    // A refused adoption leaves the transaction unpublishable.
    assert_eq!(writer.commit().unwrap_err().kind(), io::ErrorKind::Other);
    assert_eq!(published_names(&fixture), ["countries"]);
    assert!(fixture.store.extend(&key).is_err());
    let stale = staged(late);
    let mut next = fixture.store.extend(&key).unwrap();
    assert_eq!(
        next.adopt(stale).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let mut fork = next.fork().unwrap();
    fork.write("countries", &b"one"[..]).unwrap();
    for staged in fork.into_staged().unwrap() {
        next.adopt(staged).unwrap();
    }
    next.commit().unwrap();
    assert_eq!(published(&fixture, &key).0, [("countries".to_owned(), 2)]);
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 1).unwrap().unwrap()),
        b"one"
    );
    let writer = fixture.store.extend(&key).unwrap();
    assert_eq!(
        writer.into_staged().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

fn published_names(fixture: &Fixture) -> Vec<String> {
    let catalog = fixture.store.catalog().unwrap().unwrap();
    catalog
        .header
        .sections
        .iter()
        .map(|section| section.name.clone())
        .collect()
}

#[test]
fn an_older_layout_reads_as_absent_and_the_next_writer_removes_it() {
    let fixture = Fixture::new();
    let key = fixture.key();
    fs::create_dir_all(&fixture.store.directory).unwrap();
    // The range-file layout: one file per range, named by 16-byte manifest entries.
    let range = Uuid::new_v4();
    let header = serde_json::json!({
        "version": 2,
        "source": Uuid::new_v4(),
        "sections": [{"name": "countries", "count": 1}],
    });
    publish(&fixture.store.directory.join(MANIFEST), |file| {
        let header = serde_json::to_vec(&header)?;
        file.write_all(&(header.len() as u64).to_le_bytes())?;
        file.write_all(&header)?;
        file.write_all(range.as_bytes())
    })
    .unwrap();
    let old = fixture
        .store
        .directory
        .join(format!("0_0_{}.bin", range.simple()));
    fs::write(&old, b"older layout").unwrap();
    assert!(!fixture.store.is_reusable(&key).unwrap());
    assert!(fixture.store.saved_key().unwrap().is_none());
    assert!(fixture.store.read(&key, "countries", 0).unwrap().is_none());
    assert!(fixture.store.extend(&key).is_err());
    fixture.save(b"current layout");
    assert!(!old.exists());
    assert_eq!(
        bytes(fixture.store.read(&key, "countries", 0).unwrap().unwrap()),
        b"current layout"
    );
    assert_eq!(fixture.names().len(), 4); // lock, manifest, stamps, one pack
}

#[test]
fn packs_under_half_live_are_emptied_at_commit_without_changing_held_readers() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let page = |range: u64, version: u8| vec![version; 100 + range as usize];
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    for range in 0..10 {
        writer.write("pages", page(range, 0).as_slice()).unwrap();
    }
    writer.commit().unwrap();
    let first = fixture.range_path(0, 0);
    assert_eq!(
        fixture.packs(),
        [PackRecord {
            id: parse_pack(first.file_name().unwrap().to_str().unwrap()).unwrap(),
            live: 1045,
            size: 1045,
        }]
    );
    let held: Vec<_> = (0..10)
        .map(|range| fixture.store.read(&key, "pages", range).unwrap().unwrap())
        .collect();
    CREATED.set(0);
    SYNCS.set(0);
    let mut writer = fixture.store.extend(&key).unwrap();
    // The published pack keeps 430 of its 1,045 bytes listed.
    for range in 0..6 {
        writer
            .write_at("pages", range, page(range, 1).as_slice())
            .unwrap();
    }
    // This writer's own pack keeps 615 of 1,615 bytes listed.
    for version in 2..12 {
        writer
            .write_at("pages", 0, page(0, version).as_slice())
            .unwrap();
    }
    writer.commit().unwrap();
    // The writer's pack, the pack the listed bytes moved into and the
    // manifest's temporary; only the new pack and the manifest are flushed.
    assert_eq!((CREATED.get(), SYNCS.get()), (3, 1 + 2));
    let expected: Vec<_> = (0..10)
        .map(|range| match range {
            0 => page(0, 11),
            1..6 => page(range, 1),
            _ => page(range, 0),
        })
        .collect();
    let listed = fixture.packs();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].live, listed[0].size), (1045, 1045));
    assert_eq!(published(&fixture, &key).1, [expected]);
    // The next writer open removed both emptied packs; held readers kept them.
    assert!(!first.exists());
    let on_disk: u64 = fixture
        .names()
        .iter()
        .filter(|name| parse_pack(name).is_some())
        .map(|name| {
            fs::metadata(fixture.store.directory.join(name))
                .unwrap()
                .len()
        })
        .sum();
    assert_eq!(on_disk, 1045);
    for (range, held) in held.into_iter().enumerate() {
        assert_eq!(bytes(held), page(range as u64, 0));
    }
    // A later commit that leaves every pack at least half live moves nothing.
    let mut writer = fixture.store.extend(&key).unwrap();
    writer.write_at("pages", 9, page(9, 12).as_slice()).unwrap();
    writer.commit().unwrap();
    let listed = fixture.packs();
    assert_eq!(listed.len(), 2);
    for pack in &listed {
        assert!(pack.live * 2 >= pack.size, "{pack:?}");
    }
    assert_eq!(listed.iter().map(|pack| pack.live).sum::<u64>(), 1045);
}

/// Yields `bytes` bytes, then fails.
struct Failing {
    bytes: usize,
}

impl Read for Failing {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.bytes == 0 {
            return Err(io::Error::other("synthetic producer failure"));
        }
        let read = buffer.len().min(self.bytes);
        buffer[..read].fill(9);
        self.bytes -= read;
        Ok(read)
    }
}

#[test]
fn a_failed_range_leaves_no_bytes_in_its_pack() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("pages", &b"first"[..]).unwrap();
    // Small enough to stay buffered, then large enough to reach the file.
    for bytes in [3, PACK_BUFFER as usize * 2 + 10] {
        assert!(writer.write("pages", Failing { bytes }).is_err());
        assert_eq!(writer.bytes_written(), 5);
    }
    assert_eq!(writer.write("pages", &b"second"[..]).unwrap(), 1);
    assert_eq!(
        bytes(writer.read_at("pages", 1).unwrap().unwrap()),
        b"second"
    );
    writer.commit().unwrap();
    assert_eq!(
        bytes(fixture.store.read(&key, "pages", 0).unwrap().unwrap()),
        b"first"
    );
    assert_eq!(
        bytes(fixture.store.read(&key, "pages", 1).unwrap().unwrap()),
        b"second"
    );
    assert_eq!(fs::metadata(fixture.range_path(0, 1)).unwrap().len(), 11);
    let listed = fixture.packs();
    assert_eq!(
        listed,
        [PackRecord {
            id: listed[0].id,
            live: 11,
            size: 11
        }]
    );
}

#[test]
fn a_range_reads_and_seeks_only_within_its_own_bytes() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("pages", &b"abc"[..]).unwrap();
    writer.write("pages", &b"defgh"[..]).unwrap();
    writer.write("pages", &b"ijk"[..]).unwrap();
    writer.commit().unwrap();
    let mut ranges = fixture
        .store
        .read_many(&key, &[("pages", 1), ("pages", 2)])
        .unwrap();
    let other = ranges.pop().unwrap().unwrap();
    let mut range = ranges.pop().unwrap().unwrap();
    assert!(
        Arc::ptr_eq(range.file.as_ref().unwrap(), other.file.as_ref().unwrap()),
        "one pack, one handle"
    );
    assert_eq!((range.len(), range.is_empty()), (5, false));
    assert_ne!(range.id(), other.id());
    assert_eq!(range.seek(SeekFrom::End(-2)).unwrap(), 3);
    let mut tail = String::new();
    range.read_to_string(&mut tail).unwrap();
    assert_eq!(tail, "gh");
    assert_eq!(range.seek(SeekFrom::Start(10)).unwrap(), 10);
    assert_eq!(range.read(&mut [0; 4]).unwrap(), 0);
    assert!(range.seek(SeekFrom::Current(-11)).is_err());
    range.rewind().unwrap();
    assert_eq!(bytes(range.clone()), b"defgh");
    assert_eq!(range.stream_position().unwrap(), 0);
}

#[test]
#[ignore = "flushes to disk: commit cost of 5,000 ranges of 2 KB"]
fn five_thousand_two_kilobyte_ranges_commit_with_one_flush_per_pack() {
    use std::time::Instant;
    const RANGES: usize = 5_000;
    let payload = vec![7; 2048];
    let fixture = Fixture::new();
    // The range-file layout: each range its own file, flushed at commit on
    // threads bounded by the CPU count, then its directory once.
    let directory = fixture.root.join("range-files");
    fs::create_dir(&directory).unwrap();
    let started = Instant::now();
    let paths: Vec<_> = (0..RANGES)
        .map(|range| {
            let path = directory.join(format!("0_{range}_{}.bin", Uuid::new_v4().simple()));
            OpenOptions::new()
                .write(true)
                .share_mode(SHARING)
                .create_new(true)
                .open(&path)
                .unwrap()
                .write_all(&payload)
                .unwrap();
            path
        })
        .collect();
    let old_writes = started.elapsed();
    let started = Instant::now();
    let workers = std::thread::available_parallelism().map_or(1, |count| count.get());
    std::thread::scope(|scope| {
        for part in paths.chunks(paths.len().div_ceil(workers)) {
            scope.spawn(move || {
                for path in part {
                    OpenOptions::new()
                        .write(true)
                        .share_mode(SHARING)
                        .open(path)
                        .unwrap()
                        .sync_all()
                        .unwrap();
                }
            });
        }
    });
    sync_file(paths.last().unwrap()).unwrap();
    let old_commit = started.elapsed();
    eprintln!("range files: {RANGES} writes {old_writes:?}, commit {old_commit:?}");

    FORCE_SYNC.set(true);
    for forks in [0, 16] {
        let key = fixture.key();
        let started = Instant::now();
        let mut writer = fixture.store.replace(key.clone()).unwrap();
        let mut producers: Vec<_> = (0..forks).map(|_| writer.fork().unwrap()).collect();
        for range in 0..RANGES {
            match producers.len() {
                0 => writer.write("pages", payload.as_slice()).map(drop),
                count => producers[range % count]
                    .write(&format!("pages-{}", range % count), payload.as_slice())
                    .map(drop),
            }
            .unwrap();
        }
        for producer in producers {
            for staged in producer.into_staged().unwrap() {
                writer.adopt(staged).unwrap();
            }
        }
        let writes = started.elapsed();
        SYNCS.set(0);
        let started = Instant::now();
        writer.commit().unwrap();
        let commit = started.elapsed();
        eprintln!(
            "packs ({} writers): {RANGES} writes {writes:?}, commit {commit:?}, flushes {}",
            forks + 1,
            SYNCS.get()
        );
    }
    FORCE_SYNC.set(false);
}

#[test]
fn a_scope_checks_its_key_once_and_shares_pack_handles() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    for range in 0..50u64 {
        writer.write("a", &range.to_le_bytes()[..]).unwrap();
        writer.write("b", &range.to_be_bytes()[..]).unwrap();
    }
    writer.commit().unwrap();
    let before = counters::now();
    for range in 0..50 {
        fixture.store.read(&key, "a", range).unwrap().unwrap();
    }
    let unscoped = counters::now() - before;
    assert_eq!((unscoped.stats, unscoped.manifest_reads), (50, 50));
    let scope = fixture.store.scope(&key).unwrap();
    let before = counters::now();
    for _ in 0..3 {
        for range in 0..50 {
            assert_eq!(
                bytes(fixture.store.read(&key, "a", range).unwrap().unwrap()),
                range.to_le_bytes()
            );
        }
        assert_eq!(fixture.store.range_count(&key, "b").unwrap(), Some(50));
        assert!(fixture.store.is_reusable(&key).unwrap());
        let many = fixture
            .store
            .read_many_counted(&key, &[("b", 49), ("absent", 0), ("b", 50)])
            .unwrap();
        assert_eq!(many[0].0, 50);
        assert!(many[1].1.is_none() && many[2].1.is_none());
    }
    let work = counters::now() - before;
    // The scope's one key check and manifest read happened when it opened.
    assert_eq!(
        work,
        counters::Counters {
            pack_opens: 1,
            ..Default::default()
        }
    );
    // A replacement published during the scope: ranges the session already
    // opened keep their bytes; it never mixes in the new manifest's ranges.
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("a", &b"replaced"[..]).unwrap();
    writer.commit().unwrap();
    drop(fixture.store.extend(&key).unwrap());
    assert_eq!(
        bytes(fixture.store.read(&key, "a", 7).unwrap().unwrap()),
        7u64.to_le_bytes()
    );
    drop(scope);
    assert_eq!(
        bytes(fixture.store.read(&key, "a", 0).unwrap().unwrap()),
        b"replaced"
    );
    // A session over a pack a writer removed continues with the current manifest.
    let fresh = fixture.store.session(&key).unwrap().unwrap();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("a", &b"third"[..]).unwrap();
    writer.commit().unwrap();
    drop(fixture.store.extend(&key).unwrap());
    assert_eq!(bytes(fresh.read("a", 0).unwrap().unwrap()), b"third");
    // Another key, or an unscoped thread, still checks every read.
    fs::write(&fixture.content[0], b"changed content").unwrap();
    let changed = fixture.key();
    let _scope = fixture.store.scope(&key).unwrap();
    let before = counters::now();
    assert!(fixture.store.read(&changed, "a", 0).unwrap().is_none());
    assert_eq!((counters::now() - before).stats, 1);
    std::thread::scope(|threads| {
        threads.spawn(|| {
            let before = counters::now();
            fixture.store.read(&key, "a", 0).unwrap();
            assert_eq!((counters::now() - before).manifest_reads, 1);
        });
    });
}

#[test]
fn empty_ranges_need_no_pack_and_stay_readable_through_append() {
    let fixture = Fixture::new();
    let previous = fixture.key();
    let mut writer = fixture.store.replace(previous.clone()).unwrap();
    writer.write("catalogue", &b"base"[..]).unwrap();
    writer.write("catalogue", io::empty()).unwrap();
    writer.commit().unwrap();
    // Replacing the only non-empty range leaves its pack nothing listed; the
    // next writer open removes it, but the empty range never needed it.
    let mut writer = fixture.store.extend(&previous).unwrap();
    writer.write_at("catalogue", 0, &b"text"[..]).unwrap();
    writer.write_at("catalogue", 1, io::empty()).unwrap();
    assert!(writer.read_at("catalogue", 1).unwrap().unwrap().is_empty());
    writer.commit().unwrap();
    drop(fixture.store.extend(&previous).unwrap());
    let read = |key: &SourceKey, range| {
        bytes(
            fixture
                .store
                .read(key, "catalogue", range)
                .unwrap()
                .unwrap(),
        )
    };
    assert_eq!(
        (read(&previous, 0), read(&previous, 1)),
        (b"text".to_vec(), Vec::new())
    );
    let listed = fixture.packs();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!((listed[0].live, listed[0].size), (4, 4));
    // A growing recording: while its append writer is open and uncommitted
    // (its open ran cleanup), the published ranges stay readable through the
    // prefix of the previous key, and the new key is not yet published.
    fs::write(&fixture.recording, b"synthetic recording appended").unwrap();
    let current = fixture.key();
    let mut writer = fixture.store.append(&previous, current.clone()).unwrap();
    let prefix = |range| {
        bytes(
            fixture
                .store
                .read_prefix(&previous, &current, "catalogue", range)
                .unwrap()
                .unwrap(),
        )
    };
    assert_eq!((prefix(0), prefix(1)), (b"text".to_vec(), Vec::new()));
    assert!(fixture.store.session(&current).unwrap().is_none());
    writer.write("catalogue", io::empty()).unwrap();
    writer.write("catalogue", &b"tail"[..]).unwrap();
    writer.commit().unwrap();
    drop(fixture.store.extend(&current).unwrap());
    let session = fixture.store.session(&current).unwrap().unwrap();
    let ranges: Vec<_> = (0..4)
        .map(|range| bytes(session.read("catalogue", range).unwrap().unwrap()))
        .collect();
    assert_eq!(ranges, [&b"text"[..], &b""[..], &b""[..], &b"tail"[..]]);
    assert_eq!(
        session.read("catalogue", 1).unwrap().unwrap().id(),
        session.read("catalogue", 2).unwrap().unwrap().id()
    );
}
