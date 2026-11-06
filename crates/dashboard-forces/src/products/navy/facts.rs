//! Navy size facts, projected hour by hour from a columnar walk ([`replay::segments`]): the ship
//! census by holder slot and ownership, kept from each hour's changed ships, task forces and ship
//! models and sent only for the slots whose counts changed; the tags the walk's countries resolve
//! ([`dashboard_world`]) where they changed; and the spelling of each ship class token the census
//! holds, where it changed. Counts name classes by token: the ordered emit ([`super::emit`]) names
//! each by its token and the spelling then, as the row census does.

use dashboard_world::{Needs, Roster, Tagged, World};
use replay::{
    columns,
    container::Replay,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{LexicalTokenRow, ShipModelRow, ShipRow, TableId, TaskForceRow},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::TaskForces,
    TableId::Ships,
    TableId::ShipModels,
    TableId::LexicalTokens,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// A slot's ships under one ownership, as the row census sums them: ships, current hull HP in
/// fixed5 (none where a ship's is negative or the sum leaves i64), maximum hull HP (none while a
/// ship's model counter is missing or the sum leaves i64), whether the maximum's ordered sum
/// overflows, and by class token ascending its ships and their hull HP.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct Counts {
    pub ships: i64,
    pub hp: Option<i64>,
    pub maximum: Option<i64>,
    pub overflow: bool,
    pub classes: Vec<(u32, i64, i64)>,
}

impl Counts {
    /// A slot holding no ship.
    pub fn empty() -> Self {
        Self {
            ships: 0,
            hp: Some(0),
            maximum: Some(0),
            overflow: false,
            classes: Vec::new(),
        }
    }

    pub(super) fn heap(&self) -> usize {
        self.classes.capacity() * size_of::<(u32, i64, i64)>()
    }
}

/// One projected hour.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Tags the projection numbers from this hour on, by index.
    pub named: Vec<(u32, Arc<str>)>,
    /// The tags whose countries are live, with their slot and id, where they may have changed.
    pub live: Option<Vec<(u32, (u32, u32))>>,
    /// The tags the countries hold that are not UTF-8, counted each hour the countries are
    /// complete, where they changed.
    pub undisplayable: Option<Vec<Vec<u8>>>,
    /// Whether the countries were complete, so the census was read at all.
    pub census: bool,
    /// The fingerprint of the country join the row census reads (each tag's country, and the
    /// links naming a found country's slot), where the hour read it: the emit compares it over
    /// the branch, across segments.
    pub join: Option<u64>,
    /// Whether the census read the ships; an hour it did not is counted unread.
    pub read: bool,
    /// The ships no owner was found for, by ownership, while read.
    pub unattributed: [u64; 2],
    /// Whether each ownership's census holds every ship, attributed.
    pub complete: [bool; 2],
    /// Slot counts by ownership: every slot holding ships when `whole` (the others hold none),
    /// else those that changed.
    pub whole: bool,
    pub counts: Vec<(u8, u32, Arc<Counts>)>,
    /// Class token spellings (none where LexicalTokens names none): every token the census holds
    /// when `spelled_whole` (the others are unknown), else those new or changed.
    pub spelled_whole: bool,
    pub spellings: Vec<(u32, Option<Arc<str>>)>,
    /// The class tokens the hour may name anew, in the order the row census first reads them:
    /// by the lowest ship id holding each among the ships it reads.
    pub seen: Vec<u32>,
}

/// One ship as counted: its class token, task force, hull HP, maximum hull HP (none without a
/// model counter) and its owners' slots (none where unattributed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ship {
    token: u32,
    force: u64,
    hp: i64,
    maximum: Option<i64>,
    owners: [Option<u32>; 2],
}

/// A task force's raw roster and logical owner links, and the slots they name.
#[derive(Clone, Copy)]
struct Force {
    raw: [u32; 2],
    owners: [Option<u32>; 2],
}

/// Reversible sums of a slot's ships under one ownership; wide sums subtract exactly after an
/// overflowing read.
#[derive(Default)]
struct Total {
    ships: i64,
    hp: i128,
    negative: i64,
    maximum: i128,
    missing: i64,
    classes: BTreeMap<u32, (i64, i128)>,
}

