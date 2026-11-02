use crate::{
    detail::Detail,
    gate::{self, Admission, READER_BUDGET, SegmentReport, Timing},
    manifest::Resolved,
    memory::{self, Layout},
};
use replay::columnar_frames::Recording;
use replay_data::rows::TableId;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Serialize)]
pub struct Group {
    pub families: Vec<String>,
    pub tables: Vec<String>,
    pub peak_bytes: u64,
    pub timing: usize,
    #[serde(skip)]
    dimensions: Vec<usize>,
    #[serde(skip)]
    table_ids: Vec<TableId>,
}

#[derive(Serialize)]
pub struct Scenario {
    pub count: usize,
    pub groups: Vec<Group>,
    pub readers_fitting_largest_first: usize,
    pub readers_bytes: u64,
    pub admitted_shared_bytes: u64,
    #[serde(rename = "segment_peaks", serialize_with = "serialize_peaks")]
    pub segment_peaks_file: PathBuf,
    pub duplicated_decode_us_per_hour: Option<f64>,
    pub duplicated_restore_us_per_hour: Option<f64>,
}

#[derive(Serialize)]
pub struct Walk {
    pub tables: Vec<String>,
    pub segments: Detail<(usize, Timing)>,
    pub restore_us_per_hour: f64,
    pub hourly_decode_apply_us_per_hour: f64,
}

pub fn plans(
    resolved: &Resolved,
    layout: &Layout,
    segments: &Detail<SegmentReport>,
    out: &Path,
) -> io::Result<Vec<Scenario>> {
    let mut weights = vec![0; resolved.families.len()];
    segments.for_each(|segment| {
        for (at, family) in resolved.manifest.families.iter().enumerate() {
            weights[at] = weights[at].max(segment.families[&family.name].peak.bytes);
        }
        Ok(())
    })?;
    let mut order: Vec<_> = (0..weights.len()).collect();
    order.sort_by(|&a, &b| {
        weights[b].cmp(&weights[a]).then(
            resolved.manifest.families[a]
                .name
                .cmp(&resolved.manifest.families[b].name),
        )
    });
    let mut scenarios = Vec::new();
    for count in 1..=order.len() {
        let mut assignments = vec![Vec::new(); count];
        let mut totals = vec![0_u64; count];
        for &family in &order {
            let at = (0..count)
                .min_by_key(|&at| (totals[at], assignments[at].len(), at))
                .unwrap();
            assignments[at].push(family);
            totals[at] += weights[family];
        }
        let groups = assignments
            .into_iter()
            .map(|families| {
                let mut dimensions = BTreeSet::new();
                let mut tables = BTreeSet::new();
                for &family in &families {
                    dimensions.extend(layout.indexes(&resolved.families[family]));
                    let previous: Vec<_> = resolved.families[family]
                        .iter()
                        .filter(|s| s.previous)
                        .cloned()
                        .collect();
                    dimensions.extend(
                        layout
                            .indexes(&previous)
                            .iter()
                            .map(|at| layout.previous[at]),
                    );
                    dimensions.extend(
                        resolved.family_relations[family]
                            .iter()
                            .map(|at| layout.field_dimensions() + at),
                    );
                    tables.extend(resolved.families[family].iter().map(|s| s.table));
                }
                let table_ids: Vec<_> = tables.into_iter().collect();
                Group {
                    families: families
                        .into_iter()
                        .map(|at| resolved.manifest.families[at].name.clone())
                        .collect(),
                    tables: table_ids.iter().map(|t| format!("{t:?}")).collect(),
                    table_ids,
                    dimensions: dimensions.into_iter().collect(),
                    peak_bytes: 0,
                    timing: 0,
                }
            })
            .collect();
        scenarios.push(Scenario {
            count,
            groups,
            readers_fitting_largest_first: 0,
            readers_bytes: 0,
            admitted_shared_bytes: 0,
            segment_peaks_file: out.join(format!("grouping-{count:02}-peaks.jsonl")),
            duplicated_decode_us_per_hour: None,
            duplicated_restore_us_per_hour: None,
        });
    }
    Ok(scenarios)
}

#[derive(Clone, Copy, serde::Deserialize, Serialize)]
struct Candidate {
    bytes: u64,
    segment: usize,
    group: usize,
    shared: u64,
}
impl Candidate {
    fn key(self) -> (u64, usize, usize) {
        (self.bytes, self.segment, self.group)
    }
}

fn serialize_peaks<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::{Error, SerializeSeq};
    let mut sequence = serializer.serialize_seq(None)?;
    for line in BufReader::new(File::open(path).map_err(S::Error::custom)?).lines() {
        let candidate: Candidate =
            serde_json::from_str(&line.map_err(S::Error::custom)?).map_err(S::Error::custom)?;
        sequence.serialize_element(&candidate)?;
    }
    sequence.end()
}

