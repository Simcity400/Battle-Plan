use content_reader_core::ResolvedLayers;
use content_reader_core::scripted_map_modes::LayerType;
use std::{collections::BTreeSet, io};

fn reaches_sharing(
    items: &[content_reader_core::parser::Item],
    templates: &content_reader_core::scripted_triggers::ScriptedTriggers,
    visited: &mut BTreeSet<Vec<u8>>,
) -> bool {
    items.iter().any(|item| {
        if item.keyword_is("is_in_tech_sharing_group") {
            return true;
        }
        let key = item.key.as_deref().unwrap_or_default();
        if key.contains(&b'$') {
            return true;
        }
        if item
            .value
            .items()
            .is_some_and(|body| reaches_sharing(body, templates, visited))
        {
            return true;
        }
        let folded = key.to_ascii_lowercase();
        if !visited.insert(folded.clone()) {
            return false;
        }
        templates
            .templates
            .get(&folded)
            .and_then(|entry| entry.body.as_ref().ok())
            .is_some_and(|body| reaches_sharing(body, templates, visited))
    })
}

pub(super) fn read(layers: &ResolvedLayers, language: &str) -> io::Result<serde_json::Value> {
    let language = language.strip_prefix("l_").unwrap_or(language);
    let modes = content_reader_core::scripted_map_modes::read(layers)?;
    let templates = if modes.modes.is_empty() {
        Default::default()
    } else {
        content_reader_core::scripted_triggers::read(layers)?
    };
    let mut sharing_roster = false;
    let mut all_controllers = false;
    let mut controller_states = BTreeSet::new();
    for mode in &modes.modes {
        for layer in [&mode.bottom, &mode.top] {
            sharing_roster |= reaches_sharing(&layer.color, &templates, &mut BTreeSet::new());
            if let Some(targets) = &layer.targets {
                sharing_roster |= reaches_sharing(targets, &templates, &mut BTreeSet::new());
            }
            if layer.kind != LayerType::StateController {
                continue;
            }
            let Some(block) = layer.targets.as_ref().filter(|block| !block.is_empty()) else {
                all_controllers = true;
                continue;
            };
            let targets = content_reader_core::country_scorers::script_targets(block);
            if !targets.arrays.is_empty() {
                all_controllers = true;
            }
            for target in targets.targets {
                match std::str::from_utf8(&target)
                    .ok()
                    .and_then(|text| text.parse::<u32>().ok())
                {
                    Some(id) if id > 0 => {
                        controller_states.insert(id);
                    }
                    Some(0) => {}
                    _ => all_controllers = true,
                }
            }
        }
    }
    let mut victory_point = false;
    if !modes.modes.is_empty() {
        let localisation = content_reader_localisation::localisation::read(layers, language)?;
        let scripted = content_reader_scripted_localisation::scripted_localisation::names(layers)?;
        let mut pending = modes
            .modes
            .iter()
            .flat_map(|mode| {
                [
                    format!("{}_tooltip", mode.name),
                    format!("{}_tooltip_delayed", mode.name),
                ]
            })
            .collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(key) = pending.pop() {
            if !visited.insert(key.clone()) {
                continue;
            }
            if let Some(text) = localisation.get(&key) {
                let expanded = localisation.localise(key.as_bytes()).ok();
                let text = expanded.as_deref().unwrap_or(text);
                let mut rest = text;
                while let Some(at) = rest.find('$') {
                    rest = &rest[at + 1..];
                    let Some(end) = rest.find('$') else {
                        break;
                    };
                    pending.push(rest[..end].split('|').next().unwrap_or_default().to_owned());
                    rest = &rest[end + 1..];
                }
                let mut rest = text;
                while let Some(at) = rest.find('[') {
                    rest = &rest[at + 1..];
                    let Some(end) = rest.find(']') else {
                        break;
                    };
                    let property = rest[..end]
                        .split('|')
                        .next()
                        .unwrap_or_default()
                        .rsplit('.')
                        .next()
                        .unwrap_or_default();
                    if property.eq_ignore_ascii_case("GetCapitalVictoryPointName") {
                        victory_point = true;
                    }
                    pending.push(property.to_owned());
                    rest = &rest[end + 1..];
                }
            }
            if let Some(entry) = scripted.entries.get(key.as_bytes()) {
                for text in &entry.texts {
                    sharing_roster |=
                        reaches_sharing(&text.trigger, &templates, &mut BTreeSet::new());
                    use content_reader_scripted_localisation::scripted_localisation::TextValue;
                    match &text.value {
                        Some(TextValue::Key(key)) => {
                            pending.push(String::from_utf8_lossy(key).into_owned())
                        }
                        Some(TextValue::RandomList(list)) => pending.extend(
                            list.members
                                .iter()
                                .map(|member| String::from_utf8_lossy(&member.key).into_owned()),
                        ),
                        None => {}
                    }
                }
            }
        }
    }
    Ok(serde_json::json!({
        "victory_point": victory_point,
        "sharing_roster": sharing_roster,
        "all_controllers": all_controllers,
        "controller_states": controller_states.into_iter().collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_SCRIPTED_CONTENT_PROFILES"]
    fn scripted_map_demand_reads_installed_content_stacks() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let game_dir: std::path::PathBuf = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap().into();
        let profiles = std::env::var_os("BATTLE_PLAN_SCRIPTED_CONTENT_PROFILES").or_else(|| std::env::var_os("HOI4_SCRIPTED_CONTENT_PROFILES")).unwrap();
        let mut modes_seen = 0;
        for user_dir in std::env::split_paths(&profiles) {
            let content =
                content_reader_core::content::gather(&content_reader_core::content::GatherOptions {
                    game_dir: game_dir.clone(),
                    cache_dir: user_dir.join("content_cache"),
                    user_dir,
                    dlc_order: None,
                })
                .unwrap();
            let modes = content_reader_core::scripted_map_modes::read(&content.layers).unwrap();
            let demand = read(&content.layers, &content.language).unwrap();
            modes_seen += modes.modes.len();
            eprintln!(
                "{} mods, {} modes, {} mode diagnostics; demand {demand}",
                content.mods.len(),
                modes.modes.len(),
                modes.refused.len()
            );
        }
        assert!(
            modes_seen > 0,
            "the installed profiles must reach scripted modes"
        );
    }

    #[test]
    fn scripted_map_demand_empty_controller_targets_need_every_state_cache() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root =
            std::env::temp_dir().join(format!("scripted-empty-targets-{}", std::process::id()));
        std::fs::create_dir_all(root.join("common/map_modes")).unwrap();
        for selector in ["", "targets = {}"] {
            std::fs::write(root.join("common/map_modes/mod.txt"), format!(
                "scripted_map_modes = {{ mod_mode = {{ top = {{ type = state_controller {selector} }} }} }}"
            )).unwrap();
            let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
            let demand = read(&layers, "l_english").unwrap();
            assert_eq!(demand["all_controllers"], true, "{selector}");
            assert_eq!(demand["controller_states"], serde_json::json!([]));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scripted_map_demand_follows_hover_alternatives_and_limits_static_state_targets() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root = std::env::temp_dir().join(format!("scripted-map-demand-{}", std::process::id()));
        let files = [
            (
                "common/map_modes/mod.txt",
                "scripted_map_modes={mod_mode={top={type=state_controller targets={targets={8 3 8}} color={sharing_wrapper=yes}}}}",
            ),
            (
                "localisation/mod_l_english.yml",
                "\u{feff}l_english:\n mod_mode_tooltip:0 \"[State.GetModLocation]\"\n selected_city:0 \"[State.GetCapitalVictoryPointName]\"\n unused:0 \"[State.GetCapitalVictoryPointName]\"\n",
            ),
            (
                "common/scripted_localisation/mod.txt",
                "defined_text={name=GetModLocation text={localization_key=selected_city}}",
            ),
            (
                "common/scripted_triggers/mod.txt",
                "sharing_wrapper={is_in_tech_sharing_group=mod_group}",
            ),
        ];
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let demand = read(&layers, "l_english").unwrap();
        assert_eq!(demand["victory_point"], true);
        assert_eq!(demand["sharing_roster"], true);
        assert_eq!(demand["all_controllers"], false);
        assert_eq!(demand["controller_states"], serde_json::json!([3, 8]));
        std::fs::write(
            root.join("common/map_modes/mod.txt"),
            "scripted_map_modes={mod_mode={top={type=state_controller targets={targets={8 3 8}}}}}",
        )
        .unwrap();
        std::fs::write(root.join("localisation/mod_l_english.yml"),
            "\u{feff}l_english:\n mod_mode_tooltip:0 \"Plain\"\n unused:0 \"[State.GetCapitalVictoryPointName]\"\n").unwrap();
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        assert_eq!(read(&layers, "english").unwrap()["victory_point"], false);
        assert_eq!(read(&layers, "english").unwrap()["sharing_roster"], false);
        std::fs::write(root.join("localisation/mod_l_english.yml"),
            "\u{feff}l_english:\n mod_mode_tooltip:0 \"[$city_property$]\"\n city_property:0 \"GetCapitalVictoryPointName\"\n").unwrap();
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        assert_eq!(read(&layers, "english").unwrap()["victory_point"], true);
        std::fs::remove_dir_all(root).unwrap();
    }
}
