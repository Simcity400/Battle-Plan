//! Supply health's emit: each country's counts, and each count's share of its divisions, as
//! runs on the family's series publication. A run goes on while the country keeps its holder
//! and its counts are read every hour unchanged; a holder change or an unread hour starts a
//! segment.

use super::{Configured, facts};
use dashboard_series::{
    Evidence, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use std::io;

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 4;
/// A country's three counts, packed ([`pack`]).
pub const COUNTS: u64 = 0;
/// Each count's share of the country's divisions, at [`SHARE_DECIMALS`].
pub const SHARES: [u64; 3] = [1, 2, 3];
pub const SHARE_DECIMALS: u8 = 12;
const SHARE_SCALE: i128 = 100_000_000_000_000;
/// Family scalars: whether any hour held DivisionLandInputs, why content refused the
/// defines, and its standing warnings.
pub const AVAILABLE: u64 = 0;
pub const FAILURE: u64 = 1;
pub const WARNINGS: u64 = 2;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Three counts as one exact value.
pub fn pack(counts: [u32; 3]) -> i128 {
    i128::from(counts[0]) | i128::from(counts[1]) << 32 | i128::from(counts[2]) << 64
}

/// The counts [`pack`] made of `value`.
pub fn unpack(value: i128) -> io::Result<[u32; 3]> {
    if !(0..1 << 96).contains(&value) {
        return Err(dashboard_core::invalid(
            "Prepared supply counts are out of range",
        ));
    }
    Ok([0, 32, 64].map(|shift| (value >> shift) as u32))
}

/// Each count's share of `counts`; none of a country without divisions.
fn shares(counts: [u32; 3]) -> [Option<i128>; 3] {
    let total: i128 = counts.iter().map(|&count| i128::from(count)).sum();
    counts.map(|count| (total != 0).then(|| i128::from(count) * SHARE_SCALE / total))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Track {
    holder: Option<(u32, u32)>,
    counts: Option<[u32; 3]>,
}

/// The family's emit on one branch.
pub struct Emit {
    configured: Configured,
    actors: Actors,
    carry: Carry,
    /// Each projection index's actor, for the segment under way.
    indices: Vec<u64>,
    /// By actor.
    tracks: Vec<Track>,
    /// Actors whose track holds a holder or counts.
    held: Vec<u64>,
    last: Option<i32>,
    available: bool,
    /// The scalars are catalogued.
    begun: bool,
    /// The first hour since the last cut.
    first: Option<i32>,
}

impl Emit {
    pub(super) fn new(configured: &Configured) -> Self {
        Self {
            configured: configured.clone(),
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            indices: Vec::new(),
            tracks: Vec::new(),
            held: Vec::new(),
            last: None,
            available: false,
            begun: false,
            first: None,
        }
    }

    fn begin(&mut self, writer: &mut Writer<'_>) -> io::Result<()> {
        let failure = match &self.configured.defines {
            Ok(_) => Scalar::Null,
            Err(failure) => Scalar::Text(failure.as_bytes().into()),
        };
        let warnings = Scalar::List(
            self.configured
                .warnings
                .iter()
                .map(|warning| Scalar::Text(warning.as_bytes().into()))
                .collect(),
        );
        for (id, scalar) in [
            (AVAILABLE, Scalar::Boolean(false)),
            (FAILURE, failure),
            (WARNINGS, warnings),
        ] {
            let cell = family::cell(writer, &scalar)?;
            writer.catalogue(id, cell)?;
        }
        self.begun = true;
        Ok(())
    }

    fn key(&mut self, writer: &mut Writer<'_>, actor: u64, measure: u64) -> io::Result<u64> {
        let decimals = if measure == COUNTS { 0 } else { SHARE_DECIMALS };
        let key = self.actors.key(
            writer,
            Place {
                actor,
                measure,
                dimension: None,
            },
            (RunMode::Exact, decimals),
            Some(DOMAIN),
        )?;
        Ok(key)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            self.carry.close(writer, last)?;
            for actor in std::mem::take(&mut self.held) {
                self.tracks[actor as usize] = Track::default();
            }
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        if !self.begun {
            self.begin(writer)?;
        }
        if hour.land_present && !self.available {
            self.available = true;
            writer.catalogue(AVAILABLE, dashboard_series::Cell::Boolean(true))?;
        }
        if self.configured.defines.is_err() {
            return Ok(());
        }
        for (index, tag) in hour.named {
            let actor = self.actors.actor(writer, tag.as_bytes(), at)?;
            let index = index as usize;
            if index >= self.indices.len() {
                self.indices.resize(index + 1, u64::MAX);
            }
            self.indices[index] = actor;
            if actor as usize >= self.tracks.len() {
                self.tracks.resize(actor as usize + 1, Track::default());
            }
        }
        let mut handed = Vec::with_capacity(hour.countries.len());
        for country in &hour.countries {
            let actor = *self
                .indices
                .get(country.index as usize)
                .filter(|&&actor| actor != u64::MAX)
                .ok_or_else(|| dashboard_core::invalid("A supply country was never named"))?;
            handed.push(actor);
            self.apply(writer, at, adjacent, actor, country.holder, country.counts)?;
        }
        if hour.whole {
            handed.sort_unstable();
            let silent: Vec<u64> = self
                .held
                .iter()
                .copied()
                .filter(|actor| handed.binary_search(actor).is_err())
                .collect();
            for actor in silent {
                self.apply(writer, at, adjacent, actor, None, None)?;
            }
        }
        Ok(())
    }

    fn apply(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        adjacent: bool,
        actor: u64,
        holder: Option<(u32, u32)>,
        counts: Option<[u32; 3]>,
    ) -> io::Result<()> {
        let track = self.tracks[actor as usize];
        let continues =
            adjacent && holder.is_some() && track.holder == holder && track.counts.is_some();
        match counts {
            Some(counts) if !(continues && track.counts == Some(counts)) => {
                let starts = !continues;
                let key = self.key(writer, actor, COUNTS)?;
                self.carry
                    .value(writer, key, hour, Some(pack(counts)), starts)?;
                for (measure, share) in SHARES.into_iter().zip(shares(counts)) {
                    let key = self.key(writer, actor, measure)?;
                    let starts = starts || self.carry.held(key).is_none();
                    self.carry.value(writer, key, hour, share, starts)?;
                }
            }
            Some(_) => {}
            None if track.counts.is_some() => {
                for measure in std::iter::once(COUNTS).chain(SHARES) {
                    let key = self.key(writer, actor, measure)?;
                    self.carry.value(writer, key, hour, None, false)?;
                }
            }
            None => {}
        }
        let next = Track { holder, counts };
        let was = track != Track::default();
        let is = next != Track::default();
        self.tracks[actor as usize] = next;
        if is && !was {
            self.held.push(actor);
        } else if was && !is {
            self.held.retain(|&held| held != actor);
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, and the hours since the last cut as known.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
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
        Ok(())
    }

    /// What the emit goes on with after a cut: its open edges and each country's track. A
    /// restart goes on at a segment's first hour, whose projection names its countries anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        out.push(u8::from(self.available) | u8::from(self.begun) << 1);
        match self.last {
            Some(last) => {
                out.push(1);
                out.extend_from_slice(&last.to_le_bytes());
            }
            None => out.push(0),
        }
        let mut held = self.held.clone();
        held.sort_unstable();
        out.extend_from_slice(&(held.len() as u64).to_le_bytes());
        for actor in held {
            let track = self.tracks[actor as usize];
            out.extend_from_slice(&actor.to_le_bytes());
            match track.holder {
                Some((slot, id)) => {
                    out.push(1);
                    out.extend_from_slice(&slot.to_le_bytes());
                    out.extend_from_slice(&id.to_le_bytes());
                }
                None => out.push(0),
            }
            match track.counts {
                Some(counts) => {
                    out.push(1);
                    for count in counts {
                        out.extend_from_slice(&count.to_le_bytes());
                    }
                }
                None => out.push(0),
            }
        }
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(configured: &Configured, directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let invalid = || dashboard_core::invalid("Saved supply emit is not valid");
        let mut emit = Self::new(configured);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.tracks = vec![Track::default(); directory.actors.len()];
        let mut d = Decode::new(bytes);
        let flags = d.u8()?;
        if flags > 3 {
            return Err(invalid());
        }
        emit.available = flags & 1 != 0;
        emit.begun = flags & 2 != 0;
        emit.last = match d.u8()? {
            0 => None,
            1 => Some(d.i32()?),
            _ => return Err(invalid()),
        };
        for _ in 0..d.u64()? {
            let actor = d.u64()?;
            let holder = match d.u8()? {
                0 => None,
                1 => Some((d.u32()?, d.u32()?)),
                _ => return Err(invalid()),
            };
            let counts = match d.u8()? {
                0 => None,
                1 => Some([d.u32()?, d.u32()?, d.u32()?]),
                _ => return Err(invalid()),
            };
            let track = emit
                .tracks
                .get_mut(usize::try_from(actor).map_err(|_| invalid())?)
                .ok_or_else(invalid)?;
            *track = Track { holder, counts };
            emit.held.push(actor);
        }
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        Ok(emit)
    }
}
