//! Combat save coverage is narrower than the tick-refreshed presentation.
use super::*;

pub fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::LandCombatDetails, |r| land(frame, save, r));
    report.attempt_table(TableId::LandCombatDivisions, |r| divisions(frame, save, r));
    report.attempt_table(TableId::LandCombatModifiers, |r| modifiers(frame, save, r));
    report.attempt_table(TableId::LandCombatDivisionModifiers, |r| {
        division_modifiers(frame, save, r)
    });
    report.attempt_table(TableId::LandCombatSides, |r| sides(frame, save, r));
    report.attempt_table(TableId::NavalCombatDetails, |r| naval(frame, save, r));
    report.attempt_table(TableId::NavalCombatShips, |r| ships(frame, save, r));
    report.attempt_table(TableId::NavalCombatResultDetails, |r| {
        results(frame, save, r)
    });
    report.attempt_table(TableId::NavalCombatAirBaseWingHandles, |r| {
        air_base_wings(frame, r)
    });
    report.attempt_table(TableId::NavalModifierTrees, |r| naval_trees(frame, r));
    report.attempt_table(TableId::ReserveModifierTrees, |r| reserve_trees(frame, r));
}

/// `ReserveModifierTrees` holds exactly the sources the hour's reserves name:
/// each reserve's country (positive expeditionary owner, else roster owner),
/// and a commander under its leader modifier kind only where one is selected.
/// The trees and names are runtime caches no save serializes.
fn reserve_trees(frame: &DecodedFrame, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::ReserveModifierTrees>()?;
    let owners: std::collections::BTreeMap<u64, (u32, u32)> = frame
        .complete::<tables::Divisions>()?
        .iter()
        .map(|unit| (unit.id, (unit.roster_owner, unit.expeditionary_owner)))
        .collect();
    let mut countries = std::collections::BTreeSet::new();
    let mut commanders = std::collections::BTreeSet::new();
    for row in frame.complete::<tables::LandCombatDivisions>()? {
        if row.role != 2 {
            continue;
        }
        let &(roster, expeditionary) = owners
            .get(&row.division)
            .ok_or_else(|| invalid(format!("reserve {} has no Divisions row", row.division)))?;
        countries.insert(u64::from(if expeditionary as i32 > 0 {
            expeditionary
        } else {
            roster
        }));
        if row.context.leader_modifier_kind != 0 {
            commanders.insert((row.context.leader_modifier_kind, row.commander));
        }
    }
    let recorded: std::collections::BTreeSet<u64> = rows
        .iter()
        .filter(|row| row.kind == 0)
        .map(|row| row.owner)
        .collect();
    if recorded != countries {
        r.difference(
            "ReserveModifierTrees countries",
            format!("{recorded:?}"),
            format!("{countries:?}"),
        );
    }
    for row in rows.iter().filter(|row| row.kind != 0) {
        if !commanders.contains(&(row.kind, row.owner)) {
            r.difference(
                format!(
                    "ReserveModifierTrees kind {} commander {}",
                    row.kind, row.owner
                ),
                "present",
                "no reserve selects it",
            );
        }
    }
    r.limitation(
        "Reserve reinforcement-rate trees and commander names are runtime modifier caches the save omits; a commander whose rate is zero has no row; compare them against matched native hovers",
        "ReserveModifierTrees runtime trees",
    );
    r.compared("ReserveModifierTrees", rows.len());
    Ok(())
}

