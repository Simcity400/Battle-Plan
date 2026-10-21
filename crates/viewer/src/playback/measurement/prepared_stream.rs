//! Benchmark-only decoded-update transport and exact endpoint compaction.
//! Nothing here is a recording or prepared-map format.
use super::*;
use replay_data::{
    codec::{Decoder, put_bytes, put_string},
    rows::{self, DeltaRow, TableRows},
};
use replay_frame::frame::{Metrics, SharedView, TABLE_COUNT, TableUpdate};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, BufWriter, Write},
    path::PathBuf,
};

struct Packet {
    frame: Frame,
    // Endpoint acquisitions can precede the endpoint hour. Ordinary frame
    // timing alone cannot express that after intermediate hours are omitted.
    sampled: Option<Vec<Option<Hour>>>,
}

fn count(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&u32::try_from(n).unwrap().to_le_bytes());
}

fn encode_packet(packet: &Packet, selected: &[TableId]) -> (Vec<u8>, Vec<usize>) {
    let frame = &packet.frame;
    let mut out = Vec::new();
    out.extend_from_slice(&frame.branch.to_le_bytes());
    out.extend_from_slice(&frame.hour.to_le_bytes());
    out.push(u8::from(frame.keyframe));
    out.extend_from_slice(&frame.metrics.total_ns.to_le_bytes());
    out.extend_from_slice(&frame.metrics.pool_bytes.to_le_bytes());
    out.extend_from_slice(&frame.metrics.dropped_frames.to_le_bytes());
    // Retain all source metrics, including the omitted tables' read flags.
    for ns in frame.metrics.table_ns {
        out.extend_from_slice(&ns.to_le_bytes());
    }
    out.push(u8::from(packet.sampled.is_some()));
    let mut sizes = Vec::new();
    for (at, &table) in selected.iter().enumerate() {
        let start = out.len();
        if let Some(sampled) = &packet.sampled {
            out.push(u8::from(sampled[at].is_some()));
            if let Some(hour) = sampled[at] {
                out.extend_from_slice(&hour.to_le_bytes());
            }
        }
        match &frame.tables[table.index()] {
            TableUpdate::Snapshot(values) => {
                out.push(0);
                count(&mut out, values.len());
                values.encode_rows(&mut out).unwrap();
            }
            TableUpdate::Delta {
                baseline,
                changed,
                deleted,
            } => {
                out.push(1);
                out.extend_from_slice(&baseline.to_le_bytes());
                count(&mut out, changed.len());
                for row in changed {
                    out.extend_from_slice(&row.format.to_le_bytes());
                    out.extend_from_slice(&row.key.to_le_bytes());
                    out.extend_from_slice(&row.mask.to_le_bytes());
                    out.push(u8::try_from(row.recorded_fields).unwrap());
                    put_bytes(&mut out, &row.values).unwrap();
                }
                count(&mut out, deleted.len());
                for key in deleted {
                    out.extend_from_slice(&key.to_le_bytes());
                }
            }
            TableUpdate::Unchanged { baseline } => {
                out.push(2);
                out.extend_from_slice(&baseline.to_le_bytes());
            }
            TableUpdate::Truncated { rows, missing } => {
                out.push(3);
                out.extend_from_slice(&missing.to_le_bytes());
                count(&mut out, rows.len());
                rows.encode_rows(&mut out).unwrap();
            }
            TableUpdate::Unavailable { reason } => {
                out.push(4);
                put_string(&mut out, reason).unwrap();
            }
            TableUpdate::Incompatible {
                reason,
                source_format,
                recorded_bytes,
            } => {
                out.push(5);
                put_string(&mut out, reason).unwrap();
                out.extend_from_slice(&source_format.version().to_le_bytes());
                out.extend_from_slice(&(*recorded_bytes as u64).to_le_bytes());
            }
        }
        sizes.push(out.len() - start);
    }
    (out, sizes)
}

fn key(input: &mut Decoder<'_>) -> io::Result<u128> {
    Ok(u128::from_le_bytes(input.take(16)?.try_into().unwrap()))
}

