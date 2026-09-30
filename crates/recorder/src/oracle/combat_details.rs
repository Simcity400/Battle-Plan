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
    report.attempt_table(TableId::NavalCombatAirBaseWingHandles, |r| {
        air_base_wings(frame, r)
    });
}

/// `NavalCombatAirBaseWingHandles` joins: exactly the bases the hour's naval combats reference
/// (active forward planes drawing on a base or host, and live ships' carrier bases) have rows,
/// and every base and wing is in the air censuses. The save keeps no dense container order.
fn air_base_wings(frame: &DecodedFrame, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalCombatAirBaseWingHandles>()?;
    let bases: BTreeSet<u64> = frame
        .complete::<tables::AirBases>()?
        .iter()
        .map(|base| base.id)
        .collect();
    let wings: BTreeSet<u64> = frame
        .complete::<tables::AirWings>()?
        .iter()
        .map(|wing| wing.id)
        .collect();
    let mut referenced = BTreeSet::new();
    for detail in frame.complete::<tables::NavalCombatDetails>()? {
        for plane in detail.sides.iter().flat_map(|side| &side.planes) {
            if plane.source == 0
                && plane.group_index != u32::MAX
                && plane.flags[1] == 0
                && matches!(plane.efficiency.source, 2 | 3)
            {
                referenced.insert(plane.air_base);
            }
        }
    }
    for ship in frame.complete::<tables::NavalCombatShips>()? {
        if ship.live_ship != 0 && ship.weapon_context.air_base != 0 {
            referenced.insert(ship.weapon_context.air_base);
        }
    }
    let listed: BTreeSet<u64> = rows.iter().map(|row| row.base).collect();
    equal(
        r,
        "NavalCombatAirBaseWingHandles bases",
        &listed,
        &referenced,
    );
    for row in rows {
        if !bases.contains(&row.base) {
            r.difference(
                format!("NavalCombatAirBaseWingHandles {}", row.base),
                "absent from AirBases",
                "an air base",
            );
        }
        for wing in row.wings.iter().filter(|wing| !wings.contains(wing)) {
            r.difference(
                format!("NavalCombatAirBaseWingHandles {} wing {wing}", row.base),
                "absent from AirWings",
                "an air wing",
            );
        }
    }
    r.compared("NavalCombatAirBaseWingHandles", rows.len());
    r.limitation(
        "the save keeps no dense air-base container order; only the joins are checked",
        "NavalCombatAirBaseWingHandles",
    );
    Ok(())
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
    /// The raw name token at leader+20, which the save writes as `name`.
    name_token: Option<&'a [u8]>,
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
    match a.name_token {
        Some(token) => {
            optional_text_value(r, &format!("{label} name token"), token, b.name.as_deref())
        }
        None => r.limitation(
            "the combat display name at leader+40 is distinct from the saved leader name at +20",
            label,
        ),
    }
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
            name_token: None,
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

/// writes `leader` exactly when the +0x88 pointer is set; army groups carry
/// no leader and never match a side commander.
fn formations(save: &Save) -> io::Result<BTreeMap<u64, u64>> {
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
                    && formations.insert(leader, definition.id).is_some()
                {
                    return Err(invalid("saved leader commands more than one formation"));
                }
            }
        }
    }
    Ok(formations)
}

/// The commander's forward formation reference (`CArmyLeader+0x1048`) against
/// the saved group that names it as leader, the reverse pointer.
fn formation(
    r: &mut Report,
    label: &str,
    side: &replay::rows::LandCombatSideDetails,
    formations: &BTreeMap<u64, u64>,
) {
    let actual = side.commander_formation;
    if side.commander == 0 || (side.commander_category as u32) >= 2 {
        equal(r, format!("{label} commander formation"), actual, 0);
        return;
    }
    match formations.get(&side.commander) {
        Some(&saved) => equal(r, format!("{label} commander formation"), actual, saved),
        None => r.limitation(
            "side commander leads no saved formation; its forward reference has no text-save counterpart this hour",
            label,
        ),
    }
}

