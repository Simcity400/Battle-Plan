//! What the peace observer reads of an hour's tables, and how an hour hands that on: the
//! [`Reader`] reads each table an hour may have changed as the observer does and hands on only
//! what the read gives otherwise than the read before it ([`HourReads`]); the observer keeps
//! the reads ([`Reads`]) and applies each hour's. The segment's operator ([`super::facts`]) and
//! the row path (the test oracle's scan of row frames) share the reader.

use super::{CountryTags, exact::missing_reason, state_action_key};
use replay_data::rows::{
    CountryAppearanceRow, CountryCounterInputRow, CountryLinkRow, CountryRow,
    LexicalTokenRow, PeaceConferenceRow, PeaceSettlementRow, PoliticalPartyRow, PoliticsRow,
    ProvinceRow, StateRow, SubjectRow, TABLE_COUNT, TableId, TableSchema, WarRelationRow, mask,
    tables,
};
use replay_frame::frame::{OperandStatus, Tables};
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

pub(super) type War = (u32, u32, i32, i32);
pub(super) type Subject = (u32, u32, i32, i32, u8, Vec<u8>);
pub(super) type Party = (u32, u32, Vec<u8>);
/// A conference: its id, current turn and finalized byte.
pub(super) type Conference = (u64, i32, u8);

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// A keyed list as read again: whole, or the entries set and the keys gone, both ascending.
#[derive(replay_spool::Spooled)]
pub(super) enum Rows<T> {
    Whole(Vec<T>),
    Edits { set: Vec<T>, gone: Vec<u64> },
}

fn ascending<T>(rows: &[T], key: fn(&T) -> u64) -> bool {
    rows.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}

impl<T: Clone + PartialEq> Rows<T> {
    /// What `new` changed of `old`, entries keyed by `key`: none when equal. Lists not in
    /// ascending key order are handed on whole.
    fn of(old: &[T], new: &[T], key: fn(&T) -> u64) -> Option<Self> {
        if !ascending(old, key) || !ascending(new, key) {
            return (old != new).then(|| Self::Whole(new.to_vec()));
        }
        let mut set = Vec::new();
        let mut gone = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < old.len() || j < new.len() {
            match (old.get(i), new.get(j)) {
                (Some(a), Some(b)) if key(a) == key(b) => {
                    if a != b {
                        set.push(b.clone());
                    }
                    i += 1;
                    j += 1;
                }
                (Some(a), Some(b)) if key(a) < key(b) => {
                    gone.push(key(a));
                    i += 1;
                }
                (Some(a), None) => {
                    gone.push(key(a));
                    i += 1;
                }
                (_, Some(b)) => {
                    set.push(b.clone());
                    j += 1;
                }
                (None, None) => break,
            }
        }
        if set.is_empty() && gone.is_empty() {
            return None;
        }
        if set.len() + gone.len() >= new.len() {
            return Some(Self::Whole(new.to_vec()));
        }
        Some(Self::Edits { set, gone })
    }
}

impl<T> Rows<T> {
    /// `held`, ascending by `key`, with the edits applied.
    fn edit(held: &mut Vec<T>, set: Vec<T>, gone: Vec<u64>, key: fn(&T) -> u64) {
        // Where no row goes, the rows held take their new values in place.
        let set = if gone.is_empty() {
            let mut missing = Vec::new();
            for row in set {
                match held.binary_search_by_key(&key(&row), key) {
                    Ok(position) => held[position] = row,
                    Err(_) => missing.push(row),
                }
            }
            missing
        } else {
            set
        };
        if !set.is_empty() || !gone.is_empty() {
            Self::merge(held, set, gone, key);
        }
    }

    fn merge(held: &mut Vec<T>, set: Vec<T>, gone: Vec<u64>, key: fn(&T) -> u64) {
        let old = std::mem::take(held);
        held.reserve(old.len() + set.len());
        let mut set = set.into_iter().peekable();
        let mut gone = gone.into_iter().peekable();
        for row in old {
            let at = key(&row);
            while let Some(next) = set.next_if(|next| key(next) < at) {
                held.push(next);
            }
            if let Some(next) = set.next_if(|next| key(next) == at) {
                held.push(next);
                continue;
            }
            while gone.next_if(|&gone| gone < at).is_some() {}
            if gone.next_if(|&gone| gone == at).is_some() {
                continue;
            }
            held.push(row);
        }
        held.extend(set);
    }

