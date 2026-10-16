//! The viewer's province queues of floating map text, fed by the recorded
//! map events rather than native UI objects.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use eframe::wgpu;
use shader_pipeline::gpu;

use super::{Assets, Style, draw};
use crate::map::shader_scene;
use viewer_foundation::map::{
    camera::Camera, gpu_contract::Targets, native_gui::SpriteKind, native_resources::GuiResources,
    native_text, province_map::HeightField, province_schedule,
};

/// A recorded event the viewer admitted for its viewing country, with its
/// localised text. Never recorded itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    pub sequence: u64,
    /// The recorded event's family, which a viewer can hide.
    pub family: replay::rows::MapEventFamily,
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
    // Every producer gives its text this lifetime.
    const LIMIT: f32 = 6.0;

    /// The native ordered comparisons, including negative authored speeds.
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
            // A zero/negative or sub-ULP speed must not replay an arbitrarily long idle gap.
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
            // New events are first observed on this presentation boundary. Catch-up
            // ticks before their admission age older objects, never these births.
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
            // One pre-promotion step plus every active step from its promotion.
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

/// One map view's presentation. It shares the existing province schedule and animation_time;
/// it neither samples another clock nor stores native client presentation state.
pub struct Player {
    renderer: Result<draw::Renderer, String>,
    styles: BTreeMap<Style, Result<LoadedStyle, String>>,
    colors: BTreeMap<u8, [f32; 4]>,
    textures: Vec<wgpu::TextureView>,
    cutoff: Result<f32, String>,
    retained: shader_scene::State,
    provinces: BTreeMap<u32, Province>,
    schedule: province_schedule::Shared,
    clock: Option<Clock>,
    warnings: BTreeMap<String, usize>,
}

impl Player {
    pub fn new(
        assets: &Assets,
        resources: &GuiResources,
        textures: Vec<wgpu::TextureView>,
        samples: u32,
    ) -> Self {
        Self {
            renderer: assets.effect.clone().map(|effect| {
                draw::Renderer::new(
                    effect,
                    samples,
                    viewer_foundation::map::gpu_contract::SCENE_FORMAT,
                    viewer_foundation::map::gpu_contract::DEPTH_FORMAT,
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

    /// A seek/load/perspective change starts new local players. Continuous hour transitions
    /// preserve active events; their old simulation identities are never dereferenced.
    pub fn reset(&mut self) {
        self.provinces.clear();
        self.clock = None;
        self.warnings.clear();
    }

    /// Queue admitted popups at their provinces, each presented once by the
    /// events consumer ([`super::events::Consumer`]). `anchor` gives a province's
    /// first unit-stack anchor; `heights` is the map's height field and
    /// `gui` resolves inline icons.
    pub fn enqueue(
        &mut self,
        anchor: impl Fn(u32) -> Option<[f32; 3]>,
        heights: HeightField<'_>,
        gui: &GuiResources,
        events: impl IntoIterator<Item = Popup>,
    ) {
        let schedule = self.schedule.lock().unwrap();
        for event in events {
            // The text is created only on a province this view schedules.
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
                        for name in native_text::icon_lookup(name)
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
                                break; // Native lookup stops at the first available candidate.
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

    /// A fixed 60 Hz presentation step derived entirely from the animation
    /// time. The recording client's own phase is not claimed. A dump's
    /// same-time settling never advances a popup.
    pub fn advance(&mut self, camera: Camera, animation_time: f32) {
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
        let schedule = self.schedule.lock().unwrap();
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
            let Some(update) = schedule.update(province) else {
                continue;
            };
            if update == province_schedule::Update::OffScreen || camera.height >= cutoff {
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

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        caller: &mut shader_scene::State,
        camera: Camera,
        matrix: &[[f32; 4]; 4],
        native_scissor: Option<[u32; 4]>,
        shown: &dyn Fn(replay::rows::MapEventFamily) -> bool,
    ) -> bool {
        self.retained.inherit(caller);
        let retained = &mut self.retained;
        draw::bind_depth(retained); // The native empty pass also binds the graphics context's depth target.
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
            // A hidden kind keeps its queue, phase and lifetime; it only goes undrawn.
            .filter(|object| shown(object.event.family))
            .collect();
        // The draw groups objects by type bucket, keeping province and
        // activation order within a bucket.
        objects.sort_by_key(|object| {
            self.styles[&object.event.style]
                .as_ref()
                .map_or(usize::MAX, |style| style.definition.order)
        });
        let mut attempted = false;
        for object in objects {
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
                        },
                    },
                    &camera,
                    &sprite,
                )
                .map_err(|error| error.to_string())?;
                let renderer = self.renderer.as_ref().map_err(Clone::clone)?;
                let prepared = renderer
                    .prepare(arena, retained, &mesh, &camera, [1.0; 4], &|id| {
                        if id == viewer_foundation::map::sprites::NO_TEXTURE {
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
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
                // Unknown reached font/layout/style cannot certify later retained native draws.
                retained.stream.forget_program();
                retained.stream.forget_texture(
                    0,
                    installed_content::pdxshader::program::Stage::Pixel,
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

/// The popup's height: x and z rounded by adding .5 and truncating, each
/// index clamped, the raw byte scaled by .1 and clamped to sea level. It is
/// not the route sampler.
fn popup_height(heights: HeightField<'_>, x: f32, z: f32) -> f32 {
    let column = installed_content::crt::truncate_f32(x + 0.5).clamp(0, heights.width as i32 - 1) as usize;
    let bottom = installed_content::crt::truncate_f32(z + 0.5).clamp(0, heights.height as i32 - 1) as usize;
    let row = heights.height as usize - 1 - bottom;
    (f32::from(heights.values[row * heights.width as usize + column]) * 0.1)
        .max(viewer_foundation::map::camera::WATER_HEIGHT)
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
        province.advance(9, 9, |_| 0.5); // a dump's same-time warmup
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
        assert_eq!(clock.advance(1.005), None); // rewind inside the same60Hz bin
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
}
