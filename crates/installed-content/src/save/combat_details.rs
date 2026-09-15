//! Saved combat-window inputs; absence remains explicit for conditional writers.
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LandDetails {
    pub terrain: Option<Vec<u8>>,
    pub sides: [LandSide; 2],
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LandSide {
    pub committed: Vec<u64>,
    pub reserves: Vec<u64>,
    pub tactic: Option<i32>,
    pub air_counts: Vec<i32>,
    pub anti_air: Option<Decimal>,
    pub air_kills: Option<i32>,
    /// Air STR/ORG, ground STR/ORG, prevented STR/ORG; 1412a8760 omits <=0.
    pub damage: [Option<Decimal>; 6],
    pub organisation_loss: Option<Vec<Decimal>>,
    pub organisation_loss_index: Option<u32>,
    pub organisation_loss_count: Option<u32>,
}

pub const LAND_DAMAGE_KEYS: [&[u8]; 6] = [
    b"air_damage_str",
    b"air_damage_org",
    b"ground_damage_str",
    b"ground_damage_org",
    b"prevented_damage_str",
    b"prevented_damage_org",
];
fn optional_integer<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<Option<T>> {
    field(items, key)?
        .map(|value| integer(scalar(value)?))
        .transpose()
}
fn optional_bytes(items: &[Item], key: &[u8]) -> io::Result<Option<Vec<u8>>> {
    field(items, key)?
        .map(|value| Ok(scalar(value)?.to_vec()))
        .transpose()
}
fn handles(items: &[Item], key: &[u8]) -> io::Result<Vec<u64>> {
    items
        .iter()
        .filter(|item| item.key.as_deref() == Some(key))
        .map(|item| typed_handle(&item.value))
        .collect()
}
fn decimals(items: &[Item], key: &[u8]) -> io::Result<Option<Vec<Decimal>>> {
    field(items, key)?
        .map(|value| {
            block(value)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid("combat scalar array contains a named entry"));
                    }
                    Decimal::parse(scalar(&item.value)?)
                })
                .collect()
        })
        .transpose()
}
fn land_side(items: &[Item]) -> io::Result<LandSide> {
    let mut damage = std::array::from_fn(|_| None);
    for (value, key) in damage.iter_mut().zip(LAND_DAMAGE_KEYS) {
        *value = decimal(items, key)?;
    }
    Ok(LandSide {
        committed: handles(items, b"front")?,
        reserves: handles(items, b"reserves")?,
        tactic: optional_integer(items, b"tactic")?,
        air_counts: items
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"air_plane"))
            .map(|item| integer(&required_bytes(block(&item.value)?, b"amount")?))
            .collect::<io::Result<_>>()?,
        anti_air: decimal(items, b"anti_air_attack")?,
        air_kills: optional_integer(items, b"air_kills")?,
        damage,
        organisation_loss: decimals(items, b"org_loss_summary")?,
        organisation_loss_index: optional_integer(items, b"org_loss_summary_index")?,
        organisation_loss_count: optional_integer(items, b"num_org_losses")?,
    })
}
pub(super) fn land(items: &[Item]) -> io::Result<LandDetails> {
    Ok(LandDetails {
        terrain: optional_bytes(items, b"terrain")?,
        sides: [
            land_side(required_block(items, b"attacker")?)?,
            land_side(required_block(items, b"defender")?)?,
        ],
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalDetails {
    pub day: Option<i32>,
    pub duration: Option<i32>,
    pub progress: Option<Decimal>,
    pub leaders: [Option<u64>; 2],
    pub recent_join_penalty: [Option<Decimal>; 2],
    pub escort_bonus: [Option<Decimal>; 2],
    pub planes: [Vec<NavalPlane>; 2],
    pub group_counts: [u32; 2],
    pub member_positions: [BTreeMap<u32, [u32; 3]>; 2],
    pub retired: [Vec<NavalCombatParticipant>; 2],
    /// 14160ff40 writes disengage only when the native byte is nonzero.
    pub retreating: [Option<bool>; 2],
}
pub(super) fn naval(items: &[Item]) -> io::Result<NavalDetails> {
    let sides = [
        required_block(items, b"attacker")?,
        required_block(items, b"defender")?,
    ];
    let mut result = NavalDetails {
        day: optional_integer(items, b"day")?,
        duration: optional_integer(items, b"duration")?,
        progress: decimal(items, b"progress")?,
        ..Default::default()
    };
    for (index, side) in sides.into_iter().enumerate() {
        result.leaders[index] = field(side, b"last_leader")?.map(typed_handle).transpose()?;
        result.recent_join_penalty[index] = decimal(side, b"new_ships_positioning_penalty")?;
        result.escort_bonus[index] = decimal(side, b"positioning_dominance_bonus")?;
        for group in side.iter().filter(|x| x.key.as_deref() == Some(b"group")) {
            let group_index = result.group_counts[index];
            result.group_counts[index] += 1;
            let mut member_indices = [0, 0];
            let mut plane_indices = [0, 0];
            for item in block(&group.value)? {
                match item.key.as_deref() {
                    Some(b"air") | Some(b"airs_to_delete") => {
                        let source = usize::from(item.key.as_deref() == Some(b"airs_to_delete"));
                        let mut plane = naval_plane(block(&item.value)?, source as u8)?;
                        plane.group_index = group_index;
                        plane.index = plane_indices[source];
                        plane_indices[source] += 1;
                        result.planes[index].push(plane);
                    }
                    Some(b"member") | Some(b"members_to_delete") => {
                        let source = usize::from(item.key.as_deref() == Some(b"members_to_delete"));
                        let member =
                            naval_participant(block(&item.value)?, object_id(items)?, index as u8)?;
                        if result.member_positions[index]
                            .insert(
                                member.battle_slot,
                                [source as u32, group_index, member_indices[source]],
                            )
                            .is_some()
                        {
                            return Err(invalid("duplicate live/retired saved battle slot"));
                        }
                        member_indices[source] += 1;
                        if source == 1 {
                            result.retired[index].push(member);
                        }
                    }
                    _ => {}
                }
            }
        }
        result.retreating[index] = field(side, b"disengage")?.map(boolean).transpose()?;
    }
    Ok(result)
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalMemberDetails {
    pub arrival_hours: Option<i32>,
    pub last_target: Option<(bool, i32)>,
    pub criticals: Option<i32>,
    pub evacuation: Option<i32>,
    pub build_cost_ic: Option<Decimal>,
    pub cached_strength: Option<Decimal>,
    pub pride: Option<bool>,
    pub sunk_by: Option<Vec<u8>>,
    pub detection_state: Option<i32>,
    pub escape_progress: Option<Decimal>,
    pub received_damage: Option<Vec<Decimal>>,
    pub subunit_index: Option<i32>,
    pub formation_type: Option<i32>,
    pub name: Option<Vec<u8>>,
    pub type_name: Option<Vec<u8>>,
    pub sprite: Option<Vec<u8>>,
    pub variant: Option<u64>,
    pub convoy_client: Option<u64>,
    pub convoy_index: Option<i32>,
    pub owner: Option<Vec<u8>>,
    pub is_convoy: Option<bool>,
    pub hits: Vec<NavalHit>,
    pub damage_entries: Vec<NavalDamage>,
    pub air_hits: Vec<AirHit>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirHit {
    pub variant: Option<u64>,
    pub country: Vec<u8>,
    pub count: i32,
}
fn air_hits(items: &[Item]) -> io::Result<Vec<AirHit>> {
    items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"air_hit"))
        .map(|item| {
            let fields = block(&item.value)?;
            Ok(AirHit {
                variant: field(fields, b"equipment_variant_index")?
                    .map(typed_handle)
                    .transpose()?,
                country: required_bytes(fields, b"tag")?,
                count: integer(&required_bytes(fields, b"count")?)?,
            })
        })
        .collect()
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalHit {
    pub target: u32,
    pub name: Vec<u8>,
    pub convoy: bool,
    pub damage: Decimal,
    pub strength: Decimal,
    pub last_hit: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalDamage {
    pub ship: u64,
    pub country: Vec<u8>,
    pub damage: Decimal,
}
pub(super) fn naval_member(items: &[Item]) -> io::Result<NavalMemberDetails> {
    let cached = required_block(items, b"cached_info")?;
    let hits = naval_hits(items)?;
    let mut damage_entries = Vec::new();
    if let Some(value) = field(items, b"damage_received")? {
        for item in block(value)? {
            if item.key.is_some() {
                return Err(invalid("naval damage list contains named entry"));
            }
            let damage = block(&item.value)?;
            damage_entries.push(NavalDamage {
                ship: typed_handle(
                    field(damage, b"ship")?
                        .ok_or_else(|| invalid("naval damage lacks source ship"))?,
                )?,
                country: required_bytes(damage, b"tag")?,
                damage: Decimal::parse(&required_bytes(damage, b"damage")?)?,
            });
        }
    }
    Ok(NavalMemberDetails {
        arrival_hours: optional_integer(items, b"hours_to_arrive")?,
        last_target: field(items, b"last_target")?
            .map(|v| {
                let fields = block(v)?;
                Ok::<_, io::Error>((
                    required_bool(fields, b"convoy")?,
                    integer(&required_bytes(fields, b"size")?)?,
                ))
            })
            .transpose()?,
        criticals: optional_integer(items, b"critical_hits_received")?,
        evacuation: optional_integer(items, b"evacuated")?,
        build_cost_ic: decimal(cached, b"build_cost_ic")?,
        cached_strength: decimal(cached, b"strength")?,
        pride: field(cached, b"pride_of_the_fleet")?
            .map(boolean)
            .transpose()?,
        sunk_by: optional_bytes(cached, b"sunk_by")?,
        detection_state: optional_integer(items, b"hidden")?,
        escape_progress: decimal(items, b"escape_progress")?,
        received_damage: decimals(items, b"damage_received_by_gun_types")?,
        subunit_index: optional_integer(cached, b"index")?,
        formation_type: optional_integer(cached, b"type")?,
        name: optional_bytes(cached, b"ship")?,
        type_name: optional_bytes(cached, b"equipment_variant")?,
        sprite: optional_bytes(cached, b"sprite")?,
        variant: field(cached, b"highest_eq_variant")?
            .map(typed_handle)
            .transpose()?,
        convoy_client: field(cached, b"convoy_id")?.map(typed_handle).transpose()?,
        convoy_index: optional_integer(cached, b"convoy_index")?,
        owner: optional_bytes(cached, b"tag")?,
        is_convoy: field(cached, b"convoy")?.map(boolean).transpose()?,
        hits,
        damage_entries,
        air_hits: air_hits(items)?,
    })
}

fn naval_hits(items: &[Item]) -> io::Result<Vec<NavalHit>> {
    items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"naval_hit"))
        .map(|item| {
            let hit = block(&item.value)?;
            Ok(NavalHit {
                target: integer::<i32>(&required_bytes(hit, b"target")?)? as u32,
                name: required_bytes(hit, b"name")?,
                convoy: boolean(
                    field(hit, b"convoy")?.ok_or_else(|| invalid("naval hit lacks convoy flag"))?,
                )?,
                damage: Decimal::parse(&required_bytes(hit, b"damage")?)?,
                strength: Decimal::parse(&required_bytes(hit, b"strength")?)?,
                last_hit: boolean(
                    field(hit, b"last_hit")?
                        .ok_or_else(|| invalid("naval hit lacks last_hit flag"))?,
                )?,
            })
        })
        .collect()
}

pub(super) fn naval_participant(
    items: &[Item],
    combat: u64,
    side: u8,
) -> io::Result<NavalCombatParticipant> {
    Ok(NavalCombatParticipant {
        combat,
        side,
        battle_slot: integer::<i32>(&required_bytes(items, b"unique_id")?)? as u32,
        ship: field(items, b"ship")?.map(typed_handle).transpose()?,
        convoy: field(items, b"convoy")?.map(typed_handle).transpose()?,
        owner: required_bytes(required_block(items, b"cached_info")?, b"tag")?,
        state: integer::<i32>(&required_bytes(items, b"state")?)? as u32,
        details: naval_member(items)?,
    })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalPlane {
    pub source: u8,
    pub group_index: u32,
    pub index: u32,
    pub names: Vec<Vec<u8>>,
    pub carrier: Option<i32>,
    pub air_base: Option<u64>,
    pub wings: [Vec<u64>; 2],
    pub initial: i32,
    pub current: i32,
    pub casualties: i32,
    pub time_duration: i32,
    pub country: Vec<u8>,
    pub date: GameDate,
    pub flags: [bool; 2],
    pub name: Vec<u8>,
    pub damage_mult: Decimal,
    pub naval_hits: Vec<NavalHit>,
    pub air_hits: Vec<AirHit>,
}
fn required_bool(items: &[Item], key: &[u8]) -> io::Result<bool> {
    boolean(field(items, key)?.ok_or_else(|| invalid("combat flag missing"))?)
}
fn naval_plane(items: &[Item], source: u8) -> io::Result<NavalPlane> {
    Ok(NavalPlane {
        source,
        group_index: 0,
        index: 0,
        names: field(items, b"names")?
            .map(|v| {
                block(v)?
                    .iter()
                    .map(|item| {
                        if item.key.is_some() {
                            return Err(invalid("plane names contains keyed entry"));
                        }
                        Ok(scalar(&item.value)?.to_vec())
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
        carrier: optional_integer(items, b"carrier")?,
        air_base: field(items, b"air_base")?.map(typed_handle).transpose()?,
        wings: [
            handles(items, b"naval_strike")?,
            handles(items, b"air_wing")?,
        ],
        initial: integer(&required_bytes(items, b"max")?)?,
        current: integer(&required_bytes(items, b"alive")?)?,
        casualties: integer(&required_bytes(items, b"casualties")?)?,
        time_duration: integer(&required_bytes(items, b"time_duration")?)?,
        country: required_bytes(items, b"tag")?,
        date: GameDate::parse(&required_bytes(items, b"last_external_wave_date")?)?,
        flags: [
            required_bool(items, b"external_wave_complete")?,
            required_bool(items, b"external")?,
        ],
        name: required_bytes(items, b"name")?,
        damage_mult: Decimal::parse(&required_bytes(items, b"damage_mult")?)?,
        naval_hits: naval_hits(items)?,
        air_hits: air_hits(items)?,
    })
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResultSideDetails {
    pub leader: Option<u64>,
    pub planes: Vec<ResultPlane>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultPlane {
    pub initial: i32,
    pub current: i32,
    pub lost: Option<i32>,
    pub country: Vec<u8>,
    pub variant: u64,
    pub name: Vec<u8>,
    pub naval_hits: Vec<NavalHit>,
    pub air_hits: Vec<AirHit>,
}
pub(super) fn result_side(items: &[Item]) -> io::Result<ResultSideDetails> {
    Ok(ResultSideDetails {
        leader: field(items, b"last_leader")?
            .map(typed_handle)
            .transpose()?,
        planes: items
            .iter()
            .filter(|x| x.key.as_deref() == Some(b"air_wing"))
            .map(|x| {
                let items = block(&x.value)?;
                Ok(ResultPlane {
                    initial: integer(&required_bytes(items, b"max")?)?,
                    current: integer(&required_bytes(items, b"alive")?)?,
                    lost: optional_integer(items, b"killed")?,
                    country: required_bytes(items, b"tag")?,
                    variant: typed_handle(
                        field(items, b"equipment_variant_index")?
                            .ok_or_else(|| invalid("result plane lacks variant"))?,
                    )?,
                    name: required_bytes(items, b"air_base")?,
                    naval_hits: naval_hits(items)?,
                    air_hits: air_hits(items)?,
                })
            })
            .collect::<io::Result<_>>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fields(text: &[u8]) -> Vec<Item> {
        parser::parse(text).unwrap().items
    }
    #[test]
    fn land_preserves_order_full_handles_precision_and_absence() {
        let side=land_side(&fields(br#"front={id=7 type=8} front={id=9 type=10} reserves={id=7 type=11} air_damage_str=0.00001 prevented_damage_org=-0.25 org_loss_summary={1.125 -0.00001 0} org_loss_summary_index=2 num_org_losses=25 tactic=12 air_plane={amount=17}"#)).unwrap();
        assert_eq!(side.committed, vec![(7_u64 << 32) | 8, (9_u64 << 32) | 10]);
        assert_eq!(side.reserves, vec![(7_u64 << 32) | 11]);
        assert_eq!(side.damage[0].as_ref().unwrap().raw, 1);
        assert_eq!(side.damage[5].as_ref().unwrap().raw, -25000);
        assert!(side.damage[1].is_none());
        assert!(side.anti_air.is_none());
        assert_eq!(
            side.organisation_loss
                .unwrap()
                .iter()
                .map(|x| x.raw)
                .collect::<Vec<_>>(),
            [112500, -1, 0]
        );
        assert_eq!(side.air_counts, [17]);
        assert!(land_side(&fields(b"tactic=1 tactic=2")).is_err());
        assert!(land_side(&fields(b"org_loss_summary={bad=1}")).is_err());
    }
    #[test]
    fn naval_hits_keep_order_signed_slot_and_exact_damage() {
        let value=naval_member(&fields(br#"cached_info={tag="ENG" index=6 type=128 sprite="submarine"} hours_to_arrive=-1 naval_hit={target=-1 name="Convoy" convoy=yes damage=0.00001 strength=60 last_hit=no} damage_received_by_gun_types={0 1 2 3 4 5} damage_received={{ship={id=8 type=9} tag="FRA" damage=0.00002}}"#)).unwrap();
        assert_eq!(value.arrival_hours, Some(-1));
        assert_eq!(value.hits[0].target, u32::MAX);
        assert_eq!(value.hits[0].damage.raw, 1);
        assert_eq!(value.damage_entries[0].ship, (8_u64 << 32) | 9);
        assert_eq!(value.damage_entries[0].damage.raw, 2);
        assert!(value.escape_progress.is_none());
    }
    #[test]
    fn retired_live_slots_do_not_change_legacy_participant_census() {
        let items = fields(
            br#"id={id=7 type=60} location=1 progress=0.00001
          attacker={positioning=1 anti_air=0 group={
            member={unique_id=3 state=1 cached_info={tag=GER}}
            members_to_delete={unique_id=4 state=2 cached_info={tag=ENG}}
          }} defender={positioning=1 anti_air=0}"#,
        );
        let mut participants = BTreeMap::new();
        let value = parse_naval_combat(&items, &mut participants).unwrap();
        assert_eq!(participants.len(), 1);
        assert_eq!(participants.values().next().unwrap().battle_slot, 3);
        assert_eq!(value.details.retired[0][0].battle_slot, 4);
        assert_eq!(value.details.progress.unwrap().raw, 1);
        assert_eq!(value.details.group_counts, [1, 0]);
    }
    #[test]
    fn plane_and_result_air_records_preserve_distinct_names_and_conditional_kills() {
        let live = naval_plane(
            &fields(
                br#"carrier=-2 air_base={id=17 type=69}
            naval_strike={id=3 type=69} naval_strike={id=2 type=69}
            air_wing={id=4 type=69} max=10 alive=8 casualties=2 time_duration=3
            tag=GER last_external_wave_date=1939.1.1.1 external_wave_complete=yes external=no
            name="Live Wing" damage_mult=-0.00001"#,
            ),
            1,
        )
        .unwrap();
        assert_eq!(live.carrier, Some(-2));
        assert_eq!(live.wings[0], [(3_u64 << 32) | 69, (2_u64 << 32) | 69]);
        assert_eq!(live.damage_mult.raw, -1);
        let retained=result_side(&fields(br#"air_wing={max=5 alive=4 tag=GER equipment_variant_index={id=9 type=54} air_base="Retained Wing"}
            air_wing={max=8 alive=3 killed=5 tag=ENG equipment_variant_index={id=10 type=54} air_base="Second"}"#)).unwrap();
        assert_eq!(retained.planes[0].name, b"Retained Wing");
        assert_eq!(retained.planes[0].lost, None);
        assert_eq!(retained.planes[1].lost, Some(5));
    }
}
