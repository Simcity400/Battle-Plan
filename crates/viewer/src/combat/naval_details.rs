//! Native naval member grouping and retained ship-result categories (format 22).
use super::{
    HoverText, Selection, Values, Widget,
    binding::{Bindings, FLAG, Flags, NavalEntry, TEXT, country_flag, fill, hide, sprite, text},
    tooltips::{hover_text, set_hover},
};
use crate::{
    document::Document,
    map::{
        perspective::{SharingUnknown, Vision},
        tables::rows,
    },
    table_access::TokenNames,
};
use replay::{
    frame::DecodedFrame,
    rows::{
        AirBaseRow, AirWingRow, CountryRow, NavalCombatPlane, NavalCombatShipRow, NavalResultShip,
        VariantDefinitionRow, tables,
    },
};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    io,
};

struct AirGroup<'a> {
    side: u8,
    model: usize,
    owner: u32,
    planes: Vec<&'a NavalCombatPlane>,
}

/// 1417D9C60 visits the forward groups' active-air arrays before the ship
/// arrays. 1417D9510 joins a plane with the first entry having the same
/// CEquipmentType+0x418 registration ID and equivalent country.
fn air_group_slot(groups: &[AirGroup<'_>], side: u8, model: usize, owner: u32) -> Option<usize> {
    groups
        .iter()
        .position(|group| group.side == side && group.model == model && group.owner == owner)
}

fn display_air_plane(plane: &NavalCombatPlane) -> bool {
    plane.source == 0
        && plane.group_index != u32::MAX
        && plane.variant != 0
        && plane.carrier_state & 4 == 0
}

/// 1419660C0 obtains +0x88 from CAirBase::GetDisplayName@140C44FC0:
/// CState::display_name@1409CC380 for a state base, otherwise the host
/// CShip::GetName@140C29A20. 141963460 builds distinct +0xA8 short names
/// from the ordered +0x40 wings via 140F4A4B0/140BBAA50.
/// I: 1419663F0 only appends wings and 141966030 rebuilds the list; no
/// +0x40 removal path is known that would leave a stale +0xA8 name.
fn air_presentation(
    document: &Document,
    frame: &DecodedFrame,
    plane: &NavalCombatPlane,
    wings: &BTreeMap<u64, &AirWingRow>,
    bases: &BTreeMap<u64, &AirBaseRow>,
    countries: &BTreeMap<u32, &CountryRow>,
    ships: &BTreeMap<u64, &NavalCombatShipRow>,
) -> Result<(String, String), String> {
    let loc = &document.names.localisation;
    let base = bases
        .get(&plane.air_base)
        .copied()
        .ok_or_else(|| format!("CFEXAir base {} has no AirBases row", plane.air_base))?;
    let name = if let Some(id) = base.state {
        let states = rows::<tables::States>(frame).map_err(|error| error.to_string())?;
        if !states.is_complete() {
            return Err("States is incomplete for the naval air-base name".into());
        }
        let state = states
            .rows()
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("naval air base state {id} has no States row"))?;
        super::text::state_name(
            loc,
            &document.recorded_language,
            id,
            &state.custom_name,
            &state.owner_name,
            || {
                let key = document
                    .state_name_keys
                    .get(&id)
                    .ok_or_else(|| format!("naval air base state {id} has no content definition"))?
                    .as_ref()
                    .map_err(Clone::clone)?;
                super::text::localise(loc, key, &[])
            },
        )?
    } else if base.host == 0 {
        String::new()
    } else {
        let ship = ships
            .get(&base.host)
            .copied()
            .filter(|row| row.live_ship != 0)
            .ok_or_else(|| {
                format!(
                    "naval air-base host ship {} has no recorded combat name",
                    base.host
                )
            })?;
        dashboard::scripted_text::stored_text(
            loc,
            &document.recorded_language,
            &ship.name,
            "naval air-base host CShip::GetName",
        )?
    };
    if plane.wings[1].len() != plane.wing_types.len() {
        return Err("CFEXAir wing handles and type indices differ in length".into());
    }
    let sub_units = document
        .icons
        .combat
        .commander_tooltips
        .sub_units()
        .map_err(str::to_owned)?;
    let mut names = Vec::<String>::new();
    for (&handle, &index) in plane.wings[1].iter().zip(&plane.wing_types) {
        if index == u32::MAX {
            return Err(format!(
                "historical CFEXAir wing {handle} no longer resolves to its type"
            ));
        }
        let wing = wings
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("CFEXAir wing {handle} has no AirWings row"))?;
        let selector = if wing.country_override > 0 {
            wing.country_override as u32
        } else {
            wing.owner
        };
        let country = countries.get(&selector).copied().ok_or_else(|| {
            format!("CFEXAir wing {handle} country {selector} has no Countries row")
        })?;
        let tag = std::str::from_utf8(&country.tag)
            .map_err(|_| format!("CFEXAir wing {handle} country tag is not UTF-8"))?;
        let key = sub_units
            .name(index)
            .ok_or_else(|| format!("CFEXAir wing {handle} subunit index {index} is absent"))??;
        let short = super::naval_air_tooltips::wing_short_name(loc, tag, key)?;
        if !names.contains(&short) {
            names.push(short);
        }
    }
    Ok((name, names.join(", ")))
}

fn air_count_label(side: u8, tag: &str, current: i32, maximum: i32, low: bool) -> String {
    let colour = if low { 'R' } else { 'W' };
    if side == 0 {
        format!(
            "\u{11}{colour} {}{} {current}/{maximum}\u{11}!",
            super::INLINE_FLAG,
            tag
        )
    } else {
        format!(
            "\u{11}{colour} {current}/{maximum} {}{}\u{11}!",
            super::INLINE_FLAG,
            tag
        )
    }
}

fn air_icon_widget(name: Option<String>, side: u8) -> Widget {
    Widget {
        hidden: name.is_none(),
        sprite: name,
        // 1417CEAF0's side byte is 1 for left and 0 for right; 1417DAA40
        // sends byte+1 to CIcon. Native GUI bound frames are one based.
        frame: Some(if side == 0 { 2 } else { 1 }),
        // 1417DAA40 sets this dynamic hover after constructing the row.
        hover_owned: true,
        ..Widget::default()
    }
}

