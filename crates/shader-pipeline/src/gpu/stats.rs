//! Counts of the GPU commands and uploads a frame records, read by the playback-cost harness.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Count {
    Draws,
    Pipelines,
    BindGroups,
    /// Vertex, index and blend-constant sets.
    States,
    Uploads,
    UploadBytes,
    CreatedBindGroups,
    CreatedBuffers,
    /// Shader programs compiled and render pipelines built, on any thread.
    Builds,
    /// Of those, the ones the window thread made.
    WindowBuilds,
    /// GUI batches whose first draw of an unforeseen pipeline waited for its build.
    GuiJoins,
    /// Microseconds those batches waited.
    GuiJoinMicros,
}

pub const NAMES: [&str; 12] = [
    "draws",
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
    "gui_join_us",
];

static COUNTS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
static BUILDS_TOTAL: AtomicU64 = AtomicU64::new(0);

pub fn builds_total() -> u64 {
    BUILDS_TOTAL.load(Relaxed)
}

std::thread_local! {
    static WINDOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks the calling thread as the one that draws the window's frames.
pub fn mark_window_thread() {
    WINDOW.with(|window| window.set(true));
}

/// Whether the calling thread draws the window's frames ([`mark_window_thread`]).
pub fn on_window_thread() -> bool {
    WINDOW.with(std::cell::Cell::get)
}

/// A shader compile or pipeline build, attributed to the window thread when it made it.
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

/// The counts since the last take.
pub fn take() -> [u64; 12] {
    std::array::from_fn(|index| COUNTS[index].swap(0, Relaxed))
}

pub fn write_buffer(queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
    upload(bytes.len());
    queue.write_buffer(buffer, offset, bytes);
}

pub fn write_texture(
    queue: &wgpu::Queue,
    texture: wgpu::TexelCopyTextureInfo<'_>,
    bytes: &[u8],
    layout: wgpu::TexelCopyBufferLayout,
    size: wgpu::Extent3d,
) {
    upload(bytes.len());
    queue.write_texture(texture, bytes, layout, size);
}

pub fn create_buffer_init(
    device: &wgpu::Device,
    descriptor: &wgpu::util::BufferInitDescriptor<'_>,
) -> wgpu::Buffer {
    use wgpu::util::DeviceExt as _;
    upload(descriptor.contents.len());
    created_buffer();
    device.create_buffer_init(descriptor)
}

pub fn create_bind_group(
    device: &wgpu::Device,
    descriptor: &wgpu::BindGroupDescriptor<'_>,
) -> wgpu::BindGroup {
    created_bind_group();
    device.create_bind_group(descriptor)
}
