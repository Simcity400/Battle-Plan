use super::{loc, paths_named, province::Facts};
use crate::binding::Bindings;
use crate::modifier_text::{Block, Definitions};
use crate::tooltips::{hover_text, refuse_hovers, set_hover};
use content_reader_gui::gui::native;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{Recorded, tables};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::perspective::{SharingUnknown, Vision};

const POPULATION: &str = "state view population hover: its manpower breakdown lists the sources of the state's recruitable population and growth modifiers for its owner, which are not recorded";

const WITHHELD: &[(&str, &str)] = &[
    ("population_icon", POPULATION),
    ("manpower", POPULATION),
    ("shared_slot_count", "state view shared slots hover: its modifier breakdown is not recorded"),
];

const NAME_SEPARATOR: &str = " & ";

const ATTRITION: [u32; 5] = {
    use engine_executable::modifier_keys::*;
    [WINTER_ATTRITION, WINTER_ATTRITION_FACTOR, HEAT_ATTRITION, HEAT_ATTRITION_FACTOR, ATTRITION]
};

fn joined_names(names: impl IntoIterator<Item = String>) -> String {
    let mut joined = String::new();
    for name in names {
        if !joined.is_empty() {
            joined.push_str(NAME_SEPARATOR);
        }
        joined.push_str(&name);
    }
    joined
}

fn weather_names(content: &super::Content, frame: &DecodedFrame, rows: &[replay_data::rows::WeatherProvinceRow], row: &replay_data::rows::WeatherProvinceRow) -> Result<Vec<String>, String> {
    let ranges = content.assets.temperature.as_ref().map_err(Clone::clone)?;
    let temperature = super::hour_temperature(content, frame, rows, row.id)?;
    let mut names = Vec::new();
    if let Some(range) = ranges.ranges.iter().find(|range| range.lower <= temperature && temperature < range.upper) {
        names.push(match &range.modifier {
            Some(key) => loc(content, key, &[])?,
            None => String::new(),
        });
    }
    for entry in &static_modifiers(row)?.entries {
        names.push(loc(content, modifier_name(entry)?, &[])?);
    }
    Ok(names)
}

fn static_modifiers(row: &replay_data::rows::WeatherProvinceRow) -> Result<&replay_data::rows::ProvinceStaticModifiers, String> {
    match &row.static_modifiers {
        Recorded::Value(Some(modifiers)) => Ok(modifiers),
        Recorded::Value(None) => Err("the province's static modifier list was refused when read".into()),
        Recorded::NotRecorded => Err("the province's static modifier list is not recorded".into()),
    }
}

fn modifier_name(entry: &replay_data::rows::ProvinceStaticModifier) -> Result<&str, String> {
    std::str::from_utf8(&entry.name).map_err(|_| format!("static modifier {} is not named in UTF-8", String::from_utf8_lossy(&entry.name)))
}

