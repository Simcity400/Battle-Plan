
use super::*;
use content_reader_script::sub_units::SubUnits;
use replay_data::{rows::ShipModelRow, rows::ShipRow, rows::TaskForceRow};
use viewer_unit_reads::counter_order;
use viewer_runtime::msvc_stable_sort::sort;
use viewer_unit_reads::naval_count_approx;

struct Member<'a> {
    force: &'a TaskForceRow,
    owner: u32,
    visibility: u16,
}

fn members<'a>(context: &Context<'a>, ids: &[u64]) -> Result<Vec<Member<'a>>, String> {
    let rows = context
        .frame
        .complete::<tables::TaskForces>()
        .map_err(|e| e.to_string())?;
    ids.iter()
        .filter_map(|&id| {
            rows.binary_search_by_key(&id, |row| row.id)
                .ok()
                .map(|index| (id, &rows[index]))
        })
        .map(|(id, force)| {
            let owner = context.slot(force.roster_owner)?;
            let visibility = context
                .vision
                .navy_byte_known(id, owner)
                .ok_or_else(|| format!("task force {id} naval intel is unavailable"))?;
            Ok(Member {
                force,
                owner,
                visibility,
            })
        })
        .collect()
}

fn line(context: &Context<'_>, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
    localise(context.loc, key, args).map(|text| format!("{text}\n"))
}

pub(super) fn append(context: &Context<'_>, ids: &[u64], coarse: bool, out: &mut Tooltip) {
    if ids.is_empty() {
        return;
    }
    let members = match members(context, ids) {
        Ok(members) => members,
        Err(reason) => {
            refuse(out, "task force summary admission", reason);
            return;
        }
    };
    if coarse {
        append_coarse(context, &members, out);
    } else {
        append_detail(context, &members, out);
    }
}

fn append_coarse(context: &Context<'_>, members: &[Member<'_>], out: &mut Tooltip) {
    let known = members
        .iter()
        .filter(|member| member.visibility >= 250)
        .collect::<Vec<_>>();
    if known.len() < members.len() {
        super::append(
            out,
            context,
            false,
            "coarse naval unknown",
            line(
                context,
                if known.is_empty() {
                    "COARSE_COUNTER_UNKNOWN_UNITS"
                } else {
                    "COARSE_COUNTER_KNOWN_UNITS"
                },
                &[],
            ),
        );
    }
    if known.is_empty() {
        return;
    }
    let text = (|| {
        let ships = context
            .frame
            .complete::<tables::Ships>()
            .map_err(|e| e.to_string())?;
        let count = known.iter().fold(0_i32, |total, member| {
            total.wrapping_add(
                ships
                    .iter()
                    .filter(|ship| ship.task_force == member.force.id)
                    .count() as i32,
            )
        });
        line(
            context,
            "COARSE_COUNTER_NAVIES_DESC",
            &[
                ("SHIPS", Arg::Int32(count)),
                ("NAVIES", Arg::Int32(known.len() as i32)),
            ],
        )
    })();
    super::append(out, context, false, "coarse naval counts", text);
}

fn name(context: &Context<'_>, id: u64) -> Result<String, String> {
    let rows = context
        .frame
        .complete::<tables::TaskForceCounterInputs>()
        .map_err(|e| e.to_string())?;
    let row = rows
        .binary_search_by_key(&id, |row| row.task_force)
        .map(|index| &rows[index])
        .map_err(|_| format!("TaskForceCounterInputs task force {id} is absent"))?;
    dashboard_text::unit_names::task_force_name(row, context.loc, context.recorded_language)
}

fn models<'a>(
    context: &Context<'a>,
    ships: &[&'a ShipRow],
) -> Result<Vec<(&'a ShipRow, &'a ShipModelRow)>, String> {
    if ships.is_empty() {
        return Ok(Vec::new());
    }
    let rows = context
        .frame
        .complete::<tables::ShipModels>()
        .map_err(|e| e.to_string())?;
    ships
        .iter()
        .map(|&ship| {
            let model = rows
                .binary_search_by_key(&ship.id, |row| row.ship)
                .map(|index| &rows[index])
                .map_err(|_| format!("ShipModels ship {} is absent", ship.id))?;
            if model.counter.is_none() {
                return Err(format!("ship {} counter operands are unrecorded", ship.id));
            }
            Ok((ship, model))
        })
        .collect()
}

