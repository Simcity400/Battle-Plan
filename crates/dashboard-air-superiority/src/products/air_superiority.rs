//! Regional air comparison, bounded by the rows and dates drawn.
//!
//! Strategic-air entries are indexed by the region's runtime slot. Friendly
//! scores sum the friendly roster's contributions minus enemy suppression;
//! enemy scores reverse those rosters. Both clamp at zero.
//! The rosters include wartime allies and volunteer relationships, so the side
//! comparison is an application comparison of views. Co-belligerents are
//! assigned once by recorded war relations (assignment.rs).
//! The map uses friendly*100000/(friendly+enemy), strictly >60000 good and
//! <40000 bad. Zero colouring also reads the regional active-mission counters, which
//! this product's score display does not use.
//! Threshold labels are application wording. AirWings.mission_region is the
//! assigned mission region, including inactive assignments. Theatre groups
//! and members are the recorded theatre roster, joined through those wings.
//! RaidAirScores is sparse: complete tables witness every air-object owner;
//! a missing pair for that owner is zero, an absent owner has no air object.
//! Partial absent pairs are unknown. The score array is not saved: no oracle.

use dashboard_base::products::{MAX_PRODUCT_REPLY_BYTES, Product};
use dashboard_base::{catalogue, content, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_core::invalid;
#[cfg(test)]
use dashboard_text::metadata::recorded_rows;
use dashboard_text::metadata::check_cancel;
use dashboard_text::metadata::{link_slots, tagged_countries_indexed};
use replay::container::Replay;
#[cfg(any(test, feature = "test-support"))]
use replay_data::rows::TableId;
use replay_data::rows::tables;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};

mod assignment;
// The row observer's block pre-pass: the reference a native port tests against.
#[cfg(any(test, feature = "test-support"))]
pub mod block;
pub mod columnar;
mod daily;
pub mod emit;
mod model;
pub mod prepared;
mod scan;
#[cfg(any(test, feature = "test-support"))]
mod scores;
mod served;
pub use model::*;
use scan::{DayCache, Snapshot, span};
#[cfg(test)]
use scan::{extend_days, extend_days_anchored, extend_days_shared, partial_day, scan_days};
use std::sync::Arc;
#[cfg(test)]
use {
    dashboard_text::metadata::validate_campaign, replay_frame::frame::FrameDecoder,
    std::sync::Mutex,
};

pub struct AirSuperiority;
/// Hours in a game day: the recording's daily stagger period, the day boundary the keyed
/// weather and naval supremacy families sample by too.
const DAY_HOURS: i32 = replay_data::cadence::STAGGER_PERIOD;

#[cfg(test)]
type AnchorEntry = (
    Arc<dashboard_base::products::country_groups::Scan>,
    Arc<assignment::SideAnchors>,
);
#[cfg(test)]
#[derive(Default)]
struct AirBranch {
    gate: Mutex<()>,
    data: Mutex<Arc<DayCache>>,
    anchors: Mutex<Option<AnchorEntry>>,
}
#[cfg(test)]
impl AirBranch {
    fn fork(&self) -> Self {
        Self {
            gate: Mutex::new(()),
            data: Mutex::new(Arc::clone(
                &self.data.lock().unwrap_or_else(|e| e.into_inner()),
            )),
            anchors: Mutex::new(
                self.anchors
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone(),
            ),
        }
    }
}
#[cfg(test)]
type ViewpointEntry = (u64, String, bool, Arc<AirBranch>);

#[cfg(test)]
#[derive(Default)]
struct AirCache {
    branches: Vec<(u64, Arc<AirBranch>)>,
    viewpoints: Vec<ViewpointEntry>,
}
#[cfg(test)]
impl Clone for AirCache {
    fn clone(&self) -> Self {
        Self {
            branches: self
                .branches
                .iter()
                .map(|(branch, data)| (*branch, Arc::new(data.fork())))
                .collect(),
            viewpoints: self
                .viewpoints
                .iter()
                .map(|(branch, tag, country, data)| {
                    (*branch, tag.clone(), *country, Arc::new(data.fork()))
                })
                .collect(),
        }
    }
}
#[cfg(test)]
fn cached_branch(source: &Source, branch: u64, view: Option<(&str, bool)>) -> Arc<AirBranch> {
    let registry = source.append_state::<AirCache>();
    let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((tag, country)) = view {
        let entry = if let Some(index) = registry
            .viewpoints
            .iter()
            .position(|(old, key, kind, _)| *old == branch && key == tag && *kind == country)
        {
            registry.viewpoints.remove(index)
        } else {
            let data = AirBranch::default();
            *data.data.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(DayCache {
                viewpoint: Some(tag.into()),
                viewpoint_country: country,
                ..Default::default()
            });
            (branch, tag.into(), country, Arc::new(data))
        };
        let result = Arc::clone(&entry.3);
        registry.viewpoints.insert(0, entry);
        registry.viewpoints.truncate(2);
        result
    } else {
        let entry =
            if let Some(index) = registry.branches.iter().position(|(old, _)| *old == branch) {
                registry.branches.remove(index)
            } else {
                (branch, Arc::new(AirBranch::default()))
            };
        let result = Arc::clone(&entry.1);
        registry.branches.insert(0, entry);
        registry.branches.truncate(2);
        result
    }
}
#[cfg(test)]
fn branch_anchors(
    entry: &AirBranch,
    groups: &Arc<dashboard_base::products::country_groups::Scan>,
) -> Arc<assignment::SideAnchors> {
    if let Some((_, anchors)) = entry
        .anchors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|(scan, _)| Arc::ptr_eq(scan, groups))
    {
        return Arc::clone(anchors);
    }
    // No state lock spans the history walk. Moment-only requests populate this
    // small revision index without materialising the campaign's daily scores.
    let anchors = Arc::new(assignment::historical_anchors(groups));
    *entry.anchors.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((Arc::clone(groups), Arc::clone(&anchors)));
    anchors
}