    fn bytes(&self, heap: fn(&T) -> usize) -> usize {
        match self {
            Self::Whole(rows) => list_bytes(rows, heap),
            Self::Edits { set, gone } => list_bytes(set, heap) + gone.capacity() * size_of::<u64>(),
        }
    }
}

fn list_bytes<T>(rows: &Vec<T>, heap: fn(&T) -> usize) -> usize {
    rows.capacity() * size_of::<T>() + rows.iter().map(heap).sum::<usize>()
}

fn none<T>(_: &T) -> usize {
    0
}

/// How the finalizer table reads: why it misses conferences (none when complete), and whether
/// its rows can be read.
#[derive(Clone, Default, PartialEq)]
#[derive(replay_spool::Spooled)]
pub(super) struct Finalizers {
    pub reason: Option<String>,
    pub readable: bool,
}

/// A finalizer row read for the first time, with the names its actions' tokens have in the
/// dictionary that hour.
#[derive(replay_spool::Spooled)]
pub(super) struct Settled {
    pub row: PeaceSettlementRow,
    pub tokens: HashMap<u32, String>,
}

/// A read field as read again: none when unchanged, none inside when it cannot be read.
type Change<T> = Option<Option<T>>;

/// What one hour's reads give otherwise than the reads before them.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub struct HourReads {
    pub(super) countries: Change<Arc<CountryTags>>,
    pub(super) owners: Change<Rows<(u32, u32)>>,
    pub(super) wars: Change<Rows<War>>,
    pub(super) subjects: Change<Rows<Subject>>,
    pub(super) existence: Change<Rows<(u32, bool)>>,
    pub(super) politics: Change<Rows<(u32, Option<u32>)>>,
    pub(super) parties: Change<Rows<Party>>,
    pub(super) appearances: Change<Rows<(u32, Vec<u8>)>>,
    pub(super) vps: Change<Rows<(u32, i64)>>,
    pub(super) roster: Change<Rows<Conference>>,
    pub(super) finalizers: Option<Finalizers>,
    /// The finalizer rows the hour reads for the first time.
    pub(super) settled: Vec<Settled>,
    /// The state-taking keyword's spelling, or why it is unknown.
    pub(super) action_key: Option<Result<String, String>>,
}

fn change_bytes<T>(change: &Change<Rows<T>>, heap: fn(&T) -> usize) -> usize {
    change.as_ref().and_then(Option::as_ref).map_or(0, |rows| rows.bytes(heap))
}

fn country_bytes(countries: &CountryTags) -> usize {
    countries.tags.capacity() * size_of::<(u32, Arc<str>)>()
        + countries.identities.capacity() * size_of::<(Arc<str>, (u32, u32))>()
        + countries.identities.keys().map(|tag| tag.len()).sum::<usize>()
}

fn settled_bytes(settled: &Settled) -> usize {
    let row = &settled.row;
    let actions: usize = row
        .actions
        .iter()
        .map(|action| {
            action.name_key.as_ref().map_or(0, Vec::capacity)
                + action.cached_name.as_ref().map_or(0, Vec::capacity)
        })
        .sum();
    (row.winners.capacity() + row.losers.capacity()) * size_of::<u32>()
        + row.actions.capacity() * size_of::<replay_data::rows::SettledPeaceAction>()
        + actions
        + row.transfers.capacity() * size_of::<replay_data::rows::SettledPeaceTransfer>()
        + row.subjects.capacity() * size_of::<replay_data::rows::SettledPeaceSubject>()
        + row.failure.as_ref().map_or(0, Vec::capacity)
        + settled.tokens.capacity() * size_of::<(u32, String)>()
        + settled.tokens.values().map(String::capacity).sum::<usize>()
}

impl HourReads {
    /// Whether it changes no read.
    pub fn is_empty(&self) -> bool {
        self.countries.is_none()
            && self.owners.is_none()
            && self.wars.is_none()
            && self.subjects.is_none()
            && self.existence.is_none()
            && self.politics.is_none()
            && self.parties.is_none()
            && self.appearances.is_none()
            && self.vps.is_none()
            && self.roster.is_none()
            && self.finalizers.is_none()
            && self.settled.is_empty()
            && self.action_key.is_none()
    }

