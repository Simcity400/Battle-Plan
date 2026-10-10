//! CFlagTrigger comparison operands on the supported build.
//! The referenced flag record has a separate layout.

/// CFlagTrigger constructor.
pub const CONSTRUCTOR_RVA: usize = 0x003c_5ee0;
/// Flag comparison uses signed i16 stored value and set hour.
pub const COMPARE_RVA: usize = 0x0138_f270;
/// Signed i32 value operand compared with the sign-extended i16 flag value.
pub const VALUE: usize = 0x7c;
/// CFlagTrigger value operator i32.
pub const VALUE_OPERATOR: usize = 0x80;
/// CFlagTrigger set-date hour operand i32.
pub const SET_DATE_HOUR: usize = 0x90;
/// CFlagTrigger set-date operator i32.
pub const SET_DATE_OPERATOR: usize = 0xa0;
/// CFlagTrigger elapsed-days operand i32.
pub const ELAPSED_DAYS: usize = 0xa4;
/// CFlagTrigger elapsed-days operator u32.
pub const ELAPSED_DAYS_OPERATOR: usize = 0xa8;
/// CFlagTrigger greater-than operator id.
pub const OPERATOR_GREATER_THAN: u32 = 0x1d3;
/// CFlagTrigger less-than operator id.
pub const OPERATOR_LESS_THAN: u32 = 0x1d4;
/// CFlagTrigger inactive date/days operator id.
pub const OPERATOR_INACTIVE: u32 = 0x165;

/// CFlagTrigger flag-name MSVC string.
pub const FLAG_NAME: usize = 0x58;
/// CFlagTrigger embedded set-date CGameDate vtable.
pub const SET_DATE: usize = 0x88;
/// Second embedded date vtable location; surrounding semantics remain unknown.
pub const SECOND_DATE_COMPONENT: usize = 0x98;
