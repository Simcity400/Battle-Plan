//! Production's prepared history: every tag's samples of the line and Rocket output, the
//! totals that changed at each and the hour each stopped holding, and every variant's names
//! as each read recorded them, as Equipment produced reads the branch. Each flush appends one
//! range beside the facts' range: a header, which every reply reads; every tag's running totals
//! whole and by refits, which a ranking of every type reads; one frame per tag of its samples and
//! one of its Rocket output, read for the tags a reply ranks or shows; one block per
//! equipment token class of every tag's totals of it, read only for the classes a ranking of
//! some types counts; and one frame per tag of its totals by design, read only for the tags a
//! reply shows.

use super::{check_cancel, section};
use crate::products::equipment::production::{
    CostTotal, Entry, Identity, ProductionSpendingUnknown, Purpose, RocketProduced,
    ScannedProduced, TagHistory, Total,
};
use crate::products::equipment::{RawVariant, VariantCatalogue, VariantVersion};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Seek};
use std::sync::{Arc, atomic::AtomicBool};

/// The section each branch's history ranges are appended to, as `{HISTORY}/{branch}`.
pub const HISTORY: &str = "production-history-v7";

const VERSION: u8 = 7;

/// The branch's history section.
pub(crate) fn section_of(branch: u64) -> String {
    section(HISTORY, branch)
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Prepared production history is malformed: {what}"),
    )
}

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u(&mut self, mut value: u128) {
        while value >= 0x80 {
            self.0.push(value as u8 | 0x80);
            value >>= 7;
        }
        self.0.push(value as u8);
    }
    fn i(&mut self, value: i128) {
        self.u(((value << 1) ^ (value >> 127)) as u128);
    }
    fn byte(&mut self, value: u8) {
        self.0.push(value);
    }
    fn str(&mut self, value: &str) {
        self.u(value.len() as u128);
        self.0.extend_from_slice(value.as_bytes());
    }
    fn hours(&mut self, hours: &[i32]) {
        self.u(hours.len() as u128);
        for hour in hours {
            self.i(i128::from(*hour));
        }
    }
}

struct In<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> In<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn done(&self) -> bool {
        self.at == self.bytes.len()
    }
    fn byte(&mut self) -> io::Result<u8> {
        let byte = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| invalid("a record runs past its range"))?;
        self.at += 1;
        Ok(byte)
    }
    fn u(&mut self) -> io::Result<u128> {
        let mut value = 0u128;
        for shift in (0..128).step_by(7) {
            let byte = self.byte()?;
            value |= u128::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid("a number is too long"))
    }
    fn i(&mut self) -> io::Result<i128> {
        let value = self.u()?;
        Ok((value >> 1) as i128 ^ -((value & 1) as i128))
    }
    fn small<T: TryFrom<u128>>(&mut self) -> io::Result<T> {
        T::try_from(self.u()?).map_err(|_| invalid("a number is out of range"))
    }
    fn hour(&mut self) -> io::Result<i32> {
        i32::try_from(self.i()?).map_err(|_| invalid("an hour is out of range"))
    }
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| invalid("a record runs past its range"))?;
        let bytes = &self.bytes[self.at..end];
        self.at = end;
        Ok(bytes)
    }
    fn str(&mut self) -> io::Result<String> {
        let count = self.small()?;
        String::from_utf8(self.take(count)?.to_vec()).map_err(|_| invalid("text is not UTF-8"))
    }
    fn hours(&mut self) -> io::Result<Vec<i32>> {
        (0..self.small::<usize>()?).map(|_| self.hour()).collect()
    }
}

fn reason_of(code: u8) -> io::Result<Option<ProductionSpendingUnknown>> {
    super::facts::reason_of(code)
}

fn purpose(code: u8) -> io::Result<Purpose> {
    Purpose::ALL
        .get(usize::from(code))
        .copied()
        .ok_or_else(|| invalid("an unknown purpose"))
}

// The writer.

/// One sample as the history records it.
pub(super) struct Sampled<'a> {
    pub hour: i32,
    pub starts: bool,
    pub identity: Identity,
    pub presence: Option<bool>,
    /// The entries whose totals changed, or none where every entry is listed.
    pub entries: Option<&'a [Entry]>,
    pub units: &'a BTreeMap<Entry, Total>,
    pub costs: &'a BTreeMap<Entry, CostTotal>,
}

/// A running total of a set of entries: its units (or applied IC) and how many of them are
/// not counted exactly.
type Sum = (i128, i64);

/// A token class's running totals: by (class, whether refits), each change as (sample, total).
type ClassLists = BTreeMap<(u32, bool), Vec<(u32, i128, i64)>>;

/// What one source of a holder recorded since the last flush, as the reply reads it.
#[derive(Default)]
struct Pending {
    hours: Vec<i32>,
    starts: Vec<bool>,
    identities: Vec<Identity>,
    presence: Vec<Option<bool>>,
    /// Each sample that stopped holding, and its last hour.
    closes: Vec<(u32, i32)>,
    changes: BTreeMap<Entry, Vec<(u32, Total)>>,
    cost_changes: BTreeMap<Entry, Vec<(u32, CostTotal)>>,
    /// The totals held when the first record since the flush came.
    base: Option<(BTreeMap<Entry, Total>, BTreeMap<Entry, CostTotal>)>,
    /// Records kept, for the flush policy.
    records: usize,
}

/// One source's history of a holder: its samples so far, the totals they last recorded and
/// the entries named so far, which a resumed preparation keeps, the token totals the last
/// flush ended with, and what it recorded since.
#[derive(Default, Deserialize, Serialize)]
pub(super) struct Log {
    samples: u32,
    #[serde(with = "crate::prepared_production::map_pairs")]
    units: BTreeMap<Entry, Total>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    costs: BTreeMap<Entry, CostTotal>,
    /// The entries it has said delivered units, and those it has said took applied IC.
    keys: BTreeSet<Entry>,
    #[serde(default)]
    cost_keys: BTreeSet<Entry>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    unit_classes: BTreeMap<(u32, bool), Sum>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    cost_classes: BTreeMap<(u32, bool), Sum>,
    /// The whole totals the last flush ended with, by whether refits.
    #[serde(with = "crate::prepared_production::map_pairs")]
    unit_totals: BTreeMap<(u32, bool), Sum>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    cost_totals: BTreeMap<(u32, bool), Sum>,
    #[serde(skip)]
    pending: Pending,
    /// The entries a sample takes from its track's edits.
    #[serde(skip)]
    pub scratch: Vec<Entry>,
}

impl Log {
    fn begin(&mut self) {
        if self.pending.base.is_none() {
            self.pending.base = Some((self.units.clone(), self.costs.clone()));
        }
    }

    /// The open sample stopped holding after `last`.
    pub fn close(&mut self, last: i32) {
        if let Some(sample) = self.samples.checked_sub(1) {
            self.begin();
            self.pending.closes.push((sample, last));
            self.pending.records += 1;
        }
    }

    /// A sample and the totals that changed at it.
    pub fn sample(&mut self, sample: &Sampled<'_>) {
        self.begin();
        let all: Vec<Entry>;
        let entries = match sample.entries {
            Some(entries) => entries,
            None => {
                all = sample
                    .units
                    .keys()
                    .chain(sample.costs.keys())
                    .chain(self.units.keys())
                    .chain(self.costs.keys())
                    .copied()
                    .collect::<BTreeSet<Entry>>()
                    .into_iter()
                    .collect();
                &all
            }
        };
        let index = self.samples;
        self.samples += 1;
        let Self {
            units,
            costs,
            pending,
            ..
        } = self;
        pending.hours.push(sample.hour);
        pending.starts.push(sample.starts);
        pending.identities.push(sample.identity);
        pending.presence.push(sample.presence);
        pending.records += 1;
        for &entry in entries {
            let now = sample.units.get(&entry).copied();
            pending.records += usize::from(take(units, &mut pending.changes, index, entry, now));
            let now = sample.costs.get(&entry).copied();
            pending.records +=
                usize::from(take(costs, &mut pending.cost_changes, index, entry, now));
        }
    }

    /// Bytes its records take, about.
    pub fn bytes(&self) -> usize {
        self.pending.records * 24
    }

    fn has_records(&self) -> bool {
        self.pending.records > 0
    }

