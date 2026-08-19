use std::collections::BTreeMap;
use std::io::{self, Read};

pub const TRACE_MAGIC: [u8; 8] = *b"H4RTRACE";
pub const TRACE_VERSION: u32 = 1;
pub const TRACE_ENTRY_LEN: usize = 24;
pub const TRACE_HEADER_LEN: usize = 32;

pub const TRACE_KIND_READ: u8 = 1;
pub const TRACE_KIND_POINTER: u8 = 2;

pub const TRACE_FLAG_TRUNCATED: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DerivationSpan(pub u64);

pub const DEFAULT_DERIVATION_SPANS: [DerivationSpan; 3] = [
    DerivationSpan(0x1000),
    DerivationSpan(0x1_0000),
    DerivationSpan(0x10_0000),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    pub kind: u8,
    pub bracket_tag: u16,
    pub len: u32,
    pub seq: u64,
    pub value: u64,
}

impl TraceEntry {
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < TRACE_ENTRY_LEN {
            return None;
        }
        Some(Self {
            kind: bytes[0],
            bracket_tag: u16::from_le_bytes([bytes[2], bytes[3]]),
            len: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            seq: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
            value: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
        })
    }

    pub fn encode(&self) -> [u8; TRACE_ENTRY_LEN] {
        let mut out = [0u8; TRACE_ENTRY_LEN];
        out[0] = self.kind;
        out[2..4].copy_from_slice(&self.bracket_tag.to_le_bytes());
        out[4..8].copy_from_slice(&self.len.to_le_bytes());
        out[8..16].copy_from_slice(&self.seq.to_le_bytes());
        out[16..24].copy_from_slice(&self.value.to_le_bytes());
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceHeader {
    pub version: u32,
    pub entry_len: u32,
    pub frame_ordinal: u64,
    pub flags: u32,
}

impl TraceHeader {
    pub fn encode(&self) -> [u8; TRACE_HEADER_LEN] {
        let mut out = [0u8; TRACE_HEADER_LEN];
        out[0..8].copy_from_slice(&TRACE_MAGIC);
        out[8..12].copy_from_slice(&self.version.to_le_bytes());
        out[12..16].copy_from_slice(&self.entry_len.to_le_bytes());
        out[16..24].copy_from_slice(&self.frame_ordinal.to_le_bytes());
        out[24..28].copy_from_slice(&self.flags.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < TRACE_HEADER_LEN {
            return Err(format!(
                "trace header is {} byte(s); {TRACE_HEADER_LEN} required",
                bytes.len()
            ));
        }
        if bytes[0..8] != TRACE_MAGIC {
            return Err("trace header magic is not H4RTRACE".to_string());
        }
        let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if version != TRACE_VERSION {
            return Err(format!(
                "trace version {version} is not the {TRACE_VERSION} this build reads"
            ));
        }
        let entry_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        if entry_len as usize != TRACE_ENTRY_LEN {
            return Err(format!(
                "trace entry stride {entry_len} is not the {TRACE_ENTRY_LEN} this build reads"
            ));
        }
        Ok(Self {
            version,
            entry_len,
            frame_ordinal: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            flags: u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        })
    }

    pub fn truncated(&self) -> bool {
        self.flags & TRACE_FLAG_TRUNCATED != 0
    }
}

pub fn read_trace(
    reader: &mut impl Read,
    max_entries: usize,
) -> Result<(TraceHeader, Vec<TraceEntry>), String> {
    let mut header_bytes = [0u8; TRACE_HEADER_LEN];
    read_exact_or_eof(reader, &mut header_bytes)
        .map_err(|error| format!("read trace header: {error}"))?
        .then_some(())
        .ok_or_else(|| "trace file is shorter than its header".to_string())?;
    let header = TraceHeader::decode(&header_bytes)?;

    let mut entries = Vec::new();
    let mut buf = [0u8; TRACE_ENTRY_LEN];
    loop {
        let full = read_exact_or_eof(reader, &mut buf)
            .map_err(|error| format!("read trace entry {}: {error}", entries.len()))?;
        if !full {
            break;
        }
        let entry = TraceEntry::decode(&buf)
            .ok_or_else(|| format!("truncated trace entry {}", entries.len()))?;
        if entry.kind != TRACE_KIND_READ && entry.kind != TRACE_KIND_POINTER {
            return Err(format!(
                "trace entry {} has unknown kind {}",
                entries.len(),
                entry.kind
            ));
        }
        if entry.kind == TRACE_KIND_READ && entry.len == 0 {
            return Err(format!(
                "trace entry {} is a zero-length read at {:#x}; the read \
                 primitives cannot produce one",
                entries.len(),
                entry.value
            ));
        }
        entries.push(entry);
        if entries.len() > max_entries {
            return Err(format!(
                "trace holds more than {max_entries} entries; raise the cap rather than \
                 censusing a prefix"
            ));
        }
    }
    Ok((header, entries))
}

fn read_exact_or_eof(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<bool> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("{filled} of {} byte(s)", buf.len()),
                ));
            }
            Ok(n) => filled += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Range {
    pub address: u64,
    pub len: u64,
}

fn end_excl(range: Range) -> u128 {
    u128::from(range.address) + u128::from(range.len)
}

pub fn coalesce(ranges: Vec<Range>) -> Vec<Range> {
    coalesce_with_gap(ranges, 0)
}

pub fn coalesce_with_gap(mut ranges: Vec<Range>, gap: u64) -> Vec<Range> {
    ranges.sort_unstable();
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.len == 0 {
            continue;
        }
        match merged.last_mut() {
            Some(last) if u128::from(range.address) <= end_excl(*last) + u128::from(gap) => {
                let end = end_excl(range).max(end_excl(*last));
                last.len = u64::try_from(end - u128::from(last.address)).unwrap_or(u64::MAX);
            }
            _ => merged.push(range),
        }
    }
    merged
}

