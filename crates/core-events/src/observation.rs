use crate::{Hour, Interval, ObservationId, Reason, SpanError, SpanView, Spans};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationUnknown<'a> {
    Gap(Reason<'a>),
    Damage(Reason<'a>),
    Refused(Reason<'a>),
    Unavailable(Reason<'a>),
    NotRecorded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Truth<'a> {
    Holds,
    Fails,
    Unknown(ObservationUnknown<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownRun<'a> {
    pub hours: Interval,
    pub reason: ObservationUnknown<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationRun<'a> {
    pub hours: Interval,
    pub truth: Truth<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationSpans<'a> {
    pub observation: ObservationId,
    hours: Interval,
    holds: SpanView<'a>,
    unknown: &'a [UnknownRun<'a>],
    holds_hours: u64,
    unknown_hours: u64,
}

impl<'a> ObservationSpans<'a> {
    pub fn hours(self) -> Interval {
        self.hours
    }
    pub fn new(
        observation: ObservationId,
        hours: Interval,
        holds: SpanView<'a>,
        unknown: &'a [UnknownRun<'a>],
    ) -> Result<Self, SpanError> {
        if unknown
            .windows(2)
            .any(|pair| pair[0].hours.last() >= pair[1].hours.first())
            || unknown
                .iter()
                .any(|run| holds.intersection(run.hours).next().is_some())
        {
            return Err(SpanError::OutOfOrder);
        }
        Ok(Self::clipped(observation, hours, holds, unknown))
    }
    fn clipped(
        observation: ObservationId,
        hours: Interval,
        holds: SpanView<'a>,
        unknown: &'a [UnknownRun<'a>],
    ) -> Self {
        let mut view = Self {
            observation,
            hours,
            holds,
            unknown,
            holds_hours: 0,
            unknown_hours: 0,
        };
        view.holds_hours = view.holds().map(Interval::hours).sum();
        view.unknown_hours = view.unknown().map(|run| run.hours.hours()).sum();
        view
    }
    pub fn holds_hours(self) -> u64 {
        self.holds_hours
    }
    pub fn unknown_hours(self) -> u64 {
        self.unknown_hours
    }
    pub fn false_hours(self) -> u64 {
        self.hours.hours() - self.holds_hours - self.unknown_hours
    }
    pub fn holds(self) -> impl Iterator<Item = Interval> + 'a {
        self.holds.intersection(self.hours)
    }
    pub fn unknown(self) -> impl Iterator<Item = UnknownRun<'a>> + 'a {
        let start = self
            .unknown
            .partition_point(|run| run.hours.last() < self.hours.first());
        self.unknown[start..]
            .iter()
            .take_while(move |run| run.hours.first() <= self.hours.last())
            .map(move |run| UnknownRun {
                hours: run.hours.intersection(self.hours).unwrap(),
                reason: run.reason,
            })
    }
}

pub trait ObservationStream {
    fn known_hours(&self) -> Option<Interval>;
    fn metadata_watermark(&self) -> Option<Hour> {
        self.known_hours().map(Interval::last)
    }
    fn visit(
        &self,
        hours: Interval,
        visitor: &mut dyn FnMut(ObservationRun<'_>) -> io::Result<()>,
    ) -> io::Result<()>;
    fn truth_at(&self, hour: Hour) -> io::Result<Option<Truth<'_>>>;
}

pub struct ObservationRuns<'a> {
    first: Hour,
    watermark: Option<Hour>,
    spans: Spans<'a>,
    unknown: &'a mut [UnknownRun<'a>],
    unknown_len: usize,
}

impl<'a> ObservationRuns<'a> {
    pub fn new(
        first: Hour,
        storage: &'a mut [Interval],
        unknown: &'a mut [UnknownRun<'a>],
    ) -> Self {
        Self {
            first,
            watermark: None,
            spans: Spans::new(storage),
            unknown,
            unknown_len: 0,
        }
    }
    pub fn append(&mut self, hours: Interval, truth: Truth<'a>) -> Result<(), SpanError> {
        let next = self
            .watermark
            .map_or(i64::from(self.first), |hour| i64::from(hour) + 1);
        if i64::from(hours.first()) != next {
            return Err(SpanError::OutOfOrder);
        }
        match truth {
            Truth::Holds => self.spans.append(hours)?,
            Truth::Fails => {}
            Truth::Unknown(reason) => {
                let last = self.unknown[..self.unknown_len].last_mut();
                if let Some(last) = last.filter(|last| {
                    last.reason == reason
                        && i64::from(last.hours.last()) + 1 == i64::from(hours.first())
                }) {
                    last.hours = Interval::new(last.hours.first(), hours.last())?;
                } else {
                    if self.unknown_len == self.unknown.len() {
                        return Err(SpanError::Capacity {
                            needed: self.unknown_len + 1,
                        });
                    }
                    self.unknown[self.unknown_len] = UnknownRun { hours, reason };
                    self.unknown_len += 1;
                }
            }
        }
        self.watermark = Some(hours.last());
        Ok(())
    }
    pub fn delivery(
        &self,
        observation: ObservationId,
        hours: Interval,
    ) -> io::Result<ObservationSpans<'_>> {
        self.require_known(hours)?;
        Ok(ObservationSpans::clipped(
            observation,
            hours,
            self.spans.view(),
            &self.unknown[..self.unknown_len],
        ))
    }
    fn require_known(&self, hours: Interval) -> io::Result<()> {
        if self
            .known_hours()
            .is_some_and(|known| known.intersection(hours) == Some(hours))
        {
            Ok(())
        } else {
            Err(io::ErrorKind::UnexpectedEof.into())
        }
    }
}

impl ObservationStream for ObservationRuns<'_> {
    fn known_hours(&self) -> Option<Interval> {
        self.watermark
            .map(|last| Interval::new(self.first, last).unwrap())
    }
    fn visit(
        &self,
        hours: Interval,
        visitor: &mut dyn FnMut(ObservationRun<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        let delivery = self.delivery(ObservationId(0), hours)?;
        let mut holds = delivery.holds().peekable();
        let mut unknown = delivery.unknown().peekable();
        while holds.peek().is_some() || unknown.peek().is_some() {
            let run = if unknown
                .peek()
                .is_none_or(|u| holds.peek().is_some_and(|h| h.first() < u.hours.first()))
            {
                ObservationRun {
                    hours: holds.next().unwrap(),
                    truth: Truth::Holds,
                }
            } else {
                let run = unknown.next().unwrap();
                ObservationRun {
                    hours: run.hours,
                    truth: Truth::Unknown(run.reason),
                }
            };
            visitor(run)?;
        }
        Ok(())
    }
    fn truth_at(&self, hour: Hour) -> io::Result<Option<Truth<'_>>> {
        if self.require_known(Interval::hour(hour)).is_err() {
            return Ok(None);
        }
        let unknown = &self.unknown[..self.unknown_len];
        let slot = unknown.partition_point(|run| run.hours.last() < hour);
        if let Some(run) = unknown.get(slot).filter(|run| run.hours.first() <= hour) {
            return Ok(Some(Truth::Unknown(run.reason)));
        }
        Ok(Some(
            if self
                .spans
                .view()
                .intersection(Interval::hour(hour))
                .next()
                .is_some()
            {
                Truth::Holds
            } else {
                Truth::Fails
            },
        ))
    }
}
