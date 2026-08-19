use core::sync::atomic::{AtomicU64, Ordering};

pub const BOUND_TRIP_UNIT_LEADER_SKILL: usize = 0;
pub const BOUND_TRIP_FOCUS_COUNT: usize = 1;
pub const BOUND_TRIP_COMPLETED_TECH_COUNT: usize = 2;
pub const BOUND_TRIP_OWNER_TAG_DB_ID: usize = 3;
pub const BOUND_TRIP_PLAYER_TAG_DB_ID: usize = 4;
pub const BOUND_TRIP_TAG_DB_ID_1024: usize = 5;
pub const BOUND_TRIP_FACTION_SLOTS: usize = 6;
pub const BOUND_TRIP_FACTION_MEMBERS: usize = 7;
pub const BOUND_TRIP_COUNTRY_LINK_INDEX: usize = 8;
pub const BOUND_TRIP_SUPPLY_PROVINCE_ID: usize = 9;
pub const BOUND_TRIP_WEATHER_ARRAY_LEN: usize = 10;
pub const BOUND_TRIP_POLITICAL_PARTY_COUNT: usize = 11;
pub const BOUND_TRIP_MAP_STATE_ID: usize = 12;
pub const BOUND_TRIP_FACTION_GROUP_INDEX: usize = 13;
pub const BOUND_TRIP_MAP_PROVINCE_ID: usize = 14;
pub const BOUND_TRIP_MAP_PROVINCE_SLOTS: usize = 15;
pub const BOUND_TRIP_CSUPPLY_INPUT_LEN: usize = 16;
pub const BOUND_TRIP_ID_COUNT: usize = 17;

pub const BOUND_TRIP_UNITS: [(&str, &str); BOUND_TRIP_ID_COUNT] = [
    (
        "bt_unit_leader_skill_trips",
        "bt_unit_leader_skill_raw_sample",
    ),
    ("bt_focus_count_trips", "bt_focus_count_raw_sample"),
    (
        "bt_completed_tech_count_trips",
        "bt_completed_tech_count_raw_sample",
    ),
    ("bt_owner_tag_db_id_trips", "bt_owner_tag_db_id_raw_sample"),
    (
        "bt_player_tag_db_id_trips",
        "bt_player_tag_db_id_raw_sample",
    ),
    ("bt_tag_db_id_1024_trips", "bt_tag_db_id_1024_raw_sample"),
    ("bt_faction_slots_trips", "bt_faction_slots_raw_sample"),
    ("bt_faction_members_trips", "bt_faction_members_raw_sample"),
    (
        "bt_country_link_index_trips",
        "bt_country_link_index_raw_sample",
    ),
    (
        "bt_supply_province_id_trips",
        "bt_supply_province_id_raw_sample",
    ),
    (
        "bt_weather_array_len_trips",
        "bt_weather_array_len_raw_sample",
    ),
    (
        "bt_political_party_count_trips",
        "bt_political_party_count_raw_sample",
    ),
    ("bt_map_state_id_trips", "bt_map_state_id_raw_sample"),
    (
        "bt_faction_group_index_trips",
        "bt_faction_group_index_raw_sample",
    ),
    ("bt_map_province_id_trips", "bt_map_province_id_raw_sample"),
    (
        "bt_map_province_slots_trips",
        "bt_map_province_slots_raw_sample",
    ),
    (
        "bt_csupply_input_len_trips",
        "bt_csupply_input_len_raw_sample",
    ),
];

#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_ZERO: AtomicU64 = AtomicU64::new(0);
static TRIP_COUNTS: [AtomicU64; BOUND_TRIP_ID_COUNT] = [ATOMIC_ZERO; BOUND_TRIP_ID_COUNT];
static TRIP_LAST_RAW: [AtomicU64; BOUND_TRIP_ID_COUNT] = [ATOMIC_ZERO; BOUND_TRIP_ID_COUNT];
static ACTIVE_FRAME_RESERVATION: AtomicU64 = AtomicU64::new(0);
static NEXT_FRAME_RESERVATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct BoundTripFrameReservation {
    reservation_id: u64,
    snapshot: [(u64, u64); BOUND_TRIP_ID_COUNT],
    active: bool,
}

impl BoundTripFrameReservation {
    pub fn snapshot(&self) -> &[(u64, u64); BOUND_TRIP_ID_COUNT] {
        &self.snapshot
    }

