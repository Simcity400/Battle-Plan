use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_script::sub_units::SubUnits;
use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_gui::gui::{self, native::{resources::{self, Registry}, sprites::{Plain, SpriteFacts, SpriteKind}}};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex},
};
use viewer_native_gui::sprites::Interface;
use viewer_native_gui::{
    lazy_icons::{LazyFlags, LazyImages, LazyTextures, Lookup},
    native_gui::{self, Sheets},
    native_resources::GuiResources,
    native_windows::NativeWindows,
    sprites::{NO_TEXTURE, SpriteSheet},
};
use viewer_native_gui::tooltip_painter::TooltipDefines;

type PortraitFailures = BTreeMap<String, (Option<native_gui::RefusalKind>, String)>;

pub struct Assets {
    pub(super) layers: Arc<ResolvedLayers>,
    pub(super) air_icons:
        std::sync::OnceLock<Result<dashboard_base::products::equipment_icon::IconContent, String>>,
    pub(super) air_timeout: Result<i32, String>,
    pub(super) air_fuel_penalty: Result<i64, String>,
    pub(super) air_day_night: Result<replay_data::day_night::Defines, String>,
    pub(super) air_mine_maximum: Result<i64, String>,
    pub(super) naval_region_ship_intel: Result<[i64; 4], String>,
    pub(super) naval_region_convoy_intel: Result<[i64; 3], String>,
    pub(super) air_cipher: Result<dashboard_text::cryptology::Defines, String>,
    pub(super) air_cipher_modifiers: Result<content_reader_script::modifiers::Modifiers, String>,
    pub(crate) native_windows: NativeWindows,
    pub gui: GuiResources,
    window_sprites: BTreeSet<Vec<u8>>,
    flag_selectors: Mutex<BTreeMap<String, usize>>,
    lazy_sprites: BTreeMap<String, (String, Plain)>,
    portraits: Arc<LazyImages>,
    portrait_failures: Arc<Mutex<PortraitFailures>>,
    flags: Arc<LazyFlags>,
    pub convoy_weapon_bound: Result<[i64; 2], String>,
    pub naval_defines: Option<content_reader_naval_combat_defines::naval_combat_defines::Defines>,
    pub(super) naval_escape_defines: Result<super::naval_escape::Defines, String>,
    pub(super) naval_result_pride_color: Result<[f32; 4], String>,
    pub(super) naval_escape_screening: super::naval_escape::Screening,
    pub(super) naval_air_groups: super::naval_air_groups::Content,
    pub(super) naval_weather_modifiers: Vec<Result<content_reader_script::modifiers::Modifiers, String>>,
    pub(super) naval_screening_modifiers: [Result<content_reader_script::modifiers::Modifiers, String>; 2],
    pub(super) naval_custom_sea_modifiers:
        BTreeMap<String, Result<content_reader_script::modifiers::Modifiers, String>>,
    pub(super) naval_terrain: Result<super::naval_terrain::Content, String>,
    pub(super) land_terrain: Result<Arc<content_reader_key_names::terrain::TerrainDefinitions>, String>,
    pub temperature: Result<content_reader_weather::weather::TemperatureRanges, String>,
    pub(crate) province_weather: Result<content_reader_weather::weather::ProvinceModifierRules, String>,
    pub(crate) required_rules: Result<RequiredRules, String>,
    pub(crate) owner_info_modifiers: Result<[content_reader_script::modifiers::Modifiers; 3], String>,
    pub(crate) province_icon_modifiers: BTreeMap<String, Result<content_reader_script::modifiers::Modifiers, String>>,
    pub(crate) non_core_manpower: Result<i64, String>,
    pub(crate) max_shared_slots: Result<i32, String>,
    pub(super) border_events: content_reader_events::events::LazyEvents,
    pub(crate) land_defines: Result<super::defines::LandDefines, String>,
    pub division_names: Arc<content_reader_division_names::division_names::Database>,
    pub(super) commander_tooltips: super::commander_tooltips::Content,
    pub tactics: super::tactics::Content,
    pub tooltip_defines: Result<TooltipDefines, String>,
    pub warnings: Vec<String>,
}
#[derive(Debug)]
pub(crate) struct RequiredRules {
    pub rules: Vec<content_reader_map::adjacency_rules::AdjacencyRule>,
    pub by_province: BTreeMap<u32, usize>,
}

