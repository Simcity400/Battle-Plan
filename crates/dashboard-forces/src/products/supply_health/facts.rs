//! Supply health's facts, projected hour by hour from a columnar walk ([`replay::segments`]):
//! each country's holder and its divisions' counts by supply state, handed only when either
//! changed. Each division's owner and count is kept and moved from the hour's changed slots,
//! read from its own columns; the countries and their links are the walk's
//! ([`dashboard_world`]). A segment's first hour, and any hour after a skipped one, hands every
//! country.

use super::{Defines, Supply};
use dashboard_text::metadata::slot_read;
use dashboard_world::{Needs, Roster, Tag, Tagged, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    display::{Step, stepped_format},
    format::FormatVersion,
    rows::{TableId, compatibility::RECORDER_INPUTS_FORMAT},
};
use std::{collections::HashMap, io, mem::size_of, sync::Arc};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionLandInputs,
    TableId::DivisionStatus,
];

/// The division tables whose reads date and validate an hour.
const DIVISIONS: [TableId; 3] = [
    TableId::Divisions,
    TableId::DivisionLandInputs,
    TableId::DivisionStatus,
];

/// What a projection reads of its walk's world.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// A division's count when its supply is not known this hour.
const UNKNOWN: u8 = 3;
/// A division's owner naming a link no counted country holds.
const NO_COUNTRY: u32 = u32::MAX;

/// A country's facts at an hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Country {
    /// Its index in the projection ([`Hour::named`]).
    pub index: u32,
    /// The canonical slot and id of the country holding its tag.
    pub holder: Option<(u32, u32)>,
    /// Its divisions by supply state (no supply, strained, in supply); None when any is not
    /// known, or it has no holder.
    pub counts: Option<[u32; 3]>,
}

/// One projected hour.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Countries the projection numbers from this hour on, by index, with their tags.
    pub named: Vec<(u32, Arc<str>)>,
    /// Every named country is handed: one left out holds nothing.
    pub whole: bool,
    /// Countries whose holder or counts changed, ascending by index.
    pub countries: Vec<Country>,
    /// Whether DivisionLandInputs is not unavailable.
    pub land_present: bool,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.named.capacity() * size_of::<(u32, Arc<str>)>()
            + self.named.iter().map(|(_, tag)| tag.len()).sum::<usize>()
            + self.countries.capacity() * size_of::<Country>()
    }
}

/// The columns a projection reads by name.
struct Fields {
    owners: [usize; 2],
    transfer: usize,
    supply: [usize; 3],
}

#[derive(Clone)]
struct Named {
    holder: Option<(u32, u32)>,
    counts: [u32; 3],
    /// Its divisions of unknown count.
    unknown: u32,
    /// What the projection last handed of it.
    handed: Option<(Option<(u32, u32)>, Option<[u32; 3]>)>,
    /// Reached by a change this hour.
    reached: bool,
}

#[derive(Clone, Copy)]
struct Division {
    owner: u32,
    count: u8,
}

/// One segment's projection.
pub struct Projector {
    format: FormatVersion,
    defines: Option<Defines>,
    fields: Fields,
    /// Whether DivisionStatus records its grace operands in this format.
    grace: bool,
    numbers: HashMap<Tag, u32>,
    named: Vec<Named>,
    /// The country found holding each canonical slot.
    slots: HashMap<u32, u32>,
    divisions: HashMap<u64, Division>,
    /// Countries a change reached this hour, by index.
    reached: Vec<u32>,
    owners_known: bool,
    valid: Option<bool>,
    countries_ok: Option<bool>,
    /// Whether DivisionLandInputs and DivisionStatus were each truncated or unavailable the
    /// hour before: a change places every division again, since restored rows may equal the
    /// cells the walker kept.
    shape: Option<[(bool, bool); 2]>,
    last: Option<i32>,
}

fn names(mask: u64, columns: &[usize]) -> bool {
    columns
        .iter()
        .any(|&column| column >= 64 || mask >> column & 1 != 0)
}

/// Whether the hour changed any of `fields` of `table`, or added or removed a row.
fn touched(state: &State, table: TableId, fields: &[usize]) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(!changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || changes.masks.iter().any(|&mask| names(mask, fields))
        || changes.fill_masks.iter().any(|&mask| names(mask, fields)))
}

