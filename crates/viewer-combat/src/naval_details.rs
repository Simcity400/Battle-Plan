//! Native naval member grouping and retained result dispatch.
use super::{
    HoverText, Selection, Values, Widget,
    binding::{Bindings, Flags, NavalEntry, TEXT, country_flag, fill, hide, text},
    tooltips::{hover_text, set_hover},
};
use replay::{
    frame::DecodedFrame,
    rows::{
        AirBaseRow, AirWingRow, CountryRow, NavalCombatPlane, NavalCombatShipRow,
        VariantDefinitionRow, tables,
    },
};
use std::{cmp::Reverse, collections::BTreeMap, io};
use viewer_foundation::map::native_gui::container_frame;
use viewer_foundation::table_access::Available;
use viewer_foundation::tooltip_text::TextIcons;
use viewer_foundation::tooltip_text::localise;
use viewer_foundation::tooltip_text::state_name;
use viewer_foundation::tooltip_text::wing_short_name;
use {
    super::content::Content as StaticContent,
    viewer_foundation::{
        map::{
            perspective::{SharingUnknown, Vision},
            tables::rows,
        },
        table_access::TokenNames,
    },
};

struct AirGroup<'a> {
    side: u8,
    model: usize,
    owner: u32,
    planes: Vec<&'a NavalCombatPlane>,
}

/// Visit the forward groups' active-air arrays before the ship arrays. Join
/// a plane with the first entry having the same equipment registration ID
/// and equivalent country.
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

/// Air-base display names use the state name for a state base, otherwise the
/// host ship name. Wing short names are distinct and retain wing order.
/// Wing additions and list rebuilds keep this name list current.
fn air_presentation(
    static_content: &StaticContent,
    frame: &DecodedFrame,
    plane: &NavalCombatPlane,
    wings: &BTreeMap<u64, &AirWingRow>,
    bases: &BTreeMap<u64, &AirBaseRow>,
    countries: &BTreeMap<u32, &CountryRow>,
    ships: &BTreeMap<u64, &NavalCombatShipRow>,
) -> Result<(String, String), String> {
    let loc = &static_content.localisation;
    let base = bases
        .get(&plane.air_base)
        .copied()
        .ok_or_else(|| format!("naval air base {} has no AirBases row", plane.air_base))?;
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
        state_name(
            loc,
            &static_content.recorded_language,
            id,
            &state.custom_name,
            &state.owner_name,
            || {
                let key = static_content
                    .state_name_keys
                    .get(&id)
                    .ok_or_else(|| format!("naval air base state {id} has no content definition"))?
                    .as_ref()
                    .map_err(Clone::clone)?;
                localise(loc, key, &[])
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
        dashboard_text::scripted_text::stored_text(
            loc,
            &static_content.recorded_language,
            &ship.name,
            "naval air-base host ship name",
        )?
    };
    if plane.wings[1].len() != plane.wing_types.len() {
        return Err("plane wing handles and type indices differ in length".into());
    }
    let sub_units = static_content
        .assets
        .commander_tooltips
        .sub_units()
        .map_err(str::to_owned)?;
    let mut names = Vec::<String>::new();
    for (&handle, &index) in plane.wings[1].iter().zip(&plane.wing_types) {
        if index == u32::MAX {
            return Err(format!(
                "historical plane wing {handle} no longer resolves to its type"
            ));
        }
        let wing = wings
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("plane wing {handle} has no AirWings row"))?;
        let selector = if wing.country_override > 0 {
            wing.country_override as u32
        } else {
            wing.owner
        };
        let country = countries.get(&selector).copied().ok_or_else(|| {
            format!("plane wing {handle} country {selector} has no Countries row")
        })?;
        let tag = std::str::from_utf8(&country.tag)
            .map_err(|_| format!("plane wing {handle} country tag is not UTF-8"))?;
        let key = sub_units
            .name(index)
            .ok_or_else(|| format!("plane wing {handle} subunit index {index} is absent"))??;
        let short = wing_short_name(loc, tag, key)?;
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
        // The left side uses frame 2 and the right uses frame 1.
        // Bound GUI frames are one based.
        frame: Some(if side == 0 { 2 } else { 1 }),
        // The game sets this dynamic hover after constructing the row.
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

/// The carrier section appends the linked member's 0x24 (and, for a fighter
/// pool, 0x1B6) source trees, each followed by its box-2/3
/// screening line, only when that member is present and the sortie
/// efficiency differs from ONE. Source formatting takes the ship's sources
/// when the member's ship is live and its side's sources otherwise
/// ([`super::naval_trees::Sources`]).
#[allow(
    clippy::too_many_arguments,
    reason = "the plane, its side, the joins, the trees and the text resources are separate operands"
)]
fn sortie_member_breakdown(
    plane: &NavalCombatPlane,
    sortie_efficiency: i64,
    detail: &replay::rows::NavalCombatDetailsRow,
    side_index: u8,
    joins: &super::naval_air_groups::CarrierJoins<'_>,
    trees: &super::naval_trees::Sources<'_>,
    content: &super::naval_air_groups::Content,
    icons: &dyn TextIcons,
    loc: &installed_content::localisation::Localisation,
) -> Result<String, String> {
    let mut text = String::new();
    if plane.carrier_member_present == 0 || sortie_efficiency == 100_000 {
        return Ok(text);
    }
    let member = joins
        .carrier_member(side_index, plane)
        .ok_or("linked carrier member is unavailable")?;
    let side = detail
        .sides
        .get(usize::from(side_index))
        .ok_or("carrier side is unavailable")?;
    let opposing = &detail.sides[1 - usize::from(side_index)];
    for (lane, key) in [(0, 0x24), (1, 0x1b6)] {
        if lane == 1 && plane.efficiency.pool_type_flags & 0x1_0000_0400 == 0 {
            continue;
        }
        if member.live_ship != 0 {
            trees.ship(member, key, "  ", &mut text, icons, loc)?;
        } else {
            trees.side(
                detail.combat,
                side,
                opposing,
                key,
                "  ",
                &mut text,
                icons,
                loc,
            )?;
        }
        let screening = super::naval_air_groups::display_screening_extra(
            plane, side, side_index, joins, content, lane,
        )
        .ok_or("carrier sortie screening breakdown is unavailable")?;
        text.push_str(&super::naval_air_tooltips::screening_line(
            screening, loc, icons,
        )?);
    }
    Ok(text)
}

