//! CountryLocalSubunits rows held between hours
//! ([`crate::local_subunit_changes`]).
//!
//! CountryLocalSubunits, which the 3D model selectors read, is walked every
//! hour. A country's rows record its deployment status's clones
//! (in the deployment status) whose requirements differ from their
//! global definition's; every run-time writer of the clone vector and of a
//! clone's requirements is a hooked site, and a status is replaced only
//! through its destructor's hooked clear
//! ([`engine_mutations::local_subunit_mutations`]). A country's held rows are carried,
//! without reading its clones, when its country object is the held walk's
//! and either
//! - the domain's quiet token equals the one taken before the held walk, so
//!   no hooked writer ran since: nothing of the country is read; or
//! - no hooked writer touched its deployment status since the held walk
//!   registered it (same epoch and sequence), and the status and clone vector
//!   header read this hour are the held ones.
//!
//! Every other country is read afresh. A walk is held only when it was
//! complete and the quiet token stood still across it, so no certificate
//! taken during it can postdate a write it missed. Allocation is advisory: a
//! walk that cannot be held is read again next hour.
use crate::local_subunit_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_deployment::{
    cdeployment_status as native, cdeployment_status::Deployment,
    cdeployment_status::HeldLocalSubunits, cdeployment_status::LocalRequirement,
    cdeployment_status::LocalSelection, cdeployment_status::LocalSubunitRow,
    cdeployment_status::LocalSubunitWalk,
};
use engine_native::walk::Outcome;
use std::{cell::Cell, ops::Range};

/// One country of the held walk.
#[derive(Clone, Debug)]
struct HeldCountry {
    pointer: usize,
    slot: u32,
    id: u32,
    deployment: Deployment,
    /// The status's certificate; `None` when it could not be registered.
    certificate: Option<(u64, u64)>,
    rows: Range<usize>,
}

/// The latest complete walk, with the quiet token taken before it.
#[derive(Default)]
pub struct LocalStore {
    quiet: Option<(u64, u64)>,
    countries: Vec<HeldCountry>,
    rows: Vec<LocalSubunitRow>,
    requirements: Vec<LocalRequirement>,
    /// This walk's countries, swapped in when the walk is held.
    next: Vec<HeldCountry>,
}

impl LocalStore {
    /// Nothing held certifies anything.
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

    /// Holds a complete walk taken under `quiet`, with its countries in
    /// `next`; storage is advisory, so a walk that cannot be held is read
    /// again next hour.
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

/// The rows of `country`, whose requirement spans index `requirements`.
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

/// One hour's CountryLocalSubunits into `rows` and `requirements`, carrying
/// what the store and the domain certify. Records the hour's counts.
pub fn capture(
    memory: &impl engine_native::Memory,
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
    // Held countries are in walk order, so one cursor finds each one's entry.
    let cursor = Cell::new(0);
    // The walked country's held entry.
    let current = Cell::new(None::<usize>);
    let mut stored = Ok(());
    let walk = native::walk_local_subunits_selected_growing(
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
    // Held only when no hooked writer ran across the walk.
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