impl Total {
    fn change(&mut self, ship: &Ship, sign: i64) {
        self.ships += sign;
        self.hp += i128::from(sign) * i128::from(ship.hp);
        self.negative += sign * i64::from(ship.hp < 0);
        match ship.maximum {
            Some(maximum) => self.maximum += i128::from(sign) * i128::from(maximum),
            None => self.missing += sign,
        }
        let class = self.classes.entry(ship.token).or_default();
        class.0 += sign;
        class.1 += i128::from(sign) * i128::from(ship.hp);
        if class.0 == 0 {
            self.classes.remove(&ship.token);
        }
    }
}

/// The fields a projection reads in place.
struct Fields {
    task_force: usize,
    strength: usize,
    class: usize,
    roster_owner: usize,
    logical_owner: usize,
    counter: usize,
    literal: usize,
    /// Ships' task_force, strength and class_id, as change-mask bits.
    ship_mask: u64,
    force_mask: u64,
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    /// Nothing is held: the next census is read whole.
    cold: bool,
    last: Option<i32>,
    /// Whether ShipModels and TaskForces were complete at the last read.
    models: Option<bool>,
    forces_complete: Option<bool>,
    /// Whether LexicalTokens was complete when the spellings were last sent.
    lexical: Option<bool>,
    /// Each tag's index, and the tags sent live.
    numbers: HashMap<Arc<str>, u32>,
    live: Option<Vec<(u32, (u32, u32))>>,
    /// The slots a found tag holds.
    present: HashSet<u32>,
    undisplayable: Option<Vec<Vec<u8>>>,
    forces: HashMap<u64, Force>,
    /// Each task force's ships, by task force id, whether or not the force is held.
    fleets: HashMap<u64, Vec<u64>>,
    ships: HashMap<u64, Ship>,
    /// Each ship's model maximum, by ship id: none where its counter is missing.
    maxima: HashMap<u64, Option<i64>>,
    totals: [HashMap<u32, Total>; 2],
    unattributed: [i64; 2],
    /// The counts last sent, by ownership and slot.
    sent: [HashMap<u32, Arc<Counts>>; 2],
    dirty: [HashSet<u32>; 2],
    /// Each class token the census held, with the spelling last sent; the tokens new since.
    tokens: HashMap<u32, Option<Arc<str>>>,
    fresh: Vec<u32>,
    /// The ships the hour counted.
    touched: Vec<u64>,
    /// The fingerprint of the country join last read; none after countries not complete.
    join: Option<u64>,
}

/// The row census's country join: each held tag with its found country, and every link naming
/// a found country's slot.
#[derive(Hash)]
struct Join {
    tags: Vec<(Vec<u8>, Option<(u32, u32)>)>,
    owners: Vec<(u32, u32)>,
}

impl Join {
    /// The join's fingerprint, the same for equal joins whatever order a walk numbered tags in.
    fn fingerprint(roster: &Roster) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        Self::read(roster).hash(&mut hasher);
        hasher.finish()
    }

    fn read(roster: &Roster) -> Self {
        let mut found = HashSet::new();
        let mut tags: Vec<(Vec<u8>, Option<(u32, u32)>)> = roster
            .held()
            .map(|tag| {
                let country = match roster.tagged(tag) {
                    Tagged::Found(country) => {
                        found.insert(country.slot);
                        Some((country.slot, country.id))
                    }
                    _ => None,
                };
                (roster.tag(tag).to_vec(), country)
            })
            .collect();
        tags.sort_unstable();
        let mut owners: Vec<(u32, u32)> = roster
            .links()
            .iter()
            .filter(|link| found.contains(&link.canonical_slot))
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        owners.sort_unstable();
        Self { tags, owners }
    }
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"navy projector: cold";

/// Whether the hour inserted, deleted or changed a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

/// The slots of `table` the hour inserted or wrote any of `mask` at.
fn written(state: &State, table: TableId, mask: u64) -> io::Result<Vec<u32>> {
    let changes = columns::changes(state, table)?;
    let mut slots: Vec<u32> = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|(_, written)| **written & mask != 0)
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied())
        .collect();
    slots.sort_unstable();
    slots.dedup();
    Ok(slots)
}

