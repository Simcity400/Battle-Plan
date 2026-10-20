//! Where a country's dockyard IC goes: naval-domain factory IC applied to new
//! ships and to ship refits, each per target design, and the dockyard output
//! spent on repairs.
//!
//! New ships and refits are Production spending's exact applied input, split
//! by purpose ([`Purposes::Split`]) and kept to naval designs and designs whose
//! type content cannot resolve (which could be naval). Refit input belongs to
//! the target design. Input from a line no census listed has no recorded
//! purpose: it counts as new, kept apart as dated `unproven` runs.
//!
//! Repairs are the dockyards a country uses for repairs times the output one
//! of its dockyards applies to naval production at that hour, over the hours
//! they were used ([`super::dockyard_repairs`]): an amount of dockyard output,
//! not an input the game records. The game assigns repair dockyards to naval
//! bases, not to ships, so repairs are not attributed to a design.

use super::{
    Product, equipment::production::ProductionSpendingUnknown,
    production_spending::ProductionSpendingReply, validate_tag,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod prepared;

pub struct DockyardSpending;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DockyardSpendingParams {
    pub branch_id: String,
    pub tag: String,
    pub view: DockyardView,
}

/// What a reply answers: amounts through its dates, or one column per month.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum DockyardView {
    Circle,
    Months,
}

/// An exact fixed5 IC amount, or why it is not known.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DockyardAmount {
    /// Decimal string of the fixed5 amount; null is unknown, never a partial sum.
    pub total_raw: Option<String>,
    pub reason: Option<ProductionSpendingUnknown>,
}

impl DockyardAmount {
    fn of(value: Result<i128, ProductionSpendingUnknown>) -> Self {
        match value {
            Ok(raw) => Self {
                total_raw: Some(raw.to_string()),
                reason: None,
            },
            Err(reason) => Self {
                total_raw: None,
                reason: Some(reason),
            },
        }
    }
}

/// Dockyard output spent on repairs through the reply's hour and its end.
#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DockyardRepairs {
    /// Repair amounts, here and in the months, are exact numerators over
    /// `denominator`: fixed5 daily dockyard output times hours, over the
    /// hours of a day, so parts and their total add exactly.
    pub denominator: u32,
    pub hour: DockyardAmount,
    pub end: DockyardAmount,
    /// The first hour the country's repair output is known, when a whole
    /// campaign's amounts start there rather than at the recording's start.
    pub since: Option<i32>,
    /// The repaired designs' shares and the unattributed rest, which sum to
    /// the total wherever every part is known.
    pub parts: Vec<DockyardRepairPart>,
}

/// One repaired design's share of repairs, or (`variant` null) the dockyards
/// the game's repair selection does not place on a design.
#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DockyardRepairPart {
    pub variant: Option<String>,
    /// The design's content archetype, or its recorded token when content
    /// cannot resolve it; empty for the unattributed part.
    pub type_key: String,
    pub name: Option<String>,
    pub domain: Option<super::equipment::Domain>,
    pub names: Vec<super::equipment::StockVariantName>,
    pub hour: DockyardAmount,
    pub end: DockyardAmount,
}

/// One calendar month's dockyard IC, through the reply's hour.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DockyardMonth {
    pub first_hour: i32,
    pub last_hour: i32,
    pub new_ships: DockyardAmount,
    pub refits: DockyardAmount,
    pub repairs: DockyardAmount,
    /// Input from a line whose purpose is not proved may be counted as new.
    pub purpose_unproven: bool,
    /// A design whose type content cannot resolve spent IC this month, so
    /// the naval amounts may lack it.
    pub unresolved: bool,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DockyardSpendingReply {
    /// Production spending's reply split by purpose, holding only naval and
    /// unresolved designs' leaves (none in the months view).
    #[serde(flatten)]
    pub spending: ProductionSpendingReply,
    pub repairs: DockyardRepairs,
    /// The months view's columns, in order; empty in the circle view.
    pub months: Vec<DockyardMonth>,
}

impl super::ReplyWarnings for DockyardSpendingReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.spending.warnings
    }
}

/// The hours of a day: the denominator of every repair amount.
pub const REPAIR_DENOMINATOR: u32 = 24;

/// The first hour of the engine-calendar month holding `hour`, and of the next.
pub fn month_of(hour: i32) -> (i32, i32) {
    use engine_native::cdate::{HOUR_EPOCH, MONTH_LENGTHS};
    const YEAR: i64 = 365 * 24;
    let elapsed = i64::from(hour) - i64::from(HOUR_EPOCH);
    let year_start = i64::from(HOUR_EPOCH) + elapsed.div_euclid(YEAR) * YEAR;
    let mut start = year_start;
    for days in MONTH_LENGTHS {
        let next = start + i64::from(days) * 24;
        if i64::from(hour) < next {
            let clamp = |value: i64| i32::try_from(value).unwrap_or(i32::MAX);
            return (clamp(start), clamp(next));
        }
        start = next;
    }
    unreachable!("a year's months cover it")
}

/// The calendar months overlapping `first..=last`, each clipped to it.
pub fn months(first: i32, last: i32) -> Vec<(i32, i32)> {
    let mut result = Vec::new();
    let mut at = first;
    while at <= last {
        let (_, next) = month_of(at);
        let end = next.saturating_sub(1).min(last);
        result.push((at, end));
        if end == i32::MAX {
            break;
        }
        at = end + 1;
    }
    result
}

impl Product for DockyardSpending {
    const KEY: &'static str = "dockyard-spending";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DockyardSpendingParams;
    type Reply = DockyardSpendingReply;

    fn types() -> Vec<String> {
        vec![
            DockyardSpendingParams::decl(),
            DockyardView::decl(),
            DockyardAmount::decl(),
            DockyardRepairs::decl(),
            DockyardRepairPart::decl(),
            DockyardMonth::decl(),
            DockyardSpendingReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        validate_tag(&params.tag)
    }
}

#[cfg(test)]
mod tests;
