
use super::{Arg, Class, Context, Localisation, Tooltip, Unit, line, localise, refuse};
use crate::map::division_eta::{CounterPreview, CounterQueries, CounterUnit};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

pub(super) struct SelectionTravel {
    pub(super) hours: Result<i32, String>,
    pub(super) route: Result<Vec<u32>, String>,
}

pub(super) fn source(frame: &DecodedFrame, unit: Unit) -> Result<Option<u32>, String> {
    let province = match unit {
        Unit::Division(handle) => {
            let rows = frame
                .complete::<tables::Divisions>()
                .map_err(|e| e.to_string())?;
            let index = rows
                .binary_search_by_key(&handle, |row| row.id)
                .map_err(|_| format!("Divisions member {handle} is absent"))?;
            rows[index].province
        }
        Unit::TaskForce(handle) => {
            let rows = frame
                .complete::<tables::TaskForces>()
                .map_err(|e| e.to_string())?;
            let index = rows
                .binary_search_by_key(&handle, |row| row.id)
                .map_err(|_| format!("TaskForces member {handle} is absent"))?;
            rows[index].province
        }
        Unit::RailwayGun(handle) => {
            let rows = frame
                .complete::<tables::RailwayGunCounterInputs>()
                .map_err(|e| e.to_string())?;
            let index = rows
                .binary_search_by_key(&handle, |row| row.gun)
                .map_err(|_| format!("RailwayGunCounterInputs member {handle} is absent"))?;
            rows[index].province
        }
    };
    Ok((province != 0).then_some(province))
}

fn matches_class(class: Class, unit: Unit) -> bool {
    matches!(
        (class, unit),
        (Class::Army, Unit::Division(_)) | (Class::Railway, Unit::RailwayGun(_))
    )
}

fn counter_unit(unit: Unit) -> CounterUnit {
    match unit {
        Unit::Division(handle) => CounterUnit::Division(handle),
        Unit::TaskForce(handle) => CounterUnit::TaskForce(handle),
        Unit::RailwayGun(handle) => CounterUnit::Railway(handle),
    }
}

fn selection_travel(
    class: Class,
    selected: &[Unit],
    mut source: impl FnMut(Unit) -> Result<Option<u32>, String>,
    mut land: impl FnMut(u32) -> Result<bool, String>,
    mut query: impl FnMut(Unit) -> Result<CounterPreview, String>,
) -> SelectionTravel {
    let mut matching = selected
        .iter()
        .copied()
        .filter(|unit| matches_class(class, *unit));
    let mut result = SelectionTravel {
        hours: Ok(-1),
        route: Ok(Vec::new()),
    };
    let Some(first) = matching.next() else {
        return result;
    };
    let first_land = source(first).and_then(|source| match source {
        Some(province) => land(province),
        None => Ok(false),
    });
    match first_land {
        Ok(true) => match query(first) {
            Ok(preview) => {
                result.route = Ok(preview.route);
                result.hours = preview
                    .duration
                    .map(|duration| game_model::movement::unit::eta_hours(duration) as i32);
            }
            Err(reason) => {
                result.route = Err(reason.clone());
                result.hours = Err(reason);
            }
        },
        Ok(false) => {}
        Err(reason) => {
            result.route = Err(reason.clone());
            result.hours = Err(reason);
        }
    }
    if matching.next().is_some() {
        result.hours = fallback_hours(selected, &mut source, &mut query);
    }
    result
}

