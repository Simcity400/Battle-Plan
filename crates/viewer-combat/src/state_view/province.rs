use dashboard_effects::ScriptedEffects;
use super::{loc, paths_named, withhold_named};
use crate::binding::{self, Bindings, Flags};
use crate::content::Content;
use content_reader_gui::gui::native;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{ProvinceRow, Recorded, StateRow, tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;

pub(super) struct Facts<'a> {
    pub province: &'a ProvinceRow,
    pub state: &'a StateRow,
    pub links: &'a BTreeMap<u32, u32>,
    pub flags: &'a Flags,
}

impl Facts<'_> {
    pub(super) fn slot(&self, raw: u32) -> Result<u32, String> {
        if raw == 0 {
            return Ok(0);
        }
        self.links
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("country {raw} has no CountryLinks row"))
    }
}

fn hide_named(out: &mut Bindings, root: &native::Element, name: &str) {
    for path in paths_named(root, name) {
        binding::hide(&mut out.values, &path);
    }
}

fn bind_named<T: Clone>(out: &mut Bindings, root: &native::Element, name: &str, field: binding::Field<T>, value: Result<T, String>) {
    for path in paths_named(root, name) {
        out.bind(&path, field, value.clone());
    }
}

pub(super) fn bind(content: &Content, frame: &DecodedFrame, root: &native::Element, out: &mut Bindings, facts: &Facts<'_>) {
    victory_points(content, root, out, facts);
    name(content, frame, root, out, facts);
    let dynamic = dynamic_modifiers(content, frame, root, out, facts);
    weather(content, frame, root, out, facts, dynamic);
    strategic_locations(content, frame, root, out, facts);
    match &content.assets.required_rules {
        Ok(required) if required.of(facts.province.id).is_some() => {}
        Ok(_) => hide_named(out, root, "province_required_rule"),
        Err(why) => withhold_named(out, root, &["province_required_rule"], &format!("state view required rule icon: {why}")),
    }
}

fn victory_points(content: &Content, root: &native::Element, out: &mut Bindings, facts: &Facts<'_>) {
    let value = facts.province.victory_points;
    if value == 0 {
        hide_named(out, root, "victory_points");
        hide_named(out, root, "victory_points_bg");
        return;
    }
    let text = loc(content, "STATE_VIEW_WICTORY_POINTS", &[("VALUE", Argument::Int32(value))])
        .map_err(|why| format!("state view victory points: {why}"));
    bind_named(out, root, "victory_points", binding::TEXT, text);
}

fn name(content: &Content, frame: &DecodedFrame, root: &native::Element, out: &mut Bindings, facts: &Facts<'_>) {
    let text = (|| {
        let id = facts.province.id;
        let mut name = crate::battle_name::province_display_name(content, frame, id)
            .map_err(|why| why.to_string())??;
        if name.is_empty() {
            name = loc(content, "STATE_PROVINCE", &[])?;
        }
        if facts.slot(facts.state.owner)? != facts.slot(facts.province.controller)? {
            name = loc(content, "STATE_PROVINCE_NAME_OCCUPIED_COLOR", &[("PROVINCE", Argument::Text(&name))])?;
        }
        let flag = binding::country_flag(facts.flags, facts.province.controller)?;
        let tag = format!("{}{flag}", crate::INLINE_FLAG);
        let terrain = content
            .terrain
            .combat_terrain
            .get(&id)
            .ok_or_else(|| format!("province {id} has no terrain in the content"))?;
        loc(
            content,
            "STATE_PROVINCE_PLUS_TAG",
            &[("PROVINCE", Argument::Text(&name)), ("TAG", Argument::Text(&tag)), ("TYPE", Argument::Text(terrain))],
        )
    })()
    .map_err(|why| format!("state view province name: {why}"));
    bind_named(out, root, "province", binding::TEXT, text);
}

