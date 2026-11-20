//! Production lines and the production catalogue, answered from the recording's own tables.
//!
//! A country's lines over a branch are the `ProductionLines` rows its holders' raw countries
//! produced under, found from each run's index of line lives (read once per run at open,
//! [`RunLines`]); a line's steps are the change points of its variant and factories, its final
//! count the `ProductionLineOutput` rows published as ended. The live table, the licences and the
//! organisations are rows at an hour ([`super::at_hour`]); a line's values over a window and its
//! modifier breakdown are read over that window only.
//!
//! Ordinary line values are recorded `ProductionLines` fields: `cost_raw` is the IC one unit
//! costs (fixed5, after the organisation's cost bonus); `speed_raw` the IC added at each tick, so
//! units per week are `speed × 700000 ÷ cost` ([`weekly_units_raw`]); `efficiency_raw` Q15 (100% is
//! 3276800); `priority` the queue place from 0; `amount` the units still ordered, −1 for no limit.

pub mod breakdown;
pub mod catalogue;
pub mod lines;
pub mod live;
pub mod names;
#[cfg(test)]
mod tests;

pub use breakdown::ProductionLineBreakdown;
pub use catalogue::{EquipmentDesigns, IndustrialOrganisations, ProductionLicences};
pub use lines::{ProductionLineDetail, ProductionLines};
pub use live::ProductionLinesAt;

use dashboard_base::products::Country;
use crate::span::HourSpan;
use dashboard_query::{Query, identity::Holder, read};
use replay::columnar_frames::Recording;
use replay_columnar::summary::Source as Column;
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

/// Fixed-point decimals of IC and units values.
pub const DECIMALS: u8 = 5;
/// The hours one `production-line-detail` window and one breakdown cover: 30 days.
pub const DETAIL_HOURS: i32 = 720;

/// Units per week by the game's weekly formula, fixed5: `speed × 7 × 100000 ÷ cost`, truncated;
/// none without a positive cost.
pub fn weekly_units_raw(speed_raw: i64, cost_raw: i64) -> Option<i64> {
    if cost_raw <= 0 {
        return None;
    }
    i64::try_from(i128::from(speed_raw) * 700_000 / i128::from(cost_raw)).ok()
}

/// The kind of line, by its native class.
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
        use engine_native::cequipment_production_line as native;
        match u8::try_from(kind).ok()? {
            native::KIND_MILITARY => Some(Self::Military),
            native::KIND_NAVAL => Some(Self::Naval),
            native::KIND_SHIP_REFIT => Some(Self::ShipRefit),
            native::KIND_RAILWAY_GUN => Some(Self::RailwayGun),
            _ => None,
        }
    }
}

/// One (variant, organisation) part of a line's output.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputPart {
    pub variant: String,
    /// The organisation's reference; "0" for none.
    pub mio: String,
    /// Units delivered since `sinceHour`, as a decimal string.
    pub units: String,
    /// False when a tick could not be counted: `units` then leave it out.
    pub exact: bool,
}

/// A line's output rows over hours they held still.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Its parts by row ordinal; the total is their sum, exact only when every part is.
    pub parts: Vec<OutputPart>,
    /// The hour the branch's observation of the line began.
    pub since_hour: i32,
    /// The line was first seen after the branch's first complete census: the units are its whole life's.
    pub whole_life: bool,
}

/// A produced variant's names and icon as the game's production row shows them, read at `hour`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct VariantName {
    pub id: String,
    /// The hour its names were read; absent in a reply at an hour, whose hour it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    /// False where the variant had no definition row at that hour; the other fields are then empty.
    pub known: bool,
    /// The equipment type's script key.
    pub token: String,
    pub name: Option<String>,
    pub type_name: Option<String>,
    /// Why the names are unknown, when they are.
    pub name_unknown: Option<String>,
    /// The icon's key in `icons`; null when it shows none or it is unknown.
    pub icon: Option<String>,
    pub icon_unknown: Option<String>,
}

/// A military-industrial organisation as the game names and pictures it, read at `hour`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OrganisationName {
    pub id: String,
    /// The hour its names were read; absent in a reply at an hour, whose hour it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    /// The script key; empty where its row was not read.
    pub token: String,
    pub name: Option<String>,
    pub name_unknown: Option<String>,
    pub icon: Option<String>,
    pub icon_unknown: Option<String>,
}

/// One picture, shared by every variant or organisation showing it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentIcon {
    pub key: String,
    /// An artwork URL.
    pub image: String,
}

/// A line's life in one run under one producing country.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineLife {
    pub handle: u64,
    pub first: i32,
    pub last: i32,
    /// The producing country's raw reference.
    pub country: u32,
}

/// An output row's life in one run, and the first hour it was published as its line's last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputLife {
    pub key: u128,
    pub first: i32,
    pub last: i32,
    pub ended: Option<i32>,
}

/// One run's production lines and output rows, read once from their tables while the
/// recording is open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunLines {
    /// Ascending by handle, then first hour.
    pub lines: Vec<LineLife>,
    /// Ascending by key.
    pub outputs: Vec<OutputLife>,
}

impl RunLines {
    /// The output rows of line `handle` (keys `handle:ordinal`).
    pub fn outputs_of(&self, handle: u64) -> &[OutputLife] {
        let low = u128::from(handle) << 32;
        let high = low | u128::from(u32::MAX);
        let start = self.outputs.partition_point(|row| row.key < low);
        let end = self.outputs.partition_point(|row| row.key <= high);
        &self.outputs[start..end]
    }
}

