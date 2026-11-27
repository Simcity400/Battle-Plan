
use dashboard_base::products::{BranchParams, ContentOptions, LoadContext, Product};
use dashboard_core::protocol::validate_tag;
use dashboard_base::{Source, catalogue::{self, Catalogue, plain_text}, content};
use dashboard_core::invalid;
use dashboard_text::{flag_choices, scripted_text::{ScriptedText, TABLES as SCRIPTED_TABLES, probe_unselected_tables}};
use crate::products::country_flags::{CountryFlagChoice, FlagFolders};
use dashboard_query::{Query, read};
use content_reader_country_names::country_names::{CountryName, CountryNames};
use content_reader_localisation::localisation::Localisation;
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use replay_looks::Look;
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{Recorded, TableId, TagHolderRow};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const NO_COLOUR: &str = "no recorded map colour";
const NOT_A_COUNTRY: &str = "not a country at this hour";
const UNKNOWN: &str = "the country tables are not complete at this hour";

pub fn country_name(names: &CountryNames<'_>, slot: u32) -> Result<String, String> {
    names
        .name(slot)
        .and_then(|name| plain_text(&name).map_err(|reason| format!("its name holds {reason}")))
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryStyleParams {
    pub branch_id: String,
    pub hour: i32,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryStyle {
    pub tag: String,
    #[doc = " The name the game showed; null when it could not be resolved."]
    pub name: Option<String>,
    #[doc = " The recorded map colour as sRGB bytes; null when not recorded this hour."]
    pub color: Option<[u8; 3]>,
    #[doc = " The inclusive `[first, last]` hour interval this style or absence is proven for."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub valid: Option<[i32; 2]>,
    #[doc = " True when the tag is proven not to exist as a country across `valid`."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub absent: Option<bool>,
    #[doc = " Why the style or part of it is missing at this hour."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub reason: Option<String>,
    #[doc = " The dated flag choice for this country across `valid`."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub flag_choice: Option<CountryFlagChoice>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryStyleReply {
    pub branch_id: String,
    pub hour: i32,
    pub styles: Vec<CountryStyle>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryFactsReply {
    pub branch_id: String,
    #[doc = " The hours the runs answer, as `[first, last]` spans in order."]
    pub known: Vec<[i32; 2]>,
    #[doc = " Names, each once; a run holds its index."]
    pub names: Vec<String>,
    #[doc = " Why part of a style is missing, each once; a run holds its index."]
    pub reasons: Vec<String>,
    #[doc = " Flag choices, each once; a run holds its index."]
    pub flags: Vec<CountryFlagChoice>,
    pub countries: Vec<CountryFactRuns>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryFactRuns {
    pub tag: String,
    #[doc = " `[hour, name, colour, reason, flag]` from `hour` until the next run: the name's index in"]
    #[doc = " `names`, the colour as `0xRRGGBB`, the reason's index in `reasons` and the flag choice's"]
    #[doc = " index in `flags`, each -1 when there is none. A tag that is not a country has a run with"]
    #[doc = " no name, colour or flag and its reason."]
    pub runs: Vec<[i32; 5]>,
}

dashboard_base::reply_warnings!(CountryStyleReply, CountryFactsReply);

const MAX_CACHED_SCRIPTED_FRAMES: usize = 4;

struct BoundedFrames {
    entries: HashMap<(u32, i32), DecodedFrame>,
    order: VecDeque<(u32, i32)>,
}

impl BoundedFrames {
    fn new() -> Self {
        Self { entries: HashMap::new(), order: VecDeque::new() }
    }

    fn take_or_empty(&mut self, run: u32, hour: i32) -> DecodedFrame {
        if let Some(frame) = self.entries.remove(&(run, hour)) {
            if let Some(pos) = self.order.iter().position(|&k| k == (run, hour)) {
                self.order.remove(pos);
            }
            return frame;
        }
        DecodedFrame {
            branch: 0,
            hour,
            tables: std::array::from_fn(|_| not_selected()),
            metrics: Metrics::default(),
        }
    }

    fn put(&mut self, run: u32, hour: i32, frame: DecodedFrame) {
        let key = (run, hour);
        if self.entries.contains_key(&key) {
            if let Some(pos) = self.order.iter().position(|&k| k == key) {
                self.order.remove(pos);
            }
        } else if self.order.len() >= MAX_CACHED_SCRIPTED_FRAMES
            && let Some(oldest) = self.order.pop_front()
        {
            self.entries.remove(&oldest);
        }
        self.order.push_back(key);
        self.entries.insert(key, frame);
    }
}

pub(crate) struct ScriptedEvaluator {
    read_tables: Box<dyn Fn(u32, i32, &[TableId]) -> io::Result<DecodedFrame> + Send + Sync>,
    catalogue: Result<Arc<Catalogue>, String>,
    frames: Mutex<BoundedFrames>,
}

impl ScriptedEvaluator {
    pub(crate) fn for_source(source: &Source, content: Option<&ContentOptions>) -> Self {
        let replay = source.replay();
        let read_tables = Box::new(move |run: u32, hour: i32, tables: &[TableId]| {
            let reads: Vec<_> = tables
                .iter()
                .map(|&table| replay_lanes::at::Read {
                    table,
                    columns: None,
                    keys: replay_lanes::at::Keys::All,
                })
                .collect();
            replay_lanes::at::frame(replay.recording(), run, hour, &reads)
        });
        let catalogue = match content {
            Some(content) => catalogue::recorded_catalogue(source, content).map_err(|error| error.to_string()),
            None => Err("Game content is not configured".to_owned()),
        };
        Self { read_tables, catalogue, frames: Mutex::new(BoundedFrames::new()) }
    }

    pub(crate) fn expand(&self, run: u32, hour: i32, slot: u32, key: &str, text: &str) -> Result<String, String> {
        let catalogue = self.catalogue.as_ref().map_err(|why| format!("localisation {key}: {why}"))?;
        let content = catalogue.scripted_content().map_err(|why| format!("localisation {key}: {why}"))?;
        let mut frame = self.frames.lock().unwrap_or_else(|error| error.into_inner()).take_or_empty(run, hour);
        let mut last_result: Result<String, String> = Err("scripted country name did not resolve".to_owned());
        for _ in 0..=SCRIPTED_TABLES.len() {
            let scripted = ScriptedText::new(&frame, &catalogue.localisation, content, slot);
            let mut result = None;
            let needed = probe_unselected_tables(&frame, || {
                result = Some(scripted.expand(text));
            });
            last_result = result.expect("probe ran expansion");
            if needed.is_empty() {
                break;
            }
            let partial = (self.read_tables)(run, hour, &needed).map_err(|why| format!("localisation {key}: {why}"));
            let partial = match partial {
                Ok(partial) => partial,
                Err(err) => {
                    self.frames.lock().unwrap_or_else(|error| error.into_inner()).put(run, hour, frame);
                    return Err(err);
                }
            };
            frame.branch = partial.branch;
            for table in needed {
                frame.tables[table.index()] = partial.tables[table.index()].clone();
            }
        }
        self.frames.lock().unwrap_or_else(|error| error.into_inner()).put(run, hour, frame);
        let expanded = last_result.map_err(|why| format!("localisation {key}: {why}"))?;
        plain_text(&expanded).map_err(|reason| format!("localisation {key}: its name holds {reason}"))
    }
}

fn format_problem(unnamed: Option<String>, colour: Option<[u8; 3]>) -> Option<String> {
    match (unnamed, colour) {
        (Some(reason), None) => Some(format!("{reason}; {NO_COLOUR}")),
        (Some(reason), Some(_)) => Some(reason),
        (None, None) => Some(NO_COLOUR.to_owned()),
        (None, Some(_)) => None,
    }
}

fn resolve_look_name(
    look: &Look,
    localisation: &Localisation,
    expand: impl FnOnce(&str, &str, u32) -> Result<String, String>,
) -> Result<String, String> {
    let names = CountryNames::recorded(localisation, look.chain.iter().cloned().collect(), look.built);
    match names.resolve_name(look.slot) {
        Ok(CountryName::Plain(name)) => plain_text(&name).map_err(|reason| format!("its name holds {reason}")),
        Ok(CountryName::Scripted { key, text }) => expand(&key, &text, look.slot),
        Err(reason) => Err(reason),
    }
}

pub(crate) fn style_with(
    tag: &str,
    look: &Look,
    localisation: Option<&Localisation>,
    expand: impl FnOnce(&str, &str, u32) -> Result<String, String>,
) -> (CountryStyle, Option<String>) {
    let (name, unnamed) = match localisation {
        None => (None, None),
        Some(localisation) => match resolve_look_name(look, localisation, expand) {
            Ok(name) => (Some(name), None),
            Err(reason) => (None, Some(reason)),
        },
    };
    let problem = format_problem(unnamed, look.colour);
    (
        CountryStyle {
            tag: tag.to_owned(),
            name,
            color: look.colour,
            valid: None,
            absent: None,
            reason: None,
            flag_choice: None,
        },
        problem,
    )
}

fn flag_chain(tag: &str, look: &Look) -> flag_choices::Chain {
    if (look.id as i32) <= 0 {
        return flag_choices::Chain { candidates: Vec::new(), refusal: Some(format!("country {} names no country object", look.id)) };
    }
    let appearance = look.flag.appearance.as_ref().map(|(cosmetic, reference, original)| flag_choices::Appearance {
        cosmetic,
        reference: *reference,
        original: match original {
            Recorded::NotRecorded => Recorded::NotRecorded,
            Recorded::Value(original) => Recorded::Value(original.as_deref()),
        },
    });
    flag_choices::Operands::of(look.id, tag.as_bytes(), appearance, look.flag.ruling, look.flag.group.as_deref()).chain()
}

pub struct RunLooks {
    pub spans: Vec<(u32, u32, i32, i32)>,
    pub unknown: Vec<(i32, i32)>,
    pub looks: HashMap<u32, Look>,
    pub raw_looks: HashMap<u32, Box<[u8]>>,
    pub tags: HashMap<u32, Arc<str>>,
}

pub fn run_looks(recording: &Recording, run: u32) -> io::Result<RunLooks> {
    let held = recording.run(run)?;
    let end = held.first_hour + held.ordinals as i32 - 1;
    let (mut looks, mut raw_looks, mut tags) = (HashMap::new(), HashMap::new(), HashMap::new());
    let mut failed = None;
    let held = read::at(recording, run, &[read::Read::all(TableId::LookFacts), read::Read::all(TableId::TagNames)], end)?;
    let rows = held[0].view();
    for slot in rows.rows() {
        let (Some(key), Some(bytes)) = (rows.key(slot), read::text(&rows, 0, slot)) else { continue };
        match Look::decode(bytes) {
            Ok(look) => {
                looks.insert(key as u32, look);
                raw_looks.insert(key as u32, Box::from(bytes));
            }
            Err(error) => failed = Some(error),
        }
    }
    let rows = held[1].view();
    for slot in rows.rows() {
        let (Some(key), Some(bytes)) = (rows.key(slot), read::text(&rows, 0, slot)) else { continue };
        tags.insert(key as u32, Arc::from(String::from_utf8_lossy(bytes).as_ref()));
    }
    if let Some(error) = failed {
        return Err(error);
    }
    let (mut spans, mut unknown) = (Vec::new(), Vec::new());
    for (key, first, last) in dashboard_query::cost::lifetimes(recording, run, TableId::LookSpans)? {
        let (number, look) = ((key >> 64) as u32, (key >> 32) as u32);
        if number == TagHolderRow::UNKNOWN {
            unknown.push((first as i32, last as i32));
        } else {
            spans.push((number, look, first as i32, last as i32));
        }
    }
    Ok(RunLooks { spans, unknown, looks, raw_looks, tags })
}

fn localisation_result(source: &Source, content: Option<&ContentOptions>, warnings: &mut Vec<String>) -> io::Result<Result<Arc<Localisation>, String>> {
    let localisation = content::name_localisation(source, content);
    match (&localisation, content) {
        (Err(reason), _) => warnings.push(reason.clone()),
        (Ok(_), Some(content)) => content::check_replay(source.replay_ref(), content.allow_unverified, warnings)?,
        (Ok(_), None) => {}
    }
    Ok(localisation)
}

pub(crate) fn localisation(source: &Source, content: Option<&ContentOptions>, warnings: &mut Vec<String>) -> io::Result<Option<Arc<Localisation>>> {
    Ok(localisation_result(source, content, warnings)?.ok())
}

#[derive(Default)]
struct CachedFlagFolders(Option<(Arc<content_reader_layers::ResolvedContent>, Arc<FlagFolders>)>);

fn flag_folders(source: &Source, content: Option<&ContentOptions>) -> io::Result<Result<Arc<FlagFolders>, String>> {
    let Some(content) = content else {
        return Ok(Err("flags cannot be chosen: Game content is not configured".to_owned()));
    };
    let resolved = match content::check_replay(source.replay_ref(), content.allow_unverified, &mut Vec::new()).and_then(|()| source.content(&content.options)) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => return Ok(Err(format!("flags cannot be chosen: {error}"))),
    };
    let state = source.state::<CachedFlagFolders>();
    if let Some((held_content, held_folders)) = state.lock().unwrap_or_else(|error| error.into_inner()).0.as_ref()
        && Arc::ptr_eq(held_content, &resolved)
    {
        return Ok(Ok(Arc::clone(held_folders)));
    }
    match FlagFolders::read(&resolved) {
        Ok(folders) => {
            let folders = Arc::new(folders);
            state.lock().unwrap_or_else(|error| error.into_inner()).0 = Some((resolved, Arc::clone(&folders)));
            Ok(Ok(folders))
        }
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
        Err(error) => Ok(Err(format!("flags cannot be chosen: {error}"))),
    }
}

#[derive(Default)]
struct Dictionary {
    numbered: HashMap<String, i32>,
    texts: Vec<String>,
}

impl Dictionary {
    fn number(&mut self, text: &str) -> i32 {
        if let Some(&number) = self.numbered.get(text) {
            return number;
        }
        let number = self.texts.len() as i32;
        self.numbered.insert(text.to_owned(), number);
        self.texts.push(text.to_owned());
        number
    }
}

fn facts(source: &Source, query: &Query<'_>, branch_id: &str, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<CountryFactsReply> {
    let mut warnings = Vec::new();
    let localisation = localisation_result(source, context.content.as_ref(), &mut warnings)?;
    let folders = flag_folders(source, context.content.as_ref())?.map_err(|reason| CountryFlagChoice {
        keys: [None, None, None],
        reasons: [Some(reason.clone()), Some(reason.clone()), Some(reason)],
        searched: Default::default(),
    });
    let (mut names, mut reasons) = (Dictionary::default(), Dictionary::default());
    let mut flags: Vec<CountryFlagChoice> = Vec::new();
    let mut flag_numbers: HashMap<CountryFlagChoice, i32> = HashMap::new();
    let mut known: Vec<[i32; 2]> = Vec::new();
    let mut runs_read: HashMap<u32, Arc<RunLooks>> = HashMap::new();
    let mut unique_index: HashMap<(Arc<str>, Box<[u8]>), usize> = HashMap::new();
    let mut unique_looks: Vec<(Arc<str>, u32, u32)> = Vec::new();
    let mut raw_segments: HashMap<Arc<str>, Vec<(i32, i32, u32, usize)>> = HashMap::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if let std::collections::hash_map::Entry::Vacant(slot) = runs_read.entry(stretch.run) {
            slot.insert(crate::products::styles::cached_looks(source, stretch.run)?);
        }
        let run = &runs_read[&stretch.run];
        let mut cursor = stretch.first;
        let mut unknown: Vec<(i32, i32)> = run.unknown.iter().map(|&(a, b)| (a.max(stretch.first), b.min(stretch.last))).filter(|(a, b)| a <= b).collect();
        unknown.sort_unstable();
        for (a, b) in unknown {
            if a > cursor {
                known.push([cursor, a - 1]);
            }
            cursor = cursor.max(b + 1);
        }
        if cursor <= stretch.last {
            known.push([cursor, stretch.last]);
        }
        for &(number, look_number, first, last) in &run.spans {
            let (first, last) = (first.max(stretch.first), last.min(stretch.last));
            if first > last {
                continue;
            }
            let Some(tag) = run.tags.get(&number) else { continue };
            let Some(raw) = run.raw_looks.get(&look_number) else {
                return Err(invalid("A look span names no look"));
            };
            let key = (Arc::clone(tag), raw.clone());
            let id = *unique_index.entry(key).or_insert_with(|| {
                let id = unique_looks.len();
                unique_looks.push((Arc::clone(tag), stretch.run, look_number));
                id
            });
            raw_segments.entry(Arc::clone(tag)).or_default().push((first, last, stretch.run, id));
        }
    }
    let chains: Vec<flag_choices::Chain> = if folders.is_ok() {
        unique_looks
            .iter()
            .map(|(tag, run_id, look_number)| flag_chain(tag, &runs_read[run_id].looks[look_number]))
            .collect()
    } else {
        Vec::new()
    };
    if let Ok(folders) = &folders {
        let mut unique_chains: Vec<flag_choices::Chain> = Vec::new();
        let mut seen_chains: std::collections::HashSet<&flag_choices::Chain> = std::collections::HashSet::with_capacity(chains.len());
        for chain in &chains {
            if seen_chains.insert(chain) {
                unique_chains.push(chain.clone());
            }
        }
        folders.warm_all(&unique_chains);
    }
    enum FactEntry {
        Fixed([i32; 4]),
        Scripted { slot: u32, colour: Option<[u8; 3]>, key: String, text: String, flag: i32 },
    }
    let mut resolved_facts: Vec<FactEntry> = Vec::with_capacity(unique_looks.len());
    for (idx, (_tag, run_id, look_number)) in unique_looks.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let look = &runs_read[run_id].looks[look_number];
        let choice = match &folders {
            Ok(folders) => folders.choose(&chains[idx])?,
            Err(none) => none.clone(),
        };
        let flag = *flag_numbers.entry(choice).or_insert_with_key(|choice| {
            flags.push(choice.clone());
            flags.len() as i32 - 1
        });
        let color_packed = look.colour.map_or(-1, |[red, green, blue]| (i32::from(red) << 16) | (i32::from(green) << 8) | i32::from(blue));
        match localisation.as_deref() {
            Err(loc_reason) => {
                let problem = format_problem(Some(loc_reason.to_owned()), look.colour);
                resolved_facts.push(FactEntry::Fixed([-1, color_packed, problem.map_or(-1, |problem| reasons.number(&problem)), flag]));
            }
            Ok(localisation) => {
                let country_names = CountryNames::recorded(localisation, look.chain.iter().cloned().collect(), look.built);
                match country_names.resolve_name(look.slot) {
                    Ok(CountryName::Scripted { key, text }) => {
                        resolved_facts.push(FactEntry::Scripted { slot: look.slot, colour: look.colour, key, text, flag });
                    }
                    Ok(CountryName::Plain(raw)) => {
                        let (name, unnamed) = match plain_text(&raw) {
                            Ok(plain) => (Some(plain), None),
                            Err(reason) => (None, Some(format!("its name holds {reason}"))),
                        };
                        let problem = format_problem(unnamed, look.colour);
                        resolved_facts.push(FactEntry::Fixed([
                            name.as_deref().map_or(-1, |name| names.number(name)),
                            color_packed,
                            problem.map_or(-1, |problem| reasons.number(&problem)),
                            flag,
                        ]));
                    }
                    Err(reason) => {
                        let problem = format_problem(Some(reason), look.colour);
                        resolved_facts.push(FactEntry::Fixed([-1, color_packed, problem.map_or(-1, |problem| reasons.number(&problem)), flag]));
                    }
                }
            }
        }
    }
    let mut segments: Vec<(Arc<str>, Vec<(i32, i32, Option<[i32; 4]>)>)> = Vec::with_capacity(raw_segments.len());
    struct ScriptedSpan {
        tag_idx: usize,
        first: i32,
        last: i32,
        fact_id: usize,
    }
    let mut scripted_by_run: std::collections::BTreeMap<u32, Vec<ScriptedSpan>> = std::collections::BTreeMap::new();
    for (tag, held) in raw_segments {
        let tag_idx = segments.len();
        let mut out = Vec::with_capacity(held.len());
        for (first, last, run_id, id) in held {
            match &resolved_facts[id] {
                FactEntry::Fixed(fact) => out.push((first, last, Some(*fact))),
                FactEntry::Scripted { .. } => {
                    scripted_by_run.entry(run_id).or_default().push(ScriptedSpan { tag_idx, first, last, fact_id: id });
                }
            }
        }
        segments.push((tag, out));
    }
    if !scripted_by_run.is_empty() {
        let eval = ScriptedEvaluator::for_source(source, context.content.as_ref());
        struct ActiveSpan {
            tag_idx: usize,
            last: i32,
            fact_id: usize,
            run_start: i32,
            run_fact: Option<[i32; 4]>,
        }
        for (run_id, mut spans) in scripted_by_run {
            spans.sort_unstable_by_key(|span| span.first);
            let mut active: Vec<ActiveSpan> = Vec::new();
            let mut next = 0;
            let mut hour = 0i32;
            while !active.is_empty() || next < spans.len() {
                if active.is_empty() {
                    hour = spans[next].first;
                }
                while next < spans.len() && spans[next].first == hour {
                    let span = &spans[next];
                    active.push(ActiveSpan {
                        tag_idx: span.tag_idx,
                        last: span.last,
                        fact_id: span.fact_id,
                        run_start: span.first,
                        run_fact: None,
                    });
                    next += 1;
                }
                let mut i = 0;
                while i < active.len() {
                    dashboard_text::metadata::check_cancel(cancelled)?;
                    let item = &mut active[i];
                    let FactEntry::Scripted { slot, colour, key, text, flag } = &resolved_facts[item.fact_id] else {
                        unreachable!();
                    };
                    let color_packed = colour.map_or(-1, |[red, green, blue]| (i32::from(red) << 16) | (i32::from(green) << 8) | i32::from(blue));
                    let (name, unnamed) = match eval.expand(run_id, hour, *slot, key, text) {
                        Ok(name) => (Some(name), None),
                        Err(reason) => (None, Some(reason)),
                    };
                    let problem = format_problem(unnamed, *colour);
                    let fact = [
                        name.as_deref().map_or(-1, |name| names.number(name)),
                        color_packed,
                        problem.map_or(-1, |problem| reasons.number(&problem)),
                        *flag,
                    ];
                    match item.run_fact {
                        Some(prev) if prev == fact => {}
                        Some(prev) => {
                            segments[item.tag_idx].1.push((item.run_start, hour - 1, Some(prev)));
                            item.run_start = hour;
                            item.run_fact = Some(fact);
                        }
                        None => {
                            item.run_start = hour;
                            item.run_fact = Some(fact);
                        }
                    }
                    if hour >= item.last {
                        if let Some(prev) = item.run_fact {
                            segments[item.tag_idx].1.push((item.run_start, item.last, Some(prev)));
                        }
                        active.swap_remove(i);
                    } else {
                        i += 1;
                    }
                }
                if !active.is_empty() {
                    hour = hour.checked_add(1).expect("scripted hour overflow");
                }
            }
        }
    }
    known.sort_unstable();
    let mut joined: Vec<[i32; 2]> = Vec::with_capacity(known.len());
    for span in known {
        match joined.last_mut() {
            Some(held) if i64::from(held[1]) + 1 >= i64::from(span[0]) => held[1] = held[1].max(span[1]),
            _ => joined.push(span),
        }
    }
    let known = joined;
    let not_a_country = [-1, -1, -2, -1];
    let mut countries: Vec<CountryFactRuns> = segments
        .into_iter()
        .map(|(tag, mut held)| {
            held.sort_unstable_by_key(|segment| segment.0);
            let mut runs: Vec<[i32; 5]> = Vec::new();
            let mut last: Option<[i32; 4]> = None;
            let mut give = |hour: i32, fact: [i32; 4], runs: &mut Vec<[i32; 5]>| {
                if last != Some(fact) {
                    runs.push([hour, fact[0], fact[1], fact[2], fact[3]]);
                    last = Some(fact);
                }
            };
            let mut next = 0;
            for &[first, end] in &known {
                let mut cursor = first;
                while next < held.len() && held[next].0 <= end {
                    let (a, b, fact) = held[next];
                    if a > cursor {
                        give(cursor, not_a_country, &mut runs);
                    }
                    give(a.max(cursor), fact.unwrap_or(not_a_country), &mut runs);
                    cursor = b + 1;
                    next += 1;
                }
                if cursor <= end {
                    give(cursor, not_a_country, &mut runs);
                }
            }
            CountryFactRuns { tag: tag.to_string(), runs }
        })
        .filter(|country| !country.runs.is_empty())
        .collect();
    countries.sort_unstable_by(|a, b| a.tag.cmp(&b.tag));
    if countries.iter().flat_map(|country| &country.runs).any(|run| run[3] == -2) {
        let reason = reasons.number(NOT_A_COUNTRY);
        for run in countries.iter_mut().flat_map(|country| country.runs.iter_mut()) {
            if run[3] == -2 {
                run[3] = reason;
            }
        }
    }
    Ok(CountryFactsReply {
        branch_id: branch_id.to_owned(),
        known,
        names: names.texts,
        reasons: reasons.texts,
        flags,
        countries,
        warnings,
    })
}

pub struct CountryFacts;

impl Product for CountryFacts {
    const KEY: &'static str = "country-facts";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = CountryFactsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CountryFactRuns::decl(), CountryFlagChoice::decl(), CountryFactsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid country facts branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid country facts branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        facts(source, &query, &params.branch_id, context, cancelled)
    }
}

