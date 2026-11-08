//! Readers' view of production's facts: one index per publication, decoded
//! once behind an `Arc`, the endpoint states it decodes on demand, and the
//! classes content resolves for it.

use super::facts::{self, Endpoint, Holder, In, NameCode, Out, Preamble};
use super::{FACTS, PageCounts, check_cancel, refusal_lines, section};
use crate::Source;
use crate::products::equipment::production::{
    Named, ProductionSpendingUnknown, Purpose, RocketProduced, ScannedProduced, TagHistory,
};
use crate::products::equipment::{self, Domain, RawVariant, StockVariantName};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Read, Seek};
use std::sync::{Arc, Mutex, Weak, atomic::AtomicBool};

type CoverageRuns = Vec<(u64, Option<Option<ProductionSpendingUnknown>>)>;

/// What spending readers count of the branch's reads, holders by id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SpendingFacts {
    pub reads: Vec<i32>,
    pub observations: Vec<i32>,
    pub unlisted: usize,
    pub unread_reason: Option<String>,
    /// Each observation's first spending taint: its hour and reason.
    pub tainted: Vec<(u32, i32, ProductionSpendingUnknown)>,
    pub rocket_reads: Vec<i32>,
    pub rocket_observations: Vec<i32>,
    pub rocket_unread: usize,
    pub rocket_unattributed: usize,
    pub rocket_inconsistent: usize,
    pub equipment_holders: Vec<u32>,
    pub rocket_holders: Vec<u32>,
}

impl SpendingFacts {
    /// What spending's reply reads of the branch, for `holder` (tag `tag`).
    pub(crate) fn scanned(&self, holder: Option<u32>, tag: &str) -> ScannedProduced {
        let mut rocket = RocketProduced::default();
        rocket.reads = self.rocket_reads.clone();
        rocket.observations = self.rocket_observations.clone();
        rocket.unread = self.rocket_unread;
        rocket.unattributed = self.rocket_unattributed;
        rocket.inconsistent = self.rocket_inconsistent;
        let held = |holders: &[u32]| holder.is_some_and(|holder| holders.contains(&holder));
        if held(&self.rocket_holders) {
            rocket.tags.insert(tag.to_owned(), TagHistory::default());
        }
        ScannedProduced {
            tags: if held(&self.equipment_holders) {
                BTreeMap::from([(tag.to_owned(), TagHistory::default())])
            } else {
                BTreeMap::new()
            },
            reads: self.reads.clone(),
            observations: self.observations.clone(),
            unlisted: self.unlisted,
            unread_reason: self.unread_reason.clone(),
            spending_tainted: self
                .tainted
                .iter()
                .map(|&(observation, hour, reason)| (observation, (hour, reason)))
                .collect(),
            rocket,
            ..Default::default()
        }
    }
}

/// The diagnostics as of a flush (P7).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Diagnostics {
    /// What the latest endpoint's warnings were rendered from.
    pub counts: PageCounts,
    /// Its unnamed country-hours when last sealed.
    pub unnamed: usize,
    pub spending: SpendingFacts,
    pub variant_refusals: BTreeMap<String, BTreeSet<u64>>,
    /// Every equipment observation's counting start, ascending.
    pub observation_starts: Vec<i32>,
}

fn write_hours(out: &mut Out, hours: &[i32]) -> io::Result<()> {
    out.count(hours.len())?;
    for hour in hours {
        out.i32(*hour);
    }
    Ok(())
}

fn read_hours(input: &mut In<'_>) -> io::Result<Vec<i32>> {
    (0..input.count()?).map(|_| input.i32()).collect()
}

fn write_holders(out: &mut Out, holders: &[u32]) -> io::Result<()> {
    out.count(holders.len())?;
    for holder in holders {
        out.u32(*holder);
    }
    Ok(())
}

fn read_holders(input: &mut In<'_>) -> io::Result<Vec<u32>> {
    (0..input.count()?).map(|_| input.u32()).collect()
}

fn write_refusals(out: &mut Out, refusals: &BTreeMap<String, BTreeSet<u64>>) -> io::Result<()> {
    out.count(refusals.len())?;
    for (reason, variants) in refusals {
        out.str(reason)?;
        out.count(variants.len())?;
        for variant in variants {
            out.u64(*variant);
        }
    }
    Ok(())
}

fn read_refusals(input: &mut In<'_>) -> io::Result<BTreeMap<String, BTreeSet<u64>>> {
    let mut refusals = BTreeMap::new();
    for _ in 0..input.count()? {
        let reason = input.str()?;
        let variants = (0..input.count()?)
            .map(|_| input.u64())
            .collect::<io::Result<_>>()?;
        refusals.insert(reason, variants);
    }
    Ok(refusals)
}

