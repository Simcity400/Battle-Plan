//! Absolute typed patches. Recording wire deltas never enter this codec.
use super::rows::Row;
use crate::{
    codec::{Decoder, invalid_static as invalid},
    rows::{Fields, RowCodec, TableRows},
};
use engine_allocation as allocation;
use std::{fmt, io};

pub(crate) fn full_mask(fields: usize) -> u64 {
    u64::MAX
        .checked_shr((64usize.saturating_sub(fields)) as u32)
        .unwrap_or(0)
}
fn check<T: Row + Fields>(key: u128, mask: u64, replace: bool) -> io::Result<()> {
    if T::KEY_BITS < 128 && key >> T::KEY_BITS != 0 {
        return Err(invalid("endpoint key exceeds table width"));
    }
    let full = full_mask(T::SCHEMA.len());
    if mask == 0 || mask & !full != 0 {
        return Err(invalid("endpoint field mask is empty or has unknown bits"));
    }
    if replace && mask != full {
        return Err(invalid("endpoint insertion requires every field"));
    }
    Ok(())
}

pub(crate) struct Edit {
    key: u128,
    mask: u64,
    replace: bool,
}

pub struct EndpointRows {
    pub(crate) rows: TableRows,
    pub(crate) edits: Vec<Edit>,
}
impl EndpointRows {
    pub fn len(&self) -> usize {
        self.edits.len()
    }
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
    pub fn edits(&self) -> impl ExactSizeIterator<Item = (u128, u64, bool)> + '_ {
        self.edits
            .iter()
            .map(|edit| (edit.key, edit.mask, edit.replace))
    }
    pub fn inserts(&self) -> bool {
        self.edits.iter().any(|edit| edit.replace)
    }
}
impl fmt::Debug for EndpointRows {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("EndpointRows")
            .field("table", &self.rows.table())
            .field("len", &self.len())
            .finish()
    }
}

#[inline(never)]
pub(crate) fn encode<T: Row + Fields>(
    rows: &[T],
    edits: &[(u128, u64, bool)],
    out: &mut Vec<u8>,
) -> io::Result<()> {
    allocation::reserve(out, 4)?;
    out.extend_from_slice(
        &u32::try_from(edits.len())
            .map_err(|_| invalid("endpoint row count exceeds u32"))?
            .to_le_bytes(),
    );
    let mut previous = None;
    for &(key, mask, replace) in edits {
        check::<T>(key, mask, replace)?;
        if previous.is_some_and(|old| key <= old) {
            return Err(invalid("endpoint keys must be unique and ascending"));
        }
        let at = rows
            .binary_search_by_key(&key, RowCodec::key)
            .map_err(|_| invalid("endpoint changed key is absent"))?;
        let row = &rows[at];
        row.check_key_fields()?;
        allocation::reserve(out, 25)?;
        out.extend_from_slice(&key.to_le_bytes());
        out.extend_from_slice(&mask.to_le_bytes());
        out.push(u8::from(replace));
        row.encode_endpoint_fields(mask, out)?;
        previous = Some(key);
    }
    Ok(())
}

#[inline(never)]
pub(crate) fn decode<T: Row + Fields>(input: &mut Decoder<'_>) -> io::Result<(Vec<T>, Vec<Edit>)> {
    let count = input.count(25)?;
    let mut rows = allocation::vec(count)?;
    let mut edits = allocation::vec(count)?;
    let mut previous = None;
    for _ in 0..count {
        let key = u128::from_le_bytes(input.take(16)?.try_into().unwrap());
        let mask = input.u64()?;
        let replace = input.bool()?;
        check::<T>(key, mask, replace)?;
        if previous.is_some_and(|old| key <= old) {
            return Err(invalid("endpoint keys must be unique and ascending"));
        }
        rows.push(T::decode_endpoint_fields(key, mask, input)?);
        edits.push(Edit { key, mask, replace });
        previous = Some(key);
    }
    Ok((rows, edits))
}

