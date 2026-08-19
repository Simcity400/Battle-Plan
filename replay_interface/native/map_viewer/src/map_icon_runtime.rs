
use crate::unit_counter_assets::{
    container_layout_from_text, masked_shield_definitions, parse_sprite_defs, sprite_asset_from_def,
};
use crate::{
    resource_definitions_from_resolver, vp_icon_relation_column, vp_strip_frame_index, AtlasFrame,
    FurnitureMarkerKind, GameFileResolver, ProductionMapCamera, Result, UnitStackPlacement,
    VpIconDefines, VpIconDiplomaticPairState, MAPICONS_GUI_SOURCE, VP_STRIP_FRAME_COUNT,
    VP_STRIP_SPRITE_NAME,
};
use replay_core::{
    CapitalLabelSource, Leaf, RaidFurnitureIdentity, RaidFurniturePresentation, ViewStatus,
    WorldFrame,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

pub const XMAP21_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XMAP21_VP_ANCHOR_SLOT: u32 = 38;
pub const XMAP21_VP_FALLBACK_SLOT: u32 = 10;
pub const XMAP21_ALIGNMENT_BITS: u8 = 0x0a;
pub const XMAP21_SORT_DEPTH_SCALE: f32 = 1_000_000.0;
pub const XMAP21_RESOURCE_ALIGNMENT_BITS: u8 = 0x12;
pub const XMAP21_RAID_ALIGNMENT_BITS: u8 = 0x0a;
pub const XMAP21_RESOURCE_MODE_ID: &str = "resources";
pub const XMAP21_RESOURCE_VIEWER_TOKEN: &str = "viewer.map_mode.resources";
pub const XMAP21_RAID_MODE_ID: &str = "raids";
pub const XMAP21_RAID_VIEWER_TOKEN: &str = "viewer.map_mode.raids";
pub const RESOURCE_MAP_ICON_TEXT_CUTOFF_KEY: &str = "RESOURCE_MAP_ICON_TEXT_CUTOFF";
pub const RAID_MAP_ICON_DRAW_DISTANCE_MIN_KEY: &str = "RAID_MAP_ICON_DRAW_DISTANCE_MIN";
pub const RAID_MAP_ICON_DRAW_DISTANCE_MAX_KEY: &str = "RAID_MAP_ICON_DRAW_DISTANCE_MAX";
pub const RAID_MAP_ICON_DRAW_DISTANCE_MAX_RAID_MAP_MODE_KEY: &str =
    "RAID_MAP_ICON_DRAW_DISTANCE_MAX_RAID_MAP_MODE";
pub const XMAP21_RESOURCE_TEXT_BATCH_SCHEMA: &str =
    "hoi4.replay.xmap21-resource-text-operations.v1";
pub const XMAP21_VP_CAPITAL_TEXT_BATCH_SCHEMA: &str =
    "hoi4.replay.xmap21-vp-capital-text-operations.v1";
pub const XMAP21_PREPARED_PACKET_SCHEMA: &str = "hoi4.replay.xmap21-map-furniture-packet.v5";
pub const XMAP21_PREPARE_RECEIPT_SCHEMA: &str = "hoi4.replay.xmap21-map-furniture-prepare.v1";
pub const XMAP21_PREPARE_STAGE_NAME: &str = "xmap21.map_furniture.prepare";
pub const XMAP21_PREPARE_OPERATION_NAME: &str = "map_furniture.prepare";
pub const XMAP21_EXECUTE_STAGE_NAME: &str = "xmap21.map_furniture.execute";
pub const XMAP21_EXECUTE_OPERATION_NAME: &str = "map_furniture.icons.draw";
pub const XMAP21_RAID_GFX_SOURCES: &[&str] = &[
    "interface/general_stuff.gfx",
    "interface/military_raids/military_raids.gfx",
];
pub const XMAP21_RESOURCE_FLAG_SPRITE_NAME: &str = "GFX_flag_smallest";
pub const XMAP21_RESOURCE_FLAG_SAMPLING_LAW: &str = "maskedflag.direct-d3d11-base-mask-overlay.v1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactVpFileAnchor {
    pub point: [f32; 3],
    pub slot: u32,
}

