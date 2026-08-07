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

pub fn coalesce(ranges: Vec<Range>) -> Vec<Range> {
    coalesce_with_gap(ranges, 0)
}

/// Merge ranges separated by a gap of AT MOST `gap` bytes, as well as those
/// that overlap or touch. `gap == 0` is exactly [`coalesce`].
///
/// THE COMPARISON IS `<=`, and that is the definition this whole curve reports
/// against: "maximum merged gap 64 B" means two spans with exactly 64 unread
/// bytes between them become one copy, and a 65-byte hole does not. The output
/// ranges INCLUDE the bridged bytes, because a copier that issues one
/// descriptor for the merged span moves them.
///
/// Output is sorted and pairwise separated by MORE than `gap`.
///
/// # THIS FUNCTION IS THE LOSSY VIEW. USE [`coalesce_spans`] FOR TOTALS.
///
/// A `Range` carries a `u64` length, and exactly one merge result cannot be
/// expressed in one: a span covering the WHOLE address space is 2^64 bytes and
/// this function reports it as `u64::MAX`, one byte short. Nothing a tracer can
/// record reaches that width and no threshold this tool ships gets near it, but
/// the case exists and is pinned by
/// `the_range_shaped_view_is_a_byte_short_only_for_a_whole_space_span`.
/// [`coalesce_spans`] returns the same merge as exact `u128` half-open spans and
/// is what every published number goes through.
pub fn coalesce_with_gap(mut ranges: Vec<Range>, gap: u64) -> Vec<Range> {
    ranges.sort_unstable();
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.len == 0 {
            continue;
        }
        match merged.last_mut() {
            // `<=` merges adjacency as well as overlap; `+ gap` extends that to
            // a bridgeable hole. At `gap == 0` the term vanishes and this is the
            // original adjacency rule, byte for byte.
            Some(last) if u128::from(range.address) <= end_excl(*last) + u128::from(gap) => {
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

/// Everything one bracket's entry stream yields: the census row, plus the
/// provenance a copy-recipe analysis needs and the row alone cannot supply.
///
/// This exists so the census table and the gap/window analysis cannot drift:
/// [`Census::build`] and the curve tool both go through [`walk_bracket`], so a
/// change to how a read is counted, deduplicated or rooted moves both at once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BracketWalk {
    /// The census row, identical to the one [`Census`] publishes for this tag.
    pub row: BracketCensus,
    /// One [`Range`] per non-empty read EVENT, in sequence order and
    /// undeduplicated. Feeding these to [`coalesce_with_gap`] reproduces the
    /// row's `coalesced_ranges` at gap 0.
    pub distinct: std::collections::BTreeSet<(u64, u64)>,
    /// Raw per-event ranges. Kept undeduplicated because every consumer here
    /// deduplicates with its own rule.
    pub ranges: Vec<Range>,
    /// Distinct addresses of reads the derivation could NOT explain — the
    /// bracket's discovery roots. `row.root_reads` counts the EVENTS; this
    /// counts the addresses, which is the anchor set a per-root window recipe
    /// would actually place windows on.
    pub root_addresses: Vec<u64>,
    /// Distinct heap-shaped pointer VALUES this bracket's reads returned,
    /// ascending. These are the object bases the reader dereferenced, i.e. the
    /// other candidate anchor set for a window recipe.
    pub pointer_values: Vec<u64>,
}

/// Reduce one bracket's entries. `group` MUST already be sorted by `seq`:
/// depth is causal, and a pointer can only explain a read recorded after it.
pub fn walk_bracket(bracket_tag: u16, group: &[TraceEntry], span: DerivationSpan) -> BracketWalk {
    let mut row = BracketCensus {
        bracket_tag,
        ..Default::default()
    };
    let mut distinct: std::collections::BTreeSet<(u64, u64)> = std::collections::BTreeSet::new();
    let mut ranges: Vec<Range> = Vec::new();
    let mut root_addresses: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();

    // Observed pointer value -> depth of the READ that produced it.
    // Shallowest wins: a value seen at depth 2 and again at depth 5
    // explains its consumers at depth 3, not 6.
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

/// Walk every bracket in a sidecar, keyed by tag. Same grouping and per-group
/// sort as [`Census::build`].
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

/// The gap thresholds the curve reports by default. 16/32/64/128 are the four
/// the floor re-pricing named; the powers of two up to 4096 exist because the
/// per-family window recipes it proposed reach that far.
pub const DEFAULT_GAP_THRESHOLDS: [u64; 9] = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

/// A half-open `[start, end)` span in `u128`.
///
/// EVERY PUBLISHED BYTE TOTAL IS MEASURED IN THIS FORM, not from a `Range`'s
/// `len`. A merged span can be 2^64 bytes wide — the whole address space — and
/// that width does not fit in the `u64` a `Range` carries, so
/// [`coalesce_with_gap`] clamps it. Nothing a tracer can record reaches that
/// width, but the module's standard is that union arithmetic is exact rather
/// than conditionally exact, so the curve and the coverage figures never route
/// a total through the clamped length.
pub type Span = (u128, u128);

/// Merge raw ranges into exact sorted disjoint spans, bridging holes of at most
/// `gap`. Zero-length ranges are dropped, matching [`coalesce_with_gap`].
///
/// This is [`coalesce_with_gap`] without the `u64` length, and it is the
/// function a caller that needs a TOTAL should use.
pub fn coalesce_spans(ranges: &[Range], gap: u64) -> Vec<Span> {
    let mut sorted: Vec<Range> = ranges.iter().copied().filter(|range| range.len > 0).collect();
    sorted.sort_unstable();
    let mut out: Vec<Span> = Vec::with_capacity(sorted.len());
    for range in sorted {
        let start = u128::from(range.address);
        let end = end_excl(range);
        match out.last_mut() {
            // `<=` merges adjacency and overlap; `+ gap` bridges a hole.
            Some(last) if start <= last.1 + u128::from(gap) => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// The same merge over spans that are ALREADY sorted and disjoint — the shape
/// [`coalesce_spans`] returns. Feeding it anything else is a caller bug.
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

/// Measure of a disjoint span list, exact.
pub fn span_measure(spans: &[Span]) -> u128 {
    spans.iter().map(|(start, end)| end - start).sum()
}

/// Render an exact `u128` byte total as JSON.
///
/// JSON numbers here are `u64`. A total that does not fit becomes a decimal
/// STRING rather than a truncated number, because a silently wrong figure in a
/// published table is worse than an inconveniently typed one. This lives in the
/// library rather than the binary for the reason stated in the binary's own
/// header: the arithmetic a decision rests on must not sit where no test
/// reaches it.
pub fn exact_json_number(value: u128) -> serde_json::Value {
    match u64::try_from(value) {
        Ok(fits) => fits.into(),
        Err(_) => value.to_string().into(),
    }
}

/// One point on a gap-merge curve: what a copier that may bridge holes of at
/// most `gap` bytes would have to issue, and move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GapCurvePoint {
    /// Largest hole this policy bridges. See [`coalesce_with_gap`]: the
    /// comparison is `<=`.
    pub gap: u64,
    /// Copy descriptors after merging.
    pub ranges: u64,
    /// Bytes the merged descriptors span, INCLUDING bridged holes. `u128`
    /// because the exact value can be 2^64; see [`Span`].
    pub bytes: u128,
    /// `bytes` minus the gap-0 union: the padding this policy actually commits,
    /// measured rather than bounded.
    pub over_copy_bytes: u128,
    /// The bound a reader gets with no trace in hand: every one of the gap-0
    /// ranges merges and every bridge costs the full threshold. Always `>=`
    /// `bytes`; the distance between them is how pessimistic the bound is.
    pub worst_case_envelope_bytes: u128,
}

/// A whole curve for one range set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GapCurve {
    /// What the range set is: `"GLOBAL"`, `"tag 42"`, `"SUM OF BRACKETS"`, …
    pub label: String,
    /// Distinct `(address, len)` pairs behind the set, when the caller knows
    /// them; zero when the set was handed over already reduced.
    pub distinct_ranges: u64,
    /// Copy descriptors at gap 0 — adjacency merge only.
    pub baseline_ranges: u64,
    /// Union measure at gap 0: the bytes that are genuinely read.
    pub baseline_bytes: u128,
    pub points: Vec<GapCurvePoint>,
}

/// The no-trace upper bound on a gap policy's byte cost: assume every hole
/// between the `baseline_ranges` gap-0 spans is bridged and every bridge is the
/// full `gap`. This is the arithmetic behind the floor re-pricing's
/// "full image worst case" column, restated here so the measured curve can be
/// checked against the bound it was supposed to respect.
///
/// An N-span list has N-1 INTERIOR holes, so the bound is
/// `baseline_bytes + gap*(N-1)` and a single span cannot be padded at all.
pub fn worst_case_envelope_bytes(baseline_bytes: u128, baseline_ranges: u64, gap: u64) -> u128 {
    let bridges = u128::from(baseline_ranges.saturating_sub(1));
    baseline_bytes + bridges * u128::from(gap)
}

/// Compute a gap-merge curve over `ranges`.
///
/// `ranges` is the RAW, undeduplicated per-read set for whatever scope the
/// caller means. Deduplication happens here, once, at gap 0, and every
/// threshold is then applied to that gap-0 set. That is not an optimisation
/// with a different answer: merging under "holes ≤ g" over the connected
/// components of "holes = 0" yields the connected components of "holes ≤ g",
/// which is what `coalesce_with_gap` on the raw input produces. The tests pin
/// that equivalence.
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

/// Add two curves point-for-point. Used to build the WITHIN-BRACKET-ONLY
/// variant, where each bracket is reduced against itself and the rows are then
/// summed — the cost of a copy plan that gives every bracket its own image.
///
/// `gaps` is passed in rather than read off the first curve, deliberately: a
/// trace with NO brackets must still report a row per requested threshold, and
/// taking the thresholds from `curves[0]` silently returns an empty table in
/// exactly that case — a table that reads as "no thresholds were asked for"
/// rather than "every threshold costs nothing".
///
/// Panics only on mismatched thresholds, which is a caller bug rather than a
/// data condition.
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

/// Both deduplication scopes of the same trace, computed together so a reader
/// cannot pick one up believing it is the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GapCurveReport {
    /// DEDUPLICATED ACROSS BRACKETS FIRST, then merged. This is the scope a
    /// central copy plan needs: one witness image serving every domain, so a
    /// byte two brackets read is copied once.
    pub global: GapCurve,
    /// One curve per bracket tag, each deduplicated against ITSELF only.
    /// Ordered by tag.
    pub per_bracket: Vec<GapCurve>,
    /// The per-bracket curves added up: the cost of the other plan, where every
    /// bracket gets its own image and shared bytes are copied once per bracket.
    pub summed_per_bracket: GapCurve,
}

/// Build both scopes from a walked trace.
///
/// THE SCOPES ARE NOT INTERCHANGEABLE. `global` unions every bracket's raw read
/// ranges before any merging; `per_bracket` never lets one bracket's span merge
/// with another's. On t35 the difference is roughly a factor of two in bytes,
/// because per-bracket sums double-count everything shared.
pub fn gap_curve_report(walks: &BTreeMap<u16, BracketWalk>, gaps: &[u64]) -> GapCurveReport {
    let mut all_ranges: Vec<Range> = Vec::new();
    let mut all_distinct: std::collections::BTreeSet<(u64, u64)> = std::collections::BTreeSet::new();
    let mut per_bracket = Vec::with_capacity(walks.len());
    for (bracket_tag, walk) in walks {
        all_ranges.extend_from_slice(&walk.ranges);
        all_distinct.extend(walk.distinct.iter().copied());
        // Each bracket's curve is built from ITS OWN ranges. Hoisting this to
        // `all_ranges` would silently turn the per-bracket scope into a second
        // copy of the global one.
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

/// What a fixed-size-window-per-anchor copy recipe would and would not cover.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowCoverage {
    /// Distinct anchor addresses the recipe places a window on.
    pub anchors: u64,
    /// Window size, in bytes, placed at `[anchor, anchor + window_len)`.
    pub window_len: u64,
    /// Copy descriptors after the windows themselves are coalesced.
    pub window_ranges: u64,
    /// Union measure of the windows: what the recipe moves. `u128` for the same
    /// reason [`GapCurvePoint::bytes`] is.
    pub window_bytes: u128,
    /// Union measure of the data the bracket actually read.
    pub data_bytes: u128,
    /// Bytes of that data the windows contain.
    pub covered_bytes: u128,
    /// Gap-0 data spans, and how they fall against the windows.
    pub data_ranges: u64,
    pub fully_covered_ranges: u64,
    pub partially_covered_ranges: u64,
    pub uncovered_ranges: u64,
}

impl WindowCoverage {
    /// Fraction of the read bytes the recipe would have captured, in percent.
    pub fn byte_coverage_pct(&self) -> f64 {
        if self.data_bytes == 0 {
            return 100.0;
        }
        self.covered_bytes as f64 * 100.0 / self.data_bytes as f64
    }

    /// Fraction of the read spans the recipe captures WHOLE. A partially
    /// covered span is a miss for a recipe: the reader still needs the rest.
    pub fn range_coverage_pct(&self) -> f64 {
        if self.data_ranges == 0 {
            return 100.0;
        }
        self.fully_covered_ranges as f64 * 100.0 / self.data_ranges as f64
    }
}

/// Measure a `[anchor, anchor + window_len)`-per-anchor recipe against the data
/// a bracket actually read.
///
/// Both sides are reduced to gap-0 disjoint spans first, so containment in the
/// window UNION is the same question as containment in one window span.
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

    // Two-pointer sweep over two sorted disjoint span lists.
    //
    // `window_index` may only advance past windows that END at or before the
    // CURRENT span's start. Data spans are sorted ascending, so such a window
    // also ends at or before every later span's start and can never be needed
    // again. `probe` then scans forward from there without moving the shared
    // cursor, so a window overlapping several spans is still seen by each.
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

    // ----------------------------------------------------------------------
    // GAP-MERGE CURVE
    // ----------------------------------------------------------------------

    /// The four spans below are laid out so every threshold in the assertion
    /// has a different answer, and each answer is arithmetic done here in the
    /// comment rather than copied out of a run:
    ///
    /// ```text
    ///   A 0x1000..0x1008        gap A->B = 0x1010-0x1008 =  8
    ///   B 0x1010..0x1018        gap B->C = 0x1030-0x1018 = 24
    ///   C 0x1030..0x1038        gap C->D = 0x1080-0x1038 = 72
    ///   D 0x1080..0x1088
    /// ```
    ///
    /// Production change that fails this: `<` instead of `<=` in
    /// [`coalesce_with_gap`] — every threshold that exactly equals a hole
    /// (8, 24, 72) would stop merging and report the row above it.
    #[test]
    fn the_gap_curve_is_the_hand_computed_one_and_the_comparison_is_inclusive() {
        let ranges = vec![
            Range { address: 0x1000, len: 8 },
            Range { address: 0x1010, len: 8 },
            Range { address: 0x1030, len: 8 },
            Range { address: 0x1080, len: 8 },
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

        assert_eq!((at(0).ranges, at(0).bytes), (4, 32), "gap 0 is plain coalescing");
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
        assert_eq!((at(23).ranges, at(23).bytes), (3, 40), "24 is still too wide at 23");
        assert_eq!(
            (at(24).ranges, at(24).bytes),
            (2, 64),
            "A..C is 0x1000..0x1038 = 56 bytes, plus D's 8"
        );
        assert_eq!((at(71).ranges, at(71).bytes), (2, 64), "72 is still too wide at 71");
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

    /// The staged computation `gap_curve` uses — deduplicate once at gap 0,
    /// then merge that set at each threshold — must give the same answer as
    /// merging the RAW read stream at each threshold directly.
    ///
    /// The two paths share a merge rule, so this ALONE cannot catch a defect in
    /// that rule; `the_curve_reduces_unsorted_nested_and_empty_input_by_hand`
    /// below is the independent known answer that can. What this catches is
    /// staging that is not equivalent: applying thresholds cumulatively in a way
    /// that is not monotone, or dropping the gap-0 reduction and letting
    /// duplicate reads inflate the range count.
    #[test]
    fn staged_reduction_equals_merging_the_raw_stream_at_each_threshold() {
        // Duplicates, an overlap, an exact adjacency and three different holes.
        let raw = vec![
            Range { address: 0x9000, len: 16 },
            Range { address: 0x9000, len: 16 },
            Range { address: 0x9008, len: 16 },
            Range { address: 0x9018, len: 8 },
            Range { address: 0x9030, len: 4 },
            Range { address: 0x9100, len: 32 },
            Range { address: 0x9130, len: 1 },
        ];
        let gaps = [0u64, 1, 4, 8, 16, 32, 64, 128, 256];
        let curve = gap_curve("staged", &raw, &gaps);
        for gap in gaps {
            let direct = coalesce_with_gap(raw.clone(), gap);
            let direct_bytes = direct.iter().map(|range| u128::from(range.len)).sum::<u128>();
            let point = curve.points.iter().find(|p| p.gap == gap).expect("point");
            assert_eq!(
                (point.ranges, point.bytes),
                (direct.len() as u64, direct_bytes),
                "staged and direct merging disagree at gap {gap}"
            );
        }
    }

    /// The curve's merge rule against a HAND-COMPUTED answer, on input chosen
    /// so each structural hazard has a different consequence. Nothing here is
    /// derived from another call into the same code.
    ///
    /// ```text
    ///   input, in this order:  (100,10)  (0,10)  (10,1)  (150,0)  (0,100)  (200,10)
    ///   sorted, zero dropped:  (0,10) (0,100) (10,1) (100,10) (200,10)
    ///   gap 0 -> 0..110 and 200..210            = 2 spans, 120 bytes
    ///   gap 89 -> the hole 110..200 is 90 wide  = 2 spans, 120 bytes
    ///   gap 90 -> bridged                       = 1 span,  210 bytes
    /// ```
    ///
    /// Production changes that fail this, each of which the equivalence test
    /// above would pass because both of its paths would move together:
    ///  * dropping the sort — the leading (100,10) would anchor the first span
    ///    and (0,10) would be appended after it;
    ///  * taking the new end instead of `max(old, new)` — the NESTED (10,1)
    ///    would truncate 0..100 to 0..11;
    ///  * keeping zero-length ranges — (150,0) sits ALONE in the hole between
    ///    110 and 200, deliberately, so retaining it makes a THIRD baseline
    ///    span. Put inside an existing span it would have been invisible.
    #[test]
    fn the_curve_reduces_unsorted_nested_and_empty_input_by_hand() {
        let raw = vec![
            Range { address: 100, len: 10 },
            Range { address: 0, len: 10 },
            Range { address: 10, len: 1 },
            Range { address: 150, len: 0 },
            Range { address: 0, len: 100 },
            Range { address: 200, len: 10 },
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

        // An empty set has no spans, no bytes and no bound, at every threshold.
        let empty = gap_curve("empty", &[], &[0, 64]);
        assert_eq!(empty.baseline_ranges, 0);
        assert_eq!(empty.baseline_bytes, 0);
        assert_eq!(empty.points.len(), 2, "a threshold asked for is a row reported");
        assert!(empty.points.iter().all(|p| p.ranges == 0 && p.bytes == 0));
    }

    /// A merged span can be the WHOLE address space, 2^64 bytes, which no `u64`
    /// length can hold. The curve's totals must still be exact.
    ///
    /// Production change that fails this: taking the byte totals from
    /// `coalesce_with_gap`'s `Range::len` (which clamps to `u64::MAX`) instead
    /// of from the `u128` span arithmetic — the answer would be one byte short,
    /// and `worst_case_envelope_bytes` would saturate to the same wrong value.
    #[test]
    fn a_merge_spanning_the_entire_address_space_is_still_counted_exactly() {
        let raw = vec![
            Range { address: 0, len: 1 },
            Range { address: u64::MAX, len: 1 },
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

    /// The two deduplication scopes are different measurements and the tool
    /// must not confuse them: the GLOBAL curve unions across bracket tags
    /// first, the per-bracket curve reduces each tag against itself and the
    /// rows are then summed.
    ///
    /// Production change that fails this: pooling every bracket's ranges into
    /// one vector before building the per-bracket curves (bytes would collapse
    /// from 16 to 24 and ranges from 2 to 1 below), or unioning the global
    /// curve per bracket.
    #[test]
    fn per_bracket_curves_never_merge_across_the_dedup_boundary() {
        // Two brackets, eight bytes apart, so a gap of 8 WOULD bridge them if
        // the scopes were pooled.
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

    /// EVERY field `sum_curves` adds, pinned to a hand-computed answer, because
    /// a summed table that quietly drops one column reads as a real measurement.
    ///
    /// ```text
    ///   tag 1: reads (0,1) and (3,1)   -> gap 0: 2 spans, 2 B; gap 2: 1 span 0..4 = 4 B
    ///   tag 2: reads (100,1) and (103,1) -> the same, shifted
    ///   each bracket's bound at gap 2 is 2 bytes + 2*(2 spans - 1) = 4
    ///   sum at gap 2: 2 ranges, 8 bytes, over-copy 4, envelope 4+4 = 8
    ///   baselines: 4 spans, 4 bytes, 4 distinct (address,len) pairs
    /// ```
    ///
    /// Production change that fails this: dropping any one of the
    /// `distinct_ranges` / `baseline_ranges` / `baseline_bytes` /
    /// `over_copy_bytes` / `worst_case_envelope_bytes` accumulations.
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
        assert_eq!(summed.distinct_ranges, 4, "four distinct (address,len) pairs");
        assert_eq!(summed.baseline_ranges, 4, "two spans in each of two brackets");
        assert_eq!(summed.baseline_bytes, 4, "four one-byte reads");
        let at2 = summed.points.iter().find(|p| p.gap == 2).unwrap();
        assert_eq!(at2.ranges, 2, "each bracket's pair bridges its own 2-byte hole");
        assert_eq!(at2.bytes, 8, "0..4 and 100..104");
        assert_eq!(at2.over_copy_bytes, 4, "two bridged holes of 2 bytes each");
        assert_eq!(
            at2.worst_case_envelope_bytes, 8,
            "each bracket read 2 bytes over 2 spans, so its bound is \
             2 + 2*(2-1) = 4, and the pair sums to 8"
        );
    }

    /// A trace with NO brackets still reports the thresholds it was asked for,
    /// in BOTH scopes. An empty table is a different claim from a zero table.
    ///
    /// Production change that fails this: taking `sum_curves`' threshold list
    /// off `curves[0]` instead of the caller's `gaps`.
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

    /// Cross-bracket SHARING is deduplicated by the global curve exactly once,
    /// mirroring `full_set_unique_bytes`. The global curve at gap 0 must equal
    /// the census's own full-set figures on the same trace.
    ///
    /// Production change that fails this: building the global range set from
    /// the per-bracket COALESCED spans of a bundled row, or from the distinct
    /// `(address, len)` set (which would drop the overlap arithmetic).
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

    /// The re-pricing's published "full image worst case" column is this
    /// formula and nothing else. Pinning it here is what lets the report say
    /// the published numbers were a BOUND rather than a measurement, with the
    /// arithmetic shown.
    ///
    /// Production change that fails this: using `baseline_ranges` instead of
    /// `baseline_ranges - 1` for the bridge count (an interval list of N spans
    /// has N-1 interior holes).
    #[test]
    fn the_worst_case_envelope_is_baseline_plus_gap_times_interior_holes() {
        // t35's measured full set: 61,606,245 bytes over 504,423 gap-0 spans.
        assert_eq!(
            worst_case_envelope_bytes(61_606_245, 504_423, 16),
            69_676_997,
            "61,606,245 + 16*504,422 = 69,676,997 = 66.45 MiB, the published 16 B row"
        );
        assert_eq!(worst_case_envelope_bytes(61_606_245, 504_423, 32), 77_747_749);
        assert_eq!(worst_case_envelope_bytes(61_606_245, 504_423, 64), 93_889_253);
        assert_eq!(worst_case_envelope_bytes(61_606_245, 504_423, 128), 126_172_261);
        assert_eq!(
            worst_case_envelope_bytes(100, 1, 4096),
            100,
            "a single span has no interior hole, so no threshold can pad it"
        );
        assert_eq!(worst_case_envelope_bytes(100, 0, 4096), 100, "and neither can none");
    }

    /// The `Range`-shaped view is lossy in EXACTLY ONE case and this pins both
    /// halves of that claim: it is exact everywhere a length fits in a `u64`,
    /// and one byte short only for a span covering the whole address space —
    /// where [`coalesce_spans`] is exact.
    ///
    /// Production change that fails this: making `coalesce_with_gap` lossy
    /// anywhere else (e.g. saturating the exclusive end in `u64`), or making
    /// `coalesce_spans` inherit the clamp.
    #[test]
    fn the_range_shaped_view_is_a_byte_short_only_for_a_whole_space_span() {
        let whole = vec![
            Range { address: 0, len: 1 },
            Range { address: u64::MAX, len: 1 },
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

        // One byte narrower and the two agree exactly, which is every case a
        // trace can actually produce.
        let narrower = vec![
            Range { address: 1, len: 1 },
            Range { address: u64::MAX, len: 1 },
        ];
        let lossy = coalesce_with_gap(narrower.clone(), u64::MAX - 1);
        assert_eq!(lossy.len(), 1);
        assert_eq!(u128::from(lossy[0].len), u128::from(u64::MAX));
        assert_eq!(
            span_measure(&coalesce_spans(&narrower, u64::MAX - 1)),
            u128::from(u64::MAX)
        );
    }

    /// Summing curves built for DIFFERENT thresholds would publish one gap's
    /// totals under another's label. That must abort, not be tolerated.
    ///
    /// Production change that fails this: deleting the per-point `gap` assert
    /// in `sum_curves`.
    #[test]
    #[should_panic(expected = "threshold lists disagree")]
    fn summing_curves_built_for_different_thresholds_aborts() {
        let curve = gap_curve("tag 1", &[Range { address: 0, len: 1 }], &[0]);
        let _ = sum_curves("mismatched", &[curve], &[64]);
    }

    /// An exact `u128` total that does not fit a JSON number becomes a decimal
    /// STRING. A truncating cast would publish 2^64 as 0.
    ///
    /// Production change that fails this: `Err(_) => (value as u64).into()`.
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

    // ----------------------------------------------------------------------
    // WINDOW COVERAGE
    // ----------------------------------------------------------------------

    /// A per-anchor fixed window either contains a read span whole, clips it,
    /// or misses it, and the recipe question is which. Hand-computed:
    ///
    /// ```text
    ///   window at 0x4000, 64 bytes -> 0x4000..0x4040
    ///   window at 0x5000, 64 bytes -> 0x5000..0x5040
    ///   read 0x4000..0x4008  inside window 1                -> FULL,  8 bytes
    ///   read 0x4038..0x4048  window 1 ends at 0x4040        -> PARTIAL, 8 of 16
    ///   read 0x4040..0x4048  starts exactly at the end      -> MISS,   0 bytes
    ///   read 0x5000..0x5008  inside window 2                -> FULL,   8 bytes
    /// ```
    ///
    /// The 0x4040 read coalesces into the 0x4038 span here, so the exact-end
    /// boundary is exercised standalone by
    /// `a_read_starting_at_a_windows_exclusive_end_is_a_miss` instead.
    ///
    /// Production change that fails this: counting a CLIPPED span as covered,
    /// or dropping the anchor deduplication.
    #[test]
    fn window_coverage_separates_whole_spans_from_clipped_and_missed_ones() {
        let data = vec![
            Range { address: 0x4000, len: 8 },
            Range { address: 0x4038, len: 16 },
            Range { address: 0x4040, len: 8 },
            Range { address: 0x5000, len: 8 },
        ];
        // 0x4038..0x4048 and 0x4040..0x4048 coalesce into 0x4038..0x4048.
        let coverage = window_coverage(&data, &[0x4000, 0x5000, 0x5000], 64);
        assert_eq!(coverage.anchors, 2, "the repeated 0x5000 anchor is one window");
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
        assert!((coverage.byte_coverage_pct() - 75.0).abs() < 1e-9, "24 of 32 bytes");
        assert!(
            (coverage.range_coverage_pct() - (2.0 / 3.0 * 100.0)).abs() < 1e-9,
            "a clipped span is NOT covered for recipe purposes"
        );
    }

    /// A window wide enough for every read covers everything, and a window of
    /// zero bytes covers nothing — the two ends of the recipe question.
    #[test]
    fn window_size_moves_coverage_between_the_two_extremes() {
        let data = vec![
            Range { address: 0x4000, len: 8 },
            Range { address: 0x4040, len: 8 },
        ];
        let wide = window_coverage(&data, &[0x4000], 0x1000);
        assert_eq!(wide.covered_bytes, 16);
        assert_eq!(wide.fully_covered_ranges, 2);
        assert_eq!(wide.window_bytes, 0x1000, "and it moves 4096 bytes to get 16");

        let none = window_coverage(&data, &[0x4000], 0);
        assert_eq!(none.window_ranges, 0, "a zero-length window is not a copy");
        assert_eq!(none.covered_bytes, 0);
        assert_eq!(none.uncovered_ranges, 2);
    }

    /// A window is HALF-OPEN. A read starting at exactly `anchor + W` is
    /// outside it, and nothing else in the fixture may rescue that byte.
    ///
    /// Production change that fails this: `<=` on the window's exclusive end in
    /// the sweep, which would report this recipe as reaching a span it does not.
    #[test]
    fn a_read_starting_at_a_windows_exclusive_end_is_a_miss() {
        let coverage = window_coverage(&[Range { address: 0x4040, len: 8 }], &[0x4000], 64);
        assert_eq!(coverage.window_ranges, 1);
        assert_eq!(coverage.window_bytes, 64, "0x4000..0x4040");
        assert_eq!(coverage.data_ranges, 1);
        assert_eq!(coverage.covered_bytes, 0, "0x4040 is the first byte OUTSIDE");
        assert_eq!(coverage.fully_covered_ranges, 0);
        assert_eq!(coverage.partially_covered_ranges, 0);
        assert_eq!(coverage.uncovered_ranges, 1);
        assert_eq!(coverage.byte_coverage_pct(), 0.0);
        assert_eq!(coverage.range_coverage_pct(), 0.0);

        // One byte earlier the same window DOES reach it, so the assertion
        // above is about the boundary and not about the window being too small.
        let inside = window_coverage(&[Range { address: 0x403F, len: 1 }], &[0x4000], 64);
        assert_eq!(inside.covered_bytes, 1);
        assert_eq!(inside.fully_covered_ranges, 1);
    }

    /// A window shared by several data spans must be seen by all of them: the
    /// sweep's cursor may not consume it on the first.
    ///
    /// Production change that fails this: advancing `window_index` inside the
    /// inner probe loop, which would leave later spans uncovered.
    #[test]
    fn one_window_covering_several_spans_is_seen_by_each_of_them() {
        let data = vec![
            Range { address: 0x100, len: 4 },
            Range { address: 0x140, len: 4 },
            Range { address: 0x180, len: 4 },
        ];
        let coverage = window_coverage(&data, &[0x100], 0x100);
        assert_eq!(coverage.window_ranges, 1, "one window 0x100..0x200");
        assert_eq!(coverage.data_ranges, 3);
        assert_eq!(coverage.fully_covered_ranges, 3, "all three lie inside it");
        assert_eq!(coverage.covered_bytes, 12);
        assert_eq!(coverage.uncovered_ranges, 0);
    }

    /// The coverage figures are exact at the same edge the curve is: a window
    /// union can be 2^64 bytes wide, which no `u64` can hold.
    ///
    /// Production change that fails this: computing `window_bytes` (or
    /// `data_bytes`, or `covered_bytes`) from `coalesce(...)`'s clamped
    /// `Range::len` instead of the span arithmetic — every one of them would
    /// report 2^64-1.
    #[test]
    fn window_and_data_totals_are_exact_across_the_whole_address_space() {
        // Two anchors one byte apart, each with a window as wide as the space:
        // their union is 0..2^64, the entire address space.
        let data = vec![
            Range { address: 0, len: 1 },
            Range { address: u64::MAX, len: 1 },
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

        // And the DATA union alone can be that wide too.
        let all_data = window_coverage(
            &[Range { address: 0, len: u64::MAX }, Range { address: u64::MAX, len: 1 }],
            &[],
            64,
        );
        assert_eq!(all_data.data_ranges, 1);
        assert_eq!(all_data.data_bytes, 1u128 << 64);
        assert_eq!(all_data.covered_bytes, 0, "no anchors, no windows");
        assert_eq!(all_data.uncovered_ranges, 1);
    }

    /// Coverage of nothing is 100%, not `NaN`. A recipe that has no data to
    /// miss has not missed anything, and a `NaN` would sort and compare wrongly
    /// in any table built from these rows.
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

    // ----------------------------------------------------------------------
    // WALK / CENSUS FIDELITY
    // ----------------------------------------------------------------------

    /// Depth is CAUSAL, so the per-tag sort by `seq` is load-bearing, and a
    /// test built from already-ordered input cannot see it. These entries are
    /// in the reverse of their sequence order — the tracer's file order is
    /// per-thread flush order, so this is the real shape, not a contrivance —
    /// and the expected row is hard-coded rather than compared against a second
    /// caller of the same code.
    ///
    /// Production change that fails this: removing the `sort_unstable_by_key`
    /// in `walk_all` and/or `Census::build`. Unsorted, the POINTER at seq 1 is
    /// consumed before the READ at seq 0 exists, so 0x2000 looks like a root:
    /// `max_depth` falls to 0 and `root_reads` rises to 2.
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

        // And the census agrees, which is the point of the shared walk.
        let census = Census::build(header(), &entries, span);
        assert_eq!(census.brackets, vec![walk.row.clone()]);
    }

    /// `walk_bracket` is the single implementation behind both the census
    /// table and the curve, so the two can never disagree about what a bracket
    /// read. This asserts the identity on a trace with re-reads, an overlap,
    /// two brackets and a pointer chain.
    ///
    /// Production change that fails this: giving the curve its own grouping or
    /// its own dedup, which is exactly the drift this refactor removes.
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

    /// Roots are counted per EVENT and anchored per ADDRESS, and the two
    /// differ whenever a root is re-read. A window recipe places one window per
    /// address, so conflating them would over-count the recipe's cost.
    #[test]
    fn a_re_read_root_is_many_root_reads_but_one_anchor() {
        let entries: Vec<TraceEntry> = (0..4).map(|i| read(29, i, 0x6000, 8)).collect();
        let walks = walk_all(&entries, DerivationSpan(0x1000));
        let walk = &walks[&29];
        assert_eq!(walk.row.root_reads, 4);
        assert_eq!(walk.root_addresses, vec![0x6000]);
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