    /// Its samples and closes since the last flush.
    fn write_samples(&self, out: &mut Out) {
        let pending = &self.pending;
        let first = self.samples - pending.hours.len() as u32;
        out.u(u128::from(first));
        out.u(pending.hours.len() as u128);
        let mut before = 0i128;
        let mut identity: Option<Identity> = None;
        for (index, hour) in pending.hours.iter().enumerate() {
            out.i(i128::from(*hour) - before);
            before = i128::from(*hour);
            let presence = match pending.presence[index] {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            };
            // An identity is written where it changes, and at each range's first sample.
            let changed = identity != Some(pending.identities[index]);
            out.byte(u8::from(pending.starts[index]) | presence << 1 | u8::from(changed) << 3);
            if changed {
                let (slot, country, observation) = pending.identities[index];
                out.u(u128::from(slot));
                out.u(u128::from(country));
                out.u(u128::from(observation));
                identity = Some(pending.identities[index]);
            }
        }
        out.u(pending.closes.len() as u128);
        for (sample, last) in &pending.closes {
            out.u(u128::from(*sample));
            out.i(i128::from(*last));
        }
    }

    /// Its totals by design since the last flush.
    fn write_entries(&self, out: &mut Out) {
        let pending = &self.pending;
        out.u(pending.changes.len() as u128);
        for (entry, list) in &pending.changes {
            write_entry(out, entry, list.len());
            let (mut sample, mut units) = (0u32, 0i128);
            for (at, total) in list {
                out.u(u128::from(at - sample));
                out.i(total.units - units);
                out.u(u128::from(total.inexact));
                (sample, units) = (*at, total.units);
            }
        }
        out.u(pending.cost_changes.len() as u128);
        for (entry, list) in &pending.cost_changes {
            write_entry(out, entry, list.len());
            let (mut sample, mut raw, mut unproven) = (0u32, 0i128, 0i128);
            for (at, cost) in list {
                out.u(u128::from(at - sample));
                out.i(cost.raw - raw);
                out.byte(super::facts::reason_code(cost.reason));
                out.i(cost.unproven_raw - unproven);
                (sample, raw, unproven) = (*at, cost.raw, cost.unproven_raw);
            }
        }
    }

    /// Its totals since the last flush, as units and as applied IC: by token class (`class`
    /// giving each design's), then whole; each by whether refits, at the samples they changed,
    /// every one the flush began with that moved restated at the range's first sample.
    fn lists(&mut self, class: &dyn Fn(u64) -> u32) -> [ClassLists; 4] {
        let first = self.samples - self.pending.hours.len() as u32;
        let (base_units, base_costs) = self.pending.base.take().unwrap_or_default();
        let units = |total: &Total| (total.units, i64::from(total.inexact));
        let costs = |cost: &CostTotal| (cost.raw, i64::from(cost.reason.is_some()));
        let whole = |_: u64| 0u32;
        let changes = &self.pending.changes;
        let cost_changes = &self.pending.cost_changes;
        [
            class_lists(&base_units, changes, &mut self.unit_classes, first, class, units),
            class_lists(&base_costs, cost_changes, &mut self.cost_classes, first, class, costs),
            class_lists(&base_units, changes, &mut self.unit_totals, first, &whole, units),
            class_lists(&base_costs, cost_changes, &mut self.cost_totals, first, &whole, costs),
        ]
    }

    /// The designs its records since the last flush hold.
    fn variants(&self, out: &mut BTreeSet<u64>) {
        out.extend(self.pending.changes.keys().map(|entry| entry.0));
        out.extend(self.pending.cost_changes.keys().map(|entry| entry.0));
    }

    /// The entries whose units, and those whose applied IC, it named since it last said, which
    /// it now has said: apart, as a scan keys each count.
    fn new_keys(&mut self) -> [Vec<Entry>; 2] {
        let units: Vec<Entry> = self
            .pending
            .changes
            .keys()
            .filter(|entry| !self.keys.contains(entry))
            .copied()
            .collect();
        let costs: Vec<Entry> = self
            .pending
            .cost_changes
            .keys()
            .filter(|entry| !self.cost_keys.contains(entry))
            .copied()
            .collect();
        self.keys.extend(units.iter().copied());
        self.cost_keys.extend(costs.iter().copied());
        [units, costs]
    }

    fn clear(&mut self) {
        self.pending = Pending::default();
    }
}

/// `changes` (each entry's totals at the samples they changed, from `base`) as each token
/// class's totals: `held` the classes' totals the last range ended with, which it then holds.
fn class_lists<V: Copy + Default>(
    base: &BTreeMap<Entry, V>,
    changes: &BTreeMap<Entry, Vec<(u32, V)>>,
    held: &mut BTreeMap<(u32, bool), Sum>,
    first: u32,
    class: &dyn Fn(u64) -> u32,
    value: impl Fn(&V) -> Sum,
) -> ClassLists {
    let value = &value;
    let key = |entry: &Entry| (class(entry.0), entry.1 == Purpose::Refit);
    // The classes' totals at the range's start, as its classes give them.
    let mut sums: BTreeMap<(u32, bool), Sum> = BTreeMap::new();
    for (entry, total) in base.iter().filter(|(entry, _)| entry.0 != 0) {
        let sum = sums.entry(key(entry)).or_default();
        let (units, inexact) = value(total);
        *sum = (sum.0 + units, sum.1 + inexact);
    }
    let mut out = ClassLists::new();
    // A class moved since the last range: restated at the first sample.
    for class in sums.keys().chain(held.keys()).copied().collect::<BTreeSet<_>>() {
        let now = sums.get(&class).copied().unwrap_or_default();
        if held.get(&class).copied().unwrap_or_default() != now {
            out.entry(class).or_default().push((first, now.0, now.1));
        }
    }
    let mut events: Vec<(u32, Entry, Sum)> = changes
        .iter()
        .filter(|(entry, _)| entry.0 != 0)
        .flat_map(|(entry, list)| list.iter().map(move |(sample, total)| (*sample, *entry, value(total))))
        .collect();
    events.sort_by_key(|(sample, entry, _)| (*sample, *entry));
    let mut current: BTreeMap<Entry, Sum> = base
        .iter()
        .filter(|(entry, _)| entry.0 != 0)
        .map(|(entry, total)| (*entry, value(total)))
        .collect();
    let mut index = 0;
    while index < events.len() {
        let sample = events[index].0;
        let mut touched = BTreeSet::new();
        while let Some(&(_, entry, now)) = events.get(index).filter(|event| event.0 == sample) {
            let before = current.insert(entry, now).unwrap_or_default();
            let sum = sums.entry(key(&entry)).or_default();
            *sum = (sum.0 + now.0 - before.0, sum.1 + now.1 - before.1);
            touched.insert(key(&entry));
            index += 1;
        }
        for class in touched {
            let now = sums[&class];
            let list = out.entry(class).or_default();
            let last = list.last().map(|(_, value, inexact)| (*value, *inexact)).unwrap_or_else(|| {
                held.get(&class).copied().unwrap_or_default()
            });
            if last != now {
                if list.last().is_some_and(|(at, _, _)| *at == sample) {
                    let held = list.last_mut().expect("a change at this sample");
                    (held.1, held.2) = now;
                } else {
                    list.push((sample, now.0, now.1));
                }
            }
        }
    }
    *held = sums.into_iter().filter(|(_, sum)| *sum != (0, 0)).collect();
    out
}

fn write_list(out: &mut Out, list: &[(u32, i128, i64)]) {
    out.u(list.len() as u128);
    let (mut sample, mut total) = (0u32, 0i128);
    for (at, value, inexact) in list {
        out.u(u128::from(at - sample));
        out.i(value - total);
        out.i(i128::from(*inexact));
        (sample, total) = (*at, *value);
    }
}

fn read_list(input: &mut In<'_>, into: &mut Vec<(u32, i128, i64)>) -> io::Result<()> {
    let (mut sample, mut total) = (0u32, 0i128);
    for _ in 0..input.small::<usize>()? {
        sample = sample
            .checked_add(input.small()?)
            .ok_or_else(|| invalid("a total of no sample"))?;
        total += input.i()?;
        let inexact = i64::try_from(input.i()?).map_err(|_| invalid("a count is out of range"))?;
        into.push((sample, total, inexact));
    }
    Ok(())
}

fn write_entry(out: &mut Out, entry: &Entry, count: usize) {
    out.u(u128::from(entry.0));
    out.byte(entry.1.index() as u8);
    out.u(count as u128);
}

