
use super::*;
use replay_data::{
    rows::CountryCounterInputRow, rows::FleetCounterInputRow, rows::TaskForceCounterInputRow,
    rows::TaskForceRow,
};
use viewer_native_gui::counter_hovers::task_force::{Control, Hover};
use viewer_unit_reads::counter_order;
use viewer_foundation::tooltip_text::ensure_newlines;

fn force<'a>(context: &Context<'a>, id: u64) -> Result<Option<&'a TaskForceRow>, String> {
    let rows = context
        .frame
        .complete::<tables::TaskForces>()
        .map_err(|e| e.to_string())?;
    Ok(rows
        .binary_search_by_key(&id, |row| row.id)
        .ok()
        .map(|index| &rows[index]))
}

fn inputs<'a>(context: &Context<'a>, id: u64) -> Result<&'a TaskForceCounterInputRow, String> {
    let rows = context
        .frame
        .complete::<tables::TaskForceCounterInputs>()
        .map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&id, |row| row.task_force)
        .map(|index| &rows[index])
        .map_err(|_| format!("TaskForceCounterInputs task force {id} is absent"))
}

fn fleet<'a>(context: &Context<'a>, id: u64) -> Result<&'a FleetCounterInputRow, String> {
    let rows = context
        .frame
        .complete::<tables::FleetCounterInputs>()
        .map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&id, |row| row.fleet)
        .map(|index| &rows[index])
        .map_err(|_| format!("FleetCounterInputs fleet {id} is absent"))
}

#[derive(Default)]
struct Any {
    found: bool,
    unknown: Option<String>,
}

impl Any {
    fn add(&mut self, term: Result<bool, String>) {
        match term {
            Ok(value) => self.found |= value,
            Err(reason) => {
                self.unknown.get_or_insert(reason);
            }
        }
    }

    fn result(self) -> Result<bool, String> {
        if self.found {
            Ok(true)
        } else {
            self.unknown.map_or(Ok(false), Err)
        }
    }
}

pub(super) fn specific(context: &Context<'_>, input: &Hover) -> Option<Tooltip> {
    let mut out = Tooltip::default();
    match input.control {
        Control::NoOrders => {
            let mut missing = Any::default();
            for &id in input.members.iter() {
                missing.add((|| {
                    let Some(force) = force(context, id)? else {
                        return Ok(false);
                    };
                    if !counter_inputs::needs_regions(force.mission) {
                        return Ok(false);
                    }
                    Ok(inputs(context, id)?.mission_region_count == 0)
                })());
                if missing.found {
                    break;
                }
            }
            let text = missing.result().and_then(|missing| {
                if missing {
                    localise(context.loc, "ARMY_NAVIES_MISSION_NO_REGIONS", &[])
                } else {
                    Ok(String::new())
                }
            });
            super::append(&mut out, context, false, "task force no orders", text);
        }
        Control::Carrier => carrier(context, &input.members, &mut out),
        Control::Pride => {
            if !pride(context, input, &mut out) {
                return None;
            }
        }
        _ => return None,
    }
    Some(out)
}

