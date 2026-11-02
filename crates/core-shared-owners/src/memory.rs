//! In-memory owners for tests: each is fed dated values in hour order and answers its trait from
//! them. They favour plainness over speed and are not the segment owners.

use crate::{
    Absence, Change, CharacterId, CharacterNaming, Characters, ContentId, CountryId, CountryNaming,
    CountryNamingFacts, Dated, Handle, History, Hour, Interval, Link, Lookup, Missing, MissingKind,
    NameError, NamingLook, NamingRead, OperandsId, Owner, Ownership, PlaceKey, ProvinceFacts,
    ProvinceId, Reason, Roster, RosterKey, Slot, Spelling, Spellings, StateFacts, StateId,
    QueryId, QueryRead, ReadScope, StateNameUsage, StateNames, Tag, TextId, TextRead, Token,
    TriggerContext, VariantId,
    VariantKey, VariantVersion, Variants, past_watermark,
};
use core_events::{Key, ObservationRun, ObservationUnknown, Truth};
use std::{collections::BTreeMap, io};

fn map<'a, T, U>(lookup: Lookup<'a, T>, f: impl FnOnce(T) -> U) -> Lookup<'a, U> {
    match lookup {
        Lookup::Known(value) => Lookup::Known(f(value)),
        Lookup::Missing(missing) => Lookup::Missing(missing),
    }
}

fn dated<T: Copy>(value: Dated<&T>) -> Dated<T> {
    Dated {
        since: value.since,
        value: *value.value,
    }
}

/// The hours an owner has applied, in order.
#[derive(Clone, Copy, Debug, Default)]
struct Clock(Option<Hour>);

impl Clock {
    fn apply(&mut self, hour: Hour) -> io::Result<()> {
        if self.0.is_some_and(|last| hour < last) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "shared owner fed an earlier hour",
            ));
        }
        self.0 = Some(hour);
        Ok(())
    }

    fn check(self, hour: Hour) -> io::Result<Hour> {
        match self.0 {
            Some(last) if hour <= last => Ok(last),
            _ => Err(past_watermark()),
        }
    }

    fn check_span(self, hours: Interval) -> io::Result<()> {
        self.check(hours.last()).map(|_| ())
    }
}

/// A refusal as tests feed it: its reason's name and detail.
pub type Refusal = (String, String);

fn reason(refusal: &Refusal) -> Reason<'_> {
    Reason {
        name: &refusal.0,
        detail: &refusal.1,
    }
}

/// Visits changes collected from several histories in hour order, keeping each hour's order.
fn in_order<K>(
    mut all: Vec<Change<K>>,
    visit: &mut dyn FnMut(Change<K>) -> io::Result<()>,
) -> io::Result<()> {
    all.sort_by_key(|change| change.hour);
    all.into_iter().try_for_each(visit)
}

fn resolved(tag: Dated<&Result<Vec<u8>, Refusal>>) -> Dated<Result<Tag<'_>, Reason<'_>>> {
    Dated {
        since: tag.since,
        value: match tag.value {
            Ok(tag) => Ok(Tag(tag)),
            Err(refusal) => Err(reason(refusal)),
        },
    }
}

/// A roster fed link, holder, tag and epoch versions.
#[derive(Clone, Debug, Default)]
pub struct MemoryRoster {
    clock: Clock,
    census: History<(), ()>,
    links: History<Link, Option<Slot>>,
    countries: History<CountryId, Option<Slot>>,
    holders: History<Slot, Vec<(CountryId, Vec<u8>)>>,
    name_tags: History<Slot, Option<Vec<u8>>>,
    original_tags: History<CountryId, Result<Vec<u8>, Refusal>>,
    reserved_zero: History<(), Result<Vec<u8>, Refusal>>,
    epoch: History<(), u64>,
}

