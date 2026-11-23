
use recorder_derive_framework::framework::{
    Hour, columns,
    definitions::{Define, Form},
};
use engine_land_inputs::ncombat_log_cequipment_loss::month_bucket;
use replay_columnar::{
    manifest::Status,
    store::{Kept, TableView, Value as Held},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CombatLogEquipmentRow, CombatLogMonthRow, CombatLogStateRow, CountryLinkRow, CountryRow, DivisionCensusRow, EquipmentDestructionRow, LendLeaseEquipmentRow, PlayerRow, RowCodec, SimulationFeatureRow, SunkConvoyRow, SunkShipEntry,
        SunkShipsRow, TableId,
    },
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

pub(super) fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(super) const COUNTRY_SLOT: [usize; 1] = columns::<CountryRow, 1>(["canonical_slot"]);
pub(super) const LINK_SLOT: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
pub(super) const COMBAT_AMOUNT: [usize; 1] = columns::<CombatLogEquipmentRow, 1>(["amount_raw"]);
pub(super) const CONVOY_COUNT: [usize; 1] = columns::<SunkConvoyRow, 1>(["convoys"]);
pub(super) const LEND_AMOUNT: [usize; 1] = columns::<LendLeaseEquipmentRow, 1>(["amount_raw"]);
pub(super) const CENSUS: [usize; 2] = columns::<DivisionCensusRow, 2>(["grouped", "ungrouped"]);
pub(super) const GATE: [usize; 2] = columns::<CombatLogStateRow, 2>(["viewing_country_raw", "prune_observed"]);
pub(super) const PLAYER_COUNTRY: [usize; 1] = columns::<PlayerRow, 1>(["country"]);
pub(super) const FEATURES: [usize; 1] = columns::<SimulationFeatureRow, 1>(["dlc_mask"]);

fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, slot: u32, mask: u64) -> io::Result<R> {
    let held = hour.table(table);
    let key = held.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&held, slot, column, format))
}

fn unsigned(view: &TableView<'_>, column: usize, slot: u32) -> Option<u32> {
    view.number(column, slot).map(|value| value as u32)
}

pub(super) fn country(view: &TableView<'_>, slot: u32) -> Option<(u32, u32)> {
    Some((view.key(slot)? as u32, unsigned(view, COUNTRY_SLOT[0], slot)?))
}

pub(super) fn link(view: &TableView<'_>, slot: u32) -> Option<(u32, u32)> {
    Some((view.key(slot)? as u32, unsigned(view, LINK_SLOT[0], slot)?))
}

pub(super) fn destruction(hour: &Hour<'_>) -> io::Result<Vec<EquipmentDestructionRow>> {
    let view = hour.table(TableId::EquipmentDestruction);
    let mut rows = Vec::with_capacity(view.live().count());
    for slot in view.rows() {
        rows.push(row::<EquipmentDestructionRow>(hour, TableId::EquipmentDestruction, slot, u64::MAX)?);
    }
    rows.sort_unstable_by_key(RowCodec::key);
    Ok(rows)
}

pub(super) fn combat_country(key: u128) -> u32 {
    (key >> 88) as u32
}

pub(super) fn combat_suffix(key: u128) -> u128 {
    key & ((1 << 88) - 1)
}

pub(super) fn combat_key(country: u32, suffix: u128) -> u128 {
    (country as u128) << 88 | suffix
}

pub(super) fn combat_leaf(key: u128) -> (u8, u32) {
    ((key >> 48) as u8, key as u32)
}

pub(super) fn combat_amount(view: &TableView<'_>, slot: u32) -> Option<i64> {
    view.number(COMBAT_AMOUNT[0], slot)
}

pub(super) const COMBAT_DATE: [usize; 1] = columns::<CombatLogEquipmentRow, 1>(["date"]);

pub(super) fn combat_dropped(cells: &[Kept]) -> (Option<i32>, Option<i64>) {
    let number = |column: usize| match cells.get(column).map(Kept::value) {
        Some(Held::Number(bits)) => Some(bits as i64),
        _ => None,
    };
    (number(COMBAT_DATE[0]).map(|date| date as i32), number(COMBAT_AMOUNT[0]))
}

pub(super) fn combat_before(hour: &Hour<'_>) -> HashMap<u32, Option<i64>> {
    let mut before = HashMap::new();
    for (slot, column, value) in hour.previous(TableId::CombatLogEquipment) {
        if usize::from(*column) == COMBAT_AMOUNT[0] {
            before.entry(*slot).or_insert(match value.value() {
                Held::Number(bits) => Some(bits as i64),
                _ => None,
            });
        }
    }
    before
}

pub(super) fn dropped_amount(view: &TableView<'_>, slot: u32) -> Option<i64> {
    view.number(1, slot)
}

pub(super) const RETENTION: Define = Define { section: "NGame", name: "COMBAT_LOG_MAX_MONTHS", form: Form::Int32 };

pub(super) const MONTH_DATE: [usize; 1] = columns::<CombatLogMonthRow, 1>(["date"]);

fn array_of_line(key: u128) -> u128 {
    (key >> 88) << 40 | (key >> 56 & 0xFFFF_FFFF) << 8 | (key >> 48 & 0xFF)
}

fn array_of_month(key: u128) -> u128 {
    key >> 32
}

