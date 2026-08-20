use portable_content_manifest::UsedContentKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const FOUNDATION_RENDER_SCHEMA: &str = "hoi4.viewer-map-foundation.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapVisualFamily {
    CameraView,
    Terrain,
    PoliticalFillGradients,
    FogOfWar,
    ThinBorders,
    WaterIce,
    RiversStraits,
    LightingShadows,
    GradingPostFx,
    Trees,
    CitiesBuildingsMapObjects,
    UnitsCounters,
    CombatMarkers,
    OrdersFrontsArrows,
    RailSupply,
    LabelsMapText,
    MapModes,
    WeatherAtmosphere,
    ParticlesEffects,
    MapFurnitureIcons,
}

impl MapVisualFamily {
    pub const ALL: [Self; 20] = [
        Self::CameraView,
        Self::Terrain,
        Self::PoliticalFillGradients,
        Self::FogOfWar,
        Self::ThinBorders,
        Self::WaterIce,
        Self::RiversStraits,
        Self::LightingShadows,
        Self::GradingPostFx,
        Self::Trees,
        Self::CitiesBuildingsMapObjects,
        Self::UnitsCounters,
        Self::CombatMarkers,
        Self::OrdersFrontsArrows,
        Self::RailSupply,
        Self::LabelsMapText,
        Self::MapModes,
        Self::WeatherAtmosphere,
        Self::ParticlesEffects,
        Self::MapFurnitureIcons,
    ];

