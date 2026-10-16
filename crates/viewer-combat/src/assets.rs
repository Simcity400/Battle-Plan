//! Only combat-window assets; all interface bytes pass through the resolved layers.
use installed_content::unit_leader::SubUnits;
use installed_content::{
    ResolvedLayers,
    defines::Defines,
    gui,
    gui::native::{
        resources::{self, Registry},
        sprites::{Plain, SpriteFacts, SpriteKind, Sprites},
    },
    image::dds,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex},
};
use viewer_foundation::map::sprites::Interface;
use viewer_foundation::map::{
    lazy_icons::{LazyFlags, LazyImages, LazyTextures, Lookup},
    native_gui::{self, Sheets},
    native_resources::GuiResources,
    native_windows::NativeWindows,
    sprites::{NO_TEXTURE, SpriteSheet},
};
use viewer_foundation::tooltip_painter::TooltipDefines;

/// Combat portrait textures kept resident at once beyond the ones on screen.
const COMBAT_PORTRAIT_CAPACITY: usize = 256;

/// Portrait decode failures by sprite name: the refusal kind and its reason.
type PortraitFailures = BTreeMap<String, (Option<native_gui::RefusalKind>, String)>;

pub struct Assets {
    pub(crate) native_windows: NativeWindows,
    pub gui: GuiResources,
    window_sprites: BTreeSet<Vec<u8>>,
    flag_selectors: Mutex<BTreeMap<String, usize>>,
    /// Leader portrait sprites by name: their texture and declaration, drawn
    /// once `portraits` has decoded the texture.
    lazy_sprites: BTreeMap<String, (String, Plain)>,
    portraits: Arc<LazyImages>,
    portrait_failures: Arc<Mutex<PortraitFailures>>,
    /// The normal set, decoded as windows draw it.
    flags: Arc<LazyFlags>,
    /// The range of convoy weapon values the content allows, fixed5.
    pub convoy_weapon_bound: Result<[i64; 2], String>,
    /// The naval combat defines the window's positioning, screening and anti-air read;
    /// `None`, with a warning, when the content's defines do not load.
    pub naval_defines: Option<installed_content::naval_combat_defines::Defines>,
    /// NNavy operands, loaded from the matching content stack.
    pub(super) naval_escape_defines: Result<super::naval_escape::Defines, String>,
    /// Loss-equipment tint from the resolved content.
    pub(super) naval_result_pride_color: Result<[f32; 4], String>,
    pub(super) naval_escape_screening: super::naval_escape::Screening,
    pub(super) naval_air_groups: super::naval_air_groups::Content,
    /// six weather-event static accumulators, in event order.
    pub(super) naval_weather_modifiers: Vec<Result<installed_content::modifiers::Modifiers, String>>,
    /// Full screening bonus blocks, formatted at the freshly computed screening scale.
    pub(super) naval_screening_modifiers: [Result<installed_content::modifiers::Modifiers, String>; 2],
    /// Effective named static blocks of the strategic templates' custom sea grid.
    pub(super) naval_custom_sea_modifiers:
        BTreeMap<String, Result<installed_content::modifiers::Modifiers, String>>,
    /// terrain category prefix for the naval background hover.
    pub(super) naval_terrain: Result<super::naval_terrain::Content, String>,
    pub(super) land_terrain: Result<Arc<installed_content::terrain::TerrainDefinitions>, String>,
    pub temperature: Result<installed_content::weather::TemperatureRanges, String>,
    pub(super) border_events: installed_content::events::LazyEvents,
    /// The land window's defines, or why the content lacks them.
    pub(crate) land_defines: Result<super::defines::LandDefines, String>,
    /// `NNavy.BATTLE_NAME_VP_FACTOR` and `BATTLE_NAME_VP_CUTOFF`,
    /// fixed5, or why they are unavailable.
    pub(crate) naval_name_defines: Result<[i64; 2], String>,
    /// Authored division name groups, which render the recorded selection.
    pub division_names: Arc<installed_content::division_names::Database>,
    /// Authored leader traits and advisor ranks for the commander hovers.
    pub(super) commander_tooltips: super::commander_tooltips::Content,
    /// The content's tactic registry: triggers, weights, effects and
    /// pictures.
    pub tactics: super::tactics::Content,
    /// Tooltip delays and screen offsets, or why the content lacks them.
    pub tooltip_defines: Result<TooltipDefines, String>,
    pub warnings: Vec<String>,
}
impl Assets {
    pub fn movement_terrain(&self) -> Result<Arc<installed_content::terrain::TerrainDefinitions>, String> {
        self.land_terrain.clone()
    }
    pub fn sub_units(&self) -> Result<&installed_content::unit_leader::SubUnits, &str> {
        self.commander_tooltips.sub_units()
    }
    pub fn shared_sub_units(&self) -> Result<Arc<SubUnits>, String> {
        self.commander_tooltips.shared_sub_units()
    }
    pub(crate) fn window_sprites(&self) -> &BTreeSet<Vec<u8>> {
        &self.window_sprites
    }

