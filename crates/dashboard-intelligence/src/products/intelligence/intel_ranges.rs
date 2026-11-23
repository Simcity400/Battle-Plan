use super::{
    intel_details::{IntelDetail, detail, diplomacy_unlock, v},
    intel_explanations,
};
use content_reader_defines::defines::Defines;
use content_reader_script::sub_units::SubUnits;
use replay_data::{
    intel_estimates::{self as native, Parameters},
    rows::{self, TableId, tables},
};
use replay_frame::frame::DecodedFrame;

pub(super) const FUEL_TABLES: &[TableId] = &[
    TableId::CountryLinks,
    TableId::CountryIntel,
    TableId::CountryFuel,
    TableId::WarRelations,
    TableId::FactionMembers,
    TableId::Subjects,
    TableId::Clock,
];

struct Metric {
    key: &'static str,
    label: &'static str,
    lane: usize,
    minimum: &'static str,
    maximum: &'static str,
    spread: &'static str,
    salt: i32,
}
const METRICS: &[Metric] = &[
    Metric {
        key: "divisions",
        label: "Divisions",
        lane: 1,
        minimum: "ARMY_ARMY_COUNT_RANGE_INTEL_MIN",
        maximum: "ARMY_ARMY_COUNT_RANGE_INTEL_MAX",
        spread: "ARMY_ARMY_COUNT_RANGE_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1140,
    },
    Metric {
        key: "special_forces",
        label: "Special forces",
        lane: 1,
        minimum: "ARMY_ARMY_COUNT_RANGE_INTEL_MIN",
        maximum: "ARMY_ARMY_COUNT_RANGE_INTEL_MAX",
        spread: "ARMY_SPECIAL_FORCES_COUNT_RANGE_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1161,
    },
    Metric {
        key: "army_manpower",
        label: "Army manpower",
        lane: 1,
        minimum: "ARMY_DEPLOYED_MANPOWER_COUNT_RANGE_INTEL_MIN",
        maximum: "ARMY_DEPLOYED_MANPOWER_COUNT_RANGE_INTEL_MAX",
        spread: "ARMY_DEPLOYED_MANPOWER_COUNT_RANGE_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 962,
    },
    Metric {
        key: "fuel",
        label: "Fuel",
        lane: 0,
        minimum: "CIVILIAN_FUEL_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_FUEL_RANGE_INTEL_MAX",
        spread: "CIVILIAN_FUEL_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 618,
    },
    Metric {
        key: "production",
        label: "Production",
        lane: 0,
        minimum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MAX",
        spread: "CIVILIAN_PRODUCTION_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 604,
    },
    Metric {
        key: "available_manpower",
        label: "Available manpower",
        lane: 0,
        minimum: "CIVILIAN_MANPOWER_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_MANPOWER_RANGE_INTEL_MAX",
        spread: "CIVILIAN_MANPOWER_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 644,
    },
    Metric {
        key: "convoys",
        label: "Convoys",
        lane: 0,
        minimum: "CIVILIAN_CONVOYS_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_MIN_INTEL_TO_SHOW_CONVOYS_GRAPH",
        spread: "CIVILIAN_CONVOYS_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 631,
    },
    Metric {
        key: "trucks",
        label: "Trucks",
        lane: 0,
        minimum: "CIVILIAN_TRUCKS_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_TRUCKS_RANGE_INTEL_MAX",
        spread: "CIVILIAN_TRUCKS_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1671,
    },
    Metric {
        key: "trains",
        label: "Trains",
        lane: 0,
        minimum: "CIVILIAN_TRAINS_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_TRAINS_RANGE_INTEL_MAX",
        spread: "CIVILIAN_TRAINS_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1681,
    },
    Metric {
        key: "supply",
        label: "Supply",
        lane: 0,
        minimum: "CIVILIAN_SUPPLY_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_SUPPLY_RANGE_INTEL_MAX",
        spread: "CIVILIAN_SUPPLY_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1692,
    },
    Metric {
        key: "civilian_industry",
        label: "Civilian industry",
        lane: 0,
        minimum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MAX",
        spread: "CIVILIAN_PRODUCTION_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 521,
    },
    Metric {
        key: "military_industry",
        label: "Military industry",
        lane: 0,
        minimum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MAX",
        spread: "CIVILIAN_PRODUCTION_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 534,
    },
    Metric {
        key: "naval_industry",
        label: "Naval industry",
        lane: 0,
        minimum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MAX",
        spread: "CIVILIAN_PRODUCTION_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 547,
    },
    Metric {
        key: "industry_total",
        label: "Assigned industry",
        lane: 0,
        minimum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MAX",
        spread: "CIVILIAN_PRODUCTION_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 565,
    },
    Metric {
        key: "production_category",
        label: "Production allocation",
        lane: 0,
        minimum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MIN",
        maximum: "CIVILIAN_PRODUCTION_RANGE_INTEL_MAX",
        spread: "CIVILIAN_PRODUCTION_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 590,
    },
    Metric {
        key: "fleets",
        label: "Fleets",
        lane: 2,
        minimum: "NAVY_FLEET_COUNT_INTEL_MIN",
        maximum: "NAVY_FLEET_COUNT_INTEL_MAX",
        spread: "NAVY_FLEET_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1218,
    },
    Metric {
        key: "task_forces",
        label: "Task forces",
        lane: 2,
        minimum: "NAVY_TASKFORCE_COUNT_INTEL_MIN",
        maximum: "NAVY_TASKFORCE_COUNT_INTEL_MAX",
        spread: "NAVY_TASKFORCE_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1230,
    },
    Metric {
        key: "ships",
        label: "Ships",
        lane: 2,
        minimum: "NAVY_SHIP_COUNT_INTEL_MIN",
        maximum: "NAVY_SHIP_COUNT_INTEL_MAX",
        spread: "NAVY_SHIP_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1249,
    },
    Metric {
        key: "navy_manpower",
        label: "Navy manpower",
        lane: 2,
        minimum: "NAVY_DEPLOYED_MANPOWER_COUNT_RANGE_INTEL_MIN",
        maximum: "NAVY_DEPLOYED_MANPOWER_COUNT_RANGE_INTEL_MAX",
        spread: "NAVY_DEPLOYED_MANPOWER_COUNT_RANGE_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 976,
    },
    Metric {
        key: "air_wings",
        label: "Aircraft",
        lane: 3,
        minimum: "AIR_AIRWING_COUNT_INTEL_MIN",
        maximum: "AIR_AIRWING_COUNT_INTEL_MAX",
        spread: "AIR_AIRWING_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1280,
    },
    Metric {
        key: "air_manpower",
        label: "Air manpower",
        lane: 3,
        minimum: "AIR_DEPLOYED_MANPOWER_COUNT_RANGE_INTEL_MIN",
        maximum: "AIR_DEPLOYED_MANPOWER_COUNT_RANGE_INTEL_MAX",
        spread: "AIR_DEPLOYED_MANPOWER_COUNT_RANGE_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 990,
    },
    Metric {
        key: "template_divisions",
        label: "Template divisions",
        lane: 1,
        minimum: "ARMY_TEMPLATE_UNIT_COUNT_INTEL_MIN",
        maximum: "ARMY_TEMPLATE_UNIT_COUNT_INTEL_MAX",
        spread: "ARMY_TEMPLATE_UNIT_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1186,
    },
    Metric {
        key: "stockpile",
        label: "Stockpile equipment groups",
        lane: 1,
        minimum: "ARMY_STOCKPILE_COUNT_INTEL_MIN",
        maximum: "ARMY_STOCKPILE_COUNT_INTEL_MAX",
        spread: "ARMY_STOCKPILE_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1174,
    },
    Metric {
        key: "air_type",
        label: "Aircraft types",
        lane: 3,
        minimum: "AIR_WING_TYPE_COUNT_INTEL_MIN",
        maximum: "AIR_WING_TYPE_COUNT_INTEL_MAX",
        spread: "AIR_WING_TYPE_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1293,
    },
    Metric {
        key: "ship_type",
        label: "Ship types",
        lane: 2,
        minimum: "NAVY_SHIP_TYPE_COUNT_INTEL_MIN",
        maximum: "NAVY_SHIP_TYPE_COUNT_INTEL_MAX",
        spread: "NAVY_SHIP_TYPE_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
        salt: 1261,
    },
];
fn slot(frame: &DecodedFrame, raw: u32) -> Result<u32, String> {
    if raw == 0 {
        return Ok(0);
    }
    frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| format!("country links for intel estimate: {e}"))?
        .iter()
        .find(|r| r.id == raw)
        .map(|r| r.canonical_slot)
        .ok_or_else(|| format!("country links lacks intel estimate country {raw}"))
}
fn same(frame: &DecodedFrame, raw: u32, target: u32) -> Result<bool, String> {
    Ok(raw == target || slot(frame, raw)? == slot(frame, target)?)
}
fn lanes(frame: &DecodedFrame, viewer: u32, target: u32) -> Result<[i64; 4], String> {
    if target == 0 {
        return Ok([0; 4]);
    }
    let target_slot = slot(frame, target)?;
    let viewer_slot = slot(frame, viewer)?;
    let rows = frame
        .complete::<tables::CountryIntel>()
        .map_err(|e| format!("country intel for viewer {viewer} / target {target}: {e}"))?;
    for row in rows {
        if row.target_index == target_slot && slot(frame, row.owner_country)? == viewer_slot {
            return Ok([row.civilian, row.army, row.navy, row.air]);
        }
    }
    Ok([0; 4])
}
fn count(count: usize) -> i64 {
    i64::from(count as i32) << 15
}
fn quantity(value: i32) -> i64 {
    i64::from(value) << 15
}
fn special_forces(
    frame: &DecodedFrame,
    target: u32,
    content: Result<&SubUnits, &str>,
) -> Result<i64, String> {
    let units = frame
        .complete::<tables::Divisions>()
        .map_err(|e| format!("Divisions: {e}"))?;
    let templates = frame
        .complete::<tables::DivisionTemplateIcons>()
        .map_err(|e| format!("division template icons: {e}"))?;
    let tokens = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|e| format!("lexical tokens for special-forces definitions: {e}"))?;
    let content = content.map_err(str::to_owned)?;
    let mut count = 0_i32;
    for unit in units {
        if !same(frame, unit.roster_owner, target)? {
            continue;
        }
        let template = templates
            .iter()
            .find(|row| row.template == unit.template)
            .ok_or_else(|| {
                format!(
                    "division template icons lacks special-forces template {}",
                    unit.template
                )
            })?;
        if template.model_unreadable != 0 {
            return Err(format!(
                "division template icons composition unavailable for {}",
                unit.template
            ));
        }
        let (mut all, mut special) = (0_i32, 0_i32);
        for subunit in &template.subunits {
            all = all.wrapping_add(subunit.count);
            if subunit.token == 0 {
                continue;
            }
            let name = &tokens
                .iter()
                .find(|row| row.token == subunit.token)
                .ok_or_else(|| format!("lexical tokens lacks sub-unit {}", subunit.token))?
                .literal;
            if content.special_forces(name)? {
                special = special.wrapping_add(subunit.count);
            }
        }
        let ratio = if all == 0 {
            0
        } else {
            i64::from(special).wrapping_mul(10_000_000_000) / (i64::from(all) * 100_000)
        };
        if ratio > 0 {
            count = count.wrapping_add(1);
        }
    }
    Ok(quantity(count))
}
fn truth(
    frame: &DecodedFrame,
    target: u32,
    key: &str,
    subunits: Result<&SubUnits, &str>,
) -> Result<i64, String> {
    let target_slot = slot(frame, target)?;
    if target_slot == 0 {
        return Err("Reserved slot-zero truth rosters are not recorded for intel estimates".into());
    }
    macro_rules! rows {
        ($table:ident) => {
            frame
                .complete::<tables::$table>()
                .map_err(|e| format!("{} for intel estimate: {e}", stringify!($table)))?
        };
    }
    let matching = |id| slot(frame, id).map(|slot| slot == target_slot);
    match key {
        "divisions"=>{
            let mut total=0_i32;
            for row in rows!(Divisions) {if matching(row.roster_owner)? {total=total.wrapping_add(1);}}
            Ok(quantity(total))
        }
        "special_forces"=>special_forces(frame,target,subunits),
        "fleets"=>{
            let mut total=0_i32;
            for row in rows!(Fleets) {if matching(row.owner)? {total=total.wrapping_add(1);}}
            Ok(quantity(total))
        }
        "task_forces"|"ships"=>{
            let mut held=std::collections::BTreeSet::new();
            for row in rows!(TaskForces) {if matching(row.roster_owner)? {held.insert(row.id);}}
            if key=="task_forces" {return Ok(count(held.len()));}
            Ok(count(rows!(Ships).iter().filter(|r|held.contains(&r.task_force)).count()))
        }
        "air_wings"=>{
            let mut total=0_i32;
            for row in rows!(AirWings) {if matching(row.owner)? {total=total.wrapping_add(row.planes);}}
            Ok(quantity(total))
        }
        "available_manpower"=>{
            let countries=rows!(Countries);
            let country=countries.iter().find(|r|r.canonical_slot==target_slot).ok_or("Countries lacks intel estimate target")?;
            let mut total=country.manpower_residual;
            let counters=rows!(CountryCounterInputs);
            let counters=counters.iter().find(|r|r.country==country.id).ok_or("country counter inputs lacks target controlled-state membership")?;
            let states=counters.controlled_states.require("country counter inputs controlled states")?.as_ref().ok_or("country counter inputs controlled states unavailable")?;
            let values=rows!(States);
            for state in states {
                let row=values.iter().find(|r|r.id==*state).ok_or_else(||format!("States lacks controlled state {state}"))?;
                total=total.wrapping_add(row.manpower_available);
            }
            Ok(quantity(total))
        }
        "fuel"=>{
            if frame.tables[TableId::CountryFuel.index()].sampled_at()!=Some(frame.hour) {
                return Err("country fuel quantity not recorded at its supplying hour".into());
            }
            let row=rows!(CountryFuel).iter().find(|r|slot(frame,r.country)==Ok(target_slot)).ok_or("country fuel lacks intel estimate target")?;
            Ok(row.current_raw)
        }
        "convoys"=>{
            let row=rows!(Countries).iter().find(|r|r.canonical_slot==target_slot).ok_or("Countries lacks intel estimate target")?;
            let raw=row.convoys_raw.ok_or("Countries convoys cache unavailable")?;
            Ok(quantity((raw/100_000) as i32))
        }
        "civilian_industry"|"military_industry"|"naval_industry"|"production"=>{
            let row=rows!(CountryFactories).iter().find(|r|slot(frame,r.country)==Ok(target_slot)).ok_or("country factories lacks intel estimate target")?;
            let civilian=((row.civilian_total_raw/100_000) as i32).wrapping_sub(row.civilian_to_others).wrapping_sub(row.civilian_to_overlord);
            let military=((row.military_total_raw/100_000) as i32).wrapping_sub(row.military_to_others).wrapping_sub(row.military_to_overlord);
            let naval=(row.naval_total_raw/100_000) as i32;
            Ok(quantity(match key {"civilian_industry"=>civilian,"military_industry"=>military,"naval_industry"=>naval,_=>civilian.wrapping_add(military).wrapping_add(naval)}))
        }
        "trucks"=>{
            let read=replay_data::sliced::read_hour(rows!(SliceReads),TableId::TrainInventory,target_slot);
            if read!=Some(frame.hour) {return Err("train inventory allocated-truck cache not freshly recorded for this hour".into());}
            let row=rows!(TrainInventory).iter().find(|r|r.country==target_slot).ok_or("train inventory lacks intel estimate target")?;
            let amount=row.allocated_trucks.require("train inventory allocated trucks")?.ok_or("train inventory allocated trucks unavailable")?;
            Ok(quantity(amount))
        }
        "trains"=>Err("National intelligence train-count cache is not recorded; allocated/required trains are different operands".into()),
        "supply"=>Err("National intelligence supply-fulfilment aggregate is not recorded".into()),
        "army_manpower"=>Err("Army manpower effective-controller and owned-expeditionary membership operands are not recorded".into()),
        "navy_manpower"=>Err("Stored ship crew amounts for navy manpower are not recorded".into()),
        "air_manpower"=>Err("Stored wing manpower and volunteer-source operands are not recorded".into()),
        "industry_total"=>Err("Production-line intelligence allocation counter is not recorded".into()),
        "production_category"=>Err("Production-line allocation counters needed for category totals are not recorded".into()),
        "template_divisions"=>Err("Per-viewer template grouping provenance is not recorded; standard and alternate holders differ".into()),
        "stockpile"=>Err("Stockpile available/deficit group aggregates and runtime equipment-category identities are not recorded".into()),
        "air_type"=>Err("Aircraft-type runtime equipment-category identities and group-source operands are not recorded".into()),
        "ship_type"=>Err("Ship-type runtime equipment-category identities and group-source operands are not recorded".into()),
        _=>Err(format!("Unknown intel estimate kind {key}")),
    }
}
fn normalized(
    frame: &DecodedFrame,
    defines: &Defines,
    viewer: u32,
    target: u32,
    lane: usize,
) -> Result<(Vec<i64>, String), String> {
    let context = diplomacy_unlock(frame, viewer, target);
    if context.as_ref().is_ok_and(|reason| reason.is_some()) {
        return Ok((vec![100_000], String::new()));
    }
    let cap = intel_explanations::country_caps(defines)?[lane];
    let ordinary = if cap == 0 {
        i64::from(u32::MAX)
    } else {
        native::normalized(lanes(frame, viewer, target)?[lane], cap).map_err(str::to_owned)?
    };
    let missing = context
        .err()
        .unwrap_or_else(|| "Intel fog-of-war console switch is not recorded".into());
    Ok((
        if ordinary == 100_000 {
            vec![ordinary]
        } else {
            vec![ordinary, 100_000]
        },
        missing,
    ))
}

