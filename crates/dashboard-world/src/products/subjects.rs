
use dashboard_base::{products::{BranchParams, LoadContext, Product}, queries::country_ids::{Names, RunLinks}};
use dashboard_base::{Source, catalogue::{self, Catalogue}, span::HourSpan};
use dashboard_query::{read, timeline::Stretch};
use content_reader_politics::autonomy::AutonomyStates;
use replay_recording::columnar_frames::Recording;
use replay_lanes::lanes;
use replay_columnar::summary::{Cells, Source as Column};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const START: usize = 0;
const END: usize = 1;
const CANCEL: usize = 2;
const STATE: usize = 4;

pub struct Subjects;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SubjectLevel {
    #[doc = " The autonomy state's id, as recorded."]
    pub token: String,
    #[doc = " The game's name for it; null when the localisation has none or is not read."]
    pub name: Option<String>,
    #[doc = " Whether the content defines the state and orders it (by its `min_freedom_level`); a level"]
    #[doc = " it does not is listed after those it does."]
    pub ordered: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LevelStep {
    pub first_hour: i32,
    pub last_hour: i32,
    pub level: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SubjectRun {
    pub overlord: String,
    pub subject: String,
    #[doc = " The hour the relation began, as the engine dates it (may be before the timeline)."]
    pub start_hour: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The subject's level through the run, in hour order, covering every hour of it."]
    pub steps: Vec<LevelStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SubjectReply {
    pub branch_id: String,
    #[doc = " Every level the runs name, least free first in the content's order."]
    pub levels: Vec<SubjectLevel>,
    #[doc = " Every run, by first hour, then overlord, subject and start hour."]
    pub runs: Vec<SubjectRun>,
    #[doc = " Hours inside the timeline whose relations were not observed: a run that ends or starts"]
    #[doc = " next to one of these is a gap, not an end or a start."]
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(SubjectReply);

fn relation(cells: &dyn Cells, slot: u32) -> replay_columnar::summary::Computed {
    let value = (|| {
        if cells.number(CANCEL, slot)? != 0 {
            return None;
        }
        let (start, end) = (cells.number(START, slot)?, cells.number(END, slot)?);
        Some(start << 32 | i64::from(end as u32))
    })();
    replay_columnar::summary::Computed::of(value)
}

fn relation_column() -> Column {
    Column::Computed { id: 0, inputs: 1 << START | 1 << END | 1 << CANCEL, value: relation }
}

fn split(cell: i64) -> (i32, i32) {
    ((cell >> 32) as i32, cell as i32)
}

#[derive(Default)]
pub struct RunSubjects {
    relations: Vec<(u32, u32, read::Runs)>,
    levels: HashMap<u32, Vec<(i32, i32, Option<String>)>>,
}

impl RunSubjects {
    pub fn relation_at(&self, overlord: u32, subject: u32, hour: i32) -> Option<(i32, Option<String>)> {
        let (_, _, runs) = self.relations.iter().find(|(a, b, _)| *a == overlord && *b == subject)?;
        let at = runs.partition_point(|run| run.1 < hour);
        let cell = runs.get(at).filter(|run| run.0 <= hour)?.2?;
        let level = self.levels.get(&subject)
            .and_then(|levels| levels.iter().find(|level| level.0 <= hour && hour <= level.1))
            .and_then(|level| level.2.clone());
        Some((split(cell).0, level))
    }
}

pub fn run_subjects(recording: &Recording, run: u32) -> io::Result<RunSubjects> {
    let held = recording.run(run)?;
    let (first, last) = (held.first_hour, held.first_hour + held.ordinals as i32 - 1);
    let mut relations: Vec<(u32, u32, read::Runs)> = read::runs(recording, run, TableId::Subjects, relation_column(), None, first, last)?
        .into_iter()
        .filter(|(_, runs)| runs.iter().any(|run| run.2.is_some()))
        .map(|(key, mut runs)| {
            runs.sort_unstable_by_key(|run| run.0);
            ((key >> 32) as u32, key as u32, runs)
        })
        .collect();
    relations.sort_unstable_by_key(|(overlord, subject, _)| (*overlord, *subject));
    let mut subjects: Vec<u128> = relations.iter().map(|(_, subject, _)| u128::from(*subject)).collect();
    subjects.sort_unstable();
    subjects.dedup();
    if subjects.is_empty() {
        return Ok(RunSubjects { relations, levels: HashMap::new() });
    }
    let history = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::Autonomy, &[STATE], lanes::Keys::These(&subjects), first, last))?;
    Ok(RunSubjects { relations, levels: level_runs(&history) })
}

fn level_runs(history: &lanes::History) -> HashMap<u32, Vec<(i32, i32, Option<String>)>> {
    let mut levels = HashMap::new();
    for (key, runs) in history.runs(STATE).unwrap_or_default() {
        let mut out: Vec<(i32, i32, Option<String>)> = Vec::with_capacity(runs.len());
        for (first, last, cell) in runs {
            let level = cell.and_then(|cell| history.text(cell)).and_then(|bytes| std::str::from_utf8(bytes).ok()).map(str::to_owned);
            match out.last_mut() {
                Some(held) if held.2 == level => held.1 = last,
                _ => out.push((first, last, level)),
            }
        }
        levels.insert(key as u32, out);
    }
    levels
}

fn push_step(steps: &mut Vec<LevelStep>, first_hour: i32, last_hour: i32, level: Option<String>) {
    match steps.last_mut() {
        Some(held) if held.level == level && i64::from(held.last_hour) + 1 == i64::from(first_hour) => held.last_hour = last_hour,
        _ => steps.push(LevelStep { first_hour, last_hour, level }),
    }
}

fn steps(levels: Option<&Vec<(i32, i32, Option<String>)>>, first: i32, last: i32) -> Vec<LevelStep> {
    let mut out = Vec::new();
    let mut at = i64::from(first);
    for (a, b, level) in levels.map(Vec::as_slice).unwrap_or_default() {
        let (a, b) = ((*a).max(first), (*b).min(last));
        if a > b {
            continue;
        }
        if i64::from(a) > at {
            push_step(&mut out, at as i32, a - 1, None);
        }
        push_step(&mut out, a, b, level.clone());
        at = i64::from(b) + 1;
    }
    if at <= i64::from(last) {
        push_step(&mut out, at as i32, last, None);
    }
    out
}

fn fold(stretches: &[Stretch], held: &HashMap<u32, (Arc<RunSubjects>, Arc<RunLinks>)>, names: &Names) -> (Vec<SubjectRun>, Vec<HourSpan>) {
    let mut runs: Vec<SubjectRun> = Vec::new();
    let mut unobserved: Vec<HourSpan> = Vec::new();
    for stretch in stretches {
        let Some((subjects, links)) = held.get(&stretch.run) else { continue };
        for (overlord, subject, cells) in &subjects.relations {
            let mut previous: Option<(i32, i32)> = None;
            let start = cells.partition_point(|run| run.1 < stretch.first);
            for &(a, b, cell) in cells[start..].iter().take_while(|run| run.0 <= stretch.last) {
                let Some(cell) = cell else { continue };
                let (began, end) = split(cell);
                let (a, mut b) = (a.max(stretch.first), b.min(stretch.last));
                if end > began {
                    b = b.min(end.saturating_sub(1));
                }
                if a > b {
                    continue;
                }
                if let Some((held_start, held_last)) = previous
                    && held_start == began
                    && i64::from(held_last) + 1 < i64::from(a)
                {
                    unobserved.push(HourSpan { first_hour: held_last + 1, last_hour: a - 1 });
                }
                previous = Some((began, b));
                for (overlord_tag, oa, ob) in names.tags_over(links, *overlord, a, b) {
                    for (subject_tag, sa, sb) in names.tags_over(links, *subject, oa, ob) {
                        match (overlord_tag, subject_tag) {
                            (Some(overlord_tag), Some(subject_tag)) => runs.push(SubjectRun {
                                overlord: overlord_tag.to_owned(),
                                subject: subject_tag.to_owned(),
                                start_hour: began,
                                first_hour: sa,
                                last_hour: sb,
                                steps: steps(subjects.levels.get(subject), sa, sb),
                            }),
                            _ => unobserved.push(HourSpan { first_hour: sa, last_hour: sb }),
                        }
                    }
                }
            }
        }
    }
    runs.sort_by(|a, b| (&a.overlord, &a.subject, a.start_hour, a.first_hour).cmp(&(&b.overlord, &b.subject, b.start_hour, b.first_hour)));
    let mut joined: Vec<SubjectRun> = Vec::with_capacity(runs.len());
    for run in runs {
        match joined.last_mut() {
            Some(held)
                if (&held.overlord, &held.subject, held.start_hour) == (&run.overlord, &run.subject, run.start_hour)
                    && i64::from(held.last_hour) + 1 == i64::from(run.first_hour) =>
            {
                held.last_hour = run.last_hour;
                for step in run.steps {
                    push_step(&mut held.steps, step.first_hour, step.last_hour, step.level);
                }
            }
            _ => joined.push(run),
        }
    }
    sort_runs(&mut joined);
    (joined, unobserved)
}

fn sort_runs(runs: &mut [SubjectRun]) {
    runs.sort_by(|a, b| (a.first_hour, &a.overlord, &a.subject, a.start_hour).cmp(&(b.first_hour, &b.overlord, &b.subject, b.start_hour)));
}

pub fn settle(mut spans: Vec<HourSpan>) -> Vec<HourSpan> {
    spans.sort_unstable_by_key(|span| span.first_hour);
    let mut out: Vec<HourSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(held) if i64::from(held.last_hour) + 1 >= i64::from(span.first_hour) => held.last_hour = held.last_hour.max(span.last_hour),
            _ => out.push(span),
        }
    }
    out
}

