//! Dockyard allocation and ordered repair queues.
//!
//! Names: `TAG@slot|<province>|<measure>` for port presence, name token,
//! priority, policy token and dockyards; `TAG@slot|<province>|queue|<ordinal>`
//! holds the full decimal ship handle (repeats preserved).
//! `TAG@slot|<province>|ship|<full handle>|<measure>` holds hull, maximum,
//! design/name tokens, task and repair orders. Country totals use
//! `TAG@slot|total|<measure>`. Shared `known|<table>`, `read|<table>` and
//! `refusal|<token>` channels preserve independent supplier availability.
use super::yards::{self, NavalYardsParams, Operand, State, YardChoice, YardGroup};
use dashboard_base::{prepared::prepass::Family, products::Product};
use dashboard_keyed::{
    Observer, Values,
    countries::Countries,
    query::{KeyedWindow, Query},
};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use ts_rs::TS;

pub struct NavalRepairs;
#[derive(Clone, Debug, Serialize, TS)]
pub struct NavalRepairsReply {
    pub window: KeyedWindow,
    pub ports: Vec<YardGroup>,
    pub tokens: BTreeMap<String, YardChoice>,
    pub warnings: Vec<String>,
}
impl Product for NavalRepairs {
    const KEY: &'static str = "naval-repairs";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalYardsParams;
    type Reply = NavalRepairsReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
    fn types() -> Vec<String> {
        let mut types = yards::types();
        types.push(NavalRepairsReply::decl());
        types
    }
}
#[derive(Default, Serialize, Deserialize)]
pub struct RepairsObserver {
    pub state: State,
}

// names: TAG@slot|province|ship|full-handle|measure; queue|ordinal retains repeats.
const SOURCES: &[TableId] = &[
    TableId::NavalRepairCountries,
    TableId::CountryFactories,
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::NavalRepairTaskForces,
    TableId::NavalRepairChildren,
    TableId::VariantDefinitions,
];
pub const TABLES: &[TableId] = yards::management::TABLES;
pub static FAMILY: Family = yards::management::family::<RepairsObserver>(
    "naval-repairs",
    "naval-repairs.extract",
    "naval-repairs.sequential",
    "naval-repairs.fallback",
);
impl Observer for RepairsObserver {
    const NAME: &'static str = "naval-repairs";
    const TABLES: &'static [TableId] = TABLES;
    const READ: TableId = TableId::NavalRepairQueue;
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
        self.state.observe(format, frame, SOURCES, false, values)
    }
}