fn push_air_entry(
    out: &mut Bindings,
    heights: &mut BTreeMap<(u8, &str), f32>,
    side: u8,
    name: Option<String>,
    size: [f32; 2],
    label: Option<String>,
    hover: Option<HoverText>,
) {
    let mut values = Values::new();
    for key in [
        "bar_str",
        "bar_org",
        "icon_hidden",
        "escape_progress",
        "escape_icon_left",
        "escape_icon_right",
        "pride",
        "desc_left",
        "desc_right",
    ] {
        hide(&mut values, key);
    }
    values.insert("icon".into(), air_icon_widget(name, side));
    set_hover(&mut values, "icon", hover, None);
    if let Some(label) = label {
        let count_key = if side == 0 { "desc_left" } else { "desc_right" };
        text(&mut values, count_key, label);
        values.entry(count_key.into()).or_default().hidden = false;
    }
    let wnd = if side == 0 { "left" } else { "right" };
    let grid = format!("battle_window/top_wnd/{wnd}_wnd/grid_carrier");
    let height = heights.entry((side, "carrier")).or_default();
    out.naval_entries.push(NavalEntry {
        side,
        unknown: false,
        grid,
        offset: Some([combat_box_x(size[0]), *height]),
        size,
        values,
        battle_slots: Vec::new(),
    });
    *height += size[1] + 3.0;
}

