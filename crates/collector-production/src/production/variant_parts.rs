use super::*;
use std::sync::Arc;

pub struct VariantPart {
    first: usize,
    sources: Vec<cequipment_variant::RegistryRow>,
    carried: Vec<Option<Arc<rows::VariantDesignState>>>,
    leaves: Variants,
    fresh: bool,
    details: bool,
    definitions_only: bool,
    ticks: [u64; N],
}
impl VariantPart {
    pub fn capture(&mut self, memory: &impl Memory, base: usize) {
        let source = &self.sources;
        let begin = qpc();
        let mut reads = read_accounting::table(TABLES[3]);
        let mut carried = self.carried.iter();
        capture_text(
            &mut self.leaves.definitions,
            &mut self.leaves.definition_bytes,
            Ok(source),
            |source, rows, arena| {
                cequipment_variant::walk_definitions_carried(
                    memory,
                    base,
                    source,
                    rows,
                    arena,
                    |_| Ok(carried.next().cloned().flatten()),
                    |state| bonus_snapshots::intern(state).map(Arc::new),
                )
            },
        );
        self.ticks[3] = elapsed(begin);
        if self.definitions_only || !self.details {
            return;
        }
        reads.select(TABLES[4]);
        let begin = qpc();
        capture_source(&mut self.leaves.modules, Ok(source), |source, rows| {
            equipment_module_container::walk(memory, base, source, rows)
        });
        let mut carry = DesignCarry {
            held: &mut self.leaves.organisation_holds,
            fresh: self.fresh,
        };
        let mut witness = design_changes::StatLaneWitness;
        capture_source(&mut self.leaves.stats, Ok(source), |source, rows| {
            cequipment_variant::walk_stats_carried(
                memory,
                base,
                source,
                rows,
                &mut carry,
                &mut witness,
            )
        });
        if let Outcome::Rows { len } = self.leaves.stats.outcome {
            self.leaves.organisation_holds.truncate(len);
        }
        self.leaves.organisations_complete = matches!(
            self.leaves.stats.outcome,
            Outcome::Rows { len } if len == source.len()
        );
        capture_source(&mut self.leaves.upgrades, Ok(source), |source, rows| {
            cequipment_upgrades_instance::walk(memory, base, source, rows)
        });
        self.ticks[4] = elapsed(begin);
    }
}

fn ranges(len: usize, lanes: usize) -> impl Iterator<Item = std::ops::Range<usize>> {
    let width = len.div_ceil(64).div_ceil(lanes.max(1)).max(1) * 64;
    (0..len)
        .step_by(width)
        .map(move |first| first..len.min(first + width))
}

fn add_phase_ticks(total: &mut [u64; N], parts: &[VariantPart]) {
    for (index, total) in total.iter_mut().enumerate() {
        let phase = parts
            .iter()
            .map(|part| part.ticks[index])
            .max()
            .unwrap_or(0);
        *total = total.saturating_add(phase);
    }
}

