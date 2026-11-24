use super::{InstanceRing, PacketUpload, gpu};
use eframe::wgpu;
use std::{ops::Range, sync::Arc};

struct CopySpan {
    source: Range<usize>,
    target: usize,
}

struct Copies {
    source_stride: usize,
    stride: usize,
    spans: Vec<CopySpan>,
}

impl Copies {
    fn new(layout: &gpu::VertexLayout, prefix: u64) -> Option<(gpu::VertexLayout, Self)> {
        let mut packed = layout.clone();
        let mut stride = 0_u64;
        let mut spans: Vec<CopySpan> = Vec::new();
        for (input, output) in layout.attributes.iter().zip(&mut packed.attributes) {
            let end = input.offset.checked_add(input.format.size())?;
            if end > prefix {
                return None;
            }
            output.offset = stride;
            let source = usize::try_from(input.offset).ok()?..usize::try_from(end).ok()?;
            let target = usize::try_from(stride).ok()?;
            if let Some(last) = spans.last_mut().filter(|last| {
                last.source.end == source.start && last.target + last.source.len() == target
            }) {
                last.source.end = source.end;
            } else {
                spans.push(CopySpan { source, target });
            }
            stride = stride.checked_add(input.format.size())?.checked_add(3)? & !3;
        }
        packed.stride = stride;
        Some((
            packed,
            Self {
                source_stride: usize::try_from(layout.stride).ok()?,
                stride: usize::try_from(stride).ok()?,
                spans,
            },
        ))
    }

    fn pack(&self, source: &[u8], count: usize, bytes: &mut Vec<u8>) -> Option<()> {
        let size = self.stride.checked_mul(count)?;
        bytes.clear();
        bytes.try_reserve_exact(size).ok()?;
        bytes.resize(size, 0);
        if self.stride == 0 {
            return Some(());
        }
        if source.len() < self.source_stride.checked_mul(count)? {
            return None;
        }
        for (input, output) in source
            .chunks_exact(self.source_stride)
            .take(count)
            .zip(bytes.chunks_exact_mut(self.stride))
        {
            for span in &self.spans {
                output[span.target..span.target + span.source.len()]
                    .copy_from_slice(&input[span.source.clone()]);
            }
        }
        Some(())
    }
}

pub(super) struct Plan {
    native: [gpu::VertexLayout; 2],
    layouts: [gpu::VertexLayout; 2],
    quad: Copies,
    instances: Copies,
}

impl Plan {
    pub(super) fn new(layouts: &[gpu::VertexLayout]) -> Option<Arc<Self>> {
        if layouts.len() != 2
            || layouts[0].stride != 8
            || layouts[0].step != wgpu::VertexStepMode::Vertex
            || layouts[1].stride != 48
            || layouts[1].step != wgpu::VertexStepMode::Instance
        {
            return None;
        }
        let (quad_layout, quad) = Copies::new(&layouts[0], 8)?;
        let (instance_layout, instances) = Copies::new(&layouts[1], 36)?;
        Some(Arc::new(Self {
            native: [layouts[0].clone(), layouts[1].clone()],
            layouts: [quad_layout, instance_layout],
            quad,
            instances,
        }))
    }
}

struct Quad {
    plan: Arc<Plan>,
    vertices: gpu::Slice,
    #[cfg(test)]
    bytes: Vec<u8>,
}

struct Actor {
    plan: Arc<Plan>,
    vertices: [gpu::Slice; 2],
    #[cfg(test)]
    bytes: [Vec<u8>; 2],
}

pub(super) enum Attempt<'a> {
    Unproved,
    Handled(Option<PacketUpload<'a>>),
}

#[derive(Default)]
pub(super) struct Uploads {
    quads: Vec<Quad>,
    actors: Vec<Actor>,
    bytes: Vec<u8>,
    start: u64,
    end: u64,
}

impl Uploads {
    pub(super) fn clear_view(&mut self) {
        self.actors.clear();
        self.quads.clear();
        self.bytes.clear();
        self.start = 0;
        self.end = 0;
    }

    pub(super) fn begin(&mut self, start: u64, end: u64) {
        self.actors.clear();
        self.start = start;
        self.end = end;
    }

