use super::*;
use crate::display::Rounding;
use crate::value::value_struct;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const TENTH: Step = Step::fixed5(10_000, Rounding::TowardZero);
pub const WHOLE: Step = Step::fixed5(100_000, Rounding::HalfAwayFromZero);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Gauge {
    pub id: u32,
    pub level: i64,
}
value_struct!(Gauge { id: u32, level @ TENTH: i64 });
opaque_value!(Vec<Gauge>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SteppedRow {
    pub id: u64,
    pub exact: i64,
    pub level: i64,
    pub optional: Option<i64>,
    pub recorded: Recorded<i64>,
    pub pair: [i64; 2],
    pub gauges: Vec<Gauge>,
}
typed_codec!(SteppedRow, 64, |row| row.id as u128,
 |key| Self { id: key as u64, ..Default::default() },
 { 0 => (exact): I64, 1 => (level): I64 @ TENTH, 2 => (optional): OptionalI64 @ TENTH, 3 => (recorded): I64 @ WHOLE, 4 => (pair): Opaque @ TENTH, 5 => (gauges): Opaque }, |_row| Ok(()));

pub static COUNTED_ROW_CHECKS: AtomicUsize = AtomicUsize::new(0);
#[derive(Clone, Default, PartialEq, Eq)]
pub struct CountedRow {
    pub key: u32,
    pub text: Vec<u8>,
}
typed_codec!(CountedRow, 32, |row| row.key as u128,
|key| Self { key: key as u32, ..Default::default() },
{ 0 => (text): Bytes }, |row| {
    COUNTED_ROW_CHECKS.fetch_add(1, Ordering::Relaxed);
    if row.text.is_empty() { return Err(invalid("synthetic missing text")); }
    Ok(())
});
