//! Party facts, projected hour by hour from a columnar walk ([`replay::segments`]): for each tag
//! the walk's countries hold, what its parties, its ruling party's naming operands, its first
//! country leader and its last election date read this hour ([`Observation`]), sent where it
//! changed since the tag's last in the segment, and every held tag's at a whole hour (a segment's
//! first, a roster change or a table's status change). Runs, catalogues and elections are the
//! ordered emit's ([`super::emit`]).
//!
//! Rows are kept by table slot from the hour's changes and grouped by the holder slot their
//! country's link resolves; an hour reads again only the rows whose read columns it wrote, and
//! recomputes only the tags those rows reach.

use super::{PartyNaming, ScannedLeader, leader_portrait};
use dashboard_base::products::idea_history::IdeaCharacter;
use dashboard_world::{Needs, Roster, Tag, Tagged, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CharacterPortraitRow, CharacterRow, CountryAppearanceRow, CountryRow, LexicalTokenRow,
        PoliticalPartyRow, PoliticsRow, ReservedCountryNamingRow, TableId,
    },
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
    TableId::PoliticalParties,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
    TableId::ReservedCountryNaming,
];

/// The tables whose status change reads every tag anew.
const READ: [TableId; 7] = [
    TableId::PoliticalParties,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
    TableId::ReservedCountryNaming,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// The ruling party's ideology group and naming operands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruling {
    pub token: Arc<str>,
    /// Its naming and the selected original tag, or why it is unknown.
    pub naming: Arc<(PartyNaming, Result<String, String>)>,
}

/// A holder's parties as one hour reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parties {
    /// Each valid ideology group token's popularity, in row order; none at a duplicate hour.
    pub values: Vec<(Arc<str>, i64)>,
    /// None at a duplicate hour too.
    pub ruling: Option<Ruling>,
    /// Ideology tokens that are empty or not UTF-8, as recorded.
    pub invalid: Vec<Vec<u8>>,
    /// The rows repeat a token or hold two ruling parties: the first such token and its value.
    pub duplicate: Option<(Arc<str>, i64)>,
}

/// The ruling party's first country leader, as the holder's appearance records it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leader {
    /// Empty when the party has no leader.
    pub ideology: Arc<str>,
    pub person: Option<ScannedLeader>,
}

/// What one hour reads of a tag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// Its holder's canonical slot and raw id.
    pub identity: (u32, u32),
    pub parties: Arc<Parties>,
    /// None while the holder has no appearance row.
    pub leader: Option<Arc<Leader>>,
    /// The holder slot's first Politics row's last election date.
    pub election: Option<i32>,
}

fn text_heap(text: &str) -> usize {
    text.len()
}

impl Observation {
    fn heap(&self) -> usize {
        let parties = &self.parties;
        let naming = parties.ruling.as_ref().map_or(0, |ruling| {
            let (naming, selected) = &*ruling.naming;
            size_of::<(PartyNaming, Result<String, String>)>()
                + naming.token.capacity()
                + naming.tag.as_ref().map_or(0, String::capacity)
                + naming.original_tag.as_ref().map_or(0, String::capacity)
                + naming.key.as_ref().map_or(0, String::capacity)
                + match selected {
                    Ok(text) | Err(text) => text.capacity(),
                }
        });
        let leader = self.leader.as_ref().map_or(0, |leader| {
            size_of::<Leader>()
                + text_heap(&leader.ideology)
                + leader.person.as_ref().map_or(0, |person| {
                    person.character.token.capacity()
                        + person.character.name.capacity()
                        + match &person.portrait {
                            super::RecordedPortrait::Entry { identity, .. } => identity.capacity(),
                            _ => 0,
                        }
                })
        });
        size_of::<Parties>()
            + parties.values.capacity() * size_of::<(Arc<str>, i64)>()
            + parties.invalid.iter().map(Vec::capacity).sum::<usize>()
            + naming
            + leader
    }
}

/// What the ordered emit reads of an hour that changes a tag.
pub struct Facts {
    /// Every held tag is in `changed`: a tag held before and missing from it is held no more.
    pub whole: bool,
    /// The tags whose observation changed, by tag.
    pub changed: Vec<(Arc<str>, Arc<Observation>)>,
}

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Countries, CountryLinks or PoliticalParties is not complete: no tag's parties are read.
    pub unavailable: bool,
    /// None when the hour changes no tag.
    pub facts: Option<Box<Facts>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        let Some(facts) = &self.facts else {
            return 0;
        };
        size_of::<Facts>()
            + facts.changed.capacity() * size_of::<(Arc<str>, Arc<Observation>)>()
            + facts
                .changed
                .iter()
                .map(|(tag, observation)| {
                    tag.len() + size_of::<Observation>() + observation.heap()
                })
                .sum::<usize>()
    }
}

/// Rows grouped by the holder slot their country's link resolves, kept by table slot.
#[derive(Default)]
struct Grouped {
    /// Each table slot's key and holder slot, none where no link resolves it.
    rows: Vec<Option<(u128, Option<u32>)>>,
    /// Each holder slot's rows, by key: the key and the table slot.
    holders: HashMap<u32, Vec<(u128, u32)>>,
}