fn read_entry(input: &mut In<'_>) -> io::Result<(Entry, usize)> {
    let variant = input.small()?;
    let purpose = purpose(input.byte()?)?;
    Ok(((variant, purpose), input.small()?))
}

/// `entry`'s total at `sample` (none: not held), noted where it differs from what it held;
/// true when noted.
fn take<V: Copy + Default + PartialEq>(
    held: &mut BTreeMap<Entry, V>,
    changes: &mut BTreeMap<Entry, Vec<(u32, V)>>,
    sample: u32,
    entry: Entry,
    now: Option<V>,
) -> bool {
    let before = held.get(&entry).copied().unwrap_or_default();
    let value = now.unwrap_or_default();
    let changed = before != value;
    if changed {
        changes.entry(entry).or_default().push((sample, value));
    }
    match now {
        Some(now) => {
            held.insert(entry, now);
        }
        None => {
            held.remove(&entry);
        }
    }
    changed
}

/// Every variant's names as the variant table's reads recorded them, as a scan of the branch
/// names them: a version wherever a complete read's operands for it differ from its last, and
/// an unknown one for each known variant wherever names stop being known.
#[derive(Default, Deserialize, Serialize)]
pub(super) struct Names {
    /// Each variant's latest version's operands, by raw operand id; none while unknown.
    #[serde(with = "crate::prepared_production::map_pairs")]
    latest: BTreeMap<u64, Option<u32>>,
    /// Names were forgotten since the last complete read: it names every live variant anew.
    forgotten: bool,
    /// Each variant's latest token, of the latest operands that spelled one.
    #[serde(with = "crate::prepared_production::map_pairs")]
    tokens: BTreeMap<u64, String>,
    #[serde(skip)]
    events: Vec<(u64, i32, Option<u32>)>,
}

impl Names {
    /// A complete read at `hour`: `read` the operands of every variant it may have changed,
    /// `live` every variant's current operands, which a read after forgetting names whole.
    pub fn read(
        &mut self,
        hour: i32,
        read: impl IntoIterator<Item = (u64, u32, String)>,
        live: impl IntoIterator<Item = (u64, u32, String)>,
    ) {
        let Self {
            latest,
            tokens,
            events,
            forgotten,
        } = self;
        let mut apply = |id: u64, raw: u32, token: String| {
            if latest.get(&id).copied().flatten() != Some(raw) {
                latest.insert(id, Some(raw));
                events.push((id, hour, Some(raw)));
            }
            if !token.is_empty() {
                tokens.insert(id, token);
            }
        };
        if std::mem::take(forgotten) {
            for (id, raw, token) in live {
                apply(id, raw, token);
            }
        } else {
            for (id, raw, token) in read {
                apply(id, raw, token);
            }
        }
    }

    /// Whether the next complete read names every live variant.
    pub fn forgotten(&self) -> bool {
        self.forgotten
    }

    /// A variant's latest token, empty without one.
    fn token(&self, variant: u64) -> &str {
        self.tokens.get(&variant).map_or("", String::as_str)
    }

    /// Names are not known from `hour` until the next complete read.
    pub fn forget(&mut self, hour: i32) {
        self.forgotten = true;
        for (id, latest) in &mut self.latest {
            if latest.take().is_some() {
                self.events.push((*id, hour, None));
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.events.len() * 16
    }
}

/// What the branch read since the last flush, beside the holders' events.
#[derive(Default)]
pub(super) struct Batch {
    pub reads: Vec<i32>,
    pub observations: Vec<i32>,
    pub rocket_reads: Vec<i32>,
    pub rocket_observations: Vec<i32>,
}

/// The branch's counts as of a flush: each range carries them whole.
pub(super) struct Counts<'a> {
    pub last_hour: Option<i32>,
    pub unattributed: usize,
    pub inexact: usize,
    pub lost: usize,
    pub unlisted: usize,
    pub unread: usize,
    pub unread_reason: Option<&'a str>,
    pub unnamed: usize,
    pub missed: usize,
    pub tainted: &'a BTreeMap<u32, i32>,
    pub spending_tainted: &'a BTreeMap<u32, (i32, ProductionSpendingUnknown)>,
    pub rocket_unread: usize,
    pub rocket_unattributed: usize,
    pub rocket_inconsistent: usize,
    pub refusals: &'a BTreeMap<String, BTreeSet<u64>>,
}

/// The token classes the branch's designs are totalled by, kept across flushes: a design's
/// class is its token, or the design alone while it has none.
#[derive(Default, Deserialize, Serialize)]
pub(super) struct Classes {
    names: Vec<String>,
    ids: BTreeMap<String, u32>,
    /// Every design's class as last written: a design held but unchanged in a range is still
    /// totalled by it.
    #[serde(with = "crate::prepared_production::map_pairs")]
    of: BTreeMap<u64, u32>,
    written: usize,
    raws_written: usize,
}

impl Classes {
    fn id(&mut self, token: &str, variant: u64) -> u32 {
        if let Some(&id) = (!token.is_empty()).then(|| self.ids.get(token)).flatten() {
            return id;
        }
        let name = class_name(token, variant);
        if let Some(&id) = self.ids.get(&name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.clone());
        self.ids.insert(name, id);
        id
    }

    /// Classes every design `new` names and every one held before by its token now: those
    /// that are new or moved, as (design, class).
    fn assign(&mut self, names: &Names, new: BTreeSet<u64>) -> Vec<(u64, u32)> {
        let designs: BTreeSet<u64> = self.of.keys().copied().chain(new).filter(|variant| *variant != 0).collect();
        let mut assigned = Vec::new();
        for variant in designs {
            let id = self.id(names.token(variant), variant);
            if self.of.insert(variant, id) != Some(id) {
                assigned.push((variant, id));
            }
        }
        assigned
    }
}

/// A design's class name: its token, or its handle while it has none.
pub(crate) fn class_name(token: &str, variant: u64) -> String {
    if token.is_empty() {
        format!("#{variant}")
    } else {
        token.to_owned()
    }
}

/// What one flush writes besides the counts.
pub(super) struct Flush<'a> {
    pub batch: &'a mut Batch,
    pub names: &'a mut Names,
    pub classes: &'a mut Classes,
    /// The raw operands every name names, by id.
    pub raws: &'a [RawVariant],
}

/// The bytes before a range's parts: the header's and the totals' lengths.
const LENGTHS: usize = 16;

