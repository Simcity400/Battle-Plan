//! Research facts, projected hour by hour from a columnar walk ([`replay::segments`]): what the
//! country step ([`super::Scan`]) reads of each hour, as the hourly walk's index step found it
//! in the hour's frame (the holders, owners and read hours by slot where they changed, the slots
//! to observe again, and each fresh read's rows), and each tag's grant facts where they changed.
//! Every segment starts cold. Its lens ([`super::lens`]) folds every country from its last read
//! before the segment's first hour, and hands the ordered emit ([`super::emit`]) what each
//! country publishes from its first read a day into the segment; the emit folds the rest.
//!
//! A fresh read sends its slots' rows, what its bonus and modifier rows attribute to each slot,
//! and the inventory rows its assignments can ask for. Attributions are derived here from the
//! two modifier scopes and the bonus scope a slot reads, and derived again only where a row of
//! them, or the slot's technology, changed; an unchanged result is the same shared value.
//!
//! A fresh read joins its continuing assignments to the inventory by the template indexes its
//! country's state holds, which the projection does not know. A continuing assignment was read in
//! its slot within the two read periods before, so a read keeps the rows of every index its
//! slot's reads of those periods held; one within two read periods of the segment's first hour
//! keeps its owner's whole inventory, shared by its reads while no row of it changes.

use super::{
    Attribution, CountryRows, IndexChanges, Indexes, Technology, attributed_bonuses,
    grants::{self, GrantInventory},
    lens, modifiers, valid_inputs,
};
use crate::{ResearchBonus, ResearchModifiers};
use dashboard_world::{Needs, Roster, Tag, Tagged, World};
use engine_research::research_modifiers::grants::INVENTORY_INDEX;
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    cadence::STAGGER_PERIOD,
    format::FormatVersion,
    rows::{
        LexicalTokenRow, Recorded, ResearchAppliedBonusRow, ResearchInputRow, ResearchModifierRow,
        ResearchTechnologyRow, RowCodec, TableId,
    },
    scope::CaptureMode,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ResearchTechnologies,
    TableId::ResearchInputs,
    TableId::ResearchModifiers,
    TableId::ResearchAppliedBonuses,
    TableId::SliceReads,
    TableId::LexicalTokens,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// The hours before a read whose reads of its slot can hold an assignment it continues.
const REACH: i32 = 2 * STAGGER_PERIOD;

/// A slot's fresh read, as the country step reads it ([`super::FreshRead`]).
#[derive(replay_spool::Spooled)]
pub struct Fresh {
    /// Its slots' rows; none when the read is not this hour's, which the country step skips.
    pub(super) inputs: Vec<ResearchInputRow>,
    pub(super) read: Option<i32>,
    /// What each slot's bonus and modifier rows attribute to it, at its position; none when
    /// the read is not this hour's or its slots are refused.
    pub(super) derived: Vec<Derived>,
    pub(super) inventory: Option<Inventory>,
}

/// What a slot's bonus and modifier rows attribute to it: None where they were not read with
/// it or do not attribute.
#[derive(Clone, Default, replay_spool::Spooled)]
pub struct Derived {
    pub(super) bonuses: Option<Arc<Vec<ResearchBonus>>>,
    pub(super) modifiers: Option<Arc<ResearchModifiers>>,
}

impl Derived {
    pub(super) fn attribution(&self) -> Attribution<'_> {
        Attribution {
            bonuses: self.bonuses.as_deref().map(Vec::as_slice),
            modifiers: self.modifiers.as_deref(),
        }
    }
}

/// The inventory a fresh read joins its assignments to, by template index, ascending.
#[derive(replay_spool::Spooled)]
pub enum Inventory {
    /// Its owner's whole part.
    Owner(Arc<[(u32, Technology)]>),
    /// Its owner's rows of every template index a continuing assignment can ask for.
    Rows(Vec<(u32, Technology)>),
}

impl Inventory {
    pub(super) fn rows(&self) -> &[(u32, Technology)] {
        match self {
            Self::Owner(rows) => rows,
            Self::Rows(rows) => rows,
        }
    }
}

/// An hour's index step that changed something the country step reads.
#[derive(replay_spool::Spooled)]
pub struct Changed {
    /// The slots to observe, and whether the holders changed.
    pub(crate) dirty: BTreeSet<u32>,
    pub(crate) countries_changed: bool,
    pub(crate) changes: IndexChanges,
    /// Every slot one of whose owners was read at the hour; its shared attributions and
    /// inventory go to the spool once while the projection holds them.
    pub(crate) fresh: BTreeMap<u32, Fresh>,
}

/// An hour's index step.
#[derive(replay_spool::Spooled)]
pub enum Index {
    /// A table the indexes read was not complete.
    Incomplete,
    /// No slot to observe, index value or fresh read.
    Quiet,
    Changed(Box<Changed>),
}

/// What a grant inventory's rows hold.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub enum Content {
    /// Every name unlocalised.
    Inventory(GrantInventory),
    /// Why the rows hold no inventory.
    Refused(String),
    /// Why a table is not complete, which a reply dates at its hour.
    Table(String),
}

fn text_heap(text: &String) -> usize {
    text.capacity()
}

fn texts_heap(texts: &[&Option<String>]) -> usize {
    texts
        .iter()
        .map(|text| text.as_ref().map_or(0, text_heap))
        .sum()
}

impl Content {
    /// The bytes it holds past its own size.
    pub fn heap(&self) -> usize {
        match self {
            Self::Inventory(inventory) => {
                texts_heap(&[&inventory.unavailable])
                    + inventory.grants.capacity() * size_of::<grants::ResearchGrant>()
                    + inventory
                        .grants
                        .iter()
                        .map(|grant| {
                            text_heap(&grant.kind)
                                + text_heap(&grant.token)
                                + texts_heap(&[
                                    &grant.name,
                                    &grant.speed_bonus_raw,
                                    &grant.ahead_reduction_raw,
                                    &grant.cost_reduction_raw,
                                ])
                                + grant
                                    .technologies
                                    .iter()
                                    .chain(&grant.categories)
                                    .map(|restriction| {
                                        size_of::<grants::GrantRestriction>()
                                            + text_heap(&restriction.token)
                                            + texts_heap(&[&restriction.name])
                                    })
                                    .sum::<usize>()
                        })
                        .sum::<usize>()
            }
            Self::Refused(reason) | Self::Table(reason) => text_heap(reason),
        }
    }
}

/// What a tag's grant inventory reads at an hour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct Grant {
    /// Its holder's id and canonical slot, while the countries name one.
    pub identity: Option<(u32, u32)>,
    /// Its holder's ResearchModifiers read hour.
    pub read: Option<i32>,
    /// Whether every owner of its slot holding grant rows was read at `read`, the tables
    /// complete.
    pub matched: bool,
    /// What the rows hold, while the countries name a holder and ResearchModifiers is complete.
    pub content: Option<Arc<Content>>,
}

impl Grant {
    pub(super) const NONE: Self = Self {
        identity: None,
        read: None,
        matched: false,
        content: None,
    };

    /// Whether it reads as `other` does, shared contents compared by their place first.
    pub(super) fn same(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.read == other.read
            && self.matched == other.matched
            && same_content(self.content.as_ref(), other.content.as_ref())
    }
}

/// Whether two contents are equal, shared ones compared by their place first.
pub(super) fn same_content(a: Option<&Arc<Content>>, b: Option<&Arc<Content>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
        (None, None) => true,
        _ => false,
    }
}

