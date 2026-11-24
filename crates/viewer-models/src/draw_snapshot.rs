use crate::*;

#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn count(&mut self, count: usize) {
        self.0.extend(u64::try_from(count).unwrap().to_le_bytes());
    }

    fn text(&mut self, text: &str) {
        self.count(text.len());
        self.0.extend(text.as_bytes());
    }

    fn optional(&mut self, text: &Option<String>) {
        self.0.push(u8::from(text.is_some()));
        if let Some(text) = text {
            self.text(text);
        }
    }

    fn floats(&mut self, values: &[f32]) {
        self.count(values.len());
        for value in values {
            self.0.extend(value.to_bits().to_le_bytes());
        }
    }

    fn words(&mut self, values: &[u32]) {
        self.count(values.len());
        for value in values {
            self.0.extend(value.to_le_bytes());
        }
    }
}

pub(super) fn snapshot(library: &ModelLibrary, slot: usize, textures: &[String]) -> Vec<u8> {
    assert_eq!(textures.len(), library.textures.len());
    let mut bytes = Bytes::default();
    let descriptors = &library.meshes[slot].native;
    bytes.count(descriptors.len());
    for descriptor in descriptors {
        bytes.text(&descriptor.name);
        bytes.count(descriptor.node);
        bytes.floats(&descriptor.distances);
        bytes.count(descriptor.lods.len());
        for lod in &descriptor.lods {
            bytes.count(lod.len());
            for part in lod {
                let (path, scale, shape, sub) = &part.geometry_key;
                bytes.text(path);
                bytes.words(&[*scale]);
                bytes.count(*shape);
                bytes.count(*sub);
                bytes.optional(&part.material.shader);
                bytes.optional(&part.material.shader_file);
                bytes.text(&part.material.diffuse);
                bytes.text(&part.material.normal);
                bytes.text(&part.material.specular);
                let material = &library.materials[part.material_index];
                for texture in [material.diffuse, material.normal, material.specular] {
                    bytes.text(&textures[texture]);
                }
                let geometry = &part.geometry;
                bytes.floats(&geometry.vertices);
                bytes.words(&geometry.indices);
                bytes.count(geometry.skin.len());
                for skin in &geometry.skin {
                    bytes.0.extend(skin.indices);
                    bytes.floats(&skin.weights);
                }
                bytes.0.push(u8::from(geometry.uv1));
                bytes.0.extend(geometry.stride.to_le_bytes());
            }
        }
    }
    bytes.0
}

#[cfg(test)]
mod tests {
    use crate::draw_snapshot::*;
    use viewer_scene_base::mesh_shader::{Geometry, SkinVertex};

