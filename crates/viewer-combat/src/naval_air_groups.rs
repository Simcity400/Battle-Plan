use super::keyed::Keyed;
use engine_executable::modifier_keys as keys;
use content_reader_layers::ResolvedLayers;
use content_reader_politics::static_modifiers::{self, Request, Resolved};
use content_reader_naval_combat_defines::naval_combat_defines::Defines;
use replay_data::{
    rows::AirBaseRow, rows::AirWingRow, rows::NavalCombatAirBaseWingHandlesRow,
    rows::NavalCombatAirEfficiency, rows::NavalCombatPlane, rows::NavalCombatShipRow,
    rows::NavalCombatSideDetails, rows::ShipModelRow, rows::ShipRow,
};
use std::collections::{BTreeMap, btree_map::Entry};

const ONE: i64 = 100_000;

const SCREENING_MODIFIERS: [&str; 2] = ["screening_bonus", "capital_screening_bonus"];
const SORTIE: [u32; 2] = [keys::SORTIE_EFFICIENCY, keys::FIGHTER_SORTIE_EFFICIENCY];
const SORTIE_KEYS: [&str; 2] = ["sortie_efficiency", "fighter_sortie_efficiency"];

#[derive(Default)]
pub(super) struct Content {
    screening: [[Option<i64>; 2]; 2],
    equipment: Option<std::sync::Arc<content_reader_key_names::equipment::Database>>,
}

impl Content {
    pub(super) fn load(
        layers: &ResolvedLayers,
        warnings: &mut Vec<String>,
        equipment: std::sync::Arc<content_reader_key_names::equipment::Database>,
    ) -> Self {
        let requests: Vec<_> = SCREENING_MODIFIERS
            .iter()
            .flat_map(|&modifier| {
                SORTIE_KEYS
                    .iter()
                    .map(move |&key| Request { modifier, key })
            })
            .collect();
        let equipment = if let Some(reason) = &equipment.lost {
            warnings.push(format!(
                "Naval air entries: stacked equipment definitions are unavailable: {reason}."
            ));
            None
        } else {
            Some(equipment)
        };
        let mut content = match static_modifiers::read(layers, &requests) {
            Ok(read) => {
                warnings.extend(read.notes.iter().map(|note| {
                    format!(
                        "Static modifiers content: {note}; the values follow the game's reading."
                    )
                }));
                Self::from_values(read.values, warnings)
            }
            Err(error) => {
                warnings.push(format!(
                    "Static modifiers ({}): {error}; the screening sortie terms of carrier air positioning are unavailable.",
                    static_modifiers::DIRECTORY
                ));
                Self::default()
            }
        };
        content.equipment = equipment;
        content
    }

    pub(super) fn equipment(&self) -> Option<&content_reader_key_names::equipment::Database> {
        self.equipment.as_deref()
    }

    pub(super) fn shared_equipment(
        &self,
    ) -> Option<std::sync::Arc<content_reader_key_names::equipment::Database>> {
        self.equipment.clone()
    }