    pub(crate) fn flag_index(&self, key: &str) -> Result<i32, String> {
        let mut indices = self
            .flag_selectors
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let next = indices
            .len()
            .checked_add(1)
            .ok_or_else(|| "resolved flag selector count overflow".to_owned())?;
        let index = *indices.entry(key.to_owned()).or_insert(next);
        i32::try_from(index)
            .map_err(|_| format!("resolved flag selector {index} exceeds the native i32 operand"))
    }

    pub fn load_gui(
        &mut self,
        layers: &ResolvedLayers,
        localisation: &installed_content::localisation::Localisation,
        extra: &native_gui::Assets,
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<Vec<String>> {
        let mut assets = native_window_assets(&self.native_windows, localisation);
        assets.extend(extra.clone());
        self.window_sprites.extend(assets.sprites.iter().cloned());
        assets.sprites.retain(|name| {
            std::str::from_utf8(name)
                .ok()
                .is_none_or(|name| !self.lazy_sprites.contains_key(name))
        });
        self.gui.load(layers, &assets, images)?;
        let mut warnings: Vec<_> = self
            .gui
            .missing_textures()
            .into_iter()
            .map(|(name, why)| format!("Combat UI: texture {name}: {why} (1 missing_texture)."))
            .collect();
        warnings.extend(sprite_warnings(
            &self.gui,
            &shortcut_assets(&self.native_windows).sprites,
            false, // missing_textures above already reports these textures.
        ));
        for name in &assets.fonts {
            let Some(font) = self.gui.font(name) else {
                warnings.push(format!(
                    "Combat UI: font {} is unavailable (1 missing_font).",
                    String::from_utf8_lossy(name)
                ));
                continue;
            };
            for fnt in &font.glyphs.missing {
                warnings.push(format!(
                    "Combat UI: font {}: missing font file {fnt} (1 missing_font_file).",
                    String::from_utf8_lossy(name)
                ));
            }
            for (page, loaded) in font.glyphs.pages.iter().zip(&font.pages) {
                if loaded.is_none() {
                    warnings.push(format!("Combat UI: font {}: page {} did not load; its glyphs advance and draw nothing (1 missing_font_page).",
                        String::from_utf8_lossy(name), String::from_utf8_lossy(page)));
                }
            }
            if font.glyphs.glyphs.is_empty() {
                warnings.push(format!(
                    "Combat UI: font {} loads no characters (1 empty_font).",
                    String::from_utf8_lossy(name)
                ));
            }
            if font.glyphs.line_height.is_none() || font.glyphs.base.is_none() {
                warnings.push(format!("Combat UI: font {} has no first-file lineHeight or base (1 missing_font_metrics).", String::from_utf8_lossy(name)));
            }
            for (kind, why) in &font.refusals {
                warnings.push(format!(
                    "Combat UI: font {}: {why} (1 {}).",
                    String::from_utf8_lossy(name),
                    kind.name()
                ));
            }
        }
        Ok(warnings)
    }

    /// Inventoried once per GUI load, then shared by counter and combat ToolTip resources.
    pub fn localisation_assets(
        &self,
        localisation: &installed_content::localisation::Localisation,
    ) -> (native_gui::Assets, Vec<String>) {
        let mut assets = tooltip_assets(&self.native_windows, localisation);
        let mut warnings = Vec::new();
        // the game generates inline sub-unit sprite names from data, not localisation.
        match self.commander_tooltips.sub_units() {
            Ok(subunits) => {
                let (unit_assets, unit_warnings) = unit_assets(subunits);
                assets.extend(unit_assets);
                warnings.extend(unit_warnings);
            }
            Err(reason) => warnings.push(format!(
                "Province hover: sub-unit icon content unavailable: {reason} (1 subunit_icon)."
            )),
        }
        (assets, warnings)
    }

    pub fn native_sprite(&self, name: &[u8]) -> Option<native_gui::SpriteInfo> {
        let mut info = self.gui.sprite(name)?;
        let Some((path, _)) = std::str::from_utf8(name)
            .ok()
            .and_then(|name| self.lazy_sprites.get(name))
        else {
            if self
                .gui
                .registry()
                .sprite(name)?
                .kind
                .plain()
                .is_some_and(|plain| plain.frames < 0)
            {
                info.refusals.push((
                    native_gui::RefusalKind::MissingResource,
                    format!(
                        "negative_frame_count: noOfFrames is {} (1 refusal)",
                        info.frames
                    ),
                ));
            }
            return Some(info);
        };
        match self.portraits.get(path) {
            Lookup::Loaded { size, .. } => info.texture = size.map(|value| value as i32),
            Lookup::Pending => {
                info.missing = Some("portrait texture is queued for native decoding".into());
                info.hit = native_gui::Hit::Through;
            }
            Lookup::Failed => {
                info.hit = native_gui::Hit::Through;
                let failures = self
                    .portrait_failures
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some((kind, why)) = failures.get(path) {
                    info.missing = Some(why.clone());
                    if let Some(kind) = kind {
                        info.refusals.push((*kind, why.clone()));
                    }
                } else {
                    info.refusals.push((
                        native_gui::RefusalKind::MissingResource,
                        format!("portrait texture {path} failed without a recorded decode reason"),
                    ));
                }
            }
        }
        Some(info)
    }

    /// Preserve the producers' bound dimensions while the native walk owns actual geometry.
    pub fn sprite_size(&self, name: &str) -> Option<[f32; 2]> {
        let info = self.native_sprite(name.as_bytes())?;
        if self.lazy_sprites.contains_key(name) {
            if info.missing.is_some() || !info.refusals.is_empty() {
                return None;
            }
            return Some([
                info.texture[0] as f32 / info.frames.max(1) as f32,
                info.texture[1] as f32,
            ]);
        }
        prepared_size(&self.gui, name.as_bytes(), &info)
    }

    /// Sheets for only the portrait names the frame reaches; lazy IDs are never reused.
    pub(crate) fn portrait_sheets<'a>(
        &self,
        names: impl IntoIterator<Item = &'a [u8]>,
    ) -> BTreeMap<Vec<u8>, SpriteSheet> {
        names
            .into_iter()
            .filter_map(|name| {
                let (path, plain) = self.lazy_sprites.get(std::str::from_utf8(name).ok()?)?;
                let Lookup::Loaded { texture, size } = self.portraits.peek(path) else {
                    return None;
                };
                Some((
                    name.to_vec(),
                    SpriteSheet {
                        name: String::from_utf8_lossy(name).into_owned(),
                        texture,
                        frames: plain.frames.max(1) as u32,
                        size,
                    },
                ))
            })
            .collect()
    }

