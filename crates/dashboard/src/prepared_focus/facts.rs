//! National focus facts, projected hour by hour from a columnar walk ([`replay::segments`]): the
//! four tables' refusals where they change, the countries without an observation by reason
//! where those counts change, and each tag's observation (its country, progress and order)
//! where it changed. A tag held by several countries is observed as its highest country id that
//! has one. An hour that changes nothing the observation reads carries nothing but its hour.
//! What a tag held before the segment is the ordered emit's ([`super::emit`]).

use super::{INCOMPLETE_ORDER, INCOMPLETE_TABLES, NO_FOCUS_ROW, NO_TAG, TableRefusal, UNRESOLVED};
use dashboard_world::{Country, Needs, Roster, Tag, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{CompletedFocusRow, FocusRow, TableId},
    scope::CaptureMode,
};
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads, in the order their refusals are listed.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Focus,
    TableId::CompletedFocuses,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// The reasons a country has no observation, by index.
const REASONS: [&str; 4] = [UNRESOLVED, NO_FOCUS_ROW, NO_TAG, INCOMPLETE_ORDER];

/// A country's focus order as recorded: its completed focuses' tokens in order, its active and
/// continuous focus (none when empty), and whether it is paused. Compared by recorded bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct Order {
    pub completed: Arc<[Arc<[u8]>]>,
    pub current: Option<Arc<[u8]>>,
    pub continuous: Option<Arc<[u8]>>,
    pub paused: bool,
}

/// A tag's focus observation at an hour: its country's canonical slot and id, its raw progress
/// and its order, shared while it is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub slot: u32,
    pub country_id: u32,
    pub progress: i64,
    pub order: Arc<Order>,
}

/// What the ordered emit reads of an hour that changes what the observation reads.
pub struct Facts {
    /// The four tables' refusals, at a segment's first hour and where they changed.
    pub refusals: Option<Vec<TableRefusal>>,
    /// The hour's countries without an observation, by reason, where those counts changed.
    pub tally: Option<Vec<(&'static str, u64)>>,
    /// Whether `tags` lists every tag observed this hour, the others observed by none.
    pub whole: bool,
    /// Each tag whose observation changed, none where it is no longer observed.
    pub tags: Vec<(Arc<str>, Option<Observation>)>,
}

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// None when the hour changes nothing the observation reads.
    pub facts: Option<Box<Facts>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        let Some(facts) = &self.facts else {
            return 0;
        };
        // Orders and tokens are the projector's, shared; an observation charges its list.
        size_of::<Facts>()
            + facts.refusals.as_ref().map_or(0, |refusals| {
                refusals
                    .iter()
                    .map(|refusal| {
                        size_of::<TableRefusal>()
                            + refusal.table.capacity()
                            + refusal.reason.as_ref().map_or(0, String::capacity)
                            + refusal.missing_rows.as_ref().map_or(0, String::capacity)
                    })
                    .sum::<usize>()
            })
            + facts
                .tally
                .as_ref()
                .map_or(0, |tally| tally.capacity() * size_of::<(&str, u64)>())
            + facts.tags.capacity() * size_of::<(Arc<str>, Option<Observation>)>()
            + facts
                .tags
                .iter()
                .map(|(tag, observed)| {
                    tag.len()
                        + observed.as_ref().map_or(0, |observed| {
                            size_of::<Order>()
                                + observed.order.completed.len() * size_of::<Arc<[u8]>>()
                        })
                })
                .sum::<usize>()
    }
}

/// A Focus row as the observation reads it, and its order once built.
struct Focus {
    /// Its raw country (the row's key), and the canonical slot its link names.
    raw: u32,
    holder: Option<u32>,
    progress: i64,
    count: i32,
    current: Option<Arc<[u8]>>,
    continuous: Option<Arc<[u8]>>,
    paused: bool,
    order: Option<Arc<Order>>,
}

