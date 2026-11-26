use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_save::Save;
use replay_data::{rows::CountryRow, rows::StateRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{
    ContentRead, Report, complete, convoy_total, country_tag, invalid, lexical_tokens,
    required_context, required_country_tag, tag_text,
};

pub fn compare_province_names(
    frame: &DecodedFrame,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::ProvinceNames>(frame)?;
    if !report.refresh_is_current(TableId::ProvinceNames, frame.hour) {
        report.limitation(replay_data::cadence::STAGGER_LIMITATION, "ProvinceNames");
        return Ok(());
    }
    let mut remaining: BTreeMap<u32, &[u8]> = save
        .provinces
        .values()
        .filter_map(|p| p.name.as_deref().map(|name| (p.id, name)))
        .collect();
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.province == 0 || row.name.is_empty() || !seen.insert(row.province) {
            return Err(invalid(
                "ProvinceNames has a null, empty or duplicate sparse row",
            ));
        }
        let subject = format!("province {}.name override", row.province);
        let name = super::serialized_text(report, &subject, &row.name);
        report.scalar_checks += 1;
        match remaining.remove(&row.province) {
            Some(saved) if saved == name => {}
            Some(saved) => report.difference(subject, name, saved),
            None => report.difference(subject, "present", "absent"),
        }
    }
    for (province, _) in remaining {
        report.difference(
            format!("province {province}.name override"),
            "absent",
            "present",
        );
    }
    report.compared("ProvinceNames", rows.len());
    report.limitation("province overrides retain literal engine bytes (possibly localization keys), not resolved display strings", "ProvinceNames.name");
    Ok(())
}

pub fn compare_countries(
    save: &Save,
    equipment: &ContentRead<'_, content_reader_key_names::equipment::Database>,
    by_tag: &BTreeMap<&[u8], &CountryRow>,
    report: &mut Report,
) {
    report.record_country_emissions(save);
    for (&tag, captured) in by_tag {
        let label = String::from_utf8_lossy(tag);
        let Some(saved) = save.countries.get(tag) else {
            report.difference(format!("country {label}"), "present", "absent");
            continue;
        };
        report.countries += 1;
        report.scalar_checks += 1;
        if captured.manpower_residual != saved.manpower_residual {
            report.difference(
                format!("{label}.manpower.current"),
                captured.manpower_residual,
                saved.manpower_residual,
            );
        }
        report.scalar(
            &label,
            "political_power",
            captured.political_power_raw,
            saved.political_power.as_ref(),
        );
        report.scalar(
            &label,
            "command_power",
            Some(captured.command_power_raw),
            saved.command_power.as_ref(),
        );
        report.scalar(
            &label,
            "stability",
            Some(captured.stability_raw),
            saved.stability.as_ref(),
        );
        report.scalar(
            &label,
            "war_support",
            Some(captured.war_support_raw),
            saved.war_support.as_ref(),
        );
        match (
            captured.convoys_raw,
            required_context("verified equipment", equipment)
                .map_err(|error| error.to_string())
                .and_then(|equipment| convoy_total(saved, save, equipment)),
        ) {
            (None, _) => report.limitation(
                "convoy cache was invalid at capture; no value was invented",
                label.to_string(),
            ),
            (Some(actual), Ok(expected)) => {
                report.scalar_checks += 1;
                if actual != expected {
                    report.difference(format!("{label}.convoys"), actual, expected);
                }
            }
            (Some(_), Err(reason)) => {
                report.failed_comparisons.insert(
                    format!("country {label} convoys"),
                    format!("{label}.convoys cannot be compared: {reason}"),
                );
            }
        }
    }
    report.limitation(
        "rendered country names are not serialized by the country writer",
        format!("all {} countries", by_tag.len()),
    );
    report.limitation(
        "border_land_doctrine_level is a stored technology cache; reset can leave it stale, so saved research counts do not certify it",
        "Countries.border_land_doctrine_level",
    );
    report.limitation(
        "peace_conference_participant is the peace-conference participant flag, which the game sets for the \
         running conference's winner and loser tag lists and the game \
         clears; no saved country field is known to hold it and the parsed peace_conference \
         history does not carry those lists, so it is not compared (UNVERIFIED until a live \
         conference is recorded)",
        format!("all {} countries", by_tag.len()),
    );
    for (tag, _) in report
        .canonical_countries(&save.countries)
        .into_iter()
        .filter(|(tag, _)| !by_tag.contains_key(tag.as_slice()))
    {
        report.difference(
            format!("country {}", String::from_utf8_lossy(tag)),
            "absent",
            "present",
        );
    }
}