impl MemoryRoster {
    /// From `hour` the census is complete: every link and Countries row is set, so a key without
    /// a row is absent rather than unrecorded.
    pub fn set_complete(&mut self, hour: Hour) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.census.set(hour, (), Ok(())).map(drop)
    }

    /// None where the link has no row.
    pub fn set_link(
        &mut self,
        hour: Hour,
        link: Link,
        slot: Result<Option<Slot>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.links.set(hour, link, slot).map(drop)
    }

    /// A Countries row's own slot; none where the row is gone.
    pub fn set_country(
        &mut self,
        hour: Hour,
        country: CountryId,
        slot: Result<Option<Slot>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.countries.set(hour, country, slot).map(drop)
    }

    /// `rows` are the slot's Countries rows in table order.
    pub fn set_holders(
        &mut self,
        hour: Hour,
        slot: Slot,
        rows: Result<Vec<(CountryId, Vec<u8>)>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.holders.set(hour, slot, rows).map(drop)
    }

    pub fn set_name_tag(
        &mut self,
        hour: Hour,
        slot: Slot,
        tag: Result<Option<Vec<u8>>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.name_tags.set(hour, slot, tag).map(drop)
    }

    pub fn set_original_tag(
        &mut self,
        hour: Hour,
        country: CountryId,
        tag: Result<Result<Vec<u8>, Refusal>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.original_tags.set(hour, country, tag).map(drop)
    }

    pub fn set_reserved_zero(
        &mut self,
        hour: Hour,
        tag: Result<Result<Vec<u8>, Refusal>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.reserved_zero.set(hour, (), tag).map(drop)
    }

    pub fn set_epoch(&mut self, hour: Hour, epoch: Result<u64, Absence>) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.epoch.set(hour, (), epoch).map(drop)
    }

    /// Every key held is missing from `hour`, as a gap or damage leaves it.
    pub fn forget(&mut self, hour: Hour, absence: Absence) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.census.set(hour, (), Err(absence.clone()))?;
        for country in self.countries.keys().copied().collect::<Vec<_>>() {
            self.countries.set(hour, country, Err(absence.clone()))?;
        }
        for link in self.links.keys().copied().collect::<Vec<_>>() {
            self.links.set(hour, link, Err(absence.clone()))?;
        }
        for slot in self.holders.keys().copied().collect::<Vec<_>>() {
            self.holders.set(hour, slot, Err(absence.clone()))?;
        }
        for slot in self.name_tags.keys().copied().collect::<Vec<_>>() {
            self.name_tags.set(hour, slot, Err(absence.clone()))?;
        }
        for country in self.original_tags.keys().copied().collect::<Vec<_>>() {
            self.original_tags
                .set(hour, country, Err(absence.clone()))?;
        }
        self.reserved_zero.set(hour, (), Err(absence.clone()))?;
        self.epoch.set(hour, (), Err(absence))?;
        Ok(())
    }

    /// Nothing changed through `hour`.
    pub fn advance(&mut self, hour: Hour) -> io::Result<()> {
        self.clock.apply(hour)
    }
}

impl Owner for MemoryRoster {
    fn watermark(&self) -> Option<Hour> {
        self.clock.0
    }
}

/// `key` at `hour`: its own answer, or known none where a complete census holds and the key
/// has no version yet.
fn censused<'a, K: Ord + Clone, V: PartialEq>(
    census: &'a History<(), ()>,
    history: &'a History<K, Option<V>>,
    key: &K,
    hour: Hour,
    last: Hour,
) -> Lookup<'a, Dated<Option<&'a V>>> {
    match history.at(key, hour, last) {
        Lookup::Known(value) => Lookup::Known(Dated {
            since: value.since,
            value: value.value.as_ref(),
        }),
        Lookup::Missing(missing) if history.first(key).is_none_or(|first| first > hour) => {
            match census.at(&(), hour, last) {
                Lookup::Known(complete) => Lookup::Known(Dated {
                    since: complete.since,
                    value: None,
                }),
                Lookup::Missing(_) => Lookup::Missing(missing),
            }
        }
        Lookup::Missing(missing) => Lookup::Missing(missing),
    }
}

/// The keys of `history` that are members at `hour`, once the census is complete there: a key
/// whose first version is later doesn't exist yet; any other missing key makes the answer missing.
fn members<'a, K: Ord + Clone, V: PartialEq>(
    census: &'a History<(), ()>,
    history: &'a History<K, V>,
    hour: Hour,
    last: Hour,
) -> Result<Vec<(&'a K, Dated<&'a V>)>, Missing<'a>> {
    if let Lookup::Missing(missing) = census.at(&(), hour, last) {
        return Err(missing);
    }
    let mut held = Vec::new();
    for key in history.keys() {
        match history.at(key, hour, last) {
            Lookup::Known(value) => held.push((key, value)),
            Lookup::Missing(_) if history.first(key).is_some_and(|first| first > hour) => {}
            Lookup::Missing(missing) => return Err(missing),
        }
    }
    Ok(held)
}

