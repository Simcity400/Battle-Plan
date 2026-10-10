//! At-sea task-force counter tooltips.
//! Port mission items use a separate callback without an arrival query.

use super::*;
use crate::map::unit_selection;
use replay::rows::TaskForceMapMotionRow;

pub use super::gui_authored::Authored;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Control {
    #[default]
    Normal,
    InvalidOrders,
    NoOrders,
    Supply,
    Carrier,
    Pride,
}

/// The shared handler observes the control actually hit inside the drawn at-sea plate.
#[derive(Clone, Debug, PartialEq)]
pub struct Hover {
    pub control: Control,
    /// The corresponding typed plate in this presented CounterReport.
    pub unit_hit: usize,
    pub rect: [f32; 4],
    pub(crate) hit: Option<crate::map::native_gui::HitGeometry>,
    pub authored: Option<Authored>,
    pub supported: bool,
    pub ours: bool,
    pub coarse: bool,
    pub hidden: bool,
    pub members: Arc<[u64]>,
}

impl Hover {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.hit.as_ref().map_or_else(
            || crate::combat::contains(self.rect, point),
            |hit| hit.contains(point),
        )
    }

    pub fn matches(&self, unit: &unit_selection::Hit) -> bool {
        !unit.port && unit.kind == unit_selection::Kind::TaskForce && unit.members == self.members
    }
}

fn motion<'a>(context: &Context<'a>, id: u64) -> Result<&'a TaskForceMapMotionRow, String> {
    let rows = context
        .frame
        .complete::<tables::TaskForceMapMotion>()
        .map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&id, |row| row.task_force)
        .map(|index| &rows[index])
        .map_err(|_| format!("TaskForceMapMotion task force {id} is absent"))
}

