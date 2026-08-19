
#[cfg(windows)]
use crate::NativeXmap05PoliticalHandoff;
#[cfg(windows)]
use crate::NativeXmap07WaterHandoff;
use crate::{GameFileResolver, PreparedTerrainFrame, Result, TerrainDependentResourceBinding};
use snapshot::sha256::Sha256;
#[cfg(windows)]
use std::collections::BTreeMap;

const XMAP04_WATER_REFRACTION_DXGI_FORMAT: u32 = 87;
const XMAP04_D3D11_MAX_TEXTURE_2D_DIMENSION: u32 = 16_384;
const XMAP04_WATER_REFRACTION_COLOR_DESCRIPTOR_DOMAIN: &[u8] =
    b"xmap04.water_refraction.color_descriptor.v1\0";
const XMAP04_WATER_REFRACTION_DEPTH_DESCRIPTOR_DOMAIN: &[u8] =
    b"xmap04.water_refraction.depth_descriptor.v1\0";
const XMAP04_WATER_REFRACTION_TEXTURE_DESCRIPTOR_DOMAIN: &[u8] =
    b"hoi4.xmap04.water-refraction.texture-descriptor.v1\0";
const XMAP04_WATER_REFRACTION_RTV_DESCRIPTOR_DOMAIN: &[u8] =
    b"hoi4.xmap04.water-refraction.rtv-descriptor.v1\0";
const XMAP04_WATER_REFRACTION_SRV_DESCRIPTOR_DOMAIN: &[u8] =
    b"hoi4.xmap04.water-refraction.srv-descriptor.v1\0";
const XMAP04_MAP_SHADER_RESOURCE_DESCRIPTOR_DOMAIN: &[u8] =
    b"hoi4.xmap04.map-shader-resource-descriptor.v1\0";
const XMAP04_MAP_SHADER_RESOURCE_SET: [(&str, u32); 4] = [
    ("TerrainIDMap", 3),
    ("HeightMap", 4),
    ("TerrainNormal", 5),
    ("HeightNormal", 6),
];
pub const XMAP04_WATER_REFRACTION_RESOURCE_IDENTITY_DOMAIN: &[u8] =
    b"hoi4.xmap04.water-refraction.resource-identity.v2\0";
pub const XMAP04_WATER_REFRACTION_PRODUCER_ALLOCATION_IDENTITY_DOMAIN: &[u8] =
    b"hoi4.xmap04.water-refraction.producer-allocation-identity.v1\0";

pub fn xmap04_underwater_color_descriptor_sha256(
    width: u32,
    height: u32,
    dxgi_format: u32,
    sample_count: u32,
    sample_quality: u32,
) -> String {
    let mut identity = XMAP04_WATER_REFRACTION_COLOR_DESCRIPTOR_DOMAIN.to_vec();
    for value in [width, height, dxgi_format, sample_count, sample_quality] {
        identity.extend_from_slice(&value.to_le_bytes());
    }
    Sha256::hex(&identity)
}

#[allow(clippy::too_many_arguments)]
pub fn xmap04_underwater_texture_descriptor_sha256(
    width: u32,
    height: u32,
    dxgi_format: u32,
    sample_count: u32,
    sample_quality: u32,
    mip_levels: u32,
    array_size: u32,
    usage: u32,
    bind_flags: u32,
    cpu_access_flags: u32,
    misc_flags: u32,
) -> String {
    let mut identity = XMAP04_WATER_REFRACTION_TEXTURE_DESCRIPTOR_DOMAIN.to_vec();
    for value in [
        width,
        height,
        dxgi_format,
        sample_count,
        sample_quality,
        mip_levels,
        array_size,
        usage,
        bind_flags,
        cpu_access_flags,
        misc_flags,
    ] {
        identity.extend_from_slice(&value.to_le_bytes());
    }
    Sha256::hex(&identity)
}

pub fn xmap04_underwater_rtv_descriptor_sha256(
    dxgi_format: u32,
    view_dimension: u32,
    mip_slice: u32,
    first_array_slice: u32,
    array_size: u32,
) -> String {
    let mut identity = XMAP04_WATER_REFRACTION_RTV_DESCRIPTOR_DOMAIN.to_vec();
    for value in [
        dxgi_format,
        view_dimension,
        mip_slice,
        first_array_slice,
        array_size,
    ] {
        identity.extend_from_slice(&value.to_le_bytes());
    }
    Sha256::hex(&identity)
}

pub fn xmap04_underwater_srv_descriptor_sha256(
    dxgi_format: u32,
    view_dimension: u32,
    most_detailed_mip: u32,
    mip_levels: u32,
    first_array_slice: u32,
    array_size: u32,
) -> String {
    let mut identity = XMAP04_WATER_REFRACTION_SRV_DESCRIPTOR_DOMAIN.to_vec();
    for value in [
        dxgi_format,
        view_dimension,
        most_detailed_mip,
        mip_levels,
        first_array_slice,
        array_size,
    ] {
        identity.extend_from_slice(&value.to_le_bytes());
    }
    Sha256::hex(&identity)
}

pub fn xmap04_underwater_producer_allocation_identity_sha256(
    xmap07_water_frame_sha256: &str,
    producer_allocation_generation: u64,
    texture_descriptor_sha256: &str,
) -> Result<String> {
    if producer_allocation_generation == 0
        || !canonical_sha256(xmap07_water_frame_sha256)
        || !canonical_sha256(texture_descriptor_sha256)
    {
        return Err(
            "XMAP04 WaterRefraction producer allocation identity inputs are invalid".into(),
        );
    }
    let mut identity = XMAP04_WATER_REFRACTION_PRODUCER_ALLOCATION_IDENTITY_DOMAIN.to_vec();
    identity.extend_from_slice(b"XMAP07\0WaterRefraction\0underwater-output\0");
    identity.extend_from_slice(xmap07_water_frame_sha256.as_bytes());
    identity.extend_from_slice(&producer_allocation_generation.to_le_bytes());
    identity.extend_from_slice(texture_descriptor_sha256.as_bytes());
    Ok(Sha256::hex(&identity))
}

pub fn xmap04_underwater_resource_identity_sha256(
    xmap07_water_frame_sha256: &str,
    producer_allocation_identity_sha256: &str,
    texture_descriptor_sha256: &str,
) -> Result<String> {
    if !canonical_sha256(xmap07_water_frame_sha256)
        || !canonical_sha256(producer_allocation_identity_sha256)
        || !canonical_sha256(texture_descriptor_sha256)
    {
        return Err(
            "XMAP04 WaterRefraction resource identity inputs are not canonical SHA-256 values"
                .into(),
        );
    }
    let mut identity = XMAP04_WATER_REFRACTION_RESOURCE_IDENTITY_DOMAIN.to_vec();
    identity.extend_from_slice(b"XMAP07\0WaterRefraction\0underwater-output\0");
    identity.extend_from_slice(xmap07_water_frame_sha256.as_bytes());
    identity.extend_from_slice(producer_allocation_identity_sha256.as_bytes());
    identity.extend_from_slice(texture_descriptor_sha256.as_bytes());
    Ok(Sha256::hex(&identity))
}

#[allow(clippy::too_many_arguments)]
pub fn xmap04_underwater_depth_descriptor_sha256(
    width: u32,
    height: u32,
    dxgi_format: u32,
    sample_count: u32,
    sample_quality: u32,
    mip_levels: u32,
    array_size: u32,
    has_stencil: bool,
) -> String {
    let mut identity = XMAP04_WATER_REFRACTION_DEPTH_DESCRIPTOR_DOMAIN.to_vec();
    for value in [
        width,
        height,
        dxgi_format,
        sample_count,
        sample_quality,
        mip_levels,
        array_size,
    ] {
        identity.extend_from_slice(&value.to_le_bytes());
    }
    identity.push(u8::from(has_stencil));
    Sha256::hex(&identity)
}

fn validate_underwater_dds_shape(width: u32, height: u32, raw_mip_levels: u32) -> Result<u32> {
    if width == 0
        || height == 0
        || width > XMAP04_D3D11_MAX_TEXTURE_2D_DIMENSION
        || height > XMAP04_D3D11_MAX_TEXTURE_2D_DIMENSION
    {
        return Err("XMAP04 underwater DDS dimensions exceed the D3D11 texture contract".into());
    }
    let mip_levels = raw_mip_levels.max(1);
    let maximum_mip_levels = u32::BITS - width.max(height).leading_zeros();
    if mip_levels > maximum_mip_levels {
        return Err(format!(
            "XMAP04 underwater DDS declares {mip_levels} mips but dimensions {width}x{height} permit at most {maximum_mip_levels}"
        )
        .into());
    }
    Ok(mip_levels)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap04D3dHandoffReceipt {
    pub terrain_frame_sha256: String,
    pub terrain_program_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub compiled_programs: u32,
    pub uploaded_shared_buffers: u32,
    pub uploaded_draw_buffers: u32,
    pub retained_static_resources: u32,
    pub uploaded_generated_resources: u32,
    pub bound_downstream_resources: Vec<TerrainDependentResourceBinding>,
    pub blocked_visible_draw_packets: Vec<String>,
    pub deterministic_sha256: String,
}

pub const XMAP04_TERRAIN_DRAW_RECEIPT_SCHEMA: &str = "hoi4.xmap04.native-terrain-draw.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap04TerrainDrawReceipt {
    pub schema: String,
    pub terrain_frame_sha256: String,
    pub terrain_program_sha256: String,
    pub ready_draw_contract_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub main_scene_resource_identity_sha256: String,
    pub main_scene_producer_frame_sha256: String,
    pub main_scene_allocation_generation: u64,
    pub main_scene_descriptor_sha256: String,
    pub ordered_scene_cb0_composition_sha256s: Vec<String>,
    pub point_lights_cb2_sha256: String,
    pub point_lights_cb2_descriptor_sha256: String,
    pub ordered_dependency_resources: Vec<TerrainDependentResourceBinding>,
    pub draw_count: u32,
    pub deterministic_sha256: String,
}