fn dynamic_modifiers(content: &Content, frame: &DecodedFrame, root: &native::Element, out: &mut Bindings, facts: &Facts<'_>) -> bool {
    const GRID: &str = "dynamic_modifiers_grid";
    let entries = (|| -> Result<Vec<crate::Values>, String> {
        let rows = frame.complete::<tables::StateDynamicModifiers>().map_err(|e| e.to_string())?;
        let effects = content
            .scripted
            .as_ref()
            .as_ref()
            .map_err(Clone::clone)?
            .effects()?;
        let database = effects
            .view(None)
            .dynamic_modifiers
            .ok_or("the dynamic modifier definitions were not read")?;
        let mut mine: Vec<_> = rows.iter().filter(|row| row.state == facts.state.id).collect();
        mine.sort_by_key(|row| row.ordinal);
        let mut entries = Vec::new();
        for row in mine {
            let token = String::from_utf8_lossy(content_reader_parser::parser::c_string(&row.token)).into_owned();
            let definition = database
                .find(&token)?
                .ok_or_else(|| format!("dynamic modifier {token} is not in the content"))?;
            if definition.icon.is_empty() {
                continue;
            }
            let mut values = crate::Values::new();
            let icon = String::from_utf8_lossy(&definition.icon).into_owned();
            content.assets.acquire_sprite(&icon);
            binding::sprite(&mut values, "icon", icon);
            if row.scope_country == 0 {
                binding::hide(&mut values, "flag");
            } else {
                match binding::country_flag(facts.flags, row.scope_country) {
                    Ok(flag) => values.entry("flag".into()).or_default().flag = Some(flag),
                    Err(why) => {
                        binding::hide(&mut values, "flag");
                        return Err(format!("dynamic modifier {token}'s flag: {why}"));
                    }
                }
            }
            entries.push(values);
        }
        Ok(entries)
    })();
    match entries {
        Ok(entries) => {
            let shown = !entries.is_empty();
            for path in paths_named(root, GRID) {
                let widget = out.values.entry(path).or_default();
                widget.entry_template = Some(super::entry::DYNAMIC_MODIFIER.into());
                widget.entries = entries.clone();
            }
            shown
        }
        Err(why) => {
            withhold_named(out, root, &[GRID], &format!("state view dynamic modifiers: {why}"));
            false
        }
    }
}

pub(super) fn custom_icons(root: &native::Element) -> Vec<(String, &native::Element)> {
    let Some(container) = paths_named(root, "custom_icon_container").into_iter().next() else { return Vec::new() };
    let Some(element) = root.path_named(&container) else { return Vec::new() };
    element.nested().into_iter().map(|child| (format!("{container}/{}", child.name()), child)).collect()
}

pub(super) fn icon_modifier(icon: &native::Element) -> Option<&str> {
    std::str::from_utf8(icon.name_bytes()).ok()?.strip_suffix("_icon")
}

pub(super) fn icon_modifier_names(root: &native::Element) -> Vec<String> {
    custom_icons(root).into_iter().filter_map(|(_, icon)| icon_modifier(icon).map(str::to_owned)).collect()
}

fn static_modifier_icons(row: &replay_data::rows::WeatherProvinceRow) -> Result<Vec<Vec<u8>>, String> {
    match &row.static_modifiers {
        Recorded::Value(Some(modifiers)) => Ok(modifiers.entries.iter().map(|entry| [entry.name.as_slice(), b"_icon"].concat()).collect()),
        Recorded::Value(None) => Err("the province's static modifier list was refused when read".to_owned()),
        Recorded::NotRecorded => Err("the province's static modifier list is not recorded".to_owned()),
    }
}

fn withhold_paths(out: &mut Bindings, paths: &[(String, &native::Element)], why: &str) {
    if !paths.is_empty() {
        let keys: Vec<&str> = paths.iter().map(|(path, _)| path.as_str()).collect();
        out.withhold(&keys, why.to_owned());
    }
}