impl Grouped {
    fn clear(&mut self) {
        self.rows.clear();
        self.holders.clear();
    }

    /// Forgets the row at `slot`, returning its holder.
    fn remove(&mut self, slot: u32) -> Option<u32> {
        let (key, holder) = self.rows.get_mut(slot as usize).and_then(Option::take)?;
        let holder = holder?;
        if let Some(rows) = self.holders.get_mut(&holder) {
            if let Ok(at) = rows.binary_search_by_key(&key, |&(held, _)| held) {
                rows.remove(at);
            }
            if rows.is_empty() {
                self.holders.remove(&holder);
            }
        }
        Some(holder)
    }

    fn insert(&mut self, slot: u32, key: u128, holder: Option<u32>) {
        self.remove(slot);
        let index = slot as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, None);
        }
        self.rows[index] = Some((key, holder));
        if let Some(holder) = holder {
            let rows = self.holders.entry(holder).or_default();
            let at = rows.partition_point(|&(held, _)| held < key);
            rows.insert(at, (key, slot));
        }
    }

    fn holder(&self, slot: u32) -> Option<Option<u32>> {
        self.rows
            .get(slot as usize)
            .copied()
            .flatten()
            .map(|(_, holder)| holder)
    }

    /// `holder`'s table slots, by key.
    fn of(&self, holder: u32) -> &[(u128, u32)] {
        self.holders.get(&holder).map_or(&[], Vec::as_slice)
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<Option<(u128, Option<u32>)>>()
            + self.holders.capacity() * size_of::<(u32, Vec<(u128, u32)>)>()
            + self
                .holders
                .values()
                .map(|rows| rows.capacity() * size_of::<(u128, u32)>())
                .sum::<usize>()
    }
}

/// A PoliticalParties row as the parties read it.
#[derive(Clone)]
struct Party {
    popularity: i64,
    ruling: bool,
    token: Arc<[u8]>,
    /// The party's own tag, a raw link id.
    tag: u32,
    name_default: u8,
    name: Arc<[u8]>,
}

/// A CountryAppearance row's leader and naming operands.
#[derive(Clone, PartialEq, Eq)]
struct Appearance {
    leader_character: Option<u32>,
    leader_ideology: Vec<u8>,
    /// The original tag operands, as `original_tag` reads them.
    naming: CountryAppearanceRow,
}

impl Appearance {
    fn of(row: CountryAppearanceRow) -> Self {
        Self {
            leader_character: row.leader_character,
            leader_ideology: row.leader_ideology.clone(),
            naming: CountryAppearanceRow {
                country: row.country,
                original_tag: row.original_tag,
                original_tag_reference: row.original_tag_reference,
                ..Default::default()
            },
        }
    }

    fn heap(&self) -> usize {
        self.leader_ideology.capacity() + 64
    }
}

/// What a tag must read again.
const PARTIES: u8 = 1;
const LEADER: u8 = 2;
const ELECTION: u8 = 4;
const ALL: u8 = PARTIES | LEADER | ELECTION;

/// The columns a projection reads.
struct Fields {
    popularity: usize,
    /// Every PoliticalParties column but `long_name`.
    parties: u64,
    appearance: u64,
    appearance_naming: u64,
    last_election: usize,
    characters: u64,
    literal: usize,
}

impl Fields {
    fn new() -> io::Result<Self> {
        let bits = |table: TableId, names: &[&str]| -> io::Result<u64> {
            let mut bits = 0;
            for name in names {
                bits |= 1_u64 << columns::field(table, name)?;
            }
            Ok(bits)
        };
        let parties = TableId::PoliticalParties;
        let appearance = TableId::CountryAppearance;
        Ok(Self {
            popularity: columns::field(parties, "popularity")?,
            parties: bits(
                parties,
                &["popularity", "is_ruling", "token", "tag", "name_default", "name"],
            )?,
            appearance: bits(
                appearance,
                &[
                    "leader_ideology",
                    "leader_character",
                    "original_tag",
                    "original_tag_reference",
                ],
            )?,
            appearance_naming: bits(appearance, &["original_tag", "original_tag_reference"])?,
            last_election: columns::field(TableId::Politics, "last_election")?,
            characters: bits(TableId::Characters, &["name_token_id", "token"])?,
            literal: columns::field(TableId::LexicalTokens, "literal")?,
        })
    }
}

/// The slots `table`'s hour wrote any of `mask` at, with what it wrote.
fn written(state: &State, table: TableId, mask: u64) -> io::Result<Vec<(u32, u64)>> {
    let changes = columns::changes(state, table)?;
    Ok(changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &bits)| bits & mask != 0)
        .map(|(&slot, &bits)| (slot, bits))
        .collect())
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

