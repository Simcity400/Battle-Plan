//! The map scene: GPU resources, the render targets and the pass order.
//!
//! One scene is built per document and reused for every frame, so the game
//! textures are uploaded once. Per-hour data arrives through the `set_*` hooks,
//! which rewrite the data textures in place and rebind the passes.

use std::io;

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;

use super::{
    // borders
    border_pass::{BorderAssets, BorderFrame, BorderPass, StraitVertex, wrap_offsets},
    borders::BorderMeshes,
    camera::{Camera, MAP_SIZE_X},
    frame::{DayNight, FrameConstants},
    // political fill
    political::Regions,
    terrain::{Grid, MARGIN, TerrainInputs, TerrainPass, grid_for},
    textures::{
        Bound, GRADIENT_HEIGHT, GRADIENT_WIDTH, MapAssets, SNOW_MUD_HEIGHT, SNOW_MUD_WIDTH,
        Samplers, data_texture, upload, write_data,
    },
    water::{WaterInputs, WaterPass},
};
// names
use super::{
    name_pass::{NamePass, Placements},
    sprites::{Batch, IconTextures, MapIcons, SpritePass},
};

/// `multi_sampling=4` in the reference profile (R1 section 0).
pub const SAMPLE_COUNT: u32 = 4;
/// Depth buffer format; the engine's exact format does not reach a pixel.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
// colour: the output stage. Egui expects gamma-encoded RGB in an unorm view,
// and the game's back buffer is not sRGB either, so the same bytes serve both.
// R1 section 4.4 inferred that from the shaders; the binary settles it. The
// (`clausewitz/pdx_gfx/gfx_dx11.cpp`), which fills a `DXGI_SWAP_CHAIN_DESC1`
// whose `Format` field is the literal `0x1c` = 28 = `DXGI_FORMAT_R8G8B8A8_UNORM`
// (not 29, the `_SRGB` twin) and hands it to `IDXGIFactory2::CreateSwapChainForHwnd`
// (vtable +0x78); the back-buffer render-target view is then created with a null
// descriptor, so it inherits that unorm format. Encoding the final colour with
// the sRGB transfer function was measured on both reference pairs and is far
// worse: the daylight pair goes from mae 16.74 to 30.24 and the night pair from
// 36.05 to 54.60.
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The frame constants plus the two viewer-only uniform blocks.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    pub frame: FrameConstants,
    /// `(origin x, origin z, step, unused)`.
    pub grid: [f32; 4],
    /// `(columns, rows, unused, unused)`.
    pub counts: [u32; 4],
    /// `pdxwater.shader`'s `vTime_HalfPixelOffset`.
    pub time_half_pixel: [f32; 4],
}

/// The multisampled attachments, their resolve target and the refraction copy.
pub struct Targets {
    color_ms: wgpu::TextureView,
    depth_ms: wgpu::TextureView,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    refraction: wgpu::Texture,
    refraction_view: wgpu::TextureView,
    size: [u32; 2],
}

