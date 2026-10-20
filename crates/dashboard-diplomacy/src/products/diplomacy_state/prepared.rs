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
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

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
    #[serde(with = "country_map")]
    pairs: BTreeMap<DiplomacyCountry, i32>,
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
    raw_tuple_reads: BTreeMap<u32, i32>,
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
    country_keys: Option<CountryKeys>,
    #[serde(skip)]
    links: Option<replay_frame::frame::ViewIdentity>,
    #[serde(skip)]
    pub(super) projected_rows: usize,
}
impl Prepared {
    fn key(&mut self, name: String) -> u64 {
        let next = self.keys.len() as u64;
        *self.keys.entry(name).or_insert(next)
    }
    fn number(&mut self, name: String, hour: i32, value: Option<i128>, starts: bool) {
        let key = self.key(name);
        self.series
            .observe_if_changed(key, hour, value, starts, false);
    }
    fn close_owner(&mut self, country: &DiplomacyCountry, hour: i32) {
        if let Some(owner) = self.owners.remove(country) {
            for target in owner.pairs.keys() {
                self.unindex_pair(country, target);
                self.number(pair_key(country, target), hour, None, false);
            }
            self.number(country_key("o", country), hour, None, false);
            self.number(
                country_key("r", country),
                hour,
                owner.read.map(i128::from),
                false,
            );
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
            if let Some(state) = self.owners.get_mut(&owner) {
                state.pairs.remove(target);
            }
            self.number(pair_key(&owner, target), hour, None, false);
        }
    }
    pub fn break_segment(&mut self) {
        self.segment_first = None;
        self.joins = None;
        self.country_keys = None;
        self.links = None;
    }
    fn close_tuple_reads(&mut self, hour: i32) {
        self.raw_tuple_reads.clear();
        let owners = std::mem::take(&mut self.tuple_reads);
        for owner in owners.keys() {
            self.number(country_key("t", owner), hour, None, false);
        }
    }
    fn end_pact(&mut self, key: u64, bounded: bool) {
        let p = self.pacts.get_mut(&key).expect("pact");
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
    fn update_joins(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> Option<Joins> {
        let countries = frame.complete::<tables::Countries>().ok()?;
        let same = self
            .country_keys
            .as_ref()
            .is_some_and(|keys| keys.same_changes(countries, changes.rows(TableId::Countries)))
            && self
                .links
                .as_ref()
                .is_some_and(|view| view.is(&frame.tables[TableId::CountryLinks.index()]));
        if !same || self.joins.is_none() {
            self.joins = Joins::read(frame);
            self.opinion_view = None;
            self.tuple_view = None;
            self.country_keys = Some(CountryKeys::read(countries));
            self.links = Some(frame.tables[TableId::CountryLinks.index()].identity());
        }
        self.joins.take()
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
        let Some(joins) = self.update_joins(frame, changes) else {
            let old: Vec<_> = self.owners.keys().cloned().collect();
            for country in old {
                self.close_owner(&country, hour);
            }
            self.end_pacts(None, true);
            self.suspend_queue();
            self.close_tuple_reads(hour);
            let old = std::mem::take(&mut self.live_countries);
            for country in old {
                self.number(country_key("i", &country), hour, None, false);
            }
            self.number("opinions:coverage".into(), hour, None, false);
            self.number("tuples:coverage".into(), hour, None, false);
            self.number("queue:coverage".into(), hour, None, false);
            self.number("unresolved".into(), hour, Some(1), false);
            self.break_segment();
            return Ok(());
        };
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
            self.number(country_key("t", country), hour, None, false);
            self.number(country_key("i", country), hour, None, false);
        }
        for country in now
            .difference(&self.live_countries)
            .cloned()
            .collect::<Vec<_>>()
        {
            self.number(country_key("i", &country), hour, Some(1), true);
            self.catalogue.countries.insert(country);
        }
        // A new segment restarts every continuing identity; one that just joined has
        // already started its run this hour, and a second start would leave a one-hour
        // run before it that clips the owner's read evidence to the segment's first hour.
        if !continuous {
            let continuing: Vec<_> = now.intersection(&self.live_countries).cloned().collect();
            for country in &continuing {
                self.number(country_key("i", country), hour, Some(1), true);
            }
        }
        self.live_countries = now;
        self.opinions(frame, format, &joins, check)?;
        self.tuples(frame, format, &joins, check)?;
        self.proposals(frame, format, &joins, check)?;
        let unresolved = joins.unresolved
            + self.orphan_opinions
            + self
                .owners
                .values()
                .map(|owner| owner.unresolved)
                .sum::<usize>();
        self.number("unresolved".into(), hour, Some(unresolved as i128), false);
        self.joins = Some(joins);
        Ok(())
    }
    fn role_at_read(&self, joins: &Joins, raw: u32, read: i32) -> Option<DiplomacyCountry> {
        let country = joins.raw.get(&raw)?;
        let key = self.keys.get(&country_key("i", country))?;
        let run = self.series.state(*key)?.last?;
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
