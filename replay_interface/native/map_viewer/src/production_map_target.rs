//! Capture-independent opaque production-target derivation and D3D11 ownership.

use crate::{
    PreparedProductionMapFrame, PreparedRuntimeDraw, Result, ViewerOwnedMapState, Xmap10TargetPlan,
    Xmap10TargetRole,
};
use replay_core::{PreparedRenderTarget, RenderOutputState, RenderTargetDesc};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionMainSceneTargetDesc {
    pub color_attachment_ordinal: u32,
    pub color_width: u32,
    pub color_height: u32,
    pub color_format: u32,
    pub color_samples: u32,
    pub color_quality: u32,
    pub depth_attachment_ordinal: u32,
    pub depth_width: u32,
    pub depth_height: u32,
    pub depth_format: u32,
    pub depth_samples: u32,
    pub depth_quality: u32,
}

impl ProductionMainSceneTargetDesc {
    pub fn validate(&self) -> Result<()> {
        if self.color_attachment_ordinal == self.depth_attachment_ordinal {
            return Err("production color/depth attachment ordinals must be distinct".into());
        }
        if self.color_width == 0 || self.color_height == 0 {
            return Err("production color target dimensions must be nonzero".into());
        }
        if self.depth_width == 0 || self.depth_height == 0 {
            return Err("production depth target dimensions must be nonzero".into());
        }
        if self.color_width != self.depth_width || self.color_height != self.depth_height {
            return Err("production color/depth dimensions differ".into());
        }
        if self.color_samples == 0 || self.depth_samples == 0 {
            return Err("production sample counts must be nonzero".into());
        }
        if self.color_samples != self.depth_samples || self.color_quality != self.depth_quality {
            return Err("production color/depth multisample descriptors differ".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionMainSceneInputs {
    pub desc: ProductionMainSceneTargetDesc,
    pub color: RenderTargetDesc,
    pub depth: RenderTargetDesc,
    pub producer_sha256: String,
}

impl ProductionMainSceneInputs {
    pub fn validate(&self, viewer: &ViewerOwnedMapState, targets: &Xmap10TargetPlan) -> Result<()> {
        viewer.validate()?;
        targets.validate()?;
        self.desc.validate()?;
        let main = targets
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::MainScene)
            .ok_or("XMAP10 target plan omitted MainScene")?;
        let expected_desc = targets.production_main_scene_desc()?;
        if main.width != viewer.viewport_width
            || main.height != viewer.viewport_height
            || self.desc != expected_desc
            || self.color
                != render_target_desc(
                    self.desc.color_attachment_ordinal,
                    self.desc.color_width,
                    self.desc.color_height,
                    self.desc.color_format,
                    self.desc.color_samples,
                )
            || self.depth
                != render_target_desc(
                    self.desc.depth_attachment_ordinal,
                    self.desc.depth_width,
                    self.desc.depth_height,
                    self.desc.depth_format,
                    self.desc.depth_samples,
                )
            || self.producer_sha256 != main_scene_inputs_digest(&self.desc, targets)
        {
            return Err("viewer-owned XMAP10 MainScene inputs are inconsistent".into());
        }
        Ok(())
    }
}

pub fn production_main_scene_inputs_from_viewer(
    viewer: &ViewerOwnedMapState,
    targets: &Xmap10TargetPlan,
) -> Result<ProductionMainSceneInputs> {
    viewer.validate()?;
    targets.validate()?;
    let main = targets
        .ordered_targets
        .iter()
        .find(|target| target.role == Xmap10TargetRole::MainScene)
        .ok_or("XMAP10 target plan omitted MainScene")?;
    if main.width != viewer.viewport_width
        || main.height != viewer.viewport_height
        || main.dxgi_format != 10
        || main.mip_levels != 1
        || main.sample_count != 1
    {
        return Err("XMAP10 MainScene target differs from the live viewer contract".into());
    }
    let desc = targets.production_main_scene_desc()?;
    let inputs = ProductionMainSceneInputs {
        color: render_target_desc(
            desc.color_attachment_ordinal,
            desc.color_width,
            desc.color_height,
            desc.color_format,
            desc.color_samples,
        ),
        depth: render_target_desc(
            desc.depth_attachment_ordinal,
            desc.depth_width,
            desc.depth_height,
            desc.depth_format,
            desc.depth_samples,
        ),
        producer_sha256: main_scene_inputs_digest(&desc, targets),
        desc,
    };
    inputs.validate(viewer, targets)?;
    Ok(inputs)
}

fn render_target_desc(
    attachment_ordinal: u32,
    width: u32,
    height: u32,
    dxgi_format_u32: u32,
    sample_count: u32,
) -> RenderTargetDesc {
    RenderTargetDesc {
        prepared_target: PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal },
        width,
        height,
        dxgi_format_u32,
        mip_level: 0,
        sample_count,
        sample_quality: 0,
        persistent_content: None,
        exact_resource: None,
    }
}

fn main_scene_inputs_digest(
    desc: &ProductionMainSceneTargetDesc,
    targets: &Xmap10TargetPlan,
) -> String {
    Sha256::hex(
        &serde_json::to_vec(&(
            "hoi4.xmap10.viewer-main-scene-inputs.a729.v1",
            desc,
            &targets.deterministic_sha256,
        ))
        .expect("MainScene input identity serialization cannot fail"),
    )
}

pub fn production_main_scene_desc(
    frame: &PreparedProductionMapFrame,
) -> Result<Option<ProductionMainSceneTargetDesc>> {
    production_main_scene_desc_from_outputs(frame.ordered_runtime_draws.iter().map(
        |draw| match draw {
            PreparedRuntimeDraw::Particle { packet, .. } => &packet.output_state,
            PreparedRuntimeDraw::Entity { submission, .. }
            | PreparedRuntimeDraw::O04City { submission, .. }
            | PreparedRuntimeDraw::O05Building { submission, .. } => &submission.output_state,
        },
    ))
}

pub fn production_main_scene_desc_from_outputs<'a>(
    outputs: impl IntoIterator<Item = &'a RenderOutputState>,
) -> Result<Option<ProductionMainSceneTargetDesc>> {
    let mut identities =
        std::collections::BTreeMap::<PreparedRenderTarget, (&RenderTargetDesc, &'static str)>::new(
        );
    let mut candidate: Option<((u32, u32), ProductionMainSceneTargetDesc)> = None;
    for output in outputs {
        let Some(depth) = &output.depth_target else {
            continue;
        };
        let primary = output
            .ordered_color_targets
            .first()
            .ok_or("production draw with depth has no carried color target")?;
        let mut output_identities = std::collections::BTreeSet::new();
        for color in &output.ordered_color_targets {
            opaque_ordinal(&color.prepared_target)
                .ok_or("production MRT draw cannot mix the swapchain backbuffer with depth")?;
            validate_materializable_target(color)?;
            if !output_identities.insert(color.prepared_target.clone()) {
                return Err("production draw repeats a carried color-target identity".into());
            }
            merge_identity_desc(&mut identities, color, "color")?;
            if color.width != depth.width
                || color.height != depth.height
                || color.sample_count != depth.sample_count
                || color.sample_quality != depth.sample_quality
            {
                return Err(
                    "production MRT color/depth dimensions or multisample descriptors differ"
                        .into(),
                );
            }
        }
        let color_ordinal = opaque_ordinal(&primary.prepared_target)
            .expect("all depth-bearing color identities checked opaque");
        let depth_ordinal = opaque_ordinal(&depth.prepared_target)
            .ok_or("production depth target cannot be the swapchain backbuffer")?;
        validate_materializable_target(depth)?;
        if !output_identities.insert(depth.prepared_target.clone()) {
            return Err("production draw binds one opaque identity as both color and depth".into());
        }
        merge_identity_desc(&mut identities, depth, "depth")?;
        let desc = ProductionMainSceneTargetDesc {
            color_attachment_ordinal: color_ordinal,
            color_width: primary.width,
            color_height: primary.height,
            color_format: primary.dxgi_format_u32,
            color_samples: primary.sample_count,
            color_quality: primary.sample_quality,
            depth_attachment_ordinal: depth_ordinal,
            depth_width: depth.width,
            depth_height: depth.height,
            depth_format: depth.dxgi_format_u32,
            depth_samples: depth.sample_count,
            depth_quality: depth.sample_quality,
        };
        desc.validate()?;
        let key = (color_ordinal, depth_ordinal);
        if candidate.as_ref().is_none_or(|(current, _)| key < *current) {
            candidate = Some((key, desc));
        }
    }
    Ok(candidate.map(|(_, desc)| desc))
}

fn merge_identity_desc<'a>(
    identities: &mut std::collections::BTreeMap<
        PreparedRenderTarget,
        (&'a RenderTargetDesc, &'static str),
    >,
    candidate: &'a RenderTargetDesc,
    role: &'static str,
) -> Result<()> {
    if let Some((current, current_role)) = identities.get(&candidate.prepared_target) {
        if *current_role != role {
            return Err(
                "opaque render-target identity changes color/depth role within frame".into(),
            );
        }
        if *current != candidate {
            return Err(format!(
                "submitted production {role} identities/descriptors disagree across draws"
            )
            .into());
        }
    } else {
        identities.insert(candidate.prepared_target.clone(), (candidate, role));
    }
    Ok(())
}

