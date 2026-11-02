//! The parties' emit: each tag's holder, each party's popularity, the ruling party, its name and
//! its first country leader as runs on the family's series publication, changes of government and
//! of leader and elections as events, and each tag's namings, leaders and refusals as its
//! catalogue; and whether the hour's parties were read at all.
//!
//! A run goes on while the tag keeps its holder and the value it reads stays; a new ruling party,
//! name or leader starts a run of its own value, a break where it does not follow the hour
//! before's. An hour not walked, a tag leaving the countries or a value no longer read ends its
//! runs. A change of ruling party or of leader that follows the hour before is a mark; an election
//! is the hour a holder's recorded last election date first differs from the one before, across
//! gaps of the same holder.

use super::facts::{self, Leader, Observation, Parties};
use super::{POPULARITY_DECIMALS, PartyNaming, ScannedLeader};
use dashboard_series::{
    Blob, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A tag's measures: its holder's identity (canonical slot above the raw id); each party's
/// popularity, by the token's string id; the ruling party, its name and its leader ([`ruling`],
/// [`name`], [`leader`]); and the marks of changes of government and of leader, and elections, as
/// events of their own keys.
pub const OWNER: u64 = 0;
pub const POPULARITY: u64 = 1;
pub const RULING: u64 = 2;
pub const NAME: u64 = 3;
pub const LEADER: u64 = 4;
pub const RULING_MARKS: u64 = 5;
pub const LEADER_MARKS: u64 = 6;
pub const ELECTIONS: u64 = 7;
/// The actor of the family's own key: whether each hour's parties were unavailable.
pub const FAMILY: u64 = u64::MAX;
pub const UNAVAILABLE: u64 = 0;
/// Family scalars: every token and leader ideology the runs name, by string id; and each tag's
/// catalogue ([`Meta`], as JSON), by actor.
pub const STRINGS: u64 = 0;
pub const META: u64 = 1 << 20;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds; no event holds fields.
pub mod kind {
    pub const RULING: u64 = 0;
    pub const LEADER: u64 = 1;
    pub const ELECTION: u64 = 2;
}

/// A ruling run's value: its ordinal on the tag above its token's string id.
pub fn ruling(value: i128) -> u32 {
    value as u32
}

/// A name run's value: its ordinal above its naming's index in [`Meta::namings`].
pub fn name(value: i128) -> u32 {
    value as u32
}

/// A leader run's value: its ordinal above its ideology's string id above its person's index
/// in [`Meta::people`] plus one, zero for none.
pub fn leader(value: i128) -> (u32, Option<u32>) {
    let ideology = (value >> 32) as u32;
    let person = (value as u32).checked_sub(1);
    (ideology, person)
}

/// A holder's identity as its OWNER value.
pub fn owner(identity: (u32, u32)) -> i128 {
    (i128::from(identity.0) << 32) | i128::from(identity.1)
}

/// What a tag's replies name beside its runs, in first-seen order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    /// Each ruling party naming, and its selected original tag or why it is unknown.
    pub namings: Vec<PartyNaming>,
    pub selected: Vec<Result<String, String>>,
    /// Each leader, as named and pictured.
    pub people: Vec<ScannedLeader>,
    /// Every leader ideology, empty ones aside.
    pub types: BTreeSet<String>,
    /// Every ideology token that was empty or not UTF-8.
    pub invalid: BTreeSet<Vec<u8>>,
    /// The first hour its rows repeated a token or held two ruling parties, with the first such
    /// token and value.
    pub duplicate: Option<(i32, String, i64)>,
}

/// Each measure's key a tag holds, by measure but popularity's.
const MEASURES: usize = 8;

