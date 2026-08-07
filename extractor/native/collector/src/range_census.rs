//! THE RANGE/BYTE/DEPTH CENSUS: what a capture frame actually touches.
//!
//! # Why this exists
//!
//! The capture-floor plan (stage S0) needs one number the collector has never
//! measured: how much of the game's memory a single observation frame reads,
//! and in how many separate pieces. Every figure in the plan today is a MODEL
//! — 34-58 MiB of unique engine bytes across 150k-450k distinct source ranges,
//! anchored on two hand-derived per-domain censuses (`d47_cost_bench`'s
//! ~75_500 ranges / ~5 MiB, and `d81_frame_cost`'s foreign-subtree lower
//! bound). Both were derived by reading offset constants and multiplying by a
//! row count off the wire. Neither observed a read.
//!
//! This module is the OTHER half of the instrument. [`super::attach_log`]'s
//! `range_tracer` (compiled only under the `range-tracer` cargo feature)
//! records every guarded read of one frame into a sidecar file; everything
//! here reads that sidecar back and reduces it. The reduction lives in the
//! library, not in the tool, so it can be tested against known-answer inputs
//! that state their own arithmetic.
//!
//! # THE ROW UNIT IS A BRACKET TAG, NOT A DOMAIN
//!
//! Attribution comes from `ReaderClockBracketGuard`, and that guard is opened
//! per WORKER, not per domain. Most workers read exactly one domain and the tag
//! is that domain's id, but two kinds of worker do not:
//!
//!  * `LW_TAG_COUNTRY_BUNDLE` (tag 200) is a synthetic id for the single worker
//!    that walks the per-country domains 10..=17 in one pass;
//!  * some workers cover a neighbour (domain 49's bracket also covers 50).
//!
//! So a row is "everything one bracket read", and a bundle row is the union of
//! its member domains. THIS MATTERS IN ONE DIRECTION ONLY, and it is the safe
//! one for a floor: bundling can never inflate the full-set totals, because
//! those are computed by unioning ranges across every row and are indifferent
//! to how the rows were grouped. What bundling does cost is resolution — a
//! bundled row cannot be split between its members, and byte sharing BETWEEN
//! bundle members is invisible (it is already deduplicated inside the row
//! rather than showing up in `shared_bytes`). A per-domain split of a bundle
//! would need instrumentation inside that worker, which this instrument
//! deliberately does not have.
//!
//! # What a census counts, exactly
//!
//! For each bracket tag, and then for the union of all of them:
//!
//!  * `distinct_ranges` — the number of DISTINCT `(address, length)` pairs.
//!    Two reads of the same eight bytes are one range; a 4-byte and an 8-byte
//!    read at the same address are two. This is the number the copy-only
//!    bracket argument turns on: it is a count of separately-addressed heap
//!    objects and string buffers, not of bytes.
//!  * `coalesced_ranges` — those same ranges after overlapping and ADJACENT
//!    spans are merged. A reader that reads two neighbouring fields
//!    separately pays for two `distinct_ranges` and one `coalesced_range`, so
//!    the pair brackets what a scatter/gather pass would have to issue.
//!  * `unique_bytes` — the measure of the union of the ranges. Bytes read
//!    twice count once. This is the "unique engine bytes" the floor model
//!    bounds.
//!  * `read_count` / `bytes_moved` — every read event and the sum of their
//!    lengths, undeduplicated. `bytes_moved / unique_bytes` is the re-read
//!    factor.
//!  * `max_depth` — the pointer-discovery depth, defined below.
//!
//! and across the whole set: `unique_bytes` of the union of every domain's
//! ranges, plus `shared_bytes` = (sum of per-domain unique bytes) − (full-set
//! unique bytes), which is exactly the double-counting a per-domain table
//! would commit if its rows were added up.
//!
//! # Depth is DERIVED, and the derivation has one free parameter
//!
//! Pointer-discovery depth is not observable at a read primitive: the
//! primitive sees an address, not the chain that produced it. So the tracer
//! also records every heap-shaped qword each read RETURNED, and the depth is
//! reconstructed here from that provenance:
//!
//!  * a read whose address cannot be derived from any pointer observed
//!    EARLIER in the same bracket is at depth 0 — a root;
//!  * otherwise its depth is one more than the depth of the READ that
//!    produced the pointer it came from;
//!  * "came from" means the address lies within [`DerivationSpan`] bytes at or
//!    after an observed pointer value, i.e. it is that object's base plus a
//!    field offset.
//!
//! When several observed pointers could explain an address, the SHALLOWEST is
//! chosen. That makes `max_depth` the length of the longest SHORTEST-chain,
//! which is the conservative reading: an address that is reachable in two hops
//! is never reported as five because some unrelated deep pointer happened to
//! sit below it.
//!
//! The span is the free parameter and it is reported rather than hidden. Too
//! small and a legitimate deep field read (domain 47 reads a `u32` at `+0xF4C`
//! from a `CUnitLeader` base) is misclassified as a root, understating depth;
//! too large and an unrelated pointer can adopt an address, overstating the
//! chain. [`Census::depth_by_span`] therefore computes the whole table at
//! several spans so the sensitivity is visible instead of assumed.
//!
//! # What this is NOT
//!
//!  * NOT a claim about a whole capture. A census covers ONE frame. Frames
//!    differ (a domain that discovers nothing this hour reads almost nothing),
//!    so a single frame is a sample, and the tool prints which frame.
//!  * NOT a copy plan. A range here is what a reader consumed, which is an
//!    upper bound on what it needed and a lower bound on what a bracket that
//!    also had to DISCOVER those addresses would have to hold.
//!  * NOT complete when the sidecar says it was capped. The tracer bounds its
//!    own memory and marks the sidecar `truncated`; every consumer here
//!    carries that flag forward so a capped run cannot be read as a total.

