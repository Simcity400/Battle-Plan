use engine_native_memory::Memory;
use engine_naval_history::cnaval_accident_report as native;
use replay_data::rows::NavalAccidentReportRow;
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
struct Held {
    certificate: Option<(u64, u64)>,
    failure: Option<&'static str>,
    records: Vec<native::Record>,
    len: usize,
}
impl Held {
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
    entries: HashMap<usize, Held>,
    lifetime: Option<u64>,
    retained_bytes: u64,
    charge: collector_primitives::raw_memory::Charge,
}
impl Cache {
    pub fn prepare(&mut self, lifetime: Option<u64>, demand: usize) -> Result<(), &'static str> {
        if lifetime != self.lifetime {
            self.entries.clear();
            self.lifetime = lifetime;
            self.retained_bytes = 0;
        }
        lifetime.ok_or("naval accident world lifetime unavailable")?;
        self.entries
            .try_reserve(demand.saturating_sub(self.entries.len()))
            .map_err(|_| "naval accident cache allocation failed")?;
        Ok(())
    }
    pub fn country(
        &mut self,
        memory: &impl Memory,
        base: usize,
        navy: usize,
        country: u32,
        certificate: Option<(u64, u64)>,
        mut emit: impl FnMut(NavalAccidentReportRow) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        self.lifetime
            .ok_or("naval accident world lifetime unavailable")?;
        let held = self.entries.entry(navy).or_default();
        let old_capacity = held.records.capacity();
        let result = held.refresh(certificate, |records| {
            native::capture(memory, base, navy, records)
        });
        self.retained_bytes = self
            .retained_bytes
            .saturating_sub((old_capacity * size_of::<native::Record>()) as u64)
            .saturating_add((held.records.capacity() * size_of::<native::Record>()) as u64);
        self.charge.set(collector_primitives::raw_memory::Capacity {
            index_map_entries_lower_bound: (self.entries.capacity() * size_of::<(usize, Held)>())
                as u64,
            auxiliary_vectors: self.retained_bytes,
            ..Default::default()
        });
        result?;
        let held = self
            .entries
            .get(&navy)
            .ok_or("naval accident held receiver absent")?;
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
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

#[derive(Default)]
pub struct Captured;
impl Captured {
    pub fn prepare(&mut self, lifetime: Option<u64>, demand: usize) -> Result<(), &'static str> {
        CACHE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get_or_insert_with(Cache::default)
            .prepare(lifetime, demand)
    }
    pub fn country(
        &mut self,
        memory: &impl Memory,
        base: usize,
        navy: usize,
        country: u32,
        certificate: Option<(u64, u64)>,
        emit: impl FnMut(NavalAccidentReportRow) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        CACHE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get_or_insert_with(Cache::default)
            .country(memory, base, navy, country, certificate, emit)
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
