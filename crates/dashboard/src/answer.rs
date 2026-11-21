
use crate::Source;
use dashboard_core::config::LoadContext;
use serde_json::Value;
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

pub const SERIES_PRODUCTS: &[&str] = &[
    "series-state",
    "series-summary",
    "series-next-change",
    "events-page",
    dashboard_query::tile::PRODUCT,
    dashboard_query::series::PARTS,
];

pub use dashboard_base::queries::{index, lexicon, lifetimes, links, query, spellings, warm_lexicons};

pub fn forget(source: &Source) {
    dashboard_base::queries::forget(source);
    *source.state::<Runs>().lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Runs::default();
    crate::products::wars::forget(source);
    crate::products::leaders::forget(source);
    crate::products::feed::scope::forget(source);
    crate::products::air_ledger::forget(source);
}

#[derive(Default)]
struct Runs {
    looks: std::collections::HashMap<u32, Arc<crate::products::styles::RunLooks>>,
    groups: std::collections::HashMap<u32, Arc<crate::products::groups::RunGroups>>,
    territory: std::collections::HashMap<u32, Arc<Vec<(u128, i64, i64)>>>,
    army_mix: std::collections::HashMap<u32, Arc<Vec<(u128, i64, i64)>>>,
    subjects: std::collections::HashMap<u32, Arc<crate::products::subjects::RunSubjects>>,
    pacts: std::collections::HashMap<u32, Arc<crate::products::diplomacy::RunPacts>>,
    production_lines: std::collections::HashMap<u32, Arc<crate::products::production::RunLines>>,
    convoys: std::collections::HashMap<u32, Arc<crate::products::convoys::RunConvoys>>,
    construction: std::collections::HashMap<u32, Arc<crate::products::construction::RunLines>>,
    division_templates: std::collections::HashMap<u32, Arc<Vec<crate::products::division_log::SeenTemplate>>>,
    wars: std::collections::HashMap<u32, Arc<crate::products::wars::RunWars>>,
    casualty_ends: std::collections::HashMap<u32, Arc<crate::products::casualties::EndedCounters>>,
}

fn held<T>(source: &Source, pick: impl Fn(&mut Runs) -> &mut std::collections::HashMap<u32, Arc<T>>, run: u32, read: impl FnOnce() -> io::Result<T>) -> io::Result<Arc<T>> {
    let state = source.state::<Runs>();
    if let Some(found) = pick(&mut state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)).get(&run) {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(read()?);
    pick(&mut state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)).insert(run, Arc::clone(&made));
    Ok(made)
}

pub fn looks(source: &Source, run: u32) -> io::Result<Arc<crate::products::styles::RunLooks>> {
    held(source, |runs| &mut runs.looks, run, || crate::products::styles::run_looks(source.replay_ref().recording(), run))
}

pub fn groups(source: &Source, run: u32) -> io::Result<Arc<crate::products::groups::RunGroups>> {
    held(source, |runs| &mut runs.groups, run, || crate::products::groups::run_groups(source.replay_ref().recording(), run))
}

pub fn territory(source: &Source, run: u32) -> io::Result<Arc<Vec<(u128, i64, i64)>>> {
    held(source, |runs| &mut runs.territory, run, || crate::products::land_owners::holding(source.replay_ref().recording(), run))
}

pub fn army_mix(source: &Source, run: u32) -> io::Result<Arc<Vec<(u128, i64, i64)>>> {
    held(source, |runs| &mut runs.army_mix, run, || {
        dashboard_query::cost::lifetimes(source.replay_ref().recording(), run, replay_data::rows::TableId::ArmyMix)
    })
}

const KEPT_LIFETIMES: [replay_data::rows::TableId; 9] = [
    replay_data::rows::TableId::AirForceParts,
    replay_data::rows::TableId::AirForceWingKinds,
    replay_data::rows::TableId::NavyClasses,
    replay_data::rows::TableId::MilitaryAid,
    replay_data::rows::TableId::OperativeMemberships,
    replay_data::rows::TableId::Raids,
    replay_data::rows::TableId::CasualtyLifetimes,
    replay_data::rows::TableId::UnitLeaders,
    replay_data::rows::TableId::ProductionTypeTotals,
];



pub fn subjects(source: &Source, run: u32) -> io::Result<Arc<crate::products::subjects::RunSubjects>> {
    held(source, |runs| &mut runs.subjects, run, || crate::products::subjects::run_subjects(source.replay_ref().recording(), run))
}

pub fn pacts(source: &Source, run: u32) -> io::Result<Arc<crate::products::diplomacy::RunPacts>> {
    held(source, |runs| &mut runs.pacts, run, || crate::products::diplomacy::run_pacts(source.replay_ref().recording(), run))
}

