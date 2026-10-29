//! Stability's measures projected from a columnar walk ([`replay::segments`]): each hour,
//! from the walker's tables alone, every country whose measures may have changed, at least
//! those [`super::Scan::step`] would offer, for [`super::Scan::emit`] to keep in order. A
//! segment's first hour projects every country: the emitting scan's tracks keep only what
//! changed.

use replay::{
    columns,
    segments::{self, Operator, State},
};
use replay_data::rows::{
    CountryCommandPowerRow, CountryLinkRow, CountryRow, CountryStabilityInputRow, TableId,
};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryStabilityInputs,
    TableId::SliceReads,
    TableId::CountryCommandPower,
];

/// One country's measures at a projected hour.
pub struct Projected {
    pub id: u32,
    pub tag: Arc<str>,
    pub slot: u32,
    /// Its inputs' read hour.
    pub read: i32,
    pub row: CountryStabilityInputRow,
    pub values: [Option<i64>; 3],
}

/// One projected hour.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// The tags with a current holder, when they may have changed.
    pub valid: Option<BTreeSet<String>>,
    /// By id.
    pub countries: Vec<Projected>,
}

/// The columns a projection reads by name.
struct Fields {
    country_tag: usize,
    country_slot: usize,
    link_slot: usize,
    sample_hour: usize,
}

/// One segment's projection.
pub struct Projector {
    format: u16,
    fields: Fields,
    /// Each country's tag and mapped slot by id, and each slot's countries.
    roster: BTreeMap<u32, (Arc<str>, u32)>,
    slots: BTreeMap<u32, Vec<u32>>,
    /// Which of [`TABLES`] held rows the hour before.
    available: Option<[bool; 5]>,
    hours: Vec<Hour>,
}

/// Whether a mask names any of `columns`; a column past the mask's 64 counts as named.
fn names(mask: u64, columns: &[usize]) -> bool {
    columns
        .iter()
        .any(|&column| column >= 64 || mask >> column & 1 != 0)
}

impl Projector {
    pub fn new(format: u16) -> io::Result<Self> {
        Ok(Self {
            format,
            fields: Fields {
                country_tag: columns::field(TableId::Countries, "tag")?,
                country_slot: columns::field(TableId::Countries, "canonical_slot")?,
                link_slot: columns::field(TableId::CountryLinks, "canonical_slot")?,
                sample_hour: columns::field(TableId::SliceReads, "sample_hour")?,
            },
            roster: BTreeMap::new(),
            slots: BTreeMap::new(),
            available: None,
            hours: Vec::new(),
        })
    }

    /// Whether the hour changed what the roster reads: a country's tag or slot, a country
    /// or link added or removed, or a link's slot.
    fn roster_changed(&self, state: &State) -> io::Result<bool> {
        let countries = columns::changes(state, TableId::Countries)?;
        let links = columns::changes(state, TableId::CountryLinks)?;
        let country_fields = [self.fields.country_tag, self.fields.country_slot];
        let link_fields = [self.fields.link_slot];
        Ok(!countries.inserted.is_empty()
            || !countries.deleted.is_empty()
            || countries
                .masks
                .iter()
                .any(|&mask| names(mask, &country_fields))
            || countries
                .fill_masks
                .iter()
                .any(|&mask| names(mask, &country_fields))
            || !links.inserted.is_empty()
            || !links.deleted.is_empty()
            || links.masks.iter().any(|&mask| names(mask, &link_fields))
            || links
                .fill_masks
                .iter()
                .any(|&mask| names(mask, &link_fields)))
    }

