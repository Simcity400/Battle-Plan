use crate::native::*;

#[derive(Default)]
pub(super) struct Scratch {
    values: Vec<[u8; 208]>,
    current: usize,
    pixel: Option<usize>,
}
#[derive(Clone, Copy)]
pub(super) struct Checkpoint {
    current: usize,
    pixel: Option<usize>,
    length: usize,
}
impl Scratch {
    pub fn begin(&mut self, object: &ObjectConstants) {
        self.values.clear();
        self.values.push(object.bytes);
        self.current = 0;
        self.pixel = object.pixel.map(|pixel| {
            self.values.push(pixel);
            1
        });
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            current: self.current,
            pixel: self.pixel,
            length: self.values.len(),
        }
    }
    pub fn restore(&mut self, before: Checkpoint) {
        self.current = before.current;
        self.pixel = before.pixel;
        self.values.truncate(before.length);
    }
    pub fn update(&mut self, world: &[[f32; 4]; 4], userdata: Option<[f32; 8]>) {
        let mut bytes = self.values[self.current];
        bytes[..64].copy_from_slice(bytemuck::bytes_of(world));
        if let Some(data) = userdata {
            bytes[64..96].copy_from_slice(bytemuck::bytes_of(&data));
        }
        self.current = self.values.len();
        self.values.push(bytes);
        if userdata.is_some() {
            self.pixel = Some(self.current);
        }
    }
    pub fn vertex(&self) -> &[u8] {
        &self.values[self.current]
    }
    pub fn pixel(&self) -> Option<&[u8]> {
        self.pixel.map(|source| self.bytes(source))
    }
    pub fn bytes(&self, source: usize) -> &[u8] {
        &self.values[source]
    }
    pub fn sync(&self, object: &mut ObjectConstants) {
        object.bytes = self.values[self.current];
        object.pixel = self.pixel.map(|source| self.values[source]);
    }
}

#[derive(Clone, Copy)]
pub(super) enum Source {
    Frame(usize),
    Grid,
    Object(usize),
    Skin(usize, usize),
}
pub(super) struct PendingSources {
    pub state: Pending,
    sources: [Option<Source>; CONSTANT_BINDINGS],
}
impl Default for PendingSources {
    fn default() -> Self {
        Self {
            state: Pending::default(),
            sources: [None; CONSTANT_BINDINGS],
        }
    }
}
impl PendingSources {
    pub fn constants_mask(
        &mut self, mask: u32, view: usize, scratch: &Scratch,
        instance: usize, node: usize,
    ) {
        self.constants_mask_at(mask, view, scratch.current, scratch.pixel, instance, node);
    }
    pub fn constants_mask_at(
        &mut self, mask: u32, view: usize, current: usize, pixel: Option<usize>,
        instance: usize, node: usize,
    ) {
        for slot in 0..6 {
            if mask & (1 << slot) == 0
                || (slot / 2 == 0 && self.state.frame == Some(view))
                || (slot == 5 && self.state.grid)
            { continue; }
            self.sources[slot] = Some(match slot {
                0 | 1 => Source::Frame(view),
                2 => Source::Object(current),
                3 => Source::Object(pixel.expect("a supplied PS snapshot has an origin")),
                4 => Source::Skin(instance, node),
                5 => Source::Grid,
                _ => unreachable!(),
            });
        }
        self.state.frame = Some(view);
        self.state.grid |= mask & (1 << 5) != 0;
    }
    pub fn resolve_with<'a>(&self, bytes: impl Fn(Source) -> &'a [u8]) -> Constants<'a> {
        std::array::from_fn(|slot| self.sources[slot].map(&bytes))
    }
    pub fn resolve<'a>(
        &self,
        inputs: Inputs<'a>,
        scratch: &'a Scratch,
        poses: &'a [Option<ActorPoses>],
    ) -> Constants<'a> {
        std::array::from_fn(|slot| {
            self.sources[slot].map(|source| match source {
                Source::Frame(view) => inputs.frames[view],
                Source::Grid => inputs.grid,
                Source::Object(source) => scratch.bytes(source),
                Source::Skin(instance, node) => bytemuck::cast_slice(
                    &poses[instance].as_ref().expect("demanded actor")[node]
                        .pose
                        .as_ref()
                        .expect("checked skin")
                        .bones,
                ),
            })
        })
    }
    pub fn apply(
        self,
        native: &Renderer,
        textures: Textures,
        inputs: Inputs,
        scratch: &Scratch,
        poses: &[Option<ActorPoses>],
        retained: &mut shader_scene::State,
    ) {
        let buffers = self.resolve(inputs, scratch, poses);
        self.state
            .apply_sources(|key| &native.residents[key], textures, buffers, retained);
    }
}

#[cfg(test)]
mod tests;
