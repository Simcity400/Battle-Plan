//! Lane PE: what the d47 discovery-witness REPLAY costs, and what removing
//! the dead per-span zero-fill takes off it.
//!
//! # Why this file exists
//!
//! oz_tip prices d47 at 15.15 ms/frame of a 59.1 ms critical path — rank 2,
//! 25.7%. Its own counters split the 23.37 ms read (443 invocations) into
//! discovery 14.53, walk 6.77 and 2.07 of remainder, and split discovery
//! again into `discovery_reused_us` 9.61 ms/invocation (the witness replay,
//! run on 408 of 443 frames, twice per frame — once at open and once after
//! the walk) against `discovery_full_us` 4.92. **The replay is 41% of d47's
//! read**, and it is the phase this module measures.
//!
//! The replay's shape at oz_tip, per frame: 4,137 spans, 116,902 pieces,
//! 22.7 MB of witness bytes, and `witness_span_fallbacks` = **0** over the
//! whole capture — the whole-span read never once refused, so the loop below
//! IS the hot path and the per-piece fallback is cold.
//!
//! # The arms
//!
//! [`Arm::Legacy`] is a VERBATIM transcription of the replay as it stood
//! before lane PE, kept in the tree so the differential gate has something
//! real to compare against. [`Arm::Shipped`] calls the shipped
//! [`domain47_witness_replay_locating`]. [`Arm::Compare`] runs both over the
//! same witness and the same memory and asserts they returned the same
//! verdict, the same locator and the same stats.
//!
//! `cargo test -p collector --release -- --ignored --nocapture d47_replay`

use super::*;

// ==================================================================
// The arena the replay reads
// ==================================================================

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        alloc_type: Dword,
        protect: Dword,
    ) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, free_type: Dword) -> Bool;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: Dword,
        old_protect: *mut Dword,
    ) -> Bool;
}
const PROFILE_MEM_RESERVE: Dword = 0x2000;
const PROFILE_MEM_COMMIT: Dword = 0x1000;
const PROFILE_MEM_RELEASE: Dword = 0x8000;
const PROFILE_PAGE_READWRITE: Dword = 0x04;
const PROFILE_PAGE_READONLY: Dword = 0x02;
const PROFILE_PAGE: usize = 0x1000;

/// A witness's memory. ONE `VirtualAlloc` per element, so a shape can put all
/// its spans inside a single VQ region (`chunks: 1`) or spread them over many
/// (`chunks: n`) — the difference between a rig the region cache answers from
/// its MRU and the scattered game heap a real witness sits in.
struct ProfileArena {
    chunks: Vec<Chunk>,
}

struct Chunk {
    base: usize,
    size: usize,
}

impl ProfileArena {
    fn new(chunk_count: usize, chunk_size: usize) -> ProfileArena {
        let size = (chunk_size + 0xFFF) & !0xFFF;
        let chunks = (0..chunk_count)
            .map(|_| {
                let base = unsafe {
                    VirtualAlloc(
                        std::ptr::null_mut(),
                        size,
                        PROFILE_MEM_RESERVE | PROFILE_MEM_COMMIT,
                        PROFILE_PAGE_READWRITE,
                    )
                };
                assert!(!base.is_null(), "profile arena VirtualAlloc failed");
                Chunk {
                    base: base as usize,
                    size,
                }
            })
            .collect();
        ProfileArena { chunks }
    }

    fn write(&self, addr: usize, bytes: &[u8]) {
        assert!(
            self.chunks
                .iter()
                .any(|c| addr >= c.base && addr + bytes.len() <= c.base + c.size),
            "write outside every chunk"
        );
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) };
    }
}

impl Drop for ProfileArena {
    fn drop(&mut self) {
        for chunk in &self.chunks {
            unsafe { VirtualFree(chunk.base as *mut c_void, 0, PROFILE_MEM_RELEASE) };
        }
    }
}

/// Deterministic filler: the witness must hold bytes the compare cannot
/// short-circuit on, so zeros are not good enough.
struct Xorshift(u64);
impl Xorshift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