    pub(super) fn packet<'a>(
        &'a mut self,
        arena: &gpu::Arena,
        plan: &Arc<Plan>,
        ring: &InstanceRing,
        quad: &[u8],
        offset: u64,
        count: u32,
    ) -> Attempt<'a> {
        let Some(actor_count) = self
            .end
            .checked_sub(self.start)
            .filter(|n| n.is_multiple_of(48))
            .and_then(|n| u32::try_from(n / 48).ok())
        else {
            return Attempt::Unproved;
        };
        let Some(relative) = offset
            .checked_sub(self.start)
            .filter(|n| n.is_multiple_of(48))
        else {
            return Attempt::Unproved;
        };
        if offset
            .checked_add(u64::from(count) * 48)
            .is_none_or(|end| end > self.end)
        {
            return Attempt::Unproved;
        }
        let at = if let Some(at) = self
            .actors
            .iter()
            .position(|entry| Arc::ptr_eq(&entry.plan, plan) || entry.plan.native == plan.native)
        {
            at
        } else {
            let allocation = ring.allocation.lock().unwrap_or_else(|e| e.into_inner());
            let Some(start) = usize::try_from(self.start).ok() else {
                return Attempt::Unproved;
            };
            let Some(end) = usize::try_from(self.end).ok() else {
                return Attempt::Unproved;
            };
            let Some(known_at) = allocation.known.len().checked_sub(actor_count as usize) else {
                return Attempt::Unproved;
            };
            if ring.cursor != self.end
                || self.end > allocation.size
                || end != allocation.bytes.len()
                || quad.len() != 4 * 8
                || !allocation.known[known_at..]
                    .iter()
                    .enumerate()
                    .all(|(i, range)| {
                        let from = self.start + i as u64 * 48;
                        *range == (from..from + 36)
                    })
            {
                return Attempt::Unproved;
            }
            let q = if let Some(at) = self.quads.iter().position(|entry| {
                Arc::ptr_eq(&entry.plan, plan) || entry.plan.native[0] == plan.native[0]
            }) {
                at
            } else {
                if plan.quad.pack(quad, 4, &mut self.bytes).is_none() {
                    return Attempt::Handled(None);
                }
                let Ok(vertices) = arena.vertices(&self.bytes) else {
                    return Attempt::Handled(None);
                };
                self.quads.push(Quad {
                    plan: Arc::clone(plan),
                    vertices,
                    #[cfg(test)]
                    bytes: self.bytes.clone(),
                });
                self.quads.len() - 1
            };
            if plan
                .instances
                .pack(
                    &allocation.bytes[start..end],
                    actor_count as usize,
                    &mut self.bytes,
                )
                .is_none()
            {
                return Attempt::Handled(None);
            }
            let Ok(vertices) = arena.vertices(&self.bytes) else {
                return Attempt::Handled(None);
            };
            let quad = &self.quads[q];
            self.actors.push(Actor {
                plan: Arc::clone(plan),
                vertices: [quad.vertices.clone(), vertices],
                #[cfg(test)]
                bytes: [quad.bytes.clone(), self.bytes.clone()],
            });
            self.actors.len() - 1
        };
        let entry = &self.actors[at];
        let stride = entry.plan.layouts[1].stride;
        let Some(byte_offset) = (relative / 48).checked_mul(stride) else {
            return Attempt::Handled(None);
        };
        let Some(byte_length) = u64::from(count).checked_mul(stride) else {
            return Attempt::Handled(None);
        };
        Attempt::Handled(Some(PacketUpload {
            layouts: &entry.plan.layouts,
            vertices: [
                entry.vertices[0].clone(),
                gpu::Slice {
                    buffer: entry.vertices[1].buffer.clone(),
                    offset: entry.vertices[1].offset + byte_offset,
                    size: byte_length.max(4),
                },
            ],
            #[cfg(test)]
            bytes: [
                entry.bytes[0].clone(),
                entry.bytes[1][byte_offset as usize..(byte_offset + byte_length) as usize].to_vec(),
            ],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_pipeline::vertices::{NativeBuffer, pack_native};
    use std::collections::BTreeMap;

    #[test]
    fn weather_copy_plans_match_native_formats_order_subsets_duplicates_and_padding() {
        let quad = bytemuck::cast_slice(&viewer_scene_base::particle_shader::QUAD);
        let attrs = [
            (wgpu::VertexFormat::Float32x4, 0),
            (wgpu::VertexFormat::Float32x3, 16),
            (wgpu::VertexFormat::Uint8x4, 28),
            (wgpu::VertexFormat::Unorm8x4, 32),
            (wgpu::VertexFormat::Uint8x2, 29),
        ];
        let mut scratch = vec![0xff; 2048];
        for rotation in 0..attrs.len() {
            for subset in 0..(1 << attrs.len()) {
                let mut attributes = Vec::new();
                for index in 0..attrs.len() {
                    if subset & (1 << index) != 0 {
                        let (format, offset) = attrs[(index + rotation) % attrs.len()];
                        attributes.push(wgpu::VertexAttribute {
                            format,
                            offset,
                            shader_location: index as u32,
                        });
                    }
                }
                if subset % 3 == 1 && !attributes.is_empty() {
                    let mut duplicate = attributes[0];
                    duplicate.shader_location = 17;
                    attributes.push(duplicate);
                }
                let layouts = [
                    gpu::VertexLayout {
                        stride: 8,
                        step: wgpu::VertexStepMode::Vertex,
                        attributes: vec![wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 20,
                        }],
                    },
                    gpu::VertexLayout {
                        stride: 48,
                        step: wgpu::VertexStepMode::Instance,
                        attributes,
                    },
                ];
                let plan = Plan::new(&layouts).unwrap();
                for count in [0, 1, 2, 11] {
                    let bytes: Vec<_> = (0..count * 48).map(|n| n as u8).collect();
                    let known: Vec<_> = (0..count)
                        .map(|n| (n * 48) as u64..(n * 48 + 36) as u64)
                        .collect();
                    let quad_known = 0..quad.len() as u64;
                    let buffers = BTreeMap::from([
                        (
                            0,
                            Some(NativeBuffer {
                                bytes: quad,
                                size: quad.len() as u64,
                                offset: 0,
                                known: std::slice::from_ref(&quad_known),
                            }),
                        ),
                        (
                            1,
                            Some(NativeBuffer {
                                bytes: &bytes,
                                size: bytes.len() as u64,
                                offset: 0,
                                known: &known,
                            }),
                        ),
                    ]);
                    let expected = pack_native(&layouts, &buffers, 4, count as u32).unwrap();
                    assert_eq!(plan.layouts.as_slice(), expected.layouts);
                    plan.quad.pack(quad, 4, &mut scratch).unwrap();
                    assert_eq!(scratch, expected.bytes[0]);
                    plan.instances.pack(&bytes, count, &mut scratch).unwrap();
                    assert_eq!(scratch, expected.bytes[1]);
                    scratch.fill(0xff);
                }
            }
        }
    }

    #[test]
    fn weather_copy_plans_leave_unknown_tail_and_nonordinary_declarations_to_native_fetches() {
        let layouts = vec![
            gpu::VertexLayout {
                stride: 8,
                step: wgpu::VertexStepMode::Vertex,
                attributes: vec![wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                }],
            },
            gpu::VertexLayout {
                stride: 48,
                step: wgpu::VertexStepMode::Instance,
                attributes: vec![wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 1,
                }],
            },
        ];
        assert!(Plan::new(&layouts).is_some());
        for slot in 0..2 {
            for offset in [if slot == 0 { 8 } else { 36 }, u64::MAX] {
                let mut changed = layouts.clone();
                changed[slot].attributes[0].offset = offset;
                assert!(Plan::new(&changed).is_none());
            }
        }
        let mut changed = layouts.clone();
        changed[1].stride = 44;
        assert!(Plan::new(&changed).is_none());
        changed = layouts.clone();
        changed[1].step = wgpu::VertexStepMode::Vertex;
        assert!(Plan::new(&changed).is_none());
        assert!(Plan::new(&layouts[..1]).is_none());
    }
}
