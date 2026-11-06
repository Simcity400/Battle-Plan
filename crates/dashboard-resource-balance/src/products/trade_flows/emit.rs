//! Trade flows' emit: each flow's runs on the family's series publication (the first hour of its
//! current run, with the run's terms, aliases and integrals so far as an event at its start, and
//! its daily amount), whether each hour's read was known, and the deals each read left out by
//! seller and buyer.
//!
//! A flow's run goes on while the reads stay known and contiguous, no named country changes its
//! holder, and its terms stay the same. A named country's new holder or tag starts every flow's
//! run; an unknown read, an hour not walked or a flow no deal counts toward ends them. Missing
//! data has no run, never a zero.
//!
//! Every key is bound to the family's domain: each prepared hour is known.

use super::{
    FlowKey, Terms, flow_token,
    facts::{self, Facts, Group, Holder},
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use dashboard_base::products::resources::RESOURCE_DECIMALS;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A flow's keys (its actor is named by its flow token): the first hour of its current run,
/// each run's [`kind::RUN`] event at that hour, and its daily amount delivered.
pub const FACT: u64 = 0;
pub const DELIVERED: u64 = 1;
/// The family's keys: whether each read was known, and each group's deals left out (by group).
pub const KNOWN: u64 = 0;
pub const REFUSALS: u64 = 1;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: every resource named (`[Unsigned id, Text token]`, by id), every alias a run
/// named (`Text`, by alias), each group's deals left out over every read (`[Unsigned group,
/// Unsigned deals]`) and whether a read was ever known; each group's seller and buyer (`Null`, or
/// `[seller slot, seller id, buyer slot, buyer id]`), by group; and each flow's first aliases
/// (`[Unsigned seller, Unsigned buyer]`), by actor.
pub const RESOURCES: u64 = 0;
pub const ALIASES: u64 = 1;
pub const TOTALS: u64 = 2;
pub const READ: u64 = 3;
pub const GROUPS: u64 = 1 << 24;
pub const FIRSTS: u64 = 1 << 28;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds.
pub mod kind {
    /// `[Unsigned seller alias, Unsigned buyer alias, Integer delivered, Integer factories,
    /// Integer convoys assigned, Integer convoys required, Unsigned deals, Integer integral ×6]`
    /// on a flow's FACT key: the run's terms, and each integral through the hour before it.
    pub const RUN: u64 = 0;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved trade emit is not valid")
}

fn exceeded() -> io::Error {
    dashboard_core::invalid("Trade integral exceeds its integer range")
}

/// A flow's latest run: its first hour, its last once it no longer holds, its terms, and its
/// integrals through the hour before its first.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Current {
    first: i32,
    last: Option<i32>,
    terms: Terms,
    base: [i128; 6],
}

/// The integrals through `through` of a run from `first` holding `terms`, `base` before it.
pub(crate) fn integrals(
    base: [i128; 6],
    terms: Terms,
    first: i32,
    through: i32,
) -> Option<[i128; 6]> {
    let hours = i128::from(through) - i128::from(first) + 1;
    let mut totals = base;
    for (total, rate) in totals.iter_mut().zip(terms.rates()) {
        *total = total.checked_add(rate.checked_mul(hours)?)?;
    }
    Some(totals)
}

