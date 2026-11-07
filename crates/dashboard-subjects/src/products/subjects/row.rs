//! The subject history read hour by hour from the recording's rows: the scan the family's
//! native preparation is held to ([`super::scan`]), kept as its parity oracle.

use super::{
    Counts, Feed, LevelStep, SubjectReply, SubjectRun, SubjectStart, active, sort_runs, warnings,
};
use dashboard_base::{
    Source,
    products::{
        LoadContext,
        country_groups::{HourSpan, extend, mark_unobserved},
    },
};
use dashboard_text::metadata::{
    CountryKeys, LinkTags, branch_bounds, link_slots, validate_campaign,
};
use replay::campaign::Replay;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// A relation's identity: its countries' (slot, id) and its start hour.
type RelationStart = ((u32, u32), (u32, u32), i32);

/// One branch's subject history, before levels are named.
#[derive(Clone, Default)]
pub struct Scan {
    pub(crate) runs: Vec<SubjectRun>,
    pub(crate) unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
    pub starts: Vec<SubjectStart>,
    pub observed: bool,
    open: BTreeMap<(String, String, i32), usize>,
    tags: LinkTags,
    unmapped: BTreeSet<u32>,
    incomplete: u32,
    unleveled: u32,
    unreadable: u32,
    conflicting: BTreeSet<String>,
    unreadable_blocks: u32,
    stopped: u32,
    seen_starts: BTreeSet<RelationStart>,
    named_since: BTreeMap<u32, (String, i32)>,
    continuous_since: Option<i32>,
    first_observed: Option<i32>,
    previous_complete: Option<i32>,
    previous_identities: BTreeMap<u32, u32>,
    identities: BTreeMap<u32, u32>,
    identity_since: BTreeMap<u32, (u32, i32)>,
    previous_hour: Option<i32>,
}

impl Scan {
    pub fn new(replay: &Replay, branch: u64) -> Self {
        Self {
            previous_hour: replay
                .branches
                .iter()
                .find(|start| start.id == branch)
                .and_then(|start| branch_bounds(replay, start).0.checked_sub(1)),
            ..Self::default()
        }
    }

    pub fn runs(&self) -> &[SubjectRun] {
        &self.runs
    }

