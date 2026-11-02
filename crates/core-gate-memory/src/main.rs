mod cpu;
mod detail;
mod encoding;
mod gate;
mod gated;
mod grouping;
mod history;
mod keyset;
mod manifest;
mod memory;
mod proof;
mod reads;
mod readset;
mod report;
mod reverse;
#[cfg(test)]
mod synthetic_tests;

use detail::Detail;
use gate::{Admission, Lost, SegmentReport};
use manifest::{Manifest, invalid};
use replay::{columnar_frames::Recording, container::Replay};
use replay_data::{
    codec::Decoder,
    identity::{Identity, IdentityCodec},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::atomic::Ordering,
};

struct Args {
    recording: PathBuf,
    columnar: PathBuf,
    manifest: PathBuf,
    out: PathBuf,
    threads: usize,
    projects_scoped: Option<bool>,
}

impl Args {
    fn read() -> io::Result<Self> {
        let mut args = std::env::args_os().skip(1);
        let mut values = BTreeMap::new();
        while let Some(name) = args.next() {
            if name == "--help" {
                println!(
                    "core-gate-memory --recording <.battleplan> --columnar <.h4col> --manifest <toml> --out <new empty folder> --threads N [--projects-scoped true|false]"
                );
                std::process::exit(0);
            }
            let name = name
                .into_string()
                .map_err(|_| invalid("Non-UTF-8 option"))?;
            if !matches!(
                name.as_str(),
                "--recording"
                    | "--columnar"
                    | "--manifest"
                    | "--out"
                    | "--threads"
                    | "--projects-scoped"
            ) {
                return Err(invalid(format!("Unknown option {name}")));
            }
            let value = args
                .next()
                .ok_or_else(|| invalid(format!("Missing value for {name}")))?;
            if values.insert(name.clone(), value).is_some() {
                return Err(invalid(format!("Repeated option {name}")));
            }
        }
        let projects_scoped = values
            .remove("--projects-scoped")
            .map(|value| match value.to_str() {
                Some("true") => Ok(true),
                Some("false") => Ok(false),
                _ => Err(invalid(
                    "--projects-scoped requires the content-derived true or false value",
                )),
            })
            .transpose()?;
        let mut take = |name: &str| {
            values
                .remove(name)
                .ok_or_else(|| invalid(format!("Required option {name}")))
        };
        let recording = take("--recording")?.into();
        let columnar = take("--columnar")?.into();
        let manifest = take("--manifest")?.into();
        let out = take("--out")?.into();
        let threads = take("--threads")?
            .to_str()
            .and_then(|s| s.parse().ok())
            .filter(|&n: &usize| n > 0)
            .ok_or_else(|| invalid("--threads must be a positive integer"))?;
        Ok(Self {
            recording,
            columnar,
            manifest,
            out,
            threads,
            projects_scoped,
        })
    }
}

#[derive(Serialize)]
struct Tool {
    wall_seconds: f64,
    process_cpu_seconds: f64,
    peak_commit_bytes: usize,
    peak_working_set_bytes: usize,
    under_three_gb: bool,
    requested_threads: usize,
    maximum_worker_threads: usize,
    maximum_tool_threads_including_main: usize,
    peak_active_readers: usize,
    thread_cycle_hz: f64,
    cpu_clock: &'static str,
}

#[derive(Serialize)]
struct Lane {
    family: String,
    spool: String,
    records: u64,
    records_per_hour: f64,
    bytes: u64,
    zstd3_bytes: u64,
    read_path_cycles: u64,
    read_path_us_per_hour: f64,
    projected_read_path_seconds: f64,
    measurement_scope: String,
    active_hours: BTreeMap<String, u64>,
    segments: Detail<SegmentDelivery>,
}

#[derive(serde::Deserialize, Serialize)]
struct SegmentDelivery {
    segment: usize,
    hours: u64,
    delivery: encoding::Delivery,
    read_path_us_per_hour: f64,
}

#[derive(Serialize)]
struct Report {
    version: u32,
    recording: PathBuf,
    columnar: PathBuf,
    metadata: serde_json::Value,
    manifest: manifest::Manifest,
    manifest_provenance: serde_json::Value,
    definitions: serde_json::Value,
    recorded_hours: u64,
    measured_hours: u64,
    segments: Detail<SegmentReport>,
    excluded_damage: Detail<Lost>,
    damage_count: usize,
    totals: serde_json::Value,
    groupings: Vec<grouping::Scenario>,
    group_walks: Vec<grouping::Walk>,
    shared_walk: Option<usize>,
    union_walk: Option<usize>,
    ordered_lanes: Vec<Lane>,
    verdict: String,
}