impl RequiredRules {
    fn new(rules: Vec<content_reader_map::adjacency_rules::AdjacencyRule>) -> Self {
        let mut by_province = BTreeMap::new();
        for (index, rule) in rules.iter().enumerate() {
            for &province in &rule.required_provinces {
                if province != 0 {
                    by_province.entry(province).or_insert(index);
                }
            }
        }
        Self { rules, by_province }
    }

    pub fn of(&self, province: u32) -> Option<&content_reader_map::adjacency_rules::AdjacencyRule> {
        self.by_province.get(&province).map(|&index| &self.rules[index])
    }
}

impl Assets {
    pub fn prepare_air_icons(
        &self,
        scripted: &Result<dashboard_text::scripted_text::ScriptedContent, String>,
    ) {
        self.air_icons.get_or_init(|| {
            let equipment = self
                .naval_air_groups
                .shared_equipment()
                .ok_or("air combat equipment definitions are unavailable".to_owned())?;
            dashboard_base::products::equipment_icon::IconContent::native(
                Arc::clone(&self.layers),
                equipment,
                scripted,
                self.gui.registry(),
            )
            .map_err(|e| e.to_string())
        });
    }

    pub fn movement_terrain(
        &self,
    ) -> Result<Arc<content_reader_key_names::terrain::TerrainDefinitions>, String> {
        self.land_terrain.clone()
    }
    pub fn sub_units(&self) -> Result<&content_reader_script::sub_units::SubUnits, &str> {
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

    pub fn defer_window_art(
        &mut self,
        layers: &ResolvedLayers,
        localisation: &content_reader_localisation::localisation::Localisation,
        extra: &native_gui::Assets,
        textures: &Arc<LazyTextures>,
    ) -> Vec<String> {
        let mut assets = native_window_assets(&self.native_windows, localisation);
        assets.extend(extra.clone());
        self.window_sprites.extend(assets.sprites.iter().cloned());
        assets.sprites.retain(|name| {
            std::str::from_utf8(name)
                .ok()
                .is_none_or(|name| !self.lazy_sprites.contains_key(name))
        });
        let mut warnings = self.gui.loaded_font_warnings(&assets.fonts);
        warnings.extend(self.gui.missing_textures().into_iter().map(|(name, why)| format!("Combat UI: texture {name}: {why} (1 missing_texture).")));
        for name in shortcut_assets(&self.native_windows).sprites {
            if self.gui.registry().sprite(&name).is_none() {
                warnings.push(format!("Combat UI: sprite {} is unavailable (1 missing_sprite).", String::from_utf8_lossy(&name)));
            }
        }
        self.gui.defer_window_art(layers, &assets, textures);
        warnings
    }

    pub fn localisation_assets(
        &self,
        localisation: &content_reader_localisation::localisation::Localisation,
    ) -> (native_gui::Assets, Vec<String>) {
        let mut assets = tooltip_assets(&self.native_windows, localisation);
        let mut warnings = Vec::new();
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

    fn lazy_sprite<'a>(&'a self, name: &'a str) -> Option<(&'a str, &'a Plain)> {
        if let Some(path) = name
            .strip_prefix("air_texture:")
            .filter(|path| !path.is_empty())
        {
            return self
                .lazy_sprites
                .get("air_texture:")
                .map(|(_, plain)| (path, plain));
        }
        self.lazy_sprites
            .get(name)
            .map(|(path, plain)| (path.as_str(), plain))
    }

    pub fn native_sprite(&self, name: &[u8]) -> Option<native_gui::SpriteInfo> {
        self.native_sprite_info(name).map(Cow::into_owned)
    }

    pub(crate) fn native_sprite_info(
        &self,
        name: &[u8],
    ) -> Option<Cow<'_, native_gui::SpriteInfo>> {
        let mut info = Cow::Borrowed(self.gui.sprite_info(Self::lazy_key(name))?);
        let Some((path, _)) = std::str::from_utf8(name)
            .ok()
            .and_then(|name| self.lazy_sprite(name))
        else {
            if self
                .gui
                .registry()
                .sprite(name)?
                .kind
                .plain()
                .is_some_and(|plain| plain.frames < 0)
            {
                let frames = info.frames;
                info.to_mut().refusals.push((
                    native_gui::RefusalKind::MissingResource,
                    format!("negative_frame_count: noOfFrames is {frames} (1 refusal)"),
                ));
            }
            return Some(info);
        };
        let mut info = info.into_owned();
        match self.portraits.get(path) {
            Lookup::Loaded { size, .. } => info.texture = size.map(|value| value as i32),
            Lookup::Pending => {
                info.missing = Some("sprite texture is queued for native decoding".into());
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
                        format!("sprite texture {path} failed without a recorded decode reason"),
                    ));
                }
            }
        }
        Some(Cow::Owned(info))
    }

    fn lazy_key<'a>(name: &'a [u8]) -> &'a [u8] {
        if name.starts_with(b"air_texture:") {
            b"air_texture:".as_slice()
        } else {
            name
        }
    }

    pub fn sprite_size(&self, name: &str) -> Option<[f32; 2]> {
        if let Some((path, _)) = self.lazy_sprite(name) {
            let info = self.gui.sprite_info(Self::lazy_key(name.as_bytes()))?;
            if !info.refusals.is_empty() {
                return None;
            }
            let Lookup::Loaded { size, .. } = self.portraits.acquire(path) else {
                return None;
            };
            return Some([
                (size[0] / info.frames.max(1) as u32) as f32,
                size[1] as f32,
            ]);
        }
        let info = self.native_sprite(name.as_bytes())?;
        prepared_size(&self.gui, name.as_bytes(), &info)
    }

    pub(crate) fn portrait_sheets<'a>(
        &self,
        names: impl IntoIterator<Item = &'a [u8]>,
    ) -> BTreeMap<Vec<u8>, SpriteSheet> {
        names
            .into_iter()
            .filter_map(|name| {
                let (path, plain) = self.lazy_sprite(std::str::from_utf8(name).ok()?)?;
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

    pub(crate) fn acquire_sprite(&self, name: &str) {
        if let Some((path, _)) = self.lazy_sprite(name) {
            let _ = self.portraits.acquire(path);
        }
    }

    pub fn has_sprite(&self, name: &str) -> bool {
        if let Some((path, _)) = self.lazy_sprite(name) {
            return matches!(self.portraits.acquire(path), Lookup::Loaded { .. });
        }
        self.gui
            .sprite(name.as_bytes())
            .is_some_and(|info| prepared_size(&self.gui, name.as_bytes(), &info).is_some())
    }

    pub(super) fn drawable_sprite(&self, name: &[u8]) -> Option<native_gui::SpriteInfo> {
        if let Some((path, _)) = std::str::from_utf8(name)
            .ok()
            .and_then(|text| self.lazy_sprite(text))
        {
            let mut info = self.gui.sprite_info(Self::lazy_key(name))?.clone();
            if !info.refusals.is_empty() {
                return None;
            }
            let Lookup::Loaded { size, .. } = self.portraits.acquire(path) else {
                return None;
            };
            info.texture = size.map(|value| value as i32);
            return Some(info);
        }
        let info = self.native_sprite(name)?;
        prepared_size(&self.gui, name, &info).map(|_| info)
    }

    pub fn has_flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }

    pub fn set_count_trigger_names(
        &mut self,
        buildings: &content_reader_key_names::buildings::Buildings,
        ideologies: &content_reader_key_names::ideologies::Ideologies,
        resources: &content_reader_key_names::resources::Resources,
    ) -> content_reader_script::trigger_keys::TriggerNames {
        self.tactics
            .set_count_names(content_reader_key_names::key_name_products::count_trigger_names(
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
            && !matches!(info.kind, native_gui::SpriteKind::LineChart { .. })
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

pub const WINDOW_TEMPLATES: &[&[u8]] = &[
    b"strategicairview",
    b"air_combat_damage_received",
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
        .chain(super::confirmation::TEMPLATES)
        .chain(super::minimap::TEMPLATES)
        .chain(super::find_view::TEMPLATES)
        .chain(super::state_view::TEMPLATES)
        .chain(super::naval_region_view::TEMPLATES)
        .filter_map(|name| windows.gui_type(name))
}

fn native_window_assets(
    windows: &NativeWindows,
    localisation: &content_reader_localisation::localisation::Localisation,
) -> native_gui::Assets {
    let mut assets = native_gui::Assets::default();
    for root in native_templates(windows) {
        assets.extend(native_gui::assets_in(
            root,
            &|name| windows.gui_type(name),
            localisation,
        ));
    }
    assets.add_text(&super::find_view::victory_point_icon(1));
    assets
}

fn tooltip_assets(
    windows: &NativeWindows,
    localisation: &content_reader_localisation::localisation::Localisation,
) -> native_gui::Assets {
    let mut assets = native_gui::Assets::from_localisation(localisation);
    assets.extend(shortcut_assets(windows));
    assets
}

fn shortcut_assets(windows: &NativeWindows) -> native_gui::Assets {
    let mut assets = native_gui::Assets::default();
    for root in native_templates(windows) {
        for (_, element) in super::gui::elements(root) {
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

fn content_named_sprites(layers: &ResolvedLayers, warnings: &mut Vec<String>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut unread = |what: &str, why: io::Error| {
        warnings.push(format!("Combat UI: the {what} do not load: {why} (1 missing_resource)."));
    };
    match content_reader_key_names::buildings::read(layers) {
        Ok(buildings) => names.extend(buildings.types.into_values().map(|building| building.special_icon)),
        Err(why) => unread("building special icons", why),
    }
    match content_reader_politics::occupation_laws::read(layers) {
        Ok(laws) => names.extend(laws.definitions.into_values().map(|law| law.sprite)),
        Err(why) => unread("occupation law icons", why),
    }
    match content_reader_politics::dynamic_modifiers::read(layers) {
        Ok(modifiers) => names.extend(
            modifiers
                .definitions
                .into_values()
                .filter_map(|modifier| String::from_utf8(modifier.icon).ok()),
        ),
        Err(why) => unread("dynamic modifier icons", why),
    }
    names.remove("");
    names
}

fn path(raw: &str) -> String {
    raw.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

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

fn portrait_paths(items: &[content_reader_parser::parser::Item], paths: &mut BTreeSet<String>) {
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
#[derive(Debug)]
pub struct CombatContent {
    traits: content_reader_key_names::unit_leader::Traits,
    tactics: super::tactics::Content,
    division_names: content_reader_division_names::division_names::Database,
    commander_tooltips: super::commander_tooltips::Content,
    tooltip_defines: Result<TooltipDefines, String>,
    warnings: Vec<String>,
}

impl CombatContent {
    pub fn read(layers: &ResolvedLayers, defines: &Defines) -> io::Result<Self> {
        Self::read_with_products(None, layers, defines)
    }

    pub fn read_with_products(
        products: Option<&map_content::loader::Loader>,
        layers: &ResolvedLayers,
        defines: &Defines,
    ) -> io::Result<Self> {
        let map_content::loader::CombatInputs {
            traits,
            sub_units,
            country_traits: country_leader_traits,
            database,
            scripted,
        } = match products {
            Some(products) => products.combat(layers)?,
            None => map_content::loader::CombatInputs::read(layers)?,
        };
        let tactics = super::tactics::Content::from_database(database)
            .with_inventories(traits.clone(), sub_units.as_ref(), scripted)
            .with_country_leader_traits(country_leader_traits.clone());
        let (division_names, mut warnings) =
            content_reader_division_names::division_names::read(layers)?;
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

    fn tooltip_warning(&self) -> Option<String> {
        let reason = self.tooltip_defines.as_ref().err()?;
        Some(format!("Combat tooltips are unavailable: {reason}."))
    }

    pub fn set_count_trigger_names(
        &mut self,
        buildings: &content_reader_key_names::buildings::Buildings,
        ideologies: &content_reader_key_names::ideologies::Ideologies,
        resources: &content_reader_key_names::resources::Resources,
    ) {
        self.tactics
            .set_count_names(content_reader_key_names::key_name_products::count_trigger_names(
                buildings, ideologies, resources,
            ));
    }

    pub fn warnings(&self) -> impl Iterator<Item = String> + '_ {
        self.warnings
            .iter()
            .chain(self.tactics.warnings())
            .cloned()
            .chain(self.tooltip_warning())
    }

    pub fn refusals(&self) -> impl Iterator<Item = &str> {
        self.tactics.refusals()
    }
}

impl Assets {
    #[expect(clippy::too_many_arguments, reason = "the layers, regions, defines, interface and client the assets load from, and the images they fill")]
    pub fn load(
        shared: &Arc<ResolvedLayers>,
        regions: &BTreeMap<u32, content_reader_regions::regions::StrategicRegion>,
        defines: &Defines,
        interface: &Interface,
        flags: &Arc<LazyFlags>,
        client: (&str, [u32; 2], &NativeWindows),
        images: &mut Vec<viewer_gpu::textures::GuiSource>,
        lazy: &Arc<LazyTextures>,
    ) -> io::Result<Self> {
        Self::load_with_products(
            None, shared, regions, defines, interface, flags, client, images, lazy,
        )
    }

    pub fn load_with_products(
        products: Option<&map_content::loader::Loader>,
        shared: &Arc<ResolvedLayers>,
        regions: &BTreeMap<u32, content_reader_regions::regions::StrategicRegion>,
        defines: &Defines,
        interface: &Interface,
        flags: &Arc<LazyFlags>,
        client: (&str, [u32; 2], &NativeWindows),
        images: &mut Vec<viewer_gpu::textures::GuiSource>,
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
        let mut direct = Vec::new();
        let mut direct_portraits = BTreeSet::new();
        let names = layers.key_names()?;
        for file in layers
            .paths()?
            .filter(|p| p.starts_with(b"common/characters/") && p.ends_with(b".txt"))
        {
            portrait_paths(
                &content_reader_key_names::characters::read_file(&layers.read_path(file)?, &names)
                    .0
                    .items,
                &mut direct_portraits,
            );
        }
        for texture in direct_portraits {
            let key = format!("combat_portrait:{texture}");
            direct.push(texture_sprite(key, texture));
        }
        direct.push(texture_sprite("air_texture:".into(), String::new()));
        let content = CombatContent::read_with_products(products, layers, defines)?;
        let content_warnings: Vec<String> = content.warnings().collect();
        let CombatContent {
            traits,
            tactics,
            division_names,
            commander_tooltips,
            tooltip_defines,
            ..
        } = content;
        for (key, texture) in tactics.pictures() {
            direct.push(texture_sprite(key.to_owned(), texture));
        }
        let null_picture = "gfx/interface/landcombat/tactics/.dds";
        if layers.has_file(null_picture)? {
            direct.push(texture_sprite(
                super::tactics::NULL_PICTURE.into(),
                null_picture.into(),
            ));
        }
        let registry = Registry::with_sprites(Arc::clone(&interface.registry), "combat runtime textures".into(), direct);
        let mut wanted = BTreeSet::new();
        let native_assets = native_window_assets(native_windows, &Default::default());
        wanted.extend(
            native_assets
                .sprites
                .iter()
                .filter_map(|name| std::str::from_utf8(name).ok().map(str::to_owned)),
        );
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
        for sprite in registry.visible_sprites().values() {
            let name = &sprite.name;
            let portrait = sprite
                .kind
                .plain()
                .is_some_and(|plain| plain.texture.starts_with(b"gfx/leaders/"));
            if name.starts_with(b"GFX_terrain_")
                || name.starts_with(b"GFX_navalcombat_ship_icon_")
                || name.starts_with(b"GFX_navalcombat_air_icon_")
                || name.starts_with(b"GFX_navalcombat_air_small_icon_")
                || name.ends_with(b"_medium")
                || name.starts_with(b"air_texture:")
                || (name.starts_with(b"GFX_unit_")
                    && (name.ends_with(b"_icon_medium") || name.ends_with(b"_icon_medium_black")))
                || name.starts_with(b"combat_tactic:")
                || name.starts_with(b"combat_portrait:")
                || name.starts_with(b"GFX_mapmode_buttons_")
                || name.starts_with(b"GFX_strategic_location_")
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
        wanted.extend(
            content_named_sprites(layers, &mut warnings)
                .into_iter()
                .filter(|name| registry.sprite(name.as_bytes()).is_some()),
        );
        let mut lazy_sprites = BTreeMap::new();
        let window_sprites = native_assets.sprites.clone();
        let mut resident = native_gui::Assets {
            sprites: native_assets.sprites,
            fonts: native_assets.fonts,
            textures: native_assets.textures,
        };
        for name in &wanted {
            let always_eager = name.starts_with("GFX_mapmode_buttons_")
                || name.starts_with("combat_tactic:")
                || name == super::tactics::NULL_PICTURE;
            let window = window_sprites.contains(name.as_bytes());
            if !always_eager
                && let Some(facts) = registry.sprite(name.as_bytes())
                && let SpriteKind::Plain(plain) = &facts.kind
                && (!window
                    || plain.texture.starts_with(b"gfx/leaders/")
                    || name.ends_with("_medium")
                    || name.starts_with("air_texture:"))
                && (!plain.texture.is_empty() || name.starts_with("air_texture:"))
                && !plain.transparence_check
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
        let strict = content_reader_layers::content::is_strict();
        let recorded = (!strict).then(|| reader.without_overlays()).flatten();
        let decode = interface.decode;
        let portrait_failures = Arc::new(Mutex::new(BTreeMap::new()));
        let failures = Arc::clone(&portrait_failures);
        let portraits = LazyImages::new(
            "combat sprite",
            Arc::new(move |path: &str| {
                let load = || {
                    resources::stack_texture_or_recorded(
                        &reader,
                        recorded.as_ref(),
                        path.as_bytes(),
                        decode,
                    )
                };
                let loaded = if strict {
                    content_reader_layers::content::strict(load)
                } else {
                    load()
                };
                match loaded {
                    Ok(Ok(texture)) => Ok(texture.image),
                    Ok(Err(why)) => {
                        let kind = if why.native_failure() {
                            None
                        } else if matches!(
                            why,
                            resources::NoTexture::Device { .. }
                                | resources::NoTexture::OverBudget { .. }
                        ) {
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
            lazy,
        );
        if !lazy_sprites.is_empty() {
            warnings.push(format!(
                "Combat UI: {} combat sprites decode when a window first draws them; one whose texture cannot be read stays empty.",
                lazy_sprites.len()
            ));
        }
        let equipment = content_reader_key_names::equipment::Database::shared(layers)?;
        let (convoy_weapon_bound, misreads) =
            super::naval_environment::convoy_weapon_bound_from_database(&equipment);
        warnings.extend(misreads.into_iter().map(|misread| {
            format!(
                "Convoy equipment content: {misread}; the weapon bound follows the game's reading."
            )
        }));
        if let Err(reason) = &convoy_weapon_bound {
            warnings.push(format!(
                "Weapon totals stay a dash while convoys are in a battle: the recording does not hold the convoy equipment's weapon values ({reason})."
            ));
        }
        let naval_defines = match content_reader_naval_combat_defines::naval_combat_defines::read(layers)
        {
            Ok(defines) => {
                if let Err(why) = &defines.piercing {
                    warnings.push(format!(
                        "Naval piercing defines: {why}; the light and heavy gun piercing explanations are unavailable."
                    ));
                }
                Some(defines)
            }
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
        let weather_modifiers = &super::naval_environment::WEATHER_MODIFIERS;
        let naval_weather_modifiers = match content_reader_politics::static_modifiers::read_blocks(
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
                    content_reader_politics::static_modifiers::DIRECTORY,
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
            match content_reader_politics::static_modifiers::read_blocks(layers, &screening_names) {
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
                        content_reader_politics::static_modifiers::DIRECTORY
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
            match content_reader_politics::static_modifiers::read_blocks(layers, &names) {
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
        let land_terrain = content_reader_key_names::terrain::read_definitions(layers)
            .map(Arc::new)
            .map_err(|error| error.to_string());
        let naval_terrain = land_terrain.clone().and_then(|terrain| {
            super::naval_terrain::Content::load(layers, defines, terrain)
                .map_err(|error| error.to_string())
        });
        Ok(Self {
            native_windows: native_windows.clone(),
            layers: Arc::clone(shared),
            air_icons: Default::default(),
            air_cipher_modifiers: content_reader_politics::static_modifiers::read_blocks(
                layers,
                &["passive_decryption_modifier"],
            )
            .map_err(|why| why.to_string())
            .and_then(|blocks| {
                blocks
                    .blocks
                    .into_iter()
                    .next()
                    .ok_or("air cipher modifier read is incomplete".to_owned())?
                    .map_err(|why| format!("{} in {}: {}", why.modifier, why.file, why.reason))
            }),
            air_timeout: defines
                .int32("NGame", "AIR_LOG_TIMEOUT_HOURS")
                .map_err(|e| e.to_string()),
            air_fuel_penalty: defines
                .fixed5("NAir", "MISSION_EFFICIENCY_MULT_AT_LACK_OF_FUEL")
                .map_err(|e| e.to_string()),
            air_day_night: replay_data::day_night::Defines::read(
                |group, key| defines.int32(group, key),
                |group, key| defines.fixed5(group, key),
            )
            .map_err(|e| e.to_string()),
            air_mine_maximum: defines
                .fixed5("NNavy", "NAVAL_MINES_IN_REGION_MAX")
                .map(|value| value.max(100_000))
                .map_err(|e| e.to_string()),
            naval_region_ship_intel: (|| {
                let caps = defines.fixed5_array("NIntel", "COUNTRY_LEVEL_INTEL_MAXIMUMS")?;
                let cap = caps.first().copied().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "NIntel.COUNTRY_LEVEL_INTEL_MAXIMUMS is empty")
                })?;
                let fixed = |key| defines.fixed5("NIntel", key);
                Ok::<_, std::io::Error>([
                    cap,
                    fixed("NAVY_SHIP_COUNT_INTEL_MIN")?,
                    fixed("NAVY_SHIP_COUNT_INTEL_MAX")?,
                    fixed("NAVY_SHIP_COUNT_INTEL_RANGE_AT_LOWEST_INTEL")?,
                ])
            })()
            .map_err(|e| e.to_string()),
            naval_region_convoy_intel: (|| {
                let fixed = |key| defines.fixed5("NIntel", key);
                Ok::<_, std::io::Error>([
                    fixed("CIVILIAN_CONVOYS_RANGE_INTEL_MIN")?,
                    fixed("CIVILIAN_MIN_INTEL_TO_SHOW_CONVOYS_GRAPH")?,
                    fixed("CIVILIAN_CONVOYS_INTEL_RANGE_AT_LOWEST_INTEL")?,
                ])
            })()
            .map_err(|e| e.to_string()),
            air_cipher: dashboard_text::cryptology::Defines::read(defines),
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
            temperature: content_reader_weather::weather::temperature_ranges(layers)
                .map_err(|error| error.to_string()),
            province_weather: content_reader_weather::weather::province_modifier_rules(layers)
                .map_err(|error| error.to_string()),
            required_rules: content_reader_map::adjacency_rules::read(layers)
                .map(RequiredRules::new)
                .map_err(|error| error.to_string()),
            owner_info_modifiers: content_reader_politics::static_modifiers::read_blocks(
                layers,
                &super::state_view::OWNER_INFO_MODIFIERS,
            )
            .map_err(|why| why.to_string())
            .and_then(|blocks| {
                let read: Vec<_> = blocks
                    .blocks
                    .into_iter()
                    .map(|block| block.map_err(|why| format!("{} in {}: {}", why.modifier, why.file, why.reason)))
                    .collect::<Result<_, _>>()?;
                <[_; 3]>::try_from(read).map_err(|_| "the state owner static modifier read is incomplete".to_owned())
            }),
            province_icon_modifiers: {
                let names = super::state_view::icon_modifier_names(native_windows);
                let keys: Vec<&str> = names.iter().map(String::as_str).collect();
                match content_reader_politics::static_modifiers::read_blocks(layers, &keys) {
                    Ok(blocks) => names
                        .iter()
                        .cloned()
                        .zip(blocks.blocks.into_iter().map(|block| block.map_err(|why| format!("{} in {}: {}", why.modifier, why.file, why.reason))))
                        .collect(),
                    Err(error) => names.iter().map(|name| (name.clone(), Err(format!("{name}: {error}")))).collect(),
                }
            },
            non_core_manpower: defines
                .fixed5("NCountry", "LOCAL_MANPOWER_ACCESSIBLE_NON_CORE_FACTOR")
                .map_err(|error| error.to_string()),
            max_shared_slots: defines.int32("NBuildings", "MAX_SHARED_SLOTS").map_err(|error| error.to_string()),
            border_events: content_reader_events::events::LazyEvents::new(Arc::clone(shared)),
            land_defines: super::defines::LandDefines::read(defines)
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
pub(crate) mod tests;

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
