
mod membership;
#[path = "province_colors.rs"]
mod province_colors;
pub use province_colors::ProvinceColors;

use std::sync::MutexGuard;
use viewer_gpu::camera::MapSize;
use viewer_gpu::gradient_math::DIVISOR;
use viewer_gpu::gradient_math::FieldDefines;
use viewer_gpu::textures::gradient_size;
use viewer_runtime::profiling::span;

use viewer_gpu::gpu_contract::{compute_limits, unsupported};

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, Mutex, Weak},
};

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;

use map_display_data::landscape::{Draw, ModeLayer, Source, Style};
use super::map_mode::POLITICAL;
use map_display_data::landscape::Regions;
use viewer_scene_base::gradient::{self, GRID_CELL, Layer};
use viewer_gpu::province_map::ProvinceMap;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
    stride: u32,
    packed_ids: u32,
    count: u32,
    dispatch_stride: u32,
    opacity: u32,
    no_color: u32,
    distance_cap: u32,
    distance_in_b: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Pod, Zeroable)]
struct Region {
    color: u32,
    alpha_offset: u32,
    alpha_count: u32,
    radius: u32,
    attributes: u32,
    fx: u32,
    fill: u32,
}

fn style_words(style: gradient::RegionStyle) -> (u32, u32) {
    (
        u32::from(style.alpha) | (u32::from(style.pulse) << 8),
        style.fill.to_bits(),
    )
}

#[derive(Clone, Copy)]
struct Province {
    pixels: u64,
    bounds: [u32; 4],
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> io::Result<wgpu::Buffer> {
    check_size(device, label, bytes.len().max(4) as u64, usage)?;
    Ok(
        shader_pipeline::gpu::stats::create_buffer_init(&device, &wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: if bytes.is_empty() { &[0; 4] } else { bytes },
            usage,
        }),
    )
}

fn check_size(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> io::Result<()> {
    let limits = device.limits();
    if size > limits.max_buffer_size
        || (usage.contains(wgpu::BufferUsages::STORAGE)
            && size > limits.max_storage_buffer_binding_size)
    {
        return Err(unsupported(format!(
            "{label} needs {size} bytes, device storage limit {}",
            limits.max_storage_buffer_binding_size
        )));
    }
    Ok(())
}

fn zero_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> io::Result<wgpu::Buffer> {
    check_size(device, label, size, usage)?;
    Ok(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    }))
}

const REGION_BYTES: u64 = std::mem::size_of::<Region>() as u64;

fn rgb(color: [u8; 3]) -> u32 {
    u32::from_le_bytes([color[0], color[1], color[2], 0])
}

pub fn check_device(device: &wgpu::Device) -> io::Result<()> {
    if !compute_limits(&device.limits()) {
        return Err(unsupported(
            "device lacks compute with eight storage buffers and 64-thread workgroups",
        ));
    }
    Ok(())
}

#[derive(Default)]
pub struct Cache(Mutex<Vec<(wgpu::Device, Weak<Static>)>>);

pub struct Inputs<'a> {
    pub map: ProvinceMap<'a>,
    pub strategic: &'a [u32],
    pub field: &'a FieldDefines,
}

impl Cache {
    pub fn warm(
        &self,
        device: &wgpu::Device,
        inputs: Inputs<'_>,
    ) -> io::Result<Arc<Static>> {
        let mut entries = self.0.lock().unwrap_or_else(|error| error.into_inner());
        entries.retain(|(_, value)| value.strong_count() != 0);
        if let Some(shared) = entries
            .iter()
            .find_map(|(owner, value)| (owner == device).then(|| value.upgrade()).flatten())
        {
            return Ok(shared);
        }
        let shared = Arc::new(Static::new(device, inputs)?);
        entries.push((device.clone(), Arc::downgrade(&shared)));
        Ok(shared)
    }
}

pub struct Static {
    strategic: PoliticalGpu,
    size: [u32; 2],
    field: FieldDefines,
    strategic_keys: Vec<u32>,
    registry: Mutex<Registry>,
    #[cfg(test)]
    race_checks: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
struct Registry {
    states: Vec<Weak<State>>,
    pool: Vec<Arc<State>>,
    views: usize,
}

impl Registry {
    fn keep(&mut self, state: Arc<State>) {
        if Arc::strong_count(&state) == 1 && self.pool.len() < self.views {
            self.pool.push(state);
        }
    }
}

impl Static {
    fn new(device: &wgpu::Device, inputs: Inputs<'_>) -> io::Result<Self> {
        let Inputs {
            map,
            strategic,
            field,
        } = inputs;
        let color = |_| field.navy_neutral;
        let mut layer = Layer::new(
            gradient::LayerKind::Fixed(field.strategic_thickness),
            strategic,
            &color,
            field,
        );
        layer.region_opacity = 0;
        layer.no_region_color = field.navy_neutral;
        let gpu = PoliticalGpu::new(device, map, &layer)?;
        Ok(Self {
            strategic: gpu,
            size: gradient_size(MapSize {
                width: map.width,
                height: map.height,
            }),
            field: *field,
            strategic_keys: strategic.to_vec(),
            registry: Mutex::new(Registry::default()),
            #[cfg(test)]
            race_checks: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    fn state(&self, device: &wgpu::Device) -> io::Result<Arc<State>> {
        let empty = vec![0; self.strategic.provinces.len()];
        let color = |_| [0; 3];
        let layer = Layer::new(gradient::LayerKind::Country, &empty, &color, &self.field);
        let gpu = self
            .strategic
            .layer_with(device, &layer, Some(Arc::clone(&self.strategic.scratch)))?;
        let mode = self
            .strategic
            .layer_with(device, &layer, Some(Arc::clone(&self.strategic.scratch)))?;
        let [width, height] = self.size;
        let textures = ["GradientBorderChannel1", "GradientBorderChannel2"].map(|label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        let layer = Layer::new(
            gradient::LayerKind::Fixed(self.field.strategic_thickness),
            &self.strategic_keys,
            &color,
            &self.field,
        );
        let naval = self
            .strategic
            .layer_with(device, &layer, Some(Arc::clone(&self.strategic.scratch)))?;
        let state = Arc::new(State {
            textures,
            fill: Mutex::new(Fill {
                gpu,
                naval,
                mode,
                channels: POLITICAL,
                shown: None,
                strategic_styles: Arc::new(BTreeMap::new()),
            }),
        });
        let mut registry = self.registry();
        registry.states.retain(|state| state.strong_count() != 0);
        registry.states.push(Arc::downgrade(&state));
        Ok(state)
    }

    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    #[cfg(test)]
    fn race_point(&self) {
        let excluded = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    matches!(
                        self.registry.try_lock(),
                        Err(std::sync::TryLockError::WouldBlock)
                    )
                })
                .join()
                .unwrap()
        });
        assert!(
            excluded,
            "the registry must be held across this critical section"
        );
        self.race_checks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn find(&self, regions: &Regions, styles: &BTreeMap<u32, Style>) -> Option<Arc<State>> {
        let mut registry = self.registry();
        let found = registry
            .states
            .iter()
            .filter_map(Weak::upgrade)
            .find(|state| {
                let fill = state.lock();
                fill.strategic_styles.as_ref() == styles
                    && fill
                        .shown
                        .as_ref()
                        .is_some_and(|shown| same_fill(shown, regions))
            });
        #[cfg(test)]
        self.race_point();
        if let Some(found) = &found {
            registry.pool.retain(|state| !Arc::ptr_eq(state, found));
        }
        found
    }

    fn recycle(&self) -> Option<Arc<State>> {
        let mut registry = self.registry();
        let state = registry.pool.pop()?;
        state.lock().shown = None;
        Some(state)
    }

    fn release(&self, state: Arc<State>) {
        let mut registry = self.registry();
        #[cfg(test)]
        if Arc::strong_count(&state) == 1 {
            self.race_point();
        }
        registry.keep(state);
    }

    fn claim(&self, state: &Arc<State>) -> bool {
        let _registry = self.registry();
        if Arc::strong_count(state) != 1 {
            return false;
        }
        state.lock().shown = None;
        true
    }

    #[cfg(test)]
    fn pooled(&self) -> usize {
        self.registry().pool.len()
    }

    #[cfg(test)]
    fn live_states(&self) -> usize {
        self.registry()
            .states
            .iter()
            .filter(|state| state.strong_count() != 0)
            .count()
    }
}

fn same_fill(a: &Regions, b: &Regions) -> bool {
    std::ptr::eq(a, b)
        || (a.country_of == b.country_of
            && a.country_colors == b.country_colors
            && a.strategic_attributes == b.strategic_attributes
            && a.mode == b.mode)
}

pub struct State {
    textures: [wgpu::Texture; 2],
    fill: Mutex<Fill>,
}

struct Fill {
    gpu: PoliticalGpu,
    naval: PoliticalGpu,
    mode: PoliticalGpu,
    channels: [Source; 2],
    shown: Option<Arc<Regions>>,
    strategic_styles: Arc<BTreeMap<u32, Style>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Naval,
    Country,
    Mode,
}

fn holders(channels: [Source; 2]) -> [Held; 2] {
    let empty = if channels.contains(&Source::Mode) {
        Held::Country
    } else {
        Held::Mode
    };
    channels.map(|source| match source {
        Source::Strategic => Held::Naval,
        Source::Country => Held::Country,
        Source::Mode => Held::Mode,
        Source::Second => Held::Country,
        Source::Empty => empty,
    })
}

impl State {
    fn lock(&self) -> MutexGuard<'_, Fill> {
        self.fill.lock().unwrap_or_else(|error| error.into_inner())
    }

