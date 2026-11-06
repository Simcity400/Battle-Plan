use crate::{
    cpu,
    encoding::{self, Encoded, Spool},
    manifest::{Resolved, Selection, invalid},
    memory::{Census, Footprint, Layout, Peak, Trace},
    reverse::{Index, Size},
};
use replay::columnar_frames::{self, Damage, Recording, Segment, Walker};
use replay_columnar::walk::State;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

pub const READER_BUDGET: u64 = 900_000_000;
const TOOL_READER_BUDGET: u64 = 2_200_000_000;
const TABLE_ALLOWANCE: u64 = 1_800_000_000;
const DECODE_ALLOWANCE: u64 = 256_000_000;
const DICTIONARY_ALLOWANCE: u64 = 32_000_000;

pub fn spool_cache_allowance() -> u64 {
    TOOL_READER_BUDGET - TABLE_ALLOWANCE - DECODE_ALLOWANCE - DICTIONARY_ALLOWANCE
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Timing {
    pub restore_cycles: u64,
    pub hourly_decode_apply_cycles: u64,
    pub hours: u64,
    pub hourly_cycles_file: Option<PathBuf>,
    pub tail_cycles: u64,
    pub restore_us: Option<f64>,
    pub restore_us_per_hour: Option<f64>,
    pub hourly_decode_apply_us_per_hour: Option<f64>,
}

impl Timing {
    pub fn calibrate(&mut self, rate: f64) {
        self.restore_us = Some(self.restore_cycles as f64 / rate * 1e6);
        if self.hours != 0 {
            self.restore_us_per_hour =
                Some(self.restore_cycles as f64 / rate * 1e6 / self.hours as f64);
            self.hourly_decode_apply_us_per_hour =
                Some(self.hourly_decode_apply_cycles as f64 / rate * 1e6 / self.hours as f64);
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct TableSize {
    pub named: Peak,
    pub reader_allocation: Peak,
    pub declared_only: Peak,
    pub whole: Peak,
    pub unnamed_cells: Peak,
    pub restore_status: String,
    pub named_peak_status: String,
    pub status_hours: BTreeMap<String, u64>,
}

#[derive(Deserialize, Serialize)]
pub struct ReverseSize {
    pub name: String,
    pub restore: Size,
    pub peak: Size,
    pub peak_hour: i32,
    pub bootstrap_cycles: u64,
    pub upkeep_cycles: u64,
    pub bootstrap_us_per_hour: Option<f64>,
    pub upkeep_us_per_hour: Option<f64>,
}

#[derive(Deserialize, Serialize)]
pub struct SegmentReport {
    pub index: usize,
    pub run: u32,
    pub branch: u64,
    pub first_ordinal: u32,
    pub last_ordinal: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub timing: Timing,
    pub tables: BTreeMap<String, TableSize>,
    pub families: BTreeMap<String, Peak>,
    pub families_primary: BTreeMap<String, Peak>,
    pub families_fallback: BTreeMap<String, Peak>,
    pub families_private: BTreeMap<String, Peak>,
    pub families_private_reader: BTreeMap<String, Peak>,
    pub shared: BTreeMap<String, Peak>,
    pub shared_whole: BTreeMap<String, Peak>,
    pub shared_reader: BTreeMap<String, Peak>,
    pub shared_reader_union: Peak,
    pub shared_union: Peak,
    pub shared_without_trigger: Peak,
    pub code_families_whole: BTreeMap<String, Peak>,
    pub code_families_union_share: BTreeMap<String, (Option<f64>, Option<f64>)>,
    pub union: Peak,
    pub union_reader: Peak,
    pub declared_reader: Peak,
    pub declared_only: Peak,
    pub reverse: Vec<ReverseSize>,
    pub checkpoints: Vec<Encoded>,
    pub restore_checkpoint_verification: Option<Encoded>,
    pub spools: BTreeMap<String, Spools>,
    pub spool_index_cache: crate::keyset::CacheSize,
    pub dictionary_peak_bytes: u64,
    pub reader_auxiliary_peak_bytes: u64,
    pub reverse_peak_bytes: u64,
    pub previous_peak_bytes: u64,
    pub previous_reader_peak_bytes: u64,
    pub actual_table_storage_peaks: BTreeMap<String, u64>,
    pub trace: PathBuf,
}

#[derive(Deserialize, Serialize)]
pub struct Spools {
    pub full: Encoded,
    pub filtered: Encoded,
    pub trigger_context_upper_bound: Option<Encoded>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Lost {
    pub segment: Option<usize>,
    pub run: Option<u32>,
    pub first_ordinal: Option<u32>,
    pub last_ordinal: Option<u32>,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub scope: String,
    pub reason: String,
}

fn full(selected: &Selection) -> Selection {
    Selection {
        fields: (0..selected.table.schema().len()).collect(),
        recorded_only: Default::default(),
        ..selected.clone()
    }
}

fn unnamed(census: &Census, selected: &Selection) -> Footprint {
    let cells = census.footprint(&[full(selected)]).cells_bytes
        - census.footprint(std::slice::from_ref(selected)).cells_bytes;
    Footprint {
        cells_bytes: cells,
        bytes: cells,
        live_slots: census.live[&selected.table],
        ..Footprint::default()
    }
}

fn fallback_footprint(census: &Census, selected: &[Selection], primary: &[Selection]) -> Footprint {
    let cells = census.footprint(selected).cells_bytes - census.footprint(primary).cells_bytes;
    Footprint {
        cells_bytes: cells,
        bytes: cells,
        ..Footprint::default()
    }
}

fn shared_footprint(census: &Census, selected: &[Selection], indexes: &[usize]) -> Footprint {
    let mut value = census.footprint(selected);
    value.index_bytes = indexes.iter().map(|&at| census.values[at]).sum();
    value.bytes += value.index_bytes;
    value
}

fn reader_shared_footprint(
    census: &Census,
    selected: &[Selection],
    indexes: &[usize],
) -> Footprint {
    let mut value = census.reader_footprint(selected);
    value.index_bytes = indexes.iter().map(|&at| census.values[at]).sum();
    value.bytes += value.index_bytes;
    value
}

pub fn measure(
    index: usize,
    recording: &Recording,
    resolved: &Resolved,
    layout: &Layout,
    format: replay_data::format::FormatVersion,
    out: &Path,
    projects_scoped: Option<bool>,
    cache: &crate::keyset::Cache,
) -> io::Result<SegmentReport> {
    let segment = &recording.segments()[index];
    let run = recording.run(segment.run)?;
    let first_hour = i32::try_from(i64::from(run.first_hour) + i64::from(segment.first))
        .map_err(|_| invalid("Segment first hour exceeds i32"))?;
    let last_hour = i32::try_from(i64::from(run.first_hour) + i64::from(segment.last))
        .map_err(|_| invalid("Segment last hour exceeds i32"))?;
    let folder = out.join(format!("segment-{index:05}"));
    std::fs::create_dir(&folder)?;
    let mut state = State::new(columnar_frames::schema()?)?;
    let mut walker = Walker::new(recording)?;
    let start = cpu::cycles()?;
    walker.begin(segment, &mut state)?;
    let mut timing = Timing {
        restore_cycles: cpu::cycles()?.wrapping_sub(start),
        ..Timing::default()
    };
    let mut census = Census::new(layout.clone(), &columnar_frames::schema()?);
    census.observe(&state, true)?;
    let shared_dimensions = |selected: &[Selection]| {
        resolved
            .shared_relations
            .iter()
            .filter(|&&at| {
                resolved.relations[at]
                    .steps
                    .iter()
                    .any(|(table, _)| selected.iter().any(|s| s.table == *table))
            })
            .map(|at| layout.field_dimensions() + at)
            .collect::<Vec<_>>()
    };
    let owner_indexes: Vec<_> = resolved
        .shared
        .iter()
        .map(|s| shared_dimensions(s))
        .collect();
    let union_indexes = shared_dimensions(&resolved.shared_union);
    let without_trigger_indexes = shared_dimensions(&resolved.shared_without_trigger);
    let mut indexes: Vec<_> = resolved.relations.iter().map(Index::new).collect();
    let reverse_at = layout.field_dimensions();
    let mut reverse = Vec::new();
    for (at, (relation, index)) in resolved.relations.iter().zip(&mut indexes).enumerate() {
        let start = cpu::cycles()?;
        index.update(relation, &state, true)?;
        let cycles = cpu::cycles()?.wrapping_sub(start);
        census.values[reverse_at + at] = index.size.bytes;
        reverse.push(ReverseSize {
            name: relation.name.clone(),
            restore: index.size.clone(),
            peak: index.size.clone(),
            peak_hour: first_hour - 1,
            bootstrap_cycles: cycles,
            upkeep_cycles: 0,
            bootstrap_us_per_hour: None,
            upkeep_us_per_hour: None,
        });
    }
    let mut tables = BTreeMap::new();
    let mut reverse_peak_bytes: u64 = indexes.iter().map(|i| i.size.bytes).sum();
    let mut previous_peak_bytes = 0;
    let mut previous_reader_peak_bytes = 0;
    for selected in &resolved.union {
        let declared: Vec<_> = resolved
            .declared
            .iter()
            .filter(|s| s.table == selected.table)
            .cloned()
            .collect();
        tables.insert(
            format!("{:?}", selected.table),
            TableSize {
                named: Peak::new(
                    census.footprint(std::slice::from_ref(selected)),
                    first_hour - 1,
                ),
                reader_allocation: Peak::new(
                    census.reader_footprint(std::slice::from_ref(selected)),
                    first_hour - 1,
                ),
                declared_only: Peak::new(census.footprint(&declared), first_hour - 1),
                whole: Peak::new(census.footprint(&[full(selected)]), first_hour - 1),
                unnamed_cells: Peak::new(unnamed(&census, selected), first_hour - 1),
                restore_status: format!("{:?}", replay::columns::status(&state, selected.table)),
                named_peak_status: format!("{:?}", replay::columns::status(&state, selected.table)),
                status_hours: BTreeMap::new(),
            },
        );
    }
    let mut families: BTreeMap<_, _> = resolved
        .manifest
        .families
        .iter()
        .zip(&resolved.families)
        .map(|(f, s)| {
            (
                f.name.clone(),
                Peak::new(census.footprint(s), first_hour - 1),
            )
        })
        .collect();
    let mut shared: BTreeMap<_, _> = resolved
        .manifest
        .shared
        .iter()
        .zip(&resolved.shared)
        .zip(&owner_indexes)
        .map(|((owner, selected), indexes)| {
            (
                owner.name.clone(),
                Peak::new(shared_footprint(&census, selected, indexes), first_hour - 1),
            )
        })
        .collect();
    let family_peaks = |selections: &[Vec<Selection>]| {
        resolved
            .manifest
            .families
            .iter()
            .zip(selections)
            .map(|(family, selected)| {
                (
                    family.name.clone(),
                    Peak::new(census.footprint(selected), first_hour - 1),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let mut families_primary = family_peaks(&resolved.primary);
    let mut families_private = family_peaks(&resolved.private);
    let mut families_private_reader: BTreeMap<_, _> = resolved
        .manifest
        .families
        .iter()
        .zip(&resolved.private)
        .map(|(family, selected)| {
            (
                family.name.clone(),
                Peak::new(census.reader_footprint(selected), first_hour - 1),
            )
        })
        .collect();
    let mut shared_reader: BTreeMap<_, _> = resolved
        .manifest
        .shared
        .iter()
        .zip(&resolved.shared)
        .zip(&owner_indexes)
        .map(|((owner, selected), indexes)| {
            (
                owner.name.clone(),
                Peak::new(
                    reader_shared_footprint(&census, selected, indexes),
                    first_hour - 1,
                ),
            )
        })
        .collect();
    let mut shared_reader_union = Peak::new(
        reader_shared_footprint(&census, &resolved.shared_union, &union_indexes),
        first_hour - 1,
    );
    let mut reader_auxiliary_peak_bytes = census.values[layout.reader_auxiliary];
    let mut families_fallback: BTreeMap<_, _> = resolved
        .manifest
        .families
        .iter()
        .enumerate()
        .map(|(at, family)| {
            (
                family.name.clone(),
                Peak::new(
                    fallback_footprint(&census, &resolved.families[at], &resolved.primary[at]),
                    first_hour - 1,
                ),
            )
        })
        .collect();
    let whole_shared: Vec<Vec<_>> = resolved
        .shared
        .iter()
        .map(|s| s.iter().map(full).collect())
        .collect();
    let mut shared_whole: BTreeMap<_, _> = resolved
        .manifest
        .shared
        .iter()
        .zip(&whole_shared)
        .zip(&owner_indexes)
        .map(|((owner, s), indexes)| {
            (
                owner.name.clone(),
                Peak::new(shared_footprint(&census, s, indexes), first_hour - 1),
            )
        })
        .collect();
    let mut shared_union = Peak::new(
        shared_footprint(&census, &resolved.shared_union, &union_indexes),
        first_hour - 1,
    );
    let mut shared_without_trigger = Peak::new(
        shared_footprint(
            &census,
            &resolved.shared_without_trigger,
            &without_trigger_indexes,
        ),
        first_hour - 1,
    );
    let whole_families: Vec<_> = resolved
        .manifest
        .families
        .iter()
        .zip(&resolved.families)
        .filter(|(f, _)| f.origin != "note")
        .map(|(f, s)| (f.name.clone(), s.iter().map(full).collect::<Vec<_>>()))
        .collect();
    let mut code_families_whole: BTreeMap<_, _> = whole_families
        .iter()
        .map(|(name, s)| (name.clone(), Peak::new(census.footprint(s), first_hour - 1)))
        .collect();
    let whole_union: Vec<_> = resolved.union.iter().map(full).collect();
    let union_bytes = census.footprint(&whole_union).bytes;
    let mut code_families_union_share: BTreeMap<_, _> = whole_families
        .iter()
        .map(|(name, s)| {
            let share =
                (union_bytes != 0).then(|| census.footprint(s).bytes as f64 / union_bytes as f64);
            (name.clone(), (share, share))
        })
        .collect();
    let mut union = Peak::new(census.footprint(&resolved.union), first_hour - 1);
    let mut union_reader = Peak::new(census.reader_footprint(&resolved.union), first_hour - 1);
    let mut declared_reader =
        Peak::new(census.reader_footprint(&resolved.declared), first_hour - 1);
    let mut declared_only = Peak::new(census.footprint(&resolved.declared), first_hour - 1);
    let mut checkpoints = vec![encoding::checkpoint(
        &state,
        &resolved.union,
        first_hour - 1,
        &folder.join("restore.checkpoint"),
    )?];
    let mut spools: Vec<_> = resolved
        .manifest
        .families
        .iter()
        .zip(&resolved.families)
        .zip(&resolved.readsets)
        .filter(|((f, _), _)| f.ordered)
        .map(|((f, s), readset)| {
            let mut spool = Spool::new(&folder.join(format!("{}.spool", f.name)), s)?;
            let mut filtered = Spool::filtered(
                &folder.join(format!("{}.filtered.spool", f.name)),
                readset
                    .as_ref()
                    .ok_or_else(|| invalid("Ordered family lacks a read set"))?,
                format,
                cache,
            )?;
            spool.restored(&state, first_hour - 1)?;
            filtered.restored(&state, first_hour - 1)?;
            let hours = match f.name.as_str() {
                "politics_ledger" => Some(crate::readset::Hours::LedgerDated),
                "projects" if projects_scoped == Some(true) => {
                    Some(crate::readset::Hours::ScopedProjects)
                }
                _ => None,
            };
            let upper_bound = hours
                .map(|hours| {
                    let trigger = resolved
                        .manifest
                        .shared
                        .iter()
                        .position(|owner| owner.name == "trigger-context")
                        .ok_or_else(|| invalid("Missing trigger-context owner"))?;
                    let whole: Vec<_> = resolved.shared[trigger].iter().map(full).collect();
                    Spool::filtered(
                        &folder.join(format!("{}.trigger-context-upper-bound.spool", f.name)),
                        &crate::readset::ReadSet::upper_bound(&whole, hours),
                        format,
                        cache,
                    )
                })
                .transpose()?;
            Ok((f.name.clone(), spool, filtered, upper_bound))
        })
        .collect::<io::Result<_>>()?;
    let trace = folder.join("census.bin");
    let mut trace_writer =
        Trace::new(BufWriter::new(File::create_new(&trace)?), layout.dimensions)?;
    trace_writer.snapshot(first_hour - 1, &census.values)?;
    let hourly_path = folder.join("hourly-cycles.jsonl");
    let mut hourly = BufWriter::new(File::create_new(&hourly_path)?);
    timing.hourly_cycles_file = Some(hourly_path);
    let mut before = cpu::cycles()?;
    let reads = crate::reads::Reads::new(format);
    walker.walk(segment, &mut state, |state, manifest, first| {
        let cycles = cpu::cycles()?.wrapping_sub(before);
        timing.hourly_decode_apply_cycles += cycles;
        serde_json::to_writer(&mut hourly, &(manifest.hour, cycles))?;
        writeln!(hourly)?;
        timing.hours += 1;
        for (&source, &target) in &layout.previous {
            census.values[target] = census.values[source];
        }
        census.observe(state, false)?;
        reader_auxiliary_peak_bytes =
            reader_auxiliary_peak_bytes.max(census.values[layout.reader_auxiliary]);
        for (at, (relation, index)) in resolved.relations.iter().zip(&mut indexes).enumerate() {
            let start = cpu::cycles()?;
            index.update(relation, state, false)?;
            reverse[at].upkeep_cycles += cpu::cycles()?.wrapping_sub(start);
            census.values[reverse_at + at] = index.size.bytes;
            if index.size.bytes > reverse[at].peak.bytes {
                reverse[at].peak = index.size.clone();
                reverse[at].peak_hour = manifest.hour;
            }
        }
        trace_writer.snapshot(manifest.hour, &census.values)?;
        reverse_peak_bytes = reverse_peak_bytes.max(indexes.iter().map(|i| i.size.bytes).sum());
        previous_peak_bytes = previous_peak_bytes.max(
            layout
                .previous
                .iter()
                .filter(|(source, _)| **source < layout.field_dimensions())
                .map(|(_, &at)| census.values[at])
                .sum(),
        );
        previous_reader_peak_bytes = previous_reader_peak_bytes.max(
            layout
                .previous
                .iter()
                .filter(|(source, _)| **source >= layout.field_dimensions())
                .map(|(_, &at)| census.values[at])
                .sum(),
        );
        for selected in &resolved.union {
            let report = tables.get_mut(&format!("{:?}", selected.table)).unwrap();
            let declared: Vec<_> = resolved
                .declared
                .iter()
                .filter(|s| s.table == selected.table)
                .cloned()
                .collect();
            let named = census.footprint(std::slice::from_ref(selected));
            let status = format!("{:?}", replay::columns::status(state, selected.table));
            *report.status_hours.entry(status.clone()).or_default() += 1;
            if named.bytes > report.named.peak.bytes {
                report.named_peak_status = status;
            }
            report.named.observe(named, manifest.hour);
            report.reader_allocation.observe(
                census.reader_footprint(std::slice::from_ref(selected)),
                manifest.hour,
            );
            report
                .declared_only
                .observe(census.footprint(&declared), manifest.hour);
            report
                .whole
                .observe(census.footprint(&[full(selected)]), manifest.hour);
            report
                .unnamed_cells
                .observe(unnamed(&census, selected), manifest.hour);
        }
        for (family, selected) in resolved.manifest.families.iter().zip(&resolved.families) {
            families
                .get_mut(&family.name)
                .unwrap()
                .observe(census.footprint(selected), manifest.hour);
        }
        for (at, family) in resolved.manifest.families.iter().enumerate() {
            families_private_reader
                .get_mut(&family.name)
                .unwrap()
                .observe(
                    census.reader_footprint(&resolved.private[at]),
                    manifest.hour,
                );
            families_primary
                .get_mut(&family.name)
                .unwrap()
                .observe(census.footprint(&resolved.primary[at]), manifest.hour);
            families_private
                .get_mut(&family.name)
                .unwrap()
                .observe(census.footprint(&resolved.private[at]), manifest.hour);
            families_fallback.get_mut(&family.name).unwrap().observe(
                fallback_footprint(&census, &resolved.families[at], &resolved.primary[at]),
                manifest.hour,
            );
        }
        for ((owner, selected), indexes) in resolved
            .manifest
            .shared
            .iter()
            .zip(&resolved.shared)
            .zip(&owner_indexes)
        {
            shared_reader.get_mut(&owner.name).unwrap().observe(
                reader_shared_footprint(&census, selected, indexes),
                manifest.hour,
            );
            shared
                .get_mut(&owner.name)
                .unwrap()
                .observe(shared_footprint(&census, selected, indexes), manifest.hour);
        }
        for ((owner, selected), indexes) in resolved
            .manifest
            .shared
            .iter()
            .zip(&whole_shared)
            .zip(&owner_indexes)
        {
            shared_whole
                .get_mut(&owner.name)
                .unwrap()
                .observe(shared_footprint(&census, selected, indexes), manifest.hour);
        }
        shared_reader_union.observe(
            reader_shared_footprint(&census, &resolved.shared_union, &union_indexes),
            manifest.hour,
        );
        shared_union.observe(
            shared_footprint(&census, &resolved.shared_union, &union_indexes),
            manifest.hour,
        );
        shared_without_trigger.observe(
            shared_footprint(
                &census,
                &resolved.shared_without_trigger,
                &without_trigger_indexes,
            ),
            manifest.hour,
        );
        let union_bytes = census.footprint(&whole_union).bytes;
        for (name, selected) in &whole_families {
            let value = census.footprint(selected);
            let peak = code_families_whole.get_mut(name).unwrap();
            if value.bytes > peak.peak.bytes {
                code_families_union_share.get_mut(name).unwrap().1 =
                    (union_bytes != 0).then_some(value.bytes as f64 / union_bytes as f64);
            }
            peak.observe(value, manifest.hour);
        }
        union.observe(census.footprint(&resolved.union), manifest.hour);
        union_reader.observe(census.reader_footprint(&resolved.union), manifest.hour);
        declared_reader.observe(census.reader_footprint(&resolved.declared), manifest.hour);
        declared_only.observe(census.footprint(&resolved.declared), manifest.hour);
        let supplied = reads.supplied(manifest);
        for (_, spool, filtered, upper_bound) in &mut spools {
            spool.hour(state, manifest, first, &supplied)?;
            filtered.hour(state, manifest, first, &supplied)?;
            if let Some(upper_bound) = upper_bound {
                upper_bound.hour(state, manifest, first, &supplied)?;
            }
        }
        before = cpu::cycles()?;
        Ok(true)
    })?;
    timing.tail_cycles = cpu::cycles()?.wrapping_sub(before);
    timing.hourly_decode_apply_cycles += timing.tail_cycles;
    if timing.hours != u64::from(segment.last) - u64::from(segment.first) + 1 {
        return Err(invalid("Walker delivered a short segment"));
    }
    hourly.flush()?;
    trace_writer.finish()?;
    checkpoints.push(encoding::checkpoint(
        &state,
        &resolved.union,
        last_hour,
        &folder.join("end.checkpoint"),
    )?);
    let spools = spools
        .into_iter()
        .map(|(name, spool, filtered, upper_bound)| {
            Ok((
                name,
                Spools {
                    full: spool.finish()?,
                    filtered: filtered.finish()?,
                    trigger_context_upper_bound: upper_bound.map(Spool::finish).transpose()?,
                },
            ))
        })
        .collect::<io::Result<_>>()?;
    Ok(SegmentReport {
        index,
        run: segment.run,
        branch: run.branch,
        first_ordinal: segment.first,
        last_ordinal: segment.last,
        first_hour,
        last_hour,
        timing,
        tables,
        families,
        families_primary,
        families_fallback,
        families_private,
        families_private_reader,
        shared,
        shared_whole,
        shared_reader,
        shared_reader_union,
        shared_union,
        shared_without_trigger,
        code_families_whole,
        code_families_union_share,
        union,
        union_reader,
        declared_reader,
        declared_only,
        reverse,
        checkpoints,
        restore_checkpoint_verification: None,
        spools,
        spool_index_cache: cache.size(),
        dictionary_peak_bytes: census.dictionary_peak,
        reader_auxiliary_peak_bytes,
        reverse_peak_bytes,
        previous_peak_bytes,
        previous_reader_peak_bytes,
        actual_table_storage_peaks: census
            .held_peak
            .into_iter()
            .map(|(t, b)| (format!("{t:?}"), b))
            .collect(),
        trace,
    })
}

pub fn timed_walk(recording: &Recording, segment: &Segment) -> io::Result<Timing> {
    let mut state = State::new(columnar_frames::schema()?)?;
    let mut walker = Walker::new(recording)?;
    let start = cpu::cycles()?;
    walker.begin(segment, &mut state)?;
    let mut timing = Timing {
        restore_cycles: cpu::cycles()?.wrapping_sub(start),
        ..Timing::default()
    };
    let mut before = cpu::cycles()?;
    walker.walk(segment, &mut state, |_, _, _| {
        timing.hourly_decode_apply_cycles += cpu::cycles()?.wrapping_sub(before);
        timing.hours += 1;
        before = cpu::cycles()?;
        Ok(true)
    })?;
    timing.tail_cycles = cpu::cycles()?.wrapping_sub(before);
    timing.hourly_decode_apply_cycles += timing.tail_cycles;
    if timing.hours != u64::from(segment.last) - u64::from(segment.first) + 1 {
        return Err(invalid("Timed walker delivered a short segment"));
    }
    Ok(timing)
}

pub fn damage(
    index: usize,
    segment: &Segment,
    recording: &Recording,
    error: &io::Error,
) -> Option<Lost> {
    if error.kind() != io::ErrorKind::InvalidData {
        return None;
    }
    let scope = Damage::of(error)
        .map(|damage| damage.scope.to_string())
        .unwrap_or_else(|| "Decoded state of the segment".into());
    let first = recording
        .run(segment.run)
        .ok()
        .map(|r| i64::from(r.first_hour));
    Some(Lost {
        segment: Some(index),
        run: Some(segment.run),
        first_ordinal: Some(segment.first),
        last_ordinal: Some(segment.last),
        first_hour: first.and_then(|h| i32::try_from(h + i64::from(segment.first)).ok()),
        last_hour: first.and_then(|h| i32::try_from(h + i64::from(segment.last)).ok()),
        scope,
        reason: error.to_string(),
    })
}

pub fn parallel(
    count: usize,
    threads: usize,
    work: impl Fn(usize) -> io::Result<()> + Sync,
) -> io::Result<()> {
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| -> io::Result<()> {
        let mut handles = Vec::new();
        for _ in 0..threads.min(count) {
            let next = &next;
            let work = &work;
            handles.push(scope.spawn(move || -> io::Result<()> {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= count {
                        return Ok(());
                    }
                    work(index)?;
                }
            }));
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_| io::Error::other("Gate worker panicked"))??;
        }
        Ok(())
    })
}

pub struct Admission {
    held: Mutex<u64>,
    changed: std::sync::Condvar,
    pub active: AtomicUsize,
    pub peak: AtomicUsize,
}
pub struct Permit<'a> {
    admission: &'a Admission,
    bytes: u64,
    spool_cache_bytes: u64,
}

impl Admission {
    pub fn new() -> Self {
        Self {
            held: Mutex::new(0),
            changed: std::sync::Condvar::new(),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }
    pub fn enter(&self, bytes: u64) -> io::Result<Permit<'_>> {
        if bytes > TOOL_READER_BUDGET {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!(
                    "A measured reader needs {bytes} bytes of the tool's {TOOL_READER_BUDGET}-byte reader allowance"
                ),
            ));
        }
        let mut held = self
            .held
            .lock()
            .map_err(|_| io::Error::other("Admission mutex poisoned"))?;
        while *held + bytes > TOOL_READER_BUDGET {
            held = self
                .changed
                .wait(held)
                .map_err(|_| io::Error::other("Admission mutex poisoned"))?;
        }
        *held += bytes;
        let active = self.active.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak.fetch_max(active, Ordering::Relaxed);
        Ok(Permit {
            admission: self,
            bytes,
            spool_cache_bytes: 0,
        })
    }
    pub fn union(&self) -> io::Result<Permit<'_>> {
        self.enter(TOOL_READER_BUDGET)
    }
}

impl Permit<'_> {
    pub fn spool_cache(&mut self) -> io::Result<crate::keyset::Cache> {
        if self.bytes != TOOL_READER_BUDGET || self.spool_cache_bytes != 0 {
            return Err(invalid(
                "Spool scratch cache requires an unpartitioned exclusive union permit",
            ));
        }
        self.spool_cache_bytes = spool_cache_allowance();
        crate::keyset::Cache::reserved(
            "spool scratch page cache",
            usize::try_from(self.spool_cache_bytes)
                .map_err(|_| invalid("Spool cache exceeds address space"))?,
        )
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut held = self
            .admission
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *held -= self.bytes;
        self.admission.active.fetch_sub(1, Ordering::Relaxed);
        self.admission.changed.notify_all();
    }
}

#[derive(Default)]
pub struct Boundaries {
    end: Option<(u32, u32)>,
}
impl Boundaries {
    pub fn restore_is_duplicate(&mut self, run: u32, first: u32, last: u32) -> bool {
        let duplicate = self.end == Some((run, first));
        self.end = last.checked_add(1).map(|end| (run, end));
        duplicate
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn restore_cost_uses_each_segments_real_length() {
        let mut short = Timing {
            restore_cycles: 64_800_000,
            hours: 720,
            ..Timing::default()
        };
        let mut long = Timing {
            hours: 2160,
            ..short.clone()
        };
        short.calibrate(1_000_000_000.0);
        long.calibrate(1_000_000_000.0);
        assert_eq!(short.restore_us, Some(64_800.0));
        assert_eq!(long.restore_us, short.restore_us);
        assert_eq!(short.restore_us_per_hour, Some(90.0));
        assert_eq!(long.restore_us_per_hour, Some(30.0));
        let mut empty = Timing::default();
        empty.calibrate(1_000_000_000.0);
        assert_eq!(empty.restore_us_per_hour, None);
    }

    #[test]
    fn fifty_three_contiguous_segments_have_fifty_four_boundaries() {
        let mut boundaries = Boundaries::default();
        let count: usize = (0..53)
            .map(|i| {
                1 + usize::from(!boundaries.restore_is_duplicate(0, i * 2160, (i + 1) * 2160 - 1))
            })
            .sum();
        assert_eq!(count, 54);
        assert!(!boundaries.restore_is_duplicate(0, 54 * 2160, 55 * 2160 - 1));
        assert!(!boundaries.restore_is_duplicate(1, 55 * 2160, 56 * 2160 - 1));
    }
}
