//! Lend-lease balance facts, projected hour by hour from a columnar walk ([`replay::segments`]).
//! An hour is read where the row preparation read one: a segment's first, one after an hour not
//! walked, and one whose Countries, CountryLinks, WarOwnedRelations or WarLendLeaseHistories
//! changed. A read hands the emit what changed since the segment last sent it: the owner-partner
//! records, the owners' totals, the owners whose dropped records are zero, the tags due and the
//! read's refusal counts; or why nothing is known. Rows are kept by slot from the hour's changes;
//! a segment's first read, a link change or a table changing status reads them whole. What came
//! before the segment is the ordered emit's ([`super::emit`]).

use super::MEASURES;
use dashboard_world::{Needs, Roster, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{TableId, WarLendLeaseHistoryRow, WarOwnedRelationRow},
    scope::CaptureMode,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarOwnedRelations,
    TableId::WarLendLeaseHistories,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// A record's measures, as recorded.
pub type Values = [i64; MEASURES];
/// An owner's totals of its records.
pub type Sums = [i128; MEASURES];

/// How a read reads the war rosters, for its refusals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, replay_spool::Spooled)]
pub enum Rosters {
    /// Not read: a record names no owner identity.
    Skipped,
    /// Not complete: the refusal, naming the hour.
    Unavailable(String),
    /// Read, with this many roster countries naming no identity.
    Read(u64),
}

/// A read's refusal counts: records with no owner identity, records with no partner identity,
/// and the rosters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, replay_spool::Spooled)]
pub struct Counts {
    pub orphans: u64,
    pub partners: u64,
    pub rosters: Rosters,
}

/// A read's changes since the segment last sent one; everything it holds when `whole`.
#[derive(Debug, Default, PartialEq, Eq, replay_spool::Spooled)]
pub struct Read {
    pub whole: bool,
    /// Owners whose dropped records are zero (true) or end (false).
    pub zeroable: Vec<(u32, bool)>,
    /// Each owner-partner record by holder slots, none where no row holds it any more.
    pub records: Vec<((u32, u32), Option<Values>)>,
    /// Each owner's totals, none where it holds no total any more.
    pub totals: Vec<(u32, Option<Sums>)>,
    /// The slots the read tags whose tag is due, with it.
    pub tags: Vec<(u32, Arc<str>)>,
    pub counts: Option<Counts>,
}

/// What an hour hands the emit.
#[derive(Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum Facts {
    /// A read that changed nothing the segment sent.
    Same,
    Read(Box<Read>),
    /// A read that knows nothing, and why.
    Refused(String),
}

/// One hour of a segment: none where the row preparation read nothing.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub facts: Option<Facts>,
}

/// The dirty sets a change of the kept rows marks.
#[derive(Default)]
struct Dirty {
    records: HashSet<(u32, u32)>,
    owners: HashSet<u32>,
    /// Slots whose place among those a read tags may have changed.
    tagged: HashSet<u32>,
}

impl Dirty {
    fn clear(&mut self) {
        self.records.clear();
        self.owners.clear();
        self.tagged.clear();
    }
}

/// A WarLendLeaseHistories row: its key, its owner's and partner's holder slots, none where no
/// link resolves them, and its measures.
#[derive(Clone, Copy)]
struct History {
    key: u128,
    owner: Option<u32>,
    target: Option<u32>,
    values: Values,
}

/// An owner's records: their sums, how many there are, and how many name no partner identity.
#[derive(Default)]
struct Owned {
    sums: Sums,
    rows: u32,
    partial: u32,
}

/// The histories by table slot, by owner-partner pair and by owner.
#[derive(Default)]
struct Histories {
    rows: Vec<Option<History>>,
    /// The table slots of each pair's rows: several raw ids can link to one holder.
    pairs: HashMap<(u32, u32), Vec<u32>>,
    owners: HashMap<u32, Owned>,
    /// Rows naming each partner.
    targets: HashMap<u32, u32>,
    orphans: u64,
    partners: u64,
}

impl Histories {
    fn clear(&mut self) {
        self.rows.clear();
        self.pairs.clear();
        self.owners.clear();
        self.targets.clear();
        self.orphans = 0;
        self.partners = 0;
    }

