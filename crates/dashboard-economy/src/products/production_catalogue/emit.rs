//! The catalogue's emit: each domain's values as runs on the family's series publication, a
//! value holding from the first read showing it to the first read that does not, a gap ending
//! it at its last read.
//!
//! Every key is the family's own ([`ACTOR`]): a licence's fields by its licensee holder and
//! position (`slot << 32 | position`), each licensed variant's under its own measure
//! ([`LICENCE_VARIANTS`] + its index); an organisation's fields by its id; a design's by its
//! handle; each domain's known hours by domain; each refusal's count per hour it held, by
//! reason. Events: each holder's tag from the hour a row it is party to was read with it
//! ([`TAGS`]), each organisation's naming operands where they changed ([`ORGANISATIONS`]), and
//! each variant's naming read where it changed ([`VARIANTS`]). Texts and reasons are family
//! scalars, numbered in the order first seen.

use super::{
    Field,
    facts::{self, Sent, Val},
};
use dashboard_base::products::{
    equipment::{RawVariant, VariantHour, VariantVersion},
    production_lines::RawOrganisation,
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
/// The actor of every key.
pub const ACTOR: u64 = u64::MAX;

/// By domain: 1 while its roster was read complete.
pub const KNOWN: u64 = 0;
/// By reason: its count at each hour it held.
pub const REFUSALS: u64 = 1;
/// [`kind::TAG`] events.
pub const TAGS: u64 = 2;
/// [`kind::ORGANISATION`] events.
pub const ORGANISATIONS: u64 = 3;
/// [`kind::VARIANT`] events.
pub const VARIANTS: u64 = 4;
/// By licensee holder and position.
pub const LICENCE_GIVER: u64 = 10;
pub const LICENCE_UNIDENTIFIED: u64 = 11;
pub const LICENCE_LENT: u64 = 12;
pub const LICENCE_REQUIRED: u64 = 13;
pub const LICENCE_START: u64 = 14;
/// By organisation id.
pub const MIO_HOLDER: u64 = 20;
pub const MIO_SIZE: u64 = 21;
pub const MIO_FUNDS: u64 = 22;
pub const MIO_POINTS: u64 = 23;
/// A text's number.
pub const MIO_POLICY: u64 = 24;
pub const MIO_POLICY_NAME: u64 = 25;
pub const MIO_COOLDOWN: u64 = 26;
pub const MIO_UNLOCKED: u64 = 27;
pub const MIO_POLICY_UNRESOLVED: u64 = 28;
/// By design handle.
pub const DESIGN_CREATOR: u64 = 30;
pub const DESIGN_OBSOLETE: u64 = 31;
/// Plus the variant's index, by licensee holder and position.
pub const LICENCE_VARIANTS: u64 = 1 << 32;

/// Family scalars: the first hour the designs were read complete, each text and each reason,
/// by number.
pub const DESIGNS_FROM: u64 = 0;
pub const TEXTS: u64 = 1 << 20;
pub const REASONS: u64 = 1 << 30;

pub mod kind {
    /// `[Unsigned slot, Text tag]`: the holder's tag from the event's hour.
    pub const TAG: u64 = 0;
    /// `[Unsigned id, Text operands]`: the organisation's naming operands, as JSON.
    pub const ORGANISATION: u64 = 1;
    /// `[Unsigned id, Text version]`: the variant's naming read, as JSON.
    pub const VARIANT: u64 = 2;
}

/// Every key's evidence: each hour the emit was handed is known.
const DOMAIN: u64 = 0;

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Saved production catalogue emit is not valid",
    )
}

/// A licence's dimension: its licensee holder and position.
pub fn licence(owner: u32, at: u32) -> u64 {
    u64::from(owner) << 32 | u64::from(at)
}

