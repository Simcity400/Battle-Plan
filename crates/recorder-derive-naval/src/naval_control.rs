
use recorder_derive_framework::{
    forces::invalid,
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::{
    format::FormatVersion,
    rows::{
        AirCountryRelationRow, CountryCounterInputRow, NavalRegionControlRow, NavalSupremacyRow, ReservedCountryNamingRow, TableId,
        tables,
    },
    scope::CaptureMode,
};
use replay_derive::naval_region_control::{self, ControlContext, World};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

const CURRENT: [usize; 1] = columns::<NavalSupremacyRow, 1>(["current"]);
const NAMING: [usize; 2] = columns::<ReservedCountryNamingRow, 2>(["faction", "reverse_visibility"]);
const COUNTERS: [usize; 2] = columns::<CountryCounterInputRow, 2>(["forward_visibility", "directed_wars"]);
const TUPLE_COUNTRIES: [usize; 2] = [0, 1];
const SLOT: [usize; 1] = [0];

const READS: &[Read] = &[
    Read { table: TableId::NavalSupremacy, columns: Columns::These(&CURRENT), watch: Watch::EveryRead, required: true },
    Read { table: TableId::Countries, columns: Columns::These(&SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::FactionMembers, columns: Columns::Every, watch: Watch::Changed, required: true },
    Read { table: TableId::Subjects, columns: Columns::These(&[]), watch: Watch::Changed, required: true },
    Read { table: TableId::WarRelations, columns: Columns::These(&[]), watch: Watch::Changed, required: true },
    Read { table: TableId::RelationTuples, columns: Columns::These(&TUPLE_COUNTRIES), watch: Watch::Changed, required: true },
    Read { table: TableId::AirCountryRelations, columns: Columns::Every, watch: Watch::Changed, required: true },
    Read { table: TableId::ReservedCountryNaming, columns: Columns::These(&NAMING), watch: Watch::Changed, required: false },
    Read { table: TableId::CountryCounterInputs, columns: Columns::These(&COUNTERS), watch: Watch::Changed, required: false },
];
const RELATIONS: std::ops::Range<usize> = 1..8;
const RESERVED: std::ops::Range<usize> = 8..10;

const OUTSIDER: u32 = u32::MAX - 1;

#[derive(Debug, PartialEq, Eq)]
struct Outputs {
    control: Result<(i64, i64), String>,
    share: Result<(i64, i64), String>,
    own: Option<i64>,
}

impl Outputs {
    fn zero(&self) -> bool {
        self.control == Ok((0, 0)) && self.share == Ok((0, 0)) && self.own == Some(0)
    }

    fn cells(&self, out: &mut Rows) -> Vec<Value> {
        let pair = |held: &Result<(i64, i64), String>| match held {
            Ok((a, b)) => [Value::I64(*a), Value::I64(*b)],
            Err(_) => [Value::Unrecorded; 2],
        };
        let mut cells = Vec::with_capacity(7);
        cells.extend(pair(&self.control));
        cells.extend(pair(&self.share));
        cells.push(self.own.map_or(Value::Unrecorded, Value::I64));
        for held in [&self.control, &self.share] {
            cells.push(out.text(held.as_ref().err().map_or(&[][..], |why| why.as_bytes())));
        }
        cells
    }
}

#[derive(Default)]
struct Concerned {
    factions: HashMap<u32, u64>,
    members: HashMap<u64, Vec<u32>>,
    others: HashMap<u32, BTreeSet<u32>>,
    troubled: BTreeSet<u32>,
}

impl Concerned {
    fn read(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> Self {
        let slot = |raw: u32| if raw == 0 { Some(0) } else { links.get(&raw).copied() };
        let mut out = Self::default();
        let relate = |a: Option<u32>, b: Option<u32>, both: bool, out: &mut Self| {
            if let (Some(a), Some(b)) = (a, b) {
                out.others.entry(a).or_default().insert(b);
                if both {
                    out.others.entry(b).or_default().insert(a);
                }
            }
        };
        if let Ok(rows) = frame.complete::<tables::FactionMembers>() {
            for row in rows {
                if let Some(member) = slot(row.country) {
                    out.factions.insert(member, row.faction);
                    out.members.entry(row.faction).or_default().push(member);
                }
            }
        }
        if let Ok(rows) = frame.complete::<tables::Subjects>() {
            for row in rows {
                relate(slot(row.overlord), slot(row.subject), true, &mut out);
            }
        }
        if let Ok(rows) = frame.complete::<tables::WarRelations>() {
            for row in rows {
                relate(slot(row.first_country), slot(row.second_country), true, &mut out);
            }
        }
        if let Ok(rows) = frame.complete::<tables::RelationTuples>() {
            for row in rows.iter().filter(|row| row.type_id == naval_region_control::NAVAL_BLOCKADE_RELATION_TYPE) {
                relate(slot(row.first_country), slot(row.second_country), false, &mut out);
            }
        }
        if let Ok(rows) = frame.complete::<tables::AirCountryRelations>() {
            let mut ordinals: HashMap<u32, Vec<u32>> = HashMap::new();
            for row in rows {
                if row.kind == AirCountryRelationRow::ENEMY {
                    ordinals.entry(row.country).or_default().push(row.ordinal);
                } else if row.kind != AirCountryRelationRow::ALLY || (row.other as i32) <= 0 {
                    continue;
                }
                match slot(row.other) {
                    Some(listed) => relate(Some(listed), Some(row.country), false, &mut out),
                    None => {
                        out.troubled.insert(row.country);
                    }
                }
            }
            for (country, mut held) in ordinals {
                held.sort_unstable();
                if held.iter().enumerate().any(|(at, &ordinal)| ordinal as usize != at) {
                    out.troubled.insert(country);
                }
            }
        }
        out
    }

    fn of(&self, owner: u32, into: &mut BTreeSet<u32>) {
        into.insert(owner);
        if let Some(members) = self.factions.get(&owner).and_then(|faction| self.members.get(faction)) {
            into.extend(members.iter().copied());
        }
        if let Some(others) = self.others.get(&owner) {
            into.extend(others.iter().copied());
        }
    }
}

fn frame(hour: &Hour<'_>, reserved: bool) -> io::Result<DecodedFrame> {
    let mut tables = vec![
        TableId::NavalSupremacy,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::ReservedCountryNaming,
        TableId::FactionMembers,
        TableId::Subjects,
        TableId::WarRelations,
        TableId::RelationTuples,
        TableId::AirCountryRelations,
    ];
    if reserved {
        tables.push(TableId::CountryCounterInputs);
    }
    let mut frame = replay_frame::frame::stored_frame(
        tables.iter().map(|&table| (table, hour.table(table))),
        (hour.branch, hour.hour),
        CaptureMode::Full,
        FormatVersion::CURRENT.version(),
    )?;
    let at = TableId::NavalSupremacy.index();
    frame.tables[at] = frame.tables[at].clone().with_sampled_at(hour.table(TableId::NavalSupremacy).sampled_at());
    Ok(frame)
}

fn reserved(links: &TableView<'_>, supremacy: &TableView<'_>) -> bool {
    let linked_to_zero = |raw: u32| links.slot(u128::from(raw)).and_then(|slot| links.number(SLOT[0], slot)) == Some(0);
    links.rows().any(|slot| links.number(SLOT[0], slot) == Some(0))
        || supremacy.rows().filter_map(|slot| supremacy.key(slot)).any(|key| key as u32 == 0 || linked_to_zero(key as u32))
}

#[derive(Default)]
pub struct NavalControl {
    rebuild: bool,
    held: HashMap<u32, Vec<u32>>,
}

impl NavalControl {
    fn region<'w, 'a>(world: &'w World<'a>, contexts: &mut HashMap<u32, ControlContext<'w, 'a>>, frame: &DecodedFrame, links: &BTreeMap<u32, u32>, live: &BTreeSet<u32>, concerned: &Concerned, region: u32) -> Result<Vec<(u32, Outputs)>, String> {
        let rows = naval_region_control::regional_rows(frame, region)?;
        if rows.iter().all(|row| row.current == 0) {
            return Ok(Vec::new());
        }
        let slot = |row: &NavalSupremacyRow| if row.country == 0 { Some(0) } else { links.get(&row.country).copied() };
        let mut viewers = BTreeSet::new();
        let mut everyone = false;
        for row in &rows {
            let owner = slot(row);
            viewers.extend(owner);
            match owner {
                _ if row.current == 0 => {}
                Some(0) => everyone = true,
                Some(owner) => concerned.of(owner, &mut viewers),
                None => {}
            }
        }
        viewers.extend(concerned.troubled.iter().copied());
        viewers.insert(0);
        if everyone {
            viewers.extend(live.iter().copied());
        }
        viewers.retain(|viewer| live.contains(viewer));
        let unresolved = rows.iter().any(|row| row.current != 0 && slot(row).is_none());
        let mut out = Vec::new();
        for viewer in viewers.into_iter().chain([OUTSIDER]) {
            let context = contexts.entry(viewer).or_insert_with(|| ControlContext::shared(world, viewer));
            let own = match rows.iter().find(|row| slot(row) == Some(viewer)) {
                Some(row) => Some(row.current),
                None if unresolved => None,
                None => Some(0),
            };
            let outputs = Outputs { control: context.sums(region), share: context.share(region, i64::MIN), own };
            if !outputs.zero() {
                out.push((if viewer == OUTSIDER { NavalRegionControlRow::EVERY_VIEWER } else { viewer }, outputs));
            }
        }
        Ok(out)
    }
}

impl Writer for NavalControl {
    fn name(&self) -> &'static str {
        "NavalControl"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::NavalRegionControl)]
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
        let table = TableId::NavalRegionControl;
        let supremacy = hour.table(TableId::NavalSupremacy);
        let refused: Option<&[u8]> = match supremacy.sampled_at() {
            None => Some(&b"NavalSupremacy acquisition provenance is unrecorded"[..]),
            Some(read) if read > hour.hour || read.div_euclid(24) != hour.hour.div_euclid(24) => Some(&b"NavalSupremacy is outside its native daily cache epoch"[..]),
            Some(_) => None,
        };
        if let Some(why) = refused {
            out.unavailable(table, why);
            self.rebuild = true;
            return Ok(());
        }
        let links_view = hour.table(TableId::CountryLinks);
        let moved = |range: std::ops::Range<usize>| READS[range].iter().any(|read| hour.changes(read.table).status_changed || hour.wrote(read.table, read.columns));
        let mut whole = hour.fresh || self.rebuild || moved(RELATIONS);
        let reserved = reserved(&links_view, &supremacy);
        whole |= reserved && moved(RESERVED);
        let mut regions: BTreeSet<u32> = BTreeSet::new();
        if whole {
            self.rebuild = false;
            self.held.clear();
            for key in recorder_derive_framework::forces::held(out, table) {
                self.held.entry(key as u32).or_default().push((key >> 32) as u32);
            }
            regions.extend(self.held.keys().copied());
            regions.extend(supremacy.rows().filter_map(|slot| supremacy.key(slot)).map(|key| (key >> 32) as u32));
        } else {
            regions.extend(hour.deleted_keys(TableId::NavalSupremacy).map(|key| (key >> 32) as u32));
            for slot in hour.written_rows(TableId::NavalSupremacy, Columns::These(&CURRENT)) {
                let key = supremacy.key(slot).ok_or_else(|| invalid(format!("NavalSupremacy slot {slot} holds no row")))?;
                regions.insert((key >> 32) as u32);
            }
            if regions.is_empty() {
                return Ok(());
            }
        }
        let frame = frame(hour, reserved)?;
        let links: BTreeMap<u32, u32> = frame.complete::<tables::CountryLinks>()?.iter().map(|row| (row.id, row.canonical_slot)).collect();
        let live: BTreeSet<u32> = links.values().copied().collect();
        let world = World::new(&frame, &links);
        let concerned = Concerned::read(&frame, &links);
        let mut contexts = HashMap::new();
        for region in regions {
            let rows = match Self::region(&world, &mut contexts, &frame, &links, &live, &concerned, region) {
                Ok(rows) => rows,
                Err(why) => {
                    out.unavailable(table, why.as_bytes());
                    self.rebuild = true;
                    return Ok(());
                }
            };
            let mut viewers = Vec::with_capacity(rows.len());
            for (viewer, outputs) in &rows {
                let cells = outputs.cells(out);
                out.set(table, u128::from(*viewer) << 32 | u128::from(region), cells);
                viewers.push(*viewer);
            }
            for &viewer in self.held.get(&region).into_iter().flatten() {
                if !viewers.contains(&viewer) {
                    out.delete(table, u128::from(viewer) << 32 | u128::from(region));
                }
            }
            if viewers.is_empty() {
                self.held.remove(&region);
            } else {
                self.held.insert(region, viewers);
            }
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("NavalControl's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