/// `NavalModifierTrees` holds only sources this hour's live combats reach:
/// each live side's modifier sources and the recorded sources (scopes
/// 0-15) of each escaping live ship and each carrier member a linked air
/// group names. The trees themselves are runtime caches no save serializes;
/// `recorder derived` checks them against the naval_tree_oracle samples.
fn naval_trees(frame: &DecodedFrame, r: &mut Report) -> io::Result<()> {
    use replay::naval_trees::{self, SideSource};
    let rows = frame.complete::<tables::NavalModifierTrees>()?;
    let details = frame.complete::<tables::NavalCombatDetails>()?;
    let ships = frame.complete::<tables::NavalCombatShips>()?;
    let mut reached = BTreeSet::new();
    for detail in details {
        for side in 0..2 {
            for source in SideSource::ALL {
                if let Some(owner) = naval_trees::side_source(
                    detail.combat,
                    &detail.sides[side],
                    &detail.sides[1 - side],
                    source,
                ) {
                    reached.insert(owner.key());
                }
            }
        }
    }
    for ship in ships.iter().filter(|ship| ship.live_ship != 0) {
        let carrier = details
            .iter()
            .filter(|detail| detail.combat == ship.combat)
            .filter_map(|detail| detail.sides.get(usize::from(ship.side)))
            .flat_map(|side| &side.planes)
            .any(|plane| {
                plane.source == 0
                    && plane.group_index != u32::MAX
                    && plane.variant != 0
                    && plane.carrier_state & 4 == 0
                    && plane.carrier_member_present != 0
                    && plane.carrier_slot >= 0
                    && plane.carrier_slot as u32 == ship.battle_slot
            });
        if ship.state != 8 && !carrier {
            continue;
        }
        for source in &ship.weapon_context.sources {
            if let Some(owner) = naval_trees::ship_source(ship.ship, &ship.weapon_context, source) {
                reached.insert(owner.key());
            }
        }
    }
    for row in rows {
        if !reached.contains(&row.key()) {
            r.difference(
                format!(
                    "NavalModifierTrees kind {} owner {:#x} sub {} reached by a live combat",
                    row.kind, row.owner, row.sub
                ),
                false,
                true,
            );
        }
    }
    r.limitation(
        "runtime modifier trees and their stored labels are not serialized in saves; recorder derived compares them with naval_tree_oracle samples",
        "NavalModifierTrees",
    );
    r.compared("NavalModifierTrees", rows.len());
    Ok(())
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
/// set must hold the logical owner of every saved unit on its side: the game
/// appends the joining unit's logical owner to the side's set.
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

// Text serialization consumes c_str; retain raw suffixes in the recording.
fn text_projection<'a>(r: &mut Report, label: &str, value: &'a [u8]) -> &'a [u8] {
    let prefix = installed_content::parser::c_string(value);
    if prefix.len() < value.len() {
        r.limitation(
            "text serialization stops at the first NUL; captured suffix bytes are not comparable",
            label,
        );
        prefix
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
    /// The leader's stored name token, which the save writes as `name`.
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
            "the combat display name is distinct from the saved leader name",
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
    tokens: &TokenNames<'_>,
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
    if let Some(traits) = r.recorded(
        &a.timed_traits,
        "NavalCommanderInputs.timed_traits",
        format!("{label} timed traits"),
    ) {
        timed_traits(r, label, a.commander, a.category, traits, save, tokens);
    }
    Ok(())
}

/// Each saved formation's handle, keyed by its leader handle. The game
/// writes `leader` exactly when the commander pointer is set; army groups carry
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

/// The commander's active formation reference against
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

/// Naval commanders' timed traits (+0xDE0) against `traits_to_remove`
/// (0x3969), which the timed-trait writer writes in vector order as each trait's +8 token and
/// signed remaining days, omitting an empty vector. The saved leader's
/// presence is `commander_values`' comparison.
fn timed_traits(
    r: &mut Report,
    label: &str,
    commander: u64,
    category: i32,
    actual: &[replay::rows::LandCombatCommanderTimedTrait],
    save: &Save,
    tokens: &TokenNames<'_>,
) {
    if commander == 0 {
        equal(r, format!("{label} timed traits"), actual.len(), 0);
        return;
    }
    if !(0..=2).contains(&category) {
        return;
    }
    let Some(saved) = save
        .characters
        .manager
        .as_ref()
        .and_then(|manager| manager.unit_leaders.get(&commander))
    else {
        return;
    };
    equal(
        r,
        format!("{label} timed trait count"),
        actual.len(),
        saved.timed_traits.len(),
    );
    for (index, (a, b)) in actual.iter().zip(&saved.timed_traits).enumerate() {
        let label = format!("{label} timed trait {index}");
        equal(r, format!("{label} days"), a.remaining_days, b.days);
        match u32::try_from(a.token_id).ok().and_then(|id| tokens.get(&id)) {
            Some(literal) if a.token_id != 0x167 && !literal.contains(&0) => {
                equal(r, format!("{label} token"), *literal, b.token.as_slice())
            }
            _ => r.limitation(
                "timed trait token has no recorded lexical spelling or is the reserved numeric formatter token; only its days are compared",
                label,
            ),
        }
    }
}