fn cell<T: Copy>(values: &[T], slot: u32) -> io::Result<T> {
    values
        .get(slot as usize)
        .copied()
        .ok_or_else(|| io::Error::other("A navy table's slot holds no cell"))
}

impl Projector {
    pub fn new(replay: &Replay) -> io::Result<Self> {
        Self::with(replay.format)
    }

    pub fn with(format: FormatVersion) -> io::Result<Self> {
        let field = |table, name| columns::field(table, name);
        let task_force = field(TableId::Ships, "task_force")?;
        let strength = field(TableId::Ships, "strength")?;
        let class = field(TableId::Ships, "class_id")?;
        let roster_owner = field(TableId::TaskForces, "roster_owner")?;
        let logical_owner = field(TableId::TaskForces, "logical_owner")?;
        Ok(Self {
            format,
            fields: Fields {
                task_force,
                strength,
                class,
                roster_owner,
                logical_owner,
                counter: field(TableId::ShipModels, "counter")?,
                literal: field(TableId::LexicalTokens, "literal")?,
                ship_mask: 1 << task_force | 1 << strength | 1 << class,
                force_mask: 1 << roster_owner | 1 << logical_owner,
            },
            cold: true,
            last: None,
            models: None,
            forces_complete: None,
            lexical: None,
            numbers: HashMap::new(),
            live: None,
            present: HashSet::new(),
            undisplayable: None,
            forces: HashMap::new(),
            fleets: HashMap::new(),
            ships: HashMap::new(),
            maxima: HashMap::new(),
            totals: Default::default(),
            unattributed: [0; 2],
            sent: Default::default(),
            dirty: Default::default(),
            tokens: HashMap::new(),
            fresh: Vec::new(),
            touched: Vec::new(),
            join: None,
        })
    }

    /// The tags the countries resolve, with their slots, and the tags that are not UTF-8.
    fn countries(
        &mut self,
        roster: &Roster,
        named: &mut Vec<(u32, Arc<str>)>,
    ) -> (Vec<(u32, (u32, u32))>, Vec<Vec<u8>>) {
        let mut live = Vec::new();
        let mut undisplayable = Vec::new();
        let complete = roster.complete();
        self.present.clear();
        for tag in roster.held() {
            let bytes = roster.tag(tag);
            let Ok(text) = std::str::from_utf8(bytes) else {
                undisplayable.push(bytes.to_vec());
                if complete && let Tagged::Found(country) = roster.tagged(tag) {
                    self.present.insert(country.slot);
                }
                continue;
            };
            if !complete {
                continue;
            }
            let Tagged::Found(country) = roster.tagged(tag) else {
                continue;
            };
            self.present.insert(country.slot);
            let index = match self.numbers.get(text) {
                Some(&index) => index,
                None => {
                    let index = self.numbers.len() as u32;
                    let tag: Arc<str> = Arc::from(text);
                    self.numbers.insert(Arc::clone(&tag), index);
                    named.push((index, tag));
                    index
                }
            };
            live.push((index, (country.slot, country.id)));
        }
        live.sort_unstable();
        undisplayable.sort_unstable();
        (live, undisplayable)
    }

    /// The slot `raw` counts for: its link's, where a found tag holds that slot.
    fn owner(&self, roster: &Roster, raw: u32) -> Option<u32> {
        roster
            .slot_of(raw)
            .filter(|slot| self.present.contains(slot))
    }

    /// The owners of task force `force`: none for a force not held.
    fn owners(&self, force: u64) -> [Option<u32>; 2] {
        self.forces
            .get(&force)
            .map_or([None; 2], |force| force.owners)
    }

    /// Counts `ship` in (`sign` 1) or out (-1).
    fn count(&mut self, ship: &Ship, sign: i64) {
        for (channel, owner) in ship.owners.iter().enumerate() {
            match owner {
                Some(slot) => {
                    let total = self.totals[channel].entry(*slot).or_default();
                    total.change(ship, sign);
                    if total.ships == 0 {
                        self.totals[channel].remove(slot);
                    }
                    self.dirty[channel].insert(*slot);
                }
                None => self.unattributed[channel] += sign,
            }
        }
        if sign > 0 && !self.tokens.contains_key(&ship.token) {
            self.tokens.insert(ship.token, None);
            self.fresh.push(ship.token);
        }
    }

