//! Saved portrait identities and province membership witnesses.
use super::*;
fn eq<T: PartialEq + std::fmt::Debug>(r: &mut Report, label: &str, a: T, b: T) {
    r.scalar_checks += 1;
    if a != b {
        r.difference(label, a, b);
    }
}
pub(super) fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::ProvinceUnitOrder, |r| provinces(frame, save, r));
    report.attempt_table(TableId::CharacterPortraits, |r| characters(frame, save, r));
    report.attempt_table(TableId::LeaderPortraits, |r| leaders(frame, save, r));
}
fn provinces(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::ProvinceUnitOrder>()?;
    let province_ids: BTreeSet<_> = frame
        .complete::<tables::Provinces>()?
        .iter()
        .map(|p| p.id)
        .collect();
    eq(
        r,
        "ProvinceUnitOrder province census",
        rows.iter().map(|p| p.province).collect::<BTreeSet<_>>(),
        province_ids,
    );
    let mut seen_divisions = BTreeSet::new();
    let mut seen_tasks = BTreeSet::new();
    for row in rows {
        for &handle in &row.mixed {
            if let Some(d) = save.divisions.get(&handle) {
                if !seen_divisions.insert(handle) {
                    return Err(invalid("division repeated in province mixed roster"));
                }
                eq(
                    r,
                    &format!("ProvinceUnitOrder {handle} division location"),
                    Some(row.province),
                    d.province,
                );
            } else if let Some(t) = save.task_forces.get(&handle) {
                if !seen_tasks.insert(handle) {
                    return Err(invalid("task force repeated in province mixed roster"));
                }
                eq(
                    r,
                    &format!("ProvinceUnitOrder {handle} task location"),
                    Some(row.province),
                    t.province,
                );
            } else {
                r.limitation("this mixed-roster handle has no saved division/task-force counterpart; railway-gun membership is outside this comparison",format!("ProvinceUnitOrder {handle}"));
            }
        }
        let expected: BTreeSet<_> = save
            .task_forces
            .values()
            .filter(|t| t.province == Some(row.province))
            .map(|t| t.id)
            .collect();
        eq(
            r,
            &format!("ProvinceUnitOrder {} naval membership", row.province),
            row.naval.iter().copied().collect::<BTreeSet<_>>(),
            expected,
        );
        if row.naval.iter().collect::<BTreeSet<_>>().len() != row.naval.len() {
            return Err(invalid("duplicate naval province occupant"));
        }
    }
    eq(
        r,
        "ProvinceUnitOrder located division census",
        seen_divisions,
        save.divisions
            .values()
            .filter(|d| d.province.is_some())
            .map(|d| d.id)
            .collect(),
    );
    eq(
        r,
        "ProvinceUnitOrder located task-force census",
        seen_tasks,
        save.task_forces
            .values()
            .filter(|t| t.province.is_some())
            .map(|t| t.id)
            .collect(),
    );
    r.compared("ProvinceUnitOrder", rows.len());
    r.limitation("CProvince E6CA90 omits occupant arrays; CUnit BF7010 saves location but no province insertion ordinal. Country unit serialization order is not an independent witness of the current province order. Membership/location are checked; mixed/naval ordinals remain unverified", "ProvinceUnitOrder order");
    Ok(())
}
fn characters(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::CharacterPortraits>()?;
    let manager = save
        .characters
        .manager
        .as_ref()
        .ok_or_else(|| invalid("save lacks character manager"))?;
    let expected: BTreeSet<_> = manager
        .characters
        .keys()
        .map(|id| (id >> 32) as u32)
        .collect();
    eq(
        r,
        "CharacterPortraits character census",
        rows.iter().map(|p| p.character).collect::<BTreeSet<_>>(),
        expected,
    );
    const ROLES: [&[u8]; 6] = [
        b"civilian",
        b"army",
        b"navy",
        b"air",
        b"operative",
        b"scientist",
    ];
    const SIZES: [&[u8]; 2] = [b"small", b"large"];
    for row in rows {
        let Some(character) = manager
            .characters
            .get(&((u64::from(row.character) << 32) | 73))
        else {
            continue;
        };
        let saved = character
            .portraits
            .as_ref()
            .ok_or_else(|| invalid("character save lacks portrait holder"))?;
        let mut projected = Vec::new();
        for entry in &row.portraits {
            let role = ROLES
                .get(entry.category as usize)
                .ok_or_else(|| invalid("portrait category outside serializer mapping"))?;
            let size = SIZES
                .get(entry.size as usize)
                .ok_or_else(|| invalid("portrait size outside serializer mapping"))?;
            if entry.kind > 1 {
                return Err(invalid("portrait kind has no serialized scalar"));
            }
            projected.push(save::characters::Portrait {
                category: role.to_vec(),
                size: size.to_vec(),
                identity: entry
                    .identity
                    .split(|&b| b == 0)
                    .next()
                    .unwrap_or_default()
                    .to_vec(),
            });
        }
        eq(
            r,
            &format!("CharacterPortraits {} ordered identities", row.character),
            &projected,
            saved,
        );
    }
    r.compared("CharacterPortraits", rows.len());
    r.limitation("138A110/13887E0 serialize category, size and the identity string for kinds0/1; the set byte and kind distinction are not serialized. Text identity comparison ends at embedded NUL; trailing resource/cache pointers are not part of the contract", "CharacterPortraits descriptor");
    Ok(())
}
fn leaders(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::LeaderPortraits>()?;
    let by_id: BTreeMap<_, _> = rows.iter().map(|p| (p.leader, p)).collect();
    for (&id, saved) in &save.characters.portraits {
        let Some(row) = by_id.get(&id) else {
            r.difference(format!("LeaderPortraits {id}"), "absent", "saved");
            continue;
        };
        for (index, actual) in [&row.portrait_c0, &row.portrait_e0, &row.portrait_100]
            .into_iter()
            .enumerate()
        {
            eq(
                r,
                &format!("LeaderPortraits {id} string{index}"),
                actual.split(|&b| b == 0).next().unwrap_or_default(),
                saved[index].as_slice(),
            );
        }
    }
    r.compared("LeaderPortraits", rows.len());
    r.limitation("C0D920 omits empty portrait_path/picture/gfx; comparison uses their proven empty-string default and text bytes through NUL. All serialized role wrappers are checked, including non-character leaders; extra registry-resident unsaved leaders and raw character references cannot be verified by these strings", "LeaderPortraits");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{CharacterPortrait, CharacterPortraitRow, LeaderPortraitRow};
    #[test]
    fn ordered_portraits_and_noncharacter_strings_compare_and_mismatches_fail() {
        let (mut frame, mut saved, _, _) = super::super::tests::fixture();
        let doc=installed_content::parser::parse(br#"countries={GER={units={corps_commander={id={id=8 type=4713} portrait_path="x.dds" picture="p" gfx="GFX_z"}}}} character_manager={historical={character={id={id=7 type=73} token=x portraits={civilian={small="GFX_a"} civilian={large="GFX_b"}}}}}"#).unwrap();
        saved.characters = save::characters::parse(&doc.items).unwrap();
        *frame.tables[TableId::CharacterPortraits.index()] =
            TableView::Complete(TableRows::CharacterPortraits(vec![CharacterPortraitRow {
                character: 7,
                portraits: vec![
                    CharacterPortrait {
                        category: 0,
                        size: 0,
                        is_set: 255,
                        kind: 0,
                        identity: b"GFX_a".to_vec(),
                    },
                    CharacterPortrait {
                        category: 0,
                        size: 1,
                        is_set: 1,
                        kind: 0,
                        identity: b"GFX_b".to_vec(),
                    },
                ],
            }]));
        *frame.tables[TableId::LeaderPortraits.index()] =
            TableView::Complete(TableRows::LeaderPortraits(vec![LeaderPortraitRow {
                leader: (8 << 32) | 4713,
                character: 0,
                portrait_c0: b"x.dds".to_vec(),
                portrait_e0: b"p".to_vec(),
                portrait_100: b"GFX_z".to_vec(),
            }]));
        let mut report = Report::default();
        characters(&frame, &saved, &mut report).unwrap();
        leaders(&frame, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 5);
        if let TableView::Complete(TableRows::CharacterPortraits(rows)) =
            &mut *frame.tables[TableId::CharacterPortraits.index()]
        {
            rows[0].portraits.swap(0, 1);
        }
        saved
            .characters
            .portraits
            .get_mut(&((8 << 32) | 4713))
            .unwrap()[1] = b"different".to_vec();
        let mut report = Report::default();
        characters(&frame, &saved, &mut report).unwrap();
        leaders(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
    }
}
