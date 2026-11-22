mod movement;
mod travel;

use super::{Arg, Context, Localisation, Tooltip, localise, units};
use replay_data::{rows::Recorded, rows::tables};
use replay_frame::frame::DecodedFrame;
use crate::map::division_eta::CounterQueries;
use viewer_runtime::unit_handle::Unit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Army,
    Railway,
}

impl Class {
    fn plural(self) -> &'static str {
        match self {
            Self::Army => "DIVISION_PLURAL",
            Self::Railway => "RAILWAY_GUN_PLURAL",
        }
    }
}

fn gather(selected: &[Unit]) -> Option<(Class, Vec<u64>)> {
    let class = match selected.first()? {
        Unit::Division(_) => Class::Army,
        Unit::RailwayGun(_) => Class::Railway,
        Unit::TaskForce(_) => return None,
    };
    let members = selected
        .iter()
        .filter_map(|unit| match (class, unit) {
            (Class::Army, Unit::Division(handle)) | (Class::Railway, Unit::RailwayGun(handle)) => {
                Some(*handle)
            }
            _ => None,
        })
        .collect();
    Some((class, members))
}

pub(super) fn refuse(out: &mut Tooltip, name: &str, reason: String) {
    *out.refused
        .entry(format!("province selected {name}: {reason}"))
        .or_default() += 1;
}

pub(super) fn line(out: &mut Tooltip, name: &str, indent: usize, value: Result<String, String>) {
    match value {
        Ok(value) => {
            if !out.title.is_empty() && !out.title.ends_with('\n') {
                out.title.push('\n');
            }
            out.title.extend(std::iter::repeat_n(' ', indent * 4));
            out.title.push_str(&value);
        }
        Err(reason) => refuse(out, name, reason),
    }
}

fn count_text(loc: &Localisation, class: Class, key: &str, count: i32) -> Result<String, String> {
    let plural = localise(loc, class.plural(), &[])?;
    localise(
        loc,
        key,
        &[
            ("UNIT_TYPE_PLURAL", Arg::Text(&plural)),
            ("NUM", Arg::Int32(count)),
        ],
    )
}

fn add_count(count: &mut Result<i32, String>, present: Result<bool, String>) {
    match present {
        Ok(true) => {
            if let Ok(value) = count {
                *count = value
                    .checked_add(1)
                    .ok_or_else(|| "selected count exceeds i32".into());
            }
        }
        Ok(false) => {}
        Err(reason) if count.is_ok() => *count = Err(reason),
        Err(_) => {}
    }
}

fn border_presence(frame: &DecodedFrame, handle: u64) -> Result<bool, String> {
    let divisions = frame
        .complete::<tables::Divisions>()
        .map_err(|e| e.to_string())?;
    let index = divisions
        .binary_search_by_key(&handle, |row| row.id)
        .map_err(|_| format!("Divisions member {handle} is absent"))?;
    let group = divisions[index].orders_group;
    if group == 0 {
        return Ok(false);
    }
    let groups = frame
        .complete::<tables::OrdersGroupCounterInputs>()
        .map_err(|e| e.to_string())?;
    let index = groups
        .binary_search_by_key(&group, |row| row.group)
        .map_err(|_| format!("OrdersGroupCounterInputs group {group} is absent"))?;
    let present = groups[index]
        .border_war_presence
        .require("OrdersGroupCounterInputs.border_war_presence")?
        .ok_or("OrdersGroupCounterInputs.border_war_presence was unreadable when recorded")?;
    Ok(present != 0)
}

fn border(out: &mut Tooltip, frame: &DecodedFrame, members: &[u64], loc: &Localisation) {
    let mut count = Ok(0);
    let mut positive = false;
    for &handle in members {
        let present = border_presence(frame, handle);
        positive |= present.as_ref().is_ok_and(|present| *present);
        add_count(&mut count, present);
    }
    match &count {
        Ok(count) if *count > 0 => line(
            out,
            "border-war count",
            1,
            localise(
                loc,
                "SOME_UNITS_IN_BORDER_WAR",
                &[("NUM", Arg::Int32(*count))],
            ),
        ),
        Err(reason) => refuse(out, "border-war count", reason.clone()),
        Ok(_) => {}
    }
    if positive {
        line(
            out,
            "border-war description",
            1,
            localise(loc, "SOME_UNITS_IN_BORDER_WAR_DESC", &[]),
        );
    } else if let Err(reason) = count {
        refuse(out, "border-war description", reason);
    }
}