/// The grant facts an hour changed.
#[derive(replay_spool::Spooled)]
pub struct Grants {
    /// Whether ResearchModifiers held every row, where that changed.
    pub complete: Option<bool>,
    /// Whether `facts` names every tag the roster holds: a tag it leaves out holds no grants.
    pub whole: bool,
    /// Each tag whose facts changed, in tag order.
    pub facts: Vec<(String, Grant)>,
}

/// One hour of a segment, or what the segment's lens hands back after the hour it folded last.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub index: Index,
    pub grants: Option<Box<Grants>>,
    /// What the lens hands the emit with the hour.
    pub(super) lens: Option<Box<lens::Part>>,
    /// Whether it only hands the lens's countries back, after hour `hour`.
    pub(super) handover: bool,
}

impl Hour {
    /// What the lens hands back after `hour`.
    fn handed(branch: u64, hour: i32, part: Box<lens::Part>) -> Self {
        Self {
            branch,
            hour,
            index: Index::Quiet,
            grants: None,
            lens: Some(part),
            handover: true,
        }
    }
}

fn bonuses_heap(bonuses: &[ResearchBonus]) -> usize {
    size_of::<Vec<ResearchBonus>>()
        + bonuses.len() * size_of::<ResearchBonus>()
        + bonuses
            .iter()
            .map(|bonus| {
                texts_heap(&[
                    &bonus.token,
                    &bonus.name,
                    &bonus.speed_bonus_raw,
                    &bonus.cost_reduction_raw,
                    &bonus.ahead_reduction_years_raw,
                ])
            })
            .sum::<usize>()
}

/// A row group of a table: the live rows it holds, one past the highest member any of them
/// held since the table was read whole, and the hour a row of it was last inserted, deleted
/// or changed in a followed field.
#[derive(Clone, Copy)]
struct Group {
    rows: u32,
    bound: u32,
    changed: i32,
}

/// A table whose keys hold a group in their bits from 32 up and a member below, by group,
/// followed from the hour's changes; empty while the table is not complete.
struct Keyed {
    table: TableId,
    /// A group's owner is its bits from here up.
    owner_shift: u32,
    /// The change-mask bits of the fields a group's hour follows.
    fields: u64,
    complete: bool,
    groups: HashMap<u64, Group>,
    /// Each owner's live rows.
    owners: HashMap<u32, u32>,
    /// The owners whose rows the hour inserted, deleted or changed, or every owner held before
    /// or after where they were read whole; ascending.
    touched: Vec<u32>,
    /// Each group's members and slots in key order, once a group too sparse to look up by
    /// member was read; followed from then on.
    index: Option<HashMap<u64, Vec<(u32, u32)>>>,
}

impl Keyed {
    fn new(table: TableId, owner_shift: u32, fields: u64) -> Self {
        Self {
            table,
            owner_shift,
            fields,
            complete: false,
            groups: HashMap::new(),
            owners: HashMap::new(),
            touched: Vec::new(),
            index: None,
        }
    }

    fn owner_of(&self, group: u64) -> u32 {
        (group >> self.owner_shift) as u32
    }

    fn add(&mut self, key: u128, slot: u32, at: i32) -> u32 {
        let group = (key >> 32) as u64;
        if let Some(index) = &mut self.index {
            let members = index.entry(group).or_default();
            let place = members.partition_point(|&(member, _)| member < key as u32);
            members.insert(place, (key as u32, slot));
        }
        let held = self.groups.entry(group).or_insert(Group {
            rows: 0,
            bound: 0,
            changed: at,
        });
        held.rows += 1;
        held.bound = held.bound.max((key as u32).saturating_add(1));
        held.changed = at;
        let owner = self.owner_of(group);
        *self.owners.entry(owner).or_default() += 1;
        owner
    }

    fn remove(&mut self, key: u128, at: i32) -> u32 {
        let group = (key >> 32) as u64;
        if let Some(index) = &mut self.index
            && let Some(members) = index.get_mut(&group)
        {
            members.retain(|&(member, _)| member != key as u32);
            if members.is_empty() {
                index.remove(&group);
            }
        }
        if let Some(held) = self.groups.get_mut(&group) {
            held.rows -= 1;
            held.changed = at;
            if held.rows == 0 {
                self.groups.remove(&group);
            }
        }
        let owner = self.owner_of(group);
        if let Some(rows) = self.owners.get_mut(&owner) {
            *rows -= 1;
            if *rows == 0 {
                self.owners.remove(&owner);
            }
        }
        owner
    }

    fn update(&mut self, state: &State, whole: bool, at: i32) -> io::Result<()> {
        self.touched.clear();
        let complete = columns::complete(state, self.table);
        let whole = whole || !self.complete;
        self.complete = complete;
        if whole || !complete {
            self.touched.extend(self.owners.drain().map(|(owner, _)| owner));
            self.groups.clear();
            self.index = None;
        }
        if complete {
            let held = columns::table(state, self.table)?;
            if whole {
                for slot in held.live().ones() {
                    if let Some(key) = held.key(slot) {
                        self.add(key, slot, at);
                    }
                }
                self.touched.extend(self.owners.keys().copied());
            } else {
                let edits = columns::changes(state, self.table)?;
                for &key in &edits.deleted_keys {
                    let owner = self.remove(key, at);
                    self.touched.push(owner);
                }
                for &slot in &edits.inserted {
                    if let Some(key) = held.key(slot) {
                        let owner = self.add(key, slot, at);
                        self.touched.push(owner);
                    }
                }
                for (&slot, &written) in edits
                    .changed
                    .iter()
                    .zip(&edits.written)
                    .chain(edits.fill_slots.iter().zip(&edits.fill_masks))
                {
                    if let Some(key) = held.key(slot) {
                        let group = (key >> 32) as u64;
                        let owner = self.owner_of(group);
                        self.touched.push(owner);
                        if written & self.fields != 0
                            && let Some(kept) = self.groups.get_mut(&group)
                        {
                            kept.changed = at;
                        }
                    }
                }
            }
        }
        self.touched.sort_unstable();
        self.touched.dedup();
        Ok(())
    }

    /// Whether `owner` holds a row.
    fn holds(&self, owner: u32) -> bool {
        self.owners.contains_key(&owner)
    }

    /// The hour `group` last changed; None while it holds no row.
    fn changed(&self, group: u64) -> Option<i32> {
        self.groups.get(&group).map(|held| held.changed)
    }

    /// The slots of `group`'s rows in key order, into `out`, and whether its members are its
    /// first `rows` numbers.
    fn members(&mut self, state: &State, group: u64, out: &mut Vec<u32>) -> io::Result<bool> {
        out.clear();
        let Some(&Group { rows, bound, .. }) = self.groups.get(&group) else {
            return Ok(true);
        };
        let held = columns::table(state, self.table)?;
        let base = u128::from(group) << 32;
        for member in 0..rows {
            match held.slot(base | u128::from(member)) {
                Some(slot) => out.push(slot),
                None => break,
            }
        }
        if out.len() == rows as usize {
            return Ok(true);
        }
        out.clear();
        if u64::from(bound) <= 4 * u64::from(rows) + 1024 {
            for member in 0..bound {
                if let Some(slot) = held.slot(base | u128::from(member)) {
                    out.push(slot);
                    if out.len() == rows as usize {
                        break;
                    }
                }
            }
        } else {
            // Members too sparse to look up one by one: every group's, read once and followed.
            let index = self.index.get_or_insert_with(|| {
                let mut index: HashMap<u64, Vec<(u32, u32)>> = HashMap::new();
                for slot in held.live().ones() {
                    if let Some(key) = held.key(slot) {
                        index
                            .entry((key >> 32) as u64)
                            .or_default()
                            .push((key as u32, slot));
                    }
                }
                for members in index.values_mut() {
                    members.sort_unstable();
                }
                index
            });
            out.extend(index.get(&group).into_iter().flatten().map(|&(_, slot)| slot));
        }
        Ok(false)
    }
}

