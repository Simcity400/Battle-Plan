use super::{Edits, Live};
use serde_json::Value;
use std::{io, ops::Range, sync::Arc};

/// A complete MessagePack record, indexed at the scalar cells a product can patch.
pub(super) struct Encoding {
    pub body: Arc<[u8]>,
    offsets: Vec<Vec<Range<usize>>>,
}

impl Encoding {
    pub fn new(value: &Value) -> io::Result<Self> {
        let mut body = Vec::new();
        let mut offsets = Vec::new();
        object(value, &mut body, |key, value, body| {
            if key != "live" { return write(value, body); }
            object(value, body, |key, value, body| {
                if key != "groups" { return write(value, body); }
                array(value, body, |group, body| object(group, body, |key, value, body| {
                    if key != "values" { return write(value, body); }
                    let mut group_offsets = Vec::new();
                    array(value, body, |cell, body| {
                        let start = body.len();
                        write(cell, body)?;
                        group_offsets.push(start..body.len());
                        Ok(())
                    })?;
                    offsets.push(group_offsets);
                    Ok(())
                }))
            })
        })?;
        Ok(Self { body: body.into(), offsets })
    }

    pub fn patch(&mut self, data: &Value, live: &Live, edits: &Edits) -> io::Result<()> {
        let mut replacements = Vec::new();
        for (i, group) in live.groups.iter().enumerate() {
            let Some(rows) = edits.tables.get(&group.table) else { continue };
            for (&key, &mask) in rows {
                let Ok(row) = group.keys.binary_search(&key) else { continue };
                for (col, &field) in group.columns.iter().enumerate() {
                    if mask & (1 << field) == 0 { continue; }
                    let index = row * group.columns.len() + col;
                    let old = &self.offsets[i][index];
                    let bytes = rmp_serde::to_vec(&data["live"]["groups"][i]["values"][index]).map_err(io::Error::other)?;
                    if self.body[old.clone()] != bytes { replacements.push((old.clone(), bytes)); }
                }
            }
        }
        if replacements.is_empty() { return Ok(()); }
        let mut next = Vec::with_capacity(self.body.len());
        let mut at = 0;
        let mut delta = 0isize;
        let mut edits = replacements.iter().peekable();
        for offset in self.offsets.iter_mut().flatten() {
            let before = offset.clone();
            offset.start = before.start.checked_add_signed(delta).unwrap();
            if let Some((range, bytes)) = edits.next_if(|(range, _)| *range == before) {
                next.extend_from_slice(&self.body[at..range.start]);
                next.extend_from_slice(bytes);
                at = range.end;
                delta += bytes.len() as isize - range.len() as isize;
            }
            offset.end = before.end.checked_add_signed(delta).unwrap();
        }
        next.extend_from_slice(&self.body[at..]);
        self.body = next.into();
        Ok(())
    }
}

fn write(value: &Value, out: &mut Vec<u8>) -> io::Result<()> {
    rmp_serde::encode::write(out, value).map_err(io::Error::other)
}

fn object(value: &Value, out: &mut Vec<u8>, mut field: impl FnMut(&str, &Value, &mut Vec<u8>) -> io::Result<()>) -> io::Result<()> {
    let Some(map) = value.as_object() else { return write(value, out) };
    rmp::encode::write_map_len(out, u32::try_from(map.len()).map_err(io::Error::other)?).map_err(io::Error::other)?;
    for (key, value) in map {
        rmp_serde::encode::write(&mut *out, key).map_err(io::Error::other)?;
        field(key, value, out)?;
    }
    Ok(())
}

fn array(value: &Value, out: &mut Vec<u8>, mut item: impl FnMut(&Value, &mut Vec<u8>) -> io::Result<()>) -> io::Result<()> {
    let Some(items) = value.as_array() else { return write(value, out) };
    rmp::encode::write_array_len(out, u32::try_from(items.len()).map_err(io::Error::other)?).map_err(io::Error::other)?;
    for value in items {
        item(value, out)?;
    }
    Ok(())
}
