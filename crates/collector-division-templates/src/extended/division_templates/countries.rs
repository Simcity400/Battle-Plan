use super::*;
use crate::template_changes::Changes;
use std::sync::Arc;

#[derive(Default, Clone)]
pub(super) struct Entry {
    country: u32,
    pointer: usize,
    row: Option<CountryRow>,
    manager: usize,
    config: Option<[(u64, u64); 3]>,
    config_attempted: bool,
    config_valid: bool,
    occupation: Option<usize>,
    garrison: Option<[(u64, u64); 2]>,
}

impl Entry {
    fn allocated_bytes(&self) -> usize {
        self.row
            .as_ref()
            .and_then(|row| row.garrisons.as_ref())
            .map_or(0, |rows| {
                rows.capacity() * size_of::<GarrisonTemplateSelection>()
            })
    }
}

pub(super) struct Visit {
    country: engine_native_memory::country_identity::Country,
    entry: Entry,
    result: Result<(CountryRow, bool), &'static str>,
}

#[derive(Default)]
pub(super) struct Store {
    entries: Vec<Entry>,
    order: Vec<u32>,
    live: Vec<u32>,
    cursor: usize,
    payload_bytes: usize,
    pub changed: bool,
    scratch: Vec<GarrisonTemplateSelection>,
}

fn config(changes: &dyn Changes, country: u32, manager: usize) -> Option<[(u64, u64); 3]> {
    Some([
        changes.sequence(country as usize)?,
        if manager == 0 {
            (0, 0)
        } else {
            changes.sequence(manager)?
        },
        if manager == 0 {
            (0, 0)
        } else {
            changes.deployment_reset_sequence(manager)?
        },
    ])
}

fn garrison_certificate(
    changes: &dyn Changes,
    occupation: Option<usize>,
    country: u32,
) -> Option<[(u64, u64); 2]> {
    Some([
        changes.sequence(occupation?)?,
        changes.sequence(country as usize)?,
    ])
}

impl Store {
    pub fn forget(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.payload_bytes = 0;
    }
    pub fn begin(&mut self) {
        self.changed = false;
        self.cursor = 0;
    }
    pub fn finish(&mut self) {
        self.changed |= self.order.len() != self.cursor;
        self.order.truncate(self.cursor);
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.order.capacity() * size_of::<u32>()
            + self.live.capacity() * size_of::<u32>()
            + self.scratch.capacity() * size_of::<GarrisonTemplateSelection>()
            + self.payload_bytes
    }

    pub fn prune(&mut self) {
        if self.entries.len() <= self.order.len() {
            return;
        }
        self.live.clear();
        if engine_allocation::reserve(&mut self.live, self.order.len()).is_err() {
            return;
        }
        self.live.extend_from_slice(&self.order);
        self.live.sort_unstable();
        self.entries.retain(|entry| {
            let keep = self.live.binary_search(&entry.country).is_ok();
            if !keep {
                self.payload_bytes -= entry.allocated_bytes();
            }
            keep
        });
    }

