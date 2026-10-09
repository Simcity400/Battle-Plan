use super::{
    AirBasesJob, AirHistoryJob, AirStatsJob, AirWingJob, CHARACTER_PARTS, CapitulationJob,
    CharactersJob, ConstructionRestJob, DivisionsJob, EconomyRestJob, FocusJob, IntelJob,
    LexicalJob, NavalJob, PlayersJob, ProductionRestJob, ProjectsJob, RadarJob, RelationsJob,
    RetiredPayload, SessionJob, SunkJob, SupremacyJob, WeatherJob, defer_payload,
};
use std::sync::Mutex;

/// Live raw payloads beyond this count are not retained; further refreshes
/// take the existing gap path until the worker catches up (see module docs).
pub(super) const JOB_POOL_KEEP: usize = 2;

pub(super) static RELATIONS_JOB_POOL: Mutex<Vec<RelationsJob>> = Mutex::new(Vec::new());
pub(super) static CHARACTERS_JOB_POOL: Mutex<[Vec<CharactersJob>; CHARACTER_PARTS]> =
    Mutex::new([const { Vec::new() }; CHARACTER_PARTS]);
pub(super) static PROJECTS_JOB_POOL: Mutex<Vec<ProjectsJob>> = Mutex::new(Vec::new());
pub(super) static LEXICAL_JOB_POOL: Mutex<Vec<LexicalJob>> = Mutex::new(Vec::new());
pub(super) static DIVISIONS_JOB_POOL: Mutex<Vec<DivisionsJob>> = Mutex::new(Vec::new());
pub(super) static FOCUS_JOB_POOL: Mutex<Vec<FocusJob>> = Mutex::new(Vec::new());
pub(super) static AIR_WING_JOB_POOL: Mutex<Vec<AirWingJob>> = Mutex::new(Vec::new());
pub(super) static AIR_STATS_JOB_POOL: Mutex<Vec<AirStatsJob>> = Mutex::new(Vec::new());
pub(super) static NAVAL_JOB_POOL: Mutex<Vec<NavalJob>> = Mutex::new(Vec::new());
pub(super) static PLAYERS_JOB_POOL: Mutex<Vec<PlayersJob>> = Mutex::new(Vec::new());
pub(super) static SESSION_JOB_POOL: Mutex<Vec<SessionJob>> = Mutex::new(Vec::new());
pub(super) static PRODUCTION_REST_JOB_POOL: Mutex<Vec<ProductionRestJob>> = Mutex::new(Vec::new());
pub(super) static CONSTRUCTION_REST_JOB_POOL: Mutex<Vec<ConstructionRestJob>> =
    Mutex::new(Vec::new());