#[allow(clippy::too_many_arguments)] // The hour's frame, the battle's identity and its draw state.
fn air_entries(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    flags: &Flags,
    aliases: &BTreeMap<u32, u32>,
    vision: &Vision,
    heights: &mut BTreeMap<(u8, &str), f32>,
) -> io::Result<()> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        if !details.is_complete() {
            out.refuse(format!(
                "Naval combat {id} air entries are omitted: NavalCombatDetails is incomplete."
            ));
        }
        return Ok(());
    };
    let variants = rows::<tables::VariantDefinitions>(frame)?;
    let by_variant: BTreeMap<_, _> = variants.rows().iter().map(|row| (row.id, row)).collect();
    let countries = rows::<tables::Countries>(frame)?;
    let by_country: BTreeMap<_, _> = countries.rows().iter().map(|row| (row.id, row)).collect();
    let tokens = TokenNames::new(frame)?;
    let database = document.icons.combat.naval_air_groups.equipment();
    let wings = rows::<tables::AirWings>(frame)?;
    let bases = rows::<tables::AirBases>(frame)?;
    let ship_rows = rows::<tables::NavalCombatShips>(frame)?;
    let ship_models = rows::<tables::ShipModels>(frame)?;
    let live_ships = rows::<tables::Ships>(frame)?;
    let breakdowns = rows::<tables::NavalModifierBreakdowns>(frame)?;
    let breakdown_layout =
        replay::fixed_layout::validate_recording(frame, replay::fixed_layout::NAVAL_BREAKDOWNS);
    let modifier_definitions = rows::<tables::ModifierDefinitions>(frame)?;
    let expanded_breakdowns = breakdowns
        .is_complete()
        .then(|| super::naval_summary_tooltips::expanded_trees(breakdowns.rows()));
    let breakdown_index = expanded_breakdowns.as_ref().map(|expanded| {
        let mut by_member = BTreeMap::new();
        for row in expanded.iter().filter(|row| row.use_.combat == id) {
            by_member
                .entry((row.use_.side, row.use_.battle_slot, row.key))
                .or_insert_with(Vec::new)
                .push(row);
        }
        by_member
    });
    let by_modifier = modifier_definitions.is_complete().then(|| {
        modifier_definitions
            .rows()
            .iter()
            .map(|row| (row.index, row))
            .collect::<BTreeMap<_, _>>()
    });
    let members: Vec<_> = ship_rows
        .rows()
        .iter()
        .filter(|row| row.combat == id)
        .collect();
    let models = ship_models
        .is_complete()
        .then(|| super::keyed::Keyed::new(ship_models.rows(), |row| row.ship))
        .flatten();
    let joins = super::naval_air_groups::CarrierJoins::new(
        ship_rows.is_complete().then_some(members.as_slice()),
        bases.is_complete().then_some(bases.rows()),
        live_ships.is_complete().then_some(live_ships.rows()),
        models,
    );
    let wing_index = wings
        .is_complete()
        .then(|| super::keyed::Keyed::new(wings.rows(), |row| row.id))
        .flatten();
    let base_index = bases
        .is_complete()
        .then(|| super::keyed::Keyed::new(bases.rows(), |row| row.id))
        .flatten();
    let presentation_wings: BTreeMap<_, _> = wings.rows().iter().map(|row| (row.id, row)).collect();
    let presentation_bases: BTreeMap<_, _> = bases.rows().iter().map(|row| (row.id, row)).collect();
    let mut presentation_ships = BTreeMap::new();
    for row in ship_rows.rows().iter().filter(|row| row.live_ship != 0) {
        presentation_ships.entry(row.ship).or_insert(row);
    }
    let mut groups = Vec::<AirGroup<'_>>::new();
    for (side, side_detail) in detail.sides.iter().enumerate() {
        for plane in side_detail
            .planes
            .iter()
            .filter(|plane| display_air_plane(plane))
        {
            let model = (|| {
                let variant: &VariantDefinitionRow =
                    by_variant.get(&plane.variant).copied().ok_or_else(|| {
                        format!("variant {} has no VariantDefinitions row", plane.variant)
                    })?;
                let name = tokens
                    .name(variant.equipment_token)
                    .map_err(|error| error.to_string())?;
                let database = database.ok_or("the stacked equipment database is unavailable")?;
                let index = database.get(name.as_bytes()).ok_or_else(|| {
                    format!("equipment type {name} is absent from the stacked content")
                })?;
                let archetype = database.equipment[index].archetype.unwrap_or(index);
                let archetype_name = tokens
                    .name(variant.archetype_token)
                    .map_err(|error| error.to_string())?;
                if !database.equipment[archetype]
                    .name
                    .eq_ignore_ascii_case(archetype_name.as_bytes())
                {
                    return Err(format!(
                        "variant {} archetype {archetype_name} differs from the stacked equipment archetype",
                        variant.id
                    ));
                }
                Ok::<_, String>(index)
            })();
            let model = match model {
                Ok(model) => model,
                Err(reason) => {
                    out.refuse(format!(
                        "Naval combat {id} air plane {} is omitted: {reason}.",
                        plane.index
                    ));
                    continue;
                }
            };
            let Some(&owner) = aliases.get(&plane.country) else {
                out.refuse(format!("Naval combat {id} air plane {} is omitted: country {} has no CountryLinks equivalence row.", plane.index, plane.country));
                continue;
            };
            if let Some(slot) = air_group_slot(&groups, side as u8, model, owner) {
                groups[slot].planes.push(plane);
            } else {
                groups.push(AirGroup {
                    side: side as u8,
                    model,
                    owner,
                    planes: vec![plane],
                });
            }
        }
    }
    let Some(database) = database else {
        return Ok(());
    };
    let stacking_penalty = super::tactics::viewing_side(
        &detail.sides[0].summary.countries,
        "attacker",
        aliases,
        vision.ours(),
    )
    .and_then(|side| {
        out.naval_penalties[side]
            .as_ref()
            .ok_or_else(|| "the perspective carrier penalty has not been evaluated".to_owned())?
            .as_ref()
            .map(|values| values.0)
            .map_err(Clone::clone)
    });
    for group in groups {
        let first = group.planes[0];
        let definition = &database.equipment[group.model];
        let archetype = &database.equipment[definition.archetype.unwrap_or(group.model)];
        let archetype_name = std::str::from_utf8(&archetype.name);
        let sprite_name = std::str::from_utf8(&definition.sprite);
        let tag = match by_country
            .get(&first.country)
            .ok_or_else(|| format!("country {} has no Countries row", first.country))
            .and_then(|row| {
                std::str::from_utf8(&row.tag)
                    .map_err(|_| format!("country {} tag is not UTF-8", first.country))
            }) {
            Ok(tag) if !tag.contains('\0') => tag,
            Ok(_) => {
                out.refuse(format!(
                    "Naval combat {id} air group model {} is omitted: country {} tag contains NUL.",
                    group.model + 1,
                    first.country
                ));
                continue;
            }
            Err(reason) => {
                out.refuse(format!(
                    "Naval combat {id} air group model {} is omitted: {reason}.",
                    group.model + 1
                ));
                continue;
            }
        };
        let mut candidates = Vec::new();
        if let Ok(name) = &archetype_name {
            candidates.push(format!("GFX_navalcombat_air_icon_{tag}_{name}"));
        }
        if let Ok(name) = archetype_name {
            candidates.push(format!("GFX_navalcombat_air_icon_{name}"));
        }
        if let Ok(name) = sprite_name {
            candidates.push(format!("GFX_navalcombat_air_icon_{name}"));
        }
        // 1417DAA40's country, archetype, definition.sprite cascade is tested
        // against the effective .gfx registry, never a vanilla name list.
        let found = candidates.into_iter().find_map(|name| {
            document
                .icons
                .combat
                .sprite_size(&name)
                .map(|size| (name, size))
        });
        let (name, size) = if let Some((name, size)) = found {
            (Some(name), size)
        } else {
            // 1417DAA40 with lStack_350 == 0 hides only CIcon. The row and
            // its counts retain the navalcombat.gui template's own size.
            out.refuse(format!("Naval combat {id} air group model {} icon is hidden: no stacked naval air icon resolves from its country, archetype and definition sprite.", group.model + 1));
            let size = document
                .icons
                .combat
                .native_windows
                .gui_type(b"navalcombat_ship_entry")
                .and_then(|entry| crate::map::native_gui::container_frame(entry, None).ok())
                .map(|frame| frame.size.map(|value| value as f32));
            if size.is_none() {
                out.refuse(format!("Naval combat {id} air group model {} layout is unavailable: navalcombat_ship_entry has no native GUI frame.", group.model + 1));
            }
            let size = size.unwrap_or_default();
            (None, size)
        };
        let hover = stacking_penalty
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|penalty| {
                let prefix = if first.wings[1].is_empty() || first.efficiency.source != 3 {
                    String::new()
                } else {
                    let (sortie_efficiency, _, _) =
                        super::naval_air_groups::display_efficiency_parts(
                            first,
                            &detail.sides[group.side as usize],
                            group.side,
                            detail.duration,
                            &joins,
                            &document.icons.combat.naval_air_groups,
                            document.icons.combat.naval_defines.as_ref(),
                        )
                        .ok_or("141963700 carrier efficiency operands are unavailable")?;
                    let defines = document
                        .icons
                        .combat
                        .naval_defines
                        .as_ref()
                        .ok_or("matching carrier sortie defines are unavailable")?;
                    let mut member_breakdown = String::new();
                    if first.carrier_member_present != 0 && sortie_efficiency != 100_000 {
                        breakdown_layout.as_ref().map_err(Clone::clone)?;
                        let member = joins
                            .carrier_member(group.side, first)
                            .ok_or("141963700 linked carrier member is unavailable")?;
                        let trees = breakdown_index
                            .as_ref()
                            .ok_or("NavalModifierBreakdowns is incomplete")?;
                        let definitions = by_modifier
                            .as_ref()
                            .ok_or("ModifierDefinitions is incomplete")?;
                        for (lane, key) in [(0, 0x24), (1, 0x1b6)] {
                            if lane == 1 && first.efficiency.pool_type_flags & 0x1_0000_0400 == 0 {
                                continue;
                            }
                            member_breakdown.push_str(
                                &super::naval_air_tooltips::carrier_key_breakdown(
                                    member,
                                    trees
                                        .get(&(group.side, member.battle_slot, key))
                                        .map(Vec::as_slice)
                                        .unwrap_or(&[]),
                                    definitions.get(&key).copied(),
                                    key,
                                    &document.recorded_language,
                                    &document.names.localisation,
                                    &document.icons.combat,
                                )?,
                            );
                            let screening = super::naval_air_groups::display_screening_extra(
                                first,
                                &detail.sides[group.side as usize],
                                group.side,
                                &joins,
                                &document.icons.combat.naval_air_groups,
                                lane,
                            )
                            .ok_or("carrier sortie screening breakdown is unavailable")?;
                            member_breakdown.push_str(&super::naval_air_tooltips::screening_line(
                                screening,
                                &document.names.localisation,
                                &document.icons.combat,
                            )?);
                        }
                    }
                    super::naval_air_tooltips::efficiency_prefix(
                        first,
                        sortie_efficiency,
                        defines.base_carrier_sortie_efficiency,
                        &member_breakdown,
                        &document.names.localisation,
                        &document.icons.combat,
                    )?
                };
                let mut efficiency_suffixes = Vec::with_capacity(group.planes.len());
                for plane in &group.planes {
                    if plane.efficiency.source != 3 || plane.wings[1].is_empty() {
                        efficiency_suffixes.push(String::new());
                        continue;
                    }
                    // 1417D2430 chooses the first +0x40 wing whose resolved air
                    // base has no +0x68 state before asking 141965010 for text.
                    let wings = wing_index.as_ref().ok_or("AirWings are incomplete")?;
                    let bases = base_index.as_ref().ok_or("AirBases are incomplete")?;
                    let eligible = plane.wings[1].iter().any(|handle| {
                        wings
                            .get(*handle)
                            .and_then(|wing| bases.get(wing.air_base))
                            .is_some_and(|base| base.state.is_none())
                    });
                    if !eligible {
                        efficiency_suffixes.push(String::new());
                        continue;
                    }
                    let (_, final_efficiency, organisation_ratio) =
                        super::naval_air_groups::display_efficiency_parts(
                            plane,
                            &detail.sides[group.side as usize],
                            group.side,
                            detail.duration,
                            &joins,
                            &document.icons.combat.naval_air_groups,
                            document.icons.combat.naval_defines.as_ref(),
                        )
                        .ok_or("141965010 carrier efficiency operands are unavailable")?;
                    efficiency_suffixes.push(super::naval_air_tooltips::carrier_org_suffix(
                        final_efficiency,
                        organisation_ratio,
                        &document.names.localisation,
                        &document.icons.combat,
                    )?);
                }
                if !wings.is_complete()
                    || !bases.is_complete()
                    || !countries.is_complete()
                    || !ship_rows.is_complete()
                {
                    return Err("naval air presentation input tables are incomplete".into());
                }
                let presentations = group
                    .planes
                    .iter()
                    .map(|plane| {
                        air_presentation(
                            document,
                            frame,
                            plane,
                            &presentation_wings,
                            &presentation_bases,
                            &by_country,
                            &presentation_ships,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                super::naval_air_tooltips::group_hover(
                    &group.planes,
                    &presentations,
                    &efficiency_suffixes,
                    &prefix,
                    frame.hour,
                    *penalty,
                    &document.recorded_language,
                    &document.names.localisation,
                    &document.icons.combat,
                )
            });
        let hover = match hover {
            Ok(hover) => hover_text(&hover),
            Err(reason) => {
                out.refuse(format!(
                    "Naval combat {id} air group model {} hover is withheld: {reason}.",
                    group.model + 1
                ));
                None
            }
        };
        // 1417DAA40 sums current and max(initial,current) in signed i32.
        let current = group
            .planes
            .iter()
            .fold(0_i32, |sum, plane| sum.wrapping_add(plane.current_planes));
        let maximum = group.planes.iter().fold(0_i32, |sum, plane| {
            sum.wrapping_add(plane.initial_planes.max(plane.current_planes))
        });
        let low_efficiency = (|| {
            for plane in &group.planes {
                // 1417DAA40 reads CFEXAir+0x40, the active second wing
                // vector. A missing resolved wing or base is skipped by the
                // native reference walk; incomplete tables make it unknown.
                for handle in &plane.wings[1] {
                    let wings = wing_index
                        .as_ref()
                        .ok_or("AirWings is incomplete or has duplicate handles")?;
                    let bases = base_index
                        .as_ref()
                        .ok_or("AirBases is incomplete or has duplicate handles")?;
                    let Some(wing) = wings.get(*handle) else {
                        continue;
                    };
                    let Some(base) = bases.get(wing.air_base) else {
                        continue;
                    };
                    if base.state.is_some() {
                        continue;
                    }
                    let efficiency = super::naval_air_groups::display_efficiency(
                        plane,
                        &detail.sides[group.side as usize],
                        group.side,
                        detail.duration,
                        &joins,
                        &document.icons.combat.naval_air_groups,
                        document.icons.combat.naval_defines.as_ref(),
                    )
                    .ok_or("141965010 efficiency operands are unavailable")?;
                    if efficiency < 50_000 {
                        return Ok::<_, &'static str>(true);
                    }
                }
            }
            Ok(false)
        })();
        let label = low_efficiency.map_err(str::to_owned).and_then(|low| {
            country_flag(flags, first.country)
                .map(|tag| air_count_label(group.side, &tag, current, maximum, low))
        });
        let label = match label {
            Ok(label) => Some(label),
            Err(reason) => {
                out.refuse(format!(
                    "Naval combat {id} air group model {} count/flag is withheld: {reason}.",
                    group.model + 1,
                ));
                None
            }
        };
        push_air_entry(out, heights, group.side, name, size, label, hover);
    }
    Ok(())
}

pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    selection: Selection,
    flags: &Flags,
    vision: &Vision,
) -> io::Result<()> {
    match selection {
        Selection::Naval(id) => active(out, document, frame, id, flags, vision),
        Selection::NavalResult(id) => result(out, document, frame, id, flags),
        Selection::Land(_) => Ok(()),
    }
}