    fn from_values(values: Vec<Resolved>, warnings: &mut Vec<String>) -> Self {
        let total = values.len();
        let mut refused = 0_usize;
        let mut screening = [[None; 2]; 2];
        for (index, value) in values.into_iter().enumerate() {
            screening[index / 2][index % 2] = match value {
                Ok(raw) => Some(raw),
                Err(refusal) => {
                    refused += 1;
                    warnings.push(format!(
                        "Static modifier {}.{} in {}: {}; carrier air positioning that needs it is unavailable.",
                        refusal.modifier, refusal.key, refusal.file, refusal.reason
                    ));
                    None
                }
            };
        }
        if refused > 0 {
            warnings.push(format!(
                "Static modifiers: {refused} of {total} screening sortie values refused."
            ));
        }
        Self {
            screening,
            equipment: None,
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "native sortie law has independent operands"
)]
fn efficiency_parts(
    plane: &NavalCombatPlane,
    combat_duration: i32,
    side_index: usize,
    member: Option<&NavalCombatShipRow>,
    host: Option<CarrierHost>,
    side: &NavalCombatSideDetails,
    screening: &[[Option<i64>; 2]; 2],
    defines: Option<&Defines>,
) -> Option<(i64, i64, i64)> {
    let e = &plane.efficiency;
    match e.source {
        0 => return Some((0, 0, ONE)),
        1 | 2 => return Some((ONE, ONE, ONE)),
        3 => (),
        _ => return None,
    }
    let mul = |a: i64, b: i64| a.wrapping_mul(b) / ONE;
    let value = |index: usize| -> Option<i64> {
        let term = &e.modifiers[index];
        if term.present == 0 {
            return Some(0);
        }
        metadata_value(e, usize::from(index == 0), term.raw)
    };
    let weather = ONE.wrapping_add(value(1)?).clamp(0, ONE);
    let night = ONE
        .wrapping_add(value(2)?)
        .wrapping_add(if e.night > 0 { value(0)? } else { 0 })
        .clamp(0, ONE);
    let defines = defines?;
    let mut base = if combat_duration < defines.selected_sortie_initial_time {
        let mission_factor = defines.mission_factor(e.mission)?;
        if side_index == 0 {
            mission_factor
        } else {
            ONE.wrapping_sub(mission_factor)
        }
    } else {
        defines.base_carrier_sortie_efficiency
    };
    let host = host?;
    if plane.carrier_member_present != 0 {
        let carrier = member
            .filter(|row| row.member_source == 0 && row.live_ship != 0 && row.ship == host.ship)?;
        if !carrier
            .weapon_context
            .sources
            .iter()
            .any(|source| source.scope == 26)
        {
            return None;
        }
        if plane.carrier_slot >= 0 && carrier.battle_slot != plane.carrier_slot as u32 {
            return None;
        }
        for (lane, key) in SORTIE.into_iter().enumerate() {
            if lane == 1 && e.pool_type_flags & 0x100000400 == 0 {
                continue;
            }
            base = base
                .wrapping_add(super::naval_environment::modifier(
                    &carrier.weapon_context,
                    key,
                )?)
                .wrapping_add(screening_extra(
                    carrier,
                    host.pool_flags,
                    e,
                    side,
                    screening,
                    lane,
                )?);
        }
    }
    let crowding = if e.container_present != 0 {
        e.base_crowding
    } else {
        0
    };
    let base = mul(night, mul(weather, mul(base, mul(crowding, ONE)))).max(0);
    let maximum = host.maximum_organisation;
    let organisation = host.organisation;
    let valid = (organisation as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd;
    let ratio = if maximum != 0 && valid {
        organisation.wrapping_mul(ONE).checked_div(maximum)?
    } else {
        0xffff_ffff
    };
    Some((base, mul(ratio, base).max(0), ratio))
}

#[expect(
    clippy::too_many_arguments,
    reason = "native sortie law has independent operands"
)]
fn efficiency(
    plane: &NavalCombatPlane,
    combat_duration: i32,
    side_index: usize,
    member: Option<&NavalCombatShipRow>,
    host: Option<CarrierHost>,
    side: &NavalCombatSideDetails,
    screening: &[[Option<i64>; 2]; 2],
    defines: Option<&Defines>,
) -> Option<i64> {
    efficiency_parts(
        plane,
        combat_duration,
        side_index,
        member,
        host,
        side,
        screening,
        defines,
    )
    .map(|(_, final_value, _)| final_value)
}

pub(super) fn display_efficiency(
    plane: &NavalCombatPlane,
    side: &NavalCombatSideDetails,
    side_index: u8,
    duration: i32,
    joins: &CarrierJoins<'_>,
    content: &Content,
    defines: Option<&Defines>,
) -> Option<i64> {
    display_efficiency_parts(plane, side, side_index, duration, joins, content, defines)
        .map(|(_, final_value, _)| final_value)
}

pub(super) fn display_efficiency_parts(
    plane: &NavalCombatPlane,
    side: &NavalCombatSideDetails,
    side_index: u8,
    duration: i32,
    joins: &CarrierJoins<'_>,
    content: &Content,
    defines: Option<&Defines>,
) -> Option<(i64, i64, i64)> {
    let host = if plane.efficiency.source == 3 {
        Some(joins.host(plane)?)
    } else {
        None
    };
    let member = if plane.efficiency.source == 3 && plane.carrier_member_present != 0 {
        Some(joins.member(side_index, plane.group_index, host?.ship)?)
    } else {
        None
    };
    efficiency_parts(
        plane,
        duration,
        side_index as usize,
        member,
        host,
        side,
        &content.screening,
        defines,
    )
}