/// The reason the row decoder gives a table that is not complete at `hour`.
fn refusal(
    state: &State,
    table: TableId,
    hour: i32,
    mode: CaptureMode,
) -> io::Result<String> {
    Ok(match columns::status(state, table) {
        columns::Status::Unavailable { reason } => {
            let reason = state
                .dictionary()
                .get(reason)
                .ok_or_else(|| io::Error::other("A table's reason names no dictionary entry"))?;
            format!(
                "hour {hour} {table:?} is unavailable: {}",
                String::from_utf8_lossy(reason)
            )
        }
        columns::Status::Truncated { missing } => {
            format!("hour {hour} {table:?} is truncated ({missing} missing rows)")
        }
        columns::Status::NotRecorded => {
            format!("hour {hour} {table:?} is not recorded in a {mode} recording")
        }
        columns::Status::Complete => {
            return Err(io::Error::other("A complete table has no refusal"));
        }
    })
}

/// The reserved country zero's tag and original-or-own tag, as the row readers read them.
fn reserved(row: &ReservedCountryNamingRow) -> (Result<String, String>, Result<String, String>) {
    let tag = std::str::from_utf8(&row.tag)
        .map(str::to_owned)
        .map_err(|_| "reserved country tag is not UTF-8".to_owned());
    let original = (|| -> Result<String, String> {
        let reference = row
            .original_tag_reference
            .require("ReservedCountryNaming.original_tag_reference")?
            .ok_or("ReservedCountryNaming.original_tag_reference capture refused")?;
        let bytes = if reference > 0 {
            row.original_tag
                .require("ReservedCountryNaming.original_tag")?
                .as_ref()
                .ok_or("ReservedCountryNaming.original_tag capture refused")?
        } else {
            &row.tag
        };
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| "reserved original-or-own tag is not UTF-8".to_owned())
    })();
    (tag, original)
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    fields: Fields,
    cold: bool,
    last: Option<i32>,
    parties: Grouped,
    party: Vec<Option<Party>>,
    /// By raw country id.
    appearances: HashMap<u32, Appearance>,
    /// Politics rows by holder, and each table slot's last election.
    politics: Grouped,
    elections: Vec<i32>,
    /// The reserved country zero's tag and original-or-own tag, or why each is unknown.
    reserved: Option<(Result<String, String>, Result<String, String>)>,
    /// Each leader character read, by id, with the token its script name is spelled by; and the
    /// characters read of each token.
    persons: HashMap<u32, (ScannedLeader, Option<u32>)>,
    spelled: HashMap<u32, HashSet<u32>>,
    /// Each ruling row's naming, by its table slot.
    namings: HashMap<u32, Arc<(PartyNaming, Result<String, String>)>>,
    /// Each held tag's name and last sent observation.
    sent: HashMap<Tag, (Arc<str>, Arc<Observation>)>,
    /// Each tag's ruling party's tag slot, and its leader's character.
    targets: Relation,
    leaders: Relation,
    dirty: HashMap<Tag, u8>,
}

/// Each tag's one value, and each value's tags.
#[derive(Default)]
struct Relation {
    of: HashMap<Tag, u32>,
    tags: HashMap<u32, HashSet<Tag>>,
}

impl Relation {
    fn clear(&mut self) {
        self.of.clear();
        self.tags.clear();
    }

    fn set(&mut self, tag: Tag, value: Option<u32>) {
        if self.of.get(&tag).copied() == value {
            return;
        }
        if let Some(old) = self.of.remove(&tag)
            && let Some(tags) = self.tags.get_mut(&old)
        {
            tags.remove(&tag);
            if tags.is_empty() {
                self.tags.remove(&old);
            }
        }
        if let Some(value) = value {
            self.of.insert(tag, value);
            self.tags.entry(value).or_default().insert(tag);
        }
    }

    /// The tags of `value`.
    fn of_value(&self, value: u32) -> Vec<Tag> {
        self.tags
            .get(&value)
            .map(|tags| tags.iter().copied().collect())
            .unwrap_or_default()
    }