/// The first member's ordinary route and target gate both native movement lines.
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
        // The game must resolve before either ownership
        // branch. A foreign first-edge query never bypasses this final gate.
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
    // Call the map handler before the counter fallback. The counter
    // declines immediately when its representative reference does not resolve.
    // The detailed generic arm's handled byte is set only by the ETA formatter.
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
                authored.fallback(context, &mut out);
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
    if !input.coarse
        && let Some(authored) = &input.authored
    {
        match handled {
            Ok(false) => authored.fallback(context, &mut out),
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
mod tests {
    use super::*;
    use crate::map::{test_frame, test_put};
    use replay::rows::{CountryLinkRow, ProvinceRow, TableRows, TaskForceRow};

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
        let names = installed_content::division_names::Database::default();
        let units = Err("unreached".into());
        let vision = Vision::sharing(1, []);
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
                TableRows::TaskForces(vec![TaskForceRow {
                    id: 7,
                    route_count: count,
                    ..Default::default()
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
            test_put(
                &mut frame,
                TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                    task_force: 7,
                    path: vec![2, last],
                    target,
                    ..Default::default()
                }]),
            );
            let context = Context {
                nato: false,
                content: None,
                dig_in_factor: Err("unreached".into()),
                loc: &loc,
                recorded_language: &loc.language,
                icons: &crate::combat::binding::NoIcons,
                names: &names,
                units: &units,
                supply_grace: Err("unreached".into()),
                comms_max_distance: Err("unreached".into()),
                frame: &frame,
                vision: &vision,
                links: Ok(frame.complete::<tables::CountryLinks>().unwrap()),
            };
            let input = Hover {
                control: Control::Normal,
                unit_hit: 0,
                rect: [0.; 4],
                hit: None,
                authored: None,
                supported: true,
                ours,
                coarse,
                hidden,
                members: vec![7, 999].into(),
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
        }
    }

    #[test]
    fn task_force_hover_never_matches_a_port_or_other_unit_class_with_the_same_handle() {
        let hover = Hover {
            control: Control::Normal,
            unit_hit: 0,
            rect: [0.; 4],
            hit: None,
            authored: None,
            supported: true,
            ours: true,
            coarse: false,
            hidden: false,
            members: vec![7].into(),
        };
        let mut hit = unit_selection::Hit {
            ours: true,
            port: false,
            rect: [0.; 4],
            kind: unit_selection::Kind::TaskForce,
            members: Arc::clone(&hover.members),
            layer: 0,
        };
        assert!(hover.matches(&hit));
        hit.port = true;
        assert!(!hover.matches(&hit));
        hit.port = false;
        hit.kind = unit_selection::Kind::Division;
        assert!(!hover.matches(&hit));
    }

    #[test]
    fn task_force_handled_empty_suppresses_authored_text_and_dangling_first_falls_back() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let loc: Localisation = [
            ("TITLE".into(), "authored title".into()),
            ("BODY".into(), "authored body".into()),
        ]
        .into_iter()
        .collect();
        let gui = installed_content::gui::native::read(
            br#"guiTypes = { iconType = {
            name = control pdx_tooltip = TITLE pdx_tooltip_delayed = BODY
        } }"#,
        );
        let authored = Authored::read(gui.template("control").unwrap(), false).unwrap();
        let names = installed_content::division_names::Database::default();
        let units = Err("unreached".into());
        let vision = Vision::sharing(1, []);
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
            let context = Context {
                nato: false,
                content: None,
                dig_in_factor: Err("unreached".into()),
                loc: &loc,
                recorded_language: &loc.language,
                icons: &crate::combat::binding::NoIcons,
                names: &names,
                units: &units,
                supply_grace: Err("unreached".into()),
                comms_max_distance: Err("unreached".into()),
                frame: &frame,
                vision: &vision,
                links: Err("unreached".into()),
            };
            let input = Hover {
                control: Control::NoOrders,
                unit_hit: 0,
                rect: [0.; 4],
                hit: None,
                authored: Some(authored.clone()),
                supported: true,
                ours: true,
                coarse: false,
                hidden: false,
                members: Arc::from([7]),
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
        }
    }

    #[test]
    fn task_force_detailed_fallback_follows_eta_handled_flag_not_summary_text() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use replay::rows::{
            CountryAppearanceRow, CountryRow, PoliticsRow, TaskForceCounterInputRow,
        };
        let loc: Localisation = [
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
        let gui = installed_content::gui::native::read(
            br#"guiTypes = { iconType = {
            name = control pdx_tooltip = TITLE pdx_tooltip_delayed = BODY
        } }"#,
        );
        let authored = Authored::read(gui.template("control").unwrap(), false).unwrap();
        let names = installed_content::division_names::Database::default();
        let units = Err("unreached".into());
        let vision = Vision::sharing(1, []);
        for (moving, coarse, remaining) in [
            (false, false, Ok(0)),
            (true, false, Ok(0)),
            (true, false, Err("fresh query unavailable".to_owned())),
            (false, true, Ok(0)),
        ] {
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
                TableRows::TaskForces(vec![TaskForceRow {
                    id: 7,
                    roster_owner: 1,
                    route_count: i32::from(moving),
                    ..Default::default()
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
            let context = Context {
                nato: false,
                content: None,
                dig_in_factor: Err("unreached".into()),
                loc: &loc,
                recorded_language: &loc.language,
                icons: &crate::combat::binding::NoIcons,
                names: &names,
                units: &units,
                supply_grace: Err("unreached".into()),
                comms_max_distance: Err("unreached".into()),
                frame: &frame,
                vision: &vision,
                links: Ok(frame.complete::<tables::CountryLinks>().unwrap()),
            };
            let input = Hover {
                control: Control::Normal,
                unit_hit: 0,
                rect: [0.; 4],
                hit: None,
                authored: Some(authored.clone()),
                supported: true,
                ours: true,
                coarse,
                hidden: false,
                members: Arc::from([7]),
            };
            let failed = remaining.is_err();
            let mut summary = Tooltip::default();
            super::super::task_force_summary::append(&context, &input.members, false, &mut summary);
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
        }
    }
}