fn header(
    context: &Context<'_>,
    member: &Member<'_>,
    total: i32,
    ships: &Result<Vec<(&ShipRow, &ShipModelRow)>, String>,
) -> Result<String, String> {
    let capitals = ships
        .as_ref()
        .map_err(Clone::clone)?
        .iter()
        .fold(0_i32, |count, (_, model)| {
            count.wrapping_add(i32::from(
                counter_order::is_capital(model).expect("counter operands admitted"),
            ))
        });
    let (count, capital_count) = if member.visibility >= 250 {
        (total.to_string(), capitals.to_string())
    } else {
        let defines = &context
            .content
            .ok_or("naval count content is unavailable")?
            .naval_counts;
        (
            naval_count_approx::format_count(
                total,
                member.force.id,
                member.visibility,
                false,
                total,
                defines,
            ),
            naval_count_approx::format_count(
                capitals,
                member.force.id,
                member.visibility,
                true,
                total,
                defines,
            ),
        )
    };
    let navy = name(context, member.force.id)?;
    let country = context.country_name(member.owner)?;
    line(
        context,
        "MAP_ICON_NAVY_SHIPS",
        &[
            ("NAVY", Arg::Text(&navy)),
            ("COUNTRY", Arg::Text(&country)),
            ("VALUE", Arg::Text(&count)),
            ("VALUE2", Arg::Text(&capital_count)),
        ],
    )
}

fn ship_type(sub_units: &SubUnits, model: &ShipModelRow) -> Result<(i32, String), String> {
    let index = model
        .subunit_index
        .require("ShipModels.subunit_index")?
        .ok_or_else(|| format!("ship {} copied sub-unit index is unreadable", model.ship))?;
    let name = if index == 0 {
        String::new()
    } else {
        sub_units
            .name(index as u32)
            .ok_or_else(|| format!("ship {} sub-unit index {index} has no name", model.ship))??
            .to_owned()
    };
    Ok((index, name))
}

fn type_database(sub_units: &SubUnits) -> Result<&SubUnits, String> {
    if sub_units.registered().any(|name| name.contains(&0)) {
        return Err("sub-unit registration with embedded-NUL keys is unsupported".into());
    }
    Ok(sub_units)
}

struct Group<'a> {
    hull: counter_order::Hull<'a>,
    name: String,
    lower: i32,
    upper: i32,
}

fn grouped<'a>(
    context: &Context<'a>,
    member: &Member<'_>,
    ships: &[(&'a ShipRow, &'a ShipModelRow)],
) -> Result<BTreeMap<i32, Group<'a>>, String> {
    if ships.is_empty() {
        return Ok(BTreeMap::new());
    }
    let content = context.content.ok_or("ship type content is unavailable")?;
    let sub_units = type_database(content.sub_units.as_deref().map_err(Clone::clone)?)?;
    let mut roster = ships.to_vec();
    roster.sort_by_key(|(_, model)| model.counter.as_ref().expect("admitted").roster_ordinal);
    let mut groups = BTreeMap::<i32, Group<'a>>::new();
    for (ship, model) in roster {
        let (index, name) = ship_type(sub_units, model)?;
        let group = groups.entry(index).or_insert_with(|| Group {
            hull: counter_order::Hull {
                strength: ship.strength,
                model,
            },
            name,
            lower: 0,
            upper: 0,
        });
        group.lower = group.lower.wrapping_add(1);
    }
    for group in groups.values_mut() {
        (group.lower, group.upper) = type_range(
            group.lower,
            member.visibility,
            member.force.id,
            group.hull.model.formation_flags,
            ships.len() as i32,
            &content.naval_counts,
        );
    }
    Ok(groups)
}