impl Roster for MemoryRoster {
    fn slot_of(&self, link: Link, hour: Hour) -> io::Result<Lookup<'_, Dated<Option<Slot>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(censused(&self.census, &self.links, &link, hour, last), |slot| Dated {
            since: slot.since,
            value: slot.value.copied(),
        }))
    }

    fn country(
        &self,
        country: CountryId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Option<Slot>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(censused(&self.census, &self.countries, &country, hour, last), |slot| Dated {
            since: slot.since,
            value: slot.value.copied(),
        }))
    }

    fn countries<'a>(
        &'a self,
        hour: Hour,
        visit: &mut dyn FnMut(CountryId, Slot) -> io::Result<()>,
    ) -> io::Result<Lookup<'a, usize>> {
        let last = self.clock.check(hour)?;
        let held = match members(&self.census, &self.countries, hour, last) {
            Ok(held) => held,
            Err(missing) => return Ok(Lookup::Missing(missing)),
        };
        let mut count = 0;
        for (country, slot) in held {
            if let Some(slot) = slot.value {
                visit(*country, *slot)?;
                count += 1;
            }
        }
        Ok(Lookup::Known(count))
    }

    fn holders<'a>(
        &'a self,
        slot: Slot,
        hour: Hour,
        visit: &mut dyn FnMut(CountryId, Tag<'a>) -> io::Result<()>,
    ) -> io::Result<Lookup<'a, Dated<usize>>> {
        let last = self.clock.check(hour)?;
        Ok(match self.holders.at(&slot, hour, last) {
            Lookup::Known(rows) => {
                for (country, tag) in rows.value {
                    visit(*country, Tag(tag))?;
                }
                Lookup::Known(Dated {
                    since: rows.since,
                    value: rows.value.len(),
                })
            }
            Lookup::Missing(missing) => Lookup::Missing(missing),
        })
    }

    fn name_tag(&self, slot: Slot, hour: Hour) -> io::Result<Lookup<'_, Dated<Option<Tag<'_>>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(self.name_tags.at(&slot, hour, last), |tag| Dated {
            since: tag.since,
            value: tag.value.as_deref().map(Tag),
        }))
    }

    fn original_tag(
        &self,
        country: CountryId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Result<Tag<'_>, Reason<'_>>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(self.original_tags.at(&country, hour, last), resolved))
    }

    fn reserved_zero_original_tag(
        &self,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Result<Tag<'_>, Reason<'_>>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(self.reserved_zero.at(&(), hour, last), resolved))
    }

    fn tagged(
        &self,
        tag: Tag<'_>,
        hour: Hour,
        visit: &mut dyn FnMut(Slot) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>> {
        let last = self.clock.check(hour)?;
        // A slot not known now might carry the tag: the answer is unknown, not partial.
        let held = match members(&self.census, &self.holders, hour, last) {
            Ok(held) => held,
            Err(missing) => return Ok(Lookup::Missing(missing)),
        };
        let mut found = Vec::new();
        for (slot, rows) in held {
            if let Some((first, _)) = rows.value.iter().find(|(_, held)| Tag(held) == tag) {
                found.push((*first, *slot));
            }
        }
        found.sort();
        for (_, slot) in &found {
            visit(*slot)?;
        }
        Ok(Lookup::Known(found.len()))
    }

    fn links(
        &self,
        slot: Slot,
        hour: Hour,
        visit: &mut dyn FnMut(Link) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>> {
        let last = self.clock.check(hour)?;
        let held = match members(&self.census, &self.links, hour, last) {
            Ok(held) => held,
            Err(missing) => return Ok(Lookup::Missing(missing)),
        };
        let mut count = 0;
        let found: Vec<Link> = held
            .into_iter()
            .filter(|(_, named)| *named.value == Some(slot))
            .map(|(link, _)| *link)
            .collect();
        for link in found {
            visit(link)?;
            count += 1;
        }
        Ok(Lookup::Known(count))
    }

    fn epoch(&self, hour: Hour) -> io::Result<Lookup<'_, Dated<u64>>> {
        let last = self.clock.check(hour)?;
        Ok(map(self.epoch.at(&(), hour, last), dated))
    }

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<RosterKey>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.clock.check_span(hours)?;
        let mut all = Vec::new();
        let mut push = |hour, key| {
            all.push(Change { hour, key });
            Ok(())
        };
        self.links
            .changes(hours, &mut |hour, link| push(hour, RosterKey::Link(*link)))?;
        self.countries.changes(hours, &mut |hour, country| {
            push(hour, RosterKey::Country(*country))
        })?;
        self.holders.changes(hours, &mut |hour, slot| {
            push(hour, RosterKey::Holders(*slot))
        })?;
        self.name_tags.changes(hours, &mut |hour, slot| {
            push(hour, RosterKey::NameTag(*slot))
        })?;
        self.original_tags.changes(hours, &mut |hour, country| {
            push(hour, RosterKey::OriginalTag(*country))
        })?;
        self.reserved_zero
            .changes(hours, &mut |hour, _| push(hour, RosterKey::ReservedZero))?;
        self.epoch
            .changes(hours, &mut |hour, _| push(hour, RosterKey::Epoch))?;
        self.census
            .changes(hours, &mut |hour, _| push(hour, RosterKey::Census))?;
        in_order(all, visit)
    }
}

/// Spellings fed token versions; equal literals share a content identity.
#[derive(Clone, Debug, Default)]
pub struct MemorySpellings {
    clock: Clock,
    spellings: History<Token, (Vec<u8>, ContentId)>,
    contents: BTreeMap<Vec<u8>, ContentId>,
}

impl MemorySpellings {
    pub fn set_spelling(
        &mut self,
        hour: Hour,
        token: Token,
        literal: Result<Vec<u8>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        let next = ContentId(self.contents.len() as u64);
        let value = literal.map(|literal| {
            let content = *self.contents.entry(literal.clone()).or_insert(next);
            (literal, content)
        });
        self.spellings.set(hour, token, value).map(drop)
    }

