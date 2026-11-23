use super::*;
use content_reader_parser::parser::c_string;
use replay_data::rows::Recorded;

mod directed_wars;
mod forward_visibility;

pub fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::TaskForceCounterInputs, |r| {
        task_forces(frame, save, r)
    });
    report.attempt_table(TableId::TaskForceMapMotion, |r| {
        task_force_motion(frame, save, r)
    });
    report.attempt("TaskForceMapMotion fresh", |r| {
        task_force_fresh(frame, save, r)
    });
    report.attempt_table(TableId::TaskForceMissions, |r| {
        task_force_missions(frame, save, r)
    });
    super::naval_missions::compare(frame, save, report);
    report.attempt_table(TableId::FleetCounterInputs, |r| fleets(frame, save, r));
    report.attempt_table(TableId::CountryCounterInputs, |r| countries(frame, save, r));
    report.attempt("CountryCounterInputs forward visibility", |r| {
        forward_visibility::compare(frame, r)
    });
    report.attempt("CountryCounterInputs directed wars", |r| {
        directed_wars::compare(frame, r)
    });
    report.attempt_table(TableId::OrdersGroupCounterInputs, |r| {
        groups(frame, save, r)
    });
    report.attempt_table(TableId::LeaderPortraitPaths, |r| {
        leader_paths(frame, save, r)
    });
}

fn task_force_missions(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::TaskForceMissions>()?;
    let forces: BTreeSet<u64> = frame
        .complete::<tables::TaskForces>()?
        .iter()
        .map(|row| row.id)
        .collect();
    let links: BTreeSet<u32> = frame
        .complete::<tables::CountryLinks>()?
        .iter()
        .map(|link| link.id)
        .collect();
    let saved = &save.counter_inputs.task_forces;
    let mut listed = BTreeSet::new();
    for row in rows {
        let label = format!("TaskForceMissions {}", row.task_force);
        if !forces.contains(&row.task_force) {
            return Err(invalid(format!("{label} names no task force")));
        }
        if !links.contains(&row.owner) {
            return Err(invalid(format!(
                "{label} owner {} is unresolved",
                row.owner
            )));
        }
        if !listed.insert(row.task_force) {
            r.difference(label, "a repeated row", "one row");
            continue;
        }
        let Some(expected) = saved.get(&row.task_force).filter(|expected| {
            expected.spotting_region.is_some() || expected.accessible_regions.is_some()
        }) else {
            r.difference(
                label,
                "present",
                "absent: no saved spotting_region or accessible_regions",
            );
            continue;
        };
        eq(
            r,
            &format!("{label}.current_region"),
            (row.current_region != 0).then_some(row.current_region),
            expected.spotting_region,
        );
        eq(
            r,
            &format!("{label}.regions"),
            (!row.regions.is_empty()).then_some(row.regions.as_slice()),
            expected.accessible_regions.as_deref(),
        );
    }
    for (&task_force, expected) in saved {
        if (expected.spotting_region.is_some() || expected.accessible_regions.is_some())
            && !listed.contains(&task_force)
        {
            r.difference(
                format!("TaskForceMissions {task_force}"),
                "absent",
                "saved spotting_region or accessible_regions",
            );
        }
    }
    r.compared("TaskForceMissions", rows.len());
    r.limitation(
        "the game does not write the mission owner whose strategic navy registers the mission; it is checked only to resolve to a recorded country",
        "TaskForceMissions.owner",
    );
    Ok(())
}

