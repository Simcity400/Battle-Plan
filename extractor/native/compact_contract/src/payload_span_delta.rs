//! `PSD1` — a wire delta for whole DOMAIN_DATA payloads (lane PS, 2026-08-21).
//!
//! # What this is
//!
//! Domain 100 ships [`crate::xmap07_snow_mud`]'s XSM2, which spells one hour's
//! snow/mud grid as a keyframe, a no-change header, or the 16x16 tiles that
//! moved. This module is the same idea for domains whose payload is NOT a grid:
//! a byte payload of any grammar, spelled as a keyframe, a no-change header, or
//! a copy/literal program against a named baseline. The domain's own grammar is
//! untouched — `PSD1` wraps the payload the domain already produces, and the
//! consumer unwraps it back to exactly those bytes before anything else looks at
//! them.
//!
//! # The declared volatile header, and why the basis excludes it
//!
//! `elapsed_wall_ms` is a `u64` of RECORDER wall-clock time stamped at offset 0
//! of ~42 domain payload grammars (lane PN,
//! `extractor/native/runs/pn_structural/MEASUREMENT.md` §3c). It is recorder
//! telemetry, not engine state, and it is the same value for every domain in a
//! frame — so it moves every hour whatever the game did, and a whole-payload
//! identity gate over the sealed `captures/oz_tip` therefore caught 4.2% of the
//! spool while a gate that skipped a leading header caught 13.8%.
//!
//! `PSD1` carries those 8 bytes VERBATIM in its own header, and diffs only
//! `payload[VOLATILE_HEADER_LEN..]` — the BASIS. Nothing about the field's
//! meaning changes: it rides every record exactly as it did, at full width, and
//! the reconstructed payload is byte-identical to the one the domain encoded.
//! It is excluded from the *comparison*, not from the *wire*.
//!
//! # The three bodies, and the measurement that chose them
//!
//! Measured over both sealed spools that still carry their collector stream
//! (`extractor/native/runs/ps_wire_delta/`), for the three domains lane PN
//! priced as the largest remaining size prize:
//!
//! | | payload | hours whose BASIS is identical | hours whose length moved |
//! |---|---|---|---|
//! | d53 `captures/oz_tip` | 1,329 MB | 166 / 460 | 37 |
//! | d43 `captures/oz_tip` | 237 MB | 81 / 471 | 83 |
//! | d41 `captures/oz_tip` | 1.8 MB | 0 / 472 | 46 |
//!
//! So:
//!
//! * [`PayloadSpanDeltaBody::NoChange`] — the basis is bit-identical to the
//!   baseline. The record is a header and nothing else. 166 of d53's 460 hours.
//! * [`PayloadSpanDeltaBody::Delta`] — a COPY/LITERAL program against the
//!   baseline basis. Copy/literal and not fixed blocks, and not a
//!   prefix/suffix splice, because the measurement rejected both:
//!   - a 256-byte block delta cost d41 **more** than the whole payload (its
//!     changed bytes are ~36 per hour, so a block delta pays 512 bytes to carry
//!     36), while a span delta costs 69 bytes a hour;
//!   - both are defeated by a length change, which SHIFTS every byte after the
//!     insertion. On `captures/oz_tip` d53 that is 36 hours out of 460, and
//!     under a prefix/suffix splice those 36 hours cost **103.7 MB of the
//!     107.0 MB** the whole domain would have cost — 97% of the residual, because
//!     the payload's leading row-count fields move too and kill the common
//!     prefix at byte ~4. A copy program is indifferent to a shift.
//! * [`PayloadSpanDeltaBody::Keyframe`] — the whole payload. Emitted with no
//!   baseline, and whenever a delta would not actually be smaller.
//!
//! # Why a retained baseline cannot corrupt the fold
//!
//! Exactly XSM2's rule. Every non-keyframe record names the baseline it was
//! built against by digest ([`PayloadSpanDeltaRecord::base_basis_sha256`]);
//! [`apply_to_baseline`] refuses unless the baseline it is handed hashes to that
//! value, and then re-verifies that the RECONSTRUCTED basis hashes to
//! `basis_sha256`. A consumer whose baseline is stale — a dropped record, a
//! gapped frame, a producer-side reset the consumer did not see — gets a named
//! refusal for that hour rather than a payload nobody observed, and the next
//! keyframe re-syncs it.
//!
//! # Fail-closed by construction
//!
//! * a delta that names a baseline the consumer does not hold refuses BY NAME
//!   (`PSD1 has no retained baseline ...`);
//! * an unassigned body kind, an unassigned status bit, a copy that reaches
//!   outside the baseline, a program that does not reproduce
//!   `reconstructed_len`, and a reconstruction whose digest disagrees are each
//!   an `Err`, never a guess;
//! * a domain-hour that cannot be delta'd emits a KEYFRAME. There is no third
//!   outcome in which an hour is dropped to save bytes.
//!
//! # Where it runs
//!
//! The producer side runs at EMISSION, on the collector thread, after
//! `thread::scope` has joined every reader — i.e. after every
//! `ReaderClockBracketGuard` has closed. It performs no engine reads at all: its
//! only inputs are the payload the domain already encoded and the producer's own
//! retained baseline. In-bracket cost is zero by construction.

use sha2::{Digest, Sha256};

pub const MAGIC: [u8; 4] = *b"PSD1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 112;

/// The declared volatile header: `elapsed_wall_ms`, a `u64` at payload offset 0.
///
/// Carried verbatim on the wire, excluded from the comparison basis. See the
/// module docs.
pub const VOLATILE_HEADER_LEN: usize = 8;

pub const BODY_KIND_KEYFRAME: u8 = 0;
pub const BODY_KIND_NO_CHANGE: u8 = 1;
pub const BODY_KIND_DELTA: u8 = 2;

pub const OP_COPY: u8 = 0;
pub const OP_LITERAL: u8 = 1;

/// Bytes one encoded op costs before its literal payload.
///
/// `COPY`: tag + u32 source offset + u32 length. `LITERAL`: tag + u32 length,
/// then the bytes.
pub const COPY_OP_LEN: usize = 1 + 4 + 4;
pub const LITERAL_OP_HEADER_LEN: usize = 1 + 4;

/// Match window for the producer's index, in bytes.
///
/// The producer indexes the baseline at every `MATCH_WINDOW`-aligned offset and
/// scans the target with a rolling window of the same width, which is the rsync
/// construction: any run of at least `2 * MATCH_WINDOW - 1` identical bytes must
/// contain a window-aligned baseline block, so every match of 63 bytes or more
/// is found regardless of how far it shifted. Shorter runs fall into literals,
/// which is the right trade — a 32-byte copy op costs 9 bytes to describe.
///
/// This is a PRODUCER tuning constant only. The decoder validates the op stream
/// it is given and never consults this value, so changing it cannot invalidate
/// an already-encoded record.
pub const MATCH_WINDOW: usize = 32;

/// Fail-closed ceiling on a reconstructed payload, matching the record framing's
/// own `COMPACT_MAX_PAYLOAD_LEN`.
///
/// THIS IS NOT THE CEILING THE PRODUCER CHECKS. An envelope is larger than the
/// payload it carries — [`HEADER_LEN`] of header against the 8 bytes of volatile
/// header it replaces — so a payload that fits the record framing exactly does
/// NOT fit once wrapped. [`encode_payload_with_basis_digest`] therefore bounds the
/// ENCODED envelope, not the payload; this constant bounds what a DECODER will
/// admit, which is the other direction.
pub const MAX_RECONSTRUCTED_LEN: usize = 1 << 28;

/// Bytes an envelope adds to the payload it carries: the header, less the
/// volatile header it absorbs.
pub const ENVELOPE_OVERHEAD: usize = HEADER_LEN - VOLATILE_HEADER_LEN;

/// Fail-closed ceiling on the op count of a delta body.
///
/// Every op costs at least [`LITERAL_OP_HEADER_LEN`] bytes of body, so the body
/// length already bounds this; the explicit ceiling exists so the decoder can
/// size its allocation before it trusts anything, the same ordering
/// `xmap07_snow_mud::decode_payload` documents for its tile count.
pub const MAX_OPS: usize = 1 << 22;

/// The domains whose DOMAIN_DATA payloads ride as `PSD1` envelopes.
///
/// 43 `market_trade_live`, 53 `diplomacy_intel_operatives_live`, and 81
/// `rail_replay_state_live`. The first two are two of the three lane PN priced
/// as the largest remaining size prize; d81 is the largest remaining domain
/// after them (`runs/pn_structural/`'s table), put on the same codec by lane QK
/// after lane PW proved its payload splits into a geometry collection that is
/// 93-98% byte-identical hour over hour and a hub collection whose changed
/// bytes are a few thousand scattered inside a ~1 MB payload
/// (`runs/qk_d81_wire_delta/MEASUREMENT.md` §1). The list is here, in the
/// codec, because BOTH sides consult it: the producer to decide what to wrap,
/// the decoder to refuse an envelope naming a domain nobody enabled.
/// 43 `market_trade_live` and 53 `diplomacy_intel_operatives_live` — two of the
/// three lane PN priced as the largest remaining size prize. 47
/// `characters_leaders_cabinet_live` is the second-largest REMAINING domain
/// (lane PN: ~684 KB/hour, ~3.2% hour-over-hour identity of the payload BASIS,
/// i.e. after the volatile header; whole-payload identity measures 0-0.5%), priced
/// by lane QL at 34-49x under this exact codec on five sealed spools
/// (`runs/ql_d47_wire_delta/`). The list is here, in the codec, because BOTH
/// sides consult it: the producer to decide what to wrap, the decoder to refuse
/// an envelope naming a domain nobody enabled.
///
/// # Why 41 `projects_live` is NOT on this list
///
/// It was, and it came off for a reason no size argument outweighs (Codex review
/// round 2, finding 1). d41 is one of the four domains that copy their own H4CR
/// sequence INTO their payload: its PPA1 authority trailer carries
/// `record_sequence`, and `attach_log::stamp_record_bound_sequence` settles that
/// field AFTER the frame's drop passes have removed records and
/// `resequence_compact_records` has assigned the sequence the stream actually
/// ships. The stamper finds the trailer by walking back from the payload END.
///
/// Wrapping d41 puts an envelope where that payload used to be, and the trailer
/// is the last thing the domain appends — so on a keyframe the trailer's bytes
/// are the last bytes of the ENVELOPE'S BASIS, the stamper's magic/version/length
/// check matches them, and it rewrites eight bytes underneath `basis_sha256`.
/// Every d41 keyframe in a frame that dropped a record would then fail its own
/// digest. Not stamping is no better: it re-opens the defect the stamp exists to
/// close (69 of 369 d41 hours refused on `captures/ob_first_bmp`). And a delta
/// body cannot be stamped at all, because the trailer's bytes need not appear in
/// it.
///
/// The price of leaving it out is measured and small: on both spools that still
/// carry their bytes, d41's delta is worth 1.03x-1.06x — 105,483 B of a 4.79 GB
/// spool on `captures/oz_tip`. See `runs/ps_wire_delta/MEASUREMENT.md` §3b for
/// the capture where it might have been worth more, and why that is a projection
/// rather than a measurement.
///
/// 70 `sunk_convoy_history` and 100/101 are on the same stamped roster and are
/// excluded for the same reason; 100 already has its own delta.
///
/// # Why d81 needs no exception of its own
///
/// Its v3 payload bakes no record sequence (`compact_contract::rail_replay`'s
/// header is magic/version/header_len/row_len/reserved/status flags/wall
/// clock/observed counts/row count), so `stamp_record_bound_sequence` has only
/// the envelope-header arm to settle for it, exactly as for 43 and 53. Its wall
/// clock rides at payload offset 16..24, INSIDE the comparison basis rather
/// than in the declared volatile header — measured cost: NoChange never fires
/// and each hour's delta carries one small literal op for those bytes
/// (`runs/qk_d81_wire_delta/MEASUREMENT.md` §1); correctness is untouched,
/// because the basis is opaque bytes both sides.
/// # Why 47 qualifies where 41 does not
///
/// d47's grammar (`characters_leaders_cabinet_live::domain47_compact`) carries no
/// record sequence anywhere: its 64-byte header is `elapsed_wall_ms`, status and
/// schema flags, string-table sizes and per-family row counts, and every row is
/// fixed-width ids and string-table indexes. Nothing in the payload names the
/// record that carries it, so the stamping law has nothing to rewrite, and the
/// volatile-header rule applies to it exactly as to d43/d53: byte 0 is the same
/// recorder wall-clock `u64` those grammars stamp.
pub const DELTA_ENABLED_DOMAINS: &[u16] = &[43, 47, 53, 81];

