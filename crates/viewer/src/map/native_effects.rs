//! The game's GUI effects as [`super::native_gui`] draws them: each effect file's techniques, the
//! program each runs, its blend state and the sampler state of each texture slot, as data the
//! sprite pass ([`super::sprites`]) executes.
//!
//! - A sprite's state picks a technique by name: `CSpriteType::initialize_animations@14222B5F0`
//!   `engine_shader_effect_lookup@1423F4EC0` into `CSpriteType+0x150`, `+0x154`, `+0x158` and
//!   `+0x15C`, and `DrawWithStateEffect@14222D0D0` draws state 2 with Down, 3 with Over, 4 with
//!   Disable and any other with Up. A progress bar looks up `Texture` or `Color`
//!   (`CProgressbarSpriteType::build_resources@14234A5E0`), a bitmap font `Text`.
//! - A technique the file does not declare, or an effect file no layer holds, looks up nothing:
//!   `LookupShaderEffectHandle@1423F5D60` returns the zero-initialised record at `1435C1970`,
//!   whose null program `142400520` leaves the previous draw's program bound, whose null sampler
//!   `1424005A0` leaves the previous sampler in its slot, and whose null blend state `142400090`
//!   selects the device's default (blending off, every channel written). [`Technique::Keep`]
//!   carries that; with a masking texture bound the binder reads through the null record
//!   (`+0x28`) and the game crashes.
//! - The programs of the game's own effect files are modelled here ([`Pixel`]), each keyed by the
//!   file it comes from: the effect's resolved `.shader` bytes and every include's, as sha256
//!   ([`EffectFacts`]). The same bytes under another path draw the same. Any other effect runs
//!   code this viewer does not run yet: it is refused as a custom effect, by name.

// The layers above this one resolve their effects through this module; until they are stacked on
// it, only the tests and the native GUI module use it.
#![cfg_attr(not(test), allow(dead_code))]

use installed_content::gui::native::shader;

use super::native_gui::lossy;

/// A sampler's addressing on one axis (`read_address@1423F7F20`: `wrap` 0, `mirror` 1, `clamp`
/// 2, `border` 3, `mirror_once` 4).
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

/// The sampler state a texture slot is read with, as the effect's `Samplers` row declares it and
/// `lower_d3d11@1423FD550` makes it: the border colour is always transparent black, `W` always
/// wraps, and `lower_filter@1424110F0` takes a `MipFilter` of `None` as point (only `Linear`
/// blends levels).
///
/// The sprite pass reads each texel itself ([`super::sprites`]), so every addressing mode, the
/// level of detail with its bias, and the choice between the magnifying and minifying filter are
/// the effect's.
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
    /// Blending off: `BlendEnable = no`, or the default state a null record selects
    /// (`142400090`).
    Off,
}

/// How a button-state program adds its shade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mix {
    None,
    /// `rgb += (0.5 + rgb) * 0.15 * (0.81 - (0.9 - saturate(dt * 4))^2)` (buttonstate's Over).
    TimedOver,
    /// `rgb -= (0.5 + rgb) * 0.15 * (0.81 - (0.9 - saturate(dt * 16))^2)` (buttonstate's Down).
    TimedDown,
    /// `rgba += (0.1, 0.1, 0.1, 0)` (`buttonstate_nodowneffect`'s Over).
    AddTenth,
    /// `rgba -= (0.1, 0.1, 0.1, 0)` (`buttonstate_linear`'s Down).
    SubtractTenth,
    /// `rgb += (0.5 + rgb) * 0.15` (`buttonstate_blendframes`'s Over).
    Over,
    /// `rgb -= (0.5 + rgb) * 0.15` (`buttonstate_blendframes`'s Down).
    Down,
}

/// Which texture coordinate a masking program reads its mask at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaskAt {
    /// `saturate(vTexCoord * 1000)` of the unshifted vertex coordinate (buttonstate's Up).
    Saturated,
    /// The frame's coordinate (buttonstate's other states read `v.vTexCoord`).
    Frame,
}

