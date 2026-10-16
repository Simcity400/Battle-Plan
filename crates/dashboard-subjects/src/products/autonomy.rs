//! Subjects' autonomy over a branch, for the Autonomy widget: each subject's
//! level, overlord, autonomy score and the scores at which it moves down or up a
//! level, hour by hour.
//!
//! Every value is a recorded field, kept exact:
//!
//! - `Autonomy` is the subject's recorded status: `score_raw`, `lower_score`
//!   and `upper_score` are fixed5; `state` is the level's id, and
//!   `previous`/`next` are the levels below and above it, null at either end.
//!   The bounds use the level's and next level's `min_freedom_level` times
//!   the total score. A subject may pass its upper bound at the top level.
//! - `Subjects` names the overlord (overlord to subject relations), and its
//!   fixed5 `autonomy_ratio` is carried as recorded. Its role in the game is
//!   not established, so it is not interpreted.
//! - Levels are ordered by the content's autonomy states (`installed_content::autonomy`,
//!   by `min_freedom_level`) and named by their localisation. Without the game
//!   folders, or for a level the content does not define, the order falls back
//!   to the lowest move-down score recorded at that level, and the name is null.
//!
//! Both tables are hourly. A value holds for its hour only: runs join equal
//! consecutive recorded hours of one country identity and end at a gap or an
//! hour whose tables are incomplete.
//!
//! A row whose subject or overlord `CountryLinks` and `Countries` do not name
//! leaves its hour only partly known: the hour stays recorded, is listed in
//! `unidentified` and counted in a warning, so a country without a lane there is
//! never taken for one that was not a subject.

use dashboard_base::{
    Source,
    catalogue::{Catalogue, View, catalogue},
};
use dashboard_base::{
    products::LoadContext, products::Product, products::country_groups::HourSpan,
    products::validate_tag,
};
use dashboard_text::metadata::link_slots;
#[cfg(test)]
use dashboard_text::metadata::validate_campaign;
#[cfg(test)]
use replay::{container::Replay, frame::FrameDecoder};
use replay::{
    frame::DecodedFrame,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::{collections::VecDeque, sync::atomic::Ordering};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct Autonomy;
pub mod prepared;

pub(crate) const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Autonomy,
    TableId::Subjects,
];
const FIELDS: [&str; 5] = ["score", "lower", "upper", "ratio", "relative"];

/// Current native holders are independent of hourly country balances.
struct Holders {
    slots: HashMap<u32, u32>,
    countries: HashMap<u32, (u32, Vec<u8>)>,
    tags: BTreeMap<String, Option<(u32, u32)>>,
}
impl Holders {
    fn read(frame: &DecodedFrame) -> Option<Self> {
        let countries = dashboard_text::metadata::CountryKeys::read(
            frame.complete::<tables::Countries>().ok()?,
        );
        let slots = link_slots(frame.complete::<tables::CountryLinks>().ok()?);
        Some(Self::bind(&countries, slots))
    }
    fn bind(countries: &dashboard_text::metadata::CountryKeys, slots: HashMap<u32, u32>) -> Self {
        // Transient identity operands let the existing holder resolver authenticate
        // its exact duplicate/tag rules without revisiting the recorded table.
        let countries: Vec<_> = countries
            .iter()
            .map(|(id, canonical_slot, tag)| replay::rows::CountryRow {
                id,
                canonical_slot,
                tag: tag.to_vec(),
                ..Default::default()
            })
            .collect();
        let tags = dashboard_text::metadata::tagged_countries_indexed(&countries, &slots)
            .into_iter()
            .map(|(tag, holder)| {
                (
                    String::from_utf8_lossy(tag).into_owned(),
                    holder.found().map(|(row, slot)| (slot, row.id)),
                )
            })
            .collect();
        let countries = countries
            .iter()
            .filter_map(|row| {
                slots
                    .get(&row.id)
                    .copied()
                    .filter(|slot| *slot == row.canonical_slot)
                    .map(|slot| (slot, (row.id, row.tag.clone())))
            })
            .collect();
        Self {
            slots,
            countries,
            tags,
        }
    }
}
/// Only current relation operands are retained; holder changes rebind them.
struct Relations {
    raw: Vec<(u32, u32, i64)>,
    overlords: HashMap<u32, (u32, i64)>,
    shared: HashSet<u32>,
    unidentified: bool,
}
impl Relations {
    fn read(frame: &DecodedFrame, holders: &Holders) -> Option<Self> {
        let rows = frame.complete::<tables::Subjects>().ok()?;
        let raw = rows
            .iter()
            .map(|row| (row.overlord, row.subject, row.autonomy_ratio))
            .collect();
        Some(Self::bind(raw, holders))
    }
    fn bind(raw: Vec<(u32, u32, i64)>, holders: &Holders) -> Self {
        let mut overlords = HashMap::new();
        let mut shared = HashSet::new();
        let mut unidentified = false;
        for &(overlord, subject, ratio) in &raw {
            let (Some(overlord), Some(subject)) = (
                holders.slots.get(&overlord).copied(),
                holders.slots.get(&subject).copied(),
            ) else {
                unidentified = true;
                continue;
            };
            unidentified |= !holders.countries.contains_key(&overlord)
                || !holders.countries.contains_key(&subject);
            if overlords.insert(subject, (overlord, ratio)).is_some() {
                shared.insert(subject);
            }
        }
        Self {
            raw,
            overlords,
            shared,
            unidentified,
        }
    }
}
/// Current score operands omit fields neither the sampler nor episode step reads.
#[derive(Clone)]
struct AutonomyOperand {
    subject: u32,
    state: Vec<u8>,
    score: i64,
    lower_score: i64,
    upper_score: i64,
    lowest: bool,
    highest: bool,
}
impl AutonomyOperand {
    fn read(frame: &DecodedFrame) -> Option<Arc<BTreeMap<u32, Self>>> {
        Some(Arc::new(
            frame
                .complete::<tables::Autonomy>()
                .ok()?
                .iter()
                .map(|row| {
                    (
                        row.subject,
                        Self {
                            subject: row.subject,
                            state: row.state.clone(),
                            score: row.score,
                            lower_score: row.lower_score,
                            upper_score: row.upper_score,
                            lowest: row.previous.is_none(),
                            highest: row.next.is_none(),
                        },
                    )
                })
                .collect(),
        ))
    }
}
/// One native join supplies categorical observations and stateless numeric samples.
struct Inputs {
    autonomy: Vec<AutonomyOperand>,
    holders: Arc<Holders>,
    relations: Arc<Relations>,
}
impl Inputs {
    fn read(frame: &DecodedFrame) -> Option<Self> {
        let holders = Arc::new(Holders::read(frame)?);
        let relations = Arc::new(Relations::read(frame, &holders)?);
        Some(Self {
            autonomy: AutonomyOperand::read(frame)?.values().cloned().collect(),
            holders,
            relations,
        })
    }
    fn relation(&self, slot: u32) -> Option<&(u32, i64)> {
        self.relations
            .overlords
            .get(&slot)
            .filter(|_| !self.relations.shared.contains(&slot))
    }
}

