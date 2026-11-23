use crate::native::dump_tests::*;

#[derive(Default)]
pub(super) struct Resources {
    buffers: Vec<(wgpu::Buffer, wgpu::Buffer)>,
    textures: Vec<(wgpu::Texture, wgpu::Texture)>,
    views: Vec<(wgpu::TextureView, wgpu::TextureView)>,
}

fn pair<T: Clone + PartialEq + std::fmt::Debug>(pairs: &mut Vec<(T, T)>, a: &T, b: &T) {
    let left = pairs.iter().position(|(seen, _)| seen == a);
    let right = pairs.iter().position(|(_, seen)| seen == b);
    assert_eq!(
        left, right,
        "resource mapping changed or is not one-to-one: {a:?} -> {b:?}"
    );
    if left.is_none() {
        pairs.push((a.clone(), b.clone()));
    }
}

fn bijective<T: PartialEq>(pairs: &[(T, T)]) {
    for (index, (a, b)) in pairs.iter().enumerate() {
        for (other_a, other_b) in &pairs[..index] {
            assert!(a != other_a && b != other_b, "duplicate resource mapping");
        }
    }
}

impl Resources {
    pub fn buffer(&mut self, a: &wgpu::Buffer, b: &wgpu::Buffer) {
        assert_eq!((a.size(), a.usage()), (b.size(), b.usage()));
        pair(&mut self.buffers, a, b);
    }

    fn texture(&mut self, a: &wgpu::Texture, b: &wgpu::Texture) {
        assert_eq!(
            (
                a.size(),
                a.mip_level_count(),
                a.sample_count(),
                a.dimension(),
                a.format(),
                a.usage()
            ),
            (
                b.size(),
                b.mip_level_count(),
                b.sample_count(),
                b.dimension(),
                b.format(),
                b.usage()
            )
        );
        pair(&mut self.textures, a, b);
    }

    pub fn view(&mut self, a: &wgpu::TextureView, b: &wgpu::TextureView) {
        self.texture(a.texture(), b.texture());
        pair(&mut self.views, a, b);
    }

    pub fn check(&self) {
        bijective(&self.buffers);
        bijective(&self.textures);
        bijective(&self.views);
    }

    pub fn owners(&mut self, a: &ModelPass, b: &ModelPass) {
        assert_eq!(a.textures.len(), b.textures.len());
        for (a, b) in a.textures.iter().zip(&b.textures) {
            match (a, b) {
                (Some(a), Some(b)) => {
                    self.texture(&a.texture, &b.texture);
                    self.view(&a.view, &b.view);
                }
                (None, None) => {}
                _ => panic!("different texture residency"),
            }
        }
        let (a, b) = (a.native.borrow(), b.native.borrow());
        let (a, b) = (a.as_ref().unwrap(), b.as_ref().unwrap());
        assert_eq!(
            a.geometry.keys().collect::<Vec<_>>(),
            b.geometry.keys().collect::<Vec<_>>()
        );
        for (a, b) in a.geometry.values().zip(b.geometry.values()) {
            assert_eq!(
                (
                    a.stride,
                    a.native_stride,
                    &a.cpu_vertices,
                    &a.cpu_skin,
                    a.vertices,
                    a.indices,
                    a.skin_bones
                ),
                (
                    b.stride,
                    b.native_stride,
                    &b.cpu_vertices,
                    &b.cpu_skin,
                    b.vertices,
                    b.indices,
                    b.skin_bones
                )
            );
        }
        assert_eq!(
            a.pools.keys().collect::<Vec<_>>(),
            b.pools.keys().collect::<Vec<_>>()
        );
        for (a, b) in a
            .pools
            .values()
            .zip(b.pools.values())
            .chain(std::iter::once((&a.indices, &b.indices)))
        {
            assert_eq!(a.chunks.len(), b.chunks.len());
            for (a, b) in a.chunks.iter().zip(&b.chunks) {
                assert_eq!((a.capacity, a.used), (b.capacity, b.used));
                for (a, b) in a.streams.iter().zip(&b.streams) {
                    match (a, b) {
                        (Some(a), Some(b)) => self.buffer(a, b),
                        (None, None) => {}
                        _ => panic!("different geometry stream presence"),
                    }
                }
            }
        }
        assert_eq!(a.residents.len(), b.residents.len());
        for (a, b) in a.residents.iter().zip(&b.residents) {
            for (a, b) in [
                (a.retained.selection(), b.retained.selection()),
                (&a.pipeline.selection, &b.pipeline.selection),
            ] {
                assert_eq!(a.index, b.index);
                assert_eq!(a.effect.source, b.effect.source);
                assert_eq!(a.effect.defines, b.effect.defines);
                assert_eq!(
                    a.effect.compiled(a.index).unwrap(),
                    b.effect.compiled(b.index).unwrap()
                );
            }
            assert_eq!(a.retained.layouts(), b.retained.layouts());
            assert_eq!(a.retained.state(), b.retained.state());
            assert_eq!(
                (
                    &a.required_constants,
                    &a.required_textures,
                    a.constant_mask,
                    a.texture_mask,
                    a.caster_texture,
                    a.blended,
                    a.samplers
                ),
                (
                    &b.required_constants,
                    &b.required_textures,
                    b.constant_mask,
                    b.texture_mask,
                    b.caster_texture,
                    b.blended,
                    b.samplers
                )
            );
            match (&a.program, &b.program) {
                (Some((a_stamp, a)), Some((b_stamp, b))) => {
                    assert!(a_stamp == b_stamp);
                    assert_eq!(a.constant_reads(), b.constant_reads());
                    let (a, b) = (
                        a.texture_views_for_tests().collect::<Vec<_>>(),
                        b.texture_views_for_tests().collect::<Vec<_>>(),
                    );
                    assert_eq!(a.len(), b.len());
                    for ((a_slot, a), (b_slot, b)) in a.into_iter().zip(b) {
                        assert_eq!(a_slot, b_slot);
                        self.view(a, b);
                    }
                }
                (None, None) => {}
                _ => panic!("different successful program publication"),
            }
        }
    }