    /// The bytes it holds beyond its own size. Shared country tags count where handed on.
    pub fn bytes(&self) -> usize {
        self.countries
            .as_ref()
            .and_then(Option::as_ref)
            .map_or(0, |countries| country_bytes(countries))
            + change_bytes(&self.owners, none)
            + change_bytes(&self.wars, none)
            + change_bytes(&self.subjects, |row| row.5.capacity())
            + change_bytes(&self.existence, none)
            + change_bytes(&self.politics, none)
            + change_bytes(&self.parties, |row| row.2.capacity())
            + change_bytes(&self.appearances, |row| row.1.capacity())
            + change_bytes(&self.vps, none)
            + change_bytes(&self.roster, none)
            + self
                .finalizers
                .as_ref()
                .and_then(|finalizers| finalizers.reason.as_ref())
                .map_or(0, String::capacity)
            + self.settled.capacity() * size_of::<Settled>()
            + self.settled.iter().map(settled_bytes).sum::<usize>()
            + self.action_key.as_ref().map_or(0, |key| match key {
                Ok(text) | Err(text) => text.capacity(),
            })
    }
}

fn owner_key(row: &(u32, u32)) -> u64 {
    u64::from(row.0)
}
fn war_key(row: &War) -> u64 {
    u64::from(row.0) << 32 | u64::from(row.1)
}
fn subject_key(row: &Subject) -> u64 {
    u64::from(row.0) << 32 | u64::from(row.1)
}
fn existence_key(row: &(u32, bool)) -> u64 {
    u64::from(row.0)
}
fn politics_key(row: &(u32, Option<u32>)) -> u64 {
    u64::from(row.0)
}
fn party_key(row: &Party) -> u64 {
    u64::from(row.0) << 32 | u64::from(row.1)
}
fn appearance_key(row: &(u32, Vec<u8>)) -> u64 {
    u64::from(row.0)
}
fn vp_key(row: &(u32, i64)) -> u64 {
    u64::from(row.0)
}
fn conference_key(row: &Conference) -> u64 {
    row.0
}

/// Which reads an hour changed, as the observer compares them.
#[derive(Default)]
pub(super) struct Changed {
    pub holders: bool,
    pub owners: bool,
    pub wars: bool,
    pub subjects: bool,
    pub existence: bool,
    pub politics: bool,
    pub parties: bool,
    pub appearances: bool,
}

/// The reads the observer holds: each table's native operands it reads, none where the table
/// cannot be read whole.
#[derive(Default)]
pub(super) struct Reads {
    pub countries: Option<Arc<CountryTags>>,
    pub owners: Option<Vec<(u32, u32)>>,
    pub wars: Option<Vec<War>>,
    pub subjects: Option<Vec<Subject>>,
    pub existence: Option<Vec<(u32, bool)>>,
    pub politics: Option<Vec<(u32, Option<u32>)>>,
    pub parties: Option<Vec<Party>>,
    pub appearances: Option<Vec<(u32, Vec<u8>)>>,
    /// Victory points summed by state.
    pub vps: Option<Arc<HashMap<u32, i64>>>,
    pub roster: Option<Vec<Conference>>,
    pub finalizers: Finalizers,
}

/// `held` with `change` applied: whether its rows changed.
fn apply<T: PartialEq>(
    held: &mut Option<Vec<T>>,
    change: Change<Rows<T>>,
    key: fn(&T) -> u64,
) -> io::Result<bool> {
    Ok(match change {
        None => false,
        Some(None) => held.take().is_some(),
        Some(Some(Rows::Whole(rows))) => {
            let changed = held.as_ref() != Some(&rows);
            *held = Some(rows);
            changed
        }
        Some(Some(Rows::Edits { set, gone })) => {
            let held = held
                .as_mut()
                .ok_or_else(|| invalid("Peace reads edit rows never read"))?;
            Rows::edit(held, set, gone, key);
            true
        }
    })
}