fn invalid_orders<'a>(
    context: &Context<'_>,
    input: &Hover,
    mut aggression: impl FnMut() -> Result<&'a [i64], String>,
    out: &mut Tooltip,
) {
    let mut hold_fire = Any::default();
    let mut idle = Any::default();
    for &id in input.members.iter() {
        let member = force(context, id).and_then(|force| {
            let Some(force) = force else { return Ok(None) };
            if !matches!(force.mission, 2 | 3) && (force.mission != 0 || force.fleet == 0) {
                return Ok(None);
            }
            Ok(Some((force, inputs(context, id)?)))
        });
        let (force, operands) = match member {
            Ok(Some(member)) => member,
            Ok(None) => continue,
            Err(reason) => {
                hold_fire.add(Err(reason.clone()));
                idle.add(Err(reason));
                continue;
            }
        };
        if operands.repair_parent != 0 {
            continue;
        }
        if !hold_fire.found {
            hold_fire.add(if !matches!(force.mission, 2 | 3) {
                Ok(false)
            } else if operands.running_away != 0 {
                Ok(true)
            } else {
                aggression().and_then(|values| {
                    counter_inputs::engagement_refused(
                        0,
                        operands.engagement_rule,
                        force.mission,
                        values,
                    )
                    .ok_or_else(|| {
                        format!(
                            "NNavy.AGGRESSION_SETTINGS_VALUES has no engagement rule {}",
                            operands.engagement_rule
                        )
                    })
                })
            });
        }
        if !idle.found {
            idle.add(if force.mission != 0 || force.fleet == 0 {
                Ok(false)
            } else {
                fleet(context, force.fleet).map(|fleet| fleet.mission_region_count != 0)
            });
        }
    }
    match hold_fire.result() {
        Ok(true) => {
            for key in [
                "SHIP_ENGAGEMENT_MAP_ICON",
                "SHIP_ENGAGEMENT_BUTTON_HOLD_FIRE_DESC",
            ] {
                super::append(
                    out,
                    context,
                    false,
                    "task force engagement",
                    localise(context.loc, key, &[]),
                );
            }
        }
        Ok(false) => (),
        Err(reason) => refuse(out, "task force engagement", reason),
    }
    match idle.result() {
        Ok(true) => super::append(
            out,
            context,
            false,
            "task force idle mission",
            localise(context.loc, "ARMY_NAVIES_MISSION_IDLE", &[]),
        ),
        Ok(false) => (),
        Err(reason) => refuse(out, "task force idle mission", reason),
    }
}

fn pride(context: &Context<'_>, input: &Hover, out: &mut Tooltip) -> bool {
    for &id in input.members.iter() {
        let chosen: Result<Option<&CountryCounterInputRow>, String> = (|| {
            let Some(force) = force(context, id)? else {
                return Ok(None);
            };
            let slot = context.slot(force.roster_owner)?;
            let roster = context
                .frame
                .complete::<tables::Countries>()
                .map_err(|e| e.to_string())?;
            let raw = roster
                .iter()
                .find(|country| country.canonical_slot == slot)
                .map(|country| country.id)
                .ok_or_else(|| format!("Countries canonical slot {slot} is absent"))?;
            let countries = context
                .frame
                .complete::<tables::CountryCounterInputs>()
                .map_err(|e| e.to_string())?;
            let country = countries
                .binary_search_by_key(&raw, |row| row.country)
                .map(|index| &countries[index])
                .map_err(|_| format!("CountryCounterInputs country {raw} is absent"))?;
            if country.pride_of_the_fleet == 0 {
                return Ok(None);
            }
            let ships = context
                .frame
                .complete::<tables::Ships>()
                .map_err(|e| e.to_string())?;
            let ship = ships
                .binary_search_by_key(&country.pride_of_the_fleet, |row| row.id)
                .ok()
                .map(|index| &ships[index]);
            let present = counter_inputs::pride(
                id,
                Some(country.pride_of_the_fleet),
                |_| ship.map(|ship| ship.task_force),
                true,
            )
            .expect("complete ship census");
            Ok(present.then_some(country))
        })();
        match chosen {
            Ok(None) => (),
            Ok(Some(country)) => {
                super::append(
                    out,
                    context,
                    false,
                    "task force pride name",
                    pride_name(context, country, id),
                );
                return true;
            }
            Err(reason) => {
                refuse(out, "task force pride", reason);
                return true;
            }
        }
    }
    false
}

