
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use replay_columnar::{
    leaf::Writer as Bytes,
    store::{TableView, Value as Held},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        AirBaseRegistrationRow, AirWingGameScoreStatsRow, CountryFactoriesRow, CountryGameScoreInputRow, CountryGameScoreRow as Score, GameScoreAirSite,
        GameScoreMultiplierRow, RowCodec, TableId,
    },
};
use std::{collections::HashMap, io};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const Q: i64 = 100_000;

const INPUT: [usize; 9] = columns::<CountryGameScoreInputRow, 9>([
    "army_count",
    "army_error",
    "navy_count",
    "navy_error",
    "land_count",
    "land_error",
    "air_state",
    "air_error",
    "first_sites",
]);
const MULTIPLIERS: [usize; 6] = columns::<GameScoreMultiplierRow, 6>(["army_raw", "navy_raw", "air_raw", "industry_raw", "land_raw", "total_raw"]);
const FACTORY: [usize; 7] = columns::<CountryFactoriesRow, 7>([
    "naval_total_raw",
    "civilian_total_raw",
    "military_total_raw",
    "civilian_to_others",
    "civilian_to_overlord",
    "military_to_others",
    "military_to_overlord",
]);
const REGISTRATION: [usize; 1] = columns::<AirBaseRegistrationRow, 1>(["wings"]);
const WING: [usize; 4] = columns::<AirWingGameScoreStatsRow, 4>(["planes", "planes_error", "stat_raw", "stat_error"]);
const LINK: [usize; 1] = [0];

const READS: &[Read] = &[
    Read { table: TableId::Countries, columns: Columns::These(&[]), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryGameScoreInputs, columns: Columns::These(&INPUT), watch: Watch::Changed, required: false },
    Read { table: TableId::GameScoreMultipliers, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::CountryFactories, columns: Columns::These(&FACTORY), watch: Watch::Changed, required: false },
    Read { table: TableId::AirBaseRegistrations, columns: Columns::These(&REGISTRATION), watch: Watch::Changed, required: false },
    Read { table: TableId::AirWingGameScoreStats, columns: Columns::These(&WING), watch: Watch::Changed, required: false },
];

const OPTIONAL: [TableId; 5] = [
    TableId::CountryGameScoreInputs,
    TableId::GameScoreMultipliers,
    TableId::CountryFactories,
    TableId::AirBaseRegistrations,
    TableId::AirWingGameScoreStats,
];

const ARMY: usize = 0;
const NAVY: usize = 1;
const AIR: usize = 2;
const INDUSTRY: usize = 3;
const LAND: usize = 4;
const TOTAL: usize = 5;

type Part = Result<i64, u8>;

fn count_score(count: i32, multiplier: i64) -> i64 {
    i64::from(count).wrapping_mul(multiplier).wrapping_mul(Q) / Q
}

fn industry_count(factories: [i64; 7]) -> i32 {
    let [naval, civilian, military, civilian_others, civilian_overlord, military_others, military_overlord] = factories;
    ((naval / Q) as i32)
        .wrapping_add((civilian / Q) as i32)
        .wrapping_add((military / Q) as i32)
        .wrapping_sub(civilian_others as i32)
        .wrapping_sub(civilian_overlord as i32)
        .wrapping_sub(military_others as i32)
        .wrapping_sub(military_overlord as i32)
}

fn air_contribution(planes: i32, stat: i64) -> i64 {
    stat.wrapping_mul(i64::from(planes).wrapping_mul(Q)) / Q
}

fn total_score(army: i64, navy: i64, air: i64, multiplier: i64) -> i64 {
    air.wrapping_add(navy).wrapping_add(army).wrapping_mul(multiplier) / Q
}

