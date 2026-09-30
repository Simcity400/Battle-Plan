//! Ambient actor admission140B49340, placement141633D00 and lifetime141634900.
use super::{
    camera::Camera,
    entity_animation,
    models::{ModelField, ModelInstance, ModelLibrary},
};
use installed_content::{ResolvedLayers, ambient, image::IndexedImage};
use std::{collections::BTreeSet, io};

pub struct FrameAssets {
    pub library: ModelLibrary,
    pub field: ModelField,
    pub presentation: Presentation,
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

///Presentation starts at observation; no recorded native process clock or RNG.
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
    pub fn load(layers: &ResolvedLayers, warnings: &mut Vec<String>) -> io::Result<Self> {
        let types = ambient::read(layers)?;
        let dlcs = layers.dlc_names()?;
        let roster = layers.descriptor_roster()?;
        warnings.extend(
            roster
                .unreadable
                .iter()
                .map(|e| format!("Map frame descriptor unavailable: {e}")),
        );
        let default = installed_content::terrain::read_default_map(layers)?;
        let heights = installed_content::image::bmp::decode_indexed(
            &layers.read(&format!("map/{}", default.heightmap))?,
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
        let mut mesh_names = Vec::new();
        let mut event_names = BTreeSet::new();
        for (kind_index, kind) in types.iter().enumerate() {
            let condition = kind
                .dlc
                .as_ref()
                .map(|items| {
                    super::equipment_graphics::empty_scope_condition(items, |name| {
                        Some(dlcs.contains(name))
                    })
                })
                .transpose();
            if matches!(condition, Ok(Some(false))) {
                continue;
            }
            let condition_error = condition.err().map(|e| e.to_string());
            let inherited = roster.descriptors.inherit(&kind.entity);
            let mesh = roster.descriptors.resolve(&kind.entity).is_some();
            let reason = if let Some(error) = condition_error.as_deref() {
                Some(error)
            } else if roster.ambiguous.contains(&kind.entity)
                || roster.unordered_clones.contains(&kind.entity)
            {
                Some("ambiguous descriptor or unordered clone")
            } else if !mesh && inherited.is_none() {
                Some("no entity or mesh declaration")
            } else if !mesh
                && inherited
                    .as_ref()
                    .is_some_and(|e| !e.attachments.is_empty())
            {
                Some("meshless root with attachments has no native attachment transform")
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
                if mesh {
                    mesh_names.push(kind.entity.clone());
                } else {
                    event_names.insert(kind.entity.clone());
                }
            }
        }
        let mut library = ModelLibrary::load(layers, &roster.descriptors, &mesh_names, warnings)?;
        for name in event_names {
            library.add_event_entity(layers, &roster.descriptors, &name)?;
        }
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
            warnings.push("Ambient animation and particle clocks start at viewer observation; native historical populations, RNG and process-clock phase are not recorded. Ambient sound playback is unavailable.".into());
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
        })
    }
}

fn instance(kind: &ambient::AmbientObjectType, object: &ambient::AmbientObject) -> ModelInstance {
    //141633DB5 divides by360 then multiplies TAU40C90FDB; only negative-Y rotation.
    ModelInstance {
        position: object.position,
        rotation: object.rotation[1] / 360. * f32::from_bits(0x40c90fdb),
        scale: kind.scale,
        animation: if kind.use_animation {
            entity_animation::AMBIENT_IDLE
        } else {
            entity_animation::STOPPED
        },
        ..Default::default()
    }
}

///140B46EC0: 4x4 quarter-map boxes, extended one quarter outside each outer edge.
///141634490 assigns the first inclusive box containing the authored position.
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
fn in_view([low, high]: [[f32; 3]; 2], planes: &[[f32; 4]; 4], offset: f32) -> bool {
    planes.iter().all(|plane| {
        let p: [f32; 3] = std::array::from_fn(|i| if plane[i] < 0. { low[i] } else { high[i] });
        (p[0] + offset) * plane[0] + p[1] * plane[1] + p[2] * plane[2] + plane[3] >= 0.
    })
}
///141633DD7 passes clampSea=true to140A51CC0: nearest sample, bottom-up, no wrap.
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
        let planes = camera.side_planes(size);
        let visible: Vec<_> = self
            .bounds
            .iter()
            .map(|b| {
                [0., -camera.map.x(), camera.map.x()]
                    .iter()
                    .any(|o| in_view(*b, &planes, *o))
            })
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
        //140B49340 visits the sixteen bucket banks, then the always-queued list.
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
                //141634330's comparison is counter>max, not an active-count ceiling.
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
        //141634900 first captures the clock, then subtracts elapsed process time.
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
    let types = ambient::parse(b"type={type=mod_frame scale=2 always_visible=yes use_animation=no object={position={3 -4 5} rotation={90 90 90}}}").unwrap();
    let value = instance(&types[0], &types[0].objects[0]);
    assert_eq!(value.position, [3., -4., 5.]);
    assert_eq!(value.scale, 2.);
    assert_eq!(value.rotation, f32::from_bits(0x3fc90fdb));
    assert_eq!((value.pitch, value.roll), (0., 0.));
    assert_eq!(value.animation, entity_animation::STOPPED);
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
    assert!(in_view(b[0], &[[1., 0., 0., -101.]; 4], 0.));
    assert!(!in_view(b[0], &[[1., 0., 0., -102.]; 4], 0.));
}

#[test]
fn nearest_ambient_height_clamps_and_uses_bottom_up_rows() {
    let image = IndexedImage {
        width: 2,
        height: 2,
        indices: vec![100, 200, 0, 150],
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
    visible[0] = false; //Leaving a bucket does not destroy its live actor.
    assert_eq!(slots(p.advance(11., &visible)), [1, 2, 0]);
    assert_eq!(slots(p.advance(12., &visible)), [0, 2]); //last-slot replacement
    assert_eq!(p.counters, [0, 1, 0]);
    visible[0] = true;
    assert_eq!(slots(p.advance(20., &visible)), [0, 2, 1]); //first tick subtracts zero
    assert_ne!(p.active[2].epoch, old_epoch);
    assert_eq!(p.counters, [0, 0, 0]);
    assert_eq!(slots(p.advance(1., &visible)), [1, 2, 0]); //seek starts a fresh presentation
    p.active.retain(|a| a.placement != 1);
    p.counters[1] = 2; //The signed native counter comparison deliberately stays >.
    assert_eq!(slots(p.advance(1., &visible)), [2, 0]);
}

#[test]
fn event_only_ambient_uses_shared_player_scale_and_entry_clock() {
    let descriptors = installed_content::mesh::parse_descriptors(br#"entity={name=emitter scale=0.3 state={name=idle event={time=0 particle=mist keep_particle=yes}}}"#, "gfx/synthetic.asset").unwrap();
    let mut library = ModelLibrary::default();
    library
        .add_event_entity(&ResolvedLayers::default(), &descriptors, "emitter")
        .unwrap();
    let slot = library.slot("emitter").unwrap();
    assert!(library.meshes[slot].lods.is_empty());
    let placed = ModelInstance {
        position: [3., 4., 5.],
        scale: 4.,
        animation: entity_animation::AMBIENT_IDLE,
        animation_epoch: 1,
        ..Default::default()
    };
    let mut pass = super::models::ModelPass::new(&library, "map frame");
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
