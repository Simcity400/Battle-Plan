//! A state's strategic resource amount, as `CResourceCountTrigger::GetValue`
//! (141390640) reads it in a state scope: `CState::AccumulateStrategicResourceAmounts`
//! (1409CC730) over the state's recorded operands, compared by
//! `CIntTrigger::Evaluate` (14138CC70). The raid script (`map::raids::script`)
//! and the tactic triggers (`combat::tactics`) both evaluate it here, from
//! one recorded hour.

use std::{collections::BTreeMap, io, sync::OnceLock};

use installed_content::{buildings::Buildings, defines::Defines, triggers::Comparison};
use replay::{
    frame::DecodedFrame,
    rows::{
        CountryBuildingResourceRow, StateResourceInputRow, StateResourceRow, TableSchema, tables,
    },
};

type Index<T> = OnceLock<Result<T, String>>;
/// Building type token to its `local_resources` grants by resource token.
type Grants = BTreeMap<u32, Result<Vec<(u32, i64)>, String>>;

/// One in fixed5.
pub const ONE: i64 = 100_000;

/// The operands `AccumulateStrategicResourceAmounts@1409CC730` combines for
/// one state and resource, all fixed5.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceOperands {
    /// `StateBaseResources`: the template amount.
    pub template: i64,
    /// `CState + 0x1c0`.
    pub pool: i64,
    /// `CState + 0x1b4`: whether `additive` counts.
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
    /// The infrastructure `+0x40` level and the controller's modifier 0x292.
    pub infrastructure: Option<(i32, i64)>,
    pub supply_route: bool,
    /// `INFRASTRUCTURE_RESOURCE_BONUS` and `SUPPLY_ROUTE_RESOURCE_BONUS`.
    pub bonuses: [i64; 2],
}

