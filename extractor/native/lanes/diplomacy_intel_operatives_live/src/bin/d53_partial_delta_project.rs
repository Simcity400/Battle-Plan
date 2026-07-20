//! Measurement harness (lane/d53-partial-delta): project the d53 stream volume
//! BEFORE vs AFTER enabling delta emission on tolerated-PARTIAL frames.
//!
//! It replays a REAL collector stream in place (never copies), decodes each d53
//! DOMAIN_DATA frame, and simulates the NEW collector emit policy exactly:
//!   * CLEAN frame (no PARTIAL bit): full baseline; advances the pinned baseline.
//!   * PARTIAL frame with a baseline: DELTA|PARTIAL — changed/new rows + a
//!     tombstone for every baseline row absent from the read (tombstone-all);
//!     the baseline is NOT advanced.
//!   * first frame / partial before any baseline: full (no delta).
//! For each frame it encodes the reduced payload with the authoritative
//! `encode_payload` and sums the on-wire bytes, so the "after" figure is the
//! true reduced payload size (header + framing + trailers included).
//!
//! Usage: d53_partial_delta_project <stream.bin> [max_d53_frames]

use std::collections::{BTreeMap, HashMap};
use std::io::{BufReader, Read};

use diplomacy_intel_operatives_live::compact as d53;

const HDR: usize = compact_contract::COMPACT_RECORD_HEADER_LEN;
const DATA: u16 = compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA;
const D53: u16 = 53;
const PARTIAL: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;

#[derive(Default)]
struct Baseline {
    countries: BTreeMap<u32, d53::CountryDiplomacyRow>,
    wars: BTreeMap<(u32, u32), d53::WarRelationRow>,
    opinions: BTreeMap<(u32, u32), d53::OpinionRow>,
    relation_flags: BTreeMap<(u32, u32, u16), d53::RelationFlagRow>,
    subjects: BTreeMap<(u32, u32), d53::SubjectRow>,
    agencies: BTreeMap<u32, d53::AgencyRow>,
    clusters: BTreeMap<u32, d53::AgencyCapacityClusterRow>,
    operatives: BTreeMap<u32, d53::OperativeRow>,
    cryptology: BTreeMap<(u32, u32), d53::CryptologyRow>,
    state_events: BTreeMap<(u32, Option<u32>, u16, u16, u32), d53::StateEventRow>,
}

impl Baseline {
    fn is_empty(&self) -> bool {
        self.countries.is_empty()
            && self.wars.is_empty()
            && self.opinions.is_empty()
            && self.relation_flags.is_empty()
            && self.subjects.is_empty()
            && self.agencies.is_empty()
            && self.clusters.is_empty()
            && self.operatives.is_empty()
            && self.cryptology.is_empty()
            && self.state_events.is_empty()
    }
}