pub fn production_lines(source: &Source, run: u32) -> io::Result<Arc<crate::products::production::RunLines>> {
    held(source, |runs| &mut runs.production_lines, run, || crate::products::production::run_lines(source.replay_ref().recording(), run))
}

#[derive(Clone)]
pub struct ProductionIndex {
    replay: Arc<replay::campaign::Replay>,
    state: Arc<std::sync::Mutex<Runs>>,
}

impl ProductionIndex {
    pub fn of(source: &Source) -> Self {
        Self { replay: source.replay(), state: source.state::<Runs>() }
    }

    pub fn run(&self, run: u32) -> io::Result<Arc<crate::products::production::RunLines>> {
        if let Some(found) = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).production_lines.get(&run) {
            return Ok(Arc::clone(found));
        }
        let made = Arc::new(crate::products::production::run_lines(self.replay.recording(), run)?);
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).production_lines.insert(run, Arc::clone(&made));
        Ok(made)
    }
}

#[derive(Clone)]
pub struct ConvoyIndex {
    replay: Arc<replay::campaign::Replay>,
    state: Arc<std::sync::Mutex<Runs>>,
}

impl ConvoyIndex {
    pub fn of(source: &Source) -> Self {
        Self { replay: source.replay(), state: source.state::<Runs>() }
    }

    pub fn run(&self, run: u32) -> io::Result<Arc<crate::products::convoys::RunConvoys>> {
        if let Some(found) = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).convoys.get(&run) {
            return Ok(Arc::clone(found));
        }
        let made = Arc::new(crate::products::convoys::run_convoys(self.replay.recording(), run)?);
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).convoys.insert(run, Arc::clone(&made));
        Ok(made)
    }
}

pub fn convoys(source: &Source, run: u32) -> io::Result<Arc<crate::products::convoys::RunConvoys>> {
    held(source, |runs| &mut runs.convoys, run, || crate::products::convoys::run_convoys(source.replay_ref().recording(), run))
}

pub fn construction(source: &Source, run: u32) -> io::Result<Arc<crate::products::construction::RunLines>> {
    held(source, |runs| &mut runs.construction, run, || crate::products::construction::run_lines(source, run))
}

pub fn division_templates(source: &Source, run: u32) -> io::Result<Arc<Vec<crate::products::division_log::SeenTemplate>>> {
    held(source, |runs| &mut runs.division_templates, run, || crate::products::division_log::run_templates(source.replay_ref().recording(), run))
}

pub fn wars(source: &Source, run: u32) -> io::Result<Arc<crate::products::wars::RunWars>> {
    held(source, |runs| &mut runs.wars, run, || crate::products::wars::run_wars(source.replay_ref().recording(), run))
}

pub fn casualty_ends(source: &Source, run: u32) -> io::Result<Arc<crate::products::casualties::EndedCounters>> {
    held(source, |runs| &mut runs.casualty_ends, run, || crate::products::casualties::run_ends(source, run))
}

pub fn open_all(source: &Source) -> io::Result<()> {
    if let Some(timelines) = source.replay_ref().timelines() {
        for timeline in &timelines.list {
            open(source, timeline.id)?;
        }
    }
    Ok(())
}

type Part<'a> = Box<dyn FnOnce() -> io::Result<()> + Send + 'a>;

fn build_all(parts: Vec<Part<'_>>) -> io::Result<()> {
    let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get()).min(parts.len());
    if threads <= 1 {
        return parts.into_iter().try_for_each(|part| part());
    }
    let queue = std::sync::Mutex::new(parts.into_iter());
    let next = || queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).next();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                job_priority::spawn_in(scope, || {
                    while let Some(part) = next() {
                        part()?;
                    }
                    Ok(())
                })
            })
            .collect();
        workers.into_iter().map(|worker| worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect::<io::Result<Vec<()>>>().map(drop)
    })
}

