//! Game score's emit: each country's score parts as runs on the family's series publication,
//! with whether it was observed and its identity, missing reasons and continuity as an event
//! where they change; the global multipliers alike; and the sort hints and every warning's
//! observations as a family scalar.
//!
//! A country's runs go on while it keeps its native holder from one observed hour to the next;
//! a holder change, a gap and a country leaving the hour start or end them. A holder's span
//! runs while any of its countries' totals is known.

use super::facts::{self, Facts, Warning};
use super::{GameScoreIdentity, GameScoreMissing, GameScoreValues, GameScoreWarning, ScoreContent};
use dashboard_base::products::country_groups::HourSpan;
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// Each actor's measures: whether it was observed (1), its detail events, then each score part
/// in [`super::PARTS`] order.
pub const PRESENT: u64 = 0;
pub const PART: u64 = 1;
/// The actor of the multipliers' keys.
pub const FAMILY: u64 = u64::MAX;
/// The family scalar holding [`Metadata`] as JSON text.
pub const METADATA: u64 = 0;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
/// The score parts' fixed5 scale.
pub const DECIMALS: u8 = 5;

/// Event kinds.
pub mod kind {
    /// `[Text detail]`: a [`super::Detail`] as JSON, on an actor's PRESENT key.
    pub const DETAIL: u64 = 0;
}

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// What a reply reads beside the values: the sort hints, every warning with its observations
/// and the content's warnings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub hints: BTreeMap<String, String>,
    pub warnings: Vec<GameScoreWarning>,
    pub content_warnings: Vec<String>,
}

/// An actor's identity, missing reasons and continuity at an observed hour; none for the
/// multipliers' identity and continuity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detail {
    pub identity: Option<GameScoreIdentity>,
    pub missing: [Option<GameScoreMissing>; 6],
    pub read_each: [bool; 6],
    pub stretch: Option<String>,
}

/// Each country's native holder and whether its total is known, at the observed hour.
#[derive(Default)]
struct Holders {
    /// Each country's holder key, when found, and whether its total is known.
    tags: HashMap<String, (Option<String>, bool)>,
    /// Countries by holder key.
    countries: HashMap<String, Vec<String>>,
    /// Countries of each holder whose total is known.
    live: HashMap<String, usize>,
}

impl Holders {
    fn insert(&mut self, tag: &str, holder: Option<String>, known: bool) {
        if let Some(holder) = &holder {
            self.countries
                .entry(holder.clone())
                .or_default()
                .push(tag.to_owned());
            if known {
                *self.live.entry(holder.clone()).or_default() += 1;
            }
        }
        self.tags.insert(tag.to_owned(), (holder, known));
    }

    /// Whether any of the holder's countries has a known total.
    fn is_live(&self, holder: &str) -> bool {
        self.live.get(holder).is_some_and(|count| *count > 0)
    }
}

/// Observed hours, and each present warning's per-hour count with the hour count through which
/// it was added to the observations.
#[derive(Default)]
struct Counting {
    hours: u64,
    counts: BTreeMap<Warning, (u64, u64)>,
}

/// The native holder a country's continuity is keyed by.
fn holder(identity: &GameScoreIdentity) -> io::Result<Option<String>> {
    match (identity.canonical_slot, identity.native_country) {
        (Some(slot), Some(country)) => Ok(Some(serde_json::to_string(&(slot, country))?)),
        _ => Ok(None),
    }
}

fn same_holder(before: Option<&GameScoreIdentity>, identity: &GameScoreIdentity) -> bool {
    before.is_some_and(|before| {
        before.canonical_slot == identity.canonical_slot
            && before.native_country == identity.native_country
    })
}

/// Adds observations of one warning, keeping them in reason order.
fn add_observations(
    warnings: &mut Vec<GameScoreWarning>,
    (kind, reason): &Warning,
    observations: u64,
) -> bool {
    if observations == 0 {
        return false;
    }
    match warnings.binary_search_by(|warning| (&warning.kind, &warning.reason).cmp(&(kind, reason)))
    {
        Ok(index) => warnings[index].observations += observations,
        Err(index) => warnings.insert(
            index,
            GameScoreWarning {
                kind: kind.clone(),
                reason: reason.clone(),
                observations,
            },
        ),
    }
    true
}