pub fn compare_provinces(
    frame: &DecodedFrame,
    save: &Save,
    membership: &content_reader_save::PartialMapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut provinces = BTreeMap::new();
    for province in frame.complete::<tables::Provinces>()? {
        if provinces.insert(province.id, province).is_some() {
            return Err(invalid("duplicate province row"));
        }
    }
    let mut affected = BTreeSet::new();
    let mut skipped = 0;
    for (&id, entry) in &membership.provinces {
        if let content_reader_save::ProvinceMembership::Unknown(states) = entry {
            affected.extend(states);
            skipped += usize::from(id != 0);
        }
    }
    report.membership_skip("Provinces", skipped, membership, &affected);
    let control = save
        .province_control(membership)?
        .into_iter()
        .map(|row| (row.id, row))
        .collect::<BTreeMap<_, _>>();
    let mut nonpositive_owners = 0;
    for &id in membership.provinces.keys().filter(|&&id| id != 0) {
        let Some(actual) = provinces.remove(&id) else {
            report.difference(format!("province {id}"), "absent", "present");
            continue;
        };
        report.provinces += 1;
        let victory_points = save
            .provinces
            .get(&id)
            .and_then(|saved| saved.victory_points)
            .unwrap_or(0);
        report.compared("Provinces.victory_points", 1);
        if actual.victory_points != victory_points {
            report.difference(
                format!("province {id}.victory_points"),
                actual.victory_points,
                victory_points,
            );
        }
        let expected = control.get(&id).expect("province has a control row");
        let owner = if !expected.membership_known {
            None
        } else {
            if actual.state != expected.state.unwrap_or(0) {
                report.difference(
                    format!("province {}.state", expected.id),
                    actual.state,
                    expected.state,
                );
            }
            let owner = if expected.owner.is_none() {
                nonpositive_owners += 1;
                if (actual.owner as i32) > 0 {
                    report.difference(
                        format!("province {}.owner positive class", expected.id),
                        actual.owner,
                        "signed nonpositive owner",
                    );
                }
                None
            } else if (actual.owner as i32) <= 0 {
                None
            } else {
                country_tag(actual.owner, links)?
            };
            if expected.owner.is_some() && owner != expected.owner.as_deref() {
                report.difference(
                    format!("province {}.owner", expected.id),
                    tag_text(owner),
                    tag_text(expected.owner.as_deref()),
                );
            }
            owner
        };
        if expected.membership_known || expected.controller_explicit {
            if expected.controller.is_none() {
                if (actual.controller as i32) > 0 {
                    report.difference(
                        format!("province {}.controller positive class", expected.id),
                        actual.controller,
                        "signed nonpositive controller",
                    );
                }
                continue;
            }
            let controller = if (actual.controller as i32) <= 0 && !expected.controller_explicit {
                owner
            } else if (actual.controller as i32) <= 0 {
                None
            } else {
                country_tag(actual.controller, links)?
            };
            if controller != expected.controller.as_deref() {
                report.difference(
                    format!("province {}.controller", expected.id),
                    tag_text(controller),
                    tag_text(expected.controller.as_deref()),
                );
            }
        }
    }
    for id in provinces.keys() {
        report.difference(format!("province {id}"), "present", "absent");
    }
    if nonpositive_owners > 0 {
        report.limitation(
            "omitted state owners and province controllers certify the signed nonpositive class, not raw0 or the exact negative magnitude",
            format!("Provinces: {nonpositive_owners} nonpositive-owner provinces"),
        );
    }
    report.limitation(
        "Provinces.move_gate has no save field: the COwnerArea partition and each province's border-conflict set are rebuilt at load",
        "Provinces.move_gate",
    );
    report.limitation(
        "Provinces.port_sea has no save field: it is content, the seventh map/buildings.txt field of each admitted port slot that map-building loading stores in file order (last write wins, zero included). The oracle does not reproduce that loader's slot admission (raster placement, land and declared-state checks, the floating harbour exemption), which only the viewer's map-building layer implements, so it is not derived here",
        "Provinces.port_sea",
    );
    Ok(())
}