impl Xmap04TerrainDrawReceipt {
    pub fn validate(&self) -> Result<()> {
        let expected = [
            ("ProvinceSecondaryColorMap", "XMAP05"),
            ("SnowMudData", "XMAP07"),
            ("LightIndexMap", "XMAP09"),
            ("LightDataMap", "XMAP09"),
            ("ShadowMap", "XMAP10"),
            ("GradientBorderChannel1", "XMAP05"),
            ("GradientBorderChannel2", "XMAP05"),
        ];
        if self.schema != XMAP04_TERRAIN_DRAW_RECEIPT_SCHEMA
            || self.main_scene_allocation_generation == 0
            || self.draw_count == 0
            || self.ordered_scene_cb0_composition_sha256s.is_empty()
            || self.ordered_dependency_resources.len() != expected.len()
            || self.ordered_dependency_resources.iter().zip(expected).any(
                |(binding, (symbol, owner))| {
                    binding.symbol != symbol
                        || binding.owner_packet != owner
                        || !canonical_sha256(&binding.resource_sha256)
                        || !canonical_sha256(&binding.descriptor_sha256)
                },
            )
        {
            return Err("XMAP04 terrain draw receipt has an incomplete execution contract".into());
        }
        for digest in [
            &self.terrain_frame_sha256,
            &self.terrain_program_sha256,
            &self.ready_draw_contract_sha256,
            &self.map_copy_receipt_set_sha256,
            &self.main_scene_resource_identity_sha256,
            &self.main_scene_producer_frame_sha256,
            &self.main_scene_descriptor_sha256,
            &self.point_lights_cb2_sha256,
            &self.point_lights_cb2_descriptor_sha256,
            &self.deterministic_sha256,
        ]
        .into_iter()
        .chain(self.ordered_scene_cb0_composition_sha256s.iter())
        {
            if !canonical_sha256(digest) {
                return Err("XMAP04 terrain draw receipt contains an invalid SHA-256".into());
            }
        }
        if self.deterministic_sha256 != terrain_draw_receipt_digest(self) {
            return Err("XMAP04 terrain draw receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap04NativeMapShaderResourceBinding {
    pub symbol: String,
    pub owner_packet: String,
    pub shader_register: u32,
    pub replay_frame_identity_sha256: String,
    pub camera_frame_sha256: String,
    pub source_content_sha256: String,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
}

impl Xmap04NativeMapShaderResourceBinding {
    pub fn validate(&self) -> Result<()> {
        if self.owner_packet != "XMAP04"
            || !XMAP04_MAP_SHADER_RESOURCE_SET
                .contains(&(self.symbol.as_str(), self.shader_register))
            || !canonical_sha256(&self.replay_frame_identity_sha256)
            || !canonical_sha256(&self.camera_frame_sha256)
            || !canonical_sha256(&self.source_content_sha256)
            || !canonical_sha256(&self.payload_sha256)
            || !canonical_sha256(&self.descriptor_sha256)
        {
            return Err("XMAP04 native map shader resource binding is invalid".into());
        }
        Ok(())
    }
}

fn admitted_heightmap_content(
    frame: &PreparedTerrainFrame,
) -> Result<&portable_content_manifest::UsedContentKey> {
    frame
        .ordered_used_content
        .iter()
        .find(|content| content.normalized_game_relative_path == "map/heightmap.bmp")
        .ok_or_else(|| "XMAP04 prepared frame omitted admitted map/heightmap.bmp".into())
}

fn map_shader_resource_descriptor_sha256(
    symbol: &str,
    width: u32,
    height: u32,
    mip_levels: u32,
    dxgi_format: u32,
    usage: u32,
    subresource_pitches: &[(u32, u32)],
) -> Result<String> {
    if !XMAP04_MAP_SHADER_RESOURCE_SET
        .iter()
        .any(|(expected, _)| *expected == symbol)
        || width == 0
        || height == 0
        || width > XMAP04_D3D11_MAX_TEXTURE_2D_DIMENSION
        || height > XMAP04_D3D11_MAX_TEXTURE_2D_DIMENSION
        || mip_levels == 0
        || subresource_pitches.len() != usize::try_from(mip_levels)?
        || subresource_pitches
            .iter()
            .any(|(row_pitch, slice_pitch)| *row_pitch == 0 || *slice_pitch == 0)
    {
        return Err("XMAP04 map shader texture descriptor is invalid".into());
    }
    let mut identity = XMAP04_MAP_SHADER_RESOURCE_DESCRIPTOR_DOMAIN.to_vec();
    identity.extend_from_slice(symbol.as_bytes());
    identity.push(0);
    for value in [
        width,
        height,
        mip_levels,
        1,
        dxgi_format,
        1,
        0,
        usage,
        8,
        0,
        0,
    ] {
        identity.extend_from_slice(&value.to_le_bytes());
    }
    for (row_pitch, slice_pitch) in subresource_pitches {
        identity.extend_from_slice(&row_pitch.to_le_bytes());
        identity.extend_from_slice(&slice_pitch.to_le_bytes());
    }
    Ok(Sha256::hex(&identity))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap04UnderwaterDrawReceipt {
    pub terrain_frame_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub camera_frame_sha256: String,
    pub xmap09_camera_frame_sha256: String,
    pub underwater_program_sha256: String,
    pub target_resource_identity_sha256: String,
    pub target_producer_frame_sha256: String,
    pub target_producer_allocation_generation: u64,
    pub target_producer_allocation_identity_sha256: String,
    pub target_texture_descriptor_sha256: String,
    pub target_rtv_descriptor_sha256: String,
    pub target_srv_descriptor_sha256: String,
    pub target_descriptor_sha256: String,
    pub depth_descriptor_sha256: String,
    pub scene_cb0_sha256: String,
    pub scene_cb0_descriptor_sha256: String,
    pub point_lights_cb2_sha256: String,
    pub point_lights_cb2_descriptor_sha256: String,
    pub ordered_light_resources: Vec<TerrainDependentResourceBinding>,
    pub draw_count: u32,
    pub deterministic_sha256: String,
}

impl Xmap04UnderwaterDrawReceipt {
    pub fn validate(&self) -> Result<()> {
        for (digest, label) in [
            (&self.terrain_frame_sha256, "terrain frame"),
            (&self.replay_frame_identity_sha256, "replay frame"),
            (&self.camera_frame_sha256, "camera frame"),
            (&self.xmap09_camera_frame_sha256, "XMAP09 camera frame"),
            (&self.underwater_program_sha256, "underwater program"),
            (&self.target_resource_identity_sha256, "target resource"),
            (&self.target_producer_frame_sha256, "target producer frame"),
            (
                &self.target_producer_allocation_identity_sha256,
                "target producer allocation",
            ),
            (
                &self.target_texture_descriptor_sha256,
                "target texture descriptor",
            ),
            (&self.target_rtv_descriptor_sha256, "target RTV descriptor"),
            (&self.target_srv_descriptor_sha256, "target SRV descriptor"),
            (&self.target_descriptor_sha256, "target descriptor"),
            (&self.depth_descriptor_sha256, "depth descriptor"),
            (&self.scene_cb0_sha256, "SceneCB0 payload"),
            (&self.scene_cb0_descriptor_sha256, "SceneCB0 descriptor"),
            (&self.point_lights_cb2_sha256, "PointLightsCB2 payload"),
            (
                &self.point_lights_cb2_descriptor_sha256,
                "PointLightsCB2 descriptor",
            ),
            (&self.deterministic_sha256, "receipt"),
        ] {
            if !canonical_sha256(digest) {
                return Err(format!("XMAP04 underwater draw {label} identity is invalid").into());
            }
        }
        let expected_resources = [
            ("LightIndexMap", "XMAP09"),
            ("LightDataMap", "XMAP09"),
            ("ShadowMap", "XMAP10"),
        ];
        if self.target_producer_allocation_generation == 0
            || self.draw_count == 0
            || self.camera_frame_sha256 != self.xmap09_camera_frame_sha256
            || self.ordered_light_resources.len() != expected_resources.len()
            || self
                .ordered_light_resources
                .iter()
                .zip(expected_resources)
                .any(|(binding, (symbol, owner))| {
                    binding.symbol != symbol
                        || binding.owner_packet != owner
                        || !canonical_sha256(&binding.resource_sha256)
                        || !canonical_sha256(&binding.descriptor_sha256)
                })
        {
            return Err(
                "XMAP04 underwater draw receipt has an incomplete execution/resource contract"
                    .into(),
            );
        }
        if self.target_producer_allocation_identity_sha256
            != xmap04_underwater_producer_allocation_identity_sha256(
                &self.target_producer_frame_sha256,
                self.target_producer_allocation_generation,
                &self.target_texture_descriptor_sha256,
            )?
            || self.target_resource_identity_sha256
                != xmap04_underwater_resource_identity_sha256(
                    &self.target_producer_frame_sha256,
                    &self.target_producer_allocation_identity_sha256,
                    &self.target_texture_descriptor_sha256,
                )?
        {
            return Err(
                "XMAP04 underwater draw receipt target allocation/resource identity mismatch"
                    .into(),
            );
        }
        if self.deterministic_sha256 != underwater_draw_receipt_digest(self) {
            return Err("XMAP04 underwater draw receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap04UnderwaterTargetContract {
    pub owner_packet: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub descriptor_sha256: String,
    pub producer_frame_sha256: String,
    pub producer_allocation_generation: u64,
    pub producer_allocation_identity_sha256: String,
    pub resource_identity_sha256: String,
    pub texture_mip_levels: u32,
    pub texture_array_size: u32,
    pub texture_usage: u32,
    pub texture_bind_flags: u32,
    pub texture_cpu_access_flags: u32,
    pub texture_misc_flags: u32,
    pub texture_descriptor_sha256: String,
    pub rtv_dxgi_format: u32,
    pub rtv_view_dimension: u32,
    pub rtv_mip_slice: u32,
    pub rtv_first_array_slice: u32,
    pub rtv_array_size: u32,
    pub rtv_descriptor_sha256: String,
    pub srv_dxgi_format: u32,
    pub srv_view_dimension: u32,
    pub srv_most_detailed_mip: u32,
    pub srv_mip_levels: u32,
    pub srv_first_array_slice: u32,
    pub srv_array_size: u32,
    pub srv_descriptor_sha256: String,
    pub depth_dxgi_format: u32,
    pub depth_sample_count: u32,
    pub depth_sample_quality: u32,
    pub depth_mip_levels: u32,
    pub depth_array_size: u32,
    pub depth_has_stencil: bool,
    pub depth_descriptor_sha256: String,
}

impl Xmap04UnderwaterTargetContract {
    pub fn validate(&self) -> Result<()> {
        let exact_texture_hash = xmap04_underwater_texture_descriptor_sha256(
            self.width,
            self.height,
            self.dxgi_format,
            self.sample_count,
            self.sample_quality,
            self.texture_mip_levels,
            self.texture_array_size,
            self.texture_usage,
            self.texture_bind_flags,
            self.texture_cpu_access_flags,
            self.texture_misc_flags,
        );
        let exact_producer_allocation_hash = xmap04_underwater_producer_allocation_identity_sha256(
            &self.producer_frame_sha256,
            self.producer_allocation_generation,
            &exact_texture_hash,
        )?;
        if self.owner_packet != "XMAP07"
            || self.width == 0
            || self.height == 0
            || self.dxgi_format != XMAP04_WATER_REFRACTION_DXGI_FORMAT
            || self.sample_count != 1
            || self.sample_quality != 0
            || self.descriptor_sha256
                != xmap04_underwater_color_descriptor_sha256(
                    self.width,
                    self.height,
                    self.dxgi_format,
                    self.sample_count,
                    self.sample_quality,
                )
            || self.texture_mip_levels != 1
            || self.texture_array_size != 1
            || self.texture_usage != 0
            || self.texture_bind_flags != 0x28
            || self.texture_cpu_access_flags != 0
            || self.texture_misc_flags != 0
            || self.texture_descriptor_sha256 != exact_texture_hash
            || self.producer_allocation_identity_sha256 != exact_producer_allocation_hash
            || self.resource_identity_sha256
                != xmap04_underwater_resource_identity_sha256(
                    &self.producer_frame_sha256,
                    &self.producer_allocation_identity_sha256,
                    &self.texture_descriptor_sha256,
                )?
            || self.rtv_dxgi_format != self.dxgi_format
            || self.rtv_view_dimension != 4
            || self.rtv_mip_slice != 0
            || self.rtv_first_array_slice != 0
            || self.rtv_array_size != 1
            || self.rtv_descriptor_sha256
                != xmap04_underwater_rtv_descriptor_sha256(
                    self.rtv_dxgi_format,
                    self.rtv_view_dimension,
                    self.rtv_mip_slice,
                    self.rtv_first_array_slice,
                    self.rtv_array_size,
                )
            || self.srv_dxgi_format != self.dxgi_format
            || self.srv_view_dimension != 4
            || self.srv_most_detailed_mip != 0
            || self.srv_mip_levels != 1
            || self.srv_first_array_slice != 0
            || self.srv_array_size != 1
            || self.srv_descriptor_sha256
                != xmap04_underwater_srv_descriptor_sha256(
                    self.srv_dxgi_format,
                    self.srv_view_dimension,
                    self.srv_most_detailed_mip,
                    self.srv_mip_levels,
                    self.srv_first_array_slice,
                    self.srv_array_size,
                )
            || self.depth_sample_count != self.sample_count
            || self.depth_sample_quality != self.sample_quality
            || self.depth_mip_levels != 1
            || self.depth_array_size != 1
            || self.depth_descriptor_sha256
                != xmap04_underwater_depth_descriptor_sha256(
                    self.width,
                    self.height,
                    self.depth_dxgi_format,
                    self.depth_sample_count,
                    self.depth_sample_quality,
                    self.depth_mip_levels,
                    self.depth_array_size,
                    self.depth_has_stencil,
                )
        {
            return Err("XMAP04 WaterRefraction target contract is not exact".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
pub unsafe trait NativeXmap04UnderwaterTarget {
    fn target_contract(&self) -> &Xmap04UnderwaterTargetContract;
    fn owner_packet(&self) -> &str {
        &self.target_contract().owner_packet
    }
    fn width(&self) -> u32 {
        self.target_contract().width
    }
    fn height(&self) -> u32 {
        self.target_contract().height
    }
    fn dxgi_format(&self) -> u32 {
        self.target_contract().dxgi_format
    }
    fn sample_count(&self) -> u32 {
        self.target_contract().sample_count
    }
    fn sample_quality(&self) -> u32 {
        self.target_contract().sample_quality
    }
    fn descriptor_sha256(&self) -> &str {
        &self.target_contract().descriptor_sha256
    }
    fn resource_identity_sha256(&self) -> &str {
        &self.target_contract().resource_identity_sha256
    }
    fn producer_allocation_generation(&self) -> u64 {
        self.target_contract().producer_allocation_generation
    }
    fn producer_allocation_identity_sha256(&self) -> &str {
        &self.target_contract().producer_allocation_identity_sha256
    }
    fn texture_mip_levels(&self) -> u32 {
        self.target_contract().texture_mip_levels
    }
    fn texture_array_size(&self) -> u32 {
        self.target_contract().texture_array_size
    }
    fn texture_usage(&self) -> u32 {
        self.target_contract().texture_usage
    }
    fn texture_bind_flags(&self) -> u32 {
        self.target_contract().texture_bind_flags
    }
    fn texture_cpu_access_flags(&self) -> u32 {
        self.target_contract().texture_cpu_access_flags
    }
    fn texture_misc_flags(&self) -> u32 {
        self.target_contract().texture_misc_flags
    }
    fn texture_descriptor_sha256(&self) -> &str {
        &self.target_contract().texture_descriptor_sha256
    }
    fn rtv_dxgi_format(&self) -> u32 {
        self.target_contract().rtv_dxgi_format
    }
    fn rtv_view_dimension(&self) -> u32 {
        self.target_contract().rtv_view_dimension
    }
    fn rtv_mip_slice(&self) -> u32 {
        self.target_contract().rtv_mip_slice
    }
    fn rtv_first_array_slice(&self) -> u32 {
        self.target_contract().rtv_first_array_slice
    }
    fn rtv_array_size(&self) -> u32 {
        self.target_contract().rtv_array_size
    }
    fn rtv_descriptor_sha256(&self) -> &str {
        &self.target_contract().rtv_descriptor_sha256
    }
    fn srv_dxgi_format(&self) -> u32 {
        self.target_contract().srv_dxgi_format
    }
    fn srv_view_dimension(&self) -> u32 {
        self.target_contract().srv_view_dimension
    }
    fn srv_most_detailed_mip(&self) -> u32 {
        self.target_contract().srv_most_detailed_mip
    }
    fn srv_mip_levels(&self) -> u32 {
        self.target_contract().srv_mip_levels
    }
    fn srv_first_array_slice(&self) -> u32 {
        self.target_contract().srv_first_array_slice
    }
    fn srv_array_size(&self) -> u32 {
        self.target_contract().srv_array_size
    }
    fn srv_descriptor_sha256(&self) -> &str {
        &self.target_contract().srv_descriptor_sha256
    }
    fn depth_dxgi_format(&self) -> u32 {
        self.target_contract().depth_dxgi_format
    }
    fn depth_sample_count(&self) -> u32 {
        self.target_contract().depth_sample_count
    }
    fn depth_sample_quality(&self) -> u32 {
        self.target_contract().depth_sample_quality
    }
    fn depth_mip_levels(&self) -> u32 {
        self.target_contract().depth_mip_levels
    }
    fn depth_array_size(&self) -> u32 {
        self.target_contract().depth_array_size
    }
    fn depth_has_stencil(&self) -> bool {
        self.target_contract().depth_has_stencil
    }
    fn depth_descriptor_sha256(&self) -> &str {
        &self.target_contract().depth_descriptor_sha256
    }
    fn raw_render_target_view(&self) -> *mut winapi::um::d3d11::ID3D11RenderTargetView;
    fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView;
}

#[cfg(windows)]
pub unsafe trait NativeXmap04UnderwaterSceneConstantBuffer {
    fn owner_packet(&self) -> &str;
    fn symbol(&self) -> &str;
    fn shader_register(&self) -> u32;
    fn byte_len(&self) -> u32;
    fn viewport_width(&self) -> u32;
    fn viewport_height(&self) -> u32;
    fn frame_identity_sha256(&self) -> &str;
    fn replay_frame_identity_sha256(&self) -> &str;
    fn payload_sha256(&self) -> &str;
    fn descriptor_sha256(&self) -> &str;
    fn raw_buffer(&self) -> *mut winapi::um::d3d11::ID3D11Buffer;
}

#[cfg(windows)]
pub unsafe trait NativeXmap04UnderwaterPointLightsConstantBuffer {
    fn owner_packet(&self) -> &str;
    fn symbol(&self) -> &str;
    fn shader_register(&self) -> u32;
    fn byte_len(&self) -> u32;
    fn frame_identity_sha256(&self) -> &str;
    fn replay_frame_identity_sha256(&self) -> &str;
    fn payload_sha256(&self) -> &str;
    fn descriptor_sha256(&self) -> &str;
    fn raw_buffer(&self) -> *mut winapi::um::d3d11::ID3D11Buffer;
}

#[cfg(windows)]
pub unsafe trait NativeXmap04UnderwaterShaderResource {
    fn owner_packet(&self) -> &str;
    fn symbol(&self) -> &str;
    fn shader_register(&self) -> u32;
    fn frame_identity_sha256(&self) -> &str;
    fn replay_frame_identity_sha256(&self) -> &str;
    fn resource_sha256(&self) -> &str;
    fn descriptor_sha256(&self) -> &str;
    fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView;
}

#[cfg(windows)]
struct NativeXmap04UnderwaterProgram {
    deterministic_sha256: String,
    vertex_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    pixel_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    input_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    static_textures: BTreeMap<String, NativeXmap04UnderwaterTexture>,
    samplers: Vec<(
        u32,
        crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>,
    )>,
    blend_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    depth_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    rasterizer_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
}

#[cfg(windows)]
pub(crate) struct NativeXmap04UnderwaterTexture {
    pub(crate) _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    pub(crate) view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    resource_sha256: String,
    descriptor_sha256: String,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct NativeXmap04ShaderResourceView<'a> {
    symbol: String,
    shader_register: u32,
    replay_frame_identity_sha256: String,
    resource_sha256: String,
    descriptor_sha256: String,
    raw_srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    _owner: std::marker::PhantomData<&'a NativeXmap04TerrainHandoff>,
}

#[cfg(windows)]
impl NativeXmap04ShaderResourceView<'_> {
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn shader_register(&self) -> u32 {
        self.shader_register
    }

    pub fn replay_frame_identity_sha256(&self) -> &str {
        &self.replay_frame_identity_sha256
    }

    pub fn resource_sha256(&self) -> &str {
        &self.resource_sha256
    }

    pub fn descriptor_sha256(&self) -> &str {
        &self.descriptor_sha256
    }

    pub fn as_raw(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.raw_srv
    }
}

#[cfg(windows)]
struct NativeXmap04UnderwaterDepth {
    width: u32,
    height: u32,
    descriptor_sha256: String,
    has_stencil: bool,
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilView>,
}

#[cfg(windows)]
struct NativeXmap04MapShaderTexture {
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    binding: Xmap04NativeMapShaderResourceBinding,
}

/// Borrowed XMAP04 map-resource view. The lifetime prevents the XMAP04
#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct NativeXmap04MapShaderResourceView<'a> {
    pub binding: Xmap04NativeMapShaderResourceBinding,
    raw_srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    raw_device: *mut winapi::um::d3d11::ID3D11Device,
    _owner: std::marker::PhantomData<&'a NativeXmap04TerrainHandoff>,
}

#[cfg(windows)]
impl NativeXmap04MapShaderResourceView<'_> {
    pub fn as_raw(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.raw_srv
    }

    pub fn device(&self) -> *mut winapi::um::d3d11::ID3D11Device {
        self.raw_device
    }

    pub fn validate_for_frame(
        &self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        replay_frame_identity_sha256: &str,
        camera_frame_sha256: &str,
    ) -> Result<()> {
        self.binding.validate()?;
        if device.is_null()
            || self.raw_device != device
            || self.raw_srv.is_null()
            || self.binding.replay_frame_identity_sha256 != replay_frame_identity_sha256
            || self.binding.camera_frame_sha256 != camera_frame_sha256
        {
            return Err(
                "XMAP04 map shader resource belongs to another device or replay/camera frame"
                    .into(),
            );
        }
        Ok(())
    }
}

#[cfg(windows)]
pub struct NativeXmap04TerrainHandoff {
    device: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Device>,
    vertex_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    pixel_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    input_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    shared_slot0: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    shared_index: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    terrain_id_map_texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    terrain_id_map_view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    terrain_static_textures: BTreeMap<String, NativeXmap04UnderwaterTexture>,
    terrain_samplers: Vec<(
        u32,
        crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>,
    )>,
    terrain_blend_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    terrain_depth_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    terrain_rasterizer_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
    map_shader_textures: BTreeMap<String, NativeXmap04MapShaderTexture>,
    draw_uploads: Vec<NativeXmap04DrawUpload>,
    static_resources: Vec<(String, std::sync::Arc<[u8]>)>,
    downstream_pixel_slots: BTreeMap<String, u32>,
    underwater: Option<NativeXmap04UnderwaterProgram>,
    underwater_depth: Option<NativeXmap04UnderwaterDepth>,
    receipt: Xmap04D3dHandoffReceipt,
}

#[cfg(windows)]
struct NativeXmap04DrawUpload {
    slot1: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    cb1: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
}

#[cfg(windows)]
impl NativeXmap04TerrainHandoff {
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        frame: &PreparedTerrainFrame,
    ) -> Result<Self> {
        use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
        use std::ffi::CString;
        use std::ptr;
        use winapi::um::d3d11::{
            ID3D11InputLayout, ID3D11PixelShader, ID3D11VertexShader, D3D11_INPUT_ELEMENT_DESC,
        };
        use winapi::um::unknwnbase::IUnknown;

        frame.validate()?;
        if device.is_null() {
            return Err("XMAP04 app-device handoff received a null D3D11 device".into());
        }
        (*(device as *mut IUnknown)).AddRef();
        let retained_device = D3dPtr::from_raw(device, "XMAP04 retained D3D11 device")?;
        let mut vertex = ptr::null_mut::<ID3D11VertexShader>();
        native_d3d11_hr(
            (*device).CreateVertexShader(
                frame.program.vertex_bytecode.as_ptr() as *const _,
                frame.program.vertex_bytecode.len(),
                ptr::null_mut(),
                &mut vertex,
            ),
            "CreateVertexShader(XMAP04 admitted terrain)",
        )?;
        let vertex_shader = D3dPtr::from_raw(vertex, "XMAP04 terrain vertex shader")?;
        let mut pixel = ptr::null_mut::<ID3D11PixelShader>();
        native_d3d11_hr(
            (*device).CreatePixelShader(
                frame.program.pixel_bytecode.as_ptr() as *const _,
                frame.program.pixel_bytecode.len(),
                ptr::null_mut(),
                &mut pixel,
            ),
            "CreatePixelShader(XMAP04 admitted terrain)",
        )?;
        let pixel_shader = D3dPtr::from_raw(pixel, "XMAP04 terrain pixel shader")?;

        let position = CString::new("POSITION")?;
        let texcoord = CString::new("TEXCOORD")?;
        let per_vertex = winapi::um::d3d11::D3D11_INPUT_PER_VERTEX_DATA;
        let elements = [
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: position.as_ptr(),
                SemanticIndex: 0,
                Format: crate::TERRAIN_CHUNK_SLOT0_POSITION_FORMAT as _,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: per_vertex,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: texcoord.as_ptr(),
                SemanticIndex: 0,
                Format: crate::TERRAIN_CHUNK_SLOT1_TEXCOORD_FORMAT as _,
                InputSlot: 1,
                AlignedByteOffset: 0,
                InputSlotClass: per_vertex,
                InstanceDataStepRate: 0,
            },
        ];
        let mut input = ptr::null_mut::<ID3D11InputLayout>();
        native_d3d11_hr(
            (*device).CreateInputLayout(
                elements.as_ptr(),
                elements.len() as u32,
                frame.program.vertex_bytecode.as_ptr() as *const _,
                frame.program.vertex_bytecode.len(),
                &mut input,
            ),
            "CreateInputLayout(XMAP04 terrain)",
        )?;
        let input_layout = D3dPtr::from_raw(input, "XMAP04 terrain input layout")?;
        let shared_slot0 = create_buffer(
            device,
            &frame.shared_slot0,
            winapi::um::d3d11::D3D11_BIND_VERTEX_BUFFER,
            "XMAP04 shared slot0",
        )?;
        let shared_index = create_buffer(
            device,
            &frame.shared_index,
            winapi::um::d3d11::D3D11_BIND_INDEX_BUFFER,
            "XMAP04 shared R16 index",
        )?;
        let draw_uploads = upload_draws(device, frame)?;
        let mut static_resources = Vec::new();
        let mut generated_terrain_id = None;
        for sampler in &frame.program.samplers {
            if let Some(content) = &sampler.admitted_static_content {
                let bytes: std::sync::Arc<[u8]> =
                    resolver.read_admitted_used_content(content)?.into();
                static_resources.push((sampler.name.clone(), bytes));
            } else if let (Some(payload), Some(descriptor)) = (
                &sampler.generated_static_payload,
                &sampler.generated_static_descriptor,
            ) {
                static_resources.push((sampler.name.clone(), std::sync::Arc::clone(payload)));
                if sampler.name == "TerrainIDMap" {
                    generated_terrain_id = Some((payload.as_ref(), descriptor));
                }
            }
        }
        let generated_terrain_id = generated_terrain_id
            .ok_or("XMAP04 terrain program omitted generated TerrainIDMap payload")?;
        let (terrain_id_map_texture, terrain_id_map_view) =
            create_generated_texture(device, generated_terrain_id)?;
        let map_shader_textures =
            create_map_shader_textures(device, resolver, frame, generated_terrain_id)?;
        let mut terrain_static_textures = BTreeMap::new();
        let mut terrain_samplers = Vec::with_capacity(frame.program.samplers.len());
        for sampler in &frame.program.samplers {
            terrain_samplers.push((
                sampler.shader_register,
                create_underwater_sampler(device, sampler)?,
            ));
            if sampler.owner_packet == "XMAP04" && sampler.admitted_static_content.is_some() {
                let texture = create_terrain_static_texture(device, resolver, sampler)?;
                if terrain_static_textures
                    .insert(sampler.name.clone(), texture)
                    .is_some()
                {
                    return Err("XMAP04 terrain static texture is duplicated".into());
                }
            }
        }
        let downstream_pixel_slots = frame
            .program
            .samplers
            .iter()
            .filter(|sampler| sampler.owner_packet != "XMAP04")
            .map(|sampler| (sampler.name.clone(), sampler.shader_register))
            .collect();
        let receipt = handoff_receipt(frame, draw_uploads.len(), static_resources.len())?;
        let underwater = frame
            .underwater_program
            .as_ref()
            .map(|program| create_underwater_program(device, resolver, program))
            .transpose()?;
        Ok(Self {
            device: retained_device,
            vertex_shader,
            pixel_shader,
            input_layout,
            shared_slot0,
            shared_index,
            terrain_id_map_texture,
            terrain_id_map_view,
            terrain_static_textures,
            terrain_samplers,
            terrain_blend_state: create_underwater_blend_state(device)?,
            terrain_depth_state: create_terrain_depth_state(device)?,
            terrain_rasterizer_state: create_terrain_rasterizer_state(device)?,
            map_shader_textures,
            draw_uploads,
            static_resources,
            downstream_pixel_slots,
            underwater,
            underwater_depth: None,
            receipt,
        })
    }

    pub unsafe fn update_frame(
        &mut self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        frame: &PreparedTerrainFrame,
    ) -> Result<&Xmap04D3dHandoffReceipt> {
        frame.validate()?;
        if frame.program.deterministic_sha256 != self.receipt.terrain_program_sha256 {
            return Err(
                "XMAP04 terrain program changed; construct a fresh app-device handoff".into(),
            );
        }
        if frame
            .underwater_program
            .as_ref()
            .map(|program| program.deterministic_sha256.as_str())
            != self
                .underwater
                .as_ref()
                .map(|program| program.deterministic_sha256.as_str())
        {
            return Err(
                "XMAP04 underwater program changed; construct a fresh app-device handoff".into(),
            );
        }
        let heightmap_source_sha256 = admitted_heightmap_content(frame)?.payload_sha256.as_str();
        if self
            .map_shader_textures
            .get("HeightMap")
            .map(|texture| texture.binding.source_content_sha256.as_str())
            != Some(heightmap_source_sha256)
        {
            return Err(
                "XMAP04 admitted HeightMap changed; construct a fresh app-device handoff".into(),
            );
        }
        if frame.deterministic_sha256 == self.receipt.terrain_frame_sha256 {
            return Ok(&self.receipt);
        }
        let _ = (
            self.vertex_shader.as_raw(),
            self.pixel_shader.as_raw(),
            self.input_layout.as_raw(),
            self.shared_slot0.as_raw(),
            self.shared_index.as_raw(),
            self.device.as_raw(),
            self.map_shader_textures.len(),
            self.static_resources.len(),
        );
        self.draw_uploads = upload_draws(device, frame)?;
        self.receipt =
            handoff_receipt(frame, self.draw_uploads.len(), self.static_resources.len())?;
        for texture in self.map_shader_textures.values_mut() {
            texture.binding.replay_frame_identity_sha256 =
                frame.replay_frame_identity_sha256.clone();
            texture.binding.camera_frame_sha256 = frame.camera_frame_sha256.clone();
            texture.binding.validate()?;
        }
        Ok(&self.receipt)
    }

    pub fn receipt(&self) -> &Xmap04D3dHandoffReceipt {
        &self.receipt
    }

    /// map effect. Identity and lifetime remain tied to this handoff.
    pub fn underwater_static_resource<'a>(
        &'a self,
        frame: &PreparedTerrainFrame,
        symbol: &str,
        shader_register: u32,
    ) -> Result<NativeXmap04ShaderResourceView<'a>> {
        frame.validate()?;
        if frame.deterministic_sha256 != self.receipt.terrain_frame_sha256 {
            return Err(
                "XMAP04 shared underwater resource belongs to a stale terrain frame".into(),
            );
        }
        let texture = self
            .underwater
            .as_ref()
            .ok_or("XMAP04 shared underwater resources require the high-GFX program")?
            .static_textures
            .get(symbol)
            .ok_or_else(|| format!("XMAP04 owns no underwater static resource {symbol}"))?;
        Ok(NativeXmap04ShaderResourceView {
            symbol: symbol.into(),
            shader_register,
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            resource_sha256: texture.resource_sha256.clone(),
            descriptor_sha256: texture.descriptor_sha256.clone(),
            raw_srv: texture.view.as_raw(),
            _owner: std::marker::PhantomData,
        })
    }

    pub fn terrain_static_resource<'a>(
        &'a self,
        frame: &PreparedTerrainFrame,
        symbol: &str,
        shader_register: u32,
    ) -> Result<NativeXmap04ShaderResourceView<'a>> {
        frame.validate()?;
        if frame.deterministic_sha256 != self.receipt.terrain_frame_sha256 {
            return Err("XMAP04 shared static resource belongs to a stale terrain frame".into());
        }
        let texture = self
            .terrain_static_textures
            .get(symbol)
            .ok_or_else(|| format!("XMAP04 visible terrain owns no static resource {symbol}"))?;
        Ok(NativeXmap04ShaderResourceView {
            symbol: symbol.into(),
            shader_register,
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            resource_sha256: texture.resource_sha256.clone(),
            descriptor_sha256: texture.descriptor_sha256.clone(),
            raw_srv: texture.view.as_raw(),
            _owner: std::marker::PhantomData,
        })
    }

    pub(crate) fn xmap10_shared_slot0(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
        self.shared_slot0.as_raw()
    }

    pub(crate) fn xmap10_shared_index(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
        self.shared_index.as_raw()
    }

    pub(crate) fn xmap10_draw_slot1(
        &self,
        draw_index: usize,
    ) -> Result<*mut winapi::um::d3d11::ID3D11Buffer> {
        let draw = self
            .draw_uploads
            .get(draw_index)
            .ok_or("XMAP10 requested an XMAP04 terrain draw outside the uploaded census")?;
        Ok(draw.slot1.as_raw())
    }

    pub fn map_shader_resource_views(&self) -> Result<Vec<NativeXmap04MapShaderResourceView<'_>>> {
        if self.map_shader_textures.len() != XMAP04_MAP_SHADER_RESOURCE_SET.len() {
            return Err("XMAP04 map shader resource set is incomplete".into());
        }
        XMAP04_MAP_SHADER_RESOURCE_SET
            .iter()
            .map(|(symbol, shader_register)| {
                let texture = self
                    .map_shader_textures
                    .get(*symbol)
                    .ok_or_else(|| format!("XMAP04 map shader resource {symbol} is absent"))?;
                texture.binding.validate()?;
                if texture.binding.shader_register != *shader_register
                    || texture.view.as_raw().is_null()
                {
                    return Err(format!("XMAP04 map shader resource {symbol} is malformed").into());
                }
                Ok(NativeXmap04MapShaderResourceView {
                    binding: texture.binding.clone(),
                    raw_srv: texture.view.as_raw(),
                    raw_device: self.device.as_raw(),
                    _owner: std::marker::PhantomData,
                })
            })
            .collect()
    }

    pub unsafe fn bind_xmap05_dependencies(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        political: &NativeXmap05PoliticalHandoff,
    ) -> Result<&Xmap04D3dHandoffReceipt> {
        if context.is_null() {
            return Err("XMAP04/XMAP05 terrain binding received a null D3D11 context".into());
        }
        let bindings = &political.receipt().terrain_dependency_bindings;
        validate_xmap05_dependency_bindings(bindings)?;
        for binding in bindings {
            let slot = self
                .downstream_pixel_slots
                .get(&binding.symbol)
                .copied()
                .ok_or_else(|| {
                    format!(
                        "XMAP04 admitted terrain program has no reflected XMAP05 sampler {}",
                        binding.symbol
                    )
                })?;
            let view = political.shader_resource_view(&binding.symbol)?;
            (*context).PSSetShaderResources(slot, 1, &view);
        }
        record_xmap05_dependency_bindings(&mut self.receipt, bindings)?;
        Ok(&self.receipt)
    }

    /// for current replay/camera identity and same-device ownership before the
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn render_visible_terrain(
        &self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        frame: &PreparedTerrainFrame,
        target: &crate::NativeProductionMainSceneTarget,
        political: &crate::NativeXmap05PoliticalHandoff,
        water: &crate::NativeXmap07WaterHandoff,
        lighting: &crate::NativeXmap09LightingHandoff,
        scene_cb0s: &[&crate::NativeXmap10MapCopySceneCb0View<'_>],
        shadow_map: &crate::NativeXmap10ShaderResourceView<'_>,
    ) -> Result<Xmap04TerrainDrawReceipt> {
        use std::{collections::BTreeMap, ptr};
        use winapi::shared::dxgiformat::DXGI_FORMAT_R16_UINT;
        use winapi::um::d3d11::{D3D11_CLEAR_DEPTH, D3D11_CLEAR_STENCIL, D3D11_VIEWPORT};
        use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

        frame.validate()?;
        target.desc().validate()?;
        if context.is_null()
            || frame.deterministic_sha256 != self.receipt.terrain_frame_sha256
            || frame.program.deterministic_sha256 != self.receipt.terrain_program_sha256
            || target.device_identity() == 0
            || target.allocation_generation() == 0
            || !canonical_sha256(target.resource_identity_sha256())
            || !canonical_sha256(target.producer_frame_sha256())
        {
            return Err("XMAP04 terrain draw received a stale handoff or target".into());
        }
        let context_device = d3d11_device_child_identity(context as *mut _, "terrain context")?;
        if context_device != target.device_identity()
            || d3d11_device_child_identity(self.vertex_shader.as_raw() as *mut _, "terrain VS")?
                != context_device
            || d3d11_device_child_identity(target.color_rtv() as *mut _, "MainScene RTV")?
                != context_device
            || d3d11_device_child_identity(target.depth_dsv() as *mut _, "MainScene DSV")?
                != context_device
        {
            return Err("XMAP04 terrain program/target belongs to another D3D11 device".into());
        }

        let map_copy_rows = frame.ordered_map_copy_receipts()?;
        let map_copy_receipt_set_sha256 = frame.map_copy_receipt_set_sha256()?;
        if scene_cb0s.len() != map_copy_rows.len()
            || scene_cb0s.iter().zip(&map_copy_rows).any(|(scene, row)| {
                scene.owner_packet() != "XMAP10"
                    || scene.symbol() != "SceneCB0"
                    || scene.shader_register() != 0
                    || scene.byte_len() != 416
                    || scene.camera_frame_sha256() != frame.camera_frame_sha256
                    || scene.replay_frame_identity_sha256() != frame.replay_frame_identity_sha256
                    || scene.map_copy_receipt_set_sha256() != map_copy_receipt_set_sha256
                    || scene.render_view_mask() != row.render_view_mask
                    || scene.map_copy_offset_f32_bits() != row.map_copy_offset_f32_bits
                    || scene.viewport_width() != target.desc().color_width
                    || scene.viewport_height() != target.desc().color_height
                    || scene.main_scene_resource_identity_sha256()
                        != target.resource_identity_sha256()
                    || scene.main_scene_allocation_generation() != target.allocation_generation()
                    || scene.device_identity() != context_device
                    || scene.raw_buffer().is_null()
            })
        {
            return Err(
                "XMAP04 terrain SceneCB0 census differs from current map-copy/target identity"
                    .into(),
            );
        }
        for scene in scene_cb0s {
            if d3d11_device_child_identity(scene.raw_buffer() as *mut _, "terrain SceneCB0")?
                != context_device
            {
                return Err("XMAP04 terrain SceneCB0 belongs to another D3D11 device".into());
            }
        }

        let point_lights = lighting.point_lights_cb2_view();
        if point_lights.binding.owner_packet != "XMAP09"
            || point_lights.binding.symbol != "PointLightsCB2"
            || point_lights.binding.shader_register != 2
            || point_lights.binding.camera_frame_sha256 != frame.camera_frame_sha256
            || point_lights.binding.replay_frame_identity_sha256
                != frame.replay_frame_identity_sha256
            || point_lights.as_raw().is_null()
            || d3d11_device_child_identity(point_lights.as_raw() as *mut _, "PointLightsCB2")?
                != context_device
        {
            return Err(
                "XMAP04 terrain PointLightsCB2 is stale or belongs to another device".into(),
            );
        }

        let light_index = lighting.light_index_map_view().at_shader_register(10);
        let light_data = lighting.light_data_map_view().at_shader_register(11);
        let snow_mud = water.terrain_snow_mud_view()?;
        let mut resources = BTreeMap::new();
        let mut insert = |binding: TerrainDependentResourceBinding,
                          raw: *mut winapi::um::d3d11::ID3D11ShaderResourceView|
         -> Result<()> {
            if raw.is_null()
                || d3d11_device_child_identity(raw as *mut _, &binding.symbol)? != context_device
                || resources
                    .insert(binding.symbol.clone(), (binding, raw))
                    .is_some()
            {
                return Err(
                    "XMAP04 terrain dependency is null, duplicated, or cross-device".into(),
                );
            }
            Ok(())
        };
        for symbol in [
            "ProvinceSecondaryColorMap",
            "GradientBorderChannel1",
            "GradientBorderChannel2",
        ] {
            let view = political.shader_resource(symbol)?;
            insert(
                TerrainDependentResourceBinding {
                    symbol: view.symbol.into(),
                    owner_packet: view.owner_packet.into(),
                    resource_sha256: view.resource_sha256.into(),
                    descriptor_sha256: view.descriptor_sha256.into(),
                },
                view.shader_resource_view,
            )?;
        }
        insert(
            TerrainDependentResourceBinding {
                symbol: snow_mud.binding.symbol.clone(),
                owner_packet: snow_mud.binding.owner_packet.clone(),
                resource_sha256: snow_mud.binding.resource_sha256.clone(),
                descriptor_sha256: snow_mud.binding.descriptor_sha256.clone(),
            },
            snow_mud.as_raw(),
        )?;
        for view in [&light_index, &light_data] {
            insert(
                TerrainDependentResourceBinding {
                    symbol: view.binding.symbol.clone(),
                    owner_packet: view.binding.owner_packet.clone(),
                    resource_sha256: view.binding.resource_sha256.clone(),
                    descriptor_sha256: view.binding.descriptor_sha256.clone(),
                },
                view.as_raw(),
            )?;
        }
        insert(
            TerrainDependentResourceBinding {
                symbol: shadow_map.binding().symbol.clone(),
                owner_packet: shadow_map.binding().owner_packet.clone(),
                resource_sha256: shadow_map.resource_sha256().into(),
                descriptor_sha256: shadow_map.descriptor_sha256().into(),
            },
            shadow_map.as_raw(),
        )?;
        if shadow_map.device_identity() != context_device
            || shadow_map.frame_identity_sha256() != frame.camera_frame_sha256
            || shadow_map.replay_frame_identity_sha256() != frame.replay_frame_identity_sha256
        {
            return Err("XMAP04 terrain ShadowMap is stale or cross-device".into());
        }

        let expected_symbols = [
            "ProvinceSecondaryColorMap",
            "SnowMudData",
            "LightIndexMap",
            "LightDataMap",
            "ShadowMap",
            "GradientBorderChannel1",
            "GradientBorderChannel2",
        ];
        let ordered_dependency_resources = expected_symbols
            .iter()
            .map(|symbol| {
                resources
                    .get(*symbol)
                    .map(|(binding, _)| binding.clone())
                    .ok_or_else(|| format!("XMAP04 terrain omitted dependency {symbol}").into())
            })
            .collect::<Result<Vec<_>>>()?;
        let ready =
            frame.admit_visible_draw_dependencies(crate::TerrainVisibleDrawDependencies {
                scene_cb0_sha256: scene_cb0s
                    .iter()
                    .find(|scene| scene.render_view_mask() == 1)
                    .ok_or("XMAP04 terrain omitted main-map SceneCB0")?
                    .payload_sha256()
                    .into(),
                resources: ordered_dependency_resources.clone(),
            })?;

        let rtv = target.color_rtv();
        let dsv = target.depth_dsv();
        (*context).OMSetRenderTargets(1, &rtv, dsv);
        (*context).ClearRenderTargetView(rtv, &[0.0, 0.0, 0.0, 0.0]);
        (*context).ClearDepthStencilView(dsv, D3D11_CLEAR_DEPTH | D3D11_CLEAR_STENCIL, 1.0, 0);
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: target.desc().color_width as f32,
            Height: target.desc().color_height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        (*context).RSSetViewports(1, &viewport);
        (*context).RSSetScissorRects(0, ptr::null());
        (*context).RSSetState(self.terrain_rasterizer_state.as_raw());
        (*context).OMSetBlendState(self.terrain_blend_state.as_raw(), &[1.0; 4], u32::MAX);
        (*context).OMSetDepthStencilState(self.terrain_depth_state.as_raw(), 0);
        (*context).IASetInputLayout(self.input_layout.as_raw());
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        (*context).IASetIndexBuffer(self.shared_index.as_raw(), DXGI_FORMAT_R16_UINT, 0);
        (*context).VSSetShader(self.vertex_shader.as_raw(), ptr::null_mut(), 0);
        (*context).PSSetShader(self.pixel_shader.as_raw(), ptr::null_mut(), 0);
        (*context).GSSetShader(ptr::null_mut(), ptr::null_mut(), 0);
        (*context).HSSetShader(ptr::null_mut(), ptr::null_mut(), 0);
        (*context).DSSetShader(ptr::null_mut(), ptr::null_mut(), 0);
        for sampler in &frame.program.samplers {
            let raw = if sampler.name == "TerrainIDMap" {
                self.terrain_id_map_view.as_raw()
            } else if sampler.owner_packet == "XMAP04" {
                self.terrain_static_textures
                    .get(&sampler.name)
                    .ok_or_else(|| format!("XMAP04 terrain omitted static SRV {}", sampler.name))?
                    .view
                    .as_raw()
            } else {
                resources
                    .get(&sampler.name)
                    .ok_or_else(|| format!("XMAP04 terrain omitted joined SRV {}", sampler.name))?
                    .1
            };
            (*context).PSSetShaderResources(sampler.shader_register, 1, &raw);
        }
        for (slot, sampler) in &self.terrain_samplers {
            let raw = sampler.as_raw();
            (*context).PSSetSamplers(*slot, 1, &raw);
        }
        let point_lights_raw = point_lights.as_raw();
        (*context).PSSetConstantBuffers(2, 1, &point_lights_raw);
        let slot0 = self.shared_slot0.as_raw();
        let mut strides = [
            crate::TERRAIN_CHUNK_SLOT0_STRIDE,
            crate::TERRAIN_CHUNK_SLOT1_STRIDE,
        ];
        let mut offsets = [0u32, 0u32];
        for (draw_index, draw) in frame.draws.iter().enumerate() {
            let scene = scene_cb0s
                .iter()
                .find(|scene| scene.render_view_mask() == draw.render_view_mask)
                .ok_or("XMAP04 terrain draw omitted its map-copy SceneCB0")?;
            let upload = self
                .draw_uploads
                .get(draw_index)
                .ok_or("XMAP04 terrain draw upload census changed")?;
            let buffers = [slot0, upload.slot1.as_raw()];
            (*context).IASetVertexBuffers(
                0,
                2,
                buffers.as_ptr(),
                strides.as_mut_ptr(),
                offsets.as_mut_ptr(),
            );
            let scene_raw = scene.raw_buffer();
            (*context).VSSetConstantBuffers(0, 1, &scene_raw);
            (*context).PSSetConstantBuffers(0, 1, &scene_raw);
            let cb1 = upload.cb1.as_raw();
            (*context).VSSetConstantBuffers(1, 1, &cb1);
            (*context).DrawIndexed(crate::TERRAIN_CHUNK_INDEX_COUNT, 0, 0);
        }
        let null_srvs = [ptr::null_mut(); 16];
        (*context).PSSetShaderResources(0, 16, null_srvs.as_ptr());

        let mut receipt = Xmap04TerrainDrawReceipt {
            schema: XMAP04_TERRAIN_DRAW_RECEIPT_SCHEMA.into(),
            terrain_frame_sha256: frame.deterministic_sha256.clone(),
            terrain_program_sha256: frame.program.deterministic_sha256.clone(),
            ready_draw_contract_sha256: ready.deterministic_sha256,
            map_copy_receipt_set_sha256,
            main_scene_resource_identity_sha256: target.resource_identity_sha256().into(),
            main_scene_producer_frame_sha256: target.producer_frame_sha256().into(),
            main_scene_allocation_generation: target.allocation_generation(),
            main_scene_descriptor_sha256: Sha256::hex(&serde_json::to_vec(target.desc())?),
            ordered_scene_cb0_composition_sha256s: scene_cb0s
                .iter()
                .map(|scene| scene.composition_receipt_sha256().to_owned())
                .collect(),
            point_lights_cb2_sha256: point_lights.binding.payload_sha256.clone(),
            point_lights_cb2_descriptor_sha256: point_lights.binding.descriptor_sha256.clone(),
            ordered_dependency_resources,
            draw_count: u32::try_from(frame.draws.len())?,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = terrain_draw_receipt_digest(&receipt);
        receipt.validate()?;
        Ok(receipt)
    }

    pub unsafe fn render_underwater_refraction<T, S, P>(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        frame: &PreparedTerrainFrame,
        target: &T,
        scene_cb0: &S,
        point_lights_cb2: &P,
        light_resources: &[&dyn NativeXmap04UnderwaterShaderResource],
    ) -> Result<Xmap04UnderwaterDrawReceipt>
    where
        T: NativeXmap04UnderwaterTarget,
        S: NativeXmap04UnderwaterSceneConstantBuffer,
        P: NativeXmap04UnderwaterPointLightsConstantBuffer,
    {
        use winapi::shared::dxgiformat::DXGI_FORMAT_R16_UINT;
        use winapi::um::d3d11::{D3D11_CLEAR_DEPTH, D3D11_VIEWPORT};
        use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

        frame.validate()?;
        let program_desc = frame
            .underwater_program
            .as_ref()
            .ok_or("XMAP04 underwater draw is unavailable for the low-GFX profile")?;
        let program = self
            .underwater
            .as_ref()
            .ok_or("XMAP04 native handoff omitted the admitted underwater program")?;
        target.target_contract().validate()?;
        let target_rtv = target.raw_render_target_view();
        let target_srv = target.raw_shader_resource_view();
        let scene_buffer = scene_cb0.raw_buffer();
        let point_lights_buffer = point_lights_cb2.raw_buffer();
        let light_resource_views = light_resources
            .iter()
            .map(|resource| resource.raw_shader_resource_view())
            .collect::<Vec<_>>();
        if context.is_null()
            || frame.deterministic_sha256 != self.receipt.terrain_frame_sha256
            || program_desc.deterministic_sha256 != program.deterministic_sha256
            || target_rtv.is_null()
            || target_srv.is_null()
            || target.owner_packet() != "XMAP07"
            || target.dxgi_format() != XMAP04_WATER_REFRACTION_DXGI_FORMAT
            || target.width() == 0
            || target.height() == 0
            || target.width() != scene_cb0.viewport_width()
            || target.height() != scene_cb0.viewport_height()
            || scene_cb0.frame_identity_sha256() != frame.camera_frame_sha256
            || scene_cb0.replay_frame_identity_sha256() != frame.replay_frame_identity_sha256
            || !canonical_sha256(scene_cb0.frame_identity_sha256())
            || !canonical_sha256(scene_cb0.replay_frame_identity_sha256())
            || !canonical_sha256(target.resource_identity_sha256())
            || !canonical_sha256(target.descriptor_sha256())
            || !canonical_sha256(target.texture_descriptor_sha256())
            || !canonical_sha256(target.rtv_descriptor_sha256())
            || !canonical_sha256(target.srv_descriptor_sha256())
            || !canonical_sha256(target.depth_descriptor_sha256())
            || target.descriptor_sha256()
                != xmap04_underwater_color_descriptor_sha256(
                    target.width(),
                    target.height(),
                    target.dxgi_format(),
                    target.sample_count(),
                    target.sample_quality(),
                )
            || target.depth_descriptor_sha256()
                != xmap04_underwater_depth_descriptor_sha256(
                    target.width(),
                    target.height(),
                    target.depth_dxgi_format(),
                    target.depth_sample_count(),
                    target.depth_sample_quality(),
                    target.depth_mip_levels(),
                    target.depth_array_size(),
                    target.depth_has_stencil(),
                )
            || target.sample_count() != 1
            || target.sample_quality() != 0
            || target.texture_mip_levels() != 1
            || target.texture_array_size() != 1
            || target.texture_usage() != 0
            || target.texture_bind_flags()
                != (winapi::um::d3d11::D3D11_BIND_RENDER_TARGET
                    | winapi::um::d3d11::D3D11_BIND_SHADER_RESOURCE)
            || target.texture_cpu_access_flags() != 0
            || target.texture_misc_flags() != 0
            || target.sample_count() != target.depth_sample_count()
            || target.sample_quality() != target.depth_sample_quality()
            || target.depth_mip_levels() != 1
            || target.depth_array_size() != 1
        {
            return Err(
                "XMAP04 underwater draw received an invalid XMAP07 viewport refraction target"
                    .into(),
            );
        }
        validate_underwater_target_identity(target, target_rtv, target_srv)?;
        validate_underwater_cb(
            scene_cb0.owner_packet(),
            scene_cb0.symbol(),
            scene_cb0.shader_register(),
            scene_cb0.byte_len(),
            scene_cb0.payload_sha256(),
            scene_cb0.descriptor_sha256(),
            scene_cb0.raw_buffer(),
            "XMAP10",
            "SceneCB0",
            0,
            416,
        )?;
        validate_underwater_cb(
            point_lights_cb2.owner_packet(),
            point_lights_cb2.symbol(),
            point_lights_cb2.shader_register(),
            point_lights_cb2.byte_len(),
            point_lights_cb2.payload_sha256(),
            point_lights_cb2.descriptor_sha256(),
            point_lights_cb2.raw_buffer(),
            "XMAP09",
            "PointLightsCB2",
            2,
            16,
        )?;
        if point_lights_cb2.frame_identity_sha256() != frame.camera_frame_sha256
            || point_lights_cb2.replay_frame_identity_sha256() != frame.replay_frame_identity_sha256
            || !canonical_sha256(point_lights_cb2.frame_identity_sha256())
            || !canonical_sha256(point_lights_cb2.replay_frame_identity_sha256())
        {
            return Err(
                "XMAP04 underwater PointLightsCB2 is not from the current replay/camera frame"
                    .into(),
            );
        }
        let ordered_light_resources = validate_underwater_light_resources(
            light_resources,
            &light_resource_views,
            &frame.camera_frame_sha256,
            &frame.replay_frame_identity_sha256,
        )?;

        let device = validate_underwater_device_ownership(
            context,
            self.vertex_shader.as_raw(),
            target_rtv,
            scene_buffer,
            point_lights_buffer,
            &light_resource_views,
        )?;
        let needs_depth = self.underwater_depth.as_ref().is_none_or(|depth| {
            depth.width != target.width()
                || depth.height != target.height()
                || depth.descriptor_sha256 != target.depth_descriptor_sha256()
        });
        if needs_depth {
            self.underwater_depth = Some(create_underwater_depth(
                device.as_raw(),
                target.width(),
                target.height(),
                target.depth_dxgi_format(),
                target.depth_sample_count(),
                target.depth_sample_quality(),
                target.depth_mip_levels(),
                target.depth_array_size(),
                target.depth_has_stencil(),
                target.depth_descriptor_sha256(),
            )?);
        }
        let depth = self
            .underwater_depth
            .as_ref()
            .ok_or("XMAP04 underwater depth target was not retained")?;

        let color = target_rtv;
        let depth_view = depth.view.as_raw();
        (*context).OMSetRenderTargets(1, &color, depth_view);
        (*context).ClearRenderTargetView(color, &[0.0, 0.0, 0.0, 0.0]);
        let clear_flags = if depth.has_stencil {
            D3D11_CLEAR_DEPTH | winapi::um::d3d11::D3D11_CLEAR_STENCIL
        } else {
            D3D11_CLEAR_DEPTH
        };
        (*context).ClearDepthStencilView(depth_view, clear_flags, 1.0, 0);
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: target.width() as f32,
            Height: target.height() as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        (*context).RSSetViewports(1, &viewport);
        (*context).RSSetState(program.rasterizer_state.as_raw());
        (*context).OMSetBlendState(
            program.blend_state.as_raw(),
            &[0.0, 0.0, 0.0, 0.0],
            u32::MAX,
        );
        (*context).OMSetDepthStencilState(program.depth_state.as_raw(), 0);
        (*context).IASetInputLayout(program.input_layout.as_raw());
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        (*context).IASetIndexBuffer(self.shared_index.as_raw(), DXGI_FORMAT_R16_UINT, 0);
        (*context).HSSetShader(std::ptr::null_mut(), std::ptr::null(), 0);
        (*context).DSSetShader(std::ptr::null_mut(), std::ptr::null(), 0);
        (*context).GSSetShader(std::ptr::null_mut(), std::ptr::null(), 0);
        (*context).VSSetShader(program.vertex_shader.as_raw(), std::ptr::null(), 0);
        (*context).PSSetShader(program.pixel_shader.as_raw(), std::ptr::null(), 0);
        let scene = scene_buffer;
        (*context).VSSetConstantBuffers(0, 1, &scene);
        (*context).PSSetConstantBuffers(0, 1, &scene);
        let point_lights = point_lights_buffer;
        (*context).PSSetConstantBuffers(2, 1, &point_lights);
        for (slot, sampler) in &program.samplers {
            let raw = sampler.as_raw();
            (*context).PSSetSamplers(*slot, 1, &raw);
        }
        for (name, texture) in &program.static_textures {
            let slot = program_desc
                .samplers
                .iter()
                .find(|sampler| &sampler.name == name)
                .map(|sampler| sampler.shader_register)
                .ok_or("XMAP04 retained underwater texture has no reflected sampler")?;
            let raw = texture.view.as_raw();
            (*context).PSSetShaderResources(slot, 1, &raw);
        }
        for (resource, raw) in light_resources.iter().zip(light_resource_views) {
            (*context).PSSetShaderResources(resource.shader_register(), 1, &raw);
        }

        let slot0 = self.shared_slot0.as_raw();
        let mut strides = [
            crate::TERRAIN_CHUNK_SLOT0_STRIDE,
            crate::TERRAIN_CHUNK_SLOT1_STRIDE,
        ];
        let mut offsets = [0u32, 0u32];
        for upload in &self.draw_uploads {
            let slot1 = upload.slot1.as_raw();
            let buffers = [slot0, slot1];
            (*context).IASetVertexBuffers(
                0,
                2,
                buffers.as_ptr(),
                strides.as_mut_ptr(),
                offsets.as_mut_ptr(),
            );
            let cb1 = upload.cb1.as_raw();
            (*context).VSSetConstantBuffers(1, 1, &cb1);
            (*context).DrawIndexed(crate::TERRAIN_CHUNK_INDEX_COUNT, 0, 0);
        }

        let mut receipt = Xmap04UnderwaterDrawReceipt {
            terrain_frame_sha256: frame.deterministic_sha256.clone(),
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            camera_frame_sha256: frame.camera_frame_sha256.clone(),
            xmap09_camera_frame_sha256: point_lights_cb2.frame_identity_sha256().into(),
            underwater_program_sha256: program_desc.deterministic_sha256.clone(),
            target_resource_identity_sha256: target.resource_identity_sha256().into(),
            target_producer_frame_sha256: target.target_contract().producer_frame_sha256.clone(),
            target_producer_allocation_generation: target.producer_allocation_generation(),
            target_producer_allocation_identity_sha256: target
                .producer_allocation_identity_sha256()
                .into(),
            target_texture_descriptor_sha256: target.texture_descriptor_sha256().into(),
            target_rtv_descriptor_sha256: target.rtv_descriptor_sha256().into(),
            target_srv_descriptor_sha256: target.srv_descriptor_sha256().into(),
            target_descriptor_sha256: target.descriptor_sha256().into(),
            depth_descriptor_sha256: target.depth_descriptor_sha256().into(),
            scene_cb0_sha256: scene_cb0.payload_sha256().into(),
            scene_cb0_descriptor_sha256: scene_cb0.descriptor_sha256().into(),
            point_lights_cb2_sha256: point_lights_cb2.payload_sha256().into(),
            point_lights_cb2_descriptor_sha256: point_lights_cb2.descriptor_sha256().into(),
            ordered_light_resources,
            draw_count: u32::try_from(self.draw_uploads.len())?,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = underwater_draw_receipt_digest(&receipt);
        receipt.validate()?;
        Ok(receipt)
    }

    pub unsafe fn bind_xmap07_dependencies(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        water: &NativeXmap07WaterHandoff,
    ) -> Result<&Xmap04D3dHandoffReceipt> {
        if context.is_null() {
            return Err("XMAP04/XMAP07 terrain binding received a null D3D11 context".into());
        }
        let view = water.terrain_snow_mud_view()?;
        let slot = self
            .downstream_pixel_slots
            .get("SnowMudData")
            .copied()
            .ok_or("XMAP04 admitted terrain program has no SnowMudData sampler")?;
        (*context).PSSetShaderResources(slot, 1, &view.as_raw());
        let binding = TerrainDependentResourceBinding {
            symbol: "SnowMudData".into(),
            owner_packet: "XMAP07".into(),
            resource_sha256: view.binding.resource_sha256,
            descriptor_sha256: view.binding.descriptor_sha256,
        };
        self.receipt
            .bound_downstream_resources
            .retain(|existing| existing.symbol != "SnowMudData");
        self.receipt.bound_downstream_resources.push(binding);
        self.receipt
            .blocked_visible_draw_packets
            .retain(|packet| packet != "XMAP07");
        self.receipt.deterministic_sha256 = handoff_receipt_digest(&self.receipt);
        Ok(&self.receipt)
    }
}

fn validate_xmap05_dependency_bindings(bindings: &[TerrainDependentResourceBinding]) -> Result<()> {
    const EXPECTED: [&str; 3] = [
        "ProvinceSecondaryColorMap",
        "GradientBorderChannel1",
        "GradientBorderChannel2",
    ];
    if bindings.len() != EXPECTED.len()
        || bindings
            .iter()
            .zip(EXPECTED)
            .any(|(binding, symbol)| binding.owner_packet != "XMAP05" || binding.symbol != symbol)
    {
        return Err("XMAP05 terrain dependency binding set is incomplete or misordered".into());
    }
    Ok(())
}

fn record_xmap05_dependency_bindings(
    receipt: &mut Xmap04D3dHandoffReceipt,
    bindings: &[TerrainDependentResourceBinding],
) -> Result<()> {
    validate_xmap05_dependency_bindings(bindings)?;
    receipt.bound_downstream_resources = bindings.to_vec();
    receipt
        .blocked_visible_draw_packets
        .retain(|packet| packet != "XMAP05");
    receipt.deterministic_sha256 = handoff_receipt_digest(receipt);
    Ok(())
}

fn canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && !value.bytes().all(|byte| byte == b'0')
}

fn validate_underwater_device_identity(
    expected_device_identity: usize,
    actual_device_identity: usize,
    resource: &str,
) -> Result<()> {
    if expected_device_identity == 0
        || actual_device_identity == 0
        || actual_device_identity != expected_device_identity
    {
        return Err(
            format!("XMAP04 underwater {resource} belongs to a different D3D11 device").into(),
        );
    }
    Ok(())
}

fn require_underwater_device_object<T>(object: *mut T, resource: &str) -> Result<()> {
    if object.is_null() {
        Err(format!("XMAP04 underwater {resource} returned null").into())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
unsafe fn validate_underwater_device_ownership(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    handoff_vertex_shader: *mut winapi::um::d3d11::ID3D11VertexShader,
    target_rtv: *mut winapi::um::d3d11::ID3D11RenderTargetView,
    scene_cb0: *mut winapi::um::d3d11::ID3D11Buffer,
    point_lights_cb2: *mut winapi::um::d3d11::ID3D11Buffer,
    light_resources: &[*mut winapi::um::d3d11::ID3D11ShaderResourceView],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Device>> {
    use crate::d3d11_device::D3dPtr;
    use std::ptr;
    use winapi::um::d3d11::ID3D11Device;

    if context.is_null() {
        return Err("XMAP04 underwater draw received a null D3D11 context".into());
    }
    let mut context_device = ptr::null_mut::<ID3D11Device>();
    (*context).GetDevice(&mut context_device);
    let context_device = D3dPtr::from_raw(context_device, "XMAP04 underwater context device")?;
    let expected_identity = context_device.as_raw() as usize;

    macro_rules! require_context_device {
        ($object:expr, $resource:expr) => {{
            let object = $object;
            require_underwater_device_object(object, $resource)?;
            let mut owner = ptr::null_mut::<ID3D11Device>();
            (*object).GetDevice(&mut owner);
            let owner_role = format!("XMAP04 underwater {} owning device", $resource);
            let owner = D3dPtr::from_raw(owner, &owner_role)?;
            validate_underwater_device_identity(
                expected_identity,
                owner.as_raw() as usize,
                $resource,
            )?;
        }};
    }

    require_context_device!(handoff_vertex_shader, "retained handoff vertex shader");
    require_context_device!(target_rtv, "XMAP07 WaterRefraction RTV");
    require_context_device!(scene_cb0, "XMAP10 SceneCB0");
    require_context_device!(point_lights_cb2, "XMAP09 PointLightsCB2");
    const LIGHT_RESOURCE_NAMES: [&str; 3] = ["LightIndexMap", "LightDataMap", "ShadowMap"];
    if light_resources.len() != LIGHT_RESOURCE_NAMES.len() {
        return Err("XMAP04 underwater light/shadow ownership set is incomplete".into());
    }
    for (resource, name) in light_resources.iter().copied().zip(LIGHT_RESOURCE_NAMES) {
        require_context_device!(resource, name);
    }

    Ok(context_device)
}

#[cfg(windows)]
unsafe fn validate_underwater_target_identity<T: NativeXmap04UnderwaterTarget>(
    target: &T,
    rtv: *mut winapi::um::d3d11::ID3D11RenderTargetView,
    srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
) -> Result<()> {
    use std::ptr;
    use winapi::shared::winerror::SUCCEEDED;
    use winapi::um::d3d11::{
        ID3D11Resource, ID3D11Texture2D, D3D11_RENDER_TARGET_VIEW_DESC,
        D3D11_RTV_DIMENSION_TEXTURE2D, D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_TEXTURE2D_DESC,
    };
    use winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURE2D;
    use winapi::um::unknwnbase::IUnknown;
    use winapi::Interface;

    let mut rtv_resource = ptr::null_mut::<ID3D11Resource>();
    let mut srv_resource = ptr::null_mut::<ID3D11Resource>();
    (*rtv).GetResource(&mut rtv_resource);
    (*srv).GetResource(&mut srv_resource);
    let rtv_resource = crate::d3d11_device::D3dPtr::from_raw(
        rtv_resource,
        "XMAP04 underwater XMAP07 RTV resource",
    )?;
    let srv_resource = crate::d3d11_device::D3dPtr::from_raw(
        srv_resource,
        "XMAP04 underwater XMAP07 SRV resource",
    )?;
    if rtv_resource.as_raw() != srv_resource.as_raw() {
        return Err(
            "XMAP04 underwater XMAP07 RTV and SRV do not reference the same texture resource"
                .into(),
        );
    }

    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    let hr = (*(rtv_resource.as_raw() as *mut IUnknown)).QueryInterface(
        &ID3D11Texture2D::uuidof(),
        &mut texture as *mut *mut ID3D11Texture2D as *mut *mut _,
    );
    if !SUCCEEDED(hr) {
        return Err(format!(
            "XMAP04 underwater XMAP07 refraction resource is not Texture2D: 0x{:08x}",
            hr as u32
        )
        .into());
    }
    let texture =
        crate::d3d11_device::D3dPtr::from_raw(texture, "XMAP04 underwater XMAP07 Texture2D")?;
    let mut texture_desc: D3D11_TEXTURE2D_DESC = std::mem::zeroed();
    (*texture.as_raw()).GetDesc(&mut texture_desc);
    let texture_hash = xmap04_underwater_texture_descriptor_sha256(
        texture_desc.Width,
        texture_desc.Height,
        texture_desc.Format,
        texture_desc.SampleDesc.Count,
        texture_desc.SampleDesc.Quality,
        texture_desc.MipLevels,
        texture_desc.ArraySize,
        texture_desc.Usage,
        texture_desc.BindFlags,
        texture_desc.CPUAccessFlags,
        texture_desc.MiscFlags,
    );

    let mut rtv_desc: D3D11_RENDER_TARGET_VIEW_DESC = std::mem::zeroed();
    (*rtv).GetDesc(&mut rtv_desc);
    let (rtv_mip, rtv_first, rtv_count) = if rtv_desc.ViewDimension == D3D11_RTV_DIMENSION_TEXTURE2D
    {
        ((*rtv_desc.u.Texture2D()).MipSlice, 0, 1)
    } else {
        return Err("XMAP04 underwater XMAP07 RTV is not a Texture2D view".into());
    };
    let rtv_hash = xmap04_underwater_rtv_descriptor_sha256(
        rtv_desc.Format,
        rtv_desc.ViewDimension,
        rtv_mip,
        rtv_first,
        rtv_count,
    );

    let mut srv_desc: D3D11_SHADER_RESOURCE_VIEW_DESC = std::mem::zeroed();
    (*srv).GetDesc(&mut srv_desc);
    let (srv_most_detailed, srv_mips, srv_first, srv_count) =
        if srv_desc.ViewDimension == D3D11_SRV_DIMENSION_TEXTURE2D {
            let texture2d = *srv_desc.u.Texture2D();
            (texture2d.MostDetailedMip, texture2d.MipLevels, 0, 1)
        } else {
            return Err("XMAP04 underwater XMAP07 SRV is not a Texture2D view".into());
        };
    let srv_hash = xmap04_underwater_srv_descriptor_sha256(
        srv_desc.Format,
        srv_desc.ViewDimension,
        srv_most_detailed,
        srv_mips,
        srv_first,
        srv_count,
    );

    if texture_desc.Width != target.width()
        || texture_desc.Height != target.height()
        || texture_desc.Format != target.dxgi_format()
        || texture_desc.SampleDesc.Count != target.sample_count()
        || texture_desc.SampleDesc.Quality != target.sample_quality()
        || texture_desc.MipLevels != target.texture_mip_levels()
        || texture_desc.ArraySize != target.texture_array_size()
        || texture_desc.Usage != target.texture_usage()
        || texture_desc.BindFlags != target.texture_bind_flags()
        || texture_desc.CPUAccessFlags != target.texture_cpu_access_flags()
        || texture_desc.MiscFlags != target.texture_misc_flags()
        || texture_hash != target.texture_descriptor_sha256()
        || rtv_desc.Format != target.rtv_dxgi_format()
        || rtv_desc.ViewDimension != target.rtv_view_dimension()
        || rtv_mip != target.rtv_mip_slice()
        || rtv_first != target.rtv_first_array_slice()
        || rtv_count != target.rtv_array_size()
        || rtv_hash != target.rtv_descriptor_sha256()
        || srv_desc.Format != target.srv_dxgi_format()
        || srv_desc.ViewDimension != target.srv_view_dimension()
        || srv_most_detailed != target.srv_most_detailed_mip()
        || srv_mips != target.srv_mip_levels()
        || srv_first != target.srv_first_array_slice()
        || srv_count != target.srv_array_size()
        || srv_hash != target.srv_descriptor_sha256()
        || rtv_desc.Format != texture_desc.Format
        || srv_desc.Format != texture_desc.Format
        || rtv_mip != 0
        || srv_most_detailed != 0
        || srv_mips != 1
    {
        return Err(
            "XMAP04 underwater XMAP07 texture/RTV/SRV identity or subresource contract is invalid"
                .into(),
        );
    }
    Ok(())
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn validate_underwater_cb(
    owner: &str,
    symbol: &str,
    register: u32,
    byte_len: u32,
    payload_sha256: &str,
    descriptor_sha256: &str,
    raw: *mut winapi::um::d3d11::ID3D11Buffer,
    expected_owner: &str,
    expected_symbol: &str,
    expected_register: u32,
    expected_len: u32,
) -> Result<()> {
    if owner != expected_owner
        || symbol != expected_symbol
        || register != expected_register
        || byte_len != expected_len
        || !canonical_sha256(payload_sha256)
        || !canonical_sha256(descriptor_sha256)
        || raw.is_null()
    {
        return Err(format!(
            "XMAP04 underwater {expected_symbol} provider is incomplete or misowned"
        )
        .into());
    }
    Ok(())
}

#[cfg(windows)]
fn validate_underwater_light_resources(
    resources: &[&dyn NativeXmap04UnderwaterShaderResource],
    raw_views: &[*mut winapi::um::d3d11::ID3D11ShaderResourceView],
    expected_camera_frame_sha256: &str,
    expected_replay_frame_sha256: &str,
) -> Result<Vec<TerrainDependentResourceBinding>> {
    const EXPECTED: [(&str, u32, &str); 3] = [
        ("LightIndexMap", 10, "XMAP09"),
        ("LightDataMap", 11, "XMAP09"),
        ("ShadowMap", 12, "XMAP10"),
    ];
    if resources.len() != EXPECTED.len() || raw_views.len() != EXPECTED.len() {
        return Err("XMAP04 underwater light/shadow resource set is incomplete".into());
    }
    let mut bindings = Vec::with_capacity(EXPECTED.len());
    for ((resource, raw_view), (symbol, slot, owner)) in
        resources.iter().zip(raw_views).zip(EXPECTED)
    {
        if resource.owner_packet() != owner
            || resource.symbol() != symbol
            || resource.shader_register() != slot
            || resource.frame_identity_sha256() != expected_camera_frame_sha256
            || resource.replay_frame_identity_sha256() != expected_replay_frame_sha256
            || !canonical_sha256(resource.frame_identity_sha256())
            || !canonical_sha256(resource.replay_frame_identity_sha256())
            || !canonical_sha256(resource.resource_sha256())
            || !canonical_sha256(resource.descriptor_sha256())
            || raw_view.is_null()
        {
            return Err(format!(
                "XMAP04 underwater {owner} resource {symbol} at t{slot} is invalid"
            )
            .into());
        }
        bindings.push(TerrainDependentResourceBinding {
            symbol: symbol.into(),
            owner_packet: owner.into(),
            resource_sha256: resource.resource_sha256().into(),
            descriptor_sha256: resource.descriptor_sha256().into(),
        });
    }
    Ok(bindings)
}

fn terrain_draw_receipt_digest(receipt: &Xmap04TerrainDrawReceipt) -> String {
    let mut identity = b"hoi4.xmap04.native-terrain-draw.receipt.v1\0".to_vec();
    for value in [
        &receipt.schema,
        &receipt.terrain_frame_sha256,
        &receipt.terrain_program_sha256,
        &receipt.ready_draw_contract_sha256,
        &receipt.map_copy_receipt_set_sha256,
        &receipt.main_scene_resource_identity_sha256,
        &receipt.main_scene_producer_frame_sha256,
        &receipt.main_scene_descriptor_sha256,
        &receipt.point_lights_cb2_sha256,
        &receipt.point_lights_cb2_descriptor_sha256,
    ] {
        identity.extend_from_slice(value.as_bytes());
        identity.push(0);
    }
    identity.extend_from_slice(&receipt.main_scene_allocation_generation.to_le_bytes());
    identity.extend_from_slice(&receipt.draw_count.to_le_bytes());
    for value in &receipt.ordered_scene_cb0_composition_sha256s {
        identity.extend_from_slice(value.as_bytes());
    }
    for binding in &receipt.ordered_dependency_resources {
        identity.extend_from_slice(binding.symbol.as_bytes());
        identity.push(0);
        identity.extend_from_slice(binding.owner_packet.as_bytes());
        identity.push(0);
        identity.extend_from_slice(binding.resource_sha256.as_bytes());
        identity.extend_from_slice(binding.descriptor_sha256.as_bytes());
    }
    Sha256::hex(&identity)
}

fn underwater_draw_receipt_digest(receipt: &Xmap04UnderwaterDrawReceipt) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(receipt.terrain_frame_sha256.as_bytes());
    identity.extend_from_slice(receipt.replay_frame_identity_sha256.as_bytes());
    identity.extend_from_slice(receipt.camera_frame_sha256.as_bytes());
    identity.extend_from_slice(receipt.xmap09_camera_frame_sha256.as_bytes());
    identity.extend_from_slice(receipt.underwater_program_sha256.as_bytes());
    identity.extend_from_slice(receipt.target_resource_identity_sha256.as_bytes());
    identity.extend_from_slice(receipt.target_producer_frame_sha256.as_bytes());
    identity.extend_from_slice(&receipt.target_producer_allocation_generation.to_le_bytes());
    identity.extend_from_slice(
        receipt
            .target_producer_allocation_identity_sha256
            .as_bytes(),
    );
    identity.extend_from_slice(receipt.target_texture_descriptor_sha256.as_bytes());
    identity.extend_from_slice(receipt.target_rtv_descriptor_sha256.as_bytes());
    identity.extend_from_slice(receipt.target_srv_descriptor_sha256.as_bytes());
    identity.extend_from_slice(receipt.target_descriptor_sha256.as_bytes());
    identity.extend_from_slice(receipt.depth_descriptor_sha256.as_bytes());
    identity.extend_from_slice(receipt.scene_cb0_sha256.as_bytes());
    identity.extend_from_slice(receipt.scene_cb0_descriptor_sha256.as_bytes());
    identity.extend_from_slice(receipt.point_lights_cb2_sha256.as_bytes());
    identity.extend_from_slice(receipt.point_lights_cb2_descriptor_sha256.as_bytes());
    for binding in &receipt.ordered_light_resources {
        identity.extend_from_slice(binding.symbol.as_bytes());
        identity.extend_from_slice(binding.owner_packet.as_bytes());
        identity.extend_from_slice(binding.resource_sha256.as_bytes());
        identity.extend_from_slice(binding.descriptor_sha256.as_bytes());
    }
    identity.extend_from_slice(&receipt.draw_count.to_le_bytes());
    Sha256::hex(&identity)
}

#[cfg(windows)]
unsafe fn create_underwater_program(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &GameFileResolver,
    program: &crate::PreparedUnderwaterTerrainProgram,
) -> Result<NativeXmap04UnderwaterProgram> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ffi::CString;
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11InputLayout, ID3D11PixelShader, ID3D11VertexShader, D3D11_INPUT_ELEMENT_DESC,
    };

    program.validate()?;
    let mut vertex = ptr::null_mut::<ID3D11VertexShader>();
    native_d3d11_hr(
        (*device).CreateVertexShader(
            program.vertex_bytecode.as_ptr() as *const _,
            program.vertex_bytecode.len(),
            ptr::null_mut(),
            &mut vertex,
        ),
        "CreateVertexShader(XMAP04 underwater)",
    )?;
    let vertex_shader = D3dPtr::from_raw(vertex, "XMAP04 underwater vertex shader")?;
    let mut pixel = ptr::null_mut::<ID3D11PixelShader>();
    native_d3d11_hr(
        (*device).CreatePixelShader(
            program.pixel_bytecode.as_ptr() as *const _,
            program.pixel_bytecode.len(),
            ptr::null_mut(),
            &mut pixel,
        ),
        "CreatePixelShader(XMAP04 underwater)",
    )?;
    let pixel_shader = D3dPtr::from_raw(pixel, "XMAP04 underwater pixel shader")?;
    let position = CString::new("POSITION")?;
    let texcoord = CString::new("TEXCOORD")?;
    let per_vertex = winapi::um::d3d11::D3D11_INPUT_PER_VERTEX_DATA;
    let elements = [
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: position.as_ptr(),
            SemanticIndex: 0,
            Format: crate::TERRAIN_CHUNK_SLOT0_POSITION_FORMAT as _,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: per_vertex,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: texcoord.as_ptr(),
            SemanticIndex: 0,
            Format: crate::TERRAIN_CHUNK_SLOT1_TEXCOORD_FORMAT as _,
            InputSlot: 1,
            AlignedByteOffset: 0,
            InputSlotClass: per_vertex,
            InstanceDataStepRate: 0,
        },
    ];
    let mut input = ptr::null_mut::<ID3D11InputLayout>();
    native_d3d11_hr(
        (*device).CreateInputLayout(
            elements.as_ptr(),
            elements.len() as u32,
            program.vertex_bytecode.as_ptr() as *const _,
            program.vertex_bytecode.len(),
            &mut input,
        ),
        "CreateInputLayout(XMAP04 underwater)",
    )?;
    let input_layout = D3dPtr::from_raw(input, "XMAP04 underwater input layout")?;

    let mut static_textures = BTreeMap::new();
    let mut samplers = Vec::new();
    for sampler in &program.samplers {
        samplers.push((
            sampler.shader_register,
            create_underwater_sampler(device, sampler)?,
        ));
        if sampler.owner_packet == "XMAP04" {
            let texture = create_underwater_static_texture(device, resolver, sampler)?;
            if static_textures
                .insert(sampler.name.clone(), texture)
                .is_some()
            {
                return Err("XMAP04 underwater static texture is duplicated".into());
            }
        }
    }
    Ok(NativeXmap04UnderwaterProgram {
        deterministic_sha256: program.deterministic_sha256.clone(),
        vertex_shader,
        pixel_shader,
        input_layout,
        static_textures,
        samplers,
        blend_state: create_underwater_blend_state(device)?,
        depth_state: create_underwater_depth_state(device)?,
        rasterizer_state: create_underwater_rasterizer_state(device)?,
    })
}

#[cfg(windows)]
pub(crate) unsafe fn create_underwater_sampler(
    device: *mut winapi::um::d3d11::ID3D11Device,
    sampler: &crate::TerrainSamplerRequirement,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11SamplerState, D3D11_COMPARISON_NEVER, D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT,
        D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_FILTER_MIN_MAG_MIP_POINT,
        D3D11_FILTER_MIN_MAG_POINT_MIP_LINEAR, D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP,
        D3D11_TEXTURE_ADDRESS_MIRROR, D3D11_TEXTURE_ADDRESS_WRAP,
    };
    let filter = match (
        sampler.min_filter.as_str(),
        sampler.mag_filter.as_str(),
        sampler.mip_filter.as_str(),
    ) {
        ("Linear", "Linear", "Linear") => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        ("Linear", "Linear", "Point" | "None") => D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT,
        ("Point", "Point", "Linear") => D3D11_FILTER_MIN_MAG_POINT_MIP_LINEAR,
        ("Point", "Point", "Point" | "None") => D3D11_FILTER_MIN_MAG_MIP_POINT,
        other => {
            return Err(format!(
                "XMAP04 underwater sampler {} has unsupported exact filter tuple {other:?}",
                sampler.name
            )
            .into())
        }
    };
    let address = |value: &str| -> Result<_> {
        match value {
            "Wrap" => Ok(D3D11_TEXTURE_ADDRESS_WRAP),
            "Clamp" => Ok(D3D11_TEXTURE_ADDRESS_CLAMP),
            "Mirror" => Ok(D3D11_TEXTURE_ADDRESS_MIRROR),
            _ => Err(format!("XMAP04 underwater sampler address {value} is unsupported").into()),
        }
    };
    let parse_f32 = |value: &Option<String>, default: f32, label: &str| -> Result<f32> {
        value.as_deref().map_or(Ok(default), |raw| {
            raw.trim_matches('"')
                .parse::<f32>()
                .map_err(|error| format!("XMAP04 underwater sampler {label} {raw}: {error}").into())
        })
    };
    let parse_u32 = |value: &Option<String>, default: u32, label: &str| -> Result<u32> {
        value.as_deref().map_or(Ok(default), |raw| {
            raw.trim_matches('"')
                .parse::<u32>()
                .map_err(|error| format!("XMAP04 underwater sampler {label} {raw}: {error}").into())
        })
    };
    let desc = D3D11_SAMPLER_DESC {
        Filter: filter,
        AddressU: address(&sampler.address_u)?,
        AddressV: address(&sampler.address_v)?,
        AddressW: address(&sampler.address_u)?,
        MipLODBias: parse_f32(&sampler.mip_map_lod_bias, 0.0, "MipLODBias")?,
        MaxAnisotropy: parse_u32(&sampler.max_anisotropy, 1, "MaxAnisotropy")?,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0; 4],
        MinLOD: parse_f32(&sampler.mip_map_min_lod, 0.0, "MinLOD")?,
        MaxLOD: parse_f32(&sampler.mip_map_max_lod, 1024.0, "MaxLOD")?,
    };
    let mut state = ptr::null_mut::<ID3D11SamplerState>();
    native_d3d11_hr(
        (*device).CreateSamplerState(&desc, &mut state),
        &format!("CreateSamplerState(XMAP04 underwater {})", sampler.name),
    )?;
    D3dPtr::from_raw(
        state,
        &format!("XMAP04 underwater {} sampler", sampler.name),
    )
}

#[cfg(windows)]
pub(crate) unsafe fn create_underwater_static_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &GameFileResolver,
    sampler: &crate::TerrainSamplerRequirement,
) -> Result<NativeXmap04UnderwaterTexture> {
    let content = sampler
        .admitted_static_content
        .as_ref()
        .ok_or("XMAP04 underwater static sampler has no admitted content")?;
    let bytes = resolver.read_admitted_used_content(content)?;
    let mut texture = match sampler.name.as_str() {
        "TerrainDiffuse" | "TerrainColorTint" => {
            create_underwater_dds_texture(device, &bytes, &sampler.name)
        }
        "HeightNormal" => create_underwater_world_normal_texture(device, &bytes),
        other => Err(format!("XMAP04 owns no underwater texture uploader for {other}").into()),
    }?;
    texture.resource_sha256 = content.payload_sha256.clone();
    texture.descriptor_sha256 =
        underwater_static_texture_descriptor_sha256(texture._texture.as_raw())?;
    Ok(texture)
}

