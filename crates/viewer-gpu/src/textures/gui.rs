use super::{Bound, ColorSpace, check_upload, dds, format_of, invalid, upload_unpacking};
use content_reader_gui::gui::native::resources;
use content_reader_image::native::Decode;
use content_reader_layers::ResolvedLayers;
use eframe::wgpu;
use std::{collections::HashMap, io, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuiMeta {
    pub width: u32,
    pub height: u32,
    pub faces: u32,
    pub levels: u32,
    pub format: dds::TextureFormat,
}

impl GuiMeta {
    pub fn of(texture: &dds::Texture) -> Self {
        Self {
            width: texture.width,
            height: texture.height,
            faces: texture.faces,
            levels: texture.mip_count() as u32,
            format: texture.format,
        }
    }
}

#[derive(Clone)]
pub struct GuiSource(Arc<GuiSourceInner>);

enum GuiSourceInner {
    Held(dds::Texture),
    Stack {
        layers: Arc<ResolvedLayers>,
        recorded: Option<Arc<ResolvedLayers>>,
        path: Vec<u8>,
        decode: Decode,
        meta: GuiMeta,
    },
    Generated {
        label: String,
        meta: GuiMeta,
        build: Box<dyn Fn(bool) -> io::Result<(dds::Texture, Vec<String>)> + Send + Sync>,
    },
}

impl From<dds::Texture> for GuiSource {
    fn from(texture: dds::Texture) -> Self {
        Self::held(texture)
    }
}

impl GuiSource {
    pub fn held(texture: dds::Texture) -> Self {
        Self(Arc::new(GuiSourceInner::Held(texture)))
    }

    pub fn deferred_stack(
        layers: Arc<ResolvedLayers>,
        recorded: Option<Arc<ResolvedLayers>>,
        path: Vec<u8>,
        decode: Decode,
        texture: &dds::Texture,
    ) -> Self {
        Self(Arc::new(GuiSourceInner::Stack {
            layers,
            recorded,
            path,
            decode,
            meta: GuiMeta::of(texture),
        }))
    }

    pub fn deferred_generated(
        label: impl Into<String>,
        meta: GuiMeta,
        build: impl Fn(bool) -> io::Result<(dds::Texture, Vec<String>)> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(GuiSourceInner::Generated {
            label: label.into(),
            meta,
            build: Box::new(build),
        }))
    }

    pub fn meta(&self) -> GuiMeta {
        match self.0.as_ref() {
            GuiSourceInner::Held(texture) => GuiMeta::of(texture),
            GuiSourceInner::Stack { meta, .. } | GuiSourceInner::Generated { meta, .. } => *meta,
        }
    }

    pub fn is_held(&self) -> bool {
        matches!(self.0.as_ref(), GuiSourceInner::Held(_))
    }

    pub fn decode_optional(&self) -> io::Result<Result<dds::Texture, resources::NoTexture>> {
        match self.0.as_ref() {
            GuiSourceInner::Held(texture) => Ok(Ok(texture.clone())),
            GuiSourceInner::Stack {
                layers,
                recorded,
                path,
                decode,
                ..
            } => resources::stack_texture_or_recorded_transient(
                layers,
                recorded.as_deref(),
                path,
                *decode,
            )
            .map(|found| found.map(|texture| texture.image)),
            GuiSourceInner::Generated { build, .. } => build(false).map(|(texture, _)| Ok(texture)),
        }
    }

    pub fn path(&self) -> Option<&[u8]> {
        match self.0.as_ref() {
            GuiSourceInner::Held(_) | GuiSourceInner::Generated { .. } => None,
            GuiSourceInner::Stack { path, .. } => Some(path.as_slice()),
        }
    }

    pub fn decode(&self) -> io::Result<dds::Texture> {
        let path = self.path();
        self.decode_optional()?.map_err(|why| {
            invalid(format!(
                "{}: {why}",
                path.map(|bytes| String::from_utf8_lossy(bytes))
                    .unwrap_or_default()
            ))
        })
    }
}

