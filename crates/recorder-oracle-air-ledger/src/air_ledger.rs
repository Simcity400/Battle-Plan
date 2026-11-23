use super::{ContentRead, Report, complete, country_context, invalid};
use engine_air_history::air_loss_sites::FIELD_COUNT;
use content_reader_save::air::Pack;
use replay::campaign::Replay;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

fn event_hour(frame: &DecodedFrame, game_hour: i32, label: &str) -> io::Result<()> {
    if game_hour > frame.hour {
        return Err(invalid(format!(
            "{label} event hour {game_hour} is after its supplying read"
        )));
    }
    Ok(())
}

const ORACLE_TABLES: [TableId; 4] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::AirLossEvents,
    TableId::AirDailyRoster,
];

fn loss_only_ordinal(index: usize) -> bool {
    !matches!(index, 0|1|2|36..=39|41..=43|77..=80)
}

pub fn compare_air_losses(
    replay: &Replay,
    frame: &DecodedFrame,
    save: &Pack,
    regions: ContentRead<'_, content_reader_regions::regions::StrategicRegions>,
    report: &mut Report,
) -> io::Result<()> {
    let regions = regions.map_err(|error| {
        invalid(format!(
            "air ledger strategic-region content refused: {error}"
        ))
    })?;
    let mut check = DailyCheck::new(frame.hour)?;
    check.event_losses = replay.event_losses(frame.branch).collect();
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| {
            block.branch == frame.branch
                && block.first_hour <= frame.hour
                && block.last_hour >= check.start
        })
        .collect();
    blocks.sort_by_key(|block| block.first_hour);
    for block in blocks {
        replay.visit_blocks(
            &[block],
            &ORACLE_TABLES,
            |_, _, hour_frame| {
            if hour_frame.hour > frame.hour {
                return Ok(false);
            }
            let decoded = Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?;
            if decoded.hour >= check.start {
                check.observe(decoded, regions)?;
            }
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    check.finish(save, report)
}

struct DailyCheck {
    start: i32,
    midnight: i32,
    end: i32,
    next: i64,
    roster: BTreeMap<Vec<u8>, bool>,
    seen: BTreeSet<(i32, Vec<u8>)>,
    ids: BTreeSet<u64>,
    sums: BTreeMap<(Vec<u8>, u32, u32), [i64; FIELD_COUNT]>,
    event_losses: Vec<(i32, i32)>,
}
impl DailyCheck {
    fn new(end: i32) -> io::Result<Self> {
        let midnight = end
            .checked_sub(end.rem_euclid(24))
            .ok_or_else(|| invalid("air daily oracle midnight underflow"))?;
        let start = midnight
            .checked_sub(24)
            .ok_or_else(|| invalid("air daily oracle hour underflow"))?;
        Ok(Self {
            start,
            midnight,
            end,
            next: i64::from(start),
            roster: BTreeMap::new(),
            seen: BTreeSet::new(),
            ids: BTreeSet::new(),
            sums: BTreeMap::new(),
            event_losses: Vec::new(),
        })
    }
    fn observe(
        &mut self,
        frame: &DecodedFrame,
        regions: &content_reader_regions::regions::StrategicRegions,
    ) -> io::Result<()> {
        if i64::from(frame.hour) != self.next {
            return Err(invalid("air daily oracle missing or repeated hour"));
        }
        self.next += 1;
        let countries = country_context(frame)?;
        if frame.hour == self.start || frame.hour == self.midnight {
            let roster = complete::<tables::AirDailyRoster>(frame)?;
            let mut witnessed = BTreeSet::new();
            for row in roster {
                let country = countries
                    .links
                    .get(&row.country)
                    .ok_or_else(|| invalid("air daily roster country has no CountryLink"))?;
                let tag = country.tag.clone();
                if !witnessed.insert(tag.clone()) {
                    return Err(invalid("air daily roster country repeated"));
                }
                self.seen.insert((frame.hour, tag.clone()));
                self.roster
                    .entry(tag)
                    .and_modify(|eligible| *eligible &= row.eligible)
                    .or_insert(row.eligible);
            }
            for country in complete::<tables::Countries>(frame)? {
                if !witnessed.contains(&country.tag) {
                    return Err(invalid("air daily roster lacks country witness"));
                }
            }
        }
        if let Some(&(after, lost)) = self
            .event_losses
            .iter()
            .find(|&&(_, lost)| lost == frame.hour)
        {
            return Err(invalid(format!(
                "the air day window includes a recorded event loss between the reads of hours {after} and {lost}: the recorder was detached for part of it, so its air losses are incomplete"
            )));
        }
        for row in complete::<tables::AirLossEvents>(frame)? {
            event_hour(frame, row.game_hour, "AirLossEvents")?;
            if !self.ids.insert(row.id) {
                return Err(invalid("AirLossEvents has a duplicate event id"));
            }
            if !regions.regions.contains_key(&row.region) {
                return Err(invalid("AirLossEvents region absent from verified content"));
            }
            let country = countries
                .links
                .get(&row.country)
                .ok_or_else(|| invalid("AirLossEvents country has no CountryLink"))?;
            if row
                .opponent
                .is_some_and(|tag| !countries.links.contains_key(&tag))
            {
                return Err(invalid("AirLossEvents opponent has no CountryLink"));
            }
            if row.game_hour < self.start {
                continue;
            }
            let sums = self
                .sums
                .entry((country.tag.clone(), row.region, row.daily_slot))
                .or_insert([0; FIELD_COUNT]);
            for index in [row.family_base, row.family_base + row.native_cause] {
                sums[index as usize] = sums[index as usize]
                    .checked_add(row.amount_raw)
                    .ok_or_else(|| invalid("air daily oracle event sum overflow"))?;
            }
        }
        Ok(())
    }
    fn finish(mut self, save: &Pack, report: &mut Report) -> io::Result<()> {
        if !save.daily_errors.is_empty() {
            return Err(invalid(format!(
                "AirDailySaveMalformed: {}",
                save.daily_errors.join("; ")
            )));
        }
        if self.next != i64::from(self.end) + 1 {
            return Err(invalid(
                "air daily oracle lacks complete two-midnight coverage",
            ));
        }
        let data_tags: BTreeSet<_> = self
            .sums
            .keys()
            .map(|(tag, _, _)| tag.clone())
            .chain(save.daily_statistics.iter().map(|row| row.country.clone()))
            .collect();
        for tag in &data_tags {
            self.roster.entry(tag.clone()).or_insert(false);
        }
        for (tag, eligible) in &mut self.roster {
            *eligible &= self.seen.contains(&(self.start, tag.clone()))
                && self.seen.contains(&(self.midnight, tag.clone()));
            if !*eligible && data_tags.contains(tag) {
                report.limitation("AirDailyCountryOutsideRoster: the country did not advance at both recorded midnights; the saved slots may include events before the covered window",
                    String::from_utf8_lossy(tag).into_owned());
            }
        }
        let mut saved_keys = BTreeSet::new();
        let mut compared = 0;
        for row in &save.daily_statistics {
            if !saved_keys.insert((row.country.clone(), row.region)) {
                return Err(invalid("air daily save has duplicate country/region"));
            }
            if self.roster.get(&row.country) != Some(&true) {
                continue;
            }
            let capacity = engine_air_history::cloop_history::DAILY_WINDOW_CAPACITY;
            let previous_slot = (row.current_slot + capacity - 1) % capacity;
            for (current, slot, fields) in [
                (false, previous_slot, &row.previous_fields),
                (true, row.current_slot, &row.current_fields),
            ] {
                let sums = self
                    .sums
                    .remove(&(row.country.clone(), row.region, slot))
                    .unwrap_or([0; FIELD_COUNT]);
                for (index, (sum, saved)) in sums.iter().zip(fields).enumerate() {
                    if !loss_only_ordinal(index) {
                        continue;
                    }
                    if sum != saved {
                        report.difference(
                            format!(
                                "AirLossEvents {} region {} {} slot field {index}",
                                String::from_utf8_lossy(&row.country),
                                row.region,
                                if current { "current" } else { "previous" }
                            ),
                            *sum,
                            *saved,
                        );
                    }
                    compared += 1;
                }
            }
        }
        for row in &save.statistics {
            if self.roster.get(&row.country) == Some(&true)
                && !saved_keys.contains(&(row.country.clone(), row.region))
            {
                report.difference(
                    format!(
                        "AirLossEvents {} region {} daily window",
                        String::from_utf8_lossy(&row.country),
                        row.region
                    ),
                    "native history present",
                    "saved daily window absent",
                );
            }
        }
        for ((tag, region, slot), sums) in self.sums {
            if self.roster.get(&tag) != Some(&true) {
                continue;
            }
            if saved_keys.contains(&(tag.clone(), region)) {
                continue;
            }
            for (index, amount) in sums.into_iter().enumerate() {
                if amount != 0 && loss_only_ordinal(index) {
                    report.difference(
                        format!(
                            "AirLossEvents {} region {region} daily slot {slot} field {index}",
                            String::from_utf8_lossy(&tag)
                        ),
                        amount,
                        "saved queue absent",
                    );
                }
            }
        }
        report.compared("AirLossEvents", compared);
        Ok(())
    }
}

pub fn compare_bombing_damage(
    frame: &DecodedFrame,
    buildings: ContentRead<'_, content_reader_key_names::buildings::Buildings>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::BuildingBombingDamage>(frame)?;
    let countries = country_context(frame)?;
    let buildings = buildings
        .map_err(|error| invalid(format!("air ledger building content refused: {error}")))?;
    let mut tokens = std::collections::BTreeMap::new();
    for row in complete::<tables::LexicalTokens>(frame)? {
        if tokens.insert(row.token, row.literal.as_slice()).is_some() {
            return Err(invalid("air ledger has an ambiguous lexical token"));
        }
    }
    let building_names: BTreeSet<_> = buildings
        .types
        .keys()
        .map(|name| name.as_bytes().to_ascii_lowercase())
        .collect();
    let states: BTreeSet<_> = complete::<tables::States>(frame)?
        .iter()
        .map(|row| row.id)
        .collect();
    let provinces: BTreeSet<_> = complete::<tables::Provinces>(frame)?
        .iter()
        .map(|row| row.id)
        .collect();
    let mut ids = BTreeSet::new();
    let mut null_victims = 0;
    for row in rows {
        if !ids.insert(row.id) {
            return Err(invalid("BuildingBombingDamage has a duplicate event id"));
        }
        event_hour(frame, row.game_hour, "BuildingBombingDamage")?;
        if row.amount_raw == 0 {
            return Err(invalid(
                "BuildingBombingDamage contains a zero applied health change",
            ));
        }
        null_victims += usize::from(row.victim == 0);
        for country in row
            .attacker
            .into_iter()
            .chain((row.victim != 0).then_some(row.victim))
        {
            if !countries.links.contains_key(&country) {
                return Err(invalid(format!(
                    "BuildingBombingDamage country {country} has no CountryLink"
                )));
            }
        }
        let name = tokens.get(&row.building_type).ok_or_else(|| {
            invalid(format!(
                "BuildingBombingDamage type token {} has no LexicalToken",
                row.building_type
            ))
        })?;
        if !building_names.contains(&name.to_ascii_lowercase()) {
            return Err(invalid(format!(
                "BuildingBombingDamage type {} is absent from verified building content",
                String::from_utf8_lossy(name)
            )));
        }
        let state_known = if row.state == 0 {
            row.province.is_some()
        } else {
            states.contains(&row.state)
        };
        if !state_known {
            return Err(invalid(format!(
                "BuildingBombingDamage state {} has no State row",
                row.state
            )));
        }
        if let Some(province) = row.province
            && !provinces.contains(&province)
        {
            return Err(invalid(format!(
                "BuildingBombingDamage province {province} has no Province row"
            )));
        }
    }
    report.compared("BuildingBombingDamage", rows.len());
    if null_victims != 0 {
        report.limitation(
            &format!("BuildingBombingNullVictim: {null_victims} applied damage events retain native victim 0, the reserved null country. Their amounts remain recorded without inventing a credited victim country"),
            "BuildingBombingDamage.victim",
        );
    }
    report.limitation(
        "BuildingBombingEventHistoryNotSaved: the building save writer serializes current total level, healthy level and partial health. The existing StateBuildings / ProvinceBuildings comparisons certify those saved quantities independently. It writes no damage event, attacker, native cause, applied health drop or credited return value. Repair, construction, scripts and other damage prevent health deltas from certifying bombing events; no health difference is waived",
        "BuildingBombingDamage.amount_raw / credited_raw",
    );
    report.limitation(
        "BuildingBombingHookCoverage: recording self-check only (positive applied health drops, event ids/hours, country/state/province joins and building-type tokens resolved through LexicalTokens and verified content). Null attackers and unknown causes remain explicit. Effective-country attribution requires coordinated live hook evidence; a passing self-check does not establish event completeness or zero lost events",
        "BuildingBombingDamage.attacker / victim / building_type / cause",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::frame;
    use content_reader_save::air::DailyStatistics;
    use replay_data::{
        rows::AirLossEventRow, rows::BuildingBombingDamageCause, rows::BuildingBombingDamageRow,
        rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow, rows::ProvinceRow,
        rows::StateRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;

    fn loss() -> AirLossEventRow {
        AirLossEventRow {
            id: 1,
            game_hour: 36,
            country: 4999,
            region: 4001,
            native_cause: 1,
            family_base: 30,
            credited_enemy: false,
            amount_raw: 100000,
            opponent: None,
            daily_slot: 29,
        }
    }

    fn bombing() -> BuildingBombingDamageRow {
        BuildingBombingDamageRow {
            id: 0,
            game_hour: 7,
            attacker: None,
            victim: 4999,
            state: 4001,
            province: Some(8001),
            building_type: 60001,
            native_cause: u32::MAX,
            cause: BuildingBombingDamageCause::Unknown,
            amount_raw: i64::MAX,
            credited_raw: i64::MIN,
        }
    }

    fn fixture(
        losses: Vec<AirLossEventRow>,
        damage: Vec<BuildingBombingDamageRow>,
    ) -> DecodedFrame {
        frame([
            TableRows::Countries(vec![CountryRow {
                id: 4999,
                canonical_slot: 4999,
                tag: b"mod country".to_vec(),
                ..Default::default()
            }]),
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 4999,
                canonical_slot: 4999,
            }]),
            TableRows::States(vec![StateRow {
                id: 4001,
                ..Default::default()
            }]),
            TableRows::Provinces(vec![ProvinceRow {
                id: 8001,
                state: 4001,
                ..Default::default()
            }]),
            TableRows::AirLossEvents(losses),
            TableRows::BuildingBombingDamage(damage),
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 60001,
                literal: b"MOD_AIR_LEDGER_BUILDING".to_vec(),
            }]),
        ])
    }

    fn region_content() -> content_reader_regions::regions::StrategicRegions {
        let mut regions = content_reader_regions::regions::StrategicRegions::default();
        regions.regions.insert(
            4001,
            content_reader_regions::regions::StrategicRegion {
                id: 4001,
                ..Default::default()
            },
        );
        regions
    }

    fn building_content() -> content_reader_key_names::buildings::Buildings {
        let mut buildings = content_reader_key_names::buildings::Buildings::default();
        buildings.types.insert(
            "mod_air_ledger_building".into(),
            content_reader_key_names::buildings::BuildingType {
                name: "mod_air_ledger_building".into(),
                ..Default::default()
            },
        );
        buildings
    }

    fn check_bombing(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
        compare_bombing_damage(frame, Ok(&building_content()), report)
    }

    fn daily_check(missing: bool, eligible: bool) -> DailyCheck {
        let mut events = BTreeMap::new();
        if !missing {
            events.insert(37, vec![loss()]);
        }
        events.insert(
            51,
            vec![AirLossEventRow {
                id: 2,
                game_hour: 50,
                daily_slot: 0,
                ..loss()
            }],
        );
        daily_events(events, eligible)
    }
    fn daily_events(mut events: BTreeMap<i32, Vec<AirLossEventRow>>, eligible: bool) -> DailyCheck {
        let mut check = DailyCheck::new(53).unwrap();
        let regions = region_content();
        for hour in 24..=53 {
            let losses = events.remove(&hour).unwrap_or_default();
            let mut frame = fixture(losses, vec![]);
            frame.hour = hour;
            *frame.tables[TableId::AirDailyRoster.index()] =
                TableView::Complete(TableRows::AirDailyRoster(vec![
                    replay_data::rows::AirDailyRosterRow {
                        country: 4999,
                        eligible,
                    },
                ]));
            check.observe(&frame, &regions).unwrap();
        }
        check
    }
    fn daily_save() -> Pack {
        let mut fields = [0; FIELD_COUNT];
        fields[30] = 100000;
        fields[31] = 100000;
        Pack {
            daily_statistics: vec![DailyStatistics {
                country: b"mod country".to_vec(),
                region: 4001,
                current_slot: 0,
                wrapped: true,
                previous_fields: fields,
                current_fields: fields,
            }],
            ..Default::default()
        }
    }
    #[test]
    fn daily_current_previous_sums_match_and_missing_event_is_a_difference() {
        let mut report = Report::default();
        daily_check(false, true)
            .finish(&daily_save(), &mut report)
            .unwrap();
        assert!(report.differences.is_empty());
        assert!(report.limitations.is_empty());
        let mut report = Report::default();
        daily_check(true, true)
            .finish(&daily_save(), &mut report)
            .unwrap();
        assert_eq!(report.differences.len(), 2);
        assert!(
            report
                .differences
                .iter()
                .all(|difference| difference.contains("previous"))
        );
    }
    #[test]
    fn daily_country_outside_roster_gets_only_the_named_limitation() {
        let mut report = Report::default();
        daily_check(false, false)
            .finish(&daily_save(), &mut report)
            .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .all(|reason| reason.starts_with("AirDailyCountryOutsideRoster:"))
        );
        assert_eq!(report.limitations.len(), 1);
    }
    #[test]
    fn daily_slots_bucket_writes_on_both_midnight_sides_exactly() {
        let events = BTreeMap::from([
            (
                24,
                vec![
                    AirLossEventRow {
                        id: 3,
                        game_hour: 24,
                        daily_slot: 28,
                        ..loss()
                    },
                    AirLossEventRow {
                        id: 4,
                        game_hour: 24,
                        daily_slot: 29,
                        ..loss()
                    },
                ],
            ),
            (
                49,
                vec![
                    AirLossEventRow {
                        id: 5,
                        game_hour: 48,
                        daily_slot: 29,
                        ..loss()
                    },
                    AirLossEventRow {
                        id: 6,
                        game_hour: 48,
                        daily_slot: 0,
                        ..loss()
                    },
                ],
            ),
        ]);
        let mut save = daily_save();
        save.daily_statistics[0].previous_fields[30] = 200000;
        save.daily_statistics[0].previous_fields[31] = 200000;
        let mut report = Report::default();
        daily_events(events, true)
            .finish(&save, &mut report)
            .unwrap();
        assert!(report.differences.is_empty());
        assert!(report.limitations.is_empty());
    }
    #[test]
    fn daily_credited_missile_total_is_compared() {
        let mut save = daily_save();
        save.daily_statistics[0].previous_fields[81] = 123;
        let mut report = Report::default();
        daily_check(false, true).finish(&save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("previous slot field 81"));
    }
    #[test]
    fn daily_dormant_country_without_data_has_no_roster_limitation() {
        let mut check = daily_check(false, true);
        check.roster.insert(b"dormant mod country".to_vec(), false);
        let mut report = Report::default();
        check.finish(&daily_save(), &mut report).unwrap();
        assert!(report.limitations.is_empty());
    }
    #[test]
    fn daily_other_writer_fields_including_enemy_damage_are_not_loss_oracles() {
        let mut save = daily_save();
        for index in [0, 1, 2, 36, 37, 38, 39, 41, 42, 43, 77, 78, 79, 80] {
            save.daily_statistics[0].previous_fields[index] = 123;
            save.daily_statistics[0].current_fields[index] = -456;
        }
        let mut report = Report::default();
        daily_check(false, true).finish(&save, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(report.limitations.is_empty());
    }
    #[test]
    fn daily_missing_saved_window_is_not_zero_for_an_existing_history() {
        let mut save = daily_save();
        save.daily_statistics.clear();
        save.statistics.push(content_reader_save::air::Statistics {
            country: b"mod country".to_vec(),
            region: 4001,
            year_slot: 0,
            year_window_wrapped: false,
            channels: [0; 24],
        });
        let mut report = Report::default();
        daily_check(true, true).finish(&save, &mut report).unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("saved daily window absent"))
        );
    }
    #[test]
    fn malformed_daily_save_fails_only_air_group() {
        let text = br#"strategic_air={"mod country"={history={4001={history_queue={0={max_elements=30 offset=30 is_full=no} 2={max_elements=5 offset=1 is_full=no}}}}}}"#;
        let doc = content_reader_parser::parser::parse(text).unwrap();
        let save = content_reader_save::air::parse(&doc.items).unwrap();
        assert_eq!(save.daily_errors.len(), 1);
        assert_eq!(save.statistics.len(), 1);
        let mut report = Report::default();
        assert!(!report.attempt_table(TableId::AirLossEvents, |report| {
            daily_check(false, true).finish(&save, report)
        }));
        assert!(report.attempt("unrelated save group", |report| {
            report.compared("unrelated", 1);
            Ok(())
        }));
        assert_eq!(report.failed_comparisons.len(), 1);
        assert!(report.failed_comparisons["AirLossEvents"].contains("AirDailySaveMalformed"));
        assert_eq!(report.compared_tables["unrelated"], 1);
    }
    #[test]
    fn daily_missing_roster_hour_and_unavailable_event_interval_refuse() {
        let regions = region_content();
        let mut frame = fixture(vec![], vec![]);
        frame.hour = 24;
        let mut check = DailyCheck::new(53).unwrap();
        assert!(check.observe(&frame, &regions).is_err());
        *frame.tables[TableId::AirDailyRoster.index()] =
            TableView::Complete(TableRows::AirDailyRoster(vec![
                replay_data::rows::AirDailyRosterRow {
                    country: 4999,
                    eligible: true,
                },
            ]));
        let mut check = DailyCheck::new(53).unwrap();
        check.observe(&frame, &regions).unwrap();
        frame.hour = 25;
        *frame.tables[TableId::AirLossEvents.index()] = TableView::Unavailable {
            reason: "lost".into(),
        };
        assert!(check.observe(&frame, &regions).is_err());
        let mut check = DailyCheck::new(53).unwrap();
        frame.hour = 26;
        assert!(check.observe(&frame, &regions).is_err());
    }

    #[test]
    fn a_recorded_event_loss_in_the_window_is_named_and_never_compared() {
        let regions = region_content();
        let mut frame = fixture(vec![], vec![]);
        *frame.tables[TableId::AirDailyRoster.index()] =
            TableView::Complete(TableRows::AirDailyRoster(vec![
                replay_data::rows::AirDailyRosterRow {
                    country: 4999,
                    eligible: true,
                },
            ]));
        let mut check = DailyCheck::new(53).unwrap();
        check.event_losses = vec![(24, 25)];
        frame.hour = 24;
        check.observe(&frame, &regions).unwrap();
        frame.hour = 25;
        let error = check.observe(&frame, &regions).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("recorded event loss between the reads of hours 24 and 25")
        );
    }

    #[test]
    fn bombing_unknown_and_no_wing_events_remain_visible_with_health_history_limitation() {
        for cause in [
            BuildingBombingDamageCause::Unknown,
            BuildingBombingDamageCause::Scripted,
        ] {
            let frame = fixture(
                vec![],
                vec![BuildingBombingDamageRow { cause, ..bombing() }],
            );
            let mut report = Report::default();
            check_bombing(&frame, &mut report).unwrap();
            assert!(report.differences.is_empty());
            assert_eq!(report.compared_tables["BuildingBombingDamage"], 1);
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("BuildingBombingEventHistoryNotSaved"))
            );
        }
    }

    #[test]
    fn bombing_bad_joins_hours_amounts_duplicates_and_unavailability_refuse() {
        for row in [
            BuildingBombingDamageRow {
                attacker: Some(5000),
                ..bombing()
            },
            BuildingBombingDamageRow {
                victim: 5000,
                ..bombing()
            },
            BuildingBombingDamageRow {
                state: 4002,
                ..bombing()
            },
            BuildingBombingDamageRow {
                province: Some(8002),
                ..bombing()
            },
            BuildingBombingDamageRow {
                building_type: 60002,
                ..bombing()
            },
            BuildingBombingDamageRow {
                game_hour: 8,
                ..bombing()
            },
            BuildingBombingDamageRow {
                amount_raw: 0,
                ..bombing()
            },
        ] {
            assert!(check_bombing(&fixture(vec![], vec![row]), &mut Report::default()).is_err());
        }
        let mut frame = fixture(vec![], vec![bombing(), bombing()]);
        assert!(check_bombing(&frame, &mut Report::default()).is_err());
        *frame.tables[TableId::BuildingBombingDamage.index()] = TableView::Unavailable {
            reason: "overflow".into(),
        };
        assert!(check_bombing(&frame, &mut Report::default()).is_err());
    }

    #[test]
    fn late_air_events_and_signed_bombing_corrections_keep_their_native_dates() {
        let frame = fixture(
            vec![],
            vec![BuildingBombingDamageRow {
                game_hour: 1,
                amount_raw: -1,
                ..bombing()
            }],
        );
        event_hour(&frame, 1, "AirLossEvents").unwrap();
        check_bombing(&frame, &mut Report::default()).unwrap();
        assert!(event_hour(&frame, frame.hour + 1, "AirLossEvents").is_err());
    }

    #[test]
    fn stateless_province_damage_preserves_native_state_absence() {
        let mut frame = fixture(
            vec![],
            vec![BuildingBombingDamageRow {
                state: 0,
                ..bombing()
            }],
        );
        *frame.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                id: 8001,
                state: 0,
                ..Default::default()
            }]));
        check_bombing(&frame, &mut Report::default()).unwrap();
        *frame.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 0,
                ..Default::default()
            }]));
        *frame.tables[TableId::BuildingBombingDamage.index()] =
            TableView::Complete(TableRows::BuildingBombingDamage(vec![
                BuildingBombingDamageRow {
                    state: 0,
                    province: None,
                    ..bombing()
                },
            ]));
        assert!(check_bombing(&frame, &mut Report::default()).is_err());
    }

    #[test]
    fn verified_content_refusals_and_unknown_building_literals_remain_failures() {
        let mut frame = fixture(vec![loss()], vec![bombing()]);
        assert!(
            compare_bombing_damage(
                &frame,
                Err("building form unsupported".into()),
                &mut Report::default()
            )
            .unwrap_err()
            .to_string()
            .contains("building content refused")
        );
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 60001,
                literal: b"removed_mod_building".to_vec(),
            }]));
        assert!(
            check_bombing(&frame, &mut Report::default())
                .unwrap_err()
                .to_string()
                .contains("absent from verified building content")
        );
    }

    #[test]
    fn null_bombing_victim_retains_damage_without_inventing_a_country() {
        let frame = fixture(
            vec![],
            vec![BuildingBombingDamageRow {
                victim: 0,
                ..bombing()
            }],
        );
        let mut report = Report::default();
        check_bombing(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["BuildingBombingDamage"], 1);
        assert!(report.limitations.keys().any(|reason| reason.contains("BuildingBombingNullVictim: 1 applied damage events")));
    }
}
