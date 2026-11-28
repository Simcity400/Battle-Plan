use content_reader_mesh::mesh::MeshProducts as _;
use content_reader_layers::ResolvedLayers;
use content_reader_map::ambient;
use content_reader_image::IndexedImage;
use content_reader_image::native::Decode;
use std::io;
#[cfg(test)]
use viewer_gpu::province_admission::admits_box;
use viewer_models::{entity_animation, ModelField, ModelInstance, ModelLibrary, strike_effects};
use viewer_gpu::{camera::Camera, province_admission::Admission};

pub struct FrameAssets {
    pub library: ModelLibrary,
    pub field: ModelField,
    pub presentation: Presentation,
    pub strikes: strike_effects::Assets,
}

#[derive(Clone)]
struct Placement {
    kind: usize,
    slot: usize,
    instance: ModelInstance,
    bucket: Option<usize>,
}
#[derive(Clone)]
struct Kind {
    duration: f32,
    max_visible: i32,
}
#[derive(Clone)]
struct Active {
    placement: usize,
    remaining: f32,
    epoch: u32,
    last: Option<f32>,
}

#[derive(Clone, Default)]
pub struct Presentation {
    placements: Vec<Placement>,
    kinds: Vec<Kind>,
    bounds: Vec<[[f32; 3]; 2]>,
    active: Vec<Active>,
    counters: Vec<i32>,
    last: Option<f32>,
    epoch: u32,
}

impl FrameAssets {
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        facts: dashboard_text::triggers::ContentFacts<'_>,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let types = ambient::read(layers)?;
        let triggers = facts.triggers.map_err(io::Error::other)?;
        let world =
            dashboard_text::triggers::World::unrecorded(facts.dlcs, triggers, facts.tag_aliases);
        let roster = layers.descriptor_roster()?;
        warnings.extend(
            roster
                .unreadable
                .iter()
                .map(|e| format!("Map frame descriptor unavailable: {e}")),
        );
        let default = content_reader_key_names::terrain::read_default_map(layers)?;
        let heights = content_reader_map::terrain_images::read_indexed_image(
            layers, &format!("map/{}", default.heightmap),
        )?;
        let bounds = buckets([heights.width, heights.height]);
        let mut presentation = Presentation {
            bounds,
            counters: vec![0; types.len()],
            kinds: types
                .iter()
                .map(|t| Kind {
                    duration: t.duration,
                    max_visible: t.max_visible,
                })
                .collect(),
            ..Default::default()
        };
        let mut refused = Vec::new();
        let mut candidates = Vec::new();
        let mut names = Vec::new();
        let job = roster.descriptor_job();
        let descriptors = job.view();
        for (kind_index, kind) in types.iter().enumerate() {
            let condition = kind
                .dlc
                .as_ref()
                .map(|items| {
                    world
                        .evaluate(items, dashboard_text::triggers::Scopes::empty())
                        .decided()
                        .map_err(io::Error::other)
                })
                .transpose();
            if matches!(condition, Ok(Some(false))) {
                continue;
            }
            let condition_error = condition.err().map(|e| e.to_string());
            let inherited = descriptors.inherit(&kind.entity);
            let mesh = descriptors.resolve(&kind.entity).is_some();
            let reason = if let Some(error) = condition_error.as_deref() {
                Some(error)
            } else if !mesh && inherited.is_none() {
                Some("no entity or mesh declaration")
            } else {
                None
            };
            for object in &kind.objects {
                let bucket = (!object.always_queued)
                    .then(|| {
                        presentation
                            .bounds
                            .iter()
                            .position(|b| contains(*b, object.position))
                    })
                    .flatten();
                if let Some(reason) = reason.or((!object.always_queued && bucket.is_none())
                    .then_some("placement outside native ambient buckets"))
                {
                    refused.push(format!("{} / {} ({reason})", kind.entity, object.name));
                    continue;
                }
                let mut instance = instance(kind, object);
                if !kind.always_visible {
                    instance.position[1] += height(&heights, object.position);
                }
                instance.animation_seed =
                    u32::try_from(candidates.len()).map_err(io::Error::other)? ^ 0x414d_4249;
                candidates.push((kind_index, kind.entity.clone(), instance, bucket));
                names.push(kind.entity.clone());
            }
        }
        names.extend(
            strike_effects::Assets::names()
                .into_iter()
                .filter(|name| {
                    descriptors.resolve(name).is_some()
                        || descriptors.inherit(name).is_some()
                })
                .map(str::to_owned),
        );
        let mut model_warnings = Vec::new();
        let library = ModelLibrary::load(
            layers,
            decode,
            descriptors,
            &names,
            &mut model_warnings,
        )?;
        warnings.extend(model_warnings.into_iter().map(|(_, message)| message));
        let strikes = strike_effects::Assets::resolve(&library, warnings);
        for (kind, name, instance, bucket) in candidates {
            if let Some(slot) = library.slot(&name) {
                presentation.placements.push(Placement {
                    kind,
                    slot,
                    instance,
                    bucket,
                });
            } else {
                refused.push(format!("{name} (actor resources failed to load)"));
            }
        }
        if !refused.is_empty() {
            warnings.push(format!(
                "Map shader map frame: {} unsupported ambient placements: {}",
                refused.len(),
                refused.join(", ")
            ));
        }
        if presentation
            .placements
            .iter()
            .any(|p| p.instance.animation != entity_animation::STOPPED)
        {
            warnings.push("Ambient animation and particle clocks start at viewer observation; native historical populations, RNG and process-clock phase are not recorded.".into());
        }
        let field = ModelField::build(
            &library,
            presentation
                .placements
                .iter()
                .map(|p| (p.slot, p.instance))
                .collect(),
            256.,
        );
        Ok(Self {
            library,
            field,
            presentation,
            strikes,
        })
    }
}

