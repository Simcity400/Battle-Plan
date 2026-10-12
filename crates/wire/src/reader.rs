use crate::errors::{Result, WireError, checked_size, err, usize_count};
use crate::model::*;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub(crate) fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("validated width"),
    )
}
pub(crate) fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated width"),
    )
}
pub(crate) fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated width"),
    )
}
pub(crate) fn i64_at(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated width"),
    )
}
fn named<T>(result: Result<T>, name: &'static str) -> Result<T> {
    result.map_err(|_| WireError::new(name, "unknown numeric code"))
}
fn bytes_range<'a>(
    bytes: &'a [u8],
    offset: u64,
    length: u64,
    name: &'static str,
) -> Result<&'a [u8]> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| WireError::new(name, "range overflow"))?;
    bytes
        .get(usize_count(offset)?..usize_count(end)?)
        .ok_or_else(|| WireError::new(name, "range outside received bytes"))
}

pub fn read<'a>(
    bytes: &'a [u8],
    registry: &(impl ShapeRegistry + ?Sized),
    context: ReadContext,
) -> Result<ValidatedReply<'a>> {
    read_inner(bytes, registry, context, false)
}
fn read_inner<'a>(
    bytes: &'a [u8],
    registry: &(impl ShapeRegistry + ?Sized),
    context: ReadContext,
    embedded: bool,
) -> Result<ValidatedReply<'a>> {
    if bytes.len() < HEADER_BYTES {
        return err("truncated-header", "128-byte header required");
    }
    if &bytes[..4] != b"H4RQ" {
        return err("invalid-magic", "expected H4RQ");
    }
    let schema_version = u16_at(bytes, 4);
    if schema_version != SCHEMA_VERSION {
        return err("unknown-schema-version", format!("schema {schema_version}"));
    }
    let flags = u16_at(bytes, 6);
    if flags & !15 != 0 {
        return err("invalid-flags", "reserved flag bits set");
    }
    if u64_at(bytes, 112) != bytes.len() as u64 {
        return err(
            "invalid-length",
            "header length differs from received bytes",
        );
    }
    if xxhash_rust::xxh3::xxh3_64(&bytes[HEADER_BYTES..]) != u64_at(bytes, 120) {
        return err("checksum-mismatch", "body hash differs");
    }
    let reply_kind = named(ReplyKind::try_from(u32_at(bytes, 48)), "invalid-reply-kind")?;
    let unit_kind = named(UnitKind::try_from(u32_at(bytes, 64)), "invalid-unit-kind")?;
    if u32_at(bytes, 68) != 0 {
        return err("invalid-flags", "reserved header field is nonzero");
    }
    let header = Header {
        schema_version,
        flags,
        query_id: u64_at(bytes, 8),
        campaign_id: bytes[16..32].try_into().expect("header"),
        revision: u64_at(bytes, 32),
        generation: u64_at(bytes, 40),
        reply_kind,
        shape_id: u32_at(bytes, 52),
        shape_version: u32_at(bytes, 56),
        unit_kind,
        unit_start: u64_at(bytes, 72),
        unit_end: u64_at(bytes, 80),
        row_count: u64_at(bytes, 88),
        total_count: u64_at(bytes, 96),
        diagnostic_total: u64_at(bytes, 104),
    };
    if header.unit_end.checked_sub(header.unit_start) != Some(header.row_count)
        || (header.total_count != UNKNOWN_TOTAL && header.unit_end > header.total_count)
    {
        return err("unit-range", "row unit is inconsistent");
    }
    if flags & FLAG_ERROR != 0
        && (reply_kind != ReplyKind::Diagnostics
            || flags & FLAG_FINAL == 0
            || flags & FLAG_CONTINUATION != 0)
    {
        return err(
            "invalid-diagnostics",
            "error reply must be complete diagnostics",
        );
    }
    let shape = registry
        .shape(header.shape_id)
        .ok_or_else(|| WireError::new("unknown-shape", format!("shape {}", header.shape_id)))?;
    if shape.version != header.shape_version {
        return err(
            "unknown-shape-version",
            format!("shape {} version {}", shape.name, header.shape_version),
        );
    }
    if shape.reply_kind != reply_kind || !shape.unit_kinds.contains(&unit_kind) {
        return err(
            "shape-mismatch",
            "reply kind or unit kind differs from registered shape",
        );
    }
    let section_count = u32_at(bytes, 60);
    let directory_end = HEADER_BYTES
        .checked_add(checked_size(u64::from(section_count), 24)?)
        .ok_or_else(|| WireError::new("section-bounds", "directory overflow"))?;
    if directory_end > bytes.len() {
        return err("section-bounds", "directory is truncated");
    }
    let mut sections = Vec::new();
    let mut seen = HashSet::new();
    let mut end = directory_end;
    for i in 0..section_count as usize {
        let slot = HEADER_BYTES + i * 24;
        let s = Section {
            kind: u32_at(bytes, slot),
            count: u32_at(bytes, slot + 4),
            offset: u64_at(bytes, slot + 8),
            length: u64_at(bytes, slot + 16),
        };
        if !(1..=9).contains(&s.kind) {
            return err("unknown-section-kind", format!("section {}", s.kind));
        }
        if !seen.insert(s.kind) {
            return err("duplicate-section", format!("section {}", s.kind));
        }
        if s.kind == 9 && (s.count != 1 || s.length != 24) {
            return err("invalid-bounds", "bounds must have one 24-byte record");
        }
        if !s.offset.is_multiple_of(8) {
            return err("section-alignment", "section does not start on eight bytes");
        }
        bytes_range(bytes, s.offset, s.length, "section-bounds")?;
        let start = usize_count(s.offset)?;
        if start < end {
            return err(
                "section-overlap",
                "sections must follow directory order without overlap",
            );
        }
        if bytes[end..start].iter().any(|&v| v != 0) {
            return err("nonzero-padding", "section gap contains nonzero bytes");
        }
        end = usize_count(
            s.offset
                .checked_add(s.length)
                .ok_or_else(|| WireError::new("section-bounds", "section range overflow"))?,
        )?;
        sections.push(s);
    }
    if bytes[end..].iter().any(|&v| v != 0) {
        return err("nonzero-padding", "trailing padding contains nonzero bytes");
    }
    let get = |kind: u32| sections.iter().find(|s| s.kind == kind).copied();
    let payload = |s: Section| bytes_range(bytes, s.offset, s.length, "section-bounds");
    let ds = get(1).ok_or_else(|| WireError::new("missing-section", "descriptors"))?;
    if ds.length != u64::from(ds.count) * DESCRIPTOR_BYTES as u64 {
        return err("descriptor-count", "descriptor section byte count differs");
    }
    let raw = payload(ds)?;
    let mut descriptors = Vec::new();
    for i in 0..ds.count as usize {
        let p = i * DESCRIPTOR_BYTES;
        descriptors.push(Descriptor {
            name_id: u32_at(raw, p),
            logical_type: named(
                LogicalType::try_from(u16_at(raw, p + 4)),
                "unknown-logical-type",
            )?,
            physical_encoding: named(Encoding::try_from(raw[p + 6]), "unknown-encoding")?,
            state_layers: raw[p + 7],
            type_param_a: u32_at(raw, p + 8),
            type_param_b: u32_at(raw, p + 12),
            dictionary_ref: u32_at(raw, p + 16),
            child_first: u32_at(raw, p + 20),
            child_count: u32_at(raw, p + 24),
            data_buffer: u32_at(raw, p + 28),
            length: u64_at(raw, p + 32),
            aux_buffer: u32_at(raw, p + 40),
            state_buffer: u32_at(raw, p + 44),
            state_widths: u16_at(raw, p + 48),
            gate_column: u16_at(raw, p + 50),
            gate_value: u32_at(raw, p + 52),
            refusal_ref: u32_at(raw, p + 56),
            reserved: u32_at(raw, p + 60),
        });
    }
    let ts = get(2).ok_or_else(|| WireError::new("missing-section", "top_level"))?;
    if ts.length != u64::from(ts.count) * 4 {
        return err("section-count", "top_level byte count differs");
    }
    let top_level: Vec<u32> = payload(ts)?.chunks_exact(4).map(|b| u32_at(b, 0)).collect();
    let bs = get(3).ok_or_else(|| WireError::new("missing-section", "buffers"))?;
    let raw = payload(bs)?;
    let table_end = checked_size(u64::from(bs.count), 16)?;
    if table_end > raw.len() {
        return err("buffer-bounds", "buffer table truncated");
    }
    let mut buffers = Vec::new();
    let mut ranges = Vec::new();
    for i in 0..bs.count as usize {
        let offset = u64_at(raw, i * 16);
        let length = u64_at(raw, i * 16 + 8);
        if !offset.is_multiple_of(8) {
            return err("buffer-alignment", "buffer is not eight-byte aligned");
        }
        let view = bytes_range(raw, offset, length, "buffer-bounds")?;
        if offset < table_end as u64 {
            return err("buffer-overlap", "buffer overlaps its table");
        }
        ranges.push((
            usize_count(offset)?,
            usize_count(
                offset
                    .checked_add(length)
                    .ok_or_else(|| WireError::new("buffer-bounds", "buffer range overflow"))?,
            )?,
        ));
        buffers.push(view);
    }
    ranges.sort_unstable();
    let mut cursor = table_end;
    for (start, finish) in ranges {
        if start < cursor {
            return err("buffer-overlap", "column buffers overlap");
        }
        if raw[cursor..start].iter().any(|&v| v != 0) {
            return err("nonzero-padding", "buffer gap contains nonzero bytes");
        }
        cursor = finish;
    }
    if raw[cursor..].iter().any(|&v| v != 0) {
        return err(
            "nonzero-padding",
            "buffer trailing padding contains nonzero bytes",
        );
    }
    let mut dictionary_refs = Vec::new();
    if let Some(s) = get(4) {
        if s.length != u64::from(s.count) * 88 {
            return err("section-count", "dictionary reference length");
        }
        let raw = payload(s)?;
        for chunk in raw.chunks_exact(88) {
            let kind = named(
                DictionaryKind::try_from(u32_at(chunk, 80)),
                "dictionary-kind",
            )?;
            let first_entry = u64_at(chunk, 64);
            let entry_count = u64_at(chunk, 72);
            if entry_count > 0 && first_entry.checked_add(entry_count - 1).is_none() {
                return err("dictionary-overflow", "canonical entry range overflow");
            }
            if u32_at(chunk, 84) != 0 {
                return err("dictionary-reference", "unknown dictionary flags");
            }
            dictionary_refs.push(DictionaryRef {
                dictionary_identity: chunk[..32].try_into().expect("record"),
                shard_identity: chunk[32..64].try_into().expect("record"),
                first_entry,
                entry_count,
                kind,
                flags: 0,
            });
        }
    }
    let bounds = if let Some(s) = get(9) {
        if s.count != 1 || s.length != 24 {
            return err("invalid-bounds", "bounds must have one 24-byte record");
        }
        let raw = payload(s)?;
        if u32_at(raw, 4) != 0 {
            return err("invalid-bounds", "bounds padding is nonzero");
        }
        let b = Bounds {
            kind: named(BoundsKind::try_from(u32_at(raw, 0)), "invalid-bounds")?,
            start: i64_at(raw, 8),
            end: i64_at(raw, 16),
        };
        if b.start > b.end {
            return err("invalid-bounds", "bounds reversed");
        }
        Some(b)
    } else {
        None
    };
    if reply_kind == ReplyKind::Diagnostics && bounds.is_some() {
        return err("invalid-bounds", "diagnostics cannot carry bounds");
    }
    if (shape.bounds == BoundsRule::Forbidden && bounds.is_some())
        || (shape.bounds == BoundsRule::Required && bounds.is_none())
    {
        return err("invalid-bounds", "bounds presence differs from shape");
    }
    if context.require_bounds_match && bounds != context.expected_bounds {
        return err(
            "bounds-mismatch",
            "reply bounds differ from normalized request",
        );
    }
    let token = |kind| -> Result<Option<&'a [u8]>> {
        get(kind)
            .map(|s| {
                if s.count != 1 {
                    return err("section-count", "continuation count must be one");
                }
                payload(s)
            })
            .transpose()
    };
    let continuation = token(6)?;
    let diagnostic_continuation = token(7)?;
    if continuation.is_some() != (flags & FLAG_CONTINUATION != 0) {
        return err(
            "continuation-flags",
            "continuation presence differs from flag",
        );
    }
    let mut extents = Vec::new();
    if let Some(s) = get(8) {
        if s.length != u64::from(s.count) * 40 {
            return err("invalid-extent", "extent section length");
        }
        for raw in payload(s)?.chunks_exact(40) {
            if raw[5] > 1 || u16_at(raw, 6) != 0 {
                return err("invalid-extent", "extent final or padding");
            }
            extents.push(Extent {
                column: u32_at(raw, 0),
                coordinate: named(Coordinate::try_from(raw[4]), "invalid-extent")?,
                final_extent: raw[5] == 1,
                index: u64_at(raw, 8),
                first: u64_at(raw, 16),
                end: u64_at(raw, 24),
                total: u64_at(raw, 32),
            });
        }
    }
    if descriptors.len() != shape.columns.len() {
        return err("shape-mismatch", "descriptor count differs from shape");
    }
    let mut reply = ValidatedReply {
        header,
        sections: sections.clone(),
        descriptors,
        top_level,
        buffers,
        dictionary_refs,
        diagnostics: None,
        continuation,
        diagnostic_continuation,
        extents,
        bounds,
        bytes,
        parents: Vec::new(),
        run_ends: Vec::new(),
    };
    validate_columns(&mut reply, shape)?;
    if let Some(s) = get(5) {
        if embedded || reply_kind == ReplyKind::Diagnostics || s.count != 1 {
            return err("invalid-diagnostics", "diagnostics embedding is invalid");
        }
        let diagnostic_bytes = payload(s)?;
        if diagnostic_bytes.len() < HEADER_BYTES
            || u32_at(diagnostic_bytes, 48) != ReplyKind::Diagnostics as u32
        {
            return err(
                "invalid-diagnostics",
                "embedded payload must be a complete diagnostics reply",
            );
        }
        let d = read_inner(diagnostic_bytes, registry, ReadContext::default(), true)?;
        if d.header.reply_kind != ReplyKind::Diagnostics
            || d.header.query_id != reply.header.query_id
            || d.header.campaign_id != reply.header.campaign_id
            || d.header.revision != reply.header.revision
            || d.header.generation != reply.header.generation
        {
            return err(
                "diagnostics-identity",
                "embedded diagnostics do not answer this query",
            );
        }
        if d.header.total_count != reply.header.diagnostic_total {
            return err(
                "invalid-diagnostics",
                "diagnostic stream total differs from parent",
            );
        }
        reply.diagnostics = Some(Box::new(d));
    }
    Ok(reply)
}

