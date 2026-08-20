use crate::{
    FoundationFeatureContext, FoundationFeatureOutput, ProductionMapFeatureAdapter, Result,
};
use portable_render_stream::{FamilyDisposition, FoundationGap, MapVisualFamily};
use snapshot::sha256::Sha256;

/// The map-authority inputs XMAP21 needs before it can prepare furniture.
///
/// `map_furniture.current_victory_points` is owner-proven since lane OV built
/// its producer (compact domain 101, `map_furniture_victory_points_live`).
/// `country.current_capital_state` is not, and the family needs BOTH, so it
/// still refuses — see [`XMAP21_UNPROVEN_AUTHORITY_INPUTS`].
pub const XMAP21_REQUIRED_AUTHORITY_INPUTS: [&str; 2] = [
    "country.current_capital_state",
    "map_furniture.current_victory_points",
];

/// The required inputs XMAP00 does NOT yet call owner-proven.
///
/// Derived rather than written down: a second lane proving the capital state
/// must not have to remember to edit a refusal string here, and a refusal that
/// names a proven input as missing is a false statement about the recording.
pub fn xmap21_unproven_authority_inputs() -> Vec<&'static str> {
    XMAP21_REQUIRED_AUTHORITY_INPUTS
        .into_iter()
        .filter(|input| crate::map_authority::map_authority_owner_proven_admission(input).is_none())
        .collect()
}

pub const XMAP21_FURNITURE_ABSENT_GAP_ID: &str = "XMAP21.map_furniture_authority_absent";

/// MapFurnitureIcons is registered so the readiness graph sees the family and
/// its real blocker rather than a generic "no adapter" placeholder.
///
/// This adapter is deliberately ADAPTER-SHAPED, not content-shaped: it carries
/// an optional prepared furniture packet and reports on it. Whoever lands the
/// furniture content fills `Prepared` in; the registration, the disposition
/// vocabulary, and the readiness wiring do not have to move again.
pub enum Xmap21MapFurnitureAdapter {
    Prepared(Box<crate::PreparedMapFurniturePacket>),
    AuthorityAbsent,
}

impl Xmap21MapFurnitureAdapter {
    pub fn new(prepared: crate::PreparedMapFurniturePacket) -> Result<Self> {
        prepared.validate()?;
        Ok(Self::Prepared(Box::new(prepared)))
    }

    /// No furniture authority is admitted on the wire, so the family refuses.
    pub fn authority_absent() -> Self {
        Self::AuthorityAbsent
    }

    pub(crate) fn producer_registration_sha256() -> String {
        Sha256::hex(
            format!(
                "XMAP21\0{}\0{}\0{}",
                crate::XMAP21_PREPARED_PACKET_SCHEMA,
                crate::XMAP21_PREPARE_STAGE_NAME,
                crate::XMAP21_EXECUTE_STAGE_NAME
            )
            .as_bytes(),
        )
    }

    pub(crate) fn authority_absent_output(hour: i64) -> FoundationFeatureOutput {
        let unproven = xmap21_unproven_authority_inputs();
        let reason = format!(
            "no admitted map-furniture authority for this hour: {} are required and {} is not \
             owner-proven",
            XMAP21_REQUIRED_AUTHORITY_INPUTS.join(", "),
            if unproven.is_empty() {
                "none of them".to_string()
            } else {
                unproven.join(", ")
            }
        );
        let witness_sha256 = Sha256::hex(
            format!("{XMAP21_FURNITURE_ABSENT_GAP_ID}\n{hour}\n{hour}\n{reason}").as_bytes(),
        );
        FoundationFeatureOutput::without_payload(
            FamilyDisposition::Unavailable {
                gap: FoundationGap {
                    gap_id: XMAP21_FURNITURE_ABSENT_GAP_ID.into(),
                    first_unavailable_hour: hour,
                    last_unavailable_hour: hour,
                    witness_sha256,
                    reason,
                },
            },
            Vec::new(),
        )
    }
}

