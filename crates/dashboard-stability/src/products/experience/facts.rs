//! Experience facts, projected hour by hour from a columnar walk ([`replay::segments`]): the
//! countries holding exactly one tag, and each canonical slot's banked experience and gross
//! gains as its one CountryExperience and CountryExperienceGains rows give them, from the
//! walk's countries ([`dashboard_world`]) and the rows the hour changed. Only the slots whose
//! joined values changed leave an hour; a segment's first hour, an hour after a skipped one,
//! an hour a table becomes complete or stops being and an hour the links move hand on every
//! slot.

use dashboard_text::metadata::slot_read;
use dashboard_world::{Needs, Roster, Tag, Tagged, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryExperienceGainRow, CountryExperienceRow, TableId},
};
use std::{collections::HashMap, io, mem::size_of, sync::Arc};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryExperience,
    TableId::CountryExperienceGains,
];

/// What a projection reads of its walk's world.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// Army, navy and air experience.
pub type Amounts = [i64; 3];

/// One projected hour.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// CountryExperience's daily read is dated at this hour.
    pub read: bool,
    /// Tags the projection numbers from this hour on, by index.
    pub named: Vec<(u32, Arc<str>)>,
    /// Each country holding exactly one tag, by canonical slot and id, with its tag's index,
    /// ascending, when they may have changed.
    pub holders: Option<Vec<((u32, u32), u32)>>,
    /// [`Self::banked`] and [`Self::gained`] name every slot that holds a value, not only the
    /// slots whose values changed.
    pub whole: bool,
    /// Slots' banked experience: None without exactly one row.
    pub banked: Vec<(u32, Option<Amounts>)>,
    /// Slots' gross gains: None without exactly one row, or where it holds no complete interval.
    pub gained: Vec<(u32, Option<Amounts>)>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.named.capacity() * size_of::<(u32, Arc<str>)>()
            + self.named.iter().map(|(_, tag)| tag.len()).sum::<usize>()
            + self.holders.as_ref().map_or(0, |holders| {
                holders.capacity() * size_of::<((u32, u32), u32)>()
            })
            + (self.banked.capacity() + self.gained.capacity())
                * size_of::<(u32, Option<Amounts>)>()
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// One segment's projection.
pub struct Projector {
    format: FormatVersion,
    /// CountryExperience's army, navy and air banked columns.
    banked_fields: [usize; 3],
    /// CountryExperienceGains' army, navy and air columns, then its completeness.
    gain_fields: [usize; 4],
    numbers: HashMap<Tag, u32>,
    last: Option<i32>,
    /// Whether the countries, CountryExperience and CountryExperienceGains held every row the
    /// hour before.
    available: Option<[bool; 3]>,
    /// Each raw country's CountryExperienceGains instances, ascending.
    instances: HashMap<u32, Vec<u32>>,
    /// What the projection last handed on of each slot.
    banked: HashMap<u32, Amounts>,
    gained: HashMap<u32, Amounts>,
    /// The hour's slots to join again, ascending once sorted.
    slots: Vec<u32>,
}

/// The bits of `fields` in a change mask; a field past the mask's 64 is in every mask.
fn mask(fields: &[usize]) -> u64 {
    fields
        .iter()
        .map(|&field| 1_u64.checked_shl(field as u32).unwrap_or(u64::MAX))
        .fold(0, |mask, bit| mask | bit)
}