impl Reads {
    /// `hour`'s reads applied: which changed. Its new finalizer rows and keyword stay in it.
    pub(super) fn apply(&mut self, hour: &mut HourReads) -> io::Result<Changed> {
        let mut changed = Changed::default();
        if let Some(countries) = hour.countries.take() {
            changed.holders = self.countries.as_deref() != countries.as_deref();
            self.countries = countries;
        }
        changed.owners = apply(&mut self.owners, hour.owners.take(), owner_key)?;
        changed.wars = apply(&mut self.wars, hour.wars.take(), war_key)?;
        changed.subjects = apply(&mut self.subjects, hour.subjects.take(), subject_key)?;
        changed.existence = apply(&mut self.existence, hour.existence.take(), existence_key)?;
        changed.politics = apply(&mut self.politics, hour.politics.take(), politics_key)?;
        changed.parties = apply(&mut self.parties, hour.parties.take(), party_key)?;
        changed.appearances =
            apply(&mut self.appearances, hour.appearances.take(), appearance_key)?;
        match hour.vps.take() {
            None => {}
            Some(None) => self.vps = None,
            Some(Some(Rows::Whole(totals))) => {
                self.vps = Some(Arc::new(totals.into_iter().collect()));
            }
            Some(Some(Rows::Edits { set, gone })) => {
                let totals = self
                    .vps
                    .as_mut()
                    .ok_or_else(|| invalid("Peace reads edit victory points never read"))?;
                // Snapshots keep the totals they took.
                let totals = Arc::make_mut(totals);
                for state in gone {
                    totals.remove(&(state as u32));
                }
                totals.extend(set);
            }
        }
        apply(&mut self.roster, hour.roster.take(), conference_key)?;
        if let Some(finalizers) = hour.finalizers.take() {
            self.finalizers = finalizers;
        }
        Ok(changed)
    }
}

/// What the reader last handed on of a field: none before it handed it on, none inside when
/// the table could not be read.
type Sent<T> = Option<Option<T>>;

/// `new` where it differs from `sent`, which it becomes.
fn rows<T: Clone + PartialEq>(
    sent: &mut Sent<Vec<T>>,
    new: Option<Vec<T>>,
    key: fn(&T) -> u64,
) -> Change<Rows<T>> {
    let change = match (sent.as_ref(), &new) {
        (Some(Some(old)), Some(rows)) => Some(Some(Rows::of(old, rows, key)?)),
        (Some(None), None) => return None,
        (_, Some(rows)) => Some(Some(Rows::Whole(rows.clone()))),
        (_, None) => Some(None),
    };
    *sent = Some(new);
    change
}

/// Each row of `T`'s complete table as `project` reads it, of the fields `mask` names: none
/// when it cannot be read whole.
fn read<T: TableSchema, R>(
    frame: &impl Tables,
    mask: u64,
    project: impl Fn(&T::Row) -> R,
) -> io::Result<Option<Vec<R>>> {
    let Ok(rows) = frame.complete_rows::<T>(mask) else {
        return Ok(None);
    };
    let mut read = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        read.push(project(&*row?));
    }
    Ok(Some(read))
}

/// The countries, each its id, canonical slot and tag, and the links, each its raw id and
/// canonical slot: none when either cannot be read whole.
fn country_tables(frame: &impl Tables) -> io::Result<Option<CountryTags>> {
    let countries = read::<tables::Countries, _>(
        frame,
        mask::<CountryRow>(&["canonical_slot", "tag"]),
        |row| (row.id, row.canonical_slot, row.tag.clone()),
    )?;
    let links = read::<tables::CountryLinks, _>(
        frame,
        mask::<CountryLinkRow>(&["canonical_slot"]),
        |row| (row.id, row.canonical_slot),
    )?;
    Ok(match (countries, links) {
        (Some(countries), Some(links)) => CountryTags::resolve(&countries, &links),
        _ => None,
    })
}