/// Raw series operands need no catalogue: a subject first seen in a tail is usable.
#[allow(clippy::type_complexity)]
pub(crate) fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>> {
    let holders = Holders::read(frame).map(Arc::new);
    let inputs = holders.as_ref().and_then(|holders| {
        Some(Inputs {
            autonomy: AutonomyOperand::read(frame)?.values().cloned().collect(),
            relations: Arc::new(Relations::read(frame, holders)?),
            holders: Arc::clone(holders),
        })
    });
    sample_inputs(holders.as_deref(), inputs.as_ref(), requested)
}
#[allow(clippy::type_complexity)]
fn sample_inputs(
    holders: Option<&Holders>,
    inputs: Option<&Inputs>,
    requested: Option<&[(String, String)]>,
) -> io::Result<BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>> {
    if requested.is_some_and(|keys| {
        keys.iter()
            .any(|(_, field)| !FIELDS.contains(&field.as_str()))
    }) {
        return Err(dashboard_core::invalid("Unknown autonomy sample field"));
    }
    let wanted = requested.map(|keys| {
        keys.iter()
            .map(|(tag, field)| (tag.as_str(), field.as_str()))
            .collect::<HashSet<_>>()
    });
    let mut result = BTreeMap::new();
    if let Some(holders) = holders {
        if let Some(keys) = requested {
            for (tag, field) in keys {
                result.insert(
                    (tag.clone(), field.clone()),
                    (holders.tags.get(tag).copied().flatten(), None),
                );
            }
        } else {
            for (tag, identity) in &holders.tags {
                for field in FIELDS {
                    result.insert((tag.clone(), field.to_owned()), (*identity, None));
                }
            }
        }
    }
    if let Some(inputs) = inputs {
        for row in &inputs.autonomy {
            let Some((slot, country)) = inputs
                .holders
                .slots
                .get(&row.subject)
                .copied()
                .and_then(|slot| inputs.holders.countries.get(&slot).map(|row| (slot, row)))
            else {
                continue;
            };
            let tag = String::from_utf8_lossy(&country.1);
            let values = numeric_values(
                i128::from(row.score),
                i128::from(row.lower_score),
                i128::from(row.upper_score),
                inputs.relation(slot).map(|(_, ratio)| i128::from(*ratio)),
            );
            for (field, value) in FIELDS.into_iter().zip(values) {
                if wanted
                    .as_ref()
                    .is_none_or(|keys| keys.contains(&(tag.as_ref(), field)))
                {
                    result.insert(
                        (tag.to_string(), field.to_owned()),
                        (Some((slot, country.0)), value),
                    );
                }
            }
        }
    }
    if let Some(keys) = requested {
        for key in keys {
            result.entry(key.clone()).or_insert((None, None));
        }
    }
    Ok(result)
}

