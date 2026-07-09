use crate::*;

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