pub(super) fn pride_name(
    context: &Context<'_>,
    country: &CountryCounterInputRow,
    task_force: u64,
) -> Result<String, String> {
    let bytes = country
        .pride_name
        .require("CountryCounterInputs.pride_name")?
        .as_ref()
        .ok_or_else(|| {
            format!(
                "CountryCounterInputs.pride_name for ship {} was unreadable when recorded",
                country.pride_of_the_fleet
            )
        })?;
    let name = dashboard_text::scripted_text::stored_text(
        context.loc,
        context.recorded_language,
        bytes,
        "pride ship retained name",
    )?;
    let navy_name = dashboard_text::unit_names::task_force_name(
        inputs(context, task_force)?,
        context.loc,
        context.recorded_language,
    )?;
    localise(
        context.loc,
        "SPECIFIC_NAVY_CONTAINS_PRIDE_OF_FLEET",
        &[
            ("NAVY_NAME", Arg::Text(&navy_name)),
            ("NAME", Arg::Text(&name)),
        ],
    )
}

pub(super) fn carrier(context: &Context<'_>, members: &[u64], out: &mut Tooltip) {
    let ships = context
        .frame
        .complete::<tables::Ships>()
        .map_err(|e| e.to_string());
    let models = context
        .frame
        .complete::<tables::ShipModels>()
        .map_err(|e| e.to_string());
    let registrations = context
        .frame
        .complete::<tables::AirBaseRegistrations>()
        .map_err(|e| e.to_string());
    for &id in members {
        let bases = (|| {
            if force(context, id)?.is_none() {
                return Ok(Vec::new());
            }
            let ships = ships.as_ref().map_err(Clone::clone)?;
            let mut roster = Vec::new();
            for ship in ships.iter().filter(|ship| ship.task_force == id) {
                let models = models.as_ref().map_err(Clone::clone)?;
                let model = models
                    .binary_search_by_key(&ship.id, |row| row.ship)
                    .map(|index| &models[index])
                    .map_err(|_| format!("ShipModels ship {} is absent", ship.id))?;
                let operands = model.counter.as_ref().ok_or_else(|| {
                    format!(
                        "ShipModels ship {} carrier and roster operands were not recorded",
                        ship.id
                    )
                })?;
                roster.push((operands.roster_ordinal, model));
            }
            roster.sort_by_key(|(ordinal, _)| *ordinal);
            let mut bases = Vec::new();
            for (_, ship) in roster {
                if !counter_order::is_carrier(ship).expect("counter operands checked") {
                    continue;
                }
                let base = ship
                    .counter
                    .as_ref()
                    .expect("counter operands checked")
                    .carrier_air_base;
                if base == 0 {
                    continue;
                }
                let registrations = registrations.as_ref().map_err(Clone::clone)?;
                let key = (base, context.vision.ours());
                if registrations
                    .binary_search_by_key(&key, |row| (row.base, row.slot))
                    .is_ok()
                {
                    bases.push(base);
                }
            }
            Ok::<_, String>(bases)
        })();
        match bases {
            Ok(bases) => {
                for base in bases {
                    super::carrier_summary::summary(context, base, out);
                }
            }
            Err(reason) => refuse(out, "task force carrier admission", reason),
        }
    }
}