pub(super) static ECONOMY_REST_JOB_POOL: Mutex<Vec<EconomyRestJob>> = Mutex::new(Vec::new());
pub(super) static AIR_BASES_JOB_POOL: Mutex<Vec<AirBasesJob>> = Mutex::new(Vec::new());
pub(super) static AIR_HISTORY_JOB_POOL: Mutex<Vec<AirHistoryJob>> = Mutex::new(Vec::new());
pub(super) static SUPREMACY_JOB_POOL: Mutex<Vec<SupremacyJob>> = Mutex::new(Vec::new());
pub(super) static SUNK_JOB_POOL: Mutex<Vec<SunkJob>> = Mutex::new(Vec::new());
pub(super) static RADAR_JOB_POOL: Mutex<Vec<RadarJob>> = Mutex::new(Vec::new());
pub(super) static INTEL_JOB_POOL: Mutex<Vec<IntelJob>> = Mutex::new(Vec::new());
pub(super) static WEATHER_JOB_POOL: Mutex<Vec<WeatherJob>> = Mutex::new(Vec::new());
pub(super) static CAPITULATION_JOB_POOL: Mutex<Vec<CapitulationJob>> = Mutex::new(Vec::new());

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_relations_job() -> RelationsJob {
    if let Ok(mut pool) = RELATIONS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    RelationsJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_relations_job(job: RelationsJob) {
    defer_payload(RetiredPayload::Relations(job));
}

pub(super) fn release_relations_job(mut job: RelationsJob) {
    if crate::allocation::failed() {
        return;
    }
    job.return_opinions();
    if !job.is_allocated() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = RELATIONS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_characters_job(part: usize) -> CharactersJob {
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.try_lock()
        && let Some(job) = pool[part].pop()
    {
        return job;
    }
    let mut job = CharactersJob::default();
    job.part = part;
    job
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_characters_job(job: CharactersJob) {
    defer_payload(RetiredPayload::Characters(job));
}

pub(super) fn release_characters_job(mut job: CharactersJob) {
    if crate::allocation::failed() {
        return;
    }
    if !job.has_storage() {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    job.prepare_read_plans();
    if crate::allocation::failed() {
        return;
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.lock()
        && pool[job.part].len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool[job.part], 1).is_ok()
    {
        let part = job.part;
        pool[part].push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_projects_job() -> ProjectsJob {
    if let Ok(mut pool) = PROJECTS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ProjectsJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_projects_job(job: ProjectsJob) {
    defer_payload(RetiredPayload::Projects(job));
}

pub(super) fn release_projects_job(mut job: ProjectsJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Projects", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    job.presize_countries();
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = PROJECTS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

fn pop_job<J: Default>(pool: &Mutex<Vec<J>>) -> J {
    if let Ok(mut pool) = pool.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    J::default()
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_lexical_job() -> LexicalJob {
    pop_job(&LEXICAL_JOB_POOL)
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_lexical_job(job: LexicalJob) {
    defer_payload(RetiredPayload::Lexical(job));
}

pub(super) fn release_lexical_job(mut job: LexicalJob) {
    if crate::allocation::failed() {
        return;
    }
    if !job.initialized() {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = LEXICAL_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_divisions_job() -> DivisionsJob {
    pop_job(&DIVISIONS_JOB_POOL)
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_divisions_job(job: DivisionsJob) {
    defer_payload(RetiredPayload::Divisions(job));
}

pub(super) fn release_divisions_job(mut job: DivisionsJob) {
    if crate::allocation::failed() {
        return;
    }
    if !job.initialized() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = DIVISIONS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_focus_job() -> FocusJob {
    pop_job(&FOCUS_JOB_POOL)
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_focus_job(job: FocusJob) {
    defer_payload(RetiredPayload::Focus(job));
}

pub(super) fn release_focus_job(mut job: FocusJob) {
    if crate::allocation::failed() {
        return;
    }
    if !job.is_provisioned() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = FOCUS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Exclusive spool-thread sizing of idle jobs after live name demand grows.
pub(crate) fn presize_air_wing_names() {
    if let Ok(mut pool) = AIR_WING_JOB_POOL.lock() {
        for job in pool.iter_mut() {
            if job.grow_for_next() {
                job.grow();
            }
        }
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_air_wing_job() -> AirWingJob {
    pop_job(&AIR_WING_JOB_POOL)
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_air_wing_job(job: AirWingJob) {
    defer_payload(RetiredPayload::AirWing(job));
}

pub(super) fn release_air_wing_job(mut job: AirWingJob) {
    if crate::allocation::failed() {
        return;
    }
    if !job.has_storage() && !job.grow_for_next() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.presize_game_score();
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = AIR_WING_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_air_stats_job() -> AirStatsJob {
    pop_job(&AIR_STATS_JOB_POOL)
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_air_stats_job(job: AirStatsJob) {
    defer_payload(RetiredPayload::AirStats(job));
}

pub(super) fn release_air_stats_job(mut job: AirStatsJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("AirStats", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = AIR_STATS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_naval_job() -> NavalJob {
    if let Ok(mut pool) = NAVAL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    NavalJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_naval_job(job: NavalJob) {
    defer_payload(RetiredPayload::Naval(job));
}

pub(super) fn release_naval_job(mut job: NavalJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Naval", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    job.presize_countries();
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = NAVAL_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_players_job() -> PlayersJob {
    if let Ok(mut pool) = PLAYERS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    PlayersJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_players_job(job: PlayersJob) {
    defer_payload(RetiredPayload::Players(job));
}

pub(super) fn release_players_job(mut job: PlayersJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Players", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = PLAYERS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_session_job() -> SessionJob {
    if let Ok(mut pool) = SESSION_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SessionJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_session_job(job: SessionJob) {
    defer_payload(RetiredPayload::Session(job));
}

pub(super) fn release_session_job(mut job: SessionJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Session", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = SESSION_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_production_rest_job() -> ProductionRestJob {
    if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ProductionRestJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_production_rest_job(job: ProductionRestJob) {
    defer_payload(RetiredPayload::ProductionRest(job));
}

pub(super) fn release_production_rest_job(mut job: ProductionRestJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("ProductionRest", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    // Compile sealed source/control metadata at retirement, after any required
    // decode. Production decoding does not modify captured bytes. Reusing a
    // successful plan avoids recompiling it on every returned job.
    job.log.prepare_read_plan();
    if crate::allocation::failed() {
        return;
    }
    job.presize_countries();
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_construction_rest_job() -> ConstructionRestJob {
    if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ConstructionRestJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_construction_rest_job(job: ConstructionRestJob) {
    defer_payload(RetiredPayload::ConstructionRest(job));
}

pub(super) fn release_construction_rest_job(mut job: ConstructionRestJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("ConstructionRest", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    job.presize_countries();
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_economy_rest_job() -> EconomyRestJob {
    if let Ok(mut pool) = ECONOMY_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    EconomyRestJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_economy_rest_job(job: EconomyRestJob) {
    defer_payload(RetiredPayload::EconomyRest(job));
}

pub(super) fn release_economy_rest_job(mut job: EconomyRestJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("EconomyRest", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = ECONOMY_REST_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_air_bases_job() -> AirBasesJob {
    if let Ok(mut pool) = AIR_BASES_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirBasesJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_air_bases_job(job: AirBasesJob) {
    defer_payload(RetiredPayload::AirBases(job));
}

pub(super) fn release_air_bases_job(mut job: AirBasesJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("AirBases", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    job.presize_countries();
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = AIR_BASES_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_air_history_job() -> AirHistoryJob {
    if let Ok(mut pool) = AIR_HISTORY_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirHistoryJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_air_history_job(job: AirHistoryJob) {
    defer_payload(RetiredPayload::AirHistory(job));
}

pub(super) fn release_air_history_job(mut job: AirHistoryJob) {
    if crate::allocation::failed() {
        return;
    }
    if !job.has_storage() && !job.grow_for_next() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = AIR_HISTORY_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_supremacy_job() -> SupremacyJob {
    if let Ok(mut pool) = SUPREMACY_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SupremacyJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_supremacy_job(job: SupremacyJob) {
    defer_payload(RetiredPayload::Supremacy(job));
}

pub(super) fn release_supremacy_job(mut job: SupremacyJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Supremacy", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = SUPREMACY_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_sunk_job() -> SunkJob {
    if let Ok(mut pool) = SUNK_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SunkJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_sunk_job(job: SunkJob) {
    defer_payload(RetiredPayload::Sunk(job));
}

pub(super) fn release_sunk_job(mut job: SunkJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Sunk", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = SUNK_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_radar_job() -> RadarJob {
    if let Ok(mut pool) = RADAR_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    RadarJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_radar_job(job: RadarJob) {
    defer_payload(RetiredPayload::Radar(job));
}

pub(super) fn release_radar_job(mut job: RadarJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.allocated_bytes() == 0 {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = RADAR_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_intel_job() -> IntelJob {
    if let Ok(mut pool) = INTEL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    IntelJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_intel_job(job: IntelJob) {
    defer_payload(RetiredPayload::Intel(job));
}

pub(super) fn release_intel_job(mut job: IntelJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.allocated_bytes() == 0 {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = INTEL_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_weather_job() -> WeatherJob {
    if let Ok(mut pool) = WEATHER_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    WeatherJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_weather_job(job: WeatherJob) {
    defer_payload(RetiredPayload::Weather(job));
}

pub(super) fn release_weather_job(mut job: WeatherJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Weather", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = WEATHER_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_capitulation_job() -> CapitulationJob {
    if let Ok(mut pool) = CAPITULATION_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    CapitulationJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_capitulation_job(job: CapitulationJob) {
    defer_payload(RetiredPayload::Capitulation(job));
}

pub(super) fn release_capitulation_job(mut job: CapitulationJob) {
    if crate::allocation::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Capitulation", &job.log);
    if job.grow_for_next() {
        job.grow();
        if crate::allocation::failed() {
            return;
        }
    }
    if crate::allocation::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = CAPITULATION_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
        && engine::allocation::reserve(&mut pool, 1).is_ok()
    {
        pool.push(job);
    }
}

/// Spool thread: presize pooled raw-copy jobs whose country census follows the
/// registry, after a larger registry was observed (for example a save loaded
/// after attaching at the main menu). The game thread only `try_lock`s these
/// pools, so a concurrent callback takes a fresh job instead of waiting.
pub fn presize_country_pools() {
    presize_pool(
        &AIR_WING_JOB_POOL,
        AirWingJob::presize_game_score,
        release_air_wing_job,
    );
    presize_pool(
        &PROJECTS_JOB_POOL,
        ProjectsJob::presize_countries,
        release_projects_job,
    );
    presize_pool(
        &PRODUCTION_REST_JOB_POOL,
        ProductionRestJob::presize_countries,
        release_production_rest_job,
    );
    presize_pool(
        &CONSTRUCTION_REST_JOB_POOL,
        ConstructionRestJob::presize_countries,
        release_construction_rest_job,
    );
    presize_pool(
        &AIR_BASES_JOB_POOL,
        AirBasesJob::presize_countries,
        release_air_bases_job,
    );
    presize_pool(
        &NAVAL_JOB_POOL,
        NavalJob::presize_countries,
        release_naval_job,
    );
}

/// New modifier hints also reach jobs prepared before the campaign loaded.
pub(crate) fn presize_advisor_pools() {
    let jobs = match CHARACTERS_JOB_POOL.lock() {
        Ok(mut pool) => std::mem::take(&mut pool[2]),
        Err(_) => return,
    };
    for mut job in jobs {
        if job.grow_for_next() {
            job.grow();
        }
        release_characters_job(job);
    }
}

fn presize_pool<J>(
    pool: &Mutex<Vec<J>>,
    mut presize: impl FnMut(&mut J),
    mut release: impl FnMut(J),
) {
    let jobs = match pool.lock() {
        Ok(mut pool) => std::mem::take(&mut *pool),
        Err(_) => return,
    };
    for mut job in jobs {
        presize(&mut job);
        if crate::allocation::failed() {
            return;
        }
        release(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_score_pool_presize_releases_mutex_during_work() {
        let pool = Mutex::new(vec![1, 2]);
        let mut grown = 0;
        presize_pool(
            &pool,
            |job| {
                assert_eq!(pool.try_lock().unwrap().len(), grown);
                *job += 10;
                grown += 1;
            },
            |job| {
                pool.lock().unwrap().push(job);
            },
        );
        assert_eq!((grown, pool.into_inner().unwrap()), (2, vec![11, 12]));
    }
}
