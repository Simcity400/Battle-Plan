//! Each country's DynamicModifiers rows, held between the Governance job's
//! hourly walks ([`crate::world_input_changes`]). A country's rows are
//! carried, without reading its container, while it is the held country
//! (same id and CCountry) and its certificate is the one taken before the
//! held read: the dynamic modifier domain's sequence (add, renew, remove and
//! the daily update notify the container's country) joined to the naval tree
//! family's (`modifier recalculation`, the only country caller of the value
//! recalculation, and `country appearance reset`, the only country caller of
//! the clear). The daily update runs for every country, so every country is
//! read once a day after it; an event reads only the countries it touched.
//! Each country is read afresh and compared with its held rows on its day of
//! the sampled rotation (by id), and `world_input_verify` compares every
//! carryable country; a difference poisons the domain, and every country is
//! read from then on. A read is held only when the certificate stood still
//! across it; a failed job forgets the store.
use crate::{
    sampled_verify::Check,
    world_input_changes::{self as changes, Changes, Domain},
};
use engine::{ccountry, cdynamic_modifier_container as container};
use std::sync::{Mutex, MutexGuard, TryLockError};

type Certificate = [(u64, u64); 2];

/// One country's latest complete read: its rows with token and value
/// offsets relative to its own arenas.
struct Held {
    country: u32,
    pointer: usize,
    certificate: Certificate,
    rows: Vec<container::Row>,
    tokens: Vec<u8>,
    values: Vec<i64>,
    /// The walk that last saw the country.
    seen: u64,
}

/// Held countries, ascending by id.
pub(crate) struct Store {
    held: Vec<Held>,
    walks: u64,
}

static STORE: Mutex<Store> = Mutex::new(Store {
    held: Vec::new(),
    walks: 0,
});

/// One walk's use of the store: what each country does and the walk's counts.
pub(crate) struct Walk<'a, C: Changes + ?Sized> {
    store: MutexGuard<'a, Store>,
    changes: &'a C,
    check: Check,
    carried: u64,
    read: u64,
    compared: u64,
    mismatches: u64,
}

/// One country's plan: its certificate before the read, and whether its
/// held rows may be published.
pub(crate) struct Plan {
    certificate: Option<Certificate>,
    pub(crate) carry: bool,
}

impl<'a, C: Changes + ?Sized> Walk<'a, C> {
    /// The production store, or `None` while another walk holds it.
    pub(crate) fn begin(changes: &'a C) -> Option<Self> {
        Self::begin_in(
            &STORE,
            changes,
            crate::sampled_verify::current_burned_in(changes::VERIFY),
        )
    }

    pub(crate) fn begin_in(store: &'a Mutex<Store>, changes: &'a C, check: Check) -> Option<Self> {
        let mut store = match store.try_lock() {
            Ok(store) => store,
            // A walk that panicked left it: hold nothing it held.
            Err(TryLockError::Poisoned(poisoned)) => {
                let mut store = poisoned.into_inner();
                store.held.clear();
                store
            }
            Err(TryLockError::WouldBlock) => return None,
        };
        store.walks += 1;
        Some(Self {
            store,
            changes,
            check,
            carried: 0,
            read: 0,
            compared: 0,
            mismatches: 0,
        })
    }

    fn find(&self, country: u32) -> Result<usize, usize> {
        self.store
            .held
            .binary_search_by_key(&country, |held| held.country)
    }

    /// Before `country`'s container is read.
    pub(crate) fn plan(&mut self, country: ccountry::Country) -> Plan {
        let certificate = self.changes.country(country.pointer);
        let walk = self.store.walks;
        let carry = match self.find(country.id) {
            Ok(index) => {
                let held = &mut self.store.held[index];
                held.seen = walk;
                certificate.is_some()
                    && held.pointer == country.pointer
                    && Some(held.certificate) == certificate
                    && !self.check.covers(u64::from(country.id))
            }
            Err(_) => false,
        };
        Plan { certificate, carry }
    }

