use std::{collections::BTreeMap, io};

use content_reader_defines::defines::Defines;
use content_reader_layers::ResolvedLayers;
use content_reader_save::{PartialMapMembership, Save};

pub(super) fn recounted_weights(
    save: &Save,
    tag: &[u8],
    membership: &PartialMapMembership,
    layers: &ResolvedLayers,
    defines: &Defines,
) -> io::Result<[i32; 4]> {
    let country = save.countries.get(tag).ok_or_else(|| {
        io::Error::other(format!(
            "recount witness country {} is absent from the save",
            String::from_utf8_lossy(tag)
        ))
    })?;
    let (terrain, map) = content_reader_key_names::terrain::read(layers)?;
    let provinces =
        content_reader_map::map::province_terrains(&layers.read(&map.file("definitions")?)?)?;
    let factor = defines.fixed5("NDiplomacy", "VICTORY_POINT_WORTH_FACTOR")?;
    let mut cores = BTreeMap::<u32, i32>::new();
    for &state in &country.cores {
        *cores.entry(state).or_default() += 1;
    }
    let mut held_core = 0_i32;
    let mut total_core = 0_i32;
    let mut held_territory = 0_i32;
    let mut total_territory = 0_i32;
    let relevant = |state: &content_reader_save::State| {
        state.owner.as_deref() == Some(tag)
            || state.contested_owners.iter().any(|other| other == tag)
    };
    for province in save.province_control(membership)? {
        if !province.membership_known {
            return Err(io::Error::other(format!(
                "province {} lacks state membership for the recount witness",
                province.id
            )));
        }
        let Some(state) = province.state.and_then(|id| save.states.get(&id)) else {
            continue;
        };
        let owned = state.owner.as_deref() == Some(tag);
        if !relevant(state) {
            continue;
        }
        let worth = provinces
            .get(&province.id)
            .and_then(|name| terrain.category(name))
            .and_then(|index| terrain.categories.get(index))
            .and_then(|category| category.control_weight)
            .ok_or_else(|| {
                io::Error::other(format!(
                    "province {} has no verified terrain weight",
                    province.id
                ))
            })?;
        let points = save
            .provinces
            .get(&province.id)
            .and_then(|row| row.victory_points)
            .unwrap_or(0);
        let weighted = (i64::from(points) * 100_000).wrapping_mul(factor) / 10_000_000_000;
        let weight = (weighted as i32).wrapping_add(worth).wrapping_add(1);
        let core = cores.get(&state.id).copied().unwrap_or(0);
        total_territory = total_territory.wrapping_add(weight);
        if owned {
            total_core = total_core.wrapping_add(weight.wrapping_mul(core));
        }
        if province.controller.as_deref() == Some(tag) {
            held_territory = held_territory.wrapping_add(weight);
            if core > 0 {
                held_core = held_core.wrapping_add(weight);
            }
        }
    }
    Ok([held_core, total_core, held_territory, total_territory])
}