fn fit_file(path: &Path, segment_slots: usize) -> io::Result<(usize, u64, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut charged = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path.with_extension("admitted.bin"))?;
    charged.set_len(segment_slots as u64)?;
    let (mut count, mut bytes, mut shared_bytes) = (0, 0, 0);
    let mut after = None;
    loop {
        let mut batch = std::collections::BinaryHeap::new();
        let mut candidates = BTreeMap::new();
        for line in BufReader::new(File::open(path)?).lines() {
            let candidate: Candidate = serde_json::from_str(&line?)?;
            let key = candidate.key();
            if after.is_some_and(|last| key >= last) {
                continue;
            }
            batch.push(std::cmp::Reverse(key));
            candidates.insert(key, candidate);
            if batch.len() > 1024 {
                candidates.remove(&batch.pop().unwrap().0);
            }
        }
        if candidates.is_empty() {
            return Ok((count, bytes, shared_bytes));
        }
        for (_, candidate) in candidates.into_iter().rev() {
            charged.seek(SeekFrom::Start(candidate.segment as u64))?;
            let mut flag = [0];
            charged.read_exact(&mut flag)?;
            let shared = if flag[0] == 0 { candidate.shared } else { 0 };
            let cost = candidate
                .bytes
                .checked_add(shared)
                .ok_or_else(|| io::Error::other("Admission overflow"))?;
            if cost > READER_BUDGET - bytes {
                return Ok((count, bytes, shared_bytes));
            }
            charged.seek(SeekFrom::Start(candidate.segment as u64))?;
            charged.write_all(&[1])?;
            bytes += cost;
            shared_bytes += shared;
            count += 1;
            after = Some(candidate.key());
        }
    }
}

pub fn peaks(
    plans: &mut [Scenario],
    layout: &Layout,
    segments: &Detail<SegmentReport>,
) -> io::Result<()> {
    let mut outputs: Vec<_> = plans
        .iter()
        .map(|p| Ok(BufWriter::new(File::create_new(&p.segment_peaks_file)?)))
        .collect::<io::Result<_>>()?;
    let mut dimensions = vec![Vec::new(); layout.dimensions];
    for (scenario, plan) in plans.iter().enumerate() {
        for (group, held) in plan.groups.iter().enumerate() {
            for &dim in &held.dimensions {
                dimensions[dim].push((scenario, group));
            }
        }
    }
    segments.for_each(|segment| {
        let mut current: Vec<Vec<u64>> = plans.iter().map(|p| vec![0; p.count]).collect();
        let mut maxima = current.clone();
        let mut previous = vec![0; layout.dimensions];
        memory::read_trace(
            BufReader::new(File::open(&segment.trace)?),
            layout.dimensions,
            |_, values| {
                for (dim, (&now, old)) in values.iter().zip(&mut previous).enumerate() {
                    if *old == now {
                        continue;
                    }
                    for &(plan, group) in &dimensions[dim] {
                        current[plan][group] = current[plan][group] - *old + now;
                    }
                    *old = now;
                }
                for (maximum, current) in maxima.iter_mut().zip(&current) {
                    for (m, &c) in maximum.iter_mut().zip(current) {
                        *m = (*m).max(c);
                    }
                }
                Ok(())
            },
        )?;
        let shared = segment.shared.values().map(|p| p.peak.bytes).sum();
        for ((plan, maxima), out) in plans.iter_mut().zip(maxima).zip(&mut outputs) {
            for (at, (group, bytes)) in plan.groups.iter_mut().zip(maxima).enumerate() {
                group.peak_bytes = group.peak_bytes.max(bytes);
                serde_json::to_writer(
                    &mut *out,
                    &Candidate {
                        bytes,
                        segment: segment.index,
                        group: at,
                        shared,
                    },
                )?;
                writeln!(out)?;
            }
        }
        Ok(())
    })?;
    for out in &mut outputs {
        out.flush()?;
    }
    drop(outputs);
    for plan in plans {
        let (count, bytes, shared) = fit_file(&plan.segment_peaks_file, segments.slots)?;
        plan.readers_fitting_largest_first = count;
        plan.readers_bytes = bytes;
        plan.admitted_shared_bytes = shared;
    }
    Ok(())
}

