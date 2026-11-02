//! Construction facts, projected hour by hour from a columnar walk ([`replay::segments`]): each
//! building table's levels (every building at a segment's first hour and where the table is
//! complete again, else the buildings whose level or healthy level the hour wrote), and the
//! queues and the output only at their slot's reads, each line's owner resolved through the
//! walk's countries ([`dashboard_world`]). What came before the segment is the ordered emit's
//! ([`super::emit`]), which runs the claim algorithm over these hours.

use super::{
    SpendingGap,
    observed::{Building, BuildingLine, Buildings, Hour, Output, OutputLine, Queue, RailwayLine},
};
use dashboard_world::{Needs, Roster, World};
use engine_native::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        BuildingConstructionRow, BuildingRow, ConstructionLineOutputRow, LexicalTokenRow,
        ProvinceRow, RailwayConstructionRow, Recorded, TableId,
    },
};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Provinces,
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::ConstructionLineOutput,
    TableId::LexicalTokens,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// The building tables, in the scan's order.
const BUILDINGS: [TableId; 2] = [TableId::StateBuildings, TableId::ProvinceBuildings];

/// BuildingConstruction's fields the scan reads, in [`Fields::queue`]'s order.
const QUEUE: [&str; 11] = [
    "country",
    "active_factories",
    "cost",
    "amount",
    "created_hour",
    "building_type",
    "target_flags",
    "target_location",
    "target_state",
    "repair_levels",
    "queue_ordinal",
];

/// ConstructionLineOutput's fields the scan reads of a line still observed, in
/// [`Fields::output`]'s order.
const OUTPUT: [&str; 9] = [
    "railway",
    "construction_raw",
    "repair_raw",
    "repair_ticks",
    "since_hour",
    "observed_since_creation",
    "linked",
    "ended",
    "exact",
];

const OUTPUT_IDENTITY: [&str; 8] = [
    "country",
    "building_type",
    "target_flags",
    "target_location",
    "target_state",
    "target_country",
    "route_first",
    "route_last",
];

/// The columns a projection reads.
struct Fields {
    /// A building's level and healthy level, and their bits in a change mask.
    level: usize,
    healthy: usize,
    levels: u64,
    queue: [usize; 11],
    province_state: usize,
    output: [usize; 9],
    output_identity: [usize; 8],
    literal: usize,
}

impl Fields {
    fn new() -> io::Result<Self> {
        let building = |name| columns::field(TableId::StateBuildings, name);
        let (level, healthy) = (building("level")?, building("healthy_level")?);
        // Both building tables hold the one row type.
        if columns::field(TableId::ProvinceBuildings, "level")? != level
            || columns::field(TableId::ProvinceBuildings, "healthy_level")? != healthy
        {
            return Err(io::Error::other(
                "The building tables do not share their columns",
            ));
        }
        let mut queue = [0; 11];
        for (at, name) in queue.iter_mut().zip(QUEUE) {
            *at = columns::field(TableId::BuildingConstruction, name)?;
        }
        let mut output = [0; 9];
        for (at, name) in output.iter_mut().zip(OUTPUT) {
            *at = columns::field(TableId::ConstructionLineOutput, name)?;
        }
        let mut output_identity = [0; 8];
        for (at, name) in output_identity.iter_mut().zip(OUTPUT_IDENTITY) {
            *at = columns::field(TableId::ConstructionLineOutput, name)?;
        }
        Ok(Self {
            level,
            healthy,
            levels: bit(level) | bit(healthy),
            queue,
            province_state: columns::field(TableId::Provinces, "state")?,
            output,
            output_identity,
            literal: columns::field(TableId::LexicalTokens, "literal")?,
        })
    }
}

/// `column`'s bit in a change mask.
fn bit(column: usize) -> u64 {
    u32::try_from(column)
        .ok()
        .and_then(|column| 1_u64.checked_shl(column))
        .unwrap_or(0)
}

/// Why the row decoder finds the output table unavailable.
fn gap(state: &State) -> io::Result<SpendingGap> {
    Ok(
        match columns::status(state, TableId::ConstructionLineOutput) {
            columns::Status::Unavailable { reason } => {
                let reason = state.dictionary().get(reason).ok_or_else(|| {
                    io::Error::other("An output reason names no dictionary entry")
                })?;
                if String::from_utf8_lossy(reason).contains("not recorded") {
                    SpendingGap::NotRecorded
                } else {
                    SpendingGap::ReadFailed
                }
            }
            columns::Status::NotRecorded => SpendingGap::NotRecorded,
            columns::Status::Truncated { .. } | columns::Status::Complete => {
                SpendingGap::ReadFailed
            }
        },
    )
}