pub(super) fn project(
    state: &mut State,
    frame: &DecodedFrame,
    countries: &Countries,
    owners: &[String],
    values: &mut Values,
) {
    if let Ok(rows) = frame.complete::<tables::NavalRepairCountries>() {
        for row in rows {
            if let Some(owner) = countries.raw(row.country) {
                for (field, amount) in [
                    ("usedDockyards", row.num_used_dockyards),
                    ("repairLimit", row.max_allowed_dockyards_to_repair),
                    ("buildingRepairLimit", row.max_factories_for_repair),
                ] {
                    values.push(format!("{owner}|total|{field}"), i128::from(amount));
                }
            }
        }
    }
    if let Ok(rows) = frame.complete::<tables::CountryFactories>() {
        for row in rows {
            if let Some(owner) = countries.raw(row.country) {
                values.push(
                    format!("{owner}|total|dockyardsRaw"),
                    i128::from(row.naval_total_raw),
                );
                values.push(
                    format!("{owner}|total|dockyards"),
                    i128::from(row.naval_total_raw / 100_000),
                );
            }
        }
    }
    let Ok(bases) = frame.complete::<tables::NavalBases>() else {
        for owner in owners {
            state.refuse(values, owner, "Repair port ownership not recorded", 1);
        }
        return;
    };
    let ownership_complete = bases
        .iter()
        .all(|base| countries.raw(base.country).is_some());
    let repair_bases = frame.complete::<tables::NavalRepairBases>().ok();
    let queues = frame.complete::<tables::NavalRepairQueue>().ok();
    let mut totals = BTreeMap::<String, (usize, BTreeSet<u64>)>::new();
    for base in bases {
        let Some(owner) = countries.raw(base.country) else {
            continue;
        };
        let prefix = format!("{owner}|{}", base.province);
        let port = yards::find(repair_bases, base.province, |row| row.province);
        let entries = queues.map(|rows| {
            let first = rows.partition_point(|row| row.province < base.province);
            let count = rows[first..].partition_point(|row| row.province == base.province);
            &rows[first..first + count]
        });
        if port.is_none() && entries.is_none_or(|rows| rows.is_empty()) {
            continue;
        }
        values.push(format!("{prefix}|present"), 1);
        if let Some(province) = yards::find(
            frame.complete::<tables::Provinces>().ok(),
            base.province,
            |row| row.id,
        ) {
            values.push(format!("{prefix}|state"), i128::from(province.state));
        }
        state.label(
            values,
            format!("{prefix}|name"),
            Operand::Port(yards::port_operand(frame, countries, base.province)),
        );
        if let Some(port) = port {
            values.push(format!("{prefix}|priority"), i128::from(port.priority));
            values.push(
                format!("{prefix}|dockyards"),
                i128::from(port.allocated_dockyards),
            );
            state.label(
                values,
                format!("{prefix}|policy"),
                Operand::Policy {
                    port: true,
                    value: port.priority,
                },
            );
        }
        if let Some(entries) = entries {
            values.push(format!("{prefix}|queueCount"), entries.len() as i128);
            let mut unique = BTreeSet::new();
            for entry in entries {
                values.push(
                    format!("{prefix}|queue|{}", entry.ordinal),
                    i128::from(entry.ship),
                );
                let total = totals.entry(owner.clone()).or_default();
                total.0 += 1;
                total.1.insert(entry.ship);
                if unique.insert(entry.ship) {
                    let ship_prefix = format!("{prefix}|ship|{}", entry.ship);
                    values.push(format!("{ship_prefix}|present"), 1);
                    state.ship(frame, values, &owner, &ship_prefix, entry.ship);
                }
            }
        }
    }
    for owner in owners {
        values.push(
            format!("{owner}|total|queueCensusComplete"),
            i128::from(ownership_complete && queues.is_some()),
        );
    }
    if queues.is_some() && ownership_complete {
        for owner in owners {
            let total = totals.get(owner);
            values.push(
                format!("{owner}|total|queueEntries"),
                total.map_or(0, |total| total.0) as i128,
            );
            values.push(
                format!("{owner}|total|distinctQueuedReferences"),
                total.map_or(0, |total| total.1.len()) as i128,
            );
            if let Ok(ships) = frame.complete::<tables::Ships>()
                && total.is_none_or(|total| {
                    total
                        .1
                        .iter()
                        .all(|ship| yards::find(Some(ships), *ship, |row| row.id).is_some())
                })
            {
                values.push(
                    format!("{owner}|total|distinctQueuedShips"),
                    total.map_or(0, |total| total.1.len()) as i128,
                );
            }
            if let Some((count, _)) = total {
                state.refuse(
                    values,
                    owner,
                    "Repair rate and ETA unavailable: the exact repair rate is not recorded",
                    *count,
                );
            }
        }
    } else {
        for owner in owners {
            state.refuse(
                values,
                owner,
                "Repair queue or port ownership not recorded; empty queue cannot be established",
                1,
            );
        }
    }
}

pub fn query(query: &Query<'_>) -> io::Result<serde_json::Value> {
    let params = yards::params::<NavalRepairs>(query)?;
    let (first, last) = query.window()?;
    let mut reply = NavalRepairsReply {
        window: KeyedWindow {
            first_hour: first,
            last_hour: last,
            ..Default::default()
        },
        ports: Vec::new(),
        tokens: BTreeMap::new(),
        warnings: Vec::new(),
    };
    if let Some(mut opened) = query.open::<RepairsObserver>()? {
        reply.window = opened.window(first, last, query.through(), |name| {
            yards::selected(name, &params.tag)
        })?;
        (reply.tokens, reply.warnings) =
            yards::present(query, &reply.window, &opened.scan.observer.state)?;
        reply.ports = yards::groups(&reply.window, &reply.tokens, false);
    }
    yards::checked(&reply)
}
