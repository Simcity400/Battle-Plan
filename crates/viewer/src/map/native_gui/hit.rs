//! Pointer geometry for retained GUI objects.
//! Hit extents and transforms are independent of stretched or cropped draw quads.

use super::{Clip, Hit, Holder, Rect, truncate_f32};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HitGeometry {
    pub form: Hit,
    position: [i32; 2],
    viewport: [i32; 2],
    // Absolute logical positions, outer object first. Epsilon gates and f32
    // rounding make a composed affine differ from the native per-node walk.
    parents: Vec<(f32, [i32; 2])>,
    // Only synthetic holders use this additional initial translation.
    initial: [f32; 2],
    shape: Shape,
    clip: Option<Clip>,
}

#[derive(Clone, Debug, PartialEq)]
enum Shape {
    Sprite {
        size: [i32; 2],
        frame: [f32; 2],
        physical: [i32; 2],
    },
    Text {
        local: [i32; 2],
        size: [i32; 2],
        measured_height: i32,
    },
    Rectangle([i32; 2]),
}

impl HitGeometry {
    pub(super) fn sprite(
        position: [i32; 2],
        size: [i32; 2],
        frame: [f32; 2],
        form: Hit,
        viewport: [i32; 2],
        physical: [i32; 2],
        clip: Option<Clip>,
    ) -> Self {
        Self {
            form,
            position,
            viewport,
            parents: Vec::new(),
            initial: [0.0; 2],
            shape: Shape::Sprite {
                size,
                frame,
                physical,
            },
            clip,
        }
    }

    pub(super) fn text(
        graphics: [i32; 2],
        local: [i32; 2],
        size: [i32; 2],
        measured_height: i32,
        viewport: [i32; 2],
        clip: Option<Clip>,
    ) -> Self {
        Self {
            form: Hit::Frame,
            position: graphics,
            viewport,
            parents: Vec::new(),
            initial: [0.0; 2],
            shape: Shape::Text {
                local,
                size,
                measured_height,
            },
            clip,
        }
    }

    pub(super) fn rectangle(
        position: [i32; 2],
        size: [i32; 2],
        viewport: [i32; 2],
        clip: Option<Clip>,
    ) -> Self {
        Self {
            form: Hit::Frame,
            position,
            viewport,
            parents: Vec::new(),
            initial: [0.0; 2],
            shape: Shape::Rectangle(size),
            clip,
        }
    }

    pub(super) fn inherit(&mut self, scale: f32, position: [i32; 2]) {
        self.parents.insert(0, (scale, position));
    }

    pub(super) fn placed(&self, holder: &Holder, place: [i32; 2], clip: Option<Clip>) -> Self {
        let mut hit = self.clone();
        hit.translate([0, 1].map(|axis| holder.logical[axis].wrapping_add(place[axis])));
        if let Some(parents) = &holder.hit_scales {
            let mut outer = parents.clone();
            outer.append(&mut hit.parents);
            hit.parents = outer;
        } else {
            hit.parents.insert(0, (holder.scale, holder.logical));
            hit.initial = [0, 1]
                .map(|axis| hit.initial[axis] + holder.origin[axis] - holder.logical[axis] as f32);
        }
        hit.clip = clip;
        hit
    }

    fn translate(&mut self, offset: [i32; 2]) {
        self.position = [0, 1].map(|axis| self.position[axis].wrapping_add(offset[axis]));
        for (_, position) in &mut self.parents {
            *position = [0, 1].map(|axis| position[axis].wrapping_add(offset[axis]));
        }
    }

    pub(crate) fn shifted(&self, origin: [i32; 2]) -> Self {
        let mut hit = self.clone();
        hit.translate(origin);
        hit.clip = hit.clip.map(|clip| {
            [
                clip[0].wrapping_add(origin[0]),
                clip[1].wrapping_add(origin[1]),
                clip[2].wrapping_add(origin[0]),
                clip[3].wrapping_add(origin[1]),
            ]
        });
        hit
    }

