//! Exact grouped-storage aliases. Native acquisition and public frame rows stay literal.
use super::MAX_BLOCK_RAW_BYTES;
use crate::{
    Hour,
    codec::{Decoder, invalid_static as invalid, put_string},
    format::FormatVersion,
    frame::{DecodedFrame, Frame, SharedView, TableUpdate, TableView, decode_update},
    rows::{self, TableId, TableSchema, compatibility::HISTORY},
};
use engine_allocation as allocation;
use std::io;

#[path = "private_aliases.rs"]
pub(super) mod private;

fn complete<T: TableSchema>(views: &[(TableId, SharedView)]) -> Option<&[T::Row]> {
    let (_, view) = views.iter().find(|(id, _)| *id == T::ID)?;
    match &**view {
        TableView::Complete(rows) => rows.rows::<T>(),
        _ => None,
    }
}

fn find<T: rows::RowCodec>(rows: Option<&[T]>, key: u128) -> Option<&T> {
    let rows = rows?;
    rows.binary_search_by_key(&key, rows::RowCodec::key)
        .ok()
        .map(|at| &rows[at])
}

/// The ordinary plain update of one view, preserving explicit snapshot resets.
fn plain(
    view: &TableView,
    previous: Option<(Hour, &TableView)>,
    snapshot: bool,
) -> io::Result<Vec<u8>> {
    let mut out = allocation::vec(9)?;
    match view {
        TableView::Complete(rows) => {
            if !snapshot && let Some((hour, TableView::Complete(before))) = previous {
                out.push(1);
                out.extend_from_slice(&hour.to_le_bytes());
                if !rows.encode_changes(before, &mut out, &mut Vec::new())? {
                    out.truncate(1);
                    out[0] = 2;
                    out.extend_from_slice(&hour.to_le_bytes());
                }
            } else {
                out.push(0);
                out.extend_from_slice(
                    &u32::try_from(rows.len())
                        .map_err(|_| invalid("too many alias rows"))?
                        .to_le_bytes(),
                );
                rows.encode_rows(&mut out)?;
            }
        }
        TableView::Truncated { rows, missing } => {
            out.push(3);
            out.extend_from_slice(&missing.to_le_bytes());
            out.extend_from_slice(
                &u32::try_from(rows.len())
                    .map_err(|_| invalid("too many alias rows"))?
                    .to_le_bytes(),
            );
            rows.encode_rows(&mut out)?;
        }
        TableView::Unavailable { reason } => {
            out.push(4);
            put_string(&mut out, reason)?;
        }
        TableView::NotRecorded { .. } => {
            return Err(invalid(
                "a table its capture mode does not record has no update",
            ));
        }
    }
    Ok(out)
}

fn update(table: TableId, bytes: &[u8], keyframe: bool) -> io::Result<TableUpdate> {
    let mut input = Decoder::new(bytes);
    let update = decode_update(
        table,
        &HISTORY[table.index()],
        FormatVersion::CURRENT,
        keyframe,
        &mut input,
    )?;
    input.finish()?;
    Ok(update)
}

#[derive(Default)]
pub(super) struct Encoder {
    private: private::Encoder,
}

pub(super) struct Prepared {
    pub private: private::Prepared,
    pub stored_sizes: [usize; rows::TABLE_COUNT],
    pub stored_len: usize,
    pub payload: Vec<u8>,
    pub sizes: [usize; rows::TABLE_COUNT],
    pub views: DecodedFrame,
}

impl Encoder {
    pub fn prepare(
        &self,
        frame: &Frame,
        decoded: &DecodedFrame,
        payload: &[u8],
        sizes: &[usize; rows::TABLE_COUNT],
    ) -> io::Result<Prepared> {
        let mut views = decoded.clone();
        let mut parts = allocation::vec(private::TABLES.len())?;
        let mut new_sizes = *sizes;
        let mut total = payload.len();
        let private = self.private.prepare(frame, decoded, &mut views)?;
        for (table, bytes) in private.parts() {
            total = allocation::add(total - sizes[table.index()], bytes.len())?;
            new_sizes[table.index()] = bytes.len();
            parts.push((table, allocation::copy_vec(bytes)?));
        }
        let mut stored_sizes = new_sizes;
        let stored_len = allocation::add(total, private.charge(&mut stored_sizes)?)?;
        let mut out = allocation::vec(total)?;
        let header = payload.len() - sizes.iter().sum::<usize>();
        out.extend_from_slice(&payload[..header]);
        let mut offset = header;
        for table in TableId::ALL {
            let end = offset + sizes[table.index()];
            match parts.iter().find(|(id, _)| *id == table) {
                Some((_, bytes)) => out.extend_from_slice(bytes),
                None => out.extend_from_slice(&payload[offset..end]),
            }
            offset = end;
        }
        Ok(Prepared {
            private,
            stored_sizes,
            stored_len,
            payload: out,
            sizes: new_sizes,
            views,
        })
    }

    pub fn commit(&mut self, prepared: Prepared) {
        self.private.commit(prepared.private);
    }
}