    pub fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        country: engine_native_memory::country_identity::Country,
        changes: &dyn Changes,
    ) -> Result<CountryRow, &'static str> {
        let mut scratch = std::mem::take(&mut self.scratch);
        let visit = self.observe(memory, base, state, country, changes, &mut scratch);
        self.scratch = scratch;
        self.commit(visit)
    }

    pub fn observe(
        &self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        country: engine_native_memory::country_identity::Country,
        changes: &dyn Changes,
        scratch: &mut Vec<GarrisonTemplateSelection>,
    ) -> Visit {
        let mut entry = match self
            .entries
            .binary_search_by_key(&country.id, |entry| entry.country)
        {
            Ok(at) => self.entries[at].clone(),
            Err(_) => Entry {
                country: country.id,
                ..Default::default()
            },
        };
        let result = read_entry(&mut entry, scratch, memory, base, state, country, changes);
        Visit {
            country,
            entry,
            result,
        }
    }

    pub fn commit(&mut self, visit: Visit) -> Result<CountryRow, &'static str> {
        let id = visit.country.id;
        if self.order.get(self.cursor) != Some(&id) {
            self.changed = true;
            if self.cursor == self.order.len() {
                engine_allocation::reserve(&mut self.order, 1)
                    .map_err(|_| "country template membership allocation failed")?;
                self.order.push(id);
            } else {
                self.order[self.cursor] = id;
            }
        }
        self.cursor += 1;
        let index = match self
            .entries
            .binary_search_by_key(&id, |entry| entry.country)
        {
            Ok(at) => at,
            Err(at) => {
                engine_allocation::reserve(&mut self.entries, 1)
                    .map_err(|_| "country template cache allocation failed")?;
                self.entries.insert(
                    at,
                    Entry {
                        country: id,
                        ..Default::default()
                    },
                );
                at
            }
        };
        let previous = self.entries[index].allocated_bytes();
        self.entries[index] = visit.entry;
        self.payload_bytes = self.payload_bytes - previous + self.entries[index].allocated_bytes();
        let (row, changed) = visit.result?;
        self.changed |= changed;
        Ok(row)
    }
}

impl Visit {
    pub fn garrison_failure(&self) -> Option<&'static str> {
        self.result
            .as_ref()
            .ok()
            .and_then(|(row, _)| row.garrison_failure)
    }
}

