//! Fuel facts, projected hour by hour from a columnar walk ([`replay::segments`]): each tag
//! whose country, state ownership, fuel row or read the hour may have changed, with what the
//! hour shows of it, from the walk's countries and owners ([`dashboard_world`]) and the fuel
//! and demand rows. A read hour, an hour the fuel table changes shape and a segment's first
//! hour project every tag; a quiet hour projects none.

use super::*;
use dashboard_world::{Needs, Roster, Tagged, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{FuelDemandPartRow, RowCodec, TABLE_COUNT, TableRows},
};
use replay_frame::frame::TableView;
use std::{collections::HashMap, mem::size_of};

/// What a projection reads of its walk's world: the countries and the states' owners.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        owners: true,
        ..Needs::default()
    }
}

pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::States,
    TableId::CountryFuel,
    TableId::FuelDemandParts,
];

/// What an hour shows of a tag's country, for the fuel update the hour governs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub enum Sample {
    /// The country owns a state.
    Owns(Identity),
    /// The country owns none.
    Stateless(Identity),
    /// No country holds the tag.
    Absent,
    /// The tables do not tell.
    Unknown,
}

/// One tag at a projected hour.
#[derive(replay_spool::Spooled)]
pub struct Country {
    /// Its tag's index in the projection ([`Hour::named`]).
    pub(super) index: u32,
    /// The country holding it, when the links resolve one.
    pub(super) identity: Option<Identity>,
    /// Whether a country holds it.
    pub(super) present: bool,
    pub(super) seen: Sample,
    /// Its fuel row's level and capacity, where the hour may have changed them: None without
    /// a row.
    pub(super) level: Option<Option<(i64, i64)>>,
    /// On a read hour, what its row supplies.
    pub(super) supplied: Option<Box<Supplied>>,
}

/// What a read hour supplies of a tag's row: its read and current consumption, before the
/// day's rules, and what the detail products read of it.
#[derive(replay_spool::Spooled)]
pub struct Supplied {
    pub(super) read: RawRead,
    pub(super) consumption: demand::RawConsumption,
    /// Its applied day's recipients ([`pack::recipients`]), where the day is not refused.
    pub(super) recipients: Option<Vec<u8>>,
    /// Its demand pack; None where it is the one the projection last supplied for the tag.
    pub(super) pack: Option<Handed>,
}

/// A demand pack as the projection hands it on.
#[derive(replay_spool::Spooled)]
pub(super) enum Handed {
    Whole(Arc<pack::Encoded>),
    /// The delta ([`pack::delta`]) from the pack the projection last supplied for the tag.
    Delta(Vec<u8>),
}

impl Supplied {
    /// Counts the pack whole: the projection's cache shares it only until the row changes.
    fn heap(&self) -> usize {
        read_heap(&self.read)
            + self.recipients.as_ref().map_or(0, Vec::capacity)
            + match &self.pack {
                None => 0,
                Some(Handed::Whole(pack)) => size_of::<pack::Encoded>() + pack.heap(),
                Some(Handed::Delta(bytes)) => bytes.capacity(),
            }
    }
}

/// One projected hour.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Tags the projection numbers from this hour on, by index.
    pub(super) named: Vec<(u32, Arc<str>)>,
    /// Every tag a country holds, where the countries may have changed.
    pub(super) present: Option<Vec<u32>>,
    /// Tags the hour may have changed, at least each one it did, in tag order.
    pub(super) countries: Vec<Country>,
    /// What the hour shows of a tag no country holds.
    pub(super) prior: Sample,
    /// Countries whose tags are not UTF-8, where the countries may have changed.
    pub(super) unnamed: Vec<u32>,
    /// Whether the hour reads the fuel table, and whether it holds every row.
    pub(super) read: bool,
    pub(super) available: bool,
}

impl Hour {
    pub fn changed_countries(&self) -> usize {
        self.countries.len()
    }