fn status(
    frame: &DecodedFrame,
    class: Class,
    handle: u64,
) -> Result<(u8, &Recorded<Option<u8>>, &'static str), String> {
    match class {
        Class::Army => {
            let rows = frame
                .complete::<tables::DivisionMovement>()
                .map_err(|e| e.to_string())?;
            let index = rows
                .binary_search_by_key(&handle, |row| row.division)
                .map_err(|_| format!("DivisionMovement member {handle} is absent"))?;
            Ok((
                rows[index].retreat,
                &rows[index].withdrawal,
                "DivisionMovement.withdrawal",
            ))
        }
        Class::Railway => {
            let rows = frame
                .complete::<tables::RailwayGunCounterInputs>()
                .map_err(|e| e.to_string())?;
            let index = rows
                .binary_search_by_key(&handle, |row| row.gun)
                .map_err(|_| format!("RailwayGunCounterInputs member {handle} is absent"))?;
            Ok((
                rows[index].retreat,
                &rows[index].withdrawal,
                "RailwayGunCounterInputs.withdrawal",
            ))
        }
    }
}

fn statuses(
    out: &mut Tooltip,
    frame: &DecodedFrame,
    class: Class,
    members: &[u64],
    loc: &Localisation,
) {
    let mut retreat = Ok(0);
    let mut withdrawal = Ok(0);
    for &handle in members {
        match status(frame, class, handle) {
            Ok((byte, _, _)) if byte != 0 => add_count(&mut retreat, Ok(true)),
            Ok((_, byte, field)) => {
                let present = byte
                    .require(field)
                    .and_then(|byte| {
                        byte.ok_or_else(|| format!("{field} was unreadable when recorded"))
                    })
                    .map(|byte| byte != 0);
                add_count(&mut withdrawal, present);
            }
            Err(reason) => {
                add_count(&mut retreat, Err(reason.clone()));
                add_count(&mut withdrawal, Err(reason));
            }
        }
    }
    for (name, key, count) in [
        ("retreat count", "PROV_TOOLTIP_DIVS_RETREATING", retreat),
        (
            "withdrawal count",
            "PROV_TOOLTIP_DIVS_WITHDRAWING",
            withdrawal,
        ),
    ] {
        match count {
            Ok(count) if count > 0 => line(out, name, 1, count_text(loc, class, key, count)),
            Err(reason) => refuse(out, name, reason),
            Ok(_) => {}
        }
    }
}

fn location(frame: &DecodedFrame, class: Class, handle: u64) -> Result<u32, String> {
    travel::source(
        frame,
        match class {
            Class::Army => Unit::Division(handle),
            Class::Railway => Unit::RailwayGun(handle),
        },
    )
    .map(|province| province.unwrap_or(0))
}

fn preview_gaps(
    out: &mut Tooltip,
    frame: &DecodedFrame,
    class: Class,
    members: &[u64],
    target: u32,
    travel: &travel::SelectionTravel,
) {
    if class == Class::Railway {
        return;
    }
    let locations: Vec<_> = members
        .iter()
        .map(|&handle| location(frame, class, handle))
        .collect();
    let at_target = locations
        .iter()
        .any(|source| source.as_ref().is_ok_and(|source| *source == target));
    let positive = travel.hours.as_ref().is_ok_and(|hours| *hours > 0);
    if !at_target && !positive {
        if !travel.route.as_ref().is_ok_and(|route| !route.is_empty()) {
            let notice = match class {
                Class::Army => "no-time notice",
                Class::Railway => "no-railway notice",
            };
            refuse(
                out,
                notice,
                "prospective rejection/route gate is unavailable".into(),
            );
        }
        if class == Class::Army {
            refuse(
                out,
                "neutral-attack notice",
                "prospective travel/relation gate is unavailable".into(),
            );
        }
    }
}

struct RailwaySecondGate<'a> {
    travel: &'a travel::SelectionTravel,
    rejection: &'a Result<bool, String>,
}

