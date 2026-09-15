//! Hourly supply train operands, with independent table windows and costs.
use crate::{
    sampling::{full_match, qpc},
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, ccountry_supply_system, supply_node, supply_train_entry, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{SupplyNodeRow, TableId, TrainAllocation, TrainInventoryRow, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::SupplyNodes, TableId::TrainInventory];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    nodes: usize,
    countries: usize,
    paths: usize,
    entries: usize,
    access: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            nodes: 16384,
            countries: 512,
            paths: 131072,
            entries: 4096,
            access: 262144,
        }
    }
}
pub struct Captured {
    nodes: CaptureRows<supply_node::Row>,
    countries: CaptureRows<ccountry_supply_system::TrainRow>,
    paths: Arena<u32>,
    entries: Arena<supply_train_entry::Row>,
    access: Arena<u8>,
}
impl Captured {
    pub fn with_capacities(c: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            nodes: CaptureRows::new(c.nodes, successful[0]),
            countries: CaptureRows::new(c.countries, successful[1]),
            paths: Arena::new(c.paths, successful[0]),
            entries: Arena::new(c.entries, successful[1]),
            access: Arena::new(c.access, successful[1]),
        }
    }
    pub fn reset(&mut self) {
        self.nodes.reset();
        self.countries.reset();
        self.paths.reset();
        self.entries.reset();
        self.access.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.nodes.allocated_bytes()
            + self.countries.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.entries.allocated_bytes()
            + self.access.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities, successful: &[u16]) {
        self.nodes.grow(&mut c.nodes, successful[0]);
        self.countries.grow(&mut c.countries, successful[1]);
        self.paths.grow(&mut c.paths, successful[0]);
        self.entries.grow(&mut c.entries, successful[1]);
        self.access.grow(&mut c.access, successful[1]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let mut matched = [false; N];
        let mut ticks = [0; N];
        let begin = qpc();
        let first = supply_node::walk(
            memory,
            base,
            state,
            &mut self.nodes.rows,
            &mut self.paths.first,
        );
        self.nodes.outcome = first.outcome;
        self.nodes.checked = needed[0];
        self.paths.len = first.paths;
        self.paths.required = first.required_paths;
        if needed[0] {
            let second = supply_node::walk(
                memory,
                base,
                state,
                &mut self.nodes.second,
                &mut self.paths.second,
            );
            self.nodes.compare(second.outcome);
            self.nodes.mismatches +=
                usize::from(!self.paths.compare(second.paths, second.required_paths));
            matched[0] = full_match(first.outcome, second.outcome, self.nodes.mismatches);
        }
        ticks[0] = qpc().saturating_sub(begin).max(0) as u64;
        let begin = qpc();
        let first = ccountry_supply_system::walk_trains(
            memory,
            base,
            state,
            &mut self.countries.rows,
            &mut self.entries.first,
            &mut self.access.first,
        );
        self.countries.outcome = first.outcome;
        self.countries.checked = needed[1];
        self.entries.len = first.entries;
        self.entries.required = first.required_entries;
        self.access.len = first.access;
        self.access.required = first.required_access;
        if needed[1] {
            let second = ccountry_supply_system::walk_trains(
                memory,
                base,
                state,
                &mut self.countries.second,
                &mut self.entries.second,
                &mut self.access.second,
            );
            self.countries.compare(second.outcome);
            self.countries.mismatches += usize::from(
                !self
                    .entries
                    .compare(second.entries, second.required_entries),
            );
            self.countries.mismatches +=
                usize::from(!self.access.compare(second.access, second.required_access));
            matched[1] = full_match(first.outcome, second.outcome, self.countries.mismatches);
        }
        ticks[1] = qpc().saturating_sub(begin).max(0) as u64;
        (matched, ticks)
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            if self.nodes.checked && self.nodes.mismatches != 0 {
                Outcome::Unavailable {
                    reason: "supply nodes changed during double read",
                }
            } else {
                self.nodes.outcome
            },
            if self.countries.checked && self.countries.mismatches != 0 {
                Outcome::Unavailable {
                    reason: "train inventory changed during double read",
                }
            } else {
                self.countries.outcome
            },
        ]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.nodes.mismatches, self.countries.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        let nodes =
            table::<tables::SupplyNodes, _>(outcomes[0], &self.nodes.rows, |r| SupplyNodeRow {
                country: r.country,
                index: r.index,
                province: r.province,
                kind: r.kind,
                flag: r.flag,
                available: r.available,
                consumed: r.consumed,
                parent: r.parent,
                route_from: r.route.from,
                route_to: r.route.to,
                break_after: r.route.break_after,
                fallback_emit: r.route.fallback_emit,
                path: self.paths.first[r.route.start..r.route.start + r.route.len].to_vec(),
            });
        // The immediate-edge contract must close over the captured node census.
        // A foreign/non-node parent cannot silently become a guessed railway path.
        let nodes = match nodes {
            TableView::Complete(replay::rows::TableRows::SupplyNodes(ref rows)) => {
                let keys: std::collections::BTreeSet<_> =
                    rows.iter().map(|r| (r.country, r.province)).collect();
                if rows
                    .iter()
                    .any(|r| r.parent > 0 && !keys.contains(&(r.country, r.parent as u32)))
                {
                    TableView::Unavailable {
                        reason: "supply parent graph leaves recorded node census".into(),
                    }
                } else {
                    nodes
                }
            }
            _ => nodes,
        };
        [
            nodes,
            table::<tables::TrainInventory, _>(outcomes[1], &self.countries.rows, |r| {
                TrainInventoryRow {
                    country: r.country,
                    node_count: r.node_count,
                    allocations: self.entries.first
                        [r.entries_start..r.entries_start + r.entries_len]
                        .iter()
                        .map(|e| TrainAllocation {
                            variant: e.variant,
                            count: e.count,
                        })
                        .collect(),
                    controller_access: self.access.first
                        [r.access_start..r.access_start + r.access_len]
                        .to_vec(),
                }
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_windows_own_their_arenas_and_source_failures_stay_unavailable() {
        let c = Capacities {
            nodes: 2,
            countries: 2,
            paths: 2,
            entries: 2,
            access: 2,
        };
        let mut capture = Captured::with_capacities(&c, &[200, 0]);
        assert!(capture.nodes.second.is_empty() && capture.paths.second.is_empty());
        assert_eq!(capture.countries.second.len(), 2);
        assert_eq!(capture.entries.second.len(), 2);
        assert_eq!(capture.access.second.len(), 2);
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        assert_eq!(
            capture.capture(&memory, 0, 0, &[false, true]).0,
            [false, false]
        );
        assert!(
            capture
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        capture.reset();
        assert_eq!(capture.checked_mismatches(), [0, 0]);
    }
}
