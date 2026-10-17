//! World tension over a branch, for the World tension & threat widget: the world
//! tension the game showed, each country's contribution to it and the events
//! (threat sources) that raised or lowered it.
//!
//! Every value is a recorded field, kept exact (`engine::cworld_threat`,
//! `engine::cthreat_source`):
//!
//! - A threat source's contributions (`current_raw`, `final_raw`) are fixed
//!   point with [`cthreat_source::FIXED_POINT_SCALE`] units per percentage point
//!   of world tension: sum every source's current contribution,
//!   clamp the sum to [`cworld_threat::CLAMP_CEILING`] (100%) and store it
//!   divided by [`cworld_threat::SCALAR_DIVISOR`] as the world tension
//!   (`WorldTension.total_raw`). So world tension has 1,000 units per percentage
//!   point and contributions 100,000, and the sum of the contributions can pass
//!   100% or include negative contributions, such as those from a peace treaty.
//! - `ThreatCountryAggregates` is the engine's own sum of the current
//!   contributions per canonical country slot, with its array sized to the
//!   country census, so every country is listed and a zero is a recorded
//!   zero. These are the widget's layers. Each read checks it against the sources
//!   and the world tension against the clamped sum, and the reply says how many
//!   reads disagreed.
//! - A source's `reason` is the text the game built for it in the recording's
//!   language with colour codes, so its event name is the game's own
//!   ([`display_text`]).
//!   `final_raw` is the contribution it adds once its ramp ends, `date` the hour
//!   it was created. The vector has no ids and is compacted, so an event is
//!   identified by its own recorded country reference, second country, date and
//!   reason (and its place among sources equal in all four), never by the slot
//!   the reference resolves to: a reference first read unresolved and named at a
//!   later read stays one event, which takes the country once it is named.
//!
//! A country that no longer exists keeps its sources until they decay, so a
//! slot keeps the tag, and a raw link the slot, they were last recorded with, at
//! any hour recording `Countries` and `CountryLinks`, a read or not.
//!
//! A country's contributions are kept by its identity, its canonical slot, and
//! named by the tag the slot held at the read: one slot still contributing under a
//! tag that another slot now holds is a second country, never joined to it, and a
//! slot recorded under another tag is another entry.
//!
//! The three tables are read together every hour (daily, one slot, in older
//! recordings). A read is an hour whose frame timed a walk of them
//! ([`slot_read`]). Values start at a read, are carried over
//! the following recorded hours and end at a gap or at an hour whose tables are
//! incomplete, so an hour the recording lacks is never given a value.

use dashboard_base::{Source, catalogue::display_text};
use dashboard_base::{
    products::BranchParams, products::Product, products::country_history::CountryIdentityRun,
    products::country_measure::FixedRun,
};
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::{products::LoadContext, products::country_history::branch_histories};
#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::validate_campaign;
use dashboard_text::metadata::{link_slots, slot_read};
use engine::{cthreat_source, cworld_threat};
#[cfg(any(test, feature = "test-support"))]
use replay::{container::Replay, frame::FrameDecoder};
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::Ordering},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

pub struct WorldTension;

pub mod block;
mod first_view;
pub mod prepared;

/// Branches kept per source.
#[cfg(any(test, feature = "test-support"))]
pub const CACHED_BRANCHES: usize = 4;
/// World tension units per percentage point: contributions' units over the
/// divisor the engine stores the clamped sum with.
const TENSION_SCALE: i64 = cthreat_source::FIXED_POINT_SCALE / cworld_threat::SCALAR_DIVISOR;
const _: () = assert!(
    TENSION_SCALE * cworld_threat::SCALAR_DIVISOR == cthreat_source::FIXED_POINT_SCALE
        && cthreat_source::FIXED_POINT_SCALE <= u32::MAX as i64
);

