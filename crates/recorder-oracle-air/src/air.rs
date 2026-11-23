use std::{collections::BTreeMap, fmt::Debug, io};

use content_reader_save::air::{self, Pack};
use replay_data::{rows::CountryRow, rows::tables};
use replay_frame::frame::DecodedFrame;

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

pub fn compare(
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
    complete &= report.attempt("air AirStatistics", |report| {
        compare_statistics(frame, save, links, report)
    });
    complete &= report.attempt("air RaidAirScores", |report| {
        let rows = super::complete::<tables::RaidAirScores>(frame)?;
        for row in rows { required_country_tag(row.country, links)?; }
        report.limitation("the strategic air save writer omits the entire regional-score array; fresh saves cannot certify the stored superiority and spotting operands", "RaidAirScores.friendly / enemy / impact / air_spotting_scale");
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
    report.limitation("air base province is a cached pointer-derived location omitted by the air-base writer; a state base's host reference and dense registered-country order are also omitted. Country order is certified by the existing registration writers and needs a matched native base hover", "AirBases.province / state-base host / carrier.registered_countries");
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
                report.limitation("the air-wing writer omits every nonpositive accident count; the exact raw omitted value is not serialized", format!("{label}.accidents"));
            }
        } else {
            missing(report, format!("{label}.cumulative"));
        }
    }
    extra(report, "wing cumulative", &cumulative);
    report.compared("WingCumulative", matched);
    report.limitation("wing quantity differences are not evidence of combat losses; accidents and native annual statistics have distinct meanings", "WingCumulative / AirStatistics");
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
            let label = format!("{label}.{name}");
            let text = super::serialized_text(report, &label, actual);
            check(report, &label, text, expected.as_slice());
        }
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
            replay_data::rows::Recorded::Value(Some(fields)) => {
                check(report, &format!("{label}.modifier"), &fields.definition, &expected.modifier);
                check(report, &format!("{label}.is_female"), fields.gender != 0, expected.female);
                check(report, &format!("{label}.portrait"), fields.pool_index, expected.portrait);
            }
            replay_data::rows::Recorded::Value(None) => report.difference(
                format!("{label}.presentation"), "capture failed", "saved modifier / portrait operands"),
            replay_data::rows::Recorded::NotRecorded => report.limitation(
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
                    report.limitation("ace writer omits every nonpositive killer-country reference; exact omitted reference bits are not serialized", format!("{label}.killer_country"));
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
            report.limitation("ace writer does not serialize its reverse wing reference; an omitted wing ace field also permits an unresolved reference", format!("{label}.assigned_wing"));
        }
    }
    extra(report, "air ace", &actual);
    report.compared("AirAces", matched);
    report.limitation("ace aircraft-type mask is not compared: the saved modifier definition-to-mask mapping is not implemented/proven", "AirAces.aircraft_type_mask");
    report.limitation("the raw nonzero gender byte and the game interface's retained portrait pools/culture are not serialized; is_female compares their boolean meaning only, and artwork needs a matched-hour native portrait", "AirAces.presentation.portrait");
    report.limitation(
        "saved country tags cannot certify raw country-link alias bits",
        "extended air country references",
    );
    Ok(())
}