// ==================================================================
// Staging a witness of a chosen shape
// ==================================================================

/// The geometry knobs. `pieces_per_span` x `spans` and the length cycle set
/// the piece count and the byte total; `inner_gap` is the only thing oz_tip's
/// counters do NOT pin (they give spans, pieces and PIECE bytes, not SPAN
/// bytes), so the profile sweeps it instead of guessing it.
#[derive(Clone, Copy, Debug)]
struct Shape {
    spans: usize,
    pieces_per_span: usize,
    inner_gap: usize,
    /// How many separate VQ regions the spans are spread over. A real witness
    /// sits in the game's heap, so its 4,137 spans are scattered over many
    /// regions and the reads are cold; `regions: 1` is the contiguous rig
    /// that flatters both arms equally but understates the absolute cost.
    regions: usize,
}

impl Shape {
    fn one_region(spans: usize, pieces_per_span: usize, inner_gap: usize) -> Shape {
        Shape { spans, pieces_per_span, inner_gap, regions: 1 }
    }
}

/// Piece lengths, drawn in a CONTINUING cycle across the whole witness — not
/// restarted per span. Two consequences, both wanted:
///
/// * the mean is exactly 194.0 bytes over any whole number of cycles, which is
///   oz_tip's 22,716,117 witness bytes over 116,903 pieces (194.3);
/// * consecutive spans take DIFFERENT windows of the cycle, so their lengths
///   differ. That is what lets `both_replay_forms_agree_on_the_shapes_that_succeed`
///   prove the shipped form asks for the same `(addr, len)` ranges as the form
///   it replaced. With the cycle restarted per span every span was the same
///   length, `widest` equalled every `span.len`, and passing the WHOLE buffer
///   to the fill would have been indistinguishable from slicing it.
const PROFILE_PIECE_LENS: [usize; 10] = [8, 8, 16, 16, 32, 64, 128, 256, 512, 900];
/// Wider than `DOMAIN47_WITNESS_CLUSTER_GAP`, so the clusterer breaks a span
/// here and nowhere else.
const PROFILE_SPAN_GAP: usize = 2048;

struct Staged {
    witness: Domain47DiscoveryWitness,
    _arena: ProfileArena,
    span_bytes: u64,
    /// How many DISTINCT span lengths this shape produced. The range gate is
    /// only meaningful above 1 — see `PROFILE_PIECE_LENS`.
    distinct_span_lens: usize,
}

fn manager_vector(data: usize) -> Domain47ManagerVectorSnapshot {
    Domain47ManagerVectorSnapshot {
        data,
        capacity: 0,
        size: 0,
        entries: Vec::new(),
    }
}

