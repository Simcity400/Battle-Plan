use crate::particle_authored::ParticleInputRefusal;
use crate::particle_runtime::ParticleLightBinding;
use crate::production_features::xmap09::Xmap09PointLight;
use crate::xmap09_light_template::{Xmap09LightInstanceClock, Xmap09LightTemplate};
use crate::xmap09_lighting_law::{
    XMAP09_LIGHTS_PER_CELL, XMAP09_LIGHTS_PER_FRAME, XMAP09_LIGHT_CELL_CHANNEL_ORDER,
};
use crate::{BuildingEntityEvent, Xmap09PointLightCreatorReach, XMAP09_POINT_LIGHT_CREATORS};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Xmap09LightCensusRefusal {
    ParticleInput {
        instance: String,
        refusal: ParticleInputRefusal,
    },
    MissingDomain {
        domain: String,
    },
    InvalidJoin {
        instance: String,
    },
    InvalidValue {
        instance: String,
    },
    PairedParticleRuntimeUnavailable {
        entity: String,
        event_index: usize,
        particle: String,
    },
}

impl std::fmt::Display for Xmap09LightCensusRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParticleInput { instance, refusal } => write!(f, "{instance}: {refusal}"),
            Self::PairedParticleRuntimeUnavailable {
                entity,
                event_index,
                particle,
            } => {
                write!(f, "XMAP09_PAIRED_PARTICLE_RUNTIME_UNAVAILABLE: {entity} event {event_index} particle {particle}")
            }
            Self::MissingDomain { domain } => {
                write!(f, "XMAP09_LIGHT_CENSUS_DOMAIN_UNAVAILABLE: {domain}")
            }
            Self::InvalidJoin { instance } => {
                write!(f, "XMAP09_LIGHT_CENSUS_JOIN_INVALID: {instance}")
            }
            Self::InvalidValue { instance } => {
                write!(f, "XMAP09_LIGHT_CENSUS_VALUE_INVALID: {instance}")
            }
        }
    }
}
impl std::error::Error for Xmap09LightCensusRefusal {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09LightCensusCounts {
    pub creator_sites: usize,
    pub placed_entities: usize,
    pub authored_light_events: usize,
    pub fired_instances: usize,
    pub admitted_instances: usize,
    pub uploaded_lights: usize,
    pub paired_fired_instances: usize,
    pub paired_admitted_instances: usize,
    pub paired_uploaded_lights: usize,
    pub fired_by_creator: BTreeMap<u32, usize>,
}

impl Xmap09LightCensusCounts {
    pub(crate) fn is_consistent(&self) -> bool {
        self.creator_sites == XMAP09_POINT_LIGHT_CREATORS.len()
            && self.fired_by_creator.len() == self.creator_sites
            && XMAP09_POINT_LIGHT_CREATORS.iter().all(|creator| {
                self.fired_by_creator
                    .get(&creator.rva)
                    .is_some_and(|count| {
                        matches!(
                            creator.reach,
                            Xmap09PointLightCreatorReach::EntityAnimationRuntime
                        ) || *count == 0
                    })
            })
            && self
                .fired_by_creator
                .values()
                .try_fold(0usize, |sum, n| sum.checked_add(*n))
                == Some(self.fired_instances)
            && self.admitted_instances <= self.fired_instances
            && self.uploaded_lights <= self.admitted_instances
            && self.paired_fired_instances <= self.fired_instances
            && self.paired_admitted_instances <= self.paired_fired_instances
            && self.paired_admitted_instances <= self.admitted_instances
            && self.paired_uploaded_lights <= self.paired_admitted_instances
            && self.paired_uploaded_lights <= self.uploaded_lights
            && self.uploaded_lights <= XMAP09_LIGHTS_PER_FRAME
            && (self.fired_instances == 0
                || (self.placed_entities > 0 && self.authored_light_events > 0))
    }
}

#[derive(Clone)]
pub struct Xmap09LightEntity<'a> {
    pub id: &'a str,
    pub scene_mask: u32,
    pub events: &'a [BuildingEntityEvent],
    pub placement: Option<[f32; 16]>,
}