use std::collections::BTreeMap;
use std::io::{self, Read};

/// Sidecar magic. Bumped only when the record layout changes.
pub const TRACE_MAGIC: [u8; 8] = *b"H4RTRACE";
/// Sidecar format version this build writes and reads.
pub const TRACE_VERSION: u32 = 1;
/// Bytes per sidecar entry. Fixed width so the file can be streamed and its
/// entry count derived from its length.
pub const TRACE_ENTRY_LEN: usize = 24;
/// Bytes in the sidecar header, ahead of the first entry.
pub const TRACE_HEADER_LEN: usize = 32;

/// A read event: the tracer observed a guarded read of `len` bytes at `addr`.
pub const TRACE_KIND_READ: u8 = 1;
/// A heap-shaped qword that a read RETURNED. Always follows the `TRACE_KIND_READ`
/// entry it came out of, in the same bracket, with the same sequence block.
pub const TRACE_KIND_POINTER: u8 = 2;

/// Set in the header flags when the tracer hit its entry cap and stopped
/// recording. A truncated sidecar is a LOWER bound on every count here.
pub const TRACE_FLAG_TRUNCATED: u32 = 1;

/// How far past an observed pointer an address may lie and still be treated as
/// that object's field. See the module header: this is the derivation's one
/// free parameter, and it is always reported beside the depth it produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DerivationSpan(pub u64);

/// The spans the tool reports by default. `0x1000` is a page — tight enough
/// that only same-object field reads chain; `0x10000` comfortably covers the
/// widest structure any reader in this collector indexes into (domain 47's
/// `CUnitLeader` is 0xF50 bytes); `0x100000` is deliberately far too generous
/// and exists to show what the number does when the parameter is abused.
pub const DEFAULT_DERIVATION_SPANS: [DerivationSpan; 3] = [
    DerivationSpan(0x1000),
    DerivationSpan(0x1_0000),
    DerivationSpan(0x10_0000),
];

/// One decoded sidecar entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    pub kind: u8,
    pub bracket_tag: u16,
    /// Length of the read, in bytes. Zero for a pointer entry.
    pub len: u32,
    /// Global monotonic order in which the tracer recorded this entry.
    pub seq: u64,
    /// The read's address, or the pointer's value.
    pub value: u64,
}

impl TraceEntry {
    /// Decode one fixed-width entry. Returns `None` on a short slice.
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

