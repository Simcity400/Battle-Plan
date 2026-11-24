use super::{Batch, Exact, Quad};
use crate::native_shader::draw;
use std::sync::Arc;


pub(super) struct Upload {
    quads: usize,
    windows: Vec<super::BatchWindow>,
    loose: Vec<(usize, Quad)>,
    order: Vec<usize>,
    size: [u32; 2],
    scale: u32,
    artwork: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stands {
    Window(super::WindowKey, super::WindowStamp, [i32; 2], u32),
    Loose,
}

impl Upload {
    pub(super) fn new(
        batch: &Batch,
        order: &[usize],
        size: [u32; 2],
        scale: f32,
        artwork: Option<u64>,
    ) -> Self {
        let mut upload = Self {
            quads: 0,
            windows: Vec::new(),
            loose: Vec::new(),
            order: Vec::new(),
            size,
            scale: scale.to_bits(),
            artwork,
        };
        upload.update(batch, order, size, scale, artwork);
        upload
    }

    pub(super) fn update(
        &mut self,
        batch: &Batch,
        order: &[usize],
        size: [u32; 2],
        scale: f32,
        artwork: Option<u64>,
    ) {
        self.quads = batch.quads.len();
        self.windows.clear();
        self.windows.extend(batch.windows());
        self.loose.clear();
        self.loose.extend(
            (0..self.quads)
                .filter(|index| stands(&self.windows, *index) == Stands::Loose)
                .map(|index| (index, batch.quads[index].clone())),
        );
        self.order.clear();
        if order.len() == batch.quads.len() {
            self.order.extend_from_slice(order);
        }
        self.size = size;
        self.scale = scale.to_bits();
        self.artwork = artwork;
    }

    fn stood(&self, index: usize) -> Stands {
        stands(&self.windows, index)
    }

    pub(super) fn matches(
        &self,
        batch: &Batch,
        size: [u32; 2],
        scale: f32,
        artwork: Option<u64>,
    ) -> bool {
        self.size == size
            && self.scale == scale.to_bits()
            && self.artwork == artwork
            && self.quads == batch.quads.len()
            && self.windows.iter().copied().eq(batch.windows())
            && self
                .loose
                .iter()
                .all(|(index, quad)| same_quad(quad, &batch.quads[*index]))
    }

    pub(super) fn moved_windows(
        &self,
        batch: &Batch,
        size: [u32; 2],
        scale: f32,
        artwork: Option<u64>,
    ) -> Option<Vec<usize>> {
        if self.size != size
            || self.scale != scale.to_bits()
            || self.artwork != artwork
            || self.quads != batch.quads.len()
        {
            return None;
        }
        let mut moved = Vec::new();
        let mut windows = batch.windows();
        for (at, held) in self.windows.iter().enumerate() {
            let now = windows.next()?;
            if (now.key, now.stamp, now.first, now.count)
                != (held.key, held.stamp, held.first, held.count)
            {
                return None;
            }
            if now.shift != held.shift {
                moved.push(at);
            }
        }
        if windows.next().is_some() {
            return None;
        }
        self.loose
            .iter()
            .all(|(index, quad)| same_quad(quad, &batch.quads[*index]))
            .then_some(moved)
    }

    pub(super) fn window(&self, at: usize) -> super::BatchWindow {
        self.windows[at]
    }