    /// A carrying plan's rows, appended at `first`, `text` and `values`
    /// (their offsets stay relative, as a walk's); the walk's lengths.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn carry(
        &mut self,
        country: ccountry::Country,
        rows: &mut Vec<container::Row>,
        first: usize,
        tokens: &mut Vec<u8>,
        text: usize,
        values: &mut Vec<i64>,
        value_start: usize,
    ) -> container::Walk {
        let index = self
            .find(country.id)
            .expect("a carrying plan names a held country");
        let held = &self.store.held[index];
        let grown = (|| {
            use engine::walk::Buffer::Growing;
            Growing(&mut *rows).ensure(first + held.rows.len())?;
            Growing(&mut *tokens).ensure(text + held.tokens.len())?;
            Growing(&mut *values).ensure(value_start + held.values.len())?;
            Ok::<(), &'static str>(())
        })();
        if let Err(reason) = grown {
            return container::Walk {
                outcome: engine::walk::Outcome::Unavailable { reason },
                token_bytes: 0,
                values_len: 0,
                required_token_bytes: held.tokens.len(),
                required_values: held.values.len(),
            };
        }
        rows[first..first + held.rows.len()].copy_from_slice(&held.rows);
        tokens[text..text + held.tokens.len()].copy_from_slice(&held.tokens);
        values[value_start..value_start + held.values.len()].copy_from_slice(&held.values);
        self.carried += 1;
        container::Walk {
            outcome: engine::walk::Outcome::Rows {
                len: held.rows.len(),
            },
            token_bytes: held.tokens.len(),
            values_len: held.values.len(),
            required_token_bytes: held.tokens.len(),
            required_values: held.values.len(),
        }
    }

    /// After a fresh read of `country`: its rows, token bytes and values, when
    /// the read completed. A country whose held rows the plan could have
    /// carried is compared with them; the read is held when the certificate
    /// stood still across it.
    pub(crate) fn read(
        &mut self,
        country: ccountry::Country,
        plan: Plan,
        read: Option<(&[container::Row], &[u8], &[i64])>,
    ) {
        self.read += 1;
        let Some((rows, tokens, values)) = read else {
            return;
        };
        let at = self.find(country.id);
        if let Ok(index) = at {
            let held = &self.store.held[index];
            if plan.certificate.is_some()
                && held.pointer == country.pointer
                && Some(held.certificate) == plan.certificate
            {
                self.compared += 1;
                if held.rows != rows || held.tokens != tokens || held.values != values {
                    self.mismatches += 1;
                    self.changes.mismatch(Domain::DynamicModifiers);
                }
            }
        }
        let held = plan
            .certificate
            .filter(|&certificate| self.changes.country(country.pointer) == Some(certificate))
            .and_then(|certificate| {
                Some(Held {
                    country: country.id,
                    pointer: country.pointer,
                    certificate,
                    rows: copy(rows)?,
                    tokens: copy(tokens)?,
                    values: copy(values)?,
                    seen: self.store.walks,
                })
            });
        match (at, held) {
            (Ok(index), Some(held)) => self.store.held[index] = held,
            (Ok(index), None) => {
                self.store.held.remove(index);
            }
            (Err(index), Some(held)) => {
                if engine::allocation::reserve(&mut self.store.held, 1).is_ok() {
                    self.store.held.insert(index, held);
                }
            }
            (Err(_), None) => {}
        }
    }

    /// The walk ended; `census` when it visited every live country, whose
    /// unvisited held countries are then gone.
    pub(crate) fn finish(mut self, census: bool) {
        if census {
            let walk = self.store.walks;
            self.store.held.retain(|held| held.seen == walk);
        }
        changes::record_countries(self.carried, self.read, self.compared, self.mismatches);
    }
}

/// An owned copy; `None` when it cannot be allocated, and nothing is held.
fn copy<T: Copy>(values: &[T]) -> Option<Vec<T>> {
    let mut copied = Vec::new();
    engine::allocation::reserve(&mut copied, values.len()).ok()?;
    copied.extend_from_slice(values);
    Some(copied)
}

impl Store {
    #[cfg(test)]
    pub(crate) const fn new() -> Self {
        Self {
            held: Vec::new(),
            walks: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_input_changes::Isolated;

    fn row(days: i32) -> container::Row {
        container::Row {
            country: 7,
            days_remaining: days,
            token_len: 3,
            value_len: 1,
            ..container::Row::default()
        }
    }

    /// A country none of its writers touched is carried; a write to its
    /// container reads it afresh, a read across a write is not held, and the
    /// country's day of the rotation compares a read with its held rows,
    /// poisoning the domain at a difference.
    #[test]
    fn a_country_is_carried_until_a_writer_touches_its_container() {
        let changes = Isolated::new();
        let store = Mutex::new(Store::new());
        let country = ccountry::Country {
            pointer: 0x7000,
            canonical_slot: 3,
            id: 7,
        };
        let hour = |check: Check, rows: &[container::Row], mid_read: bool| {
            let mut walk = Walk::begin_in(&store, &changes, check).unwrap();
            let plan = walk.plan(country);
            let carried = plan.carry.then(|| {
                let (mut out, mut tokens, mut values) = (Vec::new(), Vec::new(), Vec::new());
                walk.carry(country, &mut out, 0, &mut tokens, 0, &mut values, 0);
                out
            });
            if carried.is_none() {
                if mid_read {
                    changes.write_country(country.pointer);
                }
                walk.read(country, plan, Some((rows, b"abc", &[5])));
            }
            walk.finish(true);
            carried
        };
        let quiet = Check::Slice { hour: 1 };
        assert_eq!(hour(quiet, &[row(3)], false), None, "first read");
        assert_eq!(hour(quiet, &[row(9)], false), Some(vec![row(3)]));
        // The daily update decremented its days: read afresh.
        changes.write_country(country.pointer);
        assert_eq!(hour(quiet, &[row(2)], false), None);
        assert_eq!(hour(quiet, &[row(9)], false), Some(vec![row(2)]));
        // A writer ran during the read: published, not held.
        changes.write_country(country.pointer);
        assert_eq!(hour(quiet, &[row(1)], true), None);
        assert_eq!(hour(quiet, &[row(1)], false), None);
        assert_eq!(hour(quiet, &[row(9)], false), Some(vec![row(1)]));
        // Country 7's day of the rotation: read and compared.
        let day = Check::Slice { hour: 7 };
        assert_eq!(hour(day, &[row(1)], false), None);
        assert!(!changes.poisoned(Domain::DynamicModifiers));
        assert_eq!(hour(day, &[row(4)], false), None);
        assert!(changes.poisoned(Domain::DynamicModifiers));
        assert_eq!(hour(quiet, &[row(4)], false), None, "poisoned: always read");
    }
}
