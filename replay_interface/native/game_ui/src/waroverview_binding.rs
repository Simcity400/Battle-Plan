//! War-overview / war-summary panel population (U1 wave —
//! `interface/waroverview.gui` `waroverview_window`: the global list of ongoing
//! wars, their belligerents, and per-participant summary stats).
//!
//! Population seam over `replay_core`'s read-only [`DiplomacyView`] (items
//! 8/8b). It parses no recording bytes (Constraint 2) and fabricates nothing
//! (Constraint 3): an unmeasured casualty tally is `None`, never `0`.
//!
//! ## What is POPULATED
//!
//! * **Belligerent-pair casualties** — from `DiplomacyView::war_relations`
//!   (the live `diplomacy_live.war_relations` feed), each row is an ordered
//!   `(first, second)` belligerent pair plus the two running casualty tallies.
//!   These feed the war-participant rows' `casualties` textbox.
//! * **Exact war objects** — ordered attacker/defender sides, engine-resolved
//!   war name, participant participation/breakdown/capitulation rows, and the
//!   signed overall-progress raw value from `DiplomacyView::wars`.
//! * **Belligerent set** — the union of every country in an `at_war==true`
//!   pair (`DiplomacyView::pairs`) or any `war_relations` row, giving the
//!   participant column contents.
//! * **POV belligerents** — the countries the POV country is at war with, for
//!   the POV-highlight (mirrors the diplomacy panel's `wars`).
//!
//! ## KNOWN GAPS (fail-closed; filed in `LANE_REPORT.md` / docs)
//!
//! * **W1C fields** — participant division count, participant IC, ordered live
//!   wargoals, and terminal `war_result` are distinct scheduled-reader inputs.
//!   They remain absent; pair-scoped D60 values are never substituted.

use replay_core::{
    DiplomacyView, Leaf, ViewStatus, WarObjectRow, WarParticipantOverviewRow, WorldFrame,
};
use std::collections::BTreeSet;

/// One live belligerent-pair casualty row from `DiplomacyView::war_relations`.
/// The pair identity requires both tags measured; each casualty tally is
/// `Some` only when measured (fail-closed, never a fabricated `0`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WarRelationSummary {
    pub first: String,
    pub second: String,
    pub first_casualties: Option<i64>,
    pub second_casualties: Option<i64>,
}

/// One exact participant row. Raw fixed-point values remain raw; rendering
/// converts them only after this admission validates their a729 bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct WarParticipantSummary {
    pub tag: Leaf<String>,
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

/// One exact war row in producer order. `source_key` is identity only and is
/// never substituted for a missing engine-resolved display name.
#[derive(Clone, Debug, PartialEq)]
pub struct WarObjectSummary {
    pub source_key: String,
    pub war_name: Leaf<String>,
    pub attackers: Leaf<Vec<String>>,
    pub defenders: Leaf<Vec<String>>,
    pub attacker_participants: Leaf<Vec<WarParticipantSummary>>,
    pub defender_participants: Leaf<Vec<WarParticipantSummary>>,
    pub overall_progress_raw: Leaf<i64>,
    pub front: Leaf<String>,
    pub start_hour: Leaf<i64>,
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
        war_name: row.war_name.clone(),
        attackers: row.attackers.clone(),
        defenders: row.defenders.clone(),
        attacker_participants: participant_summaries(&row.attacker_participants),
        defender_participants: participant_summaries(&row.defender_participants),
        overall_progress_raw: row.overall_progress_raw.clone(),
        front: row.front.clone(),
        start_hour: row.start_hour.clone(),
    }
}

/// Read-only war-overview panel binding for one POV at one world frame (U1).
#[derive(Clone, Debug, PartialEq)]
pub struct WarOverviewBinding {
    pub pov_tag: String,
    /// `true` only when the underlying [`DiplomacyView`] status is `Present`.
    pub available: bool,
    /// Live belligerent-pair casualty rows (verbatim from `war_relations`),
    /// filtered to rows whose pair identity (`first`,`second`) is measured.
    pub war_relations: Vec<WarRelationSummary>,
    /// Exact source order and leaf authority are preserved.
    pub war_objects: Leaf<Vec<WarObjectSummary>>,
    /// Every country in any `at_war==true` pair OR any `war_relations` row,
    /// sorted ascending and de-duplicated — the participant column.
    pub belligerents: Vec<String>,
    /// The POV's own belligerents (other tags with `at_war == Some(true)` from
    /// `pov -> other`), sorted ascending — for the POV highlight.
    pub pov_belligerents: Vec<String>,
}

