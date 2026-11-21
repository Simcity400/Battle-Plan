use super::keyed::Keyed;
use super::naval_air_groups::{
    self, CarrierJoins, DenseBases, PositioningAir, PositioningAirSources,
};
use content_reader_naval_combat_defines::naval_combat_defines::Defines;
use replay_data::{
    rows::AirBaseRow, rows::NavalCombatDetailsRow, rows::NavalCombatShipRow,
    rows::NavalCombatSideDetails, rows::NavalCombatSummaryInputs, rows::ShipModelRow,
    rows::ShipRow,
};

const ONE: i64 = 100_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Air {
    pub weighted_planes: i64,
    pub average_base_fill: i64,
}

pub(super) struct Inputs<'a> {
    pub side: &'a NavalCombatSideDetails,
    pub opponent: &'a NavalCombatSideDetails,
    pub defender: bool,
    pub duration: i32,
    pub surface_detection: [i64; 2],
    pub active_ship_slots: [i32; 2],
    pub air: [Air; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Terms {
    pub value: i64,
    pub base: i64,
    pub escort: i64,
    pub detection: i64,
    pub ship_ratio: i64,
    pub carrier_ratio: i64,
    pub recent_join: i64,
    pub early_defender: bool,
}

fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

fn modifier(summary: &NavalCombatSummaryInputs, scope: usize, key: usize) -> Option<i64> {
    let cell = &summary.modifiers[scope][key];
    if cell.present == 0 {
        return Some(0);
    }
    let [flags, selector] = summary.modifier_metadata[key];
    if selector != 0 {
        match summary.eligibility_callback {
            0 => return Some(0),
            1 if selector & summary.feature_flags == 0 => return Some(0),
            1 => (),
            _ => return None,
        }
    }
    Some(if flags & 0x10 != 0 {
        cell.raw.clamp(0, ONE)
    } else {
        cell.raw
    })
}

fn summary_modifiers(
    summary: &NavalCombatSummaryInputs,
    leader_scope: usize,
    key: usize,
) -> Option<i64> {
    [0, 1, 2, leader_scope]
        .into_iter()
        .try_fold(0_i64, |sum, scope| {
            Some(sum.wrapping_add(modifier(summary, scope, key)?))
        })
}

fn ratio_numerator_valid(value: i64) -> bool {
    (value.wrapping_add(i64::MAX - 1) as u64) < u64::MAX - 2
}

fn detection_bonus(own: i64, opposite: i64, defines: &Defines) -> i64 {
    let difference = own.wrapping_sub(opposite);
    let ratio = if ratio_numerator_valid(difference) {
        difference.wrapping_mul(ONE) / 10_000_000
    } else {
        i64::from(u32::MAX)
    };
    mul(ratio, defines.summary_fixed[1])
        .min(defines.summary_fixed[2])
        .max(0)
}

fn ship_ratio_penalty(input: &Inputs<'_>, defines: &Defines) -> Option<i64> {
    let [own, opposite] = input.active_ship_slots;
    if opposite <= 0 || own < defines.summary_integer[0] {
        return Some(0);
    }
    let denominator = i64::from(opposite) * ONE;
    let ratio = if denominator == 0 || i64::from(own) * ONE < -(i64::MAX - 1) {
        i64::from(u32::MAX)
    } else {
        i64::from(own).wrapping_mul(10_000_000_000) / denominator
    };
    let mut penalty = mul(ratio.wrapping_sub(ONE), defines.summary_fixed[3]);
    if input.opponent.commander != 0 {
        let admiral = modifier(&input.side.summary, 4, 4)?;
        if admiral != 0 {
            penalty = mul(ONE.wrapping_add(admiral), penalty);
        }
    }
    Some(penalty.min(defines.summary_fixed[4]).max(0))
}

fn carrier_ratio_penalty(input: &Inputs<'_>, defines: &Defines) -> i64 {
    let [own, opposite] = input.air;
    if opposite.weighted_planes <= 0 {
        return 0;
    }
    let ratio = if ratio_numerator_valid(own.weighted_planes) {
        own.weighted_planes.wrapping_mul(ONE) / opposite.weighted_planes
    } else {
        i64::from(u32::MAX)
    };
    mul(
        mul(ONE.wrapping_sub(ratio), defines.summary_fixed[5]),
        opposite.average_base_fill,
    )
    .min(defines.summary_fixed[6])
    .max(0)
}

pub(super) fn displayed_terms(input: &Inputs<'_>, defines: &Defines) -> Option<Terms> {
    let mut value = defines.summary_fixed[0];
    let summary = &input.side.summary;
    let early_defender = !summary.countries.is_empty()
        && input.defender
        && input.duration < defines.summary_integer[1];
    if !summary.countries.is_empty() {
        value = value.wrapping_add(summary_modifiers(summary, 3, 2)?);
        if early_defender {
            value = value.wrapping_add(summary_modifiers(summary, 4, 0)?);
        }
        value = value.wrapping_add(summary.escort_bonus);
    }
    let detection = detection_bonus(
        input.surface_detection[0],
        input.surface_detection[1],
        defines,
    );
    value = value.wrapping_add(detection);
    let ship_ratio = ship_ratio_penalty(input, defines)?;
    value = value.wrapping_sub(ship_ratio);
    let carrier_ratio = carrier_ratio_penalty(input, defines);
    value = value.wrapping_sub(carrier_ratio);
    if summary.recent_join_penalty > 0 {
        value = value.wrapping_sub(summary.recent_join_penalty);
    }
    Some(Terms {
        value: value.clamp(0, ONE),
        base: defines.summary_fixed[0],
        escort: if summary.countries.is_empty() {
            0
        } else {
            summary.escort_bonus
        },
        detection,
        ship_ratio,
        carrier_ratio,
        recent_join: summary.recent_join_penalty.max(0),
        early_defender,
    })
}

pub(super) fn active_ship_slots(rows: &[&NavalCombatShipRow], side: u8) -> i32 {
    rows.iter()
        .filter(|row| {
            row.side == side
                && row.member_source == 0
                && row.group_index != u32::MAX
                && row.state & 0x34 == 0
        })
        .fold(0_i32, |count, _| count.wrapping_add(1))
}

#[expect(
    clippy::too_many_arguments,
    reason = "positioning joins independent simulation sources"
)]
pub(super) fn positioning_values(
    detail: &NavalCombatDetailsRow,
    ships: Option<&[NavalCombatShipRow]>,
    models: Option<Keyed<'_, ShipModelRow>>,
    bases: Option<&[AirBaseRow]>,
    live_ships: Option<&[ShipRow]>,
    dense: Option<DenseBases<'_>>,
    content: &naval_air_groups::Content,
    defines: Option<&Defines>,
) -> [Option<i64>; 2] {
    positioning_terms_values(
        detail, ships, models, bases, live_ships, dense, content, defines,
    )
    .map(|terms| terms.map(|terms| terms.value))
}