fn buffer<'a>(r: &ValidatedReply<'a>, index: u32) -> Result<&'a [u8]> {
    r.buffers
        .get(index as usize)
        .copied()
        .ok_or_else(|| WireError::new("buffer-index", format!("buffer {index}")))
}
fn width(d: &Descriptor) -> Result<u64> {
    let w = match d.logical_type {
        LogicalType::Bool => 1,
        LogicalType::Unsigned | LogicalType::Signed | LogicalType::Enum | LogicalType::Refusal => {
            d.type_param_a as u64
        }
        LogicalType::Safe53 | LogicalType::Hour | LogicalType::NativeDate => 64,
        LogicalType::WideInt => 128,
        LogicalType::FixedPoint => (d.type_param_a & 255) as u64,
        LogicalType::Float => d.type_param_a as u64,
        _ => 0,
    };
    if ![0, 1, 8, 16, 32, 64, 128].contains(&w) {
        return err("invalid-type-parameter", "numeric width is unknown");
    }
    Ok(w)
}
fn packed_length(count: u64, bits: u64) -> Result<usize> {
    usize_count(
        count
            .checked_mul(bits)
            .and_then(|n| n.checked_add(7))
            .ok_or_else(|| WireError::new("size-overflow", "packed length overflow"))?
            / 8,
    )
}
fn exact_length(bytes: &[u8], expected: usize, name: &'static str) -> Result<()> {
    if bytes.len() != expected {
        return err(name, format!("{} bytes; expected {expected}", bytes.len()));
    }
    Ok(())
}
fn state_bits(d: &Descriptor, layer: usize) -> Result<u64> {
    match (d.state_widths >> (layer * 4)) & 15 {
        0 => Ok(1),
        1 => Ok(8),
        2 => Ok(16),
        3 => Ok(32),
        4 => Ok(64),
        _ => err("invalid-state-width", "unknown physical state width"),
    }
}
fn state_index(d: &Descriptor, layer: usize) -> Result<u32> {
    d.state_buffer
        .checked_add((d.state_layers & ((1 << layer) - 1)).count_ones())
        .ok_or_else(|| WireError::new("buffer-index", "state buffer index overflow"))
}
fn read_bits(bytes: &[u8], index: u64, bits: u64) -> Result<u64> {
    #[cfg(test)]
    VALIDATION_READ_BUDGET.with(|budget| {
        if let Some(remaining) = budget.get() {
            assert!(remaining > 0, "validation expanded compact logical rows");
            budget.set(Some(remaining - 1));
        }
    });
    if [8, 16, 32, 64].contains(&bits) {
        let start = checked_size(index, bits / 8)?;
        let end = start
            .checked_add((bits / 8) as usize)
            .ok_or_else(|| WireError::new("size-overflow", "numeric coordinate overflow"))?;
        let raw = bytes
            .get(start..end)
            .ok_or_else(|| WireError::new("buffer-length", "value out of bounds"))?;
        return Ok(match bits {
            8 => u64::from(raw[0]),
            16 => u64::from(u16_at(raw, 0)),
            32 => u64::from(u32_at(raw, 0)),
            64 => u64_at(raw, 0),
            _ => unreachable!(),
        });
    }
    let position = index
        .checked_mul(bits)
        .ok_or_else(|| WireError::new("size-overflow", "packed coordinate overflow"))?;
    let mut out = 0;
    for bit in 0..bits {
        let p = position
            .checked_add(bit)
            .ok_or_else(|| WireError::new("size-overflow", "packed coordinate overflow"))?;
        let b = *bytes
            .get(usize_count(p / 8)?)
            .ok_or_else(|| WireError::new("buffer-length", "packed value out of bounds"))?;
        out |= u64::from((b >> (p % 8)) & 1) << bit;
    }
    Ok(out)
}

