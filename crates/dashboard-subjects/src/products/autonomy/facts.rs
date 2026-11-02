//! Subject autonomy facts, projected hour by hour from a columnar walk ([`replay::segments`]):
//! each subject tag's sample (its holder, overlord, level and scores) where the hour changed it,
//! whether the hour is recorded (every table read complete), whether a subject or overlord the
//! recording does not name holds then, and how many subjects hold more than one relation.
//!
//! The Autonomy and Subjects rows are kept by slot from the hour's changes; a row is read again
//! only where the hour wrote a field the sample reads, and a score or ratio alone is read from
//! its column. A subject is sampled again where its row, its relation or the countries changed,
//! or a relation reached its recorded end. A segment's first hour, and the first after a table
//! was not complete, sends every sample whole: what came before is the emit's.

use crate::products::subjects::holds;
use dashboard_world::{Needs, Roster, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{AutonomyRow, SubjectRow, TableId},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Autonomy,
    TableId::Subjects,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// A subject's recorded autonomy at an hour: the native country holding its tag (canonical slot
/// and id), its overlord's tag where one relation alone names it and the countries name the
/// overlord, its level's id and scores, whether no level lies below or above, and the relation's
/// ratio.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    pub holder: (u32, u32),
    pub overlord: Option<Arc<str>>,
    pub level: Arc<[u8]>,
    pub score: i64,
    pub lower: i64,
    pub upper: i64,
    pub lowest: bool,
    pub highest: bool,
    pub ratio: Option<i64>,
}

impl Sample {
    fn heap(&self) -> usize {
        size_of::<Self>()
    }
}

/// A tag's sample from this hour on, none where no row names it any more; `order` is the lowest
/// raw subject id of its rows, the order the row scan first sees tags and levels in.
pub struct Change {
    pub tag: Arc<str>,
    pub order: u32,
    pub sample: Option<Arc<Sample>>,
    /// Every row's raw subject id and sample, ascending (`sample` is the last's): each row's
    /// level, move-down score and overlord count, in id order across tags, as the row scan
    /// observes every row.
    pub rows: Vec<(u32, Arc<Sample>)>,
}

/// What the ordered emit reads of an hour that changed something.
pub struct Facts {
    /// Whether every table read is complete, where that changed.
    pub recorded: Option<bool>,
    /// `changes` holds every sampled tag: a tag it does not name holds none.
    pub whole: bool,
    /// By `order`.
    pub changes: Vec<Change>,
    /// Whether a holding relation or an Autonomy row names a country the recording does not,
    /// and the subjects more than one relation holds, where they changed.
    pub unidentified: Option<bool>,
    pub shared: Option<u32>,
}

/// One hour of a segment: no facts where it changed nothing the samples read.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub facts: Option<Box<Facts>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.facts.as_ref().map_or(0, |facts| {
            size_of::<Facts>()
                + facts.changes.capacity() * size_of::<Change>()
                + facts
                    .changes
                    .iter()
                    .flat_map(|change| &change.rows)
                    .map(|(_, sample)| sample.heap())
                    .sum::<usize>()
                + facts
                    .changes
                    .iter()
                    .map(|change| change.rows.capacity() * size_of::<(u32, Arc<Sample>)>())
                    .sum::<usize>()
        })
    }
}

/// An Autonomy row: its subject's raw id, scores, level and whether a level lies below or above.
#[derive(Clone)]
struct Level {
    subject: u32,
    score: i64,
    lower: i64,
    upper: i64,
    state: Arc<[u8]>,
    lowest: bool,
    highest: bool,
}

/// A Subjects row.
#[derive(Clone, Copy)]
struct Relation {
    overlord: u32,
    subject: u32,
    start: i32,
    end: i32,
    cancel: u8,
    ratio: i64,
}

/// The relations holding a subject's canonical slot: how many, and the last one's overlord slot,
/// ratio and Subjects slot.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Held {
    count: u32,
    overlord: u32,
    ratio: i64,
    row: u32,
}

/// An Autonomy row's sample, where the links and countries name its subject.
struct Resolved {
    tag: Arc<str>,
    sample: Arc<Sample>,
}

/// The columns a projection reads, and their change-mask bits.
struct Fields {
    score: usize,
    lower: usize,
    upper: usize,
    ratio: usize,
    /// Every Autonomy field a sample reads, and those read from the row alone.
    levels: u64,
    whole_level: u64,
    /// Every Subjects field a relation reads, and those changing which relations hold.
    relations: u64,
    whole_relation: u64,
}