#[cfg(windows)]
unsafe fn underwater_static_texture_descriptor_sha256(
    texture: *mut winapi::um::d3d11::ID3D11Texture2D,
) -> Result<String> {
    if texture.is_null() {
        return Err("XMAP04 underwater static texture is null".into());
    }
    let mut desc: winapi::um::d3d11::D3D11_TEXTURE2D_DESC = std::mem::zeroed();
    (*texture).GetDesc(&mut desc);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"hoi4.xmap04.underwater-static-texture-desc.v1\0");
    for value in [
        desc.Width,
        desc.Height,
        desc.MipLevels,
        desc.ArraySize,
        desc.Format as u32,
        desc.SampleDesc.Count,
        desc.SampleDesc.Quality,
        desc.Usage as u32,
        desc.BindFlags,
        desc.CPUAccessFlags,
        desc.MiscFlags,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(Sha256::hex(&bytes))
}

#[cfg(windows)]
unsafe fn d3d11_device_child_identity(
    object: *mut winapi::um::d3d11::ID3D11DeviceChild,
    role: &str,
) -> Result<usize> {
    use crate::d3d11_device::D3dPtr;
    use std::ptr;
    use winapi::um::d3d11::ID3D11Device;
    if object.is_null() {
        return Err(format!("XMAP04 {role} is null").into());
    }
    let mut device = ptr::null_mut::<ID3D11Device>();
    (*object).GetDevice(&mut device);
    Ok(D3dPtr::from_raw(device, &format!("XMAP04 {role} owning device"))?.as_raw() as usize)
}

