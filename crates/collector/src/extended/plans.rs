//! Hourly battle-plan operands over the shared ArmyOrders/group source census.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::{Memory, cfront, cfront_section, corder_instance, corders_group, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{ArmyOrderPlanRow, FrontRow, OrdersGroupColorRow, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::ArmyOrderPlans,
    TableId::OrdersGroupColors,
    TableId::Fronts,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    plans: usize,
    groups: usize,
    fronts: usize,
    assigned: usize,
    sections: usize,
    pairs: usize,
    handles: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            plans: 2048,
            groups: 2048,
            fronts: 1024,
            assigned: 16384,
            sections: 4096,
            pairs: 65536,
            handles: 2048,
        }
    }
}
pub struct Captured {
    plans: CaptureRows<corder_instance::PlanRow>,
    colors: CaptureRows<corders_group::ColorRow>,
    fronts: CaptureRows<cfront::Row>,
    assigned: Arena<u64>,
    sections: Arena<cfront_section::Row>,
    pairs: Arena<u32>,
    handles: Vec<u64>,
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
            sections: 0,
            pairs: 0,
            handles: 0,
        })
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            plans: CaptureRows::new(c.plans),
            colors: CaptureRows::new(c.groups),
            fronts: CaptureRows::new(c.fronts),
            assigned: Arena::new(c.assigned),
            sections: Arena::new(c.sections),
            pairs: Arena::new(c.pairs),
            handles: vec![0; c.handles],
            required_handles: 0,
            ticks: [0; N],
            supplied: false,
        }
    }
    pub fn reset(&mut self) {
        self.plans.reset();
        self.colors.reset();
        self.fronts.reset();
        self.assigned.reset();
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
            + self.assigned.allocated_bytes()
            + self.sections.allocated_bytes()
            + self.pairs.allocated_bytes()
            + self.handles.capacity() * size_of::<u64>()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        self.plans.grow(&mut c.plans);
        self.colors.grow(&mut c.groups);
        self.fronts.grow(&mut c.fronts);
        self.assigned.grow(&mut c.assigned);
        self.sections.grow(&mut c.sections);
        self.pairs.grow(&mut c.pairs);
        grow_capacity(&mut c.handles, self.required_handles);
        grow_vec(&mut self.handles, c.handles);
    }
    /// Called on the Orders worker, which may grow payloads from owned census
    /// lengths and the original native headers.
    pub fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        roots: &[corders_group::Root],
        visits: &[corder_instance::Visit],
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
                corders_group::walk_colors(memory, base, roots, root_source, &mut self.colors.rows)
            }
            Err(_) => Outcome::Unavailable {
                reason: "order color allocation failed",
            },
        };
        self.ticks[1] += qpc().saturating_sub(begin).max(0) as u64;
        read_phase.select(TABLES[0]);
        let begin = qpc();
        let report = corder_instance::walk_plans(
            memory,
            base,
            roots,
            visits,
            order_source,
            &mut self.plans.rows,
            &mut self.assigned.first,
        );
        self.plans.outcome = report.outcome;
        self.assigned.len = report.assigned;
        self.assigned.required = report.required_assigned;
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
        let total = qpc().saturating_sub(begin_all).max(0) as u64;
        self.ticks[0] += total.saturating_sub(self.ticks.iter().sum::<u64>() - before);
        total
    }
    /// Terminal worker/admission failure; no partial table may be published.
    pub(crate) fn fail(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.plans.outcome = outcome;
        self.colors.outcome = outcome;
        self.fronts.outcome = outcome;
        self.assigned.len = 0;
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
        }
        self.ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.plans.outcome, self.colors.outcome, self.fronts.outcome]
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
                    assigned_divisions: self.assigned.first
                        [r.assigned_start..r.assigned_start + r.assigned_len]
                        .to_vec(),
                }
            }),
            table::<tables::OrdersGroupColors, _>(outcomes[1], &self.colors.rows, |r| {
                OrdersGroupColorRow {
                    group: r.group,
                    red: r.red,
                    green: r.green,
                    blue: r.blue,
                    alpha: r.alpha,
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
        let r = corder_instance::walk(
            m,
            0,
            &mut vec![corder_instance::Row::default(); 4],
            &mut vec![0; 4],
            &mut groups,
            &mut visits,
        );
        assert_eq!(r.outcome, Outcome::Rows { len: 1 });
        c.visit(
            m,
            0,
            &groups[..r.root_outcome.len()],
            &visits,
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
            matches!(&c.tables()[1], TableView::Complete(replay::rows::TableRows::OrdersGroupColors(rows)) if rows[0].red == 0x7fc01234)
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
}
