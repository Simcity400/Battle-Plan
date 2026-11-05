#[cfg(test)]
use super::*;
#[cfg(test)]
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::{fs, path::PathBuf};

/// Committed payload bytes by section name and range.
pub type PublishedPayloads = BTreeMap<(String, u64), Vec<u8>>;

pub struct Fixture {
    pub store: store::Store,
    pub key: store::SourceKey,
    pub directory: PathBuf,
    root: PathBuf,
}
impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    pub fn new() -> Self {
        let id = uuid::Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("replay-series-{id}"));
        fs::create_dir(&root).unwrap();
        let path = root.join("recording");
        fs::write(&path, b"synthetic series input").unwrap();
        Self {
            store: store::Store::local(id).unwrap(),
            directory: PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
                .join("HOI4Replay/prepared")
                .join(id.to_string()),
            key: store::SourceKey::capture(&path, &[]).unwrap(),
            root,
        }
    }
    pub fn writer(&self) -> store::Writer {
        if self.store.saved_key().unwrap().is_some() {
            self.store.extend(&self.key).unwrap()
        } else {
            self.store.replace(self.key.clone()).unwrap()
        }
    }
    pub fn append_writer(&mut self) -> store::Writer {
        use std::io::Write;
        fs::OpenOptions::new()
            .append(true)
            .open(self.root.join("recording"))
            .unwrap()
            .write_all(b" appended tail")
            .unwrap();
        let previous = self.key.clone();
        self.key = self.key.recapture().unwrap();
        self.store.append(&previous, self.key.clone()).unwrap()
    }
    /// Every committed payload by section and range: the store's listing of
    /// its manifest names them, and each is opened through one verified
    /// session of the real store reader.
    pub fn payloads(&self) -> PublishedPayloads {
        let listing = store::Listing::open(&self.directory.join("manifest"))
            .unwrap()
            .unwrap();
        let session = self
            .store
            .session(&self.key)
            .unwrap()
            .expect("a published manifest");
        let mut payloads = BTreeMap::new();
        for (name, count) in listing.sections() {
            for range in 0..count {
                let mut bytes = Vec::new();
                session
                    .read(name, range)
                    .unwrap()
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                payloads.insert((name.to_owned(), range), bytes);
            }
        }
        payloads
    }
    #[cfg(test)]
    fn publish(&self, batch: Batch, states: &mut BTreeMap<u64, KeyState>) {
        let mut batch = batch;
        for (&key, state) in states.iter() {
            batch.register(key, state);
        }
        let mut writer = self.writer();
        *states = batch
            .write(&mut writer, "test", 0, &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
    }
}
/// Names each differing section and range with its first differing offset,
/// rather than dumping the payloads.
pub fn assert_payloads_equal(
    actual: &PublishedPayloads,
    expected: &PublishedPayloads,
    label: &str,
) {
    let keys: BTreeSet<_> = actual.keys().chain(expected.keys()).collect();
    let mut differences = Vec::new();
    for key in keys {
        let (left, right) = (actual.get(key), expected.get(key));
        if left == right {
            continue;
        }
        let first = left.zip(right).map(|(left, right)| {
            left.iter()
                .zip(right)
                .position(|(left, right)| left != right)
                .unwrap_or(left.len().min(right.len()))
        });
        differences.push(format!(
            "{} range={} lengths={:?}/{:?} first-offset={first:?}",
            key.0,
            key.1,
            left.map(Vec::len),
            right.map(Vec::len)
        ));
    }
    assert!(
        differences.is_empty(),
        "{label}:\n{}",
        differences.join("\n")
    );
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.store.clear().unwrap();
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[cfg(test)]
fn active_run_batch_bytes(fixture: &Fixture, ordinal: u64) -> Vec<u8> {
    let mut file = fixture
        .store
        .read(&fixture.key, "active-policy", ordinal)
        .unwrap()
        .unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    bytes
}

#[cfg(test)]
fn active_run_work_and_bytes(count: u64, frames: i32) -> usize {
    let dense_files = Fixture::new();
    let sparse_files = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut dense = Accumulator::default();
    let mut sparse = Accumulator::default();
    for hour in 0..=frames {
        dense.through(hour);
        sparse.advance(hour).unwrap();
        for key in 0..count {
            let value = if key == 7 && hour >= frames / 2 {
                -12_345
            } else {
                i128::from(key)
            };
            dense.observe(key, hour, Some(value), false);
        }
        if hour == 0 {
            for key in 0..count {
                sparse.observe(key, hour, Some(i128::from(key)), false);
            }
        } else if hour == frames / 2 {
            sparse.observe(7, hour, Some(-12_345), false);
        }
    }
    assert_eq!(dense.key_visits, count as usize * (frames as usize + 1));
    assert_eq!(sparse.key_visits, count as usize + 1);
    assert_eq!(sparse.state(0).unwrap().last.unwrap().last_hour, 0);
    assert_eq!(sparse.active.len(), count as usize);
    for (files, data) in [(&dense_files, &mut dense), (&sparse_files, &mut sparse)] {
        let mut writer = files.writer();
        data.flush(&mut writer, "active-policy", 0, &flag).unwrap();
        writer.commit().unwrap();
    }
    assert_eq!(sparse.key_visits, count as usize * 2 + 1);
    let bytes = active_run_batch_bytes(&sparse_files, 0);
    assert_eq!(bytes, active_run_batch_bytes(&dense_files, 0));
    assert_eq!(
        serde_json::to_value(&sparse.states).unwrap(),
        serde_json::to_value(&dense.states).unwrap()
    );
    let mut dense_reader = Reader::new(
        &dense_files.store,
        &dense_files.key,
        "active-policy".into(),
        &flag,
    )
    .unwrap();
    let mut sparse_reader = Reader::new(
        &sparse_files.store,
        &sparse_files.key,
        "active-policy".into(),
        &flag,
    )
    .unwrap();
    let keys: Vec<_> = (0..count).collect();
    assert_eq!(
        sparse_reader.values_at(&keys, frames).unwrap(),
        dense_reader.values_at(&keys, frames).unwrap()
    );
    for key in [0, 7, count - 1] {
        for width in [1, 16, frames as u32 + 1] {
            assert_eq!(
                serde_json::to_vec(
                    &sparse_reader
                        .summary(key, (0, frames), (0.0, f64::from(frames), width), 0)
                        .unwrap()
                )
                .unwrap(),
                serde_json::to_vec(
                    &dense_reader
                        .summary(key, (0, frames), (0.0, f64::from(frames), width), 0)
                        .unwrap()
                )
                .unwrap()
            );
        }
    }
    bytes.len()
}

#[test]
fn active_run_unchanged_frames_touch_only_changes_and_publish_dense_bytes() {
    let first = active_run_work_and_bytes(32, 10);
    let tenfold = active_run_work_and_bytes(32, 100);
    assert!(tenfold <= first * 10);
}

#[test]
fn active_run_observe_boundary_splits_equal_values_without_identity_break_and_seals_active_keys() {
    let mut values = Accumulator::default();
    values.advance(0).unwrap();
    values.observe(7, 0, Some(10), false);
    values.observe(8, 0, Some(10), false);
    values.advance(1).unwrap();
    values.observe_boundary(7, 1, Some(10), false, true);
    values.observe_boundary(8, 1, Some(10), false, false);
    assert!(values.active.contains(&7));
    assert!(values.active.contains(&8));
    for hour in 2..=4 {
        values.advance(hour).unwrap();
    }
    values.seal();
    assert_eq!(values.state(7).unwrap().runs, 2);
    assert_eq!(
        values.state(7).unwrap().last,
        Some(Run::new(1, 4, 10, false))
    );
    assert_eq!(
        values.pending_runs(7),
        [Run::new(0, 0, 10, true), Run::new(1, 4, 10, false)]
    );
    assert_eq!(values.state(8).unwrap().runs, 1);
    assert_eq!(
        values.state(8).unwrap().last,
        Some(Run::new(0, 4, 10, true))
    );
    assert_eq!(values.pending_runs(8), [Run::new(0, 4, 10, true)]);
    values.advance(5).unwrap();
    values.observe_boundary(7, 5, None, false, true);
    assert!(!values.active.contains(&7), "an unknown never stays active");
    values.advance(6).unwrap();
    values.seal();
    assert_eq!(values.state(7).unwrap().last.unwrap().last_hour, 4);
    assert_eq!(values.state(8).unwrap().last.unwrap().last_hour, 6);
}

#[test]
fn active_run_observe_boundary_matches_explicit_bytes_after_commits_resume_and_append() {
    let mut explicit_files = Fixture::new();
    let mut observed_files = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut explicit = Accumulator::default();
    let mut observed = Accumulator::default();
    for (batch, (first, last)) in [(0, 2), (3, 5), (6, 9)].into_iter().enumerate() {
        for hour in first..=last {
            explicit.through(hour);
            observed.advance(hour).unwrap();
            for key in [7, 8] {
                let boundary = key == 7 && matches!(hour, 2 | 3 | 7);
                explicit.push_boundary(key, Run::new(hour, hour, 10, hour == 0), boundary);
                if hour == 0 || boundary || key == 8 && hour == first {
                    observed.observe_boundary(key, hour, Some(10), false, boundary);
                }
            }
            if hour == first {
                let row = json!({"read":hour});
                explicit.event(7, hour, &row).unwrap();
                observed.event(7, hour, &row).unwrap();
            }
        }
        observed.seal();
        assert!(observed.active.contains(&7));
        assert!(observed.active.contains(&8));
        for (files, data) in [
            (&mut explicit_files, &mut explicit),
            (&mut observed_files, &mut observed),
        ] {
            let mut writer = if batch == 2 {
                files.append_writer()
            } else {
                files.writer()
            };
            data.flush(&mut writer, "active-policy", 0, &flag).unwrap();
            writer
                .write_at(
                    "boundary-checkpoint",
                    0,
                    serde_json::to_vec(data).unwrap().as_slice(),
                )
                .unwrap();
            writer.commit().unwrap();
            *data = serde_json::from_reader(std::io::BufReader::new(
                files
                    .store
                    .read(&files.key, "boundary-checkpoint", 0)
                    .unwrap()
                    .unwrap(),
            ))
            .unwrap();
        }
        assert_eq!(
            active_run_batch_bytes(&observed_files, batch as u64),
            active_run_batch_bytes(&explicit_files, batch as u64)
        );
        assert_eq!(
            serde_json::to_value(&observed.states).unwrap(),
            serde_json::to_value(&explicit.states).unwrap()
        );
        assert_eq!(observed.state(7).unwrap().runs, [2, 3, 4][batch]);
        assert_eq!(observed.state(8).unwrap().runs, 1);
        assert_eq!(observed.state(7).unwrap().last.unwrap().last_hour, last);
        assert!(
            observed.active.contains(&7),
            "resume retains active boundary values"
        );
        let mut reader = Reader::new(
            &observed_files.store,
            &observed_files.key,
            "active-policy".into(),
            &flag,
        )
        .unwrap();
        for hour in 0..=last {
            let (first, end, broken) = match hour {
                0 | 1 => (0, 1.min(last), true),
                2 => (2, 2, false),
                3..=6 => (3, 6.min(last), false),
                _ => (7, last, false),
            };
            assert_eq!(
                reader.value_at(7, hour).unwrap(),
                Some(Run::new(first, end, 10, broken))
            );
            assert_eq!(
                reader.value_at(8, hour).unwrap(),
                Some(Run::new(0, last, 10, true))
            );
        }
        let page = reader.events(&[7], 0, last, 0, 10).unwrap();
        assert_eq!(page.total, batch as u64 + 1);
        assert_eq!(
            page.rows
                .iter()
                .map(|row| (row.hour, row.ordinal))
                .collect::<Vec<_>>(),
            [(0, 0), (3, 1), (6, 2)][..=batch]
        );
    }
}

#[test]
fn active_run_gap_checkpoint_cancel_resume_append_keeps_dense_bytes_and_ordinals() {
    let mut dense_files = Fixture::new();
    let mut sparse_files = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut dense = Accumulator::default();
    let mut sparse = Accumulator::default();
    for (batch, (first, last)) in [(-3, 1), (4, 7), (8, 11)].into_iter().enumerate() {
        for hour in first..=last {
            dense.through(hour);
            sparse.advance(hour).unwrap();
            let values = [
                Some(10),
                match hour {
                    6 => Some(42),
                    7 | 8 | 11 => None,
                    9 | 10 => Some(43),
                    _ => Some(-5),
                },
            ];
            for (key, value) in values.into_iter().enumerate() {
                dense.observe(key as u64, hour, value, key == 1 && hour == 5);
                if hour == -3 || hour == 4 || key == 1 && matches!(hour, 5 | 6 | 7 | 9 | 11) {
                    sparse.observe(key as u64, hour, value, key == 1 && hour == 5);
                }
            }
            if matches!(hour, -1 | 5 | 9) {
                for duplicate in 0..2 {
                    let row = json!({"native":hour - 2,"duplicate":duplicate});
                    dense.event(2, hour, &row).unwrap();
                    sparse.event(2, hour, &row).unwrap();
                }
            }
        }
        for (files, data) in [
            (&mut dense_files, &mut dense),
            (&mut sparse_files, &mut sparse),
        ] {
            let mut writer = if batch == 2 {
                files.append_writer()
            } else {
                files.writer()
            };
            data.flush(&mut writer, "active-policy", 0, &flag).unwrap();
            writer
                .write_at(
                    "active-checkpoint",
                    0,
                    serde_json::to_vec(data).unwrap().as_slice(),
                )
                .unwrap();
            writer.commit().unwrap();
            *data = serde_json::from_reader(std::io::BufReader::new(
                files
                    .store
                    .read(&files.key, "active-checkpoint", 0)
                    .unwrap()
                    .unwrap(),
            ))
            .unwrap();
        }
        assert_eq!(
            active_run_batch_bytes(&sparse_files, batch as u64),
            active_run_batch_bytes(&dense_files, batch as u64)
        );
        assert_eq!(
            serde_json::to_value(&sparse.states).unwrap(),
            serde_json::to_value(&dense.states).unwrap()
        );
        assert_eq!(sparse.frontier, Some(last));
        let checkpoint = serde_json::to_vec(&sparse).unwrap();
        sparse.seal();
        assert_eq!(serde_json::to_vec(&sparse).unwrap(), checkpoint);
        assert!(sparse.batch.first.is_none());
        if batch == 1 {
            let mut discarded: Accumulator = serde_json::from_slice(&checkpoint).unwrap();
            discarded.advance(8).unwrap();
            discarded.observe(0, 8, Some(999), false);
            let mut writer = sparse_files.writer();
            assert_eq!(
                discarded
                    .flush(&mut writer, "active-policy", 0, &AtomicBool::new(true))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::Interrupted
            );
            drop(writer);
            assert_eq!(
                sparse_files
                    .store
                    .range_count(&sparse_files.key, "active-policy")
                    .unwrap(),
                Some(2)
            );
            sparse = serde_json::from_reader(std::io::BufReader::new(
                sparse_files
                    .store
                    .read(&sparse_files.key, "active-checkpoint", 0)
                    .unwrap()
                    .unwrap(),
            ))
            .unwrap();
            assert_eq!(sparse.frontier, Some(7));
        }
    }
    let mut reader = Reader::new(
        &sparse_files.store,
        &sparse_files.key,
        "active-policy".into(),
        &flag,
    )
    .unwrap();
    assert_eq!(
        reader.runs(0, -3, 11).unwrap(),
        vec![Run::new(-3, 1, 10, true), Run::new(4, 11, 10, true)]
    );
    assert_eq!(
        reader.runs(1, -3, 11).unwrap(),
        vec![
            Run::new(-3, 1, -5, true),
            Run::new(4, 4, -5, true),
            Run::new(5, 5, -5, true),
            Run::new(6, 6, 42, false),
            Run::new(9, 10, 43, true)
        ]
    );
    for hour in [2, 3] {
        assert!(reader.value_at(0, hour).unwrap().is_none());
    }
    for hour in [2, 3, 7, 8, 11] {
        assert!(reader.value_at(1, hour).unwrap().is_none());
    }
    let events = reader.events(&[2], -3, 11, 0, 100).unwrap();
    assert_eq!(events.total, 6);
    assert_eq!(
        events
            .rows
            .iter()
            .map(|row| (row.hour, row.ordinal))
            .collect::<Vec<_>>(),
        [(-1, 0), (-1, 1), (5, 2), (5, 3), (9, 4), (9, 5)]
    );
    let mut other = Accumulator::default();
    other.advance(11).unwrap();
    other.observe(0, 11, Some(99), false);
    other.seal();
    assert_eq!(
        other.state(0).unwrap().last,
        Some(Run::new(11, 11, 99, true)),
        "a new branch inherits no held value or run origin"
    );
}

#[test]
fn active_run_explicit_spans_events_and_dense_points_do_not_authenticate_hours() {
    let mut points = Accumulator::default();
    points.observe(1, 0, Some(10), false);
    points.observe(1, 1, None, false);
    points.through(20);
    points.event(2, 20, &json!({"read":20})).unwrap();
    points.push(3, Run::new(10, 15, 30, true));
    points.seal();
    assert!(points.active.is_empty());
    assert_eq!(
        points.state(1).unwrap().last,
        Some(Run::new(0, 0, 10, true))
    );
    assert_eq!(
        points.state(3).unwrap().last,
        Some(Run::new(10, 15, 30, true))
    );
    let mut active = Accumulator::default();
    active.advance(0).unwrap();
    active.observe(1, 0, Some(10), false);
    active.advance(1).unwrap();
    active.advance(2).unwrap();
    active.push_boundary(1, Run::new(2, 2, 10, false), true);
    active.push(3, Run::new(1, 1, 30, true));
    active.event(2, 1, &json!({"read":1})).unwrap();
    active.advance(3).unwrap();
    active.seal();
    assert!(active.active.is_empty());
    assert_eq!(active.state(1).unwrap().runs, 2);
    assert_eq!(
        active.state(1).unwrap().last,
        Some(Run::new(2, 2, 10, false))
    );
    assert_eq!(
        active.state(3).unwrap().last,
        Some(Run::new(1, 1, 30, true))
    );
    let before = serde_json::to_vec(&active).unwrap();
    let bytes = active.bytes();
    active.advance(3).unwrap();
    active.seal();
    assert_eq!(serde_json::to_vec(&active).unwrap(), before);
    assert_eq!(active.bytes(), bytes);
    assert_eq!(
        active.advance(2).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(serde_json::to_vec(&active).unwrap(), before);
    let mut extremes = Accumulator::default();
    extremes.advance(i32::MAX - 1).unwrap();
    extremes.observe(1, i32::MAX - 1, Some(10), false);
    extremes.advance(i32::MAX).unwrap();
    extremes.seal();
    assert_eq!(
        extremes.state(1).unwrap().last,
        Some(Run::new(i32::MAX - 1, i32::MAX, 10, true))
    );
}

#[test]
fn active_run_dense_observation_with_advance_preserves_existing_batch_bytes() {
    let dense_files = Fixture::new();
    let advanced_files = Fixture::new();
    let mut dense = Accumulator::default();
    let mut advanced = Accumulator::default();
    let flag = AtomicBool::new(false);
    for hour in [0, 1, 2, 4, 5] {
        dense.through(hour);
        advanced.advance(hour).unwrap();
        for (key, value) in [Some(10), (hour != 2).then_some(20), Some(i128::from(hour))]
            .into_iter()
            .enumerate()
        {
            dense.observe(key as u64, hour, value, hour == 5);
            advanced.observe(key as u64, hour, value, hour == 5);
        }
        dense.event(3, hour, &json!({"read":hour})).unwrap();
        advanced.event(3, hour, &json!({"read":hour})).unwrap();
    }
    for (files, data) in [(&dense_files, &mut dense), (&advanced_files, &mut advanced)] {
        let mut writer = files.writer();
        data.flush(&mut writer, "active-policy", 0, &flag).unwrap();
        writer.commit().unwrap();
    }
    assert_eq!(
        active_run_batch_bytes(&advanced_files, 0),
        active_run_batch_bytes(&dense_files, 0)
    );
    assert_eq!(
        serde_json::to_value(&advanced.states).unwrap(),
        serde_json::to_value(&dense.states).unwrap()
    );
}

#[test]
fn active_run_frame_work_ignores_tenfold_inactive_history() {
    for historical in [100, 1_000] {
        let mut data = Accumulator::default();
        for key in 100..100 + historical {
            data.push(key, Run::new(-1, -1, 99, true));
        }
        data.advance(0).unwrap();
        for key in 0..16 {
            data.observe(key, 0, Some(10), false);
        }
        let visits = data.key_visits;
        for hour in 1..100 {
            data.advance(hour).unwrap();
        }
        assert_eq!(data.key_visits, visits);
        data.observe(7, 99, Some(20), false);
        assert_eq!(data.key_visits, visits + 1);
        data.seal();
        assert_eq!(data.key_visits, visits + 17);
        assert_eq!(data.state(100).unwrap().last.unwrap().last_hour, -1);
    }
}

#[test]
fn event_clusters_within_count_only_membership_and_keep_native_ordinals_across_batches() {
    use crate::products::country_groups::HourSpan;
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut data = Accumulator::default();
    for hours in [&[0, 2, 4][..], &[5, 5, 6, 8][..]] {
        for &hour in hours {
            data.event(0, hour, &json!({"hour":hour})).unwrap();
        }
        let mut writer = fixture.writer();
        data.flush(&mut writer, "members", 0, &flag).unwrap();
        writer.commit().unwrap();
        data = serde_json::from_slice(&serde_json::to_vec(&data).unwrap()).unwrap();
    }
    let mut writer = fixture.append_writer();
    for hour in [9, 10] {
        data.event(0, hour, &json!({"hour":hour})).unwrap();
    }
    data.flush(&mut writer, "members", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "members".into(), &flag).unwrap();
    let spans = [
        HourSpan {
            first_hour: 6,
            last_hour: 8,
        },
        HourSpan {
            first_hour: 2,
            last_hour: 2,
        },
        HourSpan {
            first_hour: 5,
            last_hour: 8,
        },
    ];
    let whole = reader.event_clusters(0, 0.0, 10.0, 18).unwrap();
    assert_eq!(whole.clusters[0].count, "9");
    let clipped = reader
        .event_clusters_within(0, 0.0, 10.0, 18, &spans)
        .unwrap();
    assert_eq!(clipped.clusters.len(), 1);
    let cluster = &clipped.clusters[0];
    assert_eq!((cluster.first_hour, cluster.last_hour), (2, 8));
    assert_eq!(cluster.first_ordinal, "1");
    assert_eq!(
        cluster.count, "5",
        "outside spans and overlapping membership are never counted"
    );
    assert!(
        reader
            .event_clusters_within(0, 0.0, 10.0, 18, &[])
            .unwrap()
            .clusters
            .is_empty()
    );
    let fraction = reader
        .event_clusters_within(0, 2.25, 5.75, 18, &spans)
        .unwrap();
    assert_eq!(fraction.clusters[0].count, "2");
    assert_eq!(fraction.clusters[0].first_ordinal, "3");
    assert_eq!(
        (
            fraction.clusters[0].first_hour,
            fraction.clusters[0].last_hour
        ),
        (5, 5)
    );
}

#[test]
fn navigation_joins_keys_and_sections_with_clipped_edges_and_unknown_stops() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    let mut ranks = Accumulator::default();
    for (first, last) in [(0, 3), (4, 8)] {
        values.through(first);
        ranks.through(first);
        values.push(1, Run::new(first, last, 10, false));
        if first == 0 {
            ranks.push(2, Run::new(0, 2, 1, false));
            ranks.push(2, Run::new(3, 3, 2, false));
        } else {
            ranks.push(2, Run::new(4, 5, 2, false));
            ranks.push(2, Run::new(7, 8, 3, false));
        }
        values.through(last);
        ranks.through(last);
        let mut writer = fixture.writer();
        values.flush(&mut writer, "values", 0, &flag).unwrap();
        ranks.flush(&mut writer, "ranks", 0, &flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
        ranks = serde_json::from_slice(&serde_json::to_vec(&ranks).unwrap()).unwrap();
    }
    let mut values = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let mut ranks = Reader::new(&fixture.store, &fixture.key, "ranks".into(), &flag).unwrap();
    for (key, from, expected) in [
        (MeasureKey::Home, None, 1.5),
        (MeasureKey::End, None, 7.0),
        (MeasureKey::ArrowRight, Some(1.5), 3.0),
        (MeasureKey::ArrowLeft, Some(7.0), 6.0),
        (MeasureKey::ArrowRight, Some(5.0), 6.0),
        (MeasureKey::ArrowRight, Some(6.0), 7.0),
        (MeasureKey::ArrowLeft, Some(3.0), 1.5),
        (MeasureKey::ArrowRight, Some(8.0), 8.0),
        (MeasureKey::ArrowLeft, Some(1.5), 1.5),
    ] {
        assert_eq!(
            navigate(
                &mut [
                    (&mut values, vec![(1, None)]),
                    (&mut ranks, vec![(2, None)])
                ],
                &Navigation {
                    from,
                    first_hour: 1.5,
                    last_hour: 8.0,
                    key
                }
            )
            .unwrap(),
            Some(expected),
            "{key:?} {from:?}"
        );
    }
}

#[test]
fn navigation_keeps_per_key_membership_edges_disjoint_spans_and_empty_selections() {
    use crate::products::country_groups::HourSpan;
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for (first, last, value) in [(0, 3, 10), (4, 8, 20), (9, 12, 30)] {
        values.push(7, Run::new(first, last, value, first == 0));
    }
    values.push(8, Run::new(0, 12, 40, true));
    let mut writer = fixture.writer();
    values.flush(&mut writer, "values", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let disjoint = [
        HourSpan {
            first_hour: 2,
            last_hour: 5,
        },
        HourSpan {
            first_hour: 9,
            last_hour: 10,
        },
    ];
    let middle = [HourSpan {
        first_hour: 6,
        last_hour: 7,
    }];
    for (key, from, expected) in [
        (MeasureKey::Home, None, 2.0),
        (MeasureKey::End, None, 9.0),
        (MeasureKey::ArrowRight, Some(2.0), 4.0),
        (MeasureKey::ArrowRight, Some(4.0), 6.0),
        (MeasureKey::ArrowRight, Some(6.0), 9.0),
        (MeasureKey::ArrowLeft, Some(9.0), 6.0),
        (MeasureKey::ArrowLeft, Some(6.0), 4.0),
        (MeasureKey::ArrowLeft, Some(4.0), 2.0),
        (MeasureKey::ArrowRight, Some(11.0), 11.0),
        (MeasureKey::ArrowLeft, Some(1.0), 1.0),
    ] {
        assert_eq!(
            navigate(
                &mut [(&mut reader, vec![(7, Some(&disjoint)), (8, Some(&middle))])],
                &Navigation {
                    from,
                    first_hour: 0.0,
                    last_hour: 12.0,
                    key
                },
            )
            .unwrap(),
            Some(expected),
            "{key:?} {from:?}"
        );
    }
    for (keys, first_hour, last_hour, key, from, expected) in [
        (
            vec![(7, Some(disjoint.as_slice())), (8, Some([].as_slice()))],
            2.5,
            9.5,
            MeasureKey::Home,
            None,
            Some(2.5),
        ),
        (
            vec![(7, Some(disjoint.as_slice()))],
            0.0,
            12.0,
            MeasureKey::ArrowRight,
            Some(4.0),
            Some(9.0),
        ),
        (
            vec![(7, Some(disjoint.as_slice())), (8, None)],
            0.0,
            12.0,
            MeasureKey::Home,
            None,
            Some(0.0),
        ),
        (
            vec![(7, Some([].as_slice()))],
            0.0,
            12.0,
            MeasureKey::Home,
            None,
            None,
        ),
        (
            vec![(7, Some([].as_slice()))],
            0.0,
            12.0,
            MeasureKey::End,
            None,
            None,
        ),
        (
            vec![(7, Some([].as_slice()))],
            0.0,
            12.0,
            MeasureKey::ArrowRight,
            Some(20.0),
            Some(12.0),
        ),
        (
            vec![(7, Some([].as_slice()))],
            0.0,
            12.0,
            MeasureKey::ArrowLeft,
            Some(-1.0),
            Some(0.0),
        ),
    ] {
        assert_eq!(
            navigate(
                &mut [(&mut reader, keys)],
                &Navigation {
                    from,
                    first_hour,
                    last_hour,
                    key
                },
            )
            .unwrap(),
            expected,
            "{key:?} {from:?}"
        );
    }
}

#[test]
fn accumulator_preserves_supplying_read_boundaries_and_empty_flush_after_resume_append() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for (index, (first, last)) in [(0, 2), (3, 5), (6, 8)].into_iter().enumerate() {
        for hour in first..=last {
            values.push_boundary(
                7,
                Run::new(hour, hour, 10, hour == 0),
                hour == 2 || hour == 6,
            );
            if hour == first {
                values.event(7, hour, &json!({"read":hour})).unwrap();
            }
        }
        let mut writer = if index == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        values.flush(&mut writer, "values", 0, &flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
        let before = serde_json::to_value(&values).unwrap();
        let mut writer = fixture.writer();
        values.flush(&mut writer, "values", 0, &flag).unwrap();
        writer.commit().unwrap();
        assert_eq!(
            serde_json::to_value(&values).unwrap(),
            before,
            "empty flush discarded open state"
        );
    }
    let state = values.state(7).unwrap();
    assert_eq!((state.runs, state.events), (3, 3));
    assert_eq!(
        fixture.store.range_count(&fixture.key, "values").unwrap(),
        Some(3)
    );
    let mut reader = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    for (first, last, broken) in [(0, 1, true), (2, 5, false), (6, 8, false)] {
        for hour in first..=last {
            assert_eq!(
                reader.value_at(7, hour).unwrap(),
                Some(Run::new(first, last, 10, broken))
            );
        }
    }
    let page = reader.events(&[7], 0, 8, 0, 10).unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(
        page.rows
            .iter()
            .map(|row| (row.hour, row.ordinal))
            .collect::<Vec<_>>(),
        [(0, 0), (3, 1), (6, 2)]
    );
}

#[test]
fn pending_runs_complete_committed_history_without_overlap() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for hour in 0..=3 {
        values.observe(1, hour, Some(10), false);
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "values", 0, &flag).unwrap();
    writer.commit().unwrap();
    assert!(values.pending_runs(1).is_empty());
    for (hour, value) in [(4, 10), (5, 10), (6, 20)] {
        values.observe(1, hour, Some(value), false);
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let committed = reader.runs(1, 0, 6).unwrap();
    assert_eq!(
        committed
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value))
            .collect::<Vec<_>>(),
        [(0, 3, 10)]
    );
    assert_eq!(
        values
            .pending_runs(1)
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value))
            .collect::<Vec<_>>(),
        [(4, 5, 10), (6, 6, 20)]
    );
    assert!(values.pending_runs(2).is_empty());
}

