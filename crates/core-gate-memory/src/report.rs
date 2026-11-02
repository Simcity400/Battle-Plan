use crate::{Lane, Report, detail::Detail, gate::SegmentReport, manifest::Manifest};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

const NO_NOTE: &[&str] = &[
    "air",
    "air-missions",
    "air-statistics",
    "air_ledger",
    "army",
    "casualties",
    "country-styles",
    "diplomacy_state",
    "division-equipment",
    "divisions",
    "fuel",
    "identities",
    "land",
    "manpower",
    "naval-refits",
    "naval-repairs",
    "nukes",
    "place_names",
    "stability",
    "stockpile",
    "supply",
];

pub fn provenance(manifest: &Manifest) -> Value {
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut undeclared = BTreeMap::new();
    for family in &manifest.families {
        *counts.entry(&family.origin).or_default() += 1;
        let tables: Vec<_> = family
            .tables
            .iter()
            .filter(|t| t.source == "undeclared")
            .map(|t| &t.name)
            .collect();
        if !tables.is_empty() {
            undeclared.insert(&family.name, tables);
        }
    }
    json!({"origins":counts,"undeclared_tables_by_family":undeclared,"spec_no_note_families":NO_NOTE,
        "unresolved_tables_or_fields":[],"source_discrepancy":"family-naval-regions.md covers minefields and naval-supremacy; naval's direct fields are derived from its operator and shared readers."})
}

fn add(value: &mut Value, name: &str, count: u64) {
    value[name] = json!(value[name].as_u64().unwrap_or(0) + count);
}
fn maximum(
    value: &mut Value,
    name: &str,
    bytes: u64,
    candidate: impl serde::Serialize,
) -> io::Result<()> {
    let old = &value[name];
    let old_bytes = old["peak"]["bytes"].as_u64().or_else(|| old.as_u64());
    if old_bytes.is_none_or(|n| bytes > n) {
        value[name] = serde_json::to_value(candidate)?;
    }
    Ok(())
}

