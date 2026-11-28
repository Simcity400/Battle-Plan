use dashboard_base::{Source, catalogue::{self, Catalogue}, products::{LoadContext, Product}, queries};
use dashboard_naval::products::naval::battles::{battle_id, tail as naval};
use dashboard_land::products::land_battles::tail as land;
use dashboard_query::{Query, read::Keys};
use dashboard_world::products::place_names::{Content, PlaceHistory};
use dashboard_text::metadata::check_cancel;
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, HashMap}, io, sync::{Arc, OnceLock, PoisonError, atomic::AtomicBool}};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Land(u64, i32, Option<i32>),
    Naval(u64, i32),
    Report(u64),
}

impl Key {
    fn id(self) -> String {
        match self {
            Self::Land(id, start, None) => format!("battle:{id}:{start}"),
            Self::Land(id, start, Some(written)) => format!("battle:{id}:{start}:{written}"),
            Self::Naval(id, start) => battle_id(id, start),
            Self::Report(id) => format!("result:{id}"),
        }
    }
}

struct Battle {
    key: Key,
    date: i32,
    observed: i32,
    province: Option<u32>,
    port_strike: bool,
    reports: Vec<u64>,
    live_hours: Option<(i32, i32)>,
}

pub type Title = Result<Arc<str>, Arc<str>>;

#[derive(Default)]
pub struct Names {
    titles: HashMap<String, Title>,
    live: BTreeMap<(u64, i32), (i32, Title)>,
}

impl Names {
    pub fn get(&self, id: &str) -> Option<&Title> {
        self.titles.get(id)
    }

    pub fn naval(&self, combat: u64, hour: i32) -> Option<&Title> {
        let (&(id, _), (last, title)) = self.live.range(..=(combat, hour)).next_back()?;
        (id == combat && hour <= *last).then_some(title)
    }

    fn install(&mut self, battle: Battle, title: Title) {
        if let Key::Naval(combat, _) = battle.key && let Some((first, last)) = battle.live_hours {
            self.live.insert((combat, first), (last, title.clone()));
        }
        for report in battle.reports {
            self.titles.insert(format!("result:{report}"), title.clone());
        }
        self.titles.insert(battle.key.id(), title);
    }
}

fn ordinal(rank: usize) -> String {
    const WORDS: [&str; 10] = ["First", "Second", "Third", "Fourth", "Fifth", "Sixth", "Seventh", "Eighth", "Ninth", "Tenth"];
    if let Some(word) = WORDS.get(rank - 1) { return (*word).into(); }
    let suffix = match (rank % 100, rank % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{rank}{suffix}")
}

fn number(battles: Vec<(Battle, Result<String, String>)>) -> Names {
    let mut names = Names::default();
    let mut groups = HashMap::<_, Vec<_>>::new();
    for (battle, title) in battles {
        match title {
            Ok(title) => groups.entry(title).or_default().push(battle),
            Err(reason) => names.install(battle, Err(reason.into())),
        }
    }
    for (base, mut group) in groups {
        group.sort_unstable_by_key(|battle| (battle.date, battle.key));
        let repeated = group.len() > 1;
        for (i, battle) in group.into_iter().enumerate() {
            let title = if repeated { format!("{} {base}", ordinal(i + 1)).into() } else { Arc::from(base.as_str()) };
            names.install(battle, Ok(title));
        }
    }
    names
}

fn join(source: &Source, query: &Query<'_>, content: &Content, cancelled: &AtomicBool) -> io::Result<Names> {
    let mut battles: Vec<_> = land::battles(query, cancelled)?.into_iter().map(|battle| Battle {
        key: Key::Land(battle.combat, battle.start, battle.written), date: battle.start, observed: battle.observed, province: battle.province,
        reports: Vec::new(), port_strike: false, live_hours: None,
    }).collect();
    battles.extend(naval::battles(source, query, cancelled)?.into_iter().map(|battle| Battle {
        key: match battle.battle { naval::Battle::Live(id, start) => Key::Naval(id, start), naval::Battle::Result(id, _) => Key::Report(id) },
        date: battle.date, observed: battle.observed, province: battle.province, reports: battle.reports,
        port_strike: battle.port_strike, live_hours: battle.live_hours,
    }));
    battles.sort_unstable_by_key(|battle| battle.observed);
    let mut pending = battles.into_iter().peekable();
    let mut joined = Vec::with_capacity(pending.len());
    for span in &query.timeline.stretches {
        check_cancel(cancelled)?;
        let local: Vec<_> = std::iter::from_fn(|| pending.next_if(|battle| battle.observed <= span.last)).collect();
        if local.is_empty() { continue; }
        let naval_provinces: Vec<_> = local.iter().filter_map(|battle| {
            (!battle.port_strike && !matches!(battle.key, Key::Land(..))).then_some(battle.province).flatten()
        }).collect();
        let naval = !naval_provinces.is_empty();
        let mut provinces: Vec<_> = local.iter().filter_map(|battle| battle.province.map(u128::from)).collect();
        if naval { provinces.extend(content.naval_candidate_provinces(naval_provinces).into_iter().map(u128::from)); }
        provinces.sort_unstable();
        provinces.dedup();
        let first = span.first.max(local[0].observed);
        let last = local.last().map_or(first, |battle| battle.observed).max(first);
        let history = PlaceHistory::load(query.replay.recording(), span.run, first, last, Keys::These(&provinces), Keys::These(&[]), naval, cancelled)?;
        for battle in local {
            let title = match (battle.province, battle.key) {
                (None, _) => Err("Battle province not recorded".into()),
                (Some(province), Key::Land(..)) => history.land_battle_title(content, province, battle.observed),
                (Some(province), _) => history.naval_battle_title(content, province, battle.port_strike, battle.observed),
            }.and_then(|title| catalogue::plain_text(&title))
                .and_then(|title| if title.trim().is_empty() { Err("Battle title is empty".into()) } else { Ok(title.trim().into()) });
            joined.push((battle, title));
        }
    }
    joined.extend(pending.map(|battle| {
        let reason = query.timeline.reason(battle.observed);
        (battle, Err(reason))
    }));
    Ok(number(joined))
}

#[derive(Default)]
struct Cached {
    catalogue: Option<Arc<Catalogue>>,
    branches: HashMap<u64, Arc<Names>>,
}

type Build = Arc<OnceLock<io::Result<Arc<Names>>>>;

#[derive(Default)]
struct Ahead(u64, HashMap<u64, Build>);

pub fn ahead(source: &Arc<Source>, branch: u64, load: &LoadContext) -> io::Result<Option<Arc<Names>>> {
    let Some(content) = &load.content else { return read(source, branch, load, &false.into()).map(Some) };
    let generation = content.presentation.binding().generation;
    let state = source.state::<Ahead>();
    let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
    if held.0 != generation { *held = Ahead(generation, HashMap::new()); }
    if let std::collections::hash_map::Entry::Vacant(slot) = held.1.entry(branch) {
        let build = Build::default();
        let (source, load, pending) = (Arc::downgrade(source), LoadContext { content: Some(content.clone()) }, Arc::clone(&build));
        dashboard_base::products::read_ahead::spawn("battle titles ahead", move || {
            job_priority::background_thread();
            let Some(source) = source.upgrade() else { return };
            let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.in_scope(|| read(&source, branch, &load, &false.into()))))
                .unwrap_or_else(|_| Err(io::Error::other("Battle title preparation failed")));
            let _ = pending.set(made);
        })?;
        slot.insert(build);
    }
    result(&held.1[&branch])
}