impl Projector {
    pub(super) fn new(format: FormatVersion, defines: Option<Defines>) -> io::Result<Self> {
        Ok(Self {
            format,
            defines,
            fields: Fields {
                owners: [
                    columns::field(TableId::Divisions, "roster_owner")?,
                    columns::field(TableId::Divisions, "expeditionary_owner")?,
                ],
                transfer: columns::field(TableId::DivisionLandInputs, "naval_transfer")?,
                supply: [
                    columns::field(TableId::DivisionStatus, "supply_ratio")?,
                    columns::field(TableId::DivisionStatus, "supply_grace_maximum")?,
                    columns::field(TableId::DivisionStatus, "supply_grace_remaining")?,
                ],
            },
            grace: format.version() >= RECORDER_INPUTS_FORMAT,
            numbers: HashMap::new(),
            named: Vec::new(),
            slots: HashMap::new(),
            divisions: HashMap::new(),
            reached: Vec::new(),
            owners_known: false,
            valid: None,
            countries_ok: None,
            shape: None,
            last: None,
        })
    }

    /// The index of `tag`, named in `out` when new.
    fn number(&mut self, tag: Tag, text: &str, out: &mut Vec<(u32, Arc<str>)>) -> u32 {
        if let Some(&index) = self.numbers.get(&tag) {
            return index;
        }
        let index = self.named.len() as u32;
        let text: Arc<str> = Arc::from(text);
        out.push((index, text));
        self.numbers.insert(tag, index);
        self.named.push(Named {
            holder: None,
            counts: [0; 3],
            unknown: 0,
            handed: None,
            reached: false,
        });
        index
    }

    /// Each held tag's holder, as every per-country product reads it: a tag's first holder by
    /// id the links resolve. A slot two tags' holders share is the later tag's.
    fn hold(&mut self, countries: &Roster, named: &mut Vec<(u32, Arc<str>)>) {
        for country in &mut self.named {
            country.holder = None;
        }
        self.slots.clear();
        if !countries.countries_complete() {
            return;
        }
        let mut tags: Vec<(&str, Tag)> = countries
            .held()
            .filter_map(|tag| Some((std::str::from_utf8(countries.tag(tag)).ok()?, tag)))
            .collect();
        tags.sort_unstable();
        for (text, tag) in tags {
            let index = self.number(tag, text, named);
            if !countries.complete() {
                continue;
            }
            if let Tagged::Found(country) = countries.tagged(tag) {
                self.named[index as usize].holder = Some((country.slot, country.id));
                self.slots.insert(country.slot, index);
            }
        }
    }

