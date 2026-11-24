use super::*;
use engine_native_parts::ccountry_database;
use engine_native::cgame_state;
use engine_native_memory::{memory::SliceMemory, msvc_string};

const STATE: usize = 0x100;
const DATABASE: usize = 0x800;
pub(super) const ARRAYS: usize = 0x1000;
const COUNTRY_BYTES: usize = 0x1600;
const DIPLOMACY_BYTES: usize = 0x300;
const SUBJECTS_PER_COUNTRY: usize = 3;

pub(super) struct World {
    pub(super) bytes: Vec<u8>,
    pub(super) origin: usize,
    pub(super) module: usize,
    pub(super) countries: usize,
}

impl World {
    pub(super) fn new(countries: usize, populated: bool) -> Self {
        let slots = countries + 1;
        let links = ARRAYS + slots * 8;
        let tags = links + slots * 4;
        let objects = (tags + slots * msvc_string::STRIDE + 7) & !7;
        let diplomacy = objects + countries * COUNTRY_BYTES;
        let subjects = diplomacy + countries * DIPLOMACY_BYTES;
        let mut bytes = vec![0; subjects + countries * SUBJECTS_PER_COUNTRY * 4];
        let origin = bytes.as_ptr() as usize;
        let module = origin
            .checked_sub(ccountry_database::SINGLETON_RVA.0)
            .unwrap();
        let put = |bytes: &mut [u8], offset: usize, value: &[u8]| {
            bytes[offset..offset + value.len()].copy_from_slice(value);
        };
        let array = |bytes: &mut [u8], offset: usize, data: usize, count: usize| {
            put(bytes, offset, &((origin + data) as u64).to_le_bytes());
            put(bytes, offset + 8, &(count as u32).to_le_bytes());
            put(bytes, offset + 12, &(count as u32).to_le_bytes());
        };
        put(&mut bytes, 0, &((origin + DATABASE) as u64).to_le_bytes());
        put(
            &mut bytes,
            DATABASE + ccountry_database::DYNAMIC_TAG_BOUNDARY,
            &(slots as u32).to_le_bytes(),
        );
        array(&mut bytes, STATE + cgame_state::COUNTRIES, ARRAYS, slots);
        array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, links, slots);
        array(&mut bytes, STATE + cgame_state::COUNTRY_TAGS, tags, slots);
        for slot in 0..slots {
            put(
                &mut bytes,
                tags + slot * msvc_string::STRIDE + msvc_string::CAPACITY,
                &msvc_string::INLINE_CAPACITY.to_le_bytes(),
            );
        }
        for index in 0..countries {
            let slot = index + 1;
            let country = objects + index * COUNTRY_BYTES;
            let diplomacy = diplomacy + index * DIPLOMACY_BYTES;
            put(
                &mut bytes,
                ARRAYS + slot * 8,
                &((origin + country) as u64).to_le_bytes(),
            );
            put(&mut bytes, links + slot * 4, &(slot as u32).to_le_bytes());
            put(
                &mut bytes,
                country,
                &((module + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                country + ccountry::ID,
                &(slot as u32).to_le_bytes(),
            );
            put(
                &mut bytes,
                country + ccountry::DIPLOMACY_STATUS,
                &((origin + diplomacy) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy,
                &((module + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            for offset in [
                ccountry::COSMETIC_TAG,
                ccountry::GRAPHICAL_CULTURE,
                ccountry::GRAPHICAL_CULTURE_2D,
                ccountry::OLD_NAME,
                ccountry::OLD_ADJECTIVE,
                ccountry::OLD_DEFINITE_NAME,
            ] {
                put(
                    &mut bytes,
                    country + offset + msvc_string::CAPACITY,
                    &msvc_string::INLINE_CAPACITY.to_le_bytes(),
                );
            }
            let payload = subjects + index * SUBJECTS_PER_COUNTRY * 4;
            array(
                &mut bytes,
                diplomacy + engine_native_parts::cdiplomacy_status_layout::SUBJECTS,
                payload,
                if populated { SUBJECTS_PER_COUNTRY } else { 0 },
            );
            for (entry, value) in [slot as i32, -1, slot as i32].into_iter().enumerate() {
                put(&mut bytes, payload + entry * 4, &value.to_le_bytes());
            }
        }
        Self {
            bytes,
            origin,
            module,
            countries,
        }
    }

    pub(super) fn state(&self) -> usize {
        self.origin + STATE
    }

    pub(super) fn capacities(&self) -> Capacities {
        Capacities {
            rows: self.countries,
            subjects: self.countries * SUBJECTS_PER_COUNTRY,
            naming_bytes: 0,
            ..Default::default()
        }
    }

    pub(super) fn read(&self, captured: &mut Captured) -> Vec<CountryAppearanceRow> {
        captured.reset();
        captured.capture(
            &SliceMemory {
                base: self.origin,
                bytes: &self.bytes,
            },
            self.module,
            self.state(),
        );
        rows(captured)
    }
}

pub(super) fn rows(captured: &Captured) -> Vec<CountryAppearanceRow> {
    let [
        TableView::Complete(replay_data::rows::TableRows::CountryAppearance(rows)),
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete appearance");
    };
    rows
}

