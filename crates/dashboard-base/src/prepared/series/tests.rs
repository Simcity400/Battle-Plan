#[cfg(test)]
use super::*;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::{fs, path::PathBuf};

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
    /// Every committed payload by section and range: the documented manifest
    /// header names them, and each is opened through the real store reader.
    pub fn payloads(&self) -> BTreeMap<(String, u64), Vec<u8>> {
        let mut manifest = fs::File::open(self.directory.join("manifest")).unwrap();
        let mut length = [0; 8];
        manifest.read_exact(&mut length).unwrap();
        let header: Value =
            serde_json::from_reader(manifest.take(u64::from_le_bytes(length))).unwrap();
        let mut payloads = BTreeMap::new();
        for section in header["sections"].as_array().unwrap() {
            let name = section["name"].as_str().unwrap();
            for range in 0..section["count"].as_u64().unwrap() {
                let mut bytes = Vec::new();
                self.store
                    .read(&self.key, name, range)
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
    actual: &BTreeMap<(String, u64), Vec<u8>>,
    expected: &BTreeMap<(String, u64), Vec<u8>>,
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

#[cfg(feature = "slow-tests")]
#[test]
fn active_run_5000_keys_keep_frame_work_flat_at_tenfold_hours() {
    let first = active_run_work_and_bytes(5_000, 10);
    let tenfold = active_run_work_and_bytes(5_000, 100);
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
            *data = serde_json::from_reader(
                files
                    .store
                    .read(&files.key, "boundary-checkpoint", 0)
                    .unwrap()
                    .unwrap(),
            )
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
            *data = serde_json::from_reader(
                files
                    .store
                    .read(&files.key, "active-checkpoint", 0)
                    .unwrap()
                    .unwrap(),
            )
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
            sparse = serde_json::from_reader(
                sparse_files
                    .store
                    .read(&sparse_files.key, "active-checkpoint", 0)
                    .unwrap()
                    .unwrap(),
            )
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
        rows.extend(page.rows.into_iter().map(|row| row.row));
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn files_follow_commits_and_hold_all_five_thousand_keys() {
    let fixture = Fixture::new();
    let mut states: BTreeMap<_, _> = (0..5000).map(|key| (key, KeyState::default())).collect();
    for first in [0, 5000] {
        let mut batch = Batch::default();
        batch.through(first);
        batch.through(first + 4999);
        for (&key, state) in &mut states {
            batch.push(
                key,
                state,
                Run::new(first, first + 4999, i128::from(key), first == 0),
            );
        }
        fixture.publish(batch, &mut states);
    }
    assert_eq!(
        fixture.store.range_count(&fixture.key, "test").unwrap(),
        Some(2)
    );
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(&fixture.store, &fixture.key, "test".into(), &flag).unwrap();
    let value = reader.value_at(4999, 9999).unwrap().unwrap();
    assert_eq!(
        (value.first_hour, value.last_hour, value.value),
        (0, 9999, 4999)
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
        assert_eq!(row.row, json!([hour, key, index % 3]));
        assert_eq!(
            (row.hour, row.key, row.ordinal),
            (hour as i32, key, hour as u64 * 3 + index as u64 % 3)
        );
    }
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
            assert!(
                json["buckets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|bucket| !bucket.is_null())
                    .all(|bucket| bucket["absoluteTotal"].is_string())
            );
            assert_eq!(
                serde_json::from_value::<ChartSummary>(json).unwrap(),
                summary
            );
        }
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn values_at_matches_single_key_reads_across_commits_resume_append_and_gaps_at_5000_keys() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for (batch, (first, last)) in [(100, 101), (102, 103), (106, 107)].into_iter().enumerate() {
        for hour in first..=last {
            for key in 0..5_000 {
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
    let keys: Vec<_> = (0..=5_000).rev().chain([0, 1]).collect();
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
fn late_index_skips_five_thousand_late_events_with_one_ordinary_read() {
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut values = Accumulator::default();
    let mut late = LateIndex::default();
    for ordinal in 0..5_000 {
        values
            .event(0, 100, &json!({"native": 99, "ordinal": ordinal}))
            .unwrap();
        late.insert(99, 100, 0, ordinal);
    }
    values
        .event(0, 100, &json!({"native": 100, "ordinal": 5_000}))
        .unwrap();
    late.insert(100, 100, 0, 5_000); // Ordinary facts are never indexed.
    assert_eq!(late.ordinals[&0], [(0, 4_999)]);
    let saved = serde_json::to_value(&late).unwrap();
    assert_eq!(
        saved,
        serde_json::to_value(&late.rows).unwrap(),
        "no transient field is persisted"
    );
    late = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(
        late.ordinals[&0],
        [(0, 4_999)],
        "load rebuilds the coalesced spans"
    );
    assert_eq!(serde_json::to_value(&late).unwrap(), saved);

    let mut writer = fixture.writer();
    values
        .flush(&mut writer, "late-ordinary", 0, &cancelled)
        .unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        "late-ordinary".into(),
        &cancelled,
    )
    .unwrap();
    let mut cursor = 0;
    let mut requests = 0;
    let mut reached = Vec::new();
    while let Some(ordinary) = late
        .next_ordinary(0, cursor)
        .filter(|ordinal| *ordinal <= 5_000)
    {
        requests += 1;
        assert!(
            requests <= 1,
            "the ordinary walk never reads its 5,000 late-row prefix"
        );
        let page = reader.events(&[0], 100, 100, ordinary, 1).unwrap();
        assert_eq!(page.rows.len(), 1);
        let row = &page.rows[0];
        if row.row["native"] == 100 {
            reached.push(row.ordinal);
        }
        cursor = row.ordinal + 1;
    }
    assert_eq!(reached, [5_000]);
    assert_eq!(requests, 1);
    assert_eq!(late.previous_ordinary(0, 5_000), Some(5_000));
    assert_eq!(
        late.previous_ordinary(0, 4_999),
        None,
        "a backward walk skips the entire late prefix too"
    );
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
fn published(accumulator: &mut Accumulator) -> (Value, BTreeMap<(String, u64), Vec<u8>>) {
    let state = serde_json::to_value(&*accumulator).unwrap();
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