pub fn levels(named: &BTreeSet<&str>, states: Option<&AutonomyStates>, catalogue: Option<&Catalogue>) -> Vec<SubjectLevel> {
    let name = |token: &str| catalogue.and_then(|catalogue| catalogue.localised(token, catalogue::View::Plain));
    let mut result = Vec::new();
    let mut placed = BTreeSet::new();
    if let Some(states) = states {
        for state in states.ordered().placed {
            if named.contains(state.id.as_str()) {
                placed.insert(state.id.as_str());
                result.push(SubjectLevel { token: state.id.clone(), name: name(&state.id), ordered: true });
            }
        }
    }
    for token in named.iter().filter(|token| !placed.contains(*token)) {
        result.push(SubjectLevel { token: (*token).to_owned(), name: name(token), ordered: false });
    }
    result
}

struct SubjectContent {
    catalogue: Arc<Catalogue>,
    states: AutonomyStates,
    warnings: Vec<String>,
}

fn subject_content(source: &Source, options: &dashboard_base::products::ContentOptions) -> io::Result<Arc<SubjectContent>> {
    let catalogue = catalogue::catalogue(source, options)?;
    let state = source.state::<Option<Arc<SubjectContent>>>();
    let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(cached) = state.as_ref()
        && Arc::ptr_eq(&cached.catalogue, &catalogue)
    {
        return Ok(Arc::clone(cached));
    }
    let (states, warnings) = content_reader_politics::autonomy::read(&catalogue.content.layers)?;
    let loaded = Arc::new(SubjectContent { catalogue, states, warnings: warnings.into_iter().map(|(_, message)| message).collect() });
    *state = Some(Arc::clone(&loaded));
    Ok(loaded)
}

