//! A state's strategic resource amount from its recorded operands,
//! compared as a whole 32-bit integer in a state scope. The shared trigger
//! evaluator ([`crate::triggers`], used by the viewer's raids and by scripted
//! text) and the viewer's tactic triggers both evaluate it here, from one
//! recorded hour.

use std::{collections::BTreeMap, io, sync::OnceLock};

use installed_content_core::{buildings::Buildings, defines::Defines, parser::Operator};
use replay_data::{
    rows::BuildingRow, rows::CountryBuildingResourceRow, rows::ResourceDefinitionRow,
    rows::StateBaseResourceRow, rows::StateResourceInputRow, rows::StateResourceRow,
    rows::TableSchema, rows::mask, rows::tables,
};
use replay_frame::frame::{HourFrame, Rowset};

/// The fields of each table the resource operands read.
const ACTIVE: u64 = mask::<ResourceDefinitionRow>(&["token", "active"]);
const INPUT: u64 = mask::<StateResourceInputRow>(&[
    "gate",
    "global_factor",
    "origin",
    "infrastructure_level",
    "infrastructure_modifier",
    "supply_route",
]);
const STORED: u64 = mask::<StateResourceRow>(&["pool", "additive", "factor", "cost"]);
const BASE: u64 = mask::<StateBaseResourceRow>(&["amount"]);
const HEALTHY: u64 = mask::<BuildingRow>(&["healthy_level"]);
const OVERRIDE: u64 =
    mask::<CountryBuildingResourceRow>(&["building", "count", "resource", "amount"]);

type Index<T> = OnceLock<crate::triggers::refusal::Resolved<Result<T, crate::triggers::Failure>>>;
/// Building type token to its `local_resources` grants by resource token.
type Grants = BTreeMap<u32, Result<Vec<(u32, i64)>, crate::triggers::Failure>>;

/// One in fixed5.
pub const ONE: i64 = 100_000;

/// The operands combined for one state and resource, all fixed5.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceOperands {
    /// `StateBaseResources`: the template amount.
    pub template: i64,
    /// The retained resource pool.
    pub pool: i64,
    /// Whether `additive` counts.
    pub gate: bool,
    pub additive: i64,
    pub factor: i64,
    pub global_factor: i64,
    /// The controller's origin for the state; `None` makes both factors one.
    pub origin: Option<OriginFactors>,
    pub cost: i64,
    /// The healthy-level building grants.
    pub grants: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OriginFactors {
    /// The infrastructure level and the controller's infrastructure modifier.
    pub infrastructure: Option<(i32, i64)>,
    pub supply_route: bool,
    /// `INFRASTRUCTURE_RESOURCE_BONUS` and `SUPPLY_ROUTE_RESOURCE_BONUS`.
    pub bonuses: [i64; 2],
}

/// The unscaled base amount: `(template + pool + additive under the gate) x factor x global
/// factor`, each product truncated, each factor one plus its modifier and never below zero.
/// It is also the full amount a resource origin holds for the resource.
pub fn base_amount(operands: &ResourceOperands) -> i64 {
    let additive = if operands.gate { operands.additive } else { 0 };
    let factor = operands.factor.wrapping_add(ONE).max(0);
    let global = operands.global_factor.wrapping_add(ONE).max(0);
    let base = operands
        .pool
        .wrapping_add(additive)
        .wrapping_add(operands.template)
        .wrapping_mul(factor)
        / ONE;
    base.wrapping_mul(global) / ONE
}

/// The origin's infrastructure factor: one, plus the level times
/// `INFRASTRUCTURE_RESOURCE_BONUS`, plus the controller's modifier (scaled by the level when
/// positive); one without an infrastructure building.
pub fn infrastructure_factor(infrastructure: Option<(i32, i64)>, bonus: i64) -> i64 {
    match infrastructure {
        None => ONE,
        Some((level, modifier)) => {
            let level = i64::from(level).wrapping_mul(ONE);
            let modifier = if modifier > 0 {
                level.wrapping_mul(modifier) / ONE
            } else {
                modifier
            };
            modifier
                .wrapping_add(ONE)
                .wrapping_add(level.wrapping_mul(bonus) / ONE)
        }
    }
}

