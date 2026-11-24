
pub use shader_pipeline::clock::Settling;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Address {
    Wrap,
    Mirror,
    Clamp,
    Border,
    MirrorOnce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    Point,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampler {
    pub u: Address,
    pub v: Address,
    pub mag: Filter,
    pub min: Filter,
    pub mip: Filter,
    pub lod_bias: f32,
}

impl Sampler {
    pub fn packed(self) -> u32 {
        let address = |address: Address| match address {
            Address::Wrap => 0,
            Address::Mirror => 1,
            Address::Clamp => 2,
            Address::Border => 3,
            Address::MirrorOnce => 4,
        };
        let filter = |filter: Filter| u32::from(filter == Filter::Linear);
        let mip = match self.mip {
            Filter::Point => 1,
            Filter::Linear => 2,
        };
        address(self.u)
            | address(self.v) << 3
            | filter(self.mag) << 6
            | filter(self.min) << 7
            | mip << 8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Blend {
    Alpha,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixtureTechnique {
    pub id: u32,
    pub settles: Option<Settling>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Technique {
    Run {
        fixture: FixtureTechnique,
        blend: Blend,
    },
    Keep,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub file: String,
    pub compiled: Option<Arc<shader_pipeline::library::Effect>>,
    pub techniques: Vec<(&'static str, FixtureTechnique, Blend)>,
    pub samplers: [Option<Sampler>; 6],
}

impl Effect {
    pub fn load(
        library: &mut shader_pipeline::library::Library,
        file: &[u8],
        defines: &[String],
    ) -> Result<Self, String> {
        Ok(Self {
            file: String::from_utf8_lossy(file).into_owned(),
            compiled: Some(library.load(file, defines)?),
            techniques: Vec::new(),
            samplers: [None; 6],
        })
    }

    pub fn has_technique(&self, name: &str) -> bool {
        self.compiled.as_ref().map_or_else(
            || {
                self.techniques
                    .iter()
                    .any(|(own, _, _)| own.eq_ignore_ascii_case(name))
            },
            |effect| effect.source.technique(name).is_some(),
        )
    }

    pub fn technique(&self, name: &str) -> Technique {
        self.techniques
            .iter()
            .find(|(own, _, _)| *own == name)
            .map_or(Technique::Keep, |(_, fixture, blend)| Technique::Run {
                fixture: *fixture,
                blend: *blend,
            })
    }

    pub fn settles(&self, name: &str) -> Settles {
        if let Some(compiled) = &self.compiled {
            return compiled_settles(compiled, name);
        }
        match self.technique(name) {
            Technique::Run { fixture, .. } => Settles::Program(fixture.settles),
            Technique::Keep => Settles::Previous,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Settles {
    Program(Option<Settling>),
    Previous,
    Refused,
}

pub fn compiled_settles(
    effect: &Arc<shader_pipeline::library::Effect>,
    name: &str,
) -> Settles {
    use super::native_shader::draw::{SPRITE_ANIMATION_TIME, SPRITE_TIME};
    match effect.select(name) {
        Ok(Some(selection)) => match selection.effect.compiled(selection.index) {
            Ok(program) => Settles::Program(program.settles(0, SPRITE_TIME, SPRITE_ANIMATION_TIME)),
            Err(_) => Settles::Refused,
        },
        Ok(None) => Settles::Previous,
        Err(_) => Settles::Refused,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod synthetic {
    use super::*;

    fn row(address: Address, filter: Filter, mip: Filter) -> Sampler {
        Sampler {
            u: address,
            v: address,
            mag: filter,
            min: filter,
            mip,
            lod_bias: 0.0,
        }
    }

    fn technique(
        name: &'static str,
        id: u32,
        seconds: f32,
    ) -> (&'static str, FixtureTechnique, Blend) {
        (
            name,
            FixtureTechnique {
                id,
                settles: Some(Settling::after(seconds)),
            },
            Blend::Alpha,
        )
    }

    pub fn buttonstate() -> Effect {
        Effect {
            file: "gfx/FX/fixture_button.lua".into(),
            compiled: None,
            techniques: vec![
                technique("Up", 1, 0.0),
                technique("Down", 2, 0.0625),
                technique("Disable", 3, 0.0),
                technique("Over", 4, 0.25),
            ],
            samplers: [
                Some(Sampler {
                    lod_bias: -0.375,
                    ..row(Address::Clamp, Filter::Linear, Filter::Point)
                }),
                Some(row(Address::Mirror, Filter::Linear, Filter::Point)),
                Some(row(Address::Wrap, Filter::Point, Filter::Point)),
                Some(row(Address::Border, Filter::Linear, Filter::Point)),
                Some(row(Address::MirrorOnce, Filter::Point, Filter::Point)),
                Some(row(Address::Clamp, Filter::Point, Filter::Point)),
            ],
        }
    }

    pub fn progress() -> Effect {
        let point = row(Address::Clamp, Filter::Point, Filter::Point);
        Effect {
            file: "gfx/FX/fixture_bar.lua".into(),
            compiled: None,
            techniques: vec![technique("Color", 5, 0.0), technique("Texture", 6, 0.0)],
            samplers: [Some(point), Some(point), None, None, None, None],
        }
    }

    pub fn circular() -> Effect {
        let linear = row(Address::Mirror, Filter::Linear, Filter::Point);
        Effect {
            file: "gfx/FX/fixture_ring.shader".into(),
            compiled: None,
            techniques: vec![technique("CircularProgressBar", 7, 0.0)],
            samplers: [Some(linear), Some(linear), None, None, None, None],
        }
    }

    pub fn text() -> Effect {
        let glyphs = row(Address::Clamp, Filter::Linear, Filter::Linear);
        Effect {
            file: "gfx/FX/fixture_glyphs.lua".into(),
            compiled: None,
            techniques: vec![technique("Text", 8, 0.0)],
            samplers: [Some(glyphs), None, None, None, None, None],
        }
    }
}

