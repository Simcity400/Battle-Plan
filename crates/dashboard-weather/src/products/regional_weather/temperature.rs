//! One content binding shared by branches, and one reconstruction memo owned by
//! each branch observer. Province drills keep a separate source/branch memo
//! and use the same binding and reconstruction without walking branch frames.

use dashboard_base::{Source, catalogue, content};
use dashboard_core::config::ContentOptions;
use dashboard_keyed::Values;
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
    sync::{Arc, atomic::AtomicBool},
};

/// Membership is each region's native province vector, not the province's
/// final region backlink (overlapping templates retain both vectors).
struct Bound {
    members: BTreeMap<u32, Vec<u32>>,
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

fn read(layers: &ResolvedLayers, mut warnings: Vec<String>) -> io::Result<Bound> {
    let (graph, geometry) = ProvinceGraph::read(layers)?;
    let regions = regions::read(layers)?;
    warnings.extend(regions.notes.iter().cloned());
    Ok(Bound {
        members: membership(&regions, &geometry),
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
    memo: Memo,
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

    pub(super) fn bind_checked(
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
        if loaded
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
        {
            return loaded.map(|_| ());
        }
        self.binding = Some(binding);
        self.memo = Memo::default();
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

    pub(super) fn observe(&mut self, frame: &DecodedFrame, values: &mut Values) -> bool {
        let Some(content) = &self.content else {
            if self.status.as_deref() == Some("available") {
                // A restored observer cannot advance without rebinding its
                // runtime content, even though its published past stays exact.
                self.status = None;
            }
            return false;
        };
        let content = Arc::clone(content);
        let states = match self.states(frame) {
            Ok(states) => states,
            Err(reason) => {
                *self.refused.entry(reason).or_default() += 1;
                return false;
            }
        };
        let mut known = false;
        let mut missing = 0_u64;
        for (&region, members) in &content.members {
            if let Some(mean) = average(members, states) {
                // Names: <regionId>|temp, fixed5 degrees Celsius, all members.
                values.push(format!("{region}|temp"), mean);
                known = true;
            } else {
                missing += 1;
            }
        }
        if missing > 0 {
            *self
                .refused
                .entry(
                    "region membership is empty or a member province temperature is unavailable"
                        .into(),
                )
                .or_default() += missing;
        }
        known
    }
}

mod retained;

#[cfg(test)]
mod tests;
