//! Diplomacy's hours from a columnar walk ([`replay::segments`]). Each segment keeps, from the
//! tables' change lists, which owner every opinion and relation row belongs to, the reads
//! SliceReads stamps, and the incoming queue. An hour hands the ordered emit
//! ([`super::emit::Emit`]) the identities where they may have changed, the owners its tables
//! read then with their opinions and relation rows, shared with the read before while those
//! are unchanged, and the queue at a fresh read.
use super::{
    DiplomacyCountry,
    evidence::{self, Joins},
    opinions,
};
use dashboard_text::metadata::slot_read;
use dashboard_world::{Needs, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Operator, Sink, Start, State},
};
use replay_data::{
    cadence,
    format::FormatVersion,
    rows::{
        IncomingDiplomaticActionRow, LexicalTokenRow, RelationTupleRow, TABLE_COUNT, TableId,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables diplomacy reads: the identities, the read stamps, opinions, relations, the
/// incoming queue and the relation kinds' spellings.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::SliceReads,
    TableId::RelationOpinions,
    TableId::RelationTuples,
    TableId::IncomingDiplomaticActions,
    TableId::LexicalTokens,
];

/// What a projection reads of its walk's world: the countries, and their identity tables.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        countries: true,
        frame: dashboard_world::identity()?,
        ..Needs::default()
    })
}

/// A cell the recording did not record, which a row decoded from the columns refuses too.
fn unrecorded(table: TableId, slot: u32) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{table:?} slot {slot} holds no recorded value"),
    )
}

fn slice(hour: i32) -> u32 {
    hour.rem_euclid(cadence::STAGGER_PERIOD) as u32
}

/// One identity read, shared with the emit; a country is its index into `countries`.
#[derive(replay_spool::Spooled)]
pub(crate) struct Ids {
    /// Ascending.
    pub(crate) countries: Vec<DiplomacyCountry>,
    /// Country index by raw id, ascending.
    index: Vec<(u32, u32)>,
    /// Each country's raw aliases, as the links list them.
    pub(crate) aliases: Vec<Box<[u32]>>,
    /// By scheduled slice.
    slices: Vec<Vec<u32>>,
    /// Every Countries row's id, ascending.
    canonical: Vec<u32>,
    /// The tags no country resolves.
    pub(crate) unresolved: usize,
}

impl Ids {
    fn new(joins: Joins, canonical: Vec<u32>) -> Self {
        let countries: Vec<DiplomacyCountry> = joins.countries.into_values().collect();
        let index: Vec<(u32, u32)> = joins
            .raw
            .iter()
            .filter_map(|(&raw, country)| {
                Some((raw, countries.binary_search(country).ok()? as u32))
            })
            .collect();
        let aliases: Vec<Box<[u32]>> = countries
            .iter()
            .map(|country| {
                joins
                    .aliases
                    .get(country)
                    .map_or_else(Default::default, |raws| raws.as_slice().into())
            })
            .collect();
        let mut slices = vec![Vec::new(); cadence::STAGGER_PERIOD as usize];
        for (at, country) in countries.iter().enumerate() {
            slices[cadence::slice_of(country.country_id) as usize].push(at as u32);
        }
        Self {
            countries,
            index,
            aliases,
            slices,
            canonical,
            unresolved: joins.unresolved,
        }
    }

    /// The bytes it owns, itself included.
    fn heap(&self) -> usize {
        size_of::<Self>()
            + self.countries.capacity() * size_of::<DiplomacyCountry>()
            + self
                .countries
                .iter()
                .map(|country| country.tag.capacity())
                .sum::<usize>()
            + self.index.capacity() * size_of::<(u32, u32)>()
            + self.aliases.capacity() * size_of::<Box<[u32]>>()
            + self
                .aliases
                .iter()
                .map(|raws| raws.len() * size_of::<u32>())
                .sum::<usize>()
            + self.slices.capacity() * size_of::<Vec<u32>>()
            + self
                .slices
                .iter()
                .map(|slice| slice.capacity() * size_of::<u32>())
                .sum::<usize>()
            + self.canonical.capacity() * size_of::<u32>()
    }

    /// The country `raw` names.
    pub(crate) fn index_of(&self, raw: u32) -> Option<u32> {
        self.index
            .binary_search_by_key(&raw, |&(raw, _)| raw)
            .ok()
            .map(|at| self.index[at].1)
    }
}

