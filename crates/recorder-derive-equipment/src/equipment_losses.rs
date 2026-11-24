
use recorder_derive_framework::framework::{Columns, Definition, Hour, Output, Read, Rows, Uses, Value, Watch, Writer};
use engine_land_inputs::ncombat_log_cequipment_loss::month_bucket;
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
    store::{Dropped, Kept, TableView, Value as Held},
};
use replay_data::rows::{
    EQUIPMENT_DESTRUCTION_LEGACY_PARTIAL_REASON, EQUIPMENT_DESTRUCTION_PARTIAL_REASON, EquipmentDestructionCoverage, EquipmentLossCoverageRow as Coverage, EquipmentLossTotalRow as Total, Recorded, RowCodec, ShipLossRow, SunkShipEntry,
    TableId, compatibility::EQUIPMENT_DESTRUCTION_STARTUP_FORMAT,
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

mod inputs;
#[cfg(test)]
mod tests;

use inputs::*;

const READS: &[Read] = &[
    Read { table: TableId::Countries, columns: Columns::These(&COUNTRY_SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK_SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::EquipmentDestruction, columns: Columns::Every, watch: Watch::EveryRead, required: false },
    Read { table: TableId::CombatLogEquipment, columns: Columns::These(&COMBAT_AMOUNT), watch: Watch::Dropped, required: false },
    Read { table: TableId::SunkShips, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::SunkConvoys, columns: Columns::These(&CONVOY_COUNT), watch: Watch::Changed, required: false },
    Read { table: TableId::LendLeaseEquipment, columns: Columns::These(&LEND_AMOUNT), watch: Watch::EveryRead, required: false },
    Read { table: TableId::DivisionCensus, columns: Columns::These(&CENSUS), watch: Watch::Changed, required: false },
    Read { table: TableId::CombatLogState, columns: Columns::These(&GATE), watch: Watch::Changed, required: false },
    Read { table: TableId::CombatLogMonths, columns: Columns::These(&MONTH_DATE), watch: Watch::Changed, required: false },
    Read { table: TableId::Players, columns: Columns::These(&PLAYER_COUNTRY), watch: Watch::Changed, required: false },
    Read { table: TableId::SimulationFeatures, columns: Columns::These(&FEATURES), watch: Watch::Changed, required: false },
];

const TABLES: &[Output] = &[Output::State(TableId::EquipmentLossTotals), Output::State(TableId::CombatLogDropped), Output::State(TableId::ShipLosses), Output::State(TableId::EquipmentLossCoverage)];

const DESTRUCTION_TRUNCATED: &str = Coverage::DRAIN_TRUNCATED;
const SUPPLEMENTAL_NOT_RECORDED: &str = Coverage::DRAIN_SUPPLEMENTAL_NOT_RECORDED;
const OWNER_UNKNOWN: &str = Coverage::DRAIN_OWNER_UNKNOWN;
const NOT_RECORDED: &str = Coverage::DRAIN_NOT_RECORDED;
const NOT_READ: &str = Coverage::DRAIN_NOT_READ;
const LOST: &str = Coverage::DRAIN_LOST;

const EQUIPMENT_LOSSES_MASK: u32 = engine_dlc::dlc_feature_state::EQUIPMENT_LOSSES_MASK;

const FREED: usize = 2;

const RUN_KEY: u128 = (Coverage::RUN as u128) << 32 | Coverage::RUN as u128;

fn leaf(slot: u32, country: u32, source: u8, variant: Option<u32>) -> u128 {
    (slot as u128) << 73 | (country as u128) << 41 | (source as u128) << 33 | variant.map_or(0, |variant| 1 << 32 | variant as u128)
}

fn leaf_source(key: u128) -> u8 {
    (key >> 33) as u8
}

fn identity_key(slot: u32, country: u32) -> u128 {
    (slot as u128) << 32 | country as u128
}

fn ship_key(slot: u32, country: u32, hour: i32, signature: u64) -> u128 {
    ShipLossRow { slot, country, hour, signature, ..Default::default() }.key()
}

fn signature(entry: &SunkShipEntry, seed: u64) -> u64 {
    let mut hash = seed;
    let mut eat = |bytes: &[u8]| {
        for &byte in bytes {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    };
    eat(&(entry.ship_name.len() as u64).to_le_bytes());
    eat(&entry.ship_name);
    eat(&entry.sink_hour.to_le_bytes());
    eat(&entry.victim_equipment_variant.to_le_bytes());
    eat(&entry.battle.to_le_bytes());
    eat(&entry.damage_source_reference.to_le_bytes());
    eat(&[entry.cause_record]);
    let mut mixed = hash ^ seed.rotate_left(17);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

const SIGNATURE_SEED: u64 = 0xCBF2_9CE4_8422_2325;
const CHECK_SEED: u64 = 0x6C62_272E_07BB_0142;

struct Listed {
    slot: u32,
    signature: u64,
    count: u32,
    sink_hour: i32,
    variant: u64,
    check: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Add(i128),
    Set(i128),
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Run {
    counted_from: Option<i32>,
    withheld_from: Option<i32>,
    withheld: Option<String>,
    incomplete_from: Option<i32>,
    convoys_from: Option<i32>,
    lend_from: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Gate {
    known: bool,
    all: bool,
    players: HashSet<u32>,
    unresolved: bool,
    features: Option<bool>,
    prune_observed: bool,
}

#[derive(Default)]
struct Pending {
    totals: HashMap<u128, Change>,
    incomplete: HashMap<u128, i32>,
    coverage: HashSet<u128>,
    drops: HashMap<u128, u32>,
    ended: HashMap<u128, u32>,
    ended_from: Option<i32>,
    freed: HashMap<u128, Change>,
}

impl Pending {
    fn add(&mut self, key: u128, amount: i128) {
        let change = self.totals.entry(key).or_insert(Change::Add(0));
        *change = match *change {
            Change::Add(held) => Change::Add(held + amount),
            Change::Set(held) => Change::Set(held + amount),
            Change::Unknown => Change::Unknown,
        };
    }

    fn free(&mut self, key: u128, change: Change) {
        let held = self.freed.entry(key).or_insert(Change::Add(0));
        *held = match (*held, change) {
            (Change::Unknown, _) | (_, Change::Unknown) => Change::Unknown,
            (Change::Add(held), Change::Add(amount)) => Change::Add(held + amount),
            (Change::Set(held), Change::Add(amount)) => Change::Set(held + amount),
            (_, Change::Set(level)) => Change::Set(level),
        };
    }

    fn incomplete(&mut self, key: u128, from: i32) {
        let held = self.incomplete.entry(key).or_insert(from);
        *held = (*held).min(from);
    }
}

enum Recount {
    None,
    Slots(HashSet<u32>),
    All,
}

impl Recount {
    fn covers(&self, slot: u32) -> bool {
        match self {
            Self::None => false,
            Self::Slots(slots) => slots.contains(&slot),
            Self::All => true,
        }
    }
}

#[derive(Clone, Copy)]
struct Gone {
    key: u128,
    slot: u32,
    amount: Option<i64>,
}

struct Deleted<'a> {
    log: &'a [(u32, u128)],
    kept: &'a Dropped,
    rows: Vec<(u32, u32)>,
}

impl<'a> Deleted<'a> {
    fn read(hour: &Hour<'a>) -> io::Result<Self> {
        let table = TableId::CombatLogEquipment;
        let log: &'a [(u32, u128)] = &hour.changes(table).deleted;
        let mut rows: Vec<(u32, u32)> = (0..log.len() as u32).map(|at| (at, u32::MAX)).collect();
        rows.sort_unstable_by_key(|&(at, _)| (log[at as usize].1, std::cmp::Reverse(at)));
        rows.dedup_by_key(|(at, _)| log[*at as usize].1);
        let kept = hour.dropped(table);
        for (row, (key, _)) in kept.iter().enumerate() {
            if let Ok(at) = rows.binary_search_by_key(key, |&(at, _)| log[at as usize].1) {
                rows[at].1 = row as u32;
            }
        }
        if rows.iter().any(|&(_, row)| row == u32::MAX) {
            return Err(invalid("CombatLogEquipment rows were deleted without their cells kept"));
        }
        Ok(Self { log, kept, rows })
    }

    fn gone(&self, (at, row): (u32, u32)) -> Gone {
        let (slot, key) = self.log[at as usize];
        Gone { key, slot, amount: combat_dropped(self.kept.row(row as usize)).1 }
    }

    fn get(&self, key: u128) -> Option<Gone> {
        self.rows.binary_search_by_key(&key, |&(at, _)| self.log[at as usize].1).ok().map(|at| self.gone(self.rows[at]))
    }

    fn keys(&self) -> impl Iterator<Item = u128> + '_ {
        self.rows.iter().map(|&(at, _)| self.log[at as usize].1)
    }

    fn cells(&self) -> impl Iterator<Item = (u128, &'a [Kept])> + '_ {
        self.rows.iter().map(|&(at, row)| (self.log[at as usize].1, self.kept.row(row as usize)))
    }

    fn rows(&self) -> impl Iterator<Item = Gone> + '_ {
        self.rows.iter().map(|&row| self.gone(row))
    }
}

fn earliest(held: &mut Option<i32>, hour: i32) {
    *held = Some(held.map_or(hour, |from| from.min(hour)));
}

#[derive(Default)]
pub struct EquipmentLosses {
    rebuild: bool,
    last: Option<i32>,
    links: HashMap<u32, u32>,
    aliases: HashMap<u32, Vec<u32>>,
    countries: HashMap<u32, (u32, bool)>,
    identities: HashMap<u32, Vec<u32>>,
    run: Run,
    convoys: HashMap<u128, i32>,
    convoys_seen: bool,
    lend: HashMap<u128, i64>,
    lend_read: Option<i32>,
    lend_seen: bool,
    unresolved: HashSet<u32>,
    unresolved_combat: HashSet<u32>,
    listed: Option<Vec<(u32, u64, u32)>>,
    gate: Gate,
    census_complete: bool,
    census_unresolved: HashSet<u32>,
}

impl EquipmentLosses {
    fn ids(&self, slot: u32) -> &[u32] {
        self.identities.get(&slot).map_or(&[], Vec::as_slice)
    }

    fn slot_ids(&self, raw: u32) -> Option<(u32, &[u32])> {
        let slot = *self.links.get(&raw)?;
        Some((slot, self.ids(slot)))
    }

    fn read_identities(&mut self, hour: &Hour<'_>) {
        self.links.clear();
        self.aliases.clear();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            if let Some((raw, canonical)) = link(&links, slot) {
                self.links.insert(raw, canonical);
                self.aliases.entry(canonical).or_default().push(raw);
            }
        }
        self.aliases.values_mut().for_each(|raws| raws.sort_unstable());
        self.countries.clear();
        self.identities.clear();
        let countries = hour.table(TableId::Countries);
        for slot in countries.rows() {
            if let Some((raw, canonical)) = country(&countries, slot) {
                let resolved = self.links.get(&raw) == Some(&canonical);
                self.countries.insert(raw, (canonical, resolved));
                if resolved {
                    self.identities.entry(canonical).or_default().push(raw);
                }
            }
        }
        self.identities.values_mut().for_each(|ids| ids.sort_unstable());
    }

    fn read_run(&mut self, out: &Rows<'_>) {
        let view = out.table(TableId::EquipmentLossCoverage);
        let Some(slot) = view.slot(RUN_KEY) else {
            self.run = Run::default();
            return;
        };
        let at = |column: usize| match view.value(column, slot) {
            Held::Number(bits) => Some(bits as i32),
            _ => None,
        };
        let withheld_from = at(7);
        let drain_state = match view.value(15, slot) {
            Held::Number(bits) => Some(bits as u8),
            _ => None,
        };
        let withheld = withheld_from.and_then(|_| view.text(4, slot)).and_then(|id| view.string(id)).map(|text| {
            let text = String::from_utf8_lossy(text).into_owned();
            if drain_state.is_none_or(|state| state & Coverage::DRAIN_STATE_LOST != 0) {
                if text == LOST {
                    String::new()
                } else if let Some(rest) = text.strip_prefix(LOST).and_then(|rest| rest.strip_prefix("; ")) {
                    rest.to_owned()
                } else {
                    text
                }
            } else {
                text
            }
        });
        self.run = Run { counted_from: at(6), withheld_from, withheld, incomplete_from: at(8), convoys_from: at(9), lend_from: at(10) };
    }

    fn identities_changed(&mut self, pending: &mut Pending, out: &mut Rows<'_>) {
        for key in recorder_derive_framework::forces::held(out, TableId::EquipmentLossCoverage) {
            if key != RUN_KEY && !self.countries.get(&(key as u32)).is_some_and(|&(slot, _)| slot == (key >> 32) as u32) {
                out.delete(TableId::EquipmentLossCoverage, key);
            }
        }
        pending.coverage.extend(self.countries.iter().map(|(&raw, &(slot, _))| identity_key(slot, raw)));
    }

    fn group_value(&self, aliases: &[u32], suffix: u128, value: impl Fn(u128) -> Option<Option<i64>>) -> Option<Option<i64>> {
        aliases.iter().rev().find_map(|&raw| value(combat_key(raw, suffix)))
    }

    fn moved_slots(&self, aliases: &HashMap<u32, Vec<u32>>, identities: &HashMap<u32, Vec<u32>>) -> HashSet<u32> {
        let mut moved = HashSet::new();
        for (before, now) in [(aliases, &self.aliases), (identities, &self.identities)] {
            moved.extend(before.iter().filter(|(slot, held)| now.get(*slot) != Some(*held)).map(|(&slot, _)| slot));
            moved.extend(now.iter().filter(|(slot, held)| before.get(*slot) != Some(*held)).map(|(&slot, _)| slot));
        }
        moved
    }

    fn combat(&mut self, hour: &Hour<'_>, recount: &Recount, (whole, turned): (bool, bool), pending: &mut Pending, out: &mut Rows<'_>) -> io::Result<()> {
        let table = TableId::CombatLogEquipment;
        let live = hour.table(table);
        let deleted = Deleted::read(hour)?;
        let gone: Vec<u128> = deleted.keys().collect();
        let freed = freed_lines(hour, &live, &out.table(TableId::CombatLogDropped), &gone, whole, turned);
        for (key, cells) in deleted.cells() {
            if live.slot(key).is_none() && freed.binary_search(&key).is_err() {
                let (date, amount) = combat_dropped(cells);
                out.set(TableId::CombatLogDropped, key, vec![date.map_or(Value::Unrecorded, Value::I32), amount.map_or(Value::Unrecorded, Value::I64)]);
            }
        }
        let all = matches!(recount, Recount::All);
        let inserted = &hour.changes(table).inserted;
        for slot in live.rows().filter(|_| all).chain(inserted.iter().copied().filter(|_| !all)) {
            let Some(key) = live.key(slot) else { continue };
            if out.table(TableId::CombatLogDropped).slot(key).is_some() {
                out.delete(TableId::CombatLogDropped, key);
            }
        }
        let dropped = out.table(TableId::CombatLogDropped);
        let now = |key: u128| live.slot(key).map(|slot| combat_amount(&live, slot));
        let held = |key: u128| dropped.slot(key).map(|slot| dropped_amount(&dropped, slot));
        let after = |key: u128| now(key).or_else(|| deleted.get(key).map(|gone| gone.amount)).or_else(|| held(key));
        let release: Vec<u128> = freed.iter().copied().filter(|&key| dropped.slot(key).is_some()).collect();
        let mut unresolved = HashSet::new();
        if !matches!(recount, Recount::All) {
            let before = combat_before(hour);
            let inserted: HashSet<u32> = hour.changes(table).inserted.iter().copied().collect();
            let was = |key: u128| -> Option<Option<i64>> {
                if let Some(gone) = deleted.get(key) {
                    return Some(before.get(&gone.slot).copied().unwrap_or(gone.amount));
                }
                match live.slot(key) {
                    Some(slot) if !inserted.contains(&slot) => Some(before.get(&slot).copied().unwrap_or_else(|| combat_amount(&live, slot))),
                    _ => held(key),
                }
            };
            let written = hour.written_rows(table, Columns::These(&COMBAT_AMOUNT));
            let touched = written.iter().filter_map(|&slot| live.key(slot)).chain(deleted.keys());
            let mut groups: HashSet<(u32, u128)> = HashSet::new();
            for key in touched {
                let Some(&slot) = self.links.get(&combat_country(key)) else {
                    unresolved.insert(combat_country(key));
                    continue;
                };
                if recount.covers(slot) {
                    continue;
                }
                let suffix = combat_suffix(key);
                let aliases = self.aliases.get(&slot).map_or(&[][..], Vec::as_slice);
                if aliases.len() > 1 && !groups.insert((slot, suffix)) {
                    continue;
                }
                let old = self.group_value(aliases, suffix, &was);
                let new = self.group_value(aliases, suffix, &after);
                let change = match (old.unwrap_or(Some(0)), new.unwrap_or(Some(0))) {
                    (Some(old), Some(new)) => Change::Add(i128::from(new) - i128::from(old)),
                    _ => Change::Unknown,
                };
                let (kind, variant) = combat_leaf(key);
                let source = combat_source(kind)?;
                for &id in self.ids(slot) {
                    let key = leaf(slot, id, source, Some(variant));
                    match change {
                        Change::Add(amount) => pending.add(key, amount),
                        other => {
                            pending.totals.insert(key, other);
                        }
                    }
                }
            }
        }
        if !matches!(recount, Recount::None) {
            self.combat_levels(&out.table(TableId::EquipmentLossTotals), &live, &dropped, &deleted, recount, pending, &mut unresolved)?;
        }
        self.free(&freed, &after, pending)?;
        for key in release {
            out.delete(TableId::CombatLogDropped, key);
        }
        self.note_combat_unresolved(unresolved);
        Ok(())
    }

    fn free(&self, freed: &[u128], after: &impl Fn(u128) -> Option<Option<i64>>, pending: &mut Pending) -> io::Result<()> {
        let without = |key: u128| if freed.binary_search(&key).is_ok() { None } else { after(key) };
        let mut groups: HashSet<(u32, u128)> = HashSet::new();
        for &key in freed {
            let Some(&slot) = self.links.get(&combat_country(key)) else { continue };
            let suffix = combat_suffix(key);
            let aliases = self.aliases.get(&slot).map_or(&[][..], Vec::as_slice);
            if aliases.len() > 1 && !groups.insert((slot, suffix)) {
                continue;
            }
            let change = match (self.group_value(aliases, suffix, after).unwrap_or(Some(0)), self.group_value(aliases, suffix, without).unwrap_or(Some(0))) {
                (Some(old), Some(new)) => Change::Add(i128::from(old) - i128::from(new)),
                _ => Change::Unknown,
            };
            let (kind, variant) = combat_leaf(key);
            let source = combat_source(kind)?;
            for &id in self.ids(slot) {
                pending.free(leaf(slot, id, source, Some(variant)), change);
            }
        }
        Ok(())
    }

    fn note_combat_unresolved(&mut self, raws: HashSet<u32>) {
        for raw in raws {
            if !self.unresolved.contains(&raw) {
                self.unresolved_combat.insert(raw);
            }
        }
    }

    fn note_unresolved(&mut self, raw: u32) {
        self.unresolved_combat.remove(&raw);
        self.unresolved.insert(raw);
    }

    fn combat_levels(&self, totals: &TableView<'_>, live: &TableView<'_>, dropped: &TableView<'_>, deleted: &Deleted<'_>, recount: &Recount, pending: &mut Pending, unresolved: &mut HashSet<u32>) -> io::Result<()> {
        let present = |key: u128| live.slot(key).is_some() || deleted.get(key).is_some() || dropped.slot(key).is_some();
        let mut levels: HashMap<(u32, u8, u32), Option<i128>> = HashMap::new();
        {
            let mut take = |key: u128, amount: Option<i64>| {
                let raw = combat_country(key);
                let Some(&slot) = self.links.get(&raw) else {
                    unresolved.insert(raw);
                    return;
                };
                if !recount.covers(slot) {
                    return;
                }
                let suffix = combat_suffix(key);
                let aliases = self.aliases.get(&slot).map_or(&[][..], Vec::as_slice);
                if aliases.iter().rev().take_while(|&&alias| alias > raw).any(|&alias| present(combat_key(alias, suffix))) {
                    return;
                }
                let (kind, variant) = combat_leaf(suffix);
                let level = levels.entry((slot, kind, variant)).or_insert(Some(0));
                *level = level.zip(amount).map(|(sum, amount)| sum + i128::from(amount));
            };
            for slot in live.rows() {
                if let Some(key) = live.key(slot) {
                    take(key, combat_amount(live, slot));
                }
            }
            for slot in dropped.rows() {
                if let Some(key) = dropped.key(slot)
                    && live.slot(key).is_none()
                    && deleted.get(key).is_none()
                {
                    take(key, dropped_amount(dropped, slot));
                }
            }
            for gone in deleted.rows() {
                if live.slot(gone.key).is_none() {
                    take(gone.key, gone.amount);
                }
            }
        }
        let mut freed: HashMap<(u32, u8, u32), Option<i128>> = HashMap::new();
        let mut baseline = Vec::new();
        for row in totals.rows() {
            let Some(key) = totals.key(row) else { continue };
            let (slot, id) = ((key >> 73) as u32, (key >> 41) as u32);
            if leaf_source(key) >= Total::SUNK_CONVOYS || !recount.covers(slot) {
                continue;
            }
            let held = totals.number(FREED, row).map(i128::from);
            let most = freed.entry((slot, leaf_source(key), key as u32)).or_insert(Some(0));
            *most = most.zip(held).map(|(most, held)| most.max(held));
            if self.ids(slot).contains(&id) {
                baseline.push(key);
            }
        }
        let freed_of = |slot: u32, source: u8, variant: u32| freed.get(&(slot, source, variant)).copied().unwrap_or(Some(0));
        let set = |amount: Option<i128>| amount.map_or(Change::Unknown, Change::Set);
        for key in baseline {
            let amount = freed_of((key >> 73) as u32, leaf_source(key), key as u32);
            pending.totals.insert(key, set(amount));
            pending.freed.insert(key, set(amount));
        }
        for ((slot, kind, variant), level) in levels {
            let source = combat_source(kind)?;
            let amount = freed_of(slot, source, variant);
            for &id in self.ids(slot) {
                let key = leaf(slot, id, source, Some(variant));
                pending.totals.insert(key, set(level.zip(amount).map(|(level, amount)| level + amount)));
                pending.freed.insert(key, set(amount));
            }
        }
        Ok(())
    }

    fn destruction(&mut self, hour: &Hour<'_>, out: &Rows<'_>, pending: &mut Pending) -> io::Result<(u8, String)> {
        if let Some(reason) = &self.run.withheld {
            return Ok((Coverage::DRAIN_STATE_WITHHELD, reason.clone()));
        }
        let view = hour.table(TableId::EquipmentDestruction);
        let at = hour.hour;
        match view.status() {
            Status::Complete if view.sampled_at() == Some(at) => {}
            Status::Complete => {
                if self.run.counted_from.is_none() {
                    return Ok((Coverage::DRAIN_STATE_NOT_READ, String::new()));
                }
                earliest(&mut self.run.incomplete_from, at);
                return Ok((Coverage::DRAIN_STATE_NOT_READ, NOT_READ.into()));
            }
            Status::Unavailable { reason } => {
                let text = view.string(reason).map(|text| String::from_utf8_lossy(text).into_owned()).unwrap_or_default();
                if hour.format < EQUIPMENT_DESTRUCTION_STARTUP_FORMAT && text == EQUIPMENT_DESTRUCTION_LEGACY_PARTIAL_REASON {
                    if self.run.counted_from.is_some() {
                        earliest(&mut self.run.incomplete_from, at);
                    }
                    return Ok((Coverage::DRAIN_STATE_PARTIAL_INTERVAL, text));
                }
                let detail = if text.starts_with(replay_frame::frame::TRUNCATED_KEYFRAME_REASON) { DESTRUCTION_TRUNCATED.to_owned() } else { text };
                return Ok((Coverage::DRAIN_STATE_WITHHELD, self.withhold(at, out, detail, pending)));
            }
            Status::Truncated { .. } => return Ok((Coverage::DRAIN_STATE_WITHHELD, self.withhold(at, out, DESTRUCTION_TRUNCATED.into(), pending))),
            Status::NotRecorded => return Ok((Coverage::DRAIN_STATE_WITHHELD, self.withhold(at, out, NOT_RECORDED.into(), pending))),
        }
        let rows = destruction(hour)?;
        if rows.iter().any(|row| row.coverage == Recorded::Value(EquipmentDestructionCoverage::PartialInterval)) {
            if rows.len() != 1 {
                return Err(invalid("Equipment destruction partial interval marker cannot appear with loss rows"));
            }
            let text = match &rows[0].unavailable_reason {
                Recorded::Value(reason) => reason.clone(),
                Recorded::NotRecorded => EQUIPMENT_DESTRUCTION_PARTIAL_REASON.to_owned(),
            };
            if self.run.counted_from.is_some() {
                earliest(&mut self.run.incomplete_from, at);
            }
            return Ok((Coverage::DRAIN_STATE_PARTIAL_INTERVAL, text));
        }
        if self.run.counted_from.is_none() {
            self.run.counted_from = Some(at);
        }
        let mut supplemental = None;
        let mut unresolved = false;
        let mut sums: HashMap<(u32, u8, u32), i128> = HashMap::new();
        for row in &rows {
            if row.coverage == Recorded::Value(EquipmentDestructionCoverage::SupplementalUnavailable) {
                supplemental = Some(match &row.unavailable_reason {
                    Recorded::Value(reason) => reason.clone(),
                    Recorded::NotRecorded => SUPPLEMENTAL_NOT_RECORDED.to_owned(),
                });
                continue;
            }
            if row.not_recorded_fields() != 0 {
                supplemental.get_or_insert_with(|| SUPPLEMENTAL_NOT_RECORDED.to_owned());
            }
            let variant = Total::variant_id(row.variant).ok_or_else(|| invalid(format!("Equipment destruction design {:#x} has an unsupported reference type", row.variant)))?;
            if row.amount_raw == 0 {
                return Err(invalid("Equipment destruction contains a zero event amount"));
            }
            let Some(&slot) = self.links.get(&row.country) else {
                unresolved = true;
                self.note_unresolved(row.country);
                continue;
            };
            *sums.entry((slot, row.cause as u8, variant)).or_default() += i128::from(row.amount_raw);
        }
        for ((slot, cause, variant), amount) in sums {
            for &id in self.ids(slot) {
                pending.add(leaf(slot, id, Total::DESTRUCTION + cause, Some(variant)), amount);
            }
        }
        let mut state = Coverage::DRAIN_STATE_COMPLETE;
        if unresolved {
            state |= Coverage::DRAIN_STATE_OWNER_UNKNOWN;
        }
        if supplemental.is_some() {
            state |= Coverage::DRAIN_STATE_SUPPLEMENTAL_UNAVAILABLE;
        }
        let reason = match (unresolved.then_some(OWNER_UNKNOWN), supplemental) {
            (Some(owner), Some(supplemental)) => format!("{owner}; {supplemental}"),
            (Some(owner), None) => owner.to_owned(),
            (None, Some(supplemental)) => supplemental,
            (None, None) => String::new(),
        };
        if !reason.is_empty() {
            earliest(&mut self.run.incomplete_from, at);
        }
        Ok((state, reason))
    }

    fn withhold(&mut self, hour: i32, out: &Rows<'_>, reason: String, pending: &mut Pending) -> String {
        self.run.withheld_from = Some(hour);
        self.run.withheld = Some(reason.clone());
        earliest(&mut self.run.incomplete_from, hour);
        let totals = out.table(TableId::EquipmentLossTotals);
        for key in totals.rows().filter_map(|slot| totals.key(slot)) {
            if leaf_source(key) >= Total::DESTRUCTION {
                pending.totals.insert(key, Change::Unknown);
            }
        }
        reason
    }

    fn sunk(&mut self, hour: &Hour<'_>) -> io::Result<Option<Vec<Listed>>> {
        let Some(entries) = sunk_ships(hour)? else { return Ok(None) };
        let mut counts: HashMap<(u32, u64), Listed> = HashMap::new();
        for entry in &entries {
            let Some(&slot) = self.links.get(&entry.victim_country) else {
                self.note_unresolved(entry.victim_country);
                continue;
            };
            let check = signature(entry, CHECK_SEED);
            let hash = signature(entry, SIGNATURE_SEED) & ShipLossRow::SIGNATURE;
            let held = counts.entry((slot, hash)).or_insert(Listed { slot, signature: hash, count: 0, sink_hour: entry.sink_hour, variant: entry.victim_equipment_variant, check });
            if held.check != check {
                return Err(invalid("Two sunk ships' signatures share a hash"));
            }
            held.count += 1;
        }
        let mut listed: Vec<Listed> = counts.into_values().collect();
        listed.sort_unstable_by_key(|ship| (ship.slot, ship.signature));
        Ok(Some(listed))
    }

    fn ships(&mut self, hour: &Hour<'_>, recount: &Recount, out: &mut Rows<'_>) -> io::Result<()> {
        for key in recorder_derive_framework::forces::held(out, TableId::ShipLosses) {
            out.delete(TableId::ShipLosses, key);
        }
        if !hour.complete(TableId::SunkShips) {
            self.listed = None;
            return Ok(());
        }
        let before = if hour.fresh { None } else { self.listed.take() };
        let moved = matches!(recount, Recount::Slots(slots) if !slots.is_empty());
        if before.is_some() && !moved && !hour.changed(TableId::SunkShips) {
            self.listed = before;
            return Ok(());
        }
        let listed = self.sunk(hour)?.unwrap_or_default();
        for ship in &listed {
            if let Some(before) = &before
                && !recount.covers(ship.slot)
            {
                let earlier = before.binary_search_by_key(&(ship.slot, ship.signature), |&(slot, signature, _)| (slot, signature)).map_or(0, |at| before[at].2);
                if ship.count <= earlier {
                    continue;
                }
            }
            for &id in self.ids(ship.slot) {
                if ship.slot > ShipLossRow::FIELD || id > ShipLossRow::FIELD {
                    return Err(invalid("A sunk ship's slot or country passes 24 bits"));
                }
                let cells = vec![Value::I32(ship.sink_hour), Value::U64(ship.variant), Value::U32(ship.count), Value::U64(ship.check)];
                out.set(TableId::ShipLosses, ship_key(ship.slot, id, hour.hour, ship.signature), cells);
            }
        }
        self.listed = Some(listed.into_iter().map(|ship| (ship.slot, ship.signature, ship.count)).collect());
        Ok(())
    }

    fn convoys(&mut self, hour: &Hour<'_>, whole: bool, pending: &mut Pending) -> io::Result<()> {
        let table = TableId::SunkConvoys;
        let view = hour.table(table);
        if view.status() != Status::Complete || view.sampled_at().is_none() {
            return Ok(());
        }
        if !self.convoys_seen {
            self.convoys.clear();
            for slot in view.rows() {
                let (key, _, count) = convoy(hour, slot)?;
                self.convoys.insert(key, count);
            }
            self.convoys_seen = true;
            self.run.convoys_from = Some(hour.hour);
            return Ok(());
        }
        let visit: Vec<u32> = if whole {
            let live: HashSet<u128> = view.rows().filter_map(|slot| view.key(slot)).collect();
            self.convoys.retain(|key, _| live.contains(key));
            view.rows().collect()
        } else {
            for key in hour.deleted_keys(table) {
                self.convoys.remove(&key);
            }
            hour.written_rows(table, Columns::These(&CONVOY_COUNT))
        };
        for slot in visit {
            let (key, owner, count) = convoy(hour, slot)?;
            let last = self.convoys.insert(key, count).unwrap_or(0);
            let (amount, fell) = if count < last { (count, true) } else { (count - last, false) };
            if !self.links.contains_key(&owner) {
                self.note_unresolved(owner);
                continue;
            }
            let Some((slot, ids)) = self.slot_ids(owner) else { continue };
            for &id in ids {
                if fell {
                    *pending.drops.entry(identity_key(slot, id)).or_default() += 1;
                }
                if amount != 0 {
                    pending.add(leaf(slot, id, Total::SUNK_CONVOYS, None), i128::from(amount) * i128::from(AMOUNT_SCALE));
                }
            }
        }
        Ok(())
    }

    fn lend(&mut self, hour: &Hour<'_>, whole: bool, pending: &mut Pending) -> io::Result<()> {
        let table = TableId::LendLeaseEquipment;
        let view = hour.table(table);
        let read = view.status() == Status::Complete && view.sampled_at() == Some(hour.hour);
        if view.status() != Status::Complete || view.sampled_at().is_none() || !(read || whole) {
            return Ok(());
        }
        if !self.lend_seen {
            if !read {
                return Ok(());
            }
            self.lend.clear();
            for slot in view.rows() {
                let pool = pool(hour, slot)?;
                self.lend.insert(pool.key, pool.amount);
            }
            self.lend_seen = true;
            self.lend_read = Some(hour.hour);
            self.run.lend_from = Some(hour.hour);
            return Ok(());
        }
        let ended_from = self.lend_read.map_or(hour.hour, |read| read.saturating_add(1));
        let (gone, visit): (Vec<u128>, Vec<u32>) = if whole {
            let live: HashSet<u128> = view.rows().filter_map(|slot| view.key(slot)).collect();
            (self.lend.keys().filter(|key| !live.contains(key)).copied().collect(), view.rows().collect())
        } else {
            (hour.deleted_keys(table).collect(), hour.written_rows(table, Columns::These(&LEND_AMOUNT)))
        };
        let mut removed = Vec::new();
        for key in gone {
            if self.lend.remove(&key).is_none() {
                continue;
            }
            removed.push(key);
            let (giver, kind, variant) = pool_key(key);
            let source = lend_source(kind)?;
            if let Some((slot, ids)) = self.slot_ids(giver) {
                for &id in ids {
                    pending.incomplete(leaf(slot, id, source, Some(variant)), ended_from);
                }
            }
        }
        for slot in visit {
            let pool = pool(hour, slot)?;
            let last = self.lend.insert(pool.key, pool.amount).unwrap_or(0);
            let amount = i128::from(pool.amount) - i128::from(last);
            let source = lend_source(pool.kind)?;
            if !self.links.contains_key(&pool.giver) {
                self.note_unresolved(pool.giver);
            }
            if amount == 0 {
                continue;
            }
            if let Some((slot, ids)) = self.slot_ids(pool.giver) {
                for &id in ids {
                    pending.add(leaf(slot, id, source, Some(pool.variant)), amount);
                }
            }
        }
        if !removed.is_empty() {
            let open: HashSet<u128> = self.lend.keys().map(|&key| exchange_of(key)).collect();
            let ended: HashSet<u128> = removed.into_iter().map(exchange_of).filter(|exchange| !open.contains(exchange)).collect();
            for exchange in ended {
                if let Some((slot, ids)) = self.slot_ids((exchange >> 32) as u32) {
                    for &id in ids {
                        *pending.ended.entry(identity_key(slot, id)).or_default() += 1;
                    }
                    pending.ended_from = Some(ended_from);
                }
            }
        }
        if read {
            self.lend_read = Some(hour.hour);
        }
        Ok(())
    }

    fn land(&mut self, hour: &Hour<'_>, whole: bool, pending: &mut Pending) {
        let moved = |table: TableId, columns: Columns| hour.changes(table).status_changed || hour.wrote(table, columns);
        let mut every = whole;
        if whole || moved(TableId::CombatLogState, Columns::These(&GATE)) || moved(TableId::Players, Columns::These(&PLAYER_COUNTRY)) || moved(TableId::SimulationFeatures, Columns::These(&FEATURES)) {
            let gate = self.read_gate(hour);
            every |= gate != self.gate;
            self.gate = gate;
        }
        let census = hour.table(TableId::DivisionCensus);
        let complete = census.status() == Status::Complete;
        let had_unresolved = !self.census_unresolved.is_empty();
        if complete != self.census_complete || (complete && whole) {
            every = true;
            self.census_unresolved.clear();
            if complete {
                for slot in census.rows() {
                    let Some(raw) = census.key(slot).map(|key| key as u32) else { continue };
                    self.census_raw(&census, raw);
                }
            }
        } else if complete {
            let mut raws: Vec<u32> = hour.written_rows(TableId::DivisionCensus, Columns::These(&CENSUS)).into_iter().filter_map(|slot| census.key(slot)).map(|key| key as u32).collect();
            raws.extend(hour.deleted_keys(TableId::DivisionCensus).map(|key| key as u32));
            for raw in raws {
                self.census_raw(&census, raw);
                if let Some((slot, ids)) = self.slot_ids(raw) {
                    pending.coverage.extend(ids.iter().map(|&id| identity_key(slot, id)));
                }
            }
        }
        self.census_complete = complete;
        every |= had_unresolved != !self.census_unresolved.is_empty();
        if every {
            pending.coverage.extend(self.countries.iter().map(|(&raw, &(slot, _))| identity_key(slot, raw)));
        }
    }

    fn census_raw(&mut self, census: &TableView<'_>, raw: u32) {
        let counted = census.slot(u128::from(raw)).is_some() && census_of(census, raw).is_none_or(|(grouped, ungrouped)| grouped != 0 || ungrouped != 0);
        if counted && !self.links.contains_key(&raw) {
            self.census_unresolved.insert(raw);
        } else {
            self.census_unresolved.remove(&raw);
        }
    }

    fn read_gate(&self, hour: &Hour<'_>) -> Gate {
        let Some((operand, prune_observed)) = combat_state(hour) else { return Gate::default() };
        let mut gate = Gate { all: operand < 1, prune_observed, ..Gate::default() };
        if !gate.all {
            let Some(players) = players(hour) else { return gate };
            for raw in players {
                match self.links.get(&raw) {
                    Some(&slot) => {
                        gate.players.insert(slot);
                    }
                    None => gate.unresolved = true,
                }
            }
        }
        gate.known = true;
        gate.features = feature_mask(hour).map(|mask| mask & EQUIPMENT_LOSSES_MASK != 0);
        gate
    }

    fn coverage(&self, hour: &Hour<'_>, slot: u32, resolved: bool) -> (u8, Option<u32>) {
        if !resolved || !self.gate.known {
            return (Coverage::UNKNOWN, None);
        }
        if !self.gate.all && !self.gate.players.contains(&slot) {
            return (if self.gate.unresolved { Coverage::UNKNOWN } else { Coverage::NOT_LOGGED }, None);
        }
        match self.gate.features {
            None => return (Coverage::UNKNOWN, None),
            Some(false) => return (Coverage::NOT_LOGGED, None),
            Some(true) => {}
        }
        if !self.census_complete || !self.census_unresolved.is_empty() {
            return (Coverage::UNKNOWN, None);
        }
        let census = hour.table(TableId::DivisionCensus);
        let (mut grouped, mut ungrouped, mut found) = (0_u64, 0_u64, false);
        for &raw in self.aliases.get(&slot).map_or(&[][..], Vec::as_slice) {
            if census.slot(u128::from(raw)).is_none() {
                continue;
            }
            let Some((more_grouped, more_ungrouped)) = census_of(&census, raw) else { return (Coverage::UNKNOWN, None) };
            grouped += u64::from(more_grouped);
            ungrouped += u64::from(more_ungrouped);
            found = true;
        }
        let (Ok(grouped), Ok(ungrouped)) = (u32::try_from(grouped), u32::try_from(ungrouped)) else { return (Coverage::UNKNOWN, None) };
        land_coverage(found, grouped, ungrouped, self.gate.prune_observed)
    }

    fn write(&self, hour: &Hour<'_>, pending: Pending, drain: &str, drain_state: u8, out: &mut Rows<'_>) {
        let mut keys: HashSet<u128> = pending.totals.keys().copied().collect();
        keys.extend(pending.incomplete.keys().copied());
        keys.extend(pending.freed.keys().copied());
        for key in keys {
            let totals = out.table(TableId::EquipmentLossTotals);
            let held = totals.slot(key);
            if held.is_none() && !pending.totals.contains_key(&key) && !pending.incomplete.contains_key(&key) {
                continue;
            }
            let held_total = held.map(|slot| totals.number(0, slot));
            let held_from = held.and_then(|slot| totals.number(1, slot)).map(|from| from as i32);
            let from = match (held_from, pending.incomplete.get(&key)) {
                (Some(held), Some(&from)) => Some(held.min(from)),
                (held, from) => held.or(from.copied()),
            };
            let total = match (pending.totals.get(&key), held_total) {
                (None, Some(held)) => held,
                (None, None) => Some(0),
                (Some(Change::Add(0) | Change::Set(0)), None) if from.is_none() => continue,
                (Some(Change::Add(amount)), Some(Some(held))) => i64::try_from(i128::from(held) + amount).ok(),
                (Some(Change::Add(_)), Some(None)) => None,
                (Some(Change::Add(amount)), None) => i64::try_from(*amount).ok(),
                (Some(Change::Set(level)), _) => i64::try_from(*level).ok(),
                (Some(Change::Unknown), _) => None,
            };
            let freed = (leaf_source(key) < Total::SUNK_CONVOYS)
                .then(|| {
                    let base = held.map_or(Some(0), |slot| totals.number(FREED, slot));
                    match pending.freed.get(&key) {
                        None => base,
                        Some(Change::Add(amount)) => base.and_then(|base| i64::try_from(i128::from(base) + amount).ok()),
                        Some(Change::Set(level)) => i64::try_from(*level).ok(),
                        Some(Change::Unknown) => None,
                    }
                })
                .flatten();
            out.set(TableId::EquipmentLossTotals, key, vec![total.map_or(Value::Unrecorded, Value::I64), Value::OptionalI32(from), freed.map_or(Value::Unrecorded, Value::I64)]);
        }
        let mut identities = pending.coverage;
        identities.extend(pending.drops.keys().copied());
        identities.extend(pending.ended.keys().copied());
        for key in identities {
            let (slot, raw) = ((key >> 32) as u32, key as u32);
            let Some(&(held_slot, resolved)) = self.countries.get(&raw) else { continue };
            if held_slot != slot {
                continue;
            }
            let (land, ungrouped) = self.coverage(hour, slot, resolved);
            let coverage = out.table(TableId::EquipmentLossCoverage);
            let held = |column: usize| coverage.slot(key).and_then(|at| coverage.number(column, at));
            let drops = held(3).map_or(0, |held| held as u32) + pending.drops.get(&key).copied().unwrap_or(0);
            let ended = pending.ended.get(&key).copied().unwrap_or(0);
            let ended_from = if ended == 0 { held(12).map(|from| from as i32) } else { pending.ended_from };
            let mut cells = vec![Value::U8(land), ungrouped.map_or(Value::Unrecorded, Value::U32), Value::U8(u8::from(resolved)), Value::U32(drops)];
            cells.resize(11, Value::Unrecorded);
            cells.extend([Value::U32(held(11).map_or(0, |held| held as u32) + ended), Value::OptionalI32(ended_from), Value::Unrecorded, Value::Unrecorded, Value::Unrecorded]);
            out.set(TableId::EquipmentLossCoverage, key, cells);
        }
        let mut unread = 0;
        for (table, bit) in [
            (TableId::CombatLogEquipment, Coverage::UNREAD_COMBAT_LOG),
            (TableId::SunkShips, Coverage::UNREAD_SHIPS),
            (TableId::SunkConvoys, Coverage::UNREAD_CONVOYS),
            (TableId::LendLeaseEquipment, Coverage::UNREAD_LEND_LEASE),
        ] {
            if !hour.complete(table) {
                unread |= bit;
            }
        }
        let run = &self.run;
        let mut cells = vec![Value::Unrecorded; 4];
        cells.extend([
            out.text(drain.as_bytes()),
            Value::U8(unread),
            Value::OptionalI32(run.counted_from),
            Value::OptionalI32(run.withheld_from),
            Value::OptionalI32(run.incomplete_from),
            Value::OptionalI32(run.convoys_from),
            Value::OptionalI32(run.lend_from),
            Value::Unrecorded,
            Value::Unrecorded,
            Value::U32(self.unresolved.len() as u32),
            Value::U32((self.unresolved.len() + self.unresolved_combat.len()) as u32),
            Value::U8(drain_state),
        ]);
        out.set(TableId::EquipmentLossCoverage, RUN_KEY, cells);
    }
}

const AMOUNT_SCALE: i64 = 100_000;

fn exchange_of(key: u128) -> u128 {
    key >> 40
}

fn census_of(census: &TableView<'_>, raw: u32) -> Option<(u32, u32)> {
    inputs::census(census, raw)
}

fn combat_source(kind: u8) -> io::Result<u8> {
    if kind < Total::SUNK_CONVOYS - Total::COMBAT_LOG {
        Ok(Total::COMBAT_LOG + kind)
    } else {
        Err(invalid(format!("Combat Log kind {kind} has no equipment loss source")))
    }
}

fn lend_source(kind: u8) -> io::Result<u8> {
    if kind < Total::DESTRUCTION - Total::LEND_LEASE {
        Ok(Total::LEND_LEASE + kind)
    } else {
        Err(invalid(format!("Lend-lease pool kind {kind} has no equipment loss source")))
    }
}

fn land_coverage(found: bool, grouped: u32, ungrouped: u32, prune_observed: bool) -> (u8, Option<u32>) {
    if !found {
        (Coverage::UNKNOWN, None)
    } else if grouped == 0 && ungrouped != 0 {
        (Coverage::NOT_LOGGED, None)
    } else if ungrouped != 0 {
        (Coverage::PARTIAL, Some(ungrouped))
    } else if prune_observed {
        (Coverage::LOGGED, None)
    } else {
        (Coverage::PRUNE_UNOBSERVED, None)
    }
}

impl Writer for EquipmentLosses {
    fn name(&self) -> &'static str {
        "EquipmentLosses"
    }

    fn version(&self) -> u32 {
        5
    }

    fn tables(&self) -> &'static [Output] {
        TABLES
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn definitions(&self) -> &'static [Uses] {
        &[Uses { definition: Definition::Define(RETENTION), required: false }]
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let whole = hour.fresh || self.rebuild;
        let mut pending = Pending::default();
        let lost = hour.fresh && !hour.first && self.last.is_some_and(|last| last.saturating_add(1) < hour.hour);
        if whole {
            self.read_run(out);
        }
        let lost = lost && self.run.counted_from.is_some() && self.run.withheld.is_none();
        if lost && let Some(last) = self.last {
            earliest(&mut self.run.incomplete_from, last + 1);
        }
        let identities = whole || hour.wrote(TableId::Countries, Columns::These(&COUNTRY_SLOT)) || hour.wrote(TableId::CountryLinks, Columns::These(&LINK_SLOT));
        let mut recount = if hour.fresh { Recount::All } else { Recount::None };
        if identities {
            let (aliases, ids) = (std::mem::take(&mut self.aliases), std::mem::take(&mut self.identities));
            self.read_identities(hour);
            if !hour.fresh {
                recount = Recount::Slots(self.moved_slots(&aliases, &ids));
            }
            self.identities_changed(&mut pending, out);
        }
        let turned = month_bucket(hour.hour) != month_bucket(hour.hour.saturating_sub(1));
        self.combat(hour, &recount, (whole, turned), &mut pending, out)?;
        let (mut drain_state, mut drain) = self.destruction(hour, out, &mut pending)?;
        if lost {
            drain_state |= Coverage::DRAIN_STATE_LOST;
            if drain.is_empty() {
                drain = LOST.to_owned();
            } else {
                drain = format!("{LOST}; {drain}");
            }
        }
        self.ships(hour, &recount, out)?;
        self.convoys(hour, whole, &mut pending)?;
        self.lend(hour, whole, &mut pending)?;
        self.land(hour, identities, &mut pending);
        self.write(hour, pending, &drain, drain_state, out);
        self.rebuild = false;
        self.last = Some(hour.hour);
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        self.read_identities(hour);
        self.listed = match hour.complete(TableId::SunkShips) {
            true => self.sunk(hour).ok().map(|listed| listed.unwrap_or_default().into_iter().map(|ship| (ship.slot, ship.signature, ship.count)).collect()),
            false => None,
        };
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        let hour = |blob: &mut Bytes, hour: Option<i32>| blob.varint(hour.map_or(0, |hour| u64::from(hour as u32) + 1));
        hour(blob, self.last)?;
        hour(blob, self.lend_read)?;
        blob.u8(u8::from(self.convoys_seen) | u8::from(self.lend_seen) << 1)?;
        let mut convoys: Vec<(u128, i32)> = self.convoys.iter().map(|(&key, &count)| (key, count)).collect();
        convoys.sort_unstable();
        blob.varint(convoys.len() as u64)?;
        for (key, count) in convoys {
            blob.raw(&key.to_le_bytes())?;
            blob.varint(u64::from(count as u32))?;
        }
        let mut lend: Vec<(u128, i64)> = self.lend.iter().map(|(&key, &amount)| (key, amount)).collect();
        lend.sort_unstable();
        blob.varint(lend.len() as u64)?;
        for (key, amount) in lend {
            blob.raw(&key.to_le_bytes())?;
            blob.varint(amount as u64)?;
        }
        for set in [&self.unresolved, &self.unresolved_combat] {
            let mut raws: Vec<u32> = set.iter().copied().collect();
            raws.sort_unstable();
            blob.varint(raws.len() as u64)?;
            for raw in raws {
                blob.varint(u64::from(raw))?;
            }
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        let hour = |reader: &mut Reader<'_>| -> io::Result<Option<i32>> {
            Ok(match reader.varint()? {
                0 => None,
                held => Some(u32::try_from(held - 1).map_err(|_| invalid("EquipmentLosses' state names an hour past 32 bits"))? as i32),
            })
        };
        self.last = hour(&mut reader)?;
        self.lend_read = hour(&mut reader)?;
        let seen = reader.u8()?;
        (self.convoys_seen, self.lend_seen) = (seen & 1 != 0, seen & 2 != 0);
        let key = |reader: &mut Reader<'_>| -> io::Result<u128> { Ok(u128::from_le_bytes(reader.raw(16)?.try_into().map_err(|_| invalid("A short key"))?)) };
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("EquipmentLosses' state counts more convoy rows than it holds"));
        }
        for _ in 0..count {
            let held = key(&mut reader)?;
            let convoys = u32::try_from(reader.varint()?).map_err(|_| invalid("A convoy count past 32 bits"))? as i32;
            self.convoys.insert(held, convoys);
        }
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("EquipmentLosses' state counts more lend-lease pools than it holds"));
        }
        for _ in 0..count {
            let held = key(&mut reader)?;
            let amount = reader.varint()? as i64;
            self.lend.insert(held, amount);
        }
        for set in [&mut self.unresolved, &mut self.unresolved_combat] {
            let count = reader.count()?;
            if count > blob.len() {
                return Err(invalid("EquipmentLosses' state counts more raw countries than it holds"));
            }
            for _ in 0..count {
                set.insert(reader.varint_u32()?);
            }
        }
        reader.finish()?;
        self.rebuild = true;
        Ok(())
    }
}
