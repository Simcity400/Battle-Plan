//! Each part of one country's displayed stability and war support over a
//! branch, for the drill-down's parts-over-time chart.
//!
//! A part is what the breakdown shows at a read ([`country_stability`]): a
//! formula term, one modifier tree line (by the identity the recorder proved,
//! or an unpaired line's recorded text), what the lines do not add up to, or
//! the clamp. A read's parts add up to its displayed value. Parts are known
//! only at hours whose read has every term and recorded its lines with its
//! inputs (`observed`); there a part without a run is zero. Elsewhere nothing
//! is known, never zero. A run never bridges an unrecorded hour or a change of
//! country identity.

use super::country_stability::{self, BreakdownLine, FormulaBreakdown, StabilityBreakdown};

use dashboard_base::{Source, products::country_measure::FixedRun};
use dashboard_base::{
    products::CountryParams,
    products::LoadContext,
    products::Product,
    products::country_groups::HourSpan,
    products::politics_content::{PoliticsContent, politics_content},
};
use dashboard_text::{
    identity::NamingCountry,
    metadata::{mapped_slot, validate_campaign},
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
mod progressive;

/// Runs kept per country before the scan refuses.
const MAX_RUNS: usize = 400_000;
/// Recorded hours whose character tables are kept to name a read's characters.
const NAMED_HOURS: usize = 24 * 8;
/// Countries whose parts are kept per source.
const CACHED_COUNTRIES: usize = 4;
/// The formula terms and recorded modifier names (`engine::country_stability::KEY_NAMES`).
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
    Result<(String, Vec<u8>, i32, bool), String>,
);

/// One part's value at a read.
#[derive(Clone, Serialize, Deserialize)]
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

#[derive(Default, Serialize, Deserialize)]
struct Part {
    term: Option<String>,
    line: Option<BreakdownLine>,
    character: Option<Character>,
    /// The line's country at its hour, which idea names need.
    country: NamingCountry,
    runs: Vec<FixedRun>,
}

impl Part {
    fn update(&mut self, value: &Value, country: &NamingCountry) {
        self.term.clone_from(&value.term);
        if value.line.is_some() {
            self.line.clone_from(&value.line);
            self.character.clone_from(&value.character);
            self.country.clone_from(country);
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
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
        country: &NamingCountry,
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
            part.update(value, country);
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
#[derive(Default, Serialize, Deserialize)]
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
        // Whether a dynamic country is in civil war (dashboard_text::identity::civil_war).
        TableId::CountryCivilWars,
        TableId::CountryWars,
        TableId::WarRelations,
        TableId::CountryCounterInputs,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Stability request cancelled");
    let mut result = Scanned::default();
    let mut observer = Scan::default();
    let mut runs = 0_usize;
    let selected_tag = BTreeSet::from([tag]);
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
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
                    observer.step(
                        decoded,
                        Some(&selected_tag),
                        None,
                        |_, _, _, joined, values, naming| {
                            for (measure, values) in result.measures.iter_mut().zip(values) {
                                if let Some(values) = values {
                                    measure.record(frame.hour, joined, values, naming, &mut runs);
                                }
                            }
                            Ok(())
                        },
                    )?;
                    if runs > MAX_RUNS {
                        return Err(io::Error::other(
                            "Stability parts exceed their memory budget",
                        ));
                    }
                }
            }
            Ok(())
        },
    )?;
    result.warnings = observer.warnings(tag);
    Ok(result)
}