#[allow(clippy::too_many_arguments)] // The hour's frame, the battle's identity and its draw state.
fn air_entries(
    out: &mut Bindings,
    static_content: &StaticContent,
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
    let database = static_content.assets.naval_air_groups.equipment();
    let wings = rows::<tables::AirWings>(frame)?;
    let bases = rows::<tables::AirBases>(frame)?;
    let ship_rows = rows::<tables::NavalCombatShips>(frame)?;
    let ship_models = rows::<tables::ShipModels>(frame)?;
    let live_ships = rows::<tables::Ships>(frame)?;
    let trees = super::naval_trees::Sources::read(frame, &static_content.recorded_labels)?;
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
        // country, archetype, definition.sprite cascade is tested
        // against the effective `.gfx` registry, never a vanilla name list.
        let found = candidates.into_iter().find_map(|name| {
            static_content
                .assets
                .sprite_size(&name)
                .map(|size| (name, size))
        });
        let (name, size) = if let Some((name, size)) = found {
            (Some(name), size)
        } else {
            // An absent drawable hides only the icon. The row and its counts retain
            // their authored template size.
            out.refuse(format!("Naval combat {id} air group model {} icon is hidden: no stacked naval air icon resolves from its country, archetype and definition sprite.", group.model + 1));
            let size = static_content
                .assets
                .native_windows
                .gui_type(b"navalcombat_ship_entry")
                .and_then(|entry| container_frame(entry, None).ok())
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
                            &static_content.assets.naval_air_groups,
                            static_content.assets.naval_defines.as_ref(),
                        )
                        .ok_or("carrier efficiency operands are unavailable")?;
                    let defines = static_content
                        .assets
                        .naval_defines
                        .as_ref()
                        .ok_or("matching carrier sortie defines are unavailable")?;
                    let member_breakdown = sortie_member_breakdown(
                        first,
                        sortie_efficiency,
                        detail,
                        group.side,
                        &joins,
                        &trees,
                        &static_content.assets.naval_air_groups,
                        static_content.assets.as_ref(),
                        &static_content.localisation,
                    )?;
                    super::naval_air_tooltips::efficiency_prefix(
                        first,
                        sortie_efficiency,
                        defines.base_carrier_sortie_efficiency,
                        &member_breakdown,
                        &static_content.localisation,
                        static_content.assets.as_ref(),
                    )?
                };
                let mut efficiency_suffixes = Vec::with_capacity(group.planes.len());
                for plane in &group.planes {
                    if plane.efficiency.source != 3 || plane.wings[1].is_empty() {
                        efficiency_suffixes.push(String::new());
                        continue;
                    }
                    // Select the first wing in the active second list whose resolved
                    // air base has no state before constructing its text.
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
                            &static_content.assets.naval_air_groups,
                            static_content.assets.naval_defines.as_ref(),
                        )
                        .ok_or("carrier efficiency operands are unavailable")?;
                    efficiency_suffixes.push(super::naval_air_tooltips::carrier_org_suffix(
                        final_efficiency,
                        organisation_ratio,
                        &static_content.localisation,
                        static_content.assets.as_ref(),
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
                            static_content,
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
                    &static_content.recorded_language,
                    &static_content.localisation,
                    static_content.assets.as_ref(),
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
        // The game sums current and max(initial,current) in signed i32.
        let current = group
            .planes
            .iter()
            .fold(0_i32, |sum, plane| sum.wrapping_add(plane.current_planes));
        let maximum = group.planes.iter().fold(0_i32, |sum, plane| {
            sum.wrapping_add(plane.initial_planes.max(plane.current_planes))
        });
        let low_efficiency = (|| {
            for plane in &group.planes {
                // Walk the active second wing list. Skip missing resolved wings
                // or bases; incomplete tables make the result unknown.
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
                        &static_content.assets.naval_air_groups,
                        static_content.assets.naval_defines.as_ref(),
                    )
                    .ok_or("efficiency operands are unavailable")?;
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
    static_content: &StaticContent,
    frame: &DecodedFrame,
    selection: Selection,
    flags: &Flags,
    vision: &Vision,
) -> io::Result<()> {
    match selection {
        Selection::Naval(id) => active(out, static_content, frame, id, flags, vision),
        Selection::NavalResult(id) => {
            super::naval_results::bind(out, static_content, frame, id, flags, vision)
        }
        Selection::Land(_) => Ok(()),
    }
}

// The game uses the same display-box predicate as carrier sortie modifiers:
// The game tests both the ship and its positive equipment pool's bits 22/37.
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
    // These branches of the game do not depend on the equipment predicate.
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
    // The game collapses all unseen incoming entries in their box. Otherwise
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
    static_content: &StaticContent,
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
        Available::Complete(_) => None,
        Available::Truncated { missing, .. } => Some(format!("{missing} rows not reached")),
        Available::Unavailable { reason } => Some((*reason).to_owned()),
    };
    // Incoming live ships whose owner's sharing the hour cannot
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
                    "NavalCombatShips slot {} combat box is withheld: {reason}.",
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
            "NavalCombatShips: {ships} incoming ships of combat {id} omitted: whether the viewer sees them (their owner's shared vision) is unknown, {}.",
            cause.describe()
        ));
    }
    // The game sorts each combat box after grouping. More
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
        static_content,
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
        let Some(asset) = static_content.assets.sprite_size(&name) else {
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
                    static_content,
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
                                static_content,
                            );
                        }
                        Err((slot, reason)) => out.warnings.push(format!(
                            "NavalCombatShips slot {slot}: its maximum organisation is unknown: {reason}; its group's organisation bar is omitted."
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
                    static_content,
                );
            }
            if members.iter().any(|r| r.pride != 0) {
                show(&mut values, "pride");
                center_icon(
                    &mut values,
                    "pride",
                    "GFX_NV_icon_pride_small",
                    asset[0],
                    static_content,
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
                static_content,
            );
            let icon = if key.side == 0 {
                "escape_icon_left"
            } else {
                "escape_icon_right"
            };
            show(&mut values, icon);
            // The game places the escape icon at the bar's final x plus the
            // icon's authored x; CENTER_UP alone would add half the ship width.
            values.entry(icon.into()).or_default().offset[0] =
                escape_bar_x(asset[0], static_content) - asset[0] / 2.0;
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
    out.warnings.push("Format-22 naval rows: native state/formation/equipment/reveal boxes, source0 and retained source1 slots, normalized-state/subunit/formation/country/damaged grouping, copied sprites, native sorted group counts, mean strength/organisation and escape progress are bound. Organisation maxima apply recorded modifiers46/47/27e with default-leader and HQ sources; missing terms in complete captured sources mean zero. Sunk entries follow the content grid's slots. Pride tint, incoming-entry relation edge cases and remaining flag/text geometry remain unverified.".into());
    Ok(())
}

