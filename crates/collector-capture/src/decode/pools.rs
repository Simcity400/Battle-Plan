use super::{
    AirBasesJob, AirStatsJob, AirWingJob, CHARACTER_PARTS, CapitulationJob, CharactersJob,
    ConstructionRestJob, DivisionsJob, EconomyRestJob, IntelJob, LexicalJob, NavalJob, PlayersJob,
    ProductionRestJob, ProjectsJob, RadarJob, RelationsJob, RetiredPayload, SessionJob, SunkJob,
    SupremacyJob, WeatherJob, defer_payload,
};
use std::sync::Mutex;

pub(super) const JOB_POOL_STEADY: usize = 2;
const JOB_POOL_MAX: usize = 8;

fn keep() -> usize {
    let shared = crate::sampling::SHARED.load(std::sync::atomic::Ordering::Acquire);
    let backlog = if shared.is_null() {
        0
    } else {
        // SAFETY: the loader publishes SHARED only once its state is built, which the null check
        // above covers, and keeps that state alive until every worker that returns jobs here has
        // stopped.
        unsafe { &*shared }
            .frames
            .live()
            .saturating_sub(crate::frame_pool::STEADY)
    };
    (JOB_POOL_STEADY + backlog).min(JOB_POOL_MAX)
}

fn retain<J>(pool: &mut Vec<J>, job: J) {
    let keep = keep();
    if pool.len() < keep && engine_allocation::reserve(pool, 1).is_ok() {
        pool.push(job);
    }
    pool.truncate(keep);
}

pub(super) static RELATIONS_JOB_POOL: Mutex<Vec<RelationsJob>> = Mutex::new(Vec::new());
pub(super) static CHARACTERS_JOB_POOL: Mutex<[Vec<CharactersJob>; CHARACTER_PARTS]> =
    Mutex::new([const { Vec::new() }; CHARACTER_PARTS]);
pub(super) static PROJECTS_JOB_POOL: Mutex<Vec<ProjectsJob>> = Mutex::new(Vec::new());
pub(super) static LEXICAL_JOB_POOL: Mutex<Vec<LexicalJob>> = Mutex::new(Vec::new());
pub(super) static DIVISIONS_JOB_POOL: Mutex<Vec<DivisionsJob>> = Mutex::new(Vec::new());
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
pub(super) static SUPREMACY_JOB_POOL: Mutex<Vec<SupremacyJob>> = Mutex::new(Vec::new());
pub(super) static SUNK_JOB_POOL: Mutex<Vec<SunkJob>> = Mutex::new(Vec::new());
pub(super) static RADAR_JOB_POOL: Mutex<Vec<RadarJob>> = Mutex::new(Vec::new());
pub(super) static INTEL_JOB_POOL: Mutex<Vec<IntelJob>> = Mutex::new(Vec::new());
pub(super) static WEATHER_JOB_POOL: Mutex<Vec<WeatherJob>> = Mutex::new(Vec::new());
pub(super) static CAPITULATION_JOB_POOL: Mutex<Vec<CapitulationJob>> = Mutex::new(Vec::new());

pub(crate) fn acquire_relations_job() -> RelationsJob {
    if let Ok(mut pool) = RELATIONS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    RelationsJob::default()
}

pub(crate) fn recycle_relations_job(job: RelationsJob) {
    defer_payload(RetiredPayload::Relations(job));
}