    /// Marks everything kept dirty, as a whole read clearing it changes it.
    fn dirty_all(&self, dirty: &mut Dirty) {
        dirty.records.extend(self.pairs.keys().copied());
        dirty.owners.extend(self.owners.keys().copied());
        dirty.tagged.extend(self.owners.keys().copied());
        dirty.tagged.extend(self.targets.keys().copied());
    }

    fn remove(&mut self, slot: u32, dirty: &mut Dirty) {
        let Some(row) = self.rows.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        let Some(owner) = row.owner else {
            self.orphans -= 1;
            return;
        };
        dirty.owners.insert(owner);
        dirty.tagged.insert(owner);
        let owned = self.owners.get_mut(&owner).expect("indexed owner");
        owned.rows -= 1;
        for (sum, value) in owned.sums.iter_mut().zip(row.values) {
            *sum -= i128::from(value);
        }
        match row.target {
            None => {
                owned.partial -= 1;
                self.partners -= 1;
            }
            Some(target) => {
                let rows = self.pairs.get_mut(&(owner, target)).expect("indexed pair");
                rows.retain(|&held| held != slot);
                if rows.is_empty() {
                    self.pairs.remove(&(owner, target));
                }
                let named = self.targets.get_mut(&target).expect("indexed partner");
                *named -= 1;
                if *named == 0 {
                    self.targets.remove(&target);
                }
                dirty.records.insert((owner, target));
                dirty.tagged.insert(target);
            }
        }
        if owned.rows == 0 {
            self.owners.remove(&owner);
        }
    }

    fn insert(&mut self, slot: u32, row: History, dirty: &mut Dirty) {
        self.remove(slot, dirty);
        let index = slot as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, None);
        }
        self.rows[index] = Some(row);
        let Some(owner) = row.owner else {
            self.orphans += 1;
            return;
        };
        dirty.owners.insert(owner);
        dirty.tagged.insert(owner);
        let owned = self.owners.entry(owner).or_default();
        owned.rows += 1;
        for (sum, value) in owned.sums.iter_mut().zip(row.values) {
            *sum += i128::from(value);
        }
        match row.target {
            None => {
                owned.partial += 1;
                self.partners += 1;
            }
            Some(target) => {
                self.pairs.entry((owner, target)).or_default().push(slot);
                *self.targets.entry(target).or_default() += 1;
                dirty.records.insert((owner, target));
                dirty.tagged.insert(target);
            }
        }
    }

    /// A pair's record: the row last by key, as a row read keeps the last of equal pairs.
    fn record(&self, pair: (u32, u32)) -> Option<Values> {
        self.pairs
            .get(&pair)?
            .iter()
            .filter_map(|&slot| self.rows[slot as usize])
            .max_by_key(|row| row.key)
            .map(|row| row.values)
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<Option<History>>()
            + self.pairs.capacity() * size_of::<((u32, u32), Vec<u32>)>()
            + self.pairs.values().map(|rows| rows.capacity() * 4).sum::<usize>()
            + self.owners.capacity() * size_of::<(u32, Owned)>()
            + self.targets.capacity() * size_of::<(u32, u32)>()
    }
}

/// The war rosters by table slot: each row's owner and sides as holder slots, none where no link
/// resolves them, and the rows naming each holder.
#[derive(Default)]
struct Relations {
    rows: Vec<Option<[Option<u32>; 3]>>,
    named: HashMap<u32, u32>,
    unresolved: u64,
}

impl Relations {
    fn clear(&mut self) {
        self.rows.clear();
        self.named.clear();
        self.unresolved = 0;
    }

    /// Marks every holder kept dirty, as a whole read clearing them changes them.
    fn dirty_all(&self, dirty: &mut Dirty) {
        dirty.owners.extend(self.named.keys().copied());
        dirty.tagged.extend(self.named.keys().copied());
    }