pub fn walks(
    plans: &mut [Scenario],
    segments: &Detail<SegmentReport>,
    columnar: &Path,
    threads: usize,
    rate: f64,
    admission: &Admission,
    union_tables: &[TableId],
    shared_tables: &[TableId],
    out: &Path,
    hours: u64,
    union_timing: &Timing,
) -> io::Result<(Vec<Walk>, usize)> {
    let mut unique = BTreeMap::<Vec<TableId>, usize>::new();
    for plan in plans.iter_mut() {
        for group in &mut plan.groups {
            let next = unique.len();
            group.timing = *unique.entry(group.table_ids.clone()).or_insert(next);
        }
    }
    let next = unique.len();
    let shared_timing = *unique.entry(shared_tables.to_vec()).or_insert(next);
    let mut tables = vec![Vec::new(); unique.len()];
    for (ids, at) in unique {
        tables[at] = ids;
    }
    let walks: Vec<_> = tables
        .iter()
        .enumerate()
        .map(|(at, ids)| {
            Ok(Walk {
                tables: ids.iter().map(|t| format!("{t:?}")).collect(),
                segments: Detail::new(&out.join(format!("walk-{at:04}")), segments.slots)?,
                restore_us_per_hour: 0.0,
                hourly_decode_apply_us_per_hour: 0.0,
            })
        })
        .collect::<io::Result<_>>()?;
    let sums = Mutex::new(vec![(0_u64, 0_u64); tables.len()]);
    gate::parallel(tables.len() * segments.slots, threads, |job| {
        let table_set = job / segments.slots;
        let index = job % segments.slots;
        let Some(segment) = segments.read(index)? else {
            return walks[table_set].segments.write(index, None);
        };
        let ids = &tables[table_set];
        let mut timing = if ids == union_tables {
            segment.timing.clone()
        } else {
            let storage: u64 = ids
                .iter()
                .map(|t| segment.actual_table_storage_peaks[&format!("{t:?}")])
                .sum();
            let reservation = storage
                .checked_add(320_000_000)
                .ok_or_else(|| io::Error::other("Reader reservation overflow"))?;
            let _permit = admission.enter(reservation)?;
            let mut recording = Recording::open_path(columnar, gate::limits())?;
            recording.select(ids);
            let selected = recording
                .segments()
                .get(segment.index)
                .ok_or_else(|| io::Error::other("Recording changed during measurement"))?;
            if selected.run != segment.run
                || selected.first != segment.first_ordinal
                || selected.last != segment.last_ordinal
            {
                return Err(io::Error::other("Recording changed during measurement"));
            }
            gate::timed_walk(&recording, selected)?
        };
        timing.hourly_cycles_file = None;
        timing.calibrate(rate);
        walks[table_set]
            .segments
            .write(index, Some(&(segment.index, timing.clone())))?;
        let mut sums = sums
            .lock()
            .map_err(|_| io::Error::other("Timing aggregate mutex poisoned"))?;
        sums[table_set].0 += timing.restore_cycles;
        sums[table_set].1 += timing.hourly_decode_apply_cycles;
        Ok(())
    })?;
    let mut walks = walks;
    let us = |cycles: u64| cycles as f64 / rate * 1e6 / hours as f64;
    for (walk, (restore, hourly)) in walks.iter_mut().zip(
        sums.into_inner()
            .map_err(|_| io::Error::other("Timing aggregate mutex poisoned"))?,
    ) {
        walk.restore_us_per_hour = us(restore);
        walk.hourly_decode_apply_us_per_hour = us(hourly);
    }
    for plan in plans {
        plan.duplicated_decode_us_per_hour = Some(
            plan.groups
                .iter()
                .map(|g| walks[g.timing].hourly_decode_apply_us_per_hour)
                .sum::<f64>()
                + walks[shared_timing].hourly_decode_apply_us_per_hour
                - us(union_timing.hourly_decode_apply_cycles),
        );
        plan.duplicated_restore_us_per_hour = Some(
            plan.groups
                .iter()
                .map(|g| walks[g.timing].restore_us_per_hour)
                .sum::<f64>()
                + walks[shared_timing].restore_us_per_hour
                - us(union_timing.restore_cycles),
        );
    }
    Ok((walks, shared_timing))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fit(candidates: impl IntoIterator<Item = Candidate>, slots: usize) -> (usize, u64, u64) {
        let path = std::env::temp_dir().join(format!(
            "gate-admission-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut out = File::create_new(&path).unwrap();
        for candidate in candidates {
            serde_json::to_writer(&mut out, &candidate).unwrap();
            writeln!(out).unwrap();
        }
        drop(out);
        let result = fit_file(&path, slots).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(path.with_extension("admitted.bin")).unwrap();
        result
    }

    #[test]
    fn shared_context_is_charged_once_per_admitted_segment() {
        assert_eq!(
            fit(
                [
                    Candidate {
                        bytes: 300_000_000,
                        segment: 0,
                        group: 0,
                        shared: 100_000_000
                    },
                    Candidate {
                        bytes: 300_000_000,
                        segment: 0,
                        group: 1,
                        shared: 100_000_000
                    },
                    Candidate {
                        bytes: 150_000_000,
                        segment: 1,
                        group: 0,
                        shared: 100_000_000
                    },
                ],
                2
            ),
            (2, 700_000_000, 100_000_000)
        );
    }

    #[test]
    fn largest_first_admission_counts_actual_segment_group_peaks() {
        let candidates = |bytes: Vec<u64>| {
            bytes
                .into_iter()
                .enumerate()
                .map(|(group, bytes)| Candidate {
                    bytes,
                    segment: 0,
                    group,
                    shared: 0,
                })
        };
        assert_eq!(
            fit(candidates(vec![100_000_000; 20]), 1),
            (9, 900_000_000, 0)
        );
        assert_eq!(fit(candidates(vec![901_000_000, 1]), 1), (0, 0, 0));
        assert_eq!(
            fit(
                candidates(vec![400_000_000, 300_000_000, 200_000_000, 1]),
                1
            ),
            (3, 900_000_000, 0)
        );
        assert_eq!(fit(candidates(vec![0; 1025]), 1), (1025, 0, 0));
    }
}
