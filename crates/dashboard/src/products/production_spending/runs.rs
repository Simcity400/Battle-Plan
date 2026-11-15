//! The arithmetic of one producer's spending runs: a key's history steps as pieces covering a span,
//! pieces of two sources joined hour by hour, and the reasons a value is not known. No recording is
//! read here.

use super::ProductionSpendingUnknown;
use replay_data::rows::ProductionTotalRow;

/// A value from `first` through `last`, both inclusive.
pub(crate) type Piece<T> = (i32, i32, T);

/// `steps` (each a value from its hour until the next step) as pieces covering `first..=last`,
/// `absent` before the first step; equal neighbours are one piece.
pub(crate) fn pieces<T: Clone + PartialEq>(steps: impl IntoIterator<Item = (i32, T)>, first: i32, last: i32, absent: T) -> Vec<Piece<T>> {
    let mut out: Vec<Piece<T>> = vec![(first, last, absent)];
    for (hour, value) in steps {
        if hour > last {
            break;
        }
        let hour = hour.max(first);
        let Some(open) = out.last_mut() else { continue };
        if open.2 == value {
            continue;
        }
        if open.0 == hour {
            open.2 = value;
            let merged = out.len() >= 2 && out[out.len() - 2].2 == out[out.len() - 1].2;
            if merged {
                let closed = out.pop().expect("two pieces");
                if let Some(previous) = out.last_mut() {
                    previous.1 = closed.1;
                }
            }
            continue;
        }
        open.1 = hour - 1;
        out.push((hour, last, value));
    }
    out
}

/// `a` and `b`, two piece lists covering the same span, joined by `join` at every hour; equal
/// neighbours are one piece.
pub(crate) fn zip<A, B, C: PartialEq>(a: &[Piece<A>], b: &[Piece<B>], mut join: impl FnMut(&A, &B) -> C) -> Vec<Piece<C>> {
    let mut out: Vec<Piece<C>> = Vec::with_capacity(a.len().max(b.len()));
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let first = a[i].0.max(b[j].0);
        let last = a[i].1.min(b[j].1);
        if first <= last {
            let value = join(&a[i].2, &b[j].2);
            match out.last_mut() {
                Some(open) if open.2 == value && open.1.checked_add(1) == Some(first) => open.1 = last,
                _ => out.push((first, last, value)),
            }
        }
        if a[i].1 <= b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// What a spending total holds at an hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Amount {
    Known(i128),
    Unknown(ProductionSpendingUnknown),
}

impl Amount {
    /// The sum of two amounts: unknown where either is, with the first one's reason.
    pub(crate) fn plus(self, other: Self) -> Self {
        match (self, other) {
            (Self::Known(a), Self::Known(b)) => a.checked_add(b).map_or(Self::Unknown(ProductionSpendingUnknown::SpendingUnavailable), Self::Known),
            (Self::Unknown(reason), _) | (_, Self::Unknown(reason)) => Self::Unknown(reason),
        }
    }
}

/// Whether the producer's own rows can be read at an hour: its country row shown, and nothing its
/// source's coverage says is unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Context {
    /// The country totals row is not shown: the writer did not run, or the table was not read.
    NotShown,
    Unknown(ProductionSpendingUnknown),
    Known,
}

impl Context {
    pub(crate) fn of(shown: bool, coverage: Option<ProductionSpendingUnknown>) -> Self {
        match (shown, coverage) {
            (false, _) => Self::NotShown,
            (true, Some(reason)) => Self::Unknown(reason),
            (true, None) => Self::Known,
        }
    }

    /// A row's amount under this context: no row is zero, a cell not recorded is unknown for `reason`.
    pub(crate) fn amount(self, row: Option<Option<i64>>, reason: ProductionSpendingUnknown) -> Amount {
        match self {
            Self::NotShown => Amount::Unknown(ProductionSpendingUnknown::NotRecorded),
            Self::Unknown(why) => Amount::Unknown(why),
            Self::Known => match row {
                None => Amount::Known(0),
                Some(Some(value)) => Amount::Known(i128::from(value)),
                Some(None) => Amount::Unknown(reason),
            },
        }
    }
}

/// A [`ProductionTotalRow`] or coverage reason code as the reply names it.
pub(crate) fn reason(code: u8) -> ProductionSpendingUnknown {
    match code {
        ProductionTotalRow::NOT_RECORDED => ProductionSpendingUnknown::NotRecorded,
        ProductionTotalRow::LOST_OUTPUT => ProductionSpendingUnknown::LostOutput,
        ProductionTotalRow::MISSED_PUBLICATION => ProductionSpendingUnknown::MissedPublication,
        ProductionTotalRow::ROCKET_INPUT_INEXACT => ProductionSpendingUnknown::RocketInputInexact,
        ProductionTotalRow::ROCKET_OWNER_UNREADABLE => ProductionSpendingUnknown::RocketOwnerUnreadable,
        _ => ProductionSpendingUnknown::SpendingUnavailable,
    }
}

/// A coverage row's cells: where its observation began, the first hour its spending is unknown
/// from, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Coverage {
    pub observation: Option<i32>,
    pub unknown_from: Option<i32>,
    pub reason: u8,
}

