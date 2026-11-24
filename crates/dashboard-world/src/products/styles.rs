
use dashboard_base::products::{BranchParams, ContentOptions, LoadContext, Product};
use dashboard_core::protocol::validate_tag;
use dashboard_base::{Source, catalogue::plain_text, content};
use dashboard_core::invalid;
use dashboard_text::flag_choices;
use crate::products::country_flags::{CountryFlagChoice, FlagFolders};
use dashboard_query::{Query, read};
use content_reader_country_names::country_names::CountryNames;
use content_reader_localisation::localisation::Localisation;
use replay_looks::Look;
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{Recorded, TableId, TagHolderRow};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, atomic::AtomicBool},
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

pub(crate) fn style(tag: &str, look: &Look, localisation: Option<&Localisation>) -> (CountryStyle, Option<String>) {
    let (name, unnamed) = match localisation {
        None => (None, None),
        Some(localisation) => {
            let names = CountryNames::recorded(localisation, look.chain.iter().cloned().collect(), look.built);
            match country_name(&names, look.slot) {
                Ok(name) => (Some(name), None),
                Err(reason) => (None, Some(reason)),
            }
        }
    };
    let problem = match (unnamed, look.colour) {
        (Some(reason), None) => Some(format!("{reason}; {NO_COLOUR}")),
        (Some(reason), Some(_)) => Some(reason),
        (None, None) => Some(NO_COLOUR.to_owned()),
        (None, Some(_)) => None,
    };
    (CountryStyle { tag: tag.to_owned(), name, color: look.colour }, problem)
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

pub(crate) fn localisation(source: &Source, content: Option<&ContentOptions>, warnings: &mut Vec<String>) -> io::Result<Option<Arc<Localisation>>> {
    let localisation = content::name_localisation(source, content);
    match (&localisation, content) {
        (Err(reason), _) => warnings.push(reason.clone()),
        (Ok(_), Some(content)) => content::check_replay(source.replay_ref(), content.allow_unverified, warnings)?,
        (Ok(_), None) => {}
    }
    Ok(localisation.ok())
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
    let localisation = localisation(source, context.content.as_ref(), &mut warnings)?;
    let folders = flag_folders(source, context.content.as_ref())?.map_err(|reason| CountryFlagChoice {
        keys: [None, None, None],
        reasons: [Some(reason.clone()), Some(reason.clone()), Some(reason)],
    });
    let (mut names, mut reasons) = (Dictionary::default(), Dictionary::default());
    let mut flags: Vec<CountryFlagChoice> = Vec::new();
    let mut flag_numbers: HashMap<CountryFlagChoice, i32> = HashMap::new();
    let mut known: Vec<[i32; 2]> = Vec::new();
    let mut runs_read: HashMap<u32, Arc<RunLooks>> = HashMap::new();
    let mut unique_index: HashMap<(Arc<str>, Box<[u8]>), usize> = HashMap::new();
    let mut unique_looks: Vec<(Arc<str>, u32, u32)> = Vec::new();
    let mut raw_segments: HashMap<Arc<str>, Vec<(i32, i32, usize)>> = HashMap::new();
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
            raw_segments.entry(Arc::clone(tag)).or_default().push((first, last, id));
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
    let mut resolved_facts: Vec<[i32; 4]> = Vec::with_capacity(unique_looks.len());
    for (idx, (tag, run_id, look_number)) in unique_looks.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let look = &runs_read[run_id].looks[look_number];
        let (style, problem) = style(tag, look, localisation.as_deref());
        let choice = match &folders {
            Ok(folders) => folders.choose(&chains[idx])?,
            Err(none) => none.clone(),
        };
        let flag = *flag_numbers.entry(choice).or_insert_with_key(|choice| {
            flags.push(choice.clone());
            flags.len() as i32 - 1
        });
        resolved_facts.push([
            style.name.as_deref().map_or(-1, |name| names.number(name)),
            style.color.map_or(-1, |[red, green, blue]| (i32::from(red) << 16) | (i32::from(green) << 8) | i32::from(blue)),
            problem.map_or(-1, |problem| reasons.number(&problem)),
            flag,
        ]);
    }
    let segments: HashMap<Arc<str>, Vec<(i32, i32, Option<[i32; 4]>)>> = raw_segments
        .into_iter()
        .map(|(tag, held)| (tag, held.into_iter().map(|(first, last, id)| (first, last, Some(resolved_facts[id]))).collect()))
        .collect();
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
        let localisation = localisation(source, context.content.as_ref(), &mut warnings)?;
        let stretch = query.timeline.at(params.hour).copied().ok_or_else(|| invalid("The timeline holds no such hour"))?;
        let run = crate::products::styles::cached_looks(source, stretch.run)?;
        let known = !run.unknown.iter().any(|&(a, b)| a <= params.hour && params.hour <= b);
        let mut at: HashMap<&str, &Look> = HashMap::new();
        for &(number, look, first, last) in &run.spans {
            if first <= params.hour && params.hour <= last
                && let (Some(tag), Some(look)) = (run.tags.get(&number), run.looks.get(&look))
            {
                at.insert(tag, look);
            }
        }
        let styles = params
            .tags
            .iter()
            .map(|tag| {
                let missing = |reason: &str, warnings: &mut Vec<String>| {
                    warnings.push(format!("{tag}: {reason}"));
                    CountryStyle { tag: tag.clone(), name: None, color: None }
                };
                match at.get(tag.as_str()) {
                    Some(look) => {
                        let (style, problem) = style(tag, look, localisation.as_deref());
                        warnings.extend(problem.map(|problem| format!("{tag}: {problem}")));
                        style
                    }
                    _ if !known => missing(UNKNOWN, &mut warnings),
                    _ => missing(NOT_A_COUNTRY, &mut warnings),
                }
            })
            .collect();
        Ok(CountryStyleReply {
            branch_id: params.branch_id.clone(),
            hour: params.hour,
            styles,
            warnings,
        })
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