    /// Whether `name` is a sprite a window can draw: a declared one, or a
    /// leader portrait whose texture is not known to be unreadable.
    pub fn has_sprite(&self, name: &str) -> bool {
        if let Some((path, _)) = self.lazy_sprites.get(name) {
            return self.portraits.peek(path) != Lookup::Failed;
        }
        self.gui
            .sprite(name.as_bytes())
            .is_some_and(|info| prepared_size(&self.gui, name.as_bytes(), &info).is_some())
    }

    /// One metadata lookup for an inline icon's drawable frame count.
    pub(super) fn drawable_sprite(&self, name: &[u8]) -> Option<native_gui::SpriteInfo> {
        let info = self.native_sprite(name)?;
        let name = std::str::from_utf8(name).ok()?;
        if let Some((path, _)) = self.lazy_sprites.get(name) {
            (self.portraits.peek(path) != Lookup::Failed).then_some(info)
        } else {
            prepared_size(&self.gui, name.as_bytes(), &info).map(|_| info)
        }
    }

    /// Whether a normal-set flag file exists for `key` and is not known to be
    /// unreadable: the key the bindings choose among a country's candidates.
    pub fn has_flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }

    /// Hands the tactic triggers the building, ideology and resource names
    /// count triggers find, from the databases the
    /// document already loaded, and gives back the scripted triggers and
    /// count names: the one read every trigger consumer shares.
    pub fn set_count_trigger_names(
        &mut self,
        buildings: &installed_content::buildings::Buildings,
        ideologies: &installed_content::ideologies::Ideologies,
        resources: &[installed_content::resources::Resource],
    ) -> installed_content::trigger_keys::TriggerNames {
        self.tactics
            .set_count_names(installed_content::trigger_keys::CountTriggerNames::new(
                buildings, ideologies, resources,
            ));
        self.tactics.trigger_names()
    }
}

fn prepared_size(
    gui: &GuiResources,
    name: &[u8],
    info: &native_gui::SpriteInfo,
) -> Option<[f32; 2]> {
    if info.missing.is_some()
        || !info.refusals.is_empty()
        || gui
            .registry()
            .sprite(name)?
            .kind
            .plain()
            .is_some_and(|plain| plain.frames < 0)
    {
        return None;
    }
    let sheet = gui.sheet(name)?;
    if sheet.texture == NO_TEXTURE
        || gui
            .mask(name)
            .is_some_and(|sheet| sheet.texture == NO_TEXTURE)
    {
        return None;
    }
    match info.kind {
        native_gui::SpriteKind::Progress { size, .. }
        | native_gui::SpriteKind::CorneredTile { size, .. }
        | native_gui::SpriteKind::Tile { size } => Some(size.map(|value| value as f32)),
        native_gui::SpriteKind::Resizeable => {
            let SpriteKind::Resizeable(sprite) = &gui.registry().sprite(name)?.kind else {
                return None;
            };
            Some(sprite.size.map(|value| value as f32))
        }
        _ => Some(sheet.frame_size()),
    }
}