/// An opinion owner's known pairs by target index, and its rows refusing them.
#[derive(replay_spool::Spooled)]
pub(crate) struct Values {
    /// Shared while the owner's targets do not change.
    pub(crate) targets: Arc<[u32]>,
    pub(crate) values: Box<[i32]>,
    pub(crate) unresolved: usize,
}

impl Values {
    /// The bytes it owns, itself and its targets included.
    fn heap(&self) -> usize {
        size_of::<Self>() + (self.targets.len() + self.values.len()) * size_of::<u32>()
    }

    /// Values of `pairs`, sharing what the owner's last values hold alike.
    fn new(pairs: Vec<(u32, i32)>, unresolved: usize, old: Option<&Arc<Self>>) -> Arc<Self> {
        let targets = old.map(|old| &old.targets).filter(|targets| {
            targets
                .iter()
                .copied()
                .eq(pairs.iter().map(|&(target, _)| target))
        });
        if let Some(old) = old
            && targets.is_some()
            && old.unresolved == unresolved
            && old
                .values
                .iter()
                .copied()
                .eq(pairs.iter().map(|&(_, value)| value))
        {
            return Arc::clone(old);
        }
        Arc::new(Self {
            targets: match targets {
                Some(targets) => Arc::clone(targets),
                None => pairs.iter().map(|&(target, _)| target).collect(),
            },
            values: pairs.iter().map(|&(_, value)| value).collect(),
            unresolved,
        })
    }
}

/// A raw relation owner's rows, in key order.
#[derive(replay_spool::Spooled)]
pub(crate) struct TupleRows {
    /// Each row's spelling; shared while no row's key, endpoints, start or spelling changes.
    pub(crate) identity: Arc<[Option<Arc<str>>]>,
    pub(crate) rows: Box<[RelationTupleRow]>,
}

impl TupleRows {
    /// The bytes it owns, itself and its spellings' slots included.
    fn heap(&self) -> usize {
        size_of::<Self>()
            + self.identity.len() * size_of::<Option<Arc<str>>>()
            + self.rows.len() * size_of::<RelationTupleRow>()
    }

    /// Rows of one owner, sharing what its last rows hold alike.
    fn new(
        rows: Vec<RelationTupleRow>,
        spellings: Vec<Option<Arc<str>>>,
        old: Option<&Arc<Self>>,
    ) -> Arc<Self> {
        let identity = old
            .filter(|old| {
                *old.identity == *spellings
                    && old.rows.len() == rows.len()
                    && old.rows.iter().zip(&rows).all(|(old, row)| {
                        old.key() == row.key()
                            && old.first_country == row.first_country
                            && old.second_country == row.second_country
                            && old.start_hour == row.start_hour
                    })
            })
            .map(|old| &old.identity);
        if let Some(old) = old
            && identity.is_some()
            && *old.rows == *rows
        {
            return Arc::clone(old);
        }
        Arc::new(Self {
            identity: match identity {
                Some(identity) => Arc::clone(identity),
                None => spellings.into(),
            },
            rows: rows.into_boxed_slice(),
        })
    }
}

/// An incoming action and the country index of each of its roles, in [`Proposal::raw`]'s order.
#[derive(replay_spool::Spooled)]
pub(crate) struct Proposal {
    pub(crate) row: IncomingDiplomaticActionRow,
    pub(crate) roles: [Option<u32>; 5],
}

impl Proposal {
    /// Its owner, actor, recipient, original actor and original recipient.
    pub(crate) fn raw(&self) -> [u32; 5] {
        let row = &self.row;
        [
            row.country,
            row.actor,
            row.recipient,
            row.original_actor,
            row.original_recipient,
        ]
    }

    fn new(row: IncomingDiplomaticActionRow, ids: &Ids) -> Self {
        let mut proposal = Self {
            row,
            roles: [None; 5],
        };
        proposal.roles = proposal.raw().map(|raw| ids.index_of(raw));
        proposal
    }
}

/// The incoming queue, by row key, with each owner's handles and how many repeat one.
#[derive(Clone, Default)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Queue {
    pub(crate) rows: BTreeMap<u128, Arc<Proposal>>,
    pub(crate) handles: BTreeMap<(u32, u64), usize>,
    pub(crate) duplicates: usize,
}

impl Queue {
    /// The bytes it owns, itself and its proposals included.
    fn heap(&self) -> usize {
        size_of::<Self>()
            + self.rows.len() * (size_of::<(u128, Arc<Proposal>)>() + size_of::<Proposal>())
            + self.handles.len() * size_of::<((u32, u64), usize)>()
    }