fn bit(field: usize) -> u64 {
    u32::try_from(field)
        .ok()
        .and_then(|field| 1_u64.checked_shl(field))
        .unwrap_or(0)
}

fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    names
        .iter()
        .try_fold(0, |mask, name| Ok(mask | bit(columns::field(table, name)?)))
}

/// The slots of `table` the hour wrote any of `fields` at, with what it wrote, ascending.
fn written(state: &State, table: TableId, fields: u64) -> io::Result<Vec<(u32, u64)>> {
    let changes = columns::changes(state, table)?;
    let mut slots: Vec<(u32, u64)> = changes
        .changed
        .iter()
        .zip(&changes.written)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &mask)| mask & fields != 0)
        .map(|(&slot, &mask)| (slot, mask & fields))
        .collect();
    slots.sort_unstable_by_key(|&(slot, _)| slot);
    slots.dedup_by(|next, held| {
        if next.0 != held.0 {
            return false;
        }
        held.1 |= next.1;
        true
    });
    Ok(slots)
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    /// Nothing is held: the next hour reads every table whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    /// Whether every table was complete the hour before.
    complete: bool,
    /// Each Autonomy slot's row, and each subject raw id's slot.
    levels: Vec<Option<Level>>,
    by_subject: HashMap<u32, u32>,
    relations: Vec<Option<Relation>>,
    /// The holding relations by subject canonical slot, the subjects more than one holds,
    /// whether a holding one names a country the recording does not, and the first recorded end
    /// still ahead.
    held: HashMap<u32, Held>,
    shared: u32,
    unnamed_relation: bool,
    expires: Option<i32>,
    /// Each resolved country's id and tag, by canonical slot: the last by id the links resolve.
    countries: HashMap<u32, (u32, Arc<str>)>,
    /// Level ids, read once a segment.
    spellings: HashMap<Box<[u8]>, Arc<[u8]>>,
    /// Each Autonomy row's sample by subject raw id, and the rows naming no country.
    resolved: HashMap<u32, Resolved>,
    unresolved: HashSet<u32>,
    /// Each tag's rows' subject raw ids, ascending, and the samples sent for them.
    tags: HashMap<Arc<str>, Vec<u32>>,
    sent: HashMap<Arc<str>, Vec<(u32, Arc<Sample>)>>,
    /// The last flags sent.
    unidentified: Option<bool>,
    sent_shared: Option<u32>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since the next hour reads