/// One raw owner's CompletedFocuses rows by ordinal, ascending, how many hold an empty token,
/// and their tokens in order once listed.
#[derive(Default)]
struct Owned {
    rows: Vec<(u32, Arc<[u8]>)>,
    empty: usize,
    list: Option<Arc<[Arc<[u8]>]>>,
}

impl Owned {
    fn insert(&mut self, ordinal: u32, token: Arc<[u8]>) {
        self.list = None;
        self.empty += usize::from(token.is_empty());
        match self.rows.binary_search_by_key(&ordinal, |&(held, _)| held) {
            Ok(at) => {
                let held = std::mem::replace(&mut self.rows[at].1, token);
                self.empty -= usize::from(held.is_empty());
            }
            Err(at) => self.rows.insert(at, (ordinal, token)),
        }
    }

    fn remove(&mut self, ordinal: u32) {
        if let Ok(at) = self.rows.binary_search_by_key(&ordinal, |&(held, _)| held) {
            let (_, held) = self.rows.remove(at);
            self.empty -= usize::from(held.is_empty());
            self.list = None;
        }
    }

    /// Whether `count` completed focuses are held whole and in order: ordinals from zero, none
    /// missing, none empty.
    fn complete(&self, count: i32) -> bool {
        usize::try_from(count).ok() == Some(self.rows.len())
            && self.empty == 0
            && self
                .rows
                .last()
                .is_none_or(|&(ordinal, _)| ordinal as usize + 1 == self.rows.len())
    }

    fn list(&mut self) -> Arc<[Arc<[u8]>]> {
        if let Some(list) = &self.list {
            return Arc::clone(list);
        }
        let list: Arc<[Arc<[u8]>]> = self.rows.iter().map(|(_, token)| Arc::clone(token)).collect();
        self.list = Some(Arc::clone(&list));
        list
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<(u32, Arc<[u8]>)>()
            + self.rows.iter().map(|(_, token)| token.len()).sum::<usize>()
            + self.list.as_ref().map_or(0, |list| list.len() * size_of::<Arc<[u8]>>())
    }
}

/// The columns the observation reads.
struct Fields {
    current: usize,
    continuous: usize,
    progress: usize,
    paused: usize,
    count: usize,
    token: usize,
}

impl Fields {
    fn bit(field: usize) -> u64 {
        1_u64.checked_shl(field as u32).unwrap_or(0)
    }
}

/// The text the row decoder gives a table that is not complete.
fn refusal(
    state: &State,
    table: TableId,
    status: columns::Status,
    mode: CaptureMode,
) -> io::Result<Option<TableRefusal>> {
    let (reason, missing_rows) = match status {
        columns::Status::Complete => return Ok(None),
        columns::Status::Unavailable { reason } => {
            let reason = state.dictionary().get(reason).ok_or_else(|| {
                io::Error::other(format!("A {table:?} reason names no dictionary entry"))
            })?;
            (Some(String::from_utf8_lossy(reason).into_owned()), None)
        }
        columns::Status::Truncated { missing } => (None, Some(missing.to_string())),
        columns::Status::NotRecorded => (Some(format!("not recorded in a {mode} recording")), None),
    };
    Ok(Some(TableRefusal {
        table: format!("{table:?}"),
        reason,
        missing_rows,
    }))
}

