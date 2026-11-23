use crate::local_subunit_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_deployment::{
    cdeployment_status as native, cdeployment_status::Deployment,
    cdeployment_status::HeldLocalSubunits, cdeployment_status::LocalRequirement,
    cdeployment_status::LocalSelection, cdeployment_status::LocalSubunitRow,
    cdeployment_status::LocalSubunitWalk,
};
use engine_native_memory::walk::Outcome;
use std::{cell::Cell, ops::Range};

#[derive(Clone, Debug)]
struct HeldCountry {
    pointer: usize,
    slot: u32,
    id: u32,
    deployment: Deployment,
    certificate: Option<(u64, u64)>,
    rows: Range<usize>,
}

#[derive(Default)]
pub struct LocalStore {
    quiet: Option<(u64, u64)>,
    countries: Vec<HeldCountry>,
    rows: Vec<LocalSubunitRow>,
    requirements: Vec<LocalRequirement>,
    next: Vec<HeldCountry>,
}

impl LocalStore {
    pub fn forget(&mut self) {
        self.quiet = None;
        self.countries.clear();
        self.rows.clear();
        self.requirements.clear();
        self.next.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        (self.countries.capacity() + self.next.capacity()) * size_of::<HeldCountry>()
            + self.rows.capacity() * size_of::<LocalSubunitRow>()
            + self.requirements.capacity() * size_of::<LocalRequirement>()
    }

    fn hold(
        &mut self,
        quiet: (u64, u64),
        rows: &[LocalSubunitRow],
        requirements: &[LocalRequirement],
    ) {
        let copied = allocation::advisory(|| {
            self.rows.clear();
            self.requirements.clear();
            allocation::reserve(&mut self.rows, rows.len())?;
            allocation::reserve(&mut self.requirements, requirements.len())
        });
        if copied.is_err() {
            self.forget();
            return;
        }
        self.rows.extend_from_slice(rows);
        self.requirements.extend_from_slice(requirements);
        std::mem::swap(&mut self.countries, &mut self.next);
        self.next.clear();
        self.quiet = Some(quiet);
    }
}

fn held<'a>(
    country: &HeldCountry,
    rows: &'a [LocalSubunitRow],
    requirements: &'a [LocalRequirement],
) -> HeldLocalSubunits<'a> {
    HeldLocalSubunits {
        rows: &rows[country.rows.clone()],
        requirements,
    }
}

pub fn capture(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
    store: &mut LocalStore,
    changes: &dyn Changes,
    rows: &mut Vec<LocalSubunitRow>,
    requirements: &mut Vec<LocalRequirement>,
) -> LocalSubunitWalk {
    let before = changes.quiet();
    let quiet = before.is_some() && before == store.quiet;
    let mut counts = Counts {
        quiet,
        ..Counts::default()
    };
    let LocalStore {
        countries: held_countries,
        rows: held_rows,
        requirements: held_requirements,
        next,
        ..
    } = &mut *store;
    let (held_countries, held_rows, held_requirements) =
        (&*held_countries, &*held_rows, &*held_requirements);
    next.clear();
    let cursor = Cell::new(0);
    let current = Cell::new(None::<usize>);
    let mut stored = Ok(());
    let walk = native::walk_local_subunits_selected(
        memory,
        base,
        game_state,
        rows,
        requirements,
        |country| {
            let mut index = cursor.get();
            while held_countries
                .get(index)
                .is_some_and(|entry| entry.slot < country.canonical_slot)
            {
                index += 1;
            }
            cursor.set(index);
            current.set(None);
            let Some(entry) = held_countries.get(index).filter(|entry| {
                (entry.pointer, entry.slot, entry.id)
                    == (country.pointer, country.canonical_slot, country.id)
            }) else {
                return LocalSelection::Read;
            };
            current.set(Some(index));
            let certified = entry.certificate.is_some()
                && (quiet || entry.certificate == changes.sequence(entry.deployment.status));
            let kept = held(entry, held_rows, held_requirements);
            match (certified, quiet) {
                (false, _) => LocalSelection::Read,
                (true, true) => LocalSelection::Carry(kept),
                (true, false) => LocalSelection::Confirm {
                    deployment: entry.deployment,
                    held: kept,
                },
            }
        },
        |visit| {
            let entry = current.get().map(|index| &held_countries[index]);
            let Some(deployment) = visit.deployment.or(entry.map(|entry| entry.deployment)) else {
                unreachable!("only a held country is carried");
            };
            let certificate = if visit.read {
                counts.read += 1;
                counts.read_rows += visit.rows.len() as u64;
                changes.sequence(deployment.status)
            } else {
                counts.reused += 1;
                counts.reused_rows += visit.rows.len() as u64;
                entry.and_then(|entry| entry.certificate)
            };
            if stored.is_ok() {
                stored = allocation::advisory(|| allocation::reserve(next, 1));
                if stored.is_ok() {
                    next.push(HeldCountry {
                        pointer: visit.country.pointer,
                        slot: visit.country.canonical_slot,
                        id: visit.country.id,
                        deployment,
                        certificate,
                        rows: visit.rows,
                    });
                }
            }
        },
    );
    let complete = match walk.outcome {
        Outcome::Rows { len } => rows
            .get(..len)
            .zip(requirements.get(..walk.requirements_len)),
        _ => None,
    };
    match (complete, before) {
        (Some((walked, walked_requirements)), Some(before))
            if stored.is_ok() && changes.quiet() == Some(before) =>
        {
            store.hold(before, walked, walked_requirements)
        }
        _ => store.forget(),
    }
    crate::local_subunit_changes::record(
        &counts,
        complete.map(|_| (counts.read + counts.reused) as usize),
    );
    walk
}

#[cfg(test)]
mod tests;
