//! Lend-lease balance's emit: on the family's series publication, whether the histories were
//! read complete, each holder's totals by measure and its record of each partner by partner and
//! measure, as runs; each holder's tags and the refusals, as scalars.
//!
//! A value holds from the read showing it until a read showing another or none; an hour between
//! reads carries it. A refused read and an hour not walked end every value. A record a read no
//! longer lists is zero while its owner's dropped records are, and ends otherwise; once ended it
//! stays so. The refusals count once a read, as the row preparation counted them.

use super::{
    MEASURES, ORPHAN, PARTNER, UNROSTERED,
    facts::{self, Counts, Facts, Read, Rosters, Sums, Values},
};
use dashboard_series::{
    Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
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
/// A holder's totals, by measure, and its record of each partner, by [`record`].
pub const TOTAL: u64 = 0;
pub const RECORD: u64 = 1;
/// The family's key: 1 while the histories were read complete.
pub const KNOWN: u64 = 0;
/// The actor of the family's own key.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: the refusals (`[[Text reason, Unsigned reads]…]`), and from [`TAGS`] each
/// holder's tags by actor (`[[Integer hour, Text tag]…]`).
pub const REFUSALS: u64 = 0;
pub const TAGS: u64 = 1 << 31;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// A record key's dimension: its partner's slot and its measure.
pub fn record(partner: u32, measure: usize) -> u64 {
    u64::from(partner) << 2 | measure as u64
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// What a holder slot holds now.
#[derive(Default)]
struct Holder {
    actor: u64,
    totals: Option<Sums>,
    /// Each partner's record, and whether a row lists it (else it is held at zero).
    records: HashMap<u32, (Values, bool)>,
    /// Its tag from each hour a read first tagged it so.
    tags: Vec<(i32, Arc<str>)>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    holders: HashMap<u32, Holder>,
    /// The owners whose dropped records are zero.
    zeroable: HashSet<u32>,
    /// Whether it holds a read: none before the first, after a refused read and past an hour not
    /// walked.
    holding: bool,
    counts: Option<Counts>,
    refusals: BTreeMap<String, u64>,
    /// Whether the refusals changed since the last cut, and the holders whose tags did.
    refused: bool,
    retagged: Vec<u32>,
    last: Option<i32>,
    /// The first hour since the last cut.
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
            holders: HashMap::new(),
            zeroable: HashSet::new(),
            holding: false,
            counts: None,
            refusals: BTreeMap::new(),
            refused: false,
            retagged: Vec::new(),
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    /// `slot`'s holder, catalogued at `hour` when it is new.
    fn holder(&mut self, writer: &mut Writer, slot: u32, hour: i32) -> io::Result<&mut Holder> {
        if !self.holders.contains_key(&slot) {
            let actor = self.actors.actor(writer, slot.to_string().as_bytes(), hour)?;
            self.holders.insert(
                slot,
                Holder {
                    actor,
                    ..Holder::default()
                },
            );
        }
        Ok(self.holders.get_mut(&slot).expect("holder just kept"))
    }

    /// `slot`'s values of `measure` from `at` on: `values` by measure, none to end them.
    fn values(
        &mut self,
        writer: &mut Writer,
        at: i32,
        (actor, measure, dimension): (u64, u64, impl Fn(usize) -> u64),
        values: Option<Sums>,
    ) -> io::Result<()> {
        for index in 0..MEASURES {
            let key = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: Some(dimension(index)),
                },
            )?;
            let value = values.map(|values| values[index]);
            self.carry.value(writer, key, at, value, false)?;
        }
        Ok(())
    }

    fn totals(&mut self, writer: &mut Writer, at: i32, owner: u32, totals: Option<Sums>) -> io::Result<()> {
        if totals.is_none() && !self.holders.contains_key(&owner) {
            return Ok(());
        }
        let holder = self.holder(writer, owner, at)?;
        if holder.totals == totals {
            return Ok(());
        }
        holder.totals = totals;
        let actor = holder.actor;
        self.values(writer, at, (actor, TOTAL, |index| index as u64), totals)
    }

    /// `owner`'s record of `partner` from `at` on: listed by a row (`listed`) or held at zero;
    /// none to end it.
    fn record(
        &mut self,
        writer: &mut Writer,
        at: i32,
        (owner, partner): (u32, u32),
        held: Option<(Values, bool)>,
    ) -> io::Result<()> {
        if held.is_none() && !self.holders.contains_key(&owner) {
            return Ok(());
        }
        let holder = self.holder(writer, owner, at)?;
        let was = match held {
            Some(held) => holder.records.insert(partner, held),
            None => holder.records.remove(&partner),
        };
        if was.map(|(values, _)| values) == held.map(|(values, _)| values) {
            return Ok(());
        }
        let actor = holder.actor;
        let sums = held.map(|(values, _)| values.map(i128::from));
        self.values(writer, at, (actor, RECORD, |index| record(partner, index)), sums)
    }

    /// A record no row lists any more: zero while its owner's dropped records are, else ended.
    fn dropped(&mut self, writer: &mut Writer, at: i32, pair: (u32, u32)) -> io::Result<()> {
        let held = self
            .zeroable
            .contains(&pair.0)
            .then_some(([0; MEASURES], false));
        self.record(writer, at, pair, held)
    }

    fn refuse(&mut self, reason: &str, times: u64) {
        if times == 0 {
            return;
        }
        match self.refusals.get_mut(reason) {
            Some(count) => *count += times,
            None => {
                self.refusals.insert(reason.to_owned(), times);
            }
        }
        self.refused = true;
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("lend_lease.emit", None);
        let at = hour.hour;
        match self.last {
            Some(last) if last.checked_add(1) == Some(at) => {}
            Some(last) if at <= last => {
                return Err(invalid("A lend-lease hour does not follow the hour before"));
            }
            // An hour not walked ends every value.
            Some(last) => self.close(writer, last)?,
            None => {}
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        match hour.facts {
            None => Ok(()),
            Some(Facts::Refused(reason)) => {
                self.refuse(&reason, 1);
                match at.checked_sub(1) {
                    Some(before) => self.close(writer, before),
                    None => Ok(()),
                }
            }
            Some(Facts::Same) => self.read(writer, at, None),
            Some(Facts::Read(read)) => self.read(writer, at, Some(*read)),
        }
    }

    /// A read at `at`: what changed since the read before, if anything did.
    fn read(&mut self, writer: &mut Writer, at: i32, read: Option<Read>) -> io::Result<()> {
        if !self.holding && read.as_ref().is_none_or(|read| !read.whole) {
            return Err(invalid("A lend-lease read goes on from none"));
        }
        let known = self.key(
            writer,
            Place {
                actor: FAMILY,
                measure: KNOWN,
                dimension: None,
            },
        )?;
        self.carry.value(writer, known, at, Some(1), false)?;
        if let Some(read) = read {
            self.apply(writer, at, read)?;
        }
        self.holding = true;
        let counts = self.counts.take().ok_or_else(|| invalid("A lend-lease read counts nothing"))?;
        self.refuse(ORPHAN, counts.orphans);
        self.refuse(PARTNER, counts.partners);
        match &counts.rosters {
            Rosters::Skipped => {}
            Rosters::Unavailable(reason) => self.refuse(reason, 1),
            Rosters::Read(unresolved) => self.refuse(UNROSTERED, *unresolved),
        }
        self.counts = Some(counts);
        Ok(())
    }

    fn apply(&mut self, writer: &mut Writer, at: i32, read: Read) -> io::Result<()> {
        let Read {
            whole,
            zeroable,
            records,
            totals,
            tags,
            counts,
        } = read;
        if let Some(counts) = counts {
            self.counts = Some(counts);
        }
        // Owners whose dropped records end now.
        let mut ended = Vec::new();
        if whole {
            let now: HashSet<u32> = zeroable
                .iter()
                .filter(|&&(_, zeroes)| zeroes)
                .map(|&(owner, _)| owner)
                .collect();
            ended.extend(self.zeroable.iter().copied().filter(|owner| !now.contains(owner)));
            self.zeroable = now;
        } else {
            for (owner, zeroes) in zeroable {
                if zeroes {
                    self.zeroable.insert(owner);
                } else if self.zeroable.remove(&owner) {
                    ended.push(owner);
                }
            }
        }
        ended.sort_unstable();
        for owner in ended {
            let Some(holder) = self.holders.get(&owner) else {
                continue;
            };
            let mut zeroed: Vec<u32> = holder
                .records
                .iter()
                .filter(|(_, (_, listed))| !listed)
                .map(|(&partner, _)| partner)
                .collect();
            zeroed.sort_unstable();
            for partner in zeroed {
                self.record(writer, at, (owner, partner), None)?;
            }
        }
        if whole {
            let listed: HashSet<(u32, u32)> = records.iter().map(|&(pair, _)| pair).collect();
            let mut unlisted: Vec<(u32, u32)> = self
                .holders
                .iter()
                .flat_map(|(&owner, holder)| {
                    holder
                        .records
                        .iter()
                        .filter(|(_, (_, listed))| *listed)
                        .map(move |(&partner, _)| (owner, partner))
                })
                .filter(|pair| !listed.contains(pair))
                .collect();
            unlisted.sort_unstable();
            for pair in unlisted {
                self.dropped(writer, at, pair)?;
            }
        }
        for (pair, values) in records {
            match values {
                Some(values) => self.record(writer, at, pair, Some((values, true)))?,
                None => self.dropped(writer, at, pair)?,
            }
        }
        if whole {
            let listed: HashSet<u32> = totals.iter().map(|&(owner, _)| owner).collect();
            let mut unlisted: Vec<u32> = self
                .holders
                .iter()
                .filter(|&(owner, holder)| holder.totals.is_some() && !listed.contains(owner))
                .map(|(&owner, _)| owner)
                .collect();
            unlisted.sort_unstable();
            for owner in unlisted {
                self.totals(writer, at, owner, None)?;
            }
        }
        for (owner, sums) in totals {
            self.totals(writer, at, owner, sums)?;
        }
        for (slot, tag) in tags {
            let holder = self.holder(writer, slot, at)?;
            if holder.tags.last().is_none_or(|(_, held)| *held != tag) {
                holder.tags.push((at, tag));
                self.retagged.push(slot);
            }
        }
        Ok(())
    }

    /// Ends every value after `last`: the hour after it holds no read.
    fn close(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        for holder in self.holders.values_mut() {
            holder.totals = None;
            holder.records.clear();
        }
        self.zeroable.clear();
        self.holding = false;
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
        if std::mem::take(&mut self.refused) {
            let refusals = Scalar::List(
                self.refusals
                    .iter()
                    .map(|(reason, &reads)| {
                        Scalar::List(vec![
                            Scalar::Text(reason.as_bytes().into()),
                            Scalar::Unsigned(reads),
                        ])
                    })
                    .collect(),
            );
            let cell = family::cell(writer, &refusals)?;
            writer.catalogue(REFUSALS, cell)?;
        }
        let mut retagged = std::mem::take(&mut self.retagged);
        retagged.sort_unstable();
        retagged.dedup();
        for slot in retagged {
            let holder = &self.holders[&slot];
            let tags = Scalar::List(
                holder
                    .tags
                    .iter()
                    .map(|(hour, tag)| {
                        Scalar::List(vec![
                            Scalar::Integer(i128::from(*hour)),
                            Scalar::Text(tag.as_bytes().into()),
                        ])
                    })
                    .collect(),
            );
            let id = TAGS
                .checked_add(holder.actor)
                .filter(|&id| id < family::ACTORS)
                .ok_or_else(|| invalid("Lend-lease holders exceed the tag scalars"))?;
            let cell = family::cell(writer, &tags)?;
            writer.catalogue(id, cell)?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let mut holders: Vec<(u32, &Holder)> = self.holders.iter().map(|(&slot, holder)| (slot, holder)).collect();
        holders.sort_unstable_by_key(|&(slot, _)| slot);
        let mut zeroable: Vec<u32> = self.zeroable.iter().copied().collect();
        zeroable.sort_unstable();
        let saved = Saved {
            last: self.last,
            holding: self.holding,
            counts: self.counts.clone(),
            zeroable,
            refusals: self.refusals.clone(),
            holders: holders
                .into_iter()
                .map(|(slot, holder)| {
                    let mut records: Vec<(u32, Values, bool)> = holder
                        .records
                        .iter()
                        .map(|(&partner, &(values, listed))| (partner, values, listed))
                        .collect();
                    records.sort_unstable_by_key(|&(partner, ..)| partner);
                    SavedHolder {
                        slot,
                        actor: holder.actor,
                        totals: holder.totals,
                        records,
                        tags: holder
                            .tags
                            .iter()
                            .map(|(hour, tag)| (*hour, tag.to_string()))
                            .collect(),
                    }
                })
                .collect(),
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
        let length = usize::try_from(d.u64()?).map_err(|_| invalid("Saved lend-lease emit is not valid"))?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.holding = saved.holding;
        emit.counts = saved.counts;
        emit.zeroable = saved.zeroable.into_iter().collect();
        emit.refusals = saved.refusals;
        for holder in saved.holders {
            if directory.actors.get(holder.actor as usize).map(|actor| &actor.name[..])
                != Some(holder.slot.to_string().as_bytes())
            {
                return Err(invalid("Saved lend-lease emit is not valid"));
            }
            emit.holders.insert(
                holder.slot,
                Holder {
                    actor: holder.actor,
                    totals: holder.totals,
                    records: holder
                        .records
                        .into_iter()
                        .map(|(partner, values, listed)| (partner, (values, listed)))
                        .collect(),
                    tags: holder
                        .tags
                        .into_iter()
                        .map(|(hour, tag)| (hour, Arc::from(tag)))
                        .collect(),
                },
            );
        }
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedHolder {
    slot: u32,
    actor: u64,
    totals: Option<Sums>,
    records: Vec<(u32, Values, bool)>,
    tags: Vec<(i32, String)>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    holding: bool,
    counts: Option<Counts>,
    zeroable: Vec<u32>,
    refusals: BTreeMap<String, u64>,
    holders: Vec<SavedHolder>,
}
