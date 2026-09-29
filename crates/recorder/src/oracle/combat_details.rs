//! Combat save coverage is narrower than the tick-refreshed presentation.
use super::*;

pub(super) fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::LandCombatDetails, |r| land(frame, save, r));
    report.attempt_table(TableId::LandCombatDivisions, |r| divisions(frame, save, r));
    report.attempt_table(TableId::LandCombatModifiers, |r| modifiers(frame, save, r));
    report.attempt_table(TableId::LandCombatSides, |r| sides(frame, save, r));
    report.attempt_table(TableId::NavalCombatDetails, |r| naval(frame, save, r));
    report.attempt_table(TableId::NavalCombatShips, |r| ships(frame, save, r));
    report.attempt_table(TableId::NavalCombatResultDetails, |r| {
        results(frame, save, r)
    });
}

/// One row per saved land combat. The save serializes neither country set of a
/// side, nor the indicator byte or flag words. Beyond the census, each primary
/// set must hold the logical owner of every saved unit on its side: `1413CC0D0`
/// appends the joining unit's logical owner to the side's `+0x38` set.
fn sides(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatSides>()?;
    let countries = country_context(frame)?;
    let mut seen = BTreeSet::new();
    let mut unsaved_units = 0_usize;
    for row in rows {
        if row.combat == 0 || !seen.insert(row.combat) {
            return Err(invalid("LandCombatSides: null or duplicate combat handle"));
        }
        let Some(combat) = save.land_combats.get(&row.combat) else {
            r.difference(
                format!("LandCombatSides {}", row.combat),
                "present",
                "absent",
            );
            continue;
        };
        for (label, units, primary) in [
            ("attacker", &combat.attackers, &row.attacker_primary),
            ("defender", &combat.defenders, &row.defender_primary),
        ] {
            let mut tags = BTreeSet::new();
            for &raw in primary {
                if let Some(tag) = country_tag(raw, &countries.links)? {
                    tags.insert(tag);
                }
            }
            for unit in units {
                let Some(division) = save.divisions.get(unit) else {
                    unsaved_units += 1;
                    continue;
                };
                r.scalar_checks += 1;
                if !tags.contains(division.logical_owner.as_slice()) {
                    r.difference(
                        format!("LandCombatSides {}.{label}_primary", row.combat),
                        format!(
                            "missing {}",
                            tag_text(Some(division.logical_owner.as_slice()))
                        ),
                        "every saved unit's logical owner",
                    );
                }
            }
        }
    }
    for combat in save.land_combats.keys().filter(|id| !seen.contains(id)) {
        r.difference(format!("LandCombatSides {combat}"), "absent", "present");
    }
    r.compared("LandCombatSides", rows.len());
    if unsaved_units > 0 {
        r.limitation(
            "A saved combat side names a unit the save's division rosters do not hold; its logical owner cannot be checked against the primary set.",
            "LandCombatSides",
        );
    }
    r.limitation(
        "The text save serializes neither side's country sets, the indicator byte nor the flag words. The census is compared, and each primary set must hold every saved unit's logical owner; the full ordered raw lists, secondary sets and operands need a paused live-memory check.",
        "LandCombatSides",
    );
    Ok(())
}

fn equal<T: PartialEq + std::fmt::Debug>(
    r: &mut Report,
    label: impl AsRef<str>,
    actual: T,
    expected: T,
) {
    r.scalar_checks += 1;
    if actual != expected {
        r.difference(label.as_ref(), actual, expected);
    }
}

// ADF40/AF5A0 text writing consumes c_str; retain raw suffixes in recording.
fn text_projection<'a>(r: &mut Report, label: &str, value: &'a [u8]) -> &'a [u8] {
    if let Some(end) = value.iter().position(|b| *b == 0) {
        r.limitation(
            "text serialization stops at the first NUL; captured suffix bytes are not comparable",
            label,
        );
        &value[..end]
    } else {
        value
    }
}
fn text_value(r: &mut Report, label: &str, actual: &[u8], saved: &[u8]) {
    let actual = text_projection(r, label, actual);
    equal(r, label, actual, saved);
}
fn optional_text_value(r: &mut Report, label: &str, actual: &[u8], saved: Option<&[u8]>) {
    let projected = text_projection(r, label, actual);
    equal(r, label, (!actual.is_empty()).then_some(projected), saved);
}
// The native serializer emits these scalar fields only when strictly positive.
// Absence proves <=0, not an exact zero, so keep the distinction explicit.
fn positive_decimal(r: &mut Report, label: &str, actual: i64, saved: Option<&Decimal>) {
    match saved {
        Some(saved) => equal(r, label, actual, saved.raw),
        None => equal(
            r,
            format!("{label} positive-field presence"),
            actual > 0,
            false,
        ),
    }
}

