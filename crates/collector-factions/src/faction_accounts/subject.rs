use super::Account;
use collector_observers::country_input_changes as changes;
use engine_allocation as allocation;
use engine_native::{
    Memory, ccountry, modifier_accumulator, modifier_accumulator::Value,
    modifier_definition::Operands,
};
const SUBJECT_GAIN: u32 = 0x288;
const POPULATION_GROWTH: u32 = 0x48;
const KEYS: [u32; 2] = [SUBJECT_GAIN, POPULATION_GROWTH];
type Certificate = (u64, u64);
type Raw = [Result<Value, &'static str>; 2];
type Signature = Option<(u32, u32, u32)>;
struct Held {
    country: ccountry::Country,
    certificate: Certificate,
    raw: Raw,
}
#[derive(Default)]
pub(super) struct Store {
    held: Vec<Held>,
    demanded: Vec<u32>,
    next_demanded: Vec<u32>,
    metadata: [Signature; 2],
    quiet: Option<Certificate>,
    member_order: Vec<u32>,
    has_present: [bool; 2],
    ready: bool,
}
fn signature(value: &Result<Operands, &'static str>) -> Signature {
    value
        .as_ref()
        .ok()
        .map(|v| (v.flags, v.selector, v.eligibility_flags))
}
fn demanded_account(demanded: &[u32], account: &Account) -> bool {
    demanded.binary_search(&account.account).is_ok()
}
impl Store {
    pub fn allocated_bytes(&self) -> usize {
        self.held.capacity() * size_of::<Held>()
            + (self.demanded.capacity() + self.next_demanded.capacity()) * 4
            + self.member_order.capacity() * 4
    }
    fn raw(
        &mut self,
        memory: &impl Memory,
        country: ccountry::Country,
        sequence: &impl Fn(usize) -> Option<Certificate>,
    ) -> Raw {
        let Some(before) = sequence(country.pointer) else {
            return [Err("faction countries' totals could not be followed in the game"); 2];
        };
        let at = self
            .held
            .binary_search_by_key(&country.canonical_slot, |held| held.country.canonical_slot);
        if let Ok(at) = at {
            let old = &self.held[at];
            if old.country == country && old.certificate == before {
                return old.raw;
            }
        }
        let raw = KEYS.map(|key| {
            country
                .pointer
                .checked_add(ccountry::MODIFIER_ACCUMULATOR)
                .ok_or("faction country accumulator address overflow")
                .and_then(|at| modifier_accumulator::lookup_key_at(memory, at, key))
        });
        if sequence(country.pointer) != Some(before) {
            return [Err("faction country accumulator changed during lookup"); 2];
        }
        let held = Held {
            country,
            certificate: before,
            raw,
        };
        match at {
            Ok(at) => self.held[at] = held,
            Err(at) => {
                if allocation::reserve(&mut self.held, 1).is_err() {
                    return [Err("faction country carry allocation failed"); 2];
                }
                self.held.insert(at, held);
            }
        }
        raw
    }
    pub fn invalidate(&mut self) {
        self.quiet = None;
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        members: &[engine_native::cfaction_system::MemberRow],
        accounts: &mut [Account],
    ) -> bool {
        let before = changes::accumulator_quiet();
        let mut metadata = std::array::from_fn::<_, 2, _>(|i| {
            self.has_present[i]
                .then(|| engine_native::modifier_definition::operands(memory, base, KEYS[i]))
        });
        let current = std::array::from_fn(|i| metadata[i].as_ref().and_then(signature));
        if before == self.quiet
            && self.ready
            && current == self.metadata
            && members
                .iter()
                .map(|row| row.country)
                .eq(self.member_order.iter().copied())
        {
            return false;
        }
        let order_additional = members.len().saturating_sub(self.member_order.len());
        let order_ok = allocation::reserve(&mut self.member_order, order_additional).is_ok();
        self.member_order.clear();
        if order_ok {
            self.member_order
                .extend(members.iter().map(|row| row.country));
        }
        self.ready = order_ok;
        self.has_present = [false; 2];
        self.next_demanded.clear();
        let countries = (!members.is_empty() && before.is_some())
            .then(|| ccountry::Reader::new(memory, base, state));
        if !members.is_empty() && !matches!(countries.as_ref(), Some(Ok(_))) {
            self.ready = false;
        }
        let demand_ok = allocation::reserve(&mut self.next_demanded, members.len()).is_ok();
        if demand_ok {
            if let Some(Ok(reader)) = &countries {
                for member in members {
                    match reader.resolve(member.country) {
                        Ok(country) => self.next_demanded.push(country.canonical_slot),
                        Err(_) => self.ready = false,
                    }
                }
            }
            self.next_demanded.sort_unstable();
            self.next_demanded.dedup();
        }
        self.ready &= demand_ok;
        let mut changed = false;
        for &slot in &self.demanded {
            if self.next_demanded.binary_search(&slot).is_err()
                && let Some(account) = accounts.get_mut(slot as usize)
            {
                changed |= account.contribution_subject_gain.take().is_some();
                changed |= account.population_growth.take().is_some();
            }
        }
        std::mem::swap(&mut self.demanded, &mut self.next_demanded);
        changes::accumulator_demand(members.len());
        self.held.retain(|held| {
            self.demanded
                .binary_search(&held.country.canonical_slot)
                .is_ok()
        });
        for at in 0..self.demanded.len() {
            let Some(account) = accounts.get_mut(self.demanded[at] as usize) else {
                self.ready = false;
                continue;
            };
            let next = (|| {
                if !demand_ok || before.is_none() || !demanded_account(&self.demanded, account) {
                    return [None; 2];
                }
                let Some(country) = countries
                    .as_ref()
                    .and_then(|reader| reader.as_ref().ok())
                    .and_then(|reader| reader.resolve(account.country).ok())
                    .filter(|country| country.canonical_slot == account.account)
                else {
                    self.ready = false;
                    return [None; 2];
                };
                let raw = self.raw(memory, country, &changes::accumulator_sequence);
                std::array::from_fn(|i| match raw[i] {
                    Ok(raw) if raw.present => {
                        self.has_present[i] = true;
                        metadata[i]
                            .get_or_insert_with(|| {
                                engine_native::modifier_definition::operands(memory, base, KEYS[i])
                            })
                            .as_ref()
                            .ok()
                            .map(|operands| operands.effective(raw.raw))
                    }
                    Ok(_) => Some(0),
                    Err(why) => {
                        if matches!(
                            why,
                            "faction countries' totals could not be followed in the game"
                                | "faction country accumulator changed during lookup"
                                | "faction country carry allocation failed"
                        ) {
                            self.ready = false;
                        }
                        None
                    }
                })
            })();
            changed |= account.contribution_subject_gain != next[0]
                || account.population_growth != next[1];
            account.contribution_subject_gain = next[0];
            account.population_growth = next[1];
        }
        self.quiet = before.filter(|_| before == changes::accumulator_quiet());
        if self.quiet.is_none() {
            for &slot in &self.demanded {
                if let Some(account) = accounts.get_mut(slot as usize) {
                    changed |= account.contribution_subject_gain.take().is_some();
                    changed |= account.population_growth.take().is_some();
                }
            }
        }
        self.metadata = std::array::from_fn(|i| metadata[i].as_ref().and_then(signature));
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_account_and_member_aliases_demand_and_carry_both_country_keys() {
        let bytes = super::super::tests::alias_fixture();
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
        let account = Account {
            account: 3,
            country: 5,
            ..Default::default()
        };
        let member = reader.resolve(9).unwrap();
        let owner = reader.resolve(account.country).unwrap();
        assert_eq!(member, owner);
        assert!(demanded_account(&[member.canonical_slot], &account));
        assert!(!demanded_account(&[4], &account));
        let registry = collector_certificates::watch_registry::WatchRegistry::enabled(16);
        let sequence = |pointer| registry.sequence(pointer);
        let mut store = Store::default();
        let first = store.raw(&memory, owner, &sequence);
        assert_eq!(first[0].unwrap().raw, -17_000);
        assert_eq!(first[1].unwrap().raw, 25_000);
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("quiet alias carry read memory")
            }
        }
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(store.raw(&NoReads, member, &sequence), first);
        });
        let mut changed = bytes.clone();
        changed[0x6008..0x6010].copy_from_slice(&(-25_000i64).to_le_bytes());
        let changed = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &changed,
        };
        drop(registry.notify(owner.pointer));
        let next = store.raw(&changed, owner, &sequence);
        assert_eq!(next[0], first[0]);
        assert_eq!(next[1].unwrap().raw, -25_000);
    }
    #[test]
    fn point_keys_carry_without_reads_and_retry_only_their_writer() {
        struct Counted {
            bytes: Vec<u8>,
            reads: std::cell::Cell<usize>,
            fail: std::cell::Cell<bool>,
        }
        impl Memory for Counted {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                !self.fail.get()
                    && engine_native::memory::SliceMemory {
                        base: 0,
                        bytes: &self.bytes,
                    }
                    .copy(at, out)
            }
        }
        let country = ccountry::Country {
            pointer: 0x1000,
            canonical_slot: 3,
            id: 7,
        };
        let mut bytes = vec![0; 0x3000];
        let accumulator = country.pointer + ccountry::MODIFIER_ACCUMULATOR;
        bytes[accumulator..accumulator + 8].copy_from_slice(&0x2000u64.to_le_bytes());
        bytes[accumulator + modifier_accumulator::COUNT
            ..accumulator + modifier_accumulator::COUNT + 4]
            .copy_from_slice(&1i32.to_le_bytes());
        bytes[0x2000..0x2004].copy_from_slice(&SUBJECT_GAIN.to_le_bytes());
        bytes[0x2008..0x2010].copy_from_slice(&(-17i64).to_le_bytes());
        let memory = Counted {
            bytes,
            reads: std::cell::Cell::new(0),
            fail: std::cell::Cell::new(false),
        };
        let registry = collector_certificates::watch_registry::WatchRegistry::enabled(16);
        let sequence = |pointer| registry.sequence(pointer);
        let mut store = Store::default();
        assert_eq!(
            store.raw(&memory, country, &sequence)[0].unwrap(),
            Value {
                raw: -17,
                present: true
            }
        );
        let reads = memory.reads.get();
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(store.raw(&memory, country, &sequence)[0].unwrap().raw, -17);
        });
        assert_eq!(memory.reads.get(), reads);
        drop(registry.notify(country.pointer));
        memory.fail.set(true);
        assert!(store.raw(&memory, country, &sequence)[0].is_err());
        let reads = memory.reads.get();
        memory.fail.set(false);
        assert!(store.raw(&memory, country, &sequence)[0].is_err());
        assert_eq!(memory.reads.get(), reads);
        drop(registry.notify(country.pointer));
        assert_eq!(store.raw(&memory, country, &sequence)[0].unwrap().raw, -17);
    }
}