#[cfg(any(test, feature = "test-support"))]
pub const CACHED_BRANCHES: usize = 4;
/// Units per point of an autonomy score: fixed5, retaining five decimal places.
const SCORE_SCALE: u32 = 100_000;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutonomyParams {
    pub branch_id: String,
    /// Every subject these countries had in the branch.
    pub overlords: Vec<String>,
    /// These countries, whenever they were subjects. Both selections empty asks
    /// for the whole branch, so frontend country lineage can resolve tag changes.
    pub subjects: Vec<String>,
}

/// A subject level, in level order (least free first).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AutonomyLevel {
    /// The autonomy state's id, as recorded.
    pub id: String,
    /// Its localised name; null without the game folders or a localisation.
    pub name: Option<String>,
    /// Whether the content's autonomy states place it in level order (define it with
    /// a plain `min_freedom_level`).
    pub defined: bool,
}

/// A subject's recorded autonomy held over consecutive hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AutonomyRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Follows a gap, an hour without the tables or another country under the tag.
    pub starts_segment: bool,
    /// The overlord's tag; null when no relation names the subject this hour.
    pub overlord: Option<String>,
    /// Index into [`AutonomyReply::levels`].
    pub level: u32,
    /// In [`AutonomyReply::score_scale`] units.
    pub score_raw: String,
    /// The score below which it moves down a level.
    pub lower_raw: String,
    /// The score at which it moves up a level.
    pub upper_raw: String,
    /// No level below (`previous` is null) or above (`next` is null).
    pub lowest: bool,
    pub highest: bool,
    /// The relation's recorded `autonomy_ratio`, fixed5; null without a relation.
    pub ratio_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AutonomySubject {
    pub tag: String,
    pub runs: Vec<AutonomyRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AutonomyReply {
    pub branch_id: String,
    /// Units per point of a score.
    pub score_scale: u32,
    /// Every level recorded in the branch, in level order.
    pub levels: Vec<AutonomyLevel>,
    /// The requested subjects in the order they first became subjects.
    pub subjects: Vec<AutonomySubject>,
    /// The hours recorded with both tables complete, in order: outside them nothing
    /// is known, so a country without a lane there is not recorded, not a non-subject.
    pub recorded: Vec<HourSpan>,
    /// The recorded hours, in order, with a subject or overlord the recording does
    /// not name: a country without a lane there may be one of them.
    pub unidentified: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

/// One subject's recorded state at an hour; `level` indexes the scan's levels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Sample {
    overlord: Option<String>,
    level: usize,
    score: i64,
    lower: i64,
    upper: i64,
    lowest: bool,
    highest: bool,
    ratio: Option<i64>,
}
impl Sample {
    fn run(&self, first_hour: i32, last_hour: i32, starts_segment: bool) -> AutonomyRun {
        AutonomyRun {
            first_hour,
            last_hour,
            starts_segment,
            overlord: self.overlord.clone(),
            level: self.level as u32,
            score_raw: self.score.to_string(),
            lower_raw: self.lower.to_string(),
            upper_raw: self.upper.to_string(),
            lowest: self.lowest,
            highest: self.highest,
            ratio_raw: self.ratio.map(|ratio| ratio.to_string()),
        }
    }
}
// The open metadata band's start/break accompanies the already retained sample.
type LastAutonomy = Option<(i32, u32, Sample, (i32, bool))>;
fn numeric_values(score: i128, lower: i128, upper: i128, ratio: Option<i128>) -> [Option<i128>; 5] {
    [
        Some(score),
        Some(lower),
        Some(upper),
        ratio,
        (upper != lower).then(|| (score - lower) * 1_000_000_000 / (upper - lower)),
    ]
}

#[derive(Default, Serialize, Deserialize)]
pub struct ScannedAutonomy {
    /// Recorded level ids, in the order first seen.
    pub levels: Vec<String>,
    /// The lowest move-down score recorded at each level.
    pub lowest_lower: Vec<i64>,
    /// (tag, runs with scan level indices), in the order first seen.
    pub subjects: Vec<(String, Vec<AutonomyRun>)>,
    /// Hours where a subject had more than one overlord.
    pub shared_subjects: usize,
    /// The hours recorded with both tables complete, in order.
    pub recorded: Vec<HourSpan>,
    /// The recorded hours with a row whose subject or overlord is not named, in order.
    pub unidentified: Vec<HourSpan>,
    #[serde(with = "dashboard_core::map_pairs")]
    level_indices: HashMap<Vec<u8>, usize>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    by_tag: HashMap<String, usize>,
    last: Vec<LastAutonomy>,
    /// Branch-wide selection needs only which subjects each overlord ever held.
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    overlord_subjects: HashMap<String, BTreeSet<String>>,
    batch: dashboard_base::prepared::series::Accumulator,
    #[serde(skip)]
    changed: Vec<(usize, MetaEvent)>,
    prepared_hour: Option<i32>,
    active: BTreeSet<usize>,
    shared_rate: usize,
    #[serde(skip)]
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    #[serde(skip)]
    cached_holders: Option<Arc<Holders>>,
    #[serde(skip)]
    link_inputs: Option<HashMap<u32, u32>>,
    #[serde(skip)]
    cached_relations: Option<Arc<Relations>>,
    #[serde(skip)]
    cached_autonomy: Option<Arc<BTreeMap<u32, AutonomyOperand>>>,
    #[serde(skip)]
    samples_by_slot: BTreeMap<u32, BTreeSet<u32>>,
    #[serde(skip)]
    sample_unidentified: bool,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MetaEvent {
    previous_hour: Option<i32>,
    run: AutonomyRun,
}

/// Adds an hour to ascending spans, joining it to the last span when it follows it.
fn mark(spans: &mut Vec<HourSpan>, hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) == Some(hour) => span.last_hour = hour,
        Some(span) if span.last_hour >= hour => {}
        _ => spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        }),
    }
}