#[derive(Clone)]
pub struct Xmap09FiredLightInstance<'a> {
    pub id: &'a str,
    pub entity_id: &'a str,
    pub event_index: usize,
    pub creator_rva: u32,
    pub clock: Option<&'a Xmap09LightInstanceClock>,
    pub attach_transform: Option<[f32; 16]>,
    pub paired_particle: Option<ParticleLightBinding>,
    pub enabled: Option<bool>,
}

pub struct Xmap09LightCensusRows<'a> {
    pub resolve_attach: Option<
        &'a dyn Fn(&Xmap09FiredLightInstance<'_>) -> Result<[f32; 16], Xmap09LightCensusRefusal>,
    >,
    pub entities: &'a [Xmap09LightEntity<'a>],
    pub templates: &'a [Xmap09LightTemplate],
    pub fired_instances: &'a [Xmap09FiredLightInstance<'a>],
}

pub struct Xmap09AdmittedLightCensus {
    pub counts: Xmap09LightCensusCounts,
    pub lights: Vec<Xmap09PointLight>,
    pub cells: Vec<[u8; XMAP09_LIGHTS_PER_CELL]>,
}

fn missing(domain: &str) -> Xmap09LightCensusRefusal {
    Xmap09LightCensusRefusal::MissingDomain {
        domain: domain.into(),
    }
}

