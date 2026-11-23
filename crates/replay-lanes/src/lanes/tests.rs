use super::*;
use replay_journal::fixtures;

#[test]
fn lifetimes_read_past_levels_a_run_closes_no_piece_of() {
    let recording = Recording::open(fixtures::native_recording(recorder_derive::writers(), 20, |_| {})).unwrap();
    let lives = lifetimes(&recording, 0, TableId::Countries).unwrap();
    assert!(!lives.is_empty());
}

#[test]
fn coarse_chart_names_level_zero_buckets() {
    let recording = Recording::open(fixtures::native_recording(recorder_derive::writers(), 20, |_| {})).unwrap();
    let source = replay_recording::summaries::registry()
        .unwrap()
        .into_iter()
        .find_map(|(table, shape)| match shape {
            Shape::Values(columns) if table == TableId::Countries.index() => {
                let power = (0..TableId::Countries.field_count()).find(|&field| TableId::Countries.field_name(field) == Some("political_power_raw"))?;
                columns.iter().find(|column| matches!(column.source, summary::Source::Cell(cell) if cell == power)).map(|column| column.source.id())
            }
            _ => None,
        })
        .unwrap();
    let extent = recording.table_entry(0, 0, TableId::Countries.index()).unwrap().summary.unwrap();
    let bytes = recording.stored_bytes(&extent).unwrap();
    let piece = Piece::read(&bytes).unwrap();
    assert_eq!(piece.rank(piece.column_of(source).unwrap()), 0);
    let (from, to, bin_hours) = (100, 100 + 20 * UNIT_ORDINALS as i32 - 1, summary::bucket_hours(1) * 4);
    let coarse = chart_at(&recording, 0, TableId::Countries, source, None, from, to, 1, bin_hours).unwrap();
    let walked = chart_at(&recording, 0, TableId::Countries, source, None, from, to, WALKED, bin_hours).unwrap();
    assert!(coarse.pieces > 0);
    assert_eq!(coarse.keys, walked.keys);
    let ratio = (summary::bucket_hours(1) / summary::bucket_hours(0)) as i32;
    let (mut compared, mut moved) = (0, 0);
    for at in 0..coarse.keys.len() {
        let (coarse, walked) = (coarse.values(at), walked.values(at));
        for bin in (1..coarse.len() - 1).filter(|&bin| i64::from(walked[bin].hours) == bin_hours) {
            let mut expected = walked[bin];
            expected.min_bucket = expected.min_bucket.div_euclid(ratio) * ratio;
            expected.max_bucket = expected.max_bucket.div_euclid(ratio) * ratio;
            assert_eq!(coarse[bin], expected, "bin {bin}");
            compared += 1;
            moved += usize::from(walked[bin].min != walked[bin].max);
        }
    }
    assert!(compared > 0 && moved > 0, "{compared} bins compared, {moved} holding more than one value");
}

fn stepped(recording: &Recording, run: u32, table: TableId, column: usize, from: i32, to: i32) -> Vec<(u128, Runs)> {
    let mut reader = TableReader::new(recording, run, table, Some(&[column])).unwrap();
    reader.index_keys();
    let held = recording.run(run).unwrap();
    reader.seek(recording, held.ordinal(from).unwrap()).unwrap();
    let mut out: HashMap<u128, Runs> = HashMap::new();
    for hour in from..=to {
        if hour > from {
            assert!(reader.step(recording).unwrap());
        }
        let view = reader.view();
        let mut seen = Vec::new();
        for slot in view.rows() {
            let key = view.key(slot).unwrap();
            let shown = reader.available() && reader.read_rows().is_none_or(|read| read.binary_search(&slot).is_ok());
            let value = shown.then(|| view.number(column, slot)).flatten();
            let runs = out.entry(key).or_default();
            match runs.last_mut() {
                Some((_, last, now)) if *now == value && *last + 1 == hour => *last = hour,
                _ => runs.push((hour, hour, value)),
            }
            seen.push(key);
        }
        for (key, runs) in &mut out {
            if !seen.contains(key) {
                match runs.last_mut() {
                    Some((_, last, None)) if *last + 1 == hour => *last = hour,
                    _ => runs.push((hour, hour, None)),
                }
            }
        }
    }
    let mut out: Vec<(u128, Runs)> = out.into_iter().collect();
    out.sort_unstable_by_key(|(key, _)| *key);
    out
}

#[test]
fn runs_and_next_from_tracks_are_the_hours_stepped() {
    const RUNS: [(u64, i32, u32); 3] = [(1, 100, 5), (2, 50_000, 3), (1, 100 + 5 * UNIT_ORDINALS as i32, 2)];
    let (bytes, _) = fixtures::churned(recorder_derive::writers(), &RUNS);
    let recording = Recording::open(bytes).unwrap();
    let field = |table: TableId, name: &str| (0..table.field_count()).find(|&field| table.field_name(field) == Some(name)).unwrap();
    let columns = [
        (TableId::Countries, field(TableId::Countries, "stability_raw")),
        (TableId::Countries, field(TableId::Countries, "political_power_raw")),
        (TableId::WorldTension, field(TableId::WorldTension, "total_raw")),
    ];
    let mut compared = 0;
    for run in 0..recording.run_count() {
        let held = recording.run(run).unwrap();
        let (first, last) = (held.first_hour, held.last_hour());
        for (from, to) in [(first, last), (first + 333, last - 17), (first + 719, first + 720)] {
            for &(table, column) in &columns {
                let read = super::runs(&recording, run, table, column, from, to).unwrap_or_else(|error| panic!("run {run} {table:?} column {column} {from}..={to}: {error}"));
                assert_eq!(read, stepped(&recording, run, table, column, from, to), "run {run} {table:?} column {column} {from}..={to}");
                compared += 1;
            }
        }
        for (key, after) in [(2_u128, first + 5), (4, first + 50), (1, first + 3), (3, first + 1000)] {
            if after >= last {
                continue;
            }
            let column = field(TableId::Countries, "stability_raw");
            let reference = stepped(&recording, run, TableId::Countries, column, after, last);
            let expected = reference.iter().find(|(held, _)| *held == key).and_then(|(_, runs)| runs.get(1).map(|run| run.0));
            assert_eq!(next(&recording, run, TableId::Countries, column, key, after).unwrap(), expected, "run {run} key {key} after {after}");
        }
    }
    assert!(compared > 0);
}

