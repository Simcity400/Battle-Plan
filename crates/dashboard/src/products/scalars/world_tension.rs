//! World tension at an hour, taken apart: the tension the game showed, every country's
//! contribution (the game's own sum per canonical slot), the part from slots no tag ever held, and
//! every threat source the game lists, named. A rows-at-an-hour product over a few tables of a few
//! hundred rows, followed as a panel: its numbers move daily, so each is a live cell the head
//! follows and the widget paints (the tension and the contributions' sum on row `world`, each
//! contribution on its slot's row, each source's current amount on `source<ordinal>`), and the
//! panel carries the rows, their order, names and texts. The tension and each contribution over
//! time are the declared `world-tension`, `threat-contributions` and `threat-sum` series.
//!
//! Contributions are fixed point with 100,000 units a percentage point of tension; the game sums
//! the sources' current contributions, clamps the sum to 100% and stores it divided by 100 as the
//! tension, so tension has 1,000 units a percentage point. A slot no tag holds at the hour keeps
//! the tag that last held it (its sources decay after its country is gone).

use super::{Named, rows};
use crate::{
    Source,
    products::{
        LoadContext,
        at_hour::{At, AtHour},
    },
};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_query::{
    Query,
    series::{self, NavigationKey},
};
use dashboard_text::metadata::link_slot;
use engine_stability::{cthreat_source, cworld_threat};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct WorldTension;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldTensionParams {
    pub branch_id: String,
}

/// A country slot's contribution: its value, in contribution units, is the live cell (slot, `value`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TensionContribution {
    pub slot: u32,
    /// The tag holding the slot, or the one that last held it; null when none ever did.
    pub tag: Option<String>,
    /// The tag no longer holds the slot: it is the last that did.
    pub remembered: bool,
}

/// A threat source the game lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TensionSource {
    pub ordinal: u32,
    /// The country that caused it, or the tag that last held its slot; null when none is known.
    pub tag: Option<String>,
    pub remembered: bool,
    /// The game's own text for it, without formatting codes.
    pub reason: String,
    /// What it adds once its ramp ends and each day, in contribution units; what it adds now is the
    /// live cell (`source<ordinal>`, `current`).
    pub final_raw: String,
    pub daily_raw: String,
    /// The hour the game created it.
    pub date: i32,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WorldTensionReply {
    /// Tension units a percentage point.
    pub tension_scale: u32,
    /// Contribution units a percentage point.
    pub contribution_scale: u32,
    /// The sum the tension is capped at, in contribution units.
    pub ceiling_raw: String,
    /// Live cells of row `world`: `tension` (the tension the game showed, tension units), `sum`
    /// (every contribution summed before the cap) and, where `unnamed`, `unnamed` (the part of the
    /// sum from slots no tag ever held), both in contribution units.
    pub unnamed: bool,
    /// Every nonzero contribution, greatest first, ties by slot.
    pub contributions: Vec<TensionContribution>,
    /// In the game's list order.
    pub sources: Vec<TensionSource>,
    /// Why the tension is not known at this hour.
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(WorldTensionReply);

/// The row of the world's live cells, and a source's.
pub(super) const WORLD: &str = "world";

pub(super) fn source_row(ordinal: u32) -> String {
    format!("source{ordinal}")
}

/// The columns the cells read: WorldTension's tension, ThreatTotals' sum, the aggregates' and the
/// sources' current amounts.
pub(super) struct Columns {
    pub tension: usize,
    pub sum: usize,
    pub contribution: usize,
    pub current: usize,
}

impl Columns {
    fn new() -> io::Result<Self> {
        Ok(Self {
            tension: dashboard_declare::field(TableId::WorldTension, "total_raw")?,
            sum: dashboard_declare::field(TableId::ThreatTotals, "sum")?,
            contribution: dashboard_declare::field(TableId::ThreatCountryAggregates, "total_raw")?,
            current: dashboard_declare::field(TableId::ThreatSources, "current_raw")?,
        })
    }
}

/// Names the reply's moving numbers as live cells, raw units: the world's tension and sum, the part
/// of `unnamed` slots where there is one, each contribution of `slots` and each source of `ordinals`.
pub(super) fn name_cells(live: &Cells, columns: &Columns, slots: &[u32], unnamed: &[u32], ordinals: &[u32]) {
    live.value(WORLD, "tension", Cell { table: TableId::WorldTension, column: columns.tension, key: 0 }, 1.0);
    live.value(WORLD, "sum", Cell { table: TableId::ThreatTotals, column: columns.sum, key: 0 }, 1.0);
    if !unnamed.is_empty() {
        let sources: Vec<Cell> = unnamed.iter().map(|&slot| Cell { table: TableId::ThreatCountryAggregates, column: columns.contribution, key: u128::from(slot) }).collect();
        live.sum(WORLD, "unnamed", &sources, 1.0);
    }
    for &slot in slots {
        live.value(&slot.to_string(), "value", Cell { table: TableId::ThreatCountryAggregates, column: columns.contribution, key: u128::from(slot) }, 1.0);
    }
    for &ordinal in ordinals {
        live.value(&source_row(ordinal), "current", Cell { table: TableId::ThreatSources, column: columns.current, key: u128::from(ordinal) }, 1.0);
    }
}

/// A source's reason as plain text, or its bytes as they are with why not.
fn reason_text(bytes: &[u8], warnings: &mut Vec<String>) -> String {
    match dashboard_text::catalogue::display_text(bytes) {
        Ok(text) => text,
        Err(why) => {
            let raw = String::from_utf8_lossy(bytes).into_owned();
            warnings.push(format!("Threat reason {raw:?} {why}"));
            raw
        }
    }
}

impl AtHour for WorldTension {
    const KEY: &'static str = "world-tension";
    const TITLE: &'static str = "World tension";
    // Reasons are the recording's own text.
    const CONTENT_DEPENDENT: bool = false;
    type Params = WorldTensionParams;
    type Reply = WorldTensionReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![WorldTensionParams::decl(), TensionContribution::decl(), TensionSource::decl(), WorldTensionReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::WorldTension, TableId::ThreatCountryAggregates, TableId::ThreatSources, TableId::ThreatTotals, TableId::CountryLinks]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut reply = WorldTensionReply {
            tension_scale: (cthreat_source::FIXED_POINT_SCALE / cworld_threat::SCALAR_DIVISOR) as u32,
            contribution_scale: cthreat_source::FIXED_POINT_SCALE as u32,
            ceiling_raw: cworld_threat::CLAMP_CEILING.to_string(),
            unnamed: false,
            contributions: Vec::new(),
            sources: Vec::new(),
            missing: None,
            warnings: Vec::new(),
        };
        match rows::<tables::WorldTension>(frame) {
            Ok(held) if held.is_empty() => reply.missing = Some("World tension held no row at this hour".into()),
            Ok(_) => {}
            Err(reason) => reply.missing = Some(format!("World tension is not known at this hour: {reason}")),
        }
        let named = Named::new(at.holders, frame.hour);
        let mut unnamed = Vec::new();
        match rows::<tables::ThreatCountryAggregates>(frame) {
            Err(reason) => reply.warnings.push(format!("Country contributions are not known at this hour: {reason}")),
            Ok(aggregates) => {
                unnamed.extend(aggregates.iter().filter(|row| !named.known(row.country_slot) && row.total_raw != 0).map(|row| row.country_slot));
                reply.unnamed = !unnamed.is_empty();
                let mut held: Vec<_> = aggregates.iter().filter(|row| row.total_raw != 0).collect();
                held.sort_unstable_by(|a, b| b.total_raw.cmp(&a.total_raw).then(a.country_slot.cmp(&b.country_slot)));
                for row in held {
                    let tag = named.slot(row.country_slot, frame.hour);
                    reply.contributions.push(TensionContribution {
                        slot: row.country_slot,
                        tag: tag.map(|(tag, _)| tag.to_owned()),
                        remembered: tag.is_some_and(|(_, remembered)| remembered),
                    });
                }
            }
        }
        let links = rows::<tables::CountryLinks>(frame).ok();
        match rows::<tables::ThreatSources>(frame) {
            Err(reason) => reply.warnings.push(format!("Threat sources are not known at this hour: {reason}")),
            Ok(sources) => {
                for source in sources {
                    let slot = links.and_then(|links| link_slot(links, source.country));
                    let tag = slot.and_then(|slot| named.slot(slot, frame.hour));
                    reply.sources.push(TensionSource {
                        ordinal: source.ordinal,
                        tag: tag.map(|(tag, _)| tag.to_owned()),
                        remembered: tag.is_some_and(|(_, remembered)| remembered),
                        reason: reason_text(&source.reason, &mut reply.warnings),
                        final_raw: source.final_raw.to_string(),
                        daily_raw: source.daily_raw.to_string(),
                        date: source.date,
                    });
                }
            }
        }
        let slots: Vec<u32> = reply.contributions.iter().map(|item| item.slot).collect();
        let ordinals: Vec<u32> = reply.sources.iter().map(|item| item.ordinal).collect();
        name_cells(at.live, &Columns::new()?, &slots, &unnamed, &ordinals);
        Ok(reply)
    }
}