/// One flush's range: its header, its totals, each holder's frame of its samples and then of its
/// Rocket output in tag order, its class blocks in class order and each holder's frame of its
/// totals by design, in tag order. The range is self-contained: a reader needs its bytes alone.
pub(super) fn range<'a>(
    flush: Flush<'_>,
    counts: &Counts<'_>,
    logs: impl Iterator<Item = (&'a str, &'a mut [Log; 2])>,
) -> io::Result<Vec<u8>> {
    let logs: Vec<(&str, &mut [Log; 2])> = logs.filter(|(_, logs)| logs.iter().any(Log::has_records)).collect();
    // The class of every design held, as its token now gives it.
    let mut variants = BTreeSet::new();
    for (_, logs) in &logs {
        logs[0].variants(&mut variants);
    }
    let assigned = flush.classes.assign(flush.names, variants);
    let of = &flush.classes.of;
    let class = |variant: u64| -> u32 { of.get(&variant).copied().unwrap_or(u32::MAX) };
    let mut totals = Out::default();
    let mut sampled: Vec<Vec<u8>> = Vec::with_capacity(logs.len());
    let mut rockets: Vec<Vec<u8>> = Vec::new();
    // Each class's block: how many tags it holds, then each tag's totals of it.
    let mut blocks: BTreeMap<(u32, bool), (usize, Out)> = BTreeMap::new();
    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(logs.len());
    let mut directory = Out::default();
    directory.u(logs.len() as u128);
    let mut logs = logs;
    for (ordinal, (tag, logs)) in logs.iter_mut().enumerate() {
        let [equipment, rocket] = &mut **logs;
        let (mut samples_len, mut samples_new) = (0, 0);
        if equipment.has_records() {
            let mut own = Out::default();
            equipment.write_samples(&mut own);
            let frame = crate::prepared::series::compress(&own.0, 3)?;
            (samples_len, samples_new) = (frame.len(), equipment.pending.hours.len());
            sampled.push(frame);
            let [unit_classes, cost_classes, unit_totals, cost_totals] = equipment.lists(&class);
            totals.u(ordinal as u128);
            for lists in [&unit_totals, &cost_totals] {
                for refits in [false, true] {
                    write_list(&mut totals, lists.get(&(0, refits)).map_or(&[][..], Vec::as_slice));
                }
            }
            let held: BTreeSet<(u32, bool)> = unit_classes.keys().chain(cost_classes.keys()).copied().collect();
            for key in held {
                let (count, block) = blocks.entry(key).or_default();
                *count += 1;
                block.u(ordinal as u128);
                for lists in [&unit_classes, &cost_classes] {
                    write_list(block, lists.get(&key).map_or(&[][..], Vec::as_slice));
                }
            }
        }
        let (mut rocket_len, mut rocket_new) = (0, 0);
        if rocket.has_records() {
            let mut own = Out::default();
            rocket.write_samples(&mut own);
            rocket.write_entries(&mut own);
            let frame = crate::prepared::series::compress(&own.0, 3)?;
            (rocket_len, rocket_new) = (frame.len(), rocket.pending.hours.len());
            rockets.push(frame);
        }
        let mut entries = Out::default();
        equipment.write_entries(&mut entries);
        let frame = crate::prepared::series::compress(&entries.0, 3)?;
        directory.str(tag);
        directory.u(samples_len as u128);
        directory.u(samples_new as u128);
        directory.u(rocket_len as u128);
        directory.u(rocket_new as u128);
        directory.u(frame.len() as u128);
        for log in [&mut *equipment, &mut *rocket] {
            for keys in log.new_keys() {
                directory.u(keys.len() as u128);
                for (variant, purpose) in keys {
                    directory.u(u128::from(variant));
                    directory.byte(purpose.index() as u8);
                }
            }
        }
        frames.push(frame);
        equipment.clear();
        rocket.clear();
    }
    let blocks: Vec<((u32, bool), Vec<u8>)> = blocks
        .into_iter()
        .map(|(key, (count, block))| {
            let mut out = Out::default();
            out.u(count as u128);
            out.0.extend_from_slice(&block.0);
            Ok((key, crate::prepared::series::compress(&out.0, 3)?))
        })
        .collect::<io::Result<_>>()?;
    directory.u(blocks.len() as u128);
    for ((class, refits), block) in &blocks {
        directory.u(u128::from(*class));
        directory.byte(u8::from(*refits));
        directory.u(block.len() as u128);
    }
    let mut header = Out::default();
    header.byte(VERSION);
    match counts.last_hour {
        Some(hour) => {
            header.byte(1);
            header.i(i128::from(hour));
        }
        None => header.byte(0),
    }
    let batch = &mut *flush.batch;
    header.hours(&std::mem::take(&mut batch.reads));
    header.hours(&std::mem::take(&mut batch.observations));
    header.hours(&std::mem::take(&mut batch.rocket_reads));
    header.hours(&std::mem::take(&mut batch.rocket_observations));
    for count in [
        counts.unattributed,
        counts.inexact,
        counts.lost,
        counts.unlisted,
        counts.unread,
        counts.unnamed,
        counts.missed,
        counts.rocket_unread,
        counts.rocket_unattributed,
        counts.rocket_inconsistent,
    ] {
        header.u(count as u128);
    }
    match counts.unread_reason {
        Some(reason) => {
            header.byte(1);
            header.str(reason);
        }
        None => header.byte(0),
    }
    header.u(counts.tainted.len() as u128);
    for (observation, hour) in counts.tainted {
        header.u(u128::from(*observation));
        header.i(i128::from(*hour));
    }
    header.u(counts.spending_tainted.len() as u128);
    for (observation, (hour, reason)) in counts.spending_tainted {
        header.u(u128::from(*observation));
        header.i(i128::from(*hour));
        header.byte(super::facts::reason_code(Some(*reason)));
    }
    header.u(counts.refusals.len() as u128);
    for (reason, variants) in counts.refusals {
        header.str(reason);
        header.u(variants.len() as u128);
        for variant in variants {
            header.u(u128::from(*variant));
        }
    }
    // Classes named since the last flush, and every design's class that is new or moved.
    let classes = &mut *flush.classes;
    header.u((classes.names.len() - classes.written) as u128);
    for name in &classes.names[classes.written..] {
        header.str(name);
    }
    classes.written = classes.names.len();
    header.u(assigned.len() as u128);
    for (variant, class) in &assigned {
        header.u(u128::from(*variant));
        header.u(u128::from(*class));
    }
    // Raw operands named since the last flush, then the names their reads recorded.
    header.u((flush.raws.len() - classes.raws_written) as u128);
    for raw in &flush.raws[classes.raws_written..] {
        let mut bytes = super::facts::Out::default();
        super::read::write_raw(raw, &mut bytes)?;
        header.u(bytes.0.len() as u128);
        header.0.extend_from_slice(&bytes.0);
    }
    classes.raws_written = flush.raws.len();
    let events = std::mem::take(&mut flush.names.events);
    header.u(events.len() as u128);
    for (variant, hour, raw) in events {
        header.u(u128::from(variant));
        header.i(i128::from(hour));
        header.u(raw.map_or(0, |raw| u128::from(raw) + 1));
    }
    header.0.extend_from_slice(&directory.0);
    let header = crate::prepared::series::compress(&header.0, 3)?;
    let totals = crate::prepared::series::compress(&totals.0, 3)?;
    let parts = [&header, &totals];
    let mut out = Vec::with_capacity(
        LENGTHS
            + parts.iter().map(|part| part.len()).sum::<usize>()
            + sampled.iter().chain(&rockets).map(Vec::len).sum::<usize>()
            + blocks.iter().map(|(_, block)| block.len()).sum::<usize>()
            + frames.iter().map(Vec::len).sum::<usize>(),
    );
    for part in parts {
        out.extend_from_slice(&(part.len() as u64).to_le_bytes());
    }
    for part in parts {
        out.extend_from_slice(part);
    }
    for frame in sampled.iter().chain(&rockets) {
        out.extend_from_slice(frame);
    }
    for (_, block) in &blocks {
        out.extend_from_slice(block);
    }
    for frame in frames {
        out.extend_from_slice(&frame);
    }
    Ok(out)
}

// The reader.

/// A track being read: its history, and whether its last sample still holds.
#[derive(Default)]
struct Track {
    history: TagHistory,
    open: bool,
}

impl Track {
    /// Appends one range's samples and closes.
    fn samples(&mut self, input: &mut In<'_>) -> io::Result<()> {
        let history = &mut self.history;
        let first: usize = input.small()?;
        if first != history.hours.len() {
            return Err(invalid("a range's samples do not follow the samples before it"));
        }
        let count: usize = input.small()?;
        history.hours.reserve(count);
        history.last_hours.reserve(count);
        history.starts.reserve(count);
        history.presence.reserve(count);
        history.identities.reserve(count);
        let mut hour = 0i128;
        let mut identity: Option<Identity> = None;
        for _ in 0..count {
            hour += input.i()?;
            let hour = i32::try_from(hour).map_err(|_| invalid("an hour is out of range"))?;
            let flags = input.byte()?;
            if flags >> 4 != 0 {
                return Err(invalid("unknown sample flags"));
            }
            history.hours.push(hour);
            history.last_hours.push(hour);
            history.starts.push(flags & 1 != 0);
            history.presence.push(match (flags >> 1) & 3 {
                0 => None,
                1 => Some(false),
                2 => Some(true),
                _ => return Err(invalid("an unknown presence")),
            });
            if flags & 8 != 0 {
                identity = Some((input.small()?, input.small()?, input.small()?));
            }
            history
                .identities
                .push(identity.ok_or_else(|| invalid("a range's first sample has no identity"))?);
        }
        let samples = history.hours.len();
        self.open |= count > 0;
        for _ in 0..input.small::<usize>()? {
            let sample: usize = input.small()?;
            let last = input.hour()?;
            *history
                .last_hours
                .get_mut(sample)
                .ok_or_else(|| invalid("a close of no sample"))? = last;
            if sample + 1 == samples {
                self.open = false;
            }
        }
        Ok(())
    }

    /// The history read, its last sample held to `last_hour` while it still holds.
    fn finish(mut self, last_hour: Option<i32>) -> TagHistory {
        if self.open
            && let (Some(held), Some(last)) = (self.history.last_hours.last_mut(), last_hour)
        {
            *held = last;
        }
        self.history
    }
}