pub fn compare_states(
    frame: &DecodedFrame,
    save: &Save,
    membership: &content_reader_save::PartialMapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut states = BTreeMap::new();
    for state in frame.complete::<tables::States>()? {
        if states.insert(state.id, state).is_some() {
            return Err(invalid("duplicate state row"));
        }
    }
    report.limitation(
        "text saves omit the cached victory-point index and the cached controller-group branch; recorded range/availability checks cannot certify the cached choices, which need matched native observations",
        "States.best_vp_index and States.single_controller",
    );
    let mut province_counts = BTreeMap::<u32, usize>::new();
    for province in frame.complete::<tables::Provinces>()? {
        *province_counts.entry(province.state).or_default() += 1;
    }
    for state in states.values() {
        if let Some(&Some(index)) = report.recorded(
            &state.best_vp_index,
            "States.best_vp_index",
            format!("state {} cached victory point", state.id),
        ) {
            let count = province_counts.get(&state.id).copied().unwrap_or(0);
            if index >= 0 && index as usize >= count {
                report.difference(
                    format!("state {}.best_vp_index", state.id),
                    index,
                    format!("negative or below {count}"),
                );
            }
        }
        if let Some(value) = report.recorded(
            &state.single_controller,
            "States.single_controller",
            format!("state {} cached controller branch", state.id),
        ) && !matches!(value, None | Some(0 | 1))
        {
            report.difference(
                format!("state {}.single_controller", state.id),
                value,
                "captured 0 or 1",
            );
        }
    }
    let tokens = lexical_tokens(frame, report)?;
    for expected in save.state_control_emissions(membership)? {
        let id = match &expected {
            content_reader_save::StateControlEmission::Positive(control) => control.id,
            content_reader_save::StateControlEmission::NonPositiveOwner { id } => *id,
        };
        let Some(actual) = states.remove(&id) else {
            report.difference(format!("state {id}"), "absent", "present");
            continue;
        };
        report.states += 1;
        match expected {
            content_reader_save::StateControlEmission::Positive(expected) => {
                let owner = country_tag(actual.owner, links)?;
                if owner != Some(expected.owner.as_slice()) {
                    report.difference(
                        format!("state {id}.owner"),
                        tag_text(owner),
                        tag_text(Some(&expected.owner)),
                    );
                }
                let controller = country_tag(actual.controller, links)?;
                if controller != Some(expected.controller.as_slice()) {
                    report.difference(
                        format!("state {id}.controller"),
                        tag_text(controller),
                        tag_text(Some(&expected.controller)),
                    );
                }
            }
            content_reader_save::StateControlEmission::NonPositiveOwner { .. } => {
                if (actual.owner as i32) > 0 {
                    report.difference(format!("state {id}.owner positive class"), true, false);
                }
                report.limitation(
                    "the state save writer omits a signed-nonpositive owner; omission certifies the nonpositive class, not raw0 or the exact negative magnitude",
                    format!("state {id}.owner"),
                );
                report.limitation(
                    "the state save writer also omits controller inside the nonpositive-owner arm; no controller value can be inferred",
                    format!("state {id}.controller"),
                );
            }
        }
        compare_state_fields(actual, &save.states[&id], &tokens, report);
        compare_marked_owners(actual, &save.states[&id], links, report);
    }
    for id in states.keys() {
        report.difference(format!("state {id}"), "present", "absent");
    }
    report.limitation(
        "the save does not carry the owner-qualified state name the game sets on each owner change, so it is not compared",
        "States.owner_name",
    );
    report.limitation(
        "the save does not carry the retained continent cache; final authored provinces cannot establish its stale continent cache",
        "States.continent",
    );
    report.limitation(
        "for positive owners, state serializer omits controllers equal to owner by raw or canonical country-link identity; omitted fields compare to the saved owner without accepting captured zero",
        "positive-owner states with no explicit saved controller",
    );
    Ok(())
}