pub(super) fn append(context: &Context<'_>, input: &Hover, out: &mut Tooltip) {
    match input.control {
        Control::InvalidOrders => {
            invalid_orders(
                context,
                input,
                || {
                    context
                        .content
                        .map(|content| content.aggression.as_slice())
                        .ok_or_else(|| "recorded counter content is unavailable".into())
                },
                out,
            );
        }
        Control::Supply if !input.coarse => {
            let permitted = (|| {
                let Some(&first) = input.members.first() else {
                    return Ok(false);
                };
                let Some(force) = force(context, first)? else {
                    return Ok(false);
                };
                let owner = context.slot(force.roster_owner)?;
                context
                    .vision
                    .shares_vision_proven(owner)
                    .map_err(|e| e.describe())
            })();
            match permitted {
                Ok(true) => {
                    let members = input.members.iter().filter_map(|&id| match force(context, id) {
                        Ok(None) => None,
                        Err(reason) => Some(Err(reason)),
                        Ok(Some(_)) => Some((|| {
                            inputs(context, id)?.supply.require("TaskForceCounterInputs.supply")?
                                .as_ref().ok_or_else(|| format!(
                                    "TaskForceCounterInputs.supply task force {id} was unreadable when recorded"))
                        })()),
                    }).collect::<Vec<_>>();
                    let mut result = Tooltip::default();
                    super::supply::member_footer(context, &members, &mut result);
                    if !result.body.is_empty() {
                        ensure_newlines(&mut out.title, 2);
                        out.title.push_str(&result.body);
                    }
                    for (reason, count) in result.refused {
                        *out.refused.entry(reason).or_default() += count;
                    }
                }
                Ok(false) => (),
                Err(reason) => refuse(out, "task force supply detail", reason),
            }
        }
        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::Recorded, rows::ShipRow, rows::TableRows,
        rows::UnitSupplyDemand, rows::UnitSupplyInputs,
    };
    use viewer_foundation::test_support::{test_frame, test_put};

    fn with_context(frame: &DecodedFrame, run: impl FnOnce(&Context<'_>)) {
        with_language(frame, None, run);
    }

    fn with_language(frame: &DecodedFrame, recorded: Option<&str>, run: impl FnOnce(&Context<'_>)) {
        let mut loc: Localisation = [
            ("ARMY_NAVIES_MISSION_NO_REGIONS", "no regions"),
            ("SHIP_ENGAGEMENT_MAP_ICON", "engagement:"),
            ("SHIP_ENGAGEMENT_BUTTON_HOLD_FIRE_DESC", "hold fire"),
            ("ARMY_NAVIES_MISSION_IDLE", "idle"),
            (
                "SPECIFIC_NAVY_CONTAINS_PRIDE_OF_FLEET",
                "$NAVY_NAME$: $NAME$",
            ),
            ("NEEDED_SUPPLY_WITH_VALUE", "needed $VAL|2$"),
            ("SUPPLY_SHIP_DEMAND", "ships $AMOUNT$ $VAL|2$"),
            ("RECEIVED_SUPPLY_WITH_VALUE", "received $VAL|2$ $RATIO$"),
            ("SUPPLY_FLOW_DISRUPTION", "disruption $SUPPLY|2$"),
        ]
        .into_iter()
        .map(|(key, text)| (key.into(), text.into()))
        .collect();
        loc.language = "english".into();
        super::super::task_force::tests::with_test_context(
            frame,
            &loc,
            "unreached",
            recorded,
            |context| run(&context),
        );
    }

    fn hover(control: Control, members: &[u64]) -> Hover {
        Hover {
            control,
            unit_hit: 0,
            rect: [0.; 4],
            hit: None,
            authored: None,
            supported: true,
            ours: false,
            coarse: false,
            hidden: true,
            members: members.into(),
        }
    }

    #[test]
    fn no_orders_does_not_require_inputs_for_a_mission_without_regions() {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                mission: 8,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let out = specific(context, &hover(Control::NoOrders, &[7])).unwrap();
            assert!(out.title.is_empty());
            assert!(out.refused.is_empty());
        });
    }

    #[test]
    fn no_orders_known_positive_survives_an_unknown_member_without_intel_or_repair_gates() {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 7,
                    mission: 9,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 999,
                    mission: 9,
                    ..Default::default()
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                task_force: 7,
                repair_parent: 55,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            for members in [&[999, 7][..], &[7, 999][..]] {
                let out = specific(context, &hover(Control::NoOrders, members)).unwrap();
                assert_eq!(out.title, "no regions");
                assert!(out.body.is_empty());
                assert!(out.refused.is_empty());
            }
        });
    }

    #[test]
    fn no_orders_skips_complete_dangling_members_without_demanding_mission_inputs() {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                mission: 8,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            for members in [&[999][..], &[7, 999, 7][..], &[999, 7][..]] {
                let out = specific(context, &hover(Control::NoOrders, members)).unwrap();
                assert!(out.title.is_empty() && out.body.is_empty());
                assert!(out.refused.is_empty(), "{:?}", out.refused);
            }
        });
    }

    #[test]
    fn other_naval_children_skip_complete_dangling_members_without_demanding_other_tables() {
        let mut frame = test_frame(0);
        test_put(&mut frame, TableRows::TaskForces(Vec::new()));
        with_context(&frame, |context| {
            let out = specific(context, &hover(Control::Carrier, &[999])).unwrap();
            assert!(out.title.is_empty() && out.body.is_empty());
            assert!(out.refused.is_empty(), "{:?}", out.refused);
            assert!(specific(context, &hover(Control::Pride, &[999])).is_none());
            let mut out = Tooltip::default();
            invalid_orders(
                context,
                &hover(Control::InvalidOrders, &[999]),
                || panic!("a dangling member cannot reach engagement defines"),
                &mut out,
            );
            assert!(out.title.is_empty() && out.body.is_empty());
            assert!(out.refused.is_empty(), "{:?}", out.refused);
        });
    }

    #[test]
    fn incomplete_member_census_does_not_turn_unknown_references_into_native_skips() {
        let frame = test_frame(0);
        with_context(&frame, |context| {
            for control in [Control::NoOrders, Control::Carrier, Control::Pride] {
                let out = specific(context, &hover(control, &[999])).unwrap();
                assert!(out.title.is_empty() && out.body.is_empty());
                assert!(!out.refused.is_empty());
            }
            let mut out = Tooltip::default();
            invalid_orders(
                context,
                &hover(Control::InvalidOrders, &[999]),
                || panic!("an unknown member cannot reach engagement defines"),
                &mut out,
            );
            assert!(out.title.is_empty() && out.body.is_empty());
            assert_eq!(out.refused.values().sum::<usize>(), 2);
        });
    }

    #[test]
    fn invalid_orders_retains_both_warnings_and_raw_idle_for_a_coarse_foreign_stack() {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 7,
                    mission: 2,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 8,
                    fleet: 10,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 9,
                    mission: 3,
                    ..Default::default()
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![
                TaskForceCounterInputRow {
                    task_force: 7,
                    running_away: 1,
                    ..Default::default()
                },
                TaskForceCounterInputRow {
                    task_force: 8,
                    detached_activity: 1,
                    repair_last_mission: 2,
                    ..Default::default()
                },
                TaskForceCounterInputRow {
                    task_force: 9,
                    repair_parent: 4,
                    engagement_rule: -1,
                    ..Default::default()
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::FleetCounterInputs(vec![FleetCounterInputRow {
                fleet: 10,
                mission_region_count: 1,
                mission_regions: (None).into(),
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let mut input = hover(Control::InvalidOrders, &[7, 8, 9]);
            input.coarse = true;
            let mut out = Tooltip {
                title: "summary".into(),
                body: "arrival".into(),
                ..Default::default()
            };
            invalid_orders(
                context,
                &input,
                || panic!("engagement define is unreached"),
                &mut out,
            );
            assert_eq!(out.title, "summaryengagement:hold fireidle");
            assert_eq!(out.body, "arrival");
            assert!(out.refused.is_empty());
        });
    }

    #[test]
    fn invalid_orders_missing_engagement_keeps_known_idle() {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 7,
                    mission: 2,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 8,
                    fleet: 10,
                    ..Default::default()
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![
                TaskForceCounterInputRow {
                    task_force: 7,
                    engagement_rule: -1,
                    ..Default::default()
                },
                TaskForceCounterInputRow {
                    task_force: 8,
                    ..Default::default()
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::FleetCounterInputs(vec![FleetCounterInputRow {
                fleet: 10,
                mission_region_count: -1,
                mission_regions: (None).into(),
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let mut out = Tooltip::default();
            invalid_orders(
                context,
                &hover(Control::InvalidOrders, &[7, 8]),
                || Ok(&[100_000]),
                &mut out,
            );
            assert_eq!(out.title, "idle");
            assert_eq!(out.refused.len(), 1);
            assert!(
                out.refused
                    .keys()
                    .next()
                    .unwrap()
                    .contains("engagement rule -1")
            );
        });
    }

    #[test]
    fn supply_uses_roster_friendship_and_coarse_never_demands_the_packet() {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 1,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 2,
                    canonical_slot: 2,
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                roster_owner: 2,
                logical_owner: 1,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let mut out = Tooltip::default();
            append(context, &hover(Control::Supply, &[7]), &mut out);
            assert!(out.refused.is_empty());
            let mut input = hover(Control::Supply, &[999]);
            input.coarse = true;
            append(context, &input, &mut out);
            assert!(out.refused.is_empty());
        });
    }

    #[test]
    fn pride_resolves_the_roster_alias_and_dangling_ship_falls_through_to_normal() {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 5,
                canonical_slot: 1,
            }]),
        );
        test_put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 7,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 9,
                roster_owner: 5,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 7,
                pride_of_the_fleet: 11,
                ..Default::default()
            }]),
        );
        test_put(&mut frame, TableRows::Ships(Vec::new()));
        with_context(&frame, |context| {
            assert!(specific(context, &hover(Control::Pride, &[9])).is_none());
        });
        test_put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: 11,
                task_force: 9,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let out = specific(context, &hover(Control::Pride, &[9])).unwrap();
            assert!(out.title.is_empty());
            assert_eq!(out.refused.len(), 1);
            assert!(
                out.refused
                    .keys()
                    .next()
                    .unwrap()
                    .contains("CountryCounterInputs.pride_name")
            );
        });
        test_put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    forward_visibility: replay_data::rows::Recorded::NotRecorded,
                    country: 7,
                    pride_of_the_fleet: 11,
                    pride_name: Some(b"ship eleven".to_vec()).into(),
                    ..Default::default()
                },
                CountryCounterInputRow {
                    forward_visibility: replay_data::rows::Recorded::NotRecorded,
                    country: 12,
                    pride_of_the_fleet: 99,
                    pride_name: Some(b"unrelated ship".to_vec()).into(),
                    ..Default::default()
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                task_force: 9,
                name: Some(b"task nine".to_vec()).into(),
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let out = specific(context, &hover(Control::Pride, &[9])).unwrap();
            assert_eq!(out.title, "task nine: ship eleven");
            assert!(out.refused.is_empty());
        });
        test_put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: (1_u64 << 32) | 11,
                task_force: 9,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            assert!(specific(context, &hover(Control::Pride, &[9])).is_none());
        });
    }

    #[test]
    fn empty_carrier_roster_does_not_require_model_or_air_registration_tables() {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                ..Default::default()
            }]),
        );
        test_put(&mut frame, TableRows::Ships(Vec::new()));
        with_context(&frame, |context| {
            let out = specific(context, &hover(Control::Carrier, &[7])).unwrap();
            assert!(out.title.is_empty());
            assert!(out.body.is_empty());
            assert!(out.refused.is_empty());
        });
    }

    #[test]
    fn pride_cached_names_keep_empty_refused_and_language_mismatch_distinct() {
        for (ship_name, task_name, recorded, expected) in [
            (
                Recorded::Value(Some(Vec::new())),
                Vec::new(),
                Some("other"),
                Ok(": "),
            ),
            (
                Recorded::Value(Some(b"ship".to_vec())),
                b"task".to_vec(),
                Some("other"),
                Err("stored in other"),
            ),
            (
                Recorded::Value(Some(Vec::new())),
                b"task".to_vec(),
                Some("other"),
                Err("task force cached name"),
            ),
            (
                Recorded::Value(None),
                b"task".to_vec(),
                None,
                Err("unreadable when recorded"),
            ),
            (
                Recorded::NotRecorded,
                b"task".to_vec(),
                None,
                Err("CountryCounterInputs.pride_name"),
            ),
            (
                Recorded::Value(Some(vec![0xff])),
                b"task".to_vec(),
                None,
                Err("not UTF-8"),
            ),
        ] {
            let mut frame = test_frame(0);
            test_put(
                &mut frame,
                TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                    task_force: 9,
                    name: Some(task_name).into(),
                    ..Default::default()
                }]),
            );
            let country = CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 7,
                pride_of_the_fleet: 11,
                pride_name: ship_name,
                ..Default::default()
            };
            with_language(&frame, recorded, |context| {
                let actual = pride_name(context, &country, 9);
                match expected {
                    Ok(text) => assert_eq!(actual.unwrap(), text),
                    Err(reason) => assert!(actual.unwrap_err().contains(reason)),
                }
            });
        }
    }

    fn supply_frame(first: Recorded<Option<UnitSupplyInputs>>) -> DecodedFrame {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 5,
                canonical_slot: 1,
            }]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 7,
                    roster_owner: 5,
                    logical_owner: 99,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 8,
                    roster_owner: 5,
                    logical_owner: 99,
                    ..Default::default()
                },
            ]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![
                TaskForceCounterInputRow {
                    task_force: 7,
                    received_supply_ratio: 100_000,
                    supply: first,
                    ..Default::default()
                },
                TaskForceCounterInputRow {
                    task_force: 8,
                    received_supply_ratio: 100_000,
                    supply: Some(UnitSupplyInputs {
                        asked: 300_000,
                        received: 175_000,
                        disruption: 1_501,
                        ship_demand: UnitSupplyDemand {
                            count: 3,
                            value: 500_000,
                        },
                        origins: Some(Vec::new()),
                        ..Default::default()
                    })
                    .into(),
                    ..Default::default()
                },
            ]),
        );
        frame
    }

    #[test]
    fn task_force_supply_joins_packets_in_item_order_and_preserves_arrival() {
        let frame = supply_frame(
            Some(UnitSupplyInputs {
                asked: 100_000,
                received: 25_000,
                disruption: 5_000,
                ship_demand: UnitSupplyDemand {
                    count: 2,
                    value: 200_000,
                },
                origins: Some(Vec::new()),
                ..Default::default()
            })
            .into(),
        );
        with_context(&frame, |context| {
            for (members, last_disruption) in [
                (&[7, 8][..], "0.02"),
                (&[8, 7][..], "0.05"),
                (&[7, 999, 8][..], "0.02"),
            ] {
                let mut out = Tooltip {
                    title: "summary".into(),
                    body: "arrival".into(),
                    ..Default::default()
                };
                append(context, &hover(Control::Supply, members), &mut out);
                assert_eq!(
                    out.title,
                    format!(
                        "summary\n\nneeded 4.00\n  ships 5 7.00\n\nreceived 2.00 \u{11}R50%\u{11}!\n  disruption {last_disruption}\n"
                    )
                );
                assert_eq!(out.body, "arrival");
                assert!(out.refused.is_empty(), "{:?}", out.refused);
            }
        });
    }

    #[test]
    fn task_force_supply_packet_failure_keeps_only_the_known_last_disruption() {
        for first in [Recorded::NotRecorded, Recorded::Value(None)] {
            let frame = supply_frame(first);
            with_context(&frame, |context| {
                let mut out = Tooltip {
                    title: "summary".into(),
                    body: "arrival".into(),
                    ..Default::default()
                };
                append(context, &hover(Control::Supply, &[7, 8]), &mut out);
                assert_eq!(out.title, "summary\n\n  disruption 0.02\n");
                assert_eq!(out.body, "arrival");
                assert_eq!(out.refused.values().sum::<usize>(), 1);
                assert!(
                    out.refused
                        .keys()
                        .next()
                        .unwrap()
                        .contains("TaskForceCounterInputs.supply")
                );
                let mut out = Tooltip::default();
                append(context, &hover(Control::Supply, &[8, 7]), &mut out);
                assert!(out.title.is_empty() && out.body.is_empty());
                assert_eq!(out.refused.values().sum::<usize>(), 1);
            });
        }
    }
}