#[cfg(windows)]
unsafe fn create_terrain_static_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &GameFileResolver,
    sampler: &crate::TerrainSamplerRequirement,
) -> Result<NativeXmap04UnderwaterTexture> {
    let content = sampler
        .admitted_static_content
        .as_ref()
        .ok_or("XMAP04 terrain static sampler has no admitted content")?;
    let bytes = resolver.read_admitted_used_content(content)?;
    let mut texture = if sampler.name == "HeightNormal" {
        create_underwater_world_normal_texture(device, &bytes)
    } else {
        create_terrain_dds_texture(device, &bytes, &sampler.name)
    }?;
    texture.resource_sha256 = content.payload_sha256.clone();
    texture.descriptor_sha256 =
        underwater_static_texture_descriptor_sha256(texture._texture.as_raw())?;
    Ok(texture)
}

#[cfg(windows)]
unsafe fn create_terrain_dds_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    symbol: &str,
) -> Result<NativeXmap04UnderwaterTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_IMMUTABLE,
    };

    if bytes.len() < 128 || bytes.get(..4) != Some(b"DDS ") {
        return Err(format!("XMAP04 terrain {symbol} is not a complete DDS").into());
    }
    let read_u32 = |offset: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("XMAP04 terrain DDS header is truncated")?
                .try_into()?,
        ))
    };
    if read_u32(4)? != 124 || read_u32(76)? != 32 {
        return Err("XMAP04 terrain DDS has noncanonical header sizes".into());
    }
    let width = read_u32(16)?;
    let height = read_u32(12)?;
    let mip_levels = validate_underwater_dds_shape(width, height, read_u32(28)?)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or("XMAP04 terrain DDS fourcc is truncated")?;
    let (file_format, block_bytes, bytes_per_pixel, payload_offset) = if fourcc == b"DX10" {
        if bytes.len() < 148 || read_u32(132)? != 3 || read_u32(140)? != 1 {
            return Err("XMAP04 terrain DX10 DDS must be one Texture2D".into());
        }
        let format = read_u32(128)?;
        let block = match format {
            70..=72 | 79..=81 => 8,
            73..=78 | 82..=84 | 97..=99 => 16,
            _ => 0,
        };
        let pixel = match format {
            27..=29 | 87 | 90..=93 => 4,
            _ if block != 0 => 0,
            _ => {
                return Err(
                    format!("XMAP04 terrain {symbol} DX10 format {format} is unsupported").into(),
                )
            }
        };
        (format, block, pixel, 148usize)
    } else {
        match fourcc {
            b"DXT1" => (71, 8, 0, 128),
            b"DXT3" => (74, 16, 0, 128),
            b"DXT5" => (77, 16, 0, 128),
            _ if read_u32(88)? == 32 => {
                let masks = (read_u32(92)?, read_u32(96)?, read_u32(100)?, read_u32(104)?);
                let format = match masks {
                    (0x000000ff, 0x0000ff00, 0x00ff0000, 0xff000000) => 28,
                    (0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000) => 87,
                    _ => {
                        return Err(format!(
                            "XMAP04 terrain {symbol} DDS has unsupported RGBA masks {masks:?}"
                        )
                        .into())
                    }
                };
                (format, 0, 4, 128)
            }
            _ => {
                return Err(format!(
                    "XMAP04 terrain {symbol} DDS encoding {:?} is unsupported",
                    String::from_utf8_lossy(fourcc)
                )
                .into())
            }
        }
    };
    let format = terrain_registered_format(symbol, file_format)?;
    let mut offset = payload_offset;
    let mut initial = Vec::with_capacity(usize::try_from(mip_levels)?);
    for mip in 0..mip_levels {
        let mip_width = (width >> mip).max(1);
        let mip_height = (height >> mip).max(1);
        let (row_pitch, slice_pitch) = if block_bytes != 0 {
            let row = mip_width
                .div_ceil(4)
                .max(1)
                .checked_mul(block_bytes)
                .ok_or("XMAP04 terrain DDS row pitch overflow")?;
            (
                row,
                row.checked_mul(mip_height.div_ceil(4).max(1))
                    .ok_or("XMAP04 terrain DDS slice pitch overflow")?,
            )
        } else {
            let row = mip_width
                .checked_mul(bytes_per_pixel)
                .ok_or("XMAP04 terrain DDS row pitch overflow")?;
            (
                row,
                row.checked_mul(mip_height)
                    .ok_or("XMAP04 terrain DDS slice pitch overflow")?,
            )
        };
        let end = offset
            .checked_add(usize::try_from(slice_pitch)?)
            .ok_or("XMAP04 terrain DDS mip range overflow")?;
        let payload = bytes
            .get(offset..end)
            .ok_or("XMAP04 terrain DDS mip payload is truncated")?;
        initial.push(D3D11_SUBRESOURCE_DATA {
            pSysMem: payload.as_ptr() as *const _,
            SysMemPitch: row_pitch,
            SysMemSlicePitch: slice_pitch,
        });
        offset = end;
    }
    if offset != bytes.len() {
        return Err(format!(
            "XMAP04 terrain {symbol} DDS has {} trailing bytes outside its mip chain",
            bytes.len() - offset
        )
        .into());
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: mip_levels,
        ArraySize: 1,
        Format: format as _,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut raw_texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, initial.as_ptr(), &mut raw_texture),
        &format!("CreateTexture2D(XMAP04 terrain {symbol})"),
    )?;
    let texture = D3dPtr::from_raw(raw_texture, &format!("XMAP04 terrain {symbol} texture"))?;
    let mut raw_view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut raw_view),
        &format!("CreateShaderResourceView(XMAP04 terrain {symbol})"),
    )?;
    Ok(NativeXmap04UnderwaterTexture {
        _texture: texture,
        view: D3dPtr::from_raw(raw_view, &format!("XMAP04 terrain {symbol} SRV"))?,
        resource_sha256: String::new(),
        descriptor_sha256: String::new(),
    })
}