fn compare_state_fields(
    actual: &StateRow,
    saved: &content_reader_save::State,
    tokens: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) {
    let field = format!("state {}.category", actual.id);
    if let Some(&category) = report.recorded(&actual.category, "States.category", &field) {
        if tokens.is_empty() {
            report.limitation(
                "the recorded lexical dictionary is unavailable, so the category token is not named",
                field,
            );
            return compare_state_scalars(actual, saved, report);
        }
        let recorded = category.map(|token| tokens.get(&token).copied().ok_or(token));
        let saved_name = saved.category.as_deref().map(String::from_utf8_lossy);
        match (recorded, saved.category.as_deref()) {
            (Some(Ok(name)), Some(saved)) if name.eq_ignore_ascii_case(saved) => {}
            (Some(Ok(name)), _) => {
                report.difference(field, String::from_utf8_lossy(name), saved_name)
            }
            (Some(Err(token)), _) => report.difference(
                field,
                format!("token {token} outside the recorded dictionary"),
                saved_name,
            ),
            (None, _) => report.difference(field, "refused read", saved_name),
        }
    }
    compare_state_scalars(actual, saved, report);
}

fn compare_marked_owners(
    actual: &StateRow,
    saved: &content_reader_save::State,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) {
    let field = format!("state {}.marked_owners", actual.id);
    let expected = saved
        .contested_owners
        .iter()
        .map(|tag| String::from_utf8_lossy(tag).into_owned())
        .collect::<Vec<_>>();
    let Some(owners) = report.recorded(&actual.marked_owners, "States.marked_owners", &field)
    else {
        return;
    };
    let Some(owners) = owners else {
        report.difference(field, "refused read", expected.join(" "));
        return;
    };
    let recorded = owners
        .iter()
        .map(|&raw| match country_tag(raw, links) {
            Ok(tag) => tag_text(tag),
            Err(_) => format!("unresolved country link {raw}"),
        })
        .collect::<Vec<_>>();
    if recorded != expected {
        report.difference(field, recorded.join(" "), expected.join(" "));
    }
}

fn compare_state_scalars(actual: &StateRow, saved: &content_reader_save::State, report: &mut Report) {
    let field = format!("state {}.manpower_available", actual.id);
    match saved.manpower_available {
        Some(saved) if saved != actual.manpower_available => {
            report.difference(field, actual.manpower_available, saved);
        }
        Some(_) => {}
        None => report.limitation(
            "CState omits the manpower pool when total <= 0; available is not independently saved",
            field,
        ),
    }
    if let Some(&locked) = report.recorded(
        &actual.manpower_locked,
        "States.manpower_locked",
        format!("state {}.manpower_locked", actual.id),
    ) && saved.population.is_some()
        && saved.manpower_locked != Some(locked)
    {
        report.difference(
            format!("state {}.manpower_locked", actual.id),
            Some(locked),
            saved.manpower_locked,
        );
    }
    let field = format!("state {}.population", actual.id);
    if let Some(&population) = report.recorded(&actual.population, "States.population", &field) {
        match saved.population {
            Some(Some(saved)) if saved != population => {
                report.difference(field, population, saved)
            }
            Some(Some(_)) => {}
            Some(None) => report.difference(field, population, "pool without total"),
            None if population > 0 => {
                report.difference(field, population, "omitted pool (total <= 0)")
            }
            None => report.limitation(
                "CState omits the manpower pool when total <= 0; omission certifies a nonpositive population, not its value",
                field,
            ),
        }
    }
    let field = format!("state {}.custom_name", actual.id);
    let name = (!actual.custom_name.is_empty())
        .then(|| super::serialized_text(report, &field, &actual.custom_name));
    if name != saved.name.as_deref() {
        report.difference(field, name, saved.name.as_deref());
    }
}

