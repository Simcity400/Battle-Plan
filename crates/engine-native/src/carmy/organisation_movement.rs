//! Exact organisation operands only where ordinary movement takes its penalty.
use super::*;

/// The loaded low-organisation attack threshold, fixed5.
pub const LOW_ORGANISATION_FOR_ATTACK_RVA: usize = 0x0331_cec0;

fn below_threshold(current: i64, maximum: i64, threshold: i64) -> Result<bool, &'static str> {
    let share = if (current as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        MOVEMENT_ONE
    } else {
        cunit::eta_ratio(current, maximum)?.clamp(0, MOVEMENT_ONE)
    };
    Ok(share < threshold)
}

/// Whether the first speed query or a later non-self edge reaches ordinary land speed, and how many
/// unreadable route positions were seen before that proof. An unreadable edge
/// never cancels a later readable proof. Sea-to-land invasion itineraries use
/// their fixed landing speed instead of ordinary land speed.
pub fn land_speed_demand(
    location: Result<(u32, u8), &'static str>,
    path: impl IntoIterator<Item = Result<(u32, u8), &'static str>>,
) -> (Result<bool, &'static str>, u64) {
    let mut path = path.into_iter();
    let Some(first) = path.next() else {
        return (Ok(false), 0);
    };
    let location = match location {
        Ok(location) => location,
        Err(reason) => return (Err(reason), 1),
    };
    if location.1 & (crate::province_definition::LAND_FLAG | crate::province_definition::LAKE_FLAG)
        == 0
    {
        match first {
            Ok((_, flags)) if flags & crate::province_definition::LAND_FLAG != 0 => {
                return (Ok(false), 0);
            }
            Err(reason) => return (Err(reason), 1),
            _ => {}
        }
    }
    let mut from = Ok(location);
    let mut refused = 0;
    let mut failure = None;
    for (index, to) in std::iter::once(first).chain(path).enumerate() {
        if let Err(reason) = to {
            refused += 1;
            failure.get_or_insert(reason);
        }
        if let (Ok((from_id, from_flags)), Ok((to_id, to_flags))) = (from, to)
            && from_flags & crate::province_definition::LAND_FLAG != 0
            && to_flags & crate::province_definition::LAND_FLAG != 0
            && (index == 0 || from_id != to_id)
        {
            return (Ok(true), refused);
        }
        from = to;
    }
    (failure.map_or(Ok(false), Err), refused)
}

/// Only branch inputs of a province, with no state/controller/region demand.
fn land_province(
    memory: &impl Memory,
    base: usize,
    province: usize,
) -> Result<(u32, u8), &'static str> {
    let definition = movement_definition(memory, base, province)?;
    Ok((
        presentation_field(memory, province, cprovince::ID)?,
        presentation_field::<u8>(
            memory,
            definition,
            crate::province_definition::TERRAIN_FLAGS,
        )?,
    ))
}