    // Construction adds a truncated double-precision half-viewport before
    // subtracting the float half. Those terms do not always cancel in f32.
    fn wrapper_local(&self, position: [i32; 2]) -> [f32; 2] {
        let half = self.viewport.map(|v| v as f32 * 0.5);
        let at = [0, 1].map(|axis| {
            position[axis].wrapping_add((self.viewport[axis] as f64 * 0.5) as i32) as f32
        });
        [at[0] + -(self.viewport[0] as f32) * 0.5, half[1] - at[1]]
    }

    fn world(&self) -> ([f32; 2], [f32; 2], f32) {
        let mut logical = [0; 2];
        let mut translation = [self.initial[0], -self.initial[1]];
        let mut scale = 1.0f32;
        for &(own, at) in &self.parents {
            let relative = [0, 1].map(|axis| at[axis].wrapping_sub(logical[axis]));
            advance(
                &mut translation,
                &mut scale,
                self.wrapper_local(relative),
                own,
                None,
            );
            logical = at;
        }
        let relative = [0, 1].map(|axis| self.position[axis].wrapping_sub(logical[axis]));
        let (world, pivot) = match self.shape {
            Shape::Sprite { size, physical, .. } => {
                let half = physical.map(|v| v as f32 * 0.5);
                // Own scale is already reflected in the queried size and frame. A shown
                // object uses its retained hit geometry.
                let local = [
                    relative[0] as f32 + -(physical[0] as f32) * 0.5,
                    (half[1] - size[1] as f32) - relative[1] as f32,
                ];
                advance(&mut translation, &mut scale, local, 1.0, Some(half));
                let world = [
                    half[0] + translation[0],
                    -((translation[1] - half[1]) + size[1] as f32),
                ];
                (world, [world[0], world[1] + size[1] as f32])
            }
            Shape::Text {
                local,
                measured_height,
                ..
            } => {
                // Add the retained layout position after traversal; graphical local
                // translation begins at zero.
                advance(
                    &mut translation,
                    &mut scale,
                    [relative[0] as f32, -(relative[1] as f32)],
                    1.0,
                    None,
                );
                let world = [
                    local[0] as f32 + translation[0],
                    local[1] as f32 - translation[1],
                ];
                (world, [world[0], world[1] + measured_height as f32])
            }
            Shape::Rectangle(size) => {
                advance(
                    &mut translation,
                    &mut scale,
                    self.wrapper_local(relative),
                    1.0,
                    None,
                );
                let half = self.viewport.map(|v| v as f32 * 0.5);
                // A zero size query falls back to the authored extent before applying
                // the size-dependent world position.
                let pivot = [
                    half[0] + translation[0],
                    -((translation[1] - half[1]) + size[1] as f32),
                ];
                ([translation[0], -translation[1]], pivot)
            }
        };
        (world, pivot, scale)
    }

    /// Conservative diagnostic footprint; dispatch must use contains.
    pub(crate) fn bounds(&self) -> Option<Rect> {
        let (world, pivot, inherited) = self.world();
        let at = world.map(truncate_f32);
        let size = match self.shape {
            Shape::Sprite { frame, .. } => frame,
            Shape::Text { size, .. } | Shape::Rectangle(size) => size.map(|v| v as f32),
        };
        let scale = if scaled(inherited) { inherited } else { 1.0 };
        let low = [0, 1].map(|axis| ((at[axis] as f32 - 1.0) - pivot[axis]) * scale + pivot[axis]);
        let high = [0, 1]
            .map(|axis| ((at[axis] as f32 + size[axis] + 1.0) - pivot[axis]) * scale + pivot[axis]);
        let mut rect = [
            low[0].min(high[0]),
            low[1].min(high[1]),
            (high[0] - low[0]).abs(),
            (high[1] - low[1]).abs(),
        ];
        if let Some([left, top, right, bottom]) = self.clip {
            let end = [
                (rect[0] + rect[2]).min(right as f32),
                (rect[1] + rect[3]).min(bottom as f32),
            ];
            rect[0] = rect[0].max(left as f32);
            rect[1] = rect[1].max(top as f32);
            rect[2] = end[0] - rect[0];
            rect[3] = end[1] - rect[1];
        }
        (rect.iter().all(|v| v.is_finite()) && rect[2] >= 0.0 && rect[3] >= 0.0).then_some(rect)
    }

