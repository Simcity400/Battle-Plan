//! Preserve native IA fetches when a retained declaration overlaps buffer strides.
use crate::gpu::VertexLayout;
use std::collections::BTreeMap;

/// A native allocation of `size` bytes whose written prefix is `bytes`. `offset` is the
/// bound IA byte offset. `known` lists the initialized byte ranges, sorted and disjoint,
/// within `bytes`; unknown bytes must not be supplied as invented zeros.
pub struct NativeBuffer<'a> {
    pub bytes: &'a [u8],
    pub size: u64,
    pub offset: u64,
    pub known: &'a [std::ops::Range<u64>],
}

pub struct PackedBuffers {
    pub layouts: Vec<VertexLayout>,
    pub bytes: Vec<Vec<u8>>,
}

/// Copy attributes without numeric conversion. The GPU still decodes each
/// original format, including UNORM, integer inputs and missing components.
/// D3D11.3 spec8.19.2 makes a fetch outside the bound allocation zero in the
/// format's present components. An absent map entry is unknown; `None` is null.
pub fn pack_native(
    layouts: &[VertexLayout],
    buffers: &BTreeMap<u32, Option<NativeBuffer<'_>>>,
    vertices: u32,
    instances: u32,
) -> Result<PackedBuffers, String> {
    let mut out = PackedBuffers {
        layouts: Vec::new(),
        bytes: Vec::new(),
    };
    for (slot, layout) in layouts.iter().enumerate() {
        let mut packed = layout.clone();
        let mut stride = 0_u64;
        for attribute in &mut packed.attributes {
            attribute.offset = stride;
            stride = (stride + attribute.format.size() + 3) & !3;
        }
        packed.stride = stride;
        let count = match layout.step {
            wgpu::VertexStepMode::Vertex => vertices,
            wgpu::VertexStepMode::Instance => instances,
        };
        let size = stride
            .checked_mul(u64::from(count))
            .and_then(|size| usize::try_from(size).ok())
            .ok_or("packed native vertex allocation overflow")?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|e| e.to_string())?;
        bytes.resize(size, 0);
        if !layout.attributes.is_empty() {
            let buffer = buffers
                .get(&(slot as u32))
                .ok_or_else(|| format!("native vertex buffer slot {slot} has not been supplied"))?;
            if let Some(buffer) = buffer {
                // Byte span of one record's fetches, to check each record's reads at once.
                let first = layout
                    .attributes
                    .iter()
                    .map(|a| a.offset)
                    .min()
                    .unwrap_or(0);
                let last = layout
                    .attributes
                    .iter()
                    .map(|a| a.offset + a.format.size())
                    .max()
                    .unwrap_or(0);
                for index in 0..u64::from(count) {
                    let base = index
                        .checked_mul(layout.stride)
                        .and_then(|value| value.checked_add(buffer.offset))
                        .ok_or("native vertex address overflow")?;
                    let span = base
                        .checked_add(last)
                        .filter(|end| *end <= buffer.size)
                        .is_some_and(|end| covered(buffer.known, base + first, end));
                    for (input, output) in layout.attributes.iter().zip(&packed.attributes) {
                        let from = base
                            .checked_add(input.offset)
                            .ok_or("native vertex address overflow")?;
                        let end = from
                            .checked_add(input.format.size())
                            .ok_or("native vertex address overflow")?;
                        if end > buffer.size {
                            continue;
                        }
                        if !span && !covered(buffer.known, from, end) {
                            return Err(format!(
                                "native vertex buffer slot {slot}, record {index}, location {} reads unavailable bytes {from}..{end}",
                                input.shader_location
                            ));
                        }
                        let target = (index * stride + output.offset) as usize;
                        bytes[target..target + input.format.size() as usize]
                            .copy_from_slice(&buffer.bytes[from as usize..end as usize]);
                    }
                }
            }
        }
        out.layouts.push(packed);
        out.bytes.push(bytes);
    }
    Ok(out)
}

/// Whether sorted, disjoint `known` ranges cover `from..end`.
fn covered(known: &[std::ops::Range<u64>], from: u64, end: u64) -> bool {
    let mut index = known.partition_point(|range| range.end <= from);
    let mut at = from;
    while at < end {
        match known.get(index) {
            Some(range) if range.start <= at => at = range.end,
            _ => return false,
        }
        index += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_fetches_preserve_formats_offsets_nulls_and_unknown_bytes() {
        let layout = VertexLayout {
            stride: 4,
            step: wgpu::VertexStepMode::Vertex,
            attributes: vec![wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32x2,
                offset: 4,
                shader_location: 3,
            }],
        };
        let bytes: Vec<_> = (0_u8..20).collect();
        let complete = 0..20;
        let partial = 0..12;
        let mut sources = BTreeMap::from([(
            0,
            Some(NativeBuffer {
                bytes: &bytes,
                size: 20,
                offset: 4,
                known: std::slice::from_ref(&complete),
            }),
        )]);
        let out = pack_native(std::slice::from_ref(&layout), &sources, 4, 1).unwrap();
        assert_eq!(out.layouts[0].stride, 8);
        assert_eq!(
            out.layouts[0].attributes[0].format,
            wgpu::VertexFormat::Uint32x2
        );
        assert_eq!(
            out.bytes[0],
            [bytes[8..16].to_vec(), bytes[12..20].to_vec(), vec![0; 16]].concat()
        );
        sources.get_mut(&0).unwrap().as_mut().unwrap().known = std::slice::from_ref(&partial);
        assert!(
            pack_native(std::slice::from_ref(&layout), &sources, 4, 1)
                .err()
                .unwrap()
                .contains("unavailable bytes 8..16")
        );
        sources.insert(0, None);
        assert_eq!(
            pack_native(std::slice::from_ref(&layout), &sources, 4, 1)
                .unwrap()
                .bytes[0],
            [0; 32]
        );
        sources.clear();
        assert!(
            pack_native(&[layout], &sources, 4, 1)
                .err()
                .unwrap()
                .contains("slot 0 has not been supplied")
        );
    }

    #[test]
    fn written_prefix_of_a_larger_allocation_reads_known_ranges_only() {
        // A ring: 4-byte records with 3 known bytes each, written 2 of 4 records deep.
        let layout = VertexLayout {
            stride: 4,
            step: wgpu::VertexStepMode::Instance,
            attributes: vec![
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint8x2,
                    offset: 1,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint8x2,
                    offset: 0,
                    shader_location: 1,
                },
            ],
        };
        let bytes: Vec<_> = (10_u8..18).collect();
        let known = [0..2, 2..3, 4..7];
        let source = |offset| {
            BTreeMap::from([(
                0,
                Some(NativeBuffer {
                    bytes: &bytes,
                    size: 16,
                    offset,
                    known: &known,
                }),
            )])
        };
        let out = pack_native(std::slice::from_ref(&layout), &source(0), 1, 2).unwrap();
        // Adjacent known ranges cover a fetch across them.
        assert_eq!(
            out.bytes[0],
            [11, 12, 0, 0, 10, 11, 0, 0, 15, 16, 0, 0, 14, 15, 0, 0]
        );
        // Inside the allocation but past the written prefix is unknown, not zero.
        let error = pack_native(std::slice::from_ref(&layout), &source(0), 1, 3)
            .err()
            .unwrap();
        assert!(error.contains("record 2") && error.contains("unavailable bytes 9..11"));
        // Past the allocation's size reads zero.
        let out = pack_native(std::slice::from_ref(&layout), &source(16), 1, 2).unwrap();
        assert_eq!(out.bytes[0], [0; 16]);
    }
}