    /// Replaces ship `id`'s census entry by `ship`, keeping its task force's index.
    fn set(&mut self, id: u64, ship: Option<Ship>) {
        let old = match ship {
            Some(ship) => self.ships.insert(id, ship),
            None => self.ships.remove(&id),
        };
        if old == ship {
            return;
        }
        if let Some(old) = &old {
            self.count(old, -1);
        }
        if let Some(new) = &ship {
            self.count(new, 1);
            self.touched.push(id);
        }
        let (from, to) = (old.map(|ship| ship.force), ship.map(|ship| ship.force));
        if from != to {
            if let Some(force) = from
                && let Some(ids) = self.fleets.get_mut(&force)
            {
                ids.retain(|&held| held != id);
                if ids.is_empty() {
                    self.fleets.remove(&force);
                }
            }
            if let Some(force) = to {
                self.fleets.entry(force).or_default().push(id);
            }
        }
    }

    /// The ship at Ships `slot`: its id, task force, hull HP and class token.
    fn ship_row(&self, state: &State, slot: u32) -> io::Result<(u64, u64, i64, u32)> {
        let table = TableId::Ships;
        let held = columns::table(state, table)?;
        let id = held
            .key(slot)
            .ok_or_else(|| io::Error::other("A Ships slot holds no row"))? as u64;
        let fields = &self.fields;
        if [fields.task_force, fields.strength, fields.class]
            .iter()
            .all(|&field| held.recorded(field, slot))
        {
            return Ok((
                id,
                cell(columns::plain::<u64>(held, fields.task_force)?, slot)?,
                cell(columns::plain::<i64>(held, fields.strength)?, slot)?,
                cell(columns::plain::<i32>(held, fields.class)?, slot)? as u32,
            ));
        }
        let row: ShipRow = columns::row(state, table, slot, self.format.version())?;
        Ok((id, row.task_force, row.strength, row.class_id as u32))
    }

    /// The ship at Ships `slot` as counted.
    fn ship(&self, state: &State, slot: u32, models: bool) -> io::Result<(u64, Ship)> {
        let (id, force, hp, token) = self.ship_row(state, slot)?;
        Ok((
            id,
            Ship {
                token,
                force,
                hp,
                maximum: if models {
                    self.maxima.get(&id).copied().flatten()
                } else {
                    None
                },
                owners: self.owners(force),
            },
        ))
    }

    /// The task force at TaskForces `slot`: its id and raw owner links.
    fn force_row(&self, state: &State, slot: u32) -> io::Result<(u64, [u32; 2])> {
        let table = TableId::TaskForces;
        let held = columns::table(state, table)?;
        let id = held
            .key(slot)
            .ok_or_else(|| io::Error::other("A TaskForces slot holds no row"))?
            as u64;
        let fields = &self.fields;
        if [fields.roster_owner, fields.logical_owner]
            .iter()
            .all(|&field| held.recorded(field, slot))
        {
            return Ok((
                id,
                [
                    cell(columns::plain::<u32>(held, fields.roster_owner)?, slot)?,
                    cell(columns::plain::<u32>(held, fields.logical_owner)?, slot)?,
                ],
            ));
        }
        let row: TaskForceRow = columns::row(state, table, slot, self.format.version())?;
        Ok((id, [row.roster_owner, row.logical_owner]))
    }

    /// The model at ShipModels `slot`: its ship and maximum hull HP, which clamps the model
    /// counter's word to 100 (fixed5); none without a counter.
    fn model(&self, state: &State, slot: u32) -> io::Result<(u64, Option<i64>)> {
        let row: ShipModelRow =
            columns::row(state, TableId::ShipModels, slot, self.format.version())?;
        Ok((
            row.ship,
            row.counter
                .map(|counter| counter.maximum_strength.max(100)),
        ))
    }