#[cfg(test)]
fn extend_branch(
    entry: &AirBranch,
    replay: &Replay,
    branch: u64,
    groups: &Arc<dashboard_base::products::country_groups::Scan>,
    base: Option<&DayCache>,
    last: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Arc<DayCache>> {
    let _gate = entry.gate.lock().unwrap_or_else(|e| e.into_inner());
    let mut cache = Arc::clone(&entry.data.lock().unwrap_or_else(|e| e.into_inner()));
    let unchanged = cache.segments.len()
        == replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .count();
    let needs_partial = last.is_some_and(|last| {
        cache
            .day(last.div_euclid(DAY_HOURS))
            .is_some_and(|sample| sample.hour > last)
            && cache.partial.as_ref().is_none_or(|(at, _)| *at != last)
    });
    if unchanged && !needs_partial {
        return Ok(cache);
    }
    // Copy only the small index. All materialised block segments stay shared.
    let mutable = Arc::make_mut(&mut cache);
    let anchors = base.map_or_else(
        || branch_anchors(entry, groups),
        |base| Arc::clone(&base.anchors),
    );
    extend_days_anchored(mutable, replay, branch, groups, base, anchors, cancelled)?;
    if let Some(last) = last
        && mutable
            .day(last.div_euclid(DAY_HOURS))
            .is_some_and(|sample| sample.hour > last)
        && mutable.partial.as_ref().is_none_or(|(at, _)| *at != last)
    {
        mutable.partial = partial_day(replay, branch, groups, mutable, last, cancelled)?
            .map(|sample| (last, Arc::new(sample)));
    }
    *entry.data.lock().unwrap_or_else(|e| e.into_inner()) = Arc::clone(&cache);
    Ok(cache)
}

fn hours(params: &AirParams) -> io::Result<(i32, i32)> {
    let (Some(first), Some(last)) = (params.first_hour, params.last_hour) else {
        return Err(invalid("Air view needs firstHour and lastHour"));
    };
    if first > last {
        return Err(invalid("Air view hours are reversed"));
    }
    Ok((first, last))
}
fn side_filter(params: &AirParams) -> Option<HashSet<&str>> {
    params
        .sides
        .as_ref()
        .filter(|keys| !keys.is_empty())
        .map(|keys| keys.iter().map(String::as_str).collect())
}

fn selected(cell: &AirCell, sides: &[AirSide], keys: Option<&HashSet<&str>>) -> AirCell {
    selected_by(
        cell,
        &|index| sides.get(index as usize).map(|side| side.key.as_str()),
        keys,
    )
}

/// [`selected`], each side's key by its index from `key_of`.
fn selected_by<'k>(
    cell: &AirCell,
    key_of: &dyn Fn(u32) -> Option<&'k str>,
    keys: Option<&HashSet<&str>>,
) -> AirCell {
    if let Some(AirValue::Scores(scores)) = &cell.0 {
        if keys.is_none() {
            return cell.clone();
        }
        return selected_by(
            &AirCell(Some(AirValue::Sides(AirSideCell {
                scores: scores.clone(),
                viewpoints: vec![],
                unknown: vec![],
                unknown_set: None,
                no_air_object: vec![],
                filtered: false,
                excluded: 0,
            }))),
            key_of,
            keys,
        );
    }
    let Some(AirValue::Sides(cell)) = &cell.0 else {
        return cell.clone();
    };
    let keep =
        |index: &u32| keys.is_none_or(|keys| key_of(*index).is_some_and(|key| keys.contains(key)));
    AirCell(Some(AirValue::Sides(AirSideCell {
        viewpoints: cell
            .viewpoints
            .iter()
            .filter(|(index, _)| keep(index))
            .cloned()
            .collect(),
        scores: cell
            .scores
            .iter()
            .filter(|(index, _)| keep(index))
            .cloned()
            .collect(),
        unknown: cell
            .unknown
            .iter()
            .filter(|index| keep(index))
            .copied()
            .collect(),
        unknown_set: None,
        no_air_object: cell
            .no_air_object
            .iter()
            .filter(|index| keep(index))
            .copied()
            .collect(),
        filtered: keys.is_some(),
        excluded: cell.excluded,
    })))
}
struct TheatreFilter {
    regions: Option<HashMap<usize, HashSet<u32>>>,
}
impl TheatreFilter {
    fn new<'a>(samples: impl Iterator<Item = &'a Snapshot>, params: &AirParams) -> Self {
        let regions = params
            .theatres
            .as_ref()
            .filter(|ids| !ids.is_empty())
            .map(|ids| {
                let mut selected = HashMap::new();
                for sample in samples {
                    selected
                        .entry(Arc::as_ptr(&sample.theatres) as usize)
                        .or_insert_with(|| {
                            ids.iter()
                                .filter_map(|id| sample.theatres.get(id))
                                .flatten()
                                .copied()
                                .collect()
                        });
                }
                selected
            });
        Self { regions }
    }
    fn at(&self, sample: &Snapshot) -> Option<&HashSet<u32>> {
        self.regions
            .as_ref()
            .map(|regions| &regions[&(Arc::as_ptr(&sample.theatres) as usize)])
    }
}