fn fallback_hours(
    selected: &[Unit],
    source: &mut impl FnMut(Unit) -> Result<Option<u32>, String>,
    query: &mut impl FnMut(Unit) -> Result<CounterPreview, String>,
) -> Result<i32, String> {
    let mut sources = [0_u32; 12];
    let mut occupied = 0;
    let mut best = None;
    let mut unavailable = None;
    for &unit in selected {
        let Some(province) = source(unit)? else {
            continue;
        };
        if sources[..occupied].contains(&province) {
            continue;
        }
        sources[occupied] = province;
        occupied += 1;
        match query(unit)
            .and_then(|preview| preview.duration)
            .map(|duration| game_model::movement::unit::eta_hours(duration) as i32)
        {
            Ok(1) => return Ok(1),
            Ok(hours) if hours > 0 => best = Some(best.map_or(hours, |old: i32| old.min(hours))),
            Ok(_) => {}
            Err(reason) if unavailable.is_none() => unavailable = Some(reason),
            Err(_) => {}
        }
        if occupied == sources.len() {
            break;
        }
    }
    if let Some(reason) = unavailable {
        return Err(reason);
    }
    Ok(best.unwrap_or(-1))
}

pub(super) fn build(
    context: &Context<'_>,
    class: Class,
    selected: &[Unit],
    queries: &CounterQueries<'_>,
) -> SelectionTravel {
    selection_travel(
        class,
        selected,
        |unit| source(context.frame, unit),
        |province| {
            let movement = context
                .content
                .movement
                .as_deref()
                .ok_or("prospective source terrain content is unavailable")?;
            movement
                .map
                .terrain(province)
                .map(|terrain| terrain.flags & 1 != 0)
                .ok_or_else(|| format!("prospective source terrain {province} is unavailable"))
        },
        |unit| queries.preview(counter_unit(unit), context.province.id, false),
    )
}

fn sentence(loc: &Localisation, class: Class, hours: i32) -> Result<String, String> {
    let singular = localise(
        loc,
        match class {
            Class::Army => "DIVISION",
            Class::Railway => "RAILWAY_GUN",
        },
        &[],
    )?;
    let days = hours / 24;
    if days == 0 {
        localise(
            loc,
            "PROVINCE_UNIT_MOVE_HOURS",
            &[
                ("UNIT_TYPE", Arg::Text(&singular)),
                ("HOURS", Arg::Int32(hours)),
            ],
        )
    } else {
        localise(
            loc,
            "PROVINCE_UNIT_MOVE_DAYS_HOURS",
            &[
                ("UNIT_TYPE", Arg::Text(&singular)),
                ("DAYS", Arg::Int32(days)),
                ("HOURS", Arg::Int32(hours % 24)),
            ],
        )
    }
}