    fn heap(&self) -> usize {
        self.of.capacity() * size_of::<(Tag, u32)>()
            + self.tags.capacity() * size_of::<(u32, HashSet<Tag>)>()
            + self
                .tags
                .values()
                .map(|tags| tags.capacity() * size_of::<Tag>())
                .sum::<usize>()
    }
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"parties projector: cold";

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        Ok(Self {
            format,
            mode,
            fields: Fields::new()?,
            cold: true,
            last: None,
            parties: Grouped::default(),
            party: Vec::new(),
            appearances: HashMap::new(),
            politics: Grouped::default(),
            elections: Vec::new(),
            reserved: None,
            persons: HashMap::new(),
            spelled: HashMap::new(),
            namings: HashMap::new(),
            sent: HashMap::new(),
            targets: Relation::default(),
            leaders: Relation::default(),
            dirty: HashMap::new(),
        })
    }

    fn version(&self) -> u16 {
        self.format.version()
    }

    fn mark(&mut self, tag: Tag, what: u8) {
        *self.dirty.entry(tag).or_default() |= what;
    }

    /// Marks every tag a country at holder `slot` holds.
    fn mark_slot(&mut self, roster: &Roster, slot: u32, what: u8) {
        let tags: Vec<Tag> = roster.at_slot(slot).map(|country| country.tag).collect();
        for tag in tags {
            self.mark(tag, what);
        }
    }

    /// Marks the tags whose ruling party's tag is at `slot`, and forgets every naming read.
    fn renamed(&mut self, slot: u32) {
        self.namings.clear();
        for tag in self.targets.of_value(slot) {
            self.mark(tag, PARTIES);
        }
    }

    /// Forgets leader `character`, and marks the tags it leads.
    fn reread(&mut self, character: u32) {
        let Some((_, token)) = self.persons.remove(&character) else {
            return;
        };
        if let Some(token) = token
            && let Some(characters) = self.spelled.get_mut(&token)
        {
            characters.remove(&character);
            if characters.is_empty() {
                self.spelled.remove(&token);
            }
        }
        for tag in self.leaders.of_value(character) {
            self.mark(tag, LEADER);
        }
    }

    fn read_party(&self, state: &State, slot: u32) -> io::Result<(u128, u32, Party)> {
        let table = TableId::PoliticalParties;
        let row: PoliticalPartyRow = columns::row(state, table, slot, self.version())?;
        let key = (u128::from(row.country) << 32) | u128::from(row.id);
        Ok((
            key,
            row.country,
            Party {
                popularity: row.popularity,
                ruling: row.is_ruling != 0,
                token: Arc::from(row.token),
                tag: row.tag,
                name_default: row.name_default,
                name: Arc::from(row.name),
            },
        ))
    }

    fn popularity(&self, state: &State, slot: u32) -> io::Result<i64> {
        let held = columns::table(state, TableId::PoliticalParties)?;
        if !held.recorded(self.fields.popularity, slot) {
            let row: PoliticalPartyRow =
                columns::row(state, TableId::PoliticalParties, slot, self.version())?;
            return Ok(row.popularity);
        }
        columns::plain::<i64>(held, self.fields.popularity)?
            .get(slot as usize)
            .copied()
            .ok_or_else(|| io::Error::other("A PoliticalParties slot holds no cell"))
    }

    fn put_party(&mut self, slot: u32, party: Party) {
        let index = slot as usize;
        if self.party.len() <= index {
            self.party.resize(index + 1, None);
        }
        self.party[index] = Some(party);
    }

    fn read_election(&self, state: &State, slot: u32) -> io::Result<i32> {
        let held = columns::table(state, TableId::Politics)?;
        if !held.recorded(self.fields.last_election, slot) {
            let row: PoliticsRow = columns::row(state, TableId::Politics, slot, self.version())?;
            return Ok(row.last_election);
        }
        columns::plain::<i32>(held, self.fields.last_election)?
            .get(slot as usize)
            .copied()
            .ok_or_else(|| io::Error::other("A Politics slot holds no cell"))
    }

    fn put_election(&mut self, slot: u32, date: i32) {
        let index = slot as usize;
        if self.elections.len() <= index {
            self.elections.resize(index + 1, 0);
        }
        self.elections[index] = date;
    }

    fn read_appearance(&self, state: &State, slot: u32) -> io::Result<Appearance> {
        let row: CountryAppearanceRow =
            columns::row(state, TableId::CountryAppearance, slot, self.version())?;
        Ok(Appearance::of(row))
    }

    fn read_reserved(&mut self, state: &State, hour: i32) -> io::Result<()> {
        let table = TableId::ReservedCountryNaming;
        self.reserved = Some(if columns::complete(state, table) {
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            match live[..] {
                [slot] => {
                    let row: ReservedCountryNamingRow =
                        columns::row(state, table, slot, self.version())?;
                    reserved(&row)
                }
                _ => {
                    let why = "ReservedCountryNaming has no singleton row for country zero";
                    (Err(why.to_owned()), Err(why.to_owned()))
                }
            }
        } else {
            let why = format!(
                "ReservedCountryNaming is unavailable this hour: {}",
                refusal(state, table, hour, self.mode)?
            );
            (Err(why.clone()), Err(why))
        });
        Ok(())
    }

    /// Every row read anew, as at a segment's first hour.
    fn index(&mut self, state: &State, roster: &Roster, hour: i32) -> io::Result<()> {
        self.parties.clear();
        self.party.clear();
        self.appearances.clear();
        self.politics.clear();
        self.elections.clear();
        self.persons.clear();
        self.spelled.clear();
        self.namings.clear();
        let table = TableId::PoliticalParties;
        if columns::complete(state, table) {
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            for slot in live {
                let (key, country, party) = self.read_party(state, slot)?;
                self.parties.insert(slot, key, roster.slot_of(country));
                self.put_party(slot, party);
            }
        }
        let table = TableId::CountryAppearance;
        if columns::complete(state, table) {
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            for slot in live {
                let appearance = self.read_appearance(state, slot)?;
                self.appearances
                    .insert(appearance.naming.country, appearance);
            }
        }
        let table = TableId::Politics;
        if columns::complete(state, table) {
            let held = columns::table(state, table)?;
            let live: Vec<(u32, u128)> = held
                .live()
                .ones()
                .map(|slot| held.key(slot).map(|key| (slot, key)))
                .collect::<Option<_>>()
                .ok_or_else(|| io::Error::other("A Politics slot holds no row"))?;
            for (slot, key) in live {
                let date = self.read_election(state, slot)?;
                self.politics.insert(slot, key, roster.slot_of(key as u32));
                self.put_election(slot, date);
            }
        }
        self.read_reserved(state, hour)
    }

    /// The hour's changed rows, marking the tags they reach.
    fn follow(&mut self, state: &State, roster: &Roster, hour: i32) -> io::Result<()> {
        let table = TableId::PoliticalParties;
        if columns::complete(state, table) && touched(state, table)? {
            let changes = columns::changes(state, table)?;
            let (deleted, inserted) = (changes.deleted.clone(), changes.inserted.clone());
            for slot in deleted {
                if let Some(holder) = self.parties.remove(slot) {
                    self.mark_slot(roster, holder, PARTIES);
                }
                if let Some(held) = self.party.get_mut(slot as usize) {
                    *held = None;
                }
                self.namings.remove(&slot);
            }
            for &slot in &inserted {
                let (key, country, party) = self.read_party(state, slot)?;
                let holder = roster.slot_of(country);
                self.parties.insert(slot, key, holder);
                self.put_party(slot, party);
                self.namings.remove(&slot);
                if let Some(holder) = holder {
                    self.mark_slot(roster, holder, PARTIES);
                }
            }
            let popularity = 1_u64 << self.fields.popularity;
            for (slot, bits) in written(state, table, self.fields.parties)? {
                if inserted.contains(&slot) {
                    continue;
                }
                let Some(Some(holder)) = self.parties.holder(slot) else {
                    // A row no link resolves reaches no tag; its fields are read when it moves.
                    if self.party.get(slot as usize).is_some_and(Option::is_some) {
                        let (_, _, party) = self.read_party(state, slot)?;
                        self.put_party(slot, party);
                    }
                    continue;
                };
                if bits & self.fields.parties & !popularity != 0 {
                    let (_, _, party) = self.read_party(state, slot)?;
                    self.put_party(slot, party);
                    self.namings.remove(&slot);
                } else {
                    let value = self.popularity(state, slot)?;
                    match self.party.get_mut(slot as usize).and_then(Option::as_mut) {
                        Some(party) if party.popularity == value => continue,
                        Some(party) => party.popularity = value,
                        None => return Err(io::Error::other("A party row is not indexed")),
                    }
                }
                self.mark_slot(roster, holder, PARTIES);
            }
        }
        let table = TableId::CountryAppearance;
        if columns::complete(state, table) && touched(state, table)? {
            let changes = columns::changes(state, table)?;
            let gone: Vec<u32> = changes.deleted_keys.iter().map(|&key| key as u32).collect();
            let inserted = changes.inserted.clone();
            let mut edited: Vec<u32> = written(state, table, self.fields.appearance)?
                .into_iter()
                .map(|(slot, _)| slot)
                .chain(inserted)
                .collect();
            edited.sort_unstable();
            edited.dedup();
            let mut moved = Vec::new();
            for id in gone {
                if let Some(old) = self.appearances.remove(&id) {
                    moved.push((id, Some(old), None));
                }
            }
            for slot in edited {
                let now = self.read_appearance(state, slot)?;
                let id = now.naming.country;
                let old = self.appearances.insert(id, now.clone());
                if old.as_ref() != Some(&now) {
                    moved.push((id, old, Some(now)));
                }
            }
            for (id, old, now) in moved {
                let Some(country) = roster.get(id).copied() else {
                    continue;
                };
                let leader = |held: &Option<Appearance>| {
                    held.as_ref()
                        .map(|held| (held.leader_character, held.leader_ideology.clone()))
                };
                let naming = |held: &Option<Appearance>| held.as_ref().map(|held| held.naming.clone());
                if leader(&old) != leader(&now) {
                    self.mark(country.tag, LEADER);
                }
                if naming(&old) != naming(&now) {
                    self.mark(country.tag, PARTIES);
                    self.renamed(country.slot);
                }
            }
        }
        let table = TableId::Politics;
        if columns::complete(state, table) && touched(state, table)? {
            let changes = columns::changes(state, table)?;
            let (deleted, inserted) = (changes.deleted.clone(), changes.inserted.clone());
            for slot in deleted {
                if let Some(holder) = self.politics.remove(slot) {
                    self.mark_slot(roster, holder, ELECTION);
                }
            }
            let mut edited: Vec<u32> = written(state, table, 1 << self.fields.last_election)?
                .into_iter()
                .map(|(slot, _)| slot)
                .chain(inserted)
                .collect();
            edited.sort_unstable();
            edited.dedup();
            let held = columns::table(state, table)?;
            for slot in edited {
                let key = held
                    .key(slot)
                    .ok_or_else(|| io::Error::other("A Politics slot holds no row"))?;
                let date = self.read_election(state, slot)?;
                let holder = roster.slot_of(key as u32);
                if self.politics.holder(slot).is_none() {
                    self.politics.insert(slot, key, holder);
                } else if self.elections.get(slot as usize) == Some(&date) {
                    continue;
                }
                self.put_election(slot, date);
                if let Some(holder) = holder {
                    self.mark_slot(roster, holder, ELECTION);
                }
            }
        }
        let table = TableId::Characters;
        if columns::complete(state, table) && touched(state, table)? {
            let changes = columns::changes(state, table)?;
            let held = columns::table(state, table)?;
            let mut ids: Vec<u32> = changes
                .deleted_keys
                .iter()
                .map(|&key| key as u32)
                .chain(
                    changes
                        .inserted
                        .iter()
                        .filter_map(|&slot| held.key(slot).map(|key| key as u32)),
                )
                .collect();
            for (slot, _) in written(state, table, self.fields.characters)? {
                ids.extend(held.key(slot).map(|key| key as u32));
            }
            for id in ids {
                self.reread(id);
            }
        }
        let table = TableId::CharacterPortraits;
        if columns::complete(state, table) && touched(state, table)? {
            let changes = columns::changes(state, table)?;
            let held = columns::table(state, table)?;
            let ids: Vec<u32> = changes
                .deleted_keys
                .iter()
                .map(|&key| key as u32)
                .chain(
                    changes
                        .inserted
                        .iter()
                        .chain(&changes.changed)
                        .chain(&changes.fill_slots)
                        .filter_map(|&slot| held.key(slot).map(|key| key as u32)),
                )
                .collect();
            for id in ids {
                self.reread(id);
            }
        }
        let table = TableId::LexicalTokens;
        if columns::complete(state, table) && touched(state, table)? {
            let changes = columns::changes(state, table)?;
            let held = columns::table(state, table)?;
            let tokens: Vec<u32> = changes
                .deleted_keys
                .iter()
                .map(|&key| key as u32)
                .chain(
                    changes
                        .inserted
                        .iter()
                        .chain(&changes.changed)
                        .chain(&changes.fill_slots)
                        .filter_map(|&slot| held.key(slot).map(|key| key as u32)),
                )
                .collect();
            for token in tokens {
                // Taken whole first, so each reread finds no bucket to search.
                for id in self.spelled.remove(&token).unwrap_or_default() {
                    self.reread(id);
                }
            }
        }
        let table = TableId::ReservedCountryNaming;
        if columns::complete(state, table) && touched(state, table)? {
            self.read_reserved(state, hour)?;
            self.renamed(0);
        }
        Ok(())
    }

    /// The spelling LexicalTokens holds of `token`.
    fn spelling(&self, state: &State, token: u32) -> io::Result<Option<Vec<u8>>> {
        let table = TableId::LexicalTokens;
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let Some(slot) = held.slot(u128::from(token)) else {
            return Ok(None);
        };
        if held.recorded(self.fields.literal, slot) {
            let text = *columns::text(held, self.fields.literal)?
                .get(slot as usize)
                .ok_or_else(|| io::Error::other("A token's slot holds no cell"))?;
            return Ok(Some(
                state
                    .dictionary()
                    .get(text)
                    .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?
                    .to_vec(),
            ));
        }
        let row: LexicalTokenRow = columns::row(state, table, slot, self.version())?;
        Ok(Some(row.literal))
    }

    /// Leader `id`'s character and portrait, as the advisor views name and picture it.
    fn person(&mut self, state: &State, id: u32) -> io::Result<ScannedLeader> {
        if let Some((person, _)) = self.persons.get(&id) {
            return Ok(person.clone());
        }
        let table = TableId::Characters;
        let found: Option<CharacterRow> = if columns::complete(state, table) {
            match columns::table(state, table)?.slot(u128::from(id)) {
                Some(slot) => Some(columns::row(state, table, slot, self.version())?),
                None => None,
            }
        } else {
            None
        };
        let spelled = found
            .as_ref()
            .and_then(|row| u32::try_from(row.name_token_id).ok());
        let token = match spelled {
            Some(token) => self
                .spelling(state, token)?
                .and_then(|literal| String::from_utf8(literal).ok())
                .unwrap_or_default(),
            None => String::new(),
        };
        let name = found
            .as_ref()
            .and_then(|row| std::str::from_utf8(&row.token).ok())
            .unwrap_or_default()
            .to_owned();
        let table = TableId::CharacterPortraits;
        let holder: Option<CharacterPortraitRow> = if columns::complete(state, table) {
            match columns::table(state, table)?.slot(u128::from(id)) {
                Some(slot) => Some(columns::row(state, table, slot, self.version())?),
                None => None,
            }
        } else {
            None
        };
        let portrait = leader_portrait(holder.as_ref().map(std::slice::from_ref), id);
        let person = ScannedLeader {
            character: IdeaCharacter { id, token, name },
            portrait,
        };
        self.persons.insert(id, (person.clone(), spelled));
        if let Some(token) = spelled {
            self.spelled.entry(token).or_default().insert(id);
        }
        Ok(person)
    }

    /// The party row at table `slot`'s naming, for the tag held by `holder` at canonical
    /// `slot`: its own tag, resolved through the links, not its current owner's.
    fn naming(
        &mut self,
        roster: &Roster,
        (row, party): (u32, &Party),
        token: &str,
        (holder, slot): (u32, u32),
    ) -> Arc<(PartyNaming, Result<String, String>)> {
        if let Some(naming) = self.namings.get(&row) {
            return Arc::clone(naming);
        }
        let reserved = |reserved: &Option<(Result<String, String>, Result<String, String>)>| {
            reserved.clone().unwrap_or_else(|| {
                let why = "ReservedCountryNaming is unavailable".to_owned();
                (Err(why.clone()), Err(why))
            })
        };
        let (tag, selected) = if party.tag == 0 {
            let (tag, original) = reserved(&self.reserved);
            (tag.ok(), original)
        } else {
            let resolved = (|| -> Result<(String, Result<String, String>), String> {
                let target = roster
                    .slot_of(party.tag)
                    .ok_or_else(|| format!("party tag link {} is unresolved", party.tag))?;
                if target == 0 {
                    let (tag, original) = reserved(&self.reserved);
                    return Ok((tag?, original));
                }
                let country = if target == slot {
                    roster.get(holder).copied()
                } else {
                    roster.at_slot(target).last().copied()
                }
                .ok_or_else(|| format!("party tag slot {target} has no Countries row"))?;
                let bytes = roster.tag(country.tag);
                let tag = std::str::from_utf8(bytes)
                    .map_err(|_| "party tag is not UTF-8".to_owned())?
                    .to_owned();
                let selected = self
                    .appearances
                    .get(&country.id)
                    .ok_or_else(|| {
                        format!("party tag slot {target} has no complete CountryAppearance row")
                    })
                    .and_then(|appearance| {
                        let row = CountryRow {
                            tag: bytes.to_vec(),
                            ..Default::default()
                        };
                        installed_content_country_names::country_names::original_tag(
                            &row,
                            &appearance.naming,
                        )
                        .map(str::to_owned)
                    });
                Ok((tag, selected))
            })();
            match resolved {
                Ok((tag, selected)) => (Some(tag), selected),
                Err(reason) => (None, Err(reason)),
            }
        };
        let naming = Arc::new((
            PartyNaming {
                token: token.to_owned(),
                tag,
                original_tag: selected.as_ref().ok().cloned(),
                key: (party.name_default == 0)
                    .then(|| String::from_utf8_lossy(&party.name).into_owned()),
            },
            selected,
        ));
        self.namings.insert(row, Arc::clone(&naming));
        naming
    }

    /// The parties of the tag `tag` held by `holder` at canonical `slot`.
    fn parties(&mut self, roster: &Roster, tag: Tag, (holder, slot): (u32, u32)) -> Parties {
        let mut parties = Parties::default();
        let mut ruling: Option<(u32, Arc<str>)> = None;
        let mut rulers = 0_usize;
        let rows: Vec<u32> = self.parties.of(slot).iter().map(|&(_, row)| row).collect();
        let mut target = None;
        for row in rows {
            let Some(party) = self.party.get(row as usize).and_then(Option::as_ref) else {
                continue;
            };
            let Some(token) = std::str::from_utf8(&party.token)
                .ok()
                .filter(|token| !token.is_empty())
            else {
                if !parties.invalid.iter().any(|held| **held == *party.token) {
                    parties.invalid.push(party.token.to_vec());
                }
                continue;
            };
            let token: Arc<str> = parties
                .values
                .iter()
                .find(|(held, _)| **held == *token)
                .map(|(held, _)| Arc::clone(held))
                .unwrap_or_else(|| Arc::from(token));
            if parties.duplicate.is_none()
                && parties.values.iter().any(|(held, _)| *held == token)
            {
                let first = parties
                    .values
                    .iter()
                    .find(|(held, _)| *held == token)
                    .map_or(party.popularity, |&(_, value)| value);
                parties.duplicate = Some((Arc::clone(&token), first));
            }
            parties.values.push((Arc::clone(&token), party.popularity));
            if party.ruling {
                rulers += 1;
                if ruling.is_none() {
                    ruling = Some((row, Arc::clone(&token)));
                    target = Some(if party.tag == 0 {
                        0
                    } else {
                        roster.slot_of(party.tag).unwrap_or(u32::MAX)
                    });
                }
            }
        }
        if rulers > 1 && parties.duplicate.is_none() {
            parties.duplicate = ruling
                .as_ref()
                .and_then(|(_, token)| parties.values.iter().find(|(held, _)| held == token))
                .cloned();
        }
        self.targets.set(tag, target);
        if parties.duplicate.is_some() {
            parties.values.clear();
            return parties;
        }
        if let Some((row, token)) = ruling {
            let party = self.party[row as usize]
                .clone()
                .expect("a ruling row is indexed");
            let naming = self.naming(roster, (row, &party), &token, (holder, slot));
            parties.ruling = Some(Ruling { token, naming });
        }
        parties
    }

    /// The leader the holder `holder`'s appearance records.
    fn leader(&mut self, state: &State, tag: Tag, holder: u32) -> io::Result<Option<Arc<Leader>>> {
        let Some(appearance) = self.appearances.get(&holder) else {
            self.leaders.set(tag, None);
            return Ok(None);
        };
        let ideology: Arc<str> = Arc::from(String::from_utf8_lossy(&appearance.leader_ideology));
        let character = appearance.leader_character;
        self.leaders.set(tag, character);
        let person = match character {
            Some(id) => Some(self.person(state, id)?),
            None => None,
        };
        Ok(Some(Arc::new(Leader { ideology, person })))
    }

    /// The election date the holder slot `slot`'s first Politics row records.
    fn election(&self, slot: u32) -> Option<i32> {
        self.politics
            .of(slot)
            .first()
            .and_then(|&(_, row)| self.elections.get(row as usize).copied())
    }

    /// What `tag` reads this hour, the parts `what` names read again; None when no resolved
    /// country holds it, or its tag is not UTF-8.
    fn observe(
        &mut self,
        state: &State,
        roster: &Roster,
        tag: Tag,
        what: u8,
    ) -> io::Result<Option<(Arc<str>, Arc<Observation>)>> {
        let Tagged::Found(country) = roster.tagged(tag) else {
            return Ok(None);
        };
        let country = *country;
        let held = self
            .sent
            .get(&tag)
            .filter(|(_, held)| held.identity == (country.slot, country.id))
            .cloned();
        let (name, what) = match &held {
            Some((name, _)) => (Arc::clone(name), what),
            None => match std::str::from_utf8(roster.tag(tag)) {
                Ok(name) => (Arc::from(name), ALL),
                Err(_) => return Ok(None),
            },
        };
        let previous = held.map(|(_, held)| held);
        let parties = match &previous {
            Some(previous) if what & PARTIES == 0 => Arc::clone(&previous.parties),
            _ => Arc::new(self.parties(roster, tag, (country.id, country.slot))),
        };
        let leader = match &previous {
            Some(previous) if what & LEADER == 0 => previous.leader.clone(),
            _ => self.leader(state, tag, country.id)?,
        };
        let election = match &previous {
            Some(previous) if what & ELECTION == 0 => previous.election,
            _ => self.election(country.slot),
        };
        Ok(Some((
            name,
            Arc::new(Observation {
                identity: (country.slot, country.id),
                parties,
                leader,
                election,
            }),
        )))
    }

    fn heap(&self) -> usize {
        self.parties.heap()
            + self.politics.heap()
            + self.party.capacity() * size_of::<Option<Party>>()
            + self
                .party
                .iter()
                .flatten()
                .map(|party| party.token.len() + party.name.len())
                .sum::<usize>()
            + self.appearances.capacity() * size_of::<(u32, Appearance)>()
            + self
                .appearances
                .values()
                .map(Appearance::heap)
                .sum::<usize>()
            + self.elections.capacity() * 4
            + self.persons.capacity() * size_of::<(u32, (ScannedLeader, Option<u32>))>()
            + self.spelled.capacity() * size_of::<(u32, HashSet<u32>)>()
            + self
                .spelled
                .values()
                .map(|characters| characters.capacity() * 4)
                .sum::<usize>()
            + self.namings.capacity() * size_of::<(u32, Arc<(PartyNaming, Result<String, String>)>)>()
            + self.sent.capacity() * size_of::<(Tag, (Arc<str>, Arc<Observation>))>()
            + self
                .sent
                .values()
                .map(|(name, observation)| name.len() + observation.heap())
                .sum::<usize>()
            + self.targets.heap()
            + self.leaders.heap()
            + self.dirty.capacity() * size_of::<(Tag, u8)>()
    }
}