    /// Every token held is missing from `hour`.
    pub fn forget(&mut self, hour: Hour, absence: Absence) -> io::Result<()> {
        self.clock.apply(hour)?;
        for token in self.spellings.keys().copied().collect::<Vec<_>>() {
            self.spellings.set(hour, token, Err(absence.clone()))?;
        }
        Ok(())
    }

    pub fn advance(&mut self, hour: Hour) -> io::Result<()> {
        self.clock.apply(hour)
    }
}

impl Owner for MemorySpellings {
    fn watermark(&self) -> Option<Hour> {
        self.clock.0
    }
}

impl Spellings for MemorySpellings {
    fn spelling(&self, token: Token, hour: Hour) -> io::Result<Lookup<'_, Dated<Spelling<'_>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(self.spellings.at(&token, hour, last), |held| Dated {
            since: held.since,
            value: Spelling {
                literal: &held.value.0,
                content: held.value.1,
            },
        }))
    }

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<Token>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.clock.check_span(hours)?;
        self.spellings.changes(hours, &mut |hour, token| {
            visit(Change { hour, key: *token })
        })
    }
}

/// A variant version as tests feed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedVersion {
    pub operands: OperandsId,
    pub token: Option<String>,
    /// A refusal is its reason's name and detail.
    pub name: Result<Option<String>, (String, String)>,
    pub design: Option<String>,
}

/// The ledger's truth over a run, as tests feed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerTruth {
    Holds,
    Fails,
    Unknown(LedgerUnknown, String, String),
    NotRecorded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerUnknown {
    Gap,
    Damage,
    Refused,
    Unavailable,
}

/// A variant owner fed versions, naming reads, forgets and the ledger's truth. A forget makes
/// every version missing until the next complete naming read, which keeps unchanged dates.
#[derive(Clone, Debug, Default)]
pub struct MemoryVariants {
    clock: Clock,
    ids: BTreeMap<Handle, VariantId>,
    handles: Vec<Handle>,
    versions: History<VariantId, OwnedVersion>,
    reads: Vec<(Hour, NamingRead)>,
    forgets: Vec<(Hour, Absence)>,
    ledger: Vec<(Hour, LedgerTruth)>,
}

impl MemoryVariants {
    /// The id of `handle`, assigned at its first sight in id order.
    pub fn assign(&mut self, handle: Handle) -> VariantId {
        let next = VariantId(self.handles.len() as u32);
        *self.ids.entry(handle).or_insert_with(|| {
            self.handles.push(handle);
            next
        })
    }

    pub fn set_version(
        &mut self,
        hour: Hour,
        handle: Handle,
        version: Result<OwnedVersion, Absence>,
    ) -> io::Result<VariantId> {
        self.clock.apply(hour)?;
        let id = self.assign(handle);
        self.versions.set(hour, id, version)?;
        Ok(id)
    }

    pub fn read(&mut self, hour: Hour, read: NamingRead) -> io::Result<()> {
        self.clock.apply(hour)?;
        if self.reads.last() != Some(&(hour, read)) {
            self.reads.push((hour, read));
        }
        Ok(())
    }

    pub fn forget(&mut self, hour: Hour, absence: Absence) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.forgets.push((hour, absence));
        Ok(())
    }

    /// The ledger's truth from `hour` until the next.
    pub fn set_ledger(&mut self, hour: Hour, truth: LedgerTruth) -> io::Result<()> {
        self.clock.apply(hour)?;
        if self.ledger.last().is_none_or(|(_, held)| *held != truth) {
            self.ledger.push((hour, truth));
        }
        Ok(())
    }

    pub fn advance(&mut self, hour: Hour) -> io::Result<()> {
        self.clock.apply(hour)
    }

    fn last_read(&self, hour: Hour, census: bool) -> Option<Hour> {
        self.reads
            .iter()
            .rev()
            .find(|(at, read)| *at <= hour && (!census || *read == NamingRead::Census))
            .map(|(at, _)| *at)
    }
}

impl Owner for MemoryVariants {
    fn watermark(&self) -> Option<Hour> {
        self.clock.0
    }
}

impl Variants for MemoryVariants {
    fn id(&self, handle: Handle) -> Option<VariantId> {
        self.ids.get(&handle).copied()
    }

    fn handle(&self, id: VariantId) -> Option<Handle> {
        self.handles.get(id.0 as usize).copied()
    }

    fn version(
        &self,
        id: VariantId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<VariantVersion<'_>>>> {
        let last = self.clock.check(hour)?;
        if let Lookup::Missing(missing) = self.known_since(hour)? {
            return Ok(Lookup::Missing(missing));
        }
        Ok(map(self.versions.at(&id, hour, last), |held| Dated {
            since: held.since,
            value: VariantVersion {
                operands: held.value.operands,
                token: held.value.token.as_deref(),
                name: match &held.value.name {
                    Ok(name) => Ok(name.as_deref()),
                    Err((name, detail)) => Err(Reason { name, detail }),
                },
                design: held.value.design.as_deref(),
            },
        }))
    }

