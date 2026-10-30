//! Supply health projected from a columnar walk ([`replay::segments`]): each division's
//! owner and supply bucket and each country's counts, kept from the hour's changed slots,
//! and handed on hour by hour for [`super::Scan::observe_projected`] to keep in order. A
//! segment's first hour, a roster change and a change of validity or owner resolution hand
//! every country's counts; the emitting scan's tracks keep only what changed.

use super::{Defines, Supply, slot_read};
use dashboard_text::metadata::tagged_countries;
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryLinkRow, CountryRow, DivisionLandInputsRow, DivisionStatusRow, TableId},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    mem::size_of,
    sync::Arc,
};

/// The division tables whose changes make an hour relevant.
const DIVISIONS: [TableId; 3] = [
    TableId::Divisions,
    TableId::DivisionLandInputs,
    TableId::DivisionStatus,
];

/// The countries an hour names: each tag's holder (canonical slot and country id) when it
/// resolves, in tag order.
pub struct Roster {
    pub tagged: Vec<(Arc<str>, Option<(u32, u32)>)>,
}

/// One projected hour.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// The countries, at a segment's first hour and when they may have changed; an error
    /// when Countries or CountryLinks hold no complete rows.
    pub roster: Option<Result<Roster, ()>>,
    /// Every country's tag in the Countries table's order, whenever Countries holds every row
    /// and the countries may have changed or cannot be joined.
    pub tags: Option<Vec<String>>,
    /// Whether DivisionLandInputs is not unavailable.
    pub land_present: bool,
    /// Whether any division table changed.
    pub relevant: bool,
    /// Whether the hour's division tables were all held and read.
    pub valid: bool,
    /// Whether every division's owner names a country link.
    pub owners_known: bool,
    /// Counts by tag: every country's when `all`, else those of countries a division
    /// change reached. None when a division of the country has no known bucket.
    pub counts: BTreeMap<Arc<str>, Option<[u32; 3]>>,
    pub all: bool,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        // Tags are the projection's, shared.
        let roster = match &self.roster {
            Some(Ok(roster)) => {
                roster.tagged.capacity() * size_of::<(Arc<str>, Option<(u32, u32)>)>()
            }
            _ => 0,
        };
        let tags = self.tags.as_ref().map_or(0, |tags| {
            tags.capacity() * size_of::<String>() + tags.iter().map(String::capacity).sum::<usize>()
        });
        roster + tags + self.counts.len() * size_of::<(Arc<str>, Option<[u32; 3]>)>()
    }
}

/// The columns a projection reads by name.
struct Fields {
    country_tag: usize,
    country_slot: usize,
    link_slot: usize,
    owners: [usize; 2],
    transfer: usize,
    supply: [usize; 3],
}