/// Recorded bytes as an order holds them: none when empty.
fn bytes(value: Option<Vec<u8>>) -> Option<Arc<[u8]>> {
    value.filter(|value| !value.is_empty()).map(Arc::from)
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    fields: Fields,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    statuses: [Option<columns::Status>; 4],
    /// The rows are not indexed: the next complete hour reads them whole.
    stale: bool,
    focus: Vec<Option<Focus>>,
    /// Each raw country's Focus slot, and each canonical slot's Focus slots.
    by_raw: HashMap<u32, u32>,
    by_holder: HashMap<u32, Vec<u32>>,
    owned: HashMap<u32, Owned>,
    /// Each tag's text, none where it is empty or not UTF-8.
    names: HashMap<Tag, Option<Arc<str>>>,
    /// Each country without an observation's reason, by id, and how many hold each.
    reasons: HashMap<u32, usize>,
    counts: [u64; 4],
    /// What the emit was last sent: the refusals, the counts, and each observed tag's
    /// observation.
    sent_refusals: Option<Vec<TableRefusal>>,
    sent_tally: Option<[u64; 5]>,
    sent: HashMap<Tag, (Arc<str>, Observation)>,
    /// Canonical slots whose countries' observations may have changed.
    dirty: HashSet<u32>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"focus projector: cold";

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        Ok(Self {
            format,
            mode,
            fields: Fields {
                current: columns::field(TableId::Focus, "current")?,
                continuous: columns::field(TableId::Focus, "continuous")?,
                progress: columns::field(TableId::Focus, "progress")?,
                paused: columns::field(TableId::Focus, "paused")?,
                count: columns::field(TableId::Focus, "completed_count")?,
                token: columns::field(TableId::CompletedFocuses, "token")?,
            },
            cold: true,
            last: None,
            statuses: [None; 4],
            stale: true,
            focus: Vec::new(),
            by_raw: HashMap::new(),
            by_holder: HashMap::new(),
            owned: HashMap::new(),
            names: HashMap::new(),
            reasons: HashMap::new(),
            counts: [0; 4],
            sent_refusals: None,
            sent_tally: None,
            sent: HashMap::new(),
            dirty: HashSet::new(),
        })
    }

    fn clear(&mut self) {
        self.focus.clear();
        self.by_raw.clear();
        self.by_holder.clear();
        self.owned.clear();
        self.reasons.clear();
        self.counts = [0; 4];
        self.dirty.clear();
        self.stale = true;
    }

    /// The Focus row at `slot`, its holder from `roster`.
    fn read_focus(&self, state: &State, roster: &Roster, slot: u32) -> io::Result<Focus> {
        let row: FocusRow =
            columns::row(state, TableId::Focus, slot, self.format.version())?;
        Ok(Focus {
            raw: row.country,
            holder: roster.slot_of(row.country),
            progress: row.progress,
            count: row.completed_count,
            current: bytes(row.current),
            continuous: bytes(row.continuous),
            paused: row.paused != 0,
            order: None,
        })
    }

    /// The fields a write of `mask` to the Focus row at `slot` changed: its progress, pause and
    /// count read in place; its order texts, or an unrecorded cell, from its row.
    fn rewrite_focus(
        &mut self,
        state: &State,
        roster: &Roster,
        slot: u32,
        mask: u64,
    ) -> io::Result<()> {
        let (progress, paused, count) = (self.fields.progress, self.fields.paused, self.fields.count);
        let texts = Fields::bit(self.fields.current) | Fields::bit(self.fields.continuous);
        let plain = Fields::bit(progress) | Fields::bit(paused) | Fields::bit(count);
        if mask & (texts | plain) == 0 {
            return Ok(());
        }
        if self.focus.get(slot as usize).is_none_or(Option::is_none) {
            let focus = self.read_focus(state, roster, slot)?;
            self.insert_focus(slot, focus);
            return Ok(());
        }
        let held = columns::table(state, TableId::Focus)?;
        let recorded = [progress, paused, count]
            .iter()
            .all(|&field| held.recorded(field, slot));
        if mask & texts != 0 || !recorded {
            let mut row = self.read_focus(state, roster, slot)?;
            let Some(focus) = self.focus.get_mut(slot as usize).and_then(Option::as_mut) else {
                return Err(io::Error::other("A changed Focus slot holds no indexed row"));
            };
            let unchanged = focus.current == row.current
                && focus.continuous == row.continuous
                && focus.paused == row.paused;
            if unchanged {
                row.order = focus.order.take();
            }
            row.holder = focus.holder;
            *focus = row;
        } else {
            let at = slot as usize;
            let missing = || io::Error::other("A Focus slot holds no cell");
            let progress = *columns::plain::<i64>(held, progress)?.get(at).ok_or_else(missing)?;
            let paused = *columns::plain::<u8>(held, paused)?.get(at).ok_or_else(missing)? != 0;
            let count = *columns::plain::<i32>(held, count)?.get(at).ok_or_else(missing)?;
            let Some(focus) = self.focus.get_mut(at).and_then(Option::as_mut) else {
                return Err(io::Error::other("A changed Focus slot holds no indexed row"));
            };
            focus.progress = progress;
            focus.count = count;
            if focus.paused != paused {
                focus.paused = paused;
                focus.order = None;
            }
        }
        if let Some(holder) = self.focus[slot as usize].as_ref().and_then(|focus| focus.holder) {
            self.dirty.insert(holder);
        }
        Ok(())
    }

    fn insert_focus(&mut self, slot: u32, focus: Focus) {
        let at = slot as usize;
        if self.focus.len() <= at {
            self.focus.resize_with(at + 1, || None);
        }
        self.by_raw.insert(focus.raw, slot);
        if let Some(holder) = focus.holder {
            self.by_holder.entry(holder).or_default().push(slot);
            self.dirty.insert(holder);
        }
        self.focus[at] = Some(focus);
    }

    fn remove_focus(&mut self, slot: u32) {
        let Some(focus) = self.focus.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        if self.by_raw.get(&focus.raw) == Some(&slot) {
            self.by_raw.remove(&focus.raw);
        }
        if let Some(holder) = focus.holder {
            if let Some(slots) = self.by_holder.get_mut(&holder) {
                slots.retain(|&held| held != slot);
                if slots.is_empty() {
                    self.by_holder.remove(&holder);
                }
            }
            self.dirty.insert(holder);
        }
    }

    /// The token of the CompletedFocuses row at `slot`.
    fn token(&self, state: &State, slot: u32) -> io::Result<Arc<[u8]>> {
        let held = columns::table(state, TableId::CompletedFocuses)?;
        if !held.recorded(self.fields.token, slot) {
            let row: CompletedFocusRow =
                columns::row(state, TableId::CompletedFocuses, slot, self.format.version())?;
            return Ok(Arc::from(row.token));
        }
        let text = *columns::text(held, self.fields.token)?
            .get(slot as usize)
            .ok_or_else(|| io::Error::other("A CompletedFocuses slot holds no cell"))?;
        let token = state
            .dictionary()
            .get(text)
            .ok_or_else(|| io::Error::other("A completed focus names no dictionary entry"))?;
        Ok(Arc::from(token))
    }

    /// The raw owner and ordinal of the CompletedFocuses row at `slot`.
    fn completed_key(state: &State, slot: u32) -> io::Result<(u32, u32)> {
        let key = columns::table(state, TableId::CompletedFocuses)?
            .key(slot)
            .ok_or_else(|| io::Error::other("A CompletedFocuses slot holds no row"))?;
        Ok(((key >> 32) as u32, key as u32))
    }

    /// `raw`'s completed focuses changed: its Focus row's order is built again.
    fn touch_owner(&mut self, raw: u32) {
        let Some(&slot) = self.by_raw.get(&raw) else {
            return;
        };
        if let Some(focus) = self.focus.get_mut(slot as usize).and_then(Option::as_mut) {
            focus.order = None;
            if let Some(holder) = focus.holder {
                self.dirty.insert(holder);
            }
        }
    }

    /// Every Focus and CompletedFocuses row, read whole.
    fn index(&mut self, state: &State, roster: &Roster) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("focus.columnar.index", None);
        self.clear();
        let live: Vec<u32> = columns::table(state, TableId::Focus)?.live().ones().collect();
        for slot in live {
            let focus = self.read_focus(state, roster, slot)?;
            self.insert_focus(slot, focus);
        }
        let live: Vec<u32> = columns::table(state, TableId::CompletedFocuses)?
            .live()
            .ones()
            .collect();
        for slot in live {
            let (raw, ordinal) = Self::completed_key(state, slot)?;
            let token = self.token(state, slot)?;
            self.owned.entry(raw).or_default().insert(ordinal, token);
        }
        self.dirty.clear();
        self.stale = false;
        Ok(())
    }

    /// The hour's changes to the Focus and CompletedFocuses rows.
    fn apply(&mut self, state: &State, roster: &Roster) -> io::Result<()> {
        let edits = columns::changes(state, TableId::Focus)?;
        if edits.truncated {
            return Err(io::Error::other("A complete Focus table was read in part"));
        }
        for &slot in &edits.deleted {
            self.remove_focus(slot);
        }
        for &slot in &edits.inserted {
            self.remove_focus(slot);
            let focus = self.read_focus(state, roster, slot)?;
            self.insert_focus(slot, focus);
        }
        let inserted: HashSet<u32> = edits.inserted.iter().copied().collect();
        let written: Vec<(u32, u64)> = edits
            .changed
            .iter()
            .zip(&edits.masks)
            .chain(edits.fill_slots.iter().zip(&edits.fill_masks))
            .filter(|(slot, _)| !inserted.contains(slot))
            .map(|(&slot, &mask)| (slot, mask))
            .collect();
        for (slot, mask) in written {
            self.rewrite_focus(state, roster, slot, mask)?;
        }
        let edits = columns::changes(state, TableId::CompletedFocuses)?;
        if edits.truncated {
            return Err(io::Error::other(
                "A complete CompletedFocuses table was read in part",
            ));
        }
        let mut owners = HashSet::new();
        for &key in &edits.deleted_keys {
            let (raw, ordinal) = ((key >> 32) as u32, key as u32);
            if let Some(owned) = self.owned.get_mut(&raw) {
                owned.remove(ordinal);
                if owned.rows.is_empty() {
                    self.owned.remove(&raw);
                }
            }
            owners.insert(raw);
        }
        let token = Fields::bit(self.fields.token);
        let written: Vec<u32> = edits
            .inserted
            .iter()
            .copied()
            .chain(
                edits
                    .changed
                    .iter()
                    .zip(&edits.masks)
                    .chain(edits.fill_slots.iter().zip(&edits.fill_masks))
                    .filter(|&(_, &mask)| mask & token != 0)
                    .map(|(&slot, _)| slot),
            )
            .collect();
        for slot in written {
            let (raw, ordinal) = Self::completed_key(state, slot)?;
            let token = self.token(state, slot)?;
            self.owned.entry(raw).or_default().insert(ordinal, token);
            owners.insert(raw);
        }
        for raw in owners {
            self.touch_owner(raw);
        }
        Ok(())
    }

    /// Every Focus row's holder named again, after the links changed.
    fn regroup(&mut self, roster: &Roster) {
        self.by_holder.clear();
        for (slot, focus) in self.focus.iter_mut().enumerate() {
            let Some(focus) = focus else { continue };
            focus.holder = roster.slot_of(focus.raw);
            if let Some(holder) = focus.holder {
                self.by_holder.entry(holder).or_default().push(slot as u32);
            }
        }
    }

    /// Canonical slot `slot`'s Focus slot: the row of the highest raw country its links name.
    fn winner(&self, slot: u32) -> Option<u32> {
        self.by_holder.get(&slot)?.iter().copied().max_by_key(|&at| {
            self.focus[at as usize]
                .as_ref()
                .map_or(0, |focus| focus.raw)
        })
    }

    /// `country`'s observation, or why it has none.
    fn classify(&mut self, country: &Country, named: bool) -> Result<Observation, usize> {
        if !country.resolved {
            return Err(0);
        }
        let Some(at) = self.winner(country.slot) else {
            return Err(1);
        };
        if !named {
            return Err(2);
        }
        let (raw, count, progress, cached) = {
            let focus = self.focus[at as usize].as_ref().expect("an indexed winner");
            (focus.raw, focus.count, focus.progress, focus.order.clone())
        };
        let complete = match self.owned.get(&raw) {
            Some(owned) => owned.complete(count),
            None => count == 0,
        };
        if !complete {
            return Err(3);
        }
        let order = match cached {
            Some(order) => order,
            None => {
                let completed = match self.owned.get_mut(&raw) {
                    Some(owned) => owned.list(),
                    None => Arc::from(Vec::new()),
                };
                let focus = self.focus[at as usize].as_mut().expect("an indexed winner");
                let order = Arc::new(Order {
                    completed,
                    current: focus.current.clone(),
                    continuous: focus.continuous.clone(),
                    paused: focus.paused,
                });
                focus.order = Some(Arc::clone(&order));
                order
            }
        };
        Ok(Observation {
            slot: country.slot,
            country_id: country.id,
            progress,
            order,
        })
    }

    fn name(&mut self, roster: &Roster, tag: Tag) -> Option<Arc<str>> {
        self.names
            .entry(tag)
            .or_insert_with(|| {
                std::str::from_utf8(roster.tag(tag))
                    .ok()
                    .filter(|text| !text.is_empty())
                    .map(Arc::from)
            })
            .clone()
    }

    /// `tag`'s observation now: its highest country id observed. Its countries' reasons are
    /// counted anew.
    fn evaluate(&mut self, roster: &Roster, tag: Tag) -> Option<(Arc<str>, Observation)> {
        let name = self.name(roster, tag);
        let mut best = None;
        for country in roster.holders(tag) {
            let held = self.reasons.remove(&country.id);
            if let Some(reason) = held {
                self.counts[reason] -= 1;
            }
            match self.classify(country, name.is_some()) {
                Ok(observed) => best = Some(observed),
                Err(reason) => {
                    self.reasons.insert(country.id, reason);
                    self.counts[reason] += 1;
                }
            }
        }
        Some((name?, best?))
    }

    /// The tags whose observation changed since last sent, of `tags` (every held tag when
    /// `all`, the others no longer observed).
    fn changes(
        &mut self,
        roster: &Roster,
        tags: Vec<Tag>,
        all: bool,
        out: &mut Vec<(Arc<str>, Option<Observation>)>,
    ) {
        let mut seen = HashSet::new();
        for tag in tags {
            if !seen.insert(tag) {
                continue;
            }
            match self.evaluate(roster, tag) {
                Some((name, now)) => {
                    let same = self.sent.get(&tag).is_some_and(|(_, held)| *held == now);
                    if !same {
                        out.push((Arc::clone(&name), Some(now.clone())));
                        self.sent.insert(tag, (name, now));
                    }
                }
                None => {
                    if let Some((name, _)) = self.sent.remove(&tag) {
                        out.push((name, None));
                    }
                }
            }
        }
        if all {
            let gone: Vec<Tag> = self
                .sent
                .keys()
                .copied()
                .filter(|tag| !seen.contains(tag))
                .collect();
            for tag in gone {
                let (name, _) = self.sent.remove(&tag).expect("a sent tag");
                out.push((name, None));
            }
        }
    }

    fn heap(&self) -> usize {
        self.focus.capacity() * size_of::<Option<Focus>>()
            + self
                .focus
                .iter()
                .flatten()
                .map(|focus| {
                    focus.current.as_ref().map_or(0, |text| text.len())
                        + focus.continuous.as_ref().map_or(0, |text| text.len())
                        + focus.order.as_ref().map_or(0, |order| {
                            size_of::<Order>() + order.completed.len() * size_of::<Arc<[u8]>>()
                        })
                })
                .sum::<usize>()
            + (self.by_raw.capacity() + self.reasons.capacity()) * 16
            + self.by_holder.capacity() * size_of::<(u32, Vec<u32>)>()
            + self.owned.capacity() * size_of::<(u32, Owned)>()
            + self.owned.values().map(Owned::heap).sum::<usize>()
            + self.names.capacity() * size_of::<(Tag, Option<Arc<str>>)>()
            + self.sent.capacity() * size_of::<(Tag, (Arc<str>, Observation))>()
            + self.dirty.capacity() * 4
    }
}