/// The SliceReads tables whose owners' read hours a projection follows.
const STAMPED: [TableId; 3] = [
    TableId::ResearchInputs,
    TableId::ResearchAppliedBonuses,
    TableId::ResearchModifiers,
];
const INPUTS: usize = 0;
const BONUSES: usize = 1;
const MODIFIERS: usize = 2;

fn stamped(table: u32) -> Option<usize> {
    STAMPED
        .iter()
        .position(|kept| kept.index() as u32 == table)
}

/// SliceReads' read hours of [`STAMPED`], by owner; none while it is not complete.
#[derive(Default)]
struct Stamps {
    complete: bool,
    reads: [HashMap<u32, i32>; 3],
    /// The owners whose read row of each table the hour wrote or deleted, or every owner held
    /// before or after where they were read whole; ascending.
    touched: [Vec<u32>; 3],
    /// Whether the hour changed any SliceReads row, or read them whole.
    changed: bool,
}

impl Stamps {
    fn update(&mut self, state: &State, whole: bool, field: usize) -> io::Result<()> {
        let table = TableId::SliceReads;
        for touched in &mut self.touched {
            touched.clear();
        }
        let complete = columns::complete(state, table);
        let whole = whole || !self.complete || !complete;
        self.complete = complete;
        if whole {
            for (reads, touched) in self.reads.iter_mut().zip(&mut self.touched) {
                touched.extend(reads.drain().map(|(owner, _)| owner));
            }
        }
        self.changed = whole;
        if complete {
            let held = columns::table(state, table)?;
            let cells = columns::plain::<i32>(held, field)?;
            let decode = |slot: u32| -> io::Result<Option<(usize, u32, i32)>> {
                let Some((at, owner)) = held
                    .key(slot)
                    .and_then(|key| Some((stamped((key >> 32) as u32)?, key as u32)))
                else {
                    return Ok(None);
                };
                match cells.get(slot as usize) {
                    Some(&read) if held.recorded(field, slot) => Ok(Some((at, owner, read))),
                    _ => Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "A SliceReads read hour is not recorded",
                    )),
                }
            };
            if whole {
                for slot in held.live().ones() {
                    if let Some((at, owner, read)) = decode(slot)? {
                        self.reads[at].insert(owner, read);
                        self.touched[at].push(owner);
                    }
                }
            } else {
                let edits = columns::changes(state, table)?;
                self.changed = !edits.changed.is_empty()
                    || !edits.fill_slots.is_empty()
                    || !edits.inserted.is_empty()
                    || !edits.deleted.is_empty();
                for &key in &edits.deleted_keys {
                    if let Some(at) = stamped((key >> 32) as u32) {
                        self.reads[at].remove(&(key as u32));
                        self.touched[at].push(key as u32);
                    }
                }
                for &slot in edits
                    .changed
                    .iter()
                    .chain(&edits.fill_slots)
                    .chain(&edits.inserted)
                {
                    if let Some((at, owner, read)) = decode(slot)? {
                        self.reads[at].insert(owner, read);
                        self.touched[at].push(owner);
                    }
                }
            }
        }
        for touched in &mut self.touched {
            touched.sort_unstable();
            touched.dedup();
        }
        Ok(())
    }

    fn read(&self, at: usize, owner: u32) -> Option<i32> {
        self.reads[at].get(&owner).copied()
    }

    fn heap(&self) -> usize {
        self.reads
            .iter()
            .map(|reads| reads.capacity() * size_of::<(u32, i32)>())
            .sum::<usize>()
            + self.touched.iter().map(|touched| touched.capacity() * 4).sum::<usize>()
    }
}

/// Each link's canonical slot by raw id, and each canonical slot's holders the links resolve
/// (their place among the countries by id, id and tag), as the hourly walk indexes them.
fn holders(roster: &Roster) -> (HashMap<u32, u32>, CountryRows) {
    let slots: HashMap<u32, u32> = roster
        .links()
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let mut rows = CountryRows::new();
    for (index, country) in roster.iter().enumerate() {
        if slots.get(&country.id) == Some(&country.slot) {
            rows.entry(country.slot)
                .or_default()
                .push((index, country.id, roster.tag(country.tag).to_vec()));
        }
    }
    (slots, rows)
}

/// Each canonical slot's raw link ids, ascending.
fn owners(roster: &Roster) -> BTreeMap<u32, Vec<u32>> {
    let mut owners: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for link in roster.links() {
        owners.entry(link.canonical_slot).or_default().push(link.id);
    }
    owners
}

/// The template indexes `inputs` hold with their recorded occurrence.
fn occurrences(inputs: &[ResearchInputRow]) -> impl Iterator<Item = u32> + '_ {
    inputs
        .iter()
        .filter(|input| input.technology_present != 0)
        .filter_map(|input| match input.database_index {
            Recorded::Value(index) => Some(index as u32),
            _ => None,
        })
}

/// Why `table` is not complete, as a row frame's read of it says past its hour.
fn incomplete(state: &State, table: TableId, mode: CaptureMode) -> io::Result<String> {
    Ok(match columns::status(state, table) {
        columns::Status::Unavailable { reason } => {
            let reason = state.dictionary().get(reason).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "A table's reason names no dictionary entry",
                )
            })?;
            format!(
                "{table:?} is unavailable: {}",
                String::from_utf8_lossy(reason)
            )
        }
        columns::Status::Truncated { missing } => {
            format!("{table:?} is truncated ({missing} missing rows)")
        }
        columns::Status::NotRecorded => format!("{table:?} is not recorded in a {mode} recording"),
        columns::Status::Complete => {
            return Err(io::Error::other("A complete table has no reason"));
        }
    })
}

/// Whether the hour inserted, deleted or changed a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

/// A ResearchModifiers or ResearchAppliedBonuses group: an owner's rows of one slot index.
fn scope(owner: u32, index: u32) -> u64 {
    u64::from(owner) << 32 | u64::from(index)
}

/// A slot's modifier breakdown as last derived: from the hours its slot and country scopes
/// last changed, its technology handle and its applied speed bonus.
struct Breakdown {
    from: (Option<i32>, Option<i32>, u32, i64),
    value: Option<Arc<ResearchModifiers>>,
}

/// A slot's bonus attribution as last derived: from the hour its scope last changed, its
/// technology handle and its applied bonus count.
struct Attributed {
    from: (Option<i32>, u32, u32),
    value: Option<Arc<Vec<ResearchBonus>>>,
}

/// A tag's grant inventory as last assembled: from its holder's slot's owners, each with the
/// hour its inventory rows last changed, and whether its rows spell a token.
struct Assembled {
    from: Vec<(u32, Option<i32>)>,
    content: Arc<Content>,
    spells: bool,
}