#[test]
fn canonical_join_is_equal_contiguous_and_without_a_break() {
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(7, KeyState::default())]);
    for (first, last, value, broken) in [
        (0, 2, 9, true),
        (3, 4, 9, false),
        (5, 7, 9, true),
        (9, 10, 9, false),
        (11, 12, 8, false),
    ] {
        let mut batch = Batch::default();
        batch.through(first);
        batch.through(last);
        batch.push(
            7,
            states.get_mut(&7).unwrap(),
            Run::new(first, last, value, broken),
        );
        fixture.publish(batch, &mut states);
        // The persisted state grows with keys, never with batches or run history.
        states = serde_json::from_slice(&serde_json::to_vec(&states).unwrap()).unwrap();
    }
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    for hour in 0..5 {
        let run = reader.value_at(7, hour).unwrap().unwrap();
        assert_eq!(
            (run.first_hour, run.last_hour, run.value, run.break_before),
            (0, 4, 9, true)
        );
    }
    let run = reader.value_at(7, 6).unwrap().unwrap();
    assert_eq!(
        (run.first_hour, run.last_hour, run.break_before),
        (5, 7, true)
    );
    assert!(reader.value_at(7, 8).unwrap().is_none());
    assert_eq!(reader.value_at(7, 9).unwrap().unwrap().last_hour, 10);
    assert_eq!(reader.next_change(7, 2.0, true).unwrap(), Some(5));
    assert_eq!(reader.next_change(7, 7.0, true).unwrap(), Some(8));
    assert_eq!(reader.next_change(7, 8.0, true).unwrap(), Some(9));
    assert_eq!(reader.next_change(7, 9.0, false).unwrap(), Some(8));
    assert_eq!(reader.next_change(7, 5.0, false).unwrap(), Some(0));
    assert!(reader.value_at(999, 2).unwrap().is_none());
}

#[test]
fn selected_spans_preserve_extrema_rename_continuity_and_excluded_hours() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut batch = Accumulator::default();
    batch.through(0);
    batch.through(63);
    batch.push(1, Run::new(0, 15, 2, true));
    batch.push(1, Run::new(16, 63, 999, false));
    batch.push(2, Run::new(16, 31, -4, true));
    batch.push(2, Run::new(32, 63, 8, false));
    let mut writer = fixture.writer();
    batch.flush(&mut writer, "test", 0, &flag).unwrap();
    let states = serde_json::to_value(&batch).unwrap();
    batch.flush(&mut writer, "test", 0, &flag).unwrap();
    assert_eq!(
        serde_json::to_value(&batch).unwrap(),
        states,
        "empty commits retain continuation counters"
    );
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let masks = [
        (1, 0, 15, Some(true)),
        (2, 16, 31, Some(false)),
        (2, 40, 63, Some(true)),
    ];
    let dense = reader
        .summary_spans(&masks, (0, 63), (0.0, 63.0, 1), 0)
        .unwrap();
    let bucket = dense.buckets[0].unwrap();
    assert_eq!((bucket.min_value, bucket.max_value), (-4.0, 8.0));
    assert_eq!(bucket.absolute_total, 288);
    assert!(bucket.has_break);
    let joined = reader
        .summary_spans(&masks[..2], (0, 63), (0.0, 31.0, 1), 0)
        .unwrap();
    assert!(
        !joined.buckets[0].unwrap().has_break,
        "the known alias change continues"
    );
    let exact = reader
        .summary_spans(&masks, (0, 63), (14.25, 40.75, 100), 0)
        .unwrap();
    assert_eq!(
        exact
            .runs
            .unwrap()
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value, run.break_before))
            .collect::<Vec<_>>(),
        [
            (0, 15, 2.0, true),
            (16, 31, -4.0, false),
            (40, 63, 8.0, true)
        ]
    );
    assert_eq!(reader.next_change(2, 31.0, true).unwrap(), Some(32));
    assert_eq!(reader.next_change(2, 16.0, false).unwrap(), None);
}

#[test]
fn summaries_preserve_extrema_unknown_spans_and_boundary_segments() {
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(0, KeyState::default())]);
    let mut expected = Vec::new();
    for (first, last) in [(-7, 2), (3, 17), (18, 81), (82, 130)] {
        let mut batch = Batch::default();
        batch.through(first);
        batch.through(last);
        for hour in first..=last {
            let value = match hour {
                1 | 16 | 83 => None,
                64 => Some(999),
                65 => Some(-999),
                _ => Some(i128::from(hour % 5)),
            };
            if let Some(value) = value {
                batch.push(
                    0,
                    states.get_mut(&0).unwrap(),
                    Run::new(hour, hour, value, hour == 17),
                );
            }
            expected.push((hour, value));
        }
        fixture.publish(batch, &mut states);
    }
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    for (from, to, width) in [
        (-5.25, 125.75, 150),
        (-5.25, 125.75, 19),
        (-7.0, 130.0, 2),
        (64.0, 65.0, 100),
    ] {
        let summary = reader.summary(0, (-7, 130), (from, to, width), 0).unwrap();
        assert!(summary.buckets.len() <= 2 * width as usize + 2);
        // Independent hourly fold over the exact requested display bins.
        let count = to.floor() as i64 - from.floor().max(-7.0) as i64 + 1;
        let per_pixel = (count / i64::from(width)).max(1);
        let fine = per_pixel <= 16;
        let size = if fine {
            1_i64 << (63 - (per_pixel as u64).leading_zeros())
        } else {
            (count as u64).div_ceil(u64::from(width)) as i64
        };
        assert_eq!(
            summary.runs.is_some(),
            size == 1,
            "only hourly summaries carry original runs"
        );
        let mut start = from.floor().max(-7.0) as i32;
        for bucket in summary.buckets {
            let end = if fine {
                -7 + ((i64::from(start) + 7) / size + 1) * size - 1
            } else {
                i64::from(start) + size - 1
            }
            .min(to.floor() as i64) as i32;
            let mut wanted = None;
            let mut initialized = false;
            for &(hour, value) in expected
                .iter()
                .filter(|&&(hour, _)| hour >= start && hour <= end)
            {
                let next = value.map(|value| ChartBucket::hour(hour, value as f64, hour == 17));
                wanted = if initialized {
                    merge(wanted, next)
                } else {
                    next
                };
                initialized = true;
            }
            assert_eq!(
                bucket, wanted,
                "{from}..{to} width {width}, bin {start}..{end}"
            );
            start = end + 1;
        }
    }
}

