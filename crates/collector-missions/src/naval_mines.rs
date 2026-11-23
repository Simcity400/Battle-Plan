use engine_native_memory::{Memory, cpdx_array::Array};
use engine_native_naval::naval_mine_inputs as native;
use replay_data::rows::NavalRegionalMineRow;
use std::sync::{Mutex, MutexGuard};

type Token = (u64, u64);
#[derive(Default)]
struct Part {
    certificate: Option<Token>,
    rows: Vec<(u32, i64)>,
    len: usize,
    failure: Option<&'static str>,
}
struct Held {
    navy: usize,
    quiet: Option<Token>,
    lifecycle: Option<Token>,
    header: Result<Array, &'static str>,
    parts: [Part; native::DECAY_SLICES],
}
impl Default for Held {
    fn default() -> Self {
        Self {
            navy: 0,
            quiet: None,
            lifecycle: None,
            header: Err("regional mine header not acquired"),
            parts: std::array::from_fn(|_| Part::default()),
        }
    }
}
impl Held {
    fn reset_for(&mut self, navy: usize) {
        if self.navy != navy {
            self.navy = navy;
            self.quiet = None;
            self.lifecycle = None;
            self.header = Err("regional mine header not acquired");
            for part in &mut self.parts {
                part.certificate = None;
                part.len = 0;
                part.failure = None;
            }
        }
    }
    fn refresh(
        &mut self,
        quiet: Token,
        lifecycle: Token,
        tokens: impl Fn(usize) -> Option<Token>,
        header: impl FnOnce() -> Result<Array, &'static str>,
        mut read: impl FnMut(Array, usize, &mut Vec<(u32, i64)>) -> Result<usize, &'static str>,
        validate: impl FnOnce() -> bool,
    ) -> Result<(), &'static str> {
        if self.quiet == Some(quiet) {
            return self.result();
        }
        let changed_lifecycle = self.lifecycle != Some(lifecycle);
        let mut current = [None; native::DECAY_SLICES];
        for (part, token) in current.iter_mut().enumerate() {
            *token = Some(tokens(part).ok_or("regional mine slice certificate unavailable")?);
        }
        if changed_lifecycle {
            self.header = header();
        }
        if let Ok(source) = self.header {
            for (index, part) in self.parts.iter_mut().enumerate() {
                if changed_lifecycle || part.certificate != current[index] {
                    part.len = 0;
                    part.failure = match read(source, index, &mut part.rows) {
                        Ok(len) => {
                            part.len = len;
                            None
                        }
                        Err(reason) => Some(reason),
                    };
                }
            }
        }
        if !validate() {
            for (part, token) in self.parts.iter_mut().zip(current) {
                if changed_lifecycle || part.certificate != token {
                    part.certificate = None;
                }
            }
            if changed_lifecycle {
                self.lifecycle = None;
            }
            self.quiet = None;
            return Err("regional mines changed during acquisition");
        }
        self.lifecycle = Some(lifecycle);
        for (part, token) in self.parts.iter_mut().zip(current) {
            part.certificate = token;
        }
        self.quiet = Some(quiet);
        self.result()
    }
    fn result(&self) -> Result<(), &'static str> {
        self.header?;
        self.parts
            .iter()
            .find_map(|part| part.failure)
            .map_or(Ok(()), Err)
    }
    fn allocated_bytes(&self) -> usize {
        self.parts
            .iter()
            .map(|part| part.rows.capacity() * size_of::<(u32, i64)>())
            .sum()
    }
}

#[derive(Default)]
struct Cache {
    entries: Vec<Held>,
    lifetime: Option<u64>,
    row_bytes: usize,
    charge: collector_primitives::raw_memory::Charge,
}
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

pub struct Session<'a> {
    guard: MutexGuard<'a, Option<Cache>>,
    quiet: Token,
    dirty: bool,
}