    pub fn unobserved(&self) -> &[HourSpan] {
        &self.unobserved
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn unobserved_hour(&mut self, hour: i32) {
        self.incomplete += 1;
        mark_unobserved(&mut self.unobserved, hour, hour);
        self.continuous_since = None;
        self.previous_complete = None;
    }

    /// Observes one hour whole.
    pub fn observe(&mut self, decoded: &DecodedFrame) {
        let hour = decoded.hour;
        let first = *self.first_observed.get_or_insert(hour);
        if let Some(previous) = self.previous_hour
            && hour > previous + 1
        {
            mark_unobserved(&mut self.unobserved, previous + 1, hour - 1);
            self.continuous_since = None;
        }
        self.previous_hour = Some(hour);
        let keys = decoded
            .complete::<tables::Countries>()
            .ok()
            .map(CountryKeys::read);
        let links = decoded
            .complete::<tables::CountryLinks>()
            .ok()
            .map(link_slots);
        if self.tags.update_projected(keys.as_ref(), links.as_ref()).is_err() {
            self.unobserved_hour(hour);
            return;
        }
        let (Some(keys), Some(links)) = (keys, links) else {
            self.unobserved_hour(hour);
            return;
        };
        self.identities = keys.iter().map(|(id, slot, _)| (slot, id)).collect();
        self.identity_since
            .retain(|slot, (id, _)| self.identities.get(slot) == Some(id));
        for (&slot, &id) in &self.identities {
            self.identity_since.entry(slot).or_insert((id, hour));
        }
        self.named_since
            .retain(|raw, (tag, _)| self.tags.get(*raw) == Some(tag));
        for (raw, tag) in self.tags.iter() {
            self.named_since
                .entry(*raw)
                .or_insert_with(|| (tag.clone(), hour));
        }
        let Ok(rows) = decoded.complete::<tables::Subjects>() else {
            self.unobserved_hour(hour);
            return;
        };
        self.observed = true;
        let since = *self.continuous_since.get_or_insert(hour);
        // Each subject's current level this hour, by tag; none when the table is not
        // complete. A subject the rows give two levels has no known level.
        let levels = decoded.complete::<tables::Autonomy>().ok().map(|autonomy| {
            let mut levels: HashMap<String, Option<Vec<u8>>> = HashMap::new();
            for row in autonomy {
                let Some(tag) = self.tags.get(row.subject) else {
                    continue;
                };
                let level = levels
                    .entry(tag.clone())
                    .or_insert_with(|| Some(row.state.clone()));
                if level.as_deref() != Some(row.state.as_slice()) {
                    self.conflicting.insert(tag.clone());
                    *level = None;
                }
            }
            levels
        });
        let mut named_all = true;
        for row in rows.iter().filter(|row| active(row, hour)) {
            let (Some(overlord), Some(subject)) =
                (self.tags.get(row.overlord), self.tags.get(row.subject))
            else {
                for raw in [row.overlord, row.subject] {
                    if self.tags.get(raw).is_none() {
                        self.unmapped.insert(raw);
                    }
                }
                named_all = false;
                continue;
            };
            // A level whose recorded id is not text (or is empty) is refused, and counted.
            let level = match levels
                .as_ref()
                .and_then(|levels| levels.get(subject).and_then(|level| level.as_deref()))
            {
                Some(bytes) => match std::str::from_utf8(bytes) {
                    Ok(token) if !token.is_empty() => Some(token.to_owned()),
                    _ => {
                        self.unreadable += 1;
                        None
                    }
                },
                None => None,
            };
            if level.is_none() {
                self.unleveled += 1;
            }
            let key = (overlord.clone(), subject.clone(), row.start_hour);
            let started = extend(&mut self.open, &mut self.runs, key.clone(), hour, || {
                SubjectRun {
                    overlord: overlord.clone(),
                    subject: subject.clone(),
                    start_hour: row.start_hour,
                    first_hour: hour,
                    last_hour: hour,
                    steps: Vec::new(),
                }
            });
            if started {
                let identity_key = [row.overlord, row.subject].map(|raw| {
                    links
                        .get(&raw)
                        .copied()
                        .and_then(|slot| self.identities.get(&slot).map(|id| (slot, *id)))
                });
                let identity_key = identity_key[0]
                    .zip(identity_key[1])
                    .map(|(overlord, subject)| (overlord, subject, row.start_hour));
                let new_identity =
                    identity_key.is_some_and(|identity| self.seen_starts.insert(identity));
                if new_identity && row.start_hour > first {
                    // Relations serialize start_date; names are exact only while both raw
                    // links keep their tags. A start at or before the first observed frame is
                    // an initial snapshot, even if first published later.
                    let date_named = [row.overlord, row.subject].into_iter().all(|raw| {
                        self.named_since
                            .get(&raw)
                            .is_some_and(|(_, from)| since.max(*from) <= row.start_hour)
                    }) && identity_key.is_some_and(|(overlord, subject, _)| {
                        [overlord, subject].into_iter().all(|(slot, id)| {
                            self.identity_since
                                .get(&slot)
                                .is_some_and(|(seen, from)| *seen == id && *from <= row.start_hour)
                        })
                    });
                    let exact = date_named && row.start_hour <= hour;
                    let witnessed = self
                        .previous_complete
                        .is_some_and(|previous: i32| previous.checked_add(1) == Some(hour))
                        && identity_key.is_some_and(|(overlord, subject, _)| {
                            [overlord, subject]
                                .into_iter()
                                .all(|(slot, id)| self.previous_identities.get(&slot) == Some(&id))
                        });
                    if exact || witnessed {
                        self.starts.push(SubjectStart {
                            subject: subject.clone(),
                            overlord: overlord.clone(),
                            hour: if exact { row.start_hour } else { hour },
                            read_hour: hour,
                            exact,
                        });
                    }
                }
            }
            let run = &mut self.runs[self.open[&key]];
            match run.steps.last_mut() {
                Some(step) if step.last_hour == hour => {}
                Some(step) if step.level == level && step.last_hour + 1 == hour && !started => {
                    step.last_hour = hour;
                }
                _ => {
                    run.steps.push(LevelStep {
                        first_hour: hour,
                        last_hour: hour,
                        level,
                    });
                }
            }
        }
        if !named_all {
            mark_unobserved(&mut self.unobserved, hour, hour);
        }
        self.previous_complete = named_all.then_some(hour);
        self.previous_identities.clone_from(&self.identities);
        self.open
            .retain(|_, index| self.runs[*index].last_hour == hour);
    }

    /// What the scan counted.
    pub fn counts(&self) -> Counts {
        Counts {
            unmapped: self.unmapped.len() as u64,
            incomplete: u64::from(self.incomplete),
            unleveled: u64::from(self.unleveled),
            unreadable: u64::from(self.unreadable),
            conflicting: self.conflicting.len() as u64,
            unreadable_blocks: u64::from(self.unreadable_blocks),
            stopped: u64::from(self.stopped),
            observed: self.observed,
        }
    }

    /// The branch's history once its last hour is observed: the hours after it unobserved,
    /// the runs in the reply's order and the warnings.
    fn settle(&mut self, replay: &Replay, branch: u64) {
        let bounds = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map(|start| branch_bounds(replay, start));
        if let (Some((first, last)), previous) = (bounds, self.previous_hour) {
            let from = previous.map_or(first, |previous| previous.saturating_add(1));
            if from <= last {
                mark_unobserved(&mut self.unobserved, from, last);
            }
        }
        sort_runs(&mut self.runs);
        self.warnings = warnings(&self.counts());
    }

    pub fn reply(
        &self,
        source: &Source,
        branch_id: &str,
        context: &LoadContext,
    ) -> io::Result<SubjectReply> {
        super::reply(
            source,
            branch_id,
            context,
            self.runs.clone(),
            self.unobserved.clone(),
            &self.warnings,
        )
    }

    /// What the event feed reads of the branch.
    pub fn feed(&self) -> Feed {
        Feed::new(
            self.starts.clone(),
            self.observed,
            warnings(&Counts {
                unreadable_blocks: 0,
                stopped: 0,
                ..self.counts()
            }),
        )
    }
}

/// Each branch's scan, kept for this source revision.
#[derive(Default)]
struct Scans(BTreeMap<u64, Arc<Scan>>);

pub fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Scan>> {
    let state = source.state::<Scans>();
    let cached = state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .0
        .get(&branch)
        .map(Arc::clone);
    // Scan outside the lock, so requests for other branches are not held up.
    let scan = match cached {
        Some(scan) => scan,
        None => {
            let scan = Arc::new(scan_branch(&source.replay(), branch, cancelled)?);
            let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
            Arc::clone(state.0.entry(branch).or_insert(scan))
        }
    };
    Ok(scan)
}

pub fn scan_branch(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Subjects,
        TableId::Autonomy,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Subjects request cancelled");
    let mut scan = Scan::new(replay, branch);
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
                let Ok(frames) = frames else {
                    scan.unreadable_blocks += 1;
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = Ok::<&DecodedFrame, std::io::Error>(&frame.decoded) else {
                        scan.stopped += 1;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        scan.stopped += 1;
                        break;
                    }
                    scan.observe(decoded);
                }
            }
            Ok(())
        },
    )?;
    scan.settle(replay, branch);
    Ok(scan)
}

/// What the event feed reads of `branch`, from the recording's rows.
pub fn feed(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Feed> {
    Ok(scan_branch(replay, branch, cancelled)?.feed())
}