    #[cfg(test)]
    pub fn bytes(&self) -> u64 {
        let textures: u64 = self
            .textures
            .iter()
            .map(|texture| {
                let size = texture.size();
                u64::from(size.width) * u64::from(size.height) * 4
            })
            .sum();
        let fill = self.lock();
        textures + fill.gpu.own_bytes() + fill.naval.own_bytes() + fill.mode.own_bytes()
    }
}

pub struct View {
    pulses: std::cell::Cell<Option<bool>>,
    base: Arc<Static>,
    spare: Option<Arc<State>>,
    shown: Option<Arc<State>>,
    strategic_styles: Arc<BTreeMap<u32, Style>>,
}

impl View {
    pub fn new(base: Arc<Static>, device: &wgpu::Device) -> io::Result<Self> {
        base.registry().views += 1;
        let mut view = Self {
            pulses: std::cell::Cell::new(None),
            base,
            spare: None,
            shown: None,
            strategic_styles: Arc::new(BTreeMap::new()),
        };
        view.spare = Some(match view.base.recycle() {
            Some(state) => state,
            None => view.base.state(device)?,
        });
        Ok(view)
    }

    #[cfg(test)]
    pub fn state_bytes(&self) -> Option<u64> {
        self.shown.as_ref().map(|state| state.bytes())
    }

    pub fn textures(&self) -> Option<&[wgpu::Texture; 2]> {
        self.shown.as_ref().map(|state| &state.textures)
    }

    pub fn style_pulses(&self) -> bool {
        if let Some(pulses) = self.pulses.get() {
            return pulses;
        }
        let pulses = self.shown.as_ref().is_some_and(|state| {
            let fill = state.lock();
            holders(fill.channels).into_iter().any(|layer| {
                let gpu = match layer {
                    Held::Naval => &fill.naval,
                    Held::Country => &fill.gpu,
                    Held::Mode => &fill.mode,
                };
                gpu.keys
                    .values()
                    .any(|&index| gpu.regions[index].fx & 256 != 0)
            })
        });
        self.pulses.set(Some(pulses));
        pulses
    }

    pub fn set_strategic_styles(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        styles: &BTreeMap<u32, Style>,
    ) -> io::Result<bool> {
        if self.strategic_styles.as_ref() != styles {
            self.strategic_styles = Arc::new(styles.clone());
        }
        let regions = self
            .shown
            .as_ref()
            .and_then(|state| state.lock().shown.clone());
        match regions {
            Some(regions) => self.show(device, queue, &regions),
            None => Ok(false),
        }
    }

    pub fn show_changed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        region_of: &[u32],
        color: &dyn Fn(u32) -> [u8; 3],
        provinces: &[usize],
        keys: &[u32],
    ) -> io::Result<bool> {
        let Some(state) = self.shown.as_ref().filter(|state| Arc::strong_count(state) == 1) else { return Ok(false) };
        let mut fill = state.lock();
        if fill.channels != POLITICAL || fill.shown.as_ref().is_some_and(|shown| shown.mode.is_some()) {
            return Ok(false);
        }
        let field = self.base.field;
        let layer = Layer::new(gradient::LayerKind::Country, region_of, color, &field);
        fill.gpu.update_changed(device, queue, &layer, provinces, keys)?;
        self.pulses.set(None);
        fill.shown = None;
        Ok(true)
    }

    pub fn show_naval(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        attributes: &dyn Fn(u32) -> map_display_data::landscape::Attributes,
        keys: &[u32],
    ) -> io::Result<bool> {
        let Some(state) = self.shown.as_ref().filter(|state| Arc::strong_count(state) == 1) else { return Ok(false) };
        let mut fill = state.lock();
        if fill.channels != POLITICAL || fill.shown.as_ref().is_some_and(|shown| shown.mode.is_some()) {
            return Ok(false);
        }
        let selected_style = |key| self.strategic_styles.get(&key);
        let navy = |key| selected_style(key).map_or(self.base.field.navy_neutral, |s| [s.color[0], s.color[1], s.color[2]]);
        let style = |key| {
            let value = selected_style(key);
            gradient::RegionStyle { alpha: value.map_or(0, |s| s.color[3]), pulse: value.is_some_and(|s| s.pulse), fill: value.map_or(1.0, |s| s.fill), thickness: None }
        };
        let field = self.base.field;
        let mut layer = Layer::new(gradient::LayerKind::Fixed(field.strategic_thickness), &self.base.strategic_keys, &navy, &field);
        layer.region_opacity = 0;
        layer.no_region_color = field.navy_neutral;
        layer.region_attributes = Some(attributes);
        if !self.strategic_styles.is_empty() {
            layer.region_style = Some(&style);
        }
        fill.naval.update_changed(device, queue, &layer, &[], keys)?;
        self.pulses.set(None);
        fill.shown = None;
        Ok(true)
    }

