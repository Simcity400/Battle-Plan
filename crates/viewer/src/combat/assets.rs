//! Only combat-window assets; all interface bytes pass through the resolved layers.
use super::layout::{Node, number, string, value};
use crate::map::{sprites::SpriteSheet, text};
use installed_content::{
    ResolvedLayers, fonts, gfx, gui,
    image::{Rgba8Image, dds, tga},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// One wanted sprite's file reads, resolved up front so the merge below
/// keeps the warning order and texture ids of the serial load.
enum WantedSprite {
    Skip,
    Warn(String),
    Load {
        progress: bool,
        first: String,
        second: Option<String>,
    },
}

/// Portraits and inline icons resolve outside the verified layers.
fn is_uncovered(path: &str) -> bool {
    path.starts_with("gfx/leaders/") || path.starts_with("gfx/texticons/")
}
pub struct Sprite {
    pub sheet: SpriteSheet,
    pub second: Option<SpriteSheet>,
    pub size: [f32; 2],
    pub vertical: bool,
    pub progress: bool,
    /// `effectFile = gfx/FX/progress_startend.lua`: three stacked bands.
    pub start_end: bool,
    /// `corneredTileSpriteType` `borderSize` in texels, and `tilingCenter`.
    pub border: Option<[f32; 2]>,
    pub tiling_center: bool,
}
pub struct Assets {
    pub windows: BTreeMap<String, Node>,
    pub sprites: BTreeMap<String, Sprite>,
    pub fonts: BTreeMap<String, text::Font>,
    pub flags: BTreeMap<String, Flag>,
    pub tactic_pictures: BTreeMap<String, String>,
    pub score_defines: Option<super::land_details::ScoreDefines>,
    pub convoy_weapon_bound: Option<i64>,
    pub text_colors: fonts::TextColors,
    pub warnings: Vec<String>,
}
pub struct Flag {
    pub texture: usize,
    pub uv: [f32; 4],
    pub inline: Option<(usize, [f32; 4])>,
}
fn path(raw: &str) -> String {
    raw.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}
fn decode_image(layers: &ResolvedLayers, path: &str, uncovered: bool) -> io::Result<dds::Texture> {
    // Shipped declarations retain .tga names after their textures were converted to DDS.
    // Resolve only a declared asset's same-stem replacement, through the verified layer index.
    let replacement = path.rsplit_once('.').map(|(stem, _)| format!("{stem}.dds"));
    let read = |path: &str| {
        if uncovered {
            layers.read_uncovered(path)
        } else {
            layers.read(path)
        }
    };
    let bytes = match read(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            read(replacement.as_deref().unwrap_or(path))?
        }
        result => result?,
    };
    if bytes.starts_with(b"DDS ") {
        Ok(super::texture::uploadable(dds::decode(&bytes)?)?)
    } else {
        Ok(dds::Texture::from_rgba8(tga::decode(&bytes).map_err(
            |error| io::Error::new(error.kind(), format!("combat texture {path}: {error}")),
        )?))
    }
}
fn load_image(
    layers: &ResolvedLayers,
    path: &str,
    images: &mut Vec<dds::Texture>,
    uncovered: bool,
) -> io::Result<SpriteSheet> {
    let texture = decode_image(layers, path, uncovered)?;
    let sheet = SpriteSheet {
        name: path.into(),
        texture: images.len(),
        frames: 1,
        size: [texture.width, texture.height],
    };
    images.push(texture);
    Ok(sheet)
}
fn collect(node: &Node, sprites: &mut BTreeSet<String>, fonts: &mut BTreeSet<String>) {
    if let Some(name) = gui::sprite(&node.items) {
        sprites.insert(name.into());
    }
    if let Some(name) = gui::font(&node.items) {
        fonts.insert(name.into());
    }
    for child in &node.children {
        collect(child, sprites, fonts);
    }
}
fn texture_declaration(name: String, texture: String, source: &str) -> gfx::SpriteType {
    gfx::SpriteType {
        name,
        kind: "spriteType".into(),
        texture: Some(texture),
        frames: 1,
        always_transparent: false,
        items: Vec::new(),
        source: source.into(),
    }
}
fn portrait_paths(items: &[installed_content::parser::Item], paths: &mut BTreeSet<String>) {
    for item in items {
        if let Some(text) = item.value.scalar_str() {
            let p = path(text);
            if p.starts_with("gfx/leaders/") && (p.ends_with(".dds") || p.ends_with(".tga")) {
                paths.insert(p);
            }
        }
        if let Some(children) = item.value.items() {
            portrait_paths(children, paths);
        }
    }
}
fn expand_gui(document: &mut installed_content::parser::Document) {
    use installed_content::parser::Value;
    let variables: BTreeMap<_, _> = document
        .items
        .iter()
        .filter_map(|item| {
            let key = item.key.as_ref()?;
            key.starts_with(b"@")
                .then(|| (key.clone(), item.value.clone()))
        })
        .collect();
    fn walk(value: &mut Value, variables: &BTreeMap<Vec<u8>, Value>) {
        match value {
            Value::Scalar(key) => {
                if let Some(v) = variables.get(key) {
                    *value = v.clone();
                }
            }
            Value::Block(items) | Value::Tagged { items, .. } => {
                for item in items {
                    walk(&mut item.value, variables);
                }
            }
        }
    }
    for item in &mut document.items {
        walk(&mut item.value, &variables);
    }
}
impl Assets {
    pub fn load(
        layers: &ResolvedLayers,
        declarations: &BTreeMap<String, gfx::SpriteType>,
        images: &mut Vec<dds::Texture>,
        combat_details: bool,
    ) -> io::Result<Self> {
        // Combat widgets also reference the naval leader module's nested .gfx.
        let mut declarations = declarations.clone();
        let nested = "interface/building_roster/naval_leader_module.gfx";
        for declaration in gfx::parse_sprites(&layers.read(nested)?, nested)? {
            declarations.insert(declaration.name.clone(), declaration);
        }
        // Inventory literal portrait references once per content set, never by
        // decoding every historical hour. DDS compression stays intact on upload.
        let mut direct_portraits = BTreeSet::new();
        for file in layers.paths().filter(|p| {
            combat_details && p.starts_with("common/characters/") && p.ends_with(".txt")
        }) {
            portrait_paths(
                &installed_content::parser::parse(&layers.read(file)?)?.items,
                &mut direct_portraits,
            );
        }
        for texture in direct_portraits {
            let key = format!("combat_portrait:{texture}");
            declarations.insert(
                key.clone(),
                texture_declaration(key, texture, "common/characters"),
            );
        }
        let mut tactic_pictures = BTreeMap::new();
        let file = "common/combat_tactics.txt";
        for item in installed_content::parser::parse(&layers.read(file)?)?.items {
            let Some(items) = item.value.items() else {
                continue;
            };
            let (Some(token), Some(picture)) = (
                item.key
                    .as_deref()
                    .and_then(|s| std::str::from_utf8(s).ok()),
                string(items, "picture"),
            ) else {
                continue;
            };
            let texture = format!("gfx/interface/landcombat/tactics/{picture}.dds");
            let key = format!("combat_tactic:{picture}");
            declarations.insert(key.clone(), texture_declaration(key.clone(), texture, file));
            tactic_pictures.insert(token.into(), key);
        }
        let mut windows = BTreeMap::new();
        for (file, names) in [
            ("interface/mapicons.gui", vec!["naval_combat_mapicon"]),
            (
                "interface/naval_strategic_region_view.gui",
                vec!["naval_region_modifier_entry"],
            ),
            (
                "interface/landcombat.gui",
                vec![
                    "landcombatview",
                    "division_combat_attacker_entry",
                    "division_combat_defender_entry",
                    "reserve_combat_entry",
                    "global_modifier_icon_entry",
                    "modifier_icon_entry",
                ],
            ),
            (
                "interface/navalcombat.gui",
                vec!["navalcombatview", "navalcombat_ship_entry"],
            ),
            (
                "interface/navalcombatresultswindow.gui",
                vec![
                    "navalcombatresultswindow",
                    "naval_combat_results_attacker_survivor_entry",
                    "naval_combat_results_defender_survivor_entry",
                    "naval_combat_results_attacker_lost_line_entry",
                    "naval_combat_results_defender_lost_line_entry",
                    "naval_combat_results_attacker_lost_ship_entry",
                    "naval_combat_results_defender_lost_ship_entry",
                    "naval_combat_results_attacker_air_entry",
                    "naval_combat_results_defender_air_entry",
                    "naval_combat_results_attacker_air_lost_line_entry",
                    "naval_combat_results_defender_air_lost_line_entry",
                    "naval_combat_results_attacker_lost_air_entry",
                    "naval_combat_results_defender_lost_air_entry",
                ],
            ),
            (
                "interface/unitleaderwindow.gui",
                vec!["unit_leader_trait_entry"],
            ),
        ] {
            let mut document = gui::read(layers, file)?;
            expand_gui(&mut document);
            for name in names {
                let items = gui::find(&document, "containerWindowType", name)
                    .ok_or_else(|| super::invalid(format!("{file} has no {name}")))?;
                windows.insert(name.into(), Node::new("containerWindowType", items));
            }
        }
        let core = gui::read(layers, "interface/core.gui")?;
        for name in ["left_vertical_slider", "right_vertical_slider"] {
            let items = gui::find(&core, "extendedScrollbarType", name)
                .ok_or_else(|| super::invalid(format!("core.gui has no {name}")))?;
            windows.insert(name.into(), Node::new("extendedScrollbarType", items));
        }
        let mut wanted = BTreeSet::new();
        let mut wanted_fonts = BTreeSet::new();
        for node in windows.values() {
            collect(node, &mut wanted, &mut wanted_fonts);
        }
        // Trait-tree connectors also start with GFX_trait_. Inventory actual
        // unit-leader definitions instead of loading that unrelated editor UI.
        for file in layers.paths().filter(|p| {
            combat_details && p.starts_with("common/unit_leader/") && p.ends_with(".txt")
        }) {
            for item in installed_content::parser::parse(&layers.read(file)?)?.items {
                if item.key.as_deref() != Some(b"leader_traits") {
                    continue;
                }
                for entry in item.value.items().unwrap_or_default() {
                    if let Some(token) = entry
                        .key
                        .as_deref()
                        .and_then(|s| std::str::from_utf8(s).ok())
                    {
                        wanted.insert(format!("GFX_trait_{token}"));
                    }
                }
            }
        }
        for name in declarations.keys().filter(|name| {
            name.starts_with("GFX_terrain_")
                || name.starts_with("GFX_navalcombat_ship_icon_")
                || (name.starts_with("GFX_unit_")
                    && (name.ends_with("_icon_medium") || name.ends_with("_icon_medium_black")))
                || name.starts_with("combat_tactic:")
                || name.starts_with("combat_portrait:")
                || (combat_details
                    && declarations[*name]
                        .texture
                        .as_ref()
                        .is_some_and(|p| p.starts_with("gfx/leaders/")))
                || name == &"GFX_organization_texticon"
                || name == &"GFX_strenght_texticon"
                || name == &"GFX_modifiers_weather_icons"
                || (name.starts_with("GFX_div_templ_") && name.ends_with("_large"))
                || name.starts_with("GFX_mapicon_naval_combat")
                || name == &"GFX_naval_hq_combat_frame"
                || name == &"GFX_navalcombat_bar_progress_attacker_154"
                || name == &"GFX_flag_small2_not_masked"
        }) {
            wanted.insert(name.clone());
        }
        let mut sprites = BTreeMap::new();
        let mut warnings = Vec::new();
        let mut decoded = BTreeMap::new();
        let mut uncovered_portraits = false;
        // Resolve every wanted sprite's reads up front; the first textures
        // decode on scoped threads below and the merge keeps the serial
        // warning order and texture ids.
        let mut planned = Vec::new();
        let mut first_paths = Vec::new();
        for name in wanted {
            if name == "GFX_tiled_window_transparent" || name == "GFX_flag_small2" {
                planned.push((name, WantedSprite::Skip));
                continue;
            }
            let Some(declaration) = declarations.get(&name) else {
                let warning =
                    format!("Combat UI: sprite {name} is not declared; its widget stays empty.");
                planned.push((name, WantedSprite::Warn(warning)));
                continue;
            };
            let progress = declaration.kind.eq_ignore_ascii_case("progressbartype");
            let first = declaration
                .texture
                .as_deref()
                .or_else(|| string(&declaration.items, "textureFile1"));
            let Some(first) = first else {
                let warning = format!("Combat UI: {name} has no texture; widget stays empty.");
                planned.push((name, WantedSprite::Warn(warning)));
                continue;
            };
            let first = path(first);
            if is_uncovered(&first) {
                uncovered_portraits = true;
            }
            if !first_paths.contains(&first) {
                first_paths.push(first.clone());
            }
            planned.push((
                name,
                WantedSprite::Load {
                    progress,
                    first,
                    second: string(&declaration.items, "textureFile2").map(path),
                },
            ));
        }
        // First textures decode on scoped threads; the merge keeps the
        // serial warning order and texture ids.
        let mut firsts: BTreeMap<String, io::Result<dds::Texture>> = std::thread::scope(|scope| {
            first_paths
                .iter()
                .map(|path| {
                    scope.spawn(move || {
                        (path.clone(), decode_image(layers, path, is_uncovered(path)))
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("combat sprite decoder panicked"))
                .collect()
        });
        for (name, sprite) in planned {
            let (progress, first, second) = match sprite {
                WantedSprite::Skip => continue,
                WantedSprite::Warn(warning) => {
                    warnings.push(warning);
                    continue;
                }
                WantedSprite::Load {
                    progress,
                    first,
                    second,
                } => (progress, first, second),
            };
            let declaration = declarations
                .get(&name)
                .expect("wanted sprite declared up front");
            let mut sheet = match decoded.get(&first).cloned() {
                Some(sheet) => sheet,
                None => {
                    match firsts
                        .remove(&first)
                        .expect("combat sprite decoded up front")
                    {
                        Ok(texture) => {
                            let sheet = SpriteSheet {
                                name: first.clone(),
                                texture: images.len(),
                                frames: 1,
                                size: [texture.width, texture.height],
                            };
                            images.push(texture);
                            decoded.insert(first.clone(), sheet.clone());
                            sheet
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {
                            warnings.push(format!("Combat UI: declared sprite {name} has no texture {first}; widget stays empty."));
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                }
            };
            sheet.frames = declaration.frames;
            sheet.name = name.clone();
            let size = value(&declaration.items, "size")
                .and_then(|v| v.items())
                .map(|items| {
                    [
                        number(items, "x", sheet.frame_size()[0]),
                        number(items, "y", sheet.frame_size()[1]),
                    ]
                })
                .unwrap_or(sheet.frame_size());
            let second = second
                .map(|p| load_image(layers, &p, images, false))
                .transpose()?;
            sprites.insert(
                name,
                Sprite {
                    sheet,
                    second,
                    size,
                    vertical: string(&declaration.items, "horizontal") == Some("no"),
                    progress,
                    start_end: string(&declaration.items, "effectFile")
                        .is_some_and(|file| file.ends_with("progress_startend.lua")),
                    border: (declaration
                        .kind
                        .eq_ignore_ascii_case("corneredTileSpriteType"))
                    .then(|| value(&declaration.items, "borderSize").and_then(|v| v.items()))
                    .flatten()
                    .map(|items| [number(items, "x", 0.0), number(items, "y", 0.0)]),
                    tiling_center: string(&declaration.items, "tilingCenter") == Some("yes"),
                },
            );
        }
        if uncovered_portraits {
            warnings.push("Combat portraits and inline icons under gfx/leaders and gfx/texticons are outside this recording's content hash coverage; their visual identity is UNVERIFIED without a covered content manifest.".into());
        }
        let definitions = fonts::read_definitions(layers)?;
        let fonts = wanted_fonts
            .into_iter()
            .map(|name| {
                Ok((
                    name.clone(),
                    text::load(layers, &definitions, &name, images)?,
                ))
            })
            .collect::<io::Result<_>>()?;
        // 14162FC20: maskedShieldType defaults to the large82x52 atlas;
        // medium41x26 is a different ordinary-sprite path. Bake only the point
        // sampled shield alpha at twice its resolution, preserving native2:1
        // linear minification and keeping the authored overlay a separate draw.
        let mask = tga::decode(&layers.read("gfx/interface/shield_small_mask.tga")?)?;
        let mut flags = flag_atlas(
            layers,
            "gfx/flags/",
            [82, 52],
            Some(&mask),
            images,
            &mut warnings,
        )?;
        let small = flag_atlas(
            layers,
            "gfx/flags/small/",
            [10, 7],
            None,
            images,
            &mut warnings,
        )?;
        for (key, flag) in &mut flags {
            if let Some(inline) = small
                .get(key)
                .or_else(|| key.get(..3).and_then(|tag| small.get(tag)))
            {
                flag.inline = Some((inline.texture, inline.uv));
            }
        }
        Ok(Self {
            windows,
            sprites,
            fonts,
            flags,
            tactic_pictures,
            score_defines: super::land_details::ScoreDefines::load(layers)?,
            convoy_weapon_bound: super::naval_environment::convoy_weapon_bound(layers)?,
            text_colors: fonts::read_text_colors(layers)?,
            warnings,
        })
    }
}

/// One bounded atlas per native flag class, independent of replay length.
fn flag_atlas(
    layers: &ResolvedLayers,
    directory: &str,
    cell: [u32; 2],
    mask: Option<&installed_content::image::Rgba8Image>,
    images: &mut Vec<dds::Texture>,
    warnings: &mut Vec<String>,
) -> io::Result<BTreeMap<String, Flag>> {
    let files: Vec<_> = layers
        .paths()
        .filter(|p| {
            p.strip_prefix(directory)
                .is_some_and(|name| !name.contains('/') && name.ends_with(".tga"))
        })
        .collect();
    let width = cell[0] * 32;
    let height = cell[1] * (files.len() as u32).div_ceil(32).max(1);
    let texture = images.len();
    let mut pixels = vec![0; (width * height * 4) as usize];
    let mut flags = BTreeMap::new();
    // Flag files read and decode on scoped threads; the pack below runs in
    // file order, so grid positions and warnings match the serial load.
    let decoded: Vec<io::Result<Rgba8Image>> = std::thread::scope(|scope| {
        files
            .iter()
            .map(|file| scope.spawn(move || tga::decode(&layers.read(file)?)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().expect("combat flag decoder panicked"))
            .collect()
    });
    for (index, (file, image)) in files.into_iter().zip(decoded).enumerate() {
        let image = image?;
        if [image.width, image.height] != cell {
            warnings.push(format!(
                "Combat UI: {file} is not the native{}x{} flag size; omitted.",
                cell[0], cell[1]
            ));
            continue;
        }
        let x = index as u32 % 32 * cell[0];
        let y = index as u32 / 32 * cell[1];
        for row in 0..cell[1] {
            for column in 0..cell[0] {
                let from = ((row * cell[0] + column) * 4) as usize;
                let to = (((y + row) * width + x + column) * 4) as usize;
                pixels[to..to + 4].copy_from_slice(&image.pixels[from..from + 4]);
                if let Some(mask) = mask {
                    let at = ((row * mask.height / cell[1] * mask.width
                        + column * mask.width / cell[0])
                        * 4
                        + 3) as usize;
                    pixels[to + 3] = mask.pixels[at];
                }
            }
        }
        flags.insert(
            file.trim_start_matches(directory)
                .trim_end_matches(".tga")
                .into(),
            Flag {
                texture,
                uv: [
                    x as f32 / width as f32,
                    y as f32 / height as f32,
                    (x + cell[0]) as f32 / width as f32,
                    (y + cell[1]) as f32 / height as f32,
                ],
                inline: None,
            },
        );
    }
    images.push(dds::Texture::from_rgba8(installed_content::image::Rgba8Image {
        width,
        height,
        pixels,
    }));
    Ok(flags)
}