    /// Each division's count from its status and land rows, the hour's columns read once:
    /// [`UNKNOWN`] when either row is not held this hour or its grace is not recorded.
    fn counter<'s>(
        &self,
        state: &'s State,
    ) -> io::Result<impl Fn(u64) -> io::Result<u8> + use<'s>> {
        // An unavailable table holds no rows this hour, whatever cells the walker kept.
        let usable = !columns::unavailable(state, TableId::DivisionLandInputs)
            && !columns::unavailable(state, TableId::DivisionStatus)
            && self.grace;
        let defines = self.defines.filter(|_| usable);
        let land = columns::table(state, TableId::DivisionLandInputs)?;
        let status = columns::table(state, TableId::DivisionStatus)?;
        let [received, maximum, remaining] = self.fields.supply;
        let step = |field| {
            TableId::DivisionStatus
                .display_step(field)
                .filter(|_| stepped_format(self.format.version()))
        };
        let received_step = step(received);
        let remaining_step = step(remaining);
        let native = |units, step: Option<Step>| step.map_or(Ok(units), |step| step.value(units));
        let transfer = columns::plain::<bool>(land, self.fields.transfer)?;
        let received = columns::plain::<i64>(status, received)?;
        let maximum = columns::plain::<i64>(status, maximum)?;
        let remaining = columns::plain::<i64>(status, remaining)?;
        let land_read = columns::truncated(state, TableId::DivisionLandInputs);
        let status_read = columns::truncated(state, TableId::DivisionStatus);
        let read = |slot: u32, truncated: Option<&[u32]>| {
            truncated.is_none_or(|read| read.binary_search(&slot).is_ok())
        };
        Ok(move |division: u64| {
            let Some(defines) = defines else {
                return Ok(UNKNOWN);
            };
            let (Some(land_slot), Some(status_slot)) = (
                land.slot(u128::from(division))
                    .filter(|&slot| read(slot, land_read)),
                status
                    .slot(u128::from(division))
                    .filter(|&slot| read(slot, status_read)),
            ) else {
                return Ok(UNKNOWN);
            };
            let at = status_slot as usize;
            Ok(defines.bucket(&Supply {
                received: native(received.get(at).copied().unwrap_or_default(), received_step)?,
                maximum_grace: maximum.get(at).copied().unwrap_or_default(),
                remaining_grace: native(
                    remaining.get(at).copied().unwrap_or_default(),
                    remaining_step,
                )?,
                naval_transfer: transfer
                    .get(land_slot as usize)
                    .copied()
                    .unwrap_or_default(),
            }))
        })
    }

    /// The country owning the division at Divisions `slot`: its expeditionary owner when
    /// positive, else its roster owner, through its link; None when the owner names no link.
    fn owner(&self, countries: &Roster, owners: (&[u32], &[u32]), slot: u32) -> Option<u32> {
        let at = slot as usize;
        let owner = match owners.1.get(at) {
            Some(&owner) if owner as i32 > 0 => owner,
            _ => owners.0.get(at).copied().unwrap_or_default(),
        };
        let slot = countries.slot_of(owner)?;
        Some(self.slots.get(&slot).copied().unwrap_or(NO_COUNTRY))
    }

    /// Moves `division` to `next` (None removes it), marking the countries it left and joined.
    fn place(&mut self, division: u64, next: Option<Division>) {
        if let Some(held) = self.divisions.get(&division).copied()
            && let Some(country) = self.named.get_mut(held.owner as usize)
        {
            match held.count {
                UNKNOWN => country.unknown -= 1,
                count => country.counts[count as usize] -= 1,
            }
            if !country.reached {
                country.reached = true;
                self.reached.push(held.owner);
            }
        }
        match next {
            Some(next) => {
                self.divisions.insert(division, next);
                if let Some(country) = self.named.get_mut(next.owner as usize) {
                    match next.count {
                        UNKNOWN => country.unknown += 1,
                        count => country.counts[count as usize] += 1,
                    }
                    if !country.reached {
                        country.reached = true;
                        self.reached.push(next.owner);
                    }
                }
            }
            None => {
                self.divisions.remove(&division);
            }
        }
    }

    /// Every division placed anew; false, with nothing placed, when an owner names no link.
    fn recount(
        &mut self,
        state: &State,
        countries: &Roster,
        count: &impl Fn(u64) -> io::Result<u8>,
    ) -> io::Result<bool> {
        self.divisions.clear();
        for country in &mut self.named {
            country.counts = [0; 3];
            country.unknown = 0;
        }
        let table = columns::table(state, TableId::Divisions)?;
        let owners = (
            columns::plain::<u32>(table, self.fields.owners[0])?,
            columns::plain::<u32>(table, self.fields.owners[1])?,
        );
        let mut owned = Vec::new();
        for slot in table.live().ones() {
            let Some(owner) = self.owner(countries, owners, slot) else {
                return Ok(false);
            };
            if let Some(division) = table.key(slot).map(|key| key as u64) {
                engine_allocation::reserve(&mut owned, 1)?;
                owned.push((
                    division,
                    Division {
                        owner,
                        count: count(division)?,
                    },
                ));
            }
        }
        for (division, placed) in owned {
            self.place(division, Some(placed));
        }
        Ok(true)
    }

    /// The hour's added, removed and re-owned divisions placed; false when an owner names no
    /// link, which only a recount resolves.
    fn move_divisions(
        &mut self,
        state: &State,
        countries: &Roster,
        count: &impl Fn(u64) -> io::Result<u8>,
    ) -> io::Result<bool> {
        let table = columns::table(state, TableId::Divisions)?;
        let changes = columns::changes(state, TableId::Divisions)?;
        for &key in &changes.deleted_keys {
            self.place(key as u64, None);
        }
        let fields = self.fields.owners;
        let owners = (
            columns::plain::<u32>(table, fields[0])?,
            columns::plain::<u32>(table, fields[1])?,
        );
        let moved: Vec<u32> = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            .filter(|&(_, &mask)| names(mask, &fields))
            .map(|(&slot, _)| slot)
            .chain(changes.inserted.iter().copied())
            .collect();
        let mut placed = Vec::with_capacity(moved.len());
        for slot in moved {
            let Some(owner) = self.owner(countries, owners, slot) else {
                return Ok(false);
            };
            if let Some(division) = table.key(slot).map(|key| key as u64) {
                placed.push((
                    division,
                    Division {
                        owner,
                        count: count(division)?,
                    },
                ));
            }
        }
        for (division, next) in placed {
            self.place(division, Some(next));
        }
        Ok(true)
    }

    /// Counted divisions whose transfer or supply the hour changed, added or removed, counted
    /// again.
    fn resupply(
        &mut self,
        state: &State,
        count: &impl Fn(u64) -> io::Result<u8>,
    ) -> io::Result<()> {
        let mut dirty = Vec::new();
        for (table, fields) in [
            (TableId::DivisionLandInputs, &[self.fields.transfer][..]),
            (TableId::DivisionStatus, &self.fields.supply[..]),
        ] {
            let held = columns::table(state, table)?;
            let changes = columns::changes(state, table)?;
            for (&slot, &mask) in changes
                .changed
                .iter()
                .zip(&changes.masks)
                .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            {
                if names(mask, fields) {
                    dirty.extend(held.key(slot).map(|key| key as u64));
                }
            }
            for &slot in &changes.inserted {
                dirty.extend(held.key(slot).map(|key| key as u64));
            }
            dirty.extend(changes.deleted_keys.iter().map(|&key| key as u64));
        }
        dirty.sort_unstable();
        dirty.dedup();
        let mut recounted = Vec::new();
        for division in dirty {
            let Some(held) = self.divisions.get(&division).copied() else {
                continue;
            };
            let count = count(division)?;
            if count != held.count {
                recounted.push((division, Division { count, ..held }));
            }
        }
        for (division, next) in recounted {
            self.place(division, Some(next));
        }
        Ok(())
    }

    /// A country's counts as counted: its divisions' when none is of unknown count and it
    /// has a holder; none while the hour is not valid or an owner is unresolved.
    fn counts(&self, country: &Named, valid: bool) -> Option<[u32; 3]> {
        (valid && self.owners_known && country.unknown == 0 && country.holder.is_some())
            .then_some(country.counts)
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        if hour.starts {
            self.numbers.clear();
            self.named.clear();
        }
        let state = hour.state;
        let countries = hour.world.countries()?;
        let land_present = !columns::unavailable(state, TableId::DivisionLandInputs);
        let whole = hour.starts
            || self
                .last
                .is_none_or(|last| last.checked_add(1) != Some(hour.hour));
        self.last = Some(hour.hour);
        let mut named = Vec::new();
        let countries_ok = countries.complete();
        let roster_changed =
            whole || self.countries_ok != Some(countries_ok) || countries_ok && countries.changed();
        self.countries_ok = Some(countries_ok);
        if roster_changed || !countries_ok {
            self.hold(countries, &mut named);
        }
        let metrics = metrics(hour.manifest);
        let read = DIVISIONS
            .iter()
            .all(|&table| slot_read(self.format, &metrics, table, hour.hour));
        // As the row decoder's recorded rows: complete, truncated, or unavailable and empty.
        let recorded = |table| {
            columns::complete(state, table)
                || columns::truncated(state, table).is_some()
                || columns::unavailable(state, table)
        };
        let shape = [TableId::DivisionLandInputs, TableId::DivisionStatus].map(|table| {
            (
                columns::truncated(state, table).is_some(),
                columns::unavailable(state, table),
            )
        });
        let reshaped = self.shape != Some(shape);
        self.shape = Some(shape);
        let valid = countries_ok
            && columns::complete(state, TableId::Divisions)
            && recorded(TableId::DivisionLandInputs)
            && recorded(TableId::DivisionStatus)
            && land_present
            && read;
        let mut all = roster_changed || self.valid != Some(valid);
        self.valid = Some(valid);
        if valid && self.defines.is_some() {
            let known = self.owners_known;
            let truncated = DIVISIONS
                .iter()
                .any(|&table| columns::truncated(state, table).is_some());
            // Unresolved owners stay so until an owner, a division or the countries change.
            let resolvable =
                all || reshaped || touched(state, TableId::Divisions, &self.fields.owners)?;
            if known || resolvable {
                let count = self.counter(state)?;
                if all
                    || reshaped
                    || truncated
                    || !known
                    || !self.move_divisions(state, countries, &count)?
                {
                    self.owners_known = self.recount(state, countries, &count)?;
                    all = true;
                } else {
                    self.resupply(state, &count)?;
                }
            }
            all |= known != self.owners_known;
        }
        let mut candidates = std::mem::take(&mut self.reached);
        if all {
            candidates = (0..self.named.len() as u32).collect();
        } else {
            candidates.sort_unstable();
        }
        let mut handed = Vec::new();
        for index in candidates {
            let index = index as usize;
            let next = (
                self.named[index].holder,
                self.counts(&self.named[index], valid),
            );
            let country = &mut self.named[index];
            country.reached = false;
            if whole || country.handed != Some(next) {
                country.handed = Some(next);
                handed.push(Country {
                    index: index as u32,
                    holder: next.0,
                    counts: next.1,
                });
            }
        }
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            named,
            whole,
            countries: handed,
            land_present,
        })
    }
}