/// A sprite program of the button-state family, in its order: the texture (at the frame's
/// offset, or blended with the next frame's by `AnimationTime`), grey, the mask's alpha, the
/// fade, `Color`, the shade, `Color` again, and an opaque alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpriteProgram {
    /// `lerp(frame, next frame, AnimationTime)` (`buttonstate_blendframes`).
    pub blend_frames: bool,
    /// The effect's constant buffer has one `vXOffset` where the engine writes `Offset`: only the
    /// offset's `x` moves the texture (`buttonstate_nodowneffect`, `_linear`,
    /// `_nodownordisableeffect`, `static_button`).
    pub x_offset_only: bool,
    pub grey: bool,
    /// `MASKING` (`initialize_animations` adds the define when a `masking_texture` loads): the
    /// mask texture's alpha multiplies the alpha.
    pub mask: Option<MaskAt>,
    /// `buttonstate_fade_frames_to_black`: times `min(t / 0.05, 1) * min((1 - t) / 0.1, 1)`.
    pub fade_frames: bool,
    pub color_before: bool,
    pub mix: Mix,
    pub color_after: bool,
    pub opaque: bool,
}

impl SpriteProgram {
    /// The texture times `Color`.
    pub const PLAIN: Self = Self {
        blend_frames: false,
        x_offset_only: false,
        grey: false,
        mask: None,
        fade_frames: false,
        color_before: true,
        mix: Mix::None,
        color_after: false,
        opaque: false,
    };
}

/// Which progress program a progress bar's technique runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProgressProgram {
    /// `progress.shader`: the first where `u <= CurrentState`.
    Plain,
    /// `progress_reverse.shader`: the first where `u >= 1 - CurrentState`; its texture program
    /// composes the start, middle and stop thirds of the first texture.
    Reverse,
    /// `progress_startend.shader`: as plain, the texture program composing the thirds.
    StartEnd,
    /// `progress_minmax.shader`: from the middle outwards.
    MinMax,
    /// `progress_radial.shader`: its texture program by angle.
    Radial,
}

/// A program the sprite pass runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pixel {
    Sprite(SpriteProgram),
    /// A progress bar's `Color` (`texture: false`) or `Texture` technique.
    Progress {
        program: ProgressProgram,
        texture: bool,
        /// The vertex program negates `v` (every progress file but the radial one).
        flip_v: bool,
    },
    /// `text.shader`'s `PixelShaderText`: the glyph texel times the vertex colour times
    /// `ModColor`.
    Text,
    /// `maskedflag.shader`: the flag's colour at `FlagCoords.xy + uv * FlagCoords.zw` (slot 0),
    /// the mask's alpha at `uv` (slot 1), then `Over` adds a tenth, `Down` takes one, `Disable`
    /// greys.
    MaskedFlag(FlagMix),
    /// `circularprogressbar.shader`'s `CircularProgressBar`: at `t = (uv - 0.5) * 0.5 + 0.25`,
    /// `t.y = 1 - t.y`, slot 0 and slot 1 read at `t - 0.5`, blended by the triangle's sector
    /// (`vTextureToRender.x`: 0 the remaining sector, 1 the completed one).
    Circular,
}

/// How a masked flag's state changes its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlagMix {
    None,
    Over,
    Down,
    Grey,
}

impl Pixel {
    /// The program as the sprite pass's vertex attribute: the family in bits 0-3, then its
    /// fields.
    pub fn packed(self) -> u32 {
        match self {
            Self::Sprite(program) => {
                let mix = match program.mix {
                    Mix::None => 0,
                    Mix::TimedOver => 1,
                    Mix::TimedDown => 2,
                    Mix::AddTenth => 3,
                    Mix::SubtractTenth => 4,
                    Mix::Over => 5,
                    Mix::Down => 6,
                };
                let (masked, saturated) = match program.mask {
                    None => (0, 0),
                    Some(MaskAt::Frame) => (1, 0),
                    Some(MaskAt::Saturated) => (1, 1),
                };
                u32::from(program.blend_frames) << 4
                    | u32::from(program.x_offset_only) << 5
                    | u32::from(program.grey) << 6
                    | u32::from(program.color_before) << 7
                    | mix << 8
                    | u32::from(program.color_after) << 11
                    | u32::from(program.fade_frames) << 12
                    | u32::from(program.opaque) << 13
                    | masked << 14
                    | saturated << 15
            }
            Self::Progress {
                program,
                texture,
                flip_v,
            } => {
                let variant = match program {
                    ProgressProgram::Plain => 0,
                    ProgressProgram::Reverse => 1,
                    ProgressProgram::StartEnd => 2,
                    ProgressProgram::MinMax => 3,
                    ProgressProgram::Radial => 4,
                };
                1 | u32::from(texture) << 4 | u32::from(flip_v) << 5 | variant << 6
            }
            Self::Text => 2,
            Self::MaskedFlag(mix) => {
                let mix = match mix {
                    FlagMix::None => 0,
                    FlagMix::Over => 1,
                    FlagMix::Down => 2,
                    FlagMix::Grey => 3,
                };
                3 | mix << 4
            }
            Self::Circular => 4,
        }
    }
}

