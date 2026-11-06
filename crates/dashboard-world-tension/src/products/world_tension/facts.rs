//! World tension facts, projected hour by hour from a columnar walk ([`replay::segments`]): the
//! slot each country's tag names and every link ([`dashboard_world`]), the world tension, the
//! nonzero country contributions and the threat sources, each where it changed, and whether the
//! tension tables were read. What came before the segment is the ordered emit's
//! ([`super::emit`]). An hour that changes none of them carries nothing but its hour and read.

use dashboard_world::{Needs, Roster, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{TableId, TableRows, ThreatCountryAggregateRow, ThreatSourceRow, WorldTensionRow},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WorldTension,
    TableId::ThreatSources,
    TableId::ThreatCountryAggregates,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// The complete Countries and CountryLinks: each canonical slot's tag, as its last country by
/// id the links resolve holds it, and every link (raw id, slot), by raw id.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct Identity {
    pub tags: Vec<(u32, Arc<str>)>,
    pub links: Vec<(u32, u32)>,
}

impl Identity {
    fn read(roster: &Roster) -> Self {
        let mut tags = BTreeMap::new();
        for country in roster.iter().filter(|country| country.resolved) {
            tags.insert(
                country.slot,
                Arc::from(String::from_utf8_lossy(roster.tag(country.tag))),
            );
        }
        Self {
            tags: tags.into_iter().collect(),
            links: roster
                .links()
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect(),
        }
    }

    fn heap(&self) -> usize {
        size_of::<Self>()
            + self.tags.capacity() * size_of::<(u32, Arc<str>)>()
            + self.tags.iter().map(|(_, tag)| tag.len()).sum::<usize>()
            + self.links.capacity() * 8
    }
}

/// The WorldTension row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct Tension {
    pub total_raw: i64,
    pub source_count: i32,
}

/// A threat source's event as its row records it: every field but its current contribution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct Source {
    pub country: u32,
    pub second_country: u32,
    pub date: i32,
    pub reason: Box<[u8]>,
    pub final_raw: i64,
}

/// The threat sources a read holds, in stored order: every row, or the rows a truncated table
/// read; their current contributions' sum, none past 64 bits; and whether the events they name
/// changed since the last sent, which shares them otherwise.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct Sources {
    pub rows: Arc<[Source]>,
    pub complete: bool,
    pub sum: Option<i64>,
    pub regrouped: bool,
}

