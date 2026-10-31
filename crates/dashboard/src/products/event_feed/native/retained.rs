//! Retained observations are details of state at a read, never past fires. An observation
//! lists only the parts its read changed; a countdown that only ran down is not a change.
use super::*;
use replay_data::event_feed_rows::{
    EventFeedScope, EventFeedScriptChange as Change,
    decisions::{CONTAINERS, DecisionState, RetainedDecision, RetainedRandomCount},
    state::{DelayedEvent, PackHeader, PendingEvent, ScriptEventState, ScriptIdentity},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

#[cfg(test)]
mod tests;

fn identity(value: &ScriptIdentity) -> String {
    format!(
        "{} [{}]",
        String::from_utf8_lossy(&value.token),
        value.native_event
    )
}
fn scopes(notes: &mut Vec<String>, label: &str, values: &[EventFeedScope]) {
    for (id, scope) in values.iter().enumerate() {
        notes.push(format!("{label} scope {id}: country reference {}; state {}; ROOT {}, FROM {}, PREV {}; random {:?}; references {:?}; strategic region {:?}; missing combat {}, saved targets {}, tooltip targets {}.", scope.country,scope.state,scope.bindings[0],scope.bindings[1],scope.bindings[2],scope.random,scope.references,scope.strategic_region,scope.combat_context_unrecorded,scope.target_storage_unrecorded,scope.tooltip_storage_unrecorded));
    }
}

/// A part of the retained state an observation lists, in its order: the pending instances,
/// the fired set, then each delayed-event owner and each decision owner, by the reducer's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) enum Slot {
    Pending,
    Fired,
    Delayed(u32, u32),
    Decisions(u32, u32),
}

/// What a part holds but its countdowns, read alike from the state and from a kept copy.
enum View<'a> {
    Pending(Result<&'a [PendingEvent], &'a str>),
    Fired(Result<&'a [ScriptIdentity], &'a str>),
    Delayed(EventFeedCountry, Result<&'a [DelayedEvent], &'a str>),
    /// The owner, and what it holds when complete.
    Decisions(EventFeedCountry, Option<Holds<'a>>),
}
struct Holds<'a> {
    members: [&'a [RetainedDecision]; CONTAINERS],
    random_counts: &'a [RetainedRandomCount],
    ignored: &'a BTreeSet<Vec<u8>>,
    candidates: &'a BTreeSet<(u8, Vec<u8>, u32, u32)>,
}

impl View<'_> {
    /// What the part's digest is taken of.
    fn feed(&self, hasher: &mut blake3::Hasher) -> serde_json::Result<()> {
        match self {
            View::Pending(pending) => serde_json::to_writer(hasher, pending),
            View::Fired(fired) => serde_json::to_writer(hasher, fired),
            View::Delayed(country, rows) => {
                serde_json::to_writer(&mut *hasher, country)?;
                match rows {
                    Ok(rows) => {
                        serde_json::to_writer(&mut *hasher, &rows.len())?;
                        for timer in *rows {
                            let held = (
                                timer.instance,
                                &timer.definition,
                                timer.originator,
                                &timer.scopes,
                            );
                            serde_json::to_writer(&mut *hasher, &held)?;
                        }
                        Ok(())
                    }
                    Err(reason) => serde_json::to_writer(hasher, reason),
                }
            }
            View::Decisions(country, holds) => {
                serde_json::to_writer(&mut *hasher, &(country, holds.is_some()))?;
                if let Some(holds) = holds {
                    for (container, members) in holds.members.iter().enumerate() {
                        for member in *members {
                            let held = (
                                container as u8,
                                member.instance,
                                &member.token,
                                member.lifecycle,
                                member.flags,
                                member.country_target,
                                member.numeric_target,
                            );
                            serde_json::to_writer(&mut *hasher, &held)?;
                        }
                    }
                    let rest = (holds.random_counts, holds.ignored, holds.candidates);
                    serde_json::to_writer(&mut *hasher, &rest)?;
                }
                Ok(())
            }
        }
    }

    /// Whether both hold the same, which is whether their digests are taken of the same.
    fn same(&self, other: &View<'_>) -> bool {
        match (self, other) {
            (View::Pending(held), View::Pending(other)) => held == other,
            (View::Fired(held), View::Fired(other)) => held == other,
            (View::Delayed(country, held), View::Delayed(other_country, other)) => {
                country == other_country
                    && match (held, other) {
                        (Ok(held), Ok(other)) => {
                            held.len() == other.len()
                                && held.iter().zip(*other).all(|(held, other)| {
                                    held.instance == other.instance
                                        && held.definition == other.definition
                                        && held.originator == other.originator
                                        && held.scopes == other.scopes
                                })
                        }
                        (Err(held), Err(other)) => held == other,
                        _ => false,
                    }
            }
            (View::Decisions(country, held), View::Decisions(other_country, other)) => {
                country == other_country
                    && match (held, other) {
                        (None, None) => true,
                        (Some(held), Some(other)) => {
                            held.members
                                .iter()
                                .zip(&other.members)
                                .all(|(held, other)| {
                                    held.len() == other.len()
                                        && held.iter().zip(other.iter()).all(|(held, other)| {
                                            (
                                                held.instance,
                                                &held.token,
                                                held.lifecycle,
                                                held.flags,
                                                held.country_target,
                                                held.numeric_target,
                                            ) == (
                                                other.instance,
                                                &other.token,
                                                other.lifecycle,
                                                other.flags,
                                                other.country_target,
                                                other.numeric_target,
                                            )
                                        })
                                })
                                && held.random_counts == other.random_counts
                                && held.ignored == other.ignored
                                && held.candidates == other.candidates
                        }
                        _ => false,
                    }
            }
            _ => false,
        }
    }
}