    /// A test's hour of one tag, `tag`, its country `identity` owning a state and its fuel read
    /// at `level` (fuel, capacity); `first` names the tag, as a segment's first hour does.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_read(
        branch: u64,
        hour: i32,
        tag: &str,
        identity: (u32, u32),
        first: bool,
        level: (i64, i64),
    ) -> Self {
        let row = CountryFuelRow {
            country: identity.1,
            current_raw: level.0,
            maximum_raw: level.1,
            ..Default::default()
        };
        let mut reasons = Reasons::default();
        let read = read_of(&row, hour, false, None, &mut reasons);
        let (consumption, demand) = demand::summary_of(&row, hour, hour, false, &mut reasons);
        let pack = Arc::new(pack::Encoded::of(&demand).expect("a test pack encodes"));
        Self {
            branch,
            hour,
            named: if first {
                vec![(0, Arc::from(tag))]
            } else {
                Vec::new()
            },
            present: first.then(|| vec![0]),
            countries: vec![Country {
                index: 0,
                identity: Some(identity),
                present: true,
                seen: Sample::Owns(identity),
                level: Some(Some(level)),
                supplied: Some(Box::new(Supplied {
                    read,
                    consumption,
                    recipients: recipients_of_day(&row, hour),
                    pack: Some(Handed::Whole(pack)),
                })),
            }],
            prior: Sample::Absent,
            unnamed: Vec::new(),
            read: true,
            available: true,
        }
    }
}

fn read_heap(read: &RawRead) -> usize {
    match &read.day {
        Some(RawDay::Awards(_)) => size_of::<AwardSums>(),
        _ => 0,
    }
}

struct Cells<T> {
    rows: BTreeMap<u128, T>,
}

impl<T> Default for Cells<T> {
    fn default() -> Self {
        Self {
            rows: BTreeMap::new(),
        }
    }
}

impl<T: Clone> Cells<T> {
    fn update(
        &mut self,
        state: &State,
        table: TableId,
        whole: bool,
        mut decode: impl FnMut(u32) -> io::Result<T>,
    ) -> io::Result<Vec<(u128, Option<T>, Option<T>)>> {
        let held = columns::table(state, table)?;
        let edits = columns::changes(state, table)?;
        let mut changed = Vec::new();
        if whole || !columns::complete(state, table) {
            let mut next = BTreeMap::new();
            if columns::complete(state, table) {
                for slot in held.live().ones() {
                    if let Some(key) = held.key(slot) {
                        next.insert(key, decode(slot)?);
                    }
                }
            }
            for (key, old) in std::mem::take(&mut self.rows) {
                changed.push((key, Some(old), None));
            }
            for (&key, now) in &next {
                changed.push((key, None, Some(now.clone())));
            }
            self.rows = next;
        } else {
            for &key in &edits.deleted_keys {
                if let Some(old) = self.rows.remove(&key) {
                    changed.push((key, Some(old), None));
                }
            }
            let slots: BTreeSet<_> = edits
                .changed
                .iter()
                .chain(&edits.fill_slots)
                .chain(&edits.inserted)
                .copied()
                .collect();
            for slot in slots {
                if let Some(key) = held.key(slot) {
                    let now = decode(slot)?;
                    let old = self.rows.insert(key, now.clone());
                    changed.push((key, old, Some(now)));
                }
            }
        }
        Ok(changed)
    }
}

/// A row's current consumption, computed once while its row and parts stay the same.
struct Summary {
    row: Arc<CountryFuelRow>,
    status: columns::Status,
    hour: i32,
    consumption: demand::RawConsumption,
    pack: Arc<pack::Encoded>,
}

/// `row`'s applied-day recipients at a read at `hour`, encoded, where the day is not refused.
fn recipients_of_day(row: &CountryFuelRow, hour: i32) -> Option<Vec<u8>> {
    awards_of(row, hour).ok()?;
    let replay_data::rows::Recorded::Value(day) = &row.award_day else {
        return None;
    };
    let mut out = Vec::new();
    pack::recipients(&mut out, &day.recipients);
    Some(out)
}

/// A tag the countries hold: its index and the country holding it.
#[derive(Clone)]
struct Held {
    tag: Arc<str>,
    index: u32,
    identity: Option<Identity>,
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    frontier: Option<i32>,
    /// Each tag's index, in the order the projection first named it.
    numbers: HashMap<Arc<str>, u32>,
    names: Vec<Arc<str>>,
    /// The tags the countries hold, in tag order, and each one's place by index.
    held: Vec<Held>,
    places: HashMap<u32, usize>,
    /// Each canonical slot's held tags, by place.
    slots: HashMap<u32, Vec<usize>>,
    /// Whether the owners were valid the hour before, and some state's owner unresolved.
    owned: (bool, bool),
    fuel: Cells<Arc<CountryFuelRow>>,
    by_slot: HashMap<u32, BTreeMap<u128, Arc<CountryFuelRow>>>,
    parts: Cells<Arc<FuelDemandPartRow>>,
    by_country: HashMap<u32, BTreeMap<u128, Arc<FuelDemandPartRow>>>,
    reasons: Reasons,
    summaries: HashMap<u32, Summary>,
    /// The pack the projection last supplied for each tag, by index, while it supplied one at
    /// every read hour since.
    packs: HashMap<u32, Arc<pack::Encoded>>,
}

