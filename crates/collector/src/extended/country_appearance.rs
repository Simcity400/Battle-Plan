//! Current country colour bits, cosmetic tag, name operands, the retained old
//! names and the ruling party's first leader's character, sampled hourly.
//! The reserved-country naming singleton has independent availability and
//! live-sized operand bytes; it never enters the positive country census.
use super::country_naval_movement_inputs as naval;
use super::reserved_visibility;
use super::transport_mission_shared as transport;
use crate::{
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{Memory, ccountry, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{CountryAppearanceRow, ReservedCountryNamingRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::CountryAppearance, TableId::ReservedCountryNaming];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

static ORIGINAL_TAG_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static POSITIVE_ORIGINAL_TAG_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static ORIGINAL_TAG_BUCKET_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static COUNTRY_ARRAY_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static RESERVED_OWNED_STATE_COUNT_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static RESERVED_FACTION_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static RESERVED_LAUNCH_STATE_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static RESERVED_LAUNCH_STATE_REQUIRED: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static RESERVED_LAUNCH_STATE_CAPACITY: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(crate) fn original_tag_data() -> impl serde::Serialize {
    let refused = ORIGINAL_TAG_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let positive = POSITIVE_ORIGINAL_TAG_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let buckets = ORIGINAL_TAG_BUCKET_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let array = COUNTRY_ARRAY_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let owned_count =
        RESERVED_OWNED_STATE_COUNT_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let faction = RESERVED_FACTION_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    let launch_states = RESERVED_LAUNCH_STATE_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    crate::status::status!({
        "refused": refused.saturating_add(positive).saturating_add(buckets).saturating_add(array).saturating_add(owned_count).saturating_add(faction).saturating_add(launch_states),
        "positive_refused": positive,
        "reserved_refused": refused,
        "bucket_refused": buckets,
        "country_array_refused": array,
        "reserved_owned_state_count_refused": owned_count,
        "reserved_faction_refused": faction,
        "reserved_launch_state_refused": launch_states,
        "reserved_launch_states_required": RESERVED_LAUNCH_STATE_REQUIRED.load(std::sync::atomic::Ordering::Relaxed),
        "reserved_launch_states_capacity": RESERVED_LAUNCH_STATE_CAPACITY.load(std::sync::atomic::Ordering::Relaxed),
        "refusals": crate::status::map([
            ("positive country original tag guarded read or arena refusal", positive),
            ("reserved country original tag guarded read or arena refusal", refused),
            ("native original-tag buckets guarded read or arena refusal", buckets),
            ("physical country array guarded read or arena refusal", array),
            ("reserved owned state count guarded read refusal", owned_count),
            ("reserved faction identity guarded read refusal", faction),
            ("reserved ordered launch state roster refusal", launch_states),
        ].into_iter().filter(|(_, count)| *count != 0)),
        "scope": "cumulative independent original-tag, physical-array, reserved count/faction and ordered launch-state refusals; launch arena floors are peaks, not successful publication or measured gameplay cost",
    })
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    subjects: usize,
    naming_bytes: usize,
    original_tag_buckets: usize,
    original_tag_members: usize,
    country_array: usize,
    launch_states: usize,
    reverse_visibility: usize,
    naval: naval::SharedCapacities,
    transport: transport::Capacities,
    convoys: super::transport_convoys::Capacities,
    spotting: super::transport_spotting_context::Capacities,
    raw: engine::raw_country_observations::Capacity,
}
impl Capacities {
    pub(crate) fn raise_for_transport(&mut self, demand: &super::transport_demand::Demand) {
        self.transport.navies = self.transport.navies.max(demand.missions.navies);
        self.transport.ledgers = self.transport.ledgers.max(demand.missions.ledgers);
        self.transport.occurrences = self.transport.occurrences.max(demand.missions.occurrences);
        self.transport.receivers = self.transport.receivers.max(demand.missions.receivers);
        self.transport.access_modes = self
            .transport
            .access_modes
            .max(demand.missions.access_modes);
        self.transport.ships = self.transport.ships.max(demand.missions.ships);
        self.transport.weather_entries = self
            .transport
            .weather_entries
            .max(demand.missions.weather_entries);
        let search = &mut self.transport.search;
        search.regions = search.regions.max(demand.missions.search.regions);
        search.edges = search.edges.max(demand.missions.search.edges);
        search.occurrences = search.occurrences.max(demand.missions.search.occurrences);
        search.names = search.names.max(demand.missions.search.names);
        search.bytes = search.bytes.max(demand.missions.search.bytes);
        self.convoys.countries = self.convoys.countries.max(demand.convoys.countries);
        self.convoys.pool = self.convoys.pool.max(demand.convoys.pool);
        self.convoys.reservations = self.convoys.reservations.max(demand.convoys.reservations);
        self.convoys.variants = self.convoys.variants.max(demand.convoys.variants);
        self.spotting.owners = self.spotting.owners.max(demand.spotting.owners);
        self.spotting.radar_values = self.spotting.radar_values.max(demand.spotting.radar_values);
        self.spotting.intel_values = self.spotting.intel_values.max(demand.spotting.intel_values);
    }
    #[cfg(test)]
    pub(crate) fn with_rows(rows: usize) -> Self {
        Self {
            rows,
            ..Default::default()
        }
    }
    fn raw_capacity(&self) -> engine::raw_country_observations::Capacity {
        let mut raw = self.raw;
        raw.links = raw.links.max(self.rows).max(self.country_array);
        // Countries reads at most two link words and one tag per physical
        // country; CountryLinks reads at most three words per live link.
        // Appearance retains the original scalar, selection and text, then
        // its optional suffix. Leave room for reserved controls and demands.
        // The zero-sized worker placeholder must remain allocation-free.
        let active = raw.links != 0 || self.rows != 0;
        raw.occurrences = raw.occurrences.max(
            self.rows
                .saturating_mul(8)
                .saturating_add(raw.links.saturating_mul(4))
                .saturating_add(usize::from(active) * 24),
        );
        raw.demands = raw.demands.max(raw.links);
        raw.bytes = raw.bytes.max(
            self.naming_bytes
                .saturating_add(self.rows.saturating_mul(16)),
        );
        raw
    }

    pub(crate) fn raise_for_naval_movement(&mut self, demand: naval::SharedCapacities) {
        self.naval.raise(demand);
    }
    /// One row per canonical country slot.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
        crate::country_scale::raise(&mut self.subjects, crate::country_scale::rows(slots, 1));
    }

    pub(crate) fn raise_reserved_naming(&mut self, bytes: usize) {
        crate::country_scale::raise(&mut self.naming_bytes, bytes);
    }

    pub(crate) fn raise_original_tag_lists(&mut self, demand: engine::original_tag_lists::Demand) {
        crate::country_scale::raise(&mut self.original_tag_buckets, demand.buckets);
        crate::country_scale::raise(&mut self.original_tag_members, demand.members);
    }

    pub(crate) fn raise_country_array(&mut self, count: usize) {
        crate::country_scale::raise(&mut self.country_array, count);
    }

    pub(crate) fn raise_reverse_visibility(&mut self, slots: usize) {
        crate::country_scale::raise(
            &mut self.reverse_visibility,
            crate::country_scale::rows(slots, 1),
        );
    }
}

struct Preacquired {
    base: usize,
    state: usize,
    hour: i32,
    ticks: Option<[u64; N]>,
}
pub struct Captured {
    pub(crate) raw: engine::raw_country_observations::Journal,
    preacquired: Option<Preacquired>,
    rows: CaptureRows<
        ccountry::AppearanceRow<
            engine::capture_text::Owned,
            engine::capture_text::Owned,
            engine::capture_text::Owned,
        >,
    >,
    subject_starts: Vec<usize>,
    subjects: Arena<u32>,
    reserved: ccountry::ReservedNamingWalk,
    naming: Arena<u8>,
    original_tag_buckets: Arena<engine::original_tag_lists::Bucket>,
    original_tag_members: Arena<u32>,
    original_tag_bucket_len: Result<usize, &'static str>,
    country_array: Arena<u32>,
    country_array_len: Result<usize, &'static str>,
    naval: naval::Shared,
    transport: transport::Shared,
    pub(crate) convoys: super::transport_convoys::Captured,
    pub(crate) spotting_owners: super::transport_spotting_context::Captured,
    land_theatre_count: Option<u32>,
    launch_states: Arena<u32>,
    launch_state_scratch: Arena<ccountry::launch_sources::Occurrence>,
    launch_state_result: Result<usize, &'static str>,
    reverse_visibility: reserved_visibility::Captured,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            subjects: 0,
            naming_bytes: 0,
            original_tag_buckets: 0,
            original_tag_members: 0,
            country_array: 0,
            launch_states: 0,
            reverse_visibility: 0,
            naval: Default::default(),
            transport: Default::default(),
            convoys: Default::default(),
            spotting: Default::default(),
            raw: Default::default(),
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        let raw = capacity.raw_capacity();
        Self {
            raw: engine::raw_country_observations::Journal::try_new(raw)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| engine::raw_country_observations::Journal::vacant()),
            preacquired: None,
            rows: CaptureRows::new(capacity.rows),
            subject_starts: owned_vec(engine::allocation::default_vec(capacity.rows)),
            subjects: Arena::new(capacity.subjects),
            reserved: ccountry::ReservedNamingWalk {
                row: ccountry::ReservedNamingRow::default(),
                outcome: Outcome::Unavailable {
                    reason: "clock unavailable",
                },
                bytes_len: 0,
                required_bytes: 0,
                launch_states: None,
                visibility: None,
            },
            naming: Arena::new(capacity.naming_bytes),
            original_tag_buckets: Arena::new(capacity.original_tag_buckets),
            original_tag_members: Arena::new(capacity.original_tag_members),
            original_tag_bucket_len: Err("clock unavailable"),
            country_array: Arena::new(capacity.country_array),
            country_array_len: Err("clock unavailable"),
            naval: naval::Shared::new(capacity.naval),
            transport: transport::Shared::new(capacity.transport),
            convoys: super::transport_convoys::Captured::new(
                super::transport_convoys::Capacities {
                    countries: capacity.convoys.countries.max(capacity.rows),
                    ..capacity.convoys
                },
            ),
            spotting_owners: super::transport_spotting_context::Captured::new(
                super::transport_spotting_context::Capacities {
                    owners: capacity
                        .rows
                        .max(capacity.country_array)
                        .max(capacity.spotting.owners),
                    ..capacity.spotting
                },
            ),
            land_theatre_count: None,
            launch_states: Arena::new(capacity.launch_states),
            launch_state_scratch: Arena::new(capacity.launch_states),
            launch_state_result: Err("clock unavailable"),
            reverse_visibility: reserved_visibility::Captured::new(capacity.reverse_visibility),
        }
    }
    pub fn reset(&mut self) {
        self.preacquired = None;
        self.raw.reset();
        self.rows.reset();
        self.subjects.reset();
        self.reserved.outcome = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        self.reserved.launch_states = None;
        self.reserved.visibility = None;
        self.naming.reset();
        self.original_tag_buckets.reset();
        self.original_tag_members.reset();
        self.original_tag_bucket_len = Err("clock unavailable");
        self.country_array.reset();
        self.country_array_len = Err("clock unavailable");
        self.naval.reset();
        self.transport.reset();
        self.convoys.reset();
        self.spotting_owners.reset();
        self.land_theatre_count = None;
        self.launch_states.reset();
        self.launch_state_scratch.reset();
        self.launch_state_result = Err("clock unavailable");
        self.reverse_visibility.reset();
    }

    pub(crate) fn fail(&mut self, reason: &'static str) {
        let required = self.transport.required();
        let convoys_required = self.convoys.required();
        self.reset();
        self.transport.retain_required(required);
        self.transport.fail(reason);
        self.convoys.retain_required(convoys_required);
        self.convoys.fail();
        self.spotting_owners.fail();
    }

    pub(crate) fn install_land_theatre_count(&mut self, count: Option<u32>) {
        self.land_theatre_count = count;
    }

    fn naming_span(&self, name: engine::csubject_relation::Name) -> Option<&[u8]> {
        self.naming
            .first
            .get(..self.naming.len)?
            .get(name.start..name.start.checked_add(name.len)?)
    }

    /// The singleton's required prefix must be projectable before the
    /// restored assembly admits its optional owner-zero theatre suffix.
    pub(crate) fn reserved_land_certificate_ready(&self) -> bool {
        let row = &self.reserved.row;
        matches!(self.reserved.outcome, Outcome::Rows { len: 1 })
            && row.built & !7 == 0
            && [
                Some(row.tag),
                Some(row.name_tag),
                Some(row.cosmetic_tag),
                Some(row.leader_ideology),
                row.group,
                row.autonomy,
            ]
            .into_iter()
            .flatten()
            .all(|name| self.naming_span(name).is_some())
    }
    pub fn allocated_bytes(&self) -> usize {
        self.raw.allocated_bytes()
            + self.rows.allocated_bytes()
            + self
                .rows
                .rows
                .iter()
                .map(|row| {
                    row.cosmetic_tag.allocated_bytes()
                        + row.name_tag.allocated_bytes()
                        + row.leader_ideology.allocated_bytes()
                        + row.graphical_culture.allocated_bytes()
                        + row.graphical_culture_2d.allocated_bytes()
                        + row.old_names.as_ref().map_or(0, |names| {
                            names
                                .iter()
                                .map(engine::capture_text::Owned::allocated_bytes)
                                .sum::<usize>()
                        })
                })
                .sum::<usize>()
            + self.subjects.allocated_bytes()
            + self.subject_starts.capacity() * std::mem::size_of::<usize>()
            + self.naming.allocated_bytes()
            + self.original_tag_buckets.allocated_bytes()
            + self.original_tag_members.allocated_bytes()
            + self.country_array.allocated_bytes()
            + self.naval.allocated_bytes()
            + self.transport.allocated_bytes()
            + self.convoys.allocated_bytes()
            + self.spotting_owners.allocated_bytes()
            + self.launch_states.allocated_bytes()
            + self.launch_state_scratch.allocated_bytes()
            + self.reverse_visibility.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        let required = self.raw.required();
        crate::table_capture::grow_capacity(&mut capacity.raw.occurrences, required.occurrences);
        crate::table_capture::grow_capacity(&mut capacity.raw.links, required.links);
        crate::table_capture::grow_capacity(&mut capacity.raw.demands, required.demands);
        crate::table_capture::grow_capacity(&mut capacity.raw.bytes, required.bytes);
        self.preacquired = None;
        self.rows.grow(&mut capacity.rows);
        crate::table_capture::grow_vec(&mut self.subject_starts, capacity.rows);
        self.subjects.grow(&mut capacity.subjects);
        self.naming.grow(&mut capacity.naming_bytes);
        self.original_tag_buckets
            .grow(&mut capacity.original_tag_buckets);
        self.original_tag_members
            .grow(&mut capacity.original_tag_members);
        self.country_array.grow(&mut capacity.country_array);
        self.naval.grow(&mut capacity.naval);
        self.transport.grow(&mut capacity.transport);
        capacity.convoys.countries = capacity.convoys.countries.max(capacity.rows);
        self.convoys.grow(&mut capacity.convoys);
        self.spotting_owners
            .prepare(super::transport_spotting_context::Capacities {
                owners: capacity
                    .rows
                    .max(capacity.country_array)
                    .max(capacity.spotting.owners),
                ..capacity.spotting
            });
        self.launch_states.grow(&mut capacity.launch_states);
        self.launch_state_scratch.grow(&mut capacity.launch_states);
        self.reverse_visibility
            .grow(&mut capacity.reverse_visibility);
        // Registry and arena hints must prepare the same journal storage as
        // construction, even before an unused frame has observed pressure.
        capacity.raw = capacity.raw_capacity();
        let _ = self
            .raw
            .reserve(capacity.raw)
            .map_err(crate::allocation::io);
    }
    #[cfg(test)]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let hour = self.preacquired.as_ref().map_or(0, |marker| marker.hour);
        self.capture_at(memory, base, state, hour)
    }
    pub(crate) fn capture_at(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
    ) -> [u64; N] {
        if let Some(marker) = &mut self.preacquired {
            if (marker.base, marker.state, marker.hour) == (base, state, hour) {
                return marker.ticks.take().unwrap_or([0; N]);
            }
            self.fail_preacquired(
                base,
                state,
                hour,
                "country naming preacquisition context mismatch",
            );
            return [0; N];
        }
        self.raw.begin(base, state, hour);
        self.capture_original(memory, base, state)
    }
    pub(crate) fn capture_preacquired(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
    ) {
        let ticks = self.capture_original(memory, base, state);
        self.preacquired = Some(Preacquired {
            base,
            state,
            hour,
            ticks: Some(ticks),
        });
    }
    pub(crate) fn fail_preacquired(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        reason: &'static str,
    ) {
        self.convoys.fail();
        self.spotting_owners.fail();
        self.rows.fail(Outcome::Unavailable { reason });
        self.reserved.outcome = Outcome::Unavailable { reason };
        self.raw.refuse(reason);
        self.raw.seal();
        self.preacquired = Some(Preacquired {
            base,
            state,
            hour,
            ticks: Some([0; N]),
        });
    }
    fn capture_original(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.raw.enable_growth();
        self.land_theatre_count = None;
        self.reserved.visibility = None;
        self.reverse_visibility.reset();
        self.naval.reset();
        self.subjects.reset();
        self.naming.reset();
        self.launch_states.reset();
        self.launch_state_scratch.reset();
        self.launch_state_result = Err("reserved launch state control unavailable");
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        match ccountry::dynamic_boundary(memory, base) {
            Some(boundary) => {
                self.rows.outcome = ccountry::walk_appearance_observed_growing(
                    memory,
                    base,
                    state,
                    boundary,
                    &mut self.rows.rows,
                    &mut self.raw,
                );
                if !matches!(self.rows.outcome, Outcome::Unavailable { .. }) {
                    self.capture_subjects(memory);
                }
            }
            None => {
                self.rows.outcome = Outcome::Unavailable {
                    reason: "country tag database unreadable",
                }
            }
        }
        let appearance_ticks = qpc().saturating_sub(begin).max(0) as u64;
        drop(_reads);
        let _reads = crate::read_accounting::table(TABLES[1]);
        let begin = qpc();
        self.raw.table(
            engine::raw_country_observations::Source::AppearanceOriginal,
            self.rows.outcome,
        );
        let spotting = &mut self.spotting_owners;
        self.reserved = ccountry::read_reserved_naming_observed_with_transport_growing(
            memory,
            base,
            state,
            &mut self.naming.first,
            &mut self.raw,
            &mut |reader, original, owned_count| {
                if let Ok(reader) = reader {
                    spotting.reserved_with_owned_count(memory, base, reader, original, owned_count);
                } else {
                    spotting.fail();
                }
            },
        );
        self.spotting_owners.finish();
        self.raw.table(
            engine::raw_country_observations::Source::ReservedTag,
            self.reserved.outcome,
        );
        // This control exists even when mandatory naming failed. Consume its
        // one attempted count and clear its root before leaving the callback.
        if let Some(control) = self.reserved.launch_states.take() {
            let result = control.capture_growing(
                memory,
                base,
                state,
                &mut self.launch_state_scratch.first,
                &mut self.launch_states.first,
            );
            self.launch_states.required = result.required;
            self.launch_state_scratch.required = result.required;
            self.launch_states.len = result.len.unwrap_or(0);
            self.launch_state_result = result.len;
            RESERVED_LAUNCH_STATE_REQUIRED
                .fetch_max(result.required as u64, std::sync::atomic::Ordering::Relaxed);
            RESERVED_LAUNCH_STATE_CAPACITY.fetch_max(
                self.launch_states
                    .first
                    .len()
                    .min(self.launch_state_scratch.first.len()) as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            if result.len.is_err() {
                RESERVED_LAUNCH_STATE_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.naming.len = self.reserved.bytes_len;
        self.naming.required = self.reserved.required_bytes;
        if matches!(self.reserved.outcome, Outcome::Rows { len: 1 })
            && self.reserved.row.original_tag.is_none()
        {
            ORIGINAL_TAG_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if matches!(self.reserved.outcome, Outcome::Rows { len: 1 })
            && self.reserved.row.owned_state_count.is_none()
        {
            RESERVED_OWNED_STATE_COUNT_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if matches!(self.reserved.outcome, Outcome::Rows { len: 1 })
            && self.reserved.row.faction.is_none()
        {
            RESERVED_FACTION_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let reserved_ticks = qpc().saturating_sub(begin).max(0) as u64;
        drop(_reads);
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        ccountry::capture_appearance_original_tags_observed_growing(
            memory,
            state,
            &mut self.rows.rows[..self.rows.outcome.len()],
            &mut self.naming.first,
            &mut self.naming.len,
            &mut self.naming.required,
            &mut self.raw,
        );
        let refused = self.rows.rows[..self.rows.outcome.len()]
            .iter()
            .filter(|row| row.original_tag.is_none())
            .count();
        POSITIVE_ORIGINAL_TAG_REFUSALS
            .fetch_add(refused as u64, std::sync::atomic::Ordering::Relaxed);
        let appearance_ticks =
            appearance_ticks.saturating_add(qpc().saturating_sub(begin).max(0) as u64);
        drop(_reads);
        let _reads = crate::read_accounting::table(TABLES[1]);
        let begin = qpc();
        let buckets = engine::original_tag_lists::capture_growing(
            memory,
            state,
            &mut self.original_tag_buckets.first,
            &mut self.original_tag_members.first,
        );
        self.original_tag_buckets.required = buckets.demand.buckets;
        self.original_tag_members.required = buckets.demand.members;
        self.original_tag_bucket_len = buckets.len;
        self.original_tag_buckets.len = buckets.len.unwrap_or(0);
        self.original_tag_members.len = if buckets.len.is_ok() {
            buckets.demand.members
        } else {
            0
        };
        if buckets.len.is_err() {
            ORIGINAL_TAG_BUCKET_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let array = engine::country_roster::capture_growing(
            memory,
            base,
            state,
            &mut self.country_array.first,
        );
        self.country_array.required = array.required;
        self.country_array_len = array.len;
        self.country_array.len = array.len.unwrap_or(0);
        if array.len.is_err() {
            COUNTRY_ARRAY_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        [
            appearance_ticks,
            reserved_ticks.saturating_add(qpc().saturating_sub(begin).max(0) as u64),
        ]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.reserved.outcome]
    }

    pub(crate) fn transport_missions_mut(&mut self) -> &mut transport::Shared {
        &mut self.transport
    }
    pub(crate) fn take_transport_regions(
        &mut self,
        hour: i32,
    ) -> super::transport_spotting_context::RegionLoan {
        self.transport.take_spotting_regions(hour)
    }
    pub(crate) fn restore_transport_regions(
        &mut self,
        loan: super::transport_spotting_context::RegionLoan,
    ) -> Result<(), &'static str> {
        self.transport.restore_spotting_regions(loan)
    }

    pub(crate) fn naval_movement_mut(&mut self) -> &mut naval::Shared {
        &mut self.naval
    }
    pub(crate) fn naval_movement(&self) -> &naval::Shared {
        &self.naval
    }

    pub(crate) fn take_reserved_visibility(
        &mut self,
    ) -> Option<ccountry::ReservedVisibilityObservation> {
        self.reserved.visibility.take()
    }

    pub(crate) fn reverse_visibility_mut(&mut self) -> &mut reserved_visibility::Captured {
        &mut self.reverse_visibility
    }

    pub(crate) fn reverse_visibility_ticks(&self) -> u64 {
        self.reverse_visibility.ticks()
    }

    pub(crate) fn observe_reverse_visibility(
        &self,
        work: &mut reserved_visibility::Work,
    ) -> std::io::Result<()> {
        self.reverse_visibility.observe(work)
    }

    fn capture_subjects(&mut self, memory: &impl Memory) {
        let result = (|| {
            let initial = self.rows.outcome;
            for row in &self.rows.rows[..initial.len()] {
                self.subjects.required = self
                    .subjects
                    .required
                    .checked_add(row.subjects.count as usize)
                    .ok_or("country subject arena demand overflow")?;
            }
            if !crate::table_capture::grow_vec(&mut self.subjects.first, self.subjects.required)
                || !crate::table_capture::grow_vec(&mut self.subject_starts, initial.len())
            {
                return Err("country subject arena allocation failed");
            }
            let mut len = 0;
            let mut unread_rows = match initial {
                Outcome::Truncated { unread_rows, .. } => unread_rows,
                _ => 0,
            };
            for index in 0..initial.len() {
                let subjects = self.rows.rows[index].subjects;
                let end = self
                    .subjects
                    .len
                    .checked_add(subjects.count as usize)
                    .ok_or("country subject arena demand overflow")?;
                // Native faction previews name the stored roster in its order
                // (1404AC450). Read no payload for countries without subjects.
                let copied = self
                    .subjects
                    .first
                    .get_mut(self.subjects.len..end)
                    .is_some_and(|destination| {
                        engine::cpdx_array::copy_u32s(
                            memory,
                            subjects,
                            destination,
                            "country subject roster payload unreadable",
                        )
                        .is_ok()
                    });
                if !copied {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("country appearance unread row count overflow")?;
                    continue;
                }
                if len != index {
                    self.rows.rows.swap(len, index);
                }
                self.subject_starts[len] = self.subjects.len;
                len += 1;
                self.subjects.len = end;
            }
            self.rows.outcome = if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            };
            Ok(())
        })();
        if let Err(reason) = result {
            self.rows.fail(Outcome::Unavailable { reason });
        }
    }

    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.tables_attack(None, None)
    }
    pub(super) fn tables_with_attack_and_air(
        &self,
        attack: &super::attack_inputs::Map,
        air: &super::air::Captured,
        hour: i32,
    ) -> [TableView; N] {
        self.tables_attack(Some(attack), Some((air, hour)))
    }
    fn tables_attack(
        &self,
        attack: Option<&super::attack_inputs::Map>,
        air: Option<(&super::air::Captured, i32)>,
    ) -> [TableView; N] {
        [
            self.appearance_table(),
            self.reserved_table_attack(attack, air),
        ]
    }

    fn appearance_table(&self) -> TableView {
        let Some(subjects) = self.subjects.first.get(..self.subjects.len) else {
            return unavailable("country subject arena invalid");
        };
        if self.rows.rows[..self.rows.outcome.len()]
            .iter()
            .zip(&self.subject_starts)
            .any(|(row, &start)| {
                start
                    .checked_add(row.subjects.count as usize)
                    .and_then(|end| subjects.get(start..end))
                    .is_none()
            })
        {
            return unavailable("country subject span outside captured arena");
        }
        crate::spooling::table_iter::<tables::CountryAppearance, _>(
            self.outcomes()[0],
            self.rows.rows[..self.rows.outcome.len()]
                .iter()
                .zip(&self.subject_starts),
            |(r, &start)| CountryAppearanceRow {
                country: r.country,
                subjects: replay::rows::Recorded::Value(owned_vec(
                    engine::allocation::collect_vec(
                        subjects[start..start + r.subjects.count as usize]
                            .iter()
                            .map(|&value| value as i32),
                    ),
                )),
                red: r.rgba[0],
                green: r.rgba[1],
                blue: r.rgba[2],
                alpha: r.rgba[3],
                cosmetic_tag: owned_vec(copy_vec(r.cosmetic_tag.as_bytes())),
                name_tag: owned_vec(copy_vec(r.name_tag.as_bytes())),
                leader_ideology: owned_vec(copy_vec(r.leader_ideology.as_bytes())),
                overlord: r.overlord,
                graphical_culture: owned_vec(copy_vec(r.graphical_culture.as_bytes())),
                graphical_culture_2d: owned_vec(copy_vec(r.graphical_culture_2d.as_bytes())),
                leader_character: r.leader_character,
                ui_red: r.ui_rgba[0],
                ui_green: r.ui_rgba[1],
                ui_blue: r.ui_rgba[2],
                ui_alpha: r.ui_rgba[3],
                old_name: r
                    .old_names
                    .as_ref()
                    .map(|names| owned_vec(copy_vec(names[0].as_bytes()))),
                old_adjective: r
                    .old_names
                    .as_ref()
                    .map(|names| owned_vec(copy_vec(names[1].as_bytes()))),
                old_definite_name: r
                    .old_names
                    .as_ref()
                    .map(|names| owned_vec(copy_vec(names[2].as_bytes()))),
                original_tag: replay::rows::Recorded::Value(
                    r.original_tag
                        .and_then(|name| {
                            name.start.checked_add(name.len).and_then(|end| {
                                self.naming
                                    .first
                                    .get(..self.naming.len)?
                                    .get(name.start..end)
                            })
                        })
                        .map(|bytes| owned_vec(copy_vec(bytes))),
                ),
                original_tag_reference: replay::rows::Recorded::Value(Some(
                    r.original_tag_reference,
                )),
                transport_convoys: replay::rows::Recorded::Value(
                    self.convoys.projection(r.country).unwrap_or_else(|error| {
                        crate::allocation::observe(&error);
                        None
                    }),
                ),
            },
        )
    }

    fn reserved_table_attack(
        &self,
        attack: Option<&super::attack_inputs::Map>,
        air: Option<(&super::air::Captured, i32)>,
    ) -> TableView {
        let r = &self.reserved.row;
        if self.naming.first.get(..self.naming.len).is_none() {
            return unavailable("reserved naming arena invalid");
        }
        let span = |name| self.naming_span(name);
        let Some(tag) = span(r.tag) else {
            return unavailable("reserved naming tag span invalid");
        };
        let Some(name_tag) = span(r.name_tag) else {
            return unavailable("reserved naming original tag span invalid");
        };
        let Some(cosmetic_tag) = span(r.cosmetic_tag) else {
            return unavailable("reserved naming cosmetic tag span invalid");
        };
        let Some(leader_ideology) = span(r.leader_ideology) else {
            return unavailable("reserved naming ideology span invalid");
        };
        let group = match r.group {
            Some(name) => match span(name) {
                Some(bytes) => Some(bytes),
                None => return unavailable("reserved naming group span invalid"),
            },
            None => None,
        };
        let autonomy = match r.autonomy {
            Some(name) => match span(name) {
                Some(bytes) => Some(bytes),
                None => return unavailable("reserved naming autonomy span invalid"),
            },
            None => None,
        };
        // Malformed optional spans refuse only their field, never the prefix.
        let original_tag = r.original_tag.and_then(span);
        crate::spooling::table::<tables::ReservedCountryNaming, _>(
            self.reserved.outcome,
            std::slice::from_ref(r),
            |_| ReservedCountryNamingRow {
                attack: replay::rows::Recorded::Value(attack.and_then(|attack| attack.country(0))),
                raw_country_observations: replay::rows::Recorded::Value(self.raw_packet()),
                tag: owned_vec(copy_vec(tag)),
                name_tag: owned_vec(copy_vec(name_tag)),
                cosmetic_tag: owned_vec(copy_vec(cosmetic_tag)),
                leader_ideology: owned_vec(copy_vec(leader_ideology)),
                group: group.map(|bytes| owned_vec(copy_vec(bytes))),
                overlord: r.overlord,
                autonomy: autonomy.map(|bytes| owned_vec(copy_vec(bytes))),
                built: r.built,
                original_tag: replay::rows::Recorded::Value(
                    original_tag.map(|bytes| owned_vec(copy_vec(bytes))),
                ),
                original_tag_reference: replay::rows::Recorded::Value(Some(
                    r.original_tag_reference,
                )),
                original_tag_buckets: replay::rows::Recorded::Value(self.original_tag_lists()),
                country_array: replay::rows::Recorded::Value(
                    self.country_array_len
                        .ok()
                        .and_then(|len| {
                            self.country_array
                                .first
                                .get(..self.country_array.len)?
                                .get(..len)
                        })
                        .map(|values| owned_vec(copy_vec(values))),
                ),
                owned_state_count: replay::rows::Recorded::Value(r.owned_state_count),
                faction: replay::rows::Recorded::Value(r.faction),
                land_theatre_count: replay::rows::Recorded::Value(self.land_theatre_count),
                movement: replay::rows::Recorded::Value(self.naval.projection().unwrap_or_else(
                    |error| {
                        crate::allocation::observe(&error);
                        None
                    },
                )),
                launch_sources: replay::rows::Recorded::Value(
                    self.launch_projection().unwrap_or_else(|error| {
                        crate::allocation::observe(&error);
                        None
                    }),
                ),
                reverse_visibility: replay::rows::Recorded::Value(
                    self.reverse_visibility
                        .projection()
                        .unwrap_or_else(|error| {
                            crate::allocation::observe(&error);
                            None
                        }),
                ),
                transport_mission_inputs: replay::rows::Recorded::Value(
                    self.transport_projection(air).unwrap_or_else(|error| {
                        crate::allocation::observe(&error);
                        None
                    }),
                ),
                country_intel_sample: replay::rows::Recorded::Value(None),
                dynamic_tag_boundary: replay::rows::Recorded::Value(Some(r.dynamic_tag_boundary)),
            },
        )
    }

    /// All worker jobs have joined before this numerical-only assembly. A
    /// valid sibling packet cannot rescue an absent/current-hour refusal.
    fn transport_projection(
        &self,
        air: Option<(&super::air::Captured, i32)>,
    ) -> std::io::Result<Option<replay::rows::TransportMissionInputs>> {
        let Some(mut packet) = self.transport.projection()? else {
            return Ok(None);
        };
        packet.spotting_context = super::transport_spotting_context::merge(
            self.spotting_owners.projection()?,
            packet.spotting_context,
        )
        .filter(|context| context.sample_hour == packet.sample_hour);
        if let Some((air, hour)) = air {
            if packet.sample_hour != hour {
                return Ok(None);
            }
            packet.air = air.transport_air_packet(hour)?;
        }
        Ok(Some(packet))
    }

    fn raw_packet(&self) -> Option<replay::rows::RawCountryObservations> {
        use engine::raw_country_observations::{Mapping, Tag};
        use replay::rows::{
            RawCountryMapping, RawCountryObservation, RawCountryObservations, RawCountryTag,
        };
        if !self.raw.complete() {
            return None;
        }
        let mut entries = owned_vec(engine::allocation::collect_vec(
            self.raw.entries().iter().map(|entry| {
                let mapping = match entry.mapping {
                    Mapping::Unreached => RawCountryMapping::Unreached,
                    Mapping::Refused(_) => RawCountryMapping::Refused,
                    Mapping::Observed {
                        canonical_slot,
                        country_id,
                    } => RawCountryMapping::Observed {
                        canonical_slot,
                        country_id,
                    },
                };
                let tag = match entry.tag {
                    Tag::Unreached => RawCountryTag::Unreached,
                    Tag::Refused(_) => RawCountryTag::Refused,
                    Tag::Bytes(span) => match self.raw.bytes(span) {
                        Some(bytes) => RawCountryTag::Bytes(owned_vec(copy_vec(bytes))),
                        None => RawCountryTag::Refused,
                    },
                };
                RawCountryObservation {
                    raw: entry.raw,
                    mapping,
                    tag,
                }
            }),
        ));
        entries.sort_unstable_by_key(|entry| entry.raw);
        let packet = RawCountryObservations {
            sample_hour: self.raw.sample_hour(),
            entries,
        };
        packet.check().ok()?;
        Some(packet)
    }

    fn launch_projection(&self) -> std::io::Result<Option<replay::rows::ReservedLaunchSources>> {
        if self.naval.launch_sample_hour().is_none() {
            return Ok(None);
        }
        let owned_states = match self.launch_state_result {
            Ok(len) => {
                let values = self
                    .launch_states
                    .first
                    .get(..self.launch_states.len)
                    .and_then(|values| values.get(..len))
                    .ok_or_else(|| std::io::Error::other("reserved launch state arena invalid"))?;
                Some(copy_vec(values)?)
            }
            Err(_) => None,
        };
        self.naval.projection_launch(owned_states)
    }

    fn original_tag_lists(&self) -> Option<Vec<Vec<u32>>> {
        let len = self.original_tag_bucket_len.ok()?;
        let buckets = self
            .original_tag_buckets
            .first
            .get(..self.original_tag_buckets.len)?
            .get(..len)?;
        let members = self
            .original_tag_members
            .first
            .get(..self.original_tag_members.len)?;
        if buckets.iter().any(|bucket| {
            bucket
                .start
                .checked_add(bucket.len)
                .and_then(|end| members.get(bucket.start..end))
                .is_none()
        }) {
            return None;
        }
        Some(owned_vec(engine::allocation::collect_vec(
            buckets.iter().map(|bucket| {
                owned_vec(copy_vec(&members[bucket.start..bucket.start + bucket.len]))
            }),
        )))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "country_appearance/launch_tests.rs"]
mod launch_tests;

#[cfg(test)]
#[path = "country_appearance/bench/tests.rs"]
mod bench;
