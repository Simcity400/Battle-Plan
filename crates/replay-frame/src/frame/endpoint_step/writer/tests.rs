use super::*;
use crate::rows::{ClockRow, CountryRow};

/// A fixed xorshift sequence, so every run checks the same states.
struct Sequence(u64);
impl Sequence {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn chance(&mut self, one_in: u64) -> bool {
        self.next() % one_in == 0
    }
    /// Ascending keys below `universe`, each kept one time in `one_in`.
    fn keys(&mut self, universe: u128, one_in: u64) -> Vec<u128> {
        (0..universe).filter(|_| self.chance(one_in)).collect()
    }
}

/// Any sets the writer can hold, whether or not a recording could reach them.
fn pending(sequence: &mut Sequence, universe: u128) -> Pending {
    let mut edits = Vec::new();
    for key in sequence.keys(universe, 3) {
        edits.push((key, sequence.next() & 0b1111, sequence.chance(3)));
    }
    Pending {
        reset: sequence.chance(4),
        known: true,
        rebuilt: sequence.chance(2),
        members: sequence.keys(universe, 2),
        introduced: sequence.keys(universe, 6),
        edits,
        deleted: sequence.keys(universe, 6),
        boundaries: Vec::new(),
    }
}

/// Deletions of present and absent keys, and changes of old and new keys,
/// some of them also deleted.
fn delta(sequence: &mut Sequence, universe: u128) -> (Vec<DeltaRow>, Vec<u128>) {
    let deleted = sequence.keys(universe, 5);
    let mut changed = Vec::new();
    for key in sequence.keys(universe, 3) {
        if deleted.binary_search(&key).is_ok() && !sequence.chance(4) {
            continue;
        }
        changed.push(DeltaRow {
            format: 0,
            key,
            mask: 1 << (sequence.next() % 4) | sequence.next() & 0b1111,
            recorded_fields: 0,
            values: Vec::new(),
        });
    }
    (changed, deleted)
}

#[test]
fn ascending_deltas_leave_every_set_and_boundary_as_applying_them_key_by_key() {
    let mut sequence = Sequence(0x9e37_79b9_7f4a_7c15);
    for round in 0..3_000 {
        let universe = [6, 48, 700][round % 3];
        let mut each = pending(&mut sequence, universe);
        let mut ascending = each.clone();
        for hour in 0..16 {
            if sequence.chance(8) {
                each.reset = !each.reset;
                ascending.reset = each.reset;
            }
            let (changed, deleted) = delta(&mut sequence, universe);
            let boundary = || MembershipBoundary {
                table: TableId::Countries,
                hour,
                kind: BoundaryKind::Delta,
                inserted: Vec::new(),
                deleted: Vec::new(),
            };
            let (mut by_key, mut walked) = (boundary(), boundary());
            each.apply_each(&changed, &deleted, 0b1111, &mut by_key)
                .unwrap();
            ascending
                .apply_ascending(&changed, &deleted, 0b1111, &mut walked)
                .unwrap();
            assert_eq!(walked, by_key, "round {round} hour {hour}");
            assert_eq!(ascending, each, "round {round} hour {hour}");
        }
    }
}

const TABLES: &[TableId] = &[TableId::Clock, TableId::Countries];

