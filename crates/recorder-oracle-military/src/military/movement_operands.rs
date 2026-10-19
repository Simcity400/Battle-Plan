//! The land movement speed's runtime operand tables. Each holds runtime
//! accumulator entries, predicates or getter results the save omits, so the
//! oracle checks the rows' own contract (key order, the recorded branch) and
//! names the values a paused-memory comparison must certify.
use std::io;

use replay_data::rows::StateMovementTerms;
use replay_data::rows::TableId;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

use super::{Report, invalid};

pub fn compare(frame: &DecodedFrame, report: &mut Report) {
    report.attempt_table(TableId::MovementStateTerms, |r| {
        r.compared("MovementStateTerms", state_terms(frame)?);
        r.limitation(
            "Movement state terms are the location state's country modifier block entries (army speed, attack speed, controller and enemy army speed) and the owner's movement relation to the state's controller, runtime accumulator values and predicates the save omits; compare them against paused memory at the sampled hour",
            "MovementStateTerms runtime entries",
        );
        Ok(())
    });
    report.attempt_table(TableId::CoreSupplyAccess, |r| {
        let rows = frame.complete::<tables::CoreSupplyAccess>()?;
        in_key_order(
            "CoreSupplyAccess",
            rows.iter().map(|row| (row.owner, row.province)),
        )?;
        if rows.iter().any(|row| row.access.is_none()) {
            return Err(invalid("CoreSupplyAccess contains a guarded read refusal"));
        }
        r.compared("CoreSupplyAccess", rows.len());
        r.limitation(
            "Core supply access is the runtime predicate over the owner's supply access in the location's controller area, which the save omits; compare it against paused memory at the sampled hour",
            "CoreSupplyAccess runtime access",
        );
        Ok(())
    });
    report.attempt_table(TableId::CountryUnitSpeedEntries, |r| {
        let rows = frame.complete::<tables::CountryUnitSpeedEntries>()?;
        in_key_order(
            "CountryUnitSpeedEntries",
            rows.iter().map(|row| (row.country, row.key)),
        )?;
        if rows.iter().any(|row| row.entry.is_none()) {
            return Err(invalid("CountryUnitSpeedEntries contains a guarded read refusal"));
        }
        r.compared("CountryUnitSpeedEntries", rows.len());
        r.limitation(
            "Country unit speed entries are runtime country modifier accumulator entries for the movement priority sub-unit's and its categories' keys, which the save omits; compare them against paused memory at the sampled hour",
            "CountryUnitSpeedEntries runtime entries",
        );
        Ok(())
    });
    report.attempt_table(TableId::RiverRelations, |r| {
        let rows = frame.complete::<tables::RiverRelations>()?;
        in_key_order(
            "RiverRelations",
            rows.iter().map(|row| (row.owner, row.controller)),
        )?;
        if rows.iter().any(|row| row.entry.is_none()) {
            return Err(invalid("RiverRelations contains a guarded read refusal"));
        }
        r.compared("RiverRelations", rows.len());
        r.limitation(
            "River relations are the runtime river_crossing_factor_against entries of the owner's diplomatic relations, whose modifier accumulators the save omits; compare them against paused memory at the sampled hour",
            "RiverRelations runtime entries",
        );
        Ok(())
    });
    report.attempt_table(TableId::DivisionTransportSpeeds, |r| {
        let rows = frame.complete::<tables::DivisionTransportSpeeds>()?;
        in_key_order(
            "DivisionTransportSpeeds",
            rows.iter().map(|row| (row.owner, row.region)),
        )?;
        if rows.iter().any(|row| row.speed.is_none()) {
            return Err(invalid("DivisionTransportSpeeds contains a guarded read refusal"));
        }
        r.compared("DivisionTransportSpeeds", rows.len());
        r.limitation(
            "Division transport speeds are runtime getter results (regional weather and every enemy navy's regional operands) the save omits; compare them against paused memory at the sampled hour",
            "DivisionTransportSpeeds runtime speeds",
        );
        Ok(())
    });
}