/// The key `field` is held in.
pub fn place(field: Field) -> Place {
    let (measure, dimension) = match field {
        Field::LicenceGiver(owner, at) => (LICENCE_GIVER, licence(owner, at)),
        Field::LicenceUnidentified(owner, at) => (LICENCE_UNIDENTIFIED, licence(owner, at)),
        Field::LicenceLent(owner, at) => (LICENCE_LENT, licence(owner, at)),
        Field::LicenceRequired(owner, at) => (LICENCE_REQUIRED, licence(owner, at)),
        Field::LicenceStart(owner, at) => (LICENCE_START, licence(owner, at)),
        Field::LicenceVariant(owner, at, index) => {
            (LICENCE_VARIANTS + u64::from(index), licence(owner, at))
        }
        Field::MioHolder(id) => (MIO_HOLDER, id),
        Field::MioSize(id) => (MIO_SIZE, id),
        Field::MioFunds(id) => (MIO_FUNDS, id),
        Field::MioPoints(id) => (MIO_POINTS, id),
        Field::MioPolicy(id) => (MIO_POLICY, id),
        Field::MioPolicyName(id) => (MIO_POLICY_NAME, id),
        Field::MioCooldown(id) => (MIO_COOLDOWN, id),
        Field::MioUnlocked(id) => (MIO_UNLOCKED, id),
        Field::MioPolicyUnresolved(id) => (MIO_POLICY_UNRESOLVED, id),
        Field::Known(domain) => (KNOWN, u64::from(domain)),
        Field::DesignCreator(id) => (DESIGN_CREATOR, id),
        Field::DesignObsolete(id) => (DESIGN_OBSOLETE, id),
    };
    Place {
        actor: ACTOR,
        measure,
        dimension: Some(dimension),
    }
}

/// The variants' naming reads as the row census's variant catalogue keeps them: a read ending
/// at a gap or a due read that did not land, a variant's version only where its operands
/// changed.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Variants {
    last: Option<i32>,
    /// Each variant's latest version.
    latest: HashMap<u64, VariantVersion>,
    /// The variants whose latest version names them.
    known: BTreeSet<u64>,
}

impl Variants {
    fn forget(&mut self, hour: i32, changed: &mut Vec<(u64, VariantVersion)>) {
        for id in std::mem::take(&mut self.known) {
            let version = VariantVersion {
                first_hour: hour,
                raw: None,
            };
            self.latest.insert(id, version.clone());
            changed.push((id, version));
        }
    }

    /// The versions `hour` records.
    fn observe(
        &mut self,
        hour: i32,
        read: VariantHour,
        built: Vec<(u64, RawVariant)>,
    ) -> Vec<(u64, VariantVersion)> {
        let mut changed = Vec::new();
        if let Some(last) = self.last
            && last.checked_add(1) != Some(hour)
        {
            self.forget(last.saturating_add(1), &mut changed);
        }
        self.last = Some(hour);
        match read {
            VariantHour::Carried { complete: true } => {}
            VariantHour::Carried { complete: false } | VariantHour::Incomplete => {
                self.forget(hour, &mut changed);
            }
            VariantHour::Read => {
                for (id, raw) in built {
                    let same = self.latest.get(&id).is_some_and(|last| {
                        last.raw.as_ref().is_some_and(|last| {
                            RawVariant {
                                first_hour: hour,
                                ..last.clone()
                            } == raw
                        })
                    });
                    if same {
                        continue;
                    }
                    let version = VariantVersion {
                        first_hour: hour,
                        raw: Some(raw),
                    };
                    self.latest.insert(id, version.clone());
                    self.known.insert(id);
                    changed.push((id, version));
                }
            }
        }
        changed
    }
}

/// What the emit goes on with after a cut, beside its open edges.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    texts: Vec<String>,
    reasons: Vec<String>,
    held: [Vec<u64>; 3],
    tags: Vec<(u32, String)>,
    organisations: Vec<(u64, RawOrganisation)>,
    variants: Variants,
    refusals: Vec<(String, u64)>,
    refused: Vec<u64>,
    designs_from: Option<i32>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    last: Option<i32>,
    /// The first hour handed since the domain's evidence was last written.
    first: Option<i32>,
    texts: HashMap<Box<str>, u64>,
    spelled: Vec<String>,
    reasons: HashMap<String, u64>,
    stated: Vec<String>,
    /// Each domain's keys holding a value.
    held: [HashSet<KeyHandle>; 3],
    /// Each holder's tag as last written.
    tags: HashMap<u32, Box<str>>,
    /// Each organisation's naming operands as last written.
    organisations: HashMap<u64, RawOrganisation>,
    variants: Variants,
    /// The projection's refusals holding, and the refusal keys open.
    refusals: BTreeMap<String, u64>,
    refused: HashSet<KeyHandle>,
    designs_from: Option<i32>,
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

