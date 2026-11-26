
use std::{collections::BTreeMap, io, sync::OnceLock};

use content_reader_key_names::buildings::Buildings;
use content_reader_defines::defines::Defines;
use content_reader_parser::parser::Operator;
use replay_data::{
    rows::BuildingRow, rows::CountryBuildingResourceRow, rows::ResourceDefinitionRow,
    rows::SliceReadRow, rows::StateBaseResourceRow, rows::StateResourceInputRow, rows::StateResourceRow,
    rows::TableSchema, rows::mask, rows::tables,
};
use replay_frame::frame::{HourFrame, Rowset};

const ACTIVE: u64 = mask::<ResourceDefinitionRow>(&["token", "active"]);
const BASE_INPUT: u64 = mask::<StateResourceInputRow>(&["gate", "global_factor"]);
const ORIGIN_INPUT: u64 = mask::<StateResourceInputRow>(&[
    "origin",
    "infrastructure_level",
    "infrastructure_modifier",
    "supply_route",
]);
const SAMPLE: u64 = mask::<SliceReadRow>(&["sample_hour"]);

pub(crate) fn supply_read_key(state: u32) -> u128 {
    ((tables::StateResourceInputs::ID.index() as u128) << 32) | u128::from(state)
}
const STORED: u64 = mask::<StateResourceRow>(&["pool", "additive", "factor", "cost"]);
const BASE: u64 = mask::<StateBaseResourceRow>(&["amount"]);
const HEALTHY: u64 = mask::<BuildingRow>(&["healthy_level"]);
const OVERRIDE: u64 =
    mask::<CountryBuildingResourceRow>(&["building", "count", "resource", "amount"]);

type Index<T> = OnceLock<crate::triggers::refusal::Resolved<Result<T, crate::triggers::Failure>>>;
type Grants = BTreeMap<u32, Result<Vec<(u32, i64)>, crate::triggers::Failure>>;