fn weather(content: &Content, frame: &DecodedFrame, root: &native::Element, out: &mut Bindings, facts: &Facts<'_>, dynamic: bool) {
    const ICONS: [&str; 5] = ["temperature_icons", "modifiers_weather_icons", "modifiers_mud_icon", "modifiers_snow_icons", "impassability_icon"];
    let id = facts.province.id;
    let custom = custom_icons(root);
    let rows = match frame.complete::<tables::WeatherProvinces>() {
        Ok(rows) => rows,
        Err(why) => {
            let why = format!("state view weather: {why}");
            withhold_named(out, root, &ICONS, &why);
            withhold_paths(out, &custom, &why);
            withhold_named(out, root, &["terrain_picture"], &why);
            return;
        }
    };
    let Some(weather) = rows.binary_search_by_key(&id, |row| row.id).ok().map(|at| &rows[at]) else {
        if !dynamic {
            for name in ICONS {
                hide_named(out, root, name);
            }
            hide_named(out, root, "dynamic_modifiers_grid");
        }
        withhold_paths(out, &custom, &format!("state view custom icons: province {id} has no weather, and what the game shows for it is not ported"));
        withhold_named(out, root, &["terrain_picture"], &format!("state view terrain picture: province {id} has no weather, so the game keeps the previous province's picture"));
        return;
    };
    let picture = content.terrain.combat_terrain.get(&id).map(|terrain| {
        let mut sprite = format!("GFX_terrain_{terrain}");
        if weather.snow > 0 {
            sprite.push_str("_winter");
        }
        sprite
    });
    match picture {
        Some(sprite) if content.assets.has_sprite(&sprite) => {
            bind_named(out, root, "terrain_picture", binding::SPRITE, Ok(sprite));
        }
        Some(sprite) => withhold_named(out, root, &["terrain_picture"], &format!("state view terrain picture: the content declares no {sprite}, so the game keeps the previous picture")),
        None => withhold_named(out, root, &["terrain_picture"], &format!("state view terrain picture: province {id} has no terrain in the content")),
    }
    let place = |name: &str| -> Option<[i32; 2]> {
        let element = content.assets.native_windows.gui_type(name.as_bytes())?;
        Some(element.position_type()?.position)
    };
    let (Some(start), Some(step)) = (place("countrystateview_weathericons_position"), place("countrystateview_weathericons_offset")) else {
        let why = "state view weather icons: the content declares no weather icon positions";
        withhold_named(out, root, &ICONS, why);
        withhold_paths(out, &custom, why);
        return;
    };
    let rules = match &content.assets.province_weather {
        Ok(rules) => rules,
        Err(why) => {
            let why = format!("state view weather icons: {why}");
            withhold_named(out, root, &ICONS, &why);
            withhold_paths(out, &custom, &why);
            return;
        }
    };
    let temperature = super::hour_temperature(content, frame, rows, id).map(|value| {
        rules
            .temperature
            .iter()
            .enumerate()
            .filter(|(_, [lower, upper])| *lower <= value && value < *upper)
            .map(|(at, _)| at as u32)
            .next_back()
    });
    let region_weather = (|| {
        let region = content
            .region_of
            .get(&id)
            .ok_or_else(|| format!("province {id} has no strategic region"))?;
        let regions = frame.complete::<tables::WeatherRegions>().map_err(|e| e.to_string())?;
        let row = regions
            .binary_search_by_key(region, |row| row.id)
            .ok()
            .map(|at| &regions[at]);
        Ok::<_, String>(row.and_then(|row| {
            [row.light_rain, row.heavy_rain, row.snow, row.blizzard, row.sandstorm]
                .iter()
                .enumerate()
                .filter(|(_, set)| **set != 0)
                .map(|(at, _)| at as u32)
                .next_back()
        }))
    })();
    let mud = match weather.raw_mud {
        Recorded::Value(mud) => Ok(mud != 0),
        Recorded::NotRecorded => Err("the province's mud is not recorded".to_owned()),
    };
    let snow = weather.snow.wrapping_add(weather.permanent_snow).min(rules.snow_gain_max);
    let [medium, high] = rules.ground_snow;
    let snow = if medium[0] <= snow && snow < medium[1] {
        Some(0)
    } else if high[0] <= snow && snow < high[1] {
        Some(1)
    } else {
        None
    };
    let mut at = start;
    let mut lay = |out: &mut Bindings, name: &str, frame: Result<Option<Option<u32>>, String>| match frame {
        Ok(Some(frame)) => {
            for path in paths_named(root, name) {
                let widget = out.values.entry(path).or_default();
                widget.hidden = false;
                widget.offset = at.map(|value| value as f32);
                if frame.is_some() {
                    widget.frame = frame;
                }
            }
            at = [at[0].wrapping_add(step[0]), at[1].wrapping_add(step[1])];
        }
        Ok(None) => hide_named(out, root, name),
        Err(why) => withhold_named(out, root, &[name], &format!("state view {name}: {why}")),
    };
    lay(out, "temperature_icons", temperature.map(|frame| frame.map(Some)));
    lay(out, "modifiers_weather_icons", region_weather.map(|frame| frame.map(Some)));
    lay(out, "modifiers_mud_icon", mud.map(|mud| mud.then_some(None)));
    lay(out, "modifiers_snow_icons", Ok(snow.map(Some)));
    let placed = static_modifier_icons(weather).map(|icons| {
        for (path, child) in &custom {
            if icons.iter().any(|icon| icon.as_slice() == child.name_bytes()) {
                let widget = out.values.entry(path.clone()).or_default();
                widget.hidden = false;
                widget.offset = at.map(|value| value as f32);
                at = [at[0].wrapping_add(step[0]), at[1].wrapping_add(step[1])];
            } else {
                binding::hide(&mut out.values, path);
            }
        }
    });
    if let Err(why) = &placed {
        withhold_paths(out, &custom, &format!("state view custom icons: {why}"));
    }
    let impassable = content
        .air_context
        .as_ref()
        .map(|context| context.province_hover.impassable_state(facts.state.id));
    match (impassable, placed) {
        (Some(false), _) => hide_named(out, root, "impassability_icon"),
        (Some(true), Ok(())) => {
            for path in paths_named(root, "impassability_icon") {
                let widget = out.values.entry(path).or_default();
                widget.hidden = false;
                widget.offset = at.map(|value| value as f32);
            }
        }
        (Some(true), Err(why)) => withhold_named(out, root, &["impassability_icon"], &format!("state view impassable icon: its place follows the custom icons, and {why}")),
        (None, _) => withhold_named(out, root, &["impassability_icon"], "state view impassable icon: the state content is not loaded"),
    }
}