/// A slot's key, as a row id.
fn id(key: Option<u128>) -> io::Result<u64> {
    key.map(|key| key as u64)
        .ok_or_else(|| io::Error::other("A live slot holds no row"))
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    /// Nothing is held: the next hour reads every building whole.
    cold: bool,
    last: Option<i32>,
    /// Whether each building table was complete the hour before.
    complete: [bool; 2],
    /// Each raw country's owner tag, while the countries hold.
    owners: HashMap<u32, Option<Arc<str>>>,
    /// Each building type a read or the building tables named in this segment, and whether it
    /// was spelled.
    spelled: HashMap<u32, bool>,
    /// The building types the building tables hold, met in this segment.
    present: HashSet<u32>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"construction projector: cold";

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            fields: Fields::new()?,
            cold: true,
            last: None,
            complete: [false; 2],
            owners: HashMap::new(),
            spelled: HashMap::new(),
            present: HashSet::new(),
        })
    }

    /// A building table this hour: whole where the segment opens or it is complete again, else
    /// the buildings whose levels the hour wrote.
    fn buildings(&mut self, state: &State, at: usize, opens: bool) -> io::Result<Buildings> {
        let table = BUILDINGS[at];
        let complete = columns::complete(state, table);
        let was = std::mem::replace(&mut self.complete[at], complete);
        if !complete {
            return Ok(Buildings::Incomplete);
        }
        let held = columns::table(state, table)?;
        let written_levels = self.fields.levels;
        let edits = (!opens && was)
            .then(|| columns::changes(state, table))
            .transpose()?;
        if edits.is_some_and(|edits| {
            edits.inserted.is_empty()
                && edits.deleted.is_empty()
                && edits
                    .masks
                    .iter()
                    .chain(&edits.fill_masks)
                    .all(|mask| mask & written_levels == 0)
        }) {
            return Ok(Buildings::Unchanged);
        }
        let (level, healthy) = (self.fields.level, self.fields.healthy);
        let levels = columns::plain::<i16>(held, level)?;
        let healths = columns::plain::<i16>(held, healthy)?;
        let version = self.format.version();
        let read = |slot: u32| -> io::Result<Building> {
            let key = held
                .key(slot)
                .ok_or_else(|| io::Error::other("A building slot holds no row"))?;
            let at = slot as usize;
            let cells = (held.recorded(level, slot) && held.recorded(healthy, slot))
                .then(|| levels.get(at).copied().zip(healths.get(at).copied()))
                .flatten();
            let (level, healthy) = match cells {
                Some(cells) => cells,
                None => {
                    let row: BuildingRow = columns::row(state, table, slot, version)?;
                    (row.level, row.healthy_level)
                }
            };
            Ok(Building {
                location: (key >> 32) as u32,
                kind: key as u32,
                level,
                healthy,
            })
        };
        let Some(edits) = edits else {
            let mut rows = Vec::with_capacity(held.live().count());
            for slot in held.live().ones() {
                rows.push(read(slot)?);
            }
            self.present.extend(rows.iter().map(|row| row.kind));
            return Ok(Buildings::Whole(rows));
        };
        let mut rows = Vec::new();
        // Gone first: a key a deleted slot held can be inserted again at another.
        for &key in &edits.deleted_keys {
            rows.push(Building {
                location: (key >> 32) as u32,
                kind: key as u32,
                level: 0,
                healthy: 0,
            });
        }
        let written = edits
            .changed
            .iter()
            .zip(&edits.masks)
            .chain(edits.fill_slots.iter().zip(&edits.fill_masks))
            .filter(|(_, mask)| **mask & written_levels != 0)
            .map(|(&slot, _)| slot);
        for slot in edits.inserted.iter().copied().chain(written) {
            if held.live().get(slot) {
                let row = read(slot)?;
                self.present.insert(row.kind);
                rows.push(row);
            }
        }
        Ok(if rows.is_empty() {
            Buildings::Unchanged
        } else {
            Buildings::Changed(rows)
        })
    }

    /// The tag of the country raw link `raw` names, when the links resolve it: its slot's first
    /// country by id whose own id the links resolve to that slot.
    fn owner(
        owners: &mut HashMap<u32, Option<Arc<str>>>,
        roster: &Roster,
        raw: u32,
    ) -> Option<Arc<str>> {
        if !roster.complete() {
            return None;
        }
        owners
            .entry(raw)
            .or_insert_with(|| {
                let slot = roster.slot_of(raw)?;
                let country = roster.at_slot(slot).find(|country| country.resolved)?;
                Some(Arc::from(
                    String::from_utf8_lossy(roster.tag(country.tag)).as_ref(),
                ))
            })
            .clone()
    }

    /// The building queue at its read: every line, by id.
    fn queue(&mut self, state: &State, roster: &Roster) -> io::Result<Queue<BuildingLine>> {
        let table = TableId::BuildingConstruction;
        if !columns::complete(state, table) {
            return Ok(Queue::Failed);
        }
        let held = columns::table(state, table)?;
        let fields = self.fields.queue;
        let [
            country,
            factories,
            cost,
            amount,
            created,
            kind,
            flags,
            location,
            target,
            repair,
            ordinal,
        ] = fields;
        let ordinal = columns::plain::<u32>(held, ordinal)?;
        let (country, factories, cost, amount, created) = (
            columns::plain::<u32>(held, country)?,
            columns::plain::<i32>(held, factories)?,
            columns::plain::<i64>(held, cost)?,
            columns::plain::<i32>(held, amount)?,
            columns::plain::<i32>(held, created)?,
        );
        let (kind, flags, location, target, repair) = (
            columns::plain::<u32>(held, kind)?,
            columns::plain::<u32>(held, flags)?,
            columns::plain::<u32>(held, location)?,
            columns::plain::<u32>(held, target)?,
            columns::plain::<i32>(held, repair)?,
        );
        let version = self.format.version();
        let mut lines = Vec::with_capacity(held.live().count());
        for slot in held.live().ones() {
            let id = id(held.key(slot))?;
            let at = slot as usize;
            let cells = fields
                .iter()
                .all(|&field| held.recorded(field, slot))
                .then(|| {
                    Some(BuildingConstructionRow {
                        id,
                        country: *country.get(at)?,
                        queue_ordinal: *ordinal.get(at)?,
                        active_factories: *factories.get(at)?,
                        cost: *cost.get(at)?,
                        amount: *amount.get(at)?,
                        created_hour: *created.get(at)?,
                        building_type: *kind.get(at)?,
                        target_flags: *flags.get(at)?,
                        target_location: *location.get(at)?,
                        target_state: *target.get(at)?,
                        repair_levels: *repair.get(at)?,
                        ..Default::default()
                    })
                })
                .flatten();
            let row = match cells {
                Some(row) => row,
                None => columns::row(state, table, slot, version)?,
            };
            lines.push(BuildingLine {
                owner: Self::owner(&mut self.owners, roster, row.country),
                row,
            });
        }
        lines.sort_unstable_by_key(|line| line.row.id);
        Ok(Queue::Read(lines))
    }

    /// `province`'s state in a complete Provinces table; none where it names none.
    fn province_state(&self, state: &State, province: u32) -> io::Result<Option<u32>> {
        let table = TableId::Provinces;
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let Some(slot) = held.slot(u128::from(province)) else {
            return Ok(None);
        };
        let field = self.fields.province_state;
        let cell = held
            .recorded(field, slot)
            .then(|| columns::plain::<u32>(held, field))
            .transpose()?
            .and_then(|states| states.get(slot as usize).copied());
        let owner = match cell {
            Some(owner) => owner,
            None => columns::row::<ProvinceRow>(state, table, slot, self.format.version())?.state,
        };
        Ok((owner != 0).then_some(owner))
    }

    /// The railway queue at its read: every line, by id, with its route's end states.
    fn railways(&mut self, state: &State, roster: &Roster) -> io::Result<Queue<RailwayLine>> {
        let table = TableId::RailwayConstruction;
        if !columns::complete(state, table) {
            return Ok(Queue::Failed);
        }
        let held = columns::table(state, table)?;
        let mut lines = Vec::with_capacity(held.live().count());
        for slot in held.live().ones() {
            let row: RailwayConstructionRow =
                columns::row(state, table, slot, self.format.version())?;
            let from = match row.route.first() {
                Some(&province) => self.province_state(state, province)?,
                None => None,
            };
            let to = match row.route.last() {
                Some(&province) => self.province_state(state, province)?,
                None => None,
            };
            lines.push(RailwayLine {
                owner: Self::owner(&mut self.owners, roster, row.country),
                from,
                to,
                row,
            });
        }
        lines.sort_unstable_by_key(|line| line.row.id);
        Ok(Queue::Read(lines))
    }

    /// The output table at its read: every row, by line; an ended line's is read whole, its
    /// completion with it.
    fn output(&mut self, state: &State, roster: &Roster) -> io::Result<Output> {
        let table = TableId::ConstructionLineOutput;
        if !columns::complete(state, table) {
            return Ok(Output::Unavailable(gap(state)?));
        }
        let held = columns::table(state, table)?;
        let identity_fields = self.fields.output_identity;
        let identity = if self.format.version()
            >= replay_data::rows::compatibility::CONSTRUCTION_OUTPUT_OWNER_FORMAT
        {
            Some([
                columns::optional::<u32>(held, identity_fields[0])?,
                columns::optional::<u32>(held, identity_fields[1])?,
                columns::optional::<u32>(held, identity_fields[2])?,
                columns::optional::<u32>(held, identity_fields[3])?,
                columns::optional::<u32>(held, identity_fields[4])?,
                columns::optional::<u32>(held, identity_fields[5])?,
                columns::optional::<u32>(held, identity_fields[6])?,
                columns::optional::<u32>(held, identity_fields[7])?,
            ])
        } else {
            None
        };
        let fields = self.fields.output;
        let [
            railway,
            construction,
            repair,
            ticks,
            since,
            observed,
            linked,
            ended,
            exact,
        ] = fields;
        let (railway, construction, repair, ticks, since) = (
            columns::plain::<bool>(held, railway)?,
            columns::plain::<i64>(held, construction)?,
            columns::plain::<i64>(held, repair)?,
            columns::plain::<i32>(held, ticks)?,
            columns::plain::<i32>(held, since)?,
        );
        let (observed, linked, ended, exact) = (
            columns::plain::<bool>(held, observed)?,
            columns::plain::<bool>(held, linked)?,
            columns::plain::<bool>(held, ended)?,
            columns::plain::<bool>(held, exact)?,
        );
        let version = self.format.version();
        let mut rows = Vec::with_capacity(held.live().count());
        for slot in held.live().ones() {
            let line = id(held.key(slot))?;
            let at = slot as usize;
            let cells = fields
                .iter()
                .all(|&field| held.recorded(field, slot))
                .then(|| {
                    // An ended line's completion decision is read with its whole row.
                    if *ended.get(at)? {
                        return None;
                    }
                    Some(ConstructionLineOutputRow {
                        line,
                        railway: *railway.get(at)?,
                        construction_raw: *construction.get(at)?,
                        repair_raw: *repair.get(at)?,
                        repair_ticks: *ticks.get(at)?,
                        since_hour: *since.get(at)?,
                        observed_since_creation: *observed.get(at)?,
                        linked: *linked.get(at)?,
                        exact: *exact.get(at)?,
                        ended: false,
                        ..Default::default()
                    })
                })
                .flatten();
            let mut row = match cells {
                Some(row) => row,
                None => columns::row(state, table, slot, version)?,
            };
            for ((index, field), destination) in identity_fields.iter().enumerate().zip([
                &mut row.country,
                &mut row.building_type,
                &mut row.target_flags,
                &mut row.target_location,
                &mut row.target_state,
                &mut row.target_country,
                &mut row.route_first,
                &mut row.route_last,
            ]) {
                *destination = if let Some(identity) = &identity
                    && held.recorded(*field, slot)
                {
                    let (values, present) = identity[index];
                    Recorded::Value(if present.get(slot) {
                        Some(*values.get(at).ok_or_else(|| {
                            io::Error::other("An output identity slot holds no cell")
                        })?)
                    } else {
                        None
                    })
                } else {
                    Recorded::NotRecorded
                };
            }
            let owner = match row.country {
                Recorded::Value(Some(country)) => Self::owner(&mut self.owners, roster, country),
                _ => None,
            };
            let province_state = |province| match province {
                Recorded::Value(Some(province)) => self.province_state(state, province),
                _ => Ok(None),
            };
            let from = province_state(row.route_first)?;
            let to = province_state(row.route_last)?;
            rows.push(OutputLine {
                row,
                owner,
                from,
                to,
            });
        }
        rows.sort_unstable_by_key(|line| line.row.line);
        Ok(Output::Read(rows))
    }

    /// `token`'s spelling in a complete LexicalTokens table; none where it holds none, or one
    /// that is not text.
    fn literal(&self, state: &State, token: u32) -> io::Result<Option<String>> {
        let table = TableId::LexicalTokens;
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let Some(slot) = held.slot(u128::from(token)) else {
            return Ok(None);
        };
        let field = self.fields.literal;
        let literal = if held.recorded(field, slot) {
            let text = *columns::text(held, field)?
                .get(slot as usize)
                .ok_or_else(|| io::Error::other("A token's slot holds no cell"))?;
            state
                .dictionary()
                .get(text)
                .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?
                .to_vec()
        } else {
            columns::row::<LexicalTokenRow>(state, table, slot, self.format.version())?.literal
        };
        Ok(String::from_utf8(literal)
            .ok()
            .filter(|text| !text.contains('\0')))
    }

    /// The spellings of the types the hour's reads or the building tables name for the first
    /// time in the segment, and of those a dictionary spelled none of before, where a complete
    /// one now does: a type an earlier segment's project named is spelled as soon as a
    /// dictionary can, when any building of it stands.
    fn types(
        &mut self,
        state: &State,
        world: &World,
        opens: bool,
        hour: &Hour,
    ) -> io::Result<Vec<(u32, Option<String>)>> {
        let lexical = TableId::LexicalTokens;
        let mut types = Vec::new();
        if !opens && world.touched(lexical, u64::MAX) && columns::complete(state, lexical) {
            let unspelled: Vec<u32> = self
                .spelled
                .iter()
                .filter(|(_, spelled)| !**spelled)
                .map(|(&id, _)| id)
                .collect();
            for id in unspelled {
                if let Some(token) = self.literal(state, id)? {
                    self.spelled.insert(id, true);
                    types.push((id, Some(token)));
                }
            }
        }
        let mut named: Vec<u32> = match &hour.queue {
            Queue::Read(lines) => lines.iter().map(|line| line.row.building_type).collect(),
            Queue::NotRead | Queue::Failed => Vec::new(),
        };
        if matches!(&hour.railways, Queue::Read(lines) if !lines.is_empty()) {
            named.push(RAILWAY_TEMPLATE_TOKEN);
        }
        if let Output::Read(lines) = &hour.output {
            for line in lines {
                if line.row.railway {
                    named.push(RAILWAY_TEMPLATE_TOKEN);
                } else if let Recorded::Value(Some(kind)) = line.row.building_type {
                    named.push(kind);
                }
            }
        }
        named.extend(
            self.present
                .iter()
                .copied()
                .filter(|id| !self.spelled.contains_key(id)),
        );
        named.sort_unstable();
        named.dedup();
        for id in named {
            if self.spelled.contains_key(&id) {
                continue;
            }
            let token = self.literal(state, id)?;
            self.spelled.insert(id, token.is_some());
            types.push((id, token));
        }
        Ok(types)
    }
}

