
use std::{collections::BTreeMap, io, sync::Arc};

use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_map::map::MapData;
use content_reader_mesh::mesh;
use viewer_foundation::building_facts::{self, SiteSlots, SlotAdmission, admit_slot};
use viewer_gpu::province_map::ProvinceMap;

use super::Random;
use viewer_models::ModelInstance;

pub(super) const PASS_SECONDS: f32 = 2.0;
const SEED: u32 = 0x4c41_554e;
const INSTANCE_SEED: u32 = 0x5355_4e43;
const FIXED5: f32 = 100_000.0;
const UNIT: f32 = 1.0 / 2_147_483_648.0;
pub(super) const AIR_BASE: i32 = building_facts::AIR_BASE as i32;
pub(super) const ROCKET_SITE: i32 = building_facts::ROCKET_SITE as i32;
pub(super) const GUN_EMPLACEMENT: i32 = building_facts::GUN_EMPLACEMENT as i32;

pub(super) type Sites = BTreeMap<(u32, i32), Result<[f32; 3], &'static str>>;

pub(super) type ExplosionPlaces = Arc<Vec<(u32, Option<([f32; 3], [f32; 4])>)>>;

#[derive(Debug)]
pub(super) struct Content {
    sites: Result<Sites, String>,
    max_delay: Result<f32, &'static str>,
}

impl Content {
    pub(super) fn load(
        layers: &ResolvedLayers,
        map: &MapData,
        (defines, graphics): (&Defines, &Defines),
        roster: &mesh::Roster,
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
    ) -> Self {
        let sites = sites(layers, map, defines, roster).map_err(|error| error.to_string());
        let sites = match sites {
            Ok((sites, refused)) => {
                for (name, count) in refused {
                    warnings.push((
                        content_reader_parser::DiagnosticKind::Refused,
                        format!(
                            "Building slot {name} ({count} admitted records) is refused for state base positions: it names a building declared more than once differently, which could change where its state's base stands"
                        ),
                    ));
                }
                Ok(sites)
            }
            Err(reason) => Err(reason),
        };
        if let Err(reason) = &sites {
            warnings.push((
                content_reader_parser::DiagnosticKind::Refused,
                format!(
                    "State air bases, rocket sites and gun emplacements have no position, so their flights and launch effects are not drawn: the building sites do not read: {reason}"
                ),
            ));
        }
        Self {
            sites,
            max_delay: layers.graphics_define("missile launches", graphics, defines, max_delay),
        }
    }

    pub(super) fn site(&self, state: u32, kind: i32) -> Result<[f32; 3], &'static str> {
        let kind = if matches!(kind, ROCKET_SITE | GUN_EMPLACEMENT) {
            kind
        } else {
            AIR_BASE
        };
        let sites = self
            .sites
            .as_ref()
            .map_err(|_| "the content's building sites do not read")?;
        match sites.get(&(state, kind)) {
            Some(site) => *site,
            None if kind == ROCKET_SITE => Err("its state has no admitted rocket-site slot"),
            None if kind == GUN_EMPLACEMENT => {
                Err("its state has no admitted gun-emplacement slot")
            }
            None => Err("its state has no admitted air-base slot of an air-base type"),
        }
    }

    pub(super) fn max_delay(&self) -> Result<f32, &'static str> {
        self.max_delay
    }

    #[cfg(test)]
    pub(super) fn for_tests(sites: Sites, max_delay: f32) -> Self {
        Self {
            sites: Ok(sites),
            max_delay: Ok(max_delay),
        }
    }
}

pub(super) fn max_delay(defines: &Defines) -> Result<f32, &'static str> {
    defines
        .fixed5("NAirGfx", "RANDOM_SCENARIO_ANIMATION_MAX_DELAY")
        .map(|raw| raw.max(0) as f32 / FIXED5)
        .map_err(|_| "NDefines.NAirGfx.RANDOM_SCENARIO_ANIMATION_MAX_DELAY is unreadable")
}