fn sprite_warnings(
    gui: &GuiResources,
    names: &BTreeSet<Vec<u8>>,
    report_textures: bool,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for name in names {
        let display = String::from_utf8_lossy(name);
        let Some(info) = gui.sprite(name) else {
            warnings.push(format!(
                "Combat UI: sprite {display} is unavailable (1 missing_sprite)."
            ));
            continue;
        };
        if !info.refusals.is_empty() {
            for (kind, why) in &info.refusals {
                warnings.push(format!(
                    "Combat UI: sprite {display}: {why} (1 {}).",
                    kind.name()
                ));
            }
        } else if report_textures && let Some(why) = info.missing {
            warnings.push(format!(
                "Combat UI: sprite {display}: {why} (1 missing_texture)."
            ));
        } else if gui
            .registry()
            .sprite(name)
            .and_then(|facts| facts.kind.plain())
            .is_some_and(|plain| plain.frames < 0)
        {
            warnings.push(format!(
                "Combat UI: sprite {display}: noOfFrames is {} (1 negative_frame_count).",
                info.frames
            ));
        } else if report_textures
            && (gui
                .sheet(name)
                .is_none_or(|sheet| sheet.texture == NO_TEXTURE)
                || gui
                    .mask(name)
                    .is_some_and(|sheet| sheet.texture == NO_TEXTURE))
        {
            warnings.push(format!("Combat UI: sprite {display} requires loaded base and secondary textures (1 missing_texture)."));
        }
    }
    warnings
}

// Retained entry templates share their registry.
pub const WINDOW_TEMPLATES: &[&[u8]] = &[
    b"naval_region_modifier_entry",
    b"landcombatview",
    b"division_combat_attacker_entry",
    b"division_combat_defender_entry",
    b"reserve_combat_entry",
    b"tactics_list_view",
    b"tactics_list_entry",
    b"global_modifier_icon_entry",
    b"modifier_icon_entry",
    b"navalcombatview",
    b"navalcombat_ship_entry",
    b"navalcombatresultswindow",
    b"naval_combat_results_attacker_survivor_entry",
    b"naval_combat_results_defender_survivor_entry",
    b"naval_combat_results_attacker_lost_line_entry",
    b"naval_combat_results_defender_lost_line_entry",
    b"naval_combat_results_attacker_lost_ship_entry",
    b"naval_combat_results_defender_lost_ship_entry",
    b"naval_combat_results_attacker_air_entry",
    b"naval_combat_results_defender_air_entry",
    b"naval_combat_results_attacker_air_lost_line_entry",
    b"naval_combat_results_defender_air_lost_line_entry",
    b"naval_combat_results_attacker_lost_air_entry",
    b"naval_combat_results_defender_lost_air_entry",
    b"unit_leader_trait_entry",
    b"unit_leader_exile_trait_entry",
];

fn native_templates(windows: &NativeWindows) -> impl Iterator<Item = &gui::native::Element> {
    WINDOW_TEMPLATES
        .iter()
        .chain(super::picker::TEMPLATES)
        .filter_map(|name| windows.gui_type(name))
}

fn native_window_assets(
    windows: &NativeWindows,
    localisation: &installed_content::localisation::Localisation,
) -> native_gui::Assets {
    let mut assets = native_gui::Assets::default();
    for root in native_templates(windows) {
        assets.extend(native_gui::assets_in(
            root,
            &|name| windows.gui_type(name),
            localisation,
        ));
    }
    assets
}

fn tooltip_assets(
    windows: &NativeWindows,
    localisation: &installed_content::localisation::Localisation,
) -> native_gui::Assets {
    // inline icons use the global sprite registry.
    // Both combat drawing and the shared map tooltip painter load this inventory.
    let mut assets = native_gui::Assets::from_localisation(localisation);
    assets.extend(shortcut_assets(windows));
    assets
}

fn shortcut_assets(windows: &NativeWindows) -> native_gui::Assets {
    let mut assets = native_gui::Assets::default();
    for root in native_templates(windows) {
        for (_, element) in super::gui::elements(root) {
            // the first shortcut supplies inline keycap sprites.
            // The hover binder names and refuses non-UTF-8 shortcuts.
            if let Some(shortcut) = element
                .button()
                .and_then(|button| button.shortcuts.first())
                .and_then(|shortcut| std::str::from_utf8(shortcut).ok())
            {
                assets.add_text(&super::tooltips::shortcut_keycaps(shortcut));
            }
        }
    }
    assets
}

