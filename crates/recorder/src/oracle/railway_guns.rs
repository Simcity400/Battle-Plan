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
            "The save omits naval transfer, the unit combat vector (its count, the first combat's side and province, and the attacking byte), the assignee pointer, the stat definition and its maximum strength, the received-supply ratio and the railway building level at the gun; retreat is not compared by this oracle; its combat list is the gun's own reference array, compared by its stored count; compare the rest against paused memory at the sampled hour",
            "RailwayGunCounterInputs live-only operands",
        );
        Ok(())
    });
    report.attempt("RailwayGunCounterInputs fresh", |r| compare_fresh(frame, r));
}

fn compare_fresh(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::RailwayGunCounterInputs>()?;
    for row in rows {
        let label = format!("RailwayGunCounterInputs {}.fresh", row.gun);
        let Some(packet) = report.recorded(&row.fresh, "RailwayGunCounterInputs.fresh", &label)
        else {
            continue;
        };
        let Some(packet) = packet else {
            report.difference(
                &label,
                "unavailable",
                "required current-hour railway-gun packet",
            );
            continue;
        };
        if !report.packet_fresh(&label, packet.sample_hour, frame.hour) {
            continue;
        }
        if row.naval_transfer == 0 {
            for (name, missing) in [
                ("maximum_speed", packet.maximum_speed.is_none()),
                ("land_supply_factor", packet.land_supply_factor.is_none()),
            ] {
                if missing {
                    report.difference(
                        format!("{label}.{name}"),
                        "unavailable",
                        "required land getter operand",
                    );
                }
            }
        } else {
            report.limitation("nonnull native transfer selects transport and does not reach land maximum-speed or land-supply getters", &label);
        }
        if packet.orders_group.is_none() {
            report.difference(
                format!("{label}.orders_group"),
                "unavailable",
                "required genuine unit group identity",
            );
        }
        report.limitation("fresh variant maximum speed and composed land-supply factor are runtime getters, independently of saved equipment or strength; the unit parent group is distinct from the railway assignee and has no proved text-save join in this comparison", &label);
    }
    report.compared("RailwayGunCounterInputs.fresh", rows.len());
    Ok(())
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
                // The railway-gun writer delegates to the unit writer,
                // which gates the name on stored length.
                let label = format!("{label}.name");
                let name = (!name.is_empty()).then(|| super::serialized_text(report, &label, name));
                check(report, &label, name, saved.name.as_deref());
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
            // The unit writer drops signed-nonpositive expeditionary tags; only
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
        // The unit save writer emits the movement target as support_attack
        // when nonnull, and the alternate route as full_path when nonempty.
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
        // The railway-gun writer delegates to the unit writer,
        // which writes the exile flag only when nonzero.
        check(
            report,
            &format!("{label}.exiled truth"),
            row.exiled != 0,
            saved.exile,
        );
        if let Some(value) = report.recorded(
            &row.withdrawal,
            "RailwayGunCounterInputs.withdrawal",
            format!("{label}.withdrawal"),
        ) {
            match value {
                Some(raw) => check(
                    report,
                    &format!("{label}.withdrawal"),
                    *raw != 0,
                    saved.withdrawal,
                ),
                None => report.difference(
                    format!("{label}.withdrawal: guarded read refusal"),
                    "unavailable",
                    saved.withdrawal,
                ),
            }
        }
        report.scalar(
            &label,
            "current_strength",
            Some(row.current_strength),
            Some(&saved.strength),
        );
        // A gun's saved `combat` list is its own reference array: the railway-gun
        // writer emits it when its count is nonzero. The reference-presence
        // test uses that same count. The unit writer never saves the unit
        // combat vector that `combat_count` holds, so that has no saved
        // counterpart for a gun. The writer emits exactly the stored count,
        // making it the saved list's length.
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
    report.limitation("the unit save writer saves withdraw only as zero/nonzero truth, not raw nonzero byte identity", "RailwayGunCounterInputs.withdrawal raw byte");
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
            withdrawal: replay::rows::Recorded::Value(Some(0)),
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
            exile: false,
            withdrawal: false,
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
    fn saved_withdrawal_compares_truth_and_does_not_hide_prefix_differences() {
        use replay::rows::Recorded::{NotRecorded, Value};
        for (value, expected, differences) in [
            (Value(Some(0)), false, 0),
            (Value(Some(255)), true, 0),
            (Value(Some(2)), false, 1),
            (Value(None), false, 1),
            (NotRecorded, true, 0),
        ] {
            let frame = with(|row| row.withdrawal = value);
            let (_, mut save, _, _) = super::super::tests::fixture();
            save.railway_guns.insert(
                GUN,
                save::RailwayGun {
                    withdrawal: expected,
                    ..saved_gun()
                },
            );
            let mut report = Report::default();
            compare_saved(&frame, &save, &mut report).unwrap();
            assert_eq!(
                report.differences.len(),
                differences,
                "{:?}",
                report.differences
            );
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("zero/nonzero truth"))
            );
            save.railway_guns.get_mut(&GUN).unwrap().path.push(99);
            let mut report = Report::default();
            compare_saved(&frame, &save, &mut report).unwrap();
            assert_eq!(
                report.differences.len(),
                differences + 1,
                "{:?}",
                report.differences
            );
        }
    }

    #[test]
    fn saved_exile_compares_truth_and_preserves_existing_path_differences() {
        for (raw, expected, differences) in
            [(0, false, 0), (1, true, 0), (0, true, 1), (1, false, 1)]
        {
            let frame = with(|row| row.exiled = raw);
            let (_, mut saved, _, _) = super::super::tests::fixture();
            saved.railway_guns.insert(
                GUN,
                save::RailwayGun {
                    exile: expected,
                    ..saved_gun()
                },
            );
            let mut report = Report::default();
            compare_saved(&frame, &saved, &mut report).unwrap();
            assert_eq!(report.differences.len(), differences);
            saved.railway_guns.get_mut(&GUN).unwrap().path.push(99);
            let mut report = Report::default();
            compare_saved(&frame, &saved, &mut report).unwrap();
            assert_eq!(report.differences.len(), differences + 1);
            assert!(
                report
                    .differences
                    .iter()
                    .any(|difference| difference.contains("ordinary_path"))
            );
        }
    }

    #[test]
    fn saved_names_compare_first_nul_empty_omission_and_capture_failures() {
        let run = |actual, saved| {
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
            report
        };
        use replay::rows::Recorded::{NotRecorded, Value};
        let name = vec![0xff, b'\\', b'"', b'G'];
        assert!(
            run(Value(Some(name.clone())), Some(name))
                .differences
                .is_empty()
        );
        assert!(run(Value(Some(Vec::new())), None).differences.is_empty());
        assert!(run(NotRecorded, None).differences.is_empty());
        for name in [b"G\0!".as_slice(), b"\0!".as_slice()] {
            let report = run(
                Value(Some(name.to_vec())),
                Some(installed_content::parser::c_string(name).to_vec()),
            );
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            assert_eq!(
                report
                    .limitations
                    .values()
                    .flatten()
                    .filter(|field| field.ends_with(".name"))
                    .count(),
                1
            );
        }
        for (actual, saved) in [
            (Value(None), None),
            (Value(Some(Vec::new())), Some(Vec::new())),
            (Value(Some(b"G".to_vec())), None),
            (Value(Some(b"\0!".to_vec())), None),
            (Value(Some(b"G\0!".to_vec())), Some(b"X".to_vec())),
        ] {
            let found = run(actual, saved).differences;
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].contains("name"), "{found:?}");
        }
    }

    #[test]
    fn fresh_railway_packets_preserve_transfer_gates_and_prefix_differences() {
        use replay::rows::{FreshRailwayCounterEta, Recorded};
        let mut frame = fixture();
        let packet = FreshRailwayCounterEta {
            sample_hour: replay::rows::PacketHour::Host,
            maximum_speed: Some(i64::MIN),
            land_supply_factor: Some(i64::MAX),
            orders_group: Some(0),
        };
        let row = RailwayGunCounterInputRow {
            ordinary_path: vec![1],
            fresh: Recorded::Value(Some(packet)),
            ..gun()
        };
        let mut run = |row: RailwayGunCounterInputRow| {
            put(&mut frame, TableRows::RailwayGunCounterInputs(vec![row]));
            let mut report = Report::default();
            compare_fresh(&frame, &mut report).unwrap();
            report
        };
        assert!(run(row.clone()).differences.is_empty());
        for change in [
            (|packet: &mut FreshRailwayCounterEta| packet.maximum_speed = None)
                as fn(&mut FreshRailwayCounterEta),
            |packet| packet.land_supply_factor = None,
            |packet| packet.orders_group = None,
        ] {
            let mut packet = packet;
            change(&mut packet);
            assert_eq!(
                run(RailwayGunCounterInputRow {
                    fresh: Recorded::Value(Some(packet)),
                    ..row.clone()
                })
                .differences
                .len(),
                1
            );
        }
        let transfer = FreshRailwayCounterEta {
            maximum_speed: None,
            land_supply_factor: None,
            ..packet
        };
        assert!(
            run(RailwayGunCounterInputRow {
                naval_transfer: 1,
                fresh: Recorded::Value(Some(transfer)),
                ..row.clone()
            })
            .differences
            .is_empty()
        );
        assert_eq!(
            run(RailwayGunCounterInputRow {
                naval_transfer: 1,
                fresh: Recorded::Value(Some(FreshRailwayCounterEta {
                    orders_group: None,
                    ..transfer
                })),
                ..row.clone()
            })
            .differences
            .len(),
            1
        );
        assert_eq!(
            run(RailwayGunCounterInputRow {
                fresh: Recorded::Value(None),
                ..row.clone()
            })
            .differences
            .len(),
            1
        );
        // The genuine unit group and fresh profile are captured even while idle.
        let idle_refusal = run(RailwayGunCounterInputRow {
            ordinary_path: vec![],
            fresh: Recorded::Value(None),
            ..row.clone()
        });
        assert_eq!(
            idle_refusal.differences.len(),
            1,
            "{:?}",
            idle_refusal.differences
        );
        assert!(
            idle_refusal.differences[0].contains(&format!("RailwayGunCounterInputs {GUN}.fresh"))
        );
        assert!(idle_refusal.outcome().is_err());
        assert!(
            run(RailwayGunCounterInputRow {
                ordinary_path: vec![],
                ..row.clone()
            })
            .differences
            .is_empty()
        );
        let historical = run(RailwayGunCounterInputRow {
            fresh: Recorded::NotRecorded,
            ..row.clone()
        });
        assert!(historical.differences.is_empty());
        assert_eq!(
            historical.limitations["RailwayGunCounterInputs.fresh not recorded"],
            vec![format!("RailwayGunCounterInputs {GUN}.fresh")],
        );
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![RailwayGunCounterInputRow {
                ordinary_path: vec![2],
                fresh: Recorded::Value(None),
                ..row
            }]),
        );
        let (_, mut saved, _, _) = super::super::tests::fixture();
        saved.railway_guns.insert(
            GUN,
            save::RailwayGun {
                path: vec![1],
                ..saved_gun()
            },
        );
        let mut report = Report::default();
        compare_saved(&frame, &saved, &mut report).unwrap();
        compare_fresh(&frame, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("ordinary_path"))
        );
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(".fresh"))
        );
    }

    /// W2: a spawned gun supporting two combats saves both in its reference
    /// array while its unit combat vector stays empty.
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