fn counted(count: Option<i32>, multiplier: Option<i64>) -> Part {
    let count = count.ok_or(Score::COUNT_UNREAD)?;
    Ok(count_score(count, multiplier.ok_or(Score::MULTIPLIER)?))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Reach {
    registrations: Vec<u128>,
    wings: Vec<u64>,
}

trait Joins {
    fn registration(&mut self, key: u128) -> io::Result<Result<Vec<u64>, u8>>;
    fn wing(&mut self, id: u64) -> io::Result<Result<(Option<i32>, Option<i64>), u8>>;
}

fn air(row: &CountryGameScoreInputRow, slot: Option<u32>, multiplier: Option<i64>, joins: &mut dyn Joins, reach: &mut Reach) -> io::Result<Part> {
    match row.air_state {
        CountryGameScoreInputRow::AIR_ABSENT => return Ok(Ok(0)),
        CountryGameScoreInputRow::AIR_PRESENT => {}
        _ => return Ok(Err(Score::AIR_ENTRY)),
    }
    let mut sum = 0_i64;
    for site in &row.first_sites {
        match site.access {
            GameScoreAirSite::ACCESS_FALSE => continue,
            GameScoreAirSite::ACCESS_TRUE => {}
            _ => return Ok(Err(Score::BASE_ACCESS)),
        }
        let Some(slot) = slot else { return Ok(Err(Score::NO_LINK)) };
        let key = u128::from(site.base) << 32 | u128::from(slot);
        reach.registrations.push(key);
        let wings = match joins.registration(key)? {
            Ok(wings) => wings,
            Err(reason) => return Ok(Err(reason)),
        };
        for id in wings {
            reach.wings.push(id);
            let (planes, stat) = match joins.wing(id)? {
                Ok(read) => read,
                Err(reason) => return Ok(Err(reason)),
            };
            let Some(planes) = planes else { return Ok(Err(Score::PLANES)) };
            let Some(stat) = stat else { return Ok(Err(Score::STAT)) };
            sum = sum.wrapping_add(air_contribution(planes, stat));
        }
    }
    Ok(multiplier.map(|multiplier| sum.wrapping_mul(multiplier) / Q).ok_or(Score::MULTIPLIER))
}

fn scores(
    input: Result<&CountryGameScoreInputRow, u8>,
    factories: Result<[i64; 7], u8>,
    multipliers: &[Option<i64>; 6],
    slot: Option<u32>,
    joins: &mut dyn Joins,
    reach: &mut Reach,
) -> io::Result<[Part; 6]> {
    let mut parts = match input {
        Err(reason) => [Err(reason); 6],
        Ok(row) => {
            let army = counted(row.army_count, multipliers[ARMY]);
            let navy = counted(row.navy_count, multipliers[NAVY]);
            let land = counted(row.land_count, multipliers[LAND]);
            let air = air(row, slot, multipliers[AIR], joins, reach)?;
            let total = match (army, navy, air, multipliers[TOTAL]) {
                (Ok(army), Ok(navy), Ok(air), Some(multiplier)) => Ok(total_score(army, navy, air, multiplier)),
                _ => Err(Score::TOTAL_PARTS),
            };
            let mut parts = [Err(Score::KNOWN); 6];
            (parts[ARMY], parts[NAVY], parts[AIR], parts[LAND], parts[TOTAL]) = (army, navy, air, land, total);
            parts
        }
    };
    parts[INDUSTRY] = factories.and_then(|factories| {
        let multiplier = multipliers[INDUSTRY].ok_or(Score::MULTIPLIER)?;
        Ok(count_score(industry_count(factories), multiplier))
    });
    Ok(parts)
}

fn cells(parts: &[Part; 6]) -> Vec<Value> {
    let values = parts.iter().map(|part| part.map_or(Value::Unrecorded, Value::I64));
    let reasons = parts.iter().map(|part| Value::U8(part.err().unwrap_or(Score::KNOWN)));
    values.chain(reasons).collect()
}

fn number(table: &TableView<'_>, column: usize, slot: u32) -> Option<i64> {
    match table.value(column, slot) {
        Held::Number(bits) => Some(bits as i64),
        _ => None,
    }
}

fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, slot: u32, mask: u64) -> io::Result<R> {
    let held = hour.table(table);
    let key = held.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&held, slot, column, format))
}

