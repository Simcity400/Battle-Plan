//! What the selected army's attack-modifier list reads, including river, fort, snow and
//! the fort base, that playback cannot take from other recorded fields or the recorded
//! content. The playback derivation computes the list from them.
//!
//! From [`ATTACK_INPUTS_FORMAT`] the rows record the inputs:
//!
//! - DivisionStatus: each division's stat-block operands
//!   ([`DivisionAttackStats`]), carried while no hooked writer rewrote the
//!   stat block, and its acclimatization factor.
//! - OrdersGroupCounterInputs: each group's direct commander's terrain
//!   adjusters and fort, river, snow and night values ([`CommanderAttackStats`]).
//! - CountryCounterInputs: each country's raw `land_night_attack` and
//!   `land_bunker_effectiveness_factor` accumulator entries
//!   ([`CountryAttackModifiers`]).
//! - Provinces: for a fortified province with a positive controller, its
//!   state's `land_bunker_effectiveness_factor` for that controller.
//! - WeatherRegions: the raw `army_attack_factor` and `breakthrough_factor`
//!   entries of the region's weather modifier ([`RegionCombatWeather`]).
//!
//! Everything else the list reads is recorded already or static content: the
//! province controllers and states, the direct-war cells
//! ([`crate::directed_war_derive`]), the four defines
//! (SimulationFeatures.runtime_scalars), the healthy land fort level (the
//! province's active ProvinceBuildings rows and each building's `land_fort`),
//! snow (WeatherProvinces, WeatherRegions), the night factor
//! ([`crate::day_night`]), the commander's identity (OrdersGroups), enemy air
//! superiority (RaidAirScores), and the terrains, crossings and regions of the
//! map.
//!
//! Recordings through format 101 recorded the native operands themselves
//! ([`DivisionAttackInputs`] and its siblings, each per-terrain value already the
//! combined stat-block and terrain adjuster attack), which still answer for those
//! recordings. From format 102 those fields are [`Witnessed`] packets: current writers
//! leave them empty, except for the divisions the collector's off-by-default
//! `attack_witness` diagnostic samples, which the recorder's derivation check compares
//! with the derived operands.
//!
//! [`ATTACK_INPUTS_FORMAT`]: crate::rows::compatibility::ATTACK_INPUTS_FORMAT
use super::*;

/// A packet only some rows hold: an empty payload is a row without it, and
/// any other payload is the packet. A current writer stores it only where a
/// diagnostic witnessed it, so it costs other rows nothing; older recordings
/// hold it in every row, whose payload is never empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Witnessed<T>(pub Option<T>);

impl<T> Witnessed<T> {
    pub fn get(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl<T: Value> Value for Witnessed<T> {
    const MINIMUM_ENCODED_BYTES: usize = 0;
    const HAS_STEPS: bool = T::HAS_STEPS;
    fn round_display(&mut self) {
        if let Some(value) = &mut self.0 {
            value.round_display();
        }
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        self.0.as_ref().map_or(Ok(0), Value::encoded_bytes)
    }
    fn recorded_bytes(&self) -> io::Result<usize> {
        self.0.as_ref().map_or(Ok(0), Value::recorded_bytes)
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.0.as_ref().map(Value::try_clone).transpose().map(Self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match &self.0 {
            Some(value) => value.encode_reserved(output),
            None => Ok(()),
        }
    }
    fn decode(input: &mut crate::codec::Decoder<'_>) -> io::Result<Self> {
        if input.remaining() == 0 {
            return Ok(Self(None));
        }
        T::decode(input).map(|value| Self(Some(value)))
    }
    fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
        if let Some(value) = &self.0 {
            value.visit_field_gaps(visit);
        }
    }
}

/// The division's stat block (the division's stat block pointer), as the attack list reads it. The
/// active-stat refresh rewrites it through equipment-stat calculation, the only caller writing a
/// live stat block, once a day per division and on ability refreshes; every value is an exact
/// fixed5 word.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionAttackStats {
    /// The terrain `unit adjuster` array, each entry's attack, in array order and as many as its
    /// live count. Terrain attack reads entry `index - 1` for the terrain whose database index is
    /// `index`, and adds the terrain's own adjuster.
    pub terrain: Vec<i64>,
    /// The river crossing pair the attack list adds to.
    pub river: [i64; 2],
    /// The fort pair.
    pub fort: [i64; 2],
    /// The snow term.
    pub snow: i64,
    /// The night term.
    pub night: i64,
}
value_struct!(DivisionAttackStats { terrain: Vec<i64>, river: [i64; 2], fort: [i64; 2], snow: i64, night: i64 });
opaque_value!(Option<DivisionAttackStats>);

/// The stat block's movement operands, read with [`DivisionAttackStats`] and carried under the same
/// certificate: each terrain adjuster's movement, parallel to [`DivisionAttackStats::terrain`]
/// (entry `i` serves terrain database index `i + 1`); the fort, river and snow adjusters' movement,
/// `None` when those adjusters did not authenticate; the word the movement speed's fuel capacity
/// scales (`MOVEMENT_FUEL_RATE`) and the anti-air word its air term reads (`MOVEMENT_AIR_DEFENSE`),
/// each raw fixed5.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionMovementStats {
    pub terrain: Vec<i64>,
    pub scalars: Option<[i64; 3]>,
    pub fuel_rate: i64,
    pub air_defense: i64,
}
value_struct!(DivisionMovementStats { terrain: Vec<i64>, scalars: Option<[i64; 3]>, fuel_rate: i64, air_defense: i64 });
opaque_value!(Option<DivisionMovementStats>);