/// A land side's hover inputs against its saved combatant: the stored
/// anti-air (`anti_air_attack`) and the prevented lanes (`str/org` prevented
/// damage), which the serializer writes only when strictly
/// positive, the lanes at their display step; and the commander's timed traits in vector
/// order. The skill progress, offence and defence, description token and
/// shown learning rows have no saved counterpart (a limitation in `land`).
fn land_side_inputs(
    r: &mut Report,
    label: &str,
    actual: &replay::rows::LandCombatSideDetails,
    inputs: &replay::rows::LandCombatSideInputs,
    saved: &save::combat_details::LandSide,
    save: &Save,
    tokens: &TokenNames<'_>,
) {
    positive_decimal(
        r,
        &format!("{label} anti_air_attack"),
        inputs.anti_air_attack,
        saved.anti_air.as_ref(),
    );
    for (lane, value) in [4, 5].into_iter().zip(inputs.prevented_damage) {
        let field = format!(
            "{label} {}",
            String::from_utf8_lossy(save::combat_details::LAND_DAMAGE_KEYS[lane])
        );
        match saved.damage[lane].as_ref() {
            Some(saved) => {
                let expected = r.displayed(
                    "LandCombatDetails.side_inputs.prevented_damage",
                    replay::rows::LandCombatSideInputs::PREVENTED_STEP,
                    saved.raw,
                );
                equal(r, field, value, expected);
            }
            None => equal(
                r,
                format!("{field} positive-field presence"),
                value > 0,
                false,
            ),
        }
    }
    timed_traits(
        r,
        &format!("{label} commander"),
        actual.commander,
        actual.commander_category,
        &inputs.timed_traits,
        save,
        tokens,
    );
}

fn timed_trait_tokens<'a>(frame: &'a DecodedFrame, r: &mut Report, needed: bool) -> TokenNames<'a> {
    match needed.then(|| lexical_tokens(frame, r)) {
        None => TokenNames::new(),
        Some(Ok(tokens)) => tokens,
        Some(Err(error)) => {
            r.limitation(
                "recorded lexical dictionary is incomplete; timed trait tokens are not compared",
                error.to_string(),
            );
            TokenNames::new()
        }
    }
}

fn land(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatDetails>()?;
    let formations = formations(save)?;
    // Only timed traits need the dictionary; its absence limits them alone.
    let timed = rows.iter().any(|row| {
        matches!(&row.side_inputs, replay::rows::Recorded::Value(sides)
            if sides.iter().any(|side| !side.timed_traits.is_empty()))
    });
    let tokens = timed_trait_tokens(frame, r, timed);
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
        if let Some(border_war) = r.recorded(
            &row.border_war,
            "LandCombatDetails.border_war",
            format!("{label} border-war operands"),
        ) {
            equal(
                r,
                format!("{label} border-war class"),
                border_war.is_some(),
                saved.details.border_war,
            );
            if let Some(border) = border_war {
                let countries = country_context(frame)?;
                if border.transfer_state > 1 {
                    r.limitation("the game saves change_state_after_war as boolean; the magnitude of an unusual nonzero byte is unsaved", format!("{label} border transfer byte"));
                }
                // The save writes combat_width unconditionally, even at zero.
                if let Some(width) = r.recorded(
                    &border.combat_width,
                    "LandBorderWarInputs.combat_width",
                    format!("{label} combat_width"),
                ) {
                    r.scalar(
                        &label,
                        "combat_width",
                        Some(*width),
                        saved.details.combat_width.as_ref(),
                    );
                }
                equal(
                    r,
                    format!("{label} change_state_after_war"),
                    Some(border.transfer_state != 0),
                    saved.details.transfer_state,
                );
                for (index, (actual, side)) in
                    border.sides.iter().zip(&saved.details.sides).enumerate()
                {
                    equal(
                        r,
                        format!("{label}:{index} border tag"),
                        country_tag(actual.country as u32, &countries.links)?,
                        side.border_country.as_deref(),
                    );
                    equal(
                        r,
                        format!("{label}:{index} border state"),
                        Some(actual.state),
                        side.border_state,
                    );
                    equal(
                        r,
                        format!("{label}:{index} on_win"),
                        (!actual.win_event.is_empty()).then_some(actual.win_event.as_slice()),
                        side.border_win_event.as_deref(),
                    );
                    equal(
                        r,
                        format!("{label}:{index} on_lose"),
                        (!actual.lose_event.is_empty()).then_some(actual.lose_event.as_slice()),
                        side.border_lose_event.as_deref(),
                    );
                }
            }
        }
        equal(r, format!("{label} province"), row.province, saved.province);
        let terrain = text_projection(r, &label, &row.stored_terrain_name);
        equal(
            r,
            format!("{label} stored terrain"),
            (row.stored_terrain_valid != 0).then_some(terrain),
            saved.details.terrain.as_deref(),
        );
        let side_inputs = r.recorded(
            &row.side_inputs,
            "LandCombatDetails.side_inputs",
            format!("{label} side inputs"),
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
                    // The side records the commander's skill progress, the
                    // ratio its hover prints, not the raw experience.
                    experience: None,
                    name_token: Some(&actual.commander_name_token),
                    traits: &actual.traits,
                },
                save,
            )?;
            if let Some(inputs) = side_inputs.map(|sides| &sides[index]) {
                land_side_inputs(r, &label, actual, inputs, side, save, &tokens);
            }
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
            // The recorded air strength and organisation lanes; the ground
            // lanes are retired, and the prevented lanes are the side
            // inputs'.
            for ((key, &actual), saved) in save::combat_details::LAND_DAMAGE_KEYS
                .iter()
                .zip(&actual.damage)
                .zip(&side.damage)
            {
                positive_decimal(
                    r,
                    &format!("{label} {}", String::from_utf8_lossy(key)),
                    actual,
                    saved.as_ref(),
                );
            }
            equal(
                r,
                format!("{label} num_org_losses"),
                Some(actual.organisation_loss_count),
                side.organisation_loss_count,
            );
            if actual.organisation_loss_count >= 24 {
                // The row records the samples' qword sum, the one figure
                // The loss calculation takes from them; the save holds all 24.
                equal(
                    r,
                    format!("{label} org_loss_summary samples"),
                    Some(24),
                    side.organisation_loss.as_ref().map(Vec::len),
                );
                equal(
                    r,
                    format!("{label} org_loss_summary sum"),
                    Some(actual.organisation_loss_sum),
                    side.organisation_loss.as_ref().map(|values| {
                        values
                            .iter()
                            .fold(0_i64, |sum, value| sum.wrapping_add(value.raw))
                    }),
                );
            }
        }
    }
    for id in save.land_combats.keys().filter(|id| !seen.contains(id)) {
        r.difference(format!("LandCombatDetails {id}"), "absent", "present");
    }
    r.compared("LandCombatDetails", rows.len());
    r.limitation("Hourly samples cannot reproduce every tick or battles beginning and ending between samples. Ordered committed/reserve handles, air counts, positive air damage, anti-air, prevented damage (at its display step) and kill fields, valid tactic ids, commander name tokens, ordered timed traits (tokens and signed days) and formation references, and mature organisation-loss rings are compared; the saved loss-ring index has no recorded counterpart (no window or tooltip reads it). A positive-only omitted scalar proves only nonpositive, not its exact raw value. Before 24 organisation-loss observations the window does not consume the ring and the recording leaves its values unsampled; only the count is compared. Saved terrain is compared to its separately captured stored terrain cache, never to the independent province-derived window terrain. Phase, width/bonus inputs, tactic definition metadata, commander runtime modifiers/traits and other presentation caches are not validated by these save matches. The side hover inputs' skill progress (the save keeps the residual experience but not the next level's requirement), offence and defence (keys 0x5F/0x60 of a runtime leader block), description token and learning rows above the live LEADER_TRAITS_XP_SHOW have no saved counterpart and are not compared", "LandCombatDetails");
    Ok(())
}

