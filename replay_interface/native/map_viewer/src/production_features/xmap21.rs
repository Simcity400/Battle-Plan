use crate::{
    FoundationFeatureContext, FoundationFeatureOutput, ProductionMapFeatureAdapter, Result,
};
use portable_render_stream::{FamilyDisposition, FoundationGap, MapVisualFamily};
use snapshot::sha256::Sha256;

/// The map-authority inputs XMAP21 needs, read from the owner table rather than
/// restated here.
///
/// The table assigns four: `country.current_capital_state`, `mapmode.raids`,
/// `mapmode.resource_deposits` and `map_furniture.current_victory_points`. A
/// hand-written pair listed only the last two, which let the family report that
/// the recording admitted every input it owns on an hour where
/// `mapmode.resource_deposits` was unavailable, and let the global accounting
/// pass because ANY unavailable XMAP21 row satisfies it.
pub fn xmap21_required_authority_inputs() -> Vec<&'static str> {
    crate::production_foundation::map_authority_inputs_owned_by("XMAP21")
}

/// The required inputs XMAP00 does NOT yet call owner-proven.
///
/// Derived rather than written down: a second lane proving one of them must not
/// have to remember to edit a refusal string here, and a refusal that names a
/// proven input as missing is a false statement about the recording.
pub fn xmap21_unproven_authority_inputs() -> Vec<&'static str> {
    xmap21_required_authority_inputs()
        .into_iter()
        .filter(|input| crate::map_authority::map_authority_owner_proven_admission(input).is_none())
        .collect()
}

/// The recording does not admit an input this family owns.
pub const XMAP21_FURNITURE_ABSENT_GAP_ID: &str = "XMAP21.map_furniture_authority_absent";

/// The recording admits every input and the ENGINE derivations are what is
/// missing.
///
/// A separate id because the two are different defect classes and a reader (or
/// a script) routed by the id alone must not be sent to the recorder for an
/// engine gap.
pub const XMAP21_DERIVATIONS_OWED_GAP_ID: &str = "XMAP21.map_furniture_engine_derivations_owed";

/// What the draw path must present before a frame that deferred XMAP21 may seal.
pub const XMAP21_FURNITURE_DRAW_CONTRACT: &str = "XMAP21.furniture_frame_receipt";

/// XMAP21 answers at DRAW time, and only for a recording that admits every
/// input it owns.
///
/// A furniture packet is screen-space: every icon is projected through the live
/// camera and viewport, so no packet exists at once-per-hour foundation time.
/// What this adapter CAN decide here is whether the recording admits the inputs
/// the projection reads, and - when it does - whether the derivations that turn
/// them into furniture exist at all. [`crate::XMAP21_OWED_DERIVATIONS`] is the
/// second question, and it names ENGINE gaps rather than recording gaps.
pub struct Xmap21MapFurnitureAdapter;

impl Xmap21MapFurnitureAdapter {
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

    /// The recording does not admit one or more of the inputs this family owns.
    ///
    /// `blockers` carry XMAP00's own words for each gapped input, so the refusal
    /// distinguishes a key the recorder never emitted from one it emitted and
    /// could not fill, without this module restating either.
    pub(crate) fn authority_absent_output(
        hour: i64,
        gapped: &[String],
        blockers: &[String],
    ) -> FoundationFeatureOutput {
        let unproven = xmap21_unproven_authority_inputs();
        let mut reason = format!(
            "the recording does not admit XMAP21 authority input(s) {}: {}",
            gapped.join(", "),
            blockers.join(" | ")
        );
        if !unproven.is_empty() {
            reason.push_str(&format!(
                "; {} is not owner-proven either",
                unproven.join(", ")
            ));
        }
        Self::refusal(XMAP21_FURNITURE_ABSENT_GAP_ID, hour, reason, gapped.to_vec())
    }

