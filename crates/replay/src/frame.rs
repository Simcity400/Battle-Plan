//! Per-table baselines. A failed table cannot lend its rows to a later delta.

use crate::{
    BranchId, Hour, KEYFRAME_INTERVAL,
    codec::{Decoder, invalid, put_string},
    rows::{self, DeltaRow, Row, TableId},
};
use std::{collections::BTreeSet, io};

pub const TABLE_COUNT: usize = 5;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    pub total_ns: u64,
    pub table_ns: [u64; TABLE_COUNT],
    pub pool_bytes: u64,
    pub dropped_frames: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableView {
    Complete(Vec<Row>),
    Truncated { rows: Vec<Row>, missing: u32 },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedFrame {
    pub branch: BranchId,
    pub hour: Hour,
    pub tables: [TableView; TABLE_COUNT],
    pub metrics: Metrics,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableUpdate {
    Snapshot(Vec<Row>),
    Delta {
        baseline: Hour,
        changed: Vec<DeltaRow>,
        deleted: Vec<u32>,
    },
    Unchanged {
        baseline: Hour,
    },
    // A partial walk never asserts that absent keys were deleted.
    Truncated {
        rows: Vec<Row>,
        missing: u32,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub branch: BranchId,
    pub hour: Hour,
    pub keyframe: bool,
    pub metrics: Metrics,
    pub tables: [TableUpdate; TABLE_COUNT],
}

fn put_count(out: &mut Vec<u8>, n: usize) -> io::Result<()> {
    out.extend_from_slice(
        &u32::try_from(n)
            .map_err(|_| invalid("too many rows"))?
            .to_le_bytes(),
    );
    Ok(())
}

fn validate_rows(table: TableId, rows: &[Row]) -> io::Result<()> {
    let mut previous = None;
    let mut country_slots = BTreeSet::new();
    for row in rows {
        if row.table() != table || previous.is_some_and(|key| key >= row.key()) {
            return Err(invalid(
                "table rows must have matching types and unique ascending keys",
            ));
        }
        if let Row::Country(country) = row
            && !country_slots.insert(country.canonical_slot)
        {
            return Err(invalid("country rows have duplicate canonical slots"));
        }
        previous = Some(row.key());
    }
    Ok(())
}

fn validate_complete(table: TableId, rows: &[Row], hour: Hour) -> io::Result<()> {
    validate_rows(table, rows)?;
    validate_singleton(table, rows, hour)
}

fn validate_singleton(table: TableId, rows: &[Row], hour: Hour) -> io::Result<()> {
    if matches!(table, TableId::Clock | TableId::Campaign) && rows.len() != 1 {
        return Err(invalid(format!(
            "complete {table:?} table must contain exactly one row"
        )));
    }
    if table == TableId::Clock
        && let Row::Clock(clock) = &rows[0]
        && clock.hour != hour
    {
        return Err(invalid("clock row hour differs from frame hour"));
    }
    Ok(())
}

impl Frame {
    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.encode_with_sizes(out).map(|_| ())
    }

    pub fn table_encoded_sizes(&self) -> io::Result<[usize; TABLE_COUNT]> {
        self.encode_with_sizes(&mut Vec::new())
    }

    pub fn encode_with_sizes(&self, out: &mut Vec<u8>) -> io::Result<[usize; TABLE_COUNT]> {
        out.extend_from_slice(&self.branch.to_le_bytes());
        out.extend_from_slice(&self.hour.to_le_bytes());
        out.push(u8::from(self.keyframe));
        out.extend_from_slice(&self.metrics.total_ns.to_le_bytes());
        for ns in self.metrics.table_ns {
            out.extend_from_slice(&ns.to_le_bytes());
        }
        out.extend_from_slice(&self.metrics.pool_bytes.to_le_bytes());
        out.extend_from_slice(&self.metrics.dropped_frames.to_le_bytes());
        let mut sizes = [0; TABLE_COUNT];
        for (table, update) in TableId::ALL.into_iter().zip(&self.tables) {
            let table_start = out.len();
            if self.keyframe
                && !matches!(
                    update,
                    TableUpdate::Snapshot(_) | TableUpdate::Unavailable { .. }
                )
            {
                return Err(invalid("keyframe contains a dependent or partial table"));
            }
            match update {
                TableUpdate::Snapshot(values) => {
                    validate_complete(table, values, self.hour)?;
                    out.push(0);
                    encode_rows(table, values, out)?;
                }
                TableUpdate::Delta {
                    baseline,
                    changed,
                    deleted,
                } => {
                    validate_delta(changed, deleted)?;
                    out.push(1);
                    out.extend_from_slice(&baseline.to_le_bytes());
                    put_count(out, changed.len())?;
                    for value in changed {
                        rows::encode_delta(table, value, out)?;
                    }
                    put_count(out, deleted.len())?;
                    for key in deleted {
                        out.extend_from_slice(&key.to_le_bytes());
                    }
                }
                TableUpdate::Unchanged { baseline } => {
                    out.push(2);
                    out.extend_from_slice(&baseline.to_le_bytes());
                }
                TableUpdate::Truncated { rows, missing } => {
                    if *missing == 0 {
                        return Err(invalid("truncated table has no missing rows"));
                    }
                    out.push(3);
                    out.extend_from_slice(&missing.to_le_bytes());
                    encode_rows(table, rows, out)?;
                }
                TableUpdate::Unavailable { reason } => {
                    if reason.is_empty() {
                        return Err(invalid("unavailable table needs a reason"));
                    }
                    out.push(4);
                    put_string(out, reason)?;
                }
            }
            sizes[table.index()] = out.len() - table_start;
        }
        Ok(sizes)
    }

    pub fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let branch = input.u64()?;
        let hour = input.i32()?;
        let keyframe = input.bool()?;
        let total_ns = input.u64()?;
        let mut table_ns = [0; TABLE_COUNT];
        for ns in &mut table_ns {
            *ns = input.u64()?;
        }
        let metrics = Metrics {
            total_ns,
            table_ns,
            pool_bytes: input.u64()?,
            dropped_frames: input.u64()?,
        };
        let mut tables = Vec::with_capacity(TABLE_COUNT);
        for table in TableId::ALL {
            let update = match input.u8()? {
                0 => TableUpdate::Snapshot(decode_rows(table, input)?),
                1 => {
                    let baseline = input.i32()?;
                    let count = input.count(5)?;
                    let mut changed = Vec::with_capacity(count);
                    for _ in 0..count {
                        changed.push(rows::decode_delta(table, input)?);
                    }
                    let count = input.count(4)?;
                    let mut deleted = Vec::with_capacity(count);
                    for _ in 0..count {
                        deleted.push(input.u32()?);
                    }
                    validate_delta(&changed, &deleted)?;
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
                    if missing == 0 {
                        return Err(invalid("truncated table has no missing rows"));
                    }
                    TableUpdate::Truncated {
                        rows: decode_rows(table, input)?,
                        missing,
                    }
                }
                4 => {
                    let reason = input.string()?;
                    if reason.is_empty() {
                        return Err(invalid("unavailable table needs a reason"));
                    }
                    TableUpdate::Unavailable { reason }
                }
                _ => return Err(invalid("unknown table status")),
            };
            if keyframe
                && !matches!(
                    update,
                    TableUpdate::Snapshot(_) | TableUpdate::Unavailable { .. }
                )
            {
                return Err(invalid("keyframe contains a dependent or partial table"));
            }
            tables.push(update);
        }
        Ok(Self {
            branch,
            hour,
            keyframe,
            metrics,
            tables: tables.try_into().unwrap(),
        })
    }
}

fn encode_rows(table: TableId, values: &[Row], out: &mut Vec<u8>) -> io::Result<()> {
    validate_rows(table, values)?;
    put_count(out, values.len())?;
    for value in values {
        rows::encode_row(table, value, out)?;
    }
    Ok(())
}

fn decode_rows(table: TableId, input: &mut Decoder<'_>) -> io::Result<Vec<Row>> {
    let count = input.count(4)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(rows::decode_row(table, input)?);
    }
    validate_rows(table, &values)?;
    Ok(values)
}

