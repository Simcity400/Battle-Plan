//! `1418C00D0` class 13 -> `140E74FB0`; coarse counts stay in `1418C00D0`.

use super::*;
use replay::rows::RailwayGunCounterInputRow;

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
    input
        .members
        .iter()
        .map(|id| {
            rows.binary_search_by_key(id, |row| row.gun)
                .map(|index| &rows[index])
                .map_err(|_| format!("RailwayGunCounterInputs gun {id} is absent"))
        })
        .collect()
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
    // 140E74FB0 uses the first original pointer, even when a different gun is
    // the sole known member. Country is the roster owner (+1D8), not logical owner.
    let country = context.country_name(context.slot(first.roster_owner)?)?;
    if count == 1 {
        let bytes = first
            .name
            .require("RailwayGunCounterInputs.name")?
            .as_ref()
            .ok_or("RailwayGunCounterInputs.name was unreadable when recorded")?;
        // Slot +68 -> 140BEDEB0 returns the cached CUnit+258 string. SetName
        // 140BF5460 records no provenance flag for arbitrary or generated text.
        let name = dashboard::scripted_text::stored_text(
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

pub(super) fn build(context: &Context<'_>, input: &CounterHover) -> Tooltip {
    let mut out = Tooltip::default();
    if input.members.is_empty() {
        return out;
    }
    let members = match members(context, input) {
        Ok(members) => members,
        Err(reason) => {
            refuse(&mut out, "railway gun inputs", reason);
            return out;
        }
    };
    let known = known(context, &members);
    if !input.coarse && !members[0].ordinary_path.is_empty() && members[0].target == 0 {
        if input.ours || !input.hidden {
            refuse(
                &mut out,
                "destination ETA",
                "native counter client-UI path-cache cost is unrecorded",
            );
        }
        let next = (|| {
            if input.timing.len() != members.len() {
                return Err("railway gun member timing is unavailable".into());
            }
            let timed: Vec<_> = members
                .iter()
                .zip(input.timing.iter())
                .map(|(gun, timing)| {
                    (
                        gun.gun,
                        gun.province,
                        gun.ordinary_path.len() as u32,
                        timing,
                    )
                })
                .collect();
            next_location(context.loc, next_hours(&timed)?)
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
            return out;
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
        return out;
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
            super::super::railway_gun_counter::strength_ratio(
                gun.current_strength,
                gun.maximum_strength,
            )
        }))
        .expect("known railway guns are nonempty");
        // 140E74FB0 appends no newline after this statistic.
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
    out
}