fn strategic_locations(content: &Content, frame: &DecodedFrame, root: &native::Element, out: &mut Bindings, facts: &Facts<'_>) {
    const GRID: &str = "strategic_locations_grid";
    let id = facts.province.id;
    let entries = (|| -> Result<Vec<crate::Values>, String> {
        let rows = frame.complete::<tables::StateCores>().map_err(|e| e.to_string())?;
        let row = rows
            .iter()
            .find(|row| row.state == facts.state.id)
            .ok_or_else(|| format!("state {} has no StateCores row", facts.state.id))?;
        let (Recorded::Value(state), Recorded::Value(provinces)) = (&row.strategic_locations, &row.province_strategic_locations) else {
            return Err("the strategic locations are not recorded".into());
        };
        let tokens = state
            .iter()
            .filter(|&&pair| pair as u32 == id)
            .map(|&pair| (pair >> 32) as u32)
            .chain(provinces.iter().filter(|&&pair| (pair >> 32) as u32 == id).map(|&pair| pair as u32));
        let names = dashboard_text::table_access::TokenNames::new(frame).map_err(|e| e.to_string())?;
        tokens
            .map(|token| {
                let spelling = names.name(token).map_err(|e| e.to_string())?;
                let mut values = crate::Values::new();
                let sprite = format!("GFX_strategic_location_{spelling}");
                if content.assets.has_sprite(&sprite) {
                    binding::sprite(&mut values, "icon", sprite);
                }
                Ok(values)
            })
            .collect()
    })();
    match entries {
        Ok(entries) => {
            for path in paths_named(root, GRID) {
                let widget = out.values.entry(path).or_default();
                widget.entry_template = Some(super::entry::STRATEGIC_LOCATION.into());
                widget.entries = entries.clone();
            }
        }
        Err(why) => withhold_named(out, root, &[GRID], &format!("state view strategic locations: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{ProvinceStaticModifier, ProvinceStaticModifiers, WeatherProvinceRow};

    #[test]
    fn custom_icons_are_the_containers_children_named_by_present_static_modifiers() {
        content_reader_script::test_game::enter();
        let gui = native::read(
            b"guiTypes={containerWindowType={name=countrystateview containerWindowType={name=province_info_window containerWindowType={name=custom_icon_container iconType={name=flooded_icon} iconType={name=the_great_wall_icon}}}}}",
        );
        let root = gui.templates().iter().find(|root| root.is_named(super::super::WINDOW)).unwrap();
        let custom: Vec<String> = custom_icons(root).into_iter().map(|(path, _)| path).collect();
        assert_eq!(custom, ["province_info_window/custom_icon_container/flooded_icon", "province_info_window/custom_icon_container/the_great_wall_icon"]);
        let row = |static_modifiers| WeatherProvinceRow { static_modifiers, ..Default::default() };
        let present = ProvinceStaticModifiers { entries: vec![ProvinceStaticModifier { name: b"the_great_wall".to_vec(), days: 0 }, ProvinceStaticModifier { name: b"flooded".to_vec(), days: 12 }] };
        assert_eq!(static_modifier_icons(&row(Recorded::Value(Some(present)))), Ok(vec![b"the_great_wall_icon".to_vec(), b"flooded_icon".to_vec()]));
        assert_eq!(static_modifier_icons(&row(Recorded::Value(None))), Err("the province's static modifier list was refused when read".to_owned()));
        assert_eq!(static_modifier_icons(&row(Recorded::NotRecorded)), Err("the province's static modifier list is not recorded".to_owned()));
    }
}