#[inline(never)]
pub(crate) fn apply<T: Row + Fields>(
    previous: Vec<T>,
    rows: Vec<T>,
    edits: Vec<Edit>,
    deleted: &[u128],
) -> io::Result<Vec<T>> {
    if rows.len() != edits.len() {
        return Err(invalid("endpoint row and edit counts differ"));
    }
    if deleted.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(invalid(
            "endpoint deleted keys must be unique and ascending",
        ));
    }
    for &key in deleted {
        if T::KEY_BITS < 128 && key >> T::KEY_BITS != 0 {
            return Err(invalid("endpoint deleted key exceeds table width"));
        }
    }
    if edits.windows(2).any(|pair| pair[0].key >= pair[1].key) {
        return Err(invalid("endpoint keys must be unique and ascending"));
    }
    if deleted.is_empty() && edits.iter().all(|edit| !edit.replace) {
        let mut previous = previous;
        let mut from = 0;
        for (next, edit) in rows.into_iter().zip(edits) {
            check::<T>(edit.key, edit.mask, edit.replace)?;
            let at = from + previous[from..].partition_point(|row| row.key() < edit.key);
            let row = previous
                .get_mut(at)
                .filter(|row| row.key() == edit.key)
                .ok_or_else(|| invalid("endpoint patch has no row baseline"))?;
            row.install_endpoint_fields(next, edit.mask)?;
            row.check_key_fields()?;
            if row.key() != edit.key {
                return Err(invalid("endpoint row key differs from edit key"));
            }
            from = at + 1;
        }
        return Ok(previous);
    }
    let capacity = allocation::add(previous.len(), edits.len())?;
    let mut result = allocation::vec(capacity)?;
    let mut before = previous.into_iter().peekable();
    let mut removed = 0;
    for (next, edit) in rows.into_iter().zip(edits) {
        check::<T>(edit.key, edit.mask, edit.replace)?;
        if deleted.binary_search(&edit.key).is_ok() {
            return Err(invalid("endpoint changed and deleted keys overlap"));
        }
        while before.peek().is_some_and(|row| row.key() < edit.key) {
            let row = before.next().unwrap();
            if deleted.binary_search(&row.key()).is_err() {
                result.push(row);
            } else {
                removed += 1;
            }
        }
        let old = if before.peek().is_some_and(|row| row.key() == edit.key) {
            before.next()
        } else {
            None
        };
        let row = if edit.replace {
            next
        } else {
            let mut row = old.ok_or_else(|| invalid("endpoint patch has no row baseline"))?;
            row.install_endpoint_fields(next, edit.mask)?;
            row
        };
        row.check_key_fields()?;
        if row.key() != edit.key {
            return Err(invalid("endpoint row key differs from edit key"));
        }
        result.push(row);
    }
    for row in before {
        if deleted.binary_search(&row.key()).is_err() {
            result.push(row);
        } else {
            removed += 1;
        }
    }
    if removed != deleted.len() {
        return Err(invalid("endpoint deletes an absent key"));
    }
    Ok(result)
}

#[cfg(test)]
#[inline(never)]
pub(crate) fn field_round_trip<T: Row + Fields>(source: &T, mask: u64) -> io::Result<bool> {
    let mut expected = Vec::new();
    source.encode_endpoint_fields(mask, &mut expected)?;
    let mut input = Decoder::new(&expected);
    let decoded = T::decode_endpoint_fields(source.key(), mask, &mut input)?;
    input.finish()?;
    let mut empty = Decoder::new(&[]);
    let mut installed = T::decode_endpoint_fields(source.key(), 0, &mut empty)?;
    installed.install_endpoint_fields(decoded, mask)?;
    let mut actual = Vec::new();
    installed.encode_endpoint_fields(mask, &mut actual)?;
    Ok(actual == expected)
}

type EncodeTable = fn(&TableRows, &[(u128, u64, bool)], &mut Vec<u8>) -> io::Result<()>;