pub const fn is_delta_enabled_domain(domain_id: u16) -> bool {
    matches!(domain_id, 43 | 47 | 53 | 81)
}

/// Hours a domain may go without a keyframe.
///
/// SAME CADENCE AS D100'S XSM2, and for the same reason: a consumer whose
/// baseline is stale — a record lost to framing damage, a fold that skipped an
/// already-finalized hour — refuses every later delta BY NAME until a record
/// arrives that needs no baseline. Without a cadence that is the rest of the
/// capture; with one it is at most this many hours.
///
/// Priced on `captures/oz_tip`: d53's keyframes cost 2.9 MB each, so a 64-hour
/// cadence adds ~7 of them (20 MB) to a domain whose delta stream is otherwise
/// 3.4 MB against a 1,329 MB payload. That is 0.4% of the spool for a bounded
/// worst case, and the alternative is an unbounded one.
pub const KEYFRAME_CADENCE_HOURS: u32 = 64;

/// One instruction of a delta body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PayloadSpanDeltaOp {
    /// Copy `len` bytes from the baseline basis at `source_offset`.
    Copy { source_offset: u32, len: u32 },
    /// Append these bytes literally.
    Literal(Vec<u8>),
}

impl PayloadSpanDeltaOp {
    pub fn encoded_len(&self) -> usize {
        match self {
            Self::Copy { .. } => COPY_OP_LEN,
            Self::Literal(bytes) => LITERAL_OP_HEADER_LEN.saturating_add(bytes.len()),
        }
    }

    /// Bytes this op contributes to the reconstruction.
    pub fn output_len(&self) -> usize {
        match self {
            Self::Copy { len, .. } => *len as usize,
            Self::Literal(bytes) => bytes.len(),
        }
    }
}

/// How one hour's payload is spelled on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PayloadSpanDeltaBody {
    /// The whole payload basis. The only body decodable without a baseline.
    Keyframe,
    /// The basis is bit-identical to the named baseline; no basis bytes ride.
    NoChange { base_basis_sha256: [u8; 32] },
    /// A copy/literal program against the named baseline basis.
    ///
    /// THE PROGRAM IS CARRIED AS ITS ENCODED BYTES, not as a `Vec` of decoded
    /// ops, and that is a memory-safety property rather than a convenience
    /// (Codex review round 4, finding 1). A decoded op is ~32 bytes and a
    /// literal op owns a heap `Vec`, while the same op costs as little as 6
    /// bytes on the wire — so materialising a program amplified a hostile record
    /// several-fold and, worse, turned a 24 MiB body of one-byte literals into
    /// four million separate heap allocations, all before the baseline digest
    /// could refuse it. Eight parallel fold workers walk the same stream
    /// independently.
    ///
    /// Held this way the decoded record is never larger than the record that
    /// arrived, every walk is a pass over bytes already in hand, and the only
    /// allocation the program can cause is the reconstruction itself — which
    /// [`ProvenProgram`] does not permit until every copy has been proven to lie
    /// inside the retained baseline.
    Delta {
        base_basis_sha256: [u8; 32],
        /// The op stream exactly as it rode: `op_count` ops, no trailing bytes.
        program: Vec<u8>,
        op_count: u32,
    },
}

impl PayloadSpanDeltaBody {
    pub fn kind_byte(&self) -> u8 {
        match self {
            Self::Keyframe => BODY_KIND_KEYFRAME,
            Self::NoChange { .. } => BODY_KIND_NO_CHANGE,
            Self::Delta { .. } => BODY_KIND_DELTA,
        }
    }

    pub fn base_basis_sha256(&self) -> Option<[u8; 32]> {
        match self {
            Self::Keyframe => None,
            Self::NoChange { base_basis_sha256 } => Some(*base_basis_sha256),
            Self::Delta {
                base_basis_sha256, ..
            } => Some(*base_basis_sha256),
        }
    }
}

/// The H4CR header fields a `PSD1` envelope is JOINED to.
///
/// # Why the envelope carries these at all
///
/// An envelope names a baseline by digest, so a delta cannot be applied to the
/// wrong BYTES. That is not the same as being applied to the wrong HOUR. A
/// record whose H4CR header says hour `H+1` while its envelope was built for `H`
/// reconstructs perfectly and attributes one hour's observed state to another —
/// which is the standing "hour misattribution is unacceptable" stop condition,
/// reached without a single byte of corruption inside the delta.
///
/// So the envelope restates the three identity fields of the record that carries
/// it, and [`PayloadSpanDeltaConsumer::apply`] refuses unless all three agree.
/// Exactly the join XSM2 makes with its own `source_sequence` and
/// `game_hour_raw` (`crate::xmap07_snow_mud`, and the
/// `XSM2 source sequence != H4CR sequence` refusal the host raises on it).
///
/// The producer does not PREDICT the sequence — it is handed the same value the
/// record header is written with, in the same call — which is the defect XSM2
/// had to add `stamp_source_sequence` to fix after 68 of 368 hours on
/// `captures/ob_first_bmp` were refused for an arithmetic guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PayloadSpanDeltaIdentity {
    pub domain_id: u16,
    pub source_sequence: u64,
    pub game_hour_raw: i32,
}

/// A decoded `PSD1` record.
///
/// Deliberately NOT the domain payload. A `NoChange` or `Delta` record does not
/// contain one, and inventing one here would mean guessing at a baseline this
/// type has no access to. [`apply_to_baseline`] is where a record plus a proven
/// baseline becomes the payload the domain encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadSpanDeltaRecord {
    /// The declared volatile header, carried verbatim (`elapsed_wall_ms`).
    pub volatile_header: [u8; VOLATILE_HEADER_LEN],
    /// The H4CR identity this envelope claims. Joined against the record that
    /// carried it before a single byte is reconstructed.
    pub identity: PayloadSpanDeltaIdentity,
    /// Length of the FULL payload this record represents, volatile header
    /// included.
    pub reconstructed_len: u32,
    /// Digest of the reconstructed payload's BASIS
    /// (`payload[VOLATILE_HEADER_LEN..]`).
    pub basis_sha256: [u8; 32],
    pub body: PayloadSpanDeltaBody,
    /// The whole basis, present only on a keyframe.
    pub keyframe_basis: Vec<u8>,
}

impl PayloadSpanDeltaRecord {
    pub fn domain_id(&self) -> u16 {
        self.identity.domain_id
    }

    pub fn base_basis_sha256(&self) -> Option<[u8; 32]> {
        self.body.base_basis_sha256()
    }

    pub fn body_kind(&self) -> u8 {
        self.body.kind_byte()
    }
}

/// One domain's retained baseline: the basis of the last record it emitted, and
/// that basis's digest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PayloadSpanDeltaBaseline {
    pub basis: Vec<u8>,
    pub basis_sha256: [u8; 32],
}

impl PayloadSpanDeltaBaseline {
    pub fn from_basis(basis: Vec<u8>) -> Self {
        let basis_sha256 = Sha256::digest(&basis).into();
        Self {
            basis,
            basis_sha256,
        }
    }
}

/// The digest of a payload's basis, without materialising the basis.
pub fn basis_sha256_of(payload: &[u8]) -> [u8; 32] {
    Sha256::digest(basis_of(payload)).into()
}

/// The comparison basis of a payload: everything after the declared volatile
/// header. A payload shorter than the header has an EMPTY basis and is carried
/// whole in the volatile field's place — see [`encode_payload`], which refuses
/// such a payload outright rather than inventing a split.
pub fn basis_of(payload: &[u8]) -> &[u8] {
    payload.get(VOLATILE_HEADER_LEN..).unwrap_or(&[])
}

// ---------------------------------------------------------------------------
// producer
// ---------------------------------------------------------------------------

/// A 64-bit polynomial rolling hash over a `MATCH_WINDOW`-byte window.
///
/// Rolling matters because the scan advances one byte at a time only through
/// UNMATCHED bytes: on a hit it jumps to the end of the match and re-primes.
/// The measured shape (median 3 changed spans in a 3.2 MB d53 payload) means
/// almost the whole payload is skipped rather than scanned.
struct RollingHash {
    value: u64,
    /// `BASE^(MATCH_WINDOW - 1)`, the multiplier of the byte leaving the window.
    drop_multiplier: u64,
}

const ROLL_BASE: u64 = 0x0000_0100_0000_01b3;

impl RollingHash {
    fn primed(window: &[u8]) -> Self {
        debug_assert_eq!(window.len(), MATCH_WINDOW);
        let mut drop_multiplier = 1u64;
        for _ in 1..MATCH_WINDOW {
            drop_multiplier = drop_multiplier.wrapping_mul(ROLL_BASE);
        }
        let mut value = 0u64;
        for byte in window {
            value = value.wrapping_mul(ROLL_BASE).wrapping_add(*byte as u64);
        }
        Self {
            value,
            drop_multiplier,
        }
    }

    fn roll(&mut self, leaving: u8, entering: u8) {
        self.value = self
            .value
            .wrapping_sub((leaving as u64).wrapping_mul(self.drop_multiplier))
            .wrapping_mul(ROLL_BASE)
            .wrapping_add(entering as u64);
    }
}

/// Open-addressed index of the baseline's window-aligned blocks.
///
/// A plain `HashMap` would allocate ~100k nodes per hour for a d53-sized
/// baseline; this is one power-of-two `Vec` pair, rebuilt in place, so the
/// producer's per-hour allocation does not scale with the payload.
struct BaselineIndex {
    mask: usize,
    hashes: Vec<u64>,
    offsets: Vec<u32>,
}

/// Sentinel for an empty index slot. A real entry always stores `offset + 1`.
const EMPTY_SLOT: u32 = 0;

impl BaselineIndex {
    fn build(baseline: &[u8]) -> Option<Self> {
        if baseline.len() < MATCH_WINDOW {
            return None;
        }
        let blocks = baseline.len() / MATCH_WINDOW;
        let mut capacity = 1usize;
        while capacity < blocks.saturating_mul(2).max(16) {
            capacity = capacity.checked_mul(2)?;
        }
        let mut index = Self {
            mask: capacity - 1,
            hashes: vec![0u64; capacity],
            offsets: vec![EMPTY_SLOT; capacity],
        };
        for block in 0..blocks {
            let at = block * MATCH_WINDOW;
            let hash = RollingHash::primed(&baseline[at..at + MATCH_WINDOW]).value;
            index.insert(hash, at as u32);
        }
        Some(index)
    }

    fn insert(&mut self, hash: u64, offset: u32) {
        let mut slot = (hash as usize) & self.mask;
        // FIRST WRITER WINS. A repeated block keeps its earliest offset, which
        // is the one most likely to sit before the shift the target is chasing.
        for _ in 0..=self.mask {
            if self.offsets[slot] == EMPTY_SLOT {
                self.hashes[slot] = hash;
                self.offsets[slot] = offset.saturating_add(1);
                return;
            }
            if self.hashes[slot] == hash {
                return;
            }
            slot = (slot + 1) & self.mask;
        }
    }

    fn lookup(&self, hash: u64) -> Option<usize> {
        let mut slot = (hash as usize) & self.mask;
        for _ in 0..=self.mask {
            let stored = self.offsets[slot];
            if stored == EMPTY_SLOT {
                return None;
            }
            if self.hashes[slot] == hash {
                return Some((stored - 1) as usize);
            }
            slot = (slot + 1) & self.mask;
        }
        None
    }
}

