//! One country's standing naval orders and repair/refit queue at the requested hour.
//! Nothing scans a campaign: the demand is one selected frame and its recorded content.
use dashboard_base::{
    Source,
    catalogue::{self, Catalogue, View},
    products::{LoadContext, Product, check_reply_budget},
};
use dashboard_text::{
    metadata::{link_slot, tagged_country},
    variant_names::{self, Tags, Texts},
};
use replay::campaign::SeekResult;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub mod projection;

pub struct NavalManagement;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalManagementParams {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalManagementSection {
    pub key: String,
    pub title: String,
    pub lines: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalManagementReply {
    pub branch_id: String,
    pub hour: i32,
    pub sections: Vec<NavalManagementSection>,
    pub warnings: Vec<String>,
}
const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::Fleets,
    TableId::TaskForces,
    TableId::Ships,
    TableId::TaskForceCounterInputs,
    TableId::ShipModels,
    TableId::VariantDefinitions,
    TableId::ProductionLines,
    TableId::NavalBases,
    TableId::NavalRepairCountries,
    TableId::NavalRepairFleets,
    TableId::NavalRepairTaskForces,
    TableId::NavalTaskForceComposition,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::NavalRepairChildren,
];
impl Product for NavalManagement {
    const KEY: &'static str = "naval-management";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = NavalManagementParams;
    type Reply = NavalManagementReply;
    fn types() -> Vec<String> {
        vec![
            NavalManagementParams::decl(),
            NavalManagementSection::decl(),
            NavalManagementReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        dashboard_core::protocol::validate_tag(&params.tag)?;
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid naval management branch")?;
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::other("Naval management request cancelled"));
        }
        let branch = params.branch_id.parse::<u64>().map_err(io::Error::other)?;
        let frame = match source
            .replay()
            .seek_selected(branch, params.hour, Some(TABLES))?
        {
            SeekResult::Frame(frame) => frame,
            SeekResult::Gap { reason, .. } => {
                return Err(dashboard_base::prepared::query::frames::gap_error(
                    &reason,
                    format!("Naval management unavailable: {reason}"),
                ));
            }
        };
        let catalogue = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let mut reply = project(&frame, params, catalogue.as_ref().ok())?;
        if let Err(why) = &catalogue {
            reply
                .warnings
                .push(format!("Naval content names unavailable: {why}"));
        }
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
fn on(value: u8) -> &'static str {
    if value != 0 { "On" } else { "Off" }
}
fn fixed(raw: i64) -> String {
    let raw = i128::from(raw);
    let magnitude = raw.abs();
    format!(
        "{}{}.{:05}",
        if raw < 0 { "-" } else { "" },
        magnitude / 100000,
        magnitude % 100000
    )
}
fn project(
    frame: &DecodedFrame,
    params: &NavalManagementParams,
    catalogue: Option<&Arc<Catalogue>>,
) -> io::Result<NavalManagementReply> {
    let mut reply = NavalManagementReply {
        branch_id: params.branch_id.clone(),
        hour: params.hour,
        sections: Vec::new(),
        warnings: Vec::new(),
    };
    macro_rules! rows {
        ($table:ident) => {
            match frame.complete::<tables::$table>() {
                Ok(rows) => rows,
                Err(why) => {
                    reply
                        .warnings
                        .push(format!("{}: {why}", stringify!($table)));
                    &[]
                }
            }
        };
    }
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(io::Error::other)?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(io::Error::other)?;
    let (_, slot) = tagged_country(countries, links, &params.tag)
        .found()
        .ok_or_else(|| dashboard_core::refused("Selected naval country identity unavailable"))?;
    let own = |raw| link_slot(links, raw) == Some(slot);
    let lexical = rows!(LexicalTokens);
    let variants = rows!(VariantDefinitions);
    let tags = Tags::new(
        countries,
        links,
        frame.complete::<tables::CountryAppearance>().ok(),
    );
    let design = |handle| {
        if handle == 0 {
            return "None".to_owned();
        }
        variants
            .iter()
            .find(|row| row.id == handle)
            .and_then(|row| {
                let raw = variant_names::raw_variant(row, params.hour, &tags, lexical);
                variant_names::variant_name(&raw, catalogue.map(|cat| cat.as_ref() as &dyn Texts))
                    .ok()
                    .flatten()
            })
            .unwrap_or_else(|| "Design name unavailable".into())
    };
    let localise = |key: &str| catalogue.and_then(|cat| cat.localised(key, View::Plain));
    let policy = |value, dockyard| {
        localise(&format!(
            "NAVY_{}_POLICY_{value}",
            if dockyard {
                "DOCKYARD_REPAIR"
            } else {
                "REPAIR"
            }
        ))
        .unwrap_or_else(|| "Policy name unavailable".into())
    };
    let subunits = catalogue.and_then(|cat| super::navy::content_units(cat, &mut reply.warnings));
    let class = |index: i32| {
        if index == -1 {
            return "Any class".to_owned();
        }
        subunits
            .as_ref()
            .and_then(|units| units.name(index as u32))
            .and_then(Result::ok)
            .and_then(&localise)
            .unwrap_or_else(|| "Class name unavailable".into())
    };
    let budgets = rows!(NavalRepairCountries);
    if let Some(row) = budgets.iter().find(|row| own(row.country)) {
        reply.sections.push(NavalManagementSection {
            key: "budgets".into(),
            title: "Repair allocation".into(),
            lines: vec![
                format!(
                    "Repair dockyards: {} used; limit {}",
                    row.num_used_dockyards, row.max_allowed_dockyards_to_repair
                ),
                format!(
                    "Building repair factory limit: {}",
                    row.max_factories_for_repair
                ),
            ],
        });
    }
    let fleet_settings = rows!(NavalRepairFleets);
    for fleet in rows!(Fleets).iter().filter(|row| own(row.owner)) {
        let mut lines = vec![format!("Home base: province {}", fleet.home_base)];
        if let Some(setting) = fleet_settings.iter().find(|row| row.fleet == fleet.id) {
            lines.push(format!(
                "Automatic home base: {}",
                on(setting.automated_homebase)
            ));
        } else {
            lines.push("Automatic home base unavailable".into());
        }
        let title = fleet
            .name
            .require("Fleets.name")
            .ok()
            .and_then(|name| name.as_ref())
            .and_then(|bytes| {
                if bytes.is_empty() {
                    Some(String::new())
                } else {
                    catalogue.and_then(|cat| {
                        variant_names::Texts::stored_text(
                            cat.as_ref(),
                            std::str::from_utf8(bytes).ok()?,
                            "fleet name",
                        )
                        .ok()
                    })
                }
            })
            .unwrap_or_else(|| "Fleet name unavailable".into());
        reply.sections.push(NavalManagementSection {
            key: format!("fleet:{}", fleet.id),
            title,
            lines,
        });
    }
    let settings = rows!(NavalRepairTaskForces);
    let children = rows!(NavalRepairChildren);
    let composition = rows!(NavalTaskForceComposition);
    let names = rows!(TaskForceCounterInputs);
    let ships_known = frame.complete::<tables::Ships>().is_ok();
    let ships = rows!(Ships);
    let models = rows!(ShipModels);
    let work = rows!(ProductionLines);
    let progress_known = frame.complete::<tables::NavalRefitProgress>().is_ok();
    let progress = rows!(NavalRefitProgress);
    let refits = rows!(NavalRefitShips);
    let tasks = rows!(TaskForces);
    for task in tasks.iter().filter(|row| own(row.roster_owner)) {
        let title = names
            .iter()
            .find(|row| row.task_force == task.id)
            .and_then(|row| {
                catalogue.and_then(|cat| {
                    dashboard_text::unit_names::task_force_name(
                        row,
                        &cat.localisation,
                        &cat.recorded_language,
                    )
                    .ok()
                })
            })
            .unwrap_or_else(|| "Task force name unavailable".into());
        let mut lines = vec![if ships_known {
            format!(
                "Current strength: {} ships",
                ships.iter().filter(|row| row.task_force == task.id).count()
            )
        } else {
            "Current ship count unavailable".into()
        }];
        if let Some(row) = settings.iter().find(|row| row.task_force == task.id) {
            lines.extend([
                format!("Repair priority: {}", policy(row.repair_mode, false)),
                format!(
                    "Split for repair: {}; automatic reinforcement: {}",
                    on(row.repair_split),
                    on(row.auto_reinforcement)
                ),
                format!(
                    "Refit design: {}; after repair: {}",
                    design(row.refit_variant),
                    design(row.refit_after_repair)
                ),
            ]);
            if row.composition_token != engine_native::naval_management::NO_AI_COMPOSITION_TOKEN {
                let template = lexical
                    .iter()
                    .find(|entry| entry.token == row.composition_token)
                    .and_then(|entry| std::str::from_utf8(&entry.literal).ok())
                    .and_then(&localise)
                    .unwrap_or_else(|| "Template name unavailable".into());
                lines.push(format!("AI composition: {template}"));
            }
            if row.repair_target != 0 {
                lines.push(format!("Repair target: province {}", row.repair_target));
            }
            if row.merge_after_repair != 0
                && tasks
                    .binary_search_by_key(&row.merge_after_repair, |task| task.id)
                    .is_ok()
            {
                lines.push(format!(
                    "Rejoin task force {} after repair",
                    row.merge_after_repair >> 32
                ));
            }
            for child in children.iter().filter(|child| child.task_force == task.id) {
                lines.push(format!(
                    "Repair detachment {}: task force {}",
                    u64::from(child.ordinal) + 1,
                    child.child >> 32
                ));
            }
            for entry in composition
                .iter()
                .filter(|entry| entry.task_force == task.id)
            {
                lines.push(format!(
                    "{}: {} - {} (role preference {}; design preference {})",
                    if entry.kind == 0 {
                        "Intended"
                    } else {
                        "Composition count"
                    },
                    class(entry.sub_unit),
                    entry.amount,
                    entry.role,
                    entry.definition
                ));
            }
        } else {
            lines.push("Repair and reinforcement settings unavailable".into());
        }
        for ship in ships.iter().filter(|ship| ship.task_force == task.id) {
            if let Some(link) = refits.iter().find(|row| row.ship == ship.id) {
                let typed = projection::refit(link, progress_known.then_some(progress), Some(work));
                let amount = typed.progress;
                if progress_known && amount.is_none() {
                    continue;
                }
                let line = typed.metadata;
                lines.push(match amount {
                    Some(amount) => format!(
                        "Ship {} refit progress: {} IC",
                        ship.id >> 32,
                        fixed(amount.progress_raw)
                    ),
                    None => format!("Ship {} refit progress unavailable", ship.id >> 32),
                });
                lines.push(format!(
                    "Last recorded target: {}; cost: {}; dockyard allocation: {}",
                    line.map(|line| design(line.variant))
                        .unwrap_or_else(|| "Unavailable".into()),
                    line.map(|line| format!("{} IC", fixed(line.cost_raw)))
                        .unwrap_or_else(|| "Unavailable".into()),
                    line.map(|line| format!("{} dockyards", line.active_factories))
                        .unwrap_or_else(|| "Unavailable".into())
                ));
            }
        }
        reply.sections.push(NavalManagementSection {
            key: format!("task:{}", task.id),
            title,
            lines,
        });
    }
    let bases = rows!(NavalRepairBases);
    let queues = rows!(NavalRepairQueue);
    let owned: BTreeSet<_> = rows!(NavalBases)
        .iter()
        .filter(|row| own(row.country))
        .map(|row| row.province)
        .collect();
    if queues.iter().any(|row| owned.contains(&row.province)) {
        reply
            .warnings
            .push("Repair ETA unavailable: the exact repair rate is not recorded.".into());
    }
    for base in bases.iter().filter(|row| owned.contains(&row.province)) {
        let mut lines = vec![format!(
            "Repair priority: {}; assigned dockyards: {}",
            policy(base.priority, true),
            base.allocated_dockyards
        )];
        for entry in queues.iter().filter(|row| row.province == base.province) {
            let hull = projection::hull(
                ships.iter().find(|row| row.id == entry.ship),
                models.iter().find(|row| row.ship == entry.ship),
            );
            let strength = hull
                .current_raw
                .map(fixed)
                .unwrap_or_else(|| "Unavailable".into());
            let maximum = hull
                .maximum_raw
                .map(fixed)
                .unwrap_or_else(|| "Unavailable".into());
            lines.push(format!(
                "{}. Ship {} - hull HP {} / {}",
                u64::from(entry.ordinal) + 1,
                entry.ship >> 32,
                strength,
                maximum
            ));
        }
        reply.sections.push(NavalManagementSection {
            key: format!("base:{}", base.province),
            title: format!("Repair port - province {}", base.province),
            lines,
        });
    }
    if let Some(cat) = catalogue {
        reply.warnings.extend(cat.refusal_lines());
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use replay_frame::frame::{Metrics, TableView};
    #[test]
    fn rejoin_order_follows_destination_census_after_detachment_changes_fleet() {
        let task = 61 | (3_u64 << 32);
        let destination = 61 | (4_u64 << 32);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 101,
                canonical_slot: 7,
                tag: b"MOD".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 101,
                canonical_slot: 7,
            }]));
        *frame.tables[TableId::NavalRepairTaskForces.index()] =
            TableView::Complete(TableRows::NavalRepairTaskForces(vec![
                NavalRepairTaskForceRow {
                    task_force: task,
                    merge_after_repair: destination,
                    composition_token: engine_native::naval_management::NO_AI_COMPOSITION_TOKEN,
                    ..Default::default()
                },
            ]));
        let params = NavalManagementParams {
            branch_id: "1".into(),
            hour: 1,
            tag: "MOD".into(),
        };
        for fleet in [1, 2] {
            *frame.tables[TableId::TaskForces.index()] =
                TableView::Complete(TableRows::TaskForces(vec![
                    TaskForceRow {
                        id: task,
                        fleet,
                        roster_owner: 101,
                        ..Default::default()
                    },
                    TaskForceRow {
                        id: destination,
                        fleet: 1,
                        ..Default::default()
                    },
                ]));
            let reply = project(&frame, &params, None).unwrap();
            assert!(
                reply
                    .sections
                    .iter()
                    .flat_map(|section| &section.lines)
                    .any(|line| line == "Rejoin task force 4 after repair")
            );
        }
        // Destination retirement changes no carried source settings.
        for replacement in [None, Some(destination + 1)] {
            let mut tasks = vec![TaskForceRow {
                id: task,
                fleet: 2,
                roster_owner: 101,
                ..Default::default()
            }];
            if let Some(id) = replacement {
                tasks.push(TaskForceRow {
                    id,
                    ..Default::default()
                });
            }
            *frame.tables[TableId::TaskForces.index()] =
                TableView::Complete(TableRows::TaskForces(tasks));
            let reply = project(&frame, &params, None).unwrap();
            assert!(
                !reply
                    .sections
                    .iter()
                    .flat_map(|section| &section.lines)
                    .any(|line| line.starts_with("Rejoin task force"))
            );
        }
    }

    #[test]
    fn refit_progress_and_daily_metadata_remain_independently_available() {
        let task = 3_u64 << 32;
        let ship = 4_u64 << 32;
        let line = 5_u64 << 32;
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 101,
                canonical_slot: 7,
                tag: b"MOD".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 101,
                canonical_slot: 7,
            }]));
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                id: task,
                roster_owner: 101,
                ..Default::default()
            }]));
        *frame.tables[TableId::Ships.index()] =
            TableView::Complete(TableRows::Ships(vec![ShipRow {
                id: ship,
                task_force: task,
                ..Default::default()
            }]));
        *frame.tables[TableId::NavalRefitShips.index()] =
            TableView::Complete(TableRows::NavalRefitShips(vec![NavalRefitShipRow {
                ship,
                line,
            }]));
        *frame.tables[TableId::NavalRefitProgress.index()] =
            TableView::Complete(TableRows::NavalRefitProgress(vec![NavalRefitProgressRow {
                line,
                progress_raw: 123456,
            }]));
        let params = NavalManagementParams {
            branch_id: "1".into(),
            hour: 1,
            tag: "MOD".into(),
        };
        // A refit can start after this country's daily production slice, or that
        // table can be refused altogether. Neither hides the captured progress.
        for production in [
            TableView::Complete(TableRows::ProductionLines(vec![])),
            TableView::Unavailable {
                reason: "synthetic production fault".into(),
            },
        ] {
            *frame.tables[TableId::ProductionLines.index()] = production;
            let reply = project(&frame, &params, None).unwrap();
            let lines = &reply
                .sections
                .iter()
                .find(|s| s.key == format!("task:{task}"))
                .unwrap()
                .lines;
            assert!(
                lines
                    .iter()
                    .any(|s| s == "Ship 4 refit progress: 1.23456 IC")
            );
            let metadata = lines
                .iter()
                .find(|s| s.starts_with("Last recorded target:"))
                .unwrap();
            assert!(metadata.contains("target: Unavailable"));
            assert!(metadata.contains("cost: Unavailable"));
            assert!(metadata.contains("dockyard allocation: Unavailable"));
        }
        *frame.tables[TableId::ProductionLines.index()] =
            TableView::Complete(TableRows::ProductionLines(vec![ProductionLineRow {
                handle: line,
                cost_raw: 234567,
                active_factories: 2,
                ..Default::default()
            }]));
        // The daily metadata may still name a cancelled/pruned line. Only the
        // complete hourly refit-production rows certify that it currently resolves.
        for lines in [
            vec![],
            vec![NavalRefitProgressRow {
                line: line + 1,
                progress_raw: 123456,
            }],
        ] {
            *frame.tables[TableId::NavalRefitProgress.index()] =
                TableView::Complete(TableRows::NavalRefitProgress(lines));
            let reply = project(&frame, &params, None).unwrap();
            assert!(
                !reply
                    .sections
                    .iter()
                    .flat_map(|section| &section.lines)
                    .any(|text| text.starts_with("Ship 4 refit")
                        || text.starts_with("Last recorded target:"))
            );
        }
        *frame.tables[TableId::NavalRefitProgress.index()] = TableView::Unavailable {
            reason: "synthetic progress fault".into(),
        };
        let reply = project(&frame, &params, None).unwrap();
        let lines = &reply
            .sections
            .iter()
            .find(|s| s.key == format!("task:{task}"))
            .unwrap()
            .lines;
        assert!(
            lines
                .iter()
                .any(|s| s == "Ship 4 refit progress unavailable")
        );
        assert!(lines.iter().any(|s| s
            == "Last recorded target: None; cost: 2.34567 IC; dockyard allocation: 2 dockyards"));
    }

    #[test]
    fn naval_management_projection_keeps_native_amounts_and_foreign_ports_out() {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 101,
                canonical_slot: 7,
                tag: b"MOD".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 7,
                },
                CountryLinkRow {
                    id: 101,
                    canonical_slot: 7,
                },
            ]));
        *frame.tables[TableId::NavalRepairCountries.index()] =
            TableView::Complete(TableRows::NavalRepairCountries(vec![
                NavalRepairCountryRow {
                    country: 101,
                    max_factories_for_repair: -1,
                    max_allowed_dockyards_to_repair: 3,
                    num_used_dockyards: 2,
                },
            ]));
        *frame.tables[TableId::NavalBases.index()] =
            TableView::Complete(TableRows::NavalBases(vec![
                NavalBaseRow {
                    province: 9,
                    country: 11,
                    ..Default::default()
                },
                NavalBaseRow {
                    province: 10,
                    country: 12,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::NavalRepairBases.index()] =
            TableView::Complete(TableRows::NavalRepairBases(vec![
                NavalRepairBaseRow {
                    province: 9,
                    priority: -3,
                    allocated_dockyards: 2,
                },
                NavalRepairBaseRow {
                    province: 10,
                    priority: 0,
                    allocated_dockyards: 1,
                },
            ]));
        *frame.tables[TableId::NavalRepairQueue.index()] =
            TableView::Complete(TableRows::NavalRepairQueue(vec![NavalRepairQueueRow {
                province: 9,
                ordinal: 0,
                ship: u64::MAX,
            }]));
        let params = NavalManagementParams {
            branch_id: "1".into(),
            hour: 1,
            tag: "MOD".into(),
        };
        let reply = project(&frame, &params, None).unwrap();
        assert!(
            reply
                .sections
                .iter()
                .find(|section| section.key == "budgets")
                .unwrap()
                .lines[1]
                .ends_with("-1")
        );
        assert!(reply.sections.iter().any(|section| section.key == "base:9"));
        assert!(
            !reply
                .sections
                .iter()
                .any(|section| section.key == "base:10")
        );
        *frame.tables[TableId::NavalRepairQueue.index()] = TableView::Unavailable {
            reason: "synthetic queue fault".into(),
        };
        let reply = project(&frame, &params, None).unwrap();
        assert!(
            reply
                .warnings
                .iter()
                .any(|line| line.contains("synthetic queue fault"))
        );
        assert!(
            reply
                .sections
                .iter()
                .any(|section| section.key == "budgets")
        );
        assert_eq!(fixed(i64::MIN), "-92233720368547.75808");
    }
}