/// Appends one range's totals by design, of a track holding `samples` samples.
fn read_entries(
    input: &mut In<'_>,
    samples: usize,
    changes: &mut BTreeMap<Entry, Vec<(u32, Total)>>,
    cost_changes: &mut BTreeMap<Entry, Vec<(u32, CostTotal)>>,
) -> io::Result<()> {
    let sample_of = |at: u32| {
        (at as usize)
            .lt(&samples)
            .then_some(at)
            .ok_or_else(|| invalid("a change of no sample"))
    };
    for _ in 0..input.small::<usize>()? {
        let (entry, count) = read_entry(input)?;
        let list = changes.entry(entry).or_default();
        let (mut sample, mut units) = (0u32, 0i128);
        for _ in 0..count {
            sample = sample
                .checked_add(input.small()?)
                .ok_or_else(|| invalid("a change of no sample"))?;
            units += input.i()?;
            list.push((
                sample_of(sample)?,
                Total {
                    units,
                    inexact: input.small()?,
                },
            ));
        }
    }
    for _ in 0..input.small::<usize>()? {
        let (entry, count) = read_entry(input)?;
        let list = cost_changes.entry(entry).or_default();
        let (mut sample, mut raw, mut unproven) = (0u32, 0i128, 0i128);
        for _ in 0..count {
            sample = sample
                .checked_add(input.small()?)
                .ok_or_else(|| invalid("a change of no sample"))?;
            raw += input.i()?;
            let reason = reason_of(input.byte()?)?;
            unproven += input.i()?;
            list.push((
                sample_of(sample)?,
                CostTotal {
                    raw,
                    reason,
                    unproven_raw: unproven,
                },
            ));
        }
    }
    Ok(())
}

/// Where one part of one tag lies in one range.
#[derive(Clone, Copy, Debug)]
struct Frame {
    ordinal: u64,
    offset: u64,
    len: u64,
}

/// One tag's frames: of its samples, of its Rocket output and of its totals by design.
#[derive(Default)]
struct Frames {
    samples: Vec<Frame>,
    rockets: Vec<Frame>,
    entries: Vec<Frame>,
    /// Its samples and its Rocket samples, through every range.
    count: usize,
    rocket_count: usize,
}

/// Where one range's parts a ranking reads lie: its totals, its tags' samples and Rocket
/// output, each class block, and its tags by their place in it.
struct Layout {
    ordinal: u64,
    totals: (u64, u64),
    samples: (u64, u64),
    rockets: (u64, u64),
    blocks: BTreeMap<(u32, bool), (u64, u64)>,
    tags: Vec<String>,
}

/// A branch's history ranges, wherever they are kept: each range the self-contained payload one
/// flush wrote ([`range`]), read by ordinal and offset alone.
pub(crate) trait Ranges: Sync {
    /// `len` bytes at `offset` of range `ordinal`.
    fn read(&self, ordinal: u64, offset: u64, len: u64) -> io::Result<Vec<u8>>;
}

/// The ranges a prepared store keeps in a branch's history section.
pub(crate) struct Stored<'a> {
    pub store: &'a store::Store,
    pub key: &'a store::SourceKey,
    pub name: String,
}

impl Ranges for Stored<'_> {
    fn read(&self, ordinal: u64, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        let mut range = self
            .store
            .read(self.key, &self.name, ordinal)?
            .ok_or_else(|| invalid("a published range is missing"))?;
        range.seek(io::SeekFrom::Start(offset))?;
        let mut bytes =
            vec![0u8; usize::try_from(len).map_err(|_| invalid("a part is too long"))?];
        range.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}

/// Reads `len` bytes at `offset` of a range and decompresses them.
fn part(ranges: &dyn Ranges, ordinal: u64, (offset, len): (u64, u64)) -> io::Result<Vec<u8>> {
    crate::prepared::series::decompress(&ranges.read(ordinal, offset, len)?)
}

/// One range's header, decompressed, and its parts' lengths.
fn fetch(
    ranges: &dyn Ranges,
    ordinal: u64,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<u8>, [u64; 2])> {
    check_cancel(cancelled)?;
    let bytes = ranges.read(ordinal, 0, LENGTHS as u64)?;
    let lengths: [u64; 2] = std::array::from_fn(|at| {
        u64::from_le_bytes(bytes[at * 8..at * 8 + 8].try_into().expect("eight bytes"))
    });
    Ok((part(ranges, ordinal, (LENGTHS as u64, lengths[0]))?, lengths))
}

/// Each frame of one kind, decompressed and read in order: a region of each range read at once.
fn regions<'a>(
    ranges: &dyn Ranges,
    layouts: &'a [Layout],
    region: impl Fn(&Layout) -> (u64, u64) + Sync,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<u64, (u64, Vec<u8>)>> {
    let read = crate::workers::map(layouts, "production history", |layout| {
        check_cancel(cancelled)?;
        let (offset, len) = region(layout);
        ranges.read(layout.ordinal, offset, len)
    })?;
    Ok(layouts
        .iter()
        .zip(read)
        .map(|(layout, bytes)| (layout.ordinal, (region(layout).0, bytes)))
        .collect())
}

/// A frame's bytes in its range's region.
fn within<'a>(regions: &'a BTreeMap<u64, (u64, Vec<u8>)>, frame: &Frame) -> io::Result<&'a [u8]> {
    let (start, bytes) = regions
        .get(&frame.ordinal)
        .ok_or_else(|| invalid("a frame of no range"))?;
    let at = usize::try_from(frame.offset - start).map_err(|_| invalid("a frame is too far"))?;
    let end = at + usize::try_from(frame.len).map_err(|_| invalid("a frame is too long"))?;
    bytes
        .get(at..end)
        .ok_or_else(|| invalid("a frame runs past its range"))
}

/// A tag's samples from its frames, in range order.
fn sampled<'a>(
    frames: impl Iterator<Item = io::Result<std::borrow::Cow<'a, [u8]>>>,
    last_hour: Option<i32>,
) -> io::Result<TagHistory> {
    let mut track = Track::default();
    for bytes in frames {
        let bytes = crate::prepared::series::decompress(&bytes?)?;
        let mut input = In::new(&bytes);
        track.samples(&mut input)?;
        if !input.done() {
            return Err(invalid("a frame has trailing bytes"));
        }
    }
    Ok(track.finish(last_hour))
}

/// A tag's Rocket output from its frames, in range order: its samples, then its totals.
fn rocket_of<'a>(
    frames: impl Iterator<Item = io::Result<std::borrow::Cow<'a, [u8]>>>,
    last_hour: Option<i32>,
    named: Option<&TagHistory>,
    compressed: bool,
) -> io::Result<TagHistory> {
    let mut track = Track::default();
    let mut totals: Vec<Vec<u8>> = Vec::new();
    for bytes in frames {
        let bytes = bytes?;
        let bytes = if compressed {
            std::borrow::Cow::Owned(crate::prepared::series::decompress(&bytes)?)
        } else {
            bytes
        };
        let mut input = In::new(&bytes);
        track.samples(&mut input)?;
        totals.push(bytes[input.at..].to_vec());
    }
    let mut history = track.finish(last_hour);
    let samples = history.hours.len();
    for bytes in totals {
        let mut input = In::new(&bytes);
        read_entries(&mut input, samples, &mut history.changes, &mut history.cost_changes)?;
        if !input.done() {
            return Err(invalid("a frame has trailing bytes"));
        }
    }
    if let Some(named) = named {
        for entry in named.changes.keys() {
            history.changes.entry(*entry).or_default();
        }
        for entry in named.cost_changes.keys() {
            history.cost_changes.entry(*entry).or_default();
        }
    }
    Ok(history)
}

/// What a ranking of some types counts, by token class: whether a class is counted, and
/// whether it is left out though it could belong to a counted type.
pub(crate) struct CountPlan<'a> {
    pub costs: bool,
    pub refits: bool,
    pub counted: &'a [bool],
    pub outside: &'a [bool],
}

/// A tag's counted total and its left-out total, each as (sample, total, exact) at the samples
/// it changed.
pub(crate) struct Counted {
    pub sums: Vec<(u32, i128, bool)>,
    pub outside: Vec<(u32, i128, bool)>,
}

/// One tag's lists a ranking sums, as read so far: its whole totals by whether refits, and its
/// totals of each class read.
#[derive(Default)]
struct Terms {
    totals: [Vec<(u32, i128, i64)>; 2],
    classes: BTreeMap<(u32, bool), Vec<(u32, i128, i64)>>,
}