fn cell_at(
    sample: &Snapshot,
    region: u32,
    sides: &[AirSide],
    theatres: Option<&HashSet<u32>>,
    keys: Option<&HashSet<&str>>,
) -> AirCell {
    if let Some(theatres) = theatres {
        if !sample.theatres_known {
            return AirCell(Some(AirValue::Reason("theatres-not-recorded".into())));
        }
        if !theatres.contains(&region) {
            return AirCell(Some(AirValue::Reason("outside-theatres".into())));
        }
    }
    selected(
        sample.cells.get(&region).unwrap_or(&sample.fallback),
        sides,
        keys,
    )
}
fn activity_at(
    sample: &Snapshot,
    region: u32,
    sides: &[AirSide],
    theatres: Option<&HashSet<u32>>,
    keys: Option<&HashSet<&str>>,
) -> u128 {
    if theatres.is_some_and(|theatres| !sample.theatres_known || !theatres.contains(&region)) {
        return 0;
    }
    let keep = |index: u32| {
        keys.is_none_or(|keys| {
            sides
                .get(index as usize)
                .is_some_and(|side| keys.contains(side.key.as_str()))
        })
    };
    match &sample.cells.get(&region).unwrap_or(&sample.fallback).0 {
        Some(AirValue::Scores(scores)) => scores
            .iter()
            .filter(|(index, _)| keep(*index))
            .filter_map(|(_, score)| score.parse::<u128>().ok())
            .sum(),
        Some(AirValue::Sides(cell)) => cell
            .scores
            .iter()
            .filter(|(index, _)| keep(*index))
            .filter_map(|(_, score)| score.parse::<u128>().ok())
            .sum(),
        Some(AirValue::Viewpoint((friendly, enemy)))
        | Some(AirValue::SideViewpoint {
            view: (friendly, enemy),
            ..
        }) => friendly.parse::<u128>().unwrap_or(0) + enemy.parse::<u128>().unwrap_or(0),
        _ => 0,
    }
}
fn compact_sides(
    sides: &[AirSide],
    cells: &mut [&mut AirCell],
    read_hours: &[i32],
    last: i32,
) -> Vec<AirSide> {
    compact_sides_with(&|index| &sides[index as usize], cells, read_hours, last)
}

/// [`compact_sides`], each side by its index from `side_of`.
fn compact_sides_with<'s>(
    side_of: &dyn Fn(u32) -> &'s AirSide,
    cells: &mut [&mut AirCell],
    read_hours: &[i32],
    last: i32,
) -> Vec<AirSide> {
    let mut used = BTreeSet::new();
    for cell in cells.iter() {
        match &cell.0 {
            Some(AirValue::Scores(cell)) => used.extend(cell.iter().map(|(index, _)| *index)),
            Some(AirValue::SideViewpoint { side, .. }) => {
                used.insert(*side);
            }
            Some(AirValue::Sides(cell)) => {
                used.extend(cell.scores.iter().map(|(index, _)| *index));
                used.extend(&cell.unknown);
                used.extend(&cell.no_air_object);
            }
            _ => {}
        }
    }
    let remap: HashMap<_, _> = used
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new as u32))
        .collect();
    for cell in cells.iter_mut() {
        match &mut cell.0 {
            Some(AirValue::Scores(cell)) => {
                for (index, _) in cell {
                    *index = remap[index];
                }
            }
            Some(AirValue::SideViewpoint { side, .. }) => *side = remap[side],
            Some(AirValue::Sides(cell)) => {
                for (index, _) in &mut cell.viewpoints {
                    *index = remap[index];
                }
                for (index, _) in &mut cell.scores {
                    *index = remap[index];
                }
                for index in cell.unknown.iter_mut().chain(&mut cell.no_air_object) {
                    *index = remap[index];
                }
            }
            _ => {}
        }
    }
    used.into_iter()
        .map(|index| {
            let side = side_of(index);
            // A superseded partial-day read must not survive the returned samples.
            let mut reads = Vec::new();
            for &hour in read_hours.iter().filter(|&&hour| hour <= last) {
                let at = side.reads.partition_point(|read| read.0 <= hour);
                let Some(read) = at.checked_sub(1).map(|at| &side.reads[at]) else {
                    continue;
                };
                let mut read = read.clone();
                read.0 = hour;
                if reads.last().is_some_and(|old: &model::AirSideRead| {
                    (&old.1, &old.2, &old.3, old.4, &old.5, &old.6)
                        == (&read.1, &read.2, &read.3, read.4, &read.5, &read.6)
                }) {
                    continue;
                }
                reads.push(read);
            }
            let first = reads.first();
            AirSide {
                key: side.key.clone(),
                faction: side.faction.clone(),
                colour_faction: first
                    .map_or_else(|| side.colour_faction.clone(), |read| read.6.clone()),
                tag: first.map_or_else(|| side.tag.clone(), |read| read.3.clone()),
                name: first.map_or_else(|| side.name.clone(), |read| read.1.clone()),
                leader: first.map_or_else(|| side.leader.clone(), |read| read.2.clone()),
                co_belligerents: first.map_or(side.co_belligerents, |read| read.4),
                factions: first.map_or_else(|| side.factions.clone(), |read| read.5.clone()),
                reads,
            }
        })
        .collect()
}