fn path(raw: &str) -> String {
    raw.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Plain texture type defaults from plain sprite construction.
fn texture_sprite(name: String, texture: String) -> SpriteFacts {
    SpriteFacts {
        name: name.into_bytes(),
        load_type: 0,
        kind: SpriteKind::Plain(Plain {
            texture: texture.into_bytes(),
            ..Plain::default()
        }),
        refusals: Vec::new(),
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
/// What the combat interface reads from the content alone, before any
/// artwork: the trait names, the tactic registry, the division names and the
/// commander and tooltip content, with the reads' warnings in the order
/// [`Assets::load`] reports them. The tactic registry keeps its own
/// ([`CombatContent::warnings`]).
#[derive(Debug)]
pub struct CombatContent {
    traits: installed_content::unit_leader::Traits,
    tactics: super::tactics::Content,
    division_names: installed_content::division_names::Database,
    commander_tooltips: super::commander_tooltips::Content,
    /// The tooltip placement and timing defines, or why they are unreadable.
    tooltip_defines: Result<TooltipDefines, String>,
    warnings: Vec<String>,
}

impl CombatContent {
    pub fn read(layers: &ResolvedLayers, defines: &Defines) -> io::Result<Self> {
        // The one trait-name inventory (trait registration) and the one
        // sub-unit database, for has_trait, has_artillery_ratio, the trait
        // sprites and the commander hovers.
        let traits = installed_content::unit_leader::Traits::registered(layers)?;
        let sub_units =
            installed_content::unit_leader::sub_units(layers).map_err(|error| error.to_string());
        // The country-leader traits, which a character's has_trait and the
        // advisor-trait hover lines name.
        let country_leader_traits = installed_content::leader_traits::read(layers)
            .map(|(traits, _)| Arc::new(traits))
            .map_err(|error| error.to_string());
        // The tactic registry as the game reads it: the file's one read
        // (installed_content::combat_tactics).
        let tactics =
            super::tactics::Content::from_database(installed_content::combat_tactics::read(layers)?)
                .with_inventories(
                    traits.clone(),
                    sub_units.as_ref(),
                    installed_content::scripted_triggers::read(layers)?,
                )
                .with_country_leader_traits(country_leader_traits.clone());
        let (division_names, mut warnings) = installed_content::division_names::read(layers)?;
        let (commander_tooltips, skills) = super::commander_tooltips::Content::load(
            layers,
            defines,
            sub_units,
            country_leader_traits,
        )?;
        let tooltips = super::tooltips::Content::load(skills);
        warnings.extend(tooltips.warnings().iter().cloned());
        warnings.extend(commander_tooltips.warnings());
        warnings.extend(traits.unsupported.iter().map(|reason| {
            format!("Combat UI: {reason}; no trait sprite is loaded for those names.")
        }));
        let tooltip_defines = TooltipDefines::read(defines).map_err(|error| error.to_string());
        Ok(Self {
            traits,
            tactics,
            division_names,
            commander_tooltips,
            tooltip_defines,
            warnings,
        })
    }

    /// Why the combat tooltips are unavailable, as a warning.
    fn tooltip_warning(&self) -> Option<String> {
        let reason = self.tooltip_defines.as_ref().err()?;
        Some(format!("Combat tooltips are unavailable: {reason}."))
    }

    /// Hands the tactic triggers the count trigger names, as
    /// [`Assets::set_count_trigger_names`] does.
    pub fn set_count_trigger_names(
        &mut self,
        buildings: &installed_content::buildings::Buildings,
        ideologies: &installed_content::ideologies::Ideologies,
        resources: &[installed_content::resources::Resource],
    ) {
        self.tactics
            .set_count_names(installed_content::trigger_keys::CountTriggerNames::new(
                buildings, ideologies, resources,
            ));
    }

    /// The reads' warnings, then the tactic registry's and the tooltip
    /// defines', in the order [`Assets::load`] reports them.
    pub fn warnings(&self) -> impl Iterator<Item = String> + '_ {
        self.warnings
            .iter()
            .chain(self.tactics.warnings())
            .cloned()
            .chain(self.tooltip_warning())
    }

    /// Each tactic the registry refuses, or whose picture it cannot form.
    pub fn refusals(&self) -> impl Iterator<Item = &str> {
        self.tactics.refusals()
    }
}

impl Assets {
    #[allow(clippy::too_many_arguments)] // The client's content, windows and texture stores.
    pub fn load(
        shared: &Arc<ResolvedLayers>,
        regions: &BTreeMap<u32, installed_content::regions::StrategicRegion>,
        defines: &Defines,
        interface: &Interface,
        flags: &Arc<LazyFlags>,
        client: (&str, [u32; 2], &NativeWindows),
        images: &mut Vec<dds::Texture>,
        lazy: &Arc<LazyTextures>,
    ) -> io::Result<Self> {
        let (language, map_size, native_windows) = client;
        let layers: &ResolvedLayers = shared;
        for name in WINDOW_TEMPLATES {
            if native_windows.gui_type(name).is_none() {
                return Err(super::invalid(format!(
                    "Combat UI: no .gui file declares {} (1 missing_gui_type)",
                    String::from_utf8_lossy(name)
                )));
            }
        }
        let mut files = interface.registry.files.clone();
        let mut direct = Sprites::default();
        // Inventory literal portrait references once per content set, never by
        // decoding every historical hour. DDS compression stays intact on upload.
        // Each file as the characters reader reads it (the reader replay).
        let mut direct_portraits = BTreeSet::new();
        let names = layers.key_names()?;
        for file in layers
            .paths()
            .filter(|p| p.starts_with("common/characters/") && p.ends_with(".txt"))
        {
            portrait_paths(
                &installed_content::characters::read_file(&layers.read(file)?, &names)
                    .0
                    .items,
                &mut direct_portraits,
            );
        }
        for texture in direct_portraits {
            let key = format!("combat_portrait:{texture}");
            direct.sprites.push(texture_sprite(key, texture));
        }
        let content = CombatContent::read(layers, defines)?;
        let content_warnings: Vec<String> = content.warnings().collect();
        let CombatContent {
            traits,
            tactics,
            division_names,
            commander_tooltips,
            tooltip_defines,
            ..
        } = content;
        // Each proven tactic's picture declares its texture; the widgets look
        // it up through the registry.
        for (key, texture) in tactics.pictures() {
            direct.sprites.push(texture_sprite(key.to_owned(), texture));
        }
        // The singleton picture is empty; its authored drawable stays
        // unless that empty picture's path exists.
        let null_picture = "gfx/interface/landcombat/tactics/.dds";
        if layers.has_file(null_picture)? {
            direct.sprites.push(texture_sprite(
                super::tactics::NULL_PICTURE.into(),
                null_picture.into(),
            ));
        }
        files.push(("combat runtime textures".into(), direct));
        let registry = Registry::from_files(files);
        let mut wanted = BTreeSet::new();
        // Construction finds scrollbar and listbox templates through all `.gui` files.
        let native_assets = native_window_assets(native_windows, &Default::default());
        wanted.extend(
            native_assets
                .sprites
                .iter()
                .filter_map(|name| std::str::from_utf8(name).ok().map(str::to_owned)),
        );
        // Trait-tree connectors also start with GFX_trait_. Load only the
        // sprites of registered traits, by their
        // first spelling, instead of that editor UI. A name that is not UTF-8
        // is not registered (Traits::unsupported names it).
        for name in traits.spellings() {
            wanted.insert(format!("GFX_trait_{name}"));
        }
        let mut warnings = content_warnings;
        for (file, refusal) in native_windows.reader_failures() {
            warnings.push(format!("Combat UI: {file}: {refusal} (1 gui_reader)."));
        }
        for root in native_templates(native_windows) {
            for refusal in root.subtree_refusals() {
                warnings.push(format!("Combat UI: {refusal} (1 gui_reader)."));
            }
        }
        for sprite in installed_content::gui::native::sprites::registry(&registry.files).values() {
            let name = &sprite.name;
            let portrait = sprite
                .kind
                .plain()
                .is_some_and(|plain| plain.texture.starts_with(b"gfx/leaders/"));
            if name.starts_with(b"GFX_terrain_")
                || name.starts_with(b"GFX_navalcombat_ship_icon_")
                // the game resolves the air icon by country/archetype/sprite
                // at runtime, outside the authored `.gui` resource names.
                || name.starts_with(b"GFX_navalcombat_air_icon_")
                // Retained aircraft survivors use the same cascade.
                || name.starts_with(b"GFX_navalcombat_air_small_icon_")
                || (name.starts_with(b"GFX_unit_")
                    && (name.ends_with(b"_icon_medium") || name.ends_with(b"_icon_medium_black")))
                || name.starts_with(b"combat_tactic:")
                || name.starts_with(b"combat_portrait:")
                // The map mode picker selects scripted modes' button sprites by name.
                || name.starts_with(b"GFX_mapmode_buttons_")
                || portrait
                || [
                    b"GFX_organization_texticon".as_slice(),
                    b"GFX_strenght_texticon",
                    b"GFX_modifiers_weather_icons",
                    b"GFX_flag_small2_not_masked",
                ]
                .contains(&name.as_slice())
                || (name.starts_with(b"GFX_div_templ_") && name.ends_with(b"_large"))
            {
                // Resource identifiers stay byte-for-byte in the native loader.
                // The producer's existing text names cannot address other bytes.
                match std::str::from_utf8(name) {
                    Ok(name) => {
                        wanted.insert(name.to_owned());
                    }
                    Err(_) => warnings.push(format!(
                        "Combat UI: sprite name {name:?} cannot be addressed by the text-valued combat bindings; its widget stays empty (1 refusal)."
                    )),
                }
            }
        }
        let mut lazy_sprites = BTreeMap::new();
        let window_sprites = native_assets.sprites.clone();
        let mut resident = native_gui::Assets {
            sprites: native_assets.sprites,
            fonts: native_assets.fonts,
            textures: native_assets.textures,
        };
        for name in &wanted {
            if let Some(facts) = registry.sprite(name.as_bytes())
                && let SpriteKind::Plain(plain) = &facts.kind
                && plain.texture.starts_with(b"gfx/leaders/")
                && plain.masking_texture.is_empty()
                && (plain.legacy_lazy_load || plain.animations.is_empty())
                && plain.frames >= 0
                && facts.refusals.is_empty()
                && let Ok(first) = std::str::from_utf8(&plain.texture)
            {
                lazy_sprites.insert(name.clone(), (first.to_owned(), plain.clone()));
                resident.sprites.remove(name.as_bytes());
            } else {
                resident.sprites.insert(name.as_bytes().to_vec());
            }
        }
        // language-family selection compares the complete l_ language key.
        let mut gui = GuiResources::new(
            registry,
            format!("l_{language}").as_bytes(),
            map_size,
            interface.decode,
        );
        gui.load(layers, &resident, images)?;
        for name in lazy_sprites.keys() {
            gui.load_lazy_plain(layers, name.as_bytes())
                .map_err(super::invalid)?;
        }
        warnings.extend(sprite_warnings(&gui, &resident.sprites, true));
        warnings.extend(gui.uncovered_note("Combat UI"));
        let reader = Arc::clone(shared);
        let decode = interface.decode;
        let portrait_failures = Arc::new(Mutex::new(BTreeMap::new()));
        let failures = Arc::clone(&portrait_failures);
        let portraits = LazyImages::new(
            "combat portrait",
            Arc::new(move |path: &str| {
                match resources::stack_texture(&reader, path.as_bytes(), decode) {
                    Ok(Ok(texture)) => Ok(texture.image),
                    Ok(Err(why)) => {
                        let kind = if why.native_failure() {
                            None
                        } else if matches!(why, resources::NoTexture::Device { .. }) {
                            Some(native_gui::RefusalKind::MissingInput)
                        } else {
                            Some(native_gui::RefusalKind::UndefinedRead)
                        };
                        let reason = why.to_string();
                        failures
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .insert(path.to_owned(), (kind, reason.clone()));
                        Err(io::Error::new(io::ErrorKind::InvalidData, reason))
                    }
                    Err(error) => {
                        failures
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .insert(
                                path.to_owned(),
                                (
                                    Some(native_gui::RefusalKind::MissingResource),
                                    error.to_string(),
                                ),
                            );
                        Err(error)
                    }
                }
            }),
            COMBAT_PORTRAIT_CAPACITY,
            lazy,
        );
        if !lazy_sprites.is_empty() {
            warnings.push(format!(
                "Combat UI: {} leader portrait sprites decode when a window first draws them; one whose texture cannot be read stays empty.",
                lazy_sprites.len()
            ));
        }
        let equipment = installed_content::equipment::Database::load(layers)?;
        let (convoy_weapon_bound, misreads) =
            super::naval_environment::convoy_weapon_bound_from_database(&equipment);
        warnings.extend(misreads.into_iter().map(|misread| {
            format!(
                "Convoy equipment content: {misread}; the weapon bound follows the game's reading."
            )
        }));
        if let Err(reason) = &convoy_weapon_bound {
            warnings.push(format!(
                "Naval weapon totals with live convoys stay dash: the convoy equipment's weapon values are unknown ({reason})."
            ));
        }
        let naval_defines = match installed_content::naval_combat_defines::read(layers) {
            Ok(defines) => Some(defines),
            Err(error) => {
                warnings.push(format!(
                    "Naval combat defines: {error}; positioning, screening and anti-air are unavailable."
                ));
                None
            }
        };
        let naval_escape_defines = super::naval_escape::Defines::read(defines);
        let naval_escape_screening = super::naval_escape::Screening::load(layers);
        let naval_air_groups =
            super::naval_air_groups::Content::load(layers, &mut warnings, equipment);
        // six modifiers share registered names.
        let weather_modifiers = &super::naval_environment::WEATHER_MODIFIERS;
        let naval_weather_modifiers = match installed_content::static_modifiers::read_blocks(
            layers,
            weather_modifiers,
        ) {
            Ok(blocks) => {
                warnings.extend(blocks.notes.into_iter().map(|note| {
                    format!(
                        "Naval weather static modifiers: {note}; values follow the game's reading."
                    )
                }));
                let mut refused = 0;
                let modifiers = blocks
                    .blocks
                    .into_iter()
                    .map(|block| {
                        block.map_err(|refusal| {
                            refused += 1;
                            format!(
                                "{} in {}: {}",
                                refusal.modifier, refusal.file, refusal.reason
                            )
                        })
                    })
                    .collect();
                if refused > 0 {
                    warnings.push(format!(
                        "Naval weather static modifiers: {refused} of {} full accumulators refused.",
                        weather_modifiers.len()
                    ));
                }
                modifiers
            }
            Err(error) => {
                warnings.push(format!(
                    "Naval weather static modifiers ({}): {error}; all {} full accumulators unavailable.",
                    installed_content::static_modifiers::DIRECTORY,
                    weather_modifiers.len()
                ));
                weather_modifiers
                    .map(|name| Err(format!("{name}: {error}")))
                    .into_iter()
                    .collect()
            }
        };
        let screening_names = ["screening_bonus", "capital_screening_bonus"];
        let naval_screening_modifiers =
            match installed_content::static_modifiers::read_blocks(layers, &screening_names) {
                Ok(blocks) => {
                    warnings.extend(
                        blocks
                            .notes
                            .into_iter()
                            .map(|note| format!("Naval screening static modifiers: {note}")),
                    );
                    let mut blocks = blocks.blocks.into_iter();
                    std::array::from_fn(|_| {
                        blocks
                            .next()
                            .expect("one result per requested static block")
                            .map_err(|refusal| {
                                format!(
                                    "{} in {}: {}",
                                    refusal.modifier, refusal.file, refusal.reason
                                )
                            })
                    })
                }
                Err(error) => std::array::from_fn(|index| {
                    Err(format!(
                        "{} in {}: {error}",
                        screening_names[index],
                        installed_content::static_modifiers::DIRECTORY
                    ))
                }),
            };
        let custom_names: Vec<String> = regions
            .values()
            .flat_map(|region| region.static_modifier_names.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let naval_custom_sea_modifiers = if custom_names.is_empty() {
            BTreeMap::new()
        } else {
            let names: Vec<&str> = custom_names.iter().map(String::as_str).collect();
            match installed_content::static_modifiers::read_blocks(layers, &names) {
                Ok(blocks) => {
                    warnings.extend(
                        blocks
                            .notes
                            .into_iter()
                            .map(|note| format!("Naval custom sea static modifiers: {note}")),
                    );
                    custom_names
                        .into_iter()
                        .zip(blocks.blocks.into_iter().map(|block| {
                            block.map_err(|refusal| {
                                format!(
                                    "{} in {}: {}",
                                    refusal.modifier, refusal.file, refusal.reason
                                )
                            })
                        }))
                        .collect()
                }
                Err(error) => custom_names
                    .into_iter()
                    .map(|name| (name.clone(), Err(format!("{name}: {error}"))))
                    .collect(),
            }
        };
        // The one terrain database, shared by both combat views.
        let land_terrain = installed_content::terrain::read_definitions(layers)
            .map(Arc::new)
            .map_err(|error| error.to_string());
        let naval_terrain = land_terrain.clone().and_then(|terrain| {
            super::naval_terrain::Content::load(layers, defines, terrain)
                .map_err(|error| error.to_string())
        });
        Ok(Self {
            native_windows: native_windows.clone(),
            gui,
            window_sprites,
            flag_selectors: Mutex::default(),
            lazy_sprites,
            portraits,
            portrait_failures,
            flags: Arc::clone(flags),
            convoy_weapon_bound,
            naval_defines,
            naval_escape_defines,
            naval_result_pride_color: super::naval_results::pride_color(defines),
            naval_escape_screening,
            naval_air_groups,
            naval_weather_modifiers,
            naval_screening_modifiers,
            naval_custom_sea_modifiers,
            naval_terrain,
            land_terrain,
            temperature: installed_content::weather::temperature_ranges(layers)
                .map_err(|error| error.to_string()),
            border_events: installed_content::events::LazyEvents::new(Arc::clone(shared)),
            land_defines: super::defines::LandDefines::read(defines)
                .map_err(|error| error.to_string()),
            naval_name_defines: ["BATTLE_NAME_VP_FACTOR", "BATTLE_NAME_VP_CUTOFF"]
                .map(|name| defines.fixed5("NNavy", name))
                .into_iter()
                .collect::<io::Result<Vec<_>>>()
                .map(|values| [values[0], values[1]])
                .map_err(|error| error.to_string()),
            division_names: Arc::new(division_names),
            commander_tooltips,
            tactics,
            tooltip_defines,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;

/// Glyph inventory from the parsed sub-unit database. Both NATO frames
/// use the same declaration; missing declarations draw nothing.
fn unit_assets(subunits: &SubUnits) -> (native_gui::Assets, Vec<String>) {
    let mut assets = native_gui::Assets::default();
    let mut refused = Vec::new();
    for spelling in subunits.registered() {
        let result = (|| {
            let name = std::str::from_utf8(spelling)
                .map_err(|_| format!("sub-unit {:?} name is not UTF-8", spelling))?;
            let category = subunits.category(spelling)?;
            let prefix = if category & 0x80004003C1 != 0 {
                "ship_"
            } else {
                "unit_"
            };
            Ok::<_, String>(format!("\u{13}{prefix}{name}_icon_small|1 "))
        })();
        match result {
            Ok(text) => assets.add_text(&text),
            Err(reason) => refused.push(format!("Unit composition: {reason} (1 subunit_icon).")),
        }
    }
    (assets, refused)
}
