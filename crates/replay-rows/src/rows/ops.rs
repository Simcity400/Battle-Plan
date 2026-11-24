use super::*;
use crate::checkpoint::endpoint::{self, Edit};
use std::{fmt, ops::Range};

pub trait TableRow:
    RowCodec + crate::checkpoint::rows::Row + Fields + fmt::Debug + 'static
{
}
impl<
    R: RowCodec + crate::checkpoint::rows::Row + Fields + fmt::Debug + 'static,
> TableRow for R
{
}
pub struct RowOps<R: 'static> {
    pub decode: fn(usize, &mut Decoder<'_>) -> io::Result<Vec<R>>,
    pub from_wire:
        for<'a> fn(&[u128], &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>) -> io::Result<Vec<R>>,
    pub from_wire_fields: for<'a> fn(
        &[u128],
        u64,
        &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
    ) -> io::Result<Vec<R>>,
    pub patch: for<'a> fn(
        &mut [R],
        &[(u128, u64)],
        &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
    ) -> io::Result<()>,
    pub range_fits: fn(&[R], &[R], &[Range<u128>]) -> bool,
    #[cfg(any(test, feature = "capture", feature = "test-support"))]
    pub range_encode: fn(
        &[R],
        &[R],
        &[Range<u128>],
        &mut Vec<u8>,
        &mut Vec<u8>,
        Option<&mut ChangedRows>,
    ) -> io::Result<bool>,
    pub range_apply: fn(&mut Vec<R>, Vec<R>, &[Range<u128>]) -> io::Result<Vec<R>>,
    pub heap_bytes: fn(&Vec<R>) -> usize,
    pub truncate: fn(&mut Vec<R>, usize),
    pub round_to_display: fn(&mut [R]),
    pub contains_key: fn(&[R], u128) -> bool,
    pub validate: fn(&[R]) -> io::Result<()>,
    pub try_clone: fn(&[R]) -> io::Result<Vec<R>>,
    pub try_clone_validated: fn(&[R]) -> io::Result<Vec<R>>,
    pub recorded_bytes: fn(&[R]) -> io::Result<usize>,
    pub encode: fn(&[R], &mut Vec<u8>) -> io::Result<()>,
    pub encode_row_fields: fn(&[R], usize, &mut Vec<u8>) -> io::Result<()>,
    pub encode_row_recorded_fields: fn(&[R], usize, &mut Vec<u8>) -> io::Result<u64>,
    pub encode_changes:
        fn(&[R], &[R], &mut Vec<u8>, &mut Vec<u8>, Option<&mut ChangedRows>) -> io::Result<bool>,
    #[cfg(any(test, feature = "capture", feature = "test-support"))]
    pub encode_replacements: fn(&[R], &[R], &[usize], &mut Vec<u8>) -> io::Result<bool>,
    #[cfg(any(test, feature = "capture", feature = "test-support"))]
    pub install_replacements: fn(&mut [R], &mut [R], &[usize]),
    pub apply_delta: fn(Vec<R>, &[DeltaRow], &[u128]) -> io::Result<Vec<R>>,
    pub apply_delta_validated: fn(Vec<R>, &[DeltaRow], &[u128]) -> io::Result<Vec<R>>,
    pub apply_delta_shared: fn(&[R], &[DeltaRow], &[u128]) -> io::Result<Vec<R>>,
    pub apply_delta_shared_validated: fn(&[R], &[DeltaRow], &[u128]) -> io::Result<Vec<R>>,
    pub changes_since: fn(&[R], &[R]) -> io::Result<(Vec<R>, Vec<u128>)>,
    pub rows_at: fn(&[R], &[u128]) -> io::Result<Vec<R>>,
    pub apply_changes: fn(Vec<R>, Vec<R>, &[u128]) -> io::Result<Vec<R>>,
    pub checkpoint_encode: fn(&[R], &mut Vec<u8>) -> io::Result<()>,
    pub checkpoint_decode: fn(&mut Decoder<'_>) -> io::Result<Vec<R>>,
    pub endpoint_encode: fn(&[R], &[(u128, u64, bool)], &mut Vec<u8>) -> io::Result<()>,
    pub endpoint_decode: fn(&mut Decoder<'_>) -> io::Result<(Vec<R>, Vec<Edit>)>,
    pub endpoint_apply: fn(Vec<R>, Vec<R>, Vec<Edit>, &[u128]) -> io::Result<Vec<R>>,
    pub clone: fn(&Vec<R>) -> Vec<R>,
    pub eq: fn(&[R], &[R]) -> bool,
    pub row_eq: fn(&R, &R) -> bool,
    pub debug: fn(&Vec<R>, &mut fmt::Formatter<'_>) -> fmt::Result,
    pub row_debug: fn(&R, &mut fmt::Formatter<'_>) -> fmt::Result,
}

impl<R: TableRow> RowOps<R> {
    pub const fn new() -> Self {
        Self {
            decode: decode_typed::<R>,
            from_wire: rows_from_wire_typed::<R>,
            from_wire_fields: rows_from_wire_fields_typed::<R>,
            patch: patch_typed::<R>,
            range_fits: crate::sliced::ranges::fits::<R>,
            #[cfg(any(test, feature = "capture", feature = "test-support"))]
            range_encode: crate::sliced::ranges::encode::<R>,
            range_apply: crate::sliced::ranges::apply::<R>,
            heap_bytes: heap_bytes::<R>,
            truncate: truncate::<R>,
            round_to_display: round_rows_to_display::<R>,
            contains_key: contains_key::<R>,
            validate: validate_typed::<R>,
            try_clone: try_clone::<R>,
            try_clone_validated: try_clone_validated::<R>,
            recorded_bytes: recorded_bytes::<R>,
            encode: encode_typed::<R>,
            encode_row_fields: encode_row_fields::<R>,
            encode_row_recorded_fields: encode_row_recorded_fields::<R>,
            encode_changes: encode_changes_typed::<R>,
            #[cfg(any(test, feature = "capture", feature = "test-support"))]
            encode_replacements: encode_replacements_typed::<R>,
            #[cfg(any(test, feature = "capture", feature = "test-support"))]
            install_replacements: install_replacements::<R>,
            apply_delta: apply_typed::<R>,
            apply_delta_validated: apply_typed_validated::<R>,
            apply_delta_shared: apply_typed_shared::<R>,
            apply_delta_shared_validated: apply_typed_shared_validated::<R>,
            changes_since: changes_typed::<R>,
            rows_at: rows_at_typed::<R>,
            apply_changes: apply_changes_typed::<R>,
            checkpoint_encode: crate::checkpoint::rows::encode_rows::<R>,
            checkpoint_decode: crate::checkpoint::rows::decode_rows::<R>,
            endpoint_encode: endpoint::encode::<R>,
            endpoint_decode: endpoint::decode::<R>,
            endpoint_apply: endpoint::apply::<R>,
            clone: clone::<R>,
            eq: eq::<R>,
            row_eq: row_eq::<R>,
            debug: debug::<R>,
            row_debug: row_debug::<R>,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct FixtureOps<R> {
    pub spool: fn(&Vec<R>, &mut replay_spool::Writer<'_>) -> io::Result<()>,
    pub unspool: fn(&mut replay_spool::Reader<'_>) -> io::Result<Vec<R>>,
    pub one_default: fn() -> Vec<R>,
    pub sparse_masks: fn(&[R]),
    pub field_round_trip: fn(&[R], u64) -> io::Result<bool>,
}

#[cfg(any(test, feature = "test-support"))]
impl<R> FixtureOps<R>
where
    R: TableRow + replay_spool::Spooled + endpoint::sparse::SparseReference,
{
    pub const OPS: Self = Self {
        spool: spool::<R>,
        unspool: unspool::<R>,
        one_default: one_default::<R>,
        sparse_masks: sparse_masks::<R>,
        field_round_trip: field_round_trip::<R>,
    };
}

#[macro_export]
macro_rules! row_ops {
    ($($table:ident => $row:ident),+ $(,)?) => {
        $(#[expect(non_upper_case_globals, reason = "a descriptor is named as the table it serves")]
        pub static $table: $crate::rows::RowOps<$row> = $crate::rows::RowOps::new();)+
        #[cfg(feature = "test-support")]
        pub mod fixtures {
            use super::*;
            $(#[expect(non_upper_case_globals, reason = "a fixture descriptor shares its table name")]
            pub static $table: $crate::rows::ops::FixtureOps<$row> =
                $crate::rows::ops::FixtureOps::OPS;)+
        }
    };
}

fn heap_bytes<R: RowCodec>(rows: &Vec<R>) -> usize {
    rows.capacity() * std::mem::size_of::<R>()
        + rows.iter().map(RowCodec::heap_bytes).sum::<usize>()
}

fn truncate<R>(rows: &mut Vec<R>, len: usize) {
    rows.truncate(len);
}

fn contains_key<R: RowCodec>(rows: &[R], key: u128) -> bool {
    rows.binary_search_by_key(&key, |row| row.key()).is_ok()
}

fn try_clone<R: RowCodec>(rows: &[R]) -> io::Result<Vec<R>> {
    let mut owned = allocation::vec(rows.len())?;
    for row in rows {
        owned.push(RowCodec::try_clone(row)?);
    }
    Ok(owned)
}

fn try_clone_validated<R: RowCodec>(rows: &[R]) -> io::Result<Vec<R>> {
    let mut owned = allocation::vec(rows.len())?;
    for row in rows {
        owned.push(RowCodec::try_clone_validated(row)?);
    }
    Ok(owned)
}

fn recorded_bytes<R: RowCodec>(rows: &[R]) -> io::Result<usize> {
    validate_typed(rows)?;
    let mut bytes = 4;
    let mut previous = 0;
    for row in rows {
        bytes = allocation::add(bytes, varint_bytes(row.key() - previous))?;
        bytes = allocation::add(bytes, row.recorded_fields_bytes()?)?;
        previous = row.key();
    }
    Ok(bytes)
}

fn encode_row_fields<R: RowCodec>(rows: &[R], index: usize, output: &mut Vec<u8>) -> io::Result<()> {
    rows.get(index)
        .ok_or_else(|| invalid("row index out of range"))?
        .encode_fields(output)
}

fn encode_row_recorded_fields<R: RowCodec>(
    rows: &[R],
    index: usize,
    output: &mut Vec<u8>,
) -> io::Result<u64> {
    rows.get(index)
        .ok_or_else(|| invalid("row index out of range"))?
        .encode_recorded_fields(output)
}

#[cfg(any(test, feature = "capture", feature = "test-support"))]
fn install_replacements<R>(previous: &mut [R], rows: &mut [R], indices: &[usize]) {
    for (&index, row) in indices.iter().zip(rows.iter_mut()) {
        std::mem::swap(&mut previous[index], row);
    }
}

fn clone<R: Clone>(rows: &Vec<R>) -> Vec<R> {
    rows.clone()
}

fn eq<R: PartialEq>(rows: &[R], other: &[R]) -> bool {
    rows == other
}

fn row_eq<R: PartialEq>(row: &R, other: &R) -> bool {
    row == other
}

fn debug<R: fmt::Debug>(rows: &Vec<R>, out: &mut fmt::Formatter<'_>) -> fmt::Result {
    fmt::Debug::fmt(rows, out)
}

fn row_debug<R: fmt::Debug>(row: &R, out: &mut fmt::Formatter<'_>) -> fmt::Result {
    fmt::Debug::fmt(row, out)
}

#[cfg(any(test, feature = "test-support"))]
fn spool<R: replay_spool::Spooled>(
    rows: &Vec<R>,
    out: &mut replay_spool::Writer<'_>,
) -> io::Result<()> {
    replay_spool::Spooled::spool(rows, out)
}

#[cfg(any(test, feature = "test-support"))]
fn unspool<R: replay_spool::Spooled>(input: &mut replay_spool::Reader<'_>) -> io::Result<Vec<R>> {
    <Vec<R> as replay_spool::Spooled>::unspool(input)
}

#[cfg(any(test, feature = "test-support"))]
fn one_default<R: Default>() -> Vec<R> {
    vec![R::default()]
}

#[cfg(any(test, feature = "test-support"))]
fn sparse_masks<R: crate::checkpoint::endpoint::sparse::SparseReference>(rows: &[R]) {
    assert!(!rows.is_empty());
    for row in rows {
        crate::checkpoint::endpoint::sparse::check_sparse_masks(row);
    }
}

#[cfg(any(test, feature = "test-support"))]
fn field_round_trip<R: crate::checkpoint::rows::Row + Fields>(
    rows: &[R],
    mask: u64,
) -> io::Result<bool> {
    for row in rows {
        if !endpoint::field_round_trip(row, mask)? {
            return Ok(false);
        }
    }
    Ok(true)
}