/// The side commander's timed traits (+0xDE0) against `traits_to_remove`
/// (0x3969), which C0D920 writes in vector order as each trait's +8 token and
/// signed remaining days, omitting an empty vector. The saved leader's
/// presence is `commander_values`' comparison.
fn timed_traits(
    r: &mut Report,
    label: &str,
    side: &replay::rows::LandCombatSideDetails,
    save: &Save,
    tokens: &TokenNames<'_>,
) {
    if side.commander == 0 {
        equal(
            r,
            format!("{label} timed traits"),
            side.timed_traits.len(),
            0,
        );
        return;
    }
    if !(0..=2).contains(&side.commander_category) {
        return;
    }
    let Some(saved) = save
        .characters
        .manager
        .as_ref()
        .and_then(|manager| manager.unit_leaders.get(&side.commander))
    else {
        return;
    };
    equal(
        r,
        format!("{label} timed trait count"),
        side.timed_traits.len(),
        saved.timed_traits.len(),
    );
    for (index, (a, b)) in side
        .timed_traits
        .iter()
        .zip(&saved.timed_traits)
        .enumerate()
    {
        let label = format!("{label} timed trait {index}");
        equal(r, format!("{label} days"), a.remaining_days, b.days);
        match u32::try_from(a.token_id).ok().and_then(|id| tokens.get(&id)) {
            Some(literal) if a.token_id != 0x167 && !literal.contains(&0) => equal(
                r,
                format!("{label} token"),
                *literal,
                b.token.as_slice(),
            ),
            _ => r.limitation(
                "timed trait token has no recorded lexical spelling or is the reserved numeric formatter token; only its days are compared",
                label,
            ),
        }
    }
}

fn land(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatDetails>()?;
    let formations = formations(save)?;
    // Only timed traits need the dictionary; its absence limits them alone.
    let timed = rows
        .iter()
        .any(|row| row.sides.iter().any(|side| !side.timed_traits.is_empty()));
    let tokens = match timed.then(|| lexical_tokens(frame, r)) {
        None => TokenNames::new(),
        Some(Ok(tokens)) => tokens,
        Some(Err(error)) => {
            r.limitation(
                "recorded lexical dictionary is incomplete; timed trait tokens are not compared",
                error.to_string(),
            );
            TokenNames::new()
        }
    };
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
                    name_token: Some(&actual.commander_name_token),
                    traits: &actual.traits,
                },
                save,
            )?;
            timed_traits(r, &format!("{label} commander"), actual, save, &tokens);
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
            formation(r, &label, actual, &formations);
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
    r.limitation("Hourly samples cannot reproduce every tick or battles beginning and ending between samples. Ordered committed/reserve handles, air counts, positive damage/anti-air/kill fields, valid tactic ids, commander name tokens, ordered timed traits (tokens and signed days) and formation references, and mature organisation-loss rings are compared; the saved loss-ring index has no recorded counterpart (no window or tooltip reads it: only serialization and 1412A2AA0's count test touch +0x1A8/+0x1AC). A positive-only omitted scalar proves only nonpositive, not its exact raw value. Before 24 organisation-loss observations the window does not consume the ring and the recording leaves its values unsampled; only the count is compared. Saved terrain is compared to its separately captured stored terrain cache, never to the independent province-derived window terrain. Phase, width/bonus inputs, tactic definition metadata, commander runtime modifiers/traits and other presentation caches are not validated by these save matches", "LandCombatDetails");
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
                    reference: row.commander,
                    category: None,
                    skill: commander.skill,
                    experience: None,
                    name_token: Some(&commander.name_token),
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
        naming(r, &label, row, unit, &save.division_templates);
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
    r.limitation("Only participant order/full handles, province, template, committed/reserve role, division commander name tokens and the naming operands are compared; hit points, organisation and entrenchment are the Divisions and DivisionStatus oracle's. The naming type, order word, ordered and programmatic flags and a player's custom name are compared against the saved division_name block (1409BCC70), and the selected group key against the saved division_names_group of the member's template (140B977F0); a programmatic override is compared where the game shows it (a type-zero member whose group is null, 1409B5640) and is otherwise generated text the game does not show, never recorded. Direction latch words, exile operands and the stat block's equipment category mask (`categories`, *(CUnit+0x138)+0x5A8, read by CEquipmentTypeTrigger 141390370) are live 141298E00/1415CA660/141390370 inputs with no text-save counterpart (the per-unit stat block is rebuilt, not saved) and are never compared here; their viewer replay is verified by render parity instead. Current combat stat block, raw maximum caches, attack/defense composition operands, initiative/reconnaissance and commander selection are not established by those matches. Current division names and combat commander names/skills/traits are hourly inputs; remaining template/icon dictionary joins retain their existing cadence. This table is an hourly input sample, not a per-attack damage event stream", "LandCombatDivisions");
    Ok(())
}

