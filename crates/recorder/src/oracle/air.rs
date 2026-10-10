//! Comparisons for the native extended air tables.
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    io,
};

use installed_content::save::air::{self, Pack};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
};

use super::{Report, complete, country_tag, invalid, required_country_tag};

fn check<T: PartialEq + Debug>(report: &mut Report, label: &str, actual: T, expected: T) {
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

fn unique<K: Ord + Debug, V>(
    rows: impl IntoIterator<Item = (K, V)>,
    label: &str,
) -> io::Result<BTreeMap<K, V>> {
    let mut result = BTreeMap::new();
    for (key, value) in rows {
        if result.insert(key, value).is_some() {
            return Err(invalid(format!(
                "ambiguous duplicate {label} parent identity"
            )));
        }
    }
    Ok(result)
}

fn unique_checked<K: Ord + Debug, V>(
    rows: impl IntoIterator<Item = io::Result<(K, V)>>,
) -> io::Result<BTreeMap<K, V>> {
    let mut result = BTreeMap::new();
    for row in rows {
        let (key, value) = row?;
        if result.insert(key, value).is_some() {
            return Err(invalid("ambiguous duplicate captured air parent identity"));
        }
    }
    Ok(result)
}

fn missing(report: &mut Report, label: impl AsRef<str>) {
    report.difference(label, "absent", "present");
}

fn extra<K: Debug, V>(report: &mut Report, label: &str, rows: &BTreeMap<K, V>) {
    for key in rows.keys() {
        report.difference(format!("{label} {key:?}"), "present", "absent");
    }
}

fn tag<'a>(id: u32, links: &BTreeMap<u32, &'a CountryRow>) -> io::Result<&'a [u8]> {
    Ok(country_tag(id, links)?.unwrap_or(b"---"))
}

pub(super) fn compare(
    frame: &DecodedFrame,
    save: &Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = report.attempt("air AirBases", |report| compare_bases(frame, save, report));
    complete &= report.attempt("air WingCumulative", |report| {
        compare_cumulative(frame, save, report)
    });
    complete &= report.attempt("air WingEquipment", |report| {
        compare_equipment(frame, save, report)
    });
    complete &= report.attempt("air AirAces", |report| {
        compare_aces(frame, save, links, report)
    });
    complete &= report.attempt("air AirRegionHistory", |report| {
        compare_history(frame, save, links, report)
    });
    complete &= report.attempt("air AirRegionDamage", |report| {
        compare_damage(frame, save, links, report)
    });
    complete &= report.attempt("air AirStatistics", |report| {
        compare_statistics(frame, save, links, report)
    });
    complete &= report.attempt("air RaidAirScores", |report| {
        let rows = super::complete::<tables::RaidAirScores>(frame)?;
        for row in rows { required_country_tag(row.country, links)?; }
        report.limitation("CStrategicAir::vf2@140C57230 omits the entire +E0 regional-score array; fresh saves cannot certify the stored superiority operands", "RaidAirScores.friendly / enemy");
        Ok(())
    });
    if complete {
        Ok(())
    } else {
        Err(invalid(
            "air comparison incomplete; see scoped table failures",
        ))
    }
}

fn compare_bases(frame: &DecodedFrame, save: &Pack, report: &mut Report) -> io::Result<()> {
    let mut actual = unique(
        complete::<tables::AirBases>(frame)?
            .iter()
            .map(|row| (row.id, row)),
        "captured air base",
    )?;
    let expected = unique(save.bases.iter().map(|row| (row.id, row)), "saved air base")?;
    let mut matched = 0;
    for (id, expected) in expected {
        let label = format!("air base {id}");
        let Some(actual) = actual.remove(&id) else {
            missing(report, &label);
            continue;
        };
        matched += 1;
        check(
            report,
            &format!("{label}.state"),
            actual.state,
            expected.state,
        );
        check(
            report,
            &format!("{label}.capacity"),
            actual.capacity,
            expected.capacity,
        );
        check(report, &format!("{label}.kind"), actual.kind, expected.kind);
        if let Some(host) = expected.host {
            check(report, &format!("{label}.host"), actual.host, host);
        }
    }
    extra(report, "air base", &actual);
    report.compared("AirBases", matched);
    report.limitation("air base province is a cached pointer-derived location omitted by C57070; a state base's host reference is also omitted", "AirBases.province / state-base host");
    Ok(())
}

