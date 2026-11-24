use super::super::spending::{ConstructionSpendingReply, LeafProject, SpendingLeaf, SpendingUnknown, SpentStep};
use super::super::{ConstructionType, ProjectKind, SpendingGap};
use super::at_hour;
use dashboard_base::products::breakdown::BreakdownState;
use dashboard_query::series::MissingSpan;

fn leaf(repair: bool, building: u32, spent: &[(i32, Option<i64>)]) -> SpendingLeaf {
    SpendingLeaf {
        repair,
        kind: ProjectKind::Building,
        building: Some(building),
        state: Some(1),
        to_state: None,
        spent: spent.iter().map(|&(known_at, total)| SpentStep { known_at, total_raw: total.map(|value| value.to_string()) }).collect(),
        unknown: Vec::new(),
        projects: vec![LeafProject { project: building.to_string(), first_read: spent.first().map_or(0, |step| step.0), until: None }],
    }
}

fn reply(leaves: Vec<SpendingLeaf>) -> ConstructionSpendingReply {
    ConstructionSpendingReply {
        branch_id: "1".into(),
        tag: "GER".into(),
        cost_scale: 100_000,
        recorded: true,
        types: vec![
            ConstructionType { id: 7, token: Some("arms_factory".into()), name: Some("Military factory".into()), icon: None },
            ConstructionType { id: 8, token: Some("industrial_complex".into()), name: None, icon: None },
        ],
        repair_icon: None,
        states: Vec::new(),
        before_first_read: 0,
        leaves,
        unattributed: Vec::new(),
        missing: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn spending_at_an_hour_is_the_running_total_then() {
    let at = at_hour(&reply(vec![leaf(false, 7, &[(10, Some(500_000)), (30, Some(900_000))]), leaf(false, 8, &[(20, Some(200_000))]), leaf(true, 7, &[(15, Some(100_000))])]), 0, 25);
    let parts = &at.breakdown.parts;
    assert_eq!(at.breakdown.decimals, 5);
    assert_eq!(at.breakdown.total.raw.as_deref(), Some("800000"), "the step read at hour 30 is not counted at hour 25");
    assert_eq!(parts.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["New buildings", "Repairs"]);
    assert_eq!(parts[0].children.iter().map(|part| part.name.as_str()).collect::<Vec<_>>(), ["Military factory", "industrial_complex"]);
    assert_eq!(at.through, 29);
}

#[test]
fn unrecorded_output_and_hours_are_named_never_counted_as_nothing() {
    let mut gapped = leaf(false, 7, &[(10, Some(500_000)), (20, None)]);
    gapped.unknown.push(SpendingUnknown { known_at: 12, reason: SpendingGap::InexactOutput, project: "p".into(), until: None });
    let mut held = reply(vec![gapped]);
    held.missing.push(MissingSpan { first_hour: 2, last_hour: 4, reason: "gap".into() });
    let partial = at_hour(&held, 0, 15);
    assert_eq!(partial.breakdown.parts[0].state, BreakdownState::Partial);
    assert_eq!(partial.breakdown.unrecorded_hours, 3);
    assert_eq!(partial.breakdown.total.state, BreakdownState::Partial);
    let missing = at_hour(&held, 0, 25);
    assert_eq!(missing.breakdown.total.state, BreakdownState::NotRecorded, "a total not recorded is not zero");
    assert_eq!(missing.breakdown.total.raw, None);
}