    pub(super) fn matches_in_order(
        &self,
        batch: &Batch,
        order: &[usize],
        size: [u32; 2],
        scale: f32,
        artwork: Option<u64>,
    ) -> bool {
        if self.size != size
            || self.scale != scale.to_bits()
            || self.artwork != artwork
            || self.quads != batch.quads.len()
            || self.order.len() != self.quads
            || order.len() != batch.quads.len()
        {
            return false;
        }
        let windows: Vec<_> = batch.windows().collect();
        order.iter().zip(&self.order).all(|(&now, &then)| {
            let stands = stands(&windows, now);
            match (stands, self.stood(then)) {
                (Stands::Window(..), _) | (_, Stands::Window(..)) => stands == self.stood(then),
                (Stands::Loose, Stands::Loose) => self
                    .loose
                    .binary_search_by_key(&then, |(index, _)| *index)
                    .is_ok_and(|at| same_drawn(&batch.quads[now], &self.loose[at].1)),
            }
        })
    }
}

fn stands(windows: &[super::BatchWindow], index: usize) -> Stands {
    let at = windows.partition_point(|window| (window.first as usize) <= index);
    match at.checked_sub(1).map(|at| windows[at]) {
        Some(window) if window.quads().contains(&index) => {
            Stands::Window(window.key, window.stamp, window.shift, index as u32 - window.first)
        }
        _ => Stands::Loose,
    }
}

#[inline]
fn floats(a: &[f32], b: &[f32]) -> bool {
    let a: &[u8] = bytemuck::cast_slice(a);
    a == bytemuck::cast_slice::<_, u8>(b)
}

fn same_quad(a: &Quad, b: &Quad) -> bool {
    a.layer == b.layer && same_drawn(a, b)
}

fn same_drawn(a: &Quad, b: &Quad) -> bool {
    a.texture == b.texture
        && floats(&a.rect, &b.rect)
        && floats(&a.uv, &b.uv)
        && floats(&a.color, &b.color)
        && match (a.rotation, b.rotation) {
            (None, None) => true,
            (Some(a), Some(b)) => a.to_bits() == b.to_bits(),
            _ => false,
        }
        && a.shade.to_bits() == b.shade.to_bits()
        && match (a.anchor, b.anchor) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.family == b.family
                    && floats(&a.world, &b.world)
                    && a.cutoff.to_bits() == b.cutoff.to_bits()
            }
            _ => false,
        }
        && match (a.present, b.present) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.rank == b.rank
                    && floats(&a.pivot, &b.pivot)
                    && a.scale.to_bits() == b.scale.to_bits()
                    && a.alpha.to_bits() == b.alpha.to_bits()
            }
            _ => false,
        }
        && match (&a.exact, &b.exact) {
            (None, None) => true,
            (Some(a), Some(b)) => same_exact(a, b),
            _ => false,
        }
}

fn same_exact(a: &Exact, b: &Exact) -> bool {
    a.scissor == b.scissor
        && a.technique == b.technique
        && a.blend == b.blend
        && a.samplers == b.samplers
        && a.second == b.second
        && a.masking == b.masking
        && floats(
            bytemuck::cast_slice(&a.corners),
            bytemuck::cast_slice(&b.corners),
        )
        && floats(bytemuck::cast_slice(&a.uv), bytemuck::cast_slice(&b.uv))
        && floats(&a.lod_bias, &b.lod_bias)
        && floats(&a.offsets, &b.offsets)
        && floats(&a.inputs, &b.inputs)
        && floats(&a.second_color, &b.second_color)
        && match (&a.native, &b.native) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || same_native(a, b),
            _ => false,
        }
}

