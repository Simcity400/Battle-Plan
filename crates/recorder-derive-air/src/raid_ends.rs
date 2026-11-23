
use recorder_derive_framework::{
    forces::invalid,
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use engine_native::craid_instance::{PHASE_ENDED, PHASE_IN_PROGRESS};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
    store::{Kept, TableView, Value as Held},
};
use replay_data::rows::{CountryLinkRow, ProvinceRow, RaidEndRow, RaidScope, StateRow, TableId, TagHolderRow};
use std::{collections::HashMap, io};

const PARENT_SLOT: usize = 0;
const PHASE: usize = 2;
const OUTCOME: usize = 3;
const TRAVELLED: usize = 16;
const PREPARATION: usize = 17;
const COURSE: [usize; 2] = [PHASE, TRAVELLED];
const TARGET: [(usize, Kind); 10] = [
    (0, Kind::U32),
    (1, Kind::U32),
    (2, Kind::U64),
    (3, Kind::U32),
    (4, Kind::U32),
    (5, Kind::U32),
    (6, Kind::U32),
    (7, Kind::I32),
    (8, Kind::OptionalI32),
    (9, Kind::U32),
];
const PROVINCE: [usize; 2] = columns::<ProvinceRow, 2>(["state", "controller"]);
const STATE: [usize; 1] = columns::<StateRow, 1>(["controller"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);

const READS: &[Read] = &[
    Read { table: TableId::Raids, columns: Columns::These(&COURSE), watch: Watch::Deleted, required: true },
    Read { table: TableId::RaidTargets, columns: Columns::These(&[]), watch: Watch::Deleted, required: true },
    Read { table: TableId::Provinces, columns: Columns::These(&PROVINCE), watch: Watch::Changed, required: false },
    Read { table: TableId::States, columns: Columns::These(&STATE), watch: Watch::Changed, required: false },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK), watch: Watch::Changed, required: false },
];

#[derive(Clone, Copy)]
enum Kind {
    U32,
    U64,
    I32,
    OptionalI32,
}

enum Cells<'a> {
    Live(&'a TableView<'a>, u32),
    Kept(&'a [Kept]),
}

impl Cells<'_> {
    fn value(&self, column: usize) -> Held<'_> {
        match self {
            Self::Live(view, slot) => view.value(column, *slot),
            Self::Kept(cells) => cells.get(column).map_or(Held::Unrecorded, Kept::value),
        }
    }

    fn number(&self, column: usize) -> io::Result<u64> {
        match self.value(column) {
            Held::Number(bits) => Ok(bits),
            _ => Err(invalid(format!("Raids column {column} is not recorded and its row cannot say so"))),
        }
    }

    fn optional(&self, column: usize) -> io::Result<Option<u64>> {
        match self.value(column) {
            Held::Number(bits) => Ok(Some(bits)),
            Held::Missing => Ok(None),
            _ => Err(invalid(format!("Raids column {column} is not recorded and its row cannot say so"))),
        }
    }

    fn copy(&self, column: usize, kind: Kind) -> Value {
        match (self.value(column), kind) {
            (Held::Number(bits), Kind::U32) => Value::U32(bits as u32),
            (Held::Number(bits), Kind::U64) => Value::U64(bits),
            (Held::Number(bits), Kind::I32) => Value::I32(bits as i32),
            (Held::Number(bits), Kind::OptionalI32) => Value::OptionalI32(Some(bits as i32)),
            (Held::Missing, Kind::OptionalI32) => Value::OptionalI32(None),
            _ => Value::Unrecorded,
        }
    }
}