fn compare_cumulative(frame: &DecodedFrame, save: &Pack, report: &mut Report) -> io::Result<()> {
    let mut cumulative = unique(
        complete::<tables::WingCumulative>(frame)?
            .iter()
            .map(|row| (row.id, row)),
        "captured wing cumulative",
    )?;
    let expected = unique(save.wings.iter().map(|row| (row.id, row)), "saved wing")?;
    let mut matched = 0;
    for (id, expected) in expected {
        let label = format!("wing {id}");
        if let Some(actual) = cumulative.remove(&id) {
            matched += 1;
            report.scalar(
                &label,
                "experience",
                Some(actual.experience),
                Some(&expected.experience),
            );
            if let Some(accidents) = expected.accidents {
                check(
                    report,
                    &format!("{label}.accidents"),
                    actual.accidents,
                    accidents,
                );
            } else if actual.accidents > 0 {
                report.difference(
                    format!("{label}.accidents"),
                    actual.accidents,
                    "omitted: native value must be nonpositive",
                );
            } else {
                report.limitation("F539B0 omits every nonpositive accident count; the exact raw omitted value is not serialized", format!("{label}.accidents"));
            }
        } else {
            missing(report, format!("{label}.cumulative"));
        }
    }
    extra(report, "wing cumulative", &cumulative);
    report.compared("WingCumulative", matched);
    report.limitation("wing quantity differences are not evidence of combat losses; accidents, retained regional damage and native annual statistics have distinct meanings", "WingCumulative / AirRegionDamage / AirStatistics");
    Ok(())
}

fn compare_equipment(frame: &DecodedFrame, save: &Pack, report: &mut Report) -> io::Result<()> {
    let equipment = unique(
        complete::<tables::WingEquipment>(frame)?
            .iter()
            .map(|row| ((row.wing, row.ordinal), row)),
        "captured wing equipment",
    )?;
    let mut banks = BTreeMap::<u64, Vec<_>>::new();
    for ((wing, ordinal), row) in equipment {
        let entries = banks.entry(wing).or_default();
        if ordinal as usize != entries.len() {
            return Err(invalid("wing equipment native ordinals are not contiguous"));
        }
        entries.push(row);
    }
    let expected = unique(save.wings.iter().map(|row| (row.id, row)), "saved wing")?;
    let mut equipment_matched = 0;
    for (id, expected) in expected {
        let label = format!("wing {id}");
        let entries = banks.remove(&id).unwrap_or_default();
        let visible = entries
            .iter()
            .filter(|entry| expected.equipment.allow_zero || entry.amount_raw != 0)
            .collect::<Vec<_>>();
        check(
            report,
            &format!("{label}.equipment.count"),
            visible.len(),
            expected.equipment.entries.len(),
        );
        for (ordinal, (actual, expected)) in
            visible.iter().zip(&expected.equipment.entries).enumerate()
        {
            equipment_matched += 1;
            check(
                report,
                &format!("{label}.equipment[{ordinal}].variant"),
                actual.variant,
                expected.variant,
            );
            report.scalar(
                &format!("{label}.equipment[{ordinal}]"),
                "amount",
                Some(actual.amount_raw),
                Some(&expected.amount),
            );
        }
    }
    extra(report, "wing equipment parent", &banks);
    report.compared("WingEquipment", equipment_matched);
    report.limitation("equipment serialization preserves the ordered visible sequence, omitting zero entries when allow_zero_entries is false; omitted zero membership and absolute native ordinal are not save-certified", "WingEquipment");
    Ok(())
}