fn write_size(out: &mut Out, value: usize) {
    out.u64(value as u64);
}

fn read_size(input: &mut In<'_>) -> io::Result<usize> {
    usize::try_from(input.u64()?).map_err(|_| facts::invalid("a count exceeds this machine"))
}

pub(crate) fn write_diagnostics(out: &mut Out, diagnostics: &Diagnostics) -> io::Result<()> {
    let counts = &diagnostics.counts.recording;
    out.bool(counts.reads);
    write_size(out, counts.unread);
    out.opt_str(counts.unread_reason.as_deref())?;
    out.opt_i32(counts.tainted_from);
    for count in [
        counts.missed,
        counts.inexact,
        counts.lost,
        counts.unattributed,
        counts.unlisted,
        counts.observations,
    ] {
        write_size(out, count);
    }
    out.bool(counts.rocket_reads);
    for count in [
        counts.rocket_unread,
        counts.rocket_unattributed,
        counts.rocket_inconsistent,
    ] {
        write_size(out, count);
    }
    write_refusals(out, &diagnostics.counts.refusals)?;
    out.u32(diagnostics.counts.restarts);
    write_size(out, diagnostics.unnamed);
    let spending = &diagnostics.spending;
    write_hours(out, &spending.reads)?;
    write_hours(out, &spending.observations)?;
    write_size(out, spending.unlisted);
    out.opt_str(spending.unread_reason.as_deref())?;
    out.count(spending.tainted.len())?;
    for &(observation, hour, reason) in &spending.tainted {
        out.u32(observation);
        out.i32(hour);
        out.u8(facts::reason_code(Some(reason)));
    }
    write_hours(out, &spending.rocket_reads)?;
    write_hours(out, &spending.rocket_observations)?;
    for count in [
        spending.rocket_unread,
        spending.rocket_unattributed,
        spending.rocket_inconsistent,
    ] {
        write_size(out, count);
    }
    write_holders(out, &spending.equipment_holders)?;
    write_holders(out, &spending.rocket_holders)?;
    write_refusals(out, &diagnostics.variant_refusals)?;
    write_hours(out, &diagnostics.observation_starts)
}

fn read_diagnostics(input: &mut In<'_>) -> io::Result<Diagnostics> {
    let mut counts = super::SavedCounts {
        reads: input.bool()?,
        unread: read_size(input)?,
        unread_reason: input.opt_str()?,
        tainted_from: input.opt_i32()?,
        ..Default::default()
    };
    counts.missed = read_size(input)?;
    counts.inexact = read_size(input)?;
    counts.lost = read_size(input)?;
    counts.unattributed = read_size(input)?;
    counts.unlisted = read_size(input)?;
    counts.observations = read_size(input)?;
    counts.rocket_reads = input.bool()?;
    counts.rocket_unread = read_size(input)?;
    counts.rocket_unattributed = read_size(input)?;
    counts.rocket_inconsistent = read_size(input)?;
    let counts = PageCounts {
        recording: counts,
        refusals: read_refusals(input)?,
        restarts: input.u32()?,
    };
    let unnamed = read_size(input)?;
    let mut spending = SpendingFacts {
        reads: read_hours(input)?,
        observations: read_hours(input)?,
        unlisted: read_size(input)?,
        unread_reason: input.opt_str()?,
        ..Default::default()
    };
    for _ in 0..input.count()? {
        let observation = input.u32()?;
        let hour = input.i32()?;
        let reason = facts::reason_of(input.u8()?)?
            .ok_or_else(|| facts::invalid("a spending taint has no reason"))?;
        spending.tainted.push((observation, hour, reason));
    }
    spending.rocket_reads = read_hours(input)?;
    spending.rocket_observations = read_hours(input)?;
    spending.rocket_unread = read_size(input)?;
    spending.rocket_unattributed = read_size(input)?;
    spending.rocket_inconsistent = read_size(input)?;
    spending.equipment_holders = read_holders(input)?;
    spending.rocket_holders = read_holders(input)?;
    Ok(Diagnostics {
        counts,
        unnamed,
        spending,
        variant_refusals: read_refusals(input)?,
        observation_starts: read_hours(input)?,
    })
}

pub(crate) fn write_raw(raw: &RawVariant, out: &mut Out) -> io::Result<()> {
    out.u64(raw.id);
    out.str(&raw.token)?;
    out.str(&raw.display_name)?;
    out.str(&raw.suffix)?;
    out.bool(raw.append);
    out.opt_str(raw.name_tag.as_deref())?;
    out.opt_str(raw.type_tag.as_deref())?;
    out.opt_str(raw.design.as_deref())?;
    out.count(raw.refusals.len())?;
    for (field, why) in &raw.refusals {
        out.str(field)?;
        out.str(why)?;
    }
    Ok(())
}