#[test]
fn ace_names_use_first_nul_and_callsign_stored_length() {
    let (mut frame, _, _, _) = super::fixtures::fixture();
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
        let doc = content_reader_parser::parser::parse(format!("countries={{AAA={{ace={{id={{id=1 type=4}} name=N surname=S portrait=0 {callsign}}}}}}}").as_bytes()).unwrap();
        let saved = air::parse(&doc.items).unwrap();
        *frame.tables[replay_data::rows::TableId::AirAces.index()] =
            replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::AirAces(vec![
                replay_data::rows::AirAceRow {
                    id: (1_u64 << 32) | 4,
                    country: 101,
                    alive: 1,
                    name: b"N\0tail".to_vec(),
                    surname: b"S\0tail".to_vec(),
                    callsign: stored.to_vec(),
                    presentation: replay_data::rows::Recorded::NotRecorded,
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

pub fn compare_region_combat(
    frame: &DecodedFrame,
    save: &Pack,
    links: &BTreeMap<u32, &CountryRow>,
    timeout: i32,
    report: &mut Report,
) -> io::Result<()> {
    use replay_data::rows::AirCombatRecord;
    use replay_derive::air_region_derive;
    let rows = complete::<tables::AirRegionCombat>(frame)?;
    let mut alive = std::collections::BTreeSet::new();
    for state in complete::<tables::States>(frame)? {
        if let Some(country) = links.get(&state.owner) {
            alive.insert(country.canonical_slot);
        }
    }
    let mut actual = unique_checked(rows.iter().map(|row| {
        Ok((
            (
                required_country_tag(row.country, links)?.to_vec(),
                row.region,
            ),
            row,
        ))
    }))?;
    let expected = unique(
        save.regions
            .iter()
            .map(|row| ((row.country.clone(), row.region), row)),
        "saved air combat region",
    )?;
    let mut matched = 0;
    for (key, expected) in expected {
        let label = format!("air region {}:{}", String::from_utf8_lossy(&key.0), key.1);
        let Some(row) = actual.remove(&key) else {
            if !expected.entries.is_empty() {
                missing(report, &label);
            }
            continue;
        };
        let pruned = links
            .get(&row.country)
            .is_some_and(|country| alive.contains(&country.canonical_slot));
        let (own, foreign): (&[AirCombatRecord], &[AirCombatRecord]) = if pruned {
            let state = air_region_derive::retained(row, frame.hour, timeout);
            (state.own, state.foreign)
        } else {
            report.limitation(air_region_derive::UNPRUNED_CONTAINER, &label);
            (&row.own[..], &row.foreign[..])
        };
        check(
            report,
            &format!("{label}.owner"),
            tag(row.owner, links)?,
            expected.owner.as_slice(),
        );
        let records = own
            .iter()
            .map(|record| (0_u8, record))
            .chain(foreign.iter().map(|record| (1_u8, record)));
        check(
            report,
            &format!("{label}.records"),
            own.len() + foreign.len(),
            expected.entries.len(),
        );
        for (ordinal, ((side, actual), entry)) in records.zip(&expected.entries).enumerate() {
            matched += 1;
            let label = format!("{label}[{ordinal}]");
            check(
                report,
                &format!("{label}.identity"),
                (side, actual.date, actual.wing_id, actual.classification),
                (
                    entry.side,
                    entry.date,
                    entry.wing_id,
                    u8::from(entry.classification),
                ),
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
            for (channel, (recorded, saved)) in [&actual.first_damage, &actual.second_damage]
                .into_iter()
                .zip(&entry.damage)
                .enumerate()
            {
                check(
                    report,
                    &format!("{label}.damage[{channel}].count"),
                    recorded.len(),
                    saved.len(),
                );
                for (index, (recorded, saved)) in recorded.iter().zip(saved).enumerate() {
                    let label = format!("{label}.damage[{channel}][{index}]");
                    check(
                        report,
                        &format!("{label}.cause"),
                        recorded.cause,
                        saved.cause,
                    );
                    report.scalar(&label, "amount", Some(recorded.amount), Some(&saved.amount));
                }
            }
        }
    }
    actual.retain(|_, row| {
        let pruned = links
            .get(&row.country)
            .is_some_and(|country| alive.contains(&country.canonical_slot));
        let state = air_region_derive::retained(row, frame.hour, timeout);
        !pruned || !state.own.is_empty() || !state.foreign.is_empty()
    });
    extra(report, "air region combat", &actual);
    report.compared("AirRegionCombat", matched);
    report.limitation("the copied equipment pools of a retained record are serialized but not recorded; their allow-zero byte is the recorded copied flag", "AirRegionCombat.equipment");
    report.limitation(
        "allocated holders without records have no row; the save's empty holders are not compared",
        "AirRegionCombat empty holders",
    );
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
    use content_reader_parser::parser;
    use replay_data::{
        rows::AirStatisticsRow, rows::TableId, rows::TableRows, rows::WingCumulativeRow,
        rows::WingEquipmentRow,
    };
    use replay_frame::frame::TableView;

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
    fn region_combat_compares_the_pruned_records_and_both_damage_channels() {
        use replay_data::{
            rows::AirCombatDamage, rows::AirCombatRecord, rows::AirRegionCombatRow, rows::StateRow,
        };
        let save = parsed(br#"strategic_air={GER={combat_history={4={tag="GER" friend={id=9 equipment={allow_zero_entries=yes} time=43800030 mission=5 count=3 tag="GER" ground_attack=yes destination=no sender={type=2 value=1.5} receiver={type=4 value=2}} enemy={id=10 equipment={allow_zero_entries=no} time=43800031 mission=1 count=2 tag="---" ground_attack=no destination=yes}}}}}"#);
        let germany = CountryRow {
            id: 12,
            canonical_slot: 3,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany), (112, &germany)]);
        let kept = AirCombatRecord {
            wing_id: 9,
            date: 43_800_030,
            planes: 3,
            mission_mask: 5,
            selection: 0,
            classification: 1,
            copied: 1,
            attributed_country: 12,
            first_damage: vec![AirCombatDamage {
                cause: 2,
                amount: 150_000,
            }],
            second_damage: vec![AirCombatDamage {
                cause: 4,
                amount: 200_000,
            }],
            equipment: replay_data::rows::Recorded::Value(None),
        };
        let stale = AirCombatRecord {
            date: 43_800_010,
            ..kept.clone()
        };
        let row = AirRegionCombatRow {
            country: 112,
            region: 4,
            owner: 12,
            own: std::sync::Arc::new(vec![stale, kept]),
            foreign: std::sync::Arc::new(vec![AirCombatRecord {
                wing_id: 10,
                date: 43_800_031,
                planes: 2,
                mission_mask: 1,
                selection: 1,
                ..Default::default()
            }]),
        };
        let state = StateRow {
            id: 1,
            owner: 12,
            controller: 12,
            ..Default::default()
        };
        let mut frame = frame([
            TableRows::AirRegionCombat(vec![row.clone()]),
            TableRows::States(vec![state]),
        ]);
        frame.hour = 43_800_000 + 24 * 2 + 5;
        let mut report = Report::default();
        compare_region_combat(&frame, &save, &links, 24, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let mut dead = crate::test_support::frame([TableRows::AirRegionCombat(vec![row])]);
        dead.hour = frame.hour;
        let mut report = Report::default();
        compare_region_combat(&dead, &save, &links, 24, &mut report).unwrap();
        assert!(!report.differences.is_empty());
        let TableView::Complete(TableRows::AirRegionCombat(rows)) =
            &mut *frame.tables[TableId::AirRegionCombat.index()]
        else {
            unreachable!()
        };
        std::sync::Arc::make_mut(&mut rows[0].own)[1].second_damage[0].amount += 1;
        let mut report = Report::default();
        compare_region_combat(&frame, &save, &links, 24, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("damage[1][0]"));
    }

    #[test]
    fn air_statistics_compare_through_country_aliases() {
        let mut save = Pack::default();
        let germany = CountryRow {
            id: 12,
            canonical_slot: 3,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany), (112, &germany)]);
        let mut channels = [0; 24];
        channels[23] = i64::MIN;
        save.statistics.push(air::Statistics {
            country: b"GER".to_vec(),
            region: 0,
            year_slot: 4,
            year_window_wrapped: true,
            channels,
        });
        let mut frame = frame([TableRows::AirStatistics(vec![AirStatisticsRow {
            country: 112,
            region: 0,
            year_slot: 4,
            year_window_wrapped: 1,
            channels,
        }])]);
        let mut report = Report::default();
        compare_statistics(&frame, &save, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let TableView::Complete(TableRows::AirStatistics(rows)) =
            &mut *frame.tables[TableId::AirStatistics.index()]
        else {
            unreachable!()
        };
        rows[0].year_slot = 3;
        let mut report = Report::default();
        compare_statistics(&frame, &save, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
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
        *frame.tables[TableId::WingCumulative.index()] = TableView::Unavailable {
            reason: "guarded copy failed".into(),
        };
        for table in [TableId::AirRegionHistory, TableId::AirRegionDamage] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: format!(
                    "{table:?} retired at format {0}; this recording is format {0}",
                    replay_data::FORMAT_VERSION
                ),
            };
        }
        let mut report = Report::default();
        assert!(compare(&frame, &save, &links, &mut report).is_err());
        assert_eq!(
            report.failed_comparisons.keys().collect::<Vec<_>>(),
            ["air WingCumulative"]
        );
        assert_eq!(
            report.compared_tables.keys().collect::<Vec<_>>(),
            ["AirAces", "AirBases", "AirStatistics", "WingEquipment"]
        );
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("air statistics GER:23.channel[0]"));
    }
    #[test]
    fn ace_oracle_checks_selected_definition_and_saved_portrait_operands() {
        use replay_data::{rows::AirAcePresentation, rows::AirAceRow, rows::Recorded};
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
