
use super::{COST_SCALE, Content, ConstructionStateName, ConstructionType, ProjectKind, RunLines, SpendingGap, col, gap_of, hour, id32, missing, number};
use dashboard_base::{Source, products::{LoadContext, Product}, queries::country_ids::{Names, RunLinks}};
use dashboard_base::products::{BranchParams, battles::BattleWindow};
use dashboard_query::series::MissingSpan;
use replay_lanes::lanes::{Cell, Keys, Step};
use replay_data::{
    codec::Decoder,
    rows::{ConstructionProjectRow as P, ConstructionRise, TableId},
    value::Value,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ConstructionLogManifest;
pub struct ConstructionLog;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConstructionLogParams {
    pub branch_id: String,
    #[doc = " Index into [`ConstructionLogManifestReply::windows`]."]
    pub window: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EndKind {
    Finished,
    Removed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum UnknownEnd {
    NotRecorded,
    SharedTarget,
    NothingLeft,
    CompletionNotRecorded,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectEnd {
    pub kind: EndKind,
    #[doc = " The native outcome hour, when observed; otherwise only `known_at` bounds it."]
    pub hour: Option<i32>,
    #[doc = " The read that showed the line gone, or that settled its ending."]
    pub known_at: i32,
    pub unknown: Option<UnknownEnd>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LevelsBuilt {
    #[doc = " The hour the building rose; null when only the read shows it."]
    pub hour: Option<i32>,
    #[doc = " The read that established it."]
    pub known_at: i32,
    pub count: u32,
    #[doc = " Levels repaired (or repair work), not new levels."]
    pub repair: bool,
    #[doc = " The count reached before the window: known by `known_at`, its own reads in earlier windows."]
    pub carried: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub factories: i32,
    pub remaining: i32,
    pub repair_levels: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConstructionOutputRun {
    pub from: i32,
    pub to: i32,
    #[doc = " Fixed5 factory output since observation began; null when not known (`reason`)."]
    pub construction_raw: Option<String>,
    pub repair_raw: Option<String>,
    pub reason: Option<SpendingGap>,
    #[doc = " Output before observation began is not recorded."]
    pub before_observation: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputOrigin {
    pub tag: String,
    pub building: Option<u32>,
    pub state: Option<u32>,
    pub to_state: Option<u32>,
    pub province: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProjectSource {
    Census,
    OutputAlone,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConstructionProject {
    #[doc = " The line's native handle, as a decimal string."]
    pub id: String,
    pub kind: ProjectKind,
    pub source: ProjectSource,
    pub census_read: Option<i32>,
    pub output_origin: Option<OutputOrigin>,
    pub queue_ordinal: Option<u32>,
    #[doc = " The queue owner's tag at the line's first read."]
    pub tag: String,
    #[doc = " The building type's lexer id ([`ConstructionType::id`])."]
    pub building: Option<u32>,
    #[doc = " It had levels to repair at the first read the window shows of it."]
    pub repair: Option<bool>,
    pub state: Option<u32>,
    pub to_state: Option<u32>,
    pub province: Option<u32>,
    pub route_provinces: Option<u32>,
    pub base_level: Option<i32>,
    pub target_level: Option<i32>,
    #[doc = " Fixed5 cost per level (railway: the whole route)."]
    pub cost_raw: Option<String>,
    pub created_hour: Option<i32>,
    pub first_read: i32,
    #[doc = " Work before its first read was not recorded; its counts are lower bounds."]
    pub earlier_unknown: bool,
    pub runs: Vec<ProjectRun>,
    #[doc = " New levels built and levels repaired (`repair`)."]
    pub built: Vec<LevelsBuilt>,
    #[doc = " Levels that cannot be told built or dropped."]
    pub unaccounted: Vec<LevelsBuilt>,
    #[doc = " New levels it may have both built and had added."]
    pub possible: Vec<LevelsBuilt>,
    #[doc = " Levels added while under way (more queued, or new damage)."]
    pub added: Vec<LevelsBuilt>,
    #[doc = " Null while it is in the queue at the window's last hour."]
    pub end: Option<ProjectEnd>,
    #[doc = " The first output read counting a repair advance."]
    pub first_repair_tick: Option<i32>,
    pub outputs: Vec<ConstructionOutputRun>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConstructionLogManifestReply {
    #[doc = " Units per factory output point of `cost_raw`."]
    pub cost_scale: u32,
    #[doc = " The branch's blocks: the log's windows, in hour order."]
    pub windows: Vec<BattleWindow>,
    #[doc = " Either construction queue was read in the branch."]
    pub recorded: bool,
    pub types: Vec<ConstructionType>,
    #[doc = " The construction queue's repair marker."]
    pub repair_icon: Option<String>,
    #[doc = " Hours of the branch the recording does not hold."]
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConstructionLogReply {
    pub window: usize,
    pub interval: BattleWindow,
    #[doc = " Every line whose row was shown in the window, by first read, then handle."]
    pub projects: Vec<ConstructionProject>,
    #[doc = " The names of the states its lines name."]
    pub states: Vec<ConstructionStateName>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ConstructionLogManifestReply, ConstructionLogReply);

fn branch(id: &str) -> io::Result<u64> {
    id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

impl Product for ConstructionLogManifest {
    const KEY: &'static str = "construction-log-manifest";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = ConstructionLogManifestReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![BranchParams::decl(), BattleWindow::decl(), ConstructionType::decl(), ConstructionLogManifestReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let windows = super::windows(&query)?;
        let mut tokens = BTreeMap::new();
        for stretch in &query.timeline.stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let lines = crate::products::construction::cached_construction(source, stretch.run)?;
            let ids: BTreeSet<u32> = lines.within(stretch.first, stretch.last).filter_map(|line| line.building).collect();
            super::spellings(&query, stretch.run, stretch.last, &ids, &mut tokens)?;
        }
        let mut warnings = Vec::new();
        let named = Content::load(source, context).name(source, &tokens, &BTreeSet::new(), &mut warnings);
        let reply = ConstructionLogManifestReply {
            cost_scale: COST_SCALE,
            windows: windows.into_iter().map(|(_, window)| window).collect(),
            recorded: super::recorded(&query)?,
            types: named.types,
            repair_icon: named.repair_icon,
            missing: missing(&query, query.timeline.first, query.timeline.last),
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

impl Product for ConstructionLog {
    const KEY: &'static str = "construction-log";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ConstructionLogParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, window];
    type Reply = ConstructionLogReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ConstructionLogParams::decl(),
            ConstructionStateName::decl(),
            ProjectKind::decl(),
            ProjectSource::decl(),
            SpendingGap::decl(),
            EndKind::decl(),
            UnknownEnd::decl(),
            ProjectEnd::decl(),
            LevelsBuilt::decl(),
            ProjectRun::decl(),
            ConstructionOutputRun::decl(),
            OutputOrigin::decl(),
            ConstructionProject::decl(),
            ConstructionLogReply::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let windows = super::windows(&query)?;
        let (run, interval) = windows.get(params.window).map(|(run, window)| (*run, window.clone())).ok_or_else(|| dashboard_core::invalid("No such construction log window"))?;
        let lines = crate::products::construction::cached_construction(source, run)?;
        let links = dashboard_base::queries::links(source, run)?;
        let names = Names::new(&query.holders);
        let mut warnings = Vec::new();
        let projects = window(source, run, interval.first_hour, interval.last_hour, &lines, &names, &links, &mut warnings)?;
        let states: BTreeSet<u32> = projects
            .iter()
            .flat_map(|project| [project.state, project.to_state, project.output_origin.as_ref().and_then(|origin| origin.state), project.output_origin.as_ref().and_then(|origin| origin.to_state)])
            .flatten()
            .collect();
        let states = if states.is_empty() { Vec::new() } else { Content::load(source, context).name(source, &BTreeMap::new(), &states, &mut warnings).states };
        let reply = ConstructionLogReply { window: params.window, interval, projects, states, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[allow(clippy::too_many_arguments, reason = "one window's read and the identity it names lines by")]
fn window(source: &Source, run: u32, from: i32, to: i32, lines: &RunLines, names: &Names, links: &RunLinks, warnings: &mut Vec<String>) -> io::Result<Vec<ConstructionProject>> {
    let recording = source.replay_ref().recording();
    let mut keys: Vec<u128> = lines.within(from, to).map(|line| u128::from(line.id)).collect();
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    keys.sort_unstable();
    let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(recording, run, TableId::ConstructionProjects, &col::ALL, Keys::These(&keys), from, to))?;
    let mut asks = Vec::new();
    for (key, steps) in &history.keys {
        asks.extend(rise_hours(steps).into_iter().map(|hour| (hour, *key)));
    }
    let mut rises: HashMap<(u128, i32), Rises> = HashMap::new();
    if !asks.is_empty() {
        dashboard_query::cost::storage(|| {
            replay_lanes::lanes::at_each(recording, run, TableId::ConstructionProjects, Some(&[col::BUILT_RISES, col::REPAIRED_RISES][..]), &asks, &mut |hour, key, row| {
                if let Some((view, slot)) = row {
                    let list = |column: usize| -> io::Result<Vec<ConstructionRise>> {
                        let Some(bytes) = view.list(column, slot) else { return Ok(Vec::new()) };
                        let mut input = Decoder::new(bytes);
                        let rises = <Vec<ConstructionRise> as Value>::decode(&mut input)?;
                        input.finish()?;
                        Ok(rises)
                    };
                    rises.insert((key, hour), Rises { built: list(col::BUILT_RISES)?, repaired: list(col::REPAIRED_RISES)? });
                }
                Ok(())
            })
        })?;
    }
    let mut unnamed = 0usize;
    let mut out = Vec::new();
    for (key, steps) in &history.keys {
        let tag = |country: u32, at: i32| names.tag_at(links, country, at).map(str::to_owned);
        match assemble(*key as u64, steps, history.to, &|hour| rises.get(&(*key, hour)), &tag) {
            Assembled::Project(project) => out.push(*project),
            Assembled::Unnamed => unnamed += 1,
            Assembled::Hidden => {}
        }
    }
    if unnamed > 0 {
        warnings.push(format!("{unnamed} construction line{} in this window name no owner the recording resolves; they are left out.", if unnamed == 1 { "" } else { "s" }));
    }
    out.sort_by(|a, b| (a.first_read, a.kind, a.id.len(), &a.id).cmp(&(b.first_read, b.kind, b.id.len(), &b.id)));
    Ok(out)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rises {
    pub built: Vec<ConstructionRise>,
    pub repaired: Vec<ConstructionRise>,
}

fn shown<'a>(steps: &'a [Step], to: i32) -> Vec<(i32, i32, &'a [Cell])> {
    steps
        .iter()
        .enumerate()
        .filter_map(|(at, step)| {
            let cells = step.cells.as_deref()?;
            let last = steps.get(at + 1).map_or(to, |next| next.hour - 1);
            Some((step.hour, last, cells))
        })
        .collect()
}

pub(crate) fn rise_hours(steps: &[Step]) -> Vec<i32> {
    let mut out = Vec::new();
    let mut held: Option<(i64, i64)> = None;
    for step in steps {
        let Some(cells) = step.cells.as_deref() else { continue };
        let now = (number(cells, col::BUILT).unwrap_or(0), number(cells, col::REPAIRED).unwrap_or(0));
        if let Some(before) = held
            && (now.0 > before.0 || now.1 > before.1)
        {
            out.push(step.hour);
        }
        held = Some(now);
    }
    out
}

pub(crate) enum Assembled {
    Project(Box<ConstructionProject>),
    Unnamed,
    Hidden,
}

fn end_of(cells: &[Cell], at: i32) -> Option<ProjectEnd> {
    let kind = match u8::try_from(number(cells, col::END)?).ok()? {
        P::FINISHED => EndKind::Finished,
        P::REMOVED => EndKind::Removed,
        P::UNKNOWN_END => EndKind::Unknown,
        _ => return None,
    };
    let unknown = match number(cells, col::END_REASON).and_then(|reason| u8::try_from(reason).ok()) {
        Some(P::NOT_RECORDED) => Some(UnknownEnd::NotRecorded),
        Some(P::SHARED_TARGET) => Some(UnknownEnd::SharedTarget),
        Some(P::NOTHING_LEFT) => Some(UnknownEnd::NothingLeft),
        Some(P::COMPLETION_NOT_RECORDED) => Some(UnknownEnd::CompletionNotRecorded),
        _ => None,
    };
    Some(ProjectEnd { kind, hour: hour(cells, col::END_HOUR), known_at: hour(cells, col::END_KNOWN_AT).unwrap_or(at), unknown })
}

fn rises_of(shown: &[(i32, i32, &[Cell])], column: usize, repair: bool, dated: &dyn Fn(i32) -> Option<Vec<ConstructionRise>>) -> Vec<LevelsBuilt> {
    let mut out = Vec::new();
    let mut held: Option<i64> = None;
    for &(at, _, cells) in shown {
        let now = number(cells, column).unwrap_or(0);
        match held {
            None if now > 0 => out.push(LevelsBuilt { hour: None, known_at: at, count: u32::try_from(now).unwrap_or(u32::MAX), repair, carried: true }),
            Some(before) if now > before => {
                let delta = u32::try_from(now - before).unwrap_or(u32::MAX);
                let list = dated(at).unwrap_or_default();
                if !list.is_empty() && list.iter().map(|rise| u64::from(rise.count)).sum::<u64>() == u64::from(delta) {
                    out.extend(list.iter().map(|rise| LevelsBuilt { hour: Some(rise.hour), known_at: at, count: rise.count, repair, carried: false }));
                } else {
                    out.push(LevelsBuilt { hour: None, known_at: at, count: delta, repair, carried: false });
                }
            }
            _ => {}
        }
        held = Some(held.map_or(now, |before| before.max(now)));
    }
    out
}

fn merged(mut a: Vec<LevelsBuilt>, b: Vec<LevelsBuilt>) -> Vec<LevelsBuilt> {
    a.extend(b);
    a.sort_by_key(|levels| (levels.known_at, !levels.carried, levels.repair, levels.hour));
    a
}

pub(crate) fn assemble<'r>(
    id: u64,
    steps: &[Step],
    to: i32,
    rises: &dyn Fn(i32) -> Option<&'r Rises>,
    tag: &dyn Fn(u32, i32) -> Option<String>,
) -> Assembled {
    let shown = shown(steps, to);
    let (Some(&(_, _, first)), Some(&(last_at, _, last))) = (shown.first(), shown.last()) else { return Assembled::Hidden };
    let first_read = hour(last, col::FIRST_READ).unwrap_or(shown[0].0);
    let country = id32(last, col::COUNTRY).unwrap_or(0);
    let Some(owner) = tag(country, first_read) else { return Assembled::Unnamed };
    let alone = |cells: &[Cell]| number(cells, col::OUTPUT_ALONE) == Some(1);
    let output_origin = (alone(first) && !alone(last)).then(|| OutputOrigin {
        tag: id32(first, col::COUNTRY).and_then(|country| tag(country, first_read)).unwrap_or_else(|| owner.clone()),
        building: id32(first, col::BUILDING_TYPE),
        state: id32(first, col::TARGET_STATE),
        to_state: id32(first, col::TO_STATE),
        province: id32(first, col::PROVINCE),
    });
    let census = !alone(last);
    let open = |cells: &[Cell]| number(cells, col::END).is_none_or(|end| end == i64::from(P::OPEN));
    let mut runs: Vec<ProjectRun> = Vec::new();
    for &(at, through, cells) in &shown {
        if alone(cells) || !open(cells) {
            continue;
        }
        let (Some(factories), Some(remaining), Some(repair_levels)) = (hour(cells, col::FACTORIES), hour(cells, col::REMAINING), hour(cells, col::REPAIR_LEVELS)) else { continue };
        match runs.last_mut() {
            Some(run) if run.last_hour + 1 == at && (run.factories, run.remaining, run.repair_levels) == (factories, remaining, repair_levels) => run.last_hour = through,
            _ => runs.push(ProjectRun { first_hour: at, last_hour: through, factories, remaining, repair_levels }),
        }
    }
    let mut outputs: Vec<ConstructionOutputRun> = Vec::new();
    for &(at, through, cells) in &shown {
        let since = hour(cells, col::OUTPUT_SINCE);
        let reason = number(cells, col::OUTPUT_REASON).and_then(|reason| u8::try_from(reason).ok()).and_then(gap_of);
        if since.is_none() && reason.is_some_and(|reason| reason == SpendingGap::NotRecorded) {
            continue;
        }
        let raw = |column: usize| if reason.is_none() { number(cells, column).map(|value| value.to_string()) } else { None };
        let run = ConstructionOutputRun {
            from: at,
            to: through,
            construction_raw: raw(col::CONSTRUCTION_RAW),
            repair_raw: raw(col::REPAIR_RAW),
            reason,
            before_observation: since.is_some() && number(cells, col::OBSERVED_SINCE_CREATION) == Some(0),
        };
        match outputs.last_mut() {
            Some(held)
                if held.to + 1 == at
                    && (&held.construction_raw, &held.repair_raw, held.reason, held.before_observation)
                        == (&run.construction_raw, &run.repair_raw, run.reason, run.before_observation) =>
            {
                held.to = through
            }
            _ => outputs.push(run),
        }
    }
    let built = |at: i32| rises(at).map(|rises| rises.built.clone());
    let repaired = |at: i32| rises(at).map(|rises| rises.repaired.clone());
    let undated = |_: i32| -> Option<Vec<ConstructionRise>> { None };
    let census_first = shown.iter().find(|(_, _, cells)| !alone(cells)).map(|(_, _, cells)| *cells);
    Assembled::Project(Box::new(ConstructionProject {
        id: id.to_string(),
        kind: if number(last, col::KIND) == Some(i64::from(P::RAILWAY)) { ProjectKind::Railway } else { ProjectKind::Building },
        source: if census { ProjectSource::Census } else { ProjectSource::OutputAlone },
        census_read: hour(last, col::CENSUS_READ),
        output_origin,
        queue_ordinal: id32(last, col::QUEUE_ORDINAL),
        tag: owner,
        building: id32(last, col::BUILDING_TYPE),
        repair: census_first.map(|cells| number(cells, col::REPAIR_LEVELS).is_some_and(|levels| levels > 0)),
        state: id32(last, col::TARGET_STATE),
        to_state: id32(last, col::TO_STATE),
        province: id32(last, col::PROVINCE),
        route_provinces: id32(last, col::ROUTE_PROVINCES),
        base_level: hour(last, col::BASE_LEVEL),
        target_level: hour(last, col::TARGET_LEVEL),
        cost_raw: number(last, col::COST).map(|cost| cost.to_string()),
        created_hour: hour(last, col::CREATED_HOUR),
        first_read,
        earlier_unknown: number(last, col::EARLIER_UNKNOWN) == Some(1),
        runs,
        built: merged(rises_of(&shown, col::BUILT, false, &built), rises_of(&shown, col::REPAIRED, true, &repaired)),
        unaccounted: merged(rises_of(&shown, col::UNACCOUNTED_NEW, false, &undated), rises_of(&shown, col::UNACCOUNTED_REPAIR, true, &undated)),
        possible: rises_of(&shown, col::POSSIBLE, false, &undated),
        added: merged(rises_of(&shown, col::ADDED_NEW, false, &undated), rises_of(&shown, col::ADDED_REPAIR, true, &undated)),
        end: end_of(last, last_at),
        first_repair_tick: hour(last, col::FIRST_REPAIR_TICK),
        outputs,
    }))
}