/// What the emit goes on with after a cut, beside its open runs.
#[derive(Default, Serialize, Deserialize)]
struct State {
    metadata: Metadata,
    /// Each actor's last detail and the hour it was observed or dated to.
    last: BTreeMap<u64, (i32, Detail)>,
    /// Each holder's span of known totals.
    entities: BTreeMap<String, HourSpan>,
    /// Each country's stretch: its first hour with its holder, the first of its known totals
    /// since, and the hour it was last dated to.
    stretches: BTreeMap<u64, (i32, Option<i32>, i32)>,
    /// The countries of the observed hour.
    active: BTreeSet<u64>,
    /// The last hour kept, and the last observed.
    frontier: Option<i32>,
    observed: Option<i32>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// Each country's keys (PRESENT, then each part), by actor, and the multipliers'.
    keys: Vec<[KeyHandle; 7]>,
    family: Option<[KeyHandle; 7]>,
    state: State,
    /// Every country's facts at the observed hour, with their holders, read anew at each
    /// whole hour.
    current: BTreeMap<String, (GameScoreValues, GameScoreIdentity)>,
    holders: Option<Holders>,
    counting: Option<Counting>,
    /// The metadata changed since the last cut.
    stale: bool,
    /// The first hour since the last cut.
    first: Option<i32>,
}

