//! Water technique selection and compiled pipeline cache.
use eframe::wgpu;
use std::{cell::RefCell, io, sync::Arc};
use viewer_foundation::map::gpu_contract::DEPTH_FORMAT;

/// Separate linear sea-bed target bound at t5.
pub struct WaterPass {
    compiled: RefCell<Option<(String, Result<super::water_shader::Water, String>)>>,
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
    /// Whether the water this frame drew reads the snow/mud texture, so snow can show on it.
    pub fn shows_snow_mud(&self) -> bool {
        self.compiled
            .borrow()
            .as_ref()
            .is_some_and(|(_, water)| water.as_ref().is_ok_and(|water| water.reads_snow_mud))
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
                )
                .map_err(|error| error.to_string()),
            ));
        }
        viewer_foundation::map::gpu_contract::compiled(
            &compiled.as_ref().expect("created above").1,
        )?
        .draw(arena, encoder, target, inputs)
    }
}
