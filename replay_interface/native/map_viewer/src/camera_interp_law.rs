
pub const CAMERA_INTERPOLATION_SPEED: f32 = 0.19;
pub const CAMERA_SPEED_IN_MENUS: f32 = 0.1;
pub const CAMERA_MIN_HEIGHT: f32 = 50.0;
pub const CAMERA_MAX_HEIGHT: f32 = 3000.0;
pub const CAMERA_RAW_DELTA_CAP: f32 = 0.7;
pub const CAMERA_DISTANCE_SQUARED_GATE: f32 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraTemporalState {
    pub current_eye: [f32; 3],
    pub current_look_at: [f32; 3],
    pub target_eye: [f32; 3],
    pub target_look_at: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraTemporalInputs {
    pub raw_owner_delta: f32,
    pub ramp_6f4: f32,
    pub settings_camera_speed: f32,
    pub interpolation_speed: f32,
    pub camera_min_height_i32: i32,
    pub camera_max_height_i32: i32,
    pub active_map_min_height_i32: i32,
    pub map_x_max_i32: i32,
    pub map_z_max_i32: i32,
    pub wrap_x: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraTemporalOutcome {
    pub state: CameraTemporalState,
    pub capped_delta: f32,
    pub alpha: f32,
    pub update_result_6d8: u8,
}

fn minss(left: f32, right: f32) -> f32 {
    if left.is_nan() || right.is_nan() || left == right {
        right
    } else if left < right {
        left
    } else {
        right
    }
}

fn maxss(left: f32, right: f32) -> f32 {
    if left.is_nan() || right.is_nan() || left == right {
        right
    } else if left > right {
        left
    } else {
        right
    }
}

fn clamp_sse(value: f32, lower: f32, upper: f32) -> f32 {
    minss(maxss(value, lower), upper)
}

fn squared_distance(current: [f32; 3], target: [f32; 3]) -> f32 {
    let dx = target[0] - current[0];
    let dy = target[1] - current[1];
    let dz = target[2] - current[2];
    let xy = dx * dx + dy * dy;
    xy + dz * dz
}

fn interpolate_component(current: f32, target: f32, alpha: f32) -> f32 {
    let difference = target - current;
    let scaled = difference * alpha;
    scaled + current
}

fn repair_eye_look_at_pair(
    eye: &mut [f32; 3],
    look_at: &mut [f32; 3],
    map_x_max: f32,
    map_z_max: f32,
    wrap_x: bool,
) {
    let saved_eye = *eye;
    let saved_look_at = *look_at;
    look_at[2] = clamp_sse(look_at[2], 0.0, map_z_max);
    if !wrap_x {
        look_at[0] = clamp_sse(look_at[0], 0.0, map_x_max);
    }
    for component in 0..3 {
        let eye_to_look_at = saved_eye[component] - saved_look_at[component];
        eye[component] = eye_to_look_at + look_at[component];
    }
}

pub fn camera_temporal_update_and_bounds_repair(
    mut state: CameraTemporalState,
    inputs: CameraTemporalInputs,
) -> CameraTemporalOutcome {
    let capped_delta = minss(inputs.raw_owner_delta, CAMERA_RAW_DELTA_CAP);
    let mut coefficient = 100.0 - inputs.settings_camera_speed;
    coefficient = coefficient * inputs.ramp_6f4;
    coefficient = coefficient * inputs.interpolation_speed;
    coefficient = coefficient * capped_delta;
    let alpha = clamp_sse(coefficient, 0.0, 1.0);

    let eye_distance = squared_distance(state.current_eye, state.target_eye);
    let look_at_distance = squared_distance(state.current_look_at, state.target_look_at);
    if eye_distance <= CAMERA_DISTANCE_SQUARED_GATE
        && look_at_distance <= CAMERA_DISTANCE_SQUARED_GATE
    {
        return CameraTemporalOutcome {
            state,
            capped_delta,
            alpha,
            update_result_6d8: 0,
        };
    }

    for component in 0..3 {
        state.current_eye[component] = interpolate_component(
            state.current_eye[component],
            state.target_eye[component],
            alpha,
        );
        state.current_look_at[component] = interpolate_component(
            state.current_look_at[component],
            state.target_look_at[component],
            alpha,
        );
    }

    let map_x_max = inputs.map_x_max_i32 as f32;
    let map_z_max = inputs.map_z_max_i32 as f32;
    repair_eye_look_at_pair(
        &mut state.current_eye,
        &mut state.current_look_at,
        map_x_max,
        map_z_max,
        inputs.wrap_x,
    );
    repair_eye_look_at_pair(
        &mut state.target_eye,
        &mut state.target_look_at,
        map_x_max,
        map_z_max,
        inputs.wrap_x,
    );

    let configured_minimum = inputs.camera_min_height_i32 as f32;
    let active_map_minimum = inputs.active_map_min_height_i32 as f32;
    let lower = maxss(configured_minimum, active_map_minimum);
    let upper = inputs.camera_max_height_i32 as f32;
    state.current_eye[1] = clamp_sse(state.current_eye[1], lower, upper);
    state.target_eye[1] = clamp_sse(state.target_eye[1], lower, upper);

    CameraTemporalOutcome {
        state,
        capped_delta,
        alpha,
        update_result_6d8: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn common_inputs(raw_owner_delta: f32) -> CameraTemporalInputs {
        CameraTemporalInputs {
            raw_owner_delta,
            ramp_6f4: 1.0,
            settings_camera_speed: 50.0,
            interpolation_speed: CAMERA_INTERPOLATION_SPEED,
            camera_min_height_i32: 50,
            camera_max_height_i32: 3000,
            active_map_min_height_i32: 0,
            map_x_max_i32: 10_000,
            map_z_max_i32: 10_000,
            wrap_x: false,
        }
    }

    #[test]
    fn c02_first_multi_delta_step_matches_frozen_bits() {
        let state = CameraTemporalState {
            current_eye: [0.0, 100.0, 0.0],
            current_look_at: [0.0, 0.0, 0.0],
            target_eye: [100.0, 200.0, 50.0],
            target_look_at: [100.0, 100.0, 50.0],
        };
        let outcome = camera_temporal_update_and_bounds_repair(state, common_inputs(0.016));
        assert_eq!(outcome.capped_delta.to_bits(), 0x3c83126f);
        assert_eq!(outcome.alpha.to_bits(), 0x3e1ba5e4);
        assert_eq!(
            outcome.state.current_eye.map(f32::to_bits),
            [0x41733334, 0x42e66666, 0x40f33334]
        );
        assert_eq!(outcome.update_result_6d8, 1);
    }

    #[test]
    fn c02_distance_gate_skips_interpolation_and_repair() {
        let state = CameraTemporalState {
            current_eye: [0.0, 100.0, 0.0],
            current_look_at: [0.0, 0.0, 0.0],
            target_eye: [0.1, 100.0, 0.0],
            target_look_at: [0.1, 0.0, 0.0],
        };
        let outcome = camera_temporal_update_and_bounds_repair(state, common_inputs(0.016));
        assert_eq!(outcome.state, state);
        assert_eq!(outcome.update_result_6d8, 0);
    }
}