/// A part's kept copy, never changed once made: what it held but its countdowns when it was
/// seen to change, and its digest once taken.
pub(crate) struct Kept {
    part: Part,
    digest: OnceLock<[u8; 16]>,
}
enum Part {
    Pending(Result<Vec<PendingEvent>, &'static str>),
    Fired(Result<Vec<ScriptIdentity>, &'static str>),
    /// Its timers' remaining hours are those of when it was kept.
    Delayed(EventFeedCountry, Result<Vec<DelayedEvent>, &'static str>),
    Decisions(Decided),
}
/// A decision owner as kept. Its members' days are those of when it was kept, and an incomplete
/// owner keeps no members.
pub(crate) struct Decided {
    pub(crate) country: EventFeedCountry,
    pub(crate) complete: bool,
    pub(crate) members: [Vec<RetainedDecision>; CONTAINERS],
    pub(crate) random_counts: Vec<RetainedRandomCount>,
    pub(crate) ignored: BTreeSet<Vec<u8>>,
    pub(crate) candidates: BTreeSet<(u8, Vec<u8>, u32, u32)>,
}

impl Decided {
    /// The bytes it owns on the heap.
    fn heap(&self) -> usize {
        self.members
            .iter()
            .map(|members| {
                members.capacity() * size_of::<RetainedDecision>()
                    + members
                        .iter()
                        .map(|member| member.token.capacity())
                        .sum::<usize>()
            })
            .sum::<usize>()
            + self.random_counts.capacity() * size_of::<RetainedRandomCount>()
            + self
                .random_counts
                .iter()
                .map(|count| count.token.capacity())
                .sum::<usize>()
            + self
                .ignored
                .iter()
                .map(|token| size_of::<Vec<u8>>() + token.capacity())
                .sum::<usize>()
            + self
                .candidates
                .iter()
                .map(|(_, token, _, _)| size_of::<(u8, Vec<u8>, u32, u32)>() + token.capacity())
                .sum::<usize>()
    }
}

fn rows<'a, T>(kept: &'a Result<Vec<T>, &'static str>) -> Result<&'a [T], &'static str> {
    match kept {
        Ok(rows) => Ok(rows.as_slice()),
        Err(reason) => Err(*reason),
    }
}

impl Kept {
    /// The bytes it owns, itself included.
    pub(crate) fn heap(&self) -> usize {
        let scopes = |scopes: &Vec<EventFeedScope>| scopes.capacity() * size_of::<EventFeedScope>();
        size_of::<Self>()
            + match &self.part {
                Part::Pending(Ok(events)) => {
                    events.capacity() * size_of::<PendingEvent>()
                        + events
                            .iter()
                            .map(|event| event.definition.token.capacity() + scopes(&event.scopes))
                            .sum::<usize>()
                }
                Part::Fired(Ok(fired)) => {
                    fired.capacity() * size_of::<ScriptIdentity>()
                        + fired
                            .iter()
                            .map(|identity| identity.token.capacity())
                            .sum::<usize>()
                }
                Part::Delayed(_, Ok(events)) => {
                    events.capacity() * size_of::<DelayedEvent>()
                        + events
                            .iter()
                            .map(|event| event.definition.token.capacity() + scopes(&event.scopes))
                            .sum::<usize>()
                }
                Part::Decisions(decided) => decided.heap(),
                Part::Pending(Err(_)) | Part::Fired(Err(_)) | Part::Delayed(_, Err(_)) => 0,
            }
    }
    fn new(part: Part) -> Self {
        Self {
            part,
            digest: OnceLock::new(),
        }
    }
    fn view(&self) -> View<'_> {
        match &self.part {
            Part::Pending(kept) => View::Pending(rows(kept)),
            Part::Fired(kept) => View::Fired(rows(kept)),
            Part::Delayed(country, kept) => View::Delayed(*country, rows(kept)),
            Part::Decisions(decided) => View::Decisions(
                decided.country,
                decided.complete.then(|| Holds {
                    members: std::array::from_fn(|container| decided.members[container].as_slice()),
                    random_counts: &decided.random_counts,
                    ignored: &decided.ignored,
                    candidates: &decided.candidates,
                }),
            ),
        }
    }
    /// The decision owner kept, when this is one.
    pub(crate) fn decided(&self) -> Option<&Decided> {
        match &self.part {
            Part::Decisions(decided) => Some(decided),
            _ => None,
        }
    }
    /// The digest of what it holds, taken once by whichever thread asks first.
    pub(crate) fn digest(&self) -> io::Result<[u8; 16]> {
        if let Some(digest) = self.digest.get() {
            return Ok(*digest);
        }
        let taken = digest(|hasher| self.view().feed(hasher))?;
        Ok(*self.digest.get_or_init(|| taken))
    }
}

/// Each decision owner's last kept copy, by its id, with the version and completeness it was
/// kept at, so its roster rebuilds and its observations share one.
pub(crate) type Contents = BTreeMap<u32, (u64, bool, Arc<Kept>)>;

/// `owner` (of id `id`) as it stands, kept: the copy kept at its version and completeness, or
/// a new one.
pub(crate) fn decided(contents: &mut Contents, id: u32, owner: &DecisionState) -> Arc<Kept> {
    let complete = owner.complete();
    let version = owner.version();
    if let Some(version) = version
        && let Some((held, available, kept)) = contents.get(&id)
        && (*held, *available) == (version, complete)
    {
        return Arc::clone(kept);
    }
    let kept = Arc::new(Kept::new(Part::Decisions(Decided {
        country: owner.country,
        complete,
        members: std::array::from_fn(|container| {
            owner
                .members(container as u8)
                .map(<[_]>::to_vec)
                .unwrap_or_default()
        }),
        random_counts: owner.random_counts.clone(),
        ignored: owner.ignored.clone(),
        candidates: owner.candidates.clone(),
    })));
    if let Some(version) = version {
        contents.insert(id, (version, complete, Arc::clone(&kept)));
    }
    kept
}

/// What a part held when last seen: its kept copy, or only the digest a saved observation left.
enum Last {
    Digest([u8; 16]),
    Kept(Arc<Kept>),
}

/// A part as last seen: what it held but its countdowns, and its countdowns.
pub(super) struct Seen {
    last: Last,
    countdowns: Vec<i32>,
    /// The part's version and availability when it was last compared.
    version: Option<(u64, bool)>,
}

#[derive(Serialize)]
struct Saved<'a> {
    digest: [u8; 16],
    countdowns: &'a [i32],
}
#[derive(Deserialize)]
struct Restored {
    digest: [u8; 16],
    countdowns: Vec<i32>,
}
impl Serialize for Seen {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let digest = match &self.last {
            Last::Digest(digest) => *digest,
            Last::Kept(kept) => kept.digest().map_err(serde::ser::Error::custom)?,
        };
        Saved {
            digest,
            countdowns: &self.countdowns,
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Seen {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Restored { digest, countdowns } = Restored::deserialize(deserializer)?;
        Ok(Self {
            last: Last::Digest(digest),
            countdowns,
            version: None,
        })
    }
}

#[cfg(test)]
thread_local! {
    static DIGESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The parts this thread has digested.
#[cfg(test)]
pub(super) fn digests() -> usize {
    DIGESTS.with(std::cell::Cell::get)
}

fn digest(
    feed: impl FnOnce(&mut blake3::Hasher) -> serde_json::Result<()>,
) -> io::Result<[u8; 16]> {
    let started = replay_frame::cpu::thread_cycles();
    let mut hasher = blake3::Hasher::new();
    feed(&mut hasher)?;
    let mut digest = [0; 16];
    digest.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    replay_frame::cpu::note("retained digest", "event_feed", started);
    #[cfg(test)]
    DIGESTS.with(|digests| digests.set(digests.get() + 1));
    Ok(digest)
}

/// Each part as last seen.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Sections(
    #[serde(with = "crate::prepared_production::map_pairs")] BTreeMap<Slot, Seen>,
);

impl Sections {
    /// Records `slot` as it stands: what it holds but its countdowns (`now`, kept by `keep`), and
    /// its `countdowns`. Whether it changed since it was last seen: what it holds differs, or a
    /// countdown rose, appeared or went; if so, it as kept and its countdowns. A part first seen
    /// while unavailable (`available` false) is recorded without being a change. A part seen
    /// again at the `version` and availability it was compared at holds the same, so is not
    /// compared again.
    fn see(
        &mut self,
        slot: Slot,
        version: Option<u64>,
        available: bool,
        now: View<'_>,
        countdowns: impl Iterator<Item = i32>,
        mut keep: impl FnMut() -> Arc<Kept>,
    ) -> io::Result<Option<(Arc<Kept>, Vec<i32>)>> {
        let version = version.map(|version| (version, available));
        let seen = match self.0.entry(slot) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                let seen = entry.insert(Seen {
                    last: Last::Kept(keep()),
                    countdowns: countdowns.collect(),
                    version,
                });
                return Ok(available.then(|| seen.changed()).flatten());
            }
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
        };
        let mut changed = false;
        if version.is_none() || seen.version != version {
            let same = match &seen.last {
                Last::Digest(held) => {
                    let taken = digest(|hasher| now.feed(hasher))?;
                    Err((*held == taken, taken))
                }
                Last::Kept(kept) => Ok(kept.view().same(&now)),
            };
            changed = match same {
                // A saved part is kept again, with the digest just taken of it.
                Err((same, taken)) => {
                    let kept = keep();
                    let _ = kept.digest.set(taken);
                    seen.last = Last::Kept(kept);
                    !same
                }
                Ok(true) => false,
                Ok(false) => {
                    seen.last = Last::Kept(keep());
                    true
                }
            };
            seen.version = version;
        } else {
            replay_frame::cpu::note(
                "retained digest skipped",
                "event_feed",
                replay_frame::cpu::thread_cycles(),
            );
        }
        let mut at = 0;
        for now in countdowns {
            match seen.countdowns.get_mut(at) {
                Some(was) => {
                    changed |= now > *was;
                    *was = now;
                }
                None => {
                    changed = true;
                    seen.countdowns.push(now);
                }
            }
            at += 1;
        }
        if at != seen.countdowns.len() {
            changed = true;
            seen.countdowns.truncate(at);
        }
        if changed && let Last::Digest(_) = seen.last {
            seen.last = Last::Kept(keep());
        }
        Ok(changed.then(|| seen.changed()).flatten())
    }
    /// Forgets each part's version and kept copy, so each is digested when next seen, as a
    /// saved observation's are.
    #[cfg(test)]
    pub(super) fn forget_versions(&mut self) {
        for seen in self.0.values_mut() {
            seen.version = None;
            let held = match &seen.last {
                Last::Kept(kept) => kept.digest().expect("a kept part digests"),
                Last::Digest(held) => *held,
            };
            seen.last = Last::Digest(held);
        }
    }
}

