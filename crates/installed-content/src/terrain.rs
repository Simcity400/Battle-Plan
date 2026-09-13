//! Terrain categories, the `terrain.bmp` palette and `map/default.map`.
//!
//! The `terrain` section maps palette indices of `terrain.bmp` onto a category
//! and an atlas texture slot. Its entry names repeat in the shipped file
//! (`desert` appears three times), so the palette is keyed by index and a
//! repeated index, not a repeated name, is the error.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{block, boolean, integer, invalid, key_str, last, require, scalar_list, text},
    image::{IndexedImage, Rgba8Image, bmp},
    map::parse_color,
    parser,
};

/// One `categories` entry: how a terrain behaves and how the map legend paints it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerrainCategory {
    pub name: String,
    pub color: [u8; 3],
    pub is_water: bool,
    pub naval_terrain: bool,
}

/// One `terrain` entry: a palette index and the atlas slot it samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainType {
    /// Authored name; the shipped file repeats names across entries.
    pub name: String,
    /// Category key into [`TerrainDefinitions::categories`].
    pub category: String,
    /// `texture = N`, the slot inside `atlas0..2.dds`. `255` means none.
    pub atlas_index: u32,
    pub perm_snow: bool,
    pub spawn_city: bool,
}

/// Everything `common/terrain/00_terrain.txt` declares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerrainDefinitions {
    pub categories: BTreeMap<String, TerrainCategory>,
    /// `terrain.bmp` palette index to the terrain that index selects.
    pub palette: BTreeMap<u8, TerrainType>,
}

/// The file names and tree indices `map/default.map` declares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefaultMap {
    pub definitions: String,
    pub provinces: String,
    pub terrain: String,
    pub rivers: String,
    pub heightmap: String,
    pub tree_definition: String,
    /// `tree = { 3 4 7 10 }`: palette indices of `trees.bmp` that count as trees.
    pub tree_indices: Vec<u8>,
    /// Every other key of the file, in source order, as authored text.
    pub other: Vec<(String, String)>,
}

/// The decoded bitmaps `map/default.map` names, plus `world_normal.bmp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapImages {
    pub terrain: IndexedImage,
    pub heightmap: IndexedImage,
    pub rivers: IndexedImage,
    pub trees: IndexedImage,
    pub world_normal: Rgba8Image,
}

/// Parses `common/terrain/00_terrain.txt`.
pub fn parse_definitions(input: &[u8]) -> io::Result<TerrainDefinitions> {
    let document = parser::parse(input)?;
    let mut definitions = TerrainDefinitions::default();
    for value in crate::fields::all(&document.items, b"categories") {
        for item in block(value, "terrain categories")? {
            let name = key_str(item, "terrain categories")?.to_owned();
            let items = block(&item.value, "terrain category")?;
            let category = TerrainCategory {
                color: parse_color(require(items, "color", &format!("category {name}"))?)?,
                is_water: match last(items, b"is_water") {
                    Some(value) => boolean(value, "is_water")?,
                    None => false,
                },
                naval_terrain: match last(items, b"naval_terrain") {
                    Some(value) => boolean(value, "naval_terrain")?,
                    None => false,
                },
                name: name.clone(),
            };
            definitions.categories.insert(name, category);
        }
    }
    for value in crate::fields::all(&document.items, b"terrain") {
        for item in block(value, "graphical terrain")? {
            let name = key_str(item, "graphical terrain")?.to_owned();
            let items = block(&item.value, "graphical terrain entry")?;
            let what = format!("terrain {name}");
            let category = text(require(items, "type", &what)?, "terrain type")?.to_owned();
            if !definitions.categories.contains_key(&category) {
                return Err(invalid(format!(
                    "{what} names an undeclared category {category:?}"
                )));
            }
            let indices: Vec<u8> =
                scalar_list(require(items, "color", &what)?, "terrain palette index")?;
            if indices.is_empty() {
                return Err(invalid(format!("{what} lists no palette index")));
            }
            let entry = TerrainType {
                name,
                category,
                atlas_index: integer(require(items, "texture", &what)?, "terrain texture")?,
                perm_snow: match last(items, b"perm_snow") {
                    Some(value) => boolean(value, "perm_snow")?,
                    None => false,
                },
                spawn_city: match last(items, b"spawn_city") {
                    Some(value) => boolean(value, "spawn_city")?,
                    None => false,
                },
            };
            for index in indices {
                if let Some(previous) = definitions.palette.insert(index, entry.clone()) {
                    return Err(invalid(format!(
                        "terrain palette index {index} is claimed by both {:?} and {:?}",
                        previous.name, entry.name
                    )));
                }
            }
        }
    }
    if definitions.categories.is_empty() {
        return Err(invalid("00_terrain.txt declares no categories"));
    }
    Ok(definitions)
}

