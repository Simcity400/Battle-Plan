//! Worker-only deep state. The ordinary observer owns the certificate; these
//! values cannot make an incomplete interval complete.
use super::*;
use std::{
    collections::HashMap,
    sync::{Arc, MutexGuard},
};

type Entry = (usize, Arc<rows::VariantDesignState>);
static HELD: OnceLock<Mutex<HashMap<u64, Entry>>> = OnceLock::new();
fn cache() -> MutexGuard<'static, HashMap<u64, Entry>> {
    HELD.get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(in super::super) struct Certified(MutexGuard<'static, HashMap<u64, Entry>>);
impl Certified {
    pub(in super::super) fn get(
        &self,
        source: &native::RegistryRow,
    ) -> Option<Arc<rows::VariantDesignState>> {
        self.0
            .get(&source.id)
            .filter(|(pointer, _)| *pointer == source.pointer())
            .map(|(_, state)| Arc::clone(state))
    }
}
pub(in super::super) fn certified() -> Option<Certified> {
    (native::mutations::WRITER_AUDIT_REFUSAL.is_none()
        && SAVED_BASELINE.load(Ordering::Acquire)
        && SOURCE_BASELINE.load(Ordering::Acquire)
        && LIVENESS_BASELINE.load(Ordering::Acquire)
        && !RECOVER.load(Ordering::Acquire)
        && !pending()
        && active() == 0)
        .then(|| Certified(cache()))
}
pub(super) fn seed(
    registry: &[native::RegistryRow],
    variants: &Variants,
) -> Result<(), &'static str> {
    let mut held = cache();
    held.clear();
    engine_allocation::reserve_map(&mut held, registry.len())
        .map_err(|_| "variant held state allocation failed")?;
    for (source, row) in registry.iter().zip(&variants.definitions.rows) {
        if row.identity.id != source.id {
            return Err("variant held state identity mismatch");
        }
        held.insert(source.id, (source.pointer(), Arc::clone(&row.design)));
    }
    Ok(())
}
pub(super) fn apply(events: &[Event], rows: &[rows::VariantChangeRow]) -> Result<(), &'static str> {
    if events.is_empty() {
        return Ok(());
    }
    if events
        .iter()
        .filter(|event| matches!(event.kind, Kind::Design | Kind::Scalar(_)))
        .count()
        != rows
            .iter()
            .filter(|row| row.sources.is_none() && row.organisation.is_none())
            .count()
    {
        return Err("variant held state event count mismatch");
    }
    let mut held = cache();
    apply_to(&mut held, events, rows)
}
fn apply_to(
    held: &mut HashMap<u64, Entry>,
    events: &[Event],
    rows: &[rows::VariantChangeRow],
) -> Result<(), &'static str> {
    let mut events = events
        .iter()
        .filter(|event| matches!(event.kind, Kind::Design | Kind::Scalar(_)));
    for row in rows {
        if row.organisation.is_some() {
            continue;
        }
        if let Some(sources) = &row.sources {
            let (pointer, previous) = held
                .get(&row.variant)
                .ok_or("variant source change lacks preceding state")?;
            let mut state = rows::NativeValue::try_clone(previous.as_ref())
                .map_err(|_| "variant source change allocation failed")?;
            state.bonus_sources = engine_allocation::copy_vec(&sources.direct)
                .map_err(|_| "variant source change allocation failed")?;
            state.named_sources = engine_allocation::copy_vec(&sources.named)
                .map_err(|_| "variant source change allocation failed")?;
            let pointer = *pointer;
            held.insert(row.variant, (pointer, Arc::new(state)));
            continue;
        }
        let event = events
            .next()
            .ok_or("variant held state event count mismatch")?;
        if row.removed {
            held.remove(&row.variant);
            continue;
        }
        if let Some(scalar) = &row.scalar {
            let (pointer, previous) = held
                .get(&row.variant)
                .ok_or("variant scalar has no preceding state")?;
            if *pointer != event.pointer {
                return Err("variant scalar address mismatch");
            }
            if scalar.changes_design() {
                let mut state = rows::NativeValue::try_clone(previous.as_ref())
                    .map_err(|_| "variant scalar allocation failed")?;
                scalar
                    .apply_design(&mut state)
                    .map_err(|_| "variant scalar apply failed")?;
                let pointer = *pointer;
                held.insert(row.variant, (pointer, Arc::new(state)));
            }
            continue;
        }
        let state = if let Some(flags) = row.flags {
            let (pointer, previous) = held
                .get(&row.variant)
                .ok_or("variant held eligibility has no preceding state")?;
            if *pointer != event.pointer {
                return Err("variant held eligibility address mismatch");
            }
            let mut state = rows::NativeValue::try_clone(previous.as_ref())
                .map_err(|_| "variant held state allocation failed")?;
            state.can_upgrade_type = flags[0];
            state.can_upgrade_variant = flags[1];
            state.can_upgrade_modules = flags[2];
            state
        } else {
            let state = row
                .snapshot
                .as_ref()
                .ok_or("variant held event lacks its snapshot")?
                .definition
                .design
                .require("VariantChanges.design")
                .map_err(|_| "variant held event lacks its design state")?;
            rows::NativeValue::try_clone(state)
                .map_err(|_| "variant held state allocation failed")?
        };
        engine_allocation::reserve_map(held, 1)
            .map_err(|_| "variant held state allocation failed")?;
        held.insert(row.variant, (event.pointer, Arc::new(state)));
    }
    Ok(())
}
pub(super) fn release() {
    cache().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_events_keep_untouched_designs_and_name_only_edits_share_deep_storage() {
        let previous = Arc::new(rows::VariantDesignState {
            manpower: 99,
            ..Default::default()
        });
        let mut held = HashMap::from([
            (1, (100, Arc::clone(&previous))),
            (2, (200, Arc::clone(&previous))),
        ]);
        let event = Event {
            kind: Kind::Scalar(scalar::Field::Name),
            sequence: 1,
            hour: 7,
            pointer: 100,
            variant: 1,
            flags: None,
            removed: false,
            start: 0,
            end: 0,
        };
        let row = rows::VariantChangeRow {
            sequence: 1,
            hour: 7,
            variant: 1,
            scalar: Some(rows::VariantScalarChange {
                display_name: Some(b"saved".to_vec()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let rows = [row];
        collector_test_support::with_allocations_denied(|| apply_to(&mut held, &[event], &rows))
            .unwrap();
        assert!(Arc::ptr_eq(&held[&1].1, &previous));
        let row = rows::VariantChangeRow {
            sequence: 2,
            hour: 7,
            variant: 1,
            scalar: Some(rows::VariantScalarChange {
                auto_upgraded: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        };
        apply_to(&mut held, &[event], &[row]).unwrap();
        assert_eq!(held[&1].1.auto_upgraded, 1);
        assert_eq!(held[&1].1.manpower, 99);
        assert!(Arc::ptr_eq(&held[&2].1, &previous));
    }
    #[test]
    fn variant_held_values_replace_only_touched_designs_and_refuse_unknown_flags() {
        let original = Arc::new(rows::VariantDesignState {
            manpower: 9,
            ..Default::default()
        });
        let quiet = Arc::clone(&original);
        let mut held = HashMap::from([(1, (100, original)), (2, (200, Arc::clone(&quiet)))]);
        collector_test_support::with_allocations_denied(|| apply_to(&mut held, &[], &[])).unwrap();
        assert!(Arc::ptr_eq(&held[&1].1, &quiet));
        let event = Event {
            kind: Kind::Design,
            sequence: 1,
            hour: 7,
            pointer: 100,
            variant: 1,
            flags: Some([1, 2, 3]),
            removed: false,
            start: 0,
            end: 0,
        };
        let row = rows::VariantChangeRow {
            sequence: 1,
            hour: 7,
            variant: 1,
            flags: Some([1, 2, 3]),
            ..Default::default()
        };
        apply_to(&mut held, &[event], std::slice::from_ref(&row)).unwrap();
        assert_eq!(held[&1].1.can_upgrade_modules, 3);
        assert_eq!(held[&1].1.manpower, 9);
        assert!(Arc::ptr_eq(&held[&2].1, &quiet));
        assert!(!Arc::ptr_eq(&held[&1].1, &quiet));
        assert!(
            apply_to(
                &mut held,
                &[Event {
                    pointer: 101,
                    ..event
                }],
                std::slice::from_ref(&row)
            )
            .is_err()
        );
        assert!(
            apply_to(
                &mut held,
                &[Event {
                    variant: 3,
                    ..event
                }],
                &[rows::VariantChangeRow {
                    variant: 3,
                    ..row.clone()
                }]
            )
            .is_err()
        );
        apply_to(
            &mut held,
            &[Event {
                removed: true,
                ..event
            }],
            &[rows::VariantChangeRow {
                removed: true,
                flags: None,
                ..row
            }],
        )
        .unwrap();
        assert!(!held.contains_key(&1));
        assert!(Arc::ptr_eq(&held[&2].1, &quiet));
    }
}