fn task_forces(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::TaskForceCounterInputs>()?;
    eq(
        r,
        "TaskForceCounterInputs TaskForces census",
        rows.iter()
            .map(|row| row.task_force)
            .collect::<BTreeSet<_>>(),
        frame
            .complete::<tables::TaskForces>()?
            .iter()
            .map(|row| row.id)
            .collect(),
    );
    let saved = &save.counter_inputs.task_forces;
    let mut unresolved_links = 0;
    let forces: BTreeMap<_, _> = frame
        .complete::<tables::TaskForces>()?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    for row in rows {
        let label = format!("TaskForceCounterInputs {}", row.task_force);
        let supply_label = format!("{label}.supply");
        if let Some(supply) =
            r.recorded(&row.supply, "TaskForceCounterInputs.supply", &supply_label)
        {
            match supply {
                None => r.difference(&supply_label, "capture refused", "native supply inputs"),
                Some(supply) => {
                    r.limitation(
                        "the task force save writer and the unit save writer establish no save path for this supply-consumer footer pack, stored at the footer's display step; asked/received, state/air-drop receipts, demand pairs, disruption and ordered origins require matched native memory and hovers",
                        &supply_label,
                    );
                    if supply.origins.is_none() {
                        r.difference(
                            format!("{supply_label}.origins"),
                            "capture refused",
                            "native origin vector",
                        );
                    }
                }
            }
        }
        let name_label = format!("{label}.name");
        match &row.name {
            Recorded::NotRecorded => {
                r.limitation("TaskForceCounterInputs.name not recorded", &name_label);
            }
            Recorded::Value(None) => {
                r.difference(&name_label, "capture refused", "native cached name");
            }
            Recorded::Value(Some(name)) => match save.task_forces.get(&row.task_force) {
                Some(expected) => {
                    let text = c_string(name);
                    eq(
                        r,
                        &name_label,
                        (!name.is_empty()).then_some(text),
                        expected.name.as_deref(),
                    );
                    if text.len() != name.len() {
                        r.limitation(
                            "unit writer name text serialization stops at the first NUL; captured suffix bytes are not save-comparable",
                            &name_label,
                        );
                    }
                }
                None => r.difference(&name_label, "task force present", "absent from save"),
            },
        }
        let Some(expected) = saved.get(&row.task_force) else {
            r.difference(label, "present", "absent");
            continue;
        };
        if let Some(force) = forces.get(&row.task_force) {
            task_force_port(frame.hour, row, force, expected, r);
        }
        eq(
            r,
            &format!("{label}.is_in_regions"),
            row.is_in_regions != 0,
            expected.is_in_regions.unwrap_or(false),
        );
        eq(
            r,
            &format!("{label}.engagement_rule"),
            Some(row.engagement_rule),
            expected.engagement_rule,
        );
        eq(
            r,
            &format!("{label}.bombardment_region"),
            (row.bombardment_region != 0).then_some(row.bombardment_region),
            expected.bombardment_region,
        );
        eq(
            r,
            &format!("{label}.mission_region_count"),
            (row.mission_region_count > 0).then_some(row.mission_region_count as usize),
            expected.accessible_regions.as_ref().map(Vec::len),
        );
        let activity = match expected.detached_activity.as_deref() {
            None => Some(0),
            Some(b"repairing") => Some(1),
            Some(b"moving_to_refit") => Some(2),
            Some(b"refitting") => Some(3),
            Some(b"reinforcing") => Some(4),
            Some(_) => None,
        };
        eq(
            r,
            &format!("{label}.detached_activity"),
            (1..=4)
                .contains(&row.detached_activity)
                .then_some(row.detached_activity)
                .or(Some(0)),
            activity,
        );
        eq(
            r,
            &format!("{label}.repair_last_mission"),
            (row.repair_last_mission != 0).then_some(row.repair_last_mission),
            expected.repair_last_mission,
        );
        eq(
            r,
            &format!("{label}.attached_headquarters"),
            usize::try_from(row.attached_headquarters).ok(),
            Some(expected.attached_headquarters),
        );
        match expected.repair_parent {
            Some(handle) => eq(
                r,
                &format!("{label}.repair_parent"),
                row.repair_parent,
                handle,
            ),
            None if row.repair_parent != 0 => unresolved_links += 1,
            None => {}
        }
    }
    eq(
        r,
        "TaskForceCounterInputs saved task-force census",
        rows.len(),
        saved.len(),
    );
    if unresolved_links != 0 {
        r.limitation("the game writes repair_parent only when the game resolves the nonzero reference; nonzero raw references without a saved field did not resolve at save time and are not compared", format!("TaskForceCounterInputs.repair_parent: {unresolved_links} rows"));
    }
    r.compared("TaskForceCounterInputs", rows.len());
    r.limitation("the running-away byte is not written by the mission or task-force writer; is_in_regions is saved only as truth, and detached_activity values outside 1..4 are omitted like 0", "TaskForceCounterInputs running_away, is_in_regions byte value, detached_activity outside 1..4");
    r.limitation("the task force writer does not save the supply-consumer received-supply ratio; compare it against paused memory", "TaskForceCounterInputs.received_supply_ratio");
    Ok(())
}

