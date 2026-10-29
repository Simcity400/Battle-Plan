//! Sparse numeric proof and occurrence pages, atomically published with continuation.
use super::{
    evidence::{self, Joins},
    *,
};
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    series::{self, Accumulator, CatalogueWriter},
};
use dashboard_text::metadata::{CountryKeys, check_cancel, slot_read};
use replay_data::{
    format::FormatVersion,
    rows::{RelationTupleRow, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};

pub mod columnar;
mod opinions;
mod proposals;
mod tuples;

pub const FAMILY_NAME: &str = "diplomacy-state";
pub fn section(branch: u64) -> String {
    format!("diplomacy-series-{branch}")
}
pub fn state_section(branch: u64) -> String {
    format!("diplomacy-state-{branch}")
}
pub(super) fn country_key(prefix: &str, country: &DiplomacyCountry) -> String {
    format!(
        "{prefix}:{}",
        serde_json::to_string(country).expect("country JSON")
    )
}
pub(super) fn pair_key(owner: &DiplomacyCountry, target: &DiplomacyCountry) -> String {
    format!(
        "p:{}:{}",
        serde_json::to_string(owner).expect("owner JSON"),
        serde_json::to_string(target).expect("target JSON")
    )
}
/// A country's series, in the order of `SERIES_PREFIX`.
#[derive(Clone, Copy)]
enum Series {
    Owner,
    Read,
    Tuples,
    Identity,
}
const SERIES_PREFIX: [&str; 4] = ["o", "r", "t", "i"];
/// The family's fixed series, in the order of `FIXED_NAME`.
#[derive(Clone, Copy)]
enum Fixed {
    OpinionCoverage,
    TupleCoverage,
    QueueCoverage,
    QueueDuplicates,
    Unresolved,
}
const FIXED_NAME: [&str; 5] = [
    "opinions:coverage",
    "tuples:coverage",
    "queue:coverage",
    "queue:duplicates",
    "unresolved",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Entry {
    pub key: u64,
    pub queue: bool,
    pub roles: Vec<Option<DiplomacyCountry>>,
    pub raw_roles: Vec<u32>,
    pub unresolved_kind: Option<u32>,
}
#[derive(Default, Deserialize, Serialize)]
pub(super) struct Catalogue {
    pub countries: BTreeSet<DiplomacyCountry>,
    pub entries: Vec<Entry>,
    pub kinds: BTreeMap<String, u32>,
    pub last_hour: Option<i32>,
}
#[derive(Default, Deserialize, Serialize)]
struct Owner {
    read: Option<i32>,
    known: bool,
    unresolved: usize,
    #[serde(with = "pair_map")]
    pairs: BTreeMap<DiplomacyCountry, Pair>,
    /// The projected values the pairs were stored from.
    #[serde(skip)]
    projected: Option<Arc<columnar::Values>>,
}
/// A known pair's opinion and its series key; a restored pair looks its key up again.
#[derive(Clone, Copy)]
struct Pair {
    value: i32,
    key: Option<u64>,
}
/// A raw tuple owner's supplying read, and the projected rows observed at it.
#[derive(Deserialize, Serialize)]
#[serde(transparent)]
struct RawRead {
    read: i32,
    #[serde(skip)]
    memo: Option<columnar::TupleMemo>,
}
#[derive(Deserialize, Serialize)]
struct PactLive {
    signature: String,
    raw_owner: u32,
    reply: DiplomacyPact,
}
#[derive(Deserialize, Serialize)]
struct QueueLive {
    signature: String,
    reply: DiplomacyProposal,
}

#[derive(Default, Deserialize, Serialize)]
pub struct Prepared {
    pub(super) keys: BTreeMap<String, u64>,
    pub(super) catalogue: Catalogue,
    pub(super) series: Accumulator,
    #[serde(with = "country_map")]
    owners: BTreeMap<DiplomacyCountry, Owner>,
    #[serde(with = "country_map")]
    pair_owners: BTreeMap<DiplomacyCountry, BTreeSet<DiplomacyCountry>>,
    live_countries: BTreeSet<DiplomacyCountry>,
    #[serde(with = "country_map")]
    tuple_reads: BTreeMap<DiplomacyCountry, i32>,
    raw_tuple_reads: BTreeMap<u32, RawRead>,
    pacts: BTreeMap<u64, PactLive>,
    queue: BTreeMap<u64, QueueLive>,
    pact_keys: BTreeMap<String, u64>,
    pacts_by_owner: BTreeMap<u32, BTreeSet<u64>>,
    orphan_opinions: usize,
    orphan_tuple_owners: BTreeSet<u32>,
    #[serde(skip)]
    tuple_view: Option<replay_frame::frame::ViewIdentity>,
    #[serde(skip)]
    opinion_view: Option<replay_frame::frame::ViewIdentity>,
    queue_keys: BTreeMap<String, u64>,
    dirty: BTreeSet<u64>,
    last_hour: Option<i32>,
    segment_first: Option<i32>,
    queue_read: Option<i32>,
    #[serde(skip)]
    catalogue_writer: CatalogueWriter,
    #[serde(skip)]
    joins: Option<Joins>,
    #[serde(skip)]
    ids: Option<Arc<columnar::Ids>>,
    #[serde(skip)]
    country_keys: Option<CountryKeys>,
    #[serde(skip)]
    links: Option<replay_frame::frame::ViewIdentity>,
    /// Numbered country series; `keys` never loses an entry, so a held key stays right.
    #[serde(skip)]
    country_series: HashMap<DiplomacyCountry, [Option<u64>; 4]>,
    /// Numbered fixed series, by `Fixed`.
    #[serde(skip)]
    fixed: [Option<u64>; 5],
    #[serde(skip)]
    pub(super) projected_rows: usize,
    #[serde(skip)]
    opinion_deadlines: Option<BTreeSet<(i64, DiplomacyCountry)>>,
    #[serde(skip)]
    tuple_deadlines: Option<BTreeSet<(i64, DiplomacyCountry)>>,
    #[serde(skip)]
    raw_tuple_deadlines: Option<BTreeSet<(i64, u32)>>,
    #[serde(skip)]
    owner_unresolved: Option<usize>,
    #[serde(skip)]
    queue_numbered: BTreeMap<u128, (Arc<proposals::Proposal>, u64)>,
}
impl Prepared {
    fn key(&mut self, name: String) -> u64 {
        let next = self.keys.len() as u64;
        *self.keys.entry(name).or_insert(next)
    }
    fn number(&mut self, series: Fixed, hour: i32, value: Option<i128>) {
        let key = match self.fixed[series as usize] {
            Some(key) => key,
            None => {
                let key = self.key(FIXED_NAME[series as usize].to_owned());
                self.fixed[series as usize] = Some(key);
                key
            }
        };
        self.series
            .observe_if_changed(key, hour, value, false, false);
    }
    fn held_country_series(&self, series: Series, country: &DiplomacyCountry) -> Option<u64> {
        self.country_series.get(country)?[series as usize]
    }
    fn hold_country_series(&mut self, series: Series, country: &DiplomacyCountry, key: u64) {
        match self.country_series.get_mut(country) {
            Some(keys) => keys[series as usize] = Some(key),
            None => {
                let mut keys = [None; 4];
                keys[series as usize] = Some(key);
                self.country_series.insert(country.clone(), keys);
            }
        }
    }
    fn country_number(
        &mut self,
        series: Series,
        country: &DiplomacyCountry,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) {
        let key = match self.held_country_series(series, country) {
            Some(key) => key,
            None => {
                let key = self.key(country_key(SERIES_PREFIX[series as usize], country));
                self.hold_country_series(series, country, key);
                key
            }
        };
        self.series
            .observe_if_changed(key, hour, value, starts, false);
    }
    fn pair_number(
        &mut self,
        owner: &DiplomacyCountry,
        target: &DiplomacyCountry,
        held: Option<u64>,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> u64 {
        let key = match held {
            Some(key) => key,
            None => self.key(pair_key(owner, target)),
        };
        self.series
            .observe_if_changed(key, hour, value, starts, false);
        key
    }
    fn close_owner(&mut self, country: &DiplomacyCountry, hour: i32) {
        if let Some(owner) = self.owners.remove(country) {
            if let Some(total) = &mut self.owner_unresolved {
                *total -= owner.unresolved;
            }
            for (target, pair) in &owner.pairs {
                self.unindex_pair(country, target);
                self.pair_number(country, target, pair.key, hour, None, false);
            }
            self.country_number(Series::Owner, country, hour, None, false);
            self.country_number(
                Series::Read,
                country,
                hour,
                owner.read.map(i128::from),
                false,
            );
        }
    }
    fn index_pair(&mut self, owner: &DiplomacyCountry, target: &DiplomacyCountry) {
        match self.pair_owners.get_mut(target) {
            Some(owners) => {
                if !owners.contains(owner) {
                    owners.insert(owner.clone());
                }
            }
            None => {
                self.pair_owners
                    .insert(target.clone(), BTreeSet::from([owner.clone()]));
            }
        }
    }
    fn unindex_pair(&mut self, owner: &DiplomacyCountry, target: &DiplomacyCountry) {
        if let Some(owners) = self.pair_owners.get_mut(target) {
            owners.remove(owner);
            if owners.is_empty() {
                self.pair_owners.remove(target);
            }
        }
    }
    fn close_target(&mut self, target: &DiplomacyCountry, hour: i32) {
        for owner in self.pair_owners.remove(target).unwrap_or_default() {
            let held = self
                .owners
                .get_mut(&owner)
                .and_then(|state| {
                    state.projected = None;
                    state.pairs.remove(target)
                })
                .and_then(|pair| pair.key);
            self.pair_number(&owner, target, held, hour, None, false);
        }
    }
    pub fn break_segment(&mut self) {
        self.segment_first = None;
        self.joins = None;
        self.ids = None;
        self.country_keys = None;
        self.links = None;
    }
    /// An hour whose opinions are not read holds no opinion row, an orphan owner's neither.
    fn close_opinion_reads(&mut self, hour: i32) {
        let old: Vec<_> = self.owners.keys().cloned().collect();
        for country in old {
            self.close_owner(&country, hour);
        }
        self.orphan_opinions = 0;
        self.opinion_view = None;
    }
    fn close_tuple_reads(&mut self, hour: i32) {
        self.raw_tuple_reads.clear();
        let owners = std::mem::take(&mut self.tuple_reads);
        for owner in owners.keys() {
            self.country_number(Series::Tuples, owner, hour, None, false);
        }
    }
    fn end_pact(&mut self, key: u64, bounded: bool) {
        let p = self.pacts.get_mut(&key).expect("pact");
        if let Some(held) = self.raw_tuple_reads.get_mut(&p.raw_owner) {
            held.memo = None;
        }
        p.reply.present_at_end = false;
        p.reply.ended_by = bounded;
        self.pact_keys.remove(&p.signature);
        if let Some(keys) = self.pacts_by_owner.get_mut(&p.raw_owner) {
            keys.remove(&key);
        }
        self.dirty.insert(key);
    }
    fn end_pacts(&mut self, identities: Option<&BTreeSet<DiplomacyCountry>>, bounded: bool) {
        let keys: Vec<_> = self
            .pacts
            .iter()
            .filter(|(_, p)| {
                p.reply.present_at_end
                    && identities.is_none_or(|ids| {
                        p.reply.identities.iter().flatten().any(|c| ids.contains(c))
                    })
            })
            .map(|(&key, _)| key)
            .collect();
        for key in keys {
            self.end_pact(key, bounded);
        }
    }
    fn end_queue_identities(&mut self, identities: &BTreeSet<DiplomacyCountry>) {
        self.queue_numbered.clear();
        for (&key, q) in &mut self.queue {
            if q.reply.present_at_end
                && q.reply
                    .identities
                    .iter()
                    .flatten()
                    .any(|c| identities.contains(c))
            {
                q.reply.present_at_end = false;
                q.reply.gone_by_bound = true;
                self.queue_keys.remove(&q.signature);
                self.dirty.insert(key);
            }
        }
    }
    fn suspend_queue(&mut self) {
        self.queue_numbered.clear();
        for (&key, q) in &mut self.queue {
            if q.reply.present_at_end {
                q.reply.present_at_end = false;
                q.reply.gone_by_bound = true;
                self.dirty.insert(key);
            }
        }
        self.queue_keys.clear();
        self.queue_read = None;
    }
    /// This hour's joins, and whether they were read again rather than kept.
    fn update_joins(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> Option<(Joins, bool)> {
        let countries = frame.complete::<tables::Countries>().ok()?;
        let same = self
            .country_keys
            .as_ref()
            .is_some_and(|keys| keys.same_changes(countries, changes.rows(TableId::Countries)))
            && self
                .links
                .as_ref()
                .is_some_and(|view| view.is(&frame.tables[TableId::CountryLinks.index()]));
        let read = !same || self.joins.is_none();
        if read {
            self.joins = Joins::read(frame);
            self.opinion_view = None;
            self.tuple_view = None;
            self.country_keys = Some(CountryKeys::read(countries));
            self.links = Some(frame.tables[TableId::CountryLinks.index()].identity());
        }
        self.joins.take().map(|joins| (joins, read))
    }
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        format: FormatVersion,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.observe_checked(frame, format, changes, &mut || check_cancel(cancelled))
    }
    pub fn observe_checked(
        &mut self,
        frame: &DecodedFrame,
        format: FormatVersion,
        changes: &FrameChanges,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        let hour = frame.hour;
        let continuous = self.begin_hour(hour)?;
        let Some((joins, read)) = self.update_joins(frame, changes) else {
            self.refuse_joins(hour);
            return Ok(());
        };
        if read {
            self.update_live_countries(&joins, hour, continuous);
        } else if !continuous {
            // Kept joins hold exactly the live countries, so each one continues.
            let live = std::mem::take(&mut self.live_countries);
            for country in &live {
                self.country_number(Series::Identity, country, hour, Some(1), true);
            }
            self.live_countries = live;
        }
        self.opinions(frame, format, &joins, check)?;
        self.tuples(frame, format, &joins, check)?;
        self.proposals(frame, format, &joins, check)?;
        self.publish_unresolved(hour, joins.unresolved);
        self.joins = Some(joins);
        Ok(())
    }
    fn begin_hour(&mut self, hour: i32) -> io::Result<bool> {
        let continuous = self.segment_first.is_some()
            && self.last_hour.and_then(|h| h.checked_add(1)) == Some(hour);
        self.series.advance(hour)?;
        if !continuous {
            self.end_pacts(None, true);
            self.suspend_queue();
            let old: Vec<_> = self.owners.keys().cloned().collect();
            for country in old {
                self.close_owner(&country, hour);
            }
            self.close_tuple_reads(hour);
        }
        self.segment_first.get_or_insert(hour);
        self.last_hour = Some(hour);
        self.catalogue.last_hour = Some(hour);
        Ok(continuous)
    }
    fn refuse_joins(&mut self, hour: i32) {
        let old: Vec<_> = self.owners.keys().cloned().collect();
        for country in old {
            self.close_owner(&country, hour);
        }
        self.end_pacts(None, true);
        self.suspend_queue();
        self.close_tuple_reads(hour);
        let old = std::mem::take(&mut self.live_countries);
        for country in &old {
            self.country_number(Series::Identity, country, hour, None, false);
        }
        self.number(Fixed::OpinionCoverage, hour, None);
        self.number(Fixed::TupleCoverage, hour, None);
        self.number(Fixed::QueueCoverage, hour, None);
        self.number(Fixed::Unresolved, hour, Some(1));
        self.break_segment();
    }
    fn publish_unresolved(&mut self, hour: i32, joins: usize) {
        let owners = *self
            .owner_unresolved
            .get_or_insert_with(|| self.owners.values().map(|owner| owner.unresolved).sum());
        self.number(
            Fixed::Unresolved,
            hour,
            Some((joins + self.orphan_opinions + owners) as i128),
        );
    }
    fn remember_opinion_deadline(&mut self, country: &DiplomacyCountry, read: Option<i32>) {
        if let (Some(deadlines), Some(read)) = (&mut self.opinion_deadlines, read) {
            deadlines.insert((deadline(read), country.clone()));
        }
    }
    fn update_live_countries(&mut self, joins: &Joins, hour: i32, continuous: bool) {
        let now: BTreeSet<_> = joins.countries.values().cloned().collect();
        let removed: BTreeSet<_> = self.live_countries.difference(&now).cloned().collect();
        if !removed.is_empty() {
            self.end_pacts(Some(&removed), true);
            self.end_queue_identities(&removed);
        }
        for country in &removed {
            self.close_owner(country, hour);
            self.close_target(country, hour);
            self.tuple_reads.remove(country);
            self.country_number(Series::Tuples, country, hour, None, false);
            self.country_number(Series::Identity, country, hour, None, false);
        }
        for country in now
            .difference(&self.live_countries)
            .cloned()
            .collect::<Vec<_>>()
        {
            self.country_number(Series::Identity, &country, hour, Some(1), true);
            self.catalogue.countries.insert(country);
        }
        // A new segment restarts every continuing identity; one that just joined has
        // already started its run this hour, and a second start would leave a one-hour
        // run before it that clips the owner's read evidence to the segment's first hour.
        if !continuous {
            let continuing: Vec<_> = now.intersection(&self.live_countries).cloned().collect();
            for country in &continuing {
                self.country_number(Series::Identity, country, hour, Some(1), true);
            }
        }
        self.live_countries = now;
    }
    fn role_at_read(&mut self, joins: &Joins, raw: u32, read: i32) -> Option<DiplomacyCountry> {
        let country = joins.raw.get(&raw)?;
        let key = match self.held_country_series(Series::Identity, country) {
            Some(key) => key,
            None => {
                let key = *self.keys.get(&country_key(
                    SERIES_PREFIX[Series::Identity as usize],
                    country,
                ))?;
                self.hold_country_series(Series::Identity, country, key);
                key
            }
        };
        let run = self.series.state(key)?.last?;
        (run.first_hour <= read).then(|| country.clone())
    }
    pub fn buffered_bytes(&self) -> usize {
        self.series.bytes()
            + self.dirty.len() * std::mem::size_of::<DiplomacyPact>()
            + self
                .pacts
                .values()
                .map(|p| p.signature.capacity() + p.reply.id.capacity() + p.reply.kind.capacity())
                .sum::<usize>()
            + self
                .queue
                .values()
                .map(|q| q.signature.capacity() + q.reply.id.capacity() + q.reply.handle.capacity())
                .sum::<usize>()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        let hour = self.last_hour.unwrap_or(0);
        for &key in &self.dirty {
            check_cancel(cancelled)?;
            if let Some(p) = self.pacts.get(&key) {
                self.series.event(key, hour, &p.reply)?;
            } else if let Some(q) = self.queue.get(&key) {
                self.series.event(key, hour, &q.reply)?;
            }
        }
        self.series.flush(writer, &section(branch), 0, cancelled)?;
        self.catalogue_writer
            .write(writer, FAMILY_NAME, branch, &self.keys, &self.catalogue)?;
        self.dirty.clear();
        self.pacts.retain(|_, p| p.reply.present_at_end);
        self.pacts_by_owner.retain(|_, keys| !keys.is_empty());
        // Ambiguous gap episodes wait for an actual missing-row read.
        self.queue.retain(|_, q| q.reply.gone_by.is_none());
        Ok(())
    }
    pub fn save(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        writer.write_at(
            &state_section(branch),
            0,
            series::compress(&serde_json::to_vec(self)?, 9)?.as_slice(),
        )
    }
    pub fn restore(range: impl io::Read) -> io::Result<Self> {
        let mut range = range;
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut range, &mut bytes)?;
        Ok(serde_json::from_slice(&series::decompress(&bytes)?)?)
    }
}
fn tag(country: &Option<DiplomacyCountry>) -> Option<String> {
    country.as_ref().map(|country| country.tag.clone())
}
fn boolean(raw: u8) -> Option<bool> {
    match raw {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}
fn date(raw: i32) -> Option<i32> {
    Some(raw)
}
fn tuple_signature(
    row: &RelationTupleRow,
    roles: &[Option<DiplomacyCountry>],
    kind: &str,
) -> io::Result<String> {
    Ok(serde_json::to_string(&(
        row.owner_country,
        row.target_country,
        row.type_id,
        row.ordinal,
        row.first_country,
        row.second_country,
        row.start_hour,
        roles,
        kind,
    ))?)
}

/// Pairs as `[target, opinion]` entries; the series keys are not saved.
mod pair_map {
    use super::{DiplomacyCountry, Pair};
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;
    pub fn serialize<S: Serializer>(
        map: &BTreeMap<DiplomacyCountry, Pair>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(map.iter().map(|(target, pair)| (target, pair.value)))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<DiplomacyCountry, Pair>, D::Error> {
        Ok(Vec::<(DiplomacyCountry, i32)>::deserialize(deserializer)?
            .into_iter()
            .map(|(target, value)| (target, Pair { value, key: None }))
            .collect())
    }
}
mod country_map {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;
    pub fn serialize<K: Serialize + Ord, V: Serialize, S: Serializer>(
        map: &BTreeMap<K, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<
        'de,
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    >(
        deserializer: D,
    ) -> Result<BTreeMap<K, V>, D::Error> {
        Ok(Vec::<(K, V)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}

fn deadline(read: i32) -> i64 {
    i64::from(read) + i64::from(replay_data::cadence::STAGGER_PERIOD)
}
fn due<K: Ord>(deadlines: &mut BTreeSet<(i64, K)>, hour: i32) -> Vec<(i64, K)> {
    let mut expired = Vec::new();
    while deadlines
        .first()
        .is_some_and(|(at, _)| *at <= i64::from(hour))
    {
        expired.push(deadlines.pop_first().expect("due read"));
    }
    expired
}
