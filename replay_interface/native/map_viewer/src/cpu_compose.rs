use crate::*;
use std::collections::BTreeMap;

pub(crate) fn make_map_visual(
    province_ids: &[u32],
    definitions: &BTreeMap<u32, ProvinceDefinition>,
    colormap: &RgbxImage,
    water: &RgbxImage,
    terrain_atlas: &RgbxImage,
    terrain_normal_atlas: &RgbxImage,
    _underwater: &RgbxImage,
    _waterspec: &RgbxImage,
    _river_diffuse: &RgbxImage,
    _border_country: &RgbxImage,
    _border_state: &RgbxImage,
    _border_sea: &RgbxImage,
    terrain_indices: &IndexImage,
    terrain_texture_map: &BTreeMap<u8, u8>,
    heightmap: &IndexImage,
    height_normal: &RgbxImage,
    _rivers: &IndexImage,
    _province_to_state: &BTreeMap<u32, u32>,
    _state_owners: &BTreeMap<u32, String>,
) -> Vec<u32> {
    let width = heightmap.width;
    let height = heightmap.height;
    let mut out = vec![0u32; width * height];

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let province = province_ids[idx];
            let is_water = definitions
                .get(&province)
                .map(|d| d.province_type == "sea" || d.province_type == "lake")
                .unwrap_or(true);
            let base = if is_water {
                sample_scaled(water, x, y, width, height)
            } else {
                sample_scaled(colormap, x, y, width, height)
            };
            out[idx] = if is_water {
                compose_water(base)
            } else {
                let terrain_index = terrain_indices.pixels[idx];
                let texture_id = terrain_texture_map
                    .get(&terrain_index)
                    .copied()
                    .unwrap_or(255);
                apply_terrain_texture(
                    base,
                    terrain_atlas,
                    terrain_normal_atlas,
                    texture_id,
                    x,
                    y,
                    width,
                    height,
                    heightmap.pixels[idx],
                    height_normal,
                )
            };
        }
    }

    out
}

pub(crate) fn rgb_u32(rgb: [u8; 3]) -> u32 {
    (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2])
}