// 14195ED00 uses the same display-box predicate as carrier sortie modifiers:
// 140C2B7E0 tests both the ship and its positive equipment pool's bits 22/37.
pub(super) fn column(row: &NavalCombatShipRow, pool_flags: u64) -> Option<&'static str> {
    match super::naval_air_groups::display_box(row, pool_flags) {
        1 => Some("screen"),
        2 => Some("battle"),
        3 => Some("carrier"),
        4 => Some("hidden"),
        5 => Some("visible"),
        6 => Some("sunk"),
        7 => Some("reserve"),
        _ => None,
    }
}

fn pool_flags(
    row: &NavalCombatShipRow,
    models: &BTreeMap<u64, &replay::rows::ShipModelRow>,
    incomplete: Option<&str>,
) -> Result<u64, String> {
    // These branches of 14195ED00 do not depend on the equipment predicate.
    if !matches!(row.state, 1 | 2 | 8)
        || row.live_convoy != 0
        || row.live_ship == 0
        || row.formation_flags & ((1 << 7) | (1 << 22) | (1 << 37)) != 0
    {
        return Ok(0);
    }
    models
        .get(&row.ship)
        .and_then(|model| model.counter.as_ref())
        .map(|counter| counter.positive_equipment_flags)
        .ok_or_else(|| {
            incomplete.map_or_else(
                || format!("ship {} has no ShipModels counter operands", row.ship),
                |why| format!("ShipModels is incomplete: {why}"),
            )
        })
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    side: u8,
    column: &'static str,
    state: u32,
    formation: i32,
    subunit: i32,
    convoy: u8,
    owner: u32,
    damaged: bool,
    unknown: bool,
}
fn key(row: &NavalCombatShipRow, owner: u32, unknown: bool, pool_flags: u64) -> Option<GroupKey> {
    let column = column(row, pool_flags)?;
    // 1417D9730 collapses all unseen incoming entries in their box. Otherwise
    // it compares cached formation/subunit/convoy, country aliases and damage.
    Some(GroupKey {
        side: row.side,
        column,
        state: if unknown {
            0
        } else if row.state == 1 {
            2
        } else {
            row.state
        },
        formation: if unknown { 0 } else { row.formation_type },
        subunit: if unknown { 0 } else { row.subunit_index },
        convoy: if unknown { 0 } else { row.is_convoy },
        owner: if unknown { 0 } else { owner },
        damaged: !unknown
            && (row.live_ship != 0 || row.live_convoy != 0)
            && row.strength < row.maximum_hit_points,
        unknown,
    })
}