impl std::fmt::Debug for GuiSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.as_ref() {
            GuiSourceInner::Held(texture) => formatter
                .debug_struct("GuiSource::Held")
                .field("meta", &GuiMeta::of(texture))
                .finish(),
            GuiSourceInner::Stack { path, meta, .. } => formatter
                .debug_struct("GuiSource::Deferred")
                .field("path", &String::from_utf8_lossy(path))
                .field("meta", meta)
                .finish(),
            GuiSourceInner::Generated { label, meta, .. } => formatter
                .debug_struct("GuiSource::Generated")
                .field("label", label)
                .field("meta", meta)
                .finish(),
        }
    }
}

enum DecodedItem<'a> {
    Ready {
        texture: std::borrow::Cow<'a, dds::Texture>,
        warnings: Vec<String>,
    },
    Missing(String),
}

trait GuiUploadItem {
    fn meta(&self) -> GuiMeta;
    fn label(&self, index: usize) -> String;
    fn decode_item(&self, tolerant: bool) -> io::Result<DecodedItem<'_>>;
}

impl GuiUploadItem for dds::Texture {
    fn meta(&self) -> GuiMeta {
        GuiMeta::of(self)
    }

    fn label(&self, index: usize) -> String {
        format!("map icon {index}")
    }

    fn decode_item(&self, _tolerant: bool) -> io::Result<DecodedItem<'_>> {
        Ok(DecodedItem::Ready {
            texture: std::borrow::Cow::Borrowed(self),
            warnings: Vec::new(),
        })
    }
}

impl GuiUploadItem for GuiSource {
    fn meta(&self) -> GuiMeta {
        GuiSource::meta(self)
    }

    fn label(&self, index: usize) -> String {
        match self.0.as_ref() {
            GuiSourceInner::Held(_) => format!("map icon {index}"),
            GuiSourceInner::Stack { path, .. } => {
                format!("map icon {index} ({})", String::from_utf8_lossy(path))
            }
            GuiSourceInner::Generated { label, .. } => format!("map icon {index} ({label})"),
        }
    }

    fn decode_item(&self, tolerant: bool) -> io::Result<DecodedItem<'_>> {
        match self.0.as_ref() {
            GuiSourceInner::Held(texture) => Ok(DecodedItem::Ready {
                texture: std::borrow::Cow::Borrowed(texture),
                warnings: Vec::new(),
            }),
            GuiSourceInner::Stack { path, .. } => match self.decode_optional() {
                Ok(Ok(texture)) => Ok(DecodedItem::Ready {
                    texture: std::borrow::Cow::Owned(texture),
                    warnings: Vec::new(),
                }),
                Ok(Err(why)) if tolerant => Ok(DecodedItem::Missing(format!(
                    "{}: {why}",
                    String::from_utf8_lossy(path)
                ))),
                Ok(Err(why)) => Err(invalid(format!(
                    "{}: {why}",
                    String::from_utf8_lossy(path)
                ))),
                Err(error) if tolerant => Ok(DecodedItem::Missing(format!(
                    "{}: {error}",
                    String::from_utf8_lossy(path)
                ))),
                Err(error) => Err(error),
            },
            GuiSourceInner::Generated { label, build, .. } => match build(tolerant) {
                Ok((texture, warnings)) => Ok(DecodedItem::Ready {
                    texture: std::borrow::Cow::Owned(texture),
                    warnings,
                }),
                Err(error) if tolerant => Ok(DecodedItem::Missing(format!("{label}: {error}"))),
                Err(error) => Err(error),
            },
        }
    }
}

fn transparent_placeholder(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    unpack: &wgpu::ComputePipeline,
) -> io::Result<GuiTexture> {
    let blank = dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![0, 0, 0, 0],
    });
    GuiTexture::upload(device, queue, label, &blank, unpack)
}

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

