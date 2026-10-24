use super::*;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Sparse {
    #[cfg(test)]
    pub(super) hours: usize,
    pub(super) provinces: Provinces,
    pub(super) combat: Combat,
    pub(super) keys: Vec<u128>,
    pub(super) candidate_marks: Marks,
    pub(super) source_keys: Vec<u128>,
    union: Vec<u128>,
}

#[derive(Default)]
pub(super) struct Provinces {
    ready: bool,
    // XOR recovers the remaining province after any occurrence is removed.
    occurrences: HashMap<u64, (usize, u32)>,
}

impl Provinces {
    pub(super) fn unique(&self, id: u64) -> Option<u32> {
        self.occurrences
            .get(&id)
            .and_then(|&(count, province)| (count == 1).then_some(province))
    }

    fn insert(&mut self, id: u64, province: u32) -> io::Result<()> {
        if !self.occurrences.contains_key(&id) {
            allocation::reserve_map(&mut self.occurrences, 1)?;
        }
        let (count, xor) = self.occurrences.entry(id).or_default();
        *count = allocation::add(*count, 1)?;
        *xor ^= province;
        Ok(())
    }

    pub(super) fn reset(&mut self, sources: &[(TableId, SharedView)]) -> io::Result<()> {
        self.ready = false;
        self.occurrences.clear();
        let Some((_, view)) = sources
            .iter()
            .find(|(table, _)| *table == TableId::ProvinceUnitOrder)
        else {
            return Ok(());
        };
        if let TableView::Complete(TableRows::ProvinceUnitOrder(rows)) = &**view {
            for row in rows {
                for &id in &row.mixed {
                    self.insert(id, row.province)?;
                }
            }
            self.ready = true;
        }
        Ok(())
    }

    pub(super) fn ensure(&mut self, sources: &[(TableId, SharedView)]) -> io::Result<()> {
        if !self.ready {
            self.reset(sources)?;
        }
        Ok(())
    }

    pub(super) fn remove(&mut self, view: &TableView, keys: &[u128]) {
        if !self.ready {
            return;
        }
        let TableView::Complete(TableRows::ProvinceUnitOrder(rows)) = view else {
            self.ready = false;
            self.occurrences.clear();
            return;
        };
        for &key in keys {
            if let Some(row) = find(Some(rows), key) {
                for id in &row.mixed {
                    let (count, xor) = self.occurrences.get_mut(id).expect("indexed occurrence");
                    *count -= 1;
                    *xor ^= row.province;
                    if *count == 0 {
                        self.occurrences.remove(id);
                    }
                }
            }
        }
    }

