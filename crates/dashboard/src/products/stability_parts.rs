//! Each part of one country's displayed stability and war support over a
//! branch, for the drill-down's parts-over-time chart (mock-up drill-1).
//!
//! A part is what the breakdown shows at a read ([`country_stability`]): a
//! formula term, one modifier tree line (by the identity the recorder proved,
//! or an unpaired line's recorded text), what the lines do not add up to, or
//! the clamp. A read's parts add up to its displayed value. Parts are known
//! only at hours whose read has every term and recorded its lines with its
//! inputs (`observed`); there a part without a run is zero. Elsewhere nothing
//! is known, never zero. A run never bridges an unrecorded hour or a change of
//! country identity.

use super::{
    CountryParams, LoadContext, Product,
    country_groups::HourSpan,
    country_stability::{self, BreakdownLine, FormulaBreakdown, StabilityBreakdown},
    politics_content::{PoliticsContent, politics_content},
    recorded_rows,
};
use crate::{
    Source,
    metadata::{tagged_country, validate_campaign},
    products::country_measure::FixedRun,
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    frame::{SharedView, TableView},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct StabilityParts;

/// Runs kept per country before the scan refuses.
const MAX_RUNS: usize = 400_000;
/// Recorded hours whose character tables are kept to name a read's characters.
const NAMED_HOURS: usize = 24 * 8;
/// Countries whose parts are kept per source.
const CACHED_COUNTRIES: usize = 4;
/// The formula term each measure's modifier lines replace, and their key.
const LINES_OF: [(&str, &str); 2] = [
    ("stabilityFactor", "stability_factor"),
    ("warSupportFactor", "war_support_factor"),
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StabilityPart {
    /// Stable within a reply: `term:<term>`, `line:<kind>:<identity>:<id>:<secondary>`,
    /// `unpaired:<text>`, `remainder` or `clamp`.
    pub id: String,
    /// The formula term (`base`, `war`, …) of a term part.
    pub term: Option<String>,
    /// A line part's latest line, named as the breakdown names it.
    pub line: Option<BreakdownLine>,
    /// Its value, fixed5, where it is not zero.
    pub runs: Vec<FixedRun>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureParts {
    pub parts: Vec<StabilityPart>,
    /// Hours whose parts are known; outside them nothing is.
    pub observed: Vec<HourSpan>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StabilityPartsReply {
    pub branch_id: String,
    pub tag: String,
    /// Decimal places of every raw value in this reply.
    pub decimals: u8,
    pub stability: MeasureParts,
    pub war_support: MeasureParts,
    pub warnings: Vec<String>,
}

/// A character line's character as recorded: script token, saved name and
/// template.
type Character = (
    Option<String>,
    String,
    Result<installed_content::characters::BuiltTemplate, String>,
);

/// One part's value at a read.
#[derive(Clone)]
struct Value {
    id: String,
    term: Option<String>,
    line: Option<BreakdownLine>,
    character: Option<Character>,
    raw: i64,
}

/// A measure's parts at one read, or None when they are not all known. A
/// character line's character is read from the frame the read is first seen
/// at, and a renamed character is its own part, named as it was.
fn parts_of(
    formula: &FormulaBreakdown,
    breakdown: &StabilityBreakdown,
    lines_key: &str,
    lines_term: &str,
    character: &dyn Fn(&BreakdownLine) -> Option<Character>,
) -> Option<Vec<Value>> {
    let mut values: Vec<Value> = Vec::new();
    let mut add = |id: String,
                   term: Option<String>,
                   line: Option<BreakdownLine>,
                   character: Option<Character>,
                   raw: i64| {
        if raw == 0 {
            return;
        }
        match values.iter_mut().find(|value| value.id == id) {
            // Two lines of one source add up, as the game's total does.
            Some(value) => value.raw = value.raw.wrapping_add(raw),
            None => values.push(Value {
                id,
                term,
                line,
                character,
                raw,
            }),
        }
    };
    let modifier = breakdown
        .modifiers
        .iter()
        .find(|modifier| modifier.key == lines_key);
    for term in &formula.terms {
        let raw: i64 = term.value_raw.as_deref()?.parse().ok()?;
        if term.term == lines_term {
            // The modifier term is its lines, or nothing is known of its parts.
            let modifier = modifier.filter(|_| breakdown.lines_available)?;
            for line in &modifier.lines {
                let recorded = character(line);
                let id = match (&*line.kind, &recorded) {
                    ("unpaired", _) => {
                        format!("unpaired:{}", line.text.as_deref().unwrap_or_default())
                    }
                    (_, Some((script, saved, _))) => format!(
                        "line:{}:{}:{}:{}:{}:{saved}",
                        line.kind,
                        line.identity,
                        line.identity_id,
                        line.secondary_raw,
                        script.as_deref().unwrap_or_default()
                    ),
                    _ => format!(
                        "line:{}:{}:{}:{}",
                        line.kind, line.identity, line.identity_id, line.secondary_raw
                    ),
                };
                add(
                    id,
                    None,
                    Some(line.clone()),
                    recorded,
                    line.value_raw.parse().ok()?,
                );
            }
            if let Some(remainder) = &modifier.remainder_raw {
                add(
                    "remainder".into(),
                    None,
                    None,
                    None,
                    remainder.parse().ok()?,
                );
            }
            continue;
        }
        add(
            format!("term:{}", term.term),
            Some(term.term.clone()),
            None,
            None,
            raw,
        );
    }
    if let Some(clamp) = &formula.clamp_raw {
        add("clamp".into(), None, None, None, clamp.parse().ok()?);
    }
    formula.value_raw.as_ref()?;
    Some(values)
}

#[derive(Default)]
struct Part {
    term: Option<String>,
    line: Option<BreakdownLine>,
    character: Option<Character>,
    /// The line's country at its hour, which idea names need.
    country: super::NamingCountry,
    runs: Vec<FixedRun>,
}

#[derive(Default)]
struct Measure {
    parts: BTreeMap<String, Part>,
    observed: Vec<HourSpan>,
}

impl Measure {
    fn record(
        &mut self,
        hour: i32,
        joined: bool,
        values: &[Value],
        country: &super::NamingCountry,
        runs: &mut usize,
    ) {
        match self.observed.last_mut() {
            Some(span) if joined && span.last_hour.checked_add(1) == Some(hour) => {
                span.last_hour = hour;
            }
            _ => self.observed.push(HourSpan {
                first_hour: hour,
                last_hour: hour,
            }),
        }
        for value in values {
            let part = self.parts.entry(value.id.clone()).or_default();
            part.term.clone_from(&value.term);
            if value.line.is_some() {
                part.line.clone_from(&value.line);
                part.character.clone_from(&value.character);
                part.country.clone_from(country);
            }
            let raw = value.raw.to_string();
            match part.runs.last_mut() {
                Some(run)
                    if joined
                        && run.last_hour.checked_add(1) == Some(hour)
                        && run.value_raw == raw =>
                {
                    run.last_hour = hour;
                }
                last => {
                    let follows = joined
                        && last.is_some_and(|run| run.last_hour.checked_add(1) == Some(hour));
                    *runs += 1;
                    part.runs.push(FixedRun {
                        first_hour: hour,
                        last_hour: hour,
                        value_raw: raw,
                        starts_segment: !follows,
                    });
                }
            }
        }
    }
}

/// The parts of the read in use: the identity and read hour they belong to, and
/// each measure's parts (None when not all known).
type ReadParts = ((u32, u32), i32, [Option<Vec<Value>>; 2]);

/// One country's parts over a branch, before lines are named.
#[derive(Default)]
struct Scanned {
    measures: [Measure; 2],
    warnings: Vec<String>,
}

fn scan(replay: &Replay, branch: u64, tag: &str, cancelled: &AtomicBool) -> io::Result<Scanned> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryStabilityInputs,
        TableId::CountryStabilityLines,
        TableId::ModifierDefinitions,
        TableId::SliceReads,
        TableId::Characters,
        TableId::LexicalTokens,
        TableId::CountryAppearance,
        // Whether a dynamic country is in civil war (super::civil_war).
        TableId::CountryCivilWars,
        TableId::CountryWars,
        TableId::WarRelations,
        TableId::CountryCounterInputs,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Stability request cancelled");
    let mut result = Scanned::default();
    let mut last: Option<((u32, u32), i32)> = None;
    // The parts of the read in use, kept while it is carried and its lines were
    // recorded (a read whose lines were not is asked again at its next hour).
    let mut current: Option<ReadParts> = None;
    // Reads whose lines were missing at some hour, and reads that had them at one.
    let mut unlined: BTreeSet<i32> = BTreeSet::new();
    let mut lined: BTreeSet<i32> = BTreeSet::new();
    // Each recent hour's Characters and LexicalTokens (shared views, not copies), so
    // a read's characters are named as recorded at the read itself, as the hover
    // breakdown names them. A read older than these is left unnamed.
    let mut recent_names: VecDeque<(i32, SharedView, SharedView)> = VecDeque::new();
    let mut runs = 0_usize;
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            let hour = frame.hour;
            recent_names.push_back((
                hour,
                decoded.tables[TableId::Characters.index()].clone(),
                decoded.tables[TableId::LexicalTokens.index()].clone(),
            ));
            while recent_names.len() > NAMED_HOURS {
                recent_names.pop_front();
            }
            let (Ok(countries), Ok(links), Ok(inputs), Ok(reads)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
                recorded_rows::<tables::CountryStabilityInputs>(decoded),
                decoded.complete::<tables::SliceReads>(),
            ) else {
                continue;
            };
            let Some((country, slot)) = tagged_country(countries, links, tag).found() else {
                continue;
            };
            let Ok(index) = inputs.binary_search_by_key(&country.id, |row| row.country) else {
                continue;
            };
            let row = &inputs[index];
            let Some(read) =
                replay::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)
            else {
                continue;
            };
            let identity = (slot, country.id);
            let joined =
                last.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            last = Some((identity, hour));
            // Parts are known at an hour only if its lines table holds this read's
            // lines, whether or not the read's parts were worked out before.
            let lines_read =
                replay::sliced::read_hour(reads, TableId::CountryStabilityLines, row.country);
            if decoded.complete::<tables::CountryStabilityLines>().is_err()
                || lines_read.is_some_and(|lines_read| lines_read != read)
            {
                unlined.insert(read);
                continue;
            }
            if !current
                .as_ref()
                .is_some_and(|(seen, at, _)| *seen == identity && *at == read)
            {
                let mut notes = Vec::new();
                let breakdown = country_stability::breakdown_of(
                    country_stability::BreakdownInput {
                        decoded,
                        hour,
                        row,
                        slot,
                        read_hour: read,
                        reads,
                        tag,
                    },
                    &mut notes,
                );
                let at_read = recent_names.iter().find(|(at, _, _)| *at == read);
                let characters = at_read.and_then(|(_, view, _)| match &**view {
                    TableView::Complete(rows) => rows.rows::<tables::Characters>(),
                    _ => None,
                });
                let lexical = at_read.and_then(|(_, _, view)| match &**view {
                    TableView::Complete(rows) => rows.rows::<tables::LexicalTokens>(),
                    _ => None,
                });
                let character = |line: &BreakdownLine| {
                    if !matches!(&*line.kind, "advisor" | "countryLeader") {
                        return None;
                    }
                    let found = country_stability::recorded_character(line, characters, lexical)?;
                    Some((
                        found.script.map(str::to_owned),
                        found.saved.to_owned(),
                        found.template,
                    ))
                };
                let values = [
                    (&breakdown.stability, LINES_OF[0]),
                    (&breakdown.war_support, LINES_OF[1]),
                ]
                .map(|(formula, (term, key))| parts_of(formula, &breakdown, key, term, &character));
                lined.insert(read);
                current = Some((identity, read, values));
            }
            let Some((_, _, values)) = current.as_ref() else {
                continue;
            };
            let naming = super::NamingCountry::read(decoded, country);
            for (measure, values) in result.measures.iter_mut().zip(values.iter()) {
                if let Some(values) = values {
                    measure.record(hour, joined, values, &naming, &mut runs);
                }
            }
            if runs > MAX_RUNS {
                return Err(io::Error::other(
                    "Stability parts exceed their memory budget",
                ));
            }
        }
    }
    let missing = unlined.difference(&lined).count();
    if missing > 0 {
        result.warnings.push(format!(
            "{tag}: {missing} reads recorded no modifier lines with their inputs; their parts are not known."
        ));
    }
    Ok(result)
}

