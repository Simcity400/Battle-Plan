//! GUI effects loaded from the resolved content stack and compiled when a technique is used.
//! Missing techniques retain the preceding program and samplers, while their blend state
//! defaults to off. Tests describe selection and scheduling with explicit fixture metadata.
#![cfg_attr(not(test), allow(dead_code))]

use std::sync::Arc;

/// A sampler's addressing on one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Address {
    Wrap,
    Mirror,
    Clamp,
    Border,
    MirrorOnce,
}

/// A sampler's filter (`MagFilter`, `MinFilter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    Point,
    Linear,
}

/// A texture slot's addressing, filtering and mip bias. The border is transparent black and
/// the third axis wraps; an absent mip filter selects the nearest level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampler {
    pub u: Address,
    pub v: Address,
    pub mag: Filter,
    pub min: Filter,
    /// `MipFilter`: `Point` (or `None`) the nearest level, `Linear` the two nearest blended.
    pub mip: Filter,
    /// `MipMapLodBias`, added to the level of detail before the filter is chosen.
    pub lod_bias: f32,
}

impl Sampler {
    /// Ten bits: `u` and `v` (three each), the magnifying and minifying filters, and the mip
    /// filter (1 point, 2 linear).
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

/// How a technique's output meets what is under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Blend {
    /// `BlendEnable = yes`, `src_alpha` over `inv_src_alpha`.
    Alpha,
    /// Blending off, including the default state selected by an absent technique.
    Off,
}

/// Selection and clock metadata for a compiler-independent layout fixture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixtureTechnique {
    pub id: u32,
    pub settles: Option<f32>,
}

/// A selected fixture technique, or an absent technique that retains the previous program.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Technique {
    Run {
        fixture: FixtureTechnique,
        blend: Blend,
    },
    Keep,
}

/// A resolved effect and its executable source. Layout fixtures provide metadata instead.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub file: String,
    pub compiled: Option<Arc<super::native_shader::library::Effect>>,
    pub techniques: Vec<(&'static str, FixtureTechnique, Blend)>,
    /// An absent row retains the preceding draw's sampler in that slot.
    pub samplers: [Option<Sampler>; 6],
}

impl Effect {
    /// Loads the effect named by the sprite through the content stack.
    pub fn load(
        library: &mut super::native_shader::library::Library,
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

    /// Compiled programs derive their clock dependence from their instructions. Fixtures
    /// supply it directly; an absent technique takes the preceding draw's answer.
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

/// Whether a draw still changes after its drawable's restart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Settles {
    /// Seconds after restart, or continuous activity when absent.
    Program(Option<f32>),
    /// The preceding program remains bound, with this draw's constants.
    Previous,
    /// A declared program cannot compile; it and later inherited draws are refused.
    Refused,
}

/// Inspects the selected compiled program using the sprite clock fields.
pub fn compiled_settles(
    effect: &Arc<super::native_shader::library::Effect>,
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

/// Compiler-independent technique, clock and sampler metadata for layout tests.
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
                settles: Some(seconds),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_select_metadata_and_keep_absent_techniques() {
        let effect = synthetic::buttonstate();
        assert_eq!(
            effect.technique("Over"),
            Technique::Run {
                fixture: FixtureTechnique {
                    id: 4,
                    settles: Some(0.25)
                },
                blend: Blend::Alpha,
            }
        );
        assert_eq!(effect.technique("Texture"), Technique::Keep);
        assert!(effect.has_technique("over"));
        assert_eq!(effect.settles("Down"), Settles::Program(Some(0.0625)));
        assert_eq!(effect.settles("Texture"), Settles::Previous);
    }

    #[test]
    fn fixture_clock_metadata_can_describe_continuous_activity() {
        let mut effect = synthetic::buttonstate();
        effect.techniques[0].1.settles = None;
        assert_eq!(effect.settles("Up"), Settles::Program(None));
    }

    #[test]
    fn sampler_metadata_preserves_packed_fields() {
        assert_eq!(
            Sampler {
                u: Address::Mirror,
                v: Address::Wrap,
                mag: Filter::Point,
                min: Filter::Point,
                mip: Filter::Point,
                lod_bias: 0.0,
            }
            .packed(),
            1 | 1 << 8
        );
        assert_eq!(
            Sampler {
                u: Address::MirrorOnce,
                v: Address::Border,
                mag: Filter::Linear,
                min: Filter::Point,
                mip: Filter::Linear,
                lod_bias: 0.0,
            }
            .packed(),
            4 | 3 << 3 | 1 << 6 | 2 << 8
        );
    }
}
