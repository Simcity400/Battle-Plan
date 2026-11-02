//! The row recording's tension scan: the parity oracle of the family's columnar preparation
//! ([`super::facts`], [`super::emit`]), read from row frames hour by hour.

use super::{ScannedTension, TensionAdded, TensionEvent, TensionRead};
use dashboard_base::{catalogue::display_text, products::country_measure::FixedRun};
use dashboard_text::metadata::{link_slots, slot_read, validate_campaign};
use engine_stability::cworld_threat;
use replay::container::Replay;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::{DecodedFrame, FrameDecoder, TableView};
use std::{
    collections::{HashMap, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Branches kept per source.
pub const CACHED_BRANCHES: usize = 4;

/// An event's identity: its recorded country and second country references, date, reason, and
/// its place among sources equal in all four within a read.
type EventKey = (u32, u32, i32, Vec<u8>, u32);
/// A source's country and second country references, date and reason.
type SourceIdentity<'a> = (u32, u32, i32, &'a [u8]);

/// A country's runs while scanning: runs by read index.
#[derive(Default)]
struct Building {
    tag: String,
    slot: u32,
    first_hour: i32,
    /// (first read, last read, value, starts segment)
    runs: Vec<(usize, usize, i64, bool)>,
}

/// The holder operands the current link projection authenticates.
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
        TableView::Unavailable { .. } | TableView::NotRecorded { .. } => return None,
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

/// The tension scan of row frames observed in hour order.
#[derive(Default)]
pub struct Scan {
    pub(in crate::products::world_tension) result: ScannedTension,
    building: Vec<Building>,
    by_country: HashMap<(u32, String), usize>,
    known: HashMap<u32, String>,
    known_links: HashMap<u32, u32>,
    event_indices: HashMap<EventKey, usize>,
    last_hour: Option<i32>,
    open: bool,
}

impl Scan {
    /// Observes the next row frame.
    pub fn observe(
        &mut self,
        format: replay_data::format::FormatVersion,
        decoded: &DecodedFrame,
    ) -> io::Result<()> {
        let hour = decoded.hour;
        let continues = self.last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        self.last_hour = Some(hour);
        self.open &= continues;
        let country_keys = decoded
            .complete::<tables::Countries>()
            .ok()
            .map(dashboard_text::metadata::CountryKeys::read);
        let links = decoded
            .complete::<tables::CountryLinks>()
            .ok()
            .map(link_slots);
        // Identities are remembered from every hour that records them, with or without the
        // tension tables.
        let identity = country_keys.as_ref().zip(links.as_ref());
        if let Some((countries, links)) = identity {
            self.known.extend(slot_tags(countries, links));
            self.known_links
                .extend(links.iter().map(|(&raw, &slot)| (raw, slot)));
        }
        let aggregates = aggregate_operands(decoded);
        let tables = identity
            .map(|_| ())
            .zip(decoded.complete::<tables::WorldTension>().ok())
            .zip(aggregates.as_ref());
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
            match runs.last_mut() {
                Some(run) if !starts_segment && run.1 + 1 == index && run.2 == total => {
                    run.1 = index;
                }
                // One identity, so only a gap breaks its line.
                _ => runs.push((index, index, total, starts_segment)),
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
        let sources = source_operands(decoded);
        let whole = sources.as_ref().is_some_and(|(rows, complete)| {
            *complete && usize::try_from(tension.source_count).ok() == Some(rows.len())
        });
        if !whole {
            self.result.partial_sources += 1;
        } else if let Some((rows, _)) = &sources {
            let current = rows.iter().try_fold(0_i64, |sum, row| {
                sum.checked_add(row.current_raw).ok_or_else(|| {
                    io::Error::other("World tension source sum exceeds signed 64-bit range")
                })
            })?;
            if current != sum {
                self.result.aggregate_mismatches += 1;
            }
        }
        // Sources equal in all four, counted in stored order.
        let mut seen: HashMap<SourceIdentity, u32> = HashMap::new();
        for row in sources
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

    /// A block the scan could not read: its hours end the held read.
    pub fn unreadable_block(&mut self) {
        self.open = false;
    }

    /// What the scan read: each country's runs over the reads, in tag then slot order, and
    /// the events in hour then first-read order.
    pub fn finish(mut self) -> ScannedTension {
        let mut result = self.result;
        let reads = &result.reads;
        self.building
            .sort_by(|a, b| (&a.tag, a.slot).cmp(&(&b.tag, b.slot)));
        result.first_contributions = self
            .building
            .iter()
            .map(|country| country.first_hour)
            .collect();
        result.countries = self
            .building
            .into_iter()
            .map(|country| super::TensionCountry {
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
        result
            .events
            .sort_by_key(|event| (event.hour, event.first_read));
        result
    }
}

/// Each branch's scan, kept per source.
#[derive(Default)]
pub(super) struct Cache {
    pub(super) entries: VecDeque<(u64, Arc<ScannedTension>)>,
}
