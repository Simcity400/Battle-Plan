//! Hourly live refit work and independently dated daily target/cost/allocation.
//!
//! Names: `TAG@slot|<full line>|progress` and `...|live` follow only the
//! complete hourly progress census. `...|cost`, `...|dockyards`, `...|target`
//! and `...|targetName` are daily metadata, never a liveness certificate.
//! `TAG@slot|<full line>|ship|<full ship>|present` dates actual live membership;
//! the sibling ship measures use the repair product's grammar.
//! Country totals use `TAG@slot|total|<measure>`; `known|<table>`,
//! `read|<table>` and `refusal|<token>` retain supplier-local evidence.
use super::yards::{self, NavalYardsParams, State, YardChoice, YardGroup};
use dashboard_base::{
    prepared::prepass::Family,
    products::{Product, finite_fixed64},
};
use dashboard_keyed::{
    Observer, Values,
    countries::Countries,
    query::{KeyedWindow, Query},
};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{NavalRefitShipRow, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use ts_rs::TS;

pub struct NavalRefits;
#[derive(Clone, Debug, Serialize, TS)]
pub struct NavalRefitsReply {
    pub window: KeyedWindow,
    pub lines: Vec<YardGroup>,
    pub tokens: BTreeMap<String, YardChoice>,
    pub warnings: Vec<String>,
}
impl Product for NavalRefits {
    const KEY: &'static str = "naval-refits";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalYardsParams;
    type Reply = NavalRefitsReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
    fn types() -> Vec<String> {
        let mut types = yards::types();
        types.push(NavalRefitsReply::decl());
        types
    }
}
#[derive(Default, Serialize, Deserialize)]
pub struct RefitsObserver {
    pub state: State,
}
const SOURCES: &[TableId] = &[
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::ProductionLines,
    TableId::VariantDefinitions,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::NavalRepairTaskForces,
    TableId::NavalRepairChildren,
];
pub const TABLES: &[TableId] = yards::management::TABLES;
pub static FAMILY: Family = yards::management::family::<RefitsObserver>(
    "naval-refits",
    "naval-refits.extract",
    "naval-refits.sequential",
    "naval-refits.fallback",
);
impl Observer for RefitsObserver {
    const NAME: &'static str = "naval-refits";
    const TABLES: &'static [TableId] = TABLES;
    const READ: TableId = TableId::NavalRefitProgress;
    fn family() -> &'static Family {
        &FAMILY
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        yards::fields(TABLES)
    }
    fn changed(changes: &dashboard_base::prepared::frame_changes::FrameChanges) -> bool {
        yards::management::changed(changes)
    }
    fn bind(
        &mut self,
        source: &dashboard_base::Source,
        _: Option<&dashboard_core::config::ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        self.state.bind(source);
        Ok(())
    }
    fn observe(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        self.state.observe(format, frame, SOURCES, true, values)
    }
}