fn validate_delta(changed: &[DeltaRow], deleted: &[u32]) -> io::Result<()> {
    if changed.windows(2).any(|w| w[0].key >= w[1].key) || deleted.windows(2).any(|w| w[0] >= w[1])
    {
        return Err(invalid("delta keys must be unique and ascending"));
    }
    let mut deleted = deleted.iter().peekable();
    for row in changed {
        while deleted.peek().is_some_and(|key| **key < row.key) {
            deleted.next();
        }
        if deleted.peek().is_some_and(|key| **key == row.key) {
            return Err(invalid("delta both changes and deletes a key"));
        }
    }
    Ok(())
}

#[derive(Default)]
struct Baselines {
    branch: Option<BranchId>,
    last_hour: Option<Hour>,
    tables: [Option<(Hour, Vec<Row>)>; TABLE_COUNT],
}

#[derive(Default)]
pub struct FrameEncoder {
    state: Baselines,
    keyframe_hour: Option<Hour>,
}

impl FrameEncoder {
    pub fn new() -> Self {
        Self::default()
    }
    /// Segment recovery must call with force_keyframe; it may never reuse a failed write's delta.
    pub fn encode(&mut self, source: &DecodedFrame, force_keyframe: bool) -> io::Result<Frame> {
        let new_branch = self.state.branch != Some(source.branch);
        if !new_branch && self.state.last_hour.is_some_and(|hour| hour >= source.hour) {
            return Err(invalid("frame hour did not advance on its branch"));
        }
        let keyframe = force_keyframe
            || new_branch
            || self.keyframe_hour.is_none_or(|hour| {
                i64::from(source.hour) - i64::from(hour) >= i64::from(KEYFRAME_INTERVAL)
            });
        let mut tables = Vec::with_capacity(TABLE_COUNT);
        for (table, view) in TableId::ALL.into_iter().zip(&source.tables) {
            let update = match view {
                TableView::Complete(values) => {
                    validate_complete(table, values, source.hour)?;
                    match self.state.tables[table.index()]
                        .as_ref()
                        .filter(|_| !keyframe)
                    {
                        None => TableUpdate::Snapshot(values.clone()),
                        Some((baseline, previous)) => {
                            let mut changed = Vec::new();
                            let mut deleted = Vec::new();
                            let mut old = previous.iter().peekable();
                            // Both walks are key-sorted. Merge once without allocating
                            // a tree node for every unchanged province every hour.
                            for row in values {
                                while old
                                    .peek()
                                    .is_some_and(|previous| previous.key() < row.key())
                                {
                                    deleted.push(old.next().unwrap().key());
                                }
                                let baseline_row = if old
                                    .peek()
                                    .is_some_and(|previous| previous.key() == row.key())
                                {
                                    old.next()
                                } else {
                                    None
                                };
                                if let Some(delta) = row.delta(baseline_row)? {
                                    changed.push(delta);
                                }
                            }
                            deleted.extend(old.map(Row::key));
                            if changed.is_empty() && deleted.is_empty() {
                                TableUpdate::Unchanged {
                                    baseline: *baseline,
                                }
                            } else {
                                TableUpdate::Delta {
                                    baseline: *baseline,
                                    changed,
                                    deleted,
                                }
                            }
                        }
                    }
                }
                TableView::Truncated { rows, missing } => {
                    validate_rows(table, rows)?;
                    if *missing == 0 {
                        return Err(invalid("truncated table has no missing rows"));
                    }
                    if keyframe {
                        TableUpdate::Unavailable {
                            reason: format!("truncated at keyframe: {missing} rows not read"),
                        }
                    } else {
                        TableUpdate::Truncated {
                            rows: rows.clone(),
                            missing: *missing,
                        }
                    }
                }
                TableView::Unavailable { reason } => {
                    if reason.is_empty() {
                        return Err(invalid("unavailable table needs a reason"));
                    }
                    TableUpdate::Unavailable {
                        reason: reason.clone(),
                    }
                }
            };
            tables.push(update);
        }
        let frame = Frame {
            branch: source.branch,
            hour: source.hour,
            keyframe,
            metrics: source.metrics.clone(),
            tables: tables.try_into().unwrap(),
        };
        // All fallible validation and diffing has finished. An unchanged table
        // keeps its rows and allocation; only its baseline hour advances.
        for (index, update) in frame.tables.iter().enumerate() {
            match update {
                TableUpdate::Unchanged { .. } => {
                    self.state.tables[index]
                        .as_mut()
                        .expect("unchanged table has a validated baseline")
                        .0 = source.hour;
                }
                TableUpdate::Snapshot(_) | TableUpdate::Delta { .. } => {
                    let TableView::Complete(rows) = &source.tables[index] else {
                        unreachable!("complete update came from complete source rows")
                    };
                    self.state.tables[index] = Some((source.hour, rows.clone()));
                }
                TableUpdate::Truncated { .. } | TableUpdate::Unavailable { .. } => {
                    self.state.tables[index] = None;
                }
            }
        }
        self.state.branch = Some(source.branch);
        self.state.last_hour = Some(source.hour);
        if keyframe {
            self.keyframe_hour = Some(source.hour);
        }
        Ok(frame)
    }
}

