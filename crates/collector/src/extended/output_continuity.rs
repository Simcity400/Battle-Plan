//! Branch continuity and written-publication rules shared by output captures.
use crate::{
    output_log::Drained,
    spooling::unavailable,
    table_capture::{grow_capacity, grow_vec},
};
use engine::{
    allocation::{collect_vec, default_vec, owned_vec},
    walk::Outcome,
};
use replay::{
    BranchId,
    frame::{TableInput, TableView},
};
use std::{collections::HashMap, io, marker::PhantomData, ops::Range};

pub(super) type OutputMap<L> = HashMap<u64, L>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    /// Log entries one drain can take; excess waits for off-thread growth.
    pub entries: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { entries: 4096 }
    }
}

pub trait Observation {
    type Entry: Copy + Default;
    const ASSEMBLED_BY_SPOOL: &'static str;
    fn drain(entries: &mut [Self::Entry]) -> Result<Drained, &'static str>;
}

pub struct Captured<O: Observation, const N: usize = 1> {
    entries: Vec<O::Entry>,
    pub(super) drained: Option<Result<Drained, &'static str>>,
    observation: PhantomData<O>,
}
impl<O: Observation, const N: usize> Captured<O, N> {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            entries: owned_vec(default_vec(capacity.entries)),
            drained: None,
            observation: PhantomData,
        }
    }
    pub fn reset(&mut self) {
        self.drained = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<O::Entry>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        if let Some(Ok(drained)) = self.drained {
            grow_capacity(&mut capacity.entries, drained.pending);
        }
        grow_vec(&mut self.entries, capacity.entries);
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [Outcome::Unavailable {
            reason: O::ASSEMBLED_BY_SPOOL,
        }; N]
    }
    pub fn tables(&self) -> [TableView; N] {
        std::array::from_fn(|_| unavailable(O::ASSEMBLED_BY_SPOOL))
    }
    /// Allocation-free hand-over at the clocked callback.
    pub fn drain(&mut self) {
        self.drained = Some(O::drain(&mut self.entries));
    }
    pub(super) fn handed_over(&self) -> &[O::Entry] {
        match self.drained {
            Some(Ok(d)) => &self.entries[..d.copied],
            _ => &[],
        }
    }
    #[cfg(test)]
    pub(crate) fn set_drained_for_test(
        &mut self,
        drained: Result<Drained, &'static str>,
        entries: &[O::Entry],
    ) {
        self.entries[..entries.len()].copy_from_slice(entries);
        self.drained = Some(drained);
    }
}

/// Common branch accounting; the capture owns the meaning of each line total.
#[derive(Debug)]
pub struct Totals<L> {
    branch: Option<BranchId>,
    pub(super) state: State,
    pub(super) complete: bool,
    pub(super) census: Baseline,
    pub(super) lines: OutputMap<L>,
    publication_written: bool,
}
impl<L> Default for Totals<L> {
    fn default() -> Self {
        Self {
            branch: None,
            state: State::Withheld("no frame observed yet"),
            complete: false,
            census: Baseline::default(),
            lines: OutputMap::default(),
            publication_written: false,
        }
    }
}
impl<L> Totals<L> {
    pub(super) fn observe(
        &mut self,
        branch: BranchId,
        hour: i32,
        drained: Option<Result<Drained, &'static str>>,
        reasons: &Reasons,
    ) -> Progress {
        let progress = observe(
            branch,
            &mut self.branch,
            &mut self.state,
            &mut self.complete,
            hour,
            drained,
            reasons,
        );
        if matches!(progress, Progress::Restart) {
            self.lines.clear();
            self.census = Baseline::default();
            self.publication_written = false;
        }
        progress
    }
    pub(super) fn written(&mut self, held: bool) {
        self.publication_written |= held;
    }
    /// Only a written publication retires ended lines. Seed the baseline once,
    /// from the first complete census; earlier drains do not prove creation.
    pub(super) fn prepare_publication(
        &mut self,
        hour: i32,
        handles: impl ExactSizeIterator<Item = u64>,
        ended: impl Fn(&L) -> bool,
    ) -> io::Result<()> {
        if self.census.0.is_none() {
            let mut baseline = collect_vec(handles)?;
            baseline.sort_unstable();
            self.census.0 = Some((hour, baseline));
        }
        if std::mem::take(&mut self.publication_written) {
            self.lines.retain(|_, line| !ended(line));
        }
        Ok(())
    }
}
#[derive(Debug, Default)]
pub(super) struct Baseline(Option<(i32, Vec<u64>)>);
impl Baseline {
    pub(super) fn observed_since_creation(&self, handle: u64, first_hour: i32) -> bool {
        let (hour, baseline) = self.0.as_ref().expect("publication has a census");
        baseline.binary_search(&handle).is_err() && first_hour > *hour
    }
}

#[derive(Debug)]
pub enum State {
    Observing {
        epoch: u64,
        next: u64,
        since_hour: i32,
    },
    Withheld(&'static str),
}
pub struct Reasons {
    pub first: &'static str,
    pub restarted: &'static str,
    pub lost: &'static str,
    pub absent: &'static str,
}
pub enum Progress {
    Restart,
    Entries(Range<usize>),
    None,
}
pub fn observe(
    branch: BranchId,
    current: &mut Option<BranchId>,
    state: &mut State,
    complete: &mut bool,
    hour: i32,
    drained: Option<Result<Drained, &'static str>>,
    reasons: &Reasons,
) -> Progress {
    if *current != Some(branch) {
        *current = Some(branch);
        *state = match drained {
            Some(Ok(d)) => State::Observing {
                epoch: d.epoch,
                next: d.cursor,
                since_hour: hour,
            },
            Some(Err(reason)) => State::Withheld(reason),
            None => State::Withheld(reasons.first),
        };
        *complete = true;
        return Progress::Restart;
    }
    let State::Observing { epoch, next, .. } = state else {
        return Progress::None;
    };
    let reason = match drained {
        Some(Ok(d)) if d.epoch != *epoch => reasons.restarted,
        Some(Ok(d)) if d.first > *next => reasons.lost,
        Some(Ok(d)) => {
            let skip = (*next - d.first).min(d.copied as u64) as usize;
            *next = (*next).max(d.first + d.copied as u64);
            *complete = *next >= d.cursor;
            return Progress::Entries(skip..d.copied);
        }
        Some(Err(reason)) => reason,
        None => reasons.absent,
    };
    *state = State::Withheld(reason);
    Progress::None
}

pub(crate) fn holds_publication(output: &TableInput) -> bool {
    match output {
        TableInput::Sampled(TableView::Complete(_)) | TableInput::SharedComplete(_) => true,
        TableInput::Sampled(TableView::Unavailable { reason }) => {
            reason == replay::cadence::SKIPPED_REASON
        }
        _ => false,
    }
}