#[test]
fn a_row_inserted_inside_a_block_runs_from_the_cells_it_was_inserted_with() {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::CountryRow;
    const RUNS: [(u64, i32, u32); 1] = [(1, 100, 3)];
    let (bytes, _) = fixtures::written(recorder_derive::writers(), b"inserted synthetic", &RUNS, &[TableId::Countries], |synthetic, _, ordinal| {
        let mut rows = vec![CountryRow { id: 1, stability_raw: 1, political_power_raw: Some(5), ..Default::default() }];
        if (37..90).contains(&(ordinal % 100)) {
            rows.push(CountryRow { id: 2, stability_raw: 70_000 + i64::from(ordinal / 100), political_power_raw: Some(-3_225), ..Default::default() });
        }
        if ordinal >= 410 {
            rows.push(CountryRow { id: 3, stability_raw: 9, political_power_raw: Some(1_000 + i64::from(ordinal / 50)), ..Default::default() });
        }
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
    });
    let recording = Recording::open(bytes).unwrap();
    let held = recording.run(0).unwrap();
    let (first, last) = (held.first_hour, held.last_hour());
    let field = |name: &str| (0..TableId::Countries.field_count()).find(|&field| TableId::Countries.field_name(field) == Some(name)).unwrap();
    for name in ["stability_raw", "political_power_raw"] {
        let column = field(name);
        for (from, to) in [(first, last), (first + 137, first + 137), (first + 130, first + 150), (first + 505, first + 520)] {
            let expected = stepped(&recording, 0, TableId::Countries, column, from, to);
            assert_eq!(super::runs(&recording, 0, TableId::Countries, column, from, to).unwrap(), expected, "{name} {from}..={to}");
            let keyed = super::keyed_runs(&recording, 0, TableId::Countries, summary::Source::Cell(column), Some(&[2, 3]), from, to).unwrap();
            for key in [2_u128, 3] {
                let valued = |runs: &[(i32, i32, Option<i64>)]| runs.iter().copied().filter(|run| run.2.is_some()).collect::<Vec<_>>();
                let stepped_runs = expected.iter().find(|(held, _)| *held == key).map_or_else(Vec::new, |(_, runs)| valued(runs));
                assert_eq!(keyed.get(&key).map_or_else(Vec::new, |runs| valued(runs)), stepped_runs, "{name} key {key} {from}..={to}");
            }
        }
    }
}

fn ranked_recording(column: usize, rank: u8) -> Recording {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::CountryRow;
    const RUNS: [(u64, i32, u32); 2] = [(1, 103, 20), (2, 60_001, 3)];
    let mut registry = replay_recording::summaries::registry().unwrap();
    let (_, shape) = registry.iter_mut().find(|(table, _)| *table == TableId::Countries.index()).unwrap();
    let Shape::Values(columns) = shape else { panic!("Countries has no value summaries") };
    let power = columns.iter_mut().find(|held| matches!(held.source, summary::Source::Cell(cell) if cell == column)).unwrap();
    power.rank = rank;
    let (bytes, _) = fixtures::written_with_summaries(recorder_derive::writers(), b"ranked synthetic", &RUNS, &[TableId::Countries], registry, |synthetic, run, ordinal| {
        let input = if run == 0 && (3000..3200).contains(&ordinal) {
            synthetic.table::<CountryRow>(TableId::Countries, Input::Unavailable { reason: "refused", read: false, sample: Sample::FromReadBit })
        } else {
            let rows: Vec<CountryRow> = (1..=12_u32)
                .filter(|&id| (ordinal / (29 + id * 13)) % 5 != 0)
                .map(|id| CountryRow {
                    id,
                    political_power_raw: ((ordinal + id) % 97 != 0).then(|| i64::from((ordinal / 24 + id * 5) % 11) * 3),
                    ..Default::default()
                })
                .collect();
            synthetic.table(TableId::Countries, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit })
        };
        input.unwrap();
    });
    Recording::open(bytes).unwrap()
}

#[test]
fn a_ranked_view_from_the_ranks_is_the_view_folding_every_key() {
    let rank = 4;
    let column = (0..TableId::Countries.field_count()).find(|&field| TableId::Countries.field_name(field) == Some("political_power_raw")).unwrap();
    let recording = ranked_recording(column, rank);
    let extent = recording.table_entry(0, 0, TableId::Countries.index()).unwrap().summary.unwrap();
    let bytes = recording.stored_bytes(&extent).unwrap();
    let piece = Piece::read(&bytes).unwrap();
    assert_eq!(piece.rank(piece.column_of(column as u32).unwrap()), rank);
    let rank = usize::from(rank);
    let held = recording.run(0).unwrap();
    let refused = top(&recording, 0, TableId::Countries, column as u32, held.first_hour, held.first_hour + 59, 20, 1, By::Last).unwrap_err();
    assert_eq!(refused.to_string(), "A top-N view's bins are six hours or wider");
    let (mut compared, mut full) = (0, 0);
    for run in 0..recording.run_count() {
        let held = recording.run(run).unwrap();
        let (first, last) = (held.first_hour, held.last_hour());
        let views = [(first, last, 6), (first, last, 30), (first, last, 96), (first, last, 600), (first, last, 4800), (first + 333, last - 17, 70), (first + 7, (first + 2000).min(last), 39)];
        for (from, to, least) in views {
            let width = ((to - from + 1) / least).max(1) as u32;
            for n in [1, 2, rank, rank + 1] {
                let read = top(&recording, run, TableId::Countries, column as u32, from, to, width, n, By::Last).unwrap();
                let folded = top_folding(&recording, run, TableId::Countries, column as u32, (from, to, width), n, By::Last).unwrap();
                assert_eq!(read, folded, "run {run} {from}..={to} width {width} n {n}");
                full += read.iter().filter(|bin| bin.len() == n).count();
                compared += 1;
            }
        }
    }
    assert!(compared == 56 && full > 100, "{compared} views, {full} full bins");
}