#[cfg(windows)]
fn terrain_registered_format(symbol: &str, file_format: u32) -> Result<u32> {
    let srgb = matches!(
        symbol,
        "TerrainDiffuse" | "TerrainColorTint" | "CityLightsAndSnowNoise" | "MudDiffuseGloss"
    );
    let linear = match file_format {
        29 => 28,
        72 => 71,
        75 => 74,
        78 => 77,
        91 => 87,
        93 => 88,
        99 => 98,
        other => other,
    };
    if !srgb {
        return Ok(linear);
    }
    match linear {
        28 => Ok(29),
        71 => Ok(72),
        74 => Ok(75),
        77 => Ok(78),
        87 => Ok(91),
        88 => Ok(93),
        98 => Ok(99),
        other => Err(format!(
            "XMAP04 terrain {symbol} format {other} has no exact sRGB view family"
        )
        .into()),
    }
}

#[cfg(windows)]
unsafe fn create_underwater_dds_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    role: &str,
) -> Result<NativeXmap04UnderwaterTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgiformat::{DXGI_FORMAT_BC1_UNORM_SRGB, DXGI_FORMAT_BC3_UNORM_SRGB};
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_IMMUTABLE,
    };

    if bytes.len() < 128 || bytes.get(..4) != Some(b"DDS ") {
        return Err(format!("XMAP04 underwater {role} is not a complete legacy DDS").into());
    }
    let read_u32 = |offset: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("XMAP04 underwater DDS header is truncated")?
                .try_into()?,
        ))
    };
    if read_u32(4)? != 124 || read_u32(76)? != 32 {
        return Err("XMAP04 underwater DDS has noncanonical header sizes".into());
    }
    let width = read_u32(16)?;
    let height = read_u32(12)?;
    let mip_levels = validate_underwater_dds_shape(width, height, read_u32(28)?)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or("XMAP04 underwater DDS fourcc is truncated")?;
    let (format, block_bytes) = match fourcc {
        b"DXT1" => (DXGI_FORMAT_BC1_UNORM_SRGB, 8u32),
        b"DXT5" => (DXGI_FORMAT_BC3_UNORM_SRGB, 16u32),
        _ => {
            return Err(format!(
                "XMAP04 underwater {role} DDS encoding {:?} is unsupported",
                String::from_utf8_lossy(fourcc)
            )
            .into())
        }
    };
    let mut offset = 128usize;
    let mut initial = Vec::with_capacity(usize::try_from(mip_levels)?);
    for mip in 0..mip_levels {
        let mip_width = (width >> mip).max(1);
        let mip_height = (height >> mip).max(1);
        let blocks_w = mip_width.div_ceil(4).max(1);
        let blocks_h = mip_height.div_ceil(4).max(1);
        let row_pitch = blocks_w
            .checked_mul(block_bytes)
            .ok_or("XMAP04 underwater DDS row pitch overflow")?;
        let slice_pitch = row_pitch
            .checked_mul(blocks_h)
            .ok_or("XMAP04 underwater DDS slice pitch overflow")?;
        let end = offset
            .checked_add(slice_pitch as usize)
            .ok_or("XMAP04 underwater DDS mip range overflow")?;
        let payload = bytes
            .get(offset..end)
            .ok_or("XMAP04 underwater DDS mip payload is truncated")?;
        initial.push(D3D11_SUBRESOURCE_DATA {
            pSysMem: payload.as_ptr() as *const _,
            SysMemPitch: row_pitch,
            SysMemSlicePitch: slice_pitch,
        });
        offset = end;
    }
    if offset != bytes.len() {
        return Err(format!(
            "XMAP04 underwater {role} DDS has {} trailing bytes outside its mip chain",
            bytes.len() - offset
        )
        .into());
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: mip_levels,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, initial.as_ptr(), &mut texture),
        &format!("CreateTexture2D(XMAP04 underwater {role})"),
    )?;
    let texture = D3dPtr::from_raw(texture, &format!("XMAP04 underwater {role} texture"))?;
    let mut view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        &format!("CreateShaderResourceView(XMAP04 underwater {role})"),
    )?;
    Ok(NativeXmap04UnderwaterTexture {
        _texture: texture,
        view: D3dPtr::from_raw(view, &format!("XMAP04 underwater {role} SRV"))?,
        resource_sha256: String::new(),
        descriptor_sha256: String::new(),
    })
}