fn adds_attrition(block: &Block, definitions: &Definitions<'_>) -> Result<bool, String> {
    for key in ATTRITION {
        if block.value(definitions, key).map_err(|e| e.to_string())? > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn peacetime(content: &super::Content) -> Result<String, String> {
    Ok(format!("\u{11}g{}\u{11}!", loc(content, "WEATHER_PEACETIME", &[])?))
}

fn trailing_newlines(text: &mut String, count: usize) {
    if text.is_empty() {
        return;
    }
    let have = text.bytes().rev().take(count).take_while(|&byte| byte == b'\n').count();
    text.extend(std::iter::repeat_n('\n', count - have));
}

fn definitions<'f>(content: &super::Content, frame: &'f DecodedFrame) -> Result<Definitions<'f>, String> {
    Definitions::read(frame, &content.modifier_names).map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

fn temperature(content: &super::Content, frame: &DecodedFrame, province: u32) -> Result<Option<(String, String)>, String> {
    let rows = frame.complete::<tables::WeatherProvinces>().map_err(|e| e.to_string())?;
    let Ok(at) = rows.binary_search_by_key(&province, |row| row.id) else {
        return Ok(None);
    };
    let row = &rows[at];
    let modifiers = row
        .modifiers
        .require("WeatherProvinces.modifiers")?
        .as_ref()
        .ok_or("the province's weather modifier accumulator was refused when recorded")?;
    let definitions = definitions(content, frame)?;
    let block = Block { values: modifiers.entries.iter().map(|entry| (entry.key, entry.value)).collect(), ..Block::default() };
    if !block.kept(&definitions).map_err(|e| e.to_string())? {
        return Ok(Some((String::new(), String::new())));
    }
    let name = joined_names(weather_names(content, frame, rows, row)?);
    let main = loc(content, "HEADER", &[("KEY", Argument::Text(&name))])?;
    let mut delayed = block
        .describe(&definitions, content.assets.as_ref(), &content.localisation, crate::fixed::ONE, "", u32::MAX)
        .map_err(|e| e.to_string())?;
    if adds_attrition(&block, &definitions)? {
        trailing_newlines(&mut delayed, 2);
        delayed.push_str(&peacetime(content)?);
    }
    Ok(Some((main, delayed)))
}

fn custom_icon(content: &super::Content, definitions: &Definitions<'_>, name: &str, days: i32) -> Result<(String, String), String> {
    let mut main = loc(content, "HEADER", &[("KEY", Argument::Text(&loc(content, name, &[])?))])?;
    if days > 0 {
        main.push_str(&loc(content, "TIMED_PROVINCE_MODIFIER", &[("NUM", Argument::Int32(days))])?);
    }
    let authored = content
        .assets
        .province_icon_modifiers
        .get(name)
        .ok_or_else(|| format!("static modifier {name} was not read with the custom icons"))?
        .as_ref()
        .map_err(Clone::clone)?;
    let (block, _) = Block::build(definitions, authored).map_err(|e| e.to_string())?;
    main.push_str(
        &block
            .describe(definitions, content.assets.as_ref(), &content.localisation, crate::fixed::ONE, "", u32::MAX)
            .map_err(|e| e.to_string())?,
    );
    let delayed = if adds_attrition(&block, definitions)? { peacetime(content)? } else { String::new() };
    Ok((main, delayed))
}

fn custom_icons(content: &super::Content, frame: &DecodedFrame, root: &native::Element, out: &mut Bindings, province: u32) {
    let icons = super::province::custom_icons(root);
    let Ok(rows) = frame.complete::<tables::WeatherProvinces>() else { return };
    let Ok(at) = rows.binary_search_by_key(&province, |row| row.id) else { return };
    let Ok(present) = static_modifiers(&rows[at]) else { return };
    let definitions = definitions(content, frame);
    for (path, icon) in icons {
        let Some((name, days)) = super::province::icon_modifier(icon)
            .and_then(|name| present.entries.iter().find(|entry| entry.name == name.as_bytes()).map(|entry| (name, entry.days)))
        else {
            continue;
        };
        match definitions.as_ref().map_err(Clone::clone).and_then(|definitions| custom_icon(content, definitions, name, days)) {
            Ok((main, delayed)) => set_hover(&mut out.values, path, hover_text(&main), hover_text(&delayed)),
            Err(why) => {
                set_hover(&mut out.values, path, None, None);
                out.refuse(format!("state view custom icon hover: {why}"));
            }
        }
    }
}

fn weather_flags(row: &replay_data::rows::WeatherRegionRow) -> [u8; 6] {
    [row.light_rain, row.heavy_rain, row.snow, row.blizzard, row.sandstorm, row.arctic_water]
}

fn region_weather_name(content: &super::Content, flags: [u8; 6]) -> Result<String, String> {
    let mut names = Vec::new();
    for (set, key) in flags.into_iter().zip(crate::naval_environment::WEATHER_MODIFIERS) {
        if set != 0 {
            names.push(loc(content, key, &[])?);
        }
    }
    if names.is_empty() {
        return loc(content, "weather_none", &[]);
    }
    Ok(names.join(", "))
}

pub(super) fn add_block(sum: &mut crate::modifier_text::Block, other: crate::modifier_text::Block, name: &str) -> Result<(), String> {
    if !other.custom_tooltips.is_empty() || other.added {
        return Err(format!("static modifier {name} holds custom tooltips or a child, whose merged order is not ported"));
    }
    for (index, value) in other.values {
        let entry = sum.values.entry(index).or_default();
        *entry = entry.wrapping_add(value);
    }
    sum.hidden.extend(other.hidden);
    Ok(())
}

fn region_weather(content: &super::Content, frame: &DecodedFrame, province: u32) -> Result<Option<(String, String)>, String> {
    let region = *content.region_of.get(&province).ok_or_else(|| format!("province {province} has no strategic region"))?;
    let rows = frame.complete::<tables::WeatherRegions>().map_err(|e| e.to_string())?;
    let Ok(at) = rows.binary_search_by_key(&region, |row| row.id) else {
        return Ok(None);
    };
    let flags = weather_flags(&rows[at]);
    let name = region_weather_name(content, flags)?;
    let main = if name.is_empty() { String::new() } else { loc(content, "HEADER", &[("KEY", Argument::Text(&name))])? };
    let definitions = crate::modifier_text::Definitions::read(frame, &content.modifier_names)
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let block = |modifiers: &content_reader_script::modifiers::Modifiers| {
        crate::modifier_text::Block::build(&definitions, modifiers).map(|(block, _)| block).map_err(|e| e.to_string())
    };
    let mut sum = crate::modifier_text::Block::default();
    for ((set, name), modifiers) in flags.into_iter().zip(crate::naval_environment::WEATHER_MODIFIERS).zip(&content.assets.naval_weather_modifiers) {
        if set != 0 {
            add_block(&mut sum, block(modifiers.as_ref().map_err(Clone::clone)?)?, name)?;
        }
    }
    let names = &content
        .strategic_regions
        .get(&region)
        .ok_or_else(|| format!("strategic region {region} has no content template"))?
        .static_modifier_names;
    if !names.is_empty() {
        let values = crate::naval_environment::region_custom_sea(&content.region_of, frame, &rows[at], region)?;
        let active = crate::naval_environment::active_custom_sea(names, &values)
            .ok_or("the region's recorded custom weather values do not match its content modifiers")?;
        for name in active {
            let modifiers = content
                .assets
                .naval_custom_sea_modifiers
                .get(name)
                .ok_or_else(|| format!("static modifier {name} has no block"))?
                .as_ref()
                .map_err(Clone::clone)?;
            add_block(&mut sum, block(modifiers)?, name)?;
        }
    }
    let delayed = sum
        .describe(&definitions, content.assets.as_ref(), &content.localisation, crate::fixed::ONE, "", u32::MAX)
        .map_err(|e| e.to_string())?;
    Ok(Some((main, delayed)))
}

fn province(content: &super::Content, frame: &DecodedFrame, facts: &Facts) -> Result<String, String> {
    let controller = facts.slot(facts.province.controller)?;
    let key = if facts.slot(facts.state.owner)? == controller { "STATE_PROVINCE_DESC" } else { "STATE_PROVINCE_OCCUPIED_DESC" };
    let names = content_reader_country_names::country_names::CountryNames::read(frame, &content.localisation);
    loc(content, key, &[("CONTROLLER", Argument::Text(&names.name(controller)?))])
}

fn energy(content: &super::Content, sees: Result<bool, SharingUnknown>) -> Result<String, String> {
    match sees {
        Ok(false) => loc(content, "PRODUCTION_NO_INTEL_ON_POWER", &[]),
        Ok(true) => Err("the state's factory energy usage breakdown is not recorded".into()),
        Err(why) => Err(why.clause()),
    }
}

fn snow_key(snow: i64, permanent: i64, maximum: i64, [medium, high]: [[i64; 2]; 2]) -> Option<&'static str> {
    let snow = snow.wrapping_add(permanent).min(maximum);
    if medium[0] <= snow && snow < medium[1] {
        Some("weather_ground_snow_medium")
    } else if high[0] <= snow && snow < high[1] {
        Some("weather_ground_snow_high")
    } else {
        None
    }
}

