//! Sparse native aid lists, indexed against the army's live division inventory: each hour
//! projects the senders whose volunteers, expeditionaries or host tallies may have changed.
use super::inventory::{Applied, Inventory, Sparse};
use dashboard_base::prepared::frame_changes::field_mask;
use replay::segments::State;
use replay_data::rows::{self, tables};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

/// An aid party: a country identity, as its sender lists and host tallies name it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Party {
    pub slot: u32,
    pub id: u32,
    pub tag: Arc<str>,
}

/// One aid channel of a sender: complete when every listed reference was resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Channel {
    pub complete: bool,
    pub values: BTreeMap<Party, i64>,
    pub reasons: BTreeMap<String, usize>,
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

/// A sender's projection: land volunteers, expeditionaries and the first matching native
/// host tally, with the native occurrences they were read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sender {
    pub party: Party,
    pub channels: [Channel; 3],
    /// Every native host occurrence, never added as separate shipments.
    pub hosts: Vec<(u32, u32, u32, i32)>,
    pub volunteers: Vec<(u32, u32, u64)>,
    pub expeditionaries: Option<Vec<u64>>,
}

/// One hour of aid.
#[derive(Clone, Debug, Default)]
pub struct Projection {
    /// The parties present, when they may have changed.
    pub parties: Option<Vec<Party>>,
    pub senders: Vec<Sender>,
    /// Whether each channel's sources were read, and every sender resolved.
    pub reads: [bool; 3],
    pub reasons: [BTreeMap<String, usize>; 3],
}

impl Projection {
    pub fn heap(&self) -> usize {
        let channels: usize = self
            .senders
            .iter()
            .flat_map(|sender| &sender.channels)
            .map(|channel| channel.values.len() * 48 + channel.reasons.len() * 48)
            .sum();
        self.parties
            .as_ref()
            .map_or(0, |parties| parties.capacity() * size_of::<Party>())
            + self.senders.capacity() * size_of::<Sender>()
            + channels
    }
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
    parties: BTreeMap<u32, Party>,
    aliases: BTreeMap<u32, Vec<u32>>,
    previous: Option<i32>,
    valid: Option<bool>,
    reads: Option<[bool; 3]>,
    divisions_known: Option<bool>,
    readiness: BTreeMap<u32, [bool; 3]>,
    incomplete: [usize; 3],
    orphaned: [BTreeSet<u32>; 3],
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

/// What the hour's walk gives the aid lists.
pub(super) struct Inputs<'a> {
    pub state: &'a State,
    pub format: u16,
    pub hour: i32,
    /// The segment's first hour: every table is read whole.
    pub whole: bool,
    /// CountryLinks, while whole.
    pub links: Option<&'a [rows::CountryLinkRow]>,
    /// The hour changed a link: an alias can name another country.
    pub links_changed: bool,
    /// Countries and CountryLinks are whole and the hour is not in a recording gap.
    pub valid: bool,
    /// The hour's tagged countries, by tag: (tag, slot, id).
    pub identities: &'a [(Arc<str>, (u32, u32))],
    pub identity_changed: bool,
    /// The hour changed Divisions rows.
    pub divisions_changed: bool,
    /// The walk's divisions, which aid references resolve against.
    pub divisions: &'a dashboard_world::Divisions,
}

impl Runtime {
    pub(super) fn step(&mut self, inputs: Inputs<'_>, inventory: &Inventory) -> Projection {
        let continuing = self
            .previous
            .is_some_and(|hour| hour.checked_add(1) == Some(inputs.hour));
        let raw_dirty = self.index_sender_lists(&inputs, continuing);
        let links = inputs.links;
        let valid = inputs.valid;
        let parties_changed = !continuing
            || inputs.identity_changed
            || inputs.links_changed
            || !valid
            || self.valid != Some(valid);
        let parties =
            parties_changed.then(|| self.refresh_parties(links, inputs.identities, valid));
        let reads = self.source_reads(valid);
        let all = parties_changed
            || self.reads != Some(reads)
            || self.divisions_known != Some(inventory.known())
            || (inputs.divisions_changed && inventory.aid_changes.is_none());
        let dirty = self.affected_senders(raw_dirty, links, &inputs, inventory, all);
        let senders = self.project_links(dirty, links, (inventory, inputs.divisions), reads);
        self.update_coverage(parties_changed, &senders);
        self.divisions_known = Some(inventory.known());
        self.previous = Some(inputs.hour);
        self.valid = Some(valid);
        self.reads = Some(reads);
        Projection {
            parties,
            senders,
            reads: self.read_coverage(reads),
            reasons: self.coverage_reasons(),
        }
    }

