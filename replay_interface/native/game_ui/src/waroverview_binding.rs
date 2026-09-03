
use replay_core::{CountryIdentityLeaf, CountryIdentityRosterLeaf};
use replay_core::NamedCountry;
use replay_core::{
    DiplomacyView, Leaf, ViewStatus, WarObjectRow, WarParticipantOverviewRow, WorldFrame,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostilityReasonToken {
    pub raw: i64,
    pub registry_token_id: u32,
    pub spelling: &'static str,
}

pub const fn resolve_hostility_reason_token(raw: i64) -> Option<HostilityReasonToken> {
    let (registry_token_id, spelling) = match raw {
        0 => (0x298D, "war"),
        1 => (0x30D1, "puppet"),
        2 => (0x299A, "ally"),
        3 => (0x381F, "asked_to_join"),
        4 => (0x28DA, "guarantee"),
        5 => (0x3820, "not_applicable"),
        _ => return None,
    };
    Some(HostilityReasonToken {
        raw,
        registry_token_id,
        spelling,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarResidualSummary {
    pub threat_fixed5_raw: i64,
    pub threat_fixed_scale: i64,
    pub first_was_instigator: bool,
    pub hostility_reason_instigator: HostilityReasonToken,
    pub hostility_reason_defender: HostilityReasonToken,
}

pub const WAR_THREAT_FIXED_SCALE: i64 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WarResidualAdmission {
    Absent,
    Incomplete { field: &'static str },
    Inconsistent { reason: String },
    Measured(WarResidualSummary),
}

pub fn admit_war_object_residuals(row: &WarObjectRow) -> WarResidualAdmission {
    admit_war_residuals(
        &row.threat_fixed5_raw,
        &row.threat_fixed_scale,
        &row.first_was_instigator,
        &row.hostility_reason_instigator_raw,
        &row.hostility_reason_instigator,
        &row.hostility_reason_defender_raw,
        &row.hostility_reason_defender,
    )
}

fn admit_war_residuals(
    threat_fixed5_raw: &Leaf<i64>,
    threat_fixed_scale: &Leaf<i64>,
    first_was_instigator: &Leaf<bool>,
    hostility_reason_instigator_raw: &Leaf<i64>,
    hostility_reason_instigator: &Leaf<String>,
    hostility_reason_defender_raw: &Leaf<i64>,
    hostility_reason_defender: &Leaf<String>,
) -> WarResidualAdmission {
    if matches!(threat_fixed5_raw, Leaf::Absent)
        && matches!(threat_fixed_scale, Leaf::Absent)
        && matches!(first_was_instigator, Leaf::Absent)
        && matches!(hostility_reason_instigator_raw, Leaf::Absent)
        && matches!(hostility_reason_instigator, Leaf::Absent)
        && matches!(hostility_reason_defender_raw, Leaf::Absent)
        && matches!(hostility_reason_defender, Leaf::Absent)
    {
        return WarResidualAdmission::Absent;
    }

    let Some(&threat_fixed5_raw) = threat_fixed5_raw.measured() else {
        return WarResidualAdmission::Incomplete { field: "threat_fixed5_raw" };
    };
    let Some(&threat_fixed_scale) = threat_fixed_scale.measured() else {
        return WarResidualAdmission::Incomplete { field: "threat_fixed_scale" };
    };
    let Some(&first_was_instigator) = first_was_instigator.measured() else {
        return WarResidualAdmission::Incomplete { field: "first_was_instigator" };
    };
    let Some(&instigator_raw) = hostility_reason_instigator_raw.measured() else {
        return WarResidualAdmission::Incomplete {
            field: "hostility_reason_instigator_raw",
        };
    };
    let Some(instigator_spelling) = hostility_reason_instigator.measured() else {
        return WarResidualAdmission::Incomplete {
            field: "hostility_reason_instigator",
        };
    };
    let Some(&defender_raw) = hostility_reason_defender_raw.measured() else {
        return WarResidualAdmission::Incomplete {
            field: "hostility_reason_defender_raw",
        };
    };
    let Some(defender_spelling) = hostility_reason_defender.measured() else {
        return WarResidualAdmission::Incomplete {
            field: "hostility_reason_defender",
        };
    };

    let Some(hostility_reason_instigator) = resolve_hostility_reason_token(instigator_raw) else {
        return WarResidualAdmission::Inconsistent {
            reason: format!(
                "instigator hostility-reason discriminator {instigator_raw} is outside the \
                 exact-a729 registry"
            ),
        };
    };
    let Some(hostility_reason_defender) = resolve_hostility_reason_token(defender_raw) else {
        return WarResidualAdmission::Inconsistent {
            reason: format!(
                "defender hostility-reason discriminator {defender_raw} is outside the \
                 exact-a729 registry"
            ),
        };
    };
    if threat_fixed_scale != WAR_THREAT_FIXED_SCALE {
        return WarResidualAdmission::Inconsistent {
            reason: format!(
                "threat_fixed_scale {threat_fixed_scale} is not the exact-a729 scale \
                 {WAR_THREAT_FIXED_SCALE}"
            ),
        };
    }
    if instigator_spelling.as_str() != hostility_reason_instigator.spelling {
        return WarResidualAdmission::Inconsistent {
            reason: format!(
                "instigator spelling {instigator_spelling:?} disagrees with the spelling raw \
                 {instigator_raw} resolves to"
            ),
        };
    }
    if defender_spelling.as_str() != hostility_reason_defender.spelling {
        return WarResidualAdmission::Inconsistent {
            reason: format!(
                "defender spelling {defender_spelling:?} disagrees with the spelling raw \
                 {defender_raw} resolves to"
            ),
        };
    }

    WarResidualAdmission::Measured(WarResidualSummary {
        threat_fixed5_raw,
        threat_fixed_scale,
        first_was_instigator,
        hostility_reason_instigator,
        hostility_reason_defender,
    })
}

fn leaf_from_admission(admission: WarResidualAdmission) -> Leaf<WarResidualSummary> {
    match admission {
        WarResidualAdmission::Absent => Leaf::Absent,
        WarResidualAdmission::Incomplete { .. } | WarResidualAdmission::Inconsistent { .. } => {
            Leaf::Unobtained
        }
        WarResidualAdmission::Measured(summary) => Leaf::Measured(summary),
    }
}

fn resolve_war_residuals(
    threat_fixed5_raw: &Leaf<i64>,
    threat_fixed_scale: &Leaf<i64>,
    first_was_instigator: &Leaf<bool>,
    hostility_reason_instigator_raw: &Leaf<i64>,
    hostility_reason_instigator: &Leaf<String>,
    hostility_reason_defender_raw: &Leaf<i64>,
    hostility_reason_defender: &Leaf<String>,
) -> Leaf<WarResidualSummary> {
    leaf_from_admission(admit_war_residuals(
        threat_fixed5_raw,
        threat_fixed_scale,
        first_was_instigator,
        hostility_reason_instigator_raw,
        hostility_reason_instigator,
        hostility_reason_defender_raw,
        hostility_reason_defender,
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub struct WarRelationSummary {
    pub first: replay_core::CountryIdentityRef,
    pub second: replay_core::CountryIdentityRef,
    pub first_casualties: Option<i64>,
    pub second_casualties: Option<i64>,
    pub residuals: Leaf<WarResidualSummary>,
}

// NO `Default`. It minted `first: String::new()` -- an empty country name that
// no census ever admitted -- and a defaulted summary is a war between two
// countries that do not exist. With `first`/`second` typed there is nothing to
// default them to, which is the point: the two belligerents come from the row
// or the summary does not exist. Nothing called it.

#[derive(Clone, Debug, PartialEq)]
pub struct WarParticipantSummary {
    pub tag: Leaf<replay_core::CountryIdentityRef>,
    pub tag_db_id: Leaf<i64>,
    pub participation_raw: Leaf<i64>,
    pub capitulation_progress_raw: Leaf<i64>,
    pub breakdown: Leaf<replay_core::WarParticipationBreakdown>,
}

impl From<&WarParticipantOverviewRow> for WarParticipantSummary {
    fn from(row: &WarParticipantOverviewRow) -> Self {
        Self {
            tag: row.tag.clone(),
            tag_db_id: row.tag_db_id.clone(),
            participation_raw: row.participation_raw.clone(),
            capitulation_progress_raw: row.capitulation_progress_raw.clone(),
            breakdown: row.breakdown.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WarObjectSummary {
    pub source_key: String,
    pub war_id: Leaf<i64>,
    pub producer_ordinal: Leaf<i64>,
    pub aggregate_summary_flag: Leaf<bool>,
    pub war_name: Leaf<String>,
    pub attackers: Leaf<Vec<replay_core::CountryIdentityRef>>,
    pub defenders: Leaf<Vec<replay_core::CountryIdentityRef>>,
    pub attacker_participants: Leaf<Vec<WarParticipantSummary>>,
    pub defender_participants: Leaf<Vec<WarParticipantSummary>>,
    pub overall_progress_raw: Leaf<i64>,
    pub front: Leaf<String>,
    pub start_hour: Leaf<i64>,
    pub first: Leaf<replay_core::CountryIdentityRef>,
    pub second: Leaf<replay_core::CountryIdentityRef>,
    pub first_tag_db_id: Leaf<i64>,
    pub second_tag_db_id: Leaf<i64>,
    pub start_date: Leaf<String>,
    pub first_casualties: Leaf<i64>,
    pub second_casualties: Leaf<i64>,
    pub residuals: Leaf<WarResidualSummary>,
}

fn participant_summaries(
    rows: &Leaf<Vec<WarParticipantOverviewRow>>,
) -> Leaf<Vec<WarParticipantSummary>> {
    match rows {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(rows) => Leaf::Measured(rows.iter().map(Into::into).collect()),
    }
}

fn war_object(row: &WarObjectRow) -> WarObjectSummary {
    WarObjectSummary {
        source_key: row.key.clone(),
        war_id: row.war_id.clone(),
        producer_ordinal: row.producer_ordinal.clone(),
        aggregate_summary_flag: row.aggregate_summary_flag.clone(),
        war_name: row.war_name.clone(),
        attackers: row.attackers.clone(),
        defenders: row.defenders.clone(),
        attacker_participants: participant_summaries(&row.attacker_participants),
        defender_participants: participant_summaries(&row.defender_participants),
        overall_progress_raw: row.overall_progress_raw.clone(),
        front: row.front.clone(),
        start_hour: row.start_hour.clone(),
        first: row.first.clone(),
        second: row.second.clone(),
        first_tag_db_id: row.first_tag_db_id.clone(),
        second_tag_db_id: row.second_tag_db_id.clone(),
        start_date: row.start_date.clone(),
        first_casualties: row.first_casualties.clone(),
        second_casualties: row.second_casualties.clone(),
        residuals: leaf_from_admission(admit_war_object_residuals(row)),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WarOverviewBinding {
    pub pov_tag: String,
    pub available: bool,
    pub war_relations: Leaf<Vec<WarRelationSummary>>,
    pub war_objects: Leaf<Vec<WarObjectSummary>>,
    pub belligerents: Vec<String>,
    pub pov_belligerents: Vec<String>,
}

impl Default for WarOverviewBinding {
    fn default() -> Self {
        Self {
            pov_tag: String::new(),
            available: false,
            war_relations: Leaf::Absent,
            war_objects: Leaf::Absent,
            belligerents: Vec::new(),
            pov_belligerents: Vec::new(),
        }
    }
}

fn war_object_summaries(view: &DiplomacyView) -> Leaf<Vec<WarObjectSummary>> {
    match &view.wars {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(rows) => Leaf::Measured(rows.iter().map(war_object).collect()),
    }
}

fn war_relation_summaries(view: &DiplomacyView) -> Leaf<Vec<WarRelationSummary>> {
    let rows = match &view.war_relations {
        Leaf::Measured(rows) => rows,
        Leaf::Absent => return Leaf::Absent,
        Leaf::Unobtained => return Leaf::Unobtained,
    };
    let summaries = rows
        .iter()
        .filter_map(|r| {
            let first = r.first.measured()?.clone();
            let second = r.second.measured()?.clone();
            Some(WarRelationSummary {
                first,
                second,
                first_casualties: r.first_casualties.measured().copied(),
                second_casualties: r.second_casualties.measured().copied(),
                residuals: resolve_war_residuals(
                    &r.threat_fixed5_raw,
                    &r.threat_fixed_scale,
                    &r.first_was_instigator,
                    &r.hostility_reason_instigator_raw,
                    &r.hostility_reason_instigator,
                    &r.hostility_reason_defender_raw,
                    &r.hostility_reason_defender,
                ),
            })
        })
        .collect();
    Leaf::Measured(summaries)
}

fn belligerents(
    view: &DiplomacyView,
    summaries: &Leaf<Vec<WarRelationSummary>>,
    wars: &Leaf<Vec<WarObjectSummary>>,
) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    if let Leaf::Measured(summaries) = summaries {
        for s in summaries {
            // DERIVED from admitted identities, so the belligerent roster
            // spells the TAGs: the census was asked where the row was decoded.
            set.insert(s.first.tag().to_string());
            set.insert(s.second.tag().to_string());
        }
    }
    if let Leaf::Measured(rows) = wars {
        for row in rows {
            if let Leaf::Measured(tags) = &row.attackers {
                set.extend(tags.iter().map(|tag| tag.tag().to_string()));
            }
            if let Leaf::Measured(tags) = &row.defenders {
                set.extend(tags.iter().map(|tag| tag.tag().to_string()));
            }
        }
    }
    for (a, b) in view.at_war_pairs() {
        set.insert(a.to_string());
        set.insert(b.to_string());
    }
    set.into_iter().collect()
}

fn pov_belligerents(view: &DiplomacyView, pov_tag: &str) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for (a, b) in view.at_war_pairs() {
        if a == pov_tag {
            set.insert(b.to_string());
        }
    }
    set.into_iter().collect()
}

pub fn war_overview_from_frame(frame: &WorldFrame, pov_tag: NamedCountry<'_>) -> WarOverviewBinding {
    let view = frame.diplomacy();
    // The caller's selector is not a country when the census says two ids reach
    // it, and a belligerent list for one is a claim about nobody.
    if frame.country_key_is_contended(pov_tag.tag()) || view.status != ViewStatus::Present {
        return WarOverviewBinding {
            pov_tag: pov_tag.to_string(),
            available: false,
            ..WarOverviewBinding::default()
        };
    }
    let war_relations = war_relation_summaries(view);
    let war_objects = war_object_summaries(view);
    let belligerents = belligerents(view, &war_relations, &war_objects);
    let pov_belligerents = pov_belligerents(view, pov_tag.tag());
    WarOverviewBinding {
        pov_tag: pov_tag.to_string(),
        available: true,
        war_relations,
        war_objects,
        belligerents,
        pov_belligerents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{DiplomaticPairState, WarRelationRow};
    use std::collections::BTreeMap;

    fn pair_at_war() -> DiplomaticPairState {
        DiplomaticPairState {
            at_war: Leaf::Measured(true),
            same_faction: Leaf::Absent,
            non_aggression_pact: Leaf::Absent,
            military_access: Leaf::Absent,
            guarantee: Leaf::Absent,
            embargo: Leaf::Absent,
            subject: Leaf::Absent,
        }
    }

    fn view_with(
        status: ViewStatus,
        pairs: Vec<((&str, &str), DiplomaticPairState)>,
        war_relations: Leaf<Vec<WarRelationRow>>,
    ) -> DiplomacyView {
        let mut view = DiplomacyView::stating_nothing_for_consumer_tests();
        view.status = status;
        view.war_relations = war_relations;
        for ((a, b), p) in pairs {
            view.state_pair_for_consumer_tests(a, b, p);
        }
        view
    }

    #[test]
    fn casualty_rows_and_belligerents_populate() {
        let view = view_with(
            ViewStatus::Present,
            vec![
                (("GER", "ENG"), pair_at_war()),
                (("ENG", "GER"), pair_at_war()),
            ],
            Leaf::Measured(vec![WarRelationRow {
                first: Leaf::Measured(crate::test_identity::identity("GER")),
                second: Leaf::Measured(crate::test_identity::identity("ENG")),
                first_casualties: Leaf::Measured(1200),
                second_casualties: Leaf::Measured(900),
                threat_fixed5_raw: Leaf::Absent,
                threat_fixed_scale: Leaf::Absent,
                first_was_instigator: Leaf::Absent,
                hostility_reason_instigator_raw: Leaf::Absent,
                hostility_reason_instigator: Leaf::Absent,
                hostility_reason_defender_raw: Leaf::Absent,
                hostility_reason_defender: Leaf::Absent,
            }]),
        );
        let summaries = war_relation_summaries(&view);
        assert_eq!(
            summaries,
            Leaf::Measured(vec![WarRelationSummary {
                first: crate::test_identity::identity("GER"),
                second: crate::test_identity::identity("ENG"),
                first_casualties: Some(1200),
                second_casualties: Some(900),
                residuals: Leaf::Absent,
            }])
        );
        assert_eq!(
            belligerents(&view, &summaries, &Leaf::Absent),
            vec!["ENG", "GER"]
        );
        assert_eq!(pov_belligerents(&view, "GER"), vec!["ENG"]);
    }

    #[test]
    fn absent_view_is_fail_closed() {
        let view = view_with(ViewStatus::DomainAbsent, vec![], Leaf::Absent);
        assert_eq!(war_relation_summaries(&view), Leaf::Absent);
        assert!(belligerents(&view, &Leaf::Absent, &Leaf::Absent).is_empty());
    }

    #[test]
    fn war_relations_leaf_states_stay_distinct() {
        let measured_empty = view_with(ViewStatus::Present, vec![], Leaf::Measured(vec![]));
        let absent = view_with(ViewStatus::Present, vec![], Leaf::Absent);
        let unobtained = view_with(ViewStatus::Present, vec![], Leaf::Unobtained);
        assert_eq!(
            war_relation_summaries(&measured_empty),
            Leaf::Measured(vec![])
        );
        assert_eq!(war_relation_summaries(&absent), Leaf::Absent);
        assert_eq!(war_relation_summaries(&unobtained), Leaf::Unobtained);

        let measured = WarOverviewBinding {
            war_relations: war_relation_summaries(&measured_empty),
            ..WarOverviewBinding::default()
        };
        let degraded = WarOverviewBinding {
            war_relations: war_relation_summaries(&unobtained),
            ..WarOverviewBinding::default()
        };
        assert_ne!(measured.war_relations, degraded.war_relations);
    }

    #[test]
    fn unmeasured_casualties_are_none_not_zero() {
        let view = view_with(
            ViewStatus::Present,
            vec![],
            Leaf::Measured(vec![WarRelationRow {
                first: Leaf::Measured(crate::test_identity::identity("ITA")),
                second: Leaf::Measured(crate::test_identity::identity("ENG")),
                first_casualties: Leaf::Unobtained,
                second_casualties: Leaf::Absent,
                threat_fixed5_raw: Leaf::Absent,
                threat_fixed_scale: Leaf::Absent,
                first_was_instigator: Leaf::Absent,
                hostility_reason_instigator_raw: Leaf::Absent,
                hostility_reason_instigator: Leaf::Absent,
                hostility_reason_defender_raw: Leaf::Absent,
                hostility_reason_defender: Leaf::Absent,
            }]),
        );
        let Leaf::Measured(s) = war_relation_summaries(&view) else {
            panic!("measured feed must stay measured")
        };
        assert_eq!(s[0].first_casualties, None);
        assert_eq!(s[0].second_casualties, None);
    }

    #[test]
    fn exact_b5_residual_tuple_resolves_authenticated_tokens() {
        let view = view_with(
            ViewStatus::Present,
            vec![],
            Leaf::Measured(vec![WarRelationRow {
                first: Leaf::Measured(crate::test_identity::identity("GER")),
                second: Leaf::Measured(crate::test_identity::identity("ENG")),
                first_casualties: Leaf::Measured(1200),
                second_casualties: Leaf::Measured(900),
                threat_fixed5_raw: Leaf::Measured(1_795_990),
                threat_fixed_scale: Leaf::Measured(100_000),
                first_was_instigator: Leaf::Measured(true),
                hostility_reason_instigator_raw: Leaf::Measured(0),
                hostility_reason_instigator: Leaf::Measured("war".to_string()),
                hostility_reason_defender_raw: Leaf::Measured(5),
                hostility_reason_defender: Leaf::Measured("not_applicable".to_string()),
            }]),
        );
        let Leaf::Measured(rows) = war_relation_summaries(&view) else {
            panic!("measured feed must stay measured")
        };
        let Leaf::Measured(residuals) = &rows[0].residuals else {
            panic!("exact H-B5 tuple was not admitted")
        };
        assert_eq!(residuals.threat_fixed5_raw, 1_795_990);
        assert!(residuals.first_was_instigator);
        assert_eq!(residuals.hostility_reason_instigator.registry_token_id, 0x298D);
        assert_eq!(residuals.hostility_reason_defender.registry_token_id, 0x3820);
    }

    #[test]
    fn d60_war_object_preserves_pair_identity_and_b5_tuple() {
        let row = WarObjectRow {
            key: "GER|ENG".to_string(),
            war_id: Leaf::Absent,
            producer_ordinal: Leaf::Absent,
            aggregate_summary_flag: Leaf::Absent,
            attackers: Leaf::Absent,
            defenders: Leaf::Absent,
            war_name: Leaf::Absent,
            front: Leaf::Absent,
            start_hour: Leaf::Absent,
            attacker_participants: Leaf::Absent,
            defender_participants: Leaf::Absent,
            overall_progress_raw: Leaf::Absent,
            first: Leaf::Measured(crate::test_identity::identity("GER")),
            second: Leaf::Measured(crate::test_identity::identity("ENG")),
            first_tag_db_id: Leaf::Measured(47),
            second_tag_db_id: Leaf::Measured(1),
            start_date: Leaf::Measured("1939.9.1.12".to_string()),
            first_casualties: Leaf::Measured(1_200),
            second_casualties: Leaf::Measured(900),
            threat_fixed5_raw: Leaf::Measured(2_000_000),
            threat_fixed_scale: Leaf::Measured(100_000),
            first_was_instigator: Leaf::Measured(true),
            hostility_reason_instigator_raw: Leaf::Measured(4),
            hostility_reason_instigator: Leaf::Measured("guarantee".to_string()),
            hostility_reason_defender_raw: Leaf::Measured(2),
            hostility_reason_defender: Leaf::Measured("ally".to_string()),
            casualties_semantics: Leaf::Absent,
            casualties_is_save_snapshot: Leaf::Absent,
        };
        let summary = war_object(&row);
        assert_eq!(summary.first.tag(), Leaf::Measured("GER"));
        assert_eq!(summary.second_tag_db_id, Leaf::Measured(1));
        let Leaf::Measured(residuals) = summary.residuals else {
            panic!("D60 H-B5 tuple was not admitted")
        };
        assert_eq!(residuals.threat_fixed5_raw, 2_000_000);
        assert_eq!(residuals.hostility_reason_instigator.registry_token_id, 0x28DA);
        assert_eq!(residuals.hostility_reason_defender.registry_token_id, 0x299A);
    }

    #[test]
    fn b5_resolver_rejects_partial_mismatched_and_unknown_tokens() {
        let raw = Leaf::Measured(3);
        let asked = Leaf::Measured("asked_to_join".to_string());
        let guarantee_raw = Leaf::Measured(4);
        let guarantee = Leaf::Measured("guarantee".to_string());
        assert!(matches!(
            resolve_war_residuals(
                &Leaf::Measured(1),
                &Leaf::Measured(100_000),
                &Leaf::Measured(false),
                &raw,
                &asked,
                &guarantee_raw,
                &guarantee,
            ),
            Leaf::Measured(_)
        ));

        for rejected in [
            resolve_war_residuals(
                &Leaf::Measured(1),
                &Leaf::Measured(1),
                &Leaf::Measured(false),
                &raw,
                &asked,
                &guarantee_raw,
                &guarantee,
            ),
            resolve_war_residuals(
                &Leaf::Measured(1),
                &Leaf::Measured(100_000),
                &Leaf::Measured(false),
                &raw,
                &Leaf::Measured("guarantee".to_string()),
                &guarantee_raw,
                &guarantee,
            ),
            resolve_war_residuals(
                &Leaf::Measured(1),
                &Leaf::Measured(100_000),
                &Leaf::Measured(false),
                &Leaf::Measured(6),
                &Leaf::Measured("modded_reason".to_string()),
                &guarantee_raw,
                &guarantee,
            ),
            resolve_war_residuals(
                &Leaf::Measured(1),
                &Leaf::Measured(100_000),
                &Leaf::Measured(false),
                &raw,
                &asked,
                &Leaf::Unobtained,
                &Leaf::Unobtained,
            ),
        ] {
            assert_eq!(rejected, Leaf::Unobtained);
        }

        assert_eq!(resolve_hostility_reason_token(6), None);
        assert_eq!(resolve_hostility_reason_token(-1), None);
    }
}