/// The hours of run `run`: its first and last.
fn run_hours(recording: &Recording, run: u32) -> io::Result<Option<(i32, i32)>> {
    let held = recording.run(run)?;
    if held.ordinals == 0 {
        return Ok(None);
    }
    Ok(Some((held.first_hour, held.first_hour + (held.ordinals - 1) as i32)))
}

/// Run `run`'s line lives (from `ProductionLines.country`'s runs) and output row lives (from
/// `ProductionLineOutput.ended`'s runs): one pass over each column's tracks, no hour stepped.
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

/// One stretch of a line under the asked tag's holder: in run `run`, hours `first..=last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub run: u32,
    pub first: i32,
    pub last: i32,
    /// It follows hours the line was not the tag's or not recorded.
    pub starts_segment: bool,
    /// The first hour of the holder run (within its stretch) it lies in.
    pub held_from: i32,
    /// The last hour of that holder run.
    pub held_to: i32,
}

/// One appearance of a line handle in the tag's branch: consecutive pieces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Appearance {
    /// Unique in the branch: the handle, and which appearance of it this is (from 1).
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

/// A line key `handle#appearance`, refused unless both are numbers and the appearance positive.
pub fn parse_key(key: &str) -> Option<(u64, u32)> {
    let (handle, appearance) = key.split_once('#')?;
    let appearance = appearance.parse::<u32>().ok().filter(|appearance| *appearance > 0)?;
    Some((handle.parse().ok()?, appearance))
}

/// Raw country references to canonical slots, as `CountryLinks` holds them at `hour` of `run`.
pub fn link_slots(recording: &Recording, run: u32, hour: i32) -> io::Result<BTreeMap<u32, u32>> {
    let slot = dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?;
    let links = read::Read { table: TableId::CountryLinks, columns: Some(&[slot]), keys: read::Keys::All };
    let snapshot = read::at(recording, run, &[links], hour)?.remove(0);
    let view = snapshot.view();
    Ok(snapshot.rows.iter().filter_map(|&(key, row)| Some((key as u32, view.number(slot, row)? as u32))).collect())
}

/// The lines the holders of `tag` produced over hours `from..=to` of `query`'s timeline, in the
/// order first seen; `lines(run)` gives a run's index. A line belongs to the tag where its
/// producing country links to the tag's holder's slot (the links as the stretch's last hour
/// holds them).
pub fn appearances(query: &Query<'_>, tag: &Country, from: i32, to: i32, lines: &dyn Fn(u32) -> io::Result<std::sync::Arc<RunLines>>) -> io::Result<Vec<Appearance>> {
    let recording = query.replay.recording();
    let mut pieces: BTreeMap<u64, Vec<Piece>> = BTreeMap::new();
    for stretch in query.timeline.within(from, to) {
        let held = query.holders.within(tag, stretch.first, stretch.last);
        if !held.iter().any(|run| matches!(run.holder, Holder::Found { .. })) {
            continue;
        }
        let index = lines(stretch.run)?;
        let slots = link_slots(recording, stretch.run, stretch.last)?;
        for run in &held {
            let Holder::Found { slot, .. } = run.holder else { continue };
            for life in &index.lines {
                if slots.get(&life.country) != Some(&slot) {
                    continue;
                }
                let (first, last) = (life.first.max(run.first), life.last.min(run.last));
                if first > last {
                    continue;
                }
                pieces.entry(life.handle).or_default().push(Piece { run: stretch.run, first, last, starts_segment: true, held_from: run.first, held_to: run.last });
            }
        }
    }
    let mut out = Vec::new();
    for (handle, mut held) in pieces {
        held.sort_unstable_by_key(|piece| piece.first);
        let mut appearance = 0;
        let mut current: Option<Appearance> = None;
        for mut piece in held {
            // Hours a run holds no row of the handle may be hours its table was not read: the
            // line stays one appearance through them, its steps broken there.
            let previous = current.as_ref().and_then(|current| current.pieces.last());
            let joins = previous.is_some_and(|last| last.run == piece.run);
            piece.starts_segment = !previous.is_some_and(|last| last.run == piece.run && last.last.checked_add(1) == Some(piece.first));
            match current.as_mut() {
                Some(current) if joins => current.pieces.push(piece),
                _ => {
                    out.extend(current.take());
                    appearance += 1;
                    current = Some(Appearance { key: format!("{handle}#{appearance}"), handle, pieces: vec![piece] });
                }
            }
        }
        out.extend(current);
    }
    out.sort_by_key(|appearance| (appearance.first(), appearance.handle));
    Ok(out)
}

/// The hours `tag` had a holder over `from..=to`: outside them its lines are not known.
pub fn held_hours(query: &Query<'_>, tag: &Country, from: i32, to: i32) -> Vec<HourSpan> {
    let mut spans: Vec<HourSpan> = Vec::new();
    for run in query.holders.within(tag, from, to) {
        if !matches!(run.holder, Holder::Found { .. }) {
            continue;
        }
        match spans.last_mut() {
            Some(last) if last.last_hour.checked_add(1) == Some(run.first) => last.last_hour = run.last,
            _ => spans.push(HourSpan { first_hour: run.first, last_hour: run.last }),
        }
    }
    spans
}

/// The hours of window `window` (`DETAIL_HOURS` each), both included; none past an `i32`.
pub fn window_hours(window: i32) -> Option<(i32, i32)> {
    let first = window.checked_mul(DETAIL_HOURS)?;
    Some((first, first.checked_add(DETAIL_HOURS - 1)?))
}

/// Cuts `runs` (each key's runs of one column) into the hours where every column holds still:
/// for each hour boundary in `first..=last`, the columns' values then. A column without a run
/// at an hour holds none there.
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