fn mask<const N: usize>(columns: [usize; N]) -> u64 {
    columns.iter().fold(0, |mask, column| mask | 1 << column)
}

struct Joined<'h, 'a> {
    hour: &'h Hour<'a>,
    complete: [bool; 2],
}

impl Joins for Joined<'_, '_> {
    fn registration(&mut self, key: u128) -> io::Result<Result<Vec<u64>, u8>> {
        if !self.complete[0] {
            return Ok(Err(Score::REGISTRATIONS_INCOMPLETE));
        }
        let table = self.hour.table(TableId::AirBaseRegistrations);
        let Some(slot) = table.slot(key) else { return Ok(Err(Score::NO_REGISTRATION)) };
        let bytes = table.list(REGISTRATION[0], slot).ok_or_else(|| invalid("An AirBaseRegistrations row holds no wings list"))?;
        Ok(Ok(bytes.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().expect("8 bytes"))).collect()))
    }

    fn wing(&mut self, id: u64) -> io::Result<Result<(Option<i32>, Option<i64>), u8>> {
        if !self.complete[1] {
            return Ok(Err(Score::WINGS_INCOMPLETE));
        }
        let table = self.hour.table(TableId::AirWingGameScoreStats);
        let Some(slot) = table.slot(u128::from(id)) else { return Ok(Err(Score::NO_WING)) };
        Ok(Ok((number(&table, WING[0], slot).map(|planes| planes as i32), number(&table, WING[2], slot))))
    }
}

#[derive(Default)]
pub struct GameScore {
    rebuild: bool,
    links: HashMap<u32, u32>,
    multipliers: [Option<i64>; 6],
    complete: [bool; 5],
    reach: HashMap<u32, Reach>,
    by_registration: HashMap<u128, Vec<u32>>,
    by_wing: HashMap<u64, Vec<u32>>,
}

fn edited(hour: &Hour<'_>, table: TableId, columns: &'static [usize]) -> Vec<u128> {
    let held = hour.table(table);
    let mut keys: Vec<u128> = hour.written_rows(table, Columns::These(columns)).into_iter().filter_map(|slot| held.key(slot)).collect();
    keys.extend(hour.deleted_keys(table));
    keys
}

impl GameScore {
    fn unindex(&mut self, id: u32) {
        let Some(reach) = self.reach.remove(&id) else { return };
        for key in reach.registrations {
            if let Some(ids) = self.by_registration.get_mut(&key) {
                ids.retain(|&held| held != id);
                if ids.is_empty() {
                    self.by_registration.remove(&key);
                }
            }
        }
        for wing in reach.wings {
            if let Some(ids) = self.by_wing.get_mut(&wing) {
                ids.retain(|&held| held != id);
                if ids.is_empty() {
                    self.by_wing.remove(&wing);
                }
            }
        }
    }

    fn index(&mut self, id: u32, mut reach: Reach) {
        if reach.registrations.is_empty() && reach.wings.is_empty() {
            return;
        }
        reach.registrations.sort_unstable();
        reach.registrations.dedup();
        reach.wings.sort_unstable();
        reach.wings.dedup();
        for &key in &reach.registrations {
            self.by_registration.entry(key).or_default().push(id);
        }
        for &wing in &reach.wings {
            self.by_wing.entry(wing).or_default().push(id);
        }
        self.reach.insert(id, reach);
    }

