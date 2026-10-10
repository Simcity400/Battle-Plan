//! Daily production-hover supplement, independent of the ordinary census.
//! Native formatter requests use the shared caller-owned tree reader. Buffers
//! grow on the acquisition worker; each failed root retains its original verdict.
//! Production selects its own daily requests and retains capture pressure for
//! off-hour growth; attrition_modifier_breakdown::demand only sizes army hovers.

use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::Arena,
};
use engine::{
    Memory,
    allocation::{collect_vec, copy_vec, owned_vec},
    attrition_modifier_breakdown as tree, production_modifiers as native,
    walk::Outcome,
};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::CountryProductionModifiers,
    TableId::ProductionLineModifiers,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("production modifier operands and source trees", 0, &[0, 1])];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    countries: usize,
    lines: usize,
    efficiencies: usize,
    requests: usize,
    nodes: usize,
    text: usize,
    stack: usize,
}
impl Capacities {
    pub(crate) fn raise_for_demand(&mut self, demand: native::Demand) {
        for (capacity, required) in [
            (&mut self.countries, demand.countries),
            (&mut self.lines, demand.lines),
            (&mut self.efficiencies, demand.efficiencies),
            (&mut self.requests, demand.requests),
        ] {
            crate::country_scale::raise(capacity, crate::country_scale::rows(required, 1));
        }
    }

    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.countries, crate::country_scale::rows(slots, 1));
        crate::country_scale::raise(
            &mut self.requests,
            crate::country_scale::rows(slots, native::KEYS.len()),
        );
    }
}