/// One resource in the game's order: base amount, then the origin's
/// infrastructure and supply-route factors, clamped at zero, minus the cost,
/// plus the grants. Uses signed i64 with truncating division and wrapping products.
pub fn state_resource_amount(operands: &ResourceOperands) -> i64 {
    let base = base_amount(operands);
    let (infrastructure, supply) = match operands.origin {
        None => (ONE, ONE),
        Some(origin) => (
            infrastructure_factor(origin.infrastructure, origin.bonuses[0]),
            if origin.supply_route {
                origin.bonuses[1].wrapping_add(ONE)
            } else {
                ONE
            },
        ),
    };
    let scaled = (infrastructure.wrapping_mul(base) / ONE).wrapping_mul(supply) / ONE;
    scaled
        .max(0)
        .wrapping_sub(operands.cost)
        .wrapping_add(operands.grants)
}

/// One building's resource grant: `bonus` is the controller's override
/// amount when the building has an override, which keeps only a positive sum.
pub fn building_grant(healthy: i16, amount: i64, bonus: Option<i64>) -> i64 {
    if healthy <= 0 {
        return 0;
    }
    let healthy = i64::from(healthy);
    match bonus {
        None => healthy.wrapping_mul(amount),
        Some(bonus) => {
            let sum = bonus.wrapping_add(amount);
            if sum > 0 {
                healthy.wrapping_mul(sum).wrapping_mul(ONE) / ONE
            } else {
                0
            }
        }
    }
}

/// The resource amount truncated to whole units and to 32 bits, then compared.
pub fn resource_comparison(amount: i64, comparison: Operator, wanted: i32) -> bool {
    int_comparison((amount / ONE) as i32, comparison, wanted)
}

/// `>` is `N < v`, `<` is `v < N`, `=` is equality, over the
/// one-character operators the lexer reads.
pub fn int_comparison(value: i32, comparison: Operator, wanted: i32) -> bool {
    match comparison {
        Operator::Greater => wanted < value,
        Operator::Less => value < wanted,
        Operator::Equals => value == wanted,
    }
}

/// `NBuildings` `INFRASTRUCTURE_RESOURCE_BONUS` and
/// `SUPPLY_ROUTE_RESOURCE_BONUS`, fixed5, as [`OriginFactors::bonuses`].
pub fn resource_bonuses(defines: &Defines) -> io::Result<[i64; 2]> {
    Ok([
        defines.fixed5("NBuildings", "INFRASTRUCTURE_RESOURCE_BONUS")?,
        defines.fixed5("NBuildings", "SUPPLY_ROUTE_RESOURCE_BONUS")?,
    ])
}

/// The hour's joins a state's amount reads through.
pub trait Links {
    /// A raw country reference's canonical slot (`CountryLinks`), `None`
    /// when the hour has none.
    fn slot(&self, raw: u32) -> Option<u32>;
    /// The lexer token of a name, as the game finds a token for a key.
    fn token_of(&self, name: &[u8]) -> Result<Option<u32>, String>;
    /// Typed input errors for trigger consumers; content-only implementations
    /// retain their ordinary unsupported diagnostics.
    fn token_input(&self, name: &[u8]) -> Result<Option<u32>, crate::triggers::Failure> {
        self.token_of(name).map_err(Into::into)
    }
}

