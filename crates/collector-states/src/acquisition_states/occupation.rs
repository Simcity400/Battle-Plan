use collector_observers::occupation_changes as changes;
use engine_allocation as allocation;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cstate;
use engine_occupation::cresistance;
use replay_data::rows::{self, Recorded, RowCodec};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
struct SharedBytes(allocation::SharedBox<AtomicUsize>);
struct Charge {
    total: SharedBytes,
    bytes: usize,
}
impl Charge {
    fn new(total: &SharedBytes, bytes: usize) -> Self {
        total.0.fetch_add(bytes, Ordering::Relaxed);
        Self {
            total: total.clone(),
            bytes,
        }
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.total.0.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

trait CertificatesSource {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
    fn quiet(&self) -> Option<(u64, u64)>;
    fn demand(&self, receivers: usize);
    fn countries_match(
        &self,
        _refs: [u32; 2],
        _receivers: [usize; 9],
    ) -> Result<bool, &'static str> {
        Ok(true)
    }
}
struct NativeCertificates<'a> {
    countries: Option<&'a engine_native_intelligence::country_memory::Cache>,
    base: usize,
    state: usize,
}
struct CacheOnly;
impl Memory for CacheOnly {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        false
    }
}
impl CertificatesSource for NativeCertificates<'_> {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        changes::sequence(receiver)
    }
    fn quiet(&self) -> Option<(u64, u64)> {
        let cache = self.countries?;
        if cache.view(&CacheOnly).seed_status() != engine_native_intelligence::country_memory::SeedStatus::Ready
        {
            return None;
        }
        changes::quiet()
    }
    fn countries_match(&self, refs: [u32; 2], receivers: [usize; 9]) -> Result<bool, &'static str> {
        if receivers[1] == 0 && receivers[2] == 0 {
            return Ok(true);
        }
        let cache = self
            .countries
            .ok_or("occupation country snapshot unavailable")?;
        let memory = cache.view(&CacheOnly);
        let countries = engine_native::ccountry::Reader::new(&memory, self.base, self.state)?;
        for (raw, expected) in refs.into_iter().zip([receivers[1], receivers[2]]) {
            let country = if raw == 0 || countries.link(raw)?.is_none() {
                countries.reserved()?
            } else {
                countries.resolve(raw)?
            };
            if country.pointer != expected {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn demand(&self, receivers: usize) {
        changes::demand(receivers);
    }
}
type Certificates = [(u64, u64); 9];
#[derive(Default)]
struct Entry {
    source: cstate::Source,
    context: (usize, usize),
    receivers: [usize; 9],
    country_refs: [u32; 2],
    certificates: Certificates,
    row: Option<Arc<CapturedRow>>,
}
struct SourceCache {
    entries: HashMap<(usize, u32), Arc<Cached>>,
    bytes: SharedBytes,
}
#[cfg(any(test, feature = "test-support"))]
impl Default for SourceCache {
    fn default() -> Self {
        Self::new().unwrap()
    }
}
struct Cached {
    receiver: usize,
    certificate: (u64, u64),
    total: Option<i64>,
    nodes: Vec<game_model::modifier_breakdown::Node>,
    failure: Vec<u8>,
    _charge: Charge,
}
impl Cached {
    fn shared(
        receiver: usize,
        certificate: (u64, u64),
        total: Option<i64>,
        nodes: Vec<game_model::modifier_breakdown::Node>,
        failure: Vec<u8>,
        bytes: &SharedBytes,
    ) -> Arc<Self> {
        let payload = size_of::<Self>()
            + 2 * size_of::<usize>()
            + nodes.capacity() * size_of::<game_model::modifier_breakdown::Node>()
            + nodes.iter().map(|node| node.name.capacity()).sum::<usize>()
            + failure.capacity();
        Arc::new(Self {
            receiver,
            certificate,
            total,
            nodes,
            failure,
            _charge: Charge::new(bytes, payload),
        })
    }
}
impl SourceCache {
    fn new() -> std::io::Result<Self> {
        Ok(Self {
            entries: HashMap::new(),
            bytes: SharedBytes(allocation::shared(AtomicUsize::new(0))?),
        })
    }
    fn ensure(
        &mut self,
        memory: &impl Memory,
        base: usize,
        root: usize,
        keys: &[u32],
        certificate: (u64, u64),
        receiver: usize,
    ) -> Result<(), &'static str> {
        if keys.iter().all(|&key| {
            self.entries
                .get(&(root, key))
                .is_some_and(|v| v.certificate == certificate && v.receiver == receiver)
        }) {
            return Ok(());
        }
        let mut missing =
            allocation::vec(keys.len()).map_err(|_| "occupation source cache allocation failed")?;
        let mut printed =
            allocation::vec(keys.len()).map_err(|_| "occupation source cache allocation failed")?;
        for &key in keys {
            if self
                .entries
                .get(&(root, key))
                .is_some_and(|v| v.certificate == certificate && v.receiver == receiver)
            {
                continue;
            }
            let total = if root == 0 {
                Ok(0)
            } else {
                engine_native_parts::modifier_accumulator::requested_effective_at(memory, base, root, key)
            };
            if total.is_ok_and(|value| value != 0) && tree_demand(key) {
                printed.push(key);
            }
            missing.push((key, total));
        }
        let trees = engine_native_politics::modifier_breakdown::printed_keys(memory, base, root, &printed);
        let (mut trees, shared_failure) = match trees {
            Ok(trees) => (trees.into_iter(), None),
            Err(reason) => (Vec::new().into_iter(), Some(reason)),
        };
        let new_keys = missing
            .iter()
            .filter(|(key, _)| !self.entries.contains_key(&(root, *key)))
            .count();
        self.entries
            .try_reserve(new_keys)
            .map_err(|_| "occupation source cache allocation failed")?;
        for (key, total) in missing {
            let tree = match total {
                Err(reason) => Err(reason),
                Ok(value) if value != 0 && tree_demand(key) => match shared_failure {
                    Some(reason) => Err(reason),
                    None => trees
                        .next()
                        .ok_or("occupation modifier tree result missing")?
                        .and_then(|nodes| {
                            replay_data::rows::printed_tree(&nodes)
                                .map_err(|_| "occupation modifier tree framing invalid")
                        }),
                },
                Ok(_) => Ok(Vec::new()),
            };
            let (nodes, failure) = match tree {
                Ok(nodes) => (nodes, Vec::new()),
                Err(reason) => (
                    Vec::new(),
                    allocation::copy_vec(reason.as_bytes())
                        .map_err(|_| "occupation source allocation failed")?,
                ),
            };
            self.entries.insert(
                (root, key),
                Cached::shared(
                    receiver,
                    certificate,
                    total.ok(),
                    nodes,
                    failure,
                    &self.bytes,
                ),
            );
        }
        Ok(())
    }
}
fn tree_demand(key: u32) -> bool {
    key != cresistance::hover::NO_COMPLIANCE_GAIN
}
fn source_receivers(receivers: [usize; 9]) -> [usize; 4] {
    [receivers[1], receivers[5], receivers[4], receivers[2]]
}
fn clone_nodes(
    nodes: &[game_model::modifier_breakdown::Node],
) -> Result<Vec<game_model::modifier_breakdown::Node>, &'static str> {
    let mut copy =
        allocation::vec(nodes.len()).map_err(|_| "occupation source allocation failed")?;
    for node in nodes {
        copy.push(game_model::modifier_breakdown::Node {
            children: node.children,
            value: node.value,
            name: allocation::copy_vec(&node.name)
                .map_err(|_| "occupation source allocation failed")?,
        });
    }
    Ok(copy)
}
fn certificates(
    observers: &impl CertificatesSource,
    receivers: [usize; 9],
) -> Result<Certificates, &'static str> {
    let mut tokens = [(0, 0); 9];
    for (receiver, token) in receivers.into_iter().zip(&mut tokens) {
        if receiver != 0 {
            *token = observers
                .sequence(receiver)
                .ok_or("occupation writer certificate unavailable")?;
        }
    }
    Ok(tokens)
}

