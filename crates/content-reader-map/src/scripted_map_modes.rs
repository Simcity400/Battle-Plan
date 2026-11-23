
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::parser::{self, Item};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerType {
    None,
    Country,
    State,
    StateController,
    Game(GameLayer),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameLayer {
    Country,
    States,
    Diplomacy,
    Players,
    Factions,
    Ideology,
}

impl LayerType {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "none" => Self::None,
            "country" => Self::Country,
            "state" => Self::State,
            "state_controller" => Self::StateController,
            "game_map_mode_country" => Self::Game(GameLayer::Country),
            "game_map_mode_states" => Self::Game(GameLayer::States),
            "game_map_mode_diplomacy" => Self::Game(GameLayer::Diplomacy),
            "game_map_mode_players" => Self::Game(GameLayer::Players),
            "game_map_mode_factions" => Self::Game(GameLayer::Factions),
            "game_map_mode_ideology" => Self::Game(GameLayer::Ideology),
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MapText {
    #[default]
    None,
    Country,
    State,
    Faction,
    Player,
}

impl MapText {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "none" => Self::None,
            "country" => Self::Country,
            "state" => Self::State,
            "faction" => Self::Faction,
            "player" => Self::Player,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub kind: LayerType,
    pub color: Vec<Item>,
    pub targets: Option<Vec<Item>>,
    pub thickness: bool,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            kind: LayerType::None,
            color: Vec::new(),
            targets: None,
            thickness: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapMode {
    pub name: String,
    pub file: script_files::ScriptPath,
    pub top: Layer,
    pub bottom: Layer,
    pub far_text: Option<MapText>,
    pub near_text: Option<MapText>,
    pub update_daily: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapModes {
    pub modes: Vec<MapMode>,
    pub refused: Vec<(script_files::ScriptPath, String)>,
}

impl MapModes {
    pub fn get(&self, name: &str) -> Option<&MapMode> {
        self.modes.iter().find(|mode| mode.name == name)
    }

    pub fn refusal_counts(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for (_, field) in &self.refused {
            match counts.iter_mut().find(|(name, _)| name == field) {
                Some((_, count)) => *count += 1,
                None => counts.push((field.clone(), 1)),
            }
        }
        counts
    }
}

fn scalar(item: &Item) -> Option<String> {
    item.value.c_str().ok().map(str::to_owned)
}

fn read_layer(
    file: &script_files::ScriptPath,
    mode: &str,
    side: &str,
    items: &[Item],
    refused: &mut Vec<(script_files::ScriptPath, String)>,
) -> Layer {
    let mut layer = Layer::default();
    for item in items {
        let key = item.key.as_deref().map(String::from_utf8_lossy);
        match key.as_deref() {
            Some("type") => match scalar(item).as_deref().and_then(LayerType::parse) {
                Some(kind) => layer.kind = kind,
                None => refused.push((
                    file.clone(),
                    format!("{mode}.{side}.type = {}", scalar(item).unwrap_or_default()),
                )),
            },
            Some("color") => layer.color = item.value.items().unwrap_or_default().to_vec(),
            Some("targets") => {
                layer.targets = Some(item.value.items().unwrap_or_default().to_vec())
            }
            Some("thickness") => {
                layer.thickness = item
                    .value
                    .scalar()
                    .and_then(parser::read_bool)
                    .unwrap_or(false)
            }
            other => refused.push((
                file.clone(),
                format!("{mode}.{side}.{}", other.unwrap_or("<anonymous>")),
            )),
        }
    }
    layer
}

pub fn parse_into(
    file: &script_files::ScriptPath,
    input: &[u8],
    modes: &mut MapModes,
) -> io::Result<()> {
    let document = parser::parse(input)?;
    for root in &document.items {
        if !root.keyword_is("scripted_map_modes") {
            modes.refused.push((
                file.clone(),
                String::from_utf8_lossy(root.key.as_deref().unwrap_or_default()).into_owned(),
            ));
            continue;
        }
        for entry in root.value.items().unwrap_or_default() {
            let Some(name) = entry.key.as_deref() else {
                modes
                    .refused
                    .push((file.clone(), "<anonymous map mode>".to_owned()));
                continue;
            };
            let name = String::from_utf8_lossy(name).into_owned();
            let mut mode = MapMode {
                name: name.clone(),
                file: file.clone(),
                top: Layer::default(),
                bottom: Layer::default(),
                far_text: None,
                near_text: None,
                update_daily: false,
            };
            for item in entry.value.items().unwrap_or_default() {
                let key = item.key.as_deref().map(String::from_utf8_lossy);
                let items = item.value.items().unwrap_or_default();
                match key.as_deref() {
                    Some("top") => {
                        mode.top = read_layer(file, &name, "top", items, &mut modes.refused)
                    }
                    Some("bottom") => {
                        mode.bottom = read_layer(file, &name, "bottom", items, &mut modes.refused)
                    }
                    Some(text @ ("far_text" | "near_text")) => {
                        match scalar(item).as_deref().and_then(MapText::parse) {
                            Some(value) if text == "far_text" => mode.far_text = Some(value),
                            Some(value) => mode.near_text = Some(value),
                            None => modes.refused.push((
                                file.clone(),
                                format!("{name}.{text} = {}", scalar(item).unwrap_or_default()),
                            )),
                        }
                    }
                    Some("update_daily") => {
                        mode.update_daily = item
                            .value
                            .scalar()
                            .and_then(parser::read_bool)
                            .unwrap_or(false)
                    }
                    other => modes.refused.push((
                        file.clone(),
                        format!("{name}.{}", other.unwrap_or("<anonymous>")),
                    )),
                }
            }
            match modes.modes.iter_mut().find(|known| known.name == name) {
                Some(known) => *known = mode,
                None => modes.modes.push(mode),
            }
        }
    }
    Ok(())
}

pub fn read(layers: &ResolvedLayers) -> io::Result<MapModes> {
    let mut modes = MapModes::default();
    for listed in script_files::paths(layers, "common/map_modes/", Some(".txt"))? {
        let bytes = listed.read(layers)?;
        if let Err(error) = parse_into(&listed, &bytes, &mut modes) {
            modes
                .refused
                .push((listed, format!("unreadable file: {error}")));
        }
    }
    Ok(modes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_layers_and_refusals() {
        let mut modes = MapModes::default();
        parse_into(
            &"common/map_modes/a.txt".into(),
            br#"scripted_map_modes = {
                first = {
                    top = { type = country }
                    far_text = country near_text = state update_daily = yes
                    unknown_switch = yes
                }
                second = { top = { type = rainbow } bottom = { type = none } far_text = sky }
            }"#,
            &mut modes,
        )
        .unwrap();
        parse_into(
            &"common/map_modes/b.txt".into(),
            b"scripted_map_modes = { first = { top = { type = state } } }",
            &mut modes,
        )
        .unwrap();
        assert_eq!(
            modes
                .modes
                .iter()
                .map(|mode| mode.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        let first = modes.get("first").unwrap();
        assert_eq!(first.file, "common/map_modes/b.txt");
        assert_eq!(first.top.kind, LayerType::State);
        assert!(!first.update_daily);
        assert_eq!(modes.get("second").unwrap().top.kind, LayerType::None);
        assert_eq!(
            modes.refusal_counts(),
            [
                ("first.unknown_switch".to_owned(), 1),
                ("second.top.type = rainbow".to_owned(), 1),
                ("second.far_text = sky".to_owned(), 1),
            ]
        );
        let mut layered = MapModes::default();
        parse_into(
            &"x.txt".into(),
            br#"scripted_map_modes = { m = {
                top = { type = country color = { set_temp_variable = { red = 1 } } thickness = yes targets = { target_array = x } }
                bottom = { type = game_map_mode_factions }
                far_text = country near_text = state update_daily = yes } }"#,
            &mut layered,
        )
        .unwrap();
        let mode = layered.get("m").unwrap();
        assert_eq!(mode.top.kind, LayerType::Country);
        assert!(mode.top.thickness && mode.top.targets.is_some() && !mode.top.color.is_empty());
        assert_eq!(mode.bottom.kind, LayerType::Game(GameLayer::Factions));
        assert_eq!(
            (mode.far_text, mode.near_text),
            (Some(MapText::Country), Some(MapText::State))
        );
        assert!(mode.update_daily && layered.refused.is_empty());
    }
}