fn stage(shape: Shape) -> Staged {
    // Lay the piece lengths out FIRST, from one continuing cycle, so span
    // lengths differ from each other (see `PROFILE_PIECE_LENS`).
    let lens: Vec<usize> = (0..shape.spans * shape.pieces_per_span)
        .map(|i| PROFILE_PIECE_LENS[i % PROFILE_PIECE_LENS.len()])
        .collect();
    let span_len = |span_index: usize| -> usize {
        let from = span_index * shape.pieces_per_span;
        lens[from..from + shape.pieces_per_span].iter().sum::<usize>()
            + shape.inner_gap * shape.pieces_per_span.saturating_sub(1)
    };
    let widest_span = (0..shape.spans).map(span_len).max().unwrap_or(0);
    assert!(
        widest_span <= DOMAIN47_WITNESS_CLUSTER_MAX_SPAN,
        "shape {shape:?} would exceed the cluster span cap"
    );
    let regions = shape.regions.max(1);
    let spans_per_region = shape.spans.div_ceil(regions);
    let chunk_size = spans_per_region * (widest_span + PROFILE_SPAN_GAP) + 0x1000;
    let mut arena = ProfileArena::new(regions, chunk_size);
    // The witness's pieces must be ADDRESS-SORTED (`domain47_witness_cluster`
    // and the replay both assume it), so the chunks are laid out in address
    // order and filled in that order.
    arena.chunks.sort_by_key(|c| c.base);

    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    let mut pieces: Vec<Domain47WitnessRecord> = Vec::with_capacity(lens.len());
    let mut bytes: Vec<u8> = Vec::with_capacity(lens.iter().sum::<usize>());
    let mut chunk = 0usize;
    let mut addr = arena.chunks[0].base;
    let mut scratch = vec![0u8; *PROFILE_PIECE_LENS.iter().max().expect("non-empty")];
    for span_index in 0..shape.spans {
        if span_index > 0 && span_index % spans_per_region == 0 {
            chunk += 1;
            addr = arena.chunks[chunk].base;
        }
        for piece in 0..shape.pieces_per_span {
            let len = lens[span_index * shape.pieces_per_span + piece];
            let buf = &mut scratch[..len];
            rng.fill(buf);
            arena.write(addr, buf);
            let off = u32::try_from(bytes.len()).expect("witness offset fits u32");
            bytes.extend_from_slice(buf);
            pieces.push(Domain47WitnessRecord {
                addr,
                len: u32::try_from(len).expect("piece len fits u32"),
                off,
            });
            addr += len;
            if piece + 1 < shape.pieces_per_span {
                addr += shape.inner_gap;
            }
        }
        addr += PROFILE_SPAN_GAP;
    }
    let spans = domain47_witness_cluster(&pieces);
    assert_eq!(
        spans.len(),
        shape.spans,
        "clusterer did not reproduce the requested span count for {shape:?}"
    );
    let span_bytes = spans.iter().map(|s| u64::from(s.len)).sum();
    let distinct_span_lens = spans
        .iter()
        .map(|s| s.len)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    Staged {
        witness: Domain47DiscoveryWitness {
            gamestate: 0xAAAA,
            base: 0x1_4000_0000,
            manager_root: 0x1000,
            manager: 0x2000,
            manager_vtable: 0x1402_92F0_00,
            historical: manager_vector(0x3000),
            dynamic: manager_vector(0x4000),
            pieces,
            bytes,
            spans,
            failed: Vec::new(),
            raw_record_count: 0,
            walk: None,
        },
        _arena: arena,
        span_bytes,
        distinct_span_lens,
    }
}

// ==================================================================
// The legacy arm — a verbatim transcription
// ==================================================================

