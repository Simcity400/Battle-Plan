use super::{Report, TokenNames, complete, invalid, required_country_tag};
use installed_content_save::Save;
use replay_data::rows::*;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) {
    report.attempt_table(TableId::PoliticsCaptureCoverage, |report| {
        coverage(frame, links, report)
    });
    report.attempt_table(TableId::CountryPowerChanges, |report| {
        changes(frame, save, links, report)
    });
    report.attempt_table(TableId::GovernmentChanges, |report| {
        governments(frame, save, links, tokens, report)
    });
    report.attempt_table(TableId::CountryPowerDaily,|report| {
        let rows=complete::<tables::CountryPowerDaily>(frame)?;
        report.limitation("native evaluated daily income is recomputed and not serialized; certify with application-site observations and the live run", "CountryPowerDaily");
        report.compared("CountryPowerDaily native origins",rows.len());Ok(())
    });
    report.attempt_table(TableId::CountryPowerDailyCosts,|report| {
        let rows=complete::<tables::CountryPowerDailyCosts>(frame)?;
        let daily=complete::<tables::CountryPowerDaily>(frame)?;
        validate_costs(daily,rows)?;
        report.limitation("daily costs are runtime evaluations absent from the save; component sum is an internal check, not independent cost proof", "CountryPowerDailyCosts");
        report.compared("CountryPowerDailyCosts joins and sum",rows.len());Ok(())
    });
}
fn coverage(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::PoliticsCaptureCoverage>(frame)?;
    let mut countries = std::collections::BTreeSet::new();
    for row in rows {
        let country = links
            .get(&row.country)
            .ok_or_else(|| invalid("politics coverage country unresolved"))?;
        if !countries.insert(country.canonical_slot) {
            return Err(invalid("politics coverage duplicates a live country"));
        }
        if row.through_hour != frame.hour {
            return Err(invalid("politics coverage has stale interval endpoint"));
        }
        if row.streams == 0 {
            report.limitation("politics interval has no certified observation baseline (attach, branch, identity change or loss)",String::from_utf8_lossy(&country.tag));
        }
    }
    for country in links.values() {
        if !countries.contains(&country.canonical_slot) {
            report.difference(
                format!(
                    "politics coverage {}",
                    String::from_utf8_lossy(&country.tag)
                ),
                "missing",
                "live country",
            );
        }
    }
    report.compared("PoliticsCaptureCoverage all-country census", rows.len());
    Ok(())
}
fn changes(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::CountryPowerChanges>(frame)?;
    let coverage = complete::<tables::PoliticsCaptureCoverage>(frame)?;
    let mut last = BTreeMap::new();
    for row in rows {
        if coverage
            .iter()
            .any(|c| c.country == row.country && c.epoch == row.epoch)
        {
            last.insert((row.country, row.currency as u8), row);
        } else {
            report.limitation(
                "historical mutation country link has no current endpoint witness",
                format!("{}:{}", row.country, row.sequence),
            );
        }
    }
    for ((country, _), row) in last {
        let tag = required_country_tag(country, links)?;
        let saved = save
            .countries
            .get(tag)
            .ok_or_else(|| invalid("power mutation country absent from save"))?;
        let (field, value) = match row.currency {
            PowerCurrency::Political => ("political_power", saved.political_power.as_ref()),
            PowerCurrency::Command => ("command_power", saved.command_power.as_ref()),
        };
        report.scalar(
            &String::from_utf8_lossy(tag),
            &format!("CountryPowerChanges final {field}"),
            Some(row.balance_after_raw),
            value,
        );
    }
    report.limitation("save retains balances but no historical charges, item occurrence order, grants/refunds or hook coverage; conservation must pass in status.json", "CountryPowerChanges");
    report.compared("CountryPowerChanges latest observed endpoints", rows.len());
    Ok(())
}
fn governments(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::GovernmentChanges>(frame)?;
    let coverage = complete::<tables::PoliticsCaptureCoverage>(frame)?;
    let parties = complete::<tables::PoliticalParties>(frame)?;
    let mut last = BTreeMap::new();
    for row in rows {
        if coverage
            .iter()
            .any(|c| c.country == row.country && c.epoch == row.epoch)
        {
            last.insert(row.country, row);
        }
    }
    for (country, row) in last {
        let tag = required_country_tag(country, links)?;
        let saved = save
            .governance
            .countries
            .get(tag)
            .and_then(|country| country.politics.as_ref())
            .and_then(|politics| politics.ruling_party.as_deref());
        let observed = row
            .after_ideology
            .and_then(|token| tokens.get(&token).copied())
            .or_else(|| {
                parties
                    .iter()
                    .find(|party| party.country == country && Some(party.id) == row.after_party)
                    .map(|party| party.token.as_slice())
            });
        report.scalar_checks += 1;
        if observed != saved {
            report.difference(
                format!(
                    "{} GovernmentChanges final ruling party",
                    String::from_utf8_lossy(tag)
                ),
                observed,
                saved,
            );
        }
    }
    report.limitation("save records current ruling party, not execution cause or old parties; elections/coups are certified by scoped native call provenance, never dates", "GovernmentChanges");
    report.compared("GovernmentChanges latest observed endpoints", rows.len());
    Ok(())
}

