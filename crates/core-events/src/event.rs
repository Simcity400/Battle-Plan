use crate::{Count, FieldId, Hour, Interval, Key, RowView, SpanView, TableId, Timer, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reason<'a> {
    pub name: &'a str,
    pub detail: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MissingKind<'a> {
    Unknown(Reason<'a>),
    NotRecorded,
    Refused(Reason<'a>),
    Lost(Reason<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Missing<'a> {
    pub hours: Interval,
    pub kind: MissingKind<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellValue<'a> {
    Known(Value<'a>),
    Missing(Missing<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status<'a> {
    Complete,
    Truncated { missing: Count<'a> },
    Unavailable { reason: Reason<'a> },
    NotRecorded,
    ReadInPart { keys: &'a [Key<'a>] },
    Refused { reason: Reason<'a>, hours: Interval },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Discontinuity<'a> {
    Gap {
        hours: Interval,
    },
    Damage {
        hours: Interval,
        reason: Reason<'a>,
    },
    BranchBoundary {
        hours: Interval,
        from: Key<'a>,
        to: Key<'a>,
    },
    SegmentStart {
        hours: Interval,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Ordinary,
    Reread,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldValue<'a> {
    pub field: FieldId,
    pub value: CellValue<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObservationId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelationId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadEvidence<'a> {
    pub manifest_bits: &'a [u64],
    pub timing: SpanView<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MembershipMissing {
    Unknown,
    Refused,
    Lost,
    NotRecorded,
    ReadInPart,
    Truncated,
    Unavailable,
    MissingReference,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MembershipEvidence<'a> {
    pub kind: MembershipMissing,
    pub hours: Interval,
    pub reason: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MembershipCause<'a> {
    Changed,
    Fresh,
    Recovered(MembershipEvidence<'a>),
    Missing(MembershipEvidence<'a>),
    Hidden(MembershipEvidence<'a>),
}

#[derive(Clone, Copy, Debug)]
pub enum Event<'a> {
    Appeared {
        table: TableId,
        key: Key<'a>,
        slot: usize,
        fields: RowView<'a>,
        source: Appearance,
    },
    Gone {
        table: TableId,
        key: Key<'a>,
        slot: usize,
    },
    Changed {
        table: TableId,
        key: Key<'a>,
        field: FieldId,
        slot: usize,
        previous: &'a CellValue<'a>,
        value: &'a Value<'a>,
    },
    Unknown {
        table: TableId,
        key: Key<'a>,
        field: FieldId,
        slot: usize,
        missing: &'a Missing<'a>,
    },
    Status {
        table: TableId,
        status: &'a Status<'a>,
        sample_hour: Option<Hour>,
    },
    Reread {
        table: TableId,
    },
    Acquired {
        table: TableId,
        evidence: ReadEvidence<'a>,
    },
    Supplied {
        table: TableId,
        evidence: ReadEvidence<'a>,
    },
    Observed {
        observation: ObservationId,
    },
    Discontinuity(Discontinuity<'a>),
    Joined {
        cause: MembershipCause<'a>,
        relation: RelationId,
        value: Key<'a>,
        member: Key<'a>,
    },
    Left {
        cause: MembershipCause<'a>,
        relation: RelationId,
        value: Key<'a>,
        member: Key<'a>,
    },
    Tick {
        hour: Hour,
        starts_day: bool,
        due: &'a [Timer<'a>],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventOrder<'a> {
    Discontinuity(Discontinuity<'a>),
    Metadata {
        table: usize,
        kind: u8,
        observation: usize,
    },
    Reread {
        table: TableId,
        row: Option<(usize, Key<'a>)>,
    },
    Gone {
        table: TableId,
        slot: usize,
        key: Key<'a>,
    },
    Appeared {
        table: TableId,
        slot: usize,
        key: Key<'a>,
    },
    Changed {
        table: TableId,
        column: FieldId,
        slot: usize,
        key: Key<'a>,
    },
    Unknown {
        table: TableId,
        column: FieldId,
        slot: usize,
        key: Key<'a>,
    },
    Membership {
        relation: RelationId,
        value: Key<'a>,
        member: Key<'a>,
        joined: bool,
    },
    Tick {
        hour: Hour,
        starts_day: bool,
        due: &'a [Timer<'a>],
    },
}

impl<'a> Event<'a> {
    pub fn order(&self) -> EventOrder<'a> {
        match *self {
            Self::Discontinuity(value) => EventOrder::Discontinuity(value),
            Self::Status { table, .. } => EventOrder::Metadata {
                table: table.index(),
                kind: 0,
                observation: 0,
            },
            Self::Acquired { table, .. } => EventOrder::Metadata {
                table: table.index(),
                kind: 1,
                observation: 0,
            },
            Self::Supplied { table, .. } => EventOrder::Metadata {
                table: table.index(),
                kind: 2,
                observation: 0,
            },
            Self::Observed { observation } => EventOrder::Metadata {
                table: usize::MAX,
                kind: 3,
                observation: observation.0,
            },
            Self::Reread { table } => EventOrder::Reread { table, row: None },
            Self::Appeared {
                table,
                key,
                slot,
                source: Appearance::Reread,
                ..
            } => EventOrder::Reread {
                table,
                row: Some((slot, key)),
            },
            Self::Gone { table, key, slot } => EventOrder::Gone { table, slot, key },
            Self::Appeared {
                table, key, slot, ..
            } => EventOrder::Appeared { table, slot, key },
            Self::Changed {
                table,
                key,
                field,
                slot,
                ..
            } => EventOrder::Changed {
                table,
                column: field,
                slot,
                key,
            },
            Self::Unknown {
                table,
                key,
                field,
                slot,
                ..
            } => EventOrder::Unknown {
                table,
                column: field,
                slot,
                key,
            },
            Self::Joined {
                relation,
                value,
                member,
                ..
            }
            | Self::Left {
                relation,
                value,
                member,
                ..
            } => EventOrder::Membership {
                relation,
                value,
                member,
                joined: matches!(self, Self::Joined { .. }),
            },
            Self::Tick {
                hour,
                starts_day,
                due,
            } => EventOrder::Tick {
                hour,
                starts_day,
                due,
            },
        }
    }
}