/// `domain47_witness_replay_locating` EXACTLY as it stood before lane PE:
/// `clear()` + `resize(len, 0)` per span (and per piece on the fallback, and
/// per failed read), then `fill` over the whole buffer. Kept so the
/// differential gate compares the shipped form against real replaced code
/// rather than against a paraphrase of it.
fn replay_locating_legacy<F: FnMut(usize, &mut [u8]) -> bool>(
    witness: &Domain47DiscoveryWitness,
    scratch: &mut Vec<u8>,
    miss_addr: &mut u64,
    fill: &mut F,
) -> Result<Domain47WitnessReplayStats, Domain47WitnessMiss> {
    *miss_addr = 0;
    let mut stats = Domain47WitnessReplayStats::default();
    for (ordinal, span) in witness.spans.iter().enumerate() {
        let ordinal = ordinal as u32;
        let pieces = &witness.pieces
            [span.first_piece as usize..(span.first_piece + span.piece_count) as usize];
        scratch.clear();
        scratch.resize(span.len as usize, 0);
        let whole = fill(span.start, &mut scratch[..]);
        if whole {
            for piece in pieces {
                let at = piece.addr - span.start;
                let expected =
                    &witness.bytes[piece.off as usize..piece.off as usize + piece.len as usize];
                let observed = &scratch[at..at + piece.len as usize];
                if observed != expected {
                    let first = observed
                        .iter()
                        .zip(expected)
                        .position(|(a, b)| a != b)
                        .unwrap_or(0);
                    *miss_addr = (piece.addr + first) as u64;
                    return Err(Domain47WitnessMiss::SpanChanged(ordinal));
                }
            }
        } else {
            stats.span_fallbacks += 1;
            for piece in pieces {
                scratch.clear();
                scratch.resize(piece.len as usize, 0);
                if !fill(piece.addr, &mut scratch[..]) {
                    *miss_addr = piece.addr as u64;
                    return Err(Domain47WitnessMiss::SpanUnreadable(ordinal));
                }
                let expected =
                    &witness.bytes[piece.off as usize..piece.off as usize + piece.len as usize];
                if scratch[..] != *expected {
                    let first = scratch
                        .iter()
                        .zip(expected)
                        .position(|(a, b)| a != b)
                        .unwrap_or(0);
                    *miss_addr = (piece.addr + first) as u64;
                    return Err(Domain47WitnessMiss::SpanChanged(ordinal));
                }
            }
        }
        stats.spans += 1;
        stats.pieces += span.piece_count;
        stats.bytes += u64::from(span.len);
    }
    for (ordinal, &(addr, len)) in witness.failed.iter().enumerate() {
        scratch.clear();
        scratch.resize(len as usize, 0);
        if fill(addr, &mut scratch[..]) {
            *miss_addr = addr as u64;
            return Err(Domain47WitnessMiss::FailedReadNowReadable(ordinal as u32));
        }
    }
    Ok(stats)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arm {
    /// The pre-PE code above: zero-fill the scratch per span, copy the span
    /// in, compare each piece out of the copy.
    Legacy,
    /// The shipped [`domain47_witness_replay_locating`]: one sizing of the
    /// scratch for the whole replay instead of a zero-fill per span.
    Shipped,
    /// Legacy and Shipped over the same witness and the same memory, asserted
    /// to agree on verdict, locator and stats.
    Compare,
}

type Verdict = (
    Result<Domain47WitnessReplayStats, Domain47WitnessMiss>,
    u64,
);

/// Run one replay under `arm`, against the LIVE guarded read primitive
/// (`read_into`), which is what production uses.
fn run_arm(arm: Arm, witness: &Domain47DiscoveryWitness, scratch: &mut Vec<u8>) -> Verdict {
    match arm {
        Arm::Legacy => {
            let mut miss = 0u64;
            let out = replay_locating_legacy(witness, scratch, &mut miss, &mut read_into);
            (out, miss)
        }
        Arm::Shipped => {
            let mut miss = 0u64;
            let out = domain47_witness_replay_locating(witness, scratch, &mut miss, &mut read_into);
            (out, miss)
        }
        Arm::Compare => {
            // Separate buffers: the whole point of the change is that the two
            // forms leave the scratch in different states, so sharing one
            // would let the legacy arm's zero-fill mask a shipped-arm bug.
            let mut legacy_scratch = std::mem::take(scratch);
            let mut shipped_scratch = Vec::new();
            let mut legacy_miss = 0u64;
            let mut shipped_miss = 0u64;
            // EVERY GUARDED READ EACH ARM ASKS FOR, in order. The equivalence
            // claim is that the shipped form asks the live process for exactly
            // the ranges the replaced form did — same addresses, same lengths,
            // therefore the same `range_confirmed_good` decision and the same
            // bytes compared. Recording them is a direct proof of that instead
            // of an argument for it.
            let mut legacy_reads: Vec<(usize, usize)> = Vec::new();
            let mut shipped_reads: Vec<(usize, usize)> = Vec::new();
            let legacy = replay_locating_legacy(
                witness,
                &mut legacy_scratch,
                &mut legacy_miss,
                &mut |addr: usize, buf: &mut [u8]| {
                    legacy_reads.push((addr, buf.len()));
                    read_into(addr, buf)
                },
            );
            let shipped = domain47_witness_replay_locating(
                witness,
                &mut shipped_scratch,
                &mut shipped_miss,
                &mut |addr: usize, buf: &mut [u8]| {
                    shipped_reads.push((addr, buf.len()));
                    read_into(addr, buf)
                },
            );
            assert_eq!(
                legacy_reads, shipped_reads,
                "the two replay forms asked the live process for different ranges"
            );
            assert_eq!(
                legacy, shipped,
                "the two replay forms disagreed on the verdict"
            );
            assert_eq!(
                legacy_miss, shipped_miss,
                "the two replay forms disagreed on the miss locator"
            );
            *scratch = legacy_scratch;
            (shipped, shipped_miss)
        }
    }
}

fn micros(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

// ==================================================================
// THE DIFFERENTIAL GATE (not #[ignore]d)
// ==================================================================

/// Every read the production replay takes, resolved BOTH ways, over shapes
/// that SUCCEED.
///
/// kills: dropping the per-read length slice (handing the fill the whole
/// buffer). `Arm::Compare` records every `(addr, len)` each arm asks the live
/// process for and asserts the two sequences are IDENTICAL, so the shipped
/// form cannot quietly widen a read. Every shape below is asserted to contain
/// more than one distinct span length, because with uniform spans `widest`
/// equals every `span.len` and that mutation would be invisible.
#[test]
fn both_replay_forms_agree_on_the_shapes_that_succeed() {
    for shape in [
        Shape::one_region(11, 1, 0),
        Shape::one_region(17, 3, 0),
        Shape::one_region(31, 28, 8),
        Shape::one_region(9, 13, 512),
        Shape { spans: 40, pieces_per_span: 6, inner_gap: 8, regions: 20 },
    ] {
        let staged = stage(shape);
        assert!(
            staged.distinct_span_lens > 1,
            "{shape:?} staged uniform spans, which makes the range gate vacuous"
        );
        page_cache_new_frame();
        let mut scratch = Vec::new();
        let (verdict, miss) = run_arm(Arm::Compare, &staged.witness, &mut scratch);
        let stats = verdict.expect("a staged, unmutated witness must replay clean");
        assert_eq!(stats.spans as usize, shape.spans, "{shape:?}");
        assert_eq!(
            stats.pieces as usize,
            shape.spans * shape.pieces_per_span,
            "{shape:?}"
        );
        assert_eq!(stats.bytes, staged.span_bytes, "{shape:?}");
        assert_eq!(stats.span_fallbacks, 0, "{shape:?}");
        assert_eq!(miss, 0, "{shape:?}");
    }
}

/// The gate above has one weakness and this closes it: a staged witness that
/// still matches proves the two forms agree on reads that SUCCEED and never
/// on reads that REFUSE or DIFFER. So: build each refusing shape by hand and
/// run all three arms over it.
///
/// It ends by asserting a miss was actually produced in every case, so it
/// cannot pass by proving nothing.
///
/// kills: reading a stale scratch tail. The shipped form no longer zeroes the
/// buffer between reads, so a compare that ran past the length it asked for
/// would see the PREVIOUS span's bytes instead of zeros — the mutated-tail
/// case below is exactly that shape, a short piece replayed after a long one.
#[test]
fn both_replay_forms_agree_on_the_refusing_shapes() {
    let mut proved: Vec<&'static str> = Vec::new();

    // 1. A byte inside a piece changed after the seal: SpanChanged, and the
    //    locator must name the exact changed address.
    {
        let staged = stage(Shape::one_region(6, 4, 16));
        let target = staged.witness.pieces[9];
        let at = target.addr + (target.len as usize - 1);
        unsafe { *(at as *mut u8) ^= 0xFF };
        page_cache_new_frame();
        let mut scratch = Vec::new();
        let (verdict, miss) = run_arm(Arm::Compare, &staged.witness, &mut scratch);
        assert!(
            matches!(verdict, Err(Domain47WitnessMiss::SpanChanged(_))),
            "a changed byte must miss, got {verdict:?}"
        );
        assert_eq!(miss, at as u64, "the locator must name the changed byte");
        proved.push("span_changed");
    }

    // 2. The SHORT-PIECE-AFTER-LONG-PIECE shape, mutated in the tail the
    //    previous read left behind. With the shipped form the buffer still
    //    holds the long span's bytes when the short one is compared.
    {
        let staged = stage(Shape::one_region(3, 10, 0));
        // Piece 0 of span 1 is 8 bytes; the span before it is ~1.9 KB.
        let target = staged.witness.pieces[10];
        assert_eq!(target.len, 8, "piece length cycle changed under this test");
        unsafe { *(target.addr as *mut u8) ^= 0xFF };
        page_cache_new_frame();
        let mut scratch = Vec::new();
        let (verdict, miss) = run_arm(Arm::Compare, &staged.witness, &mut scratch);
        assert!(
            matches!(verdict, Err(Domain47WitnessMiss::SpanChanged(_))),
            "got {verdict:?}"
        );
        assert_eq!(miss, target.addr as u64);
        proved.push("short_piece_after_long_piece");
    }

    // 3. A span that is no longer readable at all: SpanUnreadable through the
    //    per-piece fallback (the whole-span read refuses first).
    {
        let mut staged = stage(Shape::one_region(2, 2, 0));
        // Point the second span at a reserved-but-never-committed address.
        let dead = 0x7FF0_0000_0000usize;
        let first = staged.witness.spans[1].first_piece as usize;
        let shift = dead - staged.witness.spans[1].start;
        staged.witness.spans[1].start = dead;
        for piece in &mut staged.witness.pieces[first..] {
            piece.addr += shift;
        }
        page_cache_new_frame();
        let mut scratch = Vec::new();
        let (verdict, miss) = run_arm(Arm::Compare, &staged.witness, &mut scratch);
        assert_eq!(
            verdict,
            Err(Domain47WitnessMiss::SpanUnreadable(1)),
            "an uncommitted span must refuse"
        );
        assert_eq!(miss, dead as u64);
        proved.push("span_unreadable");
    }

    // 4. A read that REFUSED at discovery and is readable now.
    {
        let mut staged = stage(Shape::one_region(2, 2, 0));
        let readable = staged.witness.pieces[0].addr;
        staged.witness.failed.push((readable, 8));
        page_cache_new_frame();
        let mut scratch = Vec::new();
        let (verdict, miss) = run_arm(Arm::Compare, &staged.witness, &mut scratch);
        assert_eq!(verdict, Err(Domain47WitnessMiss::FailedReadNowReadable(0)));
        assert_eq!(miss, readable as u64);
        proved.push("failed_read_now_readable");
    }

    // 5. A read that refused at discovery and still refuses: no miss, and the
    //    zero-length degenerate beside it (both forms hand `read_into` an
    //    empty slice, which it declines).
    {
        let mut staged = stage(Shape::one_region(2, 2, 0));
        staged.witness.failed.push((0x7FF0_0000_0000, 8));
        staged.witness.failed.push((staged.witness.pieces[0].addr, 0));
        page_cache_new_frame();
        let mut scratch = Vec::new();
        let (verdict, miss) = run_arm(Arm::Compare, &staged.witness, &mut scratch);
        assert!(verdict.is_ok(), "a still-refusing read is not a miss: {verdict:?}");
        assert_eq!(miss, 0);
        proved.push("failed_read_still_refuses");
    }

    // 6. THE PER-PIECE FALLBACK, which production has never taken: oz_tip's
    //    `domain47_witness_span_fallbacks` is 0 over the whole capture, so a
    //    test is the only coverage this arm will ever get — and it is the arm
    //    the change touches most, because it is where the scratch is sliced
    //    to a length SHORTER than the span that was read into it a moment
    //    before.
    //
    //    The shape: two adjacent VQ regions that are BOTH readable (one
    //    READWRITE, one READONLY, so `VirtualQuery` will not merge them) with
    //    a span bridging the boundary. `range_confirmed_good` refuses a range
    //    that straddles two regions even when both are good, so the
    //    whole-span read refuses while every piece reads fine on its own.
    {
        let base = unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                2 * PROFILE_PAGE,
                PROFILE_MEM_RESERVE | PROFILE_MEM_COMMIT,
                PROFILE_PAGE_READWRITE,
            )
        } as usize;
        assert!(base != 0, "fallback arena VirtualAlloc failed");
        // Piece A ends exactly at the page boundary; piece B starts 8 bytes
        // past it, so the two share one span (gap 8 < the cluster gap) and
        // the span straddles the boundary.
        let a_addr = base + PROFILE_PAGE - 8;
        let b_addr = base + PROFILE_PAGE + 8;
        let a_bytes = [0x11u8; 8];
        let b_bytes = [0x22u8; 16];
        unsafe {
            std::ptr::copy_nonoverlapping(a_bytes.as_ptr(), a_addr as *mut u8, a_bytes.len());
            std::ptr::copy_nonoverlapping(b_bytes.as_ptr(), b_addr as *mut u8, b_bytes.len());
        }
        let mut old_protect: Dword = 0;
        let ok = unsafe {
            VirtualProtect(
                (base + PROFILE_PAGE) as *mut c_void,
                PROFILE_PAGE,
                PROFILE_PAGE_READONLY,
                &mut old_protect as *mut Dword,
            )
        };
        assert_ne!(ok, 0, "VirtualProtect split failed");

        let mut bytes = a_bytes.to_vec();
        bytes.extend_from_slice(&b_bytes);
        let pieces = vec![
            Domain47WitnessRecord { addr: a_addr, len: 8, off: 0 },
            Domain47WitnessRecord { addr: b_addr, len: 16, off: 8 },
        ];
        let spans = domain47_witness_cluster(&pieces);
        assert_eq!(spans.len(), 1, "the two pieces must share one span");
        assert_eq!(spans[0].len, 32, "the span must bridge the page boundary");
        let witness = Domain47DiscoveryWitness {
            gamestate: 0xAAAA,
            base: 0x1_4000_0000,
            manager_root: 0x1000,
            manager: 0x2000,
            manager_vtable: 0x1402_92F0_00,
            historical: manager_vector(0x3000),
            dynamic: manager_vector(0x4000),
            pieces,
            bytes,
            spans,
            failed: Vec::new(),
            raw_record_count: 0,
            walk: None,
        };

        page_cache_new_frame();
        let mut scratch = Vec::new();
        let (verdict, miss) = run_arm(Arm::Compare, &witness, &mut scratch);
        let stats = verdict.expect("both pieces are individually readable");
        assert_eq!(
            stats.span_fallbacks, 1,
            "the straddling span must have taken the per-piece fallback"
        );
        assert_eq!(stats.pieces, 2);
        assert_eq!(miss, 0);
        proved.push("fallback_span_straddles_two_good_regions");

        // The same fallback, now with the SHORT piece (8 bytes) matching and
        // the LONG one (16) changed, so the compare that follows a
        // shorter-then-longer pair is the one under test. The second page is
        // READONLY at this point, so it is flipped back to write the byte and
        // flipped again — the split, and therefore the fallback, must survive.
        let mut scratch_protect: Dword = 0;
        assert_ne!(
            unsafe {
                VirtualProtect(
                    (base + PROFILE_PAGE) as *mut c_void,
                    PROFILE_PAGE,
                    PROFILE_PAGE_READWRITE,
                    &mut scratch_protect as *mut Dword,
                )
            },
            0,
            "VirtualProtect back to READWRITE failed"
        );
        unsafe { *((b_addr + 5) as *mut u8) = 0x99 };
        assert_ne!(
            unsafe {
                VirtualProtect(
                    (base + PROFILE_PAGE) as *mut c_void,
                    PROFILE_PAGE,
                    PROFILE_PAGE_READONLY,
                    &mut scratch_protect as *mut Dword,
                )
            },
            0,
            "VirtualProtect re-split failed"
        );
        page_cache_new_frame();
        let mut scratch = Vec::new();
        let (verdict, miss) = run_arm(Arm::Compare, &witness, &mut scratch);
        assert_eq!(verdict, Err(Domain47WitnessMiss::SpanChanged(0)));
        assert_eq!(miss, (b_addr + 5) as u64, "the fallback must locate the byte");
        proved.push("fallback_locates_a_changed_byte");

        unsafe { VirtualFree(base as *mut c_void, 0, PROFILE_MEM_RELEASE) };
    }

    assert_eq!(
        proved,
        vec![
            "span_changed",
            "short_piece_after_long_piece",
            "span_unreadable",
            "failed_read_now_readable",
            "failed_read_still_refuses",
            "fallback_span_straddles_two_good_regions",
            "fallback_locates_a_changed_byte",
        ],
        "every refusing shape must have been exercised"
    );
}