#[derive(Default)]
pub struct FrameDecoder {
    state: Baselines,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn apply(&mut self, frame: &Frame) -> io::Result<DecodedFrame> {
        if self.state.branch == Some(frame.branch)
            && self.state.last_hour.is_some_and(|hour| hour >= frame.hour)
        {
            return Err(invalid("frame hour did not advance on its branch"));
        }
        // Reject frame-wide errors before moving any baseline. From here onward,
        // a failed update clears only its table and becomes a named unavailable view.
        if frame.keyframe
            && frame.tables.iter().any(|update| {
                !matches!(
                    update,
                    TableUpdate::Snapshot(_) | TableUpdate::Unavailable { .. }
                )
            })
        {
            return Err(invalid("keyframe contains a dependent or partial table"));
        }
        if frame.keyframe || self.state.branch != Some(frame.branch) {
            self.state = Baselines::default();
        }
        let tables = std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            let view = match apply_table(
                table,
                &frame.tables[index],
                self.state.tables[index].take(),
                frame.hour,
            ) {
                Ok(view) => view,
                Err(error) => TableView::Unavailable {
                    reason: error.to_string(),
                },
            };
            match view {
                TableView::Complete(rows) => {
                    let output = TableView::Complete(rows.clone());
                    self.state.tables[index] = Some((frame.hour, rows));
                    output
                }
                view => view,
            }
        });
        self.state.branch = Some(frame.branch);
        self.state.last_hour = Some(frame.hour);
        Ok(DecodedFrame {
            branch: frame.branch,
            hour: frame.hour,
            tables,
            metrics: frame.metrics.clone(),
        })
    }
}