/// One read of the world's tension, held over the following hours until the next.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionRead {
    /// The hour it was read, where its values start.
    pub first_hour: i32,
    /// The last hour the read's values hold.
    pub last_hour: i32,
    /// The read follows a gap or an hour without the tables.
    pub starts_segment: bool,
    /// World tension as the game showed it, in [`TensionReply::tension_scale`]
    /// units (`WorldTension.total_raw`).
    pub tension_raw: String,
    /// Every country's contribution summed before the game's cap, in
    /// [`TensionReply::contribution_scale`] units.
    pub sum_raw: String,
    /// The part of `sum_raw` from country slots no recorded country row names,
    /// in the same units; "0" when every contributing country is named.
    pub unnamed_raw: Option<String>,
    /// Fields needing earlier holder observations in a frame-only first view.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub preparing: Vec<String>,
}

/// One country's contribution: runs of reads where it was not zero. An hour of a
/// read without a run of the country is a recorded zero; an hour of no read is
/// not recorded.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionCountry {
    /// The tag the country held at these reads.
    pub tag: String,
    /// Its identity: the canonical country slot its contributions were recorded
    /// under. Two entries may share a tag, never a slot and a tag.
    pub slot: u32,
    /// In [`TensionReply::contribution_scale`] units. A run starts a segment where
    /// the hours before it were not recorded (a gap, or an hour without the
    /// tables); after a recorded zero it continues the line.
    pub runs: Vec<FixedRun>,
    /// Native holders of this cached tag/slot. Colours and names may only be
    /// sampled within these recorded bounds, including after the holder vanished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<CountryIdentityRun>>,
}

/// The contribution an event adds, as recorded from a read on.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionAdded {
    /// The read where the source first held this value.
    pub hour: i32,
    /// The source's `final_raw`, in [`TensionReply::contribution_scale`] units.
    pub added_raw: String,
}

/// A threat source: something a country did that changed world tension.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionEvent {
    /// The hour the game created it (`date`).
    pub hour: i32,
    /// The country that caused it; null when no recorded country row names it.
    pub tag: Option<String>,
    /// The game's own text for it, without formatting codes.
    pub reason: String,
    /// What it adds, from its first read on; a later entry when the game changed it.
    pub added: Vec<TensionAdded>,
    /// The first and last read listing it.
    pub first_read: i32,
    pub last_read: i32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionReply {
    pub branch_id: String,
    /// World tension units per percentage point.
    pub tension_scale: u32,
    /// Contribution units per percentage point.
    pub contribution_scale: u32,
    /// The sum of contributions world tension is capped at, in contribution units.
    pub ceiling_raw: String,
    /// In hour order.
    pub reads: Vec<TensionRead>,
    /// Every country that contributed in the branch, in tag then slot order.
    pub countries: Vec<TensionCountry>,
    /// In hour order, then first read.
    pub events: Vec<TensionEvent>,
    pub warnings: Vec<String>,
    /// Exact current amounts whose remembered contributor name is not prepared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub preparing_contributors: Vec<PreparingContributor>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreparingContributor {
    pub hour: i32,
    pub slot: u32,
    pub value_raw: String,
}

/// An event's identity: its recorded country and second country references, date,
/// reason, and its place among sources equal in all four within a read.
type EventKey = (u32, u32, i32, Vec<u8>, u32);
/// A source's country and second country references, date and reason.
type SourceIdentity<'a> = (u32, u32, i32, &'a [u8]);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannedTension {
    pub reads: Vec<TensionRead>,
    pub countries: Vec<TensionCountry>,
    pub events: Vec<TensionEvent>,
    /// Reads whose aggregates disagree with their sources' current contributions.
    pub aggregate_mismatches: usize,
    /// Reads whose world tension is not the game's cap of the aggregates' sum.
    pub tension_mismatches: usize,
    /// Reads whose source rows are incomplete, so their events may be missing.
    pub partial_sources: usize,
    unnamed_reads: usize,
    first_contributions: Vec<i32>,
    read_count: u64,
}

/// A country's runs while scanning: runs by read index, so a carried read extends
/// them without touching every country.
#[derive(Default, Serialize, Deserialize)]
struct Building {
    tag: String,
    slot: u32,
    first_hour: i32,
    /// (first read, last read, value, starts segment)
    runs: Vec<(usize, usize, i64, bool)>,
}