/// Parses `map/default.map`.
pub fn parse_default_map(input: &[u8]) -> io::Result<DefaultMap> {
    let document = parser::parse(input)?;
    let items = &document.items;
    let named = |key: &str| -> io::Result<String> {
        Ok(text(require(items, key, "default.map")?, key)?.to_owned())
    };
    let mut map = DefaultMap {
        definitions: named("definitions")?,
        provinces: named("provinces")?,
        terrain: named("terrain")?,
        rivers: named("rivers")?,
        heightmap: named("heightmap")?,
        tree_definition: named("tree_definition")?,
        tree_indices: match last(items, b"tree") {
            Some(value) => scalar_list(value, "default.map tree indices")?,
            None => Vec::new(),
        },
        other: Vec::new(),
    };
    for item in items {
        let key = key_str(item, "default.map")?;
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
        if let Some(scalar) = item.value.scalar_str() {
            map.other.push((key.to_owned(), scalar.to_owned()));
        }
    }
    Ok(map)
}

/// Reads `common/terrain/*.txt` and `map/default.map` from the layers.
pub fn read(layers: &ResolvedLayers) -> io::Result<(TerrainDefinitions, DefaultMap)> {
    let mut paths = layers
        .paths()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with("common/terrain/") && lower.ends_with(".txt")
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    paths.sort();
    let mut definitions = TerrainDefinitions::default();
    for path in paths {
        let parsed = parse_definitions(&layers.read(&path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        definitions.categories.extend(parsed.categories);
        definitions.palette.extend(parsed.palette);
    }
    if definitions.categories.is_empty() {
        return Err(invalid("no common/terrain file declares any category"));
    }
    let default_map = parse_default_map(&layers.read("map/default.map")?)
        .map_err(|error| invalid(format!("map/default.map: {error}")))?;
    Ok((definitions, default_map))
}

/// Decodes the bitmaps `map/default.map` names, plus `map/world_normal.bmp`.
pub fn read_map_images(layers: &ResolvedLayers, map: &DefaultMap) -> io::Result<MapImages> {
    let indexed = |name: &str| -> io::Result<IndexedImage> {
        let path = format!("map/{name}");
        bmp::decode_indexed(&layers.read(&path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))
    };
    Ok(MapImages {
        terrain: indexed(&map.terrain)?,
        heightmap: indexed(&map.heightmap)?,
        rivers: indexed(&map.rivers)?,
        trees: indexed(&map.tree_definition)?,
        world_normal: bmp::decode_rgb(&layers.read("map/world_normal.bmp")?)
            .map_err(|error| invalid(format!("map/world_normal.bmp: {error}")))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERRAIN: &[u8] = br#"
categories = {
    unknown = { color = { 255 0 0 } }
    ocean = { naval_terrain = yes color = { 40 83 176 } is_water = yes }
    plains = { color = { 1 2 3 } }
    mountain = { color = { 4 5 6 } }
    desert = { color = { 7 8 9 } }
    lakes = { color = { 58 91 255 } is_water = yes }
}
terrain = {
    terrain_0 = { type = plains color = { 0 } texture = 1 }
    desert = { type = mountain color = { 11 } texture = 11 }
    desert = { type = desert color = { 12 } texture = 8 }
    forest_14 = { type = lakes color = { 14 } texture = 255 }
    snow_16 = { type = mountain color = { 16 17 } texture = 11 perm_snow = yes }
    city = { type = plains color = { 13 } texture = 10 spawn_city = yes }
}
"#;

    #[test]
    fn repeated_terrain_names_are_kept_apart_by_their_palette_index() {
        let definitions = parse_definitions(TERRAIN).unwrap();
        assert_eq!(definitions.categories.len(), 6);
        assert!(definitions.categories["ocean"].is_water);
        assert!(definitions.categories["ocean"].naval_terrain);
        assert!(!definitions.categories["lakes"].naval_terrain);
        assert_eq!(definitions.categories["ocean"].color, [40, 83, 176]);
        assert!(!definitions.categories["plains"].is_water);
        // Both `desert` entries survive under their own index.
        assert_eq!(definitions.palette[&11].category, "mountain");
        assert_eq!(definitions.palette[&12].category, "desert");
        assert_eq!(definitions.palette[&11].name, "desert");
        assert_eq!(definitions.palette[&14].atlas_index, 255);
        // One entry may claim several indices.
        assert_eq!(definitions.palette[&16], definitions.palette[&17]);
        assert!(definitions.palette[&16].perm_snow);
        assert!(definitions.palette[&13].spawn_city);
        assert!(!definitions.palette[&0].perm_snow);
    }

    #[test]
    fn terrain_files_that_contradict_themselves_are_named() {
        for (input, message) in [
            (
                br#"categories = { a = { color = { 1 2 3 } } }
                    terrain = { x = { type = missing color = { 0 } texture = 1 } }"#
                    .as_slice(),
                "undeclared category",
            ),
            (
                br#"categories = { a = { color = { 1 2 3 } } }
                    terrain = { x = { type = a color = { 0 } texture = 1 }
                                y = { type = a color = { 0 } texture = 2 } }"#,
                "index 0 is claimed by both",
            ),
            (
                br#"categories = { a = { color = { 1 2 3 } } }
                    terrain = { x = { type = a color = { } texture = 1 } }"#,
                "lists no palette index",
            ),
            (
                br#"categories = { a = { color = { 1 2 3 } } }
                    terrain = { x = { type = a color = { 0 } } }"#,
                "has no texture",
            ),
            (br#"categories = { a = { } }"#, "has no color"),
            (br#"terrain = { }"#, "declares no categories"),
            (
                br#"categories = { a = { color = { 1 2 3 } is_water = maybe } }"#,
                "not yes or no",
            ),
        ] {
            let error = parse_definitions(input).unwrap_err().to_string();
            assert!(error.contains(message), "{error} lacks {message}");
        }
    }

    #[test]
    fn default_map_names_its_bitmaps_and_tree_indices() {
        let map = parse_default_map(
            br#"definitions = "definition.csv"
                provinces = "provinces.bmp"
                positions = "positions.txt"
                terrain = "terrain.bmp"
                rivers = "rivers.bmp"
                heightmap = "heightmap.bmp"
                tree_definition = "trees.bmp"
                continent = "continent.txt"
                #climate = "climate.txt"
                tree = { 3 4 7 10 }"#,
        )
        .unwrap();
        assert_eq!(map.provinces, "provinces.bmp");
        assert_eq!(map.terrain, "terrain.bmp");
        assert_eq!(map.tree_definition, "trees.bmp");
        assert_eq!(map.tree_indices, vec![3, 4, 7, 10]);
        assert_eq!(
            map.other,
            vec![
                ("positions".into(), "positions.txt".to_owned()),
                ("continent".into(), "continent.txt".to_owned()),
            ]
        );
    }

    #[test]
    fn a_default_map_without_a_required_name_is_refused() {
        let complete = br#"definitions = "d" provinces = "p" terrain = "t" rivers = "r"
             heightmap = "h" tree_definition = "td""#;
        assert!(parse_default_map(complete).unwrap().tree_indices.is_empty());
        for key in [
            "definitions",
            "provinces",
            "terrain",
            "rivers",
            "heightmap",
            "tree_definition",
        ] {
            let broken = String::from_utf8(complete.to_vec())
                .unwrap()
                .replace(key, "removed");
            let error = parse_default_map(broken.as_bytes())
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("default.map has no {key}")),
                "{error}"
            );
        }
        assert!(parse_default_map(br#"tree = { 300 }"#).is_err());
    }
}