fn check_meta(label: &str, meta: GuiMeta) -> io::Result<()> {
    let cube = meta.faces == 6;
    if meta.faces != 1 && !cube {
        return Err(invalid(format!(
            "{label} has {} faces; only a plain texture or a cube map can be uploaded",
            meta.faces
        )));
    }
    if cube && meta.width != meta.height {
        return Err(invalid(format!(
            "{label} is a {} x {} cube map; this device requires square cube-map faces",
            meta.width, meta.height
        )));
    }
    format_of(meta.format, ColorSpace::Linear)?;
    if meta.width == 0 || meta.height == 0 {
        return Err(invalid(format!("{label} has an empty texture extent")));
    }
    Ok(())
}

pub fn upload_gui(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &[dds::Texture],
    unpack: &wgpu::ComputePipeline,
) -> io::Result<Vec<GuiTexture>> {
    upload_gui_items(device, queue, images, unpack, None)
}

pub fn upload_gui_sources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &[GuiSource],
    unpack: &wgpu::ComputePipeline,
) -> io::Result<Vec<GuiTexture>> {
    upload_gui_items(device, queue, images, unpack, None)
}

pub fn upload_gui_sources_tolerant(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &[GuiSource],
    unpack: &wgpu::ComputePipeline,
    warnings: &mut Vec<String>,
) -> io::Result<Vec<GuiTexture>> {
    upload_gui_items(device, queue, images, unpack, Some(warnings))
}

fn upload_gui_items<T: GuiUploadItem>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &[T],
    unpack: &wgpu::ComputePipeline,
    mut warnings: Option<&mut Vec<String>>,
) -> io::Result<Vec<GuiTexture>> {
    let mut groups: Vec<(Shape, Vec<usize>)> = Vec::new();
    let mut shapes = HashMap::new();
    let mut output: Vec<Option<GuiTexture>> = (0..images.len()).map(|_| None).collect();
    for (index, image) in images.iter().enumerate() {
        let meta = image.meta();
        check_meta(&format!("map icon {index}"), meta)?;
        if meta.faces != 1 {
            output[index] = Some(match image.decode_item(warnings.is_some())? {
                DecodedItem::Ready {
                    texture: decoded,
                    warnings: item_warnings,
                } => {
                    if let Some(warnings) = warnings.as_deref_mut() {
                        warnings.extend(item_warnings);
                    }
                    check_upload(&format!("map icon {index}"), &decoded, ColorSpace::Linear)?;
                    GuiTexture::upload(
                        device,
                        queue,
                        &format!("map icon {index}"),
                        &decoded,
                        unpack,
                    )?
                }
                DecodedItem::Missing(reason) => {
                    if let Some(warnings) = warnings.as_deref_mut() {
                        warnings.push(format!("map icon {index} ({reason})"));
                    }
                    transparent_placeholder(device, queue, &format!("map icon {index}"), unpack)?
                }
            });
            continue;
        }
        if meta.width > device.limits().max_texture_dimension_2d
            || meta.height > device.limits().max_texture_dimension_2d
        {
            return Err(invalid(format!(
                "map icon {index}: {}x{} exceeds the device texture extent",
                meta.width, meta.height
            )));
        }
        let cropped = meta.format.block_bytes().is_some()
            && (!meta.width.is_multiple_of(4) || !meta.height.is_multiple_of(4));
        let format = format_of(meta.format, ColorSpace::Linear)?;
        let shape = Shape {
            width: meta.width,
            height: meta.height,
            levels: meta.levels,
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
            upload_group(
                device,
                queue,
                images,
                &mut output,
                shape,
                indices,
                unpack,
                warnings.as_deref_mut(),
            )?;
        }
    }
    Ok(output
        .into_iter()
        .map(|image| image.expect("every source uploaded once"))
        .collect())
}