fn decode_packet(bytes: &[u8], selected: &[TableId]) -> io::Result<Packet> {
    let mut input = Decoder::new(bytes);
    let branch = input.u64()?;
    let hour = input.i32()?;
    let keyframe = input.bool()?;
    let total_ns = input.u64()?;
    let pool_bytes = input.u64()?;
    let dropped_frames = input.u64()?;
    let mut table_ns = [0; TABLE_COUNT];
    for ns in &mut table_ns {
        *ns = input.u64()?;
    }
    let mut sampled = input.bool()?.then(Vec::new);
    let mut tables = std::array::from_fn(|_| TableUpdate::Unavailable {
        reason: String::new(),
    });
    for &table in selected {
        if let Some(samples) = &mut sampled {
            samples.push(if input.bool()? {
                Some(input.i32()?)
            } else {
                None
            });
        }
        tables[table.index()] = match input.u8()? {
            0 => {
                let n = input.u32()? as usize;
                TableUpdate::Snapshot(table.decode_rows(n, &mut input)?)
            }
            1 => {
                let baseline = input.i32()?;
                let n = input.count(31)?;
                let mut changed = Vec::with_capacity(n);
                for _ in 0..n {
                    changed.push(DeltaRow {
                        format: input.u16()?,
                        key: key(&mut input)?,
                        mask: input.u64()?,
                        recorded_fields: input.u8()? as usize,
                        values: input.bytes()?,
                    });
                }
                let n = input.count(16)?;
                let deleted = (0..n).map(|_| key(&mut input)).collect::<io::Result<_>>()?;
                TableUpdate::Delta {
                    baseline,
                    changed,
                    deleted,
                }
            }
            2 => TableUpdate::Unchanged {
                baseline: input.i32()?,
            },
            3 => {
                let missing = input.u32()?;
                let n = input.u32()? as usize;
                TableUpdate::Truncated {
                    rows: table.decode_rows(n, &mut input)?,
                    missing,
                }
            }
            4 => TableUpdate::Unavailable {
                reason: input.string()?,
            },
            5 => TableUpdate::Incompatible {
                reason: input.string()?,
                source_format: replay_data::format::FormatVersion::read(input.u16()?)?,
                recorded_bytes: input.u64()? as usize,
            },
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported benchmark update",
                ));
            }
        };
    }
    input.finish()?;
    Ok(Packet {
        frame: Frame {
            branch,
            hour,
            keyframe,
            metrics: Metrics {
                total_ns,
                table_ns,
                pool_bytes,
                dropped_frames,
            },
            tables,
        },
        sampled,
    })
}

fn difference(table: TableId, old: &TableRows, new: &TableRows, baseline: Hour) -> TableUpdate {
    let mut encoded = Vec::new();
    new.encode_changes(old, &mut encoded, &mut Vec::new())
        .unwrap();
    let mut input = Decoder::new(&encoded);
    let n = input.u32().unwrap();
    let mut previous = 0;
    let mut changed = Vec::new();
    for _ in 0..n {
        let row = rows::decode_delta(table, previous, &mut input).unwrap();
        previous = row.key;
        changed.push(row);
    }
    let n = input.u32().unwrap();
    previous = 0;
    let mut deleted = Vec::new();
    for _ in 0..n {
        previous = table.decode_key(previous, &mut input).unwrap();
        deleted.push(previous);
    }
    input.finish().unwrap();
    TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    }
}

fn endpoint(old: &DecodedFrame, new: &DecodedFrame, selected: &[TableId]) -> Packet {
    assert_eq!(old.branch, new.branch);
    let mut tables = std::array::from_fn(|_| TableUpdate::Unavailable {
        reason: String::new(),
    });
    for &table in selected {
        let a = &old.tables[table.index()];
        let b = &new.tables[table.index()];
        tables[table.index()] = match (&**a, &**b) {
            (TableView::Complete(_), TableView::Complete(_)) if SharedView::ptr_eq(a, b) => {
                TableUpdate::Unchanged { baseline: old.hour }
            }
            (TableView::Complete(a), TableView::Complete(b)) => difference(table, a, b, old.hour),
            (_, TableView::Complete(rows)) => TableUpdate::Snapshot(rows.clone()),
            (_, TableView::Truncated { rows, missing }) => TableUpdate::Truncated {
                rows: rows.clone(),
                missing: *missing,
            },
            (_, TableView::Unavailable { reason }) => TableUpdate::Unavailable {
                reason: reason.clone(),
            },
        };
    }
    Packet {
        frame: Frame {
            branch: new.branch,
            hour: new.hour,
            keyframe: false,
            tables,
            metrics: new.metrics.clone(),
        },
        sampled: Some(
            selected
                .iter()
                .map(|table| new.tables[table.index()].sampled_at())
                .collect(),
        ),
    }
}