pub fn totals(
    segments: &Detail<SegmentReport>,
    lanes: &[Lane],
    rate: f64,
    projects_scoped: Option<bool>,
) -> io::Result<Value> {
    let mut out = json!({"tables":{},"families":{},"families_primary":{},"families_fallback":{},"families_private":{},"shared":{},"shared_whole":{},"shared_reader":{},"reverse":{},"spools":{},"code_families_whole":{}});
    segments.for_each(|segment| {
        add(&mut out, "included_segments", 1);
        add(&mut out, "included_hours", segment.timing.hours);
        if out["spool_index_cache"]["peak_bytes"]
            .as_u64()
            .is_none_or(|peak| peak < segment.spool_index_cache.peak_bytes as u64)
        {
            out["spool_index_cache"] = serde_json::to_value(&segment.spool_index_cache)?;
        }
        for (name, table) in &segment.tables {
            let entry = &mut out["tables"][name];
            if entry.is_null() {
                *entry = json!({});
            }
            add(entry, "restore_named_bytes_sum", table.named.restore.bytes);
            maximum(
                entry,
                "maximum_reader_allocation_peak",
                table.reader_allocation.peak.bytes,
                &table.reader_allocation,
            )?;
            maximum(
                entry,
                "maximum_named_peak",
                table.named.peak.bytes,
                &table.named,
            )?;
            maximum(
                entry,
                "maximum_declared_only_peak",
                table.declared_only.peak.bytes,
                &table.declared_only,
            )?;
            maximum(
                entry,
                "maximum_unnamed_cells_peak",
                table.unnamed_cells.peak.bytes,
                &table.unnamed_cells,
            )?;
            maximum(
                entry,
                "maximum_whole_table_peak",
                table.whole.peak.bytes,
                &table.whole,
            )?;
        }
        for (kind, values) in [
            ("families", &segment.families),
            ("families_primary", &segment.families_primary),
            ("families_fallback", &segment.families_fallback),
            ("families_private", &segment.families_private),
            ("shared", &segment.shared),
            ("shared_whole", &segment.shared_whole),
            ("shared_reader", &segment.shared_reader),
        ] {
            for (name, peak) in values {
                let entry = &mut out[kind][name];
                if entry.is_null() {
                    *entry = json!({});
                }
                add(entry, "restore_named_bytes_sum", peak.restore.bytes);
                maximum(
                    entry,
                    "maximum_restore_bytes",
                    peak.restore.bytes,
                    peak.restore.bytes,
                )?;
                maximum(entry, "maximum_named_peak", peak.peak.bytes, peak)?;
            }
        }
        for (name, peak) in [
            ("union_reader_maximum_peak", &segment.union_reader),
            ("declared_reader_maximum_peak", &segment.declared_reader),
            (
                "shared_reader_union_maximum_peak",
                &segment.shared_reader_union,
            ),
        ] {
            maximum(&mut out, name, peak.peak.bytes, peak)?;
        }
        maximum(
            &mut out,
            "reader_auxiliary_maximum_bytes",
            segment.reader_auxiliary_peak_bytes,
            segment.reader_auxiliary_peak_bytes,
        )?;
        maximum(
            &mut out,
            "shared_union_maximum_peak",
            segment.shared_union.peak.bytes,
            &segment.shared_union,
        )?;
        maximum(
            &mut out,
            "shared_without_trigger_maximum_peak",
            segment.shared_without_trigger.peak.bytes,
            &segment.shared_without_trigger,
        )?;
        for (name, peak) in &segment.code_families_whole {
            let entry = &mut out["code_families_whole"][name];
            if entry.is_null() {
                *entry = json!({});
            }
            maximum(
                entry,
                "maximum_restore_bytes",
                peak.restore.bytes,
                peak.restore.bytes,
            )?;
            if entry["maximum_peak_bytes"]
                .as_u64()
                .is_none_or(|old| peak.peak.bytes > old)
            {
                entry["maximum_peak_bytes"] = json!(peak.peak.bytes);
                entry["share"] = json!(segment.code_families_union_share[name].1);
            }
        }
        for index in &segment.reverse {
            let entry = &mut out["reverse"][&index.name];
            if entry.is_null() {
                *entry = json!({});
            }
            add(entry, "restore_edges_sum", index.restore.edges);
            add(entry, "restore_bytes_sum", index.restore.bytes);
            add(entry, "bootstrap_cycles", index.bootstrap_cycles);
            add(entry, "upkeep_cycles", index.upkeep_cycles);
            if entry["maximum_peak"]["peak"]["bytes"]
                .as_u64()
                .is_none_or(|b| index.peak.bytes > b)
            {
                entry["maximum_peak"] = serde_json::to_value(index)?;
            }
        }
        for (name, pair) in &segment.spools {
            for (kind, spool) in [
                ("full", Some(&pair.full)),
                ("filtered", Some(&pair.filtered)),
                (
                    "trigger_context_upper_bound",
                    pair.trigger_context_upper_bound.as_ref(),
                ),
            ] {
                let Some(spool) = spool else {
                    continue;
                };
                let entry = &mut out["spools"][name][kind];
                if entry.is_null() {
                    *entry = json!({});
                }
                add(entry, "raw_bytes", spool.raw_bytes);
                add(entry, "zstd3_bytes", spool.zstd3_bytes);
                add(entry, "records", spool.records);
                for (rule, count) in &spool.active_hours {
                    let active = &mut entry["active_hours"];
                    if active.is_null() {
                        *active = json!({});
                    }
                    add(active, rule, *count);
                }
            }
        }
        add(
            &mut out,
            "union_restore_bytes_sum",
            segment.union.restore.bytes,
        );
        maximum(
            &mut out,
            "union_maximum_peak",
            segment.union.peak.bytes,
            &segment.union,
        )?;
        maximum(
            &mut out,
            "declared_only_maximum_peak",
            segment.declared_only.peak.bytes,
            &segment.declared_only,
        )?;
        add(&mut out, "restore_cycles", segment.timing.restore_cycles);
        add(
            &mut out,
            "hourly_decode_apply_cycles",
            segment.timing.hourly_decode_apply_cycles,
        );
        for checkpoint in &segment.checkpoints {
            add(&mut out, "checkpoint_boundaries", 1);
            add(&mut out, "checkpoint_raw_bytes", checkpoint.raw_bytes);
            add(&mut out, "checkpoint_zstd3_bytes", checkpoint.zstd3_bytes);
        }
        maximum(
            &mut out,
            "dictionary_maximum_bytes",
            segment.dictionary_peak_bytes,
            segment.dictionary_peak_bytes,
        )?;
        maximum(
            &mut out,
            "previous_reader_maximum_bytes",
            segment.previous_reader_peak_bytes,
            segment.previous_reader_peak_bytes,
        )?;
        maximum(
            &mut out,
            "previous_maximum_bytes",
            segment.previous_peak_bytes,
            segment.previous_peak_bytes,
        )?;
        maximum(
            &mut out,
            "reverse_maximum_simultaneous_bytes",
            segment.reverse_peak_bytes,
            segment.reverse_peak_bytes,
        )?;
        Ok(())
    })?;
    let hours = out["included_hours"].as_u64().unwrap_or(0);
    let us = |cycles: &Value| {
        cycles
            .as_u64()
            .filter(|_| hours != 0)
            .map(|c| c as f64 / rate * 1e6 / hours as f64)
    };
    out["census_restore_us_per_hour"] = json!(us(&out["restore_cycles"]));
    out["census_hourly_decode_apply_us_per_hour"] = json!(us(&out["hourly_decode_apply_cycles"]));
    for entry in out["reverse"].as_object_mut().unwrap().values_mut() {
        entry["bootstrap_us_per_hour"] = json!(us(&entry["bootstrap_cycles"]));
        entry["upkeep_us_per_hour"] = json!(us(&entry["upkeep_cycles"]));
    }
    for lane in lanes {
        out["spools"][&lane.family][&lane.spool]["delivery"] = json!({
            "records":lane.records,"bytes":lane.bytes,"read_path_cycles":lane.read_path_cycles,
            "records_per_hour":lane.records_per_hour,"zstd3_bytes":lane.zstd3_bytes,
            "read_path_us_per_hour":lane.read_path_us_per_hour,"projected_read_path_seconds":lane.projected_read_path_seconds,
            "measurement":"tool zstd-3/generic-parser/no-op read path; no lane allowance verdict", "segment_detail":lane.segments.path,
        });
    }
    if out["spools"].get("projects").is_some()
        && out["spools"]["projects"]["trigger_context_upper_bound"].is_null()
    {
        out["spools"]["projects"]["trigger_context_upper_bound"] = json!({
            "state":if projects_scoped == Some(false) { "not_applicable_unscoped" } else { "unmeasured_scope_unknown" },
            "records":null,"raw_bytes":null,"zstd3_bytes":null,"delivery":null
        });
    }
    Ok(out)
}

