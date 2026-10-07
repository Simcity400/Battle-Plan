//! Fresh singleton attrition operands over the shared authenticated divisions.
//! Scalar fragments and formatter trees retain independent failures. All live
//! pointers finish here; spool projection reads only owned rows and arenas.

use crate::{
    spooling::{table_iter, unavailable},
    table_capture::{Arena, grow_capacity, grow_vec},
};
use engine::{
    Memory,
    allocation::{collect_vec, copy_vec, owned_vec},
    attrition_modifier_breakdown as tree, carmy, carmy_attrition as native, cname_group_member,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{
        self, ArmyAttritionInputs, AttritionBorder, AttritionModifierSource, AttritionSupply,
        AttritionTerrain, AttritionThermal, AttritionWeather, tables,
    },
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Capacities {
    rows: usize,
    terrain_text: usize,
    requests: usize,
    roots: usize,
    nodes: usize,
    text: usize,
    depth: usize,
}
impl Capacities {
    pub(super) fn raise(&mut self, demand: tree::Demand) {
        for (capacity, required) in [
            (&mut self.rows, demand.rows),
            (&mut self.terrain_text, demand.terrain_text),
            (&mut self.requests, demand.requests),
            (&mut self.roots, demand.roots),
            (&mut self.nodes, demand.nodes),
            (&mut self.text, demand.text),
            (&mut self.depth, demand.depth),
        ] {
            grow_capacity(capacity, required);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Row {
    division: u64,
    inputs: Option<native::Inputs>,
    terrain_name: Option<[usize; 2]>,
    name_failure: Option<&'static str>,
    dig_in_factor: Option<Result<i64, &'static str>>,
}

pub(super) struct Captured {
    rows: Arena<Row>,
    terrain_text: Arena<u8>,
    requests: Arena<tree::Request>,
    roots: Arena<tree::Root>,
    nodes: Arena<tree::RawNode>,
    text: Arena<u8>,
    stack: Vec<tree::Frame>,
    stack_required: usize,
    outcome: Outcome,
}

impl Captured {
    pub(super) fn new(c: Capacities) -> Self {
        Self {
            rows: Arena::new(c.rows),
            terrain_text: Arena::new(c.terrain_text),
            requests: Arena::new(c.requests),
            roots: Arena::new(c.roots),
            nodes: Arena::new(c.nodes),
            text: Arena::new(c.text),
            stack: owned_vec(engine::allocation::default_vec(c.depth)),
            stack_required: 0,
            outcome: Outcome::Unavailable {
                reason: "attrition source not acquired",
            },
        }
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.terrain_text.allocated_bytes()
            + self.requests.allocated_bytes()
            + self.roots.allocated_bytes()
            + self.nodes.allocated_bytes()
            + self.text.allocated_bytes()
            + self.stack.capacity() * std::mem::size_of::<tree::Frame>()
    }

    pub(super) fn reserve_rows(&mut self, count: usize) {
        grow_vec(&mut self.rows.first, count);
        let requests = count.saturating_mul(native::SOURCE_COUNT);
        grow_vec(&mut self.requests.first, requests);
        grow_vec(&mut self.roots.first, requests);
    }

    pub(super) fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.rows);
        self.terrain_text.grow(&mut c.terrain_text);
        self.requests.grow(&mut c.requests);
        self.roots.grow(&mut c.roots);
        self.nodes.grow(&mut c.nodes);
        self.text.grow(&mut c.text);
        grow_capacity(&mut c.depth, self.stack_required);
        grow_vec(&mut self.stack, c.depth);
    }

    pub(super) fn reset(&mut self) {
        self.rows.reset();
        self.terrain_text.reset();
        self.requests.reset();
        self.roots.reset();
        self.nodes.reset();
        self.text.reset();
        self.stack_required = 0;
        self.outcome = Outcome::Unavailable {
            reason: "attrition source not acquired",
        };
    }

    pub(super) fn fail(&mut self, reason: &'static str) {
        self.reset();
        self.outcome = Outcome::Unavailable { reason };
    }

    pub(super) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[carmy::DivisionSeed],
        outcome: Outcome,
        status: &carmy::StatusPacket,
    ) {
        self.reset();
        self.rows.required = outcome.len();
        if !matches!(outcome, Outcome::Rows { .. })
            || outcome.len() > seeds.len()
            || outcome.len() > self.rows.first.len()
        {
            self.outcome = Outcome::Unavailable {
                reason: "attrition division census incomplete",
            };
            return;
        }
        for (index, seed) in seeds[..outcome.len()].iter().enumerate() {
            let inputs = native::capture(memory, base, state, seed, status.exercising_at(index));
            let (terrain_name, name_failure) = match inputs.terrain {
                Ok(Some(terrain)) => match terrain
                    .name
                    .and_then(|address| self.copy_name(memory, address))
                {
                    Ok(span) => (Some(span), None),
                    Err(reason) => (None, Some(reason)),
                },
                _ => (None, None),
            };
            for source in inputs.sources.iter().flatten().flatten() {
                let at = self.requests.required;
                self.requests.required = self.requests.required.saturating_add(1);
                if let Some(slot) = self.requests.first.get_mut(at) {
                    *slot = tree::Request {
                        accumulator: source.accumulator,
                        key: source.key,
                    };
                    self.requests.len += 1;
                }
            }
            self.rows.first[index] = Row {
                division: seed.division,
                inputs: Some(inputs),
                terrain_name,
                name_failure,
                dig_in_factor: Some(carmy::counter_dig_in_factor(memory, base, seed)),
            };
            self.rows.len += 1;
        }
        if self.requests.len != self.requests.required {
            self.outcome = Outcome::Unavailable {
                reason: "attrition request arena capacity exceeded",
            };
            return;
        }
        let mut buffers = tree::Buffers {
            requests: &mut self.requests.first,
            request_len: self.requests.len,
            request_required: self.requests.required,
            roots: &mut self.roots.first,
            root_len: 0,
            root_required: 0,
            nodes: &mut self.nodes.first,
            node_len: 0,
            node_required: 0,
            text: &mut self.text.first,
            text_len: 0,
            text_required: 0,
            stack: &mut self.stack,
            stack_required: 0,
        };
        let result = tree::capture(memory, base, &mut buffers);
        self.requests.len = buffers.request_len;
        self.requests.required = self.requests.required.max(buffers.request_required);
        self.roots.len = buffers.root_len;
        self.roots.required = buffers.root_required;
        self.nodes.len = buffers.node_len;
        self.nodes.required = buffers.node_required;
        self.text.len = buffers.text_len;
        self.text.required = buffers.text_required;
        self.stack_required = buffers.stack_required;
        self.outcome = match result {
            Ok(()) => Outcome::Rows {
                len: self.roots.len,
            },
            Err(reason) => Outcome::Unavailable { reason },
        };
    }

    fn copy_name(
        &mut self,
        memory: &impl Memory,
        address: usize,
    ) -> Result<[usize; 2], &'static str> {
        let mut names = cname_group_member::Names {
            bytes: &mut self.terrain_text.first,
            len: self.terrain_text.len,
            required: self.terrain_text.required,
        };
        let result = native::copy_terrain_name(memory, address, &mut names);
        self.terrain_text.required = names.required;
        let (start, len) = result?.ok_or("attrition terrain text arena capacity exceeded")?;
        self.terrain_text.len = start + len;
        Ok([start, len])
    }

    pub(super) fn dig_in_factor(&self, division: u64, index: usize) -> Option<i64> {
        let row = self.rows.first.get(index)?;
        if index >= self.rows.len || row.division != division {
            return None;
        }
        row.dig_in_factor?.ok()
    }

    pub(super) fn project(&self, division: u64, index: usize) -> ArmyAttritionInputs {
        let mut inputs = self.project_values(division, index);
        if let Some(Some(terrain)) = &mut inputs.terrain {
            terrain.name = self
                .terrain_name(division, index)
                .map(|name| owned_vec(copy_vec(name)));
        }
        inputs
    }

    fn terrain_name(&self, division: u64, index: usize) -> Option<&[u8]> {
        let row = self.rows.first.get(index)?;
        if index >= self.rows.len || row.division != division {
            return None;
        }
        range(
            &self.terrain_text.first[..self.terrain_text.len],
            row.terrain_name?,
        )
    }

    pub(super) fn matches(
        &self,
        division: u64,
        index: usize,
        previous: &replay::rows::Recorded<ArmyAttritionInputs>,
    ) -> bool {
        let replay::rows::Recorded::Value(previous) = previous else {
            return false;
        };
        let inputs = self.project_values(division, index);
        let terrain = match (&inputs.terrain, &previous.terrain) {
            (None, None) | (Some(None), Some(None)) => true,
            (Some(Some(current)), Some(Some(previous))) => {
                current.attrition == previous.attrition
                    && current.equipment == previous.equipment
                    && self.terrain_name(division, index) == previous.name.as_deref()
            }
            _ => false,
        };
        terrain
            && inputs.province == previous.province
            && inputs.upkeep == previous.upkeep
            && inputs.state == previous.state
            && inputs.supply == previous.supply
            && inputs.country_upkeep_modifier == previous.country_upkeep_modifier
            && inputs.moving_state == previous.moving_state
            && inputs.weather == previous.weather
            && inputs.training == previous.training
            && inputs.training_attrition == previous.training_attrition
            && inputs.border == previous.border
            && inputs.country_attrition_modifier == previous.country_attrition_modifier
            && inputs.sources == previous.sources
    }

    fn project_values(&self, division: u64, index: usize) -> ArmyAttritionInputs {
        let Some(row) = self
            .rows
            .first
            .get(index)
            .filter(|row| index < self.rows.len && row.division == division)
        else {
            return ArmyAttritionInputs::default();
        };
        let Some(inputs) = row.inputs else {
            return ArmyAttritionInputs::default();
        };
        let thermal = |value: native::Thermal| AttritionThermal {
            base: value.base,
            country_factor: value.country_factor.ok(),
            leader_factor: value.leader_factor.ok(),
        };
        ArmyAttritionInputs {
            province: inputs.province.ok(),
            upkeep: inputs.upkeep.ok(),
            terrain: inputs.terrain.ok().map(|terrain| {
                terrain.map(|terrain| AttritionTerrain {
                    name: None,
                    attrition: terrain.attrition.ok(),
                    equipment: terrain.equipment.ok(),
                })
            }),
            state: inputs.state.ok(),
            supply: inputs.supply.ok().map(|supply| AttritionSupply {
                penalty: supply.penalty.ok(),
                attrition_factor: supply.attrition_factor.ok(),
            }),
            country_upkeep_modifier: inputs.country_upkeep_modifier.ok(),
            moving_state: inputs.moving_state.ok(),
            weather: inputs.weather.ok().map(|weather| {
                weather.map(|weather| AttritionWeather {
                    acclimatization: weather.acclimatization.ok(),
                    attrition: weather.attrition.ok(),
                    heat: weather.heat.ok().map(|value| value.map(thermal)),
                    winter: weather.winter.ok().map(|value| value.map(thermal)),
                })
            }),
            training: inputs.training.ok(),
            training_attrition: inputs.training_attrition.ok(),
            border: inputs.border.ok().map(|border| {
                border.map(|border| AttritionBorder {
                    state: border.state.ok(),
                    country: border.country.ok(),
                    factor: border.factor.ok(),
                })
            }),
            country_attrition_modifier: inputs.country_attrition_modifier.ok(),
            sources: inputs.sources.map(|source| {
                source.ok().map(|source| {
                    source.map(|source| AttritionModifierSource {
                        accumulator: source.accumulator as u64,
                        key: source.key,
                        depth: source.depth,
                        scale: source.scale,
                    })
                })
            }),
        }
    }

    pub(super) fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub(super) fn table(&self) -> TableView {
        if let Outcome::Unavailable { reason } = self.outcome {
            return unavailable(reason);
        }
        let names = &self.text.first[..self.text.len];
        if self.roots.first[..self.roots.len].iter().any(|root| {
            root.failure.is_none()
                && range(&self.nodes.first[..self.nodes.len], root.nodes).is_none_or(|nodes| {
                    nodes.is_empty() || nodes.iter().any(|node| range(names, node.name).is_none())
                })
        }) {
            return unavailable("attrition modifier owned spans invalid");
        }
        table_iter::<tables::AttritionModifierBreakdowns, _>(
            self.outcome,
            self.roots.first[..self.roots.len].iter(),
            |root| rows::AttritionModifierBreakdownRow {
                accumulator: root.accumulator as u64,
                key: root.key,
                nodes: (root.failure.is_none()).then(|| {
                    owned_vec(collect_vec(
                        range(&self.nodes.first[..self.nodes.len], root.nodes)
                            .unwrap()
                            .iter()
                            .map(|node| rows::ModifierBreakdownNode {
                                children: node.children,
                                value: node.value,
                                name: owned_vec(copy_vec(range(names, node.name).unwrap())),
                            }),
                    ))
                }),
            },
        )
    }
}