    /// Re-counts the ships of task force `force` under `owners`.
    fn reattach(&mut self, force: u64) {
        let owners = self.owners(force);
        let Some(ids) = self.fleets.get(&force) else {
            return;
        };
        for id in ids.clone() {
            if let Some(&ship) = self.ships.get(&id)
                && ship.owners != owners
            {
                self.set(id, Some(Ship { owners, ..ship }));
            }
        }
    }

    /// Reads every task force, model and ship anew.
    fn read_whole(
        &mut self,
        state: &State,
        roster: &Roster,
        models: bool,
        forces: bool,
    ) -> io::Result<()> {
        for channel in 0..2 {
            self.dirty[channel].extend(self.totals[channel].keys().copied());
            self.dirty[channel].extend(self.sent[channel].keys().copied());
        }
        self.forces.clear();
        self.fleets.clear();
        self.ships.clear();
        self.maxima.clear();
        self.totals = Default::default();
        self.unattributed = [0; 2];
        self.tokens.clear();
        self.fresh.clear();
        if forces {
            let slots: Vec<u32> = columns::table(state, TableId::TaskForces)?
                .live()
                .ones()
                .collect();
            for slot in slots {
                let (id, raw) = self.force_row(state, slot)?;
                let owners = raw.map(|raw| self.owner(roster, raw));
                self.forces.insert(id, Force { raw, owners });
            }
        }
        if models {
            let slots: Vec<u32> = columns::table(state, TableId::ShipModels)?
                .live()
                .ones()
                .collect();
            for slot in slots {
                let (ship, maximum) = self.model(state, slot)?;
                self.maxima.insert(ship, maximum);
            }
        }
        let slots: Vec<u32> = columns::table(state, TableId::Ships)?
            .live()
            .ones()
            .collect();
        for slot in slots {
            let (id, ship) = self.ship(state, slot, models)?;
            self.set(id, Some(ship));
        }
        Ok(())
    }

    /// The hour's changed task forces, models and ships.
    fn read_changes(
        &mut self,
        state: &State,
        roster: &Roster,
        models: bool,
        forces: bool,
    ) -> io::Result<()> {
        if forces && touched(state, TableId::TaskForces)? {
            let changes = columns::changes(state, TableId::TaskForces)?;
            let mut moved: Vec<u64> = Vec::new();
            for &key in &changes.deleted_keys {
                if self.forces.remove(&(key as u64)).is_some() {
                    moved.push(key as u64);
                }
            }
            for slot in written(state, TableId::TaskForces, self.fields.force_mask)? {
                let (id, raw) = self.force_row(state, slot)?;
                let owners = raw.map(|raw| self.owner(roster, raw));
                let before = self.forces.insert(id, Force { raw, owners });
                if before.is_none_or(|before| before.owners != owners) {
                    moved.push(id);
                }
            }
            for force in moved {
                self.reattach(force);
            }
        }
        if models && touched(state, TableId::ShipModels)? {
            let changes = columns::changes(state, TableId::ShipModels)?;
            let mut changed: Vec<u64> = Vec::new();
            for &key in &changes.deleted_keys {
                if self.maxima.remove(&(key as u64)).is_some() {
                    changed.push(key as u64);
                }
            }
            for slot in written(state, TableId::ShipModels, 1 << self.fields.counter)? {
                let (ship, maximum) = self.model(state, slot)?;
                if self.maxima.insert(ship, maximum) != Some(maximum) {
                    changed.push(ship);
                }
            }
            for id in changed {
                if let Some(&ship) = self.ships.get(&id) {
                    let maximum = self.maxima.get(&id).copied().flatten();
                    self.set(id, Some(Ship { maximum, ..ship }));
                }
            }
        }
        if touched(state, TableId::Ships)? {
            let changes = columns::changes(state, TableId::Ships)?;
            for &key in &changes.deleted_keys {
                self.set(key as u64, None);
            }
            for slot in written(state, TableId::Ships, self.fields.ship_mask)? {
                let (id, ship) = self.ship(state, slot, models)?;
                self.set(id, Some(ship));
            }
        }
        Ok(())
    }