impl Projector {
    pub fn new(format: FormatVersion) -> Self {
        Self {
            format,
            shapes: [None; TABLE_COUNT],
            frontier: None,
            numbers: HashMap::new(),
            names: Vec::new(),
            held: Vec::new(),
            places: HashMap::new(),
            slots: HashMap::new(),
            owned: (false, false),
            fuel: Cells::default(),
            by_slot: HashMap::new(),
            parts: Cells::default(),
            by_country: HashMap::new(),
            reasons: Reasons::default(),
            summaries: HashMap::new(),
            packs: HashMap::new(),
        }
    }

    /// `tag`'s index, named in `named` when it is new to the projection.
    fn number(&mut self, tag: &str, named: &mut Vec<(u32, Arc<str>)>) -> u32 {
        if let Some(&index) = self.numbers.get(tag) {
            return index;
        }
        let tag: Arc<str> = Arc::from(tag);
        let index = self.names.len() as u32;
        self.numbers.insert(Arc::clone(&tag), index);
        self.names.push(Arc::clone(&tag));
        named.push((index, tag));
        index
    }

    fn index_fuel(
        &mut self,
        countries: &Roster,
        (key, row): (u128, Arc<CountryFuelRow>),
        add: bool,
        reached: &mut BTreeSet<u32>,
    ) {
        if let Some(slot) = countries.slot_of(row.country) {
            reached.insert(slot);
            if add {
                self.by_slot.entry(slot).or_default().insert(key, row);
            } else if let Some(rows) = self.by_slot.get_mut(&slot) {
                rows.remove(&key);
                if rows.is_empty() {
                    self.by_slot.remove(&slot);
                }
            }
        }
    }

    /// `pack` as handed on for tag `index`: nothing where it is the pack the projection last
    /// supplied for the tag, the delta from that pack where both hold lists, else whole.
    fn changed(&mut self, index: u32, pack: Arc<pack::Encoded>) -> io::Result<Option<Handed>> {
        let handed = match self.packs.get(&index) {
            Some(last) if Arc::ptr_eq(last, &pack) || **last == *pack => return Ok(None),
            Some(last)
                if matches!(
                    (&**last, &*pack),
                    (pack::Encoded::Pack { .. }, pack::Encoded::Pack { .. })
                ) =>
            {
                let mut bytes = Vec::new();
                pack::delta(last, &pack, &mut bytes)?;
                Handed::Delta(bytes)
            }
            _ => Handed::Whole(Arc::clone(&pack)),
        };
        self.packs.insert(index, pack);
        Ok(Some(handed))
    }

