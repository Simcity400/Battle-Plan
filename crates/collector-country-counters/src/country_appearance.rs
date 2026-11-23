use super::country_naval_movement_inputs as naval;
use super::reserved_visibility;
use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
    table_conversion::unavailable,
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::ccountry;
use replay_data::{
    rows::CountryAppearanceRow, rows::ReservedCountryNamingRow, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;

mod store;
pub use store::{AppearanceStore, Carry};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::CountryAppearance, TableId::ReservedCountryNaming];

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

pub fn original_tag_data() -> impl serde::Serialize {
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
    launch: naval::LaunchCapacities,
    raw: engine_native_memory::raw_country_observations::Capacity,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    subjects,
    naming_bytes,
    original_tag_buckets,
    original_tag_members,
    country_array,
    launch_states,
    reverse_visibility,
    launch,
    raw { occurrences, links, bytes },
});
impl Capacities {
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_rows(rows: usize) -> Self {
        Self {
            rows,
            ..Default::default()
        }
    }
    fn raw_capacity(&self) -> engine_native_memory::raw_country_observations::Capacity {
        let mut raw = self.raw;
        raw.links = raw.links.max(self.rows).max(self.country_array);
        let active = raw.links != 0 || self.rows != 0;
        raw.occurrences = raw.occurrences.max(
            self.rows
                .saturating_mul(8)
                .saturating_add(raw.links.saturating_mul(4))
                .saturating_add(usize::from(active) * 24),
        );
        raw.bytes = raw.bytes.max(
            self.naming_bytes
                .saturating_add(self.rows.saturating_mul(16)),
        );
        raw
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
        crate::country_scale::raise(&mut self.subjects, crate::country_scale::rows(slots, 1));
    }

    pub fn raise_reserved_naming(&mut self, bytes: usize) {
        crate::country_scale::raise(&mut self.naming_bytes, bytes);
    }

    pub fn raise_original_tag_lists(
        &mut self,
        demand: engine_stability::original_tag_lists::Demand,
    ) {
        crate::country_scale::raise(&mut self.original_tag_buckets, demand.buckets);
        crate::country_scale::raise(&mut self.original_tag_members, demand.members);
    }

    pub fn raise_country_array(&mut self, count: usize) {
        crate::country_scale::raise(&mut self.country_array, count);
    }

    pub fn raise_reverse_visibility(&mut self, slots: usize) {
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
    pub raw: engine_native_memory::raw_country_observations::Journal,
    preacquired: Option<Preacquired>,
    rows: CaptureRows<
        ccountry::AppearanceRow<
            engine_native_memory::capture_text::Owned,
            engine_native_memory::capture_text::Owned,
            engine_native_memory::capture_text::Owned,
        >,
    >,
    subject_starts: Vec<usize>,
    subjects: Arena<u32>,
    reserved: ccountry::ReservedNamingWalk,
    naming: Arena<u8>,
    original_tag_buckets: Arena<engine_stability::original_tag_lists::Bucket>,
    original_tag_members: Arena<u32>,
    original_tag_bucket_len: Result<usize, &'static str>,
    country_array: Arena<u32>,
    country_array_len: Result<usize, &'static str>,
    launch: naval::Launch,
    land_theatre_count: Option<u32>,
    launch_states: Arena<u32>,
    launch_state_scratch: Arena<ccountry::launch_sources::Occurrence>,
    launch_state_result: Result<usize, &'static str>,
    reverse_visibility: reserved_visibility::Captured,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            subjects: 0,
            naming_bytes: 0,
            original_tag_buckets: 0,
            original_tag_members: 0,
            country_array: 0,
            launch_states: 0,
            reverse_visibility: 0,
            launch: Default::default(),
            raw: Default::default(),
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        let raw = capacity.raw_capacity();
        Self {
            raw: engine_native_memory::raw_country_observations::Journal::try_new(raw)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| engine_native_memory::raw_country_observations::Journal::vacant()),
            preacquired: None,
            rows: CaptureRows::new(capacity.rows),
            subject_starts: owned_vec(engine_allocation::default_vec(capacity.rows)),
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
            launch: naval::Launch::new(capacity.launch),
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
        self.launch.reset();
        self.land_theatre_count = None;
        self.launch_states.reset();
        self.launch_state_scratch.reset();
        self.launch_state_result = Err("clock unavailable");
        self.reverse_visibility.reset();
    }

    pub fn fail(&mut self) {
        self.reset();
    }