fn same_native(a: &draw::Quad, b: &draw::Quad) -> bool {
    a.stride == b.stride
        && a.known == b.known
        && a.pixel_constants == b.pixel_constants
        && a.font_viewport == b.font_viewport
        && a.topology == b.topology
        && a.default_blend == b.default_blend
        && a.declaration == b.declaration
        && floats(&a.constants, &b.constants)
        && a.effect.technique == b.effect.technique
        && (Arc::ptr_eq(&a.effect.asset, &b.effect.asset) || a.effect.asset == b.effect.asset)
        && a.textures == b.textures
        && a.extra_constants.len() == b.extra_constants.len()
        && a.extra_constants
            .iter()
            .zip(&b.extra_constants)
            .all(|(a, b)| a.0 == b.0 && a.2 == b.2 && floats(&a.1, &b.1))
        && a.path == b.path
        && a.vertices == b.vertices
        && match (&a.bake, &b.bake) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                Arc::ptr_eq(a, b)
                    || (a.size == b.size
                        && a.draws.len() == b.draws.len()
                        && a.draws.iter().zip(&b.draws).all(|(a, b)| same_native(a, b)))
            }
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_upload_float_inputs_include_every_component_and_nan_bit() {
        fn values<const N: usize>(next: &mut u32) -> [f32; N] {
            std::array::from_fn(|_| {
                let value = f32::from_bits(*next);
                *next += 1;
                value
            })
        }
        let mut next = 0x7fc0_0001;
        let quad = Quad {
            texture: 0,
            layer: 0,
            rect: values(&mut next),
            uv: values(&mut next),
            color: values(&mut next),
            rotation: Some(values::<1>(&mut next)[0]),
            shade: values::<1>(&mut next)[0],
            exact: None,
            anchor: None,
            present: None,
        };
        assert!(same_quad(&quad, &quad.clone()));
        let mut unrotated = quad.clone();
        unrotated.rotation = None;
        assert!(!same_quad(&quad, &unrotated));
        for index in 0..14 {
            let mut changed = quad.clone();
            let value = changed
                .rect
                .iter_mut()
                .chain(&mut changed.uv)
                .chain(&mut changed.color)
                .chain(changed.rotation.iter_mut())
                .chain(std::iter::once(&mut changed.shade))
                .nth(index)
                .unwrap();
            *value = f32::from_bits(value.to_bits() ^ 1);
            assert!(!same_quad(&quad, &changed), "quad component {index}");
        }
        let exact = Exact {
            native: None,
            corners: std::array::from_fn(|_| values(&mut next)),
            uv: std::array::from_fn(|_| values(&mut next)),
            lod_bias: values(&mut next),
            offsets: values(&mut next),
            inputs: values(&mut next),
            second_color: values(&mut next),
            scissor: None,
            technique: 0,
            blend: crate::native_effects::Blend::Off,
            samplers: 0,
            second: None,
            masking: None,
        };
        assert!(same_exact(&exact, &exact.clone()));
        for index in 0..31 {
            let mut changed = exact.clone();
            let value = changed
                .corners
                .iter_mut()
                .flatten()
                .chain(changed.uv.iter_mut().flatten())
                .chain(&mut changed.lod_bias)
                .chain(&mut changed.offsets)
                .chain(&mut changed.inputs)
                .chain(&mut changed.second_color)
                .nth(index)
                .unwrap();
            *value = f32::from_bits(value.to_bits() ^ 1);
            assert!(!same_exact(&exact, &changed), "exact component {index}");
        }
    }

    #[test]
    fn retained_upload_inputs_compare_native_and_nested_bake_float_bits() {
        let program =
            content_reader_gui::pdxshader::program::read_with::<Vec<u8>>(b"absent.lua", &|_| None).unwrap();
        let a = draw::Quad {
            source_draws: 1,
            path: "synthetic".into(),
            effect: draw::Effect {
                asset: shader_pipeline::library::Effect::from_compiled(
                    Arc::new(program),
                    vec![],
                    vec![],
                ),
                technique: "Up".into(),
            },
            declaration: shader_pipeline::vertices::Declaration::Sprite,
            vertices: vec![0; 20],
            stride: 20,
            constants: vec![0.; 16],
            known: 64,
            pixel_constants: true,
            textures: vec![(0, Some(1))],
            extra_constants: vec![(1, vec![0.; 4], std::iter::once(0..16).collect())],
            font_viewport: None,
            topology: eframe::wgpu::PrimitiveTopology::TriangleStrip,
            default_blend: false,
            bake: None,
        };
        assert!(same_native(&a, &a.clone()));
        let mut b = a.clone();
        b.constants[0] = -0.;
        assert!(!same_native(&a, &b));
        b = a.clone();
        b.extra_constants[0].1[0] = -0.;
        assert!(!same_native(&a, &b));
        b = a.clone();
        b.textures[0].1 = None;
        assert!(!same_native(&a, &b));
        b = a.clone();
        b.effect.technique = "Down".into();
        assert!(!same_native(&a, &b));
        let mut a = a;
        a.bake = Some(Arc::new(draw::Bake {
            size: [4, 4],
            draws: vec![a.clone()],
        }));
        let mut b = a.clone();
        assert!(same_native(&a, &b));
        Arc::make_mut(b.bake.as_mut().unwrap()).draws[0].constants[0] = -0.;
        assert!(!same_native(&a, &b));
    }

    #[test]
    fn retained_upload_misses_reuse_snapshot_storage_and_replace_every_input() {
        let mut batch = Batch::default();
        batch.scaled(
            &super::super::SpriteSheet {
                name: "synthetic".into(),
                texture: 0,
                frames: 1,
                size: [4, 4],
            },
            0,
            [0., 0.],
            [4., 4.],
            [1.; 4],
        );
        let mut kept = Upload::new(&batch, &[], [16, 16], 1., Some(3));
        let storage = kept.loose.as_ptr();
        let capacity = kept.loose.capacity();
        batch.quads[0].rect[0] = -0.;
        assert!(!kept.matches(&batch, [17, 18], 2., Some(4)));
        kept.update(&batch, &[], [17, 18], 2., Some(4));
        assert!(kept.matches(&batch, [17, 18], 2., Some(4)));
        assert_eq!(kept.loose.as_ptr(), storage);
        assert_eq!(kept.loose.capacity(), capacity);
        batch.clear();
        kept.update(&batch, &[], [17, 18], 2., Some(4));
        assert!(kept.matches(&batch, [17, 18], 2., Some(4)));
        assert_eq!(kept.loose.capacity(), capacity);
    }

    fn sheet() -> super::super::SpriteSheet {
        super::super::SpriteSheet {
            name: "synthetic".into(),
            texture: 0,
            frames: 1,
            size: [4, 4],
        }
    }

    fn windowed(owner: u64, stamp: super::super::WindowStamp, quads: usize) -> Batch {
        use super::super::{WindowKey, WindowPool};
        let mut batch = Batch::default();
        batch.at_window(WindowKey::new(WindowPool::Base, 0, owner), stamp);
        for index in 0..quads {
            batch.scaled(&sheet(), 0, [index as f32, 0.], [4., 4.], [1.; 4]);
        }
        batch.no_window();
        batch.scaled(&sheet(), 0, [100., 0.], [4., 4.], [1.; 4]);
        batch
    }

    #[test]
    fn a_window_that_draws_what_it_drew_is_retained_without_comparing_its_quads() {
        use super::super::WindowStamp;
        let stamp = WindowStamp { painting: 7, taken: 11 };
        let batch = windowed(3, stamp, 4);
        let kept = Upload::new(&batch, &[], [16, 16], 1., Some(3));
        assert!(kept.matches(&batch, [16, 16], 1., Some(3)));
        let mut moved = batch.clone();
        for quad in &mut moved.quads[..4] {
            quad.rect[0] = f32::from_bits(0x7fc0_0001);
        }
        assert!(kept.matches(&moved, [16, 16], 1., Some(3)));
        let mut loose = batch;
        loose.quads[4].rect[0] = -0.;
        assert!(!kept.matches(&loose, [16, 16], 1., Some(3)));
        for changed in [
            windowed(3, WindowStamp { painting: 8, taken: 11 }, 4),
            windowed(3, WindowStamp { painting: 7, taken: 12 }, 4),
            windowed(3, stamp, 5),
            windowed(4, stamp, 4),
        ] {
            assert!(!kept.matches(&changed, [16, 16], 1., Some(3)));
        }
    }

    fn two_windows() -> Batch {
        use super::super::WindowStamp;
        let mut batch = Batch::default();
        for owner in [1_u64, 2] {
            batch.append(&windowed(
                owner,
                WindowStamp { painting: owner, taken: owner * 10 },
                2,
            ));
        }
        batch
    }

    #[test]
    fn a_batch_whose_windows_only_stand_elsewhere_names_those_windows_and_nothing_else() {
        let batch = two_windows();
        let kept = Upload::new(&batch, &[], [16, 16], 1., Some(3));
        assert_eq!(
            kept.moved_windows(&batch, [16, 16], 1., Some(3)),
            Some(Vec::new()),
            "a batch standing where it stood moved no window"
        );
        let mut moved = batch.clone();
        moved.standing_windows()[1].shift = [3, -2];
        for quad in &mut moved.quads[3..5] {
            quad.rect[0] += 3.0;
        }
        assert_eq!(kept.moved_windows(&moved, [16, 16], 1., Some(3)), Some(vec![1]));
        let mut painted = batch.clone();
        painted.standing_windows()[0].stamp.painting += 1;
        assert_eq!(kept.moved_windows(&painted, [16, 16], 1., Some(3)), None);
        let mut loose = moved.clone();
        loose.quads[2].rect[0] = -0.0;
        assert_eq!(kept.moved_windows(&loose, [16, 16], 1., Some(3)), None);
        assert_eq!(kept.moved_windows(&batch, [17, 16], 1., Some(3)), None);
        assert_eq!(kept.moved_windows(&batch, [16, 16], 2., Some(3)), None);
        assert_eq!(kept.moved_windows(&batch, [16, 16], 1., Some(4)), None);
        let mut more = batch;
        more.append(&windowed(
            3,
            super::super::WindowStamp { painting: 3, taken: 30 },
            2,
        ));
        assert_eq!(kept.moved_windows(&more, [16, 16], 1., Some(3)), None);
    }

    #[test]
    fn windows_that_only_changed_places_among_others_draw_the_same() {
        use super::super::WindowStamp;
        let stamp = |owner: u64| WindowStamp { painting: owner, taken: owner * 10 };
        let mut batch = Batch::default();
        for owner in [1_u64, 2] {
            batch.append(&windowed(owner, stamp(owner), 2));
        }
        let mut order: Vec<usize> = (0..batch.quads.len()).collect();
        let kept = Upload::new(&batch, &order, [16, 16], 1., Some(3));
        assert!(kept.matches_in_order(&batch, &order, [16, 16], 1., Some(3)));
        order.swap(0, 3);
        assert!(!kept.matches_in_order(&batch, &order, [16, 16], 1., Some(3)));
        let mut again = Batch::default();
        for owner in [1_u64, 2] {
            again.append(&windowed(owner, stamp(owner), 2));
        }
        for quad in &mut again.quads {
            quad.layer += 1;
        }
        let order: Vec<usize> = (0..again.quads.len()).collect();
        assert!(kept.matches_in_order(&again, &order, [16, 16], 1., Some(3)));
    }

    #[test]
    fn retained_upload_inputs_preserve_float_bits_order_and_output_parameters() {
        let mut batch = Batch::default();
        batch.scaled(
            &super::super::SpriteSheet {
                name: "synthetic".into(),
                texture: 0,
                frames: 1,
                size: [4, 4],
            },
            0,
            [0., 0.],
            [4., 4.],
            [1.; 4],
        );
        let kept = Upload::new(&batch, &[], [16, 16], 1., Some(3));
        assert!(kept.matches(&batch, [16, 16], 1., Some(3)));
        assert!(!kept.matches(&batch, [17, 16], 1., Some(3)));
        assert!(!kept.matches(&batch, [16, 16], 2., Some(3)));
        assert!(!kept.matches(&batch, [16, 16], 1., Some(4)));
        batch.quads[0].rect[0] = -0.;
        assert!(!kept.matches(&batch, [16, 16], 1., Some(3)));
        batch.quads[0].rect[0] = 0.;
        batch.quads[0].layer += 1;
        assert!(!kept.matches(&batch, [16, 16], 1., Some(3)));
        batch.quads[0].layer -= 1;
        batch.quads.push(batch.quads[0].clone());
        assert!(!kept.matches(&batch, [16, 16], 1., Some(3)));
    }
}