/// Reads each table an hour may have changed as the observer does, and hands on what they give
/// otherwise than it handed on last. A new reader hands every read on.
#[derive(Default)]
pub(super) struct Reader {
    countries: Sent<Arc<CountryTags>>,
    owners: Sent<Vec<(u32, u32)>>,
    wars: Sent<Vec<War>>,
    subjects: Sent<Vec<Subject>>,
    existence: Sent<Vec<(u32, bool)>>,
    politics: Sent<Vec<(u32, Option<u32>)>>,
    parties: Sent<Vec<Party>>,
    appearances: Sent<Vec<(u32, Vec<u8>)>>,
    vps: Sent<Vec<(u32, i64)>>,
    /// Each province's state and victory points as the totals were last summed from.
    provinces: Option<Vec<(u32, i32)>>,
    roster: Sent<Vec<Conference>>,
    finalizers: Option<Finalizers>,
    /// The finalizer rows handed on, by sequence.
    seen: HashSet<u64>,
    action_key: Option<Result<String, String>>,
}

impl Reader {
    /// The tables `frame` holds whose `changed` flag is set, read again.
    pub(super) fn read(
        &mut self,
        frame: &impl Tables,
        changed: &[bool; TABLE_COUNT],
    ) -> io::Result<HourReads> {
        let changed = |table: TableId| changed[table.index()];
        let mut hour = HourReads::default();
        if changed(TableId::Countries) || changed(TableId::CountryLinks) {
            let new = country_tables(frame)?;
            if self.countries.as_ref().is_none_or(|sent| sent.as_deref() != new.as_ref()) {
                let new = new.map(Arc::new);
                self.countries = Some(new.clone());
                hour.countries = Some(new);
            }
        }
        if changed(TableId::States) {
            let new = read::<tables::States, _>(frame, mask::<StateRow>(&["owner"]), |row| {
                (row.id, row.owner)
            })?;
            hour.owners = rows(&mut self.owners, new, owner_key);
        }
        if changed(TableId::WarRelations) {
            let new = read::<tables::WarRelations, _>(
                frame,
                mask::<WarRelationRow>(&["start_hour", "end_hour"]),
                |row| (row.first_country, row.second_country, row.start_hour, row.end_hour),
            )?;
            hour.wars = rows(&mut self.wars, new, war_key);
        }
        if changed(TableId::Subjects) {
            let new = read::<tables::Subjects, _>(
                frame,
                mask::<SubjectRow>(&["start_hour", "end_hour", "cancel", "state"]),
                |row| {
                    (
                        row.overlord,
                        row.subject,
                        row.start_hour,
                        row.end_hour,
                        row.cancel,
                        row.state.clone(),
                    )
                },
            )?;
            hour.subjects = rows(&mut self.subjects, new, subject_key);
        }
        if changed(TableId::CountryCounterInputs) {
            let new = read::<tables::CountryCounterInputs, _>(
                frame,
                mask::<CountryCounterInputRow>(&["owned_state_count"]),
                |row| (row.country, row.owned_state_count > 0),
            )?;
            hour.existence = rows(&mut self.existence, new, existence_key);
        }
        if changed(TableId::Politics) {
            let new = read::<tables::Politics, _>(
                frame,
                mask::<PoliticsRow>(&["ruling_party"]),
                |row| (row.country, row.ruling_party),
            )?;
            hour.politics = rows(&mut self.politics, new, politics_key);
        }
        if changed(TableId::PoliticalParties) {
            let new = read::<tables::PoliticalParties, _>(
                frame,
                mask::<PoliticalPartyRow>(&["token"]),
                |row| (row.country, row.id, row.token.clone()),
            )?;
            hour.parties = rows(&mut self.parties, new, party_key);
        }
        if changed(TableId::CountryAppearance) {
            let new = read::<tables::CountryAppearance, _>(
                frame,
                mask::<CountryAppearanceRow>(&["leader_ideology"]),
                |row| (row.country, row.leader_ideology.clone()),
            )?;
            hour.appearances = rows(&mut self.appearances, new, appearance_key);
        }
        if changed(TableId::Provinces) {
            hour.vps = self.victory_points(frame)?;
        }
        if changed(TableId::PeaceConferences) {
            let new = read::<tables::PeaceConferences, _>(
                frame,
                mask::<PeaceConferenceRow>(&["current_turn", "finalized"]),
                |row| (row.id, row.current_turn, row.finalized),
            )?;
            hour.roster = rows(&mut self.roster, new, conference_key);
        }
        if changed(TableId::LexicalTokens) {
            let key = state_action_key(frame);
            if self.action_key.as_ref() != Some(&key) {
                self.action_key = Some(key.clone());
                hour.action_key = Some(key);
            }
        }
        if changed(TableId::PeaceSettlements) {
            self.settlements(frame, &mut hour)?;
        }
        Ok(hour)
    }