#[expect(clippy::too_many_arguments, reason = "device, queue, source slice, output slots, group shape, layer indices, unpack pipeline, and optional warnings sink")]
fn upload_group<T: GuiUploadItem>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &[T],
    output: &mut [Option<GuiTexture>],
    shape: Shape,
    indices: &[usize],
    unpack: &wgpu::ComputePipeline,
    mut warnings: Option<&mut Vec<String>>,
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
            upload_group(
                device,
                queue,
                images,
                output,
                shape,
                left,
                unpack,
                warnings.as_deref_mut(),
            )?;
            return upload_group(
                device,
                queue,
                images,
                output,
                shape,
                right,
                unpack,
                warnings,
            );
        }
        Err(reason) => {
            return Err(io::Error::other(format!(
                "map icon {}: {reason}",
                indices[0]
            )));
        }
    };
    for (layer, index) in indices.iter().enumerate() {
        let expected = images[*index].meta();
        match images[*index].decode_item(warnings.is_some())? {
            DecodedItem::Ready {
                texture: decoded,
                warnings: item_warnings,
            } => {
                if let Some(warnings) = warnings.as_deref_mut() {
                    warnings.extend(item_warnings);
                }
                let actual = GuiMeta::of(&decoded);
                if actual != expected {
                    let message = format!(
                        "{} changed shape on reload ({}x{} {} faces {} mips {:?} -> {}x{} {} faces {} mips {:?})",
                        images[*index].label(*index),
                        expected.width,
                        expected.height,
                        expected.faces,
                        expected.levels,
                        expected.format,
                        actual.width,
                        actual.height,
                        actual.faces,
                        actual.levels,
                        actual.format
                    );
                    if let Some(warnings) = warnings.as_deref_mut() {
                        warnings.push(message);
                        output[*index] = Some(transparent_placeholder(
                            device,
                            queue,
                            &format!("map icon {index}"),
                            unpack,
                        )?);
                        continue;
                    }
                    return Err(invalid(message));
                }
                let bound = upload_unpacking(
                    device,
                    queue,
                    &format!("map icon {index}"),
                    &decoded,
                    ColorSpace::Linear,
                    Some(unpack),
                    Some((&texture, layer as u32)),
                )?;
                output[*index] = Some(GuiTexture {
                    bound,
                    array: Some((view.clone(), layer as u32)),
                });
            }
            DecodedItem::Missing(reason) => {
                if let Some(warnings) = warnings.as_deref_mut() {
                    warnings.push(format!("map icon {index} ({reason})"));
                }
                output[*index] = Some(transparent_placeholder(
                    device,
                    queue,
                    &format!("map icon {index}"),
                    unpack,
                )?);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerant_decode_turns_outer_io_errors_into_named_missing_warnings() {
        let root = std::env::temp_dir().join(format!(
            "bp_tolerant_locked_gui_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("base");
        std::fs::create_dir_all(base.join("gfx/interface")).unwrap();
        let tga = viewer_runtime::test_support::test_tga_rgba8(4, 4);
        let icon_path = base.join("gfx/interface/locked.tga");
        std::fs::write(&icon_path, &tga).unwrap();

        let layers = Arc::new(ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap());
        let initial = resources::stack_texture_or_recorded_transient(
            &layers,
            None,
            b"gfx/interface/locked.tga",
            Decode::default(),
        )
        .unwrap()
        .unwrap()
        .image;
        let source = GuiSource::deferred_stack(
            Arc::clone(&layers),
            None,
            b"gfx/interface/locked.tga".to_vec(),
            Decode::default(),
            &initial,
        );

        use std::os::windows::fs::OpenOptionsExt;
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&icon_path)
            .unwrap();
        assert!(source.decode_optional().is_err());
        assert!(source.decode_item(false).is_err());
        match source.decode_item(true).unwrap() {
            DecodedItem::Missing(warning) => {
                assert!(warning.contains("gfx/interface/locked.tga"), "{warning}");
            }
            DecodedItem::Ready { .. } => panic!("locked icon should not decode"),
        }
        drop(locked);
        let _ = std::fs::remove_dir_all(&root);
    }
}
