
use std::io;

use crate::codec::{Decoder, invalid_static as invalid, put_string};
use crate::{BranchId, Hour, SessionId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parentage {
    Probable { branch: BranchId, hour: Hour },
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchReason {
    SessionStart,
    LoadSignal,
    HourDiscontinuity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchStart {
    pub id: BranchId,
    pub session: SessionId,
    pub loaded_hour: Option<Hour>,
    pub first_hour: Hour,
    pub parent: Parentage,
    pub reason: BranchReason,
}

impl BranchStart {
    pub fn encoded_bytes(&self) -> usize {
        size_of::<BranchId>()
            + size_of::<SessionId>()
            + size_of::<u8>()
            + usize::from(self.loaded_hour.is_some()) * size_of::<Hour>()
            + size_of::<Hour>()
            + size_of::<u8>()
            + match self.parent {
                Parentage::Unknown => 0,
                Parentage::Probable { .. } => size_of::<BranchId>() + size_of::<Hour>(),
            }
            + size_of::<u8>()
    }

    pub fn validate(&self) -> io::Result<()> {
        if let Parentage::Probable { branch, hour } = self.parent {
            if branch == self.id {
                return Err(invalid("a branch cannot be its own parent"));
            }
            if Some(hour) != self.loaded_hour {
                return Err(invalid("parent hour differs from loaded hour"));
            }
        }
        Ok(())
    }

    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        engine_allocation::reserve(out, self.encoded_bytes())?;
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.session.to_le_bytes());
        match self.loaded_hour {
            None => out.push(0),
            Some(hour) => {
                out.push(1);
                out.extend_from_slice(&hour.to_le_bytes());
            }
        }
        out.extend_from_slice(&self.first_hour.to_le_bytes());
        match self.parent {
            Parentage::Unknown => out.push(0),
            Parentage::Probable { branch, hour } => {
                out.push(1);
                out.extend_from_slice(&branch.to_le_bytes());
                out.extend_from_slice(&hour.to_le_bytes());
            }
        }
        out.push(match self.reason {
            BranchReason::SessionStart => 0,
            BranchReason::LoadSignal => 1,
            BranchReason::HourDiscontinuity => 2,
        });
        Ok(())
    }

    pub fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let id = input.u64()?;
        let session = input.u64()?;
        let loaded_hour = match input.u8()? {
            0 => None,
            1 => Some(input.i32()?),
            _ => return Err(invalid("invalid loaded-hour option tag")),
        };
        let first_hour = input.i32()?;
        let parent = match input.u8()? {
            0 => Parentage::Unknown,
            1 => Parentage::Probable {
                branch: input.u64()?,
                hour: input.i32()?,
            },
            _ => return Err(invalid("invalid parentage tag")),
        };
        let reason = match input.u8()? {
            0 => BranchReason::SessionStart,
            1 => BranchReason::LoadSignal,
            2 => BranchReason::HourDiscontinuity,
            _ => return Err(invalid("invalid branch reason tag")),
        };
        let start = Self {
            id,
            session,
            loaded_hour,
            first_hour,
            parent,
            reason,
        };
        start.validate()?;
        Ok(start)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub branch: BranchId,
    pub first_hour: Hour,
    pub last_hour: Hour,
    pub reason: String,
}

impl Gap {
    pub fn encoded_bytes(&self) -> io::Result<usize> {
        engine_allocation::add(
            size_of::<BranchId>() + 2 * size_of::<Hour>() + size_of::<u32>(),
            self.reason.len(),
        )
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            branch: self.branch,
            first_hour: self.first_hour,
            last_hour: self.last_hour,
            reason: engine_allocation::string(&self.reason)?,
        })
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.first_hour > self.last_hour {
            return Err(invalid("gap hour range is reversed"));
        }
        if self.reason.trim().is_empty() {
            return Err(invalid("gap must have a reason"));
        }
        Ok(())
    }

    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        engine_allocation::reserve(out, self.encoded_bytes()?)?;
        out.extend_from_slice(&self.branch.to_le_bytes());
        out.extend_from_slice(&self.first_hour.to_le_bytes());
        out.extend_from_slice(&self.last_hour.to_le_bytes());
        put_string(out, &self.reason)
    }

    pub fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let gap = Self {
            branch: input.u64()?,
            first_hour: input.i32()?,
            last_hour: input.i32()?,
            reason: input.string()?,
        };
        gap.validate()?;
        Ok(gap)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub branch: BranchId,
    pub start: Option<BranchStart>,
}

#[derive(Debug)]
struct BranchHistory {
    start: BranchStart,
    recorded: Vec<Hour>,
    pending: Vec<(Hour, bool)>,
}

#[derive(Clone, Copy, Debug)]
struct LastObservation {
    branch: BranchId,
    hour: Hour,
    load_signal: Option<i64>,
    frame_hour: Option<Hour>,
}