/// One segment's projection.
pub struct Projector {
    format: FormatVersion,
    defines: Option<Defines>,
    fields: Fields,
    /// Each link id's tag, through its canonical slot.
    owners: HashMap<u32, Option<Arc<str>>>,
    /// Each tag's holder.
    identities: HashMap<Arc<str>, Option<(u32, u32)>>,
    /// Each division's owner tag (None for an owner naming no country), by Divisions slot.
    members: HashMap<u64, Option<Arc<str>>>,
    owners_known: bool,
    /// Each division's bucket, None when its supply is not known.
    buckets: HashMap<u64, Option<usize>>,
    /// Each tag's counts and divisions of unknown bucket.
    totals: HashMap<Arc<str>, ([u32; 3], usize)>,
    valid: Option<bool>,
    /// Whether Countries and CountryLinks held every row the hour before.
    countries_ok: Option<bool>,
    /// Whether DivisionLandInputs and DivisionStatus were each truncated or unavailable the
    /// hour before: a change refreshes every bucket, since restored rows may equal the cells
    /// the walker kept.
    shape: Option<[(bool, bool); 2]>,
    /// Each division table's (complete, truncated, unavailable) status the hour before.
    statuses: Option<[(bool, bool, bool); 3]>,
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

/// Every live row of `table`, by key.
fn rows<R: replay_data::rows::RowCodec>(
    state: &State,
    table: TableId,
    format: u16,
) -> io::Result<Vec<R>> {
    let held = columns::table(state, table)?;
    let mut slots: Vec<(u128, u32)> = held
        .live()
        .ones()
        .filter_map(|slot| Some((held.key(slot)?, slot)))
        .collect();
    slots.sort_unstable();
    slots
        .into_iter()
        .map(|(_, slot)| columns::row(state, table, slot, format))
        .collect()
}

impl Projector {
    pub(super) fn new(format: FormatVersion, defines: Option<Defines>) -> io::Result<Self> {
        Ok(Self {
            format,
            defines,
            fields: Fields {
                country_tag: columns::field(TableId::Countries, "tag")?,
                country_slot: columns::field(TableId::Countries, "canonical_slot")?,
                link_slot: columns::field(TableId::CountryLinks, "canonical_slot")?,
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
            owners: HashMap::new(),
            identities: HashMap::new(),
            members: HashMap::new(),
            owners_known: false,
            buckets: HashMap::new(),
            totals: HashMap::new(),
            valid: None,
            countries_ok: None,
            shape: None,
            statuses: None,
        })
    }

    /// The countries from every country and link, as the row path joins them; an error when
    /// `countries` (Countries' rows) or CountryLinks is not complete.
    fn roster(
        &mut self,
        state: &State,
        countries: Option<&[CountryRow]>,
    ) -> io::Result<Result<Roster, ()>> {
        self.owners.clear();
        self.identities.clear();
        let Some(countries) = countries.filter(|_| columns::complete(state, TableId::CountryLinks))
        else {
            return Ok(Err(()));
        };
        let links: Vec<CountryLinkRow> = rows(state, TableId::CountryLinks, self.format.version())?;
        let mut tagged = Vec::new();
        let mut slots = HashMap::new();
        for (tag, lookup) in tagged_countries(countries, &links) {
            let Ok(tag) = std::str::from_utf8(tag) else {
                continue;
            };
            let tag: Arc<str> = Arc::from(tag);
            let identity = lookup.found().map(|(country, slot)| (slot, country.id));
            if let Some((slot, _)) = identity {
                slots.insert(slot, Arc::clone(&tag));
            }
            self.identities.insert(Arc::clone(&tag), identity);
            tagged.push((tag, identity));
        }
        self.owners = links
            .iter()
            .map(|link| (link.id, slots.get(&link.canonical_slot).cloned()))
            .collect();
        Ok(Ok(Roster { tagged }))
    }

    /// A division's supply bucket from its status and land rows; None when either is not
    /// held or its supply is not recorded.
    fn bucket(&self, state: &State, division: u64) -> io::Result<Option<usize>> {
        let defines = self.defines.expect("projected with defines");
        // An unavailable table holds no rows this hour, whatever cells the walker kept.
        if columns::unavailable(state, TableId::DivisionStatus)
            || columns::unavailable(state, TableId::DivisionLandInputs)
        {
            return Ok(None);
        }
        let format = self.format.version();
        let land = columns::table(state, TableId::DivisionLandInputs)?;
        let status = columns::table(state, TableId::DivisionStatus)?;
        let held = |table: TableId, slot: u32| match columns::truncated(state, table) {
            Some(read) => read.binary_search(&slot).is_ok(),
            None => true,
        };
        let (Some(land_slot), Some(status_slot)) = (
            land.slot(u128::from(division))
                .filter(|&slot| held(TableId::DivisionLandInputs, slot)),
            status
                .slot(u128::from(division))
                .filter(|&slot| held(TableId::DivisionStatus, slot)),
        ) else {
            return Ok(None);
        };
        let land: DivisionLandInputsRow =
            columns::row(state, TableId::DivisionLandInputs, land_slot, format)?;
        let status: DivisionStatusRow =
            columns::row(state, TableId::DivisionStatus, status_slot, format)?;
        Ok(Supply::parts(&status, false).map(|mut row| {
            row.naval_transfer = land.naval_transfer;
            match defines.frame(&row) {
                3 => 0,
                1 | 2 => 1,
                _ => 2,
            }
        }))
    }

    /// Moves `division` to `owner` and `bucket` in the totals: the tags it left and joined.
    fn place(
        &mut self,
        division: u64,
        owner: Option<Option<Arc<str>>>,
        bucket: Option<Option<usize>>,
        reached: &mut BTreeSet<Arc<str>>,
    ) -> io::Result<()> {
        if let Some(Some(tag)) = self.members.get(&division)
            && let Some(total) = self.totals.get_mut(tag)
        {
            match self.buckets.get(&division) {
                Some(Some(bucket)) => total.0[*bucket] -= 1,
                Some(None) => total.1 -= 1,
                None => {}
            }
            reached.insert(Arc::clone(tag));
        }
        match owner {
            Some(owner) => {
                self.members.insert(division, owner);
            }
            None => {
                self.members.remove(&division);
            }
        }
        match bucket {
            Some(bucket) => {
                self.buckets.insert(division, bucket);
            }
            None => {
                self.buckets.remove(&division);
            }
        }
        if let (Some(Some(tag)), Some(bucket)) =
            (self.members.get(&division), self.buckets.get(&division))
        {
            let total = self.totals.entry(Arc::clone(tag)).or_default();
            match bucket {
                Some(bucket) => {
                    total.0[*bucket] = total.0[*bucket].checked_add(1).ok_or_else(|| {
                        dashboard_core::invalid("Supply health division count overflow")
                    })?
                }
                None => total.1 += 1,
            }
            reached.insert(Arc::clone(tag));
        }
        Ok(())
    }

    /// The owner tag of the division at Divisions `slot` through its link, the expeditionary
    /// owner first as [`super::owner`] takes it; None (outer) when the owner names no link.
    fn owner(&self, state: &State, slot: u32) -> io::Result<Option<Option<Arc<str>>>> {
        let divisions = columns::table(state, TableId::Divisions)?;
        let [roster, expeditionary] = self.fields.owners;
        let roster = columns::plain::<u32>(divisions, roster)?;
        let expeditionary = columns::plain::<u32>(divisions, expeditionary)?;
        let at = slot as usize;
        let owner = match expeditionary.get(at) {
            Some(&owner) if owner as i32 > 0 => owner,
            _ => roster.get(at).copied().unwrap_or_default(),
        };
        Ok(self.owners.get(&owner).cloned())
    }

    /// Every division placed anew.
    fn recount(&mut self, state: &State) -> io::Result<()> {
        self.members.clear();
        self.buckets.clear();
        self.totals.clear();
        self.owners_known = true;
        let divisions = columns::table(state, TableId::Divisions)?;
        let mut owned = Vec::new();
        for slot in divisions.live().ones() {
            let Some(owner) = self.owner(state, slot)? else {
                self.owners_known = false;
                return Ok(());
            };
            owned.extend(divisions.key(slot).map(|key| (key as u64, owner)));
        }
        let mut reached = BTreeSet::new();
        for (division, owner) in owned {
            let bucket = self.bucket(state, division)?;
            self.place(division, Some(owner), Some(bucket), &mut reached)?;
        }
        Ok(())
    }

    /// The hour's added, removed and re-owned divisions placed; false when an owner names no
    /// link, which only a recount resolves.
    fn move_divisions(
        &mut self,
        state: &State,
        reached: &mut BTreeSet<Arc<str>>,
    ) -> io::Result<bool> {
        let divisions = columns::table(state, TableId::Divisions)?;
        let changes = columns::changes(state, TableId::Divisions)?;
        for &key in &changes.deleted_keys {
            self.place(key as u64, None, None, reached)?;
        }
        let owners = self.fields.owners;
        let moved = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .filter(|&(_, &mask)| names(mask, &owners))
            .map(|(&slot, _)| slot)
            .chain(
                changes
                    .fill_slots
                    .iter()
                    .zip(&changes.fill_masks)
                    .filter(|&(_, &mask)| names(mask, &owners))
                    .map(|(&slot, _)| slot),
            )
            .chain(changes.inserted.iter().copied());
        for slot in moved.collect::<Vec<_>>() {
            let Some(owner) = self.owner(state, slot)? else {
                return Ok(false);
            };
            let Some(division) = divisions.key(slot).map(|key| key as u64) else {
                continue;
            };
            let bucket = self.bucket(state, division)?;
            self.place(division, Some(owner), Some(bucket), reached)?;
        }
        Ok(true)
    }
}

impl Operator for Projector {
    type Output = Hour;