struct CommanderSaveView<'a> {
    reference: u64,
    category: Option<i32>,
    skill: i32,
    experience: Option<i64>,
    traits: &'a [replay::rows::LandCombatCommanderTrait],
}
fn commander_values(
    r: &mut Report,
    label: &str,
    a: CommanderSaveView<'_>,
    save: &Save,
) -> io::Result<()> {
    if a.reference == 0 {
        return Ok(());
    }
    if a.category.is_some_and(|v| !(0..=2).contains(&v)) {
        r.limitation(
            "character manager serializes only unit-leader categories 0, 1 and 2",
            label,
        );
        return Ok(());
    }
    let manager =
        save.characters.manager.as_ref().ok_or_else(|| {
            invalid("combat commander comparison requires saved character_manager")
        })?;
    let Some(b) = manager.unit_leaders.get(&a.reference) else {
        r.difference(
            format!("{label} commander"),
            a.reference,
            "absent from saved manager",
        );
        return Ok(());
    };
    equal(r, format!("{label} skill"), a.skill, b.skill);
    if let Some(v) = a.category {
        equal(r, format!("{label} category"), v, b.category);
    }
    if let Some(v) = a.experience {
        equal(r, format!("{label} experience"), v, b.experience.raw);
    }
    r.limitation(
        "the combat display name at leader+40 is distinct from the saved leader name at +20",
        label,
    );
    equal(
        r,
        format!("{label} assigned trait count"),
        a.traits.len(),
        b.traits.len(),
    );
    for (index, (a, b)) in a.traits.iter().zip(&b.traits).enumerate() {
        let label = format!("{label} assigned trait {index}");
        if a.token_id == engine::cpdx_lexer::FIXED15_TOKEN as i32 || a.token.contains(&0) {
            r.limitation("reserved numeric or interior-NUL trait token has no proven literal text projection",label);
        } else {
            equal(r, label, a.token.as_slice(), b.as_slice());
        }
    }
    Ok(())
}
fn naval_commander_values(
    r: &mut Report,
    label: &str,
    a: &replay::rows::NavalCommanderInputs,
    save: &Save,
) -> io::Result<()> {
    commander_values(
        r,
        label,
        CommanderSaveView {
            reference: a.commander,
            category: Some(a.category),
            skill: a.skills[0],
            experience: Some(a.experience),
            traits: &a.traits,
        },
        save,
    )?;
    if let Some(b) = save
        .characters
        .manager
        .as_ref()
        .and_then(|m| m.unit_leaders.get(&a.commander))
    {
        equal(
            r,
            format!("{label} component skills"),
            &a.skills[1..],
            b.components.as_slice(),
        );
    }
    Ok(())
}
/// Each saved formation's resolved HQ unit, keyed by its leader handle.
///
/// 140BDFD90 accessor the recorder resolves live) and omits it exactly when
/// the provider is null. Army groups carry no leader and never match a side
/// commander, so only leader-bearing definitions join.
fn hq_formations(save: &Save) -> io::Result<BTreeMap<u64, Option<u64>>> {
    let mut formations = BTreeMap::new();
    let countries = save
        .naval_history
        .theatre_countries
        .as_ref()
        .ok_or_else(|| invalid("HQ comparison requires saved theatre country roster"))?;
    for rows in countries.values() {
        for theatre in &rows.land {
            for definition in &theatre.definitions {
                if let Some(leader) = definition.leader
                    && formations.insert(leader, definition.leader_unit).is_some()
                {
                    return Err(invalid("saved leader commands more than one formation"));
                }
            }
        }
    }
    Ok(formations)
}

/// The badge records a gated provider handle. The save contains the provider,
/// but omits the runtime DLC/formation gates that can hide it.
fn hq_unit(
    r: &mut Report,
    label: &str,
    side: &replay::rows::LandCombatSideDetails,
    formations: &BTreeMap<u64, Option<u64>>,
) {
    let actual = side.hq_unit;
    if side.commander == 0 || (side.commander_category as u32) >= 2 {
        equal(r, format!("{label} hq_unit"), actual, 0);
        return;
    }
    if actual != 0 {
        equal(
            r,
            format!("{label} hq_unit committed membership"),
            side.committed.contains(&actual),
            true,
        );
    }
    match formations.get(&side.commander) {
        Some(saved) => {
            let expected = saved.unwrap_or(0);
            if expected == 0 || !side.committed.contains(&expected) {
                equal(r, format!("{label} hq_unit"), actual, 0);
            } else if actual != 0 {
                equal(r, format!("{label} hq_unit"), actual, expected);
            } else {
                r.limitation(
                    "a saved committed HQ provider does not prove badge visibility: runtime DLC and formation gates are not serialized; a hidden badge is not exactly comparable",
                    label,
                );
            }
        }
        None => r.limitation(
            "side commander has no saved formation; a same-hour reorganisation after the sampled tick moves leaders between groups, so the HQ handle is not comparable this hour",
            label,
        ),
    }
}

fn land(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatDetails>()?;
    let formations = hq_formations(save)?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.combat) {
            return Err(invalid("duplicate land detail identity"));
        }
        let Some(saved) = save.land_combats.get(&row.combat) else {
            r.difference(
                format!("LandCombatDetails {}", row.combat),
                "present",
                "absent",
            );
            continue;
        };
        let label = format!("LandCombatDetails {}", row.combat);
        equal(r, format!("{label} province"), row.province, saved.province);
        let terrain = text_projection(r, &label, &row.stored_terrain_name);
        equal(
            r,
            format!("{label} stored terrain"),
            (row.stored_terrain_valid != 0).then_some(terrain),
            saved.details.terrain.as_deref(),
        );
        for (index, (actual, side)) in row.sides.iter().zip(&saved.details.sides).enumerate() {
            let label = format!("{label}:{index}");
            commander_values(
                r,
                &format!("{label} commander"),
                CommanderSaveView {
                    reference: actual.commander,
                    category: Some(actual.commander_category),
                    skill: actual.commander_skill,
                    experience: Some(actual.commander_experience),
                    traits: &actual.traits,
                },
                save,
            )?;
            equal(
                r,
                format!("{label} committed"),
                &actual.committed,
                &side.committed,
            );
            equal(
                r,
                format!("{label} reserves"),
                &actual.reserves,
                &side.reserves,
            );
            equal(
                r,
                format!("{label} air counts"),
                &actual.air_counts,
                &side.air_counts,
            );
            equal(
                r,
                format!("{label} tactic"),
                (actual.tactic_kind == 2).then_some(actual.tactic_id),
                side.tactic,
            );
            hq_unit(r, &label, actual, &formations);
            positive_decimal(
                r,
                &format!("{label} anti_air_attack"),
                actual.anti_air_attack,
                side.anti_air.as_ref(),
            );
            match side.air_kills {
                Some(value) => equal(
                    r,
                    format!("{label} air_kills"),
                    actual.air_kills as i32,
                    value,
                ),
                None => equal(
                    r,
                    format!("{label} air_kills positive-field presence"),
                    (actual.air_kills as i32) > 0,
                    false,
                ),
            }
            for (lane, key) in save::combat_details::LAND_DAMAGE_KEYS.iter().enumerate() {
                positive_decimal(
                    r,
                    &format!("{label} {}", String::from_utf8_lossy(key)),
                    actual.damage[lane],
                    side.damage[lane].as_ref(),
                );
            }
            equal(
                r,
                format!("{label} org_loss_summary_index"),
                Some(actual.organisation_loss_index),
                side.organisation_loss_index,
            );
            equal(
                r,
                format!("{label} num_org_losses"),
                Some(actual.organisation_loss_count),
                side.organisation_loss_count,
            );
            if actual.organisation_loss_count >= 24 {
                equal(
                    r,
                    format!("{label} org_loss_summary"),
                    Some(actual.organisation_loss.to_vec()),
                    side.organisation_loss
                        .as_ref()
                        .map(|values| values.iter().map(|value| value.raw).collect()),
                );
            }
        }
    }
    for id in save.land_combats.keys().filter(|id| !seen.contains(id)) {
        r.difference(format!("LandCombatDetails {id}"), "absent", "present");
    }
    r.compared("LandCombatDetails", rows.len());
    r.limitation("Hourly samples cannot reproduce every tick or battles beginning and ending between samples. Ordered committed/reserve handles, air counts, positive damage/anti-air/kill fields, valid tactic ids and mature organisation-loss rings are compared. A positive-only omitted scalar proves only nonpositive, not its exact raw value. Before 24 organisation-loss observations the window does not consume the ring and the recording leaves its values unsampled; only index/count are compared. Saved terrain is compared to its separately captured stored terrain cache, never to the independent province-derived window terrain. Phase, width/bonus inputs, tactic definition metadata, commander runtime modifiers/traits and other presentation caches are not validated by these save matches", "LandCombatDetails");
    Ok(())
}