#[test]
fn every_range_folds_runs_crossing_batches_as_their_hours_do_without_rebuilding_earlier_runs() {
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(0, KeyState::default())]);
    // (first, last, value, break, a supplying boundary), by batch: runs continue
    // across batch ends, one batch's last run ends before the batch does.
    let batches: [((i32, i32), &[(i32, i32, i128, bool, bool)]); 3] = [
        (
            (0, 40),
            &[
                (0, 9, 3, true, false),
                (10, 25, 3, false, true),
                (26, 40, 5, false, false),
            ],
        ),
        (
            (41, 90),
            &[
                (41, 47, 5, false, false),
                (53, 70, -2, true, false),
                (71, 90, 4, false, false),
            ],
        ),
        (
            (91, 150),
            &[
                (91, 95, 4, false, false),
                (96, 112, 9, true, false),
                (113, 129, -6, false, false),
                (130, 140, 1, false, false),
            ],
        ),
    ];
    let mut hours = BTreeMap::new();
    for ((first, last), runs) in batches {
        let mut batch = Batch::default();
        batch.through(first);
        batch.through(last);
        for &(from, to, value, starts, boundary) in runs {
            let state = states.get_mut(&0).unwrap();
            batch.push_boundary(0, state, Run::new(from, to, value, starts), boundary);
            for hour in from..=to {
                hours.insert(hour, (value, starts && hour == from));
            }
        }
        fixture.publish(batch, &mut states);
    }
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let before = RUN_BOUNDS.get();
    reader.range_bucket(0, 1, 30, 0).unwrap();
    assert_eq!(RUN_BOUNDS.get(), before, "no run before a batch's last is rebuilt");
    for from in 0..=150 {
        for to in from..=150 {
            // Independent hourly fold: an unknown hour is a missing child.
            let mut wanted = None;
            for hour in from..=to {
                let next = hours
                    .get(&hour)
                    .map(|&(value, starts)| ChartBucket::hour(hour, value as f64, starts));
                wanted = if hour == from {
                    next
                } else {
                    merge(wanted, next)
                };
            }
            coverage_break(&mut wanted, (from, to));
            assert_eq!(
                reader.range_bucket(0, from, to, 0).unwrap(),
                wanted,
                "{from}..={to}"
            );
        }
    }
}

#[test]
fn a_value_whose_run_ends_inside_an_aligned_piece_begins_where_it_began() {
    // Each last value begins before a 16-hour boundary and its run ends inside a later
    // piece: in one batch, and with a batch ending inside that piece.
    type Layout<'a> = &'a [((i32, i32), &'a [(i32, i32, i128)])];
    let layouts: [Layout<'_>; 2] = [
        &[(
            (0, 255),
            &[(0, 89, 1), (90, 100, 2), (140, 170, 3), (171, 200, 4)],
        )],
        &[
            ((0, 97), &[(0, 89, 1), (90, 97, 2)]),
            ((98, 255), &[(98, 100, 2), (140, 170, 3), (171, 200, 4)]),
        ],
    ];
    for layout in layouts {
        let fixture = Fixture::new();
        let mut states = BTreeMap::from([(0, KeyState::default())]);
        let mut hours = BTreeMap::new();
        for &((first, last), runs) in layout {
            let mut batch = Batch::default();
            batch.through(first);
            batch.through(last);
            for &(from, to, value) in runs {
                let state = states.get_mut(&0).unwrap();
                batch.push(0, state, Run::new(from, to, value, false));
                for hour in from..=to {
                    hours.insert(hour, value);
                }
            }
            fixture.publish(batch, &mut states);
        }
        let flag = AtomicBool::new(false);
        let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
        for (from, to, began) in [(64, 127, 90), (160, 223, 171), (0, 255, 171)] {
            let bucket = reader.range_bucket(0, from, to, 0).unwrap().unwrap();
            assert_eq!((bucket.last_value_hour, bucket.has_break), (began, true));
        }
        for from in (0..=255).step_by(3) {
            for to in from..=255 {
                // Independent hourly fold: an unknown hour is a missing child.
                let mut wanted = None;
                for hour in from..=to {
                    let next = hours
                        .get(&hour)
                        .map(|&value| ChartBucket::hour(hour, value as f64, false));
                    wanted = if hour == from {
                        next
                    } else {
                        merge(wanted, next)
                    };
                }
                coverage_break(&mut wanted, (from, to));
                assert_eq!(
                    reader.range_bucket(0, from, to, 0).unwrap(),
                    wanted,
                    "{from}..={to}"
                );
            }
        }
    }
}

#[test]
fn a_gap_ends_where_another_batch_owns_the_hour_however_batch_bounds_overlap() {
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(0, KeyState::default())]);
    // The second batch lies inside the first: from hour 6 the third owns the hours.
    for (first, last, run) in [(0, 10, None), (1, 5, None), (6, 20, Some((6, 20, 7)))] {
        let mut batch = Batch::default();
        batch.through(first);
        batch.through(last);
        if let Some((from, to, value)) = run {
            let state = states.get_mut(&0).unwrap();
            batch.push(0, state, Run::new(from, to, value, true));
        }
        fixture.publish(batch, &mut states);
    }
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let mut wanted = ChartBucket::hour(6, 7.0, true);
    wanted.absolute_total = 35;
    wanted.last_hour = 10;
    wanted.has_break = true;
    assert_eq!(reader.range_bucket(0, 1, 10, 0).unwrap(), Some(wanted));
}

#[test]
fn a_fold_of_many_bins_looks_up_its_batch_once() {
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(0, KeyState::default())]);
    let mut batch = Batch::default();
    batch.through(0);
    batch.through(255);
    for (from, to, value) in [(0, 9, 1), (10, 99, 2), (100, 255, 3)] {
        let state = states.get_mut(&0).unwrap();
        batch.push(0, state, Run::new(from, to, value, false));
    }
    fixture.publish(batch, &mut states);
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let before = BLOB_LOOKUPS.get();
    // Eight-hour bins: no stored level covers one, so each folds from the batch's runs.
    let summary = reader.summary(0, (0, 255), (0.0, 255.0, 32), 0).unwrap();
    assert_eq!(summary.buckets.len(), 32);
    assert_eq!(BLOB_LOOKUPS.get() - before, 1, "one lookup for 32 bins");
}

#[test]
fn derived_tail_matches_published_runs_with_continuations_gaps_breaks_and_membership() {
    use crate::products::country_groups::HourSpan;
    let flag = AtomicBool::new(false);
    for internal_gap in [false, true] {
        for native_break in [false, true] {
            let prefix = if internal_gap {
                vec![Run::new(-7, 3, 900, true), Run::new(5, 107, 900, false)]
            } else {
                vec![Run::new(-7, 107, 900, true)]
            };
            let tail = if native_break {
                vec![
                    Run::new(108, 112, 900, false),
                    Run::new(113, 116, -200, false),
                    Run::new(118, 125, 700, true),
                    Run::new(126, 130, 700, false),
                ]
            } else {
                vec![Run::new(108, 130, 900, false)]
            };
            let pending = Fixture::new();
            let published = Fixture::new();
            for (fixture, include_tail) in [(&pending, false), (&published, true)] {
                let mut states = BTreeMap::from([(7, KeyState::default())]);
                let mut batch = Batch::default();
                batch.through(-7);
                batch.through(107);
                for &run in &prefix {
                    batch.push(7, states.get_mut(&7).unwrap(), run);
                }
                fixture.publish(batch, &mut states);
                if include_tail {
                    let mut batch = Batch::default();
                    batch.through(108);
                    batch.through(130);
                    for &run in &tail {
                        batch.push(7, states.get_mut(&7).unwrap(), run);
                    }
                    fixture.publish(batch, &mut states);
                }
            }
            let mut pending =
                Reader::new(&pending.store, &pending.key, "test".into(), &flag).unwrap();
            let mut published =
                Reader::new(&published.store, &published.key, "test".into(), &flag).unwrap();
            let membership = [
                HourSpan {
                    first_hour: -4,
                    last_hour: 109,
                },
                HourSpan {
                    first_hour: 112,
                    last_hour: 119,
                },
                HourSpan {
                    first_hour: 121,
                    last_hour: 130,
                },
            ];
            for within in [None, Some(membership.as_slice())] {
                for view in [
                    (-7.0, 130.0, 1),
                    (-5.25, 130.0, 5),
                    (106.25, 129.75, 200),
                    (109.0, 111.0, 100),
                    (-2.0, 2.0, 100),
                    (133.0, 140.0, 1),
                ] {
                    let actual = pending
                        .summary_with_tail(7, (-7, 130), view, 0, &tail, within)
                        .unwrap();
                    let expected = published
                        .summary_within(7, (-7, 130), view, 0, within)
                        .unwrap();
                    assert_eq!(
                        actual, expected,
                        "gap={internal_gap}, break={native_break}, view={view:?}, membership={within:?}"
                    );
                    assert_eq!(
                        pending
                            .summary_with_tail(7, (-7, 130), view, 0, &[], within)
                            .unwrap(),
                        pending
                            .summary_within(7, (-7, 130), view, 0, within)
                            .unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn sparse_tail_keeps_a_settled_point_when_its_prefix_bucket_crosses_publication() {
    let pending = Fixture::new();
    let published = Fixture::new();
    let flag = AtomicBool::new(false);
    let tail = [Run::new(41, 41, 23, false)];
    for (fixture, with_tail) in [(&pending, false), (&published, true)] {
        let mut batch = Batch::default();
        let mut state = KeyState::default();
        batch.through(0);
        batch.through(if with_tail { 50 } else { 19 });
        batch.push(0, &mut state, Run::new(17, 17, 23, false));
        if with_tail {
            batch.push(0, &mut state, tail[0]);
        }
        let mut writer = fixture.writer();
        batch.write(&mut writer, "sparse", 0, &flag).unwrap();
        writer.commit().unwrap();
    }
    let mut pending = Reader::new(&pending.store, &pending.key, "sparse".into(), &flag).unwrap();
    let mut published =
        Reader::new(&published.store, &published.key, "sparse".into(), &flag).unwrap();
    for view in [(0.0, 50.0, 1), (0.0, 50.0, 100), (16.0, 50.0, 1)] {
        assert_eq!(
            pending
                .summary_with_tail(0, (0, 50), view, 0, &tail, None)
                .unwrap(),
            published.summary(0, (0, 50), view, 0).unwrap(),
            "view={view:?}"
        );
    }
}

#[test]
fn events_keep_duplicate_order_every_row_and_exact_screen_counts() {
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(0, KeyState::default()), (1, KeyState::default())]);
    for first in (0..600).step_by(100) {
        let mut batch = Batch::default();
        batch.through(first);
        batch.through(first + 99);
        for hour in first..first + 100 {
            for duplicate in 0..3 {
                batch
                    .event(
                        0,
                        states.get_mut(&0).unwrap(),
                        hour,
                        &json!({"hour":hour,"duplicate":duplicate}),
                    )
                    .unwrap();
            }
        }
        fixture.publish(batch, &mut states);
    }
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let clusters = reader.event_clusters(0, 3.25, 598.75, 99).unwrap();
    assert_eq!(
        clusters
            .clusters
            .iter()
            .map(|cluster| cluster.count.parse::<u64>().unwrap())
            .sum::<u64>(),
        595 * 3
    );
    let mut cursor = 0;
    let mut rows = Vec::new();
    loop {
        let page = reader.events(&[0], i32::MIN, 599, cursor, 32).unwrap();
        assert_eq!(page.total, 1800);
        rows.extend(page.rows.into_iter().map(|row| row.value()));
        if let Some(next) = page.cursor {
            cursor = next;
        } else {
            break;
        }
    }
    assert_eq!(rows.len(), 1800);
    for (index, row) in rows.into_iter().enumerate() {
        assert_eq!(row, json!({"hour":index / 3,"duplicate":index % 3}));
    }
    let page = reader.events(&[0], 598, 599, 0, 32).unwrap();
    assert_eq!((page.first, page.total, page.rows.len()), (1794, 6, 6));
    assert_eq!(reader.events(&[1], i32::MIN, 599, 0, 32).unwrap().total, 0);
    flag.store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(
        reader.events(&[0], 0, 599, 0, 32).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn events_reject_an_ordinal_count_their_batch_does_not_hold() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for key in [1, 2] {
        values.event(key, 0, &json!({"key": key})).unwrap();
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "events", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
    reader.data(0, 1).unwrap();
    // Make the directory's ordinal count disagree with the two stored rows:
    // the page then claims a row its batch does not hold.
    reader.directories.get_mut(&(0, 1)).unwrap().event_base += 1;
    let error = reader.events(&[1, 2], i32::MIN, 0, 0, 10).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "prepared event ordinal is outside its batch"
    );
}

#[test]
fn empty_batches_complete_partial_summaries_without_losing_known_values() {
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(0, KeyState::default())]);
    for (first, last, known) in [
        (0, 6, true),
        (7, 15, false),
        (16, 31, false),
        (32, 47, true),
    ] {
        let mut batch = Batch::default();
        batch.through(first);
        batch.through(last);
        if known {
            batch.push(
                0,
                states.get_mut(&0).unwrap(),
                Run::new(first, last, 9, first == 0),
            );
        }
        fixture.publish(batch, &mut states);
    }
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let summary = reader.summary(0, (0, 47), (0.0, 47.0, 1), 0).unwrap();
    let bucket = summary.buckets[0].unwrap();
    assert_eq!(
        (
            bucket.first_hour,
            bucket.last_hour,
            bucket.min_value,
            bucket.max_value,
            bucket.has_break
        ),
        (0, 47, 9.0, 9.0, true)
    );
    let summary = reader.summary(0, (0, 47), (0.0, 15.0, 1), 0).unwrap();
    let bucket = summary.buckets[0].unwrap();
    assert_eq!(
        (bucket.first_hour, bucket.last_hour, bucket.has_break),
        (0, 6, true)
    );
    assert!(reader.value_at(0, 20).unwrap().is_none());
}

#[test]
fn multi_key_events_resume_in_hour_key_and_duplicate_order() {
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(2, KeyState::default()), (9, KeyState::default())]);
    let mut batch = Batch::default();
    batch.through(0);
    batch.through(19);
    for hour in 0..20 {
        for key in [2, 9] {
            for duplicate in 0..3 {
                batch
                    .event(
                        key,
                        states.get_mut(&key).unwrap(),
                        hour,
                        &json!([hour, key, duplicate]),
                    )
                    .unwrap();
            }
        }
    }
    fixture.publish(batch, &mut states);
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let mut cursor = 0;
    let mut rows = Vec::new();
    loop {
        let page = reader.events(&[9, 2, 9], 1, 18, cursor, 7).unwrap();
        rows.extend(page.rows);
        if let Some(next) = page.cursor {
            cursor = next;
        } else {
            break;
        }
    }
    assert_eq!(rows.len(), 108);
    for (index, row) in rows.into_iter().enumerate() {
        let hour = index / 6 + 1;
        let key = if index % 6 < 3 { 2 } else { 9 };
        assert_eq!(row.value(), json!([hour, key, index % 3]));
        assert_eq!(
            (row.hour, row.key, row.ordinal),
            (hour as i32, key, hour as u64 * 3 + index as u64 % 3)
        );
    }
}

#[test]
fn events_within_page_only_each_keys_own_hours_in_merged_order() {
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(2, KeyState::default()), (9, KeyState::default())]);
    let mut batch = Batch::default();
    batch.through(0);
    batch.through(19);
    for hour in 0..20 {
        for key in [2, 9] {
            for duplicate in 0..3 {
                batch
                    .event(
                        key,
                        states.get_mut(&key).unwrap(),
                        hour,
                        &json!([hour, key, duplicate]),
                    )
                    .unwrap();
            }
        }
    }
    fixture.publish(batch, &mut states);
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let spans = [(2, 3, 4), (9, 4, 5), (2, 10, 10)];
    let mut cursor = 0;
    let mut rows = Vec::new();
    loop {
        let page = reader.events_within(&spans, cursor, 4).unwrap();
        assert_eq!(page.total, 15);
        rows.extend(page.rows.into_iter().map(|row| (row.hour, row.key, row.ordinal)));
        match page.cursor {
            Some(next) => cursor = next,
            None => break,
        }
    }
    let expected: Vec<(i32, u64, u64)> = [(3, 2), (4, 2), (4, 9), (5, 9), (10, 2)]
        .into_iter()
        .flat_map(|(hour, key): (i32, u64)| {
            (0..3_u64).map(move |duplicate| (hour, key, hour as u64 * 3 + duplicate))
        })
        .collect();
    assert_eq!(rows, expected);
}

#[test]
fn hourly_dense_runs_use_compact_key_blobs_and_keep_wide_values_exact() {
    let fixture = Fixture::new();
    let mut states: BTreeMap<_, _> = (0..1500).map(|key| (key, KeyState::default())).collect();
    let mut batch = Batch::default();
    batch.through(0);
    batch.through(511);
    for (&key, state) in &mut states {
        for hour in 0..512 {
            batch.push(
                key,
                state,
                Run::new(
                    hour,
                    hour,
                    i128::from(key) * 100_000 + i128::from(hour) * 17,
                    hour == 0,
                ),
            );
        }
    }
    fixture.publish(batch, &mut states);
    let length = fixture
        .store
        .read(&fixture.key, "test", 0)
        .unwrap()
        .unwrap()
        .len();
    println!(
        "dense series: {length} bytes for 768,000 hourly runs ({:.3} bytes/run)",
        length as f64 / 768_000.0
    );
    assert!(
        length < 1500 * 512 * 4,
        "{length} bytes for 768,000 changing values"
    );
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    assert_eq!(
        reader.value_at(1499, 511).unwrap().unwrap().value,
        1499 * 100_000 + 511 * 17
    );
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(0, KeyState::default())]);
    let mut batch = Batch::default();
    batch.through(-3);
    batch.through(0);
    for (hour, value) in [(-3, i128::MIN), (-2, i128::MAX), (-1, 0), (0, -1)] {
        batch.push(
            0,
            states.get_mut(&0).unwrap(),
            Run::new(hour, hour, value, hour == -3),
        );
    }
    fixture.publish(batch, &mut states);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    for (hour, value) in [(-3, i128::MIN), (-2, i128::MAX), (-1, 0), (0, -1)] {
        assert_eq!(reader.value_at(0, hour).unwrap().unwrap().value, value);
    }
}

#[test]
fn equal_series_payloads_compress_once_without_changing_batch_bytes() {
    let count = 5_000;
    let mut bytes = vec![0; count * DIRECTORY as usize];
    let mut expected = bytes.clone();
    let mut blobs = HashMap::new();
    let before = BLOB_WORK.with(|work| *work.borrow());
    let mut total = 0;
    for key in 0..count {
        // Many countries share a value's runs and summaries; a few differ.
        let raw = serde_json::to_vec(&(key % 4, vec![17; 24])).unwrap();
        total += raw.len();
        let at = key * DIRECTORY as usize + 8;
        let compressed = compress(&raw, 3).unwrap();
        let offset = expected.len() as u64;
        put(&mut expected, at, &offset.to_le_bytes());
        put(
            &mut expected,
            at + 8,
            &(compressed.len() as u64).to_le_bytes(),
        );
        expected.extend(compressed);
        blob(&mut bytes, at, raw, &mut blobs).unwrap();
    }
    let after = BLOB_WORK.with(|work| *work.borrow());
    let compressed = after.0 - before.0;
    let reused = after.1 - before.1;
    assert_eq!(compressed + reused, total);
    assert_eq!(blobs.len(), 4);
    assert!(
        compressed * 1_000 <= total,
        "{compressed} bytes compressed of {total}"
    );
    assert_eq!(bytes, expected);
}