/// The game advances sunk entries by the grid's authored slot direction and
/// limits. Fighting boxes instead supply explicit pixel offsets.
pub(super) fn entry_offset(column: &str, x: f32, y: f32) -> Option<[f32; 2]> {
    (column != "sunk").then_some([x, y])
}
/// The icon uses one-based frame 1 or 2 according to the column side,
/// reversed when any group member is escaping (8) or in state 16.
pub(super) fn icon_frame(side: u8, reversed: bool) -> u32 {
    u32::from(side == 0) ^ u32::from(reversed)
}
/// A combat-box entry is right-aligned inside its 100-wide UPPER_RIGHT grid.
/// Its offset subtracts half of width minus one using truncating integer division.
pub(super) fn combat_box_x(width: f32) -> f32 {
    100.0 - ((width - 1.0) * 0.5).trunc()
}
/// The escape bar's final x in the entry: `width / 2 - bar / 2 + authored x`.
pub(super) fn escape_bar_x(width: f32, static_content: &StaticContent) -> f32 {
    let bar = static_content
        .assets
        .sprite_size("GFX_naval_combat_escape_bar")
        .map_or(54.0, |s| s[0]);
    (width / 2.0).trunc() - (bar / 2.0).trunc() - 1.0
}
fn fixed_ratio(value: i64, maximum: i64) -> i128 {
    i128::from(value) * 100_000 / i128::from(maximum)
}
/// maximum organisation, or which operand refuses it.
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
    // Use the task force's default commander for maximum organisation.
    // Other recorded sources, including the HQ scopes, still contribute.
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
            let name = engine::modifier_keys::name(key).unwrap_or("an unnamed key");
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
fn bar(values: &mut Values, key: &str, value: f32, width: f32, static_content: &StaticContent) {
    show(values, key);
    fill(values, key, value.clamp(0.0, 1.0));
    let name = match key {
        "bar_str" => "GFX_navalcombat_bar_str",
        "bar_org" => "GFX_navalcombat_bar_org",
        _ => "GFX_naval_combat_escape_bar",
    };
    center_icon(values, key, name, width, static_content);
}
fn center_icon(
    values: &mut Values,
    key: &str,
    name: &str,
    width: f32,
    static_content: &StaticContent,
) {
    if let Some(asset) = static_content.assets.sprite_size(name) {
        // CENTER_UP already contributes width/2. Native retains authored x.
        values.entry(key.into()).or_default().offset[0] =
            (width / 2.0).trunc() - width / 2.0 - (asset[0] / 2.0).trunc();
    }
}
/// the attacker's left column writes the country flag into
/// `desc_left` and the count into `desc_right`; the right column swaps them.
/// `desc_left` keeps its authored x (-8, left format). `desc_right` moves to
/// `authored x + icon width - maxWidth` so its right-aligned text ends 8 px
/// past the icon. The flag text is the inline 0x12 country marker.
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

#[cfg(test)]
mod tests;