pub(crate) fn read_raw(input: &mut In<'_>) -> io::Result<RawVariant> {
    let id = input.u64()?;
    let token = input.str()?;
    let display_name = input.str()?;
    let suffix = input.str()?;
    let append = input.bool()?;
    let name_tag = input.opt_str()?;
    let type_tag = input.opt_str()?;
    let design = input.opt_str()?;
    let refusals = (0..input.count()?)
        .map(|_| Ok((input.str()?, input.str()?)))
        .collect::<io::Result<Vec<_>>>()?;
    Ok(RawVariant {
        id,
        first_hour: 0,
        token,
        display_name,
        suffix,
        append,
        name_tag,
        type_tag,
        design,
        refusals,
    })
}

/// One spent series key of a holder (P4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeriesKey {
    /// A design's input of every purpose (`refits`) or of all but refits.
    Spent {
        rocket: bool,
        refits: bool,
        variant: u64,
        key: u64,
    },
    /// A design's input of one of [`super::SERIES_PURPOSES`].
    Purpose {
        variant: u64,
        purpose: Purpose,
        key: u64,
    },
    /// The magnitudes of a design's input of one purpose whose ticks'
    /// purpose was not recorded.
    Unproven {
        variant: u64,
        purpose: Purpose,
        key: u64,
    },
}

/// A variant's classification as of the publication (P5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Final {
    pub token: u32,
    pub seen: u8,
    pub names: Vec<NameCode>,
}

struct RangeAt {
    ordinal: u64,
    id: Option<[u64; 3]>,
    ranking_base: u64,
    holders_base: u64,
}

#[derive(Clone, Copy)]
struct GroupAt {
    range: u32,
    first: u64,
    offset: u64,
    len: u32,
    events: u32,
}

/// Every holder at one endpoint, by holder id.
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    pub holders: Vec<Option<Holder>>,
}

impl State {
    pub(crate) fn holder(&self, holder: u32) -> Option<&Holder> {
        self.holders.get(holder as usize)?.as_ref()
    }
}

/// A branch's published facts, decoded once per publication.
pub(crate) struct Index {
    pub branch: u64,
    ranges: Vec<RangeAt>,
    pub holders: Vec<String>,
    /// Holder ids in tag order.
    pub tags: BTreeMap<String, u32>,
    /// Variant handles by id.
    pub variants: Vec<u64>,
    variant_ids: HashMap<u64, u32>,
    pub tokens: Vec<String>,
    pub raws: Vec<RawVariant>,
    pub endpoints: Vec<Endpoint>,
    endpoint_ranges: Vec<u32>,
    groups: Vec<Vec<GroupAt>>,
    naming: Vec<Vec<(u64, Vec<NameCode>)>>,
    pub finals: Vec<Option<Final>>,
    /// The latest withdrawal hour of each Rocket join, by (holder, slot,
    /// country, observation, supplying read).
    revocations: HashMap<(u32, u32, u32, Option<i32>, i32), i32>,
    pub keys: Vec<Vec<SeriesKey>>,
    pub diagnostics: Diagnostics,
    /// Endpoint states decoded, each kept while a reader holds it.
    states: Mutex<BTreeMap<u64, Weak<State>>>,
    /// Each holder's coverage runs, decoded once per publication.
    coverage: Mutex<HashMap<u32, Arc<CoverageRuns>>>,
    #[cfg(any(test, feature = "test-support"))]
    pub work: Mutex<Work>,
}

/// What an index decoded, for tests that bound a request's work.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Ranking blocks and holder groups decoded.
    pub blocks: usize,
    /// Their decoded bytes.
    pub bytes: usize,
}