// ==================================================================
// THE PROFILE
// ==================================================================

/// Interleaved arms, order flipped per repetition, minimised — the shape
/// `d81_guarded_read_cost_model` uses, and for the same reason: a
/// before-and-after of two builds cannot carry a claim on a box that is also
/// running a game.
#[test]
#[ignore = "profile"]
fn profile_d47_witness_replay_arms() {
    const REPS: usize = 51;
    println!(
        "{:>6} {:>7} {:>5} {:>8} {:>8} | {:>9} {:>9} {:>8}",
        "spans", "pieces", "gap", "regions", "span_MB", "legacy_ms", "shipped_ms", "cut"
    );
    for shape in [
        Shape::one_region(517, 28, 0),
        Shape::one_region(4137, 28, 0),
        Shape::one_region(4137, 28, 8),
        Shape::one_region(4137, 28, 64),
        Shape { spans: 4137, pieces_per_span: 28, inner_gap: 8, regions: 64 },
        Shape { spans: 4137, pieces_per_span: 28, inner_gap: 8, regions: 517 },
        Shape { spans: 4137, pieces_per_span: 28, inner_gap: 8, regions: 4137 },
    ] {
        let staged = stage(shape);
        let mut scratches = [Vec::new(), Vec::new()];
        let mut samples: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
        const ARMS: [Arm; 2] = [Arm::Legacy, Arm::Shipped];
        for rep in 0..REPS {
            // Order flipped every repetition so neither arm always runs
            // first, and the two kept adjacent in time so they see the same
            // machine state.
            for slot in 0..2 {
                let index = (slot + rep) % 2;
                page_cache_new_frame();
                let started = std::time::Instant::now();
                let (verdict, _) = run_arm(ARMS[index], &staged.witness, &mut scratches[index]);
                let us = micros(started.elapsed());
                assert!(verdict.is_ok(), "{:?} missed on a clean witness", ARMS[index]);
                samples[index].push(us);
            }
        }
        let best = |v: &mut Vec<f64>| -> f64 {
            v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
            v[0]
        };
        let legacy = best(&mut samples[0]);
        let shipped = best(&mut samples[1]);
        println!(
            "{:>6} {:>7} {:>5} {:>8} {:>8.2} | {:>9.3} {:>9.3} {:>7.1}%",
            shape.spans,
            shape.spans * shape.pieces_per_span,
            shape.inner_gap,
            shape.regions,
            staged.span_bytes as f64 / (1024.0 * 1024.0),
            legacy / 1000.0,
            shipped / 1000.0,
            100.0 * (legacy - shipped) / legacy,
        );
    }
}

/// The noise floor of THIS box for THIS rig: the same arm, three times in a
/// row. Quote no cut narrower than this.
#[test]
#[ignore = "profile"]
fn profile_d47_witness_replay_noise_floor() {
    let shape = Shape { spans: 4137, pieces_per_span: 28, inner_gap: 8, regions: 517 };
    let staged = stage(shape);
    let mut scratch = Vec::new();
    for run in 0..3 {
        let mut best = f64::MAX;
        for _ in 0..51 {
            page_cache_new_frame();
            let started = std::time::Instant::now();
            let (verdict, _) = run_arm(Arm::Legacy, &staged.witness, &mut scratch);
            best = best.min(micros(started.elapsed()));
            assert!(verdict.is_ok());
        }
        println!("d47_replay noise legacy-only run {run}: {:.3} ms", best / 1000.0);
    }
}