impl Seen {
    fn changed(&self) -> Option<(Arc<Kept>, Vec<i32>)> {
        match &self.last {
            Last::Kept(kept) => Some((Arc::clone(kept), self.countdowns.clone())),
            Last::Digest(_) => None,
        }
    }
}

/// An owner's tag at the read, or what names it without one.
fn named(
    country: EventFeedCountry,
    tag_at: &mut impl FnMut(EventFeedCountry) -> Option<String>,
) -> Result<String, String> {
    tag_at(country).ok_or_else(|| {
        format!(
            "country {} instance {}",
            country.reference, country.instance
        )
    })
}

/// A part the read changed: it as kept, its countdowns now, and its owner and the owner's name.
struct Changed {
    kept: Arc<Kept>,
    countdowns: Vec<i32>,
    owner: Option<EventFeedCountry>,
    name: Option<Result<String, String>>,
}

impl Changed {
    fn notes(&self, notes: &mut Vec<String>) {
        let name = match &self.name {
            Some(Ok(name) | Err(name)) => name.as_str(),
            None => "",
        };
        match &self.kept.part {
            Part::Pending(kept) => pending_notes(rows(kept), notes),
            Part::Fired(kept) => fired_notes(rows(kept), notes),
            Part::Delayed(_, kept) => delayed_notes(name, rows(kept), &self.countdowns, notes),
            Part::Decisions(decided) => decision_notes(name, decided, &self.countdowns, notes),
        }
    }
}

