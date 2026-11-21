use crate::{
    CallContext, Event, Hour, Inherited, Interval, Key, ObservationSpans, ProductCell,
    ProductFieldId, Publication, QuietView, ReadRule, ReplayPublication, ReplayScope, SpanView,
    Subscription, TableId,
};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadSpans<'a> {
    pub table: TableId,
    pub rule: ReadRule,
    pub spans: SpanView<'a>,
}

#[derive(Clone, Copy)]
pub struct At<'a> {
    pub hour: Hour,
    pub since: QuietView,
    pub reads: &'a [ReadSpans<'a>],
    pub observations: &'a [ObservationSpans<'a>],
    pub starts: bool,
    pub opens: bool,
    pub events: &'a [Event<'a>],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending<'a> {
    Whole {
        hours: Interval,
    },
    Refused {
        hours: Interval,
        reason: crate::Reason<'a>,
    },
    Lost {
        hours: Interval,
        reason: crate::Reason<'a>,
    },
}

#[derive(Clone, Copy)]
pub struct End<'a> {
    pub at: At<'a>,
    pub ending: Ending<'a>,
    pub delivered_watermark: Option<Hour>,
}

pub trait BoundarySummary {
    fn value(&self, key: Key<'_>, field: ProductFieldId) -> io::Result<ProductCell<'_>>;
    fn bytes(&self) -> u64;
    fn resolve(&self, reference: Inherited<'_>) -> io::Result<ProductCell<'_>> {
        self.value(reference.key, reference.field)
    }
}

pub struct ReconciledSummary<'a> {
    pub dependencies: &'a dyn crate::WindowDependencies,
    pub replayed: &'a dyn BoundarySummary,
    pub transfer: &'a dyn BoundarySummary,
}

impl BoundarySummary for ReconciledSummary<'_> {
    fn value(&self, key: Key<'_>, field: ProductFieldId) -> io::Result<ProductCell<'_>> {
        if self
            .dependencies
            .contains(key, field, crate::DependencyTarget::OutgoingSummary)?
        {
            self.replayed.value(key, field)
        } else {
            self.transfer.value(key, field)
        }
    }
    fn bytes(&self) -> u64 {
        size_of::<Self>() as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummaryKind {
    Transfer,
    Resolved,
}

pub enum Boundary<'a> {
    ClassA,
    ClassB {
        summary: &'a dyn BoundarySummary,
        kind: SummaryKind,
    },
    ClassC,
}

pub enum SummaryInput<'a> {
    Provisional(Option<&'a dyn BoundarySummary>),
    Resolved(&'a dyn BoundarySummary),
}

pub enum StartingBoundary<'a> {
    ClassA,
    ClassB(SummaryInput<'a>),
    ClassC,
    Replay {
        summary: &'a dyn BoundarySummary,
        scope: ReplayScope<'a>,
    },
}

pub struct Start<'a> {
    pub hours: Interval,
    pub opens: bool,
    pub boundary: StartingBoundary<'a>,
}

pub struct Reconciliation<'a> {
    pub scope: ReplayScope<'a>,
    pub incoming: &'a dyn BoundarySummary,
    pub expected_at_close: &'a dyn BoundarySummary,
}

pub trait WindowReplay {
    fn replay<'a>(
        &mut self,
        family: &'a mut dyn Family,
        at: &Reconciliation<'_>,
        out: &mut ReplayPublication<'_>,
    ) -> io::Result<Boundary<'a>>;
}

pub trait Family {
    fn subscription(&self) -> Subscription;
    fn begin(
        &mut self,
        at: &Start<'_>,
        context: &mut dyn CallContext,
        out: &mut dyn Publication,
    ) -> io::Result<()>;
    fn hour(
        &mut self,
        at: &At<'_>,
        context: &mut dyn CallContext,
        out: &mut dyn Publication,
    ) -> io::Result<()>;
    fn end(
        &mut self,
        at: &End<'_>,
        context: &mut dyn CallContext,
        out: &mut dyn Publication,
    ) -> io::Result<Boundary<'_>>;
    fn resolve_boundary(
        &mut self,
        incoming: &dyn BoundarySummary,
        transfer: &dyn BoundarySummary,
    ) -> io::Result<Boundary<'_>> {
        let _ = (incoming, transfer);
        Err(io::ErrorKind::Unsupported.into())
    }
}
