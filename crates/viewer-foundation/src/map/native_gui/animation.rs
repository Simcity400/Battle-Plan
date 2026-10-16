//! Eagerly loaded sprite animation records.
//!
//! The instance clock gates each record. The first two active records bind mask and
//! animated textures to slots one through four and upload their constants, selecting
//! the corresponding animated technique. [`Animated`] prepares CPU gating, textures
//! and constants; the shader pipeline runs the installed effect program.

use super::*;

/// One record as loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationRecord {
    pub data: installed_content::gui::native::sprites::AnimationData,
    /// `animationmaskfile`'s texture, or the null texture when it does not load.
    pub mask: usize,
    /// `animationtexturefile`'s texture and its size.
    pub texture: usize,
    pub texture_size: [i32; 2],
}

/// An active record's draw inputs: its textures for slots `1 + 2k` and `2 + 2k`, and its five
/// `float4` of `b1` in the game's order: `Rotation` (`cos, -sin, sin, cos`), `UVScale`,
/// (`RotationOffset`, the stored texture scale), (`AnimationOffset`, `BlendFactor`, `Type`,
/// the blend mode), (`ClampAnimation`, 0, 0, 0).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationDraw {
    pub mask: usize,
    pub texture: usize,
    pub constants: [[f32; 4]; 5],
    /// Whether the game writes `AnimationOffset` and `Type` (floats 12 and 14): only for
    /// animation types 1 to 4; any other type leaves them as the draw found them.
    pub typed: bool,
}

impl AnimationDraw {
    /// The bytes of its 80-byte record the game writes: the first three `float4`s,
    /// `BlendFactor` (52..56), the blend mode and `ClampAnimation`'s first float (60..68), and
    /// for a typed record `AnimationOffset` (48..52) and `Type` (56..60). Bytes 68..80 are
    /// never written.
    pub fn written(&self) -> Vec<std::ops::Range<u32>> {
        if self.typed {
            vec![0..48, 48..68]
        } else {
            vec![0..48, 52..56, 60..68]
        }
    }
}

/// A sprite drawn with its active records: the quad as [`PartKind::Sprite`] draws it (its
/// masking texture still bound to slot 5, though the `ANIMATED` programs are compiled without
/// `MASKING`), the effect
/// file to compile with `ANIMATED` and `NUM_ANIMATIONS_n` (`n` the records' count), the state
/// technique, and each record's inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Animated {
    pub sprite: Vec<u8>,
    pub geometry: Geometry,
    pub uv: [f32; 4],
    pub offsets: [f32; 4],
    pub effect: String,
    pub technique: &'static str,
    pub records: Vec<AnimationDraw>,
    /// Whether the selected sprite declares a masking texture.
    pub masked: bool,
}