fn apply_table(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, Vec<Row>)>,
    hour: Hour,
) -> io::Result<TableView> {
    match update {
        TableUpdate::Snapshot(values) => {
            validate_complete(table, values, hour)?;
            Ok(TableView::Complete(values.clone()))
        }
        TableUpdate::Truncated { rows, missing } => {
            validate_rows(table, rows)?;
            if *missing == 0 {
                return Err(invalid("truncated table has no missing rows"));
            }
            Ok(TableView::Truncated {
                rows: rows.clone(),
                missing: *missing,
            })
        }
        TableUpdate::Unavailable { reason } => {
            if reason.is_empty() {
                return Err(invalid("unavailable table needs a reason"));
            }
            Ok(TableView::Unavailable {
                reason: reason.clone(),
            })
        }
        TableUpdate::Delta { baseline, .. } | TableUpdate::Unchanged { baseline } => {
            let Some((_, previous_rows)) =
                previous.filter(|(previous_hour, _)| previous_hour == baseline && *baseline < hour)
            else {
                return Err(invalid(format!(
                    "missing table baseline at hour {baseline}"
                )));
            };
            if let TableUpdate::Delta {
                changed, deleted, ..
            } = update
            {
                validate_delta(changed, deleted)?;
                let mut rows =
                    Vec::with_capacity(previous_rows.len().saturating_sub(deleted.len()));
                let mut changed = changed.iter().peekable();
                let mut deleted = deleted.iter().peekable();
                // Each ordered stream is consumed once. Existing rows move into
                // the new baseline; only changed rows decode new field values.
                for previous in previous_rows {
                    let key = previous.key();
                    if deleted.peek().is_some_and(|deleted| **deleted < key) {
                        return Err(invalid("delta deletes an absent key"));
                    }
                    while changed.peek().is_some_and(|delta| delta.key < key) {
                        rows.push(changed.next().unwrap().apply(table, None)?);
                    }
                    if deleted.peek().is_some_and(|deleted| **deleted == key) {
                        deleted.next();
                    } else if changed.peek().is_some_and(|delta| delta.key == key) {
                        rows.push(changed.next().unwrap().apply(table, Some(&previous))?);
                    } else {
                        rows.push(previous);
                    }
                }
                if deleted.next().is_some() {
                    return Err(invalid("delta deletes an absent key"));
                }
                for delta in changed {
                    rows.push(delta.apply(table, None)?);
                }
                validate_complete(table, &rows, hour)?;
                Ok(TableView::Complete(rows))
            } else {
                // This private baseline already passed full row validation. Only
                // the clock's relation to the new enclosing hour can have changed.
                validate_singleton(table, &previous_rows, hour)?;
                Ok(TableView::Complete(previous_rows))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::{CampaignRow, ClockRow, CountryLinkRow, CountryRow, ProvinceRow};

    fn source(branch: BranchId, hour: Hour) -> DecodedFrame {
        DecodedFrame {
            branch,
            hour,
            metrics: Metrics {
                total_ns: 4_567_890,
                table_ns: [100, 200, 300, 400, 500],
                pool_bytes: 123_456,
                dropped_frames: 2,
            },
            tables: [
                TableView::Complete(vec![Row::Clock(ClockRow {
                    hour,
                    play_time: u64::MAX,
                    play_session_started_at: 123_456,
                    seed: -789,
                })]),
                TableView::Complete(vec![Row::Campaign(CampaignRow {
                    id: b"campaign".to_vec(),
                })]),
                TableView::Complete(vec![Row::Country(CountryRow {
                    id: 7,
                    canonical_slot: 1,
                    tag: b"GER".to_vec(),
                    name: b"Germany".to_vec(),
                    political_power_raw: Some(-9_007_199_254_740_993),
                    command_power_raw: 1,
                    stability_raw: 100_001,
                    war_support_raw: 99_999,
                    convoys_raw: None,
                })]),
                TableView::Complete(vec![Row::CountryLink(CountryLinkRow {
                    id: 2,
                    canonical_slot: 1,
                })]),
                TableView::Complete(vec![province(1, 7), province(2, 2)]),
            ],
        }
    }

    fn province(id: u32, controller: u32) -> Row {
        Row::Province(ProvinceRow {
            id,
            state: 8,
            owner: 7,
            controller,
        })
    }

    fn change_country(frame: &mut DecodedFrame, value: i64) {
        let TableView::Complete(rows) = &mut frame.tables[TableId::Countries.index()] else {
            panic!()
        };
        let Row::Country(country) = &mut rows[0] else {
            panic!()
        };
        country.command_power_raw = value;
    }

    fn wire(frame: &Frame) -> Frame {
        let mut bytes = Vec::new();
        frame.encode(&mut bytes).unwrap();
        let mut decoder = Decoder::new(&bytes);
        let result = Frame::decode(&mut decoder).unwrap();
        decoder.finish().unwrap();
        assert_eq!(&result, frame);
        result
    }

    fn unavailable(view: &TableView, expected: &str) {
        let TableView::Unavailable { reason } = view else {
            panic!("expected unavailable: {view:?}")
        };
        assert!(
            reason.contains(expected),
            "{reason:?} did not contain {expected:?}"
        );
    }

    #[test]
    fn unchanged_advances_each_baseline_and_missing_hours_do_not_clear_it() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        for hour in [100, 101, 107, 108] {
            let mut expected = source(1, hour);
            if hour >= 107 {
                change_country(&mut expected, i64::MAX);
            }
            let frame = encoder.encode(&expected, false).unwrap();
            match hour {
                100 => assert!(
                    frame
                        .tables
                        .iter()
                        .all(|t| matches!(t, TableUpdate::Snapshot(_)))
                ),
                101 => assert_eq!(
                    frame.tables[TableId::Countries.index()],
                    TableUpdate::Unchanged { baseline: 100 }
                ),
                107 => {
                    assert!(matches!(
                        frame.tables[TableId::Countries.index()],
                        TableUpdate::Delta { baseline: 101, .. }
                    ));
                    assert_eq!(
                        frame.tables[TableId::Provinces.index()],
                        TableUpdate::Unchanged { baseline: 101 }
                    );
                }
                108 => assert_eq!(
                    frame.tables[TableId::Countries.index()],
                    TableUpdate::Unchanged { baseline: 107 }
                ),
                _ => unreachable!(),
            }
            assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
        }
    }

    #[test]
    fn unavailable_and_truncated_tables_require_their_own_next_snapshot() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let first = source(1, 0);
        assert_eq!(
            decoder
                .apply(&encoder.encode(&first, false).unwrap())
                .unwrap(),
            first
        );
        let mut failed = source(1, 1);
        failed.tables[TableId::Countries.index()] = TableView::Unavailable {
            reason: "political status unreadable".into(),
        };
        failed.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: vec![province(1, 2)],
            missing: 42,
        };
        let frame = encoder.encode(&failed, false).unwrap();
        assert!(!frame.keyframe);
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), failed);

        let recovered = source(1, 2);
        let frame = encoder.encode(&recovered, false).unwrap();
        for table in [TableId::Countries, TableId::Provinces] {
            assert!(matches!(
                frame.tables[table.index()],
                TableUpdate::Snapshot(_)
            ));
        }
        assert!(matches!(
            frame.tables[TableId::Clock.index()],
            TableUpdate::Delta { baseline: 1, .. }
        ));
        assert_eq!(
            frame.tables[TableId::Campaign.index()],
            TableUpdate::Unchanged { baseline: 1 }
        );
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), recovered);
        let frame = encoder.encode(&source(1, 3), false).unwrap();
        assert_eq!(
            frame.tables[TableId::Provinces.index()],
            TableUpdate::Unchanged { baseline: 2 }
        );
    }

    #[test]
    fn truncated_keyframe_names_missing_rows_and_never_supplies_a_baseline() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut expected = source(1, 0);
        expected.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: vec![province(1, 7)],
            missing: 12,
        };
        let frame = encoder.encode(&expected, false).unwrap();
        assert!(frame.keyframe);
        assert!(frame.tables.iter().all(|t| matches!(
            t,
            TableUpdate::Snapshot(_) | TableUpdate::Unavailable { .. }
        )));
        let actual = decoder.apply(&wire(&frame)).unwrap();
        unavailable(
            &actual.tables[TableId::Provinces.index()],
            "12 rows not read",
        );
        for table in TableId::ALL
            .into_iter()
            .filter(|t| *t != TableId::Provinces)
        {
            assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
        }
        let available = source(1, 1);
        let frame = encoder.encode(&available, false).unwrap();
        assert!(matches!(
            frame.tables[TableId::Provinces.index()],
            TableUpdate::Snapshot(_)
        ));
        assert_eq!(decoder.apply(&frame).unwrap(), available);
    }

    #[test]
    fn keyframes_follow_elapsed_hours_and_reset_on_segments_and_branches() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        for (hour, keyframe) in [
            (-10, true),
            (709, false),
            (710, true),
            (1429, false),
            (1430, true),
        ] {
            let expected = source(1, hour);
            let frame = encoder.encode(&expected, false).unwrap();
            assert_eq!(frame.keyframe, keyframe, "hour {hour}");
            assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
            if keyframe {
                assert_eq!(FrameDecoder::new().apply(&frame).unwrap(), expected);
            }
        }
        let expected = source(1, 1431);
        let segment = encoder.encode(&expected, true).unwrap();
        assert!(segment.keyframe);
        assert_eq!(
            FrameDecoder::new().apply(&wire(&segment)).unwrap(),
            expected
        );
        assert_eq!(decoder.apply(&segment).unwrap(), expected);
        for (branch, hour) in [(2, 1431), (3, -50)] {
            let mut expected = source(branch, hour);
            change_country(&mut expected, -55);
            let frame = encoder.encode(&expected, false).unwrap();
            assert!(frame.keyframe);
            assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
        }
    }

    #[test]
    fn missing_baseline_isolates_one_table_and_a_snapshot_restores_it() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        decoder
            .apply(&encoder.encode(&source(1, 0), false).unwrap())
            .unwrap();
        let mut expected = source(1, 1);
        change_country(&mut expected, -123_456_789);
        let mut frame = encoder.encode(&expected, false).unwrap();
        frame.tables[TableId::Provinces.index()] = TableUpdate::Unchanged { baseline: 999 };
        let actual = decoder.apply(&wire(&frame)).unwrap();
        unavailable(
            &actual.tables[TableId::Provinces.index()],
            "baseline at hour 999",
        );
        for table in TableId::ALL
            .into_iter()
            .filter(|t| *t != TableId::Provinces)
        {
            assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
        }
        let frame = encoder.encode(&source(1, 2), false).unwrap();
        unavailable(
            &decoder.apply(&frame).unwrap().tables[TableId::Provinces.index()],
            "baseline at hour 1",
        );
        let expected = source(1, 3);
        let frame = encoder.encode(&expected, true).unwrap();
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }

    #[test]
    fn insertion_deletion_and_field_updates_reconstruct_the_direct_rows() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        decoder
            .apply(&encoder.encode(&source(1, 0), false).unwrap())
            .unwrap();
        let mut expected = source(1, 1);
        expected.tables[TableId::Provinces.index()] =
            TableView::Complete(vec![province(2, 99), province(3, 0)]);
        let frame = encoder.encode(&expected, false).unwrap();
        let TableUpdate::Delta {
            baseline,
            changed,
            deleted,
        } = &frame.tables[TableId::Provinces.index()]
        else {
            panic!()
        };
        assert_eq!(*baseline, 0);
        assert_eq!(deleted, &[1]);
        assert_eq!(
            changed.iter().map(|r| (r.key, r.mask)).collect::<Vec<_>>(),
            [(2, 4), (3, TableId::Provinces.full_mask())]
        );
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }

    #[test]
    fn sorted_delta_merge_handles_insertions_around_deleted_and_retained_rows() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut first = source(1, 0);
        first.tables[TableId::Provinces.index()] =
            TableView::Complete(vec![province(2, 2), province(4, 4), province(6, 6)]);
        decoder
            .apply(&encoder.encode(&first, false).unwrap())
            .unwrap();
        let mut expected = source(1, 1);
        expected.tables[TableId::Provinces.index()] = TableView::Complete(vec![
            province(1, 1),
            province(4, 99),
            province(5, 5),
            province(7, 7),
        ]);
        let frame = encoder.encode(&expected, false).unwrap();
        let TableUpdate::Delta {
            changed, deleted, ..
        } = &frame.tables[TableId::Provinces.index()]
        else {
            panic!()
        };
        assert_eq!(
            changed.iter().map(|row| row.key).collect::<Vec<_>>(),
            [1, 4, 5, 7]
        );
        assert_eq!(deleted, &[2, 6]);
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }

    #[test]
    fn invalid_delta_operations_do_not_damage_other_tables_or_retain_partial_rows() {
        let mut encoder = FrameEncoder::new();
        let first = encoder.encode(&source(1, 0), false).unwrap();
        let next_source = source(1, 1);
        let next = encoder.encode(&next_source, false).unwrap();
        let later = encoder.encode(&source(1, 2), false).unwrap();
        let change = province(2, 99)
            .delta(Some(&province(2, 2)))
            .unwrap()
            .unwrap();
        let cases = [
            (vec![change.clone()], vec![2], "changes and deletes"),
            (
                vec![change.clone(), change.clone()],
                vec![],
                "unique and ascending",
            ),
            (vec![], vec![1, 1], "unique and ascending"),
            (vec![], vec![1, 999], "absent key"),
            (
                vec![DeltaRow {
                    key: 3,
                    mask: 4,
                    values: 99_u32.to_le_bytes().to_vec(),
                }],
                vec![1],
                "requires every field",
            ),
            (vec![DeltaRow { mask: 8, ..change }], vec![], "unknown bits"),
        ];
        for (changed, deleted, reason) in cases {
            let mut decoder = FrameDecoder::new();
            decoder.apply(&first).unwrap();
            let mut bad = next.clone();
            bad.tables[TableId::Provinces.index()] = TableUpdate::Delta {
                baseline: 0,
                changed,
                deleted,
            };
            let actual = decoder.apply(&bad).unwrap();
            unavailable(&actual.tables[TableId::Provinces.index()], reason);
            for table in TableId::ALL
                .into_iter()
                .filter(|t| *t != TableId::Provinces)
            {
                assert_eq!(
                    actual.tables[table.index()],
                    next_source.tables[table.index()]
                );
            }
            let actual = decoder.apply(&later).unwrap();
            unavailable(
                &actual.tables[TableId::Provinces.index()],
                "baseline at hour 1",
            );
        }
        let mut empty_reason = next;
        empty_reason.tables[TableId::Provinces.index()] = TableUpdate::Unavailable {
            reason: String::new(),
        };
        let mut decoder = FrameDecoder::new();
        decoder.apply(&first).unwrap();
        unavailable(
            &decoder.apply(&empty_reason).unwrap().tables[TableId::Provinces.index()],
            "needs a reason",
        );
    }

    #[test]
    fn invalid_input_and_keyframes_are_rejected_without_advancing_state() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let initial = encoder.encode(&source(1, 0), false).unwrap();
        decoder.apply(&initial).unwrap();
        assert!(encoder.encode(&source(1, 0), false).is_err());
        assert!(decoder.apply(&initial).is_err());
        let mut invalid_source = source(1, 1);
        change_country(&mut invalid_source, i64::MAX);
        invalid_source.tables[TableId::Provinces.index()] =
            TableView::Complete(vec![province(2, 2), province(1, 7)]);
        assert!(encoder.encode(&invalid_source, false).is_err());
        let expected = source(1, 1);
        let valid = encoder.encode(&expected, false).unwrap();
        assert_eq!(
            valid.tables[TableId::Provinces.index()],
            TableUpdate::Unchanged { baseline: 0 }
        );
        let mut invalid_keyframe = valid.clone();
        invalid_keyframe.keyframe = true;
        assert!(invalid_keyframe.encode(&mut Vec::new()).is_err());
        assert!(decoder.apply(&invalid_keyframe).is_err());
        let mut invalid_last_table = FrameEncoder::new().encode(&expected, false).unwrap();
        invalid_last_table.tables[TableId::Provinces.index()] =
            TableUpdate::Unchanged { baseline: 0 };
        assert!(decoder.apply(&invalid_last_table).is_err());
        let mut bytes = Vec::new();
        valid.encode(&mut bytes).unwrap();
        bytes[12] = 1; // The keyframe byte follows the u64 branch and i32 hour.
        assert!(Frame::decode(&mut Decoder::new(&bytes)).is_err());
        assert_eq!(decoder.apply(&wire(&valid)).unwrap(), expected);
    }

    #[test]
    fn complete_singletons_and_clock_hours_are_validated_without_losing_other_tables() {
        let expected = source(1, 0);
        let valid = FrameEncoder::new().encode(&expected, false).unwrap();
        let wrong_clock = Row::Clock(ClockRow {
            hour: 99,
            ..ClockRow::default()
        });
        for (table, rows, reason) in [
            (TableId::Clock, vec![], "exactly one row"),
            (TableId::Campaign, vec![], "exactly one row"),
            (TableId::Clock, vec![wrong_clock], "hour differs"),
        ] {
            let mut bad_source = expected.clone();
            bad_source.tables[table.index()] = TableView::Complete(rows.clone());
            assert!(FrameEncoder::new().encode(&bad_source, false).is_err());
            let mut bad_frame = valid.clone();
            bad_frame.tables[table.index()] = TableUpdate::Snapshot(rows);
            assert!(bad_frame.encode(&mut Vec::new()).is_err());
            let actual = FrameDecoder::new().apply(&bad_frame).unwrap();
            unavailable(&actual.tables[table.index()], reason);
            for other in TableId::ALL.into_iter().filter(|other| *other != table) {
                assert_eq!(actual.tables[other.index()], expected.tables[other.index()]);
            }
        }
    }

    #[test]
    fn singleton_invariants_apply_after_deltas_and_unchanged_updates() {
        let mut encoder = FrameEncoder::new();
        let first = encoder.encode(&source(1, 0), false).unwrap();
        let expected = source(1, 1);
        let next = encoder.encode(&expected, false).unwrap();
        for (table, update, reason) in [
            (
                TableId::Clock,
                TableUpdate::Delta {
                    baseline: 0,
                    changed: vec![DeltaRow {
                        key: 0,
                        mask: 1,
                        values: 99_i32.to_le_bytes().to_vec(),
                    }],
                    deleted: vec![],
                },
                "hour differs",
            ),
            (
                TableId::Clock,
                TableUpdate::Unchanged { baseline: 0 },
                "hour differs",
            ),
            (
                TableId::Campaign,
                TableUpdate::Delta {
                    baseline: 0,
                    changed: vec![],
                    deleted: vec![0],
                },
                "exactly one row",
            ),
        ] {
            let mut decoder = FrameDecoder::new();
            decoder.apply(&first).unwrap();
            let mut bad = next.clone();
            bad.tables[table.index()] = update;
            let actual = decoder.apply(&wire(&bad)).unwrap();
            unavailable(&actual.tables[table.index()], reason);
            for other in TableId::ALL.into_iter().filter(|other| *other != table) {
                assert_eq!(actual.tables[other.index()], expected.tables[other.index()]);
            }
        }
    }

    #[test]
    fn duplicate_canonical_country_slots_are_ambiguous_in_snapshots_and_deltas() {
        let first_source = source(1, 0);
        let TableView::Complete(countries) = &first_source.tables[TableId::Countries.index()]
        else {
            panic!()
        };
        let Row::Country(country) = &countries[0] else {
            panic!()
        };
        let mut alias = country.clone();
        alias.id = 9;
        let alias = Row::Country(alias);
        let duplicate = vec![countries[0].clone(), alias.clone()];
        for view in [
            TableView::Complete(duplicate.clone()),
            TableView::Truncated {
                rows: duplicate,
                missing: 1,
            },
        ] {
            let mut bad = first_source.clone();
            bad.tables[TableId::Countries.index()] = view;
            assert!(FrameEncoder::new().encode(&bad, false).is_err());
        }
        let mut encoder = FrameEncoder::new();
        let first = encoder.encode(&first_source, false).unwrap();
        let expected = source(1, 1);
        let mut next = encoder.encode(&expected, false).unwrap();
        next.tables[TableId::Countries.index()] = TableUpdate::Delta {
            baseline: 0,
            changed: vec![alias.delta(None).unwrap().unwrap()],
            deleted: vec![],
        };
        let mut decoder = FrameDecoder::new();
        decoder.apply(&first).unwrap();
        let actual = decoder.apply(&wire(&next)).unwrap();
        unavailable(
            &actual.tables[TableId::Countries.index()],
            "duplicate canonical slots",
        );
        assert_eq!(
            actual.tables[TableId::CountryLinks.index()],
            expected.tables[TableId::CountryLinks.index()]
        );
        assert_eq!(
            actual.tables[TableId::Provinces.index()],
            expected.tables[TableId::Provinces.index()]
        );
    }
}
