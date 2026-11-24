
use crate::{
    clock::qpc,
    table_capture::Arena,
    table_conversion::{table_iter, unavailable},
};
use engine_allocation::{collect_vec, copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_production::{attrition_modifier_breakdown as tree, production_modifiers as native};
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::CountryProductionModifiers,
    TableId::ProductionLineModifiers,
];

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

collector_primitives::capacity_max!(Capacities {
    countries,
    lines,
    efficiencies,
    requests,
    nodes,
    text,
    stack
});
impl Capacities {
    pub fn raise_for_demand(&mut self, demand: native::Demand) {
        for (capacity, required) in [
            (&mut self.countries, demand.countries),
            (&mut self.lines, demand.lines),
            (&mut self.efficiencies, demand.efficiencies),
            (&mut self.requests, demand.requests),
        ] {
            crate::country_scale::raise(capacity, crate::country_scale::rows(required, 1));
        }
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.countries, crate::country_scale::rows(slots, 1));
        crate::country_scale::raise(
            &mut self.requests,
            crate::country_scale::rows(slots, native::KEYS.len()),
        );
    }
}

pub struct Part {
    countries: Arena<native::Country>,
    lines: Arena<native::Line>,
    efficiencies: Arena<i64>,
    requests: Arena<tree::Request>,
    roots: Arena<tree::Root>,
    nodes: Arena<tree::RawNode>,
    text: Arena<u8>,
    stack: Arena<tree::Frame>,
    archetypes: Arena<usize>,
    repeats: tree::RepeatCache,
    context: Option<native::Context>,
    outcome: Outcome,
}
impl Part {
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
            repeats: tree::RepeatCache::default(),
            context: None,
            outcome: Outcome::Unavailable {
                reason: "clock unavailable",
            },
        }
    }
    pub fn vacant() -> Self {
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
            + self.repeats.allocated_bytes()
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
    pub fn fail_capture(&mut self, hour: i32, full: bool, reason: &'static str) {
        self.outcome = super::failed_capture(TABLES[0], hour, full, reason);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        full: bool,
        slots: std::ops::Range<u32>,
        provided: Option<Result<native::Context, &'static str>>,
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
            let mut inputs = native::Buffers {
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
            let result = native::capture_slots(
                memory,
                base,
                state,
                slots,
                provided,
                &mut context,
                &mut inputs,
            );
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
                self.context = Some(native::Context::default());
                return Ok(());
            }
            let mut trees = tree::Buffers {
                requests: engine_native_memory::walk::Buffer::Growing(&mut self.requests.first),
                request_len: self.requests.len,
                request_required: self.requests.required,
                roots: engine_native_memory::walk::Buffer::Growing(&mut self.roots.first),
                root_len: 0,
                root_required: 0,
                nodes: engine_native_memory::walk::Buffer::Growing(&mut self.nodes.first),
                node_len: 0,
                node_required: 0,
                text: engine_native_memory::walk::Buffer::Growing(&mut self.text.first),
                text_len: 0,
                text_required: 0,
                stack: engine_native_memory::walk::Buffer::Growing(&mut self.stack.first),
                stack_required: 0,
                repeats: Some(&mut self.repeats),
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
    fn invalid(&self) -> Option<&'static str> {
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
            return Some("production modifier arena span invalid");
        }
        if self.countries.first[..self.countries.len]
            .iter()
            .any(|country| {
                native::KEYS
                    .iter()
                    .any(|key| self.root(country.accumulator, *key).is_none())
            })
        {
            return Some("production modifier source request missing");
        }
        None
    }
    fn root(&self, accumulator: usize, key: u32) -> Option<&tree::Root> {
        let roots = &self.roots.first[..self.roots.len];
        roots
            .binary_search_by_key(&(accumulator, key), |root| (root.accumulator, root.key))
            .ok()
            .map(|index| &roots[index])
    }
    fn country_row(
        &self,
        context: native::Context,
        country: &native::Country,
    ) -> rows::CountryProductionModifiersRow {
        let nodes = &self.nodes.first[..self.nodes.len];
        let text = &self.text.first[..self.text.len];
        let trees = owned_vec(collect_vec(native::KEYS.iter().map(|key| {
            let root = self.root(country.accumulator, *key).unwrap();
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
    }
    fn line_row(&self, line: &native::Line) -> rows::ProductionLineModifiersRow {
        rows::ProductionLineModifiersRow {
            line: line.handle,
            country: line.country,
            variant: line.variant,
            organisation: line.organisation,
            kind: line.kind,
            active_factories: line.active_factories,
            resource_penalty_raw: line.resource_penalty,
            organisation_dirty: line.organisation_dirty,
            conversion: replay_data::rows::RetiredBool::default(),
            licensed: line.licensed,
            bonuses: line.bonuses,
            conversion_output: line.conversion_output,
            refit_output: line.refit_output,
            naval_efficiency: line.naval_efficiency,
            naval_output: line.naval_output,
            converting: replay_data::rows::Recorded::Value(line.conversion),
            factory_efficiencies: owned_vec(copy_vec(
                &self.efficiencies.first
                    [line.efficiencies[0]..line.efficiencies[0] + line.efficiencies[1]],
            )),
        }
    }
}

pub const MAX_PARTS: usize = 4;
const SPLIT_COUNTRIES: usize = 16;

pub struct Captured {
    first: Part,
    extra: [Option<Box<Part>>; MAX_PARTS - 1],
    prepared: usize,
    split: usize,
    starts: [u32; MAX_PARTS - 1],
    provided: Option<Result<native::Context, &'static str>>,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self::of(Part::with_capacities(capacity))
    }
    pub fn vacant() -> Self {
        Self::of(Part::vacant())
    }
    fn of(first: Part) -> Self {
        Self {
            first,
            extra: [const { None }; MAX_PARTS - 1],
            prepared: 1,
            split: 1,
            starts: [0; MAX_PARTS - 1],
            provided: None,
        }
    }
    pub fn reset(&mut self) {
        self.first.reset();
        self.unsplit();
    }
    fn unsplit(&mut self) {
        self.split = 1;
        self.prepared = 1;
        self.provided = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.first.allocated_bytes()
            + self
                .extra
                .iter()
                .flatten()
                .map(|part| size_of::<Part>() + part.allocated_bytes())
                .sum::<usize>()
    }
    pub fn release_daily(&mut self) {
        self.first = Part::vacant();
        self.extra = [const { None }; MAX_PARTS - 1];
        self.unsplit();
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        let has_required = self.first.countries.required > 0
            || self.first.lines.required > 0
            || self.first.efficiencies.required > 0
            || self.first.requests.required > 0
            || self.first.roots.required > 0
            || self.first.nodes.required > 0
            || self.first.text.required > 0
            || self.first.stack.required > 0
            || self.first.archetypes.required > 0;
        if has_required {
            self.first.grow(capacity);
        }
        for extra in &mut self.extra {
            if extra.is_none() && !crate::allocation::failed() {
                *extra = engine_allocation::boxed(Part::vacant()).ok();
            }
        }
    }
    pub fn fail_capture(&mut self, hour: i32, full: bool, reason: &'static str) {
        self.first.fail_capture(hour, full, reason);
        self.unsplit();
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        full: bool,
    ) -> [u64; N] {
        self.split = std::mem::replace(&mut self.prepared, 1);
        let provided = self.provided.take();
        let slots = self.slots(0);
        self.first
            .capture(memory, base, state, hour, full, slots, provided)
    }
    fn slots(&self, part: usize) -> std::ops::Range<u32> {
        let first = if part == 0 { 1 } else { self.starts[part - 1] };
        let end = if part + 1 < self.split {
            self.starts[part]
        } else {
            u32::MAX
        };
        first..end
    }
    pub fn prepare_parts(&mut self, memory: &impl Memory, base: usize, starts: &[u32]) -> usize {
        let allocated = 1 + self.extra.iter().take_while(|part| part.is_some()).count();
        let parts = (starts.len() + 1).min(allocated).min(MAX_PARTS);
        self.starts[..parts - 1].copy_from_slice(&starts[..parts - 1]);
        self.prepared = parts;
        self.split = parts;
        self.provided = (parts > 1).then(|| native::Context::read(memory, base));
        parts
    }
    #[allow(clippy::type_complexity)]
    pub fn take_part(
        &mut self,
        part: usize,
    ) -> (
        Box<Part>,
        std::ops::Range<u32>,
        Option<Result<native::Context, &'static str>>,
    ) {
        let slots = self.slots(part);
        let boxed = self.extra[part - 1]
            .take()
            .expect("prepared part allocated");
        (boxed, slots, self.provided)
    }
    pub fn restore_part(
        &mut self,
        part: usize,
        mut boxed: Box<Part>,
        completed: bool,
        hour: i32,
        full: bool,
    ) {
        if !completed {
            boxed.fail_capture(hour, full, "slow acquisition failed");
        }
        self.extra[part - 1] = Some(boxed);
    }
    fn parts(&self) -> impl Iterator<Item = &Part> + Clone {
        std::iter::once(&self.first).chain(
            self.extra[..self.split - 1]
                .iter()
                .map_while(|part| part.as_deref()),
        )
    }
    fn combined(&self) -> Result<[usize; N], &'static str> {
        let returned = self.parts().count();
        let mut lens = [0; N];
        for part in self.parts() {
            match part.outcome {
                Outcome::Rows { .. } => {
                    lens[0] += part.countries.len;
                    lens[1] += part.lines.len;
                }
                Outcome::Unavailable { reason } => return Err(reason),
                Outcome::Truncated { .. } => return Err("production modifier part truncated"),
            }
        }
        if returned != self.split {
            return Err("slow acquisition failed");
        }
        Ok(lens)
    }
    pub fn next_starts(
        &self,
        previous: ([u32; MAX_PARTS - 1], usize),
    ) -> ([u32; MAX_PARTS - 1], usize) {
        let Ok(lens) = self.combined() else {
            return previous;
        };
        if lens[0] < SPLIT_COUNTRIES {
            return ([0; MAX_PARTS - 1], 0);
        }
        let total = lens[0] + lens[1];
        let mut starts = [0; MAX_PARTS - 1];
        let (mut split, mut walked) = (0, 0);
        for part in self.parts() {
            let lines = &part.lines.first[..part.lines.len];
            let mut line = 0;
            for country in &part.countries.first[..part.countries.len] {
                if split < MAX_PARTS - 1 && walked * MAX_PARTS >= total * (split + 1) {
                    starts[split] = country.slot;
                    split += 1;
                }
                walked += 1;
                while lines
                    .get(line)
                    .is_some_and(|row| row.country == country.country)
                {
                    walked += 1;
                    line += 1;
                }
            }
        }
        (starts, split)
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        match self.combined() {
            Ok(lens) => lens.map(|len| Outcome::Rows { len }),
            Err(reason) => [Outcome::Unavailable { reason }; N],
        }
    }
    pub fn tables(&self) -> [TableView; N] {
        let lens = match self.combined() {
            Ok(lens) => lens,
            Err(reason) => return [unavailable(reason), unavailable(reason)],
        };
        let Some(context) = self
            .parts()
            .find(|part| part.requests.len != 0)
            .unwrap_or(&self.first)
            .context
        else {
            return [
                unavailable("production modifier context absent"),
                unavailable("production modifier context absent"),
            ];
        };
        if let Some(reason) = self.parts().find_map(Part::invalid) {
            return [unavailable(reason), unavailable(reason)];
        }
        let parts = self.parts();
        [
            table_iter::<tables::CountryProductionModifiers, _>(
                Outcome::Rows { len: lens[0] },
                parts.clone().flat_map(|part| {
                    part.countries.first[..part.countries.len]
                        .iter()
                        .map(move |country| (part, country))
                }),
                |(part, country)| part.country_row(context, country),
            ),
            table_iter::<tables::ProductionLineModifiers, _>(
                Outcome::Rows { len: lens[1] },
                parts.flat_map(|part| {
                    part.lines.first[..part.lines.len]
                        .iter()
                        .map(move |line| (part, line))
                }),
                |(part, line)| part.line_row(line),
            ),
        ]
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;

    pub fn part(countries: &[(u32, usize, usize)], context: native::Context) -> Part {
        let lines: usize = countries.iter().map(|&(_, _, lines)| lines).sum();
        let mut capacity = Capacities {
            lines,
            efficiencies: lines,
            nodes: 1,
            text: 1,
            ..Default::default()
        };
        capacity.raise_for_countries(countries.len());
        let mut part = Part::with_capacities(&capacity);
        part.context = Some(if countries.is_empty() {
            native::Context::default()
        } else {
            context
        });
        part.outcome = Outcome::Rows { len: lines };
        let mut roots = Vec::new();
        let mut line = 0;
        for (index, &(country, accumulator, count)) in countries.iter().enumerate() {
            part.countries.first[index] = native::Country {
                country,
                accumulator,
                power_raw: i64::from(country),
                slot: country,
            };
            for ordinal in 0..count {
                part.lines.first[line] = native::Line {
                    handle: (u64::from(country) << 8) | ordinal as u64,
                    country,
                    efficiencies: [line, 1],
                    ..Default::default()
                };
                part.efficiencies.first[line] = -i64::from(country);
                line += 1;
            }
            for key in native::KEYS {
                roots.push(tree::Root {
                    accumulator,
                    key,
                    nodes: [0, 1],
                    failure: None,
                });
            }
        }
        roots.sort_by_key(|root| (root.accumulator, root.key));
        part.roots.first[..roots.len()].copy_from_slice(&roots);
        part.roots.len = roots.len();
        part.requests.len = roots.len();
        part.countries.len = countries.len();
        part.lines.len = lines;
        part.efficiencies.len = lines;
        part.nodes.first[0] = tree::RawNode {
            children: 0,
            value: 7,
            name: [0, 1],
        };
        part.nodes.len = 1;
        part.text.first[0] = b'k';
        part.text.len = 1;
        part
    }

    pub fn failed(reason: &'static str) -> Part {
        let mut part = Part::vacant();
        part.outcome = Outcome::Unavailable { reason };
        part
    }

    impl Captured {
        pub fn whole(part: Part) -> Self {
            Self::of(part)
        }
        pub fn captured_as(&mut self, part: Part) {
            self.split = std::mem::replace(&mut self.prepared, 1);
            self.provided = None;
            self.first = part;
        }
        pub fn prepared_slots(&self) -> Vec<std::ops::Range<u32>> {
            (0..self.split).map(|part| self.slots(part)).collect()
        }
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
    fn countries_and_failed_arena_pressure_grow_off_hour() {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(6);
        let mut captured = Captured::with_capacities(&capacity);
        assert!(captured.first.countries.first.len() >= 5);
        assert!(captured.first.requests.first.len() >= 5 * native::KEYS.len());
        captured.first.efficiencies.required = captured.first.efficiencies.first.len() + 1;
        captured.first.archetypes.required = captured.first.archetypes.first.len() + 1;
        captured.grow(&mut capacity);
        assert!(captured.first.efficiencies.first.len() >= captured.first.efficiencies.required);
        assert!(captured.first.stack.first.len() >= captured.first.archetypes.required);
    }
    #[test]
    fn failed_root_preserves_independent_native_sources_and_negative_factory_entries() {
        use replay_data::rows::TableRows;
        let mut capacity = Capacities {
            lines: 1,
            efficiencies: 2,
            nodes: 1,
            text: 3,
            ..Default::default()
        };
        capacity.raise_for_countries(2);
        let mut captured = Captured::with_capacities(&capacity);
        captured.first.context = Some(native::Context::default());
        captured.first.outcome = Outcome::Rows { len: 1 };
        captured.first.countries.first[0] = native::Country {
            country: 9,
            accumulator: 123,
            power_raw: -20,
            slot: 1,
        };
        captured.first.countries.len = 1;
        captured.first.lines.first[0] = native::Line {
            handle: 22,
            country: 9,
            efficiencies: [0, 2],
            conversion: true,
            ..Default::default()
        };
        captured.first.lines.len = 1;
        captured.first.efficiencies.first[..2].copy_from_slice(&[-123, 456]);
        captured.first.efficiencies.len = 2;
        captured.first.nodes.first[0] = tree::RawNode {
            children: 0,
            value: -7,
            name: [0, 3],
        };
        captured.first.nodes.len = 1;
        captured.first.text.first[..3].copy_from_slice(&[0xff, 0, b'k']);
        captured.first.text.len = 3;
        for (index, key) in native::KEYS.into_iter().enumerate() {
            captured.first.roots.first[index] = tree::Root {
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
        captured.first.roots.len = native::KEYS.len();
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
        assert_eq!(lines[0].converting, replay_data::rows::Recorded::Value(true));
    }

    #[test]
    fn split_parts_publish_the_whole_capture_s_tables_in_slot_order() {
        let countries = [(1, 900), (2, 100), (3, 500), (4, 300)];
        let part_of = |countries: &[(u32, usize)]| {
            test_support::part(
                &countries
                    .iter()
                    .map(|&(id, at)| (id, at, 1))
                    .collect::<Vec<_>>(),
                native::Context::default(),
            )
        };
        let whole = Captured::of(part_of(&countries));
        let mut split = Captured::of(part_of(&countries[..2]));
        split.extra[0] = Some(Box::new(part_of(&countries[2..])));
        split.split = 2;
        assert_eq!(split.outcomes(), whole.outcomes());
        assert_eq!(split.tables(), whole.tables());
        assert!(matches!(whole.tables()[0], TableView::Complete(_)));
        split.extra[0]
            .as_mut()
            .unwrap()
            .fail_capture(8, false, "slow acquisition failed");
        let refused = Outcome::Unavailable {
            reason: "slow acquisition failed",
        };
        assert_eq!(split.outcomes(), [refused; N]);
        assert_eq!(split.next_starts(([7, 0, 0], 1)), ([7, 0, 0], 1));
        let many: Vec<_> = (1..=20).map(|id| (id, 1_000 + id as usize)).collect();
        let (starts, parts) = Captured::of(part_of(&many)).next_starts(([0; 3], 0));
        assert_eq!(parts, MAX_PARTS - 1);
        assert!(starts.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(starts[0] > 1 && starts[2] <= 20);
        assert_eq!(whole.next_starts(([7, 0, 0], 1)), ([0; 3], 0));
    }
}