fn railway_second(
    out: &mut Tooltip,
    members: &[u64],
    target: u32,
    gate: RailwaySecondGate<'_>,
    loc: &Localisation,
    mut source: impl FnMut(u64) -> Result<u32, String>,
    query: impl FnOnce() -> Result<content_reader_unit_path::unit_path::selected_transport::Outputs, String>,
) {
    let RailwaySecondGate { travel, rejection } = gate;
    let needed = (|| {
        if *rejection.as_ref().map_err(Clone::clone)? {
            return Ok(false);
        }
        if *travel.hours.as_ref().map_err(Clone::clone)? > 0
            || !travel.route.as_ref().map_err(Clone::clone)?.is_empty()
        {
            return Ok(false);
        }
        for &handle in members {
            if source(handle)? == target {
                return Ok(false);
            }
        }
        Ok::<_, String>(true)
    })();
    match needed {
        Ok(false) => {}
        Err(reason) => refuse(out, "railway second transport gate", reason),
        Ok(true) => {
            line(
                out,
                "no-railway notice",
                1,
                localise(loc, "PROVINCE_UNIT_MOVE_NO_RAILWAY", &[]),
            );
            match query() {
                Ok(outputs) => {
                    let mut bytes = std::mem::take(&mut out.title).into_bytes();
                    content_reader_unit_path::unit_path::selected_transport::append_railway_second(
                        &mut bytes, &outputs,
                    );
                    out.title =
                        String::from_utf8(bytes).expect("transport localisation preserves UTF-8");
                }
                Err(reason) => refuse(out, "railway second transport", reason),
            }
        }
    }
}

pub(super) fn build(context: &Context<'_>, selected: &[Unit], nato: bool) -> Tooltip {
    let mut out = Tooltip::default();
    let Some((class, members)) = gather(selected) else {
        return out;
    };
    let loc = &context.content.localisation;
    let header = localise(loc, "PROV_TOOLTIP_SELECTION", &[])
        .and_then(|header| localise(loc, "HEADER", &[("KEY", Arg::Text(&header))]));
    line(&mut out, "header", 0, header);
    let count = i32::try_from(members.len())
        .map_err(|_| "selected count exceeds i32".into())
        .and_then(|count| count_text(loc, class, "PROV_TOOLTIP_SELECTION_NUM_DIVS", count));
    line(&mut out, "count", 1, count);
    if class == Class::Army {
        let canonical = |raw| context.links.get(&raw).copied();
        let composition = context
            .content
            .province_hover
            .units
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|content| {
                let prepared =
                    units::prepare_members(context.frame, &members, &canonical, context.vision)?;
                units::summary_prepared(
                    content,
                    context.frame,
                    &prepared,
                    &canonical,
                    context.vision,
                    loc,
                    nato,
                )
            });
        match composition {
            Ok(value) if !value.is_empty() => line(
                &mut out,
                "composition",
                1,
                Ok(format!("\u{11}Y{value}\u{11}! \n")),
            ),
            Err(reason) => refuse(&mut out, "composition", reason),
            Ok(_) => {}
        }
    }
    let queries = CounterQueries::new(context.content, context.reads, context.frame);
    let travel = travel::build(context, class, selected, &queries);
    travel::append(&mut out, loc, class, &travel);
    if class == Class::Army {
        border(&mut out, context.frame, &members, loc);
    }
    preview_gaps(
        &mut out,
        context.frame,
        class,
        &members,
        context.province.id,
        &travel,
    );
    match queries.selected_transport_message(selected, context.province.id) {
        Ok(outputs) => {
            let mut bytes = std::mem::take(&mut out.title).into_bytes();
            content_reader_unit_path::unit_path::selected_transport::append_first(&mut bytes, &outputs);
            out.title = String::from_utf8(bytes).expect("transport localisation preserves UTF-8");
        }
        Err(reason) => refuse(&mut out, "transport", reason),
    }
    let rejection = movement::append(&mut out, class, &members, context.province.id, &queries);
    if class == Class::Railway {
        railway_second(
            &mut out,
            &members,
            context.province.id,
            RailwaySecondGate {
                travel: &travel,
                rejection: &rejection,
            },
            loc,
            |handle| location(context.frame, Class::Railway, handle),
            || queries.selected_transport_message(selected, context.province.id),
        );
    }
    statuses_then_attack(&mut out, context.frame, class, &members, loc, |out| {
        super::selected_attack::append(out, context, &members, &travel.route, &queries);
    });
    out
}