fn war_key(a: u32, b: u32) -> (u32, u32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn maps_from(snap: &d53::DiplomacyIntelSnapshot) -> Baseline {
    let mut b = Baseline::default();
    for r in &snap.countries {
        b.countries.insert(r.tag_db_id, r.clone());
    }
    for r in &snap.wars {
        b.wars
            .insert(war_key(r.first_tag_db_id, r.second_tag_db_id), r.clone());
    }
    for r in &snap.opinions {
        b.opinions
            .insert((r.src_tag_db_id, r.target_tag_db_id), r.clone());
    }
    for r in &snap.relation_flags {
        b.relation_flags.insert(
            (r.src_tag_db_id, r.target_tag_db_id, r.flag_kind),
            r.clone(),
        );
    }
    for r in &snap.subjects {
        b.subjects
            .insert((r.overlord_tag_db_id, r.subject_tag_db_id), r.clone());
    }
    for r in &snap.agencies {
        b.agencies.insert(r.owner_tag_db_id, r.clone());
    }
    for r in &snap.agency_capacity_clusters {
        b.clusters.insert(r.owner_tag_db_id, r.clone());
    }
    for r in &snap.operatives {
        b.operatives.insert(r.op_id, r.clone());
    }
    for r in &snap.cryptology {
        b.cryptology
            .insert((r.owner_tag_db_id, r.target_tag_db_id), r.clone());
    }
    for r in &snap.state_events {
        b.state_events.insert(
            (
                r.owner_tag_db_id,
                r.target_tag_db_id,
                r.event_kind,
                r.source_kind,
                r.object_hash,
            ),
            r.clone(),
        );
    }
    b
}

const DEL: u32 = d53::ROW_ID_DELETED;

/// Diff `cur` vs `base` (tombstone-all): push changed/new rows, then a tombstone
/// (baseline row with the key field | ROW_ID_DELETED) for every baseline key not
/// present in `cur`. Row widths are fixed, so a tombstone occupies the same wire
/// bytes as the row it deletes — exactly what the real collector emits.
fn diff<K: Ord + Clone, T: Clone + PartialEq>(
    cur: &BTreeMap<K, T>,
    base: &BTreeMap<K, T>,
    out: &mut Vec<T>,
    mut mark_deleted: impl FnMut(&T) -> T,
) {
    for (k, row) in cur {
        if base.get(k).map(|b| b != row).unwrap_or(true) {
            out.push(row.clone());
        }
    }
    for (k, row) in base {
        if !cur.contains_key(k) {
            let _ = k;
            out.push(mark_deleted(row));
        }
    }
}

fn reduced_snapshot(
    base: &Baseline,
    cur: &Baseline,
    src: &d53::DiplomacyIntelSnapshot,
) -> d53::DiplomacyIntelSnapshot {
    let mut s = d53::DiplomacyIntelSnapshot {
        status_flags: src.status_flags | compact_contract::COMPACT_STATUS_DELTA_DATA,
        // trailers cloned whole every delta frame (collector parity)
        operation_anchors: src.operation_anchors.clone(),
        intel_map: src.intel_map.clone(),
        operation_instances: src.operation_instances.clone(),
        incoming_actions: src.incoming_actions.clone(),
        intel_network_graphs: src.intel_network_graphs.clone(),
        factions: src.factions.clone(),
        faction_roster_complete: src.faction_roster_complete,
        subject_autonomy_statuses: src.subject_autonomy_statuses.clone(),
        subject_autonomy_status_complete: src.subject_autonomy_status_complete,
        ..Default::default()
    };
    diff(&cur.countries, &base.countries, &mut s.countries, |r| {
        let mut r = r.clone();
        r.tag_db_id |= DEL;
        r
    });
    diff(&cur.wars, &base.wars, &mut s.wars, |r| {
        let mut r = r.clone();
        r.first_tag_db_id |= DEL;
        r
    });
    diff(&cur.opinions, &base.opinions, &mut s.opinions, |r| {
        let mut r = r.clone();
        r.src_tag_db_id |= DEL;
        r
    });
    diff(
        &cur.relation_flags,
        &base.relation_flags,
        &mut s.relation_flags,
        |r| {
            let mut r = r.clone();
            r.src_tag_db_id |= DEL;
            r
        },
    );
    diff(&cur.subjects, &base.subjects, &mut s.subjects, |r| {
        let mut r = r.clone();
        r.overlord_tag_db_id |= DEL;
        r
    });
    diff(&cur.agencies, &base.agencies, &mut s.agencies, |r| {
        let mut r = r.clone();
        r.owner_tag_db_id |= DEL;
        r
    });
    diff(
        &cur.clusters,
        &base.clusters,
        &mut s.agency_capacity_clusters,
        |r| {
            let mut r = r.clone();
            r.owner_tag_db_id |= DEL;
            r
        },
    );
    diff(&cur.operatives, &base.operatives, &mut s.operatives, |r| {
        let mut r = r.clone();
        r.op_id |= DEL;
        r
    });
    diff(&cur.cryptology, &base.cryptology, &mut s.cryptology, |r| {
        let mut r = r.clone();
        r.owner_tag_db_id |= DEL;
        r
    });
    diff(
        &cur.state_events,
        &base.state_events,
        &mut s.state_events,
        |r| {
            let mut r = r.clone();
            r.owner_tag_db_id |= DEL;
            r
        },
    );
    s
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: d53_partial_delta_project <stream.bin> [max_d53_frames]");
        std::process::exit(2);
    }
    let path = &args[1];
    let max_frames: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(400);

    let f = std::fs::File::open(path).expect("open stream");
    let mut r = BufReader::with_capacity(1 << 20, f);
    let mut hdr = [0u8; HDR];

    let mut baseline = Baseline::default();
    let mut seen_baseline = false;

    let mut frames = 0u64;
    let mut failure_frames = 0u64;
    let mut partial_frames = 0u64;
    let mut clean_frames = 0u64;
    let mut before_bytes = 0u64; // today: full payload every frame
    let mut after_bytes = 0u64; // new: reduced payload
    let mut reduced_rows = 0u64;
    let mut full_rows = 0u64;
    let mut t_old_ns = 0u64;
    let mut t_new_ns = 0u64;
    let mut t_build_ns = 0u64;
    let mut t_diff_ns = 0u64;
    let mut t_newenc_ns = 0u64;
    let mut t_opt_ns = 0u64;
    let mut bench_frames = 0u64;
    let mut opin_base: HashMap<(u32, u32), i32> = HashMap::new();
    let mut opin_gen: HashMap<(u32, u32), u32> = HashMap::new();
    let mut se_base: HashMap<(u32, Option<u32>, u16, u16, u32), i64> = HashMap::new();
    let mut se_gen: HashMap<(u32, Option<u32>, u16, u16, u32), u32> = HashMap::new();

    loop {
        if frames >= max_frames {
            break;
        }
        match read_exact_or_eof(&mut r, &mut hdr) {
            Ok(true) => {}
            _ => break,
        }
        let header = match compact_contract::parse_compact_record_header(&hdr) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("framing error: {e}");
                break;
            }
        };
        let plen = header.payload_len;
        if header.domain_id == D53 && header.record_type == DATA {
            let mut payload = vec![0u8; plen];
            if !read_full(&mut r, &mut payload) {
                break;
            }
            frames += 1;
            let snap = match d53::decode_snapshot_payload(0, &payload) {
                Ok(s) => s,
                Err(errs) => {
                    eprintln!("decode error frame {frames}: {:?}", errs.first());
                    continue;
                }
            };
            let is_partial = snap.status_flags & PARTIAL != 0;
            // FAILURE FRAMES: the collector's failure path returns an empty payload
            // BEFORE ever reaching the delta emit logic, so it never seeds or
            // advances the baseline from them and the fold rejects them. Mirror
            // that: pass such frames through untouched (they are empty, ~0 bytes)
            // and leave the baseline alone. A tolerated-partial SUCCESS frame has
            // only the PARTIAL bit set (mask it out before the failure test, like
            // the collector's post-build gate does).
            let is_failure =
                compact_contract::compact_status_is_failure(snap.status_flags & !PARTIAL);
            before_bytes += plen as u64;
            if is_failure {
                after_bytes += plen as u64;
                failure_frames += 1;
                continue;
            }
            full_rows += snap.row_count() as u64;

            // CPU micro-benchmark on the REAL frame: old-path work (encode the full
            // payload) vs new-path work (build the current maps + diff vs baseline +
            // encode the reduced payload). Timed only on delta-eligible frames (a
            // baseline already exists) so the two paths are comparable.
            if seen_baseline {
                // OLD payload-segment work on a full partial frame: encode the full
                // payload AND run the decode gate over it (build_domain53_payload
                // re-decodes/validates what it encodes).
                let t0 = std::time::Instant::now();
                let full_payload = d53::encode_payload(&snap);
                let _ = std::hint::black_box(d53::decode_snapshot_payload(0, &full_payload));
                t_old_ns += t0.elapsed().as_nanos() as u64;

                // NEW work, broken down: build current maps, diff, encode+gate the
                // (tiny) reduced payload.
                let tb = std::time::Instant::now();
                let cur_bench = maps_from(&snap);
                std::hint::black_box(&cur_bench);
                t_build_ns += tb.elapsed().as_nanos() as u64;

                let td = std::time::Instant::now();
                let reduced_bench = reduced_snapshot(&baseline, &cur_bench, &snap);
                std::hint::black_box(&reduced_bench);
                t_diff_ns += td.elapsed().as_nanos() as u64;

                let te = std::time::Instant::now();
                let red_payload = d53::encode_payload(&reduced_bench);
                let _ = std::hint::black_box(d53::decode_snapshot_payload(0, &red_payload));
                t_newenc_ns += te.elapsed().as_nanos() as u64;

                t_new_ns += tb.elapsed().as_nanos() as u64;

                // OPTIMIZED FLOOR: no per-frame map build. Keep the baseline as a
                // persistent HashMap (built once here) and do a single linear pass
                // over the current row Vecs with a hashmap lookup per row + a reused
                // per-key generation stamp for tombstone detection. Measured only on
                // the two dominant families (opinions + state_events ~= 99.5% of
                // rows) to establish the realistic lower bound for the diff cost.
                if opin_base.is_empty() && !baseline.opinions.is_empty() {
                    for (k, r) in &baseline.opinions {
                        opin_base.insert(*k, r.opinion);
                        opin_gen.insert(*k, 0u32);
                    }
                    for (k, r) in &baseline.state_events {
                        se_base.insert(*k, r.value_raw);
                        se_gen.insert(*k, 0u32);
                    }
                }
                let gen = (partial_frames + 1) as u32;
                let tf = std::time::Instant::now();
                let mut changed = 0u64;
                for row in &snap.opinions {
                    let k = (row.src_tag_db_id, row.target_tag_db_id);
                    match opin_base.get(&k) {
                        Some(v) if *v == row.opinion => {}
                        _ => changed += 1,
                    }
                    if let Some(g) = opin_gen.get_mut(&k) {
                        *g = gen;
                    }
                }
                for row in &snap.state_events {
                    let k = (
                        row.owner_tag_db_id,
                        row.target_tag_db_id,
                        row.event_kind,
                        row.source_kind,
                        row.object_hash,
                    );
                    match se_base.get(&k) {
                        Some(v) if *v == row.value_raw => {}
                        _ => changed += 1,
                    }
                    if let Some(g) = se_gen.get_mut(&k) {
                        *g = gen;
                    }
                }
                let mut tombs = 0u64;
                for g in opin_gen.values() {
                    if *g != gen {
                        tombs += 1;
                    }
                }
                for g in se_gen.values() {
                    if *g != gen {
                        tombs += 1;
                    }
                }
                std::hint::black_box((changed, tombs));
                t_opt_ns += tf.elapsed().as_nanos() as u64;

                bench_frames += 1;
            }

            let cur = maps_from(&snap);
            if !seen_baseline {
                // First frame: emit full and seed the baseline from it regardless of
                // the partial bit (mirrors the collector). Deep-war windows are
                // all-partial, so a clean-only gate would never establish a baseline.
                after_bytes += plen as u64;
                reduced_rows += snap.row_count() as u64;
                baseline = cur;
                seen_baseline = true;
            } else {
                let reduced = reduced_snapshot(&baseline, &cur, &snap);
                let bytes = d53::encode_payload(&reduced).len() as u64;
                if partial_frames < 2 || partial_frames % 60 == 0 {
                    eprintln!(
                        "success#{} ({} KB): reduced opin={} se={} country={} op={} crypto={} wars={} subj={} relflag={} agency={} cluster={} (cur opin={} se={})",
                        partial_frames + 1, bytes / 1024,
                        reduced.opinions.len(), reduced.state_events.len(), reduced.countries.len(), reduced.operatives.len(), reduced.cryptology.len(),
                        reduced.wars.len(), reduced.subjects.len(), reduced.relation_flags.len(), reduced.agencies.len(), reduced.agency_capacity_clusters.len(),
                        cur.opinions.len(), cur.state_events.len(),
                    );
                }
                after_bytes += bytes;
                reduced_rows += reduced.row_count() as u64;
                if is_partial {
                    partial_frames += 1;
                } else {
                    clean_frames += 1;
                }
                if !is_partial {
                    baseline = cur; // advance only on clean
                }
            }
        } else if !skip(&mut r, plen) {
            break;
        }
    }

    let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
    println!("== d53 partial-delta byte projection ==");
    println!("file: {path}");
    let success = frames - failure_frames;
    println!(
        "d53 DOMAIN_DATA frames: {frames} (success={success}, failure/empty={failure_frames}; of success: delta-partial={partial_frames}, clean={clean_frames}, +1 baseline)"
    );
    println!(
        "BEFORE (full re-emit every success frame): {:.3} MB total, {:.3} MB/success-frame, {} rows/success-frame(avg)",
        mb(before_bytes),
        mb(before_bytes) / success.max(1) as f64,
        full_rows / success.max(1),
    );
    println!(
        "AFTER  (delta on partial frames): {:.4} MB total, {:.5} MB/success-frame, {} rows/success-frame(avg)",
        mb(after_bytes),
        mb(after_bytes) / success.max(1) as f64,
        reduced_rows / success.max(1),
    );
    let reduction = if before_bytes > 0 {
        100.0 * (before_bytes - after_bytes) as f64 / before_bytes as f64
    } else {
        0.0
    };
    println!(
        "REDUCTION: {:.3}%  ({:.3} MB -> {:.4} MB)  factor {:.1}x",
        reduction,
        mb(before_bytes),
        mb(after_bytes),
        before_bytes as f64 / after_bytes.max(1) as f64
    );
    if bench_frames > 0 {
        let old_ms = t_old_ns as f64 / bench_frames as f64 / 1e6;
        let new_ms = t_new_ns as f64 / bench_frames as f64 / 1e6;
        let f = bench_frames as f64;
        println!(
            "CPU/frame payload work ({} delta frames):\n  OLD full encode+decode-gate = {:.3} ms\n  NEW build={:.3} + diff={:.3} + reduced encode+gate={:.3} = {:.3} ms\n  NET {:+.3} ms/frame ({:.1}% of old)",
            bench_frames,
            old_ms,
            t_build_ns as f64 / f / 1e6,
            t_diff_ns as f64 / f / 1e6,
            t_newenc_ns as f64 / f / 1e6,
            new_ms,
            new_ms - old_ms,
            100.0 * new_ms / old_ms,
        );
        let opt_ms = t_opt_ns as f64 / f / 1e6;
        println!(
            "  OPTIMIZED floor (HashMap, no rebuild, single-pass diff of opinions+state_events only) = {:.3} ms  => NET vs old {:+.3} ms/frame",
            opt_ms,
            opt_ms - old_ms,
        );
    }
}

fn read_exact_or_eof<R: Read>(r: &mut R, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => return Ok(false),
            Ok(n) => filled += n,
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> bool {
    matches!(read_exact_or_eof(r, buf), Ok(true))
}

fn skip<R: Read>(r: &mut R, mut n: usize) -> bool {
    let mut scratch = [0u8; 1 << 16];
    while n > 0 {
        let want = n.min(scratch.len());
        match read_exact_or_eof(r, &mut scratch[..want]) {
            Ok(true) => n -= want,
            _ => return false,
        }
    }
    true
}