fn active(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    flags: &Flags,
    vision: &Vision,
) -> io::Result<()> {
    let table = rows::<tables::NavalCombatShips>(frame)?;
    if table.rows().is_empty() && !table.is_complete() {
        return Ok(());
    }
    let organisation_admitted = match replay::fixed_layout::validate_recording(
        frame,
        replay::fixed_layout::SHIP_ORGANISATION,
    ) {
        Ok(()) => true,
        Err(reason) => {
            out.warnings.push(reason);
            false
        }
    };
    let mut members: Vec<_> = table.rows().iter().filter(|r| r.combat == id).collect();
    // The native forward traversal and retained-member list are separate. Do
    // not drop source1 when its ship has already left NavalCombatParticipants.
    members.sort_by_key(|r| {
        (
            r.member_source,
            r.group_index,
            r.member_index,
            r.battle_slot,
        )
    });
    let aliases: BTreeMap<_, _> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    let mut lookup = BTreeMap::new();
    let mut groups: Vec<(GroupKey, Vec<&NavalCombatShipRow>)> = Vec::new();
    let models = rows::<tables::ShipModels>(frame)?;
    let by_ship = models.rows().iter().map(|row| (row.ship, row)).collect();
    let model_gap = match &models {
        crate::table_access::Available::Complete(_) => None,
        crate::table_access::Available::Truncated { missing, .. } => {
            Some(format!("{missing} rows not reached"))
        }
        crate::table_access::Available::Unavailable { reason } => Some((*reason).to_owned()),
    };
    // Incoming live ships whose owner's sharing (1406F3A90) the hour cannot
    // decide, by missing input: their box is unknown, so they are omitted.
    let mut undecided: BTreeMap<SharingUnknown, usize> = BTreeMap::new();
    for row in members {
        let unknown = if row.state == 4 && row.live_ship != 0 {
            match vision.sees_combat(std::iter::once(aliases.get(&row.owner).copied()), None) {
                Ok(seen) => !seen,
                Err(cause) => {
                    *undecided.entry(cause).or_default() += 1;
                    continue;
                }
            }
        } else {
            false
        };
        let pool = match pool_flags(row, &by_ship, model_gap.as_deref()) {
            Ok(pool) => pool,
            Err(reason) => {
                out.refuse(format!(
                    "NavalCombatShips slot {} combat box is withheld: {reason} (140C2B7E0).",
                    row.battle_slot
                ));
                continue;
            }
        };
        let Some(key) = key(
            row,
            aliases.get(&row.owner).copied().unwrap_or(row.owner),
            unknown,
            pool,
        ) else {
            out.warnings.push(format!(
                "NavalCombatShips slot {}: state {} has no drawable live combat box; omitted.",
                row.battle_slot, row.state
            ));
            continue;
        };
        let next = groups.len();
        let index = *lookup.entry(key.clone()).or_insert(next);
        if index == next {
            groups.push((key, Vec::new()));
        }
        groups[index].1.push(row);
    }
    for (cause, ships) in &undecided {
        out.warnings.push(format!(
            "NavalCombatShips: {ships} incoming ships of combat {id} omitted: whether the viewer sees them (1406F3A90 for their owner) is unknown, {}.",
            cause.describe()
        ));
    }
    // 1417D9C60 -> 1417CC070 sorts each combat box after grouping. More
    // numerous groups precede smaller ones; the final native pointer tie is
    // transient, so retain recorded insertion order for otherwise equal keys.
    groups.sort_by_key(|(key, members)| {
        (
            key.side,
            key.column,
            Reverse(key.convoy),
            Reverse(key.formation),
            Reverse(key.subunit),
            Reverse(members.len()),
            key.damaged,
            Reverse(members.iter().map(|r| r.battle_slot).max().unwrap_or(0)),
        )
    });
    out.naval_entries.clear();
    let mut heights = BTreeMap::<(u8, &str), f32>::new();
    air_entries(
        out,
        document,
        frame,
        id,
        flags,
        &aliases,
        vision,
        &mut heights,
    )?;
    for (key, members) in groups {
        let first = members[0];
        let name = if key.unknown {
            "GFX_navalcombat_ship_icon_unknown".into()
        } else {
            format!(
                "GFX_navalcombat_ship_icon_{}",
                String::from_utf8_lossy(&first.sprite)
            )
        };
        let Some(asset) = document.icons.combat.sprite_size(&name) else {
            out.warnings.push(format!(
                "NavalCombatShips slot {}: declared sprite {name} is unavailable; row omitted.",
                first.battle_slot
            ));
            continue;
        };
        let mut values = Values::new();
        for widget in [
            "icon_hidden",
            "escape_progress",
            "escape_icon_left",
            "escape_icon_right",
            "pride",
            "bar_str",
            "bar_org",
            "desc_left",
            "desc_right",
        ] {
            hide(&mut values, widget);
        }
        let escaping: Vec<_> = members.iter().filter(|r| r.state == 8).collect();
        let reversed = !escaping.is_empty() || members.iter().any(|r| r.state == 16);
        values.insert(
            "icon".into(),
            Widget {
                sprite: Some(name),
                frame: Some(icon_frame(key.side, reversed)),
                ..Widget::default()
            },
        );
        if !key.unknown {
            descriptions(
                out,
                &mut values,
                first.owner,
                members.len(),
                key.side,
                asset[0],
                flags,
            );
            let live: Vec<_> = members
                .iter()
                .filter(|r| r.live_ship != 0 || r.live_convoy != 0)
                .collect();
            if !live.is_empty() && key.column != "sunk" {
                let ratio = live
                    .iter()
                    .map(|r| fixed_ratio(r.strength, r.maximum_hit_points.max(100)))
                    .sum::<i128>()
                    / live.len() as i128;
                bar(
                    &mut values,
                    "bar_str",
                    ratio as f32 / 100_000.0,
                    asset[0],
                    document,
                );
                // A refused ship organisation family was reported once above.
                if organisation_admitted {
                    match live.iter().try_fold(0_i128, |sum, row| {
                        let maximum = organisation_maximum(row)
                            .map_err(|reason| (row.battle_slot, reason))?;
                        Ok::<_, (u32, String)>(sum + fixed_ratio(row.organisation, maximum))
                    }) {
                        Ok(total) => {
                            let ratio = total / live.len() as i128;
                            bar(
                                &mut values,
                                "bar_org",
                                ratio as f32 / 100_000.0,
                                asset[0],
                                document,
                            );
                        }
                        Err((slot, reason)) => out.warnings.push(format!(
                            "NavalCombatShips slot {slot}: its maximum organisation (140C29DF0) is unknown: {reason}; its group's organisation bar is omitted."
                        )),
                    }
                }
            }
            if key.column == "hidden" {
                show(&mut values, "icon_hidden");
                center_icon(
                    &mut values,
                    "icon_hidden",
                    "GFX_navalcombat_ship_hidden",
                    asset[0],
                    document,
                );
            }
            if members.iter().any(|r| r.pride != 0) {
                show(&mut values, "pride");
                center_icon(
                    &mut values,
                    "pride",
                    "GFX_NV_icon_pride_small",
                    asset[0],
                    document,
                );
            }
        }
        if !escaping.is_empty() {
            let ratio = escaping
                .iter()
                .map(|r| i128::from(r.escape_progress))
                .sum::<i128>()
                / escaping.len() as i128;
            bar(
                &mut values,
                "escape_progress",
                ratio as f32 / 100_000.0,
                asset[0],
                document,
            );
            let icon = if key.side == 0 {
                "escape_icon_left"
            } else {
                "escape_icon_right"
            };
            show(&mut values, icon);
            // 1417DBBE0 places the escape icon at the bar's final x plus the
            // icon's authored x; CENTER_UP alone would add half the ship width.
            values.entry(icon.into()).or_default().offset[0] =
                escape_bar_x(asset[0], document) - asset[0] / 2.0;
        }
        let wnd = if key.side == 0 { "left" } else { "right" };
        let grid = match key.column {
            "hidden" | "visible" => {
                format!("battle_window/middle_wnd/{wnd}_wnd/grid_{}", key.column)
            }
            "sunk" => format!("bottom_wnd/{wnd}_wnd/grid"),
            "reserve" => format!("battle_window/outside_wnd/{wnd}_wnd/grid"),
            column => format!("battle_window/top_wnd/{wnd}_wnd/grid_{column}"),
        };
        let height = heights.entry((key.side, key.column)).or_default();
        let offset_x = if key.column == "sunk" {
            0.0
        } else {
            combat_box_x(asset[0])
        };
        out.naval_entries.push(NavalEntry {
            side: key.side,
            unknown: key.unknown,
            grid,
            offset: entry_offset(key.column, offset_x, *height),
            size: asset,
            values,
            battle_slots: members.iter().map(|row| row.battle_slot).collect(),
        });
        *height += asset[1] + 3.0 + if escaping.is_empty() { 0.0 } else { 8.0 };
    }
    out.warnings
        .retain(|w| !w.starts_with("NavalCombatShips: live HP/org bars"));
    out.warnings.push("Format-22 naval rows: native state/formation/equipment/reveal boxes, source0 and retained source1 slots, normalized-state/subunit/formation/country/damaged grouping, copied sprites, native sorted group counts, mean strength/organisation and escape progress are bound. Organisation maxima apply recorded modifiers46/47/27e with default-leader and HQ sources; missing terms in complete captured sources mean zero. Sunk entries follow the content grid's slots (1417D05D0). UNVERIFIED: pride tint, incoming-entry relation edge cases and remaining flag/text geometry require live comparison.".into());
    Ok(())
}

