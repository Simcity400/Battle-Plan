use compact_contract::pending_producer::{
    PendingProducerCrate, PendingProducerSlot, RequiredSchema, WireAbsence,
};

pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "xmap20_effects",
    ledger_row: "QJ content.effects (XMAP02 re-judge) + effects.semantic_lifecycle",
    blocked_on:
        "the four open carrier gates XMAP20_OPEN_CARRIER_GATES \
(map_viewer/src/production_features/xmap20.rs: G2 lifecycle sequencing, G5 census \
completeness and snapshot safety, G6 unattributed trigger receipt, G7 unsourced \
required wire fields) as recorded by \
replay_interface/re_evidence/rendering_laws/XMAP20/effect_census_authority.json. \
G7 alone names three REQUIRED wire fields with no engine source \
(field_sourcing_census): authored_event_index, authored_effect_index and lifecycle \
elapsed_f32_bits - validate_event accepts zero for each, so a producer landing today \
would emit invented zeros indistinguishable from measurements. G7's own named closing \
step for the first two - the writer census for owner+0x3b0 / owner+0x3bc - is \
PERFORMED. Its outcome and every bound on it are deliberately not restated \
here; they are stated in \
portable_engine_evidence::xmap20_particle_queue_producer::G7_ROUTE_REFUTATION and \
::WHAT_THIS_DOES_NOT_SETTLE. \
elapsed_f32_bits is untouched and still has its own \
closing steps. Also open under that \
census: effect_instance_id has no proven stable identity keying (needs_design; the \
(pointer, registration_hash_u32) key detects most but not all address reuse), and \
EffectOperation is only partly derivable (Reparent has no discriminator; StopEmission \
has an unproven candidate flag at instance+0x55). Fields this crate does NOT treat as \
blockers, because the same census live-confirms them readable: registration token and \
FNV hash, time scale (node+0x78), SceneNode parent anchor (descriptor+0x10 token, \
node+0x10 transform) and active flags bit 0. RawParticlePacket \
(production_render_submit_live/src/lib.rs::RawParticlePacket) carries only DefinitionIdentity \
(registration_token, definition_source_token, registration_hash_u32) - the resolved \
definition digest exists solely post-catalog on the portable ParticlePacket. Finally \
owed: an hourly drain of the promoted submit stream into fold state (collector \
drain_a729_runtime_captures_in_process currently has zero callers outside collector).",
    reason:
        "the replay consumer chain is complete and unconditional (replay_core/src/world.rs \
build_xmap20_effects decodes root key xmap20_effects; replay_app/src/app.rs derives the \
five provider receipts and requires the carrier every hour), so the gap must be closed \
by observation, never by a sealed empty frame that would claim a complete enumeration \
of effects the game did run",
};

pub const OWED_CARRIERS: [PendingProducerSlot; 1] = [PendingProducerSlot {
    wire_key: "xmap20_effects",
    scope: "root",
    ledger_row: "QJ content.effects (XMAP02 re-judge)",
    consumer_citation: "replay_core/src/world.rs::build_xmap20_effects",
    required_schema: RequiredSchema::Token("hoi4.xmap20.effect-frame.a729.v1"),
    wire_absence: WireAbsence::TypedNull,
    reason: "carrier gates G2/G5/G6/G7 are open (see effect_census_authority.json); a typed \
null keeps the replay's named XMAP20 requirement error instead of a silent render \
without effects",
}];

pub fn install() -> Result<(), String> {
    Err(PENDING.refusal())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE G7 OUTCOME IS NOT RESTATED IN THIS CRATE. Rounds 7 through 13 of
    /// adversarial review each found a copy of that text drifting out of step
    /// with the original, and this string was one of the copies. It may now
    /// carry only the pointer.
    ///
    /// This checks THIS string against the identifiers and outcome sentences
    /// that actually drifted into it. It is a regression guard over those, not
    /// a proof that no paraphrase could be written here.
    ///
    /// kills: a future edit that re-states the engine finding here, which is
    /// the exact defect seven consecutive rounds found.
    #[test]
    fn the_blocked_on_text_points_at_the_g7_finding_and_does_not_restate_it() {
        let text = PENDING.blocked_on;
        for engine_identifier in [
            "CSdlEvents",
            "SDL_DROPFILE",
            "SDL_DropEvent",
            "14234ddb0",
            "14234cf00",
            "1420a42b0",
            "142089670",
            "1422e3280",
            "drag-and-drop",
            // Not just identifiers: the OUTCOME sentence is a restatement too,
            // and it is what survived three sweeps (Codex review, round 14).
            "sources neither index",
            "sources nothing",
            "SOURCES NOTHING",
        ] {
            assert!(
                !text.contains(engine_identifier),
                "the pending-producer text restates the G7 finding ({engine_identifier}); it must \
                 point at portable_engine_evidence::xmap20_particle_queue_producer instead"
            );
        }
        assert!(
            text.contains("xmap20_particle_queue_producer::G7_ROUTE_REFUTATION")
                && text.contains("WHAT_THIS_DOES_NOT_SETTLE"),
            "the pending-producer text no longer points at the constants that carry the finding"
        );
    }

    #[test]
    fn an_unbuilt_producer_refuses_installation_by_name() {
        let error = install().expect_err("a pending producer must not claim an install");
        assert!(error.contains("xmap20_effects"), "{error}");
        assert!(
            error.contains("XMAP20-G7") || error.contains("G7"),
            "refusal must cite gate G7: {error}"
        );
        assert!(
            error.contains("authored_event_index"),
            "refusal must name an unsourced G7 field: {error}"
        );
    }

    #[test]
    fn owed_carriers_record_the_schema_their_consumer_requires() {
        assert_eq!(
            OWED_CARRIERS[0].required_schema.token(),
            Some("hoi4.xmap20.effect-frame.a729.v1")
        );
        assert!(OWED_CARRIERS[0].wire_absence.emits_key_when_unfed());
        for carrier in &OWED_CARRIERS {
            carrier
                .validate()
                .expect("owed carrier must be well formed");
        }
        PENDING
            .validate()
            .expect("the crate declaration must be well formed");
    }
}
