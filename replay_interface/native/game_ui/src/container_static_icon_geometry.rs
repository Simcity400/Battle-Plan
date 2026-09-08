//! Specific main peace-root -> own node -> content node -> direct CIcon join.
//! Prepares the original local strip
//! and native upload matrix for the shared native UI executor, never a Rect.

use crate::assets::AssetResolver;
use crate::compose::{ContainerRuntime, ContainerTransitionProfile};
use crate::container_sprite_clip::NativeClipGraphics;
use crate::container_sprite_geometry::{
    ContainerSpriteGeometryState, ContainerSpriteUvGeometry, NativeSpritePositionWrite,
    OrdinarySingleMipTextureSource,
};
use crate::container_sprite_lifetime::ContainerSpriteWidgetFactory;
use crate::container_sprite_projection::NativeSpriteUploadMatrix;
use crate::container_sprite_session::{
    AdmittedStaticIconPosition, ContainerSpriteConstructor, MaterializedContainerSprite,
};
use crate::gfx::SpriteDefinition;
use crate::gui::GuiElement;
use crate::peace_conference_ui::{PEACE_CONFERENCE_GUI, PEACE_CONFERENCE_ROOT};
use portable_content_manifest::UsedContentKey;
use snapshot::sha256::Sha256;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StaticIconGeometryRefusal {
    Source(String),
    MainRootFactory,
    LogicalParentGeneration,
    RootNotVisible,
    NativeCpuControlUnavailable,
    UnsupportedRootProperty(String),
    UnsupportedIconProperty(String),
    UnsupportedSpriteProperty(String),
    NotDirectStaticIcon,
    MaterializedDefinitionMismatch,
    AnimationBranch,
    Geometry(String),
}

/// Borrowed from the actual main-root materialization, prior to wrapper/layout
/// mutations. Private fields prevent an arbitrary neutral parent assertion.
pub(crate) struct MainPeaceRootDirectIconGeometry<'a> {
    authored_root: &'a GuiElement,
    graphics: &'a NativeClipGraphics,
    gui_source: UsedContentKey,
    _gui_payload: Arc<[u8]>,
    own_world_bits: [u32; 2],
    content_world_bits: [u32; 2],
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedMainRootDirectIconGeometry {
    pub gui_source: UsedContentKey,
    pub sprite_source: UsedContentKey,
    pub display_child_path: Vec<u32>,
    pub authored_position: AdmittedStaticIconPosition,
    pub setter_write: NativeSpritePositionWrite,
    /// Original triangle strip order. Preserve the Z=1 native vertices.
    pub local_positions: [[f32; 3]; 4],
    pub local_uvs: [[f32; 2]; 4],
    /// Frame Offset is a separate native shader operand; this does not choose
    /// a frame, shader define, texture sampler, color or animation state.
    pub uv_geometry: ContainerSpriteUvGeometry,
    pub reached_world_bits: [[u32; 4]; 4],
    pub upload: NativeSpriteUploadMatrix,
}