    pub fn show(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        regions: &Arc<Regions>,
    ) -> io::Result<bool> {
        self.pulses.set(None);
        if let Some(state) = &self.shown
            && {
                let fill = state.lock();
                fill.strategic_styles == self.strategic_styles
                    && fill
                        .shown
                        .as_ref()
                        .is_some_and(|shown| same_fill(shown, regions))
            }
        {
            return Ok(false);
        }
        let found = {
            let _span = span("political_find");
            self.base.find(regions, &self.strategic_styles)
        };
        let state = if let Some(state) = found {
            if let Some(spare) = self.spare.take() {
                self.base.release(spare);
            }
            state
        } else {
            let state = {
                let _span = span("political_acquire");
                let own = self
                    .shown
                    .as_ref()
                    .filter(|state| self.base.claim(state))
                    .cloned();
                match own
                    .or_else(|| self.spare.take())
                    .or_else(|| self.base.recycle())
                {
                    Some(state) => state,
                    None => self.base.state(device)?,
                }
            };
            let mut fill = state.lock();
            let channels = regions
                .mode
                .as_ref()
                .map_or(POLITICAL, |draw| draw.channels);
            if fill.channels != channels {
                let (was, now) = (holders(fill.channels), holders(channels));
                let moved = |layer: Held| was.map(|held| held == layer) != now.map(|held| held == layer);
                if moved(Held::Country) {
                    fill.gpu.refill();
                }
                if moved(Held::Naval) {
                    fill.naval.refill();
                }
                if moved(Held::Mode) {
                    fill.mode.refill();
                }
                fill.channels = channels;
            }
            let held = holders(channels);
            let empty = vec![0; fill.mode.mapping.len()];
            let field = self.base.field;
            let blank = |_| [0; 3];
            let empty_layer = Layer::new(gradient::LayerKind::Country, &empty, &blank, &field);
            {
                let _span = span("political_country");
                match regions.mode.as_deref() {
                    Some(Draw {
                        second: Some(second),
                        ..
                    }) if channels.contains(&Source::Second) => {
                        fill.gpu.update_mode(device, queue, second)?
                    }
                    _ if !channels.contains(&Source::Country) && held.contains(&Held::Country) => {
                        fill.gpu.update(device, queue, &empty_layer)?
                    }
                    Some(Draw {
                        country: Some(variant),
                        ..
                    }) => fill.gpu.update_mode(device, queue, variant)?,
                    _ => fill.gpu.update_regions(device, queue, regions)?,
                }
            }
            {
                let _span = span("political_mode");
                match regions.mode.as_ref().and_then(|draw| draw.layer.as_ref()) {
                    Some(layer) if channels.contains(&Source::Mode) => {
                        fill.mode.update_mode(device, queue, layer)?
                    }
                    _ => fill.mode.update(device, queue, &empty_layer)?,
                }
            }
            let naval_span = span("political_naval");
            let mode = regions
                .mode
                .as_ref()
                .and_then(|draw| draw.strategic.as_ref());
            let selected_style = |key| {
                self.strategic_styles
                    .get(&key)
                    .or_else(|| mode.and_then(|mode| mode.styles.get(&key)))
            };
            let navy = |key| {
                selected_style(key).map_or(self.base.field.navy_neutral, |s| {
                    [s.color[0], s.color[1], s.color[2]]
                })
            };
            let attributes = |id| {
                if mode.is_some_and(|mode| !mode.naval_fx) {
                    return map_display_data::landscape::Attributes::default();
                }
                regions
                    .strategic_attributes
                    .get(&id)
                    .copied()
                    .unwrap_or_default()
            };
            let style = |key| {
                let value = selected_style(key);
                gradient::RegionStyle {
                    alpha: value.map_or(0, |s| s.color[3]),
                    pulse: value.is_some_and(|s| s.pulse),
                    fill: value.map_or(1.0, |s| s.fill),
                    thickness: None,
                }
            };
            let mut layer = Layer::new(
                gradient::LayerKind::Fixed(self.base.field.strategic_thickness),
                mode.map_or(self.base.strategic_keys.as_slice(), |mode| {
                    mode.region_of.as_slice()
                }),
                &navy,
                &self.base.field,
            );
            layer.region_opacity = 0;
            layer.no_region_color = self.base.field.navy_neutral;
            layer.region_attributes = Some(&attributes);
            if mode.is_some() || !self.strategic_styles.is_empty() {
                layer.region_style = Some(&style);
            }
            fill.naval.update(device, queue, &layer)?;
            drop(naval_span);
            fill.strategic_styles = Arc::clone(&self.strategic_styles);
            fill.shown = Some(Arc::clone(regions));
            drop(fill);
            state
        };
        let changed = !self
            .shown
            .as_ref()
            .is_some_and(|shown| Arc::ptr_eq(shown, &state));
        if let Some(previous) = self.shown.replace(state)
            && changed
        {
            self.base.release(previous);
        }
        Ok(changed)
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) -> Option<Encoded> {
        let state = self.shown.as_ref()?;
        let [first, second] = &state.textures;
        let mut fill = state.lock();
        let held = holders(fill.channels);
        let half = first.height() / 2;
        let rows = |layer: Held| {
            [0, half]
                .into_iter()
                .zip(held)
                .filter_map(|(row, holder)| (holder == layer).then_some(row))
                .collect::<Vec<_>>()
        };
        let fill = &mut *fill;
        let mut layers = [false; 3];
        for (recorded, (layer, gpu)) in layers.iter_mut().zip([
            (Held::Naval, &mut fill.naval),
            (Held::Country, &mut fill.gpu),
            (Held::Mode, &mut fill.mode),
        ]) {
            let rows = rows(layer);
            if !rows.is_empty() {
                *recorded = gpu.encode_rows(encoder, [first, second], &rows);
            }
        }
        layers.contains(&true).then(|| Encoded {
            state: Arc::clone(state),
            layers,
            submitted: false,
        })
    }
}

#[must_use = "a recorded fill stays pending until `submitted` follows its encoder's submission"]
pub struct Encoded {
    state: Arc<State>,
    layers: [bool; 3],
    submitted: bool,
}

impl Encoded {
    pub fn submitted(mut self) {
        self.submitted = true;
    }
}

impl Drop for Encoded {
    fn drop(&mut self) {
        let mut fill = self.state.lock();
        let fill = &mut *fill;
        for (&recorded, gpu) in
            self.layers
                .iter()
                .zip([&mut fill.naval, &mut fill.gpu, &mut fill.mode])
        {
            if recorded {
                gpu.recorded(self.submitted);
            }
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        let held = [self.shown.take(), self.spare.take()];
        let mut registry = self.base.registry();
        registry.views -= 1;
        let views = registry.views;
        registry.pool.truncate(views);
        for state in held.into_iter().flatten() {
            registry.keep(state);
        }
    }
}

struct Scratch {
    distances: wgpu::Buffer,
    output: wgpu::Buffer,
}

impl Scratch {
    fn new(device: &wgpu::Device, params: &Params) -> io::Result<Self> {
        let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let cells = u64::from(params.columns) * u64::from(params.rows);
        Ok(Self {
            distances: zero_buffer(device, "political cell distances", cells * 4, rw)?,
            output: zero_buffer(
                device,
                "political output bytes",
                u64::from(params.stride) * u64::from(params.height) * 4,
                rw | wgpu::BufferUsages::COPY_SRC,
            )?,
        })
    }

