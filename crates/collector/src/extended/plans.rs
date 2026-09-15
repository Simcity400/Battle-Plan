//! Hourly battle-plan operands over the shared ArmyOrders/group source census.
use crate::{
    sampling::{full_match, qpc},
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
    matched: [bool; N],
    ticks: [u64; N],
    supplied: [bool; 2],
}
impl Captured {
    pub fn with_capacities(c: &Capacities, successful: &[u16]) -> Self {
        let order_success = successful[0].min(successful[2]);
        Self {
            plans: CaptureRows::new(c.plans, order_success),
            colors: CaptureRows::new(c.groups, successful[1]),
            fronts: CaptureRows::new(c.fronts, successful[2]),
            assigned: Arena::new(c.assigned, order_success),
            sections: Arena::new(c.sections, successful[2]),
            pairs: Arena::new(c.pairs, successful[2]),
            handles: vec![0; c.handles],
            required_handles: 0,
            matched: [false; N],
            ticks: [0; N],
            supplied: [false; 2],
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
        self.matched = [false; N];
        self.ticks = [0; N];
        self.supplied = [false; 2];
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
    pub fn grow(&mut self, c: &mut Capacities, successful: &[u16]) {
        let order_success = successful[0].min(successful[2]);
        self.plans.grow(&mut c.plans, order_success);
        self.colors.grow(&mut c.groups, successful[1]);
        self.fronts.grow(&mut c.fronts, successful[2]);
        self.assigned.grow(&mut c.assigned, order_success);
        self.sections.grow(&mut c.sections, successful[2]);
        self.pairs.grow(&mut c.pairs, successful[2]);
        grow_capacity(&mut c.handles, self.required_handles);
        grow_vec(&mut self.handles, c.handles);
    }
    /// Called inside each independently acquired native order-census pass.
    #[allow(clippy::too_many_arguments)]
    pub fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        roots: &[corders_group::Root],
        visits: &[corder_instance::Visit],
        root_source: Outcome,
        order_source: Outcome,
        second: bool,
        needed: &[bool; N],
    ) -> u64 {
        self.supplied[usize::from(second)] = true;
        let begin_all = qpc();
        let before = self.ticks.iter().sum::<u64>();
        let begin = qpc();
        if !second || needed[1] {
            let outcome = corders_group::walk_colors(
                memory,
                base,
                roots,
                root_source,
                if second {
                    &mut self.colors.second
                } else {
                    &mut self.colors.rows
                },
            );
            if second {
                self.matched[1] = self.colors.compare(outcome);
            } else {
                self.colors.outcome = outcome;
                self.colors.checked = needed[1];
            }
        }
        self.ticks[1] += qpc().saturating_sub(begin).max(0) as u64;
        let begin = qpc();
        if !second || needed[0] || needed[2] {
            let report = corder_instance::walk_plans(
                memory,
                base,
                roots,
                visits,
                order_source,
                if second {
                    &mut self.plans.second
                } else {
                    &mut self.plans.rows
                },
                if second {
                    &mut self.assigned.second
                } else {
                    &mut self.assigned.first
                },
            );
            if second {
                let arena_match = self
                    .assigned
                    .compare(report.assigned, report.required_assigned);
                if needed[0] {
                    self.plans.compare(report.outcome);
                    self.plans.mismatches += usize::from(!arena_match);
                    self.matched[0] =
                        full_match(self.plans.outcome, report.outcome, self.plans.mismatches);
                }
            } else {
                self.plans.outcome = report.outcome;
                self.plans.checked = needed[0];
                self.assigned.len = report.assigned;
                self.assigned.required = report.required_assigned;
            }
            self.ticks[0] += qpc().saturating_sub(begin).max(0) as u64;
            if !second || needed[2] {
                let begin = qpc();
                let front = cfront::walk(
                    memory,
                    base,
                    if second {
                        &self.plans.second
                    } else {
                        &self.plans.rows
                    },
                    report.outcome,
                    &mut self.handles,
                    if second {
                        &mut self.fronts.second
                    } else {
                        &mut self.fronts.rows
                    },
                    if second {
                        &mut self.sections.second
                    } else {
                        &mut self.sections.first
                    },
                    if second {
                        &mut self.pairs.second
                    } else {
                        &mut self.pairs.first
                    },
                );
                self.required_handles = self.required_handles.max(front.required_handles);
                if second {
                    self.fronts.compare(front.outcome);
                    self.fronts.mismatches += usize::from(
                        !self
                            .sections
                            .compare(front.sections, front.required_sections),
                    );
                    self.fronts.mismatches +=
                        usize::from(!self.pairs.compare(front.pairs, front.required_pairs));
                    self.matched[2] =
                        full_match(self.fronts.outcome, front.outcome, self.fronts.mismatches);
                } else {
                    self.fronts.outcome = front.outcome;
                    self.fronts.checked = needed[2];
                    self.sections.len = front.sections;
                    self.sections.required = front.required_sections;
                    self.pairs.len = front.pairs;
                    self.pairs.required = front.required_pairs;
                }
                self.ticks[2] += qpc().saturating_sub(begin).max(0) as u64;
            }
        }
        let total = qpc().saturating_sub(begin_all).max(0) as u64;
        self.ticks[0] += total.saturating_sub(self.ticks.iter().sum::<u64>() - before);
        total
    }
    pub fn capture(
        &mut self,
        _: &impl Memory,
        _: usize,
        _: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        if !self.supplied[0] || (!self.supplied[1] && needed.iter().any(|v| *v)) {
            for (i, needed) in needed.iter().enumerate() {
                if !self.supplied[0] || *needed {
                    let outcome = Outcome::Unavailable {
                        reason: "plan source pass not supplied",
                    };
                    match i {
                        0 => self.plans.outcome = outcome,
                        1 => self.colors.outcome = outcome,
                        _ => self.fronts.outcome = outcome,
                    }
                    self.matched[i] = false;
                }
            }
        }
        (self.matched, self.ticks)
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            checked(&self.plans),
            checked(&self.colors),
            checked(&self.fronts),
        ]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.plans.mismatches,
            self.colors.mismatches,
            self.fronts.mismatches,
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
fn checked<R>(rows: &CaptureRows<R>) -> Outcome {
    if rows.checked && rows.mismatches != 0 {
        Outcome::Unavailable {
            reason: "battle plan changed during double read",
        }
    } else {
        rows.outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
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
    fn pass(m: &Image, c: &mut Captured, second: bool, needed: &[bool; N]) {
        let mut groups = [corders_group::Root::default(); 4];
        let mut visits = [corder_instance::Visit::default(); 8];
        let r = corder_instance::walk(
            m,
            0,
            &mut [corder_instance::Row::default(); 4],
            &mut [0; 4],
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
            second,
            needed,
        );
    }
    #[test]
    fn independent_native_passes_reject_changed_styles_and_colors_and_spool_exact_bytes() {
        let mut m = image();
        let mut c = Captured::with_capacities(&Capacities::default(), &[0; N]);
        m.put(ORDER + corder_instance::MOVE_SPECIAL, &[255]);
        m.u32(GROUP + corders_group::COLOUR, 0x7fc01234);
        pass(&m, &mut c, false, &[true; N]);
        pass(&m, &mut c, true, &[true; N]);
        assert_eq!(c.capture(&m, 0, 0, &[true; N]).0, [true; N]);
        assert!(
            matches!(&c.tables()[0],TableView::Complete(replay::rows::TableRows::ArmyOrderPlans(rows)) if rows[0].blitz==255)
        );
        assert!(
            matches!(&c.tables()[1],TableView::Complete(replay::rows::TableRows::OrdersGroupColors(rows)) if rows[0].red==0x7fc01234)
        );
        c.reset();
        pass(&m, &mut c, false, &[true; N]);
        m.put(ORDER + corder_instance::MOVE_SPECIAL, &[0]);
        m.u32(GROUP + corders_group::COLOUR, 0);
        pass(&m, &mut c, true, &[true; N]);
        assert_eq!(c.capture(&m, 0, 0, &[true; N]).0, [false, false, true]);
        assert!(matches!(c.outcomes()[0], Outcome::Unavailable { .. }));
        assert!(matches!(c.outcomes()[1], Outcome::Unavailable { .. }));
        assert_eq!(c.outcomes()[2], Outcome::Rows { len: 0 });
    }
    #[test]
    fn growth_retains_front_source_checks_after_order_window_and_missing_pass_fails() {
        let m = image();
        let mut capacity = Capacities::default();
        let mut c =
            Captured::with_capacities(&capacity, &[CONSISTENCY_TARGET, CONSISTENCY_TARGET, 0]);
        assert!(!c.plans.second.is_empty());
        assert!(c.colors.second.is_empty());
        pass(&m, &mut c, false, &[false, false, true]);
        pass(&m, &mut c, true, &[false, false, true]);
        assert!(c.capture(&m, 0, 0, &[false, false, true]).0[2]);
        c.assigned.required = capacity.assigned + 1;
        c.pairs.required = capacity.pairs + 1;
        c.required_handles = capacity.handles + 1;
        c.grow(&mut capacity, &[CONSISTENCY_TARGET; N]);
        assert!(
            c.plans.second.is_empty()
                && c.fronts.second.is_empty()
                && c.assigned.second.is_empty()
                && c.pairs.second.is_empty()
        );
        assert!(capacity.assigned > 16384 && capacity.pairs > 65536 && capacity.handles > 2048);
        c.reset();
        assert_eq!(c.capture(&m, 0, 0, &[false; N]).0, [false; N]);
        assert!(
            c.outcomes()
                .iter()
                .all(|v| matches!(v, Outcome::Unavailable { .. }))
        );
    }
}