impl<'a> MainPeaceRootDirectIconGeometry<'a> {
    /// Only the 14184f960 -> actual CGameGui+60 -> 142246270 main-root factory
    /// may call this. That path passes optional parent zero and attaches the
    /// own node under graphics+1f0. Static factory exports are pinned in
    /// portable_engine_evidence::ordinary_sprite_root_projection; graph ancestry
    /// remains candidate in ROOT_GEOMETRY_CANDIDATES.json under the evidence run.
    /// `runtime` must be the current staged retained root evaluated against
    /// this graphics generation's LOGICAL parent size, never image dimensions.
    pub(crate) fn after_main_root_materialization(
        resolver: &AssetResolver,
        authored_root: &'a GuiElement,
        runtime: &ContainerRuntime,
        graphics: &'a NativeClipGraphics,
        native_cpu_mxcsr: Option<u32>,
    ) -> Result<Self, StaticIconGeometryRefusal> {
        use StaticIconGeometryRefusal as Reject;
        require_cpu_control(native_cpu_mxcsr)?;
        if authored_root.name != PEACE_CONFERENCE_ROOT
            || !authored_root
                .element_type
                .eq_ignore_ascii_case("containerWindowType")
        {
            return Err(Reject::MainRootFactory);
        }
        let (gui_source, payload) = verify_main_root_source(resolver, authored_root)?;
        let profile = ContainerTransitionProfile::from_root(authored_root)
            .map_err(|error| Reject::Source(error.to_string()))?;
        if !runtime.matches(&profile, graphics.logical_size) {
            return Err(Reject::LogicalParentGeneration);
        }
        if !runtime.visible {
            return Err(Reject::RootNotVisible);
        }
        for field in authored_root.raw.fields() {
            let Some(key) = field.key.as_deref() else {
                continue;
            };
            // Source scale token 5d writes type+578 -> content node+124.
            // Until the source float reader controls are admitted, only its
            // actual absent-field constructor 1.0 branch is covered here.
            // Margin evaluation belongs to the owning clip/layout parser.
            if matches!(key.to_ascii_lowercase().as_str(), "scale" | "margin") {
                return Err(Reject::UnsupportedRootProperty(key.into()));
            }
        }
        let own = NativeSpritePositionWrite::stage_container_node(
            &runtime.position(),
            &graphics.logical_size,
        )
        .map_err(|error| Reject::Geometry(format!("own node: {error:?}")))?;
        // Authenticated absence of margin is the type constructor's zero pair.
        let content =
            NativeSpritePositionWrite::stage_container_node(&[0, 0], &graphics.logical_size)
                .map_err(|error| Reject::Geometry(format!("content node: {error:?}")))?;
        // Graphics+1f0 has constructor local [0,0], scale1 and the witnessed
        // draw traversal incoming tuple [0,0,1]. Traverse all three nodes,
        // retaining each native ADDSS and multiplication by its actual scale1.
        let graphics_world = reached_unit_translation([0.0, 0.0], [0.0, 0.0]);
        let own_world = reached_unit_translation(graphics_world, own.local_translation());
        let content_world = reached_unit_translation(own_world, content.local_translation());
        Ok(Self {
            authored_root,
            graphics,
            gui_source,
            _gui_payload: payload,
            own_world_bits: own_world.map(f32::to_bits),
            content_world_bits: content_world.map(f32::to_bits),
        })
    }

    /// Own-node screen getter source; content margins must not enter it.
    pub(crate) fn own_world_translation(&self) -> [f32; 2] {
        self.own_world_bits.map(f32::from_bits)
    }