pub fn admit_light_census(
    source: Result<Xmap09LightCensusRows<'_>, Xmap09LightCensusRefusal>,
    grid: [f32; 4],
) -> Result<Xmap09AdmittedLightCensus, Xmap09LightCensusRefusal> {
    let rows = source?;
    if grid.iter().any(|v| !v.is_finite())
        || grid[2..] != [0.125, 0.125]
        || grid[..2]
            .iter()
            .any(|v| v / 8.0 != (v / 8.0).trunc() || v.abs() >= (i32::MAX / 4) as f32)
    {
        return Err(Xmap09LightCensusRefusal::InvalidValue {
            instance: "grid".into(),
        });
    }
    let invalid = |id: &str| Xmap09LightCensusRefusal::InvalidJoin {
        instance: id.into(),
    };
    let mut entities = BTreeMap::new();
    let mut templates = BTreeMap::new();
    for template in rows.templates {
        if templates.insert(template.name.as_str(), template).is_some()
            || !template.duration.is_finite()
        {
            return Err(invalid(&template.name));
        }
    }
    let mut authored_light_events = 0;
    for entity in rows.entities {
        if entity.id.is_empty() || entities.insert(entity.id, entity).is_some() {
            return Err(invalid(entity.id));
        }
        for event in entity.events {
            if let Some(light) = event.light.as_deref() {
                if !templates.contains_key(light)
                    || event
                        .time
                        .is_none_or(|time| !time.is_finite() || time < 0.0)
                {
                    return Err(invalid(entity.id));
                }
                authored_light_events += 1;
            }
        }
    }
    let mut counts = Xmap09LightCensusCounts {
        creator_sites: XMAP09_POINT_LIGHT_CREATORS.len(),
        placed_entities: rows.entities.len(),
        authored_light_events,
        fired_instances: rows.fired_instances.len(),
        admitted_instances: 0,
        uploaded_lights: 0,
        paired_fired_instances: 0,
        paired_admitted_instances: 0,
        paired_uploaded_lights: 0,
        fired_by_creator: XMAP09_POINT_LIGHT_CREATORS
            .iter()
            .map(|c| (c.rva, 0))
            .collect(),
    };
    let mut seen = BTreeSet::new();
    let mut lights = Vec::new();
    let mut cells = vec![[u8::MAX; XMAP09_LIGHTS_PER_CELL]; 64 * 64];
    for instance in rows.fired_instances {
        if instance.id.is_empty() || !seen.insert(instance.id) {
            return Err(invalid(instance.id));
        }
        let creator = XMAP09_POINT_LIGHT_CREATORS
            .iter()
            .find(|c| c.rva == instance.creator_rva)
            .ok_or_else(|| invalid(instance.id))?;
        if !matches!(
            creator.reach,
            Xmap09PointLightCreatorReach::EntityAnimationRuntime
        ) {
            return Err(invalid(instance.id));
        }
        let entity = entities
            .get(instance.entity_id)
            .ok_or_else(|| invalid(instance.id))?;
        let event = entity
            .events
            .get(instance.event_index)
            .ok_or_else(|| invalid(instance.id))?;
        let paired = event.particle.as_ref().filter(|name| !name.is_empty());
        if let Some(particle) = paired.filter(|_| instance.paired_particle.is_none()) {
            return Err(Xmap09LightCensusRefusal::PairedParticleRuntimeUnavailable {
                entity: entity.id.into(),
                event_index: instance.event_index,
                particle: particle.clone(),
            });
        }
        let template = event
            .light
            .as_deref()
            .and_then(|light| templates.get(light))
            .ok_or_else(|| invalid(instance.id))?;
        *counts.fired_by_creator.get_mut(&creator.rva).unwrap() += 1;
        counts.paired_fired_instances += usize::from(paired.is_some());
        if entity.scene_mask & 6 == 0 || instance.enabled == Some(false) {
            continue;
        }
        if template.duration >= 0.0 {
            let clock = instance
                .clock
                .ok_or_else(|| missing("lighting.instance_renderer_clock"))?;
            if !template.admits_instance(clock, entity.scene_mask, true) {
                continue;
            }
        }
        instance
            .enabled
            .ok_or_else(|| missing("lighting.instance_enabled"))?;
        counts.admitted_instances += 1;
        counts.paired_admitted_instances += usize::from(paired.is_some());
        if lights.len() == XMAP09_LIGHTS_PER_FRAME {
            continue;
        }
        let clock = instance
            .clock
            .ok_or_else(|| missing("lighting.instance_renderer_clock"))?;
        let attach = if paired.is_some() {
            let bits = instance
                .paired_particle
                .as_ref()
                .unwrap()
                .world_matrix_f32_bits;
            std::array::from_fn(|i| f32::from_bits(bits[(i % 4) * 4 + i / 4]))
        } else {
            match instance.attach_transform {
                Some(attach) => attach,
                None => rows
                    .resolve_attach
                    .ok_or_else(|| missing("lighting.instance_attach_transform"))?(
                    instance
                )?,
            }
        };
        let placement = if paired.is_some() {
            [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ]
        } else {
            entity
                .placement
                .ok_or_else(|| missing("lighting.placed_entity_transform"))?
        };
        if placement.iter().any(|value| !value.is_finite()) {
            return Err(Xmap09LightCensusRefusal::InvalidValue {
                instance: instance.id.into(),
            });
        }
        let values = template.values_at_frame(clock).map_err(|_| {
            Xmap09LightCensusRefusal::InvalidValue {
                instance: instance.id.into(),
            }
        })?;
        let position = crate::xmap09_light_world_position(placement, attach, values.position)
            .ok_or_else(|| Xmap09LightCensusRefusal::InvalidValue {
                instance: instance.id.into(),
            })?;
        let colour = values.color.map(|v| v * values.intensity);
        if colour.iter().any(|v| !v.is_finite()) {
            return Err(Xmap09LightCensusRefusal::InvalidValue {
                instance: instance.id.into(),
            });
        }
        if insert_light_cells(
            &mut cells,
            grid,
            position,
            values.radius,
            lights.len() as u8,
        )? {
            counts.paired_uploaded_lights += usize::from(paired.is_some());
            let bits = |v: [f32; 4]| v.map(f32::to_bits);
            lights.push(Xmap09PointLight {
                stable_light_id: instance.id.into(),
                global_order: lights.len() as u32,
                position_radius_f32_bits: bits([
                    position[0],
                    position[1],
                    position[2],
                    values.radius,
                ]),
                color_falloff_f32_bits: bits([colour[0], colour[1], colour[2], values.falloff]),
            });
        }
    }
    counts.uploaded_lights = lights.len();
    Ok(Xmap09AdmittedLightCensus {
        counts,
        lights,
        cells,
    })
}

