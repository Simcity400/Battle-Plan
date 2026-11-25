
use super::ProductionSpendingUnknown;
use replay_data::rows::ProductionTotalRow;

pub(crate) type Piece<T> = (i32, i32, T);

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Amount {
    Known(i128),
    Unknown(ProductionSpendingUnknown),
}

impl Amount {
    pub(crate) fn plus(self, other: Self) -> Self {
        match (self, other) {
            (Self::Known(a), Self::Known(b)) => a.checked_add(b).map_or(Self::Unknown(ProductionSpendingUnknown::SpendingUnavailable), Self::Known),
            (Self::Unknown(reason), _) | (_, Self::Unknown(reason)) => Self::Unknown(reason),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Context {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Coverage {
    pub observation: Option<i32>,
    pub unknown_from: Option<i32>,
    pub reason: u8,
}

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

pub(crate) fn either(a: &Option<ProductionSpendingUnknown>, b: &Option<ProductionSpendingUnknown>) -> Option<ProductionSpendingUnknown> {
    a.or(*b)
}

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
        assert_eq!(pieces([(10, 1), (12, 2), (40, 3)], 10, 20, 0), vec![(10, 11, 1), (12, 20, 2)]);
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
        let later = unknown_pieces(&[(0, 9, Some(Coverage { observation: None, unknown_from: Some(30), reason: 0 })), (10, 40, None)]);
        assert_eq!(later, vec![(0, 40, None)]);
    }

    #[test]
    fn reads_date_an_empty_census() {
        assert_eq!(read_at(&[4, 28, 52], 30), Some(28));
        assert_eq!(read_at(&[4, 28, 52], 3), None);
    }
}
