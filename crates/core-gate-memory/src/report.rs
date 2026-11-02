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
        "unresolved_tables_or_fields":[],"source_discrepancy":"family-naval-regions.md covers minefields and naval-supremacy; it contains no naval battle subscription. naval uses Native::tables() plus Needs.countries, with other direct fields undeclared."})
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

pub fn totals(segments: &Detail<SegmentReport>, lanes: &[Lane], rate: f64) -> io::Result<Value> {
    let mut out = json!({"tables":{},"families":{},"shared":{},"reverse":{},"spools":{},"code_families_whole":{}});
    segments.for_each(|segment| {
        add(&mut out, "included_segments", 1);
        add(&mut out, "included_hours", segment.timing.hours);
        for (name, table) in &segment.tables {
            let entry = &mut out["tables"][name];
            if entry.is_null() {
                *entry = json!({});
            }
            add(entry, "restore_named_bytes_sum", table.named.restore.bytes);
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
        for (kind, values) in [("families", &segment.families), ("shared", &segment.shared)] {
            for (name, peak) in values {
                let entry = &mut out[kind][name];
                if entry.is_null() {
                    *entry = json!({});
                }
                add(entry, "restore_named_bytes_sum", peak.restore.bytes);
                maximum(entry, "maximum_named_peak", peak.peak.bytes, peak)?;
            }
        }
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
        for (name, spool) in &segment.spools {
            let entry = &mut out["spools"][name];
            if entry.is_null() {
                *entry = json!({});
            }
            add(entry, "raw_bytes", spool.raw_bytes);
            add(entry, "zstd3_bytes", spool.zstd3_bytes);
            add(entry, "records", spool.records);
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
    out["restore_us_per_hour"] = json!(us(&out["restore_cycles"]));
    out["hourly_decode_apply_us_per_hour"] = json!(us(&out["hourly_decode_apply_cycles"]));
    for entry in out["reverse"].as_object_mut().unwrap().values_mut() {
        entry["bootstrap_us_per_hour"] = json!(us(&entry["bootstrap_cycles"]));
        entry["upkeep_us_per_hour"] = json!(us(&entry["upkeep_cycles"]));
    }
    for lane in lanes {
        out["spools"][&lane.family]["delivery"] = json!({
            "records":lane.records,"bytes":lane.bytes,"replay_cycles":lane.replay_cycles,
            "replay_us_per_hour":lane.replay_us_per_hour,"fifty_year_seconds":lane.fifty_year_seconds,
            "within_allowance":lane.within_allowance,"segment_detail":lane.segments.path,
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
        "Reader owners against the design's 900,000,000-byte allowance:\n\n| Owner | Maximum bytes |\n|---|---:|\n| Selected union including shared upper bound, undeclared included | {} |\n| Selected union including shared upper bound, undeclared omitted | {} |",
        display(union),
        display(declared)
    )?;
    writeln!(out, "| Readers budget | 900000000 |")?;
    for owner in &report.manifest.shared {
        writeln!(
            out,
            "| Shared {} whole-table upper bound, charged once per admitted segment | {} |",
            owner.name,
            display(
                report.totals["shared"][&owner.name]["maximum_named_peak"]["peak"]["bytes"]
                    .as_u64()
            )
        )?;
    }
    let reverse = report.totals["reverse_maximum_simultaneous_bytes"].as_u64();
    writeln!(
        out,
        "| Reverse allocations, simultaneous peak | {} |\n| Dictionary allocation, reported separately | {} |",
        display(reverse),
        report.totals["dictionary_maximum_bytes"]
    )?;
    writeln!(
        out,
        "| Previous named keys and cells | {} |\n| One-group reader, named keys/cells + reverse + previous | {} |\n",
        display(report.totals["previous_maximum_bytes"].as_u64()),
        display(
            report
                .groupings
                .first()
                .and_then(|s| s.groups.first())
                .map(|g| g.peak_bytes)
        )
    )?;
    writeln!(
        out,
        "The reader floor counts live cells and compact key/slot storage. Actual full-table allocation peaks are also recorded per segment. Each family group includes its direct table duplication, unique reverse indexes and declared previous lookups. The trigger-context whole-table upper bound is separate from group bytes and spools; admission charges it once per distinct segment. Duplicate restored checkpoints are verification artifacts excluded from boundary totals.\n"
    )?;
    writeln!(
        out,
        "The 21 families without notes, ranked by whole-table bytes:\n\n| Family | Tables | Restore bytes, maximum | Peak whole bytes | Share of whole-table union |\n|---|---|---:|---:|---:|"
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
    ranked.sort_by(|a, b| b.3.cmp(&a.3).then(a.0.cmp(b.0)));
    for (name, tables, restore, peak, share) in ranked {
        writeln!(
            out,
            "| {name} | {} | {} | {} | {} |",
            escape(&tables),
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
        "Manifest provenance: {}. Unresolved names: none (validation refuses them). The naval-regions note contains no naval battle subscription; naval retains undeclared fallbacks only for its direct tables without field declarations.\n",
        report.manifest_provenance["origins"]
    )?;
    writeln!(
        out,
        "Decode against the design's 150–400 µs/h:\n\n| Component | Thread CPU µs/h |\n|---|---:|\n| Restore selected tables | {} |\n| Hourly decode plus apply | {} |\n",
        report.totals["restore_us_per_hour"], report.totals["hourly_decode_apply_us_per_hour"]
    )?;
    writeln!(
        out,
        "Every column of every selected table is decoded. Decode and apply are combined. Unnamed columns' cell bytes are reported beside named bytes, at restore and peak, per table and segment in report.json. No per-column CPU is attributed.\n"
    )?;
    if let Some(at) = report.shared_walk {
        let walk = &report.group_walks[at];
        writeln!(
            out,
            "Shared trigger-context independent whole-table walk: {:.3} us/h restore, {:.3} us/h hourly decode plus apply; included once per segment in every scenario's decode accounting.\n",
            walk.restore_us_per_hour, walk.hourly_decode_apply_us_per_hour
        )?;
    }
    writeln!(
        out,
        "Ordered lanes: zstd-3 spool read and no-op delivery only; allowance 29.7 µs/h, 13 s over 438,000 hours:\n\n| Family | Records | µs/h | 50-year seconds | Within allowance |\n|---|---:|---:|---:|---|"
    )?;
    for lane in &report.ordered_lanes {
        writeln!(
            out,
            "| {} | {} | {:.3} | {:.6} | {} |",
            lane.family,
            lane.records,
            lane.replay_us_per_hour,
            lane.fifty_year_seconds,
            lane.within_allowance
        )?;
    }
    writeln!(
        out,
        "\nGroup counts, with largest (segment, group) peaks admitted first:\n\n| Groups | Readers in 0.9 GB | Admitted bytes | Duplicated decode µs/h |\n|---:|---:|---:|---:|"
    )?;
    for grouping in &report.groupings {
        writeln!(
            out,
            "| {} | {} | {} | {} |",
            grouping.count,
            grouping.readers_fitting_largest_first,
            grouping.readers_bytes,
            grouping
                .duplicated_decode_us_per_hour
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "not measured".into())
        )?;
    }
    writeln!(
        out,
        "\nSegments:\n\n| Segment | Run / branch | Hours | Named restore bytes | Named peak bytes | Growth | Checkpoint raw / zstd-3 bytes |\n|---:|---|---|---:|---:|---:|---|"
    )?;
    report.segments.for_each(|segment| {
        let raw: u64 = segment.checkpoints.iter().map(|c| c.raw_bytes).sum();
        let compressed: u64 = segment.checkpoints.iter().map(|c| c.zstd3_bytes).sum();
        writeln!(
            out,
            "| {} | {} / {} | {}–{} | {} | {} | {} | {raw} / {compressed} |",
            segment.index,
            segment.run,
            segment.branch,
            segment.first_hour,
            segment.last_hour,
            segment.union.restore.bytes,
            segment.union.peak.bytes,
            segment.union.growth_bytes
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