pub(crate) fn rgb_tuple(color: u32) -> (u8, u8, u8) {
    (
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}

pub(crate) fn sample_scaled(
    image: &RgbxImage,
    x: usize,
    y: usize,
    target_w: usize,
    target_h: usize,
) -> u32 {
    let sx = (x as f32 + 0.5) * image.width as f32 / target_w as f32 - 0.5;
    let sy = (y as f32 + 0.5) * image.height as f32 / target_h as f32 - 0.5;
    sample_xy_clamp_linear(image, sx, sy)
}

pub(crate) fn apply_terrain_texture(
    colormap: u32,
    atlas: &RgbxImage,
    normal_atlas: &RgbxImage,
    texture_id: u8,
    x: usize,
    y: usize,
    map_w: usize,
    map_h: usize,
    height: u8,
    height_normal: &RgbxImage,
) -> u32 {
    let uv2_x = (x as f32 + 0.5) / map_w as f32;
    let uv2_y = 1.0 - (y as f32 + 0.5) / map_h as f32;
    let normal = cpu_map_visual_terrain_normal(
        normal_atlas,
        texture_id,
        x,
        y,
        map_w,
        map_h,
        height_normal,
        uv2_x,
        uv2_y,
    );
    let world_pos = Vec3 {
        x: x as f32,
        y: f32::from(height) * 0.01,
        z: y as f32,
    };
    let diffuse = if texture_id == 255 {
        rgb_to_vec3(colormap)
    } else {
        let terrain = sample_terrain_texture(atlas, texture_id, x, y, map_w, map_h);
        get_overlay_vec3(
            rgb_to_vec3(terrain),
            rgb_to_vec3(colormap),
            COLORMAP_OVERLAY_STRENGTH,
        )
    };
    vec3_to_rgb_u32(apply_cpu_map_visual_lighting(
        diffuse, normal, world_pos, map_w,
    ))
}

pub(crate) fn sample_terrain_texture(
    atlas: &RgbxImage,
    texture_id: u8,
    x: usize,
    y: usize,
    map_w: usize,
    map_h: usize,
) -> u32 {
    let tile = usize::from(texture_id).min(MAP_NUM_TILES * MAP_NUM_TILES - 1);
    let index_u = (tile % MAP_NUM_TILES) as f32;
    let index_v = (tile / MAP_NUM_TILES) as f32;
    let uv2_x = (x as f32 + 0.5) / map_w as f32;
    let uv2_y = 1.0 - (y as f32 + 0.5) / map_h as f32;
    let mut tile_repeat_x = uv2_x * TERRAIN_TILE_FREQ;
    let tile_repeat_y = uv2_y * TERRAIN_TILE_FREQ;
    tile_repeat_x *= map_w as f32 / map_h as f32;
    let lod = 0.0f32;
    let mip_texels = 2.0f32.powf(ATLAS_TEXEL_POW2_EXPONENT - lod);
    sample_terrain(
        atlas,
        index_u,
        index_v,
        tile_repeat_x,
        tile_repeat_y,
        mip_texels,
    )
}

pub(crate) fn cpu_map_visual_terrain_normal(
    normal_atlas: &RgbxImage,
    texture_id: u8,
    x: usize,
    y: usize,
    map_w: usize,
    map_h: usize,
    height_normal: &RgbxImage,
    uv2_x: f32,
    uv2_y: f32,
) -> Vec3 {
    let base = sample_height_normal(height_normal, uv2_x, uv2_y);
    if texture_id == 255 {
        return base;
    }
    let normal = normal_from_rbg(sample_terrain_texture(
        normal_atlas,
        texture_id,
        x,
        y,
        map_w,
        map_h,
    ));
    compose_terrain_normal(base, normal)
}

pub(crate) fn apply_cpu_map_visual_lighting(
    diffuse: Vec3,
    normal: Vec3,
    world_pos: Vec3,
    map_w: usize,
) -> Vec3 {
    let light_source_direction = cpu_map_visual_sun_direction(world_pos, map_w as f32);
    let to_light_dir = scale_vec3(light_source_direction, -1.0);
    let ndot_l = saturate(dot_vec3(normal, to_light_dir));
    let sun_rgb = Vec3 {
        x: CPU_MAP_VISUAL_SUN_DIFFUSE_INTENSITY[0],
        y: CPU_MAP_VISUAL_SUN_DIFFUSE_INTENSITY[1],
        z: CPU_MAP_VISUAL_SUN_DIFFUSE_INTENSITY[2],
    };
    let diffuse_light = scale_vec3(sun_rgb, CPU_MAP_VISUAL_SUN_DIFFUSE_INTENSITY[3] * ndot_l);
    let ambient = ambient_light(normal, 0.0, CPU_MAP_VISUAL_NEG_FOG_MULTIPLIER);
    scale_vec3(
        mul_vec3(add_vec3(ambient, diffuse_light), diffuse),
        CPU_MAP_VISUAL_HDR_RANGE,
    )
}

pub(crate) fn cpu_map_visual_sun_direction(world_pos: Vec3, map_w: f32) -> Vec3 {
    let mut source_pos = CPU_MAP_VISUAL_VIRTUAL_SUN_POS;
    let mut second_source_pos = CPU_MAP_VISUAL_SECOND_VIRTUAL_SUN_POS;
    let half_map = map_w * 0.5;
    if world_pos.x - source_pos.x > half_map {
        source_pos.x += map_w;
    } else if world_pos.x - source_pos.x < -half_map {
        source_pos.x -= map_w;
    }
    if world_pos.x - second_source_pos.x > half_map {
        second_source_pos.x += map_w;
    } else if world_pos.x - second_source_pos.x < -half_map {
        second_source_pos.x -= map_w;
    }
    let lerp_factor = smoothstep(0.5, 1.0, (world_pos.x - source_pos.x).abs() / half_map);
    let source_pos = lerp_vec3(source_pos, second_source_pos, lerp_factor);
    normalize_vec3(sub_vec3(world_pos, source_pos))
}

// Captured from map_frame_draw020_ps_constant_buffer_slot0.bin using the
// standardfuncsgfx.fxh ConstantBuffer(0,0) layout.
pub(crate) const CPU_MAP_VISUAL_VIRTUAL_SUN_POS: Vec3 = Vec3 {
    x: 2816.0,
    y: 600.0,
    z: 848.0,
};
pub(crate) const CPU_MAP_VISUAL_SECOND_VIRTUAL_SUN_POS: Vec3 = Vec3 {
    x: 0.0,
    y: 600.0,
    z: 100.0,
};
pub(crate) const CPU_MAP_VISUAL_HDR_RANGE: f32 = 1.0;
pub(crate) const CPU_MAP_VISUAL_NEG_FOG_MULTIPLIER: f32 = 1.0;
pub(crate) const CPU_MAP_VISUAL_SUN_DIFFUSE_INTENSITY: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

pub(crate) fn sample_terrain(
    atlas: &RgbxImage,
    index_u: f32,
    index_v: f32,
    tile_repeat_x: f32,
    tile_repeat_y: f32,
    mip_texels: f32,
) -> u32 {
    let (u, v) = terrain_sample_uv(index_u, index_v, tile_repeat_x, tile_repeat_y, mip_texels);
    sample_uv_wrap_linear(atlas, u, v)
}

pub(crate) fn sample_terrain_lod(
    atlas: &RgbaMipChain,
    index_u: f32,
    index_v: f32,
    tile_repeat_x: f32,
    tile_repeat_y: f32,
    mip_texels: f32,
    lod: f32,
) -> SampledLinearRgba {
    let (u, v) = terrain_sample_uv(index_u, index_v, tile_repeat_x, tile_repeat_y, mip_texels);
    sample_uv_wrap_linear_lod_linear_rgba(atlas, u, v, lod)
}

pub(crate) fn terrain_sample_uv(
    index_u: f32,
    index_v: f32,
    tile_repeat_x: f32,
    tile_repeat_y: f32,
    mip_texels: f32,
) -> (f32, f32) {
    let texels_per_tile = mip_texels / MAP_NUM_TILES as f32;
    let tile_scale = (texels_per_tile - 1.0) / texels_per_tile;
    let u =
        ((index_u + tile_repeat_x.fract() * tile_scale) / MAP_NUM_TILES as f32) + 0.5 / mip_texels;
    let v =
        ((index_v + tile_repeat_y.fract() * tile_scale) / MAP_NUM_TILES as f32) + 0.5 / mip_texels;
    (u, v)
}

pub(crate) fn sample_uv_wrap_linear(image: &RgbxImage, u: f32, v: f32) -> u32 {
    let x = u.rem_euclid(1.0) * image.width as f32 - 0.5;
    let y = v.rem_euclid(1.0) * image.height as f32 - 0.5;
    sample_xy_wrap_linear(image, x, y)
}

#[cfg(test)]
pub(crate) fn sample_uv_wrap_linear_lod_rgba(
    chain: &RgbaMipChain,
    u: f32,
    v: f32,
    lod: f32,
) -> SampledRgba {
    let max_level = chain.levels.len().saturating_sub(1);
    let lod = lod.clamp(0.0, max_level as f32);
    let low = lod.floor() as usize;
    let high = lod.ceil() as usize;
    let low_color = sample_uv_wrap_linear_rgba(&chain.levels[low], u, v);
    if low == high {
        return low_color;
    }
    let high_color = sample_uv_wrap_linear_rgba(&chain.levels[high], u, v);
    mix_sampled_rgba(low_color, high_color, lod - low as f32)
}

pub(crate) fn sample_uv_wrap_linear_lod_linear_rgba(
    chain: &RgbaMipChain,
    u: f32,
    v: f32,
    lod: f32,
) -> SampledLinearRgba {
    let max_level = chain.levels.len().saturating_sub(1);
    let lod = lod.clamp(0.0, max_level as f32);
    let low = lod.floor() as usize;
    let high = lod.ceil() as usize;
    let low_color = sample_uv_wrap_linear_linear_rgba(&chain.levels[low], chain.color_space, u, v);
    if low == high {
        return low_color;
    }
    let high_color =
        sample_uv_wrap_linear_linear_rgba(&chain.levels[high], chain.color_space, u, v);
    mix_sampled_linear_rgba(low_color, high_color, lod - low as f32)
}

pub(crate) fn sample_uv_clamp_linear_lod_linear_rgba(
    chain: &RgbaMipChain,
    u: f32,
    v: f32,
    lod: f32,
) -> SampledLinearRgba {
    let max_level = chain.levels.len().saturating_sub(1);
    let lod = lod.clamp(0.0, max_level as f32);
    let low = lod.floor() as usize;
    let high = lod.ceil() as usize;
    let low_color = sample_uv_clamp_linear_linear_rgba(&chain.levels[low], chain.color_space, u, v);
    if low == high {
        return low_color;
    }
    let high_color =
        sample_uv_clamp_linear_linear_rgba(&chain.levels[high], chain.color_space, u, v);
    mix_sampled_linear_rgba(low_color, high_color, lod - low as f32)
}

pub(crate) fn sample_uv_wrap_linear_rgba(image: &RgbaImage, u: f32, v: f32) -> SampledRgba {
    let x = u.rem_euclid(1.0) * image.width as f32 - 0.5;
    let y = v.rem_euclid(1.0) * image.height as f32 - 0.5;
    sample_xy_wrap_linear_rgba(image, x, y)
}

pub(crate) fn sample_uv_wrap_linear_linear_rgba(
    image: &RgbaImage,
    color_space: RgbaColorSpace,
    u: f32,
    v: f32,
) -> SampledLinearRgba {
    let x = u.rem_euclid(1.0) * image.width as f32 - 0.5;
    let y = v.rem_euclid(1.0) * image.height as f32 - 0.5;
    sample_xy_wrap_linear_linear_rgba(image, color_space, x, y)
}

pub(crate) fn sample_uv_clamp_linear_linear_rgba(
    image: &RgbaImage,
    color_space: RgbaColorSpace,
    u: f32,
    v: f32,
) -> SampledLinearRgba {
    let x = u * image.width as f32 - 0.5;
    let y = v * image.height as f32 - 0.5;
    sample_xy_clamp_linear_linear_rgba(image, color_space, x, y)
}

pub(crate) fn sample_xy_wrap_linear(image: &RgbxImage, x: f32, y: f32) -> u32 {
    let x_floor = x.floor();
    let y_floor = y.floor();
    let fx = x - x_floor;
    let fy = y - y_floor;
    let x0 = wrap_index(x_floor as isize, image.width);
    let x1 = wrap_index(x_floor as isize + 1, image.width);
    let y0 = wrap_index(y_floor as isize, image.height);
    let y1 = wrap_index(y_floor as isize + 1, image.height);
    bilinear_color(
        image.pixels[y0 * image.width + x0],
        image.pixels[y0 * image.width + x1],
        image.pixels[y1 * image.width + x0],
        image.pixels[y1 * image.width + x1],
        fx,
        fy,
    )
}

pub(crate) fn sample_xy_wrap_linear_rgba(image: &RgbaImage, x: f32, y: f32) -> SampledRgba {
    let x_floor = x.floor();
    let y_floor = y.floor();
    let fx = x - x_floor;
    let fy = y - y_floor;
    let x0 = wrap_index(x_floor as isize, image.width);
    let x1 = wrap_index(x_floor as isize + 1, image.width);
    let y0 = wrap_index(y_floor as isize, image.height);
    let y1 = wrap_index(y_floor as isize + 1, image.height);
    bilinear_sampled_rgba(
        rgba_pixel(image, x0, y0),
        rgba_pixel(image, x1, y0),
        rgba_pixel(image, x0, y1),
        rgba_pixel(image, x1, y1),
        fx,
        fy,
    )
}

pub(crate) fn sample_xy_wrap_linear_linear_rgba(
    image: &RgbaImage,
    color_space: RgbaColorSpace,
    x: f32,
    y: f32,
) -> SampledLinearRgba {
    let x_floor = x.floor();
    let y_floor = y.floor();
    let fx = x - x_floor;
    let fy = y - y_floor;
    let x0 = wrap_index(x_floor as isize, image.width);
    let x1 = wrap_index(x_floor as isize + 1, image.width);
    let y0 = wrap_index(y_floor as isize, image.height);
    let y1 = wrap_index(y_floor as isize + 1, image.height);
    bilinear_sampled_linear_rgba(
        rgba_pixel_linear(image, color_space, x0, y0),
        rgba_pixel_linear(image, color_space, x1, y0),
        rgba_pixel_linear(image, color_space, x0, y1),
        rgba_pixel_linear(image, color_space, x1, y1),
        fx,
        fy,
    )
}

pub(crate) fn sample_xy_clamp_linear_linear_rgba(
    image: &RgbaImage,
    color_space: RgbaColorSpace,
    x: f32,
    y: f32,
) -> SampledLinearRgba {
    let x = x.clamp(0.0, (image.width - 1) as f32);
    let y = y.clamp(0.0, (image.height - 1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(image.width - 1);
    let y1 = (y0 + 1).min(image.height - 1);
    bilinear_sampled_linear_rgba(
        rgba_pixel_linear(image, color_space, x0, y0),
        rgba_pixel_linear(image, color_space, x1, y0),
        rgba_pixel_linear(image, color_space, x0, y1),
        rgba_pixel_linear(image, color_space, x1, y1),
        x - x0 as f32,
        y - y0 as f32,
    )
}

pub(crate) fn rgba_pixel(image: &RgbaImage, x: usize, y: usize) -> SampledRgba {
    let offset = (y * image.width + x) * 4;
    SampledRgba {
        rgb: rgb_u32([
            image.pixels[offset],
            image.pixels[offset + 1],
            image.pixels[offset + 2],
        ]),
        alpha: f32::from(image.pixels[offset + 3]) / 255.0,
    }
}

pub(crate) fn rgba_pixel_linear(
    image: &RgbaImage,
    color_space: RgbaColorSpace,
    x: usize,
    y: usize,
) -> SampledLinearRgba {
    let offset = (y * image.width + x) * 4;
    let channel = |value: u8| match color_space {
        RgbaColorSpace::Linear => f32::from(value) / 255.0,
        RgbaColorSpace::Srgb => srgb_u8_to_linear_f32(value),
    };
    SampledLinearRgba {
        rgb: Vec3 {
            x: channel(image.pixels[offset]),
            y: channel(image.pixels[offset + 1]),
            z: channel(image.pixels[offset + 2]),
        },
        alpha: f32::from(image.pixels[offset + 3]) / 255.0,
    }
}

pub(crate) fn bilinear_sampled_rgba(
    c00: SampledRgba,
    c10: SampledRgba,
    c01: SampledRgba,
    c11: SampledRgba,
    fx: f32,
    fy: f32,
) -> SampledRgba {
    let top = mix_sampled_rgba(c00, c10, fx);
    let bottom = mix_sampled_rgba(c01, c11, fx);
    mix_sampled_rgba(top, bottom, fy)
}

pub(crate) fn mix_sampled_rgba(a: SampledRgba, b: SampledRgba, t: f32) -> SampledRgba {
    SampledRgba {
        rgb: mix_color(a.rgb, b.rgb, t),
        alpha: a.alpha + (b.alpha - a.alpha) * t,
    }
}

pub(crate) fn bilinear_sampled_linear_rgba(
    c00: SampledLinearRgba,
    c10: SampledLinearRgba,
    c01: SampledLinearRgba,
    c11: SampledLinearRgba,
    fx: f32,
    fy: f32,
) -> SampledLinearRgba {
    let top = mix_sampled_linear_rgba(c00, c10, fx);
    let bottom = mix_sampled_linear_rgba(c01, c11, fx);
    mix_sampled_linear_rgba(top, bottom, fy)
}

pub(crate) fn mix_sampled_linear_rgba(
    a: SampledLinearRgba,
    b: SampledLinearRgba,
    t: f32,
) -> SampledLinearRgba {
    SampledLinearRgba {
        rgb: lerp_vec3(a.rgb, b.rgb, t),
        alpha: lerp_f32(a.alpha, b.alpha, t),
    }
}

pub(crate) fn sample_xy_clamp_linear(image: &RgbxImage, x: f32, y: f32) -> u32 {
    let x = x.clamp(0.0, (image.width - 1) as f32);
    let y = y.clamp(0.0, (image.height - 1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(image.width - 1);
    let y1 = (y0 + 1).min(image.height - 1);
    bilinear_color(
        image.pixels[y0 * image.width + x0],
        image.pixels[y0 * image.width + x1],
        image.pixels[y1 * image.width + x0],
        image.pixels[y1 * image.width + x1],
        x - x0 as f32,
        y - y0 as f32,
    )
}

pub(crate) fn wrap_index(index: isize, len: usize) -> usize {
    index.rem_euclid(len as isize) as usize
}

pub(crate) fn bilinear_color(c00: u32, c10: u32, c01: u32, c11: u32, fx: f32, fy: f32) -> u32 {
    let top = mix_color(c00, c10, fx);
    let bottom = mix_color(c01, c11, fx);
    mix_color(top, bottom, fy)
}

pub(crate) fn mix_color(a: u32, b: u32, t: f32) -> u32 {
    let (ar, ag, ab) = rgb_tuple(a);
    let (br, bg, bb) = rgb_tuple(b);
    rgb_u32([
        lerp_channel(ar, br, t),
        lerp_channel(ag, bg, t),
        lerp_channel(ab, bb, t),
    ])
}

pub(crate) fn lerp_channel(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

pub(crate) fn get_overlay_vec3(color: Vec3, overlay: Vec3, overlay_percent: f32) -> Vec3 {
    Vec3 {
        x: get_overlay_channel(color.x, overlay.x, overlay_percent),
        y: get_overlay_channel(color.y, overlay.y, overlay_percent),
        z: get_overlay_channel(color.z, overlay.z, overlay_percent),
    }
}

pub(crate) fn get_overlay_channel(color: f32, overlay: f32, overlay_percent: f32) -> f32 {
    let color_gamma = color.clamp(0.0, 1.0).powf(0.45);
    let overlay_gamma = overlay.clamp(0.0, 1.0).powf(0.45);
    let res_gamma = if overlay_gamma < 0.5 {
        2.0 * overlay_gamma * color_gamma
    } else {
        1.0 - 2.0 * (1.0 - overlay_gamma) * (1.0 - color_gamma)
    };
    let res_linear = res_gamma.powf(2.2);
    lerp_f32(color, res_linear, overlay_percent)
}

pub(crate) fn rgb_to_vec3(color: u32) -> Vec3 {
    let (r, g, b) = rgb_tuple(color);
    Vec3 {
        x: f32::from(r) / 255.0,
        y: f32::from(g) / 255.0,
        z: f32::from(b) / 255.0,
    }
}

pub(crate) fn vec3_to_rgb_u32(color: Vec3) -> u32 {
    rgb_u32([
        (color.x.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.y.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.z.clamp(0.0, 1.0) * 255.0).round() as u8,
    ])
}

pub(crate) fn srgb_u8_to_linear_f32(value: u8) -> f32 {
    let encoded = f32::from(value) / 255.0;
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn compose_water(base: u32) -> u32 {
    scale_color(base, WATER_COLOR_LIGHTNESS)
}

pub(crate) fn scale_color(color: u32, shade: f32) -> u32 {
    let (r, g, b) = rgb_tuple(color);
    rgb_u32([
        (f32::from(r) * shade).round().clamp(0.0, 255.0) as u8,
        (f32::from(g) * shade).round().clamp(0.0, 255.0) as u8,
        (f32::from(b) * shade).round().clamp(0.0, 255.0) as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn srgb_mip_sampler_decodes_rgb_before_filtering_and_preserves_alpha() {
        let chain = RgbaMipChain {
            levels: vec![RgbaImage {
                width: 1,
                height: 1,
                pixels: vec![128, 64, 255, 128],
            }],
            color_space: RgbaColorSpace::Srgb,
        };

        let sample = sample_uv_wrap_linear_lod_linear_rgba(&chain, 0.25, 0.75, 0.0);

        assert_close(sample.rgb.x, srgb_u8_to_linear_f32(128));
        assert_close(sample.rgb.y, srgb_u8_to_linear_f32(64));
        assert_close(sample.rgb.z, 1.0);
        assert_close(sample.alpha, 128.0 / 255.0);
    }

    #[test]
    fn terrain_lod_sampler_linearly_blends_shipped_mip_levels() {
        let chain = RgbaMipChain {
            levels: vec![
                RgbaImage {
                    width: 1,
                    height: 1,
                    pixels: vec![255, 0, 0, 64],
                },
                RgbaImage {
                    width: 1,
                    height: 1,
                    pixels: vec![0, 255, 0, 128],
                },
                RgbaImage {
                    width: 1,
                    height: 1,
                    pixels: vec![0, 0, 255, 255],
                },
            ],
            color_space: RgbaColorSpace::Linear,
        };
        let half = sample_uv_wrap_linear_lod_rgba(&chain, 0.25, 0.75, 0.5);
        assert_eq!(half.rgb, rgb_u32([128, 128, 0]));
        assert_close(half.alpha, 96.0 / 255.0);

        let quarter = sample_uv_wrap_linear_lod_rgba(&chain, 0.25, 0.75, 1.25);
        assert_eq!(quarter.rgb, rgb_u32([0, 191, 64]));
        assert_close(quarter.alpha, 159.75 / 255.0);

        let clamped = sample_uv_wrap_linear_lod_rgba(&chain, 0.25, 0.75, 99.0);
        assert_eq!(clamped.rgb, rgb_u32([0, 0, 255]));
        assert_close(clamped.alpha, 1.0);
    }

    #[test]
    fn cpu_map_visual_lighting_flat_normal_matches_shader_formula() {
        let diffuse = Vec3 {
            x: 0.5,
            y: 0.25,
            z: 0.125,
        };
        let lit = apply_cpu_map_visual_lighting(
            diffuse,
            Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            Vec3 {
                x: CPU_MAP_VISUAL_VIRTUAL_SUN_POS.x,
                y: 0.0,
                z: CPU_MAP_VISUAL_VIRTUAL_SUN_POS.z,
            },
            5632,
        );

        assert_vec3_close(
            lit,
            mul_vec3(
                add_vec3(
                    DAY_AMBIENT_MAP_POS_Y,
                    Vec3 {
                        x: 1.0,
                        y: 1.0,
                        z: 1.0,
                    },
                ),
                diffuse,
            ),
        );
    }

    #[test]
    fn cpu_map_visual_lighting_uses_overlay_linear_result_before_lighting() {
        let terrain = Vec3 {
            x: 0.25,
            y: 0.5,
            z: 0.75,
        };
        let colormap = Vec3 {
            x: 0.8,
            y: 0.2,
            z: 0.6,
        };
        let normal = Vec3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        let world_pos = Vec3 {
            x: CPU_MAP_VISUAL_VIRTUAL_SUN_POS.x,
            y: 0.0,
            z: CPU_MAP_VISUAL_VIRTUAL_SUN_POS.z,
        };

        let shader_order = apply_cpu_map_visual_lighting(
            get_overlay_vec3(terrain, colormap, COLORMAP_OVERLAY_STRENGTH),
            normal,
            world_pos,
            5632,
        );
        let wrong_order = get_overlay_vec3(
            apply_cpu_map_visual_lighting(terrain, normal, world_pos, 5632),
            colormap,
            COLORMAP_OVERLAY_STRENGTH,
        );

        assert_vec3_close(
            shader_order,
            mul_vec3(
                add_vec3(
                    DAY_AMBIENT_MAP_POS_Y,
                    Vec3 {
                        x: 1.0,
                        y: 1.0,
                        z: 1.0,
                    },
                ),
                get_overlay_vec3(terrain, colormap, COLORMAP_OVERLAY_STRENGTH),
            ),
        );
        assert!((shader_order.x - wrong_order.x).abs() > 0.001);
    }
}