    fn index_sender_lists(&mut self, inputs: &Inputs<'_>, continuing: bool) -> BTreeSet<u32> {
        let at = (inputs.format, inputs.hour);
        let whole = inputs.whole || !continuing;
        let mut raw_dirty = BTreeSet::new();
        let sent = self.sent.apply(inputs.state, at, u64::MAX, true, whole);
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
            inputs.state,
            at,
            field_mask(
                rows::TableId::VolunteerHosts,
                &["host", "deployed_divisions"],
            ),
            true,
            whole,
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
            inputs.state,
            at,
            field_mask(
                rows::TableId::CountryCounterInputs,
                &["sent_expeditionaries"],
            ),
            true,
            whole,
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

    fn refresh_parties(
        &mut self,
        links: Option<&[rows::CountryLinkRow]>,
        identities: &[(Arc<str>, (u32, u32))],
        valid: bool,
    ) -> Vec<Party> {
        self.parties = if valid {
            identities
                .iter()
                .map(|(tag, (slot, id))| {
                    (
                        *slot,
                        Party {
                            slot: *slot,
                            id: *id,
                            tag: Arc::clone(tag),
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
        self.parties.values().cloned().collect()
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
        inputs: &Inputs<'_>,
        inventory: &Inventory,
        all: bool,
    ) -> BTreeSet<u32> {
        let mut dirty: BTreeSet<_> = if all {
            self.parties.keys().copied().collect()
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
                .filter(|slot| self.parties.contains_key(slot));
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
        if inputs.divisions_changed {
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
        (inventory, divisions): (&Inventory, &dashboard_world::Divisions),
        reads: [bool; 3],
    ) -> Vec<Sender> {
        let mut senders = Vec::new();
        for slot in dirty {
            let Some(party) = self.parties.get(&slot).cloned() else {
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
                divisions,
                reads,
            };
            let mut references = BTreeSet::new();
            let volunteers = inputs.volunteer_occurrences();
            let land = inputs
                .project_volunteers(&mut references)
                .unwrap_or_else(Channel::refused);
            let mut expeditionaries = None;
            let expeditionary = inputs
                .project_expeditionaries(&party, &mut references, &mut expeditionaries)
                .unwrap_or_else(Channel::refused);
            let hosts = inputs.host_occurrences();
            let tally = inputs.project_tallies().unwrap_or_else(Channel::refused);
            senders.push(Sender {
                party,
                channels: [land, expeditionary, tally],
                hosts,
                volunteers,
                expeditionaries,
            });
            self.remember_references(slot, references);
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

    fn update_coverage(&mut self, parties_changed: bool, senders: &[Sender]) {
        if parties_changed {
            let retired: Vec<_> = self
                .readiness
                .keys()
                .filter(|slot| !self.parties.contains_key(slot))
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
            if let Some(old) = self.readiness.insert(sender.party.slot, known) {
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
    divisions: &'a dashboard_world::Divisions,
    reads: [bool; 3],
}

impl SenderInputs<'_> {
    fn resolve(&self, raw: u32) -> Option<Party> {
        self.links
            .and_then(|links| dashboard_text::metadata::link_slot(links, raw))
            .and_then(|slot| self.runtime.parties.get(&slot))
            .cloned()
    }

    fn resolve_division(
        &self,
        reference: u64,
        value: &mut Channel,
        missing: [&str; 2],
    ) -> Option<rows::DivisionRow> {
        let division = (reference != 0)
            .then(|| self.divisions.division(reference))
            .flatten()
            .map(dashboard_world::Division::row);
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
                .resolve(replay_data::military_aid::volunteer_host(&division))
                .ok_or("volunteer logical host unresolved")?;
            *value.values.entry(host).or_default() += 1;
        }
        Ok(value)
    }

    fn project_expeditionaries(
        &self,
        party: &Party,
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
                .resolve(replay_data::military_aid::owner(&division))
                .as_ref()
                != Some(party)
            {
                value.complete = false;
                value.reason("expeditionary list and source ownership conflict");
                continue;
            }
            let host = self
                .resolve(division.roster_owner)
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
                .resolve(row.host)
                .ok_or("VolunteerHosts host unresolved")?;
            value
                .values
                .entry(host)
                .or_insert(i64::from(row.deployed_divisions));
        }
        Ok(value)
    }
}