pub const ONE: i64 = 100_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceOperands {
    pub template: i64,
    pub pool: i64,
    pub gate: bool,
    pub additive: i64,
    pub factor: i64,
    pub global_factor: i64,
    pub origin: Option<OriginFactors>,
    pub cost: i64,
    pub grants: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OriginFactors {
    pub infrastructure: Option<(i32, i64)>,
    pub supply_route: bool,
    pub bonuses: [i64; 2],
}

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

pub fn resource_comparison(amount: i64, comparison: Operator, wanted: i32) -> bool {
    int_comparison((amount / ONE) as i32, comparison, wanted)
}

pub fn int_comparison(value: i32, comparison: Operator, wanted: i32) -> bool {
    match comparison {
        Operator::Greater => wanted < value,
        Operator::Less => value < wanted,
        Operator::Equals => value == wanted,
    }
}

pub fn resource_bonuses(defines: &Defines) -> io::Result<[i64; 2]> {
    Ok([
        defines.fixed5("NBuildings", "INFRASTRUCTURE_RESOURCE_BONUS")?,
        defines.fixed5("NBuildings", "SUPPLY_ROUTE_RESOURCE_BONUS")?,
    ])
}

pub trait Links {
    fn slot(&self, raw: u32) -> Option<u32>;
    fn token_of(&self, name: &[u8]) -> Result<Option<u32>, String>;
    fn token_input(&self, name: &[u8]) -> Result<Option<u32>, crate::triggers::Failure> {
        self.token_of(name).map_err(Into::into)
    }
}

pub struct StateResources<'a> {
    frame: HourFrame<'a>,
    buildings: &'a Buildings,
    resource_bonuses: [i64; 2],
    resources: Index<BTreeMap<u32, u8>>,
    state_buildings: Index<BTreeMap<u32, Vec<(u32, i16)>>>,
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

    fn grants(&self, links: &impl Links) -> Result<&Grants, crate::triggers::Failure> {
        get(&self.grants, || {
            let resources = self.resource_database()?;
            let lists = grant_lists(
                self.buildings,
                |token| resources.get(&token).is_some_and(|&active| active != 0),
                |name| links.token_input(name),
            )?;
            Ok(lists
                .into_iter()
                .map(|(token, grants)| (token, grants.map_err(Into::into)))
                .collect::<Grants>())
        })
    }

    pub fn amount(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
        links: &impl Links,
    ) -> Result<i64, crate::triggers::Failure> {
        Ok(state_resource_amount(
            &self.operands(state, resource, controller, links, true)?,
        ))
    }

    pub fn base_operands(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
        links: &impl Links,
    ) -> Result<ResourceOperands, crate::triggers::Failure> {
        self.operands(state, resource, controller, links, false)
    }

    fn operands(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
        links: &impl Links,
        with_origin: bool,
    ) -> Result<ResourceOperands, crate::triggers::Failure> {
        let key = (u128::from(state) << 32) | u128::from(resource);
        let fields = if with_origin { BASE_INPUT | ORIGIN_INPUT } else { BASE_INPUT };
        let input = table::<tables::StateResourceInputs>(self.frame, fields)?
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
        let origin = if with_origin && input.origin != 0 {
            Some((
                input.infrastructure_level.map(|level| (level, input.infrastructure_modifier)),
                self.resource_bonuses[1] != 0 && self.supply_route(&input)?,
            ))
        } else {
            None
        };
        let stored = table::<tables::StateResources>(self.frame, STORED)?
            .get(key)
            .map_err(walked::<tables::StateResources>)?;
        let base = table::<tables::StateBaseResources>(self.frame, BASE)?
            .get(key)
            .map_err(walked::<tables::StateBaseResources>)?;
        Ok(resource_operands(
            base.map_or(0, |row| row.amount),
            stored
                .as_deref()
                .map(|row| [row.pool, row.additive, row.factor, row.cost]),
            input.gate != 0,
            input.global_factor,
            origin,
            self.resource_bonuses,
            self.building_grants(state, resource, controller, links)?,
        ))
    }

    fn supply_route(&self, input: &StateResourceInputRow) -> Result<bool, crate::triggers::Failure> {
        let state = input.state;
        let hour = self.frame.hour();
        let sample = self.frame.complete::<tables::SliceReads>(SAMPLE)
            .and_then(|rows| rows.get(supply_read_key(state)));
        let why = match sample {
            Ok(Some(row)) if row.sample_hour == hour => return Ok(input.supply_route != 0),
            Ok(Some(row)) => format!("read at hour {}", row.sample_hour),
            Ok(None) => "no slice read was recorded".into(),
            Err(error) => error.to_string(),
        };
        Err(crate::triggers::Failure::input(
            tables::StateResourceInputs::ID,
            "supply_route",
            Some(crate::triggers::Store::State(state)),
            format!("state {state}'s supply route was not recorded for hour {hour} ({why})"),
        ))
    }

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
        building_grants_from(
            buildings.get(&state).map(Vec::as_slice).unwrap_or(&[]),
            overrides,
            resource,
            |building| {
                let list = grants
                    .get(&building)
                    .ok_or_else(|| {
                        crate::triggers::Failure::from(format!(
                            "state {state} has a building type the content lacks"
                        ))
                    })?
                    .as_ref()
                    .map_err(|failure| {
                        failure.reached();
                        failure.clone()
                    })?;
                Ok(grant_of(list, resource))
            },
        )
    }
}

pub fn grant_of(list: &[(u32, i64)], resource: u32) -> Option<i64> {
    list.iter()
        .find(|(token, _)| *token == resource)
        .map(|&(_, amount)| amount)
}