/// `value`, the one `old` holds where they are equal.
fn share<T: PartialEq>(old: Option<&Arc<T>>, value: T) -> Arc<T> {
    match old {
        Some(old) if **old == value => Arc::clone(old),
        _ => Arc::new(value),
    }
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    settings: FormatVersion,
    format: u16,
    mode: CaptureMode,
    sample_hour: usize,
    literal: usize,
    /// ResearchTechnologies' classifier, level, researched and ahead reduction columns.
    technology: [usize; 4],
    last: Option<i32>,
    /// The first hour since it was last cold.
    first: i32,
    /// Whether the holders were indexed since.
    held: bool,
    /// The index step's own state, as the hourly walk's scan keeps it ([`super::Scan`]).
    ready: bool,
    indexed: Option<i32>,
    future_read: Option<i32>,
    slots: Arc<HashMap<u32, u32>>,
    country_rows: CountryRows,
    owners: BTreeMap<u32, Vec<u32>>,
    input_slots: BTreeSet<u32>,
    input_dates: BTreeMap<u32, Vec<(u32, i32)>>,
    /// The slots of `input_dates` by each read hour they hold.
    by_read: BTreeMap<i32, BTreeSet<u32>>,
    inputs: Keyed,
    technologies: Keyed,
    bonuses: Keyed,
    modifiers: Keyed,
    stamps: Stamps,
    /// Each slot's fresh reads of the last [`REACH`] hours, with the template indexes they held.
    seen: HashMap<u32, VecDeque<(i32, Vec<u32>)>>,
    /// Each owner's whole inventory as last read, with the hour its rows had last changed.
    inventories: HashMap<u32, (Option<i32>, Arc<[(u32, Technology)]>)>,
    /// Each slot's attributions as last derived, by owner and slot index.
    breakdowns: HashMap<(u32, u32), Breakdown>,
    attributions: HashMap<(u32, u32), Attributed>,
    /// The grant facts sent since it was cold, by tag; each tag's inventory as last assembled;
    /// the tags at each slot they name; and the tags whose inventory spells a token.
    granted: HashMap<String, Grant>,
    assembled: HashMap<String, Assembled>,
    slot_tags: HashMap<u32, Vec<String>>,
    spelled: BTreeSet<String>,
    /// Whether ResearchModifiers and SliceReads were complete, and LexicalTokens' status.
    shapes: Option<([bool; 2], columns::Status)>,
    /// Slots and rows read again for each derivation, kept for their buffers.
    found: Vec<u32>,
    shared_rows: Vec<ResearchModifierRow>,
    slot_rows: Vec<ResearchModifierRow>,
    /// The segment's fold of every country, and the branch it walks.
    lens: Option<lens::Lens>,
    branch: u64,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"research projector: cold";

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let technologies = TableId::ResearchTechnologies;
        let technology = [
            columns::field(technologies, "classifier")?,
            columns::field(technologies, "level")?,
            columns::field(technologies, "researched")?,
            columns::field(technologies, "ahead_reduction_years_raw")?,
        ];
        let followed = technology
            .iter()
            .fold(0_u64, |mask, &field| mask | 1_u64.checked_shl(field as u32).unwrap_or(0));
        Ok(Self {
            settings: format,
            format: format.version(),
            mode,
            sample_hour: columns::field(TableId::SliceReads, "sample_hour")?,
            literal: columns::field(TableId::LexicalTokens, "literal")?,
            technology,
            last: None,
            first: 0,
            held: false,
            ready: false,
            indexed: None,
            future_read: None,
            slots: Arc::default(),
            country_rows: CountryRows::new(),
            owners: BTreeMap::new(),
            input_slots: BTreeSet::new(),
            input_dates: BTreeMap::new(),
            by_read: BTreeMap::new(),
            inputs: Keyed::new(TableId::ResearchInputs, 0, u64::MAX),
            technologies: Keyed::new(technologies, 0, followed),
            bonuses: Keyed::new(TableId::ResearchAppliedBonuses, 32, u64::MAX),
            modifiers: Keyed::new(TableId::ResearchModifiers, 32, u64::MAX),
            stamps: Stamps::default(),
            seen: HashMap::new(),
            inventories: HashMap::new(),
            breakdowns: HashMap::new(),
            attributions: HashMap::new(),
            granted: HashMap::new(),
            assembled: HashMap::new(),
            slot_tags: HashMap::new(),
            spelled: BTreeSet::new(),
            shapes: None,
            found: Vec::new(),
            shared_rows: Vec::new(),
            slot_rows: Vec::new(),
            lens: None,
            branch: 0,
        })
    }

    /// Forgets everything but its settings: the hour `first` is read whole.
    fn cold(&mut self, first: i32) -> io::Result<()> {
        *self = Self {
            first,
            ..Self::new(self.settings, self.mode)?
        };
        Ok(())
    }

    /// Sets `slot`'s read hours, keeping `by_read`.
    fn set_dates(&mut self, slot: u32, dates: Option<Vec<(u32, i32)>>) {
        if let Some(old) = self.input_dates.remove(&slot) {
            for (_, read) in old {
                if let Some(slots) = self.by_read.get_mut(&read) {
                    slots.remove(&slot);
                    if slots.is_empty() {
                        self.by_read.remove(&read);
                    }
                }
            }
        }
        if let Some(dates) = dates {
            for &(_, read) in &dates {
                self.by_read.entry(read).or_default().insert(slot);
            }
            self.input_dates.insert(slot, dates);
        }
    }

    /// The hour's index step, as the hourly walk's ([`super::Scan`]'s `refresh`) takes it from
    /// the hour's frame: the holders, links, research rows and read hours by slot, refreshed by
    /// the hour's changes or rebuilt.
    fn index(
        &mut self,
        state: &State,
        roster: &Roster,
        at: i32,
    ) -> io::Result<(Index, BTreeMap<u32, Fresh>)> {
        let complete = roster.complete()
            && columns::complete(state, TableId::ResearchInputs)
            && columns::complete(state, TableId::SliceReads);
        if !complete {
            // Indexes this hour's rows did not refresh are rebuilt at the next.
            self.ready = false;
            return Ok((Index::Incomplete, BTreeMap::new()));
        }
        let forced = !self.ready;
        let links_changed = forced || roster.rebuilt() || !roster.changed_links().is_empty();
        // A pending read dated after its hour becomes fresh with no row changing, so until it
        // passes the whole tables are observed.
        let narrow = !links_changed
            && self.indexed.and_then(|hour| hour.checked_add(1)) == Some(at)
            && self.future_read.is_none_or(|read| read < at);
        let mut changes = IndexChanges::default();
        let countries_changed = if forced || links_changed || roster.changed() {
            let (slots, rows) = holders(roster);
            let same = self.held && slots == *self.slots && rows == self.country_rows;
            if !same {
                self.slots = Arc::new(slots);
                self.country_rows = rows;
                self.held = true;
                changes.countries = Some((Arc::clone(&self.slots), self.country_rows.clone()));
            }
            !same
        } else {
            false
        };
        if links_changed {
            self.owners = owners(roster);
            changes.owners = Some(self.owners.clone());
        }
        let mut dirty = BTreeSet::new();
        if forced || links_changed || !self.inputs.touched.is_empty() {
            if narrow {
                let touched: BTreeSet<u32> = self
                    .inputs
                    .touched
                    .iter()
                    .filter_map(|owner| self.slots.get(owner).copied())
                    .collect();
                for slot in touched {
                    let held = self
                        .owners
                        .get(&slot)
                        .is_some_and(|owners| owners.iter().any(|&owner| self.inputs.holds(owner)));
                    if held {
                        self.input_slots.insert(slot);
                    } else {
                        self.input_slots.remove(&slot);
                    }
                    dirty.insert(slot);
                }
            } else {
                dirty.extend(self.input_slots.iter().copied());
                self.input_slots.clear();
                for owner in self.inputs.owners.keys() {
                    if let Some(&slot) = self.slots.get(owner) {
                        self.input_slots.insert(slot);
                    }
                }
                dirty.extend(self.input_slots.iter().copied());
            }
        }
        if forced || links_changed || self.stamps.changed {
            if narrow {
                let touched: BTreeSet<u32> = self.stamps.touched[INPUTS]
                    .iter()
                    .filter_map(|owner| self.slots.get(owner).copied())
                    .collect();
                for slot in touched {
                    // The slot's dates as the whole table gives them: by owner.
                    let dates: Vec<(u32, i32)> = self
                        .owners
                        .get(&slot)
                        .into_iter()
                        .flatten()
                        .filter_map(|&owner| {
                            self.stamps.read(INPUTS, owner).map(|read| (owner, read))
                        })
                        .collect();
                    if let Some(&(_, read)) = dates.iter().max_by_key(|(_, read)| *read)
                        && read > at
                    {
                        self.future_read = self.future_read.max(Some(read));
                    }
                    let dates = (!dates.is_empty()).then_some(dates);
                    if self.input_dates.get(&slot) != dates.as_ref() {
                        dirty.insert(slot);
                        changes.slot_dates.push((slot, dates.clone()));
                        self.set_dates(slot, dates);
                    }
                }
            } else {
                let mut stamps: Vec<(u32, i32)> = self.stamps.reads[INPUTS]
                    .iter()
                    .map(|(&owner, &read)| (owner, read))
                    .collect();
                stamps.sort_unstable();
                let mut dates = BTreeMap::<u32, Vec<(u32, i32)>>::new();
                for (owner, read) in stamps {
                    if let Some(&slot) = self.slots.get(&owner) {
                        dates.entry(slot).or_default().push((owner, read));
                        if read > at {
                            self.future_read = self.future_read.max(Some(read));
                        }
                    }
                }
                for (&slot, values) in &dates {
                    if self.input_dates.get(&slot) != Some(values) {
                        dirty.insert(slot);
                    }
                }
                dirty.extend(
                    self.input_dates
                        .keys()
                        .filter(|slot| !dates.contains_key(*slot))
                        .copied(),
                );
                self.input_dates.clear();
                self.by_read.clear();
                for (slot, values) in dates {
                    self.set_dates(slot, Some(values));
                }
                changes.dates = Some(self.input_dates.clone());
            }
        }
        if countries_changed {
            dirty.extend(self.country_rows.keys().copied());
        }
        self.ready = true;
        self.indexed = Some(at);
        let mut fresh = BTreeMap::new();
        let read: Vec<u32> = self
            .by_read
            .get(&at)
            .map(|slots| slots.iter().copied().collect())
            .unwrap_or_default();
        for slot in read {
            let kept = self.fresh(state, at, slot)?;
            fresh.insert(slot, kept);
        }
        let quiet = dirty.is_empty()
            && !countries_changed
            && changes.countries.is_none()
            && changes.owners.is_none()
            && changes.dates.is_none()
            && changes.slot_dates.is_empty()
            && fresh.is_empty();
        Ok(if quiet {
            (Index::Quiet, fresh)
        } else {
            let changed = Changed {
                dirty,
                countries_changed,
                changes,
                fresh: BTreeMap::new(),
            };
            (Index::Changed(Box::new(changed)), fresh)
        })
    }

    /// Each country's read before hour `at`, the lens's first, that its rows still hold: as the
    /// country step took it fresh at its own hour ([`Self::fresh`]), with the owner's whole
    /// inventory. A slot more tags hold is folded by the emit alone, and so is one whose rows
    /// the walk cannot read, which the hourly walk never read here: no read of them refuses.
    fn virgins(&mut self, state: &State, at: i32) -> io::Result<Vec<lens::Virgin>> {
        let mut virgins = Vec::new();
        let slots: Vec<(u32, u32, Vec<u8>)> = self
            .country_rows
            .iter()
            .filter_map(|(&slot, rows)| match rows.as_slice() {
                [(_, id, tag)] => Some((slot, *id, tag.clone())),
                _ => None,
            })
            .collect();
        for (slot, id, tag) in slots {
            let Ok(tag) = String::from_utf8(tag) else {
                continue;
            };
            if tag.is_empty() || tag.len() > 512 {
                continue;
            }
            match self.virgin(state, at, slot) {
                Ok(Some((read, fresh))) => virgins.push(lens::Virgin {
                    tag,
                    identity: (id, slot),
                    read,
                    fresh,
                }),
                Ok(None) => {}
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {}
                Err(error) => return Err(error),
            }
        }
        Ok(virgins)
    }

    /// `slot`'s read before hour `at` that its rows still hold, as [`Self::virgins`] takes it.
    fn virgin(&mut self, state: &State, at: i32, slot: u32) -> io::Result<Option<(i32, Fresh)>> {
        let owners = self.owners.get(&slot).cloned().unwrap_or_default();
        let inputs = self.input_rows(state, &owners)?;
        let stamp = |owner: u32| self.stamps.read(INPUTS, owner);
        let read = match inputs.first() {
            Some(input) => stamp(input.country)
                .filter(|&read| inputs.iter().all(|input| stamp(input.country) == Some(read))),
            None => owners.iter().filter_map(|&owner| stamp(owner)).max(),
        };
        let Some(read) = read.filter(|&read| read < at) else {
            return Ok(None);
        };
        let owner = inputs
            .first()
            .map(|input| input.country)
            .or_else(|| owners.iter().copied().find(|&owner| stamp(owner) == Some(read)));
        let mut derived = Vec::new();
        let refs: Vec<&ResearchInputRow> = inputs.iter().collect();
        if valid_inputs(&refs) {
            let bonuses = self.joined(state, &self.bonuses, BONUSES, &owners, read);
            let modifiers = self.joined(state, &self.modifiers, MODIFIERS, &owners, read);
            derived.reserve_exact(inputs.len());
            for input in &inputs {
                derived.push(self.derive(state, input, bonuses, modifiers)?);
            }
        }
        let inventory = match owner
            .filter(|_| columns::complete(state, TableId::ResearchTechnologies))
        {
            Some(owner) => Some(self.inventory(state, at, slot, owner, &inputs)?),
            None => None,
        };
        Ok(Some((
            read,
            Fresh {
                inputs,
                read: Some(read),
                derived,
                inventory,
            },
        )))
    }

    /// Hands the lens's countries back after the hour it folded last, `last`.
    fn hand_over(&mut self, last: i32, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        if let Some(mut lens) = self.lens.take()
            && let Some(part) = lens.hand_over()?
        {
            out.push(Hour::handed(self.branch, last, part))?;
        }
        Ok(())
    }

    /// Whether `keyed`'s table is complete and every one of `owners` holding rows of it was read
    /// at `read` too (stamps `at`): its rows then come from the same read.
    fn joined(&self, state: &State, keyed: &Keyed, at: usize, owners: &[u32], read: i32) -> bool {
        columns::complete(state, keyed.table)
            && owners
                .iter()
                .filter(|&&owner| keyed.holds(owner))
                .all(|&owner| self.stamps.read(at, owner) == Some(read))
    }

    /// `owners`' ResearchInputs rows, each owner's in key order.
    fn input_rows(&mut self, state: &State, owners: &[u32]) -> io::Result<Vec<ResearchInputRow>> {
        let table = TableId::ResearchInputs;
        let mut rows = Vec::new();
        for &owner in owners {
            self.inputs.members(state, u64::from(owner), &mut self.found)?;
            for &slot in &self.found {
                rows.push(columns::row::<ResearchInputRow>(state, table, slot, self.format)?);
            }
        }
        Ok(rows)
    }

    /// The rows of `group` of `keyed`'s table, in key order, into `out`, and whether its members
    /// are its first numbers.
    fn group_rows<R: RowCodec>(
        state: &State,
        keyed: &mut Keyed,
        group: u64,
        format: u16,
        found: &mut Vec<u32>,
        out: &mut Vec<R>,
    ) -> io::Result<bool> {
        out.clear();
        let dense = keyed.members(state, group, found)?;
        for &slot in found.iter() {
            out.push(columns::row::<R>(state, keyed.table, slot, format)?);
        }
        Ok(dense)
    }

    /// What `input`'s slot's bonus and modifier rows attribute to it, those that were read with
    /// it, derived again only where a row it reads or its technology changed.
    fn derive(
        &mut self,
        state: &State,
        input: &ResearchInputRow,
        bonuses_joined: bool,
        modifiers_joined: bool,
    ) -> io::Result<Derived> {
        let key = (input.country, input.index);
        let mut derived = Derived::default();
        if bonuses_joined {
            let group = scope(input.country, input.index);
            let from = (
                self.bonuses.changed(group),
                input.technology_handle,
                input.bonus_count,
            );
            let held = self.attributions.get(&key).filter(|held| held.from == from);
            derived.bonuses = match held {
                Some(held) => held.value.clone(),
                None => {
                    let mut rows = Vec::new();
                    Self::group_rows::<ResearchAppliedBonusRow>(
                        state,
                        &mut self.bonuses,
                        group,
                        self.format,
                        &mut self.found,
                        &mut rows,
                    )?;
                    let old = self.attributions.get(&key).and_then(|held| held.value.as_ref());
                    let value = attributed_bonuses(input, &rows).map(|value| share(old, value));
                    self.attributions.insert(
                        key,
                        Attributed {
                            from,
                            value: value.clone(),
                        },
                    );
                    value
                }
            };
        }
        if modifiers_joined && input.technology_present != 0 {
            let (shared, slot) = (
                scope(input.country, u32::MAX),
                scope(input.country, input.index),
            );
            let from = (
                self.modifiers.changed(slot),
                self.modifiers.changed(shared),
                input.technology_handle,
                input.speed_bonus_raw,
            );
            let held = self.breakdowns.get(&key).filter(|held| held.from == from);
            derived.modifiers = match held {
                Some(held) => held.value.clone(),
                None => {
                    // Rows not numbered from zero in key order break the breakdown.
                    let dense = Self::group_rows(
                        state,
                        &mut self.modifiers,
                        shared,
                        self.format,
                        &mut self.found,
                        &mut self.shared_rows,
                    )? && Self::group_rows(
                        state,
                        &mut self.modifiers,
                        slot,
                        self.format,
                        &mut self.found,
                        &mut self.slot_rows,
                    )?;
                    let breakdown = dense
                        .then(|| modifiers::scoped(input, &self.shared_rows, &self.slot_rows))
                        .flatten();
                    let old = self.breakdowns.get(&key).and_then(|held| held.value.as_ref());
                    let value = breakdown.map(|value| share(old, value));
                    self.breakdowns.insert(
                        key,
                        Breakdown {
                            from,
                            value: value.clone(),
                        },
                    );
                    value
                }
            };
        }
        Ok(derived)
    }

    /// A reader of the inventory fields of ResearchTechnologies' live slots.
    fn technologies<'s>(
        &self,
        state: &'s State,
    ) -> io::Result<impl Fn(u32) -> io::Result<Technology> + use<'s>> {
        let table = TableId::ResearchTechnologies;
        let held = columns::table(state, table)?;
        let fields = self.technology;
        let [classifier, level, researched, ahead] = fields;
        let classifier = columns::plain::<u32>(held, classifier)?;
        let level = columns::plain::<i32>(held, level)?;
        let researched = columns::plain::<u8>(held, researched)?;
        let ahead = columns::plain::<i64>(held, ahead)?;
        let format = self.format;
        Ok(move |slot: u32| {
            let at = slot as usize;
            if fields.iter().all(|&field| held.recorded(field, slot)) {
                let missing = || io::Error::other("A technology's slot holds no cell");
                return Ok(Technology {
                    classifier: *classifier.get(at).ok_or_else(missing)?,
                    level: *level.get(at).ok_or_else(missing)?,
                    researched: *researched.get(at).ok_or_else(missing)? != 0,
                    ahead_reduction_years_raw: *ahead.get(at).ok_or_else(missing)?,
                });
            }
            // An unrecorded cell reads as the row decoder reads it.
            let row: ResearchTechnologyRow = columns::row(state, table, slot, format)?;
            Ok(Technology::from(&row))
        })
    }

    /// The fresh read of `slot` at `at`, as the hourly walk's ([`super::FrameReads`]) reads it.
    fn fresh(&mut self, state: &State, at: i32, slot: u32) -> io::Result<Fresh> {
        let owners = self.owners.get(&slot).cloned().unwrap_or_default();
        let inputs = self.input_rows(state, &owners)?;
        // SliceReads proves a read of no slots as well as one of many: the read is this
        // hour's when an owner was read now and every owner with rows was too. Its inventory is
        // that of its first owner holding slots, else of its first owner read now.
        let fresh = |owner: u32| self.stamps.read(INPUTS, owner) == Some(at);
        let owner = inputs
            .first()
            .map(|input| input.country)
            .or_else(|| owners.iter().copied().find(|&owner| fresh(owner)));
        let read = owner
            .filter(|&owner| fresh(owner) && inputs.iter().all(|input| fresh(input.country)))
            .map(|_| at);
        let mut derived = Vec::new();
        let mut inventory = None;
        if let Some(read) = read {
            let refs: Vec<&ResearchInputRow> = inputs.iter().collect();
            if valid_inputs(&refs) {
                // Bonuses and modifiers join only when they come from the same read.
                let bonuses = self.joined(state, &self.bonuses, BONUSES, &owners, read);
                let modifiers = self.joined(state, &self.modifiers, MODIFIERS, &owners, read);
                derived.reserve_exact(inputs.len());
                for input in &inputs {
                    derived.push(self.derive(state, input, bonuses, modifiers)?);
                }
            }
            if let Some(owner) =
                owner.filter(|_| columns::complete(state, TableId::ResearchTechnologies))
            {
                inventory = Some(self.inventory(state, at, slot, owner, &inputs)?);
            }
        }
        let seen = self.seen.entry(slot).or_default();
        seen.push_back((at, occurrences(&inputs).collect()));
        while seen
            .front()
            .is_some_and(|(hour, _)| *hour < at.saturating_sub(REACH))
        {
            seen.pop_front();
        }
        let inputs = if read.is_some() { inputs } else { Vec::new() };
        Ok(Fresh {
            inputs,
            read,
            derived,
            inventory,
        })
    }

    /// `owner`'s inventory as `slot`'s read at `at` holding `inputs` joins it.
    fn inventory(
        &mut self,
        state: &State,
        at: i32,
        slot: u32,
        owner: u32,
        inputs: &[ResearchInputRow],
    ) -> io::Result<Inventory> {
        let held = columns::table(state, TableId::ResearchTechnologies)?;
        let technology = self.technologies(state)?;
        if at.saturating_sub(REACH) < self.first {
            let changed = self.technologies.changed(u64::from(owner));
            if let Some((when, rows)) = self.inventories.get(&owner)
                && *when == changed
            {
                return Ok(Inventory::Owner(Arc::clone(rows)));
            }
            self.technologies
                .members(state, u64::from(owner), &mut self.found)?;
            let mut rows = Vec::with_capacity(self.found.len());
            for &found in &self.found {
                let index = held.key(found).map_or(0, |key| key as u32);
                rows.push((index, technology(found)?));
            }
            let rows: Arc<[(u32, Technology)]> = rows.into();
            self.inventories.insert(owner, (changed, Arc::clone(&rows)));
            return Ok(Inventory::Owner(rows));
        }
        let mut covered: BTreeSet<u32> = occurrences(inputs).collect();
        for (hour, held) in self.seen.get(&slot).into_iter().flatten() {
            if *hour >= at.saturating_sub(REACH) {
                covered.extend(held.iter().copied());
            }
        }
        let mut rows = Vec::with_capacity(covered.len());
        for index in covered {
            if let Some(found) = held.slot(u128::from(owner) << 32 | u128::from(index)) {
                rows.push((index, technology(found)?));
            }
        }
        Ok(Inventory::Rows(rows))
    }

    /// The literal LexicalTokens holds for `token`, as a grant restriction spells it.
    fn spelling(&self, state: &State, token: u32) -> io::Result<Result<String, String>> {
        let table = TableId::LexicalTokens;
        let held = columns::table(state, table)?;
        let Some(slot) = held.slot(u128::from(token)) else {
            return Ok(Err(format!(
                "Research grant restriction token {token} is not recorded"
            )));
        };
        let literal = if held.recorded(self.literal, slot) {
            let text = *columns::text(held, self.literal)?
                .get(slot as usize)
                .ok_or_else(|| io::Error::other("A token's slot holds no cell"))?;
            state
                .dictionary()
                .get(text)
                .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?
                .to_vec()
        } else {
            columns::row::<LexicalTokenRow>(state, table, slot, self.format)?.literal
        };
        Ok(String::from_utf8(literal)
            .map_err(|_| format!("Research grant restriction token {token} has invalid text")))
    }

    /// What `owners`' grant rows hold, in their table order, and whether they spell a token.
    fn assemble(&mut self, state: &State, owners: &[u32]) -> io::Result<(Content, bool)> {
        let mut selected: Vec<ResearchModifierRow> = Vec::new();
        let mut rows = Vec::new();
        for &owner in owners {
            Self::group_rows(
                state,
                &mut self.modifiers,
                scope(owner, INVENTORY_INDEX),
                self.format,
                &mut self.found,
                &mut rows,
            )?;
            selected.append(&mut rows);
        }
        let spells = selected.iter().any(grants::spells);
        if !columns::complete(state, TableId::LexicalTokens) {
            let reason = incomplete(state, TableId::LexicalTokens, self.mode)?;
            return Ok((Content::Table(reason), spells));
        }
        let refs: Vec<&ResearchModifierRow> = selected.iter().collect();
        let mut failed = None;
        let assembled = grants::assemble(&refs, |token| match self.spelling(state, token) {
            Ok(spelled) => spelled,
            Err(error) => {
                failed = Some(error);
                Err(String::new())
            }
        });
        if let Some(error) = failed {
            return Err(error);
        }
        let content = match assembled {
            Ok(inventory) => Content::Inventory(inventory),
            Err(reason) => Content::Refused(reason),
        };
        Ok((content, spells))
    }

    /// What `tag`'s grant inventory reads at the hour, and whether its rows spell a token; its
    /// rows are assembled again only where one of them changed.
    fn grant(
        &mut self,
        state: &State,
        roster: &Roster,
        name: &str,
        tag: Tag,
    ) -> io::Result<(Grant, bool)> {
        let identity = match roster.complete().then(|| roster.tagged(tag)) {
            Some(Tagged::Found(country)) => Some((country.id, country.slot)),
            _ => None,
        };
        let Some((id, slot)) = identity else {
            return Ok((Grant::NONE, false));
        };
        let read = self.stamps.read(MODIFIERS, id);
        if !columns::complete(state, TableId::ResearchModifiers) {
            let grant = Grant {
                identity,
                read,
                matched: false,
                content: None,
            };
            return Ok((grant, false));
        }
        let aliases = roster.aliases(slot);
        let group = |owner: u32| scope(owner, INVENTORY_INDEX);
        let matched = self.stamps.complete
            && aliases
                .iter()
                .filter(|&&owner| self.modifiers.changed(group(owner)).is_some())
                .all(|&owner| self.stamps.read(MODIFIERS, owner) == read);
        let kept = self.assembled.get(name).filter(|held| {
            held.from.len() == aliases.len()
                && held.from.iter().zip(aliases).all(|(&(kept, changed), &owner)| {
                    kept == owner && changed == self.modifiers.changed(group(owner))
                })
        });
        let (content, spells) = match kept {
            Some(held) => (Arc::clone(&held.content), held.spells),
            None => {
                let (content, spells) = self.assemble(state, aliases)?;
                let old = self.assembled.get(name).map(|held| &held.content);
                let content = share(old, content);
                let from = aliases
                    .iter()
                    .map(|&owner| (owner, self.modifiers.changed(group(owner))))
                    .collect();
                self.assembled.insert(
                    name.to_owned(),
                    Assembled {
                        from,
                        content: Arc::clone(&content),
                        spells,
                    },
                );
                (content, spells)
            }
        };
        let grant = Grant {
            identity,
            read,
            matched,
            content: Some(content),
        };
        Ok((grant, spells))
    }

    /// The tags whose grant facts the hour may have changed, read again and sent where they
    /// did: every tag where the countries or a table's completeness changed, else those of the
    /// slots whose owners' rows or read hours changed, and those spelling a token where the
    /// tokens changed. The facts sent.
    fn grants(
        &mut self,
        hour: &segments::Hour<'_, World>,
        roster: &Roster,
        cold: bool,
    ) -> io::Result<Option<Box<Grants>>> {
        let state = hour.state;
        let lexical = TableId::LexicalTokens;
        // An incomplete LexicalTokens' reason is every inventory's refusal: a new one reads all.
        let shapes = (
            [
                columns::complete(state, TableId::ResearchModifiers),
                self.stamps.complete,
            ],
            columns::status(state, lexical),
        );
        let reshaped = self.shapes != Some(shapes);
        let complete =
            (self.shapes.map(|held| held.0[0]) != Some(shapes.0[0])).then_some(shapes.0[0]);
        self.shapes = Some(shapes);
        if reshaped {
            self.assembled.clear();
        }
        let all = cold || reshaped || roster.changed();
        let mut tags: BTreeSet<String> = BTreeSet::new();
        if all {
            self.slot_tags.clear();
            for tag in roster.held() {
                let Ok(name) = std::str::from_utf8(roster.tag(tag)) else {
                    continue;
                };
                if let Tagged::Found(country) = roster.tagged(tag) {
                    self.slot_tags
                        .entry(country.slot)
                        .or_default()
                        .push(name.to_owned());
                }
                tags.insert(name.to_owned());
            }
            tags.extend(self.granted.keys().cloned());
        } else {
            let mut slots = BTreeSet::new();
            for &owner in self.stamps.touched[MODIFIERS]
                .iter()
                .chain(&self.modifiers.touched)
            {
                slots.extend(roster.slot_of(owner));
            }
            for slot in slots {
                tags.extend(self.slot_tags.get(&slot).into_iter().flatten().cloned());
            }
        }
        if columns::complete(state, lexical)
            && (hour.world.restated(lexical) || touched(state, lexical)?)
        {
            // The tokens they spell are read again.
            for name in &self.spelled {
                self.assembled.remove(name);
            }
            if !all {
                tags.extend(self.spelled.iter().cloned());
            }
        }
        let mut facts = Vec::new();
        for name in tags {
            let (grant, spells) = match roster.tag_of(name.as_bytes()) {
                Some(tag) => self.grant(state, roster, &name, tag)?,
                None => (Grant::NONE, false),
            };
            let failed = matches!(
                grant.content.as_deref(),
                Some(Content::Refused(reason)) if reason.starts_with("Research grant restriction token")
            );
            if spells || failed {
                self.spelled.insert(name.clone());
            } else {
                self.spelled.remove(&name);
            }
            let sent = self.granted.get(&name);
            if sent.is_some_and(|sent| sent.same(&grant)) {
                continue;
            }
            self.granted.insert(name.clone(), grant.clone());
            facts.push((name, grant));
        }
        // A cold projector sent nothing before, so it names every tag the roster holds.
        let whole = cold;
        if !(whole || complete.is_some() || !facts.is_empty()) {
            return Ok(None);
        }
        Ok(Some(Box::new(Grants {
            complete,
            whole,
            facts,
        })))
    }

    /// Hour `hour`'s index step, fresh reads and grant facts.
    fn project(
        &mut self,
        hour: &segments::Hour<'_, World>,
        cold: bool,
    ) -> io::Result<(Index, BTreeMap<u32, Fresh>, Option<Box<Grants>>)> {
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        self.inputs.update(state, cold, at)?;
        self.technologies.update(state, cold, at)?;
        self.bonuses.update(state, cold, at)?;
        self.modifiers.update(state, cold, at)?;
        self.stamps.update(state, cold, self.sample_hour)?;
        let (index, fresh) = self.index(state, roster, at)?;
        let grants = self.grants(hour, roster, cold)?;
        Ok((index, fresh, grants))
    }

    fn heap(&self) -> usize {
        let lists = |lists: &BTreeMap<u32, Vec<(u32, i32)>>| -> usize {
            lists.values().map(|list| 48 + list.capacity() * 8).sum()
        };
        let keyed = |keyed: &Keyed| -> usize {
            keyed.groups.capacity() * (size_of::<(u64, Group)>() + 1)
                + keyed.owners.capacity() * (size_of::<(u32, u32)>() + 1)
                + keyed.touched.capacity() * 4
                + keyed.index.as_ref().map_or(0, |index| {
                    index.capacity() * (size_of::<(u64, Vec<(u32, u32)>)>() + 1)
                        + index.values().map(|members| members.capacity() * 8).sum::<usize>()
                })
        };
        self.slots.capacity() * 16
            + self
                .country_rows
                .values()
                .flatten()
                .map(|(_, _, tag)| size_of::<(usize, u32, Vec<u8>)>() + tag.capacity())
                .sum::<usize>()
            + self
                .owners
                .values()
                .map(|ids| 48 + ids.capacity() * 4)
                .sum::<usize>()
            + self.input_slots.len() * 8
            + lists(&self.input_dates)
            + self.by_read.values().map(|slots| 48 + slots.len() * 8).sum::<usize>()
            + keyed(&self.inputs)
            + keyed(&self.technologies)
            + keyed(&self.bonuses)
            + keyed(&self.modifiers)
            + self.stamps.heap()
            + self
                .seen
                .values()
                .map(|reads| {
                    48 + reads
                        .iter()
                        .map(|(_, held)| 16 + held.capacity() * 4)
                        .sum::<usize>()
                })
                .sum::<usize>()
            + self
                .inventories
                .values()
                .map(|(_, rows)| 48 + rows.len() * size_of::<(u32, Technology)>())
                .sum::<usize>()
            + self
                .breakdowns
                .values()
                .map(|held| {
                    size_of::<((u32, u32), Breakdown)>()
                        + held.value.as_deref().map_or(0, modifiers::bytes)
                })
                .sum::<usize>()
            + self
                .attributions
                .values()
                .map(|held| {
                    size_of::<((u32, u32), Attributed)>()
                        + held.value.as_deref().map_or(0, |value| bonuses_heap(value))
                })
                .sum::<usize>()
            + self
                .granted
                .iter()
                .map(|(tag, grant)| {
                    size_of::<(String, Grant)>()
                        + tag.capacity()
                        + grant
                            .content
                            .as_ref()
                            .map_or(0, |content| size_of::<Content>() + 16 + content.heap())
                })
                .sum::<usize>()
            + self
                .assembled
                .iter()
                .map(|(tag, held)| {
                    size_of::<(String, Assembled)>() + tag.capacity() + held.from.capacity() * 12
                })
                .sum::<usize>()
            + self
                .slot_tags
                .values()
                .flatten()
                .map(|tag| 24 + tag.capacity())
                .sum::<usize>()
            + self.spelled.iter().map(|tag| 48 + tag.capacity()).sum::<usize>()
            + self.found.capacity() * 4
            + (self.shared_rows.capacity() + self.slot_rows.capacity())
                * size_of::<ResearchModifierRow>()
    }
}