fn divisions(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatDivisions>()?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.side > 1 || !seen.insert((row.combat, row.side, row.ordinal)) {
            return Err(invalid("duplicate or invalid land division key"));
        }
        let label = format!(
            "LandCombatDivisions {}:{}:{}",
            row.combat, row.side, row.ordinal
        );
        let saved = save
            .land_combats
            .get(&row.combat)
            .ok_or_else(|| invalid("land detail division combat absent from save"))?;
        let members = if row.side == 0 {
            &saved.attackers
        } else {
            &saved.defenders
        };
        equal(
            r,
            format!("{label} ordered participant"),
            Some(row.division),
            members.get(row.ordinal as usize).copied(),
        );
        if let Some(commander) = &row.commander_inputs {
            commander_values(
                r,
                &format!("{label} commander"),
                CommanderSaveView {
                    reference: commander.handle,
                    category: None,
                    skill: commander.skill,
                    experience: None,
                    traits: &commander.traits,
                },
                save,
            )?;
        }
        let side = &saved.details.sides[row.side as usize];
        let role = if side.committed.contains(&row.division) {
            1
        } else if side.reserves.contains(&row.division) {
            2
        } else {
            0
        };
        if side.committed.contains(&row.division) && side.reserves.contains(&row.division) {
            return Err(invalid("saved division is both committed and reserve"));
        }
        equal(r, format!("{label} role"), row.role, role);
        let Some(unit) = save.divisions.get(&row.division) else {
            r.difference(label, "division present", "division absent");
            continue;
        };
        equal(
            r,
            format!("{label} strength"),
            row.strength,
            unit.hit_points.raw,
        );
        equal(
            r,
            format!("{label} organisation"),
            row.organisation,
            unit.organisation.raw,
        );
        equal(
            r,
            format!("{label} province"),
            Some(row.province),
            unit.province,
        );
        equal(
            r,
            format!("{label} template"),
            row.context.template,
            unit.template.unwrap_or(0),
        );
        for (name, actual, saved) in [
            (
                "entrenchment",
                row.entrenchment,
                unit.presentation.dig_in.as_ref(),
            ),
            (
                "entrenchment_cap",
                row.entrenchment_cap,
                unit.presentation.dig_in_cap.as_ref(),
            ),
        ] {
            if let Some(saved) = saved {
                equal(r, format!("{label} {name}"), actual, saved.raw);
            } else if actual > 0 {
                r.difference(
                    format!("{label} {name}"),
                    actual,
                    "omitted nonpositive field",
                );
            }
        }
    }
    for (&combat, saved) in &save.land_combats {
        for (side, members) in [(0, &saved.attackers), (1, &saved.defenders)] {
            for ordinal in 0..members.len() as u32 {
                if !seen.contains(&(combat, side, ordinal)) {
                    r.difference(
                        format!("LandCombatDivisions {combat}:{side}:{ordinal}"),
                        "absent",
                        "present",
                    );
                }
            }
        }
    }
    r.compared("LandCombatDivisions", rows.len());
    r.limitation("Only participant order/full handles, current HP/organisation, province, template, committed/reserve role and serialized entrenchment operands are compared. Direction keys, direction latch words and template-frame bits are live 141298E00/1415CA660 presentation operands with no text-save counterpart and are never compared here; their viewer replay is verified by render parity instead. Current combat stat block, raw maximum caches, attack/defense composition operands, initiative/reconnaissance and commander selection are not established by those matches. Current division names and combat commander names/skills/traits are hourly inputs; remaining template/icon dictionary joins retain their existing cadence. This table is an hourly input sample, not a per-attack damage event stream", "LandCombatDivisions");
    Ok(())
}

fn modifiers(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatModifiers>()?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.side > 1 || !seen.insert((row.combat, row.side)) {
            return Err(invalid("invalid land modifier side identity"));
        }
        let saved = save
            .land_combats
            .get(&row.combat)
            .ok_or_else(|| invalid("land modifiers combat absent from save"))?;
        let members = if row.side == 0 {
            &saved.attackers
        } else {
            &saved.defenders
        };
        equal(
            r,
            format!("LandCombatModifiers {}:{} census", row.combat, row.side),
            row.divisions.len(),
            members.len(),
        );
        for entry in &row.divisions {
            equal(
                r,
                format!(
                    "LandCombatModifiers {}:{}:{} member",
                    row.combat, row.side, entry.ordinal
                ),
                Some(entry.division),
                members.get(entry.ordinal as usize).copied(),
            );
        }
    }
    for combat in save.land_combats.keys() {
        for side in 0..2 {
            if !seen.contains(&(*combat, side)) {
                r.difference(
                    format!("LandCombatModifiers {combat}:{side}"),
                    "absent",
                    "present",
                );
            }
        }
    }
    r.compared("LandCombatModifiers", rows.len());
    r.limitation("Native ordered modifier id/A/B entries are runtime presentation inputs with no compared text-save counterpart. Ordinary side icons have static explanations, not numeric percentage fields; no invented percentage is recorded. Battle-history modifier durations are a different data structure and are not substitutes for active modifier values", "LandCombatModifiers");
    Ok(())
}

