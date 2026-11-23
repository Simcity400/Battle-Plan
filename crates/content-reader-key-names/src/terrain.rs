
use std::{collections::BTreeMap, io};

use content_reader_layers::ResolvedLayers;
use content_reader_parser::{crt::{Scanned, scan_i32}, fields::invalid, parser::Item};
use content_reader_script::{modifiers, native::{self, Shape}};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainCategory {
    pub name: Vec<u8>,
    pub movement_cost: i64,
    pub combat_width: i64,
    pub combat_support_width: i64,
    pub supply_flow_penalty_factor: i64,
    pub modifiers: modifiers::Modifiers,
    pub classes: Vec<(Vec<u8>, modifiers::Modifiers)>,
    pub minimum_seazone_dominance: i64,
    pub common_adjuster: TerrainAdjuster,
    pub class_adjusters: Vec<TerrainAdjuster>,
    pub color: Result<[u32; 4], String>,
}

impl TerrainCategory {
    pub fn color(&self) -> Result<[f32; 4], String> {
        self.color
            .as_ref()
            .map(|bits| bits.map(f32::from_bits))
            .map_err(Clone::clone)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerrainAdjuster {
    pub movement: i64,
    pub attack: i64,
    pub defence: i64,
}

impl TerrainAdjuster {
    fn apply(&mut self, fields: &[Item]) {
        for field in fields {
            let slot = match field.keyword().map(|key| key.folded()).as_deref() {
                Some(b"movement") => &mut self.movement,
                Some(b"attack") => &mut self.attack,
                Some(b"defence") => &mut self.defence,
                _ => continue,
            };
            if let Some(value) = modifiers::read_fixed5(field) {
                *slot = value;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainType {
    pub name: String,
    pub category: Option<usize>,
    pub atlas_index: u32,
    pub perm_snow: bool,
    pub spawn_city: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerrainDefinitions {
    pub categories: Vec<TerrainCategory>,
    pub palette: BTreeMap<u8, TerrainType>,
    pub skipped: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefaultMap {
    pub definitions: String,
    pub provinces: String,
    pub terrain: String,
    pub rivers: String,
    pub heightmap: String,
    pub tree_definition: String,
    pub tree_indices: Vec<u8>,
    pub other: Vec<(String, String)>,
}

impl TerrainDefinitions {
    pub fn category(&self, name: &[u8]) -> Option<usize> {
        self.categories
            .iter()
            .position(|category| category.name.eq_ignore_ascii_case(name))
    }

    pub fn naval_category(&self, naval_terrain: Option<&str>) -> Option<usize> {
        naval_terrain
            .and_then(|name| self.category(name.as_bytes()))
            .or_else(|| self.category(b"ocean"))
    }

    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut undecided = Vec::new();
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            None,
            &mut |node, field| match node {
                Node::Top => match field.keyword().as_deref() {
                    Some(b"terrain") => Shape::Object(Node::Terrains),
                    Some(b"categories") => Shape::Object(Node::Categories),
                    _ => Shape::Skip,
                },
                Node::Categories => Shape::Object(Node::Category),
                Node::Terrains => Shape::Object(Node::Graphics),
                Node::Category => category_field(field.keyword().as_deref(), field.opens())
                    .unwrap_or_else(|| {
                        modifiers::shape(
                            modifiers::Node::Modifier,
                            field,
                            modifiers::UNKNOWN,
                            &mut undecided,
                        )
                        .map(Node::Modifier)
                    }),
                Node::Class if field.keyword_is("units") => Shape::Object(Node::Adjuster),
                Node::Class => modifiers::shape(
                    modifiers::Node::Modifier,
                    field,
                    modifiers::UNKNOWN,
                    &mut undecided,
                )
                .map(Node::Modifier),
                Node::Adjuster => match field.keyword().as_deref() {
                    Some(b"attack" | b"defence" | b"movement") => Shape::One,
                    _ => Shape::Skip,
                },
                Node::Graphics => match field.keyword().as_deref() {
                    Some(b"color") => Shape::List,
                    Some(b"type" | b"texture" | b"perm_snow" | b"spawn_city") => Shape::One,
                    _ => Shape::Skip,
                },
                Node::Statements if field.fresh && !field.opens() => Shape::One,
                Node::Statements => {
                    undecided.push(modifiers::Undecided::untraced(
                        field,
                        "the terrain building-level statement reader",
                    ));
                    Shape::Skip
                }
                Node::Modifier(modifier) => {
                    modifiers::shape(modifier, field, modifiers::UNKNOWN, &mut undecided)
                        .map(Node::Modifier)
                }
            },
        );
        if let Some(first) = undecided.first() {
            return Err(invalid(format!("terrain: {first}")));
        }
        if let Some(unsettled) = &read.unsettled {
            return Err(invalid(format!(
                "terrain: {unsettled}; the rest of the file is unknown"
            )));
        }
        self.skipped
            .extend(read.misreads.iter().map(ToString::to_string));
        for section in &read.items {
            let entries = section.value.items().unwrap_or_default();
            match section.keyword().map(|keyword| keyword.folded()).as_deref() {
                Some(b"categories") => {
                    for entry in entries {
                        self.read_category(entry);
                    }
                }
                Some(b"terrain") => {
                    for entry in entries {
                        let name =
                            String::from_utf8_lossy(entry.key.as_deref().unwrap_or_default())
                                .into_owned();
                        let what = format!("terrain {name}");
                        let mut errors = Vec::new();
                        let fields = entry.value.items().unwrap_or_default();
                        match self.graphics(name, fields, &mut errors) {
                            Ok(()) => self
                                .skipped
                                .extend(errors.into_iter().map(|error| format!("{what}: {error}"))),
                            Err(error) => self.skipped.push(format!("{what} skipped: {error}")),
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn read_category(&mut self, entry: &Item) {
        let name = entry.key.clone().unwrap_or_default();
        let what = format!("terrain category {}", String::from_utf8_lossy(&name));
        let mut values = [0_i64; 4];
        let mut common = modifiers::Modifiers::default();
        let mut classes = Vec::new();
        let mut minimum_seazone_dominance = 0;
        let mut common_adjuster = TerrainAdjuster::default();
        let mut class_adjusters = Vec::new();
        let mut color = content_reader_script::color_reads::ColorReads::default();
        for field in entry.value.items().unwrap_or_default() {
            let keyword = field.keyword().map(|keyword| keyword.folded());
            if keyword.as_deref() == Some(b"color") {
                color.read(field);
                continue;
            }
            if keyword.as_deref() == Some(b"minimum_seazone_dominance") {
                if let Some(value) = modifiers::read_fixed5(field) {
                    minimum_seazone_dominance = value;
                }
                continue;
            }
            if keyword.as_deref() == Some(b"units") {
                common_adjuster.apply(field.value.items().unwrap_or_default());
                continue;
            }
            let slot = match category_field(keyword.as_deref(), field.value.items().is_some()) {
                Some(Shape::One) => match keyword.as_deref() {
                    Some(b"movement_cost") => 0,
                    Some(b"combat_width") => 1,
                    Some(b"combat_support_width") => 2,
                    Some(b"supply_flow_penalty_factor") => 3,
                    _ => continue,
                },
                Some(Shape::Object(Node::Class)) => {
                    let mut modifiers = modifiers::Modifiers::default();
                    let mut adjuster = TerrainAdjuster::default();
                    for member in field.value.items().unwrap_or_default() {
                        if member.keyword_is("units") {
                            adjuster.apply(member.value.items().unwrap_or_default());
                        } else {
                            modifiers.read(member);
                        }
                    }
                    classes.push((field.key.clone().unwrap_or_default(), modifiers));
                    class_adjusters.push(adjuster);
                    continue;
                }
                None => {
                    common.read(field);
                    continue;
                }
                _ => continue,
            };
            let key = String::from_utf8_lossy(field.key.as_deref().unwrap_or_default());
            let text = field.value.scalar().unwrap_or_default();
            match content_reader_parser::fixed5::script_fixed5(text) {
                Some(value) => values[slot] = value,
                None => self.skipped.push(format!(
                    "{what}: {key} {} does not convert to a number, so the game logs it and keeps {}",
                    String::from_utf8_lossy(text),
                    content_reader_parser::fixed5::fixed5_to_string(values[slot], -1)
                )),
            }
            if slot == 0 {
                values[0] = values[0].max(1_000);
            }
        }
        self.categories.push(TerrainCategory {
            name,
            movement_cost: values[0],
            combat_width: values[1],
            combat_support_width: values[2],
            supply_flow_penalty_factor: values[3],
            modifiers: common,
            classes,
            minimum_seazone_dominance,
            common_adjuster,
            class_adjusters,
            color: color
                .color()
                .map(|color| color.map(f32::to_bits))
                .map_err(|cause| format!("{what}: {cause}")),
        });
    }

    fn graphics(
        &mut self,
        name: String,
        fields: &[Item],
        errors: &mut Vec<String>,
    ) -> io::Result<()> {
        let mut entry = TerrainType {
            name,
            category: None,
            atlas_index: 0,
            perm_snow: false,
            spawn_city: false,
        };
        let mut indices = Vec::new();
        let integer = |item: &Item| scan_i32(item.value.scalar().unwrap_or_default());
        let flag = |field: &Item| field.value.scalar().and_then(content_reader_parser::parser::read_bool);
        for field in fields {
            match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                Some(b"type") => {
                    entry.category = self.category(field.value.scalar().unwrap_or_default());
                    if entry.category.is_none() {
                        errors.push(format!(
                            "type {} names no category read before it",
                            String::from_utf8_lossy(field.value.scalar().unwrap_or_default())
                        ));
                    }
                }
                Some(b"color") => {
                    for item in field.value.items().unwrap_or_default() {
                        match integer(item) {
                            Scanned::Stored(index) if index < 0 => {
                                return Err(invalid(format!("color index {index} is negative")));
                            }
                            Scanned::Stored(index) => indices.push(index),
                            Scanned::End => indices.push(0),
                            Scanned::NoMatch => {
                                errors.push("a color index is not an i32; it reads as 0".into());
                                indices.push(0);
                            }
                        }
                    }
                }
                Some(b"texture") => match integer(field) {
                    Scanned::Stored(slot) => {
                        entry.atlas_index = slot as u32;
                    }
                    Scanned::End => {}
                    Scanned::NoMatch => errors.push("texture is not an i32".into()),
                },
                Some(b"perm_snow") => match flag(field) {
                    Some(value) => entry.perm_snow = value,
                    None => errors.push("perm_snow is not yes or no".into()),
                },
                Some(b"spawn_city") => match flag(field) {
                    Some(value) => entry.spawn_city = value,
                    None => errors.push("spawn_city is not yes or no".into()),
                },
                _ => {}
            }
        }
        for index in indices {
            if let Ok(index) = u8::try_from(index) {
                self.palette.insert(index, entry.clone());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Categories,
    Terrains,
    Category,
    Class,
    Adjuster,
    Graphics,
    Statements,
    Modifier(modifiers::Node),
}

fn category_field(keyword: Option<&[u8]>, opens: bool) -> Option<Shape<Node>> {
    Some(match keyword {
        Some(
            b"movement_cost"
            | b"combat_width"
            | b"combat_support_width"
            | b"minimum_seazone_dominance"
            | b"ai_terrain_importance_factor"
            | b"match_value"
            | b"supply_flow_penalty_factor"
            | b"is_water"
            | b"inland_sea"
            | b"naval_terrain"
            | b"worth"
            | b"sound_type",
        ) => Shape::One,
        Some(b"color") => Shape::Color,
        Some(b"units") => Shape::Object(Node::Adjuster),
        Some(b"buildings_max_level") => Shape::Object(Node::Statements),
        _ if opens => Shape::Object(Node::Class),
        _ => return None,
    })
}

pub fn parse_definitions(input: &[u8]) -> io::Result<TerrainDefinitions> {
    let mut definitions = TerrainDefinitions::default();
    definitions.append(input)?;
    if definitions.categories.is_empty() {
        return Err(invalid("the file declares no terrain category"));
    }
    Ok(definitions)
}

pub fn parse_default_map(input: &[u8]) -> io::Result<DefaultMap> {
    let document = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(()),
        None,
        &mut |(), field| match field.keyword().as_deref() {
            Some(b"tree") => Shape::List,
            Some(
                b"definitions" | b"provinces" | b"positions" | b"terrain" | b"rivers"
                | b"heightmap" | b"tree_definition" | b"continent" | b"adjacencies"
                | b"adjacency_rules" | b"ambient_object" | b"seasons",
            ) => Shape::One,
            _ => Shape::Skip,
        },
    );
    let items = &document.items;
    let named = |key: &str| -> io::Result<String> {
        items
            .iter()
            .rfind(|item| item.keyword_is(key))
            .ok_or_else(|| invalid(format!("default.map has no {key}")))?
            .value
            .c_str()
            .map(str::to_owned)
            .map_err(|error| invalid(format!("default.map {key}: {error}")))
    };
    let mut tree_indices = Vec::new();
    for item in items.iter().filter(|item| item.keyword_is("tree")) {
        for entry in item.value.items().unwrap_or_default() {
            let index = match scan_i32(entry.value.scalar().unwrap_or_default()) {
                Scanned::Stored(index) => index,
                Scanned::End | Scanned::NoMatch => 0,
            };
            tree_indices.push(u8::try_from(index).map_err(|_| {
                invalid(format!(
                    "default.map tree palette index {index} cannot be represented by the bitmap"
                ))
            })?);
        }
    }
    let mut map = DefaultMap {
        definitions: named("definitions")?,
        provinces: named("provinces")?,
        terrain: named("terrain")?,
        rivers: named("rivers")?,
        heightmap: named("heightmap")?,
        tree_definition: named("tree_definition")?,
        tree_indices,
        other: Vec::new(),
    };
    for item in items {
        let Some(keyword) = item.keyword() else {
            continue;
        };
        let folded = keyword.folded();
        let key =
            std::str::from_utf8(&folded).map_err(|_| invalid("default.map key is not UTF-8"))?;
        if matches!(
            key,
            "definitions"
                | "provinces"
                | "terrain"
                | "rivers"
                | "heightmap"
                | "tree_definition"
                | "tree"
        ) {
            continue;
        }
        if matches!(
            key,
            "positions"
                | "continent"
                | "adjacencies"
                | "adjacency_rules"
                | "ambient_object"
                | "seasons"
        ) {
            let value = item
                .value
                .c_str()
                .map_err(|error| invalid(format!("default.map {key}: {error}")))?;
            map.other.push((key.to_owned(), value.to_owned()));
        }
    }
    Ok(map)
}

#[cfg(test)]
#[test]
fn default_map_replays_keyword_and_array_readers() {
    content_reader_script::test_game::enter();
    let source = br#"
        @path = "p.bmp"
        definitions=d.csv PROVINCES=@path terrain=t.bmp rivers=r.bmp
        heightmap=h.bmp tree_definition=trees.bmp
        ignored={ @path=ignored provinces=wrong }
        "provinces"=wrong
        tree={ 0x3 "010" bad } tree={4}
    "#;
    let map = parse_default_map(source).unwrap();
    assert_eq!(map.provinces, "p.bmp");
    assert_eq!(map.tree_indices, [3, 8, 0, 4]);
    assert!(map.other.is_empty());
    let ended = parse_default_map(&[source.as_slice(), b"} provinces=wrong"].concat()).unwrap();
    assert_eq!(ended, map);
    let error = parse_default_map(&[source.as_slice(), b"tree={256}"].concat()).unwrap_err();
    assert!(error.to_string().contains("tree palette index 256"));
}

const DEFINITIONS_DIRECTORY: &str = "common/terrain";

pub fn definition_paths(
    layers: &ResolvedLayers,
) -> io::Result<Vec<content_reader_layers::script_files::ScriptPath>> {
    layers.directory_files(DEFINITIONS_DIRECTORY, ".txt")
}

pub fn read_definitions(layers: &ResolvedLayers) -> io::Result<TerrainDefinitions> {
    layers.recorded_product("terrain database", 1, &[], read_terrain_definitions)
        .map(|definitions| definitions.as_ref().clone())
}

fn read_terrain_definitions(layers: &ResolvedLayers) -> io::Result<TerrainDefinitions> {
    let files = definition_paths(layers)?;
    if files.is_empty() {
        return Err(invalid("content has no common/terrain/*.txt file"));
    }
    let mut definitions = TerrainDefinitions::default();
    for path in files {
        let skipped = definitions.skipped.len();
        definitions
            .append(&path.read(layers)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        for reason in &mut definitions.skipped[skipped..] {
            *reason = format!("{path}: {reason}");
        }
    }
    if definitions.categories.is_empty() {
        return Err(invalid("no common/terrain file declares a category"));
    }
    Ok(definitions)
}

pub fn read(layers: &ResolvedLayers) -> io::Result<(TerrainDefinitions, DefaultMap)> {
    Ok((read_definitions(layers)?, read_default_map(layers)?))
}

impl DefaultMap {
    pub fn file(&self, key: &str) -> io::Result<String> {
        let name = match key {
            "definitions" => Some(&self.definitions),
            "provinces" => Some(&self.provinces),
            "terrain" => Some(&self.terrain),
            "rivers" => Some(&self.rivers),
            "heightmap" => Some(&self.heightmap),
            "tree_definition" => Some(&self.tree_definition),
            _ => self
                .other
                .iter()
                .rev()
                .find(|(other, _)| other == key)
                .map(|(_, value)| value),
        };
        name.map(|name| format!("map/{name}"))
            .ok_or_else(|| invalid(format!("map/default.map names no {key}")))
    }
}

pub fn read_default_map(layers: &ResolvedLayers) -> io::Result<DefaultMap> {
    layers.recorded_product("default map", 1, &[], |layers| {
        parse_default_map(&layers.read("map/default.map")?)
            .map_err(|error| invalid(format!("map/default.map: {error}")))
    }).map(|map| map.as_ref().clone())
}

#[cfg(test)]
mod tests;
