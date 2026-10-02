use super::*;

#[derive(Debug)]
pub(crate) struct OwnedFogJob<R> {
    pub(super) rows: Vec<R>,
    pub(super) provinces: Vec<u32>,
    pub(super) outcome: Outcome,
    pub(super) provinces_len: usize,
    pub(super) required_provinces: usize,
    pub(super) row_limit: usize,
    pub(super) province_limit: usize,
    pub(super) acquired: bool,
    pub(super) active: bool,
    pub(super) raw_bytes: u64,
    pub(super) capacity_charge: crate::raw_memory::Charge,
}

impl<R: Default + Copy> Default for OwnedFogJob<R> {
    fn default() -> Self {
        Self::with_storage(0, 0)
    }
}

impl<R: Default + Copy> OwnedFogJob<R> {
    pub(super) fn with_storage(rows: usize, provinces: usize) -> Self {
        let mut job = Self {
            rows: vec![R::default(); rows],
            provinces: vec![0; provinces],
            outcome: Outcome::Rows { len: 0 },
            provinces_len: 0,
            required_provinces: 0,
            row_limit: 0,
            province_limit: 0,
            acquired: false,
            active: false,
            raw_bytes: 0,
            capacity_charge: Default::default(),
        };
        job.update_capacity_charge();
        job
    }
    fn update_capacity_charge(&mut self) {
        self.capacity_charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.rows)
                + crate::raw_memory::vector_bytes(&self.provinces),
            ..Default::default()
        });
    }
    pub fn clear(&mut self) {
        self.outcome = Outcome::Rows { len: 0 };
        self.provinces_len = 0;
        self.required_provinces = 0;
        self.row_limit = 0;
        self.province_limit = 0;
        self.acquired = false;
        self.active = false;
        self.raw_bytes = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<R>() + self.provinces.capacity() * size_of::<u32>()
    }
    pub fn grow_for_next(&self) -> bool {
        self.row_limit > self.rows.len() || self.province_limit > self.provinces.len()
    }
    /// Called only during worker-side recycling.
    pub fn grow(&mut self) {
        if self.row_limit > self.rows.len() {
            self.rows.resize(self.row_limit, R::default());
        }
        if self.province_limit > self.provinces.len() {
            self.provinces.resize(self.province_limit, 0);
        }
        self.update_capacity_charge();
    }
    pub fn note_growth(&self) {
        if self.grow_for_next() {
            crate::raw::publish_growth(
                std::any::type_name::<R>(),
                serde_json::json!({
                    "row_capacity": self.rows.len(), "required_rows": self.row_limit,
                    "province_capacity": self.provinces.len(), "required_provinces": self.province_limit,
                }),
            );
        }
    }
    fn prepare(&mut self, rows: usize, provinces: usize) {
        self.clear();
        self.active = true;
        self.row_limit = rows;
        self.province_limit = provinces;
    }
    fn fits(&self, rows: usize, provinces: usize) -> bool {
        rows <= self.rows.len() && provinces <= self.provinces.len()
    }
}
impl RadarJob {
    pub fn new() -> Self {
        Self::with_storage(1024, 65_536)
    }
}
impl IntelJob {
    pub fn new() -> Self {
        Self::with_storage(32_768, 0)
    }
}