impl DivisionMovementStats {
    /// The fort, river and snow movement words.
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

/// A direct commander's movement operands, read with [`CommanderAttackStats`]: each terrain
/// adjuster's movement, parallel to [`CommanderAttackStats::terrain`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommanderMovementStats {
    pub terrain: Vec<i64>,
}
value_struct!(CommanderMovementStats { terrain: Vec<i64> });
opaque_value!(Option<Option<CommanderMovementStats>>);

/// An orders group's direct commander (its direct-commander getter) as the attack list reads it.
/// Unit-leader and army-leader trait-adjuster refreshes rebuild every value from the leader's
/// traits, the field marshal's and the country's effects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommanderAttackStats {
    /// The commander's full `unit leader` reference.
    pub leader: u64,
    /// The terrain adjuster vector's attack values in order. The commander's terrain-adjuster
    /// lookup takes the entry at the terrain's database index and the first entry for an index
    /// outside the live count, even a count of zero; with a zero count the first entry is that
    /// retained one, so this holds one value there, and none only when the vector has no storage.
    pub terrain: Vec<i64>,
    /// The fort, river, snow and night adjusters' attack values.
    pub fort: i64,
    pub river: i64,
    pub snow: i64,
    pub night: i64,
}
value_struct!(CommanderAttackStats { leader: u64, terrain: Vec<i64>, fort: i64, river: i64, snow: i64, night: i64 });
opaque_value!(Option<Option<CommanderAttackStats>>);

/// A country modifier accumulator's raw entries for the two keys the attack list reads, `None` for
/// an absent key. The effective-value getter applies the key's ModifierDefinitions row to a present
/// one ([`crate::naval_mission_derive::effective`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountryAttackModifiers {
    /// `land_night_attack` (0x103), read for the division's owner.
    pub land_night_attack: Option<i64>,
    /// `land_bunker_effectiveness_factor` (0x272), read for the target's controller by
    /// the fort-base calculation.
    pub land_bunker_effectiveness_factor: Option<i64>,
}
value_struct!(CountryAttackModifiers { land_night_attack: Option<i64>, land_bunker_effectiveness_factor: Option<i64> });
opaque_value!(Option<CountryAttackModifiers>);

/// A country accumulator's raw entries for the movement speed and the attrition hover,
/// `None` for an absent key: the army speed, armour speed, attack speed, retreat speed,
/// out-of-supply and special-forces out-of-supply factors the movement speed adds or
/// scales its supply penalty by, and the attrition and winter and heat attrition factors
/// the attrition hover multiplies by. Country modifier recalculation rewrites them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
opaque_value!(Option<CountryMovementModifiers>);