/// What the ordered emit reads of an hour that changes what the tension reads. Each part is
/// none where it did not change, and holds none while its tables are not complete.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub struct Facts {
    pub identity: Option<Option<Arc<Identity>>>,
    /// Whether this hour's item is the first to carry `identity`, which it is charged for.
    pub charged: bool,
    /// The WorldTension row; none in a complete table that holds none.
    pub tension: Option<Option<Option<Tension>>>,
    /// The nonzero aggregates (canonical slot, total), by slot.
    pub aggregates: Option<Option<Arc<[(u32, i64)]>>>,
    /// None while the table is unavailable or not recorded.
    pub sources: Option<Option<Sources>>,
}

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether the tension tables were read (`slot_read`).
    pub read: bool,
    /// None when the hour changes nothing the tension reads.
    pub facts: Option<Box<Facts>>,
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    total: usize,
    current: usize,
    /// The ThreatSources fields an event reads: every one but the current and daily ones.
    events: u64,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    /// Whether Countries and CountryLinks were complete, and what they named.
    roster: bool,
    identity: Option<Arc<Identity>>,
    uncharged: bool,
    /// The sources last sent, and each one's current contribution and slot's place among them.
    sources: Option<Arc<[Source]>>,
    currents: Vec<i64>,
    places: HashMap<u32, usize>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"tension projector: cold";

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        let table = TableId::ThreatSources;
        let current = columns::field(table, "current_raw")?;
        let daily = columns::field(table, "daily_raw")?;
        Ok(Self {
            format,
            total: columns::field(TableId::ThreatCountryAggregates, "total_raw")?,
            current,
            events: !(1_u64 << current | 1_u64 << daily),
            cold: true,
            last: None,
            roster: false,
            identity: None,
            uncharged: true,
            sources: None,
            currents: Vec::new(),
            places: HashMap::new(),
        })
    }

    /// The live slots of `table`, by key.
    fn live(state: &State, table: TableId) -> io::Result<Vec<(u128, u32)>> {
        let held = columns::table(state, table)?;
        let mut keyed = held
            .live()
            .ones()
            .map(|slot| {
                held.key(slot)
                    .map(|key| (key, slot))
                    .ok_or_else(|| io::Error::other(format!("A {table:?} slot holds no row")))
            })
            .collect::<io::Result<Vec<_>>>()?;
        keyed.sort_unstable();
        Ok(keyed)
    }

    fn tension(&self, state: &State) -> io::Result<Option<Option<Tension>>> {
        let table = TableId::WorldTension;
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let Some(&(_, slot)) = Self::live(state, table)?.first() else {
            return Ok(Some(None));
        };
        let row: WorldTensionRow = columns::row(state, table, slot, self.format.version())?;
        Ok(Some(Some(Tension {
            total_raw: row.total_raw,
            source_count: row.source_count,
        })))
    }

    fn aggregates(&self, state: &State) -> io::Result<Option<Arc<[(u32, i64)]>>> {
        let table = TableId::ThreatCountryAggregates;
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let mut nonzero = Vec::new();
        for (key, slot) in Self::live(state, table)? {
            let total = if held.recorded(self.total, slot) {
                columns::plain::<i64>(held, self.total)?[slot as usize]
            } else {
                let row: ThreatCountryAggregateRow =
                    columns::row(state, table, slot, self.format.version())?;
                row.total_raw
            };
            if total != 0 {
                nonzero.push((key as u32, total));
            }
        }
        Ok(Some(nonzero.into()))
    }

    /// The current contribution at `slot` of the sources' table.
    fn current(&self, state: &State, slot: u32) -> io::Result<i64> {
        let table = TableId::ThreatSources;
        let held = columns::table(state, table)?;
        if held.recorded(self.current, slot) {
            return Ok(columns::plain::<i64>(held, self.current)?[slot as usize]);
        }
        let row: ThreatSourceRow = columns::row(state, table, slot, self.format.version())?;
        Ok(row.current_raw)
    }

    /// Takes the hour's current contributions into the sources sent last, where the hour
    /// changed nothing else of the complete table they were read whole from; false otherwise.
    fn patched(&mut self, state: &State, restated: bool) -> io::Result<bool> {
        let table = TableId::ThreatSources;
        // A truncated read placed no slot, and is read again whole.
        if restated || self.sources.is_none() || self.places.len() != self.currents.len() {
            return Ok(false);
        }
        let changes = columns::changes(state, table)?;
        let written = || {
            changes
                .changed
                .iter()
                .zip(&changes.masks)
                .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        };
        if changes.truncated
            || !changes.inserted.is_empty()
            || !changes.deleted.is_empty()
            || written().any(|(_, &mask)| mask & self.events != 0)
        {
            return Ok(false);
        }
        let current = 1_u64 << self.current;
        let mut updates = Vec::new();
        for (&slot, _) in written().filter(|&(_, &mask)| mask & current != 0) {
            let Some(&place) = self.places.get(&slot) else {
                return Ok(false);
            };
            updates.push((place, self.current(state, slot)?));
        }
        for (place, value) in updates {
            self.currents[place] = value;
        }
        Ok(true)
    }

    /// The sources the table holds: their events shared with the last sent unless the hour
    /// changed one.
    fn sources(&mut self, state: &State, restated: bool) -> io::Result<Option<Sources>> {
        let table = TableId::ThreatSources;
        let complete = match columns::status(state, table) {
            columns::Status::Complete => true,
            columns::Status::Truncated { .. } => false,
            columns::Status::Unavailable { .. } | columns::Status::NotRecorded => {
                self.sources = None;
                self.currents.clear();
                self.places.clear();
                return Ok(None);
            }
        };
        let regrouped = if complete && self.patched(state, restated)? {
            false
        } else {
            let slots: Vec<u32> = if complete {
                Self::live(state, table)?
                    .into_iter()
                    .map(|(_, slot)| slot)
                    .collect()
            } else {
                let held = columns::table(state, table)?;
                let mut keyed = columns::truncated(state, table)
                    .unwrap_or_default()
                    .iter()
                    .map(|&slot| {
                        held.key(slot)
                            .map(|key| (key, slot))
                            .ok_or_else(|| io::Error::other("A ThreatSources slot holds no row"))
                    })
                    .collect::<io::Result<Vec<_>>>()?;
                keyed.sort_unstable();
                keyed.into_iter().map(|(_, slot)| slot).collect()
            };
            let TableRows::ThreatSources(read) =
                columns::rows_at(state, table, &slots, self.format.version())?
            else {
                return Err(io::Error::other("ThreatSources rows of another table"));
            };
            self.currents = read.iter().map(|row| row.current_raw).collect();
            let rows: Arc<[Source]> = read
                .into_iter()
                .map(|row| Source {
                    country: row.country,
                    second_country: row.second_country,
                    date: row.date,
                    reason: row.reason.into_boxed_slice(),
                    final_raw: row.final_raw,
                })
                .collect();
            // A truncated read is never patched: its slots are only those it read.
            self.places = if complete {
                slots
                    .iter()
                    .enumerate()
                    .map(|(place, &slot)| (slot, place))
                    .collect()
            } else {
                HashMap::new()
            };
            let regrouped = self.sources.as_deref() != Some(&rows[..]);
            if regrouped {
                self.sources = Some(rows);
            }
            regrouped
        };
        let sum = self
            .currents
            .iter()
            .try_fold(0_i64, |sum, &current| sum.checked_add(current));
        let rows = self
            .sources
            .clone()
            .ok_or_else(|| io::Error::other("Threat sources read without rows"))?;
        Ok(Some(Sources {
            rows,
            complete,
            sum,
            regrouped,
        }))
    }

    fn heap(&self) -> usize {
        self.identity.as_ref().map_or(0, |identity| identity.heap())
            + self.sources.as_ref().map_or(0, |rows| {
                rows.len() * size_of::<Source>() + rows.iter().map(|row| row.reason.len()).sum::<usize>()
            })
            + self.currents.capacity() * 8
            + self.places.capacity() * size_of::<(u32, usize)>()
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
        let _cpu = replay_frame::cpu::scope("tension.columnar", Some("tension"));
        let state = hour.state;
        let at = hour.hour;
        let world = hour.world;
        let roster = world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.sources = None;
            self.currents.clear();
            self.places.clear();
        }
        self.cold = false;
        self.last = Some(at);
        let mut facts = Facts::default();
        let complete = roster.complete();
        if opens || complete != self.roster || roster.changed() {
            let identity = complete.then(|| Identity::read(roster));
            let same = match (&identity, &self.identity) {
                (Some(now), Some(held)) => complete == self.roster && **held == *now,
                (None, _) => !self.roster,
                _ => false,
            };
            if opens || !same {
                self.identity = identity.map(Arc::new);
                self.uncharged = true;
                facts.identity = Some(self.identity.clone());
            }
            self.roster = complete;
        }
        let touched = |table| opens || world.touched(table, u64::MAX);
        if touched(TableId::WorldTension) {
            facts.tension = Some(self.tension(state)?);
        }
        if touched(TableId::ThreatCountryAggregates) {
            facts.aggregates = Some(self.aggregates(state)?);
        }
        if touched(TableId::ThreatSources) {
            let restated = opens || world.restated(TableId::ThreatSources);
            facts.sources = Some(self.sources(state, restated)?);
        }
        let flags = metrics(hour.manifest);
        let read =
            dashboard_text::metadata::slot_read(self.format, &flags, TableId::WorldTension, at);
        let quiet = facts.identity.is_none()
            && facts.tension.is_none()
            && facts.aggregates.is_none()
            && facts.sources.is_none();
        let facts = (!quiet).then(|| {
            facts.charged =
                matches!(facts.identity, Some(Some(_))) && std::mem::take(&mut self.uncharged);
            Box::new(facts)
        });
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            read,
            facts,
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
                "Not a saved tension projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