    fn hour(&mut self, hour: &segments::Hour<'_>, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        let state = hour.state;
        let land_present = !columns::unavailable(state, TableId::DivisionLandInputs);
        // A division table's rows or status changed, as the row path's change facts say.
        let statuses = DIVISIONS.map(|table| {
            (
                columns::complete(state, table),
                columns::truncated(state, table).is_some(),
                columns::unavailable(state, table),
            )
        });
        let mut relevant = self.statuses != Some(statuses);
        self.statuses = Some(statuses);
        for table in DIVISIONS {
            let changes = columns::changes(state, table)?;
            relevant |= !changes.changed.is_empty()
                || !changes.inserted.is_empty()
                || !changes.deleted.is_empty()
                || !changes.fill_slots.is_empty()
                || changes.truncated;
        }
        let countries_complete = columns::complete(state, TableId::Countries);
        let countries_ok = countries_complete && columns::complete(state, TableId::CountryLinks);
        let roster_changed = hour.starts
            || self.countries_ok != Some(countries_ok)
            || countries_ok
                && (touched(
                    state,
                    TableId::Countries,
                    &[self.fields.country_tag, self.fields.country_slot],
                )? || touched(state, TableId::CountryLinks, &[self.fields.link_slot])?);
        self.countries_ok = Some(countries_ok);
        // Refused countries are refused every hour, as the row path's join is; their tags are
        // still the complete Countries table's.
        let countries: Option<Vec<CountryRow>> =
            if countries_complete && (roster_changed || !countries_ok) {
                Some(rows(state, TableId::Countries, self.format.version())?)
            } else {
                None
            };
        let tags = countries.as_ref().map(|rows| {
            rows.iter()
                .filter_map(|row| std::str::from_utf8(&row.tag).ok().map(str::to_owned))
                .collect()
        });
        let roster = if !countries_ok || roster_changed {
            Some(self.roster(state, countries.as_deref())?)
        } else {
            None
        };
        relevant |= roster_changed;
        let metrics = metrics(hour.manifest);
        let read = DIVISIONS
            .iter()
            .all(|&table| slot_read(self.format, &metrics, table, hour.hour));
        // As the row path's recorded rows: complete, truncated, or unavailable and empty.
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
        let mut counts = BTreeMap::new();
        if valid && self.defines.is_some() {
            let known = self.owners_known;
            let truncated = DIVISIONS
                .iter()
                .any(|&table| columns::truncated(state, table).is_some());
            let mut reached = BTreeSet::new();
            // Unresolved owners stay so until an owner, a division or the countries change.
            let resolvable =
                all || reshaped || touched(state, TableId::Divisions, &self.fields.owners)?;
            if known || resolvable {
                if all
                    || reshaped
                    || truncated
                    || !known
                    || !self.move_divisions(state, &mut reached)?
                {
                    self.recount(state)?;
                    all = true;
                } else {
                    self.resupply(state, &mut reached)?;
                    for tag in reached {
                        counts.insert(tag.clone(), self.counts(&tag, valid));
                    }
                }
            }
            all |= known != self.owners_known;
        }
        if all {
            for tag in self.identities.keys() {
                counts.insert(Arc::clone(tag), self.counts(tag, valid));
            }
        }
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            roster,
            tags,
            land_present,
            relevant,
            valid,
            owners_known: self.owners_known,
            counts,
            all,
        })
    }
}