fn timeline(cache: &DayCache, params: &AirParams) -> io::Result<AirTimelineReply> {
    let (first, last) = hours(params)?;
    let keys = side_filter(params);
    let first_day = first.div_euclid(DAY_HOURS);
    let last_day = last.div_euclid(DAY_HOURS);
    let first_day_hour = first_day
        .checked_mul(DAY_HOURS)
        .ok_or_else(|| invalid("Air timeline start is outside the calendar range"))?;
    let count = (i64::from(last_day) - i64::from(first_day) + 1) as usize;
    // Even a one-byte date needs a separator. Refuse only a proven JSON
    // lower bound before allocating the requested calendar, never a cell cap.
    if count.saturating_mul(2).saturating_sub(1) > MAX_PRODUCT_REPLY_BYTES {
        return Err(reply_budget_error());
    }
    let mut samples = vec![None; count];
    // One forward traversal: later blocks replace the same partial daily slot.
    for segment in &cache.segments {
        for (&day, sample) in segment.days.range(first_day..=last_day) {
            if sample.hour <= last {
                samples[(day - first_day) as usize] = Some(sample);
            }
        }
    }
    #[cfg(test)]
    if let Some((_, sample)) = cache.partial.as_ref().filter(|(at, _)| *at == last) {
        samples[count - 1] = Some(sample);
    }
    let theatres = TheatreFilter::new(
        samples.iter().flatten().map(|sample| sample.as_ref()),
        params,
    );
    let completed = |column: usize| {
        first_day + (column as i32) < last_day || last.rem_euclid(DAY_HOURS) == DAY_HOURS - 1
    };
    let mut candidates: BTreeSet<u32> = params
        .regions
        .as_ref()
        .map(|regions| regions.iter().copied().collect())
        .unwrap_or_else(|| cache.regions.clone());
    if let Some(selected) = &theatres.regions
        && samples.iter().flatten().all(|sample| sample.theatres_known)
    {
        let allowed: HashSet<_> = selected.values().flatten().copied().collect();
        candidates.retain(|region| allowed.contains(region));
    }
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .map(|region| {
            let total = samples
                .iter()
                .enumerate()
                .filter(|(column, _)| completed(*column))
                .filter_map(|(_, sample)| {
                    sample.map(|sample| {
                        activity_at(
                            sample,
                            region,
                            &cache.sides,
                            theatres.at(sample),
                            keys.as_ref(),
                        )
                    })
                })
                .sum::<u128>();
            (region, total)
        })
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if params.regions.is_none() {
        ranked.truncate(params.top.unwrap_or(10));
    }
    let mut rows = Vec::new();
    for (region, _) in ranked {
        let mut cells: Vec<(usize, AirCell)> = Vec::new();
        for sample in &samples {
            let cell = sample
                .map(|sample| {
                    cell_at(
                        sample,
                        region,
                        &cache.sides,
                        theatres.at(sample),
                        keys.as_ref(),
                    )
                })
                .unwrap_or_default();
            match cells.last_mut() {
                Some((repeat, old)) if *old == cell => *repeat += 1,
                _ => cells.push((1, cell)),
            }
        }
        rows.push(AirTimelineRow {
            region,
            name: format!("Region {region}"),
            cells,
        });
    }
    let read_hours: Vec<_> = samples
        .iter()
        .flatten()
        .map(|sample| sample.hour)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let sides = compact_sides(
        &cache.sides,
        &mut rows
            .iter_mut()
            .flat_map(|row| row.cells.iter_mut().map(|(_, cell)| cell))
            .collect::<Vec<_>>(),
        &read_hours,
        last,
    );
    let displayed = |day: usize, sample: &Snapshot| {
        cache
            .sampled_days
            .get(&(first_day + day as i32))
            .copied()
            .unwrap_or(sample.hour)
    };
    let excluded = samples
        .iter()
        .enumerate()
        .filter_map(|(day, sample)| {
            sample.map(|sample| {
                let hour = displayed(day, sample);
                (hour, hour, sample.excluded)
            })
        })
        .collect();
    let theatre_reads = samples
        .iter()
        .enumerate()
        .filter_map(|(day, sample)| {
            sample.and_then(|sample| {
                let hour = displayed(day, sample);
                sample.theatre_read.map(|read| (hour, hour, read))
            })
        })
        .collect();
    Ok(AirTimelineReply {
        view: AirView::Timeline,
        sample_hours: None,
        approximate: Vec::new(),
        rows_approximate: false,
        branch_id: params.branch_id.clone(),
        available: cache.available,
        first_hour: first_day_hour,
        last_hour: last,
        days: samples
            .iter()
            .enumerate()
            .map(|(day, sample)| sample.map(|sample| displayed(day, sample)))
            .collect(),
        sides,
        segments: BTreeMap::new(),
        anchor_reads: BTreeMap::new(),
        rows,
        scale: engine_native::fixed5::SCALE as i32,
        day_hours: DAY_HOURS,
        bad_below: 40_000,
        good_above: 60_000,
        excluded,
        theatre_reads,
        warnings: cache.warnings.clone(),
    })
}

#[cfg(test)]
pub(super) fn scan_moment(
    replay: &Replay,
    branch: u64,
    groups: &dashboard_base::products::country_groups::Scan,
    params: &AirParams,
    cancelled: &AtomicBool,
) -> io::Result<AirMomentReply> {
    scan_moment_anchored(
        replay,
        branch,
        groups,
        params,
        &assignment::historical_anchors(groups),
        cancelled,
    )
}

#[cfg(test)]
fn scan_moment_anchored(
    replay: &Replay,
    branch: u64,
    groups: &dashboard_base::products::country_groups::Scan,
    params: &AirParams,
    anchors: &assignment::SideAnchors,
    cancelled: &AtomicBool,
) -> io::Result<AirMomentReply> {
    hours(params)?;
    let (samples, sides) = scan::scan_window(replay, branch, groups, params, anchors, cancelled)?;
    moment_reply(samples, sides, groups, params, anchors)
}