fn divisions(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatDivisions>()?;
    // Each division's name member template reference, where a
    // name_template_witness build recorded it with the division's status.
    let statuses = frame.complete::<tables::DivisionStatus>()?;
    let references = witnessed_name_templates(statuses);
    let templates = NameTemplates::new(frame, save, r, !references.is_empty())?;
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
        // The collector composes a reserve speed share for every
        // reserve and no other division, so the share is present exactly
        // where the save lists the division among its side's reserves; its
        // value is a live input the save does not hold.
        if let Some(share) = r.recorded(
            &row.reinforcement_speed,
            "LandCombatDivisions.reinforcement_speed",
            format!("{label} reserve speed share"),
        ) {
            if share.is_none() && role == 2 {
                r.limitation(
                    "A reserve's speed share was refused at capture (an operand unreadable or a movement-speed form the port does not follow; collector status land_reserve_speed), so its reinforcement chance is unknown",
                    label.clone(),
                );
            } else {
                equal(
                    r,
                    format!("{label} reserve speed share presence"),
                    share.is_some(),
                    role == 2,
                );
            }
        }
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
        naming(
            r,
            &label,
            Naming::from(row).with_template(references.get(&row.division).copied()),
            unit,
            &templates,
        );
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
    r.limitation("Only participant order/full handles, province, template, committed/reserve role, the reserve speed share's presence on exactly the saved reserves and the naming operands are compared; the share's value (composed from live movement-speed operands and SLOWEST_SPEED) and the stat block's initiative have no save counterpart; hit points, organisation and entrenchment are the Divisions and DivisionStatus oracle's. The naming type, order word, ordered and programmatic flags and a player's custom name are compared against the saved division_name block, and the selected group key against the saved division_names_group of the member's template (its witnessed reference, a deleted one holding none, else the template a load rebinds); a programmatic override is compared where the game shows it (a type-zero member whose group is null) and is otherwise generated text the game does not show, never recorded. Direction latch words, exile operands and the stat block's equipment category mask (`categories`) are live combat inputs with no text-save counterpart (the per-unit stat block is rebuilt, not saved) and are never compared here; their viewer replay is verified by render parity instead. Current combat stat block, raw maximum caches, attack/defense composition operands, initiative/reconnaissance and commander selection are not established by those matches. Current division names are hourly inputs; remaining template/icon dictionary joins retain their existing cadence. This table is an hourly input sample, not a per-attack damage event stream", "LandCombatDivisions");
    Ok(())
}