impl Emit {
    pub fn new(content: &ScoreContent) -> Self {
        let mut emit = Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            keys: Vec::new(),
            family: None,
            state: State::default(),
            current: BTreeMap::new(),
            holders: None,
            counting: None,
            stale: true,
            first: None,
        };
        emit.state.metadata.hints = content.hints.clone();
        emit.state.metadata.content_warnings = content.naming.warnings.clone();
        for (key, count) in &content.hint_warnings {
            add_observations(&mut emit.state.metadata.warnings, key, *count);
        }
        emit
    }

    /// The keys of `actor`, interned when new.
    fn interned(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
    ) -> io::Result<[KeyHandle; 7]> {
        let mut keys = [0; 7];
        for (measure, key) in keys.iter_mut().enumerate() {
            let measure = measure as u64;
            let decimals = if measure == PRESENT { 0 } else { DECIMALS };
            *key = self.actors.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: None,
                },
                (RunMode::Exact, decimals),
                Some(DOMAIN),
            )?;
        }
        Ok(keys)
    }

    /// The country actor named `tag`, with its keys.
    fn actor(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(tag.as_bytes()) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let keys = self.interned(writer, actor)?;
        let index = usize::try_from(actor).map_err(|_| invalid("Score actor out of range"))?;
        if index >= self.keys.len() {
            self.keys.resize(index + 1, [0; 7]);
        }
        self.keys[index] = keys;
        Ok(actor)
    }

    fn keys(&mut self, writer: &mut Writer<'_>, actor: u64) -> io::Result<[KeyHandle; 7]> {
        if actor != FAMILY {
            return self
                .keys
                .get(actor as usize)
                .copied()
                .ok_or_else(|| invalid("Score actor has no keys"));
        }
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let keys = self.interned(writer, FAMILY)?;
        self.family = Some(keys);
        Ok(keys)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("game_score.emit", None);
        let at = hour.hour;
        let continuous = match self.state.frontier {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(invalid("A game score hour does not follow the hour before"));
            }
            // An hour not walked ends every run.
            Some(last) => {
                self.carry.close(writer, last)?;
                false
            }
            None => false,
        };
        self.state.frontier = Some(at);
        self.first.get_or_insert(at);
        let Some(facts) = hour.facts else {
            if !continuous || self.counting.is_none() {
                return Err(invalid("A game score hour without facts does not go on"));
            }
            self.counted();
            return Ok(());
        };
        match *facts {
            Facts::Whole {
                multipliers,
                countries,
                warnings,
            } => {
                self.count_all(warnings);
                self.current = countries
                    .into_iter()
                    .map(|(tag, values, identity)| (tag, (values, identity)))
                    .collect();
                self.observe_all(writer, at, &multipliers, continuous)
            }
            Facts::Countries {
                countries,
                warnings,
            } => {
                if !continuous || self.holders.is_none() || self.counting.is_none() {
                    return Err(invalid("Changed game score countries do not go on"));
                }
                self.count_changed(warnings);
                let mut tags = Vec::with_capacity(countries.len());
                for (tag, values, identity) in countries {
                    let held = self
                        .current
                        .get_mut(&tag)
                        .ok_or_else(|| invalid("Changed score country is not observed"))?;
                    *held = (values, identity);
                    tags.push(tag);
                }
                self.observe_countries(writer, at, &tags)
            }
        }
    }

    /// Every country, as at a gap, a new country set or a whole read.
    fn observe_all(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        multipliers: &GameScoreValues,
        continuous: bool,
    ) -> io::Result<()> {
        self.normalize();
        let previous = continuous.then_some(self.state.observed).flatten();
        let current = std::mem::take(&mut self.current);
        let result = self.observe_current(writer, hour, multipliers, previous, &current);
        self.current = current;
        result?;
        self.state.observed = Some(hour);
        Ok(())
    }

    fn observe_current(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        multipliers: &GameScoreValues,
        previous: Option<i32>,
        current: &BTreeMap<String, (GameScoreValues, GameScoreIdentity)>,
    ) -> io::Result<()> {
        let mut holders = Holders::default();
        for (tag, (values, identity)) in current {
            let entity = holder(identity)?;
            let known = values.total.raw.is_some();
            if known && let Some(entity) = &entity {
                let span = self
                    .state
                    .entities
                    .entry(entity.clone())
                    .or_insert(HourSpan {
                        first_hour: hour,
                        last_hour: hour,
                    });
                if span.last_hour != hour {
                    if Some(span.last_hour) != previous {
                        span.first_hour = hour;
                    }
                    span.last_hour = hour;
                }
            }
            holders.insert(tag, entity, known);
        }
        let boundary = self
            .state
            .last
            .get(&FAMILY)
            .is_none_or(|(last, _)| Some(*last) != previous);
        self.values(writer, FAMILY, hour, multipliers, None, None, boundary)?;
        let mut active = BTreeSet::new();
        for (tag, (values, identity)) in current {
            let actor = self.actor(writer, tag, hour)?;
            active.insert(actor);
            let same = self.state.last.get(&actor).is_some_and(|(last, before)| {
                Some(*last) == previous && same_holder(before.identity.as_ref(), identity)
            });
            let entity = holders
                .tags
                .get(tag)
                .and_then(|(entity, _)| entity.as_deref());
            self.observe_country(writer, actor, hour, values, identity, same, entity)?;
        }
        let gone: Vec<u64> = self.state.active.difference(&active).copied().collect();
        for actor in gone {
            for key in self.keys(writer, actor)? {
                self.carry.value(writer, key, hour, None, false)?;
            }
        }
        self.state.active = active;
        self.holders = Some(holders);
        Ok(())
    }

    /// Only the countries whose facts changed, and those sharing a holder whose total became
    /// known or unknown. The countries present, their holders and the multipliers are the
    /// observed hour's.
    fn observe_countries(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        tags: &[String],
    ) -> io::Result<()> {
        let mut holders = self.holders.take().expect("holders of the observed hour");
        let current = std::mem::take(&mut self.current);
        let result = self.observe_holders(writer, hour, tags, &mut holders, &current);
        self.holders = Some(holders);
        self.current = current;
        result?;
        self.state.observed = Some(hour);
        Ok(())
    }

    fn observe_holders(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        tags: &[String],
        holders: &mut Holders,
        current: &BTreeMap<String, (GameScoreValues, GameScoreIdentity)>,
    ) -> io::Result<()> {
        let country = |tag: &str| {
            current
                .get(tag)
                .ok_or_else(|| invalid("Changed score country is not observed"))
        };
        // Whether each holder whose known totals changed was live before.
        let mut before = BTreeMap::<String, bool>::new();
        for tag in tags {
            let known = country(tag)?.0.total.raw.is_some();
            let (entity, was) = holders
                .tags
                .get_mut(tag)
                .ok_or_else(|| invalid("Changed score country has no holder"))?;
            if *was == known {
                continue;
            }
            *was = known;
            if let Some(entity) = entity {
                let count = holders.live.entry(entity.clone()).or_default();
                before.entry(entity.clone()).or_insert(*count > 0);
                if known {
                    *count += 1;
                } else {
                    *count -= 1;
                }
            }
        }
        let flipped: BTreeSet<String> = before
            .into_iter()
            .filter(|(entity, was)| *was != holders.is_live(entity))
            .map(|(entity, _)| entity)
            .collect();
        let mut dirty: BTreeSet<&str> = tags.iter().map(String::as_str).collect();
        for entity in &flipped {
            dirty.extend(
                holders
                    .countries
                    .get(entity)
                    .into_iter()
                    .flatten()
                    .map(String::as_str),
            );
        }
        // A holder's span runs while any of its countries' totals is known, and ended at the
        // previous observed hour when none is any longer.
        for entity in &flipped {
            if !holders.is_live(entity)
                && let (Some(span), Some(observed)) =
                    (self.state.entities.get_mut(entity), self.state.observed)
            {
                span.last_hour = observed;
            }
        }
        for tag in &dirty {
            if let Some((Some(entity), _)) = holders.tags.get(*tag)
                && holders.is_live(entity)
            {
                let span = self
                    .state
                    .entities
                    .entry(entity.clone())
                    .or_insert(HourSpan {
                        first_hour: hour,
                        last_hour: hour,
                    });
                if flipped.contains(entity) {
                    span.first_hour = hour;
                }
                span.last_hour = hour;
            }
        }
        for tag in dirty {
            let (values, identity) = country(tag)?;
            let actor = self
                .actors
                .find(tag.as_bytes())
                .ok_or_else(|| invalid("Changed score country has no key"))?;
            let same = self.state.active.contains(&actor)
                && self
                    .state
                    .last
                    .get(&actor)
                    .is_some_and(|(_, before)| same_holder(before.identity.as_ref(), identity));
            let entity = holders
                .tags
                .get(tag)
                .and_then(|(entity, _)| entity.as_deref());
            self.observe_country(writer, actor, hour, values, identity, same, entity)?;
        }
        Ok(())
    }

    /// One country's stretches and values. `same` continues its identity from the previous
    /// observed hour.
    #[allow(clippy::too_many_arguments)]
    fn observe_country(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        hour: i32,
        values: &GameScoreValues,
        identity: &GameScoreIdentity,
        same: bool,
        entity: Option<&str>,
    ) -> io::Result<()> {
        let current = self
            .state
            .stretches
            .entry(actor)
            .or_insert((hour, None, hour));
        if !same {
            current.0 = hour;
            current.1 = None;
        }
        if values.total.raw.is_some() {
            current.1.get_or_insert(hour);
        } else {
            current.1 = None;
        }
        current.2 = hour;
        let stretch = match entity {
            Some(key) => serde_json::to_string(&(
                key,
                self.state
                    .entities
                    .get(key)
                    .filter(|span| span.last_hour == hour)
                    .map(|span| span.first_hour),
            ))?,
            None => serde_json::to_string(&(current.0, current.1))?,
        };
        self.values(
            writer,
            actor,
            hour,
            values,
            Some(identity.clone()),
            Some(stretch),
            !same,
        )
    }

    /// Dates every retained hour marker at the observed hour, as observing every country then
    /// would: active countries, the multipliers and live holders. Between whole observations
    /// only changed countries' markers move.
    fn normalize(&mut self) {
        let (Some(observed), Some(holders)) = (self.state.observed, &self.holders) else {
            return;
        };
        for actor in self.state.active.iter().copied().chain([FAMILY]) {
            if let Some((last, _)) = self.state.last.get_mut(&actor) {
                *last = observed;
            }
        }
        for actor in &self.state.active {
            if let Some(stretch) = self.state.stretches.get_mut(actor) {
                stretch.2 = observed;
            }
        }
        for (entity, count) in &holders.live {
            if *count > 0
                && let Some(span) = self.state.entities.get_mut(entity)
            {
                span.last_hour = observed;
            }
        }
    }

    /// An hour whose warnings are the hour before's: each counts once more.
    fn counted(&mut self) {
        if let Some(counting) = &mut self.counting {
            counting.hours += 1;
        }
    }

    /// A whole hour's warnings, each with its count: what was counted before is added.
    fn count_all(&mut self, warnings: Vec<(Warning, u64)>) {
        self.add_counted();
        let hours = self.counting.as_ref().map_or(0, |counting| counting.hours);
        self.counting = Some(Counting {
            hours,
            counts: warnings
                .into_iter()
                .map(|(key, count)| (key, (count, hours)))
                .collect(),
        });
        self.counted();
    }

    /// The warnings whose counts an hour changed, none for one no longer counted.
    fn count_changed(&mut self, warnings: Vec<(Warning, Option<u64>)>) {
        let counting = self.counting.as_mut().expect("counted warnings");
        for (key, count) in warnings {
            if let Some((held, since)) = counting.counts.remove(&key) {
                self.stale |= add_observations(
                    &mut self.state.metadata.warnings,
                    &key,
                    held * (counting.hours - since),
                );
            }
            if let Some(count) = count {
                counting.counts.insert(key, (count, counting.hours));
            }
        }
        counting.hours += 1;
    }

    /// Adds every held warning's hours since it was last added.
    fn add_counted(&mut self) {
        let Some(counting) = &mut self.counting else {
            return;
        };
        for (key, (count, since)) in &mut counting.counts {
            if counting.hours > *since {
                self.stale |= add_observations(
                    &mut self.state.metadata.warnings,
                    key,
                    *count * (counting.hours - *since),
                );
                *since = counting.hours;
            }
        }
    }

    /// `actor`'s detail and values from `hour` on: the detail is an event where it changed or
    /// a run starts (`boundary`).
    #[allow(clippy::too_many_arguments)]
    fn values(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        hour: i32,
        values: &GameScoreValues,
        identity: Option<GameScoreIdentity>,
        stretch: Option<String>,
        boundary: bool,
    ) -> io::Result<()> {
        let keys = self.keys(writer, actor)?;
        let amounts = values.amounts();
        let detail = Detail {
            identity,
            missing: amounts.map(|amount| amount.missing.clone()),
            read_each: amounts.map(|amount| amount.read_each_hour),
            stretch,
        };
        if boundary
            || self
                .state
                .last
                .get(&actor)
                .is_none_or(|(_, before)| before != &detail)
        {
            let text = writer.text(&serde_json::to_string(&detail)?)?;
            let fields = writer.cells([Cell::Text(text)])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key: keys[PRESENT as usize],
                native_hour: hour,
                read_hour: None,
                sequence,
                kind: kind::DETAIL,
                fields,
            })?;
        }
        self.carry
            .value(writer, keys[PRESENT as usize], hour, Some(1), boundary)?;
        for (at, amount) in amounts.iter().enumerate() {
            self.carry.value(
                writer,
                keys[PART as usize + at],
                hour,
                amount.raw.map(i128::from),
                boundary,
            )?;
        }
        self.state.last.insert(actor, (hour, detail));
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the metadata where it changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.add_counted();
        self.normalize();
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
        if std::mem::take(&mut self.stale) {
            let text = serde_json::to_vec(&self.state.metadata)?;
            let cell = family::cell(writer, &Scalar::Text(text.into()))?;
            writer.catalogue(METADATA, cell)?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection reads every country anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let json = serde_json::to_vec(&self.state)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let malformed = || invalid("Saved game score emit is not valid");
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| malformed())?;
        let state: State = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        let key = |actor: u64| -> io::Result<Option<[KeyHandle; 7]>> {
            let mut keys = [0; 7];
            for (measure, key) in keys.iter_mut().enumerate() {
                match directory.key(actor, measure as u64, None) {
                    Some(held) => *key = held,
                    None if measure == 0 => return Ok(None),
                    None => return Err(malformed()),
                }
            }
            Ok(Some(keys))
        };
        let mut keys = Vec::with_capacity(directory.actors.len());
        for actor in 0..directory.actors.len() as u64 {
            keys.push(key(actor)?.ok_or_else(malformed)?);
        }
        Ok(Self {
            actors: Actors::resume(SCHEMA, directory)?,
            carry: dashboard_series::carry::Carry::load(&bytes[at..])?,
            keys,
            family: key(FAMILY)?,
            state,
            current: BTreeMap::new(),
            holders: None,
            counting: None,
            stale: false,
            first: None,
        })
    }
}
