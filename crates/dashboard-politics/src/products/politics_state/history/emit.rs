//! The political state's emit: each holder's values as runs on the family's series publication,
//! a holder (canonical slot) an actor named by its slot, a value holding from the first read
//! showing it to the first read that does not, a gap ending it at its last read.
//!
//! A holder's keys: its election schedule, its exile, its balances by balance identity (the
//! dimension, a token's id), the balance shown it (a token's id), its civil-war rivals by rival
//! slot, the civil-war predicate and, for a holder ever listed, its existence. The family's own
//! keys: the hours each part was read (by part), each refusal's count per hour it held (by
//! reason, so a reply sums its hours) and each holder's tags as events. Tokens and reasons are
//! family scalars.

use super::{
    Field, PART_SLOT,
    facts::{self, Sent},
    observe::Val,
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;

/// A holder's keys.
pub const ELECTIONS_HELD: u64 = 0;
pub const ELECTIONS_REFUSED: u64 = 1;
pub const ELECTION_INTERVAL: u64 = 2;
pub const NEXT_ELECTION: u64 = 3;
pub const LAST_ELECTION: u64 = 4;
pub const EXILE_HOST: u64 = 5;
pub const EXILE_LEGITIMACY: u64 = 6;
pub const EXILE_MANPOWER: u64 = 7;
pub const EXILE_CAP: u64 = 8;
pub const EXILE_LEADERS: u64 = 9;
pub const EXILE_READ: u64 = 10;
/// By balance identity: the value, active range, trending side and stored record position.
pub const BALANCE_VALUE: u64 = 11;
pub const BALANCE_RANGE: u64 = 12;
pub const BALANCE_TRENDING: u64 = 13;
pub const BALANCE_RECORD: u64 = 14;
pub const BALANCE_SHOWN: u64 = 15;
/// By rival slot.
pub const CIVIL_WAR_RIVAL: u64 = 16;
pub const CIVIL_WAR: u64 = 17;
pub const EXISTS: u64 = 18;

/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// By part.
pub const KNOWN: u64 = 0;
/// By reason.
pub const REFUSALS: u64 = 1;
/// [`kind::TAG`] events.
pub const TAGS: u64 = 2;

/// Family scalars: each token's text and each reason's, by id.
pub const TOKENS: u64 = 0;
pub const REASONS: u64 = 1 << 30;

pub mod kind {
    /// `[Unsigned slot, Text tag]`: the holder's tag from the event's hour.
    pub const TAG: u64 = 0;
}

/// Every key's evidence: each hour the emit was handed is known.
const DOMAIN: u64 = 0;

/// Why an hour cannot say which listed holder exists.
const UNKNOWN_EXISTENCE: &str = "civil-war side existence not recorded";

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Saved political state emit is not valid",
    )
}

/// The name of the actor of holder `slot`.
pub fn holder_name(slot: u32) -> String {
    slot.to_string()
}

/// The holder an actor is named for.
pub fn holder_of(name: &[u8]) -> Option<u32> {
    std::str::from_utf8(name).ok()?.parse().ok()
}