    fn known_since(&self, hour: Hour) -> io::Result<Lookup<'_, Hour>> {
        let last = self.clock.check(hour)?;
        let forget = self.forgets.iter().rev().find(|(at, _)| *at <= hour);
        let from = forget.map(|(at, _)| *at);
        let restored = self
            .reads
            .iter()
            .find(|(at, _)| from.is_none_or(|from| *at >= from) && *at <= hour)
            .map(|(at, _)| *at);
        Ok(match (restored, forget) {
            (Some(read), _) => Lookup::Known(read),
            (None, Some((at, absence))) => {
                let read = self.reads.iter().find(|(read, _)| read >= at).map(|(read, _)| *read);
                let next = self.forgets.iter().find(|(next, _)| next > at).map(|(next, _)| *next);
                let until = read.into_iter().chain(next).min().map_or(last, |end| end - 1);
                Lookup::Missing(Missing {
                    hours: Interval::new(*at, until).unwrap_or(Interval::hour(*at)),
                    kind: absence.kind(),
                })
            }
            (None, None) => Lookup::Missing(Missing {
                hours: Interval::hour(hour),
                kind: MissingKind::NotRecorded,
            }),
        })
    }

    fn census(&self, hour: Hour) -> io::Result<Option<Hour>> {
        self.clock.check(hour)?;
        Ok(self.last_read(hour, true))
    }

    fn reads(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Hour, NamingRead) -> io::Result<()>,
    ) -> io::Result<()> {
        self.clock.check_span(hours)?;
        self.reads
            .iter()
            .filter(|(at, _)| (hours.first()..=hours.last()).contains(at))
            .try_for_each(|(at, read)| visit(*at, *read))
    }

    fn ledger<'a>(
        &'a self,
        hours: Interval,
        visit: &mut dyn FnMut(ObservationRun<'a>) -> io::Result<()>,
    ) -> io::Result<()> {
        let last = self.clock.check(hours.last())?;
        let first = self.ledger.first().map_or(last + 1, |(at, _)| *at);
        if let Some(before) = Interval::new(hours.first(), first - 1)
            .ok()
            .and_then(|run| run.intersection(hours))
        {
            visit(ObservationRun {
                hours: before,
                truth: Truth::Unknown(ObservationUnknown::NotRecorded),
            })?;
        }
        for (at, (from, truth)) in self.ledger.iter().enumerate() {
            let until = self.ledger.get(at + 1).map_or(last, |(next, _)| next - 1);
            let Some(run) = Interval::new(*from, until)
                .ok()
                .and_then(|run| run.intersection(hours))
            else {
                continue;
            };
            let truth = match truth {
                LedgerTruth::Holds => Truth::Holds,
                LedgerTruth::Fails => Truth::Fails,
                LedgerTruth::NotRecorded => Truth::Unknown(ObservationUnknown::NotRecorded),
                LedgerTruth::Unknown(kind, name, detail) => {
                    let reason = Reason { name, detail };
                    Truth::Unknown(match kind {
                        LedgerUnknown::Gap => ObservationUnknown::Gap(reason),
                        LedgerUnknown::Damage => ObservationUnknown::Damage(reason),
                        LedgerUnknown::Refused => ObservationUnknown::Refused(reason),
                        LedgerUnknown::Unavailable => ObservationUnknown::Unavailable(reason),
                    })
                }
            };
            visit(ObservationRun { hours: run, truth })?;
        }
        Ok(())
    }

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<VariantKey>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.clock.check_span(hours)?;
        let within = |at: &Hour| (hours.first()..=hours.last()).contains(at);
        let mut all = Vec::new();
        self.versions.changes(hours, &mut |hour, id| {
            all.push(Change {
                hour,
                key: VariantKey::Version(*id),
            });
            Ok(())
        })?;
        for (at, read) in self.reads.iter().filter(|(at, _)| within(at)) {
            all.push(Change {
                hour: *at,
                key: VariantKey::Read(*read),
            });
        }
        for (at, _) in self.forgets.iter().filter(|(at, _)| within(at)) {
            all.push(Change {
                hour: *at,
                key: VariantKey::Forgotten,
            });
        }
        in_order(all, visit)
    }
}

/// One evaluation's footprint as the context keeps it: reads, each of a whole field or one key,
/// and deadlines.
#[derive(Clone, Debug, Default)]
struct Footprint {
    reads: Vec<(TextRead, Option<Vec<u8>>)>,
    deadlines: Vec<Hour>,
}