fn statuses_then_attack(
    out: &mut Tooltip,
    frame: &DecodedFrame,
    class: Class,
    members: &[u64],
    loc: &Localisation,
    attack: impl FnOnce(&mut Tooltip),
) {
    statuses(out, frame, class, members, loc);
    if class == Class::Army {
        attack(out);
    }
}

pub(super) fn append(out: &mut Tooltip, selected: Tooltip, loc: &Localisation) {
    for (name, count) in selected.refused {
        *out.refused.entry(name).or_default() += count;
    }
    if out.title.is_empty() || selected.title.is_empty() {
        return;
    }
    match localise(loc, "SEPARATION_LINE", &[]) {
        Ok(separator) => {
            if !out.title.trim_end_matches(' ').ends_with('\n') {
                out.title.push('\n');
            }
            out.title.push_str(&separator);
            out.title.push_str(&selected.title);
        }
        Err(reason) => refuse(out, "separator", reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::DivisionMovementRow, rows::DivisionRow, rows::OrdersGroupCounterInputRow,
        rows::RailwayGunCounterInputRow, rows::TableRows,
    };
    use viewer_foundation::test_support::{test_frame, test_put as put};

    fn loc() -> Localisation {
        [
            ("DIVISION_PLURAL", "Armies"),
            ("RAILWAY_GUN_PLURAL", "Guns"),
            (
                "PROV_TOOLTIP_DIVS_RETREATING",
                "$NUM$ $UNIT_TYPE_PLURAL$ retreat",
            ),
            (
                "PROV_TOOLTIP_DIVS_WITHDRAWING",
                "$NUM$ $UNIT_TYPE_PLURAL$ withdraw",
            ),
            ("SOME_UNITS_IN_BORDER_WAR", "$NUM$ border"),
            ("SOME_UNITS_IN_BORDER_WAR_DESC", "Border description"),
            ("SEPARATION_LINE", "<separator>"),
            ("LAND_COMBAT_ATTACK_PROVINCE_MODIFIERS", "Attack $DIVISION$"),
            ("UNIT_MOVEMENT_BLOCKED_BY_STRAIT", "Blocked strait"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    }

    #[test]
    fn first_class_dispatch_preserves_encounter_order_across_typed_handles() {
        assert_eq!(
            gather(&[Unit::RailwayGun(7), Unit::Division(7), Unit::RailwayGun(2)]),
            Some((Class::Railway, vec![7, 2]))
        );
        assert_eq!(
            gather(&[Unit::Division(7), Unit::RailwayGun(7), Unit::Division(2)]),
            Some((Class::Army, vec![7, 2]))
        );
        assert_eq!(gather(&[Unit::TaskForce(7), Unit::Division(2)]), None);
        assert_eq!(gather(&[]), None);
    }

    #[test]
    fn successful_append_indents_once_and_preserves_exact_colour_suffix_and_empty_values() {
        let mut out = Tooltip::default();
        line(&mut out, "heading", 0, Ok("Heading\n".into()));
        line(
            &mut out,
            "composition",
            1,
            Ok("\u{11}Yfirst\nsecond\u{11}! \n".into()),
        );
        line(&mut out, "empty", 1, Ok(String::new()));
        assert_eq!(out.title, "Heading\n    \u{11}Yfirst\nsecond\u{11}! \n    ");
    }

    #[test]
    fn outer_byte_length_gate_and_separator_preserve_authored_newlines_and_refusals() {
        let selected = || Tooltip {
            title: "Selection".into(),
            refused: [("gap".into(), 1)].into(),
            ..Default::default()
        };
        let mut empty = Tooltip::default();
        append(&mut empty, selected(), &loc());
        assert!(empty.title.is_empty());
        assert_eq!(empty.refused["gap"], 1);
        let mut out = Tooltip {
            title: "Location\n   ".into(),
            ..Default::default()
        };
        append(&mut out, selected(), &loc());
        assert_eq!(out.title, "Location\n   <separator>Selection");
        let with_lf: Localisation = [("SEPARATION_LINE".into(), "<separator>\n".into())]
            .into_iter()
            .collect();
        let mut out = Tooltip {
            title: "Location ".into(),
            ..Default::default()
        };
        append(&mut out, selected(), &with_lf);
        assert_eq!(out.title, "Location \n<separator>\nSelection");
    }

    #[test]
    fn border_description_survives_known_positive_and_unknown_exact_count() {
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::Divisions(vec![
                DivisionRow {
                    id: 1,
                    orders_group: 10,
                    ..Default::default()
                },
                DivisionRow {
                    id: 2,
                    orders_group: 20,
                    ..Default::default()
                },
                DivisionRow {
                    id: 3,
                    orders_group: 0,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::OrdersGroupCounterInputs(vec![
                OrdersGroupCounterInputRow {
                    group: 10,
                    border_war_presence: Recorded::Value(Some(2)),
                    ..Default::default()
                },
                OrdersGroupCounterInputRow {
                    group: 20,
                    border_war_presence: Recorded::NotRecorded,
                    ..Default::default()
                },
            ]),
        );
        assert_eq!(border_presence(&frame, 3), Ok(false));
        let mut out = Tooltip::default();
        border(&mut out, &frame, &[1, 2, 3], &loc());
        assert_eq!(out.title, "    Border description");
        assert_eq!(out.refused.len(), 1);
        assert!(
            out.refused
                .keys()
                .next()
                .unwrap()
                .contains("border-war count")
        );
    }

    #[test]
    fn retreat_does_not_demand_its_withdrawal_and_withdrawal_refusal_preserves_retreat_text() {
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::DivisionMovement(vec![
                DivisionMovementRow {
                    division: 1,
                    retreat: 2,
                    withdrawal: Recorded::NotRecorded,
                    ..Default::default()
                },
                DivisionMovementRow {
                    division: 2,
                    retreat: 0,
                    withdrawal: Recorded::Value(None),
                    ..Default::default()
                },
            ]),
        );
        let mut out = Tooltip::default();
        statuses(&mut out, &frame, Class::Army, &[1], &loc());
        assert_eq!(out.title, "    1 Armies retreat");
        assert!(out.refused.is_empty());
        let mut out = Tooltip::default();
        statuses(&mut out, &frame, Class::Army, &[1, 2], &loc());
        assert_eq!(out.title, "    1 Armies retreat");
        assert!(
            out.refused
                .keys()
                .any(|key| key.contains("withdrawal count"))
        );
    }

    #[test]
    fn railway_status_counts_use_nonzero_bytes_and_its_own_plural() {
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![RailwayGunCounterInputRow {
                gun: 7,
                withdrawal: Recorded::Value(Some(3)),
                ..Default::default()
            }]),
        );
        let mut out = Tooltip::default();
        statuses(&mut out, &frame, Class::Railway, &[7], &loc());
        assert_eq!(out.title, "    1 Guns withdraw");
        assert!(out.refused.is_empty());
    }

    fn mixed_status_frame() -> DecodedFrame {
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::DivisionMovement(vec![
                DivisionMovementRow {
                    division: 1,
                    retreat: 1,
                    withdrawal: Recorded::NotRecorded,
                    ..Default::default()
                },
                DivisionMovementRow {
                    division: 2,
                    withdrawal: Recorded::Value(Some(1)),
                    ..Default::default()
                },
            ]),
        );
        frame
    }

    #[test]
    fn selection_status_counts_precede_attack_heading_and_modifiers() {
        let frame = mixed_status_frame();
        let loc = loc();
        let mut out = Tooltip {
            title: "Selection".into(),
            ..Default::default()
        };
        statuses_then_attack(&mut out, &frame, Class::Army, &[1, 2], &loc, |out| {
            line(
                out,
                "attack heading",
                1,
                localise(
                    &loc,
                    "LAND_COMBAT_ATTACK_PROVINCE_MODIFIERS",
                    &[("DIVISION", Arg::Text("Template"))],
                ),
            );
            line(
                out,
                "attack modifier",
                2,
                Ok("Terrain: \u{11}R-10.00%\u{11}!".into()),
            );
            refuse(out, "attack weather", "weather operand unavailable".into());
        });
        assert_eq!(
            out.title,
            "Selection\n    1 Armies retreat\n    1 Armies withdraw\n    Attack Template\n        Terrain: \u{11}R-10.00%\u{11}!"
        );
        assert_eq!(
            out.refused["province selected attack weather: weather operand unavailable"],
            1
        );
    }

    #[test]
    fn selection_status_counts_precede_blocked_strait_notice() {
        let frame = mixed_status_frame();
        let loc = loc();
        let mut out = Tooltip {
            title: "Selection".into(),
            ..Default::default()
        };
        statuses_then_attack(&mut out, &frame, Class::Army, &[1, 2], &loc, |out| {
            line(
                out,
                "blocked strait",
                1,
                localise(&loc, "UNIT_MOVEMENT_BLOCKED_BY_STRAIT", &[]),
            );
        });
        assert_eq!(
            out.title,
            "Selection\n    1 Armies retreat\n    1 Armies withdraw\n    Blocked strait"
        );
        assert!(out.refused.is_empty());
    }

    #[test]
    fn refused_status_count_keeps_attack_text_and_railway_skips_attack_demand() {
        let mut out = Tooltip::default();
        statuses_then_attack(
            &mut out,
            &test_frame(42),
            Class::Army,
            &[99],
            &loc(),
            |out| {
                line(out, "attack heading", 1, Ok("Known attack".into()));
            },
        );
        assert_eq!(out.title, "    Known attack");
        assert_eq!(out.refused.len(), 2);
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![RailwayGunCounterInputRow {
                gun: 7,
                retreat: 1,
                ..Default::default()
            }]),
        );
        let mut out = Tooltip::default();
        statuses_then_attack(&mut out, &frame, Class::Railway, &[7], &loc(), |_| {
            panic!("railway attack queried")
        });
        assert_eq!(out.title, "    1 Guns retreat");
        assert!(out.refused.is_empty());
    }

    #[test]
    fn known_target_and_mixed_sources_skip_unreachable_notices_without_guessing_travel() {
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::Divisions(vec![
                DivisionRow {
                    id: 1,
                    province: 5,
                    ..Default::default()
                },
                DivisionRow {
                    id: 2,
                    province: 6,
                    ..Default::default()
                },
            ]),
        );
        let mut out = Tooltip::default();
        preview_gaps(
            &mut out,
            &frame,
            Class::Army,
            &[1, 2, 3],
            5,
            &travel::SelectionTravel {
                hours: Err("search".into()),
                route: Err("search".into()),
            },
        );
        assert!(out.refused.is_empty());
        assert!(out.title.is_empty());
        assert!(
            !out.refused
                .keys()
                .any(|key| key.contains("notice") || key.contains("attack preview"))
        );
    }

    #[test]
    fn positive_travel_or_retained_route_suppresses_only_its_proven_notice_gates() {
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 1,
                province: 5,
                ..Default::default()
            }]),
        );
        let mut positive = Tooltip::default();
        preview_gaps(
            &mut positive,
            &frame,
            Class::Army,
            &[1],
            9,
            &travel::SelectionTravel {
                hours: Ok(1),
                route: Err("route".into()),
            },
        );
        assert!(!positive.refused.keys().any(|key| key.contains("notice")));
        assert!(
            !positive
                .refused
                .keys()
                .any(|key| key.contains("attack preview"))
        );
        let mut route = Tooltip::default();
        preview_gaps(
            &mut route,
            &frame,
            Class::Army,
            &[1],
            9,
            &travel::SelectionTravel {
                hours: Err("speed".into()),
                route: Ok(vec![9]),
            },
        );
        assert!(
            !route
                .refused
                .keys()
                .any(|key| key.contains("no-time notice"))
        );
        assert!(
            route
                .refused
                .keys()
                .any(|key| key.contains("neutral-attack notice"))
        );
        assert!(
            !route
                .refused
                .keys()
                .any(|key| key.contains("attack preview"))
        );
        let mut empty = Tooltip::default();
        preview_gaps(
            &mut empty,
            &frame,
            Class::Army,
            &[1],
            9,
            &travel::SelectionTravel {
                hours: Ok(0),
                route: Ok(vec![]),
            },
        );
        assert!(empty.title.is_empty());
        assert!(
            empty
                .refused
                .keys()
                .any(|key| key.contains("no-time notice"))
        );
        assert!(
            empty
                .refused
                .keys()
                .any(|key| key.contains("neutral-attack notice"))
        );
    }

    #[test]
    fn railway_second_rejection_travel_and_route_gates_skip_later_unknown_inputs() {
        for (rejection, hours, route) in [
            (
                Ok(true),
                Err("unreached hours".into()),
                Err("unreached route".into()),
            ),
            (Ok(false), Ok(1), Err("unreached route".into())),
            (Ok(false), Ok(0), Ok(vec![9])),
        ] {
            let mut out = Tooltip::default();
            railway_second(
                &mut out,
                &[7],
                9,
                RailwaySecondGate {
                    travel: &travel::SelectionTravel { hours, route },
                    rejection: &rejection,
                },
                &Localisation::default(),
                |_| panic!("location is after the completed native gate"),
                || panic!("suppressed second transport invocation"),
            );
            assert!(out.title.is_empty() && out.refused.is_empty());
        }
        for (rejection, hours, route) in [
            (Err("rejection".into()), Ok(0), Ok(vec![])),
            (Ok(false), Err("hours".into()), Ok(vec![])),
            (Ok(false), Ok(0), Err("route".into())),
        ] {
            let mut out = Tooltip::default();
            railway_second(
                &mut out,
                &[7],
                9,
                RailwaySecondGate {
                    travel: &travel::SelectionTravel { hours, route },
                    rejection: &rejection,
                },
                &Localisation::default(),
                |_| panic!("unknown gate cannot demand later location"),
                || panic!("unknown gate cannot run the second transport call"),
            );
            assert!(out.title.is_empty());
            assert_eq!(out.refused.values().sum::<usize>(), 1);
            assert!(
                out.refused
                    .keys()
                    .all(|key| key.contains("second transport gate"))
            );
        }
    }

    #[test]
    fn railway_second_target_scan_preserves_occurrences_and_stops_at_first_match_or_fault() {
        let travel = travel::SelectionTravel {
            hours: Ok(-1),
            route: Ok(vec![]),
        };
        let mut seen = Vec::new();
        let mut out = Tooltip::default();
        railway_second(
            &mut out,
            &[7, 7, 8, 10],
            9,
            RailwaySecondGate {
                travel: &travel,
                rejection: &Ok(false),
            },
            &Localisation::default(),
            |handle| {
                seen.push(handle);
                Ok(if handle == 8 { 9 } else { 2 })
            },
            || panic!("gun already at target suppresses second call"),
        );
        assert_eq!(seen, [7, 7, 8]);
        assert!(out.title.is_empty() && out.refused.is_empty());
        let mut seen = Vec::new();
        railway_second(
            &mut out,
            &[7, 8],
            9,
            RailwaySecondGate {
                travel: &travel,
                rejection: &Ok(false),
            },
            &Localisation::default(),
            |handle| {
                seen.push(handle);
                Err("location unreadable".into())
            },
            || panic!("unknown actual location cannot admit the second call"),
        );
        assert_eq!(seen, [7]);
        assert_eq!(out.refused.values().sum::<usize>(), 1);
    }

    #[test]
    fn railway_second_runs_fresh_call_after_notice_and_appends_only_raw_primary() {
        let travel = travel::SelectionTravel {
            hours: Ok(0),
            route: Ok(vec![]),
        };
        let loc: Localisation = [("PROVINCE_UNIT_MOVE_NO_RAILWAY".into(), "No railway".into())]
            .into_iter()
            .collect();
        let mut calls = 0;
        let mut out = Tooltip {
            title: "Prior".into(),
            ..Default::default()
        };
        railway_second(
            &mut out,
            &[7],
            9,
            RailwaySecondGate {
                travel: &travel,
                rejection: &Ok(false),
            },
            &loc,
            |_| Ok(2),
            || {
                calls += 1;
                Ok(
                    content_reader_unit_path::unit_path::selected_transport::Outputs {
                        primary: b"\nRisk\ncontinued".to_vec(),
                        secondary: b"unappended".to_vec(),
                    },
                )
            },
        );
        assert_eq!(calls, 1);
        assert_eq!(out.title, "Prior\n    No railway \nRisk\ncontinued");
        assert!(out.refused.is_empty());
        railway_second(
            &mut out,
            &[7],
            9,
            RailwaySecondGate {
                travel: &travel,
                rejection: &Ok(false),
            },
            &loc,
            |_| Ok(2),
            || Err("nested risk unavailable".into()),
        );
        assert!(out.title.ends_with("\n    No railway"));
        assert_eq!(out.refused.values().sum::<usize>(), 1);
        assert!(
            out.refused
                .keys()
                .all(|key| key.contains("railway second transport:"))
        );
    }
}