fn exported(decoded: &DecodedFrame, packet: &Packet, selected: &[TableId]) -> DecodedFrame {
    let mut result = decoded.clone();
    if let Some(sampled) = &packet.sampled {
        for (&table, &sample) in selected.iter().zip(sampled) {
            result.tables[table.index()] =
                result.tables[table.index()].clone().with_sampled_at(sample);
        }
    }
    result
}

fn parity(a: &DecodedFrame, b: &DecodedFrame, selected: &[TableId]) {
    assert_eq!(
        (a.branch, a.hour, &a.metrics),
        (b.branch, b.hour, &b.metrics)
    );
    for &table in selected {
        assert_eq!(
            a.tables[table.index()],
            b.tables[table.index()],
            "{table:?} at {}",
            a.hour
        );
        assert_eq!(
            a.tables[table.index()].sampled_at(),
            b.tables[table.index()].sampled_at(),
            "{table:?} sampled at {}",
            a.hour
        );
    }
}

#[derive(Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Totals {
    field_writes: u64,
    row_writes: u64,
    insertions: u64,
    deletions: u64,
    reinsertions: u64,
    distinct_fields: u64,
    distinct_rows: u64,
    net_fields: u64,
    net_rows: u64,
    net_deletions: u64,
    net_insertions: u64,
    net_removals: u64,
    transient_rows: u64,
    reset_rows: u64,
    availability_transitions: u64,
    bytes: u64,
}

#[derive(Default)]
struct Life {
    initial: bool,
    present: bool,
    inserted: bool,
    deleted: bool,
}
#[derive(Default)]
struct Touched {
    masks: HashMap<u128, u64>,
    life: HashMap<u128, Life>,
    totals: Totals,
}

impl Touched {
    fn update(&mut self, update: &TableUpdate, live: &mut HashSet<u128>) {
        match update {
            TableUpdate::Delta {
                changed, deleted, ..
            } => {
                for row in changed {
                    self.totals.field_writes += u64::from(row.mask.count_ones());
                    self.totals.row_writes += 1;
                    *self.masks.entry(row.key).or_default() |= row.mask;
                    if live.insert(row.key) {
                        self.totals.insertions += 1;
                        let life = self.life.entry(row.key).or_default();
                        if life.deleted {
                            self.totals.reinsertions += 1;
                        }
                        life.present = true;
                        life.inserted = true;
                    }
                }
                for &key in deleted {
                    self.totals.deletions += 1;
                    let was = live.remove(&key);
                    let life = self.life.entry(key).or_insert_with(|| Life {
                        initial: was,
                        present: was,
                        ..Life::default()
                    });
                    life.deleted = true;
                    life.present = false;
                }
            }
            _ => unreachable!("count changes after snapshot normalization"),
        }
    }
    fn finish(&mut self, update: &TableUpdate, bytes: usize) -> Totals {
        let mut value = std::mem::take(&mut self.totals);
        value.distinct_fields = self
            .masks
            .values()
            .map(|mask| u64::from(mask.count_ones()))
            .sum();
        value.distinct_rows = self.masks.len() as u64;
        for life in self.life.values() {
            value.net_insertions += u64::from(!life.initial && life.present);
            value.net_removals += u64::from(life.initial && !life.present);
            value.transient_rows += u64::from(!life.initial && !life.present && life.inserted);
            value.reset_rows +=
                u64::from(life.initial && life.present && life.inserted && life.deleted);
        }
        if let TableUpdate::Delta {
            changed, deleted, ..
        } = update
        {
            value.net_fields = changed
                .iter()
                .map(|row| u64::from(row.mask.count_ones()))
                .sum();
            value.net_rows = changed.len() as u64;
            value.net_deletions = deleted.len() as u64;
        } else if let TableUpdate::Snapshot(rows) | TableUpdate::Truncated { rows, .. } = update {
            value.net_fields = rows.len() as u64 * rows.table().field_count() as u64;
            value.net_rows = rows.len() as u64;
        }
        value.bytes = bytes as u64;
        self.masks.clear();
        self.life.clear();
        value
    }
}