pub fn compare_railway_infrastructure(
    frame: &DecodedFrame,
    save: &Save,
    buildings: &content_reader_key_names::buildings::Buildings,
    defines: &content_reader_defines::defines::Defines,
    names: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::States>(frame)?;
    for row in rows {
        let label = format!("state {}.railway_infrastructure", row.id);
        let Some(value) = report.recorded(
            &row.railway_infrastructure,
            "States.railway_infrastructure",
            &label,
        ) else {
            continue;
        };
        let Some(value) = value else {
            report.difference(&label, "unavailable", "required healthy/MAX fraction");
            continue;
        };
        let name = *names
            .get(&engine_native_parts::cbuilding_database::INFRASTRUCTURE_TOKEN)
            .ok_or_else(|| invalid("infrastructure token has no recorded spelling"))?;
        let token =
            std::str::from_utf8(name).map_err(|_| invalid("infrastructure token is not UTF-8"))?;
        let definition = buildings.by_token(token)?;
        let saved = save
            .construction
            .state_buildings
            .get(&(row.id, name.to_vec()));
        let expected = match (definition, saved) {
            (None, Some(_)) => {
                return Err(invalid(
                    "saved infrastructure has no selected content template",
                ));
            }
            (None, None) | (Some(_), None) => 0,
            (Some(definition), Some(saved)) => {
                let maximum = definition.base_maximum(defines)?;
                if maximum == 0 {
                    i64::from(u32::MAX)
                } else {
                    i64::from(saved.healthy_level) * 100_000 / i64::from(maximum)
                }
            }
        };
        report.scalar_checks += 1;
        if *value != expected {
            report.difference(&label, *value, expected);
        }
    }
    report.compared("States.railway_infrastructure", rows.len());
    report.limitation("saved infrastructure health and matching content certify the healthy/MAX value; they do not serialize building-status lookup indices, pointers, or the disabled null-template object identity", "States.railway_infrastructure lookup identity");
    Ok(())
}

pub fn compare_state_flags(
    frame: &DecodedFrame,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::StateFlags>(frame)?;
    let mut captured = BTreeMap::new();
    for row in rows {
        if row.state == 0 || captured.insert(row.state, row).is_some() {
            return Err(invalid("StateFlags has a null or duplicate state id"));
        }
    }
    for (&id, state) in &save.states {
        let expected = state
            .demilitarized
            .ok_or_else(|| invalid(format!("save state {id} lacks its demilitarized boolean")))?;
        let Some(actual) = captured.remove(&id) else {
            report.difference(format!("state flag {id}"), "absent", "present");
            continue;
        };
        if (actual.demilitarized != 0) != expected {
            report.difference(
                format!("state {id}.demilitarized"),
                actual.demilitarized != 0,
                expected,
            );
        }
        let expected_border = state.is_border_conflict.ok_or_else(|| {
            invalid(format!(
                "save state {id} lacks its is_border_conflict boolean"
            ))
        })?;
        if let Some(&active) = report.recorded(
            &actual.border_conflict_active,
            "StateFlags.border_conflict_active",
            format!("state {id}.is_border_conflict"),
        ) && (active != 0) != expected_border
        {
            report.difference(
                format!("state {id}.is_border_conflict"),
                active != 0,
                expected_border,
            );
        }
    }
    for id in captured.keys() {
        report.difference(format!("state flag {id}"), "present", "absent");
    }
    report.limitation("The save booleans certify zero/nonzero flag values, not unusual nonzero byte magnitudes. The border-combat tree is rebuilt at load and its runtime membership/order is not compared; saved side state/tag references do not yet prove that invariant", "StateFlags.border_conflict_countries");
    report.compared("StateFlags", rows.len());
    report.limitation(
        "demilitarized compares exactly as the serialized boolean; the recording retains the native byte but a yes/no save cannot distinguish different nonzero byte values",
        "StateFlags.demilitarized native byte",
    );
    Ok(())
}