impl Targets {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let extent = extent(size);
        let attachment = |label, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color_ms = attachment(
            "map colour (4x)",
            OUTPUT_FORMAT,
            SAMPLE_COUNT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let depth_ms = attachment(
            "map depth (4x)",
            DEPTH_FORMAT,
            SAMPLE_COUNT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let color = attachment(
            "map output",
            OUTPUT_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let refraction = attachment(
            "water refraction",
            OUTPUT_FORMAT,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        Self {
            color_ms: color_ms.create_view(&Default::default()),
            depth_ms: depth_ms.create_view(&Default::default()),
            color_view: color.create_view(&Default::default()),
            color,
            refraction_view: refraction.create_view(&Default::default()),
            refraction,
            size,
        }
    }

    /// The multisampled colour attachment resolving into the output texture.
    pub fn color_attachment(
        &self,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPassColorAttachment<'_> {
        wgpu::RenderPassColorAttachment {
            view: &self.color_ms,
            depth_slice: None,
            resolve_target: Some(&self.color_view),
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        }
    }

    pub fn depth_attachment(
        &self,
        load: wgpu::LoadOp<f32>,
    ) -> wgpu::RenderPassDepthStencilAttachment<'_> {
        wgpu::RenderPassDepthStencilAttachment {
            view: &self.depth_ms,
            depth_ops: Some(wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn output(&self) -> &wgpu::Texture {
        &self.color
    }

    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }
}

/// The uploaded game textures, the data textures and the two passes.
pub struct Scene {
    globals: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    samplers: Samplers,
    atlas: Bound,
    atlas_normal: Bound,
    colormap: Bound,
    city_lights: Bound,
    snow: Bound,
    mud_diffuse: Bound,
    mud_normal: Bound,
    colormap_water: Bound,
    underwater: Bound,
    lean1: Bound,
    lean2: Bound,
    water_specular: Bound,
    reflection: Bound,
    ice_diffuse: Bound,
    ice_noise: Bound,
    height_normal: Bound,
    heightmap: Bound,
    terrain_ids: Bound,
    snow_mud: Bound,
    gradient1: Bound,
    gradient2: Bound,
    secondary: Bound,
    terrain: TerrainPass,
    water: WaterPass,
    targets: Option<Targets>,
    grid: Grid,
    generation: u64,
    // borders
    /// Kept so the border pass can be built after the scene, once the document has traced it.
    globals_layout: wgpu::BindGroupLayout,
    borders: Option<BorderPass>,
    // names
    icon_textures: IconTextures,
    name_pass: NamePass,
    sprite_pass: SpritePass,
    empty_batch: Batch,
    empty_placements: Placements,
}

/// One frame's worth of arguments to [`Scene::render`].
#[derive(Clone, Copy)]
pub struct Request<'a> {
    /// Render target size in device pixels.
    pub size: [u32; 2],
    pub camera: Camera,
    pub day_night: DayNight,
    /// `vGlobalTime`, which drives the water animation.
    pub animation_time: f32,
    // names
    pub overlay: Overlay<'a>,
}

// names
/// What the two overlay passes draw on top of the world passes this frame.
///
/// Both are built by the caller, which owns the camera the labels were
/// projected with, so the overlay and the frame it is drawn into always agree.
#[derive(Clone, Copy, Default)]
pub struct Overlay<'a> {
    /// The placed country and state names, in world space.
    pub names: Option<&'a Placements>,
    /// The integrated cross-fade, country first (R2 section 4.5).
    pub layer_alpha: [f32; 2],
    /// Screen-space icon quads, in the order the caller pushed them.
    pub sprites: Option<&'a Batch>,
}

impl Scene {
    /// Uploads every game texture and builds both pipelines.
    // names: `icons` was added for the name and icon layers.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &MapAssets,
        icons: &MapIcons,
    ) -> io::Result<Self> {
        if !device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        {
            return Err(invalid(
                "this GPU does not support BC texture compression, which the game's terrain atlas, \
                 water and border textures are authored in",
            ));
        }
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map frame constants"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map frame constants"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map frame constants"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let samplers = Samplers::new(device);

        let height_normal = data_texture(
            device,
            queue,
            "HeightNormal (world_normal.bmp)",
            [assets.world_normal.width, assets.world_normal.height],
            wgpu::TextureFormat::Rgba8Unorm,
            &assets.world_normal.pixels,
        )?;
        let heightmap = data_texture(
            device,
            queue,
            "heightmap",
            [assets.heightmap.width, assets.heightmap.height],
            wgpu::TextureFormat::R8Unorm,
            assets.heightmap_texels(),
        )?;
        let terrain_ids = data_texture(
            device,
            queue,
            "TerrainIDMap",
            [assets.heightmap.width, assets.heightmap.height],
            wgpu::TextureFormat::Rgba8Unorm,
            &assets.terrain_ids,
        )?;
        let snow_mud = data_texture(
            device,
            queue,
            "SnowMudData",
            [SNOW_MUD_WIDTH, SNOW_MUD_HEIGHT],
            wgpu::TextureFormat::Rgba8Unorm,
            &vec![0u8; (SNOW_MUD_WIDTH * SNOW_MUD_HEIGHT * 4) as usize],
        )?;
        let gradient = |label| {
            data_texture(
                device,
                queue,
                label,
                [GRADIENT_WIDTH, GRADIENT_HEIGHT],
                wgpu::TextureFormat::Rgba8Unorm,
                &vec![0u8; (GRADIENT_WIDTH * GRADIENT_HEIGHT * 4) as usize],
            )
        };
        // An all-zero channel 2 makes `TranspA`, `TranspB` and `FX_Alpha.g` zero,
        // so `gradient_border_apply` leaves the colour alone and reports a bloom
        // alpha of 1: no political fill until the border worker supplies fields.
        let gradient1 = gradient("GradientBorderChannel1")?;
        let gradient2 = gradient("GradientBorderChannel2")?;
        // One transparent texel: `secondary_color_mask` multiplies the stripe by
        // the mask alpha, so a zero alpha is the whole map having no occupation.
        let secondary = data_texture(
            device,
            queue,
            "ProvinceSecondaryColorMap",
            [1, 1],
            wgpu::TextureFormat::Rgba8Unorm,
            &[0, 0, 0, 0],
        )?;

        // names
        let icon_textures = IconTextures::new(device, queue, icons)?;
        let name_pass = NamePass::new(device, &globals_layout, &icon_textures, OUTPUT_FORMAT)?;
        let sprite_pass = SpritePass::new(device, &icon_textures, OUTPUT_FORMAT)?;

        let mut scene = Self {
            globals,
            globals_bind_group,
            atlas: upload(device, queue, "atlas0", &assets.atlas)?,
            atlas_normal: upload(device, queue, "atlas_normal0", &assets.atlas_normal)?,
            colormap: upload(device, queue, "TerrainColorTint", &assets.colormap)?,
            city_lights: upload(device, queue, "CityLightsAndSnowNoise", &assets.city_lights)?,
            snow: upload(device, queue, "SnowTexture", &assets.snow)?,
            mud_diffuse: upload(device, queue, "MudDiffuseGloss", &assets.mud_diffuse)?,
            mud_normal: upload(device, queue, "MudNormalSpec", &assets.mud_normal)?,
            colormap_water: upload(device, queue, "colormap_water", &assets.colormap_water)?,
            underwater: upload(device, queue, "underwater_terrain", &assets.underwater)?,
            lean1: upload(device, queue, "LeanTexture1", &assets.lean1)?,
            lean2: upload(device, queue, "LeanTexture2", &assets.lean2)?,
            water_specular: upload(device, queue, "SpecularMap", &assets.water_specular)?,
            reflection: upload(device, queue, "ReflectionCubeMap", &assets.reflection)?,
            ice_diffuse: upload(device, queue, "IceDiffuse", &assets.ice_diffuse)?,
            ice_noise: upload(device, queue, "IceNoise", &assets.ice_noise)?,
            height_normal,
            heightmap,
            terrain_ids,
            snow_mud,
            gradient1,
            gradient2,
            secondary,
            samplers,
            terrain: TerrainPass::new(device, &globals_layout, OUTPUT_FORMAT)?,
            water: WaterPass::new(device, &globals_layout, OUTPUT_FORMAT)?,
            targets: None,
            grid: Grid {
                origin: [0.0, 0.0],
                step: 1.0,
                counts: [0, 0],
            },
            generation: 0,
            // borders
            globals_layout,
            borders: None,
            // names
            icon_textures,
            name_pass,
            sprite_pass,
            empty_batch: Batch::default(),
            empty_placements: Placements::default(),
        };
        scene.rebind_terrain(device);
        Ok(scene)
    }

    fn rebind_terrain(&mut self, device: &wgpu::Device) {
        self.terrain.rebind(
            device,
            &self.samplers,
            TerrainInputs {
                atlas: &self.atlas,
                height_normal: &self.height_normal,
                colormap: &self.colormap,
                snow: &self.snow,
                atlas_normal: &self.atlas_normal,
                terrain_ids: &self.terrain_ids,
                secondary: &self.secondary,
                snow_mud: &self.snow_mud,
                city_lights: &self.city_lights,
                mud_normal: &self.mud_normal,
                mud_diffuse: &self.mud_diffuse,
                gradient1: &self.gradient1,
                gradient2: &self.gradient2,
                underwater: &self.underwater,
                colormap_water: &self.colormap_water,
                heightmap: &self.heightmap,
            },
        );
    }

    fn rebind_water(&mut self, device: &wgpu::Device) {
        let Some(targets) = &self.targets else {
            return;
        };
        self.water.rebind(
            device,
            &self.samplers,
            WaterInputs {
                heightmap: &self.heightmap,
                lean1: &self.lean1,
                lean2: &self.lean2,
                secondary: &self.secondary,
                specular: &self.water_specular,
                refraction: &targets.refraction_view,
                ice_diffuse: &self.ice_diffuse,
                ice_noise: &self.ice_noise,
                reflection: &self.reflection,
                snow_mud: &self.snow_mud,
                gradient1: &self.gradient1,
                gradient2: &self.gradient2,
            },
        );
    }

    /// Replaces `SnowMudData` for the current hour.
    ///
    /// The channels are `r` mud now, `a` mud next hour, `b` snow now, `g` snow
    /// next hour (R1 section 6.1). The replay never interpolates, so
    /// `SnowMudFade` is 0 and only `r` and `b` are read. Row 0 is world `z = 0`.
    // The weather rasteriser that fills this belongs to a later slice; the
    // texture starts all zero, which is no snow and no mud.
    #[allow(dead_code)]
    pub fn set_snow_mud(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texels: &[u8],
        width: u32,
        height: u32,
    ) -> io::Result<()> {
        if replace(
            device,
            queue,
            "SnowMudData",
            &mut self.snow_mud,
            [width, height],
            texels,
        )? {
            self.rebind_terrain(device);
            self.rebind_water(device);
        }
        Ok(())
    }

    /// Replaces both gradient-border channel textures for the current hour.
    ///
    /// The two layers are stacked vertically inside each texture, which is why
    /// `GB_TextureHeight` is 2050 rather than the map height, and channel 2's
    /// green is the per-layer transparency (`standardfuncsgfx.fxh:903-919`).
    // `map/gradient.rs` produces exactly this layout; until it lands the
    // textures stay zero, which draws no political fill at all.
    #[allow(dead_code)]
    pub fn set_gradient_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        channel1: &[[u8; 4]],
        channel2: &[[u8; 4]],
        width: u32,
        height: u32,
    ) -> io::Result<()> {
        let mut rebind = replace(
            device,
            queue,
            "GradientBorderChannel1",
            &mut self.gradient1,
            [width, height],
            bytemuck::cast_slice(channel1),
        )?;
        rebind |= replace(
            device,
            queue,
            "GradientBorderChannel2",
            &mut self.gradient2,
            [width, height],
            bytemuck::cast_slice(channel2),
        )?;
        if rebind {
            self.rebind_terrain(device);
            self.rebind_water(device);
        }
        Ok(())
    }

