use super::*;
use std::{
    collections::BTreeSet,
    fs::FileTimes,
    io::{Seek, SeekFrom},
    time::Duration,
};

#[test]
fn range_cache_keys_serialize_each_identity_operand_losslessly() {
    let first = RangeId {
        pack: u64::MAX,
        offset: 7,
        len: 11,
    };
    let encoded = serde_json::to_vec(&first.cache_key()).unwrap();
    assert_eq!(
        serde_json::from_slice::<[u64; 3]>(&encoded).unwrap(),
        [u64::MAX, 7, 11]
    );
    for other in [
        RangeId { pack: 1, ..first },
        RangeId { offset: 8, ..first },
        RangeId { len: 12, ..first },
    ] {
        assert_ne!(other.cache_key(), first.cache_key());
    }
}

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
    /// Commits stop, as a terminated process would, once their entry log and
    /// names are durable and before the root is written.
    pub(super) static TERMINATE_BEFORE_ROOT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Manifest bytes commits wrote: roots, entry logs and appended names.
    pub(super) static MANIFEST_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Runs and spans commits visited laying replacements over sections.
    pub(super) static RUN_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
    // Three packs (each fork flushes its own as it hands over its batches),
    // the entry log, the names file and the root's temporary; the parent's
    // pack, the log and the names are flushed, and the root before and after
    // its rename. The first commit also publishes the source stamps.
    assert_eq!(counts, [(3 + 3 + 1, 2 + 3 + 2 + 2), (3 + 3, 2 + 3 + 2)]);
    drop(fixture.store.extend(&key).unwrap());
    let names = fixture.names();
    // lock, manifest, stamps, names, one entry log, three packs
    assert_eq!(names.len(), 8, "{names:?}");
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
    // manifest, source stamps, names, entry log, old range, lock
    assert_eq!(fixture.names().len(), 6);
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
    let names = fixture.names();
    assert!(fixture.store.append(&old, old.clone()).is_err());
    fs::write(&fixture.recording, b"synthetic recording appended").unwrap();
    let grown = fixture.key();
    let mut wrong_prefix = old.clone();
    wrong_prefix.recording.size -= 1;
    wrong_prefix.binding = Arc::default();
    assert!(fixture.store.append(&wrong_prefix, grown).is_err());
    fs::write(&fixture.content[1], b"changed mod archive").unwrap();
    assert!(fixture.store.append(&old, fixture.key()).is_err());
    assert_eq!(fixture.names(), names);
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
fn hundred_year_range_count_opens_only_the_root_and_one_entry() {
    let fixture = Fixture::new();
    let key = fixture.save(b"placeholder");
    // One hourly range per section over 100 years, each section one run of
    // one entry log. References are synthetic; all but the final entry are
    // deliberately absent: opening and lookup cannot touch them.
    let count = 100 * 366 * 24;
    let pack = 0x0123_4567_89ab_cdef;
    let id = RangeId {
        pack,
        offset: 7,
        len: 18,
    };
    let directory = &fixture.store.directory;
    let log = 0x0fed_cba9_8765_4321;
    let entries = 2 * count * ENTRY_BYTES;
    let file = File::create(directory.join(log_name(log))).unwrap();
    // Extending the log supplies the logical count without millions of writes.
    file.set_len(entries).unwrap();
    write_all_at(&file, &id.encode(), entries - ENTRY_BYTES).unwrap();
    drop(file);
    let names = 0x1111_2222_3333_4444;
    let listed = encode_names(["countries", "wars"].into_iter());
    fs::write(directory.join(names_name(names)), &listed).unwrap();
    let runs = [0, count * ENTRY_BYTES].map(|offset| Run {
        first: 0,
        count,
        log,
        offset,
    });
    let root = encode_root(
        &Head {
            version: VERSION,
            source: fixture.store.catalog().unwrap().unwrap().header.source,
            names,
            names_bytes: listed.len() as u64,
            sections: 2,
            runs: 2,
            logs: 1,
        },
        &[(count, &runs[..1]), (count, &runs[1..])],
        &[PackRecord {
            id: log,
            live: entries,
            size: entries,
        }],
        &[PackRecord {
            id: pack,
            live: 18,
            size: 25,
        }],
    )
    .unwrap();
    publish(&directory.join(MANIFEST), |file| file.write_all(&root)).unwrap();
    let path = fixture.store.directory.join(pack_name(pack));
    publish(&path, |file| file.write_all(b"unused last century range")).unwrap();
    assert!(fixture.store.is_reusable(&key).unwrap());
    // The root holds a record per section and per run, whatever the ranges.
    assert!(fs::metadata(directory.join(MANIFEST)).unwrap().len() < 1024);
    let before = counters::now();
    let opened = fixture.store.catalog().unwrap().unwrap();
    assert_eq!(opened.runs.iter().map(Vec::len).collect::<Vec<_>>(), [1, 1]);
    assert_eq!(opened.entry(1, count - 1).unwrap(), Some(id));
    assert_eq!(
        counters::now() - before,
        counters::Counters {
            stats: 0,
            manifest_reads: 1,
            pack_opens: 0,
            log_opens: 1,
        }
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
fn corrupt_inner_manifest_counts_are_refused_before_allocating() {
    let fixture = Fixture::new();
    fixture.save(b"prepared");
    let path = fixture.store.directory.join(MANIFEST);
    let valid = fs::read(&path).unwrap();
    let body = 8 + u64::from_le_bytes(valid[..8].try_into().unwrap()) as usize;
    let mut runs = valid.clone();
    runs[body + 8..body + 16].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut head: serde_json::Value = serde_json::from_slice(&valid[8..body]).unwrap();
    head["names_bytes"] = u64::MAX.into();
    let head = serde_json::to_vec(&head).unwrap();
    let mut names = (head.len() as u64).to_le_bytes().to_vec();
    names.extend_from_slice(&head);
    names.extend_from_slice(&valid[body..]);
    for manifest in [runs, names] {
        fs::write(&path, &manifest).unwrap();
        let error = fixture.store.catalog().err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
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
    // lock, manifest, stamps, names, one entry log, one pack
    assert_eq!(fixture.names().len(), 6);
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
    // The writer's pack, the pack the listed bytes moved into, the entry log
    // and the root's temporary; only the new pack, the log and the root are
    // flushed. Dead bytes passed a quarter of the listed, so the published
    // pack was emptied too.
    assert_eq!((CREATED.get(), SYNCS.get()), (4, 2 + 2));
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
fn reading_to_the_end_appends_only_the_unread_rest_of_the_range() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("pages", &b"abc"[..]).unwrap();
    writer.write("pages", &b"defgh"[..]).unwrap();
    writer.write("pages", &b"ijk"[..]).unwrap();
    writer.commit().unwrap();
    let mut range = fixture.store.read(&key, "pages", 1).unwrap().unwrap();
    range.read_exact(&mut [0; 2]).unwrap();
    let mut buffer = b"xy".to_vec();
    assert_eq!(range.read_to_end(&mut buffer).unwrap(), 3);
    assert_eq!(buffer, b"xyfgh");
    assert_eq!(range.read_to_end(&mut buffer).unwrap(), 0);
    assert_eq!(buffer, b"xyfgh");
    assert_eq!(range.seek(SeekFrom::Start(10)).unwrap(), 10);
    assert_eq!(range.read_to_end(&mut buffer).unwrap(), 0);
    assert_eq!(buffer, b"xyfgh");
}

#[test]
fn reading_a_truncated_range_to_the_end_keeps_its_bytes_and_reserves_only_its_length() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("pages", &b"abcdefghij"[..]).unwrap();
    writer.commit().unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.range_path(0, 0))
        .unwrap()
        .set_len(4)
        .unwrap();
    let mut range = fixture.store.read(&key, "pages", 0).unwrap().unwrap();
    // A full buffer: amortized growth would double it to 32 bytes.
    let mut buffer = vec![b'x'; 16];
    let error = range.read_to_end(&mut buffer).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(&buffer[..16], &[b'x'; 16]);
    assert_eq!(&buffer[16..], b"abcd");
    // The recorded length is reserved exactly, never a doubling past it.
    assert!(buffer.capacity() <= 16 + 10, "{}", buffer.capacity());
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
    // The scope's one key check and manifest read happened when it opened;
    // its ranges share one pack handle and one entry log handle.
    assert_eq!(
        work,
        counters::Counters {
            pack_opens: 1,
            log_opens: 1,
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

/// The producers a group commit joins, each with its own sections.
const PRODUCERS: [&str; 3] = ["a", "b", "c"];

/// Publishes two equal ranges of each producer's old section.
fn producers_prefix(fixture: &Fixture) -> SourceKey {
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    for producer in PRODUCERS {
        for range in 0..2 {
            writer
                .write(
                    &format!("{producer}-old"),
                    format!("{producer} old {range}").as_bytes(),
                )
                .unwrap();
        }
    }
    writer.commit().unwrap();
    key
}

/// One producer's two batches in its own sections: a replaced and an
/// appended old range, then a new section with a replaced range, read back.
fn produce(writer: &mut Writer, producer: &str) {
    let (old, new) = (format!("{producer}-old"), format!("{producer}-new"));
    writer
        .write_at(&old, 0, format!("{producer} old 0 v2").as_bytes())
        .unwrap();
    assert_eq!(writer.write(&old, producer.as_bytes()).unwrap(), 2);
    writer.end_batch();
    writer.write(&new, &b"new 0"[..]).unwrap();
    writer.write(&new, &b"new 1"[..]).unwrap();
    writer
        .write_at(&new, 1, format!("{producer} new 1 v2").as_bytes())
        .unwrap();
    assert_eq!(
        bytes(writer.read_at(&new, 1).unwrap().unwrap()),
        format!("{producer} new 1 v2").as_bytes()
    );
}

/// Each published section's ranges by name, whatever the manifest order.
fn by_name(published: &Published) -> BTreeMap<String, Vec<Vec<u8>>> {
    published
        .0
        .iter()
        .map(|(name, _)| name.clone())
        .zip(published.1.iter().cloned())
        .collect()
}

#[test]
fn producers_adopted_into_one_commit_publish_what_their_own_commits_publish() {
    // Alone: each producer forks, writes and commits in turn.
    let alone = Fixture::new();
    let key = producers_prefix(&alone);
    let (mut created, mut syncs, mut change) = (0, 0, 0);
    for producer in PRODUCERS {
        let mut writer = alone.store.extend(&key).unwrap();
        let mut fork = writer.fork().unwrap();
        produce(&mut fork, producer);
        writer.adopt_all(fork.into_staged().unwrap()).unwrap();
        change += writer.payload_bytes_change();
        CREATED.set(0);
        SYNCS.set(0);
        writer.commit().unwrap();
        created += CREATED.get();
        syncs += SYNCS.get();
    }
    let expected = published(&alone, &key);
    // Each commit creates its entry log and the root's temporary, flushing
    // the log, the names it appends and the root before and after its
    // rename; each producer flushed its own pack as it handed it over.
    assert_eq!((created, syncs), (3 * 2, 3 * 4));

    // Together: the producers write at once on threads of their own and are
    // adopted, in another order than they ran alone, into one commit.
    let together = Fixture::new();
    let key = producers_prefix(&together);
    let mut writer = together.store.extend(&key).unwrap();
    let mut forks: Vec<_> = PRODUCERS.map(|_| writer.fork().unwrap()).into();
    std::thread::scope(|scope| {
        for (fork, producer) in forks.iter_mut().zip(PRODUCERS) {
            scope.spawn(move || produce(fork, producer));
        }
    });
    let mut staged: Vec<_> = forks
        .into_iter()
        .map(|fork| fork.into_staged().unwrap())
        .collect();
    for batches in [staged.remove(2), staged.remove(0), staged.remove(0)] {
        assert_eq!(batches.len(), 2);
        writer.adopt_all(batches).unwrap();
    }
    assert_eq!(writer.payload_bytes_change(), change);
    CREATED.set(0);
    SYNCS.set(0);
    writer.commit().unwrap();
    // One root, one entry log and one flush set for every producer.
    assert_eq!((CREATED.get(), SYNCS.get()), (2, 4));
    let actual = published(&together, &key);
    assert_eq!(by_name(&actual), by_name(&expected));
    // Range numbering is each section's own; only where sections new to the
    // manifest are listed follows which producers committed together, and
    // every lookup is by name.
    assert_eq!(actual.0[..3], expected.0[..3]);
    let names = |published: &Published| -> BTreeSet<String> {
        published.0.iter().map(|(name, _)| name.clone()).collect()
    };
    assert_eq!(names(&actual), names(&expected));
    for producer in PRODUCERS {
        let read = |section: &str, range| {
            bytes(
                together
                    .store
                    .read(&key, &format!("{producer}-{section}"), range)
                    .unwrap()
                    .unwrap(),
            )
        };
        assert_eq!(read("old", 0), format!("{producer} old 0 v2").as_bytes());
        assert_eq!(read("old", 2), producer.as_bytes());
        assert_eq!(read("new", 1), format!("{producer} new 1 v2").as_bytes());
    }
}

#[test]
fn a_refused_producer_is_taken_back_whole_and_the_others_commit_together() {
    // The reference: only the producers that are not refused, alone.
    let reference = Fixture::new();
    let key = producers_prefix(&reference);
    for producer in ["a", "c"] {
        let mut writer = reference.store.extend(&key).unwrap();
        let mut fork = writer.fork().unwrap();
        produce(&mut fork, producer);
        writer.adopt_all(fork.into_staged().unwrap()).unwrap();
        writer.commit().unwrap();
    }
    let expected = published(&reference, &key);

    let fixture = Fixture::new();
    let key = producers_prefix(&fixture);
    let mut writer = fixture.store.extend(&key).unwrap();
    let (mut a, mut refused, mut c) = (
        writer.fork().unwrap(),
        writer.fork().unwrap(),
        writer.fork().unwrap(),
    );
    produce(&mut a, "a");
    produce(&mut c, "c");
    // The refused producer's first batch creates sections, appends, replaces
    // a range it appended and one published; its second reads a range the
    // first producer replaced, so adoption refuses it after the first.
    refused.write("refused-own", &b"own 0"[..]).unwrap();
    refused.write("refused-own", &b"own 1"[..]).unwrap();
    refused
        .write_at("refused-own", 1, &b"own 1 v2"[..])
        .unwrap();
    refused.write_at("b-old", 1, &b"b old 1 v2"[..]).unwrap();
    refused.write("b-old", &b"b old 2"[..]).unwrap();
    refused.end_batch();
    refused.write("refused-later", &b"later"[..]).unwrap();
    assert_eq!(
        bytes(refused.read_at("a-old", 0).unwrap().unwrap()),
        b"a old 0"
    );
    writer.adopt_all(a.into_staged().unwrap()).unwrap();
    let adopted = (writer.bytes_written(), writer.payload_bytes_change());
    let batches = refused.into_staged().unwrap();
    assert_eq!(batches.len(), 2);
    let error = writer.adopt_all(batches).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other, "{error}");
    // Nothing of it stays listed: its sections, ranges and bytes are gone.
    assert_eq!(
        (writer.bytes_written(), writer.payload_bytes_change()),
        adopted
    );
    assert!(writer.read_at("refused-own", 0).unwrap().is_none());
    assert!(writer.read_at("b-old", 2).unwrap().is_none());
    assert_eq!(
        bytes(writer.read_at("b-old", 1).unwrap().unwrap()),
        b"b old 1"
    );
    writer.adopt_all(c.into_staged().unwrap()).unwrap();
    // The refusal left the transaction publishable.
    writer.commit().unwrap();
    let actual = published(&fixture, &key);
    assert_eq!(by_name(&actual), by_name(&expected));
    assert_eq!(actual.0, expected.0);
    // A batch of an earlier transaction is refused before anything is listed.
    let mut earlier = fixture.store.extend(&key).unwrap();
    let mut late = earlier.fork().unwrap();
    late.write("late", &b"late"[..]).unwrap();
    let late = late.into_staged().unwrap();
    earlier.commit().unwrap();
    let mut next = fixture.store.extend(&key).unwrap();
    assert_eq!(
        next.adopt_all(late).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    next.commit().unwrap();
    assert_eq!(by_name(&published(&fixture, &key)), by_name(&expected));
}

/// Deterministic pseudo-random numbers.
struct Sequence(u64);

impl Sequence {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound.max(1)
    }
}

/// Every range of every section the store lists, read through one session,
/// against what was written there last.
fn assert_reads(fixture: &Fixture, key: &SourceKey, model: &BTreeMap<String, Vec<Vec<u8>>>) {
    let session = fixture.store.session(key).unwrap().unwrap();
    for (section, ranges) in model {
        assert_eq!(
            session.range_count(section),
            Some(ranges.len() as u64),
            "{section}"
        );
        for (range, expected) in ranges.iter().enumerate() {
            let read = session.read(section, range as u64).unwrap().unwrap();
            assert_eq!(&bytes(read), expected, "{section} {range}");
        }
        assert!(
            session
                .read(section, ranges.len() as u64)
                .unwrap()
                .is_none()
        );
    }
    let catalog = fixture.store.catalog().unwrap().unwrap();
    assert_eq!(catalog.header.sections.len(), model.len());
    for runs in &catalog.runs {
        assert!(runs.len() <= MAX_RUNS, "{runs:?}");
    }
    for log in &catalog.logs {
        assert!(log.live * 2 >= log.size, "{log:?}");
    }
}

#[test]
fn reads_after_many_commits_of_appends_and_replacements_return_every_write() {
    // What every range read returns is what was last written there, as the
    // flat manifest listed it: the same sections, counts and bytes, through
    // appends, tail, middle and first-range replacements, empty ranges, new
    // sections, forks, extended and appended transactions.
    let fixture = Fixture::new();
    let mut key = fixture.key();
    let mut model: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
    let mut sequence = Sequence(7);
    let mut held = Vec::new();
    for commit in 0..120u64 {
        let mut writer = if commit == 0 {
            fixture.store.replace(key.clone()).unwrap()
        } else if commit % 40 == 0 {
            // The recording grew: the prefix carries on under a new key.
            fs::write(
                &fixture.recording,
                "synthetic recording ".repeat((commit / 40) as usize),
            )
            .unwrap();
            let grown = fixture.key();
            let writer = fixture.store.append(&key, grown.clone()).unwrap();
            key = grown;
            writer
        } else {
            fixture.store.extend(&key).unwrap()
        };
        let mut fork = (commit % 3 == 1).then(|| writer.fork().unwrap());
        for step in 0..=sequence.below(12) {
            // Forks write their own sections; new sections appear over time.
            let (target, prefix) = match &mut fork {
                Some(fork) if step % 2 == 1 => (fork, "forked"),
                _ => (&mut writer, "section"),
            };
            let section = format!("{prefix}-{}", sequence.below(4 + commit / 15));
            let ranges = model.entry(section.clone()).or_default();
            let payload = if sequence.below(5) == 0 {
                Vec::new()
            } else {
                format!("{section} {commit} {step}").into_bytes()
            };
            let count = ranges.len() as u64;
            let range = match sequence.below(4) {
                0 if count > 0 => count - 1,
                1 if count > 0 => sequence.below(count),
                2 if count > 0 => 0,
                _ => count,
            };
            target
                .write_at(&section, range, payload.as_slice())
                .unwrap();
            match ranges.get_mut(range as usize) {
                Some(slot) => *slot = payload,
                None => ranges.push(payload),
            }
        }
        // A section that grows by many ranges at every commit.
        for _ in 0..=sequence.below(20) {
            let ranges = model.entry("series".to_owned()).or_default();
            let payload = format!("series {}", ranges.len()).into_bytes();
            writer.write("series", payload.as_slice()).unwrap();
            ranges.push(payload);
        }
        if let Some(fork) = fork {
            writer.adopt_all(fork.into_staged().unwrap()).unwrap();
        }
        writer.commit().unwrap();
        if commit % 10 == 9 {
            assert_reads(&fixture, &key, &model);
            // A range held now keeps its bytes through every later commit.
            let section = format!("section-{}", sequence.below(4));
            if let Some(ranges) = model.get(&section).filter(|ranges| !ranges.is_empty()) {
                let range = sequence.below(ranges.len() as u64);
                let read = fixture.store.read(&key, &section, range).unwrap().unwrap();
                held.push((read, ranges[range as usize].clone()));
            }
        }
    }
    assert_reads(&fixture, &key, &model);
    assert!(model["series"].len() > 500);
    for (read, expected) in held {
        assert_eq!(bytes(read), expected);
    }
    // A snapshot lists the same entries without the store's names or logs.
    let listing = Listing::open(&fixture.store.directory.join(MANIFEST))
        .unwrap()
        .unwrap();
    let snapshot = fixture.root.join("snapshot");
    let mut file = File::create(&snapshot).unwrap();
    listing.write_snapshot(&mut file).unwrap();
    drop(file);
    let copy = Listing::open(&snapshot).unwrap().unwrap();
    assert!(copy.sections().eq(listing.sections()));
    for (index, (_, count)) in listing.sections().enumerate() {
        for range in 0..count {
            assert_eq!(
                copy.range(index, range).unwrap(),
                listing.range(index, range).unwrap()
            );
        }
    }
}

#[test]
fn compaction_keeps_every_entry_and_empties_logs_under_half_live() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let page = |range: u64, round: u64| format!("page {range} round {round}").into_bytes();
    let mut expected: Vec<_> = (0..200).map(|range| page(range, 0)).collect();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    for payload in &expected {
        writer.write("pages", payload.as_slice()).unwrap();
    }
    writer.write("other", &b"other"[..]).unwrap();
    writer.commit().unwrap();
    let first = fixture.store.catalog().unwrap().unwrap();
    assert_eq!(first.runs[0].len(), 1);
    let first_log = first.runs[0][0].log;
    let held = fixture.store.read(&key, "pages", 3).unwrap().unwrap();
    for round in 1..=5 {
        // Every seventh range from a new start: 29 spans split the pages'
        // runs past what a section keeps, so they are rewritten as one.
        let mut writer = fixture.store.extend(&key).unwrap();
        for range in (round - 1..200).step_by(7) {
            writer
                .write_at("pages", range, page(range, round).as_slice())
                .unwrap();
            expected[range as usize] = page(range, round);
        }
        writer.commit().unwrap();
        let catalog = fixture.store.catalog().unwrap().unwrap();
        assert_eq!(catalog.runs[0].len(), 1, "round {round}");
        // The first log kept only the other section's entry: under half
        // live, it was emptied into this commit's log.
        assert!(catalog.logs.iter().all(|log| log.id != first_log));
        let model = BTreeMap::from([
            ("other".to_owned(), vec![b"other".to_vec()]),
            ("pages".to_owned(), expected.clone()),
        ]);
        assert_reads(&fixture, &key, &model);
    }
    // The next writer open removed the emptied logs; the held range kept
    // its bytes.
    drop(fixture.store.extend(&key).unwrap());
    let catalog = fixture.store.catalog().unwrap().unwrap();
    let logs: Vec<_> = fixture
        .names()
        .iter()
        .filter_map(|name| parse_log(name))
        .collect();
    assert_eq!(logs.len(), catalog.logs.len());
    assert!(!logs.contains(&first_log));
    assert_eq!(bytes(held), page(3, 0));
}

#[test]
fn termination_between_the_entry_log_and_the_root_opens_at_the_previous_commit() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("pages", &b"page 0"[..]).unwrap();
    writer.write("pages", &b"page 1"[..]).unwrap();
    writer.commit().unwrap();
    let root = fs::read(fixture.store.directory.join(MANIFEST)).unwrap();
    let names = fixture.store.catalog().unwrap().unwrap().names;
    let logs = |fixture: &Fixture| {
        fixture
            .names()
            .iter()
            .filter(|name| parse_log(name).is_some())
            .count()
    };
    let mut writer = fixture.store.extend(&key).unwrap();
    writer.write_at("pages", 1, &b"page 1 v2"[..]).unwrap();
    writer.write("pages", &b"page 2"[..]).unwrap();
    writer
        .write("lost-section-with-a-long-name", &b"lost"[..])
        .unwrap();
    TERMINATE_BEFORE_ROOT.set(true);
    let terminated = writer.commit();
    TERMINATE_BEFORE_ROOT.set(false);
    terminated.unwrap_err();
    // Its log and the names it appended are durable; no root lists them.
    assert_eq!(
        fs::read(fixture.store.directory.join(MANIFEST)).unwrap(),
        root
    );
    assert_eq!(logs(&fixture), 2);
    let names_path = fixture.store.directory.join(names_name(names.0));
    assert!(fs::metadata(&names_path).unwrap().len() > names.1);
    let previous = BTreeMap::from([(
        "pages".to_owned(),
        vec![b"page 0".to_vec(), b"page 1".to_vec()],
    )]);
    assert_reads(&fixture, &key, &previous);
    assert!(
        fixture
            .store
            .read(&key, "lost-section-with-a-long-name", 0)
            .unwrap()
            .is_none()
    );
    // The next writer removes the log; its names go over the ones left
    // past the root's.
    let mut writer = fixture.store.extend(&key).unwrap();
    assert_eq!(logs(&fixture), 1);
    writer.write("kept", &b"kept"[..]).unwrap();
    writer.commit().unwrap();
    let catalog = fixture.store.catalog().unwrap().unwrap();
    assert_eq!(catalog.names.0, names.0);
    assert_eq!(fs::metadata(&names_path).unwrap().len(), catalog.names.1);
    let mut current = previous;
    current.insert("kept".to_owned(), vec![b"kept".to_vec()]);
    assert_reads(&fixture, &key, &current);
}

#[test]
fn manifest_bytes_per_commit_follow_what_it_changed_not_what_the_store_holds() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    // 20,000 ranges that later commits never touch: 480 KB of entries,
    // which the flat manifest rewrote at every commit.
    for _ in 0..20_000 {
        writer.write("history", io::empty()).unwrap();
    }
    writer.write("page", &b"page 0"[..]).unwrap();
    writer.commit().unwrap();
    let history = fixture.store.catalog().unwrap().unwrap().runs[0].clone();
    let (mut most, mut total) = (0, 0);
    let mut series = Vec::new();
    for commit in 0..100u64 {
        let mut writer = fixture.store.extend(&key).unwrap();
        let payload = format!("series range {commit:>8}").into_bytes();
        writer.write("series", payload.as_slice()).unwrap();
        series.push(payload);
        writer
            .write_at("page", 0, format!("page {commit}").as_bytes())
            .unwrap();
        MANIFEST_BYTES.set(0);
        writer.commit().unwrap();
        let written = MANIFEST_BYTES.get();
        (most, total) = (most.max(written), total + written);
        let catalog = fixture.store.catalog().unwrap().unwrap();
        // The history's entries stay where the first commit wrote them.
        assert_eq!(catalog.runs[0], history, "commit {commit}");
        // An appended section's runs halve towards its end.
        assert!(catalog.runs[2].len() as u64 <= commit.max(1).ilog2() as u64 + 2);
    }
    // Each commit wrote its root (a record per section, run, log and pack)
    // and its changed entries, never the history's.
    assert!(most < 32 * 1024, "{most}");
    assert!(total < 100 * 8 * 1024, "{total}");
    let model = BTreeMap::from([
        ("history".to_owned(), vec![Vec::new(); 20_000]),
        ("page".to_owned(), vec![b"page 99".to_vec()]),
        ("series".to_owned(), series),
    ]);
    assert_reads(&fixture, &key, &model);
}