/// What the emit goes on with after a cut, beside its open edges.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    tokens: Vec<String>,
    reasons: Vec<String>,
    /// Each part's keys holding a value.
    held: [Vec<u64>; 4],
    tags: Vec<(u32, String)>,
    listed: Vec<u32>,
    existence: Option<Vec<(u32, bool)>>,
    exists: Vec<u64>,
    refusals: Vec<(String, u64)>,
    refused: Vec<u64>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    last: Option<i32>,
    /// The first hour handed since the domain's evidence was last written.
    first: Option<i32>,
    tokens: HashMap<Box<str>, u64>,
    spelled: Vec<String>,
    reasons: HashMap<String, u64>,
    stated: Vec<String>,
    held: [HashSet<KeyHandle>; 4],
    /// Each holder's tag as last written.
    tags: HashMap<u32, Box<str>>,
    /// Every holder a civil-war list has named.
    listed: BTreeSet<u32>,
    existence: Option<HashMap<u32, bool>>,
    exists: HashSet<KeyHandle>,
    /// The operator's refusals holding, and the refusal keys open.
    refusals: BTreeMap<String, u64>,
    refused: HashSet<KeyHandle>,
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
            last: None,
            first: None,
            tokens: HashMap::new(),
            spelled: Vec::new(),
            reasons: HashMap::new(),
            stated: Vec::new(),
            held: Default::default(),
            tags: HashMap::new(),
            listed: BTreeSet::new(),
            existence: None,
            exists: HashSet::new(),
            refusals: BTreeMap::new(),
            refused: HashSet::new(),
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        match self.actors.held(place) {
            Some(key) => Ok(key),
            None => self
                .actors
                .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN)),
        }
    }

    /// The id of `token`, catalogued when new.
    fn token(&mut self, writer: &mut Writer<'_>, token: &str) -> io::Result<u64> {
        if let Some(&id) = self.tokens.get(token) {
            return Ok(id);
        }
        let id = self.spelled.len() as u64;
        let text = family::cell(writer, &Scalar::Text(token.as_bytes().into()))?;
        writer.catalogue(TOKENS + id, text)?;
        self.tokens.insert(token.into(), id);
        self.spelled.push(token.to_owned());
        Ok(id)
    }

    /// The id of `reason`, catalogued when new.
    fn reason(&mut self, writer: &mut Writer<'_>, reason: &str) -> io::Result<u64> {
        if let Some(&id) = self.reasons.get(reason) {
            return Ok(id);
        }
        let id = self.stated.len() as u64;
        let text = family::cell(writer, &Scalar::Text(reason.as_bytes().into()))?;
        writer.catalogue(REASONS + id, text)?;
        self.reasons.insert(reason.to_owned(), id);
        self.stated.push(reason.to_owned());
        Ok(id)
    }

    fn holder(&mut self, writer: &mut Writer<'_>, slot: u32, hour: i32) -> io::Result<u64> {
        let name = holder_name(slot);
        self.actors.actor(writer, name.as_bytes(), hour)
    }

    /// The key `field` of `slot` is held in.
    fn place(
        &mut self,
        writer: &mut Writer<'_>,
        (slot, field): &(u32, Field),
        hour: i32,
    ) -> io::Result<KeyHandle> {
        if *slot == PART_SLOT {
            let Field::Known(part) = field else {
                return Err(dashboard_core::invalid("A political part key names no part"));
            };
            return self.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure: KNOWN,
                    dimension: Some(*part as u64),
                },
            );
        }
        let (measure, dimension) = match field {
            Field::ElectionsHeld => (ELECTIONS_HELD, None),
            Field::ElectionsRefused => (ELECTIONS_REFUSED, None),
            Field::ElectionInterval => (ELECTION_INTERVAL, None),
            Field::NextElection => (NEXT_ELECTION, None),
            Field::LastElection => (LAST_ELECTION, None),
            Field::ExileHost => (EXILE_HOST, None),
            Field::ExileLegitimacy => (EXILE_LEGITIMACY, None),
            Field::ExileManpower => (EXILE_MANPOWER, None),
            Field::ExileCap => (EXILE_CAP, None),
            Field::ExileLeaders => (EXILE_LEADERS, None),
            Field::ExileRead => (EXILE_READ, None),
            Field::BalanceValue(balance) => (BALANCE_VALUE, Some(self.token(writer, balance)?)),
            Field::BalanceRange(balance) => (BALANCE_RANGE, Some(self.token(writer, balance)?)),
            Field::BalanceTrending(balance) => {
                (BALANCE_TRENDING, Some(self.token(writer, balance)?))
            }
            Field::BalanceRecord(balance) => (BALANCE_RECORD, Some(self.token(writer, balance)?)),
            Field::BalanceShown => (BALANCE_SHOWN, None),
            Field::CivilWarRival(rival) => (CIVIL_WAR_RIVAL, Some(u64::from(*rival))),
            Field::CivilWar => (CIVIL_WAR, None),
            Field::Exists => (EXISTS, None),
            Field::Known(_) => {
                return Err(dashboard_core::invalid("A holder key names a part"));
            }
        };
        let actor = self.holder(writer, *slot, hour)?;
        self.key(
            writer,
            Place {
                actor,
                measure,
                dimension,
            },
        )
    }

    fn value(&mut self, writer: &mut Writer<'_>, value: &Val) -> io::Result<i128> {
        Ok(match value {
            Val::Int(value) => *value,
            Val::Token(token) => i128::from(self.token(writer, token)?),
        })
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        if let Some(last) = self.last
            && last.checked_add(1) != Some(at)
        {
            // An unread hour ends every run; the next hour's reads start them anew.
            self.carry.close(writer, last)?;
            self.known(writer, last)?;
            self.held = Default::default();
            self.exists.clear();
            self.refused.clear();
            self.existence = None;
            self.refusals.clear();
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        let Some(facts) = hour.facts else {
            return Ok(());
        };
        let facts = *facts;
        for (slot, tag) in facts.tags {
            if self.tags.get(&slot).is_some_and(|held| **held == *tag) {
                continue;
            }
            let key = self.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure: TAGS,
                    dimension: None,
                },
            )?;
            let text = writer.text(&tag)?;
            let fields = writer.cells([Cell::Unsigned(u64::from(slot)), Cell::Text(text)])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key,
                native_hour: at,
                read_hour: Some(at),
                sequence,
                kind: kind::TAG,
                fields,
            })?;
            self.tags.insert(slot, Box::from(&*tag));
        }
        let mut listed = false;
        for sent in facts.parts {
            listed |= self.part(writer, at, sent)?;
        }
        let existence = facts.existence.is_some();
        if let Some(sent) = facts.existence {
            self.existence = sent.map(|existence| (*existence).clone());
        }
        if listed || existence {
            self.exist(writer, at)?;
        }
        let refused = facts.refusals.is_some();
        if let Some(refusals) = facts.refusals {
            self.refusals = refusals.into_iter().collect();
        }
        if listed || existence || refused {
            self.refuse(writer, at)?;
        }
        Ok(())
    }

    /// A part's values from `at`; whether it listed a holder never listed before.
    fn part(&mut self, writer: &mut Writer<'_>, at: i32, sent: Sent) -> io::Result<bool> {
        let index = sent.part as usize;
        let mut listed = false;
        let mut now = HashSet::new();
        for (place, value) in &sent.values {
            let key = self.place(writer, place, at)?;
            match value {
                Some(value) => {
                    let value = self.value(writer, value)?;
                    self.carry.value(writer, key, at, Some(value), false)?;
                    if let (slot, Field::CivilWarRival(rival)) = place {
                        listed |= self.listed.insert(*slot);
                        listed |= self.listed.insert(*rival);
                    }
                    if sent.whole {
                        now.insert(key);
                    } else {
                        self.held[index].insert(key);
                    }
                }
                None => {
                    self.carry.value(writer, key, at, None, false)?;
                    self.held[index].remove(&key);
                }
            }
        }
        if sent.whole {
            let ended: Vec<KeyHandle> = self.held[index].difference(&now).copied().collect();
            for key in ended {
                self.carry.value(writer, key, at, None, false)?;
            }
            self.held[index] = now;
        }
        Ok(listed)
    }

    /// Whether each holder ever listed exists from `at`: none where the hour cannot say.
    fn exist(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        let Some(existence) = &self.existence else {
            let ended: Vec<KeyHandle> = self.exists.drain().collect();
            for key in ended {
                self.carry.value(writer, key, at, None, false)?;
            }
            return Ok(());
        };
        let existing: Vec<u32> = self
            .listed
            .iter()
            .copied()
            .filter(|slot| existence.get(slot) == Some(&true))
            .collect();
        let mut now = HashSet::with_capacity(existing.len());
        for slot in existing {
            let key = self.place(writer, &(slot, Field::Exists), at)?;
            self.carry.value(writer, key, at, Some(1), false)?;
            now.insert(key);
        }
        let ended: Vec<KeyHandle> = self.exists.difference(&now).copied().collect();
        for key in ended {
            self.carry.value(writer, key, at, None, false)?;
        }
        self.exists = now;
        Ok(())
    }

    /// Each refusal holding from `at`, with its count.
    fn refuse(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        let mut now = self.refusals.clone();
        if let Some(existence) = &self.existence
            && self.listed.iter().any(|slot| !existence.contains_key(slot))
        {
            *now.entry(UNKNOWN_EXISTENCE.to_owned()).or_default() += 1;
        }
        let mut open = HashSet::with_capacity(now.len());
        for (reason, count) in now {
            let id = self.reason(writer, &reason)?;
            let key = self.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure: REFUSALS,
                    dimension: Some(id),
                },
            )?;
            self.carry
                .value(writer, key, at, Some(i128::from(count)), false)?;
            open.insert(key);
        }
        let ended: Vec<KeyHandle> = self.refused.difference(&open).copied().collect();
        for key in ended {
            self.carry.value(writer, key, at, None, false)?;
        }
        self.refused = open;
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.known(writer, last)
    }

    /// Writes the hours handed since the domain's evidence was last written, through `last`, as
    /// known: a run must lie within its key's evidence.
    fn known(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        let Some(first) = self.first.take() else {
            return Ok(());
        };
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
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection hands every part on whole.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let sorted = |keys: &HashSet<KeyHandle>| {
            let mut keys: Vec<u64> = keys.iter().copied().collect();
            keys.sort_unstable();
            keys
        };
        let mut tags: Vec<(u32, String)> = self
            .tags
            .iter()
            .map(|(&slot, tag)| (slot, tag.to_string()))
            .collect();
        tags.sort_unstable();
        let saved = Saved {
            last: self.last,
            tokens: self.spelled.clone(),
            reasons: self.stated.clone(),
            held: std::array::from_fn(|part| sorted(&self.held[part])),
            tags,
            listed: self.listed.iter().copied().collect(),
            existence: self.existence.as_ref().map(|existence| {
                let mut existence: Vec<(u32, bool)> =
                    existence.iter().map(|(&slot, &exists)| (slot, exists)).collect();
                existence.sort_unstable();
                existence
            }),
            exists: sorted(&self.exists),
            refusals: self
                .refusals
                .iter()
                .map(|(reason, &count)| (reason.clone(), count))
                .collect(),
            refused: sorted(&self.refused),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn resume(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        for (id, token) in saved.tokens.iter().enumerate() {
            emit.tokens.insert(token.as_str().into(), id as u64);
        }
        emit.spelled = saved.tokens;
        for (id, reason) in saved.reasons.iter().enumerate() {
            emit.reasons.insert(reason.clone(), id as u64);
        }
        emit.stated = saved.reasons;
        emit.held = saved.held.map(|keys| keys.into_iter().collect());
        emit.tags = saved
            .tags
            .into_iter()
            .map(|(slot, tag)| (slot, tag.into_boxed_str()))
            .collect();
        emit.listed = saved.listed.into_iter().collect();
        emit.existence = saved.existence.map(|existence| existence.into_iter().collect());
        emit.exists = saved.exists.into_iter().collect();
        emit.refusals = saved.refusals.into_iter().collect();
        emit.refused = saved.refused.into_iter().collect();
        Ok(emit)
    }
}