impl ProductionMapFeatureAdapter for Xmap21MapFurnitureAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::MapFurnitureIcons
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let Self::Prepared(prepared) = self else {
            return Ok(Self::authority_absent_output(
                context.world_frame.hour_index(),
            ));
        };
        prepared.validate()?;
        let mut output = FoundationFeatureOutput::without_payload(
            FamilyDisposition::Present {
                payload_schema: crate::XMAP21_PREPARED_PACKET_SCHEMA.into(),
                producer_registration_sha256: Self::producer_registration_sha256(),
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: Vec::new(),
                ordered_used_content: Vec::new(),
            },
            XMAP21_REQUIRED_AUTHORITY_INPUTS
                .iter()
                .filter(|input| {
                    context
                        .map_authority_gaps
                        .iter()
                        .any(|gap| &gap.input_id == *input)
                })
                .map(|input| (*input).to_string())
                .collect(),
        );
        // The packet's own exclusions ride the envelope's per-element channel: one province's
        // unvouched controller keeps its icon off the map without refusing the family.
        output.element_diagnostics = prepared.element_diagnostics.clone();
        output.prepared_map_furniture = Some(prepared.as_ref().clone());
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // kills: replacing the named furniture blocker with the generic
    // "adapter_unimplemented" gap the graph used to report, and dropping either
    // required authority input from the reason the owner reads.
    #[test]
    fn the_refusal_names_the_furniture_authority_that_is_missing() {
        let output = Xmap21MapFurnitureAdapter::authority_absent_output(7);
        let FamilyDisposition::Unavailable { gap } = &output.disposition else {
            panic!("furniture must refuse while no authority is admitted");
        };
        assert_eq!(gap.gap_id, XMAP21_FURNITURE_ABSENT_GAP_ID);
        assert!(!gap.gap_id.contains("adapter_unimplemented"));
        for input in XMAP21_REQUIRED_AUTHORITY_INPUTS {
            assert!(gap.reason.contains(input), "reason omitted {input}");
        }
        // kills: a refusal that goes on calling a proven input unproven. Row
        // #20's producer landed; row #7's has not, and the reason has to say
        // which one is actually missing.
        assert_eq!(
            xmap21_unproven_authority_inputs(),
            vec!["country.current_capital_state"],
            "row #20 is owner-proven and row #7 is not"
        );
        assert!(
            !gap.reason.contains("neither is owner-proven"),
            "{}",
            gap.reason
        );
        assert!(
            gap.reason.contains("country.current_capital_state is not owner-proven"),
            "{}",
            gap.reason
        );
        assert_eq!(gap.first_unavailable_hour, 7);
        assert_eq!(gap.last_unavailable_hour, 7);
        assert!(output.prepared_map_furniture.is_none());
        // The witness must bind the hour, or two different hours would share one
        // refusal receipt.
        let other = Xmap21MapFurnitureAdapter::authority_absent_output(8);
        let FamilyDisposition::Unavailable { gap: other_gap } = &other.disposition else {
            unreachable!()
        };
        assert_ne!(gap.witness_sha256, other_gap.witness_sha256);
    }

    // kills: treating an Unavailable family as vouched in the readiness graph.
    // The earlier version asserted only on the local disposition and never
    // called readiness at all, so it stayed green if Unavailable had been made
    // to seal (Codex review, round 1). It now runs the real seal law.
    #[test]
    fn a_refusing_furniture_family_can_never_seal_a_frame() {
        use portable_render_stream::{
            FamilyCoverageRow, FoundationRenderEnvelope, MapVisualFamily,
            FOUNDATION_RENDER_SCHEMA,
        };
        let output = Xmap21MapFurnitureAdapter::authority_absent_output(7);
        assert!(output.consumed_gap_ids.is_empty());
        let envelope = FoundationRenderEnvelope {
            schema: FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: 7,
            content_manifest_sha256: "1".repeat(64),
            product_modset_sha256: "2".repeat(64),
            effective_vfs_manifest_sha256: "3".repeat(64),
            foundation_content_operation_sha256: "4".repeat(64),
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: if family == MapVisualFamily::MapFurnitureIcons {
                        output.disposition.clone()
                    } else {
                        FamilyDisposition::AuthenticatedEmpty {
                            producer_registration_sha256: "5".repeat(64),
                            enumerator_witness_sha256: "6".repeat(64),
                        }
                    },
                })
                .collect(),
            element_diagnostics: Vec::new(),
        };
        assert!(envelope.has_refusing_family());
        let error = envelope
            .seal_readiness(&std::collections::BTreeSet::new())
            .unwrap_err();
        assert!(error.contains("MapFurnitureIcons"), "{error}");
        assert!(error.contains("refuses this frame"), "{error}");
    }
}