fn stepped_reads(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> Vec<(i32, i32)> {
    let mut reader = TableReader::new(recording, run, table, Some(&[0])).unwrap();
    reader.seek(recording, recording.run(run).unwrap().ordinal(from).unwrap()).unwrap();
    let mut out: Vec<(i32, i32)> = Vec::new();
    for hour in from..=to {
        if hour > from {
            assert!(reader.step(recording).unwrap());
        }
        if reader.sampled_at() == Some(hour) {
            match out.last_mut() {
                Some((_, last)) if *last + 1 == hour => *last = hour,
                _ => out.push((hour, hour)),
            }
        }
    }
    out
}

#[test]
fn reads_from_page_entries_are_the_hours_stepped_sampled_at_themselves() {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::CountryRow;
    const RUNS: [(u64, i32, u32); 2] = [(1, 100, 4), (2, 40_000, 2)];
    let (bytes, _) = fixtures::written(recorder_derive::writers(), b"read hours synthetic", &RUNS, &[TableId::Countries], |synthetic, run, ordinal| {
        let rows = [CountryRow { id: 1, stability_raw: 1 + i64::from(ordinal / 24), ..Default::default() }];
        let snapshot = |read: bool, sample: Sample| Input::Snapshot { rows: &rows, partial: None, read, sample };
        let input = match ordinal {
            0 | 700..=760 => snapshot(true, Sample::FromReadBit),
            1_000..=1_010 => Input::Unavailable { reason: "refused", read: false, sample: Sample::FromReadBit },
            1_500 => snapshot(false, Sample::Reset),
            daily if daily % 24 == 5 + run => snapshot(true, Sample::FromReadBit),
            _ => snapshot(false, Sample::FromReadBit),
        };
        synthetic.table(TableId::Countries, input).unwrap();
    });
    let recording = Recording::open(bytes).unwrap();
    let mut daily = 0;
    for run in 0..recording.run_count() {
        let held = recording.run(run).unwrap();
        let (first, last) = (held.first_hour, held.last_hour());
        for (from, to) in [(first, last), (first + 333, last - 17), (first + 719, first + 721), (first + 1_000, first + 1_000), (first + 1_490, first + 1_530)] {
            if to > last {
                continue;
            }
            let read = reads(&recording, run, TableId::Countries, from, to).unwrap();
            assert_eq!(read, stepped_reads(&recording, run, TableId::Countries, from, to), "run {run} {from}..={to}");
            daily += read.iter().filter(|(start, end)| start == end).count();
        }
        let whole = reads(&recording, run, TableId::Countries, first, last).unwrap();
        assert!(whole.contains(&(first + 700, first + 760)), "run {run}: an hourly stretch across a block's end is one run: {whole:?}");
    }
    assert!(daily > 100, "{daily} single-hour reads compared");
}

fn histories() -> Recording {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::{CountryRow, FleetRow, Recorded};
    const RUNS: [(u64, i32, u32); 2] = [(1, 100, 3), (2, 40_000, 2)];
    let (bytes, _) = fixtures::written(recorder_derive::writers(), b"histories synthetic", &RUNS, &[TableId::Countries, TableId::Fleets], |synthetic, _, ordinal| {
        let tag = match ordinal {
            0..=99 => "first".to_owned(),
            100 => "second".to_owned(),
            101..=719 => "third".to_owned(),
            720..=999 => "fourth".to_owned(),
            _ => format!("late {}", ordinal / 50 % 3),
        };
        let mut countries = vec![CountryRow {
            id: 1,
            tag: tag.into_bytes(),
            stability_raw: i64::from(ordinal / 30),
            political_power_raw: (ordinal % 53 != 0).then(|| i64::from(ordinal / 10 % 7)),
            ..Default::default()
        }];
        if !(300..340).contains(&ordinal) && !(700..760).contains(&ordinal) {
            let tag = if ordinal < 760 { b"two".to_vec() } else { b"two again".to_vec() };
            countries.push(CountryRow { id: 2, tag, stability_raw: 2, ..Default::default() });
        }
        if (1500..1600).contains(&ordinal) {
            countries.push(CountryRow { id: 3, tag: b"three".to_vec(), stability_raw: i64::from(ordinal), ..Default::default() });
        }
        countries.push(CountryRow { id: 4, tag: format!("cycle {}", ordinal / 50 % 40).into_bytes(), ..Default::default() });
        let input = if (1000..1010).contains(&ordinal) || (1435..1445).contains(&ordinal) {
            Input::Unavailable { reason: "refused", read: false, sample: Sample::FromReadBit }
        } else if ordinal % 97 == 50 {
            Input::Snapshot { rows: &countries[countries.len() - 1..], partial: Some(1), read: true, sample: Sample::FromReadBit }
        } else {
            Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }
        };
        synthetic.table(TableId::Countries, input).unwrap();
        let name = match ordinal {
            0..=99 => b"home".to_vec(),
            100 => b"second".to_vec(),
            101..=719 => b"third".to_vec(),
            _ => b"fourth".to_vec(),
        };
        let name = if (400..410).contains(&ordinal) { Recorded::NotRecorded } else { Recorded::Value(Some(name)) };
        let mut fleets = vec![FleetRow { id: 10, owner: 1, name, ..Default::default() }];
        if !(200..261).contains(&ordinal) {
            let name = if ordinal < 900 { b"eleven".to_vec() } else { b"eleven again".to_vec() };
            fleets.push(FleetRow { id: 11, owner: 1, name: Recorded::Value(Some(name)), ..Default::default() });
        }
        let input = if (1200..1206).contains(&ordinal) {
            Input::Unavailable { reason: "refused", read: false, sample: Sample::FromReadBit }
        } else {
            Input::Snapshot { rows: &fleets, partial: None, read: true, sample: Sample::FromReadBit }
        };
        synthetic.table(TableId::Fleets, input).unwrap();
    });
    Recording::open(bytes).unwrap()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    Number(i64),
    Text(Vec<u8>),
    Missing,
    Unrecorded,
    Bytes(Vec<u8>),
}

type SeenRuns = Vec<(u128, Vec<(i32, i32, Option<Vec<Seen>>)>)>;

