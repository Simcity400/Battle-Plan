//! Measurement harness (lane/d53-perfield): decode a bounded sample of a REAL
//! collector stream, filter domain-53 DOMAIN_DATA frames, and quantify:
//!   1. Per-field change statistics for the row-level-delta base kinds (the
//!      TASK 2 per-field-delta value estimate). For each changed row we tally,
//!      per non-key field, whether it actually changed vs the tracked prior
//!      value, and the wire bytes a per-field encoding would SAVE (the sum of
//!      unchanged non-key field widths).
//!   2. The FULL-EMIT residual subdomains carried in trailers
//!      (operation_anchors, intel_map, operation_instances, incoming_actions,
//!      intel_network_graphs, factions) — bytes/frame and how often the row SET
//!      actually changes (the TASK 3 row-level-delta value estimate).
//!
//! Read-only, streaming, never copies the whole file. Usage:
//!   d53_field_delta_measure <stream.bin> [max_d53_frames]
//!
//! This is an analysis tool, not production code; it is compiled by the lane
//! crate but never linked into the collector/host.

use std::collections::BTreeMap;
use std::io::{BufReader, Read};

use diplomacy_intel_operatives_live::compact as d53;

const HDR: usize = compact_contract::COMPACT_RECORD_HEADER_LEN; // 32
const DATA: u16 = compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA; // 3
const D53: u16 = 53;

// Full on-wire row width per base kind (bytes), matching encode_payload.
fn full_row_width(kind: u16) -> usize {
    match kind {
        d53::KIND_COUNTRY_DIPLOMACY => 24,
        d53::KIND_WAR_RELATION => 32,
        d53::KIND_OPINION => 16,
        d53::KIND_RELATION_FLAG => 16,
        d53::KIND_SUBJECT => 24,
        d53::KIND_AGENCY => 48,
        d53::KIND_AGENCY_CAPACITY_CLUSTER => 24,
        d53::KIND_OPERATIVE => 40,
        d53::KIND_CRYPTOLOGY => 32,
        d53::KIND_OPERATION_ANCHOR => 32,
        d53::KIND_INTEL_MAP => 32,
        d53::KIND_COMPACT_STATE_EVENT => 32,
        _ => 0,
    }
}

#[derive(Default, Clone)]
struct KindStat {
    changed_rows: u64,  // rows re-emitted (present, non-tombstone, differ from prior)
    baseline_rows: u64, // rows with no prior (first sight) — per-field can't help
    tombstones: u64,    // deletions
    full_bytes: u64,    // full-row wire bytes spent on changed_rows
    field_names: Vec<&'static str>,
    field_widths: Vec<usize>,
    field_changed: Vec<u64>, // per changed row, count of changes per field
    saved_bytes: u64,        // sum over changed rows of unchanged-non-key-field widths
}