/// Pieces of a coverage row (none where it is not shown) as the reason spending is unknown at each
/// hour: from its `unknown_from` hour on, while the row holds it.
pub(crate) fn unknown_pieces(rows: &[Piece<Option<Coverage>>]) -> Vec<Piece<Option<ProductionSpendingUnknown>>> {
    let mut out: Vec<Piece<Option<ProductionSpendingUnknown>>> = Vec::new();
    let mut push = |first: i32, last: i32, value: Option<ProductionSpendingUnknown>| match out.last_mut() {
        Some(open) if open.2 == value && open.1.checked_add(1) == Some(first) => open.1 = last,
        _ => out.push((first, last, value)),
    };
    for &(first, last, row) in rows {
        match row.and_then(|row| Some((row.unknown_from?, row.reason))) {
            Some((from, code)) if from <= last => {
                if from > first {
                    push(first, from - 1, None);
                }
                push(from.max(first), last, Some(reason(code)));
            }
            _ => push(first, last, None),
        }
    }
    out
}

/// Two reasons a source is unknown joined: the first's where it has one.
pub(crate) fn either(a: &Option<ProductionSpendingUnknown>, b: &Option<ProductionSpendingUnknown>) -> Option<ProductionSpendingUnknown> {
    a.or(*b)
}

/// The union of `spans` (each sorted or not) as sorted, disjoint pieces of `true` over
/// `first..=last`, `false` elsewhere.
pub(crate) fn union(mut spans: Vec<(i32, i32)>, first: i32, last: i32) -> Vec<Piece<bool>> {
    spans.sort_unstable();
    let mut held: Vec<(i32, i32)> = Vec::new();
    for (a, b) in spans {
        match held.last_mut() {
            Some(open) if a <= open.1.saturating_add(1) => open.1 = open.1.max(b),
            _ => held.push((a, b)),
        }
    }
    let mut out = Vec::new();
    let mut at = first;
    for (a, b) in held {
        let (a, b) = (a.max(first), b.min(last));
        if a > b {
            continue;
        }
        if a > at {
            out.push((at, a - 1, false));
        }
        out.push((a, b, true));
        at = b + 1;
    }
    if at <= last {
        out.push((at, last, false));
    }
    out
}

/// The latest of the sorted `reads` at or before `hour`.
pub(crate) fn read_at(reads: &[i32], hour: i32) -> Option<i32> {
    let at = reads.partition_point(|&read| read <= hour);
    at.checked_sub(1).map(|at| reads[at])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProductionSpendingUnknown as Why;

    #[test]
    fn steps_become_pieces_over_the_span_with_the_absent_value_before_the_first() {
        assert_eq!(pieces([(15, Some(3)), (18, Some(3)), (20, None)], 10, 30, None), vec![(10, 14, None), (15, 19, Some(3)), (20, 30, None)]);
        // A step at the span's first hour replaces the absent value; steps past the span are left out.
        assert_eq!(pieces([(10, 1), (12, 2), (40, 3)], 10, 20, 0), vec![(10, 11, 1), (12, 20, 2)]);
        // A step back to the value before joins the pieces.
        assert_eq!(pieces([(10, 1), (10, 0)], 10, 12, 0), vec![(10, 12, 0)]);
    }

    #[test]
    fn a_row_is_zero_where_absent_unknown_where_its_country_row_or_coverage_says_so() {
        let country = vec![(0, 9, true), (10, 14, false), (15, 29, true)];
        let coverage = unknown_pieces(&[(0, 29, Some(Coverage { observation: Some(0), unknown_from: Some(25), reason: ProductionTotalRow::MISSED_PUBLICATION }))]);
        assert_eq!(coverage, vec![(0, 24, None), (25, 29, Some(Why::MissedPublication))]);
        let context = zip(&country, &coverage, |&shown, why| Context::of(shown, *why));
        let row = vec![(0, 3, None), (4, 7, Some(Some(500))), (8, 20, Some(None)), (21, 29, Some(Some(900)))];
        let amounts = zip(&context, &row, |context, cell| context.amount(*cell, reason(ProductionTotalRow::LOST_OUTPUT)));
        assert_eq!(amounts, vec![
            (0, 3, Amount::Known(0)),
            (4, 7, Amount::Known(500)),
            (8, 9, Amount::Unknown(Why::LostOutput)),
            (10, 14, Amount::Unknown(Why::NotRecorded)),
            (15, 20, Amount::Unknown(Why::LostOutput)),
            (21, 24, Amount::Known(900)),
            (25, 29, Amount::Unknown(Why::MissedPublication)),
        ]);
    }

    #[test]
    fn sums_are_unknown_where_a_part_is_and_coverage_before_its_hour_is_known() {
        assert_eq!(Amount::Known(2).plus(Amount::Known(3)), Amount::Known(5));
        assert_eq!(Amount::Known(2).plus(Amount::Unknown(Why::RocketTableUnavailable)), Amount::Unknown(Why::RocketTableUnavailable));
        assert_eq!(Amount::Unknown(Why::NotRecorded).plus(Amount::Unknown(Why::LostOutput)), Amount::Unknown(Why::NotRecorded));
        // Coverage holding a later hour than its piece's end is known throughout the piece.
        let later = unknown_pieces(&[(0, 9, Some(Coverage { observation: None, unknown_from: Some(30), reason: 0 })), (10, 40, None)]);
        assert_eq!(later, vec![(0, 40, None)]);
    }

    #[test]
    fn presence_is_the_union_of_rows_and_reads_date_an_empty_census() {
        assert_eq!(union(vec![(12, 15), (3, 5), (6, 8), (30, 50)], 0, 20), vec![(0, 2, false), (3, 8, true), (9, 11, false), (12, 15, true), (16, 20, false)]);
        assert_eq!(union(Vec::new(), 0, 4), vec![(0, 4, false)]);
        assert_eq!(read_at(&[4, 28, 52], 30), Some(28));
        assert_eq!(read_at(&[4, 28, 52], 3), None);
    }
}