    /// Encode one fixed-width entry. Byte 1 is reserved padding and is written
    /// zero so a future field cannot be confused with stale bytes.
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

/// The sidecar header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceHeader {
    pub version: u32,
    pub entry_len: u32,
    /// Which extraction frame this trace covers, counted from the tracer's arm.
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

/// Read a whole sidecar. Entries are returned in file order, which is per-thread
/// flush order and NOT global sequence order; [`Census::build`] sorts per domain.
///
/// `max_entries` bounds the tool's own memory. Exceeding it is an error rather
/// than a silent truncation: a census that quietly dropped the tail would
/// under-report every count it publishes.
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
        // A zero-length READ is REFUSED rather than tolerated. The read
        // primitives never issue one (`read_bytes` and `read_into` return early
        // on `len == 0`, and `read_scalar::<N>` requires `1 <= N <= 8`), so its
        // presence means the sidecar did not come from this instrument. Letting
        // it through would put a pair in `read_count` that `distinct_ranges`
        // then excludes, quietly contradicting the definition of both columns.
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

/// `Ok(true)` when `buf` was filled, `Ok(false)` on a clean EOF at offset zero,
/// and an error on a partial record (which is a corrupt file, not an EOF).
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

/// An address range a reader consumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Range {
    pub address: u64,
    pub len: u64,
}

/// Merge overlapping and ADJACENT ranges. Input need not be sorted; output is
/// sorted, pairwise disjoint and non-adjacent.
///
/// Adjacent spans are merged deliberately: this function answers "how many
/// separate copies would a scatter/gather pass have to issue", and two
/// touching spans are one copy. The count of DISTINCT `(address, len)` pairs —
/// the other number the census reports — is taken before this runs.
/// EXCLUSIVE ENDS ARE COMPUTED IN `u128`, deliberately. A range's exclusive end
/// can be `u64::MAX + 1`, which does not fit in a `u64`; saturating it to
/// `u64::MAX` silently shortens the range by a byte and can let a
/// one-byte-separated pair merge. No live address gets near the top of the
/// address space, so this is an edge nothing has hit — but a census whose union
/// arithmetic is only conditionally right is not a measurement.
fn end_excl(range: Range) -> u128 {
    u128::from(range.address) + u128::from(range.len)
}