    fn supplied(
        &mut self,
        row: &Arc<CountryFuelRow>,
        state: &State,
        hour: i32,
        index: u32,
    ) -> io::Result<Supplied> {
        let status = columns::status(state, TableId::FuelDemandParts);
        let read = read_of(row, hour, false, None, &mut self.reasons);
        let recipients = recipients_of_day(row, hour);
        // A row whose level moved holds the same pack while its demand and parts do not move.
        if let Some(cached) = self.summaries.get(&row.country)
            && (Arc::ptr_eq(row, &cached.row)
                || (row.demand == cached.row.demand
                    && row.country_instance == cached.row.country_instance))
            && cached.status == status
            && (cached.hour == hour
                || matches!(&row.demand,
                replay_data::rows::Recorded::Value(pack)
                    if pack.hour == replay_data::rows::FuelDemandHour::Host))
        {
            let mut consumption = cached.consumption.clone();
            consumption.first_hour = hour;
            consumption.last_hour = hour;
            let pack = Arc::clone(&cached.pack);
            return Ok(Supplied {
                read,
                consumption,
                recipients,
                pack: self.changed(index, pack)?,
            });
        }
        let parts = self
            .by_country
            .get(&row.country)
            .map(|rows| {
                rows.values()
                    .map(|row| row.as_ref().try_clone())
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        let view = match status {
            columns::Status::Complete => TableView::Complete(TableRows::FuelDemandParts(parts)),
            columns::Status::Truncated { .. } => TableView::Truncated {
                rows: TableRows::FuelDemandParts(Vec::new()),
                missing: 1,
            },
            columns::Status::Unavailable { .. } => TableView::Unavailable {
                reason: String::new(),
            },
            _ => TableView::NotRecorded {
                mode: replay_data::scope::CaptureMode::Full,
            },
        };
        let rows = replay_frame::rows::with_demand_parts(
            std::slice::from_ref(row.as_ref()),
            &view,
            self.format,
        )?;
        let (consumption, demand) =
            demand::summary_of(&rows[0], hour, hour, false, &mut self.reasons);
        let pack = Arc::new(pack::Encoded::of(&demand)?);
        self.summaries.insert(
            row.country,
            Summary {
                row: Arc::clone(row),
                status,
                hour,
                consumption: consumption.clone(),
                pack: Arc::clone(&pack),
            },
        );
        Ok(Supplied {
            read,
            consumption,
            recipients,
            pack: self.changed(index, pack)?,
        })
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let state = hour.state;
        let (countries, owners) = (hour.world.countries()?, hour.world.owners()?);
        let continuous = self.frontier.and_then(|at| at.checked_add(1)) == Some(hour.hour);
        let mut changed = [false; TABLE_COUNT];
        let mut whole = [false; TABLE_COUNT];
        for &table in TABLES {
            let index = table.index();
            let status = columns::status(state, table);
            let edits = columns::changes(state, table)?;
            whole[index] =
                hour.starts || hour.opens || !continuous || self.shapes[index] != Some(status);
            changed[index] = whole[index]
                || edits.truncated
                || !edits.changed.is_empty()
                || !edits.fill_slots.is_empty()
                || !edits.inserted.is_empty()
                || !edits.deleted.is_empty();
            self.shapes[index] = Some(status);
        }
        let identities_changed = whole[TableId::Countries.index()]
            || changed[TableId::CountryLinks.index()]
            || countries.changed();
        // Indices of the tags the hour reaches.
        let mut reached = BTreeSet::new();
        let mut named = Vec::new();
        let mut unnamed = Vec::new();
        let mut present = None;
        if identities_changed {
            reached.extend(self.held.iter().map(|held| held.index));
            self.held.clear();
            self.places.clear();
            self.slots.clear();
            // Each tag's first holder by id whose identity the links resolve.
            for tag in countries.held().filter(|_| countries.complete()) {
                let identity = match countries.tagged(tag) {
                    Tagged::Found(country) => Some((country.slot, country.id)),
                    Tagged::Absent | Tagged::Unresolved => None,
                };
                match std::str::from_utf8(countries.tag(tag)) {
                    Ok(text) => {
                        let index = self.number(text, &mut named);
                        self.held.push(Held {
                            tag: Arc::clone(&self.names[index as usize]),
                            index,
                            identity,
                        });
                        reached.insert(index);
                    }
                    Err(_) => {
                        if let Some((_, id)) = identity {
                            unnamed.push(id);
                        }
                    }
                }
            }
            self.held
                .sort_unstable_by(|left, right| left.tag.as_bytes().cmp(right.tag.as_bytes()));
            for (place, held) in self.held.iter().enumerate() {
                self.places.insert(held.index, place);
                if let Some((slot, _)) = held.identity {
                    self.slots.entry(slot).or_default().push(place);
                }
            }
            present = Some(self.held.iter().map(|held| held.index).collect());
        }
        let owner_valid = owners.complete() && countries.complete();
        let unresolved = owners.unresolved() != 0;
        if std::mem::replace(&mut self.owned, (owner_valid, unresolved))
            != (owner_valid, unresolved)
        {
            reached.extend(self.held.iter().map(|held| held.index));
        } else {
            for slot in owners.reached() {
                for &place in self.slots.get(slot).into_iter().flatten() {
                    reached.insert(self.held[place].index);
                }
            }
        }
        let mut fuel_reached = BTreeSet::new();
        if changed[TableId::CountryFuel.index()] {
            let edits = self.fuel.update(
                state,
                TableId::CountryFuel,
                whole[TableId::CountryFuel.index()],
                |slot| {
                    columns::row(state, TableId::CountryFuel, slot, self.format.version())
                        .map(Arc::new)
                },
            )?;
            // An updated row keeps its summary: its pack is reused while its demand is unchanged.
            for (_, old, now) in &edits {
                if let (Some(old), None) = (old, now) {
                    self.summaries.remove(&old.country);
                }
            }
            if !identities_changed {
                for (key, old, now) in edits {
                    if let Some(old) = old {
                        self.index_fuel(countries, (key, old), false, &mut fuel_reached);
                    }
                    if let Some(now) = now {
                        self.index_fuel(countries, (key, now), true, &mut fuel_reached);
                    }
                }
            }
        }
        if identities_changed {
            self.by_slot.clear();
            let fuel: Vec<_> = self
                .fuel
                .rows
                .iter()
                .map(|(&key, row)| (key, Arc::clone(row)))
                .collect();
            for row in fuel {
                self.index_fuel(countries, row, true, &mut fuel_reached);
            }
        }
        for slot in &fuel_reached {
            for &place in self.slots.get(slot).into_iter().flatten() {
                reached.insert(self.held[place].index);
            }
        }
        if changed[TableId::FuelDemandParts.index()] {
            let edits = self.parts.update(
                state,
                TableId::FuelDemandParts,
                whole[TableId::FuelDemandParts.index()],
                |slot| {
                    columns::row(state, TableId::FuelDemandParts, slot, self.format.version())
                        .map(Arc::new)
                },
            )?;
            for (key, old, now) in edits {
                if let Some(old) = &old {
                    self.summaries.remove(&old.country);
                }
                if let Some(now) = &now {
                    self.summaries.remove(&now.country);
                }
                if let Some(old) = old
                    && let Some(rows) = self.by_country.get_mut(&old.country)
                {
                    rows.remove(&key);
                    if rows.is_empty() {
                        self.by_country.remove(&old.country);
                    }
                }
                if let Some(now) = now {
                    self.by_country
                        .entry(now.country)
                        .or_default()
                        .insert(key, now);
                }
            }
        }
        let read = slot_read(
            self.format,
            &metrics(hour.manifest),
            TableId::CountryFuel,
            hour.hour,
        );
        let available = columns::complete(state, TableId::CountryFuel);
        if read || whole[TableId::CountryFuel.index()] || !continuous {
            reached.extend(self.held.iter().map(|held| held.index));
        }
        let mut order: Vec<u32> = reached.into_iter().collect();
        order.sort_unstable_by(|&left, &right| {
            self.names[left as usize]
                .as_bytes()
                .cmp(self.names[right as usize].as_bytes())
        });
        let mut projected = Vec::with_capacity(order.len());
        for index in order {
            let place = self.places.get(&index).copied();
            let present = place.is_some();
            let identity = place.and_then(|place| self.held[place].identity);
            let seen = if owner_valid {
                match identity {
                    Some(identity) if owners.owned(identity.0) > 0 => Sample::Owns(identity),
                    Some(identity) if !unresolved => Sample::Stateless(identity),
                    None if !present => Sample::Absent,
                    _ => Sample::Unknown,
                }
            } else {
                Sample::Unknown
            };
            let row = identity
                .and_then(|(slot, _)| self.by_slot.get(&slot))
                .and_then(|rows| rows.first_key_value())
                .map(|(_, row)| Arc::clone(row));
            let sample = present
                && (identities_changed
                    || changed[TableId::CountryLinks.index()]
                    || identity.is_some_and(|(slot, _)| fuel_reached.contains(&slot))
                    || (read && available)
                    || whole[TableId::CountryFuel.index()]);
            let supplied = if read {
                row.as_ref()
                    .map(|row| self.supplied(row, state, hour.hour, index).map(Box::new))
                    .transpose()?
            } else {
                None
            };
            projected.push(Country {
                index,
                identity,
                present,
                seen,
                level: sample.then(|| row.as_ref().map(|row| (row.current_raw, row.maximum_raw))),
                supplied,
            });
        }
        // A tag a read hour supplies nothing loses its pack, as the emit's chain does.
        if read {
            let supplied: BTreeSet<u32> = projected
                .iter()
                .filter(|country| country.supplied.is_some())
                .map(|country| country.index)
                .collect();
            self.packs.retain(|index, _| supplied.contains(index));
        }
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            named,
            present,
            countries: projected,
            prior: if owner_valid {
                Sample::Absent
            } else {
                Sample::Unknown
            },
            unnamed,
            read,
            available,
        })?;
        self.frontier = Some(hour.hour);
        Ok(())
    }
}
