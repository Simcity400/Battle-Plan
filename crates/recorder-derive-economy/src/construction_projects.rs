
use recorder_derive_framework::{
    deployed::{invalid, number, row},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use engine_native_parts::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
use engine_native::cprovince::buildings::PROVINCE_FLAG;
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    store::{Kept, TableView},
};
use replay_data::{
    rows::{
        BuildingConstructionRow, BuildingRow, ConstructionLineEndRow, ConstructionLineOutputRow, ConstructionProjectRow, ConstructionRise, ProvinceRow,
        RailwayConstructionRow, Recorded, TableId,
    },
    value::EncodeValue,
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

#[cfg(test)]
mod tests;

type P = ConstructionProjectRow;

type Key = (bool, u32, u32);
type Levels = (i16, i16);
type Change = (i32, Levels, Levels);

const LEVELS: [usize; 2] = columns::<BuildingRow, 2>(["level", "healthy_level"]);
const PROVINCE_STATE: [usize; 1] = columns::<ProvinceRow, 1>(["state"]);
const BUILDING_LINE: [usize; 15] = columns::<BuildingConstructionRow, 15>([
    "country",
    "queue_ordinal",
    "active_factories",
    "cost",
    "speed",
    "produced",
    "priority",
    "amount",
    "created_hour",
    "building_type",
    "target_flags",
    "target_location",
    "target_state",
    "target_country",
    "repair_levels",
]);
const RAILWAY_LINE: [usize; 15] = columns::<RailwayConstructionRow, 15>([
    "country",
    "queue_ordinal",
    "active_factories",
    "cost",
    "speed",
    "produced",
    "priority",
    "amount",
    "created_hour",
    "base_level",
    "target_level",
    "cancelled",
    "current_segment",
    "remaining_cost",
    "route",
]);
const OUTPUT: [usize; 23] = columns::<ConstructionLineOutputRow, 23>([
    "railway",
    "construction_raw",
    "repair_raw",
    "bank_raw",
    "instant_raw",
    "carry_raw",
    "ticks",
    "since_hour",
    "observed_since_creation",
    "linked",
    "ended",
    "exact",
    "repair_ticks",
    "completion",
    "count_reset",
    "country",
    "building_type",
    "target_flags",
    "target_location",
    "target_state",
    "target_country",
    "route_first",
    "route_last",
]);

const READS: &[Read] = &[
    Read { table: TableId::BuildingConstruction, columns: Columns::These(&BUILDING_LINE), watch: Watch::EveryRead, required: false },
    Read { table: TableId::RailwayConstruction, columns: Columns::These(&RAILWAY_LINE), watch: Watch::EveryRead, required: false },
    Read { table: TableId::StateBuildings, columns: Columns::These(&LEVELS), watch: Watch::Dropped, required: false },
    Read { table: TableId::ProvinceBuildings, columns: Columns::These(&LEVELS), watch: Watch::Dropped, required: false },
    Read { table: TableId::ConstructionLineOutput, columns: Columns::These(&OUTPUT), watch: Watch::Changed, required: false },
    Read { table: TableId::Provinces, columns: Columns::These(&[]), watch: Watch::Changed, required: false },
];

const BUILDINGS: [(TableId, bool); 2] = [(TableId::StateBuildings, false), (TableId::ProvinceBuildings, true)];

fn railway_key(province: u32) -> Key {
    (true, province, RAILWAY_TEMPLATE_TOKEN)
}

fn daily(hour: i32) -> bool {
    hour.rem_euclid(24) == 0
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Family {
    changes: HashMap<Key, Vec<Change>>,
    lost: bool,
    read: Option<i32>,
}

impl Family {
    fn earlier_unknown(&self, keys: &[Key], created: i32) -> bool {
        if self.read.is_none_or(|read| created <= read) || keys.is_empty() || self.lost {
            return true;
        }
        keys.iter().any(|key| {
            let changes = self.changes.get(key).map_or(&[][..], Vec::as_slice);
            let first = changes.partition_point(|(hour, _, _)| *hour < created);
            let stretch = Stretch::of(&changes[first..], None);
            !stretch.repairs.is_empty() || !stretch.levels.is_empty()
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rise {
    hour: i32,
    count: u32,
}

#[derive(Debug, Default)]
struct Stretch {
    repairs: Vec<Rise>,
    levels: Vec<Rise>,
    damage: u32,
    doubtful: bool,
}

impl Stretch {
    fn of(changes: &[Change], newcomer: Option<i32>) -> Self {
        let mut stretch = Self::default();
        for &(hour, was, now) in changes {
            let (level, healthy) = (i32::from(was.0), i32::from(was.1));
            let (now_level, now_healthy) = (i32::from(now.0), i32::from(now.1));
            let repaired = daily(hour) && now_level == level && now_healthy > healthy;
            let built = daily(hour) && now_level > level && healthy >= level;
            if newcomer.is_some_and(|since| hour >= since) {
                stretch.doubtful |= repaired || built;
                continue;
            }
            if repaired {
                stretch.repairs.push(Rise { hour, count: (now_healthy - healthy) as u32 });
            }
            if built {
                stretch.levels.push(Rise { hour, count: (now_level - level) as u32 });
            }
            let more = (now_level - now_healthy) - (level - healthy);
            if more > 0 {
                stretch.damage += more as u32;
            }
        }
        stretch
    }
}

enum Seen<'a> {
    Lost,
    Shared,
    Clear(&'a [Change], Option<i32>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Open {
    keys: Vec<Key>,
    remaining: i32,
    repair_levels: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ended {
    completion: Option<Option<(i32, bool)>>,
    linked: bool,
    since: i32,
    railway: bool,
}

#[derive(Clone, Debug)]
struct Line {
    row: P,
    open: Option<Open>,
    run: Option<i32>,
    since: Option<i32>,
    restarted: bool,
    ended: Option<Ended>,
}

#[derive(Default)]
pub struct ConstructionProjects {
    families: [Family; 2],
    lines: HashMap<u64, Line>,
    closing: Vec<u64>,
    dirty: HashSet<u64>,
    reread: bool,
    saved: HashMap<u64, Line>,
}

fn add(count: &mut i64, by: i64) {
    if by > 0 {
        *count += by;
    }
}

fn recorded<T: Copy>(cell: &Recorded<Option<T>>) -> Option<T> {
    match cell {
        Recorded::Value(value) => *value,
        Recorded::NotRecorded => None,
    }
}

fn rises(of: &[Rise]) -> Vec<ConstructionRise> {
    of.iter().map(|rise| ConstructionRise { hour: rise.hour, count: rise.count }).collect()
}

fn end_cells(end: &ConstructionLineEndRow) -> Vec<Value> {
    vec![
        Value::U8(end.kind),
        Value::Bool(end.output_alone),
        Value::U32(end.country),
        Value::OptionalU32(end.building_type),
        Value::OptionalU32(end.target_state),
        Value::OptionalU32(end.to_state),
        Value::I32(end.first_read),
        Value::Bool(end.new_work),
        Value::Bool(end.repair_work),
        Value::Bool(end.before_observation),
        Value::Bool(end.had_output),
        Value::U8(end.output_reason),
    ]
}

fn cells(row: &P, out: &mut Rows) -> io::Result<Vec<Value>> {
    let mut list = |rises: &Vec<ConstructionRise>| -> io::Result<Value> {
        let mut bytes = Vec::new();
        rises.encode_value(&mut bytes)?;
        Ok(out.opaque(bytes))
    };
    let (built_rises, repaired_rises) = (list(&row.built_rises)?, list(&row.repaired_rises)?);
    let number = |cell: &Recorded<i32>| match cell {
        Recorded::Value(value) => Value::I32(*value),
        Recorded::NotRecorded => Value::Unrecorded,
    };
    let wide = |cell: &Recorded<i64>| match cell {
        Recorded::Value(value) => Value::I64(*value),
        Recorded::NotRecorded => Value::Unrecorded,
    };
    Ok(vec![
        Value::U8(row.kind),
        Value::Bool(row.output_alone),
        Value::U32(row.country),
        Value::OptionalU32(row.building_type),
        Value::OptionalU32(row.target_state),
        Value::OptionalU32(row.to_state),
        Value::OptionalU32(row.province),
        Value::OptionalU32(row.route_provinces),
        Value::OptionalI32(row.base_level),
        Value::OptionalI32(row.target_level),
        Value::OptionalI64(row.cost),
        Value::OptionalI32(row.created_hour),
        Value::I32(row.first_read),
        Value::OptionalI32(row.census_read),
        Value::OptionalU32(row.queue_ordinal),
        Value::Bool(row.earlier_unknown),
        number(&row.factories),
        number(&row.remaining),
        number(&row.repair_levels),
        Value::I64(row.built),
        built_rises,
        Value::I64(row.repaired),
        repaired_rises,
        Value::I64(row.added_new),
        Value::I64(row.added_repair),
        Value::I64(row.possible),
        Value::I64(row.unaccounted_new),
        Value::I64(row.unaccounted_repair),
        Value::U8(row.end),
        Value::U8(row.end_reason),
        Value::OptionalI32(row.end_hour),
        Value::OptionalI32(row.end_known_at),
        wide(&row.construction_raw),
        wide(&row.repair_raw),
        Value::OptionalI32(row.output_since),
        Value::Bool(row.observed_since_creation),
        Value::U8(row.output_reason),
        Value::OptionalI32(row.first_repair_tick),
    ])
}

fn settle(row: &mut P, open: &Open, now: Option<&BuildingConstructionRow>, seen: Seen<'_>) -> Option<(u8, u8)> {
    let (repair_left, new_left) = (open.repair_levels.max(0), open.remaining.max(0));
    let (repair_now, new_now) = now.map_or((0, 0), |now| (now.repair_levels.max(0), now.amount.max(0)));
    row.built_rises.clear();
    row.repaired_rises.clear();
    let stretch = match seen {
        Seen::Clear(changes, newcomer) => Stretch::of(changes, newcomer),
        Seen::Lost | Seen::Shared => {
            if now.is_some() {
                add(&mut row.unaccounted_repair, i64::from(repair_left - repair_now));
                add(&mut row.unaccounted_new, i64::from(new_left - new_now));
                return None;
            }
            let why = if repair_left == 0 && new_left == 0 {
                P::NOTHING_LEFT
            } else if matches!(seen, Seen::Lost) {
                P::NOT_RECORDED
            } else {
                P::SHARED_TARGET
            };
            if why != P::NOTHING_LEFT {
                row.unaccounted_repair += i64::from(repair_left);
                row.unaccounted_new += i64::from(new_left);
            }
            return Some((P::UNKNOWN_END, why));
        }
    };
    if now.is_none() {
        if repair_left == 0 && new_left == 0 {
            return Some((P::UNKNOWN_END, P::NOTHING_LEFT));
        }
        row.unaccounted_repair += i64::from(repair_left);
        row.unaccounted_new += i64::from(new_left);
        return Some((P::UNKNOWN_END, P::COMPLETION_NOT_RECORDED));
    }
    let candidates: u32 = stretch.levels.iter().map(|rise| rise.count).sum();
    let fell = u32::try_from(new_left - new_now).unwrap_or(0);
    let completions = fell.min(candidates);
    if completions > 0 {
        row.built += i64::from(completions);
        if candidates == completions {
            row.built_rises = rises(&stretch.levels);
        }
    }
    let net = i64::from(new_now) - i64::from(new_left) + i64::from(completions);
    add(&mut row.added_new, net);
    if stretch.doubtful {
        add(&mut row.unaccounted_new, -net);
    } else {
        add(&mut row.possible, i64::from(candidates - completions));
    }
    let repaired: u32 = stretch.repairs.iter().map(|rise| rise.count).sum();
    row.repaired += i64::from(repaired);
    row.repaired_rises = rises(&stretch.repairs);
    let added = i64::from(stretch.damage).max(i64::from(repair_now) - i64::from(repair_left) + i64::from(repaired));
    add(&mut row.added_repair, added);
    if stretch.doubtful {
        add(&mut row.unaccounted_repair, i64::from(repair_left) + added.max(0) - i64::from(repaired) - i64::from(repair_now));
    }
    None
}

impl ConstructionProjects {
    fn state_of(hour: &Hour<'_>, province: Option<u32>) -> Option<u32> {
        let view = hour.table(TableId::Provinces);
        view.slot(u128::from(province?)).and_then(|slot| number(&view, PROVINCE_STATE[0], slot)).map(|state| state as u32)
    }

    fn buildings(&mut self, hour: &Hour<'_>) {
        let whole = hour.fresh || BUILDINGS.iter().any(|&(table, _)| !hour.complete(table) || hour.changes(table).status_changed);
        if whole {
            self.families.iter_mut().for_each(|family| family.lost = true);
            return;
        }
        for (table, province) in BUILDINGS {
            let view = hour.table(table);
            let changes = hour.changes(table);
            let mut before: HashMap<u32, [Option<i16>; 2]> = HashMap::new();
            for (slot, column, kept) in hour.previous(table) {
                let Some(at) = LEVELS.iter().position(|&level| level == usize::from(*column)) else { continue };
                let entry = before.entry(*slot).or_default();
                if entry[at].is_none() {
                    entry[at] = Some(match kept {
                        Kept::Number(bits) => *bits as i16,
                        _ => 0,
                    });
                }
            }
            let mut found: Vec<(Key, Levels, Levels)> = Vec::new();
            for slot in hour.written_rows(table, Columns::These(&LEVELS)) {
                let Some(key) = view.key(slot) else { continue };
                let now = (view.number(LEVELS[0], slot).unwrap_or(0) as i16, view.number(LEVELS[1], slot).unwrap_or(0) as i16);
                let was = if changes.inserted.contains(&slot) {
                    (0, 0)
                } else {
                    let held = before.get(&slot).copied().unwrap_or_default();
                    (held[0].unwrap_or(now.0), held[1].unwrap_or(now.1))
                };
                found.push(((province, (key >> 32) as u32, key as u32), was, now));
            }
            for (key, kept) in hour.dropped(table) {
                let level = |at: usize| match kept.get(LEVELS[at]) {
                    Some(Kept::Number(bits)) => *bits as i16,
                    _ => 0,
                };
                found.push(((province, (*key >> 32) as u32, *key as u32), (level(0), level(1)), (0, 0)));
            }
            for (key, was, now) in found {
                if was == now {
                    continue;
                }
                self.families[0].changes.entry(key).or_default().push((hour.hour, was, now));
                if key == railway_key(key.1) {
                    self.families[1].changes.entry(key).or_default().push((hour.hour, was, now));
                }
            }
        }
    }

    fn depart(&mut self, id: u64, end: (u8, u8), hour: i32) {
        let Some(line) = self.lines.get_mut(&id) else { return };
        line.open = None;
        (line.row.end, line.row.end_reason, line.row.end_hour, line.row.end_known_at) = (end.0, end.1, None, Some(hour));
        self.dirty.insert(id);
        if line.ended.is_some() {
            self.conclude(id, hour);
        }
    }

    fn conclude(&mut self, id: u64, hour: i32) {
        let Some(line) = self.lines.get_mut(&id) else { return };
        if let Some(ended) = line.ended.filter(|ended| ended.railway == (line.row.kind == P::RAILWAY)) {
            let covered = !ended.linked
                && !line.restarted
                && (line.run.is_some_and(|run| ended.since <= run) || (line.row.output_alone && ended.since <= line.row.first_read));
            let (end, reason, at) = match ended.completion {
                Some(Some((outcome, cancelled))) if outcome <= hour => (if cancelled { P::REMOVED } else { P::FINISHED }, 0, Some(outcome)),
                Some(None) if covered => (P::REMOVED, 0, None),
                _ => (P::UNKNOWN_END, P::COMPLETION_NOT_RECORDED, None),
            };
            (line.row.end, line.row.end_reason, line.row.end_hour, line.row.end_known_at) = (end, reason, at, Some(hour));
        }
        self.dirty.insert(id);
        self.closing.push(id);
    }

    fn first(&mut self, id: u64, mut row: P, hour: i32) {
        if let Some(previous) = self.lines.get(&id).filter(|line| line.row.output_alone) {
            row.first_read = previous.row.first_read;
            row.first_repair_tick = previous.row.first_repair_tick;
            (row.construction_raw, row.repair_raw, row.output_since, row.output_reason, row.observed_since_creation) = (
                previous.row.construction_raw,
                previous.row.repair_raw,
                previous.row.output_since,
                previous.row.output_reason,
                previous.row.observed_since_creation,
            );
        }
        let held = self.lines.remove(&id);
        let (since, restarted, ended) = held.map_or((None, false, None), |line| (line.since, line.restarted, line.ended));
        let open = Open { keys: Vec::new(), remaining: 0, repair_levels: 0 };
        self.lines.insert(id, Line { row, open: Some(open), run: Some(hour), since, restarted, ended });
        self.dirty.insert(id);
    }

    fn run(line: &mut Line, factories: i32, remaining: i32, repair_levels: i32, hour: i32) {
        let same = line.row.factories == Recorded::Value(factories) && line.row.remaining == Recorded::Value(remaining) && line.row.repair_levels == Recorded::Value(repair_levels);
        if !same {
            line.run = Some(hour);
        }
        (line.row.factories, line.row.remaining, line.row.repair_levels) = (Recorded::Value(factories), Recorded::Value(remaining), Recorded::Value(repair_levels));
        if let Some(open) = &mut line.open {
            (open.remaining, open.repair_levels) = (remaining, repair_levels);
        }
    }

    fn read_buildings(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        let view = hour.table(TableId::BuildingConstruction);
        let mut present: HashMap<u64, BuildingConstructionRow> = HashMap::new();
        for slot in view.rows() {
            let line: BuildingConstructionRow = row(&view, slot, &BUILDING_LINE)?;
            present.insert(line.id, line);
        }
        let key_of = |line: &BuildingConstructionRow| (line.target_flags & PROVINCE_FLAG != 0, line.target_location, line.building_type);
        let open = |line: &Line| line.row.kind == P::BUILDING && line.open.is_some();
        let mut sharing: HashMap<Key, usize> = HashMap::new();
        for line in self.lines.values().filter(|line| open(line)) {
            *sharing.entry(line.open.as_ref().expect("open").keys[0]).or_default() += 1;
        }
        let mut newcomers: HashMap<Key, i32> = HashMap::new();
        for line in present.values().filter(|line| !self.lines.get(&line.id).is_some_and(open)) {
            newcomers.entry(key_of(line)).and_modify(|since| *since = (*since).min(line.created_hour)).or_insert(line.created_hour);
        }
        let ids: Vec<u64> = self.lines.iter().filter(|(_, line)| open(line)).map(|(&id, _)| id).collect();
        let family = std::mem::take(&mut self.families[0]);
        for id in ids {
            let line = self.lines.get_mut(&id).expect("open line");
            let held = line.open.clone().expect("open");
            let key = held.keys[0];
            let seen = if family.lost {
                Seen::Lost
            } else if sharing.get(&key).copied().unwrap_or(0) > 1 {
                Seen::Shared
            } else {
                Seen::Clear(family.changes.get(&key).map_or(&[][..], Vec::as_slice), newcomers.get(&key).copied())
            };
            let now = present.get(&id);
            let end = settle(&mut line.row, &held, now, seen);
            self.dirty.insert(id);
            match now {
                Some(now) => Self::run(line, now.active_factories, now.amount, now.repair_levels, hour.hour),
                None => self.depart(id, end.unwrap_or((P::UNKNOWN_END, P::COMPLETION_NOT_RECORDED)), hour.hour),
            }
        }
        let mut fresh: Vec<&BuildingConstructionRow> = present.values().filter(|line| !self.lines.get(&line.id).is_some_and(open)).collect();
        fresh.sort_unstable_by_key(|line| line.id);
        for line in fresh {
            let key = key_of(line);
            let row = P {
                line: line.id,
                kind: P::BUILDING,
                country: line.country,
                building_type: Some(line.building_type),
                target_state: (line.target_state != 0).then_some(line.target_state),
                province: key.0.then_some(line.target_location),
                cost: Some(line.cost),
                created_hour: Some(line.created_hour),
                first_read: hour.hour,
                census_read: Some(hour.hour),
                queue_ordinal: Some(line.queue_ordinal),
                earlier_unknown: family.earlier_unknown(&[key], line.created_hour),
                construction_raw: Recorded::NotRecorded,
                repair_raw: Recorded::NotRecorded,
                output_reason: P::OUTPUT_NOT_RECORDED,
                ..Default::default()
            };
            self.first(line.id, row, hour.hour);
            let held = self.lines.get_mut(&line.id).expect("first");
            held.open = Some(Open { keys: vec![key], remaining: line.amount, repair_levels: line.repair_levels });
            Self::run(held, line.active_factories, line.amount, line.repair_levels, hour.hour);
        }
        self.families[0] = Family { changes: HashMap::new(), lost: !BUILDINGS.iter().all(|&(table, _)| hour.complete(table)), read: Some(hour.hour) };
        Ok(())
    }

    fn read_railways(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        let view = hour.table(TableId::RailwayConstruction);
        let mut present: HashMap<u64, RailwayConstructionRow> = HashMap::new();
        for slot in view.rows() {
            let line: RailwayConstructionRow = row(&view, slot, &RAILWAY_LINE)?;
            present.insert(line.id, line);
        }
        let open = |line: &Line| line.row.kind == P::RAILWAY && line.open.is_some();
        let ids: Vec<u64> = self.lines.iter().filter(|(_, line)| open(line)).map(|(&id, _)| id).collect();
        for id in ids {
            let line = self.lines.get_mut(&id).expect("open line");
            let remaining = line.open.as_ref().expect("open").remaining;
            self.dirty.insert(id);
            match present.get(&id) {
                Some(now) => {
                    add(&mut line.row.built, i64::from(remaining.saturating_sub(now.amount)));
                    line.row.built_rises.clear();
                    Self::run(line, now.active_factories, now.amount, 0, hour.hour);
                }
                None => {
                    line.row.built_rises.clear();
                    line.row.unaccounted_new += i64::from(remaining.max(0));
                    self.depart(id, (P::UNKNOWN_END, P::COMPLETION_NOT_RECORDED), hour.hour);
                }
            }
        }
        let family = std::mem::take(&mut self.families[1]);
        let mut fresh: Vec<&RailwayConstructionRow> = present.values().filter(|line| !self.lines.get(&line.id).is_some_and(open)).collect();
        fresh.sort_unstable_by_key(|line| line.id);
        for line in fresh {
            let keys: Vec<Key> = line.route.iter().map(|&province| railway_key(province)).collect();
            let row = P {
                line: line.id,
                kind: P::RAILWAY,
                country: line.country,
                building_type: Some(RAILWAY_TEMPLATE_TOKEN),
                target_state: Self::state_of(hour, line.route.first().copied()),
                to_state: Self::state_of(hour, line.route.last().copied()),
                route_provinces: Some(line.route.len() as u32),
                base_level: Some(line.base_level),
                target_level: Some(line.target_level),
                cost: Some(line.cost),
                created_hour: Some(line.created_hour),
                first_read: hour.hour,
                census_read: Some(hour.hour),
                queue_ordinal: Some(line.queue_ordinal),
                earlier_unknown: family.earlier_unknown(&keys, line.created_hour),
                construction_raw: Recorded::NotRecorded,
                repair_raw: Recorded::NotRecorded,
                output_reason: P::OUTPUT_NOT_RECORDED,
                ..Default::default()
            };
            self.first(line.id, row, hour.hour);
            let held = self.lines.get_mut(&line.id).expect("first");
            held.open = Some(Open { keys, remaining: line.amount, repair_levels: 0 });
            Self::run(held, line.active_factories, line.amount, 0, hour.hour);
        }
        self.families[1] = Family { changes: HashMap::new(), lost: !BUILDINGS.iter().all(|&(table, _)| hour.complete(table)), read: Some(hour.hour) };
        Ok(())
    }

    fn output_of(line: &mut Line, output: &ConstructionLineOutputRow, hour: i32) {
        if line.since.is_some_and(|since| since != output.since_hour) || output.railway != (line.row.kind == P::RAILWAY) {
            line.restarted = true;
        }
        line.since = Some(output.since_hour);
        let reason = if line.row.output_alone && recorded(&output.country).is_none() {
            P::OUTPUT_OWNER_UNKNOWN
        } else if line.restarted {
            P::OUTPUT_RESTARTED
        } else if output.linked {
            P::OUTPUT_LINKED
        } else if !output.exact {
            P::OUTPUT_INEXACT
        } else {
            P::OUTPUT_KNOWN
        };
        let known = reason == P::OUTPUT_KNOWN;
        line.row.output_reason = reason;
        line.row.output_since = Some(output.since_hour);
        line.row.observed_since_creation = output.observed_since_creation;
        line.row.construction_raw = if known { Recorded::Value(output.construction_raw) } else { Recorded::NotRecorded };
        line.row.repair_raw = if known { Recorded::Value(output.repair_raw) } else { Recorded::NotRecorded };
        if output.repair_ticks > 0 && line.row.first_repair_tick.is_none() {
            line.row.first_repair_tick = Some(hour);
        }
        if output.ended {
            let completion = match &output.completion {
                Recorded::Value(completion) => Some(completion.as_ref().map(|event| (event.hour, event.cancelled))),
                Recorded::NotRecorded => None,
            };
            line.ended = Some(Ended { completion, linked: output.linked, since: output.since_hour, railway: output.railway });
        }
    }

    fn outputs(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        let table = TableId::ConstructionLineOutput;
        if !hour.complete(table) {
            if hour.changes(table).status_changed || hour.fresh {
                for (&id, line) in &mut self.lines {
                    (line.row.output_reason, line.row.construction_raw, line.row.repair_raw) = (P::OUTPUT_NOT_RECORDED, Recorded::NotRecorded, Recorded::NotRecorded);
                    self.dirty.insert(id);
                }
            }
            return Ok(());
        }
        let view = hour.table(table);
        let slots: Vec<u32> =
            if hour.fresh || hour.changes(table).status_changed { view.rows().collect() } else { hour.written_rows(table, Columns::These(&OUTPUT)) };
        for slot in slots {
            let output: ConstructionLineOutputRow = row(&view, slot, &OUTPUT)?;
            let id = output.line;
            if !self.lines.contains_key(&id) {
                if output.exact && !output.linked && output.construction_raw == 0 && output.repair_raw == 0 && !output.ended {
                    continue;
                }
                let province = recorded(&output.target_flags).is_some_and(|flags| flags & PROVINCE_FLAG != 0);
                let (target_state, to_state) = if output.railway {
                    (Self::state_of(hour, recorded(&output.route_first)), Self::state_of(hour, recorded(&output.route_last)))
                } else {
                    (recorded(&output.target_state).filter(|&state| state != 0), None)
                };
                let row = P {
                    line: id,
                    kind: if output.railway { P::RAILWAY } else { P::BUILDING },
                    output_alone: true,
                    country: recorded(&output.country).unwrap_or(0),
                    building_type: if output.railway { Some(RAILWAY_TEMPLATE_TOKEN) } else { recorded(&output.building_type) },
                    target_state,
                    to_state,
                    province: if province { recorded(&output.target_location) } else { None },
                    first_read: hour.hour,
                    ..Default::default()
                };
                self.lines.insert(id, Line { row, open: None, run: None, since: None, restarted: false, ended: None });
            }
            let line = self.lines.get_mut(&id).expect("line");
            Self::output_of(line, &output, hour.hour);
            self.dirty.insert(id);
            let gone = line.open.is_none() && (line.row.output_alone || line.row.end != P::OPEN);
            if output.ended && gone && !self.closing.contains(&id) {
                self.conclude(id, hour.hour);
            }
        }
        let missing: Vec<u64> = self
            .lines
            .iter()
            .filter(|(id, line)| line.open.is_none() && line.row.end != P::OPEN && line.ended.is_none() && view.slot(u128::from(**id)).is_none())
            .map(|(&id, _)| id)
            .collect();
        for id in missing {
            if self.closing.contains(&id) {
                continue;
            }
            let line = self.lines.get_mut(&id).expect("line");
            (line.row.output_reason, line.row.construction_raw, line.row.repair_raw) = (P::OUTPUT_FINAL_MISSING, Recorded::NotRecorded, Recorded::NotRecorded);
            self.conclude(id, hour.hour);
        }
        Ok(())
    }
}

impl Writer for ConstructionProjects {
    fn name(&self) -> &'static str {
        "ConstructionProjects"
    }

    fn version(&self) -> u32 {
        2
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::ConstructionProjects), Output::State(TableId::ConstructionLineEnds)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        if std::mem::take(&mut self.reread) {
            self.read_rows(&out.table(TableId::ConstructionProjects))?;
        }
        for key in recorder_derive_framework::forces::held(out, TableId::ConstructionLineEnds) {
            out.delete(TableId::ConstructionLineEnds, key);
        }
        for id in std::mem::take(&mut self.closing) {
            self.lines.remove(&id);
            self.dirty.remove(&id);
            out.delete(TableId::ConstructionProjects, u128::from(id));
        }
        self.buildings(hour);
        for (table, family) in [(TableId::BuildingConstruction, 0), (TableId::RailwayConstruction, 1)] {
            if hour.table(table).sampled_at() == Some(hour.hour) && hour.complete(table) {
                if family == 0 {
                    self.read_buildings(hour)?;
                } else {
                    self.read_railways(hour)?;
                }
            }
        }
        self.outputs(hour)?;
        for id in std::mem::take(&mut self.dirty) {
            if let Some(line) = self.lines.get(&id) {
                let cells = cells(&line.row, out)?;
                out.set(TableId::ConstructionProjects, u128::from(id), cells);
            }
        }
        for &id in &self.closing {
            if let Some(line) = self.lines.get(&id) {
                let end = ConstructionLineEndRow::of(hour.hour, &line.row);
                out.set(TableId::ConstructionLineEnds, ConstructionLineEndRow::key_of(hour.hour, id), end_cells(&end));
            }
        }
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        fn signed(blob: &mut Bytes, value: i32) -> io::Result<()> {
            blob.raw(&value.to_le_bytes())
        }
        fn hour(blob: &mut Bytes, value: Option<i32>) -> io::Result<()> {
            match value {
                None => blob.u8(0),
                Some(value) => {
                    blob.u8(1)?;
                    signed(blob, value)
                }
            }
        }
        fn key(blob: &mut Bytes, key: Key) -> io::Result<()> {
            blob.u8(u8::from(key.0))?;
            blob.varint(u64::from(key.1))?;
            blob.varint(u64::from(key.2))
        }
        for family in &self.families {
            blob.u8(u8::from(family.lost))?;
            hour(blob, family.read)?;
            let mut changes: Vec<(&Key, &Vec<Change>)> = family.changes.iter().collect();
            changes.sort_unstable_by_key(|(key, _)| **key);
            blob.varint(changes.len() as u64)?;
            for (&at, list) in changes {
                key(blob, at)?;
                blob.varint(list.len() as u64)?;
                for &(when, was, now) in list {
                    signed(blob, when)?;
                    for level in [was.0, was.1, now.0, now.1] {
                        blob.raw(&level.to_le_bytes())?;
                    }
                }
            }
        }
        let mut lines: Vec<(&u64, &Line)> = self.lines.iter().collect();
        lines.sort_unstable_by_key(|(id, _)| **id);
        blob.varint(lines.len() as u64)?;
        for (&id, line) in lines {
            blob.varint(id)?;
            match &line.open {
                None => blob.u8(0)?,
                Some(open) => {
                    blob.u8(1)?;
                    signed(blob, open.remaining)?;
                    signed(blob, open.repair_levels)?;
                    blob.varint(open.keys.len() as u64)?;
                    for &at in &open.keys {
                        key(blob, at)?;
                    }
                }
            }
            hour(blob, line.run)?;
            hour(blob, line.since)?;
            blob.u8(u8::from(line.restarted))?;
            match line.ended {
                None => blob.u8(0)?,
                Some(ended) => {
                    blob.u8(1)?;
                    blob.u8(u8::from(ended.linked) | u8::from(ended.railway) << 1)?;
                    signed(blob, ended.since)?;
                    match ended.completion {
                        None => blob.u8(0)?,
                        Some(None) => blob.u8(1)?,
                        Some(Some((outcome, cancelled))) => {
                            blob.u8(2 + u8::from(cancelled))?;
                            signed(blob, outcome)?;
                        }
                    }
                }
            }
        }
        let mut closing = self.closing.clone();
        closing.sort_unstable();
        blob.varint(closing.len() as u64)?;
        for id in closing {
            blob.varint(id)?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        fn signed(reader: &mut Reader<'_>) -> io::Result<i32> {
            Ok(i32::from_le_bytes(reader.raw(4)?.try_into().expect("four bytes")))
        }
        fn hour(reader: &mut Reader<'_>) -> io::Result<Option<i32>> {
            Ok(match reader.u8()? {
                0 => None,
                _ => Some(signed(reader)?),
            })
        }
        fn key(reader: &mut Reader<'_>) -> io::Result<Key> {
            Ok((reader.u8()? != 0, reader.varint_u32()?, reader.varint_u32()?))
        }
        fn level(reader: &mut Reader<'_>) -> io::Result<i16> {
            Ok(i16::from_le_bytes(reader.raw(2)?.try_into().expect("two bytes")))
        }
        let count = |reader: &mut Reader<'_>| -> io::Result<usize> {
            let count = reader.count()?;
            if count > blob.len() {
                return Err(invalid("ConstructionProjects' state counts more entries than it holds"));
            }
            Ok(count)
        };
        *self = Self { reread: true, ..Self::default() };
        let mut reader = Reader::new(blob);
        for family in &mut self.families {
            family.lost = reader.u8()? != 0;
            family.read = hour(&mut reader)?;
            for _ in 0..count(&mut reader)? {
                let at = key(&mut reader)?;
                let list = family.changes.entry(at).or_default();
                for _ in 0..count(&mut reader)? {
                    let when = signed(&mut reader)?;
                    let levels = [level(&mut reader)?, level(&mut reader)?, level(&mut reader)?, level(&mut reader)?];
                    list.push((when, (levels[0], levels[1]), (levels[2], levels[3])));
                }
            }
        }
        for _ in 0..count(&mut reader)? {
            let id = reader.varint()?;
            let open = match reader.u8()? {
                0 => None,
                _ => {
                    let (remaining, repair_levels) = (signed(&mut reader)?, signed(&mut reader)?);
                    let mut keys = Vec::new();
                    for _ in 0..count(&mut reader)? {
                        keys.push(key(&mut reader)?);
                    }
                    Some(Open { keys, remaining, repair_levels })
                }
            };
            let run = hour(&mut reader)?;
            let since = hour(&mut reader)?;
            let restarted = reader.u8()? != 0;
            let ended = match reader.u8()? {
                0 => None,
                _ => {
                    let flags = reader.u8()?;
                    let since = signed(&mut reader)?;
                    let completion = match reader.u8()? {
                        0 => None,
                        1 => Some(None),
                        tag => Some(Some((signed(&mut reader)?, tag == 3))),
                    };
                    Some(Ended { completion, linked: flags & 1 != 0, since, railway: flags & 2 != 0 })
                }
            };
            self.saved.insert(id, Line { row: P::default(), open, run, since, restarted, ended });
        }
        for _ in 0..count(&mut reader)? {
            self.closing.push(reader.varint()?);
        }
        reader.finish()
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        if self.read_rows(&hour.table(TableId::ConstructionProjects)).is_err() {
            self.lines.clear();
        }
    }
}

impl ConstructionProjects {
    fn read_rows(&mut self, view: &TableView<'_>) -> io::Result<()> {
        self.reread = false;
        let every: Vec<usize> = (0..TableId::ConstructionProjects.field_count()).collect();
        for (id, mut line) in std::mem::take(&mut self.saved) {
            let Some(slot) = view.slot(u128::from(id)) else {
                return Err(invalid(format!("ConstructionProjects' state names line {id}, which its table does not hold")));
            };
            line.row = row(view, slot, &every)?;
            self.lines.insert(id, line);
        }
        Ok(())
    }
}