/// A weather region's modifier (held by its weather record, returned through the weather manager):
/// raw entries, `None` for an absent key. Weather refresh rebuilds it from the region's weather
/// flags, the six static weather modifiers and the region's own weather states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionCombatWeather {
    /// `army_attack_factor` (0xAD).
    pub army_attack_factor: Option<i64>,
    /// `breakthrough_factor` (0x1DD).
    pub breakthrough_factor: Option<i64>,
    /// `army_speed_factor`, which the movement speed adds scaled by acclimatization.
    pub army_speed_factor: Option<i64>,
    /// `army_attack_speed_factor`, added the same way for a move into an enemy's
    /// province.
    pub army_attack_speed_factor: Option<i64>,
}
value_struct!(RegionCombatWeather { army_attack_factor: Option<i64>, breakthrough_factor: Option<i64>,
    army_speed_factor: Option<i64>, army_attack_speed_factor: Option<i64> });
opaque_value!(Option<RegionCombatWeather>);

// The native operands recordings through format 101 captured, decoded
// through their unchanged layouts. Each `sample_hour` is the packet's
// stamped read hour there; a witnessed packet of a current recording is
// read with its host row.

/// A division's operands: the stat block's scalars and, for every terrain of the live
/// database, the combined terrain attack (the stat block's entry plus the terrain's own
/// adjuster).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionAttackInputs {
    pub sample_hour: PacketHour,
    /// `Some(0)`: a null division template. Nonzero is a type-52 template handle.
    pub template: Option<u64>,
    /// Stat-block scalars; each refused alone.
    pub stats: AttackStats,
    /// One entry per terrain database index, ascending; `None` refuses the
    /// whole domain and an entry's `None` that terrain.
    pub terrain: Option<Vec<AttackTerrain>>,
    /// The freshly read acclimatization factor.
    pub acclimatization: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttackStats {
    /// The river crossing pair.
    pub river: [Option<i64>; 2],
    /// The fort pair.
    pub fort: [Option<i64>; 2],
    /// The snow term.
    pub snow: Option<i64>,
    /// The night term.
    pub night: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttackTerrain {
    pub terrain: i32,
    pub attack: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrdersGroupAttackInputs {
    pub sample_hour: PacketHour,
    /// `None`: refusal. `Some(None)`: a null direct commander.
    pub commander: Option<Option<CommanderAttackInputs>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommanderAttackInputs {
    pub leader: u64,
    /// The fort, river, snow and night adjusters' attack values.
    pub fort: Option<i64>,
    pub river: Option<i64>,
    pub snow: Option<i64>,
    pub night: Option<i64>,
    /// The commander's terrain attack for each terrain database index, ascending.
    pub terrain: Option<Vec<AttackTerrain>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeatherRegionAttackInputs {
    pub sample_hour: PacketHour,
    /// Effective `army_attack_factor` and `breakthrough_factor`.
    pub attack: Option<i64>,
    pub breakthrough: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountryAttackInputs {
    pub sample_hour: PacketHour,
    /// Effective `land_night_attack` and `land_bunker_effectiveness_factor`.
    pub land_night_attack: Option<i64>,
    pub land_bunker_effectiveness_factor: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProvinceAttackInputs {
    pub sample_hour: PacketHour,
    /// The healthy land fort level, the wrapping `i32` sum.
    pub fort_sum: Option<i32>,
    /// The state's effective factor for the controller; demanded only for a
    /// positive `fort_sum`.
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
opaque_value!(Witnessed<DivisionAttackInputs>);
opaque_value!(Witnessed<OrdersGroupAttackInputs>);
opaque_value!(Witnessed<WeatherRegionAttackInputs>);
opaque_value!(Witnessed<CountryAttackInputs>);
opaque_value!(Witnessed<ProvinceAttackInputs>);
opaque_value!(Witnessed<Option<CountryAttackInputs>>);
opaque_value!(Witnessed<u64>);

/// The terrain values of a recorded domain, ascending by terrain index.
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
    use crate::codec::Decoder;

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

    /// A current writer's empty witness costs no bytes and reads back
    /// empty in any format; a format-101 payload is its packet, stamp first.
    #[test]
    fn witnessed_packets_are_empty_unless_a_payload_holds_one() {
        let empty = Witnessed::<CountryAttackInputs>(None);
        assert!(round_trip(&empty).is_empty());
        for format in [STAMPED_PACKET_FORMAT, crate::FORMAT_VERSION] {
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
        // A witnessed packet of a current recording is read with its row.
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