// Every segment starts cold: its first hour reads every holder, link, row and read hour again;
// what came before is the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, first: i32) -> io::Result<()> {
        self.cold(first)
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("research.columnar", Some("research"));
        let state = hour.state;
        let at = hour.hour;
        let cold = hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if cold {
            if let Some(last) = self.last {
                self.hand_over(last, out)
                    .map_err(segments::family_failure)?;
            }
            self.cold(at)?;
        }
        self.branch = hour.branch;
        // Only the hour's own read refuses, before the lens folds any of it: the lens hands back
        // exactly the hours before, as the emit holds the countries it folds itself.
        let (index, fresh, grants) = match self.project(hour, cold) {
            Ok(projected) => projected,
            Err(error) => {
                if let Some(last) = self.last {
                    self.hand_over(last, out)
                        .map_err(segments::family_failure)?;
                }
                return Err(error);
            }
        };
        self.last = Some(at);
        // What the lens adds, and handing the hour on, fail only where the fold itself is
        // wrong: never a refusal, so no country keeps a part of the hour another lacks.
        (|| -> io::Result<()> {
            if cold {
                let virgins = self.virgins(state, at)?;
                self.lens = Some(lens::Lens::start(at, virgins));
            }
            let part = match &mut self.lens {
                Some(lens) => {
                    // The lens reads the indexes this step left, as the emit's adopt of it would.
                    let indexes = Indexes {
                        slots: self.held.then_some(&*self.slots),
                        rows: &self.country_rows,
                        owners: &self.owners,
                        dates: &self.input_dates,
                    };
                    lens.hour(at, &index, indexes, &fresh)?;
                    lens.part()
                }
                None => None,
            };
            let index = match index {
                Index::Changed(mut changed) => {
                    changed.fresh = fresh;
                    Index::Changed(changed)
                }
                index => index,
            };
            out.push(Hour {
                branch: hour.branch,
                hour: at,
                index,
                grants,
                lens: part,
                handover: false,
            })
        })()
        .map_err(segments::family_failure)
    }

    fn end(&mut self, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        match self.last {
            Some(last) => self
                .hand_over(last, out)
                .map_err(segments::family_failure),
            None => Ok(()),
        }
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
                "Not a saved research projector",
            ));
        }
        let last = self.last;
        self.cold(last.unwrap_or(self.first))?;
        self.last = None;
        Ok(())
    }
}