    /// Call at the materializer's actual static icon construction/setter stage,
    /// before any raster replay. Retain the result with the owning candidate.
    /// `texture` borrows the same authenticated single-mip source already
    /// admitted by the texture owner; this function does not load another view.
    pub(crate) fn prepare_direct_icon(
        &self,
        row: &MaterializedContainerSprite,
        sprite: &SpriteDefinition,
        texture: &OrdinarySingleMipTextureSource<'_>,
        retained_geometry: &ContainerSpriteGeometryState,
        current_normalization: crate::container_sprite_geometry::SpriteTextureNormalization,
    ) -> Result<PreparedMainRootDirectIconGeometry, StaticIconGeometryRefusal> {
        use StaticIconGeometryRefusal as Reject;
        let [0, ordinal] = row.display_child_path.as_slice() else {
            return Err(Reject::NotDirectStaticIcon);
        };
        let icon = self
            .authored_root
            .children
            .get(*ordinal as usize)
            .ok_or(Reject::NotDirectStaticIcon)?;
        if !icon.element_type.eq_ignore_ascii_case("iconType")
            || !icon.children.is_empty()
            || icon.name != row.element_name
            || row.constructor != ContainerSpriteConstructor::Generic
            || row.widget_factory != ContainerSpriteWidgetFactory::OrdinaryIcon2d
        {
            return Err(Reject::NotDirectStaticIcon);
        }
        let identity = row
            .identity
            .as_ref()
            .ok_or(Reject::MaterializedDefinitionMismatch)?;
        let source_name = crate::compose::primary_sprite_ref(icon)
            .ok_or(Reject::MaterializedDefinitionMismatch)?;
        if source_name != sprite.name
            || identity.sprite_name != sprite.name
            || !sprite
                .source_file
                .eq_ignore_ascii_case(&identity.definition_source.normalized_game_relative_path)
            || identity.definition_source.effective_vfs_manifest_sha256
                != self.gui_source.effective_vfs_manifest_sha256
            || row.definition_payload.is_none()
        {
            return Err(Reject::MaterializedDefinitionMismatch);
        }
        let animations = row
            .animations
            .as_ref()
            .map_err(|error| Reject::Source(error.clone()))?;
        if !animations.records.is_empty() {
            return Err(Reject::AnimationBranch);
        }
        // This allowlist is a bounded admission branch, not a claim that the
        // native parser ignores other fields. Center flags/3D/scale/rotation
        // and overrides cannot silently inherit our constructor premises.
        for field in icon.raw.fields() {
            let key = field
                .key
                .as_deref()
                .ok_or_else(|| Reject::UnsupportedIconProperty("anonymous".into()))?;
            match key.to_ascii_lowercase().as_str() {
                "name" | "position" | "spritetype" | "quadtexturesprite" | "frame" | "hide" => {}
                "orientation"
                    if field
                        .value
                        .as_scalar()
                        .is_some_and(|v| v.eq_ignore_ascii_case("upper_left")) => {}
                _ => return Err(Reject::UnsupportedIconProperty(key.into())),
            }
        }
        for field in sprite.raw.fields() {
            let key = field
                .key
                .as_deref()
                .ok_or_else(|| Reject::UnsupportedSpriteProperty("anonymous".into()))?;
            match key.to_ascii_lowercase().as_str() {
                "name" | "texturefile" | "noofframes" | "framedirectiony" | "effectfile"
                | "color" | "legacy_lazy_load" | "loadtype" => {}
                _ => return Err(Reject::UnsupportedSpriteProperty(key.into())),
            }
        }
        if retained_geometry.crop_left_top != [0, 0]
            || retained_geometry.crop_right_bottom != [0, 0]
        {
            return Err(Reject::Geometry("uncropped local strip unavailable".into()));
        }
        let position = AdmittedStaticIconPosition::parse(icon, &self.gui_source, &[*ordinal])
            .map_err(Reject::Source)?;
        let uv = ContainerSpriteUvGeometry::from_definition(
            sprite,
            texture.dimensions(),
            *retained_geometry,
            current_normalization,
        )
        .map_err(|error| Reject::Geometry(format!("native UV: {error:?}")))?;
        let physical = self.graphics.physical_size.map(|v| v as u32);
        let setter_write = position
            .stage_position_for_admitted_static_branch(&physical, &uv.native_size_at_unit_scale());
        let world_translation = reached_unit_translation(
            self.content_world_bits.map(f32::from_bits),
            setter_write.local_translation(),
        );
        let world = unit_world(world_translation);
        // Exact ordinary CSpriteType constructor142229de0 rotation+a0=0;
        // authored rotation and alternate sprite families were refused above.
        let upload = NativeSpriteUploadMatrix::ordinary_for_graphics(&world, self.graphics, 0.0)
            .map_err(|error| Reject::Geometry(format!("native upload: {error:?}")))?;
        Ok(PreparedMainRootDirectIconGeometry {
            gui_source: self.gui_source.clone(),
            sprite_source: identity.definition_source.clone(),
            display_child_path: row.display_child_path.clone(),
            authored_position: position,
            setter_write,
            local_positions: uv.uncropped_local_quad_at_unit_scale(),
            local_uvs: uv.vertex_uvs,
            uv_geometry: uv,
            reached_world_bits: world.map(|row| row.map(f32::to_bits)),
            upload,
        })
    }
}

fn reached_unit_translation(parent: [f32; 2], local: [f32; 2]) -> [f32; 2] {
    // 14229995b..983: X loads incoming then adds local; Y loads local then
    // adds incoming. The scale-minus-one branch is not taken at unit scale.
    [parent[0] + local[0], local[1] + parent[1]]
}

fn unit_world(translation: [f32; 2]) -> [[f32; 4]; 4] {
    // CGraphicalObject's constructor identity matrix plus the reached writes
    // of1422998c0: diagX/diagY=1, worldZ=0, worldX/Y=the actual tuple.
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [translation[0], translation[1], 0.0, 1.0],
    ]
}

fn require_cpu_control(source: Option<u32>) -> Result<(), StaticIconGeometryRefusal> {
    use StaticIconGeometryRefusal::NativeCpuControlUnavailable;
    if !source.is_some_and(|mode| mode & 0xffc0 == 0x1f80) {
        return Err(NativeCpuControlUnavailable);
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut host = 0_u32;
        unsafe {
            std::arch::asm!("stmxcsr [{out}]", out = in(reg) &mut host, options(nostack, preserves_flags));
        }
        if host & 0xffc0 == 0x1f80 {
            return Ok(());
        }
    }
    Err(NativeCpuControlUnavailable)
}

