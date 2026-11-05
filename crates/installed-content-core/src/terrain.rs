//! The terrain database (`common/terrain/*.txt`) and `map/default.map`.
//!
//! This is the one reader of the terrain database; the movement map
//! ([`crate::movement`]) derives its costs and ordinals from its result.
//!
//! The loader lists the direct `.txt` files under `common/terrain`
//! ([`ResolvedLayers::directory_files`]) and reads each file's top-level keys
//! in source order:
//!
//! - Every `categories` entry appends a category with the current count as
//!   its ordinal; the null terrain has ordinal zero. A redeclared name
//!   appends another entry instead of replacing the first.
//! - Every `terrain` entry resolves `type` immediately against the categories
//!   appended so far: the first case-insensitive match, else the null terrain.
//!   It sees only earlier files and earlier `categories` blocks. An entry
//!   without `color` indices is logged and discarded; otherwise every index
//!   writes the palette, and the last writer wins with a log line.
//!
//! One script reader per file ([`native`]) resolves variables in document
//! order and preserves consumption when fields leave the balanced-tree
//! grammar. Bare keywords fold; quoted keys take the non-keyword branch.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    crt::{Scanned, scan_i32},
    fields::invalid,
    gui::native::resources,
    image::{
        IndexedImage, Rgba8Image, bmp,
        dds::{Texture, TextureFormat},
        native::Decode,
    },
    modifiers,
    parser::{
        Item,
        native::{self, Shape},
    },
};

/// One authored `categories` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainCategory {
    /// The authored key bytes; lookups fold ASCII case ([`TerrainDefinitions::category`]).
    pub name: Vec<u8>,
    /// Authored `movement_cost` as fixed5, zero when absent. Every read
    /// clamps it to 0.01, including failed conversions.
    pub movement_cost: i64,
    /// Authored `combat_width` as fixed5, zero when absent.
    pub combat_width: i64,
    /// Authored `combat_support_width`, the fixed5 width each additional
    /// direction adds; zero when absent.
    pub combat_support_width: i64,
    /// Supply-flow penalty shown by the land terrain hover; starts at zero.
    pub supply_flow_penalty_factor: i64,
    /// Common modifiers and ordered class entries; common values are
    /// added to each class after loading.
    pub modifiers: modifiers::Modifiers,
    pub classes: Vec<(Vec<u8>, modifiers::Modifiers)>,
    /// Minimum sea-zone dominance and common/per-class unit adjustments
    /// used by the naval terrain hover.
    pub minimum_seazone_dominance: i64,
    pub common_adjuster: TerrainAdjuster,
    pub class_adjusters: Vec<TerrainAdjuster>,
    /// The `color` reads replayed in order over the colour object's white start, as
    /// [`f32::to_bits`] ([`Self::color`]).
    pub color: Result<[u32; 4], String>,
}

impl TerrainCategory {
    /// The category's colour: white when it has no `color`, each read applied as the
    /// colour reader does ([`crate::ideologies::ColorReads`]). `Err` names a form the
    /// reader does not convert.
    pub fn color(&self) -> Result<[f32; 4], String> {
        self.color
            .as_ref()
            .map(|bits| bits.map(f32::from_bits))
            .map_err(Clone::clone)
    }
}

/// Three fixed5 unit adjustments read from terrain definitions.
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

/// One kept `terrain` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainType {
    /// Authored name; names may repeat across entries.
    pub name: String,
    /// Position in [`TerrainDefinitions::categories`] of the category `type`
    /// resolved to when the entry was read; `None` is the null terrain.
    pub category: Option<usize>,
    /// Signed `texture` bits; upload uses the low byte, where 255 means none.
    pub atlas_index: u32,
    /// Permanent-snow flag.
    pub perm_snow: bool,
    /// City-spawn predicate flag.
    pub spawn_city: bool,
}

/// The terrain database the native loader builds from every file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerrainDefinitions {
    /// Every category in load order; the one-based native ordinal is the
    /// position plus one.
    pub categories: Vec<TerrainCategory>,
    /// `terrain.bmp` palette index to the terrain entry that wrote it last.
    /// Indices past a byte are never sampled and are not kept.
    pub palette: BTreeMap<u8, TerrainType>,
    /// Logged recovery notes: malformed entries that were skipped and
    /// failed field conversions that retained the previous value.
    pub skipped: Vec<String>,
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

impl TerrainDefinitions {
    /// The first category whose name matches without regard to ASCII case.
    pub fn category(&self, name: &[u8]) -> Option<usize> {
        self.categories
            .iter()
            .position(|category| category.name.eq_ignore_ascii_case(name))
    }

    /// The category a water province reads: its strategic region's `naval_terrain`, or
    /// `ocean` when it has no region, the region names none or the name matches no category.
    /// `None` when that fallback is missing too.
    pub fn naval_category(&self, naval_terrain: Option<&str>) -> Option<usize> {
        naval_terrain
            .and_then(|name| self.category(name.as_bytes()))
            .or_else(|| self.category(b"ocean"))
    }