fn task_force_port(
    hour: replay_data::Hour,
    row: &replay_data::rows::TaskForceCounterInputRow,
    force: &replay_data::rows::TaskForceRow,
    saved: &content_reader_save::counter_inputs::TaskForce,
    r: &mut Report,
) {
    let label = format!("TaskForceCounterInputs {}.port_mission", row.task_force);
    let Some(packet) = r.recorded(
        &row.port_mission,
        "TaskForceCounterInputs.port_mission",
        &label,
    ) else {
        return;
    };
    let Some(packet) = packet else {
        r.difference(&label, "capture refused", "native port mission packet");
        return;
    };
    r.packet_fresh(&label, packet.sample_hour, hour);
    match &packet.searching {
        None => r.difference(
            format!("{label}.searching"),
            "capture refused",
            "native searching vector",
        ),
        Some(searching) => {
            eq(
                r,
                &format!("{label}.searching.len"),
                searching.len(),
                saved.spotters.len(),
            );
            for (at, searcher) in searching.iter().enumerate() {
                let field = format!("{label}.searching[{at}]");
                match searcher.reference {
                    Some(reference) => {
                        eq(r, &field, Some(reference), saved.spotters.get(at).copied())
                    }
                    None => r.difference(
                        &field,
                        "raw reference capture refused",
                        "saved raw reference",
                    ),
                }
                if searcher.reference.is_some() && searcher.resolves.is_none() {
                    r.difference(
                        format!("{field}.resolves"),
                        "capture refused",
                        "native resolution",
                    );
                }
                if searcher.resolves == Some(true) && searcher.roster_owner.is_none() {
                    r.difference(
                        format!("{field}.roster_owner"),
                        "capture refused",
                        "native roster owner",
                    );
                }
            }
            if !searching.is_empty() {
                r.limitation("the saved spotters list contains raw references only; unit resolution and roster owner need an independent matched native/write witness, still pending", format!("{label}.searching resolution/roster_owner"));
            }
        }
    }
    let strike = force.mission == 2 && row.detached_activity == 0 && force.combat_references <= 0;
    match &packet.engagement_target {
        Some(target) => {
            eq(
                r,
                &format!("{label}.engagement_target"),
                (target.reference != 0).then_some(target.reference),
                saved.strike_force_target,
            );
            if target.resolves.is_none() {
                r.difference(
                    format!("{label}.engagement_target.resolves"),
                    "capture refused",
                    "native resolution",
                );
            }
            r.limitation("strike_force_target writes raw reference halves without resolving them; captured liveness needs an independent matched native/write witness, still pending", format!("{label}.engagement_target.resolves"));
        }
        None if strike => r.difference(
            format!("{label}.engagement_target"),
            "capture refused",
            "native raw target",
        ),
        None => {}
    }
    let target_resolves = packet
        .engagement_target
        .as_ref()
        .is_some_and(|target| target.reference != 0 && target.resolves == Some(true));
    let avoidance = strike && !target_resolves && row.engagement_rule != 4;
    match &packet.avoided_ship_types {
        Some(types) => {
            eq(
                r,
                &format!("{label}.avoided_ship_types.len"),
                types.len(),
                saved.target_ship_types.len(),
            );
            for (at, kind) in types.iter().enumerate() {
                let field = format!("{label}.avoided_ship_types[{at}]");
                match kind {
                    Some(kind) => {
                        let comparable = content_reader_parser::parser::c_string(kind);
                        if comparable.len() < kind.len() {
                            r.limitation("native text serialization ends at the first NUL; captured trailing ship-type bytes are not save-comparable", &field);
                        }
                        eq(
                            r,
                            &field,
                            Some(comparable),
                            saved.target_ship_types.get(at).map(Vec::as_slice),
                        );
                    }
                    None => r.difference(&field, "string capture refused", "saved raw string"),
                }
            }
        }
        None if avoidance => r.difference(
            format!("{label}.avoided_ship_types"),
            "capture refused",
            "native target ship types",
        ),
        None => {}
    }
}