    pub fn install_land_theatre_count(&mut self, count: Option<u32>) {
        self.land_theatre_count = count;
    }

    fn naming_span(&self, name: engine_native::crelation::subject::Name) -> Option<&[u8]> {
        self.naming
            .first
            .get(..self.naming.len)?
            .get(name.start..name.start.checked_add(name.len)?)
    }

    pub fn reserved_land_certificate_ready(&self) -> bool {
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
                                .map(engine_native_memory::capture_text::Owned::allocated_bytes)
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
            + self.launch.allocated_bytes()
            + self.launch_states.allocated_bytes()
            + self.launch_state_scratch.allocated_bytes()
            + self.reverse_visibility.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        let required = self.raw.required();
        crate::table_capture::grow_capacity(&mut capacity.raw.occurrences, required.occurrences);
        crate::table_capture::grow_capacity(&mut capacity.raw.links, required.links);
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
        self.launch.grow(&mut capacity.launch);
        self.launch_states.grow(&mut capacity.launch_states);
        self.launch_state_scratch.grow(&mut capacity.launch_states);
        self.reverse_visibility
            .grow(&mut capacity.reverse_visibility);
        capacity.raw = capacity.raw_capacity();
        let _ = self
            .raw
            .reserve(capacity.raw)
            .map_err(crate::allocation::io);
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let hour = self.preacquired.as_ref().map_or(0, |marker| marker.hour);
        self.capture_at(memory, base, state, hour)
    }
    pub fn capture_at(
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
        self.raw.begin(hour);
        let ticks = self.capture_original(memory, base, state, None);
        self.raw.finish_native();
        ticks
    }
    pub fn capture_preacquired(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        carry: Option<Carry<'_>>,
    ) {
        let ticks = self.capture_original(memory, base, state, carry);
        self.preacquired = Some(Preacquired {
            base,
            state,
            hour,
            ticks: Some(ticks),
        });
    }
    pub fn fail_preacquired(&mut self, base: usize, state: usize, hour: i32, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
        self.reserved.outcome = Outcome::Unavailable { reason };
        self.preacquired = Some(Preacquired {
            base,
            state,
            hour,
            ticks: Some([0; N]),
        });
    }
    fn capture_original(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        mut carry: Option<Carry<'_>>,
    ) -> [u64; N] {
        self.land_theatre_count = None;
        self.reserved.visibility = None;
        self.reverse_visibility.reset();
        self.launch.reset();
        self.subjects.reset();
        self.naming.reset();
        self.launch_states.reset();
        self.launch_state_scratch.reset();
        self.launch_state_result = Err("reserved launch state control unavailable");
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        if let Some(carry) = carry.as_mut() {
            carry.store.begin(carry.changes);
        }
        match ccountry::dynamic_boundary(memory, base) {
            Some(boundary) => {
                self.rows.outcome = match carry.as_mut() {
                    Some(carry) => {
                        let store = &mut *carry.store;
                        ccountry::walk_appearance_carrying(
                            memory,
                            base,
                            state,
                            boundary,
                            &mut self.rows.rows,
                            &mut self.raw,
                            &mut |globals, country, row| store.select(globals, country, row),
                        )
                    }
                    None => ccountry::walk_appearance_observed(
                        memory,
                        base,
                        state,
                        boundary,
                        &mut self.rows.rows,
                        &mut self.raw,
                    ),
                };
                if !matches!(self.rows.outcome, Outcome::Unavailable { .. }) {
                    self.capture_subjects_carrying(
                        memory,
                        carry.as_ref().map(|carry| &*carry.store),
                    );
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
            engine_native_memory::raw_country_observations::Source::AppearanceOriginal,
            self.rows.outcome,
        );
        self.reserved = ccountry::read_reserved_naming_observed(
            memory,
            base,
            state,
            &mut self.naming.first,
            &mut self.raw,
        );
        self.raw.table(
            engine_native_memory::raw_country_observations::Source::ReservedTag,
            self.reserved.outcome,
        );
        if let Some(control) = self.reserved.launch_states.take() {
            let result = control.capture(
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
        self.capture_original_tags(memory, state, carry.as_ref().map(|carry| &*carry.store));
        let refused = self.rows.rows[..self.rows.outcome.len()]
            .iter()
            .filter(|row| row.original_tag.is_none())
            .count();
        POSITIVE_ORIGINAL_TAG_REFUSALS
            .fetch_add(refused as u64, std::sync::atomic::Ordering::Relaxed);
        if let Some(carry) = carry {
            carry.store.finish(self);
        }
        let appearance_ticks =
            appearance_ticks.saturating_add(qpc().saturating_sub(begin).max(0) as u64);
        drop(_reads);
        let _reads = crate::read_accounting::table(TABLES[1]);
        let begin = qpc();
        let buckets = engine_stability::original_tag_lists::capture(
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
        let array = engine_stability::country_roster::capture(
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
        let mut ticks = [0; N];
        ticks[0] = appearance_ticks;
        ticks[1] = reserved_ticks.saturating_add(qpc().saturating_sub(begin).max(0) as u64);
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.reserved.outcome]
    }

    pub fn launch_sources_mut(&mut self) -> &mut naval::Launch {
        &mut self.launch
    }
    pub fn launch_sources(&self) -> &naval::Launch {
        &self.launch
    }

    pub fn take_reserved_visibility(&mut self) -> Option<ccountry::ReservedVisibilityObservation> {
        self.reserved.visibility.take()
    }

    pub fn reverse_visibility_mut(&mut self) -> &mut reserved_visibility::Captured {
        &mut self.reverse_visibility
    }

    pub fn reverse_visibility_ticks(&self) -> u64 {
        self.reverse_visibility.ticks()
    }

    fn capture_original_tags(
        &mut self,
        memory: &impl Memory,
        state: usize,
        store: Option<&AppearanceStore>,
    ) {
        let len = self.rows.outcome.len();
        let store = store.filter(|_| matches!(self.rows.outcome, Outcome::Rows { .. }));
        let mut start = 0;
        for index in 0..=len {
            let held = store
                .filter(|_| index < len)
                .and_then(|store| store.carried_tag(index));
            if index < len && held.is_none() {
                continue;
            }
            if start < index {
                ccountry::capture_appearance_original_tags_observed(
                    memory,
                    state,
                    &mut self.rows.rows[start..index],
                    &mut self.naming.first,
                    &mut self.naming.len,
                    &mut self.naming.required,
                    &mut self.raw,
                );
            }
            if let Some(bytes) = held
                && !self.place_held_tag(index, bytes)
            {
                ccountry::capture_appearance_original_tags_observed(
                    memory,
                    state,
                    &mut self.rows.rows[index..index + 1],
                    &mut self.naming.first,
                    &mut self.naming.len,
                    &mut self.naming.required,
                    &mut self.raw,
                );
            }
            start = index + 1;
        }
    }

    fn place_held_tag(&mut self, index: usize, bytes: &[u8]) -> bool {
        let used = self.naming.len;
        let row = &mut self.rows.rows[index];
        if row.original_tag_reference <= 0 {
            row.original_tag = Some(engine_native::crelation::subject::Name {
                start: used,
                len: 0,
            });
            row.original_tag_failure = None;
            return true;
        }
        let (Some(required), Some(end)) = (
            self.naming.required.checked_add(bytes.len()),
            used.checked_add(bytes.len()),
        ) else {
            return false;
        };
        if self.naming.first.len() < end
            && engine_allocation::advisory(|| {
                engine_allocation::resize_default(&mut self.naming.first, end)
            })
            .is_err()
        {
            return false;
        }
        self.naming.first[used..end].copy_from_slice(bytes);
        self.naming.required = required;
        self.naming.len = end;
        row.original_tag = Some(engine_native::crelation::subject::Name {
            start: used,
            len: bytes.len(),
        });
        row.original_tag_failure = None;
        true
    }

    #[cfg(test)]
    fn capture_subjects(&mut self, memory: &impl Memory) {
        self.capture_subjects_carrying(memory, None);
    }

    fn capture_subjects_carrying(&mut self, memory: &impl Memory, store: Option<&AppearanceStore>) {
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
                let held = store.and_then(|store| store.carried_subjects(index));
                let copied = self
                    .subjects
                    .first
                    .get_mut(self.subjects.len..end)
                    .is_some_and(|destination| match held {
                        Some(held) if held.len() == destination.len() => {
                            destination.copy_from_slice(held);
                            true
                        }
                        Some(_) => false,
                        None => engine_native_memory::cpdx_array::copy_u32s(
                            memory,
                            subjects,
                            destination,
                            "country subject roster payload unreadable",
                        )
                        .is_ok(),
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.tables_at(None)
    }

    pub fn table_inputs(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
        hour: Option<i32>,
    ) -> [replay_frame::frame::TableInput; N] {
        [
            self.appearance_input(projection),
            replay_frame::frame::TableInput::Sampled(self.reserved_table(hour)),
        ]
    }

    fn appearance_input(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> replay_frame::frame::TableInput {
        let Some(subjects) = self.subjects.first.get(..self.subjects.len) else {
            return unavailable("country subject arena invalid").into();
        };
        let len = self.rows.outcome.len();
        let Some((rows, starts)) = self.rows.rows.get(..len).zip(self.subject_starts.get(..len))
        else {
            return unavailable("country subject span outside captured arena").into();
        };
        if rows.iter().zip(starts).any(|(row, &start)| {
            start
                .checked_add(row.subjects.count as usize)
                .and_then(|end| subjects.get(start..end))
                .is_none()
        }) {
            return unavailable("country subject span outside captured arena").into();
        }
        let base = rows.as_ptr() as usize;
        let stride = std::mem::size_of::<
            ccountry::AppearanceRow<
                engine_native::capture_text::Owned,
                engine_native::capture_text::Owned,
                engine_native::capture_text::Owned,
            >,
        >();
        let start_of = |r: &ccountry::AppearanceRow<
            engine_native::capture_text::Owned,
            engine_native::capture_text::Owned,
            engine_native::capture_text::Owned,
        >| { starts[(r as *const _ as usize - base) / stride] };
        let original_tag_bytes = |r: &ccountry::AppearanceRow<
            engine_native::capture_text::Owned,
            engine_native::capture_text::Owned,
            engine_native::capture_text::Owned,
        >| {
            r.original_tag.and_then(|name| {
                name.start.checked_add(name.len).and_then(|end| {
                    self.naming
                        .first
                        .get(..self.naming.len)?
                        .get(name.start..end)
                })
            })
        };
        collector_projection::projection::project_table::<tables::CountryAppearance, _>(
            projection,
            self.outcomes()[0],
            rows,
            |r| u128::from(r.country),
            |r, prev| {
                if prev.country != r.country
                    || prev.red != r.rgba[0]
                    || prev.green != r.rgba[1]
                    || prev.blue != r.rgba[2]
                    || prev.alpha != r.rgba[3]
                    || prev.overlord != r.overlord
                    || prev.leader_character != r.leader_character
                    || prev.ui_red != r.ui_rgba[0]
                    || prev.ui_green != r.ui_rgba[1]
                    || prev.ui_blue != r.ui_rgba[2]
                    || prev.ui_alpha != r.ui_rgba[3]
                    || prev.original_tag_reference
                        != replay_data::rows::Recorded::Value(Some(r.original_tag_reference))
                    || prev.transport_convoys
                        != replay_data::rows::Recorded::Value(replay_data::rows::Retired)
                    || prev.cosmetic_tag.as_slice() != r.cosmetic_tag.as_bytes()
                    || prev.name_tag.as_slice() != r.name_tag.as_bytes()
                    || prev.leader_ideology.as_slice() != r.leader_ideology.as_bytes()
                    || prev.graphical_culture.as_slice() != r.graphical_culture.as_bytes()
                    || prev.graphical_culture_2d.as_slice() != r.graphical_culture_2d.as_bytes()
                {
                    return false;
                }
                let start = start_of(r);
                let cur_subjects = &subjects[start..start + r.subjects.count as usize];
                let subjects_eq = match &prev.subjects {
                    replay_data::rows::Recorded::Value(prev_subjects) => {
                        prev_subjects.len() == cur_subjects.len()
                            && prev_subjects
                                .iter()
                                .zip(cur_subjects)
                                .all(|(&a, &b)| a == b as i32)
                    }
                    _ => false,
                };
                if !subjects_eq {
                    return false;
                }
                let old_names_eq = match (
                    r.old_names.as_ref(),
                    prev.old_name.as_deref(),
                    prev.old_adjective.as_deref(),
                    prev.old_definite_name.as_deref(),
                ) {
                    (Some(cur), Some(a), Some(b), Some(c)) => {
                        a == cur[0].as_bytes() && b == cur[1].as_bytes() && c == cur[2].as_bytes()
                    }
                    (None, None, None, None) => true,
                    _ => false,
                };
                if !old_names_eq {
                    return false;
                }
                match (original_tag_bytes(r), &prev.original_tag) {
                    (Some(cur), replay_data::rows::Recorded::Value(Some(prev))) => {
                        prev.as_slice() == cur
                    }
                    (None, replay_data::rows::Recorded::Value(None)) => true,
                    _ => false,
                }
            },
            |r| {
                let start = start_of(r);
                CountryAppearanceRow {
                    country: r.country,
                    subjects: replay_data::rows::Recorded::Value(owned_vec(
                        engine_allocation::collect_vec(
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
                    original_tag: replay_data::rows::Recorded::Value(
                        original_tag_bytes(r).map(|bytes| owned_vec(copy_vec(bytes))),
                    ),
                    original_tag_reference: replay_data::rows::Recorded::Value(Some(
                        r.original_tag_reference,
                    )),
                    transport_convoys: replay_data::rows::Recorded::Value(
                        replay_data::rows::Retired,
                    ),
                }
            },
        )
    }

    pub fn tables_at(&self, hour: Option<i32>) -> [TableView; N] {
        [self.appearance_table(), self.reserved_table(hour)]
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
        crate::table_conversion::table_iter::<tables::CountryAppearance, _>(
            self.outcomes()[0],
            self.rows.rows[..self.rows.outcome.len()]
                .iter()
                .zip(&self.subject_starts),
            |(r, &start)| CountryAppearanceRow {
                country: r.country,
                subjects: replay_data::rows::Recorded::Value(owned_vec(
                    engine_allocation::collect_vec(
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
                original_tag: replay_data::rows::Recorded::Value(
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
                original_tag_reference: replay_data::rows::Recorded::Value(Some(
                    r.original_tag_reference,
                )),
                transport_convoys: replay_data::rows::Recorded::Value(replay_data::rows::Retired),
            },
        )
    }

    fn reserved_table(&self, frame_hour: Option<i32>) -> TableView {
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
        let original_tag = r.original_tag.and_then(span);
        let current = |sample: Option<i32>| {
            sample.is_some_and(|sample| frame_hour.is_none_or(|hour| sample == hour))
        };
        crate::table_conversion::table::<tables::ReservedCountryNaming, _>(
            self.reserved.outcome,
            std::slice::from_ref(r),
            |_| ReservedCountryNamingRow {
                attack: replay_data::rows::Recorded::Value(replay_data::rows::Witnessed(None)),
                raw_country_observations: replay_data::rows::Recorded::Value(
                    replay_data::rows::Retired,
                ),
                tag: owned_vec(copy_vec(tag)),
                name_tag: owned_vec(copy_vec(name_tag)),
                cosmetic_tag: owned_vec(copy_vec(cosmetic_tag)),
                leader_ideology: owned_vec(copy_vec(leader_ideology)),
                group: group.map(|bytes| owned_vec(copy_vec(bytes))),
                overlord: r.overlord,
                autonomy: autonomy.map(|bytes| owned_vec(copy_vec(bytes))),
                built: r.built,
                original_tag: replay_data::rows::Recorded::Value(
                    original_tag.map(|bytes| owned_vec(copy_vec(bytes))),
                ),
                original_tag_reference: replay_data::rows::Recorded::Value(Some(
                    r.original_tag_reference,
                )),
                original_tag_buckets: replay_data::rows::Recorded::Value(self.original_tag_lists()),
                country_array: replay_data::rows::Recorded::Value(
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
                owned_state_count: replay_data::rows::Recorded::Value(r.owned_state_count),
                faction: replay_data::rows::Recorded::Value(r.faction),
                land_theatre_count: replay_data::rows::Recorded::Value(self.land_theatre_count),
                movement: replay_data::rows::Recorded::Value(replay_data::rows::Retired),
                launch_sources: replay_data::rows::Recorded::Value(
                    self.launch_projection()
                        .unwrap_or_else(|error| {
                            crate::allocation::observe(&error);
                            None
                        })
                        .filter(|_| current(self.launch.launch_sample_hour())),
                ),
                reverse_visibility: replay_data::rows::Recorded::Value(
                    self.reverse_visibility
                        .projection()
                        .unwrap_or_else(|error| {
                            crate::allocation::observe(&error);
                            None
                        })
                        .filter(|_| current(self.reverse_visibility.sample_hour())),
                ),
                transport_mission_inputs: replay_data::rows::Recorded::Value(
                    replay_data::rows::Retired,
                ),
                country_intel_sample: replay_data::rows::Recorded::Value(None),
                dynamic_tag_boundary: replay_data::rows::Recorded::Value(Some(
                    r.dynamic_tag_boundary,
                )),
            },
        )
    }

    fn launch_projection(
        &self,
    ) -> std::io::Result<Option<replay_data::rows::ReservedLaunchSources>> {
        if self.launch.launch_sample_hour().is_none() {
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
        self.launch.projection_launch(owned_states)
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
        Some(owned_vec(engine_allocation::collect_vec(
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