impl Default for WarOverviewBinding {
    fn default() -> Self {
        Self {
            pov_tag: String::new(),
            available: false,
            war_relations: Vec::new(),
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

/// Belligerent-pair casualty rows from the view, dropping any row whose pair
/// identity is not measured (a row with no tags cannot be attributed).
fn war_relation_summaries(view: &DiplomacyView) -> Vec<WarRelationSummary> {
    let rows = match view.war_relations.measured() {
        Some(rows) => rows,
        None => return Vec::new(),
    };
    rows.iter()
        .filter_map(|r| {
            let first = r.first.measured()?.clone();
            let second = r.second.measured()?.clone();
            Some(WarRelationSummary {
                first,
                second,
                first_casualties: r.first_casualties.measured().copied(),
                second_casualties: r.second_casualties.measured().copied(),
            })
        })
        .collect()
}

/// Union of all tags in the casualty rows plus all tags in any measured
/// `at_war==true` pair, sorted + de-duplicated.
fn belligerents(
    view: &DiplomacyView,
    summaries: &[WarRelationSummary],
    wars: &Leaf<Vec<WarObjectSummary>>,
) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for s in summaries {
        set.insert(s.first.clone());
        set.insert(s.second.clone());
    }
    if let Leaf::Measured(rows) = wars {
        for row in rows {
            if let Leaf::Measured(tags) = &row.attackers {
                set.extend(tags.iter().cloned());
            }
            if let Leaf::Measured(tags) = &row.defenders {
                set.extend(tags.iter().cloned());
            }
        }
    }
    for ((a, b), pair) in &view.pairs {
        if matches!(pair.at_war, Leaf::Measured(true)) {
            set.insert(a.clone());
            set.insert(b.clone());
        }
    }
    set.into_iter().collect()
}

/// The POV's own belligerents: other tags where `pov -> other` has
/// `at_war == Some(true)`.
fn pov_belligerents(view: &DiplomacyView, pov_tag: &str) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for ((a, b), pair) in &view.pairs {
        if a == pov_tag && matches!(pair.at_war, Leaf::Measured(true)) {
            set.insert(b.clone());
        }
    }
    set.into_iter().collect()
}

/// Build the read-only war-overview panel for `pov_tag` at one world frame
/// (U1). When the diplomacy view is not `Present`, returns the fail-closed
/// empty binding (`available: false`, no rows).
pub fn war_overview_from_frame(frame: &WorldFrame, pov_tag: &str) -> WarOverviewBinding {
    let view = frame.diplomacy();
    if view.status != ViewStatus::Present {
        return WarOverviewBinding {
            pov_tag: pov_tag.to_string(),
            available: false,
            ..WarOverviewBinding::default()
        };
    }
    let war_relations = war_relation_summaries(view);
    let war_objects = war_object_summaries(view);
    let belligerents = belligerents(view, &war_relations, &war_objects);
    let pov_belligerents = pov_belligerents(view, pov_tag);
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
        let mut map = BTreeMap::new();
        for ((a, b), p) in pairs {
            map.insert((a.to_string(), b.to_string()), p);
        }
        DiplomacyView {
            status,
            pairs: map,
            countries: BTreeMap::new(),
            war_relations,
            wars: Leaf::Absent,
            faction_members: Leaf::Absent,
            operation_instances: Leaf::Absent,
            intel_levels: Leaf::Absent,
            intel_network_graphs: Leaf::Absent,
            subject_autonomy_current: Leaf::Absent,
            relation_flags: BTreeMap::new(),
        }
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
                first: Leaf::Measured("GER".to_string()),
                second: Leaf::Measured("ENG".to_string()),
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
            vec![WarRelationSummary {
                first: "GER".to_string(),
                second: "ENG".to_string(),
                first_casualties: Some(1200),
                second_casualties: Some(900),
            }]
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
        assert!(war_relation_summaries(&view).is_empty());
        assert!(belligerents(&view, &[], &Leaf::Absent).is_empty());
    }

    #[test]
    fn unmeasured_casualties_are_none_not_zero() {
        let view = view_with(
            ViewStatus::Present,
            vec![],
            Leaf::Measured(vec![WarRelationRow {
                first: Leaf::Measured("ITA".to_string()),
                second: Leaf::Measured("ENG".to_string()),
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
        let s = war_relation_summaries(&view);
        assert_eq!(s[0].first_casualties, None);
        assert_eq!(s[0].second_casualties, None);
    }
}
