use engine_executable::modifier_keys;
use content_reader_layers::ResolvedLayers;
use content_reader_politics::static_modifiers::{self, Request};
use replay_data::{
    rows::ModifierDefinitionRow, rows::NavalCombatShipRow, rows::NavalCombatSideDetails,
    rows::NavalShipWeaponContext,
};
use std::collections::BTreeMap;

const ONE: i64 = 100_000;
const SCREENING_NAMES: [&str; 2] = ["screening_bonus", "capital_screening_bonus"];
pub(super) const ESCAPE_KEYS: [u32; 4] = [
    modifier_keys::NAVAL_RETREAT_SPEED,
    modifier_keys::SUB_RETREAT_SPEED,
    modifier_keys::NAVAL_RETREAT_SPEED_AFTER_INITIAL_COMBAT,
    modifier_keys::CONVOY_RETREAT_SPEED,
];

#[derive(Debug)]
pub(super) struct Screening {
    values: [[Result<i64, String>; 4]; 2],
}

impl Screening {
    #[cfg(test)]
    pub(super) fn zero() -> Self {
        Self {
            values: std::array::from_fn(|_| std::array::from_fn(|_| Ok(0))),
        }
    }

    pub(super) fn load(layers: &ResolvedLayers) -> Self {
        let requests: Vec<_> = SCREENING_NAMES
            .iter()
            .flat_map(|&modifier| {
                ESCAPE_KEYS.iter().map(move |&key| Request {
                    modifier,
                    key: engine_executable::modifier_keys::fixed_name(key)
                        .unwrap_or_else(|error| panic!("{error}")),
                })
            })
            .collect();
        let values = match static_modifiers::read(layers, &requests) {
            Ok(read) => read
                .values
                .into_iter()
                .map(|item| {
                    item.map_err(|error| {
                        format!(
                            "{}.{} in {}: {}",
                            error.modifier, error.key, error.file, error.reason
                        )
                    })
                })
                .collect::<Vec<_>>(),
            Err(error) => (0..8).map(|_| Err(error.to_string())).collect(),
        };
        let mut values = values.into_iter();
        Self {
            values: std::array::from_fn(|_| {
                std::array::from_fn(|_| {
                    values.next().unwrap_or_else(|| {
                        Err(
                            "static modifier reader returned fewer escape values than requested"
                                .into(),
                        )
                    })
                })
            }),
        }
    }
    pub(super) fn value(&self, box_kind: usize, key: u32) -> Result<i64, String> {
        let lane = ESCAPE_KEYS
            .iter()
            .position(|value| *value == key)
            .ok_or_else(|| format!("unsupported escape modifier key {key:#x}"))?;
        self.values[box_kind][lane].clone()
    }
}

fn static_value(
    raw: i64,
    definition: &ModifierDefinitionRow,
    side: &NavalCombatSideDetails,
) -> Result<i64, String> {
    if definition.feature_selector != 0 {
        match side.summary.eligibility_callback {
            0 => return Ok(0),
            1 if definition.feature_selector & side.summary.feature_flags == 0 => return Ok(0),
            1 => (),
            value => {
                return Err(format!(
                    "escape modifier eligibility callback {value} is unsupported"
                ));
            }
        }
    }
    Ok(if definition.flags & 0x10 != 0 {
        raw.clamp(0, ONE)
    } else {
        raw
    })
}

fn pool_flags(row: &NavalCombatShipRow) -> u64 {
    row.weapon_context
        .sources
        .iter()
        .filter(|source| source.scope == 27 && source.weights[0] > 0)
        .fold(0_u64, |flags, source| flags | source.weights[1] as u64)
}