fn identity(identity: &Identity) -> serde_json::Value {
    serde_json::json!({
        "game_version": identity.game_version, "language": identity.language,
        "vanilla_hash": identity.vanilla_hash,
        "dlc": identity.dlc.iter().map(|d| serde_json::json!({"id":d.id,"hash":d.hash,"declared_checksum":d.declared_checksum})).collect::<Vec<_>>(),
        "mods_in_recorded_order": identity.mods.iter().map(|m| serde_json::json!({"id":m.id,"display_name":m.display_name,"declared_version":m.declared_version,"hash":m.hash})).collect::<Vec<_>>(),
        "verification": format!("{:?}",identity.verification), "coverage_policy": identity.coverage_policy,
    })
}

fn metadata(args: &Args, columnar: &Recording) -> io::Result<serde_json::Value> {
    let replay = Replay::open_path(&args.recording)?;
    if replay.campaign_id != columnar.campaign() {
        return Err(invalid(
            "Row and columnar recordings name different campaigns",
        ));
    }
    let mut decoder = Decoder::new(&columnar.root_identity().content);
    let content = Identity::decode(&mut decoder)?;
    decoder.finish()?;
    if content != replay.identity {
        return Err(invalid(
            "Columnar content identity differs from the row recording",
        ));
    }
    Ok(serde_json::json!({
        "campaign_id_bytes": replay.campaign_id, "format_version": replay.format.version(),
        "content": identity(&replay.identity),
        "sessions": replay.sessions.iter().map(|s| serde_json::json!({"session":s.id,"content":identity(&s.identity)})).collect::<Vec<_>>(),
        "warnings": replay.warnings, "damaged_parts": replay.damaged_parts,
        "capture_mode": format!("{:?}",replay.capture_mode),
    }))
}