/// CArmy C80E00 saves the +0x340 CNameGroupMember as `division_name` through
/// 1409BCC70, whose emission rules fix what each omitted key proves: `type` is
/// always written, `name_order` only when nonzero, `is_name_ordered` (as `no`)
/// only when +0xA8 is clear, `override` only for a nonempty +0x88 string and
/// `override_set_programmatically` only when +0xA9 is set.
fn naming(
    r: &mut Report,
    label: &str,
    row: &replay::rows::LandCombatDivisionRow,
    unit: &save::Division,
    templates: &BTreeMap<u64, save::DivisionTemplate>,
) {
    let Some(saved) = &unit.presentation.division_name else {
        r.difference(format!("{label} division_name"), "present", "absent");
        return;
    };
    equal(r, format!("{label} name type"), row.name_type, saved.kind);
    equal(
        r,
        format!("{label} name_order"),
        row.name_order,
        saved.order.unwrap_or(0),
    );
    equal(
        r,
        format!("{label} is_name_ordered"),
        row.name_ordered != 0,
        saved.ordered.unwrap_or(true),
    );
    equal(
        r,
        format!("{label} override_set_programmatically"),
        row.name_programmatic != 0,
        saved.programmatic.unwrap_or(false),
    );
    // SetTemplate 140C7EE00 and SelectFakeIntelDivisionTemplate 140C7E8C0
    // bind the member (1409BBC40) to the fake-intel template when set, else
    // the standard one; C80E00 saves those as the two template ids, and
    // 140B977F0 writes a template's division_names_group only for a set
    // group. `None` when the template has no saved entry.
    let group = match unit.alternate_template.or(unit.template) {
        None => Some(None),
        Some(template) => templates
            .get(&template)
            .map(|saved| saved.names_group.as_deref()),
    };
    // 1409B5640 shows the stored +0x88 name with +0xA9 clear, or with it set
    // for a type-zero member whose group is null; the capture records it
    // exactly there.
    if row.name_programmatic == 0 || (row.name_type == 0 && group == Some(None)) {
        optional_text_value(
            r,
            &format!("{label} override"),
            &row.name_custom,
            saved.custom.as_deref(),
        );
    } else if row.name_type != 0 {
        equal(
            r,
            format!("{label} override"),
            row.name_custom.as_slice(),
            b"".as_slice(),
        );
        r.limitation(
            "a programmatic override of a nonzero name type is shown only when its ship, operative or railway-gun group is null (1409B7290), which the capture does not read, so it is not recorded; only its flag is compared",
            label,
        );
    } else if group.is_none() {
        r.limitation(
            "the name member's template has no saved division_templates entry, so whether the game shows its programmatic override, and its group key, are not comparable",
            label,
        );
        return;
    } else {
        // The group is set: 1409B5640 shows its entry, not the stored text.
        equal(
            r,
            format!("{label} override"),
            row.name_custom.as_slice(),
            b"".as_slice(),
        );
    }
    // The capture reads the group of a type-zero member whose stored name
    // is not shown.
    if row.name_type != 0 || !row.name_custom.is_empty() {
        return;
    }
    match group {
        Some(group) => optional_text_value(r, &format!("{label} name group"), &row.name_group, group),
        None => r.limitation(
            "the name member's template has no saved division_templates entry; its group key is not comparable",
            label,
        ),
    }
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
            // 1413CC0D0 inserts only positive, canonically distinct roster owners.
            let mut seen = BTreeSet::new();
            if actual
                .summary
                .secondary_countries
                .iter()
                .any(|&raw| (raw as i32) <= 0 || !seen.insert(raw))
            {
                return Err(invalid(
                    "naval side secondary country list is not positive and distinct",
                ));
            }
            if let Some(commander) = &actual.commander_inputs {
                naval_commander_values(r, &format!("{label} commander"), commander, save)?;
            }
            // 14160FF40 writes every entry of the side's +0x20 unit array, in order.
            equal(
                r,
                format!("{label} units"),
                &actual.units,
                if side == 0 {
                    &saved.attackers
                } else {
                    &saved.defenders
                },
            );
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
    r.limitation("Compared against the save: the combat census; province, progress, day and duration; per side the ordered task-force `unit` list, the stored positioning and anti-air, the recent-join penalty (absent read as zero) and escort bonus, the commander handle when the save names a leader, the commander's category, skill, experience, traits and component skills from the saved character, and retreat truthiness; per serialized forward group, the ordered air and airs_to_delete planes with their count, group/index, carrier slot (absent read as -1), air base where saved, both wing lists, initial/current planes, casualties, time duration, country, date, both flags as booleans, ordered naval hits and first air-hit vector. Not compared though the save stores them: each plane's names list, name and damage multiplier (CFEXAir::vf2@14196A2E0 writes +0xA8, +0x88 and +0xC0), which this table does not record because no replay consumer reads them. Not compared because the save has no counterpart: capital and carrier screening, the engaged byte, the primary country list, the secondary roster-owner list (checked only as positive and distinct), the summary modifier cells, their definition metadata, feature flags and callback, each plane's carrier-member presence (the serializer writes the +0x18 slot, not the +0x10 member pointer), carrier state, variant, wing kills, every sortie-efficiency operand (source, pool flags, mission, container and crowding, night, raw modifiers and the definition metadata of keys 0x22, 0x23, 0x24 and 0x1B6), the second air-hit vector, source-2 planes and planes of paired-only groups outside the serialized forward groups. The serializer omits an unresolved air-base reference, so its raw handle is not compared; a null or invalid leader omission does not prove raw null identity; boolean serialization loses nonzero byte distinctions. Matching fields do not validate tick freshness", "NavalCombatDetails");
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
    use replay::{
        frame::TableView,
        rows::{
            AirBaseRow, AirWingRow, NavalCombatAirBaseWingHandlesRow, NavalCombatShipRow, TableRows,
        },
    };

    #[test]
    fn air_base_wing_rows_must_be_exactly_the_referenced_bases_in_the_air_censuses() {
        fn set(frame: &mut DecodedFrame, rows: TableRows) {
            let index = rows.table().index();
            *frame.tables[index] = TableView::Complete(rows);
        }
        let (mut frame, ..) = super::super::tests::fixture();
        let base = (3_u64 << 32) | 65;
        set(
            &mut frame,
            TableRows::AirBases(vec![AirBaseRow {
                id: base,
                ..Default::default()
            }]),
        );
        set(
            &mut frame,
            TableRows::AirWings(vec![AirWingRow {
                id: 9,
                ..Default::default()
            }]),
        );
        set(&mut frame, TableRows::NavalCombatDetails(Vec::new()));
        let mut ship = NavalCombatShipRow {
            live_ship: 1,
            ..Default::default()
        };
        ship.weapon_context.air_base = base;
        set(&mut frame, TableRows::NavalCombatShips(vec![ship]));
        set(
            &mut frame,
            TableRows::NavalCombatAirBaseWingHandles(vec![NavalCombatAirBaseWingHandlesRow {
                base,
                wings: vec![9, 9],
            }]),
        );
        let mut report = Report::default();
        air_base_wings(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        set(
            &mut frame,
            TableRows::NavalCombatAirBaseWingHandles(vec![
                NavalCombatAirBaseWingHandlesRow {
                    base,
                    wings: vec![10],
                },
                NavalCombatAirBaseWingHandlesRow {
                    base: base + (1 << 32),
                    wings: Vec::new(),
                },
            ]),
        );
        let mut report = Report::default();
        air_base_wings(&frame, &mut report).unwrap();
        // An unreferenced base, a base absent from AirBases and a wing absent from AirWings.
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    }

    use replay::rows::{LandCombatCommanderTimedTrait, LandCombatSideDetails};

    /// Each side's task forces compare with the saved `unit` list in writer order.
    #[test]
    fn naval_side_units_compare_with_the_saved_unit_list_in_order() {
        use replay::{
            frame::TableView,
            rows::{NavalCombatDetailsRow, TableId, TableRows},
        };
        let (frame, mut saved, _, _) = crate::oracle::tests::fixture();
        let combat = (5_u64 << 32) | 4713;
        let zero = || installed_content::save::Decimal::parse(b"0").unwrap();
        let units = [(244_u64 << 32) | 61, (9 << 32) | 61];
        saved.naval_combats.insert(
            combat,
            installed_content::save::NavalCombat {
                id: combat,
                province: 2,
                attacker_positioning: zero(),
                defender_positioning: zero(),
                attacker_anti_air: zero(),
                defender_anti_air: zero(),
                details: Default::default(),
                transfers: Vec::new(),
                attackers: units.to_vec(),
                defenders: Vec::new(),
            },
        );
        let differences = |recorded: Vec<u64>| {
            let mut row = NavalCombatDetailsRow {
                combat,
                province: 2,
                ..Default::default()
            };
            row.sides[0].units = recorded;
            let mut frame = frame.clone();
            *frame.tables[TableId::NavalCombatDetails.index()] =
                TableView::Complete(TableRows::NavalCombatDetails(vec![row]));
            let mut report = Report::default();
            naval(&frame, &saved, &mut report).unwrap();
            report
                .differences
                .iter()
                .filter(|line| line.contains(" units:"))
                .count()
        };
        assert_eq!(differences(units.to_vec()), 0);
        assert_eq!(differences(vec![units[1], units[0]]), 1);
        assert_eq!(differences(vec![units[0]]), 1);
    }

    #[test]
    fn timed_traits_compare_saved_tokens_and_signed_days_in_vector_order() {
        let save = installed_content::save::parse(
            br#"HOI4txt
            date=1942.9.6.2 game_unique_id=test version=test game_unique_seed=1 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
            countries={} states={} provinces={}
            equipment_market={contracts={}} supply_system_2={countries={}}
            character_manager={historical={character={id={id=8 type=73} token=x
                corps_commander={id={id=4 type=4713} skill=1 traits_to_remove={sick=20 wounded=-1}}
            }}}
            "#,
        )
        .unwrap();
        let timed = |token_id, remaining_days| LandCombatCommanderTimedTrait {
            token_id,
            definition_ordinal: 0,
            remaining_days,
        };
        let side = LandCombatSideDetails {
            commander: (4 << 32) | 4713,
            commander_category: 1,
            timed_traits: vec![timed(1, 20), timed(2, -1)],
            ..Default::default()
        };
        let tokens: TokenNames<'static> =
            BTreeMap::from([(1, b"sick".as_slice()), (2, b"wounded".as_slice())]);
        let compare = |side: &LandCombatSideDetails, tokens: &TokenNames<'_>| {
            let mut report = Report::default();
            timed_traits(&mut report, "side", side, &save, tokens);
            report
        };
        let report = compare(&side, &tokens);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        // Vector order and exact signed days, never a set.
        let swapped = LandCombatSideDetails {
            timed_traits: vec![timed(2, -1), timed(1, 20)],
            ..side.clone()
        };
        assert_eq!(compare(&swapped, &tokens).differences.len(), 4);
        // An unspelled token limits only its own spelling; days still compare.
        let report = compare(&side, &TokenNames::new());
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.values().flatten().count(), 2);
        // A capture longer than the save is a count difference.
        let extra = LandCombatSideDetails {
            timed_traits: vec![timed(1, 20), timed(2, -1), timed(1, 3)],
            ..side.clone()
        };
        assert_eq!(compare(&extra, &tokens).differences.len(), 1);
        // No commander carries no timed traits.
        let absent = LandCombatSideDetails {
            commander: 0,
            ..side.clone()
        };
        assert_eq!(compare(&absent, &tokens).differences.len(), 1);
    }

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
                attackers: Vec::new(),
                defenders: Vec::new(),
                transfers: Vec::new(),
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
    fn division_naming_follows_the_division_name_writer_and_member_template() {
        let (standard, fake) = (1_u64 << 32 | 52, 2_u64 << 32 | 52);
        let template = |group: Option<&[u8]>| save::DivisionTemplate {
            template: 0,
            country: b"SOV".to_vec(),
            name: Vec::new(),
            custom_icon: 0,
            custom_icon_nato: false,
            foreign_template_tag: None,
            composition: BTreeMap::new(),
            names_group: group.map(<[u8]>::to_vec),
        };
        let templates = BTreeMap::from([
            (standard, template(Some(b"SOV_INF_01"))),
            (fake, template(None)),
        ]);
        let unit = |name: Option<save::DivisionName>, alternate| save::Division {
            id: 7,
            roster_owner: b"SOV".to_vec(),
            logical_owner: b"SOV".to_vec(),
            expeditionary_owner: None,
            province: Some(1),
            template: Some(standard),
            alternate_template: alternate,
            separate_counter: false,
            hit_points: Decimal::parse(b"1").unwrap(),
            organisation: Decimal::parse(b"1").unwrap(),
            country_intel: Vec::new(),
            presentation: save::DivisionPresentation {
                division_name: name,
                ..Default::default()
            },
        };
        // An ordered historical member: every conditional key is omitted.
        let row = replay::rows::LandCombatDivisionRow {
            name_order: 0,
            name_ordered: 1,
            name_group: b"SOV_INF_01".to_vec(),
            ..Default::default()
        };
        let mut report = Report::default();
        let saved = Some(save::DivisionName::default());
        naming(
            &mut report,
            "ordered",
            &row,
            &unit(saved.clone(), None),
            &templates,
        );
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        // A fake-intel template supplies the member's group instead.
        naming(
            &mut report,
            "fake intel",
            &row,
            &unit(saved.clone(), Some(fake)),
            &templates,
        );
        assert_eq!(report.differences.len(), 1);
        // A player's name, an unordered member and a nonzero order word.
        let custom = replay::rows::LandCombatDivisionRow {
            name_order: -1,
            name_custom: b"Guards".to_vec(),
            ..Default::default()
        };
        let named = save::DivisionName {
            order: Some(-1),
            ordered: Some(false),
            custom: Some(b"Guards".to_vec()),
            ..Default::default()
        };
        let mut report = Report::default();
        naming(
            &mut report,
            "custom",
            &custom,
            &unit(Some(named.clone()), None),
            &templates,
        );
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        // A programmatic override with a set group is not shown (1409B5640)
        // and not recorded: the group is compared instead.
        let programmatic = replay::rows::LandCombatDivisionRow {
            name_programmatic: 1,
            name_custom: Vec::new(),
            name_group: b"SOV_INF_01".to_vec(),
            ..custom.clone()
        };
        let generated = save::DivisionName {
            programmatic: Some(true),
            custom: Some(b"1. Strelkovaya".to_vec()),
            ..named.clone()
        };
        naming(
            &mut report,
            "generated",
            &programmatic,
            &unit(Some(generated.clone()), None),
            &templates,
        );
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(report.limitations.is_empty());
        // With a null group (the fake-intel template saves none) the game
        // shows it, and it is compared.
        let shown = replay::rows::LandCombatDivisionRow {
            name_custom: b"1. Strelkovaya".to_vec(),
            name_group: Vec::new(),
            ..programmatic.clone()
        };
        naming(
            &mut report,
            "shown",
            &shown,
            &unit(Some(generated.clone()), Some(fake)),
            &templates,
        );
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        for (label, row, alternate) in [
            // Not recorded where the game shows it.
            ("unrecorded", &programmatic, Some(fake)),
            // Recorded where the game shows the group's entry.
            ("recorded", &shown, None),
        ] {
            let mut report = Report::default();
            naming(
                &mut report,
                label,
                row,
                &unit(Some(generated.clone()), alternate),
                &templates,
            );
            assert!(
                report
                    .differences
                    .iter()
                    .any(|difference| difference.contains("override")),
                "{label}: {:?}",
                report.differences
            );
        }
        // A nonzero type's group is not read: a named limitation.
        let mut report = Report::default();
        naming(
            &mut report,
            "ship",
            &replay::rows::LandCombatDivisionRow {
                name_type: 1,
                ..programmatic.clone()
            },
            &unit(
                Some(save::DivisionName {
                    kind: 1,
                    ..generated
                }),
                None,
            ),
            &templates,
        );
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.limitations.len(), 1);
        // Each operand difference and a missing block are reported, not waived.
        for (label, saved) in [
            (
                "type",
                save::DivisionName {
                    kind: 1,
                    ..named.clone()
                },
            ),
            (
                "order",
                save::DivisionName {
                    order: None,
                    ..named.clone()
                },
            ),
            (
                "ordered",
                save::DivisionName {
                    ordered: None,
                    ..named.clone()
                },
            ),
            (
                "custom",
                save::DivisionName {
                    custom: None,
                    ..named.clone()
                },
            ),
            (
                "programmatic",
                save::DivisionName {
                    programmatic: Some(true),
                    ..named.clone()
                },
            ),
        ] {
            let mut report = Report::default();
            naming(
                &mut report,
                label,
                &custom,
                &unit(Some(saved), None),
                &templates,
            );
            assert_eq!(report.differences.len(), 1, "{label}");
        }
        let mut report = Report::default();
        naming(
            &mut report,
            "absent",
            &custom,
            &unit(None, None),
            &templates,
        );
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn commander_formations_match_the_saved_leader_group() {
        let formations = BTreeMap::from([(7_u64, 11_u64)]);
        let side = |commander, commander_formation| replay::rows::LandCombatSideDetails {
            commander,
            commander_formation,
            commander_category: 0,
            ..Default::default()
        };
        let mut report = Report::default();
        formation(&mut report, "led", &side(7, 11), &formations);
        formation(&mut report, "no commander", &side(0, 0), &formations);
        formation(&mut report, "unsaved", &side(13, 5), &formations);
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 1);
        formation(&mut report, "wrong group", &side(7, 12), &formations);
        formation(&mut report, "no commander", &side(0, 11), &formations);
        let mut navy = side(7, 11);
        navy.commander_category = 2;
        formation(&mut report, "navy leader", &navy, &formations);
        assert_eq!(report.differences.len(), 3);
    }
}