/// Lists summed, each subtracted where it says so: their sum at each sample any of them
/// changed, as a group's totals are.
fn signed_sums<'a>(
    lists: impl Iterator<Item = (bool, &'a [(u32, i128, i64)])>,
    samples: usize,
) -> io::Result<Vec<(u32, i128, bool)>> {
    let overflow = || io::Error::other("Produced equipment group exceeds its exact integer range");
    let mut cells: Vec<(i128, i64, bool)> = vec![(0, 0, false); samples];
    for (less, list) in lists {
        let mut before = (0i128, 0i64);
        for &(sample, value, inexact) in list {
            let mut change = value.checked_sub(before.0).ok_or_else(overflow)?;
            let mut counts = inexact - before.1;
            before = (value, inexact);
            // A restatement at a range that took no sample yet changes no sample.
            let Some(cell) = cells.get_mut(sample as usize) else {
                continue;
            };
            if less {
                change = change.checked_neg().ok_or_else(overflow)?;
                counts = -counts;
            }
            cell.0 = cell.0.checked_add(change).ok_or_else(overflow)?;
            cell.1 += counts;
            cell.2 = true;
        }
    }
    let mut out: Vec<(u32, i128, bool)> = Vec::new();
    let mut sum = (0i128, 0i64);
    for (sample, cell) in cells.iter().enumerate() {
        if !cell.2 {
            continue;
        }
        sum.0 = sum.0.checked_add(cell.0).ok_or_else(overflow)?;
        sum.1 += cell.1;
        let value = (sample as u32, sum.0, sum.1 == 0);
        if out.last().is_none_or(|last| (last.1, last.2) != (value.1, value.2)) {
            out.push(value);
        }
    }
    Ok(out)
}

/// A branch's history as of a publication, every tag's samples and totals left to be read: what
/// every reply reads (the entries each tag holds, the counts), the variants' names and classes,
/// and where each range's parts and each tag's frames lie.
pub(crate) struct Base {
    /// Each tag's entries named, its equipment and Rocket samples and totals not read
    /// ([`Self::light`], [`Self::whole`]).
    pub scanned: ScannedProduced,
    frames: BTreeMap<String, Frames>,
    ranges: Vec<Layout>,
    last_hour: Option<i32>,
    /// Designs totalled in a range by a class their final token does not give: a ranking that
    /// reads classes reads a tag holding one whole.
    moved: BTreeSet<u64>,
}

impl Base {
    /// The branch's history through its first `count` ranges.
    pub(crate) fn read(ranges: &dyn Ranges, count: u64, cancelled: &AtomicBool) -> io::Result<Self> {
        let ordinals: Vec<u64> = (0..count).collect();
        let fetched = crate::workers::map(&ordinals, "production history", |&ordinal| {
            fetch(ranges, ordinal, cancelled)
        })?;
        let mut keys: BTreeMap<String, TagHistory> = BTreeMap::new();
        let mut scanned = ScannedProduced::default();
        let mut rocket = RocketProduced::default();
        let mut last_hour = None;
        let mut frames: BTreeMap<String, Frames> = BTreeMap::new();
        let mut layouts = Vec::with_capacity(fetched.len());
        let mut classes: Vec<String> = Vec::new();
        let mut assignments: BTreeMap<u64, BTreeSet<u32>> = BTreeMap::new();
        let mut raws: Vec<RawVariant> = Vec::new();
        let mut versions: BTreeMap<u64, Vec<VariantVersion>> = BTreeMap::new();
        let mut refusals: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
        let mut rocket_keys: BTreeMap<String, [BTreeSet<Entry>; 2]> = BTreeMap::new();
        for (ordinal, (header, lengths)) in ordinals.iter().copied().zip(fetched) {
            check_cancel(cancelled)?;
            let mut input = In::new(&header);
            if input.byte()? != VERSION {
                return Err(invalid("an unknown version"));
            }
            last_hour = match input.byte()? {
                0 => None,
                1 => Some(input.hour()?),
                _ => return Err(invalid("an unknown flag")),
            };
            scanned.reads.extend(input.hours()?);
            scanned.observations.extend(input.hours()?);
            rocket.reads.extend(input.hours()?);
            rocket.observations.extend(input.hours()?);
            scanned.unattributed = input.small()?;
            scanned.inexact = input.small()?;
            scanned.lost = input.small()?;
            scanned.unlisted = input.small()?;
            scanned.unread = input.small()?;
            scanned.unnamed = input.small()?;
            scanned.missed = input.small()?;
            rocket.unread = input.small()?;
            rocket.unattributed = input.small()?;
            rocket.inconsistent = input.small()?;
            scanned.unread_reason = match input.byte()? {
                0 => None,
                1 => Some(input.str()?),
                _ => return Err(invalid("an unknown flag")),
            };
            scanned.tainted = (0..input.small::<usize>()?)
                .map(|_| Ok((input.small()?, input.hour()?)))
                .collect::<io::Result<_>>()?;
            scanned.spending_tainted = (0..input.small::<usize>()?)
                .map(|_| {
                    let observation = input.small()?;
                    let hour = input.hour()?;
                    let reason = reason_of(input.byte()?)?
                        .ok_or_else(|| invalid("a taint has no reason"))?;
                    Ok((observation, (hour, reason)))
                })
                .collect::<io::Result<_>>()?;
            refusals = (0..input.small::<usize>()?)
                .map(|_| {
                    let reason = input.str()?;
                    let variants = (0..input.small::<usize>()?)
                        .map(|_| input.small())
                        .collect::<io::Result<_>>()?;
                    Ok((reason, variants))
                })
                .collect::<io::Result<_>>()?;
            for _ in 0..input.small::<usize>()? {
                classes.push(input.str()?);
            }
            for _ in 0..input.small::<usize>()? {
                let variant: u64 = input.small()?;
                let class: u32 = input.small()?;
                if class as usize >= classes.len() {
                    return Err(invalid("a design of no class"));
                }
                assignments.entry(variant).or_default().insert(class);
            }
            for _ in 0..input.small::<usize>()? {
                let len = input.small()?;
                let mut raw = super::facts::In::new(input.take(len)?);
                raws.push(super::read::read_raw(&mut raw)?);
            }
            for _ in 0..input.small::<usize>()? {
                let variant: u64 = input.small()?;
                let hour = input.hour()?;
                let raw: usize = input.small()?;
                let raw = match raw.checked_sub(1) {
                    None => None,
                    Some(raw) => Some(RawVariant {
                        first_hour: hour,
                        ..raws
                            .get(raw)
                            .ok_or_else(|| invalid("a name of no raw operands"))?
                            .clone()
                    }),
                };
                let list = versions.entry(variant).or_default();
                match list.last_mut() {
                    Some(last) if last.first_hour >= hour => {
                        *last = VariantVersion {
                            first_hour: hour,
                            raw,
                        }
                    }
                    _ => list.push(VariantVersion {
                        first_hour: hour,
                        raw,
                    }),
                }
            }
            // Past the lengths, the header and the totals: the tags' samples, their Rocket
            // output, the class blocks, then the tags' totals by design.
            let totals_at = LENGTHS as u64 + lengths[0];
            let samples_at = totals_at + lengths[1];
            let tags = input.small::<usize>()?;
            let mut placed = Vec::with_capacity(tags);
            let mut lens = Vec::with_capacity(tags);
            let mut rocket_lens = Vec::with_capacity(tags);
            let mut offset = samples_at;
            for _ in 0..tags {
                let tag = input.str()?;
                let samples_len: u64 = input.small()?;
                let samples_new: usize = input.small()?;
                let rocket_len: u64 = input.small()?;
                let rocket_new: usize = input.small()?;
                let entries_len: u64 = input.small()?;
                let held = frames.entry(tag.clone()).or_default();
                if samples_len > 0 {
                    held.samples.push(Frame {
                        ordinal,
                        offset,
                        len: samples_len,
                    });
                    held.count += samples_new;
                    offset += samples_len;
                }
                held.rocket_count += rocket_new;
                rocket_lens.push(rocket_len);
                let track = keys.entry(tag.clone()).or_default();
                for _ in 0..input.small::<usize>()? {
                    let variant = input.small()?;
                    track.changes.entry((variant, purpose(input.byte()?)?)).or_default();
                }
                for _ in 0..input.small::<usize>()? {
                    let variant = input.small()?;
                    track.cost_changes.entry((variant, purpose(input.byte()?)?)).or_default();
                }
                let rocket_held = rocket_keys.entry(tag.clone()).or_default();
                for held in rocket_held.iter_mut() {
                    for _ in 0..input.small::<usize>()? {
                        let variant = input.small()?;
                        held.insert((variant, purpose(input.byte()?)?));
                    }
                }
                lens.push(entries_len);
                placed.push(tag);
            }
            let samples = (samples_at, offset - samples_at);
            let rockets_at = offset;
            for (tag, len) in placed.iter().zip(rocket_lens) {
                if len > 0 {
                    frames
                        .get_mut(tag)
                        .expect("frames of every tag listed")
                        .rockets
                        .push(Frame { ordinal, offset, len });
                    offset += len;
                }
            }
            let rockets = (rockets_at, offset - rockets_at);
            let mut blocks = BTreeMap::new();
            for _ in 0..input.small::<usize>()? {
                let class: u32 = input.small()?;
                let refits = match input.byte()? {
                    0 => false,
                    1 => true,
                    _ => return Err(invalid("an unknown flag")),
                };
                let len: u64 = input.small()?;
                blocks.insert((class, refits), (offset, len));
                offset += len;
            }
            for (tag, len) in placed.iter().zip(lens) {
                frames
                    .get_mut(tag)
                    .expect("frames of every tag listed")
                    .entries
                    .push(Frame { ordinal, offset, len });
                offset += len;
            }
            if !input.done() {
                return Err(invalid("a range has trailing bytes"));
            }
            layouts.push(Layout {
                ordinal,
                totals: (totals_at, lengths[1]),
                samples,
                rockets,
                blocks,
                tags: placed,
            });
        }
        let catalogue = VariantCatalogue::with_refusals(versions, &refusals);
        // A design whose final token's class is not every class it was totalled by.
        let moved = assignments
            .iter()
            .filter(|(variant, ids)| {
                let name = class_name(catalogue.token(**variant), **variant);
                ids.iter().any(|id| classes[*id as usize] != name)
            })
            .map(|(variant, _)| *variant)
            .collect();
        // A tag is the scan's once it has samples, as a scan holds the tags it sampled.
        scanned.tags = keys
            .into_iter()
            .filter(|(tag, _)| frames.get(tag).is_some_and(|held| held.count > 0))
            .collect();
        rocket.tags = rocket_keys
            .into_iter()
            .filter(|(tag, _)| frames.get(tag).is_some_and(|held| held.rocket_count > 0))
            .map(|(tag, [units, costs])| {
                let mut history = TagHistory::default();
                for entry in units {
                    history.changes.entry(entry).or_default();
                }
                for entry in costs {
                    history.cost_changes.entry(entry).or_default();
                }
                (tag, history)
            })
            .collect();
        scanned.rocket = rocket;
        scanned.variants = catalogue;
        scanned.class_names = classes;
        Ok(Self {
            scanned,
            frames,
            ranges: layouts,
            last_hour,
            moved,
        })
    }

