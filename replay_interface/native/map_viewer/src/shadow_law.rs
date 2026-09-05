
use crate::mod_resolver::GameFileResolver;
use crate::Result;


pub const SHADOW_FXH_SOURCE: &str = "gfx/FX/shadow.fxh";
pub const SHADOWBLUR_SHADER_SOURCE: &str = "gfx/FX/shadowblur.shader";
pub const PDXMAP_SHADER_SOURCE: &str = "gfx/FX/pdxmap.shader";
pub const CONSTANTS_FXH_SOURCE: &str = "gfx/FX/constants.fxh";
pub const GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";


pub const SHADOW_MAP_SIZE: f32 = 2048.0;
pub const PCF_TEXEL_SIZE: f32 = 0.5 / SHADOW_MAP_SIZE;
pub const PCF_BIAS: f32 = 0.001;
pub const PCF_SHADOWED_VALUE: f32 = 0.1;
pub const PCF_LIT_VALUE: f32 = 1.0;
pub const PCF_OCCLUSION_MAX: f32 = 0.99;
pub const PCF_TAP_COUNT: usize = 5;


pub const SHADOW_WEIGHT_TERRAIN: f32 = 0.7;
pub const SHADOW_WEIGHT_MAP: f32 = 0.7;
pub const SHADOW_WEIGHT_BORDER: f32 = 0.7;
pub const SHADOW_WEIGHT_WATER: f32 = 0.5;
pub const SHADOW_WEIGHT_RIVER: f32 = 0.4;
pub const SHADOW_WEIGHT_TREE: f32 = 0.7;

pub const TERRAIN_SHADOW_FLOOR: f32 = 0.1;


pub const DRAW_SHADOWS_CUTOFF: f32 = 400.0;
pub const DRAW_SHADOWS_FADE_LENGTH: f32 = 50.0;


