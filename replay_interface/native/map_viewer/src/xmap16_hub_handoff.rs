
use crate::rail_files::{
    supply_hub_background_atlas_frame, supply_hub_common_icon_atlas_frame,
    SupplyHubContentIdentity, SupplyHubSpriteBinding,
};
use crate::{GameFileResolver, PreparedXmap16Frame, Result, ViewerOwnedMapState};
use portable_content_manifest::UsedContentKey;
use replay_core::RailHubStateFirstVisual;
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;
use std::path::Path;

pub const XMAP16_HUB_GUI_PACKET_SCHEMA: &str = "hoi4.xmap16.supply-hub-gui.a729.v1";
pub const XMAP16_HUB_UPLOAD_RECEIPT_SCHEMA: &str = "hoi4.xmap16.supply-hub-upload.a729.v1";
pub const XMAP16_HUB_EXECUTION_RECEIPT_SCHEMA: &str = "hoi4.xmap16.supply-hub-execution.a729.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Xmap16HubPresentationMode {
    Normal,
    Supply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xmap16HubViewerSelection {
    pub hour_index: i64,
    pub perspective_country_id: u32,
    pub mode: Xmap16HubPresentationMode,
}

pub trait Xmap16HubViewerSelectionProvider {
    fn selection(&self, viewer: &ViewerOwnedMapState) -> Result<Option<Xmap16HubViewerSelection>>;
}

pub trait Xmap16HubPresentationModeProvider {
    fn presentation_mode(
        &self,
        viewer: &ViewerOwnedMapState,
    ) -> Result<Option<Xmap16HubPresentationMode>>;
}

pub struct D18Xmap16HubViewerSelectionProvider<'a> {
    pub world_frame: &'a replay_core::WorldFrame,
    pub mode_provider: &'a dyn Xmap16HubPresentationModeProvider,
}