fn instance(kind: &ambient::AmbientObjectType, object: &ambient::AmbientObject) -> ModelInstance {
    ModelInstance {
        position: object.position,
        rotation: object.rotation[1] / 360. * f32::from_bits(0x40c90fdb),
        scale: kind.scale,
        animation: if kind.use_animation {
            entity_animation::AMBIENT_IDLE
        } else {
            entity_animation::DEFAULT_STATE
        },
        ..Default::default()
    }
}

fn buckets([width, height]: [u32; 2]) -> Vec<[[f32; 3]; 2]> {
    let dx = (width / 4) as f32;
    let dz = (height / 4) as f32;
    (0..4)
        .flat_map(|z| {
            (0..4).map(move |x| {
                [
                    [
                        if x == 0 { -dx } else { x as f32 * dx },
                        -100.,
                        if z == 0 { -dz } else { z as f32 * dz },
                    ],
                    [
                        if x == 3 {
                            width as f32 + dx
                        } else {
                            (x + 1) as f32 * dx
                        },
                        1000.,
                        if z == 3 {
                            height as f32 + dz
                        } else {
                            (z + 1) as f32 * dz
                        },
                    ],
                ]
            })
        })
        .collect()
}
fn contains([low, high]: [[f32; 3]; 2], p: [f32; 3]) -> bool {
    (0..3).all(|i| low[i] <= p[i] && p[i] <= high[i])
}
fn height(image: &IndexedImage, [x, _, z]: [f32; 3]) -> f32 {
    let x = ((x + 0.5) as i32).clamp(0, image.width as i32 - 1) as usize;
    let z = ((z + 0.5) as i32).clamp(0, image.height as i32 - 1) as usize;
    (f32::from(image.indices[(image.height as usize - 1 - z) * image.width as usize + x]) * 0.1)
        .max(9.5)
}

impl Presentation {
    pub fn update(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        time: f32,
    ) -> Vec<(usize, ModelInstance)> {
        let admission = Admission::new(camera, size);
        let visible: Vec<_> = self
            .bounds
            .iter()
            .map(|b| admission.admits_box(*b))
            .collect();
        self.advance(time, &visible)
    }

    fn advance(&mut self, time: f32, visible: &[bool]) -> Vec<(usize, ModelInstance)> {
        if self.last.is_some_and(|last| time < last) {
            self.active.clear();
            self.counters.fill(0);
            self.epoch = 0;
            self.last = None;
        }
        self.last = Some(time);
        for bucket in (0..self.bounds.len())
            .map(Some)
            .chain(std::iter::once(None))
        {
            if bucket.is_some_and(|i| !visible[i]) {
                continue;
            }
            for (index, placement) in self
                .placements
                .iter()
                .enumerate()
                .filter(|(_, p)| p.bucket == bucket)
            {
                let kind = &self.kinds[placement.kind];
                if self.active.iter().any(|a| a.placement == index)
                    || (kind.max_visible > 0 && self.counters[placement.kind] > kind.max_visible)
                {
                    continue;
                }
                self.counters[placement.kind] = (self.counters[placement.kind] - 1).max(0);
                self.epoch = self.epoch.wrapping_add(1);
                self.active.push(Active {
                    placement: index,
                    remaining: kind.duration,
                    epoch: self.epoch,
                    last: None,
                });
            }
        }
        let mut i = 0;
        while i < self.active.len() {
            let a = &mut self.active[i];
            if a.remaining > 0. {
                a.remaining -= time - a.last.unwrap_or(time);
            }
            a.last = Some(time);
            if self.kinds[self.placements[a.placement].kind].duration > 0. && a.remaining <= 0. {
                let removed = self.active.swap_remove(i);
                self.counters[self.placements[removed.placement].kind] =
                    self.counters[self.placements[removed.placement].kind].wrapping_add(1);
            } else {
                i += 1;
            }
        }
        self.active
            .iter()
            .map(|a| {
                let p = &self.placements[a.placement];
                let mut instance = p.instance;
                instance.animation_epoch = a.epoch;
                (p.slot, instance)
            })
            .collect()
    }
}