/// The state term rows are in key order and each read's branch names one of
/// the three controller relations.
fn state_terms(frame: &DecodedFrame) -> io::Result<usize> {
    let rows = frame.complete::<tables::MovementStateTerms>()?;
    in_key_order(
        "MovementStateTerms",
        rows.iter().map(|row| (row.state, row.roster, row.owner)),
    )?;
    if rows.iter().any(|row| {
        row.terms.is_some_and(|terms| {
            terms.branch & StateMovementTerms::RELATION_MASK > StateMovementTerms::ENEMY
                || terms.branch
                    & !(StateMovementTerms::RELATION_MASK | StateMovementTerms::ATTACK_REFUSED)
                    != 0
        })
    }) {
        return Err(invalid(
            "MovementStateTerms holds a controller branch outside neither, friendly and enemy",
        ));
    }
    if rows.iter().any(|row| {
        row.terms
            .is_none_or(|terms| terms.branch & StateMovementTerms::ATTACK_REFUSED != 0)
    }) {
        return Err(invalid(
            "MovementStateTerms contains a guarded read refusal",
        ));
    }
    Ok(rows.len())
}

fn in_key_order<K: Ord>(table: &str, mut keys: impl Iterator<Item = K>) -> io::Result<()> {
    let Some(mut previous) = keys.next() else {
        return Ok(());
    };
    for key in keys {
        if previous >= key {
            return Err(invalid(format!("{table} rows are not in key order")));
        }
        previous = key;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::CoreSupplyAccessRow;
    use replay_data::rows::CountryUnitSpeedEntryRow;
    use replay_data::rows::MovementStateTermRow;
    use replay_data::rows::OwnerTransportSpeedRow;
    use replay_data::rows::RiverRelationRow;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;

    fn put(frame: &mut DecodedFrame, rows: TableRows) {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
    }

    fn failure(frame: &DecodedFrame, table: TableId) -> Option<String> {
        let mut report = Report::default();
        compare(frame, &mut report);
        report
            .failed_comparisons
            .get(&format!("{table:?}"))
            .cloned()
    }

    /// Every table refuses rows out of key order and names its values as runtime
    /// operands the save omits.
    #[test]
    fn movement_operand_tables_check_key_order_and_name_their_runtime_values() {
        let (mut frame, ..) = crate::fixtures::fixture();
        let terms = |keys: &[(u32, u32, u32)]| {
            TableRows::MovementStateTerms(
                keys.iter()
                    .map(|&(state, roster, owner)| MovementStateTermRow {
                        state,
                        roster,
                        owner,
                        terms: Some(StateMovementTerms {
                            branch: StateMovementTerms::FRIENDLY,
                            ..Default::default()
                        }),
                    })
                    .collect(),
            )
        };
        let access = |keys: &[(u32, u32)]| {
            TableRows::CoreSupplyAccess(
                keys.iter()
                    .map(|&(owner, province)| CoreSupplyAccessRow {
                        owner,
                        province,
                        access: Some(false),
                    })
                    .collect(),
            )
        };
        let entries = |keys: &[(u32, u32)]| {
            TableRows::CountryUnitSpeedEntries(
                keys.iter()
                    .map(|&(country, key)| CountryUnitSpeedEntryRow {
                        country,
                        key,
                        entry: Some(None),
                    })
                    .collect(),
            )
        };
        let rivers = |keys: &[(u32, u32)]| {
            TableRows::RiverRelations(
                keys.iter()
                    .map(|&(owner, controller)| RiverRelationRow {
                        owner,
                        controller,
                        entry: Some(Some(-10_000)),
                    })
                    .collect(),
            )
        };
        let transport = |keys: &[(u32, u32)]| {
            TableRows::DivisionTransportSpeeds(
                keys.iter()
                    .map(|&(owner, region)| OwnerTransportSpeedRow {
                        owner,
                        region,
                        speed: Some(0),
                    })
                    .collect(),
            )
        };
        put(&mut frame, terms(&[(1, 2, 2), (1, 3, 2), (2, 2, 2)]));
        put(&mut frame, access(&[(101, 4), (102, 1)]));
        put(&mut frame, entries(&[(101, 7), (101, 9)]));
        put(&mut frame, rivers(&[(101, 101), (101, 102)]));
        put(&mut frame, transport(&[(101, 1), (102, 1)]));
        let mut report = Report::default();
        compare(&frame, &mut report);
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        for (table, rows) in [
            ("MovementStateTerms", 3),
            ("CoreSupplyAccess", 2),
            ("CountryUnitSpeedEntries", 2),
            ("RiverRelations", 2),
            ("DivisionTransportSpeeds", 2),
        ] {
            assert_eq!(report.compared_tables.get(table), Some(&rows), "{table}");
            assert!(
                report.limitations.iter().any(|(reason, subjects)| {
                    reason.contains("the save omits")
                        && subjects.iter().any(|subject| subject.starts_with(table))
                }),
                "{table}"
            );
        }
        for (rows, table) in [
            (terms(&[(1, 3, 2), (1, 2, 2)]), TableId::MovementStateTerms),
            (terms(&[(1, 2, 2), (1, 2, 2)]), TableId::MovementStateTerms),
            (access(&[(102, 1), (101, 4)]), TableId::CoreSupplyAccess),
            (
                entries(&[(101, 9), (101, 7)]),
                TableId::CountryUnitSpeedEntries,
            ),
            (rivers(&[(101, 102), (101, 101)]), TableId::RiverRelations),
            (
                transport(&[(102, 1), (101, 1)]),
                TableId::DivisionTransportSpeeds,
            ),
        ] {
            let mut frame = frame.clone();
            put(&mut frame, rows);
            let reason = failure(&frame, table).unwrap_or_default();
            assert!(reason.contains("key order"), "{table:?}: {reason}");
        }
        // A branch outside the three controller relations is malformed.
        let mut bad = frame.clone();
        put(
            &mut bad,
            TableRows::MovementStateTerms(vec![MovementStateTermRow {
                state: 1,
                roster: 2,
                owner: 2,
                terms: Some(StateMovementTerms {
                    branch: 3,
                    ..Default::default()
                }),
            }]),
        );
        let reason = failure(&bad, TableId::MovementStateTerms).unwrap_or_default();
        assert!(reason.contains("controller branch"), "{reason}");
    }
    #[test]
    fn movement_operand_read_refusals_fail_instead_of_certifying_an_unavailable_value() {
        let (mut frame, ..) = crate::fixtures::fixture();
        for rows in [
            TableRows::MovementStateTerms(vec![MovementStateTermRow::default()]),
            TableRows::CoreSupplyAccess(vec![CoreSupplyAccessRow::default()]),
            TableRows::CountryUnitSpeedEntries(vec![CountryUnitSpeedEntryRow::default()]),
            TableRows::RiverRelations(vec![RiverRelationRow::default()]),
            TableRows::DivisionTransportSpeeds(vec![OwnerTransportSpeedRow::default()]),
        ] {
            let table = rows.table();
            put(&mut frame, rows);
            assert!(
                failure(&frame, table)
                    .unwrap()
                    .contains("guarded read refusal")
            );
        }
    }
}

#[cfg(test)]
mod attack_availability_test {
    use super::*;
    #[test]
    fn a_refused_attack_entry_is_a_local_read_refusal_not_an_unknown_controller_branch() {
        let (mut frame, ..) = crate::fixtures::fixture();
        let rows = replay_data::rows::TableRows::MovementStateTerms(vec![
            replay_data::rows::MovementStateTermRow {
                state: 1,
                roster: 1,
                owner: 1,
                terms: Some(StateMovementTerms {
                    branch: StateMovementTerms::FRIENDLY | StateMovementTerms::ATTACK_REFUSED,
                    ..Default::default()
                }),
            },
        ]);
        *frame.tables[TableId::MovementStateTerms.index()] =
            replay_frame::frame::TableView::Complete(rows);
        assert!(
            state_terms(&frame)
                .unwrap_err()
                .to_string()
                .contains("guarded read refusal")
        );
    }
}
