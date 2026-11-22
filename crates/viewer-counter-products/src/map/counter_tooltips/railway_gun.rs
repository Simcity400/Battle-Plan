
use super::*;
use replay_data::rows::RailwayGunCounterInputRow;
#[cfg(test)]
use viewer_foundation::tooltip_text::NoIcons;

fn newline(text: &mut String) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
}

fn members<'a>(
    context: &Context<'a>,
    input: &CounterHover,
) -> Result<Vec<&'a RailwayGunCounterInputRow>, String> {
    let rows = context
        .frame
        .complete::<tables::RailwayGunCounterInputs>()
        .map_err(|error| error.to_string())?;
    Ok(input
        .members
        .iter()
        .filter_map(|id| {
            rows.binary_search_by_key(id, |row| row.gun)
                .map(|index| &rows[index])
                .ok()
        })
        .collect())
}

fn known<'a>(
    context: &Context<'_>,
    members: &[&'a RailwayGunCounterInputRow],
) -> Result<Vec<&'a RailwayGunCounterInputRow>, String> {
    let mut known = Vec::new();
    for &gun in members {
        let slot = context.slot(gun.roster_owner)?;
        let intel = context
            .vision
            .land_byte_known(gun.gun, slot, gun.province)
            .ok_or_else(|| format!("railway gun {} GetUnitIntelLevel is unavailable", gun.gun))?;
        if intel >= 250 {
            known.push(gun);
        }
    }
    Ok(known)
}

fn header(
    context: &Context<'_>,
    first: &RailwayGunCounterInputRow,
    count: i32,
) -> Result<String, String> {
    let country = context.country_name(context.slot(first.roster_owner)?)?;
    if count == 1 {
        let bytes = first
            .name
            .require("RailwayGunCounterInputs.name")?
            .as_ref()
            .ok_or("RailwayGunCounterInputs.name was unreadable when recorded")?;
        let name = dashboard_text::scripted_text::stored_text(
            context.loc,
            context.recorded_language,
            bytes,
            "railway gun name",
        )?;
        localise(
            context.loc,
            "RAILWAY_GUN_HEADER_SINGLE",
            &[("NAME", Arg::Text(&name)), ("COUNTRY", Arg::Text(&country))],
        )
    } else {
        localise(
            context.loc,
            "RAILWAY_GUN_HEADER_MULTIPLE",
            &[
                ("COUNT", Arg::Int32(count)),
                ("COUNTRY", Arg::Text(&country)),
            ],
        )
    }
}

fn counts(context: &Context<'_>, out: &mut Tooltip, known: i32, unknown: bool, coarse: bool) {
    if unknown {
        let key = match (coarse, known > 0) {
            (false, _) => "UNKNOWN_RAILWAY_GUN_DESC",
            (true, false) => "COARSE_COUNTER_UNKNOWN_UNITS",
            (true, true) => "COARSE_COUNTER_KNOWN_UNITS",
        };
        append(
            out,
            context,
            false,
            "railway gun intel header",
            localise(context.loc, key, &[]),
        );
        if coarse {
            out.title.push('\n');
        } else {
            newline(&mut out.title);
        }
        if !coarse && known > 0 {
            append(
                out,
                context,
                false,
                "railway gun known count",
                localise(
                    context.loc,
                    "KNOWN_RAILWAY_GUN_DESC",
                    &[("KNOWN", Arg::Int32(known))],
                ),
            );
            newline(&mut out.title);
        }
    }
    if coarse && known > 0 {
        append(
            out,
            context,
            false,
            "railway gun coarse count",
            localise(
                context.loc,
                "COARSE_COUNTER_RAILWAY_GUNS_DESC",
                &[("RAILWAY_GUNS", Arg::Int32(known))],
            ),
        );
        out.title.push('\n');
    }
}

