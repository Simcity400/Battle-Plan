//! Sparse native aid lists, indexed against the army's existing live division inventory.
use super::{
    Identity, Inventory,
    inventory::{Applied, Sparse, Update},
};
use dashboard_base::prepared::{
    frame_changes::{FrameChanges, field_mask},
    series::{self, Accumulator},
};
use replay_data::rows::{self, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct Actor {
    pub(crate) slot: u32,
    pub(crate) id: u32,
    pub(crate) tag: String,
}
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Channel {
    complete: bool,
    #[serde(with = "actor_values")]
    values: BTreeMap<Actor, i64>,
    reasons: BTreeMap<String, usize>,
}
impl Channel {
    fn refused(reason: impl Into<String>) -> Self {
        Self {
            reasons: [(reason.into(), 1)].into(),
            ..Self::default()
        }
    }
    fn reason(&mut self, reason: &str) {
        *self.reasons.entry(reason.into()).or_default() += 1;
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Sender {
    actor: Actor,
    /// Land occurrences, expeditionary occurrences, first matching native host tally.
    channels: [Channel; 3],
    /// Every native host occurrence, never added as separate shipments.
    hosts: Vec<(u32, u32, u32, i32)>,
    volunteers: Vec<(u32, u32, u64)>,
    expeditionaries: Option<Vec<u64>>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Projection {
    hour: i32,
    actors: Option<Vec<Actor>>,
    senders: Vec<Sender>,
    reads: [bool; 3],
    reasons: [BTreeMap<String, usize>; 3],
}

#[derive(Default)]
pub(super) struct Runtime {
    sent: Sparse<tables::SentVolunteers>,
    hosts: Sparse<tables::VolunteerHosts>,
    counters: Sparse<tables::CountryCounterInputs>,
    lists: BTreeMap<u32, BTreeMap<u32, rows::SentVolunteerRow>>,
    host_lists: BTreeMap<u32, BTreeMap<u32, rows::VolunteerHostRow>>,
    counter_lists: BTreeMap<u32, rows::CountryCounterInputRow>,
    refs: HashMap<u32, BTreeSet<u64>>,
    users: HashMap<u64, BTreeSet<u32>>,
    actors: BTreeMap<u32, Actor>,
    aliases: BTreeMap<u32, Vec<u32>>,
    previous: Option<(u64, i32)>,
    valid: Option<bool>,
    reads: Option<[bool; 3]>,
    divisions_known: Option<bool>,
    readiness: BTreeMap<u32, [bool; 3]>,
    incomplete: [usize; 3],
    orphaned: [BTreeSet<u32>; 3],
    #[cfg(test)]
    projections: usize,
}
fn edits<R>(applied: Applied<R>, mut edit: impl FnMut(Option<R>, Option<R>)) {
    if let Applied::Reset(rows) | Applied::Changes(rows) = applied {
        for (old, new) in rows {
            edit(old, new);
        }
    }
}
fn grouped<R>(
    groups: &mut BTreeMap<u32, BTreeMap<u32, R>>,
    old: Option<R>,
    new: Option<R>,
    key: impl Fn(&R) -> (u32, u32),
) {
    if let Some(row) = old {
        let (country, at) = key(&row);
        if let Some(list) = groups.get_mut(&country) {
            list.remove(&at);
            if list.is_empty() {
                groups.remove(&country);
            }
        }
    }
    if let Some(row) = new {
        let (country, at) = key(&row);
        groups.entry(country).or_default().insert(at, row);
    }
}
impl Runtime {
    pub(super) fn step(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        inventory: &Inventory,
        identities: &[(String, Identity)],
        identity_changed: bool,
        in_gap: bool,
    ) -> Projection {
        self.step_input(
            frame,
            changes,
            inventory,
            identities,
            identity_changed,
            in_gap,
            None,
        )
    }

    pub(super) fn step_input(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        inventory: &Inventory,
        identities: &[(String, Identity)],
        identity_changed: bool,
        in_gap: bool,
        columnar: Option<Update<'_>>,
    ) -> Projection {
        let continuing = self.previous.is_some_and(|(branch, hour)| {
            branch == frame.branch && hour.checked_add(1) == Some(frame.hour)
        });
        let raw_dirty = self.index_sender_lists(frame, changes, continuing, columnar);
        let links = frame.complete::<tables::CountryLinks>().ok();
        let valid = !in_gap && links.is_some() && frame.complete::<tables::Countries>().is_ok();
        let actors_changed = !continuing || identity_changed || !valid || self.valid != Some(valid);
        let actors = actors_changed.then(|| self.refresh_actors(links, identities, valid));
        let reads = self.source_reads(valid);
        let all = actors_changed
            || self.reads != Some(reads)
            || self.divisions_known != Some(inventory.known())
            || (changes.rows_changed[TableId::Divisions.index()]
                && inventory.aid_changes.is_none());
        let dirty = self.affected_senders(raw_dirty, links, changes, inventory, all);
        let senders = self.project_links(dirty, links, inventory, reads);
        self.update_coverage(actors_changed, &senders);
        self.divisions_known = Some(inventory.known());
        self.previous = Some((frame.branch, frame.hour));
        self.valid = Some(valid);
        self.reads = Some(reads);
        Projection {
            hour: frame.hour,
            actors,
            senders,
            reads: self.read_coverage(reads),
            reasons: self.coverage_reasons(),
        }
    }

    fn index_sender_lists(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        continuing: bool,
        columnar: Option<Update<'_>>,
    ) -> BTreeSet<u32> {
        let update = |table: TableId| {
            if let Some(update) = columnar {
                return update;
            }
            if continuing && !changes.rows_changed[table.index()] {
                Update::Unchanged(self.previous.unwrap().1)
            } else {
                Update::Decoded(continuing.then(|| changes.rows(table)).flatten())
            }
        };
        let mut raw_dirty = BTreeSet::new();
        let sent = self
            .sent
            .apply(update(TableId::SentVolunteers), frame, u64::MAX, true);
        if matches!(sent, Applied::Missing) {
            self.lists.clear();
        }
        edits(sent, |old, new| {
            raw_dirty.extend(
                old.as_ref()
                    .into_iter()
                    .chain(new.as_ref())
                    .map(|row| row.country),
            );
            grouped(&mut self.lists, old, new, |row| (row.country, row.ordinal));
        });
        let hosts = self.hosts.apply(
            update(TableId::VolunteerHosts),
            frame,
            field_mask(TableId::VolunteerHosts, &["host", "deployed_divisions"]),
            true,
        );
        if matches!(hosts, Applied::Missing) {
            self.host_lists.clear();
        }
        edits(hosts, |old, new| {
            raw_dirty.extend(
                old.as_ref()
                    .into_iter()
                    .chain(new.as_ref())
                    .map(|row| row.country),
            );
            grouped(&mut self.host_lists, old, new, |row| {
                (row.country, row.position)
            });
        });
        let counters = self.counters.apply(
            update(TableId::CountryCounterInputs),
            frame,
            field_mask(TableId::CountryCounterInputs, &["sent_expeditionaries"]),
            true,
        );
        if matches!(counters, Applied::Missing) {
            self.counter_lists.clear();
        }
        edits(counters, |old, new| {
            if let Some(row) = old {
                raw_dirty.insert(row.country);
                self.counter_lists.remove(&row.country);
            }
            if let Some(row) = new {
                raw_dirty.insert(row.country);
                self.counter_lists.insert(row.country, row);
            }
        });
        raw_dirty
    }

    fn refresh_actors(
        &mut self,
        links: Option<&[rows::CountryLinkRow]>,
        identities: &[(String, Identity)],
        valid: bool,
    ) -> Vec<Actor> {
        self.actors = if valid {
            identities
                .iter()
                .map(|(tag, (slot, id))| {
                    (
                        *slot,
                        Actor {
                            slot: *slot,
                            id: *id,
                            tag: tag.clone(),
                        },
                    )
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        self.aliases.clear();
        for row in links.unwrap_or_default() {
            self.aliases
                .entry(row.canonical_slot)
                .or_default()
                .push(row.id);
        }
        self.actors.values().cloned().collect()
    }

    fn source_reads(&self, valid: bool) -> [bool; 3] {
        // Each complete hourly baseline supplies its own read proof.
        [
            self.sent.baseline.is_some(),
            self.counters.baseline.is_some(),
            self.hosts.baseline.is_some(),
        ]
        .map(|read| read && valid)
    }

    fn affected_senders(
        &mut self,
        mut raw_dirty: BTreeSet<u32>,
        links: Option<&[rows::CountryLinkRow]>,
        changes: &FrameChanges,
        inventory: &Inventory,
        all: bool,
    ) -> BTreeSet<u32> {
        let mut dirty: BTreeSet<_> = if all {
            self.actors.keys().copied().collect()
        } else {
            BTreeSet::new()
        };
        if all {
            raw_dirty.extend(self.lists.keys());
            raw_dirty.extend(self.host_lists.keys());
            raw_dirty.extend(self.counter_lists.keys());
            for owners in &self.orphaned {
                raw_dirty.extend(owners);
            }
        }
        for raw in raw_dirty {
            let slot = links
                .and_then(|links| dashboard_text::metadata::link_slot(links, raw))
                .filter(|slot| self.actors.contains_key(slot));
            for (channel, present) in [
                self.lists.contains_key(&raw),
                self.counter_lists.contains_key(&raw),
                self.host_lists.contains_key(&raw),
            ]
            .into_iter()
            .enumerate()
            {
                if present && slot.is_none() {
                    self.orphaned[channel].insert(raw);
                } else {
                    self.orphaned[channel].remove(&raw);
                }
            }
            if let Some(slot) = slot {
                dirty.insert(slot);
            }
        }
        if changes.rows_changed[TableId::Divisions.index()] {
            for reference in inventory.aid_changes.as_deref().unwrap_or_default() {
                if let Some(users) = self.users.get(reference) {
                    dirty.extend(users);
                }
            }
        }
        dirty
    }

    fn project_links(
        &mut self,
        dirty: BTreeSet<u32>,
        links: Option<&[rows::CountryLinkRow]>,
        inventory: &Inventory,
        reads: [bool; 3],
    ) -> Vec<Sender> {
        let mut senders = Vec::new();
        for slot in dirty {
            let Some(actor) = self.actors.get(&slot).cloned() else {
                continue;
            };
            self.forget_references(slot);
            let inputs = SenderInputs {
                runtime: self,
                aliases: self
                    .aliases
                    .get(&slot)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                links,
                inventory,
                reads,
            };
            let mut references = BTreeSet::new();
            let volunteers = inputs.volunteer_occurrences();
            let land = inputs
                .project_volunteers(&mut references)
                .unwrap_or_else(Channel::refused);
            let mut expeditionaries = None;
            let expeditionary = inputs
                .project_expeditionaries(&actor, &mut references, &mut expeditionaries)
                .unwrap_or_else(Channel::refused);
            let hosts = inputs.host_occurrences();
            let tally = inputs.project_tallies().unwrap_or_else(Channel::refused);
            let sender = Sender {
                actor,
                channels: [land, expeditionary, tally],
                hosts,
                volunteers,
                expeditionaries,
            };
            self.remember_references(slot, references);
            #[cfg(test)]
            {
                self.projections += 1;
            }
            senders.push(sender);
        }
        senders
    }

    fn forget_references(&mut self, slot: u32) {
        for reference in self.refs.remove(&slot).unwrap_or_default() {
            if let Some(users) = self.users.get_mut(&reference) {
                users.remove(&slot);
                if users.is_empty() {
                    self.users.remove(&reference);
                }
            }
        }
    }

    fn remember_references(&mut self, slot: u32, mut references: BTreeSet<u64>) {
        references.remove(&0);
        for &reference in &references {
            self.users.entry(reference).or_default().insert(slot);
        }
        self.refs.insert(slot, references);
    }

    fn update_coverage(&mut self, actors_changed: bool, senders: &[Sender]) {
        if actors_changed {
            let retired: Vec<_> = self
                .readiness
                .keys()
                .filter(|slot| !self.actors.contains_key(slot))
                .copied()
                .collect();
            for slot in retired {
                self.forget_references(slot);
                if let Some(old) = self.readiness.remove(&slot) {
                    for (channel, known) in old.into_iter().enumerate() {
                        self.incomplete[channel] -= usize::from(!known);
                    }
                }
            }
        }
        for sender in senders {
            let known = sender.channels.each_ref().map(|channel| channel.complete);
            if let Some(old) = self.readiness.insert(sender.actor.slot, known) {
                for (channel, known) in old.into_iter().enumerate() {
                    self.incomplete[channel] -= usize::from(!known);
                }
            }
            for (channel, known) in known.into_iter().enumerate() {
                self.incomplete[channel] += usize::from(!known);
            }
        }
    }

    fn read_coverage(&self, reads: [bool; 3]) -> [bool; 3] {
        std::array::from_fn(|channel| {
            reads[channel] && self.incomplete[channel] == 0 && self.orphaned[channel].is_empty()
        })
    }

    fn coverage_reasons(&self) -> [BTreeMap<String, usize>; 3] {
        std::array::from_fn(|channel| {
            let count = self.orphaned[channel].len();
            if count == 0 {
                BTreeMap::new()
            } else {
                [(
                    format!(
                        "{} sender reads with unresolved country identity",
                        ["SentVolunteers", "sent expeditionaries", "VolunteerHosts"][channel]
                    ),
                    count,
                )]
                .into()
            }
        })
    }
}

struct SenderInputs<'a> {
    runtime: &'a Runtime,
    aliases: &'a [u32],
    links: Option<&'a [rows::CountryLinkRow]>,
    inventory: &'a Inventory,
    reads: [bool; 3],
}

impl SenderInputs<'_> {
    fn resolve_actor(&self, raw: u32) -> Option<Actor> {
        self.links
            .and_then(|links| dashboard_text::metadata::link_slot(links, raw))
            .and_then(|slot| self.runtime.actors.get(&slot))
            .cloned()
    }

    fn resolve_division(
        &self,
        reference: u64,
        value: &mut Channel,
        missing: [&str; 2],
    ) -> Option<&rows::DivisionRow> {
        let division = self.inventory.division(reference);
        if division.is_none() {
            value.reason(missing[usize::from(reference != 0)]);
        }
        division
    }

    fn volunteer_occurrences(&self) -> Vec<(u32, u32, u64)> {
        self.aliases
            .iter()
            .filter_map(|raw| self.runtime.lists.get(raw))
            .flat_map(|list| list.values())
            .map(|row| (row.country, row.ordinal, row.division))
            .collect()
    }

    fn project_volunteers(&self, references: &mut BTreeSet<u64>) -> Result<Channel, String> {
        if !self.reads[0] {
            return Err("SentVolunteers unavailable".to_owned());
        }
        let lists: Vec<Vec<_>> = self
            .aliases
            .iter()
            .filter_map(|raw| self.runtime.lists.get(raw))
            .map(|list| list.values().cloned().collect())
            .collect();
        let list = replay_data::military_aid::volunteer_list(lists.iter().map(Vec::as_slice))?;
        if !self.inventory.known() && list.iter().any(|row| row.division != 0) {
            return Err("live Divisions unavailable for listed volunteers".into());
        }
        let mut value = Channel {
            complete: true,
            ..Default::default()
        };
        for row in list {
            references.insert(row.division);
            let Some(division) = self.resolve_division(
                row.division,
                &mut value,
                [
                    "null volunteer references skipped",
                    "nonlive volunteer references skipped",
                ],
            ) else {
                continue;
            };
            let host = self
                .resolve_actor(replay_data::military_aid::volunteer_host(division))
                .ok_or("volunteer logical host unresolved")?;
            *value.values.entry(host).or_default() += 1;
        }
        Ok(value)
    }

    fn project_expeditionaries(
        &self,
        actor: &Actor,
        references: &mut BTreeSet<u64>,
        expeditionaries: &mut Option<Vec<u64>>,
    ) -> Result<Channel, String> {
        if !self.reads[1] {
            return Err("CountryCounterInputs sent expeditionaries unavailable".to_owned());
        }
        let mut lists = self
            .aliases
            .iter()
            .filter_map(|raw| self.runtime.counter_lists.get(raw));
        let row = lists
            .next()
            .ok_or("sent expeditionaries sender row unavailable")?;
        if lists.next().is_some() {
            return Err("sent expeditionaries sender aliases ambiguous".into());
        }
        let list = row
            .sent_expeditionaries
            .require("sent expeditionaries")?
            .as_ref()
            .ok_or("sent expeditionaries list unavailable")?;
        let mut value = Channel {
            complete: true,
            ..Default::default()
        };
        *expeditionaries = Some(list.iter().copied().collect());
        if !self.inventory.known() && list.iter().any(|&reference| reference != 0) {
            return Err("live Divisions unavailable for listed expeditionaries".into());
        }
        for &reference in list {
            references.insert(reference);
            let Some(division) = self.resolve_division(
                reference,
                &mut value,
                [
                    "null expeditionary references skipped",
                    "nonlive expeditionary references skipped",
                ],
            ) else {
                continue;
            };
            if self
                .resolve_actor(replay_data::military_aid::owner(division))
                .as_ref()
                != Some(actor)
            {
                value.complete = false;
                value.reason("expeditionary list and source ownership conflict");
                continue;
            }
            let host = self
                .resolve_actor(division.roster_owner)
                .ok_or("expeditionary roster host unresolved")?;
            *value.values.entry(host).or_default() += 1;
        }
        Ok(value)
    }

    fn host_occurrences(&self) -> Vec<(u32, u32, u32, i32)> {
        self.aliases
            .iter()
            .filter_map(|raw| self.runtime.host_lists.get(raw))
            .flat_map(|list| list.values())
            .map(|row| (row.country, row.position, row.host, row.deployed_divisions))
            .collect()
    }

    fn project_tallies(&self) -> Result<Channel, String> {
        if !self.reads[2] {
            return Err("VolunteerHosts unavailable".to_owned());
        }
        let mut lists = self
            .aliases
            .iter()
            .filter_map(|raw| self.runtime.host_lists.get(raw))
            .filter(|list| !list.is_empty());
        let list = lists.next();
        if lists.next().is_some() {
            return Err("VolunteerHosts sender aliases ambiguous".into());
        }
        let mut value = Channel {
            complete: true,
            ..Default::default()
        };
        for row in list.into_iter().flat_map(|list| list.values()) {
            let host = self
                .resolve_actor(row.host)
                .ok_or("VolunteerHosts host unresolved")?;
            value
                .values
                .entry(host)
                .or_insert(i64::from(row.deployed_divisions));
        }
        Ok(value)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Link {
    pub(crate) sender: u64,
    pub(crate) host: u64,
    pub(crate) channels: u8,
}
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Catalogue {
    pub(crate) actors: Vec<Actor>,
    pub(crate) links: Vec<Link>,
    pub(crate) warnings: [BTreeMap<String, usize>; 3],
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct History {
    catalogue: Catalogue,
    values: Accumulator,
    proof: Accumulator,
    events: Accumulator,
    #[serde(skip)]
    actor_keys: BTreeMap<Actor, u64>,
    #[serde(skip)]
    link_keys: BTreeMap<(u64, u64), u64>,
    current: BTreeMap<u64, Sender>,
    present: BTreeSet<u64>,
    last: Option<i32>,
    #[serde(skip)]
    writer: series::CatalogueWriter,
}
// JSON object keys must be strings; native tuples/actors are rebuilt from the catalogue.

mod actor_values {
    use super::*;
    pub(super) fn serialize<S: serde::Serializer>(
        values: &BTreeMap<Actor, i64>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        values.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<Actor, i64>, D::Error> {
        Ok(Vec::<(Actor, i64)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}
impl History {
    fn indexes(&mut self) {
        if self.actor_keys.len() != self.catalogue.actors.len() {
            self.actor_keys = self
                .catalogue
                .actors
                .iter()
                .cloned()
                .enumerate()
                .map(|(at, actor)| (actor, at as u64))
                .collect();
        }
        if self.link_keys.len() != self.catalogue.links.len() {
            self.link_keys = self
                .catalogue
                .links
                .iter()
                .enumerate()
                .map(|(at, link)| ((link.sender, link.host), at as u64))
                .collect();
        }
    }
    fn actor(&mut self, actor: &Actor) -> u64 {
        if let Some(&key) = self.actor_keys.get(actor) {
            return key;
        }
        let key = self.catalogue.actors.len() as u64;
        self.catalogue.actors.push(actor.clone());
        self.actor_keys.insert(actor.clone(), key);
        key
    }
    pub(super) fn advance(&mut self, hour: i32) -> io::Result<()> {
        self.values.advance(hour)?;
        self.proof.advance(hour)?;
        self.events.advance(hour)
    }
    pub(super) fn observe(&mut self, projection: Projection) -> io::Result<()> {
        self.indexes();
        let hour = projection.hour;
        self.advance(hour)?;
        let gap = self
            .last
            .is_none_or(|last| last.checked_add(1) != Some(hour));
        for (channel, reasons) in projection.reasons.into_iter().enumerate() {
            for (reason, count) in reasons {
                *self.catalogue.warnings[channel].entry(reason).or_default() += count;
            }
        }
        for (channel, read) in projection.reads.into_iter().enumerate() {
            self.proof
                .observe_if_changed(channel as u64, hour, read.then_some(1), gap, false);
        }
        if let Some(actors) = projection.actors {
            let present: BTreeSet<_> = actors.iter().map(|actor| self.actor(actor)).collect();
            for &key in self.present.union(&present) {
                self.proof.observe_if_changed(
                    4 + key * 4 + 3,
                    hour,
                    present.contains(&key).then_some(1),
                    gap || !self.present.contains(&key),
                    false,
                );
            }
            for key in self.present.difference(&present) {
                for channel in 0..3 {
                    self.proof.observe(4 + key * 4 + channel, hour, None, false);
                }
                self.current.remove(key);
            }
            self.present = present;
        }
        for sender in projection.senders {
            let key = self.actor(&sender.actor);
            let old = self.current.remove(&key);
            let changed = old.as_ref() != Some(&sender);
            if changed {
                self.events.event(key, hour, &sender)?;
            }
            for channel in 0..3 {
                let next = &sender.channels[channel];
                if !gap
                    && old
                        .as_ref()
                        .is_some_and(|old| old.channels[channel] == *next)
                {
                    continue;
                }
                self.proof.observe_if_changed(
                    4 + key * 4 + channel as u64,
                    hour,
                    next.complete.then_some(1),
                    gap,
                    false,
                );
                if changed {
                    for (reason, count) in &next.reasons {
                        *self.catalogue.warnings[channel]
                            .entry(reason.clone())
                            .or_default() += count;
                    }
                }
                let hosts: BTreeSet<_> = next
                    .values
                    .keys()
                    .chain(
                        old.as_ref()
                            .into_iter()
                            .flat_map(|old| old.channels[channel].values.keys()),
                    )
                    .cloned()
                    .collect();
                for host in hosts {
                    if !gap
                        && old.as_ref().is_some_and(|old| {
                            let before = &old.channels[channel];
                            before.complete == next.complete
                                && before.values.get(&host) == next.values.get(&host)
                        })
                    {
                        continue;
                    }
                    let host_key = self.actor(&host);
                    let link_key = match self.link_keys.get(&(key, host_key)) {
                        Some(&key) => key,
                        None => {
                            let at = self.catalogue.links.len() as u64;
                            self.catalogue.links.push(Link {
                                sender: key,
                                host: host_key,
                                channels: 0,
                            });
                            self.link_keys.insert((key, host_key), at);
                            at
                        }
                    };
                    if next.values.contains_key(&host) {
                        self.catalogue.links[link_key as usize].channels |= 1 << channel;
                    }
                    let value = next
                        .complete
                        .then(|| i128::from(next.values.get(&host).copied().unwrap_or(0)));
                    self.values.observe_if_changed(
                        link_key * 3 + channel as u64,
                        hour,
                        value,
                        gap,
                        false,
                    );
                }
            }
            if changed && sender.channels[0].complete && sender.channels[2].complete {
                let land = &sender.channels[0].values;
                let tallies = &sender.channels[2].values;
                for host in land.keys().chain(tallies.keys()).collect::<BTreeSet<_>>() {
                    if land.get(host).copied().unwrap_or(0)
                        != tallies.get(host).copied().unwrap_or(0)
                    {
                        *self.catalogue.warnings[0]
                            .entry(
                                "VolunteerHosts tally differs from resolved volunteer list".into(),
                            )
                            .or_default() += 1;
                    }
                }
            }
            self.current.insert(key, sender);
        }
        self.last = Some(hour);
        Ok(())
    }
    pub(super) fn bytes(&self) -> usize {
        self.values.bytes() + self.proof.bytes() + self.events.bytes()
    }
    pub(super) fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.values
            .flush(writer, &format!("army-aid-values-{branch}"), 0, cancelled)?;
        self.proof
            .flush(writer, &format!("army-aid-proof-{branch}"), 0, cancelled)?;
        self.events.flush(
            writer,
            &format!("army-aid-occurrences-{branch}"),
            0,
            cancelled,
        )?;
        self.writer.write(
            writer,
            "army-aid",
            branch,
            &BTreeMap::new(),
            &self.catalogue,
        )
    }
}

use dashboard_base::prepared::prepass::{Held, HeldBytes};
impl Held for Actor {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.tag);
    }
}
impl Held for Channel {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.values).heap(&self.reasons);
    }
}
impl Held for Sender {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.actor)
            .heap(&self.channels)
            .heap(&self.hosts)
            .heap(&self.volunteers)
            .heap(&self.expeditionaries);
    }
}
impl Held for Projection {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.actors)
            .heap(&self.senders)
            .heap(&self.reasons);
    }
}
#[cfg(test)]
fn direct(frame: &DecodedFrame) -> Projection {
    let changes = FrameChanges::alone(frame, replay_data::format::FormatVersion::CURRENT);
    let mut state = super::ScanState::new(true);
    state.identities(frame);
    state.apply_updates(super::Updates::decoded(&changes, None, false), frame);
    Runtime::default().step(
        frame,
        &changes,
        state.inventory.as_ref().unwrap(),
        &state.identities,
        true,
        false,
    )
}
#[cfg(test)]
mod tests;