#[derive(Debug)]
pub struct BranchTracker {
    world_changed: bool,
    sessions: Vec<SessionId>,
    session: Option<SessionId>,
    last: Option<LastObservation>,
    branches: Vec<BranchHistory>,
    next_branch: Option<BranchId>,
}

impl Default for BranchTracker {
    fn default() -> Self {
        Self {
            world_changed: false,
            sessions: Vec::new(),
            session: None,
            last: None,
            branches: Vec::new(),
            next_branch: Some(1),
        }
    }
}

impl BranchTracker {
    pub fn world_changed(&mut self) {
        self.world_changed = true;
    }
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed_recorded_branch(
        &mut self,
        start: BranchStart,
        hours: impl IntoIterator<Item = Hour>,
    ) -> io::Result<()> {
        start.validate()?;
        if self.session.is_some() {
            return Err(invalid("seed branch history before beginning a session"));
        }
        if self.history(start.id).is_some() {
            return Err(invalid("duplicate branch id"));
        }
        let mut recorded = Vec::new();
        for hour in hours {
            if hour < start.first_hour {
                return Err(invalid("recorded hour precedes branch start"));
            }
            engine_allocation::reserve(&mut recorded, 1)?;
            recorded.push(hour);
        }
        recorded.sort_unstable();
        if recorded.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("duplicate recorded hour in branch history"));
        }
        engine_allocation::reserve(&mut self.branches, 1)?;
        if !self.sessions.contains(&start.session) {
            engine_allocation::reserve(&mut self.sessions, 1)?;
        }
        if self.next_branch.is_some_and(|next| start.id >= next) {
            self.next_branch = start.id.checked_add(1);
        }
        if !self.sessions.contains(&start.session) {
            self.sessions.push(start.session);
        }
        let position = self
            .branches
            .partition_point(|history| history.start.id < start.id);
        self.branches.insert(
            position,
            BranchHistory {
                start,
                recorded,
                pending: Vec::new(),
            },
        );
        Ok(())
    }

    pub fn begin_session(&mut self, session: SessionId) -> io::Result<()> {
        if self.sessions.contains(&session) {
            return Err(invalid("session id has already been used"));
        }
        engine_allocation::reserve(&mut self.sessions, 1)?;
        self.sessions.push(session);
        self.session = Some(session);
        self.last = None;
        Ok(())
    }

    pub fn current_branch(&self) -> Option<BranchId> {
        self.last.map(|last| last.branch)
    }

    pub fn last_load_signal(&self) -> Option<i64> {
        self.last.and_then(|last| last.load_signal)
    }

    pub fn last_observation(&self) -> Option<(BranchId, Hour)> {
        self.last.map(|last| (last.branch, last.hour))
    }

    pub fn resume_session(
        &mut self,
        session: SessionId,
        branch: BranchId,
        hour: Hour,
        load_signal: Option<i64>,
    ) -> io::Result<()> {
        if self.session.is_some() {
            return Err(invalid("resume before beginning or observing a session"));
        }
        let history = self
            .history(branch)
            .ok_or_else(|| invalid("unknown resumed branch"))?;
        if history.start.session != session || history.recorded.last() != Some(&hour) {
            return Err(invalid(
                "resume does not identify that process's last recorded branch hour",
            ));
        }
        self.session = Some(session);
        self.last = Some(LastObservation {
            branch,
            hour,
            load_signal,
            frame_hour: Some(hour),
        });
        Ok(())
    }

    pub fn resume_observation(
        &mut self,
        session: SessionId,
        branch: BranchId,
        hour: Hour,
        load_signal: Option<i64>,
    ) -> io::Result<()> {
        if self.session.is_some() {
            return Err(invalid("resume before beginning or observing a session"));
        }
        let history = self
            .history(branch)
            .ok_or_else(|| invalid("unknown observed branch"))?;
        if history.start.session != session
            || hour < history.start.first_hour
            || history
                .recorded
                .last()
                .is_some_and(|recorded| *recorded > hour)
        {
            return Err(invalid(
                "observed tail conflicts with recorded branch history",
            ));
        }
        self.session = Some(session);
        self.last = Some(LastObservation {
            branch,
            hour,
            load_signal,
            frame_hour: Some(hour),
        });
        Ok(())
    }

    pub fn resume_commands(
        &mut self,
        session: SessionId,
        branch: BranchId,
        hour: Hour,
        load_signal: Option<i64>,
    ) -> io::Result<()> {
        self.resume_observation(session, branch, hour, load_signal)?;
        let frame_hour = self
            .history(branch)
            .and_then(|history| history.recorded.last().copied());
        self.last.as_mut().unwrap().frame_hour = frame_hour;
        Ok(())
    }

    pub fn branch_start(&self, branch: BranchId) -> Option<&BranchStart> {
        self.history(branch).map(|history| &history.start)
    }

    pub fn branches(&self) -> impl ExactSizeIterator<Item = &BranchStart> {
        self.branches.iter().map(|history| &history.start)
    }

    pub fn recorded_hours(&self, branch: BranchId) -> Option<&[Hour]> {
        self.history(branch)
            .map(|history| history.recorded.as_slice())
    }

    fn history(&self, branch: BranchId) -> Option<&BranchHistory> {
        let index = self
            .branches
            .binary_search_by_key(&branch, |history| history.start.id)
            .ok()?;
        Some(&self.branches[index])
    }

    fn history_mut(&mut self, branch: BranchId) -> Option<&mut BranchHistory> {
        let index = self
            .branches
            .binary_search_by_key(&branch, |history| history.start.id)
            .ok()?;
        Some(&mut self.branches[index])
    }

    pub fn observe(
        &mut self,
        loaded_hour: Option<Hour>,
        recorded_hour: Hour,
        load_signal: Option<i64>,
        known_gaps: &[Gap],
    ) -> io::Result<Observation> {
        self.observe_clock(loaded_hour, recorded_hour, load_signal, known_gaps, true)
    }

    pub fn observe_commands(
        &mut self,
        hour: Hour,
        load_signal: Option<i64>,
    ) -> io::Result<Observation> {
        self.observe_clock(None, hour, load_signal, &[], false)
    }

    pub fn past_commands(&mut self, hour: Hour) -> io::Result<BranchId> {
        let session = self
            .session
            .ok_or_else(|| invalid("begin a session before observing"))?;
        let id = self
            .next_branch
            .ok_or_else(|| invalid("branch id space exhausted"))?;
        engine_allocation::reserve(&mut self.branches, 1)?;
        self.branches.push(BranchHistory {
            start: BranchStart {
                id,
                session,
                loaded_hour: None,
                first_hour: hour,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            },
            recorded: Vec::new(),
            pending: Vec::new(),
        });
        self.next_branch = id.checked_add(1);
        Ok(id)
    }

    fn observe_clock(
        &mut self,
        loaded_hour: Option<Hour>,
        recorded_hour: Hour,
        load_signal: Option<i64>,
        known_gaps: &[Gap],
        frame: bool,
    ) -> io::Result<Observation> {
        let session = self
            .session
            .ok_or_else(|| invalid("begin a session before observing"))?;
        for gap in known_gaps {
            gap.validate()?;
            if self.current_branch() != Some(gap.branch) {
                return Err(invalid("gap does not belong to the current branch"));
            }
            let history = self.history(gap.branch).unwrap();
            let first = history
                .recorded
                .partition_point(|hour| *hour < gap.first_hour);
            if history
                .recorded
                .get(first)
                .is_some_and(|hour| *hour <= gap.last_hour)
            {
                return Err(invalid("gap includes an already recorded hour"));
            }
        }

        let reason = match self.last {
            None => Some(BranchReason::SessionStart),
            Some(_) if self.world_changed => Some(BranchReason::LoadSignal),
            Some(last) if matches!((last.load_signal, load_signal), (Some(a), Some(b)) if a != b) => {
                Some(BranchReason::LoadSignal)
            }
            Some(last)
                if last.hour == recorded_hour
                    && (!frame || last.frame_hour != Some(recorded_hour)) =>
            {
                None
            }
            Some(last) if !continues(last.hour, recorded_hour, known_gaps)? => {
                Some(BranchReason::HourDiscontinuity)
            }
            Some(_) => None,
        };
        if reason.is_none() && known_gaps.iter().any(|gap| gap.last_hour >= recorded_hour) {
            return Err(invalid(
                "a continuing branch's gap must precede its observed hour",
            ));
        }
        let start = reason
            .map(|reason| {
                let id = self
                    .next_branch
                    .ok_or_else(|| invalid("branch id space exhausted"))?;
                Ok::<_, io::Error>(BranchStart {
                    id,
                    session,
                    loaded_hour,
                    first_hour: recorded_hour,
                    parent: loaded_hour.map_or(Parentage::Unknown, |hour| self.parent_at(hour, id)),
                    reason,
                })
            })
            .transpose()?;

        if start.is_some() {
            engine_allocation::reserve(&mut self.branches, 1)?;
        } else if frame {
            engine_allocation::reserve(
                &mut self.history_mut(self.last.unwrap().branch).unwrap().pending,
                1,
            )?;
        }
        let mut first_pending = Vec::new();
        if start.is_some() && frame {
            engine_allocation::reserve(&mut first_pending, 1)?;
            first_pending.push((recorded_hour, false));
        }

        if let Some(last) = self.last {
            let history = self.history_mut(last.branch).unwrap();
            for gap in known_gaps {
                history
                    .pending
                    .retain(|(hour, _)| !(gap.first_hour..=gap.last_hour).contains(hour));
            }
        }
        let branch = match &start {
            Some(start) => {
                self.next_branch = start.id.checked_add(1);
                self.branches.push(BranchHistory {
                    start: start.clone(),
                    recorded: Vec::new(),
                    pending: first_pending,
                });
                start.id
            }
            None => self.last.unwrap().branch,
        };
        if start.is_none() && frame {
            self.history_mut(branch)
                .unwrap()
                .pending
                .push((recorded_hour, false));
        }
        self.last = Some(LastObservation {
            branch,
            hour: recorded_hour,
            load_signal: load_signal.or(self.last.and_then(|last| last.load_signal)),
            frame_hour: if frame {
                Some(recorded_hour)
            } else if start.is_none() {
                self.last.and_then(|last| last.frame_hour)
            } else {
                None
            },
        });
        self.world_changed = false;
        Ok(Observation { branch, start })
    }

    pub fn record_hour(&mut self, branch: BranchId, hour: Hour) -> io::Result<()> {
        let history = self
            .history_mut(branch)
            .ok_or_else(|| invalid("unknown branch"))?;
        let Some(position) = history
            .pending
            .iter()
            .position(|(pending, _)| *pending == hour)
        else {
            return Err(invalid(
                "hour was not observed, was already recorded, or was declared missing",
            ));
        };
        engine_allocation::reserve(&mut history.recorded, 1)?;
        history.pending.remove(position);
        let index = history
            .recorded
            .partition_point(|recorded| *recorded < hour);
        history.recorded.insert(index, hour);
        Ok(())
    }

    pub fn admit_hour(&mut self, branch: BranchId, hour: Hour) -> io::Result<()> {
        let history = self
            .history_mut(branch)
            .ok_or_else(|| invalid("unknown branch"))?;
        let Some((_, queued)) = history
            .pending
            .iter_mut()
            .find(|(pending, queued)| *pending == hour && !*queued)
        else {
            return Err(invalid(
                "hour was not observed, was already queued, or was declared missing",
            ));
        };
        *queued = true;
        Ok(())
    }

    pub fn lose_hour(&mut self, branch: BranchId, hour: Hour) -> io::Result<()> {
        let history = self
            .history_mut(branch)
            .ok_or_else(|| invalid("unknown branch"))?;
        let Some(position) = history
            .pending
            .iter()
            .position(|(pending, _)| *pending == hour)
        else {
            return Err(invalid(
                "hour was not observed, was already recorded, or was declared missing",
            ));
        };
        let (_, queued) = history.pending.remove(position);
        if queued {
            for index in 0..self.branches.len() {
                let start = &self.branches[index].start;
                if start.id > branch && start.loaded_hour == Some(hour) {
                    let parent = self.parent_at(hour, start.id);
                    self.branches[index].start.parent = parent;
                }
            }
        }
        Ok(())
    }

    fn parent_at(&self, hour: Hour, before: BranchId) -> Parentage {
        let mut containing = self
            .branches
            .iter()
            .filter(|history| {
                history.start.id < before
                    && (history.recorded.binary_search(&hour).is_ok()
                        || history.pending.contains(&(hour, true)))
            })
            .map(|history| history.start.id);
        match (containing.next(), containing.next()) {
            (Some(branch), None) => Parentage::Probable { branch, hour },
            _ => Parentage::Unknown,
        }
    }
}