    // borders
    /// Builds the border pass and uploads the traced meshes and strait ribbons.
    ///
    /// Called once per document: the geometry depends only on content, and only the class of a land
    /// edge changes with the hour ([`Scene::set_land_classes`]).
    pub fn set_borders(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &BorderAssets,
        meshes: &BorderMeshes,
        straits: &[StraitVertex],
    ) -> io::Result<()> {
        let mut pass = BorderPass::new(device, queue, &self.globals_layout, OUTPUT_FORMAT, assets)?;
        pass.upload(device, meshes, straits)?;
        self.borders = Some(pass);
        Ok(())
    }

    // borders
    /// Re-buckets the land border edges for one hour's political regions.
    pub fn set_land_classes(&mut self, device: &wgpu::Device, regions: &Regions) -> io::Result<()> {
        match &mut self.borders {
            Some(pass) => pass.set_land_classes(device, regions),
            None => Ok(()),
        }
    }

    /// Draws terrain, the sea bed, the refraction copy, the water plane and
    /// the border meshes, and after them the two overlay layers of [`Overlay`].
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: Request<'_>,
    ) -> io::Result<()> {
        let Request {
            size,
            camera,
            day_night,
            animation_time,
            overlay,
        } = request;
        camera.validate()?;
        validate_size(size, device.limits().max_texture_dimension_2d)?;
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = Some(Targets::new(device, size));
            self.generation += 1;
            self.rebind_water(device);
        }
        let targets = self.targets.as_ref().expect("created above");
        self.grid = grid_for(camera, size);
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                frame: FrameConstants::new(camera, size, day_night, animation_time),
                grid: [
                    self.grid.origin[0],
                    self.grid.origin[1],
                    self.grid.step,
                    0.0,
                ],
                counts: [self.grid.counts[0], self.grid.counts[1], 0, 0],
                // The half-pixel offset is zero: wgpu samples at texel centres.
                time_half_pixel: [animation_time, 0.0, 0.0, 0.0],
            }),
        );
        // names: both overlay passes rebuild their vertex buffers before the
        // frame's encoder starts, because they write through the queue.
        self.name_pass.upload(
            device,
            queue,
            overlay.names.unwrap_or(&self.empty_placements),
            camera,
            overlay.layer_alpha,
        );
        self.sprite_pass.upload(
            device,
            queue,
            overlay.sprites.unwrap_or(&self.empty_batch),
            size,
        );
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map") });
        self.terrain
            .draw(&mut encoder, targets, &self.globals_bind_group, self.grid);
        // `WaterRefraction` is the scene as drawn so far (`pdxwater.shader:331`).
        encoder.copy_texture_to_texture(
            targets.color.as_image_copy(),
            targets.refraction.as_image_copy(),
            extent(size),
        );
        self.water
            .draw(&mut encoder, targets, &self.globals_bind_group);
        // borders
        if let Some(borders) = &self.borders {
            borders.draw(
                &mut encoder,
                queue,
                targets,
                &self.globals_bind_group,
                BorderFrame {
                    camera_height: camera.height,
                    global_time: animation_time,
                    offsets: &wrap_offsets(camera.visible_ground(size, MARGIN), MAP_SIZE_X),
                },
            );
        }
        // names: the world-space name layer over the world passes, then the
        // screen-space icon layer last of all (R2 5.5: the icon layer wins).
        self.name_pass
            .draw(&mut encoder, targets, &self.globals_bind_group);
        self.sprite_pass.draw(&mut encoder, targets);
        queue.submit([encoder.finish()]);
        Ok(())
    }

    /// The grid the last frame drew, for reporting.
    pub fn grid(&self) -> Grid {
        self.grid
    }

    /// Egui must update its native texture registration when this changes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn targets(&self) -> Option<&Targets> {
        self.targets.as_ref()
    }

    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(Targets::output_view)
    }

    // names
    /// Name-layer vertices and icon quads the last frame drew, for reporting.
    pub fn overlay_counts(&self) -> (u32, u32) {
        (self.name_pass.vertex_count(), self.sprite_pass.quad_count())
    }

    /// Icon pages uploaded once per scene, for reporting.
    pub fn icon_page_count(&self) -> usize {
        self.icon_textures.page_count()
    }
}