fn read_bytes(
    store: &store::Store,
    key: &store::SourceKey,
    section: &str,
    ordinal: u64,
    at: u64,
    len: usize,
) -> io::Result<Vec<u8>> {
    let mut file = store
        .read(key, section, ordinal)?
        .ok_or_else(|| facts::invalid("a published range is missing"))?;
    file.seek(io::SeekFrom::Start(at))?;
    let mut bytes = vec![0; len];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

impl Index {
    fn empty(branch: u64) -> Self {
        Self {
            branch,
            ranges: Vec::new(),
            holders: Vec::new(),
            tags: BTreeMap::new(),
            variants: Vec::new(),
            variant_ids: HashMap::new(),
            tokens: Vec::new(),
            raws: Vec::new(),
            endpoints: Vec::new(),
            endpoint_ranges: Vec::new(),
            groups: Vec::new(),
            naming: Vec::new(),
            finals: Vec::new(),
            revocations: HashMap::new(),
            keys: Vec::new(),
            diagnostics: Diagnostics::default(),
            states: Mutex::new(BTreeMap::new()),
            coverage: Mutex::new(HashMap::new()),
            #[cfg(any(test, feature = "test-support"))]
            work: Mutex::new(Work::default()),
        }
    }

    /// The branch's first `count` ranges, reusing what `previous` decoded of
    /// them when its ranges are still published.
    pub(crate) fn read(
        store: &store::Store,
        key: &store::SourceKey,
        branch: u64,
        count: u64,
        previous: Option<&Index>,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let name = section(FACTS, branch);
        let reuse = previous.filter(|previous| {
            previous.branch == branch
                && previous.ranges.len() as u64 <= count
                && previous.ranges.last().is_none_or(|last| {
                    store
                        .range_id(key, &name, last.ordinal)
                        .ok()
                        .flatten()
                        .map(|id| id.cache_key())
                        == last.id
                })
        });
        let mut index = match reuse {
            Some(previous) => previous.copy(),
            None => Self::empty(branch),
        };
        for ordinal in index.ranges.len() as u64..count {
            check_cancel(cancelled)?;
            index.add_range(store, key, &name, ordinal)?;
        }
        Ok(index)
    }

    fn copy(&self) -> Self {
        Self {
            branch: self.branch,
            ranges: self
                .ranges
                .iter()
                .map(|range| RangeAt {
                    ordinal: range.ordinal,
                    id: range.id,
                    ranking_base: range.ranking_base,
                    holders_base: range.holders_base,
                })
                .collect(),
            holders: self.holders.clone(),
            tags: self.tags.clone(),
            variants: self.variants.clone(),
            variant_ids: self.variant_ids.clone(),
            tokens: self.tokens.clone(),
            raws: self.raws.clone(),
            endpoints: self.endpoints.clone(),
            endpoint_ranges: self.endpoint_ranges.clone(),
            groups: self.groups.clone(),
            naming: self.naming.clone(),
            finals: self.finals.clone(),
            revocations: self.revocations.clone(),
            keys: self.keys.clone(),
            diagnostics: self.diagnostics.clone(),
            states: Mutex::new(BTreeMap::new()),
            coverage: Mutex::new(HashMap::new()),
            #[cfg(any(test, feature = "test-support"))]
            work: Mutex::new(Work::default()),
        }
    }

    fn add_range(
        &mut self,
        store: &store::Store,
        key: &store::SourceKey,
        name: &str,
        ordinal: u64,
    ) -> io::Result<()> {
        let mut file = store
            .read(key, name, ordinal)?
            .ok_or_else(|| facts::invalid("a published range is missing"))?;
        let mut preamble = [0; facts::PREAMBLE];
        file.read_exact(&mut preamble)?;
        let preamble = Preamble::read(&preamble)?;
        let mut head = vec![
            0;
            usize::try_from(preamble.head_len)
                .map_err(|_| facts::invalid("a head is too long"))?
        ];
        file.read_exact(&mut head)?;
        let id = store.range_id(key, name, ordinal)?.map(|id| id.cache_key());
        let range = self.ranges.len() as u32;
        self.ranges.push(RangeAt {
            ordinal,
            id,
            ranking_base: preamble.ranking_base(),
            holders_base: preamble.holders_base(),
        });
        if preamble.first_endpoint != self.endpoints.len() as u64 {
            return Err(facts::invalid("a range does not follow the endpoints before it"));
        }
        let head = crate::prepared::series::decompress(&head)?;
        let mut input = In::new(&head);
        self.read_tables(&mut input.region()?)?;
        let endpoints = input.u32()?;
        let mut rows = input.region()?;
        for _ in 0..endpoints {
            self.endpoints.push(Endpoint::read(&mut rows)?);
            self.endpoint_ranges.push(range);
        }
        let names = input.u32()?;
        let mut naming = input.region()?;
        for _ in 0..names {
            let endpoint = naming.u64()?;
            let variant = naming.u32()? as usize;
            let codes = (0..naming.count()?)
                .map(|_| NameCode::read(&mut naming))
                .collect::<io::Result<Vec<_>>>()?;
            self.naming
                .get_mut(variant)
                .ok_or_else(|| facts::invalid("a name of no variant"))?
                .push((endpoint, codes));
        }
        let mut keys = input.region()?;
        for _ in 0..keys.count()? {
            let kind = keys.u8()?;
            let holder = keys.u32()? as usize;
            let variant = keys.u64()?;
            let rocket = keys.bool()?;
            let refits = keys.bool()?;
            let purpose = facts::purpose_of(keys.u8()?)?;
            let key = keys.u64()?;
            let row = match kind {
                0 => SeriesKey::Spent {
                    rocket,
                    refits,
                    variant,
                    key,
                },
                1 => SeriesKey::Purpose {
                    variant,
                    purpose,
                    key,
                },
                2 => SeriesKey::Unproven {
                    variant,
                    purpose,
                    key,
                },
                _ => return Err(facts::invalid("unknown series key")),
            };
            self.keys
                .get_mut(holder)
                .ok_or_else(|| facts::invalid("a series key of no holder"))?
                .push(row);
        }
        let mut finals = input.region()?;
        let mut latest = vec![None; self.variants.len()];
        for _ in 0..finals.count()? {
            let variant = finals.u32()? as usize;
            let token = finals.u32()?;
            let seen = finals.u8()?;
            let names = (0..finals.count()?)
                .map(|_| NameCode::read(&mut finals))
                .collect::<io::Result<Vec<_>>>()?;
            if token as usize >= self.tokens.len() {
                return Err(facts::invalid("a classification of no token"));
            }
            *latest
                .get_mut(variant)
                .ok_or_else(|| facts::invalid("a classification of no variant"))? =
                Some(Final { token, seen, names });
        }
        self.finals = latest;
        let mut revocations = input.region()?;
        for _ in 0..revocations.count()? {
            let holder = revocations.u32()?;
            let slot = revocations.u32()?;
            let country = revocations.u32()?;
            let observation = revocations.opt_i32()?;
            let read = revocations.i32()?;
            let last = revocations.i32()?;
            let held = self
                .revocations
                .entry((holder, slot, country, observation, read))
                .or_insert(last);
            *held = (*held).max(last);
        }
        self.diagnostics = read_diagnostics(&mut input.region()?)?;
        let mut directory = input.region()?;
        for _ in 0..directory.count()? {
            let holder = directory.u32()? as usize;
            let groups = self
                .groups
                .get_mut(holder)
                .ok_or_else(|| facts::invalid("leaves of no holder"))?;
            for _ in 0..directory.count()? {
                let first = directory.u64()?;
                let _last = directory.u64()?;
                let offset = directory.u64()?;
                let len = directory.u32()?;
                let events = directory.u32()?;
                if groups.last().is_some_and(|group| group.first >= first) {
                    return Err(facts::invalid("a holder's groups are out of order"));
                }
                groups.push(GroupAt {
                    range,
                    first,
                    offset,
                    len,
                    events,
                });
            }
        }
        if !input.done() {
            return Err(facts::invalid("a head has trailing bytes"));
        }
        Ok(())
    }

    fn read_tables(&mut self, input: &mut In<'_>) -> io::Result<()> {
        /// A table's new entries: where they start, which must follow those
        /// read, and how many there are.
        fn follows(input: &mut In<'_>, len: usize, what: &str) -> io::Result<usize> {
            if input.count()? != len {
                return Err(facts::invalid(&format!("{what} do not follow the table")));
            }
            input.count()
        }
        let count = follows(input, self.holders.len(), "holders")?;
        for _ in 0..count {
            let tag = input.str()?;
            self.tags.insert(tag.clone(), self.holders.len() as u32);
            self.holders.push(tag);
            self.groups.push(Vec::new());
            self.keys.push(Vec::new());
        }
        let count = follows(input, self.variants.len(), "variants")?;
        for _ in 0..count {
            let handle = input.u64()?;
            self.variant_ids.insert(handle, self.variants.len() as u32);
            self.variants.push(handle);
            self.naming.push(Vec::new());
        }
        let count = follows(input, self.tokens.len(), "tokens")?;
        for _ in 0..count {
            self.tokens.push(input.str()?);
        }
        let count = follows(input, self.raws.len(), "raw operands")?;
        for _ in 0..count {
            self.raws.push(read_raw(input)?);
        }
        Ok(())
    }

    /// The publication it was read from: its range count and last range.
    pub(crate) fn publication(&self) -> (usize, Option<[u64; 3]>) {
        (self.ranges.len(), self.ranges.last().and_then(|range| range.id))
    }

    /// The endpoint holding `hour`.
    pub(crate) fn endpoint_at(&self, hour: i32) -> Option<u64> {
        let at = self.endpoints.partition_point(|row| row.last_hour < hour);
        self.endpoints
            .get(at)
            .filter(|row| row.first_hour <= hour)
            .map(|_| at as u64)
    }

    /// The hours of endpoint `ordinal` and those it continues or that continue it: the span
    /// over which it holds, wherever flushes fell.
    pub(crate) fn span(&self, ordinal: u64) -> (i32, i32) {
        let mut first = ordinal as usize;
        while first > 0 && self.endpoints[first].continues {
            first -= 1;
        }
        let mut last = ordinal as usize;
        while self.endpoints.get(last + 1).is_some_and(|next| next.continues) {
            last += 1;
        }
        (self.endpoints[first].first_hour, self.endpoints[last].last_hour)
    }

    pub(crate) fn endpoint(&self, ordinal: u64) -> &Endpoint {
        &self.endpoints[ordinal as usize]
    }

    /// Where running totals at `hour` began counting.
    pub(crate) fn observation_at(&self, hour: i32) -> Option<i32> {
        let starts = &self.diagnostics.observation_starts;
        let reached = starts.partition_point(|since| *since <= hour);
        reached.checked_sub(1).map(|index| starts[index])
    }

    /// The warnings as of the publication.
    pub(crate) fn warnings(&self) -> Vec<String> {
        self.diagnostics
            .counts
            .warnings_with(self.diagnostics.unnamed)
    }

    /// Lines naming refused variant operands.
    pub(crate) fn variant_warnings(&self) -> Vec<String> {
        refusal_lines(&self.diagnostics.variant_refusals)
    }

    /// Whether the Rocket join of a holder's equipment source at an endpoint
    /// beginning at `first_hour` was withdrawn since it was published.
    pub(crate) fn revoked(
        &self,
        holder: u32,
        head: &facts::Head,
        read_hour: i32,
        first_hour: i32,
    ) -> bool {
        self.revocations
            .get(&(
                holder,
                head.slot,
                head.country_id,
                head.observation,
                read_hour,
            ))
            .is_some_and(|last| first_hour <= *last)
    }

    /// Every holder at endpoint `ordinal`.
    pub(crate) fn state(
        &self,
        store: &store::Store,
        key: &store::SourceKey,
        ordinal: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<Arc<State>> {
        check_cancel(cancelled)?;
        let mut keyframe = ordinal as usize;
        while !self.endpoints[keyframe].keyframe {
            keyframe = keyframe
                .checked_sub(1)
                .ok_or_else(|| facts::invalid("no ranking keyframe"))?;
        }
        let start = {
            let held = self.states.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(state) = held.get(&ordinal).and_then(Weak::upgrade) {
                return Ok(state);
            }
            held.range(keyframe as u64..ordinal)
                .rev()
                .find_map(|(at, state)| Some((*at, state.upgrade()?)))
        };
        let (mut state, from) = match start {
            Some((held, state)) => ((*state).clone(), held as usize + 1),
            None => (State::default(), keyframe),
        };
        let range = &self.ranges[self.endpoint_ranges[ordinal as usize] as usize];
        let first = &self.endpoints[from];
        let last = &self.endpoints[ordinal as usize];
        let begin = first.ranking_offset;
        let end = last.ranking_offset + u64::from(last.ranking_len);
        let bytes = read_bytes(
            store,
            key,
            &section(FACTS, self.branch),
            range.ordinal,
            range.ranking_base + begin,
            usize::try_from(end - begin).map_err(|_| facts::invalid("a block is too long"))?,
        )?;
        for row in &self.endpoints[from..=ordinal as usize] {
            let at = (row.ranking_offset - begin) as usize;
            let block = bytes
                .get(at..at + row.ranking_len as usize)
                .ok_or_else(|| facts::invalid("a ranking block runs past its range"))?;
            let block = crate::prepared::series::decompress(block)?;
            let mut input = In::new(&block);
            while !input.done() {
                facts::apply_patch(&mut state.holders, &mut input)?;
            }
            #[cfg(any(test, feature = "test-support"))]
            {
                let mut work = self.work.lock().unwrap_or_else(|error| error.into_inner());
                work.blocks += 1;
                work.bytes += block.len();
            }
        }
        let state = Arc::new(state);
        let mut held = self.states.lock().unwrap_or_else(|error| error.into_inner());
        held.retain(|_, state| state.strong_count() > 0);
        held.insert(ordinal, Arc::downgrade(&state));
        Ok(state)
    }

    /// One holder at endpoint `ordinal`, from its own leaf groups.
    pub(crate) fn holder_at(
        &self,
        store: &store::Store,
        key: &store::SourceKey,
        holder: u32,
        ordinal: u64,
    ) -> io::Result<Option<Holder>> {
        let Some(groups) = self.groups.get(holder as usize) else {
            return Ok(None);
        };
        let at = groups.partition_point(|group| group.first <= ordinal);
        let Some(group) = at.checked_sub(1).map(|at| groups[at]) else {
            return Ok(None);
        };
        let mut state = None;
        self.walk_group(store, key, holder, group, |endpoint, holder| {
            if endpoint > ordinal {
                return false;
            }
            state = holder.clone();
            true
        })?;
        Ok(state)
    }

    /// Each event of a holder group in order, until `visit` declines one.
    fn walk_group(
        &self,
        store: &store::Store,
        key: &store::SourceKey,
        holder: u32,
        group: GroupAt,
        mut visit: impl FnMut(u64, &Option<Holder>) -> bool,
    ) -> io::Result<()> {
        let range = &self.ranges[group.range as usize];
        let bytes = read_bytes(
            store,
            key,
            &section(FACTS, self.branch),
            range.ordinal,
            range.holders_base + group.offset,
            group.len as usize,
        )?;
        let bytes = crate::prepared::series::decompress(&bytes)?;
        #[cfg(any(test, feature = "test-support"))]
        {
            let mut work = self.work.lock().unwrap_or_else(|error| error.into_inner());
            work.blocks += 1;
            work.bytes += bytes.len();
        }
        let mut input = In::new(&bytes);
        let mut state = None;
        for _ in 0..group.events {
            let endpoint = input.u64()?;
            if endpoint >= self.endpoints.len() as u64 {
                return Err(facts::invalid("an event of no endpoint"));
            }
            if input.u32()? != holder {
                return Err(facts::invalid("another holder's event"));
            }
            facts::apply_holder(&mut state, &mut input)?;
            if !visit(endpoint, &state) {
                break;
            }
        }
        Ok(())
    }

    /// A holder's equipment campaign spending coverage at each endpoint where
    /// it changed: None without an equipment source. Decoded once per index.
    pub(crate) fn coverage(
        &self,
        store: &store::Store,
        key: &store::SourceKey,
        holder: u32,
    ) -> io::Result<Arc<CoverageRuns>> {
        if let Some(runs) = self
            .coverage
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&holder)
        {
            return Ok(Arc::clone(runs));
        }
        let mut runs: CoverageRuns = Vec::new();
        for group in self.groups.get(holder as usize).into_iter().flatten() {
            self.walk_group(store, key, holder, *group, |endpoint, state| {
                let reason = state
                    .as_ref()
                    .and_then(|state| state.equipment.as_ref())
                    .map(|source| source.head.campaign_reason);
                if runs.last().is_none_or(|(_, last)| *last != reason) {
                    runs.push((endpoint, reason));
                }
                true
            })?;
        }
        let runs = Arc::new(runs);
        self.coverage
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(holder, Arc::clone(&runs));
        Ok(runs)
    }

    /// A variant's names at endpoint `ordinal`; None before it was classified.
    pub(crate) fn names_at(
        &self,
        classes: &Classes,
        variant: u32,
        ordinal: u64,
    ) -> io::Result<Option<Vec<StockVariantName>>> {
        let events = &self.naming[variant as usize];
        let at = events.partition_point(|(endpoint, _)| *endpoint <= ordinal);
        let Some((_, codes)) = at.checked_sub(1).map(|at| &events[at]) else {
            return Ok(None);
        };
        let hour = self.endpoint(ordinal).names_hour;
        codes
            .iter()
            .map(|code| self.name(classes, code, hour))
            .collect::<io::Result<_>>()
            .map(Some)
    }

    /// A variant's names as of the publication.
    pub(crate) fn final_names(
        &self,
        classes: &Classes,
        variant: u32,
    ) -> io::Result<Vec<StockVariantName>> {
        self.finals[variant as usize]
            .iter()
            .flat_map(|kind| &kind.names)
            .map(|code| self.name(classes, code, None))
            .collect()
    }

    fn name(
        &self,
        classes: &Classes,
        code: &NameCode,
        names_hour: Option<i32>,
    ) -> io::Result<StockVariantName> {
        let first_hour = code
            .hour
            .or(names_hour)
            .ok_or_else(|| facts::invalid("a name dated by no read"))?;
        Ok(match code.raw {
            Some(raw) => {
                let raw = raw as usize;
                StockVariantName {
                    first_hour,
                    name: classes
                        .raw_names
                        .get(raw)
                        .ok_or_else(|| facts::invalid("a name of no raw operands"))?
                        .clone(),
                    known: true,
                    design: self.raws[raw].design.clone(),
                }
            }
            None => StockVariantName {
                first_hour,
                name: None,
                known: false,
                design: None,
            },
        })
    }

    pub(crate) fn variant_id(&self, handle: u64) -> Option<u32> {
        self.variant_ids.get(&handle).copied()
    }
}

/// A variant's type as content resolves its token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Class {
    pub key: String,
    pub ordinal: Option<usize>,
    pub domain: Option<Domain>,
    /// The type's localised name.
    pub name: Option<String>,
    /// Why content could not resolve it.
    pub reason: Option<String>,
}

