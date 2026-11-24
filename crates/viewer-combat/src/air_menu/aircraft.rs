use super::{Content, DecodedFrame};
use content_reader_map::map::ProvinceKind;
use replay_data::rows::{AirCountryRelationRow, AirWingRow, tables};
use replay_derive::{
    air_mission_sources::{AirRegistrySources, MissionCountrySources},
    aircraft_count::SortieSources,
};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet},
};
use viewer_unit_reads::air_reachability::*;

pub(super) struct Figures {
    pub counts: [Result<[i32; 3], String>; 3],
    pub efficiency: [Result<i64, String>; 2],
}

pub(super) fn figures(
    content: &Content,
    frame: &DecodedFrame,
    region: u32,
    ours: u32,
    sides: Result<&[Vec<u32>; 2], &String>,
) -> Figures {
    let result = read(content, frame, region, ours, sides);
    result.unwrap_or_else(|why| Figures {
        counts: std::array::from_fn(|_| Err(why.clone())),
        efficiency: std::array::from_fn(|_| Err(why.clone())),
    })
}

fn read(
    content: &Content,
    frame: &DecodedFrame,
    region: u32,
    ours: u32,
    sides: Result<&[Vec<u32>; 2], &String>,
) -> Result<Figures, String> {
    let links: BTreeMap<_, _> = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let wings: BTreeMap<_, _> = frame
        .complete::<tables::AirWings>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let mut registered = BTreeMap::<u32, Vec<(u32, &AirWingRow)>>::new();
    for row in frame
        .complete::<tables::AirRegionalMissions>()
        .map_err(|e| e.to_string())?
    {
        let wing = *wings
            .get(&row.wing)
            .ok_or("registered air wing is missing")?;
        if wing.mission_region != Some(region) {
            continue;
        }
        let owner = *links
            .get(&wing.owner)
            .ok_or("registered air wing owner is unresolved")?;
        registered
            .entry(owner)
            .or_default()
            .push((row.ordinal, wing));
    }
    for wings in registered.values_mut() {
        wings.sort_unstable_by_key(|(ordinal, _)| *ordinal);
    }
    let registry = OnceCell::new();
    let sortie = OnceCell::new();
    let mission_countries = OnceCell::new();
    let sources = || {
        let registry = registry
            .get_or_init(|| {
                AirRegistrySources::read(frame)
                    .map_err(|e| e.to_string())?
                    .ok_or("air base registry is incomplete".to_owned())
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let sortie = sortie
            .get_or_init(|| {
                SortieSources::read(frame)
                    .map_err(|e| e.to_string())?
                    .map_err(str::to_owned)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let countries = mission_countries
            .get_or_init(|| {
                MissionCountrySources::read(frame, &links, registry).map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?;
        Ok::<_, String>((registry, sortie, countries))
    };
    let members = OnceCell::new();
    let regions = OnceCell::new();
    let fractions = OnceCell::new();
    let wars = OnceCell::new();
    let families = OnceCell::new();
    let fog = OnceCell::new();
    let scans = OnceCell::new();
    let targets = || {
        scans
            .get_or_init(|| {
                let context = content
                    .air_context
                    .as_ref()
                    .ok_or("air target content is unavailable")?;
                let members = members.get_or_init(|| {
                    content
                        .strategic_regions
                        .iter()
                        .map(|(&id, region)| (id, region.provinces.clone()))
                        .collect::<BTreeMap<_, _>>()
                });
                let fractions = fractions.get_or_init(|| {
                    members
                        .iter()
                        .map(|(&id, provinces)| {
                            (
                                id,
                                region_fractions(
                                    provinces.len() as u64,
                                    provinces
                                        .iter()
                                        .filter(|id| {
                                            context.kinds.get(id) == Some(&ProvinceKind::Land)
                                        })
                                        .count() as u64,
                                    provinces
                                        .iter()
                                        .filter(|id| {
                                            context.kinds.get(id) == Some(&ProvinceKind::Sea)
                                        })
                                        .count() as u64,
                                ),
                            )
                        })
                        .collect()
                });
                let wars = wars
                    .get_or_init(|| Wars::read(frame, &links).map_err(|e| e.to_string()))
                    .as_ref()
                    .map_err(Clone::clone)?;
                let families = families.get_or_init(|| {
                    viewer_foundation::visibility::Families::read(frame, &links).ok()
                });
                let fog = fog.get_or_init(|| {
                    families
                        .clone()
                        .and_then(|families| families.with_attaches(frame, &links).ok())
                });
                let regions = regions
                    .get_or_init(|| viewer_unit_reads::air_reachability::RegionStates::read(frame, members).map_err(|why| why.to_string()))
                    .as_ref()
                    .map_err(Clone::clone)?;
                let needs = |mask| {
                    registered
                        .values()
                        .flatten()
                        .any(|(_, wing)| scanned_bits(wing.mission_mask) & mask != 0)
                };
                Ok::<_, String>(Scans {
                    tactical: needs(1 << 1)
                        .then(|| TacticalTargetSources::read(frame, wars))
                        .and_then(Result::ok),
                    port: needs(1 << 8)
                        .then(|| PortTargetSources::read(frame, wars))
                        .and_then(Result::ok),
                    strategic: needs(0x1_0008)
                        .then(|| StrategicTargetSources::read(frame, wars, regions))
                        .and_then(Result::ok),
                    scout: needs(1 << 14)
                        .then(|| {
                            ScoutTargetSources::read(frame, &links, wars, fog.as_ref(), regions)
                        })
                        .and_then(Result::ok),
                    naval_combat: needs(0x90)
                        .then(|| {
                            NavalCombatTargetSources::read(frame, &links, wars, families.as_ref())
                        })
                        .and_then(Result::ok),
                    occupants: needs(0x90)
                        .then(|| NavalOccupantSources::read(frame, wars, &context.kinds))
                        .and_then(Result::ok),
                    mine: needs(replay_data::rows::AIR_MISSION_LAY_MINES)
                        .then(|| MineTargetSources::read(frame, &links))
                        .and_then(Result::ok),
                    fractions,
                    dlc_mask: frame
                        .complete::<tables::SimulationFeatures>()
                        .ok()
                        .and_then(|rows| rows.first())
                        .map(|row| row.dlc_mask),
                    mine_maximum_raw: if needs(replay_data::rows::AIR_MISSION_LAY_MINES) {
                        *content
                            .assets
                            .air_mine_maximum
                            .as_ref()
                            .map_err(Clone::clone)?
                    } else {
                        0
                    },
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    };
    let counts = |owner: u32| {
        let mut counts = [0_i32; 3];
        for &(_, wing) in registered.get(&owner).into_iter().flatten() {
            if wing.mission_mask == 0
                || wing.eligibility_veto != 0
                || wing.eligibility_counter > wing.eligibility_limit
            {
                continue;
            }
            if wing.mission_active == 0 && wing.mission_mask & 0x800 == 0 {
                continue;
            }
            let (registry, sortie, countries) = sources()?;
            if registry.transfer_resolves(wing.base_override) {
                continue;
            }
            let input = sortie
                .mission(wing.id)
                .ok_or("air mission inputs are missing")?;
            let flags = input.definition_map_flags;
            if wing.planes == 0 && flags & replay_data::rows::AIR_MISSION_ZERO_PLANE_FLAGS == 0 {
                continue;
            }
            let base = registry
                .bases
                .get(&wing.air_base)
                .ok_or("air wing pool base is unresolved")?;
            if !sortie.host_admits(wing, base).map_err(str::to_owned)? {
                continue;
            }
            let country = countries
                .resolve(wing)
                .ok_or("air wing mission country is unresolved")?;
            match targets()?.admission(wing, country, region, input) {
                TargetAdmission::Blocked => continue,
                TargetAdmission::Unknown(causes) => {
                    return Err(causes
                        .iter()
                        .map(|cause| cause.reason())
                        .collect::<Vec<_>>()
                        .join("; "));
                }
                TargetAdmission::Allowed => {}
            }
            let category = if flags & replay_data::rows::AIR_MENU_FIGHTER_FLAGS != 0 {
                Some(0)
            } else if flags & replay_data::rows::AIR_MENU_BOMBER_FLAGS != 0 {
                Some(1)
            } else if flags & replay_data::rows::AIR_MENU_TRANSPORT_FLAGS != 0 {
                Some(2)
            } else {
                None
            };
            if let Some(category) = category {
                counts[category] = counts[category].wrapping_add(wing.planes);
            }
        }
        Ok::<_, String>(counts)
    };
    let mut wanted = BTreeSet::from([ours]);
    if let Ok(sides) = sides {
        wanted.extend(sides.iter().flatten().copied());
    }
    let by_country: BTreeMap<_, _> = wanted
        .into_iter()
        .map(|owner| (owner, counts(owner)))
        .collect();
    let combine = |owners: &[u32]| {
        owners.iter().try_fold([0_i32; 3], |mut total, owner| {
            let counts = by_country
                .get(owner)
                .ok_or("air count participant was not evaluated")?
                .as_ref()
                .map_err(Clone::clone)?;
            for index in 0..3 {
                total[index] = total[index].wrapping_add(counts[index]);
            }
            Ok::<_, String>(total)
        })
    };
    let efficiency = |owners: &[u32], enemy: bool| {
        let mut count = 0_i32;
        let mut total = 0_i64;
        for owner in owners {
            for &(_, wing) in registered.get(owner).into_iter().flatten() {
                let (registry, sortie, countries) = sources()?;
                let fuel = *content
                    .assets
                    .air_fuel_penalty
                    .as_ref()
                    .map_err(Clone::clone)?;
                let value = sortie
                    .wing_efficiency(wing, registry, countries, fuel)
                    .map_err(str::to_owned)?;
                count = count.wrapping_add(wing.planes);
                total = total.wrapping_add(i64::from(wing.planes).wrapping_mul(value));
            }
        }
        if count > 0 {
            return Ok(
                if (total as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
                    0xffff_ffff
                } else {
                    total.wrapping_mul(100_000) / (i64::from(count) * 100_000)
                },
            );
        }
        if enemy {
            Ok(100_000)
        } else {
            replay_derive::aircraft_count::empty_menu_efficiency(frame, ours, region)
        }
    };
    let enemies = || {
        frame
            .complete::<tables::AirCountryRelations>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|row| row.country == ours && row.kind == AirCountryRelationRow::ENEMY)
            .map(|row| {
                if row.other == 0 {
                    Ok(0)
                } else {
                    links
                        .get(&row.other)
                        .copied()
                        .ok_or("enemy air country is unresolved".to_owned())
                }
            })
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(Figures {
        counts: [
            combine(&[ours]),
            sides
                .map_err(Clone::clone)
                .and_then(|sides| combine(&sides[0])),
            sides
                .map_err(Clone::clone)
                .and_then(|sides| combine(&sides[1])),
        ],
        efficiency: [
            efficiency(&[ours], false),
            enemies().and_then(|owners| efficiency(&owners, true)),
        ],
    })
}