#[cfg(windows)]
fn decode_world_normal_bmp(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    if bytes.len() < 54 || bytes.get(..2) != Some(b"BM") {
        return Err("XMAP04 HeightNormal is not a complete BMP".into());
    }
    let read_u32 = |offset: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("XMAP04 HeightNormal BMP header is truncated")?
                .try_into()?,
        ))
    };
    let read_i32 = |offset: usize| -> Result<i32> {
        Ok(i32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("XMAP04 HeightNormal BMP header is truncated")?
                .try_into()?,
        ))
    };
    let pixel_offset = usize::try_from(read_u32(10)?)?;
    let width_i32 = read_i32(18)?;
    let height_i32 = read_i32(22)?;
    let bit_count = u16::from_le_bytes(
        bytes
            .get(28..30)
            .ok_or("XMAP04 HeightNormal BMP bit count is truncated")?
            .try_into()?,
    );
    let compression = read_u32(30)?;
    if width_i32 <= 0 || height_i32 == 0 || bit_count != 24 || compression != 0 {
        return Err("XMAP04 HeightNormal must be an uncompressed 24-bit 2D BMP".into());
    }
    let width = width_i32 as u32;
    let height = height_i32.unsigned_abs();
    if width > XMAP04_D3D11_MAX_TEXTURE_2D_DIMENSION
        || height > XMAP04_D3D11_MAX_TEXTURE_2D_DIMENSION
    {
        return Err("XMAP04 HeightNormal exceeds D3D11 Texture2D dimensions".into());
    }
    let source_row_pitch = width
        .checked_mul(3)
        .and_then(|value| value.checked_add(3))
        .map(|value| value & !3)
        .ok_or("XMAP04 HeightNormal BMP row pitch overflow")?;
    let source_len = source_row_pitch
        .checked_mul(height)
        .ok_or("XMAP04 HeightNormal BMP payload length overflow")? as usize;
    let source_end = pixel_offset
        .checked_add(source_len)
        .ok_or("XMAP04 HeightNormal BMP payload range overflow")?;
    let source = bytes
        .get(pixel_offset..source_end)
        .ok_or("XMAP04 HeightNormal BMP pixel payload is truncated")?;
    let row_pitch = width
        .checked_mul(4)
        .ok_or("XMAP04 HeightNormal row pitch overflow")?;
    let mut bgra = vec![
        0u8;
        usize::try_from(
            row_pitch
                .checked_mul(height)
                .ok_or("XMAP04 HeightNormal byte length overflow")?
        )?
    ];
    let top_down = height_i32 < 0;
    for output_y in 0..height {
        let source_y = if top_down {
            output_y
        } else {
            height - 1 - output_y
        };
        let source_row = &source[(source_y * source_row_pitch) as usize
            ..(source_y * source_row_pitch + width * 3) as usize];
        let output_row =
            &mut bgra[(output_y * row_pitch) as usize..((output_y + 1) * row_pitch) as usize];
        for x in 0..width as usize {
            output_row[x * 4..x * 4 + 3].copy_from_slice(&source_row[x * 3..x * 3 + 3]);
            output_row[x * 4 + 3] = 0xff;
        }
    }
    Ok((width, height, bgra))
}

#[cfg(windows)]
unsafe fn create_underwater_world_normal_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
) -> Result<NativeXmap04UnderwaterTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8X8_UNORM;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_IMMUTABLE,
    };

    let (width, height, bgra) = decode_world_normal_bmp(bytes)?;
    let row_pitch = width
        .checked_mul(4)
        .ok_or("XMAP04 underwater HeightNormal row pitch overflow")?;
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: bgra.as_ptr() as *const _,
        SysMemPitch: row_pitch,
        SysMemSlicePitch: u32::try_from(bgra.len())?,
    };
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8X8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, &init, &mut texture),
        "CreateTexture2D(XMAP04 underwater HeightNormal)",
    )?;
    let texture = D3dPtr::from_raw(texture, "XMAP04 underwater HeightNormal texture")?;
    let mut view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        "CreateShaderResourceView(XMAP04 underwater HeightNormal)",
    )?;
    Ok(NativeXmap04UnderwaterTexture {
        _texture: texture,
        view: D3dPtr::from_raw(view, "XMAP04 underwater HeightNormal SRV")?,
        resource_sha256: String::new(),
        descriptor_sha256: String::new(),
    })
}

#[cfg(windows)]
unsafe fn create_underwater_blend_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::{mem::zeroed, ptr};
    use winapi::um::d3d11::{ID3D11BlendState, D3D11_BLEND_DESC, D3D11_COLOR_WRITE_ENABLE_ALL};
    let mut desc: D3D11_BLEND_DESC = zeroed();
    desc.AlphaToCoverageEnable = 0;
    desc.IndependentBlendEnable = 0;
    desc.RenderTarget[0].BlendEnable = 0;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL as u8;
    let mut state = ptr::null_mut::<ID3D11BlendState>();
    native_d3d11_hr(
        (*device).CreateBlendState(&desc, &mut state),
        "CreateBlendState(XMAP04 underwater no-blend RGBA)",
    )?;
    D3dPtr::from_raw(state, "XMAP04 underwater blend state")
}