fn naval(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalCombatDetails>()?;
    let countries = country_context(frame)?;
    let actual: BTreeSet<_> = rows.iter().map(|x| x.combat).collect();
    let expected: BTreeSet<_> = save.naval_combats.keys().copied().collect();
    if actual.len() != rows.len() {
        return Err(invalid("duplicate naval details identity"));
    }
    equal(r, "NavalCombatDetails census", actual, expected);
    for row in rows {
        let Some(saved) = save.naval_combats.get(&row.combat) else {
            continue;
        };
        let label = format!("NavalCombatDetails {}", row.combat);
        equal(r, format!("{label} province"), row.province, saved.province);
        equal(
            r,
            format!("{label} progress"),
            Some(row.progress),
            saved.details.progress.as_ref().map(|v| v.raw),
        );
        equal(r, format!("{label} day"), Some(row.day), saved.details.day);
        equal(
            r,
            format!("{label} duration"),
            Some(row.duration),
            saved.details.duration,
        );
        for (side, actual) in row.sides.iter().enumerate() {
            let label = format!("{label}:{side}");
            if let Some(commander) = &actual.commander_inputs {
                naval_commander_values(r, &format!("{label} commander"), commander, save)?;
            }
            let (positioning, anti_air) = if side == 0 {
                (&saved.attacker_positioning, &saved.attacker_anti_air)
            } else {
                (&saved.defender_positioning, &saved.defender_anti_air)
            };
            equal(
                r,
                format!("{label} positioning"),
                actual.positioning,
                positioning.raw,
            );
            equal(
                r,
                format!("{label} anti_air"),
                actual.anti_air,
                anti_air.raw,
            );
            equal(
                r,
                format!("{label} recent joining"),
                actual.summary.recent_join_penalty,
                saved.details.recent_join_penalty[side]
                    .as_ref()
                    .map_or(0, |v| v.raw),
            );
            equal(
                r,
                format!("{label} escort bonus"),
                Some(actual.summary.escort_bonus),
                saved.details.escort_bonus[side].as_ref().map(|v| v.raw),
            );
            for source in 0..2 {
                let actual_planes: Vec<_> = actual
                    .planes
                    .iter()
                    .filter(|p| {
                        p.source == source && p.group_index < saved.details.group_counts[side]
                    })
                    .collect();
                let saved_planes: Vec<_> = saved.details.planes[side]
                    .iter()
                    .filter(|p| p.source == source)
                    .collect();
                equal(
                    r,
                    format!("{label} source {source} plane count"),
                    actual_planes.len(),
                    saved_planes.len(),
                );
                for (ordinal, (plane, saved_plane)) in
                    actual_planes.iter().zip(saved_planes).enumerate()
                {
                    plane_values(
                        r,
                        &format!("{label} source {source} plane {ordinal}"),
                        plane,
                        saved_plane,
                        &countries.links,
                    )?;
                }
            }
            if let Some(leader) = saved.details.leaders[side] {
                // The save holds the last leader (+0x118), which equals the shown
                // commander whenever that is nonnull. A load restores only the
                // last leader (cnaval_combatant::COMMANDER).
                if r.load_hour && actual.commander == 0 && leader != 0 {
                    r.limitation(
                        "Naval commander is null at the load hour: a load restores only the saved last_leader, and the game shows no commander until the next combat tick selects one",
                        format!("{label} commander: replay 0, save last_leader {leader}"),
                    );
                } else {
                    equal(r, format!("{label} commander"), actual.commander, leader);
                }
            }
            equal(
                r,
                format!("{label} retreating"),
                actual.retreating != 0,
                saved.details.retreating[side].unwrap_or(false),
            );
        }
    }
    r.compared("NavalCombatDetails", rows.len());
    r.limitation("Saved province/day/duration, positioning/anti-air, recent-join/escort operands, present commander handles, retreat truthiness and ordered serialized-group live/retired planes (counts, casualties, duration, wings, date, flags, names, damage multiplier, naval hits and first air-hit vector) are compared. Null/invalid leader omission does not prove raw null identity; boolean serialization loses nonzero byte distinctions. Capital/carrier screening, engaged flag, raw air-base handles omitted for unresolved references, independent source2 planes, paired orphan groups outside the serialized forward group list, carrier state, variant, five nonserialized plane caches and second air-hit vectors lack exact coverage here; matching other fields does not validate these caches or tick freshness", "NavalCombatDetails");
    Ok(())
}