pub struct Captured {
    countries: Arena<native::Country>,
    lines: Arena<native::Line>,
    efficiencies: Arena<i64>,
    requests: Arena<tree::Request>,
    roots: Arena<tree::Root>,
    nodes: Arena<tree::RawNode>,
    text: Arena<u8>,
    stack: Arena<tree::Frame>,
    archetypes: Arena<usize>,
    context: Option<native::Context>,
    outcome: Outcome,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            countries: Arena::new(capacity.countries),
            lines: Arena::new(capacity.lines),
            efficiencies: Arena::new(capacity.efficiencies),
            requests: Arena::new(capacity.requests),
            roots: Arena::new(capacity.requests),
            nodes: Arena::new(capacity.nodes),
            text: Arena::new(capacity.text),
            stack: Arena::new(capacity.stack),
            archetypes: Arena::new(capacity.stack),
            context: None,
            outcome: Outcome::Unavailable {
                reason: "clock unavailable",
            },
        }
    }
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            countries: 0,
            lines: 0,
            efficiencies: 0,
            requests: 0,
            nodes: 0,
            text: 0,
            stack: 0,
        })
    }
    pub fn reset(&mut self) {
        self.countries.reset();
        self.lines.reset();
        self.efficiencies.reset();
        self.requests.reset();
        self.roots.reset();
        self.nodes.reset();
        self.text.reset();
        self.stack.reset();
        self.archetypes.reset();
        self.context = None;
        self.outcome = Outcome::Unavailable {
            reason: "clock unavailable",
        };
    }
    pub fn allocated_bytes(&self) -> usize {
        self.countries.allocated_bytes()
            + self.lines.allocated_bytes()
            + self.efficiencies.allocated_bytes()
            + self.requests.allocated_bytes()
            + self.roots.allocated_bytes()
            + self.nodes.allocated_bytes()
            + self.text.allocated_bytes()
            + self.stack.allocated_bytes()
            + self.archetypes.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.countries.grow(&mut capacity.countries);
        self.lines.grow(&mut capacity.lines);
        self.efficiencies.grow(&mut capacity.efficiencies);
        self.requests.grow(&mut capacity.requests);
        self.roots.grow(&mut capacity.requests);
        self.nodes.grow(&mut capacity.nodes);
        crate::table_capture::grow_vec(&mut self.requests.first, capacity.requests);
        self.text.grow(&mut capacity.text);
        self.stack.grow(&mut capacity.stack);
        self.archetypes.grow(&mut capacity.stack);
        crate::table_capture::grow_vec(&mut self.stack.first, capacity.stack);
    }
    pub(crate) fn fail_capture(&mut self, hour: i32, full: bool, reason: &'static str) {
        self.outcome = super::failed_capture(TABLES[0], hour, full, reason);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        full: bool,
    ) -> [u64; N] {
        self.reset();
        if !should_walk(TABLES[0], hour, full) {
            self.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            return [0; N];
        }
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        let result = (|| {
            let mut context = None;
            let mut inputs = native::GrowingBuffers {
                countries: &mut self.countries.first,
                country_len: 0,
                country_required: 0,
                lines: &mut self.lines.first,
                line_len: 0,
                line_required: 0,
                efficiencies: &mut self.efficiencies.first,
                efficiency_len: 0,
                efficiency_required: 0,
                requests: &mut self.requests.first,
                request_len: 0,
                request_required: 0,
                archetypes: &mut self.archetypes.first,
                archetype_required: 0,
            };
            let result = native::capture_growing(memory, base, state, &mut context, &mut inputs);
            self.countries.len = inputs.country_len;
            self.countries.required = inputs.country_required;
            self.lines.len = inputs.line_len;
            self.lines.required = inputs.line_required;
            self.efficiencies.len = inputs.efficiency_len;
            self.efficiencies.required = inputs.efficiency_required;
            self.requests.len = inputs.request_len;
            self.requests.required = inputs.request_required;
            self.archetypes.required = inputs.archetype_required;
            result?;
            if self.requests.len == 0 {
                // No country/line rows can publish any of these unused values.
                self.context = Some(native::Context::default());
                return Ok(());
            }
            let mut trees = tree::Buffers {
                requests: engine::walk::Buffer::Growing(&mut self.requests.first),
                request_len: self.requests.len,
                request_required: self.requests.required,
                roots: engine::walk::Buffer::Growing(&mut self.roots.first),
                root_len: 0,
                root_required: 0,
                nodes: engine::walk::Buffer::Growing(&mut self.nodes.first),
                node_len: 0,
                node_required: 0,
                text: engine::walk::Buffer::Growing(&mut self.text.first),
                text_len: 0,
                text_required: 0,
                stack: engine::walk::Buffer::Growing(&mut self.stack.first),
                stack_required: 0,
            };
            let result = tree::capture(memory, base, &mut trees);
            self.requests.len = trees.request_len;
            self.requests.required = trees.request_required;
            self.roots.len = trees.root_len;
            self.roots.required = trees.root_required;
            self.nodes.len = trees.node_len;
            self.nodes.required = trees.node_required;
            self.text.len = trees.text_len;
            self.text.required = trees.text_required;
            self.stack.required = trees.stack_required;
            result?;
            self.context = context;
            Ok(())
        })();
        self.outcome = match result {
            Ok(()) => Outcome::Rows {
                len: self.lines.len,
            },
            Err(reason) => Outcome::Unavailable { reason },
        };
        [qpc().saturating_sub(started).max(0) as u64, 0]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        match self.outcome {
            Outcome::Rows { .. } => [
                Outcome::Rows {
                    len: self.countries.len,
                },
                Outcome::Rows {
                    len: self.lines.len,
                },
            ],
            failure => [failure; N],
        }
    }
    pub fn tables(&self) -> [TableView; N] {
        let Some(context) = self.context else {
            let reason = match self.outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "production modifier context absent",
            };
            return [unavailable(reason), unavailable(reason)];
        };
        let span = |start: usize, len: usize| start.checked_add(len);
        let roots = &self.roots.first[..self.roots.len];
        let nodes = &self.nodes.first[..self.nodes.len];
        let text = &self.text.first[..self.text.len];
        if nodes
            .iter()
            .any(|node| span(node.name[0], node.name[1]).is_none_or(|end| end > text.len()))
            || roots
                .iter()
                .any(|root| span(root.nodes[0], root.nodes[1]).is_none_or(|end| end > nodes.len()))
            || self.lines.first[..self.lines.len].iter().any(|line| {
                span(line.efficiencies[0], line.efficiencies[1])
                    .is_none_or(|end| end > self.efficiencies.len)
            })
        {
            return [
                unavailable("production modifier arena span invalid"),
                unavailable("production modifier arena span invalid"),
            ];
        }
        let source = &self.countries.first[..self.countries.len];
        if source.iter().any(|country| {
            native::KEYS.iter().any(|key| {
                roots
                    .binary_search_by_key(&(country.accumulator, *key), |root| {
                        (root.accumulator, root.key)
                    })
                    .is_err()
            })
        }) {
            return [
                unavailable("production modifier source request missing"),
                unavailable("production modifier source request missing"),
            ];
        }
        let outcomes = self.outcomes();
        [
            table::<tables::CountryProductionModifiers, _>(outcomes[0], source, |country| {
                let trees = owned_vec(collect_vec(native::KEYS.iter().map(|key| {
                    let root = &roots[roots
                        .binary_search_by_key(&(country.accumulator, *key), |root| {
                            (root.accumulator, root.key)
                        })
                        .unwrap()];
                    rows::ProductionModifierTree {
                        key: *key,
                        failure: owned_vec(copy_vec(root.failure.unwrap_or_default().as_bytes())),
                        nodes: owned_vec(collect_vec(
                            nodes[root.nodes[0]..root.nodes[0] + root.nodes[1]]
                                .iter()
                                .map(|node| rows::ModifierBreakdownNode {
                                    children: node.children,
                                    value: node.value,
                                    name: owned_vec(copy_vec(
                                        &text[node.name[0]..node.name[0] + node.name[1]],
                                    )),
                                }),
                        )),
                    }
                })));
                rows::CountryProductionModifiersRow {
                    country: country.country,
                    power_raw: country.power_raw,
                    base_start: context.start,
                    base_cap: context.cap,
                    gain_raw: context.gain,
                    balance_raw: context.balance,
                    factory_speed: context.factory_speed,
                    retention: context.retention,
                    conversion_raw: context.conversion,
                    trees,
                }
            }),
            table::<tables::ProductionLineModifiers, _>(
                outcomes[1],
                &self.lines.first[..self.lines.len],
                |line| rows::ProductionLineModifiersRow {
                    line: line.handle,
                    country: line.country,
                    variant: line.variant,
                    organisation: line.organisation,
                    kind: line.kind,
                    active_factories: line.active_factories,
                    resource_penalty_raw: line.resource_penalty,
                    organisation_dirty: line.organisation_dirty,
                    conversion: line.conversion,
                    licensed: line.licensed,
                    bonuses: line.bonuses,
                    conversion_output: line.conversion_output,
                    refit_output: line.refit_output,
                    naval_efficiency: line.naval_efficiency,
                    naval_output: line.naval_output,
                    factory_efficiencies: owned_vec(copy_vec(
                        &self.efficiencies.first
                            [line.efficiencies[0]..line.efficiencies[0] + line.efficiencies[1]],
                    )),
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_factory_headers_presize_the_first_production_modifier_capture() {
        let mut capacity = Capacities::default();
        let demand = native::Demand {
            countries: 5_000,
            lines: 30_000,
            efficiencies: 750_000,
            requests: 5_000 * native::KEYS.len(),
        };
        capacity.raise_for_demand(demand);
        assert!(capacity.countries >= demand.countries);
        assert!(capacity.lines >= demand.lines);
        assert!(capacity.efficiencies >= demand.efficiencies);
        assert!(capacity.requests >= demand.requests);
        let before = capacity;
        capacity.raise_for_demand(native::Demand::default());
        assert!(capacity == before);
    }
    #[test]
    fn five_thousand_countries_and_failed_arena_pressure_grow_off_hour() {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(5001);
        let mut captured = Captured::with_capacities(&capacity);
        assert!(captured.countries.first.len() >= 5000);
        assert!(captured.requests.first.len() >= 5000 * native::KEYS.len());
        captured.efficiencies.required = captured.efficiencies.first.len() + 1;
        captured.archetypes.required = captured.archetypes.first.len() + 1;
        captured.grow(&mut capacity);
        assert!(captured.efficiencies.first.len() >= captured.efficiencies.required);
        assert!(captured.stack.first.len() >= captured.archetypes.required);
    }
    #[test]
    fn failed_root_preserves_independent_native_sources_and_negative_factory_entries() {
        use replay::rows::TableRows;
        let mut capacity = Capacities {
            lines: 1,
            efficiencies: 2,
            nodes: 1,
            text: 3,
            ..Default::default()
        };
        capacity.raise_for_countries(2);
        let mut captured = Captured::with_capacities(&capacity);
        captured.context = Some(native::Context::default());
        captured.outcome = Outcome::Rows { len: 1 };
        captured.countries.first[0] = native::Country {
            country: 9,
            accumulator: 123,
            power_raw: -20,
        };
        captured.countries.len = 1;
        captured.lines.first[0] = native::Line {
            handle: 22,
            country: 9,
            efficiencies: [0, 2],
            ..Default::default()
        };
        captured.lines.len = 1;
        captured.efficiencies.first[..2].copy_from_slice(&[-123, 456]);
        captured.efficiencies.len = 2;
        captured.nodes.first[0] = tree::RawNode {
            children: 0,
            value: -7,
            name: [0, 3],
        };
        captured.nodes.len = 1;
        captured.text.first[..3].copy_from_slice(&[0xff, 0, b'k']);
        captured.text.len = 3;
        for (index, key) in native::KEYS.into_iter().enumerate() {
            captured.roots.first[index] = tree::Root {
                accumulator: 123,
                key,
                nodes: if index == 0 { [0, 0] } else { [0, 1] },
                failure: if index == 0 {
                    Some("synthetic root failure")
                } else {
                    None
                },
            };
        }
        captured.roots.len = native::KEYS.len();
        let [countries, lines] = captured.tables();
        let TableView::Complete(TableRows::CountryProductionModifiers(countries)) = countries
        else {
            panic!("missing country projection");
        };
        assert_eq!(countries[0].trees[0].failure, b"synthetic root failure");
        assert_eq!(countries[0].trees[1].nodes[0].name, [0xff, 0, b'k']);
        assert_eq!(countries[0].trees[1].nodes[0].value, -7);
        let TableView::Complete(TableRows::ProductionLineModifiers(lines)) = lines else {
            panic!("missing line projection");
        };
        assert_eq!(lines[0].factory_efficiencies, [-123, 456]);
    }
}