fn sum(into: &mut Totals, other: &Totals) {
    into.field_writes += other.field_writes;
    into.row_writes += other.row_writes;
    into.insertions += other.insertions;
    into.deletions += other.deletions;
    into.reinsertions += other.reinsertions;
    into.distinct_fields += other.distinct_fields;
    into.distinct_rows += other.distinct_rows;
    into.net_fields += other.net_fields;
    into.net_rows += other.net_rows;
    into.net_deletions += other.net_deletions;
    into.net_insertions += other.net_insertions;
    into.net_removals += other.net_removals;
    into.transient_rows += other.transient_rows;
    into.reset_rows += other.reset_rows;
    into.availability_transitions += other.availability_transitions;
    into.bytes += other.bytes;
}

fn write_record(file: &mut BufWriter<File>, bytes: &[u8]) {
    file.write_all(&u32::try_from(bytes.len()).unwrap().to_le_bytes())
        .unwrap();
    file.write_all(bytes).unwrap();
}

struct Window {
    k: usize,
    baseline: Option<DecodedFrame>,
    decoder: FrameDecoder,
    touched: Vec<Touched>,
    totals: Vec<Totals>,
    live: Vec<HashSet<u128>>,
    file: BufWriter<File>,
    hours: usize,
    windows: usize,
    partial: usize,
    encoded_bytes: usize,
}
impl Window {
    fn new(k: usize, selected: &[TableId], dir: &Path) -> Self {
        Self {
            k,
            baseline: None,
            decoder: FrameDecoder::for_tables(selected),
            touched: selected.iter().map(|_| Touched::default()).collect(),
            totals: selected.iter().map(|_| Totals::default()).collect(),
            live: selected.iter().map(|_| HashSet::new()).collect(),
            file: BufWriter::new(File::create(dir.join(format!("compact-{k}.updates"))).unwrap()),
            hours: 0,
            windows: 0,
            partial: 0,
            encoded_bytes: 0,
        }
    }
    fn seed(&mut self, packet: &Packet, oracle: &DecodedFrame, selected: &[TableId]) {
        let (encoded, _) = encode_packet(packet, selected);
        write_record(&mut self.file, &encoded);
        self.encoded_bytes += 4 + encoded.len();
        self.decoder.advance(&packet.frame).unwrap();
        self.baseline = Some(oracle.clone());
        for (live, table) in self.live.iter_mut().zip(selected) {
            if let TableView::Complete(rows) = &*oracle.tables[table.index()] {
                live.extend(rows.iter().map(|row| row.key()));
            }
        }
    }
    fn observe(&mut self, normalized: &[TableUpdate]) {
        for ((touched, live), update) in self.touched.iter_mut().zip(&mut self.live).zip(normalized)
        {
            touched.update(update, live);
        }
        self.hours += 1;
    }
    fn flush(&mut self, oracle: &DecodedFrame, selected: &[TableId]) {
        if self.hours == 0 {
            return;
        }
        let packet = endpoint(self.baseline.as_ref().unwrap(), oracle, selected);
        let (bytes, sizes) = encode_packet(&packet, selected);
        let loaded = decode_packet(&bytes, selected).unwrap();
        for &table in selected {
            assert_eq!(
                packet.frame.tables[table.index()],
                loaded.frame.tables[table.index()]
            );
        }
        let decoded = self.decoder.advance(&loaded.frame).unwrap();
        parity(oracle, &exported(decoded, &loaded, selected), selected);
        for (at, &table) in selected.iter().enumerate() {
            let value = self.touched[at].finish(&packet.frame.tables[table.index()], sizes[at]);
            sum(&mut self.totals[at], &value);
        }
        write_record(&mut self.file, &bytes);
        self.encoded_bytes += 4 + bytes.len();
        self.windows += 1;
        if self.hours < self.k {
            self.partial = self.hours;
        }
        self.hours = 0;
        self.baseline = Some(oracle.clone());
    }
    fn report(&mut self, selected: &[TableId], hours: usize) -> serde_json::Value {
        self.file.flush().unwrap();
        let mut overall = Totals::default();
        for value in &self.totals {
            sum(&mut overall, value);
        }
        serde_json::json!({"k":self.k, "windows":self.windows, "lastPartialHours":self.partial,
            "encodedBytesWithSeed":self.encoded_bytes, "bytesPerNativeHourWithSeed":self.encoded_bytes as f64/hours as f64,
            "overall":overall, "tables":selected.iter().zip(&self.totals).map(|(table, value)|
                serde_json::json!({"table":format!("{table:?}"),"counts":value})).collect::<Vec<_>>()})
    }
}