fn read_entry(
    entry: &mut Entry,
    scratch: &mut Vec<GarrisonTemplateSelection>,
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: engine_native_memory::country_identity::Country,
    changes: &dyn Changes,
) -> Result<(CountryRow, bool), &'static str> {
    {
        if entry.pointer != country.pointer {
            *entry = Entry {
                country: country.id,
                pointer: country.pointer,
                ..Default::default()
            };
        }
        let mut row = entry.row.clone().unwrap_or_default();
        let fresh = changes.fresh_only();
        let current = config(changes, country.id, entry.manager);
        if current.is_none() && !fresh {
            entry.config_attempted = false;
            entry.config_valid = false;
            return Err("country template writer certificate unavailable");
        }
        let read_config = fresh || !entry.config_attempted || current != entry.config;
        let mut occupation_changed = false;
        if read_config {
            entry.config_attempted = true;
            entry.config_valid = false;
            entry.config = current;
            let manager = country
                .pointer
                .checked_add(engine_native::ccountry::DEPLOYMENT_MANAGER)
                .and_then(|at| engine_native_memory::read::<u64>(memory, at))
                .and_then(|at| usize::try_from(at).ok())
                .ok_or("deployment status pointer unreadable or null")?;
            entry.manager = manager;
            let before = config(changes, country.id, manager);
            if before.is_none() && !fresh {
                entry.config_attempted = false;
                return Err("country template writer certificate unavailable");
            }
            entry.config = before;
            row.state =
                engine_deployment::cdeployment_status::template_state(memory, base, country)?;
            if manager != row.state.manager
                || (!fresh && before != config(changes, country.id, manager))
            {
                entry.config_attempted = false;
                return Err("country template configuration changed during acquisition");
            }
            entry.config_valid = true;
            let occupation = country
                .pointer
                .checked_add(engine_native::ccountry::OCCUPATION_STATUS)
                .and_then(|at| engine_native_memory::read::<u64>(memory, at))
                .and_then(|at| usize::try_from(at).ok())
                .filter(|&at| at != 0);
            occupation_changed = occupation != entry.occupation;
            entry.occupation = occupation;
        } else if current.is_none() {
            return Err("country template writer certificate unavailable");
        } else if !entry.config_valid {
            return Err(
                "country template configuration acquisition refused until a writer change or refresh",
            );
        }
        let garrison = garrison_certificate(changes, entry.occupation, country.id);
        let read_garrison = fresh || (garrison.is_some()
            && (entry.row.is_none()
                || entry.garrison.is_none()
                || occupation_changed
                || garrison != entry.garrison));
        if read_garrison {
            let read = (|| {
                if garrison.is_none() && !fresh {
                    return Err("garrison template writer certificate unavailable");
                }
                let priority = engine_occupation::ccountry_occupation_status::template_priority(
                    memory,
                    base,
                    country.pointer,
                )?;
                let countries = engine_native::ccountry::Reader::new(memory, base, state)?;
                scratch.clear();
                engine_occupation::ccountry_occupation_status::visit_template_selections(
                    memory,
                    base,
                    country,
                    &countries,
                    |selection| {
                        engine_allocation::reserve(&mut *scratch, 1)
                            .map_err(|_| "garrison template cache allocation failed")?;
                        scratch.push(GarrisonTemplateSelection {
                            occupied: selection.occupied,
                            state: selection.state,
                            template: selection.template,
                        });
                        Ok(())
                    },
                )?;
                if !fresh && garrison != garrison_certificate(changes, entry.occupation, country.id) {
                    return Err("garrison template selections changed during acquisition");
                }
                if let Some(held) = row
                    .garrisons
                    .as_ref()
                    .filter(|held| held.as_slice() == scratch.as_slice())
                {
                    return Ok((Arc::clone(held), priority));
                }
                let rows = engine_allocation::copy_vec(scratch.as_slice())
                    .map_err(|_| "garrison template projection allocation failed")?;
                Ok::<_, &'static str>((Arc::new(rows), priority))
            })();
            match read {
                Ok((rows, priority)) => {
                    row.garrison_priority = Some(priority);
                    row.garrisons = Some(rows);
                    row.garrison_failure = None;
                }
                Err(reason) => {
                    row.garrison_priority = None;
                    row.garrisons = None;
                    row.garrison_failure = Some(reason);
                }
            }
            entry.garrison = if garrison.is_some()
                && garrison == garrison_certificate(changes, entry.occupation, country.id)
            {
                garrison
            } else {
                None
            };
        } else if garrison.is_none() {
            row.garrison_priority = None;
            entry.garrison = None;
            row.garrisons = None;
            row.garrison_failure = Some("garrison template writer certificate unavailable");
        }
        let changed = entry.row.as_ref() != Some(&row);
        entry.row = Some(row.clone());
        Ok((row, changed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template_changes::Watches;

    #[test]
    fn first_country_acquisition_waits_for_a_valid_certificate_without_reading() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("unobserved country configuration must not read native state")
            }
        }
        let watches = Watches::enabled(16);
        let mut store = Store::default();
        let country = engine_native_memory::country_identity::Country {
            id: 1,
            pointer: 0x1000,
            canonical_slot: 1,
        };
        let running = watches.notify(1, 0);
        assert!(store.read(&Unreadable, 0, 0, country, &watches).is_err());
        assert!(!store.entries[0].config_attempted);
        drop(running);
        assert!(config(&watches, 1, 0).is_some());
        let running = watches.notify(0x2000, 0);
        assert!(garrison_certificate(&watches, Some(0x2000), 1).is_none());
        drop(running);
        assert!(garrison_certificate(&watches, Some(0x2000), 1).is_some());
    }

    #[test]
    fn template_configuration_requires_the_independent_deployment_reset_certificate() {
        struct Sources {
            template: Watches,
            reset: Watches,
        }
        impl Changes for Sources {
            fn quiet(&self) -> Option<(u64, u64)> {
                self.template.quiet()
            }
            fn sequence(&self, object: usize) -> Option<(u64, u64)> {
                self.template.sequence(object)
            }
            fn deployment_reset_sequence(&self, status: usize) -> Option<(u64, u64)> {
                self.reset.sequence(status)
            }
        }
        let sources = Sources {
            template: Watches::enabled(16),
            reset: Watches::enabled(16),
        };
        let before = config(&sources, 1, 0x1000).unwrap();
        let quiet = sources.quiet();
        let reset = sources.reset.notify(0x1000, 0);
        assert!(config(&sources, 1, 0x1000).is_none());
        assert_eq!(sources.quiet(), quiet);
        drop(reset);
        assert_ne!(config(&sources, 1, 0x1000), Some(before));
        assert_eq!(sources.quiet(), quiet);
    }
}