// Every segment starts cold: its first hour reads every row and tag again; what came before is
// the emit's.
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
        let _cpu = replay_frame::cpu::scope("parties.columnar", Some("parties"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let whole = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at)
            || roster.rebuilt()
            || roster.changed()
            || READ.iter().any(|&table| hour.world.restated(table));
        self.cold = false;
        self.last = Some(at);
        let unavailable =
            !roster.complete() || !columns::complete(state, TableId::PoliticalParties);
        let mut changed = Vec::new();
        if whole {
            self.index(state, roster, at)?;
            self.dirty.clear();
            self.targets.clear();
            self.leaders.clear();
            let previous = std::mem::take(&mut self.sent);
            let tags: Vec<Tag> = roster.held().collect();
            for tag in tags {
                if let Some((name, observation)) = self.observe(state, roster, tag, ALL)? {
                    self.sent
                        .insert(tag, (Arc::clone(&name), Arc::clone(&observation)));
                    changed.push((name, observation));
                }
            }
            drop(previous);
        } else {
            self.follow(state, roster, at)?;
            let dirty: Vec<(Tag, u8)> = self.dirty.drain().collect();
            for (tag, what) in dirty {
                let Some((name, observation)) = self.observe(state, roster, tag, what)? else {
                    continue;
                };
                if self
                    .sent
                    .get(&tag)
                    .is_some_and(|(_, held)| **held == *observation)
                {
                    continue;
                }
                self.sent
                    .insert(tag, (Arc::clone(&name), Arc::clone(&observation)));
                changed.push((name, observation));
            }
        }
        changed.sort_unstable_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
        let facts = (whole || !changed.is_empty()).then(|| Box::new(Facts { whole, changed }));
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            unavailable,
            facts,
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
                "Not a saved parties projector",
            ));
        }
        *self = Self::new(self.format, self.mode)?;
        Ok(())
    }
}
