//! Water technique selection and compiled pipeline cache.
use super::scene::DEPTH_FORMAT;
use eframe::wgpu;
use std::{cell::RefCell, io, sync::Arc};

/// Separate linear sea-bed target at t5 (frame33840/event10338).
pub const REFRACTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

pub struct WaterPass {
    compiled: RefCell<Option<(String, super::water_shader::Water)>>,
    effect: Arc<shader_pipeline::library::Effect>,
    map: [u32; 2],
    samples: u32,
    format: wgpu::TextureFormat,
}

impl WaterPass {
    pub fn new(
        effect: &Arc<shader_pipeline::library::Effect>,
        map: [u32; 2],
        samples: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            compiled: RefCell::new(None),
            effect: effect.clone(),
            map,
            samples,
            format,
        }
    }
    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        target: super::border_shader::Target<'_>,
        inputs: super::water_shader::Inputs<'_>,
        high: bool,
        refractions: bool,
    ) -> io::Result<()> {
        let device = arena.device();
        let technique = super::water_shader::technique(high, refractions);
        let mut compiled = self.compiled.borrow_mut();
        if compiled.as_ref().is_none_or(|(name, _)| name != technique) {
            *compiled = Some((
                technique.into(),
                super::water_shader::Water::new(
                    device,
                    &self.effect,
                    self.map,
                    self.samples,
                    self.format,
                    DEPTH_FORMAT,
                    technique,
                )?,
            ));
        }
        compiled
            .as_ref()
            .expect("created above")
            .1
            .draw(arena, encoder, target, inputs)
    }
}
