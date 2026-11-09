use super::TextureBinding;
use std::collections::HashMap;

struct View {
    array: wgpu::TextureView,
    used: u64,
}

#[derive(Default)]
pub struct Views {
    images: HashMap<wgpu::TextureView, View>,
    generation: u64,
}

impl Views {
    pub fn begin(&mut self) {
        self.generation += 1;
    }

    pub fn reserve(
        &mut self,
        device: &wgpu::Device,
        binding: &TextureBinding,
    ) -> Result<(), String> {
        if binding.array.is_some() {
            return Ok(());
        }
        if let Some(view) = self.images.get_mut(&binding.view) {
            view.used = self.generation;
            return Ok(());
        }
        let texture = binding.view.texture();
        if texture.dimension() != wgpu::TextureDimension::D2
            || texture.depth_or_array_layers() != 1
            || texture.sample_count() != 1
        {
            return Err("source is not a single-sampled two-dimensional image".into());
        }
        let array = super::scoped(device, || {
            texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(binding.format),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
                ..Default::default()
            })
        })?;
        self.images.insert(
            binding.view.clone(),
            View {
                array,
                used: self.generation,
            },
        );
        Ok(())
    }

    pub fn get<'a>(&'a self, binding: &'a TextureBinding) -> (&'a wgpu::TextureView, u32) {
        if let Some((view, layer)) = &binding.array {
            (view, *layer)
        } else {
            (&self.images[&binding.view].array, 0)
        }
    }

    pub fn finish(&mut self) {
        self.images.retain(|_, view| view.used == self.generation);
    }
}