fn normalized(frame: &Frame, old: &DecodedFrame, selected: &[TableId]) -> Vec<TableUpdate> {
    selected
        .iter()
        .map(|&table| match &frame.tables[table.index()] {
            TableUpdate::Delta { .. } => frame.tables[table.index()].clone(),
            TableUpdate::Snapshot(new) | TableUpdate::Truncated { rows: new, .. } => {
                let empty = TableRows::empty(table);
                let previous = match &*old.tables[table.index()] {
                    TableView::Complete(rows) => rows,
                    _ => &empty,
                };
                let mut result = difference(table, previous, new, old.hour);
                if matches!(&frame.tables[table.index()], TableUpdate::Truncated { .. })
                    && let TableUpdate::Delta { deleted, .. } = &mut result
                {
                    deleted.clear();
                }
                result
            }
            _ => TableUpdate::Delta {
                baseline: old.hour,
                changed: Vec::new(),
                deleted: Vec::new(),
            },
        })
        .collect()
}

fn scratch() -> PathBuf {
    PathBuf::from(std::env::var_os("BATTLE_PLAN_MAP_STREAM_EVIDENCE").or_else(|| std::env::var_os("HOI4_MAP_STREAM_EVIDENCE")).expect("BATTLE_PLAN_MAP_STREAM_EVIDENCE"))
}

#[test]
#[ignore = "deterministic recording benchmark needs BATTLE_PLAN_MAP_BENCH_REPLAY and BATTLE_PLAN_GAME_DIR"]
fn prepared_stream_counts() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let replay = Replay::open_path(Path::new(
        &std::env::var_os("BATTLE_PLAN_MAP_BENCH_REPLAY").or_else(|| std::env::var_os("HOI4_MAP_BENCH_REPLAY")).expect("BATTLE_PLAN_MAP_BENCH_REPLAY"),
    ))
    .unwrap();
    let selected = dashboard_base::prepared_map::selected_tables();
    let dir = scratch();
    std::fs::create_dir_all(&dir).unwrap();
    let mut native = BufWriter::new(File::create(dir.join("native.updates")).unwrap());
    let mut windows: Vec<_> = [6, 24, 168]
        .into_iter()
        .map(|k| Window::new(k, selected, &dir))
        .collect();
    let mut decoder = FrameDecoder::for_tables(selected);
    let mut loaded_decoder = FrameDecoder::for_tables(selected);
    let mut hours = 0;
    let mut bytes = 0;
    let mut keyframe_bytes = 0;
    let mut keyframe_count = 0;
    let mut table_bytes = vec![0_u64; selected.len()];
    let parallel = replay::container::Parallel {
        workers: 5,
        on_worker: &|| {},
    };
    for (block_at, block) in replay.index.iter().enumerate() {
        let frames = replay
            .read_block_selected_parallel(block, selected, &parallel)
            .unwrap();
        for frame in frames {
            let changes = decoder
                .current()
                .map(|old| normalized(&frame, old, selected));
            let packet = Packet {
                frame,
                sampled: None,
            };
            let (encoded, sizes) = encode_packet(&packet, selected);
            let loaded = decode_packet(&encoded, selected).unwrap();
            for &table in selected {
                assert_eq!(
                    packet.frame.tables[table.index()],
                    loaded.frame.tables[table.index()]
                );
            }
            let oracle = decoder.advance(&packet.frame).unwrap();
            parity(
                oracle,
                loaded_decoder.advance(&loaded.frame).unwrap(),
                selected,
            );
            write_record(&mut native, &encoded);
            bytes += 4 + encoded.len();
            if packet.frame.keyframe {
                keyframe_bytes += 4 + encoded.len();
                keyframe_count += 1;
            }
            for (total, size) in table_bytes.iter_mut().zip(sizes) {
                *total += size as u64;
            }
            if let Some(changes) = changes {
                for window in &mut windows {
                    window.observe(&changes);
                    if window.hours == window.k {
                        window.flush(oracle, selected);
                    }
                }
            } else {
                for window in &mut windows {
                    window.seed(&packet, oracle, selected);
                }
            }
            hours += 1;
        }
        println!(
            "PREPARED_STREAM_COUNTS_BLOCK {} hours={hours}",
            block_at + 1
        );
    }
    for window in &mut windows {
        window.flush(decoder.current().unwrap(), selected);
    }
    native.flush().unwrap();
    let report = serde_json::json!({"hours":hours,"selectedTables":selected.len(),"nativeBytes":bytes,
        "nativeBytesPerHour":bytes as f64/hours as f64,"firstSeedIncluded":true,
        "nativeKeyframeBytes":keyframe_bytes,"nativeKeyframes":keyframe_count,
        "nativeDeltaBytesPerHour":(bytes-keyframe_bytes) as f64/(hours-keyframe_count) as f64,
        "nativeTables":selected.iter().zip(table_bytes).map(|(table,bytes)|serde_json::json!({"table":format!("{table:?}"),"bytes":bytes})).collect::<Vec<_>>(),
        "windows":windows.iter_mut().map(|window|window.report(selected,hours)).collect::<Vec<_>>()});
    std::fs::write(
        dir.join("counts.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "PREPARED_STREAM_COUNTS {}",
        serde_json::json!({"hours":hours,"selectedTables":selected.len(),"nativeBytes":bytes})
    );
}