impl KindStat {
    fn ensure(&mut self, names: &[(&'static str, usize)]) {
        if self.field_names.is_empty() {
            for (n, w) in names {
                self.field_names.push(n);
                self.field_widths.push(*w);
                self.field_changed.push(0);
            }
        }
    }
}

fn key_deleted(v: u32) -> bool {
    v & d53::ROW_ID_DELETED != 0
}
fn key_undelete(v: u32) -> u32 {
    v & !d53::ROW_ID_DELETED
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: d53_field_delta_measure <stream.bin> [max_d53_frames]");
        std::process::exit(2);
    }
    let path = &args[1];
    let max_frames: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(400);

    let f = std::fs::File::open(path).expect("open stream");
    let mut r = BufReader::with_capacity(1 << 20, f);

    let mut countries: BTreeMap<u32, d53::CountryDiplomacyRow> = BTreeMap::new();
    let mut wars: BTreeMap<(u32, u32), d53::WarRelationRow> = BTreeMap::new();
    let mut opinions: BTreeMap<(u32, u32), d53::OpinionRow> = BTreeMap::new();
    let mut relflags: BTreeMap<(u32, u32, u16), d53::RelationFlagRow> = BTreeMap::new();
    let mut subjects: BTreeMap<(u32, u32), d53::SubjectRow> = BTreeMap::new();
    let mut agencies: BTreeMap<u32, d53::AgencyRow> = BTreeMap::new();
    let mut clusters: BTreeMap<u32, d53::AgencyCapacityClusterRow> = BTreeMap::new();
    let mut operatives: BTreeMap<u32, d53::OperativeRow> = BTreeMap::new();
    let mut cryptology: BTreeMap<(u32, u32), d53::CryptologyRow> = BTreeMap::new();
    let mut state_events: BTreeMap<(u32, u32, u16, u16, u32), d53::StateEventRow> = BTreeMap::new();

    let mut stat: BTreeMap<u16, KindStat> = BTreeMap::new();

    let mut prev_op_instances: Vec<d53::OperationInstanceRow> = Vec::new();
    let mut prev_incoming: Vec<d53::IncomingActionQueueRow> = Vec::new();
    let mut prev_ingr: Vec<d53::IntelNetworkGraphRow> = Vec::new();
    let mut prev_factions: Vec<d53::FactionRow> = Vec::new();
    let mut prev_anchors: Vec<d53::OperationAnchorRow> = Vec::new();
    let mut prev_intelmap: Vec<d53::IntelMapRow> = Vec::new();

    #[derive(Default)]
    struct SubStat {
        frames_present: u64,
        rows_total: u64,
        full_bytes_total: u64,
        frames_changed: u64,
        deltarows_total: u64,
    }
    let mut sub: BTreeMap<&'static str, SubStat> = BTreeMap::new();

    let mut hdr = [0u8; HDR];
    let mut d53_frames = 0u64;
    let mut d53_data_bytes = 0u64;
    let mut delta_frames = 0u64;
    let mut baseline_frames = 0u64;

    loop {
        if d53_frames >= max_frames {
            break;
        }
        match read_exact_or_eof(&mut r, &mut hdr) {
            ReadResult::Eof => break,
            ReadResult::Short => break,
            ReadResult::Ok => {}
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
            d53_frames += 1;
            d53_data_bytes += plen as u64;
            let status = header.record_flags;
            let is_delta = status & compact_contract::COMPACT_STATUS_DELTA_DATA != 0;
            if is_delta {
                delta_frames += 1;
            } else {
                baseline_frames += 1;
            }
            let snap = match d53::decode_snapshot_payload(0, &payload) {
                Ok(s) => s,
                Err(errs) => {
                    eprintln!("decode error frame {d53_frames}: {:?}", errs.first());
                    continue;
                }
            };

            for row in &snap.countries {
                let s = stat.entry(d53::KIND_COUNTRY_DIPLOMACY).or_default();
                s.ensure(&[("faction_group", 4), ("at_war_count", 4), ("casualties", 8)]);
                if key_deleted(row.tag_db_id) {
                    s.tombstones += 1;
                    countries.remove(&key_undelete(row.tag_db_id));
                    continue;
                }
                match countries.get(&row.tag_db_id) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_COUNTRY_DIPLOMACY) as u64;
                        acc_fields(
                            s,
                            &[
                                old.faction_group != row.faction_group,
                                old.at_war_count != row.at_war_count,
                                old.casualties != row.casualties,
                            ],
                        );
                    }
                    None => s.baseline_rows += 1,
                }
                countries.insert(row.tag_db_id, row.clone());
            }
            for row in &snap.wars {
                let s = stat.entry(d53::KIND_WAR_RELATION).or_default();
                s.ensure(&[("first_casualties", 8), ("second_casualties", 8)]);
                if key_deleted(row.first_tag_db_id) {
                    s.tombstones += 1;
                    wars.remove(&war_key(
                        key_undelete(row.first_tag_db_id),
                        row.second_tag_db_id,
                    ));
                    continue;
                }
                let k = war_key(row.first_tag_db_id, row.second_tag_db_id);
                match wars.get(&k) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_WAR_RELATION) as u64;
                        acc_fields(
                            s,
                            &[
                                old.first_casualties != row.first_casualties,
                                old.second_casualties != row.second_casualties,
                            ],
                        );
                    }
                    None => s.baseline_rows += 1,
                }
                wars.insert(k, row.clone());
            }
            for row in &snap.opinions {
                let s = stat.entry(d53::KIND_OPINION).or_default();
                s.ensure(&[("opinion", 4)]);
                if key_deleted(row.src_tag_db_id) {
                    s.tombstones += 1;
                    opinions.remove(&(key_undelete(row.src_tag_db_id), row.target_tag_db_id));
                    continue;
                }
                let k = (row.src_tag_db_id, row.target_tag_db_id);
                match opinions.get(&k) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_OPINION) as u64;
                        acc_fields(s, &[old.opinion != row.opinion]);
                    }
                    None => s.baseline_rows += 1,
                }
                opinions.insert(k, row.clone());
            }
            for row in &snap.relation_flags {
                let s = stat.entry(d53::KIND_RELATION_FLAG).or_default();
                s.ensure(&[("directed", 0)]);
                if key_deleted(row.src_tag_db_id) {
                    s.tombstones += 1;
                    relflags.remove(&(
                        key_undelete(row.src_tag_db_id),
                        row.target_tag_db_id,
                        row.flag_kind,
                    ));
                    continue;
                }
                let k = (row.src_tag_db_id, row.target_tag_db_id, row.flag_kind);
                match relflags.get(&k) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_RELATION_FLAG) as u64;
                        acc_fields(s, &[old.directed != row.directed]);
                    }
                    None => s.baseline_rows += 1,
                }
                relflags.insert(k, row.clone());
            }
            for row in &snap.subjects {
                let s = stat.entry(d53::KIND_SUBJECT).or_default();
                s.ensure(&[
                    ("autonomy_state_hash", 4),
                    ("autonomy_ratio_raw", 4),
                    ("start_date_raw", 4),
                ]);
                if key_deleted(row.overlord_tag_db_id) {
                    s.tombstones += 1;
                    subjects.remove(&(key_undelete(row.overlord_tag_db_id), row.subject_tag_db_id));
                    continue;
                }
                let k = (row.overlord_tag_db_id, row.subject_tag_db_id);
                match subjects.get(&k) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_SUBJECT) as u64;
                        acc_fields(
                            s,
                            &[
                                old.autonomy_state_hash != row.autonomy_state_hash,
                                old.autonomy_ratio_raw != row.autonomy_ratio_raw,
                                old.start_date_raw != row.start_date_raw,
                            ],
                        );
                    }
                    None => s.baseline_rows += 1,
                }
                subjects.insert(k, row.clone());
            }
            for row in &snap.agencies {
                let s = stat.entry(d53::KIND_AGENCY).or_default();
                s.ensure(&[
                    ("max_operatives", 4),
                    ("usable_slots", 4),
                    ("operative_count", 4),
                    ("cryptology_target_count", 4),
                    ("defense_raw", 4),
                    ("upgrade_progress_raw", 4),
                    ("name_hash", 8),
                    ("icon_hash", 8),
                ]);
                if key_deleted(row.owner_tag_db_id) {
                    s.tombstones += 1;
                    agencies.remove(&key_undelete(row.owner_tag_db_id));
                    continue;
                }
                match agencies.get(&row.owner_tag_db_id) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_AGENCY) as u64;
                        acc_fields(
                            s,
                            &[
                                old.max_operatives != row.max_operatives,
                                old.usable_slots != row.usable_slots,
                                old.operative_count != row.operative_count,
                                old.cryptology_target_count != row.cryptology_target_count,
                                old.defense_raw != row.defense_raw,
                                old.upgrade_progress_raw != row.upgrade_progress_raw,
                                old.name_hash != row.name_hash,
                                old.icon_hash != row.icon_hash,
                            ],
                        );
                    }
                    None => s.baseline_rows += 1,
                }
                agencies.insert(row.owner_tag_db_id, row.clone());
            }
            for row in &snap.agency_capacity_clusters {
                let s = stat.entry(d53::KIND_AGENCY_CAPACITY_CLUSTER).or_default();
                s.ensure(&[
                    ("value_0xe4", 4),
                    ("value_0xf0", 4),
                    ("value_0xf4", 4),
                    ("proof_hash", 4),
                ]);
                if key_deleted(row.owner_tag_db_id) {
                    s.tombstones += 1;
                    clusters.remove(&key_undelete(row.owner_tag_db_id));
                    continue;
                }
                match clusters.get(&row.owner_tag_db_id) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_AGENCY_CAPACITY_CLUSTER) as u64;
                        acc_fields(
                            s,
                            &[
                                old.value_0xe4 != row.value_0xe4,
                                old.value_0xf0 != row.value_0xf0,
                                old.value_0xf4 != row.value_0xf4,
                                old.proof_hash != row.proof_hash,
                            ],
                        );
                    }
                    None => s.baseline_rows += 1,
                }
                clusters.insert(row.owner_tag_db_id, row.clone());
            }
            for row in &snap.operatives {
                let s = stat.entry(d53::KIND_OPERATIVE).or_default();
                s.ensure(&[
                    ("owner_tag_db_id", 4),
                    ("skill", 4),
                    ("experience_raw", 8),
                    ("name_hash", 8),
                    ("state_hash", 4),
                    ("lifecycle_flag", 4),
                    ("detected_captured", 0),
                ]);
                if key_deleted(row.op_id) {
                    s.tombstones += 1;
                    operatives.remove(&key_undelete(row.op_id));
                    continue;
                }
                match operatives.get(&row.op_id) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_OPERATIVE) as u64;
                        acc_fields(
                            s,
                            &[
                                old.owner_tag_db_id != row.owner_tag_db_id,
                                old.skill != row.skill,
                                old.experience_raw != row.experience_raw,
                                old.name_hash != row.name_hash,
                                old.state_hash != row.state_hash,
                                old.lifecycle_flag != row.lifecycle_flag,
                                old.detected != row.detected || old.captured != row.captured,
                            ],
                        );
                    }
                    None => s.baseline_rows += 1,
                }
                operatives.insert(row.op_id, row.clone());
            }
            for row in &snap.cryptology {
                let s = stat.entry(d53::KIND_CRYPTOLOGY).or_default();
                s.ensure(&[("amount_raw", 8), ("decrypting", 0)]);
                if key_deleted(row.owner_tag_db_id) {
                    s.tombstones += 1;
                    cryptology.remove(&(key_undelete(row.owner_tag_db_id), row.target_tag_db_id));
                    continue;
                }
                let k = (row.owner_tag_db_id, row.target_tag_db_id);
                match cryptology.get(&k) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_CRYPTOLOGY) as u64;
                        acc_fields(
                            s,
                            &[
                                old.amount_raw != row.amount_raw,
                                old.decrypting != row.decrypting,
                            ],
                        );
                    }
                    None => s.baseline_rows += 1,
                }
                cryptology.insert(k, row.clone());
            }
            for row in &snap.state_events {
                let s = stat.entry(d53::KIND_COMPACT_STATE_EVENT).or_default();
                s.ensure(&[("value_hash", 4), ("value_raw", 8)]);
                if key_deleted(row.owner_tag_db_id) {
                    s.tombstones += 1;
                    state_events.remove(&(
                        key_undelete(row.owner_tag_db_id),
                        row.target_tag_db_id.unwrap_or(0),
                        row.event_kind,
                        row.source_kind,
                        row.object_hash,
                    ));
                    continue;
                }
                let k = (
                    row.owner_tag_db_id,
                    row.target_tag_db_id.unwrap_or(0),
                    row.event_kind,
                    row.source_kind,
                    row.object_hash,
                );
                match state_events.get(&k) {
                    Some(old) => {
                        s.changed_rows += 1;
                        s.full_bytes += full_row_width(d53::KIND_COMPACT_STATE_EVENT) as u64;
                        acc_fields(
                            s,
                            &[
                                old.value_hash != row.value_hash,
                                old.value_raw != row.value_raw,
                            ],
                        );
                    }
                    None => s.baseline_rows += 1,
                }
                state_events.insert(k, row.clone());
            }

            {
                let e = sub.entry("operation_instances").or_default();
                if !snap.operation_instances.is_empty() {
                    e.frames_present += 1;
                    e.rows_total += snap.operation_instances.len() as u64;
                    e.full_bytes_total += (snap.operation_instances.len() * 12) as u64;
                    let (changed, dr) = diff_vec(&prev_op_instances, &snap.operation_instances);
                    if changed {
                        e.frames_changed += 1;
                    }
                    e.deltarows_total += dr;
                }
                prev_op_instances = snap.operation_instances.clone();
            }
            {
                let e = sub.entry("incoming_actions").or_default();
                if !snap.incoming_actions.is_empty() {
                    e.frames_present += 1;
                    e.rows_total += snap.incoming_actions.len() as u64;
                    e.full_bytes_total += (snap.incoming_actions.len() * 24) as u64;
                    let (changed, dr) = diff_vec(&prev_incoming, &snap.incoming_actions);
                    if changed {
                        e.frames_changed += 1;
                    }
                    e.deltarows_total += dr;
                }
                prev_incoming = snap.incoming_actions.clone();
            }
            {
                let e = sub.entry("intel_network_graphs").or_default();
                if !snap.intel_network_graphs.is_empty() {
                    e.frames_present += 1;
                    e.rows_total += snap.intel_network_graphs.len() as u64;
                    let bytes: usize = snap
                        .intel_network_graphs
                        .iter()
                        .map(|r| 16 + r.state_ids.len() * 4)
                        .sum();
                    e.full_bytes_total += bytes as u64;
                    let (changed, dr) = diff_vec(&prev_ingr, &snap.intel_network_graphs);
                    if changed {
                        e.frames_changed += 1;
                    }
                    e.deltarows_total += dr;
                }
                prev_ingr = snap.intel_network_graphs.clone();
            }
            {
                let e = sub.entry("factions").or_default();
                if !snap.factions.is_empty() {
                    e.frames_present += 1;
                    e.rows_total += snap.factions.len() as u64;
                    let bytes: usize = snap
                        .factions
                        .iter()
                        .map(|r| 24 + r.member_tag_db_ids.len() * 4)
                        .sum();
                    e.full_bytes_total += bytes as u64;
                    let (changed, dr) = diff_vec(&prev_factions, &snap.factions);
                    if changed {
                        e.frames_changed += 1;
                    }
                    e.deltarows_total += dr;
                }
                prev_factions = snap.factions.clone();
            }
            {
                let e = sub.entry("operation_anchors").or_default();
                if !snap.operation_anchors.is_empty() {
                    e.frames_present += 1;
                    e.rows_total += snap.operation_anchors.len() as u64;
                    e.full_bytes_total += (snap.operation_anchors.len() * 32) as u64;
                    let (changed, dr) = diff_vec(&prev_anchors, &snap.operation_anchors);
                    if changed {
                        e.frames_changed += 1;
                    }
                    e.deltarows_total += dr;
                }
                prev_anchors = snap.operation_anchors.clone();
            }
            {
                let e = sub.entry("intel_map").or_default();
                if !snap.intel_map.is_empty() {
                    e.frames_present += 1;
                    e.rows_total += snap.intel_map.len() as u64;
                    e.full_bytes_total += (snap.intel_map.len() * 32) as u64;
                    let (changed, dr) = diff_vec(&prev_intelmap, &snap.intel_map);
                    if changed {
                        e.frames_changed += 1;
                    }
                    e.deltarows_total += dr;
                }
                prev_intelmap = snap.intel_map.clone();
            }
        } else if !skip(&mut r, plen) {
            break;
        }
    }

    println!("== d53 stream sample ==");
    println!("file: {path}");
    println!(
        "d53 DOMAIN_DATA frames: {d53_frames} (baseline={baseline_frames} delta={delta_frames}), d53 payload bytes: {d53_data_bytes}"
    );
    println!();
    println!("== TASK 2: per-field-delta value on row-level-delta base kinds ==");
    println!(
        "(saved = unchanged non-key field bytes a per-field encoder could omit on CHANGED rows)"
    );
    let mut total_full = 0u64;
    let mut total_saved = 0u64;
    for (kind, s) in &stat {
        let name = kind_name(*kind);
        let pct = if s.full_bytes > 0 {
            100.0 * s.saved_bytes as f64 / s.full_bytes as f64
        } else {
            0.0
        };
        total_full += s.full_bytes;
        total_saved += s.saved_bytes;
        println!(
            "  {name:>24}: changed={:>8} baseline={:>7} tomb={:>6} full_bytes={:>11} saved_bytes={:>11} ({pct:5.1}%)",
            s.changed_rows, s.baseline_rows, s.tombstones, s.full_bytes, s.saved_bytes
        );
        for i in 0..s.field_names.len() {
            let cr = if s.changed_rows > 0 {
                100.0 * s.field_changed[i] as f64 / s.changed_rows as f64
            } else {
                0.0
            };
            println!(
                "        {:>26} w={:<2} changed_in {:>6.1}% of changed rows",
                s.field_names[i], s.field_widths[i], cr
            );
        }
    }
    let tot_pct = if total_full > 0 {
        100.0 * total_saved as f64 / total_full as f64
    } else {
        0.0
    };
    println!(
        "  TOTAL base changed-row full_bytes={total_full} saved_bytes={total_saved} ({tot_pct:.1}% of changed-row bytes)"
    );
    println!(
        "  (saved as share of ALL d53 payload bytes: {:.2}%)",
        if d53_data_bytes > 0 {
            100.0 * total_saved as f64 / d53_data_bytes as f64
        } else {
            0.0
        }
    );
    println!();
    println!("== TASK 3: full-emit residual subdomains (re-cloned every delta frame) ==");
    for (name, e) in &sub {
        let delta_bytes_est = if e.rows_total > 0 {
            (e.full_bytes_total as f64) * (e.deltarows_total as f64) / (e.rows_total as f64)
        } else {
            0.0
        };
        let save = e.full_bytes_total as f64 - delta_bytes_est;
        let pct = if e.full_bytes_total > 0 {
            100.0 * save / e.full_bytes_total as f64
        } else {
            0.0
        };
        println!(
            "  {name:>22}: frames_present={:>5} frames_changed={:>5} rows_total={:>8} full_bytes={:>10} deltarows={:>8} => row-delta saves ~{:.0} bytes ({pct:.1}%)",
            e.frames_present, e.frames_changed, e.rows_total, e.full_bytes_total, e.deltarows_total, save
        );
    }
}