#[test]
fn scattered_replacements_commit_in_one_pass_over_the_runs() {
    // 100,000 published ranges, then every odd ordinal replaced in one
    // transaction: 50,000 spans over one run. Laying each span over the runs
    // one at a time visits ever more runs (about 2.5 billion here); one
    // ordered pass visits each run piece and span once and rewrites the
    // section as one run once it passes the runs a section keeps.
    let fixture = Fixture::new();
    let key = fixture.key();
    const RANGES: u64 = 100_000;
    let mut expected: Vec<Vec<u8>> = (0..RANGES).map(|range| vec![(range % 251) as u8]).collect();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    for payload in &expected {
        writer.write("pages", payload.as_slice()).unwrap();
    }
    writer.commit().unwrap();
    let mut writer = fixture.store.extend(&key).unwrap();
    for range in (1..RANGES).step_by(2) {
        let payload = format!("odd {range}").into_bytes();
        writer.write_at("pages", range, payload.as_slice()).unwrap();
        expected[range as usize] = payload;
    }
    RUN_VISITS.set(0);
    MANIFEST_BYTES.set(0);
    writer.commit().unwrap();
    // 50,000 kept pieces and 50,000 spans, each visited once.
    assert!(RUN_VISITS.get() <= RANGES as usize, "{}", RUN_VISITS.get());
    // The spans' entries and the rewritten section, each written once, and
    // the root.
    assert!(
        MANIFEST_BYTES.get() <= (2 * RANGES * ENTRY_BYTES) as usize + 64 * 1024,
        "{}",
        MANIFEST_BYTES.get()
    );
    let catalog = fixture.store.catalog().unwrap().unwrap();
    assert_eq!(catalog.runs[0].len(), 1);
    let model = BTreeMap::from([("pages".to_owned(), expected)]);
    assert_reads(&fixture, &key, &model);
}

