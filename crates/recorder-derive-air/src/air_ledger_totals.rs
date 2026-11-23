
use recorder_derive_framework::{
    deployed::invalid,
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
    trade_flows::Holders,
};
use replay_columnar::{
    leaf::Writer as Bytes,
    manifest::Status,
    store::{TableView, Value as Held},
};
use replay_data::rows::{AirLossEventRow, AirLossTotalRow, BombingTotalRow, BuildingBombingDamageCause, BuildingBombingDamageRow, TableId};
use std::{collections::BTreeMap, io};

#[cfg(test)]
mod tests;

const AIR: [usize; 7] = columns::<AirLossEventRow, 7>(["game_hour", "country", "region", "native_cause", "credited_enemy", "amount_raw", "opponent"]);
const BOMBING: [usize; 5] = columns::<BuildingBombingDamageRow, 5>(["game_hour", "attacker", "victim", "cause", "amount_raw"]);

const AIR_READS: &[Read] = &[
    Read { table: TableId::AirLossEvents, columns: Columns::Every, watch: Watch::Appended, required: false },
    Read { table: TableId::CountryLinks, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::TagHolders, columns: Columns::Every, watch: Watch::Changed, required: false },
];

const BOMBING_READS: &[Read] = &[
    Read { table: TableId::BuildingBombingDamage, columns: Columns::Every, watch: Watch::Appended, required: false },
    Read { table: TableId::CountryLinks, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::TagHolders, columns: Columns::Every, watch: Watch::Changed, required: false },
];

fn available(status: Status) -> bool {
    matches!(status, Status::Complete | Status::Truncated { .. })
}

struct Added<const N: usize> {
    rows: BTreeMap<u128, ([i128; N], i32)>,
}

impl<const N: usize> Added<N> {
    fn new() -> Self {
        Self { rows: BTreeMap::new() }
    }

    fn add(&mut self, key: u128, column: usize, amount: i64, late: i32) {
        let (sums, most) = self.rows.entry(key).or_insert(([0; N], 0));
        sums[column] += i128::from(amount);
        *most = (*most).max(late);
    }

    fn land(self, table: TableId, out: &mut Rows<'_>) {
        for (key, (sums, late)) in self.rows {
            let (totals, held_late) = {
                let held = out.table(table);
                match held.slot(key) {
                    Some(slot) => (std::array::from_fn::<_, N, _>(|column| held.number(column, slot)), held.number(N, slot).unwrap_or(0) as i32),
                    None => ([Some(0); N], 0),
                }
            };
            let mut cells: Vec<Value> = totals
                .iter()
                .zip(sums)
                .map(|(total, sum)| total.and_then(|total| i64::try_from(i128::from(total) + sum).ok()).map_or(Value::Unrecorded, Value::I64))
                .collect();
            cells.push(Value::I32(held_late.max(late)));
            out.set(table, key, cells);
        }
    }
}

fn cell(view: &TableView<'_>, column: usize, slot: u32, table: TableId) -> io::Result<i64> {
    view.number(column, slot).ok_or_else(|| invalid(format!("{table:?} holds an event whose cells are not recorded")))
}

fn optional(view: &TableView<'_>, column: usize, slot: u32, table: TableId) -> io::Result<Option<u32>> {
    match view.value(column, slot) {
        Held::Missing => Ok(None),
        Held::Number(bits) => Ok(Some(bits as u32)),
        _ => Err(invalid(format!("{table:?} holds an event whose cells are not recorded"))),
    }
}

fn late(hour: i32, game_hour: i64) -> i32 {
    i64::from(hour).saturating_sub(game_hour).clamp(0, i64::from(i32::MAX)) as i32
}

fn holders<'h>(held: &'h mut Option<Holders>, hour: &Hour<'_>) -> &'h Holders {
    if hour.fresh || Holders::changed(hour) {
        *held = None;
    }
    held.get_or_insert_with(|| Holders::read(hour))
}

fn counted<'a>(hour: &Hour<'a>, log: TableId) -> Option<(TableView<'a>, Vec<u32>)> {
    let rows = hour.appended(log);
    if rows.is_empty() || !available(hour.status(log)) || !available(hour.status(TableId::CountryLinks)) {
        return None;
    }
    Some((rows.cells()?, rows.rows().map(|row| row.slot).collect()))
}

#[derive(Default)]
pub struct AirLossTotals {
    holders: Option<Holders>,
}

impl Writer for AirLossTotals {
    fn name(&self) -> &'static str {
        "AirLossTotals"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::AirLossTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        AIR_READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        const TABLE: TableId = TableId::AirLossEvents;
        out.status(TableId::AirLossTotals, Status::Complete);
        let holders = holders(&mut self.holders, hour);
        let Some((view, slots)) = counted(hour, TABLE) else { return Ok(()) };
        let mut added = Added::<4>::new();
        for slot in slots {
            let game_hour = cell(&view, AIR[0], slot, TABLE)?;
            let country = cell(&view, AIR[1], slot, TABLE)? as u32;
            let region = cell(&view, AIR[2], slot, TABLE)? as u32;
            let cause = cell(&view, AIR[3], slot, TABLE)? as u32;
            let dealt = cell(&view, AIR[4], slot, TABLE)? != 0;
            let amount = cell(&view, AIR[5], slot, TABLE)?;
            let known = optional(&view, AIR[6], slot, TABLE)?.and_then(|raw| holders.of(raw)).is_some();
            let (holder_slot, id) = holders.of(country).unwrap_or((AirLossTotalRow::NONE, AirLossTotalRow::NONE));
            let column = usize::from(!dealt) * 2 + usize::from(!known);
            added.add(AirLossTotalRow::key_of(holder_slot, id, region, cause), column, amount, late(hour.hour, game_hour));
        }
        added.land(TableId::AirLossTotals, out);
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        Ok(())
    }
}

#[derive(Default)]
pub struct BombingTotals {
    holders: Option<Holders>,
}

impl Writer for BombingTotals {
    fn name(&self) -> &'static str {
        "BombingTotals"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::BombingTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        BOMBING_READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        const TABLE: TableId = TableId::BuildingBombingDamage;
        out.status(TableId::BombingTotals, Status::Complete);
        let holders = holders(&mut self.holders, hour);
        let Some((view, slots)) = counted(hour, TABLE) else { return Ok(()) };
        let mut added = Added::<2>::new();
        for slot in slots {
            let game_hour = cell(&view, BOMBING[0], slot, TABLE)?;
            let attacker = optional(&view, BOMBING[1], slot, TABLE)?;
            let victim = cell(&view, BOMBING[2], slot, TABLE)? as u32;
            let cause = BuildingBombingDamageCause::try_from(cell(&view, BOMBING[3], slot, TABLE)? as u8)?;
            let amount = cell(&view, BOMBING[4], slot, TABLE)?;
            if !matches!(cause, BuildingBombingDamageCause::AirMission | BuildingBombingDamageCause::AirNaval) {
                continue;
            }
            for (column, raw) in [(0, attacker), (1, Some(victim))] {
                let (holder_slot, id) = raw.and_then(|raw| holders.of(raw)).unwrap_or((BombingTotalRow::NONE, BombingTotalRow::NONE));
                added.add(BombingTotalRow::key_of(holder_slot, id, cause as u8), column, amount, late(hour.hour, game_hour));
            }
        }
        added.land(TableId::BombingTotals, out);
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        Ok(())
    }
}