impl StatusPacket {
    /// Reuses the census's exact current organisation and the status pass's
    /// certified maxima. Reads the live threshold once, and the destination's
    /// war relation only for movers below it. A failed field stays independent
    /// of the rest of DivisionStatus.
    pub fn capture_movement_organisation(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
    ) -> u64 {
        let mut refused = 0;
        let mut threshold = None;
        for index in 0..self.units.len() {
            let value = (|| -> Result<Option<[i64; 2]>, &'static str> {
                let seed = seeds.get(index).ok_or("organisation census missing")?;
                let route = seed
                    .ordinary_route
                    .and_then(|h| Array::from_header(h, 4))
                    .ok_or("organisation movement route unreadable")?;
                if route.count == 0 {
                    return Ok(None);
                }
                if seed
                    .naval_transfer_membership
                    .is_some_and(|value| value != 0)
                {
                    return Ok(None);
                }
                let location = seed.location.ok_or("organisation location unreadable")?;
                let (ordinary, route_refused) = land_speed_demand(
                    land_province(memory, base, location),
                    (0..route.count).map(|index| {
                        movement_route_province(memory, base, state, route, index)
                            .and_then(|province| land_province(memory, base, province))
                    }),
                );
                // Keep a proven pair available while counting independent route faults.
                refused += route_refused.saturating_sub(u64::from(ordinary.is_err()));
                if !ordinary? {
                    return Ok(None);
                }
                seed.naval_transfer_membership
                    .ok_or("organisation transfer unreadable")?;
                if seed
                    .strategic_redeployment
                    .ok_or("organisation redeployment unreadable")?
                    != 0
                {
                    let roster = || seed.roster_owner.ok_or("organisation roster unreadable");
                    let expeditionary = seed
                        .expeditionary_owner
                        .ok_or("organisation owner unreadable")?;
                    let raw = if expeditionary as i32 > 0 {
                        expeditionary
                    } else {
                        roster()?
                    };
                    let owner = ccountry::Reader::new(memory, base, state)?.resolve(raw)?;
                    let inputs = strategic_action::read_inputs(
                        memory,
                        base,
                        state,
                        raw,
                        owner.pointer,
                        roster,
                        || movement_pointer(memory, location, cprovince::STATE),
                    )
                    .map_err(|failure| failure.reason())?;
                    if !inputs.blocked() {
                        return Ok(None);
                    }
                }
                let current = seed
                    .organisation
                    .ok_or("movement organisation unreadable")?;
                let maximum = self
                    .unit_maxima(index)
                    .ok_or("movement maximum organisation unreadable")?
                    .effective()
                    .1;
                let limit = *threshold.get_or_insert_with(|| {
                    presentation_field(memory, base, LOW_ORGANISATION_FOR_ATTACK_RVA)
                });
                if !below_threshold(current, maximum, limit?)? {
                    return Ok(None);
                }
                if !movement_attacking(memory, base, state, seed.pointer)? {
                    return Ok(None);
                }
                Ok(Some([current, maximum]))
            })();
            refused += u64::from(value.is_err());
            self.units[index].movement_organisation = value.ok();
        }
        refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn movement_organisation_gate_preserves_subdisplay_values_and_native_threshold() {
        assert!(below_threshold(924_554, 3_640_000, 30_000).unwrap());
        // Display rounding would put this mover below the threshold.
        assert!(!below_threshold(300_009, 1_000_009, 30_000).unwrap());
        assert!(below_threshold(300_000, 1_000_009, 30_000).unwrap());
        for sentinel in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert!(!below_threshold(sentinel, 1_000_000, 30_000).unwrap());
        }
        assert!(below_threshold(1_000_000, 1_000_000, 100_001).unwrap());
    }
    #[test]
    fn organisation_demand_visits_later_land_edges_and_keeps_readable_proofs() {
        let land = |id| Ok((id, crate::province_definition::LAND_FLAG));
        let sea = |id| Ok((id, 0));
        assert_eq!(
            land_speed_demand(land(1), [sea(2), land(3), land(4)]),
            (Ok(true), 0)
        );
        assert_eq!(
            land_speed_demand(land(1), [land(2), Err("later refused")]),
            (Ok(true), 0)
        );
        assert_eq!(
            land_speed_demand(land(1), [Err("earlier refused"), land(3), land(4)]),
            (Ok(true), 1)
        );
        assert_eq!(land_speed_demand(land(1), [land(1), sea(2)]), (Ok(true), 0));
        assert_eq!(
            land_speed_demand(sea(1), [land(2), land(3)]),
            (Ok(false), 0)
        );
        assert_eq!(
            land_speed_demand(sea(1), [sea(2), land(3), land(4)]),
            (Ok(true), 0)
        );
        assert_eq!(land_speed_demand(land(1), [sea(2)]), (Ok(false), 0));
        let lake = |id| Ok((id, crate::province_definition::LAKE_FLAG));
        assert_eq!(
            land_speed_demand(lake(1), [land(2), land(3)]),
            (Ok(true), 0)
        );
        assert_eq!(land_speed_demand(land(1), [land(1)]), (Ok(true), 0));
        assert_eq!(
            land_speed_demand(land(1), [Err("unproved")]),
            (Err("unproved"), 1)
        );
        // A low-organisation pair is retained for the later ordinary edge,
        // so neutral 7 km/h becomes 1.4 km/h at the zero-org factor.
        let (ordinary, _) = land_speed_demand(land(1), [sea(2), land(3), land(4)]);
        assert!(ordinary.unwrap() && below_threshold(0, 1_000_000, 30_000).unwrap());
        let speed = PreparedNormalSpeed {
            modifier: MOVEMENT_ONE,
            base_speed: 700_000,
            organisation: Some(20_000),
            fuel: None,
            outer_scale: MOVEMENT_ONE,
        };
        assert_eq!(speed.evaluate(NormalEdgeOperands::default()), 140_000);
    }
}