#[test]
fn endpoint_numeric_churn_and_provenance_are_exact() {
    use replay_data::rows::{CountryLinkRow, RowCodec};
    let selected = &[TableId::CountryLinks];
    let frame = |hour, rows: Vec<CountryLinkRow>, sampled| DecodedFrame {
        branch: 1,
        hour,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|at| {
            if at == TableId::CountryLinks.index() {
                SharedView::from(TableView::Complete(TableRows::CountryLinks(rows.clone())))
                    .with_sampled_at(sampled)
            } else {
                replay_frame::frame::not_selected()
            }
        }),
    };
    let a = frame(
        10,
        vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 10,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 20,
            },
        ],
        Some(10),
    );
    let b = frame(
        34,
        vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 31,
            },
            CountryLinkRow {
                id: 3,
                canonical_slot: 7,
            },
        ],
        Some(29),
    );
    let packet = endpoint(&a, &b, selected);
    let (bytes, _) = encode_packet(&packet, selected);
    let loaded = decode_packet(&bytes, selected).unwrap();
    let mut decoder = FrameDecoder::resume_for_tables(selected, a).unwrap();
    parity(
        &b,
        &exported(decoder.advance(&loaded.frame).unwrap(), &loaded, selected),
        selected,
    );
    // The retained numeric change must be relative to the starting endpoint.
    if let TableUpdate::Delta {
        changed, deleted, ..
    } = &loaded.frame.tables[TableId::CountryLinks.index()]
    {
        assert_eq!(deleted, &[2]);
        assert_eq!(
            changed[0],
            CountryLinkRow {
                id: 1,
                canonical_slot: 31
            }
            .delta(Some(&CountryLinkRow {
                id: 1,
                canonical_slot: 10
            }))
            .unwrap()
            .unwrap()
        );
    } else {
        panic!("expected endpoint delta");
    }
    let mut touched = Touched::default();
    let mut live = HashSet::new();
    for update in [
        TableUpdate::Delta {
            baseline: 10,
            changed: vec![
                CountryLinkRow {
                    id: 4,
                    canonical_slot: 8,
                }
                .delta(None)
                .unwrap()
                .unwrap(),
            ],
            deleted: vec![],
        },
        TableUpdate::Delta {
            baseline: 11,
            changed: vec![],
            deleted: vec![4],
        },
        TableUpdate::Delta {
            baseline: 12,
            changed: vec![
                CountryLinkRow {
                    id: 4,
                    canonical_slot: 9,
                }
                .delta(None)
                .unwrap()
                .unwrap(),
            ],
            deleted: vec![],
        },
    ] {
        touched.update(&update, &mut live);
    }
    let result = touched.finish(&loaded.frame.tables[TableId::CountryLinks.index()], 0);
    assert_eq!(
        (
            result.insertions,
            result.deletions,
            result.reinsertions,
            result.distinct_fields
        ),
        (2, 1, 1, 1)
    );
}