fn append<R>(target: &mut CaptureRows<R>, source: &mut CaptureRows<R>) -> Result<(), &'static str> {
    let len = match source.outcome {
        Outcome::Rows { len } => len,
        Outcome::Unavailable { reason } => {
            if matches!(target.outcome, Outcome::Rows { .. }) {
                target.outcome = Outcome::Unavailable { reason };
            }
            return Ok(());
        }
        Outcome::Truncated { .. } => {
            target.outcome = Outcome::Unavailable {
                reason: "variant partition has an incomplete source",
            };
            return Ok(());
        }
    };
    engine_allocation::reserve(&mut target.rows, len)
        .map_err(|_| "variant partition join allocation failed")?;
    target.rows.extend(source.rows.drain(..len));
    if let Outcome::Rows { len: total } = &mut target.outcome {
        *total += len;
    }
    Ok(())
}
impl Variants {
    pub(super) fn capture_partitioned(
        &mut self,
        base: usize,
        variants: Sources<'_, cequipment_variant::RegistryRow>,
        hour: i32,
        full: bool,
        recovery_carry: bool,
        lanes: usize,
        acquire: &mut impl FnMut(Vec<VariantPart>) -> Result<Vec<VariantPart>, &'static str>,
    ) -> [u64; N] {
        let result = self.capture_partitioned_with(
            base,
            variants,
            hour,
            full,
            recovery_carry,
            lanes,
            acquire,
        );
        match result {
            Ok(ticks) => ticks,
            Err(reason) => {
                self.fail(reason, hour, full);
                [0; N]
            }
        }
    }
    fn capture_partitioned_with(
        &mut self,
        _base: usize,
        variants: Sources<'_, cequipment_variant::RegistryRow>,
        hour: i32,
        full: bool,
        recovery_carry: bool,
        lanes: usize,
        acquire: &mut impl FnMut(Vec<VariantPart>) -> Result<Vec<VariantPart>, &'static str>,
    ) -> Result<[u64; N], &'static str> {
        let sources = complete_source(variants.rows, variants.outcome)?;
        let mut carried_all = engine_allocation::vec(sources.len())
            .map_err(|_| "variant partition allocation failed")?;
        let held = (!full).then(design_changes::held::certified).flatten();
        let census = (full && recovery_carry)
            .then(design_changes::held::census)
            .flatten();
        for source in sources {
            carried_all.push(
                held.as_ref()
                    .and_then(|held| held.get(source))
                    .or_else(|| census.as_ref().and_then(|census| census.get(source))),
            );
        }
        let census = census.map(design_changes::held::Census::release);
        drop(held);
        let mut carried_all = carried_all.into_iter();
        let details = replay_data::cadence::should_walk(TableId::VariantModules, hour, full);
        if details {
            collector_observers::world_input_changes::prepare_design_mios(sources.len(), full);
        }
        let mut old_stats = std::mem::take(&mut self.stats.rows).into_iter();
        let mut old_holds = std::mem::take(&mut self.organisation_holds).into_iter();
        let mut parts = engine_allocation::vec(lanes.min(sources.len().div_ceil(64)))
            .map_err(|_| "variant partition allocation failed")?;
        for range in ranges(sources.len(), lanes) {
            let mut leaves = Variants::vacant();
            leaves.stats.rows = engine_allocation::vec(range.len())
                .map_err(|_| "variant partition allocation failed")?;
            leaves.organisation_holds = engine_allocation::vec(range.len())
                .map_err(|_| "variant partition allocation failed")?;
            for _ in range.clone() {
                leaves.stats.rows.push(old_stats.next().unwrap_or_default());
                leaves
                    .organisation_holds
                    .push(old_holds.next().unwrap_or_default());
            }
            let mut carried = engine_allocation::vec(range.len())
                .map_err(|_| "variant partition allocation failed")?;
            carried.extend(carried_all.by_ref().take(range.len()));
            parts.push(VariantPart {
                first: range.start,
                sources: engine_allocation::copy_vec(&sources[range])
                    .map_err(|_| "variant partition allocation failed")?,
                carried,
                leaves,
                fresh: full || !self.organisations_complete,
                details,
                definitions_only: false,
                ticks: [0; N],
            });
        }
        let mut parts = acquire(parts)?;
        let mut ticks = [0; N];
        add_phase_ticks(&mut ticks, &parts);
        let retry = census.as_ref().is_some_and(|census| !census.is_current());
        if retry {
            for part in &mut parts {
                part.carried.fill(None);
                part.definitions_only = true;
                part.ticks.fill(0);
            }
            parts = acquire(parts)?;
            add_phase_ticks(&mut ticks, &parts);
        }
        parts.sort_unstable_by_key(|part| part.first);
        let mut expected: usize = 0;
        for part in &parts {
            let end = expected
                .checked_add(part.sources.len())
                .ok_or("variant partition census size overflow")?;
            if part.first != expected || sources.get(expected..end) != Some(part.sources.as_slice())
            {
                return Err("variant partition census changed during acquisition");
            }
            expected = end;
        }
        if expected != sources.len() {
            return Err("variant partition census incomplete");
        }
        self.definitions.rows.clear();
        self.definitions.outcome = Outcome::Rows { len: 0 };
        self.definition_bytes.reset();
        self.modules.rows.clear();
        self.modules.outcome = Outcome::Rows { len: 0 };
        self.stats.outcome = Outcome::Rows { len: 0 };
        self.upgrades.rows.clear();
        self.upgrades.outcome = Outcome::Rows { len: 0 };
        let mut organisations_complete = true;
        for mut part in parts {
            organisations_complete &= part.leaves.organisations_complete;
            if matches!(part.leaves.definitions.outcome, Outcome::Rows { .. }) {
                let bytes = &part.leaves.definition_bytes.first[..part.leaves.definition_bytes.len];
                self.definition_bytes
                    .first
                    .truncate(self.definition_bytes.len);
                engine_allocation::reserve(&mut self.definition_bytes.first, bytes.len())
                    .map_err(|_| "variant partition text allocation failed")?;
                let offset = self.definition_bytes.len;
                self.definition_bytes.first.extend_from_slice(bytes);
                self.definition_bytes.len += bytes.len();
                for row in part
                    .leaves
                    .definitions
                    .rows
                    .iter_mut()
                    .take(part.leaves.definitions.outcome.len())
                {
                    for span in [
                        &mut row.display_name,
                        &mut row.variant_name,
                        &mut row.explicit_entity,
                        &mut row.icon,
                    ] {
                        span.start = span
                            .start
                            .checked_add(offset)
                            .ok_or("variant partition text offset overflow")?;
                    }
                }
            }
            append(&mut self.definitions, &mut part.leaves.definitions)?;
            if details {
                append(&mut self.modules, &mut part.leaves.modules)?;
                append(&mut self.stats, &mut part.leaves.stats)?;
                append(&mut self.upgrades, &mut part.leaves.upgrades)?;
                engine_allocation::reserve(
                    &mut self.organisation_holds,
                    part.leaves.organisation_holds.len(),
                )
                .map_err(|_| "variant partition join allocation failed")?;
                self.organisation_holds
                    .append(&mut part.leaves.organisation_holds);
            } else {
                self.stats.rows.append(&mut part.leaves.stats.rows);
                self.organisation_holds
                    .append(&mut part.leaves.organisation_holds);
            }
        }
        if details {
            self.organisations_complete =
                matches!(self.stats.outcome, Outcome::Rows { .. }) && organisations_complete;
            self.organisation_bytes = self
                .stats
                .rows
                .iter()
                .map(|row| {
                    row.organisation.bonuses.capacity()
                        * std::mem::size_of::<cequipment_variant::OrganisationBonus>()
                })
                .sum();
        } else {
            let skipped = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
            self.modules.outcome = skipped;
            self.stats.outcome = skipped;
            self.upgrades.outcome = skipped;
        }
        Ok(ticks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::production::variant_scale_tests::{Space, space};
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    fn registry(space: &Space) -> Vec<cequipment_variant::RegistryRow> {
        let mut source = Vec::new();
        assert!(matches!(
            cequipment_variant::walk_registry(
                &space.memory(),
                space.base,
                space.game_state,
                &mut source,
            ),
            Outcome::Rows { .. }
        ));
        source
    }
    fn views(mut variants: Variants) -> [TableView; 4] {
        variants.settle_owned();
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.restore_variants(variants);
        let [_, _, _, definitions, modules, stats, upgrades, ..] = captured.tables();
        [definitions, modules, stats, upgrades]
    }
    fn serial(
        memory: &impl Memory,
        base: usize,
        source: &[cequipment_variant::RegistryRow],
    ) -> Variants {
        let mut variants = Variants::vacant();
        variants.capture_state_carried(
            memory,
            base,
            Sources {
                rows: source,
                outcome: Outcome::Rows { len: source.len() },
            },
            22,
            true,
            false,
        );
        variants
    }
    #[test]
    fn parallel_census_matches_every_serial_leaf_for_fifteen_hundred_countries() {
        let space = space(1500, 5);
        let source = registry(&space);
        let expected = views(serial(&space.memory(), space.base, &source));
        for lanes in [2, 3, 15] {
            let widest = AtomicUsize::new(0);
            let mut variants = Variants::vacant();
            variants
                .capture_partitioned_with(
                    space.base,
                    Sources {
                        rows: &source,
                        outcome: Outcome::Rows { len: source.len() },
                    },
                    22,
                    true,
                    false,
                    lanes,
                    &mut |parts| {
                        let barrier = Barrier::new(parts.len());
                        Ok(std::thread::scope(|scope| {
                            let jobs: Vec<_> = parts
                                .into_iter()
                                .map(|mut part| {
                                    widest.fetch_max(part.sources.len(), Ordering::Relaxed);
                                    let barrier = &barrier;
                                    let space = &space;
                                    scope.spawn(move || {
                                        barrier.wait();
                                        part.capture(&space.memory(), space.base);
                                        part
                                    })
                                })
                                .collect();
                            jobs.into_iter()
                                .rev()
                                .map(|job| job.join().unwrap())
                                .collect()
                        }))
                    },
                )
                .unwrap();
            assert!(widest.load(Ordering::Relaxed) < source.len());
            assert_eq!(views(variants), expected);
        }
    }
    #[test]
    fn partitions_preserve_empty_sources_and_the_first_unreadable_leaf() {
        let space = space(1, 193);
        let source = registry(&space);
        struct Refused<'a> {
            space: &'a Space,
            at: usize,
        }
        impl Memory for Refused<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address <= self.at && self.at < address.saturating_add(output.len()) {
                    return false;
                }
                self.space.memory().copy(address, output)
            }
        }
        for source in [&source[..], &source[..0]] {
            let memory = Refused {
                space: &space,
                at: source
                    .get(130)
                    .map_or(usize::MAX, |row| row.pointer() + cequipment_variant::NICHES),
            };
            let expected = views(serial(&memory, space.base, source));
            let mut variants = Variants::vacant();
            variants
                .capture_partitioned_with(
                    space.base,
                    Sources {
                        rows: source,
                        outcome: Outcome::Rows { len: source.len() },
                    },
                    22,
                    true,
                    false,
                    3,
                    &mut |mut parts| {
                        for part in &mut parts {
                            part.capture(&memory, space.base);
                        }
                        parts.reverse();
                        Ok(parts)
                    },
                )
                .unwrap();
            assert_eq!(views(variants), expected);
        }
    }
    #[test]
    fn missing_partition_is_unavailable_not_an_empty_or_shorter_census() {
        let space = space(1, 129);
        let source = registry(&space);
        let mut variants = Variants::vacant();
        variants.capture_partitioned(
            space.base,
            Sources {
                rows: &source,
                outcome: Outcome::Rows { len: source.len() },
            },
            22,
            true,
            false,
            3,
            &mut |mut parts| {
                for part in &mut parts {
                    part.capture(&space.memory(), space.base);
                }
                parts.remove(1);
                Ok(parts)
            },
        );
        for view in views(variants) {
            assert!(matches!(view, TableView::Unavailable { .. }));
        }
    }
    #[test]
    fn partition_boundaries_keep_guarded_batches_and_have_no_population_cap() {
        for count in [0, 1, 64, 65, 6228, 7500, 150_000] {
            for lanes in [0, 1, 3, 15] {
                let ranges: Vec<_> = ranges(count, lanes).collect();
                assert_eq!(ranges.iter().map(|range| range.len()).sum::<usize>(), count);
                assert!(ranges.len() <= lanes.max(1));
                for (index, range) in ranges.iter().enumerate() {
                    assert_eq!(range.start % 64, 0);
                    assert_eq!(
                        range.start,
                        index.checked_sub(1).map_or(0, |i| ranges[i].end)
                    );
                    if range.end != count {
                        assert_eq!(range.end % 64, 0);
                    }
                }
            }
        }
    }
}