pub fn unique_bytes(ranges: &[Range]) -> u64 {
    coalesce(ranges.to_vec())
        .iter()
        .fold(0u64, |acc, range| acc.saturating_add(range.len))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BracketCensus {
    pub bracket_tag: u16,
    pub read_count: u64,
    pub bytes_moved: u64,
    pub distinct_ranges: u64,
    pub coalesced_ranges: u64,
    pub unique_bytes: u64,
    pub max_depth: u32,
    pub root_reads: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Census {
    pub header: TraceHeader,
    pub span: DerivationSpan,
    pub brackets: Vec<BracketCensus>,
    pub full_set_distinct_ranges: u64,
    pub full_set_coalesced_ranges: u64,
    pub full_set_unique_bytes: u64,
    pub shared_bytes: u64,
}

pub const TAG_UNATTRIBUTED_ROW: u16 = u16::MAX;

impl Census {
    pub fn build(header: TraceHeader, entries: &[TraceEntry], span: DerivationSpan) -> Self {
        let mut by_tag: BTreeMap<u16, Vec<TraceEntry>> = BTreeMap::new();
        for entry in entries {
            by_tag.entry(entry.bracket_tag).or_default().push(*entry);
        }

        let mut brackets = Vec::with_capacity(by_tag.len());
        let mut all_ranges: Vec<Range> = Vec::new();
        let mut all_distinct: std::collections::BTreeSet<(u64, u64)> =
            std::collections::BTreeSet::new();
        let mut per_domain_unique_sum = 0u64;

        for (bracket_tag, mut group) in by_tag {
            group.sort_unstable_by_key(|entry| entry.seq);
            let walk = walk_bracket(bracket_tag, &group, span);
            all_ranges.extend_from_slice(&walk.ranges);
            all_distinct.extend(walk.distinct.iter().copied());
            per_domain_unique_sum = per_domain_unique_sum.saturating_add(walk.row.unique_bytes);
            brackets.push(walk.row);
        }

        let full_coalesced = coalesce(all_ranges);
        let full_set_unique_bytes = full_coalesced
            .iter()
            .fold(0u64, |acc, range| acc.saturating_add(range.len));

        Self {
            header,
            span,
            brackets,
            full_set_distinct_ranges: all_distinct.len() as u64,
            full_set_coalesced_ranges: full_coalesced.len() as u64,
            full_set_unique_bytes,
            shared_bytes: per_domain_unique_sum.saturating_sub(full_set_unique_bytes),
        }
    }

    pub fn depth_by_span(
        header: TraceHeader,
        entries: &[TraceEntry],
        spans: &[DerivationSpan],
    ) -> Vec<(DerivationSpan, u32)> {
        spans
            .iter()
            .map(|span| {
                let census = Census::build(header, entries, *span);
                let max = census
                    .brackets
                    .iter()
                    .map(|row| row.max_depth)
                    .max()
                    .unwrap_or(0);
                (*span, max)
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BracketWalk {
    pub row: BracketCensus,
    pub distinct: std::collections::BTreeSet<(u64, u64)>,
    pub ranges: Vec<Range>,
    pub root_addresses: Vec<u64>,
    pub pointer_values: Vec<u64>,
}

pub fn walk_bracket(bracket_tag: u16, group: &[TraceEntry], span: DerivationSpan) -> BracketWalk {
    let mut row = BracketCensus {
        bracket_tag,
        ..Default::default()
    };
    let mut distinct: std::collections::BTreeSet<(u64, u64)> = std::collections::BTreeSet::new();
    let mut ranges: Vec<Range> = Vec::new();
    let mut root_addresses: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();

    let mut pointers: BTreeMap<u64, u32> = BTreeMap::new();
    let mut last_read_depth = 0u32;

    for entry in group {
        match entry.kind {
            TRACE_KIND_READ => {
                let len = u64::from(entry.len);
                row.read_count = row.read_count.saturating_add(1);
                row.bytes_moved = row.bytes_moved.saturating_add(len);
                if len > 0 {
                    distinct.insert((entry.value, len));
                    ranges.push(Range {
                        address: entry.value,
                        len,
                    });
                }
                let depth = derive_depth(&pointers, entry.value, span);
                match depth {
                    Some(depth) => {
                        last_read_depth = depth;
                        row.max_depth = row.max_depth.max(depth);
                    }
                    None => {
                        last_read_depth = 0;
                        row.root_reads = row.root_reads.saturating_add(1);
                        root_addresses.insert(entry.value);
                    }
                }
            }
            TRACE_KIND_POINTER => {
                pointers
                    .entry(entry.value)
                    .and_modify(|existing| *existing = (*existing).min(last_read_depth))
                    .or_insert(last_read_depth);
            }
            _ => {}
        }
    }

    row.distinct_ranges = distinct.len() as u64;
    let coalesced = coalesce(ranges.clone());
    row.coalesced_ranges = coalesced.len() as u64;
    row.unique_bytes = coalesced
        .iter()
        .fold(0u64, |acc, range| acc.saturating_add(range.len));

    BracketWalk {
        row,
        distinct,
        ranges,
        root_addresses: root_addresses.into_iter().collect(),
        pointer_values: pointers.into_keys().collect(),
    }
}

pub fn walk_all(entries: &[TraceEntry], span: DerivationSpan) -> BTreeMap<u16, BracketWalk> {
    let mut by_tag: BTreeMap<u16, Vec<TraceEntry>> = BTreeMap::new();
    for entry in entries {
        by_tag.entry(entry.bracket_tag).or_default().push(*entry);
    }
    by_tag
        .into_iter()
        .map(|(bracket_tag, mut group)| {
            group.sort_unstable_by_key(|entry| entry.seq);
            (bracket_tag, walk_bracket(bracket_tag, &group, span))
        })
        .collect()
}

pub const DEFAULT_GAP_THRESHOLDS: [u64; 9] = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

pub type Span = (u128, u128);

pub fn coalesce_spans(ranges: &[Range], gap: u64) -> Vec<Span> {
    let mut sorted: Vec<Range> = ranges
        .iter()
        .copied()
        .filter(|range| range.len > 0)
        .collect();
    sorted.sort_unstable();
    let mut out: Vec<Span> = Vec::with_capacity(sorted.len());
    for range in sorted {
        let start = u128::from(range.address);
        let end = end_excl(range);
        match out.last_mut() {
            Some(last) if start <= last.1 + u128::from(gap) => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

pub fn merge_spans_with_gap(spans: &[Span], gap: u64) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(last) if span.0 <= last.1 + u128::from(gap) => last.1 = last.1.max(span.1),
            _ => out.push(*span),
        }
    }
    out
}

pub fn span_measure(spans: &[Span]) -> u128 {
    spans.iter().map(|(start, end)| end - start).sum()
}

pub fn exact_json_number(value: u128) -> serde_json::Value {
    match u64::try_from(value) {
        Ok(fits) => fits.into(),
        Err(_) => value.to_string().into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GapCurvePoint {
    pub gap: u64,
    pub ranges: u64,
    pub bytes: u128,
    pub over_copy_bytes: u128,
    pub worst_case_envelope_bytes: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GapCurve {
    pub label: String,
    pub distinct_ranges: u64,
    pub baseline_ranges: u64,
    pub baseline_bytes: u128,
    pub points: Vec<GapCurvePoint>,
}

pub fn worst_case_envelope_bytes(baseline_bytes: u128, baseline_ranges: u64, gap: u64) -> u128 {
    let bridges = u128::from(baseline_ranges.saturating_sub(1));
    baseline_bytes + bridges * u128::from(gap)
}

pub fn gap_curve(label: impl Into<String>, ranges: &[Range], gaps: &[u64]) -> GapCurve {
    let baseline = coalesce_spans(ranges, 0);
    let baseline_ranges = baseline.len() as u64;
    let baseline_bytes = span_measure(&baseline);

    let points = gaps
        .iter()
        .map(|gap| {
            let merged = merge_spans_with_gap(&baseline, *gap);
            let bytes = span_measure(&merged);
            GapCurvePoint {
                gap: *gap,
                ranges: merged.len() as u64,
                bytes,
                over_copy_bytes: bytes - baseline_bytes,
                worst_case_envelope_bytes: worst_case_envelope_bytes(
                    baseline_bytes,
                    baseline_ranges,
                    *gap,
                ),
            }
        })
        .collect();

    GapCurve {
        label: label.into(),
        distinct_ranges: 0,
        baseline_ranges,
        baseline_bytes,
        points,
    }
}

pub fn sum_curves(label: impl Into<String>, curves: &[GapCurve], gaps: &[u64]) -> GapCurve {
    let mut points: Vec<GapCurvePoint> = gaps
        .iter()
        .map(|gap| GapCurvePoint {
            gap: *gap,
            ranges: 0,
            bytes: 0,
            over_copy_bytes: 0,
            worst_case_envelope_bytes: 0,
        })
        .collect();
    let mut distinct_ranges = 0u64;
    let mut baseline_ranges = 0u64;
    let mut baseline_bytes = 0u128;
    for curve in curves {
        assert_eq!(
            curve.points.len(),
            points.len(),
            "curves summed across brackets must share one threshold list"
        );
        distinct_ranges = distinct_ranges.saturating_add(curve.distinct_ranges);
        baseline_ranges = baseline_ranges.saturating_add(curve.baseline_ranges);
        baseline_bytes += curve.baseline_bytes;
        for (total, point) in points.iter_mut().zip(curve.points.iter()) {
            assert_eq!(total.gap, point.gap, "threshold lists disagree");
            total.ranges = total.ranges.saturating_add(point.ranges);
            total.bytes += point.bytes;
            total.over_copy_bytes += point.over_copy_bytes;
            total.worst_case_envelope_bytes += point.worst_case_envelope_bytes;
        }
    }
    GapCurve {
        label: label.into(),
        distinct_ranges,
        baseline_ranges,
        baseline_bytes,
        points,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GapCurveReport {
    pub global: GapCurve,
    pub per_bracket: Vec<GapCurve>,
    pub summed_per_bracket: GapCurve,
}

pub fn gap_curve_report(walks: &BTreeMap<u16, BracketWalk>, gaps: &[u64]) -> GapCurveReport {
    let mut all_ranges: Vec<Range> = Vec::new();
    let mut all_distinct: std::collections::BTreeSet<(u64, u64)> =
        std::collections::BTreeSet::new();
    let mut per_bracket = Vec::with_capacity(walks.len());
    for (bracket_tag, walk) in walks {
        all_ranges.extend_from_slice(&walk.ranges);
        all_distinct.extend(walk.distinct.iter().copied());
        let mut curve = gap_curve(format!("tag {bracket_tag}"), &walk.ranges, gaps);
        curve.distinct_ranges = walk.distinct.len() as u64;
        per_bracket.push(curve);
    }
    let mut global = gap_curve("GLOBAL", &all_ranges, gaps);
    global.distinct_ranges = all_distinct.len() as u64;
    GapCurveReport {
        global,
        summed_per_bracket: sum_curves("SUM OF BRACKETS", &per_bracket, gaps),
        per_bracket,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowCoverage {
    pub anchors: u64,
    pub window_len: u64,
    pub window_ranges: u64,
    pub window_bytes: u128,
    pub data_bytes: u128,
    pub covered_bytes: u128,
    pub data_ranges: u64,
    pub fully_covered_ranges: u64,
    pub partially_covered_ranges: u64,
    pub uncovered_ranges: u64,
}

impl WindowCoverage {
    pub fn byte_coverage_pct(&self) -> f64 {
        if self.data_bytes == 0 {
            return 100.0;
        }
        self.covered_bytes as f64 * 100.0 / self.data_bytes as f64
    }

    pub fn range_coverage_pct(&self) -> f64 {
        if self.data_ranges == 0 {
            return 100.0;
        }
        self.fully_covered_ranges as f64 * 100.0 / self.data_ranges as f64
    }
}

pub fn window_coverage(data: &[Range], anchors: &[u64], window_len: u64) -> WindowCoverage {
    let data_spans = coalesce_spans(data, 0);
    let window_ranges: Vec<Range> = anchors
        .iter()
        .map(|anchor| Range {
            address: *anchor,
            len: window_len,
        })
        .collect();
    let windows = coalesce_spans(&window_ranges, 0);

    let mut out = WindowCoverage {
        anchors: {
            let unique: std::collections::BTreeSet<u64> = anchors.iter().copied().collect();
            unique.len() as u64
        },
        window_len,
        window_ranges: windows.len() as u64,
        window_bytes: span_measure(&windows),
        data_bytes: span_measure(&data_spans),
        data_ranges: data_spans.len() as u64,
        ..Default::default()
    };

    let mut window_index = 0usize;
    for (span_start, span_end) in &data_spans {
        while window_index < windows.len() && windows[window_index].1 <= *span_start {
            window_index += 1;
        }
        let mut covered_here = 0u128;
        let mut probe = window_index;
        while probe < windows.len() && windows[probe].0 < *span_end {
            let overlap_start = windows[probe].0.max(*span_start);
            let overlap_end = windows[probe].1.min(*span_end);
            if overlap_end > overlap_start {
                covered_here += overlap_end - overlap_start;
            }
            probe += 1;
        }
        out.covered_bytes += covered_here;
        if covered_here == span_end - span_start {
            out.fully_covered_ranges += 1;
        } else if covered_here > 0 {
            out.partially_covered_ranges += 1;
        } else {
            out.uncovered_ranges += 1;
        }
    }
    out
}

fn derive_depth(pointers: &BTreeMap<u64, u32>, address: u64, span: DerivationSpan) -> Option<u32> {
    if span.0 == 0 {
        return None;
    }
    let low = address.saturating_sub(span.0 - 1);
    pointers
        .range(low..=address)
        .map(|(_, depth)| *depth)
        .min()
        .map(|depth| depth.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(bracket_tag: u16, seq: u64, address: u64, len: u32) -> TraceEntry {
        TraceEntry {
            kind: TRACE_KIND_READ,
            bracket_tag,
            len,
            seq,
            value: address,
        }
    }

    fn pointer(bracket_tag: u16, seq: u64, value: u64) -> TraceEntry {
        TraceEntry {
            kind: TRACE_KIND_POINTER,
            bracket_tag,
            len: 0,
            seq,
            value,
        }
    }

    fn header() -> TraceHeader {
        TraceHeader {
            version: TRACE_VERSION,
            entry_len: TRACE_ENTRY_LEN as u32,
            frame_ordinal: 7,
            flags: 0,
        }
    }

    #[test]
    fn coalesce_merges_overlap_and_adjacency_and_never_a_gap() {
        let merged = coalesce(vec![
            Range {
                address: 0x2000,
                len: 0x10,
            },
            Range {
                address: 0x2010,
                len: 0x10,
            },
            Range {
                address: 0x2018,
                len: 0x10,
            },
            Range {
                address: 0x2029,
                len: 0x8,
            },
        ]);
        assert_eq!(
            merged,
            vec![
                Range {
                    address: 0x2000,
                    len: 0x28
                },
                Range {
                    address: 0x2029,
                    len: 0x8
                },
            ],
            "0x2000+0x10, 0x2010+0x10 and 0x2018+0x10 are one span 0x2000..0x2028; \
             0x2029 is separated by the byte at 0x2028 and must stay its own span"
        );
        assert_eq!(
            unique_bytes(&[
                Range {
                    address: 0x2000,
                    len: 0x10
                },
                Range {
                    address: 0x2008,
                    len: 0x10
                },
            ]),
            0x18,
            "0x2000..0x2010 and 0x2008..0x2018 overlap by 8; the union is 0x18 bytes"
        );
    }

    #[test]
    fn rereading_the_same_bytes_is_one_range_and_many_reads() {
        let entries: Vec<TraceEntry> = (0..5).map(|i| read(47, i, 0x1_0000_0000, 8)).collect();
        let census = Census::build(header(), &entries, DerivationSpan(0x1000));
        let row = &census.brackets[0];
        assert_eq!(row.read_count, 5, "five read events were recorded");
        assert_eq!(row.bytes_moved, 40, "five eight-byte reads moved 40 bytes");
        assert_eq!(
            row.distinct_ranges, 1,
            "all five reads are the same (address, len) pair"
        );
        assert_eq!(row.unique_bytes, 8, "they cover eight distinct bytes");
        assert_eq!(census.full_set_unique_bytes, 8);
    }

    #[test]
    fn same_address_different_width_is_two_distinct_ranges_and_one_span() {
        let entries = vec![read(47, 0, 0x4000, 4), read(47, 1, 0x4000, 8)];
        let census = Census::build(header(), &entries, DerivationSpan(0x1000));
        let row = &census.brackets[0];
        assert_eq!(row.distinct_ranges, 2, "(0x4000,4) and (0x4000,8) differ");
        assert_eq!(row.coalesced_ranges, 1, "both lie in 0x4000..0x4008");
        assert_eq!(row.unique_bytes, 8);
    }

    #[test]
    fn bytes_two_domains_both_read_are_counted_once_in_the_full_set() {
        let entries = vec![read(10, 0, 0x8000, 0x40), read(11, 1, 0x8020, 0x40)];
        let census = Census::build(header(), &entries, DerivationSpan(0x1000));
        let per_domain_sum: u64 = census.brackets.iter().map(|row| row.unique_bytes).sum();
        assert_eq!(per_domain_sum, 0x80, "0x40 + 0x40 read per domain");
        assert_eq!(
            census.full_set_unique_bytes, 0x60,
            "the union 0x8000..0x8060 is 0x60 bytes"
        );
        assert_eq!(
            census.shared_bytes, 0x20,
            "0x8020..0x8040 is read by both, so the per-domain column \
             over-counts by exactly 0x20"
        );
        assert_eq!(
            census.shared_bytes,
            per_domain_sum - census.full_set_unique_bytes,
            "shared_bytes is defined as that difference"
        );
    }

    #[test]
    fn depth_is_the_length_of_the_pointer_chain_that_reached_the_address() {
        let entries = vec![
            read(81, 0, 0x1000, 8),
            pointer(81, 1, 0x20_0000),
            read(81, 2, 0x20_0000, 8),
            pointer(81, 3, 0x30_0000),
            read(81, 4, 0x30_0010, 8),
            pointer(81, 5, 0x40_0000),
            read(81, 6, 0x40_0000, 8),
        ];
        let census = Census::build(header(), &entries, DerivationSpan(0x1000));
        let row = &census.brackets[0];
        assert_eq!(
            row.max_depth, 3,
            "the chain root -> 0x20 -> 0x30 -> 0x40 is three hops"
        );
        assert_eq!(row.root_reads, 1, "only the 0x1000 read is unexplained");
    }

    #[test]
    fn a_deeper_candidate_pointer_never_lengthens_a_shorter_chain() {
        let entries = vec![
            read(60, 0, 0x1000, 8),
            pointer(60, 1, 0x50_0000),
            read(60, 2, 0x50_0000, 8),
            pointer(60, 3, 0x60_0000),
            read(60, 4, 0x60_0000, 8),
            pointer(60, 5, 0x50_0100),
            read(60, 6, 0x50_0200, 8),
        ];
        let census = Census::build(header(), &entries, DerivationSpan(0x1000));
        let row = &census.brackets[0];
        assert_eq!(
            row.max_depth, 2,
            "shortest-chain accounting keeps the 0x50_0200 read one hop from \
             0x50_0000, so the deepest read stays 0x60_0000 at depth 2"
        );
        assert_eq!(row.root_reads, 1, "only the 0x1000 read is unexplained");
    }

    #[test]
    fn the_derivation_span_changes_the_answer_and_the_report_shows_it() {
        let entries = vec![
            read(47, 0, 0x1000, 8),
            pointer(47, 1, 0x70_0000),
            read(47, 2, 0x70_2000, 4),
        ];
        let table = Census::depth_by_span(
            header(),
            &entries,
            &[DerivationSpan(0x1000), DerivationSpan(0x1_0000)],
        );
        assert_eq!(
            table,
            vec![(DerivationSpan(0x1000), 0), (DerivationSpan(0x1_0000), 1),],
            "0x70_2000 is 0x2000 past 0x70_0000: outside a 0x1000 span (root, depth 0) \
             and inside a 0x10000 one (depth 1)"
        );
    }

    #[test]
    fn sidecar_entries_and_header_round_trip() {
        let head = TraceHeader {
            version: TRACE_VERSION,
            entry_len: TRACE_ENTRY_LEN as u32,
            frame_ordinal: 12345,
            flags: TRACE_FLAG_TRUNCATED,
        };
        let entries = vec![
            read(47, 1, 0xdead_beef_0000, 0x30),
            pointer(47, 2, 0x1234_5678_9abc),
        ];
        let mut bytes = head.encode().to_vec();
        for entry in &entries {
            bytes.extend_from_slice(&entry.encode());
        }
        let (decoded_header, decoded) = read_trace(&mut bytes.as_slice(), 1024).expect("read back");
        assert_eq!(decoded_header, head);
        assert!(
            decoded_header.truncated(),
            "the flag survives the round trip"
        );
        assert_eq!(decoded, entries);
    }

    #[test]
    fn a_foreign_sidecar_is_refused_not_reinterpreted() {
        let mut bad_magic = TraceHeader {
            version: TRACE_VERSION,
            entry_len: TRACE_ENTRY_LEN as u32,
            frame_ordinal: 0,
            flags: 0,
        }
        .encode()
        .to_vec();
        bad_magic[0] = b'X';
        assert!(read_trace(&mut bad_magic.as_slice(), 16)
            .unwrap_err()
            .contains("magic"));

        let mut bad_stride = TraceHeader {
            version: TRACE_VERSION,
            entry_len: 16,
            frame_ordinal: 0,
            flags: 0,
        }
        .encode()
        .to_vec();
        bad_stride.extend_from_slice(&[0u8; TRACE_ENTRY_LEN]);
        assert!(read_trace(&mut bad_stride.as_slice(), 16)
            .unwrap_err()
            .contains("stride"));
    }

    #[test]
    fn a_range_touching_the_last_byte_of_the_address_space_keeps_its_byte() {
        assert_eq!(
            unique_bytes(&[Range {
                address: u64::MAX,
                len: 1
            }]),
            1,
            "the byte at u64::MAX is one byte, not zero"
        );
        let merged = coalesce(vec![
            Range {
                address: u64::MAX - 2,
                len: 1,
            },
            Range {
                address: u64::MAX,
                len: 1,
            },
        ]);
        assert_eq!(merged.len(), 2, "a one-byte gap is still a gap up here");
        assert_eq!(unique_bytes(&merged), 2);
    }

    #[test]
    fn a_zero_length_read_entry_is_refused() {
        let mut bytes = header().encode().to_vec();
        bytes.extend_from_slice(&read(47, 0, 0x9000, 0).encode());
        let error = read_trace(&mut bytes.as_slice(), 16).unwrap_err();
        assert!(
            error.contains("zero-length read"),
            "unexpected error: {error}"
        );
        let mut ok = header().encode().to_vec();
        ok.extend_from_slice(&pointer(47, 0, 0x1_0000_0000).encode());
        assert!(read_trace(&mut ok.as_slice(), 16).is_ok());
    }

    #[test]
    fn the_gap_curve_is_the_hand_computed_one_and_the_comparison_is_inclusive() {
        let ranges = vec![
            Range {
                address: 0x1000,
                len: 8,
            },
            Range {
                address: 0x1010,
                len: 8,
            },
            Range {
                address: 0x1030,
                len: 8,
            },
            Range {
                address: 0x1080,
                len: 8,
            },
        ];
        let curve = gap_curve("hand", &ranges, &[0, 7, 8, 23, 24, 71, 72]);
        assert_eq!(curve.baseline_ranges, 4, "no two spans touch");
        assert_eq!(curve.baseline_bytes, 32, "four eight-byte spans");

        let at = |gap: u64| -> GapCurvePoint {
            *curve
                .points
                .iter()
                .find(|point| point.gap == gap)
                .expect("threshold present")
        };

        assert_eq!(
            (at(0).ranges, at(0).bytes),
            (4, 32),
            "gap 0 is plain coalescing"
        );
        assert_eq!(
            (at(7).ranges, at(7).bytes),
            (4, 32),
            "the smallest hole is 8, so 7 bridges nothing"
        );
        assert_eq!(
            (at(8).ranges, at(8).bytes),
            (3, 40),
            "A and B become 0x1000..0x1018 = 24 bytes; +8 (C) +8 (D) = 40. \
             An exclusive comparison would leave 4 ranges / 32 bytes here"
        );
        assert_eq!(
            (at(23).ranges, at(23).bytes),
            (3, 40),
            "24 is still too wide at 23"
        );
        assert_eq!(
            (at(24).ranges, at(24).bytes),
            (2, 64),
            "A..C is 0x1000..0x1038 = 56 bytes, plus D's 8"
        );
        assert_eq!(
            (at(71).ranges, at(71).bytes),
            (2, 64),
            "72 is still too wide at 71"
        );
        assert_eq!(
            (at(72).ranges, at(72).bytes),
            (1, 136),
            "everything is 0x1000..0x1088 = 0x88 = 136 bytes"
        );

        assert_eq!(
            at(8).over_copy_bytes,
            8,
            "the only bridged hole is the 8-byte one between A and B"
        );
        assert_eq!(
            at(8).worst_case_envelope_bytes,
            32u128 + 8 * 3,
            "the no-trace bound assumes all three holes bridge at the full 8 bytes; \
             the measured cost is 40, so the bound is 16 bytes pessimistic here"
        );
        assert!(
            at(8).bytes <= at(8).worst_case_envelope_bytes,
            "the measured curve can never exceed the bound it is checked against"
        );
    }

    #[test]
    fn staged_reduction_equals_merging_the_raw_stream_at_each_threshold() {
        let raw = vec![
            Range {
                address: 0x9000,
                len: 16,
            },
            Range {
                address: 0x9000,
                len: 16,
            },
            Range {
                address: 0x9008,
                len: 16,
            },
            Range {
                address: 0x9018,
                len: 8,
            },
            Range {
                address: 0x9030,
                len: 4,
            },
            Range {
                address: 0x9100,
                len: 32,
            },
            Range {
                address: 0x9130,
                len: 1,
            },
        ];
        let gaps = [0u64, 1, 4, 8, 16, 32, 64, 128, 256];
        let curve = gap_curve("staged", &raw, &gaps);
        for gap in gaps {
            let direct = coalesce_with_gap(raw.clone(), gap);
            let direct_bytes = direct
                .iter()
                .map(|range| u128::from(range.len))
                .sum::<u128>();
            let point = curve.points.iter().find(|p| p.gap == gap).expect("point");
            assert_eq!(
                (point.ranges, point.bytes),
                (direct.len() as u64, direct_bytes),
                "staged and direct merging disagree at gap {gap}"
            );
        }
    }

    #[test]
    fn the_curve_reduces_unsorted_nested_and_empty_input_by_hand() {
        let raw = vec![
            Range {
                address: 100,
                len: 10,
            },
            Range {
                address: 0,
                len: 10,
            },
            Range {
                address: 10,
                len: 1,
            },
            Range {
                address: 150,
                len: 0,
            },
            Range {
                address: 0,
                len: 100,
            },
            Range {
                address: 200,
                len: 10,
            },
        ];
        let curve = gap_curve("hazards", &raw, &[0, 89, 90]);
        assert_eq!(
            curve.baseline_ranges, 2,
            "0..110 (the nested and unsorted pieces all fall inside or extend it) \
             and 200..210 — the zero-length range at 150 is NOT a third span"
        );
        assert_eq!(curve.baseline_bytes, 120, "110 + 10");
        let point = |gap: u64| *curve.points.iter().find(|p| p.gap == gap).unwrap();
        assert_eq!((point(0).ranges, point(0).bytes), (2, 120));
        assert_eq!(
            (point(89).ranges, point(89).bytes),
            (2, 120),
            "the hole 110..200 is 90 bytes wide, so 89 does not bridge it"
        );
        assert_eq!((point(90).ranges, point(90).bytes), (1, 210), "0..210");
        assert_eq!(point(90).over_copy_bytes, 90, "the bridged hole, exactly");

        let empty = gap_curve("empty", &[], &[0, 64]);
        assert_eq!(empty.baseline_ranges, 0);
        assert_eq!(empty.baseline_bytes, 0);
        assert_eq!(
            empty.points.len(),
            2,
            "a threshold asked for is a row reported"
        );
        assert!(empty.points.iter().all(|p| p.ranges == 0 && p.bytes == 0));
    }

    #[test]
    fn a_merge_spanning_the_entire_address_space_is_still_counted_exactly() {
        let raw = vec![
            Range { address: 0, len: 1 },
            Range {
                address: u64::MAX,
                len: 1,
            },
        ];
        let curve = gap_curve("whole space", &raw, &[0, u64::MAX - 1]);
        assert_eq!(curve.baseline_ranges, 2);
        assert_eq!(curve.baseline_bytes, 2);
        let wide = curve.points.last().unwrap();
        assert_eq!(wide.ranges, 1, "the hole is exactly u64::MAX-1 bytes wide");
        assert_eq!(
            wide.bytes,
            1u128 << 64,
            "0..=u64::MAX is 2^64 bytes; a u64 total would report 2^64-1"
        );
        assert_eq!(wide.over_copy_bytes, (1u128 << 64) - 2);
        assert_eq!(
            worst_case_envelope_bytes(2, 2, u64::MAX - 1),
            2 + u128::from(u64::MAX - 1),
            "the bound is exact too, not saturated"
        );
    }

    #[test]
    fn per_bracket_curves_never_merge_across_the_dedup_boundary() {
        let entries = vec![read(1, 0, 0x3000, 8), read(2, 1, 0x3010, 8)];
        let walks = walk_all(&entries, DerivationSpan(0x1000));
        let report = gap_curve_report(&walks, &[0, 8]);

        let summed = &report.summed_per_bracket;
        let summed_at_8 = summed.points.iter().find(|p| p.gap == 8).unwrap();
        assert_eq!(
            (summed_at_8.ranges, summed_at_8.bytes),
            (2, 16),
            "each bracket holds one lone 8-byte span; nothing inside a bracket \
             is within 8 bytes of anything else, so both stay one range"
        );

        let global = &report.global;
        let global_at_8 = global.points.iter().find(|p| p.gap == 8).unwrap();
        assert_eq!(
            (global_at_8.ranges, global_at_8.bytes),
            (1, 24),
            "across brackets the 8-byte hole at 0x3008 bridges into 0x3000..0x3018"
        );
        let global_at_0 = global.points.iter().find(|p| p.gap == 0).unwrap();
        assert_eq!(
            (global_at_0.ranges, global_at_0.bytes),
            (2, 16),
            "at gap 0 the two scopes agree, which is why only a non-zero \
             threshold can detect the boundary being crossed"
        );
    }

    #[test]
    fn the_summed_table_adds_every_column_it_publishes() {
        let entries = vec![
            read(1, 0, 0, 1),
            read(1, 1, 3, 1),
            read(2, 2, 100, 1),
            read(2, 3, 103, 1),
        ];
        let walks = walk_all(&entries, DerivationSpan(0x1000));
        let summed = gap_curve_report(&walks, &[0, 2]).summed_per_bracket;
        assert_eq!(
            summed.distinct_ranges, 4,
            "four distinct (address,len) pairs"
        );
        assert_eq!(
            summed.baseline_ranges, 4,
            "two spans in each of two brackets"
        );
        assert_eq!(summed.baseline_bytes, 4, "four one-byte reads");
        let at2 = summed.points.iter().find(|p| p.gap == 2).unwrap();
        assert_eq!(
            at2.ranges, 2,
            "each bracket's pair bridges its own 2-byte hole"
        );
        assert_eq!(at2.bytes, 8, "0..4 and 100..104");
        assert_eq!(at2.over_copy_bytes, 4, "two bridged holes of 2 bytes each");
        assert_eq!(
            at2.worst_case_envelope_bytes, 8,
            "each bracket read 2 bytes over 2 spans, so its bound is \
             2 + 2*(2-1) = 4, and the pair sums to 8"
        );
    }

    #[test]
    fn an_empty_trace_still_reports_every_requested_threshold_in_both_scopes() {
        let walks = walk_all(&[], DerivationSpan(0x1000));
        assert!(walks.is_empty());
        let report = gap_curve_report(&walks, &[0, 64, 4096]);
        for (label, curve) in [
            ("global", &report.global),
            ("summed", &report.summed_per_bracket),
        ] {
            assert_eq!(
                curve.points.iter().map(|p| p.gap).collect::<Vec<_>>(),
                vec![0, 64, 4096],
                "{label} lost a threshold"
            );
            assert!(
                curve.points.iter().all(|p| p.ranges == 0 && p.bytes == 0),
                "{label} should be all zeroes, not absent"
            );
        }
        assert!(report.per_bracket.is_empty());
    }

    #[test]
    fn the_global_curve_at_gap_zero_reproduces_the_census_full_set() {
        let entries = vec![
            read(10, 0, 0x8000, 0x40),
            read(11, 1, 0x8020, 0x40),
            read(11, 2, 0x8020, 0x40),
            read(65535, 3, 0x9000, 0x10),
        ];
        let census = Census::build(header(), &entries, DerivationSpan(0x1000));
        let walks = walk_all(&entries, DerivationSpan(0x1000));
        let global = gap_curve_report(&walks, &[0]).global;
        assert_eq!(
            global.distinct_ranges, census.full_set_distinct_ranges,
            "the global scope's distinct set is the census's full set"
        );
        assert_eq!(
            global.baseline_bytes,
            u128::from(census.full_set_unique_bytes),
            "0x8000..0x8060 (0x60) plus 0x9000..0x9010 (0x10) is 0x70"
        );
        assert_eq!(global.baseline_bytes, 0x70);
        assert_eq!(
            global.baseline_ranges, census.full_set_coalesced_ranges,
            "two disjoint spans"
        );
        assert_eq!(global.baseline_ranges, 2);
    }

    #[test]
    fn the_worst_case_envelope_is_baseline_plus_gap_times_interior_holes() {
        assert_eq!(
            worst_case_envelope_bytes(61_606_245, 504_423, 16),
            69_676_997,
            "61,606,245 + 16*504,422 = 69,676,997 = 66.45 MiB, the published 16 B row"
        );
        assert_eq!(
            worst_case_envelope_bytes(61_606_245, 504_423, 32),
            77_747_749
        );
        assert_eq!(
            worst_case_envelope_bytes(61_606_245, 504_423, 64),
            93_889_253
        );
        assert_eq!(
            worst_case_envelope_bytes(61_606_245, 504_423, 128),
            126_172_261
        );
        assert_eq!(
            worst_case_envelope_bytes(100, 1, 4096),
            100,
            "a single span has no interior hole, so no threshold can pad it"
        );
        assert_eq!(
            worst_case_envelope_bytes(100, 0, 4096),
            100,
            "and neither can none"
        );
    }

    #[test]
    fn the_range_shaped_view_is_a_byte_short_only_for_a_whole_space_span() {
        let whole = vec![
            Range { address: 0, len: 1 },
            Range {
                address: u64::MAX,
                len: 1,
            },
        ];
        let lossy = coalesce_with_gap(whole.clone(), u64::MAX - 1);
        assert_eq!(lossy.len(), 1);
        assert_eq!(
            lossy[0].len,
            u64::MAX,
            "2^64 does not fit a u64 length, so the Range view reports 2^64-1"
        );
        assert_eq!(
            span_measure(&coalesce_spans(&whole, u64::MAX - 1)),
            1u128 << 64,
            "the span view carries the byte the Range view cannot"
        );

        let narrower = vec![
            Range { address: 1, len: 1 },
            Range {
                address: u64::MAX,
                len: 1,
            },
        ];
        let lossy = coalesce_with_gap(narrower.clone(), u64::MAX - 1);
        assert_eq!(lossy.len(), 1);
        assert_eq!(u128::from(lossy[0].len), u128::from(u64::MAX));
        assert_eq!(
            span_measure(&coalesce_spans(&narrower, u64::MAX - 1)),
            u128::from(u64::MAX)
        );
    }

    #[test]
    #[should_panic(expected = "threshold lists disagree")]
    fn summing_curves_built_for_different_thresholds_aborts() {
        let curve = gap_curve("tag 1", &[Range { address: 0, len: 1 }], &[0]);
        let _ = sum_curves("mismatched", &[curve], &[64]);
    }

    #[test]
    fn a_byte_total_too_wide_for_json_becomes_a_string_not_a_truncated_number() {
        assert_eq!(exact_json_number(0), serde_json::json!(0u64));
        assert_eq!(
            exact_json_number(u128::from(u64::MAX)),
            serde_json::json!(u64::MAX),
            "the widest total that fits stays a number"
        );
        assert_eq!(
            exact_json_number(u128::from(u64::MAX) + 1),
            serde_json::json!("18446744073709551616"),
            "one wider becomes the exact decimal string, never 0"
        );
        assert_eq!(
            exact_json_number(u128::MAX),
            serde_json::json!("340282366920938463463374607431768211455")
        );
    }

    #[test]
    fn window_coverage_separates_whole_spans_from_clipped_and_missed_ones() {
        let data = vec![
            Range {
                address: 0x4000,
                len: 8,
            },
            Range {
                address: 0x4038,
                len: 16,
            },
            Range {
                address: 0x4040,
                len: 8,
            },
            Range {
                address: 0x5000,
                len: 8,
            },
        ];
        let coverage = window_coverage(&data, &[0x4000, 0x5000, 0x5000], 64);
        assert_eq!(
            coverage.anchors, 2,
            "the repeated 0x5000 anchor is one window"
        );
        assert_eq!(coverage.window_ranges, 2);
        assert_eq!(coverage.window_bytes, 128, "two 64-byte windows, disjoint");
        assert_eq!(
            coverage.data_ranges, 3,
            "0x4038..0x4048 and 0x4040..0x4048 are one span after coalescing"
        );
        assert_eq!(coverage.data_bytes, 8 + 16 + 8);
        assert_eq!(
            coverage.covered_bytes,
            8 + 8 + 8,
            "the 0x4038 span contributes only its first 8 bytes"
        );
        assert_eq!(coverage.fully_covered_ranges, 2);
        assert_eq!(coverage.partially_covered_ranges, 1);
        assert_eq!(coverage.uncovered_ranges, 0);
        assert!(
            (coverage.byte_coverage_pct() - 75.0).abs() < 1e-9,
            "24 of 32 bytes"
        );
        assert!(
            (coverage.range_coverage_pct() - (2.0 / 3.0 * 100.0)).abs() < 1e-9,
            "a clipped span is NOT covered for recipe purposes"
        );
    }

    #[test]
    fn window_size_moves_coverage_between_the_two_extremes() {
        let data = vec![
            Range {
                address: 0x4000,
                len: 8,
            },
            Range {
                address: 0x4040,
                len: 8,
            },
        ];
        let wide = window_coverage(&data, &[0x4000], 0x1000);
        assert_eq!(wide.covered_bytes, 16);
        assert_eq!(wide.fully_covered_ranges, 2);
        assert_eq!(
            wide.window_bytes, 0x1000,
            "and it moves 4096 bytes to get 16"
        );

        let none = window_coverage(&data, &[0x4000], 0);
        assert_eq!(none.window_ranges, 0, "a zero-length window is not a copy");
        assert_eq!(none.covered_bytes, 0);
        assert_eq!(none.uncovered_ranges, 2);
    }

    #[test]
    fn a_read_starting_at_a_windows_exclusive_end_is_a_miss() {
        let coverage = window_coverage(
            &[Range {
                address: 0x4040,
                len: 8,
            }],
            &[0x4000],
            64,
        );
        assert_eq!(coverage.window_ranges, 1);
        assert_eq!(coverage.window_bytes, 64, "0x4000..0x4040");
        assert_eq!(coverage.data_ranges, 1);
        assert_eq!(
            coverage.covered_bytes, 0,
            "0x4040 is the first byte OUTSIDE"
        );
        assert_eq!(coverage.fully_covered_ranges, 0);
        assert_eq!(coverage.partially_covered_ranges, 0);
        assert_eq!(coverage.uncovered_ranges, 1);
        assert_eq!(coverage.byte_coverage_pct(), 0.0);
        assert_eq!(coverage.range_coverage_pct(), 0.0);

        let inside = window_coverage(
            &[Range {
                address: 0x403F,
                len: 1,
            }],
            &[0x4000],
            64,
        );
        assert_eq!(inside.covered_bytes, 1);
        assert_eq!(inside.fully_covered_ranges, 1);
    }

    #[test]
    fn one_window_covering_several_spans_is_seen_by_each_of_them() {
        let data = vec![
            Range {
                address: 0x100,
                len: 4,
            },
            Range {
                address: 0x140,
                len: 4,
            },
            Range {
                address: 0x180,
                len: 4,
            },
        ];
        let coverage = window_coverage(&data, &[0x100], 0x100);
        assert_eq!(coverage.window_ranges, 1, "one window 0x100..0x200");
        assert_eq!(coverage.data_ranges, 3);
        assert_eq!(coverage.fully_covered_ranges, 3, "all three lie inside it");
        assert_eq!(coverage.covered_bytes, 12);
        assert_eq!(coverage.uncovered_ranges, 0);
    }

    #[test]
    fn window_and_data_totals_are_exact_across_the_whole_address_space() {
        let data = vec![
            Range { address: 0, len: 1 },
            Range {
                address: u64::MAX,
                len: 1,
            },
        ];
        let coverage = window_coverage(&data, &[0, 1], u64::MAX);
        assert_eq!(coverage.anchors, 2);
        assert_eq!(coverage.window_ranges, 1, "0..2^64-1 and 1..2^64 overlap");
        assert_eq!(
            coverage.window_bytes,
            1u128 << 64,
            "the union is the whole address space; a u64 total reports 2^64-1"
        );
        assert_eq!(coverage.data_ranges, 2);
        assert_eq!(coverage.data_bytes, 2);
        assert_eq!(coverage.covered_bytes, 2, "both reads lie inside");
        assert_eq!(coverage.fully_covered_ranges, 2);
        assert_eq!(coverage.byte_coverage_pct(), 100.0);

        let all_data = window_coverage(
            &[
                Range {
                    address: 0,
                    len: u64::MAX,
                },
                Range {
                    address: u64::MAX,
                    len: 1,
                },
            ],
            &[],
            64,
        );
        assert_eq!(all_data.data_ranges, 1);
        assert_eq!(all_data.data_bytes, 1u128 << 64);
        assert_eq!(all_data.covered_bytes, 0, "no anchors, no windows");
        assert_eq!(all_data.uncovered_ranges, 1);
    }

    #[test]
    fn coverage_percentages_are_defined_on_empty_input() {
        let empty = window_coverage(&[], &[0x1000], 64);
        assert_eq!(empty.data_ranges, 0);
        assert_eq!(empty.data_bytes, 0);
        assert_eq!(empty.byte_coverage_pct(), 100.0);
        assert_eq!(empty.range_coverage_pct(), 100.0);
        assert!(empty.byte_coverage_pct().is_finite());
        assert!(empty.range_coverage_pct().is_finite());
    }

    #[test]
    fn entries_out_of_sequence_order_are_sorted_before_depth_is_derived() {
        let entries = vec![
            read(7, 2, 0x2000, 8),
            pointer(7, 1, 0x2000),
            read(7, 0, 0x1000, 8),
        ];
        let span = DerivationSpan(0x1000);
        let walk = &walk_all(&entries, span)[&7];
        assert_eq!(
            walk.row,
            BracketCensus {
                bracket_tag: 7,
                read_count: 2,
                bytes_moved: 16,
                distinct_ranges: 2,
                coalesced_ranges: 2,
                unique_bytes: 16,
                max_depth: 1,
                root_reads: 1,
            },
            "seq 0 reads the root 0x1000, seq 1 yields the pointer 0x2000, and \
             seq 2 reads it at depth 1 — regardless of the order the tracer \
             flushed them in"
        );
        assert_eq!(walk.root_addresses, vec![0x1000]);
        assert_eq!(walk.pointer_values, vec![0x2000]);

        let census = Census::build(header(), &entries, span);
        assert_eq!(census.brackets, vec![walk.row.clone()]);
    }

    #[test]
    fn the_walk_and_the_census_report_the_same_rows() {
        let entries = vec![
            read(42, 0, 0x1_0000, 8),
            pointer(42, 1, 0x2_0000),
            read(42, 2, 0x2_0000, 8),
            read(42, 3, 0x2_0000, 8),
            read(42, 4, 0x2_0004, 8),
            read(43, 5, 0x1_0000, 8),
            read(43, 6, 0x7_0000, 4),
        ];
        let span = DerivationSpan(0x1_0000);
        let census = Census::build(header(), &entries, span);
        let walks = walk_all(&entries, span);
        assert_eq!(census.brackets.len(), walks.len());
        for row in &census.brackets {
            let walk = walks.get(&row.bracket_tag).expect("tag present");
            assert_eq!(&walk.row, row, "tag {} row drifted", row.bracket_tag);
            let coalesced = coalesce(walk.ranges.clone());
            assert_eq!(coalesced.len() as u64, row.coalesced_ranges);
            assert_eq!(
                coalesced.iter().map(|range| range.len).sum::<u64>(),
                row.unique_bytes
            );
            assert_eq!(walk.distinct.len() as u64, row.distinct_ranges);
        }

        let walk42 = &walks[&42];
        assert_eq!(
            walk42.root_addresses,
            vec![0x1_0000],
            "only the 0x1_0000 read is unexplained in bracket 42"
        );
        assert_eq!(walk42.row.root_reads, 1);
        assert_eq!(walk42.pointer_values, vec![0x2_0000]);

        let walk43 = &walks[&43];
        assert_eq!(
            walk43.root_addresses,
            vec![0x1_0000, 0x7_0000],
            "bracket 43 observed no pointer of its own, so both its reads are roots"
        );
        assert_eq!(
            walk43.row.root_reads, 2,
            "root_reads counts EVENTS; root_addresses counts addresses"
        );
    }

    #[test]
    fn a_re_read_root_is_many_root_reads_but_one_anchor() {
        let entries: Vec<TraceEntry> = (0..4).map(|i| read(29, i, 0x6000, 8)).collect();
        let walks = walk_all(&entries, DerivationSpan(0x1000));
        let walk = &walks[&29];
        assert_eq!(walk.row.root_reads, 4);
        assert_eq!(walk.root_addresses, vec![0x6000]);
    }

    #[test]
    fn a_trace_over_the_cap_is_an_error_not_a_prefix() {
        let mut bytes = header().encode().to_vec();
        for i in 0..8u64 {
            bytes.extend_from_slice(&read(1, i, 0x1000 + i * 8, 8).encode());
        }
        let error = read_trace(&mut bytes.as_slice(), 4).unwrap_err();
        assert!(
            error.contains("more than 4 entries"),
            "unexpected error: {error}"
        );
    }
}
