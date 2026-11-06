//! Land battle facts, projected hour by hour from a columnar walk ([`replay::segments`]) for
//! the census the ordered emit keeps ([`super::emit`]). Each hour carries what the census reads
//! of it: the combats that did more than step their round counter by one, the participant,
//! loss and finalizer rows that changed, and the countries and variant names where they
//! changed. A segment's first hour, and any hour after one the census could not read, carries
//! its tables whole.

use super::losses::{SideLossInputs, progress_inputs};
use dashboard_base::products::equipment::columnar::Naming;
use dashboard_base::products::equipment::{RawVariant, VariantHour};
use dashboard_text::metadata::{CountryKeys, slot_read};
use dashboard_world::{Needs, World};
use replay::{
    columnar_frames::metrics,
    columns::{self, Status},
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        LandCombatProgressRow, LandCombatResultsRow, LandCombatRow, LandCombatSideRow,
        LandEventCoverageRow, TABLE_COUNT, TableId,
    },
};
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::LandCombats,
    TableId::LandCombatSides,
    TableId::LandCombatProgress,
    TableId::LandCombatResults,
    TableId::LandEventCoverage,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::VariantChanges,
];

/// What a projection reads of its walk's world: the countries and their cosmetic tags.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        naming: true,
        ..Needs::default()
    }
}

/// A LandCombats row as the census reads it: each side's distinct units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Combat {
    pub id: u64,
    pub province: u32,
    pub duration: i32,
    pub counts: [u32; 2],
}

/// The country tables, where the census's link tags read them.
#[derive(replay_spool::Spooled)]
pub enum Countries {
    /// Countries or CountryLinks is not complete.
    Incomplete,
    /// Complete, and no slot, tag or link changed.
    Same,
    Read(Box<(CountryKeys, HashMap<u32, u32>)>),
}

/// A table the census mirrors: its rows by key.
#[derive(replay_spool::Spooled)]
pub enum Table<T> {
    /// It does not hold every row.
    Incomplete,
    /// No row the census reads changed.
    Same,
    /// Every row.
    Whole(Vec<T>),
    /// The rows written or inserted, and the keys deleted.
    Changed { rows: Vec<T>, deleted: Vec<u64> },
}

impl<T> Table<T> {
    /// Whether the hour changed the table, as the census decides to read it again.
    pub fn changed(&self) -> bool {
        !matches!(self, Self::Same)
    }

    fn heap(&self, row: impl Fn(&T) -> usize) -> usize {
        match self {
            Self::Incomplete | Self::Same => 0,
            Self::Whole(rows) => {
                rows.capacity() * size_of::<T>() + rows.iter().map(row).sum::<usize>()
            }
            Self::Changed { rows, deleted } => {
                rows.capacity() * size_of::<T>()
                    + rows.iter().map(row).sum::<usize>()
                    + deleted.capacity() * size_of::<u64>()
            }
        }
    }
}

/// The combats, as the census reads LandCombats.
#[derive(replay_spool::Spooled)]
pub enum Combats {
    /// Not read this hour.
    Unread,
    /// Read, but not complete.
    Incomplete,
    /// Nothing changed: every combat held its round counter.
    Same,
    /// Every combat, by id.
    Whole(Vec<Combat>),
    /// The combats that did more than step their round counter by one since the hour before,
    /// by id, and the ids deleted. Every other combat stepped it by one.
    Changed {
        rows: Vec<Combat>,
        deleted: Vec<u64>,
    },
}

/// The finalizers a complete LandCombatResults read holds, under a complete coverage row
/// dated this hour: those ending in its interval, by id.
#[derive(replay_spool::Spooled)]
pub struct Results {
    pub from_hour: i32,
    pub to_hour: i32,
    pub rows: Vec<LandCombatResultsRow>,
}

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether the hour holds a complete combat or finalizer read.
    pub available: bool,
    pub countries: Countries,
    pub names: VariantHour,
    pub variants: Vec<(u64, RawVariant)>,
    /// Whether each of LandCombatSides and LandCombatProgress was read this hour.
    pub sides_read: bool,
    pub sides: Table<LandCombatSideRow>,
    pub combats: Combats,
    pub progress_read: bool,
    pub progress: Table<(u64, [SideLossInputs; 2])>,
    pub results: Option<Box<Results>>,
}