    /// The victory points summed by state, where a province's state or points moved.
    fn victory_points(&mut self, frame: &impl Tables) -> io::Result<Change<Rows<(u32, i64)>>> {
        let Some(fields) = read::<tables::Provinces, _>(
            frame,
            mask::<ProvinceRow>(&["state", "victory_points"]),
            |row| (row.state, row.victory_points),
        )?
        else {
            self.provinces = None;
            return Ok(rows(&mut self.vps, None, vp_key));
        };
        let fields = fields.iter().copied();
        if self.vps.as_ref().is_some_and(Option::is_some)
            && self
                .provinces
                .as_ref()
                .is_some_and(|held| fields.clone().eq(held.iter().copied()))
        {
            return Ok(None);
        }
        let mut totals = HashMap::new();
        for (state, victory_points) in fields.clone() {
            *totals.entry(state).or_insert(0_i64) += i64::from(victory_points);
        }
        let mut totals: Vec<(u32, i64)> = totals.into_iter().collect();
        totals.sort_unstable_by_key(|&(state, _)| state);
        match &mut self.provinces {
            Some(held) => {
                held.clear();
                held.extend(fields);
            }
            None => self.provinces = Some(fields.collect()),
        }
        Ok(rows(&mut self.vps, Some(totals), vp_key))
    }

    /// The finalizer table's status where it changed, and its rows not handed on before.
    fn settlements(&mut self, frame: &impl Tables, hour: &mut HourReads) -> io::Result<()> {
        let rows = frame.rows::<tables::PeaceSettlements>(TableId::PeaceSettlements.full_mask());
        let finalizers = Finalizers {
            reason: missing_reason(frame.table_status(TableId::PeaceSettlements)),
            readable: rows.is_some(),
        };
        if self.finalizers.as_ref() != Some(&finalizers) {
            self.finalizers = Some(finalizers.clone());
            hour.finalizers = Some(finalizers);
        }
        let Some(rows) = rows else {
            return Ok(());
        };
        let tokens = frame
            .complete_rows::<tables::LexicalTokens>(mask::<LexicalTokenRow>(&["literal"]))
            .ok();
        for row in rows.iter() {
            let row = row?;
            if !self.seen.insert(row.sequence) {
                continue;
            }
            let mut spelled = Vec::new();
            for action in &row.actions {
                let Some(tokens) = &tokens else {
                    continue;
                };
                let Some(token) = tokens.get(u128::from(action.token))? else {
                    continue;
                };
                if let Ok(literal) = std::str::from_utf8(&token.literal) {
                    spelled.push((action.token, literal.to_owned()));
                }
            }
            hour.settled.push(Settled {
                row: row.into_owned(),
                tokens: spelled.into_iter().collect(),
            });
        }
        Ok(())
    }

    /// The bytes the reads it handed on hold.
    pub(super) fn bytes(&self) -> usize {
        fn sent<T>(sent: &Sent<Vec<T>>, heap: fn(&T) -> usize) -> usize {
            sent.as_ref()
                .and_then(Option::as_ref)
                .map_or(0, |rows| list_bytes(rows, heap))
        }
        self.countries
            .as_ref()
            .and_then(Option::as_ref)
            .map_or(0, |countries| country_bytes(countries))
            + sent(&self.owners, none)
            + sent(&self.wars, none)
            + sent(&self.subjects, |row| row.5.capacity())
            + sent(&self.existence, none)
            + sent(&self.politics, none)
            + sent(&self.parties, |row| row.2.capacity())
            + sent(&self.appearances, |row| row.1.capacity())
            + sent(&self.vps, none)
            + self
                .provinces
                .as_ref()
                .map_or(0, |held| held.capacity() * size_of::<(u32, i32)>())
            + sent(&self.roster, none)
            + self.seen.capacity() * size_of::<u64>()
    }
}
