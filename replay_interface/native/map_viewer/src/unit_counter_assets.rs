use crate::*;
use std::collections::{BTreeMap, BTreeSet};

pub const UNIT_COUNTER_SUBUNITICONS_GFX_SOURCE: &str = "interface/subuniticons.gfx";
pub const UNIT_COUNTER_TEXTICONS_GFX_SOURCE: &str = "interface/texticons.gfx";
pub const UNIT_COUNTER_MAPICONS_GFX_SOURCE: &str = "interface/mapicons.gfx";
pub const UNIT_COUNTER_MAPICONS_GUI_SOURCE: &str = "interface/mapicons.gui";
pub const UNIT_COUNTER_UNITVIEW_GFX_SOURCE: &str = "interface/unitview.gfx";
pub const UNIT_COUNTER_UNITVIEW_GUI_SOURCE: &str = "interface/unitview.gui";
pub const UNIT_COUNTER_DIVISION_DESIGNER_GUI_SOURCE: &str = "interface/divisiondesignerview.gui";
pub const UNIT_COUNTER_CORE_GFX_SOURCE: &str = "interface/core.gfx";
pub const UNIT_COUNTER_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
pub const UNIT_COUNTER_GAMEPLAY_DEFINES_SOURCE: &str = "common/defines/00_defines.lua";

pub const ON_MAP_DIVISION_SYMBOL_TEXTURE_PREFIX: &str = "gfx/interface/counters/divisions_small/";
pub const ON_MAP_SHIP_SYMBOL_TEXTURE_PREFIX: &str = "gfx/interface/counters/ships_small/";
pub const NATO_TEXT_ICON_TEXTURE_PREFIX: &str = "gfx/texticons/nato_unit_";

const MAP_COUNTER_SPRITE_NAMES: &[&str] = &[
    "GFX_onmap_unit_counter_overlay",
    "GFX_onmap_unit_counter_half_selected",
    "GFX_onmap_unit_counter_selected",
    "GFX_onmap_unit_counter_active_ability",
    "GFX_onmap_unit_counter_half",
    "GFX_onmap_unit_counter",
    "GFX_onmap_unit_counter_empty",
    "GFX_onmap_unit_counter_overlay_navy",
    "GFX_onmap_mission_unit_counter",
    "GFX_onmap_naval_unit_counter",
    "GFX_onmap_mission_unit_counter_selected",
    "GFX_onmap_mission_unit_counter_types",
    "GFX_onmap_unit_counter_ideology",
    "GFX_mapicon_unit_bar_org",
    "GFX_mapicon_unit_bar_str",
    "GFX_mapicon_unit_frame_strip",
    "GFX_mapicon_unit_types_strip",
    "GFX_mapicon_unit_no_orders",
    "GFX_mapicon_unit_area_defense",
    "GFX_mapicon_unit_invalid_orders",
    "GFX_onmap_digin_icon",
    "GFX_onmap_training_icon",
    "GFX_onmap_action_icon",
    "GFX_unit_attrition",
    "GFX_mapicon_tiny_unit_counter",
];

const UNITVIEW_COUNTER_SPRITE_NAMES: &[&str] = &[
    "GFX_unitlist_priority_icon_strip",
    "GFX_unitcounter_supply_status_strip",
    "GFX_unit_level",
    "GFX_div_count_bg",
    "GFX_group_div_count_bg",
    "GFX_army_badge_bar_org_median",
    "GFX_army_badge_bar_str_median",
];

#[derive(Clone, Debug, PartialEq)]
pub struct UnitCounterAssetManifest {
    pub on_map_division_symbols: Vec<CounterSpriteAsset>,
    pub unresolved_on_map_division_symbols: Vec<UnresolvedSpriteAsset>,
    pub on_map_ship_symbols: Vec<CounterSpriteAsset>,
    pub nato_text_icons: Vec<CounterSpriteAsset>,
    pub map_counter_sprites: BTreeMap<String, CounterSpriteAsset>,
    pub map_counter_code_textured_sprites: Vec<RawSpriteDef>,
    pub unitview_sprites: BTreeMap<String, CounterSpriteAsset>,
    pub army_badge_progress_bars: BTreeMap<String, ProgressBarAsset>,
    pub gui_layouts: UnitCounterGuiLayouts,
    pub division_template_layout: DivisionTemplateCounterLayout,
    pub fonts: Vec<BitmapFontBinding>,
}

impl UnitCounterAssetManifest {
    pub fn division_symbol_definition_count(&self) -> usize {
        self.on_map_division_symbols.len() + self.unresolved_on_map_division_symbols.len()
    }

    pub fn division_symbol_by_name(&self, name: &str) -> Option<&CounterSpriteAsset> {
        self.on_map_division_symbols
            .iter()
            .find(|symbol| symbol.name == name)
    }

    pub fn unique_division_symbol_name_count(&self) -> usize {
        let mut values = self
            .on_map_division_symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<BTreeSet<_>>();
        values.extend(
            self.unresolved_on_map_division_symbols
                .iter()
                .map(|symbol| symbol.name.as_str()),
        );
        values.len()
    }