/// `sat`: 0 below 0, else `MINSS(1, x)` (a NaN passes).
fn saturate(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Gate an animation record using elapsed instance time, retained state,
/// texture dimensions and blend factor. Unavailable reached state leaves
/// the record undecided.
pub fn record_draw(
    record: &AnimationRecord,
    t: f32,
    start: f32,
    state: u32,
    size: [i32; 2],
) -> Option<AnimationDraw> {
    let data = &record.data;
    let bit = 1u32.wrapping_shl(state & 31);
    // A trigger record runs from its start; one not started is inactive.
    let t = if data.trigger_frames != 0 {
        if start < 0.0 {
            return None;
        }
        t - start
    } else {
        t
    };
    if data.frames != u32::MAX && bit & data.frames == 0 {
        return None;
    }
    let time = data.time;
    // A NaN passes (`!(t > T)`).
    if !data.looping && t > time {
        return None;
    }
    let [w, h] = size.map(|value| value as f32);
    let [aw, ah] = record.texture_size.map(|value| value as f32);
    let scale = [
        if w >= h { 1.0 } else { w / h },
        if w >= h { h / w } else { 1.0 },
        w / aw,
        h / ah,
    ];
    let ratio = w.min(h) / w.max(h);
    let wrap = |t: f32| {
        if data.looping {
            t % (data.delay + time)
        } else {
            t
        }
    };
    // `0x40490FDB`.
    let turn = 2.0 * std::f32::consts::PI;
    let (kind, offset, angle) = match data.kind {
        1 => {
            // `0x3FB50481`.
            let k = (h / ah) * f32::from_bits(0x3FB5_0481);
            let s = 1.0 / ratio;
            let t = if data.looping && data.delay > 0.0 {
                t % (data.delay + time)
            } else {
                t
            };
            (
                1.0,
                t * ((k * s + 1.0) / time) - (s + 1.0) * 0.5 * k,
                data.rotation,
            )
        }
        2 | 3 => {
            let a = turn * saturate(wrap(t) / time);
            (2.0, 0.0, if data.kind == 2 { -a } else { a })
        }
        4 => {
            let pulse = 0.5 - 0.5 * (turn * saturate(wrap(t) / time)).cos();
            (3.0, pulse, data.rotation)
        }
        _ => (0.0, 0.0, data.rotation),
    };
    let (sin, cos) = angle.sin_cos();
    Some(AnimationDraw {
        mask: record.mask,
        texture: record.texture,
        typed: (1..=4).contains(&data.kind),
        constants: [
            [cos, -sin, sin, cos],
            scale,
            [
                data.rotation_offset[0],
                data.rotation_offset[1],
                data.texture_scale[0],
                data.texture_scale[1],
            ],
            [offset, 1.0, kind, data.blend_mode as f32],
            [
                if !data.looping || data.delay > 0.0 {
                    1.0
                } else {
                    0.0
                },
                0.0,
                0.0,
                0.0,
            ],
        ],
    })
}

impl Walk<'_> {
    /// The active records of the eagerly loaded sprite `sprite` for `element` in `state`, on the
    /// instance's clock (its first draw and each trigger record's start): at most the first two
    /// (`select_effect_program` stops at two). Without an instance the state is taken as never
    /// changed since the first draw (a trigger record starts then when its trigger holds the
    /// state, `inst.last` starting at 0). A clock no instance keeps, the records a caller has
    /// not loaded, and a trigger record under a state the pointer changes (whose start no
    /// instance records) are refused.
    pub(super) fn animation_records(
        &self,
        element: &Element,
        sprite: &[u8],
        info: &SpriteInfo,
        state: State,
    ) -> Result<Vec<AnimationDraw>, Refusal> {
        let path = &element.path;
        let records = self.resources.sprite_animations(sprite).ok_or_else(|| {
            Refusal::new(
                path,
                RefusalKind::MissingResource,
                format!(
                    "shows {}, whose animation records the caller has not loaded",
                    lossy(sprite)
                ),
            )
        })?;
        let time = self
            .bound(element)
            .and_then(|bound| bound.animation_time)
            .ok_or_else(|| {
                Refusal::new(
                    path,
                    RefusalKind::MissingInput,
                    format!(
                        "shows {}, whose animation records run on the instance's clock from its first draw: lay it out through its lifecycle Instance",
                        lossy(sprite)
                    ),
                )
            })?;
        if self.bindings.pointer.is_some()
            && matches!(state, State::Over(_) | State::Down(_))
            && records.iter().any(|record| record.data.trigger_frames != 0)
        {
            return Err(Refusal::new(
                path,
                RefusalKind::MissingInput,
                format!(
                    "shows {}, whose trigger records restart when the state changes, and the pointer changed it at a time the bindings do not hold",
                    lossy(sprite)
                ),
            ));
        }
        let kept = self
            .bound(element)
            .and_then(|bound| bound.triggers.as_ref());
        let bit = 1u32.wrapping_shl(state.number() & 31);
        Ok(records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                let start = match kept {
                    Some(starts) => starts.get(index).copied().unwrap_or(-1.0),
                    None if bit & record.data.trigger_frames != 0 => 0.0,
                    None => -1.0,
                };
                record_draw(record, time, start, state.number(), info.texture)
            })
            .take(2)
            .collect())
    }
}
