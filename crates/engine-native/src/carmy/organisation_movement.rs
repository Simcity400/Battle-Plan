use super::*;
use engine_sites::Rva;

pub const LOW_ORGANISATION_FOR_ATTACK_RVA: Rva = Rva(0x033e_0898);

fn below_threshold(current: i64, maximum: i64, threshold: i64) -> Result<bool, &'static str> {
    let share = if (current as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        MOVEMENT_ONE
    } else {
        cunit::eta_ratio(current, maximum)?.clamp(0, MOVEMENT_ONE)
    };
    Ok(share < threshold)
}

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
    if location.1 & (crate::cprovince::definition::LAND_FLAG | crate::cprovince::definition::LAKE_FLAG)
        == 0
    {
        match first {
            Ok((_, flags)) if flags & crate::cprovince::definition::LAND_FLAG != 0 => {
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
            && from_flags & crate::cprovince::definition::LAND_FLAG != 0
            && to_flags & crate::cprovince::definition::LAND_FLAG != 0
            && (index == 0 || from_id != to_id)
        {
            return (Ok(true), refused);
        }
        from = to;
    }
    (failure.map_or(Ok(false), Err), refused)
}

pub(super) struct OrganisationMemo {
    identities: crate::carmy_leader::PassTable<Result<(u32, usize), &'static str>>,
    flags: crate::carmy_leader::PassTable<Result<u8, &'static str>>,
    provinces: Option<Array>,
}

impl OrganisationMemo {
    pub(super) fn new(units: usize) -> std::io::Result<Self> {
        engine_allocation::advisory(|| {
            Ok(Self {
                identities: crate::carmy_leader::PassTable::try_with_capacity(units)?,
                flags: crate::carmy_leader::PassTable::try_with_capacity(units)?,
                provinces: None,
            })
        })
    }

    pub(super) fn vacant() -> Self {
        Self {
            identities: crate::carmy_leader::PassTable::vacant(),
            flags: crate::carmy_leader::PassTable::vacant(),
            provinces: None,
        }
    }

    fn clear(&mut self) {
        self.identities.clear();
        self.flags.clear();
        self.provinces = None;
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.identities.allocated_bytes() + self.flags.allocated_bytes()
    }

    fn province(
        &mut self,
        memory: &impl Memory,
        base: usize,
        province: usize,
        expected: Option<u32>,
    ) -> Result<(u32, u8), &'static str> {
        let mut fetched_flags = None;
        let (id, definition) = self.identities.get_or_if(
            province,
            || {
                if province == 0 {
                    return Err("movement province identity mismatch");
                }
                let expected_vtable =
                    engine_sites::at(base, cprovince::VTABLE_RVA).map(|p| p as u64);
                let def_at = province.checked_add(cprovince::DEFINITION);
                let id_at = province.checked_add(cprovince::ID);
                let (definition, id) = match (def_at, id_at) {
                    (Some(def_at), Some(id_at)) => {
                        let mut vtable = [0u8; 8];
                        let mut def_bytes = [0u8; 8];
                        let mut id_bytes = [0u8; 4];
                        let completed = memory.copy_scattered_prefetched(
                            [province, def_at, id_at],
                            &mut [&mut vtable, &mut def_bytes, &mut id_bytes],
                        );
                        if completed == 0 || Some(u64::from_le_bytes(vtable)) != expected_vtable {
                            return Err("movement province identity mismatch");
                        }
                        if completed < 2 {
                            return Err("movement province pointer unreadable");
                        }
                        let definition = usize::try_from(u64::from_le_bytes(def_bytes))
                            .map_err(|_| "movement province pointer overflow")?;
                        if definition == 0 {
                            return Err("movement province definition mismatch");
                        }
                        let id = if completed == 3 {
                            Some(u32::from_le_bytes(id_bytes))
                        } else {
                            None
                        };
                        (definition, id)
                    }
                    _ => {
                        if read::<u64>(memory, province) != expected_vtable {
                            return Err("movement province identity mismatch");
                        }
                        let definition = movement_pointer(memory, province, cprovince::DEFINITION)?;
                        if definition == 0 {
                            return Err("movement province definition mismatch");
                        }
                        let definition_id = presentation_field::<u32>(
                            memory,
                            definition,
                            crate::cprovince::definition::ID,
                        )?;
                        let id = presentation_field::<u32>(memory, province, cprovince::ID)?;
                        if definition_id != id {
                            return Err("movement province definition mismatch");
                        }
                        return Ok((id, definition));
                    }
                };
                let want_flags = id.is_some_and(|id| expected.is_none_or(|e| id == e))
                    && !self.flags.contains(definition);
                let (definition_id, maybe_flags) = match (
                    definition.checked_add(crate::cprovince::definition::ID),
                    want_flags
                        .then(|| definition.checked_add(crate::cprovince::definition::TERRAIN_FLAGS))
                        .flatten(),
                ) {
                    (Some(def_id_at), Some(flags_at)) => {
                        let mut def_id_bytes = [0u8; 4];
                        let mut flag_bytes = [0u8; 1];
                        let completed = memory.copy_scattered_prefetched(
                            [def_id_at, flags_at],
                            &mut [&mut def_id_bytes, &mut flag_bytes],
                        );
                        if completed == 0 {
                            return Err("presentation field unreadable");
                        }
                        (
                            u32::from_le_bytes(def_id_bytes),
                            Some(if completed == 2 {
                                Ok(flag_bytes[0])
                            } else {
                                Err("presentation field unreadable")
                            }),
                        )
                    }
                    _ => (
                        presentation_field::<u32>(
                            memory,
                            definition,
                            crate::cprovince::definition::ID,
                        )?,
                        None,
                    ),
                };
                let id = id.ok_or("presentation field unreadable")?;
                if definition_id != id {
                    return Err("movement province definition mismatch");
                }
                fetched_flags = maybe_flags;
                Ok((id, definition))
            },
            |value| value.is_ok(),
        )?;
        if expected.is_some_and(|expected| id != expected) {
            return Err("movement route province identity mismatch");
        }
        let flags = self.flags.get_or_if(
            definition,
            || match fetched_flags {
                Some(flags) => flags,
                None => presentation_field(
                    memory,
                    definition,
                    crate::cprovince::definition::TERRAIN_FLAGS,
                ),
            },
            |value| value.is_ok(),
        )?;
        Ok((id, flags))
    }

    fn route(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        route: Array,
        index: u32,
        head: Option<Option<u32>>,
    ) -> Result<(u32, u8), &'static str> {
        let id = match head.filter(|_| index == 0) {
            Some(id) => id,
            None => read::<u32>(
                memory,
                route
                    .element(index, 4)
                    .ok_or("movement route index invalid")?,
            ),
        }
        .ok_or("movement route id unreadable")?;
        let provinces = match self.provinces {
            Some(provinces) => provinces,
            None => {
                let provinces = presentation_array(memory, state, cgame_state::PROVINCES, 8)?;
                self.provinces = Some(provinces);
                provinces
            }
        };
        let province = read::<u64>(
            memory,
            provinces
                .element(id, 8)
                .ok_or("movement route province outside roster")?,
        )
        .ok_or("movement route province unreadable")? as usize;
        self.province(memory, base, province, Some(id))
    }
}