    fn remove(&mut self, slot: u32, dirty: &mut Dirty) {
        let Some(row) = self.rows.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        for country in row {
            let Some(holder) = country else {
                self.unresolved -= 1;
                continue;
            };
            let named = self.named.get_mut(&holder).expect("indexed roster holder");
            *named -= 1;
            if *named == 0 {
                self.named.remove(&holder);
                dirty.owners.insert(holder);
                dirty.tagged.insert(holder);
            }
        }
    }

    fn insert(&mut self, slot: u32, row: [Option<u32>; 3], dirty: &mut Dirty) {
        self.remove(slot, dirty);
        let index = slot as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, None);
        }
        self.rows[index] = Some(row);
        for country in row {
            let Some(holder) = country else {
                self.unresolved += 1;
                continue;
            };
            let named = self.named.entry(holder).or_default();
            *named += 1;
            if *named == 1 {
                dirty.owners.insert(holder);
                dirty.tagged.insert(holder);
            }
        }
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<Option<[Option<u32>; 3]>>()
            + self.named.capacity() * size_of::<(u32, u32)>()
    }
}

/// The tag a read records for `slot`: the UTF-8 tag of its last country by id.
fn tag_of(roster: &Roster, slot: u32) -> Option<&str> {
    let country = roster.at_slot(slot).last()?;
    std::str::from_utf8(roster.tag(country.tag))
        .ok()
        .filter(|tag| !tag.is_empty())
}

/// Whether the hour inserted, deleted or wrote a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || !changes.fill_slots.is_empty()
        || changes.written.iter().any(|&mask| mask != 0))
}