/// A trigger context fed each text's compiled reads and the rows whose fields changed; queries
/// get footprints from their evaluations.
#[derive(Clone, Debug, Default)]
pub struct MemoryTriggers {
    clock: Clock,
    texts: BTreeMap<TextId, Result<Vec<(TextRead, bool)>, Refusal>>,
    changed: Vec<(Hour, TextRead, Vec<u8>)>,
    queries: BTreeMap<QueryId, (TextId, Vec<(Hour, Footprint)>)>,
    next: u64,
}

impl MemoryTriggers {
    /// `text`'s reads, each with whether the context retains it, or why it can't compile.
    pub fn set_text(&mut self, text: TextId, reads: Result<Vec<(TextRead, bool)>, Refusal>) {
        self.texts.insert(text, reads);
    }

    /// `read` of the row `key` changed at `hour`.
    pub fn change(&mut self, hour: Hour, read: TextRead, key: Key<'_>) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.changed.push((hour, read, key.varint().to_vec()));
        Ok(())
    }

    pub fn advance(&mut self, hour: Hour) -> io::Result<()> {
        self.clock.apply(hour)
    }

    fn query(&self, query: QueryId) -> io::Result<&(TextId, Vec<(Hour, Footprint)>)> {
        self.queries.get(&query).ok_or_else(not_registered)
    }

    fn compiled(&self, text: TextId, hour: Hour) -> Result<&[(TextRead, bool)], Missing<'_>> {
        let hours = Interval::hour(hour);
        match self.texts.get(&text) {
            Some(Ok(reads)) => Ok(reads),
            Some(Err(refusal)) => Err(Missing {
                hours,
                kind: MissingKind::Refused(reason(refusal)),
            }),
            None => Err(Missing {
                hours,
                kind: MissingKind::NotRecorded,
            }),
        }
    }
}

fn not_registered() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "trigger query not registered")
}

/// The index of the footprint in force at `hour`: the latest evaluated at or before it.
fn footprint_at(versions: &[(Hour, Footprint)], hour: Hour) -> Option<usize> {
    versions
        .partition_point(|(from, _)| *from <= hour)
        .checked_sub(1)
}

impl Owner for MemoryTriggers {
    fn watermark(&self) -> Option<Hour> {
        self.clock.0
    }
}

impl TriggerContext for MemoryTriggers {
    fn reads(
        &self,
        text: TextId,
        visit: &mut dyn FnMut(TextRead, bool) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>> {
        match self.compiled(text, self.clock.0.unwrap_or(0)) {
            Ok(reads) => {
                for &(read, retained) in reads {
                    visit(read, retained)?;
                }
                Ok(Lookup::Known(reads.len()))
            }
            Err(missing) => Ok(Lookup::Missing(missing)),
        }
    }

    fn register(&mut self, text: TextId) -> io::Result<QueryId> {
        let query = QueryId(self.next);
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| io::Error::other("trigger query ids exhausted"))?;
        self.queries.insert(query, (text, Vec::new()));
        Ok(query)
    }

    fn unregister(&mut self, query: QueryId) -> io::Result<()> {
        self.queries.remove(&query).map(drop).ok_or_else(not_registered)
    }

    fn footprint(
        &mut self,
        query: QueryId,
        hour: Hour,
        reads: &[QueryRead<'_>],
        deadlines: &[Hour],
    ) -> io::Result<()> {
        self.clock.check(hour)?;
        let (_, versions) = self.queries.get_mut(&query).ok_or_else(not_registered)?;
        if versions.last().is_some_and(|(from, _)| *from > hour) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "trigger footprint before its last evaluation",
            ));
        }
        let mut deadlines = deadlines.to_vec();
        deadlines.sort_unstable();
        let footprint = Footprint {
            reads: reads
                .iter()
                .map(|read| {
                    let key = match read.scope {
                        ReadScope::Field => None,
                        ReadScope::Key(key) => Some(key.varint().to_vec()),
                    };
                    (read.read, key)
                })
                .collect(),
            deadlines,
        };
        match versions.last_mut() {
            Some((from, held)) if *from == hour => *held = footprint,
            _ => versions.push((hour, footprint)),
        }
        Ok(())
    }

    fn undeclared(
        &self,
        query: QueryId,
        visit: &mut dyn FnMut(TextRead) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>> {
        match self.compiled(self.query(query)?.0, self.clock.0.unwrap_or(0)) {
            Ok(reads) => {
                let mut count = 0;
                for &(read, retained) in reads {
                    if !retained {
                        visit(read)?;
                        count += 1;
                    }
                }
                Ok(Lookup::Known(count))
            }
            Err(missing) => Ok(Lookup::Missing(missing)),
        }
    }

    fn deadline(&self, query: QueryId, hour: Hour) -> io::Result<Lookup<'_, Option<Hour>>> {
        self.clock.check(hour)?;
        let (text, versions) = self.query(query)?;
        if let Err(missing) = self.compiled(*text, hour) {
            return Ok(Lookup::Missing(missing));
        }
        Ok(Lookup::Known(footprint_at(versions, hour).and_then(|at| {
            versions[at].1.deadlines.iter().find(|due| **due > hour).copied()
        })))
    }

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<QueryId>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.clock.check_span(hours)?;
        let within = |hour: Hour| hours.first() <= hour && hour <= hours.last();
        let mut all = std::collections::BTreeSet::new();
        for (&query, (_, versions)) in &self.queries {
            for (hour, read, key) in &self.changed {
                let touches = footprint_at(versions, *hour).is_some_and(|at| {
                    versions[at].1.reads.iter().any(|(held, scope)| {
                        held == read && scope.as_ref().is_none_or(|held| held == key)
                    })
                });
                if within(*hour) && touches {
                    all.insert((*hour, query));
                }
            }
            for (index, (from, footprint)) in versions.iter().enumerate() {
                for &due in &footprint.deadlines {
                    // A deadline counts while its own evaluation is still the one in force.
                    if due > *from && within(due) && footprint_at(versions, due) == Some(index) {
                        all.insert((due, query));
                    }
                }
            }
        }
        all.into_iter()
            .try_for_each(|(hour, key)| visit(Change { hour, key }))
    }
}