#[derive(Default, Serialize, Deserialize)]
struct Track {
    last: Option<((u32, u32), i32)>,
    current: Option<ReadParts>,
    unlined: BTreeSet<i32>,
    lined: BTreeSet<i32>,
    available: bool,
    naming: Option<NamingCountry>,
}
#[derive(Default, Serialize, Deserialize)]
struct Scan {
    #[cfg(test)]
    calculated_reads: usize,
    tracks: BTreeMap<String, Track>,
    #[serde(with = "recent_views")]
    recent_names: VecDeque<(i32, SharedView, SharedView)>,
    hour: Option<i32>,
    active: BTreeSet<String>,
    #[serde(skip)]
    native: country_stability::Native,
    #[serde(skip)]
    lines: Option<BTreeMap<u32, Vec<replay::rows::CountryStabilityLineRow>>>,
    #[serde(skip)]
    reads: Option<Vec<replay::rows::SliceReadRow>>,
    #[serde(skip)]
    removed: Vec<String>,
    #[serde(skip)]
    clock_names: BTreeSet<u32>,
    #[cfg(test)]
    #[serde(skip)]
    naming_ticks: usize,
    #[serde(skip)]
    appearances: BTreeMap<u32, replay::rows::CountryAppearanceRow>,
    #[serde(skip)]
    line_dates: BTreeMap<u32, i32>,
}
impl Scan {
    fn step(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        selected: Option<&BTreeSet<&str>>,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
        mut sink: impl FnMut(
            &str,
            (u32, u32),
            i32,
            bool,
            &[Option<Vec<Value>>; 2],
            &NamingCountry,
        ) -> io::Result<()>,
    ) -> io::Result<()> {
        let hour = decoded.hour;
        let previous = self.hour;
        let adjacent = previous.and_then(|hour| hour.checked_add(1)) == Some(hour);
        self.hour = Some(hour);
        self.removed.clear();
        self.recent_names.push_back((
            hour,
            decoded.tables[TableId::Characters.index()].clone(),
            decoded.tables[TableId::LexicalTokens.index()].clone(),
        ));
        while self.recent_names.len() > NAMED_HOURS {
            self.recent_names.pop_front();
        }
        let mut dirty = self.native.refresh(decoded, changes, false);
        dirty.extend(self.clock_names.iter().copied());
        let changed =
            |table: TableId| changes.is_none_or(|changes| changes.rows_changed[table.index()]);
        if changed(TableId::SliceReads) {
            self.reads = decoded
                .complete::<tables::SliceReads>()
                .ok()
                .map(|rows| rows.to_vec());
            let next: BTreeMap<_, _> = self
                .reads
                .iter()
                .flatten()
                .filter(|row| row.table == TableId::CountryStabilityLines as u32)
                .map(|row| (row.owner, row.sample_hour))
                .collect();
            dirty.extend(
                self.line_dates
                    .keys()
                    .chain(next.keys())
                    .copied()
                    .filter(|id| self.line_dates.get(id) != next.get(id)),
            );
            self.line_dates = next;
        }
        if changed(TableId::CountryStabilityLines) {
            let next = decoded
                .complete::<tables::CountryStabilityLines>()
                .ok()
                .map(|rows| {
                    let mut by_country: BTreeMap<u32, Vec<_>> = BTreeMap::new();
                    for row in rows {
                        by_country.entry(row.country).or_default().push(row.clone());
                    }
                    by_country
                });
            #[cfg(test)]
            {
                self.native.work.row_visits += next
                    .as_ref()
                    .map_or(0, |rows| rows.values().map(Vec::len).sum());
            }
            if self.lines.is_some() != next.is_some() {
                dirty.extend(self.native.roster.keys().copied());
            } else {
                dirty.extend(
                    self.lines
                        .iter()
                        .chain(&next)
                        .flat_map(|rows| rows.keys())
                        .copied()
                        .filter(|id| {
                            self.lines.as_ref().and_then(|rows| rows.get(id))
                                != next.as_ref().and_then(|rows| rows.get(id))
                        }),
                );
            }
            self.lines = next;
        }
        let mut naming_changed = BTreeSet::new();
        if changed(TableId::CountryAppearance) {
            let rows = decoded.complete::<tables::CountryAppearance>().ok();
            #[cfg(test)]
            {
                self.native.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next: BTreeMap<_, _> = rows
                .into_iter()
                .flatten()
                .map(|row| {
                    (
                        row.country,
                        replay::rows::CountryAppearanceRow {
                            country: row.country,
                            name_tag: row.name_tag.clone(),
                            cosmetic_tag: row.cosmetic_tag.clone(),
                            original_tag: row.original_tag.clone(),
                            original_tag_reference: row.original_tag_reference,
                            ..Default::default()
                        },
                    )
                })
                .collect();
            naming_changed.extend(
                self.appearances
                    .keys()
                    .chain(next.keys())
                    .copied()
                    .filter(|id| self.appearances.get(id) != next.get(id)),
            );
            self.appearances = next;
        }
        if changed(TableId::ReservedCountryNaming) {
            naming_changed.extend(self.native.slots.get(&0).into_iter().flatten().copied());
        }
        dirty.extend(naming_changed.iter().copied());
        if !adjacent {
            dirty.extend(self.native.roster.keys().copied());
        }
        if self.native.membership_changed {
            let valid: BTreeSet<_> = self
                .native
                .roster
                .iter()
                .filter(|(id, _)| {
                    self.native
                        .inputs
                        .as_ref()
                        .is_some_and(|rows| rows.contains_key(id))
                        && self
                            .native
                            .reads
                            .as_ref()
                            .is_some_and(|rows| rows.contains_key(id))
                })
                .map(|(_, (tag, _))| tag.clone())
                .collect();
            self.removed.extend(self.active.difference(&valid).cloned());
            for tag in &self.removed {
                if let Some(track) = self.tracks.get_mut(tag) {
                    if let Some(((_, id), _)) = track.last {
                        self.clock_names.remove(&id);
                    }
                    track.last = None;
                }
            }
            self.active = valid;
        }
        let Some(reads) = self.reads.as_deref() else {
            return Ok(());
        };
        let mut seen = BTreeSet::new();
        for id in dirty {
            let Some(country) = self.native.country(id) else {
                continue;
            };
            let Some((tag, slot)) = self.native.roster.get(&id) else {
                continue;
            };
            let tag = tag.as_str();
            if selected.is_some_and(|wanted| !wanted.contains(tag)) || !seen.insert(tag) {
                continue;
            }
            let slot = *slot;
            let track = self.tracks.entry(tag.to_owned()).or_default();
            let Some(row) = self
                .native
                .inputs
                .as_ref()
                .and_then(|rows| rows.get(&country.id))
            else {
                continue;
            };
            let Some(read) =
                replay::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)
            else {
                continue;
            };
            let identity = (slot, country.id);
            if adjacent
                && self.active.contains(tag)
                && let Some((_, at)) = &mut track.last
            {
                *at = previous.expect("adjacent parts hour");
            }
            let joined = track
                .last
                .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            track.last = Some((identity, hour));
            // Parts are known at an hour only if its lines table holds this read's
            // lines, whether or not the read's parts were worked out before.
            let lines_read =
                replay::sliced::read_hour(reads, TableId::CountryStabilityLines, row.country);
            if decoded.complete::<tables::CountryStabilityLines>().is_err()
                || lines_read.is_some_and(|lines_read| lines_read != read)
            {
                track.unlined.insert(read);
                track.available = false;
                self.clock_names.remove(&id);
                self.removed.push(tag.to_owned());
                continue;
            }
            let fresh = !track
                .current
                .as_ref()
                .is_some_and(|(seen, at, _)| *seen == identity && *at == read);
            if fresh {
                #[cfg(test)]
                {
                    self.calculated_reads += 1;
                }
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
                let at_read = self.recent_names.iter().find(|(at, _, _)| *at == read);
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
                        found.template.map(|v| (v.tag, v.file, v.line, v.generated)),
                    ))
                };
                let values = [
                    (&breakdown.stability, LINES_OF[0]),
                    (&breakdown.war_support, LINES_OF[1]),
                ]
                .map(|(formula, (term, key))| parts_of(formula, &breakdown, key, term, &character));
                track.lined.insert(read);
                track.current = Some((identity, read, values));
            }
            let Some((_, _, values)) = track.current.as_ref() else {
                continue;
            };
            let clock_name = self.clock_names.contains(&id);
            #[cfg(test)]
            {
                self.naming_ticks += usize::from(clock_name);
            }
            let naming = if !naming_changed.contains(&id)
                && !clock_name
                && joined
                && let Some(naming) = &track.naming
            {
                naming.clone()
            } else {
                NamingCountry::read_with_appearance(decoded, country, self.appearances.get(&id))
            };
            if naming
                .look
                .as_ref()
                .is_some_and(|look| !look.name_tag.is_empty())
            {
                self.clock_names.insert(id);
            } else {
                self.clock_names.remove(&id);
            }
            if changes.is_some()
                && joined
                && !fresh
                && track.available
                && track.naming.as_ref() == Some(&naming)
            {
                continue;
            }
            track.available = true;
            track.naming = Some(naming.clone());
            #[cfg(test)]
            {
                self.native.work.emitted += 1;
            }
            sink(tag, identity, read, joined, values, &naming)?;
        }
        Ok(())
    }
    fn warnings(&self, tag: &str) -> Vec<String> {
        let missing = self
            .tracks
            .get(tag)
            .map_or(0, |track| track.unlined.difference(&track.lined).count());
        if missing == 0 {
            Vec::new()
        } else {
            vec![format!(
                "{tag}: {missing} reads recorded no modifier lines with their inputs; their parts are not known."
            )]
        }
    }
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
                    template: template.clone().map(|(tag, file, line, generated)| {
                        installed_content::characters::BuiltTemplate {
                            tag,
                            file,
                            line,
                            generated,
                        }
                    }),
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
    const CONTENT_DEPENDENT: bool = true;
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
            decimals: dashboard_base::products::country_measure::MEASURE_DECIMALS,
            stability,
            war_support,
            warnings,
        })
    }
}

