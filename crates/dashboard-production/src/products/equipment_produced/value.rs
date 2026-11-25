
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub count: ProducedCount,
    pub refits: bool,
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

fn covered(rows: &[Option<Coverage>], count: ProducedCount, hour: i32) -> Option<ProducedUnknown> {
    rows.iter().flatten().find_map(|row| row.unknown(count, hour))
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
