//! Domain 41 names the pinned rows it could not materialise, or names nothing.
//!
//! `captures/e2e_05` lost 86 pinned facility rows across 65 degraded hours
//! (1.3 of ~27 per hour) and each loss cost the whole d41 hour. The declaration
//! is what turns those hours into tolerated partials, so it must be exact: it
//! exists only when the two materialise walks are the entire reason the record
//! carries `PARTIAL_DATA`, and every omitted row is named by identity.

use super::super::*;
use super::domain41_phase_split::read_with_phase_and_shortfall;
use compact_contract::d41_omission::{
    decode_d41_omission_trailer, Domain41OmissionClass, D41_OMISSION_TRAILER_MAGIC,
};

fn loss(identities: &[(u32, u32)]) -> Domain41MaterialiseLoss {
    let mut loss = Domain41MaterialiseLoss::default();
    for (index, (id, aux)) in identities.iter().enumerate() {
        loss.note_lost(index, *id, *aux);
    }
    loss
}

fn read_losing(
    facilities: &[(u32, u32)],
    scientists: &[(u32, u32)],
) -> CompactProjectsRead {
    let mut read = read_with_phase_and_shortfall(
        compact_contract::COMPACT_STATUS_PARTIAL_DATA,
        facilities.len() as u32,
        scientists.len() as u32,
    );
    read.observed_facility_rows = facilities.len() as u32;
    read.observed_scientist_rows = scientists.len() as u32;
    read.facility_loss = loss(facilities);
    read.scientist_loss = loss(scientists);
    read
}

fn declaration_on_the_wire(read: &CompactProjectsRead) -> Option<Vec<(&'static str, u32, u32)>> {
    let payload = compact_projects_data_payload(read, 1234);
    let start = payload
        .windows(4)
        .position(|window| window == D41_OMISSION_TRAILER_MAGIC)?;
    let (declaration, consumed) =
        decode_d41_omission_trailer(&payload, start).expect("the trailer this producer wrote");
    assert_eq!(
        start + consumed,
        payload.len(),
        "the omission trailer is the last of the base payload's trailers"
    );
    let mut rows = Vec::new();
    for class in Domain41OmissionClass::ALL {
        for (left, right) in declaration.identities(class) {
            rows.push((class.unit_name(), *left, *right));
        }
    }
    Some(rows)
}

/// Law: every lost pin is named, and the wire carries the canonical order.
///
/// What production change makes this fail: a walk that counts a loss without
/// recording its identity, or an encoder that ships the pin order unsorted.
#[test]
fn every_lost_pin_reaches_the_wire_by_identity() {
    let read = read_losing(&[(91, 4004), (17, 1201)], &[(5309, 0)]);
    let declaration = domain41_declared_omissions(&read).expect("a declarable loss");
    assert_eq!(declaration.count(Domain41OmissionClass::Facility), 2);
    assert_eq!(declaration.count(Domain41OmissionClass::Scientist), 1);
    assert_eq!(
        declaration.identities(Domain41OmissionClass::Facility),
        &[(17, 1201), (91, 4004)]
    );
    let mut on_wire_read = read;
    on_wire_read.omissions = Some(declaration);
    assert_eq!(
        declaration_on_the_wire(&on_wire_read),
        Some(vec![
            ("d41_declared_omission_facility", 17, 1201),
            ("d41_declared_omission_facility", 91, 4004),
            ("d41_declared_omission_scientist", 5309, 0),
        ])
    );
}

/// Law: the scientist half is keyed on the character id alone; the pin's
/// unused second half must never reach the wire as a key.
#[test]
fn a_scientist_is_declared_by_character_id_with_no_second_key() {
    let mut read = read_losing(&[], &[(77, 0)]);
    // The scientist walk pins a zero aux; prove the declaration does not
    // acquire one even if the loss carried it.
    read.scientist_loss.lost_identities[0].1 = 9;
    let declaration = domain41_declared_omissions(&read).expect("a declarable loss");
    assert_eq!(
        declaration.identities(Domain41OmissionClass::Scientist),
        &[(77, 0)]
    );
}

/// Law: a clean frame writes no trailer at all, so a decoder never sees an
/// empty declaration.
#[test]
fn a_clean_frame_declares_nothing_and_writes_no_trailer() {
    let read = read_with_phase_and_shortfall(0, 0, 0);
    assert_eq!(domain41_declared_omissions(&read), None);
    assert_eq!(declaration_on_the_wire(&read), None);
}

/// Law: a loss the producer cannot key is not declarable, and the frame keeps
/// the undeclared-partial verdict that costs the whole hour.
///
/// What production change makes this fail: a declaration built from the counts
/// instead of from the identities the walks recorded.
#[test]
fn an_unkeyable_loss_declares_nothing() {
    // The walk stopped on its row cap: the pins past the cap were never read.
    let mut capped = read_losing(&[(4, 8)], &[]);
    capped.facility_loss.capped = true;
    assert_eq!(domain41_declared_omissions(&capped), None);

    // More rows short than the walk named.
    let mut short = read_losing(&[(4, 8)], &[]);
    short.facility_row_shortfall = 3;
    assert_eq!(domain41_declared_omissions(&short), None);

    // Fewer identities than the walk counted.
    let mut truncated = read_losing(&[(4, 8), (5, 9)], &[]);
    truncated.facility_loss.lost_identities.pop();
    assert_eq!(domain41_declared_omissions(&truncated), None);

    // A refusal that has no per-row key at all shares the record.
    let mut other_cause = read_losing(&[(4, 8)], &[]);
    other_cause.invalid_breakthrough_countries = 2;
    assert_eq!(domain41_declared_omissions(&other_cause), None);

    let mut over_capacity = read_losing(&[(4, 8)], &[]);
    over_capacity.cache_too_many = true;
    assert_eq!(domain41_declared_omissions(&over_capacity), None);

    // A failing word other than PARTIAL_DATA is not what the tolerance covers.
    let mut unavailable = read_losing(&[(4, 8)], &[]);
    unavailable.status_flags |= compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
    assert_eq!(domain41_declared_omissions(&unavailable), None);

    // A pin whose id read as zero names no row the fold could drop.
    let zero_id = read_losing(&[(0, 8)], &[]);
    assert_eq!(domain41_declared_omissions(&zero_id), None);
}

/// Law: the record's own PARTIAL_DATA bit and its declaration come from the
/// same measurement, so a shortfall the walks named always ships with the word.
#[test]
fn the_declared_shortfall_is_the_one_that_raised_the_status_bit() {
    let mut read = read_losing(&[(12, 300), (12, 301)], &[]);
    read.status_flags = 0;
    apply_project_shortfall_status(&mut read);
    assert_eq!(
        read.status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK,
        compact_contract::COMPACT_STATUS_PARTIAL_DATA
    );
    let declaration = domain41_declared_omissions(&read).expect("a declarable loss");
    // Two pins that read the same facility id are two refusals; collapsing
    // them would understate the omission.
    assert_eq!(declaration.total(), 2);
    assert_eq!(
        declaration.identities(Domain41OmissionClass::Facility),
        &[(12, 300), (12, 301)]
    );
}