// Every segment starts cold: its first hour reads every building whole; what came before is
// the emit's.
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
        let _cpu = replay_frame::cpu::scope("construction.columnar", Some("construction"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.spelled.clear();
            self.present.clear();
        }
        self.cold = false;
        self.last = Some(at);
        if opens || roster.changed() || !roster.complete() {
            self.owners.clear();
        }
        let buildings = [
            self.buildings(state, 0, opens)?,
            self.buildings(state, 1, opens)?,
        ];
        let flags = metrics(hour.manifest);
        let format = self.format;
        let read = |table| dashboard_text::metadata::slot_read(format, &flags, table, at);
        let mut observed = Hour {
            branch: hour.branch,
            hour: at,
            buildings,
            ..Hour::default()
        };
        if read(TableId::BuildingConstruction) {
            observed.queue = self.queue(state, roster)?;
        }
        if read(TableId::RailwayConstruction) {
            observed.railways = self.railways(state, roster)?;
        }
        if read(TableId::ConstructionLineOutput) {
            observed.output = self.output(state, roster)?;
        }
        observed.types = self.types(state, hour.world, opens, &observed)?;
        out.push(observed)
    }

    fn carried(&self) -> Option<usize> {
        Some(
            self.owners.capacity() * std::mem::size_of::<(u32, Option<Arc<str>>)>()
                + self.spelled.capacity() * std::mem::size_of::<(u32, bool)>()
                + self.present.capacity() * std::mem::size_of::<u32>(),
        )
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved construction projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