fn stepped_cells(recording: &Recording, run: u32, table: TableId, columns: &[usize], keys: Keys<'_>, from: i32, to: i32) -> SeenRuns {
    use replay_columnar::store::Value;
    let mut reader = TableReader::new(recording, run, table, Some(columns)).unwrap();
    reader.index_keys();
    reader.seek(recording, recording.run(run).unwrap().ordinal(from).unwrap()).unwrap();
    let wants = |key: u128| match keys {
        Keys::All => true,
        Keys::These(these) => these.contains(&key),
        Keys::Range(low, high) => low <= key && key <= high,
        Keys::Ranges(ranges) => ranges.iter().any(|&(low, high)| low <= key && key <= high),
    };
    let mut out: std::collections::BTreeMap<u128, Vec<(i32, i32, Option<Vec<Seen>>)>> = std::collections::BTreeMap::new();
    if let Keys::These(these) = keys {
        for &key in these {
            out.entry(key).or_default();
        }
    }
    for hour in from..=to {
        if hour > from {
            assert!(reader.step(recording).unwrap());
        }
        let view = reader.view();
        for slot in view.rows() {
            let key = view.key(slot).unwrap();
            if wants(key) {
                out.entry(key).or_default();
            }
        }
        for (&key, runs) in &mut out {
            let shown = view.slot(key).filter(|&slot| reader.available() && reader.read_rows().is_none_or(|read| read.binary_search(&slot).is_ok()));
            let seen = shown.map(|slot| {
                columns
                    .iter()
                    .map(|&column| match view.value(column, slot) {
                        Value::Number(bits) => Seen::Number(bits as i64),
                        Value::Text(id) => Seen::Text(view.string(id).unwrap().to_vec()),
                        Value::Missing => Seen::Missing,
                        Value::Unrecorded => Seen::Unrecorded,
                        Value::List(bytes) => Seen::Bytes(bytes.to_vec()),
                    })
                    .collect()
            });
            match runs.last_mut() {
                Some((_, last, now)) if *now == seen => *last = hour,
                _ => runs.push((hour, hour, seen)),
            }
        }
    }
    out.into_iter().collect()
}

fn seen(recording: &Recording, run: u32, table: TableId, history: &History) -> SeenRuns {
    let asks: Vec<(i32, u128)> = history
        .keys
        .iter()
        .flat_map(|(key, steps)| {
            steps.iter().filter(|step| step.cells.as_ref().is_some_and(|cells| cells.iter().any(|cell| matches!(cell, Cell::Written(_))))).map(move |step| (step.hour, *key))
        })
        .collect();
    let mut bytes: HashMap<(i32, u128), Vec<Option<Vec<u8>>>> = HashMap::new();
    at_each(recording, run, table, Some(&history.columns), &asks, &mut |hour, key, row| {
        let (view, slot) = row.unwrap_or_else(|| panic!("key {key}'s step at {hour} shows its row, so the batch read does"));
        bytes.insert((hour, key), history.columns.iter().map(|&column| view.list(column, slot).map(<[u8]>::to_vec)).collect());
        Ok(())
    })
    .unwrap();
    assert_eq!(bytes.len(), asks.len(), "each ask read once");
    history
        .keys
        .iter()
        .map(|(key, steps)| {
            let runs = steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    let last = steps.get(index + 1).map_or(history.to, |next| next.hour - 1);
                    let seen = step.cells.as_ref().map(|cells| {
                        cells
                            .iter()
                            .enumerate()
                            .map(|(at, &cell)| match cell {
                                Cell::Number(number) => Seen::Number(number),
                                Cell::Text(_) => Seen::Text(history.text(cell).unwrap().to_vec()),
                                Cell::Missing => Seen::Missing,
                                Cell::Unrecorded => Seen::Unrecorded,
                                Cell::Written(_) => Seen::Bytes(bytes[&(step.hour, *key)][at].clone().unwrap()),
                            })
                            .collect()
                    });
                    (step.hour, last, seen)
                })
                .collect();
            (*key, runs)
        })
        .collect()
}

fn field(table: TableId, name: &str) -> usize {
    (0..table.field_count()).find(|&field| table.field_name(field) == Some(name)).unwrap()
}

fn checked(recording: &Recording, run: u32, table: TableId, columns: &[usize], keys: Keys<'_>, from: i32, to: i32) -> History {
    let read = history(recording, run, table, columns, keys, from, to).unwrap_or_else(|error| panic!("run {run} {table:?} {columns:?} {keys:?} {from}..={to}: {error}"));
    assert_eq!(read.columns, columns);
    assert_eq!(seen(recording, run, table, &read), stepped_cells(recording, run, table, columns, keys, from, to), "run {run} {table:?} {columns:?} {keys:?} {from}..={to}");
    read
}

fn spans(recording: &Recording, run: u32) -> Vec<(i32, i32)> {
    let held = recording.run(run).unwrap();
    let (first, last) = (held.first_hour, held.last_hour());
    [(first, last), (first + 333, last - 17), (first + 99, first + 102), (first + 719, first + 720), (first + 1001, first + 1500)].into_iter().filter(|&(_, to)| to <= last).collect()
}

#[test]
fn a_text_columns_history_holds_each_change_with_its_text() {
    let recording = histories();
    let tag = field(TableId::Countries, "tag");
    for run in 0..recording.run_count() {
        for (from, to) in spans(&recording, run) {
            checked(&recording, run, TableId::Countries, &[tag], Keys::All, from, to);
        }
    }
    let first = recording.run(0).unwrap().first_hour;
    let read = checked(&recording, 0, TableId::Countries, &[tag], Keys::All, first, first + 760);
    let runs = read.runs(tag).unwrap();
    let shown: Vec<(i32, Cell)> = runs.iter().find(|(key, _)| *key == 1).unwrap().1.iter().filter_map(|&(start, _, cell)| Some((start, cell?))).collect();
    let mut changes: Vec<(i32, &[u8])> = Vec::new();
    for pair in shown.windows(2) {
        if pair[0].1 != pair[1].1 {
            changes.push((pair[1].0 - first, read.text(pair[1].1).unwrap()));
        }
    }
    assert_eq!(changes, [(100, &b"second"[..]), (101, &b"third"[..]), (720, &b"fourth"[..])]);
    assert!(shown.len() > 4, "country 1 is hidden at the hours read in part and shown again: {shown:?}");
    let whole = checked(&recording, 0, TableId::Countries, &[tag], Keys::These(&[4]), first, recording.run(0).unwrap().last_hour());
    assert_eq!(whole.texts.len(), 40, "country 4's 40 texts, each one index however many string ids it was held under");
}

