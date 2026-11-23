use super::{Captured, DivisionsCopyContext, DivisionsCopyReport, DivisionsJob};
use collector_production::production::complete_source;
use engine_deployment::{
    army_upgrade_request_row, carmy_upgrades_requests, carmy_upgrades_status, cupgrade_delivery,
};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{carmy, cequipment_variant::pool as cequipment_variant_pool};

pub const BASE: usize = 0;
pub const GAME_STATE: usize = 0x10000;
#[cfg(test)]
pub(crate) const CTAB: usize = 0x11000;
pub(crate) const C0: usize = 0x20000;
#[cfg(test)]
pub(crate) const C1: usize = 0x21000;
#[cfg(test)]
pub(crate) const P1: usize = 0x24000;
pub(crate) const A1: usize = 0x30000;
#[cfg(test)]
pub(crate) const A2: usize = 0x31000;
#[cfg(test)]
pub(crate) const A3: usize = 0x32000;

pub(crate) use collector_divisions::census_test_fixture::*;

pub struct Split {
    pub divisions_outcome: Outcome,
    pub divisions: Vec<carmy::Row>,
    pub equipment_outcome: Outcome,
    pub equipment: Vec<cequipment_variant_pool::DivisionRow>,
    pub groups_outcome: Outcome,
    pub groups: Vec<carmy_upgrades_requests::Row>,
    pub requests_outcome: Outcome,
    pub requests: Vec<army_upgrade_request_row::Row>,
    pub deliveries_outcome: Outcome,
    pub deliveries: Vec<cupgrade_delivery::Row>,
    pub produced_outcome: Outcome,
    pub produced: Vec<cupgrade_delivery::ProducedRow>,
}

pub fn direct(memory: &impl Memory, lens: [usize; 6]) -> Split {
    let mut divisions = vec![carmy::Row::default(); lens[0]];
    let mut visited = Vec::new();
    let divisions_outcome =
        carmy::walk_with(memory, BASE, GAME_STATE, &mut divisions, |pointer, row| {
            visited.push(cequipment_variant_pool::DivisionSource {
                pointer,
                division: row.id,
                template: row.template,
                country: row.roster_owner,
            });
        });
    let first_sources = complete_source(&visited, divisions_outcome);
    let mut equipment = vec![cequipment_variant_pool::DivisionRow::default(); lens[1]];
    let equipment_outcome = match first_sources {
        Ok(source) => cequipment_variant_pool::walk_divisions(memory, BASE, source, &mut equipment),
        Err(reason) => Outcome::Unavailable { reason },
    };
    let mut groups = vec![carmy_upgrades_requests::Row::default(); lens[2]];
    let mut requests = vec![army_upgrade_request_row::Row::default(); lens[3]];
    let mut deliveries = vec![cupgrade_delivery::Row::default(); lens[4]];
    let mut produced = vec![cupgrade_delivery::ProducedRow::default(); lens[5]];
    let result = match first_sources {
        Ok(source) => carmy_upgrades_status::walk(
            memory,
            BASE,
            source,
            &mut groups,
            &mut requests,
            &mut deliveries,
            &mut produced,
        ),
        Err(reason) => carmy_upgrades_status::Outcomes::unavailable(reason),
    };
    Split {
        divisions_outcome,
        divisions,
        equipment_outcome,
        equipment,
        groups_outcome: result.groups,
        groups,
        requests_outcome: result.requests,
        requests,
        deliveries_outcome: result.deliveries,
        deliveries,
        produced_outcome: result.produced,
        produced,
    }
}

/// # Safety
/// Run only in an isolated fixture that owns the collector's process-wide read accounting. No live
/// attachment may consume the fixture's Divisions verdicts.
pub unsafe fn split(
    memory: &impl Memory,
    lens: [usize; 6],
) -> (Split, DivisionsCopyReport, DivisionsJob) {
    let ctx = DivisionsCopyContext {
        base: BASE,
        game_state: GAME_STATE,
        source_capacity: lens[0],
        detail_capacities: lens[1..].try_into().unwrap(),
        details_active: true,
    };
    let mut first = DivisionsJob::new();
    let report =
    // SAFETY: The isolated fixture owns the Divisions read accounting for this copy.
    unsafe { super::copy_divisions_refresh(memory, ctx, &mut first) };
    assert!(report.raw_ok);
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Captured::new() };
    frame.divisions = vec![carmy::Row::default(); lens[0]];
    frame.extended.test_resize_division_buffers(lens);
    let mut ticks = [0u64; 6];
    super::decode_divisions_payload(&mut frame, BASE, &mut first, &mut ticks);
    let detail = frame.extended.test_division_detail();
    assert!(!detail.divisions_raw_mode);
    (
        Split {
            divisions_outcome: frame.divisions_result,
            divisions: std::mem::take(&mut frame.divisions),
            equipment_outcome: detail.division_equipment.outcome,
            equipment: std::mem::take(&mut detail.division_equipment.rows),
            groups_outcome: detail.groups.outcome,
            groups: std::mem::take(&mut detail.groups.rows),
            requests_outcome: detail.requests.outcome,
            requests: std::mem::take(&mut detail.requests.rows),
            deliveries_outcome: detail.deliveries.outcome,
            deliveries: std::mem::take(&mut detail.deliveries.rows),
            produced_outcome: detail.produced.outcome,
            produced: std::mem::take(&mut detail.produced.rows),
        },
        report,
        first,
    )
}

#[cfg(test)]
pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(
        split.divisions_outcome, direct.divisions_outcome,
        "divisions outcome"
    );
    assert_eq!(
        &split.divisions[..split.divisions_outcome.len()],
        &direct.divisions[..direct.divisions_outcome.len()],
        "divisions rows"
    );
    assert_eq!(
        split.equipment_outcome, direct.equipment_outcome,
        "equipment outcome"
    );
    assert_eq!(
        &split.equipment[..split.equipment_outcome.len()],
        &direct.equipment[..direct.equipment_outcome.len()],
        "equipment rows"
    );
    assert_eq!(
        split.groups_outcome, direct.groups_outcome,
        "groups outcome"
    );
    assert_eq!(
        &split.groups[..split.groups_outcome.len()],
        &direct.groups[..direct.groups_outcome.len()],
        "groups rows"
    );
    assert_eq!(
        split.requests_outcome, direct.requests_outcome,
        "requests outcome"
    );
    assert_eq!(
        &split.requests[..split.requests_outcome.len()],
        &direct.requests[..direct.requests_outcome.len()],
        "requests rows"
    );
    assert_eq!(
        split.deliveries_outcome, direct.deliveries_outcome,
        "deliveries outcome"
    );
    assert_eq!(
        &split.deliveries[..split.deliveries_outcome.len()],
        &direct.deliveries[..direct.deliveries_outcome.len()],
        "deliveries rows"
    );
    assert_eq!(
        split.produced_outcome, direct.produced_outcome,
        "produced outcome"
    );
    assert_eq!(
        &split.produced[..split.produced_outcome.len()],
        &direct.produced[..direct.produced_outcome.len()],
        "produced rows"
    );
}