#[test]
fn placement_preserves_authored_height_scale_and_only_y_rotation() {
    content_reader_script::test_game::enter();
    let types = ambient::parse(b"type={type=mod_frame scale=2 always_visible=yes use_animation=no object={position={3 -4 5} rotation={90 90 90}}}", &content_reader_script::key_names::KeyNames::default()).unwrap();
    let value = instance(&types[0], &types[0].objects[0]);
    assert_eq!(value.position, [3., -4., 5.]);
    assert_eq!(value.scale, 2.);
    assert_eq!(value.rotation, f32::from_bits(0x3fc90fdb));
    assert_eq!((value.pitch, value.roll), (0., 0.));
    assert_eq!(value.animation, entity_animation::DEFAULT_STATE);
}

#[test]
fn buckets_use_first_inclusive_match_and_native_edge_extensions() {
    let b = buckets([405, 207]);
    assert_eq!(b.len(), 16);
    assert_eq!(b[0], [[-101., -100., -51.], [101., 1000., 51.]]);
    assert_eq!(b[15], [[303., -100., 153.], [506., 1000., 258.]]);
    assert_eq!(
        b.iter().position(|b| contains(*b, [101., 0., 51.])),
        Some(0)
    );
    assert!(!contains(b[0], [0., -100.01, 0.]));
    assert!(admits_box(b[0], 0., &[[-1., 0., 0., 101.]; 6]));
    assert!(!admits_box(b[0], 0., &[[-1., 0., 0., 102.]; 6]));
}

#[test]
fn ambient_buckets_keep_the_camera_side_margin() {
    use viewer_gpu::camera::{MapDefines, MapSize, Placement as CameraPlacement};

    let bounds = buckets([MapSize::TEST.width, MapSize::TEST.height])[5];
    for (distance, expected) in [(84., 1), (104., 0)] {
        let camera = CameraPlacement::map([bounds[0][0] - distance, bounds[1][2]], 50.)
            .on(MapSize::TEST, MapDefines::test());
        let mut presentation = Presentation {
            placements: vec![Placement {
                kind: 0,
                slot: 0,
                instance: ModelInstance::default(),
                bucket: Some(0),
            }],
            kinds: vec![Kind {
                duration: 0.,
                max_visible: -1,
            }],
            bounds: vec![bounds],
            counters: vec![0],
            ..Default::default()
        };
        assert_eq!(presentation.update(camera, [1024, 768], 0.).len(), expected);
    }
}

#[test]
fn nearest_ambient_height_clamps_and_uses_bottom_up_rows() {
    let image = IndexedImage {
        width: 2,
        height: 2,
        indices: vec![100, 200, 0, 150].into(),
        palette: [[0; 4]; 256],
    };
    assert_eq!(height(&image, [-100., 0., -100.]), 9.5);
    assert_eq!(height(&image, [0.49, 0., 0.49]), 9.5);
    assert_eq!(height(&image, [0.5, 0., 0.5]), 20.);
    assert_eq!(height(&image, [100., 0., 100.]), 20.);
}