    #[cfg(test)]
    fn bytes(&self) -> u64 {
        self.distances.size() + self.output.size()
    }
}

struct Kernel {
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::ComputePipeline; 5],
}

impl Kernel {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("political compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("political_fill.wgsl").into()),
        });
        let entries = (0..9)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 0 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: !matches!(binding, 5..=7),
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect::<Vec<_>>();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("political compute"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("political compute"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = [
            "boundaries",
            "distance_init",
            "distance_to_b",
            "distance_to_a",
            "field",
        ]
        .map(|entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        Self { layout, pipelines }
    }
}

pub struct PoliticalGpu {
    params: Params,
    uniform: wgpu::Buffer,
    ids: wgpu::Buffer,
    provinces: Arc<[Province]>,
    mapping: Vec<u32>,
    members: Vec<Vec<u32>>,
    member_at: Vec<u32>,
    source_keys: Vec<u32>,
    mapping_buffer: wgpu::Buffer,
    keys: HashMap<u32, usize>,
    free: Vec<usize>,
    areas: Vec<u64>,
    thickness: Vec<(u32, u32)>,
    regions: Vec<Region>,
    region_buffer: wgpu::Buffer,
    alpha: Vec<u8>,
    alpha_buffer: wgpu::Buffer,
    alpha_stride: usize,
    capacity: usize,
    alpha_keys: HashMap<(u32, u32), usize>,
    alpha_users: HashMap<(u32, u32), u32>,
    alpha_free: Vec<usize>,
    alpha_slots: usize,
    cells: wgpu::Buffer,
    scratch: Arc<Scratch>,
    dirty_buffer: wgpu::Buffer,
    indirect: wgpu::Buffer,
    dirty: Vec<bool>,
    marked: Vec<u32>,
    copies: Vec<[u32; 4]>,
    group: wgpu::BindGroup,
    kernel: Arc<Kernel>,
    field: FieldDefines,
    kind: gradient::LayerKind,
    initialized: bool,
    pending: bool,
    recorded: bool,
    pub dispatched_cells: usize,
}

impl PoliticalGpu {
    pub fn new(device: &wgpu::Device, map: ProvinceMap<'_>, layer: &Layer<'_>) -> io::Result<Self> {
        Self::new_shared(device, map, layer, None, None)
    }

    #[cfg(test)]
    pub fn new_layer(&self, device: &wgpu::Device, layer: &Layer<'_>) -> io::Result<Self> {
        self.layer_with(device, layer, None)
    }

    fn layer_with(
        &self,
        device: &wgpu::Device,
        layer: &Layer<'_>,
        scratch: Option<Arc<Scratch>>,
    ) -> io::Result<Self> {
        Self::new_shared(
            device,
            ProvinceMap {
                width: self.params.width,
                height: self.params.height,
                ids: &[],
            },
            layer,
            Some(self),
            scratch,
        )
    }

    fn new_shared(
        device: &wgpu::Device,
        map: ProvinceMap<'_>,
        layer: &Layer<'_>,
        shared: Option<&Self>,
        scratch: Option<Arc<Scratch>>,
    ) -> io::Result<Self> {
        if shared.is_none() {
            map.validate()?;
        }
        let limits = device.limits();
        check_device(device)?;
        if !map.width.is_multiple_of(DIVISOR)
            || !map.height.is_multiple_of(DIVISOR)
            || map.width / 2 > limits.max_texture_dimension_2d
            || u64::from(map.height) + 2 > u64::from(limits.max_texture_dimension_2d)
            || map.width > u16::MAX as u32
            || map.height > u16::MAX as u32
        {
            return Err(unsupported(format!(
                "map dimensions {}x{} exceed the texture/coordinate range or are not divisible by {}",
                map.width, map.height, DIVISOR
            )));
        }
        let field = *layer.field;
        let max_thickness = maximum_thickness(layer.kind, &field);
        let radius = search_radius(max_thickness, &field, map.width, map.height);
        let reach = gradient::field_reach(max_thickness, &field);
        let dx = (map.width / 2).min((radius + 1) * GRID_CELL);
        let dy = (map.height - 1).min((radius + 1) * GRID_CELL);
        let maximum_squared = u64::from(dx).pow(2) + u64::from(dy).pow(2);
        let lookup_squared = ((f64::from(reach).max(0.0).powi(2).ceil() as u64).saturating_add(2))
            .min(maximum_squared);
        let alpha_stride = (lookup_squared + 1).next_multiple_of(4);
        check_size(
            device,
            "political exact alpha",
            alpha_stride,
            wgpu::BufferUsages::STORAGE,
        )?;
        if alpha_stride > u64::from(u32::MAX) {
            return Err(unsupported("alpha lookup exceeds shader addressing range"));
        }
        let alpha_stride = alpha_stride as usize;
        let largest = shared.map_or_else(
            || map.ids.iter().copied().max().unwrap_or(0) as usize,
            |map| map.provinces.len() - 1,
        );
        if largest >= layer.region_of.len() {
            return Err(unsupported("province ID is outside the region table"));
        }
        if (largest as u64 + 1) * 4 > limits.max_storage_buffer_binding_size {
            return Err(unsupported("province ID table exceeds storage limit"));
        }
        let packed = largest <= u16::MAX as usize;
        let (provinces, ids) = if let Some(shared) = shared {
            (Arc::clone(&shared.provinces), shared.ids.clone())
        } else {
            let mut provinces = vec![
                Province {
                    pixels: 0,
                    bounds: [u32::MAX, u32::MAX, 0, 0]
                };
                largest + 1
            ];
            for (at, &id) in map.ids.iter().enumerate() {
                let province = &mut provinces[id as usize];
                province.pixels += 1;
                let x = at as u32 % map.width / GRID_CELL;
                let y = at as u32 / map.width / GRID_CELL;
                province.bounds[0] = province.bounds[0].min(x);
                province.bounds[1] = province.bounds[1].min(y);
                province.bounds[2] = province.bounds[2].max(x);
                province.bounds[3] = province.bounds[3].max(y);
            }
            check_size(
                device,
                "political province IDs",
                map.ids.len() as u64 * if packed { 2 } else { 4 },
                wgpu::BufferUsages::STORAGE,
            )?;
            let ids = if packed {
                map.ids
                    .chunks(2)
                    .map(|pair| pair[0] | (pair.get(1).copied().unwrap_or(0) << 16))
                    .collect()
            } else {
                map.ids.to_vec()
            };
            let ids = buffer(
                device,
                "political province IDs",
                bytemuck::cast_slice(&ids),
                wgpu::BufferUsages::STORAGE,
            )?;
            (Arc::from(provinces), ids)
        };
        let params = Params {
            width: map.width,
            height: map.height,
            columns: map.width.div_ceil(GRID_CELL),
            rows: map.height.div_ceil(GRID_CELL),
            stride: (map.width / 2 * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) / 4,
            packed_ids: u32::from(packed),
            count: 0,
            dispatch_stride: limits.max_compute_workgroups_per_dimension,
            opacity: u32::from(layer.region_opacity),
            no_color: rgb(layer.no_region_color),
            distance_cap: radius.max(1) + 1,
            distance_in_b: radius.max(1) % 2,
        };
        let cell_count = (params.columns * params.rows) as usize;
        let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let uniform = buffer(
            device,
            "political params",
            bytemuck::bytes_of(&params),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        )?;
        let mapping = vec![0; provinces.len()];
        let mapping_buffer = buffer(
            device,
            "political region of province",
            bytemuck::cast_slice(&mapping),
            rw,
        )?;
        let region_buffer = buffer(
            device,
            "political regions",
            bytemuck::bytes_of(&Region::default()),
            rw,
        )?;
        let alpha_buffer = zero_buffer(device, "political exact alpha", 4, rw)?;
        let cells = zero_buffer(
            device,
            "political boundary masks",
            cell_count as u64 * 36,
            rw | wgpu::BufferUsages::COPY_SRC,
        )?;
        let scratch = match scratch {
            Some(scratch) => scratch,
            None => Arc::new(Scratch::new(device, &params)?),
        };
        let dirty_buffer = zero_buffer(device, "political dirty cells", cell_count as u64 * 4, rw)?;
        let indirect = buffer(
            device,
            "political dispatch",
            &[0; 12],
            wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
        )?;
        let kernel = shared.map_or_else(
            || Arc::new(Kernel::new(device)),
            |gpu| Arc::clone(&gpu.kernel),
        );
        let group = bind(
            device,
            &kernel.layout,
            [
                &uniform,
                &ids,
                &mapping_buffer,
                &region_buffer,
                &alpha_buffer,
                &cells,
                &scratch.distances,
                &scratch.output,
                &dirty_buffer,
            ],
        );
        Ok(Self {
            params,
            uniform,
            ids,
            provinces,
            source_keys: vec![0; mapping.len()],
            member_at: vec![0; mapping.len()],
            members: Vec::new(),
            mapping,
            mapping_buffer,
            keys: HashMap::from([(0, 0)]),
            free: Vec::new(),
            areas: vec![0],
            thickness: vec![(u32::MAX, u32::MAX)],
            regions: vec![Region::default()],
            region_buffer,
            alpha: Vec::new(),
            alpha_buffer,
            alpha_stride,
            capacity: 1,
            alpha_keys: HashMap::new(),
            alpha_users: HashMap::new(),
            alpha_free: Vec::new(),
            alpha_slots: 0,
            cells,
            scratch,
            dirty_buffer,
            indirect,
            dirty: vec![false; cell_count],
            marked: Vec::new(),
            copies: Vec::new(),
            group,
            kernel,
            field,
            kind: layer.kind,
            initialized: false,
            pending: false,
            recorded: false,
            dispatched_cells: 0,
        })
    }

    pub fn update_mode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mode: &ModeLayer,
    ) -> io::Result<()> {
        let field = self.field;
        let color = |key| {
            mode.styles.get(&key).map_or([0, 0, 0], |style| {
                [style.color[0], style.color[1], style.color[2]]
            })
        };
        let style = |key| {
            let style = mode.styles.get(&key);
            gradient::RegionStyle {
                alpha: style.map_or(0, |style| style.color[3]),
                pulse: style.is_some_and(|style| style.pulse),
                fill: style.map_or(1.0, |style| style.fill),
                thickness: mode.thickness,
            }
        };
        let attributes = |_| map_display_data::landscape::Attributes {
            gradient: mode.gradient,
            ..Default::default()
        };
        let mut layer = Layer::new(
            gradient::LayerKind::Country,
            &mode.region_of,
            &color,
            &field,
        );
        layer.region_style = Some(&style);
        layer.region_attributes = Some(&attributes);
        self.update(device, queue, &layer)
    }

    pub fn refill(&mut self) {
        self.initialized = false;
    }

    pub fn update_regions(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        regions: &Regions,
    ) -> io::Result<()> {
        let field = self.field;
        let color = |key| regions.country_color(key);
        let layer = Layer::new(
            gradient::LayerKind::Country,
            &regions.country_of,
            &color,
            &field,
        );
        self.update(device, queue, &layer)
    }

    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layer: &Layer<'_>,
    ) -> io::Result<()> {
        if self.field != *layer.field
            || self.kind != layer.kind
            || self.params.opacity != u32::from(layer.region_opacity)
            || self.params.no_color != rgb(layer.no_region_color)
        {
            return Err(unsupported(
                "layer operands changed without rebuilding map resources",
            ));
        }
        if layer.region_of.len() < self.mapping.len() {
            return Err(unsupported("region table no longer covers the map"));
        }
        let region_of = &layer.region_of[..self.mapping.len()];
        let same_membership = self.source_keys == region_of;
        if self.initialized
            && !self.pending
            && same_membership
            && self.keys.iter().all(|(&key, &slot)| {
                key == 0
                    || (self.regions[slot].color == rgb((layer.region_color)(key))
                        && self.regions[slot].attributes
                            == layer.region_attributes.map_or(0, |read| read(key).packed())
                        && (self.regions[slot].fx, self.regions[slot].fill)
                            == style_words(layer.style(key))
                        && self.thickness[slot]
                            == (layer.thickness(key, self.areas[slot]).to_bits(), layer.center(key).to_bits()))
            })
        {
            self.dispatched_cells = 0;
            return Ok(());
        }
        let limits = device.limits();
        let storage = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let area_of = {
            let _span = span("political_area");
            membership::areas(
                region_of,
                &self.provinces,
                same_membership.then_some((&self.keys, &self.areas)),
            )
        };
        let released = self
            .keys
            .keys()
            .filter(|&&key| key != 0 && !area_of.contains_key(&key))
            .count();
        let added = area_of
            .keys()
            .filter(|key| !self.keys.contains_key(key))
            .count();
        let region_slots = self.regions.len() + added.saturating_sub(self.free.len() + released);
        if region_slots as u64 * REGION_BYTES > storage {
            return Err(unsupported(format!(
                "{region_slots} political regions exceed the device storage limit {storage}"
            )));
        }
        let active_thickness: HashSet<(u32, u32)> = area_of
            .iter()
            .map(|(&key, &area)| (layer.thickness(key, area).to_bits(), layer.center(key).to_bits()))
            .collect();
        let reusable = self.alpha_free.len()
            + self
                .alpha_keys
                .keys()
                .filter(|bits| !active_thickness.contains(bits))
                .count();
        let new_lookups = active_thickness
            .iter()
            .filter(|bits| !self.alpha_keys.contains_key(bits))
            .count();
        let alpha_slots = self.alpha_slots + new_lookups.saturating_sub(reusable);
        if alpha_slots as u64 * self.alpha_stride as u64 > storage {
            return Err(unsupported(format!(
                "{alpha_slots} exact alpha lookups of {} bytes exceed the device storage limit {storage}",
                self.alpha_stride
            )));
        }

        self.dirty.fill(!self.initialized || self.pending);
        self.marked.clear();
        let mut changed = Vec::new();
        {
            let _span = span("political_mapping");
            if !same_membership {
                self.keys.retain(|key, slot| {
                    if *key == 0 || area_of.contains_key(key) {
                        true
                    } else {
                        self.free.push(*slot);
                        false
                    }
                });
                for (province, &key) in region_of.iter().enumerate() {
                    let slot = if let Some(&slot) = self.keys.get(&key) {
                        slot
                    } else {
                        let slot = self.free.pop().unwrap_or_else(|| {
                            self.regions.push(Region::default());
                            self.areas.push(0);
                            self.thickness.push((u32::MAX, u32::MAX));
                            self.regions.len() - 1
                        });
                        self.keys.insert(key, slot);
                        slot
                    };
                    let old = self.mapping[province] as usize;
                    if old != slot {
                        let pixels = self.provinces[province].pixels;
                        if old != 0 {
                            self.areas[old] -= pixels;
                        }
                        if slot != 0 {
                            self.areas[slot] += pixels;
                        }
                        self.remap(province, slot);
                        changed.push(province);
                    }
                }
            }
        }
        let mut affected = vec![false; self.regions.len()];
        self.alpha_keys.retain(|bits, slot| {
            if active_thickness.contains(bits) {
                true
            } else {
                self.alpha_free.push(*slot);
                false
            }
        });
        let mut alpha_changes = Vec::new();
        for bits in active_thickness {
            if self.alpha_keys.contains_key(&bits) {
                continue;
            }
            let slot = self.alpha_free.pop().unwrap_or_else(|| {
                self.alpha_slots += 1;
                self.alpha.resize(self.alpha_slots * self.alpha_stride, 0);
                self.alpha_slots - 1
            });
            let t = f32::from_bits(bits.0);
            let field = FieldDefines { center: f32::from_bits(bits.1), ..*layer.field };
            let reach = gradient::field_reach(t, &field);
            for (d2, a) in self.alpha[slot * self.alpha_stride..(slot + 1) * self.alpha_stride]
                .iter_mut()
                .enumerate()
            {
                let d = (d2 as f32).sqrt();
                *a = if d <= reach {
                    gradient::field_alpha(d, t, field.center)
                } else {
                    0
                };
            }
            self.alpha_keys.insert(bits, slot);
            alpha_changes.push(slot);
        }
        let mut region_changes = Vec::new();
        for (&key, &slot) in &self.keys {
            if slot == 0 {
                continue;
            }
            let t = layer.thickness(key, self.areas[slot]);
            let field = FieldDefines { center: layer.center(key), ..*layer.field };
            let radius = search_radius(t, &field, self.params.width, self.params.height);
            let shape = (t.to_bits(), field.center.to_bits());
            let alpha_slot = self.alpha_keys[&shape];
            let (fx, fill) = style_words(layer.style(key));
            let region = Region {
                color: rgb((layer.region_color)(key)),
                alpha_offset: (alpha_slot * self.alpha_stride / 4) as u32,
                alpha_count: self.alpha_stride as u32,
                radius,
                attributes: layer.region_attributes.map_or(0, |read| read(key).packed()),
                fx,
                fill,
            };
            if self.regions[slot] != region {
                affected[slot] = true;
                self.regions[slot] = region;
                region_changes.push(slot);
            }
            if self.thickness[slot] != shape {
                affected[slot] = true;
                self.thickness[slot] = shape;
            }
        }
        self.alpha_users.clear();
        for &slot in self.keys.values().filter(|&&slot| slot != 0) {
            *self.alpha_users.entry(self.thickness[slot]).or_default() += 1;
        }
        let halo = self.halo();
        for &province in &changed {
            self.mark(province, halo);
        }
        for province in 0..self.mapping.len() {
            if affected[self.mapping[province] as usize] {
                self.mark(province, 1);
            }
        }
        let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let mut rebind = false;
        if self.regions.len() > self.capacity {
            self.capacity = self
                .regions
                .len()
                .max((self.capacity * 2).min((storage / REGION_BYTES) as usize));
            self.region_buffer = zero_buffer(
                device,
                "political regions",
                self.capacity as u64 * REGION_BYTES,
                rw,
            )?;
            shader_pipeline::gpu::stats::write_buffer(&queue, &self.region_buffer, 0, bytemuck::cast_slice(&self.regions));
            rebind = true;
        } else {
            for slot in region_changes {
                shader_pipeline::gpu::stats::write_buffer(&queue, 
                    &self.region_buffer,
                    slot as u64 * REGION_BYTES,
                    bytemuck::bytes_of(&self.regions[slot]),
                );
            }
        }
        if self.alpha.len() as u64 > self.alpha_buffer.size() {
            let stride = self.alpha_stride as u64;
            let size = (self.alpha.len() as u64)
                .max((self.alpha_buffer.size() * 2).min(storage / stride * stride));
            self.alpha_buffer = zero_buffer(device, "political exact alpha", size, rw)?;
            shader_pipeline::gpu::stats::write_buffer(&queue, &self.alpha_buffer, 0, &self.alpha);
            rebind = true;
        } else {
            for slot in alpha_changes {
                shader_pipeline::gpu::stats::write_buffer(&queue, 
                    &self.alpha_buffer,
                    (slot * self.alpha_stride) as u64,
                    &self.alpha[slot * self.alpha_stride..(slot + 1) * self.alpha_stride],
                );
            }
        }
        if rebind {
            self.group = bind(
                device,
                &self.kernel.layout,
                [
                    &self.uniform,
                    &self.ids,
                    &self.mapping_buffer,
                    &self.region_buffer,
                    &self.alpha_buffer,
                    &self.cells,
                    &self.scratch.distances,
                    &self.scratch.output,
                    &self.dirty_buffer,
                ],
            );
        }
        for range in runs(&changed) {
            shader_pipeline::gpu::stats::write_buffer(&queue, 
                &self.mapping_buffer,
                (range.start * 4) as u64,
                bytemuck::cast_slice(&self.mapping[range]),
            );
        }
        let dirty = self
            .dirty
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| d.then_some(i as u32))
            .collect::<Vec<_>>();
        self.dispatched_cells = dirty.len();
        self.copies = copy_rects(
            &dirty,
            self.params.columns,
            self.params.width / 2,
            self.params.height / 2,
        );
        self.params.count = dirty.len() as u32;
        self.pending = !dirty.is_empty();
        if self.pending {
            shader_pipeline::gpu::stats::write_buffer(&queue, &self.dirty_buffer, 0, bytemuck::cast_slice(&dirty));
            shader_pipeline::gpu::stats::write_buffer(&queue, &self.uniform, 0, bytemuck::bytes_of(&self.params));
            let x = self.params.count.min(self.params.dispatch_stride);
            shader_pipeline::gpu::stats::write_buffer(&queue, 
                &self.indirect,
                0,
                bytemuck::cast_slice(&[x, self.params.count.div_ceil(x), 1]),
            );
        }
        if !same_membership {
            self.source_keys.copy_from_slice(region_of);
        }
        self.marked = dirty;
        Ok(())
    }

    fn halo(&self) -> u32 {
        (self.params.distance_cap + 1).max(
            ((self.field.optimization_range.max(0.0) / GRID_CELL as f32).ceil() as u32)
                .min(self.params.columns.max(self.params.rows))
                + 1,
        )
    }

    pub fn update_changed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layer: &Layer<'_>,
        provinces: &[usize],
        keys: &[u32],
    ) -> io::Result<()> {
        if !self.initialized
            || self.field != *layer.field
            || self.kind != layer.kind
            || layer.region_of.len() < self.mapping.len()
        {
            return self.update(device, queue, layer);
        }
        if !self.pending {
            for &cell in &self.marked {
                self.dirty[cell as usize] = false;
            }
            self.marked.clear();
        }
        let mut touched = keys.to_vec();
        let mut changed = Vec::new();
        for &province in provinces {
            if province >= self.mapping.len() {
                continue;
            }
            let key = layer.region_of[province];
            let held = self.source_keys[province];
            if key == held {
                continue;
            }
            let slot = match self.keys.get(&key) {
                Some(&slot) => slot,
                None => {
                    let slot = match self.free.pop() {
                        Some(slot) => {
                            self.thickness[slot] = (u32::MAX, u32::MAX);
                            slot
                        }
                        None if self.regions.len() < self.capacity => {
                            self.regions.push(Region::default());
                            self.areas.push(0);
                            self.thickness.push((u32::MAX, u32::MAX));
                            self.regions.len() - 1
                        }
                        None => return self.refill_update(device, queue, layer),
                    };
                    self.keys.insert(key, slot);
                    slot
                }
            };
            let old = self.mapping[province] as usize;
            if old != slot {
                let pixels = self.provinces[province].pixels;
                if old != 0 {
                    self.areas[old] -= pixels;
                }
                if slot != 0 {
                    self.areas[slot] += pixels;
                }
                self.remap(province, slot);
                changed.push(province);
            }
            self.source_keys[province] = key;
            touched.extend([held, key]);
        }
        touched.sort_unstable();
        touched.dedup();
        let (mut affected, mut region_changes, mut alpha_changes) = (Vec::new(), Vec::new(), Vec::new());
        for key in touched {
            let Some(&slot) = self.keys.get(&key).filter(|_| key != 0) else { continue };
            let t = layer.thickness(key, self.areas[slot]);
            let field = FieldDefines { center: layer.center(key), ..*layer.field };
            let shape = (t.to_bits(), field.center.to_bits());
            let alpha_slot = match self.alpha_keys.get(&shape) {
                Some(&slot) => slot,
                None => {
                    let slot = match self.alpha_free.pop() {
                        Some(slot) => slot,
                        None if ((self.alpha_slots + 1) * self.alpha_stride) as u64 <= self.alpha_buffer.size() => {
                            self.alpha_slots += 1;
                            self.alpha.resize(self.alpha_slots * self.alpha_stride, 0);
                            self.alpha_slots - 1
                        }
                        None => return self.refill_update(device, queue, layer),
                    };
                    let reach = gradient::field_reach(t, &field);
                    for (d2, a) in self.alpha[slot * self.alpha_stride..(slot + 1) * self.alpha_stride].iter_mut().enumerate() {
                        let d = (d2 as f32).sqrt();
                        *a = if d <= reach { gradient::field_alpha(d, t, field.center) } else { 0 };
                    }
                    self.alpha_keys.insert(shape, slot);
                    alpha_changes.push(slot);
                    slot
                }
            };
            let (fx, fill) = style_words(layer.style(key));
            let region = Region {
                color: rgb((layer.region_color)(key)),
                alpha_offset: (alpha_slot * self.alpha_stride / 4) as u32,
                alpha_count: self.alpha_stride as u32,
                radius: search_radius(t, &field, self.params.width, self.params.height),
                attributes: layer.region_attributes.map_or(0, |read| read(key).packed()),
                fx,
                fill,
            };
            let old = self.thickness[slot];
            if old != shape {
                *self.alpha_users.entry(shape).or_default() += 1;
                if let Some(users) = self.alpha_users.get_mut(&old) {
                    *users -= 1;
                    if *users == 0 {
                        self.alpha_users.remove(&old);
                        if let Some(free) = self.alpha_keys.remove(&old) {
                            self.alpha_free.push(free);
                        }
                    }
                }
            }
            if self.regions[slot] != region || old != shape {
                self.regions[slot] = region;
                self.thickness[slot] = shape;
                affected.push(slot);
                region_changes.push(slot);
            }
        }
        let halo = self.halo();
        for &province in &changed {
            self.mark(province, halo);
        }
        for slot in affected {
            for at in 0..self.members.get(slot).map_or(0, Vec::len) {
                let province = self.members[slot][at] as usize;
                self.mark(province, 1);
            }
        }
        for slot in region_changes {
            shader_pipeline::gpu::stats::write_buffer(queue, &self.region_buffer, slot as u64 * REGION_BYTES, bytemuck::bytes_of(&self.regions[slot]));
        }
        for slot in alpha_changes {
            shader_pipeline::gpu::stats::write_buffer(
                queue,
                &self.alpha_buffer,
                (slot * self.alpha_stride) as u64,
                &self.alpha[slot * self.alpha_stride..(slot + 1) * self.alpha_stride],
            );
        }
        changed.sort_unstable();
        for range in runs(&changed) {
            shader_pipeline::gpu::stats::write_buffer(queue, &self.mapping_buffer, (range.start * 4) as u64, bytemuck::cast_slice(&self.mapping[range]));
        }
        let _ = device;
        let mut dirty = self.marked.clone();
        dirty.sort_unstable();
        self.dispatched_cells = dirty.len();
        self.copies = copy_rects(&dirty, self.params.columns, self.params.width / 2, self.params.height / 2);
        self.params.count = dirty.len() as u32;
        self.pending = !dirty.is_empty();
        if self.pending {
            shader_pipeline::gpu::stats::write_buffer(queue, &self.dirty_buffer, 0, bytemuck::cast_slice(&dirty));
            shader_pipeline::gpu::stats::write_buffer(queue, &self.uniform, 0, bytemuck::bytes_of(&self.params));
            let x = self.params.count.min(self.params.dispatch_stride);
            shader_pipeline::gpu::stats::write_buffer(queue, &self.indirect, 0, bytemuck::cast_slice(&[x, self.params.count.div_ceil(x), 1]));
        }
        Ok(())
    }

    fn refill_update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, layer: &Layer<'_>) -> io::Result<()> {
        shader_pipeline::gpu::stats::write_buffer(queue, &self.mapping_buffer, 0, bytemuck::cast_slice(&self.mapping));
        shader_pipeline::gpu::stats::write_buffer(queue, &self.region_buffer, 0, bytemuck::cast_slice(&self.regions));
        shader_pipeline::gpu::stats::write_buffer(queue, &self.alpha_buffer, 0, &self.alpha);
        self.refill();
        self.update(device, queue, layer)
    }

    #[cfg(test)]
    fn own_bytes(&self) -> u64 {
        [
            &self.uniform,
            &self.mapping_buffer,
            &self.region_buffer,
            &self.alpha_buffer,
            &self.cells,
            &self.dirty_buffer,
            &self.indirect,
        ]
        .iter()
        .map(|buffer| buffer.size())
        .sum()
    }

    fn remap(&mut self, province: usize, slot: usize) {
        let old = self.mapping[province] as usize;
        if old != 0 {
            let at = self.member_at[province] as usize;
            let list = &mut self.members[old];
            list.swap_remove(at);
            if let Some(&moved) = list.get(at) {
                self.member_at[moved as usize] = at as u32;
            }
        }
        if slot != 0 {
            if self.members.len() <= slot {
                self.members.resize_with(slot + 1, Vec::new);
            }
            self.member_at[province] = self.members[slot].len() as u32;
            self.members[slot].push(province as u32);
        }
        self.mapping[province] = slot as u32;
    }

    fn mark(&mut self, province: usize, halo: u32) {
        let Province {
            pixels,
            bounds: [x0, y0, x1, y1],
        } = self.provinces[province];
        if pixels == 0 {
            return;
        }
        let columns = self.params.columns;
        let span = (x1 - x0 + 1 + 2 * halo).min(columns);
        for y in y0.saturating_sub(halo)..=(y1 + halo).min(self.params.rows - 1) {
            for dx in 0..span {
                let x = (i64::from(x0) - i64::from(halo) + i64::from(dx))
                    .rem_euclid(i64::from(columns)) as u32;
                let cell = (y * columns + x) as usize;
                if !self.dirty[cell] {
                    self.dirty[cell] = true;
                    self.marked.push(cell as u32);
                }
            }
        }
    }

    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        textures: [&wgpu::Texture; 2],
        origin_y: u32,
    ) -> bool {
        if !self.dispatch(encoder) {
            return false;
        }
        self.copy(encoder, textures, origin_y, &self.copies);
        true
    }

    fn encode_rows(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        textures: [&wgpu::Texture; 2],
        rows: &[u32],
    ) -> bool {
        if !self.dispatch(encoder) {
            return false;
        }
        for &row in rows {
            self.copy(encoder, textures, row, &self.copies);
        }
        true
    }

    fn dispatch(&mut self, encoder: &mut wgpu::CommandEncoder) -> bool {
        if !self.pending || self.recorded {
            return false;
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("political fill"),
                timestamp_writes: None,
            });
            shader_pipeline::gpu::stats::bind();
            pass.set_bind_group(0, &self.group, &[]);
            let mut dispatch = |pipeline: usize, indirect: bool| {
                shader_pipeline::gpu::stats::pipeline();
                pass.set_pipeline(&self.kernel.pipelines[pipeline]);
                if indirect {
                    pass.dispatch_workgroups_indirect(&self.indirect, 0);
                } else {
                    pass.dispatch_workgroups(
                        self.params.columns.div_ceil(8),
                        self.params.rows.div_ceil(8),
                        1,
                    );
                }
            };
            dispatch(0, true);
            dispatch(1, false);
            for step in 0..self.params.distance_cap - 1 {
                dispatch(if step % 2 == 0 { 2 } else { 3 }, false);
            }
            dispatch(4, true);
        }
        self.recorded = true;
        true
    }

    pub fn recorded(&mut self, submitted: bool) {
        if self.recorded && submitted {
            self.pending = false;
            self.initialized = true;
        }
        self.recorded = false;
    }

    fn copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        textures: [&wgpu::Texture; 2],
        origin_y: u32,
        rectangles: &[[u32; 4]],
    ) {
        for (channel, texture) in textures.into_iter().enumerate() {
            for &[x, y, width, height] in rectangles {
                encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer: &self.scratch.output,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: channel as u64
                                * u64::from(self.params.stride)
                                * u64::from(self.params.height / 2)
                                * 4
                                + u64::from(y * self.params.stride + x) * 4,
                            bytes_per_row: Some(self.params.stride * 4),
                            rows_per_image: None,
                        },
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x,
                            y: origin_y + y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }
}

