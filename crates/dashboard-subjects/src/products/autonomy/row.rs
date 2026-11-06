//! The hourly row scan of subjects' autonomy, kept as the columnar family's parity oracle: every
//! decoded frame observed whole, each subject's runs joined over equal consecutive hours.

use super::{AutonomyParams, AutonomyReply, AutonomyRun, ScannedAutonomy, mark};
use dashboard_base::{Source, products::LoadContext};
use dashboard_text::metadata::{link_slots, validate_campaign};
use replay::container::Replay;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, FrameDecoder};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Every table the scan reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Autonomy,
    TableId::Subjects,
];

/// The links' canonical slots by raw id, and each resolved country's id and tag by slot.
struct Holders {
    slots: HashMap<u32, u32>,
    countries: HashMap<u32, (u32, Vec<u8>)>,
}

impl Holders {
    fn read(frame: &DecodedFrame) -> Option<Self> {
        let countries = frame.complete::<tables::Countries>().ok()?;
        let slots = link_slots(frame.complete::<tables::CountryLinks>().ok()?);
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
        Some(Self { slots, countries })
    }
}

/// The relations holding at the frame's hour: each subject slot's overlord and ratio, the
/// subjects more than one holds, and whether one names a country the recording does not.
struct Relations {
    overlords: HashMap<u32, (u32, i64)>,
    shared: HashSet<u32>,
    unidentified: bool,
}

impl Relations {
    fn read(frame: &DecodedFrame, holders: &Holders) -> Option<Self> {
        let rows = frame.complete::<tables::Subjects>().ok()?;
        let mut overlords = HashMap::new();
        let mut shared = HashSet::new();
        let mut unidentified = false;
        for row in rows {
            if !crate::products::subjects::holds(row.cancel, row.start_hour, row.end_hour, frame.hour)
            {
                continue;
            }
            let (Some(overlord), Some(subject)) = (
                holders.slots.get(&row.overlord).copied(),
                holders.slots.get(&row.subject).copied(),
            ) else {
                unidentified = true;
                continue;
            };
            unidentified |= !holders.countries.contains_key(&overlord)
                || !holders.countries.contains_key(&subject);
            if overlords
                .insert(subject, (overlord, row.autonomy_ratio))
                .is_some()
            {
                shared.insert(subject);
            }
        }
        Some(Self {
            overlords,
            shared,
            unidentified,
        })
    }

    fn relation(&self, slot: u32) -> Option<&(u32, i64)> {
        self.overlords
            .get(&slot)
            .filter(|_| !self.shared.contains(&slot))
    }
}

/// One subject's recorded state at an hour; `level` indexes the scan's levels.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// The scan of one branch, frame by frame.
#[derive(Default)]
pub struct Observer {
    pub scanned: ScannedAutonomy,
    level_indices: HashMap<Vec<u8>, usize>,
    by_tag: HashMap<String, usize>,
    /// Each subject's last observed hour, holder and sample.
    last: Vec<Option<(i32, (u32, u32), Sample)>>,
}

impl Observer {
    /// Observes one decoded frame: an hour without every table is not recorded.
    pub fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let hour = frame.hour;
        let Some(holders) = Holders::read(frame) else {
            return Ok(());
        };
        let Some(relations) = Relations::read(frame, &holders) else {
            return Ok(());
        };
        let Ok(rows) = frame.complete::<tables::Autonomy>() else {
            return Ok(());
        };
        let mut rows: Vec<_> = rows.iter().collect();
        rows.sort_by_key(|row| row.subject);
        let scanned = &mut self.scanned;
        mark(&mut scanned.recorded, hour);
        scanned.shared_subjects += relations.shared.len();
        let mut unidentified = relations.unidentified;
        for row in rows {
            let Some((slot, country)) = holders
                .slots
                .get(&row.subject)
                .copied()
                .and_then(|slot| holders.countries.get(&slot).map(|country| (slot, country)))
            else {
                unidentified = true;
                continue;
            };
            let tag = String::from_utf8_lossy(&country.1).into_owned();
            let level = *self.level_indices.entry(row.state.clone()).or_insert_with(|| {
                scanned
                    .levels
                    .push(String::from_utf8_lossy(&row.state).into_owned());
                scanned.lowest_lower.push(row.lower_score);
                scanned.levels.len() - 1
            });
            let lowest = &mut scanned.lowest_lower[level];
            *lowest = (*lowest).min(row.lower_score);
            let relation = relations.relation(slot);
            let sample = Sample {
                overlord: relation.and_then(|(overlord, _)| {
                    holders
                        .countries
                        .get(overlord)
                        .map(|country| String::from_utf8_lossy(&country.1).into_owned())
                }),
                level,
                score: row.score,
                lower: row.lower_score,
                upper: row.upper_score,
                lowest: row.previous.is_none(),
                highest: row.next.is_none(),
                ratio: relation.map(|(_, ratio)| *ratio),
            };
            let at = *self.by_tag.entry(tag.clone()).or_insert_with(|| {
                scanned.subjects.push((tag.clone(), Vec::new()));
                self.last.push(None);
                scanned.subjects.len() - 1
            });
            if let Some(overlord) = &sample.overlord {
                scanned
                    .overlord_subjects
                    .entry(overlord.clone())
                    .or_default()
                    .insert(tag.clone());
            }
            let previous = self.last[at].take();
            // Another native country holding the tag's slot is another history.
            let holder = (slot, country.0);
            let continues = previous.as_ref().is_some_and(|(seen, seen_holder, _)| {
                seen.checked_add(1) == Some(hour) && *seen_holder == holder
            });
            let same = continues
                && previous
                    .as_ref()
                    .is_some_and(|(_, _, seen)| *seen == sample);
            let runs = &mut scanned.subjects[at].1;
            match runs.last_mut() {
                Some(run) if same => run.last_hour = hour,
                _ => runs.push(sample.run(hour, hour, !continues)),
            }
            self.last[at] = Some((hour, holder, sample));
        }
        if unidentified {
            mark(&mut scanned.unidentified, hour);
        }
        Ok(())
    }
}

/// Scans one branch for every subject's autonomy.
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedAutonomy> {
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Autonomy request cancelled");
    let mut result = Observer::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        TABLES,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(TABLES);
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
    Ok(result.scanned)
}

/// Each branch's scan, kept for this source revision.
#[derive(Default)]
struct Cache {
    entries: BTreeMap<u64, Arc<ScannedAutonomy>>,
}

/// The row reply: the branch scanned once, kept for the next requests.
pub fn load(
    source: &Source,
    params: &AutonomyParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<AutonomyReply> {
    let branch: u64 = params.branch_id.parse().expect("validated branch id");
    let state = source.state::<Cache>();
    let cached = state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .entries
        .get(&branch)
        .map(Arc::clone);
    let scanned = match cached {
        Some(scanned) => scanned,
        None => {
            let scanned = Arc::new(scan(source.replay_ref(), branch, cancelled)?);
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            Arc::clone(cache.entries.entry(branch).or_insert(scanned))
        }
    };
    scanned
        .reply(source, params, context)
        .map(|(reply, _)| reply)
}