impl StatusPacket {
    pub fn capture_movement_organisation(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
    ) -> u64 {
        let mut refused = 0;
        let mut threshold = None;
        let mut countries = None;
        self.organisation_memo.clear();
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
                    self.organisation_memo
                        .province(memory, base, location, None),
                    (0..route.count).map(|index| {
                        self.organisation_memo.route(
                            memory,
                            base,
                            state,
                            route,
                            index,
                            seed.route_head,
                        )
                    }),
                );
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
                    let owner = countries
                        .get_or_insert_with(|| ccountry::Reader::new(memory, base, state))
                        .as_ref()
                        .map_err(|reason| *reason)?
                        .resolve(raw)?;
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
                    presentation_global(memory, base, LOW_ORGANISATION_FOR_ATTACK_RVA)
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
        assert!(!below_threshold(300_009, 1_000_009, 30_000).unwrap());
        assert!(below_threshold(300_000, 1_000_009, 30_000).unwrap());
        for sentinel in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert!(!below_threshold(sentinel, 1_000_000, 30_000).unwrap());
        }
        assert!(below_threshold(1_000_000, 1_000_000, 100_001).unwrap());
    }
    #[test]
    fn organisation_demand_visits_later_land_edges_and_keeps_readable_proofs() {
        let land = |id| Ok((id, crate::cprovince::definition::LAND_FLAG));
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
        let lake = |id| Ok((id, crate::cprovince::definition::LAKE_FLAG));
        assert_eq!(
            land_speed_demand(lake(1), [land(2), land(3)]),
            (Ok(true), 0)
        );
        assert_eq!(land_speed_demand(land(1), [land(1)]), (Ok(true), 0));
        assert_eq!(
            land_speed_demand(land(1), [Err("unproved")]),
            (Err("unproved"), 1)
        );
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
