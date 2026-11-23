use super::{binding::Bindings, content::Content};

mod buildings;
mod hovers;
mod occupation;
mod owner;
mod province;
mod resources;
mod rule;
mod stakeholders;
pub(crate) use owner::OWNER_INFO_MODIFIERS;

pub(crate) fn icon_modifier_names(windows: &viewer_native_gui::native_windows::NativeWindows) -> Vec<String> {
    windows.gui_type(WINDOW).map(province::icon_modifier_names).unwrap_or_default()
}

pub(super) fn hour_temperature(content: &Content, frame: &DecodedFrame, rows: &[replay_data::rows::WeatherProvinceRow], province: u32) -> Result<i64, String> {
    content
        .air_context
        .as_ref()
        .ok_or("the province temperature content is not loaded")?
        .province_hover
        .temperature_at(frame, rows, province)
}
use content_reader_gui::gui::native;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{Recorded, tables};
use replay_frame::frame::DecodedFrame;
use std::io;
use viewer_engagements::combat_selection::Selection;
use viewer_foundation::{perspective::Vision, tooltip_text::displayable};

pub(super) const WINDOW: &[u8] = b"countrystateview";

pub(super) mod entry {
    pub const STATE_BUILDING: &str = "state_building_entry";
    pub const SHARED_SLOT: &str = "state_shared_slot_building_entry";
    pub const PROVINCE_BUILDING: &str = "province_building_entry";
    pub const LANDMARK: &str = "province_building_special_icon_entry";
    pub const DYNAMIC_MODIFIER: &str = "state_view_dynamic_modifier_entry";
    pub const STRATEGIC_LOCATION: &str = "province_view_strategic_location_entry";
    pub const RESOURCE: &str = "state_resource_entry";
    pub const STAKEHOLDER: &str = "state_stakeholder_entry";
    pub const RESISTANCE: &str = "state_resistance_status_container";
    pub const COMPLIANCE: &str = "state_compliance_status_container";
}

pub(super) const TEMPLATES: &[&[u8]] = &[
    WINDOW,
    entry::STATE_BUILDING.as_bytes(),
    entry::SHARED_SLOT.as_bytes(),
    entry::PROVINCE_BUILDING.as_bytes(),
    entry::LANDMARK.as_bytes(),
    entry::DYNAMIC_MODIFIER.as_bytes(),
    entry::STRATEGIC_LOCATION.as_bytes(),
    entry::RESOURCE.as_bytes(),
    entry::STAKEHOLDER.as_bytes(),
    entry::RESISTANCE.as_bytes(),
    entry::COMPLIANCE.as_bytes(),
];
const SCORCHED_FEATURE: u32 = 1 << 7;


const PROVINCE_PART: [&str; 17] = [
    "victory_points",
    "province",
    "terrain_picture",
    "temperature_icons",
    "modifiers_weather_icons",
    "modifiers_mud_icon",
    "modifiers_snow_icons",
    "custom_icon_container",
    "impassability_icon",
    "province_required_rule",
    "dynamic_modifiers_grid",
    "strategic_locations_grid",
    "state_claims",
    "state_owners",
    "controller_flag",
    "controller_flag_border",
    "victory_points_bg",
];

const STATE_PART: [&str; 7] = [
    "state_resources_entries",
    "resistance_container",
    "state_building_entries",
    "state_shared_slot_building_entries",
    "province_building_entries",
    "province_custom_icon_building_entries",
    "building_modifiers_icon",
];