#[test]
fn ambient_admission_expiry_and_recreation_preserve_native_order() {
    let placement = |slot, bucket| Placement {
        kind: slot,
        slot,
        bucket,
        instance: ModelInstance {
            scale: 1.,
            ..Default::default()
        },
    };
    let mut p = Presentation {
        placements: vec![
            placement(0, None),
            placement(1, Some(0)),
            placement(2, Some(1)),
        ],
        kinds: vec![
            Kind {
                duration: 0.,
                max_visible: -1,
            },
            Kind {
                duration: 2.,
                max_visible: 1,
            },
            Kind {
                duration: 0.,
                max_visible: -1,
            },
        ],
        bounds: buckets([40, 40]),
        counters: vec![0; 3],
        ..Default::default()
    };
    let mut visible = vec![true; 16];
    let slots =
        |rows: Vec<(usize, ModelInstance)>| rows.into_iter().map(|(s, _)| s).collect::<Vec<_>>();
    assert_eq!(slots(p.advance(10., &visible)), [1, 2, 0]);
    let old_epoch = p.active[0].epoch;
    visible[0] = false;
    assert_eq!(slots(p.advance(11., &visible)), [1, 2, 0]);
    assert_eq!(slots(p.advance(12., &visible)), [0, 2]);
    assert_eq!(p.counters, [0, 1, 0]);
    visible[0] = true;
    assert_eq!(slots(p.advance(20., &visible)), [0, 2, 1]);
    assert_ne!(p.active[2].epoch, old_epoch);
    assert_eq!(p.counters, [0, 0, 0]);
    assert_eq!(slots(p.advance(1., &visible)), [1, 2, 0]);
    p.active.retain(|a| a.placement != 1);
    p.counters[1] = 2;
    assert_eq!(slots(p.advance(1., &visible)), [2, 0]);
}