fn verify_main_root_source(
    resolver: &AssetResolver,
    root: &GuiElement,
) -> Result<(UsedContentKey, Arc<[u8]>), StaticIconGeometryRefusal> {
    let fail = |detail: String| StaticIconGeometryRefusal::Source(detail);
    if !resolver.has_portable_content_admission() {
        return Err(fail("authenticated main GUI source unavailable".into()));
    }
    let source = resolver
        .admitted_content_key(PEACE_CONFERENCE_GUI)
        .ok_or_else(|| fail("main GUI winner identity unavailable".into()))?
        .clone();
    source.validate().map_err(fail)?;
    let file = resolver
        .resolve_file(PEACE_CONFERENCE_GUI)
        .map_err(|e| fail(e.to_string()))?;
    let bytes = resolver.read_file(&file).map_err(|e| fail(e.to_string()))?;
    if bytes.len() as u64 != source.payload_bytes || Sha256::hex(&bytes) != source.payload_sha256 {
        return Err(fail("main GUI winner changed".into()));
    }
    if crate::paradox_text::destructive_event_count(&crate::paradox_text::diagnose_text_decode(
        &bytes,
    )) != 0
    {
        return Err(fail("main GUI source decode loss".into()));
    }
    let document = crate::paradox_text::parse_bytes(&bytes);
    if !document.recoveries.is_empty() {
        return Err(fail("main GUI parse recovery".into()));
    }
    let roots = crate::gui::parse_gui_document(&document);
    let winner = roots
        .iter()
        .rev()
        .find(|node| node.name == PEACE_CONFERENCE_ROOT);
    if winner != Some(root) {
        return Err(fail(
            "original main GUI node/source identity differs".into(),
        ));
    }
    Ok((source, bytes.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_odd_container_nodes_survive_into_icon_world_translation() {
        let own = NativeSpritePositionWrite::stage_container_node(&[7, 9], &[101, 99]).unwrap();
        let content = NativeSpritePositionWrite::stage_container_node(&[0, 0], &[101, 99]).unwrap();
        let own_world = reached_unit_translation([0.0, 0.0], own.local_translation());
        let content_world = reached_unit_translation(own_world, content.local_translation());
        assert_eq!(own_world, [6.5, -8.5]);
        assert_eq!(content_world, [6.0, -8.0]);
        let icon = NativeSpritePositionWrite::stage_static_icon(&[10, 20], &[100, 100], &[12, 4]);
        assert_eq!(
            reached_unit_translation(content_world, icon.local_translation()),
            [-34.0, 18.0]
        );
        assert_ne!(own_world, content_world);
    }

    #[test]
    fn traversal_keeps_separate_parent_additions_before_upload() {
        let parent = reached_unit_translation([16_777_216.0, 0.0], [1.0, 0.0]);
        let child = reached_unit_translation(parent, [-16_777_216.0, 0.0]);
        assert_eq!(child[0].to_bits(), 0);
        assert_eq!(
            (16_777_216.0_f32 + (1.0_f32 - 16_777_216.0_f32)).to_bits(),
            1.0_f32.to_bits()
        );
        assert_eq!(unit_world(child)[3], [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn unavailable_source_float_control_is_not_a_host_default() {
        assert_eq!(
            require_cpu_control(None),
            Err(StaticIconGeometryRefusal::NativeCpuControlUnavailable)
        );
        assert_eq!(
            require_cpu_control(Some(0x9fc0)),
            Err(StaticIconGeometryRefusal::NativeCpuControlUnavailable)
        );
    }
}

// Append to container_static_icon_geometry.rs. The caller must already bind the
// actual ordinary 2D CIcon factory, original authenticated type and no later
// frame-setter override. This is the constructor's frame operand, not history.

pub(crate) fn native_static_icon_frame(element: &crate::gui::GuiElement) -> Result<i32, String> {
    use map_viewer::authored_fixed::{assign_native_i32, NativeI32Assignment};

    if !element.element_type.eq_ignore_ascii_case("iconType") {
        return Err("native static frame requires the owning ordinary iconType".into());
    }
    let fields = element
        .raw
        .as_object()
        .ok_or("native static icon frame has no original object")?;
    // CIconType142358490 writes +134=1. Loader142358910 casefc processes each
    // source field through1424ab970/%i, so EOF retains the previous destination.
    let mut frame = 1_i32;
    for field in fields.iter().filter(|field| field.key_matches("frame")) {
        let text = field
            .value
            .as_scalar()
            .ok_or("native static icon frame is not a scalar")?;
        if !text.is_ascii() || text.contains('\0') || text.starts_with('@') {
            return Err("native static icon frame literal is unavailable".into());
        }
        if assign_native_i32(text, &mut frame) == NativeI32Assignment::MatchingFailure {
            return Err("native static icon frame integer assignment failed".into());
        }
    }
    Ok(frame)
}

/// 14222d5a0:14222d6cf..713. `native_frame_slots` is the actual wrapping u32
/// product of type+90/+94, after their own authored/texture admission. An exact
/// positive frame deliberately bypasses that bound; negative frames use effect.
pub(crate) fn native_static_icon_draw_index(
    raw_frame: i32,
    native_effect: i32,
    native_frame_slots: u32,
) -> i32 {
    let effect_index = native_effect.wrapping_sub(1);
    let mut index = if (effect_index as u32) < native_frame_slots {
        effect_index
    } else {
        0
    };
    if raw_frame == 0 {
        index = 0;
    } else if raw_frame > 0 {
        index = raw_frame - 1;
    }
    index
}

#[cfg(test)]
mod native_frame_tests {
    use super::{native_static_icon_draw_index, native_static_icon_frame};

    fn icon(fields: &str) -> crate::gui::GuiElement {
        let parsed = crate::paradox_text::parse_str(&format!("name=test {fields}"));
        assert!(parsed.recoveries.is_empty());
        crate::gui::parse_element("iconType", &parsed.root).unwrap()
    }

    #[test]
    fn absent_zero_negative_and_large_frames_keep_distinct_raw_operands() {
        for (fields, expected) in [
            ("", 1),
            ("frame=0", 0),
            ("frame=-1", -1),
            ("frame=255", 255),
            ("frame=2147483647", i32::MAX),
        ] {
            assert_eq!(
                native_static_icon_frame(&icon(fields)),
                Ok(expected),
                "{fields}"
            );
        }
    }

    #[test]
    fn ordered_duplicate_frame_assignments_reuse_shared_percent_i_and_eof() {
        for (fields, expected) in [
            (r#"frame=010 frame="""#, 8),
            (r#"frame=010 frame="" frame=0x10suffix"#, 16),
            ("frame=2147483648", i32::MIN),
            ("frame=-9223372036854775809", 0),
        ] {
            assert_eq!(
                native_static_icon_frame(&icon(fields)),
                Ok(expected),
                "{fields}"
            );
        }
    }

    #[test]
    fn unproved_scalar_and_failed_assignment_do_not_become_constructor_one() {
        for fields in ["frame={x=2}", "frame=@unknown", "frame=oops"] {
            assert!(native_static_icon_frame(&icon(fields)).is_err(), "{fields}");
        }
        let mut other = icon("frame=2");
        other.element_type = "buttonType".into();
        assert!(native_static_icon_frame(&other).is_err());
    }

    #[test]
    fn explicit_frame_zero_and_positive_override_effect_without_count_clamp() {
        for (frame, effect, slots, expected) in [
            (0, 4, 4, 0),
            (1, 4, 4, 0),
            (2, 1, 1, 1),
            (255, 0, 4, 254),
            (i32::MAX, i32::MIN, 0, i32::MAX - 1),
        ] {
            assert_eq!(
                native_static_icon_draw_index(frame, effect, slots),
                expected
            );
        }
    }

    #[test]
    fn negative_frame_uses_unsigned_effect_range_and_wrapping_subtraction() {
        for (effect, slots, expected) in [
            (0, 4, 0),
            (1, 4, 0),
            (2, 4, 1),
            (4, 4, 3),
            (5, 4, 0),
            (-1, 4, 0),
            (i32::MIN, 4, 0),
            (4, 0, 0),
            (i32::MIN, u32::MAX, i32::MAX),
        ] {
            assert_eq!(native_static_icon_draw_index(-1, effect, slots), expected);
            assert_eq!(
                native_static_icon_draw_index(i32::MIN, effect, slots),
                expected
            );
        }
    }
}