fn compare_aces(
    frame: &DecodedFrame,
    save: &Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = unique(
        complete::<tables::AirAces>(frame)?
            .iter()
            .map(|row| (row.id, row)),
        "captured air ace",
    )?;
    let expected = unique(save.aces.iter().map(|row| (row.id, row)), "saved ace")?;
    let assignments = unique(
        save.wings
            .iter()
            .filter_map(|wing| wing.ace.map(|ace| (ace, wing.id))),
        "saved ace assignment",
    )?;
    let mut matched = 0;
    for (id, expected) in expected {
        let label = format!("air ace {id}");
        let Some(actual) = actual.remove(&id) else {
            missing(report, &label);
            continue;
        };
        matched += 1;
        check(
            report,
            &format!("{label}.country"),
            required_country_tag(actual.country, links)?,
            expected.country.as_slice(),
        );
        check(
            report,
            &format!("{label}.ordinal"),
            actual.ordinal,
            expected.ordinal,
        );
        check(
            report,
            &format!("{label}.alive"),
            actual.alive,
            u8::from(expected.alive),
        );
        check(
            report,
            &format!("{label}.handled"),
            actual.handled,
            u8::from(expected.handled),
        );
        check(
            report,
            &format!("{label}.kill_type"),
            actual.kill_type,
            expected.kill_type,
        );
        for (name, actual, expected) in [
            ("name", &actual.name, &expected.name),
            ("surname", &actual.surname, &expected.surname),
        ] {
            // CAce::Serialize@14060F1F0 always emits +0xF0/+0x110.
            let label = format!("{label}.{name}");
            let text = super::serialized_text(report, &label, actual);
            check(report, &label, text, expected.as_slice());
        }
        // 14060F1F0 gates the +0x130 callsign on stored length +0x140.
        let callsign_label = format!("{label}.callsign");
        let callsign = (!actual.callsign.is_empty())
            .then(|| super::serialized_text(report, &callsign_label, &actual.callsign));
        check(
            report,
            &callsign_label,
            callsign,
            expected.callsign.as_deref(),
        );
        match &actual.presentation {
            replay::rows::Recorded::Value(Some(fields)) => {
                check(report, &format!("{label}.modifier"), &fields.definition, &expected.modifier);
                check(report, &format!("{label}.is_female"), fields.gender != 0, expected.female);
                check(report, &format!("{label}.portrait"), fields.pool_index, expected.portrait);
            }
            replay::rows::Recorded::Value(None) => report.difference(
                format!("{label}.presentation"), "capture failed", "saved modifier / portrait operands"),
            replay::rows::Recorded::NotRecorded => report.limitation(
                "ace presentation operands not recorded; the established roster fields still compare",
                format!("{label}.presentation")),
        }
        match (&actual.death, expected.alive) {
            (Some(death), false) => {
                check(
                    report,
                    &format!("{label}.killer_ace"),
                    death.killer_ace,
                    expected.killer_ace,
                );
                if let Some(country) = &expected.killer_country {
                    check(
                        report,
                        &format!("{label}.killer_country"),
                        tag(death.killer_country, links)?,
                        country.as_slice(),
                    );
                } else if (death.killer_country as i32) > 0 {
                    report.difference(
                        format!("{label}.killer_country"),
                        death.killer_country,
                        "omitted: native value must be nonpositive",
                    );
                } else {
                    report.limitation("CAce omits every nonpositive killer-country reference; exact omitted reference bits are not serialized", format!("{label}.killer_country"));
                }
            }
            (None, true) => {}
            _ => report.difference(
                format!("{label}.death applicability"),
                actual.death.is_some(),
                !expected.alive,
            ),
        }
        if let Some(&wing) = assignments.get(&id) {
            check(
                report,
                &format!("{label}.assigned_wing"),
                actual.assigned_wing,
                wing,
            );
        } else {
            report.limitation("CAce does not serialize its reverse wing reference; an omitted wing ace field also permits an unresolved reference", format!("{label}.assigned_wing"));
        }
    }
    extra(report, "air ace", &actual);
    report.compared("AirAces", matched);
    report.limitation("CAce aircraft-type mask is not compared: the saved modifier definition-to-mask mapping is not implemented/proven", "AirAces.aircraft_type_mask");
    report.limitation("the raw nonzero gender byte and CGameGui retained portrait pools/culture are not serialized; is_female compares their boolean meaning only, and artwork needs a matched-hour native portrait", "AirAces.presentation.portrait");
    report.limitation(
        "saved country tags cannot certify raw country-link alias bits",
        "extended air country references",
    );
    Ok(())
}