fn moment_reply(
    samples: Vec<Snapshot>,
    sides: Vec<AirSide>,
    groups: &dashboard_base::products::country_groups::Scan,
    params: &AirParams,
    anchors: &assignment::SideAnchors,
) -> io::Result<AirMomentReply> {
    let (first, last) = hours(params)?;
    let theatres = TheatreFilter::new(samples.iter(), params);
    let keys = side_filter(params);
    let selected_regions: Option<HashSet<_>> = params
        .regions
        .as_ref()
        .map(|regions| regions.iter().copied().collect());
    let mut wing_runs = dashboard_base::read_runs::ReadRuns::<u32>::new();
    for sample in &samples {
        if sample.wings_known {
            wing_runs.read(
                sample.hour,
                sample
                    .wings
                    .iter()
                    .map(|(region, count)| (*region, Some(i64::from(*count)))),
            );
        } else {
            wing_runs.close();
        }
    }
    let regions: BTreeSet<_> = samples
        .iter()
        .flat_map(|sample| {
            sample
                .cells
                .keys()
                .chain(sample.wings.keys())
                .chain(&sample.active)
                .copied()
        })
        .collect();
    let mut rows = Vec::new();
    for region in regions {
        if selected_regions
            .as_ref()
            .is_some_and(|selected| !selected.contains(&region))
        {
            continue;
        }
        let mut row = AirMomentRegion {
            id: region,
            name: format!("Region {region}"),
            active: vec![],
            runs: vec![],
            wings: vec![],
        };
        for sample in &samples {
            if sample.active.contains(&region) {
                span(&mut row.active, sample.hour);
            }
            let cell = cell_at(sample, region, &sides, theatres.at(sample), keys.as_ref());
            match row.runs.last_mut() {
                Some((_, end, old)) if *end + 1 == sample.hour && *old == cell => {
                    *end = sample.hour
                }
                _ => row.runs.push((sample.hour, sample.hour, cell)),
            }
        }
        row.wings = wing_runs
            .runs
            .remove(&region)
            .unwrap_or_default()
            .into_iter()
            .map(|run| {
                (
                    run.first_hour,
                    run.last_hour,
                    run.amount_raw
                        .expect("wing count is known")
                        .parse::<u32>()
                        .expect("wing count fits"),
                )
            })
            .collect();
        rows.push(row);
    }
    let mut recorded = Vec::new();
    let mut complete = Vec::new();
    let mut wings_known = Vec::new();
    let mut fallback: Vec<(i32, i32, AirCell)> = Vec::new();
    for sample in &samples {
        let cell = if params
            .theatres
            .as_ref()
            .is_some_and(|theatres| !theatres.is_empty())
        {
            AirCell(Some(AirValue::Reason(
                if sample.theatres_known {
                    "outside-theatres"
                } else {
                    "theatres-not-recorded"
                }
                .into(),
            )))
        } else {
            selected(&sample.fallback, &sides, keys.as_ref())
        };
        match fallback.last_mut() {
            Some((_, end, old)) if end.checked_add(1) == Some(sample.hour) && *old == cell => {
                *end = sample.hour
            }
            _ => fallback.push((sample.hour, sample.hour, cell)),
        }
        span(&mut recorded, sample.hour);
        if sample.complete {
            span(&mut complete, sample.hour);
        }
        if sample.wings_known {
            span(&mut wings_known, sample.hour);
        }
    }
    let sides = compact_sides(
        &sides,
        &mut rows
            .iter_mut()
            .flat_map(|row| row.runs.iter_mut().map(|(_, _, cell)| cell))
            .chain(fallback.iter_mut().map(|(_, _, cell)| cell))
            .collect::<Vec<_>>(),
        &samples.iter().map(|sample| sample.hour).collect::<Vec<_>>(),
        last,
    );
    let mut excluded: Vec<(i32, i32, usize)> = Vec::new();
    let mut theatre_reads: Vec<(i32, i32, i32)> = Vec::new();
    for sample in &samples {
        if let Some(read) = sample.theatre_read {
            match theatre_reads.last_mut() {
                Some((_, last, old))
                    if last.checked_add(1) == Some(sample.hour) && *old == read =>
                {
                    *last = sample.hour
                }
                _ => theatre_reads.push((sample.hour, sample.hour, read)),
            }
        }
        match excluded.last_mut() {
            Some((_, last, count))
                if last.checked_add(1) == Some(sample.hour) && *count == sample.excluded =>
            {
                *last = sample.hour
            }
            _ => excluded.push((sample.hour, sample.hour, sample.excluded)),
        }
    }
    let unknown_sets = compact_unknowns(&mut rows, &mut fallback);
    let anchor_reads = anchor_reads(&sides, groups, anchors, last);
    let segments = country_segments(groups, first, last, &anchor_reads);
    Ok(AirMomentReply {
        view: AirView::Moment,
        branch_id: params.branch_id.clone(),
        available: samples.iter().any(|sample| sample.available),
        first_hour: first,
        last_hour: last,
        sides,
        segments,
        anchor_reads,
        unknown_sets,
        regions: rows,
        fallback,
        recorded,
        complete,
        wings_known,
        scale: engine_native::fixed5::SCALE as i32,
        day_hours: DAY_HOURS,
        bad_below: 40_000,
        good_above: 60_000,
        excluded,
        theatre_reads,
        warnings: groups.warnings.clone(),
    })
}

fn compact_unknowns(
    rows: &mut [AirMomentRegion],
    fallback: &mut [(i32, i32, AirCell)],
) -> Vec<Vec<u32>> {
    let mut sets = Vec::new();
    let mut indices = HashMap::new();
    for cell in rows
        .iter_mut()
        .flat_map(|row| row.runs.iter_mut().map(|(_, _, cell)| cell))
        .chain(fallback.iter_mut().map(|(_, _, cell)| cell))
    {
        if let Some(AirValue::Sides(cell)) = &mut cell.0
            && !cell.unknown.is_empty()
        {
            let index = if let Some(&index) = indices.get(&cell.unknown) {
                index
            } else {
                let index = sets.len() as u32;
                indices.insert(cell.unknown.clone(), index);
                sets.push(cell.unknown.clone());
                index
            };
            cell.unknown.clear();
            cell.unknown_set = Some(index);
        }
    }
    sets
}