fn ships(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalCombatShips>()?;
    let mut saved: BTreeMap<_, _> = save
        .naval_combat_participants
        .iter()
        .map(|(key, value)| (*key, (0, value)))
        .collect();
    for combat in save.naval_combats.values() {
        for (side, members) in combat.details.retired.iter().enumerate() {
            for member in members {
                if saved
                    .insert((combat.id, side as u8, member.battle_slot), (1, member))
                    .is_some()
                {
                    return Err(invalid("duplicate live/retired naval member slot"));
                }
            }
        }
    }
    let countries = country_context(frame)?;
    let mut seen = BTreeSet::new();
    let mut saved_seen = BTreeSet::new();
    for row in rows {
        let key = (row.combat, row.side, row.battle_slot);
        if !seen.insert(key) {
            return Err(invalid("duplicate naval detail slot"));
        }
        if row.group_index == u32::MAX {
            continue;
        }
        saved_seen.insert(key);
        let Some((source, member)) = saved.get(&key) else {
            r.difference(format!("NavalCombatShips {key:?}"), "present", "absent");
            continue;
        };
        equal(
            r,
            format!("NavalCombatShips {key:?} ship"),
            row.ship,
            member.ship.unwrap_or(0),
        );
        equal(
            r,
            format!("NavalCombatShips {key:?} state"),
            row.state,
            member.state,
        );
        equal(
            r,
            format!("NavalCombatShips {key:?} convoy"),
            row.convoy,
            member.convoy.unwrap_or(0),
        );
        equal(
            r,
            format!("NavalCombatShips {key:?} owner"),
            required_country_tag(row.owner, &countries.links)?,
            member.owner.as_slice(),
        );
        equal(
            r,
            format!("NavalCombatShips {key:?} member source"),
            row.member_source,
            *source,
        );
        let position = save.naval_combats.get(&row.combat).and_then(|combat| {
            combat.details.member_positions[row.side as usize].get(&row.battle_slot)
        });
        equal(
            r,
            format!("NavalCombatShips {key:?} saved group/member order"),
            Some([
                u32::from(row.member_source),
                row.group_index,
                row.member_index,
            ]),
            position.copied(),
        );
        let detail = &member.details;
        equal(
            r,
            format!("NavalCombatShips {key:?} last target"),
            (row.last_target.present != 0).then_some((
                row.last_target.is_convoy != 0,
                row.last_target.target_group_member_count,
            )),
            detail.last_target,
        );
        let label = format!("NavalCombatShips {key:?}");
        equal(
            r,
            format!("{label} hours_to_arrive"),
            Some(row.arrival_hours),
            detail.arrival_hours,
        );
        for (name, actual, saved) in [
            ("subunit_index", row.subunit_index, detail.subunit_index),
            ("formation_type", row.formation_type, detail.formation_type),
        ] {
            equal(r, format!("{label} {name}"), Some(actual), saved);
        }
        for (name, actual, saved) in [
            ("name", row.name.as_slice(), detail.name.as_deref()),
            (
                "type_name",
                row.type_name.as_slice(),
                detail.type_name.as_deref(),
            ),
        ] {
            optional_text_value(r, &format!("{label} {name}"), actual, saved);
        }
        let sprite = text_projection(r, &label, &row.sprite);
        equal(
            r,
            format!("{label} sprite"),
            Some(sprite),
            detail.sprite.as_deref(),
        );
        equal(
            r,
            format!("{label} variant"),
            row.variant,
            detail.variant.unwrap_or(0),
        );
        equal(
            r,
            format!("{label} convoy flag"),
            row.is_convoy != 0,
            detail.is_convoy.unwrap_or(false),
        );
        equal(
            r,
            format!("{label} hidden"),
            row.detection_state,
            detail.detection_state.unwrap_or(-1),
        );
        equal(
            r,
            format!("{label} escape_progress"),
            row.escape_progress,
            detail.escape_progress.as_ref().map_or(0, |v| v.raw),
        );
        equal(
            r,
            format!("{label} evacuated"),
            row.evacuation_state,
            detail.evacuation.unwrap_or(0),
        );
        match detail.criticals {
            Some(value) => equal(
                r,
                format!("{label} critical_hits_received"),
                row.received_criticals,
                value,
            ),
            None => equal(
                r,
                format!("{label} critical_hits_received positive-field presence"),
                row.received_criticals > 0,
                false,
            ),
        }
        equal(
            r,
            format!("{label} build_cost_ic"),
            row.build_cost_ic,
            detail.build_cost_ic.as_ref().map_or(0, |v| v.raw),
        );
        equal(
            r,
            format!("{label} cached strength"),
            row.cached_strength_ratio,
            detail.cached_strength.as_ref().map_or(0, |v| v.raw),
        );
        equal(
            r,
            format!("{label} pride_of_the_fleet"),
            row.pride != 0,
            detail.pride.unwrap_or(false),
        );
        optional_text_value(
            r,
            &format!("{label} sunk_by"),
            &row.sunk_by,
            detail.sunk_by.as_deref(),
        );
        equal(
            r,
            format!("{label} damage_received_by_gun_types"),
            Some(row.received_damage.to_vec()),
            detail
                .received_damage
                .as_ref()
                .map(|values| values.iter().map(|v| v.raw).collect()),
        );
        equal(
            r,
            format!("{label} naval hit count"),
            row.naval_hits.len(),
            detail.hits.len(),
        );
        for (ordinal, (hit, saved)) in row.naval_hits.iter().zip(&detail.hits).enumerate() {
            let label = format!("{label} naval_hit:{ordinal}");
            equal(r, format!("{label} target"), hit.target_slot, saved.target);
            text_value(r, &format!("{label} name"), &hit.target_name, &saved.name);
            equal(r, format!("{label} convoy"), hit.convoy != 0, saved.convoy);
            equal(r, format!("{label} damage"), hit.damage, saved.damage.raw);
            equal(
                r,
                format!("{label} strength"),
                hit.denominator,
                saved.strength.raw,
            );
            equal(
                r,
                format!("{label} last_hit"),
                hit.last_hit != 0,
                saved.last_hit,
            );
        }
        equal(
            r,
            format!("{label} air hit count"),
            row.air_hits[0].len(),
            detail.air_hits.len(),
        );
        for (ordinal, (hit, saved)) in row.air_hits[0].iter().zip(&detail.air_hits).enumerate() {
            let label = format!("{label} air_hit:{ordinal}");
            equal(
                r,
                format!("{label} variant"),
                hit.variant,
                saved.variant.unwrap_or(0),
            );
            equal(r, format!("{label} count"), hit.count, saved.count);
            equal(
                r,
                format!("{label} tag"),
                country_tag(hit.country, &countries.links)?.unwrap_or(b"---"),
                saved.country.as_slice(),
            );
        }
        equal(
            r,
            format!("{label} damage source count"),
            row.damage_entries.len(),
            detail.damage_entries.len(),
        );
        for (ordinal, (damage, saved)) in row
            .damage_entries
            .iter()
            .zip(&detail.damage_entries)
            .enumerate()
        {
            let label = format!("{label} damage_received:{ordinal}");
            equal(r, format!("{label} ship"), damage.source, saved.ship);
            equal(
                r,
                format!("{label} tag"),
                country_tag(damage.country, &countries.links)?.unwrap_or(b"---"),
                saved.country.as_slice(),
            );
            equal(
                r,
                format!("{label} damage"),
                damage.damage,
                saved.damage.raw,
            );
        }

        if row.live_ship != 0 {
            let unit = save
                .ships
                .get(&row.ship)
                .ok_or_else(|| invalid("naval detail live ship absent from save"))?;
            equal(
                r,
                format!("NavalCombatShips {key:?} strength"),
                row.strength,
                unit.strength.raw,
            );
            equal(
                r,
                format!("NavalCombatShips {key:?} organisation"),
                row.organisation,
                unit.organisation.raw,
            );
        }
    }
    for key in saved.keys().filter(|key| !saved_seen.contains(key)) {
        r.difference(format!("NavalCombatShips {key:?}"), "absent", "present");
    }
    r.compared("NavalCombatShips", saved_seen.len());
    r.limitation("Cached member presentation survives sunk ships and is distinct from current live ship HP/organisation. This comparison covers present cached name/type/sprite/model/category/variant, arrival, present detection/escape, six damage lanes, ordered naval hits and damage attribution. Writer141962a00 omits exactly zero evacuation/escape and -1 detection; positive-only omitted critical counts prove nonpositive, not exact zero. Writer141962d80 cached index/type, build-cost/strength, strings, variant and pride are also compared with their explicit omission rules. The first ordered air-hit vector is compared; the serializer omits the second vector entirely. Explicit orphan-group rows (group_index=u32::MAX) have no forward-group save entry and are excluded; source/group/member indices are compared for serialized groups. Raw live maximum caches and weapon/modifier context are not covered by these member scalar matches; nested serialized plane histories are compared under NavalCombatDetails. Boolean comparisons lose nonzero-byte distinctions. Naval results and country combat-log views are separate retained history, not a live-combat per-tick transcript", "NavalCombatShips");
    Ok(())
}

