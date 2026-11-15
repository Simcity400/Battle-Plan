use super::*;

#[derive(Clone, Copy, Default)]
pub(super) struct Demand {
    country: usize,
    region: usize,
    slot: u32,
    id: u32,
}

pub(super) fn capture(
    wings: &[WingIdentity],
    registered: Option<&[cair_regional_mission::Row]>,
    mut demand: Buffer<'_, Demand>,
    mut output: Buffer<'_, rows::AirGroundCrewRow>,
    mut flag: impl FnMut(usize, usize) -> Option<u8>,
) -> Result<Outcome, &'static str> {
    demand.ensure(registered.map_or(0, <[_]>::len))?;
    let mut count = 0;
    for wing in registered_wings(wings, registered)? {
        let wing = wing?;
        let Some(id) = wing.region else {
            continue;
        };
        if wing.owner_pointer == 0 || wing.region_pointer == 0 {
            return Err("Ground Crews lacks authenticated mission country or region context");
        }
        *demand
            .get_mut(count)
            .ok_or("Ground Crews demand storage incomplete")? = Demand {
            country: wing.owner_pointer,
            region: wing.region_pointer,
            slot: wing.owner_slot,
            id,
        };
        count += 1;
    }
    let demand = &mut demand[..count];
    demand.sort_unstable_by_key(|key| (key.slot, key.id));
    output.ensure(count)?;
    let mut previous = None;
    let mut len = 0;
    for key in demand.iter() {
        if previous.replace((key.slot, key.id)) == Some((key.slot, key.id)) {
            continue;
        }
        let enabled = flag(key.country, key.region).map(u32::from);
        if enabled == Some(0) {
            continue;
        }
        *output
            .get_mut(len)
            .ok_or("Ground Crews output storage incomplete")? = rows::AirGroundCrewRow {
            supply: rows::Recorded::Value(None),
            country: key.slot,
            region: key.id,
            enabled,
        };
        len += 1;
    }
    Ok(Outcome::Rows { len })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pool_owner_demand_is_independent_of_sortie_base_and_deduplicated() {
        let wings = [1, 2, 3, 4].map(|wing| WingIdentity {
            wing,
            owner: 7,
            owner_slot: 2,
            owner_pointer: 0x100,
            region: Some(if wing <= 2 { 8 } else { wing as u32 }),
            region_pointer: if wing <= 2 {
                0x800
            } else {
                wing as usize * 0x100
            },
            country_override: 99,
            ..Default::default()
        });
        let registered = [1, 2, 3, 4].map(|wing| cair_regional_mission::Row { wing, ordinal: 0 });
        let mut demand = vec![Demand::default(); 4];
        let mut output = vec![const {
            rows::AirGroundCrewRow {
                supply: rows::Recorded::Value(None),
                country: 0,
                region: 0,
                enabled: None,
            }
        }; 4];
        let mut calls = Vec::new();
        let outcome = capture(
            &wings,
            Some(&registered),
            Buffer::Growing(&mut demand),
            Buffer::Growing(&mut output),
            |country, region| {
                calls.push((country, region));
                match region {
                    0x300 => Some(0),
                    0x400 => None,
                    0x800 => Some(1),
                    _ => unreachable!(),
                }
            },
        )
        .unwrap();
        assert_eq!(calls, [(0x100, 0x300), (0x100, 0x400), (0x100, 0x800)]);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            (output[0].country, output[0].region, output[0].enabled),
            (2, 4, None)
        );
        assert_eq!(
            (output[1].country, output[1].region, output[1].enabled),
            (2, 8, Some(1))
        );
    }
}