fn insert_light_cells(
    cells: &mut [[u8; 4]],
    grid: [f32; 4],
    position: [f32; 3],
    radius: f32,
    slot: u8,
) -> Result<bool, Xmap09LightCensusRefusal> {
    let [x, _, z] = position;
    if x - radius > grid[0] + 512.0
        || z - radius > grid[1] + 512.0
        || grid[0] > x + radius
        || grid[1] > z + radius
    {
        return Ok(false);
    }
    let bounds = [
        (x - radius) * 0.125,
        (z - radius) * 0.125,
        (x + radius) * -0.125,
        (z + radius) * -0.125,
    ];
    if bounds
        .iter()
        .any(|v| !v.is_finite() || v.abs() >= (i32::MAX / 4) as f32)
    {
        return Err(Xmap09LightCensusRefusal::InvalidValue {
            instance: "cell_bounds".into(),
        });
    }
    let start_x = (grid[0] * 0.125) as i32;
    let start_z = (grid[1] * 0.125) as i32;
    let mut used = false;
    for cell_z in start_z.max(bounds[1] as i32)..(start_z + 64).min(1 - bounds[3] as i32) {
        for cell_x in start_x.max(bounds[0] as i32)..(start_x + 64).min(1 - bounds[2] as i32) {
            let cell = &mut cells[((cell_z - start_z) * 64 + cell_x - start_x) as usize];
            if cell[3] != u8::MAX {
                continue;
            }
            let dx = (cell_x as f32 * 8.0 + 4.0) - x;
            let dz = (cell_z as f32 * 8.0 + 4.0) - z;
            if (dz * dz + dx * dx).sqrt() - 32.0_f32.sqrt() <= radius {
                for channel in XMAP09_LIGHT_CELL_CHANNEL_ORDER {
                    if cell[channel] == u8::MAX {
                        cell[channel] = slot;
                        used = true;
                        break;
                    }
                }
            }
        }
    }
    Ok(used)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    const GRID: [f32; 4] = [0.0, 0.0, 0.125, 0.125];

    pub(crate) fn with_rows<T>(
        positions: &[[f32; 3]],
        test: impl FnOnce(Xmap09LightCensusRows<'_>) -> T,
    ) -> T {
        let templates = crate::parse_light_templates("light = {\nname = lamp\ncolor = { r = 1 g = 0.5 b = 0.25 }\nintensity = 2\nradius = 1\nfalloff = 3\nposition = { x = 0 y = 0 z = 0 }\n}", "test.asset").unwrap();
        let events = [BuildingEntityEvent {
            particle: None,
            keep_particle: false,
            entity: None,
            life: None,
            time: Some(0.0),
            node: Some("lamp".into()),
            light: Some("lamp".into()),
            trigger_once: false,
        }];
        let ids = (0..positions.len())
            .map(|i| format!("entity-{i}"))
            .collect::<Vec<_>>();
        let entities = ids
            .iter()
            .zip(positions)
            .map(|(id, position)| {
                let mut placement = IDENTITY;
                placement[3] = position[0];
                placement[7] = position[1];
                placement[11] = position[2];
                Xmap09LightEntity {
                    id,
                    scene_mask: 6,
                    events: &events,
                    placement: Some(placement),
                }
            })
            .collect::<Vec<_>>();
        let clock = Xmap09LightInstanceClock::default();
        let creators = XMAP09_POINT_LIGHT_CREATORS
            .iter()
            .filter(|creator| {
                matches!(
                    creator.reach,
                    Xmap09PointLightCreatorReach::EntityAnimationRuntime
                )
            })
            .collect::<Vec<_>>();
        let instances = ids
            .iter()
            .enumerate()
            .map(|(i, id)| Xmap09FiredLightInstance {
                id,
                entity_id: id,
                event_index: 0,
                creator_rva: creators[i % creators.len()].rva,
                clock: Some(&clock),
                attach_transform: Some(IDENTITY),
                paired_particle: None,
                enabled: Some(true),
            })
            .collect::<Vec<_>>();
        test(Xmap09LightCensusRows {
            resolve_attach: None,
            entities: &entities,
            templates: &templates,
            fired_instances: &instances,
        })
    }

    #[test]
    fn paired_counters_follow_disabled_filter_and_global_upload_cap() {
        let positions = (0..66)
            .map(|i| {
                [
                    4.0 + (i % 16) as f32 * 24.0,
                    2.0,
                    4.0 + (i / 16) as f32 * 24.0,
                ]
            })
            .collect::<Vec<_>>();
        with_rows(&positions, |rows| {
            let mut events = rows.entities[0].events.to_vec();
            events[0].particle = Some("paired".into());
            let mut entities = rows.entities.to_vec();
            let mut instances = rows.fired_instances.to_vec();
            for (index, (entity, instance)) in entities.iter_mut().zip(&mut instances).enumerate() {
                entity.events = &events;
                let matrix = entity.placement.unwrap();
                instance.paired_particle = Some(ParticleLightBinding {
                    event_instance_id: index as u64,
                    presentation_id: None,
                    finished: false,
                    world_matrix_f32_bits: std::array::from_fn(|i| {
                        matrix[(i % 4) * 4 + i / 4].to_bits()
                    }),
                });
            }
            instances[0].enabled = Some(false);
            let census = admit_light_census(
                Ok(Xmap09LightCensusRows {
                    entities: &entities,
                    fired_instances: &instances,
                    ..rows
                }),
                GRID,
            )
            .unwrap();
            assert_eq!(census.counts.paired_fired_instances, 66);
            assert_eq!(census.counts.paired_admitted_instances, 65);
            assert_eq!(census.counts.paired_uploaded_lights, 64);
            assert!(census.counts.is_consistent());
        });
    }

    #[test]
    fn paired_light_uses_runtime_world_bits_without_reapplying_entity_placement() {
        with_rows(&[[400., 0., 400.]], |rows| {
            let mut events = rows.entities[0].events.to_vec();
            events[0].particle = Some("paired".into());
            let mut entities = rows.entities.to_vec();
            entities[0].events = &events;
            let mut instances = rows.fired_instances.to_vec();
            let mut world = IDENTITY.map(f32::to_bits);
            world[12] = 16f32.to_bits();
            world[13] = 2f32.to_bits();
            world[14] = 24f32.to_bits();
            instances[0].attach_transform = None;
            instances[0].paired_particle = Some(ParticleLightBinding {
                event_instance_id: 3,
                presentation_id: Some(2),
                world_matrix_f32_bits: world,
                finished: false,
            });
            let census = admit_light_census(
                Ok(Xmap09LightCensusRows {
                    entities: &entities,
                    fired_instances: &instances,
                    ..rows
                }),
                GRID,
            )
            .unwrap();
            assert_eq!(census.lights.len(), 1);
            assert_eq!(census.counts.paired_fired_instances, 1);
            assert_eq!(census.counts.paired_admitted_instances, 1);
            assert_eq!(census.counts.paired_uploaded_lights, 1);
            assert!(census.counts.is_consistent());
            assert_eq!(
                census.lights[0]
                    .position_radius_f32_bits
                    .map(f32::from_bits),
                [16., 2., 24., 1.]
            );
        });
    }

    #[test]
    fn paired_particle_light_refuses_only_after_its_event_fires() {
        with_rows(&[[0.0; 3]], |rows| {
            let mut events = rows.entities[0].events.to_vec();
            events[0].particle = Some("muzzle_smoke".into());
            let mut entities = rows.entities.to_vec();
            entities[0].events = &events;
            let unfired = admit_light_census(
                Ok(Xmap09LightCensusRows {
                    entities: &entities,
                    templates: rows.templates,
                    fired_instances: &[],
                    resolve_attach: None,
                }),
                GRID,
            )
            .unwrap();
            assert_eq!(unfired.counts.fired_instances, 0);
            assert!(matches!(admit_light_census(Ok(Xmap09LightCensusRows {
                entities: &entities,
                ..rows
            }), GRID), Err(Xmap09LightCensusRefusal::PairedParticleRuntimeUnavailable {
                entity, event_index: 0, particle
            }) if entity == "entity-0" && particle == "muzzle_smoke"));
        });
    }

    #[test]
    fn light_census_missing_rows_are_not_measured_empty() {
        assert!(
            matches!(admit_light_census(Err(missing("XMAP12.placed_entities")), GRID),
            Err(Xmap09LightCensusRefusal::MissingDomain { domain }) if domain == "XMAP12.placed_entities")
        );
        with_rows(&[], |rows| {
            let census = admit_light_census(Ok(rows), GRID).unwrap();
            assert_eq!(census.counts.fired_instances, 0);
            assert_eq!(census.counts.creator_sites, 4);
            assert_eq!(census.counts.fired_by_creator.len(), 4);
            assert!(census.counts.is_consistent());
            assert!(census.lights.is_empty());
            assert!(census.cells.iter().all(|cell| *cell == [255; 4]));
        });
    }

    #[test]
    fn light_census_counts_all_creators_entities_events_and_fires_before_the_64_upload_cap() {
        let positions = (0..65)
            .map(|i| {
                [
                    4.0 + (i % 16) as f32 * 24.0,
                    2.0,
                    4.0 + (i / 16) as f32 * 24.0,
                ]
            })
            .collect::<Vec<_>>();
        with_rows(&positions, |rows| {
            let census = admit_light_census(Ok(rows), GRID).unwrap();
            assert_eq!(
                (
                    census.counts.placed_entities,
                    census.counts.authored_light_events,
                    census.counts.fired_instances,
                    census.counts.admitted_instances,
                    census.counts.uploaded_lights
                ),
                (65, 65, 65, 65, 64)
            );
            assert!(census.counts.is_consistent());
            assert_eq!(census.counts.fired_by_creator.values().sum::<usize>(), 65);
            assert_eq!(census.lights.last().unwrap().stable_light_id, "entity-63");
            assert!(census
                .cells
                .iter()
                .flatten()
                .all(|slot| *slot == 255 || *slot < 64));
            assert_eq!(
                census.lights[0].color_falloff_f32_bits.map(f32::from_bits),
                [2.0, 1.0, 0.5, 3.0]
            );
        });
    }

    #[test]
    fn light_census_four_cell_slots_use_engine_channel_order_and_do_not_consume_rejected_lights() {
        with_rows(&[[4.0, 0.0, 4.0]; 5], |rows| {
            let census = admit_light_census(Ok(rows), GRID).unwrap();
            assert_eq!(census.counts.admitted_instances, 5);
            assert_eq!(census.counts.uploaded_lights, 4);
            assert_eq!(census.cells[0], [2, 1, 0, 3]);
            assert!(census.cells[1..].iter().all(|cell| *cell == [255; 4]));
        });
        with_rows(&[[-100.0, 0.0, -100.0], [4.0, 0.0, 4.0]], |rows| {
            let census = admit_light_census(Ok(rows), GRID).unwrap();
            assert_eq!(census.counts.admitted_instances, 2);
            assert_eq!(census.counts.uploaded_lights, 1);
            assert_eq!(census.lights[0].stable_light_id, "entity-1");
            assert_eq!(census.lights[0].global_order, 0);
            assert_eq!(census.cells[0], [255, 255, 0, 255]);
        });
    }

    #[test]
    fn light_census_age_mask_and_enabled_filters_precede_cell_admission() {
        with_rows(&[[4.0, 0.0, 4.0]; 4], |rows| {
            let mut templates = rows.templates.to_vec();
            templates[0].duration = 1.0;
            let mut clock = Xmap09LightInstanceClock::default();
            clock.advance(1.0, true).unwrap();
            let mut expired = clock.clone();
            expired.advance(0.001, true).unwrap();
            let mut instances = rows.fired_instances.to_vec();
            instances[0].clock = Some(&clock);
            instances[1].clock = Some(&expired);
            instances[2].enabled = Some(false);
            let mut entities = rows.entities.to_vec();
            entities[3].scene_mask = 1;
            let census = admit_light_census(
                Ok(Xmap09LightCensusRows {
                    resolve_attach: None,
                    templates: &templates,
                    fired_instances: &instances,
                    entities: &entities,
                }),
                GRID,
            )
            .unwrap();
            assert_eq!(census.counts.fired_instances, 4);
            assert_eq!(census.counts.admitted_instances, 1);
            assert_eq!(census.lights[0].stable_light_id, "entity-0");
        });
    }

    #[test]
    fn light_census_missing_instance_inputs_name_the_domain() {
        for domain in [
            "lighting.instance_renderer_clock",
            "lighting.instance_attach_transform",
            "lighting.instance_enabled",
            "lighting.placed_entity_transform",
        ] {
            with_rows(&[[4.0, 0.0, 4.0]], |rows| {
                let mut instances = rows.fired_instances.to_vec();
                let mut entities = rows.entities.to_vec();
                match domain {
                    "lighting.instance_renderer_clock" => instances[0].clock = None,
                    "lighting.instance_attach_transform" => instances[0].attach_transform = None,
                    "lighting.instance_enabled" => instances[0].enabled = None,
                    _ => entities[0].placement = None,
                }
                let result = admit_light_census(
                    Ok(Xmap09LightCensusRows {
                        resolve_attach: None,
                        entities: &entities,
                        templates: rows.templates,
                        fired_instances: &instances,
                    }),
                    GRID,
                );
                assert!(
                    matches!(result, Err(Xmap09LightCensusRefusal::MissingDomain { domain: actual }) if actual == domain)
                );
            });
        }
    }

    #[test]
    fn light_census_excluded_instances_do_not_require_unused_inputs() {
        for exclusion in 0..3 {
            for invalid_placement in [false, true] {
                with_rows(&[[4.0, 0.0, 4.0]; 2], |rows| {
                    let mut templates = rows.templates.to_vec();
                    templates[0].duration = 1.0;
                    let mut expired = Xmap09LightInstanceClock::default();
                    expired.advance(2.0, true).unwrap();
                    let mut instances = rows.fired_instances.to_vec();
                    let mut entities = rows.entities.to_vec();
                    instances[1].attach_transform = None;
                    entities[1].placement = invalid_placement.then_some([f32::NAN; 16]);
                    match exclusion {
                        0 => {
                            instances[1].enabled = Some(false);
                            instances[1].clock = None;
                        }
                        1 => {
                            entities[1].scene_mask = 1;
                            instances[1].enabled = None;
                            instances[1].clock = None;
                        }
                        _ => {
                            instances[1].clock = Some(&expired);
                            instances[1].enabled = None;
                        }
                    }
                    let census = admit_light_census(
                        Ok(Xmap09LightCensusRows {
                            resolve_attach: None,
                            entities: &entities,
                            templates: &templates,
                            fired_instances: &instances,
                        }),
                        GRID,
                    )
                    .unwrap();
                    assert_eq!(census.counts.fired_instances, 2);
                    assert_eq!(census.counts.admitted_instances, 1);
                    assert_eq!(census.counts.uploaded_lights, 1);
                    assert!(census.counts.is_consistent());
                    assert_eq!(census.lights[0].stable_light_id, "entity-0");
                    assert_eq!(census.cells[0], [255, 255, 0, 255]);
                });
            }
        }
    }

    #[test]
    fn light_census_unused_entities_and_capped_instances_need_no_transform() {
        let positions = (0..65)
            .map(|i| {
                [
                    4.0 + (i % 16) as f32 * 24.0,
                    2.0,
                    4.0 + (i / 16) as f32 * 24.0,
                ]
            })
            .collect::<Vec<_>>();
        with_rows(&positions, |rows| {
            let mut entities = rows.entities.to_vec();
            let mut instances = rows.fired_instances.to_vec();
            entities[64].placement = None;
            instances[64].attach_transform = None;
            instances[64].clock = None;
            for fired in [&instances[..64], &instances[..]] {
                let census = admit_light_census(
                    Ok(Xmap09LightCensusRows {
                        resolve_attach: None,
                        entities: &entities,
                        templates: rows.templates,
                        fired_instances: fired,
                    }),
                    GRID,
                )
                .unwrap();
                assert_eq!(census.counts.placed_entities, 65);
                assert_eq!(census.counts.fired_instances, fired.len());
                assert_eq!(census.counts.admitted_instances, fired.len());
                assert_eq!(census.counts.uploaded_lights, 64);
                assert!(census.counts.is_consistent());
                assert_eq!(census.lights[63].stable_light_id, "entity-63");
            }
        });
    }

    #[test]
    fn light_census_rejects_unjoined_duplicate_and_gated_creator_instances() {
        for case in 0..4 {
            with_rows(&[[4.0, 0.0, 4.0]; 2], |rows| {
                let mut instances = rows.fired_instances.to_vec();
                match case {
                    0 => instances[1].id = instances[0].id,
                    1 => instances[0].entity_id = "missing",
                    2 => instances[0].event_index = 100,
                    _ => instances[0].creator_rva = 0x0163_f450,
                }
                assert!(matches!(
                    admit_light_census(
                        Ok(Xmap09LightCensusRows {
                            resolve_attach: None,
                            fired_instances: &instances,
                            ..rows
                        }),
                        GRID
                    ),
                    Err(Xmap09LightCensusRefusal::InvalidJoin { .. })
                ));
            });
        }
    }

    #[test]
    fn light_census_cell_geometry_uses_radius_and_the_clipped_64_by_64_window() {
        let mut cells = vec![[255; 4]; 64 * 64];
        assert!(insert_light_cells(&mut cells, GRID, [508.0, 0.0, 508.0], 1.0, 0).unwrap());
        assert_eq!(cells[4095], [255, 255, 0, 255]);
        assert!(!insert_light_cells(&mut cells, GRID, [520.0, 0.0, 520.0], 1.0, 1).unwrap());
        assert!(insert_light_cells(&mut cells, GRID, [8.0, 0.0, 8.0], 1.0, 1).unwrap());
        for index in [0, 1, 64, 65] {
            assert_eq!(cells[index], [255, 255, 1, 255]);
        }
        assert_eq!(cells.iter().filter(|cell| **cell != [255; 4]).count(), 5);
    }

    #[test]
    fn light_census_preserves_finite_zero_radius_and_falloff() {
        with_rows(&[[4.0, 0.0, 4.0]], |rows| {
            let mut templates = rows.templates.to_vec();
            templates[0].radius.base = 0.0;
            templates[0].falloff.base = 0.0;
            let census = admit_light_census(
                Ok(Xmap09LightCensusRows {
                    resolve_attach: None,
                    templates: &templates,
                    ..rows
                }),
                GRID,
            )
            .unwrap();
            assert_eq!(census.counts.uploaded_lights, 1);
            assert_eq!(census.lights[0].position_radius_f32_bits[3], 0);
            assert_eq!(census.lights[0].color_falloff_f32_bits[3], 0);
        });
    }
}
