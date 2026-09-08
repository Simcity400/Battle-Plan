//! Current A27 country-table slots joined to D18 raw country handles.

use crate::country_identity::{CountryHandleEqualityRefusal, CountryIdentityRef};
use crate::WorldFrame;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignCountryRefusal {
    NegativeHandle(i32),
    InvalidFrameHour(i64),
    Link(CountryHandleEqualityRefusal),
    NegativeCountryIndex(i32),
    CatalogAbsent,
    CatalogUnavailable(String),
    CountrySlotUnavailable { index: u32, count: usize },
    CountryIdentity(String),
    CountrySlotMismatch { index: u32, identity_link: u32 },
}

impl std::fmt::Display for DesignCountryRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "A27 country lookup: {self:?}")
    }
}

impl std::error::Error for DesignCountryRefusal {}

impl WorldFrame {
    /// Resolve the native country handle through this frame's country table.
    ///
    /// `140ba56a0` uses index zero for handle zero; otherwise `140ba6240`
    /// reads the link cell. Both array indices are sign-extended. A27 retains
    /// the actual country-pointer vector's physical ordinals, independently
    /// of the D18 identity carried by each country. Negative indices address
    /// outside the captured vectors and remain unavailable here.
    ///
    /// This does not consume pending template operands or use an owner or
    /// viewer country as a replacement for an unavailable handle.
    pub fn design_country_identity_for_handle(
        &self,
        handle: i32,
    ) -> Result<CountryIdentityRef, DesignCountryRefusal> {
        use DesignCountryRefusal as Refusal;
        let handle = u32::try_from(handle).map_err(|_| Refusal::NegativeHandle(handle))?;
        let hour = u64::try_from(self.hour_index())
            .map_err(|_| Refusal::InvalidFrameHour(self.hour_index()))?;
        // The existing helper preserves the native handle-zero short circuit.
        // It validates the full current link observation for every nonzero handle.
        let index = self
            .country_handle_link_value(handle)
            .map_err(Refusal::Link)?;
        if (index as i32) < 0 {
            return Err(Refusal::NegativeCountryIndex(index as i32));
        }
        let catalog = self.design_catalog().ok_or(Refusal::CatalogAbsent)?;
        let native = catalog
            .native_designer_inputs_at_hour(hour)
            .map_err(Refusal::CatalogUnavailable)?;
        let country = native
            .countries
            .get(index as usize)
            .filter(|country| country.registry_index == u64::from(index))
            .ok_or(Refusal::CountrySlotUnavailable {
                index,
                count: native.countries.len(),
            })?;
        let id = self
            .country_identity_for_tag(&country.tag)
            .map_err(Refusal::CountryIdentity)?;
        // The selected A27 slot must describe the same current country D18
        // identifies. Neither the raw handle nor the slot ordinal is a TAG id.
        let identity_link = self.country_handle_link_value(id).map_err(Refusal::Link)?;
        if identity_link != index {
            return Err(Refusal::CountrySlotMismatch {
                index,
                identity_link,
            });
        }
        CountryIdentityRef::admitted_with_id(&country.tag, id, self.contended_census()).ok_or_else(
            || {
                Refusal::CountryIdentity(format!(
                    "current identity for {:?} is refused",
                    country.tag
                ))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design_catalog::{
        DesignCatalogGap, DesignCatalogGapCause, PortableDesignCatalog, A27_LIVE_CATALOG_SCHEMA,
    };
    use serde_json::{json, Value};
    use std::sync::Arc;

    const HOUR: i64 = 79;

    fn native_catalog(tags: &[&str], hour: u64) -> PortableDesignCatalog {
        let countries: Vec<_> = tags
            .iter()
            .enumerate()
            .map(|(index, tag)| json!({"registry_index":index,"tag":tag}))
            .collect();
        let value = json!({
            "schema":A27_LIVE_CATALOG_SCHEMA,
            "authority":{"program":"hoi4.exe 1.19.2.0.a729",
                "program_sha256":"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
                "current_date_raw":43799999u64+hour,"capture_hour_index":hour,
                "coherence":"double_read_all_roots_fields_and_membership"},
            "full_replacement":true,
            "completeness":{"definitions":true,"lifecycle":true,"membership":true,
                "owner":true,"ui_occurrences":true},
            "runtime_dlc_mask":{"status":"measured","raw_u32":0},
            "country_count":countries.len(),"countries":countries,
            "division_templates":[],"variants":[]
        });
        PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn raw() -> Value {
        json!({"tag_db_id_index":{"country_composition_version":2,
            "country_link_indices":{"status":"measured","indices":[99,2,2,1,1,0,3,2]}},
            "countries":{"AAA":{"_r2_tag_db_id":5},"GER":{"_r2_tag_db_id":3},
                "FRA":{"_r2_tag_db_id":7}}})
    }

    fn frame(raw: Value, hour: i64, catalog: Option<PortableDesignCatalog>) -> WorldFrame {
        WorldFrame::from_raw_scheduled(
            crate::source::RawFrame {
                hour_index: hour,
                date: None,
                save_version: None,
                schema_version: None,
                rest: raw.as_object().unwrap().clone().into(),
            },
            catalog.map(Arc::new),
            None,
            None,
            crate::BuildSchedule::Serial,
        )
    }

    fn current(raw: Value) -> WorldFrame {
        frame(
            raw,
            HOUR,
            Some(native_catalog(&["AAA", "GER", "FRA"], HOUR as u64)),
        )
    }

    #[test]
    fn raw_alias_uses_physical_a27_slot_then_exact_current_identity() {
        let world = current(raw());
        for handle in [1, 2, 7] {
            let identity = world.design_country_identity_for_handle(handle).unwrap();
            assert_eq!(identity.tag(), "FRA");
            assert_eq!(identity.tag_db_id(), Some(7));
        }
        let identity = world.design_country_identity_for_handle(4).unwrap();
        assert_eq!((identity.tag(), identity.tag_db_id()), ("GER", Some(3)));
    }

    #[test]
    fn zero_handle_uses_slot_zero_without_reading_link_cell_zero() {
        let world = current(raw());
        assert_eq!(world.country_handle_link_value(0), Ok(0));
        let identity = world.design_country_identity_for_handle(0).unwrap();
        assert_eq!((identity.tag(), identity.tag_db_id()), ("AAA", Some(5)));
        // Slot zero is still a real country join, not a manufactured absence.
        let mut missing = raw();
        missing["tag_db_id_index"]
            .as_object_mut()
            .unwrap()
            .remove("country_link_indices");
        assert_eq!(
            current(missing).design_country_identity_for_handle(0),
            Err(DesignCountryRefusal::Link(
                CountryHandleEqualityRefusal::Missing
            ))
        );
    }

    #[test]
    fn signed_input_and_translated_indices_are_not_unsigned_country_slots() {
        let world = frame(json!({}), HOUR, None);
        for handle in [-1, i32::MIN] {
            assert_eq!(
                world.design_country_identity_for_handle(handle),
                Err(DesignCountryRefusal::NegativeHandle(handle))
            );
        }
        for bits in [u32::MAX, 0x8000_0000] {
            let mut value = raw();
            value["tag_db_id_index"]["country_link_indices"]["indices"][1] = json!(bits);
            assert_eq!(
                current(value).design_country_identity_for_handle(1),
                Err(DesignCountryRefusal::NegativeCountryIndex(bits as i32))
            );
        }
    }

    #[test]
    fn unavailable_malformed_and_out_of_range_links_keep_their_causes() {
        use CountryHandleEqualityRefusal as Link;
        for (value, expected) in [
            (Value::Null, Link::Unobtained),
            (
                json!({"status":"unavailable","reason":"changed"}),
                Link::Unavailable("changed".into()),
            ),
            (
                json!({"status":"measured","indices":[0,2,-1]}),
                Link::Malformed("link cell"),
            ),
            (
                json!({"status":"measured","indices":[]}),
                Link::OutOfRange {
                    handle: 1,
                    count: 0,
                },
            ),
        ] {
            let mut source = raw();
            source["tag_db_id_index"]["country_link_indices"] = value;
            assert_eq!(
                current(source).design_country_identity_for_handle(1),
                Err(DesignCountryRefusal::Link(expected))
            );
        }
        assert_eq!(
            current(raw()).design_country_identity_for_handle(8),
            Err(DesignCountryRefusal::Link(Link::OutOfRange {
                handle: 8,
                count: 8
            }))
        );
    }

    #[test]
    fn positive_link_outside_actual_a27_census_is_not_a_country() {
        assert_eq!(
            current(raw()).design_country_identity_for_handle(6),
            Err(DesignCountryRefusal::CountrySlotUnavailable { index: 3, count: 3 })
        );
        let empty = frame(raw(), HOUR, Some(native_catalog(&[], HOUR as u64)));
        assert_eq!(
            empty.design_country_identity_for_handle(0),
            Err(DesignCountryRefusal::CountrySlotUnavailable { index: 0, count: 0 })
        );
    }

    #[test]
    fn catalog_must_be_the_frames_own_exact_hour_without_historical_fallback() {
        assert_eq!(
            frame(raw(), HOUR, None).design_country_identity_for_handle(1),
            Err(DesignCountryRefusal::CatalogAbsent)
        );
        let old = frame(
            raw(),
            HOUR,
            Some(native_catalog(&["AAA", "GER", "FRA"], HOUR as u64 - 1)),
        );
        assert!(matches!(
            old.design_country_identity_for_handle(1),
            Err(DesignCountryRefusal::CatalogUnavailable(_))
        ));
        assert_eq!(
            frame(raw(), -1, None).design_country_identity_for_handle(1),
            Err(DesignCountryRefusal::InvalidFrameHour(-1))
        );
    }

    #[test]
    fn catalog_gap_refuses_and_current_recovery_resolves_again() {
        let mut catalog = native_catalog(&["AAA", "GER", "FRA"], HOUR as u64);
        catalog.exact_native_catalog = Value::Null;
        catalog.gaps = vec![DesignCatalogGap {
            first_hour_index: HOUR as u64,
            last_hour_index: HOUR as u64,
            cause: DesignCatalogGapCause::Torn,
            witness: json!({"field":"country_registry"}),
        }];
        let catalog =
            PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap())
                .unwrap();
        assert!(matches!(
            frame(raw(), HOUR, Some(catalog)).design_country_identity_for_handle(1),
            Err(DesignCountryRefusal::CatalogUnavailable(_))
        ));
        assert_eq!(
            current(raw())
                .design_country_identity_for_handle(1)
                .unwrap()
                .tag(),
            "FRA"
        );
    }

    #[test]
    fn swapped_catalog_tags_and_changed_d18_links_refuse_the_cross_join() {
        let swapped = frame(
            raw(),
            HOUR,
            Some(native_catalog(&["AAA", "FRA", "GER"], HOUR as u64)),
        );
        assert_eq!(
            swapped.design_country_identity_for_handle(1),
            Err(DesignCountryRefusal::CountrySlotMismatch {
                index: 2,
                identity_link: 1
            })
        );
        let mut source = raw();
        source["tag_db_id_index"]["country_link_indices"]["indices"][7] = json!(1);
        assert_eq!(
            current(source).design_country_identity_for_handle(1),
            Err(DesignCountryRefusal::CountrySlotMismatch {
                index: 2,
                identity_link: 1
            })
        );
    }

    #[test]
    fn reached_country_requires_identity_but_unrelated_exclusion_is_local() {
        let mut source = raw();
        source["countries"]["AAA"]
            .as_object_mut()
            .unwrap()
            .remove("_r2_tag_db_id");
        assert_eq!(
            current(source.clone())
                .design_country_identity_for_handle(1)
                .unwrap()
                .tag(),
            "FRA"
        );
        source["countries"]["FRA"]
            .as_object_mut()
            .unwrap()
            .remove("_r2_tag_db_id");
        assert!(matches!(
            current(source).design_country_identity_for_handle(1),
            Err(DesignCountryRefusal::CountryIdentity(_))
        ));
        let mut contended = raw();
        contended["tag_db_id_index"]["withheld_countries"] =
            json!({"contended_keys":{"FRA":{"tag_db_ids":[7,8]}}});
        assert!(current(contended)
            .design_country_identity_for_handle(1)
            .is_err());
    }

    #[test]
    fn d18_withdrawal_refuses_even_zero_handle_and_matching_a27() {
        let mut source = raw();
        source["provenance"] = json!({"collector":{"captured":true,
            "domain_problems":[{"domain_id":18,"problem":"partial_data"}]}});
        for handle in [0, 1] {
            assert!(matches!(
                current(source.clone()).design_country_identity_for_handle(handle),
                Err(DesignCountryRefusal::Link(_)) | Err(DesignCountryRefusal::CountryIdentity(_))
            ));
        }
        source["provenance"]["collector"]["domain_problems"] = json!("malformed");
        assert_eq!(
            current(source).design_country_identity_for_handle(1),
            Err(DesignCountryRefusal::Link(
                CountryHandleEqualityRefusal::CollectorLedgerMalformed
            ))
        );
    }
}