/// Rewrites a data texture in place, or rebuilds it when its size changed.
///
/// Returns whether the texture object was replaced, which is the only case
/// where the bind groups that reference its view have to be rebuilt.
fn replace(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    slot: &mut Bound,
    size: [u32; 2],
    data: &[u8],
) -> io::Result<bool> {
    let expected = size[0] as usize * size[1] as usize * 4;
    if data.len() != expected {
        return Err(invalid(format!(
            "{label} needs {expected} bytes for {}x{}, got {}",
            size[0],
            size[1],
            data.len()
        )));
    }
    if [slot.texture.width(), slot.texture.height()] == size {
        write_data(queue, &slot.texture, size, 4, data);
        return Ok(false);
    }
    *slot = data_texture(
        device,
        queue,
        label,
        size,
        wgpu::TextureFormat::Rgba8Unorm,
        data,
    )?;
    Ok(true)
}

pub(crate) fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    }
}

/// A sampled-texture entry for a bind group layout.
pub(crate) fn texture_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

/// A filtering-sampler entry for a bind group layout.
pub(crate) fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// Compiles one pass, with the shared helpers concatenated ahead of it.
pub(crate) fn module(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(
            format!("{}\n{source}", include_str!("common.wgsl")).into(),
        ),
    })
}

pub(crate) fn validate_size(size: [u32; 2], limit: u32) -> io::Result<()> {
    if size
        .into_iter()
        .any(|dimension| dimension == 0 || dimension > limit)
    {
        Err(invalid(format!(
            "render dimensions {}x{} must be nonzero and no larger than the GPU limit {limit}",
            size[0], size[1]
        )))
    } else {
        Ok(())
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_globals_block_keeps_the_constant_buffer_at_its_head() {
        assert_eq!(std::mem::size_of::<FrameConstants>(), 416);
        assert_eq!(std::mem::size_of::<Globals>(), 464);
        assert_eq!(std::mem::size_of::<Globals>() % 16, 0);
    }

    #[test]
    fn zero_and_oversized_render_sizes_are_refused() {
        assert!(validate_size([0, 100], 8192).is_err());
        assert!(validate_size([100, 0], 8192).is_err());
        assert!(validate_size([9000, 100], 8192).is_err());
        assert!(validate_size([2048, 1152], 8192).is_ok());
    }
}
