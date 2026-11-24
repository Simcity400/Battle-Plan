
pub mod breakdown;
pub mod catalogue;
pub mod live;
#[cfg(test)]
mod tests;

pub use breakdown::ProductionLineBreakdown;
pub use catalogue::{EquipmentDesigns, IndustrialOrganisations, ProductionLicences};
pub use live::ProductionLinesAt;

use dashboard_base::products::Country;
use dashboard_base::span::HourSpan;
use dashboard_query::{Query, identity::Holder, read};
use replay_recording::columnar_frames::Recording;
use replay_columnar::summary::Source as Column;
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const DECIMALS: u8 = 5;

pub(crate) fn index_of(source: &dashboard_base::Source) -> impl Fn(u32) -> io::Result<std::sync::Arc<RunLines>> + '_ {
    move |run| cached_production_lines(source, run)
}
pub const DETAIL_HOURS: i32 = 720;

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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputPart {
    pub variant: String,
    #[doc = r#" The organisation's reference; "0" for none."#]
    pub mio: String,
    #[doc = " Units delivered since `sinceHour`, as a decimal string."]
    pub units: String,
    #[doc = " False when a tick could not be counted: `units` then leave it out."]
    pub exact: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputRun {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Its parts by row ordinal; the total is their sum, exact only when every part is."]
    pub parts: Vec<OutputPart>,
    #[doc = " The hour the branch's observation of the line began."]
    pub since_hour: i32,
    #[doc = " The line was first seen after the branch's first complete census: the units are its whole life's."]
    pub whole_life: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineLife {
    pub handle: u64,
    pub first: i32,
    pub last: i32,
    pub country: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputLife {
    pub key: u128,
    pub first: i32,
    pub last: i32,
    pub ended: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunLines {
    pub lines: Vec<LineLife>,
    pub outputs: Vec<OutputLife>,
}

impl RunLines {
    pub fn outputs_of(&self, handle: u64) -> &[OutputLife] {
        let low = u128::from(handle) << 32;
        let high = low | u128::from(u32::MAX);
        let start = self.outputs.partition_point(|row| row.key < low);
        let end = self.outputs.partition_point(|row| row.key <= high);
        &self.outputs[start..end]
    }
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
    let ended = dashboard_declare::field(TableId::ProductionLineOutput, "ended")?;
    let mut outputs = Vec::new();
    for (key, runs) in read::runs(recording, run, TableId::ProductionLineOutput, Column::Cell(ended), None, from, to)? {
        let mut life: Option<OutputLife> = None;
        for (first, last, value) in runs {
            let Some(value) = value else { continue };
            let held = life.get_or_insert(OutputLife { key, first, last, ended: None });
            held.last = last;
            if value != 0 && held.ended.is_none() {
                held.ended = Some(first);
            }
        }
        outputs.extend(life);
    }
    outputs.sort_unstable_by_key(|row| row.key);
    Ok(RunLines { lines, outputs })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub run: u32,
    pub first: i32,
    pub last: i32,
    pub starts_segment: bool,
    pub held_from: i32,
    pub held_to: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub key: String,
    pub handle: u64,
    pub pieces: Vec<Piece>,
}

impl Appearance {
    pub fn first(&self) -> i32 {
        self.pieces.first().map_or(i32::MAX, |piece| piece.first)
    }

    pub fn last(&self) -> i32 {
        self.pieces.last().map_or(i32::MIN, |piece| piece.last)
    }
}

pub fn parse_key(key: &str) -> Option<(u64, u32)> {
    let (handle, appearance) = key.split_once('#')?;
    let appearance = appearance.parse::<u32>().ok().filter(|appearance| *appearance > 0)?;
    Some((handle.parse().ok()?, appearance))
}

pub fn appearances(query: &Query<'_>, tag: &Country, within: Option<&[HourSpan]>, from: i32, to: i32, lines: &dyn Fn(u32) -> io::Result<std::sync::Arc<RunLines>>) -> io::Result<Vec<Appearance>> {
    let recording = query.replay.recording();
    let mut pieces: BTreeMap<u64, Vec<Piece>> = BTreeMap::new();
    for stretch in query.timeline.within(from, to) {
        let held = query.holders.within(tag, stretch.first, stretch.last);
        if !held.iter().any(|run| matches!(run.holder, Holder::Found { .. })) {
            continue;
        }
        let index = lines(stretch.run)?;
        let links = dashboard_query::cost::storage(|| dashboard_base::queries::country_ids::run_links(recording, stretch.run))?;
        for run in &held {
            let Holder::Found { slot, .. } = run.holder else { continue };
            let spans: Vec<(i32, i32)> = match within {
                None => vec![(run.first, run.last)],
                Some(within) => within.iter().map(|span| (run.first.max(span.first_hour), run.last.min(span.last_hour))).filter(|(first, last)| first <= last).collect(),
            };
            for (held_from, held_to) in spans {
                for life in &index.lines {
                    let from = life.first.max(held_from);
                    if from > life.last.min(held_to) {
                        continue;
                    }
                    for (linked, first, owned_to) in links.within(life.country, from, held_to) {
                        let last = owned_to.min(life.last);
                        if linked == Some(slot) && first <= last {
                            pieces.entry(life.handle).or_default().push(Piece { run: stretch.run, first, last, starts_segment: true, held_from, held_to: owned_to });
                        }
                    }
                }
            }
        }
    }
    let mut runs_of: BTreeMap<u64, Vec<u32>> = pieces.keys().map(|handle| (*handle, Vec::new())).collect();
    for stretch in query.timeline.within(query.timeline.first, query.timeline.last) {
        let index = lines(stretch.run)?;
        for life in &index.lines {
            if let Some(runs) = runs_of.get_mut(&life.handle)
                && life.first <= stretch.last
                && stretch.first <= life.last
                && !runs.contains(&stretch.run)
            {
                runs.push(stretch.run);
            }
        }
    }
    let mut out = Vec::new();
    for (handle, mut held) in pieces {
        held.sort_unstable_by_key(|piece| piece.first);
        let runs = runs_of.get(&handle).map(Vec::as_slice).unwrap_or_default();
        let mut current: Option<Appearance> = None;
        for mut piece in held {
            let previous = current.as_ref().and_then(|current| current.pieces.last());
            let joins = previous.is_some_and(|last| last.run == piece.run);
            piece.starts_segment = !previous.is_some_and(|last| last.run == piece.run && last.last.checked_add(1) == Some(piece.first));
            match current.as_mut() {
                Some(current) if joins => current.pieces.push(piece),
                _ => {
                    out.extend(current.take());
                    let appearance = runs.iter().position(|run| *run == piece.run).map_or(0, |at| at + 1);
                    current = Some(Appearance { key: format!("{handle}#{appearance}"), handle, pieces: vec![piece] });
                }
            }
        }
        out.extend(current);
    }
    out.sort_by_key(|appearance| (appearance.first(), appearance.handle));
    Ok(out)
}

pub fn held_hours(query: &Query<'_>, tag: &Country, within: Option<&[HourSpan]>, from: i32, to: i32) -> Vec<HourSpan> {
    let mut spans: Vec<HourSpan> = Vec::new();
    for run in query.holders.within(tag, from, to) {
        if !matches!(run.holder, Holder::Found { .. }) {
            continue;
        }
        let clipped: Vec<(i32, i32)> = match within {
            None => vec![(run.first, run.last)],
            Some(within) => within.iter().map(|span| (run.first.max(span.first_hour), run.last.min(span.last_hour))).filter(|(first, last)| first <= last).collect(),
        };
        for (first, last) in clipped {
            match spans.last_mut() {
                Some(held) if held.last_hour.checked_add(1) == Some(first) => held.last_hour = last,
                _ => spans.push(HourSpan { first_hour: first, last_hour: last }),
            }
        }
    }
    spans
}

pub fn window_hours(window: i32) -> Option<(i32, i32)> {
    let first = window.checked_mul(DETAIL_HOURS)?;
    Some((first, first.checked_add(DETAIL_HOURS - 1)?))
}

pub fn joined(columns: &[&read::Runs], first: i32, last: i32) -> Vec<(i32, i32, Vec<Option<i64>>)> {
    let mut cuts: Vec<i32> = vec![first];
    for runs in columns {
        cuts.extend(runs.iter().map(|run| run.0).filter(|&hour| hour > first && hour <= last));
        cuts.extend(runs.iter().filter_map(|run| run.1.checked_add(1)).filter(|&hour| hour > first && hour <= last));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let value = |runs: &read::Runs, hour: i32| runs.iter().find(|run| run.0 <= hour && hour <= run.1).and_then(|run| run.2);
    let mut out: Vec<(i32, i32, Vec<Option<i64>>)> = Vec::with_capacity(cuts.len());
    for (at, &start) in cuts.iter().enumerate() {
        let end = cuts.get(at + 1).map_or(last, |next| next - 1);
        let values: Vec<Option<i64>> = columns.iter().map(|runs| value(runs, start)).collect();
        match out.last_mut() {
            Some(held) if held.2 == values => held.1 = end,
            _ => out.push((start, end, values)),
        }
    }
    out
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
