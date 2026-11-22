
use super::*;
#[cfg(test)]
use viewer_foundation::tooltip_text::NoIcons;
use viewer_unit_reads::{counter_action, railway_gun_counter};
use viewer_native_gui::counter_hovers::land::CounterControl;

pub(super) fn specific(context: &Context<'_>, input: &CounterHover) -> Option<Tooltip> {
    if input.coarse {
        return None;
    }
    let mut out = Tooltip::default();
    match input.control {
        CounterControl::Ability { country, ordinal } => {
            super::ability::append(context, input, country, ordinal, &mut out);
        }
        CounterControl::DefendingArea => super::append(
            &mut out,
            context,
            false,
            "defending area",
            localise(context.loc, "UNITS_DEFENDING_AREA", &[]),
        ),
        CounterControl::Action => {
            super::append(&mut out, context, false, "actions", actions(context, input))
        }
        CounterControl::Attrition if input.ours => {
            if input.members.len() == 1 {
                super::attrition::append(context, input.members[0], &mut out);
            } else {
                super::append(
                    &mut out,
                    context,
                    false,
                    "attrition",
                    stack_attrition(context, input),
                );
            }
        }
        CounterControl::DigIn => {
            super::append(&mut out, context, false, "dig in", dig_in(context, input))
        }
        CounterControl::NoOrders => no_orders(context, input, &mut out),
        CounterControl::LeaderEnroute if !input.railway_gun => {
            match leader_enroute_eligible(context, input) {
                Ok(false) => return None,
                Ok(true) => super::append(
                    &mut out,
                    context,
                    false,
                    "leader withdrawing",
                    leader_enroute(context, input),
                ),
                Err(reason) => {
                    refuse(&mut out, "leader withdrawing", reason);
                    if input.authored.is_some() {
                        refuse(
                            &mut out,
                            "authored tooltip dispatch",
                            "leader withdrawing operands are unavailable, so the native handled result is unknown",
                        );
                    }
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

fn dig_in(context: &Context<'_>, input: &CounterHover) -> Result<String, String> {
    let define = || {
        context
            .dig_in_factor
            .as_ref()
            .copied()
            .map_err(Clone::clone)
    };
    let factor = |status: &DivisionStatusRow| {
        status
            .dig_in_factor
            .require("DivisionStatus.dig_in_factor")?
            .ok_or_else(|| "DivisionStatus.dig_in_factor was unreadable when recorded".to_owned())
    };
    if input.members.len() == 1 {
        let status = context.status(input.members[0])?;
        let factor = factor(status)?;
        let define = define()?;
        let bonus =
            |value: i64| (value.wrapping_mul(define) / 100_000).wrapping_mul(factor) / 100_000;
        return localise(
            context.loc,
            "DIG_IN_UNIT",
            &[
                ("DIG", Arg::Fixed5(bonus(status.entrenchment))),
                ("MAX", Arg::Fixed5(bonus(status.entrenchment_cap))),
            ],
        );
    }
    let divisions = context
        .frame
        .complete::<tables::Divisions>()
        .map_err(|error| error.to_string())?;
    let mut values = Vec::with_capacity(input.members.len());
    for &id in input.members.iter() {
        if divisions.binary_search_by_key(&id, |row| row.id).is_err() {
            values.push(0);
            continue;
        }
        let status = context.status(id)?;
        values.push(if status.entrenchment > 0 {
            (status.entrenchment.wrapping_mul(factor(status)?) / 100_000).wrapping_mul(define()?)
                / 100_000
        } else {
            0
        });
    }
    let value = counter_inputs::member_mean(values).unwrap_or(0xffff_ffff);
    localise(context.loc, "DIG_IN_AVG", &[("DIG", Arg::Fixed5(value))])
}

fn leader_enroute_eligible(context: &Context<'_>, input: &CounterHover) -> Result<bool, String> {
    let Some(&id) = input.members.first() else {
        return Ok(false);
    };
    let icons = context
        .frame
        .complete::<tables::ArmyIconInputs>()
        .map_err(|error| error.to_string())?;
    let icon = icons
        .binary_search_by_key(&id, |row| row.division)
        .map(|index| &icons[index])
        .map_err(|_| format!("ArmyIconInputs division {id} is absent"))?;
    Ok(icon.separate_counter != 0)
}

fn leader_enroute(context: &Context<'_>, input: &CounterHover) -> Result<String, String> {
    let id = *input
        .members
        .first()
        .ok_or("leader withdrawing counter has no member")?;
    let group = context.division(id)?.orders_group;
    let group = context.orders_group(group)?;
    let operands = context.group_input(group.handle)?;
    let days = operands
        .leader_enroute_days
        .require("OrdersGroupCounterInputs.leader_enroute_days")?
        .ok_or("OrdersGroupCounterInputs.leader_enroute_days was unreadable when recorded")?;
    let name = context.leader_name(group.leader)?;
    localise(
        context.loc,
        if days == 1 {
            "LEADER_WITHDRAWING_HEADER_SINGULAR"
        } else {
            "LEADER_WITHDRAWING_HEADER"
        },
        &[("NAME", Arg::Text(&name)), ("DAYS", Arg::Int32(days))],
    )
}

fn no_orders(context: &Context<'_>, input: &CounterHover, out: &mut Tooltip) {
    if input.railway_gun {
        super::append(
            out,
            context,
            false,
            "railway gun location warning",
            railway_warning(context, input),
        );
        return;
    }
    let mut border = false;
    let mut missing = false;
    let mut border_unknown = None;
    let mut orders_unknown = None;
    let divisions = context.frame.complete::<tables::Divisions>();
    for &id in input.members.iter() {
        if divisions
            .as_ref()
            .is_ok_and(|rows| rows.binary_search_by_key(&id, |row| row.id).is_err())
        {
            refuse(
                out,
                "no-orders native callback",
                format!("unresolved full reference {id} reaches the native invalid-pointer path"),
            );
            return;
        }
        let result: Result<Option<bool>, String> = (|| {
            let group = context.division(id)?.orders_group;
            if group == 0 {
                return Ok(None);
            }
            let present = context
                .group_input(group)?
                .border_war_presence
                .require("OrdersGroupCounterInputs.border_war_presence")?
                .ok_or(
                    "OrdersGroupCounterInputs.border_war_presence was unreadable when recorded",
                )?;
            Ok(Some(present != 0))
        })();
        match result {
            Ok(Some(true)) => border = true,
            Ok(Some(false)) => match context.status(id) {
                Ok(status) => missing |= status.has_order == 0,
                Err(reason) => orders_unknown = Some(reason),
            },
            Ok(None) => (),
            Err(reason) => {
                border_unknown = Some(reason.clone());
                orders_unknown = Some(reason);
            }
        }
    }
    for (shown, unknown, key, line) in [
        (
            border,
            border_unknown,
            "SOME_UNITS_IN_BORDER_WAR",
            "border-war orders warning",
        ),
        (
            missing,
            orders_unknown,
            "UNITS_HAS_NO_ORDERS_WARNING",
            "orders warning",
        ),
    ] {
        if shown {
            super::append(out, context, false, line, localise(context.loc, key, &[]));
        } else if let Some(reason) = unknown {
            refuse(out, line, reason);
        }
    }
}

fn railway_warning(context: &Context<'_>, input: &CounterHover) -> Result<String, String> {
    let rows = context
        .frame
        .complete::<tables::RailwayGunCounterInputs>()
        .map_err(|error| error.to_string())?;
    let mut unknown = None;
    for &id in input.members.iter() {
        let Ok(index) = rows.binary_search_by_key(&id, |row| row.gun) else {
            continue;
        };
        let row = &rows[index];
        let warning: Result<bool, String> = (|| {
            if !row.ordinary_path.is_empty() || row.naval_transfer != 0 || row.exiled != 0 {
                return Ok(false);
            }
            let land = context
                .content
                .ok_or("matching province topology is unavailable")?
                .province_is_land(row.province)
                .ok_or_else(|| {
                    format!("province {} is absent from matching content", row.province)
                })?;
            if let Some(result) = railway_gun_counter::location_warning(row, land, None) {
                return Ok(result);
            }
            let cooldowns = context
                .frame
                .complete::<tables::RailCooldowns>()
                .map_err(|error| error.to_string())?;
            let rail = cooldowns
                .binary_search_by_key(&row.province, |row| row.province)
                .map(|index| &cooldowns[index])
                .map_err(|_| format!("RailCooldowns province {} is absent", row.province))?;
            railway_gun_counter::location_warning(
                row,
                land,
                Some(railway_gun_counter::RailStatus::read(
                    rail,
                    context.frame.hour,
                )?),
            )
            .ok_or_else(|| "railway gun location eligibility is unavailable".to_owned())
        })();
        match warning {
            Ok(true) => {
                return localise(
                    context.loc,
                    "ARMY_RAILWAY_GUN_INVALID_LOCATION_NO_RAILWAYS",
                    &[],
                )
                .map(|text| format!("\n{text}"));
            }
            Ok(false) => (),
            Err(reason) => unknown = Some(reason),
        }
    }
    unknown.map_or_else(|| Ok(String::new()), Err)
}

fn stack_attrition(context: &Context<'_>, input: &CounterHover) -> Result<String, String> {
    let divisions = context
        .frame
        .complete::<tables::Divisions>()
        .map_err(|error| error.to_string())?;
    let mut count = 0_i32;
    let mut maximum = 0_i64;
    for &id in input.members.iter() {
        if divisions.binary_search_by_key(&id, |row| row.id).is_err() {
            continue;
        }
        let status = context.status(id)?;
        if status.attrition_value > status.attrition_threshold {
            count = count.wrapping_add(1);
            maximum = maximum.max(status.attrition_value);
        }
    }
    localise(
        context.loc,
        "ATTRITION_SOME_UNITS",
        &[("COUNT", Arg::Int32(count)), ("MAX", Arg::Fixed5(maximum))],
    )
}

fn actions(context: &Context<'_>, input: &CounterHover) -> Result<String, String> {
    let links = context
        .links
        .as_ref()
        .map_err(Clone::clone)?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect::<BTreeMap<_, _>>();
    let hostility = counter_action::DestinationHostility::read(context.frame, &links)
        .map_err(|e| e.to_string())?;
    let mut counts = [0_i32; 13];
    if input.railway_gun {
        let rows = context
            .frame
            .complete::<tables::RailwayGunCounterInputs>()
            .map_err(|e| e.to_string())?;
        for &id in input.members.iter() {
            let Ok(index) = rows.binary_search_by_key(&id, |row| row.gun) else {
                continue;
            };
            let row = &rows[index];
            let enemy = hostility.destination_enemy(
                &row.ordinary_path,
                row.ordinary_path.len() as u32,
                Some(row.logical_owner),
            );
            let code = railway_gun_counter::action_code(row, enemy).map_err(str::to_owned)?;
            counts[usize::from(code)] = counts[usize::from(code)].wrapping_add(1);
        }
    } else {
        let divisions = context
            .frame
            .complete::<tables::Divisions>()
            .map_err(|error| error.to_string())?;
        let actions = counter_action::Actions::read(context.frame, &hostility, None)
            .map_err(|e| e.to_string())?;
        for &id in input.members.iter() {
            if divisions.binary_search_by_key(&id, |row| row.id).is_err() {
                continue;
            }
            let code = actions.get(id).map_err(str::to_owned)?;
            counts[usize::from(code)] = counts[usize::from(code)].wrapping_add(1);
        }
    }
    action_text(context.loc, &counts)
}

fn action_text(loc: &Localisation, counts: &[i32; 13]) -> Result<String, String> {
    const KEYS: [&str; 12] = [
        "ACTION_UNIT_STANDING_STILL",
        "ACTION_UNIT_DEFENDING",
        "ACTION_UNIT_SUPPORT",
        "ACTION_UNIT_MOVE",
        "ACTION_UNIT_ATTACK",
        "ACTION_UNIT_STRATEGIC",
        "ACTION_UNIT_NAVAL_TRANSFER",
        "ACTION_UNIT_INVASION",
        "ACTION_UNIT_NAVAL_TRANSFER_INTERCEPTED",
        "ACTION_UNIT_RETREAT",
        "ACTION_UNIT_STRATEGIC_DISABLED",
        "ACTION_UNIT_SUPPORT_BOMBARDMENT",
    ];
    let mut text = String::new();
    let mut groups = 0;
    for (code, &count) in counts.iter().enumerate() {
        if count > 0 {
            groups += 1;
            if code != 0 {
                text.push_str(&localise(
                    loc,
                    KEYS[code - 1],
                    &[("COUNT", Arg::Int32(count))],
                )?);
            }
            text.push('\n');
        }
    }
    if !text.is_empty() && groups > 1 {
        text.insert_str(
            0,
            &format!("{}\n", localise(loc, "ACTIONS_MIXED_HEADER", &[])?),
        );
    }
    Ok(text)
}

pub(super) fn append(context: &Context<'_>, input: &CounterHover, out: &mut Tooltip) {
    if input.coarse {
        return;
    }
    match input.control {
        CounterControl::InvalidOrders if !input.railway_gun => {
            let warning = invalid_orders(context, input);
            super::append(out, context, false, "invalid orders", warning);
        }
        CounterControl::Supply => {
            let reached = (|| {
                let first = *input
                    .members
                    .first()
                    .ok_or("supply counter has no members")?;
                let owner = if input.railway_gun {
                    let rows = context
                        .frame
                        .complete::<tables::RailwayGunCounterInputs>()
                        .map_err(|e| e.to_string())?;
                    rows.binary_search_by_key(&first, |row| row.gun)
                        .map(|i| rows[i].roster_owner)
                        .map_err(|_| format!("RailwayGunCounterInputs gun {first} is absent"))?
                } else {
                    context.division(first)?.roster_owner
                };
                context
                    .vision
                    .shares_vision_proven(context.slot(owner)?)
                    .map_err(|e| e.describe())
            })();
            match reached {
                Ok(false) => (),
                Ok(true) => super::supply::append(context, input, out),
                Err(reason) => refuse(out, "supply detail", reason),
            }
        }
        _ => (),
    }
}

fn invalid_orders(context: &Context<'_>, input: &CounterHover) -> Result<String, String> {
    let divisions = context
        .frame
        .complete::<tables::Divisions>()
        .map_err(|error| error.to_string())?;
    for &id in input.members.iter() {
        if divisions.binary_search_by_key(&id, |row| row.id).is_err() {
            continue;
        }
        let status = context.status(id)?;
        if status.invalid_orders == 0 {
            continue;
        }
        let raw = status
            .invalid_orders_reason
            .require("DivisionStatus.invalid_orders_reason")?
            .ok_or("DivisionStatus.invalid_orders_reason was unreadable when recorded")?;
        let key = match raw {
            1 => "UNITS_HAS_INVALID_MOVE_IN_PATH_WARNING",
            2 => "UNITS_INVALID_ATTACK_WARNING",
            3 => "UNITS_HAS_LOW_SUPPLY_MOVE_WARNING",
            4 => "UNITS_HAS_INVALID_TRANSPORT_WARNING",
            5 => "UNITS_HAS_INVALID_PATH_WARNING",
            6 => "UNITS_HAS_LOST_ITS_WAY",
            7 => "UNITS_TOO_WEAK_TO_ATTACK",
            _ => return Ok("\n".into()),
        };
        return localise(context.loc, key, &[]).map(|text| format!("\n{text}"));
    }
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(
        frame: &'a DecodedFrame,
        loc: &'a Localisation,
        vision: &'a Vision,
        names: &'a content_reader_division_names::division_names::Database,
        units: &'a Result<units::Content, String>,
    ) -> Context<'a> {
        Context {
            nato: false,
            diagnostics: true,
            content: None,
            reads: None,
            dig_in_factor: Err("unused".into()),
            loc,
            recorded_language: &loc.language,
            icons: &NoIcons,
            names,
            units,
            supply_grace: Err("unused".into()),
            comms_max_distance: Err("unused".into()),
            frame,
            vision,
            links: Ok(&[]),
        }
    }

    #[test]
    fn invalid_warning_stops_at_first_demand_and_preserves_unknown_codes() {
        use replay_data::{rows::Recorded, rows::TableId, rows::TableRows};
        use replay_frame::frame::TableView;
        let loc = [("UNITS_TOO_WEAK_TO_ATTACK".into(), "weak".into())]
            .into_iter()
            .collect::<Localisation>();
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 0,
            metrics: Default::default(),
            tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
        };
        *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
            [1, 2, 3]
                .into_iter()
                .map(|id| DivisionRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ));
        let vision = Vision::sharing(1, []);
        let names = content_reader_division_names::division_names::Database::default();
        let units = Err("unused".into());
        let hover = CounterHover {
            control: CounterControl::InvalidOrders,
            authored: None,
            rect: [0.0; 4],
            remaining: None,
            coarse: false,
            hidden: false,
            railway_gun: false,
            ours: false,
            members: vec![1, (1_u64 << 32) | 1, 2, 3].into(),
            timing: Arc::from([]),
        };
        for (reason, expected) in [(7, "\nweak"), (i32::MIN, "\n"), (0, "\n")] {
            *frame.tables[TableId::DivisionStatus.index()] =
                TableView::Complete(TableRows::DivisionStatus(vec![
                    DivisionStatusRow {
                        division: 1,
                        invalid_orders: 0,
                        ..Default::default()
                    },
                    DivisionStatusRow {
                        division: 2,
                        invalid_orders: 1,
                        invalid_orders_reason: Recorded::Value(Some(reason)),
                        ..Default::default()
                    },
                    DivisionStatusRow {
                        division: 3,
                        invalid_orders: 1,
                        ..Default::default()
                    },
                ]));
            let context = context(&frame, &loc, &vision, &names, &units);
            assert_eq!(invalid_orders(&context, &hover).unwrap(), expected);
        }
        *frame.tables[TableId::Divisions.index()] = TableView::Unavailable {
            reason: "fixture incomplete division census".into(),
        };
        let context = context(&frame, &loc, &vision, &names, &units);
        assert!(
            invalid_orders(&context, &hover)
                .unwrap_err()
                .contains("fixture incomplete division census")
        );
    }

    #[test]
    fn action_descriptions_keep_bucket_order_zero_and_mixed_header() {
        let loc = [
            ("ACTION_UNIT_ATTACK".into(), "attack $COUNT$".into()),
            ("ACTION_UNIT_RETREAT".into(), "retreat $COUNT$".into()),
            ("ACTIONS_MIXED_HEADER".into(), "mixed".into()),
        ]
        .into_iter()
        .collect();
        let mut counts = [0; 13];
        counts[10] = 3;
        counts[5] = 2;
        assert_eq!(
            action_text(&loc, &counts).unwrap(),
            "mixed\nattack 2\nretreat 3\n"
        );
        counts.fill(0);
        counts[0] = 1;
        assert_eq!(action_text(&loc, &counts).unwrap(), "\n");
    }
    #[test]
    fn dig_in_keeps_singleton_rounding_and_all_member_mean() {
        use replay_data::{rows::Recorded, rows::TableRows};
        use viewer_foundation::test_support::{test_frame, test_put as put};
        let loc: Localisation = [
            ("DIG_IN_UNIT".into(), "single $DIG|5$/$MAX|5$".into()),
            ("DIG_IN_AVG".into(), "mean $DIG|5$".into()),
        ]
        .into_iter()
        .collect();
        let mut frame = test_frame(0);
        put(
            &mut frame,
            TableRows::Divisions(
                [1, 2, 3]
                    .into_iter()
                    .map(|id| DivisionRow {
                        id,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::DivisionStatus(vec![
                DivisionStatusRow {
                    division: 1,
                    entrenchment: 123_457,
                    entrenchment_cap: 200_000,
                    dig_in_factor: Recorded::Value(Some(1_357_919)),
                    ..Default::default()
                },
                DivisionStatusRow {
                    division: 2,
                    entrenchment: 0,
                    ..Default::default()
                },
                DivisionStatusRow {
                    division: 3,
                    entrenchment: 0,
                    ..Default::default()
                },
            ]),
        );
        let vision = Vision::sharing(1, []);
        let names = content_reader_division_names::division_names::Database::default();
        let units = Err("unused".into());
        let context = Context {
            dig_in_factor: Ok(876_543),
            ..context(&frame, &loc, &vision, &names, &units)
        };
        let mut hover = CounterHover {
            control: CounterControl::DigIn,
            authored: None,
            rect: [0.0; 4],
            remaining: None,
            coarse: false,
            hidden: false,
            railway_gun: false,
            ours: false,
            members: vec![1].into(),
            timing: Arc::from([]),
        };
        assert_eq!(
            dig_in(&context, &hover).unwrap(),
            "single 146.94761/238.05487"
        );
        hover.members = vec![1, 2].into();
        assert_eq!(dig_in(&context, &hover).unwrap(), "mean 73.47385");
        hover.members = vec![1, (1_u64 << 32) | 1, 2].into();
        assert_eq!(dig_in(&context, &hover).unwrap(), "mean 48.98256");
        hover.members = vec![2].into();
        assert!(dig_in(&context, &hover).is_err());
        hover.members = vec![2, 3].into();
        let context = Context {
            dig_in_factor: Err("unused".into()),
            ..context
        };
        assert_eq!(dig_in(&context, &hover).unwrap(), "mean 0.00000");
    }

    #[test]
    fn early_attrition_actions_and_railway_warning_skip_only_proven_dangling_members() {
        use replay_data::{rows::RailwayGunCounterInputRow, rows::TableId, rows::TableRows};
        use viewer_foundation::test_support::{test_frame, test_put as put};
        let loc: Localisation = [
            (
                "ATTRITION_SOME_UNITS".into(),
                "$COUNT$ attrition $MAX|%0$".into(),
            ),
            (
                "ACTION_UNIT_STANDING_STILL".into(),
                "$COUNT$ standing".into(),
            ),
        ]
        .into_iter()
        .collect();
        let mut frame = test_frame(0);
        put(
            &mut frame,
            TableRows::Divisions(
                [1, 2]
                    .into_iter()
                    .map(|id| DivisionRow {
                        id,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::DivisionStatus(
                [1, 2]
                    .into_iter()
                    .map(|division| DivisionStatusRow {
                        division,
                        attrition_value: if division == 1 { 60_000 } else { 0 },
                        attrition_threshold: 50_000,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(
                [1, 2]
                    .into_iter()
                    .map(|gun| RailwayGunCounterInputRow {
                        gun,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let names = content_reader_division_names::division_names::Database::default();
        let units = Err("unused".into());
        let vision = Vision::sharing(1, []);
        let input = CounterHover {
            control: CounterControl::Attrition,
            authored: None,
            rect: [0.0; 4],
            remaining: None,
            ours: true,
            coarse: false,
            hidden: false,
            railway_gun: false,
            members: vec![1, (1_u64 << 32) | 1, 2].into(),
            timing: Arc::from([]),
        };
        let render = |frame: &DecodedFrame, control| {
            let context = context(frame, &loc, &vision, &names, &units);
            let input = CounterHover {
                railway_gun: control != CounterControl::Attrition,
                control,
                ..input.clone()
            };
            match control {
                CounterControl::Attrition => stack_attrition(&context, &input),
                CounterControl::Action => actions(&context, &input),
                _ => railway_warning(&context, &input),
            }
        };
        assert_eq!(
            render(&frame, CounterControl::Attrition).unwrap(),
            "1 attrition 60%"
        );
        assert_eq!(
            render(&frame, CounterControl::Action).unwrap(),
            "2 standing\n"
        );
        let mut guns = frame
            .complete::<tables::RailwayGunCounterInputs>()
            .unwrap()
            .to_vec();
        for gun in &mut guns {
            gun.naval_transfer = 1;
        }
        put(&mut frame, TableRows::RailwayGunCounterInputs(guns));
        assert_eq!(render(&frame, CounterControl::NoOrders).unwrap(), "");
        *frame.tables[TableId::RailwayGunCounterInputs.index()] =
            replay_frame::frame::TableView::Unavailable {
                reason: "fixture incomplete gun census".into(),
            };
        assert!(
            render(&frame, CounterControl::NoOrders)
                .unwrap_err()
                .contains("fixture incomplete gun census")
        );
    }

    #[test]
    fn no_orders_keeps_independent_border_and_missing_order_fragments() {
        use replay_data::{rows::OrdersGroupCounterInputRow, rows::Recorded, rows::TableRows};
        use viewer_foundation::test_support::{test_frame, test_put as put};
        let loc: Localisation = [
            ("SOME_UNITS_IN_BORDER_WAR".into(), "border".into()),
            ("UNITS_HAS_NO_ORDERS_WARNING".into(), "orders".into()),
        ]
        .into_iter()
        .collect();
        let mut frame = test_frame(0);
        put(
            &mut frame,
            TableRows::Divisions(
                (1..=4)
                    .map(|id| DivisionRow {
                        id,
                        orders_group: if id == 4 { 0 } else { id + 10 },
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::DivisionStatus(vec![
                DivisionStatusRow {
                    division: 1,
                    has_order: 0,
                    ..Default::default()
                },
                DivisionStatusRow {
                    division: 2,
                    has_order: 0,
                    ..Default::default()
                },
                DivisionStatusRow {
                    division: 4,
                    has_order: 0,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::OrdersGroupCounterInputs(vec![
                OrdersGroupCounterInputRow {
                    group: 11,
                    border_war_presence: Recorded::Value(Some(1)),
                    ..Default::default()
                },
                OrdersGroupCounterInputRow {
                    group: 12,
                    border_war_presence: Recorded::Value(Some(0)),
                    ..Default::default()
                },
                OrdersGroupCounterInputRow {
                    group: 13,
                    ..Default::default()
                },
            ]),
        );
        let vision = Vision::sharing(1, []);
        let names = content_reader_division_names::division_names::Database::default();
        let units = Err("unused".into());
        let context = context(&frame, &loc, &vision, &names, &units);
        let mut hover = CounterHover {
            control: CounterControl::NoOrders,
            authored: None,
            rect: [0.0; 4],
            remaining: None,
            coarse: false,
            hidden: false,
            railway_gun: false,
            ours: false,
            members: vec![1, 2, 3, 4].into(),
            timing: Arc::from([]),
        };
        let mut out = Tooltip::default();
        no_orders(&context, &hover, &mut out);
        assert_eq!(out.title, "borderorders");
        assert!(out.refused.is_empty());
        let dangling = (1_u64 << 32) | 1;
        for members in [vec![1, dangling, 2], vec![1, 2, dangling]] {
            hover.members = members.into();
            let mut out = Tooltip::default();
            no_orders(&context, &hover, &mut out);
            assert!(out.title.is_empty());
            assert_eq!(out.refused.values().sum::<usize>(), 1);
            assert!(out.refused.keys().any(|reason| {
                reason.starts_with("counter no-orders native callback:")
                    && reason.contains("native invalid-pointer path")
            }));
        }
        hover.members = vec![1, 3, 4].into();
        let mut out = Tooltip::default();
        no_orders(&context, &hover, &mut out);
        assert_eq!(out.title, "border");
        assert_eq!(out.refused.values().sum::<usize>(), 1);
        hover.members = vec![4].into();
        let mut out = Tooltip::default();
        no_orders(&context, &hover, &mut out);
        assert!(out.title.is_empty() && out.refused.is_empty());
    }

    #[test]
    fn leader_enroute_uses_cooldown_days_and_cached_name_not_member_count() {
        use replay_data::{
            rows::ArmyIconInputRow, rows::OrdersGroupCounterInputRow, rows::OrdersGroupRow,
            rows::Recorded, rows::TableRows, rows::UnitLeaderRow,
        };
        use viewer_foundation::test_support::{test_frame, test_put as put};
        let mut loc: Localisation = [
            (
                "LEADER_WITHDRAWING_HEADER_SINGULAR".into(),
                "$NAME$ one $DAYS$".into(),
            ),
            (
                "LEADER_WITHDRAWING_HEADER".into(),
                "$NAME$ many $DAYS$".into(),
            ),
            ("UNIT_LEADER_TOOLTIP".into(), "$NAME$".into()),
            (
                "UNIT_LEADER_WITHDRAWING_SUFFIX_SINGULAR".into(),
                " one $DAYS$".into(),
            ),
            (
                "UNIT_LEADER_WITHDRAWING_SUFFIX".into(),
                " many $DAYS$".into(),
            ),
        ]
        .into_iter()
        .collect();
        loc.language = "english".into();
        let vision = Vision::sharing(1, []);
        let names = content_reader_division_names::division_names::Database::default();
        let units = Err("unused".into());
        let mut frame = test_frame(0);
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 1,
                orders_group: 11,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::OrdersGroups(vec![OrdersGroupRow {
                handle: 11,
                leader: 9 << 32 | 52,
                hq_provider: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::UnitLeaders(vec![UnitLeaderRow {
                id: 9,
                reference_type: 52,
                inherited_text_b: b"cached".to_vec(),
                ..Default::default()
            }]),
        );
        for (days, withdrawing, cached, expected, suffix) in [
            (
                Recorded::Value(Some(1)),
                1,
                1,
                Some("cached one 1"),
                "cached one 1",
            ),
            (
                Recorded::Value(Some(2)),
                1,
                1,
                Some("cached many 2"),
                "cached many 2",
            ),
            (
                Recorded::Value(Some(1)),
                0,
                1,
                Some("cached one 1"),
                "cached",
            ),
            (Recorded::NotRecorded, 1, 1, Some(""), "cached"),
            (Recorded::Value(Some(1)), 1, 0, None, "cached one 1"),
        ] {
            put(
                &mut frame,
                TableRows::ArmyIconInputs(vec![ArmyIconInputRow {
                    division: 1,
                    separate_counter: cached,
                    ..Default::default()
                }]),
            );
            put(
                &mut frame,
                TableRows::OrdersGroupCounterInputs(vec![OrdersGroupCounterInputRow {
                    group: 11,
                    withdrawing,
                    hq_distance: Recorded::Value(Some(0)),
                    leader_enroute_days: days,
                    ..Default::default()
                }]),
            );
            let context = Context {
                comms_max_distance: Ok(10),
                ..context(&frame, &loc, &vision, &names, &units)
            };
            let hover = CounterHover {
                control: CounterControl::LeaderEnroute,
                authored: None,
                rect: [0.0; 4],
                remaining: None,
                coarse: false,
                hidden: false,
                railway_gun: false,
                ours: false,
                members: vec![1, 2, 3].into(),
                timing: Arc::from([]),
            };
            let out = specific(&context, &hover);
            assert_eq!(out.as_ref().map(|out| out.title.as_str()), expected);
            if let Some(out) = out {
                assert_eq!(
                    out.refused.is_empty(),
                    !matches!(days, Recorded::NotRecorded)
                );
            }
            let mut out = Tooltip::default();
            context.hq(&mut out, &[1]);
            assert_eq!(out.title, suffix);
            assert_eq!(
                out.refused.is_empty(),
                !matches!(days, Recorded::NotRecorded)
            );
        }
    }
}
