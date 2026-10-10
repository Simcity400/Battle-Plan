//! Carrier and land-based air in the naval positioning law.
//!
//! the game walks each side's forward groups' active air: every admitted slot's current
//! planes weighted by sortie efficiency, and the average
//! fill of the air bases it flies from (`air-base aircraft counting`). The authored screening
//! sortie bonuses and the naval defines come from matching content.
use super::keyed::Keyed;
use engine::modifier_keys as keys;
use installed_content::{
    ResolvedLayers,
    naval_combat_defines::Defines,
    static_modifiers::{self, Request, Resolved},
};
use replay::rows::{
    AirBaseRow, AirWingRow, NavalCombatAirBaseWingHandlesRow, NavalCombatAirEfficiency,
    NavalCombatPlane, NavalCombatShipRow, NavalCombatSideDetails, ShipModelRow, ShipRow,
};
use std::collections::{BTreeMap, btree_map::Entry};

const ONE: i64 = 100_000;

/// Named static modifiers weighted by capital and combined screening.
const SCREENING_MODIFIERS: [&str; 2] = ["screening_bonus", "capital_screening_bonus"];
/// The sortie law's modifier keys 0x24 and 0x1B6.
const SORTIE: [u32; 2] = [keys::SORTIE_EFFICIENCY, keys::FIGHTER_SORTIE_EFFICIENCY];
/// Their script names, the keys the screening static modifiers author.
const SORTIE_KEYS: [&str; 2] = ["sortie_efficiency", "fighter_sortie_efficiency"];

/// The authored screening static modifiers' sortie terms, `[modifier][lane]`
/// in raw fixed5; `None` where the content's value is refused.
#[derive(Default)]
pub(super) struct Content {
    screening: [[Option<i64>; 2]; 2],
    /// Equipment registration order, including generated types. Air entries
    /// group by the first variant's model ID.
    equipment: Option<installed_content::equipment::Database>,
}

impl Content {
    /// Reads the content stack's effective definitions. Each refused value is
    /// named in `warnings`, with their count, as is each place the game's
    /// reading leaves the text; a failed read leaves all four unavailable,
    /// also with a warning.
    pub(super) fn load(
        layers: &ResolvedLayers,
        warnings: &mut Vec<String>,
        equipment: installed_content::equipment::Database,
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

    pub(super) fn equipment(&self) -> Option<&installed_content::equipment::Database> {
        self.equipment.as_ref()
    }

    /// `values` in [`SCREENING_MODIFIERS`] x [`SORTIE_KEYS`] order.
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
        // Metadata rows: 0x22 (weather, modifiers 1 and 2), 0x23 (night, 0).
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
    // the game multiplies crowding by ONE before any other factor. Preserve
    // this operation too: the native signed products wrap before division.
    let base = mul(night, mul(weather, mul(base, mul(crowding, ONE)))).max(0);
    let maximum = host.maximum_organisation;
    let organisation = host.organisation;
    let valid = (organisation as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd;
    let ratio = if maximum != 0 && valid {
        organisation.wrapping_mul(ONE) / maximum
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

/// Final efficiency used by the red-text test.
/// A missing carrier, member or modifier operand leaves the colour unknown.
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

/// Report sortie factors before multiplying by host organisation.
/// Keep both values so the two hover sections use their respective gates.
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

/// Describe the carrier member's sortie modifier keys with text enabled.
/// Append SCREENING_VALUE after the source tree when the box 2/3 screening
/// addition is nonzero.
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
    /// The host's positive equipment pool flags.
    pool_flags: u64,
}

/// One combat's forward-array members by side, group and ship. `None` marks
/// a key two members share, which no reference can resolve uniquely.
type Members<'a> = BTreeMap<(u8, u32, u64), Option<&'a NavalCombatShipRow>>;

/// Index carrier-air references once per window binding and share them across
/// both sides. Resolve each slot's air base, host ship and optional combat member
/// with fixed lookups rather than table scans. Missing tables or rows and
/// ambiguous keys leave only the dependent value unknown.
pub(super) struct CarrierJoins<'a> {
    members: Option<Members<'a>>,
    bases: Option<Keyed<'a, AirBaseRow>>,
    live_ships: Option<Keyed<'a, ShipRow>>,
    models: Option<Keyed<'a, ShipModelRow>>,
}

impl<'a> CarrierJoins<'a> {
    /// `members` are one combat's complete census rows; each table is `None`
    /// when this hour does not hold it complete. `models` is the binding's
    /// shared ShipModels lookup.
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

    /// Resolve the carrier through the plane's air base independently of its
    /// optional combat member, then read the host's current and maximum
    /// organisation.
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

    /// The linked plane combat member: the one forward-array member of the
    /// side and group whose ship is the host.
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

/// The carrier-air pass yields weighted plane totals and average base fill,
/// both in fixed5. Positioning uses this side's first output and its opponent's second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PositioningAir {
    pub(super) weighted_planes: i64,
    pub(super) average_base_fill: i64,
}

/// Complete shared simulation sources for one combat side. Callers construct
/// `base_aircraft` only from complete dense-base and wing tables. The ship
/// census must be complete for source-3 carrier efficiency.
pub(super) struct PositioningAirSources<'a> {
    pub(super) side_index: u8,
    pub(super) duration: i32,
    pub(super) joins: &'a CarrierJoins<'a>,
    /// Counts derived from a complete dense air-base container walk. A
    /// missing key remains unknown, including when an indexed registration
    /// table happens to have no row for this base.
    pub(super) base_aircraft: Option<&'a BTreeMap<u64, i32>>,
    pub(super) content: &'a Content,
    pub(super) defines: Option<&'a Defines>,
}

/// the game walks only the side's forward groups and their active air
/// arrays. `source` is the array kind; `group_index == u32::MAX` identifies a
/// paired-only group after the engine normalizes its raw topology. Air groups
/// outside that pass, and a plane with its exclusion flag set, do not contribute.
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
            // Only the linked plane combat member contributes modifier keys.
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
        // Native signed products wrap before division, even though the final
        // factor of ONE cancels algebraically for small ordinary values.
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
        // The native fixed-point divide returns this sentinel if its signed
        // input guard fails; the same guard appears in the sortie law.
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

/// Air-base aircraft counting traverses dense containers, preserving repeated
/// wing occurrences. The indexed
/// registration vector cannot substitute: a save can load duplicate country
/// slots and leave an extra dense container after the indexed slot is replaced.
/// Every relevant base therefore needs a row captured from the dense vector,
/// including an explicit empty row.
#[derive(Clone, Copy)]
pub(super) struct DenseBases<'a> {
    rows: Keyed<'a, NavalCombatAirBaseWingHandlesRow>,
    wings: Keyed<'a, AirWingRow>,
}

