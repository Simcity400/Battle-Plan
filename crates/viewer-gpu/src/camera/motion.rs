
use std::{f32::consts::FRAC_PI_2, io};

use super::{Camera, Placement};

const LONGEST_FRAME: f32 = 0.7;
const SETTLED: f32 = 0.02;
const CAMERA_SPEED_RANGE: [f32; 2] = [5.0, 95.0];
const CAMERA_SPEED_SCALE: f32 = 100.0;
const FORWARD_CORRECTION: f32 = 0.015;
const FLOOR_DAMPING: f32 = 0.1;
const LEAST_KEY_MOVE: f32 = 1.0;
const EDGE_PIXELS: i32 = 2;
const PAN_PER_HEIGHT: f32 = 0.1;
const ZOOM_IN_STEP: f32 = 0.5;
const ZOOM_OUT_STEP: f32 = 0.75;
const LEAST_ZOOM_STEP: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraLaw {
    pub speed: f32,
    pub key_scale: f32,
    pub distance_mult: f32,
    pub interpolation: f32,
    pub zoom_speed: f32,
    pub camera_speed: f32,
    pub scroll_speed: f32,
    pub edge_scrolling: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speeds {
    pub zoom_speed: f32,
    pub camera_speed: f32,
    pub scroll_speed: f32,
    pub edge_scrolling: bool,
}

impl CameraLaw {
    pub fn load(
        defines: &content_reader_defines::defines::Defines,
        settings: &content_reader_settings::settings::SettingsKeys,
    ) -> io::Result<Self> {
        use content_reader_settings::settings::constructed;
        let setting = |key: &str, constructed: f32| -> io::Result<f32> {
            settings
                .f32_or_constructed(&[key], constructed)
                .ok_or_else(|| invalid(format!("settings.txt {key} is not a number")))
        };
        Ok(Self::new(
            Self::content_defines(defines)?,
            Speeds {
                zoom_speed: setting("zoom_speed", constructed::ZOOM_SPEED)?,
                camera_speed: setting("camera_speed", constructed::CAMERA_SPEED)?,
                scroll_speed: setting("scroll_speed", constructed::SCROLL_SPEED)?,
                edge_scrolling: settings
                    .bool_or_constructed(&["corner_scrolling"], constructed::CORNER_SCROLLING)
                    .ok_or_else(|| {
                        invalid("settings.txt corner_scrolling is not yes or no".to_owned())
                    })?,
            },
        ))
    }

    pub fn content_defines(defines: &content_reader_defines::defines::Defines) -> io::Result<[f32; 4]> {
        let define = |section: &str, key: &str| -> io::Result<f32> {
            let value = defines.number(section, key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.{section}.{key} is outside f32")))
            }
        };
        Ok([
            define("NGraphics", "CAMERA_ZOOM_SPEED")?,
            define("NGraphics", "CAMERA_ZOOM_KEY_SCALE")?,
            define("NGraphics", "CAMERA_ZOOM_SPEED_DISTANCE_MULT")?,
            define("NFrontend", "CAMERA_INTERPOLATION_SPEED")?,
        ])
    }

    pub fn new(defines: [f32; 4], speeds: Speeds) -> Self {
        let [speed, key_scale, distance_mult, interpolation] = defines;
        let Speeds {
            zoom_speed,
            camera_speed,
            scroll_speed,
            edge_scrolling,
        } = speeds;
        let [low, high] = CAMERA_SPEED_RANGE;
        let camera_speed = if low > camera_speed {
            low
        } else if camera_speed > high {
            high
        } else {
            camera_speed
        };
        Self {
            speed,
            key_scale,
            distance_mult,
            interpolation,
            zoom_speed,
            camera_speed,
            scroll_speed,
            edge_scrolling,
        }
    }

    pub fn edge_scrolls(&self, cursor: [f32; 2], viewport: [u32; 2]) -> bool {
        let [width, height] = viewport.map(|side| side as i32);
        let [x, y] = cursor.map(|value| value as i32);
        self.edge_scrolling
            && (y < EDGE_PIXELS
                || y >= height - EDGE_PIXELS
                || x < EDGE_PIXELS
                || x >= width - EDGE_PIXELS)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test() -> Self {
        Self::new(
            [50.0, 0.02, 6.0, 0.19],
            Speeds {
                zoom_speed: 50.0,
                camera_speed: 50.0,
                scroll_speed: 22.0,
                edge_scrolling: true,
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    target: Option<Placement>,
    lines: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pan {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input<'a> {
    pub cursor: Option<[f32; 2]>,
    pub notches: &'a [i32],
    pub ground: Option<[f32; 3]>,
    pub keys: Pan,
}

pub type Floor<'a> = &'a dyn Fn(f32, f32) -> f32;

impl Motion {
    pub fn target(&self) -> Option<Placement> {
        self.target
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn shift(&mut self, by: [f32; 2]) {
        if let Some(target) = &mut self.target {
            target.center[0] += by[0];
            target.center[1] += by[1];
        }
    }

    pub fn notches(&mut self, lines: f32) -> i32 {
        if !lines.is_finite() {
            return 0;
        }
        self.lines += lines;
        let whole = self.lines.trunc();
        self.lines -= whole;
        whole as i32
    }

    pub fn wheel_events(&mut self, lines: impl IntoIterator<Item = f32>) -> Vec<i32> {
        let mut notches = Vec::new();
        for lines in lines {
            let whole = self.notches(lines);
            notches.extend(std::iter::repeat_n(
                whole.signum(),
                whole.unsigned_abs() as usize,
            ));
        }
        notches
    }

    pub fn key(&mut self, camera: &mut Camera, law: &CameraLaw, zoom_in: bool, floor: Floor<'_>) {
        let mut step = law.speed * law.zoom_speed * law.key_scale;
        if zoom_in {
            step = -step;
        }
        let height = step + camera.height;
        let before = self.target.unwrap_or_else(|| camera.placement());
        let mut target = before;
        target.height = height - target.height + before.height;
        clamp(camera, &mut target, floor);
        self.target = Some(if LEAST_KEY_MOVE > (before.height - target.height).abs() {
            before
        } else {
            target
        });
    }

    pub fn place(
        &mut self,
        camera: &mut Camera,
        center: [f32; 2],
        height: Option<f32>,
        floor: Floor<'_>,
    ) {
        let before = self.target.unwrap_or_else(|| camera.placement());
        let mut target = Placement {
            center,
            height: height.unwrap_or(before.height),
            pose: before.pose,
        };
        clamp(camera, &mut target, floor);
        if height.is_some() && LEAST_KEY_MOVE > (before.height - target.height).abs() {
            target.height = before.height;
        }
        self.target = Some(target);
    }

    pub fn zoom_step(&mut self, camera: &mut Camera, zoom_in: bool, floor: Floor<'_>) {
        let before = self.target.unwrap_or_else(|| camera.placement());
        let factor = if zoom_in { ZOOM_IN_STEP } else { ZOOM_OUT_STEP };
        let step = (factor * before.height).max(LEAST_ZOOM_STEP);
        let height = if zoom_in {
            before.height - step
        } else {
            before.height + step
        };
        self.place(camera, before.center, Some(height), floor);
    }

    pub fn wheel(
        &mut self,
        camera: &Camera,
        law: &CameraLaw,
        frame: f32,
        notches: &[i32],
        ground: Option<[f32; 3]>,
        floor: Floor<'_>,
    ) {
        let input = Input {
            notches,
            ground,
            ..Input::default()
        };
        self.step(camera, law, frame, [0, 0], &input, floor);
    }

    pub fn step(
        &mut self,
        camera: &Camera,
        law: &CameraLaw,
        frame: f32,
        viewport: [u32; 2],
        input: &Input<'_>,
        floor: Floor<'_>,
    ) -> bool {
        let frame = longest(frame);
        let eye = camera.eye();
        let [_, top] = limits(camera);
        let low = floor_under(camera, eye, floor);
        let target = self.target.unwrap_or_else(|| camera.placement());
        let target_height = target.height;
        let pan = frame * law.scroll_speed * (PAN_PER_HEIGHT * camera.height);
        let [width, height] = viewport.map(|side| side as i32);
        let toward =
            |point: [i32; 2]| camera.pan_direction([point[0] as f32, point[1] as f32], viewport);
        let mut moved = [0.0f32; 3];
        if let Some(cursor) = input.cursor
            && law.edge_scrolls(cursor, viewport)
        {
            let direction = toward(cursor.map(|value| value as i32));
            moved = [direction[0] * pan, direction[1] * pan, direction[2] * pan];
        }
        if let Some(ground) = input.ground {
            for &notch in input.notches {
                let t = (eye[1] - low) / (top - low);
                if (t >= 1.0 && notch < 0) || (0.0 >= t && notch > 0) {
                    continue;
                }
                let correction = camera.map.height as i32 as f32 * FORWARD_CORRECTION * (1.0 - t);
                let toward = normalize([
                    ground[0] - eye[0],
                    ground[1] - eye[1],
                    ground[2] - eye[2] - correction,
                ]);
                let base = frame * law.zoom_speed * law.speed;
                let rise = (t * FRAC_PI_2 - FRAC_PI_2).sin() + 1.0;
                let step = rise * law.distance_mult * base + base;
                let notch = notch as f32;
                moved[0] += toward[0] * step * notch;
                moved[1] += toward[1] * step * notch;
                moved[2] += step * toward[2] * notch;
                let reached = moved[1] + target_height;
                if low >= reached {
                    moved[1] += low - reached;
                    moved[0] *= t + FLOOR_DAMPING;
                    moved[2] *= t + FLOOR_DAMPING;
                }
            }
        }
        let keys = input.keys;
        for (held, point) in [
            (keys.up, [width / 2, 0]),
            (keys.down, [width / 2, height]),
            (keys.left, [0, height / 2]),
            (keys.right, [width, height / 2]),
        ] {
            if held {
                let direction = toward(point);
                moved[0] += direction[0] * pan;
                moved[1] += direction[1] * pan;
                moved[2] += direction[2] * pan;
            }
        }
        if moved == [0.0; 3] {
            return false;
        }
        self.target = Some(Placement {
            center: [target.center[0] + moved[0], target.center[1] + moved[2]],
            height: target.height + moved[1],
            pose: target.pose,
        });
        true
    }

    pub fn ease(
        &mut self,
        camera: &mut Camera,
        law: &CameraLaw,
        frame: f32,
        viewport: [u32; 2],
        floor: Floor<'_>,
    ) -> bool {
        let Some(mut target) = self.target else {
            return false;
        };
        let lead = camera.defines.tilt_lead;
        let along = [
            target.center[0] - camera.center[0],
            target.height - camera.height,
            (target.center[1] - lead) - (camera.center[1] - lead),
        ];
        let look = [
            target.center[0] - camera.center[0],
            target.center[1] - camera.center[1],
        ];
        let eye_gap = along[0] * along[0] + along[1] * along[1] + along[2] * along[2];
        let look_gap = look[0] * look[0] + look[1] * look[1];
        if !(eye_gap > SETTLED || look_gap > SETTLED) {
            return false;
        }
        let rate = (CAMERA_SPEED_SCALE - law.camera_speed) * law.interpolation * longest(frame);
        let rate = if 0.0 > rate { 0.0 } else { rate.min(1.0) };
        let before = camera.placement();
        camera.height += along[1] * rate;
        camera.center[0] += look[0] * rate;
        camera.center[1] += look[1] * rate;
        clamp(camera, &mut target, floor);
        let held = camera.center[1];
        camera.clamp_center(viewport);
        if camera.center[1] != held {
            target.center[1] = camera.center[1];
        }
        self.target = Some(target);
        camera.placement() != before
    }
}

fn clamp(camera: &mut Camera, target: &mut Placement, floor: Floor<'_>) {
    let rows = camera.map.height as i32 as f32;
    for z in [&mut camera.center[1], &mut target.center[1]] {
        *z = if 0.0 > *z {
            0.0
        } else if *z > rows {
            rows
        } else {
            *z
        };
    }
    let [_, top] = limits(camera);
    let low = floor_under(camera, camera.eye(), floor);
    for height in [&mut camera.height, &mut target.height] {
        *height = if low > *height {
            low
        } else if *height > top {
            top
        } else {
            *height
        };
    }
}

fn limits(camera: &Camera) -> [f32; 2] {
    camera.defines.heights.map(|height| height as i32 as f32)
}

fn floor_under(camera: &Camera, eye: [f32; 3], floor: Floor<'_>) -> f32 {
    floor(eye[0], eye[2]).max(limits(camera)[0])
}

fn longest(frame: f32) -> f32 {
    LONGEST_FRAME.min(frame)
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length == 0.0 {
        v
    } else {
        [v[0] / length, v[1] / length, v[2] / length]
    }
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "motion_tests.rs"]
mod tests;