/// Visits each slot of `table` the hour inserted, then each it may have changed `mask` of.
fn each_written(
    state: &State,
    table: TableId,
    mask: u64,
    mut visit: impl FnMut(u32) -> io::Result<()>,
) -> io::Result<()> {
    let changes = columns::changes(state, table)?;
    for &slot in &changes.inserted {
        visit(slot)?;
    }
    let written = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks));
    for (&slot, &fields) in written {
        if fields & mask != 0 {
            visit(slot)?;
        }
    }
    Ok(())
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    /// The histories' measure columns, in measure order, and the rosters' side columns.
    measures: [usize; MEASURES],
    sides: [usize; 2],
    cold: bool,
    last: Option<i32>,
    histories: Histories,
    relations: Relations,
    /// Whether the rosters are kept complete.
    rostered: bool,
    /// The next read reads every row of the histories, or of the rosters.
    stale: [bool; 2],
    dirty: Dirty,
    /// Whether the emit holds what the segment sent since its last whole read, and what that is.
    sent: bool,
    usable: bool,
    records: HashMap<(u32, u32), Values>,
    totals: HashMap<u32, Sums>,
    zeroable: HashSet<u32>,
    tags: HashMap<u32, Arc<str>>,
    counts: Option<Counts>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"lend-lease projector: cold";

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let history = |name| columns::field(TableId::WarLendLeaseHistories, name);
        let relation = |name| columns::field(TableId::WarOwnedRelations, name);
        Ok(Self {
            format,
            mode,
            measures: [
                history("industry_sent")?,
                history("fuel_sent")?,
                history("industry_received")?,
                history("fuel_received")?,
            ],
            sides: [relation("first_country")?, relation("second_country")?],
            cold: true,
            last: None,
            histories: Histories::default(),
            relations: Relations::default(),
            rostered: false,
            stale: [true; 2],
            dirty: Dirty::default(),
            sent: false,
            usable: false,
            records: HashMap::new(),
            totals: HashMap::new(),
            zeroable: HashSet::new(),
            tags: HashMap::new(),
            counts: None,
        })
    }

    /// Why `table` is not complete at `hour`, as the row decoder words it; none while it is.
    fn gap(&self, state: &State, table: TableId, hour: i32) -> io::Result<Option<String>> {
        Ok(Some(match columns::status(state, table) {
            columns::Status::Complete => return Ok(None),
            columns::Status::Unavailable { reason } => {
                let reason = state.dictionary().get(reason).ok_or_else(|| {
                    io::Error::other(format!("A {table:?} reason names no dictionary entry"))
                })?;
                let reason = String::from_utf8_lossy(reason);
                format!("hour {hour} {table:?} is unavailable: {reason}")
            }
            columns::Status::Truncated { missing } => {
                format!("hour {hour} {table:?} is truncated ({missing} missing rows)")
            }
            columns::Status::NotRecorded => {
                format!("hour {hour} {table:?} is not recorded in a {} recording", self.mode)
            }
        }))
    }

    fn history(&self, state: &State, roster: &Roster, slot: u32) -> io::Result<History> {
        let table = TableId::WarLendLeaseHistories;
        let held = columns::table(state, table)?;
        let key = held
            .key(slot)
            .ok_or_else(|| io::Error::other("A lend-lease history slot holds no row"))?;
        let values = if self.measures.iter().all(|&field| held.recorded(field, slot)) {
            let mut values = [0; MEASURES];
            for (value, &field) in values.iter_mut().zip(&self.measures) {
                *value = *columns::plain::<i64>(held, field)?
                    .get(slot as usize)
                    .ok_or_else(|| io::Error::other("A lend-lease history slot holds no cell"))?;
            }
            values
        } else {
            let row: WarLendLeaseHistoryRow = columns::row(state, table, slot, self.format.version())?;
            [
                row.industry_sent,
                row.fuel_sent,
                row.industry_received,
                row.fuel_received,
            ]
        };
        Ok(History {
            key,
            owner: roster.slot_of((key >> 32) as u32),
            target: roster.slot_of(key as u32),
            values,
        })
    }

    fn relation(&self, state: &State, roster: &Roster, slot: u32) -> io::Result<[Option<u32>; 3]> {
        let table = TableId::WarOwnedRelations;
        let held = columns::table(state, table)?;
        let key = held
            .key(slot)
            .ok_or_else(|| io::Error::other("A war roster slot holds no row"))?;
        let sides = if self.sides.iter().all(|&field| held.recorded(field, slot)) {
            let mut sides = [0; 2];
            for (side, &field) in sides.iter_mut().zip(&self.sides) {
                *side = *columns::plain::<u32>(held, field)?
                    .get(slot as usize)
                    .ok_or_else(|| io::Error::other("A war roster slot holds no cell"))?;
            }
            sides
        } else {
            let row: WarOwnedRelationRow = columns::row(state, table, slot, self.format.version())?;
            [row.first_country, row.second_country]
        };
        Ok([
            roster.slot_of((key >> 32) as u32),
            roster.slot_of(sides[0]),
            roster.slot_of(sides[1]),
        ])
    }

    /// The histories kept: read whole, or by the hour's changes.
    fn follow_histories(&mut self, state: &State, roster: &Roster, whole: bool) -> io::Result<()> {
        let table = TableId::WarLendLeaseHistories;
        if whole {
            self.histories.dirty_all(&mut self.dirty);
            self.histories.clear();
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            for slot in live {
                let row = self.history(state, roster, slot)?;
                self.histories.insert(slot, row, &mut self.dirty);
            }
            return Ok(());
        }
        for &slot in &columns::changes(state, table)?.deleted {
            self.histories.remove(slot, &mut self.dirty);
        }
        let mask = self.measures.iter().fold(0_u64, |mask, &field| mask | 1 << field);
        each_written(state, table, mask, |slot| {
            let row = self.history(state, roster, slot)?;
            self.histories.insert(slot, row, &mut self.dirty);
            Ok(())
        })
    }

    /// The rosters kept: read whole, or by the hour's changes.
    fn follow_relations(&mut self, state: &State, roster: &Roster, whole: bool) -> io::Result<()> {
        let table = TableId::WarOwnedRelations;
        if whole {
            self.relations.dirty_all(&mut self.dirty);
            self.relations.clear();
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            for slot in live {
                let row = self.relation(state, roster, slot)?;
                self.relations.insert(slot, row, &mut self.dirty);
            }
            return Ok(());
        }
        for &slot in &columns::changes(state, table)?.deleted {
            self.relations.remove(slot, &mut self.dirty);
        }
        let mask = self.sides.iter().fold(0_u64, |mask, &field| mask | 1 << field);
        each_written(state, table, mask, |slot| {
            let row = self.relation(state, roster, slot)?;
            self.relations.insert(slot, row, &mut self.dirty);
            Ok(())
        })
    }

    /// Forgets what the segment sent: the emit holds nothing of it after a refused read.
    fn forget(&mut self) {
        self.sent = false;
        self.stale = [true; 2];
        self.histories.clear();
        self.relations.clear();
        self.rostered = false;
        self.dirty.clear();
    }

    /// The totals a read holds for `owner`: its records' sums, zeros while the rosters read name
    /// it, else none.
    fn total(&self, owner: u32, usable: bool) -> Option<Sums> {
        match self.histories.owners.get(&owner) {
            Some(owned) => Some(owned.sums),
            None => (usable && self.relations.named.contains_key(&owner)).then_some([0; MEASURES]),
        }
    }

    /// Whether the records `owner` drops are zero: the rosters read name it, and each of its
    /// records names its partner.
    fn zeroes(&self, owner: u32, usable: bool) -> bool {
        usable
            && self.relations.named.contains_key(&owner)
            && self
                .histories
                .owners
                .get(&owner)
                .is_none_or(|owned| owned.partial == 0)
    }

    /// Whether a read tags `slot`: it owns or is the partner of a record, or the rosters read name it.
    fn tagged(&self, slot: u32, usable: bool) -> bool {
        self.histories.owners.contains_key(&slot)
            || self.histories.targets.contains_key(&slot)
            || (usable && self.relations.named.contains_key(&slot))
    }

    /// The read's changes since the last sent, or everything when the emit holds nothing.
    fn read(&mut self, roster: &Roster, rosters: Option<String>) -> Facts {
        let usable = self.rostered && self.histories.orphans == 0;
        let whole = !self.sent;
        let mut read = Read {
            whole,
            ..Read::default()
        };
        let mut dirty = std::mem::take(&mut self.dirty);
        if whole {
            self.records.clear();
            self.totals.clear();
            self.zeroable.clear();
            self.tags.clear();
            self.counts = None;
            dirty.clear();
            dirty.records.extend(self.histories.pairs.keys().copied());
            dirty.owners.extend(self.histories.owners.keys().copied());
            dirty.owners.extend(self.relations.named.keys().copied());
            dirty.tagged.extend(dirty.owners.iter().copied());
            dirty.tagged.extend(self.histories.targets.keys().copied());
        } else if usable != self.usable {
            dirty.owners.extend(self.relations.named.keys().copied());
            dirty.tagged.extend(self.relations.named.keys().copied());
        }
        if roster.changed() {
            dirty.tagged.extend(self.histories.owners.keys().copied());
            dirty.tagged.extend(self.histories.targets.keys().copied());
            dirty.tagged.extend(self.relations.named.keys().copied());
        }
        for pair in dirty.records.drain() {
            let now = self.histories.record(pair);
            if self.records.get(&pair) != now.as_ref() {
                match now {
                    Some(values) => self.records.insert(pair, values),
                    None => self.records.remove(&pair),
                };
                read.records.push((pair, now));
            }
        }
        for owner in dirty.owners.drain() {
            let total = self.total(owner, usable);
            if self.totals.get(&owner) != total.as_ref() {
                match total {
                    Some(sums) => self.totals.insert(owner, sums),
                    None => self.totals.remove(&owner),
                };
                read.totals.push((owner, total));
            }
            let zeroes = self.zeroes(owner, usable);
            if self.zeroable.contains(&owner) != zeroes {
                if zeroes {
                    self.zeroable.insert(owner);
                } else {
                    self.zeroable.remove(&owner);
                }
                read.zeroable.push((owner, zeroes));
            }
        }
        for slot in dirty.tagged.drain() {
            if !self.tagged(slot, usable) {
                continue;
            }
            let Some(tag) = tag_of(roster, slot) else {
                continue;
            };
            if self.tags.get(&slot).is_none_or(|sent| **sent != *tag) {
                let tag: Arc<str> = Arc::from(tag);
                self.tags.insert(slot, Arc::clone(&tag));
                read.tags.push((slot, tag));
            }
        }
        self.dirty = dirty;
        let counts = Counts {
            orphans: self.histories.orphans,
            partners: self.histories.partners,
            rosters: match rosters {
                _ if self.histories.orphans != 0 => Rosters::Skipped,
                Some(reason) => Rosters::Unavailable(reason),
                None => Rosters::Read(self.relations.unresolved),
            },
        };
        if self.counts.as_ref() != Some(&counts) {
            self.counts = Some(counts.clone());
            read.counts = Some(counts);
        }
        self.sent = true;
        self.usable = usable;
        read.records.sort_unstable_by_key(|&(pair, _)| pair);
        read.totals.sort_unstable_by_key(|&(owner, _)| owner);
        read.zeroable.sort_unstable();
        read.tags.sort_unstable_by_key(|&(slot, _)| slot);
        let same = !whole
            && read.records.is_empty()
            && read.totals.is_empty()
            && read.zeroable.is_empty()
            && read.tags.is_empty()
            && read.counts.is_none();
        if same {
            Facts::Same
        } else {
            Facts::Read(Box::new(read))
        }
    }

    fn heap(&self) -> usize {
        self.histories.heap()
            + self.relations.heap()
            + (self.dirty.records.capacity() + self.records.capacity()) * size_of::<((u32, u32), Values)>()
            + (self.dirty.owners.capacity() + self.dirty.tagged.capacity() + self.zeroable.capacity()) * 4
            + self.totals.capacity() * size_of::<(u32, Sums)>()
            + self.tags.capacity() * size_of::<(u32, Arc<str>)>()
            + self.tags.values().map(|tag| tag.len()).sum::<usize>()
    }
}

