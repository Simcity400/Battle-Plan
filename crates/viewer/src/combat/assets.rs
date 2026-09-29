//! Only combat-window assets; all interface bytes pass through the resolved layers.
use super::layout::{Node, number, string, value};
use crate::map::{
    counters::FlagAtlas,
    lazy_icons::{LazyImages, LazyTextures, Lookup},
    sprites::SpriteSheet,
    text,
};
use installed_content::{
    ResolvedLayers,
    defines::Defines,
    flags::FlagSet,
    fonts, gfx, gui,
    image::{dds, tga},
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

/// Combat portrait textures kept resident at once beyond the ones on screen.
const COMBAT_PORTRAIT_CAPACITY: usize = 256;

/// One wanted sprite's file reads, resolved up front so the merge below
/// keeps the warning order and texture ids of the serial load.
enum WantedSprite {
    Skip,
    Warn(String),
    /// A leader portrait: decoded the first time a window draws it.
    Lazy(String),
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
#[derive(Clone)]
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
    sprites: BTreeMap<String, Sprite>,
    /// Leader portrait sprites by name: their texture and declaration, drawn
    /// once `portraits` has decoded the texture.
    lazy_sprites: BTreeMap<String, (String, gfx::SpriteType)>,
    portraits: Arc<LazyImages>,
    pub fonts: BTreeMap<String, text::Font>,
    /// The normal set, decoded as windows draw it.
    flags: Arc<FlagAtlas>,
    /// The counters' small set, which inline markers draw from.
    small_flags: Arc<FlagAtlas>,
    /// The small flag cell every [`Flag::inline`] is drawn at.
    pub inline_flag_size: [f32; 2],
    pub tactic_pictures: BTreeMap<String, String>,
    pub convoy_weapon_bound: Option<i64>,
    pub text_colors: fonts::TextColors,
    /// The land window's defines, or why the content lacks them.
    pub(crate) land_defines: Result<super::defines::LandDefines, String>,
    /// `NNavy.BATTLE_NAME_VP_FACTOR` and `BATTLE_NAME_VP_CUTOFF` (1415B06C0),
    /// fixed5, or why they are unavailable.
    pub(crate) naval_name_defines: Result<[i64; 2], String>,
    /// Authored division name groups, which render the recorded selection.
    pub(crate) division_names: installed_content::division_names::Database,
    pub warnings: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flag {
    pub texture: usize,
    pub uv: [f32; 4],
    pub inline: Option<(usize, [f32; 4])>,
}

impl Assets {
    /// A declared sprite, or a leader portrait once its texture has decoded;
    /// one not decoded yet is queued and draws nothing for that frame.
    pub fn sprite(&self, name: &str) -> Option<Cow<'_, Sprite>> {
        if let Some(sprite) = self.sprites.get(name) {
            return Some(Cow::Borrowed(sprite));
        }
        let (first, declaration) = self.lazy_sprites.get(name)?;
        let Lookup::Loaded { texture, size } = self.portraits.get(first) else {
            return None;
        };
        let sheet = SpriteSheet {
            name: first.clone(),
            texture,
            frames: 1,
            size,
        };
        Some(Cow::Owned(build_sprite(
            name,
            declaration,
            sheet,
            false,
            None,
        )))
    }

    /// Whether `name` is a sprite a window can draw: a declared one, or a
    /// leader portrait whose texture is not known to be unreadable.
    pub fn has_sprite(&self, name: &str) -> bool {
        self.sprites.contains_key(name)
            || self
                .lazy_sprites
                .get(name)
                .is_some_and(|(first, _)| self.portraits.peek(first) != Lookup::Failed)
    }

    /// Whether a normal-set flag file exists for `key` and is not known to be
    /// unreadable: the key the bindings choose among a country's candidates.
    pub fn has_flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }

    /// The normal-set flag of `key`, queued when not decoded yet.
    pub fn flag(&self, key: &str) -> Option<Flag> {
        let (texture, uv) = self.flags.get(key)?;
        Some(Flag {
            texture,
            uv,
            inline: self.inline_flag(key),
        })
    }

    /// The small flag an inline marker draws for `key`.
    ///
    /// A group flag falls back to its tag's plain flag. The tag is the key's
    /// first three bytes because `CCountryDatabase::load_tags` (`14070e900`)
    /// accepts only tags of exactly three bytes.
    pub fn inline_flag(&self, key: &str) -> Option<(usize, [f32; 4])> {
        if !self.has_flag(key) {
            return None;
        }
        match key.get(..3) {
            Some(tag) if tag != key => self.small_flags.resolve(&[key, tag]),
            _ => self.small_flags.resolve(&[key]),
        }
    }
}

