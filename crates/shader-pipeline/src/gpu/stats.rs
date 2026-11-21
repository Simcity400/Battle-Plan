
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Count {
    Draws,
    Passes,
    Pipelines,
    BindGroups,
    States,
    Uploads,
    UploadBytes,
    CreatedBindGroups,
    CreatedBuffers,
    Builds,
    WindowBuilds,
    GuiJoins,
    GuiPrepared,
    Staged,
    StagedBytes,
    GuiHanded,
    GuiNative,
    WriteNs,
    CreateNs,
}

pub const NAMES: [&str; 19] = [
    "draws",
    "render_passes",
    "pipelines",
    "bind_groups",
    "states",
    "uploads",
    "upload_bytes",
    "created_bind_groups",
    "created_buffers",
    "builds",
    "window_builds",
    "gui_joins",
    "gui_prepared",
    "staged",
    "staged_bytes",
    "gui_handed",
    "gui_native",
    "write_ns",
    "create_ns",
];

static COUNTS: [AtomicU64; NAMES.len()] = [const { AtomicU64::new(0) }; NAMES.len()];
static BUILDS_TOTAL: AtomicU64 = AtomicU64::new(0);

pub fn builds_total() -> u64 {
    BUILDS_TOTAL.load(Relaxed)
}

std::thread_local! {
    static WINDOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn mark_window_thread() {
    WINDOW.with(|window| window.set(true));
}

pub fn on_window_thread() -> bool {
    WINDOW.with(std::cell::Cell::get)
}

#[inline]
pub fn build() {
    BUILDS_TOTAL.fetch_add(1, Relaxed);
    add(Count::Builds, 1);
    if WINDOW.with(std::cell::Cell::get) {
        add(Count::WindowBuilds, 1);
    }
}

#[inline]
pub fn add(count: Count, n: u64) {
    COUNTS[count as usize].fetch_add(n, Relaxed);
}

#[inline]
pub fn draw() {
    add(Count::Draws, 1);
}

#[inline]
pub fn pass() {
    add(Count::Passes, 1);
}

pub fn passes() -> u64 {
    COUNTS[Count::Passes as usize].load(Relaxed)
}

pub trait CountedPasses {
    fn counted_render_pass<'a>(
        &'a mut self,
        descriptor: &wgpu::RenderPassDescriptor<'_>,
    ) -> wgpu::RenderPass<'a>;
}

impl CountedPasses for wgpu::CommandEncoder {
    #[inline]
    fn counted_render_pass<'a>(
        &'a mut self,
        descriptor: &wgpu::RenderPassDescriptor<'_>,
    ) -> wgpu::RenderPass<'a> {
        pass();
        self.begin_render_pass(descriptor)
    }
}

pub fn draws() -> u64 {
    COUNTS[Count::Draws as usize].load(Relaxed)
}

#[inline]
pub fn pipeline() {
    add(Count::Pipelines, 1);
}

#[inline]
pub fn bind() {
    add(Count::BindGroups, 1);
}

#[inline]
pub fn state() {
    add(Count::States, 1);
}

#[inline]
pub fn upload(bytes: usize) {
    add(Count::Uploads, 1);
    add(Count::UploadBytes, bytes as u64);
}

#[inline]
pub fn created_bind_group() {
    add(Count::CreatedBindGroups, 1);
}

#[inline]
pub fn created_buffer() {
    add(Count::CreatedBuffers, 1);
}

#[inline]
pub fn staged(bytes: usize) {
    add(Count::Staged, 1);
    add(Count::StagedBytes, bytes as u64);
}

pub fn peek() -> [u64; NAMES.len()] {
    std::array::from_fn(|index| COUNTS[index].load(Relaxed))
}

pub fn take() -> [u64; NAMES.len()] {
    std::array::from_fn(|index| COUNTS[index].swap(0, Relaxed))
}

pub fn write_buffer(queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
    upload(bytes.len());
    let started = std::time::Instant::now();
    queue.write_buffer(buffer, offset, bytes);
    add(Count::WriteNs, started.elapsed().as_nanos() as u64);
}

pub fn belt_write(
    belt: &mut wgpu::util::StagingBelt,
    encoder: &mut wgpu::CommandEncoder,
    buffer: &wgpu::Buffer,
    offset: u64,
    bytes: &[u8],
) {
    let Some(size) = wgpu::BufferSize::new(bytes.len() as u64) else {
        return;
    };
    upload(bytes.len());
    let started = std::time::Instant::now();
    belt.write_buffer(encoder, buffer, offset, size).copy_from_slice(bytes);
    add(Count::WriteNs, started.elapsed().as_nanos() as u64);
}

pub fn create_buffer(device: &wgpu::Device, descriptor: &wgpu::BufferDescriptor<'_>) -> wgpu::Buffer {
    created_buffer();
    let started = std::time::Instant::now();
    let buffer = device.create_buffer(descriptor);
    add(Count::CreateNs, started.elapsed().as_nanos() as u64);
    buffer
}

pub fn write_texture(
    queue: &wgpu::Queue,
    texture: wgpu::TexelCopyTextureInfo<'_>,
    bytes: &[u8],
    layout: wgpu::TexelCopyBufferLayout,
    size: wgpu::Extent3d,
) {
    upload(bytes.len());
    let started = std::time::Instant::now();
    queue.write_texture(texture, bytes, layout, size);
    add(Count::WriteNs, started.elapsed().as_nanos() as u64);
}

pub fn create_buffer_init(
    device: &wgpu::Device,
    descriptor: &wgpu::util::BufferInitDescriptor<'_>,
) -> wgpu::Buffer {
    use wgpu::util::DeviceExt as _;
    upload(descriptor.contents.len());
    created_buffer();
    let started = std::time::Instant::now();
    let buffer = device.create_buffer_init(descriptor);
    add(Count::CreateNs, started.elapsed().as_nanos() as u64);
    buffer
}

pub fn create_bind_group(
    device: &wgpu::Device,
    descriptor: &wgpu::BindGroupDescriptor<'_>,
) -> wgpu::BindGroup {
    created_bind_group();
    let started = std::time::Instant::now();
    let group = device.create_bind_group(descriptor);
    add(Count::CreateNs, started.elapsed().as_nanos() as u64);
    group
}