#[test]
fn a_rows_history_is_hidden_while_it_is_gone_unavailable_or_unread() {
    let recording = histories();
    let mut columns = vec![field(TableId::Countries, "tag"), field(TableId::Countries, "political_power_raw"), field(TableId::Countries, "stability_raw")];
    columns.sort_unstable();
    for run in 0..recording.run_count() {
        for (from, to) in spans(&recording, run) {
            checked(&recording, run, TableId::Countries, &columns, Keys::All, from, to);
        }
    }
    let held = recording.run(0).unwrap();
    let first = held.first_hour;
    let read = checked(&recording, 0, TableId::Countries, &columns, Keys::All, first, held.last_hour());
    let hidden = |key: u128| -> Vec<(i32, i32)> {
        let steps = &read.keys.iter().find(|(held, _)| *held == key).unwrap().1;
        steps.iter().enumerate().filter(|(_, step)| step.cells.is_none()).map(|(index, step)| (step.hour - first, steps.get(index + 1).map_or(read.to, |next| next.hour - 1) - first)).collect()
    };
    let two = hidden(2);
    for gone in [(300, 339), (700, 759), (1000, 1009), (1435, 1444)] {
        assert!(two.contains(&gone), "country 2 hidden for {gone:?}: {two:?}");
    }
    assert!(hidden(1).contains(&(1435, 1444)), "the unavailable stretch across the block's end hides country 1");
    let missing = read.keys.iter().find(|(key, _)| *key == 1).unwrap().1.iter().any(|step| step.cells.as_ref().is_some_and(|cells| cells.contains(&Cell::Missing)));
    assert!(missing, "country 1's political power holds none at times");
}

#[test]
fn a_history_of_some_keys_is_those_keys_history() {
    let recording = histories();
    let tag = field(TableId::Countries, "tag");
    let mut columns = vec![tag, field(TableId::Countries, "stability_raw")];
    columns.sort_unstable();
    for run in 0..recording.run_count() {
        for (from, to) in spans(&recording, run) {
            for keys in [Keys::These(&[2, 3, 99]), Keys::These(&[3]), Keys::Range(2, 3), Keys::Range(5, 9)] {
                checked(&recording, run, TableId::Countries, &columns, keys, from, to);
            }
        }
    }
    let held = recording.run(0).unwrap();
    let (first, last) = (held.first_hour, held.last_hour());
    let three = checked(&recording, 0, TableId::Countries, &[tag], Keys::These(&[3]), first, last);
    let text = three.texts.iter().position(|text| &**text == b"three").unwrap() as u32;
    let runs = three.runs(tag).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].1,
        [(first, first + 1499, None), (first + 1500, first + 1504, Some(Cell::Text(text))), (first + 1505, first + 1505, None), (first + 1506, first + 1599, Some(Cell::Text(text))), (first + 1600, last, None)]
    );
    let none = checked(&recording, 0, TableId::Countries, &[tag], Keys::Range(5, 9), first, last);
    assert!(none.keys.is_empty());
}