/// One hour's recorded state resource operands, each index built on first
/// use.
pub struct StateResources<'a> {
    frame: HourFrame<'a>,
    /// Building types, for their `local_resources_<resource>` grants.
    buildings: &'a Buildings,
    /// `NBuildings` `INFRASTRUCTURE_RESOURCE_BONUS` and
    /// `SUPPLY_ROUTE_RESOURCE_BONUS`, fixed5.
    resource_bonuses: [i64; 2],
    /// Resource token to its `active` byte.
    resources: Index<BTreeMap<u32, u8>>,
    /// Each state's buildings: type token and healthy level.
    state_buildings: Index<BTreeMap<u32, Vec<(u32, i16)>>>,
    /// Each canonical country's overrides in native order: building, count, resource and
    /// amount.
    overrides: Index<BTreeMap<u32, Vec<(u32, u32, u32, u32, i64)>>>,
    grants: Index<Grants>,
}

fn table<T: TableSchema>(
    frame: HourFrame<'_>,
    mask: u64,
) -> Result<Rowset<'_, T::Row>, crate::triggers::Failure> {
    frame
        .complete::<T>(mask)
        .map_err(|error| crate::triggers::Failure::input(T::ID, "", None, error.to_string()))
}

/// `rows`' failure to build a row of `T`, refused where it is reached.
fn walked<T: TableSchema>(why: io::Error) -> crate::triggers::Failure {
    crate::triggers::refusal::input_error(T::ID, why)
}

fn get<T>(
    cell: &Index<T>,
    build: impl FnOnce() -> Result<T, crate::triggers::Failure>,
) -> Result<&T, crate::triggers::Failure> {
    cell.get_or_init(|| {
        crate::triggers::refusal::Resolved::new(|| crate::triggers::refusal::deferred(build))
    })
    .value()
    .as_ref()
    .map_err(|failure| {
        failure.reached();
        failure.clone()
    })
}

impl<'a> StateResources<'a> {
    pub fn new(
        frame: impl Into<HourFrame<'a>>,
        buildings: &'a Buildings,
        resource_bonuses: [i64; 2],
    ) -> Self {
        Self {
            frame: frame.into(),
            buildings,
            resource_bonuses,
            resources: OnceLock::new(),
            state_buildings: OnceLock::new(),
            overrides: OnceLock::new(),
            grants: OnceLock::new(),
        }
    }

    /// The recorded resource database: each resource token's `active` byte.
    pub fn resource_database(&self) -> Result<&BTreeMap<u32, u8>, crate::triggers::Failure> {
        get(&self.resources, || {
            table::<tables::ResourceDefinitions>(self.frame, ACTIVE)?
                .iter()
                .map(|row| {
                    row.map(|row| (row.token, row.active))
                        .map_err(walked::<tables::ResourceDefinitions>)
                })
                .collect()
        })
    }

    /// `local_resources` grants by building token; the game drops a name
    /// that is not an active resource.
    fn grants(&self, links: &impl Links) -> Result<&Grants, crate::triggers::Failure> {
        get(&self.grants, || {
            let resources = self.resource_database()?;
            let mut out = Grants::new();
            for (name, building) in &self.buildings.types {
                let Some(token) = links.token_input(name.as_bytes())? else {
                    continue;
                };
                let grants = if let Some(reason) = &building.raid_refused {
                    Err(format!("building {name}'s resource grants are refused: {reason}").into())
                } else if !building.local_resources.is_empty()
                    && self.buildings.ambiguous_slots.contains(name)
                {
                    Err(
                        format!("building {name} is declared more than once with resource grants")
                            .into(),
                    )
                } else {
                    let mut grants = Vec::new();
                    for (resource, amount) in &building.local_resources {
                        if let Some(token) = links.token_input(resource.as_bytes())?
                            && resources.get(&token).is_some_and(|&active| active != 0)
                        {
                            grants.push((token, *amount));
                        }
                    }
                    Ok(grants)
                };
                out.insert(token, grants);
            }
            Ok(out)
        })
    }

    /// The fixed5 amount for the state's resource token; `controller` is
    /// the state's controller's canonical slot, whose building overrides apply.
    pub fn amount(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
        links: &impl Links,
    ) -> Result<i64, crate::triggers::Failure> {
        Ok(state_resource_amount(
            &self.operands(state, resource, controller, links)?,
        ))
    }