pub fn exact_vp_file_anchors(rows: &[UnitStackPlacement]) -> BTreeMap<u32, ExactVpFileAnchor> {
    let mut slots = BTreeMap::<u32, (Option<[f32; 3]>, Option<[f32; 3]>)>::new();
    for row in rows {
        let entry = slots.entry(row.province_id).or_default();
        match row.kind_raw {
            XMAP21_VP_ANCHOR_SLOT => entry.0 = Some(row.pos),
            XMAP21_VP_FALLBACK_SLOT => entry.1 = Some(row.pos),
            _ => {}
        }
    }
    slots
        .into_iter()
        .filter_map(|(province_id, (vp, defending))| {
            vp.map(|point| ExactVpFileAnchor {
                point,
                slot: XMAP21_VP_ANCHOR_SLOT,
            })
            .or_else(|| {
                defending.map(|point| ExactVpFileAnchor {
                    point,
                    slot: XMAP21_VP_FALLBACK_SLOT,
                })
            })
            .map(|anchor| (province_id, anchor))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapIconChildGeometry {
    pub offset_x: i32,
    pub offset_y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapIconTextGeometry {
    pub offset_x: i32,
    pub offset_y: i32,
    pub max_width: i32,
    pub max_height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapIconRootGeometry {
    pub width: i32,
    pub height: i32,
    pub icon: MapIconChildGeometry,
    pub text: MapIconTextGeometry,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapFurnitureRgbaImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapFurnitureSpriteAsset {
    pub name: String,
    pub definition_source_file: String,
    pub definition_source_line: usize,
    pub texture_file: String,
    pub no_of_frames: u32,
    pub frame_direction_y: bool,
    pub frames: Vec<AtlasFrame>,
    pub image: MapFurnitureRgbaImage,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MapFurnitureMaskedShieldDescriptor {
    pub sprite_name: String,
    pub definition_source_file: String,
    pub overlay_texture_path: String,
    pub overlay_sha256: String,
    pub mask_texture_path: String,
    pub mask_sha256: String,
    pub effect_file: String,
    pub effect_shader_path: String,
    pub effect_shader_sha256: String,
    pub descriptor_sha256: String,
}

impl MapFurnitureMaskedShieldDescriptor {
    pub fn validate(&self) -> Result<()> {
        if self.sprite_name != XMAP21_RESOURCE_FLAG_SPRITE_NAME
            || self.definition_source_file.is_empty()
            || self.overlay_texture_path.is_empty()
            || self.mask_texture_path.is_empty()
            || self.effect_file.is_empty()
            || self.effect_shader_path.is_empty()
        {
            return Err("XMAP21 resource masked-shield descriptor is incomplete".into());
        }
        for (value, role) in [
            (&self.overlay_sha256, "resource flag overlay"),
            (&self.mask_sha256, "resource flag mask"),
            (&self.effect_shader_sha256, "resource flag effect"),
            (&self.descriptor_sha256, "resource masked-shield descriptor"),
        ] {
            validate_lower_sha256(value, role)?;
        }
        let mut unsigned = self.clone();
        unsigned.descriptor_sha256.clear();
        if self.descriptor_sha256 != canonical_sha256(&unsigned)? {
            return Err("XMAP21 resource masked-shield descriptor digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MapFurnitureResourceFlagAssetDescriptor {
    pub logical_path: String,
    pub content_sha256: String,
    pub source_encoding: String,
    pub sampling_law: String,
}

pub fn resolve_resource_flag_assets(
    resolver: &GameFileResolver,
    resources: &[ProjectedResourceFurnitureIcon],
) -> Result<(
    MapFurnitureMaskedShieldDescriptor,
    Vec<MapFurnitureResourceFlagAssetDescriptor>,
)> {
    resolver.require_complete_admitted_dir(
        "interface",
        Some("gfx"),
        false,
        "XMAP21 resource-flag GFX namespace",
    )?;
    let mut definitions = Vec::new();
    for (source_file, path) in resolver.resolve_dir_files("interface", Some("gfx"))? {
        let bytes = fs::read(&path)?;
        let source: String = bytes.into_iter().map(char::from).collect();
        if !source.contains(XMAP21_RESOURCE_FLAG_SPRITE_NAME) {
            continue;
        }
        for (overlay, mask, effect_file) in
            masked_shield_definitions(&source, XMAP21_RESOURCE_FLAG_SPRITE_NAME)
        {
            definitions.push((source_file.clone(), overlay, mask, effect_file));
        }
    }
    let [(definition_source_file, overlay_texture_path, mask_texture_path, effect_file)] =
        definitions.as_slice()
    else {
        return Err(
            "XMAP21 DATA_BLOCKED: effective GFX_flag_smallest definition is not unique".into(),
        );
    };
    let effect_shader_path = std::path::Path::new(effect_file)
        .with_extension("shader")
        .to_string_lossy()
        .replace('\\', "/");
    let file_sha256 = |logical_path: &str, role: &str| -> Result<String> {
        let path = resolver.resolve_required(logical_path, role)?;
        Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
    };
    let effect_path = resolver.resolve_required(&effect_shader_path, "resource flag effect")?;
    let effect_bytes = fs::read(&effect_path)?;
    let effect_source: String = effect_bytes.iter().copied().map(char::from).collect();
    validate_masked_flag_effect(&effect_source)?;
    let mut shield = MapFurnitureMaskedShieldDescriptor {
        sprite_name: XMAP21_RESOURCE_FLAG_SPRITE_NAME.into(),
        definition_source_file: definition_source_file.clone(),
        overlay_texture_path: overlay_texture_path.clone(),
        overlay_sha256: file_sha256(overlay_texture_path, "resource flag overlay")?,
        mask_texture_path: mask_texture_path.clone(),
        mask_sha256: file_sha256(mask_texture_path, "resource flag mask")?,
        effect_file: effect_file.clone(),
        effect_shader_sha256: format!("{:x}", Sha256::digest(&effect_bytes)),
        effect_shader_path,
        descriptor_sha256: String::new(),
    };
    shield.descriptor_sha256 = canonical_sha256(&shield)?;
    shield.validate()?;
    let paths: BTreeSet<_> = resources
        .iter()
        .filter_map(|icon| icon.controller_flag_texture_path.clone())
        .collect();
    let flags = paths
        .into_iter()
        .map(|logical_path| {
            let is_tga = std::path::Path::new(&logical_path)
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("tga"));
            if !is_tga {
                return Err(format!(
                    "XMAP21 DATA_BLOCKED: resource controller flag {logical_path:?} does not provide the admitted TGA source for direct D3D11 masked-flag execution"
                )
                .into());
            }
            Ok(MapFurnitureResourceFlagAssetDescriptor {
                content_sha256: file_sha256(&logical_path, "resource controller flag")?,
                logical_path,
                source_encoding: "tga".into(),
                sampling_law: XMAP21_RESOURCE_FLAG_SAMPLING_LAW.into(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((shield, flags))
}

fn validate_masked_flag_effect(source: &str) -> Result<()> {
    let effect = crate::parse_effect_file(source)?;
    let sampler = |name: &str| {
        effect
            .pixel_stage
            .samplers
            .iter()
            .find(|sampler| sampler.name == name)
            .ok_or_else(|| format!("XMAP21 maskedflag effect lacks {name} sampler"))
    };
    let base = sampler("BaseTexture")?;
    let mask = sampler("MaskTexture")?;
    if base.index != 0
        || base.mag_filter.as_deref() != Some("Point")
        || base.min_filter.as_deref() != Some("Linear")
        || base.mip_filter.as_deref() != Some("Linear")
        || base.address_u.as_deref() != Some("Clamp")
        || base.address_v.as_deref() != Some("Clamp")
        || mask.index != 1
        || mask.mag_filter.as_deref() != Some("Point")
        || mask.min_filter.as_deref() != Some("Point")
        || mask.mip_filter.as_deref() != Some("None")
        || mask.address_u.as_deref() != Some("Clamp")
        || mask.address_v.as_deref() != Some("Clamp")
    {
        return Err("XMAP21 maskedflag sampler law is unsupported".into());
    }
    let up = effect
        .effect("Up")
        .ok_or("XMAP21 maskedflag effect lacks Up state")?;
    if up.pixel_shader.as_deref() != Some("PixelShader") {
        return Err("XMAP21 maskedflag Up state does not select PixelShader".into());
    }
    let main = effect
        .pixel_stage
        .main_codes
        .get("PixelShader")
        .ok_or("XMAP21 maskedflag effect lacks PixelShader body")?;
    let compact: String = main
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let expected = "float4main(VS_OUTPUTv):PDX_COLOR{float4OutColor=tex2D(BaseTexture,v.vTexCoord.xy);float4MaskColor=tex2D(MaskTexture,v.vTexCoord.zw);OutColor.a=MaskColor.a;returnOutColor;}";
    if compact != expected {
        return Err("XMAP21 maskedflag PixelShader law is unsupported".into());
    }
    let blend = effect
        .blend_states
        .get("BlendState")
        .ok_or("XMAP21 maskedflag effect lacks BlendState")?;
    let field = |key: &str| {
        blend
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    if field("BlendEnable") != Some("yes")
        || field("AlphaTest") != Some("no")
        || field("SourceBlend") != Some("SRC_ALPHA")
        || field("DestBlend") != Some("INV_SRC_ALPHA")
    {
        return Err("XMAP21 maskedflag blend law is unsupported".into());
    }
    Ok(())
}

pub fn load_map_furniture_sprite_assets(
    resolver: &GameFileResolver,
    sprite_names: &BTreeSet<String>,
) -> Result<BTreeMap<String, MapFurnitureSpriteAsset>> {
    resolver.require_complete_admitted_dir(
        "interface",
        Some("gfx"),
        false,
        "XMAP21 furniture GFX namespace",
    )?;
    let mut definitions = BTreeMap::new();
    for (source_file, path) in resolver.resolve_dir_files("interface", Some("gfx"))? {
        let bytes = fs::read(&path)?;
        let source: String = bytes.into_iter().map(char::from).collect();
        if !sprite_names.iter().any(|name| source.contains(name)) {
            continue;
        }
        for definition in parse_sprite_defs(&source, &source_file)
            .into_iter()
            .filter(|definition| sprite_names.contains(&definition.name))
        {
            if definitions
                .insert(definition.name.clone(), definition)
                .is_some()
            {
                return Err("XMAP21 raid sprite token is defined more than once".into());
            }
        }
    }
    let mut assets = BTreeMap::new();
    for name in sprite_names {
        let definition = definitions
            .remove(name)
            .ok_or_else(|| format!("XMAP21 DATA_BLOCKED: unresolved sprite token {name:?}"))?;
        let texture_file = definition
            .texture_file
            .clone()
            .ok_or_else(|| format!("XMAP21 DATA_BLOCKED: sprite {name:?} has no texture"))?;
        let metadata = sprite_asset_from_def(resolver, definition, texture_file)?;
        let path = resolver.resolve_required(&metadata.texture_file, name)?;
        let image = crate::image_load::load_dds_rgba(&path)?;
        let pixel_sha256 = format!("{:x}", Sha256::digest(&image.pixels));
        let descriptor_sha256 = canonical_sha256(&(
            name.as_str(),
            metadata.source_file.as_str(),
            metadata.source_line,
            metadata.texture_file.as_str(),
            metadata.no_of_frames,
            metadata.frame_direction_y,
            metadata.texture_width,
            metadata.texture_height,
            metadata.mip_count,
            pixel_sha256.as_str(),
        ))?;
        assets.insert(
            name.clone(),
            MapFurnitureSpriteAsset {
                name: name.clone(),
                definition_source_file: metadata.source_file,
                definition_source_line: metadata.source_line,
                texture_file: metadata.texture_file,
                no_of_frames: metadata.no_of_frames,
                frame_direction_y: metadata.frame_direction_y,
                frames: metadata.frames,
                image: MapFurnitureRgbaImage {
                    width: image.width,
                    height: image.height,
                    pixels: image.pixels,
                },
                descriptor_sha256,
            },
        );
    }
    Ok(assets)
}

pub fn load_vp_strip_asset(resolver: &GameFileResolver) -> Result<MapFurnitureSpriteAsset> {
    let names = BTreeSet::from([VP_STRIP_SPRITE_NAME.to_string()]);
    let mut assets = load_map_furniture_sprite_assets(resolver, &names)?;
    let asset = assets
        .remove(VP_STRIP_SPRITE_NAME)
        .ok_or("XMAP21 DATA_BLOCKED: effective VP strip sprite is absent")?;
    if asset.no_of_frames != VP_STRIP_FRAME_COUNT || asset.frame_direction_y {
        return Err(format!(
            "XMAP21 DATA_BLOCKED: effective {VP_STRIP_SPRITE_NAME} must be a horizontal {VP_STRIP_FRAME_COUNT}-frame atlas"
        )
        .into());
    }
    Ok(asset)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapIconContainerGeometry {
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RaidMapIconGeometry {
    pub root: MapIconContainerGeometry,
    pub actionable_child: MapIconChildGeometry,
    pub content_center_offset: [i32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapFurnitureGuiGeometry {
    pub victory_point: MapIconRootGeometry,
    pub capital: MapIconRootGeometry,
    pub resource: MapIconContainerGeometry,
    pub resource_item: MapIconContainerGeometry,
    pub resource_flag_offset: [i32; 2],
    pub resource_item_flag_offset: [i32; 2],
    pub resource_item_desc: MapIconTextGeometry,
    pub raid: RaidMapIconGeometry,
}

impl MapFurnitureGuiGeometry {
    pub fn from_resolver(resolver: &GameFileResolver) -> Result<Self> {
        let gui_path = resolver.resolve_required(MAPICONS_GUI_SOURCE, "mapicons.gui")?;
        let gui = fs::read_to_string(&gui_path)
            .map_err(|error| format!("{}: {error}", gui_path.display()))?;
        let strip = load_vp_strip_asset(resolver)?;
        if strip.image.width == 0
            || strip.image.height == 0
            || strip.image.width % VP_STRIP_FRAME_COUNT as usize != 0
        {
            return Err("VP strip has invalid authored frame geometry".into());
        }
        let icon_width = u32::try_from(strip.image.width / VP_STRIP_FRAME_COUNT as usize)
            .map_err(|_| "VP strip frame width exceeds u32")?;
        let icon_height =
            u32::try_from(strip.image.height).map_err(|_| "VP strip height exceeds u32")?;
        let parse = |name: &str| -> Result<MapIconRootGeometry> {
            let layout = container_layout_from_text(&gui, name, MAPICONS_GUI_SOURCE)?;
            let size = layout
                .size
                .ok_or_else(|| format!("{name} has no authored root size"))?;
            if size.width <= 0 || size.height <= 0 {
                return Err(format!("{name} has non-positive authored root size").into());
            }
            let icon = layout
                .controls
                .iter()
                .find(|control| control.name == "icon")
                .ok_or_else(|| format!("{name} has no authored icon child"))?;
            if icon.sprite_type.as_deref() != Some(VP_STRIP_SPRITE_NAME) {
                return Err(format!("{name}.icon does not bind {VP_STRIP_SPRITE_NAME}").into());
            }
            let position = icon
                .position
                .ok_or_else(|| format!("{name}.icon has no authored position"))?;
            let text = layout
                .controls
                .iter()
                .find(|control| control.name == "text")
                .ok_or_else(|| format!("{name} has no authored text child"))?;
            let icon_order = layout
                .controls
                .iter()
                .position(|control| control.name == "icon")
                .ok_or_else(|| format!("{name} has no authored icon child"))?;
            let text_order = layout
                .controls
                .iter()
                .position(|control| control.name == "text")
                .ok_or_else(|| format!("{name} has no authored text child"))?;
            if icon_order >= text_order {
                return Err(format!("{name} does not author icon before text").into());
            }
            if text.font.as_deref() != Some("hoi_20bs") || text.format.as_deref() != Some("left") {
                return Err(format!("{name}.text does not declare hoi_20bs/left").into());
            }
            let text_position = text
                .position
                .ok_or_else(|| format!("{name}.text has no authored position"))?;
            let max_width = text
                .max_width
                .ok_or_else(|| format!("{name}.text has no authored maxWidth"))?;
            let max_height = text
                .max_height
                .ok_or_else(|| format!("{name}.text has no authored maxHeight"))?;
            if max_width <= 0 || max_height <= 0 {
                return Err(format!("{name}.text has non-positive authored bounds").into());
            }
            Ok(MapIconRootGeometry {
                width: size.width,
                height: size.height,
                icon: MapIconChildGeometry {
                    offset_x: position.x,
                    offset_y: position.y,
                    width: icon_width,
                    height: icon_height,
                },
                text: MapIconTextGeometry {
                    offset_x: text_position.x,
                    offset_y: text_position.y,
                    max_width,
                    max_height,
                },
            })
        };
        let resource_layout =
            container_layout_from_text(&gui, "resource_mapicon", MAPICONS_GUI_SOURCE)?;
        require_authored_order(&resource_layout, &["resources_list", "flag"])?;
        let resource_item_layout =
            container_layout_from_text(&gui, "resource_mapicon_item", MAPICONS_GUI_SOURCE)?;
        require_authored_order(&resource_item_layout, &["desc", "flag"])?;
        let desc = resource_item_layout
            .controls
            .iter()
            .find(|control| control.name == "desc")
            .ok_or("resource_mapicon_item has no authored desc child")?;
        if desc.font.as_deref() != Some("hoi_16mbs") || desc.format.as_deref() != Some("left") {
            return Err("resource_mapicon_item.desc does not declare hoi_16mbs/left".into());
        }
        let desc_position = desc
            .position
            .ok_or("resource_mapicon_item.desc has no authored position")?;
        let desc_max_width = desc
            .max_width
            .ok_or("resource_mapicon_item.desc has no authored maxWidth")?;
        let desc_max_height = desc
            .max_height
            .ok_or("resource_mapicon_item.desc has no authored maxHeight")?;
        if desc_max_width <= 0 || desc_max_height <= 0 {
            return Err("resource_mapicon_item.desc has non-positive authored bounds".into());
        }
        let raid_layout = container_layout_from_text(&gui, "raid_map_icon", MAPICONS_GUI_SOURCE)?;
        require_authored_order(
            &raid_layout,
            &["raid_map_icon_frame", "raid_map_icon_button", "plus_icon"],
        )?;
        Ok(Self {
            victory_point: parse("victory_point_mapicon")?,
            capital: parse("capital_mapicon")?,
            resource: parse_container_geometry(&gui, "resource_mapicon")?,
            resource_item: parse_container_geometry(&gui, "resource_mapicon_item")?,
            resource_flag_offset: parse_control_offset(&gui, "resource_mapicon", "flag")?,
            resource_item_flag_offset: parse_control_offset(&gui, "resource_mapicon_item", "flag")?,
            resource_item_desc: MapIconTextGeometry {
                offset_x: desc_position.x,
                offset_y: desc_position.y,
                max_width: desc_max_width,
                max_height: desc_max_height,
            },
            raid: parse_raid_geometry(&gui, "raid_map_icon", "raid_map_icon_button_container")?,
        })
    }
}

fn require_authored_order(
    layout: &crate::unit_counter_assets::GuiContainerLayout,
    names: &[&str],
) -> Result<()> {
    let mut previous = None;
    for name in names {
        let ordinal = layout
            .controls
            .iter()
            .position(|control| control.name == *name)
            .ok_or_else(|| format!("{} has no authored {name} child", layout.name))?;
        if previous.is_some_and(|previous| previous >= ordinal) {
            return Err(format!("{} authored child order changed", layout.name).into());
        }
        previous = Some(ordinal);
    }
    Ok(())
}

fn parse_control_offset(gui: &str, root_name: &str, child_name: &str) -> Result<[i32; 2]> {
    let layout = container_layout_from_text(gui, root_name, MAPICONS_GUI_SOURCE)?;
    let position = layout
        .controls
        .iter()
        .find(|control| control.name == child_name)
        .and_then(|control| control.position)
        .ok_or_else(|| format!("{root_name}.{child_name} has no authored position"))?;
    Ok([position.x, position.y])
}

fn parse_container_geometry(gui: &str, root_name: &str) -> Result<MapIconContainerGeometry> {
    let layout = container_layout_from_text(gui, root_name, MAPICONS_GUI_SOURCE)?;
    let size = layout
        .size
        .ok_or_else(|| format!("{root_name} has no authored root size"))?;
    if size.width < 0 || size.height < 0 {
        return Err(format!("{root_name} has negative authored root size").into());
    }
    Ok(MapIconContainerGeometry {
        width: size.width,
        height: size.height,
    })
}

fn parse_raid_geometry(
    gui: &str,
    root_name: &str,
    child_name: &str,
) -> Result<RaidMapIconGeometry> {
    let layout = container_layout_from_text(gui, root_name, MAPICONS_GUI_SOURCE)?;
    let size = layout
        .size
        .ok_or_else(|| format!("{root_name} has no authored root size"))?;
    if size.width < 0 || size.height < 0 {
        return Err(format!("{root_name} has negative authored root size").into());
    }
    let child = layout
        .controls
        .iter()
        .find(|control| control.name == child_name)
        .ok_or_else(|| format!("{root_name} has no authored {child_name} child"))?;
    let position = child
        .position
        .ok_or_else(|| format!("{root_name}.{child_name} has no authored position"))?;
    let child_size = child
        .size
        .ok_or_else(|| format!("{root_name}.{child_name} has no authored size"))?;
    if child_size.width < 0 || child_size.height < 0 {
        return Err(format!("{root_name}.{child_name} has negative authored size").into());
    }
    let mut content_positions = ["raid_map_icon_frame", "raid_map_icon_button", "plus_icon"]
        .into_iter()
        .map(|name| {
            layout
                .controls
                .iter()
                .find(|control| control.name == name)
                .and_then(|control| control.position)
                .ok_or_else(|| format!("{root_name} has no authored position for {name}"))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let content_position = content_positions
        .pop()
        .ok_or("raid map icon has no authored content center")?;
    if content_positions
        .iter()
        .any(|position| *position != content_position)
    {
        return Err("raid map icon child centers disagree".into());
    }
    Ok(RaidMapIconGeometry {
        root: MapIconContainerGeometry {
            width: size.width,
            height: size.height,
        },
        actionable_child: MapIconChildGeometry {
            offset_x: position.x,
            offset_y: position.y,
            width: child_size.width as u32,
            height: child_size.height as u32,
        },
        content_center_offset: [
            position.x + content_position.x,
            position.y + content_position.y,
        ],
    })
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum MapFurnitureTarget {
    VictoryPoint {
        province_id: u32,
    },
    Capital {
        country_tag: String,
        capital_state_id: u32,
    },
    ResourceState {
        state_id: u32,
    },
    Raid {
        raid_identities: Vec<RaidFurnitureIdentity>,
    },
}

impl MapFurnitureTarget {
    pub fn map_icon_bucket_ordinal(&self) -> u8 {
        match self {
            Self::VictoryPoint { .. } | Self::Capital { .. } => 2,
            Self::ResourceState { .. } => 3,
            Self::Raid { .. } => 29,
        }
    }

    pub fn canonical_id(&self) -> String {
        match self {
            Self::VictoryPoint { province_id } => format!("xmap21.vp:{province_id}"),
            Self::Capital {
                country_tag,
                capital_state_id,
            } => format!("xmap21.capital:{country_tag}:{capital_state_id}"),
            Self::ResourceState { state_id } => format!("xmap21.resource:{state_id}"),
            Self::Raid { raid_identities } => format!(
                "xmap21.raid:{}",
                raid_identities
                    .iter()
                    .map(raid_identity_token)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedMapFurnitureIcon {
    pub target: MapFurnitureTarget,
    pub controller_tag: Option<String>,
    pub current_vp: Option<i32>,
    pub capital_label_source: Option<CapitalLabelSource>,
    pub anchor_world: [f32; 3],
    pub map_icon_ordinal: u32,
}

pub fn admit_map_furniture_icons(world: &WorldFrame) -> Result<Vec<AdmittedMapFurnitureIcon>> {
    if world.map_furniture_status() != ViewStatus::Present {
        return Err(format!(
            "XMAP21 DATA_BLOCKED: map_furniture status is {:?}",
            world.map_furniture_status()
        )
        .into());
    }
    let carrier = world
        .map_furniture()
        .ok_or("XMAP21 DATA_BLOCKED: Present map_furniture has no typed carrier")?;
    let roster = world.d18_country_roster();
    if roster.status != ViewStatus::Present {
        return Err(format!(
            "XMAP21 DATA_BLOCKED: complete country identity roster is {:?}",
            roster.status
        )
        .into());
    }
    let roster_by_tag: BTreeMap<&str, u32> = roster
        .entries
        .iter()
        .map(|entry| (entry.tag.as_str(), entry.tag_db_id))
        .collect();
    if world.diplomacy().status != ViewStatus::Present {
        return Err("XMAP21 DATA_BLOCKED: diplomacy country frontier is not Present".into());
    }
    let diplomacy_tags: BTreeSet<_> = world
        .diplomacy()
        .countries
        .keys()
        .map(String::as_str)
        .collect();
    let roster_tags: BTreeSet<_> = roster_by_tag.keys().copied().collect();
    if diplomacy_tags != roster_tags {
        return Err(
            "XMAP21 DATA_BLOCKED: diplomacy country frontier does not exactly match the complete roster"
                .into(),
        );
    }
    let mut expected_capitals = BTreeSet::new();
    for tag in roster_by_tag.keys() {
        let country = world
            .diplomacy()
            .countries
            .get(*tag)
            .ok_or("XMAP21 DATA_BLOCKED: roster country is absent from diplomacy")?;
        match &country.capital_state {
            Leaf::Measured(state_id) if *state_id != 0 => {
                expected_capitals.insert(((*tag).to_string(), *state_id));
            }
            _ => {
                return Err(format!(
                    "XMAP21 DATA_BLOCKED: roster country {tag:?} lacks a measured nonzero capital state"
                )
                .into())
            }
        }
    }
    let carried_capitals: BTreeSet<_> = carrier
        .capitals
        .iter()
        .map(|row| (row.country_tag.clone(), row.capital_state_id))
        .collect();
    require_complete_capital_frontier(&expected_capitals, &carried_capitals)?;
    let validate_controller = |role: &str, tag: Option<&str>, id: Option<u32>| -> Result<()> {
        match (tag, id) {
            (None, None) => Ok(()),
            (Some(tag), Some(id)) if roster_by_tag.get(tag) == Some(&id) => Ok(()),
            _ => Err(format!(
                "XMAP21 DATA_BLOCKED: {role} identity does not join the complete roster"
            )
            .into()),
        }
    };

    let mut icons = Vec::with_capacity(carrier.victory_points.len() + carrier.capitals.len());
    for row in &carrier.victory_points {
        validate_controller(
            "victory-point controller",
            row.controller_tag.as_deref(),
            row.controller_tag_db_id,
        )?;
        match world.control_by_province().get(&row.province_id) {
            Some(Leaf::Measured(controller))
                if row.controller_tag.as_deref() == Some(controller.as_str()) => {}
            Some(Leaf::Absent) if row.controller_tag.is_none() => {}
            _ => {
                return Err(format!(
                    "XMAP21 DATA_BLOCKED: province {} controller disagrees with map_furniture",
                    row.province_id
                )
                .into());
            }
        }
        icons.push(AdmittedMapFurnitureIcon {
            target: MapFurnitureTarget::VictoryPoint {
                province_id: row.province_id,
            },
            controller_tag: row.controller_tag.clone(),
            current_vp: Some(row.current_value),
            capital_label_source: None,
            anchor_world: row.anchor.to_f32().map_err(|error| {
                format!("XMAP21 DATA_BLOCKED: province {} {error}", row.province_id)
            })?,
            map_icon_ordinal: row.map_icon_ordinal,
        });
    }
    for row in &carrier.capitals {
        if roster_by_tag.get(row.country_tag.as_str()) != Some(&row.country_tag_db_id) {
            return Err(format!(
                "XMAP21 DATA_BLOCKED: capital country {} does not join the complete roster",
                row.country_tag
            )
            .into());
        }
        validate_controller(
            "capital controller",
            row.controller_tag.as_deref(),
            row.controller_tag_db_id,
        )?;
        let current_capital = world
            .diplomacy()
            .countries
            .get(&row.country_tag)
            .map(|country| &country.capital_state);
        if !matches!(
            current_capital,
            Some(Leaf::Measured(state_id)) if *state_id == row.capital_state_id
        ) {
            return Err(format!(
                "XMAP21 DATA_BLOCKED: {} capital state disagrees with map_furniture",
                row.country_tag
            )
            .into());
        }
        match world.control_by_state().get(&row.capital_state_id) {
            Some(Leaf::Measured(controller))
                if row.controller_tag.as_deref() == Some(controller.as_str()) => {}
            Some(Leaf::Absent) if row.controller_tag.is_none() => {}
            _ => {
                return Err(format!(
                    "XMAP21 DATA_BLOCKED: capital state {} controller disagrees with map_furniture",
                    row.capital_state_id
                )
                .into());
            }
        }
        icons.push(AdmittedMapFurnitureIcon {
            target: MapFurnitureTarget::Capital {
                country_tag: row.country_tag.clone(),
                capital_state_id: row.capital_state_id,
            },
            controller_tag: row.controller_tag.clone(),
            current_vp: None,
            capital_label_source: Some(row.label_source),
            anchor_world: row.anchor.to_f32().map_err(|error| {
                format!("XMAP21 DATA_BLOCKED: capital {} {error}", row.country_tag)
            })?,
            map_icon_ordinal: row.map_icon_ordinal,
        });
    }
    Ok(icons)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapFurnitureDistanceDefines {
    pub resource_text_cutoff: f32,
    pub raid_min: f32,
    pub raid_max: f32,
    pub raid_map_mode_max: f32,
}

impl MapFurnitureDistanceDefines {
    pub fn from_resolver(resolver: &GameFileResolver) -> Result<Self> {
        const SOURCE: &str = "common/defines/00_graphics.lua";
        let path = resolver.resolve_required(SOURCE, "XMAP21 graphics defines")?;
        let bytes = fs::read(&path)?;
        let source: String = bytes.into_iter().map(char::from).collect();
        let parsed = Self {
            resource_text_cutoff: crate::camera::parse_lua_number_define(
                &source,
                RESOURCE_MAP_ICON_TEXT_CUTOFF_KEY,
            )?,
            raid_min: crate::camera::parse_lua_number_define(
                &source,
                RAID_MAP_ICON_DRAW_DISTANCE_MIN_KEY,
            )?,
            raid_max: crate::camera::parse_lua_number_define(
                &source,
                RAID_MAP_ICON_DRAW_DISTANCE_MAX_KEY,
            )?,
            raid_map_mode_max: crate::camera::parse_lua_number_define(
                &source,
                RAID_MAP_ICON_DRAW_DISTANCE_MAX_RAID_MAP_MODE_KEY,
            )?,
        };
        if !parsed.resource_text_cutoff.is_finite()
            || parsed.resource_text_cutoff <= 0.0
            || !parsed.raid_min.is_finite()
            || parsed.raid_min < 0.0
            || !parsed.raid_max.is_finite()
            || parsed.raid_max <= parsed.raid_min
            || !parsed.raid_map_mode_max.is_finite()
            || parsed.raid_map_mode_max < parsed.raid_max
        {
            return Err("XMAP21 graphics distance defines are invalid".into());
        }
        Ok(parsed)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MapModeFurnitureSelection {
    pub update_index: u64,
    pub selection_epoch: u64,
    pub mode_id: String,
    pub viewer_token: String,
}

impl MapModeFurnitureSelection {
    pub fn validate_for_world(&self, world: &WorldFrame) -> Result<()> {
        if u64::try_from(world.hour_index()).ok() != Some(self.update_index) {
            return Err(
                "XMAP21 map-mode furniture selection belongs to a different WorldFrame".into(),
            );
        }
        let agrees = matches!(
            (self.mode_id.as_str(), self.viewer_token.as_str()),
            (XMAP21_RESOURCE_MODE_ID, XMAP21_RESOURCE_VIEWER_TOKEN)
                | (XMAP21_RAID_MODE_ID, XMAP21_RAID_VIEWER_TOKEN)
        ) || (self.mode_id != XMAP21_RESOURCE_MODE_ID
            && self.mode_id != XMAP21_RAID_MODE_ID
            && self.viewer_token != XMAP21_RESOURCE_VIEWER_TOKEN
            && self.viewer_token != XMAP21_RAID_VIEWER_TOKEN);
        if !agrees {
            return Err("XMAP21 map-mode id and viewer token disagree".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MapFurnitureSharedIdentities {
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
}

impl MapFurnitureSharedIdentities {
    pub fn validate(&self) -> Result<()> {
        validate_lower_sha256(&self.content_manifest_sha256, "content manifest")?;
        validate_lower_sha256(
            &self.foundation_content_operation_sha256,
            "foundation content operation",
        )?;
        validate_lower_sha256(&self.viewer_state_sha256, "viewer state")?;
        validate_lower_sha256(&self.main_scene_target_sha256, "main-scene target")?;
        validate_lower_sha256(&self.map_copy_receipt_set_sha256, "map-copy receipt set")
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MapFurniturePrepareRequest {
    pub selection: MapModeFurnitureSelection,
    pub viewed_country_tag: Option<String>,
    pub viewed_country_tag_db_id: Option<u32>,
    pub shared: MapFurnitureSharedIdentities,
    pub prepare_assignment: crate::AssignedStage,
    pub execute_assignment: crate::AssignedStage,
    pub deterministic_sha256: String,
}

impl MapFurniturePrepareRequest {
    pub fn seal(
        selection: MapModeFurnitureSelection,
        viewed_country_tag: Option<String>,
        viewed_country_tag_db_id: Option<u32>,
        shared: MapFurnitureSharedIdentities,
        prepare_assignment: crate::AssignedStage,
        execute_assignment: crate::AssignedStage,
    ) -> Result<Self> {
        let mut request = Self {
            selection,
            viewed_country_tag,
            viewed_country_tag_db_id,
            shared,
            prepare_assignment,
            execute_assignment,
            deterministic_sha256: String::new(),
        };
        request.validate_body()?;
        request.deterministic_sha256 = canonical_sha256(&request)?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        validate_lower_sha256(&self.deterministic_sha256, "prepare request")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != canonical_sha256(&unsigned)? {
            return Err("XMAP21 prepare request digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        match (
            self.viewed_country_tag.as_deref(),
            self.viewed_country_tag_db_id,
        ) {
            (None, None) => {}
            (Some(tag), Some(id))
                if id > 0
                    && !tag.is_empty()
                    && tag.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    }) => {}
            _ => {
                return Err("XMAP21 prepare request has an invalid viewed-country identity".into())
            }
        }
        self.shared.validate()?;
        self.prepare_assignment
            .validate()
            .map_err(|error| format!("XMAP21 prepare assignment: {error}"))?;
        self.execute_assignment
            .validate()
            .map_err(|error| format!("XMAP21 execute assignment: {error}"))?;
        if self.prepare_assignment.stage_name != XMAP21_PREPARE_STAGE_NAME
            || self.prepare_assignment.operation_name != XMAP21_PREPARE_OPERATION_NAME
            || self.execute_assignment.stage_name != XMAP21_EXECUTE_STAGE_NAME
            || self.execute_assignment.operation_name != XMAP21_EXECUTE_OPERATION_NAME
            || self.prepare_assignment.topology_sha256 != self.execute_assignment.topology_sha256
            || self.prepare_assignment.assigned_ordinal >= self.execute_assignment.assigned_ordinal
        {
            return Err("XMAP21 assigned-stage pair is not the frozen prepare/execute edge".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MapFurniturePacketDisposition {
    Present,
    AuthenticatedEmpty,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedMapFurniturePacket {
    pub schema: String,
    pub update_index: u64,
    pub selection_epoch: u64,
    pub mode_id: String,
    pub viewer_token: String,
    pub disposition: MapFurniturePacketDisposition,
    pub request_sha256: String,
    pub shared: MapFurnitureSharedIdentities,
    pub prepare_assignment: crate::AssignedStage,
    pub execute_assignment: crate::AssignedStage,
    pub prepare_operation_identity: crate::FoundationOperationIdentity,
    pub victory_points_and_capitals: Vec<ProjectedMapFurnitureIcon>,
    pub resources: Vec<ProjectedResourceFurnitureIcon>,
    pub raids: Vec<ProjectedRaidFurnitureIcon>,
    pub vp_capital_text: Xmap21VpCapitalTextBatch,
    pub resource_text: Option<Xmap21ResourceTextBatch>,
    pub resource_flag_masked_shield: Option<MapFurnitureMaskedShieldDescriptor>,
    pub resource_flag_assets: Vec<MapFurnitureResourceFlagAssetDescriptor>,
    pub vp_strip_descriptor_sha256: String,
    pub raid_sprite_descriptor_set_sha256: String,
    pub stable_ids_sha256: String,
    pub payload_sha256: String,
    pub deterministic_sha256: String,
}

impl PreparedMapFurniturePacket {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP21_PREPARED_PACKET_SCHEMA {
            return Err("XMAP21 prepared packet schema mismatch".into());
        }
        validate_lower_sha256(&self.request_sha256, "prepared request")?;
        self.shared.validate()?;
        self.prepare_assignment
            .validate()
            .map_err(|error| format!("XMAP21 prepared assignment: {error}"))?;
        self.execute_assignment
            .validate()
            .map_err(|error| format!("XMAP21 execute assignment: {error}"))?;
        if self.prepare_operation_identity.operation_id != XMAP21_PREPARE_OPERATION_NAME
            || self.prepare_operation_identity.global_order
                != self.prepare_assignment.assigned_ordinal
        {
            return Err("XMAP21 prepare operation identity does not echo AssignedStage".into());
        }
        validate_lower_sha256(&self.stable_ids_sha256, "stable-id set")?;
        validate_lower_sha256(
            &self.raid_sprite_descriptor_set_sha256,
            "raid sprite descriptor set",
        )?;
        validate_lower_sha256(&self.vp_strip_descriptor_sha256, "VP strip descriptor")?;
        validate_lower_sha256(&self.payload_sha256, "prepared payload")?;
        validate_lower_sha256(&self.deterministic_sha256, "prepared packet")?;
        self.vp_capital_text.validate()?;
        if self.vp_capital_text.update_index != self.update_index
            || self.vp_capital_text.selection_epoch != self.selection_epoch
            || self.vp_capital_text.map_mode_id != self.mode_id
            || self.vp_capital_text.viewer_token != self.viewer_token
        {
            return Err("XMAP21 VP/capital text batch belongs to another selection".into());
        }
        validate_vp_capital_text_frontier(
            &self.victory_points_and_capitals,
            &self.vp_capital_text,
        )?;
        if let Some(batch) = &self.resource_text {
            batch.validate()?;
            if batch.update_index != self.update_index
                || batch.selection_epoch != self.selection_epoch
            {
                return Err("XMAP21 resource text batch belongs to another selection".into());
            }
        }
        let required_flag_paths: BTreeSet<_> = self
            .resources
            .iter()
            .filter_map(|icon| icon.controller_flag_texture_path.as_deref())
            .collect();
        let carried_flag_paths: BTreeSet<_> = self
            .resource_flag_assets
            .iter()
            .map(|asset| asset.logical_path.as_str())
            .collect();
        if required_flag_paths != carried_flag_paths
            || (required_flag_paths.is_empty() != self.resource_flag_masked_shield.is_none())
        {
            return Err("XMAP21 resource flag asset frontier is incomplete".into());
        }
        if let Some(shield) = &self.resource_flag_masked_shield {
            shield.validate()?;
        }
        for asset in &self.resource_flag_assets {
            if asset.logical_path.is_empty()
                || asset.source_encoding != "tga"
                || asset.sampling_law != XMAP21_RESOURCE_FLAG_SAMPLING_LAW
                || !std::path::Path::new(&asset.logical_path)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("tga"))
            {
                return Err(
                    "XMAP21 resource flag asset lacks the direct-D3D11 TGA sampling contract"
                        .into(),
                );
            }
            validate_lower_sha256(&asset.content_sha256, "resource controller flag")?;
        }
        let is_empty = self.victory_points_and_capitals.is_empty()
            && self.resources.is_empty()
            && self.raids.is_empty()
            && self.vp_capital_text.operations.is_empty()
            && self
                .resource_text
                .as_ref()
                .is_none_or(|batch| batch.operations.is_empty());
        if (self.disposition == MapFurniturePacketDisposition::AuthenticatedEmpty) != is_empty {
            return Err("XMAP21 packet disposition does not match payload emptiness".into());
        }
        let stable_ids = map_furniture_stable_ids(
            &self.victory_points_and_capitals,
            &self.resources,
            &self.raids,
        );
        if self.stable_ids_sha256 != canonical_sha256(&stable_ids)? {
            return Err("XMAP21 stable-id set digest mismatch".into());
        }
        if self.payload_sha256
            != map_furniture_payload_sha256(
                &self.victory_points_and_capitals,
                &self.resources,
                &self.raids,
                &self.vp_capital_text,
                self.resource_text.as_ref(),
                self.resource_flag_masked_shield.as_ref(),
                &self.resource_flag_assets,
                &self.vp_strip_descriptor_sha256,
                &self.raid_sprite_descriptor_set_sha256,
            )?
        {
            return Err("XMAP21 prepared payload digest mismatch".into());
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != canonical_sha256(&unsigned)? {
            return Err("XMAP21 prepared packet digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedResourceFurnitureEntry {
    pub resource_name: String,
    pub resource_definition_ordinal: u32,
    pub total_amount: i64,
    pub available_amount: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedResourceFurnitureIcon {
    pub state_id: u32,
    pub controller_tag: Option<String>,
    pub controller_flag_texture_path: Option<String>,
    pub anchor_world: [f32; 3],
    pub entries: Vec<AdmittedResourceFurnitureEntry>,
    pub map_icon_ordinal: u32,
}

pub fn admit_resource_furniture_icons(
    world: &WorldFrame,
    resolver: &GameFileResolver,
    viewed_country: Option<&str>,
) -> Result<Vec<AdmittedResourceFurnitureIcon>> {
    let carrier = require_map_furniture_carrier(world)?;
    let roster = require_complete_roster(world)?;
    validate_viewed_country(viewed_country, &roster)?;
    let roster_pairs: BTreeSet<_> = roster.iter().map(|(tag, id)| (tag.as_str(), *id)).collect();
    let definitions = resource_definitions_from_resolver(resolver)?;
    let definition_ordinals: BTreeMap<_, _> = definitions
        .iter()
        .enumerate()
        .map(|(ordinal, definition)| (definition.name.as_str(), ordinal as u32))
        .collect();
    let mut admitted = Vec::with_capacity(carrier.resources.len());
    for row in &carrier.resources {
        validate_controller_join(
            "resource-state controller",
            row.controller_tag.as_deref(),
            row.controller_tag_db_id,
            &roster,
        )?;
        require_state_controller(
            world,
            row.state_id,
            row.controller_tag.as_deref(),
            "resource state",
        )?;
        if let Some(path) = row.controller_flag_texture_path.as_deref() {
            resolver.resolve_required(path, "resource controller flag")?;
        }
        let mut entries = Vec::with_capacity(row.entries.len());
        for entry in &row.entries {
            if definition_ordinals.get(entry.resource_name.as_str())
                != Some(&entry.resource_definition_ordinal)
            {
                return Err(format!(
                    "XMAP21 DATA_BLOCKED: resource {:?} ordinal does not join effective definitions",
                    entry.resource_name
                )
                .into());
            }
            let viewer_pairs: BTreeSet<_> = entry
                .viewer_amounts
                .iter()
                .map(|value| (value.viewer_tag.as_str(), value.viewer_tag_db_id))
                .collect();
            if viewer_pairs != roster_pairs {
                return Err(format!(
                    "XMAP21 DATA_BLOCKED: resource {:?} lacks a complete country-switch frontier",
                    entry.resource_name
                )
                .into());
            }
            let available_amount = match viewed_country {
                None => 0,
                Some(viewed) => entry
                    .viewer_amounts
                    .iter()
                    .find(|value| value.viewer_tag == viewed)
                    .map(|value| value.available_amount)
                    .ok_or_else(|| {
                        format!(
                            "XMAP21 DATA_BLOCKED: resource {:?} lacks selected viewer {viewed}",
                            entry.resource_name
                        )
                    })?,
            };
            entries.push(AdmittedResourceFurnitureEntry {
                resource_name: entry.resource_name.clone(),
                resource_definition_ordinal: entry.resource_definition_ordinal,
                total_amount: entry.total_amount,
                available_amount,
            });
        }
        admitted.push(AdmittedResourceFurnitureIcon {
            state_id: row.state_id,
            controller_tag: row.controller_tag.clone(),
            controller_flag_texture_path: row.controller_flag_texture_path.clone(),
            anchor_world: row.anchor.to_f32().map_err(|error| {
                format!(
                    "XMAP21 DATA_BLOCKED: resource state {} {error}",
                    row.state_id
                )
            })?,
            entries,
            map_icon_ordinal: row.map_icon_ordinal,
        });
    }
    Ok(admitted)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedRaidFurnitureIcon {
    pub raid_identities: Vec<RaidFurnitureIdentity>,
    pub presentation: RaidFurniturePresentation,
    pub anchor_world: [f32; 3],
    pub map_icon_ordinal: u32,
}

pub fn admit_raid_furniture_icons(
    world: &WorldFrame,
    viewed_country: Option<&str>,
) -> Result<Vec<AdmittedRaidFurnitureIcon>> {
    let carrier = require_map_furniture_carrier(world)?;
    let roster = require_complete_roster(world)?;
    validate_viewed_country(viewed_country, &roster)?;
    let raids = world.raids();
    if raids.status != ViewStatus::Present || !raids.instances_complete {
        return Err(
            "XMAP21 DATA_BLOCKED: raid instances are not a complete Present frontier".into(),
        );
    }
    let Leaf::Measured(instances) = &raids.instances else {
        return Err("XMAP21 DATA_BLOCKED: raid instances are not measured".into());
    };
    let instance_keys: BTreeSet<_> = instances
        .iter()
        .map(|instance| RaidFurnitureIdentity {
            producer_ordinal: instance.producer_ordinal,
            status_index: instance.status_index,
            raid_index: instance.raid_index,
            id: instance.id,
            id_type: instance.id_type,
        })
        .collect();
    let roster_pairs: BTreeSet<_> = roster
        .iter()
        .map(|(tag, id)| (Some(tag.as_str()), Some(*id)))
        .chain(std::iter::once((None, None)))
        .collect();
    let mut admitted = Vec::with_capacity(carrier.raids.len());
    let carried_instance_keys: BTreeSet<_> = carrier
        .raids
        .iter()
        .flat_map(|row| row.raid_identities.iter().cloned())
        .collect();
    require_complete_raid_frontier(&instance_keys, &carried_instance_keys)?;
    for row in &carrier.raids {
        if row
            .raid_identities
            .iter()
            .any(|identity| !instance_keys.contains(identity))
        {
            return Err(
                "XMAP21 DATA_BLOCKED: raid furniture identity does not join raid instances".into(),
            );
        }
        let presentation_pairs: BTreeSet<_> = row
            .presentations
            .iter()
            .map(|presentation| {
                (
                    presentation.viewer_tag.as_deref(),
                    presentation.viewer_tag_db_id,
                )
            })
            .collect();
        if presentation_pairs != roster_pairs {
            return Err(
                "XMAP21 DATA_BLOCKED: raid icon lacks complete no-viewer/all-country presentations"
                    .into(),
            );
        }
        let presentation = row
            .presentations
            .iter()
            .find(|presentation| presentation.viewer_tag.as_deref() == viewed_country)
            .cloned()
            .ok_or("XMAP21 DATA_BLOCKED: raid icon lacks selected presentation")?;
        admitted.push(AdmittedRaidFurnitureIcon {
            raid_identities: row.raid_identities.clone(),
            presentation,
            anchor_world: row
                .anchor
                .to_f32()
                .map_err(|error| format!("XMAP21 DATA_BLOCKED: raid anchor {error}"))?,
            map_icon_ordinal: row.map_icon_ordinal,
        });
    }
    Ok(admitted)
}

fn require_complete_capital_frontier(
    expected: &BTreeSet<(String, u32)>,
    carried: &BTreeSet<(String, u32)>,
) -> Result<()> {
    if expected != carried {
        return Err(
            "XMAP21 DATA_BLOCKED: capital carrier is not the complete measured current-capital frontier"
                .into(),
        );
    }
    Ok(())
}

fn require_complete_raid_frontier(
    expected: &BTreeSet<RaidFurnitureIdentity>,
    carried: &BTreeSet<RaidFurnitureIdentity>,
) -> Result<()> {
    if expected != carried {
        return Err(
            "XMAP21 DATA_BLOCKED: raid furniture is not the complete D61 instance frontier".into(),
        );
    }
    Ok(())
}

fn require_map_furniture_carrier(world: &WorldFrame) -> Result<&replay_core::MapFurnitureFrame> {
    if world.map_furniture_status() != ViewStatus::Present {
        return Err(format!(
            "XMAP21 DATA_BLOCKED: map_furniture status is {:?}",
            world.map_furniture_status()
        )
        .into());
    }
    world
        .map_furniture()
        .ok_or_else(|| "XMAP21 DATA_BLOCKED: Present map_furniture has no carrier".into())
}

fn require_complete_roster(world: &WorldFrame) -> Result<Vec<(String, u32)>> {
    let roster = world.d18_country_roster();
    if roster.status != ViewStatus::Present {
        return Err("XMAP21 DATA_BLOCKED: country roster is not Present".into());
    }
    Ok(roster
        .entries
        .iter()
        .map(|entry| (entry.tag.clone(), entry.tag_db_id))
        .collect())
}

fn validate_viewed_country(viewed_country: Option<&str>, roster: &[(String, u32)]) -> Result<()> {
    if viewed_country.is_some_and(|viewed| !roster.iter().any(|(tag, _)| tag == viewed)) {
        return Err("XMAP21 selected viewer country is not in the complete roster".into());
    }
    Ok(())
}

fn validate_controller_join(
    role: &str,
    tag: Option<&str>,
    id: Option<u32>,
    roster: &[(String, u32)],
) -> Result<()> {
    match (tag, id) {
        (None, None) => Ok(()),
        (Some(tag), Some(id)) if roster.iter().any(|pair| pair == &(tag.to_string(), id)) => Ok(()),
        _ => Err(format!("XMAP21 DATA_BLOCKED: {role} does not join country roster").into()),
    }
}

fn require_state_controller(
    world: &WorldFrame,
    state_id: u32,
    controller: Option<&str>,
    role: &str,
) -> Result<()> {
    match world.control_by_state().get(&state_id) {
        Some(Leaf::Measured(current)) if controller == Some(current.as_str()) => Ok(()),
        Some(Leaf::Absent) if controller.is_none() => Ok(()),
        _ => Err(format!(
            "XMAP21 DATA_BLOCKED: {role} {state_id} controller disagrees with map_furniture"
        )
        .into()),
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScreenRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl ScreenRect {
    pub fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.left
            && point[0] <= self.right
            && point[1] >= self.top
            && point[1] <= self.bottom
    }

    fn intersects_viewport(self, width: f32, height: f32) -> bool {
        self.right >= 0.0 && self.bottom >= 0.0 && self.left <= width && self.top <= height
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectedMapFurnitureIcon {
    pub target: MapFurnitureTarget,
    pub capital_label_source: Option<CapitalLabelSource>,
    pub root_hit_rect: ScreenRect,
    pub icon_rect: ScreenRect,
    pub dot_visible: bool,
    pub text_visible: bool,
    pub strip_frame_index: u32,
    pub projection_depth: f32,
    pub engine_sort_key: f32,
    pub map_icon_ordinal: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectedResourceFurnitureIcon {
    pub state_id: u32,
    pub controller_tag: Option<String>,
    pub controller_flag_texture_path: Option<String>,
    pub row_count: u32,
    pub root_hit_rect: ScreenRect,
    pub projection_depth: f32,
    pub engine_sort_key: f32,
    pub map_icon_ordinal: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap21ResourceTextOperation {
    pub state_id: u32,
    pub resource_definition_ordinal: u32,
    pub stable_id: String,
    pub font_token: String,
    pub resolved_text: String,
    pub rect_px: [f32; 4],
    pub max_width: i32,
    pub max_height: i32,
    pub format: String,
    pub scale_f32_bits: u32,
    pub parent_map_icon_ordinal: u32,
    pub row_index: u32,
    pub root_projection_sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap21ResourceTextBatch {
    pub schema: String,
    pub update_index: u64,
    pub selection_epoch: u64,
    pub map_mode_id: String,
    pub viewer_token: String,
    pub prepare_stage_name: String,
    pub prepare_operation_name: String,
    pub operations: Vec<Xmap21ResourceTextOperation>,
    pub deterministic_sha256: String,
}

impl Xmap21ResourceTextBatch {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP21_RESOURCE_TEXT_BATCH_SCHEMA
            || self.map_mode_id != XMAP21_RESOURCE_MODE_ID
            || self.viewer_token != XMAP21_RESOURCE_VIEWER_TOKEN
            || self.prepare_stage_name != "xmap21.map_furniture.prepare"
            || self.prepare_operation_name != "map_furniture.prepare"
        {
            return Err("XMAP21 resource text batch identity is invalid".into());
        }
        let mut stable_ids = BTreeSet::new();
        for operation in &self.operations {
            if operation.font_token != "hoi_16mbs"
                || operation.format != "left"
                || operation.scale_f32_bits != 1.0f32.to_bits()
                || !stable_ids.insert(operation.stable_id.as_str())
            {
                return Err("XMAP21 resource text operation identity is invalid".into());
            }
            validate_lower_sha256(
                &operation.root_projection_sha256,
                "resource root projection",
            )?;
            if operation
                .rect_px
                .iter()
                .any(|coordinate| !coordinate.is_finite())
            {
                return Err("XMAP21 resource text rectangle is non-finite".into());
            }
        }
        validate_lower_sha256(&self.deterministic_sha256, "resource text batch")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != canonical_sha256(&unsigned)? {
            return Err("XMAP21 resource text batch digest mismatch".into());
        }
        Ok(())
    }
}

fn exact_localisation_values(
    resolver: &GameFileResolver,
    language: &str,
    required: &BTreeSet<String>,
) -> Result<BTreeMap<String, String>> {
    let language = language.trim().to_ascii_lowercase();
    let directory = language.strip_prefix("l_").unwrap_or(&language);
    if directory.is_empty()
        || !directory
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err("XMAP21 DATA_BLOCKED: invalid localisation language identity".into());
    }
    let expected_header = format!("l_{directory}:");
    let localisation_dir = format!("localisation/{directory}");
    let admitted_localisation = resolver.require_complete_admitted_dir(
        &localisation_dir,
        Some("yml"),
        false,
        "XMAP21 selected-language localisation namespace",
    )?;
    if admitted_localisation.is_empty() {
        return Err(
            format!("XMAP21 DATA_BLOCKED: effective localisation/{directory} is empty").into(),
        );
    }
    if required.is_empty() {
        return Ok(BTreeMap::new());
    }
    let files = resolver.resolve_dir_files(&localisation_dir, Some("yml"))?;
    if files.is_empty() {
        return Err(
            format!("XMAP21 DATA_BLOCKED: effective localisation/{directory} is empty").into(),
        );
    }
    let mut values = BTreeMap::new();
    for (relative, path) in files {
        let bytes = fs::read(&path)?;
        let source = String::from_utf8(bytes).map_err(|error| {
            format!("XMAP21 DATA_BLOCKED: localisation {relative} is not UTF-8: {error}")
        })?;
        let header = source
            .lines()
            .map(|line| line.trim_start_matches('\u{feff}').trim())
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .ok_or_else(|| format!("XMAP21 DATA_BLOCKED: localisation {relative} is empty"))?;
        if !header.eq_ignore_ascii_case(&expected_header) {
            return Err(format!(
                "XMAP21 DATA_BLOCKED: localisation {relative} header {header:?} is not {expected_header:?}"
            )
            .into());
        }
        for (line_index, raw_line) in source.lines().enumerate() {
            let line = raw_line.trim_start_matches('\u{feff}').trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.eq_ignore_ascii_case(&expected_header)
            {
                continue;
            }
            let Some(colon) = line.find(':') else {
                continue;
            };
            let key = line[..colon].trim();
            if !required.contains(key) {
                continue;
            }
            let mut rest = line[colon + 1..].trim_start();
            rest = rest.trim_start_matches(|character: char| character.is_ascii_digit());
            rest = rest.trim_start();
            let quoted = rest.strip_prefix('"').ok_or_else(|| {
                format!(
                    "XMAP21 DATA_BLOCKED: {relative}:{} key {key:?} is not quoted",
                    line_index + 1
                )
            })?;
            // ONE rule for where a localisation value ends, shared with
            // `map_text` -- vanilla ships unescaped inner quotes and both
            // readers must accept the same files the game does.
            let closing = crate::map_text::closing_quote_index(quoted).ok_or_else(|| {
                format!(
                    "XMAP21 DATA_BLOCKED: {relative}:{} key {key:?} has no closing quote, or text after the value that is neither a comment nor part of it",
                    line_index + 1
                )
            })?;
            let value = &quoted[..closing];
            if values.insert(key.to_string(), value.to_string()).is_some() {
                return Err(format!(
                    "XMAP21 DATA_BLOCKED: localisation key {key:?} has multiple effective definitions"
                )
                .into());
            }
        }
    }
    for key in required {
        if !values.contains_key(key) {
            return Err(
                format!("XMAP21 DATA_BLOCKED: localisation lacks required key {key:?}").into(),
            );
        }
    }
    Ok(values)
}

fn require_flat_instant_text(value: &str, role: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.contains('$')
        || value.contains('£')
        || value.contains('§')
        || value.contains('\u{0011}')
        || value.contains('[')
        || value.contains(']')
        || value.contains('\\')
    {
        return Err(format!(
            "XMAP21 DATA_BLOCKED: {role} requires unresolved runtime or formatted localisation"
        )
        .into());
    }
    Ok(value.to_string())
}

pub fn project_resource_furniture(
    world: &WorldFrame,
    resolver: &GameFileResolver,
    language: &str,
    selection: &MapModeFurnitureSelection,
    viewed_country: Option<&str>,
    gui: MapFurnitureGuiGeometry,
    distance_defines: MapFurnitureDistanceDefines,
    camera: &ProductionMapCamera,
    screen_width: f32,
    screen_height: f32,
) -> Result<(Vec<ProjectedResourceFurnitureIcon>, Xmap21ResourceTextBatch)> {
    selection.validate_for_world(world)?;
    if selection.mode_id != XMAP21_RESOURCE_MODE_ID
        || selection.viewer_token != XMAP21_RESOURCE_VIEWER_TOKEN
    {
        return Err(
            "XMAP21 resource projection requires the exact Resources mode token pair".into(),
        );
    }
    validate_viewport(screen_width, screen_height)?;
    let admitted = admit_resource_furniture_icons(world, resolver, viewed_country)?;
    let localisation = exact_localisation_values(
        resolver,
        language,
        &BTreeSet::from(["MAP_RESOURCE_ICON_VAL".into(), "MAP_ICON_NAME".into()]),
    )?;
    let amount_template = localisation
        .get("MAP_RESOURCE_ICON_VAL")
        .ok_or("XMAP21 DATA_BLOCKED: localisation lacks MAP_RESOURCE_ICON_VAL")?;
    let wrapper_template = localisation
        .get("MAP_ICON_NAME")
        .ok_or("XMAP21 DATA_BLOCKED: localisation lacks MAP_ICON_NAME")?;
    let mut projected = Vec::new();
    let mut text = Vec::new();
    if camera.position[1] < distance_defines.resource_text_cutoff {
        for icon in admitted {
            let Some([screen_x, screen_y, depth]) =
                camera.world_to_screen_depth(icon.anchor_world, screen_width, screen_height)
            else {
                continue;
            };
            let left = (screen_x - gui.resource.width as f32 * 0.5).round();
            let top = (screen_y - gui.resource.height as f32 * 0.5).round();
            let root_hit_rect = ScreenRect {
                left,
                top,
                right: left + gui.resource.width as f32,
                bottom: top + gui.resource.height as f32,
            };
            if !root_hit_rect.intersects_viewport(screen_width, screen_height) {
                continue;
            }
            let root_projection_sha256 = canonical_sha256(&(
                icon.state_id,
                root_hit_rect,
                depth.to_bits(),
                icon.map_icon_ordinal,
            ))?;
            for (row_index, entry) in icon.entries.iter().enumerate() {
                let amount = amount_template
                    .replace("$VAL$", &entry.available_amount.to_string())
                    .replace("$MAX$", &entry.total_amount.to_string());
                let resolved_text = require_flat_instant_text(
                    &wrapper_template.replace("$VAL$", &amount),
                    "resource map icon",
                )?;
                let row_top = top + row_index as f32 * gui.resource_item.height as f32;
                let desc_left = left + gui.resource_item_desc.offset_x as f32;
                let desc_top = row_top + gui.resource_item_desc.offset_y as f32;
                text.push(Xmap21ResourceTextOperation {
                    state_id: icon.state_id,
                    resource_definition_ordinal: entry.resource_definition_ordinal,
                    stable_id: format!(
                        "state:{}:resource:{}",
                        icon.state_id, entry.resource_definition_ordinal
                    ),
                    font_token: "hoi_16mbs".into(),
                    resolved_text,
                    rect_px: [
                        desc_left,
                        desc_top,
                        desc_left + gui.resource_item_desc.max_width as f32,
                        desc_top + gui.resource_item_desc.max_height as f32,
                    ],
                    max_width: gui.resource_item_desc.max_width,
                    max_height: gui.resource_item_desc.max_height,
                    format: "left".into(),
                    scale_f32_bits: 1.0f32.to_bits(),
                    parent_map_icon_ordinal: icon.map_icon_ordinal,
                    row_index: row_index as u32,
                    root_projection_sha256: root_projection_sha256.clone(),
                });
            }
            projected.push(ProjectedResourceFurnitureIcon {
                state_id: icon.state_id,
                controller_tag: icon.controller_tag,
                controller_flag_texture_path: icon.controller_flag_texture_path,
                row_count: icon.entries.len() as u32,
                root_hit_rect,
                projection_depth: depth,
                engine_sort_key: (depth - gui.resource.height as f32) * XMAP21_SORT_DEPTH_SCALE
                    + icon.map_icon_ordinal as f32,
                map_icon_ordinal: icon.map_icon_ordinal,
            });
        }
    }
    sort_attachment_order(&mut projected, |icon| {
        (icon.engine_sort_key, icon.map_icon_ordinal)
    });
    let attachment_rank: BTreeMap<_, _> = projected
        .iter()
        .enumerate()
        .map(|(rank, icon)| (icon.map_icon_ordinal, rank))
        .collect();
    text.sort_by_key(|operation| {
        (
            attachment_rank
                .get(&operation.parent_map_icon_ordinal)
                .copied()
                .unwrap_or(usize::MAX),
            operation.row_index,
        )
    });
    Ok((projected, seal_resource_text_batch(selection, text)?))
}

fn seal_resource_text_batch(
    selection: &MapModeFurnitureSelection,
    operations: Vec<Xmap21ResourceTextOperation>,
) -> Result<Xmap21ResourceTextBatch> {
    let mut batch = Xmap21ResourceTextBatch {
        schema: XMAP21_RESOURCE_TEXT_BATCH_SCHEMA.into(),
        update_index: selection.update_index,
        selection_epoch: selection.selection_epoch,
        map_mode_id: XMAP21_RESOURCE_MODE_ID.into(),
        viewer_token: XMAP21_RESOURCE_VIEWER_TOKEN.into(),
        prepare_stage_name: "xmap21.map_furniture.prepare".into(),
        prepare_operation_name: "map_furniture.prepare".into(),
        operations,
        deterministic_sha256: String::new(),
    };
    batch.deterministic_sha256 = canonical_sha256(&batch)?;
    batch.validate()?;
    Ok(batch)
}

fn build_vp_capital_text_batch(
    resolver: &GameFileResolver,
    language: &str,
    selection: &MapModeFurnitureSelection,
    gui: MapFurnitureGuiGeometry,
    projected: &[ProjectedMapFurnitureIcon],
) -> Result<Xmap21VpCapitalTextBatch> {
    let needs_state_names = projected.iter().any(|icon| {
        icon.text_visible
            && matches!(
                icon.capital_label_source,
                Some(CapitalLabelSource::State { .. })
            )
    });
    let states = if needs_state_names {
        let files = resolver.resolve_dir_files("history/states", Some("txt"))?;
        crate::game_data::load_states(&files)?.0
    } else {
        BTreeMap::new()
    };
    let mut required_keys = BTreeSet::new();
    for icon in projected.iter().filter(|icon| icon.text_visible) {
        let key = match (&icon.target, icon.capital_label_source) {
            (MapFurnitureTarget::VictoryPoint { province_id }, None) => {
                format!("VICTORY_POINTS_{province_id}")
            }
            (
                MapFurnitureTarget::Capital { .. },
                Some(CapitalLabelSource::Province { province_id }),
            ) => format!("VICTORY_POINTS_{province_id}"),
            (MapFurnitureTarget::Capital { .. }, Some(CapitalLabelSource::State { state_id })) => {
                states
                    .get(&state_id)
                    .ok_or_else(|| {
                        format!("XMAP21 DATA_BLOCKED: capital label state {state_id} is absent")
                    })?
                    .name_key
                    .clone()
            }
            _ => {
                return Err(
                    "XMAP21 DATA_BLOCKED: VP/capital label source disagrees with icon kind".into(),
                )
            }
        };
        required_keys.insert(key);
    }
    let localisation = exact_localisation_values(resolver, language, &required_keys)?;
    let mut operations = Vec::new();
    for icon in projected.iter().filter(|icon| icon.text_visible) {
        let (localisation_key, geometry) = match (&icon.target, icon.capital_label_source) {
            (MapFurnitureTarget::VictoryPoint { province_id }, None) => {
                (format!("VICTORY_POINTS_{province_id}"), gui.victory_point)
            }
            (
                MapFurnitureTarget::Capital { .. },
                Some(CapitalLabelSource::Province { province_id }),
            ) => (format!("VICTORY_POINTS_{province_id}"), gui.capital),
            (MapFurnitureTarget::Capital { .. }, Some(CapitalLabelSource::State { state_id })) => (
                states
                    .get(&state_id)
                    .ok_or_else(|| {
                        format!("XMAP21 DATA_BLOCKED: capital label state {state_id} is absent")
                    })?
                    .name_key
                    .clone(),
                gui.capital,
            ),
            _ => {
                return Err(
                    "XMAP21 DATA_BLOCKED: VP/capital label source disagrees with icon kind".into(),
                )
            }
        };
        let resolved_text = localisation
            .get(&localisation_key)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "XMAP21 DATA_BLOCKED: localisation lacks VP/capital key {localisation_key:?}"
                )
            })?;
        let resolved_text = require_flat_instant_text(&resolved_text, &localisation_key)?;
        let left = icon.root_hit_rect.left + geometry.text.offset_x as f32;
        let top = icon.root_hit_rect.top + geometry.text.offset_y as f32;
        let root_projection_sha256 = canonical_sha256(&(
            &icon.target,
            icon.root_hit_rect,
            icon.projection_depth.to_bits(),
            icon.map_icon_ordinal,
        ))?;
        operations.push(Xmap21VpCapitalTextOperation {
            stable_id: format!("{}:text", icon.target.canonical_id()),
            font_token: "hoi_20bs".into(),
            localisation_key,
            resolved_text,
            rect_px: [
                left,
                top,
                left + geometry.text.max_width as f32,
                top + geometry.text.max_height as f32,
            ],
            max_width: geometry.text.max_width,
            max_height: geometry.text.max_height,
            format: "left".into(),
            parent_map_icon_ordinal: icon.map_icon_ordinal,
            root_projection_sha256,
        });
    }
    let mut batch = Xmap21VpCapitalTextBatch {
        schema: XMAP21_VP_CAPITAL_TEXT_BATCH_SCHEMA.into(),
        update_index: selection.update_index,
        selection_epoch: selection.selection_epoch,
        map_mode_id: selection.mode_id.clone(),
        viewer_token: selection.viewer_token.clone(),
        prepare_stage_name: XMAP21_PREPARE_STAGE_NAME.into(),
        prepare_operation_name: XMAP21_PREPARE_OPERATION_NAME.into(),
        operations,
        deterministic_sha256: String::new(),
    };
    batch.deterministic_sha256 = canonical_sha256(&batch)?;
    batch.validate()?;
    Ok(batch)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectedRaidFurnitureIcon {
    pub raid_identities: Vec<RaidFurnitureIdentity>,
    pub presentation: RaidFurniturePresentation,
    pub root_screen_px: [f32; 2],
    pub actionable_hit_rect: ScreenRect,
    pub projection_depth: f32,
    pub engine_sort_key: f32,
    pub map_icon_ordinal: u32,
}

pub fn project_raid_furniture(
    world: &WorldFrame,
    selection: &MapModeFurnitureSelection,
    viewed_country: Option<&str>,
    gui: MapFurnitureGuiGeometry,
    distance_defines: MapFurnitureDistanceDefines,
    camera: &ProductionMapCamera,
    screen_width: f32,
    screen_height: f32,
) -> Result<Vec<ProjectedRaidFurnitureIcon>> {
    selection.validate_for_world(world)?;
    validate_viewport(screen_width, screen_height)?;
    let admitted = admit_raid_furniture_icons(world, viewed_country)?;
    let maximum_distance = if selection.mode_id == XMAP21_RAID_MODE_ID
        && selection.viewer_token == XMAP21_RAID_VIEWER_TOKEN
    {
        distance_defines.raid_map_mode_max
    } else {
        distance_defines.raid_max
    };
    if camera.position[1] < distance_defines.raid_min || camera.position[1] >= maximum_distance {
        return Ok(Vec::new());
    }
    let mut projected = Vec::new();
    for icon in admitted {
        if icon.presentation.variant.is_none() {
            continue;
        }
        let Some([screen_x, screen_y, depth]) =
            camera.world_to_screen_depth(icon.anchor_world, screen_width, screen_height)
        else {
            continue;
        };
        // raid_map_icon is a zero-sized centered root. The authored
        // raid_map_icon_button_container at (-15,-15), 30x30 is its exact
        // actionable child and therefore the actual hit surface.
        let root_x = screen_x.round();
        let root_y = screen_y.round();
        let actionable_hit_rect = ScreenRect {
            left: root_x + gui.raid.actionable_child.offset_x as f32,
            top: root_y + gui.raid.actionable_child.offset_y as f32,
            right: root_x
                + gui.raid.actionable_child.offset_x as f32
                + gui.raid.actionable_child.width as f32,
            bottom: root_y
                + gui.raid.actionable_child.offset_y as f32
                + gui.raid.actionable_child.height as f32,
        };
        if !actionable_hit_rect.intersects_viewport(screen_width, screen_height) {
            continue;
        }
        projected.push(ProjectedRaidFurnitureIcon {
            raid_identities: icon.raid_identities,
            presentation: icon.presentation,
            root_screen_px: [root_x, root_y],
            actionable_hit_rect,
            projection_depth: depth,
            engine_sort_key: depth * XMAP21_SORT_DEPTH_SCALE + icon.map_icon_ordinal as f32,
            map_icon_ordinal: icon.map_icon_ordinal,
        });
    }
    sort_attachment_order(&mut projected, |icon| {
        (icon.engine_sort_key, icon.map_icon_ordinal)
    });
    Ok(projected)
}

pub fn prepare_map_furniture_packet(
    world: &WorldFrame,
    resolver: &GameFileResolver,
    language: &str,
    request: &MapFurniturePrepareRequest,
    gui: MapFurnitureGuiGeometry,
    vp_defines: &VpIconDefines,
    distance_defines: MapFurnitureDistanceDefines,
    camera: &ProductionMapCamera,
    screen_width: f32,
    screen_height: f32,
) -> Result<PreparedMapFurniturePacket> {
    request.validate()?;
    request.selection.validate_for_world(world)?;
    let roster = require_complete_roster(world)?;
    validate_viewed_country(request.viewed_country_tag.as_deref(), &roster)?;
    if let (Some(tag), Some(id)) = (
        request.viewed_country_tag.as_deref(),
        request.viewed_country_tag_db_id,
    ) {
        if !roster
            .iter()
            .any(|(roster_tag, roster_id)| roster_tag == tag && *roster_id == id)
        {
            return Err(
                "XMAP21 DATA_BLOCKED: viewed-country tag/db-id does not join roster".into(),
            );
        }
    }
    let victory_points_and_capitals = project_map_furniture_icons(
        world,
        request.viewed_country_tag.as_deref(),
        request.selection.mode_id == XMAP21_RESOURCE_MODE_ID,
        gui,
        vp_defines,
        camera,
        screen_width,
        screen_height,
    )?;
    let vp_capital_text = build_vp_capital_text_batch(
        resolver,
        language,
        &request.selection,
        gui,
        &victory_points_and_capitals,
    )?;
    let (resources, resource_text) = if request.selection.mode_id == XMAP21_RESOURCE_MODE_ID {
        let (resources, text) = project_resource_furniture(
            world,
            resolver,
            language,
            &request.selection,
            request.viewed_country_tag.as_deref(),
            gui,
            distance_defines,
            camera,
            screen_width,
            screen_height,
        )?;
        (resources, Some(text))
    } else {
        (Vec::new(), None)
    };
    let raids = project_raid_furniture(
        world,
        &request.selection,
        request.viewed_country_tag.as_deref(),
        gui,
        distance_defines,
        camera,
        screen_width,
        screen_height,
    )?;
    let raid_sprite_descriptor_set_sha256 = raid_sprite_descriptor_set_sha256(resolver, &raids)?;
    let vp_strip_descriptor_sha256 = load_vp_strip_asset(resolver)?.descriptor_sha256;
    let (resource_flag_masked_shield, resource_flag_assets) = if resources
        .iter()
        .any(|resource| resource.controller_flag_texture_path.is_some())
    {
        let (shield, flags) = resolve_resource_flag_assets(resolver, &resources)?;
        (Some(shield), flags)
    } else {
        (None, Vec::new())
    };
    let stable_ids = map_furniture_stable_ids(&victory_points_and_capitals, &resources, &raids);
    let stable_ids_sha256 = canonical_sha256(&stable_ids)?;
    let payload_sha256 = map_furniture_payload_sha256(
        &victory_points_and_capitals,
        &resources,
        &raids,
        &vp_capital_text,
        resource_text.as_ref(),
        resource_flag_masked_shield.as_ref(),
        &resource_flag_assets,
        &vp_strip_descriptor_sha256,
        &raid_sprite_descriptor_set_sha256,
    )?;
    let program_sha256 = canonical_sha256(&(
        XMAP21_PREPARED_PACKET_SCHEMA,
        XMAP21_A729_EXECUTABLE_SHA256,
        request.selection.mode_id.as_str(),
        request.selection.viewer_token.as_str(),
    ))?;
    let ordered_resources_sha256 = canonical_sha256(&(
        request.shared.content_manifest_sha256.as_str(),
        stable_ids_sha256.as_str(),
        payload_sha256.as_str(),
        resource_flag_masked_shield
            .as_ref()
            .map(|descriptor| descriptor.descriptor_sha256.as_str()),
        vp_strip_descriptor_sha256.as_str(),
        raid_sprite_descriptor_set_sha256.as_str(),
    ))?;
    let prepare_operation_identity = request
        .prepare_assignment
        .foundation_operation_identity(
            program_sha256,
            ordered_resources_sha256,
            request.shared.main_scene_target_sha256.clone(),
        )
        .map_err(|error| format!("XMAP21 prepare operation identity: {error}"))?;
    let is_empty = victory_points_and_capitals.is_empty()
        && resources.is_empty()
        && raids.is_empty()
        && vp_capital_text.operations.is_empty()
        && resource_text
            .as_ref()
            .is_none_or(|batch| batch.operations.is_empty());
    let mut packet = PreparedMapFurniturePacket {
        schema: XMAP21_PREPARED_PACKET_SCHEMA.into(),
        update_index: request.selection.update_index,
        selection_epoch: request.selection.selection_epoch,
        mode_id: request.selection.mode_id.clone(),
        viewer_token: request.selection.viewer_token.clone(),
        disposition: if is_empty {
            MapFurniturePacketDisposition::AuthenticatedEmpty
        } else {
            MapFurniturePacketDisposition::Present
        },
        request_sha256: request.deterministic_sha256.clone(),
        shared: request.shared.clone(),
        prepare_assignment: request.prepare_assignment.clone(),
        execute_assignment: request.execute_assignment.clone(),
        prepare_operation_identity,
        victory_points_and_capitals,
        resources,
        raids,
        vp_capital_text,
        resource_text,
        resource_flag_masked_shield,
        resource_flag_assets,
        vp_strip_descriptor_sha256,
        raid_sprite_descriptor_set_sha256,
        stable_ids_sha256,
        payload_sha256,
        deterministic_sha256: String::new(),
    };
    packet.deterministic_sha256 = canonical_sha256(&packet)?;
    packet.validate()?;
    Ok(packet)
}

fn raid_sprite_descriptor_set_sha256(
    resolver: &GameFileResolver,
    raids: &[ProjectedRaidFurnitureIcon],
) -> Result<String> {
    let names: BTreeSet<_> = raids
        .iter()
        .flat_map(|raid| {
            [
                raid.presentation.frame_sprite_name.as_ref(),
                raid.presentation.button_sprite_name.as_ref(),
                raid.presentation.plus_sprite_name.as_ref(),
            ]
            .into_iter()
            .flatten()
            .cloned()
        })
        .collect();
    let assets = load_map_furniture_sprite_assets(resolver, &names)?;
    for raid in raids {
        for (name, frame) in [
            (
                raid.presentation.frame_sprite_name.as_deref(),
                raid.presentation.frame_index,
            ),
            (
                raid.presentation.button_sprite_name.as_deref(),
                raid.presentation.button_frame,
            ),
            (raid.presentation.plus_sprite_name.as_deref(), Some(0)),
        ] {
            let Some(name) = name else {
                continue;
            };
            let asset = assets
                .get(name)
                .ok_or_else(|| format!("XMAP21 unresolved admitted raid sprite {name:?}"))?;
            let frame = frame.ok_or_else(|| {
                format!("XMAP21 admitted raid sprite {name:?} has no selected frame")
            })?;
            if frame >= asset.no_of_frames {
                return Err(format!(
                    "XMAP21 admitted raid sprite {name:?} frame {frame} exceeds {}",
                    asset.no_of_frames
                )
                .into());
            }
        }
    }
    canonical_sha256(
        &assets
            .values()
            .map(|asset| (&asset.name, &asset.descriptor_sha256))
            .collect::<Vec<_>>(),
    )
}

fn map_furniture_stable_ids(
    victory_points_and_capitals: &[ProjectedMapFurnitureIcon],
    resources: &[ProjectedResourceFurnitureIcon],
    raids: &[ProjectedRaidFurnitureIcon],
) -> Vec<String> {
    victory_points_and_capitals
        .iter()
        .map(|icon| match &icon.target {
            MapFurnitureTarget::VictoryPoint { province_id } => {
                format!("vp:{province_id}")
            }
            MapFurnitureTarget::Capital {
                country_tag,
                capital_state_id,
            } => format!("capital:{country_tag}:{capital_state_id}"),
            MapFurnitureTarget::ResourceState { state_id } => format!("resource:{state_id}"),
            MapFurnitureTarget::Raid { raid_identities } => format!(
                "raid:{}",
                raid_identities
                    .iter()
                    .map(raid_identity_token)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        })
        .chain(
            resources
                .iter()
                .map(|icon| format!("resource:{}", icon.state_id)),
        )
        .chain(raids.iter().map(|icon| {
            format!(
                "raid:{}",
                icon.raid_identities
                    .iter()
                    .map(raid_identity_token)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }))
        .collect()
}

fn raid_identity_token(identity: &RaidFurnitureIdentity) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        identity.producer_ordinal,
        identity.status_index,
        identity.raid_index,
        identity.id,
        identity.id_type
    )
}

fn map_furniture_payload_sha256(
    victory_points_and_capitals: &[ProjectedMapFurnitureIcon],
    resources: &[ProjectedResourceFurnitureIcon],
    raids: &[ProjectedRaidFurnitureIcon],
    vp_capital_text: &Xmap21VpCapitalTextBatch,
    resource_text: Option<&Xmap21ResourceTextBatch>,
    resource_flag_masked_shield: Option<&MapFurnitureMaskedShieldDescriptor>,
    resource_flag_assets: &[MapFurnitureResourceFlagAssetDescriptor],
    vp_strip_descriptor_sha256: &str,
    raid_sprite_descriptor_set_sha256: &str,
) -> Result<String> {
    canonical_sha256(&(
        victory_points_and_capitals,
        resources,
        raids,
        vp_capital_text.deterministic_sha256.as_str(),
        resource_text.map(|batch| batch.deterministic_sha256.as_str()),
        resource_flag_masked_shield,
        resource_flag_assets,
        vp_strip_descriptor_sha256,
        raid_sprite_descriptor_set_sha256,
    ))
}

fn validate_viewport(width: f32, height: f32) -> Result<()> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("XMAP21 projection requires a positive finite viewport".into());
    }
    Ok(())
}

fn sort_attachment_order<T, F>(rows: &mut [T], key: F)
where
    F: Fn(&T) -> (f32, u32),
{
    rows.sort_by(|left, right| {
        let left = key(left);
        let right = key(right);
        left.0
            .partial_cmp(&right.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.1.cmp(&right.1))
    });
    rows.reverse();
}

fn canonical_sha256<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("serialize XMAP21 canonical payload: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_lower_sha256(value: &str, role: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("XMAP21 {role} is not a lowercase SHA-256").into());
    }
    Ok(())
}

/// Project, clip and order active icons exactly as the CMapIcon manager does.
/// Returned order is the engine's GUI-root attachment order (descending sort
/// key after the manager's ascending stable sort).
pub fn project_map_furniture_icons(
    world: &WorldFrame,
    viewed_country: Option<&str>,
    resources_mode: bool,
    gui: MapFurnitureGuiGeometry,
    defines: &VpIconDefines,
    camera: &ProductionMapCamera,
    screen_width: f32,
    screen_height: f32,
) -> Result<Vec<ProjectedMapFurnitureIcon>> {
    if !screen_width.is_finite()
        || !screen_height.is_finite()
        || screen_width <= 0.0
        || screen_height <= 0.0
    {
        return Err("XMAP21 projection requires a positive finite viewport".into());
    }
    let roster = world.d18_country_roster();
    if let Some(viewed) = viewed_country {
        if roster.status != ViewStatus::Present
            || !roster.entries.iter().any(|entry| entry.tag == viewed)
        {
            return Err("XMAP21 selected viewer country is not in the complete roster".into());
        }
    }
    let admitted = admit_map_furniture_icons(world)?;
    let mut projected = Vec::with_capacity(admitted.len());
    for icon in admitted {
        let (kind, geometry, tier) = match &icon.target {
            MapFurnitureTarget::VictoryPoint { .. } => {
                let value = icon.current_vp.expect("admission sets VP value");
                (
                    FurnitureMarkerKind::VictoryPoint,
                    gui.victory_point,
                    Some(value),
                )
            }
            MapFurnitureTarget::Capital { .. } => {
                (FurnitureMarkerKind::Capital, gui.capital, None)
            }
            MapFurnitureTarget::ResourceState { .. } | MapFurnitureTarget::Raid { .. } => {
                return Err(
                    "XMAP21 invariant violation: VP/capital projection received another icon family"
                        .into(),
                )
            }
        };
        let dot_tier = match tier {
            Some(value) => defines.tier_for_vp(value),
            None => 0,
        };
        let vp_value = tier.unwrap_or_default();
        let dot_visible = defines.dot_visible(kind, vp_value, camera.position[1]);
        let text_visible = defines.text_visible(kind, vp_value, camera.position[1], resources_mode);
        if !dot_visible && !text_visible {
            continue;
        }
        let pair = exact_relation_state(world, icon.controller_tag.as_deref(), viewed_country)?;
        let color = vp_icon_relation_column(icon.controller_tag.as_deref(), viewed_country, pair);
        let strip_frame_index = vp_strip_frame_index(kind, dot_tier, color);
        let Some([screen_x, screen_y, depth]) =
            camera.world_to_screen_depth(icon.anchor_world, screen_width, screen_height)
        else {
            continue;
        };

        // Alignment byte 0x0a has no horizontal adjustment and subtracts half
        let left = screen_x.round();
        let top = (screen_y - geometry.height as f32 * 0.5).round();
        let root_hit_rect = ScreenRect {
            left,
            top,
            right: left + geometry.width as f32,
            bottom: top + geometry.height as f32,
        };
        if !root_hit_rect.intersects_viewport(screen_width, screen_height) {
            continue;
        }
        let icon_left = left + geometry.icon.offset_x as f32;
        let icon_top = top + geometry.icon.offset_y as f32;
        projected.push(ProjectedMapFurnitureIcon {
            target: icon.target,
            capital_label_source: icon.capital_label_source,
            root_hit_rect,
            icon_rect: ScreenRect {
                left: icon_left,
                top: icon_top,
                right: icon_left + geometry.icon.width as f32,
                bottom: icon_top + geometry.icon.height as f32,
            },
            dot_visible,
            text_visible,
            strip_frame_index,
            projection_depth: depth,
            engine_sort_key: (depth - geometry.height as f32) * XMAP21_SORT_DEPTH_SCALE
                + icon.map_icon_ordinal as f32,
            map_icon_ordinal: icon.map_icon_ordinal,
        });
    }
    projected.sort_by(|left, right| {
        left.engine_sort_key
            .partial_cmp(&right.engine_sort_key)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.map_icon_ordinal.cmp(&right.map_icon_ordinal))
    });
    projected.reverse();
    Ok(projected)
}

/// Full-root hit regions in the same attachment order returned by projection.
/// Consumers that need UI stack arbitration retain every hit instead of
/// substituting the visible sprite quad for the authored container.
pub fn hit_test_map_furniture(
    attachment_order: &[ProjectedMapFurnitureIcon],
    point: [f32; 2],
) -> Vec<MapFurnitureTarget> {
    attachment_order
        .iter()
        .filter(|icon| icon.root_hit_rect.contains(point))
        .map(|icon| icon.target.clone())
        .collect()
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap21VpCapitalTextOperation {
    pub stable_id: String,
    pub font_token: String,
    pub localisation_key: String,
    pub resolved_text: String,
    pub rect_px: [f32; 4],
    pub max_width: i32,
    pub max_height: i32,
    pub format: String,
    pub parent_map_icon_ordinal: u32,
    pub root_projection_sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap21VpCapitalTextBatch {
    pub schema: String,
    pub update_index: u64,
    pub selection_epoch: u64,
    pub map_mode_id: String,
    pub viewer_token: String,
    pub prepare_stage_name: String,
    pub prepare_operation_name: String,
    pub operations: Vec<Xmap21VpCapitalTextOperation>,
    pub deterministic_sha256: String,
}

impl Xmap21VpCapitalTextBatch {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP21_VP_CAPITAL_TEXT_BATCH_SCHEMA
            || self.prepare_stage_name != XMAP21_PREPARE_STAGE_NAME
            || self.prepare_operation_name != XMAP21_PREPARE_OPERATION_NAME
        {
            return Err("XMAP21 VP/capital text batch identity is invalid".into());
        }
        let mut stable_ids = BTreeSet::new();
        for operation in &self.operations {
            if operation.font_token != "hoi_20bs"
                || operation.format != "left"
                || operation.localisation_key.is_empty()
                || operation.max_width <= 0
                || operation.max_height <= 0
                || !stable_ids.insert(operation.stable_id.as_str())
            {
                return Err("XMAP21 VP/capital text operation identity is invalid".into());
            }
            validate_lower_sha256(
                &operation.root_projection_sha256,
                "VP/capital root projection",
            )?;
            if operation
                .rect_px
                .iter()
                .any(|coordinate| !coordinate.is_finite())
                || operation.rect_px[2] - operation.rect_px[0] != operation.max_width as f32
                || operation.rect_px[3] - operation.rect_px[1] != operation.max_height as f32
            {
                return Err("XMAP21 VP/capital text rectangle is invalid".into());
            }
        }
        validate_lower_sha256(&self.deterministic_sha256, "VP/capital text batch")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != canonical_sha256(&unsigned)? {
            return Err("XMAP21 VP/capital text batch digest mismatch".into());
        }
        Ok(())
    }
}

fn validate_vp_capital_text_frontier(
    roots: &[ProjectedMapFurnitureIcon],
    batch: &Xmap21VpCapitalTextBatch,
) -> Result<()> {
    let mut operations_by_parent = BTreeMap::new();
    for operation in &batch.operations {
        if operations_by_parent
            .insert(operation.parent_map_icon_ordinal, operation)
            .is_some()
        {
            return Err("XMAP21 VP/capital root has multiple text children".into());
        }
    }
    let mut root_ordinals = BTreeSet::new();
    for root in roots {
        if !root_ordinals.insert(root.map_icon_ordinal) {
            return Err("XMAP21 VP/capital roots reuse a map-icon ordinal".into());
        }
        let operation = operations_by_parent.remove(&root.map_icon_ordinal);
        if !root.text_visible {
            if operation.is_some() {
                return Err("XMAP21 hidden VP/capital root carries a text child".into());
            }
            continue;
        }
        let operation =
            operation.ok_or("XMAP21 visible VP/capital root is missing its text child")?;
        let expected_stable_id = format!("{}:text", root.target.canonical_id());
        let expected_projection = canonical_sha256(&(
            &root.target,
            root.root_hit_rect,
            root.projection_depth.to_bits(),
            root.map_icon_ordinal,
        ))?;
        if operation.stable_id != expected_stable_id
            || operation.parent_map_icon_ordinal != root.map_icon_ordinal
            || operation.root_projection_sha256 != expected_projection
        {
            return Err("XMAP21 VP/capital text child does not match its projected root".into());
        }
    }
    if !operations_by_parent.is_empty() {
        return Err("XMAP21 VP/capital text references an absent root".into());
    }
    Ok(())
}

/// Hit-test the complete prepared frontier in the same global attachment order
/// used by the neutral map-scene executor. Full authored roots are used for
/// VP/capital/resource icons; raids use their exact authored actionable child.
pub fn hit_test_prepared_map_furniture(
    packet: &PreparedMapFurniturePacket,
    point: [f32; 2],
) -> Result<Vec<MapFurnitureTarget>> {
    packet.validate()?;
    Ok(hit_test_prepared_map_furniture_rows(
        &packet.victory_points_and_capitals,
        &packet.resources,
        &packet.raids,
        point,
    ))
}

fn hit_test_prepared_map_furniture_rows(
    victory_points_and_capitals: &[ProjectedMapFurnitureIcon],
    resources: &[ProjectedResourceFurnitureIcon],
    raids: &[ProjectedRaidFurnitureIcon],
    point: [f32; 2],
) -> Vec<MapFurnitureTarget> {
    let mut hits: Vec<(f32, u32, MapFurnitureTarget)> = Vec::new();
    for icon in victory_points_and_capitals {
        if icon.root_hit_rect.contains(point) {
            hits.push((
                icon.engine_sort_key,
                icon.map_icon_ordinal,
                icon.target.clone(),
            ));
        }
    }
    for icon in resources {
        if icon.root_hit_rect.contains(point) {
            hits.push((
                icon.engine_sort_key,
                icon.map_icon_ordinal,
                MapFurnitureTarget::ResourceState {
                    state_id: icon.state_id,
                },
            ));
        }
    }
    for icon in raids {
        if icon.actionable_hit_rect.contains(point) {
            hits.push((
                icon.engine_sort_key,
                icon.map_icon_ordinal,
                MapFurnitureTarget::Raid {
                    raid_identities: icon.raid_identities.clone(),
                },
            ));
        }
    }
    hits.sort_by(|left, right| {
        left.0
            .partial_cmp(&right.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.1.cmp(&right.1))
    });
    hits.reverse();
    hits.into_iter().map(|(_, _, target)| target).collect()
}

fn exact_relation_state(
    world: &WorldFrame,
    controller: Option<&str>,
    viewed: Option<&str>,
) -> Result<VpIconDiplomaticPairState> {
    let (Some(controller), Some(viewed)) = (controller, viewed) else {
        return Ok(VpIconDiplomaticPairState::default());
    };
    if controller == viewed {
        return Ok(VpIconDiplomaticPairState::default());
    }
    let pair = world
        .diplomatic_pair_state(controller, viewed)
        .ok_or_else(|| {
            format!("XMAP21 DATA_BLOCKED: missing diplomatic pair {controller}->{viewed}")
        })?;
    let (Leaf::Measured(at_war), Leaf::Measured(same_faction)) = (&pair.at_war, &pair.same_faction)
    else {
        return Err(format!(
            "XMAP21 DATA_BLOCKED: incomplete diplomatic pair {controller}->{viewed}"
        )
        .into());
    };
    Ok(VpIconDiplomaticPairState {
        same_faction: *same_faction,
        at_war: *at_war,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASKED_FLAG_EFFECT: &str = r#"
PixelShader = {
  Samplers = {
    BaseTexture = { Index = 0 MagFilter = "Point" MinFilter = "Linear" MipFilter = "Linear" AddressU = "Clamp" AddressV = "Clamp" }
    MaskTexture = { Index = 1 MagFilter = "Point" MinFilter = "Point" MipFilter = "None" AddressU = "Clamp" AddressV = "Clamp" }
  }
  MainCode PixelShader [[
    float4 main( VS_OUTPUT v ) : PDX_COLOR
    {
      float4 OutColor = tex2D( BaseTexture, v.vTexCoord.xy );
      float4 MaskColor = tex2D( MaskTexture, v.vTexCoord.zw );
      OutColor.a = MaskColor.a;
      return OutColor;
    }
  ]]
}
BlendState BlendState {
  BlendEnable = yes
  AlphaTest = no
  SourceBlend = "SRC_ALPHA"
  DestBlend = "INV_SRC_ALPHA"
}
Effect Up { VertexShader = "VertexShader" PixelShader = "PixelShader" }
"#;

    fn write_fixture(root: &std::path::Path, relative: &str, source: &str) {
        let path = relative
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part));
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, source).expect("write fixture");
    }

    fn write_test_dds(root: &std::path::Path, relative: &str, width: u32, height: u32) {
        let path = relative
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part));
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        let mut bytes = vec![0u8; 128 + width as usize * height as usize * 4];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&0x0002_100fu32.to_le_bytes());
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
        fs::write(path, bytes).expect("write DDS fixture");
    }

    #[test]
    fn xmap21_localisation_is_unique_selected_language_or_blocked() {
        let root = crate::test_support::temp_test_dir("xmap21_exact_localisation");
        write_fixture(
            &root,
            "localisation/english/a_l_english.yml",
            "\u{feff}l_english:\n VICTORY_POINTS_7:0 \"London\"\n",
        );
        let resolver = GameFileResolver::vanilla(&root)
            .expect("resolver")
            .bind_all_test_content(&"11".repeat(32))
            .expect("admission");
        let required = BTreeSet::from(["VICTORY_POINTS_7".to_string()]);
        let values = exact_localisation_values(&resolver, "english", &required)
            .expect("unique exact localisation");
        assert_eq!(values["VICTORY_POINTS_7"], "London");

        write_fixture(
            &root,
            "localisation/english/b_l_english.yml",
            "l_english:\n VICTORY_POINTS_7:0 \"Londinium\"\n",
        );
        let resolver = GameFileResolver::vanilla(&root)
            .expect("resolver")
            .bind_all_test_content(&"11".repeat(32))
            .expect("admission");
        assert!(exact_localisation_values(&resolver, "english", &required)
            .expect_err("duplicate effective key must block")
            .to_string()
            .contains("multiple effective definitions"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn xmap21_empty_text_still_requires_complete_nonempty_localisation_namespace() {
        let root = crate::test_support::temp_test_dir("xmap21_empty_localisation_audit");
        fs::create_dir_all(&root).expect("create fixture game root");
        let resolver = GameFileResolver::vanilla(&root)
            .expect("resolver")
            .bind_all_test_content(&"11".repeat(32))
            .expect("admission");
        assert!(
            exact_localisation_values(&resolver, "english", &BTreeSet::new())
                .expect_err("empty requested text must not bypass namespace admission")
                .to_string()
                .contains("is empty")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn masked_flag_effect_accepts_exact_alpha_and_rejects_rgb_coverage() {
        validate_masked_flag_effect(MASKED_FLAG_EFFECT).expect("exact masked-flag law");
        let drifted = MASKED_FLAG_EFFECT.replace(
            "OutColor.a = MaskColor.a;",
            "OutColor.a = max(MaskColor.r, MaskColor.a);",
        );
        assert!(validate_masked_flag_effect(&drifted)
            .expect_err("mask RGB must not become coverage")
            .to_string()
            .contains("PixelShader law"));
    }

    #[test]
    fn resource_flag_prepare_seals_effective_definition_and_every_flag() {
        let root = crate::test_support::temp_test_dir("xmap21_resource_flag_assets");
        write_fixture(
            &root,
            "interface/general_stuff.gfx",
            r#"spriteTypes = { maskedShieldType = {
                name = "GFX_flag_smallest"
                textureFile1 = "gfx/interface/mod_overlay.dds"
                textureFile2 = "gfx/interface/mod_mask.tga"
                effectFile = "gfx/FX/maskedflag.lua"
            } }"#,
        );
        write_fixture(
            &root,
            "interface/unrelated_malformed.gfx",
            "spriteTypes = { malformed = { unrelated = yes",
        );
        write_fixture(&root, "gfx/FX/maskedflag.shader", MASKED_FLAG_EFFECT);
        write_fixture(&root, "gfx/interface/mod_overlay.dds", "overlay");
        write_fixture(&root, "gfx/interface/mod_mask.tga", "mask");
        write_fixture(&root, "gfx/flags/AAA.tga", "flag");
        let resolver = GameFileResolver::vanilla(&root)
            .expect("resolver")
            .bind_all_test_content(&"11".repeat(32))
            .expect("admission");
        let resources = vec![ProjectedResourceFurnitureIcon {
            state_id: 1,
            controller_tag: Some("AAA".into()),
            controller_flag_texture_path: Some("gfx/flags/AAA.tga".into()),
            row_count: 1,
            root_hit_rect: ScreenRect {
                left: 0.0,
                top: 0.0,
                right: 100.0,
                bottom: 25.0,
            },
            projection_depth: 0.5,
            engine_sort_key: 1.0,
            map_icon_ordinal: 1,
        }];
        let (shield, flags) =
            resolve_resource_flag_assets(&resolver, &resources).expect("sealed resources");
        assert_eq!(shield.overlay_texture_path, "gfx/interface/mod_overlay.dds");
        assert_eq!(shield.mask_texture_path, "gfx/interface/mod_mask.tga");
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].logical_path, "gfx/flags/AAA.tga");
        assert_eq!(flags[0].source_encoding, "tga");
        assert_eq!(flags[0].sampling_law, XMAP21_RESOURCE_FLAG_SAMPLING_LAW);
        shield.validate().expect("valid descriptor");

        write_fixture(&root, "gfx/flags/AAA.dds", "not decoded during admission");
        let resolver = GameFileResolver::vanilla(&root)
            .expect("resolver")
            .bind_all_test_content(&"11".repeat(32))
            .expect("admission");
        let mut dds_resource = resources[0].clone();
        dds_resource.controller_flag_texture_path = Some("gfx/flags/AAA.dds".into());
        assert!(resolve_resource_flag_assets(&resolver, &[dds_resource])
            .expect_err("DDS base is outside the admitted direct-GPU source contract")
            .to_string()
            .contains("direct D3D11"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn vp_strip_uses_effective_modded_gfx_definition_and_seals_it() {
        let root = crate::test_support::temp_test_dir("xmap21_effective_vp_strip");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_fixture(
            &vanilla,
            "interface/mapicons.gfx",
            r#"spriteTypes = { spriteType = {
                name = "GFX_onmap_victorypoints_strip"
                texturefile = "gfx/interface/vanilla_strip.dds"
                noOfFrames = 20
            } }"#,
        );
        write_test_dds(&vanilla, "gfx/interface/vanilla_strip.dds", 40, 2);
        write_fixture(
            &mod_root,
            "interface/mapicons.gfx",
            r#"spriteTypes = { spriteType = {
                name = "GFX_onmap_victorypoints_strip"
                texturefile = "gfx/interface/modded_strip.dds"
                noOfFrames = 20
            } }"#,
        );
        write_test_dds(&mod_root, "gfx/interface/modded_strip.dds", 60, 3);
        let resolver = GameFileResolver::with_mod_paths(&vanilla, vec![mod_root])
            .expect("resolver")
            .bind_all_test_content(&"11".repeat(32))
            .expect("admission");
        let asset = load_vp_strip_asset(&resolver).expect("effective strip");
        assert_eq!(asset.definition_source_file, "interface/mapicons.gfx");
        assert_eq!(asset.texture_file, "gfx/interface/modded_strip.dds");
        assert_eq!((asset.image.width, asset.image.height), (60, 3));
        validate_lower_sha256(&asset.descriptor_sha256, "VP strip descriptor")
            .expect("sealed descriptor");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn vp_capital_text_frontier_rejects_missing_extra_and_wrong_root_seals() {
        let root = ProjectedMapFurnitureIcon {
            target: MapFurnitureTarget::VictoryPoint { province_id: 7 },
            capital_label_source: None,
            root_hit_rect: ScreenRect {
                left: 10.0,
                top: 20.0,
                right: 170.0,
                bottom: 50.0,
            },
            icon_rect: ScreenRect {
                left: 5.0,
                top: 20.0,
                right: 34.0,
                bottom: 47.0,
            },
            dot_visible: true,
            text_visible: true,
            strip_frame_index: 15,
            projection_depth: 0.5,
            engine_sort_key: 1.0,
            map_icon_ordinal: 3,
        };
        let projection = canonical_sha256(&(
            &root.target,
            root.root_hit_rect,
            root.projection_depth.to_bits(),
            root.map_icon_ordinal,
        ))
        .expect("projection seal");
        let operation = Xmap21VpCapitalTextOperation {
            stable_id: "xmap21.vp:7:text".into(),
            font_token: "hoi_20bs".into(),
            localisation_key: "VICTORY_POINTS_7".into(),
            resolved_text: "City".into(),
            rect_px: [36.0, 25.0, 185.0, 55.0],
            max_width: 149,
            max_height: 30,
            format: "left".into(),
            parent_map_icon_ordinal: 3,
            root_projection_sha256: projection,
        };
        let batch = Xmap21VpCapitalTextBatch {
            schema: XMAP21_VP_CAPITAL_TEXT_BATCH_SCHEMA.into(),
            update_index: 1,
            selection_epoch: 1,
            map_mode_id: "default".into(),
            viewer_token: "viewer".into(),
            prepare_stage_name: XMAP21_PREPARE_STAGE_NAME.into(),
            prepare_operation_name: XMAP21_PREPARE_OPERATION_NAME.into(),
            operations: vec![operation.clone()],
            deterministic_sha256: "0".repeat(64),
        };
        validate_vp_capital_text_frontier(std::slice::from_ref(&root), &batch)
            .expect("exact text frontier");

        let mut missing = batch.clone();
        missing.operations.clear();
        assert!(validate_vp_capital_text_frontier(std::slice::from_ref(&root), &missing).is_err());

        let mut extra = batch.clone();
        let mut extra_operation = operation.clone();
        extra_operation.parent_map_icon_ordinal = 99;
        extra_operation.stable_id = "xmap21.vp:99:text".into();
        extra.operations.push(extra_operation);
        assert!(validate_vp_capital_text_frontier(std::slice::from_ref(&root), &extra).is_err());

        let mut wrong_seal = batch;
        wrong_seal.operations[0].root_projection_sha256 = "f".repeat(64);
        assert!(
            validate_vp_capital_text_frontier(std::slice::from_ref(&root), &wrong_seal).is_err()
        );
    }

    #[test]
    fn carrier_frontiers_reject_missing_capitals_and_raids() {
        let capitals = BTreeSet::from([("AAA".to_string(), 1), ("BBB".to_string(), 2)]);
        assert!(require_complete_capital_frontier(
            &capitals,
            &BTreeSet::from([("AAA".to_string(), 1)])
        )
        .is_err());
        require_complete_capital_frontier(&capitals, &capitals).expect("complete capitals");

        let first = RaidFurnitureIdentity {
            producer_ordinal: 1,
            status_index: 2,
            raid_index: 3,
            id: 4,
            id_type: 5,
        };
        let second = RaidFurnitureIdentity {
            producer_ordinal: 6,
            status_index: 7,
            raid_index: 8,
            id: 9,
            id_type: 10,
        };
        let raids = BTreeSet::from([first.clone(), second]);
        assert!(require_complete_raid_frontier(&raids, &BTreeSet::from([first])).is_err());
        require_complete_raid_frontier(&raids, &raids).expect("complete raids");
    }

    #[test]
    fn vp_anchor_prefers_38_then_exact_10_fallback() {
        let rows = vec![
            UnitStackPlacement {
                province_id: 7,
                kind_raw: 10,
                pos: [1.0, 2.0, 3.0],
                rotation: 0.0,
                offset: 0.0,
            },
            UnitStackPlacement {
                province_id: 7,
                kind_raw: 38,
                pos: [4.0, 5.0, 6.0],
                rotation: 0.0,
                offset: 0.0,
            },
            UnitStackPlacement {
                province_id: 8,
                kind_raw: 10,
                pos: [7.0, 8.0, 9.0],
                rotation: 0.0,
                offset: 0.0,
            },
        ];
        let anchors = exact_vp_file_anchors(&rows);
        assert_eq!(anchors[&7].slot, 38);
        assert_eq!(anchors[&7].point, [4.0, 5.0, 6.0]);
        assert_eq!(anchors[&8].slot, 10);
        assert_eq!(anchors[&8].point, [7.0, 8.0, 9.0]);
    }

    #[test]
    fn hit_test_uses_full_root_not_visible_icon() {
        let icon = ProjectedMapFurnitureIcon {
            target: MapFurnitureTarget::VictoryPoint { province_id: 7 },
            capital_label_source: None,
            root_hit_rect: ScreenRect {
                left: 100.0,
                top: 100.0,
                right: 260.0,
                bottom: 130.0,
            },
            icon_rect: ScreenRect {
                left: 95.0,
                top: 100.0,
                right: 124.0,
                bottom: 127.0,
            },
            dot_visible: true,
            text_visible: true,
            strip_frame_index: 15,
            projection_depth: 0.5,
            engine_sort_key: -29_500_000.0,
            map_icon_ordinal: 1,
        };
        assert_eq!(
            hit_test_map_furniture(&[icon], [200.0, 120.0]),
            vec![MapFurnitureTarget::VictoryPoint { province_id: 7 }]
        );
    }

    #[test]
    fn prepared_hit_test_covers_all_families_in_global_attachment_order() {
        let rect = ScreenRect {
            left: 10.0,
            top: 10.0,
            right: 40.0,
            bottom: 40.0,
        };
        let vp = ProjectedMapFurnitureIcon {
            target: MapFurnitureTarget::VictoryPoint { province_id: 7 },
            capital_label_source: None,
            root_hit_rect: rect,
            icon_rect: rect,
            dot_visible: true,
            text_visible: true,
            strip_frame_index: 0,
            projection_depth: 0.1,
            engine_sort_key: 10.0,
            map_icon_ordinal: 1,
        };
        let resource = ProjectedResourceFurnitureIcon {
            state_id: 9,
            controller_tag: Some("AAA".into()),
            controller_flag_texture_path: Some("gfx/flags/AAA.tga".into()),
            row_count: 1,
            root_hit_rect: rect,
            projection_depth: 0.2,
            engine_sort_key: 20.0,
            map_icon_ordinal: 2,
        };
        let raid_identity = RaidFurnitureIdentity {
            producer_ordinal: 3,
            status_index: 4,
            raid_index: 5,
            id: 6,
            id_type: 7,
        };
        let raid = ProjectedRaidFurnitureIcon {
            raid_identities: vec![raid_identity.clone()],
            presentation: RaidFurniturePresentation {
                viewer_tag: None,
                viewer_tag_db_id: None,
                variant: Some(replay_core::RaidMapIconVariant::Active),
                frame_sprite_name: Some("frame".into()),
                frame_index: Some(0),
                button_sprite_name: Some("button".into()),
                button_frame: Some(0),
                plus_sprite_name: None,
            },
            root_screen_px: [25.0, 25.0],
            actionable_hit_rect: rect,
            projection_depth: 0.3,
            engine_sort_key: 30.0,
            map_icon_ordinal: 3,
        };
        assert_eq!(
            hit_test_prepared_map_furniture_rows(&[vp], &[resource], &[raid], [20.0, 20.0]),
            vec![
                MapFurnitureTarget::Raid {
                    raid_identities: vec![raid_identity],
                },
                MapFurnitureTarget::ResourceState { state_id: 9 },
                MapFurnitureTarget::VictoryPoint { province_id: 7 },
            ]
        );
    }
}
