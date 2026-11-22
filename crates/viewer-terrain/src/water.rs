use viewer_gpu::gpu_contract::{SeaBed, Targets};
use eframe::wgpu;
use std::{cell::RefCell, io, sync::Arc};
use viewer_gpu::gpu_contract::DEPTH_FORMAT;

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
    pub fn shows_snow_mud(&self) -> bool {
        self.compiled
            .borrow()
            .as_ref()
            .is_some_and(|(_, water)| water.as_ref().is_ok_and(|water| water.reads_snow_mud))
    }

    pub fn sea_bed(&self, device: &wgpu::Device, high: bool, refractions: bool) -> SeaBed {
        self.compile(device, super::water_shader::technique(high, refractions));
        self.compiled
            .borrow()
            .as_ref()
            .and_then(|(_, water)| water.as_ref().ok())
            .map_or(SeaBed::Half, |water| water.sea_bed.0)
    }

    pub fn texel_reads(&self, visit: &mut dyn FnMut(&str)) {
        if let Some((_, Ok(water))) = &*self.compiled.borrow()
            && let Some(reads) = &water.texel_reads
        {
            visit(reads);
        }
    }

    pub fn sea_bed_refusal(&self) -> Option<String> {
        self.compiled
            .borrow()
            .as_ref()
            .and_then(|(_, water)| water.as_ref().ok())
            .and_then(|water| water.sea_bed.1.clone())
    }

    pub fn warm(&self, device: &wgpu::Device, high: bool, refractions: bool) {
        self.compile(device, super::water_shader::technique(high, refractions));
    }

    fn compile(&self, device: &wgpu::Device, technique: &'static str) {
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
    }

    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        inputs: super::water_shader::Inputs<'_>,
        high: bool,
        refractions: bool,
    ) -> io::Result<()> {
        self.compile(
            arena.device(),
            super::water_shader::technique(high, refractions),
        );
        let compiled = self.compiled.borrow();
        viewer_gpu::gpu_contract::compiled(
            &compiled.as_ref().expect("compiled above").1,
        )?
        .draw(arena, encoder, targets, inputs)
    }
}