pub struct Store {
    entries: Vec<Entry>,
    used: usize,
    outcome: Outcome,
    sources: SourceCache,
    quiet: Option<(u64, u64)>,
    allocated: usize,
    invalidated: bool,
    generation: u64,
}
#[cfg(any(test, feature = "test-support"))]
impl Default for Store {
    fn default() -> Self {
        Self::new().unwrap()
    }
}
impl Store {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            entries: Vec::new(),
            used: 0,
            outcome: Outcome::Unavailable {
                reason: "clock unavailable",
            },
            sources: SourceCache::new()?,
            quiet: None,
            allocated: 0,
            invalidated: false,
            generation: collector_projection::projection::generation(),
        })
    }
    pub(super) fn reset(&mut self) {
        self.used = 0;
        self.outcome = Outcome::Unavailable {
            reason: "clock unavailable",
        };
    }
    pub fn bytes(&self) -> usize {
        self.allocated + self.sources.bytes.0.load(Ordering::Relaxed)
    }
    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.sources.entries.capacity() * size_of::<((usize, u32), Arc<Cached>)>()
            + size_of::<AtomicUsize>()
            + 2 * size_of::<usize>()
    }
    pub(super) fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: &[cstate::Source],
        outcome: Outcome,
        countries: Option<&engine_native_intelligence::country_memory::Cache>,
    ) {
        self.acquire_observed(
            memory,
            base,
            state,
            sources,
            outcome,
            &NativeCertificates {
                countries,
                base,
                state,
            },
        );
    }
    fn acquire_observed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: &[cstate::Source],
        outcome: Outcome,
        observers: &impl CertificatesSource,
    ) {
        let published = (self.used, self.outcome);
        self.reset();
        if let Outcome::Unavailable { reason } = outcome {
            self.outcome = Outcome::Unavailable { reason };
            self.generation = collector_projection::projection::generation();
            return;
        }
        observers.demand(sources.len().saturating_mul(9));
        let mut changed = self.invalidated;
        if self.invalidated {
            self.entries.clear();
            self.sources.entries.clear();
            self.invalidated = false;
        }
        let result = (|| {
            let before = observers
                .quiet()
                .ok_or("occupations could not be followed in the game")?;
            if self.quiet.is_some_and(|old| old.0 != before.0) {
                changed = true;
                self.sources.entries.clear();
            }
            if self.quiet == Some(before)
                && self.entries.len() == sources.len()
                && sources.iter().zip(&self.entries).all(|(source, entry)| {
                    *source == entry.source
                        && entry.context == (base, state)
                        && entry.row.is_some()
                        && observers
                            .countries_match(entry.country_refs, entry.receivers)
                            .unwrap_or(false)
                })
            {
                if observers.quiet() != Some(before) {
                    return Err("occupation writer overlapped capture");
                }
                self.used = sources.len();
                return Ok(());
            }
            let previous = (self.entries.len(), self.entries.capacity());
            let additional = sources.len().saturating_sub(self.entries.len());
            allocation::reserve(&mut self.entries, additional)
                .map_err(|_| "occupation hover allocation failed")?;
            self.entries.resize_with(sources.len(), Entry::default);
            changed |= previous != (self.entries.len(), self.entries.capacity());
            for (&source, entry) in sources.iter().zip(&mut self.entries) {
                let held = entry.source == source
                    && entry.context == (base, state)
                    && entry.row.is_some()
                    && observers.countries_match(entry.country_refs, entry.receivers)?
                    && certificates(observers, entry.receivers)
                        .is_ok_and(|tokens| tokens == entry.certificates);
                if !held {
                    changed = true;
                    observers
                        .sequence(source.pointer)
                        .ok_or("occupation writer certificate unavailable")?;
                    let start = observers
                        .quiet()
                        .ok_or("occupations could not be followed in the game")?;
                    let (row, receivers, certificates, country_refs) =
                        capture(memory, base, state, source, &mut self.sources, observers)?;
                    if !observers.countries_match(country_refs, receivers)? {
                        return Err("occupation country alias snapshot changed");
                    }
                    if observers.quiet() != Some(start) {
                        return Err("occupation writer overlapped capture");
                    }
                    entry.source = source;
                    entry.context = (base, state);
                    entry.receivers = receivers;
                    entry.country_refs = country_refs;
                    entry.certificates = certificates;
                    entry.row = Some(Arc::new(row));
                }
                self.used += 1;
            }
            if observers.quiet() != Some(before) {
                return Err("occupation writer overlapped capture");
            }
            self.quiet = Some(before);
            Ok(())
        })();
        self.outcome = match result {
            Ok(()) => outcome,
            Err(reason) => {
                self.used = 0;
                self.quiet = None;
                Outcome::Unavailable { reason }
            }
        };
        if changed || (self.used, self.outcome) != published {
            self.generation = collector_projection::projection::generation();
        }
        if changed {
            self.allocated = self.measure();
        }
    }
    pub fn forget(&mut self) {
        self.quiet = None;
        self.invalidated = true;
    }
}

#[derive(Default)]
pub struct Occupation {
    rows: Vec<Arc<CapturedRow>>,
    outcome: Option<Outcome>,
    generation: Option<u64>,
}
impl Occupation {
    pub(super) fn fail(&mut self, reason: &'static str) {
        self.outcome = Some(Outcome::Unavailable { reason });
        self.generation = None;
    }