fn run_parts(source: &Source, run: u32) -> Vec<Part<'_>> {
    let mut parts: Vec<Part<'_>> = vec![
        Box::new(move || looks(source, run).map(drop)),
        Box::new(move || groups(source, run).map(drop)),
        Box::new(move || territory(source, run).map(drop)),
        Box::new(move || army_mix(source, run).map(drop)),
        Box::new(move || links(source, run).map(drop)),
        Box::new(move || subjects(source, run).map(drop)),
        Box::new(move || pacts(source, run).map(drop)),
        Box::new(move || production_lines(source, run).map(drop)),
        Box::new(move || convoys(source, run).map(drop)),
        Box::new(move || construction(source, run).map(drop)),
        Box::new(move || crate::products::economy::market::history::run_ended(source, run).map(drop)),
        Box::new(move || wars(source, run).map(drop)),
        Box::new(move || casualty_ends(source, run).map(drop)),
    ];
    parts.extend(KEPT_LIFETIMES.map(|table| -> Part<'_> { Box::new(move || lifetimes(source, run, table).map(drop)) }));
    parts
}

pub fn open(source: &Source, branch: u64) -> io::Result<()> {
    let query = query(source, branch)?;
    let mut runs: Vec<u32> = query.timeline.stretches.iter().map(|stretch| stretch.run).collect();
    runs.sort_unstable();
    runs.dedup();
    build_all(runs.into_iter().flat_map(|run| run_parts(source, run)).collect())?;
    build_all(vec![
        Box::new(|| crate::products::air_aces::history(source, branch).map(drop)),
        Box::new(|| crate::products::air_superiority::index::index(source, branch).map(drop)),
        Box::new(|| crate::products::intelligence::decryption::open(source, branch)),
    ])
}


fn kind(params: &Value) -> Option<&str> {
    params.get("family")?.get("kind")?.as_str()
}

pub fn answers(product: &str, params: &Value) -> bool {
    if follows(product) {
        return true;
    }
    if SERIES_PRODUCTS.contains(&product) {
        return kind(params).is_some_and(|kind| {
            dashboard_declare::series(kind).is_some() || crate::products::army::SERIES == kind
                || (product == dashboard_query::tile::PRODUCT && crate::products::player_apm::head::SERIES == kind)
        });
    }
    crate::products::MOVED.contains(&product)
}

pub fn panel(source: &Source, context: &LoadContext, query: &Value) -> Option<Result<Box<dyn dashboard_follow::panel::Panel>, String>> {
    use crate::products::at_hour::{dated_panel_of, panel_of};
    dated_panel_of(source, context, query).or_else(|| panel_of(source, context, query))
}

pub fn follows(product: &str) -> bool {
    dashboard_follow::registry::PRODUCTS.contains(&product)
}

pub fn answer(source: &Source, product: &str, params: &Value, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Value> {
    crate::metadata::check_cancel(cancelled)?;
    if follows(product) {
        return dashboard_follow::registry::registry().answer_heads(
            source.path(),
            product,
            params,
            |branch| {
                let query = query(source, branch)?;
                Ok(dashboard_follow::Identity { holders: Arc::clone(&query.holders), timeline: Arc::clone(&query.timeline) })
            },
            Box::new(|asked| panel(source, context, asked)),
            Box::new(|asked| crate::products::player_apm::head::head_of(source, asked)),
        );
    }
    if SERIES_PRODUCTS.contains(&product) {
        let branch = dashboard_query::branch(params)?;
        let query = query(source, branch)?;
        if kind(params) == Some(crate::products::player_apm::head::SERIES) {
            return Ok(serde_json::to_value(crate::products::player_apm::head::tile(source, branch, query.timeline.last, params)?)?);
        }
        if kind(params) == Some(crate::products::army::SERIES) {
            if product == dashboard_query::tile::PRODUCT {
                let line = crate::products::army::tile_line(&query, params)?;
                return Ok(serde_json::to_value(dashboard_query::tile::reply(&line, &dashboard_query::branch(params)?.to_string(), params)?)?);
            }
            return crate::products::army::series(source, &query, product, params, context);
        }
        if product == "series-next-change" && kind(params) == Some(crate::products::scalars::world_tension::SERIES) {
            return crate::products::scalars::world_tension::next_change(&query, params);
        }
        return dashboard_query::series::answer(&query, product, params)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("No declared series answers {product}")));
    }
    let registered = crate::products::find(product).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("Unknown data product: {product}")))?;
    let text = (registered.load)(source, "answer", params.clone(), context, cancelled).map_err(io::Error::other)?;
    let mut reply: Value = serde_json::from_str(&text)?;
    Ok(reply.get_mut("data").map(Value::take).unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn an_open_builds_every_part_once_and_fails_with_a_part_that_fails() {
        let built = AtomicUsize::new(0);
        let parts: Vec<Part<'_>> = (0..40)
            .map(|_| -> Part<'_> {
                Box::new(|| {
                    built.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                })
            })
            .collect();
        build_all(parts).unwrap();
        assert_eq!(built.load(Ordering::Relaxed), 40);
        let failing: Vec<Part<'_>> = vec![Box::new(|| Ok(())), Box::new(|| Err(io::Error::other("a history could not be read"))), Box::new(|| Ok(()))];
        assert_eq!(build_all(failing).unwrap_err().to_string(), "a history could not be read");
        build_all(Vec::new()).unwrap();
    }
}