fn sites(
    layers: &ResolvedLayers,
    map: &MapData,
    defines: &Defines,
    roster: &mesh::Roster,
) -> io::Result<(Sites, BTreeMap<String, usize>)> {
    let types = content_reader_key_names::buildings::read(layers)?;
    let (states, _) = content_reader_states::states::read(layers, defines, &map.definitions)?;
    let provinces = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    let slots = SiteSlots::build(
        &content_reader_map::positions::read_buildings(layers)?,
        &types,
        |x, z| provinces.at(x, z),
        |spawn, position, province| {
            admit_slot(spawn, position, province, map, states.definitions(), roster)
                == SlotAdmission::Admitted
        },
    );
    let bases = slots
        .bases()
        .map(|((state, kind), site)| ((state, kind as i32), site))
        .collect();
    Ok((bases, slots.refused().clone()))
}

fn draws(seed: u32, record: u32, stream: u32) -> Random {
    Random::new(SEED ^ seed ^ record.wrapping_mul(0x9e37_79b9) ^ stream.wrapping_mul(0x85eb_ca6b))
}

const DELAY_STREAM: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Launch {
    pub mesh: usize,
    pub wing: u64,
    pub position: [f32; 3],
    pub spawn: f32,
    pub length: f32,
    pub seed: u32,
    pub explosions: Option<ExplosionPlaces>,
}

impl Launch {
    pub(super) fn delay(seed: u32, max_delay: f32, clock_speed: f32) -> f32 {
        let unit = draws(seed, 0, DELAY_STREAM).next() as f32 * UNIT;
        unit * max_delay / clock_speed
    }

    pub(super) fn period(&self) -> f32 {
        PASS_SECONDS * (((self.spawn + self.length) / PASS_SECONDS).floor() + 1.0)
    }

    fn at(&self, seconds: f32) -> (u32, f32) {
        let period = self.period();
        let seconds = seconds.max(0.0);
        if !period.is_finite() {
            return (0, seconds);
        }
        let record = (seconds / period).floor();
        (record as u32, seconds - record * period)
    }

    fn random(&self, record: u32, stream: u32) -> Random {
        draws(self.seed, record, stream)
    }

    pub(super) fn heading(&self, record: u32) -> f32 {
        let fraction = (self.random(record, 0).next() % 100_000) as f32 / FIXED5;
        (fraction + fraction) * std::f32::consts::PI
    }

    pub(super) fn instance_for(index: usize) -> ModelInstance {
        ModelInstance {
            scale: 1.0,
            animation_seed: INSTANCE_SEED ^ index as u32,
            ..ModelInstance::default()
        }
    }

    pub(super) fn instance(&self, index: usize, seconds: f32) -> Option<ModelInstance> {
        let (record, into) = self.at(seconds);
        let age = into - self.spawn;
        (0.0..self.length).contains(&age).then(|| ModelInstance {
            position: self.position,
            rotation: self.heading(record),
            animation_epoch: record,
            animation_phase: age,
            ..Self::instance_for(index)
        })
    }

    pub(super) fn finished(&self, from: f32, to: f32) -> Option<u32> {
        if from.is_nan() || to.is_nan() || to <= from {
            return None;
        }
        let period = self.period();
        let end = self.spawn + self.length;
        let last = ((to - end) / period).floor();
        (last >= 0.0 && last * period + end > from).then_some(last as u32)
    }

    pub(super) fn finished_at_open(&self) -> Option<u32> {
        (self.spawn + self.length <= 0.0).then_some(0)
    }

    pub(super) fn explosion(&self, record: u32) -> Option<(u32, [f32; 3], [f32; 4])> {
        let places = self
            .explosions
            .as_ref()
            .filter(|places| !places.is_empty())?;
        let (province, place) = places[self.random(record, 1).next() as usize % places.len()];
        place.map(|(position, bounds)| (province, position, bounds))
    }
}