    /// The roster from every country and link, as [`super::roster`] makes it from whole tables.
    fn rebuild(&mut self, state: &State, available: &[bool; 5]) -> io::Result<()> {
        self.roster.clear();
        self.slots.clear();
        if !(available[0] && available[1]) {
            return Ok(());
        }
        let countries: Vec<CountryRow> = rows(state, TableId::Countries, self.format)?;
        let links: Vec<CountryLinkRow> = rows(state, TableId::CountryLinks, self.format)?;
        let (roster, slots) = super::roster(&countries, &links);
        self.roster = roster
            .into_iter()
            .map(|(id, (tag, slot))| (id, (Arc::from(tag), slot)))
            .collect();
        self.slots = slots;
        Ok(())
    }
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

/// Visits the keys of the rows the hour changed, inserted or filled in `table`, and of those
/// it deleted: whether it inserted or deleted any.
fn touched(state: &State, table: TableId, mut visit: impl FnMut(u128)) -> io::Result<bool> {
    let held = columns::table(state, table)?;
    let changes = columns::changes(state, table)?;
    for &slot in changes
        .changed
        .iter()
        .chain(&changes.inserted)
        .chain(&changes.fill_slots)
    {
        if let Some(key) = held.key(slot) {
            visit(key);
        }
    }
    changes.deleted_keys.iter().copied().for_each(&mut visit);
    Ok(!changes.inserted.is_empty() || !changes.deleted.is_empty())
}

impl Operator for Projector {
    fn hour(&mut self, hour: &segments::Hour<'_>) -> io::Result<()> {
        let state = hour.state;
        let inputs_truncated = columns::truncated(state, TableId::CountryStabilityInputs);
        let available = [
            columns::complete(state, TableId::Countries),
            columns::complete(state, TableId::CountryLinks),
            columns::complete(state, TableId::CountryStabilityInputs) || inputs_truncated.is_some(),
            columns::complete(state, TableId::SliceReads),
            columns::complete(state, TableId::CountryCommandPower),
        ];
        // A truncated read names its own rows each hour: every country may change.
        let full = hour.starts || self.available != Some(available) || inputs_truncated.is_some();
        self.available = Some(available);
        let mut dirty = BTreeSet::new();
        let mut membership = full;
        if full || self.roster_changed(state)? {
            dirty.extend(self.roster.keys().copied());
            self.rebuild(state, &available)?;
            dirty.extend(self.roster.keys().copied());
            membership = true;
        }
        membership |= touched(state, TableId::CountryStabilityInputs, |key| {
            dirty.insert(key as u32);
        })?;
        let of_inputs = |key: u128| (key >> 32) as u32 == TableId::CountryStabilityInputs as u32;
        let reads = columns::changes(state, TableId::SliceReads)?;
        let reads_table = columns::table(state, TableId::SliceReads)?;
        for &slot in reads.inserted.iter() {
            membership |= reads_table.key(slot).is_some_and(of_inputs);
        }
        membership |= reads.deleted_keys.iter().any(|&key| of_inputs(key));
        touched(state, TableId::SliceReads, |key| {
            if of_inputs(key) {
                dirty.insert(key as u32);
            }
        })?;
        touched(state, TableId::CountryCommandPower, |key| {
            dirty.extend(self.slots.get(&(key as u32)).into_iter().flatten().copied());
        })?;

        let inputs = columns::table(state, TableId::CountryStabilityInputs)?;
        let input_slot = |id: u32| -> Option<u32> {
            if !available[2] {
                return None;
            }
            let slot = inputs.slot(u128::from(id))?;
            match inputs_truncated {
                Some(read) => read.binary_search(&slot).is_ok().then_some(slot),
                None => Some(slot),
            }
        };
        let sample_hours = columns::plain::<i32>(reads_table, self.fields.sample_hour)?;
        let read_hour = |id: u32| -> Option<i32> {
            if !available[3] {
                return None;
            }
            let key = (TableId::CountryStabilityInputs as u128) << 32 | u128::from(id);
            let slot = reads_table.slot(key)?;
            sample_hours.get(slot as usize).copied()
        };
        let valid = membership.then(|| {
            self.roster
                .iter()
                .filter(|(id, _)| input_slot(**id).is_some() && read_hour(**id).is_some())
                .map(|(_, (tag, _))| tag.to_string())
                .collect()
        });
        let command = columns::table(state, TableId::CountryCommandPower)?;
        let mut countries = Vec::new();
        for id in dirty {
            let Some((tag, slot)) = self.roster.get(&id) else {
                continue;
            };
            let (Some(at), Some(read)) = (input_slot(id), read_hour(id)) else {
                continue;
            };
            let row: CountryStabilityInputRow =
                columns::row(state, TableId::CountryStabilityInputs, at, self.format)?;
            let held: Option<CountryCommandPowerRow> = match available[4]
                .then(|| command.slot(u128::from(*slot)))
                .flatten()
            {
                Some(at) => Some(columns::row(
                    state,
                    TableId::CountryCommandPower,
                    at,
                    self.format,
                )?),
                None => None,
            };
            let values = super::measures(&row, held.as_ref());
            countries.push(Projected {
                id,
                tag: Arc::clone(tag),
                slot: *slot,
                read,
                row,
                values,
            });
        }
        self.hours.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            valid,
            countries,
        });
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(self.hours))
    }
}