pub(crate) struct TableOps {
    pub encode: EncodeTable,
    pub decode: fn(&mut Decoder<'_>) -> io::Result<EndpointRows>,
    pub apply: fn(TableRows, EndpointRows, &[u128]) -> io::Result<TableRows>,
    #[cfg(test)]
    pub field_round_trip: fn(&TableRows, u64) -> io::Result<bool>,
    #[cfg(test)]
    pub sparse_masks: fn(&TableRows),
}

macro_rules! tables {
    ($($table:ident => $row:ty),+ $(,)?) => {
        mod endpoint_dispatch {
            use super::*;
            $(#[allow(non_snake_case)]
            mod $table {
                use super::*;
                #[inline(never)]
                pub(super) fn encode(
                    rows: &$crate::rows::TableRows,
                    edits: &[(u128, u64, bool)],
                    out: &mut Vec<u8>,
                ) -> std::io::Result<()> {
                    let $crate::rows::TableRows::$table(rows) = rows else {
                        return Err($crate::codec::invalid_static("endpoint row table mismatch"));
                    };
                    $crate::checkpoint::endpoint::encode::<$row>(rows, edits, out)
                }
                #[inline(never)]
                pub(super) fn decode(
                    input: &mut $crate::codec::Decoder<'_>,
                ) -> std::io::Result<$crate::checkpoint::endpoint::EndpointRows> {
                    let (rows, edits) = $crate::checkpoint::endpoint::decode::<$row>(input)?;
                    Ok($crate::checkpoint::endpoint::EndpointRows {
                        rows: $crate::rows::TableRows::$table(rows),
                        edits,
                    })
                }
                #[inline(never)]
                pub(super) fn apply(
                    before: $crate::rows::TableRows,
                    edits: $crate::checkpoint::endpoint::EndpointRows,
                    deleted: &[u128],
                ) -> std::io::Result<$crate::rows::TableRows> {
                    let $crate::rows::TableRows::$table(before) = before else {
                        return Err($crate::codec::invalid_static("endpoint row table mismatch"));
                    };
                    let $crate::rows::TableRows::$table(rows) = edits.rows else {
                        return Err($crate::codec::invalid_static("endpoint row table mismatch"));
                    };
                    $crate::checkpoint::endpoint::apply::<$row>(before, rows, edits.edits, deleted)
                        .map($crate::rows::TableRows::$table)
                }
                #[cfg(test)]
                pub(super) fn sparse_masks(rows: &$crate::rows::TableRows) {
                    let $crate::rows::TableRows::$table(rows) = rows else { panic!("wrong oracle table") };
                    assert!(!rows.is_empty());
                    for row in rows {
                        $crate::checkpoint::endpoint::tests::check_sparse_masks(row);
                    }
                }
                #[cfg(test)]
                #[inline(never)]
                pub(super) fn field_round_trip(
                    rows: &$crate::rows::TableRows,
                    mask: u64,
                ) -> std::io::Result<bool> {
                    let $crate::rows::TableRows::$table(rows) = rows else {
                        return Err($crate::codec::invalid_static("endpoint row table mismatch"));
                    };
                    for row in rows {
                        if !$crate::checkpoint::endpoint::field_round_trip(row, mask)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
            })+
            pub(super) static TABLES: [$crate::checkpoint::endpoint::TableOps; $crate::rows::TABLE_COUNT] = [
                $($crate::checkpoint::endpoint::TableOps {
                    encode: $table::encode,
                    decode: $table::decode,
                    apply: $table::apply,
                    #[cfg(test)]
                    field_round_trip: $table::field_round_trip,
                    #[cfg(test)]
                    sparse_masks: $table::sparse_masks,
                }),+
            ];
        }
        impl $crate::rows::TableId {
            pub fn decode_endpoint_rows(
                self,
                input: &mut $crate::codec::Decoder<'_>,
            ) -> std::io::Result<$crate::checkpoint::endpoint::EndpointRows> {
                (endpoint_dispatch::TABLES[self.index()].decode)(input)
            }
        }
        impl $crate::rows::TableRows {
            #[cfg(test)]
            pub(crate) fn check_endpoint_sparse_masks(&self) {
                (endpoint_dispatch::TABLES[self.table().index()].sparse_masks)(self)
            }
            #[cfg(test)]
            pub(crate) fn endpoint_field_round_trip(&self, mask: u64) -> std::io::Result<bool> {
                (endpoint_dispatch::TABLES[self.table().index()].field_round_trip)(self, mask)
            }
            pub fn encode_endpoint_rows(
                &self,
                edits: &[(u128, u64, bool)],
                out: &mut Vec<u8>,
            ) -> std::io::Result<()> {
                (endpoint_dispatch::TABLES[self.table().index()].encode)(self, edits, out)
            }
            pub fn apply_endpoint_rows(
                self,
                edits: $crate::checkpoint::endpoint::EndpointRows,
                deleted: &[u128],
            ) -> std::io::Result<Self> {
                let table = self.table();
                if table != edits.rows.table() {
                    return Err($crate::codec::invalid_static("endpoint row table mismatch"));
                }
                (endpoint_dispatch::TABLES[table.index()].apply)(self, edits, deleted)
            }
        }
    };
}
pub(crate) use tables;

#[cfg(test)]
pub(crate) mod tests;