fn escape(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

pub fn markdown(report: &Report, path: &Path) -> io::Result<()> {
    let mut out = BufWriter::new(File::create_new(path)?);
    writeln!(out, "{}\n", report.verdict)?;
    writeln!(
        out,
        "Measured {} of {} recorded hours, in {} included segments. {} damage records are excluded from totals. The verdict applies only to included coverage.\n",
        report.measured_hours,
        report.recorded_hours,
        report.totals["included_segments"],
        report.damage_count
    )?;
    let union = report.totals["union_maximum_peak"]["peak"]["bytes"].as_u64();
    let declared = report.totals["declared_only_maximum_peak"]["peak"]["bytes"].as_u64();
    writeln!(
        out,
        "Reader owners against the current 900,000,000-byte split of the fixed 3 GB total:\n\n| Owner | Compact named-field floor bytes | Current-reader allocation bytes |\n|---|---:|---:|\n| Selected union including shared upper bound, undeclared included | {} | {} |\n| Selected union including shared upper bound, undeclared omitted | {} | {} |",
        display(union),
        display(report.totals["union_reader_maximum_peak"]["peak"]["bytes"].as_u64()),
        display(declared),
        display(report.totals["declared_reader_maximum_peak"]["peak"]["bytes"].as_u64())
    )?;
    writeln!(out, "| Readers budget split | 900000000 | 900000000 |")?;
    for (label, key) in [
        (
            "Charged shared union, including shared reverse indexes",
            "shared_union_maximum_peak",
        ),
        (
            "Shared union without trigger-context, including its shared reverse indexes",
            "shared_without_trigger_maximum_peak",
        ),
    ] {
        writeln!(
            out,
            "| {label} | {} | {} |",
            display(report.totals[key]["peak"]["bytes"].as_u64()),
            if key == "shared_union_maximum_peak" {
                display(report.totals["shared_reader_union_maximum_peak"]["peak"]["bytes"].as_u64())
            } else {
                "not measured separately".into()
            }
        )?;
    }
    for owner in &report.manifest.shared {
        writeln!(
            out,
            "| Shared {} {} | {} | {} |",
            owner.name,
            if owner.name == "trigger-context" {
                "whole tables"
            } else {
                "declared values and recorded bits, including fallbacks"
            },
            display(
                report.totals["shared"][&owner.name]["maximum_named_peak"]["peak"]["bytes"]
                    .as_u64()
            ),
            display(
                report.totals["shared_reader"][&owner.name]["maximum_named_peak"]["peak"]["bytes"]
                    .as_u64()
            )
        )?;
        writeln!(
            out,
            "| Shared {} whole-table floor, including shared reverse indexes | {} | see allocation above |",
            owner.name,
            display(
                report.totals["shared_whole"][&owner.name]["maximum_named_peak"]["peak"]["bytes"]
                    .as_u64()
            )
        )?;
    }
    let reverse = report.totals["reverse_maximum_simultaneous_bytes"].as_u64();
    writeln!(
        out,
        "| Reverse allocations, simultaneous peak | {0} | {0} |\n| Complete dictionary allocation, conservatively charged per reader | {1} | included below |\n| Complete dictionary and union-reader view allocations, conservatively charged per reader | dictionary above | {2} |",
        display(reverse),
        display(report.totals["dictionary_maximum_bytes"].as_u64()),
        display(report.totals["reader_auxiliary_maximum_bytes"].as_u64())
    )?;
    writeln!(
        out,
        "| Previous state | {} | {} |",
        display(report.totals["previous_maximum_bytes"].as_u64()),
        display(report.totals["previous_reader_maximum_bytes"].as_u64())
    )?;
    let one_group = |representation| {
        report
            .groupings
            .iter()
            .find(|s| s.count == 1 && s.representation == representation)
            .and_then(|s| s.groups.first())
            .map(|g| g.peak_with_shared_bytes)
    };
    writeln!(
        out,
        "| One-group reader including shared owners, dictionary, reverse and previous (current-reader includes retained Changes) | {} | {} |\n",
        display(one_group(crate::grouping::Representation::Floor)),
        display(one_group(crate::grouping::Representation::ReaderAllocation))
    )?;
    writeln!(
        out,
        "Spool scratch page cache (exclusive union-pass permit): measured allocation peak {} bytes / allowance {} bytes. The allowance is the pass's 2,200,000,000 bytes minus table storage (1,800,000,000), decode buffers (256,000,000) and dictionary (32,000,000); all clean walks take that same exclusive 2.2 GB permit without spool scratch.\n",
        display(report.totals["spool_index_cache"]["peak_bytes"].as_u64()),
        display(report.totals["spool_index_cache"]["allowance_bytes"].as_u64()),
    )?;
    writeln!(
        out,
        "The floor counts live named cells, recorded bits and compact key/slot storage; it is not a usable reader allocation. Shared tables, their retained change lists (current-reader only) and reverse indexes are outside family group bytes and charged once per segment; previous-hour fields, private change lists and private reverse indexes remain in their groups. Owners' figures may overlap trigger-context's whole tables: admission and decode use their deduplicated union. Unused decoded fields retain only the recorded bits that change a served value; fallback-only bits are charged separately. The union without trigger-context retains the other owners' overlapping fields; owner maxima are independent and cannot be added. Current-reader allocation uses today's complete slot-addressed Table vectors, dead slots, key index and capacity slack, plus prior-hour complete tables for previous reads; all columns are held. Table::held_bytes is the reader's capacity accounting, not an allocator high-water sample. The key index charges hash-map element capacity rather than allocated buckets; SwissTables allocate about 8/7 as many buckets, understating this component. Every exposed Changes vector is measured at capacity, including fill_slots and fill_masks retained after FILL, and included in current group and shared-owner peaks and admission; previous-table copies exclude change lists. Complete dictionary and union-reader view allocations are conservatively charged per reader, even though dictionary chunk payloads can be Arc-shared. Retained State.scratch buffers (u8s, i8s, u16s, i16s, u32s, i32s, u64s, i64s, words, ends, keys, hunks, edits, triples, slots, positions, except and masks) and the retained capacity of State.named (the dictionary-trim id list) are explicitly unmeasured. No paging, reserve-before-growth peak or decode/staging transient is measured, so the retained-state verdict does not establish a whole-design 3 GB bound. Rebalance measured owners before paging or grouping. Duplicate restored checkpoints are verification artifacts excluded from boundary totals.\n"
    )?;
    writeln!(
        out,
        "The 21 code-derived families, ranked by declared bytes:\n\n| Family | Tables | Declared restore bytes, maximum | Declared peak bytes | Primary peak bytes | Additional fallback peak bytes | Whole restore bytes, maximum | Peak whole bytes | Share of whole-table union |\n|---|---|---:|---:|---:|---:|---:|---:|---:|"
    )?;
    let mut ranked: Vec<_> = NO_NOTE
        .iter()
        .map(|&name| {
            let stats = &report.totals["code_families_whole"][name];
            let restore = stats["maximum_restore_bytes"].as_u64();
            let peak = stats["maximum_peak_bytes"].as_u64();
            let family = report.manifest.families.iter().find(|f| f.name == name);
            let tables = family
                .map(|f| {
                    f.tables
                        .iter()
                        .map(|t| t.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let share = stats["share"].as_f64();
            (name, tables, restore, peak, share)
        })
        .collect();
    ranked.sort_by(|a, b| {
        report.totals["families"][b.0]["maximum_named_peak"]["peak"]["bytes"]
            .as_u64()
            .cmp(&report.totals["families"][a.0]["maximum_named_peak"]["peak"]["bytes"].as_u64())
            .then(a.0.cmp(b.0))
    });
    for (name, tables, restore, peak, share) in ranked {
        writeln!(
            out,
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {} |",
            escape(&tables),
            display(report.totals["families"][name]["maximum_restore_bytes"].as_u64()),
            display(
                report.totals["families"][name]["maximum_named_peak"]["peak"]["bytes"].as_u64()
            ),
            display(
                report.totals["families_primary"][name]["maximum_named_peak"]["peak"]["bytes"]
                    .as_u64()
            ),
            display(
                report.totals["families_fallback"][name]["maximum_named_peak"]["peak"]["bytes"]
                    .as_u64()
            ),
            display(restore),
            display(peak),
            share
                .map(|n| format!("{:.3}%", 100.0 * n))
                .unwrap_or_else(|| "not measured".into())
        )?;
    }
    writeln!(
        out,
        "\nWhole-union shares are measured at each family's largest whole-table peak hour. Restore and peak shares for every segment are in report.json. Code-declared partial fields are used for the named union.\n"
    )?;
    writeln!(
        out,
        "Manifest provenance: {}. Unresolved names: none (validation refuses them). Naval's direct fields are code-derived alongside the 21 families.\n",
        report.manifest_provenance["origins"]
    )?;
    writeln!(
        out,
        "Clean whole-column decode baseline (one active reader, identical permits, no census or spool callbacks):\n\n| Component | Thread CPU us/segment, mean | Thread CPU us/h |\n|---|---:|---:|"
    )?;
    if let Some(at) = report.union_walk {
        let walk = &report.group_walks[at];
        writeln!(
            out,
            "| Restore selected tables (mean over all segments, including empty restores) | {:.3} | {:.3} |\n| Hourly decode plus apply | n/a | {:.3} |\n",
            walk.restore_us_per_segment,
            walk.restore_us_per_hour,
            walk.hourly_decode_apply_us_per_hour
        )?;
    }
    let mut reverse_bootstrap = 0.0;
    let mut reverse_upkeep = 0.0;
    if let Some(indexes) = report.totals["reverse"].as_object() {
        for index in indexes.values() {
            reverse_bootstrap += index["bootstrap_us_per_hour"].as_f64().unwrap();
            reverse_upkeep += index["upkeep_us_per_hour"].as_f64().unwrap();
        }
    }
    if report.measured_hours != 0 {
        writeln!(
            out,
            "Reverse rebuild CPU measured separately: {:.3} us/h bootstrap, {:.3} us/h upkeep; full rebuilds bound incremental upkeep from above. Event building is unmeasured. The design's 150-400 us/h reader projection includes reverse upkeep and additional work; this baseline alone cannot validate it.\n",
            reverse_bootstrap, reverse_upkeep
        )?;
    }
    writeln!(
        out,
        "Every column of every selected table is decoded; decode and apply are combined. Unnamed-column bytes remain beside named bytes in report.json. Census-conditioned timings are retained under census_* totals and segment timing, and never subtracted from clean group walks. Restore is also reported per actual segment below; no 720-hour or 2160-hour normalization is assumed.\n"
    )?;
    if let Some(at) = report.shared_walk {
        let walk = &report.group_walks[at];
        writeln!(
            out,
            "Shared owners' deduplicated independent whole-table walk: {:.3} us/h restore, {:.3} us/h hourly decode plus apply; included once per segment in every scenario's decode accounting.\n",
            walk.restore_us_per_hour, walk.hourly_decode_apply_us_per_hour
        )?;
    }
    writeln!(
        out,
        "The following timing is this tool's zstd-3/generic-parser/no-op read path, including its flat per-record overhead. It does not measure section 2.5.2's lane cost. No spool timing here is assessed against the 29.7 us/h lane allowance. core-spool-bench (crate core-spool) measures the typed lane format and keyed replay cost; handler cost is separate. Trigger context initial states and gated deltas are separate UPPER-BOUND rows, outside the filtered spool, and are hydrated only at their declared hours. politics_ledger's filtered row includes historical over-delivery (already-delivered rows re-sent, and history supplied outside held-frame hours), so it overstates, never understates, that family's own spool.\n"
    )?;
    writeln!(
        out,
        "Tool read path only: zstd-3 and no-op delivery; projected seconds cover this path over 438,000 hours:\n\n| Family | Spool | Active hours by predicate | Records | Records/h | Raw bytes | Zstd-3 bytes | Tool read path us/h | Projected tool seconds |\n|---|---|---|---:|---:|---:|---:|---:|---:|"
    )?;
    for lane in &report.ordered_lanes {
        writeln!(
            out,
            "| {} | {} | {} | {} | {:.3} | {} | {} | {:.3} | {:.6} |",
            lane.family,
            if lane.spool == "trigger_context_upper_bound" {
                "UPPER-BOUND trigger_context"
            } else {
                &lane.spool
            },
            if lane.active_hours.is_empty() {
                "every (ungated)".into()
            } else {
                lane.active_hours
                    .iter()
                    .map(|(rule, hours)| format!("{rule}: {hours}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            lane.records,
            lane.records_per_hour,
            lane.bytes,
            lane.zstd3_bytes,
            lane.read_path_us_per_hour,
            lane.projected_read_path_seconds
        )?;
    }
    for family in ["politics_ledger", "projects"] {
        if report
            .manifest
            .families
            .iter()
            .any(|f| f.ordered && f.name == family)
            && !report
                .ordered_lanes
                .iter()
                .any(|lane| lane.family == family && lane.spool == "trigger_context_upper_bound")
        {
            let state = if family == "projects" && report.definitions["projects_scoped"] == false {
                "not applicable: unscoped"
            } else if family == "projects" {
                "unmeasured: scope unknown"
            } else {
                "unmeasured: no included hours"
            };
            writeln!(
                out,
                "| {family} | UPPER-BOUND trigger_context ({state}) | not measured | not measured | not measured | not measured | not measured | not measured | not measured |"
            )?;
        }
    }
    writeln!(
        out,
        "\nOrdered replay read audit (source extraction and manual helper audit, validated before opening the recording):\n\n| Family | Access | Handler source | Read-set table | Spool | Fields / full-state reason |\n|---|---|---|---|---|---|"
    )?;
    for proof in crate::proof::reads() {
        let Some(family) = report
            .manifest
            .families
            .iter()
            .find(|f| f.name == proof.family)
        else {
            continue;
        };
        let entry = family
            .readset
            .iter()
            .find(|t| t.name == proof.table)
            .unwrap();
        let fields = match &entry.fields {
            crate::manifest::Fields::All(_) => "all".into(),
            crate::manifest::Fields::Named(fields) => fields.join(", "),
        };
        writeln!(
            out,
            "| {} | {} | {} | {} | {:?} | {} |",
            proof.family,
            proof.access,
            escape(proof.source),
            proof.table,
            entry.spool,
            escape(&format!(
                "{fields}; filter={:?}; hours={:?}; {}",
                entry.filter,
                entry.hours,
                entry
                    .reason
                    .as_deref()
                    .unwrap_or("declared references hydrate the named fields")
            ))
        )?;
    }
    writeln!(
        out,
        "\nAudited reference relations:\n\n| Family | From | To | Source |\n|---|---|---|---|"
    )?;
    for family in report.manifest.families.iter().filter(|f| f.ordered) {
        for reference in &family.references {
            writeln!(
                out,
                "| {} | {} | {} | {} |",
                family.name,
                reference.from,
                reference.to,
                escape(&reference.source)
            )?;
        }
    }
    writeln!(
        out,
        "\nAudited read evidence:\n\n| Family | Table | Rule | Source |\n|---|---|---|---|"
    )?;
    for rule in crate::proof::READ_RULES
        .iter()
        .filter(|r| report.manifest.families.iter().any(|f| f.name == r.family))
    {
        writeln!(
            out,
            "| {} | {} | {} | {} |",
            rule.family,
            rule.table,
            rule.rule,
            escape(rule.source)
        )?;
    }
    writeln!(
        out,
        "\nGroup counts, with largest (segment, group) retained-state peaks admitted first. Shared tables, their retained change lists (current-reader only) and reverse indexes are charged once per admitted segment. Complete dictionary storage is conservatively charged per reader in both representations, with union-reader views also charged per current reader; even shared dictionary chunk payloads are not discounted. Current-reader group and shared peaks include every exposed Changes vector at retained capacity. Admitted averages include all these charges. Available readers = included segments x groups; segment-limited rows establish no further memory capacity. Grouping is a comparison after rebalancing, not a selected paging policy.\n\n| Representation | Groups | Readers in 0.9 GB | Available readers | Limited by segments | Admitted bytes | Shared bytes in admission | Bytes/reader, admitted mean | Duplicated clean decode us/h |\n|---|---:|---:|---:|---|---:|---:|---:|---:|"
    )?;
    for grouping in &report.groupings {
        writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            if grouping.representation == crate::grouping::Representation::Floor {
                "floor only"
            } else {
                "current-reader allocation"
            },
            grouping.count,
            grouping.readers_fitting_largest_first,
            grouping.available_readers,
            grouping.segment_limited,
            grouping.readers_bytes,
            grouping.admitted_shared_bytes,
            grouping
                .admitted_average_bytes_per_reader
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "not admitted".into()),
            grouping
                .duplicated_decode_us_per_hour
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "not measured".into())
        )?;
    }
    writeln!(
        out,
        "\nSegments:\n\n| Segment | Run / branch | Hours | Named floor restore bytes | Named floor peak bytes | Floor growth | Current-reader table peak bytes | Restore us/segment | Real segment hours | Restore us/h | Checkpoint raw / zstd-3 bytes |\n|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---|"
    )?;
    report.segments.for_each(|segment| {
        let raw: u64 = segment.checkpoints.iter().map(|c| c.raw_bytes).sum();
        let compressed: u64 = segment.checkpoints.iter().map(|c| c.zstd3_bytes).sum();
        let clean = report
            .union_walk
            .map(|at| report.group_walks[at].segments.read(segment.index))
            .transpose()?
            .flatten();
        writeln!(
            out,
            "| {} | {} / {} | {}..{} | {} | {} | {} | {} | {} | {} | {} | {raw} / {compressed} |",
            segment.index,
            segment.run,
            segment.branch,
            segment.first_hour,
            segment.last_hour,
            segment.union.restore.bytes,
            segment.union.peak.bytes,
            segment.union.growth_bytes,
            segment.union_reader.peak.bytes,
            clean
                .as_ref()
                .and_then(|(_, timing)| timing.restore_us)
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "not measured".into()),
            segment.timing.hours,
            clean
                .as_ref()
                .and_then(|(_, timing)| timing.restore_us_per_hour)
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "not measured".into())
        )?;
        Ok(())
    })?;
    writeln!(
        out,
        "\nShared owners by segment (independent maxima; overlapping trigger-context columns are charged once in the shared union):\n\n| Segment | Owner | Declared restore bytes | Declared peak bytes | Whole floor restore bytes | Whole floor peak bytes | Reader allocation restore bytes | Reader allocation peak bytes |\n|---:|---|---:|---:|---:|---:|---:|---:|"
    )?;
    report.segments.for_each(|segment| {
        for (name, named) in &segment.shared {
            let whole = &segment.shared_whole[name];
            writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} | {} | {} |",
                segment.index,
                name,
                named.restore.bytes,
                named.peak.bytes,
                whole.restore.bytes,
                whole.peak.bytes,
                segment.shared_reader[name].restore.bytes,
                segment.shared_reader[name].peak.bytes
            )?;
        }
        writeln!(
            out,
            "| {} | Charged shared union | {} | {} | n/a | n/a | {} | {} |",
            segment.index,
            segment.shared_union.restore.bytes,
            segment.shared_union.peak.bytes,
            segment.shared_reader_union.restore.bytes,
            segment.shared_reader_union.peak.bytes
        )?;
        Ok(())
    })?;
    writeln!(
        out,
        "\nRecorded content and mod stack, in recorded order:\n\n```json\n{}\n```\n",
        serde_json::to_string_pretty(&report.metadata)?
    )?;
    if report.damage_count != 0 {
        writeln!(
            out,
            "Damage excluded from totals:\n\n| Segment | Covered hours | Scope | Reason |\n|---|---|---|---|"
        )?;
        report.excluded_damage.for_each(|lost| {
            writeln!(
                out,
                "| {:?} | {:?}–{:?} | {} | {} |",
                lost.segment,
                lost.first_hour,
                lost.last_hour,
                escape(&lost.scope),
                escape(&lost.reason)
            )?;
            Ok(())
        })?;
    }
    writeln!(
        out,
        "Encodings and measurement definitions:\n\n```json\n{}\n```",
        serde_json::to_string_pretty(&report.definitions)?
    )?;
    out.flush()
}

fn display(value: Option<u64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "not measured".into())
}