/// Scans one branch for every subject's autonomy.
impl ScannedAutonomy {
    fn observe(&mut self, decoded: &DecodedFrame) -> io::Result<()> {
        self.observe_from(decoded, None)
    }
    fn observe_from(&mut self, decoded: &DecodedFrame, held_before: Option<i32>) -> io::Result<()> {
        self.observe_inputs(decoded, Inputs::read(decoded), held_before, false)
    }
    fn observe_inputs(
        &mut self,
        decoded: &DecodedFrame,
        inputs: Option<Inputs>,
        held_before: Option<i32>,
        partial: bool,
    ) -> io::Result<()> {
        self.changed.clear();
        let hour = decoded.hour;
        // An hour without the tables is not recorded: every run ends before it.
        let Some(inputs) = inputs else {
            self.active.clear();
            return Ok(());
        };
        let previous_active = if partial {
            BTreeSet::new()
        } else {
            std::mem::take(&mut self.active)
        };
        #[cfg(test)]
        {
            self.work.history_visits += inputs.autonomy.len();
        }
        mark(&mut self.recorded, hour);
        self.shared_subjects += inputs.relations.shared.len();
        let mut unidentified = inputs.relations.unidentified || partial && self.sample_unidentified;
        for row in &inputs.autonomy {
            let Some((slot, country)) = inputs
                .holders
                .slots
                .get(&row.subject)
                .copied()
                .and_then(|slot| inputs.holders.countries.get(&slot).map(|row| (slot, row)))
            else {
                unidentified = true;
                continue;
            };
            let tag = String::from_utf8_lossy(&country.1).into_owned();
            let level = *self
                .level_indices
                .entry(row.state.clone())
                .or_insert_with(|| {
                    self.levels
                        .push(String::from_utf8_lossy(&row.state).into_owned());
                    self.lowest_lower.push(row.lower_score);
                    self.levels.len() - 1
                });
            let lowest = &mut self.lowest_lower[level];
            *lowest = (*lowest).min(row.lower_score);
            let relation = inputs.relation(slot);
            let sample = Sample {
                overlord: relation.and_then(|(overlord, _)| {
                    inputs
                        .holders
                        .countries
                        .get(overlord)
                        .map(|row| String::from_utf8_lossy(&row.1).into_owned())
                }),
                level,
                score: row.score,
                lower: row.lower_score,
                upper: row.upper_score,
                lowest: row.lowest,
                highest: row.highest,
                ratio: relation.map(|(_, ratio)| *ratio),
            };
            let at = *self.by_tag.entry(tag.clone()).or_insert_with(|| {
                self.subjects.push((tag.clone(), Vec::new()));
                self.last.push(None);
                self.subjects.len() - 1
            });
            let previously_active = if partial {
                self.active.contains(&at)
            } else {
                previous_active.contains(&at)
            };
            self.active.insert(at);
            if previously_active
                && let Some(hour) = held_before
                && let Some(previous) = &mut self.last[at]
            {
                previous.0 = hour;
            }
            if let Some(overlord) = &sample.overlord {
                self.overlord_subjects
                    .entry(overlord.clone())
                    .or_default()
                    .insert(tag.clone());
            }
            let previous = self.last[at].take();
            let continues = previous.as_ref().is_some_and(|(seen, seen_slot, _, _)| {
                seen.checked_add(1) == Some(hour) && *seen_slot == slot
            });
            let subject_runs = &mut self.subjects[at].1;
            let same = continues
                && previous
                    .as_ref()
                    .is_some_and(|(_, _, seen, _)| *seen == sample);
            if same && let Some(run) = subject_runs.last_mut() {
                run.last_hour = hour;
            } else {
                subject_runs.push(sample.run(hour, hour, !continues));
            }
            let changed = !continues
                || previous.as_ref().is_none_or(|(_, _, seen, _)| {
                    (
                        seen.overlord.as_ref(),
                        seen.level,
                        seen.lowest,
                        seen.highest,
                        seen.ratio.is_some(),
                    ) != (
                        sample.overlord.as_ref(),
                        sample.level,
                        sample.lowest,
                        sample.highest,
                        sample.ratio.is_some(),
                    )
                });
            let band = if changed {
                (hour, !continues)
            } else {
                previous.as_ref().expect("continuing autonomy band").3
            };
            if changed {
                self.changed.push((
                    at,
                    MetaEvent {
                        previous_hour: previous.as_ref().map(|seen| seen.0),
                        run: subject_runs.last().expect("observed autonomy run").clone(),
                    },
                ));
            }
            self.last[at] = Some((hour, slot, sample, band));
        }
        if unidentified {
            mark(&mut self.unidentified, hour);
        }
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn prepare(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, replay::format::FormatVersion::CURRENT);
        self.prepare_changed(frame, &changes)
    }
    pub fn prepare_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.batch.advance(frame.hour)?;
        let identity_changed = changes.rows_changed[TableId::CountryLinks.index()]
            || self.country_keys.as_ref().is_none_or(|keys| {
                changes.rows_changed[TableId::Countries.index()]
                    && frame
                        .complete::<tables::Countries>()
                        .map_or(true, |rows| !keys.same(rows))
            });
        let follows = self.prepared_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        if follows
            && !identity_changed
            && ![TableId::Autonomy, TableId::Subjects]
                .iter()
                .any(|table| changes.rows_changed[table.index()])
        {
            if self
                .recorded
                .last()
                .is_some_and(|span| span.last_hour == self.prepared_hour.unwrap())
            {
                mark(&mut self.recorded, frame.hour);
                if self
                    .unidentified
                    .last()
                    .is_some_and(|span| span.last_hour == self.prepared_hour.unwrap())
                {
                    mark(&mut self.unidentified, frame.hour);
                }
            }
            // The relation join's duplicate count is a subject-hour diagnostic.
            // Its exact rate is retained from the latest projection below.
            self.shared_subjects += self.shared_rate;
            self.prepared_hour = Some(frame.hour);
            self.batch.through(frame.hour);
            return Ok(());
        }
        if changes.rows_changed[TableId::Countries.index()] && identity_changed {
            self.country_keys = frame
                .complete::<tables::Countries>()
                .ok()
                .map(dashboard_text::metadata::CountryKeys::read);
            #[cfg(test)]
            {
                self.work.row_visits += frame
                    .complete::<tables::Countries>()
                    .map_or(0, |rows| rows.len());
            }
        }
        if changes.rows_changed[TableId::CountryLinks.index()] {
            self.link_inputs = frame
                .complete::<tables::CountryLinks>()
                .ok()
                .map(link_slots);
            #[cfg(test)]
            {
                self.work.row_visits += frame
                    .complete::<tables::CountryLinks>()
                    .map_or(0, |rows| rows.len());
            }
        }
        if identity_changed || self.cached_holders.is_none() {
            self.cached_holders = self
                .country_keys
                .as_ref()
                .zip(self.link_inputs.as_ref())
                .map(|(countries, links)| Arc::new(Holders::bind(countries, links.clone())));
        }
        let old_relations = self.cached_relations.clone();
        if changes.rows_changed[TableId::Subjects.index()] || self.cached_relations.is_none() {
            self.cached_relations = self
                .cached_holders
                .as_ref()
                .and_then(|holders| Relations::read(frame, holders))
                .map(Arc::new);
            #[cfg(test)]
            {
                self.work.row_visits += frame
                    .complete::<tables::Subjects>()
                    .map_or(0, |rows| rows.len());
            }
        } else if identity_changed {
            self.cached_relations = self
                .cached_holders
                .as_ref()
                .zip(self.cached_relations.as_ref())
                .map(|(holders, relations)| {
                    Arc::new(Relations::bind(relations.raw.clone(), holders))
                });
        }
        let native_changed =
            changes.rows_changed[TableId::Autonomy.index()] || self.cached_autonomy.is_none();
        if native_changed {
            self.cached_autonomy = AutonomyOperand::read(frame);
            #[cfg(test)]
            {
                self.work.row_visits += frame
                    .complete::<tables::Autonomy>()
                    .map_or(0, |rows| rows.len());
            }
        }
        if identity_changed || native_changed {
            self.samples_by_slot.clear();
            self.sample_unidentified = false;
            if let Some((holders, samples)) = self
                .cached_holders
                .as_ref()
                .zip(self.cached_autonomy.as_ref())
            {
                for row in samples.values() {
                    self.sample_unidentified |= holders
                        .slots
                        .get(&row.subject)
                        .is_none_or(|slot| !holders.countries.contains_key(slot));
                    if let Some(&slot) = holders.slots.get(&row.subject) {
                        self.samples_by_slot
                            .entry(slot)
                            .or_default()
                            .insert(row.subject);
                    }
                }
            }
        }
        let partial =
            follows && !identity_changed && !native_changed && self.cached_relations.is_some();
        let selected: Option<BTreeSet<u32>> = if partial {
            let slots: BTreeSet<_> = old_relations
                .as_ref()
                .into_iter()
                .flat_map(|rows| rows.raw.iter())
                .chain(
                    self.cached_relations
                        .as_ref()
                        .into_iter()
                        .flat_map(|rows| rows.raw.iter()),
                )
                .filter_map(|(_, subject, _)| {
                    self.cached_holders.as_ref()?.slots.get(subject).copied()
                })
                .collect();
            Some(
                slots
                    .iter()
                    .flat_map(|slot| {
                        self.samples_by_slot
                            .get(slot)
                            .into_iter()
                            .flatten()
                            .copied()
                    })
                    .collect(),
            )
        } else {
            None
        };
        let shared_before = self.shared_subjects;
        // Seal the old frontier before a full projection removes a holder.
        if !partial {
            self.seal_prepared();
        }
        self.prepare_frame(frame, selected.as_ref(), partial)?;
        self.shared_rate = self.shared_subjects - shared_before;
        self.prepared_hour = Some(frame.hour);
        Ok(())
    }
    fn seal_prepared(&mut self) {
        let Some(hour) = self.prepared_hour else {
            return;
        };
        for &index in &self.active {
            let Some((seen, _, _, _)) = &mut self.last[index] else {
                continue;
            };
            if *seen >= hour {
                continue;
            }
            *seen = hour;
        }
    }
    fn prepare_frame(
        &mut self,
        frame: &DecodedFrame,
        selected: Option<&BTreeSet<u32>>,
        partial: bool,
    ) -> io::Result<()> {
        let inputs = self
            .cached_holders
            .as_ref()
            .zip(self.cached_relations.as_ref())
            .zip(self.cached_autonomy.as_ref())
            .map(|((holders, relations), samples)| Inputs {
                autonomy: if let Some(selected) = selected {
                    selected
                        .iter()
                        .filter_map(|key| samples.get(key).cloned())
                        .collect()
                } else {
                    samples.values().cloned().collect()
                },
                holders: Arc::clone(holders),
                relations: Arc::clone(relations),
            });
        let requested: Vec<_> = inputs
            .as_ref()
            .into_iter()
            .flat_map(|inputs| {
                inputs.autonomy.iter().filter_map(move |row| {
                    inputs
                        .holders
                        .slots
                        .get(&row.subject)
                        .and_then(|slot| inputs.holders.countries.get(slot))
                        .map(|country| String::from_utf8_lossy(&country.1).into_owned())
                })
            })
            .flat_map(|tag| {
                FIELDS
                    .into_iter()
                    .map(move |field| (tag.clone(), field.to_owned()))
            })
            .collect();
        let samples = sample_inputs(
            self.cached_holders.as_deref(),
            inputs.as_ref(),
            Some(&requested),
        )?;
        let processed: BTreeSet<_> = requested
            .iter()
            .filter_map(|(tag, _)| self.by_tag.get(tag).copied())
            .collect();
        let previous_active = if partial {
            processed
                .iter()
                .copied()
                .filter(|index| self.active.contains(index))
                .collect()
        } else {
            self.active.clone()
        };
        let previous_values: BTreeMap<_, _> = previous_active
            .iter()
            .filter_map(|&index| {
                let sample = &self.last[index].as_ref()?.2;
                Some((
                    index,
                    numeric_values(
                        i128::from(sample.score),
                        i128::from(sample.lower),
                        i128::from(sample.upper),
                        sample.ratio.map(i128::from),
                    ),
                ))
            })
            .collect();
        self.observe_inputs(frame, inputs, self.prepared_hour, partial)?;
        self.batch.through(frame.hour);
        let changed: HashSet<_> = self.changed.iter().map(|(index, _)| *index).collect();
        let observed: BTreeSet<_> = if partial {
            requested
                .iter()
                .filter_map(|(tag, _)| self.by_tag.get(tag).copied())
                .collect()
        } else {
            self.active.clone()
        };
        for &index in &observed {
            let (tag, runs) = &mut self.subjects[index];
            for run in runs.drain(..) {
                for (field, label) in FIELDS.into_iter().enumerate() {
                    let key = index as u64 * 6 + field as u64;
                    let value = samples
                        .get(&(tag.clone(), label.to_owned()))
                        .and_then(|(_, value)| *value);
                    let boundary = field == 0 && changed.contains(&index);
                    if boundary
                        || run.starts_segment
                        || !previous_active.contains(&index)
                        || previous_values.get(&index).and_then(|values| values[field]) != value
                    {
                        self.batch.observe_boundary(
                            key,
                            frame.hour,
                            value,
                            run.starts_segment,
                            boundary,
                        );
                    }
                }
            }
        }
        for index in previous_active.difference(&self.active) {
            for field in 0..5 {
                self.batch
                    .observe(*index as u64 * 6 + field, frame.hour, None, false);
            }
        }
        for (index, event) in self.changed.drain(..) {
            self.batch.event(index as u64 * 6 + 5, frame.hour, &event)?;
            #[cfg(test)]
            {
                self.work.emitted += 1;
            }
        }
        Ok(())
    }
    pub fn buffered_bytes(&self) -> usize {
        self.batch.bytes()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.seal_prepared();
        self.batch
            .flush(writer, &format!("autonomy-series-{branch}"), 0, cancelled)?;
        dashboard_base::prepared::series::write_catalogue(
            writer,
            "autonomy",
            branch,
            &self
                .by_tag
                .iter()
                .map(|(tag, index)| (tag.clone(), *index as u64))
                .collect(),
            self,
        )?;
        Ok(())
    }
}