#[expect(
    clippy::too_many_arguments,
    reason = "same recorded operands as positioning_values"
)]
pub(super) fn positioning_terms_values(
    detail: &NavalCombatDetailsRow,
    ships: Option<&[NavalCombatShipRow]>,
    models: Option<Keyed<'_, ShipModelRow>>,
    bases: Option<&[AirBaseRow]>,
    live_ships: Option<&[ShipRow]>,
    dense: Option<DenseBases<'_>>,
    content: &naval_air_groups::Content,
    defines: Option<&Defines>,
) -> [Option<Terms>; 2] {
    let (Some(ships), Some(defines)) = (ships, defines) else {
        return [None; 2];
    };
    let members: Vec<_> = ships
        .iter()
        .filter(|ship| ship.combat == detail.combat)
        .collect();
    let slots = [0, 1].map(|side| active_ship_slots(&members, side));
    let dense_counts = dense.and_then(|dense| {
        dense.counts(
            detail
                .sides
                .iter()
                .flat_map(|side| side.planes.iter().map(|plane| plane.air_base))
                .chain(members.iter().map(|member| member.weapon_context.air_base)),
        )
    });
    let dense_counts = dense_counts.as_ref();
    let detection = [0, 1].map(|side| {
        super::naval_surface_detection::sum_active_surface_detection(
            &members,
            models,
            dense_counts,
            side,
        )
    });
    let joins = CarrierJoins::new(Some(&members), bases, live_ships, models);
    let air = [0, 1].map(|side| {
        naval_air_groups::positioning_air(
            &detail.sides[side as usize],
            &PositioningAirSources {
                side_index: side,
                duration: detail.duration,
                joins: &joins,
                base_aircraft: dense_counts,
                content,
                defines: Some(defines),
            },
        )
    });
    std::array::from_fn(|side| {
        let opposite = 1 - side;
        let opposing_air = air[opposite]?;
        let own_air = if opposing_air.weighted_planes > 0 {
            air[side]?
        } else {
            PositioningAir::default()
        };
        displayed_terms(
            &Inputs {
                side: &detail.sides[side],
                opponent: &detail.sides[opposite],
                defender: side == 1,
                duration: detail.duration,
                surface_detection: [detection[side]?, detection[opposite]?],
                active_ship_slots: [slots[side], slots[opposite]],
                air: [
                    Air {
                        weighted_planes: own_air.weighted_planes,
                        average_base_fill: own_air.average_base_fill,
                    },
                    Air {
                        weighted_planes: opposing_air.weighted_planes,
                        average_base_fill: opposing_air.average_base_fill,
                    },
                ],
            },
            defines,
        )
    })
}

#[cfg(test)]
mod tests;