fn type_range(
    count: i32,
    visibility: u16,
    force: u64,
    flags: u64,
    total_ships: i32,
    defines: &naval_count_approx::Defines,
) -> (i32, i32) {
    if visibility >= 250 {
        return (count, count);
    }
    let minimum = if flags & counter_order::CAPITAL_FLAG != 0 {
        if total_ships > 3 {
            defines.medium_min_capitals
        } else {
            0
        }
    } else {
        defines.medium_min_ships
    };
    naval_count_approx::count_range(
        count,
        defines.medium_half_range_percent,
        minimum,
        (force >> 32) as u32,
    )
}

fn merge_groups<'a>(all: &mut BTreeMap<i32, Group<'a>>, groups: BTreeMap<i32, Group<'a>>) {
    for (index, next) in groups {
        if let Some(group) = all.get_mut(&index) {
            group.lower = group.lower.wrapping_add(next.lower);
            group.upper = group.upper.wrapping_add(next.upper);
        } else {
            all.insert(index, next);
        }
    }
}

fn composition(
    groups: &BTreeMap<i32, Group<'_>>,
    equipment: Result<&content_reader_key_names::equipment::Database, &str>,
) -> Result<String, String> {
    let mut ordered = groups.values().collect::<Vec<_>>();
    let mut missing = None;
    sort(&mut ordered, |a, b| {
        if missing.is_some() {
            return false;
        }
        match counter_order::better_hull(&a.hull, &b.hull, equipment) {
            Ok(less) => less,
            Err(reason) => {
                missing = Some(format!(
                    "ship types {} and {} comparison: {reason}",
                    a.name, b.name
                ));
                false
            }
        }
    });
    if let Some(reason) = missing {
        return Err(reason);
    }
    Ok(ordered
        .into_iter()
        .map(|group| {
            let count = if group.lower == group.upper {
                group.lower.to_string()
            } else {
                format!("{}-{}", group.lower, group.upper)
            };
            format!("{count} \u{13}ship_{}_icon_small|1    ", group.name)
        })
        .collect())
}

fn mean(values: Vec<Result<i64, String>>) -> Result<i64, String> {
    let values = values.into_iter().collect::<Result<Vec<_>, _>>()?;
    Ok(counter_inputs::member_mean(values).expect("exact members are nonempty"))
}