/// Visits the key of every row of `table` the hour inserted or deleted, and of every row whose
/// written columns meet `fields`.
fn touched(
    state: &State,
    table: TableId,
    fields: u64,
    mut visit: impl FnMut(u128),
) -> io::Result<()> {
    let held = columns::table(state, table)?;
    let changes = columns::changes(state, table)?;
    let meets = |masks: &[u64], at: usize| masks.get(at).is_none_or(|mask| mask & fields != 0);
    for (at, &slot) in changes.changed.iter().enumerate() {
        if meets(&changes.masks, at)
            && let Some(key) = held.key(slot)
        {
            visit(key);
        }
    }
    for (at, &slot) in changes.fill_slots.iter().enumerate() {
        if meets(&changes.fill_masks, at)
            && let Some(key) = held.key(slot)
        {
            visit(key);
        }
    }
    for &slot in &changes.inserted {
        if let Some(key) = held.key(slot) {
            visit(key);
        }
    }
    changes.deleted_keys.iter().copied().for_each(visit);
    Ok(())
}

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        let banked = |name| columns::field(TableId::CountryExperience, name);
        let gain = |name| columns::field(TableId::CountryExperienceGains, name);
        Ok(Self {
            format,
            banked_fields: [banked("army_raw")?, banked("navy_raw")?, banked("air_raw")?],
            gain_fields: [
                gain("army_raw")?,
                gain("navy_raw")?,
                gain("air_raw")?,
                gain("complete")?,
            ],
            numbers: HashMap::new(),
            last: None,
            available: None,
            instances: HashMap::new(),
            banked: HashMap::new(),
            gained: HashMap::new(),
            slots: Vec::new(),
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

    /// The countries holding exactly one tag, as the row path joins them: each UTF-8 tag's
    /// first holder by id whose identity the links resolve, while both tables are complete.
    fn holders(
        &mut self,
        roster: &Roster,
        named: &mut Vec<(u32, Arc<str>)>,
    ) -> Vec<((u32, u32), u32)> {
        let mut holders = Vec::new();
        if !roster.complete() {
            return holders;
        }
        for tag in roster.held() {
            let Tagged::Found(country) = roster.tagged(tag) else {
                continue;
            };
            let Ok(text) = std::str::from_utf8(roster.tag(tag)) else {
                continue;
            };
            let identity = (country.slot, country.id);
            let index = self.number(tag, text, named);
            holders.push((identity, index));
        }
        holders.sort_unstable();
        // A country holding two tags holds neither.
        let mut kept = Vec::with_capacity(holders.len());
        let mut at = 0;
        while at < holders.len() {
            let identity = holders[at].0;
            let end = at + holders[at..].partition_point(|held| held.0 == identity);
            if end == at + 1 {
                kept.push(holders[at]);
            }
            at = end;
        }
        kept
    }

    /// Army, navy and air banked in CountryExperience's live `slot`, as the row decoder reads
    /// them.
    fn banked_at(&self, state: &State, slot: u32) -> io::Result<Amounts> {
        let table = columns::table(state, TableId::CountryExperience)?;
        if self
            .banked_fields
            .iter()
            .all(|&field| table.recorded(field, slot))
        {
            let mut amounts = [0; 3];
            for (amount, &field) in amounts.iter_mut().zip(&self.banked_fields) {
                *amount = *columns::plain::<i64>(table, field)?
                    .get(slot as usize)
                    .ok_or_else(|| invalid("A CountryExperience slot holds no cell"))?;
            }
            return Ok(amounts);
        }
        let row: CountryExperienceRow = columns::row(
            state,
            TableId::CountryExperience,
            slot,
            self.format.version(),
        )?;
        Ok([row.army_raw, row.navy_raw, row.air_raw])
    }

    /// The gross gains CountryExperienceGains' live `slot` holds, when its interval is
    /// complete.
    fn gained_at(&self, state: &State, slot: u32) -> io::Result<Option<Amounts>> {
        let table = columns::table(state, TableId::CountryExperienceGains)?;
        if self
            .gain_fields
            .iter()
            .all(|&field| table.recorded(field, slot))
        {
            let missing = || invalid("A CountryExperienceGains slot holds no cell");
            let complete = *columns::plain::<u8>(table, self.gain_fields[3])?
                .get(slot as usize)
                .ok_or_else(missing)?;
            if complete != 1 {
                return Ok(None);
            }
            let mut amounts = [0; 3];
            for (amount, &field) in amounts.iter_mut().zip(&self.gain_fields[..3]) {
                *amount = *columns::plain::<i64>(table, field)?
                    .get(slot as usize)
                    .ok_or_else(missing)?;
            }
            return Ok(Some(amounts));
        }
        let row: CountryExperienceGainRow = columns::row(
            state,
            TableId::CountryExperienceGains,
            slot,
            self.format.version(),
        )?;
        Ok((row.complete == 1).then_some([row.army_raw, row.navy_raw, row.air_raw]))
    }

    /// Canonical `slot`'s banked experience: its one CountryExperience row's, among the rows of
    /// the raw countries linked to it.
    fn banked_of(&self, state: &State, roster: &Roster, slot: u32) -> io::Result<Option<Amounts>> {
        let table = columns::table(state, TableId::CountryExperience)?;
        let mut found = None;
        for &raw in roster.aliases(slot) {
            if let Some(at) = table.slot(u128::from(raw)) {
                if found.is_some() {
                    return Ok(None);
                }
                found = Some(at);
            }
        }
        found.map(|at| self.banked_at(state, at)).transpose()
    }

    /// Canonical `slot`'s gross gains: its one CountryExperienceGains row's, among the rows of
    /// the raw countries linked to it.
    fn gained_of(&self, state: &State, roster: &Roster, slot: u32) -> io::Result<Option<Amounts>> {
        let table = columns::table(state, TableId::CountryExperienceGains)?;
        let mut found = None;
        for &raw in roster.aliases(slot) {
            for &instance in self.instances.get(&raw).map_or(&[][..], Vec::as_slice) {
                if found.is_some() {
                    return Ok(None);
                }
                let key = u128::from(raw) << 32 | u128::from(instance);
                found = Some(
                    table
                        .slot(key)
                        .ok_or_else(|| invalid("A gains instance holds no row"))?,
                );
            }
        }
        match found {
            Some(at) => self.gained_at(state, at),
            None => Ok(None),
        }
    }

    /// Indexes every live CountryExperienceGains row by its raw country.
    fn index_gains(&mut self, state: &State) -> io::Result<()> {
        self.instances.clear();
        let table = columns::table(state, TableId::CountryExperienceGains)?;
        for slot in table.live().ones() {
            if let Some(key) = table.key(slot) {
                self.instances
                    .entry((key >> 32) as u32)
                    .or_default()
                    .push(key as u32);
            }
        }
        for instances in self.instances.values_mut() {
            instances.sort_unstable();
        }
        Ok(())
    }

    /// Moves the hour's inserted and deleted gains rows in the index.
    fn reindex_gains(&mut self, state: &State) -> io::Result<()> {
        let table = columns::table(state, TableId::CountryExperienceGains)?;
        let changes = columns::changes(state, TableId::CountryExperienceGains)?;
        for &key in &changes.deleted_keys {
            let raw = (key >> 32) as u32;
            if let Some(instances) = self.instances.get_mut(&raw) {
                if let Ok(at) = instances.binary_search(&(key as u32)) {
                    instances.remove(at);
                }
                if instances.is_empty() {
                    self.instances.remove(&raw);
                }
            }
        }
        for &slot in &changes.inserted {
            if let Some(key) = table.key(slot) {
                let instances = self.instances.entry((key >> 32) as u32).or_default();
                if let Err(at) = instances.binary_search(&(key as u32)) {
                    instances.insert(at, key as u32);
                }
            }
        }
        Ok(())
    }
}

/// Sets `held` to `value` at `slot`: whether that changed it.
fn hold(held: &mut HashMap<u32, Amounts>, slot: u32, value: Option<Amounts>) -> bool {
    match value {
        Some(value) => held.insert(slot, value) != Some(value),
        None => held.remove(&slot).is_some(),
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
        let available = [
            roster.complete(),
            columns::complete(state, TableId::CountryExperience),
            columns::complete(state, TableId::CountryExperienceGains),
        ];
        let skipped = self
            .last
            .is_some_and(|last| last.checked_add(1) != Some(hour.hour));
        self.last = Some(hour.hour);
        let whole = hour.starts
            || skipped
            || self.available != Some(available)
            || roster.rebuilt()
            || !roster.changed_links().is_empty();
        self.available = Some(available);
        let read = slot_read(
            self.format,
            &metrics(hour.manifest),
            TableId::CountryExperience,
            hour.hour,
        );
        let mut named = Vec::new();
        let holders = (whole || roster.changed()).then(|| self.holders(roster, &mut named));
        let (mut banked, mut gained) = (Vec::new(), Vec::new());
        if whole {
            if available[2] {
                self.index_gains(state)?;
            } else {
                self.instances.clear();
            }
            self.banked.clear();
            self.gained.clear();
            let mut slots = std::mem::take(&mut self.slots);
            slots.clear();
            slots.extend(roster.links().iter().map(|link| link.canonical_slot));
            slots.sort_unstable();
            slots.dedup();
            for &slot in &slots {
                if available[1]
                    && let Some(value) = self.banked_of(state, roster, slot)?
                {
                    self.banked.insert(slot, value);
                    banked.push((slot, Some(value)));
                }
                if available[2]
                    && let Some(value) = self.gained_of(state, roster, slot)?
                {
                    self.gained.insert(slot, value);
                    gained.push((slot, Some(value)));
                }
            }
            self.slots = slots;
        } else {
            if available[1] {
                let mut slots = std::mem::take(&mut self.slots);
                slots.clear();
                touched(
                    state,
                    TableId::CountryExperience,
                    mask(&self.banked_fields),
                    |key| slots.extend(roster.slot_of(key as u32)),
                )?;
                slots.sort_unstable();
                slots.dedup();
                for &slot in &slots {
                    let value = self.banked_of(state, roster, slot)?;
                    if hold(&mut self.banked, slot, value) {
                        banked.push((slot, value));
                    }
                }
                self.slots = slots;
            }
            if available[2] {
                self.reindex_gains(state)?;
                let mut slots = std::mem::take(&mut self.slots);
                slots.clear();
                touched(
                    state,
                    TableId::CountryExperienceGains,
                    mask(&self.gain_fields),
                    |key| slots.extend(roster.slot_of((key >> 32) as u32)),
                )?;
                slots.sort_unstable();
                slots.dedup();
                for &slot in &slots {
                    let value = self.gained_of(state, roster, slot)?;
                    if hold(&mut self.gained, slot, value) {
                        gained.push((slot, value));
                    }
                }
                self.slots = slots;
            }
        }
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            read,
            named,
            holders,
            whole,
            banked,
            gained,
        })
    }
}
