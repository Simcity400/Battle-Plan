use super::{Bound, ColorSpace, check_upload, dds, format_of, invalid, upload_unpacking};
use eframe::wgpu;
use std::{collections::HashMap, io};

pub struct GuiTexture {
    pub bound: Bound,
    array: Option<(wgpu::TextureView, u32)>,
}

impl std::ops::Deref for GuiTexture {
    type Target = Bound;
    fn deref(&self) -> &Bound {
        &self.bound
    }
}

impl GuiTexture {
    pub fn binding(&self) -> shader_pipeline::gpu::TextureBinding {
        shader_pipeline::gpu::TextureBinding {
            view: self.bound.view.clone(),
            format: self.bound.view_format,
            array: self.array.clone(),
        }
    }

    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        image: &dds::Texture,
        unpack: &wgpu::ComputePipeline,
    ) -> io::Result<Self> {
        let bound = upload_unpacking(
            device,
            queue,
            label,
            image,
            ColorSpace::Linear,
            Some(unpack),
            None,
        )?;
        let array = (image.faces == 1).then(|| {
            (
                bound.texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(bound.view_format),
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
                    ..Default::default()
                }),
                0,
            )
        });
        Ok(Self { bound, array })
    }

    pub fn update_cells(
        &self,
        queue: &wgpu::Queue,
        image: &dds::Texture,
        cells: impl IntoIterator<Item = [u32; 4]>,
    ) {
        let bytes_per_row = image.width * 4;
        for [x, y, width, height] in cells {
            let start = (y * bytes_per_row + x * 4) as usize;
            let end = start + ((height - 1) * bytes_per_row + width * 4) as usize;
            shader_pipeline::gpu::stats::write_texture(
                queue,
                wgpu::TexelCopyTextureInfo {
                    texture: &self.bound.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &image.base().data[start..end],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Shape {
    width: u32,
    height: u32,
    levels: u32,
    format: wgpu::TextureFormat,
    unpack: bool,
}

pub fn upload_gui(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &[dds::Texture],
    unpack: &wgpu::ComputePipeline,
) -> io::Result<Vec<GuiTexture>> {
    let mut groups: Vec<(Shape, Vec<usize>)> = Vec::new();
    let mut shapes = HashMap::new();
    let mut output: Vec<Option<GuiTexture>> = (0..images.len()).map(|_| None).collect();
    for (index, image) in images.iter().enumerate() {
        check_upload(&format!("map icon {index}"), image, ColorSpace::Linear)?;
        if image.faces != 1 {
            output[index] = Some(GuiTexture::upload(
                device,
                queue,
                &format!("map icon {index}"),
                image,
                unpack,
            )?);
            continue;
        }
        if image.width > device.limits().max_texture_dimension_2d
            || image.height > device.limits().max_texture_dimension_2d
        {
            return Err(invalid(format!(
                "map icon {index}: {}x{} exceeds the device texture extent",
                image.width, image.height
            )));
        }
        let cropped = image.format.block_bytes().is_some()
            && (!image.width.is_multiple_of(4) || !image.height.is_multiple_of(4));
        let format = format_of(image.format, ColorSpace::Linear)?;
        let shape = Shape {
            width: image.width,
            height: image.height,
            levels: image.mip_count() as u32,
            format: if cropped {
                if format.is_srgb() {
                    wgpu::TextureFormat::Rgba8UnormSrgb
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                }
            } else {
                format
            },
            unpack: cropped,
        };
        let group = *shapes.entry(shape).or_insert_with(|| {
            groups.push((shape, Vec::new()));
            groups.len() - 1
        });
        groups[group].1.push(index);
    }
    for (shape, indices) in groups {
        for indices in indices.chunks(device.limits().max_texture_array_layers as usize) {
            upload_group(device, queue, images, &mut output, shape, indices, unpack)?;
        }
    }
    Ok(output
        .into_iter()
        .map(|image| image.expect("every source uploaded once"))
        .collect())
}

fn upload_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &[dds::Texture],
    output: &mut [Option<GuiTexture>],
    shape: Shape,
    indices: &[usize],
    unpack: &wgpu::ComputePipeline,
) -> io::Result<()> {
    let made = shader_pipeline::gpu::scoped(device, || {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("GUI source images"),
            size: wgpu::Extent3d {
                width: shape.width,
                height: shape.height,
                depth_or_array_layers: indices.len() as u32,
            },
            mip_level_count: shape.levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if shape.unpack {
                shape.format.remove_srgb_suffix()
            } else {
                shape.format
            },
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | if shape.unpack {
                    wgpu::TextureUsages::STORAGE_BINDING
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: if shape.unpack {
                &[wgpu::TextureFormat::Rgba8UnormSrgb]
            } else {
                &[]
            },
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(shape.format),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
            ..Default::default()
        });
        (texture, view)
    });
    let (texture, view) = match made {
        Ok(made) => made,
        Err(reason) if indices.len() > 1 => {
            eprintln!(
                "GUI source allocation {}x{} {:?}, {} layers: {reason}; splitting the same upload",
                shape.width,
                shape.height,
                shape.format,
                indices.len()
            );
            let (left, right) = indices.split_at(indices.len() / 2);
            upload_group(device, queue, images, output, shape, left, unpack)?;
            return upload_group(device, queue, images, output, shape, right, unpack);
        }
        Err(reason) => {
            return Err(io::Error::other(format!(
                "map icon {}: {reason}",
                indices[0]
            )));
        }
    };
    for (layer, index) in indices.iter().enumerate() {
        let bound = upload_unpacking(
            device,
            queue,
            &format!("map icon {index}"),
            &images[*index],
            ColorSpace::Linear,
            Some(unpack),
            Some((&texture, layer as u32)),
        )?;
        output[*index] = Some(GuiTexture {
            bound,
            array: Some((view.clone(), layer as u32)),
        });
    }
    Ok(())
}