/// `slot` as it stands, when it changed since it was last seen.
fn part(
    sections: &mut Sections,
    contents: &mut Contents,
    state: &ScriptEventState,
    slot: Slot,
) -> io::Result<Option<Changed>> {
    let (seen, owner) = match slot {
        Slot::Pending => {
            let pending = state.pending();
            let keep = || Arc::new(Kept::new(Part::Pending(pending.map(<[_]>::to_vec))));
            let version = state.pending_version();
            let now = View::Pending(pending);
            let seen = sections.see(
                slot,
                version,
                pending.is_ok(),
                now,
                std::iter::empty(),
                keep,
            )?;
            (seen, None)
        }
        Slot::Fired => {
            let fired = state.fired();
            let keep = || Arc::new(Kept::new(Part::Fired(fired.map(<[_]>::to_vec))));
            let version = state.fired_version();
            let now = View::Fired(fired);
            let seen = sections.see(slot, version, fired.is_ok(), now, std::iter::empty(), keep)?;
            (seen, None)
        }
        Slot::Delayed(id, instance) => {
            let Some((country, version, timers)) = state.delayed_owner((id, instance)) else {
                return Ok(None);
            };
            let keep = || Arc::new(Kept::new(Part::Delayed(country, timers.map(<[_]>::to_vec))));
            let countdowns = timers
                .unwrap_or_default()
                .iter()
                .map(|timer| timer.remaining);
            let now = View::Delayed(country, timers);
            let seen = sections.see(slot, version, timers.is_ok(), now, countdowns, keep)?;
            (seen, Some(country))
        }
        Slot::Decisions(id, instance) => {
            let Some(owner) = state.decision_owner((id, instance)) else {
                return Ok(None);
            };
            let complete = owner.complete();
            let members: [&[RetainedDecision]; CONTAINERS] =
                std::array::from_fn(|container| owner.members(container as u8).unwrap_or_default());
            let now = View::Decisions(
                owner.country,
                complete.then(|| Holds {
                    members,
                    random_counts: &owner.random_counts,
                    ignored: &owner.ignored,
                    candidates: &owner.candidates,
                }),
            );
            let days = members
                .into_iter()
                .flatten()
                .filter(|_| complete)
                .map(|member| member.days);
            let keep = || decided(contents, id, owner);
            let seen = sections.see(slot, owner.version(), complete, now, days, keep)?;
            (seen, Some(owner.country))
        }
    };
    Ok(seen.map(|(kept, countdowns)| Changed {
        kept,
        countdowns,
        owner,
        name: None,
    }))
}