// CPU counters are independent of replay's optional attribution scopes: these
// trials must not add a scope and stamp to every reconstructed table.
#[cfg(windows)]
fn thread_cycles() -> u64 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryThreadCycleTime(
            thread: windows_sys::Win32::Foundation::HANDLE,
            cycles: *mut u64,
        ) -> i32;
    }
    let mut value = 0;
    assert_ne!(
        unsafe {
            QueryThreadCycleTime(
                windows_sys::Win32::System::Threading::GetCurrentThread(),
                &mut value,
            )
        },
        0
    );
    value
}
#[cfg(not(windows))]
fn thread_cycles() -> u64 {
    replay_frame::cpu::stamp()
}

fn cycle_hz() -> f64 {
    let start = Instant::now();
    let cycles = replay_frame::cpu::stamp();
    while start.elapsed().as_millis() < 100 {
        std::hint::spin_loop();
    }
    replay_frame::cpu::stamp().wrapping_sub(cycles) as f64 / start.elapsed().as_secs_f64()
}

fn records(bytes: &[u8]) -> Vec<&[u8]> {
    let mut at = 0;
    let mut result = Vec::new();
    while at < bytes.len() {
        let size = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        result.push(&bytes[at..at + size]);
        at += size;
    }
    result
}

fn trial(
    bytes: &[u8],
    selected: &[TableId],
    mode: &str,
    stride: usize,
    hz: f64,
) -> serde_json::Value {
    let records = records(bytes);
    let quiet_selection: Vec<_> = selected
        .iter()
        .copied()
        .filter(|table| *table != TableId::Clock)
        .collect();
    let selected = if mode == "quiet" {
        &quiet_selection
    } else {
        selected
    };
    let mut decoder = FrameDecoder::for_tables(selected);
    let mut held = VecDeque::new();
    let mut decode_ms = Vec::new();
    let mut apply_ms = Vec::new();
    let mut export_ms = Vec::new();
    let mut retire_ms = Vec::new();
    let mut packet_drop_ms = Vec::new();
    let mut keyframe_ms = Vec::new();
    let mut samples = Vec::new();
    let mut outputs = 0;
    let mut wall_ms = 0.0;
    // Bound resident typed updates by one source-block-sized slice. Original
    // keyframe boundaries are retained; no source parse is inside this trial.
    let mut starts: Vec<_> = records
        .iter()
        .enumerate()
        .filter_map(|(i, bytes)| (bytes[12] != 0).then_some(i))
        .collect();
    assert_eq!(starts.first(), Some(&0));
    starts.push(records.len());
    for span in starts.windows(2) {
        let mut loaded = if mode == "plain" {
            Vec::new()
        } else {
            records[span[0]..span[1]]
                .iter()
                .map(|bytes| {
                    decode_packet(bytes, dashboard_base::prepared_map::selected_tables()).unwrap()
                })
                .collect::<Vec<_>>()
        };
        if mode == "quiet" {
            for packet in &mut loaded {
                if !packet.frame.keyframe {
                    for &table in selected {
                        if let TableUpdate::Delta { baseline, .. } =
                            packet.frame.tables[table.index()]
                        {
                            packet.frame.tables[table.index()] =
                                TableUpdate::Unchanged { baseline };
                        }
                    }
                }
            }
        }
        for (offset, bytes) in records[span[0]..span[1]].iter().enumerate() {
            let wall = Instant::now();
            let owned = if mode == "plain" {
                let start = thread_cycles();
                let value = decode_packet(bytes, selected).unwrap();
                decode_ms.push(thread_cycles().wrapping_sub(start) as f64 / hz * 1000.0);
                Some(value)
            } else {
                None
            };
            let packet = owned.as_ref().unwrap_or_else(|| &loaded[offset]);
            let start = thread_cycles();
            let decoded = decoder.advance(&packet.frame).unwrap();
            let elapsed = thread_cycles().wrapping_sub(start) as f64 / hz * 1000.0;
            let emit = stride != 0 && (span[0] + offset).is_multiple_of(stride);
            if emit {
                let start = thread_cycles();
                held.push_back(black_box(exported(decoded, packet, selected)));
                export_ms.push(thread_cycles().wrapping_sub(start) as f64 / hz * 1000.0);
                outputs += 1;
                if held.len() > 8 {
                    let start = thread_cycles();
                    drop(held.pop_front());
                    retire_ms.push(thread_cycles().wrapping_sub(start) as f64 / hz * 1000.0);
                }
            }
            wall_ms += wall.elapsed().as_secs_f64() * 1000.0;
            if packet.frame.keyframe {
                keyframe_ms.push(elapsed);
            } else {
                apply_ms.push(elapsed);
            }
            let mut changed = 0;
            let mut deleted = 0;
            let mut fields = 0;
            let mut snapshots = 0;
            for &table in selected {
                match &packet.frame.tables[table.index()] {
                    TableUpdate::Delta {
                        changed: rows,
                        deleted: gone,
                        ..
                    } => {
                        changed += rows.len();
                        deleted += gone.len();
                        fields += rows
                            .iter()
                            .map(|row| row.mask.count_ones() as usize)
                            .sum::<usize>();
                    }
                    TableUpdate::Snapshot(rows) | TableUpdate::Truncated { rows, .. } => {
                        snapshots += rows.len()
                    }
                    _ => {}
                }
            }
            samples.push(serde_json::json!({"hour":packet.frame.hour,"keyframe":packet.frame.keyframe,"applyCpuMs":elapsed,
                "changedRows":changed,"deletedRows":deleted,"changedFields":fields,"snapshotRows":snapshots,"export":emit}));
            if let Some(owned) = owned {
                let start = thread_cycles();
                drop(owned);
                packet_drop_ms.push(thread_cycles().wrapping_sub(start) as f64 / hz * 1000.0);
            }
        }
    }
    serde_json::json!({"mode":mode,"stride":stride,"records":records.len(),"outputs":outputs,"selectedTables":selected.len(),
        "retainedOutputs":8,"decodeCpu":timings(&decode_ms),"deltaApplyCpu":timings(&apply_ms),"keyframeApplyCpu":timings(&keyframe_ms),
        "packetDropCpu":timings(&packet_drop_ms),"exportCpu":timings(&export_ms),"retireCpu":timings(&retire_ms),"wallMs":wall_ms,"samples":samples})
}

