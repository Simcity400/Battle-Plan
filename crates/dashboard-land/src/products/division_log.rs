mod tail;

#[cfg(feature = "test-support")]
pub use tail::is_cached;

pub(crate) use tail::forget;
pub use tail::{DivisionLogTail, dated_panel};

use dashboard_query::log::Cell;
use replay_data::{
    codec::Decoder,
    rows::{DivisionDeletionDisposition as Disposition, DivisionEventRow, EquipmentDestructionCause as Cause, Recorded, TagHolderRow},
};

#[cfg(test)]
mod tests;

pub(super) mod column {
    pub const DISPOSITION: usize = 0;
    pub const CAUSE: usize = 1;
    pub const SLOT: usize = 3;
    pub const NATIVE_HOUR: usize = 4;
    pub const EXACT: usize = 5;
    pub const AFTER: usize = 6;
    pub const PROVINCE: usize = 7;
    pub const PLACE_HOUR: usize = 8;
    pub const NAME: usize = 11;
    pub const NAMING: usize = 12;
    pub const ICON_UNIT: usize = 14;
    pub const ICON_CUSTOM: usize = 15;
    pub const ICON_NAME: usize = 16;
    pub const KILLER_SLOT: usize = 19;
    pub const KILLER_DIVISION: usize = 20;
    pub const TEMPLATE_ID: usize = 21;
    pub const MATCHED_DEATHS: usize = 22;
    pub const TEMPLATE_LITERAL: usize = 23;
    pub const TEMPLATE_KEY: usize = 24;
    pub const TEMPLATE_COUNTRY: usize = 25;
}

pub(super) fn number(cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(column) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn bytes(cells: &[Cell], column: usize) -> Option<&[u8]> {
    match cells.get(column) {
        Some(Cell::Text(text)) => Some(text.as_bytes()),
        _ => None,
    }
}

fn opaque<T: replay_data::value::Value>(bytes: Option<&[u8]>) -> Recorded<T> {
    bytes.map_or(Recorded::NotRecorded, |bytes| T::decode(&mut Decoder::new(bytes)).map_or(Recorded::NotRecorded, Recorded::Value))
}

pub fn status(kind: u8, disposition: Option<i64>, cause: Option<i64>) -> String {
    if kind == DivisionEventRow::CREATED {
        return "first_observed".into();
    }
    if let Some(cause) = cause.and_then(|cause| u8::try_from(cause).ok()).and_then(|cause| Cause::try_from(cause).ok()) {
        return cause.as_str().into();
    }
    let disposition = match disposition {
        Some(0) => Disposition::Loss,
        Some(1) => Disposition::Returned,
        Some(2) => Disposition::Merged,
        Some(3) => Disposition::HqTransferred,
        Some(4) => Disposition::VolunteerTransferred,
        Some(5) => Disposition::ExileTransferred,
        _ => return "unknown".into(),
    };
    disposition.as_str().into()
}

pub(super) fn slot(value: Option<i64>) -> Option<u32> {
    value.map(|slot| slot as u32).filter(|&slot| slot != TagHolderRow::NONE)
}

pub fn unread(read: &[(i32, i32)], first: i32, last: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut at = i64::from(first);
    for &(a, b) in read {
        if i64::from(b) < at || a > last {
            continue;
        }
        if i64::from(a) > at {
            out.push((at as i32, a - 1));
        }
        at = at.max(i64::from(b) + 1);
    }
    if at <= i64::from(last) {
        out.push((at as i32, last));
    }
    out
}
