use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{collect_vec, copy_vec, owned_vec};
use engine_native::{Memory, cnaval_base, walk::Outcome};
use replay_data::rows::{NavalBaseRow, TableId, tables};
use replay_frame::frame::{TableInput, TableView};

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    countries: usize,
}

collector_primitives::capacity_max!(Capacities { rows, countries });

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let demand = cnaval_base::count(memory, base, state);
        let rows = collector_extended_support::demand::rows(demand.outcome)?;
        crate::country_scale::raise(&mut self.rows, rows);
        crate::country_scale::raise(&mut self.countries, demand.required_countries);
        Ok(())
    }
}

pub struct Captured {
    management: collector_naval_management::bases::Captured,
    owned: Option<collector_naval_management::bases::Owned>,
    rows: CaptureRows<cnaval_base::Row>,
    countries: Arena<u32>,
    country_tags: Vec<Option<Vec<u8>>>,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            management: Default::default(),
            owned: None,
            rows: CaptureRows::new(c.rows),
            countries: Arena::new(c.countries),
            country_tags: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.discard_owned();
        self.management = Default::default();
        self.rows.reset();
        self.countries.reset();
        self.country_tags.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.management
            .tables
            .iter()
            .filter_map(|table| table.as_ref().ok())
            .map(|rows| rows.allocated_bytes())
            .sum::<usize>()
            + self.rows.allocated_bytes()
            + self.countries.allocated_bytes()
            + self.country_tags.capacity() * std::mem::size_of::<Option<Vec<u8>>>()
            + self
                .country_tags
                .iter()
                .flatten()
                .map(Vec::capacity)
                .sum::<usize>()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.rows);
        self.countries.grow(&mut c.countries);
    }

    /// # Safety
    /// Only the admitted naval-base capture (or an isolated fixture owning the global naval-base
    /// management store) may walk the authenticated image and update that store.
    pub unsafe fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        // SAFETY: Caller guarantees admitted naval-base capture ownership of the global store.
        unsafe { self.capture_impl(memory, base, state, None) }
    }

    /// # Safety
    /// Only the admitted naval-base capture (or an isolated fixture owning the global naval-base
    /// management store) may walk the authenticated image and take an owned publication loan, which
    /// must be decoded or discarded before reusing this frame or detaching.
    pub unsafe fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        stopped: fn() -> bool,
    ) -> [u64; N] {
        // SAFETY: Caller guarantees admitted naval-base capture ownership and loan settlement.
        unsafe { self.capture_impl(memory, base, state, Some(stopped)) }
    }

    /// # Safety
    /// Caller holds admitted naval-base capture ownership of the global naval-base store.
    unsafe fn capture_impl(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        stopped: Option<fn() -> bool>,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.reset();
        let mut management = match stopped {
            Some(stopped) => {
                // SAFETY: Caller guarantees admitted naval-base capture ownership and loan settlement.
                unsafe {
                    collector_naval_management::bases::Session::begin_owned(memory, base, stopped)
                }
            }
            None => {
                // SAFETY: Caller guarantees admitted naval-base capture ownership of the global store.
                unsafe { collector_naval_management::bases::Session::begin(memory, base) }
            }
        };
        let walked = cnaval_base::walk_observed(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.countries.first,
            |pointer, row| management.visit(pointer, row),
        );
        if stopped.is_some() {
            (self.management, self.owned) = management.finish_owned(walked.outcome);
        } else {
            self.management = management.finish(walked.outcome);
        }
        self.rows.outcome = walked.outcome;
        self.countries.len = walked.countries;
        self.countries.required = walked.required_countries;
        self.country_tags = owned_vec(collect_vec(
            self.countries.first[..walked.countries]
                .iter()
                .map(|&country| cnaval_base::country_text(memory, base, state, country).ok()),
        ));
        [qpc().saturating_sub(begin).max(0) as u64, 0, 0]
    }

    pub fn decode_owned(&mut self) {
        if let Some(owned) = self.owned.take() {
            self.management = owned.decode();
        }
    }

    pub fn discard_owned(&mut self) {
        self.owned = None;
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        let [bases, queues] = self.management.outcomes();
        [self.rows.outcome, bases, queues]
    }

    fn base_table(&self) -> TableView {
        let outcome = self.outcomes()[0];
        let Some(countries) = self.countries.first.get(..self.countries.len) else {
            return unavailable("naval base country arena length invalid");
        };
        let Some(rows) = self.rows.rows.get(..outcome.len()) else {
            return unavailable("walker row count exceeds allocated buffer");
        };
        if self.country_tags.len() != countries.len() {
            return unavailable("naval base country tag arena length invalid");
        }
        if rows.iter().any(|r| {
            r.countries_start
                .checked_add(r.countries_len as usize)
                .is_none_or(|end| end > countries.len())
        }) {
            return unavailable("naval base country list lies outside captured arena");
        }
        table::<tables::NavalBases, _>(outcome, rows, |r| NavalBaseRow {
            province: r.province,
            country: r.country,
            level: r.level,
            healthy_level: r.healthy_level,
            countries: owned_vec(copy_vec(
                &countries[r.countries_start..r.countries_start + r.countries_len as usize],
            )),
            countries_unreadable: r.countries_unreadable,
            country_tags: replay_data::rows::Recorded::Value(
                owned_vec(collect_vec(
                    self.country_tags
                        [r.countries_start..r.countries_start + r.countries_len as usize]
                        .iter()
                        .map(|tag| tag.as_ref().map(|tag| owned_vec(copy_vec(tag)))),
                ))
                .into_iter()
                .collect(),
            ),
        })
    }

    pub fn table_inputs(&self) -> [TableInput; N] {
        let [bases, queue] = self.management.inputs();
        [self.base_table().into(), bases, queue]
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs().map(|input| match input {
            TableInput::Sampled(view) => view,
            TableInput::SharedComplete(rows) => TableView::Complete((*rows).clone()),
            _ => unreachable!("no comparison input"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::memory::SliceMemory;

    #[test]
    fn cold_storage_publishes_first_and_later_mod_sized_base_headers() {
        use engine_native::{cgame_state, rail_anchor_selector as map};
        let mut capture = Captured::with_capacities(&Capacities::default());
        assert_eq!(capture.allocated_bytes(), 0);
        let game_state = 0x1000;
        let table = 0x2000;
        let holder = 0x3000;
        let buckets = 0x4000;
        for (count, list_len) in [(5_000_usize, 2_usize), (5_000, 37), (12_000, 37)] {
            let capacity = count.next_power_of_two();
            let bases = buckets + (capacity + 1) * 0x18;
            let list = bases + count * cnaval_base::SIZE;
            let tags = list + list_len * 4;
            let mut bytes = vec![0; tags + (list_len + 1) * engine_native::msvc_string::STRIDE];
            let mut put = |at: usize, value: &[u8]| {
                bytes[at..at + value.len()].copy_from_slice(value);
            };
            put(
                game_state + cgame_state::STRATEGIC_NAVY_MANAGER,
                &(table as u64).to_le_bytes(),
            );
            put(table + map::BUCKETS, &(buckets as u64).to_le_bytes());
            put(table + map::MASK, &((capacity - 1) as u32).to_le_bytes());
            put(table + map::MAX_PROBE, &[0]);
            put(holder + cnaval_base::HOLDER_COUNTRY, &1_u32.to_le_bytes());
            let tag_header = game_state + cgame_state::COUNTRY_TAGS;
            put(tag_header, &(tags as u64).to_le_bytes());
            for offset in [8, 12] {
                put(tag_header + offset, &((list_len + 1) as u32).to_le_bytes());
            }
            for index in 0..list_len {
                put(list + index * 4, &(index as u32 + 1).to_le_bytes());
                let text = format!("T{}", index + 1);
                let tag = tags + (index + 1) * engine_native::msvc_string::STRIDE;
                put(tag, text.as_bytes());
                put(tag + 16, &(text.len() as u64).to_le_bytes());
                put(tag + 24, &15_u64.to_le_bytes());
            }
            for index in 0..count {
                let base = bases + index * cnaval_base::SIZE;
                let entry = buckets + index * 0x18;
                put(entry + 4, &[1]);
                put(entry + 8, &(index as u32 + 1).to_le_bytes());
                put(entry + 0x10, &(base as u64).to_le_bytes());
                put(base, &(cnaval_base::VTABLE_RVA.0 as u64).to_le_bytes());
                put(base + cnaval_base::HOLDER, &(holder as u64).to_le_bytes());
                put(
                    base + cnaval_base::PROVINCE_ID,
                    &(index as u32 + 1).to_le_bytes(),
                );
                put(
                    base + cnaval_base::COUNTRY_LIST,
                    &(list as u64).to_le_bytes(),
                );
                for offset in [8, 12] {
                    put(
                        base + cnaval_base::COUNTRY_LIST + offset,
                        &(list_len as u32).to_le_bytes(),
                    );
                }
            }
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut hint = Capacities::default();
            hint.presize(&memory, 0, game_state).unwrap();
            assert_eq!(hint.rows, count);
            assert_eq!(hint.countries, count * list_len);
            // SAFETY: synthetic memory fixture with no concurrent naval-base census in this test.
            unsafe { capture.capture(&memory, 0, game_state) };
            assert_eq!(
                capture.outcomes(),
                [
                    Outcome::Rows { len: count },
                    Outcome::Unavailable {
                        reason: collector_naval_management::changes::UNAVAILABLE,
                    },
                    Outcome::Unavailable {
                        reason: collector_naval_management::changes::UNAVAILABLE,
                    },
                ]
            );
            assert_eq!(capture.countries.len, count * list_len);
            assert_eq!(capture.countries.len, capture.countries.required);
            assert!(
                capture.rows.rows[..count]
                    .iter()
                    .all(|row| row.countries_len == list_len as u32)
            );
            let [
                TableView::Complete(replay_data::rows::TableRows::NavalBases(rows)),
                _,
                _,
            ] = capture.tables()
            else {
                panic!("readable naval bases were not published");
            };
            let expected = replay_data::rows::Recorded::Value(Some(
                (1..=list_len)
                    .map(|index| format!("T{index}").into_bytes())
                    .collect(),
            ));
            assert_eq!(rows.len(), count);
            assert!(rows.iter().all(|row| row.country_tags == expected));
        }
    }

    #[test]
    fn unreadable_registrations_are_unavailable_and_reset_clears_them() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities::default());
        // SAFETY: synthetic memory fixture with no concurrent naval-base census in this test.
        unsafe { capture.capture(&memory, 0, 0x1000) };
        assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }
}