    /// The tags holding a design some range totalled by another class than its final token's.
    pub(crate) fn moved_tags(&self) -> BTreeSet<&String> {
        if self.moved.is_empty() {
            return BTreeSet::new();
        }
        self.scanned
            .tags
            .iter()
            .filter(|(_, track)| {
                track
                    .changes
                    .keys()
                    .chain(track.cost_changes.keys())
                    .any(|entry| self.moved.contains(&entry.0))
            })
            .map(|(tag, _)| tag)
            .collect()
    }

    /// Every tag's samples and every tag's Rocket output whole: each range's regions read at
    /// once, each tag's frames by one worker.
    pub(crate) fn light(
        &self,
        ranges: &dyn Ranges,
        cancelled: &AtomicBool,
    ) -> io::Result<crate::products::equipment_produced::Histories> {
        let samples = regions(ranges, &self.ranges, |layout| layout.samples, cancelled)?;
        let rockets = regions(ranges, &self.ranges, |layout| layout.rockets, cancelled)?;
        let tags: Vec<(&String, &Frames)> = self
            .scanned
            .tags
            .keys()
            .filter_map(|tag| self.frames.get_key_value(tag))
            .collect();
        let rocket_tags: Vec<(&String, &Frames, &TagHistory)> = self
            .scanned
            .rocket
            .tags
            .iter()
            .filter_map(|(tag, named)| Some((tag, self.frames.get(tag)?, named)))
            .collect();
        let histories = crate::workers::map(&tags, "production history", |(_, held)| {
            sampled(
                held.samples.iter().map(|frame| within(&samples, frame).map(std::borrow::Cow::Borrowed)),
                self.last_hour,
            )
        })?;
        let rocket_histories = crate::workers::map(&rocket_tags, "production history", |(_, held, named)| {
            rocket_of(
                held.rockets.iter().map(|frame| within(&rockets, frame).map(std::borrow::Cow::Borrowed)),
                self.last_hour,
                Some(named),
                true,
            )
        })?;
        Ok(crate::products::equipment_produced::Histories {
            tags: tags.into_iter().map(|(tag, _)| tag.clone()).zip(histories).collect(),
            rockets: rocket_tags
                .into_iter()
                .map(|(tag, _, _)| tag.clone())
                .zip(rocket_histories)
                .collect(),
        })
    }

    /// Every tag's totals a ranking by `plan` sums, from the fewer bytes of the classes it counts
    /// or of its whole totals less the classes it leaves out; and the tags whose classes cannot
    /// stand for their designs, which the ranking reads whole instead.
    pub(crate) fn counted(
        &self,
        ranges: &dyn Ranges,
        plan: &CountPlan<'_>,
        cancelled: &AtomicBool,
    ) -> io::Result<(BTreeMap<String, Counted>, Vec<String>)> {
        let taken = |refits: bool| plan.refits || !refits;
        let counted = |class: u32| plan.counted.get(class as usize).copied().unwrap_or(false);
        let outside = |class: u32| plan.outside.get(class as usize).copied().unwrap_or(false);
        let (mut direct, mut less) = (0u64, 0u64);
        for range in &self.ranges {
            for (&(class, refits), &(_, len)) in &range.blocks {
                if taken(refits) {
                    *if counted(class) { &mut direct } else { &mut less } += len;
                }
            }
        }
        // Whole totals less the classes left out, where those are fewer bytes.
        let whole = less < direct;
        let wanted =
            |class: u32, refits: bool| taken(refits) && (counted(class) != whole || outside(class));
        // Only whole totals with no class left out stand for every design whatever its class:
        // otherwise a design some range totalled by another class than its final one is read whole,
        // even where no block of a class the plan names was written.
        let by_class = !whole
            || (0..self.scanned.class_names.len() as u32).any(|class| !counted(class) || outside(class));
        type List = Vec<(u32, i128, i64)>;
        type Read = (Vec<(u32, [List; 2])>, Vec<((u32, bool), Vec<(u32, List)>)>);
        // Each range's totals and wanted class blocks, each part at once.
        let mut parts: Vec<(usize, Option<(u32, bool)>, (u64, u64))> = Vec::new();
        for (index, range) in self.ranges.iter().enumerate() {
            if whole {
                parts.push((index, None, range.totals));
            }
            for (&(class, refits), &place) in &range.blocks {
                if wanted(class, refits) {
                    parts.push((index, Some((class, refits)), place));
                }
            }
        }
        let read = crate::workers::map(&parts, "production ranking", |&(index, class, place)| -> io::Result<Read> {
            check_cancel(cancelled)?;
            let bytes = part(ranges, self.ranges[index].ordinal, place)?;
            let mut input = In::new(&bytes);
            let mut totals = Vec::new();
            let mut blocks = Vec::new();
            match class {
                None => {
                    while !input.done() {
                        let ordinal: u32 = input.small()?;
                        let mut lists: [List; 4] = Default::default();
                        for list in &mut lists {
                            read_list(&mut input, list)?;
                        }
                        let [units, unit_refits, costs, cost_refits] = lists;
                        totals.push((
                            ordinal,
                            if plan.costs {
                                [costs, cost_refits]
                            } else {
                                [units, unit_refits]
                            },
                        ));
                    }
                }
                Some(class) => {
                    let mut held = Vec::new();
                    for _ in 0..input.small::<usize>()? {
                        let ordinal: u32 = input.small()?;
                        let (mut units, mut costs) = (Vec::new(), Vec::new());
                        read_list(&mut input, &mut units)?;
                        read_list(&mut input, &mut costs)?;
                        held.push((ordinal, if plan.costs { costs } else { units }));
                    }
                    if !input.done() {
                        return Err(invalid("a class block has trailing bytes"));
                    }
                    blocks.push((class, held));
                }
            }
            Ok((totals, blocks))
        })?;
        let moved = if by_class {
            self.moved_tags()
        } else {
            BTreeSet::new()
        };
        // Parts in range order, each range's totals first: each tag's lists in sample order.
        let mut terms: BTreeMap<&str, Terms> = BTreeMap::new();
        for (&(index, _, _), (totals, blocks)) in parts.iter().zip(read) {
            let range = &self.ranges[index];
            let tag = |ordinal: u32| {
                range
                    .tags
                    .get(ordinal as usize)
                    .map(String::as_str)
                    .ok_or_else(|| invalid("totals of no tag"))
            };
            for (ordinal, lists) in totals {
                let held = terms.entry(tag(ordinal)?).or_default();
                for (into, list) in held.totals.iter_mut().zip(lists) {
                    into.extend(list);
                }
            }
            for (class, held) in blocks {
                for (ordinal, list) in held {
                    terms
                        .entry(tag(ordinal)?)
                        .or_default()
                        .classes
                        .entry(class)
                        .or_default()
                        .extend(list);
                }
            }
        }
        let tags: Vec<(&String, usize)> = self
            .scanned
            .tags
            .keys()
            .filter(|tag| !moved.contains(tag))
            .map(|tag| (tag, self.frames.get(tag).map_or(0, |held| held.count)))
            .collect();
        let none = Terms::default();
        let sums = crate::workers::map(&tags, "production ranking", |&(tag, samples)| {
            let terms = terms.get(tag.as_str()).unwrap_or(&none);
            let totals = terms
                .totals
                .iter()
                .zip([false, true])
                .filter(|(_, refits)| whole && taken(*refits))
                .map(|(list, _)| (false, list.as_slice()));
            let classes = terms
                .classes
                .iter()
                .filter(|((class, refits), _)| taken(*refits) && counted(*class) != whole)
                .map(|(_, list)| (whole, list.as_slice()));
            let left_out = terms
                .classes
                .iter()
                .filter(|((class, refits), _)| taken(*refits) && outside(*class))
                .map(|(_, list)| (false, list.as_slice()));
            Ok(Counted {
                sums: signed_sums(totals.chain(classes), samples)?,
                outside: signed_sums(left_out, samples)?,
            })
        })?;
        Ok((
            tags.into_iter().map(|(tag, _)| tag.clone()).zip(sums).collect(),
            moved.into_iter().cloned().collect(),
        ))
    }