/// A flow, its keys and its latest run.
struct Flow {
    key: FlowKey,
    keys: [KeyHandle; 2],
    run: Option<Current>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// By actor.
    flows: Vec<Flow>,
    by_key: FastMap<FlowKey, u64>,
    /// The flows holding a run now, and how many of them name each holder slot.
    active: BTreeSet<u64>,
    named: FastMap<u32, u32>,
    holders: FastMap<u32, Holder>,
    /// Every alias a run named, by alias.
    aliases: Vec<Arc<str>>,
    alias_ids: FastMap<Arc<str>, u32>,
    /// Each group's REFUSALS key, by group, and each group's id.
    groups: Vec<(Group, KeyHandle)>,
    group_ids: FastMap<Group, u64>,
    /// The deals left out now, by group, and over every read.
    refusing: BTreeMap<u64, u32>,
    totals: Vec<u64>,
    resources: BTreeMap<u32, String>,
    known_key: Option<KeyHandle>,
    /// Whether the last hour was a known read, and whether one ever was.
    known: bool,
    read: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
    /// RESOURCES, ALIASES, TOTALS and READ changed since the last cut.
    stale: [bool; 4],
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
            flows: Vec::new(),
            by_key: FastMap::default(),
            active: BTreeSet::new(),
            named: FastMap::default(),
            holders: FastMap::default(),
            aliases: Vec::new(),
            alias_ids: FastMap::default(),
            groups: Vec::new(),
            group_ids: FastMap::default(),
            refusing: BTreeMap::new(),
            totals: Vec::new(),
            resources: BTreeMap::new(),
            known_key: None,
            known: false,
            read: false,
            last: None,
            first: None,
            stale: [true; 4],
        }
    }

    fn key(&mut self, writer: &mut Writer, place: Place, decimals: u8) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, decimals), Some(DOMAIN))
    }

    fn known_key(&mut self, writer: &mut Writer) -> io::Result<KeyHandle> {
        if let Some(key) = self.known_key {
            return Ok(key);
        }
        let place = Place {
            actor: FAMILY,
            measure: KNOWN,
            dimension: None,
        };
        let key = self.key(writer, place, 0)?;
        self.known_key = Some(key);
        Ok(key)
    }

    /// The alias of `slot`'s holder now, numbered when a run first names it.
    fn alias(&mut self, slot: u32) -> io::Result<u32> {
        let (tag, _) = self
            .holders
            .get(&slot)
            .ok_or_else(|| dashboard_core::invalid("A trade flow names a slot without a holder"))?;
        if let Some(&alias) = self.alias_ids.get(tag) {
            return Ok(alias);
        }
        let alias = u32::try_from(self.aliases.len())
            .map_err(|_| dashboard_core::invalid("Trade aliases exhausted"))?;
        let tag = Arc::clone(tag);
        self.aliases.push(Arc::clone(&tag));
        self.alias_ids.insert(tag, alias);
        self.stale[1] = true;
        Ok(alias)
    }

    /// The flow actor of `key`, its keys and first aliases catalogued at `hour` when it is new.
    fn flow(&mut self, writer: &mut Writer, key: FlowKey, hour: i32) -> io::Result<u64> {
        if let Some(&actor) = self.by_key.get(&key) {
            return Ok(actor);
        }
        let (seller, buyer, resource) = key;
        let name = flow_token(seller, buyer, resource);
        let actor = self.actors.actor(writer, name.as_bytes(), hour)?;
        if actor != self.flows.len() as u64 {
            return Err(dashboard_core::invalid("Trade flow actors are not dense"));
        }
        let id = FIRSTS
            .checked_add(actor)
            .filter(|&id| id < family::ACTORS)
            .ok_or_else(|| dashboard_core::invalid("Trade flows exhausted"))?;
        let aliases = [self.alias(seller.0)?, self.alias(buyer.0)?];
        let cell = family::cell(
            writer,
            &Scalar::List(aliases.iter().map(|&alias| Scalar::Unsigned(u64::from(alias))).collect()),
        )?;
        writer.catalogue(id, cell)?;
        let mut keys = [0; 2];
        for (handle, (measure, decimals)) in keys
            .iter_mut()
            .zip([(FACT, 0), (DELIVERED, RESOURCE_DECIMALS)])
        {
            *handle = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: None,
                },
                decimals,
            )?;
        }
        self.flows.push(Flow {
            key,
            keys,
            run: None,
        });
        self.by_key.insert(key, actor);
        Ok(actor)
    }

    /// The id of `group`, with its key, catalogued when it is new.
    fn group(&mut self, writer: &mut Writer, group: Group) -> io::Result<u64> {
        if let Some(&id) = self.group_ids.get(&group) {
            return Ok(id);
        }
        let id = self.groups.len() as u64;
        let scalar = GROUPS
            .checked_add(id)
            .filter(|&scalar| scalar < FIRSTS)
            .ok_or_else(|| dashboard_core::invalid("Trade refusal groups exhausted"))?;
        let pair = match group {
            None => Scalar::Null,
            Some(((seller_slot, seller_id), (buyer_slot, buyer_id))) => Scalar::List(
                [seller_slot, seller_id, buyer_slot, buyer_id]
                    .iter()
                    .map(|&value| Scalar::Unsigned(u64::from(value)))
                    .collect(),
            ),
        };
        let cell = family::cell(writer, &pair)?;
        writer.catalogue(scalar, cell)?;
        let key = self.key(
            writer,
            Place {
                actor: FAMILY,
                measure: REFUSALS,
                dimension: Some(id),
            },
            0,
        )?;
        self.groups.push((group, key));
        self.group_ids.insert(group, id);
        self.totals.push(0);
        Ok(id)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("trade.emit", None);
        let at = hour.hour;
        let adjacent = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A trade hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run.
            Some(last) => {
                self.close(writer, last)?;
                false
            }
            None => false,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        let known = self.known_key(writer)?;
        match hour.facts {
            None if !adjacent => {
                return Err(dashboard_core::invalid(
                    "A trade hour after one not walked carries no read",
                ));
            }
            None => {}
            Some(facts) if !facts.known => self.unknown(writer, at, known)?,
            Some(facts) => self.known_read(writer, at, adjacent, known, *facts)?,
        }
        for (&group, &deals) in &self.refusing {
            let total = &mut self.totals[group as usize];
            *total = total
                .checked_add(u64::from(deals))
                .ok_or_else(|| dashboard_core::invalid("Trade refusals exceed their count"))?;
            self.stale[2] = true;
        }
        Ok(())
    }

    /// An hour whose read cannot say who trades what: every run ends.
    fn unknown(&mut self, writer: &mut Writer, at: i32, known: KeyHandle) -> io::Result<()> {
        for actor in std::mem::take(&mut self.active) {
            self.end(writer, at, actor)?;
        }
        self.named.clear();
        self.carry.value(writer, known, at, None, false)?;
        for (group, _) in std::mem::take(&mut self.refusing) {
            let key = self.groups[group as usize].1;
            self.carry.value(writer, key, at, None, false)?;
        }
        self.known = false;
        Ok(())
    }

    /// Ends `actor`'s run before `at`; it no longer names its slots.
    fn end(&mut self, writer: &mut Writer, at: i32, actor: u64) -> io::Result<()> {
        let flow = &mut self.flows[actor as usize];
        if let Some(run) = &mut flow.run {
            run.last = Some(at - 1);
        }
        let keys = flow.keys;
        for key in keys {
            self.carry.value(writer, key, at, None, false)?;
        }
        Ok(())
    }

    fn name(&mut self, actor: u64) {
        let ((seller, _), (buyer, _), _) = self.flows[actor as usize].key;
        for slot in [seller, buyer] {
            *self.named.entry(slot).or_default() += 1;
        }
    }

    fn unname(&mut self, actor: u64) {
        let ((seller, _), (buyer, _), _) = self.flows[actor as usize].key;
        for slot in [seller, buyer] {
            if let Some(count) = self.named.get_mut(&slot) {
                *count -= 1;
                if *count == 0 {
                    self.named.remove(&slot);
                }
            }
        }
    }

    fn retire(&mut self, writer: &mut Writer, at: i32, actor: u64) -> io::Result<()> {
        if self.active.remove(&actor) {
            self.end(writer, at, actor)?;
            self.unname(actor);
        }
        Ok(())
    }

    /// `actor`'s terms from `at` on: they go on unchanged where the reads `continue` and it held
    /// the same terms the hour before; else a run starts.
    fn offer(
        &mut self,
        writer: &mut Writer,
        at: i32,
        actor: u64,
        terms: Terms,
        continues: bool,
    ) -> io::Result<()> {
        let was_active = self.active.contains(&actor);
        let flow = &self.flows[actor as usize];
        if continues && was_active && flow.run.is_some_and(|run| run.terms == terms) {
            return Ok(());
        }
        let ((seller, _), (buyer, _), _) = flow.key;
        let base = match flow.run {
            Some(run) => integrals(run.base, run.terms, run.first, run.last.unwrap_or(at - 1))
                .ok_or_else(exceeded)?,
            None => [0; 6],
        };
        let [fact, delivered] = flow.keys;
        let aliases = [self.alias(seller)?, self.alias(buyer)?];
        let starts = !was_active;
        self.carry
            .value(writer, fact, at, Some(i128::from(at)), starts)?;
        let mut cells = Vec::with_capacity(13);
        cells.extend(aliases.iter().map(|&alias| Cell::Unsigned(u64::from(alias))));
        cells.extend([
            Cell::Integer(i128::from(terms.delivered)),
            Cell::Integer(i128::from(terms.factories)),
            Cell::Integer(i128::from(terms.assigned)),
            Cell::Integer(i128::from(terms.required)),
            Cell::Unsigned(u64::from(terms.deals)),
        ]);
        cells.extend(base.iter().map(|&total| Cell::Integer(total)));
        let fields = writer.cells(cells)?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key: fact,
            native_hour: at,
            read_hour: None,
            sequence,
            kind: kind::RUN,
            fields,
        })?;
        self.carry.value(
            writer,
            delivered,
            at,
            Some(i128::from(terms.delivered)),
            starts,
        )?;
        self.flows[actor as usize].run = Some(Current {
            first: at,
            last: None,
            terms,
            base,
        });
        if !was_active {
            self.active.insert(actor);
            self.name(actor);
        }
        Ok(())
    }

    /// A known read.
    fn known_read(
        &mut self,
        writer: &mut Writer,
        at: i32,
        adjacent: bool,
        known: KeyHandle,
        facts: Facts,
    ) -> io::Result<()> {
        let Facts {
            whole,
            holders,
            flows,
            rejected,
            names,
            ..
        } = facts;
        // The slots whose holder changed: a slot an active flow names breaks every run.
        let mut changed = Vec::new();
        if whole {
            let held = std::mem::take(&mut self.holders);
            for (slot, holder) in holders {
                if let Some(holder) = holder {
                    if held.get(&slot) != Some(&holder) {
                        changed.push(slot);
                    }
                    self.holders.insert(slot, holder);
                }
            }
            changed.extend(held.keys().filter(|slot| !self.holders.contains_key(*slot)));
        } else {
            for (slot, holder) in holders {
                if self.holders.get(&slot) != holder.as_ref() {
                    changed.push(slot);
                }
                match holder {
                    Some(holder) => self.holders.insert(slot, holder),
                    None => self.holders.remove(&slot),
                };
            }
        }
        let continues = self.known
            && adjacent
            && changed.iter().all(|slot| !self.named.contains_key(slot));
        let mut offered = BTreeSet::new();
        for (key, terms) in flows {
            match terms {
                Some(terms) => {
                    let actor = self.flow(writer, key, at)?;
                    self.offer(writer, at, actor, terms, continues)?;
                    offered.insert(actor);
                }
                None => {
                    if let Some(&actor) = self.by_key.get(&key) {
                        self.retire(writer, at, actor)?;
                        offered.insert(actor);
                    }
                }
            }
        }
        // A whole read lists every flow; another read's flows not listed go on as they were.
        let rest: Vec<u64> = if whole || !continues {
            self.active.difference(&offered).copied().collect()
        } else {
            Vec::new()
        };
        for actor in rest {
            if whole {
                self.retire(writer, at, actor)?;
            } else if !continues {
                let terms = self.flows[actor as usize]
                    .run
                    .ok_or_else(|| dashboard_core::invalid("An active trade flow holds no run"))?
                    .terms;
                self.offer(writer, at, actor, terms, false)?;
            }
        }
        if let Some(rejected) = rejected {
            let mut now = BTreeMap::new();
            for (group, deals) in rejected {
                now.insert(self.group(writer, group)?, deals);
            }
            let touched: BTreeSet<u64> = self.refusing.keys().chain(now.keys()).copied().collect();
            for group in touched {
                let key = self.groups[group as usize].1;
                let value = now.get(&group).map(|&deals| i128::from(deals));
                let starts = self.carry.held(key).is_none();
                self.carry.value(writer, key, at, value, starts)?;
            }
            self.refusing = now;
        }
        let starts = self.carry.held(known).is_none();
        self.carry.value(writer, known, at, Some(1), starts)?;
        for (id, token) in names {
            if let std::collections::btree_map::Entry::Vacant(entry) = self.resources.entry(id) {
                entry.insert(token);
                self.stale[0] = true;
            }
        }
        self.known = true;
        if !self.read {
            self.read = true;
            self.stale[3] = true;
        }
        Ok(())
    }

    /// Ends every run after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        for actor in std::mem::take(&mut self.active) {
            if let Some(run) = &mut self.flows[actor as usize].run {
                run.last = Some(last);
            }
        }
        self.named.clear();
        self.refusing.clear();
        self.known = false;
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
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
        let pairs = |pairs: &mut dyn Iterator<Item = (u64, Scalar)>| {
            Scalar::List(
                pairs
                    .map(|(key, value)| Scalar::List(vec![Scalar::Unsigned(key), value]))
                    .collect(),
            )
        };
        let scalars = [
            (RESOURCES, 0),
            (ALIASES, 1),
            (TOTALS, 2),
            (READ, 3),
        ];
        for (id, at) in scalars {
            if !std::mem::take(&mut self.stale[at]) {
                continue;
            }
            let scalar = match id {
                RESOURCES => pairs(&mut self.resources.iter().map(|(&id, token)| {
                    (u64::from(id), Scalar::Text(token.as_bytes().into()))
                })),
                ALIASES => Scalar::List(
                    self.aliases
                        .iter()
                        .map(|alias| Scalar::Text(alias.as_bytes().into()))
                        .collect(),
                ),
                TOTALS => pairs(
                    &mut self
                        .totals
                        .iter()
                        .enumerate()
                        .map(|(group, &total)| (group as u64, Scalar::Unsigned(total))),
                ),
                _ => Scalar::Boolean(self.read),
            };
            let cell = family::cell(writer, &scalar)?;
            writer.catalogue(id, cell)?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection reads every flow and holder anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            known: self.known,
            read: self.read,
            holders: self
                .holders
                .iter()
                .map(|(&slot, (tag, id))| (slot, tag.to_string(), *id))
                .collect(),
            aliases: self.aliases.iter().map(|alias| alias.to_string()).collect(),
            groups: self.groups.iter().map(|&(group, _)| group).collect(),
            refusing: self.refusing.iter().map(|(&group, &deals)| (group, deals)).collect(),
            totals: self.totals.clone(),
            resources: self.resources.clone(),
            flows: self
                .flows
                .iter()
                .map(|flow| SavedFlow {
                    key: flow.key,
                    run: flow.run,
                })
                .collect(),
            active: self.active.iter().copied().collect(),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.stale = [false; 4];
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        if saved.flows.len() != directory.actors.len() {
            return Err(invalid());
        }
        for (actor, flow) in saved.flows.into_iter().enumerate() {
            let actor = actor as u64;
            let mut keys = [0; 2];
            for (key, measure) in keys.iter_mut().zip([FACT, DELIVERED]) {
                *key = directory.key(actor, measure, None).ok_or_else(invalid)?;
            }
            emit.by_key.insert(flow.key, actor);
            emit.flows.push(Flow {
                key: flow.key,
                keys,
                run: flow.run,
            });
        }
        for (id, group) in saved.groups.into_iter().enumerate() {
            let key = directory
                .key(FAMILY, REFUSALS, Some(id as u64))
                .ok_or_else(invalid)?;
            emit.groups.push((group, key));
            emit.group_ids.insert(group, id as u64);
        }
        if saved.totals.len() != emit.groups.len() {
            return Err(invalid());
        }
        emit.totals = saved.totals;
        for (group, deals) in saved.refusing {
            if group as usize >= emit.groups.len() {
                return Err(invalid());
            }
            emit.refusing.insert(group, deals);
        }
        for actor in saved.active {
            if emit
                .flows
                .get(actor as usize)
                .and_then(|flow| flow.run)
                .is_none_or(|run| run.last.is_some())
            {
                return Err(invalid());
            }
            emit.active.insert(actor);
            emit.name(actor);
        }
        for alias in saved.aliases {
            let alias: Arc<str> = Arc::from(alias);
            emit.alias_ids
                .insert(Arc::clone(&alias), emit.aliases.len() as u32);
            emit.aliases.push(alias);
        }
        emit.holders = saved
            .holders
            .into_iter()
            .map(|(slot, tag, id)| (slot, (Arc::from(tag), id)))
            .collect();
        emit.resources = saved.resources;
        emit.known_key = directory.key(FAMILY, KNOWN, None);
        emit.known = saved.known;
        emit.read = saved.read;
        emit.last = saved.last;
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedFlow {
    key: FlowKey,
    run: Option<Current>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    known: bool,
    read: bool,
    holders: Vec<(u32, String, u32)>,
    aliases: Vec<String>,
    groups: Vec<Group>,
    refusing: Vec<(u64, u32)>,
    totals: Vec<u64>,
    resources: BTreeMap<u32, String>,
    flows: Vec<SavedFlow>,
    active: Vec<u64>,
}