    /// The operands [`Self::amount`] combines for the state's resource token: what the
    /// resources map mode's origins read too (the full amount, [`base_amount`], and the grants).
    pub fn operands(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
        links: &impl Links,
    ) -> Result<ResourceOperands, crate::triggers::Failure> {
        // Each table is keyed by the state, and the resource after it.
        let key = (u128::from(state) << 32) | u128::from(resource);
        let input = table::<tables::StateResourceInputs>(self.frame, INPUT)?
            .get(u128::from(state))
            .map_err(walked::<tables::StateResourceInputs>)?
            .ok_or_else(|| {
            crate::triggers::Failure::input(
                replay_data::rows::TableId::StateResourceInputs,
                "",
                Some(crate::triggers::Store::State(state)),
                format!("state {state} has no recorded resource inputs"),
            )
        })?;
        // The operands are the game's at the frame's hour (the raid state
        // family reads every state and country a hooked writer touched),
        // except the supply-route flag, the value at the state's last read,
        // up to a day old.
        let stored = table::<tables::StateResources>(self.frame, STORED)?
            .get(key)
            .map_err(walked::<tables::StateResources>)?;
        let base = table::<tables::StateBaseResources>(self.frame, BASE)?
            .get(key)
            .map_err(walked::<tables::StateBaseResources>)?;
        let operand =
            |field: fn(&StateResourceRow) -> i64| stored.as_deref().map_or(0, |row| field(row));
        Ok(ResourceOperands {
            template: base.map_or(0, |row| row.amount),
            pool: operand(|row| row.pool),
            gate: input.gate != 0,
            additive: operand(|row| row.additive),
            factor: operand(|row| row.factor),
            global_factor: input.global_factor,
            origin: (input.origin != 0).then_some(OriginFactors {
                infrastructure: input
                    .infrastructure_level
                    .map(|level| (level, input.infrastructure_modifier)),
                supply_route: input.supply_route != 0,
                bonuses: self.resource_bonuses,
            }),
            cost: operand(|row| row.cost),
            grants: self.building_grants(state, resource, controller, links)?,
        })
    }