pub(super) fn project(
    state: &mut State,
    frame: &DecodedFrame,
    countries: &Countries,
    owners: &[String],
    values: &mut Values,
) {
    let progress = frame.complete::<tables::NavalRefitProgress>().ok();
    let production = frame.complete::<tables::ProductionLines>().ok();
    let ships = frame.complete::<tables::Ships>().ok();
    let tasks = frame.complete::<tables::TaskForces>().ok();
    let links = frame.complete::<tables::NavalRefitShips>().ok();
    let mut lines = BTreeMap::<(String, u64), BTreeSet<u64>>::new();
    let mut metadata_seen = BTreeSet::new();
    let mut census_complete = progress.is_some_and(|rows| rows.is_empty())
        || (progress.is_some() && links.is_some() && ships.is_some() && tasks.is_some());
    if let Some(links) = links {
        for link in links.iter().filter(|link| link.line != 0) {
            let ship = yards::find(ships, link.ship, |row| row.id);
            let owner = ship
                .and_then(|ship| yards::find(tasks, ship.task_force, |task| task.id))
                .and_then(|task| countries.raw(task.roster_owner));
            let typed = dashboard_forces::products::naval_management::projection::refit(
                link, progress, production,
            );
            // A stale carried link is not an active or queued ship row.
            if typed.live == Some(false) {
                continue;
            }
            let Some(owner) = owner else {
                census_complete = false;
                continue;
            };
            let prefix = format!("{owner}|{}", link.line);
            if let Some(amount) = typed.progress {
                lines
                    .entry((owner.clone(), link.line))
                    .or_default()
                    .insert(link.ship);
                values.push(format!("{prefix}|live"), 1);
                values.push(
                    format!("{prefix}|progress"),
                    i128::from(amount.progress_raw),
                );
                let ship_prefix = format!("{prefix}|ship|{}", link.ship);
                values.push(format!("{ship_prefix}|present"), 1);
                state.ship(frame, values, &owner, &ship_prefix, link.ship);
            }
            // Metadata is retained even when the hourly liveness gate is unknown.
            // Shared line metadata and its refusals occur once, not once per ship.
            if !metadata_seen.insert((owner.clone(), link.line)) {
                continue;
            }
            if let Some(line) = typed.metadata {
                values.push(format!("{prefix}|cost"), i128::from(line.cost_raw));
                values.push(
                    format!("{prefix}|dockyards"),
                    i128::from(line.active_factories),
                );
                state.design_value(frame, values, &owner, &prefix, "target", line.variant);
                if let Some(amount) = typed.progress {
                    if line.cost_raw > 0
                        && finite_fixed64(line.cost_raw)
                        && finite_fixed64(amount.progress_raw)
                    {
                        values.push(
                            format!("{prefix}|remainingCostRaw"),
                            i128::from(line.cost_raw) - i128::from(amount.progress_raw),
                        );
                    } else {
                        state.refuse(values,&owner,"Refit ratio and remaining cost unavailable: nonpositive or nonfinite last recorded cost or progress",1);
                    }
                }
            } else {
                state.refuse(
                    values,
                    &owner,
                    "Refit daily target, cost and allocation not recorded",
                    1,
                );
            }
        }
    }
    // Every line contributes once, irrespective of its ship multiplicity.
    let mut totals = BTreeMap::<String, (usize, usize, Option<i128>, Option<i128>)>::new();
    for ((owner, line), ships) in &lines {
        let total = totals
            .entry(owner.clone())
            .or_insert((0, 0, Some(0), Some(0)));
        total.0 += ships.len();
        total.1 += 1;
        let link = NavalRefitShipRow {
            ship: 0,
            line: *line,
        };
        let typed = dashboard_forces::products::naval_management::projection::refit(
            &link, progress, production,
        );
        total.2 = total
            .2
            .zip(typed.metadata)
            .map(|(sum, line)| sum + i128::from(line.active_factories));
        total.3 =
            total
                .3
                .zip(typed.metadata.zip(typed.progress))
                .and_then(|(sum, (line, amount))| {
                    (line.cost_raw > 0
                        && finite_fixed64(line.cost_raw)
                        && finite_fixed64(amount.progress_raw))
                    .then_some(sum + i128::from(line.cost_raw) - i128::from(amount.progress_raw))
                });
    }
    for owner in owners {
        values.push(
            format!("{owner}|total|censusComplete"),
            i128::from(census_complete),
        );
        if census_complete {
            let total = totals
                .get(owner)
                .copied()
                .unwrap_or((0, 0, Some(0), Some(0)));
            values.push(format!("{owner}|total|ships"), total.0 as i128);
            values.push(format!("{owner}|total|lines"), total.1 as i128);
            if let Some(dockyards) = total.2 {
                values.push(format!("{owner}|total|dockyards"), dockyards);
            }
            if let Some(remaining) = total.3 {
                values.push(format!("{owner}|total|remainingCostRaw"), remaining);
            }
        } else {
            state.refuse(values,owner,"Active refit count unavailable: incomplete hourly progress, links, ships or ownership census",1);
        }
    }
}

pub fn query(query: &Query<'_>) -> io::Result<serde_json::Value> {
    let params = yards::params::<NavalRefits>(query)?;
    let (first, last) = query.window()?;
    let mut reply = NavalRefitsReply {
        window: KeyedWindow {
            first_hour: first,
            last_hour: last,
            ..Default::default()
        },
        lines: Vec::new(),
        tokens: BTreeMap::new(),
        warnings: Vec::new(),
    };
    if let Some(mut opened) = query.open::<RefitsObserver>()? {
        reply.window = opened.window(first, last, query.through(), |name| {
            yards::selected(name, &params.tag)
        })?;
        (reply.tokens, reply.warnings) =
            yards::present(query, &reply.window, &opened.scan.observer.state)?;
        reply.lines = yards::groups(&reply.window, &reply.tokens, true);
    }
    yards::checked(&reply)
}