#[cfg(test)]
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedAutonomy> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Autonomy,
        TableId::Subjects,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Autonomy request cancelled");
    let mut result = ScannedAutonomy::default();
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
                    result.observe(decoded)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(result)
}

/// The scan's levels in level order, named: the content's order (`states`) where
/// it places them, then the rest by their lowest recorded move-down score. `name`
/// localises a level's id. Returns the levels and, for each scan level, its place
/// among them.
pub fn order_levels(
    scanned: &ScannedAutonomy,
    states: Option<&installed_content::autonomy::AutonomyStates>,
    name: impl Fn(&str) -> Option<String>,
) -> (Vec<AutonomyLevel>, Vec<u32>) {
    let order: HashMap<&str, usize> = states
        .map(|states| {
            states
                .ordered()
                .into_iter()
                .filter(|state| state.min_freedom_level.is_some())
                .enumerate()
                .map(|(place, state)| (state.id.as_str(), place))
                .collect()
        })
        .unwrap_or_default();
    let mut indices: Vec<usize> = (0..scanned.levels.len()).collect();
    indices.sort_by_key(|&index| {
        let id = scanned.levels[index].as_str();
        (
            !order.contains_key(id),
            order.get(id).copied().unwrap_or(0),
            scanned.lowest_lower[index],
            index,
        )
    });
    let mut places = vec![0_u32; scanned.levels.len()];
    let levels = indices
        .iter()
        .enumerate()
        .map(|(place, &index)| {
            places[index] = place as u32;
            let id = scanned.levels[index].clone();
            AutonomyLevel {
                defined: order.contains_key(id.as_str()),
                name: name(&id),
                id,
            }
        })
        .collect();
    (levels, places)
}