/// Borrowed naming operands shared by combat and whole-roster comparisons.
pub struct Naming<'a> {
    pub kind: i32,
    pub order: i32,
    pub ordered: u8,
    pub programmatic: u8,
    pub custom: &'a [u8],
    pub group: &'a [u8],
    /// The member's own template reference (zero for none) when a
    /// `name_template_witness` build recorded it.
    pub template: Option<u64>,
}

impl<'a> Naming<'a> {
    /// These operands with the member's witnessed template reference.
    pub fn with_template(mut self, template: Option<u64>) -> Self {
        self.template = template;
        self
    }
}

/// The member template references a `name_template_witness` build recorded,
/// by division.
pub fn witnessed_name_templates(rows: &[replay::rows::DivisionStatusRow]) -> BTreeMap<u64, u64> {
    rows.iter()
        .filter_map(|row| match &row.name_template {
            replay::rows::Recorded::Value(replay::rows::Witnessed(Some(template))) => {
                Some((row.division, *template))
            }
            _ => None,
        })
        .collect()
}

/// The templates a name member's group is resolved against.
pub struct NameTemplates<'a> {
    /// The save's templates by full reference.
    pub saved: &'a BTreeMap<u64, save::DivisionTemplate>,
    /// Every template reference the recording's DivisionTemplates holds at
    /// this hour, or why it holds no current read.
    pub recorded: Result<BTreeSet<u64>, &'static str>,
}

impl<'a> NameTemplates<'a> {
    /// The save's templates, with the recording's own at this hour when a
    /// witnessed member reference may need them to prove a deletion.
    pub fn new(
        frame: &DecodedFrame,
        save: &'a Save,
        r: &Report,
        witnessed: bool,
    ) -> io::Result<Self> {
        let recorded = if !witnessed {
            Err("no member template reference is witnessed")
        } else if r.refresh_is_current(TableId::DivisionTemplates, frame.hour) {
            Ok(frame
                .complete::<tables::DivisionTemplates>()?
                .iter()
                .map(|row| row.template)
                .collect())
        } else {
            Err(
                "DivisionTemplates is not read at this hour, so a name member template with no saved division_templates entry cannot be proved deleted",
            )
        };
        Ok(Self {
            saved: &save.division_templates,
            recorded,
        })
    }
}

/// The group a type-zero member's generator resolves, as the save writes a
/// template's `division_names_group` (only for a set group): `Some(None)`
/// for no group, `None` when a difference or limitation was reported instead.
fn member_group<'t>(
    r: &mut Report,
    label: &str,
    witnessed: Option<u64>,
    unit: &save::Division,
    templates: &NameTemplates<'t>,
) -> Option<Option<&'t [u8]>> {
    // A load rebinds the member to the fake-intel template when set, else
    // the standard one; that stands in when the reference is not witnessed.
    let rebound = unit.alternate_template.or(unit.template);
    let Some(reference) = witnessed else {
        let Some(template) = rebound else {
            return Some(None);
        };
        let saved = templates.saved.get(&template);
        if saved.is_none() {
            r.difference(
                format!("{label} name template {template}"),
                "saved division_templates entry",
                "absent",
            );
        }
        return saved.map(|saved| saved.names_group.as_deref());
    };
    if reference == 0 {
        return Some(None);
    }
    if let Some(saved) = templates.saved.get(&reference) {
        return Some(saved.names_group.as_deref());
    }
    // The member can still name a template deleted since, which the game's
    // reference registry no longer resolves, so it holds no group. That is
    // proved only for a reference of the template kind (the type word of the
    // division's own template) that none of the recording's templates holds
    // at this hour.
    let kind = rebound
        .or_else(|| templates.saved.keys().next().copied())
        .map(|template| template as u32);
    if kind != Some(reference as u32) {
        r.difference(
            format!("{label} name member template {reference}"),
            "a division template reference",
            format!("{kind:?}"),
        );
        return None;
    }
    match &templates.recorded {
        Err(reason) => {
            r.limitation(reason, label);
            None
        }
        Ok(recorded) if recorded.contains(&reference) => {
            r.difference(
                format!("{label} name member template {reference}"),
                "saved division_templates entry",
                "absent",
            );
            None
        }
        Ok(_) => {
            r.compared("name members of deleted templates", 1);
            Some(None)
        }
    }
}

