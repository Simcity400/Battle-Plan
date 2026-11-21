use crate::{Coverage, Declaration, DeclarationId, Hour, Interval, Key, Missing, Subscription};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProductFieldId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndexId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Number {
    Signed(i64),
    Unsigned(u64),
    Fixed { units: i64, scale: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductValue<'a> {
    Null,
    Number(Number),
    Bool(bool),
    Key(Key<'a>),
    Bytes(&'a [u8]),
    Text(&'a str),
    List(&'a [ProductCell<'a>]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductCell<'a> {
    Known(ProductValue<'a>),
    Missing(Missing<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inherited<'a> {
    pub key: Key<'a>,
    pub field: ProductFieldId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryValue<'a, T> {
    Known(T),
    Missing(Missing<'a>),
    Inherited(Inherited<'a>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Provenance<'a> {
    pub supplying_read: Option<BoundaryValue<'a, Hour>>,
    pub holder: Option<BoundaryValue<'a, Key<'a>>>,
    pub identity_first: Option<BoundaryValue<'a, Hour>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate<'a> {
    Known(Number),
    Missing(Missing<'a>),
    Inherited(Inherited<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationValue<'a> {
    Value(ProductCell<'a>),
    End,
    Inherited(Inherited<'a>),
    Rate(Rate<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationChange<'a> {
    Record(PublicationValue<'a>),
    Correction {
        valid_hours: Interval,
        replacement: PublicationValue<'a>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicationRecord<'a> {
    pub hour: Hour,
    pub key: Key<'a>,
    pub field: ProductFieldId,
    pub change: PublicationChange<'a>,
    pub provenance: Provenance<'a>,
    pub breaks: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependencyTarget {
    Publication,
    OutgoingSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowDependency<'a> {
    pub hours: Interval,
    pub key: Key<'a>,
    pub field: ProductFieldId,
    pub target: DependencyTarget,
}

pub trait WindowDependencies {
    fn hours(
        &self,
        key: Key<'_>,
        field: ProductFieldId,
        target: DependencyTarget,
    ) -> io::Result<Option<Interval>>;
    fn contains(
        &self,
        key: Key<'_>,
        field: ProductFieldId,
        target: DependencyTarget,
    ) -> io::Result<bool> {
        Ok(self.hours(key, field, target)?.is_some())
    }
    fn visit(
        &self,
        visitor: &mut dyn FnMut(WindowDependency<'_>) -> io::Result<()>,
    ) -> io::Result<()>;
}

#[derive(Clone, Copy)]
pub struct SliceWindowDependencies<'a>(&'a [WindowDependency<'a>]);

impl<'a> SliceWindowDependencies<'a> {
    pub fn new(dependencies: &'a [WindowDependency<'a>]) -> io::Result<Self> {
        if dependencies.windows(2).any(|pair| {
            (pair[0].key, pair[0].field, pair[0].target)
                >= (pair[1].key, pair[1].field, pair[1].target)
        }) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self(dependencies))
    }
}

impl WindowDependencies for SliceWindowDependencies<'_> {
    fn hours(
        &self,
        key: Key<'_>,
        field: ProductFieldId,
        target: DependencyTarget,
    ) -> io::Result<Option<Interval>> {
        Ok(self
            .0
            .binary_search_by(|dependency| {
                (dependency.key, dependency.field, dependency.target).cmp(&(key, field, target))
            })
            .ok()
            .map(|index| self.0[index].hours))
    }
    fn visit(
        &self,
        visitor: &mut dyn FnMut(WindowDependency<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        for &dependency in self.0 {
            visitor(dependency)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowBound {
    PrefixHours(u64),
    UntilClosed,
    Segment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEnd {
    Prefix,
    Closed,
    Segment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowExtent {
    pub hours: Interval,
    pub end: WindowEnd,
}

pub trait WindowClosures {
    fn closed_at(&self, rule: DeclarationId, key: Key<'_>) -> io::Result<Option<Hour>>;
}

impl WindowBound {
    pub fn extent(self, segment: Interval, closed_at: Option<Hour>) -> io::Result<WindowExtent> {
        let count = match self {
            Self::PrefixHours(0 | u64::MAX) => return Err(io::ErrorKind::InvalidInput.into()),
            Self::PrefixHours(limit) => limit.min(segment.hours()),
            Self::UntilClosed | Self::Segment => segment.hours(),
        };
        let last = (i64::from(segment.first()) + count as i64 - 1) as Hour;
        if closed_at.is_some_and(|hour| hour < segment.first() || hour > last) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(WindowExtent {
            hours: Interval::new(segment.first(), closed_at.unwrap_or(last))
                .map_err(|_| io::ErrorKind::InvalidInput)?,
            end: if closed_at.unwrap_or(last) == segment.last() {
                WindowEnd::Segment
            } else if closed_at.is_some() {
                WindowEnd::Closed
            } else {
                WindowEnd::Prefix
            },
        })
    }
}

pub struct ProvisionalWindow<'a> {
    pub key: Key<'a>,
    pub rule: DeclarationId,
    pub hours: Interval,
    pub dependencies: &'a dyn WindowDependencies,
}

impl ProvisionalWindow<'_> {
    pub fn validate(
        &self,
        segment: Interval,
        subscription: &Subscription,
        closures: &dyn WindowClosures,
    ) -> io::Result<()> {
        let Some(Declaration::Provisional(rule)) = subscription.declarations().get(self.rule.0)
        else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        let retained = rule
            .bound
            .extent(segment, closures.closed_at(self.rule, self.key)?)?
            .hours;
        if self.hours.intersection(retained) != Some(self.hours)
            || self.hours.first() != segment.first()
            || !rule
                .inputs
                .iter()
                .any(|input| input.keys.contains(self.key))
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.dependencies.visit(&mut |dependency| {
            if dependency.hours.intersection(self.hours) != Some(dependency.hours) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            if !rule.outputs.iter().any(|output| {
                output.target == dependency.target
                    && output.keys.contains(dependency.key)
                    && output.fields.contains(&dependency.field)
            }) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            Ok(())
        })
    }
}

pub trait Publication {
    fn publish(&mut self, record: PublicationRecord<'_>) -> io::Result<()>;
    fn coverage(&mut self, coverage: Coverage<'_>) -> io::Result<()>;
    fn provisional(&mut self, window: ProvisionalWindow<'_>) -> io::Result<()>;
}
