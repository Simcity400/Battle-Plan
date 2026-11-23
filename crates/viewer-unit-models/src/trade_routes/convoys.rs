
use std::{collections::BTreeMap, io};

use content_reader_defines::defines::Defines;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_gpu::ribbon_mesh::span;

use super::{Countries, finite};
use viewer_models::ModelInstance;
use crate::unit_models::UnitModelAssets;

pub const CONVOY_ENTITY_SUFFIX: &str = "_traderoute_convoy";
pub const FALLBACK_CONVOY_ENTITY: &str = "westerngfx_traderoute_convoy";

const CONVOY_SEED: u32 = 0x7472_6164;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConvoyDefines {
    pub scale_factor: f32,
    pub max: i32,
    pub speed: f32,
    pub sleep: f32,
    pub route_offset: f32,
    pub cutoff: f32,
}

impl ConvoyDefines {
    pub fn read(defines: &Defines) -> io::Result<Self> {
        Ok(Self {
            scale_factor: finite(defines, "TRADE_ROUTE_NUM_CONVOYS_SCALE_FACTOR")?,
            max: defines.int32("NGraphics", "TRADE_ROUTE_MAX_NUM_CONVOYS")?,
            speed: finite(defines, "TRADE_ROUTE_CONVOY_SPEED")?,
            sleep: finite(defines, "TRADE_ROUTE_CONVOY_SLEEP_TIME")?,
            route_offset: finite(defines, "TRADE_ROUTE_CONVOY_ROUTE_OFFSET")?,
            cutoff: finite(defines, "UNITS_DISTANCE_CUTOFF")?,
        })
    }
}

pub fn convoy_count(assigned: i32, defines: &ConvoyDefines) -> u32 {
    if assigned == 0 {
        return 0;
    }
    let scaled = (assigned as f32 * defines.scale_factor).round() as i64;
    let at_least_one = if scaled < 1 { 1 } else { scaled };
    u32::try_from(at_least_one.min(i64::from(defines.max))).unwrap_or(0)
}

pub fn convoy_entity(culture: &str, declared: &dyn Fn(&str) -> bool) -> Result<String, String> {
    let own = format!("{culture}{CONVOY_ENTITY_SUFFIX}");
    if declared(&own) {
        return Ok(own);
    }
    if declared(FALLBACK_CONVOY_ENTITY) {
        return Ok(FALLBACK_CONVOY_ENTITY.to_owned());
    }
    Err(format!(
        "neither {own} nor {FALLBACK_CONVOY_ENTITY} is a declared entity"
    ))
}

pub(super) fn viewed_culture(
    frame: &DecodedFrame,
    countries: &Countries<'_>,
) -> Result<String, String> {
    let rows = frame
        .complete::<tables::CountryAppearance>()
        .map_err(|error| error.to_string())?;
    let row = rows
        .iter()
        .find(|row| countries.viewed(row.country) == Ok(true))
        .ok_or("the viewed country has no CountryAppearance row")?;
    String::from_utf8(row.graphical_culture.clone())
        .map_err(|_| "the viewed country's graphical culture is not text".to_owned())
}