fn hit_values(
    r: &mut Report,
    label: &str,
    actual: &[replay::rows::NavalCombatHit],
    saved: &[save::combat_details::NavalHit],
) {
    equal(
        r,
        format!("{label} naval hit count"),
        actual.len(),
        saved.len(),
    );
    for (index, (a, b)) in actual.iter().zip(saved).enumerate() {
        let label = format!("{label} naval hit {index}");
        equal(r, format!("{label} target"), a.target_slot, b.target);
        text_value(r, &format!("{label} name"), &a.target_name, &b.name);
        equal(r, format!("{label} convoy"), a.convoy != 0, b.convoy);
        equal(r, format!("{label} damage"), a.damage, b.damage.raw);
        equal(
            r,
            format!("{label} denominator"),
            a.denominator,
            b.strength.raw,
        );
        equal(r, format!("{label} last hit"), a.last_hit != 0, b.last_hit);
    }
}
fn air_hit_values(
    r: &mut Report,
    label: &str,
    actual: &[replay::rows::NavalCombatAirHit],
    saved: &[save::combat_details::AirHit],
    links: &BTreeMap<u32, &CountryRow>,
) -> io::Result<()> {
    equal(
        r,
        format!("{label} air hit count"),
        actual.len(),
        saved.len(),
    );
    for (index, (a, b)) in actual.iter().zip(saved).enumerate() {
        let label = format!("{label} air hit {index}");
        equal(
            r,
            format!("{label} variant"),
            a.variant,
            b.variant.unwrap_or(0),
        );
        equal(r, format!("{label} count"), a.count, b.count);
        equal(
            r,
            format!("{label} country"),
            country_tag(a.country, links)?.unwrap_or(b"---"),
            b.country.as_slice(),
        );
    }
    Ok(())
}
fn plane_values(
    r: &mut Report,
    label: &str,
    a: &replay::rows::NavalCombatPlane,
    b: &save::combat_details::NavalPlane,
    links: &BTreeMap<u32, &CountryRow>,
) -> io::Result<()> {
    equal(
        r,
        format!("{label} group/index"),
        (a.group_index, a.index),
        (b.group_index, b.index),
    );
    equal(
        r,
        format!("{label} names count"),
        a.names.len(),
        b.names.len(),
    );
    for (index, (actual, saved)) in a.names.iter().zip(&b.names).enumerate() {
        text_value(r, &format!("{label} names {index}"), actual, saved);
    }
    equal(
        r,
        format!("{label} carrier"),
        a.carrier_slot,
        b.carrier.unwrap_or(-1),
    );
    if let Some(base) = b.air_base {
        equal(r, format!("{label} air base"), a.air_base, base);
    }
    equal(r, format!("{label} wings"), &a.wings, &b.wings);
    equal(r, format!("{label} initial"), a.initial_planes, b.initial);
    equal(r, format!("{label} current"), a.current_planes, b.current);
    equal(r, format!("{label} casualties"), a.casualties, b.casualties);
    equal(
        r,
        format!("{label} time duration"),
        a.time_duration,
        b.time_duration,
    );
    equal(
        r,
        format!("{label} country"),
        country_tag(a.country, links)?.unwrap_or(b"---"),
        b.country.as_slice(),
    );
    equal(r, format!("{label} date"), a.date, b.date.engine_hour()?);
    equal(
        r,
        format!("{label} flags"),
        a.flags.map(|v| v != 0),
        b.flags,
    );
    text_value(r, &format!("{label} name"), &a.name, &b.name);
    equal(
        r,
        format!("{label} damage multiplier"),
        a.cached_values[0],
        b.damage_mult.raw,
    );
    hit_values(r, label, &a.naval_hits, &b.naval_hits);
    air_hit_values(r, label, &a.air_hits[0], &b.air_hits, links)
}
fn result_ship_values(
    r: &mut Report,
    label: &str,
    a: &replay::rows::NavalResultShip,
    b: &save::combat_details::NavalMemberDetails,
    links: &BTreeMap<u32, &CountryRow>,
) -> io::Result<()> {
    equal(
        r,
        format!("{label} country"),
        Some(country_tag(a.owner, links)?.unwrap_or(b"---")),
        b.owner.as_deref(),
    );
    equal(
        r,
        format!("{label} subunit index"),
        Some(a.subunit_index),
        b.subunit_index,
    );
    equal(
        r,
        format!("{label} formation type"),
        Some(a.formation_type),
        b.formation_type,
    );
    equal(
        r,
        format!("{label} build cost"),
        a.build_cost_ic,
        b.build_cost_ic.as_ref().map_or(0, |v| v.raw),
    );
    equal(
        r,
        format!("{label} strength ratio"),
        a.strength_ratio,
        b.cached_strength.as_ref().map_or(0, |v| v.raw),
    );
    equal(
        r,
        format!("{label} pride"),
        a.pride != 0,
        b.pride.unwrap_or(false),
    );
    equal(
        r,
        format!("{label} convoy"),
        a.is_convoy != 0,
        b.is_convoy.unwrap_or(false),
    );
    optional_text_value(r, &format!("{label} name"), &a.name, b.name.as_deref());
    optional_text_value(
        r,
        &format!("{label} type name"),
        &a.type_name,
        b.type_name.as_deref(),
    );
    let sprite = text_projection(r, label, &a.sprite);
    equal(
        r,
        format!("{label} sprite"),
        Some(sprite),
        b.sprite.as_deref(),
    );
    optional_text_value(
        r,
        &format!("{label} sunk by"),
        &a.sunk_by,
        b.sunk_by.as_deref(),
    );
    equal(
        r,
        format!("{label} variant"),
        a.variant,
        b.variant.unwrap_or(0),
    );
    equal(
        r,
        format!("{label} convoy client"),
        a.convoy_client,
        b.convoy_client.unwrap_or(0),
    );
    match b.convoy_index {
        Some(index) => equal(r, format!("{label} convoy index"), a.convoy_index, index),
        None => equal(
            r,
            format!("{label} convoy index presence"),
            a.convoy_index >= 0,
            false,
        ),
    }
    hit_values(r, label, &a.naval_hits, &b.hits);
    air_hit_values(r, label, &a.air_hits[0], &b.air_hits, links)
}
fn results(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalCombatResultDetails>()?;
    let countries = country_context(frame)?;
    let actual: BTreeSet<_> = rows.iter().map(|x| x.result).collect();
    if actual.len() != rows.len() {
        return Err(invalid("duplicate retained naval result detail"));
    }
    equal(
        r,
        "NavalCombatResultDetails census",
        actual,
        save.naval_history.results.keys().copied().collect(),
    );
    for row in rows {
        let Some(saved) = save.naval_history.results.get(&row.result) else {
            continue;
        };
        for (side, (a, b)) in row.sides.iter().zip(&saved.sides).enumerate() {
            let label = format!("NavalCombatResultDetails {}:{side}", row.result);
            for (name, commander) in [
                ("commander", &a.commander_context),
                ("HQ commander", &a.hq_commander_context),
            ] {
                if let Some(commander) = commander {
                    naval_commander_values(r, &format!("{label} {name}"), commander, save)?;
                }
            }
            if let Some(leader) = b.details.leader {
                equal(r, format!("{label} leader"), a.leader, leader);
            }
            equal(
                r,
                format!("{label} ship count"),
                a.ships.len(),
                b.members.len(),
            );
            for (index, (a, b)) in a.ships.iter().zip(&b.members).enumerate() {
                let label = format!("{label} ship {index}");
                equal(r, format!("{label} member"), a.member, b.id);
                result_ship_values(r, &label, a, &b.details, &countries.links)?;
            }
            equal(
                r,
                format!("{label} plane count"),
                a.planes.len(),
                b.details.planes.len(),
            );
            for (index, (a, b)) in a.planes.iter().zip(&b.details.planes).enumerate() {
                let label = format!("{label} plane {index}");
                equal(r, format!("{label} variant"), a.variant, b.variant);
                equal(r, format!("{label} initial"), a.initial_planes, b.initial);
                equal(r, format!("{label} current"), a.current_planes, b.current);
                match b.lost {
                    Some(v) => equal(r, format!("{label} killed"), a.lost_planes, v),
                    None => equal(
                        r,
                        format!("{label} killed presence"),
                        a.lost_planes > 0,
                        false,
                    ),
                }
                equal(
                    r,
                    format!("{label} country"),
                    country_tag(a.country, &countries.links)?.unwrap_or(b"---"),
                    b.country.as_slice(),
                );
                text_value(r, &format!("{label} name"), &a.name, &b.name);
                hit_values(r, &label, &a.naval_hits, &b.naval_hits);
                air_hit_values(r, &label, &a.air_hits[0], &b.air_hits, &countries.links)?;
            }
        }
    }
    r.compared("NavalCombatResultDetails", rows.len());
    r.limitation("Retained result serializers 140CD7010/140CD7500/140CD6EE0 cover ordered ship and plane snapshots, cached identities/stats/strings, naval hits and first air-hit lists. Omitted positive-only killed counts prove only nonpositive. Invalid/absent leader references cannot validate raw identity. HQ leader/influence inputs and each second air-hit list are omitted by those serializers. These retained terminal results are independent of live combat retired-slot lists and country combat-log aggregates", "NavalCombatResultDetails");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_naval_commander_is_a_limitation_only_at_an_observed_load_hour() {
        use replay::branches::BranchReason::{
            HourDiscontinuity as Gap, LoadSignal as Load, SessionStart as Start,
        };
        use replay::branches::{BranchStart, Parentage};
        use replay::rows::{NavalCombatDetailsRow, NavalCombatSideDetails};

        let (mut frame, mut save, _, _) = super::super::tests::fixture();
        let zero = Decimal::parse(b"0").unwrap();
        let combat = 17;
        let leader = (37_u64 << 32) | 4713;
        save.naval_combats.insert(
            combat,
            save::NavalCombat {
                id: combat,
                province: 2,
                attacker_positioning: zero.clone(),
                defender_positioning: zero.clone(),
                attacker_anti_air: zero.clone(),
                defender_anti_air: zero.clone(),
                details: save::combat_details::NavalDetails {
                    day: Some(0),
                    duration: Some(0),
                    progress: Some(zero.clone()),
                    leaders: [Some(leader), None],
                    escort_bonus: [Some(zero.clone()), Some(zero)],
                    ..Default::default()
                },
            },
        );
        let set_commander = |frame: &mut DecodedFrame, commander| {
            *frame.tables[TableId::NavalCombatDetails.index()] =
                TableView::Complete(TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
                    combat,
                    province: 2,
                    sides: [
                        NavalCombatSideDetails {
                            commander,
                            ..Default::default()
                        },
                        NavalCombatSideDetails::default(),
                    ],
                    ..Default::default()
                }]));
        };
        let compare = |frame: &DecodedFrame, save: &Save, branch: BranchStart| {
            let mut report = Report {
                hour: frame.hour,
                ..Default::default()
            };
            report.set_load_hour(&branch);
            naval(frame, save, &mut report).unwrap();
            assert_eq!(report.compared_tables["NavalCombatDetails"], 1);
            report
        };
        let limited = |report: &Report| {
            report
                .limitations
                .keys()
                .any(|key| key.starts_with("Naval commander is null at the load hour"))
        };
        let id = frame.branch;
        let branch = |reason, loaded_hour, first_hour| BranchStart {
            id,
            session: 1,
            loaded_hour,
            first_hour,
            parent: Parentage::Unknown,
            reason,
        };

        // Only a null commander at an observed load's loaded and first sampled
        // hour is a limitation; any other branch start, hour or value differs.
        let h = frame.hour;
        for (reason, loaded, first, sample, commander, expected) in [
            (Load, Some(h), h, h, 0, "limited"),
            (Load, Some(h), h, h, leader, "equal"),
            (Load, Some(h), h, h, leader + 1, "differs"),
            (Load, Some(h), h, h + 1, 0, "differs"),
            (Load, None, h, h, 0, "differs"),
            (Load, Some(h), h + 1, h, 0, "differs"),
            (Start, Some(h), h, h, 0, "differs"),
            (Gap, Some(h), h, h, 0, "differs"),
        ] {
            frame.hour = sample;
            set_commander(&mut frame, commander);
            let report = compare(&frame, &save, branch(reason, loaded, first));
            let outcome = match (report.differences.len(), limited(&report)) {
                (0, true) => "limited",
                (0, false) => "equal",
                (1, false) => "differs",
                _ => panic!("{report:?}"),
            };
            assert_eq!(
                outcome, expected,
                "{reason:?} {loaded:?} {first} {sample} {commander}"
            );
        }

        // Other fields of the same side stay exact at the load hour.
        frame.hour = h;
        set_commander(&mut frame, 0);
        save.naval_combats.get_mut(&combat).unwrap().details.day = Some(1);
        let report = compare(&frame, &save, branch(Load, Some(h), h));
        assert!(limited(&report));
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("day"));
    }

    #[test]
    fn positive_only_writer_does_not_invent_zero_or_waive_missing_positive() {
        let mut report = Report::default();
        positive_decimal(&mut report, "omitted negative", -10, None);
        positive_decimal(&mut report, "omitted zero", 0, None);
        assert!(report.differences.is_empty());
        positive_decimal(&mut report, "missing positive", 1, None);
        positive_decimal(
            &mut report,
            "wrong raw",
            2,
            Some(&Decimal::parse(b"0.00001").unwrap()),
        );
        assert_eq!(report.differences.len(), 2);
        assert_eq!(report.scalar_checks, 4);
    }
    #[test]
    fn hit_comparison_keeps_missing_entries_and_raw_damage_differences() {
        let a = replay::rows::NavalCombatHit {
            target_slot: u32::MAX,
            target_name: b"Convoy".to_vec(),
            convoy: 2,
            damage: 2,
            denominator: 100000,
            last_hit: 0,
        };
        let b = save::combat_details::NavalHit {
            target: u32::MAX,
            name: b"Convoy".to_vec(),
            convoy: true,
            damage: Decimal::parse(b"0.00001").unwrap(),
            strength: Decimal::parse(b"1").unwrap(),
            last_hit: false,
        };
        let mut report = Report::default();
        hit_values(&mut report, "slot", &[a.clone(), a], &[b]);
        assert_eq!(report.differences.len(), 2);
        assert!(
            report
                .differences
                .iter()
                .any(|v| v.contains("naval hit count"))
        );
        assert!(report.differences.iter().any(|v| v.contains("damage")));
    }
    #[test]
    fn text_suffix_limitation_preserves_nonempty_field_presence() {
        let mut report = Report::default();
        optional_text_value(&mut report, "empty", b"", None);
        optional_text_value(&mut report, "NUL prefix", b"\0retained suffix", Some(b""));
        assert!(report.differences.is_empty());
        optional_text_value(&mut report, "missing field", b"\0retained suffix", None);
        assert_eq!(report.differences.len(), 1);
    }
    #[test]
    fn hq_badge_checks_comparable_handles_and_reports_unsaved_visibility() {
        let formations = BTreeMap::from([(7_u64, Some(11_u64)), (9, None)]);
        let side = |commander, hq_unit| replay::rows::LandCombatSideDetails {
            commander,
            hq_unit,
            commander_category: 0,
            committed: vec![11, 12],
            ..Default::default()
        };
        let mut report = Report::default();
        hq_unit(&mut report, "shown", &side(7, 11), &formations);
        hq_unit(&mut report, "no provider", &side(9, 0), &formations);
        hq_unit(&mut report, "no commander", &side(0, 0), &formations);
        hq_unit(&mut report, "runtime hidden", &side(7, 0), &formations);
        hq_unit(&mut report, "reorganised", &side(13, 0), &formations);
        assert!(report.differences.is_empty());
        assert!(!report.limitations.is_empty());
        hq_unit(&mut report, "wrong provider", &side(7, 12), &formations);
        hq_unit(&mut report, "no provider", &side(9, 11), &formations);
        hq_unit(&mut report, "no commander", &side(0, 11), &formations);
        assert_eq!(report.differences.len(), 3);
        let mut uncommitted = side(7, 11);
        uncommitted.committed.clear();
        hq_unit(&mut report, "not committed", &uncommitted, &formations);
        assert_eq!(report.differences.len(), 5);
        let mut ineligible = side(7, 11);
        ineligible.commander_category = 2;
        hq_unit(
            &mut report,
            "ineligible commander",
            &ineligible,
            &formations,
        );
        assert_eq!(report.differences.len(), 6);
    }
}