fn validate_costs(
    daily: &[CountryPowerDailyRow],
    costs: &[CountryPowerDailyCostRow],
) -> io::Result<()> {
    let days: BTreeMap<_, _> = daily
        .iter()
        .map(|day| ((day.country, day.epoch, day.day_hour, day.pp_sequence), day))
        .collect();
    let mut sums: BTreeMap<_, i64> = days
        .iter()
        .filter(|(_, day)| day.pp_sequence.is_some() && day.pp_cost_raw.is_some())
        .map(|(key, _)| (*key, 0))
        .collect();
    for row in costs {
        let key = (row.country, row.epoch, row.day_hour, Some(row.pp_sequence));
        if !days.contains_key(&key) {
            return Err(invalid("daily cost lacks its native application"));
        }
        let sum = sums.entry(key).or_default();
        *sum = sum.wrapping_add(row.cost_raw);
    }
    for (key, sum) in sums {
        if Some(sum) != days[&key].pp_cost_raw {
            return Err(invalid(
                "daily costs do not sum to the recorded native reduction",
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn costs_join_the_exact_country_epoch_day_and_application() {
        let day = CountryPowerDailyRow {
            country: 5001,
            instance: 9,
            epoch: 3,
            day_hour: 24,
            pp_sequence: Some(16),
            pp_cost_raw: Some(7),
            ..Default::default()
        };
        let cost = CountryPowerDailyCostRow {
            country: 5001,
            instance: 9,
            epoch: 3,
            day_hour: 24,
            pp_sequence: 16,
            cost_raw: 7,
            ..Default::default()
        };
        assert!(validate_costs(std::slice::from_ref(&day), std::slice::from_ref(&cost)).is_ok());
        let changed_target = CountryPowerDailyCostRow {
            instance: 10,
            ..cost.clone()
        };
        assert!(validate_costs(std::slice::from_ref(&day), &[changed_target]).is_ok());
        assert!(validate_costs(std::slice::from_ref(&day), &[]).is_err());
        for bad in [
            CountryPowerDailyCostRow {
                country: 5002,
                ..cost.clone()
            },
            CountryPowerDailyCostRow {
                epoch: 4,
                ..cost.clone()
            },
            CountryPowerDailyCostRow {
                pp_sequence: 20,
                ..cost.clone()
            },
            CountryPowerDailyCostRow {
                cost_raw: 8,
                ..cost.clone()
            },
        ] {
            assert!(validate_costs(std::slice::from_ref(&day), &[bad]).is_err());
        }
    }
    #[test]
    fn sums_are_separate_per_country_and_keep_native_i64_wrapping() {
        let days = [
            CountryPowerDailyRow {
                country: 5001,
                pp_sequence: Some(4),
                pp_cost_raw: Some(i64::MIN),
                ..Default::default()
            },
            CountryPowerDailyRow {
                country: 5002,
                pp_sequence: Some(8),
                pp_cost_raw: Some(3),
                ..Default::default()
            },
        ];
        let costs = [
            CountryPowerDailyCostRow {
                country: 5001,
                pp_sequence: 4,
                cost_raw: i64::MAX,
                ..Default::default()
            },
            CountryPowerDailyCostRow {
                country: 5001,
                pp_sequence: 4,
                ordinal: 1,
                cost_raw: 1,
                ..Default::default()
            },
            CountryPowerDailyCostRow {
                country: 5002,
                pp_sequence: 8,
                cost_raw: 3,
                ..Default::default()
            },
        ];
        assert!(validate_costs(&days, &costs).is_ok());
    }
    #[test]
    fn fresh_save_certifies_endpoints_and_requires_every_live_country() {
        use replay_frame::frame::TableView;
        let (mut frame, save, _, _) = super::super::fixtures::fixture();
        *frame.tables[TableId::PoliticsCaptureCoverage.index()] =
            TableView::Complete(TableRows::PoliticsCaptureCoverage(vec![
                PoliticsCaptureCoverageRow {
                    country: 101,
                    instance: 9,
                    epoch: 1,
                    since_hour: frame.hour - 1,
                    through_hour: frame.hour,
                    streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
                },
            ]));
        *frame.tables[TableId::CountryPowerChanges.index()] =
            TableView::Complete(TableRows::CountryPowerChanges(vec![
                CountryPowerChangeRow {
                    country: 101,
                    instance: 9,
                    epoch: 1,
                    sequence: 4,
                    hour: frame.hour,
                    balance_after_raw: 1_000_001,
                    ..Default::default()
                },
            ]));
        let countries = frame.complete::<tables::Countries>().unwrap();
        let links = BTreeMap::from([(101, &countries[0])]);
        let mut report = Report::default();
        coverage(&frame, &links, &mut report).unwrap();
        changes(&frame, &save, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        let mut report = Report::default();
        let extra = CountryRow {
            id: 5001,
            canonical_slot: 99,
            tag: b"ZZZ".to_vec(),
            ..Default::default()
        };
        let missing = BTreeMap::from([(101, &countries[0]), (5001, &extra)]);
        coverage(&frame, &missing, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        *frame.tables[TableId::CountryPowerChanges.index()] =
            TableView::Complete(TableRows::CountryPowerChanges(vec![
                CountryPowerChangeRow {
                    country: 101,
                    instance: 9,
                    epoch: 1,
                    sequence: 4,
                    hour: frame.hour,
                    balance_after_raw: 1_000_002,
                    ..Default::default()
                },
            ]));
        let links = BTreeMap::from([(101, &frame.complete::<tables::Countries>().unwrap()[0])]);
        let mut report = Report::default();
        changes(&frame, &save, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}
