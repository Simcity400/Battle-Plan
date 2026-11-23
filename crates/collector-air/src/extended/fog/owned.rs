use super::*;
use engine_allocation::{default_vec, filled, owned_vec, resize};

#[derive(Debug)]
pub struct OwnedFogJob<R> {
    pub(super) rows: Vec<R>,
    pub(super) provinces: Vec<u32>,
    pub(super) outcome: Outcome,
    pub(super) provinces_len: usize,
    pub(super) required_provinces: usize,
    pub(super) row_limit: usize,
    pub(super) province_limit: usize,
    pub(super) acquired: bool,
    pub(super) active: bool,
    pub(super) capacity_charge: collector_primitives::raw_memory::Charge,
}

impl<R: Default + Copy> Default for OwnedFogJob<R> {
    fn default() -> Self {
        Self::with_storage(0, 0)
    }
}

impl<R: Default + Copy> OwnedFogJob<R> {
    pub(super) fn with_storage(rows: usize, provinces: usize) -> Self {
        let mut job = Self {
            rows: owned_vec(default_vec(rows)),
            provinces: owned_vec(filled(provinces, 0)),
            outcome: Outcome::Rows { len: 0 },
            provinces_len: 0,
            required_provinces: 0,
            row_limit: 0,
            province_limit: 0,
            acquired: false,
            active: false,
            capacity_charge: Default::default(),
        };
        job.update_capacity_charge();
        job
    }
    fn update_capacity_charge(&mut self) {
        self.capacity_charge
            .set(collector_primitives::raw_memory::Capacity {
                payload_vectors: collector_primitives::raw_memory::vector_bytes(&self.rows)
                    + collector_primitives::raw_memory::vector_bytes(&self.provinces),
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
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<R>() + self.provinces.capacity() * size_of::<u32>()
    }
    pub fn grow_for_next(&self) -> bool {
        self.row_limit > self.rows.len() || self.province_limit > self.provinces.len()
    }
    pub fn grow(&mut self) {
        if self.row_limit > self.rows.len()
            && resize(&mut self.rows, self.row_limit, R::default()).is_err()
        {
            return;
        }
        if self.province_limit > self.provinces.len()
            && resize(&mut self.provinces, self.province_limit, 0).is_err()
        {
            return;
        }
        self.update_capacity_charge();
    }
    pub fn note_growth(&self) {
        if self.grow_for_next() {
            collector_raw::raw::publish_growth(
                std::any::type_name::<R>(),
                collector_raw::raw::Growth::Fog {
                    row_capacity: self.rows.len(),
                    required_rows: self.row_limit,
                    province_capacity: self.provinces.len(),
                    required_provinces: self.province_limit,
                },
            );
        }
    }
    fn prepare(&mut self, rows: usize, provinces: usize) {
        self.clear();
        self.active = true;
        self.row_limit = rows;
        self.province_limit = provinces;
    }
}
impl RadarJob {
    pub fn new() -> Self {
        Self::default()
    }
}
impl IntelJob {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RadarCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; RADAR_N],
}
#[derive(Debug, Clone, Copy)]
pub struct RadarCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub row_limit: usize,
    pub province_limit: usize,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct IntelCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; INTEL_N],
}
#[derive(Debug, Clone, Copy)]
pub struct IntelCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub row_limit: usize,
}
fn acquire_radar(job: &mut RadarJob, memory: &impl Memory, ctx: RadarCopyContext) {
    let _reads = read_accounting::table(TableId::RadarCoverage);
    let (result, symbols_trusted) = collector_missions::radar_changes::with_symbols(
        memory,
        ctx.row_limit,
        ctx.force_full,
        |symbols| {
            radar_group::walk_with_symbols(
                memory,
                ctx.base,
                ctx.game_state,
                &mut job.rows,
                &mut job.provinces,
                symbols,
            )
        },
    );
    if !symbols_trusted {
        for row in &mut job.rows[..result.outcome.len()] {
            row.symbol = None;
        }
    }
    job.outcome = result.outcome;
    job.provinces_len = result.provinces_len;
    job.required_provinces = result.required_provinces;
    job.acquired = true;
}
fn acquire_intel(
    job: &mut IntelJob,
    memory: &impl Memory,
    ctx: IntelCopyContext,
    divisions: Option<unit_viewer_entry::DivisionSources<'_>>,
) {
    let _reads = read_accounting::table(TableId::UnitIntel);
    job.outcome = unit_viewer_entry::walk_with_divisions_buffer(
        memory,
        ctx.base,
        ctx.game_state,
        engine_native_memory::walk::Buffer::Growing(&mut job.rows),
        divisions,
    );
    job.acquired = true;
}

pub fn copy_radar_refresh(
    memory: &impl Memory,
    ctx: RadarCopyContext,
    job: &mut RadarJob,
) -> RadarCopyReport {
    if !replay_data::cadence::should_walk(TableId::RadarCoverage, ctx.hour, ctx.force_full) {
        job.clear();
        return RadarCopyReport::default();
    }
    let started = qpc();
    job.prepare(ctx.row_limit, ctx.province_limit);
    acquire_radar(job, memory, ctx);
    job.update_capacity_charge();
    RadarCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started)],
    }
}

pub fn copy_intel_refresh_with_divisions(
    memory: &impl Memory,
    ctx: IntelCopyContext,
    job: &mut IntelJob,
    divisions: Option<unit_viewer_entry::DivisionSources<'_>>,
) -> IntelCopyReport {
    let started = qpc();
    job.prepare(ctx.row_limit, 0);
    acquire_intel(job, memory, ctx, divisions);
    job.update_capacity_charge();
    IntelCopyReport {
        raw_ok: job.acquired,
        copy_ticks: [elapsed(started)],
    }
}

impl Captured {
    pub fn radar_copy_context(
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

    pub fn intel_copy_context(&self, base: usize, game_state: usize) -> IntelCopyContext {
        IntelCopyContext {
            base,
            game_state,
            row_limit: self.intel.rows.len(),
        }
    }

    pub fn fail_radar_capture(&mut self, reason: &'static str) {
        self.radar.outcome = if self.active[0] {
            Outcome::Unavailable { reason }
        } else {
            SKIPPED
        };
        self.radar_provinces.reset();
        self.radar_raw_mode = false;
    }

    pub fn fail_intel_capture(&mut self, reason: &'static str) {
        self.intel.outcome = Outcome::Unavailable { reason };
        self.intel_raw_mode = false;
    }

    pub fn decode_radar_payload(&mut self, job: &RadarJob, decode_ticks: &mut [u64; RADAR_N]) {
        self.active[0] = job.active;
        if !job.active {
            self.fail_radar_capture(replay_data::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        self.radar_provinces.required = job.required_provinces;
        let growth = {
            engine_native_memory::walk::Buffer::Growing(&mut self.radar.rows)
                .ensure(job.outcome.len())
                .and_then(|_| {
                    engine_native_memory::walk::Buffer::Growing(&mut self.radar_provinces.first)
                        .ensure(job.provinces_len)
                })
        };
        if let Err(reason) = growth {
            self.radar.outcome = job.outcome;
            self.radar.fail(Outcome::Unavailable { reason });
        } else if !job.acquired {
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

    pub fn decode_intel_payload(&mut self, job: &IntelJob, decode_ticks: &mut [u64; INTEL_N]) {
        let started = qpc();
        let growth = engine_native_memory::walk::Buffer::Growing(&mut self.intel.rows).ensure(job.outcome.len());
        if let Err(reason) = growth {
            self.intel.outcome = job.outcome;
            self.intel.fail(Outcome::Unavailable { reason });
        } else if !job.acquired {
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