pub struct CountryStyles;

impl Product for CountryStyles {
    const KEY: &'static str = "country-styles";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = CountryStyleParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, tags];
    type Reply = CountryStyleReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CountryStyleParams::decl(), CountryStyle::decl(), CountryStyleReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.tags.is_empty() {
            return Err("Invalid country style selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid country style branch")
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse::<u64>().map_err(|_| invalid("Invalid country style branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        if query.timeline.at(params.hour).is_none() {
            let reason = query.timeline.reason(params.hour);
            return Err(dashboard_base::gap_error(&reason, format!("Country styles unavailable at this hour: {reason}")));
        }
        let mut warnings = Vec::new();
        let localisation = localisation_result(source, context.content.as_ref(), &mut warnings)?;
        let folders = flag_folders(source, context.content.as_ref())?.map_err(|reason| CountryFlagChoice {
            keys: [None, None, None],
            reasons: [Some(reason.clone()), Some(reason.clone()), Some(reason)],
            searched: Default::default(),
        });
        let stretch = query.timeline.at(params.hour).copied().ok_or_else(|| invalid("The timeline holds no such hour"))?;
        let run = crate::products::styles::cached_looks(source, stretch.run)?;
        let unknown_span = run
            .unknown
            .iter()
            .copied()
            .find(|&(a, b)| a <= params.hour && params.hour <= b)
            .map(|(a, b)| [a.max(stretch.first), b.min(stretch.last)]);
        let known = unknown_span.is_none();
        let mut known_first = stretch.first;
        let mut known_last = stretch.last;
        if known {
            for &(a, b) in &run.unknown {
                if b < params.hour {
                    known_first = known_first.max(b.saturating_add(1));
                } else if a > params.hour {
                    known_last = known_last.min(a.saturating_sub(1));
                }
            }
        }
        let wanted: std::collections::HashSet<&str> = params.tags.iter().map(String::as_str).collect();
        let mut at: HashMap<&str, (&Look, i32, i32)> = HashMap::new();
        let mut prev_end: HashMap<&str, i32> = HashMap::new();
        let mut next_start: HashMap<&str, i32> = HashMap::new();
        if known {
            for &(number, look, first, last) in &run.spans {
                let (first, last) = (first.max(known_first), last.min(known_last));
                if first > last {
                    continue;
                }
                let Some(tag) = run.tags.get(&number).map(Arc::as_ref) else { continue };
                if !wanted.contains(tag) {
                    continue;
                }
                if first <= params.hour && params.hour <= last {
                    if let Some(look) = run.looks.get(&look) {
                        at.insert(tag, (look, first, last));
                    }
                } else if last < params.hour {
                    prev_end.entry(tag).and_modify(|end| *end = (*end).max(last)).or_insert(last);
                } else {
                    next_start.entry(tag).and_modify(|start| *start = (*start).min(first)).or_insert(first);
                }
            }
        }
        let mut styles = Vec::with_capacity(params.tags.len());
        let mut evaluator: Option<ScriptedEvaluator> = None;
        for tag in &params.tags {
            match at.get(tag.as_str()) {
                Some(&(look, first, last)) => {
                    let mut was_scripted = false;
                    let (mut built, problem) = match localisation.as_deref() {
                        Ok(loc) => style_with(tag, look, Some(loc), |key, text, slot| {
                            was_scripted = true;
                            let eval = evaluator.get_or_insert_with(|| ScriptedEvaluator::for_source(source, context.content.as_ref()));
                            eval.expand(stretch.run, params.hour, slot, key, text)
                        }),
                        Err(loc_reason) => (
                            CountryStyle {
                                tag: tag.to_owned(),
                                name: None,
                                color: look.colour,
                                valid: None,
                                absent: None,
                                reason: None,
                                flag_choice: None,
                            },
                            format_problem(Some(loc_reason.to_owned()), look.colour),
                        ),
                    };
                    let choice = match &folders {
                        Ok(folders) => folders.choose(&flag_chain(tag, look))?,
                        Err(none) => none.clone(),
                    };
                    if localisation.is_ok()
                        && let Some(ref text) = problem
                    {
                        warnings.push(format!("{tag}: {text}"));
                    }
                    built.valid = Some(if was_scripted { [params.hour, params.hour] } else { [first, last] });
                    built.reason = problem;
                    built.flag_choice = Some(choice);
                    styles.push(built);
                }
                _ if !known => {
                    warnings.push(format!("{tag}: {UNKNOWN}"));
                    styles.push(CountryStyle {
                        tag: tag.clone(),
                        name: None,
                        color: None,
                        valid: unknown_span,
                        absent: None,
                        reason: Some(UNKNOWN.to_owned()),
                        flag_choice: None,
                    });
                }
                _ => {
                    warnings.push(format!("{tag}: {NOT_A_COUNTRY}"));
                    let absent_first = prev_end.get(tag.as_str()).map_or(known_first, |&end| end.saturating_add(1).max(known_first));
                    let absent_last = next_start.get(tag.as_str()).map_or(known_last, |&start| start.saturating_sub(1).min(known_last));
                    styles.push(CountryStyle {
                        tag: tag.clone(),
                        name: None,
                        color: None,
                        valid: Some([absent_first, absent_last]),
                        absent: Some(true),
                        reason: Some(NOT_A_COUNTRY.to_owned()),
                        flag_choice: None,
                    });
                }
            }
        }
        Ok(CountryStyleReply {
            branch_id: params.branch_id.clone(),
            hour: params.hour,
            styles,
            warnings,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedCountryLook {
    pub name: Option<String>,
    pub color: Option<[u8; 3]>,
    pub name_missing: Option<String>,
    pub color_missing: Option<String>,
}

struct IndexedRunLooks {
    unknown_dates: std::sync::OnceLock<Vec<i32>>,
    run: Arc<RunLooks>,
    by_tag: HashMap<Arc<str>, Vec<(i32, i32, u32)>>,
    scripted_looks: HashSet<u32>,
}

impl IndexedRunLooks {
    fn new(run: Arc<RunLooks>, validity_localisation: Option<&Localisation>) -> Self {
        let mut by_tag: HashMap<Arc<str>, Vec<(i32, i32, u32)>> = HashMap::new();
        for &(number, look, first, last) in &run.spans {
            if let Some(tag) = run.tags.get(&number) {
                by_tag.entry(Arc::clone(tag)).or_default().push((first, last, look));
            }
        }
        for spans in by_tag.values_mut() {
            spans.sort_unstable_by_key(|&(first, last, _)| (first, last));
        }
        let scripted_looks = match validity_localisation {
            Some(localisation) => run
                .looks
                .iter()
                .filter_map(|(&id, look)| {
                    let names = CountryNames::recorded(localisation, look.chain.iter().cloned().collect(), look.built);
                    matches!(names.resolve_name(look.slot), Ok(CountryName::Scripted { .. })).then_some(id)
                })
                .collect(),
            None => HashSet::new(),
        };
        Self { run, by_tag, unknown_dates: Default::default(), scripted_looks }
    }

    fn next_change(&self, hour: i32) -> i32 {
        let spans = self.run.spans.iter().map(|&(_, _, first, last)| (first, last));
        let normal = spans
            .chain(self.run.unknown.iter().copied())
            .flat_map(|(first, last)| [first, last.saturating_add(1)])
            .filter(|&boundary| boundary > hour)
            .min()
            .unwrap_or(i32::MAX);
        if !self.scripted_looks.is_empty()
            && self
                .run
                .spans
                .iter()
                .any(|&(_, look, first, last)| first <= hour && hour <= last && self.scripted_looks.contains(&look))
        {
            return hour.saturating_add(1).min(normal);
        }
        normal
    }

    fn next_for(&self, tag: &str, hour: i32) -> Option<i32> {
        let spans = self.by_tag.get(tag).map_or(&[][..], Vec::as_slice);
        let at = spans.partition_point(|&(first, _, _)| first <= hour);
        let next = spans.get(at).map(|span| span.0);
        if let Some((_, last, look)) = at.checked_sub(1).map(|previous| spans[previous])
            && hour <= last && self.run.looks.contains_key(&look) {
            let end = if self.scripted_looks.contains(&look) {
                hour.checked_add(1)
            } else {
                last.checked_add(1)
            };
            return [end, next].into_iter().flatten().min();
        }
        let unknown = self.unknown_dates.get_or_init(|| {
            let mut dates: Vec<_> = self.run.unknown.iter().flat_map(|&(first, last)| [first, last.saturating_add(1)]).collect();
            dates.sort_unstable();
            dates.dedup();
            dates
        });
        let boundary = unknown.get(unknown.partition_point(|&at| at <= hour)).copied();
        [next, boundary].into_iter().flatten().min()
    }

    fn has_scripted_names(&self) -> bool {
        !self.scripted_looks.is_empty()
    }

    fn resolve(
        &self,
        tag: &str,
        hour: i32,
        localisation: Option<&Localisation>,
        expand: impl FnOnce(&str, &str, u32) -> Result<String, String>,
    ) -> ResolvedCountryLook {
        let known = !self.run.unknown.iter().any(|&(a, b)| a <= hour && hour <= b);
        if let Some(spans) = self.by_tag.get(tag) {
            let at = spans.partition_point(|&(first, _, _)| first <= hour);
            if let Some(&(_, last, look_id)) = at.checked_sub(1).and_then(|index| spans.get(index))
                && hour <= last
                && let Some(look) = self.run.looks.get(&look_id)
            {
                let (name, name_missing) = match localisation {
                    None => (None, Some("Game content is not configured".to_owned())),
                    Some(localisation) => match resolve_look_name(look, localisation, expand) {
                        Ok(name) => (Some(name), None),
                        Err(reason) => (None, Some(reason)),
                    },
                };
                let color_missing = look.colour.is_none().then(|| NO_COLOUR.to_owned());
                return ResolvedCountryLook { name, color: look.colour, name_missing, color_missing };
            }
        }
        let reason = if known { NOT_A_COUNTRY } else { UNKNOWN }.to_owned();
        ResolvedCountryLook { name: None, color: None, name_missing: Some(reason.clone()), color_missing: Some(reason) }
    }
}

pub struct LookResolver<'a> {
    source: &'a Source,
    timeline: &'a dashboard_query::timeline::Timeline,
    content: Option<ContentOptions>,
    localisation: Option<Arc<Localisation>>,
    evaluator: Option<ScriptedEvaluator>,
    runs: HashMap<u32, IndexedRunLooks>,
}

impl<'a> LookResolver<'a> {
    pub fn new(source: &'a Source, timeline: &'a dashboard_query::timeline::Timeline, context: &LoadContext, warnings: &mut Vec<String>) -> io::Result<Self> {
        let localisation = localisation(source, context.content.as_ref(), warnings)?;
        Ok(Self { source, timeline, content: context.content.clone(), localisation, evaluator: None, runs: HashMap::new() })
    }

    pub fn at(&mut self, tag: &str, hour: i32) -> io::Result<ResolvedCountryLook> {
        let Some(stretch) = self.timeline.at(hour) else {
            let reason = self.timeline.reason(hour);
            return Ok(ResolvedCountryLook { name: None, color: None, name_missing: Some(reason.clone()), color_missing: Some(reason) });
        };
        let run = match self.runs.entry(stretch.run) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => slot.insert(IndexedRunLooks::new(cached_looks(self.source, stretch.run)?, None)),
        };
        let source = self.source;
        let content = self.content.as_ref();
        let evaluator = &mut self.evaluator;
        Ok(run.resolve(tag, hour, self.localisation.as_deref(), |key, text, slot| {
            let eval = evaluator.get_or_insert_with(|| ScriptedEvaluator::for_source(source, content));
            eval.expand(stretch.run, hour, slot, key, text)
        }))
    }
}

pub struct BranchLooks {
    timeline: dashboard_query::timeline::Timeline,
    localisation: Option<Arc<Localisation>>,
    evaluator: Option<ScriptedEvaluator>,
    runs: HashMap<u32, IndexedRunLooks>,
}

impl BranchLooks {
    pub fn load(source: &Source, timeline: &dashboard_query::timeline::Timeline, context: &LoadContext, warnings: &mut Vec<String>) -> io::Result<Self> {
        let localisation = localisation(source, context.content.as_ref(), warnings)?;
        let mut runs = HashMap::new();
        for stretch in &timeline.stretches {
            if let std::collections::hash_map::Entry::Vacant(slot) = runs.entry(stretch.run) {
                slot.insert(IndexedRunLooks::new(cached_looks(source, stretch.run)?, localisation.as_deref()));
            }
        }
        let evaluator = runs
            .values()
            .any(IndexedRunLooks::has_scripted_names)
            .then(|| ScriptedEvaluator::for_source(source, context.content.as_ref()));
        Ok(Self { timeline: timeline.clone(), localisation, evaluator, runs })
    }

    pub fn next_change(&self, hour: i32) -> i32 {
        self.timeline.at(hour).and_then(|stretch| self.runs.get(&stretch.run)).map_or(i32::MAX, |run| run.next_change(hour))
    }

    pub fn next_for(&self, tag: &str, hour: i32) -> Option<i32> {
        let stretch = self.timeline.at(hour)?;
        self.runs.get(&stretch.run)?.next_for(tag, hour)
    }

    pub fn at(&self, tag: &str, hour: i32) -> ResolvedCountryLook {
        let Some(stretch) = self.timeline.at(hour) else {
            let reason = self.timeline.reason(hour);
            return ResolvedCountryLook { name: None, color: None, name_missing: Some(reason.clone()), color_missing: Some(reason) };
        };
        match self.runs.get(&stretch.run) {
            Some(run) => run.resolve(tag, hour, self.localisation.as_deref(), |key, text, slot| {
                match self.evaluator.as_ref() {
                    Some(eval) => eval.expand(stretch.run, hour, slot, key, text),
                    None => Err(format!("localisation {key} uses unsupported scripted text")),
                }
            }),
            None => ResolvedCountryLook { name: None, color: None, name_missing: Some(UNKNOWN.to_owned()), color_missing: Some(UNKNOWN.to_owned()) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_style_request_naming_1500_countries_is_valid() {
        let params = CountryStyleParams { branch_id: "1".into(), hour: 0, tags: (0..1_500).map(|at| format!("X{at:04}")).collect() };
        assert_eq!(CountryStyles::validate(&params), Ok(()));
    }

    #[test]
    fn two_hours_for_the_same_tag_keep_distinct_recorded_colours() {
        let red = Look { colour: Some([200, 40, 40]), ..Look::default() };
        let blue = Look { colour: Some([40, 80, 220]), ..Look::default() };
        let run = Arc::new(RunLooks {
            spans: vec![(1, 10, 0, 71), (1, 20, 72, 200), (2, 10, 12, 21)],
            unknown: vec![(212, 216)],
            looks: HashMap::from([(10, red), (20, blue)]),
            raw_looks: HashMap::new(),
            tags: HashMap::from([(1, Arc::from("GER")), (2, Arc::from("USA"))]),
        });
        let indexed = IndexedRunLooks::new(run, None);
        let no_script = |key: &str, _: &str, _: u32| Err(format!("localisation {key} uses unsupported scripted text"));
        assert_eq!(indexed.resolve("GER", 0, None, no_script).color, Some([200, 40, 40]));
        assert_eq!(indexed.resolve("GER", 72, None, no_script).color, Some([40, 80, 220]));
        assert_eq!([indexed.next_change(0), indexed.next_change(72), indexed.next_change(200), indexed.next_change(201)], [12, 201, 201, 212], "a name or colour holds until the next look starts or ends");
        assert_eq!([indexed.next_for("GER", 0), indexed.next_for("USA", 0), indexed.next_for("USA", 12), indexed.next_for("GER", 210)], [Some(72), Some(12), Some(22), Some(212)]);
        let missing = indexed.resolve("GER", 250, None, no_script);
        assert_eq!(missing.color, None);
        assert_eq!(missing.color_missing.as_deref(), Some(NOT_A_COUNTRY));
    }

    #[test]
    fn scripted_country_name_evaluates_bounds_validity_and_preserves_key_on_failure() {
        use content_reader_country_names::country_names::Operands;
        let loc: Localisation = [
            ("AAA_fascism".to_owned(), "[GetDate] Reich".to_owned()),
            ("BBB".to_owned(), "Plain Republic".to_owned()),
        ]
        .into_iter()
        .collect();
        let scripted_look = Look {
            slot: 1,
            chain: vec![(1, Ok(Operands::for_tag("AAA", Some("fascism"))))],
            colour: Some([10, 20, 30]),
            ..Look::default()
        };
        let plain_look = Look {
            slot: 2,
            chain: vec![(2, Ok(Operands::for_tag("BBB", None)))],
            colour: Some([40, 50, 60]),
            ..Look::default()
        };
        let resolved = resolve_look_name(&scripted_look, &loc, |key, text, slot| {
            assert_eq!((key, text, slot), ("AAA_fascism", "[GetDate] Reich", 1));
            Ok("Northern Reich".to_owned())
        });
        assert_eq!(resolved, Ok("Northern Reich".to_owned()));
        let failed = resolve_look_name(&scripted_look, &loc, |key, _, _| {
            Err(format!("localisation {key}: scripted trigger unavailable"))
        });
        let err = failed.unwrap_err();
        assert!(err.contains("AAA_fascism") && err.contains("scripted trigger unavailable"), "{err}");
        let plain = resolve_look_name(&plain_look, &loc, |_, _, _| panic!("plain name never invokes scripted evaluator"));
        assert_eq!(plain, Ok("Plain Republic".to_owned()));

        let run = Arc::new(RunLooks {
            spans: vec![(1, 10, 0, 48), (2, 20, 0, 48)],
            unknown: Vec::new(),
            looks: HashMap::from([(10, scripted_look), (20, plain_look)]),
            raw_looks: HashMap::new(),
            tags: HashMap::from([(1, Arc::from("AAA")), (2, Arc::from("BBB"))]),
        });
        let indexed = IndexedRunLooks::new(run, Some(&loc));
        assert!(indexed.has_scripted_names());
        assert_eq!(indexed.next_change(10), 11, "scripted look invalidates on the next hour");
        assert_eq!(indexed.next_for("AAA", 10), Some(11), "scripted tag re-reads at the next hour even inside one look span");
        assert_eq!(indexed.next_for("BBB", 10), Some(49), "plain tag keeps its full look span");
        let h10 = indexed.resolve("AAA", 10, Some(&loc), |_, _, _| Ok("Day 1 Reich".to_owned()));
        let h34 = indexed.resolve("AAA", 34, Some(&loc), |_, _, _| Ok("Day 2 Reich".to_owned()));
        assert_ne!(h10.name, h34.name, "two hours sharing one look evaluate distinct scripted names");

        let (cat, _dir) = dashboard_base::catalogue::tests::folder_catalogue(&[
            ("common/defines/synthetic.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=7}}"),
            (
                "localisation/english/date_l_english.yml",
                "\u{feff}l_english:\n AAA_fascism:0 \"[GetDate] Reich\"\n",
            ),
        ]);
        let eval = ScriptedEvaluator {
            read_tables: Box::new(|_, _, tables| panic!("date-only scripted name must not read replay tables: {tables:?}")),
            catalogue: Ok(Arc::new(cat)),
            frames: Mutex::new(BoundedFrames::new()),
        };
        let d0 = eval.expand(0, 0, 1, "AAA_fascism", "[GetDate] Reich").unwrap();
        let d24 = eval.expand(0, 24, 1, "AAA_fascism", "[GetDate] Reich").unwrap();
        assert_ne!(d0, d24, "real [GetDate] expansion distinguishes two hours with zero table reads ({d0:?} vs {d24:?})");
        assert!(d0.ends_with(" Reich") && d24.ends_with(" Reich"), "{d0:?} vs {d24:?}");
    }
}

enum RunKey {}

type Runs = dashboard_base::queries::RunCache<RunKey, RunLooks>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    Runs::forget(source);
    source.state::<CachedFlagFolders>().lock().unwrap_or_else(|error| error.into_inner()).0 = None;
}

pub fn cached_looks(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<RunLooks>> {
    Runs::held(&source.state::<Runs>(), run, || run_looks(source.replay_ref().recording(), run))
}