#[test]
fn writer_range_id_tracks_current_slots_and_fork_dependencies_without_payload_reads() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    assert_eq!(writer.range_id("page", 0).unwrap(), None);
    writer.write("page", b"first".as_slice()).unwrap();
    let first = writer.range_id("page", 0).unwrap().unwrap();
    assert_eq!(
        writer.pack.as_ref().unwrap().flushed,
        0,
        "identity lookup does not flush staged bytes"
    );
    assert_eq!(writer.read_at("page", 0).unwrap().unwrap().id(), first);
    writer.commit().unwrap();
    let mut writer = fixture.store.extend(&key).unwrap();
    assert_eq!(writer.range_id("page", 0).unwrap(), Some(first));
    let mut fork = writer.fork().unwrap();
    assert_eq!(fork.range_id("page", 0).unwrap(), Some(first));
    assert_eq!(fork.range_id("page", 1).unwrap(), None);
    writer.write_at("page", 0, b"other".as_slice()).unwrap();
    assert_ne!(writer.range_id("page", 0).unwrap(), Some(first));
    assert!(writer.adopt_all(fork.into_staged().unwrap()).is_err());
}

/// A detached batch crosses successive family publications, preserving its
/// snapshot and nested flushes while keeping the single writer lock held.
#[test]
fn detached_batch_joins_a_later_commit_with_its_nested_forks_and_exact_pages() {
    let fixture = Fixture::new();
    let key = forked_prefix(&fixture);
    let mut writer = fixture.store.extend(&key).unwrap();
    writer.write("parent", &b"parent pending"[..]).unwrap();
    let mut walk = writer.stage().unwrap();
    let first = walk.range_id("old", 0).unwrap().unwrap();
    assert_eq!(
        bytes(walk.read_at("parent", 0).unwrap().unwrap()),
        b"parent pending"
    );
    let mut nested = walk.fork().unwrap();
    nested.write_at("old", 0, &b"walk replacement"[..]).unwrap();
    nested.end_batch();
    nested.write("walk-new", &b"nested page"[..]).unwrap();
    for batch in nested.into_staged().unwrap() {
        walk.adopt(batch).unwrap();
    }
    let mut nested = walk.fork().unwrap();
    nested
        .write_at("walk-new", 0, &b"nested revised"[..])
        .unwrap();
    walk.adopt_all(nested.into_staged().unwrap()).unwrap();
    assert_eq!(
        bytes(walk.read_at("old", 0).unwrap().unwrap()),
        b"walk replacement"
    );
    walk.write("walk-new", &b"walk page"[..]).unwrap();
    let unlisted = walk.pack.as_ref().unwrap().id;
    assert!(staging(&fixture.store.directory));
    // These changes would otherwise compact the old pack, moving the identity
    // the walk read. Both family commits remain visible before walk adoption.
    writer
        .write_at("old", 1, &b"family changed old 1"[..])
        .unwrap();
    writer
        .write_at("other", 0, &b"family changed other"[..])
        .unwrap();
    writer.write("family", &b"first"[..]).unwrap();
    writer = writer.commit_continue().unwrap();
    assert_eq!(writer.range_id("old", 0).unwrap(), Some(first));
    assert!(fixture.store.directory.join(pack_name(unlisted)).is_file());
    assert!(fixture.store.read(&key, "walk-new", 0).unwrap().is_none());
    assert_eq!(
        bytes(fixture.store.read(&key, "family", 0).unwrap().unwrap()),
        b"first"
    );
    writer.write("family", &b"second"[..]).unwrap();
    writer = writer.commit_continue().unwrap();
    assert_eq!(writer.range_id("old", 0).unwrap(), Some(first));
    assert_eq!(fixture.store.range_count(&key, "family").unwrap(), Some(2));
    assert!(
        fixture.store.clear().is_err(),
        "the exclusive lock never opened a gap"
    );
    writer.adopt_all(walk.into_staged().unwrap()).unwrap();
    assert!(!staging(&fixture.store.directory));
    writer.commit().unwrap();
    let model: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::from([
        (
            "old".into(),
            vec![
                b"walk replacement".to_vec(),
                b"family changed old 1".to_vec(),
            ],
        ),
        ("other".into(), vec![b"family changed other".to_vec()]),
        ("parent".into(), vec![b"parent pending".to_vec()]),
        ("family".into(), vec![b"first".to_vec(), b"second".to_vec()]),
        (
            "walk-new".into(),
            vec![b"nested revised".to_vec(), b"walk page".to_vec()],
        ),
    ]);
    assert_eq!(by_name(&published(&fixture, &key)), model);
}