/// Authenticate the retained holder operands against the current link projection.
fn slot_tags(
    countries: &dashboard_text::metadata::CountryKeys,
    links: &HashMap<u32, u32>,
) -> HashMap<u32, String> {
    countries
        .iter()
        .filter(|&(id, slot, _tag)| links.get(&id) == Some(&slot))
        .map(|(_id, slot, tag)| (slot, String::from_utf8_lossy(tag).into_owned()))
        .collect()
}

/// Current source operands omit decay fields the reply never reads.
#[derive(Clone)]
struct SourceOperand {
    country: u32,
    second_country: u32,
    date: i32,
    reason: Vec<u8>,
    current_raw: i64,
    final_raw: i64,
}
fn source_operands(frame: &DecodedFrame) -> Option<(Vec<SourceOperand>, bool)> {
    let (rows, complete) = match &*frame.tables[TableId::ThreatSources.index()] {
        TableView::Complete(rows) => (rows.rows::<tables::ThreatSources>()?, true),
        TableView::Truncated { rows, .. } => (rows.rows::<tables::ThreatSources>()?, false),
        TableView::Unavailable { .. } => return None,
    };
    Some((
        rows.iter()
            .map(|row| SourceOperand {
                country: row.country,
                second_country: row.second_country,
                date: row.date,
                reason: row.reason.clone(),
                current_raw: row.current_raw,
                final_raw: row.final_raw,
            })
            .collect(),
        complete,
    ))
}
fn aggregate_operands(frame: &DecodedFrame) -> Option<Vec<(u32, i64)>> {
    Some(
        frame
            .complete::<tables::ThreatCountryAggregates>()
            .ok()?
            .iter()
            .map(|row| (row.country_slot, row.total_raw))
            .collect(),
    )
}

/// Scans one branch for its tension reads, contributions and events.
#[cfg(any(test, feature = "test-support"))]
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedTension> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::WorldTension,
        TableId::ThreatSources,
        TableId::ThreatCountryAggregates,
    ];
    let cancel = || {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "World tension request cancelled",
        )
    };
    let mut observer = Scan::default();
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
                    observer.open = false;
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        observer.open = false;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        observer.open = false;
                        break;
                    }
                    observer.observe(replay.format, decoded)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(observer.finish())
}