fn range<T>(values: &[T], [start, len]: [usize; 2]) -> Option<&[T]> {
    values.get(start..start.checked_add(len)?)
}

#[derive(Default)]
pub(super) struct Work {
    captured: u64,
    refusals: Vec<(&'static str, u64)>,
}
impl Work {
    fn observe_field<T>(&mut self, value: Result<T, &'static str>) -> std::io::Result<()> {
        match value {
            Ok(_) => self.captured = self.captured.saturating_add(1),
            Err(reason) => {
                let index = match self.refusals.binary_search_by_key(&reason, |entry| entry.0) {
                    Ok(index) => index,
                    Err(index) => {
                        engine::allocation::reserve(&mut self.refusals, 1)?;
                        self.refusals.insert(index, (reason, 0));
                        index
                    }
                };
                self.refusals[index].1 = self.refusals[index].1.saturating_add(1);
            }
        }
        Ok(())
    }
    pub(super) fn observe(&mut self, captured: &Captured) -> std::io::Result<()> {
        for row in &captured.rows.first[..captured.rows.len] {
            if let Some(reason) = row.name_failure {
                self.observe_field::<()>(Err(reason))?;
            }
            if let Some(value) = row.dig_in_factor {
                self.observe_field(value)?;
            }
            let Some(inputs) = row.inputs else {
                continue;
            };
            self.observe_field(inputs.province)?;
            self.observe_field(inputs.upkeep)?;
            self.observe_field(inputs.terrain)?;
            self.observe_field(inputs.state)?;
            self.observe_field(inputs.supply)?;
            self.observe_field(inputs.country_upkeep_modifier)?;
            self.observe_field(inputs.moving_state)?;
            self.observe_field(inputs.weather)?;
            self.observe_field(inputs.training)?;
            self.observe_field(inputs.training_attrition)?;
            self.observe_field(inputs.border)?;
            self.observe_field(inputs.country_attrition_modifier)?;
            if let Ok(Some(border)) = inputs.border {
                self.observe_field(border.state)?;
                self.observe_field(border.country)?;
                self.observe_field(border.factor)?;
            }
            if let Ok(Some(terrain)) = inputs.terrain {
                self.observe_field(terrain.attrition)?;
                self.observe_field(terrain.equipment)?;
            }
            if let Ok(supply) = inputs.supply {
                self.observe_field(supply.penalty)?;
                self.observe_field(supply.attrition_factor)?;
            }
            if let Ok(Some(weather)) = inputs.weather {
                self.observe_field(weather.acclimatization)?;
                self.observe_field(weather.attrition)?;
                for thermal in [weather.heat, weather.winter] {
                    self.observe_field(thermal)?;
                    if let Ok(Some(thermal)) = thermal {
                        self.observe_field(thermal.country_factor)?;
                        self.observe_field(thermal.leader_factor)?;
                    }
                }
            }
            for source in inputs.sources {
                self.observe_field(source)?;
            }
        }
        self.observe_field(match captured.outcome {
            Outcome::Unavailable { reason } => Err(reason),
            _ => Ok(()),
        })?;
        for root in &captured.roots.first[..captured.roots.len] {
            self.observe_field(root.failure.map_or(Ok(()), Err))?;
        }
        Ok(())
    }
    pub(super) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({ "captured_fragments": self.captured,
            "refused_fragments": self.refusals.iter().fold(0_u64, |sum, (_, count)| sum.saturating_add(*count)),
            "refusals": crate::status::map(self.refusals.iter().copied()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    fn inputs() -> native::Inputs {
        native::Inputs {
            province: Ok(Some(7)),
            upkeep: Ok(321),
            terrain: Ok(None),
            state: Ok(None),
            supply: Ok(native::Supply {
                penalty: Ok(0),
                attrition_factor: Ok(10),
            }),
            country_upkeep_modifier: Ok(0),
            moving_state: Ok(None),
            weather: Ok(None),
            training: Ok(false),
            training_attrition: Ok(None),
            border: Ok(None),
            country_attrition_modifier: Ok(0),
            sources: [Ok(None); native::SOURCE_COUNT],
        }
    }

    #[test]
    fn attrition_projection_keeps_independent_terrain_thermal_and_border_fragments() {
        let mut captured = Captured::new(Capacities {
            rows: 1,
            ..Default::default()
        });
        let mut values = inputs();
        values.terrain = Ok(Some(native::Terrain {
            name: Err("label fault"),
            attrition: Ok(17),
            equipment: Err("equipment fault"),
        }));
        values.weather = Ok(Some(native::Weather {
            acclimatization: Ok(90_000),
            attrition: Ok(23),
            heat: Ok(Some(native::Thermal {
                base: 31,
                country_factor: Err("heat factor fault"),
                leader_factor: Ok(None),
            })),
            winter: Ok(Some(native::Thermal {
                base: 41,
                country_factor: Ok(0),
                leader_factor: Ok(Some(-10_000)),
            })),
        }));
        values.border = Ok(Some(native::Border {
            state: Ok(7),
            country: Err("border country fault"),
            factor: Err("border factor fault"),
        }));
        captured.rows.first[0] = Row {
            division: 9,
            inputs: Some(values),
            name_failure: Some("label fault"),
            dig_in_factor: Some(Ok(100_000)),
            ..Default::default()
        };
        captured.rows.len = 1;
        let projected = captured.project(9, 0);
        assert!(captured.matches(9, 0, &replay::rows::Recorded::Value(projected.clone())));
        assert!(!captured.matches(9, 0, &replay::rows::Recorded::NotRecorded));
        assert_eq!(projected.upkeep, Some(321));
        let terrain = projected.terrain.unwrap().unwrap();
        assert_eq!(
            (terrain.name, terrain.attrition, terrain.equipment),
            (None, Some(17), None)
        );
        let weather = projected.weather.unwrap().unwrap();
        assert_eq!(weather.attrition, Some(23));
        assert_eq!(weather.heat.unwrap().unwrap().country_factor, None);
        assert_eq!(weather.winter.unwrap().unwrap().base, 41);
        let border = projected.border.unwrap().unwrap();
        assert_eq!(
            (border.state, border.country, border.factor),
            (Some(7), None, None)
        );
        assert_eq!(captured.dig_in_factor(9, 0), Some(100_000));
        let mut work = Work::default();
        work.observe(&captured).unwrap();
        for reason in [
            "label fault",
            "equipment fault",
            "heat factor fault",
            "border country fault",
            "border factor fault",
        ] {
            assert!(
                work.refusals
                    .iter()
                    .any(|&(found, count)| found == reason && count == 1)
            );
        }
    }

    #[test]
    fn attrition_root_failure_preserves_other_tree_and_numeric_terms() {
        let mut captured = Captured::new(Capacities {
            rows: 1,
            roots: 2,
            nodes: 1,
            text: 3,
            ..Default::default()
        });
        captured.rows.first[0] = Row {
            division: 9,
            inputs: Some(inputs()),
            ..Default::default()
        };
        captured.rows.len = 1;
        captured.roots.first[0] = tree::Root {
            accumulator: 0x1000,
            key: 0x4f,
            failure: Some("tree fault"),
            ..Default::default()
        };
        captured.roots.first[1] = tree::Root {
            accumulator: 0x2000,
            key: 0x4f,
            nodes: [0, 1],
            failure: None,
        };
        captured.roots.len = 2;
        captured.nodes.first[0] = tree::RawNode {
            children: 0,
            value: -17,
            name: [0, 3],
        };
        captured.nodes.len = 1;
        captured.text.first[..3].copy_from_slice(&[0, 0xff, b'X']);
        captured.text.len = 3;
        captured.outcome = Outcome::Rows { len: 2 };
        let TableView::Complete(TableRows::AttritionModifierBreakdowns(rows)) = captured.table()
        else {
            panic!("root-local faults retain complete identity census");
        };
        assert_eq!(rows[0].nodes, None);
        assert_eq!(rows[1].nodes.as_ref().unwrap()[0].name, [0, 0xff, b'X']);
        assert_eq!(captured.project(9, 0).evaluate(None), Ok(321));
        let mut work = Work::default();
        work.observe(&captured).unwrap();
        assert_eq!(work.refusals, [("tree fault", 1)]);
    }

    #[test]
    fn attrition_owned_span_fault_is_refused_without_changing_scalar_projection() {
        let mut captured = Captured::new(Capacities {
            rows: 1,
            roots: 1,
            ..Default::default()
        });
        captured.rows.first[0] = Row {
            division: 9,
            inputs: Some(inputs()),
            ..Default::default()
        };
        captured.rows.len = 1;
        captured.roots.first[0] = tree::Root {
            accumulator: 1,
            key: 0x4f,
            nodes: [0, 1],
            failure: None,
        };
        captured.roots.len = 1;
        captured.outcome = Outcome::Rows { len: 1 };
        assert!(matches!(captured.table(), TableView::Unavailable { reason }
            if reason == "attrition modifier owned spans invalid"));
        assert_eq!(captured.project(9, 0).evaluate(None), Ok(321));
    }
}