fn pending_notes(rows: Result<&[PendingEvent], &str>, notes: &mut Vec<String>) {
    match rows {
        Ok(rows) => {
            notes.push(format!("Pending event instances: {}.", rows.len()));
            for row in rows {
                let label = format!(
                    "Pending instance {}: {}; due hour {}; originator {:?}",
                    row.instance,
                    identity(&row.definition),
                    row.due_hour,
                    row.originator
                );
                notes.push(label.clone());
                scopes(notes, &label, &row.scopes);
            }
        }
        Err(reason) => notes.push(format!("Pending event instances unavailable: {reason}.")),
    }
}

fn fired_notes(rows: Result<&[ScriptIdentity], &str>, notes: &mut Vec<String>) {
    match rows {
        Ok(rows) => {
            notes.push(format!(
                "fired_event_names membership: {} (duplicates retained).",
                rows.len()
            ));
            notes.extend(rows.iter().map(|row| {
                format!(
                    "Fired-set member: {}. Fire time is not inferred from membership.",
                    identity(row)
                )
            }));
        }
        Err(reason) => notes.push(format!("fired_event_names unavailable: {reason}.")),
    }
}

/// Its timers, each with its remaining hours now (`remaining`).
fn delayed_notes(
    name: &str,
    rows: Result<&[DelayedEvent], &str>,
    remaining: &[i32],
    notes: &mut Vec<String>,
) {
    match rows {
        Ok(rows) => {
            notes.push(format!("{name}: {} delayed event timers.", rows.len()));
            for (row, remaining) in rows.iter().zip(remaining) {
                let label = format!(
                    "{name} delayed instance {}: {}; remaining {} hours; originator {:?}",
                    row.instance,
                    identity(&row.definition),
                    remaining,
                    row.originator
                );
                notes.push(label.clone());
                scopes(notes, &label, &row.scopes);
            }
        }
        Err(reason) => notes.push(format!("{name} delayed timers unavailable: {reason}.")),
    }
}