pub(crate) fn level_name(catalogue: &Catalogue, id: &str) -> Option<String> {
    catalogue.localised(id, View::Plain)
}

/// Authored levels only: offline preparation has no scores to rank unplaced levels.
pub fn content_levels(
    states: &installed_content::autonomy::AutonomyStates,
    catalogue: &Catalogue,
) -> Vec<AutonomyLevel> {
    states
        .ordered()
        .into_iter()
        .map(|state| AutonomyLevel {
            id: state.id.clone(),
            name: level_name(catalogue, &state.id),
            defined: state.min_freedom_level.is_some(),
        })
        .collect()
}

#[derive(Default)]
#[cfg(test)]
struct Cache {
    entries: VecDeque<(u64, Arc<ScannedAutonomy>)>,
}

/// The content's autonomy states and the notes reading them left.
type States = (
    installed_content::autonomy::AutonomyStates,
    Vec<installed_content::Diagnostic>,
);
/// The states read for a catalogue, read once per catalogue (held, so a new one is
/// never mistaken for it).
type StatesCache = Option<(Arc<Catalogue>, Arc<States>)>;

/// The content's autonomy states, read once per catalogue.
fn content_states(source: &Source, catalogue: &Arc<Catalogue>) -> io::Result<Arc<States>> {
    let cache = source.state::<StatesCache>();
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached, states)) = cache.as_ref()
        && Arc::ptr_eq(cached, catalogue)
    {
        return Ok(Arc::clone(states));
    }
    let states = Arc::new(installed_content::autonomy::read(&catalogue.content.layers)?);
    *cache = Some((Arc::clone(catalogue), Arc::clone(&states)));
    Ok(states)
}