pub(crate) fn named_levels(source: &Source, context: &LoadContext, named: &BTreeSet<&str>, warnings: &mut Vec<String>) -> Vec<SubjectLevel> {
    let content = match context.content.as_ref() {
        Some(options) => match subject_content(source, options) {
            Ok(content) => {
                warnings.extend(content.warnings.iter().cloned());
                Some(content)
            }
            Err(error) => {
                warnings.push(format!("Subject levels show by their ids, in no known order: {error}"));
                None
            }
        },
        None => {
            warnings.push("Subject levels show by their ids, in no known order, until the game folders are chosen.".into());
            None
        }
    };
    let out = levels(named, content.as_ref().map(|content| &content.states), content.as_ref().map(|content| &*content.catalogue));
    if let Some(content) = &content {
        warnings.extend(content.catalogue.refusal_lines());
    }
    out
}

fn held_runs(source: &Source, query: &dashboard_query::Query<'_>, cancelled: &AtomicBool) -> io::Result<HashMap<u32, (Arc<RunSubjects>, Arc<RunLinks>)>> {
    let mut held = HashMap::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if !held.contains_key(&stretch.run) {
            held.insert(stretch.run, (crate::products::subjects::cached_subjects(source, stretch.run)?, dashboard_base::queries::links(source, stretch.run)?));
        }
    }
    Ok(held)
}

pub fn history(source: &Source, query: &dashboard_query::Query<'_>, cancelled: &AtomicBool) -> io::Result<(Vec<SubjectRun>, Vec<HourSpan>)> {
    let held = held_runs(source, query, cancelled)?;
    let names = Names::new(&query.holders);
    Ok(fold(&query.timeline.stretches, &held, &names))
}

pub fn unknown_hours(query: &dashboard_query::Query<'_>) -> Vec<HourSpan> {
    let mut out: Vec<HourSpan> = query.holders.unknown.iter().map(|(first, last, _)| HourSpan { first_hour: *first, last_hour: *last }).collect();
    out.extend(query.timeline.unheld.iter().map(|gap| HourSpan { first_hour: gap.first, last_hour: gap.last }));
    out
}

impl Product for Subjects {
    const KEY: &'static str = "subjects";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = SubjectReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![BranchParams::decl(), SubjectLevel::decl(), LevelStep::decl(), SubjectRun::decl(), HourSpan::decl(), SubjectReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid subjects branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid subjects branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let (runs, mut unobserved) = history(source, &query, cancelled)?;
        unobserved.extend(unknown_hours(&query));
        let mut warnings = Vec::new();
        let named: BTreeSet<&str> = runs.iter().flat_map(|run| run.steps.iter().filter_map(|step| step.level.as_deref())).collect();
        let levels = named_levels(source, context, &named, &mut warnings);
        Ok(SubjectReply { branch_id: params.branch_id.clone(), levels, runs, unobserved: settle(unobserved), warnings })
    }
}

#[cfg(test)]
mod tests;

enum RunKey {}

type CachedRuns = dashboard_base::queries::RunCache<RunKey, RunSubjects>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    CachedRuns::forget(source);
}

pub fn cached_subjects(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<RunSubjects>> {
    CachedRuns::held(&source.state::<CachedRuns>(), run, || run_subjects(source.replay_ref().recording(), run))
}