fn weather(content: &super::Content, frame: &DecodedFrame, province: u32, heading: impl FnOnce(&replay_data::rows::WeatherProvinceRow) -> Result<Option<&'static str>, String>) -> Result<Option<(String, String)>, String> {
    let rows = frame.complete::<tables::WeatherProvinces>().map_err(|e| e.to_string())?;
    let Ok(at) = rows.binary_search_by_key(&province, |row| row.id) else {
        return Ok(None);
    };
    let main = match heading(&rows[at])? {
        Some(key) => {
            let name = loc(content, key, &[])?;
            loc(content, "HEADER", &[("KEY", Argument::Text(&name))])?
        }
        None => String::new(),
    };
    Ok(Some((main, crate::terrain_mode::weather_modifiers(content, frame, province)?)))
}

pub(super) fn bind(
    content: &super::Content,
    frame: &DecodedFrame,
    root: &native::Element,
    out: &mut Bindings,
    facts: &Facts,
    vision: &Vision,
    state_name: &Result<String, String>,
) {
    let none = super::resources::none_text(frame, facts.state.id).and_then(|none| {
        if !none {
            return Ok(None);
        }
        let state = state_name.clone()?;
        loc(content, "STATE_NO_RESOURCES", &[("STATE", Argument::Text(&state))]).map(Some)
    });
    match none.map_err(|why| format!("state view resources hover: {why}")) {
        Ok(None) => {}
        Ok(Some(text)) => {
            for name in ["state_resources", "state_resources_entries"] {
                hover(out, root, name, Ok(text.clone()));
            }
        }
        Err(why) => {
            for name in ["state_resources", "state_resources_entries"] {
                hover(out, root, name, Err(why.clone()));
            }
        }
    }
    hover(out, root, "victory_points", loc(content, "STATE_VIEW_WICTORY_POINTS_DESC", &[]).map_err(|why| format!("state view victory points hover: {why}")));
    hover(out, root, "province", province(content, frame, facts).map_err(|why| format!("state view province hover: {why}")));
    let mut refused = Vec::new();
    let terrain = crate::terrain_mode::province_text(content, frame, facts.province.id, vision, &mut refused);
    for why in refused {
        out.refuse(format!("state view terrain hover: {why}"));
    }
    hover(out, root, "terrain_picture", terrain.map_err(|why| format!("state view terrain hover: {why}")));
    let sees = facts.slot(facts.state.controller).and_then(|controller| energy(content, vision.shares_vision_proven(controller)));
    hover(out, root, "energy_text", sees.map_err(|why| format!("state view energy hover: {why}")));
    let id = facts.province.id;
    let mud = weather(content, frame, id, |_| Ok(Some("weather_mud")));
    replaceable(out, root, "modifiers_mud_icon", mud.map_err(|why| format!("state view mud hover: {why}")));
    let snow = match &content.assets.province_weather {
        Ok(rules) => weather(content, frame, id, |row| Ok(snow_key(row.snow, row.permanent_snow, rules.snow_gain_max, rules.ground_snow))),
        Err(why) => Err(why.clone()),
    };
    replaceable(out, root, "modifiers_snow_icons", snow.map_err(|why| format!("state view snow hover: {why}")));
    let temperature = temperature(content, frame, id).map_err(|why| format!("state view temperature hover: {why}"));
    owned(out, root, "temperature_icons", &temperature);
    custom_icons(content, frame, root, out, id);
    let region = region_weather(content, frame, id).map_err(|why| format!("state view weather hover: {why}"));
    owned(out, root, "modifiers_weather_icons", &region);
    withhold(out, root);
}