/// The family kind of the world's tension series, whose keyboard step is [`next_change`].
pub const SERIES: &str = "world-tension";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    key: NavigationKey,
    from: Option<f64>,
    first_hour: Option<f64>,
    last_hour: Option<f64>,
}

/// The tension chart's keyboard step, as main's: the nearest hour strictly past the origin at which
/// the tension tables were read (the tension, its sum and every contribution are read together), or
/// at which the tension or the sum moved where the recording names no read; none where there is
/// neither, never the origin itself.
pub fn next_change(query: &Query<'_>, params: &serde_json::Value) -> io::Result<serde_json::Value> {
    let step: Step = serde_json::from_value(params.clone()).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid world tension step: {error}")))?;
    let bounds = (series::hour_of(step.first_hour, "firstHour")?, series::hour_of(step.last_hour, "lastHour")?);
    let origin = step.from.map(|from| series::hour_of(Some(from), "from")).transpose()?;
    let forward = matches!(step.key, NavigationKey::ArrowRight | NavigationKey::Home);
    let mut found = vec![series::next_read(query, TableId::WorldTension, origin, step.key, bounds)?];
    for (kind, measure) in [(SERIES, "tension"), ("threat-sum", "sum")] {
        let declared = dashboard_declare::series(kind).ok_or_else(|| io::Error::other(format!("{kind} is not declared")))?;
        let measure = declared.measure(measure).ok_or_else(|| io::Error::other(format!("{kind} has no measure {measure}")))?;
        let line = series::Line::new(query, declared, measure)?;
        found.push(line.change_lines(&[series::Ask::whole("World")], &[measure], origin, step.key, bounds)?);
    }
    let hour = found.into_iter().flatten().reduce(|a, b| if forward { a.min(b) } else { a.max(b) });
    Ok(serde_json::to_value(series::SeriesKeyReply { hour: hour.map(f64::from) })?)
}