#[test]
fn ace_names_use_first_nul_and_callsign_stored_length() {
    let (mut frame, _, _, _) = super::tests::fixture();
    let country = CountryRow {
        id: 101,
        tag: b"AAA".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(101, &country)]);
    for (stored, callsign, differences) in [
        (b"C\0tail".as_slice(), "callsign=C", 0),
        (b"\0tail".as_slice(), "callsign=\"\"", 0),
        (b"".as_slice(), "", 0),
        (b"".as_slice(), "callsign=\"\"", 1),
        (b"C\0tail".as_slice(), "callsign=D", 1),
    ] {
        let doc = installed_content::parser::parse(format!("countries={{AAA={{ace={{id={{id=1 type=4}} name=N surname=S portrait=0 {callsign}}}}}}}").as_bytes()).unwrap();
        let saved = air::parse(&doc.items).unwrap();
        *frame.tables[replay::rows::TableId::AirAces.index()] =
            replay::frame::TableView::Complete(replay::rows::TableRows::AirAces(vec![
                replay::rows::AirAceRow {
                    id: (1_u64 << 32) | 4,
                    country: 101,
                    alive: 1,
                    name: b"N\0tail".to_vec(),
                    surname: b"S\0tail".to_vec(),
                    callsign: stored.to_vec(),
                    presentation: replay::rows::Recorded::NotRecorded,
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        compare_aces(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert_eq!(
            report
                .limitations
                .values()
                .flatten()
                .filter(|field| field.ends_with(".name")
                    || field.ends_with(".surname")
                    || field.ends_with(".callsign"))
                .count(),
            2 + usize::from(stored.contains(&0))
        );
    }
}

fn record_key(record: &air::CombatRecord) -> (u8, i32, u32, u8) {
    (
        record.side,
        record.date,
        record.wing_id,
        u8::from(record.classification),
    )
}

fn compare_history(
    frame: &DecodedFrame,
    save: &Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut histories = unique_checked(complete::<tables::AirRegionHistory>(frame)?.iter().map(
        |row| {
            Ok((
                (
                    required_country_tag(row.country, links)?.to_vec(),
                    row.region,
                ),
                row,
            ))
        },
    ))?;
    let expected = unique(
        save.regions
            .iter()
            .map(|row| ((row.country.clone(), row.region), row)),
        "saved air combat region",
    )?;
    let mut history_matched = 0;
    for (key, expected) in expected {
        let label = format!("air region {}:{}", String::from_utf8_lossy(&key.0), key.1);
        if let Some(actual) = histories.remove(&key) {
            check(
                report,
                &format!("{label}.history.count"),
                actual.entries.len(),
                expected.entries.len(),
            );
            for (ordinal, (actual, entry)) in
                actual.entries.iter().zip(&expected.entries).enumerate()
            {
                history_matched += 1;
                let label = format!("{label}.history[{ordinal}]");
                check(
                    report,
                    &format!("{label}.identity"),
                    (
                        actual.side,
                        actual.date,
                        actual.wing_id,
                        actual.classification,
                    ),
                    record_key(entry),
                );
                check(
                    report,
                    &format!("{label}.region_owner"),
                    tag(actual.region_owner, links)?,
                    expected.owner.as_slice(),
                );
                check(
                    report,
                    &format!("{label}.planes"),
                    actual.planes,
                    entry.planes,
                );
                check(
                    report,
                    &format!("{label}.mission_mask"),
                    actual.mission_mask,
                    entry.mission_mask,
                );
                check(
                    report,
                    &format!("{label}.selection"),
                    actual.selection,
                    u8::from(entry.selection),
                );
                check(
                    report,
                    &format!("{label}.copied"),
                    actual.copied,
                    u8::from(entry.equipment.allow_zero),
                );
                check(
                    report,
                    &format!("{label}.attributed_country"),
                    tag(actual.attributed_country, links)?,
                    entry.attributed_country.as_slice(),
                );
            }
        } else if !expected.entries.is_empty() {
            missing(report, format!("{label}.history"));
        }
    }
    extra(report, "air region history", &histories);
    report.compared("AirRegionHistory", history_matched);
    report.limitation("regional history captures retained native records, not an event stream or current wing census; recorded high-water counts and original dates are compared without substituting current wing values", "AirRegionHistory");
    report.limitation("the copied historical equipment pool's entries are serialized but outside the minimum recorded history fields", "AirRegionHistory.equipment");
    report.limitation("grouped history rows originate from emitted entries; allocation of empty native region holders is not certified", "AirRegionHistory empty holders");
    Ok(())
}

fn compare_damage(
    frame: &DecodedFrame,
    save: &Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut damages = unique_checked(complete::<tables::AirRegionDamage>(frame)?.iter().map(
        |row| {
            Ok((
                (
                    required_country_tag(row.country, links)?.to_vec(),
                    row.region,
                ),
                row,
            ))
        },
    ))?;
    let expected = unique(
        save.regions
            .iter()
            .map(|row| ((row.country.clone(), row.region), row)),
        "saved air combat region",
    )?;
    let mut damage_matched = 0;
    for (key, expected) in expected {
        let label = format!("air region {}:{}", String::from_utf8_lossy(&key.0), key.1);
        let mut seen = BTreeSet::new();
        if expected
            .entries
            .iter()
            .any(|entry| !seen.insert(record_key(entry)))
        {
            report.limitation("duplicate native record identities retain their array order, but damage entries lack a parent-record ordinal and cannot certify assignment between duplicate parents", &label);
        }
        let expected_damage = expected
            .entries
            .iter()
            .flat_map(|record| {
                record
                    .damage
                    .iter()
                    .enumerate()
                    .flat_map(move |(channel, entries)| {
                        entries
                            .iter()
                            .map(move |entry| (record, channel as u8, entry))
                    })
            })
            .collect::<Vec<_>>();
        if let Some(actual) = damages.remove(&key) {
            check(
                report,
                &format!("{label}.damage.count"),
                actual.entries.len(),
                expected_damage.len(),
            );
            for (ordinal, (actual, (record, channel, entry))) in
                actual.entries.iter().zip(expected_damage).enumerate()
            {
                damage_matched += 1;
                let label = format!("{label}.damage[{ordinal}]");
                check(
                    report,
                    &format!("{label}.record"),
                    (
                        actual.side,
                        actual.date,
                        actual.wing_id,
                        actual.classification,
                    ),
                    record_key(record),
                );
                check(report, &format!("{label}.channel"), actual.channel, channel);
                check(report, &format!("{label}.cause"), actual.cause, entry.cause);
                report.scalar(&label, "amount", Some(actual.amount), Some(&entry.amount));
            }
        } else if !expected_damage.is_empty() {
            missing(report, format!("{label}.damage"));
        }
    }
    extra(report, "air region damage", &damages);
    report.compared("AirRegionDamage", damage_matched);
    report.limitation("grouped damage rows originate from emitted entries; allocation of empty native region holders is not certified", "AirRegionDamage empty holders");
    Ok(())
}

fn compare_statistics(
    frame: &DecodedFrame,
    save: &Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = unique_checked(complete::<tables::AirStatistics>(frame)?.iter().map(|row| {
        Ok((
            (
                required_country_tag(row.country, links)?.to_vec(),
                row.region,
            ),
            row,
        ))
    }))?;
    let expected = unique(
        save.statistics
            .iter()
            .map(|row| ((row.country.clone(), row.region), row)),
        "saved air statistics region",
    )?;
    let mut matched = 0;
    for (key, expected) in expected {
        let label = format!(
            "air statistics {}:{}",
            String::from_utf8_lossy(&key.0),
            key.1
        );
        let Some(actual) = actual.remove(&key) else {
            missing(report, &label);
            continue;
        };
        matched += 1;
        check(
            report,
            &format!("{label}.year_slot"),
            actual.year_slot,
            expected.year_slot,
        );
        check(
            report,
            &format!("{label}.year_window_wrapped"),
            actual.year_window_wrapped,
            u8::from(expected.year_window_wrapped),
        );
        for (index, (actual, expected)) in
            actual.channels.iter().zip(&expected.channels).enumerate()
        {
            check(
                report,
                &format!("{label}.channel[{}]", air::STATISTIC_CHANNELS[index]),
                actual,
                expected,
            );
        }
    }
    extra(report, "air statistics", &actual);
    report.compared("AirStatistics", matched);
    report.limitation("air statistics compare 24 native channels in each allocated country/region's current annual-ring bucket; it advances after twelve 30-day buckets, not calendar years or campaign lifetime; four previous annual buckets, daily/monthly windows and 65 other channels are outside captured scope", "AirStatistics");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::frame;
    use installed_content::parser;
    use replay::{
        frame::TableView,
        rows::{
            AirDamageEntry, AirHistoryEntry, AirRegionDamageRow, AirRegionHistoryRow,
            AirStatisticsRow, TableId, TableRows, WingCumulativeRow, WingEquipmentRow,
        },
    };

    fn parsed(text: &[u8]) -> Pack {
        air::parse(&parser::parse(text).unwrap().items).unwrap()
    }

    #[test]
    fn duplicate_parents_are_refused_instead_of_overwritten() {
        assert!(unique([(7, "first"), (7, "second")], "saved air region").is_err());
    }

    #[test]
    fn typed_wing_tables_compare_visible_order_and_refuse_incomplete_data() {
        let save = parsed(br#"strategic_air={GER={air_wing_pool={definition=fixture_plane air_wings={id={id=1 type=69} experience=-0.00001 equipment={equipment={id={id=4294967295 type=4294967295} amount=-92233720368547.75808} equipment={id={id=4294967295 type=4294967295} amount=7} allow_zero_entries=no}}}}}"#);
        let wing = (1_u64 << 32) | 69;
        let mut frame = frame([
            TableRows::WingCumulative(vec![WingCumulativeRow {
                id: wing,
                accidents: -3,
                experience: -1,
            }]),
            TableRows::WingEquipment(vec![
                WingEquipmentRow {
                    wing,
                    ordinal: 0,
                    variant: u64::MAX,
                    amount_raw: 0,
                },
                WingEquipmentRow {
                    wing,
                    ordinal: 1,
                    variant: u64::MAX,
                    amount_raw: i64::MIN,
                },
                WingEquipmentRow {
                    wing,
                    ordinal: 2,
                    variant: u64::MAX,
                    amount_raw: 700_000,
                },
            ]),
        ]);
        let mut report = Report::default();
        compare_cumulative(&frame, &save, &mut report).unwrap();
        compare_equipment(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("nonpositive accident"))
        );

        let TableView::Complete(TableRows::WingEquipment(rows)) =
            &mut *frame.tables[TableId::WingEquipment.index()]
        else {
            unreachable!()
        };
        rows[2].amount_raw = 1;
        let mut report = Report::default();
        compare_equipment(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        *frame.tables[TableId::WingEquipment.index()] = TableView::Unavailable {
            reason: "guarded copy failed".into(),
        };
        assert!(compare_equipment(&frame, &save, &mut Report::default()).is_err());
    }

    #[test]
    fn typed_region_tables_preserve_duplicates_country_aliases_and_damage_order() {
        let mut save = parsed(br#"strategic_air={GER={combat_history={0={tag="GER" friend={id=0 equipment={allow_zero_entries=no} time=-4 mission=-1 count=9 tag="---" ground_attack=no destination=yes receiver={type=-1 value=-0.00001}} friend={id=0 equipment={allow_zero_entries=no} time=-4 mission=-1 count=11 tag="---" ground_attack=no destination=yes receiver={type=-1 value=2}}}}}}"#);
        let germany = CountryRow {
            id: 12,
            canonical_slot: 3,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany), (112, &germany)]);
        let entry = AirHistoryEntry {
            side: 0,
            date: -4,
            wing_id: 0,
            classification: 0,
            region_owner: 112,
            planes: 9,
            mission_mask: u32::MAX,
            selection: 1,
            copied: 0,
            attributed_country: 0,
        };
        let damage = AirDamageEntry {
            side: 0,
            date: -4,
            wing_id: 0,
            classification: 0,
            channel: 1,
            cause: u32::MAX,
            amount: -1,
        };
        let mut channels = [0; 24];
        channels[23] = i64::MIN;
        save.statistics.push(air::Statistics {
            country: b"GER".to_vec(),
            region: 0,
            year_slot: 4,
            year_window_wrapped: true,
            channels,
        });
        let mut frame = frame([
            TableRows::AirRegionHistory(vec![AirRegionHistoryRow {
                country: 112,
                region: 0,
                entries: vec![
                    entry.clone(),
                    AirHistoryEntry {
                        planes: 11,
                        ..entry
                    },
                ],
            }]),
            TableRows::AirRegionDamage(vec![AirRegionDamageRow {
                country: 112,
                region: 0,
                entries: vec![
                    damage.clone(),
                    AirDamageEntry {
                        amount: 200_000,
                        ..damage
                    },
                ],
            }]),
            TableRows::AirStatistics(vec![AirStatisticsRow {
                country: 112,
                region: 0,
                year_slot: 4,
                year_window_wrapped: 1,
                channels,
            }]),
        ]);
        let mut report = Report::default();
        compare_history(&frame, &save, &links, &mut report).unwrap();
        compare_damage(&frame, &save, &links, &mut report).unwrap();
        compare_statistics(&frame, &save, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("duplicate parents"))
        );
        let TableView::Complete(TableRows::AirRegionDamage(rows)) =
            &mut *frame.tables[TableId::AirRegionDamage.index()]
        else {
            unreachable!()
        };
        rows[0].entries.swap(0, 1);
        let mut report = Report::default();
        compare_damage(&frame, &save, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
    }

    #[test]
    fn pack_reports_every_independent_table_after_failures_and_stays_strict() {
        let germany = CountryRow {
            id: 12,
            canonical_slot: 3,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany), (112, &germany)]);
        let mut channels = [0; 24];
        channels[0] = 10;
        let save = Pack {
            statistics: vec![air::Statistics {
                country: b"GER".to_vec(),
                region: 23,
                year_slot: 4,
                year_window_wrapped: true,
                channels,
            }],
            ..Default::default()
        };
        channels[0] = 11;
        let mut frame = frame([TableRows::AirStatistics(vec![AirStatisticsRow {
            country: 112,
            region: 23,
            year_slot: 4,
            year_window_wrapped: 1,
            channels,
        }])]);
        for table in [TableId::WingCumulative, TableId::AirRegionHistory] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: "guarded copy failed".into(),
            };
        }
        let mut report = Report::default();
        assert!(compare(&frame, &save, &links, &mut report).is_err());
        assert_eq!(report.failed_comparisons.len(), 2);
        assert!(report.failed_comparisons.contains_key("air WingCumulative"));
        assert!(
            report
                .failed_comparisons
                .contains_key("air AirRegionHistory")
        );
        assert_eq!(report.compared_tables.len(), 5);
        assert!(report.compared_tables.contains_key("WingEquipment"));
        assert!(report.compared_tables.contains_key("AirRegionDamage"));
        assert!(report.compared_tables.contains_key("AirStatistics"));
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("air statistics GER:23.channel[0]"));
    }
    #[test]
    fn ace_oracle_checks_selected_definition_and_saved_portrait_operands() {
        use replay::rows::{AirAcePresentation, AirAceRow, Recorded};
        let save = parsed(br#"countries={MOD={ace={id={id=1 type=4} name="N" surname="S" modifier="authored_type" is_female=yes portrait=-7}}}"#);
        let country = CountryRow {
            id: 9,
            tag: b"MOD".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(9, &country)]);
        let row = AirAceRow {
            id: (1_u64 << 32) | 4,
            country: 9,
            alive: 1,
            name: b"N".to_vec(),
            surname: b"S".to_vec(),
            presentation: Recorded::Value(Some(AirAcePresentation {
                gender: 7,
                pool_index: -7,
                definition: Some(b"authored_type".to_vec()),
                ..Default::default()
            })),
            ..Default::default()
        };
        let sample = frame([TableRows::AirAces(vec![row.clone()])]);
        let mut report = Report::default();
        compare_aces(&sample, &save, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        let mut changed = row;
        changed.presentation = Recorded::Value(Some(AirAcePresentation::default()));
        let sample = frame([TableRows::AirAces(vec![changed])]);
        let mut report = Report::default();
        compare_aces(&sample, &save, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3);
        assert!(
            report
                .differences
                .iter()
                .any(|value| value.contains(".modifier"))
        );
        assert!(
            report
                .differences
                .iter()
                .any(|value| value.contains(".is_female"))
        );
        assert!(
            report
                .differences
                .iter()
                .any(|value| value.contains(".portrait"))
        );
    }
}