/// Its members, each with its days now (`days`, in container and member order).
fn decision_notes(name: &str, owner: &Decided, days: &[i32], notes: &mut Vec<String>) {
    let containers = [
        "taken",
        "re-enable",
        "removal",
        "timed",
        "targeted",
        "timed targeted",
    ];
    if !owner.complete {
        notes.push(format!(
            "{name} decisions unavailable: owner baseline/writer/pass certificate incomplete."
        ));
        return;
    }
    let members: usize = owner.members.iter().map(Vec::len).sum();
    notes.push(format!(
        "{name} decisions: {members} retained members, {} description counts, {} ignored, {} candidates.",
        owner.random_counts.len(),
        owner.ignored.len(),
        owner.candidates.len()
    ));
    let members = owner
        .members
        .iter()
        .enumerate()
        .flat_map(|(container, members)| members.iter().map(move |member| (container, member)));
    for ((container, member), days) in members.zip(days) {
        notes.push(format!("{name} {}: {} instance {}; duration {} days; lifecycle {}; flags {}; country target {:?}; numeric target {:?}.",containers[container],String::from_utf8_lossy(&member.token),member.instance,days,member.lifecycle,member.flags,member.country_target,member.numeric_target));
    }
    for count in &owner.random_counts {
        notes.push(format!(
            "{name} description count: {}; target {}; count {}.",
            String::from_utf8_lossy(&count.token),
            count.target,
            count.count
        ));
    }
    for token in &owner.ignored {
        notes.push(format!(
            "{name} ignored decision: {}.",
            String::from_utf8_lossy(token)
        ));
    }
    for (kind, token, target, root) in &owner.candidates {
        notes.push(format!(
            "{name} candidate kind {kind}: {}; target {target}; ROOT reference {root}.",
            String::from_utf8_lossy(token)
        ));
    }
}

/// A read's own observation notes, and the tags of the countries they name.
pub(crate) type Observations = (Vec<String>, BTreeSet<String>);

/// The notes of `pack`'s own retained-state observations, named at their hours by `tag_at`.
pub(crate) fn observations(
    pack: &EventFeedHistoryRow,
    tag_at: &mut impl FnMut(i32, EventFeedCountry) -> Option<String>,
) -> Observations {
    let mut notes = Vec::new();
    let mut tags = BTreeSet::new();
    for event in &pack.events {
        let Some(script) = event.script else {
            continue;
        };
        let Some(change) = script.change else {
            continue;
        };
        if matches!(change, Change::Decision(_) | Change::DelayedAdvance { .. }) {
            continue;
        }
        for country in [event.actor, event.target] {
            if country.reference as i32 > 0
                && let Some(tag) = tag_at(event.hour, country)
            {
                tags.insert(tag);
            }
        }
        let token = &pack.script_tokens
            [script.token_start as usize..script.token_start as usize + script.token_len as usize];
        notes.push(format!("Observation {} at {}: {:?}; content {}; identity {:?}; instance {:?}; originator {:?}; owner {:?}.",event.sequence,event.hour,change,String::from_utf8_lossy(token),script.native_event,script.instance,event.actor,event.target));
        scopes(
            &mut notes,
            &format!("observation {}", event.sequence),
            &pack.script_scopes[script.scope_start as usize
                ..script.scope_start as usize + script.scope_len as usize],
        );
    }
    (notes, tags)
}