/// What a technique draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Technique {
    Run {
        pixel: Pixel,
        blend: Blend,
    },
    /// The file declares no technique of the name: the zero record's null program, the previous
    /// draw's program and samplers, and the default blend state.
    Keep,
}

/// A resolved effect: its file, its techniques and each texture slot's sampler.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    /// The effect as the sprite names it.
    pub file: String,
    pub techniques: Vec<(&'static str, Pixel, Blend)>,
    /// Each slot's sampler (`Index` 0 to 5), the last row of its index the effect declares;
    /// `None` where it declares none, which keeps the previous draw's (`1424005A0`).
    pub samplers: [Option<Sampler>; 6],
}

impl Effect {
    /// The technique `name` looks up.
    pub fn technique(&self, name: &str) -> Technique {
        self.techniques
            .iter()
            .find(|(own, _, _)| *own == name)
            .map_or(Technique::Keep, |(_, pixel, blend)| Technique::Run {
                pixel: *pixel,
                blend: *blend,
            })
    }

    /// How long after its drawable's restart the technique `name` draws differently on the
    /// clock alone: its program's answer ([`Pixel::settles`]), or, for a technique the file
    /// lacks, the answer of the program the previous draw bound ([`Settles::Previous`]).
    pub fn settles(&self, name: &str) -> Settles {
        match self.technique(name) {
            Technique::Run { pixel, .. } => Settles::Program(pixel.settles()),
            Technique::Keep => Settles::Previous,
        }
    }
}

/// How long after its drawable's restart (`AnimationTime`, `CSprite+0x188`) a draw keeps
/// changing on the clock alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Settles {
    /// Its own program's answer: seconds after the restart, `None` for as long as it draws.
    Program(Option<f32>),
    /// Its effect lacks the technique: `LookupShaderEffectHandle@1423F5D60` gives the zeroed
    /// record at `1435C1970`, whose null program `shader_program_binder@142400520` does not bind
    /// (it returns at once for a null argument), so the draw runs the program the previous draw
    /// bound, with this draw's own constants (`CSpriteType::draw@14222D5A0` uploads its `Time`
    /// and `AnimationTime` whatever the program): that program's answer, from this drawable's
    /// restart. Which program that is depends on the draw order, which the caller resolves.
    Previous,
}

impl Pixel {
    /// Seconds after the restart (`Time - AnimationTime`, `CSprite::draw@1422E95C0`) past which
    /// this program draws the same on the clock alone. The timed shades saturate their time:
    /// `Over` `saturate(dt * 4)` at a quarter second, `Down` `saturate(dt * 16)` at a sixteenth;
    /// no other modelled program reads the clock (a frame blend reads its frame clock's
    /// `AnimationTime`, which the frame clock schedules).
    pub fn settles(self) -> Option<f32> {
        Some(match self {
            Pixel::Sprite(program) => match program.mix {
                Mix::TimedOver => 1.0 / 4.0,
                Mix::TimedDown => 1.0 / 16.0,
                _ => 0.0,
            },
            _ => 0.0,
        })
    }
}

/// The facts of an effect file as the content stack resolves it: its sha256 and its includes'
/// (`None` for an include no layer holds, which reads empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectFacts {
    pub file: String,
    pub identity: [u8; 32],
    pub includes: Vec<(String, Option<[u8; 32]>)>,
}

/// A 64-digit hexadecimal sha256.
const fn digest(hex: &str) -> [u8; 32] {
    let bytes = hex.as_bytes();
    let mut out = [0u8; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = nibble(bytes[2 * index]) << 4 | nibble(bytes[2 * index + 1]);
        index += 1;
    }
    out
}