    pub(crate) fn contains(&self, point: [f32; 2]) -> bool {
        if self.form == Hit::Through {
            return false;
        }
        // Logical clipping precedes the inverse, with inclusive upper edges.
        if self.clip.is_some_and(|[left, top, right, bottom]| {
            point[0] < left as f32
                || (right as f32) < point[0]
                || point[1] < top as f32
                || (bottom as f32) < point[1]
        }) {
            return false;
        }
        let (world, pivot, inherited) = self.world();
        let point = if scaled(inherited) {
            [0, 1].map(|axis| (point[axis] - pivot[axis]) / inherited + pivot[axis])
        } else {
            point
        };
        let at = world.map(truncate_f32);
        match self.shape {
            Shape::Text { size, .. } => {
                let point = point.map(truncate_f32);
                (0..2).all(|axis| {
                    at[axis] < point[axis] && point[axis] < at[axis].wrapping_add(size[axis])
                })
            }
            Shape::Sprite { frame, .. } => {
                let point = point.map(truncate_f32);
                (0..2).all(|axis| {
                    at[axis] <= point[axis] && (point[axis] as f32) < frame[axis] + at[axis] as f32
                })
            }
            Shape::Rectangle(size) => (0..2).all(|axis| {
                at[axis] as f32 <= point[axis]
                    && point[axis] < at[axis].wrapping_add(size[axis]) as f32
            }),
        }
    }
}

fn scaled(value: f32) -> bool {
    (value - 1.0).abs() > 0.001
}