fn frame(hour: Hour, countries: &[u32]) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 3,
        hour,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not acquired".into(),
            }
            .into()
        }),
    };
    frame.tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
        hour,
        ..Default::default()
    }]))
    .into();
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        countries
            .iter()
            .map(|&id| CountryRow {
                id,
                canonical_slot: id,
                command_power_raw: i64::from(hour),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame
}

/// A writer that emitted its first step at hour 1, its decoder and encoder.
fn emitted() -> (EndpointWriter, FrameEncoder, FrameDecoder) {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(TABLES);
    let mut writer = EndpointWriter::for_tables(TABLES);
    for hour in 0..=1 {
        let raw = encoder.encode(frame(hour, &[1, 2]), hour == 0).unwrap();
        let decoded = decoder.advance(&raw).unwrap();
        writer.observe(&raw, decoded).unwrap();
        if hour == 0 {
            writer.finish(decoded, &mut Vec::new()).unwrap();
        }
    }
    (writer, encoder, decoder)
}

#[test]
fn emitted_steps_validate_every_new_allocation_and_the_clock_at_every_hour() {
    // A new allocation is validated however long its table kept the last one.
    let (mut writer, mut encoder, mut decoder) = emitted();
    let raw = encoder.encode(frame(2, &[1, 2]), false).unwrap();
    let decoded = decoder.advance(&raw).unwrap().clone();
    writer.observe(&raw, &decoded).unwrap();
    writer.finish(&decoded, &mut Vec::new()).unwrap();
    let raw = encoder.encode(frame(3, &[1, 2]), false).unwrap();
    let decoded = decoder.advance(&raw).unwrap();
    writer.observe(&raw, decoded).unwrap();
    let mut unordered = decoded.clone();
    unordered.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 2,
            canonical_slot: 2,
            ..Default::default()
        },
        CountryRow {
            id: 1,
            canonical_slot: 1,
            ..Default::default()
        },
    ]))
    .into();
    let mut out = vec![7];
    let error = writer.finish(&unordered, &mut out).unwrap_err();
    assert!(error.to_string().contains("ascending"), "{error}");
    assert_eq!(out, [7]);

    // The clock an emitted step validated is checked again at the next hour.
    let (mut writer, mut encoder, mut decoder) = emitted();
    let raw = encoder.encode(frame(2, &[1, 2]), false).unwrap();
    let held = decoder.advance(&raw).unwrap().clone();
    writer.observe(&raw, &held).unwrap();
    writer.finish(&held, &mut Vec::new()).unwrap();
    let raw = encoder.encode(frame(3, &[1, 2]), false).unwrap();
    let decoded = decoder.advance(&raw).unwrap();
    writer.observe(&raw, decoded).unwrap();
    let mut stale = decoded.clone();
    stale.tables[TableId::Clock.index()] = held.tables[TableId::Clock.index()].clone();
    let error = writer.finish(&stale, &mut Vec::new()).unwrap_err();
    assert!(error.to_string().contains("clock row hour"), "{error}");
    // The table it validated, still that allocation, is written as before.
    let mut kept = decoded.clone();
    kept.tables[TableId::Countries.index()] = held.tables[TableId::Countries.index()].clone();
    let mut fresh = EndpointWriter::resume(TABLES, &held).unwrap();
    fresh.observe(&raw, &kept).unwrap();
    let (mut expected, mut actual) = (Vec::new(), Vec::new());
    fresh.finish(&kept, &mut expected).unwrap();
    writer.finish(&kept, &mut actual).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn same_roots_are_frames_of_one_point_metrics_allocations_and_sample_hours() {
    let state = frame(4, &[1, 2, 5]);
    let root = |frame: &DecodedFrame| {
        let mut out = Vec::new();
        EndpointWriter::root(frame, TABLES, &mut out).unwrap();
        out
    };
    let copy = state.clone();
    assert!(EndpointWriter::same_root(TABLES, &state, &copy));
    assert_eq!(root(&copy), root(&state));
    let mut unselected = state.clone();
    unselected.tables[TableId::DivisionStatus.index()] = TableView::NotRecorded {
        mode: CaptureMode::Map,
    }
    .into();
    assert!(EndpointWriter::same_root(TABLES, &state, &unselected));
    assert_eq!(root(&unselected), root(&state));
    let mut moved = state.clone();
    let rows = (*moved.tables[TableId::Countries.index()]).clone();
    moved.tables[TableId::Countries.index()] = rows.into();
    assert!(!EndpointWriter::same_root(TABLES, &state, &moved));
    let mut sampled = state.clone();
    let view = sampled.tables[TableId::Countries.index()].clone();
    sampled.tables[TableId::Countries.index()] = view.with_sampled_at(Some(4));
    assert!(!EndpointWriter::same_root(TABLES, &state, &sampled));
    assert_ne!(root(&sampled), root(&state));
    let mut metered = state.clone();
    metered.metrics.total_ns = 9;
    assert!(!EndpointWriter::same_root(TABLES, &state, &metered));
    assert_ne!(root(&metered), root(&state));
    let mut later = state.clone();
    later.hour = 5;
    assert!(!EndpointWriter::same_root(TABLES, &state, &later));
}