// Every segment starts cold: its first hour reads every row again; what came before is the
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
        let _cpu = replay_frame::cpu::scope("focus.columnar", Some("focus"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.clear();
            self.names.clear();
            self.sent.clear();
            self.sent_refusals = None;
            self.sent_tally = None;
            self.statuses = [None; 4];
        }
        self.cold = false;
        self.last = Some(at);
        let statuses = [0, 1, 2, 3].map(|at| columns::status(state, TABLES[at]));
        let complete = statuses.iter().all(|&status| status == columns::Status::Complete);
        let mut refusals = None;
        if statuses.map(Some) != self.statuses {
            let mut now = Vec::new();
            for (&table, &status) in TABLES.iter().zip(&statuses) {
                now.extend(refusal(state, table, status, self.mode)?);
            }
            if self.sent_refusals.as_ref() != Some(&now) {
                self.sent_refusals = Some(now.clone());
                refusals = Some(now);
            }
            self.statuses = statuses.map(Some);
        }
        let mut tags = Vec::new();
        let mut whole = false;
        let counts: [u64; 5] = if complete {
            if self.stale {
                // Every tag sent was sent none when the rows stopped being indexed.
                self.index(state, roster)?;
                let held: Vec<Tag> = roster.held().collect();
                whole = true;
                self.changes(roster, held, true, &mut tags);
            } else {
                if roster.rebuilt() || !roster.changed_links().is_empty() {
                    self.regroup(roster);
                }
                self.apply(state, roster)?;
                if roster.changed() {
                    // A country's tag, slot or link moved: every tag and country is read anew.
                    self.reasons.clear();
                    self.counts = [0; 4];
                    self.dirty.clear();
                    let held: Vec<Tag> = roster.held().collect();
                    self.changes(roster, held, true, &mut tags);
                } else if !self.dirty.is_empty() {
                    let dirty: Vec<u32> = self.dirty.drain().collect();
                    let mut touched: Vec<Tag> = dirty
                        .into_iter()
                        .flat_map(|slot| roster.at_slot(slot).map(|country| country.tag))
                        .collect();
                    touched.sort_unstable();
                    touched.dedup();
                    self.changes(roster, touched, false, &mut tags);
                }
            }
            let [unresolved, missing, untagged, incomplete] = self.counts;
            [unresolved, missing, untagged, incomplete, 0]
        } else {
            if !self.stale || !self.sent.is_empty() {
                self.clear();
                let gone: Vec<Tag> = self.sent.keys().copied().collect();
                for tag in gone {
                    let (name, _) = self.sent.remove(&tag).expect("a sent tag");
                    tags.push((name, None));
                }
            }
            whole = opens;
            [0, 0, 0, 0, 1]
        };
        // Built only where the counts changed: most hours change none.
        let tally = (self.sent_tally != Some(counts)).then(|| {
            self.sent_tally = Some(counts);
            REASONS
                .iter()
                .chain([&INCOMPLETE_TABLES])
                .zip(counts)
                .filter(|&(_, count)| count != 0)
                .map(|(&reason, count)| (reason, count))
                .collect()
        });
        whole |= opens;
        let quiet = !whole && refusals.is_none() && tally.is_none() && tags.is_empty();
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            facts: (!quiet).then(|| {
                Box::new(Facts {
                    refusals,
                    tally,
                    whole,
                    tags,
                })
            }),
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
                "Not a saved focus projector",
            ));
        }
        *self = Self::new(self.format, self.mode)?;
        Ok(())
    }
}
