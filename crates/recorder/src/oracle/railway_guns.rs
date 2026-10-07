//! The hourly railway-gun counter inputs against the same-hour tables and the save.
//!
//! Province membership of every gun is checked with the other mixed occupants
//! in [`super::counters`]; this compares each gun's own operands.

use super::*;

pub(super) fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::RailwayGunCounterInputs, |r| {
        r.compared("RailwayGunCounterInputs", validate(frame)?);
        compare_saved(frame, save, r)?;
        r.limitation(
            "The save omits naval transfer, exile, retreat, the CUnit combat vector (its count, the first combat's side and province, and the attacking byte), the assignee pointer, the stat definition and its maximum strength, the received-supply ratio and the railway building level at the gun; its combat list is the gun's own reference array, compared by its stored count (+0x42C); compare the rest against paused memory at the sampled hour",
            "RailwayGunCounterInputs live-only operands",
        );
        Ok(())
    });
}

/// Joins every gun's references to the same hour's complete censuses.
fn validate(frame: &DecodedFrame) -> io::Result<usize> {
    let guns = frame.complete::<tables::RailwayGunCounterInputs>()?;
    let countries = country_context(frame)?;
    let provinces: BTreeSet<_> = frame
        .complete::<tables::Provinces>()?
        .iter()
        .map(|row| row.id)
        .collect();
    let groups: BTreeSet<_> = frame
        .complete::<tables::OrdersGroups>()?
        .iter()
        .map(|row| row.handle)
        .collect();
    let tokens: BTreeSet<_> = frame
        .complete::<tables::LexicalTokens>()?
        .iter()
        .map(|row| row.token)
        .collect();
    let mut located = BTreeMap::new();
    for row in frame.complete::<tables::ProvinceUnitOrder>()? {
        for &handle in &row.mixed {
            located.insert(handle, row.province);
        }
    }
    for row in guns {
        let label = format!("RailwayGunCounterInputs gun {}", row.gun);
        let fail = |what: String| Err(invalid(format!("{label} {what}")));
        if row.roster_owner == 0 || !countries.links.contains_key(&row.roster_owner) {
            return fail("has no live roster owner".into());
        }
        for (name, owner) in [
            ("expeditionary owner", row.expeditionary_owner),
            ("logical owner", row.logical_owner),
        ] {
            if (owner as i32) > 0 && !countries.links.contains_key(&owner) {
                return fail(format!("has unknown {name} {owner}"));
            }
        }
        if row.province != 0 && !provinces.contains(&row.province) {
            return fail(format!("has unknown province {}", row.province));
        }
        if located.get(&row.gun).copied().unwrap_or(0) != row.province {
            return fail(format!(
                "location {} disagrees with the mixed province roster",
                row.province
            ));
        }
        if row.province == 0 && row.railway_building_level.is_some() {
            return fail("has a railway building level without a province".into());
        }
        if row.assignee_group != 0 && !groups.contains(&row.assignee_group) {
            return fail(format!("has unknown assignee group {}", row.assignee_group));
        }
        if row.subunit_token == 0 || !tokens.contains(&row.subunit_token) {
            return fail(format!("has unknown subunit token {}", row.subunit_token));
        }
        if row.target != 0 && !provinces.contains(&row.target) {
            return fail(format!("has unknown target {}", row.target));
        }
        if row.combat_role > 2
            || (row.combat_role == 0) != (row.combat_province == 0)
            || (row.combat_role == 0) != (row.combat_count == 0)
        {
            return fail(format!(
                "combat role {} disagrees with combat province {} or count {}",
                row.combat_role, row.combat_province, row.combat_count
            ));
        }
        // The first combat's attacker is the walk's first hit; with one combat the two agree.
        if row.attacking > 1
            || (row.combat_role == 1 && row.attacking == 0)
            || (row.combat_count == 0 && row.attacking != 0)
            || (row.combat_count == 1 && row.attacking != u8::from(row.combat_role == 1))
        {
            return fail(format!(
                "attacking byte {} disagrees with combat role {} and count {}",
                row.attacking, row.combat_role, row.combat_count
            ));
        }
        if row.combat_province != 0 && !provinces.contains(&row.combat_province) {
            return fail(format!(
                "has unknown combat province {}",
                row.combat_province
            ));
        }
        if row.naval_transfer == 0 && row.transfer_action_status != 0 {
            return fail("has a transfer action without a naval transfer".into());
        }
        for (name, value) in [
            ("naval transfer", row.naval_transfer),
            ("transfer action", row.transfer_action_status),
            ("exile", row.exiled),
            ("retreat", row.retreat),
            ("strategic redeployment", row.strategic_redeployment),
        ] {
            if value > 1 {
                return fail(format!("has non-boolean {name} byte {value}"));
            }
        }
    }
    Ok(guns.len())
}