/// Build the copy/literal program that turns `baseline` into `target`.
///
/// Greedy, single pass, no backtracking: at each unmatched position the scan
/// asks the index whether the next `MATCH_WINDOW` bytes appear anywhere in the
/// baseline, verifies a hit byte-for-byte (the hash is a filter, never a proof),
/// extends it backwards into the pending literal and forwards as far as it goes,
/// and emits. Correctness does not depend on any of that: whatever program comes
/// out, [`encode_payload`] replays it against the baseline and refuses unless it
/// reproduces `target` exactly.
pub fn build_delta_ops(baseline: &[u8], target: &[u8]) -> Vec<PayloadSpanDeltaOp> {
    let mut ops: Vec<PayloadSpanDeltaOp> = Vec::new();
    let Some(index) = BaselineIndex::build(baseline) else {
        if !target.is_empty() {
            ops.push(PayloadSpanDeltaOp::Literal(target.to_vec()));
        }
        return ops;
    };
    if target.len() < MATCH_WINDOW {
        if !target.is_empty() {
            ops.push(PayloadSpanDeltaOp::Literal(target.to_vec()));
        }
        return ops;
    }

    let mut literal_start = 0usize;
    let mut at = 0usize;
    let mut rolling = RollingHash::primed(&target[0..MATCH_WINDOW]);
    let last_start = target.len() - MATCH_WINDOW;

    loop {
        let mut matched = false;
        if let Some(candidate) = index.lookup(rolling.value) {
            if candidate + MATCH_WINDOW <= baseline.len()
                && baseline[candidate..candidate + MATCH_WINDOW]
                    == target[at..at + MATCH_WINDOW]
            {
                // Extend backwards, but never before the last emitted op.
                let mut start = at;
                let mut source = candidate;
                while start > literal_start
                    && source > 0
                    && baseline[source - 1] == target[start - 1]
                {
                    start -= 1;
                    source -= 1;
                }
                // Extend forwards.
                let mut end = at + MATCH_WINDOW;
                let mut source_end = candidate + MATCH_WINDOW;
                while end < target.len()
                    && source_end < baseline.len()
                    && baseline[source_end] == target[end]
                {
                    end += 1;
                    source_end += 1;
                }
                // A COPY THAT COSTS MORE THAN THE BYTES IT SAVES IS NOT A COPY.
                if end - start > COPY_OP_LEN {
                    if start > literal_start {
                        ops.push(PayloadSpanDeltaOp::Literal(
                            target[literal_start..start].to_vec(),
                        ));
                    }
                    ops.push(PayloadSpanDeltaOp::Copy {
                        source_offset: source as u32,
                        len: (end - start) as u32,
                    });
                    literal_start = end;
                    at = end;
                    matched = true;
                }
            }
        }
        if matched {
            if at > last_start {
                break;
            }
            rolling = RollingHash::primed(&target[at..at + MATCH_WINDOW]);
            continue;
        }
        if at >= last_start {
            break;
        }
        rolling.roll(target[at], target[at + MATCH_WINDOW]);
        at += 1;
    }

    if literal_start < target.len() {
        ops.push(PayloadSpanDeltaOp::Literal(
            target[literal_start..].to_vec(),
        ));
    }
    ops
}

/// Bytes a delta body carrying these ops occupies.
pub fn delta_body_len(ops: &[PayloadSpanDeltaOp]) -> usize {
    ops.iter()
        .map(PayloadSpanDeltaOp::encoded_len)
        .fold(0usize, usize::saturating_add)
}

/// Total encoded length of an envelope carrying this body, refused unless it
/// fits an H4CR record's payload.
///
/// AN ENVELOPE IS BIGGER THAN THE PAYLOAD IT CARRIES (Codex review round 6,
/// finding 1). A keyframe is `HEADER_LEN + basis`, i.e.
/// `payload + ENVELOPE_OVERHEAD`, so a payload in the top
/// [`ENVELOPE_OVERHEAD`] bytes of `COMPACT_MAX_PAYLOAD_LEN` satisfies every
/// bound on the PAYLOAD and still produces a record the outer parser refuses as
/// `PayloadTooLarge` — turning a payload that was valid before this codec
/// existed into a framing failure, which costs the hour or the rest of the
/// stream rather than sealing partial data. The bound belongs on what is
/// actually written, which is what this returns.
///
/// Refusing is the right arm and not a lost hour: the collector's
/// `append_compact_span_delta_data_record` answers an encode error by appending
/// the payload RAW and unflagged, exactly as it did before there was a delta.
fn encoded_envelope_len(body_len: usize) -> Result<usize, String> {
    let encoded_len = HEADER_LEN
        .checked_add(body_len)
        .ok_or("PSD1 encoded length overflows")?;
    if encoded_len > crate::COMPACT_MAX_PAYLOAD_LEN as usize {
        return Err(format!(
            "PSD1 envelope of {encoded_len} byte(s) does not fit an H4CR record: the ceiling is {}",
            crate::COMPACT_MAX_PAYLOAD_LEN
        ));
    }
    Ok(encoded_len)
}