pub fn coalesce(mut ranges: Vec<Range>) -> Vec<Range> {
    ranges.sort_unstable();
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.len == 0 {
            continue;
        }
        match merged.last_mut() {
            // `<=` merges adjacency as well as overlap.
            Some(last) if u128::from(range.address) <= end_excl(*last) => {
                let end = end_excl(range).max(end_excl(*last));
                // `end - address` is at most 2^64, and it is 2^64 only for a
                // range covering the entire space, which no trace can produce
                // (the tracer records `u32` lengths). Clamping here is
                // therefore unreachable rather than lossy, and it is present so
                // the conversion cannot panic on a corrupt sidecar.
                last.len = u64::try_from(end - u128::from(last.address)).unwrap_or(u64::MAX);
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// Measure of the union of `ranges`, in bytes. Bytes covered twice count once.
pub fn unique_bytes(ranges: &[Range]) -> u64 {
    coalesce(ranges.to_vec())
        .iter()
        .fold(0u64, |acc, range| acc.saturating_add(range.len))
}

/// One bracket tag's census row. NOT necessarily one domain — see the module
/// header's "THE ROW UNIT IS A BRACKET TAG" section.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BracketCensus {
    /// The `ReaderClockBracketGuard` tag. Usually a domain id; tag 200 is the
    /// country bundle (domains 10..=17) and [`TAG_UNATTRIBUTED_ROW`] is
    /// everything read outside any bracket.
    pub bracket_tag: u16,
    /// Every read event this bracket issued, undeduplicated.
    pub read_count: u64,
    /// Sum of every read's length, undeduplicated: what the reader moved.
    pub bytes_moved: u64,
    /// Distinct `(address, len)` pairs.
    pub distinct_ranges: u64,
    /// Distinct ranges after overlapping/adjacent merge.
    pub coalesced_ranges: u64,
    /// Measure of the union of this bracket's ranges.
    pub unique_bytes: u64,
    /// Deepest shortest-chain pointer-discovery depth, at the span in
    /// [`Census::span`].
    pub max_depth: u32,
    /// Reads at depth 0: addresses no observed pointer explains.
    pub root_reads: u64,
}

/// The whole-frame census.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Census {
    pub header: TraceHeader,
    pub span: DerivationSpan,
    pub brackets: Vec<BracketCensus>,
    /// Distinct `(address, len)` pairs across every bracket. Grouping-invariant:
    /// unaffected by which reads share a bracket tag.
    pub full_set_distinct_ranges: u64,
    /// Coalesced ranges across every bracket. Grouping-invariant.
    pub full_set_coalesced_ranges: u64,
    /// Measure of the union of EVERY bracket's ranges: the number the capture
    /// floor model bounds. Grouping-invariant, which is why a bundled bracket
    /// tag cannot distort the verdict.
    pub full_set_unique_bytes: u64,
    /// (sum of per-bracket `unique_bytes`) − `full_set_unique_bytes`. The bytes
    /// more than one BRACKET reads, i.e. exactly the double-count a reader
    /// would commit by adding the per-bracket column up.
    ///
    /// Unlike the three totals above this IS grouping-dependent, and it is a
    /// LOWER bound on cross-domain sharing: bytes two members of one bundled
    /// bracket both read are deduplicated inside that row and never appear
    /// here.
    pub shared_bytes: u64,
}

/// The row holding reads taken while no bracket was held. Mirrors
/// `attach_log::range_tracer::TAG_UNATTRIBUTED`, restated here so a consumer of
/// a census need not compile the recording half to interpret it.
pub const TAG_UNATTRIBUTED_ROW: u16 = u16::MAX;

impl Census {
    /// Reduce a decoded sidecar.
    ///
    /// Entries are grouped by domain and sorted by `seq` inside each group,
    /// because depth is a causal quantity: a pointer can only explain a read
    /// that the tracer recorded AFTER it. Sorting per domain rather than
    /// globally is deliberate and sound — domains read on separate threads and
    /// never hand each other addresses, so cross-domain interleaving carries no
    /// information the derivation could use.
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
            let mut row = BracketCensus {
                bracket_tag,
                ..Default::default()
            };
            let mut distinct: std::collections::BTreeSet<(u64, u64)> =
                std::collections::BTreeSet::new();
            let mut ranges: Vec<Range> = Vec::new();

            // Observed pointer value -> depth of the READ that produced it.
            // Shallowest wins: a value seen at depth 2 and again at depth 5
            // explains its consumers at depth 3, not 6.
            let mut pointers: BTreeMap<u64, u32> = BTreeMap::new();
            let mut last_read_depth = 0u32;

            for entry in &group {
                match entry.kind {
                    TRACE_KIND_READ => {
                        let len = u64::from(entry.len);
                        row.read_count = row.read_count.saturating_add(1);
                        row.bytes_moved = row.bytes_moved.saturating_add(len);
                        if len > 0 {
                            distinct.insert((entry.value, len));
                            all_distinct.insert((entry.value, len));
                            let range = Range {
                                address: entry.value,
                                len,
                            };
                            ranges.push(range);
                            all_ranges.push(range);
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
                            }
                        }
                    }
                    TRACE_KIND_POINTER => {
                        // Attributed to the read that returned it, which is the
                        // immediately preceding READ entry in this domain's
                        // sequence. The tracer emits them as one block, so no
                        // other read can interleave.
                        pointers
                            .entry(entry.value)
                            .and_modify(|existing| *existing = (*existing).min(last_read_depth))
                            .or_insert(last_read_depth);
                    }
                    _ => {}
                }
            }

            row.distinct_ranges = distinct.len() as u64;
            let coalesced = coalesce(ranges);
            row.coalesced_ranges = coalesced.len() as u64;
            row.unique_bytes = coalesced
                .iter()
                .fold(0u64, |acc, range| acc.saturating_add(range.len));
            per_domain_unique_sum = per_domain_unique_sum.saturating_add(row.unique_bytes);
            brackets.push(row);
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

    /// The maximum depth each of `spans` produces, so the derivation's
    /// sensitivity to its one free parameter is visible rather than assumed.
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

/// The depth an address inherits from the shallowest observed pointer that can
/// explain it, or `None` when no pointer can (a root read).
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

    /// KNOWN ANSWER, computed by hand in the assertion messages. A change to
    /// `coalesce` that stopped merging adjacency, or started merging a gap,
    /// fails here.
    #[test]
    fn coalesce_merges_overlap_and_adjacency_and_never_a_gap() {
        let merged = coalesce(vec![
            Range {
                address: 0x2000,
                len: 0x10,
            },
            // Adjacent to the one above: 0x2010 == 0x2000 + 0x10.
            Range {
                address: 0x2010,
                len: 0x10,
            },
            // Overlaps the merged 0x2000..0x2020 span.
            Range {
                address: 0x2018,
                len: 0x10,
            },
            // One byte short of adjacency with 0x2000..0x2028: a GAP at 0x2028.
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

    /// The distinction the whole census turns on: the same bytes read many
    /// times are ONE range and ONE set of unique bytes, but many reads and many
    /// moved bytes.
    ///
    /// Production change that fails this: making `Census::build` count read
    /// EVENTS into `distinct_ranges`, or dropping the dedup set in favour of
    /// `ranges.len()`.
    #[test]
    fn rereading_the_same_bytes_is_one_range_and_many_reads() {
        let entries: Vec<TraceEntry> = (0..5)
            .map(|i| read(47, i, 0x1_0000_0000, 8))
            .collect();
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

    /// A 4-byte and an 8-byte read at the SAME address are two distinct ranges
    /// but one coalesced span, and the union is the wider of the two.
    ///
    /// Production change that fails this: keying the dedup set on the address
    /// alone.
    #[test]
    fn same_address_different_width_is_two_distinct_ranges_and_one_span() {
        let entries = vec![read(47, 0, 0x4000, 4), read(47, 1, 0x4000, 8)];
        let census = Census::build(header(), &entries, DerivationSpan(0x1000));
        let row = &census.brackets[0];
        assert_eq!(row.distinct_ranges, 2, "(0x4000,4) and (0x4000,8) differ");
        assert_eq!(row.coalesced_ranges, 1, "both lie in 0x4000..0x4008");
        assert_eq!(row.unique_bytes, 8);
    }

    /// Cross-domain overlap is counted ONCE in the full set, and `shared_bytes`
    /// names exactly the double-count the per-domain column commits.
    ///
    /// Production change that fails this: building `full_set_unique_bytes` by
    /// summing the per-domain rows instead of unioning the ranges.
    #[test]
    fn bytes_two_domains_both_read_are_counted_once_in_the_full_set() {
        let entries = vec![
            read(10, 0, 0x8000, 0x40),
            // Domain 11 reads the last 0x20 bytes of the same object.
            read(11, 1, 0x8020, 0x40),
        ];
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

    /// The depth derivation, on a chain built by hand: a root read yields a
    /// pointer, a read at that pointer's object yields another, and so on.
    ///
    /// Production change that fails this: attributing a pointer to the wrong
    /// read (e.g. to the NEXT one), or taking the max instead of the min over
    /// candidate pointers.
    #[test]
    fn depth_is_the_length_of_the_pointer_chain_that_reached_the_address() {
        let entries = vec![
            // Depth 0: a static root nobody pointed at.
            read(81, 0, 0x1000, 8),
            pointer(81, 1, 0x20_0000),
            // Depth 1: the object the root pointed at.
            read(81, 2, 0x20_0000, 8),
            pointer(81, 3, 0x30_0000),
            // Depth 2: reached through the depth-1 read, at a field offset.
            read(81, 4, 0x30_0010, 8),
            pointer(81, 5, 0x40_0000),
            // Depth 3.
            read(81, 6, 0x40_0000, 8),
        ];
        let census = Census::build(header(), &entries, DerivationSpan(0x1000));
        let row = &census.brackets[0];
        assert_eq!(row.max_depth, 3, "the chain root -> 0x20 -> 0x30 -> 0x40 is three hops");
        assert_eq!(row.root_reads, 1, "only the 0x1000 read is unexplained");
    }

    /// When two pointers could explain an address the SHALLOWEST wins, so an
    /// unrelated deep pointer sitting just below an object cannot inflate the
    /// chain.
    ///
    /// Production change that fails this: `.max()` instead of `.min()` in
    /// `derive_depth`.
    #[test]
    fn a_deeper_candidate_pointer_never_lengthens_a_shorter_chain() {
        let entries = vec![
            read(60, 0, 0x1000, 8),
            // Shallow: observed by the depth-0 root read.
            pointer(60, 1, 0x50_0000),
            read(60, 2, 0x50_0000, 8),
            pointer(60, 3, 0x60_0000),
            read(60, 4, 0x60_0000, 8),
            // A DEEP pointer (produced by the depth-2 read) that also lands
            // just below the address the next read uses.
            pointer(60, 5, 0x50_0100),
            // Explained by 0x50_0000 (depth 1) and by 0x50_0100 (depth 3).
            read(60, 6, 0x50_0200, 8),
        ];
        let census = Census::build(header(), &entries, DerivationSpan(0x1000));
        let row = &census.brackets[0];
        // The chain is root(0) -> 0x50_0000(1) -> 0x60_0000(2). The final read
        // at 0x50_0200 has two candidate parents: 0x50_0000 at depth 1 and
        // 0x50_0100 at depth 3. Taking the SHALLOWEST puts it at depth 1 and
        // leaves the deepest read in the trace at 2. Taking the deepest instead
        // would put it at 4 and make `max_depth` 4, so this number is the
        // mutation detector for `.min()` in `derive_depth`.
        assert_eq!(
            row.max_depth, 2,
            "shortest-chain accounting keeps the 0x50_0200 read one hop from \
             0x50_0000, so the deepest read stays 0x60_0000 at depth 2"
        );
        assert_eq!(row.root_reads, 1, "only the 0x1000 read is unexplained");
    }

    /// The span is a real parameter with a real effect, and `depth_by_span`
    /// exposes it. A field read 0x2000 past its base is a ROOT at a 0x1000
    /// span and a child at 0x10000.
    ///
    /// Production change that fails this: hardcoding the span inside
    /// `derive_depth` and ignoring the argument.
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
            vec![
                (DerivationSpan(0x1000), 0),
                (DerivationSpan(0x1_0000), 1),
            ],
            "0x70_2000 is 0x2000 past 0x70_0000: outside a 0x1000 span (root, depth 0) \
             and inside a 0x10000 one (depth 1)"
        );
    }

    /// The sidecar round-trips. A layout change that broke the writer or the
    /// reader in isolation fails here.
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
        assert!(decoded_header.truncated(), "the flag survives the round trip");
        assert_eq!(decoded, entries);
    }

    /// A sidecar whose entry stride or magic disagrees with this build is
    /// REFUSED rather than misparsed into a plausible-looking census.
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

    /// The union arithmetic stays exact at the very top of the address space,
    /// where a range's EXCLUSIVE end does not fit in a `u64`.
    ///
    /// Production change that fails this: computing exclusive ends with
    /// `saturating_add` in `u64` (which shortens the last range by a byte and
    /// lets a one-byte-separated pair merge) instead of in `u128`.
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
        // 0xFFFF_FFFF_FFFF_FFFD..FFFE and FFFF..(end) are separated by the byte
        // at 0xFFFF_FFFF_FFFF_FFFE, so they must NOT merge.
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

    /// A zero-length READ cannot come from the read primitives, so a sidecar
    /// carrying one is REFUSED rather than half-counted.
    ///
    /// Production change that fails this: accepting the entry, which would put
    /// it in `read_count` while `distinct_ranges` silently dropped it.
    #[test]
    fn a_zero_length_read_entry_is_refused() {
        let mut bytes = header().encode().to_vec();
        bytes.extend_from_slice(&read(47, 0, 0x9000, 0).encode());
        let error = read_trace(&mut bytes.as_slice(), 16).unwrap_err();
        assert!(
            error.contains("zero-length read"),
            "unexpected error: {error}"
        );
        // A zero-length POINTER entry is normal: pointers carry no length.
        let mut ok = header().encode().to_vec();
        ok.extend_from_slice(&pointer(47, 0, 0x1_0000_0000).encode());
        assert!(read_trace(&mut ok.as_slice(), 16).is_ok());
    }

    /// A trace larger than the tool's cap is an ERROR, never a censused prefix.
    ///
    /// Production change that fails this: replacing the cap check with a
    /// `break`, which would publish a census of the first N entries under the
    /// same field names as a complete one.
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