/// One lowercase hexadecimal digit's value.
const fn nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        _ => byte - b'a' + 10,
    }
}

const BUTTONSTATE_FXH: [u8; 32] =
    digest("8a0e70eb222f1d71b1660fad3c19625859e95e72959995b3f26d478a8b95dd93");
const SPRITE_ANIMATION_FXH: [u8; 32] =
    digest("b32e1007b2e41df9ad3c41f49c6df32b7f7fce58bf0f86f55207505a1382238f");
const FADE_FRAMES_FXH: [u8; 32] =
    digest("c42254a202b0e0160fd74ba4d1564ae45e8b502dc2dfb838d953e038be4a6e70");

/// One modelled file: its identity, its includes' identities in the order they are read, and
/// its techniques.
struct Modelled {
    identity: [u8; 32],
    includes: &'static [[u8; 32]],
    techniques: fn(masked: bool) -> Vec<(&'static str, Pixel, Blend)>,
}

/// The four button states of a program family.
fn states(
    up: SpriteProgram,
    down: SpriteProgram,
    disable: SpriteProgram,
    over: SpriteProgram,
    blend: Blend,
) -> Vec<(&'static str, Pixel, Blend)> {
    vec![
        ("Up", Pixel::Sprite(up), blend),
        ("Down", Pixel::Sprite(down), blend),
        ("Disable", Pixel::Sprite(disable), blend),
        ("Over", Pixel::Sprite(over), blend),
    ]
}

/// `buttonstate.shader`: Up the texture times `Color`; Down and Over add their timed shade after
/// `Color`; Disable greys first. Under `MASKING` Up reads the mask at the saturated coordinate,
/// the others at the frame's.
fn buttonstate(masked: bool) -> Vec<(&'static str, Pixel, Blend)> {
    let mask = |at| masked.then_some(at);
    let up = SpriteProgram {
        mask: mask(MaskAt::Saturated),
        ..SpriteProgram::PLAIN
    };
    let other = SpriteProgram {
        mask: mask(MaskAt::Frame),
        ..SpriteProgram::PLAIN
    };
    states(
        up,
        SpriteProgram {
            mix: Mix::TimedDown,
            ..other
        },
        SpriteProgram {
            grey: true,
            ..other
        },
        SpriteProgram {
            mix: Mix::TimedOver,
            ..other
        },
        Blend::Alpha,
    )
}

/// The three files whose constant buffer moves only `x`: Down plain, Over (and `linear`'s Down)
/// a tenth added before `Color`.
fn x_offset(down: Mix, disable_grey: bool) -> Vec<(&'static str, Pixel, Blend)> {
    let plain = SpriteProgram {
        x_offset_only: true,
        ..SpriteProgram::PLAIN
    };
    let mixed = |mix| SpriteProgram {
        color_before: false,
        mix,
        color_after: true,
        ..plain
    };
    states(
        plain,
        if down == Mix::None {
            plain
        } else {
            mixed(down)
        },
        SpriteProgram {
            grey: disable_grey,
            ..plain
        },
        mixed(Mix::AddTenth),
        Blend::Alpha,
    )
}

/// The game's own effect files whose programs this module runs.
const MODELLED: &[Modelled] = &[
    Modelled {
        identity: digest("1eddb371702289f4cc9d9d565aef21cf77c9261bd4324194295eeea6309f413a"),
        includes: &[BUTTONSTATE_FXH, SPRITE_ANIMATION_FXH],
        techniques: buttonstate,
    },
    // `buttonstate_nodowneffect.shader`: Down as Up.
    Modelled {
        identity: digest("e44bcaaaf2542bf7ab1c23d9670be72cfcf5cd3d51d3197f8f4836035a74ace3"),
        includes: &[],
        techniques: |_| x_offset(Mix::None, true),
    },
    // `buttonstate_linear.shader`: Down a tenth darker before `Color`.
    Modelled {
        identity: digest("f03886292a9f9a4d2b803c4a6e31680740dd6f1600e918ff1716ab1c7dddbe87"),
        includes: &[],
        techniques: |_| x_offset(Mix::SubtractTenth, true),
    },
    // `buttonstate_nodownordisableeffect.shader`: Up, Down and Disable all `PixelShaderDefault`.
    Modelled {
        identity: digest("641f9245ce6eda5135a9c84ee7723e1e3d268b5c16d2b0bd14e5e85cca432075"),
        includes: &[],
        techniques: |_| x_offset(Mix::None, false),
    },
    // `buttonstate_onlydisable.shader`: Up, Down and Over the texture times `Color`.
    Modelled {
        identity: digest("8c43262388847cc5b16d44da800f0d2d1eb8b900c126dd78c7cc57c5ab483002"),
        includes: &[BUTTONSTATE_FXH, SPRITE_ANIMATION_FXH],
        techniques: |_| {
            let up = SpriteProgram::PLAIN;
            states(up, up, SpriteProgram { grey: true, ..up }, up, Blend::Alpha)
        },
    },
    // `buttonstate_nontransparent.shader`: opaque, blending off; Down and Over shade without
    // `Color`.
    Modelled {
        identity: digest("5393a16f17f5c47039436adc9ea051e78446c094ab70e602167245b2aeb53b0e"),
        includes: &[BUTTONSTATE_FXH],
        techniques: |_| {
            let up = SpriteProgram {
                opaque: true,
                ..SpriteProgram::PLAIN
            };
            let bare = SpriteProgram {
                color_before: false,
                ..up
            };
            states(
                up,
                SpriteProgram {
                    mix: Mix::TimedDown,
                    ..bare
                },
                SpriteProgram { grey: true, ..up },
                SpriteProgram {
                    mix: Mix::TimedOver,
                    ..bare
                },
                Blend::Off,
            )
        },
    },
    // `buttonstate_blendframes.shader`: the frame blended into the next by `AnimationTime`;
    // Down and Over shade by a constant, without `Color`.
    Modelled {
        identity: digest("0135e91818d1afa17c50a6e88160d68a0a5edda76561d0de2a76f6e5e53bc3d5"),
        includes: &[BUTTONSTATE_FXH],
        techniques: |_| {
            let up = SpriteProgram {
                blend_frames: true,
                ..SpriteProgram::PLAIN
            };
            let bare = SpriteProgram {
                color_before: false,
                ..up
            };
            states(
                up,
                SpriteProgram {
                    mix: Mix::Down,
                    ..bare
                },
                SpriteProgram { grey: true, ..up },
                SpriteProgram {
                    mix: Mix::Over,
                    ..bare
                },
                Blend::Alpha,
            )
        },
    },
    // `buttonstate_fade_frames_to_black.shader`: faded in and out over each frame's time.
    Modelled {
        identity: digest("0ab08d152913313609078ad85714d19c871e3a95049e2c924d6bd4f187773b8e"),
        includes: &[BUTTONSTATE_FXH, FADE_FRAMES_FXH],
        techniques: |_| {
            let up = SpriteProgram {
                fade_frames: true,
                ..SpriteProgram::PLAIN
            };
            let bare = SpriteProgram {
                color_before: false,
                ..up
            };
            states(
                up,
                SpriteProgram {
                    mix: Mix::Down,
                    ..bare
                },
                SpriteProgram { grey: true, ..up },
                SpriteProgram {
                    mix: Mix::Over,
                    ..bare
                },
                Blend::Alpha,
            )
        },
    },
    // `buttonstate_rendertarget.shader`: every state the texture times `Color` (its `PDX_OPENGL`
    // flip is not compiled on Direct3D).
    Modelled {
        identity: digest("60c5c17cc1abe1ea2bdefc963843a8ad33c3a086be3945f6edc98502edba27bb"),
        includes: &[BUTTONSTATE_FXH],
        techniques: |_| {
            let up = SpriteProgram::PLAIN;
            states(up, up, up, up, Blend::Alpha)
        },
    },
    // `static_button.shader`: every state the texture times `Color`, `x` moved only.
    Modelled {
        identity: digest("fe3c84ec351378044aca0e09cdddbc241ddab29e15b65cd21bc76b1d4e91ce00"),
        includes: &[],
        techniques: |_| {
            let up = SpriteProgram {
                x_offset_only: true,
                ..SpriteProgram::PLAIN
            };
            states(up, up, up, up, Blend::Alpha)
        },
    },
    Modelled {
        identity: digest("75cbb6f15ebdd503e2b4bc8ce03a17ad85c05f0fe127b86f3fb065546071f525"),
        includes: &[],
        techniques: |_| progress(ProgressProgram::Plain, true),
    },
    Modelled {
        identity: digest("4d0e66eef57ad8b95ee9aa94af1774e73177bbc47b9c74fc6962282e5da17bc4"),
        includes: &[],
        techniques: |_| progress(ProgressProgram::Reverse, true),
    },
    Modelled {
        identity: digest("b0bab8b288a0d5c664c5b43ed0db50fb3d65ace2d23e51aad1db3007da633d65"),
        includes: &[],
        techniques: |_| progress(ProgressProgram::StartEnd, true),
    },
    Modelled {
        identity: digest("3ebc55f4cf2feec6232bedb30b7415f3e29075f02cfb1a967c0c286b6e61855d"),
        includes: &[],
        techniques: |_| progress(ProgressProgram::MinMax, true),
    },
    Modelled {
        identity: digest("450982677dfbde87fcb775aa369401d7a7541eb317c953886be6e857e9519374"),
        includes: &[],
        techniques: |_| progress(ProgressProgram::Radial, false),
    },
    Modelled {
        identity: digest("cc64dd5cff248bee7dd12ed34451df584e7a4929518b57edcfe20f2c2707a3f8"),
        includes: &[],
        techniques: |_| masked_flag(),
    },
    // `circularprogressbar.shader` (no includes), the path `C2dCircularProgressBarType::vf9
    // @1423421E0` names itself.
    Modelled {
        identity: digest("f1e36411138bb55faeb41612ad6280da8131bb24b70b4e882090002b5600b0c7"),
        includes: &[],
        techniques: |_| vec![("CircularProgressBar", Pixel::Circular, Blend::Alpha)],
    },
    // `text.shader`: `Text` and `Text3D` run the same pixel program.
    Modelled {
        identity: digest("ae3b1e284881127d9f069d3af383e445bdd61ca40c00bf3295e74f2d87ab149e"),
        includes: &[],
        techniques: |_| {
            vec![
                ("Text", Pixel::Text, Blend::Alpha),
                ("Text3D", Pixel::Text, Blend::Alpha),
            ]
        },
    },
];

/// `maskedflag.shader`'s four states.
fn masked_flag() -> Vec<(&'static str, Pixel, Blend)> {
    vec![
        ("Up", Pixel::MaskedFlag(FlagMix::None), Blend::Alpha),
        ("Down", Pixel::MaskedFlag(FlagMix::Down), Blend::Alpha),
        ("Disable", Pixel::MaskedFlag(FlagMix::Grey), Blend::Alpha),
        ("Over", Pixel::MaskedFlag(FlagMix::Over), Blend::Alpha),
    ]
}

/// A progress file's `Color` and `Texture` techniques.
fn progress(program: ProgressProgram, flip_v: bool) -> Vec<(&'static str, Pixel, Blend)> {
    let pixel = |texture| Pixel::Progress {
        program,
        texture,
        flip_v,
    };
    vec![
        ("Color", pixel(false), Blend::Alpha),
        ("Texture", pixel(true), Blend::Alpha),
    ]
}

/// The modelled effect `facts` names, with `samplers` its slots' rows; a file this module does
/// not run is refused as a custom effect. `masked` is whether the sprite's `masking_texture`
/// loads (the `MASKING` define).
pub fn resolve(
    facts: &EffectFacts,
    samplers: [Option<Sampler>; 6],
    masked: bool,
) -> Result<Effect, String> {
    let modelled = MODELLED.iter().find(|modelled| {
        modelled.identity == facts.identity
            && modelled.includes.len() == facts.includes.len()
            && modelled
                .includes
                .iter()
                .zip(&facts.includes)
                .all(|(own, (_, read))| Some(*own) == *read)
    });
    let Some(modelled) = modelled else {
        return Err(format!(
            "custom effect {}: its programs are not the game's own files, which this viewer runs \
             (political/l5s-shader-pipeline runs a mod's effect code)",
            facts.file
        ));
    };
    Ok(Effect {
        file: facts.file.clone(),
        techniques: (modelled.techniques)(masked),
        samplers,
    })
}

/// A sampler row as `lower_d3d11@1423FD550` lowers it: `None` filters as point
/// (`lower_filter@1424110F0`); an anisotropic filter is D3D11's 0x55, which the sprite pass does
/// not run.
fn lowered(row: &shader::SamplerRow, file: &str) -> Result<Sampler, String> {
    let address = |address: shader::Address| match address {
        shader::Address::Wrap => Address::Wrap,
        shader::Address::Mirror => Address::Mirror,
        shader::Address::Clamp => Address::Clamp,
        shader::Address::Border => Address::Border,
        shader::Address::MirrorOnce => Address::MirrorOnce,
    };
    let filter = |filter: shader::Filter| match filter {
        shader::Filter::None | shader::Filter::Point => Ok(Filter::Point),
        shader::Filter::Linear => Ok(Filter::Linear),
        shader::Filter::Anisotropic => Err(format!(
            "{file} samples slot {} anisotropically (lower_d3d11@1423FD550's filter 0x55), which \
             the sprite pass does not run",
            row.index
        )),
    };
    Ok(Sampler {
        u: address(row.address_u),
        v: address(row.address_v),
        mag: filter(row.mag)?,
        min: filter(row.min)?,
        mip: filter(row.mip)?,
        lod_bias: row.lod_bias,
    })
}

/// The effect a sprite's `effect` names, as `engine_shader_effect_lookup@1423F4EC0` finds it
/// through the content stack ([`shader::load`]): a file no layer holds is the zero record
/// `1435C1970` (every technique [`Technique::Keep`]); a file this module models draws with its
/// programs and the sampler slots the file declares (`populate@142345450`, slots 0 to 5, which
/// the GUI programs read); any other is refused as a custom effect.
pub fn load(
    layers: &installed_content::ResolvedLayers,
    effect: &[u8],
    masked: bool,
) -> Result<Effect, String> {
    // Read by the name's bytes; the name as given, to its C string's end, labels it.
    let loaded = shader::load(layers, effect);
    let end = effect
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(effect.len());
    from_file(&lossy(&effect[..end]), loaded.as_ref(), masked)
}

/// [`load`] of a file already read (`None` when no layer holds it).
pub fn from_file(
    name: &str,
    loaded: Option<&shader::Effect>,
    masked: bool,
) -> Result<Effect, String> {
    let Some(loaded) = loaded else {
        return Ok(Effect {
            file: name.to_owned(),
            techniques: Vec::new(),
            samplers: [None; 6],
        });
    };
    let facts = EffectFacts {
        file: loaded.path.clone(),
        identity: loaded.identity,
        includes: loaded.includes.clone(),
    };
    let mut effect = resolve(&facts, [None; 6], masked)?;
    effect.file = name.to_owned();
    for (index, row) in loaded.sampler_slots()?.into_iter().enumerate().take(6) {
        if let Some(row) = row {
            effect.samplers[index] = Some(lowered(row, &loaded.path)?);
        }
    }
    Ok(effect)
}

/// The game's own effects as the shipped files declare them, for other modules' tests.
#[cfg(test)]
pub mod shipped {
    use super::*;

