
use dashboard_base::{Source, queries::{lifetimes, links, query}};
use dashboard_core::config::LoadContext;
use serde_json::Value;
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

pub const SERIES_PRODUCTS: &[&str] = &[
    "series-state",
    dashboard_query::series::ranked::PRODUCT,
    "series-next-change",
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
    dashboard_intelligence::forget(source);
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
                || (product == dashboard_query::tile::PRODUCT && dashboard_production::products::stockpile::lines::SERIES == kind)
                || (product == dashboard_query::tile::PRODUCT && dashboard_air::products::air_ledger::tiles::SERIES == kind)
                || (product == dashboard_query::tile::PRODUCT && dashboard_naval::products::naval::losses::tiles::SERIES == kind)
                || (product == dashboard_query::tile::PRODUCT && dashboard_wars::products::factions::tiles::SERIES == kind)
                || (product == dashboard_query::tile::PRODUCT && dashboard_production::products::equipment_produced::tiles::SERIES == kind)
                || (product == dashboard_query::tile::PRODUCT && dashboard_world::products::autonomy_lines::SERIES == kind)
        });
    }
    crate::products::in_viewer(product)
}

pub fn follows(product: &str) -> bool {
    dashboard_follow::registry::PRODUCTS.contains(&product)
}

pub fn answer(source: &Arc<Source>, product: &str, params: &Value, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Value> {
    let check_cancel = || dashboard_text::metadata::check_cancel(cancelled);
    check_cancel()?;
    if follows(product) {
        return dashboard_follow::registry::for_session(params).answer_heads(
            source.path(),
            product,
            params,
            |branch| {
                let query = query(source, branch)?;
                Ok(dashboard_follow::Identity { holders: Arc::clone(&query.holders), timeline: Arc::clone(&query.timeline) })
            },
            Box::new(|asked| crate::panel::open(source, context, asked)),
            Box::new(|asked| dashboard_players::products::player_apm::head::head_of(source, asked)),
        );
    }
    if SERIES_PRODUCTS.contains(&product) {
        let branch = dashboard_query::branch(params)?;
        let query = query(source, branch)?;
        if kind(params) == Some(dashboard_players::products::player_apm::head::SERIES) {
            return Ok(serde_json::to_value(dashboard_players::products::player_apm::head::tile(source, branch, query.timeline.last, params)?)?);
        }
        if kind(params) == Some(dashboard_production::products::stockpile::lines::SERIES) {
            if product != dashboard_query::tile::PRODUCT {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "The stockpile over time is served as tiles and its at-hour lines"));
            }
            return Ok(serde_json::to_value(dashboard_production::products::stockpile::lines::tile(source, &query, params, context, cancelled)?)?);
        }
        if kind(params) == Some(dashboard_air::products::air_ledger::tiles::SERIES) {
            if product != dashboard_query::tile::PRODUCT {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "Air losses over time are served as tiles and military-air-at"));
            }
            return Ok(serde_json::to_value(dashboard_air::products::air_ledger::tiles::tile(source, &query, params, context, cancelled)?)?);
        }
        if kind(params) == Some(dashboard_naval::products::naval::losses::tiles::SERIES) {
            if product != dashboard_query::tile::PRODUCT {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "Naval losses over time are served as tiles and naval-losses-at"));
            }
            return Ok(serde_json::to_value(dashboard_naval::products::naval::losses::tiles::tile(source, &query, params, context, cancelled)?)?);
        }
        if kind(params) == Some(dashboard_wars::products::factions::tiles::SERIES) {
            if product != dashboard_query::tile::PRODUCT {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "Faction group totals over time are served as tiles and faction-totals-at"));
            }
            return Ok(serde_json::to_value(dashboard_wars::products::factions::tiles::tile(source, &query, params, context, cancelled)?)?);
        }
        if kind(params) == Some(dashboard_production::products::equipment_produced::tiles::SERIES) {
            if product != dashboard_query::tile::PRODUCT {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "Equipment produced over time is served as tiles and equipment-produced-race"));
            }
            return Ok(serde_json::to_value(dashboard_production::products::equipment_produced::tiles::tile(source, &query, params, context, cancelled)?)?);
        }
        if kind(params) == Some(dashboard_world::products::autonomy_lines::SERIES) {
            if product != dashboard_query::tile::PRODUCT {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "Autonomy over time is served as tiles and autonomy-levels"));
            }
            return Ok(serde_json::to_value(dashboard_world::products::autonomy_lines::tile(source, &query, params, cancelled)?)?);
        }
        if kind(params) == Some(dashboard_land::products::army::SERIES) {
            if product == dashboard_query::tile::PRODUCT {
                let line = dashboard_land::products::army::tile_line(&query, params)?;
                return Ok(serde_json::to_value(dashboard_query::tile::reply(&line, &dashboard_query::branch(params)?.to_string(), params, &check_cancel)?)?);
            }
            return dashboard_land::products::army::series(source, &query, product, params, context);
        }
        if product == "series-next-change" && kind(params) == Some(dashboard_world::products::scalars::world_tension::SERIES) {
            return dashboard_world::products::scalars::world_tension::next_change(&query, params);
        }
        if product == dashboard_query::series::ranked::PRODUCT && params.get("group").is_some_and(|group| !group.is_null()) {
            let groups = dashboard_world::products::group_members::GroupsAt::for_query(source, &query, context, cancelled)?;
            let resolve = |group: &dashboard_query::series::ranked::GroupSelection, hour: i32| groups.slots_at(group, &query.holders, hour);
            return dashboard_query::series::answer_grouped(&query, product, params, &resolve, &check_cancel)?
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("No declared series answers {product}")));
        }
        return dashboard_query::series::answer(&query, product, params, &check_cancel)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("No declared series answers {product}")));
    }
    let registered = crate::products::find(product).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("Unknown data product: {product}")))?;
    registered.answer(source, params.clone(), context, cancelled)
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

    #[test]
    fn forgetting_the_dashboard_clears_every_cache() {
        use dashboard_air::products::air_ledger;
        use dashboard_base::{SourceKey, queries, test_fixtures::{replay, source}};
        use dashboard_economy::products::convoys;
        use dashboard_land::products::{army, division_log, leaders::LeadersContext};
        use dashboard_politics::products::feed::index as feed_index;
        use dashboard_production::products::{construction, production};
        use dashboard_wars::products::{casualties, diplomacy, wars};
        use dashboard_world::products::{groups, land_owners, styles, subjects};

        fn warm<'a, T: 'a>(name: impl std::fmt::Display + 'a, read: impl Fn() -> Arc<T> + 'a) -> Box<dyn FnOnce() + 'a> {
            let first = read();
            assert!(Arc::ptr_eq(&first, &read()), "{name} should be warm");
            Box::new(move || assert!(!Arc::ptr_eq(&first, &read()), "{name} should be cold"))
        }

        let hour = 100;
        let branch = 1;
        let frames = [hour, hour + 1].map(|hour| source(branch, hour, None));
        let replay = replay(frames.into()).into_timelines(&Default::default()).unwrap();
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            SourceKey { source_id: "forget".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &cancelled,
        ).unwrap();
        let source = Arc::new(source);
        let run = query(&source, branch).unwrap().timeline.stretches[0].run;
        let load = LoadContext { content: None };
        let leaders = LeadersContext::load(&source, &load).unwrap();
        leaders.keep_test_verdict(run, hour);
        answer(&source, "wars", &serde_json::json!({ "branchId": branch.to_string() }), &load, &cancelled).unwrap();
        queries::spellings(&source, run, hour, &std::collections::BTreeSet::from([7])).unwrap();
        answer(&source, "division-log-tail", &serde_json::json!({ "branchId": branch.to_string(), "limit": 1 }), &load, &cancelled).unwrap();
        feed_index::warm(&source, branch).unwrap();
        air_ledger::warm_observed(&source, branch).unwrap();

        let caches = [
            warm("query index", || queries::index(&source)),
            warm("links", || links(&source, run).unwrap()),
            warm("lexicon", || queries::lexicon(&source, run).unwrap()),
            warm("country looks", || styles::cached_looks(&source, run).unwrap()),
            warm("country groups", || groups::cached_groups(&source, run).unwrap()),
            warm("territory", || land_owners::cached_territory(&source, run).unwrap()),
            warm("subjects", || subjects::cached_subjects(&source, run).unwrap()),
            warm("army mix", || army::cached_army_mix(&source, run).unwrap()),
            warm("wars", || wars::cached_wars(&source, run).unwrap()),
            warm("diplomatic pacts", || diplomacy::cached_pacts(&source, run).unwrap()),
            warm("casualty endings", || casualties::cached_casualty_ends(&source, run).unwrap()),
            warm("production lines", || production::cached_production_lines(&source, run).unwrap()),
            warm("construction", || construction::cached_construction(&source, run).unwrap()),
            warm("convoys", || convoys::cached_convoys(&source, run).unwrap()),
        ];
        let histories = KEPT_LIFETIMES.map(|table| {
            let source = &source;
            warm(format!("{table:?}"), move || lifetimes(source, run, table).unwrap())
        });
        assert!(queries::has_cached_spellings(&source), "spellings should be warm");
        assert!(leaders.has_cached_verdicts(run, hour), "leader verdicts should be warm");
        assert!(wars::has_cached_walks(&source), "war walks should be warm");
        assert!(division_log::is_cached(&source, branch));
        assert_eq!(feed_index::kept(&source), [branch], "feed index should be warm");
        assert_eq!(air_ledger::kept_observed(&source).len(), 2, "both air ledgers should be warm");

        forget(&source);

        assert!(!queries::has_cached_spellings(&source), "spellings should be cold");
        assert!(!leaders.has_cached_verdicts(run, hour), "leader verdicts should be cold");
        assert!(!wars::has_cached_walks(&source), "war walks should be cold");
        assert!(!division_log::is_cached(&source, branch));
        assert!(feed_index::kept(&source).is_empty(), "feed index should be cold");
        assert!(air_ledger::kept_observed(&source).is_empty(), "air ledgers should be cold");
        for check in caches.into_iter().chain(histories) {
            check();
        }
    }
}
