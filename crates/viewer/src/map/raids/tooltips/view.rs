//! Egui presentation for the map's existing hover popup. Text markup uses
//! main's combat parser and assets; image decoding stays on the hover worker.
use super::Tooltip;
use crate::{
    combat::{self, TextChunk},
    map::sprites::MapIcons,
};
use eframe::egui;
use std::{collections::BTreeMap, sync::Arc};

pub struct Prepared {
    pub tooltip: Tooltip,
    lines: Vec<Vec<Run>>,
    images: Vec<Arc<egui::ColorImage>>,
}
enum Run {
    Text(String, [f32; 4]),
    Image(usize, [f32; 2]),
}

pub fn prepare(mut tooltip: Tooltip, icons: &MapIcons) -> Prepared {
    let text = if tooltip.body.is_empty() {
        tooltip.title.clone()
    } else if tooltip.title.is_empty() {
        tooltip.body.clone()
    } else {
        format!("{}\n\n{}", tooltip.title, tooltip.body)
    };
    let base = [1.; 4];
    let mut color = base;
    let chunks: Vec<_> = text
        .split('\n')
        .map(|line| combat::text_chunks(line, &icons.combat.text_colors, base, &mut color))
        .collect();
    // Queue inline flags/portraits before draining the shared loader once.
    for chunk in chunks.iter().flatten() {
        match chunk {
            TextChunk::Flag(key) => {
                icons.combat.inline_flag(key);
            }
            TextChunk::Icon(name) => {
                icons.combat.sprite(name);
            }
            TextChunk::Text(..) => {}
        }
    }
    icons.lazy.settle();
    let resident: BTreeMap<_, _> = icons.lazy.snapshot().into_iter().collect();
    let mut images = Vec::new();
    let mut lines = Vec::new();
    for chunks in chunks {
        let mut line = Vec::new();
        for chunk in chunks {
            let (name, source) = match chunk {
                TextChunk::Text(value, color) => {
                    line.push(Run::Text(value, color));
                    continue;
                }
                TextChunk::Flag(key) => {
                    let source = icons
                        .combat
                        .inline_flag(&key)
                        .map(|(id, uv)| (id, uv, icons.combat.inline_flag_size));
                    (format!("inline flag {key}"), source)
                }
                TextChunk::Icon(name) => {
                    let source = icons
                        .combat
                        .sprite(&name)
                        .map(|s| (s.sheet.texture, s.sheet.frame_uv(0), s.size));
                    (name, source)
                }
            };
            let decoded = (|| {
                let (id, uv, size) = source.ok_or("asset absent or unreadable")?;
                let texture = icons
                    .images
                    .get(id)
                    .or_else(|| resident.get(&id).map(|r| r.texture.as_ref()))
                    .ok_or("decoded image absent")?;
                let rgba = combat::texture::rgba8(texture)
                    .map_err(|_| "unsupported tooltip image format")?;
                if rgba.format != installed_content::image::dds::TextureFormat::Rgba8 {
                    return Err("unsupported tooltip image format");
                }
                let [x0, y0, x1, y1] = [
                    uv[0] * rgba.width as f32,
                    uv[1] * rgba.height as f32,
                    uv[2] * rgba.width as f32,
                    uv[3] * rgba.height as f32,
                ]
                .map(|v| v.round() as usize);
                if x0 >= x1 || y0 >= y1 || x1 > rgba.width as usize || y1 > rgba.height as usize {
                    return Err("tooltip image rectangle outside texture");
                }
                let mut bytes = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
                for y in y0..y1 {
                    bytes.extend_from_slice(
                        &rgba.base().data[(y * rgba.width as usize + x0) * 4
                            ..(y * rgba.width as usize + x1) * 4],
                    );
                }
                let image = egui::ColorImage::from_rgba_unmultiplied([x1 - x0, y1 - y0], &bytes);
                Ok((image, size))
            })();
            match decoded {
                Ok((image, size)) => {
                    line.push(Run::Image(images.len(), size));
                    images.push(Arc::new(image));
                }
                Err(reason) => tooltip.refuse(&name, reason.into()),
            }
        }
        lines.push(line);
    }
    Prepared {
        tooltip,
        lines,
        images,
    }
}

#[derive(Default)]
pub struct View {
    shown: Option<Arc<Prepared>>,
    textures: Vec<egui::TextureHandle>,
}
impl View {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        prepared: &Arc<Prepared>,
        scale: f32,
        diagnostics: bool,
    ) {
        if self
            .shown
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, prepared))
        {
            self.textures = prepared
                .images
                .iter()
                .enumerate()
                .map(|(id, image)| {
                    ui.ctx().load_texture(
                        format!("raid tooltip {id}"),
                        Arc::clone(image),
                        egui::TextureOptions::LINEAR,
                    )
                })
                .collect();
            self.shown = Some(Arc::clone(prepared));
        }
        let size = egui::TextStyle::Body.resolve(ui.style()).size * scale;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            for line in &prepared.lines {
                if line.is_empty() {
                    ui.add_space(size);
                    continue;
                }
                ui.horizontal_wrapped(|ui| {
                    for run in line {
                        match run {
                            Run::Text(text, color) => {
                                let [r, g, b, a] = color.map(|v| (v * 255.).round() as u8);
                                ui.label(
                                    egui::RichText::new(text)
                                        .size(size)
                                        .color(egui::Color32::from_rgba_unmultiplied(r, g, b, a)),
                                );
                            }
                            Run::Image(id, size) => {
                                ui.image((
                                    self.textures[*id].id(),
                                    egui::vec2(size[0] * scale, size[1] * scale),
                                ));
                            }
                        }
                    }
                });
            }
        });
        if diagnostics && !prepared.tooltip.refused.is_empty() {
            ui.separator();
            for (reason, count) in &prepared.tooltip.refused {
                ui.label(format!("{count} × {reason}"));
            }
        }
    }
}