impl<'a> From<&'a replay::rows::LandCombatDivisionRow> for Naming<'a> {
    fn from(row: &'a replay::rows::LandCombatDivisionRow) -> Self {
        Self {
            kind: row.name_type,
            order: row.name_order,
            ordered: row.name_ordered,
            programmatic: row.name_programmatic,
            custom: &row.name_custom,
            group: &row.name_group,
            template: None,
        }
    }
}

impl<'a> From<&'a replay::rows::NameOperands> for Naming<'a> {
    fn from(row: &'a replay::rows::NameOperands) -> Self {
        Self {
            kind: row.kind,
            order: row.order,
            ordered: row.ordered,
            programmatic: row.programmatic,
            custom: &row.custom,
            group: &row.group,
            template: None,
        }
    }
}

/// The army writer saves the name member as `division_name` through
/// the name writer, whose emission rules fix what each omitted key proves: `type` is
/// always written, `name_order` only when nonzero, `is_name_ordered` (as `no`)
/// only when the ordered flag is clear, `override` only for a nonempty string and
/// `override_set_programmatically` only when the programmatic flag is set.
pub fn naming<'a>(
    r: &mut Report,
    label: &str,
    row: impl Into<Naming<'a>>,
    unit: &save::Division,
    templates: &NameTemplates<'_>,
) {
    let row = row.into();
    let Some(saved) = &unit.presentation.division_name else {
        r.difference(format!("{label} division_name"), "present", "absent");
        return;
    };
    equal(r, format!("{label} name type"), row.kind, saved.kind);
    equal(
        r,
        format!("{label} name_order"),
        row.order,
        saved.order.unwrap_or(0),
    );
    equal(
        r,
        format!("{label} is_name_ordered"),
        row.ordered != 0,
        saved.ordered.unwrap_or(true),
    );
    equal(
        r,
        format!("{label} override_set_programmatically"),
        row.programmatic != 0,
        saved.programmatic.unwrap_or(false),
    );
    // Only a type-zero member's generator resolves its template's group: to
    // decide whether a programmatic override shows, and for its group key.
    let group = if row.kind == 0 && (row.programmatic != 0 || row.custom.is_empty()) {
        match member_group(r, label, row.template, unit, templates) {
            Some(group) => group,
            None => return,
        }
    } else {
        None
    };
    // The game shows the stored name when the programmatic flag is clear, or set
    // for a type-zero member whose group is null.
    if row.programmatic == 0
        || (row.kind == 0 && group.is_none())
        || (row.kind == 2 && !row.custom.is_empty())
    {
        optional_text_value(
            r,
            &format!("{label} override"),
            row.custom,
            saved.custom.as_deref(),
        );
    } else if row.kind != 0 {
        equal(r, format!("{label} override"), row.custom, b"".as_slice());
        r.limitation(
            "a programmatic override of a nonzero name type is shown only when its ship, operative or railway-gun group is null, which the capture does not read, so it is not recorded; only its flag is compared",
            label,
        );
    } else {
        // The group is set: the game shows its entry, not the stored text.
        equal(r, format!("{label} override"), row.custom, b"".as_slice());
    }
    // The capture reads the group of a type-zero member whose stored name
    // is not shown.
    if row.kind != 0 || !row.custom.is_empty() {
        return;
    }
    optional_text_value(r, &format!("{label} name group"), row.group, group);
}