fn continues(previous: Hour, next: Hour, gaps: &[Gap]) -> io::Result<bool> {
    let first_missing = i64::from(previous) + 1;
    let next = i64::from(next);
    if next < first_missing {
        return Ok(false);
    }
    if next == first_missing {
        return Ok(true);
    }
    let mut ranges = engine_allocation::vec(gaps.len())?;
    for gap in gaps {
        ranges.push((gap.first_hour, gap.last_hour));
    }
    ranges.sort_unstable();
    let mut uncovered = first_missing;
    for (first, last) in ranges {
        if i64::from(last) < uncovered {
            continue;
        }
        if i64::from(first) > uncovered {
            return Ok(false);
        }
        uncovered = i64::from(last) + 1;
        if uncovered >= next {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resuming_an_observed_gap_never_claims_missing_hours_as_recorded() {
        let mut tracker = BranchTracker::new();
        tracker
            .seed_recorded_branch(
                BranchStart {
                    id: 1,
                    session: 7,
                    loaded_hour: None,
                    first_hour: 2,
                    parent: Parentage::Unknown,
                    reason: BranchReason::SessionStart,
                },
                [],
            )
            .unwrap();
        assert!(tracker.resume_observation(8, 1, 4, Some(19)).is_err());
        assert!(tracker.resume_observation(7, 1, 1, Some(19)).is_err());
        tracker.resume_observation(7, 1, 4, Some(19)).unwrap();
        assert!(tracker.recorded_hours(1).unwrap().is_empty());
        let next = tracker.observe(None, 9, Some(20), &[]).unwrap();
        assert_eq!(next.branch, 2);
        assert_eq!(next.start.unwrap().parent, Parentage::Unknown);
        assert!(tracker.recorded_hours(1).unwrap().is_empty());
        assert!(tracker.recorded_hours(2).unwrap().is_empty());
    }

    #[test]
    fn native_world_change_splits_an_otherwise_continuous_clock_once() {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(1).unwrap();
        let first = tracker.observe(None, 10, Some(7), &[]).unwrap().branch;
        tracker.record_hour(first, 10).unwrap();
        assert_eq!(
            tracker.observe(Some(11), 11, Some(7), &[]).unwrap().branch,
            first
        );
        tracker.world_changed();
        let changed = tracker.observe(Some(12), 12, Some(7), &[]).unwrap();
        assert_ne!(changed.branch, first);
        assert_eq!(changed.start.unwrap().reason, BranchReason::LoadSignal);
        assert_eq!(
            tracker.observe(Some(13), 13, Some(7), &[]).unwrap().branch,
            changed.branch
        );
    }

    #[test]
    fn reattaching_the_same_process_preserves_session_and_checks_for_reload() {
        let start = BranchStart {
            id: 4,
            session: 8,
            loaded_hour: Some(9),
            first_hour: 10,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        };
        let mut tracker = BranchTracker::new();
        tracker.seed_recorded_branch(start, [10, 11]).unwrap();
        assert!(tracker.resume_session(9, 4, 11, Some(1)).is_err());
        assert!(tracker.resume_session(8, 4, 10, Some(1)).is_err());
        tracker.resume_session(8, 4, 11, Some(1)).unwrap();
        let next = tracker.observe(Some(11), 12, Some(1), &[]).unwrap();
        assert_eq!(next.branch, 4);
        assert!(next.start.is_none());
        tracker.record_hour(4, 12).unwrap();
        let reload = tracker.observe(Some(10), 11, Some(2), &[]).unwrap();
        assert_eq!(
            reload.start.unwrap().parent,
            Parentage::Probable {
                branch: 4,
                hour: 10
            }
        );
    }

    fn sample(tracker: &mut BranchTracker, loaded: Hour, hour: Hour, signal: i64) -> Observation {
        let observed = tracker
            .observe(Some(loaded), hour, Some(signal), &[])
            .unwrap();
        tracker.record_hour(observed.branch, hour).unwrap();
        observed
    }

    fn gap(branch: BranchId, first: Hour, last: Hour) -> Gap {
        Gap {
            branch,
            first_hour: first,
            last_hour: last,
            reason: "queue overflow".into(),
        }
    }

    #[test]
    fn reloads_and_sessions_use_loaded_hour_and_exact_history() {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(7).unwrap();
        let original = sample(&mut tracker, 9, 10, 1).branch;
        sample(&mut tracker, 10, 11, 1);
        sample(&mut tracker, 11, 12, 1);
        let earlier = sample(&mut tracker, 10, 11, 2).start.unwrap();
        assert_eq!(earlier.reason, BranchReason::LoadSignal);
        assert_eq!(
            earlier.parent,
            Parentage::Probable {
                branch: original,
                hour: 10
            }
        );
        let same = sample(&mut tracker, 10, 11, 3).start.unwrap();
        assert_eq!(
            same.parent,
            Parentage::Probable {
                branch: original,
                hour: 10
            }
        );
        let later = sample(&mut tracker, 50, 51, 4).start.unwrap();
        assert_eq!(later.parent, Parentage::Unknown);
        let next = sample(&mut tracker, 51, 52, 5).start.unwrap();
        assert_eq!(next.reason, BranchReason::LoadSignal);
        assert_eq!(
            next.parent,
            Parentage::Probable {
                branch: later.id,
                hour: 51
            }
        );
        tracker.begin_session(8).unwrap();
        let restart = sample(&mut tracker, 52, 53, 5).start.unwrap();
        assert_eq!(restart.reason, BranchReason::SessionStart);
        assert_eq!(
            restart.parent,
            Parentage::Probable {
                branch: next.id,
                hour: 52
            }
        );
        assert!(tracker.begin_session(8).is_err());
        assert_eq!(tracker.current_branch(), Some(restart.id));
    }

    #[test]
    fn discontinuities_and_ambiguous_parents_are_not_guessed() {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(1).unwrap();
        sample(&mut tracker, 9, 10, 1);
        for (loaded, hour) in [(9, 10), (4, 5), (19, 20)] {
            let start = sample(&mut tracker, loaded, hour, 1).start.unwrap();
            assert_eq!(start.reason, BranchReason::HourDiscontinuity);
        }
        let ambiguous = sample(&mut tracker, 10, 11, 2).start.unwrap();
        assert_eq!(ambiguous.parent, Parentage::Unknown);
    }

    #[test]
    fn only_complete_explicit_gaps_explain_forward_jumps() {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(1).unwrap();
        let branch = sample(&mut tracker, 9, 10, 1).branch;
        let covered = [
            gap(branch, 13, 14),
            gap(branch, 11, 12),
            gap(branch, 12, 13),
        ];
        assert!(
            tracker
                .observe(Some(14), 15, Some(1), &covered)
                .unwrap()
                .start
                .is_none()
        );
        tracker.record_hour(branch, 15).unwrap();
        let holes = [gap(branch, 16, 16), gap(branch, 18, 19)];
        let changed = tracker
            .observe(Some(19), 20, Some(1), &holes)
            .unwrap()
            .start
            .unwrap();
        assert_eq!(changed.reason, BranchReason::HourDiscontinuity);
        let loaded = tracker
            .observe(Some(22), 23, Some(2), &[gap(changed.id, 21, 22)])
            .unwrap()
            .start
            .unwrap();
        assert_eq!(loaded.reason, BranchReason::LoadSignal);
    }

    #[test]
    fn lost_frames_and_gaps_never_supply_parentage() {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(1).unwrap();
        let first = sample(&mut tracker, 99, 100, 1).branch;
        tracker.observe(Some(100), 101, Some(1), &[]).unwrap();
        let loaded = tracker
            .observe(Some(101), 102, Some(2), &[gap(first, 101, 101)])
            .unwrap()
            .start
            .unwrap();
        assert_eq!(loaded.parent, Parentage::Unknown);
        assert!(tracker.record_hour(first, 101).is_err());
        assert!(tracker.record_hour(first, 100).is_err());
        assert!(tracker.record_hour(999, 102).is_err());
        tracker.record_hour(loaded.id, 102).unwrap();
        tracker
            .observe(Some(105), 106, Some(2), &[gap(loaded.id, 103, 105)])
            .unwrap();
        tracker.record_hour(loaded.id, 106).unwrap();
        assert_eq!(
            tracker
                .observe(Some(104), 105, Some(3), &[])
                .unwrap()
                .start
                .unwrap()
                .parent,
            Parentage::Unknown
        );
    }

    #[test]
    fn a_queued_hour_is_parentage_evidence_until_its_write_is_lost() {
        for lost in [false, true] {
            let mut tracker = BranchTracker::new();
            tracker.begin_session(1).unwrap();
            let first = sample(&mut tracker, 9, 10, 1).branch;
            tracker.observe(Some(10), 11, Some(1), &[]).unwrap();
            tracker.admit_hour(first, 11).unwrap();
            assert!(tracker.admit_hour(first, 11).is_err());
            let loaded = tracker
                .observe(Some(11), 11, Some(2), &[])
                .unwrap()
                .start
                .unwrap();
            let parent = Parentage::Probable {
                branch: first,
                hour: 11,
            };
            assert_eq!(loaded.parent, parent);
            if lost {
                tracker.lose_hour(first, 11).unwrap();
                assert!(tracker.record_hour(first, 11).is_err());
            } else {
                tracker.record_hour(first, 11).unwrap();
                assert!(tracker.lose_hour(first, 11).is_err());
            }
            assert_eq!(
                tracker.branch_start(loaded.id).unwrap().parent,
                if lost { Parentage::Unknown } else { parent }
            );
            assert_eq!(
                tracker.recorded_hours(first).unwrap().to_vec(),
                if lost { vec![10] } else { vec![10, 11] }
            );
        }
    }

    #[test]
    fn a_queued_hour_on_another_branch_keeps_a_reload_ambiguous_until_it_is_lost() {
        for lost in [false, true] {
            let mut tracker = BranchTracker::new();
            tracker.begin_session(1).unwrap();
            let first = sample(&mut tracker, 9, 10, 1).branch;
            sample(&mut tracker, 10, 11, 1);
            let second = tracker
                .observe(Some(11), 11, Some(2), &[])
                .unwrap()
                .start
                .unwrap();
            let parent = Parentage::Probable {
                branch: first,
                hour: 11,
            };
            assert_eq!(second.parent, parent);
            tracker.admit_hour(second.id, 11).unwrap();
            let third = tracker
                .observe(Some(11), 11, Some(3), &[])
                .unwrap()
                .start
                .unwrap();
            assert_eq!(third.parent, Parentage::Unknown);
            if lost {
                tracker.lose_hour(second.id, 11).unwrap();
            } else {
                tracker.record_hour(second.id, 11).unwrap();
            }
            assert_eq!(
                tracker.branch_start(third.id).unwrap().parent,
                if lost {
                    parent.clone()
                } else {
                    Parentage::Unknown
                }
            );
            assert_eq!(tracker.branch_start(second.id).unwrap().parent, parent);
            tracker.lose_hour(third.id, 11).unwrap();
            assert!(tracker.record_hour(third.id, 11).is_err());
            assert_eq!(
                tracker.branch_start(third.id).unwrap().parent,
                if lost { parent } else { Parentage::Unknown }
            );
        }
    }

    #[test]
    fn unavailable_load_reads_retain_last_known_signal() {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(1).unwrap();
        tracker.observe(Some(0), 1, None, &[]).unwrap();
        assert!(
            tracker
                .observe(Some(1), 2, Some(7), &[])
                .unwrap()
                .start
                .is_none()
        );
        assert!(
            tracker
                .observe(Some(2), 3, None, &[])
                .unwrap()
                .start
                .is_none()
        );
        assert_eq!(
            tracker
                .observe(Some(3), 4, Some(8), &[])
                .unwrap()
                .start
                .unwrap()
                .reason,
            BranchReason::LoadSignal
        );
    }

    #[test]
    fn lost_load_origin_never_uses_a_later_callback_as_parent_evidence() {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(1).unwrap();
        let original = sample(&mut tracker, 99, 100, 1).branch;
        sample(&mut tracker, 100, 101, 1);
        let mut start = tracker
            .observe(None, 102, Some(2), &[])
            .unwrap()
            .start
            .unwrap();
        assert_eq!(start.reason, BranchReason::LoadSignal);
        assert_eq!(start.loaded_hour, None);
        assert_eq!(start.parent, Parentage::Unknown);
        let mut bytes = Vec::new();
        start.encode(&mut bytes).unwrap();
        assert_eq!(bytes.len(), 23);
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(BranchStart::decode(&mut decoder).unwrap(), start);
        decoder.finish().unwrap();
        start.parent = Parentage::Probable {
            branch: original,
            hour: 101,
        };
        assert!(start.validate().is_err());
        bytes[16] = 2;
        assert!(BranchStart::decode(&mut Decoder::new(&bytes)).is_err());
    }

    #[test]
    fn invalid_observations_do_not_mutate_tracking() {
        let mut tracker = BranchTracker::new();
        assert!(tracker.observe(Some(0), 1, Some(1), &[]).is_err());
        tracker.begin_session(1).unwrap();
        let branch = sample(&mut tracker, 9, 10, 1).branch;
        for bad in [
            gap(branch, 12, 11),
            gap(branch + 1, 11, 11),
            gap(branch, 10, 11),
            gap(branch, 11, 12),
            gap(branch, 11, 20),
        ] {
            assert!(tracker.observe(Some(11), 12, Some(1), &[bad]).is_err());
            assert_eq!(tracker.current_branch(), Some(branch));
            assert_eq!(tracker.branches().len(), 1);
        }
        assert!(
            tracker
                .observe(Some(10), 11, Some(1), &[])
                .unwrap()
                .start
                .is_none()
        );
    }

    #[test]
    fn hour_arithmetic_does_not_wrap() {
        assert!(continues(i32::MAX - 1, i32::MAX, &[]).unwrap());
        assert!(!continues(i32::MAX, i32::MIN, &[]).unwrap());
        assert!(continues(i32::MIN, i32::MAX, &[gap(1, i32::MIN + 1, i32::MAX - 1)]).unwrap());
        assert!(!continues(10, 10, &[gap(1, 10, 10)]).unwrap());
        assert!(!continues(10, 9, &[gap(1, 9, 10)]).unwrap());
    }

    #[test]
    fn recovered_history_can_seed_append_without_inventing_missing_hours() {
        let mut tracker = BranchTracker::new();
        let start = BranchStart {
            id: 40,
            session: 2,
            loaded_hour: Some(9),
            first_hour: 10,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        };
        assert!(
            tracker
                .seed_recorded_branch(start.clone(), [9, 10])
                .is_err()
        );
        assert!(
            tracker
                .seed_recorded_branch(start.clone(), [12, 10, 12])
                .is_err()
        );
        tracker
            .seed_recorded_branch(start.clone(), [10, 12])
            .unwrap();
        assert!(tracker.seed_recorded_branch(start, [10, 12]).is_err());
        assert!(tracker.begin_session(2).is_err());
        tracker.begin_session(3).unwrap();
        let restart = tracker
            .observe(Some(11), 12, Some(1), &[])
            .unwrap()
            .start
            .unwrap();
        assert_eq!(restart.id, 41);
        assert_eq!(restart.parent, Parentage::Unknown);
        let overflow = BranchStart {
            id: u64::MAX,
            session: 4,
            loaded_hour: Some(0),
            first_hour: 1,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        };
        let mut full = BranchTracker::new();
        full.seed_recorded_branch(overflow, [1]).unwrap();
        full.begin_session(5).unwrap();
        assert!(full.observe(Some(1), 2, Some(1), &[]).is_err());
        assert_eq!(full.current_branch(), None);
    }

    #[test]
    fn branch_and_gap_wire_values_are_explicit_and_strict() {
        let branch = BranchStart {
            id: 3,
            session: 2,
            loaded_hour: Some(11),
            first_hour: 12,
            parent: Parentage::Probable {
                branch: 1,
                hour: 11,
            },
            reason: BranchReason::LoadSignal,
        };
        let mut bytes = Vec::new();
        branch.encode(&mut bytes).unwrap();
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(BranchStart::decode(&mut decoder).unwrap(), branch);
        decoder.finish().unwrap();
        for length in 0..bytes.len() {
            assert!(BranchStart::decode(&mut Decoder::new(&bytes[..length])).is_err());
        }
        bytes[25] = 2;
        assert!(BranchStart::decode(&mut Decoder::new(&bytes)).is_err());
        let missing = gap(3, 13, 15);
        bytes.clear();
        missing.encode(&mut bytes).unwrap();
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(Gap::decode(&mut decoder).unwrap(), missing);
        decoder.finish().unwrap();
        bytes[12..16].copy_from_slice(&12_i32.to_le_bytes());
        assert!(Gap::decode(&mut Decoder::new(&bytes)).is_err());
    }

    #[test]
    fn paused_commands_establish_a_branch_without_inventing_frames_or_parentage() {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(1).unwrap();
        let commands = tracker.observe_commands(10, Some(5)).unwrap();
        assert_eq!(commands.start.as_ref().unwrap().loaded_hour, None);
        assert!(tracker.recorded_hours(commands.branch).unwrap().is_empty());
        assert!(tracker.record_hour(commands.branch, 10).is_err());
        assert!(
            tracker
                .observe_commands(10, Some(5))
                .unwrap()
                .start
                .is_none()
        );
        let frame = tracker.observe(Some(11), 11, Some(5), &[]).unwrap();
        assert_eq!(frame.branch, commands.branch);
        assert!(frame.start.is_none());
        tracker.record_hour(frame.branch, 11).unwrap();
        tracker.world_changed();
        let replacement = tracker.observe_commands(10, Some(5)).unwrap();
        assert_ne!(replacement.branch, commands.branch);
        assert_eq!(replacement.start.unwrap().parent, Parentage::Unknown);
        assert!(
            tracker
                .recorded_hours(replacement.branch)
                .unwrap()
                .is_empty()
        );
        let current = tracker.last_observation();
        let past = tracker.past_commands(5).unwrap();
        assert_eq!(tracker.last_observation(), current);
        assert_ne!(past, replacement.branch);
        assert!(tracker.record_hour(past, 5).is_err());
    }

    #[test]
    fn a_command_clock_allows_its_first_frame_but_never_a_repeated_frame() {
        for resumed in [false, true] {
            for first_hour in [10, 11] {
                let mut tracker = BranchTracker::new();
                tracker.begin_session(1).unwrap();
                let command = tracker.observe_commands(10, Some(5)).unwrap();
                if resumed {
                    let mut recovered = BranchTracker::new();
                    recovered
                        .seed_recorded_branch(
                            command.start.clone().unwrap(),
                            std::iter::empty::<Hour>(),
                        )
                        .unwrap();
                    recovered
                        .resume_commands(1, command.branch, 10, Some(5))
                        .unwrap();
                    tracker = recovered;
                }
                let frame = tracker
                    .observe(Some(first_hour), first_hour, Some(5), &[])
                    .unwrap();
                assert_eq!(frame.branch, command.branch);
                assert!(frame.start.is_none());
                tracker.record_hour(frame.branch, first_hour).unwrap();
                tracker.observe_commands(first_hour, Some(5)).unwrap();
                let repeated = tracker
                    .observe(Some(first_hour), first_hour, Some(5), &[])
                    .unwrap();
                assert_ne!(repeated.branch, frame.branch);
                assert_eq!(
                    repeated.start.unwrap().reason,
                    BranchReason::HourDiscontinuity
                );
            }
        }
        let mut tracker = BranchTracker::new();
        tracker.begin_session(1).unwrap();
        let failed = tracker.observe(None, 10, Some(5), &[]).unwrap().branch;
        tracker.lose_hour(failed, 10).unwrap();
        tracker.observe_commands(10, Some(5)).unwrap();
        assert_ne!(
            tracker.observe(None, 10, Some(5), &[]).unwrap().branch,
            failed
        );
    }
}
