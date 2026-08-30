use compact_contract::pending_producer::{
    PendingProducerCrate, PendingProducerSlot, RequiredSchema, WireAbsence,
};

pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "xui26_frontend",
    ledger_row: "EIR-NE-015",
    blocked_on: "a typed-payload producer for the twelve XUI26 frontend families; every one of \
                 them is missing a different operand and FAMILY_CAUSES says which",
    reason: "the element-semantic requirement that used to block this slot is GONE. The consumer \
             asked for a resolved semantic for every authored element of each panel, which is \
             POV-local presentation the owner ruled out of capture scope on 2026-07-28 and which \
             no recorder could produce; the replay now derives those semantics, and the row \
             templates and collection hosts with them, from the effective GUI it already \
             authenticates. What is left to produce is the typed payload each panel displays",
};

pub const OWED_CARRIERS: [PendingProducerSlot; 1] = [PendingProducerSlot {
    wire_key: "_r2_xui26_frontend_v1",
    scope: "root",
    ledger_row: "EIR-NE-015",
    consumer_citation: "replay_core/src/xui26_frontend.rs:8",
    required_schema: RequiredSchema::Token("replay.xui26-frontend.v2"),
    wire_absence: WireAbsence::TypedNull,
    reason: "no frontend producer. The carrier the consumer now admits is typed payload plus \
             collection membership and order, with no resolved element semantics and no template \
             or host identity: a recorder cannot know the replay's role-index identities, and \
             the ones it could know are derived from the same authored file the payload is \
             rendered into",
}];

/// WHY each family the frozen XUI26 contract declares a replay-payload slot for
/// is not produced. One row per family, and never a bare "not recorded": a
/// blocker that does not say which operand is missing sends the next lane at a
/// capture it does not need, which is exactly what happened to this slot.
pub const FAMILY_CAUSES: [(&str, &str); 12] = [
    (
        "frontend_game_setup",
        "pre-attach selection: the scenario, bookmark and country are chosen before a world \
         exists, so a recorder that attaches to a running game never observes them",
    ),
    (
        "tutorial_screen",
        "pre-attach selection: the tutorial and its step are chosen before attach",
    ),
    (
        "saved_game_rules",
        "pre-attach selection: the rules digest is passively readable, but the assignments are a \
         savedgamerules_load choice made before attach",
    ),
    (
        "player_ai_preferences",
        "pre-attach selection: playeraiprefs is a frontend screen and nothing in this tree reads \
         its assignments",
    ),
    (
        "settings",
        "client config not parsed: modset.rs hashes settings.txt at attach and at seal, so the \
         file identity IS on the wire, but nothing parses it into assignments",
    ),
    (
        "message_settings",
        "client config not parsed: the recorder-manifest route has no implementation and the \
         source file behind MessageSettingWindow is unidentified",
    ),
    (
        "message_filters",
        "client config not parsed: same unidentified source as message_settings",
    ),
    (
        "ingame_lobby",
        "operand not read: the member set is on the wire as domain 57's players_by_country, but \
         the SLOT ORDER the panel is keyed on is never read - the collector writes the sentinel \
         u32::MAX and the sidecar publishes session_slot as not_sourced",
    ),
    (
        "player_lobby",
        "operand not read: the same missing slot order as ingame_lobby",
    ),
    (
        "hotjoin",
        "the wire state is NOT the panel state: hotjoin_request_entry is the host's PENDING \
         accept/decline list, and domain 57's Hotjoin rows are roster transitions minted by \
         Domain57EventTracker::apply_player_snapshot - subtype 1 when a player appeared in the \
         roster and subtype 2 when one disappeared. Both are observed after the decision the \
         panel is asking for, and subtype 2 is a leave. Rendering them as requests would be a \
         substituted value, not a late one",
    ),
    (
        "matchmaking_chat",
        "on the wire, unrouted: domain 57 carries Chat and SystemMessage rows and the contract's \
         own source route names them, but nothing builds the carrier, and the row template proves \
         no authored text store, so its rows would draw blank",
    ),
    (
        "ai_trace",
        "developer window: ai_trace_window is an in-game developer surface behind the debug \
         controller, not a frontend screen, and nothing produces its entries",
    ),
];

pub fn install() -> Result<(), String> {
    Err(PENDING.refusal())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unbuilt_producer_refuses_installation_by_name() {
        let error = install().expect_err("a pending producer must not claim an install");
        assert!(error.contains("xui26_frontend"), "{error}");
    }

    /// kills: restoring EITHER of this slot's two refuted blockers.
    ///
    /// "No capture has yet exercised the path" was refuted for the families
    /// whose operands are already on the wire; and the element-semantic map is
    /// no longer asked of anyone, because the replay derives it. A blocker
    /// naming either would send the next lane at work that is not the work.
    #[test]
    fn the_blocker_is_a_typed_payload_producer_not_a_capture_or_element_semantics() {
        let blocked = PENDING.blocked_on.to_ascii_lowercase();
        assert!(
            !blocked.contains("capture"),
            "the frontend-capture blocker was refuted: {}",
            PENDING.blocked_on
        );
        assert!(
            !blocked.contains("element-semantic") && !blocked.contains("owner decision"),
            "the element-semantic requirement is closed, not owner-blocked: {}",
            PENDING.blocked_on
        );
        assert!(blocked.contains("typed-payload producer"), "{}", PENDING.blocked_on);
        PENDING
            .validate()
            .expect("the crate declaration must be well formed");
    }

    /// kills: a family whose blocker is a bare "not recorded". Each of the
    /// twelve names the operand that is missing, so the next lane can tell a
    /// pre-attach selection from an unread engine field from a route nobody
    /// built.
    #[test]
    fn every_declared_family_names_the_operand_it_is_missing() {
        // THE EXACT ROSTER. The frozen XUI26 promotion contract lives in the
        // replay tree, which this crate cannot read across the single-file
        // boundary, so the twelve family ids it declares a replay-payload slot
        // for are written out here. A family swapped for a thirteenth id, or
        // one silently dropped, fails on this set rather than on a count.
        let declared = std::collections::BTreeSet::from([
            "ai_trace",
            "frontend_game_setup",
            "hotjoin",
            "ingame_lobby",
            "matchmaking_chat",
            "message_filters",
            "message_settings",
            "player_ai_preferences",
            "player_lobby",
            "saved_game_rules",
            "settings",
            "tutorial_screen",
        ]);
        let mut seen = std::collections::BTreeSet::new();
        for (family_id, cause) in FAMILY_CAUSES {
            assert!(seen.insert(family_id), "family {family_id} is decided twice");
            assert!(
                cause.len() > 40 && cause.contains(':'),
                "family {family_id} does not name its missing operand: {cause}"
            );
        }
        assert_eq!(seen, declared);
    }

    #[test]
    fn the_owed_carrier_records_the_schema_its_consumer_requires() {
        assert_eq!(
            OWED_CARRIERS[0].required_schema.token(),
            Some("replay.xui26-frontend.v2")
        );
        OWED_CARRIERS[0]
            .validate()
            .expect("owed carrier must be well formed");
    }
}