fn task_force_motion(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::TaskForceMapMotion>()?;
    let provinces: BTreeSet<u32> = frame
        .complete::<tables::Provinces>()?
        .iter()
        .map(|row| row.id)
        .collect();
    eq(
        r,
        "TaskForceMapMotion TaskForces census",
        rows.iter()
            .map(|row| row.task_force)
            .collect::<BTreeSet<_>>(),
        frame
            .complete::<tables::TaskForces>()?
            .iter()
            .map(|row| row.id)
            .collect(),
    );
    for row in rows {
        let label = format!("TaskForceMapMotion {}", row.task_force);
        if row.combat_role > 2 {
            return Err(invalid(format!(
                "{label} combat role {} is outside 0..=2",
                row.combat_role
            )));
        }
        if let Some(unknown) = [row.target]
            .iter()
            .chain(&row.path)
            .chain(&row.alternate_path)
            .find(|&&id| id != 0 && !provinces.contains(&id))
        {
            return Err(invalid(format!("{label} names unknown province {unknown}")));
        }
        let Some(saved) = save.task_forces.get(&row.task_force) else {
            r.difference(label, "present", "absent");
            continue;
        };
        eq(
            r,
            &format!("{label}.path"),
            row.path.as_slice(),
            saved.path.as_slice(),
        );
        eq(
            r,
            &format!("{label}.target"),
            (row.target != 0).then_some(row.target),
            saved.target,
        );
        eq(
            r,
            &format!("{label}.alternate_path"),
            row.alternate_path.as_slice(),
            saved.alternate_path.as_slice(),
        );
        if let Some(value) = r.recorded(
            &row.movement_numerator,
            "TaskForceMapMotion.movement_numerator",
            format!("{label}.movement_numerator"),
        ) {
            let expected = saved
                .movement_progress
                .as_ref()
                .map_or(0, |value| value.raw);
            match value {
                Some(raw) => eq(r, &format!("{label}.movement_numerator"), *raw, expected),
                None => r.difference(
                    format!("{label}.movement_numerator: guarded read refusal"),
                    "unavailable",
                    expected,
                ),
            }
        }
        for (name, value, expected) in [
            ("retreat", &row.retreat, saved.retreat),
            ("withdrawal", &row.withdrawal, saved.withdrawal),
        ] {
            if let Some(value) = r.recorded(
                value,
                &format!("TaskForceMapMotion.{name}"),
                format!("{label}.{name}"),
            ) {
                match value {
                    Some(raw) => eq(r, &format!("{label}.{name}"), *raw != 0, expected),
                    None => r.difference(
                        format!("{label}.{name}: guarded read refusal"),
                        "unavailable",
                        expected,
                    ),
                }
            }
        }
        if let Some(value) = r.recorded(
            &row.naval_transfer,
            "TaskForceMapMotion.naval_transfer",
            format!("{label}.naval_transfer"),
        ) {
            match value {
                Some(0 | 1) => {}
                Some(raw) => {
                    return Err(invalid(format!(
                        "{label}.naval_transfer presence byte {raw} is outside 0..=1"
                    )));
                }
                None => r.difference(
                    format!("{label}.naval_transfer: guarded read refusal"),
                    "unavailable",
                    "required runtime operand",
                ),
            }
        }
        if let Some(value) = r.recorded(
            &row.combat_links,
            "TaskForceMapMotion.combat_links",
            format!("{label}.combat_links"),
        ) {
            match value {
                None => r.difference(
                    format!("{label}.combat_links: guarded read refusal"),
                    "unavailable",
                    "required runtime combat identities",
                ),
                Some([0, 0]) => {
                    eq(r, &format!("{label}.combat_role"), row.combat_role, 0);
                    eq(
                        r,
                        &format!("{label}.combat_links empty saved naval membership"),
                        save.naval_combats.values().any(|combat| {
                            combat.attackers.contains(&row.task_force)
                                || combat.defenders.contains(&row.task_force)
                        }),
                        false,
                    );
                }
                Some([0, _]) => {
                    return Err(invalid(format!(
                        "{label}.combat_links has a naval identity without a first combat"
                    )));
                }
                Some([first, naval]) => {
                    eq(
                        r,
                        &format!("{label}.combat_role has first combat"),
                        row.combat_role != 0,
                        true,
                    );
                    r.limitation(
                        "the unit attack-side test checks the current logical owner in the first attacker's owner-country set via canonical country membership; the naval participant save writer saves participant units but omits that set, so saved participant side cannot certify combat_role",
                        format!("{label}.combat_role"),
                    );
                    if *naval == 0 {
                        eq(
                            r,
                            &format!("{label}.combat_links no saved naval membership"),
                            save.naval_combats.values().any(|combat| {
                                combat.attackers.contains(&row.task_force)
                                    || combat.defenders.contains(&row.task_force)
                            }),
                            false,
                        );
                    } else if let Some(combat) = save.naval_combats.get(naval) {
                        let attacking = combat.attackers.contains(&row.task_force);
                        let defending = combat.defenders.contains(&row.task_force);
                        eq(
                            r,
                            &format!("{label}.combat_links selected saved naval membership"),
                            attacking || defending,
                            true,
                        );
                    } else if *naval != 0 {
                        r.difference(
                            format!("{label}.combat_links selected saved naval identity"),
                            naval,
                            "absent",
                        );
                    }
                    if first != naval {
                        if let Some(combat) = save.naval_combats.get(first) {
                            r.difference(
                                format!("{label}.combat_links first naval selection"),
                                [*first, *naval],
                                [*first; 2],
                            );
                            let attacking = combat.attackers.contains(&row.task_force);
                            let defending = combat.defenders.contains(&row.task_force);
                            eq(
                                r,
                                &format!("{label}.combat_links first saved naval membership"),
                                attacking || defending,
                                true,
                            );
                        } else {
                            r.limitation(
                                "the save does not identify the first combat as naval and omits the per-unit combat order that selects its role",
                                format!("{label}.combat_role runtime first-combat selection"),
                            );
                        }
                    }
                }
            }
        }
    }
    eq(
        r,
        "TaskForceMapMotion saved task-force census",
        rows.len(),
        save.task_forces.len(),
    );
    r.compared("TaskForceMapMotion", rows.len());
    r.limitation("the unit save writer saves retreat and withdraw only as zero/nonzero truth, not raw nonzero byte identity", "TaskForceMapMotion retreat/withdrawal raw bytes");
    r.limitation("the unit save writer does not serialize the transfer pointer; saved combat transfer references do not prove this task force's pointer presence", "TaskForceMapMotion.naval_transfer runtime presence");
    r.limitation("the unit save writer omits the ordered combat-reference vector; saved participant arrays cannot certify the first combat or first reached native type-2 combat, and unit joining establishes no global singleton invariant", "TaskForceMapMotion.combat_links runtime ordering and selection");
    Ok(())
}

