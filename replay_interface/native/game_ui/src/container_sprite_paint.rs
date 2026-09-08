//! Actual native UI execution over the caller's private staged destination.

use crate::assets::AssetResolver;
use crate::container_sprite_resource_schedule::InitializedOrdinarySpriteTypeResources;
use crate::container_static_icon_geometry::PreparedMainRootDirectIconGeometry;
use crate::dds::RgbaImage;
use crate::gfx::SpriteDefinition;
use crate::gui::GuiElement;
use crate::{GameUiError, Result};
use map_viewer::native_ui::{NativeUiDrawExecutor, NativeUiTargetDesc};
use map_viewer::GameFileResolver;

/// The owning runtime lends its admitted resolver, actual device executor and
/// current target descriptor. This is execution authority, not a witness for
/// native colors, source arithmetic controls or sprite resource history.
pub struct NativeUiPaintContext<'a> {
    pub resolver: &'a GameFileResolver,
    pub executor: &'a mut dyn NativeUiDrawExecutor,
    pub target: NativeUiTargetDesc,
}

impl std::fmt::Debug for NativeUiPaintContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeUiPaintContext")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl NativeUiPaintContext<'_> {
    pub(crate) fn reborrow(&mut self) -> NativeUiPaintContext<'_> {
        NativeUiPaintContext {
            resolver: self.resolver,
            executor: &mut *self.executor,
            target: self.target.clone(),
        }
    }
}