/// A read's retained state observation, decided but not yet written: its own notes, then each
/// part it changed as it stands at the read.
pub(crate) struct Observed {
    generation: u64,
    read_hour: i32,
    notes: Vec<String>,
    tags: BTreeSet<String>,
    parts: Vec<Changed>,
}

/// The read's retained state observation, with the read's own `observations`: none when it
/// changed no part and makes no notes of its own, which a read whose only changes are
/// countdown passes does not.
pub(super) fn observe(
    pack: &PackHeader,
    state: &ScriptEventState,
    sections: &mut Sections,
    contents: &mut Contents,
    (notes, tags): Observations,
    tag_at: &mut impl FnMut(i32, EventFeedCountry) -> Option<String>,
) -> io::Result<Option<Observed>> {
    let touched = state.touched();
    let all = touched.all || sections.0.is_empty();
    let mut slots = Vec::new();
    if all || touched.pending {
        slots.push(Slot::Pending);
    }
    if all || touched.fired {
        slots.push(Slot::Fired);
    }
    if all {
        slots.extend(
            state
                .delayed_keys()
                .map(|(id, instance)| Slot::Delayed(id, instance)),
        );
        slots.extend(
            state
                .decision_keys()
                .map(|(id, instance)| Slot::Decisions(id, instance)),
        );
        // A part the state no longer holds is seen anew if it returns.
        sections
            .0
            .retain(|slot, _| slots.binary_search(slot).is_ok());
    } else {
        slots.extend(
            touched
                .delayed
                .iter()
                .map(|&(id, instance)| Slot::Delayed(id, instance)),
        );
        slots.extend(
            touched
                .decisions
                .iter()
                .map(|&(id, instance)| Slot::Decisions(id, instance)),
        );
    }
    let mut parts = Vec::new();
    for slot in slots {
        parts.extend(part(sections, contents, state, slot)?);
    }
    if notes.is_empty() && parts.is_empty() {
        return Ok(None);
    }
    for changed in &mut parts {
        if let Some(country) = changed.owner {
            changed.name = Some(named(country, &mut |country| {
                tag_at(pack.read_hour, country)
            }));
        }
    }
    Ok(Some(Observed {
        generation: pack.generation,
        read_hour: pack.read_hour,
        notes,
        tags,
        parts,
    }))
}

impl Observed {
    /// The bytes it owns, itself and the kept copies of its parts included.
    pub(crate) fn heap(&self) -> usize {
        size_of::<Self>()
            + self.notes.capacity() * size_of::<String>()
            + self.notes.iter().map(String::capacity).sum::<usize>()
            + self
                .tags
                .iter()
                .map(|tag| size_of::<String>() + tag.capacity())
                .sum::<usize>()
            + self.parts.capacity() * size_of::<Changed>()
            + self
                .parts
                .iter()
                .map(|part| {
                    part.kept.heap()
                        + part.countdowns.capacity() * size_of::<i32>()
                        + match &part.name {
                            Some(Ok(name) | Err(name)) => name.capacity(),
                            None => 0,
                        }
                })
                .sum::<usize>()
    }
    pub(crate) fn read_hour(&self) -> i32 {
        self.read_hour
    }
    /// The observation's row; with `digest`, each kept part's digest is taken here too.
    pub(crate) fn row(self, digest: bool) -> io::Result<EventFeedRow> {
        let Self {
            generation,
            read_hour,
            notes,
            mut tags,
            parts,
        } = self;
        let mut cause = vec![format!(
            "Retained state changed at read hour {read_hour}: each part listed is as it stands at this read; a countdown that only ran down is not a change. Baseline membership has no recorded fire or selection time."
        )];
        cause.extend(notes);
        for changed in &parts {
            if digest {
                changed.kept.digest()?;
            }
            changed.notes(&mut cause);
            if let Some(Ok(name)) = &changed.name {
                tags.insert(name.clone());
            }
        }
        let kind = EventFeedKind::ScriptStateObserved;
        Ok(EventFeedRow {
            id: format!("script-state:{generation}:{read_hour}"),
            hour: read_hour,
            read_hour,
            name_hour: read_hour,
            exact: false,
            category: kind.category(),
            kind,
            tags: tags.into_iter().collect(),
            participants: None,
            item: None,
            amount: None,
            cause: Some(EventFeedCause {
                how: None,
                within: None,
                conference: None,
                state: None,
                ally: None,
                reason: None,
                notes: cause,
            }),
            strike: None,
        })
    }
}