#[cfg(test)]
thread_local! {
    static VALIDATION_READ_BUDGET: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
}

// None represents a completely active column. Struct children share unchanged
// ranges, so nested descriptors do not copy activity once per ancestor.
type Activity = Option<Rc<Vec<(u64, u64)>>>;

fn activity_ranges(activity: &Activity, length: u64) -> impl Iterator<Item = (u64, u64)> + '_ {
    activity
        .iter()
        .flat_map(|ranges| ranges.iter().copied())
        .chain(activity.is_none().then_some((0, length)))
}

fn append_activity(ranges: &mut Vec<(u64, u64)>, first: u64, end: u64) {
    if first == end {
        return;
    }
    if let Some(last) = ranges.last_mut()
        && last.1 == first
    {
        last.1 = end;
        return;
    }
    ranges.push((first, end));
}

fn compact_activity(ranges: Vec<(u64, u64)>, length: u64) -> Activity {
    if ranges.as_slice() == [(0, length)] || length == 0 {
        None
    } else {
        Some(Rc::new(ranges))
    }
}

fn inherited_activity(
    r: &ValidatedReply<'_>,
    index: usize,
    activity: &[Activity],
) -> Result<Activity> {
    let Some(parent) = r.parents[index].map(|p| p as usize) else {
        return Ok(None);
    };
    let inherited = &activity[parent];
    if inherited.is_none() || r.descriptors[parent].logical_type == LogicalType::Struct {
        return Ok(inherited.clone());
    }
    let offsets = buffer(r, r.descriptors[parent].aux_buffer)?;
    let mut ranges = Vec::new();
    for (first, end) in activity_ranges(inherited, r.column_length(parent)) {
        append_activity(
            &mut ranges,
            u64_at(offsets, checked_size(first, 8)?),
            u64_at(offsets, checked_size(end, 8)?),
        );
    }
    Ok(compact_activity(ranges, r.column_length(index)))
}

// Visit encoded spans intersecting activity, never expanding a run into rows.
fn visit_active_spans(
    r: &ValidatedReply<'_>,
    index: usize,
    activity: &Activity,
    mut visit: impl FnMut(u64, u64, u64) -> Result<()>,
) -> Result<()> {
    for (mut first, end) in activity_ranges(activity, r.column_length(index)) {
        if r.descriptors[index].physical_encoding == Encoding::RunLength {
            let ends = &r.run_ends[index];
            let mut run = ends.partition_point(|&end| end <= first);
            while first < end {
                let finish = ends[run].min(end);
                visit(first, finish, run as u64)?;
                first = finish;
                run += 1;
            }
        } else {
            for item in first..end {
                visit(item, item + 1, item)?;
            }
        }
    }
    Ok(())
}

fn gate_activity(r: &ValidatedReply<'_>, index: usize, inherited: Activity) -> Result<Activity> {
    let d = &r.descriptors[index];
    if d.gate_column == u16::MAX {
        return Ok(inherited);
    }
    let parent = r.parents[index].expect("validated gate") as usize;
    let tag = r.descriptors[parent].child_first as usize + d.gate_column as usize;
    let td = &r.descriptors[tag];
    let bits = if td.physical_encoding == Encoding::BitPacked {
        u64::from(td.type_param_b)
    } else {
        width(td)?
    };
    let data = buffer(r, td.data_buffer)?;
    let mut ranges = Vec::new();
    visit_active_spans(r, tag, &inherited, |first, end, value| {
        if read_bits(data, value, bits)? == u64::from(d.gate_value) {
            append_activity(&mut ranges, first, end);
        }
        Ok(())
    })?;
    Ok(compact_activity(ranges, r.column_length(index)))
}

fn state_activity(r: &ValidatedReply<'_>, index: usize, inherited: Activity) -> Result<Activity> {
    let d = &r.descriptors[index];
    if d.state_layers == 0 {
        return Ok(inherited);
    }
    let mut layers = [None; 4];
    for (layer, slot) in layers.iter_mut().enumerate() {
        if d.state_layers & (1 << layer) != 0 {
            *slot = Some((buffer(r, state_index(d, layer)?)?, state_bits(d, layer)?));
        }
    }
    if layers
        .iter()
        .flatten()
        .all(|(data, _)| data.iter().all(|&byte| byte == 0))
    {
        return Ok(inherited);
    }
    let mut ranges = Vec::new();
    for (first, end) in activity_ranges(&inherited, r.column_length(index)) {
        let mut active_first = None;
        for item in first..end {
            let mut active = true;
            for (layer, state) in layers.iter().enumerate() {
                let Some((data, bits)) = state else {
                    continue;
                };
                let code = read_bits(data, item, *bits)?;
                match layer {
                    0 | 1 if code > 2 => {
                        return err("invalid-state-code", "recording/absence code outside 0..2");
                    }
                    2 if code > 1 => {
                        return err("invalid-state-code", "knownness code outside 0..1");
                    }
                    3 if code > 0 => {
                        let reference = &r.dictionary_refs[d.refusal_ref as usize];
                        let entry = code - 1;
                        if entry >= reference.entry_count {
                            return err("dictionary-code", "refusal-state code outside shard");
                        }
                        reference.first_entry.checked_add(entry).ok_or_else(|| {
                            WireError::new(
                                "dictionary-overflow",
                                "refusal canonical index overflow",
                            )
                        })?;
                    }
                    _ => {}
                }
                if code != 0 {
                    active = false;
                    break;
                }
            }
            if active {
                active_first.get_or_insert(item);
            } else if let Some(start) = active_first.take() {
                append_activity(&mut ranges, start, item);
            }
        }
        if let Some(start) = active_first {
            append_activity(&mut ranges, start, end);
        }
    }
    Ok(compact_activity(ranges, r.column_length(index)))
}