fn validate_materializable_target(target: &RenderTargetDesc) -> Result<()> {
    if target.exact_resource.is_some() {
        return Ok(());
    }
    if target.mip_level != 0 {
        return Err("render-target schema need: nonzero mip selection requires the underlying texture mip count and exact RTV view descriptor".into());
    }
    if target.persistent_content.is_some() {
        return Err("render-target schema need: persistent initialization requires an exact texture resource descriptor and ordered subresource byte ranges with row/slice pitches".into());
    }
    Ok(())
}

fn opaque_ordinal(target: &PreparedRenderTarget) -> Option<u32> {
    match target {
        PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal } => {
            Some(*attachment_ordinal)
        }
        PreparedRenderTarget::SwapchainBackBuffer => None,
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::d3d11_device::D3dPtr;
    use std::ptr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11DepthStencilView, ID3D11Device, ID3D11RenderTargetView, ID3D11Texture2D,
        D3D11_BIND_DEPTH_STENCIL, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
        D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };

    pub struct NativeProductionMainSceneTarget {
        _color_texture: D3dPtr<ID3D11Texture2D>,
        color_rtv: D3dPtr<ID3D11RenderTargetView>,
        _depth_texture: D3dPtr<ID3D11Texture2D>,
        depth_dsv: D3dPtr<ID3D11DepthStencilView>,
        desc: ProductionMainSceneTargetDesc,
        producer_frame_sha256: String,
        allocation_generation: u64,
        resource_identity_sha256: String,
        device_identity: usize,
    }

    static NEXT_MAIN_SCENE_ALLOCATION_SERIAL: AtomicU64 = AtomicU64::new(1);

    impl NativeProductionMainSceneTarget {
        pub unsafe fn create_on_device(
            device: *mut ID3D11Device,
            desc: ProductionMainSceneTargetDesc,
            producer_frame_sha256: String,
            allocation_generation: u64,
        ) -> Result<Self> {
            desc.validate()?;
            if device.is_null() {
                return Err("cannot create MainScene on a null D3D11 device".into());
            }
            if !is_sha256(&producer_frame_sha256) || allocation_generation == 0 {
                return Err(
                    "MainScene allocation requires an authenticated producer frame and nonzero generation"
                        .into(),
                );
            }
            let allocation_serial =
                NEXT_MAIN_SCENE_ALLOCATION_SERIAL.fetch_add(1, Ordering::SeqCst);
            if allocation_serial == 0 {
                return Err("MainScene allocation serial overflowed".into());
            }
            let resource_identity_sha256 = main_scene_resource_identity_sha256(
                &desc,
                &producer_frame_sha256,
                allocation_generation,
                allocation_serial,
            )?;
            let color_desc = texture_desc(
                &desc,
                desc.color_format,
                D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
            );
            let depth_desc = texture_desc(&desc, desc.depth_format, D3D11_BIND_DEPTH_STENCIL);
            let mut color_texture = ptr::null_mut();
            hr(
                (*device).CreateTexture2D(&color_desc, ptr::null(), &mut color_texture),
                "CreateTexture2D(MainSceneColor)",
            )?;
            let color_texture = D3dPtr::from_raw(color_texture, "MainScene color texture")?;
            let mut color_rtv = ptr::null_mut();
            hr(
                (*device).CreateRenderTargetView(
                    color_texture.as_raw() as *mut _,
                    ptr::null(),
                    &mut color_rtv,
                ),
                "CreateRenderTargetView(MainSceneColor)",
            )?;
            let color_rtv = D3dPtr::from_raw(color_rtv, "MainScene color RTV")?;

            let mut depth_texture = ptr::null_mut();
            hr(
                (*device).CreateTexture2D(&depth_desc, ptr::null(), &mut depth_texture),
                "CreateTexture2D(MainSceneDepth)",
            )?;
            let depth_texture = D3dPtr::from_raw(depth_texture, "MainScene depth texture")?;
            let mut depth_dsv = ptr::null_mut();
            hr(
                (*device).CreateDepthStencilView(
                    depth_texture.as_raw() as *mut _,
                    ptr::null(),
                    &mut depth_dsv,
                ),
                "CreateDepthStencilView(MainSceneDepth)",
            )?;
            let depth_dsv = D3dPtr::from_raw(depth_dsv, "MainScene depth DSV")?;
            Ok(Self {
                _color_texture: color_texture,
                color_rtv,
                _depth_texture: depth_texture,
                depth_dsv,
                desc,
                producer_frame_sha256,
                allocation_generation,
                resource_identity_sha256,
                device_identity: device as usize,
            })
        }

        pub fn color_rtv(&self) -> *mut ID3D11RenderTargetView {
            self.color_rtv.as_raw()
        }

        pub fn color_texture(&self) -> *mut ID3D11Texture2D {
            self._color_texture.as_raw()
        }

        pub fn depth_dsv(&self) -> *mut ID3D11DepthStencilView {
            self.depth_dsv.as_raw()
        }

        pub fn depth_texture(&self) -> *mut ID3D11Texture2D {
            self._depth_texture.as_raw()
        }

        pub fn desc(&self) -> &ProductionMainSceneTargetDesc {
            &self.desc
        }

        pub fn producer_frame_sha256(&self) -> &str {
            &self.producer_frame_sha256
        }

        pub fn allocation_generation(&self) -> u64 {
            self.allocation_generation
        }

        pub fn resource_identity_sha256(&self) -> &str {
            &self.resource_identity_sha256
        }

        pub fn device_identity(&self) -> usize {
            self.device_identity
        }
    }

    fn main_scene_resource_identity_sha256(
        desc: &ProductionMainSceneTargetDesc,
        producer_frame_sha256: &str,
        allocation_generation: u64,
        allocation_serial: u64,
    ) -> Result<String> {
        Ok(Sha256::hex(&serde_json::to_vec(&(
            "hoi4.production-main-scene-allocation.v1",
            producer_frame_sha256,
            allocation_generation,
            allocation_serial,
            desc,
        ))?))
    }

    fn is_sha256(value: &str) -> bool {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && value.bytes().any(|byte| byte != b'0')
    }

    fn texture_desc(
        desc: &ProductionMainSceneTargetDesc,
        format: u32,
        bind_flags: u32,
    ) -> D3D11_TEXTURE2D_DESC {
        D3D11_TEXTURE2D_DESC {
            Width: desc.color_width,
            Height: desc.color_height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: desc.color_samples,
                Quality: desc.color_quality,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: bind_flags,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        }
    }

    fn hr(value: i32, role: &str) -> Result<()> {
        if value < 0 {
            Err(format!("{role} failed with HRESULT 0x{:08x}", value as u32).into())
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
pub use native::NativeProductionMainSceneTarget;

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        PreparedRenderTarget, RenderTargetAttachmentView, RenderTargetDesc,
        RenderTargetExactResource, RenderTargetInitialSource, RenderTargetInitialSubresource,
        RenderTargetResourceDesc, RenderTargetViewDesc, TextureResourceDimension,
    };

    #[test]
    fn descriptor_rejects_mismatched_dimensions_and_samples() {
        let mut desc = ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 4,
            color_width: 1920,
            color_height: 1080,
            color_format: 10,
            color_samples: 4,
            color_quality: 0,
            depth_attachment_ordinal: 9,
            depth_width: 1920,
            depth_height: 1080,
            depth_format: 45,
            depth_samples: 4,
            depth_quality: 0,
        };
        desc.validate().unwrap();
        desc.depth_width = 1919;
        assert!(desc
            .validate()
            .unwrap_err()
            .to_string()
            .contains("dimensions differ"));
        desc.depth_width = 1920;
        desc.depth_samples = 1;
        assert!(desc
            .validate()
            .unwrap_err()
            .to_string()
            .contains("multisample"));
        desc.depth_samples = 4;
        desc.depth_attachment_ordinal = 4;
        assert!(desc
            .validate()
            .unwrap_err()
            .to_string()
            .contains("must be distinct"));
    }

    #[test]
    fn viewer_main_scene_inputs_follow_xmap10_plan_without_recorder_outputs() {
        let viewer = ViewerOwnedMapState {
            viewport_width: 1920,
            viewport_height: 1080,
            camera: None,
            observer_country_id: None,
            map_mode_id: "viewer.map_mode.unselected".into(),
            map_mode_selection_epoch: 0,
            selected_object_ids: Vec::new(),
            input_revision: 0,
            animation_time_f32_bits: 1.0f32.to_bits(),
            frame_delta_seconds_f32_bits: 0.0f32.to_bits(),
        };
        let plan = Xmap10TargetPlan::new(1920, 1080).unwrap();
        let inputs = production_main_scene_inputs_from_viewer(&viewer, &plan).unwrap();
        assert_eq!(inputs.desc, plan.production_main_scene_desc().unwrap());
        assert_eq!(inputs.color.dxgi_format_u32, 10);
        assert_eq!(inputs.depth.dxgi_format_u32, 45);
        assert_eq!(inputs.color.sample_count, 1);
        assert_eq!(inputs.depth.sample_count, 1);
        inputs.validate(&viewer, &plan).unwrap();

        let resized = ViewerOwnedMapState {
            viewport_width: 1919,
            ..viewer
        };
        assert!(production_main_scene_inputs_from_viewer(&resized, &plan).is_err());
    }

    #[test]
    fn opaque_descriptor_preserves_exact_ordinals_and_sample_quality() {
        let color = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 4,
            },
            width: 1280,
            height: 720,
            dxgi_format_u32: 10,
            mip_level: 0,
            sample_count: 4,
            sample_quality: 7,
            persistent_content: None,
            exact_resource: None,
        };
        let depth = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 9,
            },
            width: 1280,
            height: 720,
            dxgi_format_u32: 45,
            mip_level: 0,
            sample_count: 4,
            sample_quality: 7,
            persistent_content: None,
            exact_resource: None,
        };
        let output = RenderOutputState {
            ordered_color_targets: vec![color],
            depth_target: Some(depth),
            viewport: replay_core::ViewportDesc {
                top_left_x_f32_bits: 0,
                top_left_y_f32_bits: 0,
                width_f32_bits: 1280.0f32.to_bits(),
                height_f32_bits: 720.0f32.to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1.0f32.to_bits(),
            },
            scissor: None,
        };
        let desc = production_main_scene_desc_from_outputs([&output])
            .unwrap()
            .unwrap();
        assert_eq!(desc.color_quality, 7);
        assert_eq!(desc.depth_quality, 7);
        assert_eq!(desc.color_attachment_ordinal, 4);
        assert_eq!(desc.depth_attachment_ordinal, 9);

        let mut substituted = output.clone();
        substituted.ordered_color_targets[0].prepared_target =
            PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 5,
            };
        let carried = production_main_scene_desc_from_outputs([&substituted, &output])
            .unwrap()
            .unwrap();
        assert_eq!(carried.color_attachment_ordinal, 4);
        assert_eq!(carried.depth_attachment_ordinal, 9);
    }

    #[test]
    fn depth_free_opaque_and_swapchain_outputs_do_not_gain_invented_roles() {
        for prepared_target in [
            PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 4,
            },
            PreparedRenderTarget::SwapchainBackBuffer,
        ] {
            let output = RenderOutputState {
                ordered_color_targets: vec![RenderTargetDesc {
                    prepared_target,
                    width: 1280,
                    height: 720,
                    dxgi_format_u32: 10,
                    mip_level: 0,
                    sample_count: 1,
                    sample_quality: 0,
                    persistent_content: None,
                    exact_resource: None,
                }],
                depth_target: None,
                viewport: replay_core::ViewportDesc {
                    top_left_x_f32_bits: 0,
                    top_left_y_f32_bits: 0,
                    width_f32_bits: 1280.0f32.to_bits(),
                    height_f32_bits: 720.0f32.to_bits(),
                    min_depth_f32_bits: 0,
                    max_depth_f32_bits: 1.0f32.to_bits(),
                },
                scissor: None,
            };
            assert_eq!(
                production_main_scene_desc_from_outputs([&output]).unwrap(),
                None
            );
        }
    }

    #[test]
    fn opaque_pair_rejects_unrepresentable_nonzero_mip() {
        let color = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 4,
            },
            width: 1280,
            height: 720,
            dxgi_format_u32: 10,
            mip_level: 1,
            sample_count: 1,
            sample_quality: 0,
            persistent_content: None,
            exact_resource: None,
        };
        let output = RenderOutputState {
            ordered_color_targets: vec![color],
            depth_target: Some(RenderTargetDesc {
                prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                    attachment_ordinal: 9,
                },
                width: 1280,
                height: 720,
                dxgi_format_u32: 45,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: None,
            }),
            viewport: replay_core::ViewportDesc {
                top_left_x_f32_bits: 0,
                top_left_y_f32_bits: 0,
                width_f32_bits: 1280.0f32.to_bits(),
                height_f32_bits: 720.0f32.to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1.0f32.to_bits(),
            },
            scissor: None,
        };
        assert!(production_main_scene_desc_from_outputs([&output])
            .unwrap_err()
            .to_string()
            .contains("nonzero mip"));
    }

    #[test]
    fn mrt_preserves_ordered_primary_and_validates_every_color_descriptor() {
        let primary = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 4,
            },
            width: 1280,
            height: 720,
            dxgi_format_u32: 10,
            mip_level: 0,
            sample_count: 4,
            sample_quality: 7,
            persistent_content: None,
            exact_resource: None,
        };
        let mut secondary = primary.clone();
        secondary.prepared_target = PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: 5,
        };
        secondary.dxgi_format_u32 = 28;
        let depth = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 9,
            },
            width: 1280,
            height: 720,
            dxgi_format_u32: 45,
            mip_level: 0,
            sample_count: 4,
            sample_quality: 7,
            persistent_content: None,
            exact_resource: None,
        };
        let output = RenderOutputState {
            ordered_color_targets: vec![primary, secondary],
            depth_target: Some(depth),
            viewport: replay_core::ViewportDesc {
                top_left_x_f32_bits: 0,
                top_left_y_f32_bits: 0,
                width_f32_bits: 1280.0f32.to_bits(),
                height_f32_bits: 720.0f32.to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1.0f32.to_bits(),
            },
            scissor: None,
        };
        let desc = production_main_scene_desc_from_outputs([&output])
            .unwrap()
            .unwrap();
        assert_eq!(desc.color_attachment_ordinal, 4);
        assert_eq!(desc.depth_attachment_ordinal, 9);

        let mut drifted = output;
        drifted.ordered_color_targets[1].sample_quality = 0;
        let error = production_main_scene_desc_from_outputs([&drifted])
            .unwrap_err()
            .to_string();
        assert!(error.contains("MRT color/depth"), "{error}");
    }

    #[test]
    fn persistent_target_reports_exact_missing_portable_resource_contract() {
        let mut target = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 4,
            },
            width: 64,
            height: 64,
            dxgi_format_u32: 28,
            mip_level: 0,
            sample_count: 1,
            sample_quality: 0,
            persistent_content: None,
            exact_resource: None,
        };
        target.persistent_content = Some(replay_core::UsedContentKey {
            effective_vfs_manifest_sha256: "1".repeat(64),
            winning_layer_ordinal: 0,
            layer_kind: replay_core::ContentLayerKind::Base,
            layer_id: "base".into(),
            normalized_game_relative_path: "gfx/persistent.bin".into(),
            source_sha256: "2".repeat(64),
            payload_sha256: "2".repeat(64),
            payload_bytes: 16,
        });
        let error = validate_materializable_target(&target)
            .unwrap_err()
            .to_string();
        assert!(error.contains("ordered subresource byte ranges"), "{error}");
    }

    #[test]
    fn persistent_nonzero_mip_target_with_exact_resource_is_materializable() {
        let bytes = vec![0x5a; 64];
        let target = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 14,
            },
            width: 4,
            height: 4,
            dxgi_format_u32: 28,
            mip_level: 1,
            sample_count: 1,
            sample_quality: 0,
            persistent_content: None,
            exact_resource: Some(RenderTargetExactResource {
                resource: RenderTargetResourceDesc {
                    dimension: TextureResourceDimension::Texture2d,
                    width: 8,
                    height: 8,
                    depth: 1,
                    mip_levels: 2,
                    array_size: 1,
                    dxgi_format_u32: 28,
                    sample_count: 1,
                    sample_quality: 0,
                    usage_u32: 0,
                    bind_flags_u32: 0x28,
                    cpu_access_flags_u32: 0,
                    misc_flags_u32: 0,
                },
                ordered_views: vec![RenderTargetViewDesc::RenderTarget {
                    dxgi_format_u32: 28,
                    view: RenderTargetAttachmentView::Texture2d { mip_slice: 1 },
                }],
                ordered_initial_subresources: vec![RenderTargetInitialSubresource {
                    subresource_ordinal: 1,
                    row_pitch: 16,
                    slice_pitch: 64,
                    source: RenderTargetInitialSource::Inline {
                        sha256: snapshot::sha256::Sha256::hex(&bytes),
                        bytes,
                    },
                }],
            }),
        };
        validate_materializable_target(&target).unwrap();
    }
}
