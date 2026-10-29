//! Preflight-only isolation; submission still follows on one encoder.
use crate::map::shader_scene::State;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
};
#[cfg(test)]
use viewer_foundation::map::gpu_contract::compiled;

#[cfg(test)]
#[path = "scene_passes_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Pass {
    Underwater,
    Terrain,
    LandBorders,
    Rivers,
    Railways,
    Trees,
    Water,
    SeaBorders,
    Demilitarized,
    Straits,
    /// The viewer's hatch over unknown snow and mud, on the ground and on the water.
    UnknownSnowMud,
    CountryNames,
    Sky,
}

impl Pass {
    pub fn name(self) -> &'static str {
        match self {
            Self::Underwater => "underwater",
            Self::Terrain => "terrain",
            Self::LandBorders => "land borders",
            Self::Rivers => "rivers",
            Self::Railways => "railways",
            Self::Trees => "trees",
            Self::Water => "water",
            Self::SeaBorders => "sea borders",
            Self::Demilitarized => "demilitarized",
            Self::Straits => "straits",
            Self::UnknownSnowMud => "unknown snow and mud",
            Self::CountryNames => "country names",
            Self::Sky => "sky",
        }
    }

    // river/rail/tree/water inputs retain land-border PS b2; water also binds
    // the exclusive refraction target at t5. Other colour/depth writes accumulate
    // on an independently cleared target, so missing contributions are harmless.
    fn inputs(self) -> &'static [Self] {
        match self {
            Self::Rivers | Self::Railways | Self::Trees => &[Self::LandBorders],
            Self::Water => &[Self::LandBorders, Self::Underwater],
            _ => &[],
        }
    }

    fn depends_on(self, input: Self) -> bool {
        self == input || self.inputs().iter().any(|pass| pass.depends_on(input))
    }
}

pub type Diagnostics = BTreeMap<Pass, String>;

pub struct Frame<'a> {
    diagnostics: &'a mut Diagnostics,
    failed: BTreeSet<Pass>,
}

impl<'a> Frame<'a> {
    pub fn new(diagnostics: &'a mut Diagnostics) -> Self {
        Self {
            diagnostics,
            failed: BTreeSet::new(),
        }
    }

    pub fn ready(&mut self, pass: Pass) -> bool {
        if let Some(input) = self.failed.iter().find(|&&input| pass.depends_on(input)) {
            self.diagnostics
                .entry(pass)
                .or_insert_with(|| format!("skipped: output of {} is unavailable", input.name()));
            return false;
        }
        true
    }

    /// Only for draws whose last possible error precedes their first command.
    /// terrain prepares separately; border/river/rail/strait/water/name/sky
    /// adapters collect every binding before begin_render_pass. Mid-encode
    /// tree, arrow, particle, shadow and post errors remain whole-frame.
    /// Models prepare all parts before drawing; actor content/binding refusals draw
    /// nothing. Shared library/pipeline/arena/pool/device errors stay whole-frame.
    pub fn run<T>(&mut self, pass: Pass, draw: impl FnOnce() -> io::Result<T>) -> Option<T> {
        if !self.ready(pass) {
            return None;
        }
        match draw() {
            Ok(value) => Some(value),
            Err(error) => {
                self.failed.insert(pass);
                self.diagnostics
                    .entry(pass)
                    .or_insert_with(|| error.to_string());
                None
            }
        }
    }

    pub fn world<T>(
        &mut self,
        pass: Pass,
        retained: &RefCell<State>,
        draw: impl FnOnce() -> io::Result<T>,
    ) -> Option<T> {
        let mut before = State::default();
        before.inherit(&retained.borrow());
        let result = self.run(pass, draw);
        if result.is_none() {
            retained.borrow_mut().inherit(&before);
        }
        result
    }
}

pub fn lines(diagnostics: &Diagnostics) -> impl Iterator<Item = String> + '_ {
    diagnostics
        .iter()
        .map(|(pass, reason)| format!("Map shader {}: 1 refused pass: {reason}", pass.name()))
}