#[test]
fn detached_reads_and_replacements_refuse_overlap_without_partial_adoption() {
    for read in [false, true] {
        let fixture = Fixture::new();
        let key = forked_prefix(&fixture);
        let mut writer = fixture.store.extend(&key).unwrap();
        let mut walk = writer.stage().unwrap();
        walk.write("walk-first", &b"must roll back"[..]).unwrap();
        walk.end_batch();
        if read {
            assert_eq!(bytes(walk.read_at("old", 0).unwrap().unwrap()), b"old 0");
        } else {
            walk.write_at("old", 0, &b"walk replacement"[..]).unwrap();
        }
        writer
            .write_at("old", 0, &b"family replacement"[..])
            .unwrap();
        writer = writer.commit_continue().unwrap();
        let before = (writer.bytes_written(), writer.payload_bytes_change());
        let error = writer.adopt_all(walk.into_staged().unwrap()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(
            (writer.bytes_written(), writer.payload_bytes_change()),
            before
        );
        assert!(writer.read_at("walk-first", 0).unwrap().is_none());
        assert_eq!(
            bytes(writer.read_at("old", 0).unwrap().unwrap()),
            b"family replacement"
        );
        assert!(!staging(&fixture.store.directory));
        writer.commit().unwrap();
    }
}

#[test]
fn detached_batches_cannot_join_a_different_store_or_source_key() {
    for different_store in [false, true] {
        let fixture = Fixture::new();
        let other = Fixture::new();
        let key = forked_prefix(&fixture);
        let mut writer = fixture.store.extend(&key).unwrap();
        let mut walk = writer.stage().unwrap();
        walk.write("private", &b"private"[..]).unwrap();
        let staged = walk.into_staged().unwrap();
        if different_store {
            let mut target = other.store.replace(other.key()).unwrap();
            assert_eq!(
                target.adopt_all(staged).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert!(target.read_at("private", 0).unwrap().is_none());
            target.commit().unwrap();
        } else {
            // A changed binding is rejected before any operation is applied.
            writer.key = other.key();
            assert_eq!(
                writer.adopt_all(staged).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert!(writer.read_at("private", 0).unwrap().is_none());
            writer.key = key;
        }
        assert!(!staging(&fixture.store.directory));
        writer.commit().unwrap();
    }
}

#[test]
fn detached_batches_sharing_a_pack_keep_earlier_publications_live() {
    for atomic in [false, true] {
        let fixture = Fixture::new();
        let key = fixture.key();
        let mut writer = fixture.store.replace(key.clone()).unwrap();
        let mut stage = writer.stage().unwrap();
        stage.write("pages", b"one".as_slice()).unwrap();
        stage.end_batch();
        stage.write("pages", b"two".as_slice()).unwrap();
        let mut batches = stage.into_staged().unwrap().into_iter();
        let first = batches.next().unwrap();
        let pack = first.pack.as_ref().unwrap().0;
        let adopt = |writer: &mut Writer, batch| {
            if atomic {
                writer.adopt_all(vec![batch]).unwrap();
            } else {
                writer.adopt(batch).unwrap();
            }
        };
        adopt(&mut writer, first);
        writer = writer.commit_continue().unwrap();
        assert_eq!(fixture.packs()[0].live, 3);
        adopt(&mut writer, batches.next().unwrap());
        assert!(batches.next().is_none());
        drop(batches);
        writer.commit().unwrap();
        let packs = fixture.packs();
        assert_eq!(packs.len(), 1);
        assert_eq!((packs[0].id, packs[0].live, packs[0].size), (pack, 6, 6));
        let mut writer = fixture.store.extend(&key).unwrap();
        writer.write_at("pages", 0, b"new".as_slice()).unwrap();
        writer.commit().unwrap();
        // Reopen performs cleanup. The other half of the shared pack must
        // remain listed and readable even after its first range was replaced.
        drop(fixture.store.extend(&key).unwrap());
        assert!(fixture.store.directory.join(pack_name(pack)).is_file());
        assert_eq!(
            bytes(fixture.store.read(&key, "pages", 0).unwrap().unwrap()),
            b"new"
        );
        assert_eq!(
            bytes(fixture.store.read(&key, "pages", 1).unwrap().unwrap()),
            b"two"
        );
        assert_eq!(fixture.packs().iter().map(|pack| pack.live).sum::<u64>(), 6);
    }
}

#[test]
fn compacting_a_reintroduced_stage_pack_moves_its_published_ranges() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    let mut stage = writer.stage().unwrap();
    stage.write("pages", b"one".as_slice()).unwrap();
    stage.end_batch();
    stage
        .write("pages", b"discarded long range".as_slice())
        .unwrap();
    stage.write_at("pages", 1, b"two".as_slice()).unwrap();
    let mut batches = stage.into_staged().unwrap();
    let second = batches.pop().unwrap();
    let pack = second.pack.as_ref().unwrap().0;
    writer.adopt_all(batches).unwrap();
    writer = writer.commit_continue().unwrap();
    writer.adopt_all(vec![second]).unwrap();
    assert!(!staging(&fixture.store.directory));
    writer.commit().unwrap();
    drop(fixture.store.extend(&key).unwrap());
    assert!(!fixture.store.directory.join(pack_name(pack)).exists());
    assert_eq!(
        bytes(fixture.store.read(&key, "pages", 0).unwrap().unwrap()),
        b"one"
    );
    assert_eq!(
        bytes(fixture.store.read(&key, "pages", 1).unwrap().unwrap()),
        b"two"
    );
    assert_eq!(fixture.packs().iter().map(|pack| pack.live).sum::<u64>(), 6);
}

#[test]
fn a_live_stage_keeps_its_snapshot_without_retaining_unrelated_garbage() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let page = vec![0; 1024 * 1024];
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("page", page.as_slice()).unwrap();
    writer.commit().unwrap();
    let snapshot = fixture.store.catalog().unwrap().unwrap();
    let snapshot_pack = snapshot.packs[0].id;
    let snapshot_log = snapshot.logs[0].id;
    let mut writer = fixture.store.extend(&key).unwrap();
    let mut stage = writer.stage().unwrap();
    stage.write("staged", b"one".as_slice()).unwrap();
    let mut nested = stage.fork().unwrap();
    nested.write("nested", b"two".as_slice()).unwrap();
    let nested = nested.into_staged().unwrap();
    for revision in 1..=12u8 {
        let page = vec![revision; page.len()];
        // Even revisions leave two replacements dead before publication,
        // also requiring compaction of packs unrelated to the snapshot.
        for _ in 0..if revision % 2 == 0 { 3 } else { 1 } {
            writer.write_at("page", 0, page.as_slice()).unwrap();
        }
        writer = writer.commit_continue().unwrap();
        let mut pack_bytes = 0;
        let mut all_bytes = 0;
        for entry in fs::read_dir(&fixture.store.directory).unwrap() {
            let entry = entry.unwrap();
            let size = entry.metadata().unwrap().len();
            all_bytes += size;
            if entry.file_name().to_str().and_then(parse_pack).is_some() {
                pack_bytes += size;
            }
        }
        // One snapshot page, one live page, and the two pending three-byte
        // ranges. Neither the revision count nor superseded writes add space.
        let payload_bound = 2 * page.len() as u64 + 6;
        assert!(
            pack_bytes <= payload_bound,
            "revision {revision}: {pack_bytes}"
        );
        assert!(
            all_bytes <= payload_bound + 64 * 1024,
            "revision {revision}: {all_bytes}"
        );
        assert_eq!(
            bytes(fixture.store.read(&key, "page", 0).unwrap().unwrap()),
            page
        );
    }
    // The stage has not opened its old entry log or pack yet. Cleanup must
    // preserve those names, including when neither is in the current root.
    assert!(
        fixture
            .store
            .directory
            .join(log_name(snapshot_log))
            .is_file()
    );
    assert!(
        fixture
            .store
            .directory
            .join(pack_name(snapshot_pack))
            .is_file()
    );
    assert_eq!(bytes(stage.read_at("page", 0).unwrap().unwrap()), page);
    stage.adopt_all(nested).unwrap();
    // Drop the read dependency on the superseded page by abandoning this
    // stage; its ranges and snapshot files are now reclaimable as well.
    drop(stage);
    writer = writer.commit_continue().unwrap();
    assert!(
        !fixture
            .store
            .directory
            .join(pack_name(snapshot_pack))
            .exists()
    );
    assert!(
        !fixture
            .store
            .directory
            .join(log_name(snapshot_log))
            .exists()
    );
    assert_eq!(
        fixture
            .names()
            .iter()
            .filter(|name| parse_pack(name).is_some())
            .count(),
        1
    );
    assert!(writer.read_at("staged", 0).unwrap().is_none());
}

#[test]
fn snapshot_packs_do_not_delay_compaction_of_unrelated_published_packs() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut writer = fixture.store.replace(key.clone()).unwrap();
    writer.write("snapshot", &[0; 1024][..]).unwrap();
    writer.commit().unwrap();
    let mut writer = fixture.store.extend(&key).unwrap();
    let mut stage = writer.stage().unwrap();
    for value in [b"one", b"two", b"tri"] {
        writer.write("unrelated", value.as_slice()).unwrap();
    }
    let pack = writer.range_id("unrelated", 2).unwrap().unwrap().pack;
    writer = writer.commit_continue().unwrap();
    writer.write_at("unrelated", 0, io::empty()).unwrap();
    writer.write_at("unrelated", 1, io::empty()).unwrap();
    writer = writer.commit_continue().unwrap();
    assert!(!fixture.store.directory.join(pack_name(pack)).exists());
    assert_eq!(
        bytes(writer.read_at("unrelated", 2).unwrap().unwrap()),
        b"tri"
    );
    assert_eq!(
        bytes(stage.read_at("snapshot", 0).unwrap().unwrap()),
        vec![0; 1024]
    );
    assert_eq!(
        fixture.packs().iter().map(|pack| pack.size).sum::<u64>(),
        1024 + 3
    );
}

#[test]
fn ordinary_batches_cannot_cross_a_stage_boundary() {
    let fixture = Fixture::new();
    let key = forked_prefix(&fixture);
    let mut writer = fixture.store.extend(&key).unwrap();
    let mut stage = writer.stage().unwrap();
    let mut nested = stage.fork().unwrap();
    nested.write("nested", b"unpublished".as_slice()).unwrap();
    assert_eq!(
        writer
            .adopt_all(nested.into_staged().unwrap())
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    let mut sibling = writer.fork().unwrap();
    sibling.write("sibling", b"unpublished".as_slice()).unwrap();
    let sibling = sibling.into_staged().unwrap();
    writer = writer.commit_continue().unwrap();
    assert_eq!(
        stage.adopt_all(sibling).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    drop(stage);
    writer.commit().unwrap();
}