fn run(args: Args) -> io::Result<()> {
    let clock = cpu::Clock::start()?;
    let process_start = cpu::process_ns()?;
    let resolved = Manifest::read(&args.manifest)?;
    let shipped: Manifest = toml::from_str(include_str!("../manifest.toml"))
        .map_err(|e| invalid(format!("Invalid built-in family list: {e}")))?;
    let names: std::collections::BTreeSet<_> =
        resolved.manifest.families.iter().map(|f| &f.name).collect();
    let expected: std::collections::BTreeSet<_> =
        shipped.families.iter().map(|f| &f.name).collect();
    if names != expected {
        return Err(invalid(
            "Gate 1 requires exactly the 68 c13 measured families",
        ));
    }
    let shared_names: std::collections::BTreeSet<_> =
        resolved.manifest.shared.iter().map(|o| &o.name).collect();
    let expected_shared: std::collections::BTreeSet<_> =
        shipped.shared.iter().map(|o| &o.name).collect();
    if shared_names != expected_shared {
        return Err(invalid("Gate 1 requires the shipped shared owners"));
    }
    drop(expected_shared);
    drop(shared_names);
    drop(expected);
    drop(names);
    drop(shipped);
    if args.out.exists() {
        if !args.out.is_dir() || std::fs::read_dir(&args.out)?.next().is_some() {
            return Err(invalid("--out must be a new empty directory"));
        }
    } else {
        std::fs::create_dir_all(&args.out)?;
    }
    let mut recording = Recording::open_path(&args.columnar, gate::limits())?;
    recording.select(&resolved.tables);
    let metadata = metadata(&args, &recording)?;
    let mut layout = memory::Layout::new(&resolved.tables, resolved.relations.len());
    let previous = manifest::union(resolved.families.iter().flatten().filter(|s| s.previous));
    layout.hold_previous(&previous);
    let admission = Admission::new();
    let segments = Detail::<SegmentReport>::new(
        &args.out.join("segment-reports"),
        recording.segments().len(),
    )?;
    let excluded_damage = Detail::<Lost>::new(
        &args.out.join("damage-reports"),
        recording.segments().len() + recording.damage().len(),
    )?;
    let format = replay_data::format::FormatVersion::read(
        metadata["format_version"]
            .as_u64()
            .ok_or_else(|| invalid("Metadata has no format version"))? as u16,
    )?;
    gate::parallel(recording.segments().len(), args.threads, |index| {
        let mut permit = admission.union()?;
        let cache = permit.spool_cache()?;
        eprintln!("union segment {}/{}", index + 1, recording.segments().len());
        match gate::measure(
            index,
            &recording,
            &resolved,
            &layout,
            format,
            &args.out,
            args.projects_scoped,
            &cache,
        ) {
            Ok(segment) => {
                segments.write(index, Some(&segment))?;
                excluded_damage.write(index, None)
            }
            Err(error) => {
                match gate::damage(index, &recording.segments()[index], &recording, &error) {
                    Some(damage) => {
                        segments.write(index, None)?;
                        excluded_damage.write(index, Some(&damage))
                    }
                    None => Err(error),
                }
            }
        }
    })?;
    for (at, damage) in recording.damage().iter().enumerate() {
        excluded_damage.write(
            recording.segments().len() + at,
            Some(&Lost {
                segment: None,
                run: None,
                first_ordinal: None,
                last_ordinal: None,
                first_hour: None,
                last_hour: None,
                scope: damage.scope.to_string(),
                reason: damage.to_string(),
            }),
        )?;
    }
    let (_, rate) = clock.elapsed()?;
    let mut union_timing = gate::Timing::default();
    let mut boundaries = gate::Boundaries::default();
    for at in 0..segments.slots {
        if let Some(mut segment) = segments.read(at)? {
            segment.timing.calibrate(rate);
            for index in &mut segment.reverse {
                index.bootstrap_us_per_hour =
                    Some(index.bootstrap_cycles as f64 / rate * 1e6 / segment.timing.hours as f64);
                index.upkeep_us_per_hour =
                    Some(index.upkeep_cycles as f64 / rate * 1e6 / segment.timing.hours as f64);
            }
            if boundaries.restore_is_duplicate(
                segment.run,
                segment.first_ordinal,
                segment.last_ordinal,
            ) {
                segment.restore_checkpoint_verification = Some(segment.checkpoints.remove(0));
            }
            union_timing.hours += segment.timing.hours;
            union_timing.restore_cycles += segment.timing.restore_cycles;
            union_timing.hourly_decode_apply_cycles += segment.timing.hourly_decode_apply_cycles;
            segments.replace(at, Some(&segment))?;
        }
    }
    let hours = union_timing.hours;
    let mut groupings = if hours == 0 {
        Vec::new()
    } else {
        grouping::plans(
            &resolved,
            &layout,
            &segments,
            &args.out,
            grouping::Representation::ReaderAllocation,
        )?
    };
    if hours != 0 {
        groupings.extend(grouping::plans(
            &resolved,
            &layout,
            &segments,
            &args.out,
            grouping::Representation::Floor,
        )?);
    }
    let shared_relations: Vec<_> = resolved
        .shared_relations
        .iter()
        .map(|at| layout.field_dimensions() + at)
        .collect();
    grouping::peaks(
        &mut groupings,
        &layout,
        &segments,
        &layout
            .indexes(&resolved.shared_union)
            .into_iter()
            .chain(shared_relations.iter().copied())
            .collect::<Vec<_>>(),
        &layout
            .reader_indexes(&resolved.shared_union)
            .into_iter()
            .chain(layout.change_indexes(&resolved.shared_union))
            .chain(shared_relations.iter().copied())
            .collect::<Vec<_>>(),
    )?;
    let (group_walks, shared_walk, union_walk) = if hours == 0 {
        (Vec::new(), None, None)
    } else {
        eprintln!(
            "measuring clean union, family-group and shared-owner table selections at one active reader"
        );
        let shared_tables: Vec<_> = resolved.shared_union.iter().map(|s| s.table).collect();
        let (walks, shared, union) = grouping::walks(
            &mut groupings,
            &segments,
            &args.columnar,
            args.threads,
            rate,
            &admission,
            &resolved.tables,
            &shared_tables,
            &args.out,
            hours,
        )?;
        (walks, Some(shared), Some(union))
    };
    let mut ordered_lanes = Vec::new();
    if hours != 0 {
        for family in resolved.manifest.families.iter().filter(|f| f.ordered) {
            let mut kinds = vec!["full", "filtered"];
            if family.name == "politics_ledger"
                || (family.name == "projects" && args.projects_scoped == Some(true))
            {
                kinds.push("trigger_context_upper_bound");
            }
            for kind in kinds {
                let mut lane = Lane {
                    family: family.name.clone(),
                    spool: kind.into(),
                    records: 0,
                    records_per_hour: 0.0,
                    bytes: 0,
                    zstd3_bytes: 0,
                    read_path_cycles: 0,
                    read_path_us_per_hour: 0.0,
                    projected_read_path_seconds: 0.0,
                    measurement_scope: if kind == "filtered" && family.name == "politics_ledger" {
                        "own read-set only; trigger-context share separate; own share includes historical over-delivery (already-delivered rows re-sent, and history supplied outside held-frame hours)"
                    } else if kind == "filtered" && family.name == "projects" {
                        "own read-set only; trigger-context share separate"
                    } else if kind == "filtered" {
                        "own share"
                    } else {
                        "not assessed"
                    }
                    .into(),
                    active_hours: BTreeMap::new(),
                    segments: Detail::new(
                        &args.out.join(format!("delivery-{}-{kind}", family.name)),
                        segments.slots,
                    )?,
                };
                for at in 0..segments.slots {
                    if let Some(segment) = segments.read(at)? {
                        let pair = &segment.spools[&family.name];
                        let encoded = if kind == "full" {
                            &pair.full
                        } else if kind == "filtered" {
                            &pair.filtered
                        } else {
                            pair.trigger_context_upper_bound
                                .as_ref()
                                .ok_or_else(|| invalid("Missing trigger upper bound"))?
                        };
                        let delivery = encoding::deliver(encoded)?;
                        lane.records += delivery.records;
                        lane.bytes += delivery.bytes;
                        lane.zstd3_bytes += encoded.zstd3_bytes;
                        for (rule, count) in &encoded.active_hours {
                            *lane.active_hours.entry(rule.clone()).or_default() += count;
                        }
                        lane.read_path_cycles += delivery.cycles;
                        let read_path_us_per_hour =
                            delivery.cycles as f64 / rate * 1e6 / segment.timing.hours as f64;
                        lane.segments.write(
                            at,
                            Some(&SegmentDelivery {
                                segment: segment.index,
                                hours: segment.timing.hours,
                                delivery,
                                read_path_us_per_hour,
                            }),
                        )?;
                    } else {
                        lane.segments.write(at, None)?;
                    }
                }
                lane.read_path_us_per_hour =
                    lane.read_path_cycles as f64 / rate * 1e6 / hours as f64;
                lane.records_per_hour = lane.records as f64 / hours as f64;
                lane.projected_read_path_seconds = lane.read_path_us_per_hour * 438_000.0 / 1e6;
                ordered_lanes.push(lane);
            }
        }
    }
    let verdict = if hours == 0 {
        "No verdict: no undamaged measured segments.".into()
    } else if let Some(plan) = groupings.iter().find(|p| {
        p.representation == grouping::Representation::ReaderAllocation
            && p.readers_fitting_largest_first >= 8
    }) {
        format!(
            "Current-reader allocation: {} groups admit {} of {} available readers within the 0.9 GB split ({}); duplicated clean hourly decode-plus-apply {:.3} us/h. Shared tables, their retained change lists and reverse indexes are charged once per segment; complete dictionary/view allocations are conservatively charged per reader, including shared dictionary chunk payloads. Includes private retained change lists, reverse and previous allocations; State.scratch buffers, transients and growth reservations remain unmeasured. Rebalance measured owners within 3 GB before considering paging or grouping. Use the 13-year corpus for planning; 1941 is a sanity pass.",
            plan.count,
            plan.readers_fitting_largest_first,
            plan.available_readers,
            if plan.segment_limited {
                "segment-limited; memory headroom unproven"
            } else {
                "memory-limited"
            },
            plan.duplicated_decode_us_per_hour.unwrap()
        )
    } else {
        "Current-reader allocation: no group count admits eight readers within the 0.9 GB split. Shared tables, their retained change lists and reverse indexes are charged once per segment; complete dictionary/view allocations are conservatively charged per reader, including shared dictionary chunk payloads. Includes private retained change lists, reverse and previous allocations. Rebalance measured owners within 3 GB before paging or grouping. Use the 13-year corpus for planning; 1941 is a sanity pass. State.scratch buffers, growth reservations and transients remain unmeasured.".into()
    };
    let provenance = report::provenance(&resolved.manifest);
    let totals = report::totals(&segments, &ordered_lanes, rate, args.projects_scoped)?;
    let mut damage_count = 0;
    excluded_damage.for_each(|_| {
        damage_count += 1;
        Ok(())
    })?;
    let report = Report {
        version: 2,
        recording: args.recording,
        columnar: args.columnar,
        metadata,
        manifest: resolved.manifest,
        manifest_provenance: provenance,
        definitions: serde_json::json!({
            "cpu":"QueryThreadCycleTime around Walker::begin and between hourly walk callbacks; decode and apply combined, all columns of selected tables. Clean union/group/shared walks all use timed_walk with one active reader and the same exclusive 2.2 GB permit; no census, reverse or spool work intervenes. Census-pass timing is retained separately and is not used for duplicated decode. Final page checks and dictionary trim are included as tail cycles.",
            "cycle_calibration":"Invariant x86-64 TSC delta / measured wall seconds, as dashboard measure.rs; all raw cycle counts retained.",
            "cell_bytes":"Live cells: Kind::slot_bytes plus exact list/opaque payload; one recorded bit per cell, optional presence bit, rounded to 64-bit words per column. Text cells hold 32-bit dictionary IDs. Complete dictionary allocations are reported separately and conservatively charged per admitted reader, including shared chunk payloads, in both representations.",
            "keys":"16-byte key and 4-byte slot per live row, plus 64-bit live words; compact sorted key/slot representation; no dead-slot slack counted in the floor. Actual complete-table allocation peaks also reported.",
            "reader_representation":"Today's replay-columnar Table: slot-addressed column Vec capacities, dead slots, variable payload, live/recorded/optional bits and key-to-slot hash-index capacity, using Table::held_bytes. The hash index charges element capacity, not allocated buckets; SwissTables allocate about 8/7 as many buckets, so this component is understated. All columns of a selected table are held because today's reader restores them together. Every exposed per-table Changes vector is charged at capacity through columns::changes, including retained fill_slots and fill_masks after FILL; these bytes enter current private group and shared-owner peaks and admission, not previous-table copies or the compact floor. Previous lookups retain the preceding hour's complete Table allocation; reverse vectors use measured capacity. State::held_bytes minus table allocations supplies complete dictionary and union-reader view allocations, conservatively charged per reader; dictionary chunk payloads can be Arc-shared but are not discounted. Shared tables, their change lists and reverse indexes alone are charged once per segment. These are measured retained-state capacities under today's reader accounting, not the future paged reader or a measured transient/growth-reservation peak. No page granularity or invented headroom is applied. Memory admission uses this representation; the compact named-field floor is a separate comparison.",
            "unmeasured_owners":"Retained State.scratch buffers (u8s, i8s, u16s, i16s, u32s, i32s, u64s, i64s, words, ends, keys, hunks, edits, triples, slots, positions, except and masks), the retained capacity of State.named (the dictionary-trim id list), watch/notification bookkeeping, transient section decode and restore buffers, staging and reserve-before-growth high water are unmeasured, not zero. Retained-state admission is not a complete scheduler memory claim.",
            "country_scaling":"Only recorded rows and allocations are reported. There is no measured 1500-country projection or inferred bytes-per-country scaling.",
            "spool_boundaries":"Independent segments retain initial state records; eliding a contiguous segment's repeated initial state is a separate format lever, not a claimed saving here.",
            "memory_budget":"0.9 GB is the current reader split of the fixed 3 GB total. Report owners for rebalancing before paging or grouping; no whole-design 3 GB verdict is possible without the other owners and transients.",
            "previous":"Declared previous lookups retain the previous hour's named keys/cells. No previous hour is materialized at a segment's restore.",
            "reverse":"Concrete sorted vector of (typed value, u128 row key), allocated vector capacity and owned text payload. Rebuilt only when an input changes; this is measurement upkeep CPU, not a future reader estimate.",
            "checkpoint_encoding":"Version 1: u64 LE record length; i32 LE hour, u8 event, u16 LE table, u128 LE key; status/sample or repeated u16 field, u8 recorded, u8 cell-kind and absolute LE scalar/presence/list. Lengths are u64 LE, strings inline, opaque bytes lossless; unknowns retain their decode bases. Raw and single-thread zstd level 3 streams.",
            "spool":"Absolute record framing and zstd-3 no-op delivery; explicit presence transitions, gated snapshot observations with initial state and accumulated row deltas and dated lookup hydration have matching decoder events. Full streams the merged family declarations without read-set filtering; filtered uses each ordered family's source-audited own read set, hydrates declared lookup keys at first reference and delivers changes while referenced. Full-state exceptions and handler-held reference lifetimes are declared with sources. Both include restore records in their per-hour totals. Historical reference closure over-delivers for politics_ledger: it re-sends rows already delivered and supplies history at every new-reference hour, not only held-frame hours, so that family's filtered records, bytes and delivery are inflated, never understated. No other ordered family declares historical referenced tables.",
            "spool_reference_memory":"All on-disk spool indexes in the exclusive union pass share one fully associative clock page pool. The named spool scratch page-cache owner reserves the remaining 112 MB of that pass's 2.2 GB permit before allocation: 2.2 GB minus the existing 1.8 GB table, 256 MB decode and 32 MB dictionary allowances. This is 112/2200 of the exclusive permit, inside the same exclusive reservation used by all later clean walks. Per-segment and total reports put the measured allocated page/slot-metadata peak beside this allowance. Every index can use the entire pool; spill is lossless. Clearing an index resets its root without truncation or page syscalls. Reference edges follow inserted/deleted source rows, reference-field writes and membership deltas; delivery-hour toggles do not rebuild them. Gated delivery keeps pending keys and offsets of exact last-delivered rows in the raw spool, without a second copy of their values, compares only candidates and sends a full state only on first activation. Previous-hour lookup history appends without idle-hour seeks or flushes.",
            "spool_lookup_proof":proof::reads().collect::<Vec<_>>(),
            "spool_source_accesses":proof::source_reads(),
            "spool_source_policies":proof::source_policies()?,
            "spool_index_cache_budget_bytes":gate::spool_cache_allowance(),
            "spool_read_evidence_proof":proof::READ_RULES,
            "ordered_3_spellings_followup":"Projects reads every LexicalTokens literal today; ordered-3 must move that stream to the shared spellings index (design 2.4).",
            "spool_full_baseline":"Full uses the merged family declarations without read-set filters. Filtered contains only the family's own reads. The whole trigger kit is a separate trigger_context_upper_bound at ledger dated/held hours and projects' scoped hours only. Its fields are an upper bound, never a proof requirement. Neither row is assessed against the lane allowance.",
            "spool_proof_derivation":"Source extraction scans the embedded family and cited helper files (excluding test modules) independently of the manifest: table accesses, explicit field calls, TableRead field arrays, and schema-backed members of typed row decodes and parameters. Source-derived row/hour policy witnesses check the handler predicates against the gate, including the named-payment expression, date/hold rules, decision pack/kinds, slice filters and presence witness. Startup refuses missing extracted fields/tables or changed witnesses. Hand-kept: the source-file roster, unrelated helper-path exclusions (reported per access), indirect/generic/closure data-flow READS, replay reachability, reference relations/lifetimes and read-evidence rules. This is a checked source extraction and manual indirect-access audit, not a completeness proof for arbitrary Rust data flow.",
            "trigger_context_follow_up":"CONTENT_STATIC_READ_UNION: load the recording's content and enumerate all holdable scripted texts, unioning dashboard_text::triggers::inputs::Key::dependence (crates/dashboard-text/src/triggers/inputs.rs:113). No existing exported function enumerates that static text union; Inputs::declared is evaluation-specific and cannot replace it. Resolve project mode with dashboard_special_projects::products::special_projects::facts::Binding::load and Binding::scoped (facts.rs:131-172), whose load calls completions::scripted_names (facts.rs:152). This gate does not load content; --projects-scoped must be supplied from that binding, and omission leaves that upper bound unmeasured, not zero.",
            "projects_scoped":args.projects_scoped,
            "shared_owner":"Shared owners retain the exact union of family fields, including separately reported full-row fallback columns. Trigger-context remains an explicit whole-table upper bound. Overlapping owner fields and keys are held once in the shared union, outside family weights and group bytes. Admission charges its measured peak once per distinct admitted segment; decode includes one shared-union walk per segment.",
            "ordered_timing":"This tool times its own zstd-3/generic-parser/no-op read path and projects only that path to 438000 hours. It does not measure section 2.5.2's lane cost and makes no verdict against 29.7 us/h. core-spool-bench in crate core-spool measures the lane's typed format and keyed replay cost; family handler cost remains separate.",
            "restore_timing":"Raw restore cycles and microseconds per segment plus amortized microseconds per actual measured segment hour. group_walks.restore_us_per_segment is the mean over all included segments, including empty restores. Segment 0 can have an empty restore; its first full load belongs to hourly decode. No corpus is normalized to another corpus's restart interval.",
            "tool_lifetime":"Counters include aggregation and the flushed JSON/Markdown report bodies. Only appending these fixed-size counter footers follows the final sample.",
            "grouping":"For each representation and count 1..68, sort families by maximum measured private peak descending, ties by name; assign to least sum of family peaks, ties by member count then group index. Union fields and identical reverse relations within each group, share nothing across groups; shared owners are separate. Exact group peaks replay census changes at each hour, including complete dictionary allocations per reader in both representations, union-reader views per current reader, and private retained Changes capacities in current-reader groups. Admit the descending prefix of all (segment,group) peaks; charge shared tables, their Changes capacities (current-reader only) and reverse indexes once per distinct admitted segment and stop at the first total that does not fit 900000000 bytes.",
            "duplicate_decode":"Measured sum of group-table-set walks plus one shared-owner walk per segment minus measured union walk; shared-table walks decoded independently. Negative differences are retained as timing noise, never clamped.",
            "tool_admission":"The census union pass and every clean union/group/shared walk take the same exclusive 2.2 GB reader reservation. Exactly one reader is active; --threads only caps waiting workers. This is the tool reservation, not a measured per-reader transient peak or future scheduler concurrency.",
            "damage":"Every damaged primary segment is wholly excluded from totals, spools and admission results. No zero or reconstructed substitute is inserted. The verdict describes only included coverage.",
        }),
        recorded_hours: recording.hours(),
        measured_hours: hours,
        segments,
        excluded_damage,
        damage_count,
        totals,
        groupings,
        group_walks,
        shared_walk,
        union_walk,
        ordered_lanes,
        verdict,
    };
    let mut json = BufWriter::new(
        File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(args.out.join("report.json"))?,
    );
    serde_json::to_writer_pretty(&mut json, &report)?;
    json.flush()?;
    let markdown_path = args.out.join("report.md");
    report::markdown(&report, &markdown_path)?;
    let (wall_seconds, _) = clock.elapsed()?;
    let process_cpu_seconds = cpu::process_ns()?.saturating_sub(process_start) as f64 / 1e9;
    let (peak_working_set_bytes, peak_commit_bytes) = cpu::peaks()?;
    let tool = Tool {
        wall_seconds,
        process_cpu_seconds,
        peak_commit_bytes,
        peak_working_set_bytes,
        under_three_gb: peak_commit_bytes < 3_000_000_000,
        requested_threads: args.threads,
        maximum_worker_threads: if hours == 0 {
            args.threads.min(recording.segments().len())
        } else {
            args.threads
        },
        maximum_tool_threads_including_main: if hours == 0 {
            args.threads.min(recording.segments().len()) + 1
        } else {
            args.threads + 1
        },
        peak_active_readers: admission.peak.load(Ordering::Relaxed),
        thread_cycle_hz: rate,
        cpu_clock: "QueryThreadCycleTime",
    };
    json.seek(SeekFrom::End(-1))?;
    write!(json, ",\n  \"tool\": ")?;
    serde_json::to_writer(&mut json, &tool)?;
    writeln!(json, "\n}}")?;
    json.flush()?;
    let mut markdown = File::options().append(true).open(markdown_path)?;
    writeln!(
        markdown,
        "\nTool: {:.3} s wall, {:.3} s process CPU, {} bytes peak commit, {} bytes peak working set; {} requested workers, {} peak active readers. Under 3 GB: {}.",
        tool.wall_seconds,
        tool.process_cpu_seconds,
        tool.peak_commit_bytes,
        tool.peak_working_set_bytes,
        tool.requested_threads,
        tool.peak_active_readers,
        tool.under_three_gb
    )?;
    markdown.flush()?;
    println!("{}", report.verdict);
    Ok(())
}

fn main() {
    if let Err(error) = Args::read().and_then(run) {
        eprintln!("core-gate-memory: {error}");
        std::process::exit(1);
    }
}