fn task_force_fresh(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::TaskForceMapMotion>()?;
    for row in rows {
        let label = format!("TaskForceMapMotion {}.fresh", row.task_force);
        let Some(packet) = report.recorded(&row.fresh, "TaskForceMapMotion.fresh", &label) else {
            continue;
        };
        let Some(packet) = packet else {
            report.difference(
                &label,
                "unavailable",
                "required current-hour task-force packet",
            );
            continue;
        };
        if !report.packet_fresh(&label, packet.sample_hour, frame.hour) {
            continue;
        }
        for (name, missing) in [
            ("base_speed", packet.base_speed.is_none()),
            ("speed_modifier", packet.speed_modifier.is_none()),
            ("terrain_class", packet.terrain_class.is_none()),
            ("modified_range", packet.modified_range.is_none()),
            ("exiled", packet.exiled.is_none()),
            (
                "strategic_redeployment",
                packet.strategic_redeployment.is_none(),
            ),
        ] {
            if missing {
                report.difference(
                    format!("{label}.{name}"),
                    "unavailable",
                    "required runtime operand",
                );
            }
        }
        if let Some(raw) = packet.exiled {
            match save.task_forces.get(&row.task_force) {
                Some(saved) => eq(
                    report,
                    &format!("{label}.exiled truth"),
                    raw != 0,
                    saved.exile,
                ),
                None => report.difference(
                    format!("{label}.exiled saved task force"),
                    "present",
                    "absent",
                ),
            }
        }
        report.limitation("base speed and terrain selector come from the stored task-force stat object; common speed and raw modified range combine actual-location, weather, owner, ship, leader, HQ and fuel getters before route terrain or distance scaling. Saved ships and equipment do not certify these live caches or composed results", &label);
        report.limitation("The unit writer saves task-force exile only as nonzero truth; arbitrary nonzero exile byte identity remains unverified. No task-force serializer law is proved for strategic movement, so that exact raw operand needs a matched VM witness", format!("{label}.simulation bytes"));
    }
    report.compared("TaskForceMapMotion.fresh", rows.len());
    Ok(())
}

fn fleets(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::FleetCounterInputs>()?;
    eq(
        r,
        "FleetCounterInputs Fleets census",
        rows.iter().map(|row| row.fleet).collect::<BTreeSet<_>>(),
        frame
            .complete::<tables::Fleets>()?
            .iter()
            .map(|row| row.id)
            .collect(),
    );
    let saved = &save.counter_inputs.fleets;
    for row in rows {
        let label = format!("FleetCounterInputs {}", row.fleet);
        let Some(expected) = saved.get(&row.fleet) else {
            r.difference(label, "present", "absent");
            continue;
        };
        eq(
            r,
            &format!("{label}.task_forces"),
            row.task_forces.as_slice(),
            expected.task_forces.as_slice(),
        );
        eq(
            r,
            &format!("{label}.mission_region_count"),
            (row.mission_region_count > 0).then_some(row.mission_region_count as usize),
            expected.strategic_regions.as_ref().map(Vec::len),
        );
        let mut membership = expected.strategic_regions.clone().unwrap_or_default();
        membership.sort_unstable();
        eq(
            r,
            &format!("{label}.mission_regions"),
            row.mission_regions
                .require("fleet mission regions")
                .map_err(invalid)?
                .as_ref()
                .map(|rows| rows.as_slice()),
            Some(membership.as_slice()),
        );
        eq(
            r,
            &format!("{label} saved color components"),
            expected.color.as_ref().map(Vec::len),
            Some(3),
        );
    }
    eq(
        r,
        "FleetCounterInputs saved fleet census",
        rows.len(),
        saved.len(),
    );
    r.compared("FleetCounterInputs", rows.len());
    r.limitation("the game saves the colour as quantized RGB without alpha; the four raw f32 component words are not independently compared", "FleetCounterInputs red, green, blue, alpha");
    Ok(())
}