#[derive(Default, Serialize, Deserialize)]
pub struct Scan {
    result: ScannedTension,
    building: Vec<Building>,
    #[serde(
        serialize_with = "dashboard_core::map_pairs::ordered_pairs",
        deserialize_with = "dashboard_core::map_pairs::deserialize"
    )]
    by_country: HashMap<(u32, String), usize>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    known: HashMap<u32, String>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    known_links: HashMap<u32, u32>,
    #[serde(
        serialize_with = "dashboard_core::map_pairs::ordered_pairs",
        deserialize_with = "dashboard_core::map_pairs::deserialize"
    )]
    event_indices: HashMap<EventKey, usize>,
    last_hour: Option<i32>,
    open: bool,
    current: BTreeMap<usize, i64>,
    coverage: Vec<dashboard_base::products::country_groups::HourSpan>,
    batch: dashboard_base::prepared::series::Accumulator,
    active_sources: BTreeSet<usize>,
    last_source_read: Option<i32>,
    diagnostic_rate: (usize, usize, usize, usize),
    #[serde(skip)]
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    #[serde(skip)]
    link_inputs: Option<HashMap<u32, u32>>,
    #[serde(skip)]
    aggregate_inputs: Option<Vec<(u32, i64)>>,
    #[serde(skip)]
    source_inputs: Option<(Vec<SourceOperand>, bool)>,
    #[serde(skip)]
    source_sum: Option<Result<i64, String>>,
    #[serde(skip)]
    source_count: Option<usize>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
}
impl Scan {
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        format: replay::format::FormatVersion,
        decoded: &DecodedFrame,
    ) -> io::Result<()> {
        self.refresh_holders(decoded, true, true);
        self.refresh_inputs(decoded, true, true);
        self.observe_inputs(format, decoded, true, true, true)
    }
    fn refresh_holders(&mut self, frame: &DecodedFrame, countries: bool, links: bool) {
        if countries {
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
        if links {
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
    }
    fn refresh_inputs(&mut self, frame: &DecodedFrame, aggregates: bool, sources: bool) {
        if aggregates {
            self.aggregate_inputs = aggregate_operands(frame);
            #[cfg(test)]
            {
                self.work.row_visits += self.aggregate_inputs.as_ref().map_or(0, Vec::len);
            }
        }
        if sources {
            self.source_inputs = source_operands(frame);
            #[cfg(test)]
            {
                self.work.row_visits += self
                    .source_inputs
                    .as_ref()
                    .map_or(0, |(rows, _)| rows.len());
            }
            self.source_count = self
                .source_inputs
                .as_ref()
                .and_then(|(rows, complete)| complete.then_some(rows.len()));
            self.source_sum = self.source_inputs.as_ref().map(|(rows, _)| {
                rows.iter().try_fold(0_i64, |sum, row| {
                    sum.checked_add(row.current_raw).ok_or_else(|| {
                        "World tension source sum exceeds signed 64-bit range".to_owned()
                    })
                })
            });
        }
    }
    fn observe_inputs(
        &mut self,
        format: replay::format::FormatVersion,
        decoded: &DecodedFrame,
        identities_changed: bool,
        contributions_changed: bool,
        sources_changed: bool,
    ) -> io::Result<()> {
        let hour = decoded.hour;
        let continues = self.last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        self.last_hour = Some(hour);
        self.open &= continues;
        // Identities are remembered from every hour that records them, with or
        // without the tension tables: a country seen only then still names its
        // slot and links at a later read.
        let identity = self.country_keys.as_ref().zip(self.link_inputs.as_ref());
        if identities_changed && let Some((countries, links)) = identity {
            self.known.extend(slot_tags(countries, links));
            self.known_links
                .extend(links.iter().map(|(&raw, &slot)| (raw, slot)));
        }
        let tables = identity
            .map(|_| ())
            .zip(decoded.complete::<tables::WorldTension>().ok())
            .zip(self.aggregate_inputs.as_ref());
        let Some(((_, tension), aggregates)) = tables else {
            self.open = false;
            return Ok(());
        };
        if !slot_read(format, &decoded.metrics, TableId::WorldTension, hour) {
            if self.open
                && let Some(read) = self.result.reads.last_mut()
            {
                read.last_hour = hour;
            }
            return Ok(());
        }
        let Some(tension) = tension.first() else {
            self.open = false;
            return Ok(());
        };
        let index = self.result.reads.len();
        let starts_segment = !self.open;
        self.open = true;
        let mut sum = 0_i64;
        let mut unnamed = 0_i64;
        if !contributions_changed {
            let previous = self
                .result
                .reads
                .last()
                .expect("held contribution projection");
            sum = previous.sum_raw.parse().expect("native contribution sum");
            unnamed = previous
                .unnamed_raw
                .as_deref()
                .expect("prepared unnamed contribution")
                .parse()
                .expect("native unnamed sum");
        } else {
            self.current.clear();
            for &(slot, total) in aggregates.iter().filter(|(_, total)| *total != 0) {
                sum = sum.checked_add(total).ok_or_else(|| {
                    io::Error::other("World tension contribution sum exceeds signed 64-bit range")
                })?;
                let Some(tag) = self.known.get(&slot) else {
                    unnamed = unnamed.checked_add(total).ok_or_else(|| {
                        io::Error::other(
                            "World tension unnamed contribution sum exceeds signed 64-bit range",
                        )
                    })?;
                    continue;
                };
                let at = *self
                    .by_country
                    .entry((slot, tag.clone()))
                    .or_insert_with(|| {
                        self.building.push(Building {
                            tag: tag.clone(),
                            slot,
                            first_hour: hour,
                            runs: Vec::new(),
                        });
                        self.building.len() - 1
                    });
                let runs = &mut self.building[at].runs;
                self.current.insert(at, total);
                match runs.last_mut() {
                    Some(run) if !starts_segment && run.1 + 1 == index && run.2 == total => {
                        run.1 = index;
                    }
                    // One identity, so only a gap breaks its line.
                    _ => runs.push((index, index, total, starts_segment)),
                }
            }
        }
        self.result.unnamed_reads += usize::from(unnamed != 0);
        self.result.read_count += 1;
        self.result.reads.push(TensionRead {
            first_hour: hour,
            last_hour: hour,
            starts_segment,
            tension_raw: tension.total_raw.to_string(),
            sum_raw: sum.to_string(),
            unnamed_raw: Some(unnamed.to_string()),
            preparing: Vec::new(),
        });
        // World tension stores the clamped and scaled sum.
        let capped = sum.clamp(0, cworld_threat::CLAMP_CEILING) / cworld_threat::SCALAR_DIVISOR;
        if capped != tension.total_raw {
            self.result.tension_mismatches += 1;
        }
        let whole = self
            .source_count
            .is_some_and(|count| usize::try_from(tension.source_count).ok() == Some(count));
        if !whole {
            self.result.partial_sources += 1;
        } else if let Some(current) = &self.source_sum {
            let current = current
                .as_ref()
                .map_err(|reason| io::Error::other(reason.clone()))?;
            if *current != sum {
                self.result.aggregate_mismatches += 1;
            }
        }
        if !sources_changed {
            return Ok(());
        }
        self.active_sources.clear();
        // Sources equal in all four, counted in stored order.
        let mut seen: HashMap<SourceIdentity, u32> = HashMap::new();
        for row in self
            .source_inputs
            .as_ref()
            .map(|(rows, _)| rows.as_slice())
            .unwrap_or_default()
        {
            let slot = self.known_links.get(&row.country).copied();
            let place = seen
                .entry((
                    row.country,
                    row.second_country,
                    row.date,
                    row.reason.as_slice(),
                ))
                .or_default();
            let key = (
                row.country,
                row.second_country,
                row.date,
                row.reason.clone(),
                *place,
            );
            *place += 1;
            let added = row.final_raw.to_string();
            let tag = slot.and_then(|slot| self.known.get(&slot).cloned());
            match self.event_indices.get(&key) {
                Some(&at) => {
                    self.active_sources.insert(at);
                    let event = &mut self.result.events[at];
                    event.last_read = hour;
                    // A country first read unnamed takes its name once it is recorded.
                    if event.tag.is_none() {
                        event.tag = tag;
                    }
                    if event
                        .added
                        .last()
                        .is_none_or(|last| last.added_raw != added)
                    {
                        event.added.push(TensionAdded {
                            hour,
                            added_raw: added,
                        });
                    }
                }
                None => {
                    self.active_sources.insert(self.result.events.len());
                    self.event_indices.insert(key, self.result.events.len());
                    self.result.events.push(TensionEvent {
                        hour: row.date,
                        tag,
                        reason: display_text(&row.reason).map_err(|reason| {
                            io::Error::other(format!(
                                "The recorded threat reason at hour {} {reason}",
                                row.date
                            ))
                        })?,
                        added: vec![TensionAdded {
                            hour,
                            added_raw: added,
                        }],
                        first_read: hour,
                        last_read: hour,
                    });
                }
            }
        }
        Ok(())
    }
    pub fn unreadable_block(&mut self) {
        self.open = false;
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn prepare(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, format);
        self.prepare_changed(format, frame, &changes)
    }
    pub fn prepare_changed(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.batch.advance(frame.hour)?;
        self.refresh_inputs(
            frame,
            changes.rows_changed[TableId::ThreatCountryAggregates.index()],
            changes.rows_changed[TableId::ThreatSources.index()],
        );
        let identity_changed = changes.rows_changed[TableId::CountryLinks.index()]
            || changes.rows_changed[TableId::Countries.index()]
                && self.country_keys.as_ref().is_none_or(|keys| {
                    frame
                        .complete::<tables::Countries>()
                        .map_or(true, |rows| !keys.same(rows))
                });
        let same = !identity_changed
            && ![
                TableId::WorldTension,
                TableId::ThreatSources,
                TableId::ThreatCountryAggregates,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
        if same && self.continues_to(frame.hour) {
            self.hold(frame.hour, changes.slot_read[TableId::WorldTension.index()]);
            return Ok(());
        }
        self.seal_sources();
        self.refresh_holders(
            frame,
            identity_changed && changes.rows_changed[TableId::Countries.index()],
            changes.rows_changed[TableId::CountryLinks.index()],
        );
        let before = (
            self.result.unnamed_reads,
            self.result.partial_sources,
            self.result.aggregate_mismatches,
            self.result.tension_mismatches,
        );
        let new_segment =
            !self.open || self.last_hour.and_then(|hour| hour.checked_add(1)) != Some(frame.hour);
        self.prepare_frame(
            format,
            frame,
            identity_changed,
            identity_changed
                || new_segment
                || changes.rows_changed[TableId::ThreatCountryAggregates.index()],
            identity_changed || new_segment || changes.rows_changed[TableId::ThreatSources.index()],
        )?;
        self.diagnostic_rate = (
            self.result.unnamed_reads - before.0,
            self.result.partial_sources - before.1,
            self.result.aggregate_mismatches - before.2,
            self.result.tension_mismatches - before.3,
        );
        Ok(())
    }
    /// Whether `hour` follows the last hour observed, as an hour whose inputs
    /// are unchanged must for [`Self::advance_quiet`].
    pub(crate) fn continues_to(&self, hour: i32) -> bool {
        self.last_hour.and_then(|last| last.checked_add(1)) == Some(hour)
    }
    /// [`Self::prepare_changed`] at an hour that follows the last one
    /// observed ([`Self::continues_to`]) and changes none of the rows,
    /// country keys or links it reads: the same step, given only whether the
    /// tension tables were read there (`read`, their slot read).
    pub(crate) fn advance_quiet(&mut self, hour: i32, read: bool) -> io::Result<()> {
        self.batch.advance(hour)?;
        self.hold(hour, read);
        Ok(())
    }
    /// An hour whose inputs equal the hour before's: the held read extends to
    /// it, and a read there starts the held values again at the hour with the
    /// diagnostic counts of the read before.
    fn hold(&mut self, hour: i32, read: bool) {
        if !self.open {
            self.last_hour = Some(hour);
            return;
        }
        let held = self.result.reads.last_mut().expect("held tension read");
        if read {
            self.result.read_count += 1;
            self.result.unnamed_reads += self.diagnostic_rate.0;
            self.result.partial_sources += self.diagnostic_rate.1;
            self.result.aggregate_mismatches += self.diagnostic_rate.2;
            self.result.tension_mismatches += self.diagnostic_rate.3;
            held.first_hour = hour;
            held.starts_segment = false;
            self.batch.observe(3, hour, Some(i128::from(hour)), false);
            self.last_source_read = Some(hour);
        }
        held.last_hour = hour;
        self.last_hour = Some(hour);
        dashboard_base::products::country_groups::record_hour(&mut self.coverage, hour);
    }
    fn seal_sources(&mut self) {
        if let Some(hour) = self.last_source_read {
            for &index in &self.active_sources {
                self.result.events[index].last_read = hour;
            }
        }
    }
    fn prepare_frame(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
        identities_changed: bool,
        contributions_changed: bool,
        sources_changed: bool,
    ) -> io::Result<()> {
        let previous_current = if contributions_changed {
            self.current.clone()
        } else {
            BTreeMap::new()
        };
        let was_open = self.open;
        let follows = self.last_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        let previous_reads = self.result.read_count;
        self.observe_inputs(
            format,
            frame,
            identities_changed,
            contributions_changed,
            sources_changed,
        )?;
        if self.result.read_count != previous_reads {
            self.last_source_read = Some(frame.hour);
        }
        self.batch.through(frame.hour);
        if self.open
            && let Some(read) = self
                .result
                .reads
                .last()
                .filter(|read| read.last_hour == frame.hour)
        {
            dashboard_base::products::country_groups::record_hour(&mut self.coverage, frame.hour);
            let broken = read.starts_segment && read.first_hour == frame.hour;
            let values = [
                read.tension_raw.as_str(),
                read.sum_raw.as_str(),
                read.unnamed_raw
                    .as_deref()
                    .expect("prepared unnamed contribution"),
            ]
            .map(|raw| raw.parse::<i128>().expect("native tension integer"));
            let shares_changed =
                self.batch
                    .state(0)
                    .and_then(|state| state.last)
                    .is_none_or(|run| {
                        run.value != values[0] * i128::from(cworld_threat::SCALAR_DIVISOR)
                    });
            for (key, value) in values
                .into_iter()
                .enumerate()
                .map(|(key, value)| (key as u64, value))
                .chain([(3, i128::from(read.first_hour)), (4, 0)])
            {
                let value = if key == 0 {
                    value * i128::from(cworld_threat::SCALAR_DIVISOR)
                } else {
                    value
                };
                if broken
                    || !was_open
                    || self
                        .batch
                        .state(key)
                        .and_then(|state| state.last)
                        .is_none_or(|run| run.value != value)
                {
                    self.batch.observe(key, frame.hour, Some(value), broken);
                }
            }
            let indices: BTreeSet<_> = if !contributions_changed && !shares_changed && !broken {
                BTreeSet::new()
            } else if !was_open || !follows {
                (0..self.building.len()).collect()
            } else {
                self.current
                    .keys()
                    .chain(previous_current.keys())
                    .copied()
                    .collect()
            };
            #[cfg(test)]
            {
                self.work.history_visits += indices.len();
            }
            for index in indices {
                let value = self.current.get(&index).copied().unwrap_or(0);
                let share = contribution_share(i128::from(value), values[0], values[1])?;
                for (key, value) in [
                    (5 + index as u64 * 2, i128::from(value)),
                    (6 + index as u64 * 2, share),
                ] {
                    if broken
                        || !was_open
                        || self
                            .batch
                            .state(key)
                            .and_then(|state| state.last)
                            .is_none_or(|run| run.value != value)
                    {
                        self.batch.observe(key, frame.hour, Some(value), broken);
                    }
                }
            }
        } else if was_open {
            for key in 0..5 + self.building.len() as u64 * 2 {
                self.batch.observe(key, frame.hour, None, false);
            }
        }
        if contributions_changed {
            for &index in self.current.keys() {
                self.building[index].runs.clear();
            }
        }
        if self.result.reads.len() > 1 {
            self.result.reads = self.result.reads.last().cloned().into_iter().collect();
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
        self.seal_sources();
        self.batch
            .flush(writer, &format!("tension-series-{branch}"), 5, cancelled)?;
        dashboard_base::prepared::series::write_catalogue(
            writer,
            "world-tension",
            branch,
            &self
                .by_country
                .iter()
                .map(|((slot, tag), index)| {
                    (
                        dashboard_base::prepared::series::field_key(tag, &slot.to_string()),
                        *index as u64,
                    )
                })
                .collect(),
            self,
        )
    }
    fn finish(mut self) -> ScannedTension {
        let mut result = self.result;
        let reads = &result.reads;
        self.building
            .sort_by(|a, b| (&a.tag, a.slot).cmp(&(&b.tag, b.slot)));
        result.first_contributions = self
            .building
            .iter()
            .map(|country| country.first_hour)
            .collect();
        let countries: Vec<_> = self
            .building
            .into_iter()
            .map(|country| TensionCountry {
                tag: country.tag,
                slot: country.slot,
                identities: None,
                runs: country
                    .runs
                    .into_iter()
                    .map(|(first, last, value, starts_segment)| FixedRun {
                        first_hour: reads[first].first_hour,
                        last_hour: reads[last].last_hour,
                        value_raw: value.to_string(),
                        starts_segment,
                    })
                    .collect(),
            })
            .collect();
        result.countries = countries;
        result
            .events
            .sort_by_key(|event| (event.hour, event.first_read));
        result
    }
}

#[derive(Default)]
#[cfg(any(test, feature = "test-support"))]
struct Cache {
    entries: VecDeque<(u64, Arc<ScannedTension>)>,
}

fn contribution_share(value: i128, tension: i128, sum: i128) -> io::Result<i128> {
    if sum <= 0 {
        return Ok(0);
    }
    let product = value * tension;
    let scale = i128::from(cworld_threat::SCALAR_DIVISOR);
    (product / sum)
        .checked_mul(scale)
        .and_then(|whole| whole.checked_add((product % sum) * scale / sum))
        .ok_or_else(|| io::Error::other("World tension share exceeds signed 128-bit range"))
}

impl Product for WorldTension {
    const KEY: &'static str = "world-tension";
    // Display text comes from the recording only.
    const CONTENT_DEPENDENT: bool = false;
    type Params = BranchParams;
    type Reply = TensionReply;

    fn types() -> Vec<String> {
        vec![
            BranchParams::decl(),
            TensionRead::decl(),
            TensionCountry::decl(),
            PreparingContributor::decl(),
            TensionAdded::decl(),
            TensionEvent::decl(),
            TensionReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch();
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == branch)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(&source.replay(), branch, cancelled)?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != branch);
                cache.entries.push_back((branch, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_BRANCHES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        let history = branch_histories(source, branch, false, &[], cancelled)?;
        scanned.reply(params, &history)
    }
}
impl ScannedTension {
    fn reply(
        &self,
        params: &BranchParams,
        history: &dashboard_base::products::country_history::RecordedHistories,
    ) -> io::Result<TensionReply> {
        let mut warnings = Vec::new();
        if self.read_count == 0 {
            warnings.push("No world tension was recorded in this branch.".to_owned());
        }
        if self.partial_sources > 0 {
            warnings.push(format!(
                "Reads whose threat sources were not all recorded: {}. Events they would name may be missing.",
                self.partial_sources
            ));
        }
        if self.aggregate_mismatches > 0 {
            warnings.push(format!(
                "Reads whose country contributions do not add up to their threat sources: {}.",
                self.aggregate_mismatches
            ));
        }
        if self.tension_mismatches > 0 {
            warnings.push(format!(
                "Reads whose world tension is not the game's cap of the contributions' sum: {}.",
                self.tension_mismatches
            ));
        }
        let unnamed = self.unnamed_reads;
        if unnamed > 0 {
            warnings.push(format!(
                "Reads with a contribution from a country the recording never names: {unnamed}. It is counted in Other."
            ));
        }
        let mut countries = self.countries.clone();
        for country in &mut countries {
            country.identities = Some(history.countries.get(&country.tag).map_or_else(
                Vec::new,
                |holder| {
                    holder
                        .identities
                        .iter()
                        .filter(|identity| identity.slot == country.slot)
                        .cloned()
                        .collect()
                },
            ));
        }
        let unproved = countries
            .iter()
            .zip(&self.first_contributions)
            .filter(|(country, first)| {
                !country.identities.as_ref().is_some_and(|identities| {
                    identities.iter().any(|identity| {
                        identity.first_hour <= **first && **first <= identity.last_hour
                    })
                })
            })
            .count();
        if unproved > 0 {
            warnings.push(format!("{unproved} contributor tag/slot entries have no proven native holder at their first contribution read; their counts remain exact, while country looks stay unknown."));
        }
        let reply = TensionReply {
            branch_id: params.branch_id.clone(),
            tension_scale: TENSION_SCALE as u32,
            contribution_scale: cthreat_source::FIXED_POINT_SCALE as u32,
            ceiling_raw: cworld_threat::CLAMP_CEILING.to_string(),
            reads: self.reads.clone(),
            countries,
            events: self.events.clone(),
            warnings,
            preparing_contributors: Vec::new(),
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod tests;
