//! One content binding shared by branches, and one reconstruction memo owned by
//! each branch observer. Province drills keep a separate source/branch memo
//! and use the same binding and reconstruction without walking branch frames.

use super::admission;
use dashboard_base::prepared::facts::{Credit, MemoryBudget};
use dashboard_base::{Source, catalogue, content};
use dashboard_core::config::ContentOptions;
use dashboard_keyed::{Delta, Values};
use installed_content_core::{
    ResolvedLayers, province_geometry::ProvinceGeometry, province_graph::ProvinceGraph,
};
use installed_content_regions::regions::{self, StrategicRegions};
use replay_data::{
    rows::tables,
    temperature::{Content, Memo, State},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

/// Membership is each region's native province vector, not the province's
/// final region backlink (overlapping templates retain both vectors).
struct Bound {
    members: BTreeMap<u32, Vec<u32>>,
    inverse: Mutex<Option<Arc<Inverse>>>,
    pass: Content,
    warnings: Vec<String>,
}

pub(super) fn membership(
    regions: &StrategicRegions,
    geometry: &ProvinceGeometry,
) -> BTreeMap<u32, Vec<u32>> {
    regions
        .regions
        .iter()
        .map(|(&id, region)| {
            (
                id,
                region.native_provinces(|province| geometry.centroids.contains_key(&province)),
            )
        })
        .collect()
}

struct Inverse {
    rows: Vec<(u32, u32)>,
    _credit: Credit,
}

struct InverseLease {
    owner: Arc<Bound>,
    index: Option<Arc<Inverse>>,
}

impl std::ops::Deref for InverseLease {
    type Target = Inverse;
    fn deref(&self) -> &Self::Target {
        self.index.as_deref().expect("live inverse lease")
    }
}

impl Drop for InverseLease {
    fn drop(&mut self) {
        let mut cached = self
            .owner
            .inverse
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(index) = self.index.take() else {
            return;
        };
        // Only the cache and live leases own strong references to this index.
        if Arc::strong_count(&index) == 2
            && cached
                .as_ref()
                .is_some_and(|cached| Arc::ptr_eq(cached, &index))
        {
            *cached = None;
        }
        // Release allocation and credit before a competing initializer can acquire the mutex.
        drop(index);
    }
}

impl Inverse {
    fn regions(&self, province: u32) -> impl Iterator<Item = u32> + '_ {
        let first = self.rows.partition_point(|row| row.0 < province);
        self.rows[first..]
            .iter()
            .take_while(move |row| row.0 == province)
            .map(|row| row.1)
    }
}

impl Inverse {
    fn allocation_bytes(count: usize) -> io::Result<usize> {
        admission::add(
            admission::add(2 * size_of::<usize>(), size_of::<Self>())?,
            admission::bytes::<(u32, u32)>(count)?,
        )
    }
}