pub(super) fn loc(content: &Content, key: &str, arguments: &[(&str, Argument<'_>)]) -> Result<String, String> {
    content
        .localisation
        .localise_with(key.as_bytes(), arguments)
        .map_err(|why| format!("{key}: {why}"))
        .and_then(|text| displayable(content.assets.as_ref(), text))
}

pub(super) fn paths_named(root: &native::Element, name: &str) -> Vec<String> {
    super::gui::elements(root)
        .into_iter()
        .filter(|(path, _)| path.rsplit('/').next() == Some(name))
        .map(|(path, _)| path)
        .collect()
}

pub(super) fn seed(content: &Content, root: &native::Element, out: &mut Bindings) {
    for (path, element) in super::gui::elements(root) {
        let authored = element
            .text_box()
            .map(|v| v.text.as_slice())
            .or_else(|| element.button().map(|v| v.text.as_slice()));
        if let Some(authored) = authored {
            let value = std::str::from_utf8(authored)
                .map_err(|_| "state view text key is not UTF-8".to_owned())
                .and_then(|key| if key.is_empty() { Ok(String::new()) } else { loc(content, key, &[]) });
            out.bind(&path, super::binding::TEXT, value);
        }
    }
}

pub(super) fn withhold_named(out: &mut Bindings, root: &native::Element, names: &[&str], why: &str) {
    let paths: Vec<String> = names.iter().flat_map(|name| paths_named(root, name)).collect();
    if paths.is_empty() {
        return;
    }
    let keys: Vec<&str> = paths.iter().map(String::as_str).collect();
    out.withhold(&keys, why.to_owned());
}

fn text_named(out: &mut Bindings, root: &native::Element, name: &str, value: Result<String, String>) {
    for path in paths_named(root, name) {
        out.bind(&path, super::binding::TEXT, value.clone());
    }
}

pub(super) fn bind(
    content: &Content,
    frame: &DecodedFrame,
    vision: &Vision,
    province: u32,
    nato: bool,
    rule_relations: Option<&super::RuleRelations>,
) -> io::Result<Bindings> {
    let mut out = Bindings::default();
    let Some(root) = content.assets.native_windows.gui_type(WINDOW) else {
        out.refuse("state view window is absent from the recorded content stack".into());
        return Ok(out);
    };
    seed(content, root, &mut out);
    let state = state_of(frame, province);
    let row = state.as_ref().map_err(Clone::clone).and_then(|&id| {
        frame
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.id == id)
            .cloned()
            .ok_or_else(|| format!("state {id} has no States row"))
    });
    let name = row.as_ref().map_err(Clone::clone).and_then(|row| {
        viewer_foundation::tooltip_text::state_name(
            &content.localisation,
            &content.recorded_language,
            row.id,
            &row.custom_name,
            &row.owner_name,
            || {
                let key = content
                    .state_name_keys
                    .get(&row.id)
                    .ok_or("state content name absent")?
                    .as_ref()
                    .map_err(Clone::clone)?;
                loc(content, key, &[])
            },
        )
    });
    text_named(&mut out, root, "state_name", name.clone().map_err(|why| format!("state view name: {why}")));
    let population = row.as_ref().map_err(Clone::clone).and_then(|row| match row.population {
        Recorded::Value(population) => loc(content, "STATE_POPULATION_VALUE", &[("VAL", Argument::Int32(population))]),
        Recorded::NotRecorded => Err("the state's population is not recorded".into()),
    });
    text_named(&mut out, root, "manpower", population.map_err(|why| format!("state view population: {why}")));
    let energy = row.as_ref().map_err(Clone::clone).and_then(|row| match vision.shares_vision_proven(slot(frame, row.controller)?) {
        Ok(false) => loc(content, "PRODUCTION_UNKNOWN_ENERGY_AMOUNT", &[]),
        Ok(true) => buildings::state_energy(content, frame, row).map(|usage| buildings::round_whole(usage).to_string()),
        Err(why) => Err(why.clause()),
    });
    text_named(&mut out, root, "energy_text", energy.map_err(|why| format!("state view energy: {why}")));
    let slots = row.as_ref().map_err(Clone::clone).and_then(|row| {
        let maximum = content.assets.max_shared_slots.clone()?;
        let slots = buildings::shared_slots(buildings::building_inputs(frame, row)?, row.owner, maximum)?;
        loc(content, "UNLOCKED_SLOTS", &[("NUM", Argument::Int32(slots)), ("MAX", Argument::Int32(maximum))])
    });
    text_named(&mut out, root, "shared_slot_count", slots.map_err(|why| format!("state view shared slots: {why}")));
    match scorched_feature(frame) {
        Ok(false) => {
            for path in paths_named(root, "scorched_earth_state_button") {
                super::binding::hide(&mut out.values, &path);
            }
        }
        Ok(true) => withhold_named(
            &mut out,
            root,
            &["scorched_earth_state_button"],
            "state view scorched earth button: its command validity for the observed country is not recorded",
        ),
        Err(why) => withhold_named(&mut out, root, &["scorched_earth_state_button"], &format!("state view scorched earth button: {why}")),
    }
    let parts = (|| -> Result<(replay_data::rows::ProvinceRow, std::collections::BTreeMap<u32, u32>, super::binding::Flags), String> {
        let provinces = frame.complete::<tables::Provinces>().map_err(|e| e.to_string())?;
        let picked = provinces
            .binary_search_by_key(&province, |row| row.id)
            .map(|at| provinces[at].clone())
            .map_err(|_| format!("Provinces province {province} is absent"))?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let flags = super::binding::flags(content, frame).map_err(|e| e.to_string())?;
        Ok((picked, links, flags))
    })();
    match (&row, &parts) {
        (Ok(state), Ok((picked, links, flags))) => {
            let facts = province::Facts { province: picked, state, links, flags };
            province::bind(content, frame, root, &mut out, &facts);
            stakeholders::bind(content, frame, root, &mut out, &facts, &name);
            resources::bind(content, frame, root, &mut out, state.id);
            occupation::bind(content, frame, root, &mut out, &facts, vision.ours(), nato, &name);
            buildings::bind(content, frame, root, &mut out, &facts, vision);
            hovers::bind(content, frame, root, &mut out, &facts, vision, &name);
            rule::bind(content, frame, root, &mut out, picked.id, vision.ours(), rule_relations);
        }
        (Err(why), _) | (_, Err(why)) => {
            for name in PROVINCE_PART.iter().chain(&STATE_PART) {
                withhold_named(&mut out, root, &[name], &format!("state view {name}: {why}"));
            }
            hovers::withhold(&mut out, root);
        }
    }
    super::tooltips::bind_authored(
        &mut out,
        content.assets.as_ref(),
        &content.localisation,
        &content.assets.native_windows,
        Selection::State(province),
    );
    Ok(out)
}

fn state_of(frame: &DecodedFrame, province: u32) -> Result<u32, String> {
    let rows = frame.complete::<tables::Provinces>().map_err(|e| e.to_string())?;
    let state = rows
        .binary_search_by_key(&province, |row| row.id)
        .map(|at| rows[at].state)
        .map_err(|_| format!("Provinces province {province} is absent"))?;
    if state == 0 {
        return Err(format!("province {province} has no state at this hour"));
    }
    Ok(state)
}

fn slot(frame: &DecodedFrame, raw: u32) -> Result<u32, String> {
    let links = frame.complete::<tables::CountryLinks>().map_err(|e| e.to_string())?;
    links
        .binary_search_by_key(&raw, |row| row.id)
        .map(|at| links[at].canonical_slot)
        .map_err(|_| format!("country {raw} has no CountryLinks row"))
}

fn scorched_feature(frame: &DecodedFrame) -> Result<bool, String> {
    match frame.complete::<tables::SimulationFeatures>() {
        Ok([features]) => Ok(features.dlc_mask & SCORCHED_FEATURE != 0),
        Ok(_) => Err("the DLC mask needs one SimulationFeatures row".into()),
        Err(why) => Err(why.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn withheld_controls_hide_wherever_a_mod_nests_them_and_count_once() {
        content_reader_script::test_game::enter();
        let gui = native::read(
            b"guiTypes={containerWindowType={name=countrystateview containerWindowType={name=moved gridBoxType={name=state_shared_slot_building_entries}} instantTextboxType={name=state_name} containerWindowType={name=province_info_window iconType={name=province_building_entries} iconType={name=province_custom_icon_building_entries}}}}",
        );
        let root = gui.templates().iter().find(|root| root.is_named(WINDOW)).unwrap();
        assert_eq!(paths_named(root, "state_shared_slot_building_entries"), ["moved/state_shared_slot_building_entries"]);
        let mut out = Bindings::default();
        withhold_named(&mut out, root, &["state_shared_slot_building_entries", "absent"], "why");
        withhold_named(&mut out, root, &["absent"], "unseen");
        assert!(out.values["moved/state_shared_slot_building_entries"].hidden);
        assert!(!out.values.contains_key("province_info_window/province_building_entries"));
        assert!(!out.values.contains_key("state_name"));
        assert_eq!(out.refused().len(), 1, "{:?}", out.refused());
    }
}