fn check<T: PartialEq + std::fmt::Debug>(report: &mut Report, label: &str, actual: T, saved: T) {
    report.scalar_checks += 1;
    if actual != saved {
        report.difference(label, actual, saved);
    }
}

fn compare_saved(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let countries = country_context(frame)?;
    let mut recorded: BTreeMap<_, _> = frame
        .complete::<tables::RailwayGunCounterInputs>()?
        .iter()
        .map(|row| (row.gun, row))
        .collect();
    for (&id, saved) in &save.railway_guns {
        let label = format!("railway gun type={} id={}", id as u32, id >> 32);
        let Some(row) = recorded.remove(&id) else {
            report.difference(&label, "absent", "present");
            continue;
        };
        match &row.name {
            replay::rows::Recorded::NotRecorded => report.limitation(
                "stored railway gun name not recorded by this format",
                format!("{label}.name"),
            ),
            replay::rows::Recorded::Value(None) => report.difference(
                format!("{label}.name: guarded read or name arena refusal"),
                "unavailable",
                &saved.name,
            ),
            replay::rows::Recorded::Value(Some(name)) => {
                // BF7010 omits empty names. 1424AF5A0 emits a C string;
                // post-NUL captured suffixes remain differences, never trimmed.
                check(
                    report,
                    &format!("{label}.name"),
                    (!name.is_empty()).then_some(name.as_slice()),
                    saved.name.as_deref(),
                );
            }
        }
        for (field, raw, expected) in [
            (
                "roster_owner",
                row.roster_owner,
                Some(saved.roster_owner.as_slice()),
            ),
            (
                "logical_owner",
                row.logical_owner,
                Some(saved.logical_owner.as_slice()),
            ),
            (
                "expeditionary_owner",
                row.expeditionary_owner,
                saved.expeditionary_owner.as_deref(),
            ),
        ] {
            // The CUnit writer drops signed-nonpositive expeditionary tags; only
            // an exact raw zero is proved by omission.
            if field == "expeditionary_owner" && (raw as i32) < 0 {
                report.limitation(
                    "the save omits signed-nonpositive expeditionary country references; the raw bits need paused-memory verification",
                    format!("{label}.{field}"),
                );
                continue;
            }
            let actual = country_tag(raw, &countries.links)?;
            check(
                report,
                &format!("{label}.{field}"),
                tag_text(actual),
                tag_text(expected),
            );
        }
        check(
            report,
            &format!("{label}.province"),
            row.province,
            saved.province.unwrap_or(0),
        );
        check(
            report,
            &format!("{label}.ordinary_path"),
            row.ordinary_path.as_slice(),
            saved.path.as_slice(),
        );
        // CUnit::Serialize@140BF7010 writes primary+0x1E8 as support_attack
        // (0x2EDA) iff nonnull, and primary+0x220 as full_path (0x362C) iff nonempty.
        check(
            report,
            &format!("{label}.target"),
            row.target,
            saved.support_attack.unwrap_or(0),
        );
        check(
            report,
            &format!("{label}.alternate_path"),
            row.alternate_path.as_slice(),
            saved.full_path.as_slice(),
        );
        check(
            report,
            &format!("{label}.strategic_redeployment"),
            row.strategic_redeployment != 0,
            saved.strategic_redeployment,
        );
        report.scalar(
            &label,
            "current_strength",
            Some(row.current_strength),
            Some(&saved.strength),
        );
        // A gun's saved `combat` list (token 0x2916) is its own CRailwayGun+0x420
        // reference array: CRailwayGun::Serialize@140E78CF0 writes it while the
        // count at +0x42C is nonzero, and CRailwayGun::HasReferenceEntries@140E76080
        // tests that same count. CUnit::Serialize@140BF7010 never writes the CUnit
        // combat vector (+0x1A8) that `combat_count` holds, so that has no saved
        // counterpart for a gun.
        // The writer emits exactly +0x42C entries, so the stored count is the
        // saved list's length.
        check(
            report,
            &format!("{label}.reference_entry_count"),
            i64::from(row.reference_entry_count),
            saved.combats.len() as i64,
        );
    }
    for id in recorded.keys() {
        report.difference(
            format!("railway gun type={} id={}", *id as u32, *id >> 32),
            "present",
            "absent",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{LexicalTokenRow, ProvinceUnitOrderRow, RailwayGunCounterInputRow};

    const GUN: u64 = (7_u64 << 32) | 74;

    fn put(frame: &mut DecodedFrame, rows: TableRows) {
        let index = rows.table().index();
        *frame.tables[index] = TableView::Complete(rows);
    }

    fn gun() -> RailwayGunCounterInputRow {
        RailwayGunCounterInputRow {
            gun: GUN,
            roster_owner: 102,
            logical_owner: 101,
            province: 2,
            subunit_token: 1,
            name: replay::rows::Recorded::Value(Some(Vec::new())),
            // Native route cells are raw; nothing here requires known provinces.
            alternate_path: vec![u32::MAX],
            ..Default::default()
        }
    }

    fn fixture() -> DecodedFrame {
        let (mut frame, _, _, _) = super::super::tests::fixture();
        put(
            &mut frame,
            TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
                province: 2,
                mixed: vec![GUN],
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"railway_gun".to_vec(),
            }]),
        );
        put(&mut frame, TableRows::RailwayGunCounterInputs(vec![gun()]));
        frame
    }

    fn with(change: impl FnOnce(&mut RailwayGunCounterInputRow)) -> DecodedFrame {
        let mut frame = fixture();
        let mut row = gun();
        change(&mut row);
        put(&mut frame, TableRows::RailwayGunCounterInputs(vec![row]));
        frame
    }

    fn saved_gun() -> save::RailwayGun {
        save::RailwayGun {
            id: GUN,
            name: None,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: Some(2),
            path: vec![],
            support_attack: None,
            full_path: vec![u32::MAX],
            strategic_redeployment: false,
            strength: Decimal::parse(b"0").unwrap(),
            combats: vec![],
            country_intel: Vec::new(),
        }
    }

    #[test]
    fn complete_joins_accept_raw_route_cells() {
        assert_eq!(validate(&fixture()).unwrap(), 1);
    }

    #[test]
    fn rejects_unknown_references_and_inconsistent_motion() {
        for (change, expected) in [
            (
                (|row: &mut RailwayGunCounterInputRow| row.roster_owner = 99)
                    as fn(&mut RailwayGunCounterInputRow),
                "roster owner",
            ),
            (|row| row.assignee_group = 123, "assignee group"),
            (|row| row.subunit_token = 99, "subunit token"),
            (|row| row.target = 99, "unknown target"),
            (|row| row.transfer_action_status = 1, "transfer action"),
            (|row| row.retreat = 2, "non-boolean retreat"),
            (|row| row.province = 0, "mixed province roster"),
            (|row| row.combat_role = 1, "combat role"),
            (
                |row| {
                    row.combat_role = 2;
                    row.combat_province = 2;
                    row.combat_count = 1;
                    row.attacking = 1;
                },
                "attacking byte",
            ),
            (
                |row| {
                    row.combat_role = 1;
                    row.combat_province = 99;
                    row.combat_count = 1;
                    row.attacking = 1;
                },
                "unknown combat province",
            ),
        ] {
            let error = validate(&with(change)).unwrap_err().to_string();
            assert!(error.contains(expected), "{expected}: {error}");
        }
    }

    #[test]
    fn a_census_gap_fails_the_comparison_even_without_guns() {
        let mut frame = fixture();
        put(&mut frame, TableRows::RailwayGunCounterInputs(vec![]));
        put(&mut frame, TableRows::ProvinceUnitOrder(vec![]));
        assert_eq!(validate(&frame).unwrap(), 0);
        *frame.tables[TableId::ProvinceUnitOrder.index()] = TableView::Unavailable {
            reason: "synthetic gap".into(),
        };
        assert!(validate(&frame).is_err());
    }

    #[test]
    fn saved_target_and_alternate_route_detect_changes_and_omissions() {
        let differences = |frame: &DecodedFrame, gun: save::RailwayGun| {
            let (_, mut save, _, _) = super::super::tests::fixture();
            save.railway_guns.insert(GUN, gun);
            let mut report = Report::default();
            compare_saved(frame, &save, &mut report).unwrap();
            report.differences
        };
        let routed = |target, route: Vec<u32>| save::RailwayGun {
            support_attack: target,
            full_path: route,
            ..saved_gun()
        };
        let moving = with(|row| {
            row.target = 2;
            row.alternate_path = vec![2, 1, 2];
        });
        assert!(differences(&moving, routed(Some(2), vec![2, 1, 2])).is_empty());
        for route in [vec![2, 2, 1], vec![]] {
            let found = differences(&moving, routed(None, route));
            assert_eq!(found.len(), 2, "{found:?}");
            assert!(found[0].contains("target"));
            assert!(found[1].contains("alternate_path"));
        }
        // The writer omits a null target and an empty route.
        let idle = with(|row| row.alternate_path.clear());
        assert!(differences(&idle, routed(None, vec![])).is_empty());
    }

    #[test]
    fn fresh_save_compares_the_census_and_serialized_fields() {
        let frame = fixture();
        let (_, mut save, _, _) = super::super::tests::fixture();
        save.railway_guns.insert(GUN, saved_gun());
        let mut report = Report::default();
        compare_saved(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        let saved = save.railway_guns.get_mut(&GUN).unwrap();
        saved.path = vec![99];
        saved.combats = vec![(123_u64 << 32) | 62];
        saved.strength = Decimal::parse(b"1").unwrap();
        let mut report = Report::default();
        compare_saved(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3);

        save.railway_guns.clear();
        let mut report = Report::default();
        compare_saved(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn saved_names_compare_all_bytes_empty_omission_and_capture_failures() {
        let differences = |actual, saved| {
            let frame = with(|row| row.name = actual);
            let (_, mut save, _, _) = super::super::tests::fixture();
            save.railway_guns.insert(
                GUN,
                save::RailwayGun {
                    name: saved,
                    ..saved_gun()
                },
            );
            let mut report = Report::default();
            compare_saved(&frame, &save, &mut report).unwrap();
            report.differences
        };
        use replay::rows::Recorded::{NotRecorded, Value};
        let name = vec![0xff, b'\\', b'"', b'G'];
        assert!(differences(Value(Some(name.clone())), Some(name)).is_empty());
        assert!(differences(Value(Some(Vec::new())), None).is_empty());
        assert!(differences(NotRecorded, None).is_empty());
        for (actual, saved) in [
            (Value(None), None),
            (Value(Some(Vec::new())), Some(Vec::new())),
            (Value(Some(b"G".to_vec())), None),
            // The native C-string writer truncates; the captured suffix is
            // still a save difference rather than a waived/truncated capture.
            (Value(Some(vec![b'G', 0, b'!'])), Some(b"G".to_vec())),
        ] {
            let found = differences(actual, saved);
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].contains("name"), "{found:?}");
        }
    }

    /// W2: a spawned gun supporting two combats saves both in its reference
    /// array while its CUnit combat vector stays empty.
    #[test]
    fn saved_combats_are_the_reference_entries_not_the_unit_combat_vector() {
        let (_, mut save, _, _) = super::super::tests::fixture();
        save.railway_guns.insert(
            GUN,
            save::RailwayGun {
                combats: vec![(21081_u64 << 32) | 62, (21078_u64 << 32) | 62],
                ..saved_gun()
            },
        );
        let supporting = with(|row| row.reference_entry_count = 2);
        assert_eq!(validate(&supporting).unwrap(), 1);
        let mut report = Report::default();
        compare_saved(&supporting, &save, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        let mut report = Report::default();
        compare_saved(&fixture(), &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(
            report.differences[0].contains("reference_entry_count"),
            "{:?}",
            report.differences
        );
    }
}