    /// Every task force's owners again, after the links or the found tags changed.
    fn reattribute(&mut self, roster: &Roster) {
        let mut moved = Vec::new();
        let mut forces = std::mem::take(&mut self.forces);
        for (&id, force) in &mut forces {
            let owners = force.raw.map(|raw| self.owner(roster, raw));
            if owners != force.owners {
                force.owners = owners;
                moved.push(id);
            }
        }
        self.forces = forces;
        for force in moved {
            self.reattach(force);
        }
    }

    /// Whether the maximum's sum overflows as the row census folds it, by ship id until the first
    /// ship without a counter.
    fn overflows(&self, channel: usize, slot: u32) -> bool {
        let mut held: Vec<(u64, Option<i64>)> = self
            .ships
            .iter()
            .filter(|(_, ship)| ship.owners[channel] == Some(slot))
            .map(|(&id, ship)| (id, ship.maximum))
            .collect();
        held.sort_unstable_by_key(|&(id, _)| id);
        held.iter()
            .map_while(|&(_, maximum)| maximum)
            .map(i128::from)
            .sum::<i128>()
            > i128::from(i64::MAX)
    }

    /// The counts of `slot` under `channel`'s ownership; none for a slot holding no ship.
    fn slot_counts(&self, channel: usize, slot: u32) -> Option<Counts> {
        let total = self.totals[channel].get(&slot)?;
        Some(Counts {
            ships: total.ships,
            hp: (total.negative == 0)
                .then(|| i64::try_from(total.hp).ok())
                .flatten(),
            maximum: (total.missing == 0)
                .then(|| i64::try_from(total.maximum).ok())
                .flatten(),
            overflow: total.maximum > i128::from(i64::MAX) && self.overflows(channel, slot),
            classes: total
                .classes
                .iter()
                .map(|(&token, &(ships, hp))| (token, ships, i64::try_from(hp).unwrap_or(0)))
                .collect(),
        })
    }

    /// The slot counts that changed since last sent; every slot's when `whole`.
    fn counts(&mut self, whole: bool) -> Vec<(u8, u32, Arc<Counts>)> {
        let mut out = Vec::new();
        for channel in 0..2 {
            if whole {
                self.dirty[channel].clear();
                let mut sent = HashMap::with_capacity(self.totals[channel].len());
                let mut slots: Vec<u32> = self.totals[channel].keys().copied().collect();
                slots.sort_unstable();
                for slot in slots {
                    let counts = self
                        .slot_counts(channel, slot)
                        .expect("a held total counts");
                    let counts = match self.sent[channel].get(&slot) {
                        Some(held) if **held == counts => Arc::clone(held),
                        _ => Arc::new(counts),
                    };
                    out.push((channel as u8, slot, Arc::clone(&counts)));
                    sent.insert(slot, counts);
                }
                self.sent[channel] = sent;
                continue;
            }
            let mut slots: Vec<u32> = self.dirty[channel].drain().collect();
            slots.sort_unstable();
            for slot in slots {
                let counts = self.slot_counts(channel, slot);
                let counts = match (counts, self.sent[channel].get(&slot)) {
                    (Some(counts), Some(held)) if **held == counts => continue,
                    (None, None) => continue,
                    (Some(counts), _) => Arc::new(counts),
                    (None, Some(_)) => Arc::new(Counts::empty()),
                };
                if counts.ships == 0 {
                    self.sent[channel].remove(&slot);
                } else {
                    self.sent[channel].insert(slot, Arc::clone(&counts));
                }
                out.push((channel as u8, slot, counts));
            }
        }
        out
    }

    /// The spelling LexicalTokens gives `token`: none where it holds no row for it, its literal
    /// is not UTF-8 or the table is not complete.
    fn spelling(&self, state: &State, complete: bool, token: u32) -> io::Result<Option<Arc<str>>> {
        if !complete {
            return Ok(None);
        }
        let table = TableId::LexicalTokens;
        let held = columns::table(state, table)?;
        let Some(slot) = held.slot(u128::from(token)) else {
            return Ok(None);
        };
        let literal = if held.recorded(self.fields.literal, slot) {
            let text = cell(columns::text(held, self.fields.literal)?, slot)?;
            state
                .dictionary()
                .get(text)
                .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?
                .to_vec()
        } else {
            columns::row::<LexicalTokenRow>(state, table, slot, self.format.version())?.literal
        };
        Ok(std::str::from_utf8(&literal).ok().map(Arc::from))
    }

