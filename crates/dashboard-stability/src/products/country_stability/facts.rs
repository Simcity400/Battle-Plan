//! Stability's measure facts, projected hour by hour from a columnar walk
//! ([`replay::segments`]): each country whose displayed stability, war support or maximum
//! command power may have changed, with its holder and supplying read, from the walker's
//! tables and the walk's countries ([`dashboard_world`]). Inputs rows are read only for those
//! countries, and only their measures leave the projection. A segment's first hour, and an
//! hour after a skipped one, projects every country.

use dashboard_world::{Needs, Roster, Tag, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::rows::{CountryCommandPowerRow, CountryStabilityInputRow, TableId};
use std::{
    collections::{BTreeSet, HashMap},
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

/// What a projection reads of its walk's world.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// One country's measures at a projected hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Country {
    /// Its tag's index in the projection ([`Hour::named`]).
    pub index: u32,
    /// Its canonical slot and id.
    pub holder: (u32, u32),
    /// Its inputs' read hour.
    pub read: i32,
    /// Stability, war support and maximum command power, fixed5.
    pub values: [Option<i64>; 3],
}

/// One projected hour.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Tags the projection numbers from this hour on, by index.
    pub named: Vec<(u32, Arc<str>)>,
    /// The tags with a current holder, inputs and a read, ascending, when they may have
    /// changed.
    pub valid: Option<Vec<u32>>,
    /// Countries whose measures may have changed, at least every one that did.
    pub countries: Vec<Country>,
}

/// One segment's projection.
pub struct Projector {
    format: u16,
    sample_hour: usize,
    /// Which of [`TABLES`] held rows the hour before.
    available: Option<[bool; 5]>,
    numbers: HashMap<Tag, u32>,
    last: Option<i32>,
}

/// The country of `id` and its tag, when it holds the roster's place for its tag: the tag's
/// first holder by id, even unresolved; only a resolved holder with a UTF-8 tag has measures.
fn member(roster: &Roster, id: u32) -> Option<(Tag, (u32, u32), &str)> {
    let country = roster.get(id)?;
    if !country.resolved || roster.holders(country.tag).next()?.id != id {
        return None;
    }
    Some((
        country.tag,
        (country.slot, country.id),
        std::str::from_utf8(roster.tag(country.tag)).ok()?,
    ))
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

impl Projector {
    pub fn new(format: u16) -> io::Result<Self> {
        Ok(Self {
            format,
            sample_hour: columns::field(TableId::SliceReads, "sample_hour")?,
            available: None,
            numbers: HashMap::new(),
            last: None,
        })
    }

    fn number(&mut self, tag: Tag, text: &str, named: &mut Vec<(u32, Arc<str>)>) -> u32 {
        if let Some(&index) = self.numbers.get(&tag) {
            return index;
        }
        let index = self.numbers.len() as u32;
        self.numbers.insert(tag, index);
        named.push((index, Arc::from(text)));
        index
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
        }
        let state = hour.state;
        let roster = hour.world.countries()?;
        let inputs_truncated = columns::truncated(state, TableId::CountryStabilityInputs);
        let available = [
            roster.countries_complete(),
            roster.links_complete(),
            columns::complete(state, TableId::CountryStabilityInputs) || inputs_truncated.is_some(),
            columns::complete(state, TableId::SliceReads),
            columns::complete(state, TableId::CountryCommandPower),
        ];
        let skipped = self
            .last
            .is_some_and(|last| last.checked_add(1) != Some(hour.hour));
        self.last = Some(hour.hour);
        // A truncated read names its own rows each hour: every country may change.
        let full = hour.starts
            || skipped
            || self.available != Some(available)
            || inputs_truncated.is_some();
        self.available = Some(available);
        let mut dirty = BTreeSet::new();
        let mut membership = full;
        if full || roster.rebuilt() {
            dirty.extend(roster.iter().map(|country| country.id));
            membership = true;
        } else if roster.changed() {
            // A changed tag may give its place to another of its holders.
            dirty.extend(roster.changed_ids());
            for &tag in roster.changed_tags() {
                dirty.extend(roster.holders(tag).map(|country| country.id));
            }
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
            dirty.extend(roster.at_slot(key as u32).map(|country| country.id));
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
        let sample_hours = columns::plain::<i32>(reads_table, self.sample_hour)?;
        let read_hour = |id: u32| -> Option<i32> {
            if !available[3] {
                return None;
            }
            let key = (TableId::CountryStabilityInputs as u128) << 32 | u128::from(id);
            let slot = reads_table.slot(key)?;
            sample_hours.get(slot as usize).copied()
        };
        let complete = roster.complete();
        let mut named = Vec::new();
        let valid = if membership {
            let mut valid = Vec::new();
            if complete {
                for country in roster.iter() {
                    let Some((tag, _, text)) = member(roster, country.id) else {
                        continue;
                    };
                    if input_slot(country.id).is_some() && read_hour(country.id).is_some() {
                        valid.push(self.number(tag, text, &mut named));
                    }
                }
            }
            valid.sort_unstable();
            valid.dedup();
            Some(valid)
        } else {
            None
        };
        let command = columns::table(state, TableId::CountryCommandPower)?;
        let mut countries = Vec::new();
        for id in dirty {
            let Some((tag, holder, text)) = member(roster, id).filter(|_| complete) else {
                continue;
            };
            let (Some(at), Some(read)) = (input_slot(id), read_hour(id)) else {
                continue;
            };
            let row: CountryStabilityInputRow =
                columns::row(state, TableId::CountryStabilityInputs, at, self.format)?;
            let held: Option<CountryCommandPowerRow> = match available[4]
                .then(|| command.slot(u128::from(holder.0)))
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
            countries.push(Country {
                index: self.number(tag, text, &mut named),
                holder,
                read,
                values: super::measures(&row, held.as_ref()),
            });
        }
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            named,
            valid,
            countries,
        })
    }
}