/// Encode a producer-built op list into the wire program.
fn encode_ops(ops: &[PayloadSpanDeltaOp]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(delta_body_len(ops));
    for op in ops {
        match op {
            PayloadSpanDeltaOp::Copy { source_offset, len } => {
                out.push(OP_COPY);
                out.extend_from_slice(&source_offset.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
            }
            PayloadSpanDeltaOp::Literal(bytes) => {
                out.push(OP_LITERAL);
                out.extend_from_slice(
                    &u32::try_from(bytes.len())
                        .map_err(|_| "PSD1 literal too large")?
                        .to_le_bytes(),
                );
                out.extend_from_slice(bytes);
            }
        }
    }
    Ok(out)
}

/// One op read out of a wire program, borrowing its literal bytes.
enum ProgramOp<'a> {
    Copy { source_offset: usize, len: usize },
    Literal(&'a [u8]),
}

/// Walk a wire program, yielding each op in turn.
///
/// Structural only: it proves the stream is well formed (known tags, no
/// zero-length op, no truncation, no trailing bytes, exactly `op_count` ops) and
/// that no single op claims more output than the declared basis. What it does
/// NOT check is whether a copy lies inside the baseline — that needs a baseline,
/// and it is [`ProvenProgram::prove`]'s job.
fn walk_program<'a>(
    program: &'a [u8],
    op_count: u32,
    basis_len: usize,
    mut visit: impl FnMut(usize, ProgramOp<'a>) -> Result<(), String>,
) -> Result<usize, String> {
    let mut cursor = 0usize;
    let mut produced = 0usize;
    for ordinal in 0..op_count as usize {
        let tag = *program
            .get(cursor)
            .ok_or_else(|| format!("PSD1 body truncated at op {ordinal}"))?;
        cursor += 1;
        let op = match tag {
            OP_COPY => {
                let raw: [u8; 8] = program
                    .get(cursor..cursor + 8)
                    .ok_or_else(|| format!("PSD1 body truncated inside copy op {ordinal}"))?
                    .try_into()
                    .expect("8 bytes");
                cursor += 8;
                let source_offset = u32::from_le_bytes(raw[0..4].try_into().unwrap()) as usize;
                let len = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
                if len == 0 {
                    return Err(format!("PSD1 copy op {ordinal} has zero length"));
                }
                produced = produced.saturating_add(len);
                ProgramOp::Copy { source_offset, len }
            }
            OP_LITERAL => {
                let raw: [u8; 4] = program
                    .get(cursor..cursor + 4)
                    .ok_or_else(|| format!("PSD1 body truncated inside literal op {ordinal}"))?
                    .try_into()
                    .expect("4 bytes");
                cursor += 4;
                let len = u32::from_le_bytes(raw) as usize;
                if len == 0 {
                    return Err(format!("PSD1 literal op {ordinal} has zero length"));
                }
                if len > basis_len {
                    return Err(format!(
                        "PSD1 literal op {ordinal} claims {len} byte(s), more than the declared \
                         {basis_len}-byte basis"
                    ));
                }
                let bytes = program
                    .get(cursor..cursor + len)
                    .ok_or_else(|| format!("PSD1 body truncated inside literal op {ordinal}"))?;
                cursor += len;
                produced = produced.saturating_add(len);
                ProgramOp::Literal(bytes)
            }
            other => return Err(format!("PSD1 op {ordinal} kind {other} is not assigned")),
        };
        if produced > basis_len {
            return Err(format!(
                "PSD1 program produces more than the declared {basis_len}-byte basis"
            ));
        }
        visit(ordinal, op)?;
    }
    if cursor != program.len() {
        return Err(format!(
            "PSD1 delta body has {} trailing byte(s)",
            program.len() - cursor
        ));
    }
    Ok(produced)
}

/// A program whose every op has been proven to lie inside a specific baseline,
/// carrying the exact number of bytes it will produce.
///
/// THE ONLY WAY TO GET AN OUTPUT LENGTH IS TO PROVE THE PROGRAM FIRST, and that
/// is the whole reason this type exists rather than a bare `usize`. `replay_ops`
/// reserves its buffer from `output_len`, and `output_len` cannot be read
/// without a `ProvenProgram`, which cannot be built except by
/// [`Self::prove`] — so moving the allocation ahead of the validation does not
/// produce a subtly weaker decoder, it produces a build error.
///
/// The hole it closes (Codex review round 1, finding 2): `replay_ops` used to
/// `Vec::with_capacity(reconstructed_basis_len)` and only then walk the ops. A
/// 112-byte envelope may declare the full [`MAX_RECONSTRUCTED_LEN`] and carry one
/// COPY claiming to produce it, against a small retained baseline that the
/// envelope correctly names. Every structural check in `decode_payload` passes —
/// the op total equals the declaration — and the reservation of ~256 MiB happens
/// BEFORE anyone notices the copy reaches outside the baseline. Six independent
/// walkers each doing that is a memory-exhaustion abort produced by a malformed
/// record, and a malformed record must produce an `Err`.
struct ProvenProgram {
    output_len: usize,
}

impl ProvenProgram {
    fn prove(
        baseline: &[u8],
        program: &[u8],
        op_count: u32,
        declared_basis_len: usize,
    ) -> Result<Self, String> {
        let output_len = walk_program(program, op_count, declared_basis_len, |ordinal, op| {
            match op {
                ProgramOp::Copy { source_offset, len } => {
                    let end = source_offset
                        .checked_add(len)
                        .ok_or_else(|| format!("PSD1 op {ordinal} copy extent overflows"))?;
                    if end > baseline.len() {
                        return Err(format!(
                            "PSD1 op {ordinal} copies {source_offset}..{end} from a {}-byte \
                             baseline",
                            baseline.len()
                        ));
                    }
                    Ok(())
                }
                // A literal's bytes are already held: they arrived inside the
                // record, so the body length bounds them and there is nothing
                // left to prove about their extent.
                ProgramOp::Literal(_) => Ok(()),
            }
        })?;
        if output_len != declared_basis_len {
            return Err(format!(
                "PSD1 program produces {output_len} byte(s), not the declared {declared_basis_len}"
            ));
        }
        Ok(Self { output_len })
    }
}

/// Replay a program against a baseline. Used by BOTH sides: the producer to
/// prove the program it just built, the consumer to apply the one it received.
fn replay_program(
    baseline: &[u8],
    program: &[u8],
    op_count: u32,
    reconstructed_basis_len: usize,
) -> Result<Vec<u8>, String> {
    let proven = ProvenProgram::prove(baseline, program, op_count, reconstructed_basis_len)?;
    let mut out = Vec::with_capacity(proven.output_len);
    walk_program(program, op_count, reconstructed_basis_len, |_, op| {
        match op {
            ProgramOp::Copy { source_offset, len } => {
                out.extend_from_slice(&baseline[source_offset..source_offset + len]);
            }
            ProgramOp::Literal(bytes) => out.extend_from_slice(bytes),
        }
        Ok(())
    })?;
    debug_assert_eq!(out.len(), reconstructed_basis_len);
    Ok(out)
}

/// Choose the smallest SOUND body for this payload.
///
/// `baseline` is the producer's retained baseline for this domain, when it has
/// one. The keyframe arm is taken when there is no baseline and whenever the
/// delta would not actually be smaller than carrying the basis whole — the
/// latter is a real case (d41's whole payload is ~600 bytes on
/// `captures/oz_tip`, and a delta's ops can cost more than that), and it is why
/// this returns a body rather than assuming a delta always wins.
///
/// THE PROGRAM IS PROVEN BEFORE IT IS CHOSEN. `build_delta_ops` is a heuristic;
/// this replays what it produced against the same baseline and falls back to a
/// keyframe if the replay does not reproduce the basis exactly. A producer bug
/// therefore costs bytes, never correctness.
pub fn choose_wire_body(
    payload: &[u8],
    baseline: Option<&PayloadSpanDeltaBaseline>,
) -> PayloadSpanDeltaBody {
    let Some(baseline) = baseline else {
        return PayloadSpanDeltaBody::Keyframe;
    };
    let basis = basis_of(payload);
    if baseline.basis == basis {
        return PayloadSpanDeltaBody::NoChange {
            base_basis_sha256: baseline.basis_sha256,
        };
    }
    let ops = build_delta_ops(&baseline.basis, basis);
    if ops.is_empty() || delta_body_len(&ops) >= basis.len() {
        return PayloadSpanDeltaBody::Keyframe;
    }
    let Ok(program) = encode_ops(&ops) else {
        return PayloadSpanDeltaBody::Keyframe;
    };
    let Ok(op_count) = u32::try_from(ops.len()) else {
        return PayloadSpanDeltaBody::Keyframe;
    };
    match replay_program(&baseline.basis, &program, op_count, basis.len()) {
        Ok(replayed) if replayed == basis => PayloadSpanDeltaBody::Delta {
            base_basis_sha256: baseline.basis_sha256,
            program,
            op_count,
        },
        _ => PayloadSpanDeltaBody::Keyframe,
    }
}

// ---------------------------------------------------------------------------
// wire
// ---------------------------------------------------------------------------

fn take<const N: usize>(bytes: &[u8], at: &mut usize) -> Result<[u8; N], String> {
    let end = at.checked_add(N).ok_or("PSD1 cursor overflow")?;
    let slice = bytes.get(*at..end).ok_or("PSD1 truncated")?;
    *at = end;
    Ok(slice.try_into().expect("fixed-size slice"))
}

/// Wrap one domain payload as a `PSD1` record.
///
/// The envelope carries NO domain semantics of its own — no status word, no row
/// count, nothing a validator could read instead of reading the payload. That is
/// deliberate: every consumer unwraps before it looks at the record, so a
/// mirrored field would be a second copy of something the first copy already
/// says, and the two could disagree.
pub fn encode_payload(
    identity: PayloadSpanDeltaIdentity,
    payload: &[u8],
    body: &PayloadSpanDeltaBody,
) -> Result<Vec<u8>, String> {
    let basis_sha256 = Sha256::digest(basis_of(payload)).into();
    encode_payload_with_basis_digest(identity, payload, basis_sha256, body)
}

/// [`encode_payload`] with the basis digest supplied.
///
/// The digest must be over exactly `payload[VOLATILE_HEADER_LEN..]`, and a debug
/// build asserts it. THIS EXISTS FOR THE HOT PATH AND NOTHING ELSE: hashing
/// d53's 2.88 MB basis costs about 1.5 ms on the collector thread, and the naive
/// spelling paid it twice a hour — once here and once building the retained
/// baseline — for the same bytes and the same answer.
fn encode_payload_with_basis_digest(
    identity: PayloadSpanDeltaIdentity,
    payload: &[u8],
    basis_sha256: [u8; 32],
    body: &PayloadSpanDeltaBody,
) -> Result<Vec<u8>, String> {
    let domain_id = identity.domain_id;
    if !is_delta_enabled_domain(domain_id) {
        return Err(format!(
            "PSD1 refuses domain {domain_id}, which is not delta-enabled"
        ));
    }
    if payload.len() < VOLATILE_HEADER_LEN {
        return Err(format!(
            "PSD1 payload of {} byte(s) is shorter than the {VOLATILE_HEADER_LEN}-byte declared \
             volatile header; this domain cannot carry a span delta",
            payload.len()
        ));
    }
    if payload.len() > MAX_RECONSTRUCTED_LEN {
        return Err(format!(
            "PSD1 payload of {} byte(s) is above the {MAX_RECONSTRUCTED_LEN}-byte ceiling",
            payload.len()
        ));
    }
    let basis = basis_of(payload);
    debug_assert_eq!(
        basis_sha256,
        <[u8; 32]>::from(Sha256::digest(basis)),
        "encode_payload_with_basis_digest was handed a digest of other bytes"
    );
    let (body_len, op_count) = match body {
        PayloadSpanDeltaBody::Keyframe => (basis.len(), 0u32),
        PayloadSpanDeltaBody::NoChange { base_basis_sha256 } => {
            if *base_basis_sha256 != basis_sha256 {
                return Err(
                    "PSD1 no-change body names a baseline that is not this hour's basis".into(),
                );
            }
            (0usize, 0u32)
        }
        PayloadSpanDeltaBody::Delta {
            base_basis_sha256,
            program,
            op_count,
        } => {
            if *base_basis_sha256 == basis_sha256 {
                return Err("PSD1 delta body names its own basis as the baseline".into());
            }
            if *op_count == 0 {
                return Err("PSD1 delta body carries no ops; it is a no-change".into());
            }
            if *op_count as usize > MAX_OPS {
                return Err(format!("PSD1 delta body carries more than {MAX_OPS} ops"));
            }
            let produced = walk_program(program, *op_count, basis.len(), |_, _| Ok(()))?;
            if produced != basis.len() {
                return Err(format!(
                    "PSD1 delta program produces {produced} byte(s), not this hour's {}",
                    basis.len()
                ));
            }
            (program.len(), *op_count)
        }
    };
    let encoded_len = encoded_envelope_len(body_len)?;
    let body_len_u32 = u32::try_from(body_len).map_err(|_| "PSD1 body too large")?;
    let reconstructed_len =
        u32::try_from(payload.len()).map_err(|_| "PSD1 payload length overflows")?;

    // THE CAPACITY COMES FROM THE CHECKED VALUE, so the ceiling above cannot be
    // deleted without breaking the build.
    let mut out = Vec::with_capacity(encoded_len);
    out.extend_from_slice(&payload[..VOLATILE_HEADER_LEN]);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.push(body.kind_byte());
    out.push(VOLATILE_HEADER_LEN as u8);
    out.extend_from_slice(&reconstructed_len.to_le_bytes());
    out.extend_from_slice(&op_count.to_le_bytes());
    out.extend_from_slice(&body_len_u32.to_le_bytes());
    out.extend_from_slice(&identity.source_sequence.to_le_bytes());
    out.extend_from_slice(&identity.game_hour_raw.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&basis_sha256);
    out.extend_from_slice(&body.base_basis_sha256().unwrap_or([0; 32]));
    debug_assert_eq!(out.len(), HEADER_LEN);

    match body {
        PayloadSpanDeltaBody::Keyframe => out.extend_from_slice(basis),
        PayloadSpanDeltaBody::NoChange { .. } => {}
        PayloadSpanDeltaBody::Delta { program, .. } => out.extend_from_slice(program),
    }
    Ok(out)
}

/// Byte offset of `source_sequence` inside the `PSD1` header.
///
/// Derived from [`encode_payload_with_basis_digest`]'s own write order: volatile
/// header (8) + `MAGIC` (4) + `VERSION` (2) + `HEADER_LEN` (2) + `domain_id` (2)
/// + `body_kind` (1) + `volatile_len` (1) + `reconstructed_len` (4) + `op_count`
/// (4) + `body_len` (4) = 32.
pub const SOURCE_SEQUENCE_OFFSET: usize = 32;

/// Write the record's FINAL H4CR sequence into an already-encoded envelope.
///
/// THIS EXISTS BECAUSE THE BUILD'S NUMBERING IS NOT THE STREAM'S (Codex review
/// round 2, finding 1). The producer wraps during the frame build, but the
/// frame's later passes REMOVE records — a rejected domain's records become
/// `DOMAIN_GAP`s, an unresolved hour's are dropped — and
/// `attach_log::resequence_compact_records` then assigns every survivor the
/// sequence it actually ships, by position. A sequence baked at build time
/// therefore overshoots by exactly the number of records dropped ahead of it,
/// and the consumer's identity join would refuse every one of those hours for
/// arithmetic rather than for anything the game did. The same defect was
/// measured on `captures/ob_first_bmp` for the four domains that already bake a
/// sequence: 69 of 369 d41 hours, 68 of 368 d100 hours.
///
/// Settling it after encoding is sound because no digest covers it:
/// `basis_sha256` digests the reconstructed payload's BASIS, which is the domain
/// payload after its volatile header, and this field lives in the envelope's own
/// header outside that span. Exactly the argument
/// `xmap07_snow_mud::stamp_source_sequence` makes for XSM2.
///
/// A payload that is not an envelope is left untouched and reports it, so the
/// caller cannot mistake "not an envelope" for "stamped".
pub fn stamp_source_sequence(payload: &mut [u8], source_sequence: u64) -> Result<(), String> {
    if payload.len() < HEADER_LEN {
        return Err("PSD1 payload too short to stamp a source sequence".into());
    }
    if payload.get(8..12) != Some(&MAGIC[..]) {
        return Err("PSD1 payload magic mismatch while stamping a source sequence".into());
    }
    payload[SOURCE_SEQUENCE_OFFSET..SOURCE_SEQUENCE_OFFSET + 8]
        .copy_from_slice(&source_sequence.to_le_bytes());
    Ok(())
}

pub fn decode_payload(payload: &[u8]) -> Result<PayloadSpanDeltaRecord, String> {
    if payload.len() < HEADER_LEN {
        return Err("PSD1 header truncated".into());
    }
    let mut at = 0usize;
    let volatile_header: [u8; VOLATILE_HEADER_LEN] = take(payload, &mut at)?;
    if take::<4>(payload, &mut at)? != MAGIC
        || u16::from_le_bytes(take(payload, &mut at)?) != VERSION
        || u16::from_le_bytes(take(payload, &mut at)?) as usize != HEADER_LEN
    {
        return Err("PSD1 magic/version/header mismatch".into());
    }
    let domain_id = u16::from_le_bytes(take(payload, &mut at)?);
    let body_kind = take::<1>(payload, &mut at)?[0];
    let volatile_len = take::<1>(payload, &mut at)?[0];
    if volatile_len as usize != VOLATILE_HEADER_LEN {
        return Err(format!(
            "PSD1 declares a {volatile_len}-byte volatile header, not {VOLATILE_HEADER_LEN}"
        ));
    }
    let reconstructed_len = u32::from_le_bytes(take(payload, &mut at)?);
    let op_count = u32::from_le_bytes(take(payload, &mut at)?);
    let body_len = u32::from_le_bytes(take(payload, &mut at)?) as usize;
    let source_sequence = u64::from_le_bytes(take(payload, &mut at)?);
    let game_hour_raw = i32::from_le_bytes(take(payload, &mut at)?);
    if take::<4>(payload, &mut at)? != [0u8; 4] {
        return Err("PSD1 reserved field is nonzero".into());
    }
    let basis_sha256: [u8; 32] = take(payload, &mut at)?;
    let base_basis_sha256: [u8; 32] = take(payload, &mut at)?;
    debug_assert_eq!(at, HEADER_LEN);

    let expected = HEADER_LEN
        .checked_add(body_len)
        .ok_or("PSD1 length overflow")?;
    if payload.len() != expected {
        return Err(format!("PSD1 len {} != {expected}", payload.len()));
    }
    if !is_delta_enabled_domain(domain_id) {
        return Err(format!(
            "PSD1 names domain {domain_id}, which is not delta-enabled"
        ));
    }

    // THE DECLARED LENGTH IS BOUNDED BEFORE ANYTHING IS ALLOCATED FROM IT. Every
    // allocation below — the keyframe copy, the op vector, the replay buffer —
    // is sized from `reconstructed_len` or from `op_count`, and an unchecked u32
    // in either is an out-of-memory abort rather than an `Err`. A malformed
    // record must produce an `Err`, never a termination.
    let reconstructed_len = reconstructed_len as usize;
    if reconstructed_len < VOLATILE_HEADER_LEN || reconstructed_len > MAX_RECONSTRUCTED_LEN {
        return Err(format!(
            "PSD1 reconstructed length {reconstructed_len} outside \
             {VOLATILE_HEADER_LEN}..={MAX_RECONSTRUCTED_LEN}"
        ));
    }
    let basis_len = reconstructed_len - VOLATILE_HEADER_LEN;
    let body_bytes = &payload[HEADER_LEN..];

    let mut keyframe_basis = Vec::new();
    let body = match body_kind {
        BODY_KIND_KEYFRAME => {
            if base_basis_sha256 != [0; 32] {
                return Err("PSD1 keyframe must not name a baseline".into());
            }
            if op_count != 0 {
                return Err("PSD1 keyframe must carry no ops".into());
            }
            if body_bytes.len() != basis_len {
                return Err(format!(
                    "PSD1 keyframe basis is {} byte(s), not the declared {basis_len}",
                    body_bytes.len()
                ));
            }
            let digest: [u8; 32] = Sha256::digest(body_bytes).into();
            if digest != basis_sha256 {
                return Err("PSD1 keyframe basis does not hash to basis_sha256".into());
            }
            keyframe_basis = body_bytes.to_vec();
            PayloadSpanDeltaBody::Keyframe
        }
        BODY_KIND_NO_CHANGE => {
            if op_count != 0 {
                return Err("PSD1 no-change body must carry no ops".into());
            }
            if !body_bytes.is_empty() {
                return Err("PSD1 no-change body must carry no bytes".into());
            }
            if base_basis_sha256 != basis_sha256 {
                return Err(
                    "PSD1 no-change body names a baseline that is not this hour's basis".into(),
                );
            }
            PayloadSpanDeltaBody::NoChange { base_basis_sha256 }
        }
        BODY_KIND_DELTA => {
            if base_basis_sha256 == basis_sha256 {
                return Err("PSD1 delta body names its own basis as the baseline".into());
            }
            if op_count == 0 {
                return Err("PSD1 delta body must carry at least one op".into());
            }
            let changed_op_count = op_count;
            let op_count = op_count as usize;
            if op_count > MAX_OPS {
                return Err(format!("PSD1 delta body claims more than {MAX_OPS} ops"));
            }
            // Every op costs at least this much body, so a claimed count the body
            // cannot hold is rejected before anything walks it.
            let minimum_body = op_count
                .checked_mul(LITERAL_OP_HEADER_LEN)
                .ok_or("PSD1 op table overflows")?;
            if minimum_body > body_bytes.len() {
                return Err(format!(
                    "PSD1 delta claims {op_count} op(s) but its body holds {} byte(s)",
                    body_bytes.len()
                ));
            }
            // Every op produces at least one byte, so more ops than basis bytes
            // is an impossible program (Codex review round 3, finding 1).
            if op_count > basis_len {
                return Err(format!(
                    "PSD1 delta claims {op_count} op(s) for a {basis_len}-byte basis; every op \
                     produces at least one byte"
                ));
            }
            // THE PROGRAM IS NOT MATERIALISED (Codex review round 4, finding
            // 1). Round 3 closed `op_count > basis_len`; the equal case was still
            // open, and it is worse than the one that was closed: `basis_len =
            // op_count = MAX_OPS` with a ~24 MiB body of one-byte literals passes
            // every count bound, and building a `Vec<PayloadSpanDeltaOp>` from it
            // costs ~134 MiB of vector PLUS four million individual heap
            // allocations for the one-byte literals -- all before the baseline
            // digest gets to refuse the record, and in each of up to eight
            // parallel fold workers.
            //
            // Keeping the program as its ENCODED BYTES removes the class rather
            // than the instance: the decoded record is never bigger than the
            // record that arrived, and every later walk is a pass over bytes
            // already in hand. `walk_program` below proves the stream is well
            // formed without allocating anything at all.
            let program = body_bytes.to_vec();
            let produced = walk_program(&program, changed_op_count, basis_len, |_, _| Ok(()))?;
            if produced != basis_len {
                return Err(format!(
                    "PSD1 program produces {produced} byte(s), not the declared {basis_len}"
                ));
            }
            PayloadSpanDeltaBody::Delta {
                base_basis_sha256,
                program,
                op_count: changed_op_count,
            }
        }
        other => return Err(format!("PSD1 body kind {other} is not assigned")),
    };

    Ok(PayloadSpanDeltaRecord {
        volatile_header,
        identity: PayloadSpanDeltaIdentity {
            domain_id,
            source_sequence,
            game_hour_raw,
        },
        reconstructed_len: reconstructed_len as u32,
        basis_sha256,
        body,
        keyframe_basis,
    })
}

/// Turn a decoded record plus the consumer's retained baseline back into the
/// EXACT payload the domain encoded.
///
/// THE BASELINE IS PROVEN, NOT ASSUMED, and the reconstruction is re-hashed —
/// see the module docs. The returned bytes are byte-identical to the producer's
/// `payload`: the volatile header comes back verbatim from the record, the basis
/// from the proven program.
pub fn apply_to_baseline(
    record: &PayloadSpanDeltaRecord,
    baseline: Option<&PayloadSpanDeltaBaseline>,
) -> Result<Vec<u8>, String> {
    let reconstructed_len = record.reconstructed_len as usize;
    let basis_len = reconstructed_len.saturating_sub(VOLATILE_HEADER_LEN);

    // NOTHING IS COPIED UNTIL THE LENGTHS AND THE DIGEST AGREE (Codex review
    // round 7, finding 2). The first spelling cloned the basis, built a second
    // payload-sized buffer from it, and only then compared the result against
    // the DECLARED length — so a 112-byte header-only `NoChange` naming a
    // baseline the consumer really holds, but declaring some other length, cost
    // roughly two copies of a 3.25 MB d53 basis in every walker before it was
    // refused. That is the same malformed-record amplification class as the
    // reconstruction buffer and the op vector, in a third place.
    //
    // Borrowing the source instead of cloning it is what closes it: the only
    // allocation left is the payload this function must RETURN, and by then the
    // record has been proven to describe exactly it. A delta still builds its
    // basis, but `replay_program` will not allocate until `ProvenProgram` has
    // proven every copy lies inside the retained baseline.
    let source: std::borrow::Cow<'_, [u8]> = match &record.body {
        PayloadSpanDeltaBody::Keyframe => {
            std::borrow::Cow::Borrowed(record.keyframe_basis.as_slice())
        }
        PayloadSpanDeltaBody::NoChange { base_basis_sha256 } => {
            let baseline = named_baseline(baseline, base_basis_sha256)?;
            std::borrow::Cow::Borrowed(baseline.basis.as_slice())
        }
        PayloadSpanDeltaBody::Delta {
            base_basis_sha256,
            program,
            op_count,
        } => {
            let baseline = named_baseline(baseline, base_basis_sha256)?;
            std::borrow::Cow::Owned(replay_program(
                &baseline.basis,
                program,
                *op_count,
                basis_len,
            )?)
        }
    };
    let basis: &[u8] = source.as_ref();
    if basis.len() != basis_len {
        return Err(format!(
            "PSD1 declares a {reconstructed_len}-byte payload but the basis it names is {} byte(s)",
            basis.len()
        ));
    }
    let digest: [u8; 32] = Sha256::digest(basis).into();
    if digest != record.basis_sha256 {
        return Err("PSD1 reconstructed basis does not hash to basis_sha256".into());
    }
    let mut payload = Vec::with_capacity(reconstructed_len);
    payload.extend_from_slice(&record.volatile_header);
    payload.extend_from_slice(basis);
    debug_assert_eq!(payload.len(), reconstructed_len);
    Ok(payload)
}