impl Product for Autonomy {
    const KEY: &'static str = "autonomy";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AutonomyParams;
    type Reply = AutonomyReply;

    fn types() -> Vec<String> {
        vec![
            AutonomyParams::decl(),
            AutonomyLevel::decl(),
            AutonomyRun::decl(),
            AutonomySubject::decl(),
            AutonomyReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        params
            .overlords
            .iter()
            .chain(&params.subjects)
            .try_for_each(|tag| validate_tag(tag))
    }

    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch: u64 = params.branch_id.parse().expect("validated branch id");
        let state = source.state::<Cache>();
        let cached = state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .iter()
            .find(|(cached, _)| *cached == branch)
            .map(|(_, scanned)| Arc::clone(scanned));
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(source.replay_ref(), branch, cancelled)?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != branch);
                cache.entries.push_back((branch, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_BRANCHES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        scanned
            .reply(source, params, context)
            .map(|(reply, _)| reply)
    }
}

impl ScannedAutonomy {
    fn reply(
        &self,
        source: &Source,
        params: &AutonomyParams,
        context: &LoadContext,
    ) -> io::Result<(AutonomyReply, Vec<u32>)> {
        let mut warnings = Vec::new();
        // Without the game folders the lanes still draw, ordered and named by the recording.
        let content = match context.content.as_ref() {
            Some(options) => match catalogue(source, options)
                .and_then(|catalogue| Ok((content_states(source, &catalogue)?, catalogue)))
            {
                Ok((states, catalogue)) => {
                    warnings.extend(states.1.iter().map(|(_, message)| message.clone()));
                    Some((states.0.clone(), catalogue))
                }
                Err(error) => {
                    warnings.push(format!("Subject level names and order could not be read from the game's files: {error}"));
                    None
                }
            },
            None => {
                warnings.push("Choose the game folders in Settings to see subject level names; levels are ordered by their recorded move-down scores.".into());
                None
            }
        };
        let (levels, places) =
            order_levels(self, content.as_ref().map(|(states, _)| states), |id| {
                level_name(&content.as_ref()?.1, id)
            });
        if let Some((_, catalogue)) = &content {
            warnings.extend(catalogue.refusal_lines());
            let undefined: Vec<_> = levels
                .iter()
                .filter(|level| !level.defined)
                .map(|level| level.id.as_str())
                .collect();
            if !undefined.is_empty() {
                warnings.push(format!(
                    "Subject levels the game's autonomy states do not place: {}. They follow the others, by their recorded move-down scores.",
                    undefined.join(", ")
                ));
            }
        }
        if self.recorded.is_empty() {
            warnings.push(
                "Subjects' autonomy was not recorded in this branch (the Autonomy or Subjects table is missing at every hour).".into(),
            );
        }
        let unidentified: i64 = self
            .unidentified
            .iter()
            .map(|span| i64::from(span.last_hour) - i64::from(span.first_hour) + 1)
            .sum();
        if unidentified > 0 {
            warnings.push(format!(
                "Hours with a subject or overlord the recording does not name: {unidentified}. A country without a lane then may be one of them."
            ));
        }
        if self.shared_subjects > 0 {
            warnings.push(format!(
                "Hours where a subject had more than one recorded overlord: {}. No overlord is shown then.",
                self.shared_subjects
            ));
        }
        let wanted = selection(params, &self.overlord_subjects);
        let subjects = self
            .subjects
            .iter()
            .filter(|(tag, _)| wanted(tag))
            .map(|(tag, runs)| AutonomySubject {
                tag: tag.clone(),
                runs: runs
                    .iter()
                    .map(|run| AutonomyRun {
                        level: places[run.level as usize],
                        ..run.clone()
                    })
                    .collect(),
            })
            .collect();
        let reply = AutonomyReply {
            branch_id: params.branch_id.clone(),
            score_scale: SCORE_SCALE,
            levels,
            subjects,
            recorded: self.recorded.clone(),
            unidentified: self.unidentified.clone(),
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok((reply, places))
    }
}

fn selection<'a>(
    params: &'a AutonomyParams,
    held: &'a HashMap<String, BTreeSet<String>>,
) -> impl Fn(&str) -> bool + 'a {
    let mut subjects: HashSet<_> = params.subjects.iter().map(String::as_str).collect();
    for overlord in &params.overlords {
        if let Some(tags) = held.get(overlord) {
            subjects.extend(tags.iter().map(String::as_str));
        }
    }
    move |tag| (params.subjects.is_empty() && params.overlords.is_empty()) || subjects.contains(tag)
}

#[cfg(test)]
mod tests;
