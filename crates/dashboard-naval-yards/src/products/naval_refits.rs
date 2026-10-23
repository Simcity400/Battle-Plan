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
    prepared::prepass::{Family, HeldBytes},
    products::{Product, finite_fixed64},
};
use dashboard_keyed::{
    Delta, Observer, Values,
    countries::Countries,
    query::{KeyedWindow, Query},
};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{
        NavalRefitProgressRow, NavalRefitShipRow, ProductionLineRow, ShipRow, TableId,
        TaskForceRow, tables,
    },
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
pub(super) const SOURCES: &[TableId] = &[
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
    fn observe_changes(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        names: &dashboard_keyed::delta::Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        self.state.observe_changes(format, frame, SOURCES, names, delta)
    }
    fn held(&self, bytes: &mut HeldBytes) {
        self.state.held(bytes);
    }
    fn unpublished(&self) -> usize {
        self.state.unpublished()
    }
    fn published(&mut self) {
        self.state.published();
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
            let owner = link_owner(link, ships, tasks, countries);
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
                line_progress(&prefix, amount, values);
                let ship_prefix = format!("{prefix}|ship|{}", link.ship);
                values.push(format!("{ship_prefix}|present"), 1);
                state.ship(frame, values, &owner, &ship_prefix, link.ship);
            }
            // Metadata is retained even when the hourly liveness gate is unknown.
            // Shared line metadata and its refusals occur once, not once per ship.
            if !metadata_seen.insert((owner.clone(), link.line)) {
                continue;
            }
            line_metadata(
                state,
                frame,
                values,
                &owner,
                &prefix,
                typed.metadata,
                typed.progress,
            );
        }
    }
    // Every line contributes once, irrespective of its ship multiplicity.
    let mut totals = BTreeMap::<String, RefitTotal>::new();
    for ((owner, line), ships) in &lines {
        totals
            .entry(owner.clone())
            .or_default()
            .add(ships.len(), *line, progress, production);
    }
    for owner in owners {
        refit_totals(state, values, owner, census_complete, totals.get(owner));
    }
}

/// The country whose task force holds a refit link's ship.
pub(super) fn link_owner(
    link: &NavalRefitShipRow,
    ships: Option<&[ShipRow]>,
    tasks: Option<&[TaskForceRow]>,
    countries: &Countries,
) -> Option<String> {
    yards::find(ships, link.ship, |row| row.id)
        .and_then(|ship| yards::find(tasks, ship.task_force, |task| task.id))
        .and_then(|task| countries.raw(task.roster_owner))
}

/// A live line's hourly values.
pub(super) fn line_progress(prefix: &str, amount: &NavalRefitProgressRow, values: &mut Values) {
    values.push(format!("{prefix}|live"), 1);
    values.push(
        format!("{prefix}|progress"),
        i128::from(amount.progress_raw),
    );
}

/// A line's daily metadata and its refusals.
pub(super) fn line_metadata(
    state: &mut State,
    frame: &DecodedFrame,
    values: &mut Values,
    owner: &str,
    prefix: &str,
    metadata: Option<&ProductionLineRow>,
    progress: Option<&NavalRefitProgressRow>,
) {
    if let Some(line) = metadata {
        values.push(format!("{prefix}|cost"), i128::from(line.cost_raw));
        values.push(
            format!("{prefix}|dockyards"),
            i128::from(line.active_factories),
        );
        state.design_value(frame, values, owner, prefix, "target", line.variant);
        if let Some(amount) = progress {
            if line.cost_raw > 0 && finite_fixed64(line.cost_raw) && finite_fixed64(amount.progress_raw)
            {
                values.push(
                    format!("{prefix}|remainingCostRaw"),
                    i128::from(line.cost_raw) - i128::from(amount.progress_raw),
                );
            } else {
                state.refuse(values,owner,"Refit ratio and remaining cost unavailable: nonpositive or nonfinite last recorded cost or progress",1);
            }
        }
    } else {
        state.refuse(
            values,
            owner,
            "Refit daily target, cost and allocation not recorded",
            1,
        );
    }
}

/// A country's live lines summed; dockyards and remaining cost are None once a line lacks them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RefitTotal {
    ships: usize,
    lines: usize,
    dockyards: Option<i128>,
    remaining: Option<i128>,
}

impl Default for RefitTotal {
    fn default() -> Self {
        Self {
            ships: 0,
            lines: 0,
            dockyards: Some(0),
            remaining: Some(0),
        }
    }
}

impl RefitTotal {
    /// Add a live line holding `ships` ships.
    pub(super) fn add(
        &mut self,
        ships: usize,
        line: u64,
        progress: Option<&[NavalRefitProgressRow]>,
        production: Option<&[ProductionLineRow]>,
    ) {
        let link = NavalRefitShipRow { ship: 0, line };
        let typed = dashboard_forces::products::naval_management::projection::refit(
            &link, progress, production,
        );
        self.ships += ships;
        self.lines += 1;
        self.dockyards = self
            .dockyards
            .zip(typed.metadata)
            .map(|(sum, line)| sum + i128::from(line.active_factories));
        self.remaining = self
            .remaining
            .zip(typed.metadata.zip(typed.progress))
            .and_then(|(sum, (line, amount))| {
                (line.cost_raw > 0
                    && finite_fixed64(line.cost_raw)
                    && finite_fixed64(amount.progress_raw))
                .then_some(sum + i128::from(line.cost_raw) - i128::from(amount.progress_raw))
            });
    }
}

/// A country's refit totals: `complete` whether the census is.
pub(super) fn refit_totals(
    state: &mut State,
    values: &mut Values,
    owner: &str,
    complete: bool,
    total: Option<&RefitTotal>,
) {
    values.push(
        format!("{owner}|total|censusComplete"),
        i128::from(complete),
    );
    if complete {
        let total = total.copied().unwrap_or_default();
        values.push(format!("{owner}|total|ships"), total.ships as i128);
        values.push(format!("{owner}|total|lines"), total.lines as i128);
        if let Some(dockyards) = total.dockyards {
            values.push(format!("{owner}|total|dockyards"), dockyards);
        }
        if let Some(remaining) = total.remaining {
            values.push(format!("{owner}|total|remainingCostRaw"), remaining);
        }
    } else {
        state.refuse(values,owner,"Active refit count unavailable: incomplete hourly progress, links, ships or ownership census",1);
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