impl<'a> DenseBases<'a> {
    /// Complete same-hour dense rows and AirWings roster; `None` when either
    /// holds a repeated key.
    pub(super) fn new(
        rows: &'a [NavalCombatAirBaseWingHandlesRow],
        wings: &'a [AirWingRow],
    ) -> Option<Self> {
        Some(Self {
            rows: Keyed::new(rows, |row| row.base)?,
            wings: Keyed::new(wings, |wing| wing.id)?,
        })
    }

    /// The aircraft counts of the bases one combat references, each row
    /// summed once and no other combat's row read. A referenced base without
    /// a dense row stays absent, which its readers treat as unknown; a wing
    /// outside the AirWings roster, or a row for base zero, leaves every count
    /// unknown.
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

/// `modifier value lookup` for a present raw value of the
/// efficiency row's metadata key `row` ([`naval_air_efficiency_inputs`]
/// order: 0x22, 0x23, 0x24, 0x1B6): a feature-gated key reads zero unless the
/// authenticated flags getter admits it, and flag 0x10 clamps to [0, 1].
///
/// [`naval_air_efficiency_inputs`]: engine::naval_air_efficiency_inputs::METADATA_KEYS
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

/// `combat-box selection`. States 4 and 16 are box 7,
/// 32 is box 6 and a state outside 1, 2 and 8 asserts and returns 0. For
/// the others a resolved convoy handle is box 3; an unresolved ship is 0.
/// Test formation bits 22 and 37 on the ship and its positive equipment pool:
/// with it and without formation bit 7 the ship is box 3; otherwise bit 7
/// gives 4 or 5 by detection, bit 8 gives 1 and bit 39 gives 3,
/// else 2.
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

/// The carrier member's static screening term: in box 2 or 3, stored capital
/// screening times the positive `screening_bonus` value; in box 3, also
/// combined screening times the positive `capital_screening_bonus` value.
/// Each value uses its key's global definition metadata, recorded separately
/// from the accumulator.
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