/// Native ADDSS first, then conditional MULSS/ADDSS. Half is flag144==0.
/// Own scale is gated independently of incoming scale.
fn advance(
    translation: &mut [f32; 2],
    scale: &mut f32,
    local: [f32; 2],
    own: f32,
    half: Option<[f32; 2]>,
) {
    let incoming = *scale;
    if own.abs() > 0.001 {
        *scale = own * incoming;
    }
    let delta = incoming - 1.0;
    let added = [translation[0] + local[0], local[1] + translation[1]];
    *translation = if delta.abs() > 0.001 {
        let correction = half.map_or(local, |half| [half[0] + local[0], local[1] - half[1]]);
        [
            correction[0] * delta + added[0],
            correction[1] * delta + added[1],
        ]
    } else {
        added
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGICAL: [i32; 2] = [1920, 1080];
    const PHYSICAL: [i32; 2] = [2560, 1440];

    #[test]
    fn native_add_then_correction_keeps_rounding_and_incoming_epsilon() {
        // ADDSS rounds 20_000_000 + 3 to 20_000_004 before the next ADDSS.
        // A composed 2*3 + translation would instead give 20_000_006.
        let mut at = [20_000_000.0, 0.0];
        let mut scale = 2.0;
        advance(&mut at, &mut scale, [3.0, 0.0], 0.5, None);
        assert_eq!(at[0], 20_000_008.0);
        assert_eq!(scale, 1.0);

        let mut at = [10.0, -10.0];
        let mut scale = 1.0005;
        advance(&mut at, &mut scale, [20.0, -20.0], 2.0, None);
        assert_eq!(at, [30.0, -30.0]);
        assert_eq!(scale, 2.001);
        // Zero and below-threshold content scales do not collapse descendants.
        for own in [0.0, 0.001, -0.001] {
            let mut at = [0.0; 2];
            let mut scale = 1.0;
            advance(&mut at, &mut scale, [4.0, -5.0], own, None);
            assert_eq!((at, scale), ([4.0, -5.0], 1.0));
        }
    }

    #[test]
    fn cancelled_scales_keep_each_parent_translation() {
        let mut hit = HitGeometry::text([20, 20], [3, 4], [10, 10], 4, LOGICAL, None);
        hit.inherit(0.5, [20, 20]);
        hit.inherit(2.0, [10, 10]);
        assert_eq!(hit.world(), ([33.0, 34.0], [33.0, 38.0], 1.0));
        assert!(hit.contains([34.0, 35.0]));
        assert!(!hit.contains([24.0, 25.0]));
    }

    #[test]
    fn sprite_inverse_uses_bottom_left_and_full_fractional_frame() {
        let mut hit = HitGeometry::sprite(
            [20, 20],
            [8, 10],
            [8.5, 10.5],
            Hit::Frame,
            LOGICAL,
            PHYSICAL,
            None,
        );
        hit.inherit(2.0, [10, 10]);
        assert_eq!(hit.world(), ([30.0, 40.0], [30.0, 50.0], 2.0));
        assert!(hit.contains([30.0, 30.0]));
        assert!(hit.contains([47.9, 51.9]));
        assert!(!hit.contains([48.0, 40.0]));
        assert!(!hit.contains([35.0, 52.0]));
        assert!(!hit.contains([29.9, 40.0]));
    }

    #[test]
    fn scaled_text_keeps_unscaled_layout_position_and_measured_height() {
        let mut hit = HitGeometry::text([10, 10], [20, 20], [10, 10], 4, LOGICAL, None);
        hit.inherit(2.0, [10, 10]);
        assert_eq!(hit.world(), ([30.0, 30.0], [30.0, 34.0], 2.0));
        assert!(hit.contains([32.0, 28.0]));
        assert!(!hit.contains([31.9, 28.0]));
        assert!(!hit.contains([40.0, 46.0]));
        // Clip tests the original pointer, and its upper edge is inclusive.
        hit.clip = Some([32, 28, 40, 40]);
        assert!(hit.contains([40.0, 40.0]));
        assert!(!hit.contains([40.001, 40.0]));
        hit.clip = Some([40, 40, 40, 40]);
        assert!(hit.contains([40.0, 40.0]));
        assert!(!hit.contains([39.999, 40.0]));
        assert_eq!(hit.bounds(), Some([40.0, 40.0, 0.0, 0.0]));
    }

    #[test]
    fn negative_fractional_text_truncates_both_origin_and_pointer() {
        let mut hit = HitGeometry::text([-2, -2], [1, 1], [10, 10], 4, LOGICAL, None);
        hit.inherit(1.0, [-2, -2]);
        hit.inherit(0.5, [-3, -3]);
        // World -1.5 truncates to -1 before the strict comparison.
        assert_eq!(hit.world().0, [-1.5, -1.5]);
        assert!(hit.contains([-0.2, 2.0]));
        assert!(!hit.contains([-1.25, 2.0]));
    }

    #[test]
    fn container_uses_float_pointer_and_zero_getsize_fallback_pivot() {
        let mut hit = HitGeometry::rectangle([20, 20], [8, 10], [100, 100], None);
        hit.inherit(2.0, [10, 10]);
        assert_eq!(hit.world(), ([30.0, 30.0], [80.0, 70.0], 2.0));
        assert!(hit.contains([-20.0, -10.0]));
        assert!(!hit.contains([-20.001, -10.0]));
        assert!(!hit.contains([-4.0, 0.0]));
        assert!(!hit.contains([-10.0, 10.0]));
    }

    #[test]
    fn odd_logical_viewport_preserves_integer_half_before_float_half() {
        let hit = HitGeometry::rectangle([10, 20], [8, 10], [101, 101], None);
        assert_eq!(hit.world(), ([9.5, 19.5], [60.0, 60.0], 1.0));
        assert!(hit.contains([9.0, 19.0]));
        assert!(!hit.contains([8.999, 19.0]));
        assert!(!hit.contains([17.0, 19.0]));
    }
}