// Every segment starts cold: its first hour reads every row and country again; what came before
// is the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(&mut self, hour: &segments::Hour<'_, World>, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("lend_lease.columnar", Some("lend_lease_balance"));
        let state = hour.state;
        let at = hour.hour;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        if opens {
            self.forget();
        }
        let mut read = opens;
        for &table in TABLES {
            read = read || hour.world.restated(table) || touched(state, table)?;
        }
        if !read {
            return out.push(Hour {
                branch: hour.branch,
                hour: at,
                facts: None,
            });
        }
        let roster = hour.world.countries()?;
        let refusal = if let Some(gap) = self.gap(state, TableId::Countries, at)? {
            Some(format!("country identities unavailable: {gap}"))
        } else if let Some(gap) = self.gap(state, TableId::CountryLinks, at)? {
            Some(format!("country identities unavailable: {gap}"))
        } else {
            self.gap(state, TableId::WarLendLeaseHistories, at)?
                .map(|gap| format!("lend-lease histories unavailable: {gap}"))
        };
        if let Some(reason) = refusal {
            self.forget();
            return out.push(Hour {
                branch: hour.branch,
                hour: at,
                facts: Some(Facts::Refused(reason)),
            });
        }
        let regrouped = roster.rebuilt() || !roster.changed_links().is_empty();
        let histories = TableId::WarLendLeaseHistories;
        let whole = std::mem::take(&mut self.stale[0]) || regrouped || hour.world.restated(histories);
        if whole || touched(state, histories)? {
            self.follow_histories(state, roster, whole)?;
        }
        let relations = TableId::WarOwnedRelations;
        let unrostered = self.gap(state, relations, at)?;
        if unrostered.is_some() {
            self.relations.dirty_all(&mut self.dirty);
            self.relations.clear();
            self.rostered = false;
            self.stale[1] = true;
        } else {
            let whole = std::mem::take(&mut self.stale[1]) || regrouped || hour.world.restated(relations);
            if whole || touched(state, relations)? {
                self.follow_relations(state, roster, whole)?;
            }
            self.rostered = true;
        }
        let rosters = unrostered
            .map(|gap| format!("war rosters unavailable, so unlisted totals are unknown: {gap}"));
        let facts = self.read(roster, rosters);
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            facts: Some(facts),
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
                "Not a saved lend-lease projector",
            ));
        }
        *self = Self::new(self.format, self.mode)?;
        Ok(())
    }
}