    /// The state's buildings' grants of one resource, with the controller's overrides.
    fn building_grants(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
        links: &impl Links,
    ) -> Result<i64, crate::triggers::Failure> {
        let buildings = get(&self.state_buildings, || {
            let mut out: BTreeMap<u32, Vec<(u32, i16)>> = BTreeMap::new();
            for row in table::<tables::StateBuildings>(self.frame, HEALTHY)?.iter() {
                let row = row.map_err(walked::<tables::StateBuildings>)?;
                out.entry(row.location)
                    .or_default()
                    .push((row.building_type, row.healthy_level));
            }
            Ok(out)
        })?;
        let overrides = get(&self.overrides, || {
            let mut out: BTreeMap<u32, Vec<(u32, u32, u32, u32, i64)>> = BTreeMap::new();
            for row in table::<tables::CountryBuildingResources>(self.frame, OVERRIDE)?.iter() {
                let row = row.map_err(walked::<tables::CountryBuildingResources>)?;
                if let Some(slot) = links.slot(row.country) {
                    out.entry(slot).or_default().push((
                        row.ordinal,
                        row.building,
                        row.count,
                        row.resource,
                        row.amount,
                    ));
                }
            }
            for rows in out.values_mut() {
                rows.sort_by_key(|row| row.0);
            }
            Ok(out)
        })?;
        let overrides = controller
            .and_then(|slot| overrides.get(&slot))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let grants = self.grants(links)?;
        let mut total = 0_i64;
        for &(building, healthy) in buildings.get(&state).map(Vec::as_slice).unwrap_or(&[]) {
            let list = grants
                .get(&building)
                .ok_or_else(|| format!("state {state} has a building type the content lacks"))?
                .as_ref()
                .map_err(|failure| {
                    failure.reached();
                    failure.clone()
                })?;
            let Some(&(_, amount)) = list.iter().find(|(token, _)| *token == resource) else {
                continue;
            };
            // Override presence reads the first entry; value lookup reads the
            // first matching element.
            // The amounts are the country's at the frame's hour.
            let bonus = overrides
                .iter()
                .find(|row| row.1 == building)
                .filter(|row| row.2 != 0)
                .map(|_| {
                    overrides
                        .iter()
                        .find(|row| row.1 == building && row.2 != 0 && row.3 == resource)
                        .map_or(0, |row| row.4)
                });
            total = total.wrapping_add(building_grant(healthy, amount, bonus));
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_resource_amounts_preserve_order_and_truncation() {
        use state_resource_amount as amount;
        let plain = ResourceOperands {
            template: 2_000_000,
            pool: 300_000,
            additive: 100_000,
            ..Default::default()
        };
        // The additive key counts only under the gate.
        assert_eq!(amount(&plain), 2_300_000);
        assert_eq!(
            amount(&ResourceOperands {
                gate: true,
                ..plain
            }),
            2_400_000
        );
        // Factor then global factor, each truncated; a factor below -1 is zero.
        let factored = ResourceOperands {
            factor: -50_000,
            global_factor: 33_333,
            ..plain
        };
        assert_eq!(amount(&factored), 1_150_000 * 133_333 / 100_000);
        assert_eq!(
            amount(&ResourceOperands {
                factor: -200_000,
                cost: 50_000,
                grants: 7,
                ..plain
            }),
            -49_993
        );
        // Origin factors: 1 + 3 * 0.15, a positive modifier scaled by the
        // level, a negative one added as is; the supply route adds 0.2.
        let origin = |modifier, supply_route| OriginFactors {
            infrastructure: Some((3, modifier)),
            supply_route,
            bonuses: [15_000, 20_000],
        };
        let with = |origin| {
            amount(&ResourceOperands {
                origin: Some(origin),
                ..plain
            })
        };
        assert_eq!(with(origin(0, false)), 2_300_000 * 145_000 / 100_000);
        assert_eq!(with(origin(10_000, false)), 2_300_000 * 175_000 / 100_000);
        assert_eq!(
            with(origin(-20_000, true)),
            2_300_000 * 125_000 / 100_000 * 120_000 / 100_000
        );
        assert_eq!(
            with(OriginFactors {
                infrastructure: None,
                ..origin(-20_000, false)
            }),
            2_300_000
        );
        // A negative scaled amount clamps to zero before cost and grants.
        assert_eq!(
            amount(&ResourceOperands {
                template: -5_000_000,
                cost: 1,
                grants: 10,
                ..plain
            }),
            9
        );
    }

    #[test]
    fn resource_comparisons_truncate_to_whole_32_bit_units() {
        use Operator::{Equals, Greater, Less};
        use resource_comparison as compare;
        assert!(!compare(2_499_999, Greater, 24));
        assert!(compare(2_500_000, Greater, 24));
        assert!(compare(-150_000, Equals, -1));
        assert!(compare(-150_000, Less, 0));
        assert!(compare(((1_i64 << 32) + 5) * 100_000, Equals, 5));
    }

    #[test]
    fn building_grants_use_healthy_levels_and_keep_positive_override_sums() {
        use building_grant as grant;
        assert_eq!(grant(0, 100_000, None), 0);
        assert_eq!(grant(-1, 100_000, Some(1)), 0);
        assert_eq!(grant(2, -100_000, None), -200_000);
        assert_eq!(grant(2, 100_000, Some(-100_000)), 0);
        assert_eq!(grant(2, 100_000, Some(50_000)), 300_000);
        assert_eq!(
            grant(i16::MAX, i64::MAX, None),
            i64::MAX.wrapping_mul(32767)
        );
    }
}