fn named_baseline<'a>(
    baseline: Option<&'a PayloadSpanDeltaBaseline>,
    named: &[u8; 32],
) -> Result<&'a PayloadSpanDeltaBaseline, String> {
    let baseline = baseline.ok_or_else(|| {
        format!(
            "PSD1 has no retained baseline for the one this record names, {}",
            hex32(named)
        )
    })?;
    if baseline.basis_sha256 != *named {
        return Err(format!(
            "PSD1 retained baseline {} is not the {} this record names",
            hex32(&baseline.basis_sha256),
            hex32(named)
        ));
    }
    Ok(baseline)
}

fn hex32(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The producer's retained baselines, one per delta-enabled domain.
///
/// Deliberately a small explicit store rather than a map keyed by anything
/// clever: the enabled set is three domains, and a store that silently grew a
/// baseline for a domain nobody enabled would be memory nobody accounted for.
#[derive(Clone, Debug, Default)]
pub struct PayloadSpanDeltaBaselines {
    entries: Vec<(u16, PayloadSpanDeltaBaseline)>,
}

impl PayloadSpanDeltaBaselines {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, domain_id: u16) -> Option<&PayloadSpanDeltaBaseline> {
        self.entries
            .iter()
            .find(|(id, _)| *id == domain_id)
            .map(|(_, baseline)| baseline)
    }

    pub fn set(&mut self, domain_id: u16, baseline: PayloadSpanDeltaBaseline) {
        match self.entries.iter_mut().find(|(id, _)| *id == domain_id) {
            Some(slot) => slot.1 = baseline,
            None => self.entries.push((domain_id, baseline)),
        }
    }

    /// Forget one domain's baseline, so its next record is a keyframe.
    ///
    /// This is the FAIL-CLOSED direction and the only one a caller ever needs:
    /// a producer that drops a baseline costs bytes, while a producer that keeps
    /// one the consumer does not have costs the hour a named refusal. Nothing
    /// calls this to "resynchronise" — the digest does that.
    pub fn clear(&mut self, domain_id: u16) {
        self.entries.retain(|(id, _)| *id != domain_id);
    }

    pub fn clear_all(&mut self) {
        self.entries.clear();
    }

    /// Take every retained baseline, leaving the store empty.
    pub fn drain(&mut self) -> impl Iterator<Item = (u16, PayloadSpanDeltaBaseline)> + '_ {
        self.entries.drain(..)
    }

    pub fn retained_domains(&self) -> impl Iterator<Item = u16> + '_ {
        self.entries.iter().map(|(id, _)| *id)
    }

    /// Total retained bytes, for the memory accounting the capture publishes.
    pub fn retained_bytes(&self) -> usize {
        self.entries
            .iter()
            .map(|(_, baseline)| baseline.basis.len())
            .fold(0usize, usize::saturating_add)
    }
}

/// The producer's transactional baseline store.
///
/// # Why this is a transaction and not a plain cache
///
/// Exactly d100's problem, and its answer. The body choice runs at EMISSION,
/// before bracket rejection is known, so a frame can encode a delta against
/// basis `B` and then have its whole record
/// `replace_rejected_domain_records_with_gaps`-ed into a `DOMAIN_GAP`. The fold
/// never saw that record, so a baseline advanced there would sit one hour AHEAD
/// of the fold and every later delta would name a baseline the fold cannot
/// produce — each of those hours refusing at [`apply_to_baseline`].
///
/// So the advance is STAGED at emission and committed at the frame seal, and the
/// collector rolls it back when the domain's own bracket was rejected.
/// `committed` is therefore, by construction, the basis the fold holds.
///
/// A domain may append MORE THAN ONE record in a frame — d53 does, when
/// `insert_a30_payloads_before_frame_end` has companion rows queued — so a wrap
/// diffs against the STAGED basis when one exists, which is what the fold will
/// be holding by the time it reaches that record. The keyframe-cadence undo is
/// captured once per frame, not once per wrap, or a second wrap would overwrite
/// it with the first wrap's already-advanced number (d101's lesson).
#[derive(Clone, Debug, Default)]
pub struct PayloadSpanDeltaProducer {
    committed: PayloadSpanDeltaBaselines,
    staged: PayloadSpanDeltaBaselines,
    hours_since_keyframe: Vec<(u16, u32)>,
    staged_hours_since_keyframe: Vec<(u16, u32)>,
}

impl PayloadSpanDeltaProducer {
    pub fn new() -> Self {
        Self::default()
    }

    fn hours_since_keyframe(&self, domain_id: u16) -> u32 {
        self.hours_since_keyframe
            .iter()
            .find(|(id, _)| *id == domain_id)
            .map(|(_, hours)| *hours)
            .unwrap_or(0)
    }

    fn set_hours_since_keyframe(&mut self, domain_id: u16, hours: u32) {
        match self
            .hours_since_keyframe
            .iter_mut()
            .find(|(id, _)| *id == domain_id)
        {
            Some(slot) => slot.1 = hours,
            None => self.hours_since_keyframe.push((domain_id, hours)),
        }
    }

    /// The basis the FOLD will hold when it reaches the next record this domain
    /// emits: the staged one if this frame has already emitted for it, else the
    /// committed one.
    fn current(&self, domain_id: u16) -> Option<&PayloadSpanDeltaBaseline> {
        self.staged
            .get(domain_id)
            .or_else(|| self.committed.get(domain_id))
    }

    /// True when this hour must re-send the whole payload regardless of what
    /// changed.
    pub fn keyframe_due(&self, domain_id: u16) -> bool {
        self.current(domain_id).is_none()
            || self.hours_since_keyframe(domain_id) >= KEYFRAME_CADENCE_HOURS
    }

    /// Wrap one payload, staging the baseline advance.
    ///
    /// Refuses a domain that is not delta-enabled and a payload too short to
    /// carry the declared volatile header. The CALLER decides what a refusal
    /// means; in the collector it means the record rides unwrapped and
    /// unflagged, which is the pre-delta behaviour and costs the hour nothing.
    pub fn wrap(
        &mut self,
        identity: PayloadSpanDeltaIdentity,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        let domain_id = identity.domain_id;
        let body = if self.keyframe_due(domain_id) {
            PayloadSpanDeltaBody::Keyframe
        } else {
            choose_wire_body(payload, self.current(domain_id))
        };
        // A NO-CHANGE HOUR COSTS ONE COMPARISON AND NOTHING ELSE.
        //
        // The body asserts the basis is bit-identical to the retained baseline,
        // so that baseline IS the correct next baseline and its digest IS this
        // hour's digest. Copying 2.88 MB into a new baseline and hashing it would
        // reproduce, byte for byte, what the producer is already holding — and
        // because `current` already falls back to `committed`, leaving the staged
        // slot alone is not merely an optimisation, it is the correct state: this
        // frame advanced nothing, so a rollback has nothing to undo and a commit
        // has nothing to install.
        //
        // 163 of d53's 461 hours on `captures/oz_tip` take this path.
        let next_baseline = match &body {
            PayloadSpanDeltaBody::NoChange { .. } => None,
            _ => {
                let basis = basis_of(payload).to_vec();
                let basis_sha256: [u8; 32] = Sha256::digest(&basis).into();
                Some(PayloadSpanDeltaBaseline {
                    basis,
                    basis_sha256,
                })
            }
        };
        let basis_sha256 = match (&body, &next_baseline) {
            (PayloadSpanDeltaBody::NoChange { base_basis_sha256 }, _) => *base_basis_sha256,
            (_, Some(baseline)) => baseline.basis_sha256,
            (_, None) => unreachable!("only a no-change body skips the new baseline"),
        };
        let wrapped = encode_payload_with_basis_digest(identity, payload, basis_sha256, &body)?;
        if !self
            .staged_hours_since_keyframe
            .iter()
            .any(|(id, _)| *id == domain_id)
        {
            self.staged_hours_since_keyframe
                .push((domain_id, self.hours_since_keyframe(domain_id)));
        }
        let advanced = if matches!(body, PayloadSpanDeltaBody::Keyframe) {
            0
        } else {
            self.hours_since_keyframe(domain_id).saturating_add(1)
        };
        self.set_hours_since_keyframe(domain_id, advanced);
        if let Some(baseline) = next_baseline {
            self.staged.set(domain_id, baseline);
        }
        Ok(wrapped)
    }

