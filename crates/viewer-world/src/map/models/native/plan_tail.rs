//! Compact live supplies and source order from the actual successful native program.
use super::*;
use shader_pipeline::gpu::BindingProgram;

/// Selection is pure; unavailable callers do not refuse until evaluate is reached.
#[derive(Clone, Copy)]
pub(super) enum Caller<'a> {
    Unchanged,
    Defaults,
    Unit,
    Train,
    Unavailable(&'a str),
}
impl<'a> Caller<'a> {
    pub fn select(label: &'a str) -> Self {
        match label {
            "city sprawl" | "railway bridges" => Self::Unchanged,
            "map buildings" | "flying aircraft" | "map frame" => Self::Defaults,
            "unit models" => Self::Unit,
            "supply trains" => Self::Train,
            _ => Self::Unavailable(label),
        }
    }

    // Called ONLY at the post-queue/root/userdata checkpoint, never in layout install.
    pub fn evaluate(
        self,
        source: &mesh_pose::Source,
        instance: ModelInstance,
        alpha: f32,
    ) -> io::Result<Option<[f32; 8]>> {
        let mut data = [0., 0., 1., 1., 1., 0., 0., 0.];
        match self {
            Self::Unchanged => return Ok(None),
            Self::Defaults => {}
            Self::Unit => {
                if instance.native_flags & 1 != 0
                    && let Some(authored) = &source.game_data
                {
                    let authored = authored.as_ref().map_err(|e| invalid(e.clone()))?;
                    data[0] = authored.atlas_size[0].wrapping_sub(1).min(3) as f32;
                    data[2] = authored.atlas_size[0] as f32;
                    data[3] = authored.atlas_size[1] as f32;
                    data[4] = if instance.native_flags & 2 != 0 {
                        authored.texture_anim_speed
                    } else {
                        0.
                    };
                }
            }
            Self::Train => {
                data = [
                    1., 1., 1., alpha,
                    instance.train_clip[0], instance.train_clip[1],
                    instance.train_clip[2], instance.train_clip[3],
                ];
            }
            Self::Unavailable(label) => return Err(invalid(format!(
                "native mesh userdata caller {label:?} unavailable"
            ))),
        }
        Ok(Some(data))
    }
}

// Five source families, NOT an allocation cap: two frame slots share one source.
// Optional supplies preserve absent VS/PS/skin/grid rather than filling missing bytes.
const FRAME: usize = 0;
const VERTEX: usize = 1;
const PIXEL: usize = 2;
const SKIN: usize = 3;
const GRID: usize = 4;
const SOURCES: usize = 5;

#[derive(Clone, Copy)]
pub(super) struct Supplies<'a> {
    bytes: [Option<&'a [u8]>; SOURCES],
}
impl<'a> Supplies<'a> {
    pub fn new(
        frame: &'a [u8], vertex: &'a [u8], pixel: Option<&'a [u8]>,
        skin: Option<&'a [u8]>, grid: Option<&'a [u8]>,
    ) -> Self {
        Self { bytes: [Some(frame), Some(vertex), pixel, skin, grid] }
    }
    pub fn present(self) -> u32 {
        (1 << 0) | (1 << 1) | (1 << 2)
            | if self.bytes[PIXEL].is_some() { 1 << 3 } else { 0 }
            | if self.bytes[SKIN].is_some() { 1 << 4 } else { 0 }
            | if self.bytes[GRID].is_some() { 1 << 5 } else { 0 }
    }
    /// Direct lookup remains the cold/error/diagnostic fallback, not a warm reflected dispatch.
    pub fn get(self, binding: u32) -> Option<&'a [u8]> {
        selector(binding).and_then(|source| self.bytes[source])
    }
    pub fn constants(self) -> Constants<'a> {
        std::array::from_fn(|binding| self.get(binding as u32))
    }

}

fn selector(binding: u32) -> Option<usize> {
    match binding {
        0 | 1 => Some(FRAME),
        2 => Some(VERTEX),
        3 => Some(PIXEL),
        4 => Some(SKIN),
        5 => Some(GRID),
        _ => None,
    }
}

/// Created from the ACTUAL successful effective BindingProgram, not Resident's
/// requested program or a guessed vertex/pixel traversal order. Published only
/// alongside that descriptor's successful capture. No per-bind allocation.
pub(super) struct Recipe {
    sources: Vec<Option<usize>>,
}
impl Recipe {
    // Optional accelerator failure must not add a new content/device refusal.
    pub fn new(program: &BindingProgram) -> Option<Self> {
        Self::from_bindings(program.constant_reads().iter().map(|spec| spec.binding))
    }
    fn from_bindings(bindings: impl ExactSizeIterator<Item = u32>) -> Option<Self> {
        let mut sources = Vec::new();
        sources.try_reserve_exact(bindings.len()).ok()?;
        sources.extend(bindings.map(selector));
        Some(Self { sources })
    }
    pub fn ordered<'r, 'a: 'r>(
        &'r self, values: Supplies<'a>,
    ) -> impl ExactSizeIterator<Item = Option<&'a [u8]>> + 'r {
        self.sources.iter().map(move |index| index.and_then(|index| values.bytes[index]))
    }
}

/// Binding and recipe cannot be replaced independently. If optional recipe allocation
/// fails, the successful program still binds by direct source lookup.
pub(super) struct Program {
    bindings: BindingProgram,
    recipe: Option<Recipe>,
}
impl Program {
    pub fn new(bindings: BindingProgram) -> Self {
        let recipe = Recipe::new(&bindings);
        Self { bindings, recipe }
    }
    pub fn recipe(&self) -> Option<&Recipe> { self.recipe.as_ref() }

    #[cfg(test)]
    pub fn omit_recipe_for_tests(&mut self) { self.recipe = None; }
}
impl std::ops::Deref for Program {
    type Target = BindingProgram;
    fn deref(&self) -> &BindingProgram { &self.bindings }
}

#[cfg(test)]
mod tests;