fn countries(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::CountryCounterInputs>()?;
    let pride_ships = frame.complete::<tables::Ships>();
    let tags: BTreeMap<_, _> = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|row| (row.id, row.tag.as_slice()))
        .collect();
    eq(
        r,
        "CountryCounterInputs Countries census",
        rows.iter().map(|row| row.country).collect::<BTreeSet<_>>(),
        tags.keys().copied().collect(),
    );
    super::country_rosters::compare_lists(rows, save, &tags, r)?;
    let mut owned: BTreeMap<&[u8], usize> = BTreeMap::new();
    let mut controlled: BTreeMap<&[u8], usize> = BTreeMap::new();
    for state in save.states.values() {
        if let Some(owner) = &state.owner {
            *owned.entry(owner.as_slice()).or_default() += 1;
            let controller = state.controller.as_ref().unwrap_or(owner);
            *controlled.entry(controller.as_slice()).or_default() += 1;
        }
    }
    for row in rows {
        let tag = tags
            .get(&row.country)
            .ok_or_else(|| invalid("CountryCounterInputs country missing"))?;
        let label = format!("CountryCounterInputs {}", String::from_utf8_lossy(tag));
        let name_label = format!("{label}.pride_name");
        if let Some(name) = r.recorded(
            &row.pride_name,
            "CountryCounterInputs.pride_name",
            &name_label,
        ) && row.pride_of_the_fleet != 0
        {
            match &pride_ships {
                Ok(ships)
                    if ships
                        .binary_search_by_key(&row.pride_of_the_fleet, |ship| ship.id)
                        .is_ok() =>
                {
                    match name {
                        None => r.difference(
                            &name_label,
                            "capture refused",
                            "registered pride ship cached name",
                        ),
                        Some(_) => r.limitation(
                            "the ship-name lookup returns the retained name bytes; the name member save writer omits that cache, so saved name-generator operands do not certify its text, including a known empty name",
                            &name_label,
                        ),
                    }
                }
                Ok(_) => {}
                Err(_) => r.limitation(
                    "complete Ships is required to admit the registered pride ship's current cached-name operand; country prefix and raw pride save comparisons remain available",
                    &name_label,
                ),
            }
        }
        for (name, value, reason) in [
            (
                "mio_policies",
                &row.mio_policies,
                "Production status rebuilds the allowed-policy roster on load; saved organisation policy choices cannot certify this cache or its order",
            ),
            (
                "neighbors",
                &row.neighbors,
                "the game rebuilds the neighbor roster on load; saved borders cannot certify its order, repetitions or raw references",
            ),
        ] {
            if let Some(value) = r.recorded(
                value,
                &format!("CountryCounterInputs.{name}"),
                format!("{label}.{name}"),
            ) {
                r.limitation(
                    if value.is_some() {
                        reason
                    } else {
                        "country input roster unavailable from native capture"
                    },
                    format!("{label}.{name}"),
                );
            }
        }
        let Some(pride) = save.counter_inputs.prides.get(*tag) else {
            r.difference(label, "present", "absent");
            continue;
        };
        match pride {
            Some(handle) => eq(
                r,
                &format!("{label}.pride_of_the_fleet"),
                row.pride_of_the_fleet,
                *handle,
            ),
            None => eq(
                r,
                &format!("{label}.pride_of_the_fleet (omitted)"),
                row.pride_of_the_fleet,
                0,
            ),
        }
        eq(
            r,
            &format!("{label}.owned_state_count"),
            row.owned_state_count as usize,
            owned.get(*tag).copied().unwrap_or(0),
        );
        eq(
            r,
            &format!("{label}.controlled_state_count"),
            row.controlled_state_count as usize,
            controlled.get(*tag).copied().unwrap_or(0),
        );
        eq(
            r,
            &format!("{label}.first_division"),
            row.first_division,
            save.counter_inputs
                .first_divisions
                .get(*tag)
                .copied()
                .unwrap_or(0),
        );
    }
    r.compared("CountryCounterInputs", rows.len());
    r.limitation(
        "controlled_state_count is compared with the saved states whose controller (their owner when omitted) is the country; controller updates add only states with provinces (the state province roster), and every saved state is assumed to have them, as owned_state_count assumes",
        "CountryCounterInputs.controlled_state_count",
    );
    Ok(())
}