    pub fn commit(mut self) -> Result<(), String> {
        ensure_active_reservation(self.reservation_id)?;
        for (id, (reserved, _)) in self.snapshot.iter().copied().enumerate() {
            let current = TRIP_COUNTS[id].load(Ordering::Acquire);
            if current < reserved {
                return Err(format!(
                    "bound-trip reservation {} counter {} regressed from {} to {}",
                    self.reservation_id, id, reserved, current
                ));
            }
        }
        for (id, (reserved, _)) in self.snapshot.iter().copied().enumerate() {
            if reserved != 0 {
                TRIP_COUNTS[id].fetch_sub(reserved, Ordering::AcqRel);
            }
        }
        release_active_reservation(self.reservation_id)?;
        self.active = false;
        Ok(())
    }

    pub fn rollback(mut self) -> Result<(), String> {
        release_active_reservation(self.reservation_id)?;
        self.active = false;
        Ok(())
    }
}

impl Drop for BoundTripFrameReservation {
    fn drop(&mut self) {
        if self.active {
            let _ = release_active_reservation(self.reservation_id);
            self.active = false;
        }
    }
}

fn ensure_active_reservation(reservation_id: u64) -> Result<(), String> {
    let active = ACTIVE_FRAME_RESERVATION.load(Ordering::Acquire);
    if active != reservation_id {
        return Err(format!(
            "bound-trip reservation mismatch: active {}, requested {}",
            active, reservation_id
        ));
    }
    Ok(())
}

fn release_active_reservation(reservation_id: u64) -> Result<(), String> {
    ACTIVE_FRAME_RESERVATION
        .compare_exchange(reservation_id, 0, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|active| {
            format!(
                "bound-trip reservation release mismatch: active {}, requested {}",
                active, reservation_id
            )
        })
}

#[doc(hidden)]
pub fn lock_registry_for_tests() -> std::sync::MutexGuard<'static, ()> {
    static REGISTRY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn restore_snapshot_for_tests(snapshot: &[(u64, u64); BOUND_TRIP_ID_COUNT]) {
    for (id, (count, last_raw)) in snapshot.iter().copied().enumerate() {
        TRIP_LAST_RAW[id].store(last_raw, Ordering::Relaxed);
        TRIP_COUNTS[id].store(count, Ordering::Release);
    }
}

pub fn reserve_frame_trips() -> Result<BoundTripFrameReservation, String> {
    let reservation_id = NEXT_FRAME_RESERVATION.fetch_add(1, Ordering::Relaxed);
    if reservation_id == 0 {
        return Err("bound-trip reservation id wrapped".into());
    }
    ACTIVE_FRAME_RESERVATION
        .compare_exchange(0, reservation_id, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|active| format!("bound-trip frame reservation already active: {}", active))?;
    let mut snapshot = [(0u64, 0u64); BOUND_TRIP_ID_COUNT];
    for (id, slot) in snapshot.iter_mut().enumerate() {
        let count = TRIP_COUNTS[id].load(Ordering::Acquire);
        let raw = TRIP_LAST_RAW[id].load(Ordering::Relaxed);
        *slot = (count, raw);
    }
    Ok(BoundTripFrameReservation {
        reservation_id,
        snapshot,
        active: true,
    })
}

/// published (Release), and the drain swaps the counter with Acquire before
pub fn record_bound_trip(id: usize, raw: u64) {
    if id >= BOUND_TRIP_ID_COUNT {
        return;
    }
    TRIP_LAST_RAW[id].store(raw, Ordering::Relaxed);
    TRIP_COUNTS[id].fetch_add(1, Ordering::Release);
}

/// lower-bound checks stay at the call site (a zero id is a vacant slot, not
#[inline]
pub fn gate_u32(value: u32, max: u32, id: usize) -> Option<u32> {
    if value <= max {
        Some(value)
    } else {
        record_bound_trip(id, u64::from(value));
        None
    }
}

#[inline]
pub fn bound_exceeded_u32(value: u32, max: u32, id: usize) -> bool {
    if value > max {
        record_bound_trip(id, u64::from(value));
        true
    } else {
        false
    }
}

#[inline]
pub fn tag_db_id_out_of_table(id: u32, table_size: u32) -> bool {
    if crate::country_tag_db::tag_db_id_in_table(id, table_size) {
        false
    } else {
        record_bound_trip(BOUND_TRIP_TAG_DB_ID_1024, u64::from(id));
        true
    }
}