/// One combat sprite from its declaration and its first texture's sheet.
fn build_sprite(
    name: &str,
    declaration: &gfx::SpriteType,
    mut sheet: SpriteSheet,
    progress: bool,
    second: Option<SpriteSheet>,
) -> Sprite {
    sheet.frames = declaration.frames;
    sheet.name = name.to_owned();
    let size = value(&declaration.items, "size")
        .and_then(|v| v.items())
        .map(|items| {
            [
                number(items, "x", sheet.frame_size()[0]),
                number(items, "y", sheet.frame_size()[1]),
            ]
        })
        .unwrap_or(sheet.frame_size());
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
    }
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
        shared: &Arc<ResolvedLayers>,
        defines: &Defines,
        declarations: &BTreeMap<String, gfx::SpriteType>,
        small: &Arc<FlagAtlas>,
        language: &str,
        images: &mut Vec<dds::Texture>,
        lazy: &Arc<LazyTextures>,
    ) -> io::Result<Self> {
        let layers: &ResolvedLayers = shared;
        // Combat widgets also reference the naval leader module's nested .gfx.
        let mut declarations = declarations.clone();
        let nested = "interface/building_roster/naval_leader_module.gfx";
        for declaration in gfx::parse_sprites(&layers.read(nested)?, nested)? {
            declarations.insert(declaration.name.clone(), declaration);
        }
        // Inventory literal portrait references once per content set, never by
        // decoding every historical hour. DDS compression stays intact on upload.
        // Each file as the characters reader reads it (the reader replay).
        let mut direct_portraits = BTreeSet::new();
        for file in layers
            .paths()
            .filter(|p| p.starts_with("common/characters/") && p.ends_with(".txt"))
        {
            portrait_paths(
                &installed_content::characters::read_file(&layers.read(file)?)
                    .0
                    .items,
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
        for file in layers
            .paths()
            .filter(|p| p.starts_with("common/unit_leader/") && p.ends_with(".txt"))
        {
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
                || declarations[*name]
                    .texture
                    .as_ref()
                    .is_some_and(|p| p.starts_with("gfx/leaders/"))
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
        let (division_names, mut warnings) = installed_content::division_names::read(layers)?;
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
            let second = string(&declaration.items, "textureFile2").map(path);
            // A plain leader portrait decodes on first draw: a content set can
            // declare one for every character of thousands of countries.
            if !progress && second.is_none() && first.starts_with("gfx/leaders/") {
                planned.push((name, WantedSprite::Lazy(first)));
                continue;
            }
            if !first_paths.contains(&first) {
                first_paths.push(first.clone());
            }
            planned.push((
                name,
                WantedSprite::Load {
                    progress,
                    first,
                    second,
                },
            ));
        }
        // First textures decode on a bounded pool; the merge keeps the serial
        // warning order and texture ids.
        let mut firsts: BTreeMap<String, io::Result<dds::Texture>> =
            crate::parallel::map(&first_paths, |path| {
                (path.clone(), decode_image(layers, path, is_uncovered(path)))
            })
            .into_iter()
            .collect();
        let mut lazy_sprites = BTreeMap::new();
        for (name, sprite) in planned {
            let (progress, first, second) = match sprite {
                WantedSprite::Skip => continue,
                WantedSprite::Warn(warning) => {
                    warnings.push(warning);
                    continue;
                }
                WantedSprite::Lazy(first) => {
                    let declaration = declarations[&name].clone();
                    lazy_sprites.insert(name, (first, declaration));
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
            let sheet = match decoded.get(&first).cloned() {
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
            let second = second
                .map(|p| load_image(layers, &p, images, false))
                .transpose()?;
            let sprite = build_sprite(&name, declaration, sheet, progress, second);
            sprites.insert(name, sprite);
        }
        let reader = Arc::clone(shared);
        let portraits = LazyImages::new(
            "combat portrait",
            Arc::new(move |path: &str| decode_image(&reader, path, true)),
            COMBAT_PORTRAIT_CAPACITY,
            lazy,
        );
        if !lazy_sprites.is_empty() {
            warnings.push(format!(
                "Combat UI: {} leader portrait sprites decode when a window first draws them; one whose texture cannot be read stays empty.",
                lazy_sprites.len()
            ));
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
                    text::load(layers, &definitions, &name, language, images)?,
                ))
            })
            .collect::<io::Result<_>>()?;
        // 14162FC20: maskedShieldType defaults to the normal flag atlas; the
        // medium one is a different ordinary-sprite path. Bake only the point
        // sampled shield alpha at twice its resolution, preserving native2:1
        // linear minification and keeping the authored overlay a separate draw.
        let mask = tga::decode(&layers.read("gfx/interface/shield_small_mask.tga")?)?;
        let cell = FlagSet::Normal.cell(defines)?;
        let flags = crate::map::counters::flag_set(
            shared,
            FlagSet::Normal,
            cell,
            move |image| {
                let [cell_width, cell_height] = cell.map(|side| side as usize);
                let [mask_width, mask_height] = [mask.width as usize, mask.height as usize];
                for row in 0..cell_height {
                    for column in 0..cell_width {
                        let at = (row * mask_height / cell_height * mask_width
                            + column * mask_width / cell_width)
                            * 4
                            + 3;
                        image.pixels[(row * cell_width + column) * 4 + 3] = mask.pixels[at];
                    }
                }
            },
            lazy,
        )?;
        // The inline markers draw from the counters' small set: the same
        // `gfx/flags/small` files at the same defines cell, decoded once.
        let small_cell = small.cell_size();
        Ok(Self {
            windows,
            sprites,
            lazy_sprites,
            portraits,
            fonts,
            flags,
            small_flags: Arc::clone(small),
            inline_flag_size: small_cell.map(|side| side as f32),
            tactic_pictures,
            convoy_weapon_bound: super::naval_environment::convoy_weapon_bound(layers)?,
            text_colors: fonts::read_text_colors(layers)?,
            land_defines: super::defines::LandDefines::read(defines)
                .map_err(|error| error.to_string()),
            naval_name_defines: ["BATTLE_NAME_VP_FACTOR", "BATTLE_NAME_VP_CUTOFF"]
                .map(|name| defines.fixed5("NNavy", name))
                .into_iter()
                .collect::<io::Result<Vec<_>>>()
                .map(|values| [values[0], values[1]])
                .map_err(|error| error.to_string()),
            division_names,
            warnings,
        })
    }
}