    pub(super) fn reset(&mut self) {
        self.rows.clear();
        self.outcome = None;
        self.generation = None;
    }
    pub(super) fn bytes(&self) -> usize {
        self.rows.capacity() * size_of::<Arc<CapturedRow>>()
    }
    pub(super) fn publish(&mut self, store: &Store) {
        self.reset();
        if allocation::reserve(&mut self.rows, store.used).is_err() {
            self.outcome = Some(Outcome::Unavailable {
                reason: "occupation snapshot allocation failed",
            });
            return;
        }
        self.rows.extend(
            store.entries[..store.used]
                .iter()
                .filter_map(|entry| entry.row.as_ref())
                .map(Arc::clone),
        );
        self.outcome = Some(store.outcome);
        self.generation = Some(store.generation);
    }
    pub fn table(&self) -> replay_frame::frame::TableView {
        collector_projection::table_conversion::table_iter::<rows::tables::StateResistance, _>(
            self.outcome(),
            self.rows.iter(),
            |row| row.output(),
        )
    }
    pub fn input(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> replay_frame::frame::TableInput {
        let witness = self
            .generation
            .filter(|_| matches!(self.outcome, Some(Outcome::Rows { .. })))
            .map(collector_projection::projection::Witness::generation);
        collector_projection::projection::held_input::<rows::tables::StateResistance>(
            projection,
            witness,
            |_| self.table().into(),
        )
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome.unwrap_or(Outcome::Unavailable {
            reason: "occupation store unavailable",
        })
    }
}
struct Source {
    section: u8,
    source: u8,
    key: u32,
    data: Arc<Cached>,
}
struct CapturedRow {
    row: rows::StateResistanceRow,
    sources: Vec<Source>,
    _charge: Charge,
}
impl CapturedRow {
    fn new(row: rows::StateResistanceRow, sources: Vec<Source>, bytes: &SharedBytes) -> Self {
        let payload = size_of::<Self>()
            + 2 * size_of::<usize>()
            + row.heap_bytes()
            + sources.capacity() * size_of::<Source>();
        Self {
            row,
            sources,
            _charge: Charge::new(bytes, payload),
        }
    }
    fn output(&self) -> rows::StateResistanceRow {
        let mut row = RowCodec::try_clone(&self.row).unwrap_or_default();
        row.modifier_sources = Recorded::Value(
            allocation::collect_vec(self.sources.iter().map(|source| {
                rows::OccupationModifierSource {
                    section: source.section,
                    source: source.source,
                    key: source.key,
                    total: source.data.total,
                    nodes: clone_nodes(&source.data.nodes).unwrap_or_default(),
                    failure: allocation::copy_vec(&source.data.failure).unwrap_or_default(),
                }
            }))
            .unwrap_or_default(),
        );
        row
    }
}

#[expect(
    clippy::type_complexity,
    reason = "one state capture returns its row, spans, certificates and reads together"
)]
fn capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    source: cstate::Source,
    cache: &mut SourceCache,
    observers: &impl CertificatesSource,
) -> Result<(CapturedRow, [usize; 9], Certificates, [u32; 2]), &'static str> {
    use cresistance::hover as native;
    let input = native::read_inputs(memory, base, state, source)?;
    let mut row = rows::StateResistanceRow {
        state: source.id,
        country: input.resistance.country,
        level: input.resistance.level,
        speed: input.resistance.speed,
        compliance: input.resistance.compliance,
        compliance_speed: input.resistance.compliance_speed,
        active: input.resistance.active,
        ..Default::default()
    };
    let map = cresistance::map_icons::read_inputs(memory, base, state, source, input.resistance)?;
    let mut receivers = [0; 9];
    receivers[..7].copy_from_slice(&input.receivers);
    if map.markers.is_some() {
        receivers[1] = map.receivers[0];
        receivers[2] = map.receivers[1];
        receivers[7] = map.receivers[2];
        receivers[8] = map.receivers[3];
    }
    let certificates = certificates(observers, receivers)?;
    let mut sources = Vec::new();
    if row.active != 0 || row.country as i32 > 0 {
        allocation::reserve(&mut sources, native::KEYS.len() * input.sources.len())
            .map_err(|_| "occupation source allocation failed")?;
        for (source, &root) in input.sources.iter().enumerate() {
            let receiver = source_receivers(receivers)[source];
            let certificate = if receiver == 0 {
                (0, 0)
            } else {
                observers
                    .sequence(receiver)
                    .ok_or("occupation source certificate unavailable")?
            };
            let mut keys = [0; native::KEYS.len()];
            let mut used = 0;
            for &(key, occupied_key) in &native::KEYS {
                let key = if source == 3 { occupied_key } else { key };
                if key != 0 {
                    keys[used] = key;
                    used += 1;
                }
            }
            cache.ensure(memory, base, root, &keys[..used], certificate, receiver)?;
            for (section, &(key, occupied_key)) in native::KEYS.iter().enumerate() {
                let key = if source == 3 { occupied_key } else { key };
                if key == 0 {
                    continue;
                }
                let cached = &cache.entries[&(root, key)];
                sources.push(Source {
                    section: section as u8,
                    source: source as u8,
                    key,
                    data: Arc::clone(cached),
                });
            }
        }
        sources.sort_by_key(|source| (source.section, source.source));
    }
    let mut added =
        allocation::vec(input.added.len()).map_err(|_| "occupation additions allocation failed")?;
    for target in input.added {
        added.push(rows::OccupationAddedTarget {
            value: target.value,
            controller: target.controller,
            occupied: target.occupied,
            localisation: target.localisation,
        });
    }
    let effects = |entries: Vec<(u32, i64)>| -> Result<Vec<rows::OccupationEffect>, &'static str> {
        allocation::collect_vec(
            entries
                .into_iter()
                .map(|(key, value)| rows::OccupationEffect { key, value }),
        )
        .map_err(|_| "occupation effects allocation failed")
    };
    let [resistance_effects, compliance_effects] = input.effects;
    row.hover = Recorded::Value(Some(rows::OccupationHover {
        exile_host: input.exile_host,
        legitimacy: input.legitimacy,
        flow_from: input.flow_from,
        flow: input.flow,
        gain_ratio: input.gain_ratio,
        added,
        resistance_effects: effects(resistance_effects)?,
        compliance_effects: effects(compliance_effects)?,
        resistance_extra: input.effect_extras[0],
        compliance_extra: input.effect_extras[1],
    }));
    let country_refs = [
        if map.markers.is_some() {
            map.controller
        } else {
            input.controller
        },
        row.country,
    ];
    row.map = Recorded::Value(map.markers);
    Ok((
        CapturedRow::new(row, sources, &cache.bytes),
        receivers,
        certificates,
        country_refs,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Observer(Cell<u64>);
    impl CertificatesSource for Observer {
        fn sequence(&self, _: usize) -> Option<(u64, u64)> {
            Some((1, self.0.get()))
        }
        fn quiet(&self) -> Option<(u64, u64)> {
            Some((1, self.0.get()))
        }
        fn demand(&self, _: usize) {}
    }
    fn fixture() -> (Vec<u8>, cstate::Source) {
        let mut bytes = vec![0; 0x1000];
        let source = cstate::Source {
            pointer: 0x200,
            id: 7,
        };
        let object = source.pointer + cstate::RESISTANCE;
        for (at, value) in [
            (source.pointer, cstate::VTABLE_RVA.0 as u64),
            (object, cresistance::VTABLE_RVA.0 as u64),
            (object + cresistance::STATE, source.pointer as u64),
            (object + cresistance::LEVEL, 123_456),
        ] {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[source.pointer + cstate::ID..][..4].copy_from_slice(&source.id.to_le_bytes());
        (bytes, source)
    }
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("a carried occupation row must make no native read")
        }
    }

    struct MarkerObserver(collector_certificates::watch_registry::WatchRegistry);
    impl CertificatesSource for MarkerObserver {
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            self.0.sequence(receiver)
        }
        fn quiet(&self) -> Option<(u64, u64)> {
            self.0.quiet()
        }
        fn demand(&self, _: usize) {}
    }

    const MARKER_STATE: cstate::Source = cstate::Source {
        pointer: 0x6000,
        id: 7,
    };
    const MARKER_HOLDER: usize = 0xc000;
    const MARKER_GROUP: usize = 0xd000;
    const MARKER_RETAINED_COUNTRY: usize = 0x8000;
    const MARKER_BUCKET: usize = 0x11300;
    const CORE_DEFINITIONS: usize = 0x13620;

    fn marker_put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
        bytes[at..at + N].copy_from_slice(&value);
    }

    fn marker_array(bytes: &mut [u8], at: usize, storage: usize, count: u32) {
        marker_put(bytes, at, (storage as u64).to_le_bytes());
        marker_put(bytes, at + 8, count.to_le_bytes());
        marker_put(bytes, at + 12, count.to_le_bytes());
    }

    fn marker_group_present(bytes: &mut [u8], present: bool) {
        use engine_occupation::{
            ccountry_occupation_status as holder, pdx_robin_hood_bucket as bucket,
        };
        marker_put(
            bytes,
            MARKER_HOLDER + holder::OCCUPATION_DATA_TABLE + 8,
            u32::from(present).to_le_bytes(),
        );
        bytes[MARKER_BUCKET + bucket::DISTANCE_PLUS_ONE] = u8::from(present);
    }

    fn marker_fixture() -> Vec<u8> {
        use cresistance::map_icons as map;
        use engine_native::{ccountry, cgame_state};
                use engine_occupation::{
            ccountry_occupation_data as group, ccountry_occupation_status as holder,
            pdx_robin_hood_bucket as bucket,
        };
        let mut bytes = vec![0; 0x14000];
        marker_array(&mut bytes, 0x1000 + cgame_state::COUNTRIES, 0x11000, 3);
        marker_array(&mut bytes, 0x1000 + cgame_state::COUNTRY_LINKS, 0x11100, 3);
        for (slot, country, diplomacy) in [(1_u32, 0x2000, 0x9000), (2, 0x4000, 0xa000)] {
            marker_put(
                &mut bytes,
                0x11000 + slot as usize * 8,
                (country as u64).to_le_bytes(),
            );
            marker_put(&mut bytes, 0x11100 + slot as usize * 4, slot.to_le_bytes());
            marker_put(
                &mut bytes,
                country,
                (ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            marker_put(&mut bytes, country + ccountry::ID, slot.to_le_bytes());
            marker_put(
                &mut bytes,
                country + ccountry::DIPLOMACY_STATUS,
                (diplomacy as u64).to_le_bytes(),
            );
            marker_put(
                &mut bytes,
                diplomacy,
                (engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            marker_put(
                &mut bytes,
                diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
                (country as u64).to_le_bytes(),
            );
        }
        marker_array(
            &mut bytes,
            0x9000 + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
            0x11200,
            3,
        );
        marker_put(
            &mut bytes,
            MARKER_STATE.pointer,
            (cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_STATE.pointer + cstate::ID,
            MARKER_STATE.id.to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_STATE.pointer + cstate::CONTROLLER,
            1_u32.to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_STATE.pointer + cstate::COUNTRY_MODIFIERS,
            0x12000_u64.to_le_bytes(),
        );
        let object = MARKER_STATE.pointer + cstate::RESISTANCE;
        marker_put(
            &mut bytes,
            object,
            (cresistance::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            object + cresistance::STATE,
            (MARKER_STATE.pointer as u64).to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            object + cresistance::COUNTRY,
            2_u32.to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            object + cresistance::LEVEL,
            123_456_i64.to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            object + cresistance::COMPLIANCE,
            234_567_i64.to_le_bytes(),
        );
        marker_put(&mut bytes, object + cresistance::ACTIVE, [0]);
        marker_put(
            &mut bytes,
            0x2000 + ccountry::OCCUPATION_STATUS,
            (MARKER_HOLDER as u64).to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_HOLDER,
            (holder::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_HOLDER + holder::SECONDARY_BASE,
            (holder::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        marker_array(
            &mut bytes,
            MARKER_HOLDER + holder::OCCUPATION_DATA_TABLE,
            MARKER_BUCKET,
            1,
        );
        marker_put(&mut bytes, MARKER_BUCKET + bucket::KEY, 2_u32.to_le_bytes());
        marker_put(
            &mut bytes,
            MARKER_BUCKET + bucket::VALUE,
            (MARKER_GROUP as u64).to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_GROUP,
            (group::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_GROUP + group::PARENT_STATUS,
            (MARKER_HOLDER as u64).to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_GROUP + group::OCCUPIED_COUNTRY,
            2_u32.to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_GROUP + group::OCCUPIED_COUNTRY_POINTER,
            0x4000_u64.to_le_bytes(),
        );
        marker_array(
            &mut bytes,
            object + map::STATE_RESISTANCE_DEFINITIONS,
            0x13600,
            1,
        );
        marker_array(
            &mut bytes,
            MARKER_GROUP + map::CORE_RESISTANCE_DEFINITIONS,
            CORE_DEFINITIONS,
            1,
        );
        marker_put(&mut bytes, 0x13600, 0x13400_u64.to_le_bytes());
        marker_put(&mut bytes, CORE_DEFINITIONS, 0x13500_u64.to_le_bytes());
        for (definition, name) in [(0x13400, b'S'), (0x13500, b'C'), (0x13800, b'D')] {
            marker_put(
                &mut bytes,
                definition,
                (map::DEFINITION_VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            marker_put(&mut bytes, definition + map::DEFINITION_NAME, [name]);
            marker_put(
                &mut bytes,
                definition + map::DEFINITION_NAME + 16,
                1_u64.to_le_bytes(),
            );
            marker_put(
                &mut bytes,
                definition + map::DEFINITION_NAME + 24,
                15_u64.to_le_bytes(),
            );
        }
        marker_group_present(&mut bytes, true);
        bytes
    }

    fn acquire_markers(store: &mut Store, memory: &impl Memory, observer: &MarkerObserver) {
        store.acquire_observed(
            memory,
            0,
            0x1000,
            &[MARKER_STATE],
            Outcome::Rows { len: 1 },
            observer,
        );
    }

    fn retained_country_fixture() -> Vec<u8> {
        use cresistance::map_icons as map;
        use engine_native::ccountry;
        use engine_occupation::ccountry_occupation_data as group;
        let mut bytes = marker_fixture();
        marker_put(
            &mut bytes,
            MARKER_RETAINED_COUNTRY,
            (ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_RETAINED_COUNTRY + ccountry::ID,
            3_u32.to_le_bytes(),
        );
        marker_put(
            &mut bytes,
            MARKER_GROUP + group::OCCUPIED_COUNTRY_POINTER,
            (MARKER_RETAINED_COUNTRY as u64).to_le_bytes(),
        );
        marker_array(
            &mut bytes,
            MARKER_GROUP + map::CORE_RESISTANCE_DEFINITIONS,
            0,
            0,
        );
        marker_array(
            &mut bytes,
            MARKER_GROUP + map::CORE_COMPLIANCE_DEFINITIONS,
            0,
            0,
        );
        bytes
    }

    fn emitted_markers(store: &Store) -> rows::StateResistanceRow {
        assert_eq!(store.outcome, Outcome::Rows { len: 1 });
        assert_eq!(store.used, 1);
        store.entries[0].row.as_ref().unwrap().output()
    }

    fn map_markers(row: &rows::StateResistanceRow) -> &rows::OccupationMap {
        let Recorded::Value(Some(map)) = &row.map else {
            panic!("occupied state must emit known map-marker operands")
        };
        map
    }

    #[test]
    fn occupation_retained_country_id_change_reacquires_empty_group_at_unchanged_inputs() {
        let mut bytes = retained_country_fixture();
        let observer =
            MarkerObserver(collector_certificates::watch_registry::WatchRegistry::enabled(128));
        let mut store = Store::default();
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let first = emitted_markers(&store);
        assert_eq!(map_markers(&first).core_country, Some(3));
        assert_eq!(map_markers(&first).resistance, [b"S".to_vec()]);
        assert!(map_markers(&first).compliance.is_empty());
        let receivers = store.entries[0].receivers;
        assert_eq!(
            [receivers[1], receivers[2], receivers[7]],
            [0x2000, 0x4000, MARKER_HOLDER]
        );
        assert_eq!(receivers[8], MARKER_RETAINED_COUNTRY);
        assert!(!receivers[..8].contains(&MARKER_RETAINED_COUNTRY));
        assert_eq!(store.entries[0].country_refs, [1, 2]);
        let certificates_before = store.entries[0].certificates;
        let quiet_before = observer.quiet().unwrap();
        let held = Arc::clone(store.entries[0].row.as_ref().unwrap());
        collector_test_support::with_allocations_denied(|| {
            acquire_markers(&mut store, &NoReads, &observer)
        });
        assert!(Arc::ptr_eq(store.entries[0].row.as_ref().unwrap(), &held));
        assert_eq!(emitted_markers(&store), first);

        marker_put(
            &mut bytes,
            MARKER_RETAINED_COUNTRY + engine_native::ccountry::ID,
            4_u32.to_le_bytes(),
        );
        drop(observer.0.notify(MARKER_RETAINED_COUNTRY));
        let quiet_after = observer.quiet().unwrap();
        assert_eq!(quiet_after.0, quiet_before.0);
        assert_ne!(quiet_after.1, quiet_before.1);
        let certificates_after = certificates(&observer, receivers).unwrap();
        assert_eq!(certificates_after[..8], certificates_before[..8]);
        assert_ne!(certificates_after[8], certificates_before[8]);

        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let changed = emitted_markers(&store);
        let mut expected = first.clone();
        let Recorded::Value(Some(map)) = &mut expected.map else {
            panic!("matched empty group must retain its saved country key")
        };
        map.core_country = Some(4);
        assert_eq!(changed, expected);
        assert_eq!(store.entries[0].country_refs, [1, 2]);
        assert!(!Arc::ptr_eq(store.entries[0].row.as_ref().unwrap(), &held));
        assert_eq!(held.output(), first);
        collector_test_support::with_allocations_denied(|| {
            acquire_markers(&mut store, &NoReads, &observer)
        });
        assert_eq!(emitted_markers(&store), changed);
    }

    #[test]
    fn occupation_retained_country_in_flight_and_disabled_observation_refuse_publication() {
        let bytes = retained_country_fixture();
        let observer =
            MarkerObserver(collector_certificates::watch_registry::WatchRegistry::enabled(128));
        let mut store = Store::default();
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let first = emitted_markers(&store);
        let mut frame = Occupation::default();
        frame.publish(&store);
        let running = observer.0.notify(MARKER_RETAINED_COUNTRY);
        collector_test_support::with_allocations_denied(|| {
            acquire_markers(&mut store, &NoReads, &observer)
        });
        frame.publish(&store);
        assert_eq!(
            frame.outcome(),
            Outcome::Unavailable {
                reason: "occupations could not be followed in the game"
            }
        );
        assert!(frame.rows.is_empty());
        assert_eq!(
            frame.table(),
            replay_frame::frame::TableView::Unavailable {
                reason: "occupations could not be followed in the game".into()
            }
        );
        drop(running);
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        assert_eq!(emitted_markers(&store), first);

        observer.0.set_enabled(false);
        collector_test_support::with_allocations_denied(|| {
            acquire_markers(&mut store, &NoReads, &observer)
        });
        frame.publish(&store);
        assert_eq!(
            frame.outcome(),
            Outcome::Unavailable {
                reason: "occupations could not be followed in the game"
            }
        );
        assert!(frame.rows.is_empty());
        assert_eq!(
            frame.table(),
            replay_frame::frame::TableView::Unavailable {
                reason: "occupations could not be followed in the game".into()
            }
        );
        observer.0.set_enabled(true);
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        assert_eq!(emitted_markers(&store), first);
    }

    #[test]
    fn occupation_holder_core_identity_change_reacquires_markers_at_unchanged_levels() {
        let mut bytes = marker_fixture();
        let observer =
            MarkerObserver(collector_certificates::watch_registry::WatchRegistry::enabled(128));
        let mut store = Store::default();
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let first = emitted_markers(&store);
        assert_eq!(
            map_markers(&first).resistance,
            [b"S".to_vec(), b"C".to_vec()]
        );
        assert_eq!(map_markers(&first).core_country, Some(2));
        let held = Arc::clone(store.entries[0].row.as_ref().unwrap());
        collector_test_support::with_allocations_denied(|| {
            acquire_markers(&mut store, &NoReads, &observer)
        });
        assert_eq!(emitted_markers(&store), first);

        marker_put(&mut bytes, CORE_DEFINITIONS, 0x13800_u64.to_le_bytes());
        drop(observer.0.notify(MARKER_HOLDER));
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let changed = emitted_markers(&store);
        assert_eq!(
            map_markers(&changed).resistance,
            [b"S".to_vec(), b"D".to_vec()]
        );
        assert_eq!(
            (changed.level, changed.compliance),
            (first.level, first.compliance)
        );
        assert_eq!(held.output(), first);
        collector_test_support::with_allocations_denied(|| {
            acquire_markers(&mut store, &NoReads, &observer)
        });
        assert_eq!(emitted_markers(&store), changed);

        marker_put(
            &mut bytes,
            MARKER_GROUP + cresistance::map_icons::CORE_RESISTANCE_DEFINITIONS + 12,
            0_u32.to_le_bytes(),
        );
        drop(observer.0.notify(MARKER_HOLDER));
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let cleared = emitted_markers(&store);
        assert_eq!(map_markers(&cleared).resistance, [b"S".to_vec()]);
        assert_eq!(map_markers(&cleared).core_country, Some(2));
        assert_eq!(
            (cleared.level, cleared.compliance),
            (first.level, first.compliance)
        );
    }

    #[test]
    fn occupation_holder_observes_group_birth_destruction_and_address_reuse() {
        let mut bytes = marker_fixture();
        marker_group_present(&mut bytes, false);
        let observer =
            MarkerObserver(collector_certificates::watch_registry::WatchRegistry::enabled(128));
        let mut store = Store::default();
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let absent = emitted_markers(&store);
        assert_eq!(map_markers(&absent).resistance, [b"S".to_vec()]);
        assert_eq!(map_markers(&absent).core_country, None);
        assert_eq!(store.entries[0].receivers[8], 0);
        collector_test_support::with_allocations_denied(|| {
            acquire_markers(&mut store, &NoReads, &observer)
        });

        marker_group_present(&mut bytes, true);
        drop(observer.0.notify(MARKER_HOLDER));
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let born = emitted_markers(&store);
        assert_eq!(
            map_markers(&born).resistance,
            [b"S".to_vec(), b"C".to_vec()]
        );
        assert_eq!(map_markers(&born).core_country, Some(2));
        assert_eq!(store.entries[0].receivers[8], store.entries[0].receivers[2]);

        marker_group_present(&mut bytes, false);
        drop(observer.0.notify(MARKER_HOLDER));
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        assert_eq!(emitted_markers(&store), absent);
        assert_eq!(store.entries[0].receivers[8], 0);

        marker_put(&mut bytes, CORE_DEFINITIONS, 0x13800_u64.to_le_bytes());
        marker_group_present(&mut bytes, true);
        drop(observer.0.notify(MARKER_HOLDER));
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let reused = emitted_markers(&store);
        assert_eq!(
            map_markers(&reused).resistance,
            [b"S".to_vec(), b"D".to_vec()]
        );
        assert_eq!(
            (reused.level, reused.compliance),
            (absent.level, absent.compliance)
        );
        collector_test_support::with_allocations_denied(|| {
            acquire_markers(&mut store, &NoReads, &observer)
        });
        assert_eq!(emitted_markers(&store), reused);
    }

    #[test]
    fn occupation_holder_in_flight_and_disabled_observation_refuse_marker_publication() {
        let bytes = marker_fixture();
        let observer =
            MarkerObserver(collector_certificates::watch_registry::WatchRegistry::enabled(128));
        let mut store = Store::default();
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        let first = emitted_markers(&store);
        let mut frame = Occupation::default();
        frame.publish(&store);
        let running = observer.0.notify(MARKER_HOLDER);
        acquire_markers(&mut store, &NoReads, &observer);
        frame.publish(&store);
        assert_eq!(
            frame.outcome(),
            Outcome::Unavailable {
                reason: "occupations could not be followed in the game"
            }
        );
        assert!(frame.rows.is_empty());
        assert_eq!(
            frame.table(),
            replay_frame::frame::TableView::Unavailable {
                reason: "occupations could not be followed in the game".into()
            }
        );
        drop(running);
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        assert_eq!(emitted_markers(&store), first);

        observer.0.set_enabled(false);
        acquire_markers(&mut store, &NoReads, &observer);
        frame.publish(&store);
        assert_eq!(
            frame.outcome(),
            Outcome::Unavailable {
                reason: "occupations could not be followed in the game"
            }
        );
        assert!(frame.rows.is_empty());
        assert_eq!(
            frame.table(),
            replay_frame::frame::TableView::Unavailable {
                reason: "occupations could not be followed in the game".into()
            }
        );
        observer.0.set_enabled(true);
        acquire_markers(
            &mut store,
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &observer,
        );
        assert_eq!(emitted_markers(&store), first);
    }
    #[test]
    fn occupation_failed_states_job_keeps_terminal_reason_without_retiring_shared_rows() {
        let (bytes, source) = fixture();
        let observer = Observer(Cell::new(1));
        let mut store = Store::default();
        store.acquire_observed(
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0,
            &[source],
            Outcome::Rows { len: 1 },
            &observer,
        );
        let mut inputs = super::super::Inputs::default();
        inputs.occupation.publish(&store);
        let row = Arc::clone(&inputs.occupation.rows[0]);
        let handles = Arc::strong_count(&row);
        let bytes = store.bytes();
        for _ in 0..2 {
            collector_test_support::with_allocations_denied(|| {
                inputs.fail("states acquisition failed")
            });
            assert_eq!(
                inputs.occupation.outcome(),
                Outcome::Unavailable {
                    reason: "states acquisition failed",
                }
            );
            assert_eq!(Arc::strong_count(&row), handles);
            assert_eq!(store.bytes(), bytes);
            assert_eq!(
                inputs.occupation.table(),
                replay_frame::frame::TableView::Unavailable {
                    reason: "states acquisition failed".into(),
                }
            );
        }
        inputs.reset();
        inputs.occupation.publish(&store);
        assert_eq!(inputs.occupation.outcome(), Outcome::Rows { len: 1 });
    }

    #[test]
    fn occupation_one_store_carries_a_mutation_through_all_four_rotating_frames() {
        let (mut bytes, source) = fixture();
        let observer = Observer(Cell::new(1));
        let mut store = Store::default();
        let mut frames: [Occupation; 4] = std::array::from_fn(|_| Occupation::default());
        store.acquire_observed(
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0,
            &[source],
            Outcome::Rows { len: 1 },
            &observer,
        );
        for frame in &mut frames {
            frame.publish(&store);
        }
        let old = Arc::clone(&frames[0].rows[0]);
        let at = source.pointer + cstate::RESISTANCE + cresistance::LEVEL;
        bytes[at..at + 8].copy_from_slice(&234_567_i64.to_le_bytes());
        observer.0.set(2);
        store.acquire_observed(
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0,
            &[source],
            Outcome::Rows { len: 1 },
            &observer,
        );
        let current = Arc::clone(store.entries[0].row.as_ref().unwrap());
        collector_test_support::with_allocations_denied(|| {
            for rotation in 0..8 {
                let frame = &mut frames[rotation % 4];
                store.acquire_observed(
                    &NoReads,
                    0,
                    0,
                    &[source],
                    Outcome::Rows { len: 1 },
                    &observer,
                );
                assert_eq!(store.outcome, Outcome::Rows { len: 1 });
                frame.publish(&store);
                assert!(Arc::ptr_eq(&frame.rows[0], &current));
            }
        });
        assert_eq!(old.row.level, 123_456);
        assert_eq!(current.row.level, 234_567);
    }

    #[test]
    fn occupation_generation_renews_whenever_published_rows_or_outcome_may_differ() {
        let (mut bytes, source) = fixture();
        let observer = Observer(Cell::new(1));
        let mut store = Store::default();
        let acquire = |store: &mut Store, bytes: &[u8], outcome| {
            store.acquire_observed(
                &engine_native_memory::memory::SliceMemory { base: 0, bytes },
                0,
                0,
                &[source],
                outcome,
                &observer,
            );
            store.generation
        };
        let rows = Outcome::Rows { len: 1 };
        let first = acquire(&mut store, &bytes, rows);
        assert_eq!(acquire(&mut store, &bytes, rows), first);
        let mut frame = Occupation::default();
        frame.publish(&store);
        assert_eq!(frame.generation, Some(first));
        let unread = acquire(
            &mut store,
            &bytes,
            Outcome::Unavailable { reason: "census" },
        );
        assert_ne!(unread, first);
        let again = acquire(&mut store, &bytes, rows);
        assert_ne!(again, unread);
        assert_eq!(acquire(&mut store, &bytes, rows), again);
        let at = source.pointer + cstate::RESISTANCE + cresistance::LEVEL;
        bytes[at..at + 8].copy_from_slice(&234_567_i64.to_le_bytes());
        observer.0.set(2);
        let written = acquire(&mut store, &bytes, rows);
        assert_ne!(written, again);
        store.forget();
        assert_ne!(acquire(&mut store, &bytes, rows), written);
        frame.publish(&store);
        let table: replay_frame::frame::TableInput = frame.table().into();
        assert_eq!(frame.input(None), table);
        frame.fail("states acquisition failed");
        assert_eq!(frame.generation, None);
        frame.publish(&store);
        frame.reset();
        assert_eq!(frame.generation, None);
    }

    #[test]
    fn occupation_unchanged_store_rows_compare_equal_without_conversion() {
        use collector_projection::projection::{Projection, ProjectionScratch};
        use replay_frame::frame::{FrameEncoder, FrameInput, Metrics, TableInput, TableView};
        let (mut bytes, source) = fixture();
        let observer = Observer(Cell::new(1));
        let mut store = Store::default();
        let frame = |hour, table: TableInput| {
            let mut frame = FrameInput {
                branch: 1,
                hour,
                tables: std::array::from_fn(|_| {
                    TableView::Unavailable {
                        reason: "fixture table".into(),
                    }
                    .into()
                }),
                metrics: Metrics::default(),
            };
            frame.tables[rows::TableId::StateResistance.index()] = table;
            frame
        };
        let mut held = FrameEncoder::new();
        let mut ordinary = FrameEncoder::new();
        let scratch = ProjectionScratch::default();
        let mut occupation = Occupation::default();
        for hour in 0..5 {
            if hour == 2 {
                let at = source.pointer + cstate::RESISTANCE + cresistance::LEVEL;
                bytes[at..at + 8].copy_from_slice(&234_567_i64.to_le_bytes());
                observer.0.set(2);
            }
            store.acquire_observed(
                &engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                0,
                &[source],
                Outcome::Rows { len: 1 },
                &observer,
            );
            occupation.publish(&store);
            let input =
                occupation.input(Some(&mut Projection::new(&held, 1, hour, false, &scratch)));
            assert_eq!(
                matches!(input, TableInput::ComparedEqual(_)),
                hour != 0 && hour != 2,
                "hour {hour}"
            );
            let actual = held.encode_bytes(frame(hour, input), false).unwrap();
            let expected = ordinary
                .encode_bytes(frame(hour, occupation.table().into()), false)
                .unwrap();
            assert_eq!(
                actual.into_spool_record().unwrap(),
                expected.into_spool_record().unwrap(),
                "hour {hour}"
            );
        }
    }

    #[test]
    fn occupation_worker_failure_join_does_not_walk_or_drop_shared_payloads() {
        let (bytes, source) = fixture();
        let observer = Observer(Cell::new(1));
        let mut store = Store::default();
        store.acquire_observed(
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0,
            &[source],
            Outcome::Rows { len: 1 },
            &observer,
        );
        let row = Arc::clone(store.entries[0].row.as_ref().unwrap());
        let bytes_before = store.bytes();
        collector_test_support::with_allocations_denied(|| store.forget());
        assert_eq!(store.entries.len(), 1);
        assert!(Arc::ptr_eq(store.entries[0].row.as_ref().unwrap(), &row));
        assert_eq!(store.bytes(), bytes_before);
        store.acquire_observed(
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0,
            &[source],
            Outcome::Rows { len: 1 },
            &observer,
        );
        assert_eq!(store.outcome, Outcome::Rows { len: 1 });
        assert!(!Arc::ptr_eq(store.entries[0].row.as_ref().unwrap(), &row));
    }

    #[test]
    fn occupation_exile_and_host_certificates_dirty_only_their_rows_and_no_source_tree() {
        struct Local(collector_certificates::watch_registry::WatchRegistry);
        impl CertificatesSource for Local {
            fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
                self.0.sequence(receiver)
            }
            fn quiet(&self) -> Option<(u64, u64)> {
                self.0.quiet()
            }
            fn demand(&self, _: usize) {}
        }
        let observer = Local(collector_certificates::watch_registry::WatchRegistry::enabled(128));
        let exile = [10, 11, 12, 13, 14, 15, 16, 17, 18];
        let hosted = [20, 21, 22, 23, 24, 25, 13, 27, 28];
        let unrelated = [30, 31, 32, 33, 34, 35, 36, 37, 38];
        let rows = [exile, hosted, unrelated];
        let before = rows.map(|receivers| certificates(&observer, receivers).unwrap());
        let source_before = rows.map(|receivers| {
            source_receivers(receivers).map(|receiver| observer.sequence(receiver).unwrap())
        });
        let epoch = observer.quiet().unwrap().0;
        drop(observer.0.notify(exile[3]));
        assert_eq!(observer.quiet().unwrap().0, epoch);
        assert_ne!(certificates(&observer, exile).unwrap(), before[0]);
        assert_ne!(certificates(&observer, hosted).unwrap(), before[1]);
        assert_eq!(certificates(&observer, unrelated).unwrap(), before[2]);
        assert_eq!(
            rows.map(|receivers| source_receivers(receivers)
                .map(|receiver| observer.sequence(receiver).unwrap())),
            source_before
        );
    }

    #[test]
    fn occupation_rows_share_source_versions_and_spool_materializes_owned_output() {
        let mut store = Store::default();
        let cached = Cached::shared(
            91,
            (3, 7),
            Some(123),
            vec![game_model::modifier_breakdown::Node {
                children: 0,
                value: 123,
                name: b"shared country source".to_vec(),
            }],
            vec![],
            &store.sources.bytes,
        );
        store.sources.entries.insert((91, 42), Arc::clone(&cached));
        for state in 1..=100 {
            store.entries.push(Entry {
                row: Some(Arc::new(CapturedRow::new(
                    rows::StateResistanceRow {
                        state,
                        level: state as i64,
                        ..Default::default()
                    },
                    vec![Source {
                        section: 0,
                        source: 0,
                        key: 42,
                        data: Arc::clone(&cached),
                    }],
                    &store.sources.bytes,
                ))),
                ..Default::default()
            });
        }
        store.used = store.entries.len();
        store.outcome = Outcome::Rows { len: store.used };
        let mut frame = Occupation::default();
        frame.publish(&store);
        for row in &frame.rows {
            assert!(Arc::ptr_eq(&row.sources[0].data, &cached));
            assert_eq!(
                row.sources[0].data.nodes[0].name.as_ptr(),
                cached.nodes[0].name.as_ptr()
            );
        }
        store.sources.entries.insert(
            (91, 42),
            Cached::shared(91, (3, 8), Some(456), vec![], vec![], &store.sources.bytes),
        );
        let output = frame.rows[0].output();
        let Recorded::Value(sources) = output.modifier_sources else {
            panic!("owned sources missing")
        };
        assert_eq!(sources[0].total, Some(123));
        assert_eq!(sources[0].nodes[0].name, b"shared country source");
        assert_ne!(
            sources[0].nodes[0].name.as_ptr(),
            cached.nodes[0].name.as_ptr()
        );
        let bytes = store.sources.bytes.0.load(Ordering::Relaxed);
        store.entries.clear();
        drop(cached);
        assert_eq!(store.sources.bytes.0.load(Ordering::Relaxed), bytes);
        drop(frame);
        assert!(store.sources.bytes.0.load(Ordering::Relaxed) < bytes);
    }

    const RATE_ROOT: usize = 0x1000;
    const RATE_LEAF: usize = 0x1410;
    const RATE_LEAF_VALUES: usize = 0x2200;
    const TREE_CHILDREN: usize = 0x18;
    const TREE_DISPLAY: usize = 0x48;
    const CHILD_ACCUMULATOR: usize = 0x10;
    const RATE_KEYS: [u32; 4] = [
        cresistance::hover::DECAY,
        cresistance::hover::GROWTH,
        cresistance::hover::OCCUPIED_DECAY,
        cresistance::hover::OCCUPIED_GROWTH,
    ];
    struct RateSources {
        bytes: Vec<u8>,
        reads: Cell<usize>,
        fault: Option<usize>,
    }
    impl RateSources {
        fn fixture(failed_key: Option<u32>) -> Self {
            let mut bytes = vec![0; 0x2500];
            for (root, data) in [(RATE_ROOT, 0x2000_usize), (RATE_LEAF, RATE_LEAF_VALUES)] {
                bytes[root..root + 8].copy_from_slice(&(data as u64).to_le_bytes());
                bytes[root + 8..root + 12].copy_from_slice(&4_i32.to_le_bytes());
                bytes[root + 12..root + 16].copy_from_slice(&4_i32.to_le_bytes());
                for (index, key) in RATE_KEYS.iter().enumerate() {
                    let entry = data + 16 * index;
                    bytes[entry..entry + 4].copy_from_slice(&key.to_le_bytes());
                    bytes[entry + 8..entry + 16].copy_from_slice(&123_i64.to_le_bytes());
                }
            }
            let children = RATE_ROOT + TREE_CHILDREN;
            bytes[children..children + 8].copy_from_slice(&0x2400_u64.to_le_bytes());
            bytes[children + 8..children + 12].copy_from_slice(&1_u32.to_le_bytes());
            bytes[children + 12..children + 16].copy_from_slice(&1_u32.to_le_bytes());
            bytes[0x2400..0x2408]
                .copy_from_slice(&((RATE_LEAF - CHILD_ACCUMULATOR) as u64).to_le_bytes());
            let display = RATE_LEAF + TREE_DISPLAY;
            let label = b"rate source";
            bytes[display..display + label.len()].copy_from_slice(label);
            bytes[display + 16..display + 24].copy_from_slice(&(label.len() as u64).to_le_bytes());
            bytes[display + 24..display + 32].copy_from_slice(&15_u64.to_le_bytes());
            Self {
                bytes,
                reads: Cell::new(0),
                fault: failed_key.map(|key| {
                    RATE_LEAF_VALUES
                        + RATE_KEYS
                            .iter()
                            .position(|candidate| *candidate == key)
                            .unwrap()
                            * 16
                        + 8
                }),
            }
        }
    }
    impl Memory for RateSources {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if self
                .fault
                .is_some_and(|fault| address < fault + 8 && address + out.len() > fault)
            {
                return false;
            }
            if address == engine_native_parts::modifier_definition::TABLE_RVA.0 {
                out.copy_from_slice(&0x3000_0000_u64.to_le_bytes());
                return true;
            }
            if (0x3000_0000..0x3004_0000).contains(&address) {
                out.fill(0);
                return true;
            }
            engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .copy(address, out)
        }
    }

    #[test]
    fn occupation_both_rate_trees_are_shared_carried_and_emitted_without_a_policy_refusal() {
        use cresistance::hover as native;
        let memory = RateSources::fixture(None);
        let mut cache = SourceCache::default();
        cache
            .ensure(&memory, 0, RATE_ROOT, &RATE_KEYS, (3, 7), 91)
            .unwrap();
        assert!(memory.reads.get() > 0);
        for key in RATE_KEYS {
            let source = &cache.entries[&(RATE_ROOT, key)];
            assert_eq!(source.total, Some(123));
            assert!(source.failure.is_empty());
            assert_eq!(source.nodes.len(), 2);
            assert_eq!(source.nodes[1].name, b"rate source");
        }
        let growth = Arc::clone(&cache.entries[&(RATE_ROOT, native::GROWTH)]);
        let decay = Arc::clone(&cache.entries[&(RATE_ROOT, native::DECAY)]);
        let reverse = [
            native::OCCUPIED_GROWTH,
            native::OCCUPIED_DECAY,
            native::GROWTH,
            native::DECAY,
        ];
        collector_test_support::with_allocations_denied(|| {
            for _ in 0..8 {
                cache
                    .ensure(&NoReads, 0, RATE_ROOT, &reverse, (3, 7), 91)
                    .unwrap();
            }
        });
        assert!(Arc::ptr_eq(
            &growth,
            &cache.entries[&(RATE_ROOT, native::GROWTH)]
        ));
        assert!(Arc::ptr_eq(
            &decay,
            &cache.entries[&(RATE_ROOT, native::DECAY)]
        ));
        let captured = CapturedRow::new(
            rows::StateResistanceRow::default(),
            vec![
                Source {
                    section: 1,
                    source: 0,
                    key: native::GROWTH,
                    data: growth,
                },
                Source {
                    section: 2,
                    source: 0,
                    key: native::DECAY,
                    data: decay,
                },
            ],
            &cache.bytes,
        );
        let Recorded::Value(sources) = captured.output().modifier_sources else {
            panic!("sources missing")
        };
        assert_eq!(sources.len(), 2);
        for source in sources {
            assert!(source.failure.is_empty());
            assert_eq!(source.nodes[1].name, b"rate source");
        }
        assert!(!tree_demand(native::NO_COMPLIANCE_GAIN));
    }

    #[test]
    fn occupation_decay_root_failure_preserves_growth_and_scalar_rows_and_carries_without_reads() {
        use cresistance::hover as native;
        let mut memory = RateSources::fixture(None);
        let decay = RATE_KEYS
            .iter()
            .position(|key| *key == native::DECAY)
            .unwrap();
        memory.fault = Some(0x2000 + decay * 16 + 8);
        let mut cache = SourceCache::default();
        cache
            .ensure(&memory, 0, RATE_ROOT, &RATE_KEYS, (3, 7), 91)
            .unwrap();
        let failed = Arc::clone(&cache.entries[&(RATE_ROOT, native::DECAY)]);
        assert_eq!(failed.total, None);
        assert!(!failed.failure.is_empty());
        assert!(failed.nodes.is_empty());
        let growth = Arc::clone(&cache.entries[&(RATE_ROOT, native::GROWTH)]);
        assert_eq!(growth.total, Some(123));
        assert!(growth.failure.is_empty());
        assert_eq!(growth.nodes[1].name, b"rate source");
        collector_test_support::with_allocations_denied(|| {
            cache
                .ensure(&NoReads, 0, RATE_ROOT, &RATE_KEYS, (3, 7), 91)
                .unwrap();
        });
        let row = rows::StateResistanceRow {
            state: 7,
            level: 100_000,
            compliance: 200_000,
            active: 1,
            ..Default::default()
        };
        let frame = Occupation {
            rows: vec![
                Arc::new(CapturedRow::new(
                    row,
                    vec![
                        Source {
                            section: 1,
                            source: 0,
                            key: native::GROWTH,
                            data: growth,
                        },
                        Source {
                            section: 2,
                            source: 0,
                            key: native::DECAY,
                            data: failed,
                        },
                    ],
                    &cache.bytes,
                )),
                Arc::new(CapturedRow::new(
                    rows::StateResistanceRow {
                        state: 8,
                        level: 300_000,
                        ..Default::default()
                    },
                    vec![],
                    &cache.bytes,
                )),
            ],
            outcome: Some(Outcome::Rows { len: 2 }),
            generation: None,
        };
        let replay_frame::frame::TableView::Complete(table) = frame.table() else {
            panic!("root failure refused the table")
        };
        let rows = table.rows::<rows::tables::StateResistance>().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].state, rows[0].level, rows[0].compliance),
            (7, 100_000, 200_000)
        );
        assert_eq!((rows[1].state, rows[1].level), (8, 300_000));
        let Recorded::Value(sources) = &rows[0].modifier_sources else {
            panic!("sources missing")
        };
        assert_eq!(sources[0].total, Some(123));
        assert!(sources[0].failure.is_empty());
        assert_eq!(sources[1].total, None);
        assert!(!sources[1].failure.is_empty());
    }

    #[test]
    fn occupation_growth_tree_failure_keeps_the_decay_tree_and_its_output_usable() {
        use cresistance::hover as native;
        let mut cache = SourceCache::default();
        cache
            .ensure(
                &RateSources::fixture(Some(native::GROWTH)),
                0,
                RATE_ROOT,
                &RATE_KEYS,
                (3, 7),
                91,
            )
            .unwrap();
        assert!(
            !cache.entries[&(RATE_ROOT, native::GROWTH)]
                .failure
                .is_empty()
        );
        for key in [
            native::DECAY,
            native::OCCUPIED_DECAY,
            native::OCCUPIED_GROWTH,
        ] {
            assert!(cache.entries[&(RATE_ROOT, key)].failure.is_empty());
            assert_eq!(
                cache.entries[&(RATE_ROOT, key)].nodes[1].name,
                b"rate source"
            );
        }
        let captured = CapturedRow::new(
            rows::StateResistanceRow::default(),
            vec![
                Source {
                    section: 1,
                    source: 0,
                    key: native::GROWTH,
                    data: Arc::clone(&cache.entries[&(RATE_ROOT, native::GROWTH)]),
                },
                Source {
                    section: 2,
                    source: 0,
                    key: native::DECAY,
                    data: Arc::clone(&cache.entries[&(RATE_ROOT, native::DECAY)]),
                },
            ],
            &cache.bytes,
        );
        let Recorded::Value(sources) = captured.output().modifier_sources else {
            panic!("sources missing")
        };
        assert!(!sources[0].failure.is_empty());
        assert!(sources[0].nodes.is_empty());
        assert!(sources[1].failure.is_empty());
        assert_eq!(sources[1].nodes[1].name, b"rate source");
    }

    #[test]
    fn occupation_carries_without_native_reads_and_rereads_only_after_a_writer() {
        let (mut bytes, source) = fixture();
        let observers = Observer(Cell::new(1));
        let mut captured = Store::default();
        let read = |captured: &mut Store, bytes: &[u8]| {
            captured.acquire_observed(
                &engine_native_memory::memory::SliceMemory { base: 0, bytes },
                0,
                0,
                &[source],
                Outcome::Rows { len: 1 },
                &observers,
            )
        };
        read(&mut captured, &bytes);
        assert_eq!(captured.entries[0].row.as_ref().unwrap().row.level, 123_456);
        let allocation = captured.entries.as_ptr();
        collector_test_support::with_allocations_denied(|| {
            captured.acquire_observed(
                &NoReads,
                0,
                0,
                &[source],
                Outcome::Rows { len: 1 },
                &observers,
            );
        });
        assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.entries.as_ptr(), allocation);
        let at = source.pointer + cstate::RESISTANCE + cresistance::LEVEL;
        bytes[at..at + 8].copy_from_slice(&234_567_i64.to_le_bytes());
        observers.0.set(2);
        read(&mut captured, &bytes);
        assert_eq!(captured.entries[0].row.as_ref().unwrap().row.level, 234_567);
    }
    #[test]
    fn occupation_refuses_an_overlapping_writer_instead_of_holding_a_torn_row() {
        let (bytes, source) = fixture();
        struct Changed<'a> {
            bytes: &'a [u8],
            observer: &'a Observer,
        }
        impl Memory for Changed<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.observer.0.set(self.observer.0.get() + 1);
                engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, output)
            }
        }
        let observer = Observer(Cell::new(1));
        let mut captured = Store::default();
        captured.acquire_observed(
            &Changed {
                bytes: &bytes,
                observer: &observer,
            },
            0,
            0,
            &[source],
            Outcome::Rows { len: 1 },
            &observer,
        );
        assert_eq!(
            captured.outcome,
            Outcome::Unavailable {
                reason: "occupation writer overlapped capture"
            }
        );
        assert_eq!(captured.used, 0);
        assert!(captured.entries[0].row.is_none());
    }
    #[test]
    fn occupation_carried_rows_refuse_a_writer_during_alias_validation() {
        let (bytes, source) = fixture();
        let observer = Observer(Cell::new(1));
        let mut captured = Store::default();
        captured.acquire_observed(
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0,
            &[source],
            Outcome::Rows { len: 1 },
            &observer,
        );
        struct Overlap<'a>(&'a Observer);
        impl CertificatesSource for Overlap<'_> {
            fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
                self.0.sequence(receiver)
            }
            fn quiet(&self) -> Option<(u64, u64)> {
                self.0.quiet()
            }
            fn demand(&self, _: usize) {}
            fn countries_match(&self, _: [u32; 2], _: [usize; 9]) -> Result<bool, &'static str> {
                self.0.0.set(self.0.0.get() + 1);
                Ok(true)
            }
        }
        captured.acquire_observed(
            &NoReads,
            0,
            0,
            &[source],
            Outcome::Rows { len: 1 },
            &Overlap(&observer),
        );
        assert_eq!(
            captured.outcome,
            Outcome::Unavailable {
                reason: "occupation writer overlapped capture"
            }
        );
        assert_eq!(captured.used, 0);
    }

    #[test]
    fn occupation_source_cache_supports_demanded_countries_without_hourly_allocation() {
        let mut cache = SourceCache::default();
        for country in 1..=40 {
            cache.entries.insert(
                (country * 0x1000, 42),
                Cached::shared(
                    country * 0x1000,
                    (3, 7),
                    Some(country as i64),
                    Vec::new(),
                    Vec::new(),
                    &cache.bytes,
                ),
            );
        }
        collector_test_support::with_allocations_denied(|| {
            for country in 1..=40 {
                cache
                    .ensure(
                        &NoReads,
                        0,
                        country * 0x1000,
                        &[42],
                        (3, 7),
                        country * 0x1000,
                    )
                    .unwrap();
            }
        });
        assert_eq!(cache.entries.len(), 40);
    }
}