/// Names each line part from content, a character as it was recorded for that
/// part, as the hover breakdown names it.
fn named(measure: &Measure, content: Option<&PoliticsContent>, tag: &str) -> MeasureParts {
    let mut parts = Vec::with_capacity(measure.parts.len());
    for (id, part) in &measure.parts {
        let mut line = part.line.clone();
        if let (Some(line), Some(content)) = (line.as_mut(), content) {
            let character = part.character.as_ref().map(|(script, saved, template)| {
                country_stability::RecordedCharacter {
                    script: script.as_deref(),
                    saved,
                    template: template.clone(),
                }
            });
            (line.group, line.group_kind) =
                country_stability::line_group(line, content, character.as_ref());
            // No single hour here: a closed `[...]` is refused (see line_name).
            let country = part.country.for_tag(tag);
            line.label = country_stability::line_name(line, content, character, None, country);
        }
        parts.push(StabilityPart {
            id: id.clone(),
            term: part.term.clone(),
            line,
            runs: part.runs.clone(),
        });
    }
    MeasureParts {
        parts,
        observed: measure.observed.clone(),
    }
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<Scanned>)>,
}

impl Product for StabilityParts {
    const KEY: &'static str = "stability-parts";
    type Params = CountryParams;
    type Reply = StabilityPartsReply;