/// An organisation's naming operands as the row census compares them: its token, name, icon and
/// owner.
fn naming_operands(raw: &RawOrganisation) -> (&str, &str, &str, Option<&str>, bool) {
    (
        &raw.token,
        &raw.name_key,
        &raw.icon,
        raw.owner_tag.as_deref(),
        raw.owner_unknown,
    )
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            last: None,
            first: None,
            texts: HashMap::new(),
            spelled: Vec::new(),
            reasons: HashMap::new(),
            stated: Vec::new(),
            held: Default::default(),
            tags: HashMap::new(),
            organisations: HashMap::new(),
            variants: Variants::default(),
            refusals: BTreeMap::new(),
            refused: HashSet::new(),
            designs_from: None,
        }
    }

    fn key(&mut self, writer: &mut Writer, place: Place) -> io::Result<KeyHandle> {
        match self.actors.held(place) {
            Some(key) => Ok(key),
            None => self
                .actors
                .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN)),
        }
    }

    fn family_key(&mut self, writer: &mut Writer, measure: u64) -> io::Result<KeyHandle> {
        self.key(
            writer,
            Place {
                actor: ACTOR,
                measure,
                dimension: None,
            },
        )
    }

    /// The number of `text`, catalogued when new.
    fn text(&mut self, writer: &mut Writer, text: &str) -> io::Result<u64> {
        if let Some(&id) = self.texts.get(text) {
            return Ok(id);
        }
        let id = self.spelled.len() as u64;
        let cell = family::cell(writer, &Scalar::Text(text.as_bytes().into()))?;
        writer.catalogue(TEXTS + id, cell)?;
        self.texts.insert(text.into(), id);
        self.spelled.push(text.to_owned());
        Ok(id)
    }

    /// The number of `reason`, catalogued when new.
    fn reason(&mut self, writer: &mut Writer, reason: &str) -> io::Result<u64> {
        if let Some(&id) = self.reasons.get(reason) {
            return Ok(id);
        }
        let id = self.stated.len() as u64;
        let cell = family::cell(writer, &Scalar::Text(reason.as_bytes().into()))?;
        writer.catalogue(REASONS + id, cell)?;
        self.reasons.insert(reason.to_owned(), id);
        self.stated.push(reason.to_owned());
        Ok(id)
    }

    /// An event of the family key `measure` at `at`: `id`, then `text`.
    fn event(
        &mut self,
        writer: &mut Writer,
        measure: u64,
        kind: u64,
        at: i32,
        id: u64,
        text: &str,
    ) -> io::Result<()> {
        let key = self.family_key(writer, measure)?;
        let text = writer.text(text)?;
        let fields = writer.cells([Cell::Unsigned(id), Cell::Text(text)])?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: at,
            read_hour: Some(at),
            sequence,
            kind,
            fields,
        })
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("production_catalogue.emit", None);
        let at = hour.hour;
        if let Some(last) = self.last {
            if at <= last {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "A production catalogue hour does not follow the hour before",
                ));
            }
            if last.checked_add(1) != Some(at) {
                // An unread hour ends every run; the next hour's reads start them anew.
                self.carry.close(writer, last)?;
                self.known(writer, last)?;
                self.held = Default::default();
                self.refused.clear();
                self.refusals.clear();
            }
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        for (id, version) in self.variants.observe(at, hour.variants, hour.built) {
            let text = serde_json::to_string(&version)?;
            self.event(writer, VARIANTS, kind::VARIANT, at, id, &text)?;
        }
        let Some(facts) = hour.facts else {
            return Ok(());
        };
        let facts = *facts;
        for (slot, tag) in facts.tags {
            if self.tags.get(&slot).is_some_and(|held| **held == *tag) {
                continue;
            }
            self.event(writer, TAGS, kind::TAG, at, u64::from(slot), &tag)?;
            self.tags.insert(slot, Box::from(&*tag));
        }
        for sent in facts.sent {
            self.domain(writer, at, sent)?;
        }
        for (id, raw) in facts.names {
            let same = self
                .organisations
                .get(&id)
                .is_some_and(|held| naming_operands(held) == naming_operands(&raw));
            if same {
                continue;
            }
            let text = serde_json::to_string(&raw)?;
            self.event(writer, ORGANISATIONS, kind::ORGANISATION, at, id, &text)?;
            self.organisations.insert(id, raw);
        }
        if facts.designs && self.designs_from.is_none() {
            self.designs_from = Some(at);
            writer.catalogue(DESIGNS_FROM, Cell::Integer(i128::from(at)))?;
        }
        if let Some(refusals) = facts.refusals {
            self.refusals = refusals.into_iter().collect();
            self.refuse(writer, at)?;
        }
        Ok(())
    }

    /// A domain's values from `at`.
    fn domain(&mut self, writer: &mut Writer, at: i32, sent: Sent) -> io::Result<()> {
        let index = usize::from(sent.domain);
        let mut now = HashSet::new();
        for (field, value) in sent.values {
            let held = place(field);
            match value {
                Some(value) => {
                    let key = self.key(writer, held)?;
                    let value = match value {
                        Val::Int(value) => value,
                        Val::Text(text) => i128::from(self.text(writer, &text)?),
                    };
                    self.carry.value(writer, key, at, Some(value), false)?;
                    if sent.whole {
                        now.insert(key);
                    } else {
                        self.held[index].insert(key);
                    }
                }
                // A field never holding a value has no key.
                None => {
                    if let Some(key) = self.actors.held(held) {
                        self.carry.value(writer, key, at, None, false)?;
                        self.held[index].remove(&key);
                    }
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
        Ok(())
    }

    /// Each refusal holding from `at`, with its count.
    fn refuse(&mut self, writer: &mut Writer, at: i32) -> io::Result<()> {
        let mut open = HashSet::with_capacity(self.refusals.len());
        for (reason, count) in self.refusals.clone() {
            let id = self.reason(writer, &reason)?;
            let key = self.key(
                writer,
                Place {
                    actor: ACTOR,
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
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.known(writer, last)
    }

    /// Writes the hours handed since the domain's evidence was last written, through `last`, as
    /// known: a run must lie within its key's evidence.
    fn known(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
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
    /// whose projection hands every domain on whole.
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
        let mut organisations: Vec<(u64, RawOrganisation)> = self
            .organisations
            .iter()
            .map(|(&id, raw)| (id, raw.clone()))
            .collect();
        organisations.sort_unstable_by_key(|&(id, _)| id);
        let saved = Saved {
            last: self.last,
            texts: self.spelled.clone(),
            reasons: self.stated.clone(),
            held: std::array::from_fn(|domain| sorted(&self.held[domain])),
            tags,
            organisations,
            variants: self.variants.clone(),
            refusals: self
                .refusals
                .iter()
                .map(|(reason, &count)| (reason.clone(), count))
                .collect(),
            refused: sorted(&self.refused),
            designs_from: self.designs_from,
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
        for (id, text) in saved.texts.iter().enumerate() {
            emit.texts.insert(text.as_str().into(), id as u64);
        }
        emit.spelled = saved.texts;
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
        emit.organisations = saved.organisations.into_iter().collect();
        emit.variants = saved.variants;
        emit.refusals = saved.refusals.into_iter().collect();
        emit.refused = saved.refused.into_iter().collect();
        emit.designs_from = saved.designs_from;
        Ok(emit)
    }
}