pub fn convoys_shown(
    camera_height: f32,
    defines: &ConvoyDefines,
    draw_units: bool,
    trade_routes: bool,
) -> bool {
    draw_units && trade_routes && camera_height < defines.cutoff
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConvoyLine {
    pub entity: String,
    pub points: Vec<[f32; 2]>,
    pub map_width: f32,
    pub count: u32,
    pub seed: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConvoyPose {
    pub line: usize,
    pub convoy: u32,
    pub position: [f32; 3],
    pub facing: [f32; 3],
}

pub fn convoy_path(points: &[[f32; 2]], offset: f32) -> Vec<[f32; 2]> {
    let mut out = points.to_vec();
    for i in 1..points.len().saturating_sub(1) {
        out[i][0] += offset * (points[i + 1][1] - points[i][1]);
        out[i][1] -= offset * (points[i + 1][0] - points[i][0]);
    }
    out
}

struct SailingLine {
    width: f32,
    points: Vec<[f32; 2]>,
    lengths: Vec<f32>,
    random: viewer_models::entity_animation::Random,
}
struct SailingConvoy {
    line: usize,
    convoy: u32,
    segment: usize,
    distance: f32,
    sleep: f32,
    active: bool,
    radius: f32,
    pose: ConvoyPose,
}

#[derive(Default)]
pub struct ConvoyFleet {
    lines: Vec<SailingLine>,
    convoys: Vec<SailingConvoy>,
    last: Option<f32>,
}

fn pose(
    line: &SailingLine,
    convoy: &SailingConvoy,
    height: &dyn Fn(f32, f32) -> f32,
) -> ConvoyPose {
    let i = convoy.segment;
    let (a, b) = (line.points[i], line.points[i + 1]);
    let length = line.lengths[i];
    let direction = if length > 0.0 {
        [(b[0] - a[0]) / length, (b[1] - a[1]) / length]
    } else {
        [0., 0.]
    };
    let t = if length > 0.0 {
        convoy.distance / length
    } else {
        0.0
    };
    let mut x = a[0] + (b[0] - a[0]) * t;
    if x < 0. {
        x += line.width;
    } else if x >= line.width {
        x -= line.width;
    }
    let z = a[1] + (b[1] - a[1]) * t;
    let next = line
        .points
        .get(i + 2)
        .map(|&p| {
            let length = line.lengths[i + 1];
            if length > 0.0 {
                [(p[0] - b[0]) / length, (p[1] - b[1]) / length]
            } else {
                [0., 0.]
            }
        })
        .unwrap_or(direction);
    ConvoyPose {
        line: convoy.line,
        convoy: convoy.convoy,
        position: [x, height(x, z), z],
        facing: [
            direction[0] + (next[0] - direction[0]) * t,
            0.,
            direction[1] + (next[1] - direction[1]) * t,
        ],
    }
}

fn collides(convoys: &[SailingConvoy], index: usize) -> bool {
    let a = &convoys[index];
    convoys.iter().enumerate().any(|(i, b)| {
        i != index
            && b.active
            && (0..3)
                .map(|axis| (a.pose.position[axis] - b.pose.position[axis]).powi(2))
                .sum::<f32>()
                < (a.radius + b.radius).powi(2)
    })
}

impl ConvoyFleet {
    pub fn new(
        lines: &[ConvoyLine],
        defines: &ConvoyDefines,
        counter: &dyn Fn() -> u32,
        radius: &dyn Fn(&str) -> Option<f32>,
        height: &dyn Fn(f32, f32) -> f32,
    ) -> Self {
        let mut out = Self::default();
        for (index, line) in lines.iter().enumerate() {
            let points = convoy_path(&line.points, defines.route_offset);
            let lengths = points.windows(2).map(|p| span(p[0], p[1])).collect();
            out.lines.push(SailingLine {
                width: line.map_width,
                points,
                lengths,
                random: viewer_models::entity_animation::Random::from_counter(counter()),
            });
            let Some(radius) = radius(&line.entity) else {
                continue;
            };
            if line.points.len() < 2 {
                continue;
            }
            for convoy in 0..line.count {
                out.convoys.push(SailingConvoy {
                    line: index,
                    convoy,
                    segment: 0,
                    distance: 0.,
                    sleep: 0.,
                    active: false,
                    radius,
                    pose: ConvoyPose {
                        line: index,
                        convoy,
                        position: [0.; 3],
                        facing: [0.; 3],
                    },
                });
                let i = out.convoys.len() - 1;
                for _ in 0..5 {
                    let route = &mut out.lines[index];
                    let segment = route.random.next_word() as usize % route.lengths.len();
                    let distance = (route.random.next_word() as f32 * (1.0 / 2147483648.0))
                        * route.lengths[segment];
                    out.convoys[i].segment = segment;
                    out.convoys[i].distance = distance;
                    out.convoys[i].pose = pose(route, &out.convoys[i], height);
                    if !collides(&out.convoys, i) {
                        out.convoys[i].active = true;
                        break;
                    }
                }
            }
        }
        out
    }

    pub fn advance(
        &mut self,
        defines: &ConvoyDefines,
        time: f32,
        height: &dyn Fn(f32, f32) -> f32,
    ) -> Vec<ConvoyPose> {
        let delta = self.last.map_or(0., |last| (time - last).max(0.));
        self.last = Some(time);
        for i in 0..self.convoys.len() {
            let convoy = &mut self.convoys[i];
            let line = &mut self.lines[convoy.line];
            if !convoy.active {
                convoy.sleep -= delta;
                if convoy.sleep >= 0. {
                    continue;
                }
                convoy.segment = 0;
                convoy.distance = 0.;
                convoy.pose = pose(line, convoy, height);
                if collides(&self.convoys, i) {
                    let convoy = &mut self.convoys[i];
                    let line = &mut self.lines[convoy.line];
                    convoy.sleep =
                        (line.random.next_word() as f32 * (1.0 / 2147483648.0)) * defines.sleep;
                } else {
                    self.convoys[i].active = true;
                }
                continue;
            }
            convoy.distance += delta * defines.speed;
            while convoy.distance > line.lengths[convoy.segment] {
                convoy.distance -= line.lengths[convoy.segment];
                convoy.segment += 1;
                if convoy.segment == line.lengths.len() {
                    convoy.active = false;
                    convoy.sleep =
                        (line.random.next_word() as f32 * (1.0 / 2147483648.0)) * defines.sleep;
                    break;
                }
            }
            if convoy.active {
                convoy.pose = pose(line, convoy, height);
            }
        }
        self.convoys
            .iter()
            .filter(|c| c.active)
            .map(|c| c.pose)
            .collect()
    }
}

pub fn convoy_instances(
    assets: &UnitModelAssets,
    lines: &[ConvoyLine],
    poses: &[ConvoyPose],
    refused: &mut BTreeMap<String, usize>,
) -> Vec<(usize, ModelInstance)> {
    let mut slots: BTreeMap<&str, Option<usize>> = BTreeMap::new();
    let mut out = Vec::with_capacity(poses.len());
    for pose in poses {
        let line = &lines[pose.line];
        let slot = *slots
            .entry(line.entity.as_str())
            .or_insert_with(|| assets.bake_slot(&line.entity));
        let Some(slot) = slot else {
            *refused
                .entry(format!("convoy entity {} did not bake", line.entity))
                .or_default() += 1;
            continue;
        };
        let [fx, _, fz] = pose.facing;
        let identity = line.seed.wrapping_mul(31).wrapping_add(pose.convoy);
        out.push((
            slot,
            ModelInstance {
                position: pose.position,
                rotation: (-fx).atan2(fz),
                scale: 1.0,
                animation: viewer_models::entity_animation::IDLE,
                animation_seed: CONVOY_SEED ^ identity,
                animation_identity: identity.wrapping_add(1).max(1),
                ..ModelInstance::default()
            },
        ));
    }
    out
}

pub(crate) fn presentation_counter() -> u32 {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn QueryPerformanceCounter(value: *mut i64) -> i32;
            fn QueryPerformanceFrequency(value: *mut i64) -> i32;
        }
        let (mut count, mut frequency) = (0_i64, 0_i64);
        unsafe {
            QueryPerformanceCounter(&mut count);
            QueryPerformanceFrequency(&mut frequency);
        }
        if frequency > 0 {
            return (i128::from(count) * 1_000_000_000 / i128::from(frequency)) as u32;
        }
    }
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    fn defines() -> ConvoyDefines {
        ConvoyDefines {
            scale_factor: 1.,
            max: 5,
            speed: 1.,
            sleep: 3.,
            route_offset: 0.,
            cutoff: 100.,
        }
    }
    #[test]
    fn random_launch_collisions_and_sleep_follow_the_presentation_state() {
        let line = ConvoyLine {
            entity: "convoy".into(),
            map_width: 100.,
            points: vec![[0., 0.], [10., 0.], [10., 100.]],
            count: 1,
            seed: 0,
        };
        let defines = defines();
        let height = |_, _| 0.;
        let mut fleet = ConvoyFleet::new(
            &[line.clone(), line.clone()],
            &defines,
            &|| 7,
            &|_| Some(1000.),
            &height,
        );
        assert!(fleet.convoys[0].active);
        assert!(!fleet.convoys[1].active);
        assert!(fleet.convoys[0].distance > 0.);
        let initial = fleet.advance(&defines, 0., &height);
        assert_eq!(initial.len(), 1);
        fleet.convoys[1].sleep = 100.;
        let line_index = fleet.convoys[0].line;
        fleet.convoys[0].segment = fleet.lines[line_index].lengths.len() - 1;
        fleet.convoys[0].distance = fleet.lines[line_index].lengths.last().copied().unwrap();
        fleet.advance(&defines, 0.25, &height);
        assert!(!fleet.convoys[0].active);
        assert!(fleet.convoys[0].sleep >= 0. && fleet.convoys[0].sleep < defines.sleep);
        fleet.convoys[0].sleep = 0.;
        fleet.advance(&defines, 0.25, &height);
        assert!(!fleet.convoys[0].active);
        fleet.advance(&defines, 0.5, &height);
        assert!(fleet.convoys[0].active);
        assert_eq!(fleet.convoys[0].distance, 0.);
    }
}