    fn read_whole(&mut self, hour: &Hour<'_>, complete: [bool; 5]) {
        self.complete = complete;
        self.links.clear();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(key), Some(canonical)) = (links.key(slot), number(&links, LINK[0], slot)) else { continue };
            self.links.insert(key as u32, canonical as u32);
        }
        self.multipliers = [None; 6];
        if complete[1] {
            let table = hour.table(TableId::GameScoreMultipliers);
            if let Some(slot) = table.slot(0) {
                self.multipliers = MULTIPLIERS.map(|column| number(&table, column, slot));
            }
        }
        self.reach.clear();
        self.by_registration.clear();
        self.by_wing.clear();
    }

    fn compute(&mut self, hour: &Hour<'_>, id: u32) -> io::Result<Vec<Value>> {
        let key = u128::from(id);
        let input: Result<CountryGameScoreInputRow, u8> = match self.complete[0] {
            false => Err(Score::INPUTS_INCOMPLETE),
            true => match hour.table(TableId::CountryGameScoreInputs).slot(key) {
                None => Err(Score::NO_INPUT_ROW),
                Some(slot) => Ok(row(hour, TableId::CountryGameScoreInputs, slot, mask(INPUT))?),
            },
        };
        let factories = match self.complete[2] {
            false => Err(Score::FACTORIES_INCOMPLETE),
            true => {
                let table = hour.table(TableId::CountryFactories);
                table
                    .slot(key)
                    .and_then(|slot| {
                        let read = FACTORY.map(|column| number(&table, column, slot));
                        read.iter().all(Option::is_some).then(|| read.map(|value| value.unwrap_or_default()))
                    })
                    .ok_or(Score::NO_FACTORIES_ROW)
            }
        };
        let mut reach = Reach::default();
        let mut joins = Joined { hour, complete: [self.complete[3], self.complete[4]] };
        let parts = scores(input.as_ref().map_err(|reason| *reason), factories, &self.multipliers, self.links.get(&id).copied(), &mut joins, &mut reach)?;
        reach.registrations.sort_unstable();
        reach.registrations.dedup();
        reach.wings.sort_unstable();
        reach.wings.dedup();
        let empty = reach.registrations.is_empty() && reach.wings.is_empty();
        if self.reach.get(&id).map_or(!empty, |held| *held != reach) {
            self.unindex(id);
            self.index(id, reach);
        }
        Ok(cells(&parts))
    }
}

impl Writer for GameScore {
    fn name(&self) -> &'static str {
        "GameScore"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::CountryGameScores)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let complete = OPTIONAL.map(|table| hour.complete(table));
        let whole = hour.fresh
            || self.rebuild
            || complete != self.complete
            || hour.wrote(TableId::CountryLinks, Columns::These(&LINK))
            || hour.wrote(TableId::GameScoreMultipliers, Columns::Every);
        let countries = hour.table(TableId::Countries);
        let mut dirty: Vec<u128> = Vec::new();
        if whole {
            self.rebuild = false;
            self.read_whole(hour, complete);
            for key in recorder_derive_framework::forces::held(out, TableId::CountryGameScores) {
                if countries.slot(key).is_none() {
                    out.delete(TableId::CountryGameScores, key);
                }
            }
            dirty.extend(countries.rows().filter_map(|slot| countries.key(slot)));
        } else {
            for key in hour.deleted_keys(TableId::Countries) {
                self.unindex(key as u32);
                out.delete(TableId::CountryGameScores, key);
            }
            dirty.extend(edited(hour, TableId::Countries, &[]));
            dirty.extend(edited(hour, TableId::CountryGameScoreInputs, &INPUT));
            dirty.extend(edited(hour, TableId::CountryFactories, &FACTORY));
            for key in edited(hour, TableId::AirBaseRegistrations, &REGISTRATION) {
                dirty.extend(self.by_registration.get(&key).into_iter().flatten().map(|&id| u128::from(id)));
            }
            for key in edited(hour, TableId::AirWingGameScoreStats, &WING) {
                dirty.extend(self.by_wing.get(&(key as u64)).into_iter().flatten().map(|&id| u128::from(id)));
            }
            dirty.sort_unstable();
            dirty.dedup();
        }
        for key in dirty {
            if countries.slot(key).is_none() {
                continue;
            }
            let cells = self.compute(hour, key as u32)?;
            out.set(TableId::CountryGameScores, key, cells);
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("GameScore's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