impl Bound {
    fn lease_inverse(
        self: &Arc<Self>,
        budget: &MemoryBudget,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<InverseLease> {
        let mut cached = self
            .inverse
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        check()?;
        if let Some(inverse) = cached.as_ref() {
            return Ok(InverseLease {
                owner: Arc::clone(self),
                index: Some(Arc::clone(inverse)),
            });
        }
        let count = self
            .members
            .values()
            .try_fold(0, |count, members| admission::add(count, members.len()))?;
        let credit = admission::admit_in(
            budget,
            Inverse::allocation_bytes(count)?,
            "shared inverse membership",
        )?;
        let mut rows = Vec::new();
        rows.try_reserve_exact(count).map_err(|error| {
            admission::pressure(
                "shared inverse membership",
                io::Error::new(io::ErrorKind::OutOfMemory, error),
            )
        })?;
        for (&region, provinces) in &self.members {
            check()?;
            rows.extend(provinces.iter().map(|&province| (province, region)));
        }
        rows.sort_unstable();
        check()?;
        let inverse = Arc::new(Inverse {
            rows,
            _credit: credit,
        });
        *cached = Some(Arc::clone(&inverse));
        Ok(InverseLease {
            owner: Arc::clone(self),
            index: Some(inverse),
        })
    }
}

fn read(layers: &ResolvedLayers, mut warnings: Vec<String>) -> io::Result<Bound> {
    let (graph, geometry) = ProvinceGraph::read(layers)?;
    let regions = regions::read(layers)?;
    warnings.extend(regions.notes.iter().cloned());
    let members = membership(&regions, &geometry);
    Ok(Bound {
        inverse: Mutex::default(),
        members,
        pass: installed_content_weather::weather::temperature::read_with(layers, &graph, &geometry)?,
        warnings,
    })
}

pub(super) fn content_warnings(layers: &ResolvedLayers) -> Vec<String> {
    match read(layers, Vec::new()) {
        Ok(bound) => bound.warnings,
        Err(error) => vec![format!(
            "1 weather temperature content unavailable: {error}"
        )],
    }
}

/// The app's equal-province mean, rounded toward zero to fixed5 degrees
/// Celsius. Wide accumulation avoids overflowing even at the live slot extent.
/// Empty membership and any missing province have no average, never zero.
#[cfg(test)]
fn average(members: &[u32], states: &[State]) -> Option<i128> {
    if members.is_empty() {
        return None;
    }
    let sum = members.iter().try_fold(0_i128, |sum, &id| {
        states
            .get(id as usize)
            .map(|state| sum + i128::from(state.temperature))
    })?;
    Some(sum / members.len() as i128)
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Observation {
    /// Published status/diagnostics survive continuation without serializing
    /// content, a fake frame, or a temperature simulation checkpoint.
    status: Option<String>,
    content_warnings: Vec<String>,
    refused: BTreeMap<String, u64>,
    #[serde(skip)]
    binding: Option<(
        dashboard_base::SourceKey,
        Option<catalogue::RecordedBinding>,
    )>,
    #[serde(skip)]
    content: Option<Arc<Bound>>,
    #[serde(skip)]
    inverse: Option<InverseLease>,
    #[serde(skip)]
    memo: Memo,
    #[serde(skip)]
    certified: Option<(u64, i32)>,
    #[serde(skip)]
    aggregates: Option<Aggregates>,
}

struct RegionSum {
    name: String,
    sum: i128,
    count: usize,
    total: usize,
    mean: Option<i128>,
    dirty: bool,
}

struct Aggregates {
    point: Option<(u64, i32)>,
    regions: BTreeMap<u32, RegionSum>,
    missing: u64,
    _credit: Credit,
}

impl Aggregates {
    fn new(content: &Bound, states: &[State]) -> io::Result<Self> {
        let per_region = admission::add(admission::tree_entry::<u32, RegionSum>()?, 15)?;
        let credit = admission::admit(
            content
                .members
                .len()
                .checked_mul(per_region)
                .ok_or_else(|| {
                    admission::pressure("aggregate size", io::ErrorKind::OutOfMemory.into())
                })?,
            "region aggregates",
        )?;
        let mut missing = 0;
        let mut regions = BTreeMap::new();
        for (&id, members) in &content.members {
            let (sum, count) = members
                .iter()
                .filter_map(|&id| states.get(id as usize))
                .fold((0i128, 0), |(sum, count), state| {
                    (sum + i128::from(state.temperature), count + 1)
                });
            let mean = (count == members.len() && count != 0).then(|| sum / count as i128);
            missing += u64::from(mean.is_none());
            regions.insert(
                id,
                RegionSum {
                    name: admission::name(id, "temp")?,
                    sum,
                    count,
                    total: members.len(),
                    mean,
                    dirty: false,
                },
            );
        }
        Ok(Self {
            point: None,
            regions,
            missing,
            _credit: credit,
        })
    }
}

impl Observation {
    pub(super) fn bind(
        &mut self,
        source: &Source,
        options: Option<&ContentOptions>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.bind_checked(source, options, &mut || {
            dashboard_text::metadata::check_cancel(cancelled)
        })
    }

    pub(super) fn bind_states(
        &mut self,
        source: &Source,
        options: Option<&ContentOptions>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if self.inverse.is_some() {
            self.retire();
        }
        let result = self.bind_inner(source, options, &mut || {
            dashboard_text::metadata::check_cancel(cancelled)
        });
        if result.is_err() {
            self.retire();
        }
        result
    }

    pub(super) fn bind_checked(
        &mut self,
        source: &Source,
        options: Option<&ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.bind_in(source, options, check, &MemoryBudget::preparation())
    }

    fn bind_in(
        &mut self,
        source: &Source,
        options: Option<&ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
        budget: &MemoryBudget,
    ) -> io::Result<()> {
        let result = (|| {
            self.bind_inner(source, options, check)?;
            if self.inverse.is_none()
                && let Some(content) = &self.content
            {
                let inverse = content.lease_inverse(budget, check)?;
                check()?;
                self.inverse = Some(inverse);
            }
            Ok(())
        })();
        if result.is_err() {
            self.retire();
        }
        result
    }

    pub(super) fn retire(&mut self) {
        self.inverse = None;
        self.content = None;
        self.binding = None;
        self.memo = Memo::default();
        self.certified = None;
        self.aggregates = None;
        self.status = None;
        self.content_warnings.clear();
    }

    fn bind_inner(
        &mut self,
        source: &Source,
        options: Option<&ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        let binding = (
            source.key().clone(),
            options.map(catalogue::recorded_binding),
        );
        if self.binding.as_ref() == Some(&binding) {
            return Ok(());
        }
        self.retire();
        let loaded = (|| {
            let options = options.ok_or_else(|| dashboard_core::invalid("game folders are not chosen; temperature needs the recorded stack's province geometry, adjacency, terrain, weather settings and strategic-region membership/periods"))?;
            source
                .state::<catalogue::KeyedResults<catalogue::RecordedBinding, Bound>>()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get_or_init(&catalogue::recorded_binding(options), |_| {
                    let mut warnings = Vec::new();
                    content::check_replay(
                        source.replay_ref(),
                        options.allow_unverified,
                        &mut warnings,
                    )?;
                    let resolved = source.content(&options.options)?;
                    check()?;
                    let bound = read(&resolved.layers, warnings)?;
                    check()?;
                    Ok(bound)
                })
        })();
        if loaded.as_ref().is_err_and(|error| {
            matches!(
                error.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::OutOfMemory
            )
        }) {
            return loaded.map(|_| ());
        }
        self.binding = Some(binding);
        self.memo = Memo::default();
        self.certified = None;
        self.aggregates = None;
        match loaded {
            Ok(content) => {
                self.status = Some("available".into());
                self.content_warnings = content.warnings.clone();
                self.content = Some(content);
            }
            Err(error) => {
                self.status = Some(format!("Temperature unavailable: {error}"));
                self.content_warnings.clear();
                self.content = None;
            }
        }
        Ok(())
    }

    pub(super) fn status(&self) -> &str {
        self.status.as_deref().unwrap_or(super::TEMPERATURE_REFUSAL)
    }

    pub(super) fn warnings(&self) -> Vec<String> {
        let mut warnings = self.content_warnings.clone();
        warnings.extend(self.refused.iter().map(|(reason, count)| {
            format!("{count} weather temperature observations not shown: {reason}")
        }));
        warnings
    }

    /// Reconstruct once from this reached frame's anchored packet. No held
    /// word or partial table can substitute for a proven current temperature.
    pub(super) fn states(&mut self, frame: &DecodedFrame) -> Result<&[State], String> {
        let content = self
            .content
            .as_ref()
            .ok_or_else(|| self.status().to_owned())?;
        let rows = frame
            .complete::<tables::WeatherProvinces>()
            .map_err(|error| error.to_string())?;
        let states = self
            .memo
            .states(frame.branch, frame.hour, rows, Ok(&content.pass))?;
        if states.len() != content.pass.provinces.len() {
            return Err(format!(
                "content has {} weather province slots, the recording {}",
                content.pass.provinces.len(),
                states.len()
            ));
        }
        Ok(states)
    }

    fn refresh(
        &mut self,
        frame: &DecodedFrame,
        changed: Option<&[u128]>,
        publish: impl FnMut(&str, Option<i128>),
    ) -> io::Result<bool> {
        let changed = changed.filter(|_| {
            self.certified.is_some_and(|(branch, hour)| {
                branch == frame.branch && hour.checked_add(1) == Some(frame.hour)
            })
        });
        self.refresh_with(
            frame.branch,
            frame.hour,
            |memo, content| {
                let rows = frame
                    .complete::<tables::WeatherProvinces>()
                    .map_err(|error| error.to_string())?;
                let states = memo.states_changed(
                    frame.branch,
                    frame.hour,
                    rows,
                    Ok(&content.pass),
                    changed,
                )?;
                if states.len() != content.pass.provinces.len() {
                    return Err(format!(
                        "content has {} weather province slots, the recording {}",
                        content.pass.provinces.len(),
                        states.len()
                    ));
                }
                Ok(())
            },
            publish,
        )
    }

    fn refresh_with(
        &mut self,
        branch: u64,
        hour: i32,
        observe: impl FnOnce(&mut Memo, &Bound) -> Result<(), String>,
        publish: impl FnMut(&str, Option<i128>),
    ) -> io::Result<bool> {
        let result = self.refresh_inner(branch, hour, observe, publish);
        if result.is_err() {
            self.retire();
        }
        result
    }

    fn refresh_inner(
        &mut self,
        branch: u64,
        hour: i32,
        observe: impl FnOnce(&mut Memo, &Bound) -> Result<(), String>,
        mut publish: impl FnMut(&str, Option<i128>),
    ) -> io::Result<bool> {
        let Some(content) = self.content.as_ref().map(Arc::clone) else {
            if self.status.as_deref() == Some("available") {
                self.status = None;
            }
            return Ok(false);
        };
        let previous = self.certified.filter(|&(previous_branch, previous_hour)| {
            previous_branch == branch && previous_hour.checked_add(1) == Some(hour)
        });
        self.memo.admit_changes(&content.pass)?;
        let observed = observe(&mut self.memo, &content);
        if let Err(reason) = observed {
            self.certified = None;
            *self.refused.entry(reason).or_default() += 1;
            if let Some(aggregates) = &mut self.aggregates {
                aggregates.point = None;
                for region in aggregates.regions.values_mut() {
                    if region.mean.take().is_some() {
                        publish(&region.name, None);
                    }
                }
            }
            return Ok(false);
        }
        self.certified = Some((branch, hour));
        let incremental = self
            .aggregates
            .as_ref()
            .is_some_and(|aggregates| previous.is_some() && aggregates.point == previous);
        match (incremental, self.memo.changes()) {
            (true, Some(changed)) => {
                let aggregates = self.aggregates.as_mut().unwrap();
                let inverse = self.inverse.as_ref().expect("bound inverse lease");
                let count = changed
                    .iter()
                    .try_fold(0, |count, &(province, _, _)| {
                        admission::add(count, inverse.regions(province).count())
                    })?
                    .min(aggregates.regions.len());
                let (mut dirty, _credit) = admission::vector(count, "changed regions")?;
                for &(province, before, now) in changed {
                    for id in inverse.regions(province) {
                        let region = aggregates.regions.get_mut(&id).unwrap();
                        region.sum += i128::from(now) - i128::from(before);
                        if !region.dirty {
                            region.dirty = true;
                            dirty.push(id);
                        }
                    }
                }
                dirty.sort_unstable();
                for id in dirty {
                    let region = aggregates.regions.get_mut(&id).unwrap();
                    region.dirty = false;
                    let mean = (region.count == region.total && region.total != 0)
                        .then(|| region.sum / region.total as i128);
                    if mean != region.mean {
                        publish(&region.name, mean);
                        region.mean = mean;
                    }
                }
            }
            _ => {
                let mut next = Aggregates::new(&content, &self.memo.simulation().unwrap().states)?;
                for (&id, region) in &next.regions {
                    let before = self
                        .aggregates
                        .as_ref()
                        .and_then(|held| held.regions.get(&id))
                        .and_then(|held| held.mean);
                    if before != region.mean {
                        publish(&region.name, region.mean);
                    }
                }
                next.point = Some((branch, hour));
                self.aggregates = Some(next);
            }
        }
        let aggregates = self.aggregates.as_mut().unwrap();
        aggregates.point = Some((branch, hour));
        if aggregates.missing > 0 {
            *self
                .refused
                .entry(
                    "region membership is empty or a member province temperature is unavailable"
                        .into(),
                )
                .or_default() += aggregates.missing;
        }
        Ok(aggregates.missing < aggregates.regions.len() as u64)
    }

    pub(super) fn observe(
        &mut self,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let known = self.refresh(frame, None, |_, _| {})?;
        if known {
            for region in self.aggregates.as_ref().unwrap().regions.values() {
                if let Some(mean) = region.mean {
                    values.push(region.name.clone(), mean);
                }
            }
        }
        Ok(known)
    }

    pub(super) fn observe_changes(
        &mut self,
        frame: &DecodedFrame,
        changed: Option<&[u128]>,
        delta: &mut Delta,
    ) -> io::Result<bool> {
        self.refresh(frame, changed, |name, value| match value {
            Some(value) => delta.set(name.to_owned(), value),
            None => delta.remove(name.to_owned()),
        })
    }

    pub(super) fn held(&self, bytes: &mut dashboard_base::prepared::prepass::HeldBytes) {
        bytes.add(self.memo.change_bytes());
        if let Some(inverse) = &self.inverse {
            if bytes.once(Arc::as_ptr(inverse.index.as_ref().unwrap())) {
                bytes.add(
                    2 * size_of::<usize>()
                        + size_of::<Inverse>()
                        + inverse.rows.capacity() * size_of::<(u32, u32)>(),
                );
            }
        }
        if let Some(aggregates) = &self.aggregates {
            bytes.tree::<u32, RegionSum>(aggregates.regions.len());
            for region in aggregates.regions.values() {
                bytes.heap(&region.name);
            }
        }
    }
}

mod retained;

#[cfg(test)]
mod tests;