pub fn compare_players(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut users = BTreeMap::<&[u8], Vec<&replay_data::rows::PlayerRow>>::new();
    let mut keys = BTreeSet::new();
    for player in frame.complete::<tables::Players>()? {
        if player.id == -1 || !keys.insert(player.id) {
            return Err(invalid("invalid or duplicate native player id"));
        }
        country_tag(player.country, links)?;
        users.entry(&player.user).or_default().push(player);
    }
    let mut saved_users = BTreeMap::<&[u8], Vec<&content_reader_save::Player>>::new();
    for player in &save.players {
        if player.country != b"---" && !save.countries.contains_key(&player.country) {
            return Err(invalid("saved player has unknown country tag"));
        }
        saved_users.entry(&player.user).or_default().push(player);
    }
    for (&user, players) in &users {
        let label = format!("player user {:?}", String::from_utf8_lossy(user));
        let saved = saved_users.remove(user).unwrap_or_default();
        if saved.len() != 1 {
            report.difference(format!("{label}.saved refreshed entries"), 1, saved.len());
            continue;
        }
        let expected = saved[0];
        if players.len() != 1 {
            report.limitation("native save upsert collapses equal current user bytes; only one surviving entry can be checked, not every live player or live order", format!("{label}: {} live entries", players.len()));
            let matches = players.iter().any(|actual| {
                let tag = country_tag(actual.country, links)
                    .ok()
                    .flatten()
                    .unwrap_or(b"---");
                actual.id == expected.id
                    && (actual.flags & 1 != 0) == expected.country_leader
                    && tag == expected.country
            });
            if !matches {
                report.difference(&label, "no live player matches saved survivor", expected);
            }
            continue;
        }
        let actual = players[0];
        report.players += 1;
        if actual.id != expected.id {
            report.difference(format!("{label}.id"), actual.id, expected.id);
        }
        let tag = country_tag(actual.country, links)?.unwrap_or(b"---");
        if tag != expected.country {
            report.difference(
                format!("{label}.country"),
                tag_text(Some(tag)),
                tag_text(Some(&expected.country)),
            );
        }
        if (actual.flags & 1 != 0) != expected.country_leader {
            report.difference(
                format!("{label}.country_leader"),
                actual.flags & 1,
                expected.country_leader,
            );
        }
    }
    for (user, players) in saved_users {
        report.limitation("saved header retains users absent from the current live roster; saved-only entries do not prove missing live players", format!("user {:?}: {} header entries", String::from_utf8_lossy(user), players.len()));
    }
    let mut controller_countries = BTreeSet::new();
    for controller in frame.complete::<tables::HumanControllers>()? {
        let tag = required_country_tag(controller.country, links)?;
        if !controller_countries.insert(tag) {
            return Err(invalid("duplicate resolved human-controller country"));
        }
    }
    report.record_country_emissions(save);
    let saved_countries: BTreeSet<_> = report
        .canonical_countries(&save.countries)
        .into_iter()
        .map(|(tag, _)| tag.as_slice())
        .collect();
    if controller_countries != saved_countries {
        report.difference(
            "human controller canonical country coverage",
            controller_countries,
            saved_countries,
        );
    }
    report.limitation("saved player header is refreshed by user-based upserts and is not an independent complete live-roster census", "Players");
    report.limitation("save carries CHuman user, signed id, resolved country and leader bit only; display name, upper flag bits and raw country alias bits have no independent save representation", "Players");
    report.limitation("native signed controller counts are not serialized and are not universally equal to saved player entry counts", "HumanControllers");
    Ok(())
}

