use engine_native_memory::Memory;
use engine_naval_history::cnaval_accident_report as native;
use replay_data::rows::NavalAccidentReportRow;
use std::sync::{Mutex, MutexGuard};

#[derive(Default)]
struct Held {
    navy: usize,
    quiet: Option<(u64, u64)>,
    certificate: Option<(u64, u64)>,
    failure: Option<&'static str>,
    records: Vec<native::Record>,
    len: usize,
}
impl Held {
    fn reset_for(&mut self, navy: usize) {
        if self.navy != navy {
            self.navy = navy;
            self.quiet = None;
            self.certificate = None;
            self.failure = None;
            self.len = 0;
        }
    }
    fn refresh(
        &mut self,
        certificate: Option<(u64, u64)>,
        read: impl FnOnce(&mut Vec<native::Record>) -> Result<usize, &'static str>,
    ) -> Result<(), &'static str> {
        let certificate = certificate.ok_or("naval accident writer certificate unavailable")?;
        if self.certificate != Some(certificate) {
            self.len = 0;
            self.failure = match read(&mut self.records) {
                Ok(len) => {
                    self.len = len;
                    None
                }
                Err(reason) => Some(reason),
            };
            self.certificate = Some(certificate);
        }
        self.failure.map_or(Ok(()), Err)
    }
}

#[derive(Default)]
struct Cache {
    entries: Vec<Held>,
    lifetime: Option<u64>,
    retained_bytes: u64,
    charge: collector_primitives::raw_memory::Charge,
}
impl Cache {
    fn prepare(&mut self, lifetime: Option<u64>, demand: usize) -> Result<bool, &'static str> {
        let mut dirty = false;
        if lifetime != self.lifetime {
            self.entries.clear();
            self.lifetime = lifetime;
            self.retained_bytes = 0;
            dirty = true;
        }
        lifetime.ok_or("naval accident world lifetime unavailable")?;
        if self.entries.len() < demand {
            self.entries
                .try_reserve(demand - self.entries.len())
                .map_err(|_| "naval accident cache allocation failed")?;
            self.entries.resize_with(demand, Held::default);
            dirty = true;
        }
        Ok(dirty)
    }
    fn update_charge(&mut self) {
        self.charge.set(collector_primitives::raw_memory::Capacity {
            index_vectors: (self.entries.capacity() * size_of::<Held>()) as u64,
            auxiliary_vectors: self.retained_bytes,
            ..Default::default()
        });
    }
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

pub struct Session<'a> {
    guard: MutexGuard<'a, Option<Cache>>,
    quiet: Option<(u64, u64)>,
    dirty: bool,
}

impl Session<'_> {
    pub fn country(
        &mut self,
        memory: &impl Memory,
        base: usize,
        slot: usize,
        navy: usize,
        country: u32,
        mut emit: impl FnMut(NavalAccidentReportRow) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let cache = self
            .guard
            .as_mut()
            .ok_or("naval accident cache not prepared")?;
        if slot >= cache.entries.len() {
            let additional = slot + 1 - cache.entries.len();
            cache
                .entries
                .try_reserve(additional)
                .map_err(|_| "naval accident cache allocation failed")?;
            cache.entries.resize_with(slot + 1, Held::default);
            self.dirty = true;
        }
        let held = &mut cache.entries[slot];
        held.reset_for(navy);
        if self.quiet.is_some() && held.quiet == self.quiet {
            if let Some(reason) = held.failure {
                return Err(reason);
            }
        } else {
            let certificate = crate::naval_map_changes::accident_token(navy);
            let old_capacity = held.records.capacity();
            let result = held.refresh(certificate, |records| {
                native::capture(memory, base, navy, records)
            });
            cache.retained_bytes = cache
                .retained_bytes
                .saturating_sub((old_capacity * size_of::<native::Record>()) as u64)
                .saturating_add((held.records.capacity() * size_of::<native::Record>()) as u64);
            self.dirty = true;
            if certificate != crate::naval_map_changes::accident_token(navy) {
                held.certificate = None;
                held.quiet = None;
                return Err("naval accident writer changed during acquisition");
            }
            held.quiet = self.quiet;
            result?;
        }
        let held = &cache.entries[slot];
        for record in &held.records[..held.len] {
            emit(NavalAccidentReportRow {
                country,
                ordinal: record.ordinal,
                kind: record.kind,
                mine_owner: record.mine_owner,
                region: record.region,
                equipment: record.equipment,
                damage: record.damage,
                sunk: record.sunk,
                ship: record.ship,
                owner: record.owner,
                expire_hour: record.expire_hour,
            })?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), &'static str> {
        if self.dirty
            && let Some(cache) = self.guard.as_mut()
        {
            cache.update_charge();
        }
        if let Some(quiet) = self.quiet
            && crate::naval_map_changes::accident_quiet() != Some(quiet)
        {
            return Err("naval accident writer changed during acquisition");
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Captured;
impl Captured {
    pub fn begin(
        &mut self,
        lifetime: Option<u64>,
        demand: usize,
    ) -> Result<Session<'static>, &'static str> {
        let mut guard = CACHE.lock().unwrap_or_else(|poison| poison.into_inner());
        let dirty = guard
            .get_or_insert_with(Cache::default)
            .prepare(lifetime, demand)?;
        let quiet = crate::naval_map_changes::accident_quiet();
        Ok(Session {
            guard,
            quiet,
            dirty,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_reports_and_failures_carry_without_polling() {
        let mut held = Held::default();
        held.refresh(Some((1, 2)), |records| {
            records.push(native::Record {
                damage: -7,
                ..Default::default()
            });
            Ok(1)
        })
        .unwrap();
        held.refresh(Some((1, 2)), |_| panic!("quiet reports polled"))
            .unwrap();
        assert_eq!(held.records[0].damage, -7);
        assert_eq!(
            held.refresh(Some((1, 3)), |_| Err("synthetic fault")),
            Err("synthetic fault")
        );
        assert_eq!(
            held.refresh(Some((1, 3)), |_| panic!("refused reports polled")),
            Err("synthetic fault")
        );
        held.refresh(Some((2, 3)), |_| Ok(0)).unwrap();
        assert_eq!(held.len, 0);
        assert!(
            held.refresh(None, |_| panic!("uncertified report scan"))
                .is_err()
        );
    }
}