fn saved_groups(save: &Save) -> BTreeMap<u64, &content_reader_save::naval_history::OrdersGroup> {
    save.naval_history
        .theatre_countries
        .iter()
        .flat_map(|countries| countries.values())
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
        .map(|group| (group.id, group))
        .collect()
}

fn hq_template_context(
    frame: &DecodedFrame,
    row: &replay_data::rows::OrdersGroupCounterInputRow,
    saved: &content_reader_save::naval_history::OrdersGroup,
    r: &mut Report,
) -> io::Result<()> {
    let value = row
        .template_context
        .require("OrdersGroupCounterInputs.template_context")
        .map_err(invalid)?
        .as_ref()
        .ok_or_else(|| invalid("OrdersGroupCounterInputs.template_context acquisition refused"))?;
    let label = format!("OrdersGroupCounterInputs {}.template_context", row.group);
    let target = if value.target == 0 {
        None
    } else {
        frame
            .complete::<tables::DivisionTemplates>()?
            .iter()
            .any(|row| row.template == value.target)
            .then_some(value.target)
    };
    eq(r, &format!("{label}.target"), target, saved.target_template);
    eq(
        r,
        &format!("{label}.deployed"),
        value.deployed != 0,
        saved.hq_deployed,
    );
    eq(
        r,
        &format!("{label}.queued"),
        value.queued != 0,
        saved.hq_queued,
    );
    eq(
        r,
        &format!("{label}.deployment present"),
        value.deployment.is_some(),
        saved.hq_deployment.is_some(),
    );
    match (&value.deployment, &saved.hq_deployment) {
        (Some(actual), Some(expected)) => {
            let countries = country_context(frame)?;
            eq(
                r,
                &format!("{label}.country"),
                required_country_tag(actual.country, &countries.links)?,
                expected.country.as_slice(),
            );
            eq(
                r,
                &format!("{label}.priority"),
                actual.priority,
                expected.priority,
            );
            eq(
                r,
                &format!("{label}.allow_zero_entries"),
                Some(actual.allow_zero != 0),
                expected.equipment.allow_zero_entries,
            );
            eq(
                r,
                &format!("{label}.assembled manpower"),
                actual.assembled_manpower,
                expected.assembled_manpower,
            );
            eq(
                r,
                &format!("{label}.requested manpower"),
                actual.requested_manpower,
                expected.requested_manpower,
            );
            eq(r, &format!("{label}.order"), actual.order, expected.order);
            eq(
                r,
                &format!("{label}.requisitioned"),
                actual.requisitioned != 0,
                expected.requisitioned,
            );
            let equipment = value
                .equipment
                .iter()
                .filter(|row| actual.allow_zero != 0 || row.amount != 0)
                .map(|row| (row.variant, row.amount))
                .collect::<Vec<_>>();
            let expected_equipment = expected
                .equipment
                .equipment
                .iter()
                .map(|row| (row.variant, row.amount.raw))
                .collect::<Vec<_>>();
            eq(
                r,
                &format!("{label}.assembled equipment"),
                equipment,
                expected_equipment,
            );
            let tokens = complete::<tables::LexicalTokens>(frame)?
                .iter()
                .map(|row| (row.token, row.literal.as_slice()))
                .collect::<BTreeMap<_, _>>();
            let required = value
                .required
                .iter()
                .filter(|row| row.amount != 0)
                .map(|row| {
                    let token = tokens
                        .get(&row.token)
                        .ok_or_else(|| invalid("headquarters required equipment token missing"))?;
                    Ok((c_string(token).to_vec(), row.amount))
                })
                .collect::<io::Result<Vec<_>>>()?;
            let expected_required = expected
                .required
                .iter()
                .map(|(token, amount)| (token.clone(), amount.raw))
                .collect::<Vec<_>>();
            eq(
                r,
                &format!("{label}.requested equipment"),
                required,
                expected_required,
            );
        }
        (None, _) if !value.equipment.is_empty() || !value.required.is_empty() => {
            return Err(invalid(
                "headquarters context without holder contains equipment",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn groups(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::OrdersGroupCounterInputs>()?;
    eq(
        r,
        "OrdersGroupCounterInputs OrdersGroups census",
        rows.iter().map(|row| row.group).collect::<BTreeSet<_>>(),
        frame
            .complete::<tables::OrdersGroups>()?
            .iter()
            .map(|row| row.handle)
            .collect(),
    );
    let saved = saved_groups(save);
    let mut unsaved = 0;
    for row in rows {
        match saved.get(&row.group) {
            Some(group) => {
                hq_template_context(frame, row, group, r)?;
                let (withdrawing, distance, invasion_source_distance, nearest_front) = (
                    group.withdrawing,
                    group.distance,
                    group.invasion_source_distance,
                    group.hq_nearest_front,
                );
                let value = row
                    .hq_nearest_front
                    .require("OrdersGroupCounterInputs.hq_nearest_front")
                    .map_err(invalid)?
                    .ok_or_else(|| {
                        invalid("OrdersGroupCounterInputs.hq_nearest_front acquisition refused")
                    })?;
                eq(
                    r,
                    &format!("OrdersGroupCounterInputs {}.hq_nearest_front", row.group),
                    (value >= 0).then_some(value),
                    nearest_front,
                );
                eq(
                    r,
                    &format!("OrdersGroupCounterInputs {}.withdrawing", row.group),
                    row.withdrawing != 0,
                    withdrawing,
                );
                if let Some(value) = r.recorded(
                    &row.hq_distance,
                    "OrdersGroupCounterInputs.hq_distance",
                    format!("orders group {} HQ distance", row.group),
                ) {
                    if let Some(value) = value {
                        eq(
                            r,
                            &format!("OrdersGroupCounterInputs {}.hq_distance", row.group),
                            (*value >= 0).then_some(*value),
                            distance,
                        );
                    } else {
                        r.limitation(
                            "HQ distance unreadable at capture",
                            format!("OrdersGroupCounterInputs.hq_distance {}", row.group),
                        );
                    }
                }
                if let Some(value) = r.recorded(
                    &row.invasion_source_distance,
                    "OrdersGroupCounterInputs.invasion_source_distance",
                    format!("orders group {} invasion-source distance", row.group),
                ) {
                    if let Some(value) = value {
                        eq(
                            r,
                            &format!(
                                "OrdersGroupCounterInputs {}.invasion_source_distance",
                                row.group
                            ),
                            (*value >= 0).then_some(*value),
                            invasion_source_distance,
                        );
                    } else {
                        r.limitation(
                            "invasion-source distance unreadable at capture",
                            format!(
                                "OrdersGroupCounterInputs.invasion_source_distance {}",
                                row.group
                            ),
                        );
                    }
                }
            }
            None => unsaved += 1,
        }
    }
    if unsaved != 0 {
        r.limitation("groups outside the serialized land-theatre definitions have no saved withdrawing, HQ-distance or invasion-source-distance witness", format!("OrdersGroupCounterInputs: {unsaved} rows"));
    }
    r.compared("OrdersGroupCounterInputs", rows.len());
    r.limitation(
        "the game saves withdrawing as truth only; nonzero byte values are indistinguishable",
        "OrdersGroupCounterInputs.withdrawing byte value",
    );
    r.limitation(
        "the game writes distance (token 0x2DDA) from the signed stored distance only when nonnegative; omission proves negativity but cannot certify the exact negative word",
        "OrdersGroupCounterInputs.hq_distance negative magnitude",
    );
    r.limitation("the game writes hq_distance_to_naval_invasion_source (token4540) from the signed invasion-source HQ distance only when nonnegative; omission proves negativity but cannot certify the exact negative word",
        "OrdersGroupCounterInputs.invasion_source_distance negative magnitude");
    Ok(())
}

fn leader_paths(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LeaderPortraitPaths>()?;
    let recorded: BTreeSet<_> = rows.iter().map(|row| row.leader).collect();
    super::counters::hq_leader_scope(frame, save, r, "LeaderPortraitPaths", recorded)?;
    let tags: BTreeMap<_, _> = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|row| (row.id, row.tag.as_slice()))
        .collect();
    for row in rows {
        if row.tag.is_empty() {
            continue;
        }
        let label = format!("LeaderPortraitPaths {}", row.leader);
        if let Some(tag) = tags.get(&row.tag_reference) {
            eq(
                r,
                &format!("{label} Countries tag"),
                row.tag.as_slice(),
                *tag,
            );
        }
        eq(
            r,
            &format!("{label} saved country tag"),
            save.countries.contains_key(&row.tag),
            true,
        );
    }
    r.compared("LeaderPortraitPaths", rows.len());
    r.limitation("the unit-leader writer does not serialize the resolved unit-leader owner country reference, and the original-tag reference is not compared here; tag strings are checked against the recorded Countries tags and saved country keys only", "LeaderPortraitPaths country and tag_reference");
    Ok(())
}

#[cfg(test)]
mod tests;