    pub(super) fn add(&mut self, view: &TableView, keys: &[u128]) -> io::Result<()> {
        let TableView::Complete(TableRows::ProvinceUnitOrder(rows)) = view else {
            self.ready = false;
            self.occurrences.clear();
            return Ok(());
        };
        if self.ready {
            for &key in keys {
                if let Some(row) = find(Some(rows), key) {
                    for &id in &row.mixed {
                        self.insert(id, row.province)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Combat {
    ready: bool,
    identities: HashMap<u128, u64>,
    references: HashMap<u64, Vec<u128>>,
}

impl Combat {
    fn remove(&mut self, key: u128) {
        if let Some(division) = self.identities.remove(&key) {
            let keys = self
                .references
                .get_mut(&division)
                .expect("indexed division");
            keys.remove(keys.binary_search(&key).expect("indexed combat member"));
            if keys.is_empty() {
                self.references.remove(&division);
            }
        }
    }

    fn insert(&mut self, row: &LandCombatDivisionRow) -> io::Result<()> {
        allocation::reserve_map(&mut self.identities, 1)?;
        if !self.references.contains_key(&row.division) {
            allocation::reserve_map(&mut self.references, 1)?;
        }
        let keys = self.references.entry(row.division).or_default();
        allocation::reserve(keys, 1)?;
        let at = keys.binary_search(&row.key()).unwrap_err();
        keys.insert(at, row.key());
        self.identities.insert(row.key(), row.division);
        Ok(())
    }

    pub(super) fn advance(&mut self, view: &TableView, update: &TableUpdate) -> io::Result<()> {
        let TableView::Complete(TableRows::LandCombatDivisions(rows)) = view else {
            self.ready = false;
            self.identities.clear();
            self.references.clear();
            return Ok(());
        };
        if self.ready {
            match update {
                TableUpdate::Unchanged { .. } => return Ok(()),
                TableUpdate::Delta {
                    changed, deleted, ..
                } => {
                    for &key in deleted {
                        self.remove(key);
                    }
                    for delta in changed {
                        let row = find(Some(rows), delta.key).expect("applied raw combat row");
                        if self.identities.get(&delta.key) != Some(&row.division) {
                            self.remove(delta.key);
                            self.insert(row)?;
                        }
                    }
                    return Ok(());
                }
                _ => {}
            }
        }
        self.ready = false;
        self.identities.clear();
        self.references.clear();
        for row in rows {
            self.insert(row)?;
        }
        self.ready = true;
        Ok(())
    }

    pub(super) fn project(
        &self,
        divisions: &[(u128, u64)],
        source: TableId,
        marks: &[(u128, u16)],
        default: u16,
        out: &mut Vec<u128>,
    ) -> io::Result<()> {
        for &(division, mask) in divisions {
            if let Ok(division) = u64::try_from(division)
                && let Some(keys) = self.references.get(&division)
            {
                let lanes = lanes_for(TableId::LandCombatDivisions, source, mask);
                for &key in keys {
                    let bits = default
                        ^ marks
                            .binary_search_by_key(&key, |&(key, _)| key)
                            .map_or(0, |at| marks[at].1);
                    if bits & lanes != 0 {
                        allocation::reserve(out, 1)?;
                        out.push(key);
                    }
                }
            }
        }
        Ok(())
    }
}

fn dependency_fields(table: TableId, source: TableId) -> &'static [(usize, u16)] {
    match (table, source) {
        (TableId::DivisionMovement, TableId::DivisionPanelStats) => &[(21, 1)],
        (TableId::DivisionStatus, TableId::DivisionPanelStats) => &[(0, 0), (1, 1)],
        (TableId::DivisionStatus, TableId::SimulationFeatures) => &[(5, 3)],
        (TableId::Divisions, TableId::ProvinceUnitOrder) => &[(0, 0)],
        (TableId::Divisions, TableId::DivisionStatus) => &[(42, 1)],
        (TableId::LandCombatDivisionModifiers, TableId::LandCombatDivisions) => &[(2, 0)],
        (TableId::ProductionLines, TableId::ProductionLineModifiers) => {
            &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (9, 0)]
        }
        (TableId::LandCombatDivisions, TableId::DivisionPanelStats) => &[
            (11, 0),
            (12, 1),
            (20, 2),
            (9, 3),
            (10, 4),
            (13, 5),
            (14, 6),
            (3, 7),
            (15, 8),
        ],
        (TableId::LandCombatDivisions, TableId::DivisionStatus) => &[(11, 9), (31, 10)],
        (TableId::LandCombatDivisions, TableId::DivisionMovement) => &[(23, 11)],
        _ => &[],
    }
}

fn lanes_for(table: TableId, source: TableId, mask: u64) -> u16 {
    dependency_fields(table, source)
        .iter()
        .fold(0, |bits, &(field, lane)| {
            bits | (u16::from(mask & (1u64 << field) != 0) << lane)
        })
}

impl Sparse {
    pub(super) fn union_tail(&mut self, keys: &mut Vec<u128>, from: usize) -> io::Result<()> {
        if from == keys.len() {
            return Ok(());
        }
        if !keys[from..].is_sorted() {
            keys[from..].sort_unstable();
        }
        self.union.clear();
        allocation::reserve(&mut self.union, keys.len())?;
        let (before, added) = keys.split_at(from);
        let (mut before, mut added) = (
            before.iter().copied().peekable(),
            added.iter().copied().peekable(),
        );
        loop {
            let next = match (before.peek(), added.peek()) {
                (Some(a), Some(b)) if a <= b => before.next(),
                (_, Some(_)) => added.next(),
                (Some(_), None) => before.next(),
                (None, None) => break,
            }
            .unwrap();
            if self.union.last() != Some(&next) {
                self.union.push(next);
            }
        }
        std::mem::swap(keys, &mut self.union);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn retain_source_keys(
        &self,
        table: TableId,
        source: TableId,
        rows: &TableRows,
        marks: &[(u128, u16)],
        default: u16,
        keys: &mut Vec<u128>,
        from: usize,
    ) {
        let lanes = lanes_for(table, source, u64::MAX);
        let mut write = from;
        for read in from..keys.len() {
            let key = keys[read];
            let bits = default
                ^ marks
                    .binary_search_by_key(&key, |&(key, _)| key)
                    .map_or(0, |at| marks[at].1);
            let present = match rows {
                TableRows::Divisions(rows) => find(Some(rows), key).is_some(),
                TableRows::LandCombatDivisions(rows) => find(Some(rows), key).is_some(),
                _ => true,
            };
            if bits & lanes != 0 && present {
                keys[write] = key;
                write += 1;
            }
        }
        keys.truncate(write);
    }

    pub(super) fn member_fields(
        &self,
        rows: &TableRows,
        marks: &[(u128, u16)],
        default: u16,
        view: &TableView,
        key: u128,
    ) -> u64 {
        let (
            TableRows::Divisions(rows),
            TableView::Complete(TableRows::ProvinceUnitOrder(provinces)),
        ) = (rows, view)
        else {
            return 0;
        };
        u64::from(find(Some(provinces), key).is_some_and(|province| {
            province.mixed.iter().any(|&id| {
                let key = u128::from(id);
                let bits = default
                    ^ marks
                        .binary_search_by_key(&key, |&(key, _)| key)
                        .map_or(0, |at| marks[at].1);
                bits & 1 != 0 && find(Some(rows), key).is_some()
            })
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn relevant(
        &self,
        table: TableId,
        source: TableId,
        rows: &TableRows,
        marks: &[(u128, u16)],
        default: u16,
        key: u128,
        mask: u64,
    ) -> bool {
        let lanes = lanes_for(table, source, mask);
        if lanes == 0 {
            return false;
        }
        let bits = |key| {
            default
                ^ marks
                    .binary_search_by_key(&key, |&(key, _)| key)
                    .map_or(0, |at| marks[at].1)
        };
        let measured = measurement::enabled();
        let demanded = match feeds(table, source) {
            // Membership edits maintain the inverse even without a marked receiver.
            Feeds::Members => return true,
            Feeds::Division => {
                let mut demanded = 0;
                if let Ok(id) = u64::try_from(key)
                    && let Some(keys) = self.combat.references.get(&id)
                {
                    for &key in keys {
                        demanded |= bits(key) & lanes;
                        if demanded == lanes || (!measured && demanded != 0) {
                            break;
                        }
                    }
                }
                demanded
            }
            Feeds::Table => {
                let TableRows::DivisionStatus(rows) = rows else {
                    return false;
                };
                let mut demanded = 0;
                for row in rows {
                    let bits = bits(row.key()) & lanes;
                    if bits != 0
                        && matches!(row.supply_grace_remaining, Recorded::Value(_))
                        && exact(row.supply_grace_exact).is_some()
                    {
                        demanded |= bits;
                        if demanded == lanes || !measured {
                            break;
                        }
                    }
                }
                demanded
            }
            Feeds::Key => {
                let bits = bits(key);
                if bits & lanes == 0 {
                    return false;
                }
                let present = match rows {
                    TableRows::DivisionMovement(rows) => find(Some(rows), key)
                        .is_some_and(|row| matches!(row.base_speed, Recorded::Value(Some(_)))),
                    TableRows::Divisions(rows) => find(Some(rows), key).is_some(),
                    TableRows::DivisionStatus(rows) => find(Some(rows), key).is_some(),
                    TableRows::ProductionLines(rows) => find(Some(rows), key).is_some(),
                    TableRows::LandCombatDivisionModifiers(rows) => find(Some(rows), key).is_some(),
                    _ => false,
                };
                if present { bits } else { 0 }
            }
        };
        if !measured {
            return demanded & lanes != 0;
        }
        let demanded_mask =
            dependency_fields(table, source)
                .iter()
                .fold(0, |fields, &(field, lane)| {
                    fields
                        | if demanded & (1 << lane) != 0 {
                            1u64 << field
                        } else {
                            0
                        }
                })
                & mask;
        if demanded_mask != 0 {
            measurement::source(table, source, demanded_mask);
        }
        demanded_mask != 0
    }
}

pub(super) fn view_len(view: &TableView) -> usize {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
        _ => 0,
    }
}

pub(super) fn clone_bytes(rows: &TableRows) -> usize {
    if !measurement::enabled() {
        return 0;
    }
    rows.inline_bytes()
}