/// `AccumulateStrategicResourceAmounts@1409CC730` for one resource, in the
/// game's order: `CalculateBaseResourceAmount@1409C5670`, then the origin's
/// `InfrastructureResourceFactor@140C9DCB0` and `SupplyRouteResourceFactor@140CA5170`
/// clamped at zero (`1409C6E20`), minus the cost, plus the grants. Signed i64
/// with truncating division and wrapping products, as the native code.
pub fn state_resource_amount(operands: &ResourceOperands) -> i64 {
    let additive = if operands.gate { operands.additive } else { 0 };
    let factor = operands.factor.wrapping_add(ONE).max(0);
    let global = operands.global_factor.wrapping_add(ONE).max(0);
    let base = operands
        .pool
        .wrapping_add(additive)
        .wrapping_add(operands.template)
        .wrapping_mul(factor)
        / ONE;
    let base = base.wrapping_mul(global) / ONE;
    let (infrastructure, supply) = match operands.origin {
        None => (ONE, ONE),
        Some(origin) => (
            match origin.infrastructure {
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
                        .wrapping_add(level.wrapping_mul(origin.bonuses[0]) / ONE)
                }
            },
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

/// `AccumulateBuildingResourceGrants@141163590` for one building and
/// resource: `bonus` is the controller's override amount when the building
/// has an override, which keeps only a positive sum.
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

/// `CIntTrigger::Evaluate@14138CC70` for `CResourceCountTrigger`: the
/// amount truncated to whole units and to 32 bits, then compared.
pub fn resource_comparison(amount: i64, comparison: Comparison, wanted: i32) -> Option<bool> {
    int_comparison((amount / ONE) as i32, comparison, wanted)
}

/// `CIntTrigger::Evaluate@14138CC70`: `>` is `N < v`, `<` is `v < N`, `=`
/// equality. The game reads any other comparison as equality; the parser's
/// other forms are refused rather than assumed to reach it unchanged.
pub fn int_comparison(value: i32, comparison: Comparison, wanted: i32) -> Option<bool> {
    match comparison {
        Comparison::Greater => Some(wanted < value),
        Comparison::Less => Some(value < wanted),
        Comparison::Equal => Some(value == wanted),
        _ => None,
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
}

/// One hour's recorded state resource operands, each index built on first
/// use.
pub struct StateResources<'a> {
    frame: &'a DecodedFrame,
    /// Building types, for their `local_resources_<resource>` grants.
    buildings: &'a Buildings,
    /// `NBuildings` `INFRASTRUCTURE_RESOURCE_BONUS` and
    /// `SUPPLY_ROUTE_RESOURCE_BONUS`, fixed5.
    resource_bonuses: [i64; 2],
    /// Resource token to its `active` byte.
    resources: Index<BTreeMap<u32, u8>>,
    resource_inputs: Index<BTreeMap<u32, &'a StateResourceInputRow>>,
    state_resources: Index<BTreeMap<(u32, u32), &'a StateResourceRow>>,
    base_resources: Index<BTreeMap<(u32, u32), i64>>,
    /// Each state's buildings: type token and healthy level.
    state_buildings: Index<BTreeMap<u32, Vec<(u32, i16)>>>,
    /// Each canonical country's overrides in native order.
    overrides: Index<BTreeMap<u32, Vec<&'a CountryBuildingResourceRow>>>,
    grants: Index<Grants>,
}

fn table<T: TableSchema>(frame: &DecodedFrame) -> Result<&[T::Row], String> {
    frame.complete::<T>().map_err(|error| error.to_string())
}

fn get<T>(cell: &Index<T>, build: impl FnOnce() -> Result<T, String>) -> Result<&T, String> {
    cell.get_or_init(build).as_ref().map_err(Clone::clone)
}

impl<'a> StateResources<'a> {
    pub fn new(
        frame: &'a DecodedFrame,
        buildings: &'a Buildings,
        resource_bonuses: [i64; 2],
    ) -> Self {
        Self {
            frame,
            buildings,
            resource_bonuses,
            resources: OnceLock::new(),
            resource_inputs: OnceLock::new(),
            state_resources: OnceLock::new(),
            base_resources: OnceLock::new(),
            state_buildings: OnceLock::new(),
            overrides: OnceLock::new(),
            grants: OnceLock::new(),
        }
    }

    /// The recorded resource database: each resource token's `active` byte.
    pub fn resource_database(&self) -> Result<&BTreeMap<u32, u8>, String> {
        get(&self.resources, || {
            Ok(table::<tables::ResourceDefinitions>(self.frame)?
                .iter()
                .map(|row| (row.token, row.active))
                .collect())
        })
    }

    /// `local_resources` grants by building token; the game drops a name
    /// that is not an active resource (`read_is_port@1414A9220`).
    fn grants(&self, links: &impl Links) -> Result<&Grants, String> {
        get(&self.grants, || {
            let resources = self.resource_database()?;
            let mut out = Grants::new();
            for (name, building) in &self.buildings.types {
                let Some(token) = links.token_of(name.as_bytes())? else {
                    continue;
                };
                let grants = if let Some(reason) = &building.raid_refused {
                    Err(format!(
                        "building {name}'s resource grants are refused: {reason}"
                    ))
                } else if !building.local_resources.is_empty()
                    && self.buildings.ambiguous_slots.contains(name)
                {
                    Err(format!(
                        "building {name} is declared more than once with resource grants"
                    ))
                } else {
                    let mut grants = Vec::new();
                    for (resource, amount) in &building.local_resources {
                        if let Some(token) = links.token_of(resource.as_bytes())?
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

    /// The amount `CResourceCountTrigger::GetValue@141390640` reads, fixed5,
    /// for the state's resource token; `controller` is the state's
    /// controller's canonical slot, whose building overrides apply.
    pub fn amount(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
        links: &impl Links,
    ) -> Result<i64, String> {
        let inputs = get(&self.resource_inputs, || {
            Ok(table::<tables::StateResourceInputs>(self.frame)?
                .iter()
                .map(|row| (row.state, row))
                .collect())
        })?;
        let input = inputs
            .get(&state)
            .ok_or_else(|| format!("state {state} has no recorded resource inputs"))?;
        // The operands are the state's and its controller's at their daily
        // reads (owner rule: no harsher than the game's rolling re-check of
        // raid offers), so an amount can lag up to a day.
        let resources = get(&self.state_resources, || {
            Ok(table::<tables::StateResources>(self.frame)?
                .iter()
                .map(|row| ((row.state, row.resource), row))
                .collect())
        })?;
        let base = get(&self.base_resources, || {
            Ok(table::<tables::StateBaseResources>(self.frame)?
                .iter()
                .map(|row| ((row.state, row.resource), row.amount))
                .collect())
        })?;
        let stored = resources.get(&(state, resource));
        let operand = |field: fn(&StateResourceRow) -> i64| stored.map_or(0, |row| field(row));
        Ok(state_resource_amount(&ResourceOperands {
            template: base.get(&(state, resource)).copied().unwrap_or(0),
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
        }))
    }

    /// The state's buildings' grants of one resource, with the controller's
    /// overrides (`CCountryResources + 0x7a0`).
    fn building_grants(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
        links: &impl Links,
    ) -> Result<i64, String> {
        let buildings = get(&self.state_buildings, || {
            let mut out: BTreeMap<u32, Vec<(u32, i16)>> = BTreeMap::new();
            for row in table::<tables::StateBuildings>(self.frame)? {
                out.entry(row.location)
                    .or_default()
                    .push((row.building_type, row.healthy_level));
            }
            Ok(out)
        })?;
        let overrides = get(&self.overrides, || {
            let mut out: BTreeMap<u32, Vec<&CountryBuildingResourceRow>> = BTreeMap::new();
            for row in table::<tables::CountryBuildingResources>(self.frame)? {
                if let Some(slot) = links.slot(row.country) {
                    out.entry(slot).or_default().push(row);
                }
            }
            for rows in out.values_mut() {
                rows.sort_by_key(|row| row.ordinal);
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
                .map_err(Clone::clone)?;
            let Some(&(_, amount)) = list.iter().find(|(token, _)| *token == resource) else {
                continue;
            };
            // `HasBuildingResourceOverride@140CA54A0` reads the first entry;
            // `GetBuildingResourceOverride@140CA2C70` the first matching element.
            // The amounts are the country's at its daily read.
            let bonus = overrides
                .iter()
                .find(|row| row.building == building)
                .filter(|row| row.count != 0)
                .map(|_| {
                    overrides
                        .iter()
                        .find(|row| {
                            row.building == building && row.count != 0 && row.resource == resource
                        })
                        .map_or(0, |row| row.amount)
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
    fn state_resource_amounts_follow_the_native_order_and_truncation() {
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
        use installed_content::triggers::Comparison::{Equal, Greater, GreaterEqual, Less};
        use resource_comparison as compare;
        assert_eq!(compare(2_499_999, Greater, 24), Some(false));
        assert_eq!(compare(2_500_000, Greater, 24), Some(true));
        assert_eq!(compare(-150_000, Equal, -1), Some(true));
        assert_eq!(compare(-150_000, Less, 0), Some(true));
        assert_eq!(compare(((1_i64 << 32) + 5) * 100_000, Equal, 5), Some(true));
        assert_eq!(compare(2_500_000, GreaterEqual, 24), None);
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