pub fn compare_boundary_counters(
    frame: &DecodedFrame,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    let [clock] = frame.complete::<tables::Clock>()? else {
        return Err(invalid("clock table must contain one clock"));
    };
    let saved = &save.counters;
    let recorded: [(&str, Option<i64>, i64); 13] = [
        (
            "debug_current_ref_id",
            clock.debug_current_ref_id.map(i64::from),
            i64::from(saved.debug_current_ref_id),
        ),
        ("unit", clock.unit.map(i64::from), i64::from(saved.unit)),
        (
            "order_index",
            clock.order_index.map(i64::from),
            i64::from(saved.order_index),
        ),
        (
            "front_index",
            clock.front_index.map(i64::from),
            i64::from(saved.front_index),
        ),
        (
            "theatre_index",
            clock.theatre_index.map(i64::from),
            i64::from(saved.theatre_index),
        ),
        (
            "theater_group_index",
            clock.theater_group_index.map(i64::from),
            i64::from(saved.theater_group_index),
        ),
        (
            "military_deployment_line_index",
            clock.military_deployment_line_index.map(i64::from),
            i64::from(saved.military_deployment_line_index),
        ),
        (
            "military_deployment_conveyor_index",
            clock.military_deployment_conveyor_index.map(i64::from),
            i64::from(saved.military_deployment_conveyor_index),
        ),
        (
            "equipment_variant_index",
            clock.equipment_variant_index.map(i64::from),
            i64::from(saved.equipment_variant_index),
        ),
        (
            "country_leader_index",
            clock.country_leader_index.map(i64::from),
            i64::from(saved.country_leader_index),
        ),
        (
            "navy_id",
            clock.navy_id.map(i64::from),
            i64::from(saved.navy_id),
        ),
        (
            "land_combat_id",
            clock.land_combat_id.map(i64::from),
            i64::from(saved.land_combat_id),
        ),
        (
            "multiplayer_random_count",
            clock.multiplayer_random_count.map(i64::from),
            i64::from(saved.multiplayer_random_count),
        ),
    ];
    let mut unreadable = 0;
    let mut advanced = Vec::new();
    let mut regressed = Vec::new();
    for (name, captured, saved) in recorded {
        match captured {
            Some(captured) => {
                let gap = saved - captured;
                report.boundary_witness.push(format!(
                    "{name}: recorded {captured}, save {saved}, gap {gap:+}"
                ));
                match gap.cmp(&0) {
                    std::cmp::Ordering::Greater => advanced.push(format!("{name} {gap:+}")),
                    std::cmp::Ordering::Less => regressed.push(format!("{name} {gap:+}")),
                    std::cmp::Ordering::Equal => {}
                }
            }
            None => {
                unreadable += 1;
                report
                    .boundary_witness
                    .push(format!("{name}: recorded unavailable, save {saved}"));
            }
        }
    }
    report.compared(
        "boundary counter witnesses (printed, never a difference)",
        recorded.len() - unreadable,
    );
    let verdict = if unreadable == recorded.len() {
        "no counter was recorded at the sampled tick, so the boundary is not measured; this comparison establishes nothing about when the save was written".to_string()
    } else if advanced.is_empty() && regressed.is_empty() {
        format!(
            "all {} recorded counters equal the save header; this does not prove that the save matches the compared frame: writes after the save and untracked allocations remain possible",
            recorded.len() - unreadable
        )
    } else {
        let mut verdict = if regressed.is_empty() {
            String::from("no counter moved in the direction a frame read after the save allows")
        } else {
            format!(
                "engine work ran after the save and before the compared frame was read: {} counter(s) moved ({})",
                regressed.len(),
                regressed.join(", ")
            )
        };
        if !advanced.is_empty() {
            let warning = format!(
                "{} counter(s) are higher in the save than in the compared frame ({}), so its differences are not certified; take the save at a sampled hour: pause, recorder refresh, step one hour, save, then run until the recorder has sampled that hour before detaching",
                advanced.len(),
                advanced.join(", ")
            );
            report.incomplete("boundary", warning.clone());
            report.save_after_frame = Some(warning);
            verdict.push_str(&format!(
                "; {} counter(s) are higher in the save than in the compared frame ({}), which a monotonic counter cannot do without a reload, a 32-bit wrap or a frame read before the save (one no refresh settled), so neither direction is treated as a difference",
                advanced.len(),
                advanced.join(", ")
            ));
        }
        verdict
    };
    if unreadable != 0 && unreadable != recorded.len() {
        report.limitation(
            "the collector could not read some allocation counters at the sampled tick, so the boundary witness covers only the counters it did read",
            format!("{unreadable} of {} counters", recorded.len()),
        );
    }
    report.boundary_verdict = Some(verdict);
    report.limitation(
        "the recording does not mark which frames a refresh settled; the compared frame is taken to be one, read after the save as the next hourly update is entered, as the smoke arranges. A frame read straight after its hour's update precedes the commands run before the save, so its counters may be lower than the save's",
        "boundary counters",
    );
    report.limitation(
        "the boundary witness compares tracked engine counters between the sampled tick and the save; it never waives a difference, and equality does not exclude same-hour writes or untracked allocations",
        "boundary counters",
    );
    Ok(())
}


#[cfg(test)]
#[path = "core/nonpositive_state_tests.rs"]
mod nonpositive_state_tests;