fn copy_rects(cells: &[u32], columns: u32, width: u32, height: u32) -> Vec<[u32; 4]> {
    let mut rectangles: Vec<[u32; 4]> = Vec::new();
    let mut open: HashMap<(u32, u32), usize> = HashMap::new();
    let mut previous = None;
    let mut rest = cells;
    while let Some(&first) = rest.first() {
        let row = first / columns;
        let count = rest.partition_point(|cell| cell / columns == row);
        let y = row * 8;
        let h = (height - y).min(8);
        let adjacent = previous.is_some_and(|previous| previous + 1 == row);
        let mut next = HashMap::new();
        let mut run = &rest[..count];
        while let Some(&start) = run.first() {
            let length = run
                .iter()
                .enumerate()
                .take_while(|&(i, &cell)| cell == start + i as u32)
                .count();
            let x = start % columns * 8;
            let w = ((run[length - 1] % columns + 1) * 8).min(width) - x;
            let index = match open.get(&(x, w)).filter(|_| adjacent) {
                Some(&index) => {
                    rectangles[index][3] += h;
                    index
                }
                None => {
                    rectangles.push([x, y, w, h]);
                    rectangles.len() - 1
                }
            };
            next.insert((x, w), index);
            run = &run[length..];
        }
        open = next;
        previous = Some(row);
        rest = &rest[count..];
    }
    rectangles
}