/// 1417D05D0 advances sunk entries by the grid's authored slot direction and
/// limits. Fighting boxes instead supply explicit pixel offsets (1417D0380).
pub(super) fn entry_offset(column: &str, x: f32, y: f32) -> Option<[f32; 2]> {
    (column != "sunk").then_some([x, y])
}
/// 1417DBBE0 sets the one-based icon frame to `(left ^ reversed) + 1`, where
/// `left` is the column's side byte (1 for the attacker's left column) and a
/// group is reversed when any member is escaping (8) or state 16. Fitting both
/// frames of the shipped strips to pairJ_naval_5.png agrees: ENG's fighting
/// submarines use frame 1 (MAE 34.5 against 43.0), the escaping one frame 0,
/// ROM's escaping convoy frame 1 (13.6 against 49.6) and its sunk convoy 0.
pub(super) fn icon_frame(side: u8, reversed: bool) -> u32 {
    u32::from(side == 0) ^ u32::from(reversed)
}
/// A combat-box entry's x inside its 100-wide UPPER_RIGHT grid. 1417D0380
/// passes `-(1 - width) / 2` (C division) as the item position. pairJ_naval_5
/// puts the 48-wide submarines at grid + 77 and the 52-wide convoy at + 75,
/// which is `100 - trunc((width - 1) / 2)`; the grid's own right-format
/// arithmetic is UNVERIFIED beyond these two widths.
pub(super) fn combat_box_x(width: f32) -> f32 {
    100.0 - ((width - 1.0) * 0.5).trunc()
}
/// The escape bar's final x in the entry: `width / 2 - bar / 2 + authored x`.
pub(super) fn escape_bar_x(width: f32, document: &Document) -> f32 {
    let bar = document
        .icons
        .combat
        .sprite_size("GFX_naval_combat_escape_bar")
        .map_or(54.0, |s| s[0]);
    (width / 2.0).trunc() - (bar / 2.0).trunc() - 1.0
}
fn fixed_ratio(value: i64, maximum: i64) -> i128 {
    i128::from(value) * 100_000 / i128::from(maximum)
}
/// 140C29DF0's maximum organisation, or which operand refuses it.
pub(super) fn organisation_maximum(row: &NavalCombatShipRow) -> Result<i64, String> {
    if row.live_ship == 0 {
        if row.live_convoy == 0 {
            return Err("it is neither a live ship nor a live convoy".into());
        }
        return if row.base_organisation > 0 {
            Ok(row.base_organisation)
        } else {
            Err(format!(
                "the convoy records base organisation {}, not a positive value",
                row.base_organisation
            ))
        };
    }
    // 140C29DF0 -> 140C28730 calls 140C29020 with commander=0, which
    // selects the task force's default commander rather than combat leader.
    // The rest of the captured sources, including HQ12..15, still contribute.
    let mut context = row.weapon_context.clone();
    context.sources.retain(|s| !matches!(s.scope, 9 | 10));
    for source in &mut context.sources {
        source.scope = match source.scope {
            31 => 9,
            32 => 10,
            other => other,
        };
    }
    let modifier = |key: u32| {
        super::naval_environment::modifier(&context, key).ok_or_else(|| {
            let name = engine::modifier_keys::ALL
                .iter()
                .find(|known| known.index == key)
                .map_or("an unnamed key", |known| known.name);
            format!(
                "naval_environment::modifier gives no {name} ({key:#x}) from its recorded weapon-context sources"
            )
        })
    };
    let base = row
        .base_organisation
        .wrapping_add(modifier(engine::modifier_keys::NAVY_ORG)?);
    let factor = 100_000_i64
        .wrapping_add(modifier(engine::modifier_keys::NAVY_ORG_FACTOR)?)
        .wrapping_add(modifier(engine::modifier_keys::MAX_ORGANISATION)?);
    Ok((base.wrapping_mul(factor) / 100_000).max(100))
}
fn show(values: &mut Values, key: &str) {
    values.entry(key.into()).or_default().hidden = false;
}
fn bar(values: &mut Values, key: &str, value: f32, width: f32, document: &Document) {
    show(values, key);
    fill(values, key, value.clamp(0.0, 1.0));
    let name = match key {
        "bar_str" => "GFX_navalcombat_bar_str",
        "bar_org" => "GFX_navalcombat_bar_org",
        _ => "GFX_naval_combat_escape_bar",
    };
    center_icon(values, key, name, width, document);
}
fn center_icon(values: &mut Values, key: &str, name: &str, width: f32, document: &Document) {
    if let Some(asset) = document.icons.combat.sprite_size(name) {
        // CENTER_UP already contributes width/2. Native retains authored x.
        values.entry(key.into()).or_default().offset[0] =
            (width / 2.0).trunc() - width / 2.0 - (asset[0] / 2.0).trunc();
    }
}
/// 1417DBBE0: the attacker's left column writes the country flag into
/// `desc_left` and the count into `desc_right`; the right column swaps them.
/// `desc_left` keeps its authored x (-8, left format). `desc_right` moves to
/// `authored x + icon width - maxWidth` so its right-aligned text ends 8 px
/// past the icon. The flag text is the inline 0x12 country marker.
/// pairJ_naval_5.png: ENG's flag starts at icon - 8 and its count ends at
/// icon + 56; ROM's count "1" starts at icon - 8 and its flag ends 8 px past
/// the icon.
pub(super) fn descriptions(
    out: &mut Bindings,
    values: &mut Values,
    owner: u32,
    count: usize,
    side: u8,
    width: f32,
    flags: &Flags,
) {
    let (count_key, flag_key) = if side == 0 {
        ("desc_right", "desc_left")
    } else {
        ("desc_left", "desc_right")
    };
    let offset = |key: &str| {
        if key == "desc_right" {
            [width - 105.0, 0.0]
        } else {
            [0.0; 2]
        }
    };
    values.insert(
        count_key.into(),
        Widget {
            text: Some(count.to_string()),
            offset: offset(count_key),
            ..Widget::default()
        },
    );
    out.bind_in(
        values,
        flag_key,
        TEXT,
        country_flag(flags, owner).map(|flag| format!("{}{flag}", super::INLINE_FLAG)),
    );
    values.entry(flag_key.into()).or_default().offset = offset(flag_key);
}