pub(super) fn entries(
    frame: &DecodedFrame,
    fuel_frame: Result<&DecodedFrame, &str>,
    viewer: u32,
    target: u32,
    defines: Result<&Defines, &str>,
    subunits: Result<&SubUnits, &str>,
) -> Vec<IntelDetail> {
    METRICS
        .iter()
        .map(|metric| {
            let mut entry = detail(
                format!("estimate:{viewer}:{target}:{}", metric.key),
                "Player estimates",
                metric.label,
                viewer,
                Some(target),
                Vec::new(),
            );
            let result = (|| -> Result<(), String> {
                let frame = if metric.key == "fuel" {
                    let supplied = fuel_frame.map_err(str::to_owned)?;
                    entry.supplying_hour = Some(supplied.hour);
                    supplied
                } else {
                    frame
                };
                let defines = defines.map_err(str::to_owned)?;
                let (alternatives, missing_context) =
                    normalized(frame, defines, viewer, target, metric.lane)?;
                entry.explanations.push(v(
                    "Normalized intel alternatives, fixed5",
                    format!("{alternatives:?}"),
                ));
                let minimum = defines
                    .fixed5("NIntel", metric.minimum)
                    .map_err(|e| e.to_string())?;
                entry.explanations.push(v(
                    format!("Availability threshold {}", metric.minimum),
                    minimum,
                ));
                if alternatives.iter().all(|intel| *intel < minimum) {
                    entry.values.push(v("Estimated range", "?"));
                    return Ok(());
                }
                if alternatives.iter().any(|intel| *intel < minimum) {
                    return Err(format!(
                        "{missing_context}; its alternatives change availability"
                    ));
                }
                let amount = truth(frame, target, metric.key, subunits)?;
                if amount <= 0 {
                    entry.values.push(v("Estimated range", "0 \u{2013} 0"));
                    return Ok(());
                }
                let maximum = defines
                    .fixed5("NIntel", metric.maximum)
                    .map_err(|e| e.to_string())?;
                let jitter = alternatives
                    .iter()
                    .map(|intel| native::needs_jitter(*intel, minimum, maximum))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(str::to_owned)?
                    .into_iter()
                    .any(|needed| needed);
                let spread = if jitter {
                    defines
                        .fixed5("NIntel", metric.spread)
                        .map_err(|e| e.to_string())?
                } else {
                    0
                };
                let params = Parameters {
                    minimum_intel: minimum,
                    exact_intel: maximum,
                    lowest_spread: spread,
                };
                let unused_clock = rows::ClockRow::default();
                let clock = if jitter {
                    frame
                        .complete::<tables::Clock>()
                        .map_err(|e| format!("Clock for deterministic intel range: {e}"))?
                        .first()
                        .ok_or("Clock seed/date not recorded for intel range")?
                } else {
                    &unused_clock
                };
                let display_bounds = |range: native::Range| {
                    if metric.key == "fuel" {
                        (range.lower_q15 >> 15, range.upper_q15 >> 15)
                    } else {
                        let (a, b) = native::count_bounds(range);
                        (i64::from(a), i64::from(b))
                    }
                };
                let mut ranges = alternatives.into_iter().map(|intel| {
                    native::range(amount, intel, params, clock, metric.salt).map_err(str::to_owned)
                });
                let bounds = display_bounds(ranges.next().unwrap()?);
                for alternative in ranges {
                    if display_bounds(alternative?) != bounds {
                        return Err(format!(
                            "{missing_context}; its alternatives change the range"
                        ));
                    }
                }
                entry.values.push(v(
                    "Estimated range",
                    format!("{} \u{2013} {}", bounds.0, bounds.1),
                ));
                entry.explanations.extend([
                    v(
                        "Intel lane",
                        ["Civilian", "Army", "Navy", "Air"][metric.lane],
                    ),
                    v(format!("Exact-range threshold {}", metric.maximum), maximum),
                ]);
                if jitter {
                    entry
                        .explanations
                        .push(v(format!("Lowest-intel spread {}", metric.spread), spread));
                } else {
                    entry.explanations.push(v(
                        "Range precision",
                        "Exact; spread and seed inputs not reached",
                    ));
                }
                if let Ok(Some(reason)) = diplomacy_unlock(frame, viewer, target) {
                    entry.explanations.push(v("Unlock", reason));
                }
                Ok(())
            })();
            if let Err(reason) = result {
                entry.unavailable = Some(reason);
                entry.values.push(v("Estimated range", "Unavailable"));
            }
            entry
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    #[test]
    fn zero_caps_and_nonfinite_intel_use_the_wide_fallback_without_ui_or_missing_unused_inputs() {
        let mut frame = dashboard_core::test_fixtures::source(1, 7, None);
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                rows::CountryLinkRow {
                    id: 17,
                    canonical_slot: 8,
                },
                rows::CountryLinkRow {
                    id: 9001,
                    canonical_slot: 7,
                },
            ]));
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![rows::DivisionRow {
                roster_owner: 9001,
                ..Default::default()
            }]));
        let defines = Defines::new().unwrap();
        defines.load("synthetic.lua",br#"NDefines={NIntel={COUNTRY_LEVEL_INTEL_MAXIMUMS={0,0,0,0},ARMY_ARMY_COUNT_RANGE_INTEL_MIN=0.25,ARMY_ARMY_COUNT_RANGE_INTEL_MAX=0.75}}"#).unwrap();
        let result = entries(
            &frame,
            Ok(&frame),
            17,
            9001,
            Ok(&defines),
            Err("sub-unit content missing"),
        );
        let division = result.iter().find(|r| r.label == "Divisions").unwrap();
        assert_eq!(division.values[0].value, "1 \u{2013} 1");
        assert!(
            division.unavailable.is_none(),
            "unused country intel, Clock, spread or diplomacy gaps cannot make the native zero-cap result harsher"
        );
        defines
            .load(
                "mod.lua",
                br#"NDefines.NIntel.COUNTRY_LEVEL_INTEL_MAXIMUMS={1,1,1,1}"#,
            )
            .unwrap();
        for value in [i64::MIN, i64::MIN + 1, i64::MAX] {
            *frame.tables[TableId::CountryIntel.index()] =
                TableView::Complete(TableRows::CountryIntel(vec![rows::CountryIntelRow {
                    owner_country: 17,
                    target_index: 7,
                    army: value,
                    civilian: 0,
                    navy: 0,
                    air: 0,
                }]));
            assert_eq!(
                normalized(&frame, &defines, 17, 9001, 1).unwrap().0[0],
                4294967295
            );
        }
    }
    #[test]
    fn fuel_ranges_use_every_operand_at_the_labelled_supplying_hour() {
        let hour = game_model::calendar::HOUR_EPOCH + 1936 * 365 * 24 + 17;
        let mut supplied = dashboard_core::test_fixtures::source(1, hour, None);
        *supplied.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![rows::ClockRow {
                hour,
                seed: 123,
                ..Default::default()
            }]));
        supplied.tables[TableId::CountryFuel.index()] = replay_frame::frame::SharedView::from(
            TableView::Complete(TableRows::CountryFuel(vec![rows::CountryFuelRow {
                country: 9001,
                current_raw: quantity(1000),
                ..Default::default()
            }])),
        )
        .with_sampled_at(Some(hour));
        let mut current = supplied.clone();
        current.hour += 24 * 31 + 1;
        *current.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "current aliases are unrelated".into(),
        };
        *current.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![rows::ClockRow {
                hour: current.hour,
                seed: 999,
                ..Default::default()
            }]));
        for maximum in ["0.75", "2"] {
            let defines = Defines::new().unwrap();
            defines.load("synthetic.lua", format!(
                "NDefines={{NIntel={{CIVILIAN_FUEL_RANGE_INTEL_MIN=0.25,CIVILIAN_FUEL_RANGE_INTEL_MAX={maximum},CIVILIAN_FUEL_INTEL_RANGE_AT_LOWEST_INTEL=1}}}}"
            ).as_bytes()).unwrap();
            let estimate = |query: &DecodedFrame, fuel| {
                entries(
                    query,
                    fuel,
                    9001,
                    9001,
                    Ok(&defines),
                    Err("unused subunits"),
                )
                .into_iter()
                .find(|r| r.label == "Fuel")
                .unwrap()
            };
            let fresh = estimate(&supplied, Ok(&supplied));
            let retained = estimate(&current, Ok(&supplied));
            assert!(retained.unavailable.is_none(), "{:?}", retained.unavailable);
            assert_eq!(retained.values[0].value, fresh.values[0].value);
            assert_eq!(retained.supplying_hour, Some(hour));
            if maximum == "0.75" {
                assert_eq!(retained.values[0].value, "1000 \u{2013} 1000");
            }
            let mut missing = supplied.clone();
            *missing.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
                reason: "supplying aliases missing".into(),
            };
            let refused = estimate(&supplied, Ok(&missing));
            assert!(
                refused
                    .unavailable
                    .as_deref()
                    .unwrap()
                    .contains("CountryLinks")
            );
            assert_eq!(refused.values[0].value, "Unavailable");
            assert_eq!(refused.supplying_hour, Some(hour));
            if maximum == "2" {
                let mut missing = supplied.clone();
                *missing.tables[TableId::Clock.index()] = TableView::Unavailable {
                    reason: "supplying seed missing".into(),
                };
                assert!(
                    estimate(&current, Ok(&missing))
                        .unavailable
                        .unwrap()
                        .contains("Clock")
                );
            }
        }
    }

    #[test]
    fn range_input_cadences_keep_daily_and_sliced_freshness_explicit() {
        use replay_data::cadence::{Cadence, cadence};
        for table in [
            TableId::Countries,
            TableId::CountryLinks,
            TableId::Divisions,
            TableId::DivisionTemplateIcons,
            TableId::Fleets,
            TableId::TaskForces,
            TableId::Ships,
            TableId::AirWings,
            TableId::CountryCounterInputs,
            TableId::States,
            TableId::CountryFactories,
            TableId::WarRelations,
            TableId::FactionMembers,
            TableId::Subjects,
            TableId::Clock,
        ] {
            assert_eq!(
                cadence(table),
                Cadence::Hourly,
                "{table:?}: audit freshness before using a slower input"
            );
        }
        assert_eq!(cadence(TableId::LexicalTokens), Cadence::Static);
        assert_eq!(cadence(TableId::CountryIntel), Cadence::Daily(0));
        assert_eq!(cadence(TableId::CountryFuel), Cadence::Daily(17));
        assert_eq!(cadence(TableId::TrainInventory), Cadence::Sliced);
    }

    #[test]
    fn semantic_manpower_lanes_and_thresholds_match_the_getters_not_panel_location() {
        for (key, lane) in [
            ("army_manpower", 1),
            ("navy_manpower", 2),
            ("air_manpower", 3),
        ] {
            assert_eq!(METRICS.iter().find(|m| m.key == key).unwrap().lane, lane);
        }
        for metric in METRICS {
            let prefix = ["CIVILIAN_", "ARMY_", "NAVY_", "AIR_"][metric.lane];
            assert!(
                metric.minimum.starts_with(prefix)
                    && metric.maximum.starts_with(prefix)
                    && metric.spread.starts_with(prefix)
            );
        }
    }
    #[test]
    fn pairs_need_no_ui_observation_and_missing_truth_is_named_instead_of_replaced_by_totals() {
        let mut frame = dashboard_core::test_fixtures::source(1, 7, None);
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![rows::DivisionRow {
                roster_owner: 9001,
                ..Default::default()
            }]));
        let defines = Defines::new().unwrap();
        defines.load("synthetic.lua",br#"NDefines={NIntel={ARMY_ARMY_COUNT_RANGE_INTEL_MIN=0.25,ARMY_ARMY_COUNT_RANGE_INTEL_MAX=0.75,ARMY_ARMY_COUNT_RANGE_INTEL_RANGE_AT_LOWEST_INTEL=1}}"#).unwrap();
        let rows = entries(
            &frame,
            Ok(&frame),
            9001,
            9001,
            Ok(&defines),
            Err("sub-unit content missing"),
        );
        let division = rows.iter().find(|r| r.label == "Divisions").unwrap();
        assert_eq!(division.values[0].value, "1 \u{2013} 1");
        assert!(division.unavailable.is_none());
        assert!(rows.iter().any(|r| {
            r.unavailable
                .as_deref()
                .is_some_and(|r| r.contains("NIntel"))
        }));
    }
}