pub fn building_grants_from<E>(
    buildings: &[(u32, i16)],
    overrides: &[(u32, u32, u32, u32, i64)],
    resource: u32,
    mut grant: impl FnMut(u32) -> Result<Option<i64>, E>,
) -> Result<i64, E> {
    let mut total = 0_i64;
    for &(building, healthy) in buildings {
        let Some(amount) = grant(building)? else {
            continue;
        };
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

pub fn grant_lists<E>(
    buildings: &Buildings,
    active: impl Fn(u32) -> bool,
    mut token_of: impl FnMut(&[u8]) -> Result<Option<u32>, E>,
) -> Result<BTreeMap<u32, Result<Vec<(u32, i64)>, String>>, E> {
    let mut out = BTreeMap::new();
    for (name, building) in &buildings.types {
        let Some(token) = token_of(name.as_bytes())? else {
            continue;
        };
        let grants = if let Some(reason) = &building.raid_refused {
            Err(format!("building {name}'s resource grants are refused: {reason}"))
        } else if !building.local_resources.is_empty() && buildings.ambiguous_slots.contains(name)
        {
            Err(format!(
                "building {name} is declared more than once with resource grants"
            ))
        } else {
            let mut grants = Vec::new();
            for (resource, amount) in &building.local_resources {
                if let Some(token) = token_of(resource.as_bytes())?
                    && active(token)
                {
                    grants.push((token, *amount));
                }
            }
            Ok(grants)
        };
        out.insert(token, grants);
    }
    Ok(out)
}

pub fn resource_operands(
    template: i64,
    stored: Option<[i64; 4]>,
    gate: bool,
    global_factor: i64,
    origin: Option<(Option<(i32, i64)>, bool)>,
    bonuses: [i64; 2],
    grants: i64,
) -> ResourceOperands {
    let [pool, additive, factor, cost] = stored.unwrap_or_default();
    ResourceOperands {
        template,
        pool,
        gate,
        additive,
        factor,
        global_factor,
        origin: origin.map(|(infrastructure, supply_route)| OriginFactors {
            infrastructure,
            supply_route,
            bonuses,
        }),
        cost,
        grants,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_bonus_requires_the_states_read_at_the_requested_hour() {
        use replay_data::rows::{SliceReadRow, TableId, TableRows};
        use replay_frame::frame::TableView;

        struct NoLinks;
        impl Links for NoLinks {
            fn slot(&self, _: u32) -> Option<u32> { None }
            fn token_of(&self, _: &[u8]) -> Result<Option<u32>, String> { Ok(None) }
        }

        let mut frame = replay_frame::frame::DecodedFrame {
            branch: 1,
            hour: 10,
            metrics: Default::default(),
            tables: TableId::ALL.map(|id| TableView::Complete(TableRows::empty(id)).into()),
        };
        let buildings = Buildings::default();
        *frame.tables[TableId::StateResources.index()] =
            TableView::Complete(TableRows::StateResources(vec![StateResourceRow {
                state: 3, resource: 7, pool: ONE, ..Default::default()
            }]));
        let inputs = |origin, supply_route| TableView::Complete(TableRows::StateResourceInputs(vec![
            StateResourceInputRow { state: 3, origin, supply_route, ..Default::default() }
        ]));
        let reads = |sample_hour| TableView::Complete(TableRows::SliceReads(vec![
            SliceReadRow { table: TableId::StateResourceInputs.index() as u32, owner: 3, sample_hour },
            SliceReadRow { table: TableId::StateResourceInputs.index() as u32, owner: 4, sample_hour: 11 },
        ]));
        let amount = |frame: &replay_frame::frame::DecodedFrame, bonus| {
            StateResources::new(frame, &buildings, [0, bonus]).amount(3, 7, None, &NoLinks)
        };
        *frame.tables[TableId::StateResourceInputs.index()] = inputs(1, 0);
        *frame.tables[TableId::SliceReads.index()] = reads(10);
        assert_eq!(amount(&frame, ONE).unwrap(), ONE);
        frame.hour = 11;
        assert!(amount(&frame, ONE).unwrap_err().names("StateResourceInputs.supply_route (State(3))"));
        assert_eq!(amount(&frame, 0).unwrap(), ONE);
        *frame.tables[TableId::StateResourceInputs.index()] = inputs(0, 0);
        assert_eq!(amount(&frame, ONE).unwrap(), ONE);
        *frame.tables[TableId::StateResourceInputs.index()] = inputs(1, 1);
        *frame.tables[TableId::SliceReads.index()] = reads(11);
        assert_eq!(amount(&frame, ONE).unwrap(), 2 * ONE);
        frame.hour = 12;
        assert!(amount(&frame, ONE).unwrap_err().names("StateResourceInputs.supply_route (State(3))"));
    }

    #[test]
    fn state_resource_amounts_preserve_order_and_truncation() {
        use state_resource_amount as amount;
        let plain = ResourceOperands {
            template: 2_000_000,
            pool: 300_000,
            additive: 100_000,
            ..Default::default()
        };
        assert_eq!(amount(&plain), 2_300_000);
        assert_eq!(
            amount(&ResourceOperands {
                gate: true,
                ..plain
            }),
            2_400_000
        );
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