    /// The class token spellings to send: every held token's when `all` (a whole census or a
    /// change in LexicalTokens' completeness), else the new tokens' and those whose literal the
    /// hour changed.
    fn spellings(
        &mut self,
        state: &State,
        whole: bool,
    ) -> io::Result<(bool, Vec<(u32, Option<Arc<str>>)>)> {
        let table = TableId::LexicalTokens;
        let complete = columns::complete(state, table);
        let all = whole || self.lexical != Some(complete);
        self.lexical = Some(complete);
        let fresh: HashSet<u32> = std::mem::take(&mut self.fresh).into_iter().collect();
        let mut candidates: Vec<u32> = if all {
            self.tokens.keys().copied().collect()
        } else {
            let mut candidates: Vec<u32> = fresh.iter().copied().collect();
            if complete && touched(state, table)? {
                let changes = columns::changes(state, table)?;
                let held = columns::table(state, table)?;
                candidates.extend(
                    written(state, table, 1 << self.fields.literal)?
                        .into_iter()
                        .filter_map(|slot| held.key(slot).map(|key| key as u32))
                        .chain(changes.deleted_keys.iter().map(|&key| key as u32))
                        .filter(|token| self.tokens.contains_key(token)),
                );
            }
            candidates
        };
        candidates.sort_unstable();
        candidates.dedup();
        let mut out = Vec::with_capacity(candidates.len());
        for token in candidates {
            let spelling = self.spelling(state, complete, token)?;
            let held = self.tokens.insert(token, spelling.clone());
            if all || fresh.contains(&token) || held != Some(spelling.clone()) {
                out.push((token, spelling));
            }
        }
        Ok((all, out))
    }

    /// The class tokens the hour may name anew, in the order the row census reads them: those
    /// of the ships it counted, and every respelled token, each at its lowest ship id. A census
    /// reading every ship again (a whole read, or tokens respelled) reads them by ship id.
    fn seen(&mut self, whole: bool, spellings: &[(u32, Option<Arc<str>>)]) -> Vec<u32> {
        let mut lowest: HashMap<u32, u64> = HashMap::new();
        let mut see = |id: u64, ship: &Ship| {
            let held = lowest.entry(ship.token).or_insert(id);
            *held = (*held).min(id);
        };
        if whole || !spellings.is_empty() {
            for (&id, ship) in &self.ships {
                see(id, ship);
            }
            self.touched.clear();
        } else {
            // A ship deleted later in the hour is not read.
            for id in self.touched.drain(..) {
                if let Some(ship) = self.ships.get(&id) {
                    see(id, ship);
                }
            }
        }
        let mut order: Vec<(u64, u32)> = lowest.into_iter().map(|(token, id)| (id, token)).collect();
        order.sort_unstable();
        order.into_iter().map(|(_, token)| token).collect()
    }