/// Ownership fed province and state facts.
#[derive(Clone, Debug, Default)]
pub struct MemoryOwnership {
    clock: Clock,
    provinces: History<ProvinceId, Option<ProvinceFacts>>,
    states: History<StateId, Option<StateFacts>>,
}

impl MemoryOwnership {
    pub fn set_province(
        &mut self,
        hour: Hour,
        id: ProvinceId,
        facts: Result<Option<ProvinceFacts>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.provinces.set(hour, id, facts).map(drop)
    }

    pub fn set_state(
        &mut self,
        hour: Hour,
        id: StateId,
        facts: Result<Option<StateFacts>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.states.set(hour, id, facts).map(drop)
    }

    pub fn advance(&mut self, hour: Hour) -> io::Result<()> {
        self.clock.apply(hour)
    }
}

impl Owner for MemoryOwnership {
    fn watermark(&self) -> Option<Hour> {
        self.clock.0
    }
}

impl Ownership for MemoryOwnership {
    fn province(
        &self,
        id: ProvinceId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Option<ProvinceFacts>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(self.provinces.at(&id, hour, last), dated))
    }

    fn state(&self, id: StateId, hour: Hour) -> io::Result<Lookup<'_, Dated<Option<StateFacts>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(self.states.at(&id, hour, last), dated))
    }

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<PlaceKey>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.clock.check_span(hours)?;
        let mut all = Vec::new();
        let mut push = |hour, key| {
            all.push(Change { hour, key });
            Ok(())
        };
        self.provinces
            .changes(hours, &mut |hour, id| push(hour, PlaceKey::Province(*id)))?;
        self.states
            .changes(hours, &mut |hour, id| push(hour, PlaceKey::State(*id)))?;
        in_order(all, visit)
    }
}

/// A state-name error as tests feed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnedNameError {
    Stored(Refusal),
    Templates(Refusal),
    NoTemplate,
    TemplateRefused(String, Refusal),
    NotPlain(Refusal),
}

/// State names fed per state and usage.
#[derive(Clone, Debug, Default)]
pub struct MemoryStateNames {
    clock: Clock,
    names: History<(StateId, StateNameUsage), Result<String, OwnedNameError>>,
}

impl MemoryStateNames {
    pub fn set_state_name(
        &mut self,
        hour: Hour,
        id: StateId,
        usage: StateNameUsage,
        name: Result<Result<String, OwnedNameError>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.names.set(hour, (id, usage), name).map(drop)
    }

    pub fn advance(&mut self, hour: Hour) -> io::Result<()> {
        self.clock.apply(hour)
    }
}

impl Owner for MemoryStateNames {
    fn watermark(&self) -> Option<Hour> {
        self.clock.0
    }
}

fn name_error(error: &OwnedNameError) -> NameError<'_> {
    match error {
        OwnedNameError::Stored(refusal) => NameError::Stored(reason(refusal)),
        OwnedNameError::Templates(refusal) => NameError::Templates(reason(refusal)),
        OwnedNameError::NoTemplate => NameError::NoTemplate,
        OwnedNameError::TemplateRefused(key, refusal) => NameError::TemplateRefused {
            key,
            reason: reason(refusal),
        },
        OwnedNameError::NotPlain(refusal) => NameError::NotPlain(reason(refusal)),
    }
}

impl StateNames for MemoryStateNames {
    fn state_name(
        &self,
        id: StateId,
        usage: StateNameUsage,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Result<&str, NameError<'_>>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(self.names.at(&(id, usage), hour, last), |held| Dated {
            since: held.since,
            value: held.value.as_deref().map_err(name_error),
        }))
    }

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<StateId>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.clock.check_span(hours)?;
        let mut all = Vec::new();
        self.names.changes(hours, &mut |hour, (id, _)| {
            all.push(Change { hour, key: *id });
            Ok(())
        })?;
        // One change per state and hour, whichever usages moved.
        all.sort_by_key(|change| (change.hour, change.key));
        all.dedup();
        all.into_iter().try_for_each(visit)
    }
}