    /// The frame survived: what was staged is what the fold holds.
    pub fn commit(&mut self) {
        // MOVED, NOT CLONED. Cloning each staged basis here meant that for the
        // duration of the commit the producer held THREE copies of a d53-sized
        // baseline -- committed, staged, and the clone in flight -- which is a
        // transient peak nothing accounted for (Codex review round 4, finding 2).
        for (domain_id, baseline) in self.staged.drain() {
            self.committed.set(domain_id, baseline);
        }
        self.staged_hours_since_keyframe.clear();
    }

    /// This domain's records became `DOMAIN_GAP`s: leave its baseline exactly
    /// where the fold's mirror is.
    pub fn roll_back(&mut self, domain_id: u16) {
        self.staged.clear(domain_id);
        if let Some(position) = self
            .staged_hours_since_keyframe
            .iter()
            .position(|(id, _)| *id == domain_id)
        {
            let (_, previous) = self.staged_hours_since_keyframe.remove(position);
            self.set_hours_since_keyframe(domain_id, previous);
        }
    }

    /// Retained bytes, committed and staged, for the capture's memory accounting.
    pub fn retained_bytes(&self) -> usize {
        self.committed
            .retained_bytes()
            .saturating_add(self.staged.retained_bytes())
    }
}

/// The consumer's baseline store: one per stream walker.
///
/// Not transactional. A consumer only ever sees records that were PUBLISHED, so
/// there is nothing to roll back — which is exactly the invariant the producer's
/// transaction exists to preserve.
#[derive(Clone, Debug, Default)]
pub struct PayloadSpanDeltaConsumer {
    baselines: PayloadSpanDeltaBaselines,
}

impl PayloadSpanDeltaConsumer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconstruct the exact payload the domain encoded, and advance.
    ///
    /// `identity` is the H4CR header of the record that CARRIED this envelope,
    /// and the envelope must claim exactly it — same domain, same sequence, same
    /// hour. A record that reconstructs perfectly but was framed under a
    /// different hour would attribute one hour's observed state to another,
    /// which no digest over the payload can detect. See
    /// [`PayloadSpanDeltaIdentity`].
    ///
    /// On any error the baseline is left UNTOUCHED, so the chain re-synchronises
    /// at the next keyframe rather than a later delta silently applying to
    /// whatever this record would have left behind.
    pub fn apply(
        &mut self,
        identity: PayloadSpanDeltaIdentity,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        let record = decode_payload(payload)?;
        if record.identity != identity {
            return Err(format!(
                "PSD1 envelope claims domain {} sequence {} hour {} but rides an H4CR record for \
                 domain {} sequence {} hour {}",
                record.identity.domain_id,
                record.identity.source_sequence,
                record.identity.game_hour_raw,
                identity.domain_id,
                identity.source_sequence,
                identity.game_hour_raw
            ));
        }
        let reconstructed = apply_to_baseline(&record, self.baselines.get(record.domain_id()))?;
        self.baselines.set(
            record.domain_id(),
            PayloadSpanDeltaBaseline {
                basis: reconstructed[VOLATILE_HEADER_LEN..].to_vec(),
                basis_sha256: record.basis_sha256,
            },
        );
        Ok(reconstructed)
    }

    pub fn retained_bytes(&self) -> usize {
        self.baselines.retained_bytes()
    }
}