    fn set(&mut self, key: u128, proposal: Option<Arc<Proposal>>) {
        if let Some(old) = self.rows.remove(&key) {
            let handle = (old.row.country, old.row.handle);
            let count = self.handles.get_mut(&handle).expect("queued handle");
            self.duplicates -= usize::from(*count > 1);
            *count -= 1;
            if *count == 0 {
                self.handles.remove(&handle);
            }
        }
        if let Some(proposal) = proposal {
            let count = self
                .handles
                .entry((proposal.row.country, proposal.row.handle))
                .or_default();
            self.duplicates += usize::from(*count > 0);
            *count += 1;
            self.rows.insert(key, proposal);
        }
    }
}

/// A selected opinion owner's index, supplying read and values.
pub(crate) type Opinion = (u32, Option<i32>, Arc<Values>);
/// A selected raw relation owner, its supplying read and rows.
pub(crate) type TupleOwner = (u32, i32, Arc<TupleRows>);

/// The hour's opinion reads: the owners read now, and those only a baseline reads.
#[derive(replay_spool::Spooled)]
pub(crate) struct Opinions {
    pub(crate) current: Vec<Opinion>,
    pub(crate) baseline: Vec<Opinion>,
    /// The rows of owners no country resolves.
    pub(crate) orphans: usize,
}

/// The hour's relation reads: the countries read and their supplying reads, and the raw owners
/// with their rows, now and for a baseline.
#[derive(replay_spool::Spooled)]
pub(crate) struct Tuples {
    pub(crate) countries: Vec<(u32, i32)>,
    pub(crate) baseline_countries: Vec<(u32, i32)>,
    pub(crate) owners: Vec<TupleOwner>,
    pub(crate) baseline_owners: Vec<TupleOwner>,
}

#[derive(replay_spool::Spooled)]
pub(crate) enum QueueRead {
    Unavailable,
    Carried,
    Fresh(Arc<Queue>),
}

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// The identities when the hour read them again; otherwise the hour before's hold.
    pub(crate) ids: Option<Arc<Ids>>,
    /// No identities resolve.
    pub(crate) refused: bool,
    /// Whether the ready tables carry the owners only a baseline reads.
    pub(crate) baseline: bool,
    pub(crate) opinions: Option<Opinions>,
    pub(crate) tuples: Option<Tuples>,
    pub(crate) queue: QueueRead,
    /// The bytes of the identities, values, rows and queues made since an hour last carried
    /// them: the first to carry them counts them.
    shared: usize,
}

/// `current` and `added`, whose indexes are not in `current`, in index order.
pub(crate) fn merged<T>(
    mut current: Vec<T>,
    added: impl IntoIterator<Item = T>,
    index: impl Fn(&T) -> u32,
) -> Vec<T> {
    current.extend(added);
    current.sort_unstable_by_key(index);
    current
}

/// Notes `owner` once for a run of slots it holds, as an owner's rows mostly are.
fn touch(touched: &mut Vec<u32>, owner: u32) {
    if touched.last() != Some(&owner) {
        touched.push(owner);
    }
}

/// A table's live slots by the owner their keys name, each owner's in key order.
struct Groups {
    owner: fn(u128) -> u32,
    complete: bool,
    slots: HashMap<u32, Vec<u32>>,
    /// The owners whose rows the last hour may have changed, ascending.
    touched: Vec<u32>,
}

impl Groups {
    fn new(owner: fn(u128) -> u32) -> Self {
        Self {
            owner,
            complete: false,
            slots: HashMap::new(),
            touched: Vec::new(),
        }
    }

    /// Follows the hour's changes to `table`; empty while it is not complete.
    fn update<W>(&mut self, hour: &segments::Hour<'_, W>, table: TableId) -> io::Result<()> {
        self.touched.clear();
        let complete = columns::complete(hour.state, table);
        let whole = hour.starts || hour.opens || !self.complete;
        self.complete = complete;
        if whole || !complete {
            self.touched
                .extend(self.slots.drain().map(|(owner, _)| owner));
        }
        if complete {
            let held = columns::table(hour.state, table)?;
            if whole {
                for slot in held.live().ones() {
                    if let Some(key) = held.key(slot) {
                        self.slots.entry((self.owner)(key)).or_default().push(slot);
                    }
                }
                for (&owner, slots) in &mut self.slots {
                    slots.sort_unstable_by_key(|&slot| held.key(slot));
                    self.touched.push(owner);
                }
            } else {
                let edits = columns::changes(hour.state, table)?;
                for (&slot, &key) in edits.deleted.iter().zip(&edits.deleted_keys) {
                    let owner = (self.owner)(key);
                    if let Some(slots) = self.slots.get_mut(&owner) {
                        slots.retain(|&other| other != slot);
                        if slots.is_empty() {
                            self.slots.remove(&owner);
                        }
                    }
                    touch(&mut self.touched, owner);
                }
                for &slot in &edits.inserted {
                    if let Some(key) = held.key(slot) {
                        let owner = (self.owner)(key);
                        let slots = self.slots.entry(owner).or_default();
                        let at = slots.partition_point(|&other| held.key(other) < Some(key));
                        slots.insert(at, slot);
                        touch(&mut self.touched, owner);
                    }
                }
                for &slot in edits.changed.iter().chain(&edits.fill_slots) {
                    if let Some(key) = held.key(slot) {
                        touch(&mut self.touched, (self.owner)(key));
                    }
                }
            }
        }
        self.touched.sort_unstable();
        self.touched.dedup();
        Ok(())
    }