fn finite(raw: i64) -> bool {
    (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

fn launched(raid: &Cells<'_>) -> io::Result<bool> {
    Ok(raid.number(PHASE)? as i32 == PHASE_IN_PROGRESS || raid.optional(TRAVELLED)?.is_some_and(|distance| distance != 0 && finite(distance as i64)))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Live {
    first: Option<i32>,
    first_exact: bool,
    launched: Option<i32>,
}

struct Places<'a> {
    at: i32,
    previous: i32,
    targets: TableView<'a>,
    gone: HashMap<u128, &'a [Kept]>,
    provinces: TableView<'a>,
    states: TableView<'a>,
    links: TableView<'a>,
}

impl<'a> Places<'a> {
    fn new(hour: &Hour<'a>, previous: i32) -> Self {
        Self {
            at: hour.hour,
            previous,
            targets: hour.table(TableId::RaidTargets),
            gone: hour.dropped(TableId::RaidTargets).iter().map(|(key, cells)| (*key, &cells[..])).collect(),
            provinces: hour.table(TableId::Provinces),
            states: hour.table(TableId::States),
            links: hour.table(TableId::CountryLinks),
        }
    }

    fn cell(view: &TableView<'_>, id: u32, column: usize) -> Result<u32, u8> {
        if view.status() != Status::Complete {
            return Err(RaidEndRow::LOCATION_UNKNOWN);
        }
        let slot = view.slot(u128::from(id)).ok_or(RaidEndRow::LOCATION_UNKNOWN)?;
        view.number(column, slot).map(|value| value as u32).ok_or(RaidEndRow::LOCATION_UNKNOWN)
    }

    fn target(&self, key: u128) -> Vec<Value> {
        let cells = match (self.gone.get(&key), self.targets.slot(key)) {
            (Some(cells), _) => Cells::Kept(cells),
            (None, Some(slot)) => Cells::Live(&self.targets, slot),
            (None, None) => return vec![Value::Unrecorded; TARGET.len()],
        };
        TARGET.iter().map(|&(column, kind)| cells.copy(column, kind)).collect()
    }

    fn victim(&self, target: &[Value]) -> [Value; 4] {
        let none = Value::U32(TagHolderRow::NONE);
        let Some(scope) = scope(target) else {
            return [Value::Unrecorded, Value::Unrecorded, none, Value::U8(RaidEndRow::TARGET_UNKNOWN)];
        };
        let state = scope.state_with(|id| Self::cell(&self.provinces, id, PROVINCE[0])).map_or(Value::Unrecorded, Value::OptionalU32);
        match scope.country_with(|id| Self::cell(&self.provinces, id, PROVINCE[1]), |id| Self::cell(&self.states, id, STATE[0])) {
            Err(why) => [state, Value::Unrecorded, none, Value::U8(why)],
            Ok(None) => [state, Value::OptionalU32(None), none, Value::U8(RaidEndRow::VICTIM_KNOWN)],
            Ok(Some(raw)) => match Self::cell(&self.links, raw, LINK[0]) {
                Ok(slot) => [state, Value::OptionalU32(Some(raw)), Value::U32(slot), Value::U8(RaidEndRow::VICTIM_KNOWN)],
                Err(_) => [state, Value::OptionalU32(Some(raw)), none, Value::U8(RaidEndRow::VICTIM_UNLINKED)],
            },
        }
    }

    fn end(&self, out: &mut Rows, key: u128, kind: u8, raid: Option<&Cells<'_>>, held: Live) -> io::Result<()> {
        let target = self.target(key);
        let victim = self.victim(&target);
        let mut cells = vec![Value::U8(kind), Value::I32(self.previous), Value::OptionalI32(held.first), Value::Bool(held.first_exact), Value::OptionalI32(held.launched)];
        match raid {
            Some(raid) => cells.extend([
                Value::U32(raid.number(PARENT_SLOT)? as u32),
                Value::I32(raid.number(PHASE)? as i32),
                Value::I32(raid.number(OUTCOME)? as i32),
                Value::OptionalI64(raid.optional(TRAVELLED)?.map(|distance| distance as i64)),
                Value::I32(raid.number(PREPARATION)? as i32),
            ]),
            None => cells.extend([Value::Unrecorded; 5]),
        }
        cells.extend(target);
        cells.extend(victim);
        out.set(TableId::RaidEnds, RaidEndRow::key_of(self.at, key), cells);
        Ok(())
    }
}

fn scope(target: &[Value]) -> Option<RaidScope> {
    let u32_at = |at: usize| match target[at] {
        Value::U32(value) => Some(value),
        _ => None,
    };
    Some(RaidScope {
        province: u32_at(0)?,
        state: u32_at(1)?,
        leader_reference: match target[2] {
            Value::U64(value) => value,
            _ => return None,
        },
        leader_province: u32_at(3)?,
        building_type: u32_at(4)?,
        building_location: u32_at(5)?,
        building_flags: u32_at(6)?,
        building_country: match target[7] {
            Value::I32(value) => value,
            _ => return None,
        },
        leader_country: match target[8] {
            Value::OptionalI32(value) => value,
            _ => return None,
        },
    })
}

#[derive(Default)]
pub struct RaidEnds {
    last: Option<i32>,
    live: HashMap<u64, Live>,
}

impl Writer for RaidEnds {
    fn name(&self) -> &'static str {
        "RaidEnds"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::RaidEnds)]
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
        let at = hour.hour;
        for key in recorder_derive_framework::forces::held(out, TableId::RaidEnds) {
            out.delete(TableId::RaidEnds, key);
        }
        let raids = hour.table(TableId::Raids);
        let Some(previous) = self.last.replace(at) else {
            self.live.clear();
            for slot in raids.rows() {
                let raid = Cells::Live(&raids, slot);
                if raid.number(PHASE)? as i32 != PHASE_ENDED {
                    let key = raids.key(slot).ok_or_else(|| invalid(format!("Raids slot {slot} holds no row")))? as u64;
                    self.live.insert(key, Live { first: Some(at), first_exact: false, launched: launched(&raid)?.then_some(at) });
                }
            }
            return Ok(());
        };
        if !hour.fresh && !hour.wrote(TableId::Raids, Columns::These(&COURSE)) {
            return Ok(());
        }
        let places = Places::new(hour, previous);
        for (key, cells) in hour.dropped(TableId::Raids) {
            let raid = Cells::Kept(&cells[..]);
            let held = self.live.remove(&(*key as u64));
            if held.is_none() && raid.number(PHASE)? as i32 == PHASE_ENDED {
                continue;
            }
            places.end(out, *key, RaidEndRow::DELETED, Some(&raid), held.unwrap_or_default())?;
        }
        if hour.fresh {
            let mut gone: Vec<u64> = self.live.keys().copied().filter(|&key| raids.slot(u128::from(key)).is_none()).collect();
            gone.sort_unstable();
            for key in gone {
                let held = self.live.remove(&key).unwrap_or_default();
                places.end(out, u128::from(key), RaidEndRow::DELETED, None, held)?;
            }
        }
        let inserted = &hour.changes(TableId::Raids).inserted;
        for slot in hour.written_rows(TableId::Raids, Columns::These(&COURSE)) {
            let key = raids.key(slot).ok_or_else(|| invalid(format!("Raids slot {slot} holds no row")))?;
            let raid = Cells::Live(&raids, slot);
            let ended = raid.number(PHASE)? as i32 == PHASE_ENDED;
            let launch = launched(&raid)?.then_some(at);
            let held = match self.live.remove(&(key as u64)) {
                _ if inserted.contains(&slot) => Some(Live { first: Some(at), first_exact: previous == at - 1, launched: launch }),
                Some(held) => Some(Live { launched: held.launched.or(launch), ..held }),
                None if !ended => Some(Live { first: None, first_exact: false, launched: launch }),
                None => None,
            };
            match held {
                Some(held) if ended => places.end(out, key, RaidEndRow::ENDED, Some(&raid), held)?,
                Some(held) => {
                    self.live.insert(key as u64, held);
                }
                None => {}
            }
        }
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        fn optional(blob: &mut Bytes, value: Option<i32>) -> io::Result<()> {
            match value {
                Some(value) => {
                    blob.u8(1)?;
                    blob.raw(&value.to_le_bytes())
                }
                None => blob.u8(0),
            }
        }
        optional(blob, self.last)?;
        let mut live: Vec<(&u64, &Live)> = self.live.iter().collect();
        live.sort_unstable_by_key(|(key, _)| **key);
        blob.varint(live.len() as u64)?;
        for (key, held) in live {
            blob.raw(&key.to_le_bytes())?;
            optional(blob, held.first)?;
            blob.u8(u8::from(held.first_exact))?;
            optional(blob, held.launched)?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        fn optional(reader: &mut Reader<'_>) -> io::Result<Option<i32>> {
            Ok(match reader.u8()? {
                0 => None,
                _ => Some(i32::from_le_bytes(reader.raw(4)?.try_into().expect("4 bytes"))),
            })
        }
        *self = Self::default();
        let mut reader = Reader::new(blob);
        self.last = optional(&mut reader)?;
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("RaidEnds' state counts more raids than it holds"));
        }
        for _ in 0..count {
            let key = u64::from_le_bytes(reader.raw(8)?.try_into().expect("8 bytes"));
            let first = optional(&mut reader)?;
            let first_exact = reader.u8()? != 0;
            let launched = optional(&mut reader)?;
            self.live.insert(key, Live { first, first_exact, launched });
        }
        reader.finish()
    }
}

#[cfg(test)]
mod tests;
