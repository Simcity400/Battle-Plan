
use recorder_derive_framework::{
    forces::{invalid, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{
    leaf::Writer as Bytes,
    store::{TableView, Value as Held},
};
use replay_data::rows::{AirRegionDayRow, CountryLinkRow, GroupRow, GroupSpanRow, RaidAirScoreRow, TableId, TagHolderRow};
use replay_derive::air_superiority::{
    assignment::{self, AssignedSide},
    compose::{Compose, Operands, Pair, Roster, View},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::Arc,
};

pub const DAY: i32 = replay_data::cadence::STAGGER_PERIOD;

const SCORES: [usize; 2] = columns::<RaidAirScoreRow, 2>(["friendly", "enemy"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const TAG: [usize; 1] = columns::<TagHolderRow, 1>(["tag"]);
const ORDINAL: [usize; 1] = columns::<GroupSpanRow, 1>(["ordinal"]);
const FACTION: [usize; 1] = columns::<GroupRow, 1>(["faction"]);
const OWN: [usize; 3] = columns::<AirRegionDayRow, 3>(["day", "activity", "activity_total"]);

const CONTEXT: [TableId; 4] = [TableId::CountryLinks, TableId::TagHolders, TableId::GroupSpans, TableId::Groups];

const READS: &[Read] = &[
    Read { table: TableId::RaidAirScores, columns: Columns::These(&SCORES), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK), watch: Watch::Changed, required: true },
    Read { table: TableId::TagHolders, columns: Columns::These(&TAG), watch: Watch::Changed, required: true },
    Read { table: TableId::GroupSpans, columns: Columns::These(&ORDINAL), watch: Watch::Changed, required: true },
    Read { table: TableId::Groups, columns: Columns::These(&FACTION), watch: Watch::Changed, required: true },
];

#[derive(Default)]
struct Context {
    present: Arc<BTreeSet<String>>,
    owners: HashMap<u32, String>,
    sides: Option<Vec<AssignedSide>>,
    identity: bool,
}

fn text(hour: &Hour<'_>, view: &TableView<'_>, column: usize, slot: u32) -> Option<String> {
    match view.value(column, slot) {
        Held::Text(id) => hour.text(id).map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

fn context(hour: &Hour<'_>) -> Context {
    let holders = hour.table(TableId::TagHolders);
    let mut identity = true;
    let mut tags_of: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    for slot in holders.rows() {
        let Some(key) = holders.key(slot) else { continue };
        let (number, held) = ((key >> 96) as u32, (key >> 32) as u32);
        if number == TagHolderRow::UNKNOWN {
            identity = false;
            continue;
        }
        if held == TagHolderRow::NONE {
            continue;
        }
        if let Some(tag) = text(hour, &holders, TAG[0], slot) {
            tags_of.entry(held).or_default().insert(tag);
        }
    }
    let present: BTreeSet<String> = tags_of.values().flatten().cloned().collect();
    let slot_tag: HashMap<u32, &String> = tags_of.iter().filter_map(|(slot, tags)| Some((*slot, tags.last()?))).collect();
    let links = hour.table(TableId::CountryLinks);
    let owners = links
        .rows()
        .filter_map(|slot| Some((links.key(slot)? as u32, slot_tag.get(&(links.number(LINK[0], slot)? as u32))?.to_string())))
        .collect();
    let factions_of = hour.table(TableId::Groups);
    let factions: HashMap<u32, u64> = factions_of.rows().filter_map(|slot| Some((factions_of.key(slot)? as u32, factions_of.number(FACTION[0], slot)? as u64))).collect();
    let spans = hour.table(TableId::GroupSpans);
    let mut unknown = false;
    let mut members: BTreeMap<u64, BTreeSet<String>> = BTreeMap::new();
    let mut leaders: BTreeMap<u64, (usize, String)> = BTreeMap::new();
    let mut wars = Vec::new();
    for slot in spans.rows() {
        let Some(key) = spans.key(slot) else { continue };
        let (kind, group, value) = ((key >> 96) as u8, (key >> 64) as u32, (key >> 32) as u32);
        let tags = |held: u32| tags_of.get(&held).into_iter().flatten();
        match kind {
            GroupSpanRow::INCOMPLETE => unknown = true,
            GroupSpanRow::UNOBSERVED if group != GroupSpanRow::HUMANS => unknown = true,
            GroupSpanRow::MEMBER | GroupSpanRow::LEADER if group != GroupSpanRow::HUMANS => {
                let Some(&faction) = factions.get(&group) else { continue };
                if kind == GroupSpanRow::MEMBER {
                    members.entry(faction).or_default().extend(tags(value).cloned());
                } else {
                    let order = spans.number(ORDINAL[0], slot).unwrap_or_default() as usize;
                    for tag in tags(value) {
                        if leaders.get(&faction).is_none_or(|(held, _)| order < *held) {
                            leaders.insert(faction, (order, tag.clone()));
                        }
                    }
                }
            }
            GroupSpanRow::WAR => {
                for first in tags(group) {
                    for second in tags(value) {
                        wars.push(if first <= second { (first.clone(), second.clone()) } else { (second.clone(), first.clone()) });
                    }
                }
            }
            _ => {}
        }
    }
    for held in members.values_mut() {
        held.retain(|tag| present.contains(tag));
    }
    let leaders: BTreeMap<u64, (usize, &str)> = leaders.iter().map(|(id, (order, tag))| (*id, (*order, tag.as_str()))).collect();
    let sides = if unknown { None } else { assignment::roster(members, &leaders, wars, &present) };
    Context { present: Arc::new(present), owners, sides, identity }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    pub day: i32,
    pub activity: i64,
    pub total: i64,
}

impl Activity {
    pub fn before(&self, day: i32) -> i128 {
        i128::from(self.total) + i128::from(self.activity) * i128::from(day.saturating_sub(self.day).max(0))
    }

    pub fn through(&self, day: i32) -> i128 {
        self.before(day) + i128::from(self.activity)
    }

    pub fn read(view: &TableView<'_>, slot: u32) -> Option<Self> {
        Some(Self { day: i32::try_from(view.number(OWN[0], slot)?).ok()?, activity: view.number(OWN[1], slot)?, total: view.number(OWN[2], slot)? })
    }
}

#[derive(Default)]
pub struct AirRegionDays {
    rebuild: bool,
    context: Context,
    scores: HashMap<u128, (i64, i64)>,
    regions: HashMap<u32, BTreeSet<u128>>,
    rows: HashMap<String, (u32, u32)>,
    unresolved: u32,
    composed: Option<(HashSet<String>, HashSet<String>, bool)>,
}

impl AirRegionDays {
    fn count(&mut self, key: u128, (friendly, enemy): (i64, i64), sign: i32) {
        match self.context.owners.get(&((key >> 32) as u32)) {
            Some(tag) => {
                let held = self.rows.entry(tag.clone()).or_default();
                held.0 = held.0.wrapping_add_signed(sign);
                if friendly < 0 || enemy < 0 {
                    held.1 = held.1.wrapping_add_signed(sign);
                }
                if *held == (0, 0) {
                    self.rows.remove(tag);
                }
            }
            None => self.unresolved = self.unresolved.wrapping_add_signed(sign),
        }
    }

    fn set(&mut self, key: u128, score: Option<(i64, i64)>, touched: &mut BTreeSet<u32>) {
        let region = key as u32;
        let before = match score {
            Some(score) => self.scores.insert(key, score),
            None => self.scores.remove(&key),
        };
        if before == score {
            return;
        }
        if let Some(before) = before {
            self.count(key, before, -1);
        }
        match score {
            Some(score) => {
                self.count(key, score, 1);
                self.regions.entry(region).or_default().insert(key);
            }
            None => {
                if let Some(keys) = self.regions.get_mut(&region) {
                    keys.remove(&key);
                    if keys.is_empty() {
                        self.regions.remove(&region);
                    }
                }
            }
        }
        touched.insert(region);
    }

    fn recount(&mut self) {
        self.rows.clear();
        self.unresolved = 0;
        let scores: Vec<(u128, (i64, i64))> = self.scores.iter().map(|(key, score)| (*key, *score)).collect();
        for (key, score) in scores {
            self.count(key, score, 1);
        }
    }

    fn operands(&self, complete: bool) -> Operands {
        let present = &self.context.present;
        let witnessed: HashSet<String> = self.rows.keys().filter(|tag| present.contains(*tag)).cloned().collect();
        let invalid = self.rows.iter().filter(|(tag, (_, negative))| *negative > 0 && present.contains(*tag)).map(|(tag, _)| tag.clone()).collect();
        Operands { identity_known: self.context.identity, present: Arc::clone(present), witnessed, invalid, complete, unresolved: self.unresolved > 0 }
    }

    fn pairs(&self, region: u32, operands: &Operands) -> Vec<Pair> {
        let mut supplied: BTreeMap<&str, (u32, i64, i64)> = BTreeMap::new();
        for key in self.regions.get(&region).into_iter().flatten() {
            let raw = (key >> 32) as u32;
            let Some(tag) = self.context.owners.get(&raw).filter(|tag| self.context.present.contains(*tag)) else { continue };
            if operands.invalid.contains(tag) {
                continue;
            }
            let (friendly, enemy) = self.scores[key];
            supplied.entry(tag.as_str()).and_modify(|held| if raw > held.0 { *held = (raw, friendly, enemy) }).or_insert((raw, friendly, enemy));
        }
        supplied.into_iter().map(|(tag, (raw, friendly, enemy))| Pair { tag: tag.to_owned(), raw, friendly, enemy }).collect()
    }
}

fn row(view: &TableView<'_>, slot: u32) -> io::Result<(u128, (i64, i64))> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("RaidAirScores slot {slot} holds no row")))?;
    let score = |column: usize| view.number(SCORES[column], slot).ok_or_else(|| invalid("RaidAirScores' scores are not recorded and the row cannot say so"));
    Ok((key, (score(0)?, score(1)?)))
}

impl Writer for AirRegionDays {
    fn name(&self) -> &'static str {
        "AirRegionDays"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::AirRegionDays)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let table = TableId::AirRegionDays;
        let whole = hour.fresh || self.rebuild;
        let mut touched = BTreeSet::new();
        let view = hour.table(TableId::RaidAirScores);
        let moved = whole || CONTEXT.iter().any(|table| hour.changed(*table));
        if moved {
            self.context = context(hour);
        }
        if whole {
            self.rebuild = false;
            self.scores.clear();
            self.regions.clear();
            for slot in view.rows() {
                let (key, score) = row(&view, slot)?;
                self.scores.insert(key, score);
                self.regions.entry(key as u32).or_default().insert(key);
            }
            self.recount();
        } else {
            if moved {
                self.recount();
            }
            for key in hour.deleted_keys(TableId::RaidAirScores) {
                self.set(key, None, &mut touched);
            }
            for slot in written(hour, TableId::RaidAirScores, &SCORES) {
                let (key, score) = row(&view, slot)?;
                self.set(key, Some(score), &mut touched);
            }
        }
        let operands = self.operands(hour.complete(TableId::RaidAirScores));
        let now = (operands.witnessed.clone(), operands.invalid.clone(), operands.unresolved);
        if moved || self.composed.as_ref() != Some(&now) {
            touched.extend(self.regions.keys().copied());
            touched.extend(recorder_derive_framework::forces::held(out, table).into_iter().map(|key| key as u32));
            self.composed = Some(now);
        }
        let roster = self.context.sides.as_deref().map(Roster::new);
        let compose = Compose::new(roster.as_ref(), &operands, View::Sides);
        let day = hour.hour.div_euclid(DAY);
        for region in touched {
            let pairs = self.pairs(region, &operands);
            let activity = i64::try_from(compose.activity(&pairs)).map_err(|_| invalid("A region's air activity passes a 64-bit value"))?;
            let held = {
                let own = out.table(table);
                own.slot(u128::from(region)).and_then(|slot| Activity::read(&own, slot))
            };
            let next = match held {
                Some(held) if held.activity == activity => continue,
                Some(held) => Activity { day, activity, total: i64::try_from(held.before(day)).map_err(|_| invalid("A region's air activity passes a 64-bit total"))? },
                None if activity == 0 => continue,
                None => Activity { day, activity, total: 0 },
            };
            out.set(table, u128::from(region), vec![Value::I32(next.day), Value::I64(next.activity), Value::I64(next.total)]);
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("AirRegionDays' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