    const fn sampler(
        u: Address,
        v: Address,
        filter: Filter,
        mip: Filter,
        lod_bias: f32,
    ) -> Sampler {
        Sampler {
            u,
            v,
            mag: filter,
            min: filter,
            mip,
            lod_bias,
        }
    }

    /// `gfx/FX/buttonstate.lua`: `MapTexture` (Index 0) linear and clamped with a -0.8 bias,
    /// the masking texture (Index 5) point and clamped.
    pub fn buttonstate() -> Effect {
        let clamp = sampler(
            Address::Clamp,
            Address::Clamp,
            Filter::Linear,
            Filter::Point,
            0.0,
        );
        let wrap = sampler(
            Address::Wrap,
            Address::Wrap,
            Filter::Linear,
            Filter::Point,
            0.0,
        );
        Effect {
            file: "gfx/FX/buttonstate.lua".into(),
            techniques: super::buttonstate(false),
            samplers: [
                Some(Sampler {
                    lod_bias: -0.8,
                    ..clamp
                }),
                Some(clamp),
                Some(wrap),
                Some(clamp),
                Some(wrap),
                Some(sampler(
                    Address::Clamp,
                    Address::Clamp,
                    Filter::Point,
                    Filter::Point,
                    0.0,
                )),
            ],
        }
    }

