
use super::*;
use replay_data::rows::TaskForceMapMotionRow;
#[cfg(test)]
use viewer_native_gui::counter_hovers::task_force::Control;
use viewer_native_gui::counter_hovers::task_force::Hover;
#[cfg(test)]
use viewer_foundation::tooltip_text::NoIcons;

#[cfg(test)]
use viewer_native_gui::counter_hovers::gui_authored::Authored;

fn motion<'a>(context: &Context<'a>, id: u64) -> Result<&'a TaskForceMapMotionRow, String> {
    let rows = context
        .frame
        .complete::<tables::TaskForceMapMotion>()
        .map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&id, |row| row.task_force)
        .map(|index| &rows[index])
        .map_err(|_| format!("TaskForceMapMotion task force {id} is absent"))
}

fn append_arrival(
    context: &Context<'_>,
    input: &Hover,
    out: &mut Tooltip,
    query: impl FnOnce(&TaskForceMapMotionRow, bool) -> Result<i64, String>,
) -> Result<bool, String> {
    if input.coarse || (!input.ours && input.hidden) {
        return Ok(false);
    }
    let Some(&first) = input.members.first() else {
        return Ok(false);
    };
    let arrival = (|| {
        let forces = context
            .frame
            .complete::<tables::TaskForces>()
            .map_err(|e| e.to_string())?;
        let force = forces
            .binary_search_by_key(&first, |row| row.id)
            .map(|index| &forces[index])
            .map_err(|_| format!("TaskForces task force {first} is absent"))?;
        if force.route_count < 1 {
            return Ok(None);
        }
        let movement = motion(context, first)?;
        if movement.target != 0 {
            return Ok(None);
        }
        if movement.path.len() != force.route_count as usize {
            return Err(format!(
                "task force {first} ordinary route count does not match its captured path"
            ));
        }
        let last = *movement.path.last().expect("positive matching route count");
        if last == 0 {
            return Ok(None);
        }
        let provinces = context
            .frame
            .complete::<tables::Provinces>()
            .map_err(|e| e.to_string())?;
        if provinces.binary_search_by_key(&last, |row| row.id).is_err() {
            return Ok(None);
        }
        let remaining = query(movement, input.ours)?;
        Ok(Some(counter_eta::map_arrival(remaining)))
    })();
    match arrival {
        Ok(Some(arrival)) => {
            append(
                out,
                context,
                true,
                "task force arrival",
                counter_eta::sentence(context.loc, &arrival, input.ours)
                    .map(|text| format!("{text}\n")),
            );
            Ok(true)
        }
        Ok(None) => Ok(false),
        Err(reason) => {
            refuse(out, "task force arrival", reason.clone());
            Err(reason)
        }
    }
}

