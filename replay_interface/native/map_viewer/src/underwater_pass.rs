
#[cfg(windows)]
use crate::water_law::UNDERWATER_PS_REQUIRED_SRV_SLOTS;
use crate::Result;

pub const UNDERWATER_SHADER_SOURCE: &str = "gfx/FX/pdxmap.shader";
pub const UNDERWATER_EFFECT_NAME: &str = "underwater";

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use crate::constants::{
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_USAGE_DEFAULT,
        DXGI_FORMAT_B8G8R8A8_UNORM,
    };
    use crate::shader_compile::{build_stage_hlsl, d3d_compile_stage, ShaderStage};
    use crate::{
        native_d3d11_create_depth_stencil, native_d3d11_create_render_target, native_d3d11_hr,
        native_d3d11_shader_resource_view_desc, D3dPtr, GameFileResolver,
        MapFrameOutputMergerTexture, MapFrameShaderResourceViewDesc, MapFrameTextureBinding,
        NativeD3d11DepthStencil, NativeD3d11Device, NativeD3d11PreparedDraw,
        NativeD3d11RenderTarget, WATER_REFRACTION_RESOURCE_SYMBOL,
    };
    use std::ptr;
    use winapi::shared::minwindef::UINT;
    use winapi::um::d3d11::{
        ID3D11Buffer, ID3D11DeviceContext, ID3D11PixelShader, ID3D11ShaderResourceView,
        D3D11_BIND_CONSTANT_BUFFER, D3D11_BUFFER_DESC, D3D11_CPU_ACCESS_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SUBRESOURCE_DATA,
        D3D11_USAGE_DEFAULT as D3D11_USAGE_DEFAULT_WINAPI, D3D11_USAGE_STAGING,
    };
    use winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURE2D;

    pub(crate) struct NativeD3d11UnderwaterPass {
        pub(crate) render_target: NativeD3d11RenderTarget,
        pub(crate) depth_target: NativeD3d11DepthStencil,
        pub(crate) pixel_shader: D3dPtr<ID3D11PixelShader>,
        pub(crate) srv: D3dPtr<ID3D11ShaderResourceView>,
        pub(crate) ps_bytecode: Vec<u8>,
        pub(crate) width: u32,
        pub(crate) height: u32,
        #[allow(dead_code)]
        pub(crate) seabed_diffuse_texture: D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
        pub(crate) seabed_diffuse_srv: D3dPtr<ID3D11ShaderResourceView>,
    }

    impl NativeD3d11UnderwaterPass {
        pub(crate) fn shader_resource_view(&self) -> &D3dPtr<ID3D11ShaderResourceView> {
            &self.srv
        }
        pub(crate) fn ps_bytecode(&self) -> &[u8] {
            &self.ps_bytecode
        }
        pub(crate) fn width(&self) -> u32 {
            self.width
        }
        pub(crate) fn height(&self) -> u32 {
            self.height
        }
    }

    pub(crate) fn native_d3d11_load_seabed_diffuse(
        device: &NativeD3d11Device,
        resolver: &GameFileResolver,
    ) -> Result<(
        D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
        D3dPtr<ID3D11ShaderResourceView>,
    )> {
        let path = resolver.resolve_required(
            "map/terrain/underwater_terrain_0.dds",
            "underwater seabed diffuse DDS",
        )?;
        let raw = resolver.read_required_bytes("map/terrain/underwater_terrain_0.dds", "map/terrain/underwater_terrain_0.dds").map_err(|error| {
            format!(
                "--water-full underwater pass blocked: seabed diffuse {} is not readable: {error}",
                path.display()
            )
        })?;
        if raw.len() < 128 || &raw[0..4] != b"DDS " {
            return Err(format!(
                "--water-full underwater pass blocked: {} is not a DDS file",
                path.display()
            )
            .into());
        }
        let height = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]);
        let width = u32::from_le_bytes([raw[16], raw[17], raw[18], raw[19]]);
        let mip_count = u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]).max(1);
        let fourcc = &raw[84..88];
        if fourcc != b"DXT1" {
            return Err(format!(
                "--water-full underwater pass blocked: {} fourcc {:?} is not DXT1 (BC1) — the \
                 captured engine seabed texture is BC1_SRGB; refusing a mismatched format",
                path.display(),
                fourcc
            )
            .into());
        }
        let mut offset = 128usize;
        let mut initial: Vec<D3D11_SUBRESOURCE_DATA> = Vec::with_capacity(mip_count as usize);
        let mut w = width;
        let mut h = height;
        for mip in 0..mip_count {
            let blocks_w = w.div_ceil(4).max(1);
            let blocks_h = h.div_ceil(4).max(1);
            let row_pitch = blocks_w * 8;
            let byte_len = (row_pitch * blocks_h) as usize;
            let end = offset
                .checked_add(byte_len)
                .ok_or("--water-full underwater pass blocked: seabed diffuse mip range overflow")?;
            if end > raw.len() {
                return Err(format!(
                    "--water-full underwater pass blocked: {} mip {mip} byte range {offset}..{end} \
                     exceeds file length {}",
                    path.display(),
                    raw.len()
                )
                .into());
            }
            initial.push(D3D11_SUBRESOURCE_DATA {
                pSysMem: unsafe { raw.as_ptr().add(offset) } as *const _,
                SysMemPitch: row_pitch,
                SysMemSlicePitch: byte_len as u32,
            });
            offset = end;
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        let desc = crate::MapFrameTexture2DDesc {
            width,
            height,
            mip_levels: mip_count,
            array_size: 1,
            format: crate::constants::DXGI_FORMAT_BC1_UNORM_SRGB,
            sample_count: 1,
            sample_quality: 0,
            usage: D3D11_USAGE_DEFAULT,
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            cpu_access_flags: 0,
            misc_flags: 0,
        };
        let texture = unsafe {
            crate::native_d3d11_create_texture2d_from_desc(&device.device, &desc, Some(&initial))?
        };
        let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
        let hr = unsafe {
            (*device.device.as_raw()).CreateShaderResourceView(
                texture.as_raw() as *mut _,
                ptr::null(),
                &mut srv,
            )
        };
        native_d3d11_hr(hr, "CreateShaderResourceView for seabed diffuse")?;
        let srv = unsafe { D3dPtr::from_raw(srv, "seabed diffuse SRV")? };
        Ok((texture, srv))
    }

    pub(crate) fn native_d3d11_compile_underwater_pixel_shader(
        device: &NativeD3d11Device,
        resolver: &GameFileResolver,
    ) -> Result<(D3dPtr<ID3D11PixelShader>, Vec<u8>)> {
        let hlsl = build_stage_hlsl(
            resolver,
            UNDERWATER_SHADER_SOURCE,
            UNDERWATER_EFFECT_NAME,
            ShaderStage::Pixel,
        )
        .map_err(|err| {
            format!(
                "--water-full underwater pass blocked: failed to reconstruct \
                 {UNDERWATER_SHADER_SOURCE}:{UNDERWATER_EFFECT_NAME} PixelShaderUnderwater HLSL: {err}"
            )
        })?;
        let bytecode = d3d_compile_stage(&hlsl, ShaderStage::Pixel, 0).map_err(|err| {
            format!(
                "--water-full underwater pass blocked: D3DCompile of \
                 {UNDERWATER_SHADER_SOURCE}:{UNDERWATER_EFFECT_NAME} PixelShaderUnderwater failed: {err}"
            )
        })?;
        let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
        let hr = unsafe {
            (*device.device.as_raw()).CreatePixelShader(
                bytecode.as_ptr() as *const _,
                bytecode.len(),
                ptr::null_mut(),
                &mut ps,
            )
        };
        native_d3d11_hr(
            hr,
            "CreatePixelShader for pdxmap.shader Effect underwater PixelShaderUnderwater",
        )?;
        let ps = unsafe { D3dPtr::from_raw(ps, "underwater pixel shader")? };
        Ok((ps, bytecode))
    }

    pub(crate) fn native_d3d11_validate_underwater_terrain_inputs(
        draws: &[NativeD3d11PreparedDraw],
    ) -> Result<()> {
        if draws.is_empty() {
            return Err(
                "--water-full underwater pass blocked: terrain pass has no draws to re-render \
                 for the seafloor"
                    .into(),
            );
        }
        for draw in draws {
            for (name, slot) in UNDERWATER_PS_REQUIRED_SRV_SLOTS {
                let bound = draw.shader_resources.iter().any(|resource| {
                    resource.stage == "PS"
                        && resource.slot == *slot
                        && resource.declared_name.as_deref() == Some(*name)
                });
                if !bound {
                    return Err(format!(
                        "--water-full underwater pass blocked: terrain draw {} does not bind the \
                         PixelShaderUnderwater input {name} at PS slot {slot} (required for the \
                         seafloor render); cannot reconstruct WaterRefraction",
                        draw.draw_index
                    )
                    .into());
                }
            }
        }
        Ok(())
    }

    pub(crate) struct NativeD3d11UnderwaterCb0Overrides {
        entries: Vec<(*mut ID3D11Buffer, D3dPtr<ID3D11Buffer>)>,
    }

    impl NativeD3d11UnderwaterCb0Overrides {
        pub(crate) fn patched_for(&self, live: *mut ID3D11Buffer) -> Option<*mut ID3D11Buffer> {
            self.entries
                .iter()
                .find(|(source, _)| *source == live)
                .map(|(_, patched)| patched.as_raw())
        }
    }

    unsafe fn native_d3d11_build_underwater_cb0_overrides<'a>(
        device: &NativeD3d11Device,
        draws: impl Iterator<Item = &'a NativeD3d11PreparedDraw>,
    ) -> Result<NativeD3d11UnderwaterCb0Overrides> {
        let mut entries: Vec<(*mut ID3D11Buffer, D3dPtr<ID3D11Buffer>)> = Vec::new();
        let mut saw_slot0 = false;
        for draw in draws {
            for constant_buffer in &draw.constant_buffers {
                if constant_buffer.slot != 0 {
                    continue;
                }
                saw_slot0 = true;
                let live = constant_buffer.buffer.as_raw();
                if entries.iter().any(|(source, _)| *source == live) {
                    continue;
                }
                let mut desc: D3D11_BUFFER_DESC = std::mem::zeroed();
                (*live).GetDesc(&mut desc);
                let byte_width = desc.ByteWidth as usize;
                let staging_desc = D3D11_BUFFER_DESC {
                    ByteWidth: desc.ByteWidth,
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ,
                    MiscFlags: 0,
                    StructureByteStride: 0,
                };
                let mut staging: *mut ID3D11Buffer = ptr::null_mut();
                native_d3d11_hr(
                    (*device.device.as_raw()).CreateBuffer(
                        &staging_desc,
                        ptr::null(),
                        &mut staging,
                    ),
                    "CreateBuffer for underwater scene CB0 staging snapshot",
                )?;
                let staging = D3dPtr::from_raw(staging, "underwater scene CB0 staging snapshot")?;
                (*device.context.as_raw()).CopyResource(staging.as_raw() as *mut _, live as *mut _);
                let mut mapped: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
                native_d3d11_hr(
                    (*device.context.as_raw()).Map(
                        staging.as_raw() as *mut _,
                        0,
                        D3D11_MAP_READ,
                        0,
                        &mut mapped,
                    ),
                    "Map underwater scene CB0 staging snapshot for read",
                )?;
                let mut bytes = vec![0u8; byte_width];
                std::ptr::copy_nonoverlapping(
                    mapped.pData as *const u8,
                    bytes.as_mut_ptr(),
                    byte_width,
                );
                (*device.context.as_raw()).Unmap(staging.as_raw() as *mut _, 0);
                let patched_desc = D3D11_BUFFER_DESC {
                    ByteWidth: desc.ByteWidth,
                    Usage: D3D11_USAGE_DEFAULT_WINAPI,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                    StructureByteStride: 0,
                };
                let init = D3D11_SUBRESOURCE_DATA {
                    pSysMem: bytes.as_ptr() as *const _,
                    SysMemPitch: 0,
                    SysMemSlicePitch: 0,
                };
                let mut patched: *mut ID3D11Buffer = ptr::null_mut();
                native_d3d11_hr(
                    (*device.device.as_raw()).CreateBuffer(&patched_desc, &init, &mut patched),
                    "CreateBuffer for underwater absorbed scene CB0",
                )?;
                let patched = D3dPtr::from_raw(patched, "underwater absorbed scene CB0")?;
                entries.push((live, patched));
            }
        }
        if !saw_slot0 {
            return Err(
                "--water-full underwater pass blocked: the re-rendered draws bind no slot-0 \
                 scene CB0 to absorb the underwater light rig into"
                    .into(),
            );
        }
        Ok(NativeD3d11UnderwaterCb0Overrides { entries })
    }

    fn underwater_render_target_desc(
        terrain_target: &MapFrameOutputMergerTexture,
    ) -> MapFrameOutputMergerTexture {
        let mut target = terrain_target.clone();
        target.desc.format = DXGI_FORMAT_B8G8R8A8_UNORM;
        target.desc.mip_levels = 1;
        target.desc.array_size = 1;
        target.desc.sample_count = 1;
        target.desc.sample_quality = 0;
        target.desc.usage = D3D11_USAGE_DEFAULT;
        target.desc.bind_flags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
        target.desc.cpu_access_flags = 0;
        target.desc.misc_flags = 0;
        if let Some(view) = target.view_desc.as_mut() {
            view.format = DXGI_FORMAT_B8G8R8A8_UNORM;
        }
        target.last_render_target_clear = None;
        target.pre_draw_payload = None;
        target.post_pass_payload = None;
        target
    }

    fn underwater_srv_binding(width: u32, height: u32) -> MapFrameTextureBinding {
        MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 5,
            declared_name: Some(WATER_REFRACTION_RESOURCE_SYMBOL.to_string()),
            is_null: false,
            view_desc: Some(MapFrameShaderResourceViewDesc {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                most_detailed_mip: 0,
                mip_levels: 1,
                first_array_slice: 0,
                array_size: 1,
                num_cubes: 0,
            }),
            file: None,
            byte_len: 0,
            fnv1a64: None,
            dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM,
            width,
            height,
            mip_count: 1,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: None,
            subresources: Vec::new(),
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: D3D11_USAGE_DEFAULT,
            cpu_access_flags: 0,
            misc_flags: 0,
        }
    }

    pub(crate) fn native_d3d11_prepare_underwater_pass(
        device: &NativeD3d11Device,
        resolver: &GameFileResolver,
        terrain_target: &MapFrameOutputMergerTexture,
        terrain_depth: &MapFrameOutputMergerTexture,
        terrain_draws: &[NativeD3d11PreparedDraw],
    ) -> Result<NativeD3d11UnderwaterPass> {
        native_d3d11_validate_underwater_terrain_inputs(terrain_draws)?;
        if std::env::var_os("REPLAY_DEBUG_UNDERWATER_INPUTS").is_some() {
            if let Some(draw) = terrain_draws.first() {
                unsafe {
                    for resource in &draw.shader_resources {
                        if resource.stage != "PS" || !(resource.slot == 0 || resource.slot == 2) {
                            continue;
                        }
                        let Some(view) = resource.view.as_ref() else {
                            continue;
                        };
                        let mut vd: winapi::um::d3d11::D3D11_SHADER_RESOURCE_VIEW_DESC =
                            std::mem::zeroed();
                        (*view.as_raw()).GetDesc(&mut vd);
                        let mut res: *mut winapi::um::d3d11::ID3D11Resource = std::ptr::null_mut();
                        (*view.as_raw()).GetResource(&mut res);
                        let mut tex_mips = 0u32;
                        let mut tex_w = 0u32;
                        let mut tex_h = 0u32;
                        if !res.is_null() {
                            let tex = res as *mut winapi::um::d3d11::ID3D11Texture2D;
                            let mut td: winapi::um::d3d11::D3D11_TEXTURE2D_DESC =
                                std::mem::zeroed();
                            (*tex).GetDesc(&mut td);
                            tex_mips = td.MipLevels;
                            tex_w = td.Width;
                            tex_h = td.Height;
                            (*res).Release();
                        }
                        let t2d = vd.u.Texture2D();
                        eprintln!(
                            "UNDERWATER-DEBUG srv slot {} ({:?}): view fmt {} view_mips {} most_detailed {}; texture {}x{} mips {}",
                            resource.slot,
                            resource.declared_name,
                            vd.Format,
                            t2d.MipLevels,
                            t2d.MostDetailedMip,
                            tex_w,
                            tex_h,
                            tex_mips
                        );
                    }
                    for sampler in &draw.samplers {
                        if sampler.stage == "PS" && sampler.slot == 0 {
                            let mut sd: winapi::um::d3d11::D3D11_SAMPLER_DESC = std::mem::zeroed();
                            (*sampler.state.as_raw()).GetDesc(&mut sd);
                            eprintln!(
                                "UNDERWATER-DEBUG sampler slot 0: filter {} minlod {} maxlod {} mipbias {}",
                                sd.Filter, sd.MinLOD, sd.MaxLOD, sd.MipLODBias
                            );
                        }
                    }
                }
            }
        }
        let (pixel_shader, ps_bytecode) =
            native_d3d11_compile_underwater_pixel_shader(device, resolver)?;
        let (seabed_diffuse_texture, seabed_diffuse_srv) =
            native_d3d11_load_seabed_diffuse(device, resolver)?;

        let target_desc = underwater_render_target_desc(terrain_target);
        let width = target_desc.desc.width;
        let height = target_desc.desc.height;
        let render_target = unsafe { native_d3d11_create_render_target(device, &target_desc)? };
        let depth_target = unsafe { native_d3d11_create_depth_stencil(device, terrain_depth)? };

        let srv_binding = underwater_srv_binding(width, height);
        let srv_desc = native_d3d11_shader_resource_view_desc(&srv_binding)?
            .ok_or("--water-full underwater pass blocked: underwater SRV descriptor missing")?;
        let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
        let hr = unsafe {
            (*device.device.as_raw()).CreateShaderResourceView(
                render_target.texture.as_raw() as *mut _,
                &srv_desc,
                &mut srv,
            )
        };
        native_d3d11_hr(
            hr,
            "CreateShaderResourceView for underwater WaterRefraction target",
        )?;
        let srv = unsafe { D3dPtr::from_raw(srv, "underwater WaterRefraction SRV")? };

        Ok(NativeD3d11UnderwaterPass {
            render_target,
            depth_target,
            pixel_shader,
            srv,
            ps_bytecode,
            width,
            height,
            seabed_diffuse_texture,
            seabed_diffuse_srv,
        })
    }

    pub(crate) fn native_d3d11_render_underwater_pass(
        device: &NativeD3d11Device,
        pass: &NativeD3d11UnderwaterPass,
        terrain_draws: &[NativeD3d11PreparedDraw],
    ) -> Result<()> {
        unsafe {
            let cb0_overrides =
                native_d3d11_build_underwater_cb0_overrides(device, terrain_draws.iter())?;
            let context = &device.context;
            let mut rtv = pass.render_target.view.as_raw();
            let dsv = pass.depth_target.view.as_raw();
            (*context.as_raw()).OMSetRenderTargets(1, &mut rtv, dsv);
            (*context.as_raw())
                .ClearRenderTargetView(pass.render_target.view.as_raw(), &[0.0, 0.0, 0.0, 0.0]);
            (*context.as_raw()).ClearDepthStencilView(
                pass.depth_target.view.as_raw(),
                crate::constants::D3D11_CLEAR_DEPTH | crate::constants::D3D11_CLEAR_STENCIL,
                1.0,
                0,
            );
            for draw in terrain_draws {
                native_d3d11_replay_terrain_draw_with_pixel_shader(
                    context,
                    draw,
                    &pass.render_target,
                    &pass.depth_target,
                    pass.pixel_shader.as_raw(),
                    pass.seabed_diffuse_srv.as_raw(),
                    &cb0_overrides,
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn native_d3d11_render_underwater_synthesis(
        device: &NativeD3d11Device,
        pass: &NativeD3d11UnderwaterPass,
        synthesis: &crate::NativeD3d11PreparedTerrainChunkSynthesisFrame,
        plan: &crate::TerrainChunkSynthesisPlan,
    ) -> Result<()> {
        unsafe {
            let cb0_overrides = native_d3d11_build_underwater_cb0_overrides(
                device,
                std::iter::once(&synthesis.template_draw),
            )?;
            let context = &device.context;
            let mut rtv = pass.render_target.view.as_raw();
            let dsv = pass.depth_target.view.as_raw();
            (*context.as_raw()).OMSetRenderTargets(1, &mut rtv, dsv);
            (*context.as_raw())
                .ClearRenderTargetView(pass.render_target.view.as_raw(), &[0.0, 0.0, 0.0, 0.0]);
            (*context.as_raw()).ClearDepthStencilView(
                pass.depth_target.view.as_raw(),
                crate::constants::D3D11_CLEAR_DEPTH | crate::constants::D3D11_CLEAR_STENCIL,
                1.0,
                0,
            );
            if std::env::var_os("REPLAY_DEBUG_CONSTANT_REFRACTION").is_some() {
                (*context.as_raw())
                    .ClearRenderTargetView(pass.render_target.view.as_raw(), &[0.0, 0.1, 0.2, 1.0]);
                return Ok(());
            }
            for tile in plan.tiles() {
                crate::native_d3d11_update_terrain_chunk_cb1(
                    &device.context,
                    &synthesis.template_draw,
                    tile.quad_offset_scale_is_detail,
                    tile.detail_edge_flags,
                )?;
                native_d3d11_replay_terrain_draw_with_pixel_shader_and_vertex_offsets(
                    context,
                    &synthesis.template_draw,
                    &pass.render_target,
                    &pass.depth_target,
                    pass.pixel_shader.as_raw(),
                    pass.seabed_diffuse_srv.as_raw(),
                    &[(1, tile.slot1_vertex_offset)],
                    &cb0_overrides,
                )?;
            }
        }
        Ok(())
    }

    unsafe fn native_d3d11_replay_terrain_draw_with_pixel_shader(
        context: &D3dPtr<ID3D11DeviceContext>,
        draw: &NativeD3d11PreparedDraw,
        render_target: &NativeD3d11RenderTarget,
        depth_target: &NativeD3d11DepthStencil,
        pixel_shader: *mut ID3D11PixelShader,
        seabed_diffuse_srv: *mut ID3D11ShaderResourceView,
        cb0_overrides: &NativeD3d11UnderwaterCb0Overrides,
    ) -> Result<()> {
        native_d3d11_replay_terrain_draw_with_pixel_shader_and_vertex_offsets(
            context,
            draw,
            render_target,
            depth_target,
            pixel_shader,
            seabed_diffuse_srv,
            &[],
            cb0_overrides,
        )
    }

    unsafe fn native_d3d11_replay_terrain_draw_with_pixel_shader_and_vertex_offsets(
        context: &D3dPtr<ID3D11DeviceContext>,
        draw: &NativeD3d11PreparedDraw,
        render_target: &NativeD3d11RenderTarget,
        depth_target: &NativeD3d11DepthStencil,
        pixel_shader: *mut ID3D11PixelShader,
        seabed_diffuse_srv: *mut ID3D11ShaderResourceView,
        vertex_offset_overrides: &[(u32, u32)],
        cb0_overrides: &NativeD3d11UnderwaterCb0Overrides,
    ) -> Result<()> {
        (*context.as_raw()).VSSetShader(draw.vertex_shader.as_raw(), ptr::null_mut(), 0);
        (*context.as_raw()).PSSetShader(pixel_shader, ptr::null_mut(), 0);
        (*context.as_raw()).IASetInputLayout(draw.input_layout.as_raw());
        (*context.as_raw()).IASetPrimitiveTopology(draw.topology as _);

        for vertex_buffer in &draw.vertex_buffers {
            let mut raw = vertex_buffer.buffer.as_raw();
            let mut stride = vertex_buffer.stride;
            let mut offset = vertex_offset_overrides
                .iter()
                .find(|(slot, _)| *slot == vertex_buffer.slot)
                .map(|(_, offset)| *offset)
                .unwrap_or(vertex_buffer.offset);
            (*context.as_raw()).IASetVertexBuffers(
                vertex_buffer.slot,
                1,
                &mut raw,
                &mut stride,
                &mut offset,
            );
        }
        if let Some(index_buffer) = &draw.index_buffer {
            (*context.as_raw()).IASetIndexBuffer(
                index_buffer.buffer.as_raw(),
                index_buffer.dxgi_format as _,
                index_buffer.offset,
            );
        }
        for constant_buffer in &draw.constant_buffers {
            let mut raw = constant_buffer.buffer.as_raw();
            if constant_buffer.slot == 0 {
                if let Some(patched) = cb0_overrides.patched_for(raw) {
                    raw = patched;
                }
            }
            match constant_buffer.stage.as_str() {
                "VS" => (*context.as_raw()).VSSetConstantBuffers(constant_buffer.slot, 1, &mut raw),
                "PS" => (*context.as_raw()).PSSetConstantBuffers(constant_buffer.slot, 1, &mut raw),
                stage => {
                    return Err(format!(
                        "--water-full underwater pass blocked: terrain draw {} constant buffer \
                         stage {stage} is unsupported",
                        draw.draw_index
                    )
                    .into())
                }
            }
        }
        for resource in &draw.shader_resources {
            let mut raw = resource
                .view
                .as_ref()
                .map(|view| view.as_raw())
                .unwrap_or(ptr::null_mut());
            if resource.stage == "PS" && resource.slot == 0 && !seabed_diffuse_srv.is_null() {
                raw = seabed_diffuse_srv;
            }
            match resource.stage.as_str() {
                "PS" => (*context.as_raw()).PSSetShaderResources(resource.slot, 1, &mut raw),
                stage => {
                    return Err(format!(
                        "--water-full underwater pass blocked: terrain draw {} shader resource \
                         stage {stage} is unsupported",
                        draw.draw_index
                    )
                    .into())
                }
            }
        }
        for sampler in &draw.samplers {
            let mut raw = sampler.state.as_raw();
            match sampler.stage.as_str() {
                "PS" => (*context.as_raw()).PSSetSamplers(sampler.slot, 1, &mut raw),
                stage => {
                    return Err(format!(
                        "--water-full underwater pass blocked: terrain draw {} sampler stage \
                         {stage} is unsupported",
                        draw.draw_index
                    )
                    .into())
                }
            }
        }
        (*context.as_raw()).RSSetState(
            draw.rasterizer
                .as_ref()
                .map(|state| state.as_raw())
                .unwrap_or(ptr::null_mut()),
        );
        (*context.as_raw()).OMSetBlendState(
            draw.blend
                .as_ref()
                .map(|state| state.as_raw())
                .unwrap_or(ptr::null_mut()),
            &draw.blend_factor,
            draw.sample_mask,
        );
        (*context.as_raw()).OMSetDepthStencilState(
            draw.depth_stencil
                .as_ref()
                .map(|state| state.as_raw())
                .unwrap_or(ptr::null_mut()),
            draw.stencil_ref,
        );
        (*context.as_raw()).RSSetViewports(draw.viewports.len() as UINT, draw.viewports.as_ptr());
        if !draw.scissor_rects.is_empty() {
            (*context.as_raw()).RSSetScissorRects(
                draw.scissor_rects.len() as UINT,
                draw.scissor_rects.as_ptr(),
            );
        } else {
            (*context.as_raw()).RSSetScissorRects(0, ptr::null());
        }

        let mut rtv_ptr = render_target.view.as_raw();
        let dsv_ptr = depth_target.view.as_raw();
        (*context.as_raw()).OMSetRenderTargets(1, &mut rtv_ptr, dsv_ptr);

        match draw.draw_call.as_str() {
            "DrawIndexed" => (*context.as_raw()).DrawIndexed(
                draw.draw_count,
                draw.draw_start,
                draw.base_vertex_location,
            ),
            "Draw" => (*context.as_raw()).Draw(draw.draw_count, draw.draw_start),
            "DrawIndexedInstanced" => (*context.as_raw()).DrawIndexedInstanced(
                draw.per_instance_count.ok_or_else(|| {
                    format!(
                        "--water-full underwater pass blocked: terrain draw {} is \
                         DrawIndexedInstanced but has no captured per-instance index count",
                        draw.draw_index
                    )
                })?,
                draw.instance_count.ok_or_else(|| {
                    format!(
                        "--water-full underwater pass blocked: terrain draw {} is \
                         DrawIndexedInstanced but has no captured instance count",
                        draw.draw_index
                    )
                })?,
                draw.draw_start,
                draw.base_vertex_location,
                draw.start_instance_location.ok_or_else(|| {
                    format!(
                        "--water-full underwater pass blocked: terrain draw {} is \
                         DrawIndexedInstanced but has no captured start instance location",
                        draw.draw_index
                    )
                })?,
            ),
            other => {
                return Err(format!(
                    "--water-full underwater pass blocked: terrain draw {} uses unsupported \
                     draw call {other} for the underwater re-render",
                    draw.draw_index
                )
                .into())
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) use windows_impl::{
    native_d3d11_prepare_underwater_pass, native_d3d11_render_underwater_pass,
    native_d3d11_render_underwater_synthesis,
};

#[allow(dead_code)]
const _: fn() -> Result<()> = || {
    let _ = (UNDERWATER_SHADER_SOURCE, UNDERWATER_EFFECT_NAME);
    Ok(())
};

#[cfg(test)]
mod tests {
    #[test]
    #[cfg_attr(
        not(gate_underwater_evidence_with_hoi4),
        ignore = "the captured underwater TerrainDiffuse payload at C:\\Users\\Example\\AppData\\Local\\hoi4_captures\\session_CAPFIX2_GAPS_4, plus the game install"
    )]
    fn engine_underwater_terrain_diffuse_is_the_shipped_seabed_dds() {
        let game = replay_test_gates::require_vanilla_game_root();
        let session = std::path::Path::new(
            r"C:\Users\Example\AppData\Local\hoi4_captures\session_CAPFIX2_GAPS_4",
        );
        let dds_path = game.join("map/terrain/underwater_terrain_0.dds");
        let payload_path = session.join(
            "underwater_frame_draw_map_frame_shared_payload_ps_srv_TerrainDiffuse_c09909744edaab9c.d3d11",
        );
        replay_test_gates::require_existing_file(&dds_path);
        replay_test_gates::require_existing_file(&payload_path);
        let dds = std::fs::read(&dds_path).expect("read shipped seabed DDS");
        let payload = std::fs::read(&payload_path).expect("read captured payload");
        assert_eq!(&dds[0..4], b"DDS ", "shipped file must be a DDS");
        assert_eq!(&dds[84..88], b"DXT1", "shipped seabed must be BC1/DXT1");
        let width = u32::from_le_bytes([dds[16], dds[17], dds[18], dds[19]]);
        let height = u32::from_le_bytes([dds[12], dds[13], dds[14], dds[15]]);
        let mips = u32::from_le_bytes([dds[28], dds[29], dds[30], dds[31]]);
        assert_eq!((width, height, mips), (512, 512, 10));
        let mut dds_off = 128usize;
        let mut pay_off = 0usize;
        let (mut w, mut h) = (width, height);
        for mip in 0..mips {
            let blocks_w = w.div_ceil(4).max(1) as usize;
            let blocks_h = h.div_ceil(4).max(1) as usize;
            let tight = blocks_w * 8;
            let padded = tight.max(128);
            for row in 0..blocks_h {
                let a = &dds[dds_off + row * tight..dds_off + row * tight + tight];
                let b = &payload[pay_off + row * padded..pay_off + row * padded + tight];
                assert_eq!(
                    a, b,
                    "seabed DDS mip {mip} block-row {row} must byte-match the captured engine payload"
                );
            }
            dds_off += tight * blocks_h;
            pay_off += padded * blocks_h;
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
    }
}
