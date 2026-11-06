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
//!
//! Preparation projects each hour's countries whose parts or naming may have changed
//! ([`facts`]), keeps each part as runs with its holder evidence on the family's series
//! publication with each country's part names ([`emit`]), and the series family
//! `stability-parts` answers from it ([`query`]).

use super::country_stability::{
    self, BreakdownLine, FormulaBreakdown, ModifierBreakdown, StabilityBreakdown,
};

use dashboard_base::{Source, products::country_measure::FixedRun};
use dashboard_base::{
    products::CountryParams,
    products::LoadContext,
    products::Product,
    products::country_groups::HourSpan,
    products::politics_content::{PoliticsContent, politics_content},
};
use dashboard_text::{identity::NamingCountry, metadata::validate_campaign};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::{frame::FrameDecoder, frame::SharedView, frame::TableView};
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
pub mod emit;
pub mod facts;
pub mod query;
pub mod served;

/// Runs kept per country before the scan refuses.
const MAX_RUNS: usize = 400_000;
/// Recorded hours whose character tables are kept to name a read's characters.
const NAMED_HOURS: usize = 24 * 8;
/// Countries whose parts are kept per source.
const CACHED_COUNTRIES: usize = 4;
/// The formula terms and recorded modifier names (`engine_stability::country_stability::KEY_NAMES`).
const LINES_OF: [(&str, &str); 2] = [
    ("stabilityFactor", "stability_factor"),
    ("warSupportFactor", "war_support_factor"),
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StabilityPart {
    /// Stable within a reply: `term:<term>`, `line:<kind>:<identity>:<id>:<secondary>`,
    /// `unpaired:<text>`, `unpaired-refused:<escaped label>`, `remainder` or `clamp`.
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
pub type Character = (
    Option<String>,
    String,
    Result<(String, Vec<u8>, i32, bool), String>,
);

/// One part's value at a read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Value {
    id: String,
    term: Option<String>,
    line: Option<BreakdownLine>,
    character: Option<Character>,
    raw: i64,
}

/// What an addend's part id is made of: the whole id, or a character line's id before the
/// character its read recorded completes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, replay_spool::Spooled)]
enum Name {
    Id(String),
    Character(String),
}

/// One addend of a measure's parts at a read, in tree order, its character not named yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, replay_spool::Spooled)]
pub struct Addend {
    name: Name,
    term: Option<String>,
    line: Option<BreakdownLine>,
    raw: i64,
}

impl Addend {
    /// The bytes it owns on the heap.
    fn heap(&self) -> usize {
        let name = match &self.name {
            Name::Id(id) | Name::Character(id) => id.capacity(),
        };
        name + self.term.as_ref().map_or(0, String::capacity)
            + self.line.as_ref().map_or(0, line_heap)
    }
}

fn line_heap(line: &BreakdownLine) -> usize {
    std::mem::size_of::<BreakdownLine>()
        + line.kind.capacity()
        + line.identity.capacity()
        + line.secondary_raw.capacity()
        + line.value_raw.capacity()
        + line.text.as_ref().map_or(0, String::capacity)
}

/// A measure's addends at one read, or None when its parts are not all known: `modifier` is
/// its lines' modifier, None when the read recorded no lines with it.
/// Refused labels identify separate parts by recorded bytes, not their display text.
fn addends(
    formula: &FormulaBreakdown,
    modifier: Option<&ModifierBreakdown>,
    lines_term: &str,
    refused_label: &dyn Fn(&BreakdownLine, Option<u32>) -> Option<String>,
) -> Option<Vec<Addend>> {
    let mut addends = Vec::new();
    for term in &formula.terms {
        let raw: i64 = term.value_raw.as_deref()?.parse().ok()?;
        if term.term == lines_term {
            // The modifier term is its lines, or nothing is known of its parts.
            let modifier = modifier?;
            for line in &modifier.lines {
                let base = || {
                    format!(
                        "line:{}:{}:{}:{}",
                        line.kind, line.identity, line.identity_id, line.secondary_raw
                    )
                };
                let name = match &*line.kind {
                    "unpaired" => Name::Id(match &line.text {
                        Some(text) => format!("unpaired:{text}"),
                        None => format!(
                            "unpaired-refused:{}",
                            refused_label(line, modifier.index)?
                        ),
                    }),
                    "advisor" | "countryLeader" => Name::Character(base()),
                    _ => Name::Id(base()),
                };
                addends.push(Addend {
                    name,
                    term: None,
                    line: Some(line.clone()),
                    raw: line.value_raw.parse().ok()?,
                });
            }
            if let Some(remainder) = &modifier.remainder_raw {
                addends.push(Addend {
                    name: Name::Id("remainder".into()),
                    term: None,
                    line: None,
                    raw: remainder.parse().ok()?,
                });
            }
            continue;
        }
        addends.push(Addend {
            name: Name::Id(format!("term:{}", term.term)),
            term: Some(term.term.clone()),
            line: None,
            raw,
        });
    }
    if let Some(clamp) = &formula.clamp_raw {
        addends.push(Addend {
            name: Name::Id("clamp".into()),
            term: None,
            line: None,
            raw: clamp.parse().ok()?,
        });
    }
    formula.value_raw.as_ref()?;
    Some(addends)
}

/// A measure's parts of its `addends`, each character line's character as `character` names
/// it at the read: a renamed character is its own part, named as it was.
pub(crate) fn assemble(
    addends: &[Addend],
    character: &dyn Fn(&BreakdownLine) -> Option<Character>,
) -> Vec<Value> {
    let mut values: Vec<Value> = Vec::new();
    for addend in addends.iter().filter(|addend| addend.raw != 0) {
        let (id, recorded) = match (&addend.name, &addend.line) {
            (Name::Character(base), Some(line)) => match character(line) {
                Some(found) => (
                    format!("{base}:{}:{}", found.0.as_deref().unwrap_or_default(), found.1),
                    Some(found),
                ),
                None => (base.clone(), None),
            },
            (Name::Id(id) | Name::Character(id), _) => (id.clone(), None),
        };
        match values.iter_mut().find(|value| value.id == id) {
            // Two lines of one source add up, as the game's total does.
            Some(value) => value.raw = value.raw.wrapping_add(addend.raw),
            None => values.push(Value {
                id,
                term: addend.term.clone(),
                line: addend.line.clone(),
                character: recorded,
                raw: addend.raw,
            }),
        }
    }
    values
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
    refused_label: &dyn Fn(&BreakdownLine, Option<u32>) -> Option<String>,
) -> Option<Vec<Value>> {
    let modifier = breakdown
        .modifiers
        .iter()
        .find(|modifier| modifier.key == lines_key)
        .filter(|_| breakdown.lines_available);
    let addends = addends(formula, modifier, lines_term, refused_label)?;
    Some(assemble(&addends, character))
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Part {
    term: Option<String>,
    line: Option<BreakdownLine>,
    character: Option<Character>,
    /// The line's country at its hour, which idea names need.
    country: NamingCountry,
    runs: Vec<FixedRun>,
}

impl Part {
    /// Takes `value`'s names, and a line's country; whether any changed.
    fn update(&mut self, value: &Value, country: &NamingCountry) -> bool {
        let mut changed = false;
        if self.term != value.term {
            self.term.clone_from(&value.term);
            changed = true;
        }
        if value.line.is_some() {
            if self.line != value.line {
                self.line.clone_from(&value.line);
                changed = true;
            }
            if self.character != value.character {
                self.character.clone_from(&value.character);
                changed = true;
            }
            if self.country != *country {
                self.country.clone_from(country);
                changed = true;
            }
        }
        changed
    }
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

#[derive(Default)]
struct Track {
    last: Option<((u32, u32), i32)>,
    current: Option<ReadParts>,
    unlined: BTreeSet<i32>,
    lined: BTreeSet<i32>,
    available: bool,
    naming: Option<NamingCountry>,
    refused: dashboard_text::catalogue::Refusals,
}
#[derive(Default)]
struct Scan {
    #[cfg(test)]
    calculated_reads: usize,
    tracks: BTreeMap<String, Track>,
    recent_names: VecDeque<(i32, SharedView, SharedView)>,
    hour: Option<i32>,
    active: BTreeSet<String>,
    native: country_stability::Native,
    lines: Option<BTreeMap<u32, Vec<replay_data::rows::CountryStabilityLineRow>>>,
    /// The last complete SliceReads read, held as the frame's own view.
    reads: Option<SharedView>,
    removed: Vec<String>,
    clock_names: BTreeSet<u32>,
    #[cfg(test)]
    naming_ticks: usize,
    appearances: BTreeMap<u32, replay_data::rows::CountryAppearanceRow>,
    /// Whether `appearances` is this scan's own read. A new scan knows no
    /// earlier appearance, so its first read renames every country.
    appearances_read: bool,
    appearances_complete: bool,
    line_dates: BTreeMap<u32, i32>,
}
fn appearance(
    row: &replay_data::rows::CountryAppearanceRow,
) -> replay_data::rows::CountryAppearanceRow {
    replay_data::rows::CountryAppearanceRow {
        country: row.country,
        name_tag: row.name_tag.clone(),
        cosmetic_tag: row.cosmetic_tag.clone(),
        original_tag: row.original_tag.clone(),
        original_tag_reference: row.original_tag_reference,
        ..Default::default()
    }
}

impl Scan {
    /// The naming projection of changed appearance rows. Other appearance
    /// fields do not rename a part; a new row still enters the projection.
    fn apply_appearances(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
        delta: Option<country_stability::Delta<'_>>,
        dirty: &mut BTreeSet<u32>,
    ) -> bool {
        let (true, Some((changed, deleted)), Ok(rows)) = (
            self.appearances_complete,
            delta,
            decoded.complete::<tables::CountryAppearance>(),
        ) else {
            return false;
        };
        static FIELDS: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
            dashboard_base::prepared::frame_changes::field_mask(
                TableId::CountryAppearance,
                &[
                    "name_tag",
                    "cosmetic_tag",
                    "original_tag",
                    "original_tag_reference",
                ],
            )
        });
        let found = changed
            .iter()
            .filter_map(|row| {
                (row.mask & *FIELDS != 0 || !self.appearances.contains_key(&(row.key as u32)))
                    .then_some(row.key)
            })
            .map(|key| {
                rows.binary_search_by_key(&(key as u32), |row| row.country)
                    .ok()
                    .map(|at| &rows[at])
            })
            .collect::<Option<Vec<_>>>();
        let Some(found) = found else {
            return false;
        };
        #[cfg(test)]
        {
            self.native.work.row_visits += found.len();
        }
        for row in found {
            let next = appearance(row);
            if self.appearances.get(&row.country) != Some(&next) {
                self.appearances.insert(row.country, next);
                dirty.insert(row.country);
            }
        }
        for &key in deleted {
            if self.appearances.remove(&(key as u32)).is_some() {
                dirty.insert(key as u32);
            }
        }
        true
    }
    /// This hour's stability lines from the hour's recorded changes to them
    /// (`delta`), when the hour before's lines are held: only the countries
    /// those rows name are grouped again from this hour's rows, and those
    /// whose lines differ go to `dirty`, as grouping and comparing every
    /// country's lines finds. False, with nothing changed, when the whole
    /// table is needed.
    fn apply_lines(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
        delta: Option<country_stability::Delta<'_>>,
        dirty: &mut BTreeSet<u32>,
    ) -> bool {
        let (Some((changed, deleted)), Some(held), Ok(rows)) = (
            delta,
            self.lines.as_mut(),
            decoded.complete::<tables::CountryStabilityLines>(),
        ) else {
            return false;
        };
        // A line's key leads with its country.
        let countries: BTreeSet<u32> = changed
            .iter()
            .map(|row| row.key)
            .chain(deleted.iter().copied())
            .map(|key| (key >> 64) as u32)
            .collect();
        for country in countries {
            let first = rows.partition_point(|row| row.country < country);
            let last = rows.partition_point(|row| row.country <= country);
            let next = &rows[first..last];
            #[cfg(test)]
            {
                self.native.work.row_visits += next.len();
            }
            if held.get(&country).map_or(&[][..], Vec::as_slice) != next {
                dirty.insert(country);
                if next.is_empty() {
                    held.remove(&country);
                } else {
                    held.insert(country, next.to_vec());
                }
            }
        }
        true
    }

    fn step(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
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
        let cpu = ::replay_frame::cpu::scope_phase("stability:observe_apply");
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
        // The hour's recorded changes of a table, when it continues the hour
        // before: what is held from that hour then takes only its changed rows.
        let delta = |table: TableId| changes.and_then(|changes| changes.rows(table));
        if changed(TableId::SliceReads) {
            let rows = decoded.complete::<tables::SliceReads>().ok();
            let applied = self.reads.is_some()
                && match (delta(TableId::SliceReads), rows) {
                    (Some(delta), Some(rows)) => {
                        #[cfg(test)]
                        {
                            self.native.work.row_visits += delta.0.len();
                        }
                        country_stability::apply_reads(
                            &mut self.line_dates,
                            rows,
                            TableId::CountryStabilityLines,
                            delta,
                            &mut dirty,
                        )
                        .is_some()
                    }
                    _ => false,
                };
            if !applied {
                let next: BTreeMap<_, _> = rows
                    .into_iter()
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
            self.reads = rows
                .is_some()
                .then(|| decoded.tables[TableId::SliceReads.index()].clone());
        }
        if changed(TableId::CountryStabilityLines)
            && !self.apply_lines(decoded, delta(TableId::CountryStabilityLines), &mut dirty)
        {
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
        if changed(TableId::CountryAppearance)
            && !self.apply_appearances(
                decoded,
                delta(TableId::CountryAppearance),
                &mut naming_changed,
            )
        {
            let rows = decoded.complete::<tables::CountryAppearance>().ok();
            #[cfg(test)]
            {
                self.native.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next: BTreeMap<_, _> = rows
                .into_iter()
                .flatten()
                .map(|row| (row.country, appearance(row)))
                .collect();
            if self.appearances_read {
                naming_changed.extend(
                    self.appearances
                        .keys()
                        .chain(next.keys())
                        .copied()
                        .filter(|id| self.appearances.get(id) != next.get(id)),
                );
            } else {
                // Without an earlier read, a row gone since the saved hour is
                // unknown: every country's naming is read again.
                naming_changed.extend(self.native.roster.keys().copied());
                self.appearances_read = true;
            }
            self.appearances = next;
            self.appearances_complete = rows.is_some();
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
        let Some(reads) = self.reads.as_deref().and_then(|view| match view {
            TableView::Complete(rows) => rows.rows::<tables::SliceReads>(),
            _ => None,
        }) else {
            return Ok(());
        };
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope_phase("stability:parts_projection_series");
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
                replay_data::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)
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
                replay_data::sliced::read_hour(reads, TableId::CountryStabilityLines, row.country);
            if decoded.complete::<tables::CountryStabilityLines>().is_err()
                || lines_read.is_some_and(|lines_read| lines_read != read)
            {
                track.unlined.insert(read);
                track.available = false;
                // Naming is not read at this hour, so a change to it here
                // would go unseen: the next lined hour reads it again.
                track.naming = None;
                self.clock_names.remove(&id);
                self.removed.push(tag.to_owned());
                continue;
            }
            let fresh = !track
                .current
                .as_ref()
                .is_some_and(|(seen, at, _)| *seen == identity && *at == read);
            if fresh {
                let _cpu = ::replay_frame::cpu::scope_phase("stability:parts_breakdown");
                #[cfg(test)]
                {
                    self.calculated_reads += 1;
                }
                // Refused labels belong in country warnings; `unlined` counts unrecorded lines.
                let mut notes = Vec::new();
                let breakdown = country_stability::breakdown_noting(
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
                    &mut track.refused,
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
                // Escape recorded bytes so distinct refused labels remain distinct sources.
                let recorded_lines = decoded.complete::<tables::CountryStabilityLines>().ok();
                let refused_label = |line: &BreakdownLine, key: Option<u32>| {
                    let lines = recorded_lines?;
                    let at = lines
                        .binary_search_by_key(&(row.country, line.line, key?), |recorded| {
                            (recorded.country, recorded.line, recorded.key)
                        })
                        .ok()?;
                    Some(lines[at].text.escape_ascii().to_string())
                };
                let values = [
                    (&breakdown.stability, LINES_OF[0]),
                    (&breakdown.war_support, LINES_OF[1]),
                ]
                .map(|(formula, (term, key))| {
                    parts_of(formula, &breakdown, key, term, &character, &refused_label)
                });
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
        let track = self.tracks.get(tag);
        let missing = track.map_or(0, |track| {
            track.unlined.difference(&track.lined).count()
        });
        let mut warnings = Vec::new();
        if missing != 0 {
            warnings.push(format!(
                "{tag}: {missing} reads recorded no modifier lines with their inputs; their parts are not known."
            ));
        }
        if let Some(track) = track {
            warnings.extend(track.refused.lines("stability tree labels are"));
        }
        warnings
    }
}

/// Names each line part from content, a character as it was recorded for that
/// part, as the hover breakdown names it.
fn named(measure: &Measure, content: Option<&PoliticsContent>, tag: &str) -> MeasureParts {
    named_parts(&measure.parts, &measure.observed, content, tag)
}

/// [`named`] of `parts` by id, known over `observed`.
fn named_parts(
    parts_by_id: &BTreeMap<String, Part>,
    observed: &[HourSpan],
    content: Option<&PoliticsContent>,
    tag: &str,
) -> MeasureParts {
    let mut parts = Vec::with_capacity(parts_by_id.len());
    for (id, part) in parts_by_id {
        let mut line = part.line.clone();
        if let (Some(line), Some(content)) = (line.as_mut(), content) {
            let character = part.character.as_ref().map(|(script, saved, template)| {
                country_stability::RecordedCharacter {
                    script: script.as_deref(),
                    saved,
                    template: template.clone().map(|(tag, file, line, generated)| {
                        installed_content_core::characters::BuiltTemplate {
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
        observed: observed.to_vec(),
    }
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<Scanned>)>,
}

impl Product for StabilityParts {
    const KEY: &'static str = "stability-parts";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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
            engine_stability::country_stability::KEY_NAMES.contains(&key.as_bytes()),
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
mod columnar_tests;
#[cfg(test)]
mod tests;