/// Count successful bytes while preserving the native guarded batch operations.
struct FogMemory<'a, M> {
    inner: &'a M,
    bytes: std::cell::Cell<u64>,
}
impl<M: Memory> FogMemory<'_, M> {
    fn count(&self, bytes: usize) {
        self.bytes
            .set(self.bytes.get().saturating_add(bytes as u64));
    }
}
impl<M: Memory> Memory for FogMemory<'_, M> {
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        let value = self.inner.read_scalar::<T>(address);
        if value.is_some() {
            self.count(size_of::<T>());
        }
        value
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        self.inner.authenticated_countries(base, state)
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let succeeded = self.inner.copy(address, destination);
        if succeeded {
            self.count(destination.len());
        }
        succeeded
    }
    fn copy_scattered<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        let completed = self.inner.copy_scattered(addresses, destinations);
        for field in destinations.iter().take(completed) {
            self.count(field.len());
        }
        completed
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let completed = self.inner.copy_records(sources, layout, destination);
        self.count(layout.completed_bytes(completed));
        completed
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RadarCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; RADAR_N],
    pub raw_bytes: [u64; RADAR_N],
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct RadarCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub row_limit: usize,
    pub province_limit: usize,
}
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct IntelCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; INTEL_N],
    pub raw_bytes: [u64; INTEL_N],
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct IntelCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub row_limit: usize,
}
fn acquire_radar(job: &mut RadarJob, memory: &impl Memory, ctx: RadarCopyContext) {
    let _reads = read_accounting::table(TableId::RadarCoverage);
    let source = FogMemory {
        inner: memory,
        bytes: std::cell::Cell::new(0),
    };
    let result = radar_group::walk(
        &source,
        ctx.base,
        ctx.game_state,
        &mut job.rows[..job.row_limit],
        &mut job.provinces[..job.province_limit],
    );
    job.outcome = result.outcome;
    job.provinces_len = result.provinces_len;
    job.required_provinces = result.required_provinces;
    job.raw_bytes = source.bytes.get();
    job.acquired = true;
}
fn acquire_intel(
    job: &mut IntelJob,
    memory: &impl Memory,
    ctx: IntelCopyContext,
    divisions: Option<unit_viewer_entry::DivisionSources<'_>>,
) {
    let _reads = read_accounting::table(TableId::UnitIntel);
    let source = FogMemory {
        inner: memory,
        bytes: std::cell::Cell::new(0),
    };
    job.outcome = unit_viewer_entry::walk_with_divisions(
        &source,
        ctx.base,
        ctx.game_state,
        &mut job.rows[..job.row_limit],
        divisions,
    );
    job.raw_bytes = source.bytes.get();
    job.acquired = true;
}
/// Check destination capacities before acquiring one owned radar snapshot.
pub(crate) fn copy_radar_refresh(
    memory: &impl Memory,
    ctx: RadarCopyContext,
    job: &mut RadarJob,
) -> RadarCopyReport {
    if !replay::cadence::should_walk(TableId::RadarCoverage, ctx.hour, ctx.force_full) {
        job.clear();
        return RadarCopyReport::default();
    }
    let started = qpc();
    job.prepare(ctx.row_limit, ctx.province_limit);
    if job.fits(ctx.row_limit, ctx.province_limit) {
        acquire_radar(job, memory, ctx);
    }
    RadarCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started)],
        raw_bytes: [job.raw_bytes],
    }
}
/// The caller retains the primary division census until acquisition returns.
pub(crate) fn copy_intel_refresh_with_divisions(
    memory: &impl Memory,
    ctx: IntelCopyContext,
    job: &mut IntelJob,
    divisions: Option<unit_viewer_entry::DivisionSources<'_>>,
) -> IntelCopyReport {
    let started = qpc();
    job.prepare(ctx.row_limit, 0);
    if job.fits(ctx.row_limit, 0) {
        acquire_intel(job, memory, ctx, divisions);
    }
    IntelCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started)],
        raw_bytes: [job.raw_bytes],
    }
}

impl Captured {
    pub(crate) fn radar_copy_context(
        &self,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> RadarCopyContext {
        RadarCopyContext {
            base,
            game_state,
            hour,
            force_full,
            row_limit: self.radar.rows.len(),
            province_limit: self.radar_provinces.first.len(),
        }
    }

    pub(crate) fn intel_copy_context(&self, base: usize, game_state: usize) -> IntelCopyContext {
        IntelCopyContext {
            base,
            game_state,
            row_limit: self.intel.rows.len(),
        }
    }

    /// Complete an accepted worker failure after the sibling capture has
    /// finished. The reservation gate prevented any synchronous retry; no
    /// owned payload from this failed acquisition may subsequently be decoded.
    pub(crate) fn fail_radar_capture(&mut self, reason: &'static str) {
        self.radar.outcome = if self.active[0] {
            Outcome::Unavailable { reason }
        } else {
            SKIPPED
        };
        self.radar_provinces.reset();
        self.radar_raw_mode = false;
    }

    pub(crate) fn fail_intel_capture(&mut self, reason: &'static str) {
        self.intel.outcome = Outcome::Unavailable { reason };
        self.intel_raw_mode = false;
    }

    pub(crate) fn decode_radar_payload(
        &mut self,
        job: &RadarJob,
        decode_ticks: &mut [u64; RADAR_N],
    ) {
        self.active[0] = job.active;
        if !job.active {
            self.fail_radar_capture(replay::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        if !job.acquired
            || job.row_limit != self.radar.rows.len()
            || job.province_limit != self.radar_provinces.first.len()
        {
            self.radar.outcome = Outcome::Unavailable {
                reason: "radar owned payload capacity mismatch",
            };
        } else {
            let len = job.outcome.len();
            self.radar.rows[..len].copy_from_slice(&job.rows[..len]);
            self.radar_provinces.first[..job.provinces_len]
                .copy_from_slice(&job.provinces[..job.provinces_len]);
            arena_first(
                &mut self.radar,
                &mut self.radar_provinces,
                job.outcome,
                job.provinces_len,
                job.required_provinces,
            );
        }
        decode_ticks[0] += elapsed(started);
        self.radar_raw_mode = false;
    }

    pub(crate) fn decode_intel_payload(
        &mut self,
        job: &IntelJob,
        decode_ticks: &mut [u64; INTEL_N],
    ) {
        let started = qpc();
        if !job.acquired || job.row_limit != self.intel.rows.len() {
            self.intel.outcome = Outcome::Unavailable {
                reason: "intel owned payload capacity mismatch",
            };
        } else {
            let len = job.outcome.len();
            self.intel.rows[..len].copy_from_slice(&job.rows[..len]);
            self.intel.outcome = job.outcome;
        }
        decode_ticks[0] += elapsed(started);
        self.intel_raw_mode = false;
    }
}