    fn owner(&self, owner: u32) -> &[u32] {
        self.slots
            .get(&owner)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn owners(&self) -> impl Iterator<Item = (u32, usize)> + '_ {
        self.slots
            .iter()
            .map(|(&owner, slots)| (owner, slots.len()))
    }
}

const STAMPED: [TableId; 2] = [TableId::RelationOpinions, TableId::RelationTuples];

fn stamped(table: u32) -> Option<usize> {
    STAMPED.iter().position(|kept| kept.index() as u32 == table)
}

/// SliceReads' stamps of `STAMPED`, by owner and by read hour.
#[derive(Default)]
struct Stamps {
    complete: bool,
    reads: [HashMap<u32, i32>; 2],
    hours: [BTreeMap<i32, BTreeSet<u32>>; 2],
}

impl Stamps {
    fn update<W>(&mut self, hour: &segments::Hour<'_, W>, field: usize) -> io::Result<()> {
        let table = TableId::SliceReads;
        let complete = columns::complete(hour.state, table);
        let whole = hour.starts || hour.opens || !self.complete;
        self.complete = complete;
        if whole || !complete {
            for (reads, hours) in self.reads.iter_mut().zip(&mut self.hours) {
                reads.clear();
                hours.clear();
            }
        }
        if !complete {
            return Ok(());
        }
        let held = columns::table(hour.state, table)?;
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
                _ => Err(unrecorded(table, slot)),
            }
        };
        if whole {
            for slot in held.live().ones() {
                if let Some((at, owner, read)) = decode(slot)? {
                    self.set(at, owner, read);
                }
            }
            return Ok(());
        }
        let edits = columns::changes(hour.state, table)?;
        for &key in &edits.deleted_keys {
            if let Some(at) = stamped((key >> 32) as u32) {
                self.remove(at, key as u32);
            }
        }
        for &slot in edits
            .changed
            .iter()
            .chain(&edits.fill_slots)
            .chain(&edits.inserted)
        {
            if let Some((at, owner, read)) = decode(slot)? {
                self.set(at, owner, read);
            }
        }
        Ok(())
    }

    fn set(&mut self, at: usize, owner: u32, read: i32) {
        if let Some(old) = self.reads[at].insert(owner, read) {
            self.forget(at, owner, old);
        }
        self.hours[at].entry(read).or_default().insert(owner);
    }

    fn remove(&mut self, at: usize, owner: u32) {
        if let Some(old) = self.reads[at].remove(&owner) {
            self.forget(at, owner, old);
        }
    }

    fn forget(&mut self, at: usize, owner: u32, read: i32) {
        if let Some(owners) = self.hours[at].get_mut(&read) {
            owners.remove(&owner);
            if owners.is_empty() {
                self.hours[at].remove(&read);
            }
        }
    }

    fn stamp(&self, table: TableId, owner: u32) -> Option<i32> {
        self.reads[stamped(table.index() as u32)?]
            .get(&owner)
            .copied()
    }

    fn current(&self, table: TableId, hour: i32) -> impl Iterator<Item = u32> + '_ {
        stamped(table.index() as u32)
            .and_then(|at| self.hours[at].get(&hour))
            .into_iter()
            .flatten()
            .copied()
    }
}

struct Fields {
    opinion: usize,
    sample_hour: usize,
}

pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    ids: Option<Arc<Ids>>,
    opinions: Groups,
    tuples: Groups,
    stamps: Stamps,
    /// The last values by country, and whether its rows are unchanged since.
    opinion_values: HashMap<u32, (Arc<Values>, bool)>,
    tuple_values: HashMap<u32, (Arc<TupleRows>, bool)>,
    spellings: HashMap<u32, Option<Arc<str>>>,
    orphan_counts: HashMap<u32, usize>,
    orphan_opinions: usize,
    orphan_tuples: BTreeSet<u32>,
    orphan_slices: BTreeMap<u32, BTreeSet<u32>>,
    queue: Option<Arc<Queue>>,
    /// The queue changed since an hour last carried it.
    queue_changed: bool,
    /// The bytes of the shared values made since an hour last carried them.
    charged: usize,
    /// Whether the emit may admit a baseline at the next hour.
    baseline: bool,
}

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            fields: Fields {
                opinion: columns::field(TableId::RelationOpinions, "opinion")?,
                sample_hour: columns::field(TableId::SliceReads, "sample_hour")?,
            },
            shapes: [None; TABLE_COUNT],
            ids: None,
            opinions: Groups::new(|key| (key >> 32) as u32),
            tuples: Groups::new(|key| (key >> 80) as u32),
            stamps: Stamps::default(),
            opinion_values: HashMap::new(),
            tuple_values: HashMap::new(),
            spellings: HashMap::new(),
            orphan_counts: HashMap::new(),
            orphan_opinions: 0,
            orphan_tuples: BTreeSet::new(),
            orphan_slices: BTreeMap::new(),
            queue: None,
            queue_changed: false,
            charged: 0,
            baseline: true,
        })
    }

    /// Follows the hour's tables; whether the identities may have changed.
    fn tables(&mut self, hour: &segments::Hour<'_, World>) -> io::Result<bool> {
        let state = hour.state;
        let mut changed = [false; TABLE_COUNT];
        let mut reshaped = [false; TABLE_COUNT];
        for &table in TABLES {
            let index = table.index();
            let shape = columns::status(state, table);
            reshaped[index] = self.shapes[index] != Some(shape);
            let edits = columns::changes(state, table)?;
            changed[index] = hour.starts
                || hour.opens
                || reshaped[index]
                || !edits.changed.is_empty()
                || !edits.fill_slots.is_empty()
                || !edits.inserted.is_empty()
                || !edits.deleted.is_empty()
                || edits.truncated;
            self.shapes[index] = Some(shape);
        }
        let naming = hour.starts
            || hour.opens
            || reshaped[TableId::Countries.index()]
            || changed[TableId::CountryLinks.index()]
            || hour.world.countries()?.changed();
        self.opinions.update(hour, TableId::RelationOpinions)?;
        self.tuples.update(hour, TableId::RelationTuples)?;
        self.stamps.update(hour, self.fields.sample_hour)?;
        if changed[TableId::LexicalTokens.index()] {
            self.spellings.clear();
            self.tuple_values.clear();
        }
        for raw in &self.tuples.touched {
            if let Some(rows) = self.tuple_values.get_mut(raw) {
                rows.1 = false;
            }
        }
        Ok(naming)
    }

    fn orphans(&mut self, ids: &Ids, naming: bool) {
        if naming {
            self.orphan_counts = self
                .opinions
                .owners()
                .filter(|&(raw, _)| ids.index_of(raw).is_none())
                .collect();
            self.orphan_opinions = self.orphan_counts.values().sum();
            let orphans: BTreeSet<u32> = self
                .tuples
                .owners()
                .map(|(raw, _)| raw)
                .filter(|&raw| ids.index_of(raw).is_none())
                .collect();
            if orphans != self.orphan_tuples {
                self.orphan_slices.clear();
                for &raw in &orphans {
                    self.orphan_slices
                        .entry(cadence::slice_of(raw))
                        .or_default()
                        .insert(raw);
                }
                self.orphan_tuples = orphans;
            }
            return;
        }
        for &raw in &self.opinions.touched {
            self.orphan_opinions -= self.orphan_counts.remove(&raw).unwrap_or(0);
            let count = self.opinions.owner(raw).len();
            if count > 0 && ids.index_of(raw).is_none() {
                self.orphan_counts.insert(raw, count);
                self.orphan_opinions += count;
            }
        }
        for &raw in &self.tuples.touched {
            let slice = cadence::slice_of(raw);
            if ids.index_of(raw).is_none() && !self.tuples.owner(raw).is_empty() {
                if self.orphan_tuples.insert(raw) {
                    self.orphan_slices.entry(slice).or_default().insert(raw);
                }
            } else if self.orphan_tuples.remove(&raw)
                && let Some(owners) = self.orphan_slices.get_mut(&slice)
            {
                owners.remove(&raw);
            }
        }
    }

    /// The supplying read of `raw`'s rows of `table` at `hour`: a stamp from the cadence period
    /// before, else the canonical owner's own scheduled slice.
    fn read_at(&self, ids: &Ids, table: TableId, raw: u32, hour: i32, full: bool) -> Option<i32> {
        if full {
            return Some(hour);
        }
        if let Some(read) = self.stamps.stamp(table, raw) {
            return (read <= hour
                && i64::from(hour) - i64::from(read) < i64::from(cadence::STAGGER_PERIOD))
            .then_some(read);
        }
        (ids.canonical.binary_search(&raw).is_ok() && cadence::slice_of(raw) == slice(hour))
            .then_some(hour)
    }

    fn read(&self, ids: &Ids, table: TableId, country: u32, hour: i32, full: bool) -> Option<i32> {
        std::iter::once(ids.countries[country as usize].country_id)
            .chain(ids.aliases[country as usize].iter().copied())
            .filter_map(|raw| self.read_at(ids, table, raw, hour, full))
            .max()
    }

    fn selection(
        &self,
        ids: &Ids,
        table: TableId,
        hour: i32,
        full: bool,
    ) -> Vec<(u32, Option<i32>)> {
        let mut selected: Vec<u32> = if full {
            (0..ids.countries.len() as u32).collect()
        } else {
            ids.slices[slice(hour) as usize]
                .iter()
                .copied()
                .filter(|&country| {
                    let id = ids.countries[country as usize].country_id;
                    self.read_at(ids, table, id, hour, full) == Some(hour)
                })
                .collect()
        };
        selected.extend(
            self.stamps
                .current(table, hour)
                .filter_map(|raw| ids.index_of(raw)),
        );
        selected.sort_unstable();
        selected.dedup();
        selected
            .into_iter()
            .map(|country| (country, self.read(ids, table, country, hour, full)))
            .collect()
    }

    fn baseline_selection(
        &self,
        ids: &Ids,
        table: TableId,
        hour: i32,
        full: bool,
        current: &[(u32, Option<i32>)],
    ) -> Vec<(u32, Option<i32>)> {
        (0..ids.countries.len() as u32)
            .filter(|&country| {
                let id = ids.countries[country as usize].country_id;
                current
                    .binary_search_by_key(&country, |read| read.0)
                    .is_err()
                    && self.read_at(ids, table, id, hour, full).is_some()
            })
            .map(|country| (country, self.read(ids, table, country, hour, full)))
            .collect()
    }

    fn opinion(
        &mut self,
        state: &State,
        ids: &Ids,
        selected: Vec<(u32, Option<i32>)>,
    ) -> io::Result<Vec<Opinion>> {
        if selected.is_empty() {
            return Ok(Vec::new());
        }
        let field = self.fields.opinion;
        let held = columns::table(state, TableId::RelationOpinions)?;
        let cells = columns::plain::<i32>(held, field)?;
        let mut projected = Vec::with_capacity(selected.len());
        for (country, read) in selected {
            let old = match self.opinion_values.get(&country) {
                Some((values, true)) => {
                    projected.push((country, read, Arc::clone(values)));
                    continue;
                }
                Some((values, false)) => Some(Arc::clone(values)),
                None => None,
            };
            let rows = ids.aliases[country as usize]
                .iter()
                .flat_map(|&raw| self.opinions.owner(raw))
                .map(|&slot| -> io::Result<(u32, i32)> {
                    match (held.key(slot), cells.get(slot as usize)) {
                        (Some(key), Some(&opinion)) if held.recorded(field, slot) => {
                            Ok((key as u32, opinion))
                        }
                        _ => Err(unrecorded(TableId::RelationOpinions, slot)),
                    }
                });
            let (pairs, unresolved) = opinions::project(rows, |raw| ids.index_of(raw))?;
            let values = Values::new(pairs, unresolved, old.as_ref());
            if old.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &values)) {
                self.charged += values.heap();
            }
            self.opinion_values
                .insert(country, (Arc::clone(&values), true));
            projected.push((country, read, values));
        }
        Ok(projected)
    }

    fn spelling(&mut self, state: &State, token: u32) -> io::Result<Option<Arc<str>>> {
        if let Some(spelling) = self.spellings.get(&token) {
            return Ok(spelling.clone());
        }
        let spelling: Option<Arc<str>> = if columns::complete(state, TableId::LexicalTokens)
            && let Some(slot) =
                columns::table(state, TableId::LexicalTokens)?.slot(u128::from(token))
        {
            let row: LexicalTokenRow =
                columns::row(state, TableId::LexicalTokens, slot, self.format.version())?;
            std::str::from_utf8(&row.literal).ok().map(Arc::from)
        } else {
            None
        };
        self.spellings.insert(token, spelling.clone());
        Ok(spelling)
    }

    fn tuple_rows(&mut self, state: &State, raw: u32) -> io::Result<Arc<TupleRows>> {
        let old = match self.tuple_values.get(&raw) {
            Some((rows, true)) => return Ok(Arc::clone(rows)),
            Some((rows, false)) => Some(Arc::clone(rows)),
            None => None,
        };
        let format = self.format.version();
        let rows = self
            .tuples
            .owner(raw)
            .iter()
            .map(|&slot| columns::row(state, TableId::RelationTuples, slot, format))
            .collect::<io::Result<Vec<RelationTupleRow>>>()?;
        let mut spellings = Vec::with_capacity(rows.len());
        for row in &rows {
            spellings.push(self.spelling(state, row.type_id)?);
        }
        let rows = TupleRows::new(rows, spellings, old.as_ref());
        if old.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &rows)) {
            self.charged += rows.heap();
        }
        self.tuple_values.insert(raw, (Arc::clone(&rows), true));
        Ok(rows)
    }

    /// For a baseline, every orphan with a proven read and none of `baseline`'s owners.
    fn tuple_owners(
        &mut self,
        state: &State,
        ids: &Ids,
        countries: &[(u32, i32)],
        hour: i32,
        full: bool,
        baseline: Option<&[TupleOwner]>,
    ) -> io::Result<Vec<TupleOwner>> {
        let mut selected = BTreeMap::new();
        for &(country, read) in countries {
            for &raw in ids.aliases[country as usize].iter() {
                selected.insert(raw, read);
            }
        }
        if full || baseline.is_some() {
            for &raw in &self.orphan_tuples {
                if let Some(read) = self.read_at(ids, TableId::RelationTuples, raw, hour, full) {
                    selected.insert(raw, read);
                }
            }
        } else {
            for raw in self.stamps.current(TableId::RelationTuples, hour) {
                if self.orphan_tuples.contains(&raw) {
                    selected.insert(raw, hour);
                }
            }
            // Unresolved canonical rows can still prove their own rowless scheduled read.
            for &raw in self.orphan_slices.get(&slice(hour)).into_iter().flatten() {
                if self.read_at(ids, TableId::RelationTuples, raw, hour, full) == Some(hour) {
                    selected.insert(raw, hour);
                }
            }
        }
        let current = baseline.unwrap_or_default();
        selected
            .into_iter()
            .filter(|(raw, _)| current.binary_search_by_key(raw, |owner| owner.0).is_err())
            .map(|(raw, read)| Ok((raw, read, self.tuple_rows(state, raw)?)))
            .collect()
    }

    /// The queue's census with `ids`' roles; none while the table is not complete.
    fn census(
        &mut self,
        hour: &segments::Hour<'_, World>,
        ids: &Ids,
        naming: bool,
    ) -> io::Result<()> {
        let table = TableId::IncomingDiplomaticActions;
        let state = hour.state;
        if !columns::complete(state, table) {
            self.queue = None;
            return Ok(());
        }
        let held = columns::table(state, table)?;
        let format = self.format.version();
        let project = |slot: u32| -> io::Result<(u128, Arc<Proposal>)> {
            let row: IncomingDiplomaticActionRow = columns::row(state, table, slot, format)?;
            Ok((row.key(), Arc::new(Proposal::new(row, ids))))
        };
        let follow = !(naming || hour.starts || hour.opens);
        if let Some(queue) = self.queue.as_mut().filter(|_| follow) {
            let edits = columns::changes(state, table)?;
            if edits.deleted.is_empty()
                && edits.changed.is_empty()
                && edits.fill_slots.is_empty()
                && edits.inserted.is_empty()
            {
                return Ok(());
            }
            self.queue_changed = true;
            let queue = Arc::make_mut(queue);
            for &key in &edits.deleted_keys {
                queue.set(key, None);
            }
            for &slot in edits
                .changed
                .iter()
                .chain(&edits.fill_slots)
                .chain(&edits.inserted)
            {
                if held.live().get(slot) {
                    let (key, proposal) = project(slot)?;
                    queue.set(key, Some(proposal));
                }
            }
        } else {
            let mut queue = Queue::default();
            for slot in held.live().ones() {
                let (key, proposal) = project(slot)?;
                queue.set(key, Some(proposal));
            }
            self.queue = Some(Arc::new(queue));
            self.queue_changed = true;
        }
        Ok(())
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, start: Start<'_>, _: u64, _: i32) -> io::Result<()> {
        // The emit admits a baseline only at its branch's first hour and after a refused one.
        self.baseline = match start {
            Start::Restart(state) => {
                !(columns::complete(state, TableId::Countries)
                    && columns::complete(state, TableId::CountryLinks))
            }
            Start::Run { .. } => true,
        };
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let state = hour.state;
        let naming = {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.tables"));
            self.tables(hour)?
        };
        if naming {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.identities"));
            let frame = hour.world.identity(state)?;
            let countries = hour.world.countries()?;
            self.ids = Joins::read(&frame).map(|joins| {
                let canonical: Vec<u32> = if countries.countries_complete() {
                    countries.iter().map(|country| country.id).collect()
                } else {
                    Vec::new()
                };
                let ids = Arc::new(Ids::new(joins, canonical));
                self.charged += ids.heap();
                ids
            });
            self.opinion_values.clear();
        }
        let Some(ids) = self.ids.clone() else {
            self.baseline = true;
            self.queue = None;
            return out.push(Hour {
                branch: hour.branch,
                hour: hour.hour,
                ids: None,
                refused: true,
                baseline: false,
                opinions: None,
                tuples: None,
                queue: QueueRead::Unavailable,
                shared: std::mem::take(&mut self.charged),
            });
        };
        for raw in &self.opinions.touched {
            if let Some(country) = ids.index_of(*raw)
                && let Some(values) = self.opinion_values.get_mut(&country)
            {
                values.1 = false;
            }
        }
        self.orphans(&ids, naming);
        let metrics = metrics(hour.manifest);
        let full = evidence::full_refresh_at(hour.hour, &metrics, self.format);
        let ready = |table| {
            columns::complete(state, table)
                && (full || columns::complete(state, TableId::SliceReads))
        };
        let baseline = std::mem::take(&mut self.baseline);
        let opinions = if ready(TableId::RelationOpinions) {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.opinions"));
            let table = TableId::RelationOpinions;
            let current = self.selection(&ids, table, hour.hour, full);
            let added = if baseline {
                self.baseline_selection(&ids, table, hour.hour, full, &current)
            } else {
                Vec::new()
            };
            Some(Opinions {
                current: self.opinion(state, &ids, current)?,
                baseline: self.opinion(state, &ids, added)?,
                orphans: self.orphan_opinions,
            })
        } else {
            None
        };
        let tuples = if ready(TableId::RelationTuples) {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.tuples"));
            let table = TableId::RelationTuples;
            let current = self.selection(&ids, table, hour.hour, full);
            let added = if baseline {
                self.baseline_selection(&ids, table, hour.hour, full, &current)
            } else {
                Vec::new()
            };
            let read = |(country, read): (u32, Option<i32>)| read.map(|read| (country, read));
            let countries: Vec<_> = current.into_iter().filter_map(read).collect();
            let baseline_countries: Vec<_> = added.into_iter().filter_map(read).collect();
            let owners = self.tuple_owners(state, &ids, &countries, hour.hour, full, None)?;
            let baseline_owners = if baseline {
                self.tuple_owners(
                    state,
                    &ids,
                    &baseline_countries,
                    hour.hour,
                    full,
                    Some(owners.as_slice()),
                )?
            } else {
                Vec::new()
            };
            Some(Tuples {
                countries,
                baseline_countries,
                owners,
                baseline_owners,
            })
        } else {
            None
        };
        let queue = {
            let _cpu = replay_frame::cpu::scope("project", Some("diplomacy_state.queue"));
            self.census(hour, &ids, naming)?;
            match &self.queue {
                None => QueueRead::Unavailable,
                Some(queue)
                    if slot_read(
                        self.format,
                        &metrics,
                        TableId::IncomingDiplomaticActions,
                        hour.hour,
                    ) =>
                {
                    if std::mem::take(&mut self.queue_changed) {
                        self.charged += queue.heap();
                    }
                    QueueRead::Fresh(Arc::clone(queue))
                }
                Some(_) => QueueRead::Carried,
            }
        };
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            ids: naming.then_some(ids),
            refused: false,
            baseline,
            opinions,
            tuples,
            queue,
            shared: std::mem::take(&mut self.charged),
        })
    }
}