pub(super) fn append(
    out: &mut Tooltip,
    loc: &Localisation,
    class: Class,
    travel: &SelectionTravel,
) {
    match &travel.hours {
        Ok(hours) if *hours > 0 => line(out, "travel", 1, sentence(loc, class, *hours)),
        Err(reason) => refuse(out, "travel", reason.clone()),
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::DivisionRow, rows::RailwayGunCounterInputRow, rows::TableRows, rows::TaskForceRow,
    };
    use viewer_foundation::test_support::{test_frame, test_put as put};

    fn preview(duration: Result<i64, String>, route: &[u32]) -> Result<CounterPreview, String> {
        Ok(CounterPreview {
            duration,
            route: route.to_vec(),
        })
    }

    fn handle(unit: Unit) -> u64 {
        match unit {
            Unit::Division(id) | Unit::TaskForce(id) | Unit::RailwayGun(id) => id,
        }
    }

    #[test]
    fn authenticated_null_positive_missing_and_typed_sources_remain_distinct() {
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 7,
                province: 0,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                province: 2,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![RailwayGunCounterInputRow {
                gun: 7,
                province: 3,
                ..Default::default()
            }]),
        );
        assert_eq!(source(&frame, Unit::Division(7)), Ok(None));
        assert_eq!(source(&frame, Unit::TaskForce(7)), Ok(Some(2)));
        assert_eq!(source(&frame, Unit::RailwayGun(7)), Ok(Some(3)));
        assert!(source(&frame, Unit::Division(8)).is_err());
        assert!(source(&test_frame(43), Unit::Division(7)).is_err());
        assert_eq!(counter_unit(Unit::Division(7)), CounterUnit::Division(7));
        assert_eq!(counter_unit(Unit::RailwayGun(7)), CounterUnit::Railway(7));
        assert_eq!(counter_unit(Unit::TaskForce(7)), CounterUnit::TaskForce(7));
    }

    #[test]
    fn sole_requested_class_null_or_nonland_suppresses_all_class_fallback() {
        for province in [None, Some(5)] {
            let mut queried = Vec::new();
            let result = selection_travel(
                Class::Army,
                &[Unit::Division(1), Unit::TaskForce(2), Unit::RailwayGun(3)],
                |_| Ok(province),
                |_| Ok(false),
                |unit| {
                    queried.push(unit);
                    preview(Ok(100_000), &[8])
                },
            );
            assert_eq!(result.hours, Ok(-1));
            assert_eq!(result.route, Ok(vec![]));
            assert!(queried.is_empty());
        }
    }

    #[test]
    fn railway_first_class_uses_first_railway_without_reordering_other_classes() {
        let mut queried = Vec::new();
        let result = selection_travel(
            Class::Railway,
            &[Unit::Division(1), Unit::RailwayGun(2), Unit::TaskForce(3)],
            |_| Ok(Some(5)),
            |_| Ok(true),
            |unit| {
                queried.push(unit);
                preview(Ok(230_001), &[9, 10])
            },
        );
        assert_eq!(queried, [Unit::RailwayGun(2)]);
        assert_eq!(result.hours, Ok(3));
        assert_eq!(result.route, Ok(vec![9, 10]));
    }

    #[test]
    fn second_matching_unit_replaces_only_hours_and_keeps_first_route() {
        let mut queried = Vec::new();
        let result = selection_travel(
            Class::Army,
            &[
                Unit::Division(1),
                Unit::TaskForce(2),
                Unit::RailwayGun(3),
                Unit::Division(4),
            ],
            |unit| Ok(Some(handle(unit) as u32)),
            |_| Ok(true),
            |unit| {
                queried.push(unit);
                preview(
                    Ok(if unit == Unit::TaskForce(2) {
                        200_000
                    } else {
                        500_000
                    }),
                    &[handle(unit) as u32, 99],
                )
            },
        );
        assert_eq!(result.hours, Ok(2));
        assert_eq!(result.route, Ok(vec![1, 99]));
        assert_eq!(
            queried,
            [
                Unit::Division(1),
                Unit::Division(1),
                Unit::TaskForce(2),
                Unit::RailwayGun(3),
                Unit::Division(4)
            ]
        );
    }

    #[test]
    fn first_known_route_survives_duration_refusal_and_fallback_one() {
        let mut calls = 0;
        let result = selection_travel(
            Class::Army,
            &[Unit::Division(1), Unit::Division(2)],
            |unit| Ok(Some(handle(unit) as u32)),
            |_| Ok(true),
            |_| {
                calls += 1;
                if calls == 1 {
                    preview(Err("speed".into()), &[7, 8])
                } else {
                    preview(Ok(100_000), &[99])
                }
            },
        );
        assert_eq!(calls, 2);
        assert_eq!(result.hours, Ok(1));
        assert_eq!(result.route, Ok(vec![7, 8]));
    }

    #[test]
    fn sole_known_route_and_unknown_duration_are_independent() {
        let result = selection_travel(
            Class::Army,
            &[Unit::Division(1)],
            |_| Ok(Some(5)),
            |_| Ok(true),
            |_| preview(Err("speed".into()), &[7]),
        );
        assert_eq!(result.hours, Err("speed".into()));
        assert_eq!(result.route, Ok(vec![7]));
    }

    #[test]
    fn known_empty_route_and_failed_search_do_not_share_a_sentinel() {
        let known = selection_travel(
            Class::Army,
            &[Unit::Division(1)],
            |_| Ok(Some(5)),
            |_| Ok(true),
            |_| preview(Ok(0), &[]),
        );
        assert_eq!(known.hours, Ok(0));
        assert_eq!(known.route, Ok(vec![]));
        let unknown = selection_travel(
            Class::Army,
            &[Unit::Division(1)],
            |_| Ok(Some(5)),
            |_| Ok(true),
            |_| Err("search".into()),
        );
        assert_eq!(unknown.hours, Err("search".into()));
        assert_eq!(unknown.route, Err("search".into()));
    }

    #[test]
    fn first_null_still_retains_empty_route_when_multiple_member_fallback_is_positive() {
        let mut queried = Vec::new();
        let result = selection_travel(
            Class::Army,
            &[Unit::Division(1), Unit::Division(2)],
            |unit| Ok((unit != Unit::Division(1)).then_some(5)),
            |_| panic!("the null first source does not demand LAND"),
            |unit| {
                queried.push(unit);
                preview(Ok(200_000), &[7])
            },
        );
        assert_eq!(result.hours, Ok(2));
        assert_eq!(result.route, Ok(vec![]));
        assert_eq!(queried, [Unit::Division(2)]);
    }

    #[test]
    fn failed_fallback_never_reuses_known_first_hours() {
        let mut calls = 0;
        let result = selection_travel(
            Class::Army,
            &[Unit::Division(1), Unit::Division(2)],
            |unit| Ok(Some(handle(unit) as u32)),
            |_| Ok(true),
            |_| {
                calls += 1;
                if calls == 1 {
                    preview(Ok(500_000), &[7])
                } else {
                    Err("fresh search".into())
                }
            },
        );
        assert_eq!(result.hours, Err("fresh search".into()));
        assert_eq!(result.route, Ok(vec![7]));
    }

    #[test]
    fn unknown_first_land_does_not_make_fallback_demand_land() {
        let mut land_calls = 0;
        let result = selection_travel(
            Class::Army,
            &[Unit::Division(1), Unit::Division(2)],
            |unit| Ok(Some(handle(unit) as u32)),
            |_| {
                land_calls += 1;
                Err("terrain".into())
            },
            |_| preview(Ok(100_000), &[7]),
        );
        assert_eq!(land_calls, 1);
        assert_eq!(result.hours, Ok(1));
        assert_eq!(result.route, Err("terrain".into()));
    }

    #[test]
    fn duplicate_source_cannot_promote_later_available_query() {
        let mut queried = Vec::new();
        let result = fallback_hours(
            &[Unit::Division(1), Unit::TaskForce(2), Unit::RailwayGun(3)],
            &mut |_| Ok(Some(5)),
            &mut |unit| {
                queried.push(unit);
                if unit == Unit::Division(1) {
                    Err("first query".into())
                } else {
                    preview(Ok(100_000), &[])
                }
            },
        );
        assert_eq!(result, Err("first query".into()));
        assert_eq!(queried, [Unit::Division(1)]);
    }

    #[test]
    fn distinct_source_budget_includes_twelfth_and_never_reads_thirteenth() {
        let selected: Vec<_> = (1..=13).map(Unit::Division).collect();
        let mut read = Vec::new();
        let mut queried = Vec::new();
        let result = fallback_hours(
            &selected,
            &mut |unit| {
                read.push(unit);
                Ok(Some(handle(unit) as u32))
            },
            &mut |unit| {
                queried.push(unit);
                assert_ne!(handle(unit), 13);
                preview(Ok(if handle(unit) == 12 { 200_000 } else { 500_000 }), &[])
            },
        );
        assert_eq!(result, Ok(2));
        assert_eq!(read.len(), 12);
        assert_eq!(queried.len(), 12);
    }

    #[test]
    fn null_and_duplicate_sources_do_not_consume_distinct_budget() {
        let mut selected = vec![Unit::TaskForce(0), Unit::Division(1), Unit::RailwayGun(1)];
        selected.extend((2..=13).map(Unit::Division));
        let mut queried = Vec::new();
        let result = fallback_hours(
            &selected,
            &mut |unit| {
                let id = handle(unit) as u32;
                Ok((id != 0).then_some(id))
            },
            &mut |unit| {
                queried.push(unit);
                preview(Ok(500_000), &[])
            },
        );
        assert_eq!(result, Ok(5));
        assert_eq!(queried.len(), 12);
        assert!(!queried.contains(&Unit::RailwayGun(1)));
        assert!(!queried.contains(&Unit::Division(13)));
    }

    #[test]
    fn admitted_one_survives_earlier_duration_failure_and_skips_later_unknown_source() {
        let selected = [Unit::Division(1), Unit::TaskForce(2), Unit::Division(3)];
        let mut read = Vec::new();
        let result = fallback_hours(
            &selected,
            &mut |unit| {
                read.push(unit);
                if unit == Unit::Division(3) {
                    Err("source".into())
                } else {
                    Ok(Some(handle(unit) as u32))
                }
            },
            &mut |unit| {
                if unit == Unit::Division(1) {
                    Err("duration".into())
                } else {
                    preview(Ok(1), &[])
                }
            },
        );
        assert_eq!(result, Ok(1));
        assert_eq!(read, [Unit::Division(1), Unit::TaskForce(2)]);
    }

    #[test]
    fn earlier_unknown_source_prevents_later_one_from_being_admitted() {
        let mut queried = Vec::new();
        let result = fallback_hours(
            &[Unit::Division(1), Unit::Division(2)],
            &mut |unit| {
                if unit == Unit::Division(1) {
                    Err("source".into())
                } else {
                    Ok(Some(2))
                }
            },
            &mut |unit| {
                queried.push(unit);
                preview(Ok(100_000), &[])
            },
        );
        assert_eq!(result, Err("source".into()));
        assert!(queried.is_empty());
    }

    #[test]
    fn positive_minimum_cannot_hide_unknown_duration_unless_it_is_one() {
        assert_eq!(
            fallback_hours(
                &[Unit::Division(1), Unit::Division(2)],
                &mut |unit| Ok(Some(handle(unit) as u32)),
                &mut |unit| if unit == Unit::Division(1) {
                    preview(Ok(200_000), &[])
                } else {
                    Err("duration".into())
                }
            ),
            Err("duration".into())
        );
    }

    #[test]
    fn signed_ceiling_and_i32_narrowing_precede_positive_filter() {
        for (duration, expected) in [
            (-100_001, -1),
            (-1, -1),
            (0, -1),
            (1, 1),
            (100_000, 1),
            (100_001, 2),
            (4_294_967_296_i64 * 100_000, -1),
            (4_294_967_297_i64 * 100_000, 1),
        ] {
            assert_eq!(
                fallback_hours(
                    &[Unit::Division(1)],
                    &mut |_| Ok(Some(5)),
                    &mut |_| preview(Ok(duration), &[])
                ),
                Ok(expected),
                "duration {duration}"
            );
        }
    }

    #[test]
    fn exact_selected_eta_keys_singular_and_day_hour_arguments() {
        let loc: Localisation = [
            ("DIVISION", "division"),
            ("RAILWAY_GUN", "railway gun"),
            ("PROVINCE_UNIT_MOVE_HOURS", "$UNIT_TYPE$ $HOURS$h"),
            (
                "PROVINCE_UNIT_MOVE_DAYS_HOURS",
                "$UNIT_TYPE$ $DAYS$d $HOURS$h",
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
        for (class, hours, expected) in [
            (Class::Army, 23, "    division 23h"),
            (Class::Army, 24, "    division 1d 0h"),
            (Class::Railway, 49, "    railway gun 2d 1h"),
        ] {
            let mut out = Tooltip::default();
            append(
                &mut out,
                &loc,
                class,
                &SelectionTravel {
                    hours: Ok(hours),
                    route: Ok(vec![]),
                },
            );
            assert_eq!(out.title, expected);
            assert!(out.refused.is_empty());
        }
        let mut out = Tooltip::default();
        append(
            &mut out,
            &Localisation::default(),
            Class::Army,
            &SelectionTravel {
                hours: Ok(0),
                route: Ok(vec![]),
            },
        );
        assert!(out.title.is_empty());
        assert!(out.refused.is_empty());
    }
}