/// What a tag holds now, and its keys.
#[derive(Default)]
struct Country {
    keys: [Option<KeyHandle>; MEASURES],
    /// Each popularity key, by the token's string id.
    tokens: Vec<(u32, KeyHandle)>,
    /// What it read the hour before, while it is held.
    held: Option<Arc<Observation>>,
    /// The holder it was last read of, and that holder's last election date.
    seen: Option<(u32, u32)>,
    election: Option<i32>,
    /// The ruling, name and leader runs started.
    ordinals: [u64; 3],
    meta: Meta,
    /// Its catalogue changed since the last cut.
    stale: bool,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    countries: Vec<Country>,
    strings: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, u32>,
    /// The strings grew since the last cut.
    spelled: bool,
    unavailable: Option<KeyHandle>,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved parties emit is not valid")
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            countries: Vec::new(),
            strings: Vec::new(),
            ids: HashMap::new(),
            spelled: false,
            unavailable: None,
            last: None,
            first: None,
        }
    }

    fn key(
        &mut self,
        writer: &mut Writer<'_>,
        place: Place,
        decimals: u8,
    ) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, decimals), Some(DOMAIN))
    }

    /// `actor`'s key of `measure`.
    fn measure(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        measure: u64,
    ) -> io::Result<KeyHandle> {
        if let Some(key) = self.countries[actor as usize].keys[measure as usize] {
            return Ok(key);
        }
        let place = Place {
            actor,
            measure,
            dimension: None,
        };
        let key = self.key(writer, place, 0)?;
        self.countries[actor as usize].keys[measure as usize] = Some(key);
        Ok(key)
    }

    /// `actor`'s popularity key of string `id`.
    fn popularity(&mut self, writer: &mut Writer<'_>, actor: u64, id: u32) -> io::Result<KeyHandle> {
        let tokens = &self.countries[actor as usize].tokens;
        let at = tokens.partition_point(|&(held, _)| held < id);
        if let Some(&(held, key)) = tokens.get(at)
            && held == id
        {
            return Ok(key);
        }
        let place = Place {
            actor,
            measure: POPULARITY,
            dimension: Some(u64::from(id)),
        };
        let key = self.key(writer, place, POPULARITY_DECIMALS)?;
        self.countries[actor as usize].tokens.insert(at, (id, key));
        Ok(key)
    }

    /// The string id of `text`.
    fn string(&mut self, text: &Arc<str>) -> io::Result<u32> {
        if let Some(&id) = self.ids.get(text) {
            return Ok(id);
        }
        let id = u32::try_from(self.strings.len())
            .map_err(|_| dashboard_core::invalid("Party strings exceed 32 bits"))?;
        self.strings.push(Arc::clone(text));
        self.ids.insert(Arc::clone(text), id);
        self.spelled = true;
        Ok(id)
    }

    /// The tag actor named `tag`.
    fn country(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(tag.as_bytes()) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let index = actor as usize;
        if index >= self.countries.len() {
            self.countries.resize_with(index + 1, Country::default);
        }
        Ok(actor)
    }

    /// An event of `kind` on `key` at `at`.
    fn event(writer: &mut Writer<'_>, key: KeyHandle, at: i32, kind: u64) -> io::Result<()> {
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: at,
            read_hour: None,
            sequence,
            kind,
            fields: Blob::EMPTY,
        })
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("parties.emit", None);
        let at = hour.hour;
        match self.last {
            Some(last) if last.checked_add(1) == Some(at) => {}
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A parties hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run.
            Some(last) => self.close(writer, last)?,
            None => {}
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        let unavailable = match self.unavailable {
            Some(key) => key,
            None => {
                let place = Place {
                    actor: FAMILY,
                    measure: UNAVAILABLE,
                    dimension: None,
                };
                let key = self.key(writer, place, 0)?;
                self.unavailable = Some(key);
                key
            }
        };
        self.carry.value(
            writer,
            unavailable,
            at,
            Some(i128::from(hour.unavailable)),
            false,
        )?;
        let Some(facts) = hour.facts else {
            return Ok(());
        };
        let facts::Facts { whole, changed } = *facts;
        let mut kept = vec![false; if whole { self.countries.len() } else { 0 }];
        for (tag, observation) in changed {
            let actor = self.country(writer, &tag, at)?;
            if whole {
                if kept.len() <= actor as usize {
                    kept.resize(actor as usize + 1, false);
                }
                kept[actor as usize] = true;
            }
            self.observe(writer, at, actor, observation)?;
        }
        if whole {
            for actor in 0..self.countries.len() {
                if self.countries[actor].held.is_some() && !kept.get(actor).copied().unwrap_or(false)
                {
                    self.end(writer, at, actor as u64)?;
                }
            }
        }
        Ok(())
    }

    /// `actor`'s observation from `at` on.
    fn observe(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        actor: u64,
        now: Arc<Observation>,
    ) -> io::Result<()> {
        let index = actor as usize;
        let before = self.countries[index].held.take();
        // Only the same holder the hour before goes on.
        let continues = before
            .as_ref()
            .is_some_and(|before| before.identity == now.identity);
        let identity = now.identity;
        let moved = self.countries[index].seen != Some(identity);
        let key = self.measure(writer, actor, OWNER)?;
        self.carry
            .value(writer, key, at, Some(owner(identity)), moved)?;
        // Only another holder starts the dates over; a gap of the same holder still dates an
        // election, to the hour it is first seen.
        if moved {
            self.countries[index].election = None;
        }
        if let Some(date) = now.election {
            if self.countries[index]
                .election
                .is_some_and(|previous| previous != date)
            {
                let key = self.measure(writer, actor, ELECTIONS)?;
                Self::event(writer, key, at, kind::ELECTION)?;
            }
            self.countries[index].election = Some(date);
        }
        self.countries[index].seen = Some(identity);
        let held = before.as_deref();
        let same = |left: &Arc<Parties>, right: &Arc<Parties>| {
            Arc::ptr_eq(left, right) || **left == **right
        };
        if !(continues && held.is_some_and(|held| same(&held.parties, &now.parties))) {
            self.parties(
                writer,
                at,
                actor,
                held.map(|held| &*held.parties),
                continues,
                &now.parties,
            )?;
        }
        let leaders = |left: &Option<Arc<Leader>>, right: &Option<Arc<Leader>>| match (left, right) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right) || **left == **right,
            (None, None) => true,
            _ => false,
        };
        if !(continues && held.is_some_and(|held| leaders(&held.leader, &now.leader))) {
            self.leader(
                writer,
                at,
                actor,
                held.and_then(|held| held.leader.as_deref()),
                continues,
                now.leader.as_deref(),
            )?;
        }
        self.countries[index].held = Some(now);
        Ok(())
    }

    /// `actor`'s parties from `at` on: `before` the hour before's, of the same holder when
    /// `continues`.
    fn parties(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        actor: u64,
        before: Option<&Parties>,
        continues: bool,
        now: &Parties,
    ) -> io::Result<()> {
        let index = actor as usize;
        {
            let meta = &mut self.countries[index].meta;
            let mut stale = false;
            for token in &now.invalid {
                stale |= meta.invalid.insert(token.clone());
            }
            if let Some((token, value)) = &now.duplicate
                && meta.duplicate.is_none()
            {
                meta.duplicate = Some((at, token.to_string(), *value));
                stale = true;
            }
            self.countries[index].stale |= stale;
        }
        let going = before.filter(|_| continues);
        for (token, value) in &now.values {
            let was = going.and_then(|before| {
                before
                    .values
                    .iter()
                    .find(|(held, _)| held == token)
                    .map(|&(_, value)| value)
            });
            if was == Some(*value) {
                continue;
            }
            let id = self.string(token)?;
            let key = self.popularity(writer, actor, id)?;
            self.carry
                .value(writer, key, at, Some(i128::from(*value)), was.is_none())?;
        }
        for (token, _) in before.map_or(&[][..], |before| before.values.as_slice()) {
            if now.values.iter().any(|(held, _)| held == token) {
                continue;
            }
            let id = self.string(token)?;
            let key = self.popularity(writer, actor, id)?;
            self.carry.value(writer, key, at, None, false)?;
        }
        let was = going.and_then(|before| before.ruling.as_ref());
        match &now.ruling {
            Some(ruling) => {
                if !was.is_some_and(|was| was.token == ruling.token) {
                    let starts = was.is_none();
                    let id = self.string(&ruling.token)?;
                    let country = &mut self.countries[index];
                    country.ordinals[0] += 1;
                    let value = (i128::from(country.ordinals[0]) << 32) | i128::from(id);
                    let key = self.measure(writer, actor, RULING)?;
                    self.carry.value(writer, key, at, Some(value), starts)?;
                    // A change of government follows the hour before's.
                    if !starts {
                        let key = self.measure(writer, actor, RULING_MARKS)?;
                        Self::event(writer, key, at, kind::RULING)?;
                    }
                }
                let named = was.is_some_and(|was| {
                    Arc::ptr_eq(&was.naming, &ruling.naming) || was.naming == ruling.naming
                });
                if !named {
                    let (naming, selected) = &*ruling.naming;
                    let country = &mut self.countries[index];
                    let meta = &mut country.meta;
                    let at_name = match meta
                        .namings
                        .iter()
                        .zip(&meta.selected)
                        .position(|(held, chosen)| held == naming && chosen == selected)
                    {
                        Some(at_name) => at_name,
                        None => {
                            meta.namings.push(naming.clone());
                            meta.selected.push(selected.clone());
                            country.stale = true;
                            meta.namings.len() - 1
                        }
                    };
                    let at_name = u32::try_from(at_name)
                        .map_err(|_| dashboard_core::invalid("Party namings exceed 32 bits"))?;
                    country.ordinals[1] += 1;
                    let value = (i128::from(country.ordinals[1]) << 32) | i128::from(at_name);
                    let key = self.measure(writer, actor, NAME)?;
                    self.carry.value(writer, key, at, Some(value), false)?;
                }
            }
            None => {
                if before.is_some_and(|before| before.ruling.is_some()) {
                    for measure in [RULING, NAME] {
                        let key = self.measure(writer, actor, measure)?;
                        self.carry.value(writer, key, at, None, false)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// `actor`'s leader from `at` on, as [`Self::parties`] takes its parties.
    fn leader(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        actor: u64,
        before: Option<&Leader>,
        continues: bool,
        now: Option<&Leader>,
    ) -> io::Result<()> {
        let index = actor as usize;
        let Some(now) = now else {
            if before.is_some() {
                let key = self.measure(writer, actor, LEADER)?;
                self.carry.value(writer, key, at, None, false)?;
            }
            return Ok(());
        };
        let was = before.filter(|_| continues);
        if !now.ideology.is_empty() {
            let country = &mut self.countries[index];
            country.stale |= country.meta.types.insert(now.ideology.to_string());
        }
        if was.is_some_and(|was| was.ideology == now.ideology && was.person == now.person) {
            return Ok(());
        }
        let person = match &now.person {
            Some(person) => {
                let country = &mut self.countries[index];
                let people = &mut country.meta.people;
                let at_person = match people.iter().position(|held| held == person) {
                    Some(at_person) => at_person,
                    None => {
                        people.push(person.clone());
                        country.stale = true;
                        people.len() - 1
                    }
                };
                Some(
                    u32::try_from(at_person)
                        .ok()
                        .and_then(|at_person| at_person.checked_add(1))
                        .ok_or_else(|| dashboard_core::invalid("Party leaders exceed 32 bits"))?,
                )
            }
            None => None,
        };
        let id = self.string(&now.ideology)?;
        let starts = was.is_none();
        let country = &mut self.countries[index];
        country.ordinals[2] += 1;
        let value = (i128::from(country.ordinals[2]) << 64)
            | (i128::from(id) << 32)
            | i128::from(person.unwrap_or(0));
        let key = self.measure(writer, actor, LEADER)?;
        self.carry.value(writer, key, at, Some(value), starts)?;
        // A change of leader is a mark only between two characters that follow each other.
        let changed = was
            .and_then(|was| was.person.as_ref())
            .zip(now.person.as_ref())
            .is_some_and(|(was, now)| was.character.id != now.character.id);
        if !starts && changed {
            let key = self.measure(writer, actor, LEADER_MARKS)?;
            Self::event(writer, key, at, kind::LEADER)?;
        }
        Ok(())
    }

    /// Ends what `actor`'s runs hold from `at` on: its tag is held no more.
    fn end(&mut self, writer: &mut Writer<'_>, at: i32, actor: u64) -> io::Result<()> {
        let index = actor as usize;
        let Some(held) = self.countries[index].held.take() else {
            return Ok(());
        };
        let mut keys: Vec<KeyHandle> = [OWNER, RULING, NAME, LEADER]
            .into_iter()
            .filter_map(|measure| self.countries[index].keys[measure as usize])
            .collect();
        for (token, _) in &held.parties.values {
            if let Some(&id) = self.ids.get(token) {
                let tokens = &self.countries[index].tokens;
                if let Ok(at_token) = tokens.binary_search_by_key(&id, |&(held, _)| held) {
                    keys.push(tokens[at_token].1);
                }
            }
        }
        for key in keys {
            self.carry.value(writer, key, at, None, false)?;
        }
        Ok(())
    }

    /// Ends every run after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        for country in &mut self.countries {
            country.held = None;
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the catalogues that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        if self.spelled {
            let strings = Scalar::List(
                self.strings
                    .iter()
                    .map(|text| Scalar::Text(text.as_bytes().into()))
                    .collect(),
            );
            let cell = family::cell(writer, &strings)?;
            writer.catalogue(STRINGS, cell)?;
            self.spelled = false;
        }
        for actor in 0..self.countries.len() {
            if !self.countries[actor].stale {
                continue;
            }
            let json = serde_json::to_vec(&self.countries[actor].meta)?;
            let cell = family::cell(writer, &Scalar::Text(json.into_boxed_slice()))?;
            writer.catalogue(META + actor as u64, cell)?;
            self.countries[actor].stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection reads every tag anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            countries: self
                .countries
                .iter()
                .enumerate()
                .filter(|(_, country)| country.seen.is_some() || country.held.is_some())
                .map(|(actor, country)| SavedCountry {
                    actor: actor as u64,
                    held: country.held.clone(),
                    seen: country.seen,
                    election: country.election,
                    ordinals: country.ordinals,
                })
                .collect(),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        for scalar in directory.scalar(STRINGS).map_or(&[][..], Scalar::list) {
            let text = std::str::from_utf8(scalar.text().ok_or_else(invalid)?)
                .map_err(|_| invalid())?;
            let text: Arc<str> = Arc::from(text);
            emit.ids.insert(Arc::clone(&text), emit.strings.len() as u32);
            emit.strings.push(text);
        }
        emit.countries
            .resize_with(directory.actors.len(), Country::default);
        for (actor, country) in emit.countries.iter_mut().enumerate() {
            let actor = actor as u64;
            for measure in 0..MEASURES as u64 {
                if measure != POPULARITY {
                    country.keys[measure as usize] = directory.key(actor, measure, None);
                }
            }
            country.tokens = directory
                .dimensions(actor, POPULARITY)
                .iter()
                .map(|&(id, key)| {
                    u32::try_from(id)
                        .map(|id| (id, key))
                        .map_err(|_| invalid())
                })
                .collect::<io::Result<_>>()?;
            if let Some(scalar) = directory.scalar(META + actor) {
                country.meta = serde_json::from_slice(scalar.text().ok_or_else(invalid)?)?;
            }
        }
        emit.unavailable = directory.key(FAMILY, UNAVAILABLE, None);
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        for country in saved.countries {
            let held = emit
                .countries
                .get_mut(country.actor as usize)
                .ok_or_else(invalid)?;
            held.held = country.held;
            held.seen = country.seen;
            held.election = country.election;
            held.ordinals = country.ordinals;
        }
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedCountry {
    actor: u64,
    held: Option<Arc<Observation>>,
    seen: Option<(u32, u32)>,
    election: Option<i32>,
    ordinals: [u64; 3],
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    countries: Vec<SavedCountry>,
}