#[cfg(windows)]
unsafe fn create_underwater_depth_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::{mem::zeroed, ptr};
    use winapi::um::d3d11::{
        ID3D11DepthStencilState, D3D11_COMPARISON_LESS, D3D11_DEPTH_STENCIL_DESC,
        D3D11_DEPTH_WRITE_MASK_ALL,
    };
    let mut desc: D3D11_DEPTH_STENCIL_DESC = zeroed();
    desc.DepthEnable = 1;
    desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
    desc.DepthFunc = D3D11_COMPARISON_LESS;
    desc.StencilEnable = 0;
    let mut state = ptr::null_mut::<ID3D11DepthStencilState>();
    native_d3d11_hr(
        (*device).CreateDepthStencilState(&desc, &mut state),
        "CreateDepthStencilState(XMAP04 underwater)",
    )?;
    D3dPtr::from_raw(state, "XMAP04 underwater depth state")
}

#[cfg(windows)]
unsafe fn create_terrain_depth_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::{mem::zeroed, ptr};
    use winapi::um::d3d11::{
        ID3D11DepthStencilState, D3D11_COMPARISON_ALWAYS, D3D11_COMPARISON_LESS_EQUAL,
        D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ALL, D3D11_STENCIL_OP_KEEP,
    };
    let mut desc: D3D11_DEPTH_STENCIL_DESC = zeroed();
    desc.DepthEnable = 1;
    desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
    desc.DepthFunc = D3D11_COMPARISON_LESS_EQUAL;
    desc.StencilEnable = 0;
    desc.StencilReadMask = 0xff;
    desc.StencilWriteMask = 0xff;
    desc.FrontFace.StencilFailOp = D3D11_STENCIL_OP_KEEP;
    desc.FrontFace.StencilDepthFailOp = D3D11_STENCIL_OP_KEEP;
    desc.FrontFace.StencilPassOp = D3D11_STENCIL_OP_KEEP;
    desc.FrontFace.StencilFunc = D3D11_COMPARISON_ALWAYS;
    desc.BackFace = desc.FrontFace;
    let mut state = ptr::null_mut::<ID3D11DepthStencilState>();
    native_d3d11_hr(
        (*device).CreateDepthStencilState(&desc, &mut state),
        "CreateDepthStencilState(XMAP04 terrain exact-a729)",
    )?;
    D3dPtr::from_raw(state, "XMAP04 terrain depth state")
}

#[cfg(windows)]
unsafe fn create_underwater_rasterizer_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11RasterizerState, D3D11_CULL_BACK, D3D11_FILL_SOLID, D3D11_RASTERIZER_DESC,
    };
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_BACK,
        FrontCounterClockwise: 0,
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: 1,
        ScissorEnable: 0,
        MultisampleEnable: 0,
        AntialiasedLineEnable: 0,
    };
    let mut state = ptr::null_mut::<ID3D11RasterizerState>();
    native_d3d11_hr(
        (*device).CreateRasterizerState(&desc, &mut state),
        "CreateRasterizerState(XMAP04 underwater)",
    )?;
    D3dPtr::from_raw(state, "XMAP04 underwater rasterizer state")
}

#[cfg(windows)]
unsafe fn create_terrain_rasterizer_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11RasterizerState, D3D11_CULL_BACK, D3D11_FILL_SOLID, D3D11_RASTERIZER_DESC,
    };
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_BACK,
        FrontCounterClockwise: 1,
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: 1,
        ScissorEnable: 0,
        MultisampleEnable: 0,
        AntialiasedLineEnable: 0,
    };
    let mut state = ptr::null_mut::<ID3D11RasterizerState>();
    native_d3d11_hr(
        (*device).CreateRasterizerState(&desc, &mut state),
        "CreateRasterizerState(XMAP04 terrain exact-a729)",
    )?;
    D3dPtr::from_raw(state, "XMAP04 terrain rasterizer state")
}

#[cfg(windows)]
unsafe fn create_underwater_depth(
    device: *mut winapi::um::d3d11::ID3D11Device,
    width: u32,
    height: u32,
    dxgi_format: u32,
    sample_count: u32,
    sample_quality: u32,
    mip_levels: u32,
    array_size: u32,
    has_stencil: bool,
    descriptor_sha256: &str,
) -> Result<NativeXmap04UnderwaterDepth> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11DepthStencilView, ID3D11Texture2D, D3D11_BIND_DEPTH_STENCIL, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT,
    };
    let expected_stencil = matches!(dxgi_format, 20 | 45);
    if width == 0
        || height == 0
        || sample_count == 0
        || mip_levels != 1
        || array_size != 1
        || !matches!(dxgi_format, 20 | 40 | 45 | 55)
        || has_stencil != expected_stencil
        || !canonical_sha256(descriptor_sha256)
    {
        return Err(
            "XMAP04 underwater cloned depth descriptor is unsupported or inconsistent".into(),
        );
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: mip_levels,
        ArraySize: array_size,
        Format: dxgi_format as _,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: sample_count,
            Quality: sample_quality,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_DEPTH_STENCIL,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, ptr::null(), &mut texture),
        "CreateTexture2D(XMAP04 underwater depth)",
    )?;
    let texture = D3dPtr::from_raw(texture, "XMAP04 underwater depth texture")?;
    let mut view = ptr::null_mut::<ID3D11DepthStencilView>();
    native_d3d11_hr(
        (*device).CreateDepthStencilView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        "CreateDepthStencilView(XMAP04 underwater)",
    )?;
    Ok(NativeXmap04UnderwaterDepth {
        width,
        height,
        descriptor_sha256: descriptor_sha256.into(),
        has_stencil,
        _texture: texture,
        view: D3dPtr::from_raw(view, "XMAP04 underwater DSV")?,
    })
}

#[cfg(windows)]
unsafe fn create_buffer(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    bind_flags: u32,
    role: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11Buffer, D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DEFAULT,
    };
    if bytes.is_empty() {
        return Err(format!("{role} is empty").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind_flags,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer = ptr::null_mut::<ID3D11Buffer>();
    native_d3d11_hr(
        (*device).CreateBuffer(&desc, &initial, &mut buffer),
        &format!("CreateBuffer({role})"),
    )?;
    D3dPtr::from_raw(buffer, role)
}

#[cfg(windows)]
unsafe fn upload_draws(
    device: *mut winapi::um::d3d11::ID3D11Device,
    frame: &PreparedTerrainFrame,
) -> Result<Vec<NativeXmap04DrawUpload>> {
    let mut uploads = Vec::with_capacity(frame.draws.len());
    for (index, draw) in frame.draws.iter().enumerate() {
        uploads.push(NativeXmap04DrawUpload {
            slot1: create_buffer(
                device,
                &draw.slot1,
                winapi::um::d3d11::D3D11_BIND_VERTEX_BUFFER,
                &format!("XMAP04 draw {index} slot1"),
            )?,
            cb1: create_buffer(
                device,
                &draw.cb1,
                winapi::um::d3d11::D3D11_BIND_CONSTANT_BUFFER,
                &format!("XMAP04 draw {index} CB1"),
            )?,
        });
    }
    Ok(uploads)
}

#[cfg(windows)]
unsafe fn create_map_shader_textures(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &GameFileResolver,
    frame: &PreparedTerrainFrame,
    generated_terrain_id: (
        &[u8],
        &crate::production_features::xmap04::TerrainGeneratedResourceDescriptor,
    ),
) -> Result<BTreeMap<String, NativeXmap04MapShaderTexture>> {
    let mut textures = BTreeMap::new();
    let (terrain_id_payload, terrain_id_descriptor) = generated_terrain_id;
    let (terrain_id_texture, terrain_id_view) =
        create_generated_texture(device, generated_terrain_id)?;
    let terrain_id_binding = Xmap04NativeMapShaderResourceBinding {
        symbol: "TerrainIDMap".into(),
        owner_packet: "XMAP04".into(),
        shader_register: 3,
        replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
        camera_frame_sha256: frame.camera_frame_sha256.clone(),
        source_content_sha256: terrain_id_descriptor.payload_sha256.clone(),
        payload_sha256: Sha256::hex(terrain_id_payload),
        descriptor_sha256: terrain_id_descriptor.descriptor_sha256.clone(),
    };
    terrain_id_binding.validate()?;
    textures.insert(
        "TerrainIDMap".into(),
        NativeXmap04MapShaderTexture {
            _texture: terrain_id_texture,
            view: terrain_id_view,
            binding: terrain_id_binding,
        },
    );

    let heightmap_content = admitted_heightmap_content(frame)?;
    let heightmap_bytes = resolver.read_admitted_used_content(heightmap_content)?;
    let heightmap = create_indexed_heightmap_texture(
        device,
        &heightmap_bytes,
        &frame.replay_frame_identity_sha256,
        &frame.camera_frame_sha256,
        &heightmap_content.payload_sha256,
    )?;
    textures.insert("HeightMap".into(), heightmap);

    for (symbol, shader_register) in [("TerrainNormal", 5), ("HeightNormal", 6)] {
        let sampler = frame
            .program
            .samplers
            .iter()
            .find(|sampler| sampler.name == symbol && sampler.owner_packet == "XMAP04")
            .ok_or_else(|| {
                format!("XMAP04 prepared terrain program omitted owned sampler {symbol}")
            })?;
        let content = sampler.admitted_static_content.as_ref().ok_or_else(|| {
            format!("XMAP04 prepared terrain sampler {symbol} omitted admitted content")
        })?;
        let bytes = resolver.read_admitted_used_content(content)?;
        let texture = match symbol {
            "TerrainNormal" => create_terrain_normal_texture(
                device,
                &bytes,
                &frame.replay_frame_identity_sha256,
                &frame.camera_frame_sha256,
                &content.payload_sha256,
                shader_register,
            )?,
            "HeightNormal" => create_height_normal_map_shader_texture(
                device,
                &bytes,
                &frame.replay_frame_identity_sha256,
                &frame.camera_frame_sha256,
                &content.payload_sha256,
                shader_register,
            )?,
            _ => unreachable!("fixed XMAP04 map shader resource set"),
        };
        textures.insert(symbol.into(), texture);
    }
    if textures.len() != XMAP04_MAP_SHADER_RESOURCE_SET.len() {
        return Err("XMAP04 failed to materialize its complete map shader resource set".into());
    }
    Ok(textures)
}

#[cfg(windows)]
unsafe fn create_indexed_heightmap_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    replay_frame_identity_sha256: &str,
    camera_frame_sha256: &str,
    source_content_sha256: &str,
) -> Result<NativeXmap04MapShaderTexture> {
    let decoded =
        crate::image_load::decode_bmp_indexed(bytes, std::path::Path::new("map/heightmap.bmp"))?;
    let width = u32::try_from(decoded.width)?;
    let height = u32::try_from(decoded.height)?;
    create_single_mip_map_shader_texture(
        device,
        "HeightMap",
        4,
        width,
        height,
        winapi::shared::dxgiformat::DXGI_FORMAT_R8_UNORM as u32,
        &decoded.pixels,
        width,
        replay_frame_identity_sha256,
        camera_frame_sha256,
        source_content_sha256,
    )
}

#[cfg(windows)]
unsafe fn create_height_normal_map_shader_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    replay_frame_identity_sha256: &str,
    camera_frame_sha256: &str,
    source_content_sha256: &str,
    shader_register: u32,
) -> Result<NativeXmap04MapShaderTexture> {
    let (width, height, bgra) = decode_world_normal_bmp(bytes)?;
    create_single_mip_map_shader_texture(
        device,
        "HeightNormal",
        shader_register,
        width,
        height,
        winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8X8_UNORM as u32,
        &bgra,
        width
            .checked_mul(4)
            .ok_or("XMAP04 HeightNormal row pitch overflow")?,
        replay_frame_identity_sha256,
        camera_frame_sha256,
        source_content_sha256,
    )
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
unsafe fn create_single_mip_map_shader_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    symbol: &str,
    shader_register: u32,
    width: u32,
    height: u32,
    dxgi_format: u32,
    payload: &[u8],
    row_pitch: u32,
    replay_frame_identity_sha256: &str,
    camera_frame_sha256: &str,
    source_content_sha256: &str,
) -> Result<NativeXmap04MapShaderTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_IMMUTABLE,
    };

    let slice_pitch = row_pitch
        .checked_mul(height)
        .ok_or("XMAP04 map shader resource slice pitch overflow")?;
    if payload.len() != usize::try_from(slice_pitch)? {
        return Err(format!("XMAP04 {symbol} decoded payload length is inconsistent").into());
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: dxgi_format as _,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: payload.as_ptr() as *const _,
        SysMemPitch: row_pitch,
        SysMemSlicePitch: slice_pitch,
    };
    let mut raw_texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, &initial, &mut raw_texture),
        &format!("CreateTexture2D(XMAP04 {symbol})"),
    )?;
    let texture = D3dPtr::from_raw(raw_texture, &format!("XMAP04 {symbol} texture"))?;
    let mut raw_view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(raw_texture as *mut _, ptr::null(), &mut raw_view),
        &format!("CreateShaderResourceView(XMAP04 {symbol})"),
    )?;
    let binding = Xmap04NativeMapShaderResourceBinding {
        symbol: symbol.into(),
        owner_packet: "XMAP04".into(),
        shader_register,
        replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
        camera_frame_sha256: camera_frame_sha256.into(),
        source_content_sha256: source_content_sha256.into(),
        payload_sha256: Sha256::hex(payload),
        descriptor_sha256: map_shader_resource_descriptor_sha256(
            symbol,
            width,
            height,
            1,
            dxgi_format,
            D3D11_USAGE_IMMUTABLE,
            &[(row_pitch, slice_pitch)],
        )?,
    };
    binding.validate()?;
    Ok(NativeXmap04MapShaderTexture {
        _texture: texture,
        view: D3dPtr::from_raw(raw_view, &format!("XMAP04 {symbol} SRV"))?,
        binding,
    })
}

#[cfg(windows)]
unsafe fn create_terrain_normal_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    replay_frame_identity_sha256: &str,
    camera_frame_sha256: &str,
    source_content_sha256: &str,
    shader_register: u32,
) -> Result<NativeXmap04MapShaderTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_IMMUTABLE,
    };
    if bytes.len() < 128 || bytes.get(..4) != Some(b"DDS ") {
        return Err("XMAP04 TerrainNormal must be a complete DDS".into());
    }
    let read = |offset: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("XMAP04 TerrainNormal DDS header is truncated")?
                .try_into()?,
        ))
    };
    if read(4)? != 124 || read(76)? != 32 {
        return Err("XMAP04 TerrainNormal DDS has noncanonical header sizes".into());
    }
    let width = read(16)?;
    let height = read(12)?;
    let mip_levels = validate_underwater_dds_shape(width, height, read(28)?)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or("XMAP04 TerrainNormal DDS pixel format is truncated")?;
    let (dxgi_format, block_bytes, bytes_per_pixel, payload_offset) = match fourcc {
        b"DXT1" => (71u32, 8u32, 0u32, 128usize),
        b"DXT3" => (74u32, 16u32, 0u32, 128usize),
        b"DXT5" => (77u32, 16u32, 0u32, 128usize),
        b"DX10" => {
            if bytes.len() < 148 {
                return Err("XMAP04 TerrainNormal DX10 header is truncated".into());
            }
            let format = read(128)?;
            let dimension = read(132)?;
            let misc_flags = read(136)?;
            let array_size = read(140)?;
            if dimension != 3 || misc_flags & 0x4 != 0 || array_size != 1 {
                return Err("XMAP04 TerrainNormal DX10 DDS must be one non-cube Texture2D".into());
            }
            match format {
                71 | 72 => (format, 8, 0, 148),
                74 | 75 | 77 | 78 => (format, 16, 0, 148),
                28 | 29 | 87 | 88 | 91 => (format, 0, 4, 148),
                other => {
                    return Err(format!(
                        "XMAP04 TerrainNormal DX10 format {other} has no exact uploader"
                    )
                    .into())
                }
            }
        }
        [0, 0, 0, 0] => {
            let rgb_bits = read(88)?;
            let masks = [read(92)?, read(96)?, read(100)?, read(104)?];
            let format = match (rgb_bits, masks) {
                (32, [0x000000ff, 0x0000ff00, 0x00ff0000, 0xff000000]) => 28,
                (32, [0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000]) => 87,
                (32, [0x00ff0000, 0x0000ff00, 0x000000ff, 0]) => 88,
                _ => {
                    return Err(
                        "XMAP04 TerrainNormal uncompressed DDS channel layout is unsupported"
                            .into(),
                    )
                }
            };
            (format, 0, 4, 128)
        }
        other => {
            return Err(format!(
                "XMAP04 TerrainNormal DDS encoding {:?} has no exact uploader",
                String::from_utf8_lossy(other)
            )
            .into())
        }
    };
    let mut offset = payload_offset;
    let mut initial = Vec::with_capacity(usize::try_from(mip_levels)?);
    let mut pitches = Vec::with_capacity(usize::try_from(mip_levels)?);
    for mip in 0..mip_levels {
        let mip_width = (width >> mip).max(1);
        let mip_height = (height >> mip).max(1);
        let (row_pitch, slice_pitch) = if block_bytes != 0 {
            let row_pitch = mip_width
                .div_ceil(4)
                .max(1)
                .checked_mul(block_bytes)
                .ok_or("XMAP04 TerrainNormal block row pitch overflow")?;
            let slice_pitch = row_pitch
                .checked_mul(mip_height.div_ceil(4).max(1))
                .ok_or("XMAP04 TerrainNormal block slice pitch overflow")?;
            (row_pitch, slice_pitch)
        } else {
            let row_pitch = mip_width
                .checked_mul(bytes_per_pixel)
                .ok_or("XMAP04 TerrainNormal row pitch overflow")?;
            let slice_pitch = row_pitch
                .checked_mul(mip_height)
                .ok_or("XMAP04 TerrainNormal slice pitch overflow")?;
            (row_pitch, slice_pitch)
        };
        let end = offset
            .checked_add(usize::try_from(slice_pitch)?)
            .ok_or("XMAP04 TerrainNormal mip range overflow")?;
        let payload = bytes
            .get(offset..end)
            .ok_or("XMAP04 TerrainNormal mip payload is truncated")?;
        initial.push(D3D11_SUBRESOURCE_DATA {
            pSysMem: payload.as_ptr() as *const _,
            SysMemPitch: row_pitch,
            SysMemSlicePitch: slice_pitch,
        });
        pitches.push((row_pitch, slice_pitch));
        offset = end;
    }
    if offset != bytes.len() {
        return Err("XMAP04 TerrainNormal DDS has bytes outside its declared mip chain".into());
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: mip_levels,
        ArraySize: 1,
        Format: dxgi_format as _,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut raw_texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&desc, initial.as_ptr(), &mut raw_texture),
        "CreateTexture2D(XMAP04 TerrainNormal)",
    )?;
    let texture = D3dPtr::from_raw(raw_texture, "XMAP04 TerrainNormal texture")?;
    let mut raw_view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(raw_texture as *mut _, ptr::null(), &mut raw_view),
        "CreateShaderResourceView(XMAP04 TerrainNormal)",
    )?;
    let binding = Xmap04NativeMapShaderResourceBinding {
        symbol: "TerrainNormal".into(),
        owner_packet: "XMAP04".into(),
        shader_register,
        replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
        camera_frame_sha256: camera_frame_sha256.into(),
        source_content_sha256: source_content_sha256.into(),
        payload_sha256: Sha256::hex(
            bytes
                .get(payload_offset..offset)
                .ok_or("XMAP04 TerrainNormal payload range is invalid")?,
        ),
        descriptor_sha256: map_shader_resource_descriptor_sha256(
            "TerrainNormal",
            width,
            height,
            mip_levels,
            dxgi_format,
            D3D11_USAGE_IMMUTABLE,
            &pitches,
        )?,
    };
    binding.validate()?;
    Ok(NativeXmap04MapShaderTexture {
        _texture: texture,
        view: D3dPtr::from_raw(raw_view, "XMAP04 TerrainNormal SRV")?,
        binding,
    })
}

