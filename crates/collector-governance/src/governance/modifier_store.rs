use crate::world_input_changes::{self as changes, Changes};
use engine_stability::cdynamic_modifier_container as container;
use std::sync::{Mutex, MutexGuard, TryLockError};

type Certificate = [(u64, u64); 2];

struct Held {
    country: u32,
    pointer: usize,
    certificate: Certificate,
    rows: Vec<container::Row>,
    tokens: Vec<u8>,
    values: Vec<i64>,
    seen: u64,
}

pub struct Store {
    held: Vec<Held>,
    walks: u64,
}

static STORE: Mutex<Store> = Mutex::new(Store {
    held: Vec::new(),
    walks: 0,
});

pub struct Walk<'a, C: Changes + ?Sized> {
    store: MutexGuard<'a, Store>,
    changes: &'a C,
    carried: u64,
    read: u64,
}

pub struct Plan {
    certificate: Option<Certificate>,
    pub carry: bool,
}

impl<'a, C: Changes + ?Sized> Walk<'a, C> {
    pub fn begin(changes: &'a C) -> Option<Self> {
        Self::begin_in(&STORE, changes)
    }

    pub fn begin_in(store: &'a Mutex<Store>, changes: &'a C) -> Option<Self> {
        let mut store = match store.try_lock() {
            Ok(store) => store,
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
            carried: 0,
            read: 0,
        })
    }

    fn find(&self, country: u32) -> Result<usize, usize> {
        self.store
            .held
            .binary_search_by_key(&country, |held| held.country)
    }

    pub fn plan(&mut self, country: engine_native_memory::country_identity::Country) -> Plan {
        let certificate = self.changes.country(country.pointer);
        let walk = self.store.walks;
        let carry = match self.find(country.id) {
            Ok(index) => {
                let held = &mut self.store.held[index];
                held.seen = walk;
                certificate.is_some()
                    && held.pointer == country.pointer
                    && Some(held.certificate) == certificate
            }
            Err(_) => false,
        };
        Plan { certificate, carry }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one country's modifiers carry into separate row, token and value outputs with their starts"
    )]
    pub fn carry(
        &mut self,
        country: engine_native_memory::country_identity::Country,
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
            use engine_native_memory::walk::Buffer::Growing;
            Growing(&mut *rows).ensure(first + held.rows.len())?;
            Growing(&mut *tokens).ensure(text + held.tokens.len())?;
            Growing(&mut *values).ensure(value_start + held.values.len())?;
            Ok::<(), &'static str>(())
        })();
        if let Err(reason) = grown {
            return container::Walk {
                outcome: engine_native_memory::walk::Outcome::Unavailable { reason },
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
            outcome: engine_native_memory::walk::Outcome::Rows {
                len: held.rows.len(),
            },
            token_bytes: held.tokens.len(),
            values_len: held.values.len(),
            required_token_bytes: held.tokens.len(),
            required_values: held.values.len(),
        }
    }

    pub fn read(
        &mut self,
        country: engine_native_memory::country_identity::Country,
        plan: Plan,
        read: Option<(&[container::Row], &[u8], &[i64])>,
    ) {
        self.read += 1;
        let Some((rows, tokens, values)) = read else {
            return;
        };
        let at = self.find(country.id);
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
                if engine_allocation::reserve(&mut self.store.held, 1).is_ok() {
                    self.store.held.insert(index, held);
                }
            }
            (Err(_), None) => {}
        }
    }

    pub fn finish(mut self, census: bool) {
        if census {
            let walk = self.store.walks;
            self.store.held.retain(|held| held.seen == walk);
        }
        changes::record_countries(self.carried, self.read);
    }
}

fn copy<T: Copy>(values: &[T]) -> Option<Vec<T>> {
    let mut copied = Vec::new();
    engine_allocation::reserve(&mut copied, values.len()).ok()?;
    copied.extend_from_slice(values);
    Some(copied)
}

impl Store {
    #[cfg(test)]
    pub const fn new() -> Self {
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

    #[test]
    fn a_country_is_carried_until_a_writer_touches_its_container() {
        let changes = Isolated::new();
        let store = Mutex::new(Store::new());
        let country = engine_native_memory::country_identity::Country {
            pointer: 0x7000,
            canonical_slot: 3,
            id: 7,
        };
        let hour = |rows: &[container::Row], mid_read: bool| {
            let mut walk = Walk::begin_in(&store, &changes).unwrap();
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
        assert_eq!(hour(&[row(3)], false), None, "first read");
        assert_eq!(hour(&[row(9)], false), Some(vec![row(3)]));
        changes.write_country(country.pointer);
        assert_eq!(hour(&[row(2)], false), None);
        assert_eq!(hour(&[row(9)], false), Some(vec![row(2)]));
        changes.write_country(country.pointer);
        assert_eq!(hour(&[row(1)], true), None);
        assert_eq!(hour(&[row(1)], false), None);
        assert_eq!(hour(&[row(9)], false), Some(vec![row(1)]));
    }
}
