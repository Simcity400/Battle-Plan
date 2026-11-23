use crate::{
    output_log::Drained,
    table_capture::{grow_capacity, grow_vec},
    table_conversion::unavailable,
};
use engine_allocation::{collect_vec, default_vec, owned_vec};
use engine_native::walk::Outcome;
use replay_data::BranchId;
use replay_frame::{frame::TableInput, frame::TableView};
use std::{collections::HashMap, io, marker::PhantomData, ops::Range};

pub type OutputMap<L> = HashMap<u64, L>;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub entries: usize,
}

collector_primitives::capacity_max!(Capacities { entries });

pub trait Observation {
    type Entry: Copy + Default;
    const ASSEMBLED_BY_SPOOL: &'static str;
    /// # Safety
    /// Only the admitted hour's sole observer boundary (or an isolated test holding the observer's
    /// test lock) may drain the live observer interval once while its storage remains live.
    unsafe fn drain(entries: &mut Vec<Self::Entry>) -> Result<Drained, &'static str>;
}

pub struct Captured<O: Observation, const N: usize = 1> {
    entries: Vec<O::Entry>,
    pub drained: Option<Result<Drained, &'static str>>,
    observation: PhantomData<O>,
}
impl<O: Observation, const N: usize> Captured<O, N> {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { entries: 0 })
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.drained = Some(Err(reason));
    }
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
    /// # Safety
    /// Only the admitted hour's sole observer boundary (or an isolated test holding the observer's
    /// test lock) may drain the live observer interval once while its storage remains live.
    pub unsafe fn drain(&mut self) {
        // SAFETY: Caller guarantees sole observer-boundary ownership while observer storage lives.
        self.drained = Some(unsafe { O::drain(&mut self.entries) });
    }
    pub fn handed_over(&self) -> &[O::Entry] {
        match self.drained {
            Some(Ok(d)) => &self.entries[..d.copied],
            _ => &[],
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_drained_for_test(
        &mut self,
        drained: Result<Drained, &'static str>,
        entries: &[O::Entry],
    ) {
        if self.entries.len() < entries.len() {
            engine_allocation::resize_default(&mut self.entries, entries.len()).unwrap();
        }
        self.entries[..entries.len()].copy_from_slice(entries);
        self.drained = Some(drained);
    }
}

#[derive(Debug)]
pub struct Totals<L> {
    branch: Option<BranchId>,
    pub state: State,
    pub complete: bool,
    pub census: Baseline,
    pub lines: OutputMap<L>,
    publications: u64,
    held: Option<u64>,
    written_through: u64,
    retired_through: u64,
}
impl<L> Default for Totals<L> {
    fn default() -> Self {
        Self {
            branch: None,
            state: State::Withheld("no frame observed yet"),
            complete: false,
            census: Baseline::default(),
            lines: OutputMap::default(),
            publications: 0,
            held: None,
            written_through: 0,
            retired_through: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Receipt(Option<u64>);

impl<L> Totals<L> {
    pub fn observe(
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
            self.held = None;
        }
        progress
    }
    pub fn retire(&mut self, ended: impl Fn(&L) -> Option<u64>) {
        if self.written_through > self.retired_through {
            let through = self.written_through;
            self.lines
                .retain(|_, line| ended(line).is_none_or(|at| at > through));
            self.retired_through = through;
        }
    }
    pub fn prepare_publication(
        &mut self,
        hour: i32,
        handles: impl ExactSizeIterator<Item = u64>,
    ) -> io::Result<u64> {
        if self.census.0.is_none() {
            let mut baseline = collect_vec(handles)?;
            baseline.sort_unstable();
            self.census.0 = Some((hour, baseline));
        }
        self.publications += 1;
        Ok(self.publications)
    }
    pub fn take_held(&mut self) -> Option<u64> {
        self.held.take()
    }
    pub fn published(&mut self, publication: u64) {
        self.held = Some(publication);
    }
    pub fn carry(&mut self, output: &TableView, carried: Option<u64>) {
        if let TableView::Unavailable { reason } = output
            && reason == replay_data::cadence::SKIPPED_REASON
        {
            self.held = carried;
        }
    }
    pub fn receipt(&self, output: &TableInput) -> Receipt {
        Receipt(self.held.filter(|_| holds_publication(output)))
    }
    pub fn written(&mut self, receipt: Receipt) {
        if let Receipt(Some(publication)) = receipt {
            self.written_through = self.written_through.max(publication);
        }
    }
}
#[derive(Debug, Default)]
pub struct Baseline(Option<(i32, Vec<u64>)>);
impl Baseline {
    pub fn observed_since_creation(&self, handle: u64, first_hour: i32) -> bool {
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

pub fn holds_publication(output: &TableInput) -> bool {
    match output {
        TableInput::Sampled(TableView::Complete(_)) | TableInput::SharedComplete(_) => true,
        TableInput::Sampled(TableView::Unavailable { reason }) => {
            reason == replay_data::cadence::SKIPPED_REASON
        }
        _ => false,
    }
}