fn result(build: &Build) -> io::Result<Option<Arc<Names>>> {
    build.get().map(|made| made.as_ref().map(Arc::clone).map_err(dashboard_core::duplicate)).transpose()
}

pub fn read(source: &Source, branch: u64, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<Names>> {
    let mut recorded = LoadContext { content: load.content.clone() };
    if let Some(options) = &mut recorded.content {
        options.presentation = dashboard_core::presentation::Admitted::recorded(Arc::clone(options.presentation.binding()));
    }
    let content = Content::read(source, &recorded)?;
    let cache = source.state::<Cached>();
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if cache.catalogue.as_ref().is_none_or(|held| !Arc::ptr_eq(held, &content.cat)) {
        cache.branches.clear();
        cache.catalogue = Some(Arc::clone(&content.cat));
    }
    if let Some(names) = cache.branches.get(&branch) { return Ok(Arc::clone(names)); }
    let names = Arc::new(join(source, &queries::query(source, branch)?, &content, cancelled)?);
    cache.branches.insert(branch, Arc::clone(&names));
    Ok(names)
}

#[derive(Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BattleNamesParams {
    branch_id: String,
    battles: Vec<String>,
}

#[derive(Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct BattleName {
    name: Option<String>,
    missing: Option<String>,
}

#[derive(Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct BattleNamesReply {
    titles: BTreeMap<String, BattleName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    loading: Option<String>,
    warnings: Vec<String>,
}

dashboard_base::reply_warnings!(BattleNamesReply);

pub struct BattleNames;

impl Product for BattleNames {
    const KEY: &'static str = "battle-names";
    const CONTENT_DEPENDENT: bool = true;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, "battles[]"];
    type Params = BattleNamesParams;
    type Reply = BattleNamesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![BattleNamesParams::decl(), BattleName::decl(), BattleNamesReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid battle names branch"))?;
        let state = source.state::<Ahead>();
        let build = load.content.as_ref().and_then(|content| {
            let held = state.lock().unwrap_or_else(PoisonError::into_inner);
            (held.0 == content.presentation.binding().generation).then(|| held.1.get(&branch).cloned()).flatten()
        });
        let Some(names) = build.as_ref().map_or_else(|| read(source, branch, load, cancelled).map(Some), result)? else {
            return Ok(BattleNamesReply { titles: BTreeMap::new(), loading: Some("Loading battle titles…".into()), warnings: Vec::new() });
        };
        let titles = params.battles.iter().map(|key| {
            let title = match names.get(key) {
                Some(Ok(name)) => BattleName { name: Some(name.to_string()), missing: None },
                Some(Err(reason)) => BattleName { name: None, missing: Some(reason.to_string()) },
                None => BattleName { name: None, missing: Some("Battle identity not recorded".into()) },
            };
            (key.clone(), title)
        }).collect();
        Ok(BattleNamesReply { titles, loading: None, warnings: Vec::new() })
    }
}

dashboard_base::register_products! {
    BATTLE_NAMES => BattleNames,
    @hourly
}