fn modifiers(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatModifiers>()?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.side > 1 || !seen.insert((row.combat, row.side)) {
            return Err(invalid("invalid land modifier side identity"));
        }
        if !save.land_combats.contains_key(&row.combat) {
            return Err(invalid("land modifiers combat absent from save"));
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
    r.limitation("Border modifier, dig-in and terrain factors are runtime presentation inputs with no compared text-save counterpart", "LandCombatModifiers");
    Ok(())
}

fn division_modifiers(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LandCombatDivisionModifiers>()?;
    // Rows by combat and side; each saved member has one.
    let mut counts = BTreeMap::new();
    for row in rows {
        let saved = save
            .land_combats
            .get(&row.combat)
            .ok_or_else(|| invalid("land division modifiers combat absent from save"))?;
        let members = if row.side == 0 {
            &saved.attackers
        } else {
            &saved.defenders
        };
        *counts.entry((row.combat, row.side)).or_insert(0_usize) += 1;
        equal(
            r,
            format!(
                "LandCombatDivisionModifiers {}:{}:{} member",
                row.combat, row.side, row.ordinal
            ),
            Some(row.division),
            members.get(row.ordinal as usize).copied(),
        );
    }
    for (combat, saved) in &save.land_combats {
        for (side, members) in [(0_u8, &saved.attackers), (1, &saved.defenders)] {
            equal(
                r,
                format!("LandCombatDivisionModifiers {combat}:{side} census"),
                counts.get(&(*combat, side)).copied().unwrap_or(0),
                members.len(),
            );
        }
    }
    r.compared("LandCombatDivisionModifiers", rows.len());
    r.limitation("Native ordered modifier id/A/B entries are runtime presentation inputs with no compared text-save counterpart. Ordinary side icons have static explanations, not numeric percentage fields; no invented percentage is recorded. Battle-history modifier durations are a different data structure and are not substitutes for active modifier values", "LandCombatDivisionModifiers");
    Ok(())
}

fn naval(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalCombatDetails>()?;
    let tokens = if rows
        .iter()
        .any(|row| row.sides.iter().any(|side| !side.damage_by_type.is_empty()))
    {
        lexical_tokens(frame, r)?
    } else {
        timed_trait_tokens(
            frame,
            r,
            rows.iter().any(|row| {
                row.sides.iter().any(|side| {
                    side.commander_inputs.as_ref().is_some_and(|commander| {
                        commander
                            .timed_traits
                            .require("NavalCommanderInputs.timed_traits")
                            .is_ok_and(|traits| !traits.is_empty())
                    })
                })
            }),
        )
    };
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
            equal(
                r,
                format!("{label} total_initial_strength"),
                Some(actual.original_health),
                saved.details.original_health[side]
                    .as_ref()
                    .map(|value| value.raw),
            );
            equal(
                r,
                format!("{label} total_damage_dealt"),
                Some(actual.total_damage),
                saved.details.total_damage[side]
                    .as_ref()
                    .map(|value| value.raw),
            );
            match &saved.details.damage_history[side] {
                Some(values) => {
                    equal(
                        r,
                        format!("{label} damage_dealt_by_gun_types count"),
                        actual.damage_history.len() * actual.damage_history[0].len(),
                        values.len(),
                    );
                    for (index, (actual, saved)) in actual
                        .damage_history
                        .iter()
                        .flatten()
                        .zip(values)
                        .enumerate()
                    {
                        equal(
                            r,
                            format!("{label} damage_dealt_by_gun_types {index}"),
                            *actual,
                            saved.raw,
                        );
                    }
                }
                None => r.difference(
                    format!("{label} damage_dealt_by_gun_types"),
                    "present",
                    "absent",
                ),
            }
            match &saved.details.damage_by_type[side] {
                Some(saved) => {
                    equal(
                        r,
                        format!("{label} damage_dealt_by_ship_types count"),
                        actual.damage_by_type.len(),
                        saved.len(),
                    );
                    let mut named = BTreeMap::new();
                    let mut missing = Vec::new();
                    for source in &actual.damage_by_type {
                        match tokens.get(&source.token) {
                            Some(name) if !name.contains(&0) => {
                                if named.insert(*name, source.damage).is_some() {
                                    return Err(invalid(
                                        "duplicate naval damage source token spelling",
                                    ));
                                }
                            }
                            _ => missing.push(source.token),
                        }
                    }
                    if missing.is_empty() {
                        let saved: BTreeMap<_, _> = saved
                            .iter()
                            .map(|(name, damage)| (name.as_slice(), damage.raw))
                            .collect();
                        equal(
                            r,
                            format!("{label} damage_dealt_by_ship_types"),
                            named,
                            saved,
                        );
                    } else {
                        r.limitation("Naval damage source token has no recorded lexical spelling or contains an interior NUL; the source map cannot be compared by name", format!("{label} tokens {missing:?}"));
                    }
                }
                None => equal(
                    r,
                    format!("{label} damage_dealt_by_ship_types absent"),
                    actual.damage_by_type.is_empty(),
                    true,
                ),
            }
            // The game inserts only positive, canonically distinct roster owners.
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
                naval_commander_values(r, &format!("{label} commander"), commander, save, &tokens)?;
            }
            // The game writes every entry of the side's unit array, in order.
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
                // The save holds the last leader, which equals the shown
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
    r.limitation("Compared against the save: the combat census; province, progress, day and duration; per side the ordered task-force `unit` list, the stored positioning and anti-air, the recent-join penalty (absent read as zero) and escort bonus, the commander handle when the save names a leader, the commander's category, skill, experience, traits and component skills from the saved character, and retreat truthiness; per side original health, total damage, the 6x6 damage history and ordered source-token totals; per serialized forward group, the ordered air and airs_to_delete planes with their count, group/index, carrier slot (absent read as -1), air base where saved, both wing lists, initial/current planes, casualties, time duration, country, date, both flags as booleans, ordered naval hits and first air-hit vector. Not compared though the save stores it: each plane's damage multiplier, which this table does not record because no replay consumer reads it. Not compared because the save has no counterpart: capital and carrier screening, the engaged byte, the primary country list, the secondary roster-owner list (checked only as positive and distinct), the summary modifier cells, their definition metadata, feature flags and callback, each plane's carrier-member presence (the serializer writes the slot, not the member pointer), carrier state, variant, wing kills, every sortie-efficiency operand (source, pool flags, mission, container and crowding, night, raw modifiers and the definition metadata of keys 0x22, 0x23, 0x24 and 0x1B6), the second air-hit vector, source-2 planes and planes of paired-only groups outside the serialized forward groups. The save's frozen air-hover name and ordered names are localized presentation caches, so they are not compared with the viewer's new-language derivation. Custom sea grid entries are live-only and have no save counterpart. The serializer omits an unresolved air-base reference, so its raw handle is not compared; a null or invalid leader omission does not prove raw null identity; boolean serialization loses nonzero byte distinctions. Matching fields do not validate tick freshness", "NavalCombatDetails");
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
        // Stored at its display step (replay::display); the save's exact
        // value is rounded the same way.
        let escape = r.displayed(
            "NavalCombatShips.escape_progress",
            replay::rows::NavalCombatShipRow::ESCAPE_STEP,
            detail.escape_progress.as_ref().map_or(0, |v| v.raw),
        );
        equal(
            r,
            format!("{label} escape_progress"),
            row.escape_progress,
            escape,
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
            let organisation = r.displayed(
                "NavalCombatShips.organisation",
                replay::rows::NavalCombatShipRow::ORGANISATION_STEP,
                unit.organisation.raw,
            );
            equal(
                r,
                format!("NavalCombatShips {key:?} organisation"),
                row.organisation,
                organisation,
            );
        }
    }
    for key in saved.keys().filter(|key| !saved_seen.contains(key)) {
        r.difference(format!("NavalCombatShips {key:?}"), "absent", "present");
    }
    r.compared("NavalCombatShips", saved_seen.len());
    r.limitation("Cached member presentation survives sunk ships and is distinct from current live ship HP/organisation. This comparison covers present cached name/type/sprite/model/category/variant, arrival, present detection/escape, six damage lanes, ordered naval hits and damage attribution. The cached-member writer omits exactly zero evacuation/escape and -1 detection; positive-only omitted critical counts prove nonpositive, not exact zero. Cached index/type, build-cost/strength, strings, variant and pride are also compared with their explicit omission rules. The first ordered air-hit vector is compared; the serializer omits the second vector entirely. Explicit orphan-group rows (group_index=u32::MAX) have no forward-group save entry and are excluded; source/group/member indices are compared for serialized groups. Raw live maximum caches and weapon/modifier context are not covered by these member scalar matches; nested serialized plane histories are compared under NavalCombatDetails. Boolean comparisons lose nonzero-byte distinctions. Naval results and country combat-log views are separate retained history, not a live-combat per-tick transcript", "NavalCombatShips");
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
    // The game creates these two localized presentation caches.
    // The viewer derives them in its language from air-base and wing inputs;
    // the save's frozen recording-language bytes are not a row comparator.
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
    let tokens = timed_trait_tokens(
        frame,
        r,
        rows.iter().any(|row| {
            row.sides.iter().any(|side| {
                [&side.commander_context, &side.hq_commander_context]
                    .into_iter()
                    .flatten()
                    .any(|commander| {
                        commander
                            .timed_traits
                            .require("NavalCommanderInputs.timed_traits")
                            .is_ok_and(|traits| !traits.is_empty())
                    })
            })
        }),
    );
    let countries = country_context(frame)?;
    let actual: BTreeSet<_> = rows.iter().map(|x| x.result).collect();
    if actual.len() != rows.len() {
        return Err(invalid("duplicate retained naval result detail"));
    }
    let expected: BTreeSet<_> = save.naval_history.results.keys().copied().collect();
    let missing: Vec<_> = expected.difference(&actual).copied().collect();
    equal(
        r,
        "NavalCombatResultDetails unexpected results",
        actual
            .difference(&expected)
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::<u64>::new(),
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
                    naval_commander_values(
                        r,
                        &format!("{label} {name}"),
                        commander,
                        save,
                        &tokens,
                    )?;
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
    r.limitation("Retained result serializers cover ordered ship and plane snapshots, cached identities/stats/strings, naval hits and first air-hit lists. Omitted positive-only killed counts prove only nonpositive. Invalid/absent leader references cannot validate raw identity. HQ leader/influence inputs and each second air-hit list are omitted by those serializers. These retained terminal results are independent of live combat retired-slot lists and country combat-log aggregates", "NavalCombatResultDetails");
    if !missing.is_empty() {
        return Err(invalid(format!(
            "NavalCombatResultDetails: {} retained results have unknown details (missing rows): {missing:?}; readable rows were compared",
            missing.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