fn owned(out: &mut Bindings, root: &native::Element, name: &str, text: &Result<Option<(String, String)>, String>) {
    for path in paths_named(root, name) {
        match text {
            Ok(Some((main, delayed))) => set_hover(&mut out.values, path, hover_text(main), hover_text(delayed)),
            Ok(None) => {}
            Err(why) => {
                set_hover(&mut out.values, path, None, None);
                out.refuse(why.clone());
            }
        }
    }
}

pub(super) fn withhold(out: &mut Bindings, root: &native::Element) {
    for (name, why) in WITHHELD {
        let paths = paths_named(root, name);
        if !paths.is_empty() {
            refuse_hovers(&mut out.values, paths);
            out.refuse((*why).to_owned());
        }
    }
}

fn hover(out: &mut Bindings, root: &native::Element, name: &str, text: Result<String, String>) {
    for path in paths_named(root, name) {
        match &text {
            Ok(text) => set_hover(&mut out.values, path, hover_text(text), None),
            Err(why) => {
                set_hover(&mut out.values, path, None, None);
                out.refuse(why.clone());
            }
        }
    }
}

fn replaceable(out: &mut Bindings, root: &native::Element, name: &str, text: Result<Option<(String, String)>, String>) {
    for path in paths_named(root, name) {
        match &text {
            Ok(Some((main, delayed))) => {
                let widget = out.values.entry(path).or_default();
                widget.tooltip = hover_text(main);
                widget.tooltip_description = hover_text(delayed);
            }
            Ok(None) => {}
            Err(why) => {
                set_hover(&mut out.values, path, None, None);
                out.refuse(why.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn withheld_dynamic_hovers_keep_no_authored_seed_and_count_once_per_control() {
        content_reader_script::test_game::enter();
        let gui = native::read(
            b"guiTypes={containerWindowType={name=countrystateview containerWindowType={name=moved iconType={name=shared_slot_count pdx_tooltip=AUTHORED}} instantTextboxType={name=manpower} instantTextboxType={name=state_name}}}",
        );
        let root = gui.templates().iter().find(|root| root.is_named(super::super::WINDOW)).unwrap();
        let mut out = Bindings::default();
        withhold(&mut out, root);
        let icon = &out.values["moved/shared_slot_count"];
        assert!(icon.hover_owned && icon.tooltip.is_none());
        assert!(out.values["manpower"].hover_owned);
        assert!(!out.values.contains_key("state_name"));
        assert_eq!(out.refused().len(), 2, "{:?}", out.refused());
    }

    #[test]
    fn region_weather_blocks_sum_and_a_custom_tooltip_is_refused() {
        use crate::modifier_text::Block;
        let mut sum = Block::default();
        add_block(&mut sum, Block { values: [(1, 10), (2, 5)].into(), hidden: [2].into(), ..Block::default() }, "a").unwrap();
        add_block(&mut sum, Block { values: [(1, -4), (3, 7)].into(), ..Block::default() }, "b").unwrap();
        assert_eq!(sum.values, [(1, 6), (2, 5), (3, 7)].into());
        assert_eq!(sum.hidden, [2].into());
        let keyed = Block { custom_tooltips: vec![b"KEY".to_vec()], ..Block::default() };
        assert!(add_block(&mut sum, keyed, "c").unwrap_err().contains("static modifier c"));
    }

    #[test]
    fn the_weather_name_joins_after_the_first_name_that_wrote_text() {
        let names = |names: &[&str]| joined_names(names.iter().map(|name| (*name).to_owned()));
        assert_eq!(names(&[]), "");
        assert_eq!(names(&["Very cold", "Flooded"]), "Very cold & Flooded");
        assert_eq!(names(&["", "Flooded", "Mined"]), "Flooded & Mined");
        assert_eq!(names(&["Hot", "", "Mined"]), "Hot &  & Mined");
    }

    #[test]
    fn attrition_counts_through_the_getter() {
        use engine_executable::modifier_keys::{ATTRITION as PLAIN, HEAT_ATTRITION, WINTER_ATTRITION};
        let row = |index, flags, feature_selector| replay_data::rows::ModifierDefinitionRow { index, token: index, flags, feature_selector, ..Default::default() };
        let rows = [row(PLAIN, 0, 0), row(WINTER_ATTRITION, 0, 2), row(HEAT_ATTRITION, 1 << 4, 0)];
        let definitions = Definitions::from_parts(&rows, &[], Some(1));
        let block = |values: &[(u32, i64)]| Block { values: values.iter().copied().collect(), ..Block::default() };
        assert_eq!(adds_attrition(&block(&[]), &definitions), Ok(false));
        assert_eq!(adds_attrition(&block(&[(PLAIN, -10)]), &definitions), Ok(false));
        assert_eq!(adds_attrition(&block(&[(PLAIN, 10)]), &definitions), Ok(true));
        assert_eq!(adds_attrition(&block(&[(WINTER_ATTRITION, 10)]), &definitions), Ok(false), "an ineligible definition reads zero");
        assert_eq!(adds_attrition(&block(&[(HEAT_ATTRITION, 5)]), &definitions), Ok(true));
        assert!(adds_attrition(&block(&[(engine_executable::modifier_keys::HEAT_ATTRITION_FACTOR, 5)]), &definitions).is_err(), "an entry without a definition is refused");
    }

    #[test]
    fn the_snow_heading_follows_the_capped_effective_snow() {
        let ranges = [[10, 20], [20, 40]];
        assert_eq!(snow_key(5, 4, 100, ranges), None);
        assert_eq!(snow_key(5, 5, 100, ranges), Some("weather_ground_snow_medium"));
        assert_eq!(snow_key(15, 10, 100, ranges), Some("weather_ground_snow_high"));
        assert_eq!(snow_key(50, 0, 30, ranges), Some("weather_ground_snow_high"));
        assert_eq!(snow_key(50, 0, 100, ranges), None);
    }
}