#[test]
fn series_flush_reuses_equal_payloads_after_restore_and_append() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for batch in 0..3 {
        let first = batch * 16;
        values.advance(first).unwrap();
        for key in 0..128 {
            values.observe(key, first, Some(i128::from(key % 2)), batch == 0);
        }
        values.advance_span(first + 1, first + 15).unwrap();
        let mut writer = if batch == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        let before = BLOB_WORK.with(|work| *work.borrow());
        let built = PAYLOAD_WORK.with(std::cell::Cell::get);
        values.flush(&mut writer, "repeated", 0, &flag).unwrap();
        let after = BLOB_WORK.with(|work| *work.borrow());
        assert!(
            after.1 > before.1 || PAYLOAD_WORK.with(std::cell::Cell::get) - built < 128,
            "batch {batch} reused no compressed bytes"
        );
        writer.commit().unwrap();
        let state = serde_json::to_vec(&values).unwrap();
        values = serde_json::from_slice(&state).unwrap();
        assert_eq!(serde_json::to_vec(&values).unwrap(), state);
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "repeated".into(), &flag).unwrap();
    for key in 0..128 {
        for hour in [0, 15, 16, 31, 32, 47] {
            assert_eq!(
                reader.value_at(key, hour).unwrap().unwrap().value,
                i128::from(key % 2)
            );
        }
    }
}

#[test]
fn whole_replies_are_plain_compressed_json_and_replace_only_the_requested_ordinal() {
    let fixture = Fixture::new();
    for value in ["first", "appended"] {
        let mut writer = fixture.writer();
        write_reply(&mut writer, "results", 0, &value).unwrap();
        writer.commit().unwrap();
    }
    let mut writer = fixture.writer();
    write_reply(&mut writer, "results", 1, &vec![1, 2, 3]).unwrap();
    writer.commit().unwrap();
    assert_eq!(
        read_reply::<String>(&fixture.store, &fixture.key, "results", 0)
            .unwrap()
            .as_deref(),
        Some("appended")
    );
    assert!(
        read_reply::<String>(&fixture.store, &fixture.key, "results", 2)
            .unwrap()
            .is_none()
    );
    let file = fixture
        .store
        .read(&fixture.key, "results", 0)
        .unwrap()
        .unwrap();
    let raw = zstd::stream::decode_all(file).unwrap();
    assert_eq!(raw, serde_json::to_vec("appended").unwrap());
}

#[test]
fn absolute_contribution_is_exact_across_batches_resume_append_gaps_and_membership() {
    let mut fixture = Fixture::new();
    let mut states = BTreeMap::from([(7, KeyState::default())]);
    for (index, (first, last, value, broken)) in [
        (0, 7, i128::from(i64::MIN), true),
        (8, 19, i128::from(i64::MAX), false),
        (23, 33, 24 * i128::from(i64::MAX), false),
    ]
    .into_iter()
    .enumerate()
    {
        let mut batch = Batch::default();
        batch.through(first);
        batch.through(last);
        batch.push(
            7,
            states.get_mut(&7).unwrap(),
            Run::new(first, last, value, broken),
        );
        if index == 2 {
            for (&key, state) in &states {
                batch.register(key, state);
            }
            let mut writer = fixture.append_writer();
            states = batch
                .write(&mut writer, "test", 0, &AtomicBool::new(false))
                .unwrap();
            writer.commit().unwrap();
        } else {
            fixture.publish(batch, &mut states);
        }
        states = serde_json::from_slice(&serde_json::to_vec(&states).unwrap()).unwrap();
    }
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    for (from, to, width) in [(0., 33., 1), (0., 33., 100), (4., 29., 2), (16., 31., 1)] {
        for spans in [
            None,
            Some(vec![
                crate::products::country_groups::HourSpan {
                    first_hour: 5,
                    last_hour: 10,
                },
                crate::products::country_groups::HourSpan {
                    first_hour: 24,
                    last_hour: 30,
                },
            ]),
        ] {
            let expected: u128 = (from as i32..=to as i32)
                .filter(|hour| {
                    spans.as_ref().is_none_or(|spans| {
                        spans
                            .iter()
                            .any(|span| span.first_hour <= *hour && *hour <= span.last_hour)
                    })
                })
                .map(|hour| match hour {
                    0..=7 => i128::from(i64::MIN).unsigned_abs(),
                    8..=19 => i128::from(i64::MAX).unsigned_abs(),
                    23..=33 => (24 * i128::from(i64::MAX)).unsigned_abs(),
                    _ => 0,
                })
                .sum();
            let summary = reader
                .summary_within(7, (0, 33), (from, to, width), 0, spans.as_deref())
                .unwrap();
            assert_eq!(
                summary
                    .buckets
                    .iter()
                    .flatten()
                    .map(|bucket| bucket.absolute_total)
                    .sum::<u128>(),
                expected
            );
            let json = serde_json::to_value(&summary).unwrap();
            let totals = json["buckets"]["totals"].as_array().unwrap();
            assert_eq!(totals.len(), summary.buckets.iter().flatten().count());
            assert!(totals.iter().all(serde_json::Value::is_string));
            assert_eq!(
                serde_json::from_value::<ChartSummary>(json).unwrap(),
                summary
            );
        }
    }
}

#[test]
fn values_at_matches_single_key_reads_across_commits_resume_append_and_gaps() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for (batch, (first, last)) in [(100, 101), (102, 103), (106, 107)].into_iter().enumerate() {
        for hour in first..=last {
            for key in 0..24 {
                let amount = (!(hour == 102 && key % 3 == 0)).then_some(match key % 4 {
                    0 if hour == 100 => i128::MIN,
                    1 if hour == 100 => i128::MAX,
                    0 => -(1_i128 << 80),
                    1 => 1_i128 << 80,
                    2 => -(key as i128),
                    _ => i128::from(hour) + key as i128,
                });
                values.observe(key, hour, amount, false);
            }
        }
        let mut writer = if batch == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        values.flush(&mut writer, "values", 0, &flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
    }
    let mut many = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let mut single = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let keys: Vec<_> = (0..=24).rev().chain([0, 1]).collect();
    assert!(many.values_at(&[], 101).unwrap().is_empty());
    for hour in [99, 100, 101, 102, 103, 104, 105, 106, 107, 108] {
        let expected: Vec<_> = keys
            .iter()
            .map(|&key| single.value_at(key, hour).unwrap().map(|run| run.value))
            .collect();
        assert_eq!(
            many.values_at(&keys, hour).unwrap(),
            expected,
            "hour {hour}"
        );
    }
}

/// 1,500 keys over two batches, a fifth of them holding nothing in each, their blobs empty.
#[cfg(test)]
fn many_key_batches(flag: &AtomicBool) -> Fixture {
    let mut fixture = Fixture::new();
    let mut values = Accumulator::default();
    for (batch, (first, last)) in [(0, 49), (50, 99)].into_iter().enumerate() {
        for hour in first..=last {
            for key in 0..1500_u64 {
                let amount = (key % 5 != batch as u64)
                    .then(|| i128::from(hour / (1 + key as i32 % 7)) + i128::from(key));
                values.observe(key, hour, amount, false);
            }
        }
        let mut writer = if batch == 1 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        values.flush(&mut writer, "values", 0, flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
    }
    fixture
}

#[test]
fn many_keys_runs_read_together_are_each_keys_runs_read_alone() {
    let flag = AtomicBool::new(false);
    let fixture = many_key_batches(&flag);
    // More keys than one read decodes together, asked out of their stored order.
    let keys: Vec<u64> = (0..1500).rev().collect();
    let open = || Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let (mut together, mut alone) = (open(), open());
    assert!(together.runs_each(&[], i32::MIN, i32::MAX).unwrap().is_empty());
    for (from, to) in [(i32::MIN, i32::MAX), (10, 60), (50, 99), (0, 0)] {
        let read = together.runs_each(&keys, from, to).unwrap();
        for (&key, runs) in keys.iter().zip(read) {
            let expected = alone.runs(key, from, to).unwrap();
            assert_eq!(runs.unwrap(), expected, "key {key} from {from} to {to}");
        }
    }
}

#[test]
fn many_keys_read_together_give_each_keys_blob_as_reading_it_alone() {
    let flag = AtomicBool::new(false);
    let fixture = many_key_batches(&flag);
    // More keys than one read decodes together, asked out of their stored order.
    let keys: Vec<u64> = (0..1500).rev().collect();
    assert!(keys.len() > DECODE_KEYS);
    let open = || Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let (mut together, mut alone) = (open(), open());
    for hour in [0, 25, 49, 50, 77, 99] {
        let batch = alone.batches(hour, hour).unwrap().next().unwrap();
        let expected: Vec<Option<i128>> = keys
            .iter()
            .map(|&key| {
                let rows = &alone.data(batch, key).unwrap().runs;
                let index = rows.partition_point(|run| run.first_hour <= hour);
                index
                    .checked_sub(1)
                    .map(|index| rows[index])
                    .filter(|run| run.last_hour >= hour)
                    .map(|run| run.value)
            })
            .collect();
        let read = together.values_at(&keys, hour).unwrap();
        assert_eq!(read, expected, "hour {hour}");
    }
}

#[test]
fn keys_read_together_decode_a_repeated_key_once_and_stop_when_cancelled() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for hour in 0..20 {
        for key in 0..3_u64 {
            let amount = i128::from(hour) * i128::from(key + 1);
            values.observe(key, hour, Some(amount), false);
        }
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "values", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let decodes = BLOB_DECODES.get();
    let read = reader.values_at(&[1, 1, 2, 1], 10).unwrap();
    assert_eq!(read, [Some(20), Some(20), Some(30), Some(20)]);
    assert_eq!(BLOB_DECODES.get() - decodes, 2, "each key decoded once");
    let cancelled = AtomicBool::new(false);
    let mut reader =
        Reader::new(&fixture.store, &fixture.key, "values".into(), &cancelled).unwrap();
    cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    let decodes = BLOB_DECODES.get();
    let error = reader.values_at(&[0, 1, 2], 10).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    let decoded = BLOB_DECODES.get() - decodes;
    assert_eq!(decoded, 0, "nothing decoded once cancelled");
}

#[test]
fn a_repeated_key_shares_its_first_read_however_little_the_reader_keeps() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for hour in 0..20 {
        for key in 0..3_u64 {
            let amount = i128::from(hour) + i128::from(key) * 100;
            values.observe(key, hour, Some(amount), false);
        }
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "values", 0, &flag).unwrap();
    writer.commit().unwrap();
    // Only the latest decoded blob is kept, so every other one is evicted at once.
    TEST_DECODED_BYTES.set(1);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let decodes = BLOB_DECODES.get();
    let read = reader.values_at(&[0, 0, 1, 2, 0], 10);
    let decoded = BLOB_DECODES.get() - decodes;
    TEST_DECODED_BYTES.set(DECODED_BYTES);
    let expected = [Some(10), Some(10), Some(110), Some(210), Some(10)];
    assert_eq!(read.unwrap(), expected);
    assert_eq!(decoded, 3, "each key decoded once");
}