impl Projector {
    /// The buckets of member divisions whose transfer or supply the hour changed, added or
    /// removed, placed again.
    fn resupply(&mut self, state: &State, reached: &mut BTreeSet<Arc<str>>) -> io::Result<()> {
        let mut dirty = BTreeSet::new();
        for (table, fields) in [
            (TableId::DivisionLandInputs, vec![self.fields.transfer]),
            (TableId::DivisionStatus, self.fields.supply.to_vec()),
        ] {
            let held = columns::table(state, table)?;
            let changes = columns::changes(state, table)?;
            for (at, &slot) in changes.changed.iter().enumerate() {
                if names(changes.masks[at], &fields) {
                    dirty.extend(held.key(slot).map(|key| key as u64));
                }
            }
            for (at, &slot) in changes.fill_slots.iter().enumerate() {
                if names(changes.fill_masks[at], &fields) {
                    dirty.extend(held.key(slot).map(|key| key as u64));
                }
            }
            for &slot in &changes.inserted {
                dirty.extend(held.key(slot).map(|key| key as u64));
            }
            dirty.extend(changes.deleted_keys.iter().map(|&key| key as u64));
        }
        for division in dirty {
            if !self.members.contains_key(&division) {
                continue;
            }
            let bucket = self.bucket(state, division)?;
            let owner = self.members.get(&division).cloned();
            self.place(division, owner, Some(bucket), reached)?;
        }
        Ok(())
    }

    /// A tag's counts as the row path makes them: its divisions' when none is of unknown
    /// bucket and it has a holder, zeros for a held country without divisions.
    fn counts(&self, tag: &Arc<str>, valid: bool) -> Option<[u32; 3]> {
        let identity = self.identities.get(tag).copied().flatten();
        match self.totals.get(tag) {
            Some((counts, unknown)) => (*unknown == 0)
                .then_some(identity)
                .flatten()
                .map(|_| *counts),
            None => (valid && self.owners_known)
                .then_some(identity)
                .flatten()
                .map(|_| [0; 3]),
        }
    }
}