    pub const fn owner_packet(self) -> &'static str {
        match self {
            Self::CameraView => "XMAP03",
            Self::Terrain => "XMAP04",
            Self::PoliticalFillGradients | Self::FogOfWar => "XMAP05",
            Self::ThinBorders => "XMAP06",
            Self::WaterIce => "XMAP07",
            Self::RiversStraits => "XMAP08",
            Self::LightingShadows => "XMAP09",
            Self::GradingPostFx => "XMAP10",
            Self::Trees => "XMAP11",
            Self::CitiesBuildingsMapObjects => "XMAP12",
            Self::UnitsCounters => "XMAP13",
            Self::CombatMarkers => "XMAP14",
            Self::OrdersFrontsArrows => "XMAP15",
            Self::RailSupply => "XMAP16",
            Self::LabelsMapText => "XMAP17",
            Self::MapModes => "XMAP18",
            Self::WeatherAtmosphere => "XMAP19",
            Self::ParticlesEffects => "XMAP20",
            Self::MapFurnitureIcons => "XMAP21",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationOperationIdentity {
    pub operation_id: String,
    pub global_order: u32,
    pub program_sha256: String,
    pub ordered_resources_sha256: String,
    pub target_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationGap {
    pub gap_id: String,
    pub first_unavailable_hour: i64,
    pub last_unavailable_hour: i64,
    pub witness_sha256: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum FamilyDisposition {
    Present {
        payload_schema: String,
        producer_registration_sha256: String,
        payload_sha256: String,
        ordered_operations: Vec<FoundationOperationIdentity>,
        ordered_used_content: Vec<UsedContentKey>,
    },
    AuthenticatedEmpty {
        producer_registration_sha256: String,
        enumerator_witness_sha256: String,
    },
    NotApplicable {
        applicability_witness_sha256: String,
        reason: String,
    },
    /// The family's answer is not computable at once-per-hour foundation time:
    /// it depends on live per-frame draw state (D3D resource identities, the
    /// prior frame's temporal receipt) that does not exist yet. The readiness
    /// graph records WHERE the answer will come from, not that the answer is
    /// yes. The frame still refuses to show until the draw path discharges
    /// `draw_contract_id` -- see `FoundationRenderEnvelope::seal_readiness`.
    DeferredToDraw {
        producer_registration_sha256: String,
        draw_contract_id: String,
        reason: String,
    },
    Unavailable {
        gap: FoundationGap,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyCoverageRow {
    pub family: MapVisualFamily,
    pub owner_packet: String,
    pub disposition: FamilyDisposition,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationRenderEnvelope {
    pub schema: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub families: Vec<FamilyCoverageRow>,
}

impl FoundationRenderEnvelope {
    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("foundation render envelope JSON: {error}"))?;
        value.validate()?;
        // STRICT ADMISSION. `deny_unknown_fields` on the outer types does not
        // reach inside an internally tagged enum's variants, and serde rejects
        // the attribute on those variants outright, so a stray field inside a
        // disposition used to be accepted and silently dropped: two different
        // inputs normalised to one admitted envelope and one
        // deterministic_sha256. Comparing the input tree against the re-encoded
        // decoded value closes that for the whole envelope at once, nested
        // fields included -- anything the decode dropped shows up as a
        // difference here (Codex review, round 1).
        let submitted: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("foundation render envelope JSON: {error}"))?;
        let admitted = serde_json::to_value(&value)
            .map_err(|error| format!("foundation render envelope re-encode: {error}"))?;
        if submitted != admitted {
            return Err(
                "foundation render envelope carries fields outside its admitted schema".into(),
            );
        }
        Ok(value)
    }

    pub fn deterministic_sha256(&self) -> Result<String, String> {
        Ok(format!("{:x}", Sha256::digest(self.encode_json()?)))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != FOUNDATION_RENDER_SCHEMA {
            return Err(format!(
                "unsupported foundation render schema {:?}",
                self.schema
            ));
        }
        validate_sha256(&self.content_manifest_sha256, "content manifest")?;
        validate_sha256(&self.product_modset_sha256, "product modset")?;
        validate_sha256(&self.effective_vfs_manifest_sha256, "effective VFS")?;
        validate_sha256(
            &self.foundation_content_operation_sha256,
            "foundation content operation",
        )?;
        if self.families.len() != MapVisualFamily::ALL.len() {
            return Err(format!(
                "foundation family coverage is incomplete: expected {}, got {}",
                MapVisualFamily::ALL.len(),
                self.families.len()
            ));
        }
        for (expected, row) in MapVisualFamily::ALL.iter().zip(&self.families) {
            if &row.family != expected || row.owner_packet != expected.owner_packet() {
                return Err(format!(
                    "foundation family identity/order mismatch at {:?}",
                    expected
                ));
            }
            validate_disposition(row)?;
        }
        Ok(())
    }

    pub fn gaps(&self) -> impl Iterator<Item = (&MapVisualFamily, &FoundationGap)> {
        self.families
            .iter()
            .filter_map(|row| match &row.disposition {
                FamilyDisposition::Unavailable { gap } => Some((&row.family, gap)),
                _ => None,
            })
    }

    /// True when some family outright refuses this hour. A family that defers
    /// to draw time is NOT refusing -- it is answered per-frame, and
    /// `seal_readiness` is what makes it prove that. Sealing a frame is
    /// `seal_readiness`'s job and never this predicate's: this only answers
    /// "did anything refuse", which is what the stale-frame fallback cache and
    /// the gap reporting want.
    pub fn has_refusing_family(&self) -> bool {
        self.families
            .iter()
            .any(|row| matches!(row.disposition, FamilyDisposition::Unavailable { .. }))
    }

    /// Every family that deferred its answer to draw time, in family order.
    pub fn deferred_draw_contracts(&self) -> impl Iterator<Item = (&MapVisualFamily, &str)> {
        self.families
            .iter()
            .filter_map(|row| match &row.disposition {
                FamilyDisposition::DeferredToDraw {
                    draw_contract_id, ..
                } => Some((&row.family, draw_contract_id.as_str())),
                _ => None,
            })
    }

    /// A discharge is keyed by the FAMILY that deferred as well as the contract
    /// it named. Keying on the contract string alone was not enough: an owner
    /// packet can own more than one family (XMAP05 owns both
    /// PoliticalFillGradients and FogOfWar), so two rows could defer the same
    /// owner-scoped id and a single receipt would have vouched for both.
    pub fn draw_contract_key(
        family: MapVisualFamily,
        draw_contract_id: &str,
    ) -> (MapVisualFamily, String) {
        (family, draw_contract_id.to_string())
    }

    /// The frame-may-show law. A frame seals only when EVERY family vouches:
    /// families that vouched at foundation time must not be unavailable, and
    /// families that deferred must have had their exact draw contract
    /// discharged by the draw path. This moves WHEN a family vouches, never
    /// WHETHER it has to.
    ///
    /// `discharged_draw_contracts` is the set of contract ids the draw path
    /// actually completed this frame. An id it does not recognise is a refusal,
    /// not a pass: a draw path that claims contracts nobody deferred is out of
    /// step with the foundation it is drawing.
    pub fn seal_readiness(
        &self,
        discharged_draw_contracts: &BTreeSet<(MapVisualFamily, String)>,
    ) -> Result<(), String> {
        self.validate()?;
        let mut deferred = BTreeSet::new();
        for row in &self.families {
            match &row.disposition {
                FamilyDisposition::Unavailable { gap } => {
                    return Err(format!(
                        "{:?} refuses this frame: {} ({})",
                        row.family, gap.reason, gap.gap_id
                    ));
                }
                FamilyDisposition::DeferredToDraw {
                    draw_contract_id, ..
                } => {
                    let key = Self::draw_contract_key(row.family, draw_contract_id);
                    if !discharged_draw_contracts.contains(&key) {
                        return Err(format!(
                            "{:?} deferred to draw time and its contract {draw_contract_id} was never discharged",
                            row.family
                        ));
                    }
                    deferred.insert(key);
                }
                _ => {}
            }
        }
        if let Some((family, contract)) =
            discharged_draw_contracts.difference(&deferred).next()
        {
            return Err(format!(
                "the draw path discharged {contract} for {family:?}, which did not defer it"
            ));
        }
        Ok(())
    }
}

fn validate_disposition(row: &FamilyCoverageRow) -> Result<(), String> {
    match &row.disposition {
        FamilyDisposition::Present {
            payload_schema,
            producer_registration_sha256,
            payload_sha256,
            ordered_operations,
            ordered_used_content,
        } => {
            if payload_schema.trim().is_empty() {
                return Err(format!("{:?} has a blank payload schema", row.family));
            }
            validate_sha256(producer_registration_sha256, "family producer registration")?;
            validate_sha256(payload_sha256, "family payload")?;
            let mut operation_ids = BTreeSet::new();
            let mut prior_order = None;
            for operation in ordered_operations {
                if operation.operation_id.trim().is_empty()
                    || !operation_ids.insert(operation.operation_id.as_str())
                    || prior_order.is_some_and(|prior| operation.global_order <= prior)
                {
                    return Err(format!(
                        "{:?} operations are blank, duplicated, or not strictly ordered",
                        row.family
                    ));
                }
                validate_sha256(&operation.program_sha256, "operation program")?;
                validate_sha256(
                    &operation.ordered_resources_sha256,
                    "operation resource order",
                )?;
                validate_sha256(&operation.target_sha256, "operation target")?;
                prior_order = Some(operation.global_order);
            }
            let mut used_paths = BTreeSet::new();
            for content in ordered_used_content {
                content.validate()?;
                if !used_paths.insert(content.normalized_game_relative_path.as_str()) {
                    return Err(format!("{:?} repeats a UsedContent path", row.family));
                }
            }
        }
        FamilyDisposition::AuthenticatedEmpty {
            producer_registration_sha256,
            enumerator_witness_sha256,
        } => {
            validate_sha256(
                producer_registration_sha256,
                "empty-family producer registration",
            )?;
            validate_sha256(enumerator_witness_sha256, "empty-family enumerator witness")?;
        }
        FamilyDisposition::NotApplicable {
            applicability_witness_sha256,
            reason,
        } => {
            validate_sha256(
                applicability_witness_sha256,
                "not-applicable family witness",
            )?;
            if reason.trim().is_empty() {
                return Err(format!(
                    "{:?} has a blank not-applicable reason",
                    row.family
                ));
            }
        }
        FamilyDisposition::DeferredToDraw {
            producer_registration_sha256,
            draw_contract_id,
            reason,
        } => {
            validate_sha256(
                producer_registration_sha256,
                "draw-deferred family producer registration",
            )?;
            // The contract id is what the draw path must name to discharge the
            // deferral. A blank or non-owner-scoped id would let any receipt
            // satisfy any family, so it is required to be exactly
            // "<OWNER>.<contract>".
            let owner_prefix = format!("{}.", row.family.owner_packet());
            if !draw_contract_id.starts_with(&owner_prefix)
                || draw_contract_id.len() <= owner_prefix.len()
                || draw_contract_id.trim() != draw_contract_id
            {
                return Err(format!(
                    "{:?} draw contract {draw_contract_id:?} is not scoped to its owner packet",
                    row.family
                ));
            }
            if reason.trim().is_empty() {
                return Err(format!(
                    "{:?} defers to draw time without saying why",
                    row.family
                ));
            }
        }
        FamilyDisposition::Unavailable { gap } => {
            if gap.gap_id.trim().is_empty()
                || gap.reason.trim().is_empty()
                || gap.first_unavailable_hour > gap.last_unavailable_hour
            {
                return Err(format!("{:?} has an invalid unavailable gap", row.family));
            }
            validate_sha256(&gap.witness_sha256, "unavailable family witness")?;
        }
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> FoundationRenderEnvelope {
        FoundationRenderEnvelope {
            schema: FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: 42,
            content_manifest_sha256: "b".repeat(64),
            product_modset_sha256: "c".repeat(64),
            effective_vfs_manifest_sha256: "d".repeat(64),
            foundation_content_operation_sha256: "e".repeat(64),
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: FamilyDisposition::Unavailable {
                        gap: FoundationGap {
                            gap_id: format!("{}.adapter_unimplemented", family.owner_packet()),
                            first_unavailable_hour: 42,
                            last_unavailable_hour: 42,
                            witness_sha256: "a".repeat(64),
                            reason: "adapter has not promoted".into(),
                        },
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn exhaustive_family_envelope_round_trips_byte_exactly() {
        let expected = envelope();
        let first = expected.encode_json().unwrap();
        let decoded = FoundationRenderEnvelope::decode_json(&first).unwrap();
        assert_eq!(decoded.encode_json().unwrap(), first);
        assert_eq!(decoded, expected);
        assert_eq!(decoded.gaps().count(), MapVisualFamily::ALL.len());
    }

    fn deferred(family: MapVisualFamily, contract: &str) -> FamilyDisposition {
        FamilyDisposition::DeferredToDraw {
            producer_registration_sha256: "f".repeat(64),
            draw_contract_id: format!("{}.{contract}", family.owner_packet()),
            reason: "resolves against live per-frame draw state".into(),
        }
    }

    fn all_vouched_envelope() -> FoundationRenderEnvelope {
        let mut value = envelope();
        for row in &mut value.families {
            row.disposition = FamilyDisposition::AuthenticatedEmpty {
                producer_registration_sha256: "a".repeat(64),
                enumerator_witness_sha256: "b".repeat(64),
            };
        }
        value
    }

    // kills: making DeferredToDraw satisfy the seal on its own (i.e. treating a
    // deferral as a vouch) -- the first assert would pass with an empty
    // discharge set.
    #[test]
    fn a_deferred_family_seals_only_once_its_draw_contract_is_discharged() {
        let mut value = all_vouched_envelope();
        value.families[8].disposition = deferred(MapVisualFamily::GradingPostFx, "postfx_receipt");
        value.validate().unwrap();
        assert!(!value.has_refusing_family(), "a deferral is not a refusal");

        let empty = BTreeSet::new();
        let refusal = value.seal_readiness(&empty).unwrap_err();
        assert!(refusal.contains("never discharged"), "{refusal}");

        let discharged = BTreeSet::from([FoundationRenderEnvelope::draw_contract_key(
            MapVisualFamily::GradingPostFx,
            "XMAP10.postfx_receipt",
        )]);
        value.seal_readiness(&discharged).unwrap();

        // A discharge nobody deferred is a refusal, not a pass.
        let foreign = BTreeSet::from([
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::GradingPostFx,
                "XMAP10.postfx_receipt",
            ),
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::ParticlesEffects,
                "XMAP20.effects",
            ),
        ]);
        assert!(value
            .seal_readiness(&foreign)
            .unwrap_err()
            .contains("did not defer it"));
    }

    // kills: letting the draw path discharge an unavailable family. Moving WHEN
    // a family vouches must never move WHETHER it does.
    #[test]
    fn discharging_draw_contracts_can_never_rescue_an_unavailable_family() {
        let mut value = all_vouched_envelope();
        value.families[18].disposition = FamilyDisposition::Unavailable {
            gap: FoundationGap {
                gap_id: "XMAP20.carrier_absent".into(),
                first_unavailable_hour: 42,
                last_unavailable_hour: 42,
                witness_sha256: "a".repeat(64),
                reason: "the effect lifecycle carrier does not exist".into(),
            },
        };
        let everything = BTreeSet::from([
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::ParticlesEffects,
                "XMAP20.effects",
            ),
        ]);
        assert!(value
            .seal_readiness(&everything)
            .unwrap_err()
            .contains("refuses this frame"));
    }

    // kills: accepting a blank/foreign-scoped contract id, which would let one
    // family's receipt discharge another family's deferral.
    #[test]
    fn a_draw_contract_must_be_scoped_to_the_family_that_deferred_it() {
        for bad in ["", "XMAP20.effects", "XMAP10.", " XMAP10.x"] {
            let mut value = all_vouched_envelope();
            value.families[8].disposition = FamilyDisposition::DeferredToDraw {
                producer_registration_sha256: "f".repeat(64),
                draw_contract_id: bad.into(),
                reason: "live per-frame state".into(),
            };
            assert!(value.validate().is_err(), "accepted contract id {bad:?}");
        }
        let mut blank_reason = all_vouched_envelope();
        blank_reason.families[8].disposition = FamilyDisposition::DeferredToDraw {
            producer_registration_sha256: "f".repeat(64),
            draw_contract_id: "XMAP10.postfx_receipt".into(),
            reason: "   ".into(),
        };
        assert!(blank_reason.validate().is_err());
    }

    // kills: dropping the strict round-trip admission in decode_json. Without
    // it a stray field inside a disposition is silently discarded, so two
    // different byte inputs admit to one envelope and one deterministic hash.
    #[test]
    fn a_field_outside_the_admitted_schema_is_refused_not_normalised_away() {
        let mut value = all_vouched_envelope();
        value.families[8].disposition = deferred(MapVisualFamily::GradingPostFx, "postfx_receipt");
        let clean = value.encode_json().unwrap();
        assert_eq!(FoundationRenderEnvelope::decode_json(&clean).unwrap(), value);

        let mut tree: serde_json::Value = serde_json::from_slice(&clean).unwrap();
        // INSIDE the disposition object: that is the hole. The row and the
        // envelope already carry deny_unknown_fields, but it cannot reach into
        // an internally tagged enum's variant.
        tree["families"][8]["disposition"]["smuggled"] = serde_json::json!("payload");
        let smuggled = serde_json::to_vec(&tree).unwrap();
        let error = FoundationRenderEnvelope::decode_json(&smuggled).unwrap_err();
        assert!(error.contains("outside its admitted schema"), "{error}");
    }

    // kills: keying discharges on the contract string alone. XMAP05 owns TWO
    // families, so both can legally defer the same owner-scoped contract id;
    // with a bare-string set one receipt vouched for both of them.
    #[test]
    fn one_receipt_cannot_vouch_for_two_families_that_share_an_owner_packet() {
        let mut value = all_vouched_envelope();
        for index in [2, 3] {
            value.families[index].disposition = FamilyDisposition::DeferredToDraw {
                producer_registration_sha256: "f".repeat(64),
                draw_contract_id: "XMAP05.fill_receipt".into(),
                reason: "resolves against live per-frame draw state".into(),
            };
        }
        value.validate().unwrap();
        assert_eq!(value.families[2].family, MapVisualFamily::PoliticalFillGradients);
        assert_eq!(value.families[3].family, MapVisualFamily::FogOfWar);

        // Discharging it for ONE family must not seal the other.
        let one = BTreeSet::from([FoundationRenderEnvelope::draw_contract_key(
            MapVisualFamily::PoliticalFillGradients,
            "XMAP05.fill_receipt",
        )]);
        let error = value.seal_readiness(&one).unwrap_err();
        assert!(error.contains("FogOfWar"), "{error}");

        let both = BTreeSet::from([
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::PoliticalFillGradients,
                "XMAP05.fill_receipt",
            ),
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::FogOfWar,
                "XMAP05.fill_receipt",
            ),
        ]);
        value.seal_readiness(&both).unwrap();
    }

    #[test]
    fn missing_reordered_or_recorder_presentation_state_fails_closed() {
        let mut missing = envelope();
        missing.families.pop();
        assert!(missing.validate().unwrap_err().contains("incomplete"));
        let mut reordered = envelope();
        reordered.families.swap(0, 1);
        assert!(reordered.validate().unwrap_err().contains("identity/order"));
        let mut value = serde_json::to_value(envelope()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("selected_present".into(), serde_json::json!({}));
        assert!(serde_json::from_value::<FoundationRenderEnvelope>(value).is_err());
    }
}
