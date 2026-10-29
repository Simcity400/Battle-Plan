use super::*;
use replay::{
    columnar_frames::{Views, metrics},
    columns,
    container::Replay,
    segments::{self, Operator, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{FuelDemandPartRow, RowCodec, TABLE_COUNT, TableRows},
};
use replay_frame::frame::TableView;
use std::{any::Any, collections::HashMap};

pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::States,
    TableId::CountryFuel,
    TableId::FuelDemandParts,
];

pub(super) struct Country {
    pub(super) tag: String,
    pub(super) identity: Option<Identity>,
    pub(super) present: bool,
    pub(super) seen: Holding,
    pub(super) sample: bool,
    pub(super) amount: Option<(i64, i64)>,
    pub(super) supplied: Option<(RawRead, demand::RawConsumption)>,
}

pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(super) format: FormatVersion,
    pub(super) countries: Vec<Country>,
    pub(super) identities: bool,
    pub(super) prior: Holding,
    pub(super) unnamed: BTreeSet<u32>,
    pub(super) read: bool,
    pub(super) available: bool,
    pub(super) refresh_levels: bool,
    pub(super) countries_complete: bool,
}

impl Hour {
    pub fn changed_countries(&self) -> usize {
        self.countries.len()
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
        fields: Option<u64>,
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
                .zip(&edits.masks)
                .chain(edits.fill_slots.iter().zip(&edits.fill_masks))
                .filter(|(_, mask)| fields.is_none_or(|fields| **mask & fields != 0))
                .map(|(&slot, _)| slot)
                .chain(edits.inserted.iter().copied())
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

struct Summary {
    row: Arc<CountryFuelRow>,
    status: columns::Status,
    hour: i32,
    consumption: demand::RawConsumption,
}

pub struct Projector {
    format: FormatVersion,
    naming: Views,
    country_fields: [usize; 2],
    owner_field: usize,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    frontier: Option<i32>,
    identities: BTreeMap<String, Option<Identity>>,
    tags: BTreeMap<u32, BTreeSet<String>>,
    slots: HashMap<u32, u32>,
    countries_complete: bool,
    links_complete: bool,
    states_complete: bool,
    owners: Cells<u32>,
    owner_counts: BTreeMap<u32, usize>,
    unresolved: usize,
    fuel: Cells<Arc<CountryFuelRow>>,
    by_slot: BTreeMap<u32, BTreeMap<u128, Arc<CountryFuelRow>>>,
    parts: Cells<Arc<FuelDemandPartRow>>,
    by_country: BTreeMap<u32, BTreeMap<u128, Arc<FuelDemandPartRow>>>,
    reasons: Reasons,
    summaries: BTreeMap<u32, Summary>,
    hours: Vec<Hour>,
}

impl Projector {
    pub fn new(replay: &Replay) -> io::Result<Self> {
        Self::with_format(replay.format, replay.capture_mode)
    }

    pub fn with_format(
        format: FormatVersion,
        mode: replay_data::scope::CaptureMode,
    ) -> io::Result<Self> {
        Ok(Self {
            format,
            naming: Views::new(&[TableId::Countries, TableId::CountryLinks], format, mode)?,
            country_fields: [
                columns::field(TableId::Countries, "tag")?,
                columns::field(TableId::Countries, "canonical_slot")?,
            ],
            owner_field: columns::field(TableId::States, "owner")?,
            shapes: [None; TABLE_COUNT],
            frontier: None,
            identities: BTreeMap::new(),
            tags: BTreeMap::new(),
            slots: HashMap::new(),
            countries_complete: false,
            links_complete: false,
            states_complete: false,
            owners: Cells::default(),
            owner_counts: BTreeMap::new(),
            unresolved: 0,
            fuel: Cells::default(),
            by_slot: BTreeMap::new(),
            parts: Cells::default(),
            by_country: BTreeMap::new(),
            reasons: Reasons::default(),
            summaries: BTreeMap::new(),
            hours: Vec::new(),
        })
    }

    fn owner(&mut self, raw: u32, add: bool, reached: &mut BTreeSet<u32>) {
        if let Some(&slot) = self.slots.get(&raw) {
            let before = self.owner_counts.get(&slot).copied().unwrap_or(0);
            let after = if add { before + 1 } else { before - 1 };
            if after == 0 {
                self.owner_counts.remove(&slot);
            } else {
                self.owner_counts.insert(slot, after);
            }
            if (before == 0) != (after == 0) {
                reached.insert(slot);
            }
        } else if add {
            self.unresolved += 1;
        } else {
            self.unresolved -= 1;
        }
    }

    fn index_fuel(
        &mut self,
        key: u128,
        row: Arc<CountryFuelRow>,
        add: bool,
        reached: &mut BTreeSet<u32>,
    ) {
        if let Some(&slot) = self.slots.get(&row.country) {
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

    fn supplied(
        &mut self,
        row: &Arc<CountryFuelRow>,
        state: &State,
        hour: i32,
    ) -> io::Result<(RawRead, demand::RawConsumption)> {
        let status = columns::status(state, TableId::FuelDemandParts);
        if let Some(cached) = self.summaries.get(&row.country)
            && Arc::ptr_eq(row, &cached.row)
            && cached.status == status
            && (cached.hour == hour
                || matches!(&row.demand,
                replay_data::rows::Recorded::Value(pack)
                    if pack.hour == replay_data::rows::FuelDemandHour::Host))
        {
            let mut consumption = cached.consumption.clone();
            consumption.first_hour = hour;
            consumption.last_hour = hour;
            return Ok((
                read_of(row, hour, false, None, &mut self.reasons),
                consumption,
            ));
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
        let view = match columns::status(state, TableId::FuelDemandParts) {
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
        let consumption = demand::summary_of(&rows[0], hour, hour, false, &mut self.reasons);
        self.summaries.insert(
            row.country,
            Summary {
                row: Arc::clone(row),
                status,
                hour,
                consumption: consumption.clone(),
            },
        );
        Ok((
            read_of(row, hour, false, None, &mut self.reasons),
            consumption,
        ))
    }
}

impl Operator for Projector {
    fn hour(&mut self, hour: &segments::Hour<'_>) -> io::Result<()> {
        let state = hour.state;
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
        let edits = columns::changes(state, TableId::Countries)?;
        let mask = self
            .country_fields
            .iter()
            .fold(0_u64, |mask, &field| mask | (1_u64 << field));
        let identities_changed = whole[TableId::Countries.index()]
            || changed[TableId::CountryLinks.index()]
            || !edits.inserted.is_empty()
            || !edits.deleted.is_empty()
            || edits
                .masks
                .iter()
                .chain(&edits.fill_masks)
                .any(|value| value & mask != 0);
        let mut reached = BTreeSet::new();
        let mut unnamed = BTreeSet::new();
        if identities_changed {
            reached.extend(self.identities.keys().cloned());
            let frame = self.naming.advance(state, hour.manifest, true)?;
            self.countries_complete = columns::complete(state, TableId::Countries);
            self.links_complete = columns::complete(state, TableId::CountryLinks);
            self.slots = frame
                .complete::<tables::CountryLinks>()
                .map(resolved_slots)
                .unwrap_or_default();
            self.identities.clear();
            self.tags.clear();
            if let Ok(countries) = frame.complete::<tables::Countries>() {
                if self.links_complete {
                    let keys = dashboard_text::metadata::CountryKeys::read(countries);
                    for (tag, identity) in tagged_identities(keys.iter(), &self.slots) {
                        if let Ok(tag) = std::str::from_utf8(tag) {
                            let tag = tag.to_owned();
                            if let Some((slot, _)) = identity {
                                self.tags.entry(slot).or_default().insert(tag.clone());
                            }
                            self.identities.insert(tag.clone(), identity);
                            reached.insert(tag);
                        } else if let Some((_, id)) = identity {
                            unnamed.insert(id);
                        }
                    }
                }
            }
        }
        let mut owner_reached = BTreeSet::new();
        let unresolved_before = self.unresolved != 0;
        let states_complete = columns::complete(state, TableId::States);
        let owner_valid_before =
            self.states_complete && self.countries_complete && self.links_complete;
        if changed[TableId::States.index()] {
            let owner_field = self.owner_field;
            let edits = self.owners.update(
                state,
                TableId::States,
                whole[TableId::States.index()],
                Some(1_u64 << owner_field),
                |slot| {
                    columns::plain::<u32>(columns::table(state, TableId::States)?, owner_field)?
                        .get(slot as usize)
                        .copied()
                        .ok_or_else(|| io::Error::other("State owner outside column"))
                },
            )?;
            if !identities_changed {
                for (_, old, now) in edits {
                    if old == now {
                        continue;
                    }
                    if let Some(old) = old {
                        self.owner(old, false, &mut owner_reached);
                    }
                    if let Some(now) = now {
                        self.owner(now, true, &mut owner_reached);
                    }
                }
            }
        }
        if identities_changed {
            self.owner_counts.clear();
            self.unresolved = 0;
            let owners: Vec<_> = self.owners.rows.values().copied().collect();
            for owner in owners {
                self.owner(owner, true, &mut owner_reached);
            }
        }
        self.states_complete = states_complete;
        let owner_valid = states_complete && self.countries_complete && self.links_complete;
        if owner_valid_before != owner_valid || unresolved_before != (self.unresolved != 0) {
            reached.extend(self.identities.keys().cloned());
        } else {
            for slot in owner_reached {
                reached.extend(self.tags.get(&slot).into_iter().flatten().cloned());
            }
        }
        let mut fuel_reached = BTreeSet::new();
        if changed[TableId::CountryFuel.index()] {
            let edits = self.fuel.update(
                state,
                TableId::CountryFuel,
                whole[TableId::CountryFuel.index()],
                None,
                |slot| {
                    columns::row(state, TableId::CountryFuel, slot, self.format.version())
                        .map(Arc::new)
                },
            )?;
            for (_, old, _) in &edits {
                if let Some(old) = old {
                    self.summaries.remove(&old.country);
                }
            }
            if !identities_changed {
                for (key, old, now) in edits {
                    if let Some(old) = old {
                        self.index_fuel(key, old, false, &mut fuel_reached);
                    }
                    if let Some(now) = now {
                        self.index_fuel(key, now, true, &mut fuel_reached);
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
            for (key, row) in fuel {
                self.index_fuel(key, row, true, &mut fuel_reached);
            }
        }
        for slot in &fuel_reached {
            reached.extend(self.tags.get(slot).into_iter().flatten().cloned());
        }
        if changed[TableId::FuelDemandParts.index()] {
            let edits = self.parts.update(
                state,
                TableId::FuelDemandParts,
                whole[TableId::FuelDemandParts.index()],
                None,
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
            reached.extend(self.identities.keys().cloned());
        }
        let mut countries = Vec::with_capacity(reached.len());
        for tag in reached {
            let present = self.identities.contains_key(&tag);
            let identity = self.identities.get(&tag).copied().flatten();
            let seen = if owner_valid {
                match identity {
                    Some(identity) if self.owner_counts.contains_key(&identity.0) => {
                        Some((Some(identity), true))
                    }
                    Some(identity) if self.unresolved == 0 => Some((Some(identity), false)),
                    None if !present => Some((None, false)),
                    _ => None,
                }
            } else {
                None
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
                    .map(|row| self.supplied(row, state, hour.hour))
                    .transpose()?
            } else {
                None
            };
            countries.push(Country {
                tag,
                identity,
                present,
                seen,
                sample,
                amount: row.as_ref().map(|row| (row.current_raw, row.maximum_raw)),
                supplied,
            });
        }
        self.hours.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            format: self.format,
            countries,
            identities: identities_changed,
            prior: owner_valid.then_some((None, false)),
            unnamed,
            read,
            available,
            refresh_levels: changed[TableId::CountryFuel.index()]
                || changed[TableId::CountryLinks.index()],
            countries_complete: self.countries_complete,
        });
        self.frontier = Some(hour.hour);
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(std::mem::take(&mut self.hours)))
    }
}

impl Scan {
    pub(super) fn catch_up_projected(&mut self, tag: &str) {
        let Some(through) = self.projected_frontier else {
            return;
        };
        let active = self.active.contains(tag);
        let country = self.countries.get_mut(tag).expect("projected fuel country");
        let last = country.recent.back().map(|(hour, _)| *hour);
        if active {
            let seen = country.recent.back().and_then(|(_, seen)| *seen);
            if let Some(first) = last.and_then(|hour| hour.checked_add(1)) {
                country.recent.extend(
                    (first.max(through.saturating_sub(DAY_HOURS))..=through)
                        .map(|hour| (hour, seen)),
                );
            }
            country.track.advance(through);
            if country.open
                && let Some(read) = country.result.reads.last_mut()
            {
                read.last_hour = through;
            }
            if country.demand_open
                && let Some(read) = country.result.consumption.last_mut()
            {
                read.last_hour = through;
            }
        } else {
            country.recent.extend(
                self.prior
                    .iter()
                    .filter(|(hour, _)| last.is_none_or(|last| *hour > last))
                    .copied(),
            );
        }
        while country
            .recent
            .front()
            .is_some_and(|(hour, _)| through.saturating_sub(*hour) > DAY_HOURS)
        {
            country.recent.pop_front();
        }
    }

    pub(super) fn finish_projected(&mut self) {
        if self.projected_frontier.is_some() {
            let tags: Vec<_> = self.countries.keys().cloned().collect();
            for tag in tags {
                self.catch_up_projected(&tag);
            }
        }
    }
}

impl prepared::Prepared {
    pub fn projector(&self, replay: &Replay) -> io::Result<Projector> {
        Projector::new(replay)
    }
}