impl Xmap16HubViewerSelectionProvider for D18Xmap16HubViewerSelectionProvider<'_> {
    fn selection(&self, viewer: &ViewerOwnedMapState) -> Result<Option<Xmap16HubViewerSelection>> {
        let tag = viewer
            .observer_country_id
            .as_deref()
            .ok_or("XMAP16 hub GUI requires a viewer-selected country")?;
        let roster = self.world_frame.d18_country_roster();
        if roster.status != replay_core::ViewStatus::Present {
            return Err(format!(
                "XMAP16 viewer-country D18 roster is {:?}: {}",
                roster.status,
                roster.diagnostic.as_deref().unwrap_or("no diagnostic")
            )
            .into());
        }
        let perspective_country_id = roster
            .entries
            .iter()
            .find(|entry| entry.tag == tag)
            .map(|entry| entry.tag_db_id)
            .ok_or_else(|| format!("XMAP16 viewer country {tag} is absent from the D18 roster"))?;
        let Some(mode) = self.mode_provider.presentation_mode(viewer)? else {
            return Ok(None);
        };
        Ok(Some(Xmap16HubViewerSelection {
            hour_index: self.world_frame.hour_index(),
            perspective_country_id,
            mode,
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Xmap16HubTextureRole {
    Background,
    CommonIcon,
    NavalOverlay,
}

impl Xmap16HubTextureRole {
    fn stable_name(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::CommonIcon => "common_icon",
            Self::NavalOverlay => "naval_overlay",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Xmap16HubGuiLayer {
    Background,
    CommonIcon,
    NavalOverlay,
}

impl Xmap16HubGuiLayer {
    fn texture_role(self) -> Xmap16HubTextureRole {
        match self {
            Self::Background => Xmap16HubTextureRole::Background,
            Self::CommonIcon => Xmap16HubTextureRole::CommonIcon,
            Self::NavalOverlay => Xmap16HubTextureRole::NavalOverlay,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xmap16HubGuiVertex {
    pub position_px: [f32; 2],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap16HubGuiQuad {
    pub perspective_country_id: u32,
    pub province_id: u32,
    pub subtype: u8,
    pub layer: Xmap16HubGuiLayer,
    pub texture_role: Xmap16HubTextureRole,
    pub size_px: [f32; 2],
    pub vertices: [Xmap16HubGuiVertex; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap16HubTextureDescriptor {
    pub role: Xmap16HubTextureRole,
    pub used_content: UsedContentKey,
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
    pub dxgi_format: u32,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedXmap16HubGuiPacket {
    pub schema: String,
    pub hour_index: i64,
    pub rail_supply_frame_sha256: String,
    pub camera_sha256: String,
    pub gui_scale_f32_bits: u32,
    pub selection: Xmap16HubViewerSelection,
    pub textures: Vec<Xmap16HubTextureDescriptor>,
    pub quads: Vec<Xmap16HubGuiQuad>,
    pub deterministic_sha256: String,
}

impl PreparedXmap16HubGuiPacket {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP16_HUB_GUI_PACKET_SCHEMA {
            return Err("XMAP16 hub GUI packet schema mismatch".into());
        }
        if self.selection.hour_index != self.hour_index {
            return Err("XMAP16 hub viewer selection is from a different replay hour".into());
        }
        validate_sha256(
            &self.rail_supply_frame_sha256,
            "XMAP16 hub rail/supply frame",
        )?;
        validate_sha256(&self.camera_sha256, "XMAP16 hub camera")?;
        let gui_scale = f32::from_bits(self.gui_scale_f32_bits);
        if !gui_scale.is_finite() || gui_scale <= 0.0 {
            return Err("XMAP16 hub GUI packet has invalid playback GUI scale".into());
        }
        if self.textures.len() != 3 {
            return Err("XMAP16 hub GUI packet must carry exactly three textures".into());
        }
        let expected_roles = [
            Xmap16HubTextureRole::Background,
            Xmap16HubTextureRole::CommonIcon,
            Xmap16HubTextureRole::NavalOverlay,
        ];
        for (descriptor, expected_role) in self.textures.iter().zip(expected_roles) {
            descriptor.validate()?;
            if descriptor.role != expected_role {
                return Err("XMAP16 hub texture order drifted".into());
            }
        }
        let mut previous_key = None;
        for quad in &self.quads {
            quad.validate()?;
            if quad.perspective_country_id != self.selection.perspective_country_id {
                return Err("XMAP16 hub quad escaped the selected perspective".into());
            }
            let key = (quad.province_id, quad.subtype, quad.layer);
            if previous_key.is_some_and(|previous| previous >= key) {
                return Err("XMAP16 hub GUI quad order is not canonical".into());
            }
            previous_key = Some(key);
        }
        validate_sha256(&self.deterministic_sha256, "XMAP16 hub GUI packet")?;
        if self.deterministic_sha256 != prepared_packet_digest(self) {
            return Err("XMAP16 hub GUI packet digest mismatch".into());
        }
        Ok(())
    }
}

impl Xmap16HubTextureDescriptor {
    fn validate(&self) -> Result<()> {
        self.used_content
            .validate()
            .map_err(|error| format!("XMAP16 hub texture content: {error}"))?;
        if self.width == 0
            || self.height == 0
            || self.mip_levels == 0
            || self.dxgi_format != crate::DXGI_FORMAT_B8G8R8A8_UNORM
        {
            return Err("XMAP16 hub texture descriptor is invalid".into());
        }
        validate_sha256(&self.descriptor_sha256, "XMAP16 hub texture descriptor")?;
        if self.descriptor_sha256 != texture_descriptor_digest(self) {
            return Err("XMAP16 hub texture descriptor digest mismatch".into());
        }
        Ok(())
    }
}

impl Xmap16HubGuiQuad {
    fn validate(&self) -> Result<()> {
        if self.texture_role != self.layer.texture_role()
            || self
                .vertices
                .iter()
                .flat_map(|vertex| vertex.position_px.into_iter().chain(vertex.uv))
                .any(|value| !value.is_finite())
        {
            return Err("XMAP16 hub GUI quad binding/vertex is invalid".into());
        }
        if self
            .size_px
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err("XMAP16 hub GUI quad size is invalid".into());
        }
        let [top_left, top_right, bottom_left, bottom_right] = self.vertices;
        if top_right.position_px
            != [
                top_left.position_px[0] + self.size_px[0],
                top_left.position_px[1],
            ]
            || bottom_left.position_px
                != [
                    top_left.position_px[0],
                    top_left.position_px[1] + self.size_px[1],
                ]
            || bottom_right.position_px
                != [
                    top_left.position_px[0] + self.size_px[0],
                    top_left.position_px[1] + self.size_px[1],
                ]
            || top_right.uv != [bottom_right.uv[0], top_left.uv[1]]
            || bottom_left.uv != [top_left.uv[0], bottom_right.uv[1]]
        {
            return Err("XMAP16 hub GUI quad geometry/UV winding drifted".into());
        }
        Ok(())
    }
}

pub fn prepare_xmap16_hub_gui_packet(
    frame: &PreparedXmap16Frame,
    viewer: &ViewerOwnedMapState,
    selection_provider: &dyn Xmap16HubViewerSelectionProvider,
    gui_scale: f32,
) -> Result<PreparedXmap16HubGuiPacket> {
    frame.validate()?;
    viewer.validate()?;
    let selection = selection_provider
        .selection(viewer)?
        .ok_or("XMAP16 hub GUI selection provider is unavailable")?;
    if selection.hour_index != frame.hour_index {
        return Err("XMAP16 hub GUI selection is stale for the prepared replay hour".into());
    }
    let camera = viewer
        .camera
        .as_ref()
        .ok_or("XMAP16 hub GUI requires the viewer-owned XMAP03 camera")?;
    camera.validate()?;
    if !gui_scale.is_finite() || gui_scale <= 0.0 {
        return Err("XMAP16 hub GUI requires a positive finite playback GUI scale".into());
    }
    let production_camera = camera.production_camera()?;
    let textures = hub_texture_descriptors(frame)?;
    let quads = prepare_quads_from_parts(
        &frame.hub_visual_states,
        &frame.hub_content,
        selection,
        &production_camera,
        camera.current_eye()[1],
        camera.viewport_width,
        camera.viewport_height,
        gui_scale,
    )?;
    let mut packet = PreparedXmap16HubGuiPacket {
        schema: XMAP16_HUB_GUI_PACKET_SCHEMA.into(),
        hour_index: frame.hour_index,
        rail_supply_frame_sha256: frame.deterministic_sha256.clone(),
        camera_sha256: camera.deterministic_sha256.clone(),
        gui_scale_f32_bits: gui_scale.to_bits(),
        selection,
        textures,
        quads,
        deterministic_sha256: String::new(),
    };
    packet.deterministic_sha256 = prepared_packet_digest(&packet);
    packet.validate()?;
    Ok(packet)
}

fn prepare_quads_from_parts(
    rows: &BTreeMap<(u32, u32, u8), RailHubStateFirstVisual>,
    content: &SupplyHubContentIdentity,
    selection: Xmap16HubViewerSelection,
    camera: &crate::ProductionMapCamera,
    camera_height: f32,
    viewport_width: u32,
    viewport_height: u32,
    gui_scale: f32,
) -> Result<Vec<Xmap16HubGuiQuad>> {
    if !camera_height.is_finite()
        || viewport_width == 0
        || viewport_height == 0
        || !gui_scale.is_finite()
        || gui_scale <= 0.0
    {
        return Err("XMAP16 hub projection/camera cutoff input is invalid".into());
    }
    let mut quads = Vec::new();
    for ((perspective, province, subtype), row) in rows {
        if *perspective != selection.perspective_country_id {
            continue;
        }
        validate_hub_row(*subtype, row)?;
        let eligible = match selection.mode {
            Xmap16HubPresentationMode::Normal => row.normal_eligible,
            Xmap16HubPresentationMode::Supply => row.supply_eligible,
        };
        if !eligible || !hub_visible_at_camera_height(row, camera_height)? {
            continue;
        }
        let Some(anchor) = camera.world_to_screen(
            row.final_world,
            viewport_width as f32,
            viewport_height as f32,
        ) else {
            continue;
        };
        let background = supply_hub_background_atlas_frame(content, row.background_button_frame)?;
        let common = supply_hub_common_icon_atlas_frame(content, row.common_icon_frame)?;
        let (background_offset, background_size) =
            hub_control_geometry(content, &content.background, gui_scale)?;
        let (common_offset, common_size) =
            hub_control_geometry(content, &content.common_icon, gui_scale)?;
        quads.push(hub_quad(
            selection.perspective_country_id,
            *province,
            *subtype,
            Xmap16HubGuiLayer::Background,
            anchor,
            background_offset,
            background_size,
            background.uv,
        ));
        quads.push(hub_quad(
            selection.perspective_country_id,
            *province,
            *subtype,
            Xmap16HubGuiLayer::CommonIcon,
            anchor,
            common_offset,
            common_size,
            common.uv,
        ));
        if row.naval_overlay_visible {
            let naval = content
                .naval_overlay
                .sprite
                .frame(0)
                .copied()
                .ok_or("XMAP16 naval hub overlay atlas cell is missing")?;
            let (naval_offset, naval_size) =
                hub_control_geometry(content, &content.naval_overlay, gui_scale)?;
            quads.push(hub_quad(
                selection.perspective_country_id,
                *province,
                *subtype,
                Xmap16HubGuiLayer::NavalOverlay,
                anchor,
                naval_offset,
                naval_size,
                naval.uv,
            ));
        }
    }
    quads.sort_by_key(|quad| (quad.province_id, quad.subtype, quad.layer));
    Ok(quads)
}

fn validate_hub_row(subtype: u8, row: &RailHubStateFirstVisual) -> Result<()> {
    if subtype > 1
        || row.final_world_bits != row.final_world.map(f32::to_bits)
        || row.final_world.iter().any(|value| !value.is_finite())
        || row.visibility_cutoff.value_bits != row.visibility_cutoff.value.to_bits()
        || !row.visibility_cutoff.value.is_finite()
        || row.visibility_cutoff.value < 0.0
    {
        return Err("XMAP16 state-first hub row is malformed".into());
    }
    Ok(())
}

fn hub_visible_at_camera_height(row: &RailHubStateFirstVisual, camera_height: f32) -> Result<bool> {
    use replay_core::RailHubVisibilityCutoffClass;
    Ok(match row.visibility_cutoff_class {
        RailHubVisibilityCutoffClass::SupplyIcon | RailHubVisibilityCutoffClass::End => {
            camera_height < row.visibility_cutoff.value
        }
        RailHubVisibilityCutoffClass::Unused | RailHubVisibilityCutoffClass::Disconnected => {
            camera_height <= row.visibility_cutoff.value
        }
    })
}

fn hub_control_geometry(
    content: &SupplyHubContentIdentity,
    binding: &SupplyHubSpriteBinding,
    gui_scale: f32,
) -> Result<([f32; 2], [f32; 2])> {
    let control = content
        .container
        .control(&binding.control_name)
        .ok_or_else(|| {
            format!(
                "XMAP16 hub GUI container omitted control {}",
                binding.control_name
            )
        })?;
    let position = control.position.ok_or_else(|| {
        format!(
            "XMAP16 hub GUI control {} omitted its local position",
            binding.control_name
        )
    })?;
    let intrinsic_size = [
        binding.sprite.frame_width as f32,
        binding.sprite.frame_height as f32,
    ];
    let size = control
        .size
        .map(|size| [size.width as f32, size.height as f32])
        .unwrap_or(intrinsic_size);
    if size.iter().any(|value| !value.is_finite() || *value <= 0.0) {
        return Err(format!(
            "XMAP16 hub GUI control {} has invalid effective size",
            binding.control_name
        )
        .into());
    }
    Ok((
        [position.x as f32 * gui_scale, position.y as f32 * gui_scale],
        [size[0] * gui_scale, size[1] * gui_scale],
    ))
}

fn hub_quad(
    perspective_country_id: u32,
    province_id: u32,
    subtype: u8,
    layer: Xmap16HubGuiLayer,
    anchor: (f32, f32),
    offset: [f32; 2],
    size: [f32; 2],
    uv: crate::UvRect,
) -> Xmap16HubGuiQuad {
    let left = anchor.0 + offset[0];
    let top = anchor.1 + offset[1];
    let right = left + size[0];
    let bottom = top + size[1];
    Xmap16HubGuiQuad {
        perspective_country_id,
        province_id,
        subtype,
        layer,
        texture_role: layer.texture_role(),
        size_px: size,
        vertices: [
            Xmap16HubGuiVertex {
                position_px: [left, top],
                uv: [uv.u0, uv.v0],
            },
            Xmap16HubGuiVertex {
                position_px: [right, top],
                uv: [uv.u1, uv.v0],
            },
            Xmap16HubGuiVertex {
                position_px: [left, bottom],
                uv: [uv.u0, uv.v1],
            },
            Xmap16HubGuiVertex {
                position_px: [right, bottom],
                uv: [uv.u1, uv.v1],
            },
        ],
    }
}

fn hub_texture_descriptors(frame: &PreparedXmap16Frame) -> Result<Vec<Xmap16HubTextureDescriptor>> {
    [
        (
            Xmap16HubTextureRole::Background,
            &frame.hub_content.background,
        ),
        (
            Xmap16HubTextureRole::CommonIcon,
            &frame.hub_content.common_icon,
        ),
        (
            Xmap16HubTextureRole::NavalOverlay,
            &frame.hub_content.naval_overlay,
        ),
    ]
    .into_iter()
    .map(|(role, binding)| {
        let content = frame
            .ordered_used_content
            .iter()
            .find(|content| {
                content
                    .normalized_game_relative_path
                    .eq_ignore_ascii_case(&binding.sprite.texture_file)
            })
            .ok_or_else(|| {
                format!(
                    "XMAP16 hub texture {} escaped the admitted content closure",
                    binding.sprite.texture_file
                )
            })?
            .clone();
        if content.payload_sha256 != binding.texture_sha256
            || content.payload_bytes != binding.texture_bytes
        {
            return Err(format!(
                "XMAP16 hub texture identity drifted for {}",
                binding.sprite.texture_file
            )
            .into());
        }
        texture_descriptor(role, binding, content)
    })
    .collect()
}

fn texture_descriptor(
    role: Xmap16HubTextureRole,
    binding: &SupplyHubSpriteBinding,
    used_content: UsedContentKey,
) -> Result<Xmap16HubTextureDescriptor> {
    let mut descriptor = Xmap16HubTextureDescriptor {
        role,
        used_content,
        width: binding.sprite.texture_width,
        height: binding.sprite.texture_height,
        mip_levels: binding.sprite.mip_count,
        dxgi_format: crate::DXGI_FORMAT_B8G8R8A8_UNORM,
        descriptor_sha256: String::new(),
    };
    descriptor.descriptor_sha256 = texture_descriptor_digest(&descriptor);
    descriptor.validate()?;
    Ok(descriptor)
}

fn texture_descriptor_digest(descriptor: &Xmap16HubTextureDescriptor) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(descriptor.role.stable_name().as_bytes());
    identity.extend_from_slice(
        descriptor
            .used_content
            .normalized_game_relative_path
            .as_bytes(),
    );
    identity.extend_from_slice(descriptor.used_content.payload_sha256.as_bytes());
    identity.extend_from_slice(&descriptor.used_content.payload_bytes.to_le_bytes());
    identity.extend_from_slice(&descriptor.width.to_le_bytes());
    identity.extend_from_slice(&descriptor.height.to_le_bytes());
    identity.extend_from_slice(&descriptor.mip_levels.to_le_bytes());
    identity.extend_from_slice(&descriptor.dxgi_format.to_le_bytes());
    Sha256::hex(&identity)
}

fn prepared_packet_digest(packet: &PreparedXmap16HubGuiPacket) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(packet.schema.as_bytes());
    identity.extend_from_slice(&packet.hour_index.to_le_bytes());
    identity.extend_from_slice(packet.rail_supply_frame_sha256.as_bytes());
    identity.extend_from_slice(packet.camera_sha256.as_bytes());
    identity.extend_from_slice(&packet.gui_scale_f32_bits.to_le_bytes());
    identity.extend_from_slice(&packet.selection.hour_index.to_le_bytes());
    identity.extend_from_slice(&packet.selection.perspective_country_id.to_le_bytes());
    identity.push(match packet.selection.mode {
        Xmap16HubPresentationMode::Normal => 0,
        Xmap16HubPresentationMode::Supply => 1,
    });
    for texture in &packet.textures {
        identity.extend_from_slice(texture.descriptor_sha256.as_bytes());
    }
    for quad in &packet.quads {
        identity.extend_from_slice(&quad.perspective_country_id.to_le_bytes());
        identity.extend_from_slice(&quad.province_id.to_le_bytes());
        identity.push(quad.subtype);
        identity.push(quad.layer as u8);
        for value in quad.size_px {
            identity.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        for vertex in quad.vertices {
            for value in vertex.position_px.into_iter().chain(vertex.uv) {
                identity.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
    }
    Sha256::hex(&identity)
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not a canonical SHA-256").into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap16HubUploadReceipt {
    pub schema: String,
    pub packet_sha256: String,
    pub uploaded_resources: u32,
    pub created_descriptors: u32,
    pub ordered_source_sha256: Vec<String>,
    pub ordered_decoded_bgra_sha256: Vec<String>,
    pub ordered_descriptor_sha256: Vec<String>,
    pub deterministic_sha256: String,
}

impl Xmap16HubUploadReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP16_HUB_UPLOAD_RECEIPT_SCHEMA
            || self.uploaded_resources != 3
            || self.created_descriptors != 3
            || self.ordered_source_sha256.len() != 3
            || self.ordered_decoded_bgra_sha256.len() != 3
            || self.ordered_descriptor_sha256.len() != 3
        {
            return Err("XMAP16 hub upload receipt shape is invalid".into());
        }
        validate_sha256(&self.packet_sha256, "XMAP16 hub upload packet")?;
        for value in self
            .ordered_source_sha256
            .iter()
            .chain(&self.ordered_decoded_bgra_sha256)
            .chain(&self.ordered_descriptor_sha256)
        {
            validate_sha256(value, "XMAP16 hub upload identity")?;
        }
        validate_sha256(&self.deterministic_sha256, "XMAP16 hub upload receipt")?;
        if self.deterministic_sha256 != upload_receipt_digest(self) {
            return Err("XMAP16 hub upload receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap16HubExecutionReceipt {
    pub schema: String,
    pub packet_sha256: String,
    pub upload_receipt_sha256: String,
    pub executor_identity_sha256: String,
    pub submitted_quads: u32,
    pub deterministic_sha256: String,
}

impl Xmap16HubExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP16_HUB_EXECUTION_RECEIPT_SCHEMA {
            return Err("XMAP16 hub execution receipt schema mismatch".into());
        }
        for (value, label) in [
            (&self.packet_sha256, "XMAP16 hub execution packet"),
            (
                &self.upload_receipt_sha256,
                "XMAP16 hub execution upload receipt",
            ),
            (&self.executor_identity_sha256, "XMAP16 hub GUI executor"),
            (&self.deterministic_sha256, "XMAP16 hub execution receipt"),
        ] {
            validate_sha256(value, label)?;
        }
        if self.deterministic_sha256 != execution_receipt_digest(self) {
            return Err("XMAP16 hub execution receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
pub struct NativeXmap16HubTextureSet {
    textures: Vec<NativeXmap16HubTexture>,
}

#[cfg(windows)]
struct NativeXmap16HubTexture {
    role: Xmap16HubTextureRole,
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    decoded_bgra_sha256: String,
}

#[cfg(windows)]
impl NativeXmap16HubTextureSet {
    pub fn shader_resource_view(
        &self,
        role: Xmap16HubTextureRole,
    ) -> Result<*mut winapi::um::d3d11::ID3D11ShaderResourceView> {
        self.textures
            .iter()
            .find(|texture| texture.role == role)
            .map(|texture| texture.view.as_raw())
            .ok_or_else(|| format!("XMAP16 hub upload omitted {} SRV", role.stable_name()).into())
    }
}

#[cfg(windows)]
pub trait NativeXmap16HubGuiExecutor {
    fn executor_identity_sha256(&self) -> &str;

    unsafe fn draw_xmap16_hub_quads(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        packet: &PreparedXmap16HubGuiPacket,
        textures: &NativeXmap16HubTextureSet,
    ) -> Result<()>;
}

#[cfg(windows)]
pub struct NativeXmap16HubHandoff {
    textures: NativeXmap16HubTextureSet,
    texture_descriptors: Vec<Xmap16HubTextureDescriptor>,
    packet_sha256: String,
    receipt: Xmap16HubUploadReceipt,
}

#[cfg(windows)]
impl NativeXmap16HubHandoff {
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        packet: &PreparedXmap16HubGuiPacket,
    ) -> Result<Self> {
        packet.validate()?;
        if device.is_null() {
            return Err("XMAP16 hub handoff received a null D3D11 device".into());
        }
        let textures = upload_hub_textures(device, resolver, &packet.textures)?;
        let receipt = upload_receipt(packet, &textures);
        receipt.validate()?;
        Ok(Self {
            textures,
            texture_descriptors: packet.textures.clone(),
            packet_sha256: packet.deterministic_sha256.clone(),
            receipt,
        })
    }

    pub unsafe fn update_packet(
        &mut self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        packet: &PreparedXmap16HubGuiPacket,
    ) -> Result<&Xmap16HubUploadReceipt> {
        packet.validate()?;
        if packet.textures != self.texture_descriptors {
            let replacement = upload_hub_textures(device, resolver, &packet.textures)?;
            self.textures = replacement;
            self.texture_descriptors = packet.textures.clone();
        }
        self.packet_sha256 = packet.deterministic_sha256.clone();
        self.receipt = upload_receipt(packet, &self.textures);
        self.receipt.validate()?;
        Ok(&self.receipt)
    }

    pub fn receipt(&self) -> &Xmap16HubUploadReceipt {
        &self.receipt
    }

    pub unsafe fn execute_with(
        &self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        packet: &PreparedXmap16HubGuiPacket,
        executor: &mut dyn NativeXmap16HubGuiExecutor,
    ) -> Result<Xmap16HubExecutionReceipt> {
        packet.validate()?;
        if context.is_null() {
            return Err("XMAP16 hub execution received a null D3D11 context".into());
        }
        if packet.deterministic_sha256 != self.packet_sha256
            || packet.deterministic_sha256 != self.receipt.packet_sha256
        {
            return Err("XMAP16 hub execution packet was not uploaded/prepared".into());
        }
        validate_sha256(
            executor.executor_identity_sha256(),
            "XMAP16 shared GUI executor",
        )?;
        executor.draw_xmap16_hub_quads(context, packet, &self.textures)?;
        let mut receipt = Xmap16HubExecutionReceipt {
            schema: XMAP16_HUB_EXECUTION_RECEIPT_SCHEMA.into(),
            packet_sha256: packet.deterministic_sha256.clone(),
            upload_receipt_sha256: self.receipt.deterministic_sha256.clone(),
            executor_identity_sha256: executor.executor_identity_sha256().into(),
            submitted_quads: u32::try_from(packet.quads.len())
                .map_err(|_| "XMAP16 hub quad count exceeds u32")?,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = execution_receipt_digest(&receipt);
        receipt.validate()?;
        Ok(receipt)
    }
}

#[cfg(windows)]
unsafe fn upload_hub_textures(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &GameFileResolver,
    descriptors: &[Xmap16HubTextureDescriptor],
) -> Result<NativeXmap16HubTextureSet> {
    if descriptors.len() != 3 {
        return Err("XMAP16 native hub upload requires exactly three descriptors".into());
    }
    let mut textures = Vec::with_capacity(3);
    for descriptor in descriptors {
        descriptor.validate()?;
        let bytes = resolver.read_admitted_used_content(&descriptor.used_content)?;
        let mips = decode_admitted_dds_bgra_mips(
            &bytes,
            &descriptor.used_content.normalized_game_relative_path,
            descriptor,
        )?;
        textures.push(create_hub_texture(device, descriptor, &mips)?);
    }
    Ok(NativeXmap16HubTextureSet { textures })
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct HubBgraMip {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

#[cfg(windows)]
fn decode_admitted_dds_bgra_mips(
    bytes: &[u8],
    logical_path: &str,
    descriptor: &Xmap16HubTextureDescriptor,
) -> Result<Vec<HubBgraMip>> {
    if bytes.get(0..4) != Some(b"DDS ") || bytes.len() < 128 {
        return Err(format!("XMAP16 hub texture {logical_path} is not a complete DDS").into());
    }
    let read_u32 = |offset: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("XMAP16 hub DDS header is truncated")?
                .try_into()?,
        ))
    };
    let width = read_u32(16)?;
    let height = read_u32(12)?;
    let mip_count = read_u32(28)?.max(1);
    if width != descriptor.width
        || height != descriptor.height
        || mip_count != descriptor.mip_levels
    {
        return Err(format!("XMAP16 hub DDS descriptor drifted for {logical_path}").into());
    }
    let pixel_format_flags = read_u32(80)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or("XMAP16 hub DDS pixel format is truncated")?;
    let rgb_bits = usize::try_from(read_u32(88)?)?;
    let mut offset = 128usize;
    let mut mip_width = usize::try_from(width)?;
    let mut mip_height = usize::try_from(height)?;
    let mut out = Vec::with_capacity(usize::try_from(mip_count)?);
    for _ in 0..mip_count {
        let rgba = crate::image_load::decode_dds_rgba_mip(
            Path::new(logical_path),
            bytes,
            mip_width,
            mip_height,
            offset,
            pixel_format_flags,
            fourcc,
            rgb_bits,
        )?;
        let mut bgra = rgba.pixels;
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        out.push(HubBgraMip {
            width: u32::try_from(rgba.width)?,
            height: u32::try_from(rgba.height)?,
            bytes: bgra,
        });
        offset = offset
            .checked_add(crate::image_load::dds_mip_byte_len(
                mip_width, mip_height, fourcc, rgb_bits,
            )?)
            .ok_or("XMAP16 hub DDS mip offset overflowed")?;
        mip_width = (mip_width / 2).max(1);
        mip_height = (mip_height / 2).max(1);
    }
    Ok(out)
}

#[cfg(windows)]
unsafe fn create_hub_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    descriptor: &Xmap16HubTextureDescriptor,
    mips: &[HubBgraMip],
) -> Result<NativeXmap16HubTexture> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    if mips.len() != descriptor.mip_levels as usize {
        return Err("XMAP16 decoded hub mip count drifted".into());
    }
    let mut decoded_identity = Vec::new();
    let initial = mips
        .iter()
        .map(|mip| -> Result<D3D11_SUBRESOURCE_DATA> {
            let expected = usize::try_from(mip.width)?
                .checked_mul(usize::try_from(mip.height)?)
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or("XMAP16 hub mip byte length overflowed")?;
            if mip.bytes.len() != expected {
                return Err("XMAP16 decoded hub mip byte length drifted".into());
            }
            decoded_identity.extend_from_slice(&mip.width.to_le_bytes());
            decoded_identity.extend_from_slice(&mip.height.to_le_bytes());
            decoded_identity.extend_from_slice(&mip.bytes);
            Ok(D3D11_SUBRESOURCE_DATA {
                pSysMem: mip.bytes.as_ptr() as *const _,
                SysMemPitch: mip
                    .width
                    .checked_mul(4)
                    .ok_or("XMAP16 hub mip row pitch overflowed")?,
                SysMemSlicePitch: u32::try_from(mip.bytes.len())?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let native = D3D11_TEXTURE2D_DESC {
        Width: descriptor.width,
        Height: descriptor.height,
        MipLevels: descriptor.mip_levels,
        ArraySize: 1,
        Format: winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&native, initial.as_ptr(), &mut texture),
        &format!(
            "CreateTexture2D(XMAP16 hub {})",
            descriptor.role.stable_name()
        ),
    )?;
    let texture = D3dPtr::from_raw(texture, "XMAP16 hub texture")?;
    let mut view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut view),
        &format!(
            "CreateShaderResourceView(XMAP16 hub {})",
            descriptor.role.stable_name()
        ),
    )?;
    Ok(NativeXmap16HubTexture {
        role: descriptor.role,
        _texture: texture,
        view: D3dPtr::from_raw(view, "XMAP16 hub texture SRV")?,
        decoded_bgra_sha256: Sha256::hex(&decoded_identity),
    })
}

#[cfg(windows)]
fn upload_receipt(
    packet: &PreparedXmap16HubGuiPacket,
    textures: &NativeXmap16HubTextureSet,
) -> Xmap16HubUploadReceipt {
    let mut receipt = Xmap16HubUploadReceipt {
        schema: XMAP16_HUB_UPLOAD_RECEIPT_SCHEMA.into(),
        packet_sha256: packet.deterministic_sha256.clone(),
        uploaded_resources: textures.textures.len() as u32,
        created_descriptors: textures.textures.len() as u32,
        ordered_source_sha256: packet
            .textures
            .iter()
            .map(|texture| texture.used_content.payload_sha256.clone())
            .collect(),
        ordered_decoded_bgra_sha256: textures
            .textures
            .iter()
            .map(|texture| texture.decoded_bgra_sha256.clone())
            .collect(),
        ordered_descriptor_sha256: packet
            .textures
            .iter()
            .map(|texture| texture.descriptor_sha256.clone())
            .collect(),
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = upload_receipt_digest(&receipt);
    receipt
}

fn upload_receipt_digest(receipt: &Xmap16HubUploadReceipt) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(receipt.schema.as_bytes());
    identity.extend_from_slice(receipt.packet_sha256.as_bytes());
    identity.extend_from_slice(&receipt.uploaded_resources.to_le_bytes());
    identity.extend_from_slice(&receipt.created_descriptors.to_le_bytes());
    for value in receipt
        .ordered_source_sha256
        .iter()
        .chain(&receipt.ordered_decoded_bgra_sha256)
        .chain(&receipt.ordered_descriptor_sha256)
    {
        identity.extend_from_slice(value.as_bytes());
    }
    Sha256::hex(&identity)
}

fn execution_receipt_digest(receipt: &Xmap16HubExecutionReceipt) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(receipt.schema.as_bytes());
    identity.extend_from_slice(receipt.packet_sha256.as_bytes());
    identity.extend_from_slice(receipt.upload_receipt_sha256.as_bytes());
    identity.extend_from_slice(receipt.executor_identity_sha256.as_bytes());
    identity.extend_from_slice(&receipt.submitted_quads.to_le_bytes());
    Sha256::hex(&identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AtlasFrame, CounterSpriteAsset, GuiContainerLayout, GuiControlLayout, GuiPoint, GuiSize,
        PixelRect, UvRect,
    };
    use replay_core::{RailHubExactCutoff, RailHubStateFirstVisual, RailHubVisibilityCutoffClass};
    use std::collections::BTreeSet;

    fn sprite(name: &str, path: &str, frame_count: u32) -> CounterSpriteAsset {
        CounterSpriteAsset {
            name: name.into(),
            texture_file: path.into(),
            no_of_frames: frame_count,
            frame_direction_y: false,
            texture_width: 36 * frame_count,
            texture_height: 36,
            dds_format: "DXT5".into(),
            mip_count: 1,
            frame_width: 36,
            frame_height: 36,
            frames: (0..frame_count)
                .map(|index| {
                    let u0 = index as f32 / frame_count as f32;
                    let u1 = (index + 1) as f32 / frame_count as f32;
                    AtlasFrame {
                        index,
                        rect: PixelRect {
                            x: index * 36,
                            y: 0,
                            width: 36,
                            height: 36,
                        },
                        uv: UvRect {
                            u0,
                            v0: 0.0,
                            u1,
                            v1: 1.0,
                        },
                    }
                })
                .collect(),
            source_file: "interface/supply.gfx".into(),
            source_line: 1,
        }
    }

    fn binding(role: &str, name: &str, path: &str, frames: u32) -> SupplyHubSpriteBinding {
        SupplyHubSpriteBinding {
            control_name: role.into(),
            sprite: sprite(name, path, frames),
            texture_sha256: "1".repeat(64),
            texture_bytes: 128,
        }
    }

    fn content() -> SupplyHubContentIdentity {
        SupplyHubContentIdentity {
            container: GuiContainerLayout {
                name: "supply_node_map_icon".into(),
                source_file: "interface/mapicons.gui".into(),
                source_line: 1,
                position: None,
                size: None,
                controls: vec![
                    control("background_button", -18, -18),
                    control("supply_hub_overlay_icon", -18, -17),
                    control("supply_hub_overlay_naval", -18, -17),
                ],
            },
            mapicons_gui_sha256: "2".repeat(64),
            supply_gfx_sha256: "3".repeat(64),
            background: binding(
                "background_button",
                "GFX_supply_hub_button_bg_strip",
                "background.dds",
                5,
            ),
            common_icon: binding(
                "supply_hub_overlay_icon",
                "GFX_supply_hub_icon_strip",
                "common.dds",
                5,
            ),
            naval_overlay: binding(
                "supply_hub_overlay_naval",
                "GFX_naval_supply_anchor_icon",
                "naval.dds",
                1,
            ),
        }
    }

    fn control(name: &str, x: i32, y: i32) -> GuiControlLayout {
        GuiControlLayout {
            kind: "iconType".into(),
            name: name.into(),
            source_file: "interface/mapicons.gui".into(),
            source_line: 1,
            sprite_type: None,
            quad_texture_sprite: None,
            parent_size: None,
            position: Some(GuiPoint { x, y }),
            effective_position_f32_bits: None,
            size: Some(GuiSize {
                width: 36,
                height: 36,
            }),
            slot_size: None,
            max_width: None,
            max_height: None,
            font: None,
            format: None,
            orientation: None,
            max_slots_vertical: None,
            rotation: None,
            scale: None,
            frame: None,
            center_position: None,
        }
    }

    fn row(
        world: [f32; 3],
        normal_eligible: bool,
        supply_eligible: bool,
        cutoff: f32,
    ) -> RailHubStateFirstVisual {
        RailHubStateFirstVisual {
            final_world_bits: world.map(f32::to_bits),
            final_world: world,
            background_button_frame: 2,
            common_icon_frame: 3,
            naval_overlay_visible: true,
            normal_eligible,
            supply_eligible,
            visibility_cutoff_class: RailHubVisibilityCutoffClass::Unused,
            visibility_cutoff: RailHubExactCutoff {
                value_bits: cutoff.to_bits(),
                value: cutoff,
            },
        }
    }

    fn identity_camera() -> crate::ProductionMapCamera {
        crate::ProductionMapCamera::new(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [0.0, 100.0, 0.0],
        )
        .expect("identity camera")
    }

    #[test]
    fn exact_hub_layers_offsets_and_atlas_cells_are_prepared() {
        let rows = BTreeMap::from([((7, 42, 1), row([0.0, 0.0, 0.0], true, false, 500.0))]);
        let quads = prepare_quads_from_parts(
            &rows,
            &content(),
            Xmap16HubViewerSelection {
                hour_index: 0,
                perspective_country_id: 7,
                mode: Xmap16HubPresentationMode::Normal,
            },
            &identity_camera(),
            500.0,
            200,
            100,
            1.0,
        )
        .expect("prepared hubs");
        assert_eq!(quads.len(), 3);
        assert_eq!(
            quads.iter().map(|quad| quad.layer).collect::<Vec<_>>(),
            vec![
                Xmap16HubGuiLayer::Background,
                Xmap16HubGuiLayer::CommonIcon,
                Xmap16HubGuiLayer::NavalOverlay
            ]
        );
        assert_eq!(quads[0].vertices[0].position_px, [82.0, 32.0]);
        assert_eq!(quads[1].vertices[0].position_px, [82.0, 33.0]);
        assert_eq!(quads[2].vertices[0].position_px, [82.0, 33.0]);
        assert_eq!(quads[0].vertices[0].uv, [0.2, 0.0]);
        assert_eq!(quads[0].vertices[3].uv, [0.4, 1.0]);
        assert_eq!(quads[1].vertices[0].uv, [0.4, 0.0]);
        assert_eq!(quads[1].vertices[3].uv, [0.6, 1.0]);
        assert_eq!(quads[2].vertices[0].uv, [0.0, 0.0]);
        assert_eq!(quads[2].vertices[3].uv, [1.0, 1.0]);

        let scaled = prepare_quads_from_parts(
            &rows,
            &content(),
            Xmap16HubViewerSelection {
                hour_index: 0,
                perspective_country_id: 7,
                mode: Xmap16HubPresentationMode::Normal,
            },
            &identity_camera(),
            500.0,
            200,
            100,
            2.0,
        )
        .expect("GUI-scaled hubs");
        assert_eq!(scaled[0].vertices[0].position_px, [64.0, 14.0]);
        assert_eq!(scaled[0].size_px, [72.0, 72.0]);
        assert_eq!(scaled[1].vertices[0].position_px, [64.0, 16.0]);
    }

    #[test]
    fn perspective_mode_and_hard_camera_cutoff_are_viewer_selected() {
        let rows = BTreeMap::from([
            ((7, 1, 0), row([0.0, 0.0, 0.0], true, false, 100.0)),
            ((7, 2, 0), row([0.0, 0.0, 0.0], false, true, 100.0)),
            ((8, 3, 0), row([0.0, 0.0, 0.0], false, true, 100.0)),
        ]);
        let at_cutoff = prepare_quads_from_parts(
            &rows,
            &content(),
            Xmap16HubViewerSelection {
                hour_index: 0,
                perspective_country_id: 7,
                mode: Xmap16HubPresentationMode::Supply,
            },
            &identity_camera(),
            100.0,
            200,
            100,
            1.0,
        )
        .expect("at-cutoff hubs");
        assert_eq!(
            at_cutoff
                .iter()
                .map(|quad| quad.province_id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([2])
        );
        let beyond_cutoff = prepare_quads_from_parts(
            &rows,
            &content(),
            Xmap16HubViewerSelection {
                hour_index: 0,
                perspective_country_id: 7,
                mode: Xmap16HubPresentationMode::Supply,
            },
            &identity_camera(),
            f32::from_bits(100.0f32.to_bits() + 1),
            200,
            100,
            1.0,
        )
        .expect("beyond-cutoff hubs");
        assert!(beyond_cutoff.is_empty());

        let mut strict = row([0.0, 0.0, 0.0], false, true, 100.0);
        strict.visibility_cutoff_class = RailHubVisibilityCutoffClass::SupplyIcon;
        let strict_at_equality = prepare_quads_from_parts(
            &BTreeMap::from([((7, 4, 0), strict)]),
            &content(),
            Xmap16HubViewerSelection {
                hour_index: 0,
                perspective_country_id: 7,
                mode: Xmap16HubPresentationMode::Supply,
            },
            &identity_camera(),
            100.0,
            200,
            100,
            1.0,
        )
        .expect("strict cutoff hubs");
        assert!(strict_at_equality.is_empty());
    }

    #[test]
    fn malformed_state_first_world_bits_fail_closed() {
        let mut malformed = row([0.0, 0.0, 0.0], true, true, 100.0);
        malformed.final_world_bits[0] = 1.0f32.to_bits();
        let error = prepare_quads_from_parts(
            &BTreeMap::from([((7, 1, 0), malformed)]),
            &content(),
            Xmap16HubViewerSelection {
                hour_index: 0,
                perspective_country_id: 7,
                mode: Xmap16HubPresentationMode::Normal,
            },
            &identity_camera(),
            50.0,
            200,
            100,
            1.0,
        )
        .expect_err("malformed state-first row must refuse");
        assert!(error.to_string().contains("state-first hub row"));
    }

    #[test]
    fn unsupported_subtype_fails_closed() {
        let bad_subtype = prepare_quads_from_parts(
            &BTreeMap::from([((7, 1, 2), row([0.0, 0.0, 0.0], true, true, 100.0))]),
            &content(),
            Xmap16HubViewerSelection {
                hour_index: 0,
                perspective_country_id: 7,
                mode: Xmap16HubPresentationMode::Normal,
            },
            &identity_camera(),
            50.0,
            200,
            100,
            1.0,
        )
        .expect_err("subtype outside 0/1 must refuse");
        assert!(bad_subtype.to_string().contains("state-first hub row"));
    }
}