#[inline]
pub fn country_link_index_out_of_vector(index: u32, vector_len: u32) -> bool {
    if index < vector_len {
        false
    } else {
        record_bound_trip(BOUND_TRIP_COUNTRY_LINK_INDEX, u64::from(index));
        true
    }
}

pub fn drain_frame_trips() -> [(u64, u64); BOUND_TRIP_ID_COUNT] {
    let reservation = reserve_frame_trips().expect("bound-trip compatibility drain reservation");
    let out = *reservation.snapshot();
    reservation
        .commit()
        .expect("bound-trip compatibility drain commit");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    fn lock_registry() -> MutexGuard<'static, ()> {
        lock_registry_for_tests()
    }

    #[test]
    fn tag_db_gate_follows_the_callers_table_not_a_constant() {
        let _guard = lock_registry();
        drain_frame_trips();
        let id = BOUND_TRIP_TAG_DB_ID_1024;

        assert!(!tag_db_id_out_of_table(1, 440));
        assert!(!tag_db_id_out_of_table(439, 440));
        assert!(tag_db_id_out_of_table(440, 440), "size is exclusive");
        assert!(
            tag_db_id_out_of_table(1_000, 440),
            "the old 1024 ceiling admitted this"
        );
        assert!(
            !tag_db_id_out_of_table(0, 440),
            "the null country is in range; refusing it is caller policy"
        );
        let drained = drain_frame_trips();
        assert_eq!(
            drained[id].0, 2,
            "440 and 1000 trip; the zero id is a vacant slot, never a bound trip"
        );
        assert_eq!(drained[id].1, 1_000);

        assert!(!tag_db_id_out_of_table(1_025, 12_000));
        assert!(!tag_db_id_out_of_table(11_999, 12_000));
        assert!(tag_db_id_out_of_table(12_000, 12_000));
        let drained = drain_frame_trips();
        assert_eq!(drained[id].0, 1);
        assert_eq!(drained[id].1, 12_000);
    }

    #[test]
    fn a_link_index_never_reports_as_a_country_database_id() {
        let _guard = lock_registry();
        drain_frame_trips();

        // `_CountryLinkTags` grows on every dynamic release, so its live size
        assert!(!country_link_index_out_of_vector(1_500, 2_000));
        assert!(country_link_index_out_of_vector(2_000, 2_000));
        assert!(country_link_index_out_of_vector(9_999, 2_000));

        let drained = drain_frame_trips();
        assert_eq!(drained[BOUND_TRIP_COUNTRY_LINK_INDEX].0, 2);
        assert_eq!(drained[BOUND_TRIP_COUNTRY_LINK_INDEX].1, 9_999);
        assert_eq!(
            drained[BOUND_TRIP_TAG_DB_ID_1024].0, 0,
            "a link-vector failure must never name the country database's bound"
        );
    }

    #[test]
    fn gate_passes_in_range_and_records_over_max() {
        let _guard = lock_registry();
        drain_frame_trips();
        let id = BOUND_TRIP_UNIT_LEADER_SKILL;
        assert_eq!(gate_u32(15, 15, id), Some(15));
        assert_eq!(gate_u32(0, 15, id), Some(0));
        assert_eq!(gate_u32(16, 15, id), None);
        assert_eq!(gate_u32(99, 15, id), None);
        let drained = drain_frame_trips()[id];
        assert_eq!(drained.0, 2);
        assert_eq!(drained.1, 99);
        let drained = drain_frame_trips()[id];
        assert_eq!(drained.0, 0);
        assert_eq!(drained.1, 99);
    }

    #[test]
    fn bound_exceeded_records_only_over_max() {
        let _guard = lock_registry();
        drain_frame_trips();
        let id = BOUND_TRIP_FACTION_SLOTS;
        assert!(!bound_exceeded_u32(512, 512, id));
        assert!(bound_exceeded_u32(513, 512, id));
        let drained = drain_frame_trips()[id];
        assert_eq!(drained, (1, 513));
    }

    #[test]
    fn frame_reservation_rolls_back_or_commits_only_its_prefix() {
        let _guard = lock_registry();
        drain_frame_trips();
        let id = BOUND_TRIP_FACTION_MEMBERS;
        record_bound_trip(id, 600);
        let rolled_back = reserve_frame_trips().unwrap();
        assert_eq!(rolled_back.snapshot()[id], (1, 600));
        record_bound_trip(id, 601);
        rolled_back.rollback().unwrap();
        assert_eq!(drain_frame_trips()[id], (2, 601));

        record_bound_trip(id, 700);
        let committed = reserve_frame_trips().unwrap();
        record_bound_trip(id, 701);
        committed.commit().unwrap();
        assert_eq!(
            drain_frame_trips()[id],
            (1, 701),
            "post-reservation records must remain queued"
        );
    }

    #[test]
    fn out_of_range_id_is_ignored() {
        record_bound_trip(BOUND_TRIP_ID_COUNT, 42);
        record_bound_trip(usize::MAX, 42);
    }

    #[test]
    fn concurrent_drain_never_sees_count_without_raw() {
        let _guard = lock_registry();
        drain_frame_trips();
        let id = BOUND_TRIP_PLAYER_TAG_DB_ID;
        let recorder = std::thread::spawn(move || {
            for i in 0..20_000u64 {
                record_bound_trip(id, 2_049 + (i % 7));
            }
        });
        let mut drained_total = 0u64;
        loop {
            let (count, raw) = drain_frame_trips()[id];
            if count != 0 {
                assert_ne!(
                    raw, 0,
                    "drained a counted trip whose raw attribution was lost"
                );
                drained_total += count;
            }
            if recorder.is_finished() {
                break;
            }
        }
        recorder.join().unwrap();
        let (count, raw) = drain_frame_trips()[id];
        if count != 0 {
            assert_ne!(raw, 0);
            drained_total += count;
        }
        assert_eq!(drained_total, 20_000, "no trip count may be lost");
    }

    #[test]
    fn a_supply_province_id_trip_names_its_own_bound() {
        let _guard = lock_registry();
        drain_frame_trips();
        let id = BOUND_TRIP_SUPPLY_PROVINCE_ID;

        assert!(!bound_exceeded_u32(1, 13_413, id));
        assert!(
            !bound_exceeded_u32(13_413, 13_413, id),
            "the largest vanilla province id must not be refused"
        );
        assert!(
            bound_exceeded_u32(13_414, 13_413, id),
            "one province past vanilla is exactly what a modded map adds"
        );
        assert!(bound_exceeded_u32(90_000, 13_413, id));

        let drained = drain_frame_trips();
        assert_eq!(drained[id], (2, 90_000));
        for other in 0..BOUND_TRIP_ID_COUNT {
            if other != id {
                assert_eq!(
                    drained[other].0, 0,
                    "a province-id refusal must not name bound {other}"
                );
            }
        }
    }

    #[test]
    fn each_bound_id_is_pinned_to_its_own_unit_names() {
        let mut pinned = std::vec::Vec::new();
        for (id, expected) in [
            (
                BOUND_TRIP_UNIT_LEADER_SKILL,
                (
                    "bt_unit_leader_skill_trips",
                    "bt_unit_leader_skill_raw_sample",
                ),
            ),
            (
                BOUND_TRIP_FOCUS_COUNT,
                ("bt_focus_count_trips", "bt_focus_count_raw_sample"),
            ),
            (
                BOUND_TRIP_COMPLETED_TECH_COUNT,
                (
                    "bt_completed_tech_count_trips",
                    "bt_completed_tech_count_raw_sample",
                ),
            ),
            (
                BOUND_TRIP_OWNER_TAG_DB_ID,
                ("bt_owner_tag_db_id_trips", "bt_owner_tag_db_id_raw_sample"),
            ),
            (
                BOUND_TRIP_PLAYER_TAG_DB_ID,
                (
                    "bt_player_tag_db_id_trips",
                    "bt_player_tag_db_id_raw_sample",
                ),
            ),
            (
                BOUND_TRIP_TAG_DB_ID_1024,
                ("bt_tag_db_id_1024_trips", "bt_tag_db_id_1024_raw_sample"),
            ),
            (
                BOUND_TRIP_FACTION_SLOTS,
                ("bt_faction_slots_trips", "bt_faction_slots_raw_sample"),
            ),
            (
                BOUND_TRIP_FACTION_MEMBERS,
                ("bt_faction_members_trips", "bt_faction_members_raw_sample"),
            ),
            (
                BOUND_TRIP_COUNTRY_LINK_INDEX,
                (
                    "bt_country_link_index_trips",
                    "bt_country_link_index_raw_sample",
                ),
            ),
            (
                BOUND_TRIP_SUPPLY_PROVINCE_ID,
                (
                    "bt_supply_province_id_trips",
                    "bt_supply_province_id_raw_sample",
                ),
            ),
            (
                BOUND_TRIP_WEATHER_ARRAY_LEN,
                (
                    "bt_weather_array_len_trips",
                    "bt_weather_array_len_raw_sample",
                ),
            ),
            (
                BOUND_TRIP_POLITICAL_PARTY_COUNT,
                (
                    "bt_political_party_count_trips",
                    "bt_political_party_count_raw_sample",
                ),
            ),
            (
                BOUND_TRIP_MAP_STATE_ID,
                ("bt_map_state_id_trips", "bt_map_state_id_raw_sample"),
            ),
            (
                BOUND_TRIP_FACTION_GROUP_INDEX,
                (
                    "bt_faction_group_index_trips",
                    "bt_faction_group_index_raw_sample",
                ),
            ),
            (
                BOUND_TRIP_MAP_PROVINCE_ID,
                ("bt_map_province_id_trips", "bt_map_province_id_raw_sample"),
            ),
            (
                BOUND_TRIP_MAP_PROVINCE_SLOTS,
                (
                    "bt_map_province_slots_trips",
                    "bt_map_province_slots_raw_sample",
                ),
            ),
            (
                BOUND_TRIP_CSUPPLY_INPUT_LEN,
                (
                    "bt_csupply_input_len_trips",
                    "bt_csupply_input_len_raw_sample",
                ),
            ),
        ] {
            pinned.push(id);
            assert_eq!(BOUND_TRIP_UNITS[id], expected, "bound id {id}");
        }

        pinned.sort_unstable();
        assert_eq!(
            pinned,
            (0..BOUND_TRIP_ID_COUNT).collect::<std::vec::Vec<_>>(),
            "every bound id must be pinned to its own unit NAMES above, not just \
             to the distinctness list below"
        );

        let ids = [
            BOUND_TRIP_UNIT_LEADER_SKILL,
            BOUND_TRIP_FOCUS_COUNT,
            BOUND_TRIP_COMPLETED_TECH_COUNT,
            BOUND_TRIP_OWNER_TAG_DB_ID,
            BOUND_TRIP_PLAYER_TAG_DB_ID,
            BOUND_TRIP_TAG_DB_ID_1024,
            BOUND_TRIP_FACTION_SLOTS,
            BOUND_TRIP_FACTION_MEMBERS,
            BOUND_TRIP_COUNTRY_LINK_INDEX,
            BOUND_TRIP_SUPPLY_PROVINCE_ID,
            BOUND_TRIP_WEATHER_ARRAY_LEN,
            BOUND_TRIP_POLITICAL_PARTY_COUNT,
            BOUND_TRIP_MAP_STATE_ID,
            BOUND_TRIP_FACTION_GROUP_INDEX,
            BOUND_TRIP_MAP_PROVINCE_ID,
            BOUND_TRIP_MAP_PROVINCE_SLOTS,
            BOUND_TRIP_CSUPPLY_INPUT_LEN,
        ];
        assert_eq!(
            ids.len(),
            BOUND_TRIP_ID_COUNT,
            "an id is missing from this table"
        );
        let mut sorted = ids;
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            core::array::from_fn::<usize, BOUND_TRIP_ID_COUNT, _>(|i| i),
            "the bound ids must be exactly 0..BOUND_TRIP_ID_COUNT with no alias and no hole"
        );
    }

    #[test]
    fn every_bound_id_has_its_own_counter_slot() {
        let _guard = lock_registry();
        drain_frame_trips();
        for id in 0..BOUND_TRIP_ID_COUNT {
            record_bound_trip(id, 1_000 + id as u64);
        }
        let drained = drain_frame_trips();
        for id in 0..BOUND_TRIP_ID_COUNT {
            assert_eq!(drained[id], (1, 1_000 + id as u64), "bound id {id}");
        }
        assert_eq!(BOUND_TRIP_UNITS.len(), BOUND_TRIP_ID_COUNT);
    }

    #[test]
    fn unit_names_satisfy_diagnostic_charset_and_sample_suffix() {
        for (trips, sample) in BOUND_TRIP_UNITS {
            for name in [trips, sample] {
                assert!(!name.is_empty());
                assert_eq!(
                    BOUND_TRIP_UNITS
                        .iter()
                        .filter(|(t, s)| *t == name || *s == name)
                        .count(),
                    1,
                    "unit name {name} is shared by two bounds"
                );
                assert!(name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
                assert!(name.starts_with("bt_"));
            }
            assert!(trips.ends_with("_trips"));
            assert!(!trips.ends_with("_sample"));
            assert!(sample.ends_with("_sample"));
        }
    }
}