fn maximum_thickness(kind: gradient::LayerKind, field: &FieldDefines) -> f32 {
    match kind {
        gradient::LayerKind::Country => {
            let [thin, thick] = field.country_thickness;
            thin.max(thin + (thick - thin))
                .max(field.mode_thickness_max)
        }
        gradient::LayerKind::Fixed(t) => t,
    }
}

fn search_radius(thickness: f32, field: &FieldDefines, width: u32, height: u32) -> u32 {
    let radius =
        ((thickness + field.center).sqrt() * gradient::RADIUS_MULTIPLIER / GRID_CELL as f32) as u32;
    radius
        .saturating_add(1)
        .min(width.div_ceil(GRID_CELL).max(height.div_ceil(GRID_CELL)))
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: [&wgpu::Buffer; 9],
) -> wgpu::BindGroup {
    shader_pipeline::gpu::stats::create_bind_group(&device, &wgpu::BindGroupDescriptor {
        label: Some("political compute"),
        layout,
        entries: &buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect::<Vec<_>>(),
    })
}

fn runs(indices: &[usize]) -> Vec<std::ops::Range<usize>> {
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for &i in indices {
        match ranges.last_mut() {
            Some(last) if last.end == i => last.end += 1,
            _ => ranges.push(i..i + 1),
        }
    }
    ranges
}

#[cfg(test)]
#[path = "political_gpu_tests.rs"]
pub(crate) mod tests;