/// Characters fed naming versions, answering from `retained` on.
#[derive(Clone, Debug, Default)]
pub struct MemoryCharacters {
    clock: Clock,
    retained: Option<Hour>,
    characters: History<CharacterId, Option<CharacterNaming>>,
    respellings: Vec<(Hour, Token)>,
}

impl MemoryCharacters {
    pub fn set_character(
        &mut self,
        hour: Hour,
        id: CharacterId,
        naming: Result<Option<CharacterNaming>, Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.retained.get_or_insert(hour);
        self.characters.set(hour, id, naming).map(drop)
    }

    /// `token`'s spelling changed at `hour`; every character named by it then changes.
    pub fn respell(&mut self, hour: Hour, token: Token) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.respellings.push((hour, token));
        Ok(())
    }

    /// Answers no hour before `hour` any more.
    pub fn retain_from(&mut self, hour: Hour) {
        self.retained = Some(self.retained.map_or(hour, |held| held.max(hour)));
    }

    pub fn advance(&mut self, hour: Hour) -> io::Result<()> {
        self.clock.apply(hour)
    }
}

impl Owner for MemoryCharacters {
    fn watermark(&self) -> Option<Hour> {
        self.clock.0
    }
}

impl Characters for MemoryCharacters {
    fn retained(&self) -> Option<Hour> {
        self.retained
    }

    fn character(
        &self,
        id: CharacterId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Option<CharacterNaming>>>> {
        let last = self.clock.check(hour)?;
        if self.retained.is_some_and(|first| hour < first) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "character naming asked before its retained history",
            ));
        }
        Ok(map(self.characters.at(&id, hour, last), dated))
    }

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<CharacterId>) -> io::Result<()>,
    ) -> io::Result<()> {
        let last = self.clock.check(hours.last())?;
        let mut all = std::collections::BTreeSet::new();
        self.characters.changes(hours, &mut |hour, id| {
            all.insert((hour, *id));
            Ok(())
        })?;
        for &(hour, token) in &self.respellings {
            if hour < hours.first() || hour > hours.last() {
                continue;
            }
            for id in self.characters.keys() {
                if let Lookup::Known(held) = self.characters.at(id, hour, last)
                    && held.value.is_some_and(|naming| naming.token == token)
                {
                    all.insert((hour, *id));
                }
            }
        }
        all.into_iter()
            .try_for_each(|(hour, key)| visit(Change { hour, key }))
    }
}

/// A country's appearance as tests feed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedLook {
    pub name_tag: Vec<u8>,
    pub cosmetic_tag: Vec<u8>,
    pub original_or_own_tag: Result<Vec<u8>, Refusal>,
    pub civil_war: Result<bool, Refusal>,
}

/// Country naming fed per country: its slot and appearance.
#[derive(Clone, Debug, Default)]
pub struct MemoryCountryNaming {
    clock: Clock,
    naming: History<CountryId, (Slot, Option<OwnedLook>)>,
}

impl MemoryCountryNaming {
    pub fn set_naming(
        &mut self,
        hour: Hour,
        country: CountryId,
        naming: Result<(Slot, Option<OwnedLook>), Absence>,
    ) -> io::Result<()> {
        self.clock.apply(hour)?;
        self.naming.set(hour, country, naming).map(drop)
    }

    pub fn advance(&mut self, hour: Hour) -> io::Result<()> {
        self.clock.apply(hour)
    }
}

impl Owner for MemoryCountryNaming {
    fn watermark(&self) -> Option<Hour> {
        self.clock.0
    }
}

fn look(look: &OwnedLook) -> NamingLook<'_> {
    NamingLook {
        name_tag: Tag(&look.name_tag),
        cosmetic_tag: Tag(&look.cosmetic_tag),
        original_or_own_tag: match &look.original_or_own_tag {
            Ok(tag) => Ok(Tag(tag)),
            Err(refusal) => Err(reason(refusal)),
        },
        civil_war: look.civil_war.as_ref().copied().map_err(reason),
    }
}

impl CountryNaming for MemoryCountryNaming {
    fn naming(
        &self,
        country: CountryId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<CountryNamingFacts<'_>>>> {
        let last = self.clock.check(hour)?;
        Ok(map(self.naming.at(&country, hour, last), |held| Dated {
            since: held.since,
            value: CountryNamingFacts {
                slot: held.value.0,
                look: held.value.1.as_ref().map(look),
            },
        }))
    }

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<CountryId>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.clock.check_span(hours)?;
        self.naming.changes(hours, &mut |hour, country| {
            visit(Change {
                hour,
                key: *country,
            })
        })
    }
}