fn kind_name(kind: u16) -> &'static str {
    match kind {
        d53::KIND_COUNTRY_DIPLOMACY => "country_diplomacy",
        d53::KIND_WAR_RELATION => "war_relation",
        d53::KIND_OPINION => "opinion",
        d53::KIND_RELATION_FLAG => "relation_flag",
        d53::KIND_SUBJECT => "subject",
        d53::KIND_AGENCY => "agency",
        d53::KIND_AGENCY_CAPACITY_CLUSTER => "agency_capacity_cluster",
        d53::KIND_OPERATIVE => "operative",
        d53::KIND_CRYPTOLOGY => "cryptology",
        d53::KIND_COMPACT_STATE_EVENT => "state_event",
        _ => "?",
    }
}

fn acc_fields(s: &mut KindStat, changed: &[bool]) {
    debug_assert_eq!(changed.len(), s.field_widths.len());
    for (i, &ch) in changed.iter().enumerate() {
        if ch {
            s.field_changed[i] += 1;
        } else {
            s.saved_bytes += s.field_widths[i] as u64;
        }
    }
}

fn war_key(a: u32, b: u32) -> (u32, u32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Returns (set_changed, delta_row_count) between two full-emit vecs.
fn diff_vec<T: Clone + PartialEq>(old: &[T], new: &[T]) -> (bool, u64) {
    if old == new {
        return (false, 0);
    }
    let mut used = vec![false; old.len()];
    let mut added = 0u64;
    for n in new {
        let mut found = false;
        for (i, o) in old.iter().enumerate() {
            if !used[i] && o == n {
                used[i] = true;
                found = true;
                break;
            }
        }
        if !found {
            added += 1;
        }
    }
    let removed = used.iter().filter(|u| !**u).count() as u64;
    (true, added + removed)
}

enum ReadResult {
    Ok,
    Eof,
    Short,
}

fn read_exact_or_eof<R: Read>(r: &mut R, buf: &mut [u8]) -> ReadResult {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => {
                return if filled == 0 {
                    ReadResult::Eof
                } else {
                    ReadResult::Short
                };
            }
            Ok(n) => filled += n,
            Err(_) => return ReadResult::Short,
        }
    }
    ReadResult::Ok
}

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> bool {
    matches!(read_exact_or_eof(r, buf), ReadResult::Ok)
}

fn skip<R: Read>(r: &mut R, mut n: usize) -> bool {
    let mut scratch = [0u8; 1 << 16];
    while n > 0 {
        let want = n.min(scratch.len());
        match read_exact_or_eof(r, &mut scratch[..want]) {
            ReadResult::Ok => n -= want,
            _ => return false,
        }
    }
    true
}