pub(super) fn release_relations_job(mut job: RelationsJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    job.return_opinions();
    if !job.is_allocated() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = RELATIONS_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_characters_job(part: usize) -> CharactersJob {
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.try_lock()
        && let Some(job) = pool[part].pop()
    {
        return job;
    }
    CharactersJob::vacant(part)
}

pub(crate) fn recycle_characters_job(job: CharactersJob) {
    defer_payload(RetiredPayload::Characters(job));
}

pub(super) fn release_characters_job(mut job: CharactersJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if !job.has_storage() {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    job.prepare_read_plans();
    if collector_memory::terminal::failed() {
        return;
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.lock() {
        let part = job.part;
        retain(&mut pool[part], job);
    }
}

pub(crate) fn acquire_projects_job() -> ProjectsJob {
    if let Ok(mut pool) = PROJECTS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ProjectsJob::default()
}

pub(crate) fn recycle_projects_job(job: ProjectsJob) {
    defer_payload(RetiredPayload::Projects(job));
}

pub(super) fn release_projects_job(mut job: ProjectsJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("Projects", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    job.presize_countries();
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = PROJECTS_JOB_POOL.lock() {
        retain(&mut pool, job);
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

pub(crate) fn acquire_lexical_job() -> LexicalJob {
    pop_job(&LEXICAL_JOB_POOL)
}

pub(crate) fn recycle_lexical_job(job: LexicalJob) {
    defer_payload(RetiredPayload::Lexical(job));
}

pub(super) fn release_lexical_job(mut job: LexicalJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if !job.has_storage() {
        return;
    }
    job.note_growth();
    job.clear();
    if let Ok(mut pool) = LEXICAL_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_divisions_job() -> DivisionsJob {
    pop_job(&DIVISIONS_JOB_POOL)
}

pub(crate) fn recycle_divisions_job(job: DivisionsJob) {
    defer_payload(RetiredPayload::Divisions(job));
}

pub(super) fn release_divisions_job(mut job: DivisionsJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if !job.initialized() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = DIVISIONS_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

/// # Safety
/// Only the attaching runtime or serial sizing owner may grow the live decode pools, with storage
/// kept alive and conflicting teardown excluded.
pub unsafe fn presize_air_wing_names() {
    if let Ok(mut pool) = AIR_WING_JOB_POOL.lock() {
        for job in pool.iter_mut() {
            if job.grow_for_next() {
                job.grow();
            }
        }
    }
}

pub(crate) fn acquire_air_wing_job() -> AirWingJob {
    pop_job(&AIR_WING_JOB_POOL)
}

pub(crate) fn recycle_air_wing_job(job: AirWingJob) {
    defer_payload(RetiredPayload::AirWing(job));
}

pub(super) fn release_air_wing_job(mut job: AirWingJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if !job.has_storage() && !job.grow_for_next() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.presize_game_score();
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = AIR_WING_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_air_stats_job() -> AirStatsJob {
    pop_job(&AIR_STATS_JOB_POOL)
}

pub(crate) fn recycle_air_stats_job(job: AirStatsJob) {
    defer_payload(RetiredPayload::AirStats(job));
}

pub(super) fn release_air_stats_job(mut job: AirStatsJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("AirStats", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = AIR_STATS_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_naval_job() -> NavalJob {
    if let Ok(mut pool) = NAVAL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    NavalJob::default()
}

pub(crate) fn recycle_naval_job(job: NavalJob) {
    defer_payload(RetiredPayload::Naval(job));
}

pub(super) fn release_naval_job(mut job: NavalJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("Naval", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    job.presize_countries();
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = NAVAL_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_players_job() -> PlayersJob {
    if let Ok(mut pool) = PLAYERS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    PlayersJob::default()
}

pub(crate) fn recycle_players_job(job: PlayersJob) {
    defer_payload(RetiredPayload::Players(job));
}

pub(super) fn release_players_job(mut job: PlayersJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("Players", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = PLAYERS_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_session_job() -> SessionJob {
    if let Ok(mut pool) = SESSION_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SessionJob::default()
}

pub(crate) fn recycle_session_job(job: SessionJob) {
    defer_payload(RetiredPayload::Session(job));
}

pub(super) fn release_session_job(mut job: SessionJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("Session", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = SESSION_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_production_rest_job() -> ProductionRestJob {
    if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ProductionRestJob::default()
}

pub(crate) fn recycle_production_rest_job(job: ProductionRestJob) {
    defer_payload(RetiredPayload::ProductionRest(job));
}

pub(super) fn release_production_rest_job(mut job: ProductionRestJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("ProductionRest", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    job.log.prepare_read_plan();
    if collector_memory::terminal::failed() {
        return;
    }
    job.presize_countries();
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_construction_rest_job() -> ConstructionRestJob {
    if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ConstructionRestJob::default()
}

pub(crate) fn recycle_construction_rest_job(job: ConstructionRestJob) {
    defer_payload(RetiredPayload::ConstructionRest(job));
}

pub(super) fn release_construction_rest_job(mut job: ConstructionRestJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("ConstructionRest", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    job.presize_countries();
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_economy_rest_job() -> EconomyRestJob {
    if let Ok(mut pool) = ECONOMY_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    EconomyRestJob::default()
}

pub(crate) fn recycle_economy_rest_job(job: EconomyRestJob) {
    defer_payload(RetiredPayload::EconomyRest(job));
}

pub(super) fn release_economy_rest_job(mut job: EconomyRestJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("EconomyRest", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = ECONOMY_REST_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_air_bases_job() -> AirBasesJob {
    if let Ok(mut pool) = AIR_BASES_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirBasesJob::default()
}

pub(crate) fn recycle_air_bases_job(job: AirBasesJob) {
    defer_payload(RetiredPayload::AirBases(job));
}

pub(super) fn release_air_bases_job(mut job: AirBasesJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("AirBases", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    job.presize_countries();
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = AIR_BASES_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_supremacy_job() -> SupremacyJob {
    if let Ok(mut pool) = SUPREMACY_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SupremacyJob::default()
}

pub(crate) fn recycle_supremacy_job(job: SupremacyJob) {
    defer_payload(RetiredPayload::Supremacy(job));
}

pub(super) fn release_supremacy_job(mut job: SupremacyJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("Supremacy", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = SUPREMACY_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_sunk_job() -> SunkJob {
    if let Ok(mut pool) = SUNK_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SunkJob::default()
}

pub(crate) fn recycle_sunk_job(job: SunkJob) {
    defer_payload(RetiredPayload::Sunk(job));
}

pub(super) fn release_sunk_job(mut job: SunkJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("Sunk", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = SUNK_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_radar_job() -> RadarJob {
    if let Ok(mut pool) = RADAR_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    RadarJob::default()
}

pub(crate) fn recycle_radar_job(job: RadarJob) {
    defer_payload(RetiredPayload::Radar(job));
}

pub(super) fn release_radar_job(mut job: RadarJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.allocated_bytes() == 0 {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = RADAR_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_intel_job() -> IntelJob {
    if let Ok(mut pool) = INTEL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    IntelJob::default()
}

pub(crate) fn recycle_intel_job(job: IntelJob) {
    defer_payload(RetiredPayload::Intel(job));
}

pub(super) fn release_intel_job(mut job: IntelJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.allocated_bytes() == 0 {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = INTEL_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_weather_job() -> WeatherJob {
    if let Ok(mut pool) = WEATHER_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    WeatherJob::default()
}

/// # Safety
/// Only the attaching runtime or serial sizing owner may grow the live decode pools, with storage
/// kept alive and conflicting teardown excluded.
pub unsafe fn presize_weather_jobs() {
    if let Ok(mut pool) = WEATHER_JOB_POOL.lock() {
        for job in pool.iter_mut().filter(|job| job.grow_for_next()) {
            job.grow();
        }
    }
}

pub(crate) fn recycle_weather_job(job: WeatherJob) {
    defer_payload(RetiredPayload::Weather(job));
}

pub(super) fn release_weather_job(mut job: WeatherJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("Weather", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = WEATHER_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

pub(crate) fn acquire_capitulation_job() -> CapitulationJob {
    if let Ok(mut pool) = CAPITULATION_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    CapitulationJob::default()
}

pub(crate) fn recycle_capitulation_job(job: CapitulationJob) {
    defer_payload(RetiredPayload::Capitulation(job));
}

pub(super) fn release_capitulation_job(mut job: CapitulationJob) {
    if collector_memory::terminal::failed() {
        return;
    }
    if job.log.spans.capacity() == 0 {
        return;
    }
    collector_raw::raw::note_growth("Capitulation", &job.log);
    if job.grow_for_next() {
        job.grow();
        if collector_memory::terminal::failed() {
            return;
        }
    }
    if collector_memory::terminal::failed() {
        return;
    }
    job.clear();
    if let Ok(mut pool) = CAPITULATION_JOB_POOL.lock() {
        retain(&mut pool, job);
    }
}

/// # Safety
/// Only the attaching runtime or serial sizing owner may grow the live decode pools, with storage
/// kept alive and conflicting teardown excluded.
pub unsafe fn presize_country_pools() {
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
        if collector_memory::terminal::failed() {
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
