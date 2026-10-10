//! Numerical copy from the original Countries row loan. Native selectors and
//! variant memo retire on that worker before its callback loan is returned.
use crate::table_capture::Arena;
use engine::{Memory, cconvoys::transport as native, ccountry};
use replay::rows::{
    TransportConvoyCache, TransportConvoyEntry, TransportConvoyReservation, TransportConvoyVariant,
    TransportConvoys,
};
use std::sync::atomic::{AtomicU64, Ordering};

static REFUSALS: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];
const NAMES: [&str; 13] = [
    "transport original convoy country loan refused",
    "transport original convoy cache refused",
    "transport original unreserved count refused",
    "transport original convoy pool refused",
    "transport original signed reservations refused",
    "transport first convoy variant refused",
    "transport dirty pool classifier refused",
    "transport native transfer priority refused",
    "transport convoy pool variant identity refused",
    "transport convoy visibility cache refused",
    "transport convoy surface detection cache refused",
    "transport convoy submarine detection cache refused",
    "transport convoy speed cache refused",
];
static REASONS: [crate::status::RefusalReasons; 13] =
    [const { crate::status::RefusalReasons::new() }; 13];
pub(crate) fn status() -> impl serde::Serialize {
    crate::status::status!({
        "refusals": crate::status::map(NAMES.into_iter().zip(REFUSALS.iter().map(|value| value.load(Ordering::Relaxed)))),
        "reasons": crate::status::map(NAMES.into_iter().zip(REASONS.iter())),
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Capacities {
    pub countries: usize,
    pub pool: usize,
    pub reservations: usize,
    pub variants: usize,
}
#[derive(Default)]
struct Row {
    country: u32,
    pool_start: usize,
    reservation_start: usize,
    value: native::Observation,
}
pub(crate) struct Captured {
    hour: Option<i32>,
    rows: Arena<Row>,
    pool: Arena<native::PoolEntry>,
    reservations: Arena<native::Reservation>,
    scratch: native::Scratch,
    refused: bool,
}
impl Captured {
    pub(crate) fn new(capacity: Capacities) -> Self {
        Self {
            hour: None,
            rows: Arena::new(capacity.countries),
            pool: Arena::new(capacity.pool),
            reservations: Arena::new(capacity.reservations),
            scratch: native::Scratch::try_new(capacity.variants)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| native::Scratch::vacant()),
            refused: true,
        }
    }
    pub(crate) fn reset(&mut self) {
        self.hour = None;
        self.rows.reset();
        self.pool.reset();
        self.reservations.reset();
        self.scratch.begin();
        self.refused = true;
    }
    pub(crate) fn begin(&mut self, hour: i32, country_slots: usize) {
        self.reset();
        self.hour = Some(hour);
        self.rows.required = country_slots;
        crate::table_capture::grow_vec(&mut self.rows.first, country_slots);
        self.refused = false;
    }
    pub(crate) fn observe(
        &mut self,
        memory: &impl Memory,
        base: usize,
        country: ccountry::Country,
        original: native::OriginalCache,
    ) {
        let value = native::capture_growing(
            memory,
            base,
            country,
            original,
            (&mut self.pool.first, self.pool.len),
            (&mut self.reservations.first, self.reservations.len),
            &mut self.scratch,
        );
        self.pool.required = self.pool.required.saturating_add(value.pool_required);
        self.reservations.required = self
            .reservations
            .required
            .saturating_add(value.reservations_required);
        let first = value.first.ok().flatten();
        for (index, reason) in [
            value.cache.err(),
            value.unreserved.err(),
            value.pool.err(),
            value.reservations.err(),
            value.first.err(),
            value.classifier_failure,
            value.priority.err(),
            value.variant_identity_failure,
            first.and_then(|v| v.visibility.err()),
            first.and_then(|v| v.surface_detection.err()),
            first.and_then(|v| v.submarine_detection.err()),
            first.and_then(|v| v.speed.err()),
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(reason) = reason {
                REFUSALS[index + 1].fetch_add(1, Ordering::Relaxed);
                REASONS[index + 1].record(reason);
            }
        }
        let pool_start = self.pool.len;
        let reservation_start = self.reservations.len;
        self.pool.len += value.pool.unwrap_or(0);
        self.reservations.len += value.reservations.unwrap_or(0);
        self.rows.required = self.rows.required.max(self.rows.len.saturating_add(1));
        crate::table_capture::grow_vec(&mut self.rows.first, self.rows.required);
        if let Some(row) = self.rows.first.get_mut(self.rows.len) {
            *row = Row {
                country: country.id,
                pool_start,
                reservation_start,
                value,
            };
            self.rows.len += 1;
        } else {
            self.refused = true;
            REFUSALS[0].fetch_add(1, Ordering::Relaxed);
            REASONS[0].record("transport original convoy country loan incomplete");
        }
    }
    pub(crate) fn finish(&mut self, outcome: engine::walk::Outcome) {
        self.scratch.retire();
        if let engine::walk::Outcome::Truncated { len, unread_rows } = outcome {
            self.rows.required = self
                .rows
                .required
                .max(len.saturating_add(unread_rows as usize));
        }
        if !matches!(outcome, engine::walk::Outcome::Rows { .. }) {
            self.refused = true;
            REFUSALS[0].fetch_add(1, Ordering::Relaxed);
            REASONS[0].record("transport original convoy country loan incomplete");
        }
        self.rows.first[..self.rows.len].sort_unstable_by_key(|row| row.country);
        if self.rows.first[..self.rows.len]
            .windows(2)
            .any(|pair| pair[0].country == pair[1].country)
        {
            self.refused = true;
            REFUSALS[0].fetch_add(1, Ordering::Relaxed);
            REASONS[0].record("transport original convoy country loan incomplete");
        }
    }
    pub(crate) fn fail(&mut self) {
        self.scratch.retire();
        self.refused = true;
    }
    pub(crate) fn required(&self) -> Capacities {
        Capacities {
            countries: self.rows.required,
            pool: self.pool.required,
            reservations: self.reservations.required,
            variants: self.pool.required.max(self.scratch.required()),
        }
    }
    pub(crate) fn retain_required(&mut self, required: Capacities) {
        self.rows.required = self.rows.required.max(required.countries);
        self.pool.required = self.pool.required.max(required.pool);
        self.reservations.required = self.reservations.required.max(required.reservations);
        self.pool.required = self.pool.required.max(required.variants);
    }
    pub(crate) fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.countries);
        self.pool.grow(&mut capacity.pool);
        self.reservations.grow(&mut capacity.reservations);
        // Full unfiltered occurrence demand bounds distinct variants even when
        // an earlier capacity refusal prevented observing the later identities.
        crate::table_capture::grow_capacity(
            &mut capacity.variants,
            self.pool.required.max(self.scratch.required()),
        );
        self.scratch = native::Scratch::try_new(capacity.variants)
            .map_err(crate::allocation::io)
            .unwrap_or_else(|_| native::Scratch::vacant());
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.pool.allocated_bytes()
            + self.reservations.allocated_bytes()
            + self.scratch.allocated_bytes()
    }
    pub(crate) fn projection(&self, country: u32) -> std::io::Result<Option<TransportConvoys>> {
        if self.refused {
            return Ok(None);
        }
        let Some(sample_hour) = self.hour else {
            return Ok(None);
        };
        let Ok(index) =
            self.rows.first[..self.rows.len].binary_search_by_key(&country, |row| row.country)
        else {
            return Ok(None);
        };
        let row = &self.rows.first[index];
        let value = row.value;
        let pool = value
            .pool
            .ok()
            .map(|len| {
                engine::allocation::collect_vec(
                    self.pool.first[row.pool_start..row.pool_start + len]
                        .iter()
                        .map(|entry| TransportConvoyEntry {
                            variant: entry.variant,
                            flags: entry.flags,
                            quantity: entry.quantity,
                        }),
                )
            })
            .transpose()?;
        let reservations = value
            .reservations
            .ok()
            .map(|len| {
                engine::allocation::collect_vec(
                    self.reservations.first[row.reservation_start..row.reservation_start + len]
                        .iter()
                        .map(|entry| TransportConvoyReservation {
                            priority: entry.priority,
                            quantity: entry.quantity,
                        }),
                )
            })
            .transpose()?;
        Ok(Some(TransportConvoys {
            sample_hour,
            priority: value.priority.ok(),
            unreserved: value.unreserved.ok(),
            cache: value.cache.ok().map(|cache| TransportConvoyCache {
                valid: cache.valid,
                raw: cache.raw,
            }),
            pool,
            reservations,
            first_variant: value.first.ok().map(|first| {
                first.map(|variant| TransportConvoyVariant {
                    variant: variant.variant,
                    surface_detection: variant.surface_detection.ok(),
                    submarine_detection: variant.submarine_detection.ok(),
                    visibility: variant.visibility.ok(),
                    speed: variant.speed.ok(),
                })
            }),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::walk::Outcome;
    #[test]
    fn convoy_refusals_retain_reservation_classifier_and_variant_identity_causes() {
        struct FailedVariant;
        impl Memory for FailedVariant {
            fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
                let root = 0x100 + ccountry::CONVOY_STATE;
                match (at, bytes.len()) {
                    (at, 8) if at == root => {
                        bytes.copy_from_slice(&(engine::cconvoys::VTABLE_RVA as u64).to_le_bytes())
                    }
                    (at, 8) if at == root + engine::cconvoys::EQUIPMENT_POOL => bytes
                        .copy_from_slice(
                            &(engine::cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes(),
                        ),
                    (at, 16)
                        if at
                            == root
                                + engine::cconvoys::EQUIPMENT_POOL
                                + engine::cequipment_variant_pool::ITEMS =>
                    {
                        bytes[..8].copy_from_slice(&0x5000u64.to_le_bytes());
                        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
                        bytes[12..].copy_from_slice(&1u32.to_le_bytes());
                    }
                    (0x5000, 16) => {
                        bytes[..8].fill(0);
                        bytes[8..].copy_from_slice(&(-1i64).to_le_bytes());
                    }
                    (0x0331_b7dc, 4) | (0x1388, 4) => bytes.fill(0),
                    _ => return false,
                }
                true
            }
        }
        let mut captured = Captured::new(Capacities::default());
        captured.begin(1, 1);
        captured.observe(
            &FailedVariant,
            0,
            ccountry::Country {
                id: 1,
                canonical_slot: 1,
                pointer: 0x100,
            },
            Some(Ok(native::Cache {
                valid: 0,
                raw: None,
            })),
        );
        let value = serde_json::to_value(status()).unwrap();
        for name in [
            "transport first convoy variant refused",
            "transport dirty pool classifier refused",
            "transport convoy pool variant identity refused",
        ] {
            assert!(
                value["reasons"][name]["transport convoy variant is null"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
        }
        assert!(value["reasons"]["transport original signed reservations refused"]["transport convoy reservation header unreadable or invalid"].as_u64().unwrap() > 0);
    }

    #[test]
    fn original_five_thousand_country_loans_publish_complete_first_empty_convoy_graphs() {
        struct NativeEmpty;
        impl Memory for NativeEmpty {
            fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
                if at == 0x0331_b7dc && bytes.len() == 4 {
                    bytes.copy_from_slice(&(-2_i32).to_le_bytes());
                    return true;
                }
                let Some(offset) = at.checked_sub(0x100000).map(|at| at % 0x1400) else {
                    return false;
                };
                match (offset, bytes.len()) {
                    (ccountry::CONVOY_STATE, 8) => {
                        bytes.copy_from_slice(&(engine::cconvoys::VTABLE_RVA as u64).to_le_bytes())
                    }
                    (offset, 8)
                        if offset == ccountry::CONVOY_STATE + engine::cconvoys::EQUIPMENT_POOL =>
                    {
                        bytes.copy_from_slice(
                            &(engine::cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes(),
                        );
                    }
                    (0x1288, 4) => bytes.copy_from_slice(&(-8_i32).to_le_bytes()),
                    (offset, 16)
                        if offset
                            == ccountry::CONVOY_STATE
                                + engine::cconvoys::EQUIPMENT_POOL
                                + engine::cequipment_variant_pool::ITEMS
                            || offset
                                == ccountry::CONVOY_STATE + engine::cconvoys::RESERVATIONS =>
                    {
                        bytes.fill(0)
                    }
                    _ => return false,
                }
                true
            }
        }
        let mut captured = Captured::new(Capacities::default());
        captured.begin(7, 5001);
        for id in 1..=5000 {
            captured.observe(
                &NativeEmpty,
                0,
                ccountry::Country {
                    id,
                    canonical_slot: id,
                    pointer: 0x100000 + id as usize * 0x1400,
                },
                Some(Ok(native::Cache {
                    valid: 1,
                    raw: Some(0),
                })),
            );
        }
        captured.finish(Outcome::Rows { len: 5000 });
        for id in 1..=5000 {
            let packet = captured.projection(id).unwrap().unwrap();
            assert_eq!(packet.sample_hour, 7);
            assert_eq!(packet.pool, Some(vec![]));
            assert_eq!(packet.reservations, Some(vec![]));
            assert_eq!(packet.first_variant, Some(None));
            assert_eq!(packet.unreserved, Some(-8));
        }
    }
    #[test]
    fn first_variant_leaf_refusal_preserves_spotting_visibility_and_original_identity() {
        let mut captured = Captured::new(Capacities {
            countries: 1,
            ..Default::default()
        });
        captured.begin(7, 1);
        captured.rows.first[0] = Row {
            country: 9,
            value: native::Observation {
                first: Ok(Some(native::Variant {
                    variant: 17,
                    visibility: Ok(-5),
                    surface_detection: Ok(-3),
                    submarine_detection: Ok(4),
                    speed: Err("transport convoy speed cache unreadable"),
                })),
                pool: Ok(0),
                reservations: Ok(0),
                ..Default::default()
            },
            ..Default::default()
        };
        captured.rows.len = 1;
        captured.finish(Outcome::Rows { len: 1 });
        let packet = captured.projection(9).unwrap().unwrap();
        let first = replay::transport_convoys::first(&packet).unwrap().unwrap();
        assert_eq!(first.variant, 17);
        assert_eq!(first.visibility, Some(-5));
        assert_eq!(first.surface_detection, Some(-3));
        assert_eq!(first.submarine_detection, Some(4));
        assert_eq!(first.speed, None);
        captured.begin(8, 1);
        assert!(captured.projection(9).unwrap().is_none());
    }
    #[test]
    fn terminal_failure_retains_demands_and_current_failure_replaces_previous_packet() {
        let mut captured = Captured::new(Capacities {
            countries: 1,
            ..Default::default()
        });
        captured.begin(7, 1);
        captured.rows.first[0] = Row {
            country: 9,
            value: native::Observation {
                cache: Ok(native::Cache {
                    valid: 0,
                    raw: None,
                }),
                first: Ok(None),
                pool: Ok(0),
                reservations: Ok(0),
                ..Default::default()
            },
            ..Default::default()
        };
        captured.rows.len = 1;
        captured.finish(Outcome::Rows { len: 1 });
        assert_eq!(captured.projection(9).unwrap().unwrap().sample_hour, 7);
        captured.begin(8, 5000);
        captured.pool.required = 9000;
        captured.reservations.required = 7000;
        let demand = captured.required();
        captured.reset();
        captured.retain_required(demand);
        captured.fail();
        assert!(captured.projection(9).unwrap().is_none());
        let mut capacity = Capacities::default();
        captured.grow(&mut capacity);
        assert!(
            capacity.countries >= 5000 && capacity.pool >= 9000 && capacity.reservations >= 7000
        );
        assert!(capacity.variants >= 9000);
        captured.begin(9, 5000);
        assert!(captured.projection(9).unwrap().is_none());
    }
    #[test]
    fn old_country_truncation_cannot_admit_the_optional_packet_or_lose_tail_demand() {
        let mut captured = Captured::new(Capacities::default());
        captured.begin(7, 0);
        captured.finish(Outcome::Truncated {
            len: 1500,
            unread_rows: 3500,
        });
        assert_eq!(captured.required().countries, 5000);
        assert!(captured.projection(1).unwrap().is_none());
    }
}