#[test]
fn a_logs_history_of_some_keys_reads_every_block_its_rows_are_in() {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::{CountryLinkRow, CountryRow, OperationCompletionRow};
    let falls = |country: u32| -> &'static [u32] { if country == 7 { &[50, 900, 1700] } else { &[50, 900, 1300, 1700] } };
    let streams = [TableId::Countries, TableId::CountryLinks, TableId::OperationCompletions];
    let (bytes, _) = fixtures::written(recorder_derive::writers(), b"log synthetic", &[(1, 100, 3)], &streams, |synthetic, _, ordinal| {
        let countries: Vec<CountryRow> = [7, 9]
            .map(|id| {
                let fallen = falls(id).iter().filter(|&&at| at <= ordinal).count() as i64;
                CountryRow { id, canonical_slot: id, tag: format!("C{id}").into_bytes(), political_power_raw: Some(1_000 - fallen), command_power_raw: 50, ..Default::default() }
            })
            .into();
        let links = [7, 9].map(|id| CountryLinkRow { id, canonical_slot: id });
        let done: Vec<OperationCompletionRow> = if ordinal == 900 { vec![OperationCompletionRow { owner: 7, sequence: 1, target: 3, completion_hour: 1_000, ..Default::default() }] } else { Vec::new() };
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: &links[..], partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::OperationCompletions, Input::Snapshot { rows: &done, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
    });
    let recording = Recording::open(bytes).unwrap();
    let held = recording.run(0).unwrap();
    let (first, last) = (held.first_hour, held.last_hour());
    let completions = [field(TableId::OperationCompletions, "target")];
    let refused = history(&recording, 0, TableId::OperationCompletions, &completions, Keys::All, first, last).err().expect("a log's history is refused").to_string();
    assert!(refused.contains("OperationCompletions is a log"), "{refused}");
    let mut columns = vec![field(TableId::BalanceDrops, "before"), field(TableId::BalanceDrops, "after")];
    columns.sort_unstable();
    let country = |id: u128| (id << 32, id << 32 | u128::from(u32::MAX));
    let (low, high) = country(7);
    let seven = checked(&recording, 0, TableId::BalanceDrops, &columns, Keys::Range(low, high), first, last);
    let fell: Vec<i32> = seven.keys.iter().filter_map(|(_, steps)| steps.iter().find_map(|step| step.cells.as_ref().map(|_| step.hour - first))).collect();
    assert_eq!(fell, [50, 900, 1700], "each of country 7's drops, one in each block");
    let alone = 9u128 << 32 | u128::from((first + 1300) as u32);
    let nine = checked(&recording, 0, TableId::BalanceDrops, &columns, Keys::These(&[alone]), first + 720, last);
    assert!(nine.keys[0].1.iter().any(|step| step.cells.is_some()), "country 9's drop in the second block alone");
    let (low, high) = country(9);
    checked(&recording, 0, TableId::BalanceDrops, &columns, Keys::Range(low, high), first + 333, last - 17);
}

#[test]
fn an_opaque_columns_change_hours_read_in_one_batch_are_the_hours_stepped() {
    let recording = histories();
    let name = field(TableId::Fleets, "name");
    let mut columns = vec![name, field(TableId::Fleets, "owner")];
    columns.sort_unstable();
    for run in 0..recording.run_count() {
        for (from, to) in spans(&recording, run) {
            for keys in [Keys::All, Keys::These(&[11])] {
                checked(&recording, run, TableId::Fleets, &columns, keys, from, to);
            }
        }
    }
    let first = recording.run(0).unwrap().first_hour;
    let read = checked(&recording, 0, TableId::Fleets, &[name], Keys::These(&[10]), first, first + 760);
    let runs = read.runs(name).unwrap();
    let starts: Vec<i32> = runs[0].1.iter().skip(1).map(|&(start, ..)| start - first).collect();
    assert_eq!(starts, [100, 101, 400, 410, 720]);
    assert_eq!(runs[0].1[3].2, Some(Cell::Unrecorded));
    assert_eq!(runs[0].1[4].2, Some(Cell::Written(first + 410)), "recorded again: written at that hour");
}

#[test]
fn runs_and_next_of_a_text_column_follow_its_text() {
    let recording = histories();
    let tag = field(TableId::Countries, "tag");
    for run in 0..recording.run_count() {
        for (from, to) in spans(&recording, run) {
            let read = super::runs(&recording, run, TableId::Countries, tag, from, to).unwrap();
            let stepped = stepped_cells(&recording, run, TableId::Countries, &[tag], Keys::All, from, to);
            assert_eq!(read.iter().map(|(key, _)| *key).collect::<Vec<_>>(), stepped.iter().map(|(key, _)| *key).collect::<Vec<_>>());
            let (mut texts, mut values): (HashMap<i64, Vec<u8>>, HashMap<Vec<u8>, i64>) = (HashMap::new(), HashMap::new());
            for ((key, runs), (_, stepped)) in read.iter().zip(&stepped) {
                assert_eq!(runs.len(), stepped.len(), "run {run} key {key} {from}..={to}");
                for (&(start, last, value), (begin, end, seen)) in runs.iter().zip(stepped) {
                    assert_eq!((start, last), (*begin, *end), "run {run} key {key}");
                    match (value, seen.as_deref()) {
                        (Some(value), Some([Seen::Text(text)])) => {
                            assert_eq!(texts.entry(value).or_insert_with(|| text.clone()), text, "a value stands for one text");
                            assert_eq!(*values.entry(text.clone()).or_insert(value), value, "a text has one value");
                        }
                        (None, None) => {}
                        other => panic!("run {run} key {key} at {start}: {other:?}"),
                    }
                }
            }
        }
    }
    let held = recording.run(0).unwrap();
    let (first, last) = (held.first_hour, held.last_hour());
    for after in [first + 60, first + 100, first + 101, first + 148, first + 715, first + 2000] {
        let reference = stepped_cells(&recording, 0, TableId::Countries, &[tag], Keys::These(&[1]), after, last);
        let expected = reference[0].1.get(1).map(|run| run.0);
        assert_eq!(next(&recording, 0, TableId::Countries, tag, 1, after).unwrap(), expected, "after {after}");
    }
    assert_eq!(next(&recording, 0, TableId::Countries, tag, 1, first + 60).unwrap(), Some(first + 100));
}

fn stepped_statuses(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> Vec<StatusRun> {
    let mut reader = TableReader::new(recording, run, table, Some(&[0])).unwrap();
    reader.seek(recording, recording.run(run).unwrap().ordinal(from).unwrap()).unwrap();
    let mut out: Vec<StatusRun> = Vec::new();
    for hour in from..=to {
        if hour > from {
            assert!(reader.step(recording).unwrap());
        }
        let status = reader.status();
        match out.last_mut() {
            Some(held) if held.status == status => held.last = hour,
            _ => out.push(StatusRun { first: hour, last: hour, status }),
        }
    }
    out
}

#[test]
fn statuses_from_page_entries_are_the_statuses_stepped() {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::CountryRow;
    const RUNS: [(u64, i32, u32); 2] = [(1, 100, 4), (2, 40_000, 2)];
    let (bytes, _) = fixtures::written(recorder_derive::writers(), b"status runs synthetic", &RUNS, &[TableId::Countries], |synthetic, run, ordinal| {
        let rows = [CountryRow { id: 1, stability_raw: 1, ..Default::default() }, CountryRow { id: 2, stability_raw: 2, ..Default::default() }];
        let input = match ordinal {
            300..=305 | 1_600 => Input::Snapshot { rows: &rows, partial: Some(1), read: true, sample: Sample::FromReadBit },
            700..=760 => Input::Unavailable { reason: "refused across a block's end", read: false, sample: Sample::FromReadBit },
            1_000..=1_010 if run == 0 => Input::Unavailable { reason: "refused", read: false, sample: Sample::FromReadBit },
            1_200..=1_202 => Input::Unavailable { reason: "refused while read", read: true, sample: Sample::FromReadBit },
            _ => Input::Snapshot { rows: &rows, partial: None, read: ordinal % 24 == 0, sample: Sample::FromReadBit },
        };
        synthetic.table(TableId::Countries, input).unwrap();
    });
    let recording = Recording::open(bytes).unwrap();
    for run in 0..recording.run_count() {
        let held = recording.run(run).unwrap();
        let (first, last) = (held.first_hour, held.last_hour());
        for (from, to) in [(first, last), (first + 333, last - 17), (first + 299, first + 306), (first + 705, first + 705), (first + 690, first + 1_020)] {
            if to > last {
                continue;
            }
            let runs = statuses(&recording, run, TableId::Countries, from, to).unwrap();
            assert_eq!(runs, stepped_statuses(&recording, run, TableId::Countries, from, to), "run {run} {from}..={to}");
            assert_eq!((runs.first().map(|run| run.first), runs.last().map(|run| run.last)), (Some(from), Some(to)), "the runs cover the span");
        }
        let whole = statuses(&recording, run, TableId::Countries, first, last).unwrap();
        assert!(
            whole.iter().any(|held| (held.first, held.last) == (first + 700, first + 760) && matches!(held.status, Status::Unavailable { .. })),
            "run {run}: an unavailable stretch across a block's end is one run: {whole:?}"
        );
        assert!(whole.iter().any(|held| matches!(held.status, Status::Truncated { .. })), "run {run}: a partial read is its own run");
        let read = reads(&recording, run, TableId::Countries, first, last).unwrap();
        let available = available_reads(&recording, run, TableId::Countries, first, last).unwrap();
        let inside = |runs: &[(i32, i32)], hour: i32| runs.iter().any(|&(a, b)| a <= hour && hour <= b);
        if last >= first + 1_202 {
            assert!(inside(&read, first + 1_201) && !inside(&available, first + 1_201), "run {run}: an hour read but refused is not an available read");
        }
        assert!(available.iter().all(|&(a, b)| (a..=b).all(|hour| inside(&read, hour))), "run {run}: available reads are reads");
    }
}

#[test]
fn log_span_reads_texts_filters_keys_and_reports_unavailable_gaps_across_blocks() {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::{LeaderTraitGainRow, UnitLeaderRow, UnitLeaderTraitRow};
    const L1: u64 = 11 << 32 | 5;
    const L2: u64 = 12 << 32 | 5;
    const RUNS: [(u64, i32, u32); 1] = [(1, 100, 2)];
    let streams = [TableId::UnitLeaders, TableId::UnitLeaderTraits];
    let (bytes, _) = fixtures::written(recorder_derive::writers(), b"log span text and gaps", &RUNS, &streams, |synthetic, _, ordinal| {
        let leaders = [
            UnitLeaderRow { id: 11, reference_type: 5, ..Default::default() },
            UnitLeaderRow { id: 12, reference_type: 5, ..Default::default() },
        ];
        synthetic.table(TableId::UnitLeaders, Input::Snapshot { rows: &leaders, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        let traits_input = if (300..=305).contains(&ordinal) {
            Input::Unavailable { reason: "traits unavailable", read: false, sample: Sample::FromReadBit }
        } else if (715..=725).contains(&ordinal) {
            Input::Unavailable { reason: "traits across block", read: false, sample: Sample::FromReadBit }
        } else {
            let mut rows = vec![
                UnitLeaderTraitRow { leader_reference: L1, ordinal: 0, token_id: 101, definition_ordinal: 0, token: b"panzer_leader".to_vec() },
                UnitLeaderTraitRow { leader_reference: L2, ordinal: 0, token_id: 102, definition_ordinal: 1, token: b"trickster".to_vec() },
            ];
            if ordinal >= 50 {
                rows.push(UnitLeaderTraitRow { leader_reference: L1, ordinal: 1, token_id: 103, definition_ordinal: 2, token: b"fortress_buster".to_vec() });
            }
            if ordinal >= 800 {
                rows.push(UnitLeaderTraitRow { leader_reference: L1, ordinal: 2, token_id: 104, definition_ordinal: 3, token: b"panzer_leader".to_vec() });
                rows.push(UnitLeaderTraitRow { leader_reference: L2, ordinal: 1, token_id: 105, definition_ordinal: 4, token: b"hill_fighter".to_vec() });
            }
            synthetic.table(TableId::UnitLeaderTraits, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            return;
        };
        synthetic.table::<UnitLeaderTraitRow>(TableId::UnitLeaderTraits, traits_input).unwrap();
    });
    let recording = Recording::open(bytes).unwrap();
    let held = recording.run(0).unwrap();
    let (first, last) = (held.first_hour, held.last_hour());
    let spelling = field(TableId::LeaderTraitGains, "spelling");
    let class = field(TableId::LeaderTraitGains, "class");
    let columns = [spelling, class];

    let mut all_rows = Vec::new();
    let all = log_span(&recording, 0, TableId::LeaderTraitGains, &columns, Keys::All, first, last, &mut |hour, key, cells| {
        all_rows.push((hour - first, (key >> 64) as u64, key as u32, cells[0], cells[1]));
    })
    .unwrap();
    assert_eq!(all.columns, columns);
    let named_all: Vec<_> = all_rows
        .iter()
        .map(|&(offset, leader, token, text, kind)| (offset, leader, token, all.text(text).map(<[u8]>::to_vec), kind))
        .collect();
    assert!(
        named_all.contains(&(0, L1, 101, Some(b"panzer_leader".to_vec()), Cell::Number(i64::from(LeaderTraitGainRow::HELD_BEFORE))))
            && named_all.contains(&(0, L2, 102, Some(b"trickster".to_vec()), Cell::Number(i64::from(LeaderTraitGainRow::HELD_BEFORE))))
            && named_all.contains(&(50, L1, 103, Some(b"fortress_buster".to_vec()), Cell::Number(i64::from(LeaderTraitGainRow::GAINED))))
            && named_all.contains(&(800, L1, 104, Some(b"panzer_leader".to_vec()), Cell::Number(i64::from(LeaderTraitGainRow::GAINED))))
            && named_all.contains(&(800, L2, 105, Some(b"hill_fighter".to_vec()), Cell::Number(i64::from(LeaderTraitGainRow::GAINED)))),
        "log_span exposes recorded text in caller column order: {named_all:?}"
    );
    assert!(
        all.gaps
            .iter()
            .any(|gap| (gap.first, gap.last) == (first + 300, first + 305) && gap.reason.as_deref() == Some("unit leader traits: not available (traits unavailable)"))
            && all
                .gaps
                .iter()
                .any(|gap| (gap.first, gap.last) == (first + 715, first + 725) && gap.reason.as_deref() == Some("unit leader traits: not available (traits across block)")),
        "log_span reports unavailable stretches with resolved reasons: {:?}",
        all.gaps
    );

    let l1_low = u128::from(L1) << 64;
    let l1_high = l1_low | ((1_u128 << 64) - 1);
    let mut l1_rows = Vec::new();
    let l1 = log_span(&recording, 0, TableId::LeaderTraitGains, &columns, Keys::Range(l1_low, l1_high), first, last, &mut |hour, key, cells| {
        l1_rows.push((hour - first, (key >> 64) as u64, key as u32, cells[0], cells[1]));
    })
    .unwrap();
    assert!(l1_rows.iter().all(|&(_, leader, ..)| leader == L1), "Keys::Range returns only the requested leader");
    let l1_texts: Vec<_> = l1_rows.iter().map(|&(offset, _, token, text, _)| (offset, token, l1.text(text).unwrap().to_vec())).collect();
    assert!(
        l1_texts.contains(&(0, 101, b"panzer_leader".to_vec()))
            && l1_texts.contains(&(50, 103, b"fortress_buster".to_vec()))
            && l1_texts.contains(&(800, 104, b"panzer_leader".to_vec())),
        "filtered log_span resolves texts cleanly across blocks: {l1_texts:?}"
    );
    assert!(
        l1.texts.iter().all(|text| &**text != b"trickster" && &**text != b"hill_fighter"),
        "filtered log_span must not leak unfiltered leaders' texts: {:?}",
        l1.texts
    );
    assert_eq!(l1.texts.iter().filter(|text| &***text == b"panzer_leader").count(), 1, "identical texts deduplicate across blocks");

    let mut block1_rows = Vec::new();
    let block1 = log_span(&recording, 0, TableId::LeaderTraitGains, &columns, Keys::Range(l1_low, l1_high), first + 720, first + 900, &mut |hour, key, cells| {
        block1_rows.push((hour - first, key as u32, cells[0]));
    })
    .unwrap();
    assert_eq!(
        block1.gaps.iter().map(|gap| (gap.first - first, gap.last - first, gap.reason.as_deref())).collect::<Vec<_>>(),
        [(720, 725, Some("unit leader traits: not available (traits across block)"))],
        "unavailable status carrying across a block boundary resolves its reason from the prior block restore"
    );
    assert!(
        block1_rows.iter().any(|&(offset, token, text)| offset == 800 && token == 104 && block1.text(text) == Some(&b"panzer_leader"[..])),
        "block 1 filtered read resolves text interned in block 0"
    );

    let mut gap_only_rows = Vec::new();
    let gap_only = log_span(&recording, 0, TableId::LeaderTraitGains, &columns, Keys::All, first + 300, first + 305, &mut |hour, key, _| {
        gap_only_rows.push((hour, key));
    })
    .unwrap();
    assert!(gap_only_rows.is_empty());
    assert_eq!(gap_only.gaps.len(), 1);
    assert_eq!(gap_only.gaps[0].reason.as_deref(), Some("unit leader traits: not available (traits unavailable)"));

    assert!(log_span(&recording, 0, TableId::LeaderTraitGains, &[99], Keys::All, first, last, &mut |_, _, _| {}).is_err());
    assert!(log_span(&recording, 0, TableId::Countries, &[0], Keys::All, first, last, &mut |_, _, _| {}).is_err());
}

#[test]
fn edge_walk_allocates_only_visited_bins_across_many_keys() {
    let mut walk = Walk::new(Some(summary::Source::Cell(0)), None, 816, 6, 1_000);
    for key in 0u128..1_500 {
        walk.point(key, true, Some((key % 17) as i64), 820);
        walk.point(key, true, Some(((key + 1) % 17) as i64), 821);
    }
    walk.end(822);
    assert_eq!(walk.rows.len(), 1_500);
    assert!(walk.rows.values().all(|row| row.bins.len() == 1 && row.bins[0].0 == 0 && row.bins[0].1.last_hour == 821));
}

#[test]
fn format_143_recording_opens_withholds_variant_tables_and_unverified_required_trucks() {
    use replay_recording::{columnar_frames::test_support::read_on, schema::{columns_at, schema}};
    use replay_columnar::{envelope, state::Kind as Column};
    use replay_frame::frame::TableView;
    use replay_data::rows::TableRows;
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::{CountryRow, Recorded, TrainInventoryRow};
    let format = replay_data::rows::compatibility::VARIANT_STAT_WITNESS_FORMAT - 1;
    let countries = [CountryRow {
        id: 7,
        tag: b"GER".to_vec(),
        stability_raw: 5,
        ..Default::default()
    }];
    let trains = [TrainInventoryRow {
        country: 7,
        node_count: 3,
        allocated_trucks: Recorded::Value(Some(12)),
        required_trucks_raw: Recorded::Value(Some(0)),
        required_trains: Recorded::Value(Some(4)),
        ..Default::default()
    }];
    let runs = [(1, 100, 1)];
    let (bytes, _) = fixtures::written_at(
        recorder_derive::writers_within,
        format,
        b"format 143",
        &runs,
        &[TableId::Countries, TableId::TrainInventory],
        |synthetic, _, _| {
            synthetic
                .table(
                    TableId::Countries,
                    Input::Snapshot {
                        rows: &countries,
                        partial: None,
                        read: true,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap();
            synthetic
                .table(
                    TableId::TrainInventory,
                    Input::Snapshot {
                        rows: &trains,
                        partial: None,
                        read: true,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap();
        },
    );
    let mut legacy_schema = schema().unwrap();
    for column in &mut legacy_schema[TableId::VariantStats.index()][..10] {
        *column = Column::I64;
    }
    let columns = columns_at(format).unwrap();
    let legacy_digest =
        envelope::schema_digest_prefix(&legacy_schema[..columns.len()], &columns).unwrap();

    let recording = Recording::open(bytes).unwrap();
    assert_eq!(recording.root_identity().schema, legacy_digest);
    assert_eq!(recording.withheld_reason(TableId::PowerSpendDays.index()), None);
    for withheld in [
        TableId::VariantStats,
        TableId::VariantChanges,
        TableId::RaidSpeedInputs,
        TableId::RocketProductionOutput,
    ] {
        let gap = withheld.last_change().table_gap(withheld, format).unwrap();
        assert_eq!(
            recording.withheld_reason(withheld.index()),
            Some(gap.as_str())
        );
    }
    let read = read_on(
        &recording,
        &[
            TableId::Countries,
            TableId::TrainInventory,
            TableId::VariantStats,
            TableId::VariantChanges,
            TableId::RaidSpeedInputs,
            TableId::RocketProductionOutput,
        ],
    );
    let first = &read[0].0;
    let TableView::Complete(TableRows::Countries(got_countries)) =
        &*first.tables[TableId::Countries.index()]
    else {
        panic!("countries missing");
    };
    assert_eq!(got_countries, &countries);
    let TableView::Complete(TableRows::TrainInventory(got_trains)) =
        &*first.tables[TableId::TrainInventory.index()]
    else {
        panic!("train inventory missing");
    };
    assert_eq!(got_trains[0].allocated_trucks, Recorded::Value(Some(12)));
    assert_eq!(got_trains[0].required_trains, Recorded::Value(Some(4)));
    assert_eq!(got_trains[0].required_trucks_raw, Recorded::NotRecorded);
    for withheld in [
        TableId::VariantStats,
        TableId::VariantChanges,
        TableId::RaidSpeedInputs,
        TableId::RocketProductionOutput,
    ] {
        assert!(matches!(
            &*first.tables[withheld.index()],
            TableView::Unavailable { reason } if reason.contains(&format!("{withheld:?}"))
        ));
    }

    let mut queried = Vec::new();
    let (queried_cols, _) = crate::points::cells(
        &recording,
        0,
        TableId::TrainInventory,
        &[3, 4],
        crate::lanes::Keys::All,
        100,
        100,
        &mut |hour, key, cells| {
            queried.push((hour, key, cells.map(|c| c.to_vec())));
            true
        },
    )
    .unwrap();
    assert_eq!(queried_cols, [3, 4]);
    assert_eq!(
        queried,
        vec![(
            100,
            7,
            Some(vec![
                crate::points::Cell::Number(12),
                crate::points::Cell::Unrecorded,
            ])
        )]
    );
}