pub const SHADOW_BLUR_CAMERA_DISTANCE: f32 = f32::from_bits(0x442f0000);
pub const SHADOW_BLUR_OFFSET_1_BASE: f32 = f32::from_bits(0x3fb13b14);
pub const SHADOW_BLUR_OFFSET_2_BASE: f32 = f32::from_bits(0x404ec4ec);
pub const SHADOW_BLUR_WEIGHTS: [f32; 3] = [
    f32::from_bits(0x3e6879c6),
    f32::from_bits(0x3ea1e718),
    f32::from_bits(0x3d8fe9dc),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShadowBlurConstantBuffers {
    pub horizontal_f32_bits: [u32; 8],
    pub vertical_f32_bits: [u32; 8],
}

fn shadow_blur_cb_for_dimension(camera_factor: f32, dimension: u32) -> [u32; 8] {
    let dimension = dimension as f32;
    let first_product = SHADOW_BLUR_OFFSET_1_BASE * camera_factor;
    let second_product = SHADOW_BLUR_OFFSET_2_BASE * camera_factor;
    let offset_1 = (SHADOW_BLUR_OFFSET_1_BASE - first_product) / dimension;
    let offset_2 = (SHADOW_BLUR_OFFSET_2_BASE - second_product) / dimension;
    [
        0.0f32.to_bits(),
        offset_1.to_bits(),
        offset_2.to_bits(),
        SHADOW_BLUR_WEIGHTS[0].to_bits(),
        SHADOW_BLUR_WEIGHTS[1].to_bits(),
        SHADOW_BLUR_WEIGHTS[2].to_bits(),
        1.0f32.to_bits(),
        1.0f32.to_bits(),
    ]
}

pub fn shadow_blur_constant_buffers(
    camera_y: f32,
    width: u32,
    height: u32,
) -> Result<ShadowBlurConstantBuffers> {
    if !camera_y.is_finite() {
        return Err("shadow blur blocked: camera_y is not finite".into());
    }
    let camera_factor = (camera_y / SHADOW_BLUR_CAMERA_DISTANCE).min(1.0);
    shadow_blur_constant_buffers_from_factor(camera_factor, width, height)
}

pub fn shadow_blur_constant_buffers_from_factor(
    camera_factor: f32,
    width: u32,
    height: u32,
) -> Result<ShadowBlurConstantBuffers> {
    if !camera_factor.is_finite() || !(0.0..=1.0).contains(&camera_factor) {
        return Err("shadow blur blocked: camera factor is outside exact [0,1]".into());
    }
    if width == 0 || height == 0 {
        return Err("shadow blur blocked: screen-shadow target dimensions must be nonzero".into());
    }
    Ok(ShadowBlurConstantBuffers {
        horizontal_f32_bits: shadow_blur_cb_for_dimension(camera_factor, width),
        vertical_f32_bits: shadow_blur_cb_for_dimension(camera_factor, height),
    })
}

pub const LIGHT_SHADOW_DIRECTION: [f32; 3] = [-5.0, -8.0, 5.0];


pub const SHADOW_FIT_GRID: f32 = 10.0;
pub const SHADOW_FIT_EYE_MARGIN: f32 = 30.0;

pub type ShadowMat4 = [[f32; 4]; 4];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShadowLightKind {
    Sun,
    Moon,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowFrameInput {
    pub map_focus: [f32; 2],
    pub camera_x: f32,
    pub map_width: f32,
    pub primary_sun: [f32; 3],
    pub primary_moon: [f32; 3],
    pub fit_center: [f32; 3],
    pub fit_points: [[f32; 3]; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectedShadowLight {
    pub kind: ShadowLightKind,
    pub position: [f32; 3],
    pub wrapped_x: f32,
    pub direction: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowFrameMatrices {
    pub selected_light: SelectedShadowLight,
    pub light_view_projection: ShadowMat4,
    pub shadow_map_texture_matrix: ShadowMat4,
}

fn finite3(value: [f32; 3]) -> bool {
    value.into_iter().all(f32::is_finite)
}

fn flip_sign(value: f32) -> f32 {
    f32::from_bits(value.to_bits() ^ 0x8000_0000)
}

fn normalize3_a729(value: [f32; 3]) -> Result<[f32; 3]> {
    let xx = value[0] * value[0];
    let yy = value[1] * value[1];
    let xy = xx + yy;
    let zz = value[2] * value[2];
    let length_squared = xy + zz;
    if !length_squared.is_finite() {
        return Err("shadow L02 blocked: normalize length is non-finite".into());
    }
    let length = length_squared.sqrt();
    if length == 0.0 {
        return Ok([0.0, 0.0, 0.0]);
    }
    let normalized = [value[0] / length, value[1] / length, value[2] / length];
    if !finite3(normalized) {
        return Err("shadow L02 blocked: normalized vector is non-finite".into());
    }
    Ok(normalized)
}

pub fn select_shadow_light(input: &ShadowFrameInput) -> Result<SelectedShadowLight> {
    if !input.map_focus.into_iter().all(f32::is_finite)
        || !input.camera_x.is_finite()
        || !input.map_width.is_finite()
        || input.map_width <= 0.0
        || !finite3(input.primary_sun)
        || !finite3(input.primary_moon)
    {
        return Err(
            "shadow L02 blocked: selected-light inputs are incomplete or non-finite".into(),
        );
    }

    let sun_dx = input.map_focus[0] - input.primary_sun[0];
    let sun_dz = input.map_focus[1] - input.primary_sun[2];
    let sun_xx = sun_dx * sun_dx;
    let sun_zz = sun_dz * sun_dz;
    let sun_distance_squared = sun_xx + sun_zz;

    let moon_dx = input.map_focus[0] - input.primary_moon[0];
    let moon_dz = input.map_focus[1] - input.primary_moon[2];
    let moon_xx = moon_dx * moon_dx;
    let moon_zz = moon_dz * moon_dz;
    let moon_distance_squared = moon_xx + moon_zz;
    if !sun_distance_squared.is_finite() || !moon_distance_squared.is_finite() {
        return Err("shadow L02 blocked: selected-light distance overflowed binary32".into());
    }

    let (kind, position) = if moon_distance_squared <= sun_distance_squared {
        (ShadowLightKind::Moon, input.primary_moon)
    } else {
        (ShadowLightKind::Sun, input.primary_sun)
    };

    let half_width = input.map_width * 0.5;
    let candidate_x = if half_width <= input.camera_x {
        position[0] + input.map_width
    } else {
        position[0] - input.map_width
    };
    if !candidate_x.is_finite() {
        return Err("shadow L02 blocked: selected-light wrap overflowed binary32".into());
    }
    let candidate_distance = (input.camera_x - candidate_x).abs();
    let original_distance = (input.camera_x - position[0]).abs();
    let wrapped_x = if candidate_distance < original_distance {
        candidate_x
    } else {
        position[0]
    };

    let pre_normalize = [
        input.map_focus[0] - wrapped_x,
        flip_sign(position[1]),
        input.map_focus[1] - position[2],
    ];
    if !finite3(pre_normalize) {
        return Err("shadow L02 blocked: selected-light vector is non-finite".into());
    }
    let direction = normalize3_a729(pre_normalize)?;
    Ok(SelectedShadowLight {
        kind,
        position,
        wrapped_x,
        direction,
    })
}

fn trunc_grid_a729(value: f32) -> Result<f32> {
    if !value.is_finite() || value < i32::MIN as f32 || value > 2_147_483_520.0 {
        return Err("shadow L02 blocked: 10-unit snap input is outside signed-i32 range".into());
    }
    Ok(((value as i32 / 10) * 10) as f32)
}

pub(crate) fn look_at_row_vector_a729(eye: [f32; 3], target: [f32; 3]) -> Result<ShadowMat4> {
    let forward = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let right = [forward[2], 0.0, flip_sign(forward[0])];
    let up = [
        right[2] * forward[1],
        right[0] * forward[2] - right[2] * forward[0],
        flip_sign(right[0] * forward[1]),
    ];
    let forward = normalize3_a729(forward)?;
    let right = normalize3_a729(right)?;
    let up = normalize3_a729(up)?;
    if forward == [0.0; 3] || right == [0.0; 3] || up == [0.0; 3] {
        return Err(
            "shadow L02 blocked: light direction is degenerate for world-up look-at".into(),
        );
    }

    let right_xy = eye[1] * right[1] + eye[0] * right[0];
    let right_dot = right_xy + eye[2] * right[2];
    let up_xy = eye[1] * up[1] + eye[0] * up[0];
    let up_dot = up_xy + eye[2] * up[2];
    let forward_xy = eye[0] * forward[0] + eye[1] * forward[1];
    let forward_dot = forward_xy + eye[2] * forward[2];
    let matrix = [
        [right[0], up[0], forward[0], 0.0],
        [right[1], up[1], forward[1], 0.0],
        [right[2], up[2], forward[2], 0.0],
        [
            flip_sign(right_dot),
            flip_sign(up_dot),
            flip_sign(forward_dot),
            1.0,
        ],
    ];
    if matrix.iter().flatten().any(|value| !value.is_finite()) {
        return Err("shadow L02 blocked: light view matrix is non-finite".into());
    }
    Ok(matrix)
}

fn transform_point_row_vector(point: [f32; 3], matrix: ShadowMat4) -> [f32; 3] {
    let x_y = matrix[1][0] * point[1] + matrix[0][0] * point[0];
    let x_z = x_y + matrix[2][0] * point[2];
    let y_y = matrix[1][1] * point[1] + matrix[0][1] * point[0];
    let y_z = y_y + matrix[2][1] * point[2];
    let z_y = matrix[1][2] * point[1] + matrix[0][2] * point[0];
    let z_z = z_y + matrix[2][2] * point[2];
    [x_z + matrix[3][0], y_z + matrix[3][1], z_z + matrix[3][2]]
}

pub fn compose_shadow_map_texture_matrix(light_view_projection: ShadowMat4) -> Result<ShadowMat4> {
    if light_view_projection
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err("shadow L02 blocked: LightViewProjectionRaw is non-finite".into());
    }
    let mut output = [[0.0; 4]; 4];
    for row in 0..4 {
        output[row][0] = light_view_projection[row][0] * 0.5 + if row == 3 { 0.5 } else { 0.0 };
        output[row][1] = light_view_projection[row][1] * -0.5 + if row == 3 { 0.5 } else { 0.0 };
        output[row][2] = light_view_projection[row][2];
        output[row][3] = light_view_projection[row][3];
    }
    Ok(output)
}

pub fn fit_shadow_map_matrices(
    fit_points: [[f32; 3]; 4],
    center: [f32; 3],
    selected_direction: [f32; 3],
) -> Result<(ShadowMat4, ShadowMat4)> {
    let (view_projection, texture, _) = fit_shadow_map_basis(fit_points, center, selected_direction)?;
    Ok((view_projection, texture))
}

pub(crate) fn fit_shadow_map_basis(
    fit_points: [[f32; 3]; 4],
    center: [f32; 3],
    selected_direction: [f32; 3],
) -> Result<(ShadowMat4, ShadowMat4, [[f32; 3]; 4])> {
    if !finite3(center)
        || !finite3(selected_direction)
        || fit_points.into_iter().any(|point| !finite3(point))
    {
        return Err("shadow L02 blocked: fit inputs are incomplete or non-finite".into());
    }
    let snapped_center = [
        trunc_grid_a729(center[0])?,
        center[1],
        trunc_grid_a729(center[2])?,
    ];
    let direction = normalize3_a729(selected_direction)?;
    if direction == [0.0; 3] {
        return Err("shadow L02 blocked: selected light direction has zero length".into());
    }
    let eye_axis = [
        flip_sign(direction[0]),
        flip_sign(direction[1]),
        flip_sign(direction[2]),
    ];

    let project = |point: [f32; 3]| {
        let dx = point[0] - snapped_center[0];
        let dy = point[1] - snapped_center[1];
        let dz = point[2] - snapped_center[2];
        let xy = dx * eye_axis[0] + dy * eye_axis[1];
        xy + dz * eye_axis[2]
    };
    let mut maximum = project(fit_points[0]);
    for point in fit_points.iter().skip(1) {
        let projection = project(*point);
        if maximum <= projection {
            maximum = projection;
        }
    }
    let eye_distance = maximum + SHADOW_FIT_EYE_MARGIN;
    if !eye_distance.is_finite() || eye_distance <= 0.0 {
        return Err("shadow L02 blocked: fitted light-eye distance is invalid".into());
    }
    let eye = [
        eye_axis[0] * eye_distance + snapped_center[0],
        eye_axis[1] * eye_distance + snapped_center[1],
        eye_axis[2] * eye_distance + snapped_center[2],
    ];
    let view = look_at_row_vector_a729(eye, snapped_center)?;

    let transformed = fit_points.map(|point| transform_point_row_vector(point, view));
    if transformed.iter().flatten().any(|value| !value.is_finite()) {
        return Err("shadow L02 blocked: transformed fit point is non-finite".into());
    }
    let mut minimum = transformed[0];
    let mut maximum = transformed[0];
    for point in transformed.iter().skip(1) {
        for axis in 0..3 {
            if point[axis] <= minimum[axis] {
                minimum[axis] = point[axis];
            }
            if maximum[axis] <= point[axis] {
                maximum[axis] = point[axis];
            }
        }
    }
    let minimum = [
        trunc_grid_a729(minimum[0] - SHADOW_FIT_GRID)?,
        trunc_grid_a729(minimum[1] - SHADOW_FIT_GRID)?,
        trunc_grid_a729(minimum[2] - SHADOW_FIT_GRID)?,
    ];
    let maximum = [
        trunc_grid_a729(maximum[0] + SHADOW_FIT_GRID)?,
        trunc_grid_a729(maximum[1] + SHADOW_FIT_GRID)?,
        trunc_grid_a729(maximum[2] + SHADOW_FIT_GRID)?,
    ];
    let extent = [
        maximum[0] - minimum[0],
        maximum[1] - minimum[1],
        maximum[2] - minimum[2],
    ];
    if extent
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0)
    {
        return Err("shadow L02 blocked: snapped orthographic extent is empty".into());
    }
    let scale = [2.0 / extent[0], 2.0 / extent[1], 1.0 / extent[2]];
    let translate = [
        (maximum[0] + minimum[0]) / (minimum[0] - maximum[0]),
        (maximum[1] + minimum[1]) / (minimum[1] - maximum[1]),
        flip_sign(minimum[2]) / extent[2],
    ];

    let mut light_view_projection = [[0.0; 4]; 4];
    for row in 0..4 {
        light_view_projection[row][0] = view[row][0] * scale[0] + view[row][3] * translate[0];
        light_view_projection[row][1] = view[row][1] * scale[1] + view[row][3] * translate[1];
        light_view_projection[row][2] = view[row][2] * scale[2] + view[row][3] * translate[2];
        light_view_projection[row][3] = view[row][3];
    }
    if light_view_projection
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err("shadow L02 blocked: fitted LightViewProjectionRaw is non-finite".into());
    }
    let shadow_map_texture_matrix = compose_shadow_map_texture_matrix(light_view_projection)?;
    Ok((light_view_projection, shadow_map_texture_matrix, [eye, snapped_center, scale, translate]))
}

pub fn build_shadow_frame(input: &ShadowFrameInput) -> Result<ShadowFrameMatrices> {
    let selected_light = select_shadow_light(input)?;
    let (light_view_projection, shadow_map_texture_matrix) =
        fit_shadow_map_matrices(input.fit_points, input.fit_center, selected_light.direction)?;
    Ok(ShadowFrameMatrices {
        selected_light,
        light_view_projection,
        shadow_map_texture_matrix,
    })
}

#[inline]
fn saturate(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

pub fn pcf_tap_offsets() -> [[f32; 2]; PCF_TAP_COUNT] {
    let t = PCF_TEXEL_SIZE;
    [
        [-t, 0.0],
        [0.0, t],
        [t, 0.0],
        [0.0, -t],
        [0.0, 0.0],
    ]
}

pub fn calculate_shadow(taps: &[f32; PCF_TAP_COUNT], receiver_z: f32) -> f32 {
    let b = receiver_z - PCF_BIAS;
    let mut sum = 0.0f32;
    for &a in taps.iter() {
        sum += if a < PCF_OCCLUSION_MAX && a < b {
            PCF_SHADOWED_VALUE
        } else {
            PCF_LIT_VALUE
        };
    }
    sum / PCF_TAP_COUNT as f32
}

pub fn shadow_fade_factor(cam_y: f32) -> f32 {
    saturate((DRAW_SHADOWS_CUTOFF - cam_y) / DRAW_SHADOWS_FADE_LENGTH)
}

pub fn get_shadow_scaled(weight: f32, screen_shadow_r: f32, fade: f32) -> f32 {
    let scaler = saturate(weight);
    let v = saturate(screen_shadow_r + (1.0 - fade));
    (1.0 - scaler) + scaler * v
}

pub fn terrain_shadow_term(screen_shadow_r: f32, cam_y: f32) -> f32 {
    get_shadow_scaled(
        SHADOW_WEIGHT_TERRAIN,
        screen_shadow_r,
        shadow_fade_factor(cam_y),
    )
    .max(TERRAIN_SHADOW_FLOOR)
}

pub fn shadow_gated_by_daynight(shadow_term: f32, day_factor: f32, night_factor: f32) -> f32 {
    shadow_term * saturate(day_factor + night_factor)
}

pub fn shadow_blur_3tap(center: f32, n1p: f32, n1m: f32, n2p: f32, n2m: f32, w: [f32; 3]) -> f32 {
    center * w[0] + (n1p + n1m) * w[1] + (n2p + n2m) * w[2]
}

fn read_source(resolver: &GameFileResolver, rel: &str) -> Result<String> {
    let path = resolver.resolve(rel)?.ok_or_else(|| {
        Box::<dyn std::error::Error>::from(format!("shadow_law: resolver could not find {rel}"))
    })?;
    Ok(resolver.read_required_text(rel, rel)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    fn l02_input() -> ShadowFrameInput {
        ShadowFrameInput {
            map_focus: [0.0, 0.0],
            camera_x: 0.0,
            map_width: 100.0,
            primary_sun: [-1.0, 600.0, 0.0],
            primary_moon: [1.0, 600.0, 0.0],
            fit_center: [0.0, 0.0, 0.0],
            fit_points: [
                [-20.0, 0.0, -20.0],
                [20.0, 0.0, -20.0],
                [20.0, 0.0, 20.0],
                [-20.0, 0.0, 20.0],
            ],
        }
    }


    #[test]
    fn gate_l02_finite_distance_tie_selects_primary_moon() {
        let selected = select_shadow_light(&l02_input()).unwrap();
        assert_eq!(selected.kind, ShadowLightKind::Moon);
        assert_eq!(selected.position, [1.0, 600.0, 0.0]);
        assert_eq!(selected.wrapped_x, 1.0);
    }

    #[test]
    fn gate_l02_wrap_adopts_only_strictly_nearer_candidate() {
        let mut input = l02_input();
        input.map_focus = [50.0, 0.0];
        input.camera_x = 75.0;
        input.primary_sun = [-40.0, 600.0, 0.0];
        input.primary_moon = [-500.0, 600.0, 0.0];
        let selected = select_shadow_light(&input).unwrap();
        assert_eq!(selected.kind, ShadowLightKind::Sun);
        assert_eq!(selected.wrapped_x, 60.0);

        input.map_focus = [25.0, 0.0];
        input.primary_sun = [25.0, 600.0, 0.0];
        let selected = select_shadow_light(&input).unwrap();
        assert_eq!(selected.wrapped_x, 25.0);
    }

    #[test]
    fn gate_l02_axis_aligned_fit_matches_hand_computed_row_vector_matrix() {
        let points = l02_input().fit_points;
        let (light_view_projection, shadow_map_texture_matrix) =
            fit_shadow_map_matrices(points, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap();
        let expected_light = [
            [0.0, 0.0, 1.0 / 60.0, 0.0],
            [0.0, 0.1, 0.0, 0.0],
            [-1.0 / 30.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.5, 1.0],
        ];
        assert_eq!(light_view_projection, expected_light);
        let expected_texture = [
            [0.0, 0.0, 1.0 / 60.0, 0.0],
            [0.0, -0.05, 0.0, 0.0],
            [-1.0 / 60.0, 0.0, 0.0, 0.0],
            [0.5, 0.5, 0.5, 1.0],
        ];
        assert_eq!(shadow_map_texture_matrix, expected_texture);
    }

    #[test]
    fn gate_l02_clip_bias_copies_z_w_and_translates_only_last_row() {
        let light = [
            [2.0, 4.0, 6.0, 8.0],
            [-2.0, -4.0, -6.0, -8.0],
            [0.25, -0.25, 0.5, -0.5],
            [0.75, -0.75, 0.125, 1.0],
        ];
        assert_eq!(
            compose_shadow_map_texture_matrix(light).unwrap(),
            [
                [1.0, -2.0, 6.0, 8.0],
                [-1.0, 2.0, -6.0, -8.0],
                [0.125, 0.125, 0.5, -0.5],
                [0.875, 0.875, 0.125, 1.0],
            ]
        );
    }

    #[test]
    fn gate_l02_production_seam_fails_closed() {
        let mut input = l02_input();
        input.primary_sun[0] = f32::NAN;
        assert!(build_shadow_frame(&input).is_err());

        let mut input = l02_input();
        input.map_width = 0.0;
        assert!(build_shadow_frame(&input).is_err());

        assert!(fit_shadow_map_matrices(input.fit_points, input.fit_center, [0.0; 3]).is_err());
        assert!(compose_shadow_map_texture_matrix([[f32::INFINITY; 4]; 4]).is_err());
    }


    #[test]
    fn gate_pcf_tap_offsets_match_shadow_fxh() {
        let t = 0.5f32 / 2048.0f32;
        assert_eq!(
            pcf_tap_offsets(),
            [[-t, 0.0], [0.0, t], [t, 0.0], [0.0, -t], [0.0, 0.0]]
        );
    }

    #[test]
    fn gate_pcf_all_lit_returns_one() {
        let term = calculate_shadow(&[1.0; 5], 0.5);
        assert!((term - 1.0).abs() < EPS, "all-lit -> 1.0, got {term}");
    }

    #[test]
    fn gate_pcf_all_shadowed_returns_floor() {
        let term = calculate_shadow(&[0.0; 5], 0.9);
        assert!((term - 0.1).abs() < EPS, "all-shadowed -> 0.1, got {term}");
    }

    #[test]
    fn gate_pcf_partial_two_shadowed() {
        let term = calculate_shadow(&[0.0, 0.0, 1.0, 1.0, 1.0], 0.9);
        assert!((term - 0.64).abs() < EPS, "expected 0.64, got {term}");
    }

    #[test]
    fn gate_pcf_099_self_shadow_guard() {
        let term = calculate_shadow(&[0.995; 5], 0.9);
        assert!((term - 1.0).abs() < EPS, "0.99 guard -> lit, got {term}");
    }

    #[test]
    fn gate_pcf_bias_prevents_acne() {
        let term = calculate_shadow(&[0.5; 5], 0.5);
        assert!((term - 1.0).abs() < EPS, "bias -> lit, got {term}");
        let shadowed = calculate_shadow(&[0.5; 5], 0.5 + 2.0 * PCF_BIAS);
        assert!(
            (shadowed - 0.1).abs() < EPS,
            "past bias -> shadowed, got {shadowed}"
        );
    }


    #[test]
    fn gate_shadow_fade_factor_endpoints() {
        assert!((shadow_fade_factor(50.0) - 1.0).abs() < EPS);
        assert!((shadow_fade_factor(350.0) - 1.0).abs() < EPS);
        assert!((shadow_fade_factor(375.0) - 0.5).abs() < EPS);
        assert!((shadow_fade_factor(400.0) - 0.0).abs() < EPS);
        assert!((shadow_fade_factor(3000.0) - 0.0).abs() < EPS);
    }

    #[test]
    fn gate_shadow_fully_faded_above_cutoff() {
        let fade = shadow_fade_factor(500.0);
        let term = get_shadow_scaled(SHADOW_WEIGHT_TERRAIN, 0.1, fade);
        assert!((term - 1.0).abs() < EPS, "faded -> no shadow, got {term}");
    }


    #[test]
    fn gate_get_shadow_scaled_full_shadow_near_camera() {
        let term = get_shadow_scaled(SHADOW_WEIGHT_TERRAIN, 0.1, 1.0);
        assert!((term - 0.37).abs() < EPS, "expected 0.37, got {term}");
    }

    #[test]
    fn gate_get_shadow_scaled_lit_is_one() {
        let term = get_shadow_scaled(SHADOW_WEIGHT_WATER, 1.0, 1.0);
        assert!((term - 1.0).abs() < EPS, "lit -> 1.0, got {term}");
    }

    #[test]
    fn gate_terrain_floor_binds_only_below_min() {
        let term = terrain_shadow_term(0.1, 50.0);
        assert!((term - 0.37).abs() < EPS, "expected 0.37, got {term}");
        assert!(
            (get_shadow_scaled(SHADOW_WEIGHT_TERRAIN, -10.0, 1.0)).max(TERRAIN_SHADOW_FLOOR) >= 0.1
        );
    }

    #[test]
    fn gate_shadow_gated_by_daynight() {
        assert!((shadow_gated_by_daynight(0.37, 1.0, 0.0) - 0.37).abs() < EPS);
        assert!((shadow_gated_by_daynight(0.5, 0.3, 0.2) - 0.25).abs() < EPS);
        assert!((shadow_gated_by_daynight(0.5, 0.8, 0.8) - 0.5).abs() < EPS);
    }


    #[test]
    fn gate_shadow_blur_3tap_hand_computed() {
        let out = shadow_blur_3tap(1.0, 0.5, 0.5, 0.5, 0.5, [0.4, 0.2, 0.1]);
        assert!((out - 0.7).abs() < EPS, "expected 0.7, got {out}");
        let uni = shadow_blur_3tap(0.5, 0.5, 0.5, 0.5, 0.5, [0.4, 0.15, 0.15]);
        assert!((uni - 0.5).abs() < EPS, "uniform passthrough, got {uni}");
    }

    #[test]
    fn gate_shadow_blur_cb_matches_retained_frame_35240_bits() {
        let cb = shadow_blur_constant_buffers(50.000_007_629_394_53, 1920, 1080).unwrap();
        assert_eq!(
            cb.horizontal_f32_bits,
            [
                0x00000000, 0x3a2f8af9, 0x3acccccd, 0x3e6879c6, 0x3ea1e718, 0x3d8fe9dc, 0x3f800000,
                0x3f800000,
            ]
        );
        assert_eq!(
            cb.vertical_f32_bits,
            [
                0x00000000, 0x3a9c09c0, 0x3b360b61, 0x3e6879c6, 0x3ea1e718, 0x3d8fe9dc, 0x3f800000,
                0x3f800000,
            ]
        );
    }

    #[test]
    fn gate_shadow_blur_cb_rejects_nonportable_dimensions_and_camera() {
        assert!(shadow_blur_constant_buffers(f32::NAN, 1920, 1080).is_err());
        assert!(shadow_blur_constant_buffers(50.0, 0, 1080).is_err());
        assert!(shadow_blur_constant_buffers(50.0, 1920, 0).is_err());
    }


    fn vanilla_resolver() -> GameFileResolver {
        GameFileResolver::vanilla(replay_test_gates::require_vanilla_game_root())
            .expect("vanilla resolver")
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_shadow_constants_are_shipped() {
        let resolver = vanilla_resolver();

        let constants = read_source(&resolver, CONSTANTS_FXH_SOURCE).unwrap();
        for needle in [
            "SHADOW_WEIGHT_TERRAIN    	= 0.7f",
            "SHADOW_WEIGHT_WATER   		= 0.5f",
            "SHADOW_WEIGHT_RIVER   		= 0.4f",
            "SHADOW_WEIGHT_TREE   		= 0.7f",
        ] {
            assert!(
                constants.contains(needle),
                "constants.fxh missing `{needle}`"
            );
        }

        let shadow_fxh = read_source(&resolver, SHADOW_FXH_SOURCE).unwrap();
        assert!(shadow_fxh.contains("0.5f / 2048.0f"), "PCF texel size");
        assert!(shadow_fxh.contains("fBias = 0.001f"), "PCF bias");
        assert!(
            shadow_fxh.contains("? 0.1f : 1.0f"),
            "PCF penumbra/lit values"
        );
        assert!(shadow_fxh.contains("fShadowTerm / 5.0f"), "5-tap average");
        assert!(
            shadow_fxh.contains("( 1.0 - fScaler ) + fScaler * vShadowValue"),
            "GetShadowScaled blend"
        );

        let defines = read_source(&resolver, GRAPHICS_DEFINES_SOURCE).unwrap();
        assert!(defines.contains("DRAW_SHADOWS_CUTOFF = 400"), "cutoff");
        assert!(
            defines.contains("DRAW_SHADOWS_FADE_LENGTH = 50"),
            "fade length"
        );
        assert!(
            defines.contains("LIGHT_SHADOW_DIRECTION_X = -5.0"),
            "light dir x"
        );
        assert!(
            defines.contains("LIGHT_SHADOW_DIRECTION_Y = -8.0"),
            "light dir y"
        );
        assert!(
            defines.contains("LIGHT_SHADOW_DIRECTION_Z = 5.0"),
            "light dir z"
        );

        let pdxmap = read_source(&resolver, PDXMAP_SHADER_SOURCE).unwrap();
        assert!(
            pdxmap.contains("max(GetShadowScaled( SHADOW_WEIGHT_TERRAIN, Input.vScreenCoord, ShadowMap ), 0.1f )"),
            "terrain shadow floor callsite"
        );
        assert!(
            pdxmap.contains("float fShadowTerm = CalculateShadow( Input.vShadowProj, ShadowMap);"),
            "producer callsite"
        );
    }
}