impl Product for AirSuperiority {
    const KEY: &'static str = "air-superiority";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = AirParams;
    type Reply = AirReply;
    fn types() -> Vec<String> {
        model::types()
    }
    fn validate(params: &AirParams) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid air branch")?;
        if params.view != AirView::Choices {
            hours(params).map_err(|_| "Invalid air hours")?;
            if params.mode == Some(AirMode::Viewpoint)
                && params.viewpoint.as_ref().is_none_or(String::is_empty)
            {
                return Err("Viewpoint country or side is required");
            }
        }
        if params.top == Some(0) {
            return Err("Air top region count must be positive");
        }
        Ok(())
    }
    #[cfg(test)]
    fn load(
        source: &Source,
        params: &AirParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<AirReply> {
        let mut choices_params;
        let params = if params.view == AirView::Choices {
            choices_params = params.clone();
            choices_params.mode = None;
            choices_params.viewpoint = None;
            choices_params.viewpoint_country = None;
            choices_params.sides = None;
            &choices_params
        } else {
            params
        };
        let branch = params.branch_id.parse().expect("validated branch");
        let replay = source.replay_ref();
        check_cancel(cancelled)?;
        let start = replay
            .branches
            .iter()
            .find(|known| known.id == branch)
            .ok_or_else(|| invalid("Air branch is not in this recording"))?;
        let naming_hour = params
            .last_hour
            .unwrap_or_else(|| dashboard_text::metadata::branch_bounds(replay, start).1);
        if params.view == AirView::Choices && params.windows_only == Some(true) {
            let reply = AirReply::Choices(AirChoicesReply {
                view: AirView::Choices,
                branch_id: params.branch_id.clone(),
                regions: vec![],
                sides: vec![],
                theatres: vec![],
                countries: vec![],
                windows: moment_windows(replay, branch),
                warnings: vec![],
            });
            check_reply_size(&reply)?;
            return Ok(reply);
        }
        let groups =
            dashboard_base::products::country_groups::cached_scan(source, branch, cancelled)?;
        let base_entry = cached_branch(source, branch, None);
        if params.view == AirView::Moment {
            let mut reply = AirReply::Moment(scan_moment_anchored(
                replay,
                branch,
                &groups,
                params,
                &branch_anchors(&base_entry, &groups),
                cancelled,
            )?);
            resolve_names(source, &groups, context, naming_hour, None, &mut reply);
            check_reply_size(&reply)?;
            return Ok(reply);
        }
        let base = extend_branch(&base_entry, replay, branch, &groups, None, None, cancelled)?;
        let viewpoint = (params.mode == Some(AirMode::Viewpoint))
            .then_some(params.viewpoint.as_deref())
            .flatten();
        let entry = viewpoint
            .map(|tag| {
                cached_branch(
                    source,
                    branch,
                    Some((tag, params.viewpoint_country == Some(true))),
                )
            })
            .unwrap_or(base_entry);
        let last = (params.view == AirView::Timeline)
            .then(|| hours(params).map(|(_, last)| last))
            .transpose()?;
        let cache = if viewpoint.is_some() || last.is_some() {
            extend_branch(
                &entry,
                replay,
                branch,
                &groups,
                viewpoint.map(|_| &*base),
                last,
                cancelled,
            )?
        } else {
            base
        };
        cache_reply(
            source,
            replay,
            &groups,
            &cache,
            params,
            context,
            naming_hour,
            cancelled,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn cache_reply(
    source: &Source,
    replay: &Replay,
    groups: &dashboard_base::products::country_groups::Scan,
    cache: &DayCache,
    params: &AirParams,
    context: &LoadContext,
    naming_hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<AirReply> {
    let branch = params.branch_id.parse().expect("validated branch");
    let mut reply = match params.view {
        AirView::Timeline => AirReply::Timeline(timeline(cache, params)?),
        AirView::Choices => AirReply::Choices(AirChoicesReply {
            view: AirView::Choices,
            branch_id: params.branch_id.clone(),
            regions: cache
                .regions
                .iter()
                .map(|id| AirRegionChoice {
                    id: *id,
                    name: format!("Region {id}"),
                })
                .collect(),
            sides: cache
                .sides
                .iter()
                .map(|side| {
                    let mut side = side.clone();
                    if let Some((_, name, leader, tag, co_belligerents, factions, colour_faction)) =
                        side.reads.last()
                    {
                        side.name = name.clone();
                        side.leader = leader.clone();
                        side.tag = tag.clone();
                        side.co_belligerents = *co_belligerents;
                        side.factions = factions.clone();
                        side.colour_faction = colour_faction.clone();
                    }
                    side.reads.clear();
                    side
                })
                .collect(),
            theatres: cache
                .theatres
                .keys()
                .map(|id| AirTheatreChoice {
                    id: id.clone(),
                    name: format!("Air theatre {id} (name not recorded)"),
                })
                .collect(),
            countries: cache.countries.iter().cloned().collect(),
            windows: moment_windows(replay, branch),
            warnings: cache.warnings.clone(),
        }),
        AirView::Moment => unreachable!(),
    };
    if let AirReply::Timeline(reply) = &mut reply {
        reply.anchor_reads = anchor_reads(&reply.sides, groups, &cache.anchors, reply.last_hour);
        reply.segments = country_segments(
            groups,
            reply.first_hour,
            reply.last_hour,
            &reply.anchor_reads,
        );
    }
    resolve_names(
        source,
        groups,
        context,
        naming_hour,
        Some(&cache.theatres),
        &mut reply,
    );
    if let AirReply::Choices(choices) = &mut reply {
        if let Some(regions) = &params.regions {
            let regions: HashSet<_> = regions.iter().copied().collect();
            choices
                .regions
                .retain(|region| regions.contains(&region.id));
        }
        if let Some(theatres) = params
            .theatres
            .as_ref()
            .filter(|theatres| !theatres.is_empty())
        {
            let first = params.first_hour.unwrap_or(i32::MIN);
            let last = params.last_hour.unwrap_or(i32::MAX);
            {
                let relevant: Vec<_> = cache
                    .auxiliary()
                    .filter(|(start, end, _)| *start <= last && first <= *end)
                    .collect();
                if relevant.is_empty() || relevant.iter().any(|(_, _, aux)| !aux.theatres_known) {
                    choices.regions.clear();
                    choices
                        .warnings
                        .push("Air theatre membership is not recorded in this window".into());
                } else {
                    let regions: BTreeSet<_> = relevant
                        .iter()
                        .flat_map(|(_, _, aux)| {
                            theatres
                                .iter()
                                .filter_map(|id| aux.theatres.get(id))
                                .flatten()
                                .copied()
                        })
                        .collect();
                    choices
                        .regions
                        .retain(|region| regions.contains(&region.id));
                }
            }
        }
    }
    check_cancel(cancelled)?;
    check_reply_size(&reply)?;
    Ok(reply)
}

/// A timeline reply with its anchor reads, identity segments and names, as
/// [`cache_reply`] presents one.
fn present_timeline(
    source: &Source,
    groups: &dashboard_base::products::country_groups::Scan,
    anchors: &assignment::SideAnchors,
    context: &LoadContext,
    naming_hour: i32,
    mut reply: AirTimelineReply,
    cancelled: &AtomicBool,
) -> io::Result<AirReply> {
    reply.anchor_reads = anchor_reads(&reply.sides, groups, anchors, reply.last_hour);
    reply.segments = country_segments(
        groups,
        reply.first_hour,
        reply.last_hour,
        &reply.anchor_reads,
    );
    let mut reply = AirReply::Timeline(reply);
    resolve_names(source, groups, context, naming_hour, None, &mut reply);
    check_cancel(cancelled)?;
    check_reply_size(&reply)?;
    Ok(reply)
}

fn check_reply_size(reply: &AirReply) -> io::Result<usize> {
    dashboard_core::transport::json_len(reply, MAX_PRODUCT_REPLY_BYTES)
        .ok_or_else(reply_budget_error)
}

fn reply_budget_error() -> io::Error {
    invalid(format!(
        "Air view exceeds MAX_PRODUCT_REPLY_BYTES ({MAX_PRODUCT_REPLY_BYTES} bytes); select fewer regions or a shorter range"
    ))
}

fn moment_windows(replay: &Replay, branch: u64) -> Vec<AirSpan> {
    let start = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .expect("validated branch");
    let (mut first_hour, last_hour) = dashboard_text::metadata::branch_bounds(replay, start);
    let mut windows = Vec::new();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        windows.push(AirSpan {
            first_hour,
            last_hour: block.last_hour,
        });
        first_hour = block.last_hour.saturating_add(1);
    }
    // Include gaps and the unread tail; their hours stay unknown in the reply.
    match windows.last_mut() {
        Some(window) => window.last_hour = last_hour,
        None => windows.push(AirSpan {
            first_hour,
            last_hour,
        }),
    }
    windows
}

type RegionNameEntry = (
    content::LocalisationKey,
    Arc<BTreeMap<u32, String>>,
    Arc<installed_content_localisation::localisation::Localisation>,
    Vec<String>,
);

#[derive(Clone, Default)]
struct RegionNames {
    entry: Option<RegionNameEntry>,
}

fn anchor_reads(
    sides: &[AirSide],
    groups: &dashboard_base::products::country_groups::Scan,
    anchors: &assignment::SideAnchors,
    last: i32,
) -> BTreeMap<String, (String, i32)> {
    sides
        .iter()
        .filter_map(|side| {
            let (tag, hour) = if let Some(key) = side.key.strip_prefix("w:") {
                anchors.country_reads.get(key)?
            } else {
                let id = side.faction.as_ref()?.parse::<u64>().ok()?;
                anchors.faction_reads.get(&id)?
            };
            (*hour <= last
                && groups
                    .identities
                    .get(tag)?
                    .holding(*hour, |run| (run.first_hour, run.last_hour))
                    .is_some())
            .then(|| (side.key.clone(), (tag.clone(), *hour)))
        })
        .collect()
}

fn country_segments(
    groups: &dashboard_base::products::country_groups::Scan,
    first: i32,
    last: i32,
    anchors: &BTreeMap<String, (String, i32)>,
) -> BTreeMap<String, Vec<AirSpan>> {
    let mut anchor_hours = BTreeMap::<&str, BTreeSet<i32>>::new();
    for (tag, hour) in anchors.values() {
        anchor_hours.entry(tag).or_default().insert(*hour);
    }
    groups
        .identities
        .iter()
        .filter_map(|(tag, history)| {
            let hours = |run: &dashboard_base::products::country_history::CountryIdentityRun| {
                (run.first_hour, run.last_hour)
            };
            // Runs are disjoint and in hour order: the ones read, by their first hour.
            let mut runs = BTreeMap::new();
            for run in history.overlapping(first, last, hours) {
                runs.insert(run.first_hour, run);
            }
            for hour in anchor_hours.get(tag.as_str()).into_iter().flatten() {
                if let Some(run) = history.holding(*hour, hours) {
                    runs.insert(run.first_hour, run);
                }
            }
            let segments: Vec<_> = runs
                .into_values()
                .map(|run| AirSpan {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour.min(last),
                })
                .collect();
            (!segments.is_empty()).then(|| (tag.clone(), segments))
        })
        .collect()
}

fn resolve_names(
    source: &Source,
    groups: &dashboard_base::products::country_groups::Scan,
    context: &LoadContext,
    hour: i32,
    theatre_names: Option<&BTreeMap<String, Vec<u8>>>,
    reply: &mut AirReply,
) {
    let mut warnings = Vec::new();
    let is_choices = matches!(reply, AirReply::Choices(_));
    let names = (|| {
        let options = context
            .content
            .as_ref()
            .ok_or_else(|| invalid("game folders are not chosen"))?;
        let key = content::localisation_key(options)?;
        let state = source.append_state::<RegionNames>();
        let cached = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry
            .clone();
        if let Some((old, names, loc, notes)) = cached
            && old == key
        {
            warnings.extend(notes);
            return Ok((names, loc));
        }
        content::check_replay(source.replay_ref(), options.allow_unverified, &mut warnings)?;
        let layers = source.content(&options.options)?;
        let regions = content::strategic_regions(&layers.layers)?;
        let loc = content::localisation(source, options)?;
        warnings.extend(regions.notes.iter().cloned());
        let mut names = BTreeMap::new();
        for (&id, definition) in &regions.regions {
            match (definition.name_key.is_empty() || loc.get(&definition.name_key).is_some())
                .then_some(())
                .ok_or_else(|| "localized title is absent from the recorded language".to_owned())
                .and_then(|_| {
                    catalogue::localised(&loc, &definition.name_key, catalogue::View::Plain)
                }) {
                // Successful empty text remains distinct from a missing title.
                Ok(name) => {
                    names.insert(id, name);
                }
                Err(reason) => {
                    warnings.push(format!(
                        "Region {id} name {} is unavailable: {}",
                        definition.name_key, reason
                    ));
                    names.insert(id, format!("Region {id}"));
                }
            }
        }
        let names = Arc::new(names);
        state.lock().unwrap_or_else(|e| e.into_inner()).entry =
            Some((key, Arc::clone(&names), Arc::clone(&loc), warnings.clone()));
        Ok::<_, io::Error>((names, loc))
    })();
    let language = &source.replay_ref().identity.language;
    let recorded_language = language.strip_prefix("l_").unwrap_or(language);
    let viewer_language = context
        .content
        .as_ref()
        .and_then(|options| content::language(&options.options).ok())
        .unwrap_or_else(|| recorded_language.to_owned());
    let mut stored_language = installed_content_localisation::localisation::Localisation::default();
    stored_language.language = viewer_language;
    if let AirReply::Choices(reply) = reply {
        // The theatre retains localised text; no source key or literal marker survives.
        let localisation = names
            .as_ref()
            .map_or(&stored_language, |(_, loc)| loc.as_ref());
        let mut refused = catalogue::Refusals::default();
        for theatre in &mut reply.theatres {
            let name = theatre_names
                .and_then(|names| names.get(&theatre.id))
                .ok_or_else(|| "Air theatre name was not recorded".to_owned())
                .and_then(|bytes| {
                    dashboard_text::scripted_text::stored_text(
                        localisation,
                        recorded_language,
                        bytes,
                        "Air theatre name",
                    )
                })
                .and_then(|name| {
                    if name.is_empty() {
                        Err("Air theatre name is empty".into())
                    } else {
                        Ok(name)
                    }
                });
            match name {
                Ok(name) => theatre.name = name,
                Err(reason) => refused.note(reason, format!("air theatre {}", theatre.id)),
            }
        }
        warnings.extend(refused.lines("air theatre names are"));
    }
    let (sides, reply_warnings) = match reply {
        AirReply::Timeline(reply) => {
            if let Ok((names, _)) = &names {
                for row in &mut reply.rows {
                    if let Some(name) = names.get(&row.region) {
                        row.name.clone_from(name);
                    }
                }
            }
            (&mut reply.sides, &mut reply.warnings)
        }
        AirReply::Moment(reply) => {
            if let Ok((names, _)) = &names {
                for region in &mut reply.regions {
                    if let Some(name) = names.get(&region.id) {
                        region.name.clone_from(name);
                    }
                }
            }
            (&mut reply.sides, &mut reply.warnings)
        }
        AirReply::Choices(reply) => {
            if let Ok((names, _)) = &names {
                let mut choices: BTreeMap<_, _> = std::mem::take(&mut reply.regions)
                    .into_iter()
                    .map(|region| (region.id, region))
                    .collect();
                for (id, name) in names.iter() {
                    if let Some(region) = choices.get_mut(id) {
                        region.name.clone_from(name);
                    } else {
                        choices.insert(
                            *id,
                            AirRegionChoice {
                                id: *id,
                                name: name.clone(),
                            },
                        );
                    }
                }
                reply.regions = choices.into_values().collect();
            }
            (&mut reply.sides, &mut reply.warnings)
        }
    };
    let naming = context.content.as_ref().map(|options| {
        content::check_replay(source.replay_ref(), options.allow_unverified, &mut warnings)?;
        let layers = source.content(&options.options)?;
        let localisation = match &names {
            Ok((_, loc)) => Arc::clone(loc),
            Err(_) => content::localisation(source, options)?,
        };
        Ok::<_, io::Error>((layers.layers.faction_templates(), localisation))
    });
    let (templates, localisation) = match &naming {
        Some(Ok((templates, loc))) => {
            if let Ok(templates) = templates {
                warnings.extend(templates.warnings.iter().cloned());
            }
            (templates.as_deref().map_err(Clone::clone), loc.as_ref())
        }
        Some(Err(error)) => (
            Err(format!("Faction naming content is unavailable: {error}")),
            &stored_language,
        ),
        None => (
            Err("Faction naming content is unavailable until the game folders are chosen".into()),
            &stored_language,
        ),
    };
    let mut refused = catalogue::Refusals::default();
    let mut resolved = HashMap::new();
    let mut localise = |id: &str, at: i32| {
        let at = if is_choices {
            id.parse::<u64>()
                .ok()
                .and_then(|id| groups.factions.get(&id))
                .and_then(|faction| faction.names.iter().next_back())
                .map_or(at, |name| name.last_hour)
        } else {
            at
        };
        resolved
            .entry((id.to_owned(), at))
            .or_insert_with(|| {
                match id
                    .parse::<u64>()
                    .map_err(|_| "Invalid recorded faction id".to_owned())
                    .and_then(|id| {
                        groups.resolved_faction_name_at(
                            id,
                            at,
                            templates.as_ref().copied().map_err(Clone::clone),
                            localisation,
                            recorded_language,
                        )
                    }) {
                    Ok(name) if !name.is_empty() => name,
                    result => {
                        refused.note(
                            result
                                .err()
                                .unwrap_or_else(|| "Faction name has no plain display text".into()),
                            format!("faction {id}"),
                        );
                        format!("Faction {id}")
                    }
                }
            })
            .clone()
    };
    for side in sides.iter_mut() {
        // Compacted dictionaries retain the preceding read. Its metadata is
        // the base for this window, including which real factions supplied it.
        if let Some((_, name, leader, tag, co_belligerents, factions, colour_faction)) =
            side.reads.first()
        {
            side.name.clone_from(name);
            side.leader.clone_from(leader);
            side.tag.clone_from(tag);
            side.co_belligerents = *co_belligerents;
            side.factions.clone_from(factions);
            side.colour_faction.clone_from(colour_faction);
        }
        let at = side.reads.first().map_or(hour, |read| read.0);
        if !side.factions.is_empty() {
            for (id, name) in &mut side.factions {
                *name = localise(id, at);
            }
            side.name = side
                .factions
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(" / ");
        } else if let Some(id) = &side.faction {
            side.name = localise(id, at);
        }
        for (at, name, _, _, _, factions, _) in &mut side.reads {
            if !factions.is_empty() {
                for (id, name) in factions.iter_mut() {
                    *name = localise(id, *at);
                }
                *name = factions
                    .iter()
                    .map(|(_, name)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ");
            } else if let Some(id) = &side.faction {
                *name = localise(id, *at);
            }
        }
    }
    warnings.extend(refused.lines("faction names are"));
    if let Err(error) = names {
        warnings.push(format!("Air region names show ids: {error}"));
    }
    reply_warnings.extend(warnings);
}

#[cfg(test)]
mod scale_tests;
#[cfg(test)]
mod tests;