    pub fn unique_division_symbol_texture_count(&self) -> usize {
        let mut values = self
            .on_map_division_symbols
            .iter()
            .map(|symbol| symbol.texture_file.as_str())
            .collect::<BTreeSet<_>>();
        values.extend(
            self.unresolved_on_map_division_symbols
                .iter()
                .map(|symbol| symbol.texture_file.as_str()),
        );
        values.len()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RawSpriteDef {
    pub kind: String,
    pub name: String,
    pub texture_file: Option<String>,
    pub no_of_frames: u32,
    pub frame_direction_y: bool,
    pub source_file: String,
    pub source_line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CounterSpriteAsset {
    pub name: String,
    pub texture_file: String,
    pub no_of_frames: u32,
    pub frame_direction_y: bool,
    pub texture_width: u32,
    pub texture_height: u32,
    pub dds_format: String,
    pub mip_count: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frames: Vec<AtlasFrame>,
    pub source_file: String,
    pub source_line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CounterSpriteInventory {
    pub resolved: Vec<CounterSpriteAsset>,
    pub unresolved: Vec<UnresolvedSpriteAsset>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnresolvedSpriteAsset {
    pub name: String,
    pub texture_file: String,
    pub no_of_frames: u32,
    pub source_file: String,
    pub source_line: usize,
    pub error: String,
}

impl CounterSpriteAsset {
    pub fn frame(&self, index: u32) -> Option<&AtlasFrame> {
        self.frames.get(index as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasFrame {
    pub index: u32,
    pub rect: PixelRect,
    pub uv: UvRect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvRect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProgressBarAsset {
    pub name: String,
    pub texture_file1: String,
    pub texture_file2: String,
    pub effect_file: Option<String>,
    pub horizontal: Option<bool>,
    pub size: Option<GuiSize>,
    pub source_file: String,
    pub source_line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnitCounterGuiLayouts {
    pub zoomed_in_offset: GuiPoint,
    pub land: GuiContainerLayout,
    pub navy: GuiContainerLayout,
    pub railway_gun: GuiContainerLayout,
    pub air_wing: GuiContainerLayout,
    pub active_ability_item: Option<GuiContainerLayout>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GuiContainerLayout {
    pub name: String,
    pub source_file: String,
    pub source_line: usize,
    pub position: Option<GuiPoint>,
    pub size: Option<GuiSize>,
    pub controls: Vec<GuiControlLayout>,
}

impl GuiContainerLayout {
    pub fn control(&self, name: &str) -> Option<&GuiControlLayout> {
        self.controls.iter().find(|control| control.name == name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GuiControlLayout {
    pub kind: String,
    pub name: String,
    pub source_file: String,
    pub source_line: usize,
    pub sprite_type: Option<String>,
    pub quad_texture_sprite: Option<String>,
    pub parent_size: Option<GuiSize>,
    pub position: Option<GuiPoint>,
    pub effective_position_f32_bits: Option<[u32; 2]>,
    pub size: Option<GuiSize>,
    pub slot_size: Option<GuiSize>,
    pub max_width: Option<i32>,
    pub max_height: Option<i32>,
    pub font: Option<String>,
    pub format: Option<String>,
    pub orientation: Option<String>,
    pub max_slots_vertical: Option<u32>,
    pub rotation: Option<f32>,
    pub scale: Option<f32>,
    pub frame: Option<u32>,
    pub center_position: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuiPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuiSize {
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DivisionTemplateCounterLayout {
    pub custom_icon_button: GuiContainerLayout,
    pub custom_icon_inner_button: GuiControlLayout,
    pub non_hq_symbol_button: GuiControlLayout,
    pub hq_symbol_button: GuiControlLayout,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BitmapFontBinding {
    pub is_override: bool,
    pub name: String,
    pub path: Option<String>,
    pub font_files: Vec<String>,
    pub languages: Vec<String>,
    pub source_file: String,
    pub source_line: usize,
}

#[derive(Clone, Debug)]
struct BracedBlock {
    kind: String,
    body: String,
    source_line: usize,
}

pub fn unit_counter_assets_from_resolver(
    resolver: &GameFileResolver,
) -> Result<UnitCounterAssetManifest> {
    let division_inventory = on_map_division_counter_inventory_from_resolver(resolver)?;
    Ok(UnitCounterAssetManifest {
        on_map_division_symbols: division_inventory.resolved,
        unresolved_on_map_division_symbols: division_inventory.unresolved,
        on_map_ship_symbols: on_map_ship_counter_sprites_from_resolver(resolver)?,
        nato_text_icons: nato_text_icon_sprites_from_resolver(resolver)?,
        map_counter_sprites: map_counter_sprites_from_resolver(resolver)?,
        map_counter_code_textured_sprites: map_counter_code_textured_sprites_from_resolver(
            resolver,
        )?,
        unitview_sprites: unitview_counter_sprites_from_resolver(resolver)?,
        army_badge_progress_bars: army_badge_progress_bars_from_resolver(resolver)?,
        gui_layouts: unit_counter_gui_layouts_from_resolver(resolver)?,
        division_template_layout: division_template_counter_layout_from_resolver(resolver)?,
        fonts: counter_fonts_from_resolver(resolver)?,
    })
}

pub fn on_map_division_counter_sprites_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<CounterSpriteAsset>> {
    Ok(on_map_division_counter_inventory_from_resolver(resolver)?.resolved)
}

pub fn on_map_division_counter_inventory_from_resolver(
    resolver: &GameFileResolver,
) -> Result<CounterSpriteInventory> {
    sprite_inventory_from_resolver(resolver, UNIT_COUNTER_SUBUNITICONS_GFX_SOURCE, |def| {
        def.name.ends_with("_medium_white")
            && def
                .texture_file
                .as_deref()
                .map(|texture| {
                    normalize_rel_path(texture).starts_with(ON_MAP_DIVISION_SYMBOL_TEXTURE_PREFIX)
                })
                .unwrap_or(false)
    })
}

pub fn on_map_ship_counter_sprites_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<CounterSpriteAsset>> {
    sprites_from_resolver(resolver, UNIT_COUNTER_SUBUNITICONS_GFX_SOURCE, |def| {
        (def.name.ends_with("_medium_white") || def.name.ends_with("_medium_black"))
            && def
                .texture_file
                .as_deref()
                .map(|texture| {
                    normalize_rel_path(texture).starts_with(ON_MAP_SHIP_SYMBOL_TEXTURE_PREFIX)
                })
                .unwrap_or(false)
    })
}

pub fn nato_text_icon_sprites_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<CounterSpriteAsset>> {
    sprites_from_resolver(resolver, UNIT_COUNTER_TEXTICONS_GFX_SOURCE, |def| {
        def.name.starts_with("GFX_nato_unit_")
            && def
                .texture_file
                .as_deref()
                .map(|texture| {
                    normalize_rel_path(texture).starts_with(NATO_TEXT_ICON_TEXTURE_PREFIX)
                })
                .unwrap_or(false)
    })
}

pub fn map_counter_sprites_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, CounterSpriteAsset>> {
    let wanted = MAP_COUNTER_SPRITE_NAMES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let assets = sprites_from_resolver(resolver, UNIT_COUNTER_MAPICONS_GFX_SOURCE, |def| {
        wanted.contains(def.name.as_str()) && def.texture_file.is_some()
    })?;
    Ok(assets
        .into_iter()
        .map(|asset| (asset.name.clone(), asset))
        .collect())
}

pub fn map_counter_code_textured_sprites_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<RawSpriteDef>> {
    let path = resolver.resolve_required(UNIT_COUNTER_MAPICONS_GFX_SOURCE, "map icons gfx")?;
    let text = read_latin1(&path)?;
    Ok(
        parse_sprite_defs_exact(&text, UNIT_COUNTER_MAPICONS_GFX_SOURCE)?
            .into_iter()
            .filter(|def| {
                def.texture_file.is_none()
                    && matches!(
                        def.name.as_str(),
                        "GFX_mapicon_unit_large_flag_stripe"
                            | "GFX_mapicon_unit_flag_stripe"
                            | "GFX_mapicon_unit_flag_stripe_visible"
                    )
            })
            .collect(),
    )
}

pub fn unitview_counter_sprites_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, CounterSpriteAsset>> {
    let wanted = UNITVIEW_COUNTER_SPRITE_NAMES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let assets = sprites_from_resolver(resolver, UNIT_COUNTER_UNITVIEW_GFX_SOURCE, |def| {
        wanted.contains(def.name.as_str()) && def.texture_file.is_some()
    })?;
    Ok(assets
        .into_iter()
        .map(|asset| (asset.name.clone(), asset))
        .collect())
}

pub fn army_badge_progress_bars_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, ProgressBarAsset>> {
    let path = resolver.resolve_required(UNIT_COUNTER_UNITVIEW_GFX_SOURCE, "unit view gfx")?;
    let text = read_latin1(&path)?;
    let wanted = [
        "GFX_army_badge_bar_org",
        "GFX_army_badge_bar_str",
        "GFX_army_badge_bar_org_median",
        "GFX_army_badge_bar_str_median",
    ]
    .iter()
    .copied()
    .collect::<BTreeSet<_>>();
    Ok(
        parse_progress_bar_assets(&text, UNIT_COUNTER_UNITVIEW_GFX_SOURCE)
            .into_iter()
            .filter(|asset| wanted.contains(asset.name.as_str()))
            .map(|asset| (asset.name.clone(), asset))
            .collect(),
    )
}

pub fn unit_counter_gui_layouts_from_resolver(
    resolver: &GameFileResolver,
) -> Result<UnitCounterGuiLayouts> {
    let path = resolver.resolve_required(UNIT_COUNTER_MAPICONS_GUI_SOURCE, "map icons gui")?;
    let text = read_latin1(&path)?;
    Ok(UnitCounterGuiLayouts {
        zoomed_in_offset: position_type_from_text(
            &text,
            "unit_counter_zoomed_in_offset",
            UNIT_COUNTER_MAPICONS_GUI_SOURCE,
        )?,
        land: exact_counter_container_layout_from_text(
            &text,
            "unit_counter",
            UNIT_COUNTER_MAPICONS_GUI_SOURCE,
        )?,
        navy: exact_counter_container_layout_from_text(
            &text,
            "unit_counter_navy",
            UNIT_COUNTER_MAPICONS_GUI_SOURCE,
        )?,
        railway_gun: exact_counter_container_layout_from_text(
            &text,
            "unit_counter_railway_gun",
            UNIT_COUNTER_MAPICONS_GUI_SOURCE,
        )?,
        air_wing: exact_counter_container_layout_from_text(
            &text,
            "air_wing_counter",
            UNIT_COUNTER_MAPICONS_GUI_SOURCE,
        )?,
        active_ability_item: optional_container_layout_from_text_with_all_named_controls(
            &text,
            "active_ability_item",
            UNIT_COUNTER_MAPICONS_GUI_SOURCE,
        )?,
    })
}

pub fn division_template_counter_layout_from_resolver(
    resolver: &GameFileResolver,
) -> Result<DivisionTemplateCounterLayout> {
    let path = resolver.resolve_required(
        UNIT_COUNTER_DIVISION_DESIGNER_GUI_SOURCE,
        "division designer gui",
    )?;
    let text = read_latin1(&path)?;
    let custom_icon_button = container_layout_from_text(
        &text,
        "div_templ_custom_icon_button",
        UNIT_COUNTER_DIVISION_DESIGNER_GUI_SOURCE,
    )?;
    let custom_icon_inner_button = custom_icon_button
        .control("btn")
        .cloned()
        .ok_or("missing div_templ_custom_icon_button/btn")?;
    let mut symbol_buttons = named_controls_from_text(
        &text,
        "div_templ_symbol_button",
        UNIT_COUNTER_DIVISION_DESIGNER_GUI_SOURCE,
    )
    .into_iter()
    .filter(|control| control.kind.eq_ignore_ascii_case("buttontype"))
    .collect::<Vec<_>>();
    if symbol_buttons.len() < 2 {
        return Err("missing division template symbol button variants".into());
    }
    symbol_buttons.sort_by_key(|control| {
        control
            .position
            .map(|point| (point.y, point.x))
            .unwrap_or((i32::MAX, i32::MAX))
    });
    Ok(DivisionTemplateCounterLayout {
        custom_icon_button,
        custom_icon_inner_button,
        non_hq_symbol_button: symbol_buttons[0].clone(),
        hq_symbol_button: symbol_buttons[1].clone(),
    })
}

pub fn counter_fonts_from_resolver(resolver: &GameFileResolver) -> Result<Vec<BitmapFontBinding>> {
    let path = resolver.resolve_required(UNIT_COUNTER_CORE_GFX_SOURCE, "core gfx")?;
    let text = read_latin1(&path)?;
    Ok(parse_bitmap_fonts(&text, UNIT_COUNTER_CORE_GFX_SOURCE))
}

pub fn parse_sprite_defs(text: &str, source_file: &str) -> Vec<RawSpriteDef> {
    braced_blocks(text)
        .into_iter()
        .filter(|block| {
            block.kind.eq_ignore_ascii_case("spriteType")
                || block.kind.eq_ignore_ascii_case("frameAnimatedSpriteType")
        })
        .filter_map(|block| {
            let name = parse_value_ci(&block.body, "name")?;
            let texture_file = parse_value_ci(&block.body, "textureFile");
            Some(RawSpriteDef {
                kind: block.kind,
                name,
                texture_file,
                no_of_frames: parse_u32_ci(&block.body, "noOfFrames").unwrap_or(1),
                frame_direction_y: parse_bool_ci(&block.body, "framedirectiony").unwrap_or(false),
                source_file: source_file.to_string(),
                source_line: block.source_line,
            })
        })
        .collect()
}

pub(crate) fn parse_sprite_defs_exact(text: &str, source_file: &str) -> Result<Vec<RawSpriteDef>> {
    braced_blocks(text)
        .into_iter()
        .filter(|block| {
            block.kind.eq_ignore_ascii_case("spriteType")
                || block.kind.eq_ignore_ascii_case("frameAnimatedSpriteType")
        })
        .map(|block| {
            let name = exact_present_value_ci(&block.body, "name")?.ok_or_else(|| {
                format!(
                    "{}:{} sprite definition lacks name",
                    source_file, block.source_line
                )
            })?;
            let texture_file = exact_present_value_ci(&block.body, "textureFile")?;
            let no_of_frames =
                if let Some(index) = find_direct_assignment_ci(&block.body, "noOfFrames") {
                    let value = parse_exact_u32_at(&block.body[index..]).ok_or_else(|| {
                        format!(
                            "{}:{} noOfFrames is not an exact u32 token",
                            source_file, block.source_line
                        )
                    })?;
                    if value == 0 {
                        return Err(format!(
                            "{}:{} noOfFrames must be positive",
                            source_file, block.source_line
                        )
                        .into());
                    }
                    value
                } else {
                    1
                };
            let frame_direction_y =
                if find_direct_assignment_ci(&block.body, "framedirectiony").is_some() {
                    parse_bool_ci(&block.body, "framedirectiony").ok_or_else(|| {
                        format!(
                            "{}:{} framedirectiony is not an exact boolean token",
                            source_file, block.source_line
                        )
                    })?
                } else {
                    false
                };
            Ok(RawSpriteDef {
                kind: block.kind,
                name,
                texture_file,
                no_of_frames,
                frame_direction_y,
                source_file: source_file.to_string(),
                source_line: block.source_line,
            })
        })
        .collect()
}

fn exact_present_value_ci(text: &str, key: &str) -> Result<Option<String>> {
    if find_direct_assignment_ci(text, key).is_none() {
        return Ok(None);
    }
    parse_direct_value_ci(text, key)
        .filter(|value| !value.is_empty())
        .map(Some)
        .ok_or_else(|| format!("GFX {key} assignment has no complete scalar value").into())
}

pub(crate) fn masked_shield_definition(
    text: &str,
    wanted_name: &str,
) -> Option<(String, String, String)> {
    masked_shield_definitions(text, wanted_name)
        .into_iter()
        .next()
}

pub(crate) fn masked_shield_definitions(
    text: &str,
    wanted_name: &str,
) -> Vec<(String, String, String)> {
    braced_blocks(text)
        .into_iter()
        .filter(|block| block.kind.eq_ignore_ascii_case("maskedShieldType"))
        .filter_map(|block| {
            if parse_value_ci(&block.body, "name")? != wanted_name {
                return None;
            }
            Some((
                normalize_rel_path(&parse_value_ci(&block.body, "textureFile1")?),
                normalize_rel_path(&parse_value_ci(&block.body, "textureFile2")?),
                normalize_rel_path(&parse_value_ci(&block.body, "effectFile")?),
            ))
        })
        .collect()
}

pub(crate) fn sprites_from_resolver<F>(
    resolver: &GameFileResolver,
    source_file: &str,
    keep: F,
) -> Result<Vec<CounterSpriteAsset>>
where
    F: Fn(&RawSpriteDef) -> bool,
{
    let path = resolver.resolve_required(source_file, source_file)?;
    let text = read_latin1(&path)?;
    let mut assets = Vec::new();
    for def in parse_sprite_defs_exact(&text, source_file)?
        .into_iter()
        .filter(|def| keep(def))
    {
        let Some(texture_file) = def.texture_file.clone() else {
            continue;
        };
        assets.push(sprite_asset_from_def(resolver, def, texture_file)?);
    }
    Ok(assets)
}

pub(crate) fn sprite_inventory_from_resolver<F>(
    resolver: &GameFileResolver,
    source_file: &str,
    keep: F,
) -> Result<CounterSpriteInventory>
where
    F: Fn(&RawSpriteDef) -> bool,
{
    let path = resolver.resolve_required(source_file, source_file)?;
    let text = read_latin1(&path)?;
    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    for def in parse_sprite_defs_exact(&text, source_file)?
        .into_iter()
        .filter(|def| keep(def))
    {
        let Some(texture_file) = def.texture_file.clone() else {
            continue;
        };
        match sprite_asset_from_def(resolver, def.clone(), texture_file.clone()) {
            Ok(asset) => resolved.push(asset),
            Err(err) => unresolved.push(UnresolvedSpriteAsset {
                name: def.name,
                texture_file: normalize_rel_path(&texture_file),
                no_of_frames: def.no_of_frames,
                source_file: def.source_file,
                source_line: def.source_line,
                error: err.to_string(),
            }),
        }
    }
    Ok(CounterSpriteInventory {
        resolved,
        unresolved,
    })
}

pub(crate) fn sprite_asset_from_def(
    resolver: &GameFileResolver,
    def: RawSpriteDef,
    texture_file: String,
) -> Result<CounterSpriteAsset> {
    let texture_file = normalize_rel_path(&texture_file);
    let path = resolver.resolve_required(&texture_file, &def.name)?;
    let info = dds_info(&path)?;
    let width = info.dimensions.width as u32;
    let height = info.dimensions.height as u32;
    let no_of_frames = def.no_of_frames.max(1);
    let (frame_width, frame_height) = if def.frame_direction_y {
        if height % no_of_frames != 0 {
            return Err(format!(
                "{} height {} is not divisible by noOfFrames {} for {}",
                texture_file, height, no_of_frames, def.name
            )
            .into());
        }
        (width, height / no_of_frames)
    } else {
        if width % no_of_frames != 0 {
            return Err(format!(
                "{} width {} is not divisible by noOfFrames {} for {}",
                texture_file, width, no_of_frames, def.name
            )
            .into());
        }
        (width / no_of_frames, height)
    };
    let frames = (0..no_of_frames)
        .map(|index| {
            let rect = if def.frame_direction_y {
                PixelRect {
                    x: 0,
                    y: index * frame_height,
                    width: frame_width,
                    height: frame_height,
                }
            } else {
                PixelRect {
                    x: index * frame_width,
                    y: 0,
                    width: frame_width,
                    height: frame_height,
                }
            };
            AtlasFrame {
                index,
                rect,
                uv: UvRect {
                    u0: rect.x as f32 / width as f32,
                    v0: rect.y as f32 / height as f32,
                    u1: (rect.x + rect.width) as f32 / width as f32,
                    v1: (rect.y + rect.height) as f32 / height as f32,
                },
            }
        })
        .collect::<Vec<_>>();
    Ok(CounterSpriteAsset {
        name: def.name,
        texture_file,
        no_of_frames,
        frame_direction_y: def.frame_direction_y,
        texture_width: width,
        texture_height: height,
        dds_format: info.format,
        mip_count: info.mip_count,
        frame_width,
        frame_height,
        frames,
        source_file: def.source_file,
        source_line: def.source_line,
    })
}

pub(crate) fn parse_progress_bar_assets(text: &str, source_file: &str) -> Vec<ProgressBarAsset> {
    braced_blocks(text)
        .into_iter()
        .filter(|block| block.kind.eq_ignore_ascii_case("progressbartype"))
        .filter_map(|block| {
            Some(ProgressBarAsset {
                name: parse_value_ci(&block.body, "name")?,
                texture_file1: normalize_rel_path(&parse_value_ci(&block.body, "textureFile1")?),
                texture_file2: normalize_rel_path(&parse_value_ci(&block.body, "textureFile2")?),
                effect_file: parse_value_ci(&block.body, "effectFile")
                    .map(|value| normalize_rel_path(&value)),
                horizontal: parse_bool_ci(&block.body, "horizontal"),
                size: parse_gui_size_ci(&block.body, "size"),
                source_file: source_file.to_string(),
                source_line: block.source_line,
            })
        })
        .collect()
}

pub(crate) fn parse_bitmap_fonts(text: &str, source_file: &str) -> Vec<BitmapFontBinding> {
    braced_blocks(text)
        .into_iter()
        .filter(|block| {
            block.kind.eq_ignore_ascii_case("bitmapfont")
                || block.kind.eq_ignore_ascii_case("bitmapfont_override")
        })
        .filter_map(|block| {
            Some(BitmapFontBinding {
                is_override: block.kind.eq_ignore_ascii_case("bitmapfont_override"),
                name: parse_value_ci(&block.body, "name")?,
                path: parse_value_ci(&block.body, "path"),
                font_files: parse_quoted_values_from_child_block(&block.body, "fontfiles"),
                languages: parse_quoted_values_from_child_block(&block.body, "languages"),
                source_file: source_file.to_string(),
                source_line: block.source_line,
            })
        })
        .collect()
}

fn position_type_from_text(text: &str, name: &str, source_file: &str) -> Result<GuiPoint> {
    let block = named_block_from_text(text, name, "positionType")
        .ok_or_else(|| format!("missing positionType {name} in {source_file}"))?;
    validate_gui_numeric_fields(&block.body, None)?;
    parse_gui_point_ci(&block.body, "position")
        .ok_or_else(|| format!("missing position for {name} in {source_file}").into())
}

pub(crate) fn container_layout_from_text(
    text: &str,
    name: &str,
    source_file: &str,
) -> Result<GuiContainerLayout> {
    container_layout_from_text_with_numeric_policy(text, name, source_file, false)
}

fn exact_counter_container_layout_from_text(
    text: &str,
    name: &str,
    source_file: &str,
) -> Result<GuiContainerLayout> {
    container_layout_from_text_with_numeric_policy(text, name, source_file, true)
}

fn container_layout_from_text_with_numeric_policy(
    text: &str,
    name: &str,
    source_file: &str,
    exact_numeric: bool,
) -> Result<GuiContainerLayout> {
    let block = named_block_from_text(text, name, "containerWindowType")
        .ok_or_else(|| format!("missing container {name} in {source_file}"))?;
    if exact_numeric {
        validate_gui_numeric_fields(&block.body, None)?;
    }
    let position = parse_gui_point_ci(&block.body, "position");
    let size = parse_gui_size_ci(&block.body, "size");
    Ok(GuiContainerLayout {
        name: name.to_string(),
        source_file: source_file.to_string(),
        source_line: block.source_line,
        position,
        size,
        controls: all_effective_controls_from_text(&block.body, source_file, size, exact_numeric)?,
    })
}

fn optional_container_layout_from_text_with_all_named_controls(
    text: &str,
    name: &str,
    source_file: &str,
) -> Result<Option<GuiContainerLayout>> {
    let Some(block) = named_block_from_text(text, name, "containerWindowType") else {
        return Ok(None);
    };
    validate_gui_numeric_fields(&block.body, None)?;
    let position = parse_gui_point_ci(&block.body, "position");
    let size = parse_gui_size_ci(&block.body, "size");
    Ok(Some(GuiContainerLayout {
        name: name.to_string(),
        source_file: source_file.to_string(),
        source_line: block.source_line,
        position,
        size,
        controls: all_effective_controls_from_text(&block.body, source_file, size, true)?,
    }))
}

fn gui_control_from_block(block: BracedBlock, source_file: &str) -> Option<GuiControlLayout> {
    gui_control_from_block_with_name_policy(block, source_file, false)
}

fn gui_control_from_template_block(
    block: BracedBlock,
    source_file: &str,
) -> Option<GuiControlLayout> {
    gui_control_from_block_with_name_policy(block, source_file, true)
}

fn gui_control_from_block_with_name_policy(
    block: BracedBlock,
    source_file: &str,
    retain_unnamed_template_controls: bool,
) -> Option<GuiControlLayout> {
    let name = parse_value_ci(&block.body, "name");
    let kind_lower = block.kind.to_ascii_lowercase();
    let unnamed_template_control = retain_unnamed_template_controls
        && name.is_none()
        && (parse_value_ci(&block.body, "spriteType").is_some()
            || parse_value_ci(&block.body, "quadTextureSprite").is_some()
            || parse_value_ci(&block.body, "font").is_some()
            || kind_lower.ends_with("type")
            || kind_lower == "background");
    if name.is_none() && !unnamed_template_control {
        return None;
    }
    Some(GuiControlLayout {
        name: name.unwrap_or_default(),
        kind: block.kind,
        source_file: source_file.to_string(),
        source_line: block.source_line,
        sprite_type: parse_value_ci(&block.body, "spriteType"),
        quad_texture_sprite: parse_value_ci(&block.body, "quadTextureSprite"),
        parent_size: None,
        position: parse_gui_point_ci(&block.body, "position"),
        effective_position_f32_bits: None,
        size: parse_gui_size_ci(&block.body, "size"),
        slot_size: parse_gui_size_ci(&block.body, "slotsize"),
        max_width: parse_i32_ci(&block.body, "maxWidth"),
        max_height: parse_i32_ci(&block.body, "maxHeight"),
        font: parse_value_ci(&block.body, "font"),
        format: parse_value_ci(&block.body, "format"),
        orientation: parse_value_ci(&block.body, "orientation"),
        max_slots_vertical: parse_u32_ci(&block.body, "max_slots_vertical"),
        rotation: parse_f32_ci(&block.body, "rotation"),
        scale: parse_f32_ci(&block.body, "scale"),
        frame: parse_u32_ci(&block.body, "frame"),
        center_position: parse_bool_ci(&block.body, "centerPosition"),
    })
}

fn all_effective_controls_from_text(
    text: &str,
    source_file: &str,
    container_size: Option<GuiSize>,
    exact_numeric: bool,
) -> Result<Vec<GuiControlLayout>> {
    let mut controls = Vec::new();
    collect_effective_controls(
        text,
        source_file,
        GuiPoint { x: 0, y: 0 },
        [0.0, 0.0],
        container_size,
        exact_numeric,
        &mut controls,
    )?;
    Ok(controls)
}

fn collect_effective_controls(
    text: &str,
    source_file: &str,
    parent_position: GuiPoint,
    parent_effective_position: [f32; 2],
    parent_size: Option<GuiSize>,
    exact_numeric: bool,
    controls: &mut Vec<GuiControlLayout>,
) -> Result<()> {
    for block in direct_braced_blocks(text) {
        if exact_numeric {
            validate_gui_numeric_fields(&block.body, parent_size)?;
        }
        let mut child_parent_position = parent_position;
        let mut child_effective_position = parent_effective_position;
        let mut child_parent_size = parent_size;
        if let Some(mut control) = gui_control_from_template_block(block.clone(), source_file) {
            control.parent_size = parent_size;
            if exact_numeric {
                control.size = parse_exact_gui_size_ci(&block.body, "size", parent_size);
                control.slot_size = parse_exact_gui_size_ci(&block.body, "slotsize", parent_size);
            }
            let local = control.position.unwrap_or(GuiPoint { x: 0, y: 0 });
            control.position = Some(accumulated_gui_position(parent_position, Some(local))?);
            let effective_position = [
                parent_effective_position[0] + local.x as f32,
                parent_effective_position[1] + local.y as f32,
            ];
            control.effective_position_f32_bits = Some([
                effective_position[0].to_bits(),
                effective_position[1].to_bits(),
            ]);
            child_parent_position = control.position.expect("effective GUI position");
            child_parent_size = control.size.or(parent_size);
            if contains_effective_control(&block.body) {
                child_effective_position = oriented_child_origin(
                    &control,
                    effective_position,
                    child_parent_size,
                    exact_numeric,
                )?;
            } else {
                child_effective_position = effective_position;
            }
            controls.push(control);
        }
        collect_effective_controls(
            &block.body,
            source_file,
            child_parent_position,
            child_effective_position,
            child_parent_size,
            exact_numeric,
            controls,
        )?;
    }
    Ok(())
}

fn contains_effective_control(text: &str) -> bool {
    direct_braced_blocks(text).into_iter().any(|block| {
        gui_control_from_template_block(block.clone(), "").is_some()
            || contains_effective_control(&block.body)
    })
}

fn oriented_child_origin(
    control: &GuiControlLayout,
    base: [f32; 2],
    child_size: Option<GuiSize>,
    exact_numeric: bool,
) -> Result<[f32; 2]> {
    match control.orientation.as_deref() {
        None | Some("upper_left") => Ok(base),
        Some(orientation) if orientation.eq_ignore_ascii_case("upper_left") => Ok(base),
        Some(orientation) if orientation.eq_ignore_ascii_case("center") => {
            let parent = control
                .parent_size
                .ok_or("centered GUI ancestor lacks an effective parent size")?;
            let child = child_size.ok_or("centered GUI ancestor lacks an effective size")?;
            if parent.width <= 0 || parent.height <= 0 || child.width <= 0 || child.height <= 0 {
                return Err("centered GUI ancestor has a nonpositive extent".into());
            }
            Ok([
                base[0] + (parent.width as f32 - child.width as f32) * 0.5,
                base[1] + (parent.height as f32 - child.height as f32) * 0.5,
            ])
        }
        Some(_) if exact_numeric => Err("GUI ancestor orientation is unsupported".into()),
        Some(_) => Ok(base),
    }
}

fn accumulated_gui_position(parent: GuiPoint, local: Option<GuiPoint>) -> Result<GuiPoint> {
    let local = local.unwrap_or(GuiPoint { x: 0, y: 0 });
    Ok(GuiPoint {
        x: parent
            .x
            .checked_add(local.x)
            .ok_or("GUI nested x position overflows i32")?,
        y: parent
            .y
            .checked_add(local.y)
            .ok_or("GUI nested y position overflows i32")?,
    })
}

fn named_controls_from_text(text: &str, name: &str, source_file: &str) -> Vec<GuiControlLayout> {
    braced_blocks(text)
        .into_iter()
        .filter(|block| {
            matches!(
                block.kind.to_ascii_lowercase().as_str(),
                "buttontype"
                    | "icontype"
                    | "instanttextboxtype"
                    | "gridboxtype"
                    | "containerwindowtype"
                    | "positiontype"
            )
        })
        .filter_map(|block| {
            let control = gui_control_from_block(block, source_file)?;
            if !name.is_empty() && control.name != name {
                return None;
            }
            Some(control)
        })
        .collect()
}

fn named_block_from_text(text: &str, name: &str, expected_kind: &str) -> Option<BracedBlock> {
    braced_blocks(text).into_iter().find(|block| {
        block.kind.eq_ignore_ascii_case(expected_kind)
            && parse_direct_value_ci(&block.body, "name").as_deref() == Some(name)
    })
}

fn braced_blocks(text: &str) -> Vec<BracedBlock> {
    let stripped = strip_gui_comments_quote_aware(text);
    let bytes = stripped.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    while i < bytes.len() {
        if quoted {
            if escaped {
                escaped = false;
            } else if bytes[i] == b'\\' {
                escaped = true;
            } else if bytes[i] == b'"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            quoted = true;
            i += 1;
            continue;
        }
        if !is_word_byte(bytes[i]) {
            i += 1;
            continue;
        }
        let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
        if !before_ok {
            i += 1;
            continue;
        }
        let kind_start = i;
        while i < bytes.len() && is_word_byte(bytes[i]) {
            i += 1;
        }
        let kind = stripped[kind_start..i].to_string();
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'=' {
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'{' {
            continue;
        }
        let brace = j;
        if let Some(end) = matching_brace(&stripped, brace) {
            out.push(BracedBlock {
                kind,
                body: stripped[brace + 1..end].to_string(),
                source_line: line_number_at(&stripped, kind_start),
            });
            i = brace + 1;
        } else {
            break;
        }
    }
    out
}

fn direct_braced_blocks(text: &str) -> Vec<BracedBlock> {
    let stripped = strip_gui_comments_quote_aware(text);
    let bytes = stripped.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    while i < bytes.len() {
        if quoted {
            if escaped {
                escaped = false;
            } else if bytes[i] == b'\\' {
                escaped = true;
            } else if bytes[i] == b'"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            quoted = true;
            i += 1;
            continue;
        }
        if !is_word_byte(bytes[i]) {
            i += 1;
            continue;
        }
        if i != 0 && is_word_byte(bytes[i - 1]) {
            i += 1;
            continue;
        }
        let kind_start = i;
        while i < bytes.len() && is_word_byte(bytes[i]) {
            i += 1;
        }
        let kind = stripped[kind_start..i].to_string();
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'=' {
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'{' {
            continue;
        }
        let Some(end) = matching_brace(&stripped, j) else {
            break;
        };
        out.push(BracedBlock {
            kind,
            body: stripped[j + 1..end].to_string(),
            source_line: line_number_at(&stripped, kind_start),
        });
        i = end + 1;
    }
    out
}

fn matching_brace(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut quoted = false;
    let mut escaped = false;
    let mut i = open;
    while i < bytes.len() {
        if quoted {
            if escaped {
                escaped = false;
            } else if bytes[i] == b'\\' {
                escaped = true;
            } else if bytes[i] == b'"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        match bytes[i] {
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn validate_gui_numeric_fields(text: &str, parent_size: Option<GuiSize>) -> Result<()> {
    for key in ["maxWidth", "maxHeight"] {
        if let Some(index) = find_direct_assignment_ci(text, key) {
            if parse_i32_at(&text[index..]).is_none() {
                return Err(format!("GUI {key} is not an exact i32 token").into());
            }
        }
    }
    for key in ["max_slots_vertical", "frame"] {
        if let Some(index) = find_direct_assignment_ci(text, key) {
            if parse_exact_u32_at(&text[index..]).is_none() {
                return Err(format!("GUI {key} is not an exact u32 token").into());
            }
        }
    }
    for key in ["rotation", "scale"] {
        if let Some(index) = find_direct_assignment_ci(text, key) {
            if parse_f32_at(&text[index..]).is_none_or(|value| !value.is_finite()) {
                return Err(format!("GUI {key} is not an exact finite f32 token").into());
            }
        }
    }
    for key in ["position", "size", "slotsize"] {
        let Some(_) = find_direct_assignment_ci(text, key) else {
            continue;
        };
        let body = child_block_ci(text, key)
            .ok_or_else(|| format!("GUI {key} assignment is not a complete child block"))?;
        let coordinates: &[&str] = if key == "position" {
            &["x", "y"]
        } else {
            &["width", "x", "height", "y"]
        };
        for coordinate in coordinates {
            let Some(index) = find_direct_assignment_ci(&body, coordinate) else {
                continue;
            };
            let exact_integer = parse_i32_at(&body[index..]).is_some();
            let inherited_hundred_percent = key != "position"
                && parent_size.is_some()
                && parse_exact_hundred_percent_at(&body[index..]);
            if !exact_integer && !inherited_hundred_percent {
                return Err(
                    format!(
                        "GUI {key}.{coordinate} is neither an exact i32 nor a supported parent-relative token"
                    )
                    .into(),
                );
            }
            if inherited_hundred_percent
                && parent_size.is_some_and(|size| {
                    if matches!(*coordinate, "width" | "x") {
                        size.width <= 0
                    } else {
                        size.height <= 0
                    }
                })
            {
                return Err(format!("GUI {key}.{coordinate} inherits a nonpositive parent").into());
            }
        }
    }
    Ok(())
}

fn parse_exact_gui_size_ci(text: &str, key: &str, parent_size: Option<GuiSize>) -> Option<GuiSize> {
    let body = child_block_ci(text, key)?;
    let width_index =
        find_direct_assignment_ci(&body, "width").or_else(|| find_direct_assignment_ci(&body, "x"));
    let height_index = find_direct_assignment_ci(&body, "height")
        .or_else(|| find_direct_assignment_ci(&body, "y"));
    Some(GuiSize {
        width: match width_index {
            Some(index) => {
                parse_exact_gui_dimension_at(&body[index..], parent_size.map(|size| size.width))?
            }
            None => 0,
        },
        height: match height_index {
            Some(index) => {
                parse_exact_gui_dimension_at(&body[index..], parent_size.map(|size| size.height))?
            }
            None => 0,
        },
    })
}

fn parse_exact_gui_dimension_at(value: &str, parent: Option<i32>) -> Option<i32> {
    parse_i32_at(value).or_else(|| {
        parse_exact_hundred_percent_at(value)
            .then(|| parent)
            .flatten()
    })
}

fn parse_exact_hundred_percent_at(value: &str) -> bool {
    matches!(
        parse_token_at(value).as_deref(),
        Some("100%") | Some("100%%")
    )
}

fn parse_gui_point_ci(text: &str, key: &str) -> Option<GuiPoint> {
    let body = child_block_ci(text, key)?;
    Some(GuiPoint {
        x: parse_i32_ci(&body, "x").unwrap_or(0),
        y: parse_i32_ci(&body, "y").unwrap_or(0),
    })
}

fn parse_gui_size_ci(text: &str, key: &str) -> Option<GuiSize> {
    let body = child_block_ci(text, key)?;
    let width = parse_i32_ci(&body, "width")
        .or_else(|| parse_i32_ci(&body, "x"))
        .unwrap_or(0);
    let height = parse_i32_ci(&body, "height")
        .or_else(|| parse_i32_ci(&body, "y"))
        .unwrap_or(0);
    Some(GuiSize { width, height })
}

fn parse_quoted_values_from_child_block(text: &str, key: &str) -> Vec<String> {
    let Some(body) = child_block_ci(text, key) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in body.chars() {
        if !quoted {
            if character == '"' {
                quoted = true;
                value.clear();
            }
            continue;
        }
        if escaped {
            if matches!(character, '\\' | '"') {
                value.push(character);
            } else {
                value.push('\\');
                value.push(character);
            }
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = false;
            values.push(value.clone());
        } else {
            value.push(character);
        }
    }
    values
}

fn child_block_ci(text: &str, key: &str) -> Option<String> {
    let assignment = find_direct_assignment_ci(text, key)?;
    let bytes = text.as_bytes();
    let mut brace = assignment;
    while brace < bytes.len() && bytes[brace].is_ascii_whitespace() {
        brace += 1;
    }
    if brace >= bytes.len() || bytes[brace] != b'{' {
        return None;
    }
    let end = matching_brace(text, brace)?;
    Some(text[brace + 1..end].to_string())
}

fn parse_value_ci(text: &str, key: &str) -> Option<String> {
    parse_direct_value_ci(text, key)
}

fn parse_direct_value_ci(text: &str, key: &str) -> Option<String> {
    let idx = find_direct_assignment_ci(text, key)?;
    let value = text[idx..].trim_start();
    if let Some(rest) = value.strip_prefix('"') {
        let mut parsed = String::new();
        let mut escaped = false;
        for character in rest.chars() {
            if escaped {
                if matches!(character, '\\' | '"') {
                    parsed.push(character);
                } else {
                    parsed.push('\\');
                    parsed.push(character);
                }
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                return Some(parsed);
            } else {
                parsed.push(character);
            }
        }
        return None;
    }
    let token = value
        .chars()
        .take_while(|ch| !ch.is_whitespace() && *ch != '{' && *ch != '}')
        .collect::<String>();
    (!token.is_empty()).then_some(token.trim_matches('"').to_string())
}

fn find_direct_assignment_ci(text: &str, key: &str) -> Option<usize> {
    let folded = text.to_ascii_lowercase();
    let bytes = folded.as_bytes();
    let key = key.to_ascii_lowercase();
    let key_bytes = key.as_bytes();
    if key_bytes.is_empty() || bytes.len() < key_bytes.len() {
        return None;
    }
    let mut depth = 0u32;
    let mut quoted = false;
    let mut escaped = false;
    let mut i = 0usize;
    while i + key_bytes.len() <= bytes.len() {
        if quoted {
            if escaped {
                escaped = false;
            } else if bytes[i] == b'\\' {
                escaped = true;
            } else if bytes[i] == b'"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        match bytes[i] {
            b'"' => {
                quoted = true;
                i += 1;
                continue;
            }
            b'{' => {
                depth = depth.saturating_add(1);
                i += 1;
                continue;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            _ => {}
        }
        if depth == 0 && &bytes[i..i + key_bytes.len()] == key_bytes {
            let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
            let after = i + key_bytes.len();
            let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
            if before_ok && after_ok {
                let mut j = after;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'=' {
                    return Some(j + 1);
                }
            }
        }
        i += 1;
    }
    None
}

fn strip_gui_comments_quote_aware(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quoted = false;
    let mut escaped = false;
    let mut comment = false;
    for character in text.chars() {
        if comment {
            if matches!(character, '\r' | '\n') {
                comment = false;
                out.push(character);
            }
            continue;
        }
        if quoted {
            out.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => {
                quoted = true;
                out.push(character);
            }
            '#' => comment = true,
            _ => out.push(character),
        }
    }
    out
}

fn parse_u32_ci(text: &str, key: &str) -> Option<u32> {
    let idx = find_direct_assignment_ci(text, key)?;
    parse_exact_u32_at(&text[idx..])
}

fn parse_i32_ci(text: &str, key: &str) -> Option<i32> {
    let idx = find_direct_assignment_ci(text, key)?;
    parse_i32_at(&text[idx..])
}

fn parse_f32_ci(text: &str, key: &str) -> Option<f32> {
    let idx = find_direct_assignment_ci(text, key)?;
    parse_f32_at(&text[idx..])
}

fn parse_bool_ci(text: &str, key: &str) -> Option<bool> {
    let value = parse_value_ci(text, key)?;
    match value.to_ascii_lowercase().as_str() {
        "yes" | "true" => Some(true),
        "no" | "false" => Some(false),
        _ => None,
    }
}

fn parse_i32_at(value: &str) -> Option<i32> {
    parse_token_at(value)?.parse::<i32>().ok()
}

fn parse_exact_u32_at(value: &str) -> Option<u32> {
    parse_token_at(value)?.parse::<u32>().ok()
}

fn line_number_at(text: &str, offset: usize) -> usize {
    1 + text[..offset.min(text.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn vanilla_resolver() -> GameFileResolver {
        GameFileResolver::vanilla(replay_test_gates::require_vanilla_game_root())
            .expect("vanilla resolver")
    }

    fn write_fixture(root: &Path, rel: &str, bytes: &[u8]) {
        let path = slash_join(root, rel);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
        fs::write(path, bytes).expect("write fixture");
    }

    fn slash_join(root: &Path, rel: &str) -> PathBuf {
        let mut out = root.to_path_buf();
        for part in rel.split('/') {
            out.push(part);
        }
        out
    }

    fn write_test_dds(root: &Path, rel: &str, width: u32, height: u32) {
        let mut bytes = vec![0u8; 128 + width as usize * height as usize * 4];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&0x0002_100Fu32.to_le_bytes());
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[20..24].copy_from_slice(&(width * 4).to_le_bytes());
        bytes[28..32].copy_from_slice(&1u32.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        bytes[80..84].copy_from_slice(&0x41u32.to_le_bytes());
        bytes[88..92].copy_from_slice(&32u32.to_le_bytes());
        bytes[92..96].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
        bytes[96..100].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
        bytes[100..104].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
        bytes[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        bytes[108..112].copy_from_slice(&0x1000u32.to_le_bytes());
        write_fixture(root, rel, &bytes);
    }

    #[test]
    fn unit_counter_sprite_reader_uses_mod_resolver_overrides() {
        let root = temp_test_dir("unit_counter_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        let gfx = br#"
            spriteTypes = {
                spriteType = {
                    name = "GFX_unit_infantry_icon_medium_white"
                    textureFile = "gfx/interface/counters/divisions_small/onmap_unit_infantry_icon.dds"
                    noOfFrames = 2
                }
            }
        "#;
        write_fixture(&vanilla, UNIT_COUNTER_SUBUNITICONS_GFX_SOURCE, gfx);
        write_fixture(&mod_root, UNIT_COUNTER_SUBUNITICONS_GFX_SOURCE, gfx);
        write_test_dds(
            &vanilla,
            "gfx/interface/counters/divisions_small/onmap_unit_infantry_icon.dds",
            4,
            2,
        );
        write_test_dds(
            &mod_root,
            "gfx/interface/counters/divisions_small/onmap_unit_infantry_icon.dds",
            8,
            2,
        );

        let resolver =
            GameFileResolver::with_mod_paths(&vanilla, vec![mod_root]).expect("resolver");
        let symbols =
            on_map_division_counter_sprites_from_resolver(&resolver).expect("decode symbols");

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].texture_width, 8);
        assert_eq!(symbols[0].frame_width, 4);
        assert_eq!(resolver.stats().overridden_file_count, 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn vanilla_division_symbol_inventory_and_atlas_frames_decode() {
        let resolver = vanilla_resolver();
        let manifest = unit_counter_assets_from_resolver(&resolver).expect("decode manifest");

        assert_eq!(manifest.division_symbol_definition_count(), 126);
        assert_eq!(manifest.on_map_division_symbols.len(), 125);
        assert_eq!(manifest.unresolved_on_map_division_symbols.len(), 1);
        assert_eq!(
            manifest.unresolved_on_map_division_symbols[0].name,
            "GFX_unit_jungle_pioneers_support_icon_medium_white"
        );
        assert_eq!(manifest.unique_division_symbol_name_count(), 121);
        assert_eq!(manifest.unique_division_symbol_texture_count(), 82);

        for (name, texture) in [
            (
                "GFX_unit_infantry_icon_medium_white",
                "gfx/interface/counters/divisions_small/onmap_unit_infantry_icon.dds",
            ),
            (
                "GFX_unit_medium_armor_icon_medium_white",
                "gfx/interface/counters/divisions_small/onmap_unit_medium_tank_icon.dds",
            ),
            (
                "GFX_unit_light_armor_icon_medium_white",
                "gfx/interface/counters/divisions_small/onmap_unit_light_tank_icon.dds",
            ),
            (
                "GFX_unit_artillery_icon_medium_white",
                "gfx/interface/counters/divisions_small/onmap_unit_art_icon.dds",
            ),
        ] {
            let symbol = manifest
                .division_symbol_by_name(name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(symbol.texture_file, texture);
            assert_eq!(symbol.no_of_frames, 2);
            assert_eq!((symbol.texture_width, symbol.texture_height), (60, 12));
            assert_eq!((symbol.frame_width, symbol.frame_height), (30, 12));
            assert_eq!(
                symbol.frame(0).expect("frame 0").rect,
                PixelRect {
                    x: 0,
                    y: 0,
                    width: 30,
                    height: 12
                }
            );
            assert_eq!(
                symbol.frame(1).expect("frame 1").rect,
                PixelRect {
                    x: 30,
                    y: 0,
                    width: 30,
                    height: 12
                }
            );
            assert_eq!(
                symbol.frame(1).expect("frame 1").uv,
                UvRect {
                    u0: 0.5,
                    v0: 0.0,
                    u1: 1.0,
                    v1: 1.0
                }
            );
        }
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn vanilla_map_counter_backing_sprites_cross_check_gfx_frames_against_dds_size() {
        let resolver = vanilla_resolver();
        let sprites = map_counter_sprites_from_resolver(&resolver).expect("decode map sprites");

        let counter = sprites.get("GFX_onmap_unit_counter").expect("counter");
        assert_eq!(counter.texture_file, "gfx/interface/onmap_unit_counter.dds");
        assert_eq!(counter.no_of_frames, 3);
        assert_eq!((counter.texture_width, counter.texture_height), (198, 27));
        assert_eq!((counter.frame_width, counter.frame_height), (66, 27));

        let ideology = sprites
            .get("GFX_onmap_unit_counter_ideology")
            .expect("ideology");
        assert_eq!(ideology.no_of_frames, 4);
        assert_eq!((ideology.texture_width, ideology.texture_height), (112, 27));
        assert_eq!((ideology.frame_width, ideology.frame_height), (28, 27));

        let types = sprites.get("GFX_mapicon_unit_types_strip").expect("types");
        assert_eq!(types.no_of_frames, 12);
        assert_eq!((types.texture_width, types.texture_height), (360, 12));
        assert_eq!((types.frame_width, types.frame_height), (30, 12));

        let bars = [
            sprites.get("GFX_mapicon_unit_bar_org").expect("org bar"),
            sprites.get("GFX_mapicon_unit_bar_str").expect("str bar"),
        ];
        for bar in bars {
            assert_eq!((bar.texture_width, bar.texture_height), (3, 17));
            assert_eq!((bar.frame_width, bar.frame_height), (3, 17));
        }
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn vanilla_map_counter_gui_layout_decodes_counter_quads_and_children() {
        let resolver = vanilla_resolver();
        let layouts = unit_counter_gui_layouts_from_resolver(&resolver).expect("layouts");

        assert_eq!(layouts.zoomed_in_offset, GuiPoint { x: 0, y: 20 });
        assert_eq!(
            layouts.land.size.expect("land size"),
            GuiSize {
                width: 76,
                height: 24
            }
        );
        assert_eq!(
            layouts.navy.size.expect("navy size"),
            GuiSize {
                width: 66,
                height: 33
            }
        );
        assert_eq!(
            layouts.railway_gun.size.expect("railway gun size"),
            GuiSize {
                width: 58,
                height: 24
            }
        );
        assert_eq!(
            layouts.navy.control("wings_box").expect("wings grid").size,
            layouts.navy.size
        );

        let land_type = layouts.land.control("type").expect("type control");
        assert_eq!(land_type.position, Some(GuiPoint { x: 3, y: 3 }));
        assert_eq!(
            land_type.sprite_type.as_deref(),
            Some("GFX_tiled_window_transparent")
        );
        let count = layouts.land.control("count_txt").expect("count text");
        assert_eq!(count.position, Some(GuiPoint { x: 36, y: 6 }));
        assert_eq!(count.font.as_deref(), Some("Main_14"));
        assert_eq!(count.max_width, Some(20));
        assert_eq!(count.max_height, Some(14));
        let org = layouts.land.control("bar_org").expect("org bar");
        assert_eq!(org.position, Some(GuiPoint { x: 14, y: 8 }));
        assert_eq!(org.rotation, Some(1.5708));
        let low_supply = layouts.land.control("low_supply").expect("low supply");
        assert_eq!(low_supply.position, Some(GuiPoint { x: 24, y: -4 }));
        assert_eq!(low_supply.scale, Some(0.75));
        let active_grid = layouts
            .land
            .control("active_abilities")
            .expect("active ability grid");
        assert_eq!(active_grid.position, Some(GuiPoint { x: 55, y: -9 }));
        assert_eq!(
            active_grid.slot_size,
            Some(GuiSize {
                width: 25,
                height: 25,
            })
        );
        assert_eq!(active_grid.max_slots_vertical, Some(1));
        let active_ability_item = layouts
            .active_ability_item
            .as_ref()
            .expect("active ability item template");
        let active_item = active_ability_item
            .control("active_ability")
            .expect("active ability item control");
        assert_eq!(
            active_ability_item.size,
            Some(GuiSize {
                width: 20,
                height: 20
            })
        );
        assert_eq!(
            active_ability_item
                .controls
                .iter()
                .filter(|control| {
                    control.sprite_type.is_some()
                        || control.quad_texture_sprite.is_some()
                        || control.font.is_some()
                })
                .count(),
            1
        );
        assert_eq!(active_item.orientation.as_deref(), Some("center"));
        assert_eq!(active_item.scale, Some(0.8));
        let wing_type = layouts
            .air_wing
            .control("wing_type")
            .expect("air wing type control");
        assert_eq!(wing_type.sprite_type.as_deref(), Some("GFX_wing_types_map"));
        assert_eq!(wing_type.frame, Some(1));
    }

    #[test]
    fn active_ability_template_preserves_unsupported_named_child_kinds() {
        let text = r#"
            guiTypes = {
                containerWindowType = {
                    name = "active_ability_item"
                    size = { width = 20 height = 20 }
                    iconType = {
                        tooltip = "name = \"phantom\" spriteType = \"GFX_phantom\""
                        name = "active_ability"
                        spriteType = "GFX_active_ability"
                    }
                    tooltip = "literal \"}# still quoted\""
                    tooltip = "iconType = { name = \"phantom\" spriteType = \"GFX_phantom\" }"
                    checkboxType = {
                        name = "modded_checkbox"
                        spriteType = "GFX_modded_checkbox"
                    }
                    background = {
                        quadTextureSprite = "GFX_modded_background"
                    }
                }
                containerWindowType = {
                    name = "unrelated_sibling"
                }
            }
        "#;
        let template = optional_container_layout_from_text_with_all_named_controls(
            text,
            "active_ability_item",
            "fixture.gui",
        )
        .expect("parse template")
        .expect("template");
        assert_eq!(template.controls.len(), 3);
        assert_eq!(template.controls[0].kind, "iconType");
        assert_eq!(template.controls[1].kind, "checkboxType");
        assert_eq!(template.controls[2].kind, "background");
        assert!(template.controls[2].name.is_empty());
        assert!(template.control("phantom").is_none());
        assert_eq!(
            strip_gui_comments_quote_aware("value = \"# not comment\" # comment\nnext = yes"),
            "value = \"# not comment\" \nnext = yes"
        );
        assert_eq!(
            parse_direct_value_ci(r#"path = "gfx\interface\counter.dds""#, "path").as_deref(),
            Some(r#"gfx\interface\counter.dds"#)
        );
    }

    #[test]
    fn counter_layout_censuses_unnamed_and_nested_modded_drawables() {
        let text = r#"
            guiTypes = {
                containerWindowType = {
                    name = "unit_counter"
                    checkboxType = {
                        tooltip = "name = \"phantom\" spriteType = \"GFX_phantom\""
                        name = "modded_checkbox"
                        spriteType = "GFX_modded_checkbox"
                        position = { x = 2 y = 3 }
                    }
                    background = {
                        quadTextureSprite = "GFX_modded_background"
                    }
                    containerWindowType = {
                        name = "nested"
                        position = { x = 10 y = 20 }
                        size = { width = 20 height = 20 }
                        iconType = {
                            name = "nested_icon"
                            spriteType = "GFX_nested_icon"
                            position = { x = 3 y = 4 }
                        }
                        iconType = {
                            name = "centered_icon"
                            spriteType = "GFX_centered_icon"
                            orientation = center
                        }
                    }
                }
            }
        "#;
        let layout = exact_counter_container_layout_from_text(text, "unit_counter", "fixture.gui")
            .expect("counter layout");
        assert_eq!(layout.controls.len(), 5);
        let checkbox = layout.control("modded_checkbox").expect("checkbox");
        assert_eq!(checkbox.sprite_type.as_deref(), Some("GFX_modded_checkbox"));
        assert_eq!(checkbox.position, Some(GuiPoint { x: 2, y: 3 }));
        assert!(layout.control("phantom").is_none());
        let unnamed_background = &layout.controls[1];
        assert!(unnamed_background.name.is_empty());
        assert_eq!(unnamed_background.kind, "background");
        assert_eq!(
            unnamed_background.quad_texture_sprite.as_deref(),
            Some("GFX_modded_background")
        );
        assert_eq!(
            layout.control("nested_icon").expect("nested icon").position,
            Some(GuiPoint { x: 13, y: 24 })
        );
        let centered = layout.control("centered_icon").expect("centered icon");
        assert_eq!(centered.position, Some(GuiPoint { x: 10, y: 20 }));
        assert_eq!(
            centered.parent_size,
            Some(GuiSize {
                width: 20,
                height: 20
            })
        );
        assert_eq!(centered.orientation.as_deref(), Some("center"));
        assert_eq!(parse_i32_at("10%%"), None);
        assert_eq!(parse_i32_at("2147483648"), None);
        assert_eq!(parse_i32_at("-2147483648"), Some(i32::MIN));
        assert_eq!(parse_exact_u32_at("12px"), None);
        assert_eq!(parse_exact_u32_at("4294967296"), None);
        assert_eq!(parse_exact_u32_at("4294967295"), Some(u32::MAX));
        let malformed = r#"
            containerWindowType = {
                name = "unit_counter"
                iconType = {
                    name = "bad"
                    spriteType = "GFX_bad"
                    position = { x = 10%% y = 2 }
                }
            }
        "#;
        assert!(
            exact_counter_container_layout_from_text(malformed, "unit_counter", "bad.gui").is_err()
        );
        let overflowing = r#"
            containerWindowType = {
                name = "unit_counter"
                iconType = {
                    name = "bad"
                    spriteType = "GFX_bad"
                    frame = 4294967296
                }
            }
        "#;
        assert!(exact_counter_container_layout_from_text(
            overflowing,
            "unit_counter",
            "overflow.gui",
        )
        .is_err());
    }

    #[test]
    fn exact_counter_layout_applies_centered_ancestor_and_axis_defaults() {
        let text = r#"
            containerWindowType = {
                name = "unit_counter"
                size = { width = 41 height = 41 }
                containerWindowType = {
                    name = "centered_parent"
                    position = { x = 1 }
                    size = { width = 20 height = 20 }
                    orientation = center
                    iconType = {
                        name = "nested_icon"
                        spriteType = "GFX_nested"
                        position = { y = 3 }
                        size = { width = 10 }
                    }
                }
            }
        "#;
        let layout =
            exact_counter_container_layout_from_text(text, "unit_counter", "axis-defaults.gui")
                .expect("exact layout");
        let nested = layout.control("nested_icon").expect("nested icon");
        assert_eq!(nested.position, Some(GuiPoint { x: 1, y: 3 }));
        assert_eq!(
            nested.size,
            Some(GuiSize {
                width: 10,
                height: 0
            })
        );
        assert_eq!(
            nested.effective_position_f32_bits,
            Some([11.5f32.to_bits(), 13.5f32.to_bits()])
        );

        let unsupported = text.replace("orientation = center", "orientation = lower_right");
        assert!(exact_counter_container_layout_from_text(
            &unsupported,
            "unit_counter",
            "unsupported-orientation.gui",
        )
        .is_err());
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn vanilla_faction_gui_uses_zero_for_omitted_axes() {
        let resolver = vanilla_resolver();
        let country_path = resolver
            .resolve_required(
                "interface/factions/countryfactionview.gui",
                "country faction GUI",
            )
            .expect("country faction GUI");
        let country = read_latin1(&country_path).expect("read country faction GUI");
        let pair = named_block_from_text(
            &country,
            "change_rule_item_pair_container",
            "containerWindowType",
        )
        .expect("pair container");
        validate_gui_numeric_fields(&pair.body, None).expect("exact pair numeric fields");
        assert_eq!(
            parse_gui_point_ci(&pair.body, "position"),
            Some(GuiPoint { x: 0, y: -35 })
        );

        let new_faction_path = resolver
            .resolve_required(
                "interface/factions/new_faction_window.gui",
                "new faction GUI",
            )
            .expect("new faction GUI");
        let new_faction = read_latin1(&new_faction_path).expect("read new faction GUI");
        let title = named_block_from_text(&new_faction, "faction_title", "containerWindowType")
            .expect("faction title");
        validate_gui_numeric_fields(&title.body, None).expect("exact title numeric fields");
        assert_eq!(
            parse_exact_gui_size_ci(&title.body, "size", None),
            Some(GuiSize {
                width: 550,
                height: 0
            })
        );
    }

    #[test]
    fn exact_sprite_parser_distinguishes_absent_fields_from_malformed_fields() {
        let defaults = r#"
            spriteType = {
                name = "GFX_defaulted"
            }
        "#;
        let parsed = parse_sprite_defs_exact(defaults, "defaults.gfx").expect("defaults");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].texture_file, None);
        assert_eq!(parsed[0].no_of_frames, 1);
        assert!(!parsed[0].frame_direction_y);

        for malformed in [
            r#"spriteType = { name = "GFX_bad" noOfFrames = 2junk }"#,
            r#"spriteType = { name = "GFX_bad" framedirectiony = perhaps }"#,
            r#"spriteType = { name = "GFX_bad" textureFile = {} }"#,
        ] {
            assert!(parse_sprite_defs_exact(malformed, "malformed.gfx").is_err());
        }
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn vanilla_division_template_custom_icon_layout_decodes_picker_geometry() {
        let resolver = vanilla_resolver();
        let layout =
            division_template_counter_layout_from_resolver(&resolver).expect("template layout");

        assert_eq!(
            layout.custom_icon_button.size.expect("custom button size"),
            GuiSize {
                width: 84,
                height: 50
            }
        );
        assert_eq!(
            layout
                .custom_icon_inner_button
                .size
                .expect("custom inner size"),
            GuiSize {
                width: 76,
                height: 42
            }
        );
        assert_eq!(
            layout.non_hq_symbol_button.position,
            Some(GuiPoint { x: 39, y: 51 })
        );
        assert_eq!(
            layout.hq_symbol_button.position,
            Some(GuiPoint { x: 38, y: 80 })
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn vanilla_counter_fonts_and_unitview_bars_decode() {
        let resolver = vanilla_resolver();
        let fonts = counter_fonts_from_resolver(&resolver).expect("fonts");
        assert!(fonts
            .iter()
            .any(|font| font.name == "Main_14"
                && font.path.as_deref() == Some("gfx/fonts/garamond_14")));
        assert!(fonts.iter().any(|font| {
            font.name == "hoi_18mbs"
                && font
                    .font_files
                    .iter()
                    .any(|file| file == "gfx/fonts/hoi_18mbs")
        }));

        let sprites = unitview_counter_sprites_from_resolver(&resolver).expect("unitview sprites");
        let supply = sprites
            .get("GFX_unitcounter_supply_status_strip")
            .expect("supply strip");
        assert_eq!(supply.no_of_frames, 3);
        assert_eq!((supply.texture_width, supply.texture_height), (72, 24));
        assert_eq!((supply.frame_width, supply.frame_height), (24, 24));
        let unit_level = sprites.get("GFX_unit_level").expect("unit level");
        assert_eq!(unit_level.no_of_frames, 5);

        let bars = army_badge_progress_bars_from_resolver(&resolver).expect("progress bars");
        let org = bars.get("GFX_army_badge_bar_org").expect("org progress");
        assert_eq!(
            org.size,
            Some(GuiSize {
                width: 28,
                height: 6
            })
        );
        assert_eq!(org.texture_file1, "gfx/interface/navieslist_bar_org.dds");
        assert_eq!(org.texture_file2, "gfx/interface/army_badge_bar_bg.dds");
    }
}