pub(super) fn modifier_value(
    row: &NavalCombatShipRow,
    side: &NavalCombatSideDetails,
    key: u32,
    side_total: Result<i64, String>,
    screening: &Screening,
    definitions: &BTreeMap<u32, &ModifierDefinitionRow>,
) -> Result<(i64, i64), String> {
    let mut value = if row.live_ship != 0 {
        super::naval_environment::modifier(&row.weapon_context, key)
            .ok_or_else(|| format!("state-8 ship modifier {key:#x} source unsupported"))?
    } else {
        side_total?
    };
    let display_box = super::naval_air_groups::display_box(row, pool_flags(row));
    let mut screening_value = 0;
    if matches!(display_box, 2 | 3) {
        let definition = definitions
            .get(&key)
            .ok_or_else(|| format!("state-8 modifier {key:#x} has no recorded definition"))?;
        if side.capital_screening > 0 {
            screening_value = side.capital_screening.wrapping_mul(static_value(
                screening.value(0, key)?,
                definition,
                side,
            )?) / ONE;
        }
        if display_box == 3 {
            let combined = ONE.wrapping_sub(
                ONE.wrapping_sub(side.carrier_screening)
                    .wrapping_mul(ONE.wrapping_sub(side.capital_screening))
                    / ONE,
            );
            if combined > 0 {
                screening_value = screening_value.wrapping_add(
                    combined.wrapping_mul(static_value(
                        screening.value(1, key)?,
                        definition,
                        side,
                    )?) / ONE,
                );
            }
        }
        value = value.wrapping_add(screening_value);
    }
    Ok((value, screening_value))
}

#[derive(Clone, Debug)]
pub(super) struct Defines {
    pub base: i64,
    pub per_day: i64,
    pub duration_max: i64,
    pub speed_factor: i64,
    pub hidden_sub: i64,
    pub sub_base: i64,
    pub out_of_fuel: i64,
    pub initial_duration: i32,
}

