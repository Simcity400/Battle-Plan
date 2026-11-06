//! The factories read from a recording's rows, frame by frame: the tests' oracle of the
//! family's columnar preparation ([`super::facts`], [`super::emit`]), its whole reply, and the
//! faction totals' member counts.

use super::*;
use crate::{
    metadata::{IdentityTrack, slot_read, validate_campaign},
    products::country_history::{Identity, observe_identity, unique_by_slot},
};
use replay::container::Replay;
use replay_data::rows::{TableId, tables};
use replay_frame::{frame::DecodedFrame, frame::FrameDecoder};
use std::sync::atomic::Ordering;

/// One tag's runs, and the identity and hour its last run was extended to.
#[derive(Default, Serialize, Deserialize)]
struct Track {
    runs: Vec<FactoryRun>,
    last: Option<(Identity, i32)>,
    segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
    reads: Vec<FactoryReadRun>,
}

pub(super) fn append_read(
    reads: &mut Vec<FactoryReadRun>,
    hour: i32,
    read_hour: i32,
    hourly: bool,
    continues: bool,
) {
    if let Some(last) = reads.last_mut()
        && continues
        && last.last_hour.checked_add(1) == Some(hour)
        && last.hourly == hourly
        && (hourly || last.read_hour == read_hour)
    {
        last.last_hour = hour;
    } else {
        reads.push(FactoryReadRun {
            first_hour: hour,
            last_hour: hour,
            read_hour,
            hourly,
        });
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Current {
    identity: Identity,
    values: Option<Vec<i32>>,
    read: bool,
}
#[derive(Default)]
struct Rates {
    unexplained: BTreeMap<FactoryGroupKey, usize>,
    unreadable: usize,
    unnamed: usize,
}
impl Track {
    fn advance(&mut self, hour: i32) {
        let Some(segment) = self.segments.last_mut() else {
            return;
        };
        let before = segment.last_hour;
        segment.last_hour = hour;
        self.last_identity.advance(hour);
        if let Some((_, at)) = &mut self.last
            && *at == before
        {
            *at = hour;
            if let Some(run) = self.runs.last_mut() {
                run.last_hour = hour;
            }
            if let Some(read) = self.reads.last_mut() {
                read.last_hour = hour;
            }
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Scan {
    result: ScannedFactories,
    tracks: BTreeMap<String, Track>,
    frontier: Option<i32>,
    live: BTreeSet<String>,
    #[serde(skip)]
    countries: crate::products::force_census::Countries,
    #[serde(skip)]
    current: BTreeMap<String, Current>,
    #[serde(skip)]
    supplied_slots: BTreeSet<u32>,
    #[serde(skip)]
    rates: Rates,
    #[serde(skip)]
    read: Option<bool>,
    #[serde(skip)]
    observed: Vec<String>,
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
}
impl Scan {
    /// An hour whose factory inputs are the hour before's: the rates of the
    /// last read count again.
    fn unchanged(&mut self, hour: i32) {
        self.result.unreadable += self.rates.unreadable;
        self.result.unnamed += self.rates.unnamed;
        for (&key, &count) in &self.rates.unexplained {
            *self.result.unexplained.entry(key).or_default() += count;
        }
        self.frontier = Some(hour);
    }
    pub(crate) fn observe(
        &mut self,
        decoded: &DecodedFrame,
        format: replay_data::format::FormatVersion,
    ) {
        self.observe_changes(decoded, format, None);
        self.seal();
    }
    fn seal(&mut self) {
        if let Some(hour) = self.frontier {
            for tag in &self.live {
                self.tracks
                    .get_mut(tag)
                    .expect("live factory holder")
                    .advance(hour);
            }
        }
    }
    fn observe_changes(
        &mut self,
        decoded: &DecodedFrame,
        format: replay_data::format::FormatVersion,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) {
        self.observed.clear();
        let hour = decoded.hour;
        let continuous = self
            .frontier
            .is_some_and(|at| at.checked_add(1) == Some(hour));
        let read = slot_read(format, &decoded.metrics, TableId::CountryFactories, hour);
        let joined = self.countries.update_changed(decoded, changes);
        let changed = !continuous
            || self.read != Some(read)
            || !matches!(joined, Ok(false))
            || changes
                .is_none_or(|changes| changes.rows_changed[TableId::CountryFactories.index()]);
        if !changed {
            self.unchanged(hour);
            return;
        }
        if !continuous {
            self.seal();
        }
        let partial = continuous && matches!(joined, Ok(false)) && !self.current.is_empty();
        self.read = Some(read);
        let rows = decoded.complete::<tables::CountryFactories>().ok();
        let links = decoded.complete::<tables::CountryLinks>().ok();
        let mut rates = Rates::default();
        let by_slot = links.map(|links| {
            unique_by_slot(rows.filter(|_| read).into_iter().flatten(), links, |row| {
                row.country
            })
        });
        if let Some(rows) = &by_slot {
            rates.unreadable += rows.unresolved_rows + rows.duplicate_rows;
        }
        {
            self.work.row_visits += rows.filter(|_| read).map_or(0, |rows| rows.len());
        }
        let mut current = BTreeMap::new();
        if joined.is_ok() && links.is_some() {
            let mut slots = self.supplied_slots.clone();
            if read {
                slots.extend(
                    by_slot
                        .as_ref()
                        .into_iter()
                        .flat_map(|rows| rows.rows.keys())
                        .copied(),
                );
            }
            let candidates = if partial {
                self.countries.in_slots(&slots)
            } else {
                self.countries.tags.clone()
            };
            rates.unnamed += self.countries.unreadable_tags;
            for (tag, identity) in &candidates {
                let Some(identity) = *identity else {
                    continue;
                };
                let Ok(tag) = std::str::from_utf8(tag) else {
                    continue;
                };
                let values = if rows.is_none() {
                    None
                } else if read {
                    by_slot
                        .as_ref()
                        .and_then(|rows| rows.rows.get(&identity.0))
                        .copied()
                        .flatten()
                        .and_then(|row| {
                            let Some((values, unexplained)) = values(row) else {
                                rates.unreadable += 1;
                                return None;
                            };
                            for key in unexplained {
                                *rates.unexplained.entry(key).or_default() += 1;
                            }
                            Some(values)
                        })
                } else {
                    self.current
                        .get(tag)
                        .filter(|before| continuous && before.identity == identity)
                        .and_then(|before| before.values.clone())
                        // After restore the committed native tail owns this held read.
                        .or_else(|| {
                            self.tracks
                                .get(tag)
                                .filter(|_| continuous && self.live.contains(tag))
                                .and_then(|track| {
                                    track
                                        .last
                                        .filter(|(seen, at)| {
                                            *seen == identity && Some(*at) == self.frontier
                                        })
                                        .and_then(|_| {
                                            track.runs.last().map(|run| run.values.clone())
                                        })
                                })
                        })
                };
                current.insert(
                    tag.to_owned(),
                    Current {
                        identity,
                        values,
                        read,
                    },
                );
            }
        }
        if read || rows.is_none() {
            self.supplied_slots = by_slot
                .as_ref()
                .into_iter()
                .flat_map(|rows| rows.rows.keys())
                .copied()
                .collect();
        }
        for (tag, after) in &current {
            if continuous && self.current.get(tag) == Some(after) {
                continue;
            }
            let track = self.tracks.entry(tag.clone()).or_default();
            if self.live.contains(tag)
                && let Some(before) = self.frontier
            {
                track.advance(before);
            }
            observe_identity(
                &mut track.segments,
                &mut track.last_identity,
                after.identity,
                hour,
            );
            if let Some(values) = &after.values {
                let continues = track.last.is_some_and(|(seen, at)| {
                    seen == after.identity && at.checked_add(1) == Some(hour)
                });
                let supplying = if read {
                    hour
                } else {
                    track.reads.last().map_or(hour, |last| {
                        if last.hourly {
                            last.last_hour
                        } else {
                            last.read_hour
                        }
                    })
                };
                append_read(&mut track.reads, hour, supplying, read, continues);
                match track.runs.last_mut() {
                    Some(run) if continues && run.values == *values => run.last_hour = hour,
                    _ => track.runs.push(FactoryRun {
                        first_hour: hour,
                        last_hour: hour,
                        starts_segment: !continues,
                        values: values.clone(),
                    }),
                }
                track.last = Some((after.identity, hour));
            }
            self.observed.push(tag.clone());
            {
                self.work.history_visits += 1;
            }
        }
        for tag in self
            .live
            .iter()
            .filter(|tag| !partial && !current.contains_key(*tag))
        {
            if let Some(before) = self.frontier {
                self.tracks
                    .get_mut(tag)
                    .expect("departed factory holder")
                    .advance(before);
            }
            self.observed.push(tag.clone());
        }
        if partial {
            self.current.extend(current);
        } else {
            self.live = current.keys().cloned().collect();
            self.current = current;
        }
        self.result.unreadable += rates.unreadable;
        self.result.unnamed += rates.unnamed;
        for (&key, &count) in &rates.unexplained {
            *self.result.unexplained.entry(key).or_default() += count;
        }
        self.rates = rates;
        self.frontier = Some(hour);
    }
    /// The reads refused so far: unreadable rows and unnamed country-hours.
    pub(super) fn refused(&self) -> usize {
        self.result.unreadable + self.result.unnamed
    }
    pub(super) fn finish(mut self) -> ScannedFactories {
        self.seal();
        let Self {
            mut result, tracks, ..
        } = self;
        for (tag, track) in tracks {
            if !track.runs.is_empty() {
                result.segments.insert(tag.clone(), track.segments);
                result.reads.insert(tag.clone(), track.reads);
                result.series.insert(tag, track.runs);
            }
        }
        result
    }
}

/// Scans one branch once for every country.
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedFactories> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryFactories,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Factories request cancelled");
    let mut observer = Scan::default();
    crate::blocks::scan_blocks(
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
                    observer.observe(decoded, replay.format);
                }
            }
            Ok(())
        },
    )?;
    Ok(observer.finish())
}

/// Each branch's scan for this revision, and how many scans have run. Its lock is
/// held while a branch is scanned, as the service's other products hold their
/// caches: requests for the product on one source run one scan at a time, so
/// widgets asking at once (each with its own top N) share the first one's scan
/// instead of each scanning the branch and holding its history.
#[derive(Default)]
pub struct Scans {
    entries: BTreeMap<u64, Arc<ScannedFactories>>,
    pub runs: usize,
}

/// The branch's scan: the cached one, or one scanned now while the lock keeps any
/// other request for it waiting for this scan.
pub fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedFactories>> {
    let state = source.state::<Scans>();
    let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(scanned) = scans.entries.get(&branch) {
        return Ok(Arc::clone(scanned));
    }
    scans.runs += 1;
    let scanned = Arc::new(scan(&source.replay(), branch, cancelled)?);
    scans.entries.insert(branch, Arc::clone(&scanned));
    Ok(scanned)
}

/// Clips stale factory coverage around proven country-absence reads. Zero
/// evidence wins at an overlap, so one country never contributes twice.
pub(super) fn faction_measure_runs(
    recorded: &[FactoryRun],
    absent: &[HourSpan],
    mode: FactoryCount,
    groups: &[FactoryGroupKey],
    count: &mut usize,
    cancelled: &AtomicBool,
) -> io::Result<Vec<crate::products::faction_totals::MeasureRun>> {
    use crate::products::faction_totals::{MeasureRun, push_measure};
    let mut runs = Vec::new();
    let mut absent_index = 0usize;
    for run in recorded {
        let mut first = i64::from(run.first_hour);
        let last = i64::from(run.last_hour);
        let value = Some(
            groups
                .iter()
                .map(|group| group_count(&run.values, *group, mode))
                .sum(),
        );
        while absent
            .get(absent_index)
            .is_some_and(|span| i64::from(span.last_hour) < first)
        {
            absent_index += 1;
        }
        let mut index = absent_index;
        while let Some(span) = absent.get(index) {
            if i64::from(span.first_hour) > last {
                break;
            }
            if first < i64::from(span.first_hour) {
                push_measure(
                    &mut runs,
                    MeasureRun {
                        first_hour: first as i32,
                        last_hour: (i64::from(span.first_hour) - 1) as i32,
                        value,
                        starts_segment: run.starts_segment || first != i64::from(run.first_hour),
                    },
                    count,
                    cancelled,
                )?;
            }
            first = first.max(i64::from(span.last_hour) + 1);
            if first > last {
                break;
            }
            index += 1;
        }
        absent_index = index;
        if first <= last {
            push_measure(
                &mut runs,
                MeasureRun {
                    first_hour: first as i32,
                    last_hour: last as i32,
                    value,
                    starts_segment: run.starts_segment || first != i64::from(run.first_hour),
                },
                count,
                cancelled,
            )?;
        }
    }
    for span in absent {
        push_measure(
            &mut runs,
            MeasureRun {
                first_hour: span.first_hour,
                last_hour: span.last_hour,
                starts_segment: true,
                value: Some(0),
            },
            count,
            cancelled,
        )?;
    }
    runs.sort_by_key(|run| run.first_hour);
    debug_assert!(
        runs.windows(2)
            .all(|pair| pair[0].last_hour < pair[1].first_hour)
    );
    Ok(runs)
}

pub(crate) fn faction_tracks(
    source: &Source,
    branch: u64,
    tags: &[String],
    mode: FactoryCount,
    groups: &[FactoryGroupKey],
    history: &crate::products::country_groups::FactionHistory,
    cancelled: &AtomicBool,
) -> io::Result<crate::products::faction_totals::CountryMeasures> {
    use crate::products::faction_totals::{CountryMeasures, CountryTrack, for_each_absent};
    let scanned = scanned_branch(source, branch, cancelled)?;
    let groups = if groups.is_empty() {
        &FactoryGroupKey::ALL
    } else {
        groups
    };
    let mut count = 0usize;
    let mut tracks = Vec::with_capacity(tags.len());
    for tag in tags {
        let mut absent = Vec::new();
        for_each_absent(
            &history.country_reads,
            history
                .country_presence
                .get(tag)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            cancelled,
            |first_hour, last_hour| {
                absent.push(HourSpan {
                    first_hour,
                    last_hour,
                });
                Ok(())
            },
        )?;
        tracks.push(CountryTrack {
            tag: tag.clone(),
            runs: faction_measure_runs(
                scanned
                    .series
                    .get(tag)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                &absent,
                mode,
                groups,
                &mut count,
                cancelled,
            )?,
        });
    }
    let mut warnings = Vec::new();
    if scanned.unreadable > 0 || scanned.unnamed > 0 {
        warnings.push(format!("{} factory reads could not name a country or show its count; affected faction totals are gaps.", scanned.unreadable + scanned.unnamed));
    }
    Ok(CountryMeasures {
        tracks,
        unit: Some("factories".into()),
        unavailable_reason: None,
        warnings,
    })
}

/// The whole reply from the recording's rows.
pub(super) fn load(
    source: &Source,
    params: &FactoriesParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<CountryFactoriesReply> {
    let branch = params.branch_id.parse::<u64>().expect("validated branch");
    let scanned = scanned_branch(source, branch, cancelled)?;
    let history = if params.top.is_none() && !params.tags.is_empty() {
        Some(crate::products::country_history::branch_histories(
            source,
            branch,
            false,
            &[],
            cancelled,
        )?)
    } else {
        None
    };
    let mut selection = params.clone();
    if let Some(history) = &history {
        selection.tags =
            crate::products::country_history::selected_aliases(history, &params.tags);
    }
    present(
        source,
        &selection,
        context,
        cancelled,
        &scanned,
        history.as_deref(),
        None,
    )
}