/// These are source execution/lifetime inputs. The active source-policy audit
/// does not authorize replacing any unavailable value with viewer defaults.
#[derive(Clone, Debug)]
pub(crate) struct NativeSpriteSourceOperands {
    pub native_mxcsr: Option<u32>,
    pub mod_color: Option<[u32; 4]>,
    pub engine_defines: Option<Vec<(String, String)>>,
    pub renderer_creation_aa_request: Option<u32>,
    pub text_policy: Option<crate::container_sprite_fixed_states::NativeShaderStateTextPolicy>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedContainerSpritePaint {
    pub geometry: PreparedMainRootDirectIconGeometry,
    pub resources: InitializedOrdinarySpriteTypeResources,
    pub source: NativeSpriteSourceOperands,
    pub physical_size: [i32; 2],
    pub retained_geometry: crate::container_sprite_geometry::ContainerSpriteGeometryState,
}

impl PreparedContainerSpritePaint {
    pub(crate) fn draw_over(
        &self,
        assets: &AssetResolver,
        sprite: &SpriteDefinition,
        element: &GuiElement,
        reached: &crate::container_sprite_pass::ReachedContainerSprite,
        opacity: f32,
        context: &mut NativeUiPaintContext<'_>,
        destination: &mut RgbaImage,
    ) -> Result<bool> {
        let fail =
            |reason: String| GameUiError::parse(format!("native sprite {}: {reason}", sprite.name));
        if reached.draw.geometry != self.retained_geometry {
            return Err(fail(
                "retained geometry changed after materialization".into(),
            ));
        }
        let expected = [
            u32::try_from(self.physical_size[0]).map_err(|e| fail(e.to_string()))?,
            u32::try_from(self.physical_size[1]).map_err(|e| fail(e.to_string()))?,
        ];
        if [destination.width, destination.height]
            != [
                context.target.width as usize,
                context.target.height as usize,
            ]
        {
            return Err(fail(
                "staged destination differs from actual GPU target".into(),
            ));
        }
        let color = self
            .source
            .mod_color
            .ok_or_else(|| fail("current ModColor singleton lifetime is unavailable".into()))?;
        let raw_frame = crate::container_static_icon_geometry::native_static_icon_frame(element)
            .map_err(fail)?;
        let request = crate::runtime_graphics::ContainerSpriteRequest {
            source_identity: format!("{}:{}", sprite.source_file, element.name),
            frame_count: usize::try_from(self.geometry.uv_geometry.frame_count())
                .map_err(|e| fail(e.to_string()))?,
            frame: crate::runtime_graphics::ContainerSpriteFrame::NativeIcon(raw_frame),
            interaction: None,
            color: crate::runtime_graphics::RgbaF32(color.map(f32::from_bits)),
            clip: reached.clip,
            opacity,
            reached: Some(crate::runtime_graphics::ContainerSpriteReachedState {
                effect: reached.effect,
                gui_time: reached.gui_time,
                effect_started_at: reached.draw.draw_clock,
                admitted_animation_count: reached.definitions.records.len(),
            }),
        };
        let pipeline = crate::container_sprite_pipeline::ordinary_pipeline(
            assets,
            sprite,
            reached.effect,
            Some(true),
            self.source.renderer_creation_aa_request,
            self.source.text_policy,
        )
        .map_err(fail)?;
        // 142229480 consumes the retained graphics reset packet. An ordinary
        // window resize retains that viewport; the destination's size is a
        // separate actual target operand, checked above and by the executor.
        let viewport = portable_render_stream::ViewportDesc {
            top_left_x_f32_bits: 0.0_f32.to_bits(),
            top_left_y_f32_bits: 0.0_f32.to_bits(),
            width_f32_bits: (expected[0] as f32).to_bits(),
            height_f32_bits: (expected[1] as f32).to_bits(),
            min_depth_f32_bits: 0.0_f32.to_bits(),
            max_depth_f32_bits: 1.0_f32.to_bits(),
        };
        let scissor = portable_render_stream::ScissorDesc {
            left_i32: reached.clip.left,
            top_i32: reached.clip.top,
            right_i32: reached.clip.right,
            bottom_i32: reached.clip.bottom,
        };
        let matrix = self
            .geometry
            .upload
            .rows()
            .map(|row| row.map(f32::to_bits))
            .concat();
        let matrix: [u32; 16] = matrix.try_into().expect("four native matrix rows");
        let packet = crate::container_sprite_draw::ContainerSpriteDrawPacket::build(
            assets,
            context.resolver,
            sprite,
            &request,
            crate::container_sprite_draw::ContainerSpriteGpuInputs {
                local_positions: &self.geometry.local_positions,
                uv: &self.geometry.uv_geometry,
                world_view_projection_bits: &matrix,
                engine_defines: self.source.engine_defines.as_deref(),
                native_mxcsr: self.source.native_mxcsr,
                resources: &self.resources,
                pipeline: &pipeline,
                viewport: &viewport,
                scissor: Some(&scissor),
                target: &context.target,
            },
        )
        .map_err(fail)?;
        let admitted = packet.prepare(context.resolver).map_err(fail)?;
        context
            .executor
            .draw_over(
                &admitted,
                map_viewer::native_ui::UiRgbaSurfaceMut {
                    width: context.target.width,
                    height: context.target.height,
                    pixels: &mut destination.pixels,
                },
            )
            .map_err(|e| fail(e.to_string()))?;
        Ok(true)
    }
}

pub(crate) fn prepare_materialized_native_draws(
    assets: &AssetResolver,
    registry: &crate::gfx::SpriteRegistry,
    root: &GuiElement,
    runtime: &crate::compose::ContainerRuntime,
    materialization: &crate::container_sprite_session::ContainerSpriteMaterialization,
    lifetimes: &crate::container_sprite_lifetime::ContainerSpriteLifetimes,
    resources: &mut crate::container_sprite_resources::ViewerSpriteResources,
    graphics: std::result::Result<&crate::container_sprite_clip::NativeClipGraphics, &str>,
    force_pow2_textures: Option<bool>,
    source: NativeSpriteSourceOperands,
) -> std::collections::BTreeMap<Vec<u32>, std::result::Result<PreparedContainerSpritePaint, String>>
{
    let root_geometry = graphics.map_err(str::to_owned).and_then(|graphics|
        crate::container_static_icon_geometry::MainPeaceRootDirectIconGeometry::after_main_root_materialization(
            assets, root, runtime, graphics, source.native_mxcsr,
        ).map_err(|e| format!("native main-root geometry: {e:?}")));
    materialization.elements.iter().map(|row| {
        let prepared = (|| -> std::result::Result<PreparedContainerSpritePaint, String> {
            let root_geometry = root_geometry.as_ref().map_err(Clone::clone)?;
            let identity = row.identity.as_ref().ok_or("native materialized sprite identity unavailable")?;
            let sprite = registry.get(&identity.sprite_name).ok_or("native materialized sprite winner unavailable")?;
            let retained = lifetimes.geometry(identity).map_err(|e| format!("native retained geometry: {e:?}"))?;
            // This is the materializer's reached CIcon size operation. Type
            // initialization belongs to the earlier graphics-library phase;
            // this getter refuses a type that has no such retained authority.
            let type_resources = resources.stage_icon_size_getter(assets, sprite, force_pow2_textures)?;
            let crate::container_sprite_resource_schedule::OrdinarySpriteResourceBinding::Loaded(base) = &type_resources.base else {
                return Err(format!("native icon base resource is not loaded: {:?}", type_resources.base));
            };
            let load_options = base.effective_options();
            let texture = crate::container_sprite_geometry::OrdinarySingleMipTextureSource::admit(
                base.encoded(), Some(&load_options), Some(base.force_pow2_at_load()),
            ).map_err(|e| format!("native icon retained texture: {e:?}"))?;
            let current_normalization = force_pow2_textures
                .map(crate::container_sprite_geometry::SpriteTextureNormalization::from_native_force_pow2)
                .ok_or("native icon current UV normalization is unavailable")?;
            let geometry = root_geometry.prepare_direct_icon(row, sprite, &texture, &retained, current_normalization)
                .map_err(|e| format!("native icon geometry: {e:?}"))?;
            Ok(PreparedContainerSpritePaint {
                geometry,
                resources: type_resources,
                source: source.clone(),
                physical_size: graphics.expect("root graphics admitted").physical_size,
                retained_geometry: retained,
            })
        })();
        (row.display_child_path.clone(), prepared)
    }).collect()
}
