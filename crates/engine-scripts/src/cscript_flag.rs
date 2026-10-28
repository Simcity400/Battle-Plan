//! CScriptFlag's exact token, set date, signed value and expiry countdown.

use crate::{
    Memory,
    memory::{RecordField, RecordLayout},
};

/// CScriptFlag stride.
pub const STRIDE: usize = 0x30;
/// CScriptFlag primary vtable.
pub const VTABLE_RVA: usize = 0x0279_f310;
/// CScriptFlag token id.
pub const TOKEN_ID: usize = 0x08;
/// CScriptFlag CGameDate primary vtable.
pub const DATE_VTABLE_RVA: usize = 0x0279_63d0;
/// CScriptFlag date primary object.
pub const DATE: usize = 0x10;
/// Latest-set raw hour copied from the current game date.
pub const SET_HOUR: usize = 0x18;
/// CScriptFlag CGameDate secondary vtable.
pub const DATE_SECONDARY_VTABLE_RVA: usize = 0x0279_6400;
/// CScriptFlag date secondary interface.
pub const DATE_SECONDARY: usize = 0x20;
/// CScriptFlag signed i16 value.
pub const VALUE: usize = 0x28;
/// Expiry days, signed i16; positive values decrement daily and remove at zero.
pub const REMAINING_DAYS: usize = 0x2a;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub token: u32,
    pub set_hour: i32,
    pub value: i16,
    /// Positive values count down daily; nonpositive native values stay exact.
    pub remaining_days: i16,
}

/// The three exact field spans `read_row` gathers, packed in its order.
const FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 12,
    },
    RecordField {
        source_offset: DATE,
        destination_offset: 12,
        len: 12,
    },
    RecordField {
        source_offset: DATE_SECONDARY,
        destination_offset: 24,
        len: 12,
    },
];
/// Packed bytes of one row's three spans.
pub const FIELD_BYTES: usize = 36;
/// One dense row's spans for strided record gathers over a manager's rows.
pub const LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&FIELDS, FIELD_BYTES).expect("script flag field layout");

pub fn read_row(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("flag row address overflow")
    };
    let mut token = [0_u8; 12];
    let mut date = [0_u8; 12];
    let mut value = [0_u8; 12];
    if memory.copy_scattered(
        [pointer, at(DATE)?, at(DATE_SECONDARY)?],
        &mut [&mut token, &mut date, &mut value],
    ) != 3
    {
        return Err("script flag fields unreadable");
    }
    decode(base, &token, &date, &value)
}

/// Authenticate and decode one row's spans as gathered by `LAYOUT`.
pub fn decode_fields(base: usize, fields: &[u8]) -> Result<Row, &'static str> {
    let [token, date, value] = [0, 12, 24].map(|at| &fields[at..at + 12]);
    decode(base, token, date, value)
}

fn decode(base: usize, token: &[u8], date: &[u8], value: &[u8]) -> Result<Row, &'static str> {
    let expected = |rva| {
        base.checked_add(rva)
            .map(|p| p as u64)
            .ok_or("module address overflow")
    };
    if u64::from_le_bytes(token[..8].try_into().unwrap()) != expected(VTABLE_RVA)? {
        return Err("script flag vtable mismatch");
    }
    if u64::from_le_bytes(date[..8].try_into().unwrap()) != expected(DATE_VTABLE_RVA)?
        || u64::from_le_bytes(value[..8].try_into().unwrap())
            != expected(DATE_SECONDARY_VTABLE_RVA)?
    {
        return Err("script flag date vtable mismatch");
    }
    Ok(Row {
        token: u32::from_le_bytes(token[8..12].try_into().unwrap()),
        set_hour: i32::from_le_bytes(date[8..12].try_into().unwrap()),
        value: i16::from_le_bytes(value[8..10].try_into().unwrap()),
        remaining_days: i16::from_le_bytes(value[10..12].try_into().unwrap()),
    })
}