    /// Reads one more file through the script reader ([`native`]). A block
    /// whose consumption cannot be settled refuses the file by place and reason.
    pub(crate) fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut undecided = Vec::new();
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            None,
            &mut |node, field| match node {
                // terrain and categories use their own loops; other keys,
                // including quoted spellings, report and skip their blocks.
                Node::Top => match field.keyword().as_deref() {
                    Some(b"terrain") => Shape::Object(Node::Terrains),
                    Some(b"categories") => Shape::Object(Node::Categories),
                    _ => Shape::Skip,
                },
                // Each category or terrain entry reads its fields regardless
                // of whether its value opens a block.
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
                // A per-class entry reads units as an adjustment object;
                // other fields use its modifier dispatch.
                Node::Class if field.keyword_is("units") => Shape::Object(Node::Adjuster),
                Node::Class => modifiers::shape(
                    modifiers::Node::Modifier,
                    field,
                    modifiers::UNKNOWN,
                    &mut undecided,
                )
                .map(Node::Modifier),
                // attack, defence and movement take one value each;
                // other keys are reported and skipped.
                Node::Adjuster => match field.keyword().as_deref() {
                    Some(b"attack" | b"defence" | b"movement") => Shape::One,
                    _ => Shape::Skip,
                },
                // color reads an integer list; type takes value text;
                // texture scans an integer; snow and city flags read bools.
                // Other keys are reported and skipped.
                Node::Graphics => match field.keyword().as_deref() {
                    Some(b"color") => Shape::List,
                    Some(b"type" | b"texture" | b"perm_snow" | b"spawn_city") => Shape::One,
                    _ => Shape::Skip,
                },
                // A keyed scalar statement consumes its value. Anonymous
                // values and blocks reach statement forms whose consumption
                // this reader cannot trace.
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
                        // A malformed field is logged and keeps its earlier
                        // value; only a form the game cannot hold skips the
                        // entry.
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

    /// One category, field by field. Widths and costs read fixed5; a value
    /// that does not convert is logged and leaves its field unchanged.
    fn read_category(&mut self, entry: &Item) {
        let name = entry.key.clone().unwrap_or_default();
        let what = format!("terrain category {}", String::from_utf8_lossy(&name));
        let mut values = [0_i64; 4];
        let mut common = modifiers::Modifiers::default();
        let mut classes = Vec::new();
        let mut minimum_seazone_dominance = 0;
        let mut common_adjuster = TerrainAdjuster::default();
        let mut class_adjusters = Vec::new();
        let mut color = crate::ideologies::ColorReads::default();
        for field in entry.value.items().unwrap_or_default() {
            let keyword = field.keyword().map(|keyword| keyword.folded());
            if keyword.as_deref() == Some(b"color") {
                // Each read updates the colour from where the last left it.
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
            match crate::fixed5::script_fixed5(text) {
                Some(value) => values[slot] = value,
                None => self.skipped.push(format!(
                    "{what}: {key} {} does not convert to a number, so the game logs it and keeps {}",
                    String::from_utf8_lossy(text),
                    crate::fixed5::fixed5_to_string(values[slot], -1)
                )),
            }
            // Clamp the cost after every read, including a failed one.
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

    /// One terrain entry's fields in source order, then its palette writes.
    /// `color` appends indices; other fields take their last value.
    ///
    /// A failed field conversion is reported without changing the field;
    /// the entry and its other fields remain, and the note goes to `errors`.
    /// Colour elements start at zero before scanning, so a failed scan keeps
    /// zero. Negative palette indices would be used out of range and refuse the entry.
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
        // Signed integer scans store nothing at EOF without reporting an
        // error; a nonmatching value is reported.
        let integer = |item: &Item| scan_i32(item.value.scalar().unwrap_or_default());
        // Boolean fields accept exactly yes or no.
        let flag = |field: &Item| field.value.scalar().and_then(crate::parser::read_bool);
        for field in fields {
            match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                Some(b"type") => {
                    entry.category = self.category(field.value.scalar().unwrap_or_default());
                    // Report a type lookup that selects the null terrain.
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
        // An entry without colours is discarded before any palette writes.
        for index in indices {
            if let Ok(index) = u8::try_from(index) {
                self.palette.insert(index, entry.clone());
            }
        }
        Ok(())
    }
}

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    /// File-level fields.
    Top,
    /// Category declarations.
    Categories,
    /// Terrain declarations.
    Terrains,
    /// A category's fields.
    Category,
    /// A per-class entry's fields.
    Class,
    /// Unit adjustments.
    Adjuster,
    /// A terrain entry's fields.
    Graphics,
    /// Building-level statements.
    Statements,
    Modifier(modifiers::Node),
}

/// Dispatches each category field by its keyword.
fn category_field(keyword: Option<&[u8]>, opens: bool) -> Option<Shape<Node>> {
    Some(match keyword {
        // Costs, widths, sea dominance, AI importance, match and supply
        // penalties read one fixed5 value. Water and inland-sea flags,
        // naval terrain, worth and sound type also take one value.
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
        // color reads a colour object.
        Some(b"color") => Shape::Color,
        // units reads an adjustment object.
        Some(b"units") => Shape::Object(Node::Adjuster),
        // buildings_max_level reads keyed statements with their operators,
        // passing each value to the category's statement reader.
        Some(b"buildings_max_level") => Shape::Object(Node::Statements),
        // A block under any other key opens a per-class entry; a scalar
        // uses the category's common modifier dispatch.
        _ if opens => Shape::Object(Node::Class),
        _ => return None,
    })
}

// Parses one terrain database file on its own.
pub fn parse_definitions(input: &[u8]) -> io::Result<TerrainDefinitions> {
    let mut definitions = TerrainDefinitions::default();
    definitions.append(input)?;
    if definitions.categories.is_empty() {
        return Err(invalid("the file declares no terrain category"));
    }
    Ok(definitions)
}

/// Parses `map/default.map`.
pub fn parse_default_map(input: &[u8]) -> io::Result<DefaultMap> {
    // Paths take the value as a C string; tree takes an integer list.
    // Unknown keywords are reported and their blocks skipped.
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
        // Append a zero-initialized i32 before scanning each token.
        // Repeated tree fields append rather than replace.
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// The terrain definition files in native load order
/// ([`ResolvedLayers::directory_files`]). The movement costs
/// (`province_graph`) read the same list.
pub fn definition_paths(
    layers: &ResolvedLayers,
) -> io::Result<Vec<crate::script_files::ScriptPath>> {
    layers.directory_files(DEFINITIONS_DIRECTORY, ".txt")
}

/// Reads the terrain database from every `common/terrain/*.txt` file in
/// native load order ([`definition_paths`]).
pub fn read_definitions(layers: &ResolvedLayers) -> io::Result<TerrainDefinitions> {
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

/// Reads the terrain database and `map/default.map`.
pub fn read(layers: &ResolvedLayers) -> io::Result<(TerrainDefinitions, DefaultMap)> {
    Ok((read_definitions(layers)?, read_default_map(layers)?))
}

impl DefaultMap {
    /// The `map/` path `default.map` names for `key`, as the engine opens it.
    /// A repeated key takes its last value.
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

/// Reads and parses `map/default.map`.
pub fn read_default_map(layers: &ResolvedLayers) -> io::Result<DefaultMap> {
    parse_default_map(&layers.read("map/default.map")?)
        .map_err(|error| invalid(format!("map/default.map: {error}")))
}

/// Decodes the bitmaps `map/default.map` names, plus `map/world_normal.bmp`.
pub fn read_map_images(
    layers: &ResolvedLayers,
    map: &DefaultMap,
    decode: Decode,
) -> io::Result<MapImages> {
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
        world_normal: read_world_normal(layers, decode)?,
    })
}

/// `map/world_normal.bmp`, the height-normal texture, follows ordinary
/// texture resolution ([`resources::texture`]): `.dds`, `.tga`, then the
/// requested name. The BMP decoder accepts 24- and 32-bit forms and converts
/// all-opaque 32-bit alpha to 24 bits. The client `force_pow2_textures` option
/// still applies through `decode`; the CPU bitmaps above remain unpadded.
/// A texture the game does not load, or whose format cannot use the 8-bit
/// RGBA upload, is refused by name.
pub fn read_world_normal(layers: &ResolvedLayers, decode: Decode) -> io::Result<Rgba8Image> {
    world_normal(&|path| layers.read_uncovered_path(path), decode)
}

fn world_normal(
    read: &dyn Fn(&[u8]) -> io::Result<Vec<u8>>,
    decode: Decode,
) -> io::Result<Rgba8Image> {
    let texture = resources::texture(b"map/world_normal.bmp", read, decode)?
        .map_err(|why| invalid(format!("map/world_normal.bmp {why}")))?;
    match texture.image {
        Texture {
            format: TextureFormat::Rgba8,
            faces: 1,
            width,
            height,
            mut mips,
        } if mips.len() == 1 && mips[0].len() == 1 => Ok(Rgba8Image {
            width,
            height,
            pixels: mips.swap_remove(0).swap_remove(0).data,
        }),
        other => Err(invalid(format!(
            "{} is a {:?} texture of {} faces and {} levels, which the map's HeightNormal upload \
             does not take",
            texture.path,
            other.format,
            other.faces,
            other.mips.first().map_or(0, Vec::len)
        ))),
    }
}

#[cfg(test)]
mod tests;