impl Defines {
    pub(super) fn read(numbers: &content_reader_defines::defines::Defines) -> Result<Self, String> {
        let fixed = |name| {
            numbers
                .fixed5("NNavy", name)
                .map_err(|error| error.to_string())
        };
        Ok(Self {
            base: fixed("BASE_ESCAPE_SPEED")?,
            per_day: fixed("ESCAPE_SPEED_PER_COMBAT_DAY")?,
            duration_max: fixed("MAX_ESCAPE_SPEED_FROM_COMBAT_DURATION")?,
            speed_factor: fixed("SPEED_TO_ESCAPE_SPEED")?,
            hidden_sub: fixed("ESCAPE_SPEED_HIDDEN_SUB")?,
            sub_base: fixed("ESCAPE_SPEED_SUB_BASE")?,
            out_of_fuel: fixed("OUT_OF_FUEL_SPEED_FACTOR")?,
            initial_duration: numbers
                .int32("NNavy", "COMBAT_INITIAL_DURATION")
                .map_err(|error| error.to_string())?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Parts {
    pub base: i64,
    pub duration: i64,
    pub modifiers: i64,
    pub ship_speed: i64,
    pub detection: i64,
    pub total: i64,
}

fn guard(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

fn terrain_speed(context: &NavalShipWeaponContext) -> i64 {
    if (context.ship_class as i32) >= 0 {
        context
            .sources
            .iter()
            .filter(|source| source.scope == 20)
            .fold(0_i64, |sum, source| sum.wrapping_add(source.movement))
    } else {
        let (sum, count) = context
            .sources
            .iter()
            .filter(|source| source.scope == 25)
            .fold((0_i64, 0_i64), |(sum, count), source| {
                (sum.wrapping_add(source.movement), count + 1)
            });
        if count == 0 {
            0
        } else if guard(sum) {
            sum.wrapping_mul(ONE) / count.wrapping_mul(ONE)
        } else {
            u32::MAX as i64
        }
    }
}

fn fuel_fraction([current, capacity]: [i64; 2]) -> i64 {
    if capacity < 1 || !guard(current) {
        return ONE;
    }
    current
        .wrapping_mul(ONE)
        .wrapping_div(capacity)
        .clamp(0, ONE)
}

pub(super) fn ship_speed(row: &NavalCombatShipRow, speed_modifier: i64, defines: &Defines) -> i64 {
    if row.live_ship == 0 {
        return row.escape_speed_base;
    }
    let context = &row.weapon_context;
    let mut modifier = speed_modifier.wrapping_add(terrain_speed(context));
    let fuel = fuel_fraction(context.fuel);
    if fuel < ONE {
        modifier =
            modifier.wrapping_add(ONE.wrapping_sub(fuel).wrapping_mul(defines.out_of_fuel) / ONE);
    }
    ONE.wrapping_add(modifier)
        .max(10_000)
        .wrapping_mul(row.escape_speed_base)
        / ONE
}

pub(super) fn chance(
    row: &NavalCombatShipRow,
    duration_hours: i32,
    modifier: i64,
    speed_modifier: i64,
    defines: &Defines,
) -> Parts {
    let duration_product = i64::from(duration_hours)
        .wrapping_mul(ONE)
        .wrapping_mul(defines.per_day)
        / ONE;
    let duration = if guard(duration_product) {
        duration_product.wrapping_mul(ONE) / 2_400_000
    } else {
        u32::MAX as i64
    }
    .min(defines.duration_max);
    let speed = ship_speed(row, speed_modifier, defines);
    let speed_product = speed.wrapping_mul(defines.speed_factor) / ONE;
    let ship_speed = if guard(speed_product) {
        speed_product.wrapping_mul(ONE) / 10_000_000
    } else {
        u32::MAX as i64
    };
    let detection = if row.detection_state == 0 {
        defines.hidden_sub
    } else if row.live_ship != 0 && row.formation_flags & (1 << 7) != 0 {
        defines.sub_base
    } else {
        0
    };
    let total = defines
        .base
        .wrapping_add(duration)
        .wrapping_add(modifier)
        .wrapping_add(ship_speed)
        .wrapping_add(detection)
        .max(0);
    Parts {
        base: defines.base,
        duration,
        modifiers: modifier,
        ship_speed,
        detection,
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defines() -> Defines {
        Defines {
            base: 10_000,
            per_day: 20_000,
            duration_max: 25_000,
            speed_factor: 30_000,
            hidden_sub: 5_000,
            sub_base: 3_000,
            out_of_fuel: -50_000,
            initial_duration: 24,
        }
    }

    #[test]
    fn convoy_escape_uses_the_side_root_sum() {
        let key = 0x5d;
        let row = NavalCombatShipRow {
            state: 8,
            live_convoy: 1,
            detection_state: 1,
            ..Default::default()
        };
        let definition = ModifierDefinitionRow {
            index: key,
            ..Default::default()
        };
        let screening = Screening {
            values: std::array::from_fn(|_| std::array::from_fn(|_| Ok(0))),
        };
        let (modifier, screen) = modifier_value(
            &row,
            &NavalCombatSideDetails::default(),
            key,
            Ok(10_000),
            &screening,
            &BTreeMap::from([(key, &definition)]),
        )
        .unwrap();
        assert_eq!((modifier, screen), (10_000, 0));
        assert_eq!(chance(&row, 0, modifier, 0, &defines()).total, 20_000);
        assert!(
            modifier_value(
                &row,
                &NavalCombatSideDetails::default(),
                key,
                Err("trees unavailable".into()),
                &screening,
                &BTreeMap::from([(key, &definition)]),
            )
            .is_err()
        );
    }

    #[test]
    fn ship_speed_uses_stored_base_terrain_fuel_and_default_leader_modifier() {
        let row = NavalCombatShipRow {
            live_ship: 1,
            escape_speed_base: 200_000,
            weapon_context: NavalShipWeaponContext {
                fuel: [50_000, 100_000],
                sources: vec![replay_data::rows::NavalShipModifierSource {
                    scope: 20,
                    movement: 10_000,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(ship_speed(&row, 20_000, &defines()), 210_000);
        let result = chance(&row, 24, 4_000, 20_000, &defines());
        assert_eq!(result.duration, 20_000);
        assert_eq!(result.ship_speed, 630);
        assert_eq!(result.total, 39_630);
    }
}
