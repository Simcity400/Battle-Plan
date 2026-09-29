//! Hourly battle-plan operands over the shared ArmyOrders/group source census,
//! and the naval invasions' preparation operands over the same census.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::{
    Memory, cfront, cfront_section, corder_instance, corders_group, invasion_preparations,
    modifier_definition::DefinitionMemo, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{
        ArmyOrderPlanRow, FrontRow, InvasionPreparationRow, OrdersGroupPlanRow, TableId, tables,
    },
};
use std::mem::size_of;

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::ArmyOrderPlans,
    TableId::OrdersGroupPlans,
    TableId::Fronts,
    TableId::InvasionPreparations,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    plans: usize,
    groups: usize,
    fronts: usize,
    assigned: usize,
    links: usize,
    names: usize,
    root_orders: usize,
    sections: usize,
    pairs: usize,
    handles: usize,
    /// InvasionPreparations rows: the census's naval invasion orders.
    invasions: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            plans: 2048,
            groups: 2048,
            fronts: 1024,
            assigned: 16384,
            links: 8192,
            names: 16384,
            root_orders: 4096,
            sections: 4096,
            pairs: 65536,
            handles: 2048,
            invasions: 256,
        }
    }
}
pub struct Captured {
    plans: CaptureRows<corder_instance::PlanRow>,
    colors: CaptureRows<corders_group::PlanRow>,
    fronts: CaptureRows<cfront::Row>,
    invasions: CaptureRows<invasion_preparations::Row>,
    /// The invasion terms' modifier metadata, read once per pass.
    memo: DefinitionMemo,
    assigned: Arena<u64>,
    links: Arena<u32>,
    /// ArmyOrderPlans' order-name bytes and OrdersGroupPlans' group-name bytes,
    /// each in its own arena that the walker grows from `len`. Rows hold
    /// `(start, len)` spans that tile each arena's used prefix in walk order;
    /// bytes past `len` are stale and never published. Both share
    /// `Capacities::names`.
    names: Arena<u8>,
    group_names: Arena<u8>,
    root_orders: Arena<u32>,
    sections: Arena<cfront_section::Row>,
    pairs: Arena<u32>,
    handles: Vec<u64>,
    /// Sort scratch for the ArmyOrderPlans rows; grows to the row count.
    order: Vec<(u128, u32)>,
    required_handles: usize,
    ticks: [u64; N],
    supplied: bool,
}
impl Captured {
    /// Placeholder while the Orders job owns the buffers.
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            plans: 0,
            groups: 0,
            fronts: 0,
            assigned: 0,
            links: 0,
            names: 0,
            root_orders: 0,
            sections: 0,
            pairs: 0,
            handles: 0,
            invasions: 0,
        })
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            plans: CaptureRows::new(c.plans),
            colors: CaptureRows::new(c.groups),
            fronts: CaptureRows::new(c.fronts),
            invasions: CaptureRows::new(c.invasions),
            memo: DefinitionMemo::with_capacity(3),
            assigned: Arena::new(c.assigned),
            links: Arena::new(c.links),
            names: Arena::new(c.names),
            group_names: Arena::new(c.names),
            root_orders: Arena::new(c.root_orders),
            sections: Arena::new(c.sections),
            pairs: Arena::new(c.pairs),
            handles: vec![0; c.handles],
            order: Vec::new(),
            required_handles: 0,
            ticks: [0; N],
            supplied: false,
        }
    }
    pub fn reset(&mut self) {
        self.plans.reset();
        self.colors.reset();
        self.fronts.reset();
        self.invasions.reset();
        self.memo.clear();
        self.assigned.reset();
        self.links.reset();
        self.names.reset();
        self.group_names.reset();
        self.root_orders.reset();
        self.sections.reset();
        self.pairs.reset();
        self.required_handles = 0;
        self.ticks = [0; N];
        self.supplied = false;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.plans.allocated_bytes()
            + self.colors.allocated_bytes()
            + self.fronts.allocated_bytes()
            + self.invasions.allocated_bytes()
            + self.memo.allocated_bytes()
            + self.assigned.allocated_bytes()
            + self.links.allocated_bytes()
            + self.names.allocated_bytes()
            + self.group_names.allocated_bytes()
            + self.root_orders.allocated_bytes()
            + self.sections.allocated_bytes()
            + self.pairs.allocated_bytes()
            + self.handles.capacity() * size_of::<u64>()
            + self.order.capacity() * size_of::<(u128, u32)>()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        self.plans.grow(&mut c.plans);
        self.colors.grow(&mut c.groups);
        self.fronts.grow(&mut c.fronts);
        self.invasions.grow(&mut c.invasions);
        self.assigned.grow(&mut c.assigned);
        self.links.grow(&mut c.links);
        let mut group_names = c.names;
        self.names.grow(&mut c.names);
        self.group_names.grow(&mut group_names);
        c.names = c.names.max(group_names);
        self.root_orders.grow(&mut c.root_orders);
        self.sections.grow(&mut c.sections);
        self.pairs.grow(&mut c.pairs);
        grow_capacity(&mut c.handles, self.required_handles);
        grow_vec(&mut self.handles, c.handles);
    }
    /// Called on the Orders worker, which may grow payloads from owned census
    /// lengths and the original native headers. `visits` and `successors` are
    /// the census of this same job; nothing from another hour is passed.
    /// `game_state` is this pass's, for the invasions' countries and provinces.
    #[allow(clippy::too_many_arguments)]
    pub fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        roots: &[corders_group::Root],
        visits: &[corder_instance::Visit],
        successors: &corder_instance::SuccessorLists,
        root_source: Outcome,
        order_source: Outcome,
    ) -> u64 {
        self.supplied = true;
        let begin_all = qpc();
        let before = self.ticks.iter().sum::<u64>();
        let mut read_phase = crate::read_accounting::table(TABLES[1]);
        let begin = qpc();
        let color_capacity = if let Outcome::Rows { len } = root_source
            && len == roots.len()
            && len > self.colors.rows.len()
        {
            self.colors
                .rows
                .try_reserve(len - self.colors.rows.len())
                .map(|()| self.colors.rows.resize(len, Default::default()))
        } else {
            Ok(())
        };
        self.colors.outcome = match color_capacity {
            Ok(()) => {
                let report = corders_group::walk_plans(
                    memory,
                    base,
                    roots,
                    root_source,
                    visits,
                    order_source,
                    &mut self.colors.rows,
                    &mut self.root_orders.first,
                    &mut self.group_names.first,
                );
                self.root_orders.len = report.orders;
                self.root_orders.required = report.orders;
                self.group_names.len = report.names;
                self.group_names.required = report.names;
                report.outcome
            }
            Err(_) => Outcome::Unavailable {
                reason: "order color allocation failed",
            },
        };
        self.ticks[1] += qpc().saturating_sub(begin).max(0) as u64;
        read_phase.select(TABLES[0]);
        let begin = qpc();
        let report = corder_instance::walk_plans_ordered(
            memory,
            base,
            roots,
            visits,
            successors,
            order_source,
            &mut self.plans.rows,
            &mut self.assigned.first,
            &mut self.links.first,
            &mut self.names.first,
            &mut self.order,
        );
        self.plans.outcome = report.outcome;
        self.assigned.len = report.assigned;
        self.assigned.required = report.required_assigned;
        self.links.len = report.links;
        self.links.required = report.links;
        self.names.len = report.names;
        self.names.required = report.names;
        self.ticks[0] += qpc().saturating_sub(begin).max(0) as u64;
        read_phase.select(TABLES[2]);
        let begin = qpc();
        let front = cfront::walk(
            memory,
            base,
            &self.plans.rows,
            report.outcome,
            &mut self.handles,
            &mut self.fronts.rows,
            &mut self.sections.first,
            &mut self.pairs.first,
        );
        self.required_handles = self.required_handles.max(front.required_handles);
        self.fronts.outcome = front.outcome;
        self.sections.len = front.sections;
        self.sections.required = front.required_sections;
        self.pairs.len = front.pairs;
        self.pairs.required = front.required_pairs;
        self.ticks[2] += qpc().saturating_sub(begin).max(0) as u64;
        read_phase.select(TABLES[3]);
        let begin = qpc();
        self.invasions.outcome = invasion_preparations::walk(
            memory,
            base,
            game_state,
            visits,
            order_source,
            &self.memo,
            &mut self.invasions.rows,
        );
        self.ticks[3] += qpc().saturating_sub(begin).max(0) as u64;
        let total = qpc().saturating_sub(begin_all).max(0) as u64;
        self.ticks[0] += total.saturating_sub(self.ticks.iter().sum::<u64>() - before);
        // The order records this job's census gathered for these plans.
        let (_, census_ticks) = successors.plan_gathers();
        self.ticks[0] += census_ticks;
        total + census_ticks
    }
    /// Terminal worker/admission failure; no partial table may be published.
    pub(crate) fn fail(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.plans.outcome = outcome;
        self.colors.outcome = outcome;
        self.fronts.outcome = outcome;
        self.invasions.fail(outcome);
        self.assigned.len = 0;
        self.links.len = 0;
        self.names.len = 0;
        self.group_names.len = 0;
        self.root_orders.len = 0;
        self.sections.len = 0;
        self.pairs.len = 0;
        self.ticks = [0; N];
        self.supplied = true;
    }
    pub fn capture(&mut self) -> [u64; N] {
        if !self.supplied {
            let outcome = Outcome::Unavailable {
                reason: "plan source not supplied",
            };
            self.plans.outcome = outcome;
            self.colors.outcome = outcome;
            self.fronts.outcome = outcome;
            self.invasions.outcome = outcome;
        }
        self.ticks
    }
    fn name(&self, (start, len): (usize, usize)) -> Vec<u8> {
        self.names.first[start..start + len].to_vec()
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.plans.outcome,
            self.colors.outcome,
            self.fronts.outcome,
            self.invasions.outcome,
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::ArmyOrderPlans, _>(outcomes[0], &self.plans.rows, |r| {
                ArmyOrderPlanRow {
                    owner_group: r.owner_group,
                    instance: r.instance,
                    front: r.front,
                    front_valid: r.front_valid,
                    section: r.section,
                    segment_start: r.segment_start,
                    segment_end: r.segment_end,
                    blitz: r.blitz,
                    withdraw: r.withdraw,
                    fallback: r.fallback,
                    virtual_order: r.virtual_order,
                    has_child: r.has_child,
                    owner_override: r.owner_override,
                    predecessors: self.links.first
                        [r.predecessors_start..r.predecessors_start + r.predecessors_len]
                        .to_vec(),
                    successors: self.links.first
                        [r.successors_start..r.successors_start + r.successors_len]
                        .to_vec(),
                    assigned_divisions: self.assigned.first
                        [r.assigned_start..r.assigned_start + r.assigned_len]
                        .to_vec(),
                    name_prefix: self.name(r.names[0]),
                    name: self.name(r.names[1]),
                    name_first: self.name(r.names[2]),
                    name_second: self.name(r.names[3]),
                    name_unique: self.name(r.names[4]),
                    name_postfix: self.name(r.names[5]),
                }
            }),
            table::<tables::OrdersGroupPlans, _>(outcomes[1], &self.colors.rows, |r| {
                OrdersGroupPlanRow {
                    group: r.group,
                    red: r.red,
                    green: r.green,
                    blue: r.blue,
                    alpha: r.alpha,
                    primary_orders: self.root_orders.first
                        [r.primary_start..r.primary_start + r.primary_len]
                        .to_vec(),
                    fallback_orders: self.root_orders.first
                        [r.fallback_start..r.fallback_start + r.fallback_len]
                        .to_vec(),
                    name: self.group_names.first[r.name_start..r.name_start + r.name_len]
                        .to_vec(),
                }
            }),
            table::<tables::Fronts, _>(outcomes[2], &self.fronts.rows, |r| {
                let mut row = FrontRow {
                    front: r.front,
                    ..Default::default()
                };
                for section in
                    &self.sections.first[r.section_start..r.section_start + r.section_len]
                {
                    row.section_ids.push(section.id);
                    row.province_pairs.extend_from_slice(
                        &self.pairs.first
                            [section.pair_start..section.pair_start + section.pair_len],
                    );
                    row.section_ends.push(row.province_pairs.len() as u32);
                }
                row
            }),
            table::<tables::InvasionPreparations, _>(outcomes[3], &self.invasions.rows, |r| {
                InvasionPreparationRow {
                    owner_group: r.owner_group,
                    instance: r.instance,
                    progress: r.progress,
                    target: r.target,
                    country_term: r.country_term,
                    relation_term: r.relation_term,
                    relation_bonus: r.relation_bonus,
                    leader_term: r.leader_term,
                    leader_applies: r.leader_applies,
                    days_term: r.days_term,
                }
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{creference_registry as registry, reference_bucket as bucket};
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Image(BTreeMap<usize, u8>);
    impl Image {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0
                .extend(bytes.iter().enumerate().map(|(i, v)| (at + i, *v)));
        }
        fn u64(&mut self, at: usize, v: u64) {
            self.put(at, &v.to_le_bytes());
        }
        fn u32(&mut self, at: usize, v: u32) {
            self.put(at, &v.to_le_bytes());
        }
    }
    impl Memory for Image {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            for (i, v) in out.iter_mut().enumerate() {
                let Some(b) = self.0.get(&(at + i)) else {
                    return false;
                };
                *v = *b;
            }
            true
        }
    }
    const GROUP: usize = 0x3000;
    const ORDER: usize = 0x4000;
    fn image() -> Image {
        let mut m = Image::default();
        m.u64(registry::INVALID_REFERENCE_RVA, 0);
        m.u64(registry::LOW_ROOTS_RVA + 53 * 8, 0x1000);
        m.put(0x1000, &[0; 32]);
        m.u64(0x1008, 0x2000);
        m.u32(0x1010, 1);
        m.u32(0x1014, 7);
        m.put(0x1018, &[5]);
        m.put(0x2000, &[0; 14 * bucket::STRIDE]);
        m.put(0x2000 + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        m.put(0x2000 + bucket::DISTANCE, &[1]);
        m.u64(0x2000 + bucket::TYPE, (7 << 32) | 53);
        m.u64(0x2000 + bucket::OBJECT, GROUP as u64);
        m.put(GROUP, &[0; 0x240]);
        m.u64(GROUP, corders_group::VTABLE_RVA as u64);
        m.u64(GROUP + 8, (7 << 32) | 53);
        m.u32(GROUP + corders_group::COUNTRY, 1);
        // An empty inline name: capacity 15, length 0.
        m.u64(GROUP + corders_group::NAME + 0x18, 15);
        let root = GROUP + corders_group::PRIMARY_ORDER_DATA;
        m.u64(root, 0x3800);
        m.u32(root + 8, 1);
        m.u32(root + 12, 1);
        m.u64(0x3800, ORDER as u64);
        m.put(ORDER, &[0; 0x400]);
        m.u64(ORDER, corder_instance::VTABLE_RVA as u64);
        m.u64(ORDER + corder_instance::OWNER, GROUP as u64);
        m.u32(ORDER + corder_instance::ID, 9);
        m
    }
    fn pass(m: &Image, c: &mut Captured) {
        let mut groups = [corders_group::Root::default(); 4];
        let mut visits = vec![corder_instance::Visit::default(); 8];
        let mut successors = corder_instance::SuccessorLists::default();
        let r = corder_instance::walk_listing_successors(
            m,
            0,
            &mut vec![corder_instance::Row::default(); 4],
            &mut vec![0; 4],
            &mut groups,
            &mut visits,
            &mut successors,
        );
        assert!(matches!(r.outcome, Outcome::Rows { len } if len != 0));
        c.visit(
            m,
            0,
            0,
            &groups[..r.root_outcome.len()],
            &visits,
            &successors,
            r.root_outcome,
            r.outcome,
        );
    }
    #[test]
    fn primary_plan_styles_and_colors_spool_exact_bytes() {
        let mut m = image();
        let mut c = Captured::with_capacities(&Capacities::default());
        m.put(ORDER + corder_instance::MOVE_SPECIAL, &[255]);
        m.u32(GROUP + corders_group::COLOUR, 0x7fc01234);
        pass(&m, &mut c);
        c.capture();
        assert!(
            matches!(&c.tables()[0], TableView::Complete(replay::rows::TableRows::ArmyOrderPlans(rows)) if rows[0].blitz == 255)
        );
        assert!(
            matches!(&c.tables()[1], TableView::Complete(replay::rows::TableRows::OrdersGroupPlans(rows)) if rows[0].red == 0x7fc01234)
        );
        assert_eq!(c.outcomes()[2], Outcome::Rows { len: 0 });
    }
    #[test]
    fn empty_payload_storage_grows_and_terminal_failure_hides_all_tables() {
        let m = image();
        let mut c = Captured::vacant();
        pass(&m, &mut c);
        assert_eq!(
            c.outcomes(),
            [
                Outcome::Rows { len: 1 },
                Outcome::Rows { len: 1 },
                Outcome::Rows { len: 0 },
                Outcome::Rows { len: 0 }
            ]
        );
        let bytes = c.allocated_bytes();
        assert!(bytes > 0);
        c.fail("worker failed");
        c.capture();
        assert_eq!(
            c.outcomes(),
            [Outcome::Unavailable {
                reason: "worker failed"
            }; N]
        );
        assert_eq!(c.allocated_bytes(), bytes);
        c.reset();
        pass(&m, &mut c);
        assert_eq!(c.outcomes()[0], Outcome::Rows { len: 1 });
    }
    #[test]
    fn growth_preserves_primary_arenas_and_missing_source_fails() {
        let m = image();
        let mut capacity = Capacities::default();
        let mut c = Captured::with_capacities(&capacity);
        pass(&m, &mut c);
        c.capture();
        c.assigned.required = capacity.assigned + 1;
        c.pairs.required = capacity.pairs + 1;
        c.required_handles = capacity.handles + 1;
        c.grow(&mut capacity);
        assert!(capacity.assigned > 16384 && capacity.pairs > 65536 && capacity.handles > 2048);
        assert_eq!(c.assigned.first.len(), capacity.assigned);
        assert_eq!(c.pairs.first.len(), capacity.pairs);
        assert_eq!(c.handles.len(), capacity.handles);
        c.reset();
        c.capture();
        assert!(
            c.outcomes()
                .iter()
                .all(|v| matches!(v, Outcome::Unavailable { .. }))
        );
    }

    /// A naval invasion whose inputs cannot be read withholds only
    /// InvasionPreparations; the plans of the same census are published.
    #[test]
    fn an_unreadable_invasion_withholds_only_its_own_table() {
        let mut m = image();
        m.u32(
            ORDER + corder_instance::TYPE,
            corder_instance::TYPE_INVASION as u32,
        );
        // An invasion is a named arrow: its name strings are read too.
        for offset in corder_instance::NAME_STRINGS {
            string(&mut m, ORDER + offset, b"", None);
        }
        let mut c = Captured::with_capacities(&Capacities::default());
        pass(&m, &mut c);
        c.capture();
        let outcomes = c.outcomes();
        assert_eq!(
            &outcomes[..3],
            &[
                Outcome::Rows { len: 1 },
                Outcome::Rows { len: 1 },
                Outcome::Rows { len: 0 }
            ]
        );
        assert!(matches!(outcomes[3], Outcome::Unavailable { .. }));
        assert!(matches!(c.tables()[3], TableView::Unavailable { .. }));
    }

    /// A std::string header at `at`: inline when `heap` is `None`.
    fn string(m: &mut Image, at: usize, text: &[u8], heap: Option<usize>) {
        match heap {
            Some(data) => {
                m.put(data, text);
                m.put(data + text.len(), &[0]);
                m.u64(at, data as u64);
                m.u64(at + 0x18, (text.len() as u64).max(31));
            }
            None => {
                m.put(at, text);
                m.put(at + text.len(), &[0]);
                m.u64(at + 0x18, 15);
            }
        }
        m.u64(at + 0x10, text.len() as u64);
    }

    /// Every published list and string is exactly its own span of the shared
    /// arenas: consecutive, non-overlapping, ending at the arena's used length.
    /// A second hour reuses the grown arenas with shorter text, and no stale
    /// byte past a span leaks into a row.
    #[test]
    fn arena_spans_are_exact_across_rows_growth_and_reuse() {
        const SECOND: usize = 0x4800;
        let build = |group_name: &[u8], unique: &[u8]| {
            let mut m = image();
            // Two roots; the first succeeds into the second.
            let root = GROUP + corders_group::PRIMARY_ORDER_DATA;
            m.u64(root, 0x3800);
            m.u32(root + 8, 2);
            m.u32(root + 12, 2);
            m.u64(0x3808, SECOND as u64);
            m.put(SECOND, &[0; 0x400]);
            m.u64(SECOND, corder_instance::VTABLE_RVA as u64);
            m.u64(SECOND + corder_instance::OWNER, GROUP as u64);
            m.u32(SECOND + corder_instance::ID, 10);
            m.u64(ORDER + corder_instance::SUCCESSOR_DATA, 0x3900);
            m.u32(ORDER + corder_instance::SUCCESSOR_DATA + 8, 1);
            m.u32(ORDER + corder_instance::SUCCESSOR_DATA + 12, 1);
            m.u64(0x3900, SECOND as u64);
            m.u64(SECOND + corder_instance::PREDECESSOR_DATA, 0x3a00);
            m.u32(SECOND + corder_instance::PREDECESSOR_DATA + 8, 1);
            m.u32(SECOND + corder_instance::PREDECESSOR_DATA + 12, 1);
            m.u64(0x3a00, ORDER as u64);
            // Both are named moves: inline keys and a heap unique text.
            for (order, heap) in [(ORDER, 0x5000), (SECOND, 0x5100)] {
                m.u32(order + corder_instance::TYPE, corder_instance::TYPE_MOVE as u32);
                for (index, offset) in corder_instance::NAME_STRINGS.into_iter().enumerate() {
                    let at = order + offset;
                    match index {
                        0 => string(&mut m, at, b"prefix_key", None),
                        4 => string(&mut m, at, unique, Some(heap)),
                        _ => string(&mut m, at, b"", None),
                    }
                }
            }
            string(&mut m, GROUP + corders_group::NAME, group_name, Some(0x5200));
            m
        };
        let rows = |c: &Captured| {
            let [
                TableView::Complete(replay::rows::TableRows::ArmyOrderPlans(plans)),
                TableView::Complete(replay::rows::TableRows::OrdersGroupPlans(groups)),
                _,
                _,
            ] = c.tables()
            else {
                panic!("plans unavailable: {:?}", c.outcomes());
            };
            (plans, groups)
        };
        let mut c = Captured::vacant();
        let long = b"Heeresgruppe Mitte, Second Panzer";
        pass(&build(long, b"Operation Barbarossa"), &mut c);
        c.capture();
        let (plans, groups) = rows(&c);
        assert_eq!(plans.len(), 2);
        let (first, second) = (&plans[0], &plans[1]);
        assert_eq!((first.instance, second.instance), (9, 10));
        assert_eq!((&first.successors[..], &first.predecessors[..]), (&[10][..], &[][..]));
        assert_eq!((&second.successors[..], &second.predecessors[..]), (&[][..], &[9][..]));
        for plan in &plans {
            assert_eq!(plan.name_prefix, b"prefix_key");
            assert_eq!(plan.name_unique, b"Operation Barbarossa");
            assert!(plan.name.is_empty() && plan.name_postfix.is_empty());
        }
        assert_eq!(groups[0].name, long);
        assert_eq!(groups[0].primary_orders, [9, 10]);
        // The nonempty spans tile the used prefix of each arena: rows are
        // sorted by key after the walk, so order the spans by start first.
        let mut spans: Vec<_> = c.plans.rows[..2]
            .iter()
            .flat_map(|plan| plan.names)
            .filter(|&(_, len)| len != 0)
            .collect();
        spans.sort_unstable();
        let end = spans.iter().fold(0, |end, &(start, len)| {
            assert_eq!(start, end);
            start + len
        });
        assert_eq!(end, c.names.len);
        assert_eq!(c.links.len, 2);
        assert_eq!(c.group_names.len, long.len());

        // The next hour reuses the grown arenas: shorter text must not carry
        // the previous hour's trailing bytes.
        let mut capacities = Capacities {
            plans: 0,
            groups: 0,
            fronts: 0,
            assigned: 0,
            links: 0,
            names: 0,
            root_orders: 0,
            sections: 0,
            pairs: 0,
            handles: 0,
            invasions: 0,
        };
        c.grow(&mut capacities);
        c.reset();
        pass(&build(b"Army 1", b"Op"), &mut c);
        c.capture();
        let (plans, groups) = rows(&c);
        assert!(plans.iter().all(|plan| plan.name_unique == b"Op"));
        assert_eq!(groups[0].name, b"Army 1");
        assert_eq!(c.names.len, 2 * (b"prefix_key".len() + 2));
    }

    /// No writer of the order or group names marks a change
    /// (`msvc_string::assign_bytes_candidate@140129790` rewrites a heap
    /// buffer in place when the new text fits), so every hour reads the
    /// stored bytes again. A capture reused across hours records each state
    /// byte for byte as a fresh capture of the same memory does: an
    /// unchanged hour, a same-length rename in the same buffer, a length
    /// change in that buffer, and inline to heap and back.
    #[test]
    fn reused_captures_record_every_rename_as_a_fresh_capture_does() {
        const UNIQUE: usize = 0x5000;
        const GROUP_NAME: usize = 0x5100;
        let unique_at = ORDER + corder_instance::NAME_STRINGS[4];
        let group_at = GROUP + corders_group::NAME;
        // One image carried across the hours, so a rename leaves the old
        // tail bytes behind its terminator exactly as an in-place assign does.
        let mut m = image();
        m.u32(ORDER + corder_instance::TYPE, corder_instance::TYPE_MOVE as u32);
        for (index, offset) in corder_instance::NAME_STRINGS.into_iter().enumerate() {
            let text: &[u8] = if index == 0 { b"o_operation" } else { b"" };
            string(&mut m, ORDER + offset, text, None);
        }
        m.put(UNIQUE, &[0xa5; 64]);
        m.put(GROUP_NAME, &[0xa5; 64]);
        let names = |c: &Captured| {
            let [
                TableView::Complete(replay::rows::TableRows::ArmyOrderPlans(plans)),
                TableView::Complete(replay::rows::TableRows::OrdersGroupPlans(groups)),
                _,
                _,
            ] = c.tables()
            else {
                panic!("plans unavailable: {:?}", c.outcomes());
            };
            (plans, groups)
        };
        let hours: [(&[u8], bool, &[u8], bool); 6] = [
            (b"Operation Barbarossa", true, b"Heeresgruppe Mitte", true),
            // Unchanged.
            (b"Operation Barbarossa", true, b"Heeresgruppe Mitte", true),
            // Same length, same buffer, same capacity: only the bytes differ.
            (b"Operation Zitadelle!", true, b"Heeresgruppe Sued!", true),
            // Shorter and longer in the same buffer.
            (b"Operation Blau", true, b"Heeresgruppe Nord, 16. Armee", true),
            // Heap to inline.
            (b"Op Uranus", false, b"Army 1", false),
            // Inline to heap.
            (b"Operation Bagration", true, b"1st Belorussian Front", true),
        ];
        let mut reused = Captured::vacant();
        let mut capacities = Capacities {
            plans: 0,
            groups: 0,
            fronts: 0,
            assigned: 0,
            links: 0,
            names: 0,
            root_orders: 0,
            sections: 0,
            pairs: 0,
            handles: 0,
            invasions: 0,
        };
        for (hour, (unique, unique_heap, group, group_heap)) in hours.into_iter().enumerate() {
            string(&mut m, unique_at, unique, unique_heap.then_some(UNIQUE));
            string(&mut m, group_at, group, group_heap.then_some(GROUP_NAME));
            // The heap buffers keep one capacity, as an in-place assign does.
            if unique_heap {
                m.u64(unique_at + 0x18, 63);
            }
            if group_heap {
                m.u64(group_at + 0x18, 63);
            }
            reused.reset();
            pass(&m, &mut reused);
            reused.capture();
            let mut fresh = Captured::with_capacities(&Capacities::default());
            pass(&m, &mut fresh);
            fresh.capture();
            let (plans, groups) = names(&reused);
            assert_eq!((&plans, &groups), (&names(&fresh).0, &names(&fresh).1), "hour {hour}");
            assert_eq!(plans[0].name_unique, unique, "hour {hour}");
            assert_eq!(plans[0].name_prefix, b"o_operation");
            assert_eq!(groups[0].name, group, "hour {hour}");
            reused.grow(&mut capacities);
        }
    }
}

/// Ignored capture-cost bench; see `bench_world` for the assumed scale. Each
/// case times the Orders job's pass through the plain slice and through the
/// production guarded source: the ArmyOrders census, which gathers the plans'
/// order records, then the plan visit. Each table's time is its own ticks,
/// the census's plan gathers included in ArmyOrderPlans; `order-census` is
/// the census without them. It also counts one pass's source requests per
/// table, charged as the Orders job charges them.
#[cfg(test)]
mod l3b_bench {
    use super::*;
    use crate::extended::bench_world::{self, Shape, World};

    /// The census and plan visit of one Orders job pass, with its reusable
    /// scratch. Returns the census's own ticks, without its plan gathers,
    /// and the gathers' ticks.
    struct Pass {
        captured: Captured,
        rows: Vec<corder_instance::Row>,
        geometry: Vec<u32>,
        visits: Vec<corder_instance::Visit>,
        successors: corder_instance::SuccessorLists,
    }

    impl Pass {
        fn new() -> Self {
            Self {
                captured: Captured::with_capacities(&Capacities::default()),
                rows: Vec::new(),
                geometry: Vec::new(),
                visits: Vec::new(),
                successors: corder_instance::SuccessorLists::default(),
            }
        }

        fn run(
            &mut self,
            memory: &impl Memory,
            base: usize,
            roots: &[corders_group::Root],
            root_outcome: Outcome,
        ) -> (u64, u64) {
            self.captured.reset();
            let mut reads = crate::read_accounting::table(TableId::ArmyOrders);
            let begin = qpc();
            let census = corder_instance::walk_listing_successors_of(
                memory,
                base,
                &mut self.rows,
                &mut self.geometry,
                roots,
                root_outcome,
                &mut self.visits,
                &mut self.successors,
            );
            let (plan_reads, plan_ticks) = self.successors.plan_gathers();
            let census_ticks = (qpc().saturating_sub(begin).max(0) as u64).saturating_sub(plan_ticks);
            reads.select(TableId::ArmyOrderPlans);
            plan_reads.add_current();
            drop(reads);
            self.captured.visit(
                memory,
                base,
                0,
                roots,
                &self.visits,
                &self.successors,
                census.root_outcome,
                census.outcome,
            );
            (census_ticks, plan_ticks)
        }
    }

    fn micros(ticks: u64) -> f64 {
        let mut frequency = 0;
        unsafe {
            windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency);
        }
        ticks as f64 * 1e6 / frequency as f64
    }

    fn median(mut ticks: Vec<u64>) -> f64 {
        ticks.sort_unstable();
        micros(ticks[ticks.len() / 2])
    }

    /// Median and p95 of the whole pass, each table's median, and the
    /// medians of the census without its plan gathers and of the gathers,
    /// in µs.
    fn time_passes(
        memory: &impl Memory,
        world: &World,
        roots: &[corders_group::Root],
        root_outcome: Outcome,
    ) -> (f64, f64, Vec<f64>, f64, f64) {
        let mut pass = Pass::new();
        let mut tables = Vec::new();
        let mut censuses = Vec::new();
        let mut gathers = Vec::new();
        let mut run = || {
            let (census, gathered) = pass.run(memory, world.base, roots, root_outcome);
            censuses.push(census);
            gathers.push(gathered);
            tables.push(pass.captured.ticks);
        };
        // The first pass grows every arena; later passes reuse them.
        run();
        let (median_pass, p95) = bench_world::time(50, &mut run);
        let per_table = (0..N)
            .map(|table| median(tables.iter().map(|t| t[table]).collect()))
            .collect();
        pass.captured.capture();
        assert_eq!(
            &pass.captured.outcomes()[..2],
            &[
                Outcome::Rows {
                    len: world.orders as usize
                },
                Outcome::Rows {
                    len: world.groups as usize
                }
            ]
        );
        (
            median_pass,
            p95,
            per_table,
            median(censuses),
            median(gathers),
        )
    }

    #[test]
    #[ignore = "timing bench"]
    fn l3b_bench_hourly_plan_links_and_names() {
        for countries in [1_500, 5_000] {
            for named in [true, false] {
                let world = bench_world::build(
                    countries,
                    Shape {
                        named,
                        theatre_links: true,
                    },
                );
                let slice = world.slice();
                let mut roots = vec![corders_group::Root::default(); world.groups as usize];
                let root_outcome = corders_group::walk_roots(&slice, world.base, &mut roots);
                // One warm pass's source requests, per table.
                let mut pass = Pass::new();
                let mut counts = crate::read_accounting::ZERO;
                for _ in 0..2 {
                    ((), counts) = world.counted(|memory| {
                        pass.run(memory, world.base, &roots, root_outcome);
                    });
                }
                pass.captured.capture();
                let work: Vec<String> = TABLES
                    .iter()
                    .chain([&TableId::ArmyOrders])
                    .map(|table| bench_world::work(counts[table.index()]))
                    .collect();
                println!(
                    "plans-work countries={countries} named={named}: ArmyOrderPlans {}; OrdersGroupPlans {}; Fronts {}; InvasionPreparations {}; ArmyOrders census {}",
                    work[0], work[1], work[2], work[3], work[4]
                );
                for source in ["slice", "live"] {
                    let (median, p95, per_table, census, gathers) = if source == "slice" {
                        time_passes(&slice, &world, &roots, root_outcome)
                    } else {
                        time_passes(&world.live(), &world, &roots, root_outcome)
                    };
                    println!(
                        "plans countries={countries} groups={} orders={} named={named} source={source}: pass {median:.1} us (p95 {p95:.1}); ArmyOrderPlans {:.1} us, OrdersGroupPlans {:.1} us, Fronts {:.1} us; order-census {census:.1} us, census-gathers {gathers:.1} us; links {}, order-name bytes {}, group-name bytes {}",
                        world.groups,
                        world.orders,
                        per_table[0],
                        per_table[1],
                        per_table[2],
                        pass.captured.links.len,
                        pass.captured.names.len,
                        pass.captured.group_names.len
                    );
                }
                // The census alone, gathering the plan records, over the
                // callback's registry census, as the Orders job runs it.
                let live = world.live();
                let (mut rows, mut geometry, mut visits) = (Vec::new(), Vec::new(), Vec::new());
                let mut successors = corder_instance::SuccessorLists::default();
                let root_outcome = corders_group::walk_roots(&live, world.base, &mut roots);
                let (walked, walked_p95) = bench_world::time(50, || {
                    corder_instance::walk_listing_successors_of(
                        &live,
                        world.base,
                        &mut rows,
                        &mut geometry,
                        &roots,
                        root_outcome,
                        &mut visits,
                        &mut successors,
                    );
                });
                println!(
                    "census countries={countries} named={named} source=live: census of walked roots {walked:.1} us (p95 {walked_p95:.1})"
                );
            }
        }
    }
}
