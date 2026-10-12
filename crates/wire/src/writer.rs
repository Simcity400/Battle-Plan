use crate::errors::{Result, WireError, align8, checked_size, count32};
use crate::model::*;
use crate::reader;

fn append(target: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    target
        .len()
        .checked_add(bytes.len())
        .ok_or_else(|| WireError::new("size-overflow", "reply size overflow"))?;
    target
        .try_reserve(bytes.len())
        .map_err(|_| WireError::new("size-overflow", "reply allocation failed"))?;
    target.extend_from_slice(bytes);
    Ok(())
}
fn pad(target: &mut Vec<u8>) -> Result<()> {
    let len = align8(target.len())?;
    target.resize(len, 0);
    Ok(())
}
fn u16v(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn u32v(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn descriptors(reply: &Reply) -> Result<Vec<u8>> {
    let count = count32(reply.descriptors.len())?;
    let mut out = Vec::with_capacity(checked_size(u64::from(count), DESCRIPTOR_BYTES as u64)?);
    for d in &reply.descriptors {
        u32v(&mut out, d.name_id);
        u16v(&mut out, d.logical_type as u16);
        out.push(d.physical_encoding as u8);
        out.push(d.state_layers);
        u32v(&mut out, d.type_param_a);
        u32v(&mut out, d.type_param_b);
        u32v(&mut out, d.dictionary_ref);
        u32v(&mut out, d.child_first);
        u32v(&mut out, d.child_count);
        u32v(&mut out, d.data_buffer);
        u64v(&mut out, d.length);
        u32v(&mut out, d.aux_buffer);
        u32v(&mut out, d.state_buffer);
        u16v(&mut out, d.state_widths);
        u16v(&mut out, d.gate_column);
        u32v(&mut out, d.gate_value);
        u32v(&mut out, d.refusal_ref);
        u32v(&mut out, d.reserved);
    }
    Ok(out)
}
fn buffers(reply: &Reply) -> Result<Vec<u8>> {
    let count = count32(reply.buffers.len())?;
    let table_len = checked_size(u64::from(count), 16)?;
    let mut out = vec![0; table_len];
    for (i, buffer) in reply.buffers.iter().enumerate() {
        pad(&mut out)?;
        let offset = u64::try_from(out.len())
            .map_err(|_| WireError::new("size-overflow", "buffer offset overflow"))?;
        let length = u64::try_from(buffer.len())
            .map_err(|_| WireError::new("size-overflow", "buffer length overflow"))?;
        let slot = i
            .checked_mul(16)
            .ok_or_else(|| WireError::new("size-overflow", "buffer table overflow"))?;
        out[slot..slot + 8].copy_from_slice(&offset.to_le_bytes());
        out[slot + 8..slot + 16].copy_from_slice(&length.to_le_bytes());
        append(&mut out, buffer)?;
    }
    pad(&mut out)?;
    Ok(out)
}

/// Serializes caller-authored columns and refuses anything the reader would refuse.
/// Reply-local finite descriptor counts never limit a campaign inventory.
pub fn write(
    reply: &Reply,
    registry: &(impl ShapeRegistry + ?Sized),
    context: ReadContext,
) -> Result<Vec<u8>> {
    let mut sections: Vec<(u32, u32, Vec<u8>)> = vec![
        (1, count32(reply.descriptors.len())?, descriptors(reply)?),
        (
            2,
            count32(reply.top_level.len())?,
            reply
                .top_level
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect(),
        ),
        (3, count32(reply.buffers.len())?, buffers(reply)?),
    ];
    if !reply.dictionary_refs.is_empty() {
        let count = count32(reply.dictionary_refs.len())?;
        let mut b = Vec::with_capacity(checked_size(u64::from(count), 88)?);
        for r in &reply.dictionary_refs {
            b.extend_from_slice(&r.dictionary_identity);
            b.extend_from_slice(&r.shard_identity);
            u64v(&mut b, r.first_entry);
            u64v(&mut b, r.entry_count);
            u32v(&mut b, r.kind as u32);
            u32v(&mut b, r.flags);
        }
        sections.push((4, count, b));
    }
    if let Some(b) = reply.bounds {
        let mut out = Vec::with_capacity(24);
        u32v(&mut out, b.kind as u32);
        u32v(&mut out, 0);
        out.extend_from_slice(&b.start.to_le_bytes());
        out.extend_from_slice(&b.end.to_le_bytes());
        sections.push((9, 1, out));
    }
    if let Some(b) = &reply.diagnostics {
        sections.push((5, 1, b.clone()));
    }
    if let Some(b) = &reply.continuation {
        sections.push((6, 1, b.clone()));
    }
    if let Some(b) = &reply.diagnostic_continuation {
        sections.push((7, 1, b.clone()));
    }
    if !reply.extents.is_empty() {
        let count = count32(reply.extents.len())?;
        let mut b = Vec::with_capacity(checked_size(u64::from(count), 40)?);
        for e in &reply.extents {
            u32v(&mut b, e.column);
            b.push(e.coordinate as u8);
            b.push(u8::from(e.final_extent));
            u16v(&mut b, 0);
            u64v(&mut b, e.index);
            u64v(&mut b, e.first);
            u64v(&mut b, e.end);
            u64v(&mut b, e.total);
        }
        sections.push((8, count, b));
    }
    let section_count = count32(sections.len())?;
    let initial = HEADER_BYTES
        .checked_add(checked_size(u64::from(section_count), 24)?)
        .ok_or_else(|| WireError::new("size-overflow", "directory size overflow"))?;
    let mut out = vec![0; initial];
    out[..4].copy_from_slice(b"H4RQ");
    let h = &reply.header;
    out[4..6].copy_from_slice(&h.schema_version.to_le_bytes());
    out[6..8].copy_from_slice(&h.flags.to_le_bytes());
    out[8..16].copy_from_slice(&h.query_id.to_le_bytes());
    out[16..32].copy_from_slice(&h.campaign_id);
    out[32..40].copy_from_slice(&h.revision.to_le_bytes());
    out[40..48].copy_from_slice(&h.generation.to_le_bytes());
    out[48..52].copy_from_slice(&(h.reply_kind as u32).to_le_bytes());
    out[52..56].copy_from_slice(&h.shape_id.to_le_bytes());
    out[56..60].copy_from_slice(&h.shape_version.to_le_bytes());
    out[60..64].copy_from_slice(&section_count.to_le_bytes());
    out[64..68].copy_from_slice(&(h.unit_kind as u32).to_le_bytes());
    for (offset, value) in [
        (72, h.unit_start),
        (80, h.unit_end),
        (88, h.row_count),
        (96, h.total_count),
        (104, h.diagnostic_total),
    ] {
        out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    for (i, (kind, count, payload)) in sections.iter().enumerate() {
        pad(&mut out)?;
        let offset = u64::try_from(out.len())
            .map_err(|_| WireError::new("size-overflow", "section offset overflow"))?;
        let length = u64::try_from(payload.len())
            .map_err(|_| WireError::new("size-overflow", "section length overflow"))?;
        let slot = HEADER_BYTES + i * 24;
        out[slot..slot + 4].copy_from_slice(&kind.to_le_bytes());
        out[slot + 4..slot + 8].copy_from_slice(&count.to_le_bytes());
        out[slot + 8..slot + 16].copy_from_slice(&offset.to_le_bytes());
        out[slot + 16..slot + 24].copy_from_slice(&length.to_le_bytes());
        append(&mut out, payload)?;
    }
    let length = u64::try_from(out.len())
        .map_err(|_| WireError::new("size-overflow", "reply length overflow"))?;
    out[112..120].copy_from_slice(&length.to_le_bytes());
    let hash = xxhash_rust::xxh3::xxh3_64(&out[HEADER_BYTES..]);
    out[120..128].copy_from_slice(&hash.to_le_bytes());
    reader::read(&out, registry, context)?;
    Ok(out)
}

/// Synthetic fixtures use this after mutating a body to test validation beyond checksum.
pub fn update_checksum(bytes: &mut [u8]) -> Result<()> {
    if bytes.len() < HEADER_BYTES {
        return Err(WireError::new(
            "truncated-header",
            "checksum needs a complete header",
        ));
    }
    let hash = xxhash_rust::xxh3::xxh3_64(&bytes[HEADER_BYTES..]);
    bytes[120..128].copy_from_slice(&hash.to_le_bytes());
    Ok(())
}