/// every table again, as at a segment's first hour.
const SAVED: &[u8] = b"autonomy projector: cold";

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        let autonomy = TableId::Autonomy;
        let subjects = TableId::Subjects;
        Ok(Self {
            format,
            fields: Fields {
                score: columns::field(autonomy, "score")?,
                lower: columns::field(autonomy, "lower_score")?,
                upper: columns::field(autonomy, "upper_score")?,
                ratio: columns::field(subjects, "autonomy_ratio")?,
                levels: mask(
                    autonomy,
                    &["score", "lower_score", "upper_score", "state", "previous", "next"],
                )?,
                whole_level: mask(autonomy, &["state", "previous", "next"])?,
                relations: mask(
                    subjects,
                    &["start_hour", "end_hour", "cancel", "autonomy_ratio"],
                )?,
                whole_relation: mask(subjects, &["start_hour", "end_hour", "cancel"])?,
            },
            cold: true,
            last: None,
            complete: false,
            levels: Vec::new(),
            by_subject: HashMap::new(),
            relations: Vec::new(),
            held: HashMap::new(),
            shared: 0,
            unnamed_relation: false,
            expires: None,
            countries: HashMap::new(),
            spellings: HashMap::new(),
            resolved: HashMap::new(),
            unresolved: HashSet::new(),
            tags: HashMap::new(),
            sent: HashMap::new(),
            unidentified: None,
            sent_shared: None,
        })
    }

    /// Forgets every row and sample: the next complete hour reads every table whole.
    fn forget(&mut self) {
        self.levels.clear();
        self.by_subject.clear();
        self.relations.clear();
        self.held.clear();
        self.shared = 0;
        self.unnamed_relation = false;
        self.expires = None;
        self.countries.clear();
        self.resolved.clear();
        self.unresolved.clear();
        self.tags.clear();
        self.sent.clear();
        self.unidentified = None;
        self.sent_shared = None;
    }

    fn spelling(&mut self, bytes: Vec<u8>) -> Arc<[u8]> {
        if let Some(held) = self.spellings.get(bytes.as_slice()) {
            return Arc::clone(held);
        }
        let spelled: Arc<[u8]> = Arc::from(bytes.as_slice());
        self.spellings
            .insert(bytes.into_boxed_slice(), Arc::clone(&spelled));
        spelled
    }

    /// The Autonomy row at `slot`, built from its columns.
    fn level_row(&mut self, state: &State, slot: u32) -> io::Result<Level> {
        let row: AutonomyRow =
            columns::row(state, TableId::Autonomy, slot, self.format.version())?;
        Ok(Level {
            subject: row.subject,
            score: row.score,
            lower: row.lower_score,
            upper: row.upper_score,
            state: self.spelling(row.state),
            lowest: row.previous.is_none(),
            highest: row.next.is_none(),
        })
    }

    /// `field`'s value at `slot`, from its column where the recording recorded it.
    fn plain<R: replay_data::rows::RowCodec>(
        &self,
        state: &State,
        table: TableId,
        field: usize,
        slot: u32,
        from_row: impl FnOnce(R) -> i64,
    ) -> io::Result<i64> {
        let held = columns::table(state, table)?;
        if !held.recorded(field, slot) {
            return Ok(from_row(columns::row(
                state,
                table,
                slot,
                self.format.version(),
            )?));
        }
        columns::plain::<i64>(held, field)?
            .get(slot as usize)
            .copied()
            .ok_or_else(|| io::Error::other("A live slot holds no cell"))
    }

    fn relation_row(&self, state: &State, slot: u32) -> io::Result<Relation> {
        let row: SubjectRow =
            columns::row(state, TableId::Subjects, slot, self.format.version())?;
        Ok(Relation {
            overlord: row.overlord,
            subject: row.subject,
            start: row.start_hour,
            end: row.end_hour,
            cancel: row.cancel,
            ratio: row.autonomy_ratio,
        })
    }

    fn put_level(&mut self, slot: u32, level: Level) {
        let index = slot as usize;
        if self.levels.len() <= index {
            self.levels.resize(index + 1, None);
        }
        self.by_subject.insert(level.subject, slot);
        self.levels[index] = Some(level);
    }

    fn take_level(&mut self, slot: u32) -> Option<Level> {
        let level = self.levels.get_mut(slot as usize)?.take()?;
        if self.by_subject.get(&level.subject) == Some(&slot) {
            self.by_subject.remove(&level.subject);
        }
        Some(level)
    }

    fn put_relation(&mut self, slot: u32, relation: Relation) {
        let index = slot as usize;
        if self.relations.len() <= index {
            self.relations.resize(index + 1, None);
        }
        self.relations[index] = Some(relation);
    }

    /// Reads every Autonomy row; the subjects whose rows changed are named in `dirty`.
    fn read_levels(
        &mut self,
        state: &State,
        whole: bool,
        dirty: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        let table = TableId::Autonomy;
        if whole {
            dirty.extend(self.by_subject.keys().copied());
            self.levels.clear();
            self.by_subject.clear();
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            for slot in live {
                let level = self.level_row(state, slot)?;
                dirty.insert(level.subject);
                self.put_level(slot, level);
            }
            return Ok(());
        }
        let edits = columns::changes(state, table)?;
        for &slot in &edits.deleted {
            if let Some(level) = self.take_level(slot) {
                dirty.insert(level.subject);
            }
        }
        for &slot in &edits.inserted {
            let level = self.level_row(state, slot)?;
            dirty.insert(level.subject);
            self.put_level(slot, level);
        }
        for (slot, mask) in written(state, table, self.fields.levels)? {
            if edits.inserted.contains(&slot) {
                continue;
            }
            let Some(Some(held)) = self.levels.get(slot as usize) else {
                continue;
            };
            let subject = held.subject;
            if mask & self.fields.whole_level != 0 {
                let level = self.level_row(state, slot)?;
                self.put_level(slot, level);
            } else {
                let fields = &self.fields;
                let read = |field| {
                    self.plain(state, table, field, slot, |row: AutonomyRow| {
                        if field == fields.score {
                            row.score
                        } else if field == fields.lower {
                            row.lower_score
                        } else {
                            row.upper_score
                        }
                    })
                };
                let score = (mask & bit(fields.score) != 0)
                    .then(|| read(fields.score))
                    .transpose()?;
                let lower = (mask & bit(fields.lower) != 0)
                    .then(|| read(fields.lower))
                    .transpose()?;
                let upper = (mask & bit(fields.upper) != 0)
                    .then(|| read(fields.upper))
                    .transpose()?;
                let level = self.levels[slot as usize]
                    .as_mut()
                    .expect("a written Autonomy row is held");
                level.score = score.unwrap_or(level.score);
                level.lower = lower.unwrap_or(level.lower);
                level.upper = upper.unwrap_or(level.upper);
            }
            dirty.insert(subject);
        }
        Ok(())
    }

    /// Reads the Subjects rows; whether which relations hold may have changed, and the ratios
    /// that changed alone, by Subjects slot.
    fn read_relations(&mut self, state: &State, whole: bool) -> io::Result<(bool, Vec<u32>)> {
        let table = TableId::Subjects;
        if whole {
            self.relations.clear();
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            for slot in live {
                let relation = self.relation_row(state, slot)?;
                self.put_relation(slot, relation);
            }
            return Ok((true, Vec::new()));
        }
        let edits = columns::changes(state, table)?;
        let mut structure = !edits.deleted.is_empty() || !edits.inserted.is_empty();
        for &slot in &edits.deleted {
            if let Some(held) = self.relations.get_mut(slot as usize) {
                *held = None;
            }
        }
        for &slot in &edits.inserted {
            let relation = self.relation_row(state, slot)?;
            self.put_relation(slot, relation);
        }
        let mut ratios = Vec::new();
        for (slot, mask) in written(state, table, self.fields.relations)? {
            if edits.inserted.contains(&slot)
                || self.relations.get(slot as usize).is_none_or(Option::is_none)
            {
                continue;
            }
            if mask & self.fields.whole_relation != 0 {
                let relation = self.relation_row(state, slot)?;
                self.put_relation(slot, relation);
                structure = true;
            } else {
                let ratio = self.plain(state, table, self.fields.ratio, slot, |row: SubjectRow| {
                    row.autonomy_ratio
                })?;
                if let Some(Some(relation)) = self.relations.get_mut(slot as usize) {
                    relation.ratio = ratio;
                }
                ratios.push(slot);
            }
        }
        Ok((structure, ratios))
    }

    /// The countries the roster resolves, by canonical slot, as the row scan binds them.
    fn bind_countries(&mut self, roster: &Roster) {
        self.countries.clear();
        for country in roster.iter() {
            if country.resolved {
                let tag = String::from_utf8_lossy(roster.tag(country.tag));
                self.countries
                    .insert(country.slot, (country.id, Arc::from(&*tag)));
            }
        }
    }

    /// The relations holding at `hour`, as the row scan joins them: the subjects whose holding
    /// relations changed are added to `slots`.
    fn bind_relations(&mut self, roster: &Roster, hour: i32, slots: &mut BTreeSet<u32>) {
        let mut held: HashMap<u32, Held> = HashMap::with_capacity(self.held.len());
        let mut unnamed = false;
        let mut expires: Option<i32> = None;
        for (row, relation) in self.relations.iter().enumerate() {
            let Some(relation) = relation else {
                continue;
            };
            if !holds(relation.cancel, relation.start, relation.end, hour) {
                continue;
            }
            if relation.end > relation.start {
                expires = Some(expires.map_or(relation.end, |end| end.min(relation.end)));
            }
            let (Some(overlord), Some(subject)) =
                (roster.slot_of(relation.overlord), roster.slot_of(relation.subject))
            else {
                unnamed = true;
                continue;
            };
            unnamed |=
                !self.countries.contains_key(&overlord) || !self.countries.contains_key(&subject);
            let entry = held.entry(subject).or_insert(Held {
                count: 0,
                overlord,
                ratio: relation.ratio,
                row: row as u32,
            });
            entry.count += 1;
            entry.overlord = overlord;
            entry.ratio = relation.ratio;
            entry.row = row as u32;
        }
        for (subject, now) in &held {
            if self.held.get(subject) != Some(now) {
                slots.insert(*subject);
            }
        }
        for subject in self.held.keys() {
            if !held.contains_key(subject) {
                slots.insert(*subject);
            }
        }
        self.shared = held.values().filter(|held| held.count > 1).count() as u32;
        self.held = held;
        self.unnamed_relation = unnamed;
        self.expires = expires;
    }

    /// The ratios of `rows` changed alone: their subjects' slots are added to `slots` where the
    /// row is the one relation holding them.
    fn rebind_ratios(&mut self, roster: &Roster, rows: &[u32], slots: &mut BTreeSet<u32>) {
        for &row in rows {
            let Some(Some(relation)) = self.relations.get(row as usize) else {
                continue;
            };
            let Some(subject) = roster.slot_of(relation.subject) else {
                continue;
            };
            if let Some(held) = self.held.get_mut(&subject)
                && held.row == row
                && held.ratio != relation.ratio
            {
                held.ratio = relation.ratio;
                if held.count == 1 {
                    slots.insert(subject);
                }
            }
        }
    }

    /// The sample of the Autonomy row of subject raw id `subject`, where the links and countries
    /// name it.
    fn sample(&self, roster: &Roster, level: &Level) -> Option<Resolved> {
        let slot = roster.slot_of(level.subject)?;
        let (id, tag) = self.countries.get(&slot)?;
        let relation = self.held.get(&slot).filter(|held| held.count == 1);
        Some(Resolved {
            tag: Arc::clone(tag),
            sample: Arc::new(Sample {
                holder: (slot, *id),
                overlord: relation
                    .and_then(|held| self.countries.get(&held.overlord))
                    .map(|(_, tag)| Arc::clone(tag)),
                level: Arc::clone(&level.state),
                score: level.score,
                lower: level.lower,
                upper: level.upper,
                lowest: level.lowest,
                highest: level.highest,
                ratio: relation.map(|held| held.ratio),
            }),
        })
    }

    /// Samples the rows of `subjects` (raw ids) again; the tags whose rows changed are added to
    /// `tags`.
    fn resample(
        &mut self,
        roster: &Roster,
        subjects: impl IntoIterator<Item = u32>,
        tags: &mut BTreeSet<Arc<str>>,
    ) {
        for subject in subjects {
            let level = self
                .by_subject
                .get(&subject)
                .and_then(|&slot| self.levels.get(slot as usize)?.as_ref());
            let now = level.and_then(|level| self.sample(roster, level));
            self.unresolved.remove(&subject);
            if level.is_some() && now.is_none() {
                self.unresolved.insert(subject);
            }
            if let Some(was) = self.resolved.remove(&subject) {
                if now.as_ref().is_none_or(|now| now.tag != was.tag) {
                    if let Some(rows) = self.tags.get_mut(&was.tag) {
                        rows.retain(|&held| held != subject);
                        if rows.is_empty() {
                            self.tags.remove(&was.tag);
                        }
                    }
                }
                tags.insert(was.tag);
            }
            if let Some(now) = now {
                let rows = self.tags.entry(Arc::clone(&now.tag)).or_default();
                if let Err(at) = rows.binary_search(&subject) {
                    rows.insert(at, subject);
                }
                tags.insert(Arc::clone(&now.tag));
                self.resolved.insert(subject, now);
            }
        }
    }

    /// Every row's sample, read whole.
    fn resample_all(&mut self, roster: &Roster, tags: &mut BTreeSet<Arc<str>>) {
        tags.extend(self.tags.keys().cloned());
        self.tags.clear();
        self.resolved.clear();
        self.unresolved.clear();
        let mut subjects: Vec<u32> = self.by_subject.keys().copied().collect();
        subjects.sort_unstable();
        for subject in subjects {
            let level = self
                .by_subject
                .get(&subject)
                .and_then(|&slot| self.levels.get(slot as usize)?.as_ref());
            let Some(level) = level else {
                continue;
            };
            match self.sample(roster, level) {
                Some(now) => {
                    self.tags.entry(Arc::clone(&now.tag)).or_default().push(subject);
                    tags.insert(Arc::clone(&now.tag));
                    self.resolved.insert(subject, now);
                }
                None => {
                    self.unresolved.insert(subject);
                }
            }
        }
    }

    /// The subject raw ids whose rows' subjects the links put in canonical `slots`.
    fn rows_at(&self, roster: &Roster, slots: &BTreeSet<u32>) -> Vec<u32> {
        slots
            .iter()
            .flat_map(|&slot| roster.aliases(slot))
            .copied()
            .filter(|raw| self.by_subject.contains_key(raw))
            .collect()
    }

    /// The changes of `tags`' samples since they were last sent: a tag's sample is its row's of
    /// the highest raw id, as the row scan's last observation of the tag is.
    fn changes(&mut self, tags: BTreeSet<Arc<str>>, whole: bool) -> Vec<Change> {
        let mut changes = Vec::new();
        for tag in tags {
            let rows = self.tags.get(&tag);
            let all: Vec<(u32, Arc<Sample>)> = rows
                .into_iter()
                .flatten()
                .filter_map(|&subject| {
                    let resolved = self.resolved.get(&subject)?;
                    Some((subject, Arc::clone(&resolved.sample)))
                })
                .collect();
            let order = rows.and_then(|rows| rows.first()).copied().unwrap_or(u32::MAX);
            let was = self.sent.get(&tag);
            if !whole && was.map_or(all.is_empty(), |was| *was == all) {
                continue;
            }
            if was.is_none() && all.is_empty() {
                continue;
            }
            if all.is_empty() {
                self.sent.remove(&tag);
            } else {
                self.sent.insert(Arc::clone(&tag), all.clone());
            }
            changes.push(Change {
                tag,
                order,
                sample: all.last().map(|(_, sample)| Arc::clone(sample)),
                rows: all,
            });
        }
        changes.sort_by_key(|change| change.order);
        changes
    }
}