    /// `gfx/FX/progress.lua`: both textures point-sampled and wrapped.
    pub fn progress() -> Effect {
        let wrap = sampler(
            Address::Wrap,
            Address::Wrap,
            Filter::Point,
            Filter::Point,
            0.0,
        );
        Effect {
            file: "gfx/FX/progress.lua".into(),
            techniques: super::progress(ProgressProgram::Plain, true),
            samplers: [Some(wrap), Some(wrap), None, None, None, None],
        }
    }

    /// `gfx/FX/text.lua`: `SimpleTexture` linear, linear mips, wrapped.
    /// `gfx/FX/circularprogressbar.shader`: `TextureOne` (Index 0) and `TextureTwo` (Index 1)
    /// linear and wrapped, no mip filter.
    pub fn circular() -> Effect {
        let wrap = sampler(
            Address::Wrap,
            Address::Wrap,
            Filter::Linear,
            Filter::Point,
            0.0,
        );
        Effect {
            file: "gfx/FX/circularprogressbar.shader".into(),
            techniques: vec![("CircularProgressBar", Pixel::Circular, Blend::Alpha)],
            samplers: [Some(wrap), Some(wrap), None, None, None, None],
        }
    }

    pub fn text() -> Effect {
        Effect {
            file: "gfx/FX/text.lua".into(),
            techniques: vec![("Text", Pixel::Text, Blend::Alpha)],
            samplers: [
                Some(sampler(
                    Address::Wrap,
                    Address::Wrap,
                    Filter::Linear,
                    Filter::Linear,
                    0.0,
                )),
                None,
                None,
                None,
                None,
                None,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(identity: [u8; 32], includes: &[[u8; 32]]) -> EffectFacts {
        EffectFacts {
            file: "gfx/FX/mod.lua".into(),
            identity,
            includes: includes
                .iter()
                .map(|include| ("gfx/FX/x.fxh".to_owned(), Some(*include)))
                .collect(),
        }
    }

    /// The game's buttonstate file under any name runs its programs; a changed include or file
    /// is a custom effect, refused by name.
    #[test]
    fn a_vanilla_file_is_known_by_its_bytes() {
        let identity = MODELLED[0].identity;
        let effect = resolve(
            &facts(identity, &[BUTTONSTATE_FXH, SPRITE_ANIMATION_FXH]),
            [None; 6],
            false,
        )
        .unwrap();
        assert_eq!(
            effect.technique("Over"),
            Technique::Run {
                pixel: Pixel::Sprite(SpriteProgram {
                    mix: Mix::TimedOver,
                    ..SpriteProgram::PLAIN
                }),
                blend: Blend::Alpha
            }
        );
        // A name the file does not declare is the null record.
        assert_eq!(effect.technique("Texture"), Technique::Keep);
        let error = resolve(&facts(identity, &[BUTTONSTATE_FXH]), [None; 6], false).unwrap_err();
        assert!(error.starts_with("custom effect gfx/FX/mod.lua"), "{error}");
        assert!(resolve(&facts([0; 32], &[]), [None; 6], false).is_err());
    }

    /// `MASKING` reads the mask at the saturated coordinate in Up and at the frame's elsewhere.
    #[test]
    fn masking_reads_where_each_state_reads() {
        let masked = buttonstate(true);
        assert!(matches!(
            masked[0].1,
            Pixel::Sprite(SpriteProgram {
                mask: Some(MaskAt::Saturated),
                ..
            })
        ));
        assert!(matches!(
            masked[1].1,
            Pixel::Sprite(SpriteProgram {
                mask: Some(MaskAt::Frame),
                ..
            })
        ));
    }

    /// Every packed program is distinct, so the pass runs each as modelled.
    #[test]
    fn packed_programs_are_distinct() {
        let mut seen = std::collections::HashMap::new();
        for modelled in MODELLED {
            for masked in [false, true] {
                for (_, pixel, _) in (modelled.techniques)(masked) {
                    let packed = pixel.packed();
                    if let Some(other) = seen.insert(packed, pixel) {
                        assert_eq!(other, pixel, "{packed:#x}");
                    }
                }
            }
        }
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