/// Reconstruct a DOMAIN_DATA payload when its record flags say it is an
/// envelope, and pass everything else through untouched.
///
/// This is the whole consumer-side contract, and every stream walker uses it:
/// the fold, the seal walk, and the manifest summary. `Ok(None)` means "not an
/// envelope, use the payload you were given" — which is what a domain that is
/// not delta-enabled, and every record of a capture taken before this shipped,
/// returns.
///
/// THE FLAG IS THE SIGNAL, never the payload's shape. See
/// [`crate::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA`].
pub fn reconstruct_if_enveloped(
    consumer: &mut PayloadSpanDeltaConsumer,
    record_flags: u32,
    identity: PayloadSpanDeltaIdentity,
    payload: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    if record_flags & crate::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA == 0 {
        return Ok(None);
    }
    consumer.apply(identity, payload).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    const D53: u16 = 53;

    fn identity(hour: i32, sequence: u64) -> PayloadSpanDeltaIdentity {
        PayloadSpanDeltaIdentity {
            domain_id: D53,
            source_sequence: sequence,
            game_hour_raw: hour,
        }
    }

    fn ident() -> PayloadSpanDeltaIdentity {
        identity(60_830_824, 7)
    }

    fn payload_of(basis: Vec<u8>, elapsed: u64) -> Vec<u8> {
        let mut out = elapsed.to_le_bytes().to_vec();
        out.extend_from_slice(&basis);
        out
    }

    fn basis_of_len(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    #[test]
    fn a_keyframe_round_trips_to_the_exact_payload() {
        // kills: any encode/decode pair that does not reproduce the payload
        // BYTE FOR BYTE -- the volatile header dropped or duplicated, the basis
        // written at the wrong offset, the identity not carried. It also pins the
        // encoded LENGTH, so a keyframe silently re-emitting the volatile header
        // inside the body (the natural mistake, since the body is "the payload")
        // fails here rather than showing up as 8 wasted bytes a record on a real
        // spool.
        let payload = payload_of(basis_of_len(4096, 7), 1_234);
        let encoded = encode_payload(ident(), &payload, &PayloadSpanDeltaBody::Keyframe).unwrap();
        assert_eq!(
            encoded.len(),
            HEADER_LEN + payload.len() - VOLATILE_HEADER_LEN
        );
        let decoded = decode_payload(&encoded).unwrap();
        assert_eq!(decoded.identity, ident());
        assert_eq!(apply_to_baseline(&decoded, None).unwrap(), payload);
    }

    #[test]
    fn a_no_change_hour_carries_only_a_header_and_still_returns_this_hours_clock() {
        let first = payload_of(basis_of_len(65_536, 3), 1_000);
        // Same basis, DIFFERENT elapsed_wall_ms: this is the whole reason the
        // volatile header is outside the comparison basis.
        let second = payload_of(basis_of_len(65_536, 3), 9_999);
        let baseline = PayloadSpanDeltaBaseline::from_basis(basis_of(&first).to_vec());

        let body = choose_wire_body(&second, Some(&baseline));
        assert!(matches!(body, PayloadSpanDeltaBody::NoChange { .. }));
        let encoded = encode_payload(ident(), &second, &body).unwrap();

        // kills: letting a no-change body carry basis bytes anyway, which is the
        // whole size win. 163 of d53's 461 hours on captures/oz_tip take this path.
        assert_eq!(encoded.len(), HEADER_LEN);

        let decoded = decode_payload(&encoded).unwrap();
        let applied = apply_to_baseline(&decoded, Some(&baseline)).unwrap();
        // kills: reconstructing the BASELINE's elapsed_wall_ms instead of this
        // record's. The two payloads differ in exactly those 8 bytes, so a
        // reconstruction that took them from the baseline would return `first`.
        assert_eq!(applied, second);
        assert_ne!(applied, first);
    }

    #[test]
    fn a_shifted_payload_deltas_instead_of_falling_back_to_a_keyframe() {
        // A row inserted near the FRONT, plus a header count that moved: this is
        // the exact shape that defeats a fixed-block delta and a prefix/suffix
        // splice, and it is 97% of the residual bytes on captures/oz_tip d53.
        let mut basis = basis_of_len(262_144, 5);
        let baseline = PayloadSpanDeltaBaseline::from_basis(basis.clone());
        basis[0..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        let inserted: Vec<u8> = (0..96u8).collect();
        basis.splice(64..64, inserted);
        let payload = payload_of(basis, 77);

        let body = choose_wire_body(&payload, Some(&baseline));
        let PayloadSpanDeltaBody::Delta { op_count, .. } = body else {
            panic!("expected a delta, got {body:?}");
        };
        let encoded = encode_payload(ident(), &payload, &body).unwrap();
        // kills: a spelling that pays for the shift. The whole 256 KiB moved by
        // 96 bytes; the delta must stay in the low hundreds.
        assert!(
            encoded.len() < HEADER_LEN + 1024,
            "shifted delta cost {} bytes in {op_count} ops",
            encoded.len()
        );
        let decoded = decode_payload(&encoded).unwrap();
        assert_eq!(
            apply_to_baseline(&decoded, Some(&baseline)).unwrap(),
            payload
        );
    }

    #[test]
    fn a_wrong_or_absent_baseline_is_refused_by_name_rather_than_applied() {
        // kills: dropping the base_basis_sha256 check in apply_to_baseline, the
        // ONLY thing that stops a stale consumer baseline from producing a
        // payload the recorder never observed.
        let baseline = PayloadSpanDeltaBaseline::from_basis(basis_of_len(8192, 1));
        let mut changed = basis_of_len(8192, 1);
        changed[4000] ^= 0xff;
        let payload = payload_of(changed, 5);
        let body = choose_wire_body(&payload, Some(&baseline));
        let decoded = decode_payload(&encode_payload(ident(), &payload, &body).unwrap()).unwrap();

        let stale = PayloadSpanDeltaBaseline::from_basis(basis_of_len(8192, 9));
        let error = apply_to_baseline(&decoded, Some(&stale)).unwrap_err();
        assert!(error.contains("is not the"), "{error}");
        assert!(error.contains(&hex32(&baseline.basis_sha256)), "{error}");

        let error = apply_to_baseline(&decoded, None).unwrap_err();
        assert!(error.contains("no retained baseline"), "{error}");
    }

    #[test]
    fn an_envelope_under_the_wrong_h4cr_header_is_refused_by_name() {
        // kills: dropping the H4CR identity join (Codex review round 1, finding
        // 1). A digest over the payload proves WHICH BYTES the record carries and
        // says nothing about WHICH HOUR they were observed in, so a record whose
        // envelope was built for hour H riding under a header for H+1
        // reconstructs perfectly and misattributes an hour -- the standing stop
        // condition -- without a single corrupt byte.
        let mut producer = PayloadSpanDeltaProducer::new();
        let mut consumer = PayloadSpanDeltaConsumer::new();
        let payload = payload_of(basis_of_len(4096, 2), 11);
        let wrapped = producer.wrap(identity(60_830_824, 7), &payload).unwrap();
        producer.commit();

        for wrong in [
            identity(60_830_825, 7),
            identity(60_830_824, 8),
            PayloadSpanDeltaIdentity {
                domain_id: 43,
                source_sequence: 7,
                game_hour_raw: 60_830_824,
            },
        ] {
            let error = consumer.apply(wrong, &wrapped).unwrap_err();
            assert!(error.contains("rides an H4CR record for"), "{error}");
        }
        // and the right one still works, so the join is a join and not a wall
        assert_eq!(
            consumer.apply(identity(60_830_824, 7), &wrapped).unwrap(),
            payload
        );
    }

    #[test]
    fn a_tampered_delta_body_refuses_at_the_reconstruction_hash() {
        // kills: dropping the reconstructed-basis re-hash. A baseline that hashes
        // right but a program tampered with in flight must still fail.
        //
        // The disturbed byte is the LAST of the basis on purpose, so the program
        // ends in a one-byte literal and the encoding's final byte is that
        // literal's DATA. Flipping a byte inside an op's length or offset field
        // is caught earlier and more cheaply by the structural checks in
        // `decode_payload` -- which is the wrong thing for this test to pin,
        // because it would still pass with the reconstruction hash deleted.
        let baseline = PayloadSpanDeltaBaseline::from_basis(basis_of_len(8192, 1));
        let mut changed = basis_of_len(8192, 1);
        changed[8191] ^= 0xff;
        let payload = payload_of(changed, 5);
        let body = choose_wire_body(&payload, Some(&baseline));
        let mut encoded = encode_payload(ident(), &payload, &body).unwrap();
        let last = encoded.len() - 1;
        encoded[last] ^= 1;
        let decoded = decode_payload(&encoded).unwrap();
        assert!(matches!(decoded.body, PayloadSpanDeltaBody::Delta { .. }));
        assert!(apply_to_baseline(&decoded, Some(&baseline))
            .unwrap_err()
            .contains("does not hash to basis_sha256"));
    }

    #[test]
    fn a_malformed_record_errs_and_never_allocates_from_an_unchecked_length() {
        let payload = payload_of(basis_of_len(1024, 2), 5);
        let good = encode_payload(ident(), &payload, &PayloadSpanDeltaBody::Keyframe).unwrap();

        // kills: allocating the op vector from `op_count` before the body is
        // known to hold that many ops. `op_count = u32::MAX` with an empty body
        // satisfies the total-length check and would ask for ~85 GB.
        let mut hostile = good[..HEADER_LEN].to_vec();
        hostile[18] = BODY_KIND_DELTA;
        hostile[20..24].copy_from_slice(&(VOLATILE_HEADER_LEN as u32 + 1024).to_le_bytes());
        hostile[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        hostile[28..32].copy_from_slice(&0u32.to_le_bytes());
        hostile[80..112].copy_from_slice(&[0x11; 32]);
        assert!(decode_payload(&hostile).is_err());

        // kills: allocating the keyframe copy from the body before the declared
        // reconstructed length is bounded.
        let mut hostile = good.clone();
        hostile[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_payload(&hostile)
            .unwrap_err()
            .contains("reconstructed length"));

        // kills: admitting an unassigned body kind.
        let mut hostile = good.clone();
        hostile[18] = 7;
        assert!(decode_payload(&hostile)
            .unwrap_err()
            .contains("not assigned"));

        // kills: admitting an envelope for a domain nobody enabled, which is what
        // would let a stray flag bit divert an unrelated domain's payload through
        // this codec.
        let mut hostile = good.clone();
        hostile[16..18].copy_from_slice(&99u16.to_le_bytes());
        assert!(decode_payload(&hostile)
            .unwrap_err()
            .contains("not delta-enabled"));

        // kills: admitting a nonzero reserved field.
        let mut hostile = good.clone();
        hostile[44] = 1;
        assert!(decode_payload(&hostile)
            .unwrap_err()
            .contains("reserved field"));
    }

    #[test]
    fn a_program_with_more_ops_than_output_bytes_is_refused_before_the_op_vector() {
        // kills: reserving the op vector from `op_count` before checking it
        // against the declared basis length (Codex review round 3, finding 1).
        // The body bound alone admits `basis_len = 1` with `op_count = MAX_OPS`
        // and a body of that many five-byte literal headers: ~36 MiB of record
        // that reserves four million `PayloadSpanDeltaOp` -- several hundred MiB
        // -- before the first op is walked and found to overrun. Every op
        // produces at least one byte, so more ops than basis bytes is an
        // impossible program and must be refused as one.
        //
        // The record below is built at the SMALLEST op count that trips the rule
        // rather than at MAX_OPS, because the point is the ordering and not the
        // size: a test that really allocated 36 MiB to prove an allocation is not
        // made would be its own defect.
        let basis_len = 4usize;
        let op_count = 5usize;
        let body: Vec<u8> = (0..op_count)
            .flat_map(|_| {
                let mut op = vec![OP_LITERAL];
                op.extend_from_slice(&1u32.to_le_bytes());
                op.push(0xAA);
                op
            })
            .collect();
        let mut hostile = Vec::new();
        hostile.extend_from_slice(&0u64.to_le_bytes());
        hostile.extend_from_slice(&MAGIC);
        hostile.extend_from_slice(&VERSION.to_le_bytes());
        hostile.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        hostile.extend_from_slice(&D53.to_le_bytes());
        hostile.push(BODY_KIND_DELTA);
        hostile.push(VOLATILE_HEADER_LEN as u8);
        hostile.extend_from_slice(&((VOLATILE_HEADER_LEN + basis_len) as u32).to_le_bytes());
        hostile.extend_from_slice(&(op_count as u32).to_le_bytes());
        hostile.extend_from_slice(&(body.len() as u32).to_le_bytes());
        hostile.extend_from_slice(&ident().source_sequence.to_le_bytes());
        hostile.extend_from_slice(&ident().game_hour_raw.to_le_bytes());
        hostile.extend_from_slice(&[0u8; 4]);
        hostile.extend_from_slice(&[0xAB; 32]);
        hostile.extend_from_slice(&[0xCD; 32]);
        assert_eq!(hostile.len(), HEADER_LEN);
        hostile.extend_from_slice(&body);

        let error = decode_payload(&hostile).unwrap_err();
        assert!(error.contains("produces at least one byte"), "{error}");
    }

    #[test]
    fn the_post_resequence_stamp_settles_the_field_the_join_reads() {
        // kills: deleting `stamp_source_sequence`, or pointing it at the wrong
        // offset (Codex review round 2, finding 1). The producer wraps during the
        // frame BUILD; the frame's later drop passes remove records and
        // `resequence_compact_records` renumbers every survivor, so a sequence
        // baked at build time overshoots by the number of records dropped ahead
        // of it and the consumer's identity join refuses the hour. Measured on
        // `captures/ob_first_bmp` for the domains that already bake one: 69 of
        // 369 d41 hours, 68 of 368 d100 hours.
        let mut producer = PayloadSpanDeltaProducer::new();
        let mut consumer = PayloadSpanDeltaConsumer::new();
        let payload = payload_of(basis_of_len(4096, 6), 42);
        let built_with = identity(60_830_824, 17);
        let mut wrapped = producer.wrap(built_with, &payload).unwrap();
        producer.commit();

        // Two records ahead of this one were dropped, so the stream ships it as 15.
        let shipped = identity(60_830_824, 15);
        assert!(consumer.apply(shipped, &wrapped).is_err());
        stamp_source_sequence(&mut wrapped, shipped.source_sequence).unwrap();
        assert_eq!(consumer.apply(shipped, &wrapped).unwrap(), payload);

        // kills: stamping over bytes a digest covers. The basis digest is taken
        // over the payload after its volatile header, and the stamped field is in
        // the envelope's own header, so the reconstruction still verifies -- which
        // the successful `apply` above already proves, and this pins the field
        // that actually moved.
        let decoded = decode_payload(&wrapped).unwrap();
        assert_eq!(decoded.identity, shipped);

        // kills: writing into a payload that is not an envelope.
        let mut plain = payload.clone();
        assert!(stamp_source_sequence(&mut plain, 99)
            .unwrap_err()
            .contains("magic mismatch"));
        assert_eq!(plain, payload);
    }

    #[test]
    fn a_tiny_record_cannot_reserve_a_huge_buffer_before_its_copies_are_proven() {
        // kills: reserving the reconstruction buffer from the DECLARED length
        // instead of from a proven program (Codex review round 1, finding 2).
        //
        // This record is `HEADER_LEN + 9` bytes. It correctly names a 1 KiB
        // baseline the consumer really holds, declares the maximum reconstructed
        // length, and carries ONE copy op claiming to produce all of it. Every
        // structural check in `decode_payload` passes -- the op total equals the
        // declaration -- so the old spelling reached
        // `Vec::with_capacity(MAX_RECONSTRUCTED_LEN)` and asked for 256 MiB
        // before discovering the copy reaches outside a 1 KiB baseline. Six
        // walkers doing that is an abort produced by a malformed record.
        //
        // The assertion pins the ERROR the proof pass produces. What makes the
        // ORDER safe is structural rather than asserted: `replay_ops` reserves
        // from `ProvenProgram::output_len`, and a `ProvenProgram` cannot be
        // obtained without running the extent proof, so moving the allocation
        // earlier does not weaken the decoder, it fails to compile.
        let baseline = PayloadSpanDeltaBaseline::from_basis(basis_of_len(1024, 4));
        let basis_len = (MAX_RECONSTRUCTED_LEN - VOLATILE_HEADER_LEN) as u32;
        let mut hostile = Vec::with_capacity(HEADER_LEN + COPY_OP_LEN);
        hostile.extend_from_slice(&0u64.to_le_bytes());
        hostile.extend_from_slice(&MAGIC);
        hostile.extend_from_slice(&VERSION.to_le_bytes());
        hostile.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        hostile.extend_from_slice(&D53.to_le_bytes());
        hostile.push(BODY_KIND_DELTA);
        hostile.push(VOLATILE_HEADER_LEN as u8);
        hostile.extend_from_slice(&(MAX_RECONSTRUCTED_LEN as u32).to_le_bytes());
        hostile.extend_from_slice(&1u32.to_le_bytes());
        hostile.extend_from_slice(&(COPY_OP_LEN as u32).to_le_bytes());
        hostile.extend_from_slice(&ident().source_sequence.to_le_bytes());
        hostile.extend_from_slice(&ident().game_hour_raw.to_le_bytes());
        hostile.extend_from_slice(&[0u8; 4]);
        hostile.extend_from_slice(&[0xAB; 32]);
        hostile.extend_from_slice(&baseline.basis_sha256);
        assert_eq!(hostile.len(), HEADER_LEN);
        hostile.push(OP_COPY);
        hostile.extend_from_slice(&0u32.to_le_bytes());
        hostile.extend_from_slice(&basis_len.to_le_bytes());

        let decoded = decode_payload(&hostile).expect("structurally well-formed");
        let error = apply_to_baseline(&decoded, Some(&baseline)).unwrap_err();
        assert!(
            error.contains("from a 1024-byte baseline"),
            "the copy extent must be refused against the baseline: {error}"
        );
    }

    #[test]
    fn a_program_of_many_tiny_literals_is_never_materialised_op_by_op() {
        // kills: decoding the program into a `Vec<PayloadSpanDeltaOp>` (Codex
        // review round 4, finding 1). Round 3 closed `op_count > basis_len`; the
        // EQUAL case was still open and is worse, because it is a legal-looking
        // program: `basis_len == op_count` with every op a one-byte literal
        // passes every count bound. Materialised, that is ~32 bytes of vector per
        // op PLUS one heap allocation per literal -- for the ceiling case, ~134
        // MiB and four million allocations, in each of up to eight parallel fold
        // workers, all before the baseline digest can refuse the record.
        //
        // What the assertions pin is the REPRESENTATION: the decoded body carries
        // the program's wire bytes and nothing else, so the decoded record cannot
        // be larger than the record that arrived, whatever the op count. A
        // materialising decoder cannot satisfy `program == body`, because it does
        // not keep the body.
        let op_count = 100_000usize;
        let basis_len = op_count;
        let mut body = Vec::with_capacity(op_count * (LITERAL_OP_HEADER_LEN + 1));
        for i in 0..op_count {
            body.push(OP_LITERAL);
            body.extend_from_slice(&1u32.to_le_bytes());
            body.push(i as u8);
        }
        let mut record = Vec::new();
        record.extend_from_slice(&0u64.to_le_bytes());
        record.extend_from_slice(&MAGIC);
        record.extend_from_slice(&VERSION.to_le_bytes());
        record.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        record.extend_from_slice(&D53.to_le_bytes());
        record.push(BODY_KIND_DELTA);
        record.push(VOLATILE_HEADER_LEN as u8);
        record.extend_from_slice(&((VOLATILE_HEADER_LEN + basis_len) as u32).to_le_bytes());
        record.extend_from_slice(&(op_count as u32).to_le_bytes());
        record.extend_from_slice(&(body.len() as u32).to_le_bytes());
        record.extend_from_slice(&ident().source_sequence.to_le_bytes());
        record.extend_from_slice(&ident().game_hour_raw.to_le_bytes());
        record.extend_from_slice(&[0u8; 4]);
        record.extend_from_slice(&[0xAB; 32]);
        record.extend_from_slice(&[0xCD; 32]);
        assert_eq!(record.len(), HEADER_LEN);
        record.extend_from_slice(&body);

        let decoded = decode_payload(&record).expect("structurally well-formed");
        let PayloadSpanDeltaBody::Delta {
            ref program,
            op_count: decoded_op_count,
            ..
        } = decoded.body
        else {
            panic!("expected a delta");
        };
        assert_eq!(decoded_op_count as usize, op_count);
        assert_eq!(program.as_slice(), body.as_slice());

        // and it still refuses, by name, for the baseline it does not have
        assert!(apply_to_baseline(&decoded, None)
            .unwrap_err()
            .contains("no retained baseline"));
    }

    #[test]
    fn an_envelope_that_would_not_fit_an_h4cr_record_is_refused_not_emitted() {
        // kills: bounding the PAYLOAD and not the ENVELOPE (Codex review round 6,
        // finding 1). A keyframe envelope is `payload + ENVELOPE_OVERHEAD`, so
        // the largest payload that can be wrapped is `COMPACT_MAX_PAYLOAD_LEN -
        // ENVELOPE_OVERHEAD`; one byte more passes every bound on the payload and
        // emits a record the outer H4CR parser refuses as `PayloadTooLarge`,
        // which is a framing failure rather than a named domain exclusion.
        //
        // Driven through `encoded_envelope_len` rather than through a real
        // 256 MiB payload: that is the function `encode_payload_with_basis_digest`
        // calls, and it takes the capacity of the output buffer from its return
        // value, so an encoder that skipped the check would not compile. Standing
        // up a quarter-gigabyte allocation inside a parallel test suite to observe
        // arithmetic would be its own defect.
        let ceiling = crate::COMPACT_MAX_PAYLOAD_LEN as usize;
        assert_eq!(ENVELOPE_OVERHEAD, HEADER_LEN - VOLATILE_HEADER_LEN);

        // The largest keyframe that fits, and the payload it carries.
        let largest_body = ceiling - HEADER_LEN;
        assert_eq!(encoded_envelope_len(largest_body).unwrap(), ceiling);
        assert_eq!(largest_body + VOLATILE_HEADER_LEN, ceiling - ENVELOPE_OVERHEAD);

        // One byte more is refused, by name, rather than emitted.
        let error = encoded_envelope_len(largest_body + 1).unwrap_err();
        assert!(error.contains("does not fit an H4CR record"), "{error}");

        // And a payload the record framing itself would refuse cannot sneak in
        // through the basis bound either.
        assert!(encoded_envelope_len(usize::MAX).is_err());
    }

    #[test]
    fn a_no_change_record_declaring_the_wrong_length_copies_nothing_before_refusing() {
        // kills: cloning the basis before checking the DECLARED length against it
        // (Codex review round 7, finding 2). A 112-byte header-only `NoChange`
        // can name a baseline the consumer really holds -- its digest is
        // observable -- while declaring some other reconstructed length. The
        // first spelling cloned the 3.25 MB basis and built a second
        // payload-sized buffer from it before comparing lengths, so a record
        // smaller than this comment cost ~2x a d53 basis in each of the walkers
        // that see it.
        //
        // What the assertion pins is the ERROR: it must be the length complaint,
        // raised from the borrowed source, and not the reconstruction-length
        // complaint that could only be reached after both copies were made. The
        // ordering itself is structural -- the basis is a `Cow` borrowed from the
        // baseline, and the only owned buffer is the payload that is returned.
        let basis = basis_of_len(65_536, 5);
        let baseline = PayloadSpanDeltaBaseline::from_basis(basis.clone());
        let payload = payload_of(basis, 7);
        let body = choose_wire_body(&payload, Some(&baseline));
        assert!(matches!(body, PayloadSpanDeltaBody::NoChange { .. }));
        let mut encoded = encode_payload(ident(), &payload, &body).unwrap();
        assert_eq!(encoded.len(), HEADER_LEN);

        // Declare a different reconstructed length; the body still names the
        // baseline correctly, so the digest join has nothing to complain about.
        let lie = (payload.len() + 4096) as u32;
        encoded[20..24].copy_from_slice(&lie.to_le_bytes());
        let decoded = decode_payload(&encoded).expect("structurally well-formed");
        let error = apply_to_baseline(&decoded, Some(&baseline)).unwrap_err();
        assert!(
            error.contains("but the basis it names is"),
            "expected the length refusal before any copy: {error}"
        );
    }

    #[test]
    fn arbitrary_bytes_never_panic_the_decoder() {
        // kills: any indexing, slicing or arithmetic in `decode_payload` and
        // `walk_program` that trusts a declared length, count or offset. The
        // decoder reads bytes written by a producer this process did not run --
        // a spool can be truncated, damaged in transit, or simply older than the
        // reader -- so a panic here is a crashed replay of a capture that should
        // have been refused by name. The MAGIC is stamped into every input long
        // enough to hold it, because bytes that fail the magic check exit on the
        // first branch and would exercise nothing past it.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 300;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            if bytes.len() >= 12 {
                bytes[8..12].copy_from_slice(&MAGIC);
            }
            let _ = decode_payload(&bytes);
        }
    }

    #[test]
    fn the_producer_and_consumer_stay_in_lockstep_over_a_run_of_hours() {
        let mut producer = PayloadSpanDeltaProducer::new();
        let mut consumer = PayloadSpanDeltaConsumer::new();
        let mut basis = basis_of_len(32_768, 4);
        let mut wire_bytes = 0usize;
        let mut raw_bytes = 0usize;
        for hour in 0..64u64 {
            match hour % 8 {
                1 => basis[(hour as usize * 97) % 32_000] ^= 0x5a,
                3 => {
                    let inserted: Vec<u8> = (0..48u8).map(|b| b ^ hour as u8).collect();
                    basis.splice(1000..1000, inserted);
                }
                5 => {
                    basis.truncate(basis.len() - 24);
                }
                _ => {}
            }
            let payload = payload_of(basis.clone(), hour * 1_000);
            raw_bytes += payload.len();
            let id = identity(60_830_824 + hour as i32, hour);
            let wrapped = producer.wrap(id, &payload).unwrap();
            producer.commit();
            wire_bytes += wrapped.len();
            let reconstructed = consumer.apply(id, &wrapped).unwrap();
            // kills: any drift between the two baselines. One wrong hour here
            // and every later hour reconstructs against the wrong bytes.
            assert_eq!(reconstructed, payload, "hour {hour}");
        }
        // kills: a spelling that does not actually compress this shape. 64 hours
        // of a 32 KiB payload with four kinds of small edit must cost far less
        // than the 2 MB of carrying it whole.
        assert!(
            wire_bytes * 10 < raw_bytes,
            "wire {wire_bytes} vs raw {raw_bytes}"
        );
    }

    #[test]
    fn a_rolled_back_frame_leaves_the_producer_exactly_where_the_consumer_is() {
        // kills: committing the baseline advance at emission. A frame whose
        // record becomes a DOMAIN_GAP was never delivered, so a producer that
        // advanced there would name a baseline the consumer cannot produce, and
        // EVERY later hour would refuse.
        let mut producer = PayloadSpanDeltaProducer::new();
        let mut consumer = PayloadSpanDeltaConsumer::new();
        let mut basis = basis_of_len(16_384, 8);

        let delivered = payload_of(basis.clone(), 1);
        let id0 = identity(60_830_824, 1);
        let wrapped = producer.wrap(id0, &delivered).unwrap();
        producer.commit();
        assert_eq!(consumer.apply(id0, &wrapped).unwrap(), delivered);

        // This frame's record is rejected: the consumer never sees it.
        basis[100] ^= 0xff;
        let dropped = payload_of(basis.clone(), 2);
        let _ = producer.wrap(identity(60_830_825, 2), &dropped).unwrap();
        producer.roll_back(D53);
        producer.commit();

        basis[200] ^= 0xff;
        let next = payload_of(basis.clone(), 3);
        let id2 = identity(60_830_826, 3);
        let wrapped = producer.wrap(id2, &next).unwrap();
        producer.commit();
        assert_eq!(consumer.apply(id2, &wrapped).unwrap(), next);
    }

    #[test]
    fn two_records_in_one_frame_chain_through_the_staged_baseline() {
        // kills: `current()` reading the COMMITTED baseline when a STAGED one
        // exists, which would make a domain's second record in a frame diff
        // against the previous frame while the consumer -- who applies records in
        // stream order -- diffs against the first. The two would then disagree
        // about what the baseline is, and every later hour of that domain would
        // refuse until the cadence.
        //
        // NOT a live path today, and the earlier wording here was wrong to imply
        // it was: d53's A30 companion rows are a second DOMAIN_DATA record for
        // the same domain in the same frame, but they are appended RAW and
        // UNFLAGGED by `append_compact_diplomacy_intel_operatives_data_record`
        // (`collector/src/attach_log.rs`, the A30 insertion), so they never touch
        // a baseline. See MEASUREMENT.md section 6. This test guards the
        // invariant, not a path in production, and it is worth guarding because
        // the staged/committed split is exactly what makes rollback correct.
        let mut producer = PayloadSpanDeltaProducer::new();
        let mut consumer = PayloadSpanDeltaConsumer::new();
        let mut basis = basis_of_len(16_384, 2);
        let first = payload_of(basis.clone(), 1);
        let id0 = identity(60_830_824, 1);
        let wrapped = producer.wrap(id0, &first).unwrap();
        producer.commit();
        assert_eq!(consumer.apply(id0, &wrapped).unwrap(), first);

        basis[10] ^= 0xff;
        let second = payload_of(basis.clone(), 2);
        basis[20] ^= 0xff;
        let third = payload_of(basis.clone(), 3);
        let id1 = identity(60_830_825, 2);
        let id2 = identity(60_830_825, 3);
        let wrapped_second = producer.wrap(id1, &second).unwrap();
        let wrapped_third = producer.wrap(id2, &third).unwrap();
        producer.commit();
        assert_eq!(consumer.apply(id1, &wrapped_second).unwrap(), second);
        assert_eq!(consumer.apply(id2, &wrapped_third).unwrap(), third);
    }

    #[test]
    fn the_keyframe_cadence_resynchronises_a_consumer_that_lost_a_record() {
        // kills: deleting the cadence. Without it a consumer that misses one
        // record refuses every later hour of the capture; with it the stall is
        // bounded by KEYFRAME_CADENCE_HOURS.
        let mut producer = PayloadSpanDeltaProducer::new();
        let mut consumer = PayloadSpanDeltaConsumer::new();
        let mut basis = basis_of_len(4096, 3);
        let mut refusals = 0usize;
        let mut recovered_at = None;
        for hour in 0..(KEYFRAME_CADENCE_HOURS as u64 + 4) {
            basis[(hour as usize * 7) % 4000] ^= 0x31;
            let payload = payload_of(basis.clone(), hour);
            let id = identity(60_830_824 + hour as i32, hour);
            let wrapped = producer.wrap(id, &payload).unwrap();
            producer.commit();
            // The consumer never receives hour 1.
            if hour == 1 {
                continue;
            }
            match consumer.apply(id, &wrapped) {
                Ok(reconstructed) => {
                    assert_eq!(reconstructed, payload, "hour {hour}");
                    if hour > 1 && recovered_at.is_none() {
                        recovered_at = Some(hour);
                    }
                }
                Err(error) => {
                    assert!(error.contains("PSD1"), "{error}");
                    refusals += 1;
                }
            }
        }
        assert!(refusals > 0, "the lost record must cost NAMED refusals");
        let recovered_at = recovered_at.expect("the cadence must resynchronise the consumer");
        assert!(
            recovered_at <= KEYFRAME_CADENCE_HOURS as u64 + 2,
            "recovered only at hour {recovered_at}"
        );
    }

    #[test]
    fn a_payload_shorter_than_the_declared_volatile_header_refuses_to_wrap() {
        // kills: inventing a split for a grammar that has no elapsed_wall_ms.
        let mut producer = PayloadSpanDeltaProducer::new();
        let error = producer.wrap(ident(), &[1, 2, 3]).unwrap_err();
        assert!(error.contains("shorter than"), "{error}");
    }

    #[test]
    fn a_record_without_the_flag_is_passed_through_untouched() {
        // kills: sniffing the payload instead of reading the record flag. d53
        // also carries `ICCP` companion records, whose first eight bytes are not
        // an elapsed_wall_ms and which no baseline may advance on.
        let mut consumer = PayloadSpanDeltaConsumer::new();
        let plain = payload_of(basis_of_len(4096, 6), 12);
        assert_eq!(
            reconstruct_if_enveloped(&mut consumer, 0, ident(), &plain).unwrap(),
            None
        );
        assert_eq!(consumer.retained_bytes(), 0);
    }

    #[test]
    fn the_enabled_set_and_the_allowed_record_flag_agree() {
        // kills: enabling a domain in one place and not the other, which would
        // make every one of its records fail `validate_compact_record_semantics`
        // with a rejected flag bit.
        for domain_id in 0..=300u16 {
            let allowed = crate::compact_record_allowed_flags(
                domain_id,
                crate::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            ) & crate::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                != 0;
            assert_eq!(
                allowed,
                is_delta_enabled_domain(domain_id),
                "domain {domain_id}"
            );
            assert_eq!(
                allowed,
                DELTA_ENABLED_DOMAINS.contains(&domain_id),
                "domain {domain_id}"
            );
        }
        // The flag is DOMAIN_DATA only: a perf record carrying it is malformed.
        for domain_id in DELTA_ENABLED_DOMAINS {
            assert_eq!(
                crate::compact_record_allowed_flags(
                    *domain_id,
                    crate::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                ) & crate::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
                0
            );
        }
    }
}