#[test]
fn event_only_ambient_uses_shared_player_scale_and_entry_clock() {
    content_reader_script::test_game::enter();
    let descriptors = content_reader_mesh::mesh::parse_descriptors(br#"entity={name=emitter scale=0.3 state={name=idle event={time=0 particle=mist keep_particle=yes}}}"#, "gfx/synthetic.asset").unwrap();
    let library = ModelLibrary::load(
        &ResolvedLayers::default(),
        Decode::default(),
        &descriptors,
        &["emitter".into()],
        &mut Vec::new(),
    )
    .unwrap();
    let slot = library.slot("emitter").unwrap();
    assert_eq!(library.meshes[slot].radius, 0.0);
    let placed = ModelInstance {
        position: [3., 4., 5.],
        scale: 4.,
        animation: entity_animation::AMBIENT_IDLE,
        animation_epoch: 1,
        ..Default::default()
    };
    let mut pass = viewer_models::ModelPass::new(&library, "map frame");
    pass.update_placed(vec![(slot, placed)], 256.);
    pass.animate(20., None, &[0.]);
    assert_eq!(pass.events.len(), 1);
    assert_eq!(pass.events[0].age, 0.);
    assert_eq!(pass.events[0].transform.translation, [3., 4., 5.]);
    assert!((pass.events[0].transform.uniform_scale() - 1.2).abs() < 1e-6);
    pass.animate(20.5, None, &[0.]);
    assert_eq!(pass.events[0].age, 0.5);
    pass.update_placed(
        vec![(
            slot,
            ModelInstance {
                animation: entity_animation::STOPPED,
                ..placed
            },
        )],
        256.,
    );
    pass.animate(21., None, &[0.]);
    assert!(pass.events.is_empty());
}

#[test]
fn a_still_ambient_object_plays_its_default_state_from_when_it_appears() {
    content_reader_script::test_game::enter();
    let descriptors = content_reader_mesh::mesh::parse_descriptors(
        br#"entity={name=wind default_state=idle state={name=idle state_time=1 event={trigger_once=yes sound={soundeffect=amb_wind}}}}
            entity={name=late default_state=idle state={name=idle looping=no event={time=5 sound={soundeffect=amb_late}}}}
            entity={name=twin default_state=idle state={name=idle chance=1 event={sound={soundeffect=amb_a}}} state={name=idle chance=0 event={sound={soundeffect=amb_b}}}}
            entity={name=zero default_state=idle state={name=idle chance=0 event={trigger_once=yes sound={soundeffect=amb_zero}}}}
            entity={name=animated default_state=idle state={name=idle chance=1 state_time=1 looping=no event={time=0.25 sound={soundeffect=amb_idle_a}}} state={name=idle chance=2 state_time=1 looping=no event={time=0.25 sound={soundeffect=amb_idle_b}}}}"#,
        "gfx/synthetic.asset",
    )
    .unwrap();
    let library = ModelLibrary::load(
        &ResolvedLayers::default(),
        Decode::default(),
        &descriptors,
        &[
            "wind".into(),
            "late".into(),
            "twin".into(),
            "zero".into(),
            "animated".into(),
        ],
        &mut Vec::new(),
    )
    .unwrap();
    let placed = |name: &str, identity: u32, animation: u32| {
        (
            library.slot(name).unwrap(),
            ModelInstance {
                position: [identity as f32, 0., 0.],
                scale: 1.,
                animation,
                animation_identity: identity,
                ..Default::default()
            },
        )
    };
    let still = entity_animation::DEFAULT_STATE;
    let neighbours: Vec<_> = (9..17)
        .map(|identity| {
            let mut neighbour = placed("animated", identity, entity_animation::AMBIENT_IDLE);
            neighbour.1.animation_seed = identity.wrapping_mul(0x9e37_79b9);
            neighbour
        })
        .collect();
    let mut all = vec![
        placed("wind", 1, still),
        placed("late", 2, still),
        placed("twin", 3, still),
        placed("zero", 4, still),
    ];
    all.extend(neighbours.iter().copied());
    let sounds_at = |pass: &mut viewer_models::ModelPass, time: f32| {
        pass.animate(time, None, &[0.]);
        let mut names = viewer_models::sound_events::Names::default();
        let mut sounds = Vec::new();
        viewer_models::sound_events::collect(&pass.events, &mut names, &mut sounds, |_| false);
        sounds
            .into_iter()
            .map(|sound| {
                (
                    sound.effect.to_string(),
                    sound.position,
                    sound.age,
                    sound.key,
                )
            })
            .collect::<Vec<_>>()
    };
    let mut pass = viewer_models::ModelPass::new(&library, "map frame");
    pass.update_placed(all.clone(), 256.);
    let mut alone = viewer_models::ModelPass::new(&library, "map frame");
    alone.update_placed(neighbours.clone(), 256.);
    let neighbour_keys: Vec<_> = neighbours
        .iter()
        .map(|(slot, instance)| {
            let (seed, identity) = instance.animation_key();
            (*slot, seed, identity)
        })
        .collect();
    let mut drawn = std::collections::BTreeSet::new();
    let mut compare = |pass: &mut viewer_models::ModelPass, time: f32| {
        let sounds = sounds_at(pass, time);
        let mut with = sounds
            .iter()
            .filter(|sound| sound.0.starts_with("amb_idle"))
            .map(|sound| (sound.0.clone(), sound.1, sound.2))
            .collect::<Vec<_>>();
        let mut without = sounds_at(&mut alone, time)
            .into_iter()
            .map(|sound| (sound.0, sound.1, sound.2))
            .collect::<Vec<_>>();
        let order = |a: &(String, [f32; 3], f32), b: &(String, [f32; 3], f32)| {
            a.1[0].total_cmp(&b.1[0]).then(a.0.cmp(&b.0))
        };
        with.sort_by(order);
        without.sort_by(order);
        assert_eq!(with, without, "at {time}");
        drawn.extend(with.into_iter().map(|sound| sound.0));
        for key in &neighbour_keys {
            assert_eq!(
                pass.root_play_position(*key),
                alone.root_play_position(*key),
                "at {time}"
            );
        }
        sounds
    };
    compare(&mut pass, 20.);
    for (slot, instance) in &all[..4] {
        let refused = instance.animation_identity == 3;
        assert_eq!(pass.player(*slot, instance).is_some(), !refused);
    }
    let sounds = compare(&mut pass, 20.5);
    let effects: Vec<_> = sounds.iter().map(|sound| sound.0.as_str()).collect();
    assert_eq!(effects[..2], ["amb_wind", "amb_zero"]);
    assert!(
        effects[2..]
            .iter()
            .all(|effect| effect.starts_with("amb_idle")),
        "{effects:?}"
    );
    assert_eq!(effects.len(), 2 + neighbours.len());
    assert_eq!(sounds[0].1, [1., 0., 0.]);
    assert!((sounds[0].2 - 0.5).abs() < 1e-3, "{}", sounds[0].2);
    let wind = sounds[0].3;
    assert_eq!(pass.default_state_refusals.len(), 1);
    let (refusal, count) = pass.default_state_refusals.iter().next().unwrap();
    assert!(
        refusal.starts_with("entity twin reaches state idle of 2 variants"),
        "{refusal}"
    );
    assert_eq!(*count, 1);
    for step in 1..11 {
        compare(&mut pass, 20.5 + step as f32 * 0.5);
    }
    let sounds = compare(&mut pass, 26.);
    let late = sounds
        .iter()
        .find(|sound| sound.0 == "amb_late")
        .expect("the timed event fires five seconds after the actor appeared");
    assert!((late.2 - 1.0).abs() < 1e-3, "{}", late.2);
    let still_wind = sounds.iter().find(|sound| sound.0 == "amb_wind").unwrap();
    assert_eq!(still_wind.3, wind);
    assert_eq!(drawn.len(), 2, "{drawn:?}");
}
