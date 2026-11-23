use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionAttackStats {
    pub terrain: Vec<i64>,
    pub river: [i64; 2],
    pub fort: [i64; 2],
    pub snow: i64,
    pub night: i64,
}
value_struct!(DivisionAttackStats { terrain: Vec<i64>, river: [i64; 2], fort: [i64; 2], snow: i64, night: i64 });
impl OpaqueValue for DivisionAttackStats {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionMovementStats {
    pub terrain: Vec<i64>,
    pub scalars: Option<[i64; 3]>,
    pub fuel_rate: i64,
    pub air_defense: i64,
}
value_struct!(DivisionMovementStats { terrain: Vec<i64>, scalars: Option<[i64; 3]>, fuel_rate: i64, air_defense: i64 });
impl OpaqueValue for DivisionMovementStats {}

impl DivisionMovementStats {
    pub fn fort(&self) -> Option<i64> {
        self.scalars.map(|[fort, _, _]| fort)
    }
    pub fn river(&self) -> Option<i64> {
        self.scalars.map(|[_, river, _]| river)
    }
    pub fn snow(&self) -> Option<i64> {
        self.scalars.map(|[_, _, snow]| snow)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CommanderMovementStats {
    pub terrain: Vec<i64>,
}
value_struct!(CommanderMovementStats { terrain: Vec<i64> });
impl OpaqueValue for CommanderMovementStats {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CommanderAttackStats {
    pub leader: u64,
    pub terrain: Vec<i64>,
    pub fort: i64,
    pub river: i64,
    pub snow: i64,
    pub night: i64,
}
value_struct!(CommanderAttackStats { leader: u64, terrain: Vec<i64>, fort: i64, river: i64, snow: i64, night: i64 });
impl OpaqueValue for CommanderAttackStats {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryAttackModifiers {
    pub land_night_attack: Option<i64>,
    pub land_bunker_effectiveness_factor: Option<i64>,
}
value_struct!(CountryAttackModifiers { land_night_attack: Option<i64>, land_bunker_effectiveness_factor: Option<i64> });
impl OpaqueValue for CountryAttackModifiers {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryMovementModifiers {
    pub army_speed_factor: Option<i64>,
    pub army_armor_speed_factor: Option<i64>,
    pub attrition: Option<i64>,
    pub winter_attrition_factor: Option<i64>,
    pub heat_attrition_factor: Option<i64>,
    pub out_of_supply_factor: Option<i64>,
    pub special_forces_out_of_supply_factor: Option<i64>,
    pub army_attack_speed_factor: Option<i64>,
    pub army_retreat_speed_factor: Option<i64>,
}
value_struct!(CountryMovementModifiers { army_speed_factor: Option<i64>,
    army_armor_speed_factor: Option<i64>, attrition: Option<i64>,
    winter_attrition_factor: Option<i64>, heat_attrition_factor: Option<i64>,
    out_of_supply_factor: Option<i64>, special_forces_out_of_supply_factor: Option<i64>,
    army_attack_speed_factor: Option<i64>, army_retreat_speed_factor: Option<i64> });
impl OpaqueValue for CountryMovementModifiers {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RegionCombatWeather {
    pub army_attack_factor: Option<i64>,
    pub breakthrough_factor: Option<i64>,
    pub army_speed_factor: Option<i64>,
    pub army_attack_speed_factor: Option<i64>,
}
value_struct!(RegionCombatWeather { army_attack_factor: Option<i64>, breakthrough_factor: Option<i64>,
    army_speed_factor: Option<i64>, army_attack_speed_factor: Option<i64> });
impl OpaqueValue for RegionCombatWeather {}


#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionAttackInputs {
    pub sample_hour: PacketHour,
    pub template: Option<u64>,
    pub stats: AttackStats,
    pub terrain: Option<Vec<AttackTerrain>>,
    pub acclimatization: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AttackStats {
    pub river: [Option<i64>; 2],
    pub fort: [Option<i64>; 2],
    pub snow: Option<i64>,
    pub night: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AttackTerrain {
    pub terrain: i32,
    pub attack: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OrdersGroupAttackInputs {
    pub sample_hour: PacketHour,
    pub commander: Option<Option<CommanderAttackInputs>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CommanderAttackInputs {
    pub leader: u64,
    pub fort: Option<i64>,
    pub river: Option<i64>,
    pub snow: Option<i64>,
    pub night: Option<i64>,
    pub terrain: Option<Vec<AttackTerrain>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WeatherRegionAttackInputs {
    pub sample_hour: PacketHour,
    pub attack: Option<i64>,
    pub breakthrough: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryAttackInputs {
    pub sample_hour: PacketHour,
    pub land_night_attack: Option<i64>,
    pub land_bunker_effectiveness_factor: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceAttackInputs {
    pub sample_hour: PacketHour,
    pub fort_sum: Option<i32>,
    pub land_bunker_effectiveness_factor: Option<i64>,
}

value_struct!(AttackStats { river: [Option<i64>;2], fort: [Option<i64>;2], snow: Option<i64>, night: Option<i64> });
value_struct!(AttackTerrain { terrain: i32, attack: Option<i64> });
value_struct!(DivisionAttackInputs { sample_hour: PacketHour, template: Option<u64>, stats: AttackStats, terrain: Option<Vec<AttackTerrain>>, acclimatization: Option<i64> });
value_struct!(OrdersGroupAttackInputs { sample_hour: PacketHour, commander: Option<Option<CommanderAttackInputs>> });
value_struct!(CommanderAttackInputs { leader: u64, fort: Option<i64>, river: Option<i64>, snow: Option<i64>, night: Option<i64>, terrain: Option<Vec<AttackTerrain>> });
value_struct!(WeatherRegionAttackInputs { sample_hour: PacketHour, attack: Option<i64>, breakthrough: Option<i64> });
value_struct!(CountryAttackInputs { sample_hour: PacketHour, land_night_attack: Option<i64>, land_bunker_effectiveness_factor: Option<i64> });
value_struct!(ProvinceAttackInputs { sample_hour: PacketHour, fort_sum: Option<i32>, land_bunker_effectiveness_factor: Option<i64> });
impl OpaqueValue for DivisionAttackInputs {}
impl OpaqueValue for OrdersGroupAttackInputs {}
impl OpaqueValue for WeatherRegionAttackInputs {}
impl OpaqueValue for CountryAttackInputs {}
impl OpaqueValue for ProvinceAttackInputs {}

pub fn attack_terrain(domain: &[AttackTerrain], terrain: i32) -> Result<Option<i64>, String> {
    if !domain
        .windows(2)
        .all(|pair| pair[0].terrain < pair[1].terrain)
    {
        return Err("attack terrain domain is not sorted and unique".into());
    }
    domain
        .binary_search_by_key(&terrain, |entry| entry.terrain)
        .map(|at| domain[at].attack)
        .map_err(|_| format!("terrain {terrain} is outside the recorded terrain domain"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_rows::codec::Decoder;

    fn round_trip<T: Value + PartialEq + std::fmt::Debug>(value: &T) -> Vec<u8> {
        let mut bytes = Vec::new();
        Value::encode(value, &mut bytes).unwrap();
        assert_eq!(Value::encoded_bytes(value).unwrap(), bytes.len());
        let mut input = Decoder::new(&bytes);
        assert_eq!(&T::decode(&mut input).unwrap(), value);
        input.finish().unwrap();
        bytes
    }

    #[test]
    fn inputs_round_trip_exact_signed_words() {
        round_trip(&Some(DivisionAttackStats {
            terrain: vec![i64::MIN, -15_000, 0, i64::MAX],
            river: [-30_000, i64::MIN + 1],
            fort: [0, -1],
            snow: 7,
            night: -50_000,
        }));
        round_trip(&Some(Some(CommanderAttackStats {
            leader: (9_u64 << 32) | 45,
            terrain: vec![10_000],
            fort: 1,
            river: -1,
            snow: i64::MAX,
            night: 0,
        })));
        round_trip(&Some(None::<CommanderAttackStats>));
        round_trip(&Some(CountryAttackModifiers {
            land_night_attack: None,
            land_bunker_effectiveness_factor: Some(-25_000),
        }));
        round_trip(&Some(RegionCombatWeather {
            army_attack_factor: Some(-10_000),
            breakthrough_factor: None,
            army_speed_factor: Some(i64::MIN),
            army_attack_speed_factor: Some(0),
        }));
        round_trip(&Some(CountryMovementModifiers {
            army_speed_factor: Some(-5_000),
            army_retreat_speed_factor: Some(i64::MAX),
            ..Default::default()
        }));
        round_trip(&Some(Some(CommanderMovementStats {
            terrain: vec![-1, 2],
        })));
        round_trip(&Some(DivisionMovementStats {
            terrain: vec![7],
            scalars: None,
            fuel_rate: -1,
            air_defense: 2,
        }));
    }

    #[test]
    fn witnessed_packets_are_empty_unless_a_payload_holds_one() {
        let empty = Witnessed::<CountryAttackInputs>(None);
        assert!(round_trip(&empty).is_empty());
        for format in [STAMPED_PACKET_FORMAT, replay_rows::FORMAT_VERSION] {
            let mut input = Decoder::new(&[]);
            input.format = format;
            assert_eq!(
                Witnessed::<CountryAttackInputs>::decode(&mut input).unwrap(),
                empty
            );
        }
        let packet = CountryAttackInputs {
            sample_hour: PacketHour::Host,
            land_night_attack: Some(-20_000),
            land_bunker_effectiveness_factor: None,
        };
        let mut wire = 1234_i32.to_le_bytes().to_vec();
        Value::encode(&packet, &mut wire).unwrap();
        let mut input = Decoder::new(&wire);
        input.format = STAMPED_PACKET_FORMAT;
        let decoded = Witnessed::<CountryAttackInputs>::decode(&mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(
            decoded.get(),
            Some(&CountryAttackInputs {
                sample_hour: PacketHour::Stamped(1234),
                ..packet
            })
        );
        let witnessed = Witnessed(Some(packet));
        assert!(!round_trip(&witnessed).is_empty());
    }

    #[test]
    fn recorded_terrain_domains_are_searched_by_index() {
        let domain = [
            AttackTerrain {
                terrain: 1,
                attack: Some(-5_000),
            },
            AttackTerrain {
                terrain: 4,
                attack: None,
            },
        ];
        assert_eq!(attack_terrain(&domain, 1), Ok(Some(-5_000)));
        assert_eq!(attack_terrain(&domain, 4), Ok(None));
        assert!(attack_terrain(&domain, 2).is_err());
        let unsorted = [domain[1], domain[0]];
        assert!(attack_terrain(&unsorted, 1).is_err());
    }
}
