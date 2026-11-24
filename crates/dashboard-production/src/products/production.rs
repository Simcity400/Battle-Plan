
pub mod breakdown;
pub mod catalogue;
pub mod live;
#[cfg(test)]
mod tests;

pub use breakdown::ProductionLineBreakdown;
pub use catalogue::{EquipmentDesigns, IndustrialOrganisations, ProductionLicences};
pub use live::ProductionLinesAt;

use dashboard_query::read;
use replay_recording::columnar_frames::Recording;
use replay_columnar::summary::Source as Column;
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const DECIMALS: u8 = 5;

pub fn weekly_units_raw(speed_raw: i64, cost_raw: i64) -> Option<i64> {
    if cost_raw <= 0 {
        return None;
    }
    i64::try_from(i128::from(speed_raw) * 700_000 / i128::from(cost_raw)).ok()
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LineKind {
    Military,
    Naval,
    ShipRefit,
    RailwayGun,
}

impl LineKind {
    pub fn of(kind: i64) -> Option<Self> {
        use engine_native::cproduction_status::equipment_line as native;
        match u8::try_from(kind).ok()? {
            native::KIND_MILITARY => Some(Self::Military),
            native::KIND_NAVAL => Some(Self::Naval),
            native::KIND_SHIP_REFIT => Some(Self::ShipRefit),
            native::KIND_RAILWAY_GUN => Some(Self::RailwayGun),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineLife {
    pub handle: u64,
    pub first: i32,
    pub last: i32,
    pub country: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunLines {
    pub lines: Vec<LineLife>,
}

fn run_hours(recording: &Recording, run: u32) -> io::Result<Option<(i32, i32)>> {
    let held = recording.run(run)?;
    if held.ordinals == 0 {
        return Ok(None);
    }
    Ok(Some((held.first_hour, held.first_hour + (held.ordinals - 1) as i32)))
}

pub fn run_lines(recording: &Recording, run: u32) -> io::Result<RunLines> {
    let Some((from, to)) = run_hours(recording, run)? else { return Ok(RunLines::default()) };
    let country = dashboard_declare::field(TableId::ProductionLines, "country")?;
    let mut lines = Vec::new();
    for (key, runs) in read::runs(recording, run, TableId::ProductionLines, Column::Cell(country), None, from, to)? {
        for (first, last, value) in runs {
            if let Some(country) = value {
                lines.push(LineLife { handle: key as u64, first, last, country: country as u32 });
            }
        }
    }
    lines.sort_unstable_by_key(|life| (life.handle, life.first));
    Ok(RunLines { lines })
}

enum RunKey {}

type CachedRuns = dashboard_base::queries::RunCache<RunKey, RunLines>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    CachedRuns::forget(source);
}

#[derive(Clone)]
pub(crate) struct ProductionIndex {
    replay: std::sync::Arc<replay::campaign::Replay>,
    state: std::sync::Arc<std::sync::Mutex<CachedRuns>>,
}

impl ProductionIndex {
    pub(crate) fn of(source: &dashboard_base::Source) -> Self {
        Self { replay: source.replay(), state: source.state::<CachedRuns>() }
    }

    pub(crate) fn run(&self, run: u32) -> io::Result<std::sync::Arc<RunLines>> {
        CachedRuns::held(&self.state, run, || run_lines(self.replay.recording(), run))
    }
}

pub fn cached_production_lines(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<RunLines>> {
    ProductionIndex::of(source).run(run)
}