    fn types() -> Vec<String> {
        vec![
            StabilityPart::decl(),
            MeasureParts::decl(),
            StabilityPartsReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch();
        let key = (branch, params.tag.clone());
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == key)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        // Scan outside the lock, so other countries' requests are not held up.
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(&source.replay(), branch, &params.tag, cancelled)?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_COUNTRIES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        let mut warnings = scanned.warnings.clone();
        // Lines are named in the viewer's language when content is configured;
        // without it they keep their identities and recorded text.
        let content = match context.content.as_ref() {
            Some(options) => match politics_content(source, options) {
                Ok(content) => {
                    warnings.extend(content.warnings.iter().cloned());
                    Some(content)
                }
                Err(error) => {
                    warnings.push(format!("Breakdown lines are not named: {error}"));
                    None
                }
            },
            None => None,
        };
        let [stability, war_support] = &scanned.measures;
        let stability = named(stability, content.as_deref(), &params.tag);
        let war_support = named(war_support, content.as_deref(), &params.tag);
        // Line names left out for a localisation refusal, by reason and key.
        if let Some(content) = &content {
            warnings.extend(content.catalogue.refusal_lines());
        }
        Ok(StabilityPartsReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: 5,
            stability,
            war_support,
            warnings,
        })
    }
}

#[cfg(test)]
pub(crate) fn scan_parts(
    replay: &Replay,
    branch: u64,
    tag: &str,
) -> io::Result<(MeasureParts, MeasureParts, Vec<String>)> {
    let scanned = scan(replay, branch, tag, &AtomicBool::new(false))?;
    let [stability, war_support] = &scanned.measures;
    Ok((
        named(stability, None, tag),
        named(war_support, None, tag),
        scanned.warnings.clone(),
    ))
}

#[cfg(test)]
mod tests;