/// A LandCombats slot's combat as last read.
#[derive(Clone, Copy)]
struct Held {
    combat: Combat,
    /// The hour it was last touched, so an untouched row is found without a scan.
    touched: i32,
}

/// The columns a projection reads in place.
struct Fields {
    province: usize,
    duration: usize,
    /// LandCombats' attackers and defenders.
    roster: u64,
    /// LandCombatSides' four country lists.
    sides: u64,
    /// LandCombatProgress' manpower and equipment counters.
    progress: u64,
    end_hour: usize,
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    naming: Naming,
    cold: bool,
    last: Option<i32>,
    shapes: [Option<Status>; TABLE_COUNT],
    /// Whether the countries were complete the hour before.
    roster: bool,
    /// The combats by slot, and the last hour LandCombats was read whole.
    combats: Vec<Option<Held>>,
    live: usize,
    read: Option<i32>,
    /// Whether the emit holds LandCombatSides and LandCombatProgress as of the hour before.
    sides: bool,
    progress: bool,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"land projector: cold";

fn bit(field: usize) -> u64 {
    1_u64 << field
}

/// The slots `table` wrote any of `mask` at, or inserted, ascending.
fn edited(state: &State, table: TableId, mask: u64) -> io::Result<Vec<u32>> {
    let changes = columns::changes(state, table)?;
    let mut slots: Vec<u32> = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &written)| written & mask != 0)
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied())
        .collect();
    slots.sort_unstable();
    slots.dedup();
    Ok(slots)
}

/// Whether the hour edited `table` at all, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

fn live(state: &State, table: TableId) -> io::Result<Vec<u32>> {
    Ok(columns::table(state, table)?.live().ones().collect())
}

/// The distinct units of a roster, as the census counts them.
fn distinct(handles: &[u64]) -> io::Result<u32> {
    u32::try_from(handles.iter().collect::<HashSet<_>>().len())
        .map_err(|_| io::Error::other("Recorded roster exceeds its native count"))
}