#[test]
fn a_read_cancelled_part_way_reports_a_failure_found_before_it() {
    let cancelled = || io::Error::from(io::ErrorKind::Interrupted);
    let failed = Some(io::Error::from(io::ErrorKind::InvalidData));
    let refused = refusal([None, failed, None], cancelled());
    assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    let refused = refusal([None, None], cancelled());
    assert_eq!(refused.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn a_sweep_over_more_keys_than_a_count_bound_held_decodes_each_blob_once() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for hour in 100..200 {
        for key in 0..600_u64 {
            let amount = i128::from(hour / 10) + i128::from(key % 3);
            values.observe(key, hour, Some(amount), false);
        }
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "values", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let keys: Vec<u64> = (0..600).collect();
    let hours: Vec<i32> = (100..200).collect();
    let decodes = BLOB_DECODES.get();
    for window in hours.chunks(25) {
        reader.changes_at(&keys, window).unwrap();
    }
    assert_eq!(BLOB_DECODES.get() - decodes, keys.len());
}

#[test]
fn changes_at_matches_values_at_for_every_window_across_batches_and_gaps() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for (batch, (first, last)) in [(100, 299), (300, 499)].into_iter().enumerate() {
        for hour in first..=last {
            for key in 0..6_u64 {
                // Runs of differing lengths, with gaps, so most runs lie outside a window.
                let step = i32::try_from(key).unwrap() + 2;
                let amount = (hour % 37 != key as i32).then_some(i128::from(hour / step) % 5);
                values.observe(key, hour, amount, false);
            }
        }
        let mut writer = if batch == 1 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        values.flush(&mut writer, "values", 0, &flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let keys: Vec<u64> = (0..7).collect();
    let windows: [Vec<i32>; 6] = [
        (150..214).collect(),
        (95..110).collect(),
        (290..320).step_by(3).collect(),
        (480..520).collect(),
        vec![123, 250, 251, 377, 499, 500],
        (90..510).step_by(7).collect(),
    ];
    for hours in windows {
        let expected: Vec<Vec<(usize, Option<i128>)>> = keys
            .iter()
            .map(|&key| {
                let mut changes = Vec::new();
                let mut current = None;
                for (at, &hour) in hours.iter().enumerate() {
                    let value = reader.values_at(&[key], hour).unwrap()[0];
                    if value != current {
                        changes.push((at, value));
                        current = value;
                    }
                }
                changes
            })
            .collect();
        assert_eq!(
            reader.changes_at(&keys, &hours).unwrap(),
            expected,
            "{hours:?}"
        );
    }
}

#[test]
fn changes_at_probes_backdated_and_overlapping_runs_once_per_blob() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for first in [0, 20] {
        values.through(first);
        for (key, runs) in [
            vec![(0, 2, 0), (3, 5, 0), (8, 10, 7)],
            vec![(0, 2, 1), (5, 5, 2), (0, 10, 3)],
            vec![(0, 10, 1), (2, 3, 2), (2, 7, 3)],
            vec![(0, 10, 8), (2, 3, -4)],
        ]
        .into_iter()
        .enumerate()
        {
            for (from, to, value) in runs {
                values.push(key as u64, Run::new(first + from, first + to, value, true));
            }
        }
        values.through(first + 11);
        let mut writer = fixture.writer();
        values.flush(&mut writer, "values", 0, &flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
    }
    let keys = [3, 1, 0, 2, 1];
    let mut reader = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    for hours in [
        (-1..=33).collect::<Vec<_>>(),
        vec![1, 3, 5, 10, 11, 19, 20, 22, 25, 31, 32],
        (3..=25).collect(),
        Vec::new(),
    ] {
        let decodes = BLOB_DECODES.get();
        let actual = reader.changes_at(&keys, &hours).unwrap();
        if hours.first() == Some(&-1) {
            assert_eq!(BLOB_DECODES.get() - decodes, 8, "each blob decoded once");
        }
        let mut expected = vec![Vec::new(); keys.len()];
        let mut held = vec![None; keys.len()];
        for (at, &hour) in hours.iter().enumerate() {
            for (key, value) in reader
                .values_at(&keys, hour)
                .unwrap()
                .into_iter()
                .enumerate()
            {
                if held[key] != value {
                    expected[key].push((at, value));
                    held[key] = value;
                }
            }
        }
        assert_eq!(actual, expected, "{hours:?}");
        assert!(
            actual
                .iter()
                .all(|changes| changes.windows(2).all(|p| p[0].0 < p[1].0))
        );
    }
}

#[test]
fn late_index_keeps_only_late_facts_ordered_and_bounded_by_read() {
    let mut late = LateIndex::default();
    late.insert(10, 10, 1, 0);
    late.insert(12, 40, 2, 5);
    late.insert(11, 30, 3, 4);
    late.insert(12, 20, 1, 6);
    late.insert(12, 40, 2, 5);
    let all: Vec<_> = late
        .between(i32::MIN, i32::MAX, i32::MAX)
        .copied()
        .collect();
    assert_eq!(all, vec![(11, 30, 3, 4), (12, 20, 1, 6), (12, 40, 2, 5)]);
    let read_by_30: Vec<_> = late.between(11, 12, 30).copied().collect();
    assert_eq!(read_by_30, vec![(11, 30, 3, 4), (12, 20, 1, 6)]);
    assert_eq!(late.between(13, 20, i32::MAX).count(), 0);
    assert_eq!(late.between(12, 12, 39).next_back(), Some(&(12, 20, 1, 6)));
}

#[test]
fn late_ordinal_spans_coalesce_out_of_order_and_preserve_keys_holes_and_boundaries() {
    let mut late = LateIndex::default();
    for ordinal in [4, 6, 2, 3, 5, 8, 8] {
        late.insert(10, 20, 7, ordinal);
    }
    late.insert(9, 21, 7, 3); // The same ordinal is still one span member.
    late.insert(10, 20, 8, 0);
    late.insert(10, 20, 8, u64::MAX);
    assert_eq!(late.ordinals[&7], [(2, 6), (8, 8)]);
    for (ordinal, next, previous) in [
        (1, 1, 1),
        (2, 7, 1),
        (6, 7, 1),
        (7, 7, 7),
        (8, 9, 7),
        (9, 9, 9),
    ] {
        assert_eq!(late.next_ordinary(7, ordinal), Some(next));
        assert_eq!(late.previous_ordinary(7, ordinal), Some(previous));
    }
    assert_eq!(late.next_ordinary(9, 0), Some(0), "keys remain independent");
    assert_eq!(late.next_ordinary(8, 0), Some(1));
    assert_eq!(late.previous_ordinary(8, 0), None);
    assert_eq!(late.next_ordinary(8, u64::MAX), None);
    assert_eq!(late.previous_ordinary(8, u64::MAX), Some(u64::MAX - 1));
    let restored: LateIndex = serde_json::from_value(serde_json::to_value(&late).unwrap()).unwrap();
    assert_eq!(
        restored, late,
        "native-date order does not affect the rebuilt ordinal spans"
    );
}

#[test]
fn a_damaged_series_directory_count_refuses_instead_of_overflowing() {
    let fixture = Fixture::new();
    let path = fixture.root.join("damaged-batch");
    // A directory count near u64::MAX would overflow the entry offset.
    let mut bytes = vec![0_u8; 16];
    bytes[8..16].copy_from_slice(&(u64::MAX / 2).to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    let mut file = fs::File::open(&path).unwrap();
    assert!(directory(&mut file, 7).is_err());
}

#[test]
fn saved_continuations_read_back_every_float_exactly() {
    // A restored observer compares values it saved with values it computes
    // again. A widened single-precision ratio and a product of two decimals
    // both need seventeen significant digits.
    for value in [f64::from(3.0f32 / 13.0), 1.35 * 7.0] {
        let text = serde_json::to_string(&value).unwrap();
        let read: f64 = serde_json::from_str(&text).unwrap();
        assert_eq!(read.to_bits(), value.to_bits(), "{text}");
    }
}

/// Each accumulator's saved state and its published payload.
#[cfg(test)]
fn published(accumulator: &mut Accumulator) -> (Vec<u8>, PublishedPayloads) {
    let state = serde_json::to_vec(&*accumulator).unwrap();
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    accumulator
        .flush(&mut writer, "test", 0, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    (state, fixture.payloads())
}

#[test]
fn an_advance_span_equals_advancing_every_hour() {
    // Continuing, repeating the frontier, after a gap, and in a fresh batch.
    for (first, last, flushed) in [
        (3, 3, false),
        (3, 9, false),
        (2, 6, false),
        (6, 9, false),
        (3, 9, true),
    ] {
        let setup = || {
            let mut accumulator = Accumulator::default();
            accumulator.advance(1).unwrap();
            accumulator.observe(1, 1, Some(5), true);
            accumulator.advance(2).unwrap();
            accumulator.observe(2, 2, Some(6), false);
            if flushed {
                published(&mut accumulator);
            }
            accumulator
        };
        let mut hourly = setup();
        for hour in first..=last {
            hourly.advance(hour).unwrap();
        }
        hourly.observe(1, last, Some(7), false);
        let mut span = setup();
        span.advance_span(first, last).unwrap();
        span.observe(1, last, Some(7), false);
        assert_eq!(
            published(&mut span),
            published(&mut hourly),
            "{first}..={last}"
        );
    }
}

/// Calls made on a recording accumulator and made again on a carried one
/// under other keys equal the same calls made there directly.
#[test]
fn recorded_calls_made_again_on_a_carried_accumulator_equal_direct_calls() {
    struct Target<'a>(&'a mut Accumulator, [u64; 3], Vec<usize>);
    impl ReplayTarget for Target<'_> {
        fn series(&mut self, series: u8) -> &mut Accumulator {
            assert_eq!(series, 4);
            self.0
        }
        fn key(&self, _: u8, key: u64) -> u64 {
            self.1[key as usize]
        }
        fn family(&mut self, index: usize) -> io::Result<()> {
            self.2.push(index);
            self.0.observe(self.1[2], 25, None, false);
            Ok(())
        }
    }
    let history = |accumulator: &mut Accumulator| {
        for hour in 0..5 {
            accumulator.advance(hour).unwrap();
        }
        accumulator.observe(10, 1, Some(7), true);
        accumulator.observe(12, 3, Some(3), false);
    };
    let calls =
        |accumulator: &mut Accumulator, key: [u64; 3], family: &mut dyn FnMut(&mut Accumulator)| {
            for hour in 5..40 {
                if hour == 20 {
                    continue;
                }
                accumulator.advance(hour).unwrap();
                match hour {
                    6 => accumulator.observe(key[0], hour, Some(9), false),
                    12 => {
                        accumulator.observe(key[1], hour, Some(4), true);
                        accumulator
                            .event(key[0], hour, &json!({"hour": hour}))
                            .unwrap();
                    }
                    21 => accumulator.push(key[1], Run::new(hour, hour + 2, 4, false)),
                    25 => family(accumulator),
                    30 => accumulator.through(hour + 5),
                    _ => {}
                }
            }
        };
    let carried_keys = [10, 11, 12];
    let mut direct = Accumulator::default();
    history(&mut direct);
    calls(&mut direct, carried_keys, &mut |accumulator| {
        accumulator.observe(12, 25, None, false)
    });
    let mut extract = Accumulator::default();
    extract.record_as(4);
    let mut recording = Recording::default();
    with_recording(&mut recording, || {
        calls(&mut extract, [0, 1, 2], &mut |accumulator| {
            assert!(record_family(0));
            // The family's own call makes its accumulator calls unrecorded.
            unrecorded(|| accumulator.observe(2, 25, None, false));
        })
    });
    assert!(!record_family(1), "no recording outside with_recording");
    // Unchanged hours are spans: far fewer calls than hours.
    assert!(recording.len() < 20, "{} calls", recording.len());
    let mut carried = Accumulator::default();
    history(&mut carried);
    let mut target = Target(&mut carried, carried_keys, Vec::new());
    recording.replay(&mut target).unwrap();
    assert_eq!(target.2, [0]);
    assert_eq!(published(&mut carried), published(&mut direct));
}

/// A quiet span an extract advances in one call replays as every hour of it,
/// not as its first hour followed by a gap.
#[test]
fn a_recorded_advance_span_replays_every_hour_of_it() {
    struct Target<'a>(&'a mut Accumulator);
    impl ReplayTarget for Target<'_> {
        fn series(&mut self, _: u8) -> &mut Accumulator {
            self.0
        }
        fn key(&self, _: u8, key: u64) -> u64 {
            key
        }
        fn family(&mut self, _: usize) -> io::Result<()> {
            Ok(())
        }
    }
    let history = |accumulator: &mut Accumulator| {
        accumulator.advance(0).unwrap();
        accumulator.observe(1, 0, Some(5), true);
    };
    let mut direct = Accumulator::default();
    history(&mut direct);
    for hour in 1..=9 {
        direct.advance(hour).unwrap();
    }
    direct.advance(10).unwrap();
    direct.observe(1, 10, Some(6), false);
    let mut extract = Accumulator::default();
    extract.record_as(0);
    let mut recording = Recording::default();
    with_recording(&mut recording, || {
        extract.advance(1).unwrap();
        extract.advance_span(2, 9).unwrap();
        extract.advance(10).unwrap();
        extract.observe(1, 10, Some(6), false);
    });
    let mut carried = Accumulator::default();
    history(&mut carried);
    recording.replay(&mut Target(&mut carried)).unwrap();
    assert_eq!(published(&mut carried), published(&mut direct));
}

#[test]
fn first_seen_catalogues_number_carried_and_new_names_in_order() {
    let mut local = BTreeMap::new();
    for name in ["b", "a", "c"] {
        let next = local.len() as u64;
        local.entry(name.to_owned()).or_insert(next);
    }
    let names = catalogue_names(&local).unwrap();
    assert_eq!(names, ["b", "a", "c"]);
    let mut carried = BTreeMap::from([("z".to_owned(), 0), ("a".to_owned(), 1)]);
    assert_eq!(number_names(&mut carried, &names), [2, 1, 3]);
    local.insert("gap".to_owned(), 9);
    assert!(catalogue_names(&local).is_err());
}

#[test]
fn a_reader_checks_its_key_once_and_searches_batch_hours_in_memory() {
    // 64 committed batches of 16 hours each; key 0 changes every 4 hours.
    let fixture = Fixture::new();
    let mut states = BTreeMap::from([(0, KeyState::default())]);
    for batch_index in 0..64 {
        let first = batch_index * 16;
        let mut batch = Batch::default();
        batch.through(first);
        batch.through(first + 15);
        for part in 0..4 {
            let start = first + part * 4;
            batch.push(
                0,
                states.get_mut(&0).unwrap(),
                Run::new(start, start + 3, i128::from(start % 7), start == 0),
            );
        }
        fixture.publish(batch, &mut states);
    }
    let flag = AtomicBool::new(false);
    let view = (0.0, 1023.0, 300);
    let before = store::counters::now();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let summary = reader.summary(0, (0, 1023), view, 0).unwrap();
    let work = store::counters::now() - before;
    // One key check and manifest read for the whole summary; each batch's pack
    // is opened once, however many buckets search the batches.
    assert_eq!((work.stats, work.manifest_reads), (1, 1));
    assert!(work.pack_opens <= 64, "{work:?}");
    assert!(summary.buckets.len() >= 300, "{}", summary.buckets.len());
    let before = store::counters::now();
    let again = reader.summary(0, (0, 1023), view, 0).unwrap();
    assert_eq!(
        store::counters::now() - before,
        store::counters::Counters::default()
    );
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(&summary).unwrap()
    );
    // Every bucket equals one read on its own by a fresh reader.
    for (index, bucket) in summary.buckets.iter().enumerate().step_by(37) {
        let hours = (
            bucket.map(|bucket| bucket.first_hour),
            bucket.map(|bucket| bucket.last_hour),
        );
        let (Some(first), Some(last)) = hours else {
            continue;
        };
        let mut single = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
        let alone = single
            .summary(0, (0, 1023), (f64::from(first), f64::from(last), 1), 0)
            .unwrap();
        assert_eq!(
            alone.buckets[0].map(|bucket| (bucket.min_value, bucket.max_value)),
            summary.buckets[index].map(|bucket| (bucket.min_value, bucket.max_value)),
            "bucket {index}"
        );
    }
    // A store scope serves every reader of the same key without further checks.
    let scope = fixture.store.scope(&fixture.key).unwrap();
    let before = store::counters::now();
    for _ in 0..3 {
        let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
        reader.summary(0, (0, 1023), view, 0).unwrap();
    }
    let work = store::counters::now() - before;
    assert_eq!((work.stats, work.manifest_reads), (0, 0));
    drop(scope);
}

#[test]
fn constant_open_buckets_save_as_alignments_and_publish_as_whole_buckets_do() {
    // 2,000 keys: even keys hold one value; odd keys change, break, go
    // unknown and return. Three flushes with a save and restore before the
    // last, once keeping every open bucket whole (the old state) and once
    // keeping constant ones as spans.
    let observe = |accumulator: &mut Accumulator, hours: std::ops::RangeInclusive<i32>| {
        for hour in hours {
            accumulator.advance(hour).unwrap();
            for key in 0..2000_u64 {
                let (value, starts) = if key % 2 == 0 {
                    (Some(7_000_000_000_000_i128), false)
                } else if hour % 37 == 0 {
                    (None, false)
                } else {
                    (
                        Some(i128::from(hour / 3) * (key as i128 % 5 - 2)),
                        hour % 29 == 0,
                    )
                };
                accumulator.observe(key, hour, value, starts);
            }
        }
    };
    let run = |whole: bool| {
        super::WHOLE_BUCKETS.with(|cell| cell.set(whole));
        let mut accumulator = Accumulator::default();
        observe(&mut accumulator, 100..=140);
        let first = published(&mut accumulator);
        observe(&mut accumulator, 141..=190);
        let second = published(&mut accumulator);
        let saved = serde_json::to_vec(&accumulator).unwrap();
        let mut restored: Accumulator = serde_json::from_slice(&saved).unwrap();
        observe(&mut accumulator, 191..=300);
        observe(&mut restored, 191..=300);
        let continued = published(&mut accumulator).1;
        assert_eq!(
            published(&mut restored).1,
            continued,
            "restored vs continued, whole {whole}"
        );
        super::WHOLE_BUCKETS.with(|cell| cell.set(false));
        (first.1, second.1, continued, saved)
    };
    let (whole_first, whole_second, whole_third, whole_saved) = run(true);
    let (first, second, third, saved) = run(false);
    assert_eq!(first, whole_first);
    assert_eq!(second, whole_second);
    assert_eq!(third, whole_third);
    // A constant key keeps each open bucket as its alignment alone.
    let value: Value = serde_json::from_slice(&saved).unwrap();
    let partial = value["states"]["0"]["p"].as_array().unwrap();
    let open: Vec<_> = partial.iter().filter(|bucket| !bucket.is_null()).collect();
    assert!(!open.is_empty());
    assert!(
        open.iter()
            .all(|bucket| bucket.as_object().unwrap().len() == 1),
        "{open:?}"
    );
    let whole: Value = serde_json::from_slice(&whole_saved).unwrap();
    let size = |state: &Value| serde_json::to_vec(state).unwrap().len();
    let (compact, kept) = (size(&value["states"]["0"]), size(&whole["states"]["0"]));
    assert!(
        compact * 5 < kept,
        "constant key saved {compact} vs whole {kept} bytes"
    );
    assert!(saved.len() < whole_saved.len());
}

#[test]
fn equal_numeric_inputs_build_one_payload_before_summary_work() {
    let mut accumulator = Accumulator::default();
    accumulator.advance(10).unwrap();
    for key in 0..200 {
        accumulator.observe(key, 10, Some(0), false);
    }
    accumulator.advance_span(11, 1000).unwrap();
    PAYLOAD_WORK.with(|work| work.set(0));
    published(&mut accumulator);
    assert_eq!(PAYLOAD_WORK.with(std::cell::Cell::get), 1);
}

#[test]
fn reused_numeric_payloads_keep_batch_bytes_and_restored_summaries() {
    let prepare = |whole| {
        WHOLE_PAYLOADS.with(|cell| cell.set(whole));
        let mut accumulator = Accumulator::default();
        let mut results = Vec::new();
        for (first, last) in [(-7, 35), (36, 70), (72, 106)] {
            for hour in first..=last {
                accumulator.advance(hour).unwrap();
                for key in 0..128 {
                    let value = match key % 5 {
                        0 => Some(0),
                        1 => Some(i128::from(i64::MAX) + 123),
                        2 => Some(i128::from(i64::MIN) - 123),
                        3 => (hour % 11 != 0).then_some(i128::from(hour / 3)),
                        _ => Some(i128::from(hour)),
                    };
                    accumulator.observe(key, hour, value, hour % 23 == 0);
                    if key % 5 == 4 && hour % 7 == 0 {
                        accumulator.event(key, hour, &(key, hour)).unwrap();
                    }
                }
            }
            let (_, pages) = published(&mut accumulator);
            let saved = serde_json::to_vec(&accumulator).unwrap();
            results.push((pages, saved.clone()));
            accumulator = serde_json::from_slice(&saved).unwrap();
        }
        WHOLE_PAYLOADS.with(|cell| cell.set(false));
        results
    };
    assert_eq!(prepare(false), prepare(true));
}

#[test]
fn equal_observations_extend_once_and_keep_hourly_bytes() {
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut values = [Accumulator::default(), Accumulator::default()];
    let flag = AtomicBool::new(false);
    for (batch, (first, last)) in [(0, 15), (16, 31), (35, 50)].into_iter().enumerate() {
        for (index, values) in values.iter_mut().enumerate() {
            values.advance(first).unwrap();
            values.observe(7, first, Some(19), batch == 0);
            let before = OBSERVATION_WORK.with(std::cell::Cell::get);
            values.advance_span(first + 1, first + 3).unwrap();
            WHOLE_OBSERVATIONS.with(|whole| whole.set(index == 1));
            values.observe(7, first + 3, Some(19), false);
            WHOLE_OBSERVATIONS.with(|whole| whole.set(false));
            assert_eq!(
                OBSERVATION_WORK.with(std::cell::Cell::get) - before,
                index + 1
            );
            // Supplying boundaries, unknowns, reactivation and repeated hours
            // must retain the ordinary observation semantics.
            for hour in first + 4..=last {
                values.advance(hour).unwrap();
                WHOLE_OBSERVATIONS.with(|whole| whole.set(index == 1));
                values.observe_boundary(
                    7,
                    hour,
                    (hour != first + 6).then_some(19),
                    hour == first + 9,
                    hour == first + 12,
                );
                if hour == first + 8 {
                    values.observe(7, hour, Some(19), false);
                }
                WHOLE_OBSERVATIONS.with(|whole| whole.set(false));
            }
            let mut writer = fixtures[index].writer();
            values.flush(&mut writer, "equal", 5, &flag).unwrap();
            writer.commit().unwrap();
        }
        assert_eq!(
            serde_json::to_vec(&values[0]).unwrap(),
            serde_json::to_vec(&values[1]).unwrap()
        );
        assert_payloads_equal(
            &fixtures[0].payloads(),
            &fixtures[1].payloads(),
            "equal observations",
        );
        for values in &mut values {
            *values = serde_json::from_slice(&serde_json::to_vec(&*values).unwrap()).unwrap();
        }
    }
}

#[test]
fn compact_payload_operands_do_not_expand_constant_spans() {
    let state = KeyState {
        last: Some(Run::new(0, 6, 9, true)),
        partial: LEVELS.map(|aligned| {
            Some(Partial::Span {
                aligned,
                first: 0,
                last: 6,
                value: 9,
                first_break: true,
            })
        }),
        ..Default::default()
    };
    let rows = Rows {
        state,
        ..Default::default()
    };
    let before = SPAN_BUCKET_WORK.with(std::cell::Cell::get);
    let mut identity = Vec::new();
    let expected = payload_input(&rows.runs, &rows.state, &mut identity);
    for _ in 0..5_000 {
        assert_eq!(
            payload_input(&rows.runs, &rows.state, &mut identity),
            expected
        );
    }
    assert_eq!(SPAN_BUCKET_WORK.with(std::cell::Cell::get), before);
}

#[test]
fn ordered_single_run_summaries_keep_tree_fold_bytes_through_resume_gaps_and_refusals() {
    let mut fixtures = [Fixture::new(), Fixture::new()];
    let mut values = [Accumulator::default(), Accumulator::default()];
    let flag = AtomicBool::new(false);
    for (batch, (first, last)) in [(-7, 6), (7, 18), (22, 40), (41, 67)]
        .into_iter()
        .enumerate()
    {
        for (index, values) in values.iter_mut().enumerate() {
            values.advance(first).unwrap();
            for key in 0..64 {
                values.observe(
                    key,
                    first,
                    (batch != 1).then_some(i128::from(key % 4) * -10_001),
                    batch == 0,
                );
            }
            values.advance_span(first + 1, last).unwrap();
            // A mixed run exercises the merging fallback beside constant runs.
            values.observe(1, last, Some(77), false);
            values
                .event(70, last, &serde_json::json!({"batch": batch}))
                .unwrap();
            let before = SUMMARY_MAP_WORK.with(std::cell::Cell::get);
            WHOLE_SUMMARY_MAPS.with(|whole| whole.set(index == 1));
            WHOLE_PAYLOADS.with(|whole| whole.set(index == 1));
            let mut writer = if batch == 3 {
                fixtures[index].append_writer()
            } else {
                fixtures[index].writer()
            };
            values.flush(&mut writer, "summary", 5, &flag).unwrap();
            WHOLE_SUMMARY_MAPS.with(|whole| whole.set(false));
            WHOLE_PAYLOADS.with(|whole| whole.set(false));
            let maps = SUMMARY_MAP_WORK.with(std::cell::Cell::get) - before;
            if index == 0 {
                assert!(maps <= 4, "only the mixed key needs maps: {maps}");
            } else {
                assert!(maps >= 64 * 4, "the reference builds maps for every key");
            }
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &fixtures[0].payloads(),
            &fixtures[1].payloads(),
            "single-run summaries",
        );
        assert_eq!(
            serde_json::to_vec(&values[0]).unwrap(),
            serde_json::to_vec(&values[1]).unwrap()
        );
        for values in &mut values {
            *values = serde_json::from_slice(&serde_json::to_vec(&*values).unwrap()).unwrap();
        }
    }
}