#[test]
#[ignore = "CPU benchmark needs native/compact packets at BATTLE_PLAN_MAP_STREAM_EVIDENCE and BATTLE_PLAN_GAME_DIR"]
fn prepared_stream_apply_measurement() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let selected = dashboard_base::prepared_map::selected_tables();
    let dir = scratch();
    let hz = cycle_hz();
    let native = std::fs::read(dir.join("native.updates")).unwrap();
    let mut reports = Vec::new();
    let rounds = std::env::var("BATTLE_PLAN_MAP_STREAM_ROUNDS").or_else(|_| std::env::var("HOI4_MAP_STREAM_ROUNDS")).map_or(3, |n| n.parse::<usize>().unwrap());
    for round in 0..rounds {
        let modes = if round.is_multiple_of(2) {
            ["native", "plain"]
        } else {
            ["plain", "native"]
        };
        for stride in [0, 1, 6, 24, 168] {
            for mode in modes {
                let mut value = trial(&native, selected, mode, stride, hz);
                value["round"] = serde_json::json!(round);
                println!("PREPARED_STREAM_TIMING round={round} mode={mode} stride={stride}");
                reports.push(value);
            }
        }
        reports.push(trial(&native, selected, "quiet", 24, hz));
        for k in [6, 24, 168] {
            let compact = std::fs::read(dir.join(format!("compact-{k}.updates"))).unwrap();
            let mut value = trial(&compact, selected, "plain", 1, hz);
            value["k"] = serde_json::json!(k);
            value["round"] = serde_json::json!(round);
            reports.push(value);
        }
        std::fs::write(dir.join("timing.json"),serde_json::to_vec_pretty(&serde_json::json!({"cycleHz":hz,"debugAssertions":cfg!(debug_assertions),"reports":reports})).unwrap()).unwrap();
    }
}