impl Session<'_> {
    pub fn country(
        &mut self,
        memory: &impl Memory,
        slot: usize,
        navy: usize,
        country: u32,
        mut emit: impl FnMut(NavalRegionalMineRow) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let cache = self
            .guard
            .as_mut()
            .ok_or("regional mine cache not prepared")?;
        if slot >= cache.entries.len() {
            let additional = slot + 1 - cache.entries.len();
            cache
                .entries
                .try_reserve(additional)
                .map_err(|_| "regional mine cache allocation failed")?;
            cache.entries.resize_with(slot + 1, Held::default);
            self.dirty = true;
        }
        let held = &mut cache.entries[slot];
        held.reset_for(navy);
        if held.quiet == Some(self.quiet) {
            held.result()?;
        } else {
            let lifecycle = crate::naval_map_changes::mine_lifetime(navy)
                .ok_or("regional mine lifetime certificate unavailable")?;
            let previous = held.allocated_bytes();
            let quiet = self.quiet;
            let result = held.refresh(
                quiet,
                lifecycle,
                |part| crate::naval_map_changes::mine_slice(navy, part),
                || native::header(memory, navy),
                |source, part, rows| native::capture_slice(memory, source, part, rows),
                || crate::naval_map_changes::mine_quiet() == Some(quiet),
            );
            cache.row_bytes = cache
                .row_bytes
                .saturating_sub(previous)
                .saturating_add(held.allocated_bytes());
            self.dirty = true;
            result?;
        }
        let held = &cache.entries[slot];
        emit(NavalRegionalMineRow {
            country,
            kind: 0,
            region: held.header?.count,
            amount: 0,
        })?;
        for part in &held.parts {
            for &(region, amount) in &part.rows[..part.len] {
                emit(NavalRegionalMineRow {
                    country,
                    kind: 1,
                    region,
                    amount,
                })?;
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), &'static str> {
        if self.dirty
            && let Some(cache) = self.guard.as_mut()
        {
            cache.charge.set(collector_primitives::raw_memory::Capacity {
                index_vectors: (cache.entries.capacity() * size_of::<Held>()) as u64,
                auxiliary_vectors: cache.row_bytes as u64,
                ..Default::default()
            });
        }
        if crate::naval_map_changes::mine_quiet() != Some(self.quiet) {
            return Err("regional mines changed during publication");
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
        let mut guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
        let cache = guard.get_or_insert_with(Cache::default);
        let mut dirty = false;
        if cache.lifetime != lifetime {
            cache.entries.clear();
            cache.row_bytes = 0;
            cache.lifetime = lifetime;
            dirty = true;
        }
        lifetime.ok_or("regional mine world lifetime unavailable")?;
        let quiet = crate::naval_map_changes::mine_quiet()
            .ok_or("regional mine writer certificate unavailable")?;
        if cache.entries.len() < demand {
            cache
                .entries
                .try_reserve(demand - cache.entries.len())
                .map_err(|_| "regional mine cache allocation failed")?;
            cache.entries.resize_with(demand, Held::default);
            dirty = true;
        }
        Ok(Session {
            guard,
            quiet,
            dirty,
        })
    }
}
pub fn release() {
    *CACHE.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_and_unrelated_changes_never_poll_values_or_header() {
        let mut held = Held::default();
        held.refresh(
            (1, 0),
            (1, 0),
            |_| Some((1, 0)),
            || {
                Ok(Array {
                    data: 10,
                    count: 49,
                })
            },
            |_, part, rows| {
                rows.push((part as u32, -7));
                Ok(1)
            },
            || true,
        )
        .unwrap();
        held.refresh(
            (1, 0),
            (1, 0),
            |_| panic!("quiet token lookup"),
            || panic!("quiet header"),
            |_, _, _| panic!("quiet value read"),
            || true,
        )
        .unwrap();
        held.refresh(
            (1, 1),
            (1, 0),
            |_| Some((1, 0)),
            || panic!("unrelated header"),
            |_, _, _| panic!("unrelated value read"),
            || true,
        )
        .unwrap();
        let mut reads = Vec::new();
        held.refresh(
            (1, 2),
            (1, 0),
            |part| Some((1, u64::from(part == 9))),
            || panic!("slice header"),
            |_, part, _| {
                reads.push(part);
                Ok(0)
            },
            || true,
        )
        .unwrap();
        assert_eq!(reads, [9]);
        assert_eq!(held.parts[9].len, 0);
        assert_eq!(held.parts[8].rows[0].1, -7);
    }
    #[test]
    fn failed_slice_carries_until_its_own_writer_changes() {
        let mut held = Held::default();
        assert_eq!(
            held.refresh(
                (1, 0),
                (1, 0),
                |_| Some((1, 0)),
                || Ok(Array {
                    data: 10,
                    count: 49
                }),
                |_, part, _| if part == 3 {
                    Err("synthetic slice fault")
                } else {
                    Ok(0)
                },
                || true
            ),
            Err("synthetic slice fault")
        );
        assert_eq!(
            held.refresh(
                (1, 1),
                (1, 0),
                |_| Some((1, 0)),
                || panic!("header polled"),
                |_, _, _| panic!("failed slice polled"),
                || true
            ),
            Err("synthetic slice fault")
        );
        let mut reads = Vec::new();
        held.refresh(
            (1, 2),
            (1, 0),
            |part| Some((1, u64::from(part == 3))),
            || panic!("header polled"),
            |_, part, _| {
                reads.push(part);
                Ok(0)
            },
            || true,
        )
        .unwrap();
        assert_eq!(reads, [3]);
    }
    #[test]
    fn active_writer_cannot_commit_rows_under_an_old_certificate() {
        let mut held = Held::default();
        assert!(
            held.refresh(
                (1, 0),
                (1, 0),
                |_| Some((1, 0)),
                || Ok(Array {
                    data: 10,
                    count: 49
                }),
                |_, _, _| Ok(0),
                || false
            )
            .is_err()
        );
        assert_eq!(held.lifecycle, None);
        assert!(held.parts.iter().all(|part| part.certificate.is_none()));
        let mut reads = 0;
        held.refresh(
            (1, 1),
            (1, 0),
            |_| Some((1, 0)),
            || {
                Ok(Array {
                    data: 10,
                    count: 49,
                })
            },
            |_, _, _| {
                reads += 1;
                Ok(0)
            },
            || true,
        )
        .unwrap();
        assert_eq!(reads, native::DECAY_SLICES);
    }
    #[test]
    fn lifecycle_changes_reread_even_zero_slices_and_invalid_headers_do_not_poll() {
        let mut held = Held::default();
        assert!(
            held.refresh(
                (1, 0),
                (1, 0),
                |_| Some((1, 0)),
                || Err("synthetic header fault"),
                |_, _, _| panic!("invalid header read values"),
                || true
            )
            .is_err()
        );
        assert!(
            held.refresh(
                (1, 1),
                (1, 0),
                |_| Some((1, 0)),
                || panic!("failed header polled"),
                |_, _, _| panic!("failed header values"),
                || true
            )
            .is_err()
        );
        let mut reads = 0;
        held.refresh(
            (1, 2),
            (1, 1),
            |_| Some((1, 0)),
            || Ok(Array { data: 10, count: 0 }),
            |_, _, _| {
                reads += 1;
                Ok(0)
            },
            || true,
        )
        .unwrap();
        assert_eq!(reads, native::DECAY_SLICES);
    }
}