    /// Every input is admitted and the projection still cannot run: the engine
    /// laws the furniture kinds need have not been established.
    pub(crate) fn derivation_owed_output(hour: i64) -> FoundationFeatureOutput {
        Self::refusal(
            XMAP21_DERIVATIONS_OWED_GAP_ID,
            hour,
            format!(
                "the recording admits every XMAP21 authority input ({}), and the family is held \
                 by {} ENGINE derivation(s) rather than by this recording: {}",
                xmap21_required_authority_inputs().join(", "),
                crate::XMAP21_OWED_DERIVATIONS.len(),
                crate::XMAP21_OWED_DERIVATIONS
                    .iter()
                    .map(|(_, owed)| *owed)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
            Vec::new(),
        )
    }

    fn refusal(
        gap_id: &str,
        hour: i64,
        reason: String,
        consumed_gap_ids: Vec<String>,
    ) -> FoundationFeatureOutput {
        let witness_sha256 =
            Sha256::hex(format!("{gap_id}\n{hour}\n{hour}\n{reason}").as_bytes());
        FoundationFeatureOutput::without_payload(
            FamilyDisposition::Unavailable {
                gap: FoundationGap {
                    gap_id: gap_id.into(),
                    first_unavailable_hour: hour,
                    last_unavailable_hour: hour,
                    witness_sha256,
                    reason,
                },
            },
            consumed_gap_ids,
        )
    }

    pub(crate) fn deferred_to_draw_output() -> FoundationFeatureOutput {
        FoundationFeatureOutput::without_payload(
            FamilyDisposition::DeferredToDraw {
                producer_registration_sha256: Self::producer_registration_sha256(),
                draw_contract_id: XMAP21_FURNITURE_DRAW_CONTRACT.into(),
                reason: "every furniture icon is projected through the live camera and viewport, \
                         which foundation time does not have"
                    .into(),
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
        let hour = context.world_frame.hour_index();
        let owned = xmap21_required_authority_inputs();
        let gapped = crate::account_owned_map_authority_inputs(
            "XMAP21",
            &owned,
            context.map_authority_gaps,
            context.map_authority_admitted,
        )?;
        if !gapped.is_empty() {
            let blockers = gapped
                .iter()
                .map(|input| {
                    context
                        .map_authority_gaps
                        .iter()
                        .find(|gap| &gap.input_id == input)
                        .map(|gap| format!("{input}: {}", gap.reason))
                        .ok_or_else(|| {
                            format!(
                                "XMAP21 owned-input accounting reported {input} as gapped, and \
                                 the projection does not hold it"
                            )
                        })
                })
                .collect::<std::result::Result<Vec<_>, String>>()?;
            return Ok(Self::authority_absent_output(hour, &gapped, &blockers));
        }
        if !crate::XMAP21_OWED_DERIVATIONS.is_empty() {
            return Ok(Self::derivation_owed_output(hour));
        }
        Ok(Self::deferred_to_draw_output())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Seam {
        timeline: replay_core::WorldTimeline,
        viewer: crate::production_foundation::ViewerOwnedMapState,
        selection: crate::production_features::xmap15::Xmap15ViewerSelection,
    }

    impl Seam {
        fn new() -> Self {
            use crate::production_features::xmap15::Xmap15ViewerSelection;
            use crate::production_foundation::ViewerOwnedMapState;
            use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};

            struct EmptySource;
            impl ReplaySource for EmptySource {
                fn game_id(&self) -> &str {
                    "xmap21-authority-seam"
                }
                fn hours(&self) -> &[i64] {
                    &[7, 8]
                }
                fn warnings(&self) -> &[String] {
                    &[]
                }
                fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
                    Ok(RawFrame {
                        hour_index,
                        date: None,
                        save_version: None,
                        schema_version: None,
                        rest: serde_json::Map::new(),
                    })
                }
            }

            Self {
                timeline: WorldTimeline::open(Box::new(EmptySource)).expect("timeline"),
                viewer: ViewerOwnedMapState {
                    viewport_width: 16,
                    viewport_height: 16,
                    camera: None,
                    observer_country_id: None,
                    map_mode_id: "not_a_mode".to_string(),
                    map_mode_selection_epoch: 0,
                    selected_object_ids: Vec::new(),
                    input_revision: 0,
                    animation_time_f32_bits: 0,
                    frame_delta_seconds_f32_bits: 0,
                },
                selection: Xmap15ViewerSelection {
                    selected_plan: None,
                },
            }
        }

        fn row(
            &self,
            hour: i64,
            gaps: &[crate::map_authority::MapAuthorityGap],
            admitted: &[crate::map_authority::MapAuthorityAdmittedInput],
        ) -> FoundationFeatureOutput {
            let frame = self.timeline.frame(hour).expect("frame");
            let context = crate::production_foundation::FoundationFeatureContext::new_for_owned_accounting_tests(
                &frame,
                &self.viewer,
                gaps,
                admitted,
                &self.selection,
            );
            Xmap21MapFurnitureAdapter.prepare(&context).expect("row")
        }
    }

    fn admitted_inputs() -> Vec<crate::map_authority::MapAuthorityAdmittedInput> {
        xmap21_required_authority_inputs()
            .into_iter()
            .map(
                |input| crate::map_authority::MapAuthorityAdmittedInput {
                    input_id: input.to_string(),
                    owner_packet: "XMAP21".to_string(),
                    payload_schema: "test".to_string(),
                    cadence: "hourly".to_string(),
                    identity_order: "test".to_string(),
                },
            )
            .collect()
    }

    /// kills: keeping a hand-written owned-input list beside the XMAP00 owner
    /// table. XMAP21 restated two inputs while the table assigned it more, so on
    /// `captures/rc_vanilla7` hour 17030998 it reported that the recording
    /// admitted every input it owns while a gapped one went unaccounted, and the
    /// global accounting passed because ANY unavailable XMAP21 row satisfies it.
    ///
    /// The list is TWO, not four. The owner table also assigned `mapmode.raids`
    /// and `mapmode.resource_deposits` to XMAP21 while the ledger
    /// (`MAP_AUTHORITY_OWNER_PROVEN_INPUTS`) assigns raids to XMAP18 and XMAP18's
    /// own list claims both; the r4 frame dump refused a whole frame over the
    /// double claim. XMAP21 READS raids and resource deposits to place icons,
    /// and reading is not owning.
    #[test]
    fn the_owned_inputs_are_exactly_what_the_owner_table_assigns() {
        let owned = xmap21_required_authority_inputs();
        for input in [
            "country.current_capital_state",
            "map_furniture.current_victory_points",
        ] {
            assert!(owned.contains(&input), "the table assigns {input} to XMAP21");
        }
        for not_owned in ["mapmode.raids", "mapmode.resource_deposits"] {
            assert!(
                !owned.contains(&not_owned),
                "{not_owned} is a mapmode input XMAP18 owns; two consumers of one gap is a frame                  that cannot seal"
            );
        }
        assert_eq!(owned.len(), 2, "{owned:?}");
        // ...and nothing in this module restates them.
        let source = include_str!("xmap21.rs");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("a test module")
            .0;
        for input in &owned {
            assert!(
                !production.contains(&format!("\"{input}\"")),
                "{input} is written out in production instead of read from the owner table"
            );
        }
    }

    /// kills: replacing the named furniture blocker with the generic
    /// "adapter_unimplemented" gap the graph used to report, and dropping the
    /// gapped input or XMAP00's own cause from the reason the owner reads.
    #[test]
    fn a_gapped_authority_input_is_named_with_its_cause() {
        use crate::map_authority::{MapAuthorityGap, MapAuthorityRequirementDisposition};
        let seam = Seam::new();
        let owned = xmap21_required_authority_inputs();
        let mut admitted = admitted_inputs();
        let missing = admitted.remove(1);
        let gaps = vec![MapAuthorityGap {
            input_id: missing.input_id.clone(),
            disposition: MapAuthorityRequirementDisposition::Unavailable,
            reason: "the recording carries no complete measured census".to_string(),
        }];

        let output = seam.row(7, &gaps, &admitted);
        let FamilyDisposition::Unavailable { gap } = &output.disposition else {
            panic!("furniture must refuse while an input is gapped");
        };
        assert_eq!(gap.gap_id, XMAP21_FURNITURE_ABSENT_GAP_ID);
        assert!(!gap.gap_id.contains("adapter_unimplemented"));
        assert!(gap.reason.contains(&missing.input_id), "{}", gap.reason);
        // The witness binds the id the branch SELECTED, not a fixed one: the two
        // branches carry different ids and a witness hashed over the wrong one
        // would vouch for the other branch's claim (Codex review, round 2).
        assert_eq!(
            gap.witness_sha256,
            snapshot::sha256::Sha256::hex(
                format!(
                    "{XMAP21_FURNITURE_ABSENT_GAP_ID}\n7\n7\n{}",
                    gap.reason
                )
                .as_bytes()
            )
        );
        assert!(
            gap.reason.contains("no complete measured census"),
            "{}",
            gap.reason
        );
        assert_eq!(output.consumed_gap_ids, vec![missing.input_id.clone()]);
        assert_eq!(gap.first_unavailable_hour, 7);
        assert_eq!(gap.last_unavailable_hour, 7);
        // BOTH owned inputs are owner-proven: row #20's producer landed in lane
        // OV and row #7's D14 capital-state producer in lane RJ. The one input
        // that was not - `mapmode.resource_deposits`, which has no status
        // accessor at all - turned out to belong to XMAP18, not here.
        //
        // So the refusal on this family has nothing unproven to name, and the
        // empty set is the assertion: a `prepare()` that started naming an
        // unproven input again would be naming one it does not own.
        assert!(
            xmap21_unproven_authority_inputs().is_empty(),
            "every input XMAP21 owns is owner-proven; {:?} says otherwise",
            xmap21_unproven_authority_inputs()
        );
        assert!(
            !gap.reason.contains("is not owner-proven"),
            "nothing XMAP21 owns is unproven, so the refusal must not claim one is: {}",
            gap.reason
        );

        // The witness must bind the hour, or two hours would share one receipt.
        let other = seam.row(8, &gaps, &admitted);
        let FamilyDisposition::Unavailable { gap: other_gap } = &other.disposition else {
            unreachable!()
        };
        assert_ne!(gap.witness_sha256, other_gap.witness_sha256);

        // EVERY owned input, not just the first two: a `prepare()` that
        // truncated the derived list would name a subset and still pass a test
        // that gapped only one (Codex review, round 2).
        let all_gapped: Vec<MapAuthorityGap> = owned
            .iter()
            .map(|input| MapAuthorityGap {
                input_id: (*input).to_string(),
                disposition: MapAuthorityRequirementDisposition::Unavailable,
                reason: format!("no census for {input}"),
            })
            .collect();
        let output = seam.row(7, &all_gapped, &[]);
        let FamilyDisposition::Unavailable { gap } = &output.disposition else {
            panic!("every input gapped must refuse");
        };
        assert_eq!(output.consumed_gap_ids.len(), owned.len());
        for input in &owned {
            assert!(
                output.consumed_gap_ids.contains(&(*input).to_string()),
                "{input} was not accounted"
            );
            assert!(gap.reason.contains(input), "reason omitted {input}");
            assert!(
                gap.reason.contains(&format!("no census for {input}")),
                "reason omitted the cause of {input}"
            );
        }
    }

    /// The refusal this lane corrected: with EVERY owned input admitted, the
    /// family is held by engine derivations, not by the recording - and the gap
    /// ID says so too.
    ///
    /// kills: the pre-lane text, which told a reader that a recording admitting
    /// its inputs had "no admitted map-furniture authority for this hour"; and
    /// reporting the engine-gap branch under the recording-gap identifier, which
    /// routes a reader (or a script) to the wrong defect class.
    #[test]
    fn an_admitted_authority_is_held_by_the_owed_derivations_under_its_own_gap_id() {
        let seam = Seam::new();
        let output = seam.row(7, &[], &admitted_inputs());
        let FamilyDisposition::Unavailable { gap } = &output.disposition else {
            panic!("furniture cannot vouch while derivations are owed");
        };
        assert_eq!(gap.gap_id, XMAP21_DERIVATIONS_OWED_GAP_ID);
        assert_ne!(gap.gap_id, XMAP21_FURNITURE_ABSENT_GAP_ID);
        assert!(
            !crate::XMAP21_OWED_DERIVATIONS.is_empty(),
            "emptying the owed-derivation list must restate this test, not slip past it"
        );
        assert!(
            gap.reason.contains("admits every XMAP21 authority input"),
            "{}",
            gap.reason
        );
        assert!(
            !gap.reason.contains("does not admit"),
            "the refusal still blames the recording: {}",
            gap.reason
        );
        assert_eq!(
            gap.witness_sha256,
            snapshot::sha256::Sha256::hex(
                format!(
                    "{XMAP21_DERIVATIONS_OWED_GAP_ID}\n7\n7\n{}",
                    gap.reason
                )
                .as_bytes()
            ),
            "the witness must bind the id this branch selected"
        );

        // Every admit/project entry point raises the ONE refusal constructor,
        // which is driven here for each kind. An entry point that asked the
        // keyed list and then refused on something else has nothing else to
        // return (Codex review, rounds 2 and 3).
        for (kind, owed) in crate::XMAP21_OWED_DERIVATIONS {
            let refusal =
                crate::xmap21_owed_refusal(&crate::XMAP21_OWED_DERIVATIONS, kind).to_string();
            assert!(
                refusal.starts_with("XMAP21 DATA_BLOCKED: "),
                "{kind:?}: {refusal}"
            );
            assert!(refusal.contains(owed), "{kind:?} refusal dropped its text");
            assert!(gap.reason.contains(owed), "reason omitted {owed}");
        }
        // The removed-kind arm is behaviour a test executes, not a branch that
        // only exists while every kind is owed.
        let unlanded =
            crate::xmap21_owed_refusal(&[], crate::Xmap21FurnitureKind::Raid).to_string();
        assert!(unlanded.contains("without its derivation landing"), "{unlanded}");
        assert!(!unlanded.contains("DATA_BLOCKED"), "{unlanded}");
        for input in xmap21_required_authority_inputs() {
            assert!(gap.reason.contains(input), "reason omitted {input}");
        }
        assert!(output.consumed_gap_ids.is_empty());
    }

    /// kills: letting the deferral arm vouch without a draw receipt, and
    /// letting a refusing family seal at all.
    #[test]
    fn deferring_is_not_vouching_and_refusing_never_seals() {
        use portable_render_stream::{
            FamilyCoverageRow, FoundationRenderEnvelope, MapVisualFamily,
            FOUNDATION_RENDER_SCHEMA,
        };
        let envelope = |disposition: FamilyDisposition| FoundationRenderEnvelope {
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
                        disposition.clone()
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

        let refusing = envelope(Xmap21MapFurnitureAdapter::derivation_owed_output(7).disposition);
        assert!(refusing.has_refusing_family());
        let error = refusing
            .seal_readiness(&std::collections::BTreeSet::new())
            .unwrap_err();
        assert!(error.contains("MapFurnitureIcons"), "{error}");
        assert!(error.contains("refuses this frame"), "{error}");

        let deferring = envelope(Xmap21MapFurnitureAdapter::deferred_to_draw_output().disposition);
        assert!(!deferring.has_refusing_family());
        assert!(deferring
            .seal_readiness(&std::collections::BTreeSet::new())
            .is_err());
        deferring
            .seal_readiness(&std::collections::BTreeSet::from([
                FoundationRenderEnvelope::draw_contract_key(
                    MapVisualFamily::MapFurnitureIcons,
                    XMAP21_FURNITURE_DRAW_CONTRACT,
                ),
            ]))
            .expect("a discharged furniture contract seals");
    }
}