#[test]
fn inactive_flush_keys_keep_their_bytes_without_registration_or_payload_encoding() {
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut values = [Accumulator::default(), Accumulator::default()];
    let flag = AtomicBool::new(false);
    for (batch, (first, last)) in [(0, 7), (8, 17), (21, 39), (40, 70)]
        .into_iter()
        .enumerate()
    {
        for (index, values) in values.iter_mut().enumerate() {
            values.advance(first).unwrap();
            if batch == 0 {
                for key in 0..128 {
                    values.event(key, first, &key).unwrap();
                }
            }
            values.observe(500, first, (batch != 2).then_some(19), batch == 0);
            values.advance_span(first + 1, last).unwrap();
            let registered = REGISTER_WORK.with(std::cell::Cell::get);
            let inputs = PAYLOAD_INPUT_WORK.with(std::cell::Cell::get);
            WHOLE_REGISTRATION.with(|whole| whole.set(index == 1));
            WHOLE_PAYLOADS.with(|whole| whole.set(index == 1));
            let mut writer = fixtures[index].writer();
            values.flush(&mut writer, "inactive", 5, &flag).unwrap();
            WHOLE_REGISTRATION.with(|whole| whole.set(false));
            WHOLE_PAYLOADS.with(|whole| whole.set(false));
            assert_eq!(
                REGISTER_WORK.with(std::cell::Cell::get) - registered,
                if index == 1 { 129 } else { 0 }
            );
            if batch > 0 && index == 0 {
                assert!(PAYLOAD_INPUT_WORK.with(std::cell::Cell::get) - inputs <= 1);
            }
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &fixtures[0].payloads(),
            &fixtures[1].payloads(),
            "inactive keys",
        );
        assert_eq!(
            serde_json::to_vec(&values[0]).unwrap(),
            serde_json::to_vec(&values[1]).unwrap()
        );
        for values in &mut values {
            *values = serde_json::from_slice(&serde_json::to_vec(&*values).unwrap()).unwrap();
        }
    }
}

#[test]
fn changed_projection_observations_carry_equal_values_without_visits_and_keep_bytes() {
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut values = [Accumulator::default(), Accumulator::default()];
    let flag = AtomicBool::new(false);
    for (batch, (first, last)) in [(0, 31), (32, 63), (67, 95)].into_iter().enumerate() {
        for (index, values) in values.iter_mut().enumerate() {
            for hour in first..=last {
                values.advance(hour).unwrap();
                let value = if (first + 8..=first + 10).contains(&hour) {
                    None
                } else {
                    Some(7)
                };
                let starts = hour == first && batch == 0;
                let boundary = hour == first + 15 || hour == first + 20;
                let before = values.key_visits;
                if index == 0 {
                    values.observe_if_changed(1, hour, value, starts, boundary);
                } else {
                    values.observe_boundary(1, hour, value, starts, boundary);
                }
                if hour == first + 2 {
                    assert_eq!(values.key_visits - before, index);
                }
            }
            let mut writer = fixtures[index].writer();
            values.flush(&mut writer, "projection", 5, &flag).unwrap();
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &fixtures[0].payloads(),
            &fixtures[1].payloads(),
            "changed projections",
        );
        assert_eq!(
            serde_json::to_vec(&values[0]).unwrap(),
            serde_json::to_vec(&values[1]).unwrap()
        );
        for values in &mut values {
            *values = serde_json::from_slice(&serde_json::to_vec(&*values).unwrap()).unwrap();
        }
    }
}

#[test]
fn explicit_payload_table_reuses_series_payloads_with_identical_bytes() {
    let run = |cached: bool| {
        let fixture = Fixture::new();
        let mut accumulators = [Accumulator::default(), Accumulator::default()];
        let flag = AtomicBool::new(false);
        let before = PAYLOAD_WORK.with(std::cell::Cell::get);
        let mut snapshots = Vec::new();
        for (first, last) in [(-7, 20), (21, 45), (47, 71)] {
            for accumulator in &mut accumulators {
                for hour in first..=last {
                    accumulator.advance(hour).unwrap();
                    for key in 0..16 {
                        accumulator.observe(
                            key,
                            hour,
                            (hour % 11 != 0).then_some(i128::from(key % 3)),
                            hour % 13 == 0,
                        );
                    }
                }
            }
            let mut payloads = Payloads::default();
            let mut writer = fixture.writer();
            for (index, accumulator) in accumulators.iter_mut().enumerate() {
                if cached {
                    accumulator
                        .flush_with_payloads(
                            &mut writer,
                            &format!("channel-{index}"),
                            0,
                            &flag,
                            &mut payloads,
                        )
                        .unwrap();
                } else {
                    accumulator
                        .flush(&mut writer, &format!("channel-{index}"), 0, &flag)
                        .unwrap();
                }
            }
            writer.commit().unwrap();
            let saved = accumulators
                .each_ref()
                .map(|state| serde_json::to_vec(state).unwrap());
            snapshots.push((fixture.payloads(), saved.clone()));
            accumulators = saved.map(|state| serde_json::from_slice(&state).unwrap());
        }
        (snapshots, PAYLOAD_WORK.with(std::cell::Cell::get) - before)
    };
    let (reference, full_work) = run(false);
    let (cached, work) = run(true);
    assert_eq!(cached, reference);
    assert_eq!(work * 2, full_work, "the second channel builds no payloads");
    assert!(work > 0);
}

#[test]
fn shared_encoding_distinguishes_batch_bounds_and_decimal_scale() {
    let run = |cached: bool| {
        let fixture = Fixture::new();
        let mut payloads = Payloads::default();
        let mut writer = fixture.writer();
        for (index, (first, last, decimals)) in [(0, 7, 0), (-1, 7, 0), (0, 8, 0), (0, 7, 5)]
            .into_iter()
            .enumerate()
        {
            let mut accumulator = Accumulator::default();
            accumulator.through(first);
            accumulator.push(0, Run::new(0, 7, 123456, false));
            accumulator.through(last);
            if cached {
                accumulator
                    .flush_with_payloads(
                        &mut writer,
                        &format!("channel-{index}"),
                        decimals,
                        &AtomicBool::new(false),
                        &mut payloads,
                    )
                    .unwrap();
            } else {
                accumulator
                    .flush(
                        &mut writer,
                        &format!("channel-{index}"),
                        decimals,
                        &AtomicBool::new(false),
                    )
                    .unwrap();
            }
        }
        writer.commit().unwrap();
        fixture.payloads()
    };
    assert_eq!(run(true), run(false));
}

#[test]
fn payload_tables_share_only_the_flushes_explicitly_given_the_same_table() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let flag = AtomicBool::new(false);
    let (mut first, mut other) = (Payloads::default(), Payloads::default());
    for index in 0..4 {
        let table = match index {
            0 | 3 => Some(&mut first),
            2 => Some(&mut other),
            _ => None,
        };
        let mut accumulator = Accumulator::default();
        accumulator.push(0, Run::new(0, 7, 123, false));
        let before = PAYLOAD_WORK.with(std::cell::Cell::get);
        match table {
            Some(table) => accumulator
                .flush_with_payloads(&mut writer, &format!("channel-{index}"), 0, &flag, table)
                .unwrap(),
            None => accumulator
                .flush(&mut writer, &format!("channel-{index}"), 0, &flag)
                .unwrap(),
        }
        assert_eq!(
            PAYLOAD_WORK.with(std::cell::Cell::get) - before,
            usize::from(index != 3)
        );
    }
}

#[test]
fn event_pages_reset_the_merge_frontier_on_changed_requests_and_clamping() {
    // regression: 2deaba1f3d Cut per-reply work in air superiority, wars, autonomy, trade, resources and the air ledger
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for hour in 0..2 {
        values.through(hour);
        for key in 0..3 {
            values.event(key, hour, &json!([hour, key])).unwrap();
        }
        let mut writer = fixture.writer();
        values.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
    }
    let keys: Vec<_> = (0..3).rev().chain([2, 0]).collect();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
    let mut cursor = 0;
    let mut seen = 0;
    loop {
        let page = reader.events(&keys, 0, 1, cursor, 2).unwrap();
        assert_eq!(page.total, 6);
        for row in page.rows {
            assert_eq!(row.value(), json!([seen / 3, seen % 3]));
            assert_eq!(
                (row.key, row.ordinal),
                ((seen % 3) as u64, (seen / 3) as u64)
            );
            seen += 1;
        }
        let Some(next) = page.cursor else { break };
        cursor = next;
    }
    assert_eq!(seen, 6);
    assert_eq!(
        reader.events(&[2], 1, 1, 0, 1).unwrap().rows[0].value(),
        json!([1, 2])
    );
    assert_eq!(
        reader.events(&keys, 0, 1, 0, 1).unwrap().rows[0].value(),
        json!([0, 0])
    );
    reader.through_hour(0).unwrap();
    let page = reader.events(&keys, 0, 1, 1, 10).unwrap();
    assert_eq!(page.total, 3);
    assert!(page.rows.iter().all(|row| row.hour == 0));
}

#[test]
fn warmed_point_readers_keep_local_gaps_and_clamp_runs() {
    // regression: 2deaba1f3d Cut per-reply work in air superiority, wars, autonomy, trade, resources and the air ledger
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for first in [0, 3, 6] {
        for hour in first..first + 3 {
            values.through(hour);
            for key in 0..3 {
                // A refusal, then a fresh run, keeps an actual hole at hour 4.
                let value =
                    (key != 2 || hour != 4).then_some(i128::from(key) + i128::from(hour >= 6));
                values.observe(key, hour, value, false);
            }
        }
        let mut writer = fixture.writer();
        values.flush(&mut writer, "points", 0, &flag).unwrap();
        writer.commit().unwrap();
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "points".into(), &flag).unwrap();
    for key in 0..3 {
        reader.value_at(key, 0).unwrap();
    }
    for key in 0..2 {
        let run = reader.value_at(key, 3).unwrap().unwrap();
        assert_eq!(
            (run.first_hour, run.last_hour, run.value),
            (0, 5, i128::from(key))
        );
    }
    assert!(reader.value_at(2, 4).unwrap().is_none());
    assert_eq!(reader.value_at(2, 5).unwrap().unwrap().first_hour, 5);
    reader.through_hour(2).unwrap();
    assert_eq!(reader.value_at(0, 1).unwrap().unwrap().last_hour, 2);
    assert!(reader.value_at(0, 3).unwrap().is_none());
}

#[test]
fn pushed_run_groups_keep_ordinary_payloads_counters_and_continuation() {
    let fixtures = [Fixture::new(), Fixture::new(), Fixture::new()];
    let mut values: [Accumulator; 3] = Default::default();
    let flag = AtomicBool::new(false);
    for (batch, (first, last)) in [(0, 19), (20, 39), (45, 63)].into_iter().enumerate() {
        for (index, values) in values.iter_mut().enumerate() {
            values.advance_span(first, last).unwrap();
            values.event(1, first, &batch).unwrap();
            let runs: Vec<_> = (first..last)
                .map(|hour| {
                    Run::new(
                        hour,
                        hour,
                        if hour < first + 4 {
                            7
                        } else {
                            i128::from(hour) * -13
                        },
                        hour == first,
                    )
                })
                .collect();
            if index == 0 {
                values.push_runs(1, runs);
            } else if index == 1 {
                for run in runs {
                    values.push(1, run);
                }
            } else {
                values.record_as(0);
                values.push_runs(1, runs);
            }
            values.observe(1, last, Some(i128::from(last) * -13), false);
            let mut writer = fixtures[index].writer();
            values.flush(&mut writer, "grouped-runs", 5, &flag).unwrap();
            writer.commit().unwrap();
        }
        for index in 1..3 {
            assert_payloads_equal(
                &fixtures[0].payloads(),
                &fixtures[index].payloads(),
                "grouped run pushes",
            );
            assert_eq!(
                serde_json::to_vec(&values[0]).unwrap(),
                serde_json::to_vec(&values[index]).unwrap()
            );
        }
        for values in &mut values {
            *values = serde_json::from_slice(&serde_json::to_vec(&*values).unwrap()).unwrap();
        }
    }
}

#[test]
fn unflushed_checkpoints_keep_runs_interned_events_references_and_raw_bytes() {
    #[derive(Default, Serialize, Deserialize)]
    struct Family {
        values: Accumulator,
        presentations: BTreeMap<String, u64>,
    }
    impl Family {
        fn observe(&mut self, hour: i32) {
            self.values.advance(hour).unwrap();
            let name = match hour % 5 {
                0 | 1 => "synthetic alpha",
                2 => "synthetic beta | opaque",
                _ => "synthetic gamma",
            };
            let presentation = if let Some(&ordinal) = self.presentations.get(name) {
                ordinal
            } else {
                let ordinal = self.values.state(10).map_or(0, |state| state.events);
                self.values.event(10, hour, &json!({"name": name})).unwrap();
                self.presentations.insert(name.into(), ordinal);
                ordinal
            };
            self.values.observe_boundary(
                1,
                hour,
                (hour % 7 != 0).then_some(9_007_199_254_740_993 + i128::from(hour % 3)),
                hour == 100,
                hour % 4 == 0,
            );
            self.values
                .observe(2, hour, Some(i128::from(presentation)), false);
            self.values
                .event(20, hour, &json!({"presentation": presentation}))
                .unwrap();
            if hour == 101 {
                self.values
                    .event(
                        20,
                        hour,
                        &json!({"presentation": presentation, "duplicate": true}),
                    )
                    .unwrap();
            }
            self.values
                .event_raw(
                    30,
                    hour,
                    format!(" {{ \"hour\" : {hour} }} \n").into_bytes(),
                )
                .unwrap();
            self.values
                .event_raw(31, hour, vec![0, 255, hour as u8, 128])
                .unwrap();
        }
    }
    let run = |resume: bool| {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut family = Family::default();
        let mut snapshots = Vec::new();
        for hours in [100..=105, 106..=120, 123..=125] {
            for hour in hours {
                family.observe(hour);
                if resume && matches!(hour, 101 | 104 | 109 | 118 | 124) {
                    let saved = serde_json::to_vec(&family).unwrap();
                    let restored: Family = serde_json::from_slice(&saved).unwrap();
                    assert_eq!(restored.values.bytes(), family.values.bytes());
                    assert!(restored.values.bytes() > 0);
                    assert_eq!(serde_json::to_vec(&restored).unwrap(), saved);
                    family = restored;
                }
            }
            let mut writer = fixture.writer();
            family
                .values
                .flush(&mut writer, "pending", 5, &flag)
                .unwrap();
            writer.commit().unwrap();
            snapshots.push(fixture.payloads());
        }
        let mut reader =
            Reader::new(&fixture.store, &fixture.key, "pending".into(), &flag).unwrap();
        assert_eq!(
            reader.events(&[10], 100, 125, 0, 100).unwrap().rows.len(),
            3
        );
        let observations = reader.events(&[20], 100, 125, 0, 100).unwrap();
        assert_eq!(observations.total, 25);
        for row in observations.rows {
            let ordinal = row.value()["presentation"].as_u64().unwrap();
            assert!(reader.event_at(10, ordinal).unwrap().value()["name"].is_string());
        }
        for (ordinal, hour) in (100..=120).chain(123..=125).enumerate() {
            assert_eq!(
                reader.event_raw(30, ordinal as u64).unwrap(),
                (hour, format!(" {{ \"hour\" : {hour} }} \n").into_bytes())
            );
            assert_eq!(
                reader.event_raw(31, ordinal as u64).unwrap(),
                (hour, vec![0, 255, hour as u8, 128])
            );
        }
        if resume {
            let saved = serde_json::to_vec(&family).unwrap();
            let mut restored: Family = serde_json::from_slice(&saved).unwrap();
            let mut writer = fixture.writer();
            restored
                .values
                .flush(&mut writer, "pending", 5, &flag)
                .unwrap();
            writer.commit().unwrap();
            assert_payloads_equal(
                &fixture.payloads(),
                snapshots.last().unwrap(),
                "empty restored batch publishes nothing",
            );
        }
        snapshots
    };
    let expected = run(false);
    let actual = run(true);
    for (actual, expected) in actual.iter().zip(&expected) {
        assert_payloads_equal(actual, expected, "unflushed accumulator resume");
    }
}