#[cfg(test)]
#[test]
fn line_keys_are_the_engine_s_recorded_modifier_names() {
    for (_, key) in LINES_OF {
        assert!(
            engine::country_stability::KEY_NAMES.contains(&key.as_bytes()),
            "{key}"
        );
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

// Naming consumes rows at the retained hour, never SharedView acquisition dates.
// Save each row allocation once, including equal rows read at different dates.
// The existing checkpoint codec retains complete/truncated/unavailable inputs.
mod recent_views {
    use super::*;
    use replay::frame::DecodedFrame;
    type Names = VecDeque<(i32, SharedView, SharedView)>;
    type Archive = (Vec<(i32, usize, usize)>, Vec<(usize, Vec<u8>)>);
    pub fn serialize<S: serde::Serializer>(
        views: &Names,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut indices = BTreeMap::new();
        let mut snapshots = Vec::new();
        let mut save = |hour, table: TableId, view: &SharedView| -> io::Result<usize> {
            let next = snapshots.len();
            if let Some(index) = indices.get(&(table.index(), view.as_ptr())) {
                return Ok(*index);
            }
            let mut frame = DecodedFrame {
                branch: 0,
                hour,
                metrics: Default::default(),
                tables: std::array::from_fn(|_| {
                    TableView::Unavailable {
                        reason: "Unused naming input".into(),
                    }
                    .into()
                }),
            };
            frame.tables[table.index()] = view.clone();
            let mut bytes = Vec::new();
            frame.encode_checkpoint(&mut bytes)?;
            snapshots.push((
                table.index(),
                zstd::stream::encode_all(bytes.as_slice(), 3)?,
            ));
            indices.insert((table.index(), view.as_ptr()), next);
            Ok(next)
        };
        let hours = views
            .iter()
            .map(|(hour, characters, lexical)| {
                Ok((
                    *hour,
                    save(*hour, TableId::Characters, characters)?,
                    save(*hour, TableId::LexicalTokens, lexical)?,
                ))
            })
            .collect::<io::Result<Vec<_>>>()
            .map_err(serde::ser::Error::custom)?;
        (hours, snapshots).serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Names, D::Error> {
        let (hours, snapshots) = Archive::deserialize(deserializer)?;
        let snapshots = snapshots
            .into_iter()
            .map(|(table, bytes)| {
                let bytes = zstd::stream::decode_all(bytes.as_slice())?;
                let mut decoder = replay::codec::Decoder::new(&bytes);
                let frame = DecodedFrame::decode_checkpoint(&mut decoder)?;
                decoder.finish()?;
                frame
                    .tables
                    .get(table)
                    .cloned()
                    .ok_or_else(|| dashboard_core::invalid("Invalid naming table"))
            })
            .collect::<io::Result<Vec<_>>>()
            .map_err(serde::de::Error::custom)?;
        hours
            .into_iter()
            .map(|(hour, characters, lexical)| {
                Ok((
                    hour,
                    snapshots
                        .get(characters)
                        .cloned()
                        .ok_or_else(|| dashboard_core::invalid("Missing naming characters"))?,
                    snapshots
                        .get(lexical)
                        .cloned()
                        .ok_or_else(|| dashboard_core::invalid("Missing naming tokens"))?,
                ))
            })
            .collect::<io::Result<_>>()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    countries: BTreeMap<String, Scanned>,
    actors: BTreeMap<String, crate::prepared_stability::Actor>,
    last: i32,
    batch: dashboard_base::prepared::series::Accumulator,
    open: BTreeMap<String, BTreeSet<String>>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
}
impl Prepared {
    #[cfg(test)]
    pub(crate) fn calculated_reads(&self) -> usize {
        self.scan.calculated_reads
    }
    pub fn bytes(&self) -> usize {
        self.batch.bytes()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(&mut self, frame: &replay::frame::DecodedFrame) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, replay::format::FormatVersion::CURRENT);
        self.observe_changes(frame, &changes)
    }
    pub fn observe_changes(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.last = frame.hour;
        self.batch.advance(frame.hour)?;
        progressive::sample(
            &mut self.scan,
            frame,
            None,
            Some(changes),
            |tag, identity, read, joined, index, values, naming, samples| {
                let scanned = self.countries.entry(tag.into()).or_default();
                let measure = &mut scanned.measures[index];
                let field = ["stability", "warSupport"][index];
                let open = self
                    .open
                    .entry(dashboard_base::prepared::series::field_key(tag, field))
                    .or_default();
                let next: BTreeSet<_> = values
                    .into_iter()
                    .flatten()
                    .map(|value| value.id.clone())
                    .chain(values.map(|_| String::new()))
                    .collect();
                for id in open.difference(&next) {
                    let key = serde_json::to_string(&(tag, id))?;
                    if let Some(actor) = self
                        .actors
                        .get_mut(&dashboard_base::prepared::series::field_key(&key, field))
                    {
                        self.batch.observe(actor.key, frame.hour, None, false);
                        actor.holder = None;
                    }
                }
                *open = next;
                if let Some(values) = values {
                    for value in values {
                        measure
                            .parts
                            .entry(value.id.clone())
                            .or_default()
                            .update(value, naming);
                    }
                    for (id, carried) in std::iter::once(("", None))
                        .chain(values.iter().map(|value| (value.id.as_str(), Some(value))))
                    {
                        let key = serde_json::to_string(&(tag, id))?;
                        // Character identities retain the supplying read's earlier naming
                        // frame. All stateless part amounts come from the same sampler.
                        let raw = if carried.is_some_and(|value| {
                            progressive::needs_earlier_names(value, read, frame.hour)
                        }) {
                            carried.map(|value| i128::from(value.raw))
                        } else {
                            samples
                                .get(&(key.clone(), field.into()))
                                .and_then(|sample| sample.1)
                        };
                        crate::prepared_stability::record(
                            &mut self.actors,
                            &mut self.batch,
                            &key,
                            field,
                            frame.hour,
                            raw,
                            joined,
                            (identity.0, identity.1, read),
                            false,
                            None,
                        )?;
                    }
                }
                Ok(())
            },
        )?;
        for tag in &self.scan.removed {
            for field in ["stability", "warSupport"] {
                if let Some(open) = self
                    .open
                    .remove(&dashboard_base::prepared::series::field_key(tag, field))
                {
                    for id in open {
                        let name = serde_json::to_string(&(tag, id))?;
                        if let Some(actor) = self
                            .actors
                            .get_mut(&dashboard_base::prepared::series::field_key(&name, field))
                        {
                            self.batch.observe(actor.key, frame.hour, None, false);
                            actor.holder = None;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        crate::prepared_stability::flush(
            &self.actors,
            &mut self.batch,
            writer,
            ("parts", branch, self.last, 5),
            cancelled,
        )?;
        for (tag, scanned) in &mut self.countries {
            scanned.warnings = self.scan.warnings(tag);
        }
        writer.write_at(
            &format!("parts-{branch}-names"),
            0,
            serde_json::to_vec(&self.countries)?.as_slice(),
        )
    }
}

/// The generic series adapter owns raw values, summaries, navigation and event pages.
#[allow(clippy::too_many_arguments)]
pub fn prepared_query(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    product: &str,
    mut params: serde_json::Value,
    store: &store::Store,
    source_key: &store::SourceKey,
    coverage: Option<&dashboard_base::prepared::coverage::Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    use serde_json::{Value, json};
    let branch = params["branchId"]
        .as_str()
        .and_then(|branch| branch.parse::<u64>().ok())
        .ok_or_else(|| dashboard_core::invalid("Invalid parts branch"))?;
    let index = match params["family"]["measure"].as_str() {
        Some("stability") => 0,
        Some("warSupport") => 1,
        _ => return Err(dashboard_core::invalid("Unknown parts measure")),
    };
    let mut countries: BTreeMap<String, Scanned> = if coverage.is_some() {
        let file = store
            .read(source_key, &format!("parts-{branch}-names"), 0)?
            .ok_or_else(|| dashboard_core::invalid("Missing parts names"))?;
        serde_json::from_reader(io::BufReader::new(file))?
    } else {
        BTreeMap::new()
    };
    if product == "series-summary" {
        params["from"] = json!(params["from"].as_f64().map(f64::floor));
        params["to"] = json!(params["to"].as_f64().map(f64::ceil));
    }
    let sampled = progressive::read(
        source,
        coverage,
        product,
        &params,
        &mut countries,
        cancelled,
    )?;
    let content = content
        .map(|options| politics_content(source, options))
        .transpose();
    let mut warnings = match &content {
        Ok(Some(content)) => content.warnings.clone(),
        Err(error) => vec![format!("Breakdown lines are not named: {error}")],
        _ => Vec::new(),
    };
    let content = content.ok().flatten();
    let field = if product == "series-state" {
        "reads"
    } else {
        "tags"
    };
    let requests = params[field]
        .as_array()
        .ok_or_else(|| dashboard_core::invalid("Missing parts selection"))?;
    let mut expanded = Vec::new();
    let mut within = serde_json::Map::new();
    let mut metadata = BTreeMap::new();
    for request in requests {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let tag = request
            .as_str()
            .or_else(|| request["tag"].as_str())
            .ok_or_else(|| dashboard_core::invalid("Invalid parts tag"))?;
        dashboard_core::protocol::validate_tag(tag).map_err(dashboard_core::invalid)?;
        let empty = Scanned::default();
        let scanned = countries.get(tag).unwrap_or(&empty);
        warnings.extend(scanned.warnings.iter().cloned());
        let parts = named(&scanned.measures[index], content.as_deref(), tag);
        for id in std::iter::once("").chain(parts.parts.iter().map(|part| part.id.as_str())) {
            let key = serde_json::to_string(&(tag, id))?;
            if let Some(spans) = params["within"].get(tag) {
                within.insert(key.clone(), spans.clone());
            }
            expanded.push(if product == "series-state" {
                json!({"tag":key,"hour":request["hour"]})
            } else {
                json!(key)
            });
        }
        metadata.insert(tag.to_owned(), parts);
    }
    params[field] = Value::Array(expanded);
    params["within"] = Value::Object(within);
    let tail = (!coverage.is_some_and(|coverage| coverage.complete)).then(|| {
        let start = source
            .replay_ref()
            .branches
            .iter()
            .find(|start| start.id == branch)
            .unwrap();
        (
            dashboard_text::metadata::branch_bounds(source.replay_ref(), start),
            &sampled.0,
            sampled.1.as_slice(),
            &sampled.2,
            coverage.is_some(),
        )
    });
    let mut reply = dashboard_base::prepared_measure::query::query(
        product, params, store, source_key, tail, cancelled,
    )?;
    if let Some(states) = reply["states"].as_array_mut() {
        for state in states {
            let (tag, id): (String, String) = serde_json::from_str(
                state["tag"]
                    .as_str()
                    .ok_or_else(|| dashboard_core::invalid("Missing part state tag"))?,
            )?;
            let hour = state["hour"].as_i64().unwrap() as i32;
            if id.is_empty() && sampled.3.contains(&(tag, hour)) {
                state["pending"] = json!(true);
            }
        }
    }
    for (tag, _) in &sampled.3 {
        let warning = format!(
            "{tag}: advisor and country-leader parts need their supplying read's earlier character names; still being prepared."
        );
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    if let Some(content) = &content {
        warnings.extend(content.catalogue.refusal_lines());
    }
    if product != "series-next-change" {
        reply["parts"] = serde_json::to_value(metadata)?;
        reply["warnings"] = json!(warnings);
    }
    Ok(reply)
}