pub(super) fn display_screening_extra(
    plane: &NavalCombatPlane,
    side: &NavalCombatSideDetails,
    side_index: u8,
    joins: &CarrierJoins<'_>,
    content: &Content,
    lane: usize,
) -> Option<i64> {
    if plane.carrier_member_present == 0 {
        return Some(0);
    }
    let host = joins.host(plane)?;
    let member = joins.member(side_index, plane.group_index, host.ship)?;
    screening_extra(
        member,
        host.pool_flags,
        &plane.efficiency,
        side,
        &content.screening,
        lane,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CarrierHost {
    ship: u64,
    organisation: i64,
    maximum_organisation: i64,
    pool_flags: u64,
}

type Members<'a> = BTreeMap<(u8, u32, u64), Option<&'a NavalCombatShipRow>>;

pub(super) struct CarrierJoins<'a> {
    members: Option<Members<'a>>,
    bases: Option<Keyed<'a, AirBaseRow>>,
    live_ships: Option<Keyed<'a, ShipRow>>,
    models: Option<Keyed<'a, ShipModelRow>>,
}

impl<'a> CarrierJoins<'a> {
    pub(super) fn new(
        members: Option<&[&'a NavalCombatShipRow]>,
        bases: Option<&'a [AirBaseRow]>,
        live_ships: Option<&'a [ShipRow]>,
        models: Option<Keyed<'a, ShipModelRow>>,
    ) -> Self {
        let members = members.map(|rows| {
            let mut index = Members::new();
            for &row in rows.iter().filter(|row| row.member_source == 0) {
                index
                    .entry((row.side, row.group_index, row.ship))
                    .and_modify(|found| *found = None)
                    .or_insert(Some(row));
            }
            index
        });
        Self {
            members,
            bases: bases.and_then(|rows| Keyed::new(rows, |row| row.id)),
            live_ships: live_ships.and_then(|rows| Keyed::new(rows, |row| row.id)),
            models,
        }
    }

    fn host(&self, plane: &NavalCombatPlane) -> Option<CarrierHost> {
        let ship = self.bases.as_ref()?.get(plane.air_base)?.host;
        if ship == 0 {
            return None;
        }
        let live = self.live_ships.as_ref()?.get(ship)?;
        let counter = self.models.as_ref()?.get(ship)?.counter.as_ref()?;
        Some(CarrierHost {
            ship,
            organisation: live.organisation,
            maximum_organisation: counter.maximum_organisation,
            pool_flags: counter.positive_equipment_flags,
        })
    }

    fn member(&self, side: u8, group: u32, ship: u64) -> Option<&'a NavalCombatShipRow> {
        *self.members.as_ref()?.get(&(side, group, ship))?
    }

    pub(super) fn carrier_member(
        &self,
        side: u8,
        plane: &NavalCombatPlane,
    ) -> Option<&'a NavalCombatShipRow> {
        let host = self.host(plane)?;
        self.member(side, plane.group_index, host.ship)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PositioningAir {
    pub(super) weighted_planes: i64,
    pub(super) average_base_fill: i64,
}

pub(super) struct PositioningAirSources<'a> {
    pub(super) side_index: u8,
    pub(super) duration: i32,
    pub(super) joins: &'a CarrierJoins<'a>,
    pub(super) base_aircraft: Option<&'a BTreeMap<u64, i32>>,
    pub(super) content: &'a Content,
    pub(super) defines: Option<&'a Defines>,
}

pub(super) fn positioning_air(
    side: &NavalCombatSideDetails,
    sources: &PositioningAirSources<'_>,
) -> Option<PositioningAir> {
    let mut weighted_planes = 0_i64;
    let mut sum_fill = 0_i64;
    let mut admitted = 0_i32;
    for plane in side
        .planes
        .iter()
        .filter(|plane| plane.source == 0 && plane.group_index != u32::MAX && plane.flags[1] == 0)
    {
        let host = if plane.efficiency.source == 3 {
            Some(sources.joins.host(plane)?)
        } else {
            None
        };
        let ship = if plane.efficiency.source == 3 && plane.carrier_member_present != 0 {
            Some(
                sources
                    .joins
                    .member(sources.side_index, plane.group_index, host?.ship)?,
            )
        } else {
            None
        };
        let efficiency = efficiency(
            plane,
            sources.duration,
            sources.side_index as usize,
            ship,
            host,
            side,
            &sources.content.screening,
            sources.defines,
        )?;
        weighted_planes = weighted_planes.wrapping_add(
            i64::from(plane.current_planes)
                .wrapping_mul(efficiency)
                .wrapping_mul(ONE)
                / ONE,
        );
        let fill = match plane.efficiency.source {
            0 | 1 => 0,
            2 | 3 => {
                let count = *sources.base_aircraft?.get(&plane.air_base)?;
                let denominator = sources.defines?.summary_fixed[12];
                if denominator == 0 {
                    0xffff_ffff
                } else {
                    i64::from(count)
                        .wrapping_mul(10_000_000_000)
                        .checked_div(denominator)?
                }
            }
            _ => return None,
        }
        .min(ONE);
        sum_fill = sum_fill.wrapping_add(fill.wrapping_mul(efficiency) / ONE);
        admitted = admitted.wrapping_add(1);
    }
    let average_base_fill = if admitted > 0 {
        let denominator = i64::from(admitted).wrapping_mul(ONE);
        if denominator == 0
            || (sum_fill as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
        {
            0xffff_ffff
        } else {
            sum_fill.wrapping_mul(ONE).checked_div(denominator)?
        }
    } else {
        sum_fill
    };
    Some(PositioningAir {
        weighted_planes,
        average_base_fill,
    })
}

#[derive(Clone, Copy)]
pub(super) struct DenseBases<'a> {
    rows: Keyed<'a, NavalCombatAirBaseWingHandlesRow>,
    wings: Keyed<'a, AirWingRow>,
}

impl<'a> DenseBases<'a> {
    pub(super) fn new(
        rows: &'a [NavalCombatAirBaseWingHandlesRow],
        wings: &'a [AirWingRow],
    ) -> Option<Self> {
        Some(Self {
            rows: Keyed::new(rows, |row| row.base)?,
            wings: Keyed::new(wings, |wing| wing.id)?,
        })
    }

    pub(super) fn counts(
        &self,
        bases: impl IntoIterator<Item = u64>,
    ) -> Option<BTreeMap<u64, i32>> {
        let mut counts = BTreeMap::new();
        for base in bases {
            let Entry::Vacant(slot) = counts.entry(base) else {
                continue;
            };
            let Some(row) = self.rows.get(base) else {
                continue;
            };
            if base == 0 {
                return None;
            }
            let mut count = 0_i32;
            for handle in &row.wings {
                count = count.wrapping_add(self.wings.get(*handle)?.planes);
            }
            slot.insert(count);
        }
        Some(counts)
    }
}

pub(super) fn metadata_value(e: &NavalCombatAirEfficiency, row: usize, raw: i64) -> Option<i64> {
    let [flags, selector] = e.modifier_metadata[row];
    if selector != 0 {
        match e.eligibility_callback {
            0 => return Some(0),
            1 if selector & e.feature_flags == 0 => return Some(0),
            1 => (),
            _ => return None,
        }
    }
    Some(if flags & 0x10 != 0 {
        raw.clamp(0, ONE)
    } else {
        raw
    })
}

pub(super) fn display_box(row: &NavalCombatShipRow, pool_flags: u64) -> u8 {
    match row.state {
        4 | 16 => return 7,
        32 => return 6,
        1 | 2 | 8 => (),
        _ => return 0,
    }
    if row.live_convoy != 0 {
        return 3;
    }
    if row.live_ship == 0 {
        return 0;
    }
    let flags = row.formation_flags;
    let carrier_bits = (1 << 22) | (1 << 37);
    let carrier = flags & carrier_bits != 0 || pool_flags & carrier_bits != 0;
    if carrier && flags & (1 << 7) == 0 {
        3
    } else if flags & (1 << 7) != 0 {
        4 + u8::from(row.detection_state != 0)
    } else if flags & (1 << 8) != 0 {
        1
    } else {
        2 + u8::from(flags & (1 << 39) != 0)
    }
}

fn screening_extra(
    carrier: &NavalCombatShipRow,
    pool_flags: u64,
    e: &NavalCombatAirEfficiency,
    side: &NavalCombatSideDetails,
    screening: &[[Option<i64>; 2]; 2],
    lane: usize,
) -> Option<i64> {
    let combat_box = display_box(carrier, pool_flags);
    if !matches!(combat_box, 2 | 3) {
        return Some(0);
    }
    let mul = |a: i64, b: i64| a.wrapping_mul(b) / ONE;
    let term = |raw: i64| metadata_value(e, 2 + lane, raw);
    let mut value = if side.capital_screening > 0 {
        mul(side.capital_screening, term(screening[0][lane]?)?)
    } else {
        0
    };
    if combat_box == 3 {
        let combined = ONE.wrapping_sub(mul(
            ONE.wrapping_sub(side.carrier_screening),
            ONE.wrapping_sub(side.capital_screening),
        ));
        if combined > 0 {
            value = value.wrapping_add(mul(term(screening[1][lane]?)?, combined));
        }
    }
    Some(value)
}

#[cfg(test)]
mod tests;
