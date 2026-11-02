//! The military air ledger's emit: for each family (air losses, bombing), what the row ledger
//! reads of its hours over the branch, on the family's series publication. A read is checked
//! as the row ledger checks it: its event ordinals go forward, and the first read with events
//! after an unread interval may start a new identity epoch. Each read's sums are events of the
//! family's [`SUMS`] key, dated at their native hour and read at the hour that supplied them,
//! naming their leaf from the catalogue; the hours the family read, its unknown callers and
//! each country's presence are runs.

use super::{
    LeafKey, UNNAMED_TAG,
    facts::{self, Events, Identities},
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// Of [`FAMILY`], by family: whether each hour was read ([`OBSERVED`]), whether its read held a
/// caller of unknown membership ([`INCOMPLETE`]), and its sums ([`SUMS`]), each an event of
/// fields (leaf id, sum).
pub const OBSERVED: u64 = 0;
pub const INCOMPLETE: u64 = 2;
pub const SUMS: u64 = 4;
/// Of each country actor, by family: whether the read held the country.
pub const PRESENT: u64 = 6;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
pub const SUM_KIND: u64 = 1;
/// Family scalars, by family: the counts by reason; and each leaf's key as JSON, by id, below
/// the catalogue's actors.
pub const WARNINGS: u64 = 0;
pub fn leaves(family: usize) -> u64 {
    (family as u64 + 1) << 28
}
/// Every ledger key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// A country actor's name: its tag, a byte no tag holds, and its identity.
pub fn country_name(tag: &str, identity: &str) -> Vec<u8> {
    let mut name = Vec::with_capacity(tag.len() + 1 + identity.len());
    name.extend_from_slice(tag.as_bytes());
    name.push(0);
    name.extend_from_slice(identity.as_bytes());
    name
}

/// The tag and identity of a country actor's name.
pub fn country_of(name: &[u8]) -> Option<(String, String)> {
    let at = name.iter().position(|&byte| byte == 0)?;
    Some((
        String::from_utf8(name[..at].to_vec()).ok()?,
        String::from_utf8(name[at + 1..].to_vec()).ok()?,
    ))
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved military air emit is not valid")
}

/// What one family carries over the branch.
#[derive(Default)]
struct Family {
    last_event: Option<u64>,
    /// The first hour of an unread event interval since the last read with events.
    unread_since: Option<i32>,
    warnings: BTreeMap<String, usize>,
    leaves: HashMap<LeafKey, u64>,
    countries: HashMap<(String, String), KeyHandle>,
    /// The presence keys open now, and the join they were opened from.
    present: HashSet<KeyHandle>,
    joined: Option<Arc<Identities>>,
}

impl Family {
    /// The ordinal a read with events leaves its family at, or why it is refused: its ordinals
    /// go forward, and the first read after an unread interval starts a new identity epoch
    /// when its ordinals restart and every event is dated within or after the interval; a
    /// restart dated before it is earlier events recorded again.
    fn checked(&self, events: &str, read: &facts::Read) -> io::Result<u64> {
        let mut last = self.last_event;
        if let Some(since) = self.unread_since
            && last.is_some_and(|last| read.first <= last)
        {
            if read.earliest < since {
                return Err(dashboard_core::invalid(format!(
                    "{events} events restart their ordinals after the unread interval from \
                     hour {since} but are dated from hour {}, before it: earlier \
                     events recorded again, not new ones",
                    read.earliest
                )));
            }
            last = None;
        }
        // Ordinals ascend within a read, so only its first can move backwards.
        if last.is_some_and(|last| read.first <= last) {
            return Err(dashboard_core::invalid(
                "military air event ordinal repeated or moved backwards",
            ));
        }
        if let Some(reason) = &read.refused {
            return Err(dashboard_core::invalid(reason.clone()));
        }
        Ok(read.last)
    }
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    families: [Family; 2],
    /// Each family's observed, incomplete and sums keys.
    keys: Option<[[KeyHandle; 3]; 2]>,
    identities: Option<Arc<Identities>>,
    stale: bool,
    last: Option<i32>,
    first: Option<i32>,
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
            families: Default::default(),
            keys: None,
            identities: None,
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, actor: u64, measure: u64) -> io::Result<KeyHandle> {
        self.actors.key(
            writer,
            Place {
                actor,
                measure,
                dimension: None,
            },
            (RunMode::Exact, 0),
            Some(DOMAIN),
        )
    }

    fn keys(&mut self, writer: &mut Writer<'_>) -> io::Result<[[KeyHandle; 3]; 2]> {
        if let Some(keys) = self.keys {
            return Ok(keys);
        }
        let mut keys = [[0; 3]; 2];
        for (family, held) in keys.iter_mut().enumerate() {
            for (at, measure) in [OBSERVED, INCOMPLETE, SUMS].into_iter().enumerate() {
                held[at] = self.key(writer, FAMILY, measure + family as u64)?;
            }
        }
        self.keys = Some(keys);
        Ok(keys)
    }

    /// The presence key of `country` in `family`.
    fn country(
        &mut self,
        writer: &mut Writer<'_>,
        family: usize,
        country: &(String, String),
        hour: i32,
    ) -> io::Result<KeyHandle> {
        if let Some(&key) = self.families[family].countries.get(country) {
            return Ok(key);
        }
        let name = country_name(&country.0, &country.1);
        let actor = match self.actors.find(&name) {
            Some(actor) => actor,
            None => self.actors.actor(writer, &name, hour)?,
        };
        let key = self.key(writer, actor, PRESENT + family as u64)?;
        self.families[family].countries.insert(country.clone(), key);
        Ok(key)
    }

    /// The presence of `family` from `hour`: the countries `joined` holds, or none.
    fn presence(
        &mut self,
        writer: &mut Writer<'_>,
        family: usize,
        hour: i32,
        joined: Option<&Arc<Identities>>,
    ) -> io::Result<()> {
        let held = &self.families[family];
        let same = match (joined, &held.joined) {
            (Some(new), Some(old)) => Arc::ptr_eq(new, old),
            (None, None) => held.present.is_empty(),
            _ => false,
        };
        if same {
            return Ok(());
        }
        let mut now = HashSet::new();
        if let Some(joined) = joined {
            now.reserve(joined.countries.len());
            for country in &joined.countries {
                now.insert(self.country(writer, family, country, hour)?);
            }
        }
        let before = std::mem::take(&mut self.families[family].present);
        for &key in before.difference(&now) {
            self.carry.value(writer, key, hour, None, false)?;
        }
        for &key in now.difference(&before) {
            self.carry.value(writer, key, hour, Some(1), false)?;
        }
        let held = &mut self.families[family];
        held.present = now;
        held.joined = joined.cloned();
        Ok(())
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let keys = self.keys(writer)?;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An unrecorded hour ends every run and the known hours.
            self.carry.close(writer, last)?;
            self.known(writer, last)?;
            for family in &mut self.families {
                family.present.clear();
                family.joined = None;
            }
        }
        if let Some(identities) = hour.identities {
            self.identities = Some(identities);
        }
        // Either family's read refusing writes nothing of the hour.
        let mut ordinals = [None; 2];
        for (family, events) in hour.families.iter().enumerate() {
            if let Events::Read(read) = events {
                let identities = self
                    .identities
                    .as_ref()
                    .ok_or_else(|| dashboard_core::invalid("A military air read has no join"))?;
                if let Some(read) = read.as_ref().filter(|_| !identities.refused) {
                    let name = if family == 0 { "air loss" } else { "bombing" };
                    ordinals[family] = Some(self.families[family].checked(name, read)?);
                }
            }
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        for (family, events) in hour.families.into_iter().enumerate() {
            let mut observed = false;
            let mut incomplete = false;
            let mut joined = None;
            match events {
                Events::Unread => {
                    self.families[family].unread_since.get_or_insert(at);
                }
                Events::Unjoined => {}
                Events::Read(read) => {
                    let identities = self.identities.clone().expect("a checked join");
                    if identities.invalid_tags != 0 {
                        *self.families[family]
                            .warnings
                            .entry(UNNAMED_TAG.into())
                            .or_default() += identities.invalid_tags;
                        self.stale = true;
                    }
                    observed = !identities.refused;
                    if let Some(read) = read.filter(|_| observed) {
                        incomplete = read.incomplete;
                        let held = &mut self.families[family];
                        held.unread_since = None;
                        held.last_event = ordinals[family];
                        self.read(writer, family, at, keys[family][2], *read)?;
                    }
                    joined = Some(identities);
                }
            }
            self.presence(writer, family, at, joined.as_ref())?;
            self.carry
                .value(writer, keys[family][0], at, observed.then_some(1), false)?;
            self.carry
                .value(writer, keys[family][1], at, incomplete.then_some(1), false)?;
        }
        Ok(())
    }

    /// A checked read with events: its counts and its sums.
    fn read(
        &mut self,
        writer: &mut Writer<'_>,
        family: usize,
        at: i32,
        key: KeyHandle,
        read: facts::Read,
    ) -> io::Result<()> {
        let held = &mut self.families[family];
        for (reason, count) in read.notes {
            *held.warnings.entry(reason).or_default() += count;
            self.stale = true;
        }
        for (leaf, hour, sum) in read.sums {
            let next = self.families[family].leaves.len() as u64;
            let id = match self.families[family].leaves.get(&leaf) {
                Some(&id) => id,
                None => {
                    let text =
                        family::cell(writer, &Scalar::Text(serde_json::to_vec(&leaf)?.into()))?;
                    writer.catalogue(leaves(family) + next, text)?;
                    self.families[family].leaves.insert(leaf, next);
                    next
                }
            };
            let fields = writer.cells([Cell::Unsigned(id), Cell::Integer(sum)])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key,
                native_hour: hour,
                read_hour: Some(at),
                sequence,
                kind: SUM_KIND,
                fields,
            })?;
        }
        Ok(())
    }

    /// The hours walked since the domain's last row, through `last`, as known.
    fn known(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        // An hour that failed began none: the cut goes through the last that succeeded.
        if let Some(first) = self.first.take().filter(|&first| first <= last) {
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
        Ok(())
    }

    /// Appends what is open through `last`, the hours since the last cut as known, and the
    /// counts that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.known(writer, last)?;
        if self.stale {
            for (family, held) in self.families.iter().enumerate() {
                let reasons = Scalar::List(
                    held.warnings
                        .iter()
                        .map(|(reason, &count)| {
                            Scalar::List(vec![
                                Scalar::Text(reason.as_bytes().into()),
                                Scalar::Unsigned(count as u64),
                            ])
                        })
                        .collect(),
                );
                let cell = family::cell(writer, &reasons)?;
                writer.catalogue(WARNINGS + family as u64, cell)?;
            }
            self.stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection sends its join anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            families: self.families.each_ref().map(|family| SavedFamily {
                last_event: family.last_event,
                unread_since: family.unread_since,
                warnings: family
                    .warnings
                    .iter()
                    .map(|(reason, &count)| (reason.clone(), count))
                    .collect(),
                present: family
                    .countries
                    .iter()
                    .filter(|(_, key)| family.present.contains(key))
                    .map(|(country, _)| country.clone())
                    .collect(),
            }),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.stale = false;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut keys = [[0; 3]; 2];
        for (family, held) in keys.iter_mut().enumerate() {
            for (at, measure) in [OBSERVED, INCOMPLETE, SUMS].into_iter().enumerate() {
                held[at] = directory
                    .key(FAMILY, measure + family as u64, None)
                    .ok_or_else(invalid)?;
            }
        }
        emit.keys = Some(keys);
        for (actor, described) in directory.actors.iter().enumerate() {
            let Some(country) = country_of(&described.name) else {
                continue;
            };
            for (family, held) in emit.families.iter_mut().enumerate() {
                if let Some(key) = directory.key(actor as u64, PRESENT + family as u64, None) {
                    held.countries.insert(country.clone(), key);
                }
            }
        }
        for (family, held) in emit.families.iter_mut().enumerate() {
            let mut id = 0;
            while let Some(scalar) = directory.scalar(leaves(family) + id) {
                let leaf: LeafKey = serde_json::from_slice(scalar.text().ok_or_else(invalid)?)?;
                held.leaves.insert(leaf, id);
                id += 1;
            }
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        for (held, saved) in emit.families.iter_mut().zip(saved.families) {
            held.last_event = saved.last_event;
            held.unread_since = saved.unread_since;
            held.warnings = saved.warnings.into_iter().collect();
            for country in saved.present {
                held.present
                    .insert(*held.countries.get(&country).ok_or_else(invalid)?);
            }
        }
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedFamily {
    last_event: Option<u64>,
    unread_since: Option<i32>,
    warnings: Vec<(String, usize)>,
    present: Vec<(String, String)>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    families: [SavedFamily; 2],
}