#[test]
fn empty_batches_keep_public_state_bytes_and_bounds_only_batches_resume() {
    let mut uninterrupted = Accumulator::default();
    assert_eq!(
        serde_json::to_vec(&uninterrupted).unwrap(),
        br#"{"states":{},"frontier":null,"previous":null,"active":[]}"#
    );
    uninterrupted.advance(40).unwrap();
    uninterrupted.advance(41).unwrap();
    assert_eq!(uninterrupted.bytes(), 0);
    let saved = serde_json::to_vec(&uninterrupted).unwrap();
    let mut restored: Accumulator = serde_json::from_slice(&saved).unwrap();
    assert_eq!(serde_json::to_vec(&restored).unwrap(), saved);
    let batch: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(batch["batch"]["first"], 40);
    assert_eq!(batch["batch"]["last"], 41);
    let expected = Fixture::new();
    let actual = Fixture::new();
    for (state, fixture) in [(&mut uninterrupted, &expected), (&mut restored, &actual)] {
        let mut writer = fixture.writer();
        state
            .flush(&mut writer, "bounds", 0, &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
        assert!(
            serde_json::to_value(&*state)
                .unwrap()
                .get("batch")
                .is_none()
        );
    }
    assert!(!expected.payloads().is_empty());
    assert_payloads_equal(
        &actual.payloads(),
        &expected.payloads(),
        "bounds-only checkpoint",
    );
}

#[test]
fn backdated_events_do_not_move_the_forward_frontier() {
    let flag = AtomicBool::new(false);
    for resume in [false, true] {
        let fixture = Fixture::new();
        let mut values = Accumulator::default();
        values.advance(48).unwrap();
        values.observe(1, 48, Some(100), true);
        for (hour, instance) in [(24, 1), (29, 2), (30, 1)] {
            values
                .event(9, hour, &json!({"instance": instance}))
                .unwrap();
        }
        if resume {
            values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
        }
        let mut writer = fixture.writer();
        values.flush(&mut writer, "revisions", 0, &flag).unwrap();
        writer.commit().unwrap();
        let mut reader =
            Reader::new(&fixture.store, &fixture.key, "revisions".into(), &flag).unwrap();
        assert_eq!(reader.value_at(1, 48).unwrap().unwrap().value, 100);
        let revisions = reader.events(&[9], 24, 29, 0, 10).unwrap();
        assert_eq!(revisions.total, 2);
        assert_eq!(
            revisions
                .rows
                .iter()
                .map(|row| (row.hour, row.value()["instance"].as_u64().unwrap()))
                .collect::<Vec<_>>(),
            vec![(24, 1), (29, 2)]
        );
        assert_eq!(reader.events(&[9], 30, 48, 0, 10).unwrap().total, 1);
    }
}

#[test]
fn backdated_event_ranks_include_committed_and_resumed_batches() {
    let flag = AtomicBool::new(false);
    for resume in [false, true] {
        let mut fixture = Fixture::new();
        let mut values = Accumulator::default();
        let restore = |values: &mut Accumulator| {
            if resume {
                *values = serde_json::from_slice(&serde_json::to_vec(values).unwrap()).unwrap();
            }
        };
        for hour in 24..=30 {
            values.advance(hour).unwrap();
            values.observe(1, hour, Some(100), hour == 24);
            values.event(9, hour, &json!({"instance": 1})).unwrap();
            if hour == 26 || hour == 30 {
                values.event(10, hour, &json!({"instance": 1})).unwrap();
            }
            let mut writer = fixture.writer();
            values.flush(&mut writer, "revisions", 0, &flag).unwrap();
            writer.commit().unwrap();
            restore(&mut values);
        }
        values.advance(48).unwrap();
        values.observe(1, 48, Some(4800), true);
        for hour in [24, 29, 30] {
            values.event(9, hour, &json!({"instance": 2})).unwrap();
        }
        values.event(10, 22, &json!({"instance": 2})).unwrap();
        values.event(12, 23, &json!({"instance": 7})).unwrap();
        restore(&mut values);
        let mut writer = fixture.writer();
        values.flush(&mut writer, "revisions", 0, &flag).unwrap();
        writer.commit().unwrap();
        let mut earlier =
            Reader::new(&fixture.store, &fixture.key, "revisions".into(), &flag).unwrap();
        assert_eq!(earlier.event_count_through(9, 29).unwrap(), 8);
        assert_eq!(
            earlier.latest_value(9, 29).unwrap(),
            Some((29, json!({"instance": 2})))
        );
        restore(&mut values);
        values.advance(60).unwrap();
        values.observe(1, 60, Some(6000), true);
        for (hour, instance) in [(25, 3), (28, 3), (29, 3), (29, 4)] {
            values
                .event(9, hour, &json!({"instance": instance}))
                .unwrap();
        }
        values.event(10, 21, &json!({"instance": 3})).unwrap();
        values.event(11, 60, &json!({"instance": 5})).unwrap();
        restore(&mut values);
        let mut writer = fixture.writer();
        values.flush(&mut writer, "revisions", 0, &flag).unwrap();
        writer.commit().unwrap();
        assert_eq!(earlier.event_count_through(9, 29).unwrap(), 8);
        assert_eq!(
            earlier.latest_value(9, 29).unwrap(),
            Some((29, json!({"instance": 2})))
        );
        drop(earlier);
        restore(&mut values);
        // A later ordinary publication must retain the index even without new events for its keys.
        values.advance(61).unwrap();
        values.event(11, 61, &json!({"instance": 6})).unwrap();
        let mut writer = if resume {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        values.flush(&mut writer, "revisions", 0, &flag).unwrap();
        writer.commit().unwrap();
        let mut reader =
            Reader::new(&fixture.store, &fixture.key, "revisions".into(), &flag).unwrap();
        for (hour, expected) in [
            (23, 0),
            (24, 2),
            (25, 4),
            (26, 5),
            (27, 6),
            (28, 8),
            (29, 12),
            (30, 14),
            (61, 14),
        ] {
            assert_eq!(
                reader.event_count_through(9, hour).unwrap(),
                expected,
                "hour {hour}, resume {resume}"
            );
        }
        let expected = vec![
            (24, 1),
            (24, 2),
            (25, 1),
            (25, 3),
            (26, 1),
            (27, 1),
            (28, 1),
            (28, 3),
            (29, 1),
            (29, 2),
            (29, 3),
            (29, 4),
            (30, 1),
            (30, 2),
        ];
        let rows = |page: &EventsPage| {
            page.rows
                .iter()
                .map(|row| (row.hour, row.value()["instance"].as_u64().unwrap()))
                .collect::<Vec<_>>()
        };
        let all = reader.events(&[9], 24, 61, 0, 20).unwrap();
        assert_eq!(all.total, 14);
        assert_eq!(all.cursor, None);
        assert_eq!(rows(&all), expected);
        for (ordinal, &(hour, instance)) in expected.iter().enumerate() {
            let row = reader.event_at(9, ordinal as u64).unwrap();
            assert_eq!(
                (row.hour, row.ordinal, row.value()),
                (hour, ordinal as u64, json!({"instance": instance}))
            );
            assert_eq!(reader.event_date(9, ordinal as u64).unwrap(), hour);
            let (date, raw) = reader.event_raw(9, ordinal as u64).unwrap();
            assert_eq!(
                (date, serde_json::from_slice::<Value>(&raw).unwrap()),
                (hour, json!({"instance": instance}))
            );
        }
        assert!(reader.event_at(9, 14).is_err());
        assert_eq!(
            reader.latest_value(9, 29).unwrap(),
            Some((29, json!({"instance": 4})))
        );
        assert_eq!(
            reader.latest_value(10, 25).unwrap(),
            Some((22, json!({"instance": 2})))
        );
        let latest = reader.latest_event_rows(&[9, 10, 12, 99], 29).unwrap();
        assert_eq!(
            latest
                .into_iter()
                .map(|row| row
                    .map(|(hour, raw)| (hour, serde_json::from_slice::<Value>(&raw).unwrap())))
                .collect::<Vec<_>>(),
            vec![
                Some((29, json!({"instance": 4}))),
                Some((26, json!({"instance": 1}))),
                Some((23, json!({"instance": 7}))),
                None
            ]
        );
        let first = reader.events(&[9], 24, 29, 0, 4).unwrap();
        assert_eq!((first.first, first.total, first.cursor), (0, 12, Some(4)));
        assert_eq!(rows(&first), vec![(24, 1), (24, 2), (25, 1), (25, 3)]);
        let next = reader.events(&[9], 24, 29, 4, 4).unwrap();
        assert_eq!((next.first, next.total, next.cursor), (4, 12, Some(8)));
        assert_eq!(rows(&next), vec![(26, 1), (27, 1), (28, 1), (28, 3)]);
        let last = reader.events(&[9], 24, 29, 8, 4).unwrap();
        assert_eq!((last.first, last.total, last.cursor), (8, 12, None));
        assert_eq!(rows(&last), vec![(29, 1), (29, 2), (29, 3), (29, 4)]);
        let within = reader
            .events_within(&[(9, 25, 28), (10, 21, 22), (12, 23, 23)], 0, 20)
            .unwrap();
        assert_eq!(within.total, 9);
        assert_eq!(
            within
                .rows
                .iter()
                .map(|row| (row.hour, row.key, row.value()["instance"].as_u64().unwrap()))
                .collect::<Vec<_>>(),
            vec![
                (21, 10, 3),
                (22, 10, 2),
                (23, 12, 7),
                (25, 9, 1),
                (25, 9, 3),
                (26, 9, 1),
                (27, 9, 1),
                (28, 9, 1),
                (28, 9, 3)
            ]
        );
        let clusters = reader.event_clusters(9, 24.0, 30.0, 18).unwrap().clusters;
        assert_eq!(clusters.len(), 1);
        assert_eq!(
            (
                clusters[0].first_hour,
                clusters[0].last_hour,
                clusters[0].count.as_str(),
                clusters[0].first_ordinal.as_str()
            ),
            (24, 30, "14", "0")
        );
        assert_eq!(reader.value_at(1, 29).unwrap().unwrap().value, 100);
        assert!(reader.value_at(1, 31).unwrap().is_none());
        assert_eq!(reader.value_at(1, 48).unwrap().unwrap().value, 4800);
        assert_eq!(reader.value_at(1, 60).unwrap().unwrap().value, 6000);
        assert_eq!(reader.through_hour(48).unwrap(), Some(48));
        assert_eq!(reader.event_count_through(9, 29).unwrap(), 8);
        assert_eq!(
            reader.latest_value(9, 29).unwrap(),
            Some((29, json!({"instance": 2})))
        );
        assert_eq!(reader.through_hour(30).unwrap(), Some(30));
        assert_eq!(reader.event_count_through(9, 29).unwrap(), 6);
        assert_eq!(
            reader.latest_value(9, 29).unwrap(),
            Some((29, json!({"instance": 1})))
        );
        assert_eq!(reader.latest_value(12, 30).unwrap(), None);
    }
}

#[test]
fn ordinary_event_batches_keep_their_exact_unindexed_format() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    let mut expected = PublishedPayloads::new();
    for (batch, hour) in (24_i32..=30).enumerate() {
        values.event(9, hour, &1).unwrap();
        let mut writer = fixture.writer();
        values.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
        let mut raw = vec![hour as u8 * 2, 1, b'1'];
        raw.extend_from_slice(&[0, 1, 32, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]);
        let compressed = compress(&raw, 3).unwrap();
        let mut page = Vec::new();
        page.extend_from_slice(&hour.to_le_bytes());
        page.extend_from_slice(&hour.to_le_bytes());
        for value in [1_u64, 9, 80, compressed.len() as u64, 0, 0, batch as u64, 1] {
            page.extend_from_slice(&value.to_le_bytes());
        }
        page.extend_from_slice(&hour.to_le_bytes());
        page.extend_from_slice(&[0; 4]);
        page.extend(compressed);
        expected.insert(("events".into(), batch as u64), page);
    }
    assert_payloads_equal(&fixture.payloads(), &expected, "ordinary event encoding");
    let saved = serde_json::to_value(&values).unwrap();
    assert_eq!(
        saved["states"]["9"],
        json!({"l": null, "r": 0, "e": 7, "p": [null, null, null, null]})
    );
}

#[test]
fn each_event_gives_the_rows_events_gives_late_ones_included() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    let mut writer = fixture.writer();
    for hour in 0..64 {
        values.advance(hour).unwrap();
        for key in [3, 9] {
            values
                .event(key, hour, &json!({"key": key, "hour": hour}))
                .unwrap();
        }
        if hour % 16 == 15 {
            values.flush(&mut writer, "events", 0, &flag).unwrap();
        }
    }
    // Late rows, dated behind the frontier, kept in later batches.
    values.advance(100).unwrap();
    values.event(9, 20, &json!({"late": 20})).unwrap();
    values.event(9, 40, &json!({"late": 40})).unwrap();
    values.flush(&mut writer, "events", 0, &flag).unwrap();
    values.advance(120).unwrap();
    values.event(9, -5, &json!({"late": -5})).unwrap();
    values.flush(&mut writer, "events", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
    for (from, to) in [
        (i32::MIN, i32::MAX),
        (0, 63),
        (18, 41),
        (40, 40),
        (64, 200),
        (-5, -5),
    ] {
        for key in [3, 9] {
            let mut paged = Vec::new();
            let mut cursor = 0;
            loop {
                let page = reader.events(&[key], from, to, cursor, 7).unwrap();
                paged.extend(page.rows.into_iter().map(|row| (row.hour, row.value())));
                match page.cursor {
                    Some(next) => cursor = next,
                    None => break,
                }
            }
            let mut each = Vec::new();
            reader
                .each_event(key, from, to, &mut |date, row| {
                    each.push((date, serde_json::from_slice::<Value>(row)?));
                    Ok(())
                })
                .unwrap();
            let order = |rows: &mut Vec<(i32, Value)>| rows.sort_by_key(|row| row.1.to_string());
            order(&mut paged);
            order(&mut each);
            assert_eq!(each, paged, "{key} {from}..={to}");
        }
    }
    // Cancelled while a batch is visited, the read stops before its next row.
    let mut visited = 0;
    let refused = reader.each_event(9, 0, 63, &mut |_, _| {
        visited += 1;
        flag.store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    });
    assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert_eq!(visited, 1);
}

#[test]
fn backdated_event_counts_do_not_scan_committed_batches() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    let mut writer = fixture.writer();
    for hour in 0..256 {
        values.advance(hour).unwrap();
        values.event(9, hour, &1).unwrap();
        values.flush(&mut writer, "events", 0, &flag).unwrap();
    }
    values.advance(512).unwrap();
    values.event(9, 127, &2).unwrap();
    values.flush(&mut writer, "events", 0, &flag).unwrap();
    writer.commit().unwrap();
    for offset in 1..=64 {
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
        values.advance(512 + offset).unwrap();
        values.event(9, -offset, &3).unwrap();
        let mut writer = fixture.writer();
        values.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
    let before = directory_tests::SMALL_READS.get();
    let decoded = decoded_event_rows();
    assert_eq!(reader.ordinary_count_through(9, 127).unwrap(), 128);
    let prefix_reads = directory_tests::SMALL_READS.get() - before;
    assert!(
        prefix_reads <= 11,
        "{prefix_reads} reads for 321 batch boundaries"
    );
    let before = directory_tests::SMALL_READS.get();
    assert_eq!(reader.event_count_through(9, 127).unwrap(), 193);
    let reads = directory_tests::SMALL_READS.get() - before;
    assert_eq!(decoded_event_rows(), decoded, "rank decoded event payloads");
    let root = reader.event_order(9).unwrap().unwrap();
    let height = u64::from(reader.order_node(root).unwrap().height);
    assert!(height <= 2 * (321_u64.ilog2() as u64 + 1));
    assert_eq!(reader.order_node(root).unwrap().count, 65);
    assert!(height <= 2 * (65_u64.ilog2() as u64 + 1));
    assert!(reads <= 2 * height + 5, "{reads} reads at height {height}");
    assert_eq!(reader.event_count_through(9, -65).unwrap(), 0);
    assert_eq!(reader.event_count_through(9, -1).unwrap(), 64);
    assert_eq!(reader.event_count_through(9, 576).unwrap(), 321);
    assert_eq!(reader.latest_value(9, 127).unwrap(), Some((127, json!(2))));
    for (ordinal, hour, value) in [
        (0, -64, 3),
        (63, -1, 3),
        (64, 0, 1),
        (192, 127, 2),
        (320, 255, 1),
    ] {
        let row = reader.event_at(9, ordinal).unwrap();
        assert_eq!((row.hour, row.value()), (hour, json!(value)));
    }
}

#[test]
fn event_only_backdating_keeps_unindexed_keys_and_batch_bounds_exact() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for hour in [10, 20, 30, 5, 31] {
        if hour == 31 {
            values.event(10, hour, &1).unwrap();
        } else {
            values
                .event(9, hour, &if hour == 5 { 2 } else { 1 })
                .unwrap();
        }
        if hour == 30 {
            values.event(10, hour, &1).unwrap();
        }
        let mut writer = fixture.writer();
        values.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
    for (batch, expected) in [
        (0, (10, 10)),
        (1, (20, 20)),
        (2, (30, 30)),
        (3, (30, 30)),
        (4, (31, 31)),
    ] {
        assert_eq!(reader.hours(batch).unwrap(), expected);
    }
    assert_eq!(reader.event_count_through(9, 20).unwrap(), 3);
    assert_eq!(reader.latest_value(9, 5).unwrap(), Some((5, json!(2))));
    assert_eq!(reader.event_count_through(10, 29).unwrap(), 0);
    assert_eq!(reader.latest_value(10, 29).unwrap(), None);
    assert_eq!(reader.latest_value(10, 30).unwrap(), Some((30, json!(1))));
    let mut file = fixture
        .store
        .read(&fixture.key, "events", 4)
        .unwrap()
        .unwrap();
    assert_eq!(
        Table::read(&mut file)
            .unwrap()
            .unwrap()
            .entry(9)
            .unwrap()
            .order_slot,
        Some(0)
    );
    assert_eq!(directory(&mut file, 9).unwrap().order_slot, Some(0));
    assert_eq!(reader.through_hour(20).unwrap(), Some(20));
    assert_eq!(reader.event_count_through(9, 20).unwrap(), 2);
    assert_eq!(reader.latest_value(9, 5).unwrap(), None);
}

#[cfg(test)]
struct OrderConstruction<'a> {
    inner: &'a mut dyn super::super::pages::Pages,
    read_bytes: usize,
    index_bytes: usize,
    initial_bytes: usize,
}
#[cfg(test)]
impl super::super::pages::Pages for OrderConstruction<'_> {
    fn write_at(&mut self, section: &str, range: u64, bytes: &[u8]) -> io::Result<()> {
        self.index_bytes += bytes.len() - self.initial_bytes;
        self.inner.write_at(section, range, bytes)
    }
    fn write(&mut self, section: &str, bytes: &[u8]) -> io::Result<u64> {
        self.initial_bytes = bytes.len();
        self.inner.write(section, bytes)
    }
    fn read_at(&mut self, _: &str, _: u64) -> io::Result<Option<Vec<u8>>> {
        panic!("index construction must not materialize historical pages")
    }
    fn read_slice(
        &mut self,
        section: &str,
        range: u64,
        offset: u64,
        output: &mut [u8],
    ) -> io::Result<()> {
        self.read_bytes += output.len();
        self.inner.read_slice(section, range, offset, output)
    }
}

#[test]
fn first_backdate_construction_is_independent_of_the_forward_history() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for batch in 0..256 {
        for hour in batch * 64..(batch + 1) * 64 {
            values.event(9, hour, &1).unwrap();
        }
        let mut writer = fixture.writer();
        values.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
    }
    values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
    values.advance(32768).unwrap();
    values.event(9, 17, &2).unwrap();
    let mut writer = fixture.writer();
    let mut pages = OrderConstruction {
        inner: &mut writer,
        read_bytes: 0,
        index_bytes: 0,
        initial_bytes: 0,
    };
    let decoded = decoded_event_rows();
    let blobs = BLOB_DECODES.get();
    order::BUILT_NODES.set(0);
    order::CAPACITY_GROWTH.set(0);
    order::PEAK_DEPTH.set(0);
    values.flush(&mut pages, "events", 0, &flag).unwrap();
    assert_eq!(decoded_event_rows() - decoded, 0);
    assert_eq!(BLOB_DECODES.get() - blobs, 0);
    assert_eq!(pages.read_bytes, 16);
    assert_eq!(pages.index_bytes, 72);
    assert_eq!(order::BUILT_NODES.get(), 1);
    assert_eq!(order::PEAK_DEPTH.get(), 1);
    assert!(order::CAPACITY_GROWTH.get() <= 72);
    writer.commit().unwrap();
    values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
    values.advance(32769).unwrap();
    values.event(9, 32769, &3).unwrap();
    let mut writer = fixture.writer();
    order::BUILT_NODES.set(0);
    values.flush(&mut writer, "events", 0, &flag).unwrap();
    assert_eq!(
        order::BUILT_NODES.get(),
        0,
        "forward rows must remain implicit"
    );
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
    assert_eq!(reader.event_count_through(9, 17).unwrap(), 19);
    assert_eq!(reader.event_count_through(9, 32769).unwrap(), 16386);
    for (ordinal, hour, value) in [
        (17, 17, 1),
        (18, 17, 2),
        (16384, 16383, 1),
        (16385, 32769, 3),
    ] {
        let row = reader.event_at(9, ordinal).unwrap();
        assert_eq!((row.hour, row.value()), (hour, json!(value)));
    }
}