/// What content resolves of an index: each classified variant's type, by
/// variant id, and each raw read's variant name.
pub(crate) struct Classes {
    pub of: Vec<Option<Class>>,
    pub raw_names: Vec<Option<String>>,
    /// The classified variants' ids in handle order.
    pub sorted: Vec<u32>,
}

impl Classes {
    pub(crate) fn new(index: &Index, named: Named<'_>) -> Self {
        let of = index
            .finals
            .iter()
            .enumerate()
            .map(|(variant, kind)| {
                let kind = kind.as_ref()?;
                let (kind, reason) = equipment::production::variant_type(
                    &index.tokens[kind.token as usize],
                    index.variants[variant],
                    named,
                );
                Some(Class {
                    name: named.type_name(&kind.key),
                    key: kind.key,
                    ordinal: kind.ordinal,
                    domain: kind.domain,
                    reason,
                })
            })
            .collect();
        let raw_names = index
            .raws
            .iter()
            .map(|raw| equipment::variant_names(raw, named.texts).0)
            .collect();
        Self {
            of,
            raw_names,
            sorted: Self::sorted(index),
        }
    }

    /// Classified variants' ids in handle order.
    pub(crate) fn sorted(index: &Index) -> Vec<u32> {
        let mut sorted: Vec<u32> = (0..index.finals.len() as u32)
            .filter(|&variant| index.finals[variant as usize].is_some())
            .collect();
        sorted.sort_unstable_by_key(|&variant| index.variants[variant as usize]);
        sorted
    }
}

