//! `RaidEnds`: a log of raids' ends, a row living the hour a raid's row is first read ended or
//! is gone without an ended read, carrying what a finished raid shows, so a reply reads one row
//! per finished raid instead of its row's history. The victim is the game's target country rule
//! ([`RaidScope::country_with`]) at the end hour: an ended raid's frozen at its
//! first ended read. A raid already ended at the writer's first read in the run has no row; one
//! gone during hours Raids was not complete ends at the first hour it is read again.
//!
//! Held, per live raid not yet ended: the first hour its row was read and the first read it was
//! launched, which no table holds, so they are the resume state (about 24 bytes a live raid). An
//! ended raid holds nothing; a deleted one's last cells are the hour's kept cells.

use crate::{
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

/// Raids' columns, as the schema numbers them.
const PARENT_SLOT: usize = 0;
const PHASE: usize = 2;
const OUTCOME: usize = 3;
const TRAVELLED: usize = 16;
const PREPARATION: usize = 17;
/// Raids' columns whose change can launch or end a raid.
const COURSE: [usize; 2] = [PHASE, TRAVELLED];
/// RaidTargets' columns as the schema numbers them, the scope's nine then the type token, and
/// how the end row holds each.
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

/// A row's cells: a live row's at its slot, or a deleted row's as the hour kept them.
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

    /// A number its row cannot leave unrecorded.
    fn number(&self, column: usize) -> io::Result<u64> {
        match self.value(column) {
            Held::Number(bits) => Ok(bits),
            _ => Err(invalid(format!("Raids column {column} is not recorded and its row cannot say so"))),
        }
    }

    /// An optional number: none for a missing value.
    fn optional(&self, column: usize) -> io::Result<Option<u64>> {
        match self.value(column) {
            Held::Number(bits) => Ok(Some(bits)),
            Held::Missing => Ok(None),
            _ => Err(invalid(format!("Raids column {column} is not recorded and its row cannot say so"))),
        }
    }

    /// Column `column` copied as a column of `kind` holds it; not recorded where the cell is not.
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

/// The fixed64 finite domain: every signed value but the three infinity and undefined sentinels.
fn finite(raw: i64) -> bool {
    (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

/// Whether a raid's row proves it launched: in progress, or a finite nonzero distance flown
/// (entering progress clears the distance and only progress adds to it).
fn launched(raid: &Cells<'_>) -> io::Result<bool> {
    Ok(raid.number(PHASE)? as i32 == PHASE_IN_PROGRESS || raid.optional(TRAVELLED)?.is_some_and(|distance| distance != 0 && finite(distance as i64)))
}

/// What the writer holds of a live raid not yet ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Live {
    /// The first hour its row was read; none when first read after hours Raids was not complete.
    first: Option<i32>,
    /// It was first read the hour after one Raids was read without it.
    first_exact: bool,
    launched: Option<i32>,
}

/// The hour as a raid's end reads it beside the raid's own row.
struct Places<'a> {
    at: i32,
    /// The hour Raids was read before.
    previous: i32,
    targets: TableView<'a>,
    /// RaidTargets rows the hour deleted, by key.
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

    /// `column` of row `id` of `view`, while it is complete and holds it recorded.
    fn cell(view: &TableView<'_>, id: u32, column: usize) -> Result<u32, u8> {
        if view.status() != Status::Complete {
            return Err(RaidEndRow::LOCATION_UNKNOWN);
        }
        let slot = view.slot(u128::from(id)).ok_or(RaidEndRow::LOCATION_UNKNOWN)?;
        view.number(column, slot).map(|value| value as u32).ok_or(RaidEndRow::LOCATION_UNKNOWN)
    }

    /// The target columns of raid `key`, from its RaidTargets row as the hour deleted it or as it
    /// lives; not recorded without one.
    fn target(&self, key: u128) -> Vec<Value> {
        let cells = match (self.gone.get(&key), self.targets.slot(key)) {
            (Some(cells), _) => Cells::Kept(cells),
            (None, Some(slot)) => Cells::Live(&self.targets, slot),
            (None, None) => return vec![Value::Unrecorded; TARGET.len()],
        };
        TARGET.iter().map(|&(column, kind)| cells.copy(column, kind)).collect()
    }

    /// The end row's target state, victim, victim slot and why the victim is not recorded, from
    /// its target columns.
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

    /// Raid `key`'s end row this hour: `kind`, its own cells `raid` (none when its last read is
    /// lost), and what was held of it.
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

/// The scope the target columns hold, none while one is not recorded.
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
    /// The last hour the writer read Raids; none before its first read in the run.
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

    // Hourly: its rows live one hour, and a raid's first hour is exact after a read hour.
    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let at = hour.hour;
        for key in crate::forces::held(out, TableId::RaidEnds) {
            out.delete(TableId::RaidEnds, key);
        }
        let raids = hour.table(TableId::Raids);
        let Some(previous) = self.last.replace(at) else {
            // The first read is a baseline: every raid not ended is held, none ends.
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
            // Raids gone while the writer was not run: their deletion and last read are lost.
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
                // First read after hours Raids was not complete.
                None if !ended => Some(Live { first: None, first_exact: false, launched: launch }),
                // Ended before: its end was read, or it was ended at the first read.
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

    /// The last hour read, then each live raid by key: first hour, whether it is exact, launch.
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