fn validate_text(bytes: &[u8], extent: Option<&Extent>) -> Result<()> {
    let mut start = 0;
    if extent.is_some_and(|e| e.first > 0) {
        while start < bytes.len() && bytes[start] & 0xc0 == 0x80 {
            start += 1;
        }
        if start > 3 {
            return err(
                "invalid-utf8",
                "text extent has too many leading continuations",
            );
        }
    }
    match std::str::from_utf8(&bytes[start..]) {
        Ok(_) => Ok(()),
        Err(error) if error.error_len().is_none() && extent.is_some_and(|e| !e.final_extent) => {
            let remaining = &bytes[start + error.valid_up_to()..];
            let width = match remaining[0] {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => unreachable!("UTF-8 validator accepted a truncated scalar prefix"),
            };
            let needed = (width - remaining.len()) as u64;
            if extent.is_some_and(|e| e.total != UNKNOWN_TOTAL && e.total - e.end < needed) {
                return err(
                    "invalid-utf8",
                    "text extent cannot complete its trailing scalar",
                );
            }
            Ok(())
        }
        Err(_) => err("invalid-utf8", "text value is not UTF-8"),
    }
}
impl ValidatedReply<'_> {
    pub fn state_code(&self, column: usize, index: u64, layer: usize) -> Result<u64> {
        let d = self
            .descriptors
            .get(column)
            .ok_or_else(|| WireError::new("descriptor-index", "state column does not exist"))?;
        if layer >= 4 || d.state_layers & (1 << layer) == 0 || index >= self.column_length(column) {
            return err("descriptor-index", "state coordinate does not exist");
        }
        read_bits(
            buffer(self, state_index(d, layer)?)?,
            index,
            state_bits(d, layer)?,
        )
    }
    pub fn unsigned(&self, column: usize, index: u64) -> Result<u64> {
        let d = self
            .descriptors
            .get(column)
            .ok_or_else(|| WireError::new("descriptor-index", "column does not exist"))?;
        if index >= self.column_length(column) {
            return err("descriptor-index", "element outside column");
        }
        let value_index = if d.physical_encoding == Encoding::RunLength {
            self.run_ends[column].partition_point(|&end| end <= index) as u64
        } else {
            index
        };
        let bits = if d.physical_encoding == Encoding::BitPacked {
            u64::from(d.type_param_b)
        } else {
            width(d)?
        };
        if bits > 64 {
            return err("invalid-type-parameter", "wide value requires limbs");
        }
        read_bits(buffer(self, d.data_buffer)?, value_index, bits)
    }
    pub fn signed(&self, column: usize, index: u64) -> Result<i64> {
        let d = self
            .descriptors
            .get(column)
            .ok_or_else(|| WireError::new("descriptor-index", "signed column does not exist"))?;
        let bits = if d.physical_encoding == Encoding::BitPacked {
            d.type_param_b as u64
        } else {
            width(d)?
        };
        if bits == 0 || bits > 64 {
            return err(
                "invalid-type-parameter",
                "signed accessor requires an integer up to 64 bits",
            );
        }
        let raw = self.unsigned(column, index)?;
        Ok(if bits == 64 {
            raw as i64
        } else {
            ((raw << (64 - bits)) as i64) >> (64 - bits)
        })
    }
    pub fn limbs(&self, column: usize, index: u64) -> Result<(u64, u64)> {
        let d = self
            .descriptors
            .get(column)
            .ok_or_else(|| WireError::new("descriptor-index", "wide column does not exist"))?;
        if width(d)? != 128 {
            return err(
                "invalid-type-parameter",
                "limb accessor requires a 128-bit column",
            );
        }
        if index >= self.column_length(column) {
            return err("descriptor-index", "wide element outside column");
        }
        let raw = buffer(self, d.data_buffer)?;
        let (value_index, length) = if d.physical_encoding == Encoding::RunLength {
            (
                self.run_ends[column].partition_point(|&end| end <= index) as u64,
                self.run_ends[column].len() as u64,
            )
        } else {
            (index, self.column_length(column))
        };
        let low = checked_size(value_index, 8)?;
        let high = checked_size(
            length
                .checked_add(value_index)
                .ok_or_else(|| WireError::new("size-overflow", "wide coordinate overflow"))?,
            8,
        )?;
        Ok((u64_at(raw, low), u64_at(raw, high)))
    }
    pub fn value_bytes(&self, column: usize, index: u64) -> Result<&[u8]> {
        let d = self
            .descriptors
            .get(column)
            .ok_or_else(|| WireError::new("descriptor-index", "byte column does not exist"))?;
        if !matches!(d.logical_type, LogicalType::Bytes | LogicalType::Text)
            || index >= self.column_length(column)
        {
            return err("descriptor-index", "byte element outside column");
        }
        let offsets = buffer(self, d.aux_buffer)?;
        let p = checked_size(index, 8)?;
        bytes_range(
            buffer(self, d.data_buffer)?,
            u64_at(offsets, p),
            u64_at(offsets, p + 8) - u64_at(offsets, p),
            "offsets-bounds",
        )
    }
    fn parent_coordinate(&self, column: usize, index: u64) -> Result<Option<(usize, u64)>> {
        let Some(parent) = self.parents[column] else {
            return Ok(None);
        };
        let p = parent as usize;
        let pd = &self.descriptors[p];
        if pd.logical_type == LogicalType::Struct {
            return Ok(Some((p, index)));
        }
        let offsets = buffer(self, pd.aux_buffer)?;
        let count = usize_count(self.column_length(p))?;
        let mut lo = 0;
        let mut hi = count;
        while lo < hi {
            let m = lo + (hi - lo) / 2;
            if u64_at(offsets, (m + 1) * 8) <= index {
                lo = m + 1
            } else {
                hi = m
            }
        }
        if lo == count {
            return err("offsets-bounds", "child coordinate has no list occurrence");
        }
        Ok(Some((p, lo as u64)))
    }
    /// State/gate inactivity propagates through arbitrary nesting without recursion.
    pub fn meaningful(&self, column: usize, index: u64) -> Result<bool> {
        if column >= self.descriptors.len() || index >= self.column_length(column) {
            return err("descriptor-index", "meaningful coordinate outside column");
        }
        let mut path = Vec::new();
        let mut current = Some((column, index));
        while let Some((c, i)) = current {
            path.push((c, i));
            current = self.parent_coordinate(c, i)?;
        }
        for (c, i) in path.into_iter().rev() {
            let d = &self.descriptors[c];
            if d.gate_column != u16::MAX {
                let parent = self.parents[c]
                    .ok_or_else(|| WireError::new("union-gate", "gate without parent"))?
                    as usize;
                let discriminator =
                    self.descriptors[parent].child_first as usize + d.gate_column as usize;
                if self.unsigned(discriminator, i)? != u64::from(d.gate_value) {
                    return Ok(false);
                }
            }
            for layer in 0..4 {
                if d.state_layers & (1 << layer) != 0 && self.state_code(c, i, layer)? != 0 {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    /// Original occurrence coordinates: root row, then each list-element position.
    pub fn original_occurrence(&self, column: usize, index: u64) -> Result<Vec<(u32, u64)>> {
        if column >= self.descriptors.len() || index >= self.column_length(column) {
            return err("descriptor-index", "occurrence outside column");
        }
        let mut out = Vec::new();
        let mut c = column;
        let mut i = index;
        while let Some((p, parent_index)) = self.parent_coordinate(c, i)? {
            if self.descriptors[p].logical_type == LogicalType::List {
                let offsets = buffer(self, self.descriptors[p].aux_buffer)?;
                let first = self
                    .extents
                    .iter()
                    .find(|e| e.column == p as u32 && e.index == parent_index)
                    .map_or(0, |e| e.first);
                let original = first
                    .checked_add(i - u64_at(offsets, checked_size(parent_index, 8)?))
                    .ok_or_else(|| {
                        WireError::new("size-overflow", "original list position overflow")
                    })?;
                out.push((p as u32, original));
            }
            c = p;
            i = parent_index;
        }
        out.push((
            c as u32,
            self.header
                .unit_start
                .checked_add(i)
                .ok_or_else(|| WireError::new("size-overflow", "original row overflow"))?,
        ));
        out.reverse();
        Ok(out)
    }
}

fn validate_columns(r: &mut ValidatedReply<'_>, shape: &Shape) -> Result<()> {
    let n = r.descriptors.len();
    r.parents = vec![None; n];
    r.run_ends = vec![Vec::new(); n];
    let mut roots = HashSet::new();
    for &index in &r.top_level {
        if index as usize >= n || !roots.insert(index) {
            return err("descriptor-index", "invalid top-level descriptor");
        }
    }
    for (index, d) in r.descriptors.iter().enumerate() {
        if d.reserved != 0 {
            return err(
                "descriptor-reserved",
                "descriptor reserved field is nonzero",
            );
        }
        if (roots.contains(&(index as u32))) != (d.length == TOP_LEVEL_LENGTH) {
            return err(
                "descriptor-length",
                "top-level length sentinel is inconsistent",
            );
        }
        if d.child_count == 0 {
            if d.child_first != NONE_INDEX {
                return err("descriptor-index", "empty child range requires sentinel");
            }
        } else {
            let end = d
                .child_first
                .checked_add(d.child_count)
                .ok_or_else(|| WireError::new("descriptor-index", "child range overflow"))?;
            if end as usize > n {
                return err("descriptor-index", "child descriptor outside table");
            }
            for c in d.child_first..end {
                if r.parents[c as usize].replace(index as u32).is_some() {
                    return err("descriptor-parent", "child has multiple parents");
                }
            }
        }
    }
    // Kahn traversal also finds disconnected cycles. No call-stack depth limit.
    let mut queue: Vec<usize> = r
        .parents
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.is_none().then_some(i))
        .collect();
    let mut cursor = 0;
    while cursor < queue.len() {
        let d = &r.descriptors[queue[cursor]];
        if d.child_count > 0 {
            queue.extend((d.child_first..d.child_first + d.child_count).map(|i| i as usize));
        }
        cursor += 1;
    }
    if queue.len() != n {
        return err("descriptor-cycle", "descriptor graph contains a cycle");
    }
    if r.parents
        .iter()
        .enumerate()
        .any(|(i, p)| p.is_none() != roots.contains(&(i as u32)))
    {
        return err("descriptor-parent", "every root must be top level");
    }
    for index in queue.iter().copied() {
        let d = &r.descriptors[index];
        let s = &shape.columns[index];
        if d.name_id as usize >= shape.names.len() {
            return err(
                "shape-mismatch",
                format!("column {index} name outside static table"),
            );
        }
        let length = r.column_length(index);
        let _ = usize_count(length)?;
        if d.state_layers & !15 != 0 {
            return err("invalid-state-layers", "reserved state layers");
        }
        if d.state_layers == 0 && d.state_buffer != NONE_INDEX {
            return err(
                "invalid-state-layers",
                "stateless column names state buffer",
            );
        }
        for layer in 0..4 {
            let bits = state_bits(d, layer)?;
            if d.state_layers & (1 << layer) != 0 {
                exact_length(
                    buffer(r, state_index(d, layer)?)?,
                    packed_length(length, bits)?,
                    "buffer-length",
                )?;
            } else if ((d.state_widths >> (layer * 4)) & 15) != 0 {
                return err("invalid-state-width", "absent layer has physical width");
            }
        }
        if d.state_layers & 8 != 0 {
            let refusal = r
                .dictionary_refs
                .get(d.refusal_ref as usize)
                .ok_or_else(|| {
                    WireError::new("refusal-reference", "L4 requires a refusal shard")
                })?;
            if refusal.kind != DictionaryKind::Refusal {
                return err("refusal-reference", "L4 shard has wrong kind");
            }
        } else if d.refusal_ref != NONE_INDEX {
            return err("refusal-reference", "column without L4 names refusal shard");
        }
        if d.dictionary_ref != NONE_INDEX {
            let reference = r
                .dictionary_refs
                .get(d.dictionary_ref as usize)
                .ok_or_else(|| {
                    WireError::new("dictionary-reference", "dictionary reference outside table")
                })?;
            if !matches!(d.logical_type, LogicalType::Enum | LogicalType::Refusal)
                || s.static_enum.is_some()
            {
                return err(
                    "dictionary-reference",
                    "dictionary binding on non-dictionary column",
                );
            }
            if s.dictionary_kind.is_some_and(|kind| reference.kind != kind)
                || (d.logical_type == LogicalType::Refusal
                    && reference.kind != DictionaryKind::Refusal)
            {
                return err("dictionary-kind", "column shard has wrong kind");
            }
        }
        if d.gate_column != u16::MAX {
            let parent = r.parents[index]
                .ok_or_else(|| WireError::new("union-gate", "union payload must have a parent"))?
                as usize;
            let pd = &r.descriptors[parent];
            if pd.logical_type != LogicalType::Struct || u32::from(d.gate_column) >= pd.child_count
            {
                return err("union-gate", "gate must name a struct sibling");
            }
            let tag_index = (pd.child_first + u32::from(d.gate_column)) as usize;
            let tag = &r.descriptors[tag_index];
            if tag.logical_type != LogicalType::Enum
                || tag.dictionary_ref != NONE_INDEX
                || tag.gate_column != u16::MAX
                || tag.state_layers != 0
                || shape
                    .names
                    .get(tag.name_id as usize)
                    .is_none_or(|name| name.split('.').next_back() != Some("$kind"))
                || shape.columns[tag_index]
                    .static_enum
                    .as_ref()
                    .is_none_or(|variants| d.gate_value as usize >= variants.len())
            {
                return err(
                    "union-gate",
                    "gate must reference the static $kind discriminator",
                );
            }
        } else if d.gate_value != 0 {
            return err("union-gate", "ungated column has a gate value");
        }
        match d.logical_type {
            LogicalType::List | LogicalType::Bytes | LogicalType::Text => {
                if d.physical_encoding != Encoding::Plain
                    || d.type_param_b != 0
                    || (d.logical_type == LogicalType::List
                        && (d.type_param_a != 64 || d.child_count != 1))
                    || (d.logical_type != LogicalType::List
                        && (d.type_param_a != 0 || d.child_count != 0))
                {
                    return err(
                        "invalid-type-parameter",
                        "invalid variable-length column layout",
                    );
                }
                let offsets = buffer(r, d.aux_buffer)?;
                let entries = length
                    .checked_add(1)
                    .ok_or_else(|| WireError::new("size-overflow", "offset count overflow"))?;
                exact_length(offsets, checked_size(entries, 8)?, "offsets-length")?;
                let target = if d.logical_type == LogicalType::List {
                    if d.data_buffer != NONE_INDEX {
                        return err("buffer-index", "list cannot have a data buffer");
                    }
                    r.column_length(d.child_first as usize)
                } else {
                    buffer(r, d.data_buffer)?.len() as u64
                };
                let mut previous = 0;
                for (i, chunk) in offsets.chunks_exact(8).enumerate() {
                    let value = u64_at(chunk, 0);
                    if (i == 0 && value != 0) || value < previous {
                        return err("offsets-order", "offsets must start at zero and increase");
                    }
                    if value > target {
                        return err("offsets-bounds", "offset exceeds local child/data length");
                    }
                    previous = value;
                }
                if previous != target {
                    return err(
                        "offsets-bounds",
                        "final offset must equal local child/data length",
                    );
                }
            }
            LogicalType::Struct => {
                if d.physical_encoding != Encoding::Plain
                    || d.type_param_a != 0
                    || d.type_param_b != 0
                    || d.data_buffer != NONE_INDEX
                    || d.aux_buffer != NONE_INDEX
                {
                    return err("invalid-type-parameter", "invalid struct layout");
                }
                if d.child_count > 0 {
                    for child in d.child_first..d.child_first + d.child_count {
                        if r.column_length(child as usize) != length {
                            return err(
                                "descriptor-length",
                                "struct child length differs from parent",
                            );
                        }
                    }
                }
            }
            _ => {
                if d.child_count != 0 {
                    return err("descriptor-index", "scalar column has child descriptors");
                }
                let bits = width(d)?;
                if matches!(
                    d.logical_type,
                    LogicalType::Unsigned
                        | LogicalType::Signed
                        | LogicalType::Enum
                        | LogicalType::Refusal
                ) && ![8, 16, 32, 64].contains(&bits)
                {
                    return err("invalid-type-parameter", "integer/code width invalid");
                }
                if d.logical_type == LogicalType::WideInt && d.type_param_a > 1 {
                    return err("invalid-type-parameter", "wide integer signedness invalid");
                }
                if d.logical_type == LogicalType::Float && ![32, 64].contains(&bits) {
                    return err("invalid-type-parameter", "float width invalid");
                }
                if d.logical_type == LogicalType::FixedPoint
                    && ((![8, 16, 32, 64, 128].contains(&bits))
                        || d.type_param_a & !511 != 0
                        || ![2, 10].contains(&(d.type_param_b & 255))
                        || d.type_param_b & !65535 != 0)
                {
                    return err(
                        "invalid-type-parameter",
                        "fixed-point base/radix/scale invalid",
                    );
                }
                let physical_bits = if d.physical_encoding == Encoding::BitPacked {
                    if !(1..=64).contains(&d.type_param_b)
                        || u64::from(d.type_param_b) > bits
                        || matches!(
                            d.logical_type,
                            LogicalType::Float
                                | LogicalType::Safe53
                                | LogicalType::WideInt
                                | LogicalType::FixedPoint
                                | LogicalType::Hour
                                | LogicalType::NativeDate
                        )
                    {
                        return err("invalid-type-parameter", "bit-packed width/type invalid");
                    }
                    u64::from(d.type_param_b)
                } else {
                    bits
                };
                if d.physical_encoding == Encoding::Dictionary
                    && !matches!(d.logical_type, LogicalType::Enum | LogicalType::Refusal)
                {
                    return err("unknown-encoding", "dictionary encoding requires code type");
                }
                let values = if d.physical_encoding == Encoding::RunLength {
                    let runs = buffer(r, d.aux_buffer)?;
                    if runs.len() % 4 != 0 {
                        return err("run-length", "run-length buffer is not u32 values");
                    }
                    let mut sum = 0u64;
                    let mut ends = Vec::new();
                    for chunk in runs.chunks_exact(4) {
                        let run = u32_at(chunk, 0);
                        if run == 0 {
                            return err("run-length", "zero-length run");
                        }
                        sum = sum
                            .checked_add(u64::from(run))
                            .ok_or_else(|| WireError::new("run-length", "run sum overflow"))?;
                        ends.push(sum);
                    }
                    if sum != length {
                        return err("run-length", "run lengths do not sum to column length");
                    }
                    let count = ends.len() as u64;
                    r.run_ends[index] = ends;
                    count
                } else {
                    if d.aux_buffer != NONE_INDEX {
                        return err(
                            "buffer-index",
                            "scalar column has unexpected auxiliary buffer",
                        );
                    }
                    length
                };
                exact_length(
                    buffer(r, d.data_buffer)?,
                    packed_length(values, physical_bits)?,
                    "buffer-length",
                )?;
            }
        }
    }
    if r.top_level != shape.top_level {
        return err(
            "shape-mismatch",
            "top-level order differs from registered shape",
        );
    }
    for (index, (d, s)) in r.descriptors.iter().zip(&shape.columns).enumerate() {
        let e = &s.descriptor;
        if d.name_id != e.name_id
            || d.logical_type != e.logical_type
            || d.physical_encoding != e.physical_encoding
            || d.state_layers != e.state_layers
            || d.type_param_a != e.type_param_a
            || d.type_param_b != e.type_param_b
            || d.child_first != e.child_first
            || d.child_count != e.child_count
            || d.gate_column != e.gate_column
            || d.gate_value != e.gate_value
        {
            return err(
                "shape-mismatch",
                format!("column {index} differs from registered contract"),
            );
        }
    }
    validate_extents(r)?;
    let value_extents: HashMap<_, _> = r
        .extents
        .iter()
        .map(|e| ((e.column as usize, e.index), e))
        .collect();
    // Activity is propagated once, outermost first. Lists map active ranges
    // through monotonic offsets; encoded values only visit their intersections.
    let mut activity: Vec<Activity> = vec![None; n];
    for index in queue {
        let d = &r.descriptors[index];
        let s = &shape.columns[index];
        let inherited = inherited_activity(r, index, &activity)?;
        let gated = gate_activity(r, index, inherited)?;
        activity[index] = state_activity(r, index, gated)?;
        let has_value_checks = matches!(
            d.logical_type,
            LogicalType::Enum | LogicalType::Refusal | LogicalType::Safe53 | LogicalType::Text
        ) || (d.logical_type == LogicalType::List
            && s.fixed_list_length.is_some());
        if !has_value_checks {
            continue;
        }
        let data = if matches!(
            d.logical_type,
            LogicalType::Enum | LogicalType::Refusal | LogicalType::Safe53
        ) {
            Some(buffer(r, d.data_buffer)?)
        } else {
            None
        };
        let bits = if data.is_none() {
            0
        } else if d.physical_encoding == Encoding::BitPacked {
            u64::from(d.type_param_b)
        } else {
            width(d)?
        };
        let mut last_value = None;
        visit_active_spans(r, index, &activity[index], |item, _, value| {
            // One encoded value may intersect multiple disjoint activity ranges.
            if last_value == Some(value) {
                return Ok(());
            }
            last_value = Some(value);
            match d.logical_type {
                LogicalType::Enum | LogicalType::Refusal => {
                    let code = read_bits(data.expect("code data"), value, bits)?;
                    if let Some(variants) = &s.static_enum {
                        if code >= variants.len() as u64 {
                            return err(
                                "dictionary-code",
                                "static enum code outside generated table",
                            );
                        }
                    } else {
                        let reference = r
                            .dictionary_refs
                            .get(d.dictionary_ref as usize)
                            .ok_or_else(|| {
                                WireError::new(
                                    "dictionary-reference",
                                    "meaningful code requires a shard",
                                )
                            })?;
                        if code >= reference.entry_count {
                            return err("dictionary-code", "code outside shard");
                        }
                        reference.first_entry.checked_add(code).ok_or_else(|| {
                            WireError::new("dictionary-overflow", "canonical entry index overflow")
                        })?;
                    }
                }
                LogicalType::Safe53 => {
                    let value = f64::from_bits(read_bits(data.expect("safe53 data"), value, bits)?);
                    if !value.is_finite()
                        || value.trunc() != value
                        || value.abs() > 9_007_199_254_740_992.0
                    {
                        return err("invalid-safe53", "safe53 requires exact bounded integer");
                    }
                }
                LogicalType::Text => {
                    validate_text(
                        r.value_bytes(index, item)?,
                        value_extents.get(&(index, item)).copied(),
                    )?;
                }
                LogicalType::List => {
                    if let Some(fixed) = s.fixed_list_length {
                        let length = if let Some(extent) = value_extents.get(&(index, item)) {
                            extent.total
                        } else {
                            let offsets = buffer(r, d.aux_buffer)?;
                            let p = checked_size(item, 8)?;
                            u64_at(offsets, p + 8) - u64_at(offsets, p)
                        };
                        if length != fixed {
                            return err(
                                "descriptor-length",
                                "fixed list length differs from contract",
                            );
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn validate_extents(r: &ValidatedReply<'_>) -> Result<()> {
    let mut seen = HashSet::new();
    for e in &r.extents {
        if !seen.insert((e.column, e.index)) {
            return err("duplicate-extent", "one extent per value");
        }
        let d = r
            .descriptors
            .get(e.column as usize)
            .ok_or_else(|| WireError::new("invalid-extent", "extent descriptor outside table"))?;
        let correct = match d.logical_type {
            LogicalType::List => e.coordinate == Coordinate::Element,
            LogicalType::Bytes | LogicalType::Text => e.coordinate == Coordinate::Byte,
            _ => false,
        };
        if !correct
            || e.index >= r.column_length(e.column as usize)
            || e.end < e.first
            || (e.total != UNKNOWN_TOTAL && e.end > e.total)
        {
            return err("invalid-extent", "extent coordinate/range is invalid");
        }
        if e.total != UNKNOWN_TOTAL && e.final_extent != (e.end == e.total) {
            return err(
                "invalid-extent",
                "final extent marker differs from its total",
            );
        }
        let offsets = buffer(r, d.aux_buffer)?;
        let p = checked_size(e.index, 8)?;
        if e.end - e.first != u64_at(offsets, p + 8) - u64_at(offsets, p) {
            return err("invalid-extent", "extent length differs from local value");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::write;
    fn registry(descriptors: &[Descriptor]) -> Vec<Shape> {
        vec![Shape {
            id: 123,
            version: 1,
            name: "synthetic".into(),
            reply_kind: ReplyKind::Series,
            unit_kinds: vec![UnitKind::Page],
            names: (0..descriptors.len())
                .map(|i| format!("column{i}"))
                .collect(),
            columns: descriptors
                .iter()
                .cloned()
                .map(|descriptor| ColumnSpec {
                    descriptor,
                    static_enum: None,
                    fixed_list_length: None,
                    dictionary_kind: None,
                    state_mapping: StateMapping::default(),
                })
                .collect(),
            top_level: vec![0],
            bounds: BoundsRule::Optional,
        }]
    }

    fn with_read_budget<T>(limit: usize, run: impl FnOnce() -> T) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                VALIDATION_READ_BUDGET.with(|budget| budget.set(None));
            }
        }
        VALIDATION_READ_BUDGET.with(|budget| {
            assert_eq!(budget.get(), None);
            budget.set(Some(limit));
        });
        let _reset = Reset;
        run()
    }

    #[test]
    fn compact_gated_runs_validate_encoded_values_even_at_the_final_row() {
        let length = u64::from(u32::MAX);
        let descriptors = vec![
            Descriptor {
                logical_type: LogicalType::Struct,
                type_param_b: 0,
                child_first: 1,
                child_count: 2,
                ..Descriptor::default()
            },
            Descriptor {
                name_id: 1,
                logical_type: LogicalType::Enum,
                physical_encoding: Encoding::RunLength,
                type_param_a: 8,
                type_param_b: 0,
                data_buffer: 0,
                aux_buffer: 1,
                length,
                ..Descriptor::default()
            },
            Descriptor {
                name_id: 2,
                logical_type: LogicalType::Enum,
                physical_encoding: Encoding::RunLength,
                type_param_a: 8,
                type_param_b: 0,
                data_buffer: 2,
                aux_buffer: 3,
                gate_column: 0,
                gate_value: 1,
                length,
                ..Descriptor::default()
            },
        ];
        let mut registry = registry(&descriptors);
        registry[0].names[1] = "value.$kind".into();
        registry[0].columns[1].static_enum = Some(vec!["empty".into(), "payload".into()]);
        registry[0].columns[2].static_enum = Some(vec!["valid".into()]);
        let reply = Reply {
            header: Header {
                shape_id: 123,
                row_count: length,
                unit_end: length,
                total_count: length,
                ..Header::default()
            },
            descriptors,
            top_level: vec![0],
            buffers: vec![
                vec![0, 1],
                [(u32::MAX - 1).to_le_bytes(), 1u32.to_le_bytes()].concat(),
                vec![0],
                u32::MAX.to_le_bytes().to_vec(),
            ],
            ..Reply::default()
        };
        let mut bytes = with_read_budget(8, || {
            write(&reply, &registry, ReadContext::default()).unwrap()
        });
        assert!(bytes.len() < 600);
        let (tag_byte, payload_byte) = with_read_budget(8, || {
            let validated = read(&bytes, &registry, ReadContext::default()).unwrap();
            (
                validated.buffers[0].as_ptr() as usize - bytes.as_ptr() as usize,
                validated.buffers[2].as_ptr() as usize - bytes.as_ptr() as usize,
            )
        });
        bytes[payload_byte] = 255;
        let checksum = xxhash_rust::xxh3::xxh3_64(&bytes[HEADER_BYTES..]);
        bytes[120..128].copy_from_slice(&checksum.to_le_bytes());
        let error = with_read_budget(8, || {
            read(&bytes, &registry, ReadContext::default()).unwrap_err()
        });
        assert_eq!(error.name, "dictionary-code");

        // The same physical payload remains harmless while every gate is inactive.
        bytes[tag_byte + 1] = 0;
        let checksum = xxhash_rust::xxh3::xxh3_64(&bytes[HEADER_BYTES..]);
        bytes[120..128].copy_from_slice(&checksum.to_le_bytes());
        with_read_budget(8, || {
            read(&bytes, &registry, ReadContext::default()).unwrap();
        });
    }

    #[test]
    fn nested_states_are_read_once_per_local_occurrence() {
        let depth = 64;
        let rows = 256u64;
        let mut descriptors: Vec<_> = (0..depth)
            .map(|i| Descriptor {
                name_id: i as u32,
                logical_type: LogicalType::Struct,
                type_param_b: 0,
                child_first: (i + 1) as u32,
                child_count: 1,
                state_layers: 4,
                state_buffer: 0,
                length: if i == 0 { TOP_LEVEL_LENGTH } else { rows },
                ..Descriptor::default()
            })
            .collect();
        descriptors.push(Descriptor {
            name_id: depth as u32,
            logical_type: LogicalType::Enum,
            type_param_a: 8,
            type_param_b: 0,
            data_buffer: 1,
            length: rows,
            ..Descriptor::default()
        });
        let mut registry = registry(&descriptors);
        registry[0].columns[depth].static_enum = Some(vec!["valid".into()]);
        let reply = Reply {
            header: Header {
                shape_id: 123,
                row_count: rows,
                unit_end: rows,
                total_count: rows,
                ..Header::default()
            },
            descriptors,
            top_level: vec![0],
            buffers: vec![vec![0; rows as usize / 8], vec![0; rows as usize]],
            ..Reply::default()
        };
        let reads = (depth + 1) * rows as usize;
        let bytes = with_read_budget(reads, || {
            write(&reply, &registry, ReadContext::default()).unwrap()
        });
        with_read_budget(reads, || {
            read(&bytes, &registry, ReadContext::default()).unwrap();
        });
    }

    #[test]
    fn list_activity_intersects_compact_runs_without_rechecking_shared_values() {
        let length = u64::from(u32::MAX);
        let descriptors = vec![
            Descriptor {
                logical_type: LogicalType::List,
                type_param_a: 64,
                type_param_b: 0,
                child_first: 1,
                child_count: 1,
                aux_buffer: 0,
                state_layers: 1,
                state_buffer: 1,
                state_widths: 1,
                ..Descriptor::default()
            },
            Descriptor {
                name_id: 1,
                logical_type: LogicalType::Enum,
                physical_encoding: Encoding::RunLength,
                type_param_a: 8,
                type_param_b: 0,
                data_buffer: 2,
                aux_buffer: 3,
                length,
                ..Descriptor::default()
            },
        ];
        let mut registry = registry(&descriptors);
        registry[0].columns[1].static_enum = Some(vec!["valid".into()]);
        let mut reply = Reply {
            header: Header {
                shape_id: 123,
                row_count: 3,
                unit_end: 3,
                total_count: 3,
                ..Header::default()
            },
            descriptors,
            top_level: vec![0],
            buffers: vec![
                [0, length / 3, length / 3 * 2, length]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect(),
                vec![0, 1, 0],
                vec![0],
                u32::MAX.to_le_bytes().to_vec(),
            ],
            ..Reply::default()
        };
        let bytes = with_read_budget(4, || {
            write(&reply, &registry, ReadContext::default()).unwrap()
        });
        with_read_budget(4, || {
            read(&bytes, &registry, ReadContext::default()).unwrap();
        });
        // Only the last list occurrence is active; its first element is near the
        // end of the compact logical run and still exposes the invalid value.
        reply.buffers[1] = vec![1, 1, 0];
        reply.buffers[2][0] = 255;
        let error = with_read_budget(4, || {
            write(&reply, &registry, ReadContext::default()).unwrap_err()
        });
        assert_eq!(error.name, "dictionary-code");
    }

    fn text_reply(bytes: &[u8], first: u64, total: u64, final_extent: bool) -> (Reply, Vec<Shape>) {
        let descriptor = Descriptor {
            logical_type: LogicalType::Text,
            type_param_b: 0,
            data_buffer: 0,
            aux_buffer: 1,
            ..Descriptor::default()
        };
        let registry = registry(std::slice::from_ref(&descriptor));
        (
            Reply {
                header: Header {
                    shape_id: 123,
                    row_count: 1,
                    unit_end: 1,
                    total_count: 1,
                    ..Header::default()
                },
                descriptors: vec![descriptor],
                top_level: vec![0],
                buffers: vec![
                    bytes.to_vec(),
                    [0u64.to_le_bytes(), (bytes.len() as u64).to_le_bytes()].concat(),
                ],
                extents: vec![Extent {
                    column: 0,
                    coordinate: Coordinate::Byte,
                    final_extent,
                    index: 0,
                    first,
                    end: first + bytes.len() as u64,
                    total,
                }],
                ..Reply::default()
            },
            registry,
        )
    }

    #[test]
    fn text_extents_accept_every_split_of_multibyte_scalars() {
        for value in ["\u{e9}", "\u{20ac}", "\u{1f600}"] {
            let bytes = value.as_bytes();
            for first in 0..bytes.len() {
                for end in first + 1..=bytes.len() {
                    let (reply, registry) = text_reply(
                        &bytes[first..end],
                        first as u64,
                        bytes.len() as u64,
                        end == bytes.len(),
                    );
                    let encoded = write(&reply, &registry, ReadContext::default()).unwrap();
                    let validated = read(&encoded, &registry, ReadContext::default()).unwrap();
                    assert_eq!(validated.value_bytes(0, 0).unwrap(), &bytes[first..end]);
                }
            }
            let doubled = value.repeat(2);
            let (reply, registry) = text_reply(
                &doubled.as_bytes()[1..bytes.len() + 1],
                1,
                doubled.len() as u64,
                false,
            );
            write(&reply, &registry, ReadContext::default()).unwrap();
        }
        // A middle extent can contain a scalar suffix and the next scalar prefix.
        let bytes = "\u{e9}\u{20ac}\u{1f600}".as_bytes();
        let (reply, registry) = text_reply(&bytes[1..8], 1, bytes.len() as u64, false);
        write(&reply, &registry, ReadContext::default()).unwrap();
    }

    #[test]
    fn text_extents_refuse_invalid_continuations_and_complete_truncation() {
        for (bytes, first, total, final_extent) in [
            (vec![0x80], 0, 2, false),
            (vec![0x80, 0x80, 0x80, 0x80], 1, 6, false),
            (vec![0xc3, b'x'], 0, 3, false),
            (vec![0xe0, 0x80], 0, 3, false),
            (vec![0xed, 0xa0], 1, 5, false),
            (vec![0xf4, 0x90], 1, 5, false),
            (vec![0xf0, 0x90], 0, 3, false),
            (vec![0xc3], 0, 1, true),
            (vec![0xe2, 0x82], 1, 3, true),
        ] {
            let (reply, registry) = text_reply(&bytes, first, total, final_extent);
            let error = write(&reply, &registry, ReadContext::default()).unwrap_err();
            assert_eq!(error.name, "invalid-utf8", "{bytes:x?}");
        }
        let (mut reply, registry) = text_reply(&[0xc3], 0, 2, false);
        reply.extents.clear();
        assert_eq!(
            write(&reply, &registry, ReadContext::default())
                .unwrap_err()
                .name,
            "invalid-utf8"
        );
    }
    #[test]
    fn logical_rows_above_u32_fit_a_small_rle_unit() {
        let length = u64::from(u32::MAX) + 1;
        let descriptor = Descriptor {
            logical_type: LogicalType::Unsigned,
            type_param_a: 64,
            type_param_b: 0,
            physical_encoding: Encoding::RunLength,
            data_buffer: 0,
            aux_buffer: 1,
            ..Descriptor::default()
        };
        let registry = registry(std::slice::from_ref(&descriptor));
        let reply = Reply {
            header: Header {
                shape_id: 123,
                row_count: length,
                unit_end: length,
                total_count: length,
                ..Header::default()
            },
            descriptors: vec![descriptor],
            top_level: vec![0],
            buffers: vec![
                [7u64.to_le_bytes(), 11u64.to_le_bytes()].concat(),
                [u32::MAX.to_le_bytes(), 1u32.to_le_bytes()].concat(),
            ],
            ..Reply::default()
        };
        let bytes = write(&reply, &registry, ReadContext::default()).unwrap();
        assert!(bytes.len() < 512);
        let read = read(&bytes, &registry, ReadContext::default()).unwrap();
        assert_eq!(read.unsigned(0, length - 2).unwrap(), 7);
        assert_eq!(read.unsigned(0, length - 1).unwrap(), 11);
    }
    #[test]
    fn nested_descriptor_and_coordinate_walks_have_no_chosen_depth_cap() {
        let depth = 6000usize;
        let mut descriptors = Vec::new();
        for i in 0..depth {
            descriptors.push(Descriptor {
                name_id: i as u32,
                logical_type: LogicalType::List,
                type_param_a: 64,
                type_param_b: 0,
                child_first: (i + 1) as u32,
                child_count: 1,
                aux_buffer: 0,
                length: if i == 0 { TOP_LEVEL_LENGTH } else { 1 },
                ..Descriptor::default()
            });
        }
        descriptors.push(Descriptor {
            name_id: depth as u32,
            logical_type: LogicalType::Unsigned,
            type_param_a: 32,
            type_param_b: 0,
            data_buffer: 1,
            length: 1,
            ..Descriptor::default()
        });
        let registry = registry(&descriptors);
        let reply = Reply {
            header: Header {
                shape_id: 123,
                unit_start: 40,
                unit_end: 41,
                row_count: 1,
                total_count: 41,
                ..Header::default()
            },
            descriptors,
            top_level: vec![0],
            buffers: vec![
                [0u64.to_le_bytes(), 1u64.to_le_bytes()].concat(),
                7u32.to_le_bytes().to_vec(),
            ],
            ..Reply::default()
        };
        let bytes = write(&reply, &registry, ReadContext::default()).unwrap();
        let read = read(&bytes, &registry, ReadContext::default()).unwrap();
        assert_eq!(read.unsigned(depth, 0).unwrap(), 7);
        let original = read.original_occurrence(depth, 0).unwrap();
        assert_eq!(original.len(), depth + 1);
        assert_eq!(original[0], (0, 40));
    }
    #[test]
    fn run_length_wide_integer_uses_encoded_limb_count() {
        let descriptor = Descriptor {
            logical_type: LogicalType::WideInt,
            type_param_a: 1,
            type_param_b: 0,
            physical_encoding: Encoding::RunLength,
            data_buffer: 0,
            aux_buffer: 1,
            ..Descriptor::default()
        };
        let registry = registry(std::slice::from_ref(&descriptor));
        let reply = Reply {
            header: Header {
                shape_id: 123,
                row_count: 3,
                unit_end: 3,
                total_count: 3,
                ..Header::default()
            },
            descriptors: vec![descriptor],
            top_level: vec![0],
            buffers: vec![
                [u64::MAX.to_le_bytes(), u64::MAX.to_le_bytes()].concat(),
                3u32.to_le_bytes().to_vec(),
            ],
            ..Reply::default()
        };
        let bytes = write(&reply, &registry, ReadContext::default()).unwrap();
        assert_eq!(
            read(&bytes, &registry, ReadContext::default())
                .unwrap()
                .limbs(0, 2)
                .unwrap(),
            (u64::MAX, u64::MAX)
        );
    }
}