fn combat(row: &LandCombatRow) -> io::Result<Combat> {
    Ok(Combat {
        id: row.id,
        province: row.province,
        duration: row.duration,
        counts: [distinct(&row.attackers)?, distinct(&row.defenders)?],
    })
}

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        let field = columns::field;
        let combats = TableId::LandCombats;
        let sides = TableId::LandCombatSides;
        let progress = TableId::LandCombatProgress;
        Ok(Self {
            format,
            fields: Fields {
                province: field(combats, "province")?,
                duration: field(combats, "duration")?,
                roster: bit(field(combats, "attackers")?) | bit(field(combats, "defenders")?),
                sides: [
                    "attacker_primary",
                    "attacker_secondary",
                    "defender_primary",
                    "defender_secondary",
                ]
                .iter()
                .try_fold(0, |mask, name| {
                    io::Result::Ok(mask | bit(field(sides, name)?))
                })?,
                progress: [
                    "attacker_manpower_loss",
                    "defender_manpower_loss",
                    "attacker_equipment_losses",
                    "defender_equipment_losses",
                ]
                .iter()
                .try_fold(0, |mask, name| {
                    io::Result::Ok(mask | bit(field(progress, name)?))
                })?,
                end_hour: field(TableId::LandCombatResults, "end_hour")?,
            },
            naming: Naming::new()?,
            cold: true,
            last: None,
            shapes: [None; TABLE_COUNT],
            roster: false,
            combats: Vec::new(),
            live: 0,
            read: None,
            sides: false,
            progress: false,
        })
    }

    fn version(&self) -> u16 {
        self.format.version()
    }

    fn countries(&mut self, world: &World, whole: bool) -> io::Result<Countries> {
        let roster = world.countries()?;
        let known = roster.complete();
        let became = std::mem::replace(&mut self.roster, known) != known;
        if !known {
            return Ok(Countries::Incomplete);
        }
        if !(whole || became || roster.changed()) {
            return Ok(Countries::Same);
        }
        let keys = roster
            .iter()
            .map(|country| (country.id, country.slot, roster.tag(country.tag).to_vec()))
            .collect();
        let links = roster
            .links()
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        Ok(Countries::Read(Box::new((
            CountryKeys::from_keys(keys),
            links,
        ))))
    }

    /// A mirrored table: whole where the emit holds none of it, else what changed.
    fn table<T>(
        state: &State,
        table: TableId,
        held: &mut bool,
        whole: bool,
        mask: u64,
        row: impl Fn(u32) -> io::Result<T>,
    ) -> io::Result<Table<T>> {
        if !columns::complete(state, table) {
            *held = false;
            return Ok(Table::Incomplete);
        }
        let all = whole || !std::mem::replace(held, true);
        if all {
            return live(state, table)?
                .into_iter()
                .map(&row)
                .collect::<io::Result<Vec<_>>>()
                .map(Table::Whole);
        }
        let deleted: Vec<u64> = columns::changes(state, table)?
            .deleted_keys
            .iter()
            .map(|&key| key as u64)
            .collect();
        let rows = edited(state, table, mask)?
            .into_iter()
            .map(&row)
            .collect::<io::Result<Vec<_>>>()?;
        if rows.is_empty() && deleted.is_empty() {
            return Ok(Table::Same);
        }
        Ok(Table::Changed { rows, deleted })
    }

    fn sides(&mut self, state: &State, whole: bool) -> io::Result<Table<LandCombatSideRow>> {
        let version = self.version();
        let mut sorted = Self::table(
            state,
            TableId::LandCombatSides,
            &mut self.sides,
            whole,
            self.fields.sides,
            |slot| columns::row(state, TableId::LandCombatSides, slot, version),
        )?;
        if let Table::Whole(rows) | Table::Changed { rows, .. } = &mut sorted {
            rows.sort_unstable_by_key(|row: &LandCombatSideRow| row.combat);
        }
        Ok(sorted)
    }

    fn progress(
        &mut self,
        state: &State,
        whole: bool,
    ) -> io::Result<Table<(u64, [SideLossInputs; 2])>> {
        let version = self.version();
        let mut sorted = Self::table(
            state,
            TableId::LandCombatProgress,
            &mut self.progress,
            whole,
            self.fields.progress,
            |slot| {
                let row: LandCombatProgressRow =
                    columns::row(state, TableId::LandCombatProgress, slot, version)?;
                Ok((row.combat, progress_inputs(&row)))
            },
        )?;
        if let Table::Whole(rows) | Table::Changed { rows, .. } = &mut sorted {
            rows.sort_unstable_by_key(|row: &(u64, [SideLossInputs; 2])| row.0);
        }
        Ok(sorted)
    }

    /// The combat at `slot`, read from its columns where they hold its counter and province.
    fn read_combat(&self, state: &State, slot: u32, roster: bool) -> io::Result<Combat> {
        let table = TableId::LandCombats;
        let held = columns::table(state, table)?;
        let fields = &self.fields;
        let cached = self
            .combats
            .get(slot as usize)
            .copied()
            .flatten()
            .filter(|_| !roster);
        match cached {
            Some(cached)
                if held.recorded(fields.province, slot) && held.recorded(fields.duration, slot) =>
            {
                Ok(Combat {
                    province: columns::plain::<u32>(held, fields.province)?[slot as usize],
                    duration: columns::plain::<i32>(held, fields.duration)?[slot as usize],
                    ..cached.combat
                })
            }
            _ => combat(&columns::row(state, table, slot, self.version())?),
        }
    }

    fn hold(&mut self, slot: u32, combat: Combat, hour: i32) {
        let index = slot as usize;
        if self.combats.len() <= index {
            self.combats.resize(index + 1, None);
        }
        if self.combats[index].is_none() {
            self.live += 1;
        }
        self.combats[index] = Some(Held {
            combat,
            touched: hour,
        });
    }

    fn combats(
        &mut self,
        state: &State,
        read: bool,
        hour: i32,
        whole: bool,
    ) -> io::Result<Combats> {
        let table = TableId::LandCombats;
        if !read || !columns::complete(state, table) {
            let held = read;
            self.read = None;
            self.combats.clear();
            self.live = 0;
            return Ok(if held {
                Combats::Incomplete
            } else {
                Combats::Unread
            });
        }
        let adjacent = !whole && self.read.and_then(|last| last.checked_add(1)) == Some(hour);
        self.read = Some(hour);
        if !adjacent {
            self.combats.clear();
            self.live = 0;
            let mut rows = Vec::new();
            for slot in live(state, table)? {
                let combat = combat(&columns::row(state, table, slot, self.version())?)?;
                self.hold(slot, combat, hour);
                rows.push(combat);
            }
            rows.sort_unstable_by_key(|combat| combat.id);
            return Ok(Combats::Whole(rows));
        }
        if !touched(state, table)? {
            return Ok(Combats::Same);
        }
        let changes = columns::changes(state, table)?;
        let mut deleted = Vec::with_capacity(changes.deleted.len());
        for (&slot, &key) in changes.deleted.iter().zip(&changes.deleted_keys) {
            if self
                .combats
                .get_mut(slot as usize)
                .and_then(Option::take)
                .is_some()
            {
                self.live -= 1;
            }
            deleted.push(key as u64);
        }
        let mut rows = Vec::new();
        let inserted: HashSet<u32> = changes.inserted.iter().copied().collect();
        let written: Vec<(u32, u64)> = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            .map(|(&slot, &mask)| (slot, mask))
            .chain(changes.inserted.iter().map(|&slot| (slot, u64::MAX)))
            .collect();
        let mut masks: HashMap<u32, u64> = HashMap::with_capacity(written.len());
        for (slot, mask) in written {
            *masks.entry(slot).or_default() |= mask;
        }
        for (&slot, &mask) in &masks {
            let before = (!inserted.contains(&slot))
                .then(|| self.combats.get(slot as usize).copied().flatten())
                .flatten();
            let now = self.read_combat(
                state,
                slot,
                mask & self.fields.roster != 0 || before.is_none(),
            )?;
            let stepped = before.is_some_and(|before| {
                before.combat.id == now.id
                    && before.combat.province == now.province
                    && before.combat.counts == now.counts
                    && before.combat.duration.checked_add(1) == Some(now.duration)
            });
            self.hold(slot, now, hour);
            if !stepped {
                rows.push(now);
            }
        }
        // A combat no run wrote held its counter, which the census reads as a new run.
        if masks.len() < self.live {
            for held in self.combats.iter_mut().flatten() {
                if held.touched != hour {
                    held.touched = hour;
                    rows.push(held.combat);
                }
            }
        }
        rows.sort_unstable_by_key(|combat| combat.id);
        deleted.sort_unstable();
        Ok(Combats::Changed { rows, deleted })
    }

    /// The coverage row the finalizers read: the first by key.
    fn coverage(&self, state: &State) -> io::Result<Option<(u32, LandEventCoverageRow)>> {
        let table = TableId::LandEventCoverage;
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let first = held
            .live()
            .ones()
            .filter_map(|slot| held.key(slot).map(|key| (key, slot)))
            .min();
        first
            .map(|(_, slot)| {
                columns::row::<LandEventCoverageRow>(state, table, slot, self.version())
                    .map(|row| (slot, row))
            })
            .transpose()
    }

    /// Whether any coverage row is complete and dated `hour`, as availability reads them.
    fn covered(&self, state: &State, hour: i32) -> io::Result<bool> {
        let table = TableId::LandEventCoverage;
        if !columns::complete(state, table) {
            return Ok(false);
        }
        for slot in live(state, table)? {
            let row: LandEventCoverageRow = columns::row(state, table, slot, self.version())?;
            if row.complete && row.to_hour == hour {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn results(
        &self,
        state: &State,
        read: bool,
        hour: i32,
    ) -> io::Result<(bool, Option<Box<Results>>)> {
        let table = TableId::LandCombatResults;
        if !read || !columns::complete(state, table) {
            return Ok((false, None));
        }
        let slots = live(state, table)?;
        if slots.is_empty() {
            return Ok((false, None));
        }
        let available = self.covered(state, hour)?;
        let Some((_, coverage)) = self
            .coverage(state)?
            .filter(|(_, row)| row.complete && row.to_hour == hour)
        else {
            return Ok((available, None));
        };
        let held = columns::table(state, table)?;
        let field = self.fields.end_hour;
        let mut rows = Vec::new();
        for slot in slots {
            let end = if held.recorded(field, slot) {
                Some(columns::plain::<i32>(held, field)?[slot as usize])
            } else {
                None
            };
            if end.is_some_and(|end| end <= coverage.from_hour || end > coverage.to_hour) {
                continue;
            }
            let row: LandCombatResultsRow = columns::row(state, table, slot, self.version())?;
            if row.end_hour <= coverage.from_hour || row.end_hour > coverage.to_hour {
                continue;
            }
            rows.push(row);
        }
        rows.sort_unstable_by_key(|row| row.id);
        Ok((
            available,
            Some(Box::new(Results {
                from_hour: coverage.from_hour,
                to_hour: coverage.to_hour,
                rows,
            })),
        ))
    }

    fn heap(&self) -> usize {
        self.combats.capacity() * size_of::<Option<Held>>() + self.naming.heap()
    }
}

// Every segment starts cold: its first hour reads every table again; what came before is the
// emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("land.columnar", Some("land"));
        let state = hour.state;
        let at = hour.hour;
        let whole = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if self.cold {
            self.naming = Naming::new()?;
        }
        self.cold = false;
        self.last = Some(at);
        let mut changed = [false; TABLE_COUNT];
        let mut shaped = [false; TABLE_COUNT];
        for &table in TABLES {
            let status = columns::status(state, table);
            let index = table.index();
            shaped[index] = self.shapes[index] != Some(status);
            changed[index] = whole || shaped[index] || touched(state, table)?;
            self.shapes[index] = Some(status);
        }
        let flags = metrics(hour.manifest);
        let format = self.format;
        let read = |table| slot_read(format, &flags, table, at);
        let countries = self.countries(hour.world, whole)?;
        let (names, variants) = self.naming.project(
            state,
            (hour.world.countries()?, hour.world.naming()?),
            format,
            at,
            whole,
            &shaped,
            &changed,
            read(TableId::VariantDefinitions),
        )?;
        let combats_read = read(TableId::LandCombats);
        let sides_read = read(TableId::LandCombatSides);
        let progress_read = read(TableId::LandCombatProgress);
        let results_read = read(TableId::LandCombatResults);
        let combats = self.combats(state, combats_read, at, whole)?;
        let sides = self.sides(state, whole)?;
        let progress = self.progress(state, whole)?;
        let (finalized, results) = self.results(state, results_read, at)?;
        let available = combats_read && columns::complete(state, TableId::LandCombats) || finalized;
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            available,
            countries,
            names,
            variants,
            sides_read,
            sides,
            combats,
            progress_read,
            progress,
            results,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved land projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