#[cfg(windows)]
unsafe fn create_generated_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    (payload, descriptor): (
        &[u8],
        &crate::production_features::xmap04::TerrainGeneratedResourceDescriptor,
    ),
) -> Result<(
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
)> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    let expected = usize::try_from(descriptor.row_pitch)?
        .checked_mul(usize::try_from(descriptor.height)?)
        .ok_or("XMAP04 generated texture byte length overflow")?;
    if payload.len() != expected {
        return Err("XMAP04 generated texture payload/descriptor length mismatch".into());
    }
    let native = D3D11_TEXTURE2D_DESC {
        Width: descriptor.width,
        Height: descriptor.height,
        MipLevels: descriptor.mip_levels,
        ArraySize: 1,
        Format: descriptor.dxgi_format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: payload.as_ptr() as *const _,
        SysMemPitch: descriptor.row_pitch,
        SysMemSlicePitch: u32::try_from(payload.len())?,
    };
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&native, &initial, &mut texture),
        "CreateTexture2D(XMAP04 generated TerrainIDMap)",
    )?;
    let texture = D3dPtr::from_raw(texture, "XMAP04 generated TerrainIDMap texture")?;
    let mut view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        "CreateShaderResourceView(XMAP04 generated TerrainIDMap)",
    )?;
    let view = D3dPtr::from_raw(view, "XMAP04 generated TerrainIDMap SRV")?;
    Ok((texture, view))
}

fn handoff_receipt(
    frame: &PreparedTerrainFrame,
    draw_upload_count: usize,
    static_resource_count: usize,
) -> Result<Xmap04D3dHandoffReceipt> {
    let blocked_visible_draw_packets = frame.program.required_downstream_packets.clone();
    let underwater_static_count = frame
        .underwater_program
        .as_ref()
        .map(|program| {
            program
                .samplers
                .iter()
                .filter(|sampler| sampler.owner_packet == "XMAP04")
                .count()
        })
        .unwrap_or(0);
    let mut receipt = Xmap04D3dHandoffReceipt {
        terrain_frame_sha256: frame.deterministic_sha256.clone(),
        terrain_program_sha256: frame.program.deterministic_sha256.clone(),
        map_copy_receipt_set_sha256: frame.map_copy_receipt_set_sha256()?,
        compiled_programs: if frame.underwater_program.is_some() {
            4
        } else {
            2
        },
        uploaded_shared_buffers: 2,
        uploaded_draw_buffers: u32::try_from(draw_upload_count.saturating_mul(2))
            .unwrap_or(u32::MAX),
        retained_static_resources: u32::try_from(
            static_resource_count.saturating_add(underwater_static_count),
        )
        .unwrap_or(u32::MAX),
        uploaded_generated_resources: 1,
        bound_downstream_resources: Vec::new(),
        blocked_visible_draw_packets,
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = handoff_receipt_digest(&receipt);
    Ok(receipt)
}

fn handoff_receipt_digest(receipt: &Xmap04D3dHandoffReceipt) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(receipt.terrain_frame_sha256.as_bytes());
    identity.extend_from_slice(receipt.terrain_program_sha256.as_bytes());
    identity.extend_from_slice(receipt.map_copy_receipt_set_sha256.as_bytes());
    identity.extend_from_slice(&receipt.compiled_programs.to_le_bytes());
    identity.extend_from_slice(&receipt.uploaded_shared_buffers.to_le_bytes());
    identity.extend_from_slice(&receipt.uploaded_draw_buffers.to_le_bytes());
    identity.extend_from_slice(&receipt.retained_static_resources.to_le_bytes());
    identity.extend_from_slice(&receipt.uploaded_generated_resources.to_le_bytes());
    for binding in &receipt.bound_downstream_resources {
        identity.extend_from_slice(binding.symbol.as_bytes());
        identity.extend_from_slice(binding.owner_packet.as_bytes());
        identity.extend_from_slice(binding.resource_sha256.as_bytes());
        identity.extend_from_slice(binding.descriptor_sha256.as_bytes());
    }
    for packet in &receipt.blocked_visible_draw_packets {
        identity.extend_from_slice(packet.as_bytes());
    }
    Sha256::hex(&identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_shader_binding_requires_exact_owner_symbol_register_and_frame_identities() {
        let binding = Xmap04NativeMapShaderResourceBinding {
            symbol: "HeightMap".into(),
            owner_packet: "XMAP04".into(),
            shader_register: 4,
            replay_frame_identity_sha256: "1".repeat(64),
            camera_frame_sha256: "2".repeat(64),
            source_content_sha256: "3".repeat(64),
            payload_sha256: "4".repeat(64),
            descriptor_sha256: "5".repeat(64),
        };
        binding.validate().expect("exact binding");
        let mut wrong_register = binding.clone();
        wrong_register.shader_register = 5;
        assert!(wrong_register.validate().is_err());
        let mut missing_source_identity = binding;
        missing_source_identity.source_content_sha256.clear();
        assert!(missing_source_identity.validate().is_err());
    }

    #[test]
    fn map_shader_descriptor_seals_real_shape_format_usage_and_pitches() {
        let descriptor =
            map_shader_resource_descriptor_sha256("HeightMap", 7, 3, 1, 61, 1, &[(7, 21)])
                .expect("descriptor");
        assert!(canonical_sha256(&descriptor));
        assert_ne!(
            descriptor,
            map_shader_resource_descriptor_sha256("HeightMap", 8, 3, 1, 61, 1, &[(8, 24)],)
                .expect("different descriptor")
        );
        assert!(
            map_shader_resource_descriptor_sha256("HeightMap", 7, 3, 2, 61, 1, &[(7, 21)],)
                .is_err()
        );
    }

    fn binding(symbol: &str) -> TerrainDependentResourceBinding {
        TerrainDependentResourceBinding {
            symbol: symbol.into(),
            owner_packet: "XMAP05".into(),
            resource_sha256: "1".repeat(64),
            descriptor_sha256: "2".repeat(64),
        }
    }

    #[test]
    fn underwater_device_identity_is_exact_and_fail_closed() {
        validate_underwater_device_identity(0x1000, 0x1000, "SceneCB0").unwrap();
        assert!(
            validate_underwater_device_identity(0, 0x1000, "SceneCB0").is_err(),
            "a missing context/handoff device identity must fail closed"
        );
        assert!(
            validate_underwater_device_identity(0x1000, 0, "SceneCB0").is_err(),
            "a missing resource device identity must fail closed"
        );
        let mismatch = validate_underwater_device_identity(0x1000, 0x2000, "SceneCB0").unwrap_err();
        assert!(
            mismatch.to_string().contains("SceneCB0")
                && mismatch.to_string().contains("different D3D11 device"),
            "cross-device rejection must name the offending resource"
        );
    }

    #[test]
    fn underwater_device_preflight_precedes_d3d_state_mutation() {
        let source = include_str!("xmap04_d3d_handoff.rs");
        let method = source
            .split("pub unsafe fn render_underwater_refraction")
            .nth(1)
            .expect("underwater render method must remain present");
        let preflight = method
            .find("validate_underwater_device_ownership(")
            .expect("underwater render must retain live device ownership preflight");
        let first_mutation = method
            .find("OMSetRenderTargets")
            .expect("underwater render must retain its target bind");
        assert!(
            preflight < first_mutation,
            "device ownership must be proven before the first D3D state mutation"
        );
    }

    fn valid_underwater_receipt() -> Xmap04UnderwaterDrawReceipt {
        let target_producer_frame_sha256 = "f".repeat(64);
        let target_texture_descriptor_sha256 = "c".repeat(64);
        let target_producer_allocation_generation = 19;
        let target_producer_allocation_identity_sha256 =
            xmap04_underwater_producer_allocation_identity_sha256(
                &target_producer_frame_sha256,
                target_producer_allocation_generation,
                &target_texture_descriptor_sha256,
            )
            .unwrap();
        let target_resource_identity_sha256 = xmap04_underwater_resource_identity_sha256(
            &target_producer_frame_sha256,
            &target_producer_allocation_identity_sha256,
            &target_texture_descriptor_sha256,
        )
        .unwrap();
        let ordered_light_resources = ["LightIndexMap", "LightDataMap", "ShadowMap"]
            .into_iter()
            .enumerate()
            .map(|(index, symbol)| TerrainDependentResourceBinding {
                symbol: symbol.into(),
                owner_packet: if symbol == "ShadowMap" {
                    "XMAP10".into()
                } else {
                    "XMAP09".into()
                },
                resource_sha256: format!("{:064x}", index + 1),
                descriptor_sha256: format!("{:064x}", index + 11),
            })
            .collect();
        let mut receipt = Xmap04UnderwaterDrawReceipt {
            terrain_frame_sha256: "1".repeat(64),
            replay_frame_identity_sha256: "8".repeat(64),
            camera_frame_sha256: "9".repeat(64),
            xmap09_camera_frame_sha256: "9".repeat(64),
            underwater_program_sha256: "2".repeat(64),
            target_resource_identity_sha256,
            target_producer_frame_sha256,
            target_producer_allocation_generation,
            target_producer_allocation_identity_sha256,
            target_texture_descriptor_sha256,
            target_rtv_descriptor_sha256: "d".repeat(64),
            target_srv_descriptor_sha256: "e".repeat(64),
            target_descriptor_sha256: "3".repeat(64),
            depth_descriptor_sha256: "a".repeat(64),
            scene_cb0_sha256: "4".repeat(64),
            scene_cb0_descriptor_sha256: "6".repeat(64),
            point_lights_cb2_sha256: "5".repeat(64),
            point_lights_cb2_descriptor_sha256: "7".repeat(64),
            ordered_light_resources,
            draw_count: 17,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = underwater_draw_receipt_digest(&receipt);
        receipt
    }

    #[test]
    fn xmap05_binding_receipt_unblocks_only_xmap05() {
        let bindings = vec![
            binding("ProvinceSecondaryColorMap"),
            binding("GradientBorderChannel1"),
            binding("GradientBorderChannel2"),
        ];
        let mut receipt = Xmap04D3dHandoffReceipt {
            terrain_frame_sha256: "3".repeat(64),
            terrain_program_sha256: "4".repeat(64),
            map_copy_receipt_set_sha256: "5".repeat(64),
            compiled_programs: 2,
            uploaded_shared_buffers: 2,
            uploaded_draw_buffers: 6,
            retained_static_resources: 10,
            uploaded_generated_resources: 1,
            bound_downstream_resources: Vec::new(),
            blocked_visible_draw_packets: vec![
                "XMAP05".into(),
                "XMAP07".into(),
                "XMAP09".into(),
                "XMAP10".into(),
            ],
            deterministic_sha256: String::new(),
        };
        record_xmap05_dependency_bindings(&mut receipt, &bindings).unwrap();
        assert_eq!(receipt.bound_downstream_resources, bindings);
        assert_eq!(
            receipt.blocked_visible_draw_packets,
            [
                "XMAP07".to_string(),
                "XMAP09".to_string(),
                "XMAP10".to_string()
            ]
        );
        assert_eq!(
            receipt.deterministic_sha256,
            handoff_receipt_digest(&receipt)
        );

        let mut misordered = receipt.bound_downstream_resources.clone();
        misordered.swap(0, 1);
        assert!(record_xmap05_dependency_bindings(&mut receipt, &misordered).is_err());
    }

    #[test]
    fn underwater_receipt_binds_every_cross_packet_identity() {
        let mut receipt = valid_underwater_receipt();
        receipt.validate().unwrap();
        let baseline = receipt.deterministic_sha256.clone();
        receipt.scene_cb0_descriptor_sha256 = "f".repeat(64);
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
        receipt.scene_cb0_descriptor_sha256 = "6".repeat(64);
        receipt.xmap09_camera_frame_sha256 = "8".repeat(64);
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
        receipt.xmap09_camera_frame_sha256 = "9".repeat(64);
        receipt.replay_frame_identity_sha256 = "f".repeat(64);
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
        receipt.replay_frame_identity_sha256 = "8".repeat(64);
        receipt.target_resource_identity_sha256 = "f".repeat(64);
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
        receipt.target_resource_identity_sha256 =
            valid_underwater_receipt().target_resource_identity_sha256;
        receipt.target_producer_frame_sha256 = "a".repeat(64);
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
        receipt.target_producer_frame_sha256 = "f".repeat(64);
        receipt.target_producer_allocation_generation += 1;
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
        receipt.target_producer_allocation_generation -= 1;
        receipt.target_producer_allocation_identity_sha256 = "f".repeat(64);
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
        receipt.target_producer_allocation_identity_sha256 =
            valid_underwater_receipt().target_producer_allocation_identity_sha256;
        receipt.target_srv_descriptor_sha256 = "f".repeat(64);
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
        receipt.target_srv_descriptor_sha256 = "e".repeat(64);
        receipt.ordered_light_resources[2].descriptor_sha256 = "f".repeat(64);
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
        receipt.ordered_light_resources[2].descriptor_sha256 = format!("{:064x}", 13);
        receipt.draw_count += 1;
        assert_ne!(baseline, underwater_draw_receipt_digest(&receipt));
    }

    #[test]
    fn underwater_receipt_validation_rejects_incomplete_execution() {
        let mut receipt = valid_underwater_receipt();
        receipt.target_producer_allocation_generation = 0;
        assert!(receipt.validate().is_err());

        let mut receipt = valid_underwater_receipt();
        receipt.draw_count = 0;
        assert!(receipt.validate().is_err());

        let mut receipt = valid_underwater_receipt();
        receipt.xmap09_camera_frame_sha256 = "8".repeat(64);
        receipt.deterministic_sha256 = underwater_draw_receipt_digest(&receipt);
        assert!(receipt.validate().is_err());
    }

    #[test]
    fn underwater_receipt_validation_rejects_resource_order_owner_and_hash_tamper() {
        let mut receipt = valid_underwater_receipt();
        receipt.ordered_light_resources.swap(0, 1);
        receipt.deterministic_sha256 = underwater_draw_receipt_digest(&receipt);
        assert!(receipt.validate().is_err());

        let mut receipt = valid_underwater_receipt();
        receipt.ordered_light_resources[2].owner_packet = "XMAP09".into();
        receipt.deterministic_sha256 = underwater_draw_receipt_digest(&receipt);
        assert!(receipt.validate().is_err());

        let mut receipt = valid_underwater_receipt();
        receipt.ordered_light_resources[0].resource_sha256 = "0".repeat(64);
        receipt.deterministic_sha256 = underwater_draw_receipt_digest(&receipt);
        assert!(receipt.validate().is_err());
    }

    #[test]
    fn underwater_receipt_validation_rejects_allocation_resource_and_digest_tamper() {
        let mut receipt = valid_underwater_receipt();
        receipt.target_producer_allocation_identity_sha256 = "a".repeat(64);
        receipt.deterministic_sha256 = underwater_draw_receipt_digest(&receipt);
        assert!(receipt.validate().is_err());

        let mut receipt = valid_underwater_receipt();
        receipt.target_resource_identity_sha256 = "b".repeat(64);
        receipt.deterministic_sha256 = underwater_draw_receipt_digest(&receipt);
        assert!(receipt.validate().is_err());

        let mut receipt = valid_underwater_receipt();
        receipt.deterministic_sha256 = "d".repeat(64);
        assert!(receipt.validate().is_err());
    }

    #[test]
    fn underwater_sha256_gate_rejects_placeholders_and_aliases() {
        let canonical = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        assert!(canonical_sha256(canonical));
        assert!(!canonical_sha256(&canonical.to_ascii_uppercase()));
        assert!(!canonical_sha256(&"0".repeat(64)));
        assert!(!canonical_sha256("abc"));
    }

    #[test]
    fn underwater_dds_shape_rejects_unbounded_mips_and_dimensions() {
        assert_eq!(validate_underwater_dds_shape(4, 4, 0).unwrap(), 1);
        assert_eq!(validate_underwater_dds_shape(4, 4, 3).unwrap(), 3);
        assert!(validate_underwater_dds_shape(4, 4, 4).is_err());
        assert!(
            validate_underwater_dds_shape(XMAP04_D3D11_MAX_TEXTURE_2D_DIMENSION + 1, 4, 1).is_err()
        );
    }

    #[test]
    fn underwater_target_descriptor_hashes_bind_texture_and_both_view_ranges() {
        let texture =
            xmap04_underwater_texture_descriptor_sha256(1920, 1080, 87, 1, 0, 1, 1, 0, 40, 0, 0);
        let frame = "1".repeat(64);
        let allocation_a =
            xmap04_underwater_producer_allocation_identity_sha256(&frame, 1, &texture).unwrap();
        let allocation_b =
            xmap04_underwater_producer_allocation_identity_sha256(&frame, 2, &texture).unwrap();
        assert_ne!(allocation_a, allocation_b);
        let resource =
            xmap04_underwater_resource_identity_sha256(&frame, &allocation_a, &texture).unwrap();
        assert_ne!(
            resource,
            xmap04_underwater_resource_identity_sha256(&frame, &allocation_b, &texture).unwrap()
        );
        assert!(
            xmap04_underwater_producer_allocation_identity_sha256(&frame, 0, &texture).is_err()
        );
        assert!(
            xmap04_underwater_resource_identity_sha256("not-a-sha", &allocation_a, &texture)
                .is_err()
        );
        assert_ne!(
            texture,
            xmap04_underwater_texture_descriptor_sha256(1920, 1080, 87, 1, 0, 2, 1, 0, 40, 0, 0)
        );
        let rtv = xmap04_underwater_rtv_descriptor_sha256(87, 4, 0, 0, 1);
        assert_ne!(rtv, xmap04_underwater_rtv_descriptor_sha256(87, 4, 1, 0, 1));
        let srv = xmap04_underwater_srv_descriptor_sha256(87, 4, 0, 1, 0, 1);
        assert_ne!(
            srv,
            xmap04_underwater_srv_descriptor_sha256(87, 4, 0, 1, 1, 1)
        );
    }
}