    pub fn draws(&mut self, models: &[ModelPass; 2], a: &DrawList, b: &DrawList) {
        let (old, new) = (models[0].native.borrow(), models[1].native.borrow());
        let renderers = [old.as_ref().unwrap(), new.as_ref().unwrap()];
        assert_eq!(
            a.vertices.keys().collect::<Vec<_>>(),
            b.vertices.keys().collect::<Vec<_>>()
        );
        for (a, b) in a.vertices.values().zip(b.vertices.values()) {
            for (a, b) in a.iter().zip(b) {
                match (a, b) {
                    (Some(a), Some(b)) => self.buffer(a, b),
                    (None, None) => {}
                    _ => panic!("different draw vertex streams"),
                }
            }
        }
        assert_eq!(
            a.indices.keys().collect::<Vec<_>>(),
            b.indices.keys().collect::<Vec<_>>()
        );
        for (a, b) in a.indices.values().zip(b.indices.values()) {
            self.buffer(a, b);
        }
        for (a, b) in a.draws.iter().zip(&b.draws) {
            let matches = [a, b]
                .into_iter()
                .zip(renderers)
                .map(|(draw, renderer)| {
                    renderer
                        .residents
                        .iter()
                        .enumerate()
                        .filter_map(|(index, resident)| {
                            let (_, program) = resident.program.as_ref()?;
                            program
                                .constant_buffer_for_tests(&draw.bindings)
                                .map(|buffer| (index, buffer))
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert!(
                !matches[0].is_empty(),
                "draw has no live successful-program witness"
            );
            assert_eq!(matches[0].len(), matches[1].len());
            for ((a_index, a), (b_index, b)) in matches[0].iter().zip(&matches[1]) {
                assert_eq!(a_index, b_index);
                self.buffer(a, b);
            }
        }
    }
}

#[test]
fn resource_pairs_reject_changes_and_aliases_in_both_directions() {
    let mut pairs = Vec::new();
    pair(&mut pairs, &1, &10);
    pair(&mut pairs, &2, &20);
    pair(&mut pairs, &1, &10);
    bijective(&pairs);
    assert_eq!(pairs, [(1, 10), (2, 20)]);
    for (a, b) in [(1, 20), (1, 30), (3, 10)] {
        let mut attempt = pairs.clone();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pair(&mut attempt, &a, &b)))
                .is_err()
        );
        assert_eq!(attempt, pairs);
    }
}