    /// `tags`' histories whole: their samples, totals by design and Rocket output, the samples
    /// and Rocket output taken from `light` where it holds them. Every frame is fetched by
    /// whichever worker is free, then each tag's are read in order.
    pub(crate) fn whole(
        &self,
        ranges: &dyn Ranges,
        tags: &[String],
        light: Option<&crate::products::equipment_produced::Histories>,
        cancelled: &AtomicBool,
    ) -> io::Result<crate::products::equipment_produced::Histories> {
        let shown: Vec<(&String, &Frames)> = tags
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|tag| self.frames.get_key_value(tag))
            .collect();
        // Each tag's frames to read: its samples and Rocket output unless taken, its totals.
        let mut wanted: Vec<(usize, u8, Frame)> = Vec::new();
        for (index, (tag, held)) in shown.iter().enumerate() {
            let taken = light.is_some_and(|light| light.tags.contains_key(*tag));
            if !taken {
                wanted.extend(held.samples.iter().map(|frame| (index, 0, *frame)));
            }
            wanted.extend(held.entries.iter().map(|frame| (index, 1, *frame)));
            let rocket_taken = light.is_some_and(|light| light.rockets.contains_key(*tag));
            if !rocket_taken && self.scanned.rocket.tags.contains_key(*tag) {
                wanted.extend(held.rockets.iter().map(|frame| (index, 2, *frame)));
            }
        }
        let fetched = crate::workers::map(&wanted, "production history", |(_, _, frame)| {
            check_cancel(cancelled)?;
            crate::prepared::series::decompress(&ranges.read(frame.ordinal, frame.offset, frame.len)?)
        })?;
        let mut held: Vec<[Vec<&[u8]>; 3]> = (0..shown.len()).map(|_| Default::default()).collect();
        for ((index, kind, _), bytes) in wanted.iter().zip(&fetched) {
            held[*index][usize::from(*kind)].push(bytes.as_slice());
        }
        let work: Vec<(&String, [Vec<&[u8]>; 3])> =
            shown.iter().map(|(tag, _)| *tag).zip(held).collect();
        let histories = crate::workers::map(&work, "production history", |(tag, [samples, entries, rockets])| {
            let equipment = match self.scanned.tags.get(*tag) {
                Some(named) => {
                    let mut history = match light.and_then(|light| light.tags.get(*tag)) {
                        Some(light) => TagHistory {
                            hours: light.hours.clone(),
                            last_hours: light.last_hours.clone(),
                            starts: light.starts.clone(),
                            identities: light.identities.clone(),
                            presence: light.presence.clone(),
                            ..Default::default()
                        },
                        None => {
                            let mut track = Track::default();
                            for bytes in samples {
                                let mut input = In::new(bytes);
                                track.samples(&mut input)?;
                                if !input.done() {
                                    return Err(invalid("a frame has trailing bytes"));
                                }
                            }
                            track.finish(self.last_hour)
                        }
                    };
                    let count = history.hours.len();
                    for bytes in entries {
                        let mut input = In::new(bytes);
                        read_entries(&mut input, count, &mut history.changes, &mut history.cost_changes)?;
                        if !input.done() {
                            return Err(invalid("a frame has trailing bytes"));
                        }
                    }
                    // Every entry named, as the light history names them, holds a list.
                    for entry in named.changes.keys() {
                        history.changes.entry(*entry).or_default();
                    }
                    for entry in named.cost_changes.keys() {
                        history.cost_changes.entry(*entry).or_default();
                    }
                    Some(history)
                }
                None => None,
            };
            let rocket = match (
                self.scanned.rocket.tags.get(*tag),
                light.and_then(|light| light.rockets.get(*tag)),
            ) {
                (Some(_), Some(taken)) => Some(taken.clone()),
                (Some(named), None) => Some(rocket_of(
                    rockets.iter().map(|bytes| Ok(std::borrow::Cow::Borrowed(*bytes))),
                    self.last_hour,
                    Some(named),
                    false,
                )?),
                (None, _) => None,
            };
            Ok((equipment, rocket))
        })?;
        let mut out = crate::products::equipment_produced::Histories::default();
        for ((tag, _), (equipment, rocket)) in shown.into_iter().zip(histories) {
            if let Some(history) = equipment {
                out.tags.insert(tag.clone(), history);
            }
            if let Some(history) = rocket {
                out.rockets.insert(tag.clone(), history);
            }
        }
        Ok(out)
    }

    /// The scan with `tags` whole.
    #[cfg(test)]
    pub(crate) fn with_whole(
        &self,
        ranges: &dyn Ranges,
        tags: &[String],
        cancelled: &AtomicBool,
    ) -> io::Result<ScannedProduced> {
        let mut scanned = self.scanned.clone();
        let read = self.whole(ranges, tags, None, cancelled)?;
        scanned.tags.extend(read.tags);
        scanned.rocket.tags.extend(read.rockets);
        Ok(scanned)
    }
}

/// The publication a branch's history is read through: the facts' range count and last range.
fn publication(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<(u64, Option<[u64; 3]>)>> {
    let name = section(super::FACTS, branch);
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

/// A branch's history ranges in a prepared store.
pub(crate) fn stored<'a>(store: &'a store::Store, key: &'a store::SourceKey, branch: u64) -> Stored<'a> {
    Stored {
        store,
        key,
        name: section_of(branch),
    }
}

/// The branch's history base as of its publication, kept for the source while the publication
/// is unchanged; none before production published the branch.
pub(crate) fn base(
    source: &crate::Source,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<Arc<Base>>> {
    let Some(published) = publication(store, key, branch)? else {
        return Ok(None);
    };
    crate::prepared::query::observations::scope(source, "production:history", Some(key))
        .get_entry(branch, &(), &published, cancelled, || {
            Base::read(&stored(store, key, branch), published.0, cancelled)
        })
        .map(Some)
}
