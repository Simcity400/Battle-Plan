
use shader_pipeline::gpu::stats::CountedPasses as _;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use eframe::wgpu;
use shader_pipeline::gpu;

use super::{Assets, Style, draw};
use viewer_scene_base::shader_scene;
use viewer_gpu::{
    camera::Camera,
    gpu_contract::Targets,
    province_map::HeightField,
    province_schedule,
};
use viewer_native_gui::{
    free_icons,
    native_gui::SpriteKind,
    native_resources::GuiResources,
    native_text,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    pub sequence: u64,
    pub family: replay_data::rows::MapEventFamily,
    pub province: u32,
    pub style: Style,
    pub text: Vec<u8>,
}

struct Object {
    event: Popup,
    origin: [f32; 3],
    phase: f32,
    ready_at: Option<u64>,
    icons: BTreeMap<Vec<u8>, native_text::IconSprite>,
}

impl Object {
    const LIMIT: f32 = 6.0;

    fn step(&mut self, speed: f32) {
        let next = self.phase + speed;
        self.phase = next.clamp(0.0, Self::LIMIT);
    }

    fn alive(&self) -> bool {
        self.phase < Self::LIMIT
    }

    fn step_many(&mut self, speed: f32, count: u64) {
        for _ in 0..count {
            let before = self.phase;
            self.step(speed);
            if !self.alive() || self.phase == before {
                break;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Clock {
    start: f32,
    last: f32,
    tick: u64,
}

impl Clock {
    fn advance(&mut self, time: f32) -> Option<(u64, u64)> {
        if !time.is_finite() || time < self.last {
            return None;
        }
        let elapsed = (f64::from(time) - f64::from(self.start)) * 60.0;
        if !(0.0..(u64::MAX - 1) as f64).contains(&elapsed) {
            return None;
        }
        let current = (elapsed.floor() as u64).saturating_add(1);
        let previous = self.tick;
        self.last = time;
        self.tick = current;
        Some((previous, current))
    }
}

#[derive(Default)]
struct Province {
    queue: VecDeque<Object>,
    active: Vec<Object>,
}

impl Province {
    fn advance(&mut self, previous: u64, current: u64, speed: impl Fn(Style) -> f32) {
        for object in &mut self.active {
            object.step_many(speed(object.event.style), current.saturating_sub(previous));
        }
        self.active.retain(Object::alive);
        for object in &mut self.queue {
            object
                .ready_at
                .get_or_insert(current.max(previous.saturating_add(1)));
        }
        let mut promotion = previous.saturating_add(1);
        while let Some(front) = self.queue.front() {
            promotion = promotion.max(front.ready_at.expect("queued admission boundary"));
            if promotion > current {
                break;
            }
            let mut object = self.queue.pop_front().expect("queue front");
            object.step_many(speed(object.event.style), current - promotion + 2);
            if object.alive() {
                self.active.push(object);
            }
            promotion = promotion.saturating_add(1);
        }
    }
}

struct LoadedStyle {
    definition: super::content::Definition,
    font: Option<Arc<native_text::Font>>,
}

pub struct Player {
    renderer: Result<draw::Renderer, String>,
    styles: BTreeMap<Style, Result<LoadedStyle, String>>,
    colors: BTreeMap<u8, [f32; 4]>,
    textures: Vec<shader_pipeline::gpu::TextureBinding>,
    cutoff: Result<f32, String>,
    retained: shader_scene::State,
    provinces: BTreeMap<u32, Province>,
    schedule: province_schedule::Shared,
    clock: Option<Clock>,
    warnings: BTreeMap<String, usize>,
}

impl Player {
    pub fn warm(&self) {
        if let Ok(renderer) = &self.renderer {
            renderer.warm();
        }
    }

    pub fn new(
        assets: &Assets,
        resources: &GuiResources,
        textures: Vec<shader_pipeline::gpu::TextureBinding>,
        samples: u32,
    ) -> Self {
        Self {
            renderer: assets.effect.clone().map(|effect| {
                draw::Renderer::new(
                    effect,
                    samples,
                    viewer_gpu::gpu_contract::SCENE_FORMAT,
                    viewer_gpu::gpu_contract::DEPTH_FORMAT,
                )
            }),
            styles: assets
                .definitions
                .iter()
                .map(|(&style, definition)| {
                    (
                        style,
                        definition
                            .as_ref()
                            .map(|definition| LoadedStyle {
                                definition: definition.clone(),
                                font: resources
                                    .font(&definition.text.text_block.font)
                                    .cloned()
                                    .map(Arc::new),
                            })
                            .map_err(Clone::clone),
                    )
                })
                .collect(),
            colors: resources.text_colors().clone(),
            textures,
            cutoff: assets.cutoff.clone(),
            retained: Default::default(),
            provinces: BTreeMap::new(),
            schedule: Default::default(),
            clock: None,
            warnings: BTreeMap::new(),
        }
    }

    pub fn share_schedule(&mut self, schedule: &province_schedule::Shared) {
        self.schedule = schedule.clone();
    }

    pub fn reset(&mut self) {
        self.provinces.clear();
        self.clock = None;
        self.warnings.clear();
    }

    pub fn enqueue(
        &mut self,
        anchor: impl Fn(u32) -> Option<[f32; 3]>,
        heights: HeightField<'_>,
        gui: &GuiResources,
        events: impl IntoIterator<Item = Popup>,
    ) {
        let schedule = self.schedule.lock().unwrap();
        for event in events {
            if !schedule.dirty(event.province) {
                continue;
            }
            let supported = (|| {
                let style = self.styles[&event.style].as_ref().map_err(Clone::clone)?;
                let definition = &style.definition;
                let font = style.font.as_ref().ok_or_else(|| {
                    format!(
                        "font {:?} was not loaded",
                        String::from_utf8_lossy(&definition.text.text_block.font)
                    )
                })?;
                if !font.refusals.is_empty() {
                    return Err(format!(
                        "font {:?}: {}",
                        String::from_utf8_lossy(&definition.text.text_block.font),
                        font.refusals
                            .iter()
                            .map(|(_, reason)| reason.as_str())
                            .collect::<Vec<_>>()
                            .join("; ")
                    ));
                }
                if font.facts.aux_color != [1.0; 4] {
                    return Err(format!(
                        "font {:?}: nonwhite auxiliary colour requires shared GUI/font restoration history",
                        String::from_utf8_lossy(&definition.text.text_block.font)
                    ));
                }
                self.renderer.as_ref().map(|_| ()).map_err(Clone::clone)
            })();
            if let Err(reason) = supported {
                *self.warnings.entry(reason).or_default() += 1;
                continue;
            }
            let Some(anchor) = anchor(event.province) else {
                *self
                    .warnings
                    .entry(format!(
                        "province {} has no content unitstack slot0 anchor",
                        event.province
                    ))
                    .or_default() += 1;
                continue;
            };
            if let Err(error) = heights.validate() {
                *self.warnings.entry(error.to_string()).or_default() += 1;
                continue;
            }
            let origin = [
                anchor[0],
                popup_height(heights, anchor[0], anchor[2]),
                anchor[2],
            ];
            let mut icons = BTreeMap::new();
            let mut icon_refusal = None;
            if let Some(text) = native_text::decode(&event.text) {
                let mut at = 0;
                while at < text.len() {
                    let next = native_text::special(&text[at..]);
                    if let Some((_, native_text::Special::Icon(name))) = &next {
                        for name in content_reader_localisation::localisation::icon_lookup(name)
                            .0
                            .into_iter()
                            .filter(|_| !name.is_empty())
                        {
                            if let Some(sprite) = gui.sprite(&name) {
                                if !sprite.refusals.is_empty() {
                                    icon_refusal = Some(format!(
                                        "inline sprite {:?}: {}",
                                        String::from_utf8_lossy(&name),
                                        sprite
                                            .refusals
                                            .iter()
                                            .map(|(_, reason)| reason.as_str())
                                            .collect::<Vec<_>>()
                                            .join("; ")
                                    ));
                                    break;
                                }
                                icons.insert(
                                    name,
                                    (
                                        sprite.texture,
                                        if matches!(
                                            sprite.kind,
                                            SpriteKind::Progress { .. }
                                                | SpriteKind::CorneredTile { .. }
                                        ) {
                                            1
                                        } else {
                                            sprite.frames
                                        },
                                    ),
                                );
                                break;
                            } else if gui.registry().sprite(&name).is_some() {
                                icon_refusal = Some(format!(
                                    "inline sprite {:?} is registered but its metadata was not prepared",
                                    String::from_utf8_lossy(&name)
                                ));
                                break;
                            }
                        }
                    }
                    at += next.map_or(1, |(length, _)| length);
                }
            }
            if let Some(reason) = icon_refusal {
                *self.warnings.entry(reason).or_default() += 1;
                continue;
            }
            self.provinces
                .entry(event.province)
                .or_default()
                .queue
                .push_back(Object {
                    event,
                    origin,
                    phase: 0.0,
                    ready_at: None,
                    icons,
                });
        }
    }

    pub fn advance(
        &mut self,
        camera: Camera,
        animation_time: f32,
        frame: Option<&province_schedule::ProvinceFrame>,
    ) {
        if !animation_time.is_finite() {
            return;
        }
        if self.provinces.is_empty() {
            self.clock = None;
            return;
        }
        let clock = self.clock.get_or_insert(Clock {
            start: animation_time,
            last: animation_time,
            tick: 0,
        });
        let Some((previous, current)) = clock.advance(animation_time) else {
            self.reset();
            return;
        };
        let schedule = frame.is_none().then(|| self.schedule.lock().unwrap());
        let cutoff = match &self.cutoff {
            Ok(cutoff) => *cutoff,
            Err(reason) => {
                if !self.provinces.is_empty() {
                    self.warnings.entry(reason.clone()).or_insert(1);
                }
                self.provinces.clear();
                return;
            }
        };
        for (&province, objects) in &mut self.provinces {
            let update = match (frame, &schedule) {
                (Some(frame), _) => frame.update(province),
                (None, Some(schedule)) => schedule.update(province),
                (None, None) => unreachable!("an uncut frame locks the schedule"),
            };
            let Some(update) = update else {
                continue;
            };
            let height = objects
                .active
                .iter()
                .chain(&objects.queue)
                .next()
                .map_or(camera.height, |object| camera.icon_distance(object.origin));
            if update == province_schedule::Update::OffScreen || height >= cutoff {
                objects.queue.clear();
                objects.active.clear();
                continue;
            }
            objects.advance(previous, current, |style| {
                self.styles[&style]
                    .as_ref()
                    .expect("enqueue admitted this style")
                    .definition
                    .text
                    .speed
            });
        }
        self.provinces
            .retain(|_, province| !province.queue.is_empty() || !province.active.is_empty());
    }

    #[expect(clippy::too_many_arguments, reason = "the arena, encoder and targets, the caller's state, the camera and its scissor, and the event filter")]
    pub fn draw(
        &mut self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        caller: &mut shader_scene::State,
        camera: Camera,
        matrix: &[[f32; 4]; 4],
        native_scissor: Option<[u32; 4]>,
        shown: &dyn Fn(replay_data::rows::MapEventFamily) -> bool,
        ground: Option<&free_icons::Ground<'_>>,
    ) -> bool {
        let cutoff = self.cutoff.as_ref().copied().unwrap_or(f32::INFINITY);
        let camera_view = camera;
        self.retained.inherit(caller);
        let retained = &mut self.retained;
        draw::bind_depth(retained);
        let (right, up) = camera.world_text_axes();
        let camera = draw::Camera {
            right,
            up,
            view_projection: std::array::from_fn(|i| matrix[i / 4][i % 4]),
        };
        let mut objects: Vec<_> = self
            .provinces
            .values()
            .flat_map(|province| &province.active)
            .filter(|object| shown(object.event.family))
            .collect();
        objects.sort_by_key(|object| {
            self.styles[&object.event.style]
                .as_ref()
                .map_or(usize::MAX, |style| style.definition.order)
        });
        let mut attempted = false;
        for object in objects {
            let (reach, opacity) = free_look(&camera_view, object.origin, cutoff, ground);
            if opacity < 1.0 / 255.0 {
                continue;
            }
            attempted = true;
            let result = (|| {
                let style = self.styles[&object.event.style]
                    .as_ref()
                    .map_err(Clone::clone)?;
                let definition = &style.definition;
                let font = style
                    .font
                    .as_ref()
                    .ok_or_else(|| "content font was not loaded".to_owned())?;
                let sprite = |name: &[u8]| object.icons.get(name).copied();
                let mesh = draw::geometry(
                    &draw::Text {
                        font,
                        bytes: &object.event.text,
                        color: definition.text.text_block.color,
                        global_colors: &self.colors,
                        width: definition.text.max_width,
                        animation: draw::Animation {
                            origin: object.origin,
                            phase: object.phase,
                            limit: Object::LIMIT,
                            scale: definition.scale,
                            reach,
                            opacity,
                        },
                    },
                    &camera,
                    &sprite,
                )
                .map_err(|error| error.to_string())?;
                let renderer = self.renderer.as_ref().map_err(Clone::clone)?;
                let prepared = renderer
                    .prepare(arena, retained, &mesh, &camera, [1.0; 4], &|id| {
                        if id == viewer_native_gui::sprites::NO_TEXTURE {
                            Ok(None)
                        } else {
                            self.textures
                                .get(id)
                                .cloned()
                                .map(Some)
                                .ok_or_else(|| format!("font texture {id} is not resident"))
                        }
                    })
                    .map_err(|error| error.to_string())?;
                let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("animated simulation map text"),
                    color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
                    depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
                    ..Default::default()
                });
                draw::draw(&mut pass, &prepared, targets.world_size(), native_scissor)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(())
            })();
            if let Err(reason) = result {
                *self.warnings.entry(reason).or_default() += 1;
                retained.stream.forget_program();
                retained.stream.forget_texture(
                    0,
                    content_reader_gui::pdxshader::program::Stage::Pixel,
                    None,
                );
            }
        }
        caller.inherit(retained);
        attempted
    }

    pub fn warnings(&self) -> Vec<String> {
        self.warnings
            .iter()
            .map(|(reason, count)| {
                format!("Animated map text: {count} refused events/draws: {reason}")
            })
            .collect()
    }
}

pub const FREE_SIZE_SHARE: f32 = 0.5;

fn free_look(
    camera: &Camera,
    origin: [f32; 3],
    cutoff: f32,
    ground: Option<&free_icons::Ground<'_>>,
) -> (f32, f32) {
    if camera.free().is_none() {
        return (1.0, 1.0);
    }
    let distance = camera.icon_distance(origin);
    let (shrink, fade) = free_icons::distance_look(distance, cutoff, camera.object_range());
    let sight = ground.map_or(1.0, |ground| {
        free_icons::sight_alpha(ground, camera.eye(), origin, distance)
    });
    let reference = cutoff.min(camera.object_range()) * FREE_SIZE_SHARE;
    (distance / reference * shrink, fade * sight)
}

fn popup_height(heights: HeightField<'_>, x: f32, z: f32) -> f32 {
    let column =
        content_reader_parser::crt::truncate_f32(x + 0.5).clamp(0, heights.width as i32 - 1) as usize;
    let bottom =
        content_reader_parser::crt::truncate_f32(z + 0.5).clamp(0, heights.height as i32 - 1) as usize;
    let row = heights.height as usize - 1 - bottom;
    (f32::from(heights.values[row * heights.width as usize + column]) * 0.1)
        .max(viewer_gpu::camera::WATER_HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(sequence: u64) -> Object {
        Object {
            event: Popup {
                sequence,
                family: Default::default(),
                province: 3,
                style: Style::CombatLoss,
                text: b"event".to_vec(),
            },
            origin: [0.0; 3],
            phase: 0.0,
            ready_at: None,
            icons: BTreeMap::new(),
        }
    }

    #[test]
    fn catch_up_ages_existing_events_without_backdating_new_arrivals() {
        let mut province = Province {
            queue: VecDeque::from([object(1), object(2)]),
            ..Default::default()
        };
        province.advance(0, 1, |_| 0.5);
        assert_eq!(
            province
                .active
                .iter()
                .map(|object| (object.event.sequence, object.phase))
                .collect::<Vec<_>>(),
            [(1, 1.0)]
        );
        assert_eq!(province.queue.len(), 1);
        province.queue.push_back(object(3));
        province.advance(1, 9, |_| 0.5);
        assert_eq!(
            province
                .active
                .iter()
                .map(|object| (object.event.sequence, object.phase))
                .collect::<Vec<_>>(),
            [(1, 5.0), (2, 4.5), (3, 1.0)]
        );
        province.advance(9, 9, |_| 0.5);
        assert_eq!(province.active[2].phase, 1.0);
        province.advance(9, 19, |_| 0.5);
        assert!(province.active.is_empty());
    }

    #[test]
    fn local_clock_refuses_rewind_and_extreme_time_without_unsigned_overflow() {
        let mut clock = Clock {
            start: 1.0,
            last: 1.0,
            tick: 0,
        };
        assert_eq!(clock.advance(1.0), Some((0, 1)));
        assert_eq!(clock.advance(1.0), Some((1, 1)));
        assert_eq!(clock.advance(1.01), Some((1, 1)));
        assert_eq!(clock.advance(1.005), None);
        assert_eq!(clock.advance(f32::MAX), None);
        assert_eq!(clock.advance(f32::NAN), None);
    }

    #[test]
    fn stalled_speed_skips_idle_ticks_and_height_uses_native_nearest_sample() {
        let mut object = object(1);
        object.step_many(0.0, u64::MAX);
        object.step_many(-0.01, u64::MAX);
        assert_eq!(object.phase, 0.0);
        let heights = HeightField {
            width: 2,
            height: 2,
            values: &[200, 220, 100, 140],
        };
        assert_eq!(popup_height(heights, 0.49, 0.49), 10.0);
        assert_eq!(popup_height(heights, 0.5, 0.5), 22.0);
        assert_eq!(popup_height(heights, -100.0, 100.0), 20.0);
    }

    #[test]
    fn a_free_eye_keeps_a_text_s_screen_size_and_fades_it_behind_hills() {
        use viewer_gpu::camera::{Free, Lens, MapDefines, MapSize, Placement, Pose};
        use viewer_gpu::terrain_sight::TerrainSight;
        let camera = |eye: [f32; 3], target: [f32; 3]| {
            Placement {
                center: [eye[0], eye[2]],
                height: eye[1],
                pose: Pose::Free(Free::looking_at(
                    eye,
                    target,
                    Lens {
                        fov_y: 0.9,
                        near: 0.5,
                        far: 3000.0,
                    },
                )),
            }
            .on(MapSize::TEST, MapDefines::test())
        };
        let origin = [1300.0, 10.0, 1000.0];
        let map = Placement::map([1300.0, 1000.0], 200.0).on(MapSize::TEST, MapDefines::test());
        assert_eq!(free_look(&map, origin, 500.0, None), (1.0, 1.0));
        let at = |distance: f32| camera([1300.0 - distance, 10.0, 1000.0], origin);
        let (reach, opacity) = free_look(&at(250.0), origin, 500.0, None);
        let (shrink, _) = free_icons::distance_look(250.0, 500.0, 1100.0);
        assert!((reach - shrink).abs() < 1e-4, "{reach} {shrink}");
        assert_eq!(opacity, 1.0);
        let (near_reach, _) = free_look(&at(100.0), origin, 500.0, None);
        let near_shrink = free_icons::distance_look(100.0, 500.0, 1100.0).0;
        assert!(
            (near_reach - 0.4 * near_shrink).abs() < 1e-4,
            "{near_reach}"
        );
        assert!(free_look(&at(460.0), origin, 500.0, None).1 < 1.0);
        assert_eq!(free_look(&at(500.0), origin, 500.0, None).1, 0.0);
        let (width, rows) = (MapSize::TEST.width, MapSize::TEST.height);
        let mut values = vec![95u8; (width * rows) as usize];
        for bottom in 0..rows {
            let row = (rows - 1 - bottom) as usize;
            for column in 1240..1250 {
                values[row * width as usize + column] = 220;
            }
        }
        let field = HeightField {
            width,
            height: rows,
            values: &values,
        };
        let sight = TerrainSight::new(&field);
        let ground = free_icons::Ground {
            field,
            sight: &sight,
        };
        let low = camera([1200.0, 12.0, 1000.0], origin);
        assert_eq!(free_look(&low, origin, 500.0, Some(&ground)).1, 0.0);
        let high = camera([1200.0, 300.0, 1000.0], origin);
        assert_eq!(free_look(&high, origin, 500.0, Some(&ground)).1, 1.0);
    }
}