fn append_detail(context: &Context<'_>, members: &[Member<'_>], out: &mut Tooltip) {
    let unknown = members.iter().any(|member| member.visibility < 100);
    let visible = members
        .iter()
        .filter(|member| member.visibility >= 100)
        .collect::<Vec<_>>();
    if visible.is_empty() {
        if unknown {
            super::append(
                out,
                context,
                false,
                "unknown navy",
                localise(context.loc, "UNKNOWN_NAVY_DESC", &[]),
            );
        }
        return;
    }
    let ships = context
        .frame
        .complete::<tables::Ships>()
        .map_err(|e| e.to_string());
    let mut organisations = Vec::new();
    let mut strengths = Vec::new();
    let mut groups = BTreeMap::new();
    let mut composition_error = None;
    for member in visible {
        let roster = ships.as_ref().map_err(Clone::clone).map(|rows| {
            rows.iter()
                .filter(|ship| ship.task_force == member.force.id)
                .collect::<Vec<_>>()
        });
        let hulls = roster
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|ships| models(context, ships));
        let title = roster
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|ships| header(context, member, ships.len() as i32, &hulls));
        super::append(out, context, false, "naval task force header", title);
        if member.visibility >= 250 {
            organisations.push(hulls.as_ref().map_err(Clone::clone).map(|ships| {
                counter_inputs::organisation_ratio(ships.iter().map(|(ship, model)| {
                    (
                        ship.organisation,
                        model
                            .counter
                            .as_ref()
                            .expect("admitted")
                            .maximum_organisation,
                    )
                }))
            }));
            strengths.push(hulls.as_ref().map_err(Clone::clone).map(|ships| {
                counter_inputs::strength_ratio(ships.iter().map(|(ship, model)| {
                    (
                        ship.strength,
                        model.counter.as_ref().expect("admitted").maximum_strength,
                    )
                }))
            }));
        }
        if member.visibility >= 150 {
            match hulls
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|ships| grouped(context, member, ships))
            {
                Ok(next) => merge_groups(&mut groups, next),
                Err(reason) => {
                    composition_error.get_or_insert(reason);
                }
            }
        }
    }
    if !organisations.is_empty() {
        if unknown {
            super::append(
                out,
                context,
                false,
                "additional unknown navy",
                line(context, "ADDITIONAL_UNKNOWN_NAVY_DESC", &[]),
            );
        }
        let org = mean(organisations).and_then(|value| {
            line(
                context,
                "UNIT_AVERAGE_ORG",
                &[("VALUE", Arg::Fixed5(value))],
            )
        });
        super::append(out, context, true, "naval average organisation", org);
        let strength = mean(strengths).and_then(|value| {
            line(
                context,
                "UNIT_AVERAGE_STR",
                &[("VALUE", Arg::Fixed5(value))],
            )
        });
        super::append(out, context, true, "naval average strength", strength);
    }
    if unknown {
        super::append(
            out,
            context,
            true,
            "known ships",
            line(context, "KNOWN_SHIPS", &[]),
        );
    }
    super::append(
        out,
        context,
        true,
        "naval ship composition",
        match composition_error {
            Some(reason) => Err(reason),
            None => composition(
                &groups,
                context
                    .content
                    .map(|content| content.equipment.as_ref())
                    .ok_or("equipment content is unavailable"),
            ),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::Recorded, rows::ShipCounterInputs, rows::TableId, rows::TableRows,
        rows::TaskForceCounterInputRow,
    };
    use viewer_foundation::test_support::{test_frame, test_put};

    fn loc() -> Localisation {
        let mut loc = [
            ("COARSE_COUNTER_UNKNOWN_UNITS", "unknown"),
            ("COARSE_COUNTER_KNOWN_UNITS", "partly known"),
            (
                "COARSE_COUNTER_NAVIES_DESC",
                "$NAVIES$ navies, $SHIPS$ ships",
            ),
            ("UNKNOWN_NAVY_DESC", "hidden navy"),
            ("ADDITIONAL_UNKNOWN_NAVY_DESC", "additional hidden navy"),
            ("UNIT_AVERAGE_ORG", "org $VALUE|%0$"),
            ("UNIT_AVERAGE_STR", "str $VALUE|%0$"),
            ("KNOWN_SHIPS", "known ships"),
            ("MAP_ICON_NAVY_SHIPS", "$NAVY$ $COUNTRY$: $VALUE$/$VALUE2$"),
        ]
        .into_iter()
        .map(|(key, text)| (key.into(), text.into()))
        .collect::<Localisation>();
        loc.language = "english".into();
        loc
    }

    fn with_context(frame: &DecodedFrame, run: impl FnOnce(&Context<'_>)) {
        let loc = loc();
        super::super::task_force::tests::with_test_context(
            frame,
            &loc,
            "unreached",
            None,
            |context| {
                run(&context);
            },
        );
    }

    fn model(ship: u64, variant: u64) -> ShipModelRow {
        ShipModelRow {
            ship,
            variant,
            counter: Some(ShipCounterInputs::default()),
            ..Default::default()
        }
    }

    fn group<'a>(
        model: &'a ShipModelRow,
        name: &str,
        strength: i64,
        lower: i32,
        upper: i32,
    ) -> Group<'a> {
        Group {
            hull: counter_order::Hull { strength, model },
            name: name.into(),
            lower,
            upper,
        }
    }

    #[test]
    fn complete_dangling_members_are_skipped_without_reordering_or_collapsing_references() {
        let mut frame = test_frame(600);
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![replay_data::rows::CountryLinkRow {
                id: 41,
                canonical_slot: 1,
            }]),
        );
        let first = 0x0001_0000_0007;
        let later = 0x0002_0000_0007;
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![
                TaskForceRow {
                    id: first,
                    roster_owner: 41,
                    ..Default::default()
                },
                TaskForceRow {
                    id: later,
                    roster_owner: 41,
                    ..Default::default()
                },
            ]),
        );
        with_context(&frame, |context| {
            let admitted = members(context, &[first, 99, later, first]).unwrap();
            assert_eq!(
                admitted
                    .iter()
                    .map(|member| member.force.id)
                    .collect::<Vec<_>>(),
                [first, later, first]
            );
            assert!(members(context, &[99]).unwrap().is_empty());
        });
        frame.tables[TableId::TaskForces.index()] = replay_frame::frame::TableView::Unavailable {
            reason: "unfinished task-force census".into(),
        }
        .into();
        with_context(&frame, |context| {
            assert!(
                members(context, &[99])
                    .err()
                    .unwrap()
                    .contains("unfinished")
            );
        });
    }

    #[test]
    fn coarse_threshold_counts_exact_members_without_name_model_or_content_demands() {
        let mut frame = test_frame(600);
        test_put(
            &mut frame,
            TableRows::Ships(vec![
                ShipRow {
                    id: 1,
                    task_force: 7,
                    ..Default::default()
                },
                ShipRow {
                    id: 2,
                    task_force: 7,
                    ..Default::default()
                },
                ShipRow {
                    id: 3,
                    task_force: 8,
                    ..Default::default()
                },
            ]),
        );
        let force = TaskForceRow {
            id: 7,
            ..Default::default()
        };
        let other = TaskForceRow {
            id: 8,
            ..Default::default()
        };
        for visibility in [0, 99, 100, 149, 150, 249, 250, 255] {
            with_context(&frame, |context| {
                let mut out = Tooltip::default();
                append_coarse(
                    context,
                    &[
                        Member {
                            force: &force,
                            owner: 1,
                            visibility,
                        },
                        Member {
                            force: &other,
                            owner: 2,
                            visibility: 249,
                        },
                    ],
                    &mut out,
                );
                assert_eq!(
                    out.title,
                    if visibility >= 250 {
                        "partly known\n1 navies, 2 ships\n"
                    } else {
                        "unknown\n"
                    }
                );
                assert!(out.body.is_empty());
                assert!(out.refused.is_empty());
            });
        }
    }

    #[test]
    fn hidden_detail_never_reads_ship_rosters_or_cached_names_and_has_no_newline() {
        let frame = test_frame(600);
        let force = TaskForceRow::default();
        with_context(&frame, |context| {
            let mut out = Tooltip::default();
            append_detail(
                context,
                &[Member {
                    force: &force,
                    owner: 2,
                    visibility: 99,
                }],
                &mut out,
            );
            assert_eq!(out.title, "hidden navy");
            assert!(out.body.is_empty());
            assert!(out.refused.is_empty());
        });
    }

    #[test]
    fn missing_cached_header_keeps_empty_roster_stats_and_unknown_fragments() {
        let mut frame = test_frame(600);
        test_put(&mut frame, TableRows::Ships(vec![]));
        let force = TaskForceRow {
            id: 7,
            ..Default::default()
        };
        let hidden = TaskForceRow {
            id: 8,
            ..Default::default()
        };
        with_context(&frame, |context| {
            let mut out = Tooltip::default();
            append_detail(
                context,
                &[
                    Member {
                        force: &force,
                        owner: 1,
                        visibility: 250,
                    },
                    Member {
                        force: &hidden,
                        owner: 2,
                        visibility: 99,
                    },
                ],
                &mut out,
            );
            assert_eq!(out.title, "additional hidden navy\n");
            assert_eq!(out.body, "org 0%\nstr 0%\nknown ships\n");
            assert_eq!(out.refused.len(), 1);
            assert!(
                out.refused
                    .keys()
                    .next()
                    .unwrap()
                    .starts_with("counter naval task force header:")
            );
        });
    }

    #[test]
    fn additional_hidden_title_requires_an_exact_member_but_known_ships_does_not() {
        let mut frame = test_frame(600);
        test_put(&mut frame, TableRows::Ships(vec![]));
        let force = TaskForceRow {
            id: 7,
            ..Default::default()
        };
        let hidden = TaskForceRow {
            id: 8,
            ..Default::default()
        };
        with_context(&frame, |context| {
            for visibility in [100, 149, 150, 249] {
                let mut out = Tooltip::default();
                append_detail(
                    context,
                    &[
                        Member {
                            force: &force,
                            owner: 1,
                            visibility,
                        },
                        Member {
                            force: &hidden,
                            owner: 2,
                            visibility: 99,
                        },
                    ],
                    &mut out,
                );
                assert!(out.title.is_empty());
                assert_eq!(out.body, "known ships\n");
            }
        });
    }

    #[test]
    fn copied_type_index_distinguishes_later_rejected_names_and_preserves_original_spelling() {
        content_reader_script::test_game::enter();
        let mut units = SubUnits::default();
        units.read(
            b"sub_units = { Custom_Destroyer = {} 1rejected = {} }",
            "base.txt",
        );
        units.read(
            b"sub_units = { CUSTOM_DESTROYER = {} 2rejected = {} Mod_Type = {} }",
            "mod.txt",
        );
        let mut model = model(7, 0);
        for (index, name) in [
            (0, ""),
            (1, "Custom_Destroyer"),
            (2, "1rejected"),
            (3, "2rejected"),
            (4, "Mod_Type"),
        ] {
            model.subunit_index = Recorded::Value(Some(index));
            assert_eq!(ship_type(&units, &model).unwrap(), (index, name.into()));
        }
        for index in [-1, 5, i32::MIN] {
            model.subunit_index = Recorded::Value(Some(index));
            assert!(
                ship_type(&units, &model)
                    .unwrap_err()
                    .contains(&index.to_string())
            );
        }
        model.subunit_index = Recorded::Value(None);
        assert!(
            ship_type(&units, &model)
                .unwrap_err()
                .contains("unreadable")
        );
        model.subunit_index = Recorded::NotRecorded;
        assert!(
            ship_type(&units, &model)
                .unwrap_err()
                .contains("ShipModels.subunit_index")
        );
    }

    #[test]
    fn nul_declaration_keys_refuse_index_derivation_instead_of_selecting_a_later_prefix() {
        content_reader_script::test_game::enter();
        let mut units = SubUnits::default();
        units.read(
            b"sub_units = { \"custom\0tail\" = {} custom = {} }",
            "mod.txt",
        );
        assert!(type_database(&units).unwrap_err().contains("embedded-NUL"));
    }

    #[test]
    fn retained_name_keeps_full_bytes_until_the_native_localisation_argument_boundary() {
        let mut frame = test_frame(600);
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                task_force: 7,
                name: Recorded::Value(Some(b"Fleet\0tail".to_vec())),
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let name = name(context, 7).unwrap();
            assert_eq!(name, "Fleet\0tail");
            assert_eq!(
                line(
                    context,
                    "MAP_ICON_NAVY_SHIPS",
                    &[
                        ("NAVY", Arg::Text(&name)),
                        ("COUNTRY", Arg::Text("Country")),
                        ("VALUE", Arg::Text("2")),
                        ("VALUE2", Arg::Text("1"))
                    ]
                )
                .unwrap(),
                "Fleet Country: 2/1\n"
            );
        });
    }

    #[test]
    fn medium_type_minimum_uses_raw_capital_bit_and_keeps_ranges_above_one_hundred() {
        let defines = naval_count_approx::Defines {
            medium_half_range_percent: 0,
            medium_min_ships: 4,
            medium_min_capitals: 1,
            ..naval_count_approx::Defines::FIXTURE
        };
        let force = 7_u64 << 32;
        let carrier = type_range(0, 150, force, counter_order::CARRIER_FLAG, 4, &defines);
        let capital = type_range(0, 150, force, counter_order::CAPITAL_FLAG, 4, &defines);
        assert_eq!(carrier.1, 8);
        assert_eq!(capital.1, 2);
        assert_eq!(
            type_range(2, 150, force, counter_order::CAPITAL_FLAG, 3, &defines),
            (2, 2)
        );
        assert_eq!(type_range(2, 250, force, 0, 3, &defines), (2, 2));
        let hull = model(1, 1);
        let groups = BTreeMap::from([(3, group(&hull, "mod_type", 9, 99, 104))]);
        assert_eq!(
            composition(&groups, Err("unreached equipment content")).unwrap(),
            "99-104 \u{13}ship_mod_type_icon_small|1    "
        );
    }

    #[test]
    fn cross_force_ranges_wrap_and_keep_the_first_exemplar_before_native_hull_sort() {
        let a = model(1, 1);
        let b = model(2, 2);
        let c = model(3, 3);
        let mut all = BTreeMap::from([
            (-1, group(&a, "first", 1, i32::MAX, 5)),
            (2, group(&b, "strong", 10, 2, 2)),
        ]);
        merge_groups(
            &mut all,
            BTreeMap::from([(-1, group(&c, "later", 100, 1, 2))]),
        );
        assert_eq!(all[&-1].lower, i32::MIN);
        assert_eq!(all[&-1].upper, 7);
        assert_eq!(all[&-1].hull.model.ship, 1);
        assert!(
            composition(&all, Err("unreached equipment content"))
                .unwrap()
                .starts_with("2 \u{13}ship_strong_icon_small|1    ")
        );
    }

    #[test]
    fn null_variant_sort_reverses_ties_and_distinct_nonnull_tie_is_named() {
        let null = model(1, 0);
        let groups = (0..33)
            .map(|index| (index, group(&null, &format!("type{index}"), 1, 1, 1)))
            .collect::<BTreeMap<_, _>>();
        let text = composition(&groups, Err("unreached equipment content")).unwrap();
        assert!(text.starts_with("1 \u{13}ship_type32_icon_small|1    "));
        assert!(text.ends_with("1 \u{13}ship_type0_icon_small|1    "));
        let a = model(2, 2);
        let b = model(3, 3);
        let groups = BTreeMap::from([
            (1, group(&a, "first", 1, 1, 1)),
            (2, group(&b, "second", 1, 1, 1)),
        ]);
        let reason = composition(&groups, Err("unreached equipment content")).unwrap_err();
        assert!(reason.contains("comparison"));
        assert!(reason.contains("first") && reason.contains("second"));
    }

    #[test]
    fn current_variant_depth_orders_exact_modded_composition() {
        content_reader_script::test_game::enter();
        let content = viewer_foundation::test_support::equipment_from_documents(&[br"
            equipments = {
                mod_root = { is_archetype = yes type = destroyer }
                mod_deep = { archetype = mod_root parent = mod_root }
            }
        "]);
        let database = &content;
        let selected = |ship, name: &[u8]| {
            let mut row = model(ship, ship);
            row.hull_variant = Recorded::Value(Some(replay_data::rows::HullVariantInputs {
                definition_index: Some(
                    database
                        .equipment
                        .iter()
                        .position(|definition| definition.name == name)
                        .unwrap() as i32
                        + 1,
                ),
                ..Default::default()
            }));
            row
        };
        let first = selected(1, b"mod_root");
        let deeper = selected(2, b"mod_deep");
        let groups = BTreeMap::from([
            (1, group(&first, "first", 5, 2, 2)),
            (2, group(&deeper, "deeper", 5, 3, 4)),
        ]);
        assert_eq!(
            composition(&groups, Ok(database)).unwrap(),
            "3-4 \u{13}ship_deeper_icon_small|1    2 \u{13}ship_first_icon_small|1    "
        );
    }

    #[test]
    fn native_null_definition_composition_needs_no_equipment_content() {
        let selected = |ship, marker| {
            let mut row = model(ship, ship);
            row.hull_variant = Recorded::Value(Some(replay_data::rows::HullVariantInputs {
                definition_index: Some(-1),
                module_count: Some(0),
                upgrade_level_sum: Some(0),
                module_priority_sum: Some(0),
                organisation_marker: Some(marker),
                version: None,
            }));
            row
        };
        let first = selected(1, -5);
        let better = selected(2, -2);
        let groups = BTreeMap::from([
            (0, group(&first, "", 5, 2, 2)),
            (1, group(&better, "mod_type", 5, 3, 3)),
        ]);
        assert_eq!(
            composition(&groups, Err("absent equipment content")).unwrap(),
            "3 \u{13}ship_mod_type_icon_small|1    2 \u{13}ship__icon_small|1    "
        );
    }
}
