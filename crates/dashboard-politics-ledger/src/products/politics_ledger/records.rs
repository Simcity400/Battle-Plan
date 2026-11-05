//! What the emit stores of each recorded row and item, as a reply shows them ([`super::query`]):
//! each in one text cell, MessagePack ([`codec`]). A reply decodes the rows of the hours it
//! reads and the items they name; it never names, resolves or formats anything.

use super::*;
use dashboard_base::prepared::codec;
use dashboard_series::{Cell, Writer};

/// A recorded power row: what a reply's balance changes and payments read of it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PowerRecord {
    pub country: u32,
    pub epoch: u64,
    pub sequence: u64,
    pub hour: i32,
    pub command: bool,
    pub realized: i128,
    pub covered: bool,
    /// Its item's ordinal ([`ItemRecord`]).
    pub item: u32,
    /// [`emit::PAYMENT`], [`emit::BALANCE`] or [`emit::SKIPPED`].
    pub class: u64,
    /// Read after its native day settled: shown on its own, counted by no rollup.
    pub late: bool,
    pub recurring: bool,
    pub upkeep: bool,
    /// A payment's evaluated charge; None where it is not recorded.
    pub charge: Option<i128>,
}

/// A recorded daily cost's first record, with what it is to a reply's payments.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CostRecord {
    pub country: u32,
    pub epoch: u64,
    pub day_hour: i32,
    pub ordinal: u32,
    pub pp_sequence: u64,
    pub covered: bool,
    pub cost_raw: i64,
    pub item: u32,
    /// [`emit::PAYMENT`], [`emit::UNMATCHED`] or [`emit::CREDIT`].
    pub class: u64,
    pub late: bool,
    pub recurring: bool,
    pub upkeep: bool,
}

/// A recorded daily row, as read.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct DailyRecord {
    pub epoch: u64,
    pub daily: LedgerDaily,
}

/// An item as every reply shows it, resolved at its first payment from the facts that payment's
/// hour recorded and the recorded content.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ItemRecord {
    /// Its id is its first payment's.
    pub item: LedgerSpendItem,
    pub category: (String, String),
    pub ideas: Option<LedgerIdeaTooltips>,
    /// The warnings formatting its tooltip gave.
    pub formats: Vec<String>,
    /// Its country's name at its first payment.
    pub country_name: Option<String>,
    pub currency: String,
    /// Every ability's upkeep: its payments show no item of their own.
    pub ability_upkeep: bool,
    /// A daily cost's category where its cause files it by component.
    pub daily: Option<(String, String)>,
}

/// `value` as a stored record's cell.
pub(super) fn cell<T: Serialize>(writer: &mut Writer<'_>, value: &T) -> io::Result<Cell> {
    Ok(Cell::Text(writer.bytes(&codec::encode(value)?)?))
}