pub(super) fn build(
    context: &Context<'_>,
    input: &Hover,
    query: impl FnOnce(&TaskForceMapMotionRow, bool) -> Result<i64, String>,
) -> Tooltip {
    let mut out = Tooltip::default();
    let resolved = input.members.first().map_or(Ok(false), |first| {
        context
            .frame
            .complete::<tables::TaskForces>()
            .map(|rows| rows.binary_search_by_key(first, |row| row.id).is_ok())
            .map_err(|e| e.to_string())
    });
    match resolved {
        Ok(false) => {
            if let Some(authored) = &input.authored {
                authored.fallback(context.loc, context.icons, &mut out);
            }
            return out;
        }
        Err(reason) => {
            refuse(&mut out, "task force representative", reason);
            return out;
        }
        Ok(true) => (),
    }
    if let Some(out) = super::task_force_children::specific(context, input) {
        return out;
    }
    let handled = append_arrival(context, input, &mut out, query);
    super::task_force_summary::append(context, &input.members, input.coarse, &mut out);
    super::task_force_children::append(context, input, &mut out);
    if input.ours && !input.coarse && context.diagnostics {
        super::naval_management::append(context, &input.members, &mut out);
    }
    if !input.coarse
        && let Some(authored) = &input.authored
    {
        match handled {
            Ok(false) => authored.fallback(context.loc, context.icons, &mut out),
            Err(_) => refuse(
                &mut out,
                "authored tooltip dispatch",
                "task force arrival operands are unavailable, so the native handled result is unknown",
            ),
            Ok(true) => (),
        }
    }
    out
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use replay_data::{
        rows::CountryLinkRow, rows::ProvinceRow, rows::TableRows, rows::TaskForceRow,
    };
    use viewer_foundation::test_support::{test_frame, test_put};

    pub(in crate::map::counter_tooltips) fn with_test_context<R>(
        frame: &DecodedFrame,
        loc: &Localisation,
        unavailable: &str,
        recorded_language: Option<&str>,
        run: impl FnOnce(Context<'_>) -> R,
    ) -> R {
        let names = content_reader_division_names::division_names::Database::default();
        let units = Err(unavailable.into());
        let vision = Vision::sharing(1, []);
        run(Context {
            nato: false,
            diagnostics: true,
            content: None,
            reads: None,
            dig_in_factor: Err(unavailable.into()),
            loc,
            recorded_language: recorded_language.unwrap_or(&loc.language),
            icons: &NoIcons,
            names: &names,
            units: &units,
            supply_grace: Err(unavailable.into()),
            comms_max_distance: Err(unavailable.into()),
            frame,
            vision: &vision,
            links: frame
                .complete::<tables::CountryLinks>()
                .map_err(|e| e.to_string()),
        })
    }

    fn hover(control: Control, members: &[u64]) -> Hover {
        Hover {
            control,
            unit_hit: 0,
            rect: [0.; 4],
            hit: None,
            authored: None,
            supported: true,
            ours: true,
            coarse: false,
            hidden: false,
            members: members.into(),
        }
    }

    #[test]
    fn task_force_arrival_queries_only_the_first_eligible_detail_and_uses_viewer_wording() {
        let loc: Localisation = [
            ("UNIT_MOVING_HOURS".into(), "own $HOURS$h".into()),
            (
                "UNIT_MOVING_DAYS_HOURS".into(),
                "own $DAYS$d $HOURS$h".into(),
            ),
            (
                "UNIT_MOVING_DAYS_HOURS_NONPLAYER".into(),
                "other $DAYS$d $HOURS$h".into(),
            ),
        ]
        .into_iter()
        .collect();
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            }]),
        );
        test_put(
            &mut frame,
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 2,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 3,
                    ..Default::default()
                },
            ]),
        );
        for (coarse, ours, hidden, count, target, last, remaining, expected_calls, expected) in [
            (false, true, false, 2, 0, 3, 2_900_000, 1, "own 1d 5h\n"),
            (false, false, false, 2, 0, 3, 2_900_000, 1, "other 1d 5h\n"),
            (false, true, false, 2, 0, 3, 0, 1, "own 0h\n"),
            (false, true, false, 2, 0, 3, -2_900_000, 1, "own -29h\n"),
            (false, false, true, 2, 0, 3, 2_900_000, 0, ""),
            (true, true, false, 2, 0, 3, 2_900_000, 0, ""),
            (false, true, false, 0, 0, 3, 2_900_000, 0, ""),
            (false, true, false, 2, 3, 3, 2_900_000, 0, ""),
            (false, false, false, 2, 0, 0, 2_900_000, 0, ""),
            (false, false, false, 2, 0, 999, 2_900_000, 0, ""),
        ] {
            test_put(
                &mut frame,
                TableRows::TaskForces(vec![TaskForceRow {
                    id: 7,
                    route_count: count,
                    ..Default::default()
                }]),
            );
            test_put(
                &mut frame,
                TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                    task_force: 7,
                    path: vec![2, last],
                    target,
                    ..Default::default()
                }]),
            );
            with_test_context(&frame, &loc, "unreached", None, |context| {
                let input = Hover {
                    ours,
                    coarse,
                    hidden,
                    ..hover(Control::Normal, &[7, 999])
                };
                let mut calls = 0;
                let mut out = Tooltip::default();
                let handled = append_arrival(&context, &input, &mut out, |row, query_ours| {
                    calls += 1;
                    assert_eq!(row.task_force, 7);
                    assert_eq!(query_ours, ours);
                    Ok(remaining)
                });
                assert_eq!(handled, Ok(expected_calls != 0));
                assert_eq!(calls, expected_calls);
                assert_eq!(out.body, expected);
                assert!(out.refused.is_empty());
            });
        }
    }

    #[test]
    fn task_force_hover_never_matches_a_port_or_other_unit_class_with_the_same_handle() {
        let hover = hover(Control::Normal, &[7]);
        let mut hit = viewer_native_gui::counter_hovers::unit_hits::Hit {
            plate: None,
            ours: true,
            port: false,
            rect: [0.; 4],
            kind: viewer_native_gui::counter_hovers::unit_hits::Kind::TaskForce,
            members: Arc::clone(&hover.members),
            layer: 0,
        };
        assert!(hover.matches(&hit));
        hit.port = true;
        assert!(!hover.matches(&hit));
        hit.port = false;
        hit.kind = viewer_native_gui::counter_hovers::unit_hits::Kind::Division;
        assert!(!hover.matches(&hit));
    }

    #[test]
    fn task_force_handled_empty_suppresses_authored_text_and_dangling_first_falls_back() {
        content_reader_script::test_game::enter();
        let loc: Localisation = [
            ("TITLE".into(), "authored title".into()),
            ("BODY".into(), "authored body".into()),
        ]
        .into_iter()
        .collect();
        let gui = content_reader_gui::gui::native::read(
            br#"guiTypes = { iconType = {
            name = control pdx_tooltip = TITLE pdx_tooltip_delayed = BODY
        } }"#,
        );
        let authored = Authored::read(gui.template("control").unwrap(), false).unwrap();
        for present in [Some(true), Some(false), None] {
            let mut frame = test_frame(0);
            if let Some(present) = present {
                test_put(
                    &mut frame,
                    TableRows::TaskForces(if present {
                        vec![TaskForceRow {
                            id: 7,
                            mission: 8,
                            ..Default::default()
                        }]
                    } else {
                        Vec::new()
                    }),
                );
            }
            with_test_context(&frame, &loc, "unreached", None, |context| {
                let context = Context {
                    links: Err("unreached".into()),
                    ..context
                };
                let input = Hover {
                    authored: Some(authored.clone()),
                    ..hover(Control::NoOrders, &[7])
                };
                let out = build(&context, &input, |_, _| {
                    panic!("no-orders or declined callback queried ETA")
                });
                if present == Some(false) {
                    assert_eq!(out.title, "authored title\n");
                    assert_eq!(out.body, "authored body\n");
                    assert!(out.refused.is_empty());
                } else {
                    assert!(out.title.is_empty() && out.body.is_empty());
                    assert_eq!(out.refused.is_empty(), present == Some(true));
                }
            });
        }
    }

    #[test]
    fn task_force_detailed_fallback_follows_eta_handled_flag_not_summary_text() {
        content_reader_script::test_game::enter();
        use replay_data::{
            rows::CountryAppearanceRow, rows::CountryRow, rows::PoliticsRow,
            rows::TaskForceCounterInputRow,
        };
        let mut loc: Localisation = [
            ("TITLE".into(), "authored title".into()),
            ("BODY".into(), "authored body".into()),
            ("UNIT_MOVING_HOURS".into(), "own $HOURS$h".into()),
            (
                "MAP_ICON_NAVY_SHIPS".into(),
                "$NAVY$ $COUNTRY$: $VALUE$/$VALUE2$".into(),
            ),
            ("AAA".into(), "Country".into()),
        ]
        .into_iter()
        .collect();
        loc.language = "english".into();
        let gui = content_reader_gui::gui::native::read(
            br#"guiTypes = { iconType = {
            name = control pdx_tooltip = TITLE pdx_tooltip_delayed = BODY
        } }"#,
        );
        let authored = Authored::read(gui.template("control").unwrap(), false).unwrap();
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            }]),
        );
        test_put(&mut frame, TableRows::Ships(Vec::new()));
        test_put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                tag: b"AAA".to_vec(),
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 1,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::Politics(vec![PoliticsRow {
                country: 1,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                task_force: 7,
                name: Some(b"Fleet".to_vec()).into(),
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 3,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                task_force: 7,
                path: vec![3],
                ..Default::default()
            }]),
        );
        for (moving, coarse, remaining) in [
            (false, false, Ok(0)),
            (true, false, Ok(0)),
            (true, false, Err("fresh query unavailable".to_owned())),
            (false, true, Ok(0)),
        ] {
            test_put(
                &mut frame,
                TableRows::TaskForces(vec![TaskForceRow {
                    id: 7,
                    roster_owner: 1,
                    route_count: i32::from(moving),
                    ..Default::default()
                }]),
            );
            with_test_context(&frame, &loc, "unreached", None, |context| {
                let input = Hover {
                    authored: Some(authored.clone()),
                    coarse,
                    ..hover(Control::Normal, &[7])
                };
                let failed = remaining.is_err();
                let mut summary = Tooltip::default();
                super::super::task_force_summary::append(
                    &context,
                    &input.members,
                    false,
                    &mut summary,
                );
                assert_eq!(summary.title, "Fleet Country: 0/0\n");
                let mut calls = 0;
                let out = build(&context, &input, |_, _| {
                    calls += 1;
                    remaining
                });
                assert_eq!(calls, usize::from(moving && !coarse));
                if !moving && !coarse {
                    assert_eq!(out.title, "authored title\n");
                    assert_eq!(out.body, "authored body\n");
                } else {
                    assert!(!out.title.contains("authored") && !out.body.contains("authored"));
                    if moving && !failed {
                        assert!(out.body.starts_with("own 0h\n"));
                        assert_eq!(out.title, summary.title);
                    }
                }
                assert_eq!(
                    out.refused
                        .keys()
                        .any(|reason| reason.contains("authored tooltip dispatch")),
                    failed
                );
            });
        }
    }
}