pub(super) fn freed_lines(hour: &Hour<'_>, live: &TableView<'_>, dropped: &TableView<'_>, gone: &[u128], whole: bool, turned: bool) -> Vec<u128> {
    let months = hour.table(TableId::CombatLogMonths);
    let observed = months.status() == Status::Complete;
    let changes = hour.changes(TableId::CombatLogMonths);
    let every = whole || changes.status_changed;
    let touched: HashSet<u128> = if every {
        HashSet::new()
    } else {
        let deleted = changes.deleted.iter().map(|&(_, key)| array_of_month(key));
        let dated = changes.written.iter().filter(|&&(_, column)| usize::from(column) == MONTH_DATE[0]).filter_map(|&(slot, _)| months.key(slot)).map(array_of_month);
        deleted.chain(dated).collect()
    };
    let gone: Vec<u128> = gone.iter().copied().filter(|&key| live.slot(key).is_none()).collect();
    let scan = if observed { every || !touched.is_empty() } else { every || turned };
    if !scan && gone.is_empty() {
        return Vec::new();
    }
    let retention = if observed { None } else { hour.definitions().define(RETENTION.section, RETENTION.name).ok().filter(|&months| months >= 1) };
    if !observed && retention.is_none() {
        return Vec::new();
    }
    let looked = |array: u128| every || touched.contains(&array);
    let mut oldest: HashMap<u128, u16> = HashMap::new();
    if observed {
        let arrays: HashSet<u128> = gone.iter().map(|&key| array_of_line(key)).collect();
        for slot in months.rows() {
            let Some(array) = months.key(slot).map(array_of_month).filter(|&array| looked(array) || arrays.contains(&array)) else { continue };
            let month = months.number(MONTH_DATE[0], slot).map_or(0, |date| u16::try_from(month_bucket(date as i32)).unwrap_or(0));
            let held = oldest.entry(array).or_insert(month);
            *held = (*held).min(month);
        }
    }
    let now = i64::from(month_bucket(hour.hour));
    let freed = |key: u128| {
        let (array, month) = (array_of_line(key), (key >> 32) as u16);
        match retention {
            None => oldest.get(&array).is_none_or(|&oldest| month < oldest),
            Some(retention) => i64::from(month) + retention + 2 <= now,
        }
    };
    let mut out: Vec<u128> = gone.iter().copied().filter(|&key| freed(key)).collect();
    if scan {
        let rows = dropped.rows().filter_map(|slot| dropped.key(slot));
        out.extend(rows.filter(|&key| (!observed || looked(array_of_line(key))) && live.slot(key).is_none() && freed(key)));
    }
    out.sort_unstable();
    out.dedup();
    out
}

pub(super) fn sunk_ships(hour: &Hour<'_>) -> io::Result<Option<Vec<SunkShipEntry>>> {
    let view = hour.table(TableId::SunkShips);
    let Some(slot) = view.rows().next() else { return Ok(None) };
    Ok(Some(row::<SunkShipsRow>(hour, TableId::SunkShips, slot, u64::MAX)?.entries))
}

pub(super) fn convoy(hour: &Hour<'_>, slot: u32) -> io::Result<(u128, u32, i32)> {
    let row: SunkConvoyRow = row(hour, TableId::SunkConvoys, slot, 1 << CONVOY_COUNT[0])?;
    Ok((row.key(), row.owner_country, row.convoys))
}

pub(super) struct Pool {
    pub key: u128,
    pub giver: u32,
    pub kind: u8,
    pub variant: u32,
    pub amount: i64,
}

pub(super) fn pool(hour: &Hour<'_>, slot: u32) -> io::Result<Pool> {
    let row: LendLeaseEquipmentRow = row(hour, TableId::LendLeaseEquipment, slot, 1 << LEND_AMOUNT[0])?;
    Ok(Pool { key: row.key(), giver: row.giver, kind: row.kind, variant: row.variant, amount: row.amount_raw })
}

pub(super) fn pool_key(key: u128) -> (u32, u8, u32) {
    ((key >> 72) as u32, (key >> 32) as u8, key as u32)
}

pub(super) fn census(view: &TableView<'_>, raw: u32) -> Option<(u32, u32)> {
    let slot = view.slot(u128::from(raw))?;
    Some((unsigned(view, CENSUS[0], slot)?, unsigned(view, CENSUS[1], slot)?))
}

pub(super) fn combat_state(hour: &Hour<'_>) -> Option<(i32, bool)> {
    single(hour, TableId::CombatLogState).and_then(|(view, slot)| Some((view.number(GATE[0], slot)? as i32, view.number(GATE[1], slot)? != 0)))
}

pub(super) fn players(hour: &Hour<'_>) -> Option<Vec<u32>> {
    if !hour.complete(TableId::Players) {
        return None;
    }
    let view = hour.table(TableId::Players);
    view.rows().map(|slot| unsigned(&view, PLAYER_COUNTRY[0], slot)).collect()
}

pub(super) fn feature_mask(hour: &Hour<'_>) -> Option<u32> {
    single(hour, TableId::SimulationFeatures).and_then(|(view, slot)| unsigned(&view, FEATURES[0], slot))
}

fn single<'a>(hour: &Hour<'a>, table: TableId) -> Option<(TableView<'a>, u32)> {
    if !hour.complete(table) {
        return None;
    }
    let view = hour.table(table);
    let mut rows = view.rows();
    let first = rows.next()?;
    rows.next().is_none().then_some((view, first))
}