    fn heap(&self) -> usize {
        self.numbers.capacity() * size_of::<(Arc<str>, u32)>()
            + self.numbers.keys().map(|tag| tag.len()).sum::<usize>()
            + self
                .live
                .as_ref()
                .map_or(0, |live| live.capacity() * size_of::<(u32, (u32, u32))>())
            + self.present.capacity() * 4
            + self.forces.capacity() * size_of::<(u64, Force)>()
            + self.fleets.capacity() * size_of::<(u64, Vec<u64>)>()
            + self
                .fleets
                .values()
                .map(|ids| ids.capacity() * 8)
                .sum::<usize>()
            + self.ships.capacity() * size_of::<(u64, Ship)>()
            + self.maxima.capacity() * size_of::<(u64, Option<i64>)>()
            + self
                .totals
                .iter()
                .map(|totals| {
                    totals.capacity() * size_of::<(u32, Total)>()
                        + totals
                            .values()
                            .map(|total| total.classes.len() * (size_of::<(u32, (i64, i128))>() + 16))
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self
                .sent
                .iter()
                .map(|sent| {
                    sent.capacity() * size_of::<(u32, Arc<Counts>)>()
                        + sent
                            .values()
                            .map(|counts| size_of::<Counts>() + counts.heap())
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self
                .dirty
                .iter()
                .map(|dirty| dirty.capacity() * 4)
                .sum::<usize>()
            + self.tokens.capacity() * size_of::<(u32, Option<Arc<str>>)>()
            + self
                .tokens
                .values()
                .flatten()
                .map(|spelling| spelling.len())
                .sum::<usize>()
            + self.fresh.capacity() * 4
            + self.touched.capacity() * 8
    }
}

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
        let _cpu = replay_frame::cpu::scope("navy.columnar", Some("navy"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.cold = false;
            self.live = None;
            self.undisplayable = None;
            self.models = None;
            self.forces_complete = None;
            self.lexical = None;
        }
        self.last = Some(at);
        let mut named = Vec::new();
        let regrouped = opens || roster.changed();
        let countries = roster.countries_complete();
        let mut live = None;
        let mut undisplayable = None;
        if !countries {
            if self.live.as_ref().is_none_or(|held| !held.is_empty()) {
                self.live = Some(Vec::new());
                live = Some(Vec::new());
            }
            self.present.clear();
            // The row census counts none while the countries are not complete.
            if self
                .undisplayable
                .as_ref()
                .is_none_or(|held| !held.is_empty())
            {
                self.undisplayable = Some(Vec::new());
                undisplayable = Some(Vec::new());
            }
        } else if regrouped {
            let (found, unreadable) = self.countries(roster, &mut named);
            if self.live.as_ref() != Some(&found) {
                live = Some(found.clone());
                self.live = Some(found);
            }
            if self.undisplayable.as_ref() != Some(&unreadable) {
                undisplayable = Some(unreadable.clone());
                self.undisplayable = Some(unreadable);
            }
        }
        let mut facts = Hour {
            branch: hour.branch,
            hour: at,
            named,
            live,
            undisplayable,
            census: false,
            join: None,
            read: false,
            unattributed: [0; 2],
            complete: [false; 2],
            whole: false,
            counts: Vec::new(),
            spelled_whole: false,
            spellings: Vec::new(),
            seen: Vec::new(),
        };
        if !countries {
            self.cold = true;
            self.join = None;
            return out.push(facts);
        }
        facts.census = true;
        if regrouped || self.join.is_none() {
            let join = Join::fingerprint(roster);
            self.join = Some(join);
            facts.join = Some(join);
        }
        let forces = columns::complete(state, TableId::TaskForces);
        if !columns::complete(state, TableId::Ships) {
            self.cold = true;
            return out.push(facts);
        }
        // A complete empty ship table proves no ship without the task forces.
        if !forces && columns::table(state, TableId::Ships)?.live().count() > 0 {
            self.cold = true;
            return out.push(facts);
        }
        // Without the links no ship has an owner: the census reads them all, unattributed, and
        // reads them whole again once the links are back.
        let links = roster.links_complete();
        let models = columns::complete(state, TableId::ShipModels);
        let whole = std::mem::take(&mut self.cold)
            || !links
            || self.models != Some(models)
            || self.forces_complete != Some(forces);
        self.models = Some(models);
        self.forces_complete = Some(forces);
        {
            let _cpu = replay_frame::cpu::scope("navy.columnar.census", None);
            if whole {
                self.read_whole(state, roster, models, forces)?;
            } else {
                if regrouped {
                    self.reattribute(roster);
                }
                self.read_changes(state, roster, models, forces)?;
            }
        }
        facts.read = true;
        facts.whole = whole;
        facts.counts = self.counts(whole);
        facts.unattributed = self.unattributed.map(|count| count as u64);
        facts.complete = self.unattributed.map(|count| count == 0);
        (facts.spelled_whole, facts.spellings) = self.spellings(state, whole)?;
        facts.seen = self.seen(facts.spelled_whole, &facts.spellings);
        if !links {
            self.cold = true;
        }
        out.push(facts)
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
                "Not a saved navy projector",
            ));
        }
        *self = Self::with(self.format)?;
        Ok(())
    }
}
