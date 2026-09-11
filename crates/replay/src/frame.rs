//! Per-table baselines. A failed table cannot lend its rows to a later delta.

use crate::{
    BranchId, Hour, KEYFRAME_INTERVAL,
    codec::{Decoder, invalid, put_string},
    rows::{self, DeltaRow, TableId, TableRows, TableSchema},
};
use std::{collections::BTreeSet, io};

pub const TABLE_COUNT: usize = TableId::ALL.len();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub total_ns: u64,
    pub table_ns: [u64; TABLE_COUNT],
    pub pool_bytes: u64,
    pub dropped_frames: u64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            total_ns: 0,
            table_ns: [0; TABLE_COUNT],
            pool_bytes: 0,
            dropped_frames: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableView {
    Complete(TableRows),
    Truncated { rows: TableRows, missing: u32 },
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
    Snapshot(TableRows),
    Delta {
        baseline: Hour,
        changed: Vec<DeltaRow>,
        deleted: Vec<u128>,
    },
    Unchanged {
        baseline: Hour,
    },
    // A partial walk never asserts that absent keys were deleted.
    Truncated {
        rows: TableRows,
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

impl DecodedFrame {
    pub fn complete<T: TableSchema>(&self) -> io::Result<&[T::Row]> {
        let table = T::ID;
        match &self.tables[table.index()] {
            TableView::Complete(rows) => rows.rows::<T>().ok_or_else(|| {
                invalid(format!(
                    "hour {} {table:?} contains another table's rows",
                    self.hour
                ))
            }),
            TableView::Truncated { missing, .. } => Err(invalid(format!(
                "hour {} {table:?} is truncated ({missing} missing rows)",
                self.hour
            ))),
            TableView::Unavailable { reason } => Err(invalid(format!(
                "hour {} {table:?} is unavailable: {reason}",
                self.hour
            ))),
        }
    }
}
fn validate_rows(table: TableId, rows: &TableRows) -> io::Result<()> {
    if rows.table() != table {
        return Err(invalid(
            "table rows must have matching types and unique ascending keys",
        ));
    }
    rows.validate_keys()?;
    if let TableRows::Countries(countries) = rows {
        let mut slots = BTreeSet::new();
        if countries
            .iter()
            .any(|country| !slots.insert(country.canonical_slot))
        {
            return Err(invalid("country rows have duplicate canonical slots"));
        }
    }
    Ok(())
}
fn validate_complete(table: TableId, rows: &TableRows, hour: Hour) -> io::Result<()> {
    validate_rows(table, rows)?;
    validate_singleton(table, rows, hour)
}
fn validate_singleton(table: TableId, rows: &TableRows, hour: Hour) -> io::Result<()> {
    if table.key_bits() == 0 && rows.len() != 1 {
        return Err(invalid(format!(
            "complete {table:?} table must contain exactly one row"
        )));
    }
    if let TableRows::Clock(rows) = rows
        && rows[0].hour != hour
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
                    validate_delta(table, changed, deleted)?;
                    out.push(1);
                    out.extend_from_slice(&baseline.to_le_bytes());
                    put_count(out, changed.len())?;
                    let mut previous_key = 0;
                    for value in changed {
                        rows::encode_delta(table, value, previous_key, out)?;
                        previous_key = value.key;
                    }
                    put_count(out, deleted.len())?;
                    let mut previous_key = 0;
                    for key in deleted {
                        table.encode_key(*key, previous_key, out)?;
                        previous_key = *key;
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
                    let count = decode_row_count(table, input, table.minimum_delta_bytes())?;
                    let mut changed = Vec::with_capacity(count);
                    let mut previous_key = 0;
                    for _ in 0..count {
                        let row = rows::decode_delta(table, previous_key, input)?;
                        previous_key = row.key;
                        changed.push(row);
                    }
                    let count = decode_row_count(table, input, 1)?;
                    let mut deleted = Vec::with_capacity(count);
                    let mut previous_key = 0;
                    for _ in 0..count {
                        let key = table.decode_key(previous_key, input)?;
                        previous_key = key;
                        deleted.push(key);
                    }
                    validate_delta(table, &changed, &deleted)?;
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

fn encode_rows(table: TableId, values: &TableRows, out: &mut Vec<u8>) -> io::Result<()> {
    validate_rows(table, values)?;
    put_count(out, values.len())?;
    values.encode_rows(out)
}
fn decode_rows(table: TableId, input: &mut Decoder<'_>) -> io::Result<TableRows> {
    let count = decode_row_count(table, input, table.minimum_row_bytes())?;
    let values = table.decode_rows(count, input)?;
    validate_rows(table, &values)?;
    Ok(values)
}

fn decode_row_count(
    table: TableId,
    input: &mut Decoder<'_>,
    minimum_bytes: usize,
) -> io::Result<usize> {
    let count = input.count(minimum_bytes)?;
    if count > 1 && table.key_bits() == 0 {
        return Err(invalid("singleton table cannot contain multiple rows"));
    }
    Ok(count)
}

fn validate_delta(table: TableId, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<()> {
    for key in changed
        .iter()
        .map(|row| row.key)
        .chain(deleted.iter().copied())
    {
        table.check_key(key)?;
    }
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

struct Baselines {
    branch: Option<BranchId>,
    last_hour: Option<Hour>,
    tables: [Option<(Hour, TableRows)>; TABLE_COUNT],
}

impl Default for Baselines {
    fn default() -> Self {
        Self {
            branch: None,
            last_hour: None,
            tables: std::array::from_fn(|_| None),
        }
    }
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
    pub fn encode(&mut self, source: DecodedFrame, force_keyframe: bool) -> io::Result<Frame> {
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
                        Some((baseline, previous)) if values == previous => {
                            // The current rows have passed complete validation, and
                            // baselines are immutable validated commits. Comparing
                            // typed slices avoids a keyed merge for an idle table.
                            TableUpdate::Unchanged {
                                baseline: *baseline,
                            }
                        }
                        Some((baseline, previous)) => {
                            let (changed, deleted) = values.diff(previous)?;
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
                            rows: TableRows::empty(table),
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
        let mut frame = Frame {
            branch: source.branch,
            hour: source.hour,
            keyframe,
            metrics: source.metrics,
            tables: tables.try_into().unwrap(),
        };

        // Commit only after every table has passed validation and diffing.
        // Complete source vectors become baselines; partial vectors move directly
        // into the emitted update. No changed-table or unchanged-list clone occurs.
        for ((index, view), update) in source.tables.into_iter().enumerate().zip(&mut frame.tables)
        {
            match update {
                TableUpdate::Unchanged { .. } => {
                    self.state.tables[index]
                        .as_mut()
                        .expect("unchanged table has a validated baseline")
                        .0 = source.hour;
                }
                TableUpdate::Snapshot(_) | TableUpdate::Delta { .. } => {
                    let TableView::Complete(rows) = view else {
                        unreachable!()
                    };
                    self.state.tables[index] = Some((source.hour, rows));
                }
                TableUpdate::Truncated { rows, .. } => {
                    let TableView::Truncated { rows: source, .. } = view else {
                        unreachable!()
                    };
                    *rows = source;
                    self.state.tables[index] = None;
                }
                TableUpdate::Unavailable { .. } => self.state.tables[index] = None,
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
    current: Option<DecodedFrame>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode an hour and return an owned copy for callers that retain it.
    pub fn apply(&mut self, frame: &Frame) -> io::Result<DecodedFrame> {
        self.advance(frame).cloned()
    }

    /// Advance the same decoder without copying its rows into a separate output.
    /// The borrowed hour remains available for validation before the next advance.
    pub fn advance(&mut self, frame: &Frame) -> io::Result<&DecodedFrame> {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.branch == frame.branch && current.hour >= frame.hour)
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
        let mut previous = self
            .current
            .take()
            .filter(|current| !frame.keyframe && current.branch == frame.branch)
            .map(|current| {
                current.tables.map(|table| match table {
                    // Every complete table advances its baseline to the enclosing
                    // hour, including unchanged tables after a recorded gap.
                    TableView::Complete(rows) => Some((current.hour, rows)),
                    TableView::Truncated { .. } | TableView::Unavailable { .. } => None,
                })
            })
            .unwrap_or_else(|| std::array::from_fn(|_| None));
        let tables = std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            match apply_table(
                table,
                &frame.tables[index],
                previous[index].take(),
                frame.hour,
            ) {
                Ok(view) => view,
                Err(error) => TableView::Unavailable {
                    reason: error.to_string(),
                },
            }
        });
        self.current = Some(DecodedFrame {
            branch: frame.branch,
            hour: frame.hour,
            tables,
            metrics: frame.metrics.clone(),
        });
        Ok(self.current.as_ref().unwrap())
    }
}

fn apply_table(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, TableRows)>,
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
                validate_delta(table, changed, deleted)?;
                let rows = previous_rows.apply_delta(changed, deleted)?;
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
    use crate::rows::{
        CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionRow, FactionMemberRow,
        FactionRow, ProvinceRow, RowCodec, StateRow, WarRelationRow, tables,
    };

    const FACTION_ID: u64 = 0xfedc_ba98_0000_004d;

    fn source(branch: BranchId, hour: Hour) -> DecodedFrame {
        DecodedFrame {
            branch,
            hour,
            metrics: Metrics {
                total_ns: 4_567_890,
                table_ns: std::array::from_fn(|index| (index as u64 + 1) * 100),
                pool_bytes: 123_456,
                dropped_frames: 2,
            },
            tables: {
                let initial = [
                    TableView::Complete(TableRows::Clock(vec![ClockRow {
                        hour,
                        play_time: u64::MAX,
                        play_session_started_at: 123_456,
                        seed: -789,
                        speed: 4,
                    }])),
                    TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                        id: b"campaign".to_vec(),
                    }])),
                    TableView::Complete(TableRows::Countries(vec![CountryRow {
                        id: 7,
                        canonical_slot: 1,
                        tag: b"GER".to_vec(),
                        name: b"Germany".to_vec(),
                        political_power_raw: Some(-9_007_199_254_740_993),
                        command_power_raw: 1,
                        stability_raw: 100_001,
                        war_support_raw: 99_999,
                        convoys_raw: None,
                    }])),
                    TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                        id: 2,
                        canonical_slot: 1,
                    }])),
                    TableView::Complete(TableRows::Provinces(vec![province(1, 7), province(2, 2)])),
                    TableView::Complete(TableRows::States(vec![StateRow {
                        id: 8,
                        owner: 7,
                        controller: 2,
                    }])),
                    TableView::Complete(TableRows::Divisions(vec![division(
                        0x1234_5678_0000_0035,
                        300_000,
                        100_001,
                    )])),
                    TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                        first_country: 2,
                        second_country: 7,
                        start_hour: -12,
                        end_hour: i32::MIN,
                        cancel: 0x80,
                        first_casualties: 9_007_199_254_740_993,
                        second_casualties: i64::MAX,
                        first_unknown_casualties: i64::MIN,
                        second_unknown_casualties: -1,
                        first_was_instigator: 0xfe,
                    }])),
                    TableView::Complete(TableRows::Factions(vec![faction(FACTION_ID, b"Faction")])),
                    TableView::Complete(TableRows::FactionMembers(vec![
                        faction_member(2, FACTION_ID, 1),
                        faction_member(7, FACTION_ID, 0),
                    ])),
                    TableView::Complete(TableRows::Players(Vec::new())),
                    TableView::Complete(TableRows::HumanControllers(Vec::new())),
                    TableView::Complete(TableRows::LandCombats(Vec::new())),
                    TableView::Complete(TableRows::Fleets(Vec::new())),
                    TableView::Complete(TableRows::TaskForces(Vec::new())),
                    TableView::Complete(TableRows::Ships(Vec::new())),
                    TableView::Complete(TableRows::NavalCombats(Vec::new())),
                    TableView::Complete(TableRows::NavalCombatParticipants(Vec::new())),
                    TableView::Complete(TableRows::AirWings(Vec::new())),
                    TableView::Complete(TableRows::ArmyOrders(Vec::new())),
                ];
                let mut tables = std::array::from_fn(|index| {
                    let table = TableId::ALL[index];
                    TableView::Complete(
                        rows::sample_extended_table(table, 0)
                            .unwrap_or_else(|| TableRows::empty(table)),
                    )
                });
                for (target, source) in tables.iter_mut().zip(initial) {
                    *target = source;
                }
                tables
            },
        }
    }

    fn province(id: u32, controller: u32) -> ProvinceRow {
        ProvinceRow {
            id,
            state: 8,
            owner: 7,
            controller,
            victory_points: 50,
        }
    }

    fn division(id: u64, hit_points: i64, organisation: i64) -> DivisionRow {
        DivisionRow {
            id,
            roster_owner: 7,
            logical_owner: 2,
            expeditionary_owner: 0,
            province: 1,
            template: 0xfedc_ba98_0000_002b,
            hit_points,
            organisation,
        }
    }

    fn faction(id: u64, name: &[u8]) -> FactionRow {
        FactionRow {
            id,
            name: name.to_vec(),
        }
    }

    fn faction_member(country: u32, faction: u64, ordinal: u32) -> FactionMemberRow {
        FactionMemberRow {
            country,
            faction,
            ordinal,
        }
    }

    fn change_country(frame: &mut DecodedFrame, value: i64) {
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut frame.tables[TableId::Countries.index()]
        else {
            panic!()
        };
        let country = &mut rows[0];
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
    fn padded_snapshot_counts_are_rejected_before_allocating_or_reading_rows() {
        let claimed = 16_384_u32;
        assert_eq!(TableId::Divisions.minimum_row_bytes(), 41);
        assert_eq!(TableId::Campaign.minimum_row_bytes(), 5);
        for table in TableId::ALL {
            let padding = claimed as usize * table.minimum_row_bytes() - 1;
            let mut bytes = claimed.to_le_bytes().to_vec();
            bytes.resize(bytes.len() + padding, 0);
            // A key-only bound accepts this count before reserving all rows.
            assert_eq!(Decoder::new(&bytes).count(1).unwrap(), claimed as usize);
            let mut input = Decoder::new(&bytes);
            let error = decode_rows(table, &mut input).unwrap_err();
            assert_eq!(
                error.to_string(),
                "collection length exceeds remaining payload"
            );
            assert_eq!(
                input.remaining(),
                padding,
                "read a row before rejecting {table:?}"
            );
        }
    }

    #[test]
    fn changed_row_counts_include_a_selected_value_and_singletons_never_reserve_many_rows() {
        let claimed = 16_384_u32;
        let padding = claimed as usize * 2;
        let mut bytes = claimed.to_le_bytes().to_vec();
        bytes.resize(bytes.len() + padding, 0);
        assert_eq!(Decoder::new(&bytes).count(2).unwrap(), claimed as usize);
        assert_eq!(TableId::Divisions.minimum_delta_bytes(), 3);
        assert_eq!(TableId::Campaign.minimum_delta_bytes(), 6);
        let mut input = Decoder::new(&bytes);
        let error = decode_row_count(
            TableId::Divisions,
            &mut input,
            TableId::Divisions.minimum_delta_bytes(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "collection length exceeds remaining payload"
        );
        assert_eq!(input.remaining(), padding);

        let mut bytes = 2_u32.to_le_bytes().to_vec();
        bytes.resize(68, 0);
        for table in [TableId::Clock, TableId::Campaign] {
            let mut input = Decoder::new(&bytes);
            let error = decode_rows(table, &mut input).unwrap_err();
            assert_eq!(
                error.to_string(),
                "singleton table cannot contain multiple rows"
            );
            assert_eq!(input.remaining(), 64);
        }
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
            let frame = encoder.encode(expected.clone(), false).unwrap();
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
    fn borrowed_advances_and_owned_results_keep_independent_rows() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let initial = source(1, 100);
        let frame = wire(&encoder.encode(initial.clone(), false).unwrap());
        assert_eq!(decoder.advance(&frame).unwrap(), &initial);

        let mut expected = source(1, 101);
        change_country(&mut expected, i64::MAX);
        let frame = wire(&encoder.encode(expected.clone(), false).unwrap());
        let mut owned = decoder.apply(&frame).unwrap();
        assert_eq!(owned, expected);
        for table in [TableId::Campaign, TableId::Countries, TableId::Factions] {
            let TableView::Complete(rows) = &mut owned.tables[table.index()] else {
                panic!()
            };
            match rows {
                TableRows::Campaign(rows) => rows[0].id.clear(),
                TableRows::Countries(rows) => {
                    let row = &mut rows[0];
                    row.name.clear();
                    row.tag.clear();
                    row.political_power_raw = None;
                }
                TableRows::Factions(rows) => rows[0].name.clear(),
                _ => unreachable!(),
            }
        }
        for hour in [107, 108] {
            let mut expected = source(1, hour);
            change_country(&mut expected, i64::MAX);
            let frame = wire(&encoder.encode(expected.clone(), false).unwrap());
            assert_eq!(
                frame.tables[TableId::Countries.index()],
                TableUpdate::Unchanged {
                    baseline: if hour == 107 { 101 } else { 107 }
                }
            );
            assert_eq!(decoder.advance(&frame).unwrap(), &expected);
        }
    }

    #[test]
    fn consuming_encoder_moves_vectors_and_decoder_keeps_unchanged_string_allocations() {
        let mut encoder = FrameEncoder::new();
        let first = source(1, 100);
        let input_rows = first.complete::<tables::Countries>().unwrap().as_ptr();
        let initial = encoder.encode(first, false).unwrap();
        let baseline = encoder.state.tables[TableId::Countries.index()]
            .as_ref()
            .unwrap();
        assert_eq!(
            baseline.1.rows::<tables::Countries>().unwrap().as_ptr(),
            input_rows
        );

        let mut decoder = FrameDecoder::new();
        let country = &decoder
            .advance(&initial)
            .unwrap()
            .complete::<tables::Countries>()
            .unwrap()[0];
        let name = country.name.as_ptr();
        let tag = country.tag.as_ptr();
        let mut next = source(1, 101);
        change_country(&mut next, 2);
        let input_rows = next.complete::<tables::Countries>().unwrap().as_ptr();
        let delta = encoder.encode(next, false).unwrap();
        let baseline = encoder.state.tables[TableId::Countries.index()]
            .as_ref()
            .unwrap();
        assert_eq!(
            baseline.1.rows::<tables::Countries>().unwrap().as_ptr(),
            input_rows
        );
        let country = &decoder
            .advance(&delta)
            .unwrap()
            .complete::<tables::Countries>()
            .unwrap()[0];
        assert_eq!(country.command_power_raw, 2);
        assert_eq!(country.name.as_ptr(), name);
        assert_eq!(country.tag.as_ptr(), tag);

        let mut unchanged = source(1, 102);
        change_country(&mut unchanged, 2);
        let update = encoder.encode(unchanged, false).unwrap();
        assert!(matches!(
            update.tables[TableId::Countries.index()],
            TableUpdate::Unchanged { .. }
        ));
        let baseline = encoder.state.tables[TableId::Countries.index()]
            .as_ref()
            .unwrap();
        assert_eq!(
            baseline.1.rows::<tables::Countries>().unwrap().as_ptr(),
            input_rows
        );
    }

    #[test]
    fn unavailable_and_truncated_tables_require_their_own_next_snapshot() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let first = source(1, 0);
        assert_eq!(
            decoder
                .apply(&encoder.encode(first.clone(), false).unwrap())
                .unwrap(),
            first
        );
        let mut failed = source(1, 1);
        failed.tables[TableId::Countries.index()] = TableView::Unavailable {
            reason: "political status unreadable".into(),
        };
        failed.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: TableRows::Provinces(vec![province(1, 2)]),
            missing: 42,
        };
        let frame = encoder.encode(failed.clone(), false).unwrap();
        assert!(!frame.keyframe);
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), failed);

        let recovered = source(1, 2);
        let frame = encoder.encode(recovered.clone(), false).unwrap();
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
        let frame = encoder.encode(source(1, 3).clone(), false).unwrap();
        assert_eq!(
            frame.tables[TableId::Provinces.index()],
            TableUpdate::Unchanged { baseline: 2 }
        );
    }

    #[test]
    fn state_control_deltas_and_failures_keep_separate_baselines_from_provinces() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let state_index = TableId::States.index();
        for hour in 0..=6 {
            let mut expected = source(1, hour);
            if hour > 0 {
                expected.tables[state_index] =
                    TableView::Complete(TableRows::States(vec![StateRow {
                        id: 8,
                        owner: 0,
                        controller: u32::MAX,
                    }]));
            }
            if hour == 2 {
                expected.tables[state_index] = TableView::Unavailable {
                    reason: "state table unreadable".into(),
                };
            } else if hour == 4 {
                expected.tables[state_index] = TableView::Truncated {
                    rows: TableRows::States(vec![StateRow {
                        id: 8,
                        owner: 0,
                        controller: 2,
                    }]),
                    missing: 10,
                };
            }
            let frame = encoder.encode(expected.clone(), false).unwrap();
            match hour {
                0 | 3 | 5 => assert!(matches!(
                    frame.tables[state_index],
                    TableUpdate::Snapshot(_)
                )),
                1 => {
                    let TableUpdate::Delta {
                        baseline,
                        changed,
                        deleted,
                    } = &frame.tables[state_index]
                    else {
                        panic!("state ownership change did not produce a delta")
                    };
                    assert_eq!(*baseline, 0);
                    assert_eq!(changed.len(), 1);
                    assert_eq!(changed[0].mask, 3);
                    assert!(deleted.is_empty());
                }
                6 => assert_eq!(
                    frame.tables[state_index],
                    TableUpdate::Unchanged { baseline: 5 }
                ),
                _ => {}
            }
            if hour > 0 {
                assert_eq!(
                    frame.tables[TableId::Provinces.index()],
                    TableUpdate::Unchanged { baseline: hour - 1 }
                );
            }
            assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
        }
    }

    #[test]
    fn division_handles_and_native_health_survive_insert_update_delete_and_dropped_hours() {
        // Equal instance halves with different type halves must remain distinct keys.
        let first_id = 0xfedc_ba98_0000_0035;
        let second_id = 0xfedc_ba98_0000_0036;
        let inserted_id = 0xffff_ffff_0000_0035;
        let index = TableId::Divisions.index();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut first = source(1, 10);
        first.tables[index] = TableView::Complete(TableRows::Divisions(vec![
            division(first_id, 300_000, 100_001),
            division(second_id, 300_000, 100_001),
        ]));
        let frame = encoder.encode(first.clone(), false).unwrap();
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

        let mut next = source(1, 11);
        next.tables[index] = TableView::Complete(TableRows::Divisions(vec![
            division(second_id, i64::MAX, i64::MIN),
            division(inserted_id, 9_007_199_254_740_993, -100_001),
        ]));
        let frame = encoder.encode(next.clone(), false).unwrap();
        let TableUpdate::Delta {
            baseline,
            changed,
            deleted,
        } = &frame.tables[index]
        else {
            panic!("division changes did not produce a delta")
        };
        assert_eq!(*baseline, 10);
        assert_eq!(deleted, &[u128::from(first_id)]);
        assert_eq!(changed.len(), 2);
        assert_eq!(
            (changed[0].key, changed[0].mask),
            (u128::from(second_id), 0b110_0000)
        );
        assert_eq!(
            changed[0]
                .apply(Some(division(second_id, 300_000, 100_001)))
                .unwrap(),
            division(second_id, i64::MAX, i64::MIN)
        );
        assert_eq!(
            (changed[1].key, changed[1].mask),
            (u128::from(inserted_id), TableId::Divisions.full_mask())
        );
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), next);

        let mut resting = source(1, 12);
        resting.tables[index] = next.tables[index].clone();
        let frame = encoder.encode(resting.clone(), false).unwrap();
        assert_eq!(frame.tables[index], TableUpdate::Unchanged { baseline: 11 });
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), resting);

        // The spool's gap control record explains the missing hours; surviving
        // table deltas still refer to the last successfully recorded baseline.
        let mut after_gap = source(1, 19);
        after_gap.tables[index] = TableView::Complete(TableRows::Divisions(vec![
            division(second_id, i64::MAX, i64::MIN + 1),
            division(inserted_id, 9_007_199_254_740_993, -100_001),
        ]));
        let frame = encoder.encode(after_gap.clone(), false).unwrap();
        let TableUpdate::Delta {
            baseline,
            changed,
            deleted,
        } = &frame.tables[index]
        else {
            panic!("organisation change did not produce a delta")
        };
        assert_eq!(*baseline, 12);
        assert!(deleted.is_empty());
        assert_eq!(changed.len(), 1);
        assert_eq!(
            (changed[0].key, changed[0].mask),
            (u128::from(second_id), 0b100_0000)
        );
        assert_eq!(changed[0].values, [2]); // Zigzag(+1), without rounding the native value.
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), after_gap);
    }

    #[test]
    fn division_failures_and_partial_keyframes_clear_only_their_own_baseline() {
        let index = TableId::Divisions.index();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        for hour in 0..=6 {
            let mut expected = source(1, hour);
            if matches!(hour, 0 | 4) {
                expected.tables[index] = TableView::Truncated {
                    rows: TableRows::Divisions(vec![division(
                        0x1234_5678_0000_0035,
                        300_000,
                        100_001,
                    )]),
                    missing: 17,
                };
            } else if hour == 2 {
                expected.tables[index] = TableView::Unavailable {
                    reason: "division reference unreadable".into(),
                };
            }
            let frame = encoder.encode(expected.clone(), false).unwrap();
            match hour {
                1 | 3 | 5 => assert!(matches!(frame.tables[index], TableUpdate::Snapshot(_))),
                6 => assert_eq!(frame.tables[index], TableUpdate::Unchanged { baseline: 5 }),
                _ => {}
            }
            if hour > 0 {
                assert_eq!(
                    frame.tables[TableId::Provinces.index()],
                    TableUpdate::Unchanged { baseline: hour - 1 }
                );
            }
            let actual = decoder.apply(&wire(&frame)).unwrap();
            if hour == 0 {
                unavailable(&actual.tables[index], "17 rows not read");
                expected.tables[index] = actual.tables[index].clone();
            }
            assert_eq!(actual, expected);
        }

        let expected = source(1, 7);
        let mut invalid = encoder.encode(expected.clone(), false).unwrap();
        invalid.tables[index] = TableUpdate::Unchanged { baseline: 999 };
        let actual = decoder.apply(&wire(&invalid)).unwrap();
        unavailable(&actual.tables[index], "baseline at hour 999");
        for table in TableId::ALL
            .into_iter()
            .filter(|table| *table != TableId::Divisions)
        {
            assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
        }
        let later = encoder.encode(source(1, 8).clone(), false).unwrap();
        unavailable(
            &decoder.apply(&wire(&later)).unwrap().tables[index],
            "baseline at hour 7",
        );
        let recovered = source(1, 9);
        let frame = encoder.encode(recovered.clone(), true).unwrap();
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), recovered);
    }

    #[test]
    fn war_faction_and_membership_failures_recover_independently() {
        let tables = [
            TableId::WarRelations,
            TableId::Factions,
            TableId::FactionMembers,
        ];
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        for hour in 0..=4 {
            let mut expected = source(1, hour);
            if (1..=3).contains(&hour) {
                let table = tables[hour as usize - 1];
                expected.tables[table.index()] = if table == TableId::Factions {
                    TableView::Truncated {
                        rows: TableRows::Factions(vec![faction(
                            FACTION_ID,
                            b"Incomplete name walk",
                        )]),
                        missing: 2,
                    }
                } else {
                    TableView::Unavailable {
                        reason: format!("{table:?} walk unreadable"),
                    }
                };
            }
            let frame = encoder.encode(expected.clone(), false).unwrap();
            for (offset, table) in tables.into_iter().enumerate() {
                let failure_hour = offset as i32 + 1;
                if hour == 0 || hour == failure_hour + 1 {
                    assert!(matches!(
                        frame.tables[table.index()],
                        TableUpdate::Snapshot(_)
                    ));
                } else if hour != failure_hour {
                    assert_eq!(
                        frame.tables[table.index()],
                        TableUpdate::Unchanged { baseline: hour - 1 },
                        "{table:?} lost its baseline when another table failed"
                    );
                }
            }
            assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
        }

        // A segment's first frame cannot use partial rows from any of these
        // related tables as a baseline, even when all three walks failed.
        let mut partial = source(1, 5);
        for table in tables {
            let TableView::Complete(rows) = &partial.tables[table.index()] else {
                panic!()
            };
            partial.tables[table.index()] = TableView::Truncated {
                rows: rows.clone(),
                missing: 3,
            };
        }
        let frame = encoder.encode(partial.clone(), true).unwrap();
        assert!(frame.keyframe);
        let actual = decoder.apply(&wire(&frame)).unwrap();
        for table in tables {
            unavailable(&actual.tables[table.index()], "3 rows not read");
            partial.tables[table.index()] = actual.tables[table.index()].clone();
        }
        assert_eq!(actual, partial);
        let expected = source(1, 6);
        let frame = encoder.encode(expected.clone(), false).unwrap();
        assert!(!frame.keyframe);
        for table in tables {
            assert!(matches!(
                frame.tables[table.index()],
                TableUpdate::Snapshot(_)
            ));
        }
        assert_eq!(
            frame.tables[TableId::Countries.index()],
            TableUpdate::Unchanged { baseline: 5 }
        );
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }

    #[test]
    fn faction_leader_reordering_changes_only_member_ordinals_before_insert_and_delete() {
        let factions = TableId::Factions.index();
        let members = TableId::FactionMembers.index();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let first = source(1, 0);
        let frame = encoder.encode(first.clone(), false).unwrap();
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

        let mut reordered = source(1, 1);
        reordered.tables[members] = TableView::Complete(TableRows::FactionMembers(vec![
            faction_member(2, FACTION_ID, 0),
            faction_member(7, FACTION_ID, 1),
        ]));
        let frame = encoder.encode(reordered.clone(), false).unwrap();
        assert_eq!(
            frame.tables[factions],
            TableUpdate::Unchanged { baseline: 0 }
        );
        let TableUpdate::Delta {
            baseline,
            changed,
            deleted,
        } = &frame.tables[members]
        else {
            panic!("leader reordering did not produce membership deltas")
        };
        assert_eq!(*baseline, 0);
        assert!(deleted.is_empty());
        assert_eq!(
            changed
                .iter()
                .map(|row| (row.key, row.mask))
                .collect::<Vec<_>>(),
            [(2, 2), (7, 2)]
        );
        assert_eq!(changed[0].values, [1]); // Zigzag(-1): country 2 becomes first.
        assert_eq!(changed[1].values, [2]); // Zigzag(+1): country 7 becomes second.
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), reordered);

        // Dissolve the old faction and form one with a different full-width
        // identity. An empty engine name remains a valid faction row.
        let new_faction = u64::MAX;
        let mut replaced = source(1, 2);
        replaced.tables[factions] =
            TableView::Complete(TableRows::Factions(vec![faction(new_faction, b"")]));
        replaced.tables[members] = TableView::Complete(TableRows::FactionMembers(vec![
            faction_member(2, new_faction, 1),
            faction_member(9, new_faction, 0),
        ]));
        let frame = encoder.encode(replaced.clone(), false).unwrap();
        for (table, expected_changes, expected_deleted) in [
            (
                TableId::Factions,
                vec![(u128::from(new_faction), TableId::Factions.full_mask())],
                vec![u128::from(FACTION_ID)],
            ),
            (
                TableId::FactionMembers,
                vec![(2, 3), (9, TableId::FactionMembers.full_mask())],
                vec![7],
            ),
        ] {
            let TableUpdate::Delta {
                baseline,
                changed,
                deleted,
            } = &frame.tables[table.index()]
            else {
                panic!("faction replacement did not produce a {table:?} delta")
            };
            assert_eq!(*baseline, 1);
            assert_eq!(
                changed
                    .iter()
                    .map(|row| (row.key, row.mask))
                    .collect::<Vec<_>>(),
                expected_changes
            );
            assert_eq!(deleted, &expected_deleted);
        }
        assert_eq!(
            frame.tables[TableId::WarRelations.index()],
            TableUpdate::Unchanged { baseline: 1 }
        );
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), replaced);
    }

    #[test]
    fn empty_diplomacy_tables_are_complete_baselines_for_insertions_and_deletions() {
        let tables = [
            TableId::WarRelations,
            TableId::Factions,
            TableId::FactionMembers,
        ];
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        for hour in 0..=5 {
            let mut expected = source(1, hour);
            if matches!(hour, 0 | 1 | 3 | 4) {
                for table in tables {
                    expected.tables[table.index()] = TableView::Complete(TableRows::empty(table));
                }
            }
            let frame = encoder.encode(expected.clone(), false).unwrap();
            for table in tables {
                let update = &frame.tables[table.index()];
                match hour {
                    0 => assert_eq!(update, &TableUpdate::Snapshot(TableRows::empty(table))),
                    1 | 4 => assert_eq!(update, &TableUpdate::Unchanged { baseline: hour - 1 }),
                    2 | 3 | 5 => {
                        let TableUpdate::Delta {
                            baseline,
                            changed,
                            deleted,
                        } = update
                        else {
                            panic!("{table:?} lost its complete empty baseline")
                        };
                        assert_eq!(*baseline, hour - 1);
                        let populated = source(1, hour);
                        let TableView::Complete(rows) = &populated.tables[table.index()] else {
                            panic!()
                        };
                        if hour == 3 {
                            assert!(changed.is_empty());
                            assert_eq!(
                                deleted,
                                &rows.iter().map(|row| row.key()).collect::<Vec<_>>()
                            );
                        } else {
                            assert!(deleted.is_empty());
                            assert_eq!(changed.len(), rows.len());
                            assert!(changed.iter().all(|row| row.mask == table.full_mask()));
                        }
                    }
                    _ => unreachable!(),
                }
            }
            assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
        }
    }

    #[test]
    fn key_difference_contexts_restart_for_changed_deleted_and_each_frame() {
        let table = TableId::Divisions;
        let index = table.index();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut first = source(1, 0);
        first.tables[index] = TableView::Complete(TableRows::Divisions(
            [0, 1, u64::MAX - 1, u64::MAX]
                .into_iter()
                .map(|key| division(key, 300_000, 100_001))
                .collect(),
        ));
        let frame = encoder.encode(first.clone(), false).unwrap();
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

        let mut next = source(1, 1);
        next.tables[index] = TableView::Complete(TableRows::Divisions(vec![
            division(0, 300_001, 100_001),
            division(2, 300_000, 100_001),
            division(u64::MAX, 300_000, 100_002),
        ]));
        let frame = encoder.encode(next.clone(), false).unwrap();
        let mut bytes = Vec::new();
        let sizes = frame.encode_with_sizes(&mut bytes).unwrap();
        let header_size = bytes.len() - sizes.iter().sum::<usize>();
        let table_start = header_size + sizes[..index].iter().sum::<usize>();
        let mut input = Decoder::new(&bytes[table_start..table_start + sizes[index]]);
        assert_eq!(input.u8().unwrap(), 1);
        assert_eq!(input.i32().unwrap(), 0);
        assert_eq!(input.count(1 + table.mask_width()).unwrap(), 3);
        for (previous_key, expected_key) in [(0, 0), (0, 2), (2, u128::from(u64::MAX))] {
            assert_eq!(
                rows::decode_delta(table, previous_key, &mut input)
                    .unwrap()
                    .key,
                expected_key
            );
        }
        assert_eq!(input.count(1).unwrap(), 2);
        // Deletions start again at zero, after the changed list reached u64::MAX.
        assert_eq!(input.varint().unwrap(), 1);
        assert_eq!(input.varint().unwrap(), u128::from(u64::MAX - 2));
        input.finish().unwrap();
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), next);

        let mut empty = source(1, 2);
        empty.tables[index] = TableView::Complete(TableRows::empty(table));
        let frame = encoder.encode(empty.clone(), false).unwrap();
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), empty);
        let mut inserted = source(1, 3);
        inserted.tables[index] =
            TableView::Complete(TableRows::Divisions(vec![division(0, i64::MIN, i64::MAX)]));
        let frame = encoder.encode(inserted.clone(), false).unwrap();
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), inserted);
    }

    #[test]
    fn malformed_key_differences_cannot_wrap_snapshot_or_deleted_keys() {
        let table = TableId::Divisions;
        let mut bytes = Vec::new();
        put_count(&mut bytes, 2).unwrap();
        rows::encode_row(&division(u64::MAX, 1, 2), 0, &mut bytes).unwrap();
        // This independently encoded +1 follows u64::MAX in the snapshot list.
        rows::encode_row(&division(1, 3, 4), 0, &mut bytes).unwrap();
        assert!(decode_rows(table, &mut Decoder::new(&bytes)).is_err());

        let mut encoder = FrameEncoder::new();
        encoder.encode(source(1, 0).clone(), false).unwrap();
        let frame = encoder.encode(source(1, 1).clone(), false).unwrap();
        bytes.clear();
        let sizes = frame.encode_with_sizes(&mut bytes).unwrap();
        let table_start = bytes.len() - sizes.iter().sum::<usize>()
            + sizes[..table.index()].iter().sum::<usize>();
        bytes.truncate(table_start);
        bytes.push(1); // Delta.
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        put_count(&mut bytes, 0).unwrap();
        put_count(&mut bytes, 2).unwrap();
        crate::codec::put_varint(&mut bytes, u128::from(u64::MAX));
        crate::codec::put_varint(&mut bytes, 1);
        assert!(Frame::decode(&mut Decoder::new(&bytes)).is_err());
    }

    #[test]
    fn direct_delta_application_rejects_narrow_key_overflow_and_nonzero_singleton_keys() {
        let mut encoder = FrameEncoder::new();
        let first = encoder.encode(source(1, 0).clone(), false).unwrap();
        let expected = source(1, 1);
        let next = encoder.encode(expected.clone(), false).unwrap();
        for (table, invalid_key) in [
            (TableId::Provinces, u128::from(u32::MAX) + 1),
            (TableId::Campaign, 1),
        ] {
            for delete in [false, true] {
                let mut bad = next.clone();
                let TableView::Complete(values) = &expected.tables[table.index()] else {
                    panic!()
                };
                let mut change = match values {
                    TableRows::Provinces(rows) => rows[0].delta(None),
                    TableRows::Campaign(rows) => rows[0].delta(None),
                    _ => unreachable!(),
                }
                .unwrap()
                .unwrap();
                change.key = invalid_key;
                bad.tables[table.index()] = TableUpdate::Delta {
                    baseline: 0,
                    changed: if delete { vec![] } else { vec![change] },
                    deleted: if delete { vec![invalid_key] } else { vec![] },
                };
                let reason = table.check_key(invalid_key).unwrap_err().to_string();
                assert_eq!(bad.encode(&mut Vec::new()).unwrap_err().to_string(), reason);
                let mut decoder = FrameDecoder::new();
                decoder.apply(&first).unwrap();
                let actual = decoder.apply(&bad).unwrap();
                unavailable(&actual.tables[table.index()], &reason);
                for other in TableId::ALL.into_iter().filter(|other| *other != table) {
                    assert_eq!(actual.tables[other.index()], expected.tables[other.index()]);
                }
                let later = FrameEncoder::new()
                    .encode(source(1, 2).clone(), false)
                    .unwrap();
                assert_eq!(decoder.apply(&later).unwrap(), source(1, 2));
            }
        }
    }

    #[test]
    fn truncated_keyframe_names_missing_rows_and_never_supplies_a_baseline() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut expected = source(1, 0);
        expected.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: TableRows::Provinces(vec![province(1, 7)]),
            missing: 12,
        };
        let frame = encoder.encode(expected.clone(), false).unwrap();
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
        let frame = encoder.encode(available.clone(), false).unwrap();
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
            let frame = encoder.encode(expected.clone(), false).unwrap();
            assert_eq!(frame.keyframe, keyframe, "hour {hour}");
            assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
            if keyframe {
                assert_eq!(FrameDecoder::new().apply(&frame).unwrap(), expected);
            }
        }
        let expected = source(1, 1431);
        let segment = encoder.encode(expected.clone(), true).unwrap();
        assert!(segment.keyframe);
        assert_eq!(
            FrameDecoder::new().apply(&wire(&segment)).unwrap(),
            expected
        );
        assert_eq!(decoder.apply(&segment).unwrap(), expected);
        for (branch, hour) in [(2, 1431), (3, -50)] {
            let mut expected = source(branch, hour);
            change_country(&mut expected, -55);
            let frame = encoder.encode(expected.clone(), false).unwrap();
            assert!(frame.keyframe);
            assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
        }
    }

    #[test]
    fn missing_baseline_isolates_one_table_and_a_snapshot_restores_it() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        decoder
            .apply(&encoder.encode(source(1, 0).clone(), false).unwrap())
            .unwrap();
        let mut expected = source(1, 1);
        change_country(&mut expected, -123_456_789);
        let mut frame = encoder.encode(expected.clone(), false).unwrap();
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
        let frame = encoder.encode(source(1, 2).clone(), false).unwrap();
        unavailable(
            &decoder.apply(&frame).unwrap().tables[TableId::Provinces.index()],
            "baseline at hour 1",
        );
        let expected = source(1, 3);
        let frame = encoder.encode(expected.clone(), true).unwrap();
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }

    #[test]
    fn insertion_deletion_and_field_updates_reconstruct_the_direct_rows() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        decoder
            .apply(&encoder.encode(source(1, 0).clone(), false).unwrap())
            .unwrap();
        let mut expected = source(1, 1);
        expected.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![province(2, 99), province(3, 0)]));
        let frame = encoder.encode(expected.clone(), false).unwrap();
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
        first.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
            province(2, 2),
            province(4, 4),
            province(6, 6),
        ]));
        decoder
            .apply(&encoder.encode(first.clone(), false).unwrap())
            .unwrap();
        let mut expected = source(1, 1);
        expected.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![
                province(1, 1),
                province(4, 99),
                province(5, 5),
                province(7, 7),
            ]));
        let frame = encoder.encode(expected.clone(), false).unwrap();
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
        let first = encoder.encode(source(1, 0).clone(), false).unwrap();
        let next_source = source(1, 1);
        let next = encoder.encode(next_source.clone(), false).unwrap();
        let later = encoder.encode(source(1, 2).clone(), false).unwrap();
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
                    values: vec![198, 1], // Zigzag(+99).
                }],
                vec![1],
                "requires every field",
            ),
            (
                vec![DeltaRow { mask: 16, ..change }],
                vec![],
                "unknown bits",
            ),
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
        let initial = encoder.encode(source(1, 0).clone(), false).unwrap();
        decoder.apply(&initial).unwrap();
        assert!(encoder.encode(source(1, 0).clone(), false).is_err());
        assert!(decoder.apply(&initial).is_err());
        let mut invalid_source = source(1, 1);
        change_country(&mut invalid_source, i64::MAX);
        invalid_source.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![province(2, 2), province(1, 7)]));
        assert!(encoder.encode(invalid_source.clone(), false).is_err());
        let mut invalid_last_source = source(1, 1);
        change_country(&mut invalid_last_source, i64::MIN);
        invalid_last_source.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![province(1, 7)]));
        invalid_last_source.tables[TableId::ArmyOrders.index()] =
            TableView::Complete(TableRows::ArmyOrders(vec![
                crate::rows::ArmyOrderRow::default(),
                crate::rows::ArmyOrderRow::default(),
            ]));
        assert!(encoder.encode(invalid_last_source.clone(), false).is_err());
        let expected = source(1, 1);
        let valid = encoder.encode(expected.clone(), false).unwrap();
        for table in TableId::ALL.into_iter().filter(|id| *id != TableId::Clock) {
            assert_eq!(
                valid.tables[table.index()],
                TableUpdate::Unchanged { baseline: 0 },
                "{table:?} changed its baseline before the final table was validated"
            );
        }
        let mut invalid_keyframe = valid.clone();
        invalid_keyframe.keyframe = true;
        assert!(invalid_keyframe.encode(&mut Vec::new()).is_err());
        assert!(decoder.apply(&invalid_keyframe).is_err());
        let mut invalid_last_table = FrameEncoder::new().encode(expected.clone(), false).unwrap();
        invalid_last_table.tables[TableId::ArmyOrders.index()] =
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
        let valid = FrameEncoder::new().encode(expected.clone(), false).unwrap();
        let wrong_clock = ClockRow {
            hour: 99,
            ..ClockRow::default()
        };
        for (table, rows, reason) in [
            (TableId::Clock, TableRows::Clock(vec![]), "exactly one row"),
            (
                TableId::Campaign,
                TableRows::Campaign(vec![]),
                "exactly one row",
            ),
            (
                TableId::SunkShips,
                TableRows::SunkShips(vec![]),
                "exactly one row",
            ),
            (
                TableId::WeatherLimits,
                TableRows::WeatherLimits(vec![]),
                "exactly one row",
            ),
            (
                TableId::Clock,
                TableRows::Clock(vec![wrong_clock]),
                "hour differs",
            ),
        ] {
            let mut bad_source = expected.clone();
            bad_source.tables[table.index()] = TableView::Complete(rows.clone());
            assert!(
                FrameEncoder::new()
                    .encode(bad_source.clone(), false)
                    .is_err()
            );
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
        let first = encoder.encode(source(1, 0).clone(), false).unwrap();
        let expected = source(1, 1);
        let next = encoder.encode(expected.clone(), false).unwrap();
        for (table, update, reason) in [
            (
                TableId::Clock,
                TableUpdate::Delta {
                    baseline: 0,
                    changed: vec![DeltaRow {
                        key: 0,
                        mask: 1,
                        values: vec![198, 1], // Zigzag(+99) from the hour-zero baseline.
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
        let TableView::Complete(TableRows::Countries(countries)) =
            &first_source.tables[TableId::Countries.index()]
        else {
            panic!()
        };
        let country = &countries[0];
        let mut alias = country.clone();
        alias.id = 9;
        let duplicate = TableRows::Countries(vec![countries[0].clone(), alias.clone()]);
        for view in [
            TableView::Complete(duplicate.clone()),
            TableView::Truncated {
                rows: duplicate,
                missing: 1,
            },
        ] {
            let mut bad = first_source.clone();
            bad.tables[TableId::Countries.index()] = view;
            assert!(FrameEncoder::new().encode(bad.clone(), false).is_err());
        }
        let mut encoder = FrameEncoder::new();
        let first = encoder.encode(first_source.clone(), false).unwrap();
        let expected = source(1, 1);
        let mut next = encoder.encode(expected.clone(), false).unwrap();
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