    fn shard(name: &str, reach: f32, color: u8) -> ModelLibrary {
        let descriptors = mesh::parse_descriptors(
            format!("entity = {{ name = {name} }}").as_bytes(),
            "synthetic.asset",
        )
        .unwrap();
        let mut library = ModelLibrary::load(
            &ResolvedLayers::default(),
            Decode::default(),
            &descriptors,
            &[name.into()],
            &mut Vec::new(),
        )
        .unwrap();
        library.textures.push(dds::Texture::from_rgba8(Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![color, 0, 0, 255],
        }));
        library.spaces.push(ColorSpace::Linear);
        library
            .paths
            .insert((format!("{name}.dds"), ColorSpace::Linear), 0);
        library.materials.push(ModelMaterial {
            diffuse: 0,
            normal: 0,
            specular: 0,
        });
        let part = native::Part {
            geometry: Arc::new(Geometry {
                vertices: vec![
                    0.,
                    -0.,
                    f32::from_bits(0x7fc00001),
                    1.,
                    0.,
                    0.,
                    0.,
                    0.,
                    reach,
                ],
                skin: vec![
                    SkinVertex {
                        indices: [0, 1, 2, 3],
                        weights: [1., -0., 0.]
                    };
                    3
                ],
                indices: vec![0, 1, 2],
                stride: 12,
                uv1: false,
            }),
            geometry_key: (format!("{name}.mesh"), 1_f32.to_bits(), 0, 0),
            material_index: 0,
            material: mesh::Material {
                shader: Some("SyntheticShader".into()),
                ..Default::default()
            },
        };
        let mut second_part = part.clone();
        second_part.material.shader = Some("SecondaryShader".into());
        second_part.geometry_key.3 = 1;
        Arc::make_mut(&mut second_part.geometry).vertices[8] += 1.;
        library.meshes[0].native = ["root", "attachment"]
            .into_iter()
            .map(|descriptor| native::Descriptor {
                name: descriptor.into(),
                node: 0,
                distances: vec![16., 64.],
                lods: vec![vec![part.clone()], vec![part.clone(), second_part.clone()]],
            })
            .collect();
        library
    }

    fn merged(reverse: bool) -> ModelLibrary {
        let mut library = ModelLibrary::default();
        for name in if reverse {
            ["second", "first"]
        } else {
            ["first", "second"]
        } {
            library.merge(if name == "first" {
                shard(name, 2., 37)
            } else {
                shard(name, 7., 211)
            });
        }
        library
    }

    #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
    fn draws(library: &ModelLibrary) -> BTreeMap<AssetKey, Vec<u8>> {
        let textures: Vec<_> = library
            .textures
            .iter()
            .zip(&library.spaces)
            .map(|(texture, space)| format!("{space:?}:{texture:?}"))
            .collect();
        library
            .meshes
            .iter()
            .enumerate()
            .map(|(slot, mesh)| {
                (
                    mesh.asset.clone(),
                    library.native_draw_snapshot(slot, &textures),
                )
            })
            .collect()
    }

    #[test]
    fn reversed_merges_preserve_distinct_native_draws_and_detect_swapped_effective_bindings() {
        let reference = merged(false);
        let mut reversed = merged(true);
        #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
        let expected = draws(&reference);
        assert_eq!(expected.len(), 2);
        assert_eq!(reversed.materials.len(), 2);
        assert_eq!(reversed.textures.len(), 2);
        assert_ne!(reference.slot("first"), reversed.slot("first"));
        assert_eq!(draws(&reversed), expected);
        let first = reversed.slot("first").unwrap();
        let second = reversed.slot("second").unwrap();
        let binding = reversed.meshes[first].native[0].lods[0][0].material_index;
        let other = reversed.meshes[second].native[0].lods[0][0].material_index;
        assert_ne!(binding, other);
        assert_ne!(
            reversed.meshes[first].native[0].lods[0][0]
                .geometry
                .vertices[8]
                .to_bits(),
            reversed.meshes[second].native[0].lods[0][0]
                .geometry
                .vertices[8]
                .to_bits()
        );
        let materials = reversed.materials.clone();
        let textures = reversed.textures.clone();
        reversed.meshes[first].native[0].lods[0][0].material_index = other;
        reversed.meshes[second].native[0].lods[0][0].material_index = binding;
        assert_eq!(reversed.materials, materials);
        assert_eq!(reversed.textures, textures);
        #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
        let swapped = draws(&reversed);
        assert_ne!(
            swapped[&reversed.meshes[first].asset],
            expected[&reversed.meshes[first].asset]
        );
        assert_ne!(
            swapped[&reversed.meshes[second].asset],
            expected[&reversed.meshes[second].asset]
        );
    }

    #[test]
    fn native_draw_bytes_preserve_order_node_indices_float_payloads_and_skin() {
        let library = merged(false);
        #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
        let expected = draws(&library);
        for change in 0..12 {
            let mut changed = merged(false);
            let slot = changed.slot("first").unwrap();
            let descriptors = &mut changed.meshes[slot].native;
            match change {
                0 => descriptors.reverse(),
                1 => descriptors[0].node += 1,
                2 => descriptors[0].lods.reverse(),
                3 => descriptors[0].distances[0] = -0.,
                4 => {
                    Arc::make_mut(&mut descriptors[0].lods[0][0].geometry).vertices[2] =
                        f32::from_bits(0x7fc00002)
                }
                5 => Arc::make_mut(&mut descriptors[0].lods[0][0].geometry).skin[0].weights[1] = 0.,
                6 => Arc::make_mut(&mut descriptors[0].lods[0][0].geometry)
                    .indices
                    .reverse(),
                7 => descriptors[0].lods[1].reverse(),
                8 => descriptors[0].lods[1][0].material.shader = Some("DifferentShader".into()),
                9 => Arc::make_mut(&mut descriptors[0].lods[0][0].geometry).skin[0]
                    .indices
                    .reverse(),
                10 => Arc::make_mut(&mut descriptors[0].lods[0][0].geometry).stride += 4,
                11 => Arc::make_mut(&mut descriptors[0].lods[0][0].geometry).uv1 = true,
                _ => unreachable!(),
            }
            assert_ne!(draws(&changed), expected, "native draw change {change}");
        }
    }
}