pub(super) fn build(
    context: &Context<'_>,
    input: &CounterHover,
    arrival_remaining: impl FnOnce(&RailwayGunCounterInputRow, bool) -> Result<i64, String>,
) -> (Tooltip, Result<bool, String>) {
    let mut out = Tooltip::default();
    if input.members.is_empty() {
        return (out, Ok(false));
    }
    let members = match members(context, input) {
        Ok(members) => members,
        Err(reason) => {
            refuse(&mut out, "railway gun inputs", reason.clone());
            return (out, Err(reason));
        }
    };
    let known = known(context, &members);
    let mut handled = Ok(input.coarse);
    if !input.coarse && !members[0].ordinary_path.is_empty() && members[0].target == 0 {
        let arrival: Result<Option<counter_eta::Arrival>, String> = (|| {
            if !input.ours && input.hidden {
                return Ok(None);
            }
            if !ordinary_destination(
                context,
                &members[0].ordinary_path,
                members[0].ordinary_path.len() as u32,
            )? {
                return Ok(None);
            }
            let remaining = arrival_remaining(members[0], input.ours)?;
            Ok(Some(counter_eta::map_arrival(remaining)))
        })();
        match arrival {
            Ok(Some(arrival)) => {
                handled = Ok(true);
                append(
                    &mut out,
                    context,
                    true,
                    "railway gun arrival",
                    counter_eta::sentence(context.loc, &arrival, input.ours)
                        .map(|text| format!("{text}\n")),
                );
            }
            Ok(None) => (),
            Err(reason) => {
                refuse(&mut out, "railway gun arrival", reason.clone());
                handled = Err(reason);
            }
        }
        let next = (|| {
            if input.timing.len() != input.members.len() {
                return Err("railway gun member timing is unavailable".into());
            }
            let rows = context
                .frame
                .complete::<tables::RailwayGunCounterInputs>()
                .map_err(|error| error.to_string())?;
            let timed: Vec<_> = input
                .members
                .iter()
                .zip(input.timing.iter())
                .filter_map(|(id, timing)| {
                    let gun = &rows[rows.binary_search_by_key(id, |row| row.gun).ok()?];
                    Some((
                        gun.gun,
                        gun.province,
                        gun.ordinary_path.len() as u32,
                        timing,
                    ))
                })
                .collect();
            next_location(context.loc, next_hours(&timed)?, false)
        })();
        append(&mut out, context, true, "railway gun next location", next);
    }
    let known = match known {
        Ok(known) => known,
        Err(reason) => {
            refuse(
                &mut out,
                "railway gun intel header/stat eligibility",
                reason,
            );
            return (out, handled);
        }
    };
    let count = known.len() as i32;
    counts(
        context,
        &mut out,
        count,
        known.len() < members.len(),
        input.coarse,
    );
    if input.coarse {
        return (out, handled);
    }
    if count > 0 {
        newline(&mut out.title);
        append(
            &mut out,
            context,
            false,
            "railway gun name",
            header(context, members[0], count),
        );
        let strength = counter_inputs::member_mean(known.iter().map(|gun| {
            viewer_unit_reads::railway_gun_counter::strength_ratio(
                gun.current_strength,
                gun.maximum_strength,
            )
        }))
        .expect("known railway guns are nonempty");
        append(
            &mut out,
            context,
            true,
            "railway gun strength",
            localise(
                context.loc,
                "UNIT_AVERAGE_STR",
                &[("VALUE", Arg::Fixed5(strength))],
            ),
        );
    }
    (out, handled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::CountryLinkRow, rows::ProvinceRow, rows::Recorded, rows::TableRows};
    use std::cell::Cell;
    use viewer_native_gui::counter_hovers::land::CounterControl;
    use viewer_foundation::test_support::{test_frame, test_put as put};

    fn hover() -> CounterHover {
        CounterHover {
            control: CounterControl::Normal,
            authored: None,
            rect: [0.0; 4],
            members: vec![7].into(),
            timing: vec![Timing {
                next: Some(100_001),
                kind: Some(17),
            }]
            .into(),
            coarse: false,
            hidden: false,
            railway_gun: true,
            remaining: None,
            ours: true,
        }
    }

    fn gun() -> RailwayGunCounterInputRow {
        RailwayGunCounterInputRow {
            gun: 7,
            roster_owner: 11,
            province: 3,
            ordinary_path: vec![4],
            current_strength: 10,
            maximum_strength: 10,
            name: Recorded::Value(Some(b"Gun".to_vec())),
            ..Default::default()
        }
    }

    fn localisation() -> Localisation {
        [
            ("UNIT_MOVING_HOURS", "arrival $HOURS$h"),
            ("UNIT_MOVING_HOURS_NONPLAYER", "foreign arrival $HOURS$h"),
            ("UNIT_COUNTER_MOVE_HOURS", "next $HOURS$h"),
            ("UNIT_AVERAGE_STR", "str $VALUE|%0$"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    }

    fn fixture(
        input: &CounterHover,
        gun: RailwayGunCounterInputRow,
        loc: &Localisation,
        arrival: impl FnOnce(&RailwayGunCounterInputRow, bool) -> Result<i64, String>,
    ) -> Tooltip {
        let mut frame = test_frame(20);
        put(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 4,
                ..Default::default()
            }]),
        );
        put(&mut frame, TableRows::RailwayGunCounterInputs(vec![gun]));
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 11,
                canonical_slot: 1,
            }]),
        );
        let names = content_reader_division_names::division_names::Database::default();
        let units = Err("unused division content".into());
        let vision = Vision::sharing(1, []);
        let context = Context {
            nato: false,
            diagnostics: true,
            content: None,
            reads: None,
            dig_in_factor: Err("unused".into()),
            loc,
            recorded_language: "english",
            icons: &NoIcons,
            names: &names,
            units: &units,
            supply_grace: Err("unused".into()),
            comms_max_distance: Err("unused".into()),
            frame: &frame,
            vision: &vision,
            links: frame
                .complete::<tables::CountryLinks>()
                .map_err(|error| error.to_string()),
        };
        build(&context, input, arrival).0
    }

    #[test]
    fn fresh_arrival_callback_runs_once_only_on_the_native_reached_branch() {
        for (ours, hidden, coarse, path, target, expected) in [
            (true, false, false, true, 0, 1),
            (true, true, false, true, 0, 1),
            (false, false, false, true, 0, 1),
            (false, true, false, true, 0, 0),
            (true, false, true, true, 0, 0),
            (true, false, false, false, 0, 0),
            (true, false, false, true, 4, 0),
        ] {
            let mut input = hover();
            input.ours = ours;
            input.hidden = hidden;
            input.coarse = coarse;
            let mut gun = gun();
            gun.target = target;
            if !path {
                gun.ordinary_path.clear();
            }
            let calls = Cell::new(0);
            let out = fixture(&input, gun, &localisation(), |gun, owner| {
                assert_eq!(gun.gun, 7);
                assert_eq!(owner, ours);
                calls.set(calls.get() + 1);
                Ok(100_001)
            });
            assert_eq!(calls.get(), expected);
            if expected != 0 {
                assert!(
                    out.body.starts_with(if ours {
                        "arrival 2h\nnext 2h"
                    } else {
                        "foreign arrival 2h\nnext 2h"
                    }),
                    "{}",
                    out.body
                );
            } else {
                assert!(!out.body.contains("arrival"), "{}", out.body);
            }
            assert!(
                !out.refused
                    .keys()
                    .any(|reason| reason.contains("client-UI path-cache"))
            );
        }
        let mut input = hover();
        input.members = vec![].into();
        let out = fixture(&input, gun(), &localisation(), |_, _| {
            panic!("empty counter queried")
        });
        assert!(out.body.is_empty());
    }

    #[test]
    fn arrival_refusal_or_artwork_failure_preserves_next_location_and_strength() {
        let input = hover();
        let out = fixture(&input, gun(), &localisation(), |_, _| {
            Err("fixture fresh domain refused".into())
        });
        assert!(
            out.refused
                .keys()
                .any(|reason| reason.contains("railway gun arrival: fixture fresh domain refused"))
        );
        assert_eq!(out.body, "next 2h\n\nstr 100%");
        let base = localisation();
        let loc: Localisation = base
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain([(
                "UNIT_MOVING_HOURS".to_owned(),
                "@MISSING $HOURS$".to_owned(),
            )])
            .collect();
        let out = fixture(&input, gun(), &loc, |_, _| Ok(100_001));
        assert!(
            out.refused
                .keys()
                .any(|reason| reason.contains("railway gun arrival"))
        );
        assert_eq!(out.body, "next 2h\n\nstr 100%");
        for (remaining, hours) in [(0, 0), (-100_000, -1)] {
            let out = fixture(&input, gun(), &localisation(), |_, _| Ok(remaining));
            assert_eq!(out.body, format!("arrival {hours}h\nnext 2h\n\nstr 100%"));
        }
    }
}
