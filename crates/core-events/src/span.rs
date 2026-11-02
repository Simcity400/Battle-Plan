use crate::{Hour, Reason};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Interval {
    first: Hour,
    last: Hour,
}

impl Interval {
    pub fn new(first: Hour, last: Hour) -> Result<Self, SpanError> {
        if first > last {
            return Err(SpanError::Reversed);
        }
        Ok(Self { first, last })
    }

    pub const fn hour(hour: Hour) -> Self {
        Self {
            first: hour,
            last: hour,
        }
    }

    pub fn first(self) -> Hour {
        self.first
    }
    pub fn last(self) -> Hour {
        self.last
    }
    pub fn hours(self) -> u64 {
        (i64::from(self.last) - i64::from(self.first) + 1) as u64
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        Self::new(self.first.max(other.first), self.last.min(other.last)).ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanError {
    Reversed,
    OutOfOrder,
    Capacity { needed: usize },
    NotQuiet { hours: Interval },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpanView<'a> {
    runs: &'a [Interval],
    hours: u64,
}

impl<'a> SpanView<'a> {
    pub fn new(runs: &'a [Interval]) -> Result<Self, SpanError> {
        if runs.windows(2).any(|pair| pair[0].last >= pair[1].first) {
            return Err(SpanError::OutOfOrder);
        }
        Ok(Self {
            runs,
            hours: runs.iter().map(|run| run.hours()).sum(),
        })
    }

    pub fn hours(self) -> u64 {
        self.hours
    }
    pub fn runs(self) -> &'a [Interval] {
        self.runs
    }
    pub fn is_empty(self) -> bool {
        self.runs.is_empty()
    }

    pub fn intersection(self, gap: Interval) -> impl Iterator<Item = Interval> + 'a {
        let start = self.runs.partition_point(|run| run.last < gap.first);
        self.runs[start..]
            .iter()
            .take_while(move |run| run.first <= gap.last)
            .filter_map(move |run| run.intersection(gap))
    }
}

pub struct Spans<'a> {
    storage: &'a mut [Interval],
    len: usize,
    hours: u64,
}

impl<'a> Spans<'a> {
    pub fn new(storage: &'a mut [Interval]) -> Self {
        Self {
            storage,
            len: 0,
            hours: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.hours = 0;
    }

    pub fn append(&mut self, run: Interval) -> Result<(), SpanError> {
        if let Some(last) = self.storage[..self.len].last_mut() {
            if run.first <= last.last {
                return Err(SpanError::OutOfOrder);
            }
            if i64::from(last.last) + 1 == i64::from(run.first) {
                last.last = run.last;
                self.hours += run.hours();
                return Ok(());
            }
        }
        if self.len == self.storage.len() {
            return Err(SpanError::Capacity {
                needed: self.len + 1,
            });
        }
        self.storage[self.len] = run;
        self.len += 1;
        self.hours += run.hours();
        Ok(())
    }

    pub fn view(&self) -> SpanView<'_> {
        SpanView {
            runs: &self.storage[..self.len],
            hours: self.hours,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageState<'a> {
    Accepted,
    Gap,
    Refused(Reason<'a>),
    Lost(Reason<'a>),
    NotRecorded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coverage<'a> {
    pub hours: Interval,
    pub state: CoverageState<'a>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuietView(Option<Interval>);

impl QuietView {
    pub fn interval(self) -> Option<Interval> {
        self.0
    }
    pub fn hours(self) -> u64 {
        self.0.map_or(0, Interval::hours)
    }
}

#[derive(Default)]
pub struct QuietSpans {
    interval: Option<Interval>,
    interrupted: Option<Interval>,
}

impl QuietSpans {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn append(&mut self, coverage: Coverage<'_>) -> Result<(), SpanError> {
        if let Some(hours) = self.interrupted {
            return Err(SpanError::NotQuiet { hours });
        }
        if coverage.state != CoverageState::Accepted {
            self.interrupted = Some(coverage.hours);
            return Err(SpanError::NotQuiet {
                hours: coverage.hours,
            });
        }
        if let Some(last) = self.interval {
            if coverage.hours.first <= last.last {
                return Err(SpanError::OutOfOrder);
            }
            if i64::from(last.last) + 1 != i64::from(coverage.hours.first) {
                let hours = Interval {
                    first: last.last + 1,
                    last: coverage.hours.first - 1,
                };
                self.interrupted = Some(hours);
                return Err(SpanError::NotQuiet { hours });
            }
            self.interval = Some(Interval {
                first: last.first,
                last: coverage.hours.last,
            });
        } else {
            self.interval = Some(coverage.hours);
        }
        Ok(())
    }

    pub fn view(&self) -> QuietView {
        QuietView(self.interval)
    }
}