#[test]
fn staggered_first_backdates_do_not_seed_any_of_fifteen_hundred_keys() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for hour in 0..64 {
        for key in 0..1500 {
            values.event(key, hour, &1).unwrap();
        }
        let mut writer = fixture.writer();
        values.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
    }
    for step in 0..32 {
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
        values.advance(512 + step).unwrap();
        values.event(step as u64 * 47, 7, &2).unwrap();
        let mut writer = fixture.writer();
        let mut pages = OrderConstruction {
            inner: &mut writer,
            read_bytes: 0,
            index_bytes: 0,
            initial_bytes: 0,
        };
        let decoded = decoded_event_rows();
        let blobs = BLOB_DECODES.get();
        order::BUILT_NODES.set(0);
        order::CAPACITY_GROWTH.set(0);
        order::PEAK_DEPTH.set(0);
        values.flush(&mut pages, "events", 0, &flag).unwrap();
        assert_eq!(decoded_event_rows() - decoded, 0, "first backdate {step}");
        assert_eq!(BLOB_DECODES.get() - blobs, 0, "first backdate {step}");
        assert_eq!(pages.read_bytes, 16, "first backdate {step}");
        assert_eq!(pages.index_bytes, 24056, "first backdate {step}");
        assert_eq!(order::BUILT_NODES.get(), 1, "first backdate {step}");
        assert_eq!(order::PEAK_DEPTH.get(), 1, "first backdate {step}");
        assert!(
            order::CAPACITY_GROWTH.get() <= 24056,
            "first backdate {step}"
        );
        writer.commit().unwrap();
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
    for key in (0..32).map(|step| step * 47) {
        assert_eq!(reader.event_count_through(key, 7).unwrap(), 9);
        assert_eq!(reader.event_count_through(key, 543).unwrap(), 65);
        for (ordinal, hour, value) in [(7, 7, 1), (8, 7, 2), (64, 63, 1)] {
            let row = reader.event_at(key, ordinal).unwrap();
            assert_eq!((row.hour, row.value()), (hour, json!(value)));
        }
        let root = reader.event_order(key).unwrap().unwrap();
        assert_eq!(reader.order_node(root).unwrap().count, 1);
    }
    assert_eq!(reader.event_count_through(1499, 7).unwrap(), 8);
    assert_eq!(reader.event_count_through(1499, 543).unwrap(), 64);
    assert!(reader.event_order(1499).unwrap().is_none());
}

#[test]
fn backdated_index_allocation_refusal_propagates_before_roots_change() {
    const CHILD: &str = "BATTLE_PLAN_EVENT_ORDER_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "prepared::series::tests::backdated_index_allocation_refusal_propagates_before_roots_change", "--nocapture"])
            .env(CHILD, "1").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let flag = AtomicBool::new(false);
    let mut bytes = vec![0; 80];
    put(&mut bytes, 0, &48_i32.to_le_bytes());
    put(&mut bytes, 4, &48_i32.to_le_bytes());
    put(&mut bytes, 8, &1_u64.to_le_bytes());
    let original = bytes.clone();
    let mut state = KeyState {
        events: 1,
        ..Default::default()
    };
    let mut events = [(24, vec![b'1'])];
    engine_allocation::set_stop_hook(Some(|| true));
    let result = order::publish(
        &mut writer,
        "events",
        0,
        &mut bytes,
        &mut [(9, &[], &mut events, &mut state)],
        &flag,
    );
    engine_allocation::set_stop_hook(None);
    let error = result.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert_eq!(error.to_string(), "Prepared event order allocation refused");
    assert_eq!(bytes, original);
    assert!(state.event_order.is_none());
}

#[test]
fn mixed_backdated_and_forward_rows_merge_at_publication_ties() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for (publication, rows) in [
        (10, vec![(10, 1)]),
        (20, vec![(20, 2)]),
        (20, vec![(5, 3), (20, 4), (21, 5)]),
        (21, vec![(5, 6), (5, 7), (20, 8), (21, 9), (22, 10)]),
        (48, vec![(i32::MIN, 11), (20, 12), (48, 13)]),
    ] {
        values.advance(publication).unwrap();
        for (hour, value) in rows {
            values.event(9, hour, &value).unwrap();
        }
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
        let mut writer = fixture.writer();
        values.flush(&mut writer, "events", 0, &flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "events".into(), &flag).unwrap();
    for (hour, count) in [
        (i32::MIN, 1),
        (5, 4),
        (20, 9),
        (21, 11),
        (22, 12),
        (48, 13),
        (i32::MAX, 13),
    ] {
        assert_eq!(reader.event_count_through(9, hour).unwrap(), count);
    }
    let expected = [
        (i32::MIN, 11),
        (5, 3),
        (5, 6),
        (5, 7),
        (10, 1),
        (20, 2),
        (20, 4),
        (20, 8),
        (20, 12),
        (21, 5),
        (21, 9),
        (22, 10),
        (48, 13),
    ];
    for (ordinal, (hour, value)) in expected.into_iter().enumerate() {
        let row = reader.event_at(9, ordinal as u64).unwrap();
        assert_eq!((row.hour, row.value()), (hour, json!(value)));
    }
    assert!(reader.event_at(9, 13).is_err());
    let root = reader.event_order(9).unwrap().unwrap();
    assert_eq!(reader.order_node(root).unwrap().count, 6);
    assert_eq!(reader.through_hour(21).unwrap(), Some(21));
    assert_eq!(reader.event_count_through(9, 5).unwrap(), 1);
    assert_eq!(reader.event_count_through(9, 21).unwrap(), 5);
    assert_eq!(reader.latest_value(9, 20).unwrap(), Some((20, json!(4))));
}

#[test]
fn late_only_order_roots_refuse_the_full_history_format() {
    let mut directory = [0; 64];
    put(&mut directory, 60, &1_u32.to_le_bytes());
    assert_eq!(
        order::slot(&directory, 0).unwrap_err().to_string(),
        "invalid event order directory flag"
    );
    assert!(
        serde_json::from_value::<KeyState>(json!({
            "l": null, "r": 0, "e": 1, "p": [null, null, null, null],
            "o": {"batch": 0, "offset": 80}
        }))
        .is_err()
    );
}

/// A key's payload grown in one buffer in payload order, its events copied
/// from their rows: the bytes every flush wrote before [`payload`].
#[cfg(test)]
fn payload_in_one_pass(
    runs: &[Run],
    events: &[EventRow],
    state: &mut KeyState,
    (first, last): (i32, i32),
    decimals: u8,
) -> Vec<u8> {
    let cancelled = AtomicBool::new(false);
    let mut raw = Vec::new();
    let (mut hour, mut value) = (0_i128, 0_i128);
    for run in runs {
        var(&mut raw, zig(i128::from(run.first_hour) - hour));
        var(
            &mut raw,
            (i64::from(run.last_hour) - i64::from(run.first_hour)) as u128,
        );
        var(&mut raw, zig(run.value.wrapping_sub(value)));
        var(&mut raw, u128::from(run.break_before));
        hour = i128::from(run.last_hour);
        value = run.value;
    }
    hour = 0;
    for (date, row) in events {
        var(&mut raw, zig(i128::from(*date) - hour));
        var(&mut raw, row.len() as u128);
        raw.extend_from_slice(row);
        hour = i128::from(*date);
    }
    for (level, size) in LEVELS.into_iter().enumerate() {
        let buckets =
            summary_buckets(runs, state.partial[level], size, decimals, &cancelled).unwrap();
        var(&mut raw, buckets.len() as u128);
        let mut pending =
            state.partial[level].map(|partial| (partial.aligned(), partial.bucket(decimals)));
        let mut previous = 0_i64;
        for (aligned, mut bucket) in buckets {
            if let Some(bucket) = &mut bucket {
                bucket.has_break |= i64::from(bucket.first_hour) > aligned.max(i64::from(first))
                    || i64::from(bucket.last_hour) < (aligned + size - 1).min(i64::from(last));
            }
            var(&mut raw, zig(i128::from(aligned - previous)));
            previous = aligned;
            encode_bucket(&mut raw, bucket, aligned);
            let mut prefix = pending
                .filter(|(previous, _)| *previous == aligned)
                .map_or_else(
                    || bucket.unwrap(),
                    |(_, value)| merge(Some(value), bucket).unwrap(),
                );
            prefix.has_break |=
                i64::from(prefix.last_hour) < (aligned + size - 1).min(i64::from(last));
            encode_bucket(&mut raw, Some(prefix), aligned);
            pending = Some((aligned, prefix));
        }
        let tail = i64::from(last).div_euclid(size) * size;
        let held = state.last;
        state.partial[level] = pending
            .filter(|(aligned, _)| *aligned == tail && i64::from(last) < tail + size - 1)
            .map(|(aligned, bucket)| Partial::of(aligned, bucket, held, decimals));
        let aligned = |hour: i32| i64::from(hour).div_euclid(size) * size;
        let counts = || events.chunk_by(|a, b| aligned(a.0) == aligned(b.0));
        var(&mut raw, counts().count() as u128);
        previous = 0;
        for events in counts() {
            let aligned = aligned(events[0].0);
            var(&mut raw, zig(i128::from(aligned - previous)));
            var(&mut raw, events.len() as u128);
            previous = aligned;
        }
    }
    raw
}

#[test]
fn payload_writes_the_one_pass_bytes_and_releases_each_event_row() {
    let row = |length: usize, seed: u8| -> Vec<u8> {
        (0..length).map(|at| seed.wrapping_add(at as u8)).collect()
    };
    // Successive flushes of one key: runs with events and breaks, runs alone
    // merging into the carried partial summaries, then events alone. Rows
    // reach one-, two- and three-byte lengths; dates repeat and jump.
    let batches: [(Vec<Run>, Vec<EventRow>, (i32, i32)); 3] = [
        (
            vec![
                Run::new(60_000_000, 60_000_011, 7, true),
                Run::new(60_000_012, 60_000_040, -3, false),
                Run::new(60_000_045, 60_000_045, 1 << 40, true),
            ],
            vec![
                (60_000_000, row(0, 1)),
                (60_000_000, row(5, 2)),
                (60_000_017, row(200, 3)),
                (60_000_300, row(20_000, 4)),
            ],
            (60_000_000, 60_000_300),
        ),
        (
            vec![Run::new(60_000_301, 60_000_900, -1, false)],
            Vec::new(),
            (60_000_301, 60_000_900),
        ),
        (
            Vec::new(),
            vec![
                (60_000_901, row(3, 5)),
                (60_005_000, row(130, 6)),
                (60_005_000, row(1, 7)),
            ],
            (60_000_901, 60_005_000),
        ),
    ];
    let (mut state, mut expected_state) = (KeyState::default(), KeyState::default());
    for (runs, mut events, bounds) in batches {
        for state in [&mut state, &mut expected_state] {
            state.last = runs.last().copied().or(state.last);
        }
        let expected = payload_in_one_pass(&runs, &events, &mut expected_state, bounds, 2);
        let dates: Vec<_> = events.iter().map(|(date, _)| *date).collect();
        let raw = payload(
            &runs,
            &mut events,
            &mut state,
            bounds,
            2,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(raw, expected);
        assert_eq!(
            events.iter().map(|(date, _)| *date).collect::<Vec<_>>(),
            dates
        );
        assert!(
            events
                .iter()
                .all(|(_, row)| row.is_empty() && row.capacity() == 0)
        );
        assert_eq!(
            serde_json::to_vec(&state).unwrap(),
            serde_json::to_vec(&expected_state).unwrap()
        );
    }
}

#[test]
fn a_blob_reads_its_runs_without_its_summaries_and_a_summary_read_checks_them() {
    // One run over hours 5..=7 of value 7, then a damaged summary tail.
    let mut raw = Vec::new();
    for value in [zig(5), 2, zig(7), 0] {
        var(&mut raw, value);
    }
    raw.push(0xff);
    let directory = Directory {
        run_rows: 1,
        ..Directory::default()
    };
    let cancelled = AtomicBool::new(false);
    let cached = decode(raw.clone(), directory, &cancelled).unwrap();
    assert_eq!(cached.runs, [Run::new(5, 7, 7, false)]);
    assert!(cached.decode_summaries(&cancelled).is_err());
    // Four empty levels, longer than the run before them, read from within the blob.
    raw.pop();
    raw.extend([0; 8]);
    let cached = decode(raw, directory, &cancelled).unwrap();
    assert_eq!(cached.runs, [Run::new(5, 7, 7, false)]);
    assert_eq!(cached.summaries_at, 4);
    assert_eq!(
        cached.decode_summaries(&cancelled).unwrap(),
        Summaries::default()
    );
}

#[test]
fn top_spans_keep_only_ranked_hours_and_break_at_each_crossing() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut ranks = Accumulator::default();
    for hour in 0..64 {
        let rank = match hour {
            0..=15 | 20..=23 | 48..=63 => 1,
            16..=19 => 2,
            _ => 3,
        };
        ranks.through(hour);
        ranks.observe(1, hour, Some(rank), false);
    }
    let mut writer = fixture.writer();
    ranks.flush(&mut writer, "test", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let spans = |spans: &[(i32, i32)]| -> Vec<crate::products::country_groups::HourSpan> {
        spans
            .iter()
            .map(|&(first_hour, last_hour)| crate::products::country_groups::HourSpan {
                first_hour,
                last_hour,
            })
            .collect()
    };
    let tail = [Run::new(64, 67, 2, false), Run::new(68, 71, 1, false)];
    // Eight-hour columns: hours 16..=19 ranked second split their column at the cut.
    assert_eq!(
        reader
            .top_spans(1, (0, 71), (0.0, 71.0, 9), 1, &tail)
            .unwrap(),
        spans(&[(0, 15), (20, 23), (48, 63), (68, 71)])
    );
    assert_eq!(
        reader.top_spans(1, (0, 71), (0.0, 71.0, 9), 3, &tail).unwrap(),
        spans(&[(0, 71)])
    );
    assert!(
        reader
            .top_spans(1, (0, 71), (24.0, 47.0, 3), 2, &[])
            .unwrap()
            .is_empty()
    );
    // Hourly columns give the same hours.
    assert_eq!(
        reader.top_spans(1, (0, 71), (0.0, 23.0, 24), 1, &[]).unwrap(),
        spans(&[(0, 15), (20, 23)])
    );
    // The line summarised within them ends where the country leaves the selection.
    let within = reader.top_spans(1, (0, 71), (0.0, 63.0, 8), 1, &[]).unwrap();
    let summary = reader
        .summary_within(1, (0, 71), (0.0, 63.0, 8), 0, Some(&within))
        .unwrap();
    let crossing = summary.buckets[2].unwrap();
    assert_eq!((crossing.first_hour, crossing.last_hour), (20, 23));
    assert_eq!((crossing.min_value, crossing.max_value), (1.0, 1.0));
    assert!(summary.buckets[3..6].iter().all(Option::is_none));
}

#[test]
fn a_summary_within_keeps_buckets_that_fill_a_bin_inside_one_span_as_clipping_folds_them() {
    use crate::products::country_groups::HourSpan;
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for hour in 0..400 {
        // Hourly values with a lapse, so some bins hold hours only in part.
        if (150..160).contains(&hour) {
            continue;
        }
        values.through(hour);
        values.observe(1, hour, Some(i128::from(hour % 37)), hour == 160);
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "test", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let spans =
        [(0, 99), (103, 230), (231, 260), (300, 399)].map(|(first_hour, last_hour)| HourSpan {
            first_hour,
            last_hour,
        });
    for view in [
        (0.0, 399.0, 7),
        (0.0, 399.0, 25),
        (40.0, 360.0, 100),
        (0.0, 399.0, 400),
    ] {
        let actual = reader
            .summary_within(1, (0, 399), view, 0, Some(&spans))
            .unwrap();
        // Every bucket clipped to the spans, as before buckets filling a bin were kept.
        let mut expected = reader.summary(1, (0, 399), view, 0).unwrap();
        if let Some(runs) = &mut expected.runs {
            *runs = runs_within(runs, &spans);
        }
        for bucket in &mut expected.buckets {
            *bucket = reader.bucket_within(1, *bucket, 0, &spans).unwrap();
        }
        assert_eq!(actual, expected, "{view:?}");
    }
}

/// A presenter's refusal for size alone is told apart from its other refusals, its message
/// kept, so a page halves on it until it fits, each half answered afresh for its own lines; a
/// single line too large is the only refusal.
#[test]
fn a_page_halves_on_its_presenters_budget_refusal_down_to_one_line() {
    use crate::products::{MAX_PRODUCT_REPLY_BYTES, check_reply_budget, is_over_budget, over_budget};
    // The presenter refuses more than two lines' exact runs.
    let mut answers = Vec::new();
    let reply = paged(None, 1 << 10, |page| {
        let mut read = Vec::new();
        for at in page.lines(8) {
            if !page.admit(at, &json!({ "tag": at }))? {
                break;
            }
            read.push(at);
        }
        answers.push(read.clone());
        if read.len() > 2 {
            return Err(over_budget("Air force history exceeds the reply budget"));
        }
        Ok(json!({ "series": read }))
    })
    .unwrap();
    assert_eq!(answers, [(0..8).collect::<Vec<_>>(), vec![0, 1, 2, 3], vec![0, 1]]);
    assert_eq!(reply, json!({ "series": [0, 1], "cursor": "2" }));
    // A reply measured past the budget halves as a refused one does.
    let measured = paged(Some("3"), 64, |page| {
        let mut read = Vec::new();
        for at in page.lines(7) {
            if !page.admit(at, &json!({ "tag": at }))? {
                break;
            }
            read.push(at);
        }
        Ok(json!({ "series": read, "pad": "x".repeat(10 * read.len()) }))
    })
    .unwrap();
    assert_eq!(measured["series"], json!([3, 4]));
    assert_eq!(measured["cursor"], "5");
    assert_eq!(
        over_budget("Air force history exceeds the reply budget").to_string(),
        "Air force history exceeds the reply budget"
    );
    let oversized = "x".repeat(MAX_PRODUCT_REPLY_BYTES);
    assert!(is_over_budget(&check_reply_budget(&oversized).unwrap_err()));
    assert!(!is_over_budget(&io::Error::other("exceeds the reply budget")));
    assert!(!is_over_budget(&invalid("invalid summary cursor")));
    let refused = paged(None, 1 << 10, |page| {
        assert!(page.admit(0, &json!({ "tag": 0 }))?);
        Err(over_budget("Air force history exceeds the reply budget"))
    })
    .unwrap_err();
    assert!(!is_over_budget(&refused), "a lone line is refused, not halved again");
}
