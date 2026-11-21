
use super::{ProducedCount, ProducedUnknown};
use replay_data::rows::ProductionTotalRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Amount {
    Known(i128),
    Unknown(ProducedUnknown),
}

impl Amount {
    pub fn plus(self, other: Self) -> Self {
        match (self, other) {
            (Self::Known(a), Self::Known(b)) => a.checked_add(b).map_or(Self::Unknown(ProducedUnknown::SpendingUnavailable), Self::Known),
            (Self::Unknown(reason), _) | (_, Self::Unknown(reason)) => Self::Unknown(reason),
        }
    }

    pub fn minus(self, other: Self) -> Self {
        match (self, other) {
            (Self::Known(a), Self::Known(b)) => a.checked_sub(b).map_or(Self::Unknown(ProducedUnknown::SpendingUnavailable), Self::Known),
            (Self::Unknown(reason), _) | (_, Self::Unknown(reason)) => Self::Unknown(reason),
        }
    }

    #[cfg(test)]
    pub fn known(self) -> Option<i128> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unknown(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub count: ProducedCount,
    pub refits: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub units: i64,
    pub units_inexact: i64,
    pub spent: Option<i64>,
    pub units_without_refits: i64,
    pub units_without_refits_inexact: i64,
    pub spent_without_refits: Option<i64>,
}

impl Totals {
    pub fn amount(&self, choice: Choice) -> Amount {
        match (choice.count, choice.refits) {
            (ProducedCount::Units, true) => units(self.units, self.units_inexact),
            (ProducedCount::Units, false) => units(self.units_without_refits, self.units_without_refits_inexact),
            (ProducedCount::Ic, true) => spent(self.spent, ProducedUnknown::SpendingUnavailable),
            (ProducedCount::Ic, false) => spent(self.spent_without_refits, ProducedUnknown::SpendingUnavailable),
        }
    }
}

fn units(value: i64, inexact: i64) -> Amount {
    if inexact > 0 { Amount::Unknown(ProducedUnknown::UnitsInexact) } else { Amount::Known(i128::from(value)) }
}

fn spent(value: Option<i64>, reason: ProducedUnknown) -> Amount {
    value.map_or(Amount::Unknown(reason), |value| Amount::Known(i128::from(value)))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DesignRow {
    pub purpose: u8,
    pub units: i64,
    pub inexact_rows: i64,
    pub spent: Option<i64>,
    pub spent_reason: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RocketRow {
    pub units: i64,
    pub units_exact: bool,
    pub spent: i64,
}

impl RocketRow {
    pub fn amount(&self, count: ProducedCount) -> Amount {
        match count {
            ProducedCount::Units if !self.units_exact => Amount::Unknown(ProducedUnknown::RocketInputInexact),
            ProducedCount::Units => Amount::Known(i128::from(self.units)),
            ProducedCount::Ic => Amount::Known(i128::from(self.spent)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub observation: Option<i32>,
    pub units_unknown_from: Option<i32>,
    pub spending_unknown_from: Option<i32>,
    pub reason: u8,
}

impl Coverage {
    fn unknown(&self, count: ProducedCount, hour: i32) -> Option<ProducedUnknown> {
        let from = match count {
            ProducedCount::Units => self.units_unknown_from,
            ProducedCount::Ic => self.spending_unknown_from,
        }?;
        (from <= hour).then(|| reason(self.reason))
    }
}

pub fn reason(code: u8) -> ProducedUnknown {
    match code {
        ProductionTotalRow::NOT_RECORDED => ProducedUnknown::NotRecorded,
        ProductionTotalRow::LOST_OUTPUT => ProducedUnknown::LostOutput,
        ProductionTotalRow::MISSED_PUBLICATION => ProducedUnknown::MissedPublication,
        ProductionTotalRow::ROCKET_INPUT_INEXACT => ProducedUnknown::RocketInputInexact,
        ProductionTotalRow::ROCKET_OWNER_UNREADABLE => ProducedUnknown::RocketOwnerUnreadable,
        _ => ProducedUnknown::SpendingUnavailable,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counted {
    Yes,
    #[cfg(test)]
    No,
    #[cfg(test)]
    NotKnown,
}

#[derive(Clone, Copy, Debug)]
pub struct Cells<'a> {
    pub country: Option<Totals>,
    pub lines: Option<Coverage>,
    pub rocket_run: Option<Coverage>,
    pub rocket_own: Option<Coverage>,
    pub types: Option<TypeCells<'a>>,
    pub rocket_world: bool,
    pub rocket: &'a [(Counted, Option<RocketRow>)],
}

#[derive(Clone, Copy, Debug)]
pub struct TypeCells<'a> {
    pub asked: &'a [Option<Totals>],
    pub untyped: Option<Option<Totals>>,
}

fn covered(rows: &[Option<Coverage>], count: ProducedCount, hour: i32) -> Option<ProducedUnknown> {
    rows.iter().flatten().find_map(|row| row.unknown(count, hour))
}

pub fn value(cells: &Cells<'_>, hour: i32, choice: Choice) -> Amount {
    let Some(country) = cells.country else {
        return Amount::Unknown(ProducedUnknown::NotRecorded);
    };
    let lines = match covered(&[cells.lines], choice.count, hour) {
        Some(why) => Amount::Unknown(why),
        None => match cells.types {
            None => country.amount(choice),
            Some(types) => {
                let mut sum = Amount::Known(0);
                for row in types.asked.iter().flatten() {
                    sum = sum.plus(row.amount(choice));
                }
                match types.untyped {
                    Some(Some(row)) if row.amount(choice) != Amount::Known(0) => Amount::Unknown(ProducedUnknown::TypeNotRecorded),
                    _ => sum,
                }
            }
        },
    };
    lines.plus(rocket(cells, hour, choice.count))
}

pub fn rocket(cells: &Cells<'_>, hour: i32, count: ProducedCount) -> Amount {
    if !cells.rocket_world {
        return Amount::Unknown(ProducedUnknown::RocketTableUnavailable);
    }
    if let Some(why) = covered(&[cells.rocket_own, cells.rocket_run], count, hour) {
        return Amount::Unknown(why);
    }
    let mut sum = Amount::Known(0);
    for (counted, row) in cells.rocket {
        let Some(row) = row else { continue };
        let amount = row.amount(count);
        match counted {
            Counted::Yes => sum = sum.plus(amount),
            #[cfg(test)]
            Counted::No => {}
            #[cfg(test)]
            Counted::NotKnown if amount == Amount::Known(0) => {}
            #[cfg(test)]
            Counted::NotKnown => return Amount::Unknown(ProducedUnknown::TypeNotRecorded),
        }
    }
    sum
}

pub fn design(country: bool, lines: Option<Coverage>, rows: &[DesignRow], hour: i32, choice: Choice) -> Amount {
    if !country {
        return Amount::Unknown(ProducedUnknown::NotRecorded);
    }
    if let Some(why) = covered(&[lines], choice.count, hour) {
        return Amount::Unknown(why);
    }
    let refit = replay_data::rows::OutputPurpose::Refit.raw();
    let mut sum = Amount::Known(0);
    for row in rows.iter().filter(|row| choice.refits || row.purpose != refit) {
        sum = sum.plus(match choice.count {
            ProducedCount::Units => units(row.units, row.inexact_rows),
            ProducedCount::Ic => spent(row.spent, reason(row.spent_reason)),
        });
    }
    sum
}

pub fn rocket_design(world: bool, coverage: [Option<Coverage>; 2], row: Option<RocketRow>, hour: i32, count: ProducedCount) -> Amount {
    if !world {
        return Amount::Unknown(ProducedUnknown::RocketTableUnavailable);
    }
    if let Some(why) = covered(&coverage, count, hour) {
        return Amount::Unknown(why);
    }
    row.map_or(Amount::Known(0), |row| row.amount(count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProducedUnknown as Why;

    const UNITS: Choice = Choice { count: ProducedCount::Units, refits: true };
    const IC: Choice = Choice { count: ProducedCount::Ic, refits: true };

    fn totals(units: i64, spent: Option<i64>) -> Totals {
        Totals { units, units_inexact: 0, spent, units_without_refits: units, units_without_refits_inexact: 0, spent_without_refits: spent }
    }

    fn cells<'a>(types: Option<TypeCells<'a>>, rocket: &'a [(Counted, Option<RocketRow>)]) -> Cells<'a> {
        Cells { country: Some(totals(10, Some(500))), lines: None, rocket_run: None, rocket_own: None, types, rocket_world: true, rocket }
    }

    #[test]
    fn a_type_set_sums_its_rows_and_is_unknown_while_untyped_designs_hold_something() {
        let asked = [Some(totals(4, Some(100))), None, Some(totals(3, None))];
        let typed = TypeCells { asked: &asked, untyped: Some(None) };
        assert_eq!(value(&cells(Some(typed), &[]), 5, UNITS), Amount::Known(7));
        assert_eq!(value(&cells(Some(typed), &[]), 5, IC), Amount::Unknown(Why::SpendingUnavailable));
        let zero = TypeCells { untyped: Some(Some(totals(0, Some(0)))), ..typed };
        assert_eq!(value(&cells(Some(zero), &[]), 5, UNITS), Amount::Known(7));
        let holding = TypeCells { untyped: Some(Some(totals(2, Some(0)))), ..typed };
        assert_eq!(value(&cells(Some(holding), &[]), 5, UNITS), Amount::Unknown(Why::TypeNotRecorded));
        assert_eq!(value(&cells(None, &[]), 5, IC), Amount::Known(500));
        let untyped_asked = TypeCells { asked: &asked[..1], untyped: None };
        assert_eq!(value(&cells(Some(untyped_asked), &[]), 5, UNITS), Amount::Known(4));
    }

    #[test]
    fn inexact_units_refits_coverage_and_a_missing_country_row_are_never_counted() {
        let mut country = totals(10, Some(500));
        country.units_without_refits = 8;
        country.units_inexact = 1;
        let mut held = cells(None, &[]);
        held.country = Some(country);
        assert_eq!(value(&held, 5, UNITS), Amount::Unknown(Why::UnitsInexact));
        assert_eq!(value(&held, 5, Choice { count: ProducedCount::Units, refits: false }), Amount::Known(8));
        held.lines = Some(Coverage { observation: Some(0), units_unknown_from: None, spending_unknown_from: Some(6), reason: ProductionTotalRow::LOST_OUTPUT });
        assert_eq!(value(&held, 5, IC), Amount::Known(500));
        assert_eq!(value(&held, 6, IC), Amount::Unknown(Why::LostOutput));
        assert_eq!(value(&held, 6, Choice { count: ProducedCount::Units, refits: false }), Amount::Known(8));
        held.country = None;
        assert_eq!(value(&held, 5, IC), Amount::Unknown(Why::NotRecorded));
    }

    #[test]
    fn rocket_output_adds_its_counted_designs_and_says_why_it_is_not_known() {
        let row = |units, units_exact, spent| Some(RocketRow { units, units_exact, spent });
        let designs = [(Counted::Yes, row(2, true, 40)), (Counted::No, row(9, true, 900)), (Counted::NotKnown, row(0, true, 0)), (Counted::Yes, None)];
        assert_eq!(value(&cells(None, &designs), 5, IC), Amount::Known(540));
        assert_eq!(value(&cells(None, &designs), 5, UNITS), Amount::Known(12));
        let untyped = [(Counted::Yes, row(2, true, 40)), (Counted::NotKnown, row(1, true, 10))];
        assert_eq!(rocket(&cells(None, &untyped), 5, ProducedCount::Units), Amount::Unknown(Why::TypeNotRecorded));
        let inexact = [(Counted::Yes, row(2, false, 40))];
        assert_eq!(rocket(&cells(None, &inexact), 5, ProducedCount::Units), Amount::Unknown(Why::RocketInputInexact));
        assert_eq!(rocket(&cells(None, &inexact), 5, ProducedCount::Ic), Amount::Known(40));
        let mut unread = cells(None, &designs);
        unread.rocket_world = false;
        assert_eq!(value(&unread, 5, IC), Amount::Unknown(Why::RocketTableUnavailable));
        let mut own = cells(None, &designs);
        own.rocket_own = Some(Coverage { observation: None, units_unknown_from: Some(3), spending_unknown_from: None, reason: ProductionTotalRow::ROCKET_INPUT_INEXACT });
        assert_eq!(rocket(&own, 5, ProducedCount::Units), Amount::Unknown(Why::RocketInputInexact));
        assert_eq!(rocket(&own, 2, ProducedCount::Units), Amount::Known(2));
        assert_eq!(value(&own, 2, UNITS), Amount::Known(12));
    }

    #[test]
    fn a_design_sums_its_purposes_refits_only_when_asked() {
        let refit = replay_data::rows::OutputPurpose::Refit.raw();
        let rows = [
            DesignRow { purpose: 1, units: 5, inexact_rows: 0, spent: Some(100), spent_reason: 0 },
            DesignRow { purpose: refit, units: 2, inexact_rows: 0, spent: Some(30), spent_reason: 0 },
        ];
        assert_eq!(design(true, None, &rows, 5, UNITS), Amount::Known(7));
        assert_eq!(design(true, None, &rows, 5, Choice { count: ProducedCount::Ic, refits: false }), Amount::Known(100));
        assert_eq!(design(true, None, &[], 5, IC), Amount::Known(0));
        assert_eq!(design(false, None, &rows, 5, IC), Amount::Unknown(Why::NotRecorded));
        let lost = [DesignRow { spent: None, spent_reason: ProductionTotalRow::LOST_OUTPUT, ..rows[0] }];
        assert_eq!(design(true, None, &lost, 5, IC), Amount::Unknown(Why::LostOutput));
        let inexact = [DesignRow { inexact_rows: 1, ..rows[0] }];
        assert_eq!(design(true, None, &inexact, 5, UNITS), Amount::Unknown(Why::UnitsInexact));
        assert_eq!(rocket_design(true, [None, None], None, 5, ProducedCount::Ic), Amount::Known(0));
        assert_eq!(rocket_design(false, [None, None], None, 5, ProducedCount::Ic), Amount::Unknown(Why::RocketTableUnavailable));
    }
}
