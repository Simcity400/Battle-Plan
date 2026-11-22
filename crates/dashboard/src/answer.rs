
use dashboard_base::{Source, queries::{lifetimes, links, query}};
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

pub fn forget(source: &Source) {
    dashboard_base::queries::forget(source);
    dashboard_world::forget(source);
    dashboard_land::forget(source);
    dashboard_wars::forget(source);
    dashboard_production::forget(source);
    dashboard_economy::forget(source);
    dashboard_politics::forget(source);
    dashboard_air::forget(source);
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
        Box::new(move || dashboard_world::products::styles::cached_looks(source, run).map(drop)),
        Box::new(move || dashboard_world::products::groups::cached_groups(source, run).map(drop)),
        Box::new(move || dashboard_world::products::land_owners::cached_territory(source, run).map(drop)),
        Box::new(move || dashboard_land::products::army::cached_army_mix(source, run).map(drop)),
        Box::new(move || links(source, run).map(drop)),
        Box::new(move || dashboard_world::products::subjects::cached_subjects(source, run).map(drop)),
        Box::new(move || dashboard_wars::products::diplomacy::cached_pacts(source, run).map(drop)),
        Box::new(move || dashboard_production::products::production::cached_production_lines(source, run).map(drop)),
        Box::new(move || dashboard_economy::products::convoys::cached_convoys(source, run).map(drop)),
        Box::new(move || dashboard_production::products::construction::cached_construction(source, run).map(drop)),
        Box::new(move || dashboard_economy::products::economy::market::history::run_ended(source, run).map(drop)),
        Box::new(move || dashboard_wars::products::wars::cached_wars(source, run).map(drop)),
        Box::new(move || dashboard_wars::products::casualties::cached_casualty_ends(source, run).map(drop)),
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
        Box::new(|| dashboard_air::products::air_aces::history(source, branch).map(drop)),
        Box::new(|| dashboard_air::products::air_superiority::index::index(source, branch).map(drop)),
        Box::new(|| dashboard_intelligence::products::intelligence::decryption::open(source, branch)),
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
            dashboard_declare::series(kind).is_some() || dashboard_land::products::army::SERIES == kind
                || (product == dashboard_query::tile::PRODUCT && dashboard_players::products::player_apm::head::SERIES == kind)
        });
    }
    crate::products::MOVED.contains(&product)
}

pub fn panel(source: &Source, context: &LoadContext, query: &Value) -> Option<Result<Box<dyn dashboard_follow::panel::Panel>, String>> {
    use crate::panel::{dated_panel_of, panel_of};
    dated_panel_of(source, context, query).or_else(|| panel_of(source, context, query))
}

pub fn follows(product: &str) -> bool {
    dashboard_follow::registry::PRODUCTS.contains(&product)
}

pub fn answer(source: &Source, product: &str, params: &Value, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Value> {
    dashboard_text::metadata::check_cancel(cancelled)?;
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
            Box::new(|asked| dashboard_players::products::player_apm::head::head_of(source, asked)),
        );
    }
    if SERIES_PRODUCTS.contains(&product) {
        let branch = dashboard_query::branch(params)?;
        let query = query(source, branch)?;
        if kind(params) == Some(dashboard_players::products::player_apm::head::SERIES) {
            return Ok(serde_json::to_value(dashboard_players::products::player_apm::head::tile(source, branch, query.timeline.last, params)?)?);
        }
        if kind(params) == Some(dashboard_land::products::army::SERIES) {
            if product == dashboard_query::tile::PRODUCT {
                let line = dashboard_land::products::army::tile_line(&query, params)?;
                return Ok(serde_json::to_value(dashboard_query::tile::reply(&line, &dashboard_query::branch(params)?.to_string(), params)?)?);
            }
            return dashboard_land::products::army::series(source, &query, product, params, context);
        }
        if product == "series-next-change" && kind(params) == Some(dashboard_world::products::scalars::world_tension::SERIES) {
            return dashboard_world::products::scalars::world_tension::next_change(&query, params);
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