/// A variant's classification as a spending reply reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kind {
    pub key: String,
    pub ordinal: Option<usize>,
    pub domain: Option<Domain>,
    pub name: Option<String>,
    pub reason: Option<String>,
    pub names: Vec<StockVariantName>,
}

/// The publication a reader answers from: the facts' range count and the
/// last range's identity.
fn publication(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<(u64, Option<[u64; 3]>)>> {
    let name = section(FACTS, branch);
    if !crate::prepared::coverage::accepts_pages(store, key, &name, branch)? {
        return Ok(None);
    }
    let count = store.range_count(key, &name)?.unwrap_or(0);
    let Some(last) = count.checked_sub(1) else {
        return Ok(None);
    };
    Ok(Some((
        count,
        store.range_id(key, &name, last)?.map(|id| id.cache_key()),
    )))
}

/// The branch's index and the classes the recorded content gives it, each
/// kept for the source while its publication (and content) is unchanged.
pub(crate) fn index(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<(Arc<Index>, Arc<Classes>)>> {
    let Some(published) = publication(store, key, branch)? else {
        return Ok(None);
    };
    let index = crate::prepared::query::observations::scope(source, "production:index", Some(key))
        .continue_entry(
            branch,
            &(),
            &published,
            &"production-index",
            cancelled,
            |previous| Index::read(store, key, branch, published.0, previous.as_deref(), cancelled),
        )?;
    let mut warnings = Vec::new();
    let (catalogue, units) = equipment::production::content(
        source,
        &crate::products::LoadContext {
            content: content.cloned(),
        },
        &mut warnings,
    );
    // The catalogue's address names it while the kept weak reference holds it.
    let content_of = (
        catalogue.as_ref().map(|held| Arc::as_ptr(held) as usize),
        units.as_ref().map(|held| Arc::as_ptr(held) as usize),
    );
    let classes = crate::prepared::query::observations::scope(
        source,
        "production:classes",
        Some(key),
    )
    .get_entry(
        branch,
        &content_of,
        &published,
        cancelled,
        || {
            let named = Named {
                units: units.as_deref(),
                texts: catalogue
                    .as_ref()
                    .map(|catalogue| &**catalogue as &dyn equipment::Texts),
            };
            Ok((
                catalogue.as_ref().map(Arc::downgrade),
                units.as_ref().map(Arc::downgrade),
                Arc::new(Classes::new(&index, named)),
            ))
        },
    )?;
    Ok(Some((index, Arc::clone(&classes.2))))
}

/// The branch's index decoded afresh, for tests that read it without a source.
#[cfg(test)]
pub(crate) fn read_index(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<Index>> {
    let Some((count, _)) = publication(store, key, branch)? else {
        return Ok(None);
    };
    Index::read(store, key, branch, count, None, &AtomicBool::new(false)).map(Some)
}