fn repeated(values: &mut Values, key: &str, template: String, entries: Vec<Values>) {
    values.insert(
        key.into(),
        Widget {
            entry_template: Some(template),
            entries,
            ..Widget::default()
        },
    );
}
fn result(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    flags: &Flags,
) -> io::Result<()> {
    let table = rows::<tables::NavalCombatResultDetails>(frame)?;
    let Some(detail) = table.rows().iter().find(|r| r.result == id) else {
        out.warnings.push(format!(
            "Naval result {id}: no NavalCombatResultDetails row, so the survivor and loss lists are omitted."
        ));
        return Ok(());
    };
    for (side, row) in detail.sides.iter().enumerate() {
        let label = if side == 0 { "attacker" } else { "defender" };
        let root = if side == 0 {
            "attacking_side"
        } else {
            "defending_side"
        };
        let mut survivor_keys = BTreeMap::new();
        let mut survivors: Vec<Vec<&NavalResultShip>> = Vec::new();
        let mut loss_keys = BTreeMap::new();
        let mut losses: Vec<Vec<&NavalResultShip>> = Vec::new();
        for ship in &row.ships {
            // 1417C4C30/141DF27E0/141DEBD30: survivor subunit+formation;
            // losses use copied sprite, with convoys in a separate list.
            if ship.sunk_by.is_empty() {
                let next = survivors.len();
                let index = *survivor_keys
                    .entry((ship.subunit_index, ship.formation_type))
                    .or_insert(next);
                if index == next {
                    survivors.push(Vec::new());
                }
                survivors[index].push(ship);
            } else {
                let next = losses.len();
                let index = *loss_keys
                    .entry((ship.is_convoy, ship.sprite.as_slice()))
                    .or_insert(next);
                if index == next {
                    losses.push(Vec::new());
                }
                losses[index].push(ship);
            }
        }
        let mut entries = Vec::new();
        for group in survivors {
            let mut values = Values::new();
            let name = format!(
                "GFX_unit_{}_icon_medium_black",
                String::from_utf8_lossy(&group[0].sprite)
            );
            if document.icons.combat.has_sprite(&name) {
                sprite(&mut values, "fleet", name);
            } else {
                hide(&mut values, "fleet");
            }
            text(&mut values, "number", group.len().to_string());
            result_flags(out, &mut values, &group, flags);
            entries.push(values);
        }
        repeated(
            &mut out.values,
            &format!("{root}/survivors/survivors_grid"),
            format!("naval_combat_results_{label}_survivor_entry"),
            entries,
        );
        let mut entries = Vec::new();
        for group in losses {
            let mut values = Values::new();
            let name = format!(
                "GFX_navalcombat_ship_icon_{}",
                String::from_utf8_lossy(&group[0].sprite)
            );
            if document.icons.combat.has_sprite(&name) {
                values.insert(
                    "equipment".into(),
                    Widget {
                        sprite: Some(name),
                        frame: Some(side as u32),
                        ..Widget::default()
                    },
                );
            } else {
                hide(&mut values, "equipment");
            }
            if !group.iter().any(|s| s.pride != 0) {
                hide(&mut values, "pride");
            }
            let mut countries = BTreeMap::new();
            for ship in group {
                *countries.entry(ship.owner).or_insert(0usize) += 1;
            }
            let counts = countries
                .into_iter()
                .map(|(owner, count)| {
                    let mut values = Values::new();
                    text(&mut values, "number", count.to_string());
                    out.bind_in(
                        &mut values,
                        "country_flag",
                        FLAG,
                        country_flag(flags, owner),
                    );
                    values
                })
                .collect();
            repeated(
                &mut values,
                "lost_ships_grid",
                format!("naval_combat_results_{label}_lost_ship_entry"),
                counts,
            );
            entries.push(values);
        }
        repeated(
            &mut out.values,
            &format!("{root}/lost/lost_line_grid"),
            format!("naval_combat_results_{label}_lost_line_entry"),
            entries,
        );
    }
    out.warnings.retain(|w| {
        !w.starts_with("Combat results:") && !w.starts_with("NavalCombatResultDetails side")
    });
    out.warnings.push("Format-22 retained results: copied sunk_by classifies survivors/losses; native subunit/formation survivor and sprite/convoy loss groups, recorded counts, copied sprites and country flags are bound. UNVERIFIED: country ordering, representative survivor sprite choice, plane equipment-icon joins, outcome perspective, detail hit text and result scrolling require native binder completion and a retained-result capture. No active-combat disappearance is treated as a result.".into());
    Ok(())
}
fn result_flags(
    out: &mut Bindings,
    values: &mut Values,
    ships: &[&NavalResultShip],
    flags: &Flags,
) {
    for key in [
        "country_flag_left",
        "country_flag_right",
        "country_flag_center",
    ] {
        hide(values, key);
    }
    let mut owners = Vec::new();
    let mut seen = BTreeSet::new();
    for ship in ships {
        if seen.insert(ship.owner) {
            owners.push(ship.owner);
        }
    }
    let slots: &[&str] = if owners.len() == 1 {
        &["country_flag_center"]
    } else {
        &["country_flag_left", "country_flag_right"]
    };
    for (owner, key) in owners.into_iter().zip(slots) {
        out.bind_in(values, key, FLAG, country_flag(flags, owner));
        values.entry((*key).into()).or_default().size = Some([12.0, 8.0]);
    }
}

#[cfg(test)]
mod tests;