// Every segment starts cold: its first hour reads every table again; what came before is the
// emit's.
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
        let _cpu = replay_frame::cpu::scope("autonomy.columnar", Some("autonomy"));
        let state = hour.state;
        let at = hour.hour;
        let world = hour.world;
        let roster = world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        let complete = roster.complete()
            && columns::complete(state, TableId::Autonomy)
            && columns::complete(state, TableId::Subjects);
        if !complete {
            let was = std::mem::replace(&mut self.complete, false);
            if was || opens {
                self.forget();
            }
            out.push(Hour {
                branch: hour.branch,
                hour: at,
                facts: (was || opens).then(|| {
                    Box::new(Facts {
                        recorded: Some(false),
                        whole: true,
                        changes: Vec::new(),
                        unidentified: None,
                        shared: None,
                    })
                }),
            })?;
            return Ok(());
        }
        // A table read whole: a segment's first hour, the first complete one, or a new status.
        let whole = opens
            || !self.complete
            || TABLES.iter().any(|&table| world.restated(table));
        let became = whole && !self.complete;
        self.complete = true;
        let mut tags = BTreeSet::new();
        if whole {
            self.forget();
            let mut ignored = BTreeSet::new();
            self.read_levels(state, true, &mut ignored)?;
            self.read_relations(state, true)?;
            self.bind_countries(roster);
            self.bind_relations(roster, at, &mut ignored);
            self.resample_all(roster, &mut tags);
        } else {
            let mut subjects = BTreeSet::new();
            self.read_levels(state, false, &mut subjects)?;
            let (structure, ratios) = self.read_relations(state, false)?;
            let identity = roster.rebuilt()
                || !roster.changed_ids().is_empty()
                || !roster.changed_links().is_empty();
            let expired = self.expires.is_some_and(|end| at >= end);
            if identity {
                self.bind_countries(roster);
            }
            let mut slots = BTreeSet::new();
            if identity || structure || expired {
                self.bind_relations(roster, at, &mut slots);
            } else if !ratios.is_empty() {
                self.rebind_ratios(roster, &ratios, &mut slots);
            }
            if identity {
                self.resample_all(roster, &mut tags);
            } else {
                subjects.extend(self.rows_at(roster, &slots));
                self.resample(roster, subjects, &mut tags);
            }
        }
        let changes = self.changes(tags, whole);
        let unidentified = self.unnamed_relation || !self.unresolved.is_empty();
        let unidentified =
            (whole || self.unidentified != Some(unidentified)).then_some(unidentified);
        if let Some(flag) = unidentified {
            self.unidentified = Some(flag);
        }
        let shared = (whole || self.sent_shared != Some(self.shared)).then_some(self.shared);
        if let Some(count) = shared {
            self.sent_shared = Some(count);
        }
        let quiet = !whole && changes.is_empty() && unidentified.is_none() && shared.is_none();
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            facts: (!quiet).then(|| {
                Box::new(Facts {
                    recorded: (became || opens).then_some(true),
                    whole,
                    changes,
                    unidentified,
                    shared,
                })
            }),
        })
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved autonomy projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
